//! The runtime, driven the way a screen drives it: commands in, events out.
//! A stand-in `claude` plays the engine, so no network or login is needed.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::journal::Journal;
use equipe_core::protocol::{Access, Command, Engine, Entry, Event, SessionSpec, SessionState, TurnOutcome};
use equipe_core::runtime::{self, Config};
use equipe_providers::CredentialStore;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A fresh directory. A counter, not the clock: two tests in the same
/// microsecond (macOS clock resolution) must never share one.
fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-rt-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A stand-in `claude`: answers `auth status`, and runs `turn` for a turn.
fn fake_claude(logged_in: bool, turn: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = scratch("bin").join("claude");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":{logged_in},\"authMethod\":\"claude.ai\"}}'; exit 0; fi\n{turn}\n"
    );
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// A turn that answers `text` the way Claude Code streams it.
fn answering(text: &str) -> String {
    format!(
        r#"cat <<EOF
{{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}}
{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"{text}"}}}}}}
{{"type":"result","subtype":"success","is_error":false,"result":"{text}","usage":{{"input_tokens":3,"output_tokens":1}}}}
EOF"#
    )
}

fn config(claude: &Path, home: &Path, persist: bool) -> Config {
    Config {
        // No Codex login here, so Claude Code is the engine that's ready.
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        persist,
        retention_days: 90,
    }
}

fn start(claude: &Path) -> (runtime::Handle, UnboundedReceiver<Event>) {
    runtime::start(config(claude, &scratch("home"), false))
}

/// Collect events until one matches, failing after a timeout.
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

fn spec(cwd: &Path, access: Access) -> SessionSpec {
    SessionSpec { engine: None, model: None, access, cwd: cwd.to_owned(), role: None }
}

fn member(cwd: &Path, role: &str) -> SessionSpec {
    SessionSpec { role: Some(role.into()), ..spec(cwd, Access::ReadOnly) }
}

/// A stand-in `claude` that also writes down the arguments of every turn, one
/// file per call, in order.
fn recording_claude(turn: &str) -> (PathBuf, PathBuf) {
    let calls = scratch("calls");
    let record = format!(
        "n=$(ls '{dir}' | wc -l | tr -d ' ')\nfor a in \"$@\"; do printf '%s\\n<<arg>>\\n' \"$a\"; done > '{dir}/call-'$(printf %03d $n)\n{turn}",
        dir = calls.display()
    );
    (fake_claude(true, &record), calls)
}

/// Each recorded turn's arguments, oldest first.
fn calls(dir: &Path) -> Vec<Vec<String>> {
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    files.sort();
    files
        .iter()
        .map(|f| {
            let text = std::fs::read_to_string(f).unwrap();
            text.split("\n<<arg>>\n").filter(|a| !a.is_empty()).map(str::to_owned).collect()
        })
        .collect()
}

/// The value following `flag` in one call's arguments.
fn arg<'a>(call: &'a [String], flag: &str) -> Option<&'a str> {
    call.iter().position(|a| a == flag).and_then(|i| call.get(i + 1)).map(String::as_str)
}

