//! Several tickets at once under one member: each ready ticket runs in a
//! session of its own beside the member's conversation ([`equipe_core::lanes`]),
//! and queued work never adds members. Driven the way screens and engines
//! drive it, with a stand-in `claude`; nothing reaches a network.
//!
//! What each test pins, deterministically:
//! - N ready tickets for one role start as N runs of ONE member;
//! - the order members of a role take tickets in is fixed (fewest in hand,
//!   then who joined first) and the same on every run;
//! - a ticket owned by a member by name goes to that member only;
//! - how many run at once is the lane policy's (plan usage, never members),
//!   or the person's caps on max;
//! - a call from a run that no longer holds its ticket changes nothing;
//! - one run waiting, stopped or failing leaves its siblings as they were;
//! - a restart brings each run back exactly once.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use equipe_core::journal::Journal;
use equipe_core::protocol::rpc::{RunKey, ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Engine, Event, ExecutionState, PlanUsage, PlanWindow, Recovery, SessionSpec, SessionState, Snapshot, TicketStatus};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-lanes-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

/// A stand-in `claude`, signed in. Each ticket turn's prompt is kept in
/// `prompts.log`. While `hang` exists beside it a turn goes on until it is
/// stopped; otherwise it answers in a line and ends.
struct Engine0 {
    bin: PathBuf,
}

impl Engine0 {
    fn new(hang: bool) -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("bin");
        let bin = dir.join("claude");
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = "auth" ]; then echo '{{"loggedIn":true,"authMethod":"claude.ai"}}'; exit 0; fi
if [ "$2" = --input-format ] && IFS= read -r line; then set -- -p "$(printf '%s' "$line" | python3 -c 'import json,sys; print(json.load(sys.stdin)["message"]["content"], end="")')" "$@"; fi
case "$*" in *"You're on T-"*|*"equipe restarted while you worked"*) printf '%s\n<<end>>\n' "$*" >> '{log}';; esac
echo '{{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}}'
if [ -e '{hang}' ]; then exec sleep 30; fi
echo '{{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{{"input_tokens":3,"output_tokens":1}}}}'
"#,
            log = dir.join("prompts.log").display(),
            hang = dir.join("hang").display(),
        );
        let draft = dir.join("claude.draft");
        std::fs::write(&draft, script).unwrap();
        assert!(Process::new("cp").arg(&draft).arg(&bin).status().unwrap().success());
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let engine = Engine0 { bin };
        engine.hang(hang);
        engine
    }

    fn hang(&self, on: bool) {
        let marker = self.bin.with_file_name("hang");
        if on {
            std::fs::write(marker, "").unwrap();
        } else {
            let _ = std::fs::remove_file(marker);
        }
    }

    fn prompts(&self) -> Vec<String> {
        let log = std::fs::read_to_string(self.bin.with_file_name("prompts.log")).unwrap_or_default();
        log.split("<<end>>").map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned).collect()
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A home with the squad `acme` on a fresh repository.
fn squad() -> (PathBuf, PathBuf) {
    let repo = scratch("repo");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    (home, repo)
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

fn member(cwd: &Path, role: &str) -> SessionSpec {
    SessionSpec { engine: Some(Engine::Claude), model: None, access: Access::EditFiles, cwd: cwd.to_owned(), role: Some(role.into()), squad: Some("acme".into()) }
}

async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, spec: SessionSpec) {
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
}

