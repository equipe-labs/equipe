//! The squad's heartbeat, driven the way a screen and an engine drive it: a
//! new squad's weekly check-in, on; a routine the PM proposes, saved only on
//! the person's click; and the PM woken once over a board left empty — not
//! twice in a day, and never once the person turned it off.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::journal::Journal;
use equipe_core::protocol::routines::{RoutineProposal, Schedule};
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const DAY: u64 = 24 * 60 * 60;

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-heartbeat-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A stand-in `claude`: signed in, and a turn takes a long while.
fn busy_claude() -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = scratch("bin").join("claude");
    let script = "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; exit 0; fi\nexec sleep 30\n";
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
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        person_home: Some(home.join("person")),
        lang: equipe_core::i18n::Lang::Ko,
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// A home where the squad `acme` works on one project, and that project.
fn squad_home() -> (PathBuf, PathBuf) {
    let project = scratch("site").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    (home, project)
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
    if tokio::time::timeout(Duration::from_secs(15), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

async fn start(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, role: &str, project: &Path) {
    let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.to_owned(), role: Some(role.into()), squad: Some("acme".into()) };
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

fn proposed(events: &[Event]) -> RoutineProposal {
    events.iter().find_map(|e| if let Event::RoutineProposed { proposal, .. } = e { Some(proposal.clone()) } else { None }).expect("proposed")
}

fn idle_wakes(events: &[Event]) -> usize {
    events.iter().filter(|e| matches!(e, Event::UserMessage { text, .. } if text.starts_with("[Idle · "))).count()
}

#[tokio::test]
async fn a_new_squad_starts_with_its_weekly_check_in_on() {
    let (home, _project) = squad_home();
    let (handle, _events) = runtime::start(config(&busy_claude(), &home));
    let routines = handle.snapshot("acme").await.routines;
    assert_eq!(routines.len(), 1, "{routines:?}");
    let check_in = &routines[0];
    assert_eq!((check_in.id, check_in.when, check_in.on), (1, Schedule::Weekly { weekday: 1, hour: 9, minute: 0 }, true));
    assert!(check_in.next_run.is_some_and(|at| at > now() && at <= now() + 7 * DAY), "{check_in:?}");
    assert_eq!(handle.snapshot("acme").await.idle_wake_hours, Some(24));
    handle.shutdown().await;
}

#[tokio::test]
async fn the_pm_proposes_a_routine_and_only_the_persons_click_turns_it_on() {
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&busy_claude(), &home));
    start(&handle, &mut events, 1, "pm", &project).await;
    start(&handle, &mut events, 2, "qa", &project).await;
    let weekly = json!({ "title": "의존성 점검", "prompt": "업데이트할 의존성을 찾아 줘", "every": "weekly", "weekday": 5, "hour": 16, "reason": "보안 패치가 밀려요" });

    // Only the PM proposes; a bad call is said why.
    let refused = tool(&handle, "qa", "propose_routine", weekly.clone()).await;
    assert!(refused.is_error && refused.text.contains("Only the PM"), "{refused:?}");
    let odd = tool(&handle, "pm", "propose_routine", json!({ "title": "x", "prompt": "y", "every": "weekly", "hour": 9 })).await;
    assert!(odd.is_error && odd.text.contains("weekday"), "{odd:?}");

    // Proposed: a card, nothing saved yet, and a screen attaching sees it waiting.
    let said = tool(&handle, "pm", "propose_routine", weekly.clone()).await;
    assert!(!said.is_error, "{said:?}");
    let card = proposed(&until(&mut events, |e| matches!(e, Event::RoutineProposed { .. })).await);
    assert_eq!((card.routine.title.as_str(), card.routine.when, card.by.as_str(), card.reason.as_str()), ("의존성 점검", Schedule::Weekly { weekday: 5, hour: 16, minute: 0 }, "pm", "보안 패치가 밀려요"));
    let snapshot = handle.snapshot("acme").await;
    assert_eq!(snapshot.routine_proposals, std::slice::from_ref(&card));
    assert_eq!(snapshot.routines.len(), 1, "only the weekly check-in");
    let again = tool(&handle, "pm", "propose_routine", weekly.clone()).await;
    assert!(again.is_error && again.text.contains("already waits"), "{again:?}");

    // The person's click: saved, on, its next run ahead; the card closes.
    handle.send(Command::ApproveRoutine { squad: "acme".into(), id: card.id });
    let seen = until(&mut events, |e| matches!(e, Event::RoutineProposalClosed { .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::RoutineProposalClosed { id, approved: true, .. } if *id == card.id)), "{seen:#?}");
    let snapshot = handle.snapshot("acme").await;
    let saved = snapshot.routines.iter().find(|r| r.title == "의존성 점검").expect("saved");
    assert!(saved.on && saved.id == 2 && saved.next_run.is_some(), "{saved:?}");
    assert!(snapshot.routine_proposals.is_empty());
    // Approving it twice saves nothing more.
    handle.send(Command::ApproveRoutine { squad: "acme".into(), id: card.id });
    assert_eq!(handle.snapshot("acme").await.routines.len(), 2);
    // Already kept: not proposed again.
    let kept = tool(&handle, "pm", "propose_routine", weekly).await;
    assert!(kept.is_error && kept.text.contains("already has"), "{kept:?}");

    // Declined: nothing saved.
    tool(&handle, "pm", "propose_routine", json!({ "title": "일일 스탠드업", "prompt": "어제 한 일", "every": "daily", "hour": 9 })).await;
    let other = proposed(&until(&mut events, |e| matches!(e, Event::RoutineProposed { .. })).await);
    handle.send(Command::DeclineRoutine { squad: "acme".into(), id: other.id });
    until(&mut events, |e| matches!(e, Event::RoutineProposalClosed { approved: false, .. })).await;
    let snapshot = handle.snapshot("acme").await;
    assert!(!snapshot.routines.iter().any(|r| r.title == "일일 스탠드업") && snapshot.routine_proposals.is_empty(), "{snapshot:?}");
    handle.shutdown().await;
}

