//! The Codex engine against a stand-in `codex app-server` (`tests/fake-codex/codex`)
//! that replays turns recorded from a real one: no network, no sign-in.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_providers::claude_code::Access;
use equipe_providers::codex_app::{self, Ask, SignIn, TurnRequest};
use equipe_providers::turn::{Activity, FIVE_HOURS, SEVEN_DAYS, ToolDone};
use equipe_providers::Error;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-codex-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn fake() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake-codex/codex").to_string_lossy().into_owned()
}

/// The fake behind a wrapper that sets its environment.
fn wrapped(env: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let bin = scratch("bin").join("codex");
    std::fs::write(&bin, format!("#!/bin/sh\n{env} exec '{}' \"$@\"\n", fake())).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin.to_string_lossy().into_owned()
}

/// What the app-server was sent, in order.
fn sent(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn method<'a>(log: &'a [Value], name: &str) -> Option<&'a Value> {
    log.iter().find(|m| m["method"] == name)
}

struct Run {
    activities: Vec<Activity>,
    result: equipe_providers::Result<codex_app::Turn>,
    log: Vec<Value>,
    cwd: PathBuf,
}

fn done(activities: &[Activity]) -> Vec<&ToolDone> {
    activities.iter().filter_map(|a| if let Activity::ToolDone(d) = a { Some(d) } else { None }).collect()
}

fn text(activities: &[Activity]) -> String {
    activities.iter().filter_map(|a| if let Activity::Text(t) = a { Some(t.as_str()) } else { None }).collect()
}

/// One turn on the fake, with whatever the test does alongside it.
async fn run(prompt: &str, access: Access, resume: Option<&str>, asks: Option<mpsc::UnboundedSender<Ask>>, steering: Option<mpsc::UnboundedReceiver<String>>, stop: Option<oneshot::Receiver<()>>) -> Run {
    let cwd = scratch("cwd");
    let log = scratch("log").join("sent.jsonl");
    let env = [("FAKE_CODEX_LOG".to_owned(), log.to_string_lossy().into_owned())];
    let writable = [PathBuf::from("/tmp/equipe-builds/frontend")];
    let request = TurnRequest { prompt, model: Some("gpt-6.1-sol"), effort: Some("medium"), access, resume, instructions: Some("You are qa."), writable: &writable, env: &env, ..Default::default() };
    let mut activities = Vec::new();
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        codex_app::run_turn(&fake(), &cwd, request, steering, asks, stop, |a| activities.push(a)),
    )
    .await
    .expect("the turn ends");
    Run { activities, result, log: sent(&log), cwd }
}