async fn tool(handle: &Handle, member: &str, run: Option<RunKey>, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { run, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

async fn ticket_for(handle: &Handle, member: &str, title: &str) {
    let made = tool(handle, "pm", None, "create_ticket", json!({ "title": title, "body": "-", "owner": member, "kind": "code" })).await;
    assert!(!made.is_error, "{made:?}");
}

/// The runs a snapshot lists, as `(member session, ticket, run session, generation)`, by ticket.
fn runs(snapshot: &Snapshot) -> Vec<(u64, u32, u64, u64)> {
    let mut runs: Vec<(u64, u32, u64, u64)> = snapshot.sessions.iter().filter_map(|s| s.run.map(|r| (r.of, r.ticket, s.session, r.generation))).collect();
    runs.sort_by_key(|r| r.1);
    runs
}

/// The members a snapshot lists: each once, however many tickets it runs.
fn members(snapshot: &Snapshot) -> Vec<String> {
    snapshot.sessions.iter().filter(|s| s.run.is_none()).filter_map(|s| s.member.as_ref().map(|m| m.name.clone())).collect()
}

fn key(snapshot: &Snapshot, ticket: u32) -> RunKey {
    let (_, ticket, session, generation) = *runs(snapshot).iter().find(|r| r.1 == ticket).unwrap_or_else(|| panic!("T-{ticket} runs: {:#?}", runs(snapshot)));
    RunKey { session, ticket, generation }
}

async fn started(events: &mut UnboundedReceiver<Event>, count: usize) -> Vec<(u64, u32)> {
    let mut got = Vec::new();
    while got.len() < count {
        let seen = until(events, |e| matches!(e, Event::WorkStarted { of: Some(_), .. })).await;
        if let Some(Event::WorkStarted { of: Some(of), ticket, .. }) = seen.last() {
            got.push((*of, *ticket));
        }
    }
    got
}

#[tokio::test]
async fn three_ready_tickets_for_one_role_start_as_three_runs_of_one_member() {
    let engine = Engine0::new(true);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    for title in ["Hero", "Footer", "Pricing"] {
        ticket_for(&handle, "frontend", title).await;
    }
    assert_eq!(started(&mut events, 3).await, [(2, 1), (2, 2), (2, 3)]);
    let snapshot = handle.snapshot("acme").await;
    assert_eq!(members(&snapshot), ["pm", "frontend"], "no member added: {:#?}", snapshot.sessions);
    let runs = runs(&snapshot);
    assert_eq!(runs.iter().map(|r| (r.0, r.1, r.3)).collect::<Vec<_>>(), [(2, 1, 1), (2, 2, 1), (2, 3, 1)], "one run each, generation 1, all of frontend");
    let sessions: std::collections::HashSet<u64> = runs.iter().map(|r| r.2).collect();
    assert_eq!(sessions.len(), 3, "each run its own session");
    // Each in a worktree of its own, all at work, each ticket in progress for frontend.
    let worktrees: std::collections::HashSet<PathBuf> = snapshot.tickets.iter().filter_map(|t| t.worktree.clone()).collect();
    assert_eq!(worktrees.len(), 3);
    for t in &snapshot.tickets {
        assert_eq!((t.status, t.assignee.as_deref()), (TicketStatus::InProgress, Some("frontend")));
        let run = t.execution.as_ref().unwrap();
        assert_eq!((run.state, run.session), (ExecutionState::Active, runs.iter().find(|r| r.1 == t.id).unwrap().2));
    }
    assert!(snapshot.sessions.iter().filter(|s| s.run.is_some()).all(|s| s.state == SessionState::Working));
    handle.shutdown().await;
}

#[tokio::test]
async fn a_role_spreads_its_tickets_by_who_holds_least_then_who_joined_first() {
    let engine = Engine0::new(true);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    for id in [2, 3, 4] {
        add(&handle, &mut events, id, member(&repo, "backend")).await;
    }
    for n in 1..=7 {
        ticket_for(&handle, "backend", &format!("Endpoint {n}")).await;
    }
    // backend, backend-2, backend-3 in the order they joined: 3 / 2 / 2.
    assert_eq!(started(&mut events, 7).await, [(2, 1), (3, 2), (4, 3), (2, 4), (3, 5), (4, 6), (2, 7)]);
    assert_eq!(members(&handle.snapshot("acme").await), ["pm", "backend", "backend-2", "backend-3"]);
    handle.shutdown().await;
}

#[tokio::test]
async fn a_ticket_owned_by_name_goes_to_that_member_only() {
    let engine = Engine0::new(true);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    add(&handle, &mut events, 3, member(&repo, "frontend")).await;
    ticket_for(&handle, "frontend-2", "Mine").await;
    ticket_for(&handle, "frontend-2", "Mine too").await;
    ticket_for(&handle, "frontend", "The role's").await;
    assert_eq!(started(&mut events, 3).await, [(3, 1), (3, 2), (2, 3)], "frontend-2's own, then the role's to whoever holds least");
    handle.shutdown().await;
}

#[tokio::test]
async fn how_many_run_at_once_follows_the_plan_not_the_number_of_members() {
    let engine = Engine0::new(true);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    for n in 1..=5 {
        ticket_for(&handle, "frontend", &format!("Page {n}")).await;
    }
    // The policy's three at once (`lanes::LANES_PER_MEMBER`); the rest wait their turn.
    assert_eq!(started(&mut events, 3).await, [(2, 1), (2, 2), (2, 3)]);
    tokio::time::sleep(Duration::from_millis(400)).await;
    let snapshot = handle.snapshot("acme").await;
    assert_eq!(runs(&snapshot).len(), 3);
    assert_eq!(snapshot.tickets.iter().filter(|t| t.status == TicketStatus::Todo).count(), 2);
    assert_eq!(members(&snapshot), ["pm", "frontend"], "no one added for the queue");
    // The plan nearly used up: one at a time. One run ends; nothing new starts.
    let window = PlanWindow { used_percent: 95, resets_at: 4_000_000_000 };
    handle.usage_read(Engine::Claude, PlanUsage { five_hour: Some(window), seven_day: None });
    until(&mut events, |e| matches!(e, Event::PlanUsage { .. })).await;
    handle.send(Command::Cancel { squad: "acme".into(), ticket: 1 });
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Cancelled)).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(runs(&handle.snapshot("acme").await).iter().map(|r| r.1).collect::<Vec<_>>(), [2, 3], "over the limit: nothing more starts");
    // With room again, the next goes.
    let window = PlanWindow { used_percent: 40, resets_at: 4_000_000_000 };
    handle.usage_read(Engine::Claude, PlanUsage { five_hour: Some(window), seven_day: None });
    until(&mut events, |e| matches!(e, Event::PlanUsage { .. })).await;
    handle.send(Command::Cancel { squad: "acme".into(), ticket: 2 });
    assert_eq!(started(&mut events, 2).await, [(2, 4), (2, 5)]);
    handle.shutdown().await;
}

