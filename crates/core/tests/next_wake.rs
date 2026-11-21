//! `Snapshot::next_wake_at`, end to end: what a machine that is paused while
//! nobody uses it (a hosted team's) reads to be woken in time for work the
//! daemon starts on its own. A sprint whose small items start after the
//! decide-and-tell window names that time, a few minutes early; the time is
//! kept across a restart; once it comes the sprint starts and the wake moves
//! on to its review; the person cancelling the schedule takes the wake away;
//! and nothing that waits on the person ever asks for one. A stand-in
//! `claude` plays the engine.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use equipe_core::next_wake::MARGIN;
use equipe_core::protocol::autonomy::{Kind, Level};
use equipe_core::protocol::routines::{Routine, Schedule};
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::sprint::{ItemStatus, Phase, Reading};
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::sprint::{self, data};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-next-wake-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A stand-in `claude` that answers "ok" to every turn.
fn claude() -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = scratch("bin").join("claude");
    let script = "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; exit 0; fi\n\
cat <<EOF\n{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-test\",\"session_id\":\"sess-1\"}\n\
{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}\nEOF\n";
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
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
    if tokio::time::timeout(Duration::from_secs(20), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

async fn tool(handle: &Handle, squad: &str, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { run: None, squad: squad.into(), member: member.into(), name: name.into(), arguments }).await
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// The field as the team agent reads it off the wire: `snapshot.next_wake_at`.
async fn wire_wake(handle: &Handle, squad: &str) -> Option<u64> {
    let snapshot = handle.snapshot(squad).await;
    let wire = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(wire.get("next_wake_at").and_then(|v| v.as_u64()), snapshot.next_wake_at, "{:?}", wire.get("next_wake_at"));
    snapshot.next_wake_at
}

struct Squad {
    handle: Handle,
    events: UnboundedReceiver<Event>,
    home: PathBuf,
    dir: PathBuf,
    claude: PathBuf,
}

/// A squad whose PM is started and nobody else is about: no routines, no
/// idle wake — nothing scheduled — and, with `data`, a product whose numbers
/// are read (so its sprint loop runs).
async fn squad(name: &str, data: bool) -> Squad {
    let claude = claude();
    let project = scratch("site").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, name, &project).unwrap();
    let journal = equipe_core::journal::Journal::for_squad(&home, name).unwrap();
    journal.set_routines(&[]).unwrap();
    journal.set_role_routine_added("pm").unwrap();
    journal.set_idle_wake_hours(0).unwrap();
    drop(journal);
    if data {
        data::register(name, Arc::new(data::Fake::new(|_, _, _| Ok(Reading { value: 10.0, sample: 10.0 }))));
    }
    let (handle, mut events) = runtime::start(config(&claude, &home));
    let pm = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some("pm".into()), squad: Some(name.into()) };
    handle.send(Command::Start { session: 1, spec: pm });
    until(&mut events, |e| matches!(e, Event::SessionStarted { session: 1, .. })).await;
    let dir = equipe_core::journal::squad_dir(&home, name);
    Squad { handle, events, home, dir, claude }
}

