//! The runtime, driven the way a screen drives it: commands in, events out.
//! A stand-in `claude` plays the engine, so no network or login is needed.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::journal::{self, Journal};
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{
    Access, Command, Engine, Entry, Event, OffMain, SessionSpec, SessionState, Ticket, TicketStatus, TurnOutcome,
};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::{squad_home, squads};
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
    dir.canonicalize().unwrap()
}

/// A fresh project directory called `name`, as the board names projects.
fn named(name: &str) -> PathBuf {
    let dir = scratch(name).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A stand-in `claude`: answers `auth status`, and runs `turn` for a turn.
fn fake_claude(logged_in: bool, turn: &str) -> PathBuf {
    let bin = scratch("bin").join("claude");
    // A turn's prompt comes in on stdin, as Claude Code's stream-json; put in
    // front of the arguments as `-p <prompt>`, where a test looks for it.
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":{logged_in},\"authMethod\":\"claude.ai\"}}'; exit 0; fi\nif [ \"$2\" = --input-format ] && IFS= read -r line; then set -- -p \"$(printf '%s' \"$line\" | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"message\"][\"content\"], end=\"\")')\" \"$@\"; fi\n{turn}\n"
    );
    write_script(&bin, &script);
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

/// A stand-in `claude` that also writes down the arguments of every turn, one
/// file per call, in order.
fn recording_claude(turn: &str) -> (PathBuf, PathBuf) {
    let calls = scratch("calls");
    let record = format!(
        "n=$(ls '{dir}' | wc -l | tr -d ' ')\nfor a in \"$@\"; do printf '%s\\n<<arg>>\\n' \"$a\"; done > '{dir}/call-'$(printf %03d $n)-$$\n{turn}",
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

fn config(claude: &Path, home: &Path, persist: bool) -> Config {
    Config {
        // No Codex login here, so Claude Code is the engine that's ready.
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        persist,
        retention_days: 90,
        equipe_binary: None,
        npx: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        lang: equipe_core::i18n::Lang::En,
    }
}

fn start(claude: &Path) -> (Handle, UnboundedReceiver<Event>) {
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
    SessionSpec { engine: None, model: None, access, cwd: cwd.to_owned(), role: None, squad: None }
}

/// A member of the squad `acme`, joining from `cwd`.
fn member(cwd: &Path, role: &str) -> SessionSpec {
    SessionSpec { role: Some(role.into()), squad: Some("acme".into()), ..spec(cwd, Access::ReadOnly) }
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
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

/// A home where the squad `acme` works on `projects`.
fn squad_home(projects: &[&Path]) -> PathBuf {
    let home = scratch("home");
    squads::create(&home, "acme", projects[0]).unwrap();
    for project in &projects[1..] {
        squads::add_project(&home, "acme", project).unwrap();
    }
    home
}

/// Add a member and wait until it has started.
async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, spec: SessionSpec) {
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

fn ticket_changed(events: &[Event]) -> Vec<Ticket> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::TicketChanged { ticket, .. } => Some(ticket.clone()),
            _ => None,
        })
        .collect()
}

// ── plain sessions (`equipe try`) ──

#[tokio::test]
async fn a_turn_streams_text_and_ends() {
    let claude = fake_claude(true, &answering("pong"));
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: spec(&scratch("cwd"), Access::ReadOnly) });
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
        &format!("case \"$*\" in *\"--resume sess-1\"*) T=resumed;; *) T=fresh;; esac\n{}", answering("$T")),
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
    // For after this turn: they don't steer it.
    handle.send(Command::SubmitLater { session: 1, text: "also this".into() });
    handle.send(Command::SubmitLater { session: 1, text: "and this".into() });

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
async fn a_memo_taken_back_is_never_delivered() {
    let claude = fake_claude(true, &format!("sleep 1\n{}", answering("ok")));
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: spec(&scratch("cwd"), Access::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    handle.send(Command::Submit { session: 1, text: "first".into() });
    handle.send(Command::SubmitLater { session: 1, text: "keep this".into() });
    handle.send(Command::SubmitLater { session: 1, text: "never mind".into() });
    handle.send(Command::Unqueue { session: 1, text: "never mind".into() });
    // A memo nobody left: nothing happens.
    handle.send(Command::Unqueue { session: 1, text: "not a memo".into() });

    let first = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    let withdrawn: Vec<_> = first.iter().filter_map(|e| if let Event::MemoWithdrawn { text, .. } = e { Some(text.as_str()) } else { None }).collect();
    assert_eq!(withdrawn, ["never mind"]);
    let second = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(matches!(&second[0], Event::UserMessage { text, .. } if text == "keep this"), "{second:#?}");
}

#[tokio::test]
async fn the_effort_asked_for_reaches_claude_code() {
    let (claude, recorded) = recording_claude(&answering("ok"));
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: spec(&scratch("cwd"), Access::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    handle.send(Command::SetEffort { session: 1, effort: Some(" XHigh ".into()) });
    handle.send(Command::Submit { session: 1, text: "think".into() });
    let turn = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(turn.iter().any(|e| matches!(e, Event::Notice { text, .. } if text.contains("effort xhigh"))), "{turn:#?}");
    let args = calls(&recorded).pop().expect("a turn ran");
    assert!(args.windows(2).any(|w| w[0] == "--effort" && w[1] == "xhigh"), "{args:?}");
}

#[tokio::test]
async fn a_screen_attaching_sees_the_effort_each_member_thinks_at() {
    let repo = repo();
    let claude = fake_claude(true, "exec sleep 30");
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&repo])));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    handle.send(Command::SetEffort { session: 1, effort: Some("High".into()) });
    until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("effort"))).await;
    let effort = |snapshot: equipe_core::protocol::Snapshot| snapshot.sessions.iter().find(|s| s.session == 1).and_then(|s| s.effort.clone());
    assert_eq!(effort(handle.snapshot("acme").await).as_deref(), Some("high"));
    handle.send(Command::SetEffort { session: 1, effort: None });
    until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("default effort"))).await;
    assert_eq!(effort(handle.snapshot("acme").await), None, "back on the engine's default");
    handle.shutdown().await;
}

#[tokio::test]
async fn said_while_it_works_it_reads_at_its_next_step_without_stopping() {
    // Claude Code reads a second message mid-turn, echoes it as it does, and carries on.
    let turn = r#"IFS= read -r more
text=$(printf '%s' "$more" | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["message"]["content"]))')
cat <<EOF
{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}
{"type":"user","isReplay":true,"message":{"role":"user","content":"fix the page"}}
{"type":"user","isReplay":true,"message":{"role":"user","content":$text}}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"skipping the footer"}}}
{"type":"result","subtype":"success","is_error":false,"result":"skipping the footer","usage":{"input_tokens":3,"output_tokens":1}}
EOF"#;
    let claude = fake_claude(true, turn);
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: spec(&scratch("cwd"), Access::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    handle.send(Command::Submit { session: 1, text: "fix the page".into() });
    until(&mut events, |e| matches!(e, Event::UserMessage { .. })).await;
    handle.send(Command::Submit { session: 1, text: "skip the footer".into() });
    let seen = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::SteerQueued { text, .. } if text == "skip the footer")), "{seen:#?}");
    let read = seen.iter().position(|e| matches!(e, Event::Steered { text, .. } if text == "skip the footer")).expect("read");
    let answer = seen.iter().position(|e| matches!(e, Event::TextDelta { .. })).unwrap();
    assert!(read < answer, "read mid-turn, before the rest of the answer");
    assert!(!seen.iter().any(|e| matches!(e, Event::TurnEnded { outcome: TurnOutcome::Interrupted, .. })), "nothing stopped");
    // Read: not delivered again as a memo.
    tokio::time::sleep(Duration::from_millis(300)).await;
    while let Ok(e) = events.try_recv() {
        assert!(!matches!(e, Event::MessageQueued { .. } | Event::UserMessage { .. }), "{e:?}");
    }
}

#[tokio::test]
async fn esc_delivers_waiting_memos_at_once() {
    let claude = fake_claude(true, "case \"$*\" in *\"take forever\"*) exec sleep 30;; esac\necho");
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: spec(&scratch("cwd"), Access::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    handle.send(Command::Submit { session: 1, text: "take forever".into() });
    // Sent while it works: it would read it at its next step; Esc stops it first.
    handle.send(Command::Submit { session: 1, text: "stop, do this instead".into() });
    until(&mut events, |e| matches!(e, Event::SteerQueued { .. })).await;

    let asked = std::time::Instant::now();
    handle.send(Command::Interrupt { session: 1 });
    let seen = until(&mut events, |e| matches!(e, Event::UserMessage { .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::TurnEnded { outcome: TurnOutcome::Interrupted, .. })));
    assert!(matches!(seen.last(), Some(Event::UserMessage { text, .. }) if text == "stop, do this instead"));
    assert!(asked.elapsed() < Duration::from_secs(2));
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

// ── members ──

#[tokio::test]
async fn a_member_needs_a_squad_that_exists() {
    let claude = fake_claude(true, &answering("ok"));
    let (handle, mut events) = start(&claude);
    handle.send(Command::Start { session: 1, spec: member(&scratch("cwd"), "pm") });
    let failed = until(&mut events, |e| matches!(e, Event::Error { .. })).await;
    assert!(matches!(failed.last(), Some(Event::Error { message, .. }) if message.contains("no squad called acme")));
}

#[tokio::test]
async fn equipe_speaks_to_the_person_in_their_screens_language() {
    let claude = fake_claude(true, &answering("ok"));
    let (handle, mut events) = start(&claude);
    handle.send(Command::Language { lang: "ko_KR.UTF-8".into() });
    handle.send(Command::Start { session: 1, spec: member(&scratch("cwd"), "pm") });
    let failed = until(&mut events, |e| matches!(e, Event::Error { .. })).await;
    assert!(matches!(failed.last(), Some(Event::Error { message, .. }) if message == "acme라는 스쿼드가 없습니다."), "{failed:#?}");
    // A language equipe doesn't speak changes nothing.
    handle.send(Command::Language { lang: "pt-BR".into() });
    handle.send(Command::Start { session: 2, spec: member(&scratch("cwd"), "pm") });
    let failed = until(&mut events, |e| matches!(e, Event::Error { .. })).await;
    assert!(matches!(failed.last(), Some(Event::Error { message, .. }) if message.contains("스쿼드가 없습니다")), "{failed:#?}");
}

#[tokio::test]
async fn a_squad_with_no_project_yet_gets_its_pm_and_the_pm_answers() {
    let claude = fake_claude(true, &answering("hello"));
    let home = scratch("home");
    squads::create_without_project(&home, "acme").unwrap();
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    // Opened from a folder that is no project of it — a repository elsewhere, say.
    add(&handle, &mut events, 1, member(&scratch("elsewhere"), "pm")).await;
    let pm = handle.snapshot("acme").await.sessions.into_iter().next().expect("the PM joined");
    assert_eq!(pm.member.map(|m| m.name).as_deref(), Some("pm"));
    assert_eq!(pm.project, squad_home::path(&home, "acme"), "its desk is the squad's home");

    handle.send(Command::Submit { session: 1, text: "hi".into() });
    let turn = until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    assert!(text_of(&turn).contains("hello"));
}

#[tokio::test]
async fn a_member_gets_its_charter_a_recommended_model_and_its_own_memory() {
    let (claude, recorded) = recording_claude(&answering("on it"));
    let cwd = scratch("cwd");
    let home = squad_home(&[&cwd]);
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    handle.send(Command::Start { session: 1, spec: member(&cwd, "pm") });
    let started = until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    assert!(matches!(started.last(), Some(Event::SessionStarted { member: Some(m), squad: Some(s), .. })
        if m.name == "pm" && s == "acme"));

    handle.send(Command::Submit { session: 1, text: "plan the site".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    let call = &calls(&recorded)[0];
    let charter = arg(call, "--append-system-prompt").expect("a charter");
    assert!(charter.starts_with("You are pm, the product manager (PM)"), "{charter}");
    assert_eq!(arg(call, "--model"), Some("opus"), "the PM plans: the top tier");
    // What it learns stays with this member of this squad — not in the person's
    // own Claude Code memory.
    let settings = arg(call, "--settings").expect("settings");
    let memory = journal::member_memory_dir(&home, "acme", "pm");
    assert!(settings.contains(&*memory.to_string_lossy()), "{settings}");
    // Without equipe's binary, no board tools are offered.
    assert_eq!(arg(call, "--mcp-config"), None);
}

#[tokio::test]
async fn members_are_named_by_role_and_told_who_else_is_on_the_team() {
    let (claude, recorded) = recording_claude(&answering("ok"));
    let cwd = scratch("cwd");
    let home = squad_home(&[&cwd]);
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    for (id, role) in [(1, "pm"), (2, "qa"), (3, "qa")] {
        add(&handle, &mut events, id, member(&cwd, role)).await;
    }
    let names: Vec<_> = handle.snapshot("acme").await.sessions.into_iter().map(|s| s.member.unwrap().name).collect();
    assert_eq!(names, ["pm", "qa", "qa-2"]);

    handle.send(Command::Submit { session: 3, text: "hi".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 3, .. })).await;
    let charter = arg(&calls(&recorded)[0], "--append-system-prompt").unwrap().to_owned();
    assert!(charter.starts_with("You are qa-2, the QA engineer"), "{charter}");
    assert!(charter.contains("Your teammates: pm, qa."), "{charter}");
    // What qa-2 learns is the role's: it outlasts qa-2, and qa reads it too.
    let settings = arg(&calls(&recorded)[0], "--settings").unwrap().to_owned();
    let memory = journal::member_memory_dir(&home, "acme", "qa");
    assert!(settings.contains(&format!("\"{}\"", memory.display())), "{settings}");
}

#[tokio::test]
async fn at_its_desk_a_member_reads_every_project_and_gets_only_equipes_tools() {
    let (claude, recorded) = recording_claude(&answering("ok"));
    let (site, app) = (named("site"), named("app"));
    let home = squad_home(&[&site, &app]);
    let mut config = config(&claude, &home, false);
    config.equipe_binary = Some("/opt/equipe".into());
    let (handle, mut events) = runtime::start(config);
    add(&handle, &mut events, 1, member(&site, "frontend")).await;
    handle.send(Command::Submit { session: 1, text: "what's in app?".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;

    let call = &calls(&recorded)[0];
    // At its desk it reads on its own and writes only the squad's home; anything
    // more — a fetch, a write elsewhere — asks the person rather than being refused unseen.
    assert_eq!(arg(call, "--permission-mode"), Some("acceptEdits"));
    assert_eq!(arg(call, "--permission-prompt-tool"), Some("mcp__equipe__approve"));
    assert_eq!(arg(call, "--add-dir"), Some(&*site.to_string_lossy()));
    let mcp: serde_json::Value = serde_json::from_str(arg(call, "--mcp-config").unwrap()).unwrap();
    let server = &mcp["mcpServers"]["equipe"];
    assert_eq!(server["command"], "/opt/equipe");
    let args: Vec<&str> = server["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert_eq!(&args[..5], ["mcp", "--squad", "acme", "--member", "frontend"]);
    assert!(call.iter().any(|a| a == "--strict-mcp-config"), "the person's own MCP servers stay out");
    assert_eq!(arg(call, "--allowedTools"), Some("mcp__equipe"));
    let charter = arg(call, "--append-system-prompt").unwrap();
    assert!(charter.contains("several projects: site, app") && charter.contains("`submit_for_review`"), "{charter}");
}

#[tokio::test]
async fn talking_to_a_member_directly_keeps_the_pm_in_the_loop() {
    let (claude, recorded) = recording_claude(&answering("Switching to Postgres."));
    let cwd = scratch("cwd");
    let (handle, mut events) = runtime::start(config(&claude, &squad_home(&[&cwd]), false));
    add(&handle, &mut events, 1, member(&cwd, "pm")).await;
    add(&handle, &mut events, 2, member(&cwd, "backend")).await;

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
async fn the_squad_writes_for_the_person_in_their_language() {
    let repo = repo();
    let (claude, recorded) = recording_claude(&answering("ok"));
    let home = squad_home(&[&repo]);
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    handle.send(Command::Submit { session: 1, text: "랜딩 페이지 만들자".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    assert_eq!(Journal::for_squad(&home, "acme").unwrap().language().as_deref(), Some("Korean"));
    // The PM's own turn already knows, and so does a member handed a ticket.
    let pm = arg(&calls(&recorded)[0], "--append-system-prompt").unwrap().to_owned();
    assert!(pm.contains("The person writes in Korean"), "{pm}");
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "히어로", "body": "-", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    let work = calls(&recorded).pop().unwrap();
    assert!(arg(&work, "-p").unwrap().ends_with("Write your summary and anything else for the person in Korean."));
}

// ── the board ──

/// A turn that takes a moment and writes `file` where it runs.
fn working_turn(file: &str) -> String {
    format!(
        "case \"$*\" in *\"You're on T-\"*) sleep 1; echo \"made by the squad\" > {file};; esac\n{}",
        answering("working on it")
    )
}

#[tokio::test]
async fn a_ticket_goes_to_its_owner_who_hands_it_in_and_the_person_accepts_it() {
    let repo = repo();
    let (claude, recorded) = recording_claude(&working_turn("HERO.md"));
    let home = squad_home(&[&repo]);
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;

    // The PM puts work on the board; equipe's rules hand it to the free frontend.
    let made = tool(&handle, "pm", "create_ticket", serde_json::json!({
        "title": "Landing hero", "body": "A headline and one button.", "owner": "frontend",
    }))
    .await;
    assert_eq!(made, ToolResult { text: "Created T-1.".into(), is_error: false });
    let started = until(&mut events, |e| matches!(e, Event::WorkStarted { .. })).await;
    let Some(Event::WorkStarted { session: 2, ticket: 1, workspace: Some(ws), .. }) = started.last() else {
        panic!("{started:#?}")
    };
    assert!(ws.branch.starts_with("equipe/frontend-t1-"), "{}", ws.branch);
    let assigned = ticket_changed(&started);
    assert_eq!(
        assigned.last().map(|t| (t.status, t.assignee.clone())),
        Some((TicketStatus::InProgress, Some("frontend".into())))
    );

    // Its first turn is the ticket, in its worktree, allowed to edit.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let call = calls(&recorded).pop().unwrap();
    let prompt = arg(&call, "-p").unwrap();
    assert!(prompt.starts_with("[equipe] You're on T-1: Landing hero"), "{prompt}");
    assert!(prompt.contains("A headline and one button."), "{prompt}");
    assert_eq!(arg(&call, "--permission-mode"), Some("acceptEdits"));

    // It hands the work in mid-turn; at the turn's end the work is committed.
    let handed = tool(&handle, "frontend", "submit_for_review", serde_json::json!({ "summary": "Hero done." })).await;
    assert!(!handed.is_error, "{handed:?}");
    let review =
        until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.status == TicketStatus::Review)).await;
    let ticket = ticket_changed(&review).pop().unwrap();
    assert_eq!(ticket.note.as_deref(), Some("Hero done."));
    let branch = ticket.branch.clone().unwrap();
    assert_eq!(git(&repo, &["show", &format!("{branch}:HERO.md")]), "made by the squad");
    assert!(ws.path.exists(), "the worktree stays while the work is reviewed");
    assert!(!repo.join("HERO.md").exists(), "the person's checkout is untouched");

    // Only the person accepts: done, the branch stays to merge, the worktree goes.
    handle.send(Command::Accept { squad: "acme".into(), ticket: 1 });
    let done =
        until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.status == TicketStatus::Done)).await;
    assert_eq!(ticket_changed(&done).pop().unwrap().branch.as_deref(), Some(branch.as_str()));
    assert!(!ws.path.exists());
    let notices = until(&mut events, |e| matches!(e, Event::Notice { session: 1, .. })).await;
    assert!(matches!(notices.last(), Some(Event::Notice { text, .. }) if text.starts_with("You accepted T-1")), "{notices:#?}");
}

#[tokio::test]
async fn accepted_work_is_where_the_next_ticket_starts_and_lands_on_main_only_when_the_person_merges() {
    let repo = repo();
    let (claude, recorded) = recording_claude(&working_turn("HERO.md"));
    let (handle, mut events) = runtime::start(config(&claude, &squad_home(&[&repo]), false));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Hero", "body": "-", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    tool(&handle, "frontend", "submit_for_review", serde_json::json!({ "summary": "Hero done." })).await;
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.status == TicketStatus::Review)).await;

    // Merging waits for acceptance.
    handle.send(Command::Merge { squad: "acme".into(), ticket: 1, push: false });
    let refused = until(&mut events, |e| matches!(e, Event::Error { .. })).await;
    assert!(matches!(refused.last(), Some(Event::Error { message, .. }) if message.contains("isn't accepted yet")), "{refused:#?}");

    // Accepting alone merges nothing, and the board says so.
    let main_before = git(&repo, &["rev-parse", "main"]);
    handle.send(Command::Accept { squad: "acme".into(), ticket: 1 });
    let done =
        until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.status == TicketStatus::Done)).await;
    assert_eq!(ticket_changed(&done).pop().unwrap().merged, Some(false));
    assert_eq!(git(&repo, &["rev-parse", "main"]), main_before);
    assert!(!repo.join("HERO.md").exists());

    // The next ticket starts with it, though it isn't on main.
    tool(&handle, "pm", "create_ticket", serde_json::json!({
        "title": "Pricing", "body": "-", "owner": "frontend", "depends_on": [1],
    }))
    .await;
    let started = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 2, .. })).await;
    let Some(Event::WorkStarted { workspace: Some(ws), .. }) = started.last() else { panic!("{started:#?}") };
    assert_eq!(std::fs::read_to_string(ws.path.join("HERO.md")).unwrap().trim(), "made by the squad");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let prompt = arg(&calls(&recorded).pop().unwrap(), "-p").unwrap().to_owned();
    assert!(prompt.contains("starts from the accepted work of T-1"), "{prompt}");

    // The person merges: main has it, and the board knows.
    handle.send(Command::Merge { squad: "acme".into(), ticket: 1, push: false });
    let merged = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.merged == Some(true))).await;
    assert_eq!(ticket_changed(&merged).pop().unwrap().id, 1);
    assert_eq!(std::fs::read_to_string(repo.join("HERO.md")).unwrap().trim(), "made by the squad");
}