#[tokio::test]
async fn on_max_the_persons_caps_hold_whatever_the_plan_and_auto_brings_the_plan_back() {
    use equipe_core::protocol::lanes::{Parallel, ParallelMode};
    let engine = Engine0::new(true);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    // The plan nearly used up: on auto, one at a time.
    let window = PlanWindow { used_percent: 95, resets_at: 4_000_000_000 };
    handle.usage_read(Engine::Claude, PlanUsage { five_hour: Some(window), seven_day: None });
    until(&mut events, |e| matches!(e, Event::PlanUsage { .. })).await;
    assert_eq!(handle.snapshot("acme").await.parallel, Parallel::default(), "never set: auto");
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    for n in 1..=6 {
        ticket_for(&handle, "frontend", &format!("Page {n}")).await;
    }
    assert_eq!(started(&mut events, 1).await, [(2, 1)]);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(runs(&handle.snapshot("acme").await).len(), 1);
    // Max at four per member: four at once, the plan's usage left out.
    handle.send(Command::SetParallel { squad: "acme".into(), parallel: Parallel::max(4, 16) });
    until(&mut events, |e| matches!(e, Event::ParallelChanged { parallel, .. } if parallel.is_max())).await;
    assert_eq!(started(&mut events, 3).await, [(2, 2), (2, 3), (2, 4)]);
    tokio::time::sleep(Duration::from_millis(400)).await;
    let snapshot = handle.snapshot("acme").await;
    assert_eq!(runs(&snapshot).len(), 4, "never past the cap");
    assert_eq!(snapshot.parallel, Parallel::max(4, 16));
    assert_eq!(members(&snapshot), ["pm", "frontend"], "no one added for the queue");
    assert_eq!(Journal::for_squad(&home, "acme").unwrap().parallel(), Parallel::max(4, 16), "kept");
    // Out of bounds from an old or careless screen: held within them.
    handle.send(Command::SetParallel { squad: "acme".into(), parallel: Parallel { per_member: 0, ..Parallel::max(4, 99) } });
    let told = until(&mut events, |e| matches!(e, Event::ParallelChanged { .. })).await;
    assert!(matches!(told.last(), Some(Event::ParallelChanged { parallel, .. }) if *parallel == Parallel::max(1, 32)), "{told:#?}");
    // Back on auto, the plan rules again: one run ends and nothing new starts.
    let auto = Parallel { mode: ParallelMode::Auto, ..Parallel::max(4, 16) };
    handle.send(Command::SetParallel { squad: "acme".into(), parallel: auto });
    until(&mut events, |e| matches!(e, Event::ParallelChanged { parallel, .. } if !parallel.is_max())).await;
    handle.send(Command::Cancel { squad: "acme".into(), ticket: 1 });
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Cancelled)).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(runs(&handle.snapshot("acme").await).iter().map(|r| r.1).collect::<Vec<_>>(), [2, 3, 4], "over auto's limit: nothing more starts");
    assert_eq!(handle.snapshot("acme").await.parallel, auto, "max's caps kept for next time");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_call_from_a_run_that_no_longer_holds_its_ticket_changes_nothing() {
    let engine = Engine0::new(false);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    ticket_for(&handle, "frontend", "Hero").await;
    started(&mut events, 1).await;
    let old = key(&handle.snapshot("acme").await, 1);
    // Its turn ended without a hand-in; past its lease it stalls, and the person resumes it.
    for _ in 0..100 {
        if handle.snapshot("acme").await.sessions.iter().any(|s| s.session == old.session && s.state == SessionState::Ready) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    handle.check_leases_at(now() + 1_000).await;
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.execution.as_ref().is_some_and(|x| x.state == ExecutionState::Stalled))).await;
    engine.hang(true);
    handle.send(Command::Requeue { squad: "acme".into(), ticket: 1, operation: "card:1".into(), how: Recovery::Resume });
    started(&mut events, 1).await;
    let snapshot = handle.snapshot("acme").await;
    let new = key(&snapshot, 1);
    assert_eq!((old.generation, new.generation), (1, 2));
    assert_ne!(old.session, new.session);
    let before = snapshot.tickets.iter().find(|t| t.id == 1).cloned().unwrap();

    // Generation 1 speaks after generation 2 began: every state-changing call is refused.
    for (name, args) in [
        ("estimate", json!({ "done_in_minutes": 5 })),
        ("block_on_person", json!({ "needs": ["a token"] })),
        ("submit_for_review", json!({ "summary": "done", "changes_screens": false })),
        ("comment", json!({ "id": 1, "text": "late" })),
        ("ask_decision", json!({ "questions": [{ "question": "Which?", "options": [{ "label": "A" }, { "label": "B" }] }] })),
        ("create_ticket", json!({ "title": "From a dead run", "body": "-" })),
    ] {
        let said = tool(&handle, "frontend", Some(old), name, args).await;
        assert!(said.is_error && said.text.contains("is over"), "{name}: {said:?}");
    }
    let asked = tool(&handle, "frontend", Some(old), "approve", json!({ "tool_name": "Bash", "input": { "command": "rm -rf build" }, "tool_use_id": "t" })).await;
    assert_eq!(serde_json::from_str::<serde_json::Value>(&asked.text).unwrap()["behavior"], "deny", "{asked:?}");
    // Reading is still fine.
    assert!(!tool(&handle, "frontend", Some(old), "board", json!({})).await.is_error);
    let after = handle.snapshot("acme").await;
    let ticket = after.tickets.iter().find(|t| t.id == 1).cloned().unwrap();
    assert_eq!((ticket.status, ticket.note.clone(), ticket.execution.clone()), (before.status, before.note.clone(), before.execution.clone()), "the ticket as it was");
    assert_eq!(after.tickets.len(), 1, "nothing created");
    assert!(after.approvals.is_empty() && after.questions.is_empty(), "nothing put to the person");
    let run = after.sessions.iter().find(|s| s.session == new.session).unwrap();
    assert_eq!(run.estimate, None, "the live run's estimate untouched");
    let stale: Vec<_> = Journal::for_squad(&home, "acme").unwrap().ticket_history(1).unwrap().into_iter().filter(|(_, kind, _)| kind == "stale call").collect();
    assert_eq!(stale.len(), 7, "each refusal on the ticket's record: {stale:#?}");
    // The current run's own call is taken.
    let said = tool(&handle, "frontend", Some(new), "estimate", json!({ "done_in_minutes": 5 })).await;
    assert!(!said.is_error, "{said:?}");
    handle.shutdown().await;
}