/// The PM proposes a sprint with one small, low-risk fix: on decide-and-tell
/// it starts by itself once the window passes (half an hour by default).
async fn propose(s: &mut Squad, name: &str) {
    let plan = json!({
        "north_star": { "name": "people who sign up each week", "metric": "event:signup" },
        "goal": "Fewer people stuck on the pricing page",
        "done_when": "The footer link works",
        "market_guess": "consumers",
        "items": [{ "kind": "fix", "title": "Fix the broken footer link", "change": "Point it at the pricing page.", "why": "Users wrote in about it", "expected": "fewer support mails", "impact": "low", "size": "S", "risk": "low" }],
    });
    let planned = tool(&s.handle, name, "pm", sprint::PLAN, plan).await;
    assert!(!planned.is_error, "{}", planned.text);
    until(&mut s.events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
}

async fn decide_and_tell(s: &mut Squad, name: &str) {
    s.handle.send(Command::SetAutonomy { squad: name.into(), kind: Kind::Sprint, level: Level::Tell });
    until(&mut s.events, |e| matches!(e, Event::AutonomyChanged { .. })).await;
}

// Acceptance 1–3: a sprint set to start in half an hour, nobody about.
#[tokio::test]
async fn a_sprint_due_in_half_an_hour_asks_for_the_machine_then_across_a_restart_and_moves_on_once_started() {
    let name = "wake-sprint";
    let mut s = squad(name, true).await;
    // At most the planning ask the loop's first look may make, asked again days later.
    let first = wire_wake(&s.handle, name).await;
    assert!(first.is_none_or(|at| at > now() + sprint::PLAN_AGAIN - MARGIN - 60), "nothing sooner scheduled yet: {first:?}");
    decide_and_tell(&mut s, name).await;
    let before = now();
    propose(&mut s, name).await;

    // 1–2. The snapshot names the start, a few minutes early.
    let auto_at = sprint::load(&s.dir).auto_at.expect("its items start on their own");
    assert!((before + 30 * 60..=now() + 30 * 60).contains(&auto_at), "after the half-hour window: {auto_at}");
    assert_eq!(wire_wake(&s.handle, name).await, Some(auto_at - MARGIN));

    // Kept across a restart: the schedule lives in the squad's own files.
    s.handle.shutdown().await;
    let (handle, events) = runtime::start(config(&s.claude, &s.home));
    (s.handle, s.events) = (handle, events);
    assert_eq!(wire_wake(&s.handle, name).await, Some(auto_at - MARGIN), "the same wake after a restart");

    // 3. Its time comes (moved here, rather than waited for): the sprint
    // starts, and the wake moves on to its review.
    let mut store = sprint::load(&s.dir);
    store.auto_at = Some(now() - 1);
    sprint::save(&s.dir, &store).unwrap();
    s.handle.minute_now().await;
    let store = sprint::load(&s.dir);
    let current = store.current().expect("the sprint under way");
    assert_eq!(current.phase, Phase::Active);
    assert!(store.items.iter().all(|i| i.status == ItemStatus::Started), "{:?}", store.items);
    assert_eq!(store.auto_at, None);
    let review = current.ends_at.expect("reviewed at its slot");
    assert!(review > now());
    assert_eq!(wire_wake(&s.handle, name).await, Some(review - MARGIN), "the review next, not a start already made");
    s.handle.shutdown().await;
    data::unregister(name);
}

// Acceptance 4: the schedule cancelled before the pause clears the wake.
#[tokio::test]
async fn cancelling_what_was_scheduled_takes_its_wake_away() {
    // A routine: saved, it names its next run; turned off, nothing.
    let name = "wake-routine";
    let mut s = squad(name, false).await;
    assert_eq!(wire_wake(&s.handle, name).await, None);
    let daily = Routine { id: 0, title: "Morning check".into(), prompt: "Look at last night's errors".into(), when: Schedule::Daily { hour: 9, minute: 0 }, on: true, last_run: None, next_run: None };
    s.handle.send(Command::SaveRoutine { squad: name.into(), routine: daily });
    let seen = until(&mut s.events, |e| matches!(e, Event::RoutinesChanged { .. })).await;
    let saved = seen.iter().rev().find_map(|e| if let Event::RoutinesChanged { routines, .. } = e { Some(routines[0].clone()) } else { None }).unwrap();
    let next = saved.next_run.expect("its next run");
    assert_eq!(wire_wake(&s.handle, name).await, Some(next - MARGIN));
    s.handle.send(Command::SaveRoutine { squad: name.into(), routine: Routine { on: false, ..saved } });
    until(&mut s.events, |e| matches!(e, Event::RoutinesChanged { .. })).await;
    assert_eq!(wire_wake(&s.handle, name).await, None, "turned off: left out, read as null");
    s.handle.shutdown().await;

    // A sprint's start: the person says not now, and nothing starts in half an hour.
    let name = "wake-not-now";
    let mut s = squad(name, true).await;
    decide_and_tell(&mut s, name).await;
    propose(&mut s, name).await;
    let auto_at = sprint::load(&s.dir).auto_at.unwrap();
    assert_eq!(wire_wake(&s.handle, name).await, Some(auto_at - MARGIN));
    let card = s.handle.snapshot(name).await.questions.last().expect("the sprint's card").id;
    s.handle.send(Command::DismissQuestions { squad: name.into(), id: card });
    until(&mut s.events, |e| matches!(e, Event::QuestionsClosed { id, .. } if *id == card)).await;
    let store = sprint::load(&s.dir);
    assert_eq!(store.sprints.last().map(|x| x.phase), Some(Phase::Declined));
    // What is left is the PM asked to plan again days later, not the start.
    let again = store.planning_asked.expect("asked again later") + sprint::PLAN_AGAIN;
    assert_eq!(wire_wake(&s.handle, name).await, Some(again - MARGIN));
    s.handle.shutdown().await;
    data::unregister(name);
}

// Acceptance 5's daemon side: the cloud decides whether a wake is allowed
// (plan, credits); the daemon never asks for one it wouldn't use — work that
// waits on the person doesn't start by itself.
#[tokio::test]
async fn work_that_waits_on_the_person_never_asks_for_the_machine() {
    let name = "wake-ask";
    let mut s = squad(name, true).await;
    // Sprints on "ask" (the default): the proposal waits for the person's tap.
    propose(&mut s, name).await;
    let store = sprint::load(&s.dir);
    assert_eq!((store.auto_at, store.proposed().map(|p| p.phase)), (None, Some(Phase::Proposed)));
    // A routine kept off.
    let off = Routine { id: 0, title: "Weekly report".into(), prompt: "Sum up the week".into(), when: Schedule::Weekly { weekday: 1, hour: 9, minute: 0 }, on: false, last_run: None, next_run: None };
    s.handle.send(Command::SaveRoutine { squad: name.into(), routine: off });
    until(&mut s.events, |e| matches!(e, Event::RoutinesChanged { .. })).await;
    assert_eq!(wire_wake(&s.handle, name).await, None);
    s.handle.shutdown().await;
    data::unregister(name);
}