#[tokio::test]
async fn work_not_on_main_says_why_can_be_kept_on_its_branch_and_settles_when_nothing_is_left_to_merge() {
    let repo = repo();
    let claude = fake_claude(true, &answering("ok"));
    let home = squad_home(&[&repo]);
    // T-1 changed a file and changed it back (+0 −0); T-2 rewrote the README
    // that main rewrites too; T-3 is real work.
    let on_branch = |id: u32, steps: &[(&str, Option<&str>)]| {
        let branch = format!("equipe/dev-t{id}");
        git(&repo, &["branch", &branch, "main"]);
        let path = scratch("wt").join("w");
        git(&repo, &["worktree", "add", "-q", &path.to_string_lossy(), &branch]);
        for (file, text) in steps {
            match text {
                Some(text) => std::fs::write(path.join(file), text).unwrap(),
                None => std::fs::remove_file(path.join(file)).unwrap(),
            }
            git(&path, &["add", "-A"]);
            git(&path, &["commit", "-qm", file]);
        }
        git(&repo, &["worktree", "remove", "--force", &path.to_string_lossy()]);
    };
    on_branch(1, &[("NOTE.md", Some("x\n")), ("NOTE.md", None)]);
    on_branch(2, &[("README.md", Some("theirs\n"))]);
    on_branch(3, &[("PAGE.md", Some("page\n"))]);
    std::fs::write(repo.join("README.md"), "ours\n").unwrap();
    git(&repo, &["commit", "-qam", "ours"]);
    {
        let journal = Journal::for_squad(&home, "acme").unwrap();
        for id in [1, 2, 3] {
            let made = journal
                .create_ticket(&Ticket {
                    id: 0,
                    title: format!("Piece {id}"),
                    body: "-".into(),
                    status: TicketStatus::Done,
                    owner: Some("frontend".into()),
                    assignee: None,
                    project: Some(repo.clone()),
                    depends_on: Vec::new(),
                    branch: Some(format!("equipe/dev-t{id}")),
                    worktree: None,
                    merged: None,
                    off_main: None,
                    kind: equipe_core::protocol::TicketKind::Code,
                    created_by: "pm".into(),
                    note: None,
                    asks: vec![],
                })
                .unwrap();
            let mut t = journal.ticket(made).unwrap().unwrap();
            t.merged = Some(false);
            journal.save_ticket(&t).unwrap();
        }
    }
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;

    // A merge that conflicts merges nothing, and the ticket says why.
    handle.send(Command::Merge { squad: "acme".into(), ticket: 2, push: false });
    let failed = until(&mut events, |e| matches!(e, Event::Error { .. })).await;
    let two = ticket_changed(&failed).pop().unwrap();
    assert!(matches!(&two.off_main, Some(OffMain::Conflict { detail }) if detail.contains("README.md")), "{two:?}");
    assert!(two.waits_to_merge(), "still the person's call");

    // Kept on its branch: it stops asking, and is still not on main.
    handle.send(Command::KeepOnBranch { squad: "acme".into(), ticket: 3 });
    let kept = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 3)).await;
    let three = ticket_changed(&kept).pop().unwrap();
    assert_eq!((three.off_main.clone(), three.merged), (Some(OffMain::Kept), Some(false)));
    assert!(three.not_on_main() && !three.waits_to_merge());
    // Only work accepted and off main can be kept.
    handle.send(Command::KeepOnBranch { squad: "acme".into(), ticket: 9 });
    until(&mut events, |e| matches!(e, Event::Error { .. })).await;

    // Merged after all: on main, its reason gone.
    handle.send(Command::Merge { squad: "acme".into(), ticket: 3, push: false });
    let merged = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 3 && ticket.merged == Some(true))).await;
    assert_eq!(ticket_changed(&merged).pop().unwrap().off_main, None);
    assert!(repo.join("PAGE.md").exists() && !repo.join("NOTE.md").exists());
    // T-1 had nothing left to bring: settled as merged — on the daemon's
    // minute, or after a merge at the latest — and no longer asks.
    let board = handle.snapshot("acme").await.tickets;
    let off: Vec<(u32, Option<bool>, Option<OffMain>)> = board.iter().map(|t| (t.id, t.merged, t.off_main.clone())).collect();
    assert_eq!(off, [(1, Some(true), None), (2, Some(false), two.off_main.clone()), (3, Some(true), None)]);
}

#[tokio::test]
async fn dependencies_that_conflict_hold_the_ticket_and_say_why() {
    let repo = repo();
    let claude = fake_claude(true, &answering("ok"));
    let home = squad_home(&[&repo]);
    // Two accepted tickets whose branches both rewrote the README.
    for (id, text) in [(1, "one"), (2, "two")] {
        let branch = format!("equipe/dev-t{id}");
        git(&repo, &["branch", &branch, "main"]);
        let path = scratch("wt");
        git(&repo, &["worktree", "add", "-q", &path.join("w").to_string_lossy(), &branch]);
        std::fs::write(path.join("w/README.md"), text).unwrap();
        git(&path.join("w"), &["commit", "-qam", text]);
    }
    {
        let journal = Journal::for_squad(&home, "acme").unwrap();
        for id in [1, 2] {
            let made = journal
                .create_ticket(&Ticket {
                    id: 0,
                    title: format!("Piece {id}"),
                    body: "-".into(),
                    status: TicketStatus::Done,
                    owner: Some("frontend".into()),
                    assignee: None,
                    project: Some(repo.clone()),
                    depends_on: Vec::new(),
                    branch: Some(format!("equipe/dev-t{id}")),
                    worktree: None,
                    merged: Some(false),
                    off_main: None,
                    kind: equipe_core::protocol::TicketKind::Code,
                    created_by: "pm".into(),
                    note: None,
                    asks: vec![],
                })
                .unwrap();
            assert_eq!(made, id);
        }
    }
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({
        "title": "Both", "body": "-", "owner": "frontend", "depends_on": [1, 2],
    }))
    .await;
    let held = until(&mut events, |e| {
        matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 3 && ticket.status == TicketStatus::Proposed)
    })
    .await;
    let note = ticket_changed(&held).pop().unwrap().note.unwrap();
    assert!(note.starts_with("Couldn't start") && note.contains("T-2") && note.contains("README.md"), "{note}");
}

#[tokio::test]
async fn dependencies_hold_a_ticket_until_the_one_before_is_done() {
    let cwd = scratch("cwd");
    let claude = fake_claude(true, &answering("ok"));
    let (handle, mut events) = runtime::start(config(&claude, &squad_home(&[&cwd]), false));
    add(&handle, &mut events, 1, member(&cwd, "pm")).await;
    add(&handle, &mut events, 2, member(&cwd, "qa")).await;
    add(&handle, &mut events, 3, member(&cwd, "data")).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Test plan", "body": "-", "owner": "qa" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({
        "title": "Metrics", "body": "-", "owner": "data", "depends_on": [1],
    }))
    .await;
    // data is free, but T-2 waits for T-1.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let board = handle.snapshot("acme").await.tickets;
    assert_eq!(board[1].status, TicketStatus::Todo);

    handle.send(Command::Accept { squad: "acme".into(), ticket: 1 });
    let next = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 2, .. })).await;
    assert!(matches!(next.last(), Some(Event::WorkStarted { session: 3, .. })));
}

#[tokio::test]
async fn work_sent_back_returns_to_its_member_on_the_same_branch_and_conversation() {
    let repo = repo();
    let (claude, recorded) = recording_claude(&working_turn("PAGE.md"));
    let (handle, mut events) = runtime::start(config(&claude, &squad_home(&[&repo]), false));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Page", "body": "-", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { .. })).await;
    tool(&handle, "frontend", "submit_for_review", serde_json::json!({ "summary": "Page done." })).await;
    let review =
        until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.status == TicketStatus::Review)).await;
    let branch = ticket_changed(&review).pop().unwrap().branch;

    handle.send(Command::Reopen { squad: "acme".into(), ticket: 1, note: "make the button blue".into() });
    let again = until(&mut events, |e| matches!(e, Event::WorkStarted { .. })).await;
    assert!(matches!(again.last(), Some(Event::WorkStarted { session: 2, workspace: Some(ws), .. }) if Some(&ws.branch) == branch.as_ref()));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let call = calls(&recorded).pop().unwrap();
    let prompt = arg(&call, "-p").unwrap();
    assert!(prompt.contains("The person sent it back. Reopened: make the button blue"), "{prompt}");
    assert!(prompt.contains("save that lesson in your memory"), "a reopen is a lesson to keep: {prompt}");
    assert_eq!(arg(&call, "--resume"), Some("sess-1"), "it continues the ticket's conversation");
}

#[tokio::test]
async fn a_members_ticket_is_a_proposal_until_the_pm_accepts_it() {
    let cwd = scratch("cwd");
    let claude = fake_claude(true, &answering("ok"));
    let (handle, mut events) = runtime::start(config(&claude, &squad_home(&[&cwd]), false));
    add(&handle, &mut events, 1, member(&cwd, "pm")).await;
    add(&handle, &mut events, 2, member(&cwd, "qa")).await;

    let proposed =
        tool(&handle, "qa", "create_ticket", serde_json::json!({ "title": "Flaky login test", "body": "-", "owner": "qa" })).await;
    assert!(proposed.text.starts_with("Proposed T-1"), "{proposed:?}");
    let seen = until(&mut events, |e| matches!(e, Event::Notice { session: 1, .. })).await;
    assert!(matches!(seen.last(), Some(Event::Notice { text, .. }) if text == "qa proposed T-1: Flaky login test"));
    // Only the PM changes tickets.
    let refused = tool(&handle, "qa", "update_ticket", serde_json::json!({ "id": 1, "status": "todo" })).await;
    assert!(refused.is_error);
    // The PM accepts it, and it goes to work.
    assert!(!tool(&handle, "pm", "update_ticket", serde_json::json!({ "id": 1, "status": "todo" })).await.is_error);
    until(&mut events, |e| matches!(e, Event::WorkStarted { session: 2, ticket: 1, .. })).await;
    // Work in progress is the person's to change, not the PM's.
    let late = tool(&handle, "pm", "update_ticket", serde_json::json!({ "id": 1, "title": "x" })).await;
    assert!(late.is_error && late.text.contains("in progress"), "{late:?}");
}

