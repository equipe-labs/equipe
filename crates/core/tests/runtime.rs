//! The runtime, driven the way a screen drives it: commands in, events out.
//! A stand-in `claude` plays the engine, so no network or login is needed.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::protocol::{Access, Command, Engine, Event, SessionSpec, TurnOutcome};
use equipe_core::runtime::{self, Config};
use equipe_providers::CredentialStore;
use tokio::sync::mpsc::UnboundedReceiver;

fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("equipe-rt-{name}-{}-{nanos}", std::process::id()));
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

fn start(claude: &Path) -> (runtime::Handle, UnboundedReceiver<Event>) {
    let home = scratch("home");
    runtime::start(Config {
        // No Codex login here, so Claude Code is the engine that's ready.
        store: CredentialStore::new(&home),
        home,
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
    })
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
    SessionSpec { engine: None, model: None, access, cwd: cwd.to_owned() }
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
    let (branch, stat) = landed.expect("a branch landed");
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
async fn one_turn_at_a_time() {
    let claude = fake_claude(true, "exec sleep 30");
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: spec(&scratch("cwd"), Access::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    handle.send(Command::Submit { session: 1, text: "first".into() });
    handle.send(Command::Submit { session: 1, text: "second".into() });
    let refused = until(&mut events, |e| matches!(e, Event::Error { .. })).await;
    let Some(Event::Error { message, .. }) = refused.last() else { unreachable!() };
    assert!(message.contains("already running"), "{message}");
    handle.send(Command::Interrupt { session: 1 });
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