#[tokio::test]
async fn a_recorded_turn_streams_as_entries_and_writes_its_files() {
    let run = run("add hello.txt", Access::EditFiles, None, None, None, None).await;
    let turn = run.result.unwrap();
    assert_eq!(turn.thread_id.as_deref(), Some("thr-1"), "the thread that continues the conversation");
    assert_eq!((turn.result.text.as_str(), turn.result.status.as_str()), ("DONE", "completed"));
    let usage = turn.result.usage.unwrap();
    assert_eq!((usage.output_tokens, usage.context_window), (Some(309), Some(258400)));

    let a = &run.activities;
    assert_eq!(a[0], Activity::Model("gpt-6.1-sol".into()));
    assert!(a.contains(&Activity::Thinking("Looking at the repo".into())));
    assert_eq!(text(a), "I'll add the file, then test.DONE");
    let tools: Vec<(&str, Option<&str>)> = a.iter().filter_map(|x| if let Activity::Tool { name, target } = x { Some((name.as_str(), target.as_deref())) } else { None }).collect();
    let readme = run.cwd.join("README.md").to_string_lossy().into_owned();
    let hello = run.cwd.join("hello.txt").to_string_lossy().into_owned();
    assert_eq!(tools, [
        ("Read", Some(readme.as_str())),
        ("Write", Some(hello.as_str())),
        ("Bash", Some("cargo test")),
        ("mcp__equipe__board", None),
        ("WebSearch", Some("codex app-server")),
        ("TodoWrite", Some("1/2 · Run the tests")),
    ]);
    let outcomes = done(a);
    assert!(outcomes[1].diff.as_deref().is_some_and(|d| d.starts_with("diff --git") && d.ends_with("+hi\n")), "{outcomes:?}");
    assert_eq!((outcomes[2].ok, outcomes[2].exit), (false, Some(101)));
    assert!(outcomes[2].output.contains("1 failed"));
    assert!(a.iter().any(|x| matches!(x, Activity::Limits(l) if l.window(FIVE_HOURS).is_some_and(|w| w.used_percent == 42.0) && l.plan_type.as_deref() == Some("pro"))));
    assert_eq!(std::fs::read_to_string(run.cwd.join("hello.txt")).unwrap(), "hi\n", "the edit is real, in the turn's directory");

    // What equipe sent: who it is, the thread in the worktree, the turn's sandbox.
    let hello = method(&run.log, "initialize").unwrap();
    assert_eq!(hello["params"]["clientInfo"]["name"], "equipe");
    assert!(method(&run.log, "account/read").is_some(), "the sign-in is checked before the turn");
    let start = &method(&run.log, "thread/start").unwrap()["params"];
    assert_eq!(start["cwd"], run.cwd.to_string_lossy().as_ref());
    assert_eq!((start["sandbox"].as_str(), start["approvalPolicy"].as_str()), (Some("workspace-write"), Some("on-request")));
    assert_eq!(start["developerInstructions"], "You are qa.");
    let turn = &method(&run.log, "turn/start").unwrap()["params"];
    assert_eq!(turn["sandboxPolicy"]["writableRoots"], serde_json::json!(["/tmp/equipe-builds/frontend"]));
    assert_eq!((turn["model"].as_str(), turn["effort"].as_str()), (Some("gpt-6.1-sol"), Some("medium")));
}

#[tokio::test]
async fn a_read_only_turn_runs_in_codexs_read_only_sandbox_and_never_asks() {
    let run = run("look around", Access::ReadOnly, None, None, None, None).await;
    run.result.unwrap();
    let start = &method(&run.log, "thread/start").unwrap()["params"];
    assert_eq!((start["sandbox"].as_str(), start["approvalPolicy"].as_str()), (Some("read-only"), Some("never")));
    assert_eq!(method(&run.log, "turn/start").unwrap()["params"]["sandboxPolicy"]["type"], "readOnly");
}

/// A turn whose approval request the "person" answers `allow`.
async fn approval(allow: bool) -> (Run, Ask) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Ask>();
    let person = tokio::spawn(async move {
        let ask = rx.recv().await.expect("Codex asks");
        let (shown, answer) = (Ask { tool: ask.tool.clone(), input: ask.input.clone(), answer: oneshot::channel().0 }, ask.answer);
        let _ = answer.send(allow);
        shown
    });
    let run = run("approve the push", Access::EditFiles, None, Some(tx), None, None).await;
    (run, person.await.unwrap())
}

#[tokio::test]
async fn an_approval_goes_to_the_person_and_their_answer_back_to_codex() {
    let (run, asked) = approval(true).await;
    assert_eq!(asked.tool, "Bash");
    assert_eq!(asked.input["command"], "git push origin main");
    assert_eq!(asked.input["description"], "Pushing needs the network");
    assert_eq!(asked.input["dangerouslyDisableSandbox"], true, "allowed, it runs outside the sandbox: the person is told so");
    assert_eq!(run.result.unwrap().result.text, "Pushed.");
    let answered = run.log.iter().find(|m| m["id"] == "srv-1").unwrap();
    assert_eq!(answered["result"]["decision"], "accept");

    let (declined, _) = approval(false).await;
    assert!(done(&declined.activities).iter().any(|d| d.name == "Bash" && !d.ok && d.output == "Declined."));
    assert_eq!(declined.result.unwrap().result.text, "Not pushed: declined.");
    assert_eq!(declined.log.iter().find(|m| m["id"] == "srv-1").unwrap()["result"]["decision"], "decline");
}

#[tokio::test]
async fn with_nobody_to_ask_codex_is_declined() {
    let run = run("approve the push", Access::EditFiles, None, None, None, None).await;
    assert_eq!(run.result.unwrap().result.text, "Not pushed: declined.");
}