#[tokio::test]
async fn one_run_waiting_or_stopped_leaves_its_siblings_as_they_were() {
    let engine = Engine0::new(true);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    for title in ["A", "B", "C"] {
        ticket_for(&handle, "frontend", title).await;
    }
    started(&mut events, 3).await;
    let snapshot = handle.snapshot("acme").await;
    let (a, b, c) = (key(&snapshot, 1), key(&snapshot, 2), key(&snapshot, 3));

    // A can't go on without the person; its turn is stopped, so the wait lands now.
    let blocked = tool(&handle, "frontend", Some(a), "block_on_person", json!({ "needs": ["a token"] })).await;
    assert!(!blocked.is_error, "{blocked:?}");
    handle.send(Command::Interrupt { session: a.session });
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Waiting)).await;
    let after = handle.snapshot("acme").await;
    assert_eq!(runs(&after).iter().map(|r| (r.1, r.2, r.3)).collect::<Vec<_>>(), [(2, b.session, 1), (3, c.session, 1)], "B and C go on as they were");
    assert!(after.sessions.iter().filter(|s| s.run.is_some()).all(|s| s.state == SessionState::Working));
    assert_eq!(after.tickets.iter().filter(|t| t.status == TicketStatus::InProgress).count(), 2);

    // B is stopped: only B rests; C works on.
    handle.send(Command::Interrupt { session: b.session });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session, .. } if *session == b.session)).await;
    let after = handle.snapshot("acme").await;
    let state = |s: u64| after.sessions.iter().find(|x| x.session == s).map(|x| x.state);
    assert_eq!((state(b.session), state(c.session)), (Some(SessionState::Ready), Some(SessionState::Working)));
    let exec = |id: u32| after.tickets.iter().find(|t| t.id == id).and_then(|t| t.execution.clone()).map(|e| (e.state, e.generation));
    assert_eq!((exec(2), exec(3)), (Some((ExecutionState::Active, 1)), Some((ExecutionState::Active, 1))));
    // Closing a run from a screen stops it; the member stays on the team.
    handle.send(Command::Close { session: c.session });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session, .. } if *session == c.session)).await;
    assert_eq!(members(&handle.snapshot("acme").await), ["pm", "frontend"]);
    handle.shutdown().await;
}