#[tokio::test]
async fn tools_refuse_what_does_not_make_sense() {
    let (site, app) = (named("site"), named("app"));
    let claude = fake_claude(true, &answering("ok"));
    let (handle, mut events) = runtime::start(config(&claude, &squad_home(&[&site, &app]), false));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    let nowhere = tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "x", "body": "-", "kind": "code" })).await;
    assert!(nowhere.is_error && nowhere.text.contains("several: site, app"), "{nowhere:?}");
    let odd = tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "x", "kind": "chore" })).await;
    assert!(odd.is_error && odd.text.contains("code, task or action"), "{odd:?}");
    let unknown = tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "x", "project": "web" })).await;
    assert!(unknown.is_error && unknown.text.contains("No project called web"), "{unknown:?}");
    let nothing = tool(&handle, "pm", "submit_for_review", serde_json::json!({ "summary": "?" })).await;
    assert!(nothing.is_error && nothing.text.contains("not on a ticket"), "{nothing:?}");
    let unstaffed =
        tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "x", "project": "app", "owner": "designer" })).await;
    assert!(!unstaffed.is_error && unstaffed.text.contains("/add designer"), "{unstaffed:?}");
    let board = tool(&handle, "pm", "board", serde_json::json!({})).await;
    assert_eq!(board.text, "T-1 [todo] x — code, owner designer, project app\n");
    // Without a kind or a project, with several projects, it is a task for the squad's folder.
    let research = tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Research", "owner": "designer" })).await;
    assert!(research.text.starts_with("Created T-2 (task)."), "{research:?}");
    let recast = tool(&handle, "pm", "update_ticket", serde_json::json!({ "id": 2, "kind": "code" })).await;
    assert!(recast.is_error && recast.text.contains("several: site, app"), "{recast:?}");
    let recast = tool(&handle, "pm", "update_ticket", serde_json::json!({ "id": 2, "kind": "action" })).await;
    assert!(!recast.is_error, "{recast:?}");
    let board = tool(&handle, "pm", "board", serde_json::json!({})).await;
    assert!(board.text.contains("T-2 [todo] Research — action, owner designer\n"), "{}", board.text);
}

#[tokio::test]
async fn a_member_who_leaves_puts_its_ticket_back_for_someone_else() {
    let cwd = scratch("cwd");
    let claude = fake_claude(true, "exec sleep 30");
    let (handle, mut events) = runtime::start(config(&claude, &squad_home(&[&cwd]), false));
    add(&handle, &mut events, 1, member(&cwd, "pm")).await;
    add(&handle, &mut events, 2, member(&cwd, "qa")).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Review", "body": "-", "owner": "qa" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { session: 2, .. })).await;
    add(&handle, &mut events, 3, member(&cwd, "qa")).await;

    handle.send(Command::Close { session: 2 });
    let moved = until(&mut events, |e| matches!(e, Event::WorkStarted { session: 3, .. })).await;
    assert!(
        ticket_changed(&moved).iter().any(|t| t.status == TicketStatus::Todo && t.note.as_deref() == Some("qa left the squad.")),
        "{moved:#?}"
    );
    handle.shutdown().await;
}

/// `ticket` has gone to `waiting`.
fn is_waiting(e: &Event, id: u32) -> bool {
    matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == id && ticket.status == TicketStatus::Waiting)
}

#[tokio::test]
async fn a_member_waiting_on_the_person_takes_its_next_ticket_and_comes_back_to_the_same_worktree() {
    let repo = repo();
    let (claude, recorded) = recording_claude(&working_turn("WIP.md"));
    let (handle, mut events) = runtime::start(config(&claude, &squad_home(&[&repo]), false));
    // One backend, so the next ticket waits for this one.
    handle.send(Command::Scale { squad: "acme".into(), extra: 0 });
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "backend")).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Analytics", "body": "-", "owner": "backend" })).await;
    let started = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let Some(Event::WorkStarted { workspace: Some(first), .. }) = started.last() else { panic!("{started:#?}") };
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Webhooks", "body": "-", "owner": "backend" })).await;

    // It can't go on without the person: the ticket waits, with their checklist.
    let empty = tool(&handle, "backend", "block_on_person", serde_json::json!({ "needs": [] })).await;
    assert!(empty.is_error, "{empty:?}");
    let blocked = tool(&handle, "backend", "block_on_person", serde_json::json!({
        "needs": ["GitHub token", "PostHog secrets"], "why": "to deploy",
    }))
    .await;
    assert!(!blocked.is_error, "{blocked:?}");
    let waiting = until(&mut events, |e| is_waiting(e, 1)).await;
    let ticket = ticket_changed(&waiting).pop().unwrap();
    assert_eq!(ticket.note.as_deref(), Some("Waiting on you:\n- [ ] GitHub token\n- [ ] PostHog secrets\n(to deploy)"));
    assert_eq!(ticket.asks, ["GitHub token", "PostHog secrets"], "the asks travel as a list");
    assert_eq!(ticket.assignee.as_deref(), Some("backend"));
    assert!(first.path.exists(), "the worktree stays while the ticket waits");
    let branch = ticket.branch.clone().unwrap();
    assert_eq!(git(&repo, &["show", &format!("{branch}:WIP.md")]), "made by the squad", "the work so far is kept");

    // The member is free, and takes the next ticket.
    until(&mut events, |e| matches!(e, Event::WorkStarted { session: 2, ticket: 2, .. })).await;
    tool(&handle, "backend", "submit_for_review", serde_json::json!({ "summary": "Webhooks done." })).await;
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 2 && ticket.status == TicketStatus::Review))
        .await;

    // Only a waiting ticket resumes; the person says it's done, and it comes back where it was.
    handle.send(Command::Resume { squad: "acme".into(), ticket: 2, note: String::new() });
    let refused = until(&mut events, |e| matches!(e, Event::Error { .. })).await;
    assert!(matches!(refused.last(), Some(Event::Error { message, .. }) if message.contains("isn't waiting")), "{refused:#?}");
    handle.send(Command::Resume { squad: "acme".into(), ticket: 1, note: "both are in .env.local".into() });
    let again = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    assert!(
        matches!(again.last(), Some(Event::WorkStarted { session: 2, workspace: Some(ws), .. }) if ws.path == first.path && ws.branch == branch),
        "{again:#?}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let call = calls(&recorded).pop().unwrap();
    let prompt = arg(&call, "-p").unwrap();
    assert!(prompt.contains("it is back. both are in .env.local"), "{prompt}");
    assert_eq!(arg(&call, "--resume"), Some("sess-1"), "it continues the ticket's conversation");
}

