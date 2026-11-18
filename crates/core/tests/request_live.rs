//! A person's request closes when its work is live, not when it is merely
//! on main: in a project that goes live by its own command
//! (`.equipe/deploy.json`), a request whose ticket landed but whose deploy
//! failed stays open with why; once a deploy succeeds it closes, and the
//! person is told.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::protocol::requests::{Milestone, Phase, Request};
use equipe_core::protocol::rpc::{RunKey, ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-request-live-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn write_script(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A stand-in `claude` that writes down every turn's arguments (the prompt
/// first, as `-p`), one file a call; a prompt saying LONG keeps its turn
/// going a long while, and a ticket's run leaves a change in its worktree.
fn recording_claude() -> (PathBuf, PathBuf) {
    let calls = scratch("calls");
    let bin = scratch("bin").join("claude");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\n\
if [ \"$2\" = --input-format ] && IFS= read -r line; then set -- -p \"$(printf '%s' \"$line\" | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"message\"][\"content\"], end=\"\")')\" \"$@\"; fi\n\
n=0\nwhile ! mkdir '{dir}/.n-'$n 2>/dev/null; do n=$((n+1)); done\n\
for a in \"$@\"; do printf '%s\\n<<arg>>\\n' \"$a\"; done > '{dir}/.call-'$$\nmv '{dir}/.call-'$$ '{dir}/call-'$(printf %03d $n)-$$\n\
case \"$*\" in *LONG*) exec sleep 30;; *\"You're on T-\"*) echo made > HERO.md;; esac\n\
cat <<EOF\n{{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-test\",\"session_id\":\"sess-1\"}}\n\
{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"usage\":{{\"input_tokens\":3,\"output_tokens\":1}}}}\nEOF\n",
        dir = calls.display()
    );
    write_script(&bin, &script);
    (bin, calls)
}

fn git(dir: &Path, args: &[&str]) {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn repo() -> PathBuf {
    let dir = scratch("repo");
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

fn config(claude: &Path, home: &Path) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        codex_binary: "/nonexistent/codex".into(),
        persist: true,
        retention_days: 90,
        equipe_binary: Some("/opt/equipe".into()),
        npx: None,
        design_path: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        person_home: Some(home.join("person")),
        lang: equipe_core::i18n::Lang::En,
        nemotron: None,
        watch_disk: false,
    }
}

async fn until(rx: &mut UnboundedReceiver<Event>, done: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    let wait = async {
        while let Some(event) = rx.recv().await {
            let stop = done(&event);
            seen.push(event);
            if stop {
                return;
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(30), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

/// Every event that came in meanwhile, without waiting.
fn drain(rx: &mut UnboundedReceiver<Event>) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Ok(event) = rx.try_recv() {
        seen.push(event);
    }
    seen
}

fn member(cwd: &Path, role: &str) -> SessionSpec {
    SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: cwd.to_owned(), role: Some(role.into()), squad: Some("acme".into()) }
}

async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, spec: SessionSpec) {
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

/// A tool call from the run of `ticket`, as its board server makes it.
async fn tool_on(handle: &Handle, member: &str, ticket: u32, name: &str, arguments: serde_json::Value) -> ToolResult {
    let snapshot = handle.snapshot("acme").await;
    let run = snapshot.sessions.iter().find_map(|s| s.run.filter(|r| r.ticket == ticket).map(|r| RunKey { session: s.session, ticket, generation: r.generation }));
    handle.tool(ToolCall { run, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

async fn requests(handle: &Handle) -> Vec<Request> {
    handle.snapshot("acme").await.requests
}

fn notices(events: &[Event], to: u64) -> Vec<String> {
    events.iter().filter_map(|e| if let Event::Notice { session, text, .. } = e { (*session == to).then(|| text.clone()) } else { None }).collect()
}

fn request(listed: &[Request], id: u64) -> Request {
    listed.iter().find(|r| r.id == id).cloned().unwrap_or_else(|| panic!("R-{id} isn't listed: {listed:#?}"))
}


fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Wait until request `id` is listed as `done`.
async fn request_until(handle: &Handle, id: u64, done: impl Fn(&Request) -> bool) -> Request {
    for _ in 0..300 {
        if let Some(r) = requests(handle).await.into_iter().find(|r| r.id == id)
            && done(&r)
        {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("R-{id} never got there: {:#?}", requests(handle).await);
}

#[tokio::test]
async fn a_request_closes_when_its_work_is_live_not_when_it_is_on_main() {
    // SAFETY: the only test in this file, before its daemon starts.
    unsafe {
        std::env::set_var("EQUIPE_REQUEST_SETTLE", "0");
    }
    // A project that goes live by its own command; the deploy fails until `ok` exists.
    let repo = repo();
    let marks = scratch("marks");
    let (ok, runs) = (marks.join("ok"), marks.join("runs.txt"));
    std::fs::create_dir_all(repo.join(".equipe")).unwrap();
    let deploy = format!(
        r#"{{ "command": "test -f '{ok}' || {{ echo 'Error: no deploy token' >&2; exit 4; }}; git rev-parse HEAD >> '{runs}'", "quietSeconds": 1 }}"#,
        ok = ok.display(),
        runs = runs.display()
    );
    std::fs::write(repo.join(".equipe/deploy.json"), deploy).unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "declare how it goes live"]);
    let origin = scratch("origin");
    git(&origin, &["init", "-q", "--bare", "-b", "main"]);
    git(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&repo, &["push", "-q", "origin", "main"]);
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let (claude, _calls) = recording_claude();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;

    // The person asks; a ticket serves it, and its member lands it: main is pushed.
    handle.send(Command::Submit { session: 1, text: "Add a hero banner to the home page".into(), about: None });
    until(&mut events, |e| matches!(e, Event::UserMessage { session: 1, .. })).await;
    let made = tool(&handle, "pm", "create_ticket", json!({ "title": "Hero banner", "body": "-", "owner": "frontend", "request": 1 })).await;
    assert!(!made.is_error, "{made:?}");
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let handed = tool_on(&handle, "frontend", 1, "submit_for_review", json!({ "summary": "Hero banner added.", "changes_screens": false })).await;
    assert!(!handed.is_error, "{handed:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.merged == Some(true))).await;

    // Its deploy fails: the request stays open, on main and not live, with why.
    until(&mut events, |e| matches!(e, Event::Notice { session: 1, text, .. } if text.contains("isn't live yet"))).await;
    let r = request_until(&handle, 1, |r| r.status.parts.first().is_some_and(|p| p.not_live.is_some())).await;
    assert_eq!((r.status.parts[0].phase, r.status.parts[0].not_live.as_deref(), r.status.done), (Phase::OnMain, Some("deploy_failed"), false), "{r:#?}");
    handle.minute_now().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let seen = drain(&mut events);
    assert!(!seen.iter().any(|e| matches!(e, Event::RequestNews { milestone: Milestone::AllDone, .. })), "not done while not live: {seen:#?}");
    let line = notices(&seen, 1).into_iter().find(|t| t.starts_with("Your request “Add a hero banner"));
    assert_eq!(line.as_deref(), Some("Your request “Add a hero banner to the home page”: Hero banner — on main, not live: its deploy failed"), "{seen:#?}");
    let r = request(&requests(&handle).await, 1);
    assert!(r.closed_at.is_none(), "{r:#?}");

    // Fixed, the PM deploys again: live, the request closes and the person is told.
    std::fs::write(&ok, "").unwrap();
    let again = tool(&handle, "pm", "set_deploy", json!({ "deploy_now": true })).await;
    assert!(!again.is_error, "{again:?}");
    until(&mut events, |e| matches!(e, Event::DeploysChanged { deploys, .. } if deploys.iter().any(|d| d.source.is_live() && d.tickets.contains(&1)))).await;
    assert_eq!(std::fs::read_to_string(&runs).unwrap().trim(), git_out(&repo, &["rev-parse", "origin/main"]));
    let r = request_until(&handle, 1, |r| r.status.done).await;
    assert_eq!(r.status.parts[0].phase, Phase::Live);
    drain(&mut events);
    handle.minute_now().await;
    let seen = until(&mut events, |e| matches!(e, Event::RequestNews { request: 1, milestone: Milestone::AllDone, .. })).await;
    assert!(notices(&seen, 1).iter().any(|t| t.starts_with("Your request “Add a hero banner to the home page”: Hero banner — live") && t.ends_with("all done")), "{seen:#?}");
    assert!(request(&requests(&handle).await, 1).closed_at.is_some());
    handle.shutdown().await;
}
