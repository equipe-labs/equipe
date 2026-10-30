//! The engines from a screen: where each stands, a Codex sign-in run by its
//! own tool on the daemon's machine, and signing out — without a
//! network, a real login or the person's own Claude Code.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::{Access, Command, Engine, EngineState, EngineStatus, Event, MemberRef, SessionSpec, SignInFailure, SignInStep};
use equipe_core::runtime::{self, Config};
use equipe_providers::{AuthMethod, CredentialStore, Credentials};
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("eqe-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `claude` that says it is signed in to a Max plan, and answers turns.
fn fake_claude() -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = scratch("bin").join("claude");
    let script = r#"#!/bin/sh
if [ "$1" = "auth" ]; then echo '{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"max"}'; exit 0; fi
cat <<EOF
{"type":"system","subtype":"init","model":"m","session_id":"sess-1"}
{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{"input_tokens":1,"output_tokens":1}}
EOF
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
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        person_home: Some(home.join("person")),
        lang: equipe_core::i18n::Lang::En,
        nemotron: None,
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
    tokio::time::timeout(Duration::from_secs(15), wait).await.expect("timed out")
}

async fn engines(events: &mut UnboundedReceiver<Event>) -> Vec<EngineStatus> {
    let Event::Engines { engines } = next(events, |e| matches!(e, Event::Engines { .. })).await else { unreachable!() };
    engines
}

async fn sign_in(events: &mut UnboundedReceiver<Event>) -> SignInStep {
    let Event::SignIn { step, .. } = next(events, |e| matches!(e, Event::SignIn { .. })).await else { unreachable!() };
    step
}

/// A `codex` that keeps its login as a file in `dir`: `login --device-auth`
/// prints an address and a code, then signs in; `login` (the browser's way)
/// waits for the browser, which never comes; `logout` forgets.
fn fake_codex(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("codex");
    let login = dir.join("login");
    let script = format!(
        r#"#!/bin/sh
case "$*" in
  "login status") if [ -f '{login}' ]; then echo 'Logged in using ChatGPT' >&2; exit 0; else echo 'Not logged in' >&2; exit 1; fi;;
  "login --device-auth") printf '1. Open this link in your browser and sign in to your account\n   \033[94mhttps://auth.openai.com/codex/device\033[0m\n\n2. Enter this one-time code (expires in 15 minutes)\n   \033[94mWXYZ-12345\033[0m\n'; sleep 0.3; touch '{login}'; echo 'Successfully logged in' >&2;;
  "login") echo 'If your browser did not open, navigate to this URL to authenticate:' >&2; echo 'https://auth.openai.com/oauth/authorize?state=s' >&2; sleep 30;;
  "logout") rm -f '{login}'; echo 'Successfully logged out' >&2;;
esac
"#,
        login = login.display()
    );
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// Every runtime here runs the fake `codex`, never the person's own.
fn use_fake_codex() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let codex = fake_codex(&scratch("codex"));
        // SAFETY: set once, before this binary's runtimes read it.
        unsafe { std::env::set_var("EQUIPE_CODEX", codex) };
    });
}

#[tokio::test]
async fn a_screen_sees_where_the_engines_stand_and_who_is_on_each() {
    use_fake_codex();
    let home = scratch("home");
    let project = scratch("proj");
    equipe_core::squads::create(&home, "acme", &project.canonicalize().unwrap()).unwrap();
    let (core, mut events) = runtime::start(config(&home, &fake_claude()));
    let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some("designer".into()), squad: Some("acme".into()) };
    core.send(Command::Start { session: 7, spec });
    next(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;

    core.send(Command::Engines);
    let listed = engines(&mut events).await;
    let claude = listed.iter().find(|e| e.engine == Engine::Claude).unwrap();
    assert_eq!((claude.state, claude.plan.as_deref(), claude.per_token), (EngineState::SignedIn, Some("max"), false));
    assert_eq!(claude.members, vec![MemberRef { squad: "acme".into(), member: "designer".into() }]);
    assert!(listed.iter().any(|e| e.engine == Engine::Codex && e.members.is_empty() && !e.signing_in));
}

/// Every step with the fake `codex` in one test: it keeps one login.
#[tokio::test]
async fn codex_signs_in_with_its_own_tool_on_the_daemons_machine_and_out_again() {
    use_fake_codex();
    let home = scratch("home");
    let store = CredentialStore::new(&home);
    let (core, mut events) = runtime::start(config(&home, &fake_claude()));
    core.send(Command::Engines);
    assert!(engines(&mut events).await.iter().any(|e| e.engine == Engine::Codex && e.state == EngineState::SignedOut && !e.legacy));

    // From another Mac: an address and a code for the screens, then signed in.
    core.send(Command::SignIn { engine: Engine::Codex, device: true });
    assert_eq!(sign_in(&mut events).await, SignInStep::DeviceCode { url: "https://auth.openai.com/codex/device".into(), code: "WXYZ-12345".into() });
    assert_eq!(sign_in(&mut events).await, SignInStep::Done);
    let after = engines(&mut events).await;
    assert!(after.iter().any(|e| e.engine == Engine::Codex && e.state == EngineState::SignedIn && !e.signing_in), "{after:?}");

    // On this Mac: the browser opens here; the person can stop waiting for it.
    core.send(Command::SignIn { engine: Engine::Codex, device: false });
    assert_eq!(sign_in(&mut events).await, SignInStep::Browser { url: None });
    assert_eq!(sign_in(&mut events).await, SignInStep::Browser { url: Some("https://auth.openai.com/oauth/authorize?state=s".into()) });
    core.send(Command::Engines);
    assert!(engines(&mut events).await.iter().any(|e| e.engine == Engine::Codex && e.signing_in), "a screen opened now sees it under way");
    core.send(Command::SignInCancel { engine: Engine::Codex });
    assert_eq!(sign_in(&mut events).await, SignInStep::Cancelled);
    assert!(!engines(&mut events).await.iter().any(|e| e.signing_in), "and no longer under way");

    // equipe's own older login shows beside the tool's; signing out is the tool's.
    store.save(&Credentials { provider: "openai".into(), method: AuthMethod::OauthSubscription, access: "a.b.c".into(), refresh: None, expires: None, account_id: None, residency: None }).unwrap();
    core.send(Command::SignOut { engine: Engine::Codex });
    let out = engines(&mut events).await;
    assert!(out.iter().any(|e| e.engine == Engine::Codex && e.state == EngineState::SignedOut && e.legacy), "{out:?}");
    assert!(out.iter().any(|e| e.engine == Engine::Claude && e.state == EngineState::SignedIn), "Claude Code's own login stays");

    // Claude Code signs in on its own: equipe offers no sign-in for it.
    core.send(Command::SignIn { engine: Engine::Claude, device: false });
    assert!(matches!(sign_in(&mut events).await, SignInStep::Failed { why: SignInFailure::Unsupported, .. }));
}