#[tokio::test]
async fn a_waiting_ticket_stays_put_through_a_restart_until_the_pm_resumes_it() {
    let repo = repo();
    let claude = fake_claude(true, &answering("ok"));
    let home = squad_home(&[&repo]);
    {
        let (handle, mut events) = runtime::start(config(&claude, &home, true));
        add(&handle, &mut events, 1, member(&repo, "pm")).await;
        add(&handle, &mut events, 2, member(&repo, "backend")).await;
        tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Deploy", "body": "-", "owner": "backend" })).await;
        until(&mut events, |e| matches!(e, Event::WorkStarted { .. })).await;
        tool(&handle, "backend", "block_on_person", serde_json::json!({ "needs": "a Vercel token" })).await;
        // The turn may already be over: a new message makes sure one ends after the tool call.
        handle.send(Command::Submit { session: 2, text: "end your turn".into() });
        until(&mut events, |e| is_waiting(e, 1)).await;
        handle.shutdown().await;
    }
    let (handle, mut events) = runtime::start(config(&claude, &home, true));
    let snapshot = handle.snapshot("acme").await;
    let backend = snapshot.sessions.iter().find(|s| s.session == 2).unwrap();
    assert!(backend.work.is_none(), "a waiting ticket isn't picked up again");
    assert_eq!(snapshot.tickets[0].status, TicketStatus::Waiting);
    assert_eq!(snapshot.tickets[0].asks, ["a Vercel token"], "read back from the note it is kept in");
    let worktree = snapshot.tickets[0].worktree.clone().unwrap();
    assert!(worktree.exists());

    // The PM opens it again: back to the same member, in the same worktree.
    let resumed = tool(&handle, "pm", "update_ticket", serde_json::json!({ "id": 1, "status": "todo" })).await;
    assert!(!resumed.is_error, "{resumed:?}");
    let again = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    assert!(matches!(again.last(), Some(Event::WorkStarted { session: 2, workspace: Some(ws), .. }) if ws.path == worktree), "{again:#?}");
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
async fn a_restarted_runtime_brings_the_squad_back_pm_first() {
    let claude = fake_claude(
        true,
        &format!("case \"$*\" in *\"--resume sess-1\"*) T=resumed;; *) T=fresh;; esac\n{}", answering("$T")),
    );
    let cwd = scratch("cwd");
    let home = squad_home(&[&cwd]);
    {
        let (handle, mut events) = runtime::start(config(&claude, &home, true));
        // Session ids are random; the PM need not have the smallest.
        add(&handle, &mut events, 9, member(&cwd, "designer")).await;
        add(&handle, &mut events, 5, member(&cwd, "pm")).await;
        handle.send(Command::Submit { session: 5, text: "remember me".into() });
        until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
        handle.shutdown().await;
    }

    let (handle, mut events) = runtime::start(config(&claude, &home, true));
    let snapshot = handle.snapshot("acme").await;
    let names: Vec<_> = snapshot.sessions.iter().map(|s| s.member.clone().unwrap().name).collect();
    assert_eq!(names, ["pm", "designer"]);
    let pm = &snapshot.sessions[0];
    assert_eq!(pm.title.as_deref(), Some("remember me"));
    assert_eq!(text_entries(&pm.recent), ["user: remember me", "reply: fresh"]);
    assert_eq!(snapshot.projects, [cwd]);

    // The next turn continues Claude Code's conversation.
    handle.send(Command::Submit { session: 5, text: "again".into() });
    let turn = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert_eq!(text_of(&turn), "resumed");
}

#[tokio::test]
async fn a_restarted_runtime_resumes_a_member_on_its_ticket() {
    let repo = repo();
    let claude = fake_claude(true, "exec sleep 30");
    let home = squad_home(&[&repo]);
    {
        let (handle, mut events) = runtime::start(config(&claude, &home, true));
        add(&handle, &mut events, 1, member(&repo, "pm")).await;
        add(&handle, &mut events, 2, member(&repo, "frontend")).await;
        tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Hero", "body": "-", "owner": "frontend" })).await;
        until(&mut events, |e| matches!(e, Event::WorkStarted { .. })).await;
        handle.shutdown().await;
    }
    let (handle, _events) = runtime::start(config(&claude, &home, true));
    let snapshot = handle.snapshot("acme").await;
    let frontend = snapshot.sessions.iter().find(|s| s.session == 2).unwrap();
    let work = frontend.work.as_ref().expect("still on its ticket");
    assert_eq!(work.ticket, 1);
    assert!(work.workspace.as_ref().is_some_and(|ws| ws.path.exists()));
    assert_eq!(snapshot.tickets[0].status, TicketStatus::InProgress);
    // It picks the ticket up again by itself: the squad keeps working through a restart.
    assert_eq!(frontend.state, SessionState::Working);
    assert!(frontend.recent.iter().any(|e| matches!(e, Entry::Note { text } if text.contains("picking T-1 up again"))), "{:?}", frontend.recent);
}

#[tokio::test]
async fn memos_and_the_pms_briefing_survive_a_restart() {
    // The PM says whether it was briefed; the designer's work runs long.
    let claude = fake_claude(
        true,
        &format!(
            "case \"$*\" in *\"since your last turn\"*) T=briefed;; *\"long job\"*|*footer*) exec sleep 30;; *) T=plain;; esac\n{}",
            answering("$T")
        ),
    );
    let cwd = scratch("cwd");
    let home = squad_home(&[&cwd]);
    {
        let (handle, mut events) = runtime::start(config(&claude, &home, true));
        add(&handle, &mut events, 1, member(&cwd, "pm")).await;
        add(&handle, &mut events, 2, member(&cwd, "designer")).await;
        handle.send(Command::Submit { session: 2, text: "long job".into() });
        until(&mut events, |e| matches!(e, Event::UserMessage { session: 2, .. })).await;
        handle.send(Command::SubmitLater { session: 2, text: "and the footer".into() });
        until(&mut events, |e| matches!(e, Event::MessageQueued { .. })).await;
        handle.shutdown().await;
    }

    let (handle, mut events) = runtime::start(config(&claude, &home, true));
    // The memo waited for a turn that the restart ended: it goes in now.
    let snapshot = handle.snapshot("acme").await;
    let designer = snapshot.sessions.iter().find(|s| s.session == 2).unwrap();
    assert!(designer.queued.is_empty(), "{:?}", designer.queued);
    assert!(text_entries(&designer.recent).contains(&"user: and the footer".to_owned()), "{:?}", designer.recent);
    // And the PM still hears what the person told the designer.
    handle.send(Command::Submit { session: 1, text: "status?".into() });
    let turn = until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    assert_eq!(text_of(&turn), "briefed");
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
    let cwd = scratch("cwd");
    let home = squad_home(&[&cwd]);
    let (handle, mut events) = runtime::start(config(&claude, &home, true));
    add(&handle, &mut events, 7, member(&cwd, "pm")).await;
    handle.send(Command::Submit { session: 7, text: "count".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    handle.shutdown().await;

    let journal = Journal::for_squad(&home, "acme").unwrap();
    assert_eq!(text_entries(&journal.conversation(7).unwrap()), ["user: count", "reply: one two three"]);
    assert_eq!(journal.open_sessions().unwrap()[0].tokens, 6);
}

#[tokio::test]
async fn a_screen_attaching_mid_turn_sees_the_member_working() {
    let claude = fake_claude(true, "exec sleep 30");
    let cwd = scratch("cwd");
    let (handle, mut events) = runtime::start(config(&claude, &squad_home(&[&cwd]), true));
    add(&handle, &mut events, 3, member(&cwd, "pm")).await;
    handle.send(Command::Submit { session: 3, text: "long job".into() });
    until(&mut events, |e| matches!(e, Event::UserMessage { .. })).await;

    let snapshot = handle.snapshot("acme").await;
    assert_eq!(snapshot.sessions[0].state, SessionState::Working);
    assert_eq!(text_entries(&snapshot.sessions[0].recent), ["user: long job"]);
    handle.shutdown().await;
}

#[tokio::test]
async fn a_member_who_left_is_not_resumed() {
    let claude = fake_claude(true, &answering("ok"));
    let cwd = scratch("cwd");
    let home = squad_home(&[&cwd]);
    {
        let (handle, mut events) = runtime::start(config(&claude, &home, true));
        add(&handle, &mut events, 5, member(&cwd, "pm")).await;
        handle.send(Command::Close { session: 5 });
        until(&mut events, |e| matches!(e, Event::SessionClosed { .. })).await;
        handle.shutdown().await;
    }
    let (handle, _events) = runtime::start(config(&claude, &home, true));
    assert!(handle.snapshot("acme").await.sessions.is_empty());
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

// ── modes and permission ──

use equipe_core::protocol::{Decision, Mode};

/// A config whose members get equipe's tools — and so its permission prompts.
fn with_tools(claude: &Path, home: &Path) -> Config {
    Config { equipe_binary: Some("/opt/equipe".into()), ..config(claude, home, false) }
}

/// Put T-1 on the board for `owner` and wait until its first turn has run.
async fn first_ticket_turn(handle: &Handle, events: &mut UnboundedReceiver<Event>, recorded: &Path, owner: &str) -> Vec<String> {
    let before = calls(recorded).len();
    tool(handle, "pm", "create_ticket", serde_json::json!({ "title": "Hero", "body": "-", "owner": owner })).await;
    until(events, |e| matches!(e, Event::WorkStarted { .. })).await;
    until(events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    let all = calls(recorded);
    assert_eq!(all.len(), before + 1);
    all.last().unwrap().clone()
}

/// A stand-in for `npx`: writes down each launch's arguments, then listens on
/// the port it was given, as Playwright's MCP server would.
fn fake_npx() -> (PathBuf, PathBuf) {
    let dir = scratch("npx");
    let launches = dir.join("launches");
    std::fs::create_dir_all(&launches).unwrap();
    let bin = dir.join("npx");
    write_script(
        &bin,
        &format!(
            r#"#!/bin/sh
n=$(ls '{launches}' | wc -l | tr -d ' ')
for a in "$@"; do printf '%s\n' "$a"; done > '{launches}'/launch-$(printf %03d $n)-$$
port=""
while [ $# -gt 0 ]; do [ "$1" = "--port" ] && port="$2"; shift; done
exec python3 -c 'import socket,sys,time
s=socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1", int(sys.argv[1]))); s.listen(); time.sleep(600)' "$port"
"#,
            launches = launches.display()
        ),
    );
    (bin, launches)
}

/// Each browser launch's arguments, oldest first.
fn launches(dir: &Path) -> Vec<Vec<String>> {
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    files.sort();
    files.iter().map(|f| std::fs::read_to_string(f).unwrap().lines().map(str::to_owned).collect()).collect()
}

fn browser_of(call: &[String]) -> serde_json::Value {
    let mcp: serde_json::Value = serde_json::from_str(arg(call, "--mcp-config").unwrap()).unwrap();
    mcp["mcpServers"]["browser"].clone()
}

#[tokio::test]
async fn members_browse_from_the_start_and_as_the_person_once_signed_in() {
    let repo = repo();
    let (claude, recorded) = recording_claude(&answering("ok"));
    let home = squad_home(&[&repo]);
    let (npx, launched) = fake_npx();
    let (handle, mut events) = runtime::start(Config { npx: Some(npx), ..with_tools(&claude, &home) });
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    let turn = async |handle: &Handle, events: &mut UnboundedReceiver<Event>| {
        handle.send(Command::Submit { session: 1, text: "go".into() });
        until(events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
        calls(&recorded).pop().unwrap()
    };

    // Signed in nowhere yet: a browser all the same — public pages need no
    // account — starting from an empty set of sign-ins only the person can read.
    let call = turn(&handle, &mut events).await;
    let browser = browser_of(&call);
    assert_eq!(browser["type"], "http", "{browser}");
    let url = browser["url"].as_str().unwrap().to_owned();
    let state = equipe_core::browser::state_file(&home);
    assert_eq!(std::fs::read_to_string(&state).unwrap(), r#"{"cookies":[],"origins":[]}"#);
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(std::fs::metadata(&state).unwrap().permissions().mode() & 0o777, 0o600);

    // The browser outlives the turn: the next one finds the same, with whatever
    // the person signed in to meanwhile.
    std::fs::write(&state, "{}").unwrap();
    let call = turn(&handle, &mut events).await;
    assert_eq!(browser_of(&call)["url"], url.as_str(), "the same browser, kept between turns");
    let all = launches(&launched);
    assert_eq!(all.len(), 1, "launched once: {all:?}");
    let args = &all[0];
    let has = |flag: &str| args.iter().any(|a| a == flag);
    assert!(has(equipe_core::browser::MCP_PACKAGE), "{args:?}");
    let profile = equipe_core::browser::profile(&home, "acme", "pm");
    assert!(args.windows(2).any(|w| w[0] == "--user-data-dir" && w[1] == *profile.to_string_lossy()), "a profile of its own: {args:?}");
    assert!(has("--headless") && has("--shared-browser-context") && !has("--isolated"), "{args:?}");
    assert!(url.ends_with(&format!(":{}/mcp", args.windows(2).find(|w| w[0] == "--port").unwrap()[1])), "{url} {args:?}");
    assert!(call.iter().any(|a| a == "mcp__browser"), "in auto it browses without asking");
    let settings: serde_json::Value = serde_json::from_str(arg(&call, "--settings").unwrap()).unwrap();
    assert!(
        settings["permissions"]["ask"].as_array().unwrap().iter().any(|r| r == "mcp__browser__browser_run_code_unsafe"),
        "but code outside the sandbox asks first: {settings}"
    );
    let output = &args.windows(2).find(|w| w[0] == "--output-dir").unwrap()[1];
    assert!(output.starts_with(&*std::env::temp_dir().to_string_lossy()), "pages it saves stay out of the project: {output}");
    let charter = arg(&call, "--append-system-prompt").unwrap();
    assert!(charter.contains("`ask_person`") && charter.contains("CAPTCHA"), "what it asks the person first: {charter}");

    // Read-only changes nothing anywhere — not on a website either.
    handle.send(Command::SetMode { squad: "acme".into(), member: None, mode: Some(Mode::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::ModeChanged { member: None, .. })).await;
    let call = turn(&handle, &mut events).await;
    assert!(browser_of(&call).is_null(), "{call:?}");
}

#[tokio::test]
async fn the_browser_shows_on_screen_when_asked_and_its_code_tool_follows_the_mode() {
    let repo = repo();
    let (claude, recorded) = recording_claude(&answering("ok"));
    let home = squad_home(&[&repo]);
    let (npx, launched) = fake_npx();
    let (handle, mut events) = runtime::start(Config { npx: Some(npx), ..with_tools(&claude, &home) });
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    // A turn, and the arguments of the browser that turn used.
    let turn = async |handle: &Handle, events: &mut UnboundedReceiver<Event>, session: u64| {
        handle.send(Command::Submit { session, text: "go".into() });
        until(events, |e| matches!(e, Event::TurnEnded { session: s, .. } if *s == session)).await;
        let call = calls(&recorded).pop().unwrap();
        let port = browser_of(&call)["url"].as_str().unwrap().rsplit(':').next().unwrap().trim_end_matches("/mcp").to_owned();
        let args = launches(&launched).into_iter().rfind(|a| a.windows(2).any(|w| w[0] == "--port" && w[1] == port)).unwrap();
        let settings: serde_json::Value = arg(&call, "--settings").map_or(serde_json::Value::Null, |s| serde_json::from_str(s).unwrap());
        (call, args, settings)
    };
    let asks_for_code = |settings: &serde_json::Value| {
        settings["permissions"]["ask"].as_array().is_some_and(|a| a.iter().any(|r| r == "mcp__browser__browser_run_code_unsafe"))
    };

    // Headless by default; in auto, running code in the browser asks first.
    let (call, args, settings) = turn(&handle, &mut events, 1).await;
    assert!(args.iter().any(|a| a == "--headless"), "{args:?}");
    assert!(asks_for_code(&settings), "{settings}");
    assert!(!call.iter().any(|a| a == "mcp__browser__browser_run_code_unsafe"), "not blocked: {call:?}");

    // On screen for one member — a browser of its own, opened anew — the rest stay hidden.
    handle.send(Command::ShowBrowser { squad: "acme".into(), member: Some("pm".into()), show: true });
    let (_, args, _) = turn(&handle, &mut events, 1).await;
    assert!(!args.iter().any(|a| a == "--headless"), "{args:?}");
    let (_, args, _) = turn(&handle, &mut events, 2).await;
    assert!(args.iter().any(|a| a == "--headless"), "frontend's stays hidden: {args:?}");
    // For everyone, then hidden again — and kept in the squad's journal.
    handle.send(Command::ShowBrowser { squad: "acme".into(), member: None, show: true });
    let (_, args, _) = turn(&handle, &mut events, 2).await;
    assert!(!args.iter().any(|a| a == "--headless"), "{args:?}");
    handle.send(Command::ShowBrowser { squad: "acme".into(), member: Some("frontend".into()), show: false });
    let (_, args, _) = turn(&handle, &mut events, 2).await;
    assert!(args.iter().any(|a| a == "--headless"), "{args:?}");
    let (_, args, _) = turn(&handle, &mut events, 1).await;
    assert!(!args.iter().any(|a| a == "--headless"), "pm still shows: {args:?}");
    assert!(Journal::for_squad(&home, "acme").unwrap().browser_shown("pm"));

    // In full, the code runs without asking.
    handle.send(Command::SetMode { squad: "acme".into(), member: None, mode: Some(Mode::Full) });
    until(&mut events, |e| matches!(e, Event::ModeChanged { .. })).await;
    let (_, _, settings) = turn(&handle, &mut events, 1).await;
    assert!(!asks_for_code(&settings), "{settings}");
}

#[tokio::test]
async fn running_code_in_the_browser_shows_the_code_to_the_person() {
    let repo = repo();
    let claude = fake_claude(true, "exec sleep 30");
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&repo])));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    let code = "async (page) => {\n  await page.goto('https://vercel.com');\n  return page.title();\n}";
    let handle2 = handle.clone();
    let arguments = serde_json::json!({ "tool_name": "mcp__browser__browser_run_code_unsafe", "input": { "code": code } });
    let waiting = tokio::spawn(async move { tool(&handle2, "frontend", "approve", arguments).await });
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = asked.last() else { unreachable!() };
    assert_eq!(approval.action, code, "the person sees the code itself");
    assert!(approval.reason.as_deref().is_some_and(|r| r.contains("outside the sandbox")), "{:?}", approval.reason);
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Allow });
    let verdict: serde_json::Value = serde_json::from_str(&waiting.await.unwrap().text).unwrap();
    assert_eq!(verdict["behavior"], "allow");
}

#[tokio::test]
async fn the_project_in_focus_travels_with_the_persons_message() {
    let (site, app) = (named("site"), named("app"));
    let (claude, recorded) = recording_claude(&answering("ok"));
    let (handle, mut events) = runtime::start(config(&claude, &squad_home(&[&site, &app]), false));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    handle.send(Command::Focus { squad: "acme".into(), project: Some("app".into()) });
    handle.send(Command::Submit { session: 1, text: "add an onboarding screen".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    let prompt = arg(&calls(&recorded)[0], "-p").unwrap().to_owned();
    assert!(prompt.contains("looking at the project app") && prompt.ends_with("add an onboarding screen"), "{prompt}");
    // Back to all: about the whole squad, in the same conversation.
    handle.send(Command::Focus { squad: "acme".into(), project: None });
    handle.send(Command::Submit { session: 1, text: "and the site?".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    let last = calls(&recorded).last().unwrap().clone();
    let prompt = arg(&last, "-p").unwrap().to_owned();
    assert!(!prompt.contains("the project app") && prompt.contains("looking at the whole squad"), "{prompt}");
    assert_eq!(arg(&last, "--resume"), Some("sess-1"));
}

/// A turn that reports how full its context got: `used` of a 200k window.
fn answering_with_context(text: &str, used: u64) -> String {
    format!(
        r#"cat <<EOF
{{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}}
{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{text}"}}],"usage":{{"input_tokens":5,"cache_read_input_tokens":{used},"cache_creation_input_tokens":0,"output_tokens":0}}}}}}
{{"type":"result","subtype":"success","is_error":false,"result":"{text}","usage":{{"input_tokens":5,"cache_read_input_tokens":{used},"output_tokens":2}},"modelUsage":{{"claude-test":{{"contextWindow":200000}}}}}}
EOF"#
    )
}

#[tokio::test]
async fn a_desk_conversation_grown_too_full_starts_afresh_from_a_summary() {
    let cwd = scratch("cwd");
    let home = squad_home(&[&cwd]);
    let (claude, recorded) = recording_claude(&answering_with_context("noted", 150_000));
    let (handle, mut events) = runtime::start(Config { persist: true, ..with_tools(&claude, &home) });
    add(&handle, &mut events, 1, member(&cwd, "pm")).await;
    let turn = async |handle: &Handle, events: &mut UnboundedReceiver<Event>, text: &str| {
        handle.send(Command::Submit { session: 1, text: text.into() });
        until(events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await
    };

    // The turn says it filled 150k of 200k: counted with its cache, and seen.
    let ended = turn(&handle, &mut events, "plan the launch").await;
    let Some(Event::TurnEnded { usage: Some(usage), .. }) = ended.last() else { panic!("{ended:?}") };
    assert_eq!((usage.context_tokens, usage.context_window), (Some(150_005), Some(200_000)));
    assert_eq!(usage.total(), 5 + 150_000 + 2, "cached input counts");
    let pm = handle.snapshot("acme").await.sessions.into_iter().find(|s| s.session == 1).unwrap();
    assert_eq!(pm.context, Some((150_005, 200_000)));

    // 75% full: the next turn starts a fresh conversation, handed where the last left off.
    let ended = turn(&handle, &mut events, "and the pricing?").await;
    assert!(ended.iter().any(|e| matches!(e, Event::Notice { text, .. } if text.contains("a fresh conversation: the last one had filled 150k of 200k"))), "{ended:?}");
    let call = calls(&recorded).pop().unwrap();
    assert_eq!(arg(&call, "--resume"), None, "not the old conversation");
    let prompt = arg(&call, "-p").unwrap();
    assert!(prompt.contains("this is a fresh conversation") && prompt.contains("- the person: plan the launch") && prompt.contains("- you: noted"), "{prompt}");
    assert!(prompt.ends_with("and the pricing?"), "{prompt}");
}

#[tokio::test]
async fn the_person_can_start_a_members_conversation_clean() {
    let cwd = scratch("cwd");
    let (claude, recorded) = recording_claude(&answering("ok"));
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&cwd])));
    add(&handle, &mut events, 1, member(&cwd, "pm")).await;
    handle.send(Command::Submit { session: 1, text: "first".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    handle.send(Command::Refresh { session: 1, summary: false });
    until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("starts clean"))).await;
    handle.send(Command::Submit { session: 1, text: "second".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    let call = calls(&recorded).pop().unwrap();
    assert_eq!(arg(&call, "--resume"), None);
    let prompt = arg(&call, "-p").unwrap();
    assert!(prompt.starts_with("[equipe: this is a fresh conversation. Your memory") && !prompt.contains("first"), "{prompt}");
}

#[tokio::test]
async fn work_that_waits_waits_for_its_role_and_a_rush_member_takes_nothing_new() {
    let repo = repo();
    let claude = fake_claude(true, "exec sleep 30");
    let home = squad_home(&[&repo]);
    let (handle, mut events) = runtime::start(with_tools(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "backend")).await;
    // backend-2, brought in for a rush by an older equipe: it finishes what it holds and takes nothing new.
    add(&handle, &mut events, 3, member(&repo, "backend")).await;
    Journal::for_squad(&home, "acme").unwrap().set_helpers(&[3]).unwrap();
    for (title, owner) in [("API", "backend"), ("Webhooks", "backend-2"), ("Exports", "backend")] {
        tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": title, "body": "-", "owner": owner })).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let snapshot = handle.snapshot("acme").await;
    assert_eq!(snapshot.sessions.len(), 3, "nobody is brought in, whatever waits");
    let working: Vec<_> = snapshot.tickets.iter().filter(|t| t.status == TicketStatus::InProgress).collect();
    assert_eq!(working.len(), 1, "{working:?}");
    assert_eq!(working[0].assignee.as_deref(), Some("backend"));
}

#[tokio::test]
async fn a_member_asks_the_person_before_doing_something_on_their_behalf() {
    let repo = repo();
    let claude = fake_claude(true, "exec sleep 30");
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&repo])));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "backend")).await;
    // Even in full access: this is the person's call, not the mode's.
    handle.send(Command::SetMode { squad: "acme".into(), member: None, mode: Some(Mode::Full) });
    until(&mut events, |e| matches!(e, Event::ModeChanged { .. })).await;

    let ask = |action: &str| {
        let handle = handle.clone();
        let arguments = serde_json::json!({ "action": action, "why": "T-10 needs a project key" });
        tokio::spawn(async move { tool(&handle, "backend", "ask_person", arguments).await })
    };
    let waiting = ask("Sign up for PostHog with GitHub (EU region)");
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = asked.last() else { unreachable!() };
    assert_eq!((approval.member.as_str(), approval.tool.as_str()), ("backend", "Confirm"));
    assert_eq!(approval.action, "Sign up for PostHog with GitHub (EU region)");
    assert_eq!(approval.reason.as_deref(), Some("T-10 needs a project key"));
    assert!(equipe_core::permissions::rule_for(&approval.tool, &approval.action).is_none(), "never an \"always\"");
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Allow });
    let answer = waiting.await.unwrap();
    assert!(!answer.is_error && answer.text.contains("yes"), "{}", answer.text);

    let waiting = ask("Start the Vercel Pro trial");
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = asked.last() else { unreachable!() };
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Deny });
    let answer = waiting.await.unwrap();
    assert!(answer.text.contains("no"), "{}", answer.text);
}

