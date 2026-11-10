//! Helpers for a member whose engine has no subagents of its own — Codex on
//! another provider (Nebius Token Factory here) — driven the way a screen
//! and the member's MCP server drive the runtime, on the stand-in `codex`
//! (the providers' `tests/fake-codex`): no network, no key that works.
//!
//! What a helper's turn does depends on its brief, as the stand-in reads its
//! prompt: "findings" answers in a few lines, "interrupt" works until it is
//! stopped, "spend" reports a large token usage and works on.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::helpers::{self, Limits};
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Engine, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

const MODEL: &str = "nvidia/nemotron-mock-super";

/// Every test here runs helpers under these: short, to see them hit.
fn short_limits() {
    helpers::set_limits(Limits { time: Duration::from_secs(12), tokens: 50_000, wait: Duration::from_secs(30) });
    equipe_core::subagent_watch::set_stuck_after(Duration::from_secs(1));
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-hp-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn write_script(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The stand-in `codex`: what it is sent, and each run's command line.
fn fake_codex(env: &str) -> (PathBuf, PathBuf, PathBuf) {
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("../providers/tests/fake-codex/codex").canonicalize().unwrap();
    let logs = scratch("log");
    let (log, args) = (logs.join("sent.jsonl"), logs.join("args.jsonl"));
    let bin = scratch("bin").join("codex");
    write_script(&bin, &format!("#!/bin/sh\nFAKE_CODEX_LOG='{}' FAKE_CODEX_ARGS='{}' {env} exec '{}' \"$@\"\n", log.display(), args.display(), fake.display()));
    (bin, log, args)
}

fn no_claude() -> PathBuf {
    let bin = scratch("bin").join("claude");
    write_script(&bin, "#!/bin/sh\necho '{\"loggedIn\":false}'\n");
    bin
}

fn token_factory() -> equipe_core::nemotron::Config {
    equipe_core::nemotron::Config::from_vars(|name| match name {
        "NEBIUS_API_KEY" => Some("test-not-a-key".into()),
        "EQUIPE_NEBIUS_BASE_URL" => Some("http://127.0.0.1:9/v1/".into()),
        "EQUIPE_NEBIUS_MODEL" => Some(MODEL.into()),
        "EQUIPE_NEBIUS_WIRE" => Some("responses".into()),
        _ => None,
    })
    .unwrap()
}

fn config(codex: &Path, home: &Path, nemotron: Option<equipe_core::nemotron::Config>) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: no_claude().to_string_lossy().into_owned(),
        codex_binary: codex.to_string_lossy().into_owned(),
        persist: false,
        retention_days: 90,
        equipe_binary: Some("/opt/equipe".into()),
        npx: None,
        design_path: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        person_home: Some(home.join("person")),
        lang: equipe_core::i18n::Lang::En,
        nemotron,
        watch_disk: false,
    }
}

