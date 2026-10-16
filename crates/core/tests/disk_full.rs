//! A full disk: what can't be saved is lost, and that is all — the daemon
//! says so once and goes on serving. A process of its own: its journals
//! can't grow ([`journal::no_room_for_tests`]), as on a disk with no room.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::journal;
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config};
use equipe_core::squads;
use equipe_providers::CredentialStore;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("equipe-full-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn fake_claude(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = dir.join("claude");
    let script = r#"#!/bin/sh
if [ "$1" = "auth" ]; then echo '{"loggedIn":true,"authMethod":"claude.ai"}'; exit 0; fi
cat <<EOF
{"type":"system","subtype":"init","model":"m","session_id":"sess-1"}
{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{"input_tokens":1,"output_tokens":1}}
EOF
"#;
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[tokio::test]
async fn a_full_disk_is_said_once_and_the_daemon_goes_on_serving() {
    let home = scratch("home");
    let project = scratch("project");
    squads::create(&home, "acme", &project).unwrap();
    journal::no_room_for_tests();
    let claude = fake_claude(&scratch("bin"));
    let config = Config {
        store: CredentialStore::new(&home),
        home: home.clone(),
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
    };
    let (handle, mut events) = runtime::start(config);
    let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some("pm".into()), squad: Some("acme".into()) };
    handle.send(Command::Start { session: 1, spec });
    // A message far bigger than the room the journal has left.
    handle.send(Command::Submit { session: 1, text: "plan the week ".repeat(20_000) });

    let mut full = 0;
    let mut answered = false;
    let wait = async {
        while let Some(event) = events.recv().await {
            match &event {
                Event::Error { message, .. } if message.contains("disk is full") => full += 1,
                Event::TurnEnded { .. } => answered = true,
                _ => {}
            }
            if full > 0 && answered {
                return;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(20), wait).await.expect("told, and the turn still ran");
    // Said once, however many writes failed; and the daemon still answers.
    handle.send(Command::Submit { session: 1, text: "and again ".repeat(20_000) });
    tokio::time::sleep(Duration::from_millis(500)).await;
    while let Ok(event) = events.try_recv() {
        if matches!(&event, Event::Error { message, .. } if message.contains("disk is full")) {
            full += 1;
        }
    }
    assert_eq!(full, 1);
    let busy = tokio::time::timeout(Duration::from_secs(5), handle.busy()).await.expect("busy answers");
    assert_eq!(busy.stalled, None);
}