#[tokio::test]
async fn in_auto_a_member_on_a_ticket_is_fenced_in_and_asks_for_the_rest() {
    let repo = repo();
    let (claude, recorded) = recording_claude(&answering("ok"));
    let home = squad_home(&[&repo]);
    // Once always allowed, vercel neither asks again nor finds its service fenced off.
    Journal::for_squad(&home, "acme").unwrap().allow("Bash(vercel:*)", "you").unwrap();
    let (handle, mut events) = runtime::start(with_tools(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    let call = first_ticket_turn(&handle, &mut events, &recorded, "frontend").await;

    assert_eq!(arg(&call, "--permission-mode"), Some("acceptEdits"));
    let settings: serde_json::Value = serde_json::from_str(arg(&call, "--settings").unwrap()).unwrap();
    assert_eq!(settings["sandbox"]["enabled"], true, "the operating system fences its commands in");
    assert_eq!(settings["sandbox"]["autoAllowBashIfSandboxed"], true, "inside the fence, no questions");
    assert!(settings["sandbox"]["network"]["allowedDomains"].as_array().unwrap().iter().any(|d| d == "registry.npmjs.org"));
    assert!(settings["permissions"]["ask"].as_array().unwrap().iter().any(|r| r == "Bash(git push:*)"), "pushing asks");
    assert!(!settings["permissions"]["ask"].as_array().unwrap().iter().any(|r| r == "Bash(vercel:*)"), "always means always");
    assert!(settings["sandbox"]["network"]["allowedDomains"].as_array().unwrap().iter().any(|d| d == "*.vercel.com"));
    assert_eq!(arg(&call, "--permission-prompt-tool"), Some("mcp__equipe__approve"));
    assert!(call.iter().any(|a| a == "Bash(sudo:*)"), "the blocklist travels with every member turn");
    let unread = settings["sandbox"]["filesystem"]["denyRead"].as_array().unwrap();
    assert!(unread.iter().any(|p| p.as_str().unwrap().ends_with("/browser")), "the person's sign-ins: {unread:?}");
}

#[tokio::test]
async fn the_mode_decides_how_a_ticket_runs_and_a_member_can_differ_from_the_squad() {
    let repo = repo();
    let (claude, recorded) = recording_claude(&answering("ok"));
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&repo])));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;

    handle.send(Command::SetMode { squad: "acme".into(), member: None, mode: Some(Mode::ReadOnly) });
    until(&mut events, |e| matches!(e, Event::ModeChanged { member: None, .. })).await;
    let call = first_ticket_turn(&handle, &mut events, &recorded, "frontend").await;
    assert_eq!(arg(&call, "--permission-mode"), Some("dontAsk"), "plan reads, even on a ticket");
    assert_eq!(arg(&call, "--permission-prompt-tool"), None);

    handle.send(Command::SetMode { squad: "acme".into(), member: Some("frontend".into()), mode: Some(Mode::Ask) });
    until(&mut events, |e| matches!(e, Event::ModeChanged { member: Some(_), .. })).await;
    handle.send(Command::Submit { session: 2, text: "go on".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    let call = calls(&recorded).pop().unwrap();
    assert_eq!(arg(&call, "--permission-mode"), Some("manual"), "ask me: every change is asked");
    assert_eq!(arg(&call, "--permission-prompt-tool"), Some("mcp__equipe__approve"));

    let snapshot = handle.snapshot("acme").await;
    assert_eq!(snapshot.mode, Mode::ReadOnly);
    assert_eq!(snapshot.sessions.iter().find(|s| s.session == 2).unwrap().mode, Some(Mode::Ask));
}

/// A member asks to run `command`; returns the waiting answer.
fn ask(handle: &Handle, command: &str) -> tokio::task::JoinHandle<ToolResult> {
    let handle = handle.clone();
    let arguments = serde_json::json!({
        "tool_name": "Bash",
        "input": { "command": command, "description": "Install dependencies" },
        "tool_use_id": "toolu_1",
    });
    tokio::spawn(async move {
        handle.tool(ToolCall { squad: "acme".into(), member: "frontend".into(), name: "approve".into(), arguments }).await
    })
}

#[tokio::test]
async fn a_request_for_permission_waits_for_the_person_and_always_becomes_a_rule() {
    let repo = repo();
    let claude = fake_claude(true, "exec sleep 30");
    let home = squad_home(&[&repo]);
    let (handle, mut events) = runtime::start(with_tools(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    // Ask me: every command is put to the person.
    handle.send(Command::SetMode { squad: "acme".into(), member: None, mode: Some(Mode::Ask) });
    until(&mut events, |e| matches!(e, Event::ModeChanged { .. })).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Hero", "body": "-", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { .. })).await;

    let waiting = ask(&handle, "npm install react");
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = asked.last() else { unreachable!() };
    assert_eq!((approval.member.as_str(), approval.ticket, approval.action.as_str()), ("frontend", Some(1), "npm install react"));
    assert_eq!(approval.reason.as_deref(), Some("Install dependencies"));
    assert_eq!(handle.snapshot("acme").await.approvals.len(), 1, "a screen attaching now sees it waiting");
    assert!(!waiting.is_finished(), "the member waits for the person");

    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Always });
    let answer = waiting.await.unwrap();
    let verdict: serde_json::Value = serde_json::from_str(&answer.text).unwrap();
    assert_eq!(verdict["behavior"], "allow");
    assert_eq!(verdict["updatedInput"]["command"], "npm install react");
    let seen = until(&mut events, |e| matches!(e, Event::ApprovalResolved { .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { session: 2, text } if text.starts_with("✓ You always allow Bash(npm install:*)"))));
    assert_eq!(Journal::for_squad(&home, "acme").unwrap().allowed().unwrap(), ["Bash(npm install:*)"]);

    // The same kind of thing doesn't ask again; something else does.
    let again = ask(&handle, "npm install lodash").await.unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&again.text).unwrap()["behavior"], "allow");
    let other = ask(&handle, "curl https://example.com");
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = asked.last() else { unreachable!() };
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Deny });
    let no: serde_json::Value = serde_json::from_str(&other.await.unwrap().text).unwrap();
    assert_eq!((no["behavior"].as_str(), no["message"].as_str()), (Some("deny"), Some("The person declined.")));
    handle.shutdown().await;
}

#[tokio::test]
async fn an_always_allow_rule_is_listed_for_screens_and_taken_back_asks_again() {
    let repo = repo();
    let claude = fake_claude(true, "exec sleep 30");
    let home = squad_home(&[&repo]);
    let (handle, mut events) = runtime::start(with_tools(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    handle.send(Command::SetMode { squad: "acme".into(), member: None, mode: Some(Mode::Ask) });
    until(&mut events, |e| matches!(e, Event::ModeChanged { .. })).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Hero", "body": "-", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { .. })).await;
    assert!(handle.snapshot("acme").await.allowed.is_empty());

    let waiting = ask(&handle, "npm install react");
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = asked.last() else { unreachable!() };
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Always });
    waiting.await.unwrap();
    let seen = until(&mut events, |e| matches!(e, Event::Allowed { .. })).await;
    assert!(matches!(seen.last(), Some(Event::Allowed { squad, rules }) if squad == "acme" && rules == &["Bash(npm install:*)"]), "{seen:#?}");
    assert_eq!(handle.snapshot("acme").await.allowed, ["Bash(npm install:*)"], "a screen attaching now lists it");

    // Another squad's word changes nothing here; a rule it never had, nothing at all.
    handle.send(Command::Disallow { squad: "other".into(), rule: "Bash(npm install:*)".into() });
    handle.send(Command::Disallow { squad: "acme".into(), rule: "Bash(npm install:*)".into() });
    let seen = until(&mut events, |e| matches!(e, Event::Allowed { .. })).await;
    assert!(matches!(seen.last(), Some(Event::Allowed { squad, rules }) if squad == "acme" && rules.is_empty()), "{seen:#?}");
    assert!(handle.snapshot("acme").await.allowed.is_empty());
    assert_eq!(Journal::for_squad(&home, "acme").unwrap().allowed().unwrap(), Vec::<String>::new());

    // Taken back, the same kind of thing asks again.
    let _again = ask(&handle, "npm install lodash");
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    assert!(matches!(asked.last(), Some(Event::ApprovalRequested { approval, .. }) if approval.action == "npm install lodash"));
    handle.shutdown().await;
}

#[tokio::test]
async fn in_auto_a_command_that_stays_in_the_sandbox_is_not_asked_about() {
    let repo = repo();
    let claude = fake_claude(true, "exec sleep 30");
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&repo])));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Hero", "body": "-", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { .. })).await;
    // A heredoc Claude Code asks about anyway: inside the fence, equipe allows it.
    let inside = ask(&handle, "mkdir -p src && cat > src/a.ts <<'EOF'\nexport {}\nEOF").await.unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&inside.text).unwrap()["behavior"], "allow");
    // Pushing still asks.
    let _push = ask(&handle, "npm run build && git push origin main");
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    assert!(matches!(asked.last(), Some(Event::ApprovalRequested { approval, .. }) if approval.action.contains("git push")));
    handle.shutdown().await;
}

#[tokio::test]
async fn blocked_commands_are_refused_without_asking() {
    let repo = repo();
    let claude = fake_claude(true, "exec sleep 30");
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&repo])));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    let answer = ask(&handle, "npm test && sudo rm -rf /var").await.unwrap();
    let verdict: serde_json::Value = serde_json::from_str(&answer.text).unwrap();
    assert_eq!(verdict["behavior"], "deny");
    let seen = until(&mut events, |e| matches!(e, Event::Notice { session: 2, .. })).await;
    assert!(matches!(seen.last(), Some(Event::Notice { text, .. }) if text.starts_with("✗ equipe blocked")));
    handle.shutdown().await;
}

#[tokio::test]
async fn stopping_a_member_withdraws_what_it_was_waiting_for() {
    let repo = repo();
    let claude = fake_claude(true, "exec sleep 30");
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&repo])));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Hero", "body": "-", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { .. })).await;
    let waiting = ask(&handle, "npm publish");
    until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    handle.send(Command::Interrupt { session: 2 });
    let resolved = until(&mut events, |e| matches!(e, Event::ApprovalResolved { .. })).await;
    assert!(matches!(resolved.last(), Some(Event::ApprovalResolved { decision: Decision::Deny, .. })));
    assert_eq!(serde_json::from_str::<serde_json::Value>(&waiting.await.unwrap().text).unwrap()["behavior"], "deny");
    assert!(handle.snapshot("acme").await.approvals.is_empty());
    handle.shutdown().await;
}

/// Write an executable script without this process ever holding it open for
/// writing: on Linux, a test thread forking at that moment would carry the
/// open file into its child, and running the script would fail with "Text
/// file busy". The script is written beside it and copied in by `cp`.
fn write_script(path: &std::path::Path, script: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    let draft = path.with_extension("draft");
    std::fs::write(&draft, script).unwrap();
    let copied = std::process::Command::new("cp").arg(&draft).arg(path).status().unwrap();
    assert!(copied.success());
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

// ── the squad home ──

/// The settings a recorded turn passed to Claude Code.
fn settings_of(call: &[String]) -> serde_json::Value {
    serde_json::from_str(arg(call, "--settings").unwrap()).unwrap()
}

#[tokio::test]
async fn at_its_desk_a_member_writes_the_squads_home_each_change_saved_and_taken_back() {
    let (claude, recorded) = recording_claude(&format!("mkdir -p notes && printf '5월 31일\\n' > notes/tax-deadline.md\n{}", answering("saved")));
    let (site, app) = (named("site"), named("app"));
    let home = squad_home(&[&site, &app]);
    let folder = squads_home(&home);
    let (handle, mut events) = runtime::start(with_tools(&claude, &home));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    // The person edits in Finder before the turn: that is theirs, kept apart.
    std::fs::write(folder.join("notes/mine.md"), "mine\n").unwrap();
    handle.send(Command::Submit { session: 1, text: "세금 마감일 메모해줘".into() });
    let seen = until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.starts_with("✎"))).await;
    assert!(
        seen.iter().any(|e| matches!(e, Event::Notice { session: 1, text }
            if text == "✎ pm saved notes/tax-deadline.md in the squad's folder · /home undo takes it back")),
        "{seen:#?}"
    );

    let call = &calls(&recorded)[0];
    assert_eq!(arg(call, "--permission-mode"), Some("acceptEdits"), "in auto it writes its home on its own");
    let changes = squad_home::changes(&folder, 10).unwrap();
    let saved: Vec<(&str, &str, String)> = changes.iter().map(|c| (c.by.as_str(), c.why.as_str(), c.what())).collect();
    assert_eq!(
        saved,
        [
            ("pm", "세금 마감일 메모해줘", "saved notes/tax-deadline.md".to_owned()),
            (squad_home::PERSON, "changed in the squad's folder", "saved notes/mine.md".to_owned()),
        ]
    );

    // The person takes it back; their own note stays.
    squad_home::undo(&folder, 1).unwrap();
    assert!(!folder.join("notes/tax-deadline.md").exists() && folder.join("notes/mine.md").exists());
}

fn squads_home(home: &Path) -> PathBuf {
    squad_home::path(home, "acme")
}