fn sent(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

/// The `thread/start`s of helpers, and of members, as the stand-in was sent them.
fn starts(log: &Path) -> (Vec<Value>, Vec<Value>) {
    sent(log)
        .into_iter()
        .filter(|m| m["method"] == "thread/start")
        .map(|m| m["params"].clone())
        .partition(|p| p["developerInstructions"].as_str().is_some_and(|i| i.starts_with("You are a helper")))
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
    if tokio::time::timeout(Duration::from_secs(60), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

/// A squad `acme` with member `backend` on `model` (Nemotron: no subagents of its own), at work on a long turn.
async fn at_work(model: Option<&str>, nemotron: bool) -> (Handle, UnboundedReceiver<Event>, PathBuf, PathBuf, PathBuf) {
    short_limits();
    let (codex, log, args) = fake_codex("");
    let cwd = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &cwd).unwrap();
    let (handle, mut events) = runtime::start(config(&codex, &home, nemotron.then(token_factory)));
    let spec = SessionSpec { engine: Some(Engine::Codex), model: model.map(str::to_owned), access: Access::ReadOnly, cwd: cwd.clone(), role: Some("backend".into()), squad: Some("acme".into()) };
    handle.send(Command::Start { session: 1, spec });
    until(&mut events, |e| matches!(e, Event::SessionStarted { session: 1, .. })).await;
    // The member works on until it is stopped.
    handle.send(Command::Submit { session: 1, text: "interrupt me when done".into(), about: None });
    until(&mut events, |e| matches!(e, Event::TextDelta { session: 1, .. })).await;
    (handle, events, log, args, cwd)
}

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall { run: None, squad: "acme".into(), member: "backend".into(), name: name.into(), arguments }
}

async fn spawn(handle: &Handle, brief: &str, what: &str) -> ToolResult {
    handle.tool(call(helpers::SPAWN_HELPER, json!({ "brief": brief, "what": what }))).await
}

async fn result(handle: &Handle, id: Option<&str>) -> ToolResult {
    let args = id.map_or_else(|| json!({}), |id| json!({ "id": id }));
    tokio::time::timeout(Duration::from_secs(60), handle.tool(call(helpers::HELPER_RESULT, args))).await.expect("helper_result answers")
}

#[tokio::test]
async fn only_a_member_with_no_subagents_of_its_own_is_offered_helpers() {
    // On Nemotron: the board's server carries the helpers, and may wait for them.
    let (handle, _events, log, _, _) = at_work(Some(MODEL), true).await;
    let (_, members) = starts(&log);
    let board = &members[0]["config"]["mcp_servers"]["equipe"];
    let args: Vec<&str> = board["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert!(args.contains(&"--helpers"), "{args:?}");
    assert_eq!(board["tool_timeout_sec"], helpers::tool_timeout_secs(), "past helper_result's longest wait");
    let charter = members[0]["developerInstructions"].as_str().unwrap();
    assert!(charter.contains("equipe gives you helpers instead") && !charter.contains("your Agent tool"), "{charter}");
    handle.send(Command::Interrupt { session: 1 });

    // On OpenAI, Codex keeps its own subagents: no helpers, none to call.
    let (handle, _events, log, _, _) = at_work(None, false).await;
    let (_, members) = starts(&log);
    let board = &members[0]["config"]["mcp_servers"]["equipe"];
    assert!(!board["args"].as_array().unwrap().iter().any(|a| a == "--helpers"), "{board}");
    assert!(board.get("tool_timeout_sec").is_none(), "its board server as it was: {board}");
    assert!(!members[0]["developerInstructions"].as_str().unwrap().contains("spawn_helper"));
    let refused = spawn(&handle, "look around", "look").await;
    assert!(refused.is_error && refused.text.contains("subagents of its own"), "{}", refused.text);
    handle.send(Command::Interrupt { session: 1 });
}

#[tokio::test]
async fn a_helper_reads_only_on_the_members_own_model_and_its_answer_comes_word_for_word() {
    let (handle, mut events, log, args, _) = at_work(Some(MODEL), true).await;
    let spawned = spawn(&handle, "List the findings about where sessions expire.", "find where sessions expire").await;
    assert!(!spawned.is_error && spawned.text.starts_with("Helper h1 is at work"), "{}", spawned.text);
    let answer = result(&handle, Some("h1")).await;
    let findings = "Sessions expire in `src/auth.rs:42` (`TTL = 3600`).\n\n- Refreshed by `src/auth.rs:57`.\n- Not checked: the mobile client, which isn't in this repository.";
    assert!(!answer.is_error, "{}", answer.text);
    assert!(answer.text.starts_with("Helper h1 (find where sessions expire) answered after "), "{}", answer.text);
    assert!(answer.text.ends_with(&format!(":\n\n{findings}")), "word for word, nothing after it: {}", answer.text);

    // Shown as the member's subagent: its step, and what it handed back.
    let seen = until(&mut events, |e| matches!(e, Event::SubagentEnded { .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::SubagentStarted { session: 1, id, what, .. } if id == "h1" && what == "find where sessions expire")), "{seen:#?}");
    assert!(seen.iter().any(|e| matches!(e, Event::SubagentStep { session: 1, id, step } if id == "h1" && step.starts_with("Read "))), "{seen:#?}");
    assert!(matches!(seen.last(), Some(Event::SubagentEnded { session: 1, result: Some(r), failed: false, .. }) if r == findings), "{seen:#?}");

    // Read-only, in the member's folder, on its model and provider, with no MCP server at all.
    let (helpers, members) = starts(&log);
    let start = &helpers[0];
    assert_eq!((start["sandbox"].as_str(), start["approvalPolicy"].as_str()), (Some("read-only"), Some("never")));
    assert_eq!(start["cwd"], members[0]["cwd"], "where the member works");
    assert_eq!(start["model"], MODEL);
    assert!(start["developerInstructions"].as_str().unwrap().starts_with("You are a helper for backend"));
    let servers = start["config"]["mcp_servers"].as_object().unwrap();
    assert!(servers.get("equipe").is_none(), "no board tools: it posts nothing");
    assert!(servers.values().all(|s| s["enabled"] == false), "the person's own are off: {servers:?}");
    assert_eq!(start["config"]["features"]["multi_agent"], false, "no subagents of its own either");
    let turn = sent(&log).into_iter().filter(|m| m["method"] == "turn/start").map(|m| m["params"].clone()).find(|p| p["input"][0]["text"].as_str().is_some_and(|t| t.contains("findings"))).unwrap();
    assert_eq!(turn["sandboxPolicy"], json!({ "type": "readOnly", "networkAccess": false }));
    assert_eq!(turn["input"][0]["text"], "List the findings about where sessions expire.", "its brief, as the member wrote it");
    let lines: Vec<Value> = std::fs::read_to_string(&args).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert!(lines.iter().filter(|l| l["args"][0] == "app-server").all(|l| l["args"].as_array().unwrap().iter().any(|a| a == "model_provider=\"nebius\"") && l["nebius_key"] == true), "the member's provider, its key by the environment");

    // Collected once.
    let again = result(&handle, Some("h1")).await;
    assert!(again.is_error && again.text.contains("No helper h1"), "{}", again.text);
    handle.send(Command::Interrupt { session: 1 });
}

#[tokio::test]
async fn three_work_at_once_the_rest_wait_and_each_is_stopped_at_its_time_limit() {
    let (handle, mut events, _, _, _) = at_work(Some(MODEL), true).await;
    for n in 1..=3 {
        let spawned = spawn(&handle, "Keep reading until you are interrupted.", &format!("part {n}")).await;
        assert!(spawned.text.starts_with(&format!("Helper h{n} is at work")), "{}", spawned.text);
    }
    let fourth = spawn(&handle, "Keep reading until you are interrupted.", "part 4").await;
    assert!(fourth.text.starts_with("Helper h4 waits its turn: 3 of yours are at work."), "{}", fourth.text);

    // Its time up, one hands in what it had and why; the fourth starts in its place.
    let answer = result(&handle, None).await;
    assert!(answer.text.contains("was stopped at its time limit") && answer.text.contains("Working on it"), "{}", answer.text);
    let seen = until(&mut events, |e| matches!(e, Event::SubagentStarted { id, .. } if id == "h4")).await;
    let started = seen.iter().filter(|e| matches!(e, Event::SubagentStarted { .. })).count();
    assert_eq!(started, 4, "never more than three at once: {seen:#?}");
    let ended_before = seen.iter().filter(|e| matches!(e, Event::SubagentEnded { .. })).count();
    assert!(ended_before >= 1, "{seen:#?}");
    handle.send(Command::Interrupt { session: 1 });
}

#[tokio::test]
async fn a_helper_past_its_tokens_is_stopped_and_what_they_all_used_is_the_members() {
    let (handle, mut events, _, _, _) = at_work(Some(MODEL), true).await;
    spawn(&handle, "Read everything and spend what it takes.", "read everything").await;
    let answer = result(&handle, Some("h1")).await;
    assert!(answer.text.contains("was stopped at its token limit, 50000 tokens") && answer.text.contains("Reading\n\nWorking on it"), "{}", answer.text);
    handle.send(Command::Interrupt { session: 1 });
    let ended = until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    let Some(Event::TurnEnded { usage: Some(usage), .. }) = ended.last() else { panic!("{ended:#?}") };
    assert!(usage.total() >= 90_500, "the helper's tokens count as the member's: {usage:?}");
}

#[tokio::test]
async fn stopping_the_members_turn_stops_its_helpers() {
    let (handle, mut events, log, _, _) = at_work(Some(MODEL), true).await;
    spawn(&handle, "Keep reading until you are interrupted.", "part 1").await;
    until(&mut events, |e| matches!(e, Event::SubagentStarted { .. })).await;
    let waiting = {
        let handle = handle.clone();
        tokio::spawn(async move { result(&handle, None).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    handle.send(Command::Interrupt { session: 1 });
    let answered = waiting.await.unwrap();
    assert!(answered.text.contains("Your turn ended, and your helpers with it"), "{}", answered.text);
    // The helper's own app-server is told to stop, as the member's is.
    let mut interrupts = 0;
    for _ in 0..50 {
        interrupts = sent(&log).iter().filter(|m| m["method"] == "turn/interrupt").count();
        if interrupts >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(interrupts >= 2, "the member's turn and its helper's: {interrupts}");
    // And no helper of a turn gone is collected after it.
    let after = result(&handle, None).await;
    assert_eq!(after.text, "You have no helpers out.");
}

#[tokio::test]
async fn the_person_stops_one_helper_alone_and_the_member_works_on() {
    let (handle, mut events, _, _, _) = at_work(Some(MODEL), true).await;
    spawn(&handle, "Keep reading until you are interrupted.", "part 1").await;
    until(&mut events, |e| matches!(e, Event::SubagentStarted { .. })).await;
    handle.send(Command::StopSubagent { session: 1, id: "h1".into() });
    let answer = result(&handle, Some("h1")).await;
    assert!(answer.text.contains("was stopped by the person"), "{}", answer.text);
    let seen = until(&mut events, |e| matches!(e, Event::SubagentEnded { .. })).await;
    assert!(!seen.iter().any(|e| matches!(e, Event::TurnEnded { .. })), "the member's turn goes on: {seen:#?}");
    handle.send(Command::Interrupt { session: 1 });
}

#[tokio::test]
async fn the_person_writing_lets_a_waiting_member_read_it_and_its_helpers_work_on() {
    let (handle, mut events, _, _, _) = at_work(Some(MODEL), true).await;
    spawn(&handle, "Keep reading until you are interrupted.", "part 1").await;
    until(&mut events, |e| matches!(e, Event::SubagentStarted { .. })).await;
    let waiting = {
        let handle = handle.clone();
        tokio::spawn(async move { result(&handle, None).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    handle.send(Command::Submit { session: 1, text: "Also look at the logout path.".into(), about: None });
    let answered = waiting.await.unwrap();
    assert!(answered.text.starts_with("The person sent you a message: read it now. Your helpers keep working (Still at work: h1 (part 1)"), "{}", answered.text);
    handle.send(Command::Interrupt { session: 1 });
}

#[tokio::test]
async fn a_quiet_helper_looks_stuck_and_is_never_stopped_for_it() {
    let (handle, mut events, _, _, _) = at_work(Some(MODEL), true).await;
    spawn(&handle, "Keep reading until you are interrupted.", "part 1").await;
    let seen = until(&mut events, |e| matches!(e, Event::SubagentStuck { .. })).await;
    assert!(matches!(seen.last(), Some(Event::SubagentStuck { session: 1, id, quiet_secs: Some(_) }) if id == "h1"), "{seen:#?}");
    handle.send(Command::Interrupt { session: 1 });
}

/// Codex's own subagents, on OpenAI: watched the same way, and the person stops one alone.
#[tokio::test]
async fn a_codex_subagent_gone_quiet_looks_stuck_and_the_person_stops_it_alone() {
    short_limits();
    let (codex, log, _) = fake_codex("");
    let cwd = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &cwd).unwrap();
    let (handle, mut events) = runtime::start(config(&codex, &home, None));
    let spec = SessionSpec { engine: Some(Engine::Codex), model: None, access: Access::ReadOnly, cwd, role: Some("backend".into()), squad: Some("acme".into()) };
    handle.send(Command::Start { session: 1, spec });
    until(&mut events, |e| matches!(e, Event::SessionStarted { session: 1, .. })).await;
    handle.send(Command::Submit { session: 1, text: "hand the logs to a subagent".into(), about: None });
    let seen = until(&mut events, |e| matches!(e, Event::SubagentStuck { .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::SubagentStarted { session: 1, id, .. } if id == "thr-sub")), "{seen:#?}");
    assert!(matches!(seen.last(), Some(Event::SubagentStuck { session: 1, id, quiet_secs: Some(_) }) if id == "thr-sub"), "{seen:#?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::SubagentEnded { .. })), "looking stuck stops nothing: {seen:#?}");
    handle.send(Command::StopSubagent { session: 1, id: "thr-sub".into() });
    let ended = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(ended.iter().any(|e| matches!(e, Event::SubagentEnded { id, .. } if id == "thr-sub")), "{ended:#?}");
    let text: String = ended.iter().filter_map(|e| if let Event::TextDelta { text, .. } = e { Some(text.as_str()) } else { None }).collect();
    assert!(text.contains("The subagent was interrupted. DONE"), "the member's turn went on: {text}");
    let interrupts: Vec<Value> = sent(&log).into_iter().filter(|m| m["method"] == "turn/interrupt").collect();
    assert_eq!(interrupts.len(), 1);
    assert_eq!(interrupts[0]["params"]["threadId"], "thr-sub", "only the subagent's turn");
}