#[tokio::test]
async fn an_interrupt_stops_the_turn_through_codex() {
    let (stop, stopped) = oneshot::channel();
    let interrupter = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = stop.send(());
    });
    let run = run("interrupt me", Access::EditFiles, None, None, None, Some(stopped)).await;
    interrupter.await.unwrap();
    let turn = run.result.unwrap();
    assert_eq!(turn.result.status, "interrupted");
    let interrupt = method(&run.log, "turn/interrupt").expect("turn/interrupt sent");
    assert_eq!((interrupt["params"]["threadId"].as_str(), interrupt["params"]["turnId"].as_str()), (Some("thr-1"), Some("turn-1")));
}

#[tokio::test]
async fn what_the_person_says_mid_turn_steers_it() {
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send("not the footer".to_owned()).unwrap();
    let run = run("steer me", Access::EditFiles, None, None, Some(rx), None).await;
    assert!(run.activities.contains(&Activity::Steered("not the footer".into())));
    assert_eq!(run.result.unwrap().result.text, "Read: not the footer");
    let steer = method(&run.log, "turn/steer").unwrap();
    assert_eq!(steer["params"]["expectedTurnId"], "turn-1");
}

#[tokio::test]
async fn a_thread_is_continued_and_a_lost_one_started_afresh() {
    let kept = run("again", Access::ReadOnly, Some("thr-7"), None, None, None).await;
    assert_eq!(kept.result.unwrap().thread_id.as_deref(), Some("thr-7"));
    assert_eq!(method(&kept.log, "thread/resume").unwrap()["params"]["developerInstructions"], "You are qa.", "the charter as it is now");
    assert!(method(&kept.log, "thread/start").is_none());

    let lost = run("again", Access::ReadOnly, Some("thr-gone"), None, None, None).await;
    assert_eq!(lost.result.unwrap().thread_id.as_deref(), Some("thr-1"));
    assert!(method(&lost.log, "thread/start").is_some());
}

#[tokio::test]
async fn an_exhausted_plan_fails_the_turn_with_codexs_words() {
    let run = run("fail", Access::ReadOnly, None, None, None, None).await;
    assert!(matches!(run.result, Err(Error::Backend { message, code: Some(code) }) if message.contains("usage limit") && code == "usageLimitExceeded"));
}

#[tokio::test]
async fn signed_out_or_missing_says_how_to_fix_it() {
    let signed_out = wrapped("FAKE_CODEX_SIGNED_OUT=1");
    assert_eq!(codex_app::login_status(&signed_out).await.unwrap(), Some(SignIn::SignedOut));
    assert_eq!(codex_app::login_status(&fake()).await.unwrap(), Some(SignIn::ChatGpt));
    assert_eq!(codex_app::login_status("/nonexistent/codex").await.unwrap(), None);
    let cwd = scratch("cwd");
    let result = codex_app::run_turn(&signed_out, &cwd, TurnRequest { prompt: "hi", ..Default::default() }, None, None, None, |_| {}).await;
    assert!(matches!(result, Err(Error::CodexSignedOut)));
    assert!(Error::CodexSignedOut.to_string().contains("equipe login codex"));
    let missing = codex_app::run_turn("/nonexistent/codex", &cwd, TurnRequest { prompt: "hi", ..Default::default() }, None, None, None, |_| {}).await;
    assert!(matches!(missing, Err(Error::Login(m)) if m.contains("not installed")));
}

#[tokio::test]
async fn the_plans_usage_and_the_models_are_read_without_a_turn() {
    let cwd = scratch("cwd");
    let limits = codex_app::read_limits(&fake(), &cwd).await.unwrap();
    assert_eq!(limits.window(FIVE_HOURS).map(|w| (w.used_percent, w.resets_at)), Some((42.0, Some(1760630400))));
    assert_eq!(limits.window(SEVEN_DAYS).map(|w| w.used_percent), Some(5.0));
    assert_eq!(limits.plan_type.as_deref(), Some("pro"));
    assert_eq!(codex_app::list_models(&fake(), &cwd).await.unwrap(), ["gpt-6.1-sol", "gpt-6-luna", "gpt-6-astra"]);
}