#[tokio::test]
async fn at_its_desk_the_projects_are_only_read_and_other_squads_folders_not_at_all() {
    let (claude, recorded) = recording_claude(&answering("ok"));
    let (site, app) = (named("site"), named("app"));
    let home = squad_home(&[&site, &app]);
    squads::create(&home, "beta", &named("beta-app")).unwrap();
    let beta = squad_home::path(&home, "beta");
    let (handle, mut events) = runtime::start(with_tools(&claude, &home));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    add(&handle, &mut events, 2, member(&site, "frontend")).await;
    handle.send(Command::Submit { session: 2, text: "look around".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;

    let call = &calls(&recorded)[0];
    let added: Vec<&str> = call.windows(2).filter(|w| w[0] == "--add-dir").map(|w| w[1].as_str()).collect();
    assert_eq!(added, [site.to_str().unwrap(), app.to_str().unwrap()], "every project, read from the home");
    let settings = settings_of(call);
    let deny: Vec<&str> = settings["permissions"]["deny"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
    for rule in [
        format!("Edit(/{}/**)", site.display()),
        format!("Write(/{}/**)", app.display()),
        format!("Read(/{}/**)", beta.display()),
    ] {
        assert!(deny.contains(&rule.as_str()), "{rule} in {deny:?}");
    }
    assert!(!deny.iter().any(|r| r.contains("/acme/home")), "its own home is its to write");
    assert_eq!(settings["sandbox"]["filesystem"]["denyWrite"], serde_json::json!([site, app]));
    assert!(settings["sandbox"]["filesystem"]["denyRead"].as_array().unwrap().contains(&serde_json::json!(beta)));

    // Asked through equipe, the same: refused in any mode, never put to the person.
    handle.send(Command::SetMode { squad: "acme".into(), member: None, mode: Some(Mode::Full) });
    until(&mut events, |e| matches!(e, Event::ModeChanged { .. })).await;
    let approve = |tool_name: &str, input: serde_json::Value| {
        tool(&handle, "frontend", "approve", serde_json::json!({ "tool_name": tool_name, "input": input, "tool_use_id": "t" }))
    };
    let edit = approve("Edit", serde_json::json!({ "file_path": site.join("README.md") })).await;
    let edit: serde_json::Value = serde_json::from_str(&edit.text).unwrap();
    assert_eq!(edit["behavior"], "deny");
    assert!(edit["message"].as_str().unwrap().contains("only read"), "{edit}");
    let shell = approve("Bash", serde_json::json!({ "command": format!("echo x > {}/a.txt", app.display()) })).await;
    assert_eq!(serde_json::from_str::<serde_json::Value>(&shell.text).unwrap()["behavior"], "deny");
    let peek = approve("Read", serde_json::json!({ "file_path": beta.join("README.md") })).await;
    let peek: serde_json::Value = serde_json::from_str(&peek.text).unwrap();
    assert_eq!(peek["behavior"], "deny");
    assert!(peek["message"].as_str().unwrap().contains("private"), "{peek}");
    assert!(handle.snapshot("acme").await.approvals.is_empty(), "nothing was put to the person");
}

#[tokio::test]
async fn a_squad_from_before_homes_gets_one_and_its_desks_move_there_afresh() {
    let (claude, recorded) = recording_claude(&answering("ok"));
    let site = named("site");
    let home = squad_home(&[&site]);
    let folder = squads_home(&home);
    {
        let (handle, mut events) = runtime::start(config(&claude, &home, true));
        add(&handle, &mut events, 1, member(&site, "pm")).await;
        handle.send(Command::Submit { session: 1, text: "hello".into() });
        until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
        handle.shutdown().await;
    }
    // As it was before squad homes: no home, and a desk opened in the project.
    std::fs::remove_dir_all(&folder).unwrap();
    Journal::for_squad(&home, "acme").unwrap().set_cwd(1, &site).unwrap();

    let (handle, mut events) = runtime::start(config(&claude, &home, true));
    assert!(folder.join(".git").is_dir() && folder.join("notes").is_dir(), "made on start");
    assert_eq!(handle.snapshot("acme").await.sessions[0].project, folder);
    handle.send(Command::Submit { session: 1, text: "again".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    let call = calls(&recorded).pop().unwrap();
    assert_eq!(arg(&call, "--resume"), None, "a conversation begun in the project can't go on in the home");
    assert!(arg(&call, "-p").unwrap().contains("fresh conversation"));
}

// ── tickets that aren't code ──

/// A ticket's turn that writes `file` (relative to its worktree) with `text`.
fn writing_turn(file: &str, text: &str) -> String {
    format!(
        "case \"$*\" in *\"You're on T-\"*) sleep 1; mkdir -p \"$(dirname '{file}')\"; echo '{text}' > '{file}';; esac\n{}",
        answering("working on it")
    )
}

#[tokio::test]
async fn a_task_with_no_project_runs_in_the_squads_folder_and_goes_in_when_accepted() {
    let (site, app) = (named("site"), named("app"));
    let (claude, recorded) = recording_claude(&writing_turn("outputs/T-1 research/report.md", "three competitors"));
    let home = squad_home(&[&site, &app]);
    let folder = squads_home(&home);
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    // A role that doesn't edit code writes in its worktree of the squad's folder all the same.
    add(&handle, &mut events, 2, member(&site, "data")).await;

    // Two projects, none named: a task, and it is handed out.
    let made = tool(&handle, "pm", "create_ticket", serde_json::json!({
        "title": "Competitor research", "body": "Who else does this?", "owner": "data",
    }))
    .await;
    assert_eq!(made.text, "Created T-1 (task).");
    let started = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let Some(Event::WorkStarted { session: 2, workspace: Some(ws), .. }) = started.last() else { panic!("{started:#?}") };
    assert!(ws.path.starts_with(home.join("worktrees/acme.home")), "{}", ws.path.display());
    assert!(ws.branch.starts_with("equipe/data-t1-"), "{}", ws.branch);

    tokio::time::sleep(Duration::from_millis(300)).await;
    let call = calls(&recorded).pop().unwrap();
    let prompt = arg(&call, "-p").unwrap();
    assert!(prompt.contains("Kind: task") && prompt.contains("outputs/T-1"), "{prompt}");
    assert_eq!(arg(&call, "--permission-mode"), Some("acceptEdits"));
    let read: Vec<&str> = call.windows(2).filter(|w| w[0] == "--add-dir").map(|w| w[1].as_str()).collect();
    for project in [&site, &app] {
        assert!(read.contains(&project.to_str().unwrap()), "every project is read: {read:?}");
    }

    // Handed in: the review shows the summary and the files it leaves.
    tool(&handle, "data", "submit_for_review", serde_json::json!({ "summary": "Three found." })).await;
    let review =
        until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.status == TicketStatus::Review)).await;
    let ticket = ticket_changed(&review).pop().unwrap();
    assert_eq!(ticket.note.as_deref(), Some("Three found.\n\nIn the squad's folder:\n- outputs/T-1 research/report.md\n\nIts files cite no source — check before relying on them."));
    assert!(!folder.join("outputs/T-1 research/report.md").exists(), "not in the folder before it is accepted");

    // Accepted: in the squad home's main at once — no merge question, no branch left.
    handle.send(Command::Accept { squad: "acme".into(), ticket: 1 });
    let done =
        until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.status == TicketStatus::Done)).await;
    let ticket = ticket_changed(&done).pop().unwrap();
    assert_eq!((ticket.branch.as_deref(), ticket.merged, ticket.not_on_main()), (None, None, false));
    assert_eq!(std::fs::read_to_string(folder.join("outputs/T-1 research/report.md")).unwrap().trim(), "three competitors");
    assert_eq!(git(&folder, &["status", "--porcelain"]), "", "saved in the folder");
    assert!(!ws.path.exists());
    assert_eq!(git(&folder, &["branch", "--list", "equipe/*"]), "");
    let changes = squad_home::changes(&folder, 1).unwrap();
    assert_eq!(changes[0].by, "data");
    let notices = until(&mut events, |e| matches!(e, Event::Notice { session: 1, .. })).await;
    assert!(
        matches!(notices.last(), Some(Event::Notice { text, .. }) if text.contains("its work is in the squad's folder")),
        "{notices:#?}"
    );

    // Merging is not a thing for it.
    handle.send(Command::Merge { squad: "acme".into(), ticket: 1, push: false });
    let refused = until(&mut events, |e| matches!(e, Event::Error { .. })).await;
    assert!(matches!(refused.last(), Some(Event::Error { message, .. }) if message.contains("went into the squad's folder")), "{refused:#?}");
}

#[tokio::test]
async fn a_task_whose_folder_changed_under_it_goes_back_to_its_member_instead_of_closing() {
    let (site, app) = (named("site"), named("app"));
    let claude = fake_claude(true, &writing_turn("notes/plan.md", "the squads plan"));
    let home = squad_home(&[&site, &app]);
    let folder = squads_home(&home);
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Plan", "kind": "task", "owner": "pm" })).await;
    let started = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let Some(Event::WorkStarted { workspace: Some(ws), .. }) = started.last() else { panic!("{started:#?}") };
    tool(&handle, "pm", "submit_for_review", serde_json::json!({ "summary": "Planned." })).await;
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.status == TicketStatus::Review)).await;

    // Meanwhile the person wrote the same note themselves.
    std::fs::write(folder.join("notes/plan.md"), "my plan\n").unwrap();
    handle.send(Command::Accept { squad: "acme".into(), ticket: 1 });
    let back = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    assert!(
        matches!(back.last(), Some(Event::WorkStarted { workspace: Some(again), .. }) if again.path == ws.path),
        "same worktree: {back:#?}"
    );
    let tickets = ticket_changed(&back);
    assert!(tickets.iter().all(|t| t.status != TicketStatus::Done), "{tickets:#?}");
    assert!(tickets.iter().any(|t| t.note.as_deref().is_some_and(|n| n.contains("the squad's folder changed"))), "{tickets:#?}");
    // Nothing of the task went in; the person's note is kept, as theirs.
    assert_eq!(std::fs::read_to_string(folder.join("notes/plan.md")).unwrap(), "my plan\n");
    assert_eq!(squad_home::changes(&folder, 1).unwrap()[0].by, squad_home::PERSON);
}

#[tokio::test]
async fn a_code_ticket_still_needs_its_project_when_the_squad_has_several() {
    let (site, app) = (repo(), repo());
    let claude = fake_claude(true, &answering("ok"));
    let (handle, mut events) = runtime::start(config(&claude, &squad_home(&[&site, &app]), false));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    add(&handle, &mut events, 2, member(&site, "backend")).await;
    let app_name = app.file_name().unwrap().to_str().unwrap();
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "API", "kind": "code", "project": app_name, "owner": "backend" })).await;
    let started = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let Some(Event::WorkStarted { workspace: Some(ws), project, .. }) = started.last() else { panic!("{started:#?}") };
    assert_eq!(project, &app);
    assert!(ws.path.parent().unwrap().ends_with(Path::new("worktrees").join(app_name)), "{}", ws.path.display());
}

// ── the squad's memory ──

