//! Members on the person's own Codex CLI, driven the way a screen drives the
//! runtime. A stand-in `codex app-server` (the providers' `tests/fake-codex`)
//! replays recorded turns: no network, no sign-in.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::protocol::{Access, Command, CompactTrigger, Decision, Engine, Event, PlanState, SessionSpec, TurnOutcome};
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
    // The plan's every step travels beside its line.
    let steps = turn.iter().find_map(|e| if let Event::Plan { steps, .. } = e { Some(steps.clone()) } else { None }).expect("the plan's steps");
    assert_eq!(steps.iter().map(|s| (s.text.as_str(), s.state)).collect::<Vec<_>>(), [("Add hello.txt", PlanState::Done), ("Run the tests", PlanState::Active)]);
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
async fn codex_compacting_its_thread_is_said_on_every_screen() {
    let (codex, _) = fake_codex("");
    let (handle, mut events) = runtime::start(config(&codex, &scratch("home")));
    add(&handle, &mut events, 1, SessionSpec { engine: Some(Engine::Codex), model: None, access: Access::ReadOnly, cwd: scratch("cwd"), role: None, squad: None }).await;
    handle.send(Command::Submit { session: 1, text: "compact".into(), about: None });
    let turn = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    let began = turn.iter().position(|e| matches!(e, Event::Compacting { trigger: CompactTrigger::Auto, estimate_ms: 120_000, .. })).expect("compacting");
    let done = turn.iter().position(|e| matches!(e, Event::Compacted { .. })).expect("compacted");
    assert!(began < done, "{turn:#?}");
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
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Always, words: None, said: None });
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
    let instructions = start["developerInstructions"].as_str().unwrap();
    assert!(instructions.starts_with("You are qa, the QA engineer"));
    // Its own subagents, by Codex's tool: unset, Codex is given nothing and runs its own 6, as its charter says.
    assert!(start["config"].get("agents").is_none(), "{start}");
    let flat = instructions.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("(your `spawn_agent` tool)") && flat.contains("at most 6 subagents at once: for more, start up to 6") && !flat.contains("Agent tool"), "{flat}");
    // Its skills: an index of absolute paths it reads by, never Codex's own skill folders.
    let review = home.join("skills").join("test");
    let path = instructions.split('`').find(|p| p.ends_with("/plan-review/SKILL.md")).unwrap_or_else(|| panic!("{instructions}"));
    assert!(Path::new(path).is_absolute() && Path::new(path).starts_with(&review), "{path}");
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.starts_with("---\nname: plan-review\n") && text.contains("say whether it can be tested"), "{text}");
    assert!(instructions.contains("read the matching skill's `SKILL.md` at its path") && !instructions.contains("say whether it can be tested"), "{instructions}");
}

#[tokio::test]
async fn a_codex_member_runs_the_squads_number_of_subagents_once_the_person_sets_one() {
    let (codex, log) = fake_codex("");
    let cwd = scratch("site");
    let home = scratch("home");
    squads::create(&home, "acme", &cwd).unwrap();
    let (handle, mut events) = runtime::start(config(&codex, &home));
    add(&handle, &mut events, 1, on_codex(&cwd, "qa")).await;
    handle.send(Command::SetSubagentsAtOnce { squad: "acme".into(), at_once: Some(4) });
    until(&mut events, |e| matches!(e, Event::SubagentsAtOnceChanged { .. })).await;
    handle.send(Command::Submit { session: 1, text: "survey it".into(), about: None });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    let start = &params_of(&log, "thread/start")[0];
    assert_eq!(start["config"]["agents"]["max_concurrent_threads_per_session"], 4, "{start}");
    let flat = start["developerInstructions"].as_str().unwrap().split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("Your engine runs at most 4 subagents at once: for more, start up to 4"), "{flat}");
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
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Deny, words: None, said: None });
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

/// What each turn the stand-in started was asked, in order.
fn prompts(log: &Path) -> Vec<String> {
    params_of(log, "turn/start").iter().map(|p| p["input"].as_array().into_iter().flatten().filter_map(|i| i["text"].as_str()).collect()).collect()
}

