//! Members on the person's own Codex CLI, driven the way a screen drives the
//! runtime. A stand-in `codex app-server` (the providers' `tests/fake-codex`)
//! replays recorded turns: no network, no sign-in.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::protocol::{Access, Command, Decision, Engine, Event, SessionSpec, TurnOutcome};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-cx-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn write_script(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The stand-in `codex`, writing down what it is sent; `env` more of its environment.
fn fake_codex(env: &str) -> (PathBuf, PathBuf) {
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("../providers/tests/fake-codex/codex").canonicalize().unwrap();
    let log = scratch("log").join("sent.jsonl");
    let bin = scratch("bin").join("codex");
    write_script(&bin, &format!("#!/bin/sh\nFAKE_CODEX_LOG='{}' {env} exec '{}' \"$@\"\n", log.display(), fake.display()));
    (bin, log)
}

/// A `claude` that isn't signed in: only Codex is ready.
fn no_claude() -> PathBuf {
    let bin = scratch("bin").join("claude");
    write_script(&bin, "#!/bin/sh\necho '{\"loggedIn\":false}'\n");
    bin
}

fn config(codex: &Path, home: &Path) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: no_claude().to_string_lossy().into_owned(),
        codex_binary: codex.to_string_lossy().into_owned(),
        persist: false,
        retention_days: 90,
        equipe_binary: None,
        npx: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        person_home: Some(home.join("person")),
        lang: equipe_core::i18n::Lang::En,
    }
}

/// What the stand-in was sent, in order.
fn sent(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn params_of(log: &Path, method: &str) -> Vec<Value> {
    sent(log).into_iter().filter(|m| m["method"] == method).map(|m| m["params"].clone()).collect()
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
    if tokio::time::timeout(Duration::from_secs(20), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

fn text_of(events: &[Event]) -> String {
    events.iter().filter_map(|e| if let Event::TextDelta { text, .. } = e { Some(text.as_str()) } else { None }).collect()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
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

/// A member of the squad `acme` on Codex.
fn on_codex(cwd: &Path, role: &str) -> SessionSpec {
    SessionSpec { engine: Some(Engine::Codex), model: None, access: Access::ReadOnly, cwd: cwd.to_owned(), role: Some(role.into()), squad: Some("acme".into()) }
}

async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, spec: SessionSpec) -> Vec<Event> {
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id) || matches!(e, Event::Error { .. })).await
}

#[tokio::test]
async fn codex_edits_in_its_own_worktree_and_its_work_lands_on_a_branch() {
    let (codex, log) = fake_codex("");
    let repo = repo();
    let (handle, mut events) = runtime::start(config(&codex, &scratch("home")));
    // Nothing named: an edit goes to Claude first, which isn't signed in here — so Codex.
    let started = add(&handle, &mut events, 1, SessionSpec { engine: None, model: None, access: Access::EditFiles, cwd: repo.clone(), role: None, squad: None }).await;
    let Some(Event::SessionStarted { engine: Engine::Codex, model, workspace: Some(workspace), .. }) = started.last() else { panic!("{started:#?}") };
    assert_eq!(model.as_deref(), Some("gpt-6.1-sol"), "the account's default, from its own model list");

    handle.send(Command::Submit { session: 1, text: "add hello.txt".into(), about: None });
    let turn = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert_eq!(text_of(&turn), "I'll add the file, then test.DONE");
    let hello = workspace.path.join("hello.txt").to_string_lossy().into_owned();
    assert!(turn.iter().any(|e| matches!(e, Event::ToolUse { tool, target: Some(t), .. } if tool == "Write" && *t == hello)), "{turn:#?}");
    assert!(turn.iter().any(|e| matches!(e, Event::ToolResult { tool, outcome, .. } if tool == "Write" && outcome.diff.as_deref().is_some_and(|d| d.contains("+hi")))));
    assert!(turn.iter().any(|e| matches!(e, Event::ToolResult { tool, outcome, .. } if tool == "Bash" && outcome.exit == Some(101) && !outcome.ok)));
    assert!(turn.iter().any(|e| matches!(e, Event::ToolUse { tool, .. } if tool == "TodoWrite")));
    // The plan's usage, its windows known by their length.
    let usage = turn.iter().find_map(|e| if let Event::PlanUsage { usage, engine: Engine::Codex } = e { Some(*usage) } else { None }).expect("codex's plan usage");
    assert_eq!((usage.five_hour.map(|w| w.used_percent), usage.seven_day.map(|w| w.used_percent)), (Some(42), Some(5)));
    assert!(matches!(turn.last(), Some(Event::TurnEnded { outcome: TurnOutcome::Completed, usage: Some(_), .. })));
    let plans = handle.snapshot("any").await.plans;
    assert!(plans.iter().any(|p| p.engine == Engine::Codex && p.plan_type.as_deref() == Some("pro") && p.at > 0), "{plans:?}");

    // Fenced into its worktree, asking to leave it.
    let start = &params_of(&log, "thread/start")[0];
    assert_eq!(start["cwd"], workspace.path.to_string_lossy().as_ref());
    assert_eq!((start["sandbox"].as_str(), start["approvalPolicy"].as_str()), (Some("workspace-write"), Some("on-request")));
    assert_eq!(params_of(&log, "turn/start")[0]["sandboxPolicy"]["type"], "workspaceWrite");

    // The next turn continues the thread.
    handle.send(Command::Submit { session: 1, text: "and again".into(), about: None });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert_eq!(params_of(&log, "thread/resume")[0]["threadId"], "thr-1");

    handle.send(Command::Close { session: 1 });
    let closed = until(&mut events, |e| matches!(e, Event::SessionClosed { .. })).await;
    let branch = closed.iter().find_map(|e| if let Event::BranchLanded { branch, .. } = e { Some(branch.clone()) } else { None }).expect("a branch");
    assert_eq!(git(&repo, &["show", &format!("{branch}:hello.txt")]), "hi");
}

#[tokio::test]
async fn codex_asks_the_person_and_an_always_becomes_the_squads_rule() {
    let (codex, log) = fake_codex("");
    let cwd = scratch("site");
    let home = scratch("home");
    squads::create(&home, "acme", &cwd).unwrap();
    let mut config = config(&codex, &home);
    config.equipe_binary = Some("/opt/equipe".into());
    let (handle, mut events) = runtime::start(config);
    add(&handle, &mut events, 1, on_codex(&cwd, "qa")).await;

    handle.send(Command::Submit { session: 1, text: "approve the push".into(), about: None });
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = asked.last() else { unreachable!() };
    assert_eq!((approval.tool.as_str(), approval.action.as_str()), ("Bash", "git push origin main"));
    assert!(approval.reason.as_deref().is_some_and(|r| r.contains("Pushing needs the network")), "{:?}", approval.reason);
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Always, words: None });
    let turn = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(text_of(&turn).contains("Pushed."), "{turn:#?}");
    assert_eq!(sent(&log).iter().find(|m| m["id"] == "srv-1").unwrap()["result"]["decision"], "accept");

    // Always allowed: the same kind of command doesn't ask again.
    handle.send(Command::Submit { session: 1, text: "approve the push again".into(), about: None });
    let again = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(!again.iter().any(|e| matches!(e, Event::ApprovalRequested { .. })), "{again:#?}");
    assert!(text_of(&again).contains("Pushed."));

    // Its board tools reach it as Claude Code's do, and never ask.
    let start = &params_of(&log, "thread/start")[0];
    let board = &start["config"]["mcp_servers"]["equipe"];
    assert_eq!(board["command"], "/opt/equipe");
    assert_eq!(board["default_tools_approval_mode"], "approve");
    let args: Vec<&str> = board["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert_eq!(&args[..5], ["mcp", "--squad", "acme", "--member", "qa"]);
    // Only equipe's: the person's own are switched off, by name.
    assert_eq!(start["config"]["mcp_servers"]["node_repl"]["enabled"], false);
    assert_eq!(start["config"]["plugins"]["unified-computer-use@openai-bundled"]["enabled"], false);
    assert_eq!(start["config"]["features"]["apps"], false);
    assert!(start["developerInstructions"].as_str().unwrap().starts_with("You are qa, the QA engineer"));
}