#[tokio::test]
async fn what_one_member_learns_the_next_one_knows_and_the_person_has_the_last_word() {
    let (claude, recorded) = recording_claude(&answering("noted"));
    let site = named("site");
    let home = squad_home(&[&site]);
    let folder = squads_home(&home);
    let (handle, mut events) = runtime::start(with_tools(&claude, &home));
    add(&handle, &mut events, 1, member(&site, "pm")).await;

    // The person tells pm in conversation; pm keeps it for the squad, as its engine would through equipe's tool.
    handle.send(Command::Submit { session: 1, text: "나는 매년 5월에 종합소득세를 직접 신고해".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    let fact = serde_json::json!({
        "name": "tax-filing",
        "description": "사람은 매년 5월에 종합소득세를 직접 신고한다",
        "kind": "deadline",
    });
    let saved = tool(&handle, "pm", "remember", fact).await;
    assert!(!saved.is_error, "{}", saved.text);
    let seen = until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.starts_with("✎"))).await;
    assert!(
        seen.iter().any(|e| matches!(e, Event::Notice { session: 1, text }
            if text == "✎ pm remembered for the squad: 사람은 매년 5월에 종합소득세를 직접 신고한다 · /memory shows what the squad knows")),
        "{seen:#?}"
    );
    let latest = squad_home::changes(&folder, 1).unwrap().remove(0);
    assert_eq!((latest.by.as_str(), latest.what()), ("pm", "saved memory/tax-filing.md".to_owned()));

    // backend joins later, in a session of its own, and knows it from its first turn.
    add(&handle, &mut events, 2, member(&site, "backend")).await;
    handle.send(Command::Submit { session: 2, text: "5월에 할 일 있어?".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    let charter = arg(&calls(&recorded).pop().unwrap(), "--append-system-prompt").unwrap().to_owned();
    assert!(charter.contains("- [tax-filing](tax-filing.md) (deadline, "), "{charter}");
    assert!(charter.contains("사람은 매년 5월에 종합소득세를 직접 신고한다"), "{charter}");
    assert!(charter.contains(&format!("The squad's memory is {}/memory", folder.display())), "{charter}");

    // The person fixes it in Finder: the next turn has their words, and their change is saved as theirs.
    let file = folder.join("memory/tax-filing.md");
    let fixed = std::fs::read_to_string(&file).unwrap().replace("직접 신고한다", "세무사에게 맡긴다");
    std::fs::write(&file, fixed).unwrap();
    handle.send(Command::Submit { session: 2, text: "다시 확인해줘".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    let charter = arg(&calls(&recorded).pop().unwrap(), "--append-system-prompt").unwrap().to_owned();
    assert!(charter.contains("사람은 매년 5월에 종합소득세를 세무사에게 맡긴다") && !charter.contains("직접 신고한다"), "{charter}");
    let latest = squad_home::changes(&folder, 1).unwrap().remove(0);
    assert_eq!((latest.by.as_str(), latest.what()), (squad_home::PERSON, "updated memory/tax-filing.md".to_owned()));
    // A member doesn't write over what the person wrote without their yes.
    let overwrite = serde_json::json!({ "name": "tax-filing", "description": "직접 신고한다", "kind": "deadline" });
    assert!(tool(&handle, "backend", "remember", overwrite).await.text.contains("what they wrote wins"));

    // The person deletes it: gone from the next turn, and from the index.
    std::fs::remove_file(&file).unwrap();
    handle.send(Command::Submit { session: 2, text: "또?".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    let charter = arg(&calls(&recorded).pop().unwrap(), "--append-system-prompt").unwrap().to_owned();
    assert!(!charter.contains("tax-filing") && charter.contains("The squad remembers nothing yet."), "{charter}");
    assert!(!std::fs::read_to_string(folder.join("memory/INDEX.md")).unwrap().contains("tax-filing"));
}

#[tokio::test]
async fn a_secret_value_is_refused_and_another_squad_never_sees_the_memory() {
    let (claude, recorded) = recording_claude(&answering("ok"));
    let site = named("site");
    let home = squad_home(&[&site]);
    squads::create(&home, "beta", &named("beta-app")).unwrap();
    let (handle, mut events) = runtime::start(with_tools(&claude, &home));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    let secret = serde_json::json!({ "name": "id", "description": "주민번호 900101-1234567", "kind": "person" });
    let refused = tool(&handle, "pm", "remember", secret).await;
    assert!(refused.is_error && refused.text.contains("resident registration number"), "{}", refused.text);
    let fact = serde_json::json!({ "name": "id-copy", "description": "신분증 사본은 outputs/T-3 신분증/id.pdf", "kind": "person" });
    assert!(!tool(&handle, "pm", "remember", fact).await.is_error);

    // beta's member gets beta's memory — empty — and acme's home is denied it.
    let beta = SessionSpec { squad: Some("beta".into()), ..member(&site, "pm") };
    add(&handle, &mut events, 2, beta).await;
    handle.send(Command::Submit { session: 2, text: "hi".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    let call = calls(&recorded).pop().unwrap();
    let charter = arg(&call, "--append-system-prompt").unwrap();
    assert!(!charter.contains("id-copy") && charter.contains("remembers nothing yet"), "{charter}");
    let deny = settings_of(&call)["permissions"]["deny"].to_string();
    assert!(deny.contains(&format!("Read(/{}/**)", squads_home(&home).display())), "{deny}");
}

#[tokio::test]
async fn a_send_from_a_messaging_app_waits_for_the_persons_yes_every_time() {
    use equipe_core::desktop_gate::{App, Check, Element, Scene};
    let repo = repo();
    let home = squad_home(&[&repo]);
    let claude = fake_claude(true, "exec sleep 30");
    let (handle, mut events) = runtime::start(with_tools(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "backend")).await;
    // Even in full access.
    handle.send(Command::SetMode { squad: "acme".into(), member: None, mode: Some(Mode::Full) });
    until(&mut events, |e| matches!(e, Event::ModeChanged { .. })).await;

    let teams = Scene {
        app: App { name: "Microsoft Teams".into(), bundle_id: Some("com.microsoft.teams2".into()) },
        window: Some("Chat | Kim Minji".into()),
        target: None,
        focused: Some(Element { role: "AXTextArea".into(), value: Some("내일 회의 3시로 옮겨요".into()), ..Default::default() }),
    };
    let check = |tool: &str, input: serde_json::Value, scene: &Scene| Check {
        squad: "acme".into(),
        member: "backend".into(),
        tool: tool.into(),
        input,
        scene: scene.clone(),
        screen: Some(b"\x89PNG fake".to_vec()),
    };

    // Writing the draft goes ahead.
    let typed = handle.desktop_gate(check("mcp__desktop__type_text", serde_json::json!({ "text": "내일 회의 3시로 옮겨요" }), &teams)).await;
    assert!(!typed.is_error, "{}", typed.text);
    // Return without saying what it sends is turned back, unasked.
    let unsaid = handle.desktop_gate(check("mcp__desktop__key", serde_json::json!({ "keys": "return" }), &teams)).await;
    assert!(unsaid.is_error && unsaid.text.contains("`why`"), "{}", unsaid.text);

    let send = || {
        let handle = handle.clone();
        let check = check("mcp__desktop__key", serde_json::json!({ "keys": "return", "why": "Kim Minji에게 회의 시간 변경 답장" }), &teams);
        tokio::spawn(async move { handle.desktop_gate(check).await })
    };
    // "Always" is only ever once: the next send asks again.
    for decision in [Decision::Always, Decision::Allow] {
        let waiting = send();
        let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
        let Some(Event::ApprovalRequested { approval, .. }) = asked.last() else { unreachable!() };
        assert_eq!((approval.member.as_str(), approval.tool.as_str()), ("backend", "Send"));
        assert_eq!(approval.action, "press return in Microsoft Teams · Chat | Kim Minji\n내일 회의 3시로 옮겨요");
        assert_eq!(approval.reason.as_deref(), Some("Kim Minji에게 회의 시간 변경 답장"));
        let preview = approval.preview.clone().expect("the screen, to open");
        assert_eq!(std::fs::read(&preview).unwrap(), b"\x89PNG fake");
        assert!(equipe_core::permissions::rule_for(&approval.tool, &approval.action).is_none(), "never an \"always\"");
        handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision });
        let answer = waiting.await.unwrap();
        assert!(!answer.is_error, "{}", answer.text);
        assert!(!Path::new(&preview).exists(), "the screenshot is gone once answered");
    }
    assert!(Journal::for_squad(&home, "acme").unwrap().allowed().unwrap().is_empty(), "no rule kept");

    // A no: the input never runs, and the member hears why.
    let waiting = send();
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = asked.last() else { unreachable!() };
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Deny });
    let answer = waiting.await.unwrap();
    assert!(answer.is_error && answer.text.contains("Not sent"), "{}", answer.text);

    // TextEdit isn't a messaging app: Return goes ahead.
    let textedit = Scene { app: App { name: "TextEdit".into(), bundle_id: Some("com.apple.TextEdit".into()) }, ..Default::default() };
    assert!(!handle.desktop_gate(check("key", serde_json::json!({ "keys": "return" }), &textedit)).await.is_error);
    // A password field takes nothing, anywhere, in any mode.
    let login = Scene { focused: Some(Element { role: "AXSecureTextField".into(), ..Default::default() }), ..textedit.clone() };
    let refused = handle.desktop_gate(check("type_text", serde_json::json!({ "text": "hunter2" }), &login)).await;
    assert!(refused.is_error && refused.text.contains("password"), "{}", refused.text);

    // The person takes Teams off the list: it no longer asks.
    Journal::for_squad(&home, "acme").unwrap().set_messaging_apps(&["Slack".into()]).unwrap();
    assert!(!handle.desktop_gate(check("key", serde_json::json!({ "keys": "return" }), &teams)).await.is_error);
}

// ── work that isn't code ──

#[tokio::test]
async fn a_squad_of_only_a_pm_researches_tax_deadlines_into_its_folder_and_memory() {
    let file = "outputs/T-1 세금 마감/deadlines.md";
    let report = "# 올해 세금 마감\\n- 5월 31일 종합소득세 신고·납부 (출처: https://www.nts.go.kr/income-tax)\\n- 7월 25일 부가가치세 1기 확정 (확인되지 않음)";
    let (claude, recorded) = recording_claude(&format!(
        "case \"$*\" in *\"You're on T-\"*) sleep 1; mkdir -p \"$(dirname '{file}')\"; printf '{report}\\n' > '{file}';; esac\n{}",
        answering("정리했어요")
    ));
    let home = scratch("home");
    squads::create_without_project(&home, "acme").unwrap();
    let folder = squads_home(&home);
    let (handle, mut events) = runtime::start(with_tools(&claude, &home));
    add(&handle, &mut events, 1, member(&scratch("elsewhere"), "pm")).await;

    // The person asks the PM — the only member — for something that isn't code.
    handle.send(Command::Submit { session: 1, text: "올해 내가 챙겨야 할 세금 마감을 조사해서 정리해줘".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    let charter = arg(&calls(&recorded)[0], "--append-system-prompt").unwrap().to_owned();
    for part in ["Not every request needs a ticket", "do yourself, right in the conversation", "`task` for finding out", "own it yourself", "`/add <role>`", "marked unconfirmed"] {
        assert!(charter.contains(part), "{part}: {charter}");
    }
    assert!(!charter.contains("You don't write code or edit files") && !charter.contains("software squad"), "{charter}");

    // Research worth checking: the PM puts it on the board as a task for itself — no project needed.
    let made = tool(&handle, "pm", "create_ticket", serde_json::json!({
        "title": "세금 마감 조사", "body": "올해 챙길 세금 마감을 출처와 함께 정리", "owner": "pm",
    }))
    .await;
    assert_eq!(made.text, "Created T-1 (task).");
    until(&mut events, |e| matches!(e, Event::WorkStarted { session: 1, ticket: 1, .. })).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let brief = arg(&calls(&recorded).pop().unwrap(), "-p").unwrap().to_owned();
    assert!(brief.contains("give each fact its source") && brief.contains("unconfirmed"), "{brief}");

    // What recurs goes into the squad's memory; the work is handed in.
    let fact = serde_json::json!({ "name": "income-tax-deadline", "description": "종합소득세 신고·납부는 매년 5월 31일까지", "kind": "deadline" });
    let saved = tool(&handle, "pm", "remember", fact).await;
    assert!(!saved.is_error, "{}", saved.text);
    tool(&handle, "pm", "submit_for_review", serde_json::json!({ "summary": "마감 2건 정리. 부가세 1기는 확인되지 않음." })).await;
    let review =
        until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.status == TicketStatus::Review)).await;
    let note = ticket_changed(&review).pop().unwrap().note.unwrap();
    assert_eq!(
        note,
        format!("마감 2건 정리. 부가세 1기는 확인되지 않음.\n\nIn the squad's folder:\n- {file}\n\nSources its files cite:\n- https://www.nts.go.kr/income-tax")
    );

    // Accepted: the result, with its source, is in the squad's folder, and the deadline in its memory.
    handle.send(Command::Accept { squad: "acme".into(), ticket: 1 });
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.status == TicketStatus::Done)).await;
    let kept = std::fs::read_to_string(folder.join(file)).unwrap();
    assert!(kept.contains("https://www.nts.go.kr/income-tax") && kept.contains("확인되지 않음"), "{kept}");
    assert!(std::fs::read_to_string(folder.join("memory/INDEX.md")).unwrap().contains("income-tax-deadline"));
    handle.send(Command::Submit { session: 1, text: "다음 마감은?".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    let charter = arg(&calls(&recorded).pop().unwrap(), "--append-system-prompt").unwrap().to_owned();
    assert!(charter.contains("종합소득세 신고·납부는 매년 5월 31일까지"), "{charter}");
}

#[tokio::test]
async fn a_custom_role_edits_a_projects_files_only_when_the_person_says_so() {
    let (claude, recorded) = recording_claude(&answering("ok"));
    let site = repo();
    let home = squad_home(&[&site]);
    let (handle, mut events) = runtime::start(with_tools(&claude, &home));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    add(&handle, &mut events, 2, member(&site, "writer")).await;
    add(&handle, &mut events, 3, SessionSpec { access: Access::EditFiles, ..member(&site, "editor") }).await;
    let sessions = handle.snapshot("acme").await.sessions;
    let access = |name: &str| sessions.iter().find(|s| s.member.as_ref().is_some_and(|m| m.name == name)).unwrap().access;
    assert_eq!((access("writer"), access("editor")), (Access::ReadOnly, Access::EditFiles));

    // On a code ticket the one that edits gets a branch of its own; the other only reads.
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Docs", "kind": "code", "owner": "editor" })).await;
    let started = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    assert!(matches!(started.last(), Some(Event::WorkStarted { session: 3, workspace: Some(_), .. })), "{started:#?}");
    // One turn at a time, so the stand-in records each.
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 3, .. })).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Review docs", "kind": "code", "owner": "writer" })).await;
    let started = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 2, .. })).await;
    assert!(matches!(started.last(), Some(Event::WorkStarted { session: 2, workspace: None, .. })), "{started:#?}");
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    let charters: Vec<String> = calls(&recorded).iter().filter_map(|c| arg(c, "--append-system-prompt").map(str::to_owned)).collect();
    let of = |name: &str| {
        let who = format!("You are {name},");
        charters.iter().find(|c| c.starts_with(&who)).unwrap_or_else(|| panic!("{who} {charters:#?}")).clone()
    };
    assert!(of("writer").contains("you don't change their files"), "{}", of("writer"));
    assert!(of("editor").contains("You may change a project's files on a code ticket"), "{}", of("editor"));
}

/// A turn that answers `text` with a conversation id of its own call's number,
/// so a test can see which conversation the next turn continues.
fn answering_per_call(text: &str) -> String {
    answering(text).replace("\"session_id\":\"sess-1\"", "\"session_id\":\"sess-$n\"")
}

#[tokio::test]
async fn a_member_says_when_its_work_is_done_and_when_what_it_ships_is_live() {
    let repo = repo();
    let (claude, _) = recording_claude(&format!("sleep 1\n{}", answering("ok")));
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&repo])));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    handle.send(Command::Submit { session: 1, text: "ship it".into() });
    until(&mut events, |e| matches!(e, Event::UserMessage { .. })).await;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let said = tool(&handle, "pm", "estimate", serde_json::json!({
        "done_in_minutes": 10, "what": "the release", "live_in_minutes": 20, "live_what": "v1 on Homebrew",
    }))
    .await;
    assert!(!said.is_error, "{said:?}");
    let seen = until(&mut events, |e| matches!(e, Event::Estimated { .. })).await;
    let Some(Event::Estimated { session: 1, estimate: Some(e) }) = seen.last() else { panic!("{seen:#?}") };
    assert!(e.done_at.unwrap().abs_diff(now + 600) < 5 && e.live_at.unwrap().abs_diff(now + 1200) < 5, "{e:?}");
    assert_eq!((e.what.as_deref(), e.live_what.as_deref()), (Some("the release"), Some("v1 on Homebrew")));
    // Screens attaching later see it.
    let snapshot = handle.snapshot("acme").await;
    assert!(snapshot.sessions[0].estimate.as_ref().is_some_and(|s| s.done_at.is_some()));
    // The turn over, the work is done; what ships is still on its way.
    let seen = until(&mut events, |e| matches!(e, Event::Estimated { .. })).await;
    let Some(Event::Estimated { estimate: Some(e), .. }) = seen.last() else { panic!("{seen:#?}") };
    assert!(e.done_at.is_none() && e.live_at.is_some(), "{e:?}");
    // Nothing to say: the member is told how.
    let none = tool(&handle, "pm", "estimate", serde_json::json!({})).await;
    assert!(none.is_error);
}

#[tokio::test]
async fn without_a_members_word_equipe_guesses_from_how_long_such_turns_took() {
    let repo = repo();
    let home = squad_home(&[&repo]);
    // The PM's desk turns took 5, 10 and 20 minutes before.
    let journal = equipe_core::journal::Journal::for_squad(&home, "acme").unwrap();
    for minutes in [5, 10, 20] {
        journal.record_turn_time("pm", minutes * 60).unwrap();
    }
    drop(journal);
    let (claude, _) = recording_claude(&format!("sleep 1\n{}", answering("ok")));
    let (handle, mut events) = runtime::start(with_tools(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    handle.send(Command::Submit { session: 1, text: "look into it".into() });
    let seen = until(&mut events, |e| matches!(e, Event::Estimated { .. })).await;
    let Some(Event::Estimated { estimate: Some(e), .. }) = seen.last() else { panic!("{seen:#?}") };
    assert!(e.guessed && e.done_at.unwrap().abs_diff(now + 600) < 5, "the median, 10 minutes: {e:?}");
    // The member's own word replaces the guess.
    tool(&handle, "pm", "estimate", serde_json::json!({ "done_in_minutes": 2 })).await;
    let seen = until(&mut events, |e| matches!(e, Event::Estimated { .. })).await;
    let Some(Event::Estimated { estimate: Some(e), .. }) = seen.last() else { panic!("{seen:#?}") };
    assert!(!e.guessed && e.done_at.unwrap().abs_diff(now + 120) < 5, "{e:?}");
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    // A turn under a minute isn't kept to guess from.
    let journal = equipe_core::journal::Journal::for_squad(&home, "acme").unwrap();
    assert_eq!(journal.turn_times("pm").unwrap().len(), 3);
}

#[tokio::test]
async fn a_member_keeps_one_conversation_whatever_project_is_in_view() {
    let site = named("site");
    let app = named("app");
    let (claude, recorded) = recording_claude(&answering_per_call("ok"));
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&site, &app])));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    let mut seen = Vec::new();
    for (project, text) in [(Some("app"), "app one"), (Some("site"), "site one"), (None, "squad one")] {
        handle.send(Command::Focus { squad: "acme".into(), project: project.map(str::to_owned) });
        handle.send(Command::Submit { session: 1, text: text.into() });
        seen.extend(until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await);
    }
    let all = calls(&recorded);
    let resumed: Vec<Option<&str>> = all.iter().map(|c| arg(c, "--resume")).collect();
    assert_eq!(resumed, [None, Some("sess-0"), Some("sess-1")], "one conversation, carried on");
    assert!(arg(&all[0], "-p").unwrap().contains("looking at the project app"));
    assert!(arg(&all[1], "-p").unwrap().contains("looking at the project site"));
    assert!(arg(&all[2], "-p").unwrap().contains("looking at the whole squad"));
    assert!(!seen.iter().any(|e| matches!(e, Event::LaneOpened { .. })), "{seen:#?}");
    // One conversation; each message marked with the project it was about,
    // for the screens to show one project's slice.
    let marked: Vec<Option<&str>> = seen.iter().filter_map(|e| if let Event::Thread { session: 1, project } = e { Some(project.as_deref()) } else { None }).collect();
    assert_eq!(marked, [Some("app"), Some("site"), None], "{seen:#?}");
}

#[tokio::test]
async fn each_message_its_reply_and_its_steps_are_about_the_project_in_focus_when_sent() {
    use equipe_core::protocol::transcript::scope;
    let site = named("site");
    let app = named("app");
    let (claude, _recorded) = recording_claude(&answering_per_call("ok"));
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&site, &app])));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    let mut seen = Vec::new();
    for (project, text) in [(Some("app"), "app one"), (Some("app"), "app two"), (Some("site"), "site one"), (None, "squad one")] {
        handle.send(Command::Focus { squad: "acme".into(), project: project.map(str::to_owned) });
        handle.send(Command::Submit { session: 1, text: text.into() });
        seen.extend(until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await);
    }
    // A marker only where the project changes.
    assert_eq!(seen.iter().filter(|e| matches!(e, Event::Thread { .. })).count(), 3, "{seen:#?}");
    // A screen attaching reads what each line is about from the conversation.
    let recent = handle.snapshot("acme").await.sessions.into_iter().find(|s| s.session == 1).unwrap().recent;
    let about: Vec<(String, Option<String>)> = recent
        .iter()
        .zip(scope::of(&recent))
        .filter_map(|(e, about)| match e {
            Entry::User { text } => Some((format!("user: {text}"), about)),
            Entry::Reply { text } => Some((format!("reply: {text}"), about)),
            _ => None,
        })
        .collect();
    let app = || Some("app".to_owned());
    assert_eq!(
        about,
        [
            ("user: app one".into(), app()),
            ("reply: ok".into(), app()),
            ("user: app two".into(), app()),
            ("reply: ok".into(), app()),
            ("user: site one".into(), Some("site".into())),
            ("reply: ok".into(), Some("site".into())),
            ("user: squad one".into(), None),
            ("reply: ok".into(), None),
        ]
    );
    // The PM is told as before: the whole conversation, the prefix in its prompt only.
    assert!(!recent.iter().any(|e| matches!(e, Entry::User { text } if text.starts_with('['))));
    assert!(scope::LOOKING_AT.starts_with("[equipe: the person is looking at the project") && scope::WHOLE_SQUAD.starts_with("[equipe: the person is looking at the whole squad"));
}