/// Wait until a turn the stand-in started was asked `wanted`; that prompt.
async fn asked(log: &Path, wanted: &str) -> String {
    for _ in 0..400 {
        if let Some(p) = prompts(log).into_iter().rev().find(|p| p.contains(wanted)) {
            return p;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no turn was asked {wanted:?}: {:#?}", prompts(log));
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// A Codex member's turn on a ticket that ends — completed — with no hand-in,
/// as seen on a squad moved to Codex: a yes it asked of the person with
/// `ask_person` outlived Codex's wait for the call, and the turn went on to
/// its end with the card still open. Its run is held for the person, never
/// stalled; their answer, when it comes, carries it on as a turn of its own;
/// and a turn that then ends without a hand-in is reminded how to finish, as
/// on Claude Code. Codex itself is told to wait for such calls as long as the
/// card stays open.
#[tokio::test]
async fn a_codex_turn_that_ends_without_a_hand_in_is_held_for_the_persons_answer_then_carried_on_never_stalled() {
    use equipe_core::protocol::rpc::{RunKey, ToolCall};
    use equipe_core::protocol::{ExecutionState, TicketStatus};
    let (codex, log) = fake_codex("");
    let cwd = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &cwd).unwrap();
    let mut config = config(&codex, &home);
    config.equipe_binary = Some("/opt/equipe".into());
    config.persist = true;
    let (handle, mut events) = runtime::start(config);
    add(&handle, &mut events, 1, on_codex(&cwd, "pm")).await;
    add(&handle, &mut events, 2, on_codex(&cwd, "backend")).await;
    let call = |member: &str, run: Option<RunKey>, name: &str, arguments: Value| ToolCall { run, squad: "acme".into(), member: member.into(), name: name.into(), arguments };
    let made = handle.tool(call("pm", None, "create_ticket", serde_json::json!({ "title": "Wire the export slowly", "body": "Do it.", "owner": "backend" }))).await;
    assert!(!made.is_error, "{made:?}");
    let started = until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(2), ticket: 1, .. })).await;
    let run = started.iter().find_map(|e| if let Event::WorkStarted { session, of: Some(2), .. } = e { Some(*session) } else { None }).unwrap();
    asked(&log, "You're on T-1").await;
    let ticket = |handle: &Handle| {
        let handle = handle.clone();
        async move { handle.snapshot("acme").await.tickets.into_iter().find(|t| t.id == 1).expect("on the board") }
    };

    // Codex waits for a call to the board past any card the person may hold open.
    let member_start = params_of(&log, "thread/start").into_iter().find(|p| p["config"]["mcp_servers"]["equipe"]["args"].as_array().is_some_and(|a| a.iter().any(|x| x == "backend"))).expect("the run's thread");
    let board = &member_start["config"]["mcp_servers"]["equipe"];
    assert_eq!(board["tool_timeout_sec"], runtime::board_call_timeout_secs(), "{board}");
    assert!(runtime::board_call_timeout_secs() > 30 * 60, "past the half hour a card waits for the person");

    // Mid-turn, it asks the person's yes — and the call is given up before
    // they answer, as Codex gives a call up after its timeout.
    let key = RunKey { session: run, ticket: 1, generation: 1 };
    let gave_up = tokio::time::timeout(
        Duration::from_millis(300),
        handle.tool(call("backend", Some(key), "ask_person", serde_json::json!({ "action": "Send the weekly report to the client", "why": "It goes out in your name" }))),
    )
    .await;
    assert!(gave_up.is_err(), "the call waits on the person: {gave_up:?}");
    let seen = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let approval = seen.iter().find_map(|e| if let Event::ApprovalRequested { approval, .. } = e { Some(approval.clone()) } else { None }).unwrap();
    assert_eq!((approval.session, approval.ticket), (run, Some(1)));

    // Its turn ends, completed, without a hand-in: held for the person's answer, not reminded.
    let ended = until(&mut events, |e| matches!(e, Event::TurnEnded { session, .. } if *session == run)).await;
    assert!(matches!(ended.last(), Some(Event::TurnEnded { outcome: TurnOutcome::Completed, .. })), "{ended:#?}");
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(!prompts(&log).iter().any(|p| p.contains("ended without ending the work")), "{:#?}", prompts(&log));
    // However late the check, a run whose card waits on the person is not stalled.
    handle.check_leases_at(now() + 1_000).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(ticket(&handle).await.execution.map(|e| e.state), Some(ExecutionState::Active));

    // The person says no: the answer reaches the run as a turn of its own.
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Deny, words: None, said: None });
    let late = asked(&log, "They have answered now").await;
    for wanted in ["Send the weekly report to the client", "The person said no", "submit_for_review", "block_on_person", "hand_back"] {
        assert!(late.contains(wanted), "{wanted} in: {late}");
    }
    // That turn ends without a hand-in too: reminded how to finish, as on Claude Code.
    let nudge = asked(&log, "ended without ending the work").await;
    assert!(nudge.contains("submit_for_review"), "{nudge}");
    // Ended so again: to the PM — never stalled on the way.
    for _ in 0..200 {
        if ticket(&handle).await.status == TicketStatus::Proposed {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let back = ticket(&handle).await;
    assert_eq!(back.status, TicketStatus::Proposed, "{back:#?}");
    assert!(back.handed_back(), "{back:#?}");
    assert_ne!(back.execution.map(|e| e.state), Some(ExecutionState::Stalled));
    handle.shutdown().await;
}
