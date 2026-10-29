//! Routines, driven the way a screen drives them: the person's routine saved
//! with its next run worked out, run on their click and handed to the PM as
//! their message, changed, deleted — and kept across a restart. Only the
//! person's commands make, change or run one.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::routines::{Routine, Schedule};
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-routines-{name}-{}-{n}", std::process::id()));
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

fn routines_in(events: &[Event]) -> Vec<Routine> {
    events.iter().rev().find_map(|e| if let Event::RoutinesChanged { routines, .. } = e { Some(routines.clone()) } else { None }).expect("routines")
}

async fn pm(handle: &Handle, events: &mut UnboundedReceiver<Event>, project: &Path) {
    let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.to_owned(), role: Some("pm".into()), squad: Some("acme".into()) };
    handle.send(Command::Start { session: 1, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session: 1, .. })).await;
}

#[tokio::test]
async fn the_persons_routine_is_kept_run_on_their_click_and_handed_to_the_pm() {
    let project = scratch("site").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let claude = busy_claude();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    pm(&handle, &mut events, &project).await;

    // Not without a name, a task and a real time.
    let blank = Routine { id: 0, title: " ".into(), prompt: "x".into(), when: Schedule::Daily { hour: 9, minute: 0 }, on: true, last_run: None, next_run: None };
    handle.send(Command::SaveRoutine { squad: "acme".into(), routine: blank });
    until(&mut events, |e| matches!(e, Event::Error { .. })).await;

    // Saved: numbered, its next run ahead, on every screen and in a snapshot.
    let weekly = Routine { id: 0, title: "주간 리포트".into(), prompt: "이번 주 프로젝트별로 정리해 줘".into(), when: Schedule::Weekly { weekday: 1, hour: 9, minute: 0 }, on: true, last_run: None, next_run: None };
    handle.send(Command::SaveRoutine { squad: "acme".into(), routine: weekly });
    let saved = routines_in(&until(&mut events, |e| matches!(e, Event::RoutinesChanged { .. })).await);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    assert_eq!((saved.len(), saved[0].id, saved[0].title.as_str()), (1, 1, "주간 리포트"));
    let next = saved[0].next_run.expect("a next run");
    assert!(next > now && next <= now + 7 * 24 * 3600, "within the week: {next} vs {now}");
    assert_eq!(handle.snapshot("acme").await.routines, saved);

    // Run on the person's click: the PM has it as their message, and it ran.
    handle.send(Command::RunRoutine { squad: "acme".into(), id: 1 });
    let seen = until(&mut events, |e| matches!(e, Event::RoutinesChanged { .. })).await;
    assert!(
        seen.iter().any(|e| matches!(e, Event::UserMessage { session: 1, text } if text == "[Routine · 주간 리포트]\n\n이번 주 프로젝트별로 정리해 줘")),
        "{seen:#?}"
    );
    let ran = routines_in(&seen);
    assert!(ran[0].last_run.is_some_and(|at| at >= now));

    // Changed (off, daily) keeps its number and when it last ran; a second one is 2.
    let changed = Routine { on: false, when: Schedule::Daily { hour: 18, minute: 30 }, ..ran[0].clone() };
    handle.send(Command::SaveRoutine { squad: "acme".into(), routine: changed });
    let after = routines_in(&until(&mut events, |e| matches!(e, Event::RoutinesChanged { .. })).await);
    assert_eq!((after[0].on, after[0].when, after[0].last_run), (false, Schedule::Daily { hour: 18, minute: 30 }, ran[0].last_run));
    let deps = Routine { id: 0, title: "의존성 점검".into(), prompt: "업데이트할 의존성 찾아줘".into(), when: Schedule::Weekly { weekday: 5, hour: 16, minute: 0 }, on: true, last_run: None, next_run: None };
    handle.send(Command::SaveRoutine { squad: "acme".into(), routine: deps });
    let two = routines_in(&until(&mut events, |e| matches!(e, Event::RoutinesChanged { .. })).await);
    assert_eq!(two.iter().map(|r| r.id).collect::<Vec<_>>(), [1, 2]);

    // Deleted: gone; the other stays, across a restart.
    handle.send(Command::DeleteRoutine { squad: "acme".into(), id: 1 });
    let left = routines_in(&until(&mut events, |e| matches!(e, Event::RoutinesChanged { .. })).await);
    assert_eq!(left.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["의존성 점검"]);
    handle.shutdown().await;
    let (again, _events) = runtime::start(config(&claude, &home));
    assert_eq!(again.snapshot("acme").await.routines, left);
    again.shutdown().await;
}