#[tokio::test]
async fn news_in_the_pms_conversation_is_about_its_tickets_project() {
    use equipe_core::protocol::transcript::scope;
    let site = named("site");
    let app = named("app");
    let (claude, _recorded) = recording_claude(&answering_per_call("ok"));
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&site, &app])));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    handle.send(Command::Focus { squad: "acme".into(), project: Some("site".into()) });
    handle.send(Command::Submit { session: 1, text: "hi".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    // A teammate proposes work for the app: the news is about the app.
    add(&handle, &mut events, 2, member(&site, "designer")).await;
    let made = tool(&handle, "designer", "create_ticket", serde_json::json!({ "title": "App icon", "body": "x", "project": "app" })).await;
    assert!(!made.is_error, "{made:?}");
    let seen = until(&mut events, |e| matches!(e, Event::Notice { session: 1, .. })).await;
    let marks: Vec<Option<&str>> = seen.iter().filter_map(|e| if let Event::Thread { session: 1, project } = e { Some(project.as_deref()) } else { None }).collect();
    assert_eq!(marks, [Some("app")], "the PM rests: no need to mark the way back — {seen:#?}");
    let recent = handle.snapshot("acme").await.sessions.into_iter().find(|s| s.session == 1).unwrap().recent;
    let scopes = scope::of(&recent);
    let at = recent.iter().rposition(|e| matches!(e, Entry::Note { .. })).unwrap();
    assert_eq!(scopes[at].as_deref(), Some("app"), "{recent:#?}");
    let hi = recent.iter().position(|e| matches!(e, Entry::User { text } if text == "hi")).unwrap();
    assert_eq!(scopes[hi].as_deref(), Some("site"));
}

#[tokio::test]
async fn news_of_another_project_mid_turn_leaves_the_reply_about_its_own() {
    use equipe_core::protocol::transcript::scope;
    let site = named("site");
    let app = named("app");
    let (claude, _recorded) = recording_claude(&format!("sleep 1\n{}", answering_per_call("ok")));
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&site, &app])));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    add(&handle, &mut events, 2, member(&site, "designer")).await;
    handle.send(Command::Focus { squad: "acme".into(), project: Some("site".into()) });
    handle.send(Command::Submit { session: 1, text: "hi".into() });
    until(&mut events, |e| matches!(e, Event::UserMessage { session: 1, .. })).await;
    let made = tool(&handle, "designer", "create_ticket", serde_json::json!({ "title": "App icon", "body": "x", "project": "app" })).await;
    assert!(!made.is_error, "{made:?}");
    let seen = until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    let marks: Vec<Option<&str>> = seen.iter().filter_map(|e| if let Event::Thread { session: 1, project } = e { Some(project.as_deref()) } else { None }).collect();
    assert_eq!(marks, [Some("app"), Some("site")], "{seen:#?}");
    let recent = handle.snapshot("acme").await.sessions.into_iter().find(|s| s.session == 1).unwrap().recent;
    let scopes = scope::of(&recent);
    let reply = recent.iter().rposition(|e| matches!(e, Entry::Reply { .. })).unwrap();
    assert_eq!(scopes[reply].as_deref(), Some("site"), "{recent:#?}");
}

#[tokio::test]
async fn a_memo_carries_the_project_it_was_left_about() {
    let site = named("site");
    let app = named("app");
    let (claude, recorded) = recording_claude(&format!("sleep 1\n{}", answering_per_call("ok")));
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&site, &app])));
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    handle.send(Command::Focus { squad: "acme".into(), project: Some("site".into()) });
    handle.send(Command::Submit { session: 1, text: "site one".into() });
    until(&mut events, |e| matches!(e, Event::UserMessage { .. })).await;
    handle.send(Command::SubmitLater { session: 1, text: "about the site".into() });
    until(&mut events, |e| matches!(e, Event::MessageQueued { .. })).await;
    // Looking elsewhere by the time it goes in: it is still about site.
    handle.send(Command::Focus { squad: "acme".into(), project: Some("app".into()) });
    until(&mut events, |e| matches!(e, Event::UserMessage { text, .. } if text == "about the site")).await;
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    let all = calls(&recorded);
    let memo = all.iter().find(|c| arg(c, "-p").is_some_and(|p| p.contains("about the site"))).expect("the memo's turn");
    assert!(arg(memo, "-p").unwrap().contains("looking at the project site"), "{memo:?}");
    assert_eq!(arg(memo, "--resume"), Some("sess-0"));
}

#[tokio::test]
async fn the_pm_on_a_ticket_of_its_own_is_still_reached_beside_it() {
    let site = named("site");
    let app = named("app");
    // The ticket's work is slow; the overview names the ticket, not its body.
    let turn = format!("case \"$*\" in *\"slow-work\"*) sleep 8;; esac\n{}", answering_per_call("ok"));
    let (claude, recorded) = recording_claude(&turn);
    let home = squad_home(&[&site, &app]);
    let (npx, _) = fake_npx();
    let (handle, mut events) = runtime::start(Config { npx: Some(npx), ..with_tools(&claude, &home) });
    add(&handle, &mut events, 1, member(&site, "pm")).await;
    add(&handle, &mut events, 2, member(&site, "designer")).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Translate the site", "body": "slow-work", "owner": "pm", "project": "site" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { session: 1, .. })).await;
    tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Login page", "body": "-", "owner": "qa", "project": "app" })).await;

    // The PM's own ticket runs; what the person says opens its conversation beside it.
    handle.send(Command::Focus { squad: "acme".into(), project: None });
    handle.send(Command::Submit { session: 1, text: "how is everything?".into() });
    let seen = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    let lane = seen
        .iter()
        .find_map(|e| match e {
            Event::LaneOpened { session, of: 1, project: None } => Some(*session),
            _ => None,
        })
        .expect("the PM's conversation, beside its ticket");
    // Answered while the ticket's work goes on.
    assert!(matches!(seen.last(), Some(Event::TurnEnded { session, .. }) if *session == lane), "{seen:#?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::SteerQueued { .. } | Event::MessageQueued { .. })), "not into the ticket's work");
    // About the whole squad, it is told where every project stands.
    let all = calls(&recorded);
    let call = all.iter().find(|c| arg(c, "-p").is_some_and(|p| p.contains("how is everything?"))).unwrap();
    let prompt = arg(call, "-p").unwrap();
    assert!(prompt.contains("- site: pm on T-1 (Translate the site), at work"), "{prompt}");
    assert!(prompt.contains("- app: no one on it now; 1 to do or in progress"), "{prompt}");
    // Its browser is its own, not the ticket's.
    let ticket = all.iter().find(|c| arg(c, "-p").is_some_and(|p| p.contains("slow-work"))).unwrap();
    assert_ne!(browser_of(call)["url"], browser_of(ticket)["url"]);
    // Team news shows where the PM talks now.
    tool(&handle, "designer", "create_ticket", serde_json::json!({ "title": "Hero", "body": "-", "owner": "designer", "project": "site" })).await;
    let news = until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("proposed"))).await;
    assert!(matches!(news.last(), Some(Event::Notice { session, .. }) if *session == lane), "{news:#?}");
}

#[tokio::test]
async fn a_members_model_changes_between_turns_and_its_conversation_carries_on() {
    let cwd = scratch("model");
    let (claude, recorded) = recording_claude(&answering("ok"));
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&cwd])));
    add(&handle, &mut events, 1, member(&cwd, "frontend")).await;
    handle.send(Command::Submit { session: 1, text: "one".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    handle.send(Command::SetModel { session: 1, model: "opus".into() });
    until(&mut events, |e| matches!(e, Event::ModelKnown { session: 1, model } if model == "opus")).await;
    handle.send(Command::Submit { session: 1, text: "two".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    let all = calls(&recorded);
    assert_eq!(arg(&all[0], "--model"), Some("sonnet"));
    assert_eq!(arg(&all[1], "--model"), Some("opus"));
    assert_eq!(arg(&all[1], "--resume"), Some("sess-1"));
}

#[tokio::test]
async fn a_folder_inside_a_repository_is_a_project_of_its_own_worked_in_its_folder() {
    let repo = repo();
    let desktop = repo.join("crates").join("desktop");
    std::fs::create_dir_all(&desktop).unwrap();
    std::fs::write(desktop.join("main.rs"), "fn main() {}\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "desktop"]);
    // Each turn writes down where it ran.
    let wd = scratch("wd");
    let turn = format!("pwd > '{}/cwd'\n{}", wd.display(), answering("ok"));
    let claude = fake_claude(true, &turn);
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&repo, &desktop])));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    let name = format!("{}-desktop", repo.file_name().unwrap().to_string_lossy());
    assert_eq!(equipe_core::board::project_name(&desktop), name);
    let made = tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "Window", "body": "-", "owner": "frontend", "project": name })).await;
    assert!(!made.is_error, "{made:?}");
    let started = until(&mut events, |e| matches!(e, Event::WorkStarted { .. })).await;
    let Some(Event::WorkStarted { workspace: Some(ws), project, .. }) = started.last() else { panic!("{started:#?}") };
    assert_eq!(project, &desktop);
    // Its conversation marks what follows as that project's.
    assert!(started.iter().any(|e| matches!(e, Event::Thread { session: 2, project: Some(p) } if *p == name)), "{started:#?}");
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    // The worktree is the whole repository's; the member works in the folder.
    let ran = std::fs::read_to_string(wd.join("cwd")).unwrap();
    let ran = std::fs::canonicalize(ran.trim()).unwrap();
    assert_eq!(ran, std::fs::canonicalize(ws.path.join("crates").join("desktop")).unwrap());
}

/// A Mac nobody has touched for ten minutes, unlocked.
struct QuietMac;

impl equipe_core::desktop::Probe for QuietMac {
    fn idle(&self) -> Option<f64> {
        Some(600.0)
    }
    fn screen(&self) -> equipe_core::desktop::Screen {
        equipe_core::desktop::Screen::Open
    }
}

#[tokio::test]
async fn two_members_at_the_screen_take_turns_and_desktop_stop_refuses_both() {
    use equipe_core::desktop::Use;
    use equipe_core::protocol::{DesktopState, MemberRef};

    let repo = repo();
    let claude = fake_claude(true, &answering("ok"));
    let home = squad_home(&[&repo]);
    let (handle, mut events) = runtime::start(Config { mac: Box::new(QuietMac), ..config(&claude, &home, false) });
    add(&handle, &mut events, 1, member(&repo, "frontend")).await;
    add(&handle, &mut events, 2, member(&repo, "backend")).await;
    let who = |name: &str| MemberRef { squad: "acme".into(), member: name.into() };
    // Every screen is told who has it now.
    let shows = async |events: &mut UnboundedReceiver<Event>, want: DesktopState| {
        until(events, |e| matches!(e, Event::Desktop { desktop } if *desktop == want)).await;
    };

    // Both at once: one has it, the other waits in line.
    let (a, b) = tokio::join!(handle.desktop_ask(who("frontend"), Use::Input), handle.desktop_ask(who("backend"), Use::Input));
    let (first, second, start) = match (a, b) {
        (Ok(start), Err(no)) => (who("frontend"), who("backend"), (start, no)),
        (Err(no), Ok(start)) => (who("backend"), who("frontend"), (start, no)),
        other => panic!("exactly one may use the screen: {other:?}"),
    };
    let (start, refusal) = start;
    assert!(refusal.starts_with(&format!("{} is using the screen; you are number 1 in line", first.member)), "{refusal}");
    shows(&mut events, DesktopState { holder: Some(first.clone()), queue: vec![second.clone()], stopped: false }).await;
    handle.desktop_done(first.clone(), start, true);
    assert!(handle.desktop_ask(second.clone(), Use::Look).await.is_err(), "looking waits its turn too");

    // The first one's turn ends: the one waiting carries on.
    let session = if first.member == "frontend" { 1 } else { 2 };
    handle.send(Command::Submit { session, text: "go".into() });
    shows(&mut events, DesktopState { holder: Some(second.clone()), queue: vec![], stopped: false }).await;
    assert!(handle.desktop_ask(second.clone(), Use::Input).await.is_ok());
    assert!(handle.desktop_ask(first.clone(), Use::Input).await.is_err());

    // The person takes it back: nobody's click or keystroke goes in, nobody waits.
    handle.send(Command::Desktop { open: false });
    shows(&mut events, DesktopState { holder: None, queue: vec![], stopped: true }).await;
    for member in [&first, &second] {
        let refusal = handle.desktop_ask(member.clone(), Use::Input).await.unwrap_err();
        assert!(refusal.contains("/desktop stop"), "{refusal}");
    }
    assert_eq!(handle.busy().await.desktop, DesktopState { holder: None, queue: vec![], stopped: true });
    assert!(handle.snapshot("acme").await.desktop.stopped);

    // Opened again; a member lets go of it itself.
    handle.send(Command::Desktop { open: true });
    shows(&mut events, DesktopState::default()).await;
    assert!(handle.desktop_ask(first.clone(), Use::Input).await.is_ok());
    assert!(handle.desktop_release(first.clone()).await);
    assert!(handle.desktop_ask(second, Use::Input).await.is_ok());
}

#[tokio::test]
async fn one_browser_at_a_time_comes_on_screen_and_a_member_can_call_for_it() {
    let repo = repo();
    let (claude, _) = recording_claude(&answering("ok"));
    let home = squad_home(&[&repo]);
    let (npx, launched) = fake_npx();
    let (handle, mut events) = runtime::start(Config { npx: Some(npx), ..with_tools(&claude, &home) });
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    let shown = |name: &str| Journal::for_squad(&home, "acme").unwrap().browser_shown(name);
    // Between turns, showing one opens it on screen at once…
    handle.send(Command::ShowBrowser { squad: "acme".into(), member: Some("pm".into()), show: true });
    for _ in 0..200 {
        if launches(&launched).iter().any(|a| !a.iter().any(|x| x == "--headless")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(launches(&launched).iter().any(|a| !a.iter().any(|x| x == "--headless")), "pm's window opened at once");
    // …and showing another puts the first away.
    handle.send(Command::ShowBrowser { squad: "acme".into(), member: Some("frontend".into()), show: true });
    for _ in 0..200 {
        if shown("frontend") && !shown("pm") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(shown("frontend") && !shown("pm"));
    // A member calls its own browser up for the person.
    let said = tool(&handle, "pm", "show_my_browser", serde_json::json!({ "url": "https://vercel.com/login", "why": "Sign in to Vercel" })).await;
    assert!(!said.is_error && said.text.contains("comes up on the person's screen"), "{said:?}");
    assert!(shown("pm") && !shown("frontend"));
}

#[tokio::test]
async fn after_a_member_answers_the_person_their_next_message_is_guessed() {
    let cwd = scratch("cwd");
    let home = squad_home(&[&cwd]);
    let turn = format!(
        "case \"$*\" in\n*no-session-persistence*) echo '{{\"type\":\"result\",\"is_error\":false,\"result\":\"yes, tidy them\"}}' ;;\n*)\n{}\n;;\nesac",
        answering("Done. Shall I tidy the other pages too?")
    );
    let (claude, recorded) = recording_claude(&turn);
    let (handle, mut events) = runtime::start(Config { suggest: true, ..config(&claude, &home, false) });
    add(&handle, &mut events, 1, member(&cwd, "frontend")).await;

    handle.send(Command::Submit { session: 1, text: "fix the button".into() });
    let seen = until(&mut events, |e| matches!(e, Event::Suggestion { .. })).await;
    assert!(
        seen.iter().any(|e| matches!(e, Event::Suggestion { session: 1, text } if text == "yes, tidy them")),
        "{seen:?}"
    );
    // The guess reads the end of the conversation, not the member's own one.
    let guess = &calls(&recorded)[1];
    assert_eq!(arg(guess, "--model"), Some("haiku"));
    assert_eq!(arg(guess, "--resume"), None);
    let prompt = arg(guess, "-p").unwrap();
    assert!(prompt.contains("[person] fix the button"), "{prompt}");
    assert!(prompt.contains("[frontend] Done. Shall I tidy the other pages too?"), "{prompt}");
    // And it isn't part of the conversation.
    let snapshot = handle.snapshot("acme").await;
    assert!(!snapshot.suggestions_off);

    // Turned off, none is guessed.
    handle.send(Command::Suggestions { squad: "acme".into(), on: false });
    handle.send(Command::Submit { session: 1, text: "and the footer".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(calls(&recorded).len(), 3, "the turn, its guess, the next turn");
    assert!(handle.snapshot("acme").await.suggestions_off);
}