fn text_of(events: &[Event]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            Event::TextDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn repo() -> PathBuf {
    let dir = scratch("repo");
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

#[tokio::test]
async fn a_turn_streams_text_and_ends() {
    let claude = fake_claude(true, &answering("pong"));
    let (handle, mut events) = start(&claude);
    let cwd = scratch("cwd");

    handle.send(Command::Start { session: 1, spec: spec(&cwd, Access::ReadOnly) });
    let started = until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    assert!(matches!(started.last(), Some(Event::SessionStarted { engine: Engine::Claude, workspace: None, .. })));

    handle.send(Command::Submit { session: 1, text: "ping".into() });
    let turn = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert_eq!(text_of(&turn), "pong");
    assert!(turn.iter().any(|e| matches!(e, Event::ModelKnown { model, .. } if model == "claude-test")));
    assert!(matches!(turn.last(), Some(Event::TurnEnded { outcome: TurnOutcome::Completed, usage: Some(_), .. })));
}

#[tokio::test]
async fn the_second_turn_continues_the_conversation() {
    // The stand-in answers "resumed" only when asked to continue sess-1.
    let claude = fake_claude(
        true,
        &format!(
            "case \"$*\" in *\"--resume sess-1\"*) T=resumed;; *) T=fresh;; esac\n{}",
            answering("$T")
        ),
    );
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: spec(&scratch("cwd"), Access::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;

    handle.send(Command::Submit { session: 1, text: "one".into() });
    let first = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    handle.send(Command::Submit { session: 1, text: "two".into() });
    let second = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;

    assert_eq!(text_of(&first), "fresh");
    assert_eq!(text_of(&second), "resumed");
}

#[tokio::test]
async fn an_editing_session_lands_its_changes_on_a_branch() {
    let repo = repo();
    let claude = fake_claude(true, &format!("echo 'agent was here' > NOTE.md\n{}", answering("done")));
    let (handle, mut events) = start(&claude);

    handle.send(Command::Start { session: 1, spec: spec(&repo, Access::EditFiles) });
    let started = until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    let Some(Event::SessionStarted { workspace: Some(workspace), .. }) = started.last() else {
        panic!("no workspace: {started:#?}");
    };
    assert!(workspace.branch.starts_with("equipe/try-"));

    handle.send(Command::Submit { session: 1, text: "leave a note".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    handle.send(Command::Close { session: 1 });
    let closed = until(&mut events, |e| matches!(e, Event::SessionClosed { .. })).await;

    let landed = closed.iter().find_map(|e| match e {
        Event::BranchLanded { branch, stat, .. } => Some((branch.clone(), stat.clone())),
        _ => None,
    });
    let (branch, stat) = landed.unwrap_or_else(|| panic!("no branch landed; events: {closed:#?}"));
    assert!(stat.contains("NOTE.md"), "{stat}");
    assert_eq!(git(&repo, &["show", &format!("{branch}:NOTE.md")]), "agent was here");
    // The user's checkout never saw the change.
    assert!(!repo.join("NOTE.md").exists());
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    assert!(!workspace.path.exists(), "the worktree is cleaned up");
}

#[tokio::test]
async fn an_editing_session_that_changes_nothing_keeps_no_branch() {
    let repo = repo();
    let claude = fake_claude(true, &answering("nothing to do"));
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: spec(&repo, Access::EditFiles) });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    handle.send(Command::Submit { session: 1, text: "look around".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    handle.send(Command::Close { session: 1 });
    let closed = until(&mut events, |e| matches!(e, Event::SessionClosed { .. })).await;
    assert!(closed.iter().any(|e| matches!(e, Event::NothingChanged { .. })));
    assert_eq!(git(&repo, &["branch", "--list", "equipe/*"]), "");
}

#[tokio::test]
async fn esc_stops_a_running_turn_right_away() {
    let claude = fake_claude(true, "exec sleep 30");
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: spec(&scratch("cwd"), Access::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;

    handle.send(Command::Submit { session: 1, text: "take forever".into() });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let asked = std::time::Instant::now();
    handle.send(Command::Interrupt { session: 1 });
    let ended = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(matches!(ended.last(), Some(Event::TurnEnded { outcome: TurnOutcome::Interrupted, .. })));
    assert!(asked.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn a_message_sent_mid_turn_waits_as_a_memo() {
    // Each turn takes a moment, long enough to leave a memo during the first.
    let claude = fake_claude(true, &format!("sleep 1\n{}", answering("ok")));
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: spec(&scratch("cwd"), Access::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    handle.send(Command::Submit { session: 1, text: "first".into() });
    handle.send(Command::Submit { session: 1, text: "also this".into() });
    handle.send(Command::Submit { session: 1, text: "and this".into() });

    let first = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    let memos: Vec<_> = first
        .iter()
        .filter_map(|e| match e {
            Event::MessageQueued { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(memos, ["also this", "and this"]);
    assert!(!first.iter().any(|e| matches!(e, Event::Error { .. })), "a memo is not an error: {first:#?}");

    // When the turn ends, the memos arrive together as the next message.
    let second = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(matches!(&second[0], Event::UserMessage { text, .. } if text == "also this\n\nand this"), "{second:#?}");
}

#[tokio::test]
async fn esc_delivers_waiting_memos_at_once() {
    let claude = fake_claude(true, "case \"$*\" in *\"take forever\"*) exec sleep 30;; esac\necho");
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: spec(&scratch("cwd"), Access::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    handle.send(Command::Submit { session: 1, text: "take forever".into() });
    handle.send(Command::Submit { session: 1, text: "stop, do this instead".into() });
    until(&mut events, |e| matches!(e, Event::MessageQueued { .. })).await;

    let asked = std::time::Instant::now();
    handle.send(Command::Interrupt { session: 1 });
    let seen = until(&mut events, |e| matches!(e, Event::UserMessage { .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::TurnEnded { outcome: TurnOutcome::Interrupted, .. })));
    assert!(matches!(seen.last(), Some(Event::UserMessage { text, .. }) if text == "stop, do this instead"));
    assert!(asked.elapsed() < Duration::from_secs(2));
}

// ── members ──

#[tokio::test]
async fn a_member_gets_its_charter_a_recommended_model_and_its_own_memory() {
    let (claude, recorded) = recording_claude(&answering("on it"));
    let home = scratch("home");
    let cwd = scratch("cwd");
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    handle.send(Command::Start { session: 1, spec: member(&cwd, "pm") });
    let started = until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    assert!(matches!(started.last(), Some(Event::SessionStarted { member: Some(m), .. }) if m.name == "pm" && m.role == "pm"));

    handle.send(Command::Submit { session: 1, text: "plan the site".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    let call = &calls(&recorded)[0];
    let charter = arg(call, "--append-system-prompt").expect("a charter");
    assert!(charter.starts_with("You are pm, the product manager (PM)"), "{charter}");
    // The PM plans: the top tier.
    assert_eq!(arg(call, "--model"), Some("opus"));
    // What it learns stays with this member of this project — not in the person's
    // own Claude Code memory.
    let settings = arg(call, "--settings").expect("settings");
    let memory = equipe_core::journal::member_memory_dir(&home, &runtime::project_of(&cwd), "pm");
    assert!(settings.contains(&*memory.to_string_lossy()), "{settings}");
}

#[tokio::test]
async fn members_are_named_by_role_and_told_who_else_is_on_the_team() {
    let (claude, recorded) = recording_claude(&answering("ok"));
    let (handle, mut events) = start(&claude);
    let cwd = scratch("cwd");
    for (id, role) in [(1, "pm"), (2, "frontend"), (3, "frontend")] {
        handle.send(Command::Start { session: id, spec: SessionSpec { access: Access::ReadOnly, ..member(&cwd, role) } });
        until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    }
    let names: Vec<_> = handle.snapshot(&cwd).await.sessions.into_iter().map(|s| s.member.unwrap().name).collect();
    assert_eq!(names, ["pm", "frontend", "frontend-2"]);

    handle.send(Command::Submit { session: 3, text: "hi".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 3, .. })).await;
    let charter = arg(&calls(&recorded)[0], "--append-system-prompt").unwrap().to_owned();
    assert!(charter.starts_with("You are frontend-2, the frontend engineer"), "{charter}");
    assert!(charter.contains("Your teammates: pm, frontend."), "{charter}");
}

#[tokio::test]
async fn talking_to_a_member_directly_keeps_the_pm_in_the_loop() {
    let (claude, recorded) = recording_claude(&answering("Switching to Postgres."));
    let (handle, mut events) = start(&claude);
    let cwd = scratch("cwd");
    handle.send(Command::Start { session: 1, spec: member(&cwd, "pm") });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    handle.send(Command::Start { session: 2, spec: member(&cwd, "backend") });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;

    // The person goes straight to backend…
    handle.send(Command::Submit { session: 2, text: "use Postgres".into() });
    let seen = until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    // …and every screen on the PM sees it happen,
    assert!(
        seen.iter().any(|e| matches!(e, Event::Notice { session: 1, text } if text == "↪ You → backend: use Postgres")),
        "{seen:#?}"
    );
    // without the PM spending a turn on it.
    assert_eq!(calls(&recorded).len(), 1);

    // The PM's next message carries what it missed; the person sees only theirs.
    handle.send(Command::Submit { session: 1, text: "where are we?".into() });
    let pm_turn = until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    assert!(matches!(&pm_turn[0], Event::UserMessage { text, .. } if text == "where are we?"));
    let prompt = arg(&calls(&recorded)[1], "-p").unwrap().to_owned();
    assert!(prompt.contains("The person told backend: use Postgres"), "{prompt}");
    assert!(prompt.contains("backend answered: Switching to Postgres."), "{prompt}");
    assert!(prompt.ends_with("where are we?"), "{prompt}");

    // Told once: the turn after that carries no briefing.
    handle.send(Command::Submit { session: 1, text: "thanks".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    assert_eq!(arg(&calls(&recorded)[2], "-p"), Some("thanks"));
}

#[tokio::test]
async fn an_editing_member_works_on_a_branch_named_after_it() {
    let repo = repo();
    let claude = fake_claude(true, &answering("ok"));
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: SessionSpec { access: Access::EditFiles, ..member(&repo, "frontend") } });
    let started = until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    let Some(Event::SessionStarted { workspace: Some(ws), .. }) = started.last() else { panic!("{started:#?}") };
    assert!(ws.branch.starts_with("equipe/frontend-"), "{}", ws.branch);
}

#[tokio::test]
async fn the_team_comes_back_after_a_restart_pm_first() {
    let claude = fake_claude(true, &answering("ok"));
    let home = scratch("home");
    let cwd = scratch("cwd");
    {
        let (handle, mut events) = runtime::start(config(&claude, &home, true));
        // Session ids are random; the PM need not have the smallest.
        for (id, role) in [(9, "designer"), (5, "pm")] {
            handle.send(Command::Start { session: id, spec: member(&cwd, role) });
            until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
        }
        handle.shutdown().await;
    }
    let (handle, _events) = runtime::start(config(&claude, &home, true));
    let names: Vec<_> = handle.snapshot(&cwd).await.sessions.into_iter().map(|s| s.member.unwrap().name).collect();
    assert_eq!(names, ["pm", "designer"]);
}

#[tokio::test]
async fn no_engine_ready_says_how_to_fix_it() {
    let claude = fake_claude(false, &answering("never"));
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: spec(&scratch("cwd"), Access::ReadOnly) });
    let failed = until(&mut events, |e| matches!(e, Event::Error { .. })).await;
    assert!(matches!(failed.last(), Some(Event::Error { next: Some(next), .. }) if next == "equipe login"));
}

#[tokio::test]
async fn codex_cannot_edit_yet() {
    let claude = fake_claude(true, &answering("x"));
    let (handle, mut events) = start(&claude);
    let mut edit = spec(&repo(), Access::EditFiles);
    edit.engine = Some(Engine::Codex);
    handle.send(Command::Start { session: 1, spec: edit });
    let failed = until(&mut events, |e| matches!(e, Event::Error { .. })).await;
    let Some(Event::Error { message, .. }) = failed.last() else { unreachable!() };
    assert!(message.contains("Claude Code"), "{message}");
}

// ── persistence: the daemon's runtime ──

fn text_entries(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .map(|e| match e {
            Entry::User { text } => format!("user: {text}"),
            Entry::Reply { text } => format!("reply: {text}"),
            other => format!("{other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn a_restarted_runtime_resumes_the_session_and_its_conversation() {
    let claude = fake_claude(
        true,
        &format!("case \"$*\" in *\"--resume sess-1\"*) T=resumed;; *) T=fresh;; esac\n{}", answering("$T")),
    );
    let home = scratch("home");
    let cwd = scratch("cwd");
    {
        let (handle, mut events) = runtime::start(config(&claude, &home, true));
        handle.send(Command::Start { session: 42, spec: spec(&cwd, Access::ReadOnly) });
        until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
        handle.send(Command::Submit { session: 42, text: "remember me".into() });
        until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
        handle.shutdown().await;
    }

    // A new runtime on the same home: the session is back, with its conversation.
    let (handle, mut events) = runtime::start(config(&claude, &home, true));
    let snapshot = handle.snapshot(&cwd).await;
    assert_eq!(snapshot.sessions.len(), 1, "{snapshot:#?}");
    let session = &snapshot.sessions[0];
    assert_eq!(session.session, 42);
    assert_eq!(session.title.as_deref(), Some("remember me"));
    assert_eq!(text_entries(&session.recent), ["user: remember me", "reply: fresh"]);

    // And the next turn continues Claude Code's conversation.
    handle.send(Command::Submit { session: 42, text: "again".into() });
    let turn = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert_eq!(text_of(&turn), "resumed");
}

#[tokio::test]
async fn the_journal_keeps_finished_replies_not_streaming_fragments() {
    // Three fragments stream; one reply is kept.
    let claude = fake_claude(
        true,
        r#"cat <<EOF
{"type":"system","subtype":"init","model":"m","session_id":"s"}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"one "}}}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"two "}}}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"three"}}}
{"type":"result","subtype":"success","is_error":false,"result":"one two three","usage":{"input_tokens":3,"output_tokens":3}}
EOF"#,
    );
    let home = scratch("home");
    let cwd = scratch("cwd");
    let (handle, mut events) = runtime::start(config(&claude, &home, true));
    handle.send(Command::Start { session: 7, spec: spec(&cwd, Access::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    handle.send(Command::Submit { session: 7, text: "count".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    handle.shutdown().await;

    let journal = Journal::for_project(&home, &runtime::project_of(&cwd)).unwrap();
    assert_eq!(text_entries(&journal.conversation(7).unwrap()), ["user: count", "reply: one two three"]);
    assert_eq!(journal.open_sessions().unwrap()[0].tokens, 6);
}

#[tokio::test]
async fn a_screen_attaching_mid_turn_sees_the_agent_working() {
    let claude = fake_claude(true, "exec sleep 30");
    let home = scratch("home");
    let cwd = scratch("cwd");
    let (handle, mut events) = runtime::start(config(&claude, &home, true));
    handle.send(Command::Start { session: 3, spec: spec(&cwd, Access::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    handle.send(Command::Submit { session: 3, text: "long job".into() });
    until(&mut events, |e| matches!(e, Event::UserMessage { .. })).await;

    let snapshot = handle.snapshot(&cwd).await;
    assert_eq!(snapshot.sessions[0].state, SessionState::Working);
    assert_eq!(text_entries(&snapshot.sessions[0].recent), ["user: long job"]);
    handle.shutdown().await;
}

#[tokio::test]
async fn a_closed_session_is_not_resumed() {
    let claude = fake_claude(true, &answering("ok"));
    let home = scratch("home");
    let cwd = scratch("cwd");
    {
        let (handle, mut events) = runtime::start(config(&claude, &home, true));
        handle.send(Command::Start { session: 5, spec: spec(&cwd, Access::ReadOnly) });
        until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
        handle.send(Command::Close { session: 5 });
        until(&mut events, |e| matches!(e, Event::SessionClosed { .. })).await;
        handle.shutdown().await;
    }
    let (handle, _events) = runtime::start(config(&claude, &home, true));
    assert!(handle.snapshot(&cwd).await.sessions.is_empty());
}

#[tokio::test]
async fn every_screen_sees_what_was_typed() {
    let claude = fake_claude(true, &answering("ok"));
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: spec(&scratch("cwd"), Access::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    handle.send(Command::Submit { session: 1, text: "hello team".into() });
    let seen = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::UserMessage { text, .. } if text == "hello team")));
}
