//! The person's requests, when several arrive close together — driven the
//! way a screen and an engine drive them: each is a request of its own; a
//! ticket put on the board in answer names the one it serves (the member is
//! asked which, each in one line, when it doesn't); equipe's own checks
//! cover none; one no work ticket covers is put to the PM and shown so; and
//! a request closes on its own work only — or by a member, with why.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::protocol::requests::{Link, Milestone, Request};
use equipe_core::protocol::rpc::{RunKey, ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Event, SessionSpec, TicketStatus};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-request-coverage-{name}-{}-{n}", std::process::id()));
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

/// Each recorded turn's prompt, oldest first.
fn prompts(dir: &Path) -> Vec<String> {
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("call-")).map(|e| e.path()).collect();
    files.sort();
    files
        .iter()
        .filter_map(|f| {
            let args: Vec<String> = std::fs::read_to_string(f).unwrap().split("\n<<arg>>\n").map(str::to_owned).collect();
            args.iter().position(|a| a == "-p").and_then(|i| args.get(i + 1).cloned())
        })
        .collect()
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

#[tokio::test]
async fn three_requests_in_a_row_each_get_their_own_work_none_is_dropped_nor_closed_on_anothers() {
    // SAFETY: the only test in this file, before its daemon starts.
    unsafe {
        std::env::set_var("EQUIPE_REQUEST_SETTLE", "0");
        std::env::set_var("EQUIPE_REQUEST_COVER_WAIT", "2");
    }
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let (claude, calls) = recording_claude();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;

    // Three requests to the PM within a minute, its turn on the first still going.
    handle.send(Command::Submit { session: 1, text: "Login and logout show this error\n(LONG)".into(), about: None });
    until(&mut events, |e| matches!(e, Event::UserMessage { session: 1, .. })).await;
    handle.send(Command::Submit { session: 1, text: "Dark mode: a black terminal style".into(), about: None });
    handle.send(Command::Submit { session: 1, text: "An admin-only page showing who used MCP, and how many calls per task".into(), about: None });
    assert!(requests(&handle).await.is_empty(), "words the PM is about to answer aren't listed yet");

    // A ticket that doesn't say which it serves: the PM is asked, each request in one line, and nothing is made.
    let unnamed = tool(&handle, "pm", "create_ticket", json!({ "title": "Fix the login error", "body": "-", "owner": "qa" })).await;
    assert!(unnamed.is_error, "{unnamed:?}");
    for line in ["- R-1 “Login and logout show this error” (no ticket yet) · asked ", "- R-2 “Dark mode: a black terminal style” (no ticket yet) · asked ", "- R-3 “An admin-only page showing who used MCP, and how many calls…” (no ticket yet) · asked "] {
        assert!(unnamed.text.contains(line), "{line} in {}", unnamed.text);
    }
    assert!(handle.snapshot("acme").await.tickets.is_empty());

    // Named: each joins its own, and the reply names it in the person's words.
    let login = tool(&handle, "pm", "create_ticket", json!({ "title": "Fix the login error", "body": "-", "owner": "qa", "request": 1 })).await;
    assert!(!login.is_error && login.text.contains("R-1 “Login and logout show this error”"), "{login:?}");
    let dark = tool(&handle, "pm", "create_ticket", json!({ "title": "Apply the dark theme", "body": "Its run takes a while (LONG)", "owner": "frontend", "kind": "task", "request": "R-2" })).await;
    assert!(!dark.is_error && dark.text.contains("R-2 “Dark mode"), "{dark:?}");
    // Only a check for the third: it serves it, but covers nothing.
    let check = tool(&handle, "pm", "create_ticket", json!({ "title": "Verify the MCP usage numbers", "body": "-", "owner": "qa", "kind": "task", "label": "check", "request": 3 })).await;
    assert!(!check.is_error && check.text.contains("R-3"), "{check:?}");
    // The PM's own ticket, for none of them.
    let own = tool(&handle, "pm", "create_ticket", json!({ "title": "Tidy the docs", "body": "-", "owner": "qa", "kind": "task", "request": 0 })).await;
    assert!(!own.is_error && !own.text.contains("R-"), "{own:?}");

    // At work on the dark theme, the frontend files a piece of it: it joins by its lineage, unasked.
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 2, .. })).await;
    let piece = tool_on(&handle, "frontend", 2, "create_ticket", json!({ "title": "Dark scrollbars", "body": "-", "owner": "qa", "kind": "task", "separate_why": "Another app's styles, nothing on this branch" })).await;
    assert!(!piece.is_error && piece.text.contains("R-2"), "{piece:?}");
    let listed = requests(&handle).await;
    assert_eq!(request(&listed, 1).tickets.iter().map(|t| (t.ticket, t.link)).collect::<Vec<_>>(), [(1, Link::Asked)]);
    assert_eq!(request(&listed, 2).tickets.iter().map(|t| (t.ticket, t.link)).collect::<Vec<_>>(), [(2, Link::Asked), (5, Link::Made)]);
    assert_eq!(request(&listed, 3).tickets.iter().map(|t| t.ticket).collect::<Vec<_>>(), [3]);
    assert!(listed.iter().all(|r| !r.status.uncovered), "a few minutes to put it on the board: {listed:#?}");

    // A while later, the one only a check serves is uncovered: put to the PM, shown to the person so, kept open.
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    drain(&mut events);
    handle.minute_now().await;
    let seen = until(&mut events, |e| matches!(e, Event::UserMessage { session: 1, text, .. } | Event::MessageQueued { session: 1, text, .. } if text.starts_with("[Uncovered requests]"))).await;
    let told = seen.iter().find_map(|e| match e {
        Event::UserMessage { text, .. } | Event::MessageQueued { text, .. } if text.starts_with("[Uncovered requests]") => Some(text.clone()),
        _ => None,
    });
    let told = told.unwrap();
    assert!(told.contains("R-3 — asked of pm at ") && told.contains("“An admin-only page showing who used MCP, and how many calls per task”"), "{told}");
    assert!(!told.contains("R-1") && !told.contains("R-2"), "{told}");
    let listed = requests(&handle).await;
    let r3 = request(&listed, 3);
    assert!(r3.status.uncovered && r3.closed_at.is_none(), "{r3:#?}");
    assert!(equipe_core::requests::line(&r3.status, equipe_core::i18n::core(equipe_core::i18n::Lang::En), &|_| String::new()).starts_with("no ticket serves it yet"));
    assert!(!request(&listed, 1).status.uncovered && !request(&listed, 2).status.uncovered);
    handle.minute_now().await;
    let again = drain(&mut events);
    assert!(!again.iter().any(|e| matches!(e, Event::MessageQueued { text, .. } | Event::UserMessage { text, .. } if text.starts_with("[Uncovered requests]"))), "told once");

    // The PM's next turn lists it as such, until a ticket covers it.
    handle.send(Command::Interrupt { session: 1 });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    handle.send(Command::Submit { session: 1, text: "and the login error?".into(), about: None });
    let mut last = None;
    for _ in 0..100 {
        last = prompts(&calls).into_iter().rfind(|p| p.contains("R-3 “An admin-only page") && p.contains("no ticket serves it yet"));
        if last.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let last = last.expect("the PM's turn lists the uncovered request");
    assert!(last.contains("R-1 “Login and logout show this error” (T-1) · asked "), "{last}");
    assert_eq!(requests(&handle).await.len(), 3, "asking after one isn't a new request");
    let mcp = tool(&handle, "pm", "create_ticket", json!({ "title": "Admin page of MCP calls per task", "body": "-", "owner": "qa", "request": 3 })).await;
    assert!(!mcp.is_error && mcp.text.contains("R-3"), "{mcp:?}");
    assert!(!request(&requests(&handle).await, 3).status.uncovered);

    // The dark theme lands: its request is all done and closes — the others don't, on its tickets.
    handle.send(Command::Cancel { squad: "acme".into(), ticket: 5 });
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 5 && ticket.status == TicketStatus::Cancelled)).await;
    let handed = tool_on(&handle, "frontend", 2, "submit_for_review", json!({ "summary": "Dark theme applied.", "changes_screens": false })).await;
    assert!(!handed.is_error, "{handed:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 2 && ticket.status == TicketStatus::Review)).await;
    handle.send(Command::Accept { squad: "acme".into(), ticket: 2 });
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 2 && ticket.status == TicketStatus::Done)).await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    drain(&mut events);
    handle.minute_now().await;
    let seen = drain(&mut events);
    let all_done: Vec<u64> = seen.iter().filter_map(|e| match e {
        Event::RequestNews { request, milestone: Milestone::AllDone, .. } => Some(*request),
        _ => None,
    }).collect();
    assert_eq!(all_done, [2], "{seen:#?}");
    let listed = requests(&handle).await;
    assert!(request(&listed, 2).closed_at.is_some());
    assert!(request(&listed, 1).closed_at.is_none() && request(&listed, 3).closed_at.is_none(), "{listed:#?}");

    // Closed by the PM with why: the person reads it where they asked.
    drain(&mut events);
    let closed = tool(&handle, "pm", "close_request", json!({ "request": 1, "why": "already fixed in the last release" })).await;
    assert!(!closed.is_error && closed.text.contains("closed: already fixed in the last release") && closed.text.contains("T-1"), "{closed:?}");
    let seen = drain(&mut events);
    assert!(notices(&seen, 1).iter().any(|t| t.starts_with("Your request “Login and logout show this error”: closed: already fixed in the last release · Fix the login error — ")), "{seen:#?}");
    let r1 = request(&requests(&handle).await, 1);
    assert_eq!((r1.closed_at.is_some(), r1.status.closed_why.as_deref()), (true, Some("already fixed in the last release")));
    // Asked of the PM: no other member closes it.
    let refused = tool(&handle, "frontend", "close_request", json!({ "request": 3, "why": "done" })).await;
    assert!(refused.is_error && refused.text.contains("asked of pm"), "{refused:?}");

    // Words that ask for no work: closed with why, never listed.
    handle.send(Command::Submit { session: 1, text: "thanks!".into(), about: None });
    let mut thanks = tool(&handle, "pm", "close_request", json!({ "request": 4, "why": "just thanks" })).await;
    for _ in 0..50 {
        if !thanks.is_error {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        thanks = tool(&handle, "pm", "close_request", json!({ "request": 4, "why": "just thanks" })).await;
    }
    assert!(!thanks.is_error, "{thanks:?}");
    assert!(requests(&handle).await.iter().all(|r| r.id != 4));
    handle.shutdown().await;
}
