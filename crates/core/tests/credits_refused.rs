//! A hosted team on equipe credits (`EQUIPE_AI=credits`): its Claude Code
//! calls go through equipe's gateway with equipe's key, so a 401/403 is the
//! gateway refusing the call — never a Claude sign-out. The turn's error says
//! so in plain words, with no `claude auth login` hint, and no sign-in watch,
//! sign-in page or "signed back in" notice follows.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::{Access, Command, Engine, Event, SessionSpec};
use equipe_core::runtime::{self, Config};
use equipe_providers::CredentialStore;
use tokio::sync::mpsc::UnboundedReceiver;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("eqc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `claude` whose every turn the gateway refuses with a 403.
fn refused_claude(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("claude");
    let script = r#"#!/bin/sh
if [ "$1 $2" = "auth status" ]; then echo '{"loggedIn":false,"authMethod":"none"}'; exit 0; fi
if [ "$1 $2" = "auth login" ]; then read code; exit 1; fi
read -r line
echo '{"type":"result","subtype":"success","is_error":true,"result":"Failed to authenticate. API Error: 403 {\"type\":\"error\",\"error\":{\"type\":\"permission_error\",\"message\":\"forbidden\"}}"}'
"#;
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn config(home: &Path, claude: &Path) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        codex_binary: "/nonexistent/codex".into(),
        persist: false,
        retention_days: 90,
        equipe_binary: None,
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

async fn next(events: &mut UnboundedReceiver<Event>, want: impl Fn(&Event) -> bool) -> Event {
    let wait = async {
        loop {
            let event = events.recv().await.expect("the runtime stopped");
            if want(&event) {
                return event;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(20), wait).await.expect("timed out")
}

#[tokio::test]
async fn on_equipe_credits_a_refused_call_is_said_plainly_and_starts_no_sign_in() {
    // SAFETY: set before this binary's only runtime reads them.
    unsafe {
        std::env::set_var("EQUIPE_AI", "credits");
        std::env::set_var("EQUIPE_LOGIN_SESSION", "inside");
        std::env::set_var("EQUIPE_KEYCHAIN", "unlocked");
    }
    let (home, project, bin) = (scratch("home"), scratch("proj"), scratch("bin"));
    let claude = refused_claude(&bin);
    equipe_core::squads::create(&home, "acme", &project.canonicalize().unwrap()).unwrap();
    let (core, mut events) = runtime::start(config(&home, &claude));
    let spec = SessionSpec { engine: Some(Engine::Claude), model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some("designer".into()), squad: Some("acme".into()) };
    core.send(Command::Start { session: 7, spec });
    next(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;

    core.send(Command::Submit { session: 7, text: "make the header calmer".into(), about: None });
    let Event::Error { message, next: hint, .. } = next(&mut events, |e| matches!(e, Event::Error { session: Some(7), .. })).await else { unreachable!() };
    assert_eq!(hint, None, "nothing to sign in to: {message}");
    assert_eq!(message, equipe_core::i18n::core(equipe_core::i18n::Lang::En).get("error.gateway_refused"));
    assert!(!message.contains("Mac") && !message.contains("Terminal"), "{message}");

    // No sign-in watch, page or notice follows.
    let after = async {
        let mut heard = Vec::new();
        while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(3), events.recv()).await {
            heard.push(event);
        }
        heard
    }
    .await;
    let watched = after.iter().any(|e| matches!(e, Event::EngineAuth { stopped, .. } if !stopped.is_empty()) || matches!(e, Event::SignIn { .. }));
    assert!(!watched, "{after:#?}");
    assert!(!after.iter().any(|e| matches!(e, Event::Notice { text, .. } if text.contains("signed in again"))), "{after:#?}");
}