#[tokio::test]
async fn a_restart_brings_each_run_back_exactly_once() {
    let engine = Engine0::new(false);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    ticket_for(&handle, "frontend", "Hero").await;
    ticket_for(&handle, "frontend", "Footer").await;
    started(&mut events, 2).await;
    let before = runs(&handle.snapshot("acme").await);
    assert_eq!(before.len(), 2);
    handle.shutdown().await;
    drop(events);

    let (handle, _events) = runtime::start(config(&engine.bin, &home));
    for _ in 0..200 {
        let restarted = engine.prompts().iter().filter(|p| p.contains("equipe restarted while you worked")).count();
        if restarted >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = handle.snapshot("acme").await;
    let runs_after = runs(&after);
    assert_eq!(runs_after.iter().map(|r| (r.0, r.1, r.3)).collect::<Vec<_>>(), [(2, 1, 2), (2, 2, 2)], "each back once, under generation 2");
    assert!(runs_after.iter().all(|r| !before.iter().any(|b| b.2 == r.2)), "fresh run sessions");
    assert_eq!(members(&after), ["pm", "frontend"], "one member, one conversation of its own");
    for id in [1, 2] {
        let told = engine.prompts().iter().filter(|p| p.contains(&format!("equipe restarted while you worked on T-{id}"))).count();
        assert_eq!(told, 1, "T-{id} carried on exactly once");
        let t = after.tickets.iter().find(|t| t.id == id).unwrap();
        assert_eq!(t.execution.as_ref().map(|e| (e.state, e.generation)), Some((ExecutionState::Active, 2)));
    }
    handle.shutdown().await;
}
