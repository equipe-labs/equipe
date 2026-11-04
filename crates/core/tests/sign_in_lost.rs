//! A member's turn that fails because Claude Code's sign-in no longer works
//! here: equipe looks at why, every screen hears it, the person signs in
//! again from a screen with a link and a pasted code — no terminal — and the
//! stopped turn carries on by itself, the member told why and what it was
//! asked. Without a network, a real login or the person's own Claude Code.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::{Access, AuthState, Command, Engine, Event, SessionSpec, SignInStep, TurnOutcome};
use equipe_core::runtime::{self, Config};
use equipe_providers::CredentialStore;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("eqs-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `claude` whose sign-in is a file in `dir`: without it, `auth status`
/// says signed out and a turn fails as an expired OAuth session does;
/// `auth login` prints its page, reads a code, and signs in with the right
/// one. Each turn's first line in is kept in `asked`.
fn fake_claude(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("claude");
    let script = format!(
        r#"#!/bin/sh
D='{dir}'
if [ "$1 $2" = "auth status" ]; then
  if [ -f "$D/signed" ]; then echo '{{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"max"}}'; else echo '{{"loggedIn":false,"authMethod":"none"}}'; fi
  exit 0
fi
if [ "$1 $2" = "auth login" ]; then
  echo "Opening browser to sign in…"
  echo "If the browser didn't open, visit: https://claude.com/cai/oauth/authorize?code=true&state=s"
  printf 'Paste code here if prompted > '
  read code
  if [ "$code" = "good#s" ]; then touch "$D/signed"; echo 'Login successful.'; exit 0; fi
  echo 'Login failed: Request failed with status code 400'; exit 1
fi
read -r line
printf "%s\\n" "$line" >> "$D/asked"
if [ -f "$D/signed" ]; then
  echo '{{"type":"system","subtype":"init","model":"m","session_id":"sess-1"}}'
  echo '{{"type":"result","subtype":"success","is_error":false,"result":"done","usage":{{"input_tokens":1,"output_tokens":1}}}}'
else
  echo '{{"type":"result","subtype":"success","is_error":true,"result":"Failed to authenticate: OAuth session expired and could not be refreshed"}}'
fi
"#,
        dir = dir.display()
    );
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

/// The next thing found about Claude Code that isn't "looking".
async fn verdict(events: &mut UnboundedReceiver<Event>) -> (AuthState, Vec<u64>) {
    let Event::EngineAuth { state, stopped, .. } = next(events, |e| matches!(e, Event::EngineAuth { engine: Engine::Claude, state, .. } if *state != AuthState::Checking)).await else { unreachable!() };
    (state, stopped)
}

#[tokio::test]
async fn a_lost_claude_sign_in_is_found_signed_in_again_from_a_screen_and_the_work_carries_on() {
    // This machine's own session and Keychain don't decide the test.
    // SAFETY: set before this binary's only runtime reads them.
    unsafe {
        std::env::set_var("EQUIPE_LOGIN_SESSION", "inside");
        std::env::set_var("EQUIPE_KEYCHAIN", "unlocked");
    }
    let (home, project, bin) = (scratch("home"), scratch("proj"), scratch("bin"));
    let claude = fake_claude(&bin);
    equipe_core::squads::create(&home, "acme", &project.canonicalize().unwrap()).unwrap();
    let (core, mut events) = runtime::start(config(&home, &claude));
    let spec = SessionSpec { engine: Some(Engine::Claude), model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some("designer".into()), squad: Some("acme".into()) };
    // It started signed in; then the sign-in stopped working, as an OAuth session expires.
    std::fs::write(bin.join("signed"), "").unwrap();
    core.send(Command::Start { session: 7, spec });
    next(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    std::fs::remove_file(bin.join("signed")).unwrap();

    core.send(Command::Submit { session: 7, text: "make the header calmer".into(), about: None });
    let Event::Error { next: hint, .. } = next(&mut events, |e| matches!(e, Event::Error { session: Some(7), .. })).await else { unreachable!() };
    assert_eq!(equipe_core::engine_auth::lost_by(hint.as_deref()), Some(Engine::Claude), "{hint:?}");
    // Every screen hears it's being looked at, then what was found.
    next(&mut events, |e| matches!(e, Event::EngineAuth { engine: Engine::Claude, state: AuthState::Checking, .. })).await;
    let (state, stopped) = verdict(&mut events).await;
    assert_eq!((state, stopped), (AuthState::SignedOut, vec![7]), "inside, unlocked, signed out: a sign-in is what's missing");

    // From a screen on another Mac: the page to open, and the code pasted back.
    core.send(Command::SignIn { engine: Engine::Claude, device: true });
    let url = loop {
        let Event::SignIn { step, .. } = next(&mut events, |e| matches!(e, Event::SignIn { engine: Engine::Claude, .. })).await else { unreachable!() };
        if let SignInStep::Paste { url } = step {
            break url;
        }
    };
    assert!(url.starts_with("https://claude.com/cai/oauth/authorize"), "{url}");
    core.send(Command::SignInCode { engine: Engine::Claude, code: "  good#s \n".into() });
    let Event::SignIn { step, .. } = next(&mut events, |e| matches!(e, Event::SignIn { engine: Engine::Claude, step: SignInStep::Done | SignInStep::Failed { .. } })).await else { unreachable!() };
    assert_eq!(step, SignInStep::Done);

    // Signed in: the stopped turn carries on by itself, the member told why and what it was asked.
    let (state, _) = verdict(&mut events).await;
    assert_eq!(state, AuthState::SignedIn { resumed: 1 });
    let Event::TurnEnded { outcome, .. } = next(&mut events, |e| matches!(e, Event::TurnEnded { session: 7, .. })).await else { unreachable!() };
    assert_eq!(outcome, TurnOutcome::Completed);
    let asked = std::fs::read_to_string(bin.join("asked")).unwrap();
    let carried_on = asked.lines().last().unwrap();
    assert!(carried_on.contains("make the header calmer") && carried_on.contains("sign-in"), "{carried_on}");
    assert_eq!(asked.lines().count(), 2, "once failed, once carried on: {asked}");
}
