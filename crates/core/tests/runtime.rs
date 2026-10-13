//! The runtime, driven the way a screen drives it: commands in, events out.
//! A stand-in `claude` plays the engine, so no network or login is needed.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::journal::{self, Journal};
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{
    Access, Command, Engine, Entry, Event, SessionSpec, SessionState, Ticket, TicketStatus, TurnOutcome,
};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
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
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":{logged_in},\"authMethod\":\"claude.ai\"}}'; exit 0; fi\n{turn}\n"
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
    git(&dir, &["init", "-q"]);
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
    let (handle, mut events) = runtime::start(config(&claude, &squad_home(&[&cwd]), false));
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
    // At its desk it reads on its own; anything more — a fetch, an edit — asks
    // the person rather than being refused unseen.
    assert_eq!(arg(call, "--permission-mode"), Some("manual"));
    assert_eq!(arg(call, "--permission-prompt-tool"), Some("mcp__equipe__approve"));
    assert_eq!(arg(call, "--add-dir"), Some(&*app.to_string_lossy()));
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
    let nowhere = tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "x", "body": "-" })).await;
    assert!(nowhere.is_error && nowhere.text.contains("several: site, app"), "{nowhere:?}");
    let unknown = tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "x", "project": "web" })).await;
    assert!(unknown.is_error && unknown.text.contains("No project called web"), "{unknown:?}");
    let nothing = tool(&handle, "pm", "submit_for_review", serde_json::json!({ "summary": "?" })).await;
    assert!(nothing.is_error && nothing.text.contains("not on a ticket"), "{nothing:?}");
    let unstaffed =
        tool(&handle, "pm", "create_ticket", serde_json::json!({ "title": "x", "project": "app", "owner": "designer" })).await;
    assert!(!unstaffed.is_error && unstaffed.text.contains("/add designer"), "{unstaffed:?}");
    let board = tool(&handle, "pm", "board", serde_json::json!({})).await;
    assert_eq!(board.text, "T-1 [todo] x — owner designer, project app\n");
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
        handle.send(Command::Submit { session: 2, text: "and the footer".into() });
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
for a in "$@"; do printf '%s\n' "$a"; done > '{launches}'/launch-$(printf %03d $n)
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
    // Back to all: nothing added.
    handle.send(Command::Focus { squad: "acme".into(), project: None });
    handle.send(Command::Submit { session: 1, text: "and the site?".into() });
    until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert_eq!(arg(calls(&recorded).last().unwrap(), "-p"), Some("and the site?"));
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
    let (handle, mut events) = runtime::start(with_tools(&claude, &squad_home(&[&repo])));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    let call = first_ticket_turn(&handle, &mut events, &recorded, "frontend").await;

    assert_eq!(arg(&call, "--permission-mode"), Some("acceptEdits"));
    let settings: serde_json::Value = serde_json::from_str(arg(&call, "--settings").unwrap()).unwrap();
    assert_eq!(settings["sandbox"]["enabled"], true, "the operating system fences its commands in");
    assert_eq!(settings["sandbox"]["autoAllowBashIfSandboxed"], true, "inside the fence, no questions");
    assert!(settings["sandbox"]["network"]["allowedDomains"].as_array().unwrap().iter().any(|d| d == "registry.npmjs.org"));
    assert!(settings["permissions"]["ask"].as_array().unwrap().iter().any(|r| r == "Bash(git push:*)"), "pushing asks");
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
