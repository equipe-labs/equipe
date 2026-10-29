//! Changes to the team the PM proposes (`propose_team_change`), driven the
//! way a screen and an engine drive them: the PM's tool call in, the
//! proposal out as an event, the person's approval in as their command — and
//! the member it brings joining and taking the ticket that waited. An agent
//! never makes the change: no tool does, and nothing in the daemon starts or
//! lets go a member but the person's own commands.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::team::{Change, TeamChange};
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-team-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A stand-in `claude`: signed in, and every turn takes a long while — a
/// member on a ticket stays at work on it.
fn busy_claude() -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = scratch("bin").join("claude");
    let script = "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; exit 0; fi\nexec sleep 30\n";
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn config(claude: &Path, home: &Path, persist: bool) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        // No Codex CLI either: the person's own must never run in a test.
        codex_binary: "/nonexistent/codex".into(),
        persist,
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

fn member(cwd: &Path, role: &str) -> SessionSpec {
    SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: cwd.to_owned(), role: Some(role.into()), squad: Some("acme".into()) }
}

async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, spec: SessionSpec) {
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

fn proposed(events: &[Event]) -> TeamChange {
    events.iter().find_map(|e| if let Event::TeamChangeProposed { change, .. } = e { Some(change.clone()) } else { None }).expect("proposed")
}

/// The squad's members by name, as a screen attaching sees them.
async fn names(handle: &Handle) -> Vec<String> {
    let mut names: Vec<String> = handle.snapshot("acme").await.sessions.iter().filter_map(|s| s.member.as_ref().map(|m| m.name.clone())).collect();
    names.sort();
    names
}

/// A squad of a PM and a qa at work on T-1, and T-2 for qa waiting behind it.
async fn busy_squad(home: &Path, project: &Path, persist: bool) -> (Handle, UnboundedReceiver<Event>) {
    let (handle, mut events) = runtime::start(config(&busy_claude(), home, persist));
    add(&handle, &mut events, 1, member(project, "pm")).await;
    add(&handle, &mut events, 2, member(project, "qa")).await;
    tool(&handle, "pm", "create_ticket", json!({ "title": "Review the release", "body": "-", "owner": "qa" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { session: 2, ticket: 1, .. })).await;
    tool(&handle, "pm", "create_ticket", json!({ "title": "Test checkout", "body": "-", "owner": "qa" })).await;
    (handle, events)
}

#[tokio::test]
async fn the_pm_proposes_the_person_approves_and_the_new_member_takes_the_waiting_ticket() {
    let (home, project) = squad_home();
    let (handle, mut events) = busy_squad(&home, &project, false).await;

    // A member only suggests: the tool is the PM's.
    let refused = tool(&handle, "qa", "propose_team_change", json!({ "change": "add", "role": "qa", "reason": "T-2 waits" })).await;
    assert!(refused.is_error && refused.text.contains("Only the PM"), "{refused:?}");
    // Checked before anything waits.
    let odd = tool(&handle, "pm", "propose_team_change", json!({ "change": "add", "role": "astronaut", "reason": "-" })).await;
    assert!(odd.is_error && odd.text.contains("No role called astronaut"), "{odd:?}");

    // Unasked, a few seconds after T-2 came: too soon — qa takes it as it finishes.
    let soon = tool(&handle, "pm", "propose_team_change", json!({ "change": "add", "role": "qa", "reason": "T-2가 기다려요", "unblocks": [2] })).await;
    assert!(soon.is_error && soon.text.contains("has waited 0 min"), "{soon:?}");
    // The person asked for one in words: it goes to them.
    let said = tool(&handle, "pm", "propose_team_change", json!({ "change": "add", "role": "qa", "reason": "T-2가 기다려요 (qa는 T-1 중)", "unblocks": [2], "asked": true })).await;
    assert!(!said.is_error && said.text.starts_with("Proposed — the person decides on a card."), "{said:?}");
    let change = proposed(&until(&mut events, |e| matches!(e, Event::TeamChangeProposed { .. })).await);
    assert_eq!((change.change, change.role.as_str(), change.count, change.unblocks.as_slice(), change.by.as_str(), change.session), (Change::Add, "qa", 1, &[2][..], "pm", 1));
    // Proposed is not done: nobody joined, and a screen attaching sees it waiting.
    assert_eq!(names(&handle).await, ["pm", "qa"]);
    assert_eq!(handle.snapshot("acme").await.team_changes, std::slice::from_ref(&change));
    // The same change twice is one card.
    let again = tool(&handle, "pm", "propose_team_change", json!({ "change": "add", "role": "qa", "reason": "again" })).await;
    assert!(again.is_error && again.text.contains("already waits for the person"), "{again:?}");

    // The person approves: what `/add qa` does — qa-2 joins, and takes T-2.
    handle.send(Command::ApproveTeamChange { squad: "acme".into(), id: change.id });
    let seen = until(&mut events, |e| matches!(e, Event::TeamChangeClosed { .. })).await;
    let joined = seen.iter().find_map(|e| match e {
        Event::SessionStarted { session, member: Some(m), .. } if m.name == "qa-2" => Some(*session),
        _ => None,
    });
    let joined = joined.expect("qa-2 joined");
    assert!(seen.iter().any(|e| matches!(e, Event::WorkStarted { session, ticket: 2, .. } if *session == joined)), "{seen:#?}");
    assert!(matches!(seen.last(), Some(Event::TeamChangeClosed { id, approved: true, members, .. }) if *id == change.id && members == &["qa-2".to_owned()]));
    let news = until(&mut events, |e| matches!(e, Event::Notice { session: 1, .. })).await;
    assert!(matches!(news.last(), Some(Event::Notice { text, .. }) if text == "팀에 qa-2을(를) 추가했어요"), "{news:#?}");
    assert_eq!(names(&handle).await, ["pm", "qa", "qa-2"]);
    assert!(handle.snapshot("acme").await.team_changes.is_empty());

    // Approved again from another screen: nothing more.
    handle.send(Command::ApproveTeamChange { squad: "acme".into(), id: change.id });
    handle.send(Command::DeclineTeamChange { squad: "acme".into(), id: change.id });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(names(&handle).await, ["pm", "qa", "qa-2"]);
    handle.shutdown().await;
}

#[tokio::test]
async fn declined_nothing_changes_and_the_pm_hears_of_it() {
    let (home, project) = squad_home();
    let (handle, mut events) = busy_squad(&home, &project, false).await;
    tool(&handle, "pm", "propose_team_change", json!({ "change": "add", "role": "frontend", "count": 2, "reason": "-" })).await;
    let change = proposed(&until(&mut events, |e| matches!(e, Event::TeamChangeProposed { .. })).await);
    handle.send(Command::DeclineTeamChange { squad: "acme".into(), id: change.id });
    let seen = until(&mut events, |e| matches!(e, Event::Notice { session: 1, .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::TeamChangeClosed { id, approved: false, members, .. } if *id == change.id && members.is_empty())), "{seen:#?}");
    assert!(matches!(seen.last(), Some(Event::Notice { text, .. }) if text == "팀 변경 제안을 거절했어요 · frontend 2명 추가"), "{seen:#?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::SessionStarted { .. })), "{seen:#?}");
    assert_eq!(names(&handle).await, ["pm", "qa"]);
    assert!(handle.snapshot("acme").await.team_changes.is_empty());
    handle.shutdown().await;
}

#[tokio::test]
async fn a_member_let_go_at_work_puts_its_ticket_back_and_never_the_pm() {
    let (home, project) = squad_home();
    let (handle, mut events) = busy_squad(&home, &project, false).await;
    let pm = tool(&handle, "pm", "propose_team_change", json!({ "change": "remove", "name": "pm", "reason": "-" })).await;
    assert!(pm.is_error && pm.text.contains("The PM stays"), "{pm:?}");
    let said = tool(&handle, "pm", "propose_team_change", json!({ "change": "remove", "role": "qa", "reason": "끝났어요" })).await;
    assert!(said.text.contains("qa stops at its next step and leaves; T-1 goes back to the board"), "{said:?}");
    let change = proposed(&until(&mut events, |e| matches!(e, Event::TeamChangeProposed { .. })).await);
    assert_eq!((change.member.as_deref(), change.busy_on), (Some("qa"), Some(1)));
    handle.send(Command::ApproveTeamChange { squad: "acme".into(), id: change.id });
    let seen = until(&mut events, |e| matches!(e, Event::TeamChangeClosed { .. })).await;
    // What `/close` on qa does: it leaves, its ticket back on the board.
    assert!(seen.iter().any(|e| matches!(e, Event::SessionClosed { session: 2 })), "{seen:#?}");
    assert!(matches!(seen.last(), Some(Event::TeamChangeClosed { approved: true, members, .. }) if members == &["qa".to_owned()]));
    assert_eq!(names(&handle).await, ["pm"]);
    handle.shutdown().await;
}

#[tokio::test]
async fn letting_go_a_member_that_left_anyway_no_longer_waits_and_nothing_waits_after_a_restart() {
    let (home, project) = squad_home();
    {
        let (handle, mut events) = busy_squad(&home, &project, true).await;
        tool(&handle, "pm", "propose_team_change", json!({ "change": "remove", "name": "qa", "reason": "-" })).await;
        let gone = proposed(&until(&mut events, |e| matches!(e, Event::TeamChangeProposed { .. })).await);
        tool(&handle, "pm", "propose_team_change", json!({ "change": "add", "role": "data", "reason": "-" })).await;
        let kept = proposed(&until(&mut events, |e| matches!(e, Event::TeamChangeProposed { .. })).await);
        // The person closes qa themselves: letting it go is moot.
        handle.send(Command::Close { session: 2 });
        let seen = until(&mut events, |e| matches!(e, Event::TeamChangeClosed { .. })).await;
        assert!(matches!(seen.last(), Some(Event::TeamChangeClosed { id, approved: false, .. }) if *id == gone.id), "{seen:#?}");
        assert_eq!(handle.snapshot("acme").await.team_changes, [kept]);
        handle.shutdown().await;
    }
    // What made it worth it doesn't outlast a restart: the PM proposes again if it still holds.
    let (handle, _events) = runtime::start(config(&busy_claude(), &home, true));
    assert!(handle.snapshot("acme").await.team_changes.is_empty());
    handle.shutdown().await;
}

/// The guarantee, in the code itself: the daemon starts a member only for
/// [`Command::Start`] — a screen's `/add`, or the person's approval sending
/// the same — and an approval runs only on [`Command::ApproveTeamChange`].
/// No tool, and no reading of anyone's words, reaches either.
#[test]
fn nothing_adds_or_lets_go_a_member_but_the_persons_commands() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![src.join("runtime.rs")];
    // In name order: a folder lists in a different order on Linux than on macOS.
    let mut rest: Vec<_> = std::fs::read_dir(src.join("runtime")).unwrap().flatten().map(|e| e.path()).collect();
    rest.sort();
    files.extend(rest);
    let mut starts = Vec::new();
    let mut start_commands = Vec::new();
    let mut approvals = Vec::new();
    let mut switches = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("//") {
                continue;
            }
            if line.contains("self.start(") {
                starts.push(format!("{name}: {line}"));
            }
            if line.contains("Command::Start {") {
                start_commands.push(format!("{name}: {line}"));
            }
            if line.contains("approve_team_change(") {
                approvals.push(format!("{name}: {line}"));
            }
            if line.contains("switch_member(") {
                switches.push(format!("{name}: {line}"));
            }
        }
    }
    assert_eq!(starts, ["runtime.rs: Command::Start { session, spec } => self.start(session, spec).await,"], "a member starts only for Command::Start");
    assert_eq!(
        start_commands,
        [
            "runtime.rs: Command::Start { session, spec } => self.start(session, spec).await,",
            // A member moving engines is started again under its own name (or,
            // when the other engine can't take it, where it was): nobody joins.
            "engines.rs: Box::pin(self.handle(Command::Start { session: fresh, spec })).await;",
            "engines.rs: Box::pin(self.handle(Command::Start { session: again, spec })).await;",
            "team.rs: Box::pin(self.handle(Command::Start { session, spec })).await;",
        ],
        "only the approval sends what /add sends, and moving engines restarts a member"
    );
    // A move is made only at a member's boundary, from moves the person
    // applied or their own `auto` scheduled.
    assert_eq!(
        switches,
        ["engines.rs: let moved = gone || self.switch_member(squad, id, &due).await;", "engines.rs: async fn switch_member(&mut self, squad: &str, id: SessionId, due: &Due) -> bool {"],
        "members move engines only at their boundary"
    );
    assert_eq!(
        approvals,
        ["runtime.rs: Command::ApproveTeamChange { squad, id } => self.approve_team_change(&squad, id).await,", "team.rs: pub(super) async fn approve_team_change(&mut self, squad: &str, id: u64) {"],
        "an approval runs only on the person's command"
    );
    // And no member has a tool that makes a change.
    for is_pm in [true, false] {
        for tool in equipe_core::board::tools(is_pm) {
            let name = tool["name"].as_str().unwrap();
            assert!(!name.contains("approve_team") && !name.contains("add_member") && !name.contains("remove_member"), "{name}");
        }
    }
}
