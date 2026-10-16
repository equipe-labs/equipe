//! The daemon over its socket: screens attach, detach and come back; the work
//! and its record outlive both the screens and the daemon process.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::gateway::{self, Client};
use equipe_core::protocol::rpc::ToolCall;
use equipe_core::protocol::{Access, Command, Entry, Event, SessionSpec, SessionState, TicketStatus};
use equipe_core::runtime::Config;
use equipe_core::squads;
use equipe_providers::CredentialStore;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A fresh directory — a counter keeps parallel tests apart — under /tmp,
/// because Unix socket paths are limited to ~100 bytes.
fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = PathBuf::from("/tmp").join(format!("eqg-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fake_claude(turn: &str) -> PathBuf {
    let bin = scratch("bin").join("claude");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\n{turn}\n"
    );
    write_script(&bin, &script);
    bin
}

/// Answers `text` after `delay` seconds, continuing sess-1 when asked to.
fn slow_answer(delay: &str) -> String {
    format!(
        r#"sleep {delay}
case "$*" in *"--resume sess-1"*) T=resumed;; *) T=fresh;; esac
cat <<EOF
{{"type":"system","subtype":"init","model":"m","session_id":"sess-1"}}
{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"$T"}}}}}}
{{"type":"result","subtype":"success","is_error":false,"result":"$T","usage":{{"input_tokens":1,"output_tokens":1}}}}
EOF"#
    )
}

fn config(home: &Path, claude: &Path) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        persist: true,
        retention_days: 90,
        equipe_binary: None,
        npx: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        lang: equipe_core::i18n::Lang::En,
    }
}

async fn daemon(home: &Path, claude: &Path) -> (PathBuf, tokio::task::JoinHandle<std::io::Result<()>>) {
    daemon_with(config(home, claude)).await
}