#[tokio::test]
async fn a_declined_command_is_not_run() {
    let (codex, _) = fake_codex("");
    let cwd = scratch("site");
    let home = scratch("home");
    squads::create(&home, "acme", &cwd).unwrap();
    let (handle, mut events) = runtime::start(config(&codex, &home));
    add(&handle, &mut events, 1, on_codex(&cwd, "qa")).await;
    handle.send(Command::Submit { session: 1, text: "approve the push".into(), about: None });
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = asked.last() else { unreachable!() };
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Deny, words: None });
    let turn = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(text_of(&turn).contains("declined"), "{turn:#?}");
}

#[tokio::test]
async fn esc_stops_a_codex_turn_through_codex_and_the_next_one_runs() {
    let (codex, log) = fake_codex("");
    let (handle, mut events) = runtime::start(config(&codex, &scratch("home")));
    let spec = SessionSpec { engine: Some(Engine::Codex), model: None, access: Access::ReadOnly, cwd: scratch("cwd"), role: None, squad: None };
    add(&handle, &mut events, 1, spec).await;
    handle.send(Command::Submit { session: 1, text: "interrupt me".into(), about: None });
    until(&mut events, |e| matches!(e, Event::TextDelta { .. })).await;
    handle.send(Command::Interrupt { session: 1 });
    let ended = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(matches!(ended.last(), Some(Event::TurnEnded { outcome: TurnOutcome::Interrupted, .. })));
    // Codex is told, and winds its turn down itself.
    for _ in 0..100 {
        if !params_of(&log, "turn/interrupt").is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(params_of(&log, "turn/interrupt")[0]["turnId"], "turn-1");
    handle.send(Command::Submit { session: 1, text: "go on".into(), about: None });
    let next = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(matches!(next.last(), Some(Event::TurnEnded { outcome: TurnOutcome::Completed, .. })), "ended once, then the next turn: {next:#?}");
    let read_only = &params_of(&log, "thread/start")[0];
    assert_eq!((read_only["sandbox"].as_str(), read_only["approvalPolicy"].as_str()), (Some("read-only"), Some("never")));
}

#[tokio::test]
async fn a_signed_out_codex_cli_says_how_to_sign_in() {
    let (codex, _) = fake_codex("FAKE_CODEX_SIGNED_OUT=1");
    let (handle, mut events) = runtime::start(config(&codex, &scratch("home")));
    let spec = SessionSpec { engine: Some(Engine::Codex), model: None, access: Access::ReadOnly, cwd: scratch("cwd"), role: None, squad: None };
    let failed = add(&handle, &mut events, 1, spec).await;
    let Some(Event::Error { message, next, .. }) = failed.last() else { panic!("{failed:#?}") };
    assert!(message.contains("Codex CLI isn't signed in"), "{message}");
    assert_eq!(next.as_deref(), Some("equipe login codex"));
}