/// `acme`, its board quiet for two days, the idle wake off until the test says.
fn quiet_squad() -> (PathBuf, PathBuf) {
    let (home, project) = squad_home();
    let journal = Journal::for_squad(&home, "acme").unwrap();
    journal.set_active_at(now() - 2 * DAY).unwrap();
    journal.set_idle_wake_hours(0).unwrap();
    (home, project)
}

#[tokio::test]
async fn an_empty_quiet_board_wakes_the_pm_once_and_not_again_within_the_day() {
    let (home, project) = quiet_squad();
    let (handle, mut events) = runtime::start(config(&busy_claude(), &home));
    start(&handle, &mut events, 1, "pm", &project).await;

    // On: the board has lain empty two days — the PM is woken, once.
    handle.send(Command::SetIdleWake { squad: "acme".into(), hours: 24 });
    let seen = until(&mut events, |e| matches!(e, Event::IdleWakeChanged { hours: 24, .. })).await;
    assert_eq!(idle_wakes(&seen), 1, "{seen:#?}");
    let woke = seen.iter().find_map(|e| if let Event::UserMessage { session: 1, text } = e { Some(text.clone()) } else { None }).expect("the PM's");
    assert!(woke.starts_with("[Idle · 24h]") && woke.contains("propose what's next"), "{woke}");
    assert_eq!(handle.snapshot("acme").await.idle_wake_hours, Some(24));

    // Looked at again within the day — even on a shorter wait — it isn't woken twice.
    handle.send(Command::SetIdleWake { squad: "acme".into(), hours: 1 });
    let seen = until(&mut events, |e| matches!(e, Event::IdleWakeChanged { hours: 1, .. })).await;
    assert_eq!(idle_wakes(&seen), 0, "{seen:#?}");
    handle.shutdown().await;

    // Nor after a restart.
    let (again, mut events) = runtime::start(config(&busy_claude(), &home));
    again.send(Command::SetIdleWake { squad: "acme".into(), hours: 24 });
    let seen = until(&mut events, |e| matches!(e, Event::IdleWakeChanged { hours: 24, .. })).await;
    assert_eq!(idle_wakes(&seen), 0, "{seen:#?}");
    again.shutdown().await;
}

#[tokio::test]
async fn open_work_or_the_idle_wake_off_leaves_the_pm_alone() {
    // Off: the board quiet for days, and nobody is woken.
    let (home, project) = quiet_squad();
    let (handle, mut events) = runtime::start(config(&busy_claude(), &home));
    start(&handle, &mut events, 1, "pm", &project).await;
    handle.send(Command::SetIdleWake { squad: "acme".into(), hours: 0 });
    let seen = until(&mut events, |e| matches!(e, Event::IdleWakeChanged { hours: 0, .. })).await;
    assert_eq!(idle_wakes(&seen), 0, "{seen:#?}");
    assert_eq!(handle.snapshot("acme").await.idle_wake_hours, Some(0));
    handle.shutdown().await;

    // On, but a ticket is open (one just made): the board isn't empty.
    let (home, project) = quiet_squad();
    let (handle, mut events) = runtime::start(config(&busy_claude(), &home));
    start(&handle, &mut events, 1, "pm", &project).await;
    let made = tool(&handle, "pm", "create_ticket", json!({ "title": "Ask a designer", "body": "-", "owner": "designer", "kind": "task" })).await;
    assert!(!made.is_error, "{made:?}");
    handle.send(Command::SetIdleWake { squad: "acme".into(), hours: 24 });
    let seen = until(&mut events, |e| matches!(e, Event::IdleWakeChanged { hours: 24, .. })).await;
    assert_eq!(idle_wakes(&seen), 0, "{seen:#?}");
    handle.shutdown().await;
}