async fn daemon_with(config: Config) -> (PathBuf, tokio::task::JoinHandle<std::io::Result<()>>) {
    let socket = gateway::socket_path(&config.home);
    let task = tokio::spawn({
        let socket = socket.clone();
        async move { gateway::serve(config, &socket, "test").await }
    });
    for _ in 0..100 {
        if gateway::is_running(&socket).await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (socket, task)
}

async fn until(client: &mut Client, done: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    let wait = async {
        while let Some(event) = client.next_event().await {
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

/// A designer joining the squad `acme` from `cwd`.
fn spec(cwd: &Path) -> SessionSpec {
    SessionSpec {
        engine: None,
        model: None,
        access: Access::ReadOnly,
        cwd: cwd.to_owned(),
        role: Some("designer".into()),
        squad: Some("acme".into()),
    }
}

/// A home with the squad `acme` working on a fresh project.
fn squad() -> (PathBuf, PathBuf) {
    let home = scratch("home");
    let project = scratch("proj");
    squads::create(&home, "acme", &project.canonicalize().unwrap()).unwrap();
    (home, project)
}

#[tokio::test]
async fn the_squad_outlives_its_screens_and_its_daemon() {
    let (home, project) = squad();
    let claude = fake_claude(&slow_answer("1"));
    let (socket, daemon_task) = daemon(&home, &claude).await;

    // Screen A opens a session and starts a slow turn, then leaves.
    let (mut a, hello) = Client::connect(&socket, "test", "acme").await.unwrap();
    assert!(hello.snapshot.sessions.is_empty());
    a.send(Command::Start { session: 11, spec: spec(&project) });
    until(&mut a, |e| matches!(e, Event::SessionStarted { .. })).await;
    a.send(Command::Submit { session: 11, text: "keep going without me".into(), about: None });
    until(&mut a, |e| matches!(e, Event::UserMessage { .. })).await;
    drop(a);

    // Screen B attaches mid-turn: it sees the work in progress, then its end.
    let (mut b, hello) = Client::connect(&socket, "test", "acme").await.unwrap();
    let session = &hello.snapshot.sessions[0];
    assert_eq!(session.session, 11);
    assert_eq!(session.state, SessionState::Working);
    assert_eq!(session.recent, vec![Entry::User { text: "keep going without me".into() }]);
    let rest = until(&mut b, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(rest.iter().any(|e| matches!(e, Event::TextDelta { text, .. } if text == "fresh")));
    drop(b);

    // The daemon stops; a new one resumes the session and its conversation.
    assert!(gateway::shutdown(&socket).await.unwrap());
    daemon_task.await.unwrap().unwrap();
    assert!(!socket.exists(), "the socket is cleaned up");
    let (socket, _daemon) = daemon(&home, &claude).await;

    let (mut c, hello) = Client::connect(&socket, "test", "acme").await.unwrap();
    let session = &hello.snapshot.sessions[0];
    assert_eq!(session.state, SessionState::Ready);
    assert_eq!(
        session.recent,
        vec![Entry::User { text: "keep going without me".into() }, Entry::Reply { text: "fresh".into() }]
    );
    c.send(Command::Submit { session: 11, text: "and now?".into(), about: None });
    let turn = until(&mut c, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(turn.iter().any(|e| matches!(e, Event::TextDelta { text, .. } if text == "resumed")));
    gateway::shutdown(&socket).await.unwrap();
}

#[tokio::test]
async fn two_screens_watch_the_same_team() {
    let (home, project) = squad();
    let claude = fake_claude(&slow_answer("0"));
    let (socket, _daemon) = daemon(&home, &claude).await;

    let (mut a, _) = Client::connect(&socket, "test", "acme").await.unwrap();
    let (mut b, _) = Client::connect(&socket, "test", "acme").await.unwrap();
    a.send(Command::Start { session: 21, spec: spec(&project) });
    until(&mut a, |e| matches!(e, Event::SessionStarted { .. })).await;
    // B sees a session it didn't open, and the message A typed.
    until(&mut b, |e| matches!(e, Event::SessionStarted { session: 21, .. })).await;
    a.send(Command::Submit { session: 21, text: "from A".into(), about: None });
    let seen_by_b = until(&mut b, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(seen_by_b.iter().any(|e| matches!(e, Event::UserMessage { text, .. } if text == "from A")));
    gateway::shutdown(&socket).await.unwrap();
}

#[tokio::test]
async fn a_screen_says_its_language_when_it_attaches() {
    let (home, _project) = squad();
    let claude = fake_claude(&slow_answer("0"));
    let (socket, _daemon) = daemon(&home, &claude).await;
    let (mut screen, _) = Client::connect_in(&socket, "test", "acme", Some(equipe_core::i18n::Lang::De)).await.unwrap();
    screen.send(Command::Accept { squad: "acme".into(), ticket: 99 });
    let seen = until(&mut screen, |e| matches!(e, Event::Error { .. })).await;
    assert!(matches!(seen.last(), Some(Event::Error { message, .. }) if message == "Auf dem Board gibt es kein T-99."), "{seen:#?}");
    gateway::shutdown(&socket).await.unwrap();
}

#[tokio::test]
async fn a_screen_sees_only_its_own_squad() {
    let (home, project) = squad();
    let other = scratch("other");
    squads::create(&home, "beta", &other.canonicalize().unwrap()).unwrap();
    let claude = fake_claude(&slow_answer("0"));
    let (socket, _daemon) = daemon(&home, &claude).await;
    let (mut acme, _) = Client::connect(&socket, "test", "acme").await.unwrap();
    let (mut beta, _) = Client::connect(&socket, "test", "beta").await.unwrap();
    beta.send(Command::Start { session: 31, spec: SessionSpec { squad: Some("beta".into()), ..spec(&other) } });
    until(&mut beta, |e| matches!(e, Event::SessionStarted { .. })).await;
    acme.send(Command::Start { session: 32, spec: spec(&project) });
    let seen = until(&mut acme, |e| matches!(e, Event::SessionStarted { .. })).await;
    assert!(matches!(seen.as_slice(), [Event::SessionStarted { session: 32, .. }]), "{seen:#?}");
    gateway::shutdown(&socket).await.unwrap();
}

#[tokio::test]
async fn a_members_tool_call_reaches_the_board_over_the_socket() {
    let (home, project) = squad();
    let claude = fake_claude(&slow_answer("0"));
    let (socket, _daemon) = daemon(&home, &claude).await;
    let (mut screen, _) = Client::connect(&socket, "test", "acme").await.unwrap();
    screen.send(Command::Start { session: 41, spec: spec(&project) });
    until(&mut screen, |e| matches!(e, Event::SessionStarted { .. })).await;

    let call = ToolCall {
        squad: "acme".into(),
        member: "designer".into(),
        name: "create_ticket".into(),
        arguments: serde_json::json!({ "title": "A logo", "body": "Something simple." }),
    };
    let result = gateway::call_tool(&socket, &call).await.unwrap();
    assert!(!result.is_error && result.text.starts_with("Proposed T-1"), "{result:?}");
    let seen = until(&mut screen, |e| matches!(e, Event::TicketChanged { .. })).await;
    assert!(matches!(seen.last(), Some(Event::TicketChanged { ticket, .. }) if ticket.status == TicketStatus::Proposed));

    let stranger = ToolCall { member: "nobody".into(), ..call };
    assert!(gateway::call_tool(&socket, &stranger).await.unwrap().is_error);
    gateway::shutdown(&socket).await.unwrap();
}

/// A screen that says it has a browser panel — the app, here or on another
/// Mac — gets a member's browser handed to it rather than a window on the
/// daemon's screen, for as long as it stays attached.
#[tokio::test]
async fn while_the_app_is_attached_a_handed_over_browser_goes_to_its_panel_not_this_macs_screen() {
    use equipe_core::protocol::rpc::{self, HelloParams, Message};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
    let (home, project) = squad();
    let claude = fake_claude(&slow_answer("0"));
    // The browser host: a Chrome that never says anything.
    let npx = scratch("npx").join("npx");
    write_script(&npx, "#!/bin/sh\nexec cat > /dev/null\n");
    let (socket, _daemon) = daemon_with(Config { npx: Some(npx), browser_host: true, ..config(&home, &claude) }).await;
    let (mut screen, _) = Client::connect(&socket, "test", "acme").await.unwrap();
    screen.send(Command::Start { session: 51, spec: SessionSpec { role: Some("pm".into()), ..spec(&project) } });
    until(&mut screen, |e| matches!(e, Event::SessionStarted { .. })).await;
    let call = ToolCall { squad: "acme".into(), member: "pm".into(), name: "show_my_browser".into(), arguments: serde_json::json!({ "why": "Sign in to App Store Connect" }) };

    // The app attaches, saying it has a panel (closed for now).
    let app = tokio::net::UnixStream::connect(&socket).await.unwrap();
    let (reader, mut writer) = app.into_split();
    let hello = HelloParams { version: "app".into(), squad: "acme".into(), lang: None, browser_panel: true };
    writer.write_all(Message::request(1, rpc::HELLO, hello).to_line().as_bytes()).await.unwrap();
    let mut lines = tokio::io::BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await.unwrap() {
        if Message::from_line(&line).is_ok_and(|m| m.id == Some(1)) {
            break;
        }
    }
    let result = gateway::call_tool(&socket, &call).await.unwrap();
    assert!(result.text.contains("browser panel"), "to the app's panel: {result:?}");

    // The app gone: the window on this Mac, as before.
    drop((lines, writer));
    tokio::time::sleep(Duration::from_millis(200)).await;
    let result = gateway::call_tool(&socket, &call).await.unwrap();
    assert!(result.text.contains("comes up on the person's screen"), "{result:?}");
    gateway::shutdown(&socket).await.unwrap();
}

#[tokio::test]
async fn only_one_daemon_per_socket() {
    let home = scratch("home");
    let claude = fake_claude(&slow_answer("0"));
    let (socket, _daemon) = daemon(&home, &claude).await;
    let second = gateway::serve(config(&home, &claude), &socket, "test").await;
    assert_eq!(second.unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
    gateway::shutdown(&socket).await.unwrap();
}

#[tokio::test]
async fn a_stale_socket_file_does_not_block_a_new_daemon() {
    let home = scratch("home");
    let claude = fake_claude(&slow_answer("0"));
    let socket = gateway::socket_path(&home);
    std::fs::write(&socket, b"").unwrap(); // left behind by a daemon that crashed
    let (socket, _daemon) = daemon(&home, &claude).await;
    assert!(gateway::is_running(&socket).await);
    gateway::shutdown(&socket).await.unwrap();
}

#[tokio::test]
async fn a_screen_that_said_hello_reads_back_an_upload_and_nothing_else() {
    use equipe_core::protocol::rpc::{self, Message, ReadUpload, UploadRead};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
    let (home, _project) = squad();
    let claude = fake_claude(&slow_answer("0"));
    let (socket, _daemon) = daemon(&home, &claude).await;
    let (_, shot) = gateway::create_upload(&home, "paste.png", None).unwrap();
    std::fs::write(&shot, b"picture").unwrap();

    let stream = tokio::net::UnixStream::connect(&socket).await.unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let mut ask = async |id: u64, method: &str, params: serde_json::Value| -> Message {
        writer.write_all(Message::request(id, method, params).to_line().as_bytes()).await.unwrap();
        loop {
            let message = Message::from_line(&lines.next_line().await.unwrap().unwrap()).unwrap();
            if message.id == Some(id) {
                return message;
            }
        }
    };
    let read = |path: &std::path::Path| serde_json::to_value(ReadUpload { path: path.to_owned(), size_only: false }).unwrap();

    let before = ask(1, rpc::READ_UPLOAD, read(&shot)).await;
    assert!(before.error.is_some(), "not before hello: {before:?}");
    let hello = ask(2, rpc::HELLO, serde_json::json!({ "version": "test", "squad": "acme" })).await;
    assert!(hello.result.is_some());
    let got = ask(3, rpc::READ_UPLOAD, read(&shot)).await.result_as::<UploadRead>().unwrap();
    assert_eq!(got.size, 7);
    use base64::Engine as _;
    assert_eq!(base64::engine::general_purpose::STANDARD.decode(got.data.unwrap()).unwrap(), b"picture");
    let outside = ask(4, rpc::READ_UPLOAD, read(&home.join("squads.json"))).await;
    assert!(outside.error.is_some() && outside.result.is_none(), "{outside:?}");
    let climbing = ask(5, rpc::READ_UPLOAD, read(&gateway::uploads_dir(&home).join("../squads.json"))).await;
    assert!(climbing.error.is_some());
    gateway::shutdown(&socket).await.unwrap();
}

#[tokio::test]
async fn the_socket_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let home = scratch("home");
    let claude = fake_claude(&slow_answer("0"));
    let (socket, _daemon) = daemon(&home, &claude).await;
    let mode = std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    gateway::shutdown(&socket).await.unwrap();
}

#[tokio::test]
async fn the_daemon_says_who_is_mid_turn() {
    let (home, project) = squad();
    let claude = fake_claude(&slow_answer("1"));
    let (socket, _daemon) = daemon(&home, &claude).await;
    assert!(gateway::busy(&socket).await.unwrap().unwrap().idle(), "nobody yet");

    let (mut screen, _) = Client::connect(&socket, "test", "acme").await.unwrap();
    screen.send(Command::Start { session: 4, spec: spec(&project) });
    until(&mut screen, |e| matches!(e, Event::SessionStarted { .. })).await;
    screen.send(Command::Submit { session: 4, text: "a long job".into(), about: None });
    until(&mut screen, |e| matches!(e, Event::UserMessage { .. })).await;
    let busy = gateway::busy(&socket).await.unwrap().unwrap();
    assert_eq!(busy.working, ["acme/designer"]);
    assert!(!busy.idle());

    until(&mut screen, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(gateway::busy(&socket).await.unwrap().unwrap().idle(), "between turns: safe to restart");
    gateway::shutdown(&socket).await.unwrap();
    assert_eq!(gateway::busy(&socket).await.unwrap(), None, "no daemon, no answer");
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

/// A stand-in for the daemon's phone link on `home`'s `remote.sock`: a code
/// for `pair`, the list for `devices`; every line it reads goes to the result.
fn fake_phone_link(home: &Path) -> tokio::sync::mpsc::UnboundedReceiver<String> {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::UnixListener::bind(equipe_core::pairing::socket_path(home)).unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let tx = tx.clone();
            tokio::spawn(async move {
                let (reader, mut writer) = stream.into_split();
                let mut lines = tokio::io::BufReader::new(reader).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx.send(line.clone());
                    let answer = if line.contains("\"pair\"") {
                        let expires = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 + 300_000;
                        format!("{{\"qr\":\"equipe://pair?s=secret\",\"expires\":{expires}}}\n")
                    } else if line.contains("\"devices\"") {
                        "{\"op\":\"devices\",\"devices\":[{\"id\":\"a1\",\"name\":\"iPhone\",\"platform\":\"ios\",\"added\":1}],\"relay\":\"https://r\",\"connected\":true}\n".to_owned()
                    } else {
                        continue;
                    };
                    let _ = writer.write_all(answer.as_bytes()).await;
                }
                let _ = tx.send("closed".into());
            });
        }
    });
    rx
}

#[tokio::test]
async fn a_screen_pairs_a_phone_with_the_daemons_machine_and_only_it_sees_the_code() {
    let (home, _project) = squad();
    let claude = fake_claude(&slow_answer("0"));
    let (socket, _daemon) = daemon(&home, &claude).await;
    let mut link = fake_phone_link(&home);
    let (mut asker, _) = Client::connect(&socket, "test", "acme").await.unwrap();
    let (mut other, _) = Client::connect(&socket, "test", "acme").await.unwrap();
    asker.send(Command::Devices);
    let seen = until(&mut asker, |e| matches!(e, Event::Devices { .. })).await;
    assert!(matches!(seen.last(), Some(Event::Devices { devices, connected: true, .. }) if devices[0].name == "iPhone"));
    asker.send(Command::PairStart);
    let seen = until(&mut asker, |e| matches!(e, Event::PairOffer { .. })).await;
    assert!(matches!(seen.last(), Some(Event::PairOffer { url, seconds }) if url == "equipe://pair?s=secret" && *seconds > 290));
    // The runtime never hears of it, and no other screen sees the code.
    tokio::time::sleep(Duration::from_millis(200)).await;
    while let Some(event) = other.try_event() {
        assert!(!matches!(event, Event::PairOffer { .. } | Event::Devices { .. }), "{event:?}");
    }
    assert_eq!(link.recv().await.unwrap(), r#"{"op":"devices"}"#);
    assert_eq!(link.recv().await.unwrap(), r#"{"op":"pair"}"#);
    // The screen leaving closes its line to the link, which closes its code.
    drop(asker);
    let closed = tokio::time::timeout(Duration::from_secs(5), link.recv()).await.unwrap().unwrap();
    assert_eq!(closed, "closed");
    gateway::shutdown(&socket).await.unwrap();
}

#[tokio::test]
async fn a_daemon_without_its_phone_link_says_so_in_the_screens_language() {
    let (home, _project) = squad();
    let claude = fake_claude(&slow_answer("0"));
    let (socket, _daemon) = daemon(&home, &claude).await;
    let (mut screen, _) = Client::connect_in(&socket, "test", "acme", Some(equipe_core::i18n::Lang::De)).await.unwrap();
    screen.send(Command::PairStart);
    let seen = until(&mut screen, |e| matches!(e, Event::PairEnded { .. })).await;
    assert!(
        matches!(seen.last(), Some(Event::PairEnded { why: equipe_core::protocol::PairEnd::Unavailable, message }) if message.contains("Daemon")),
        "{seen:#?}"
    );
    gateway::shutdown(&socket).await.unwrap();
}
