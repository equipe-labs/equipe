//! A turn that lost its engine's sign-in, and its card, as the daemon would
//! tell it — nothing is run, nothing is signed in. `FAKE_SIGNIN`:
//! `outside` — the daemon runs outside the Mac's login session (started over
//! SSH): "로그인 세션에서 다시 띄우기" moves it, and 2.5 s later Claude Code
//! works and the turn carries on. `outside_stuck` — the same, with no login
//! service to move it with. `locked` — the login Keychain is locked;
//! "다시 확인" finds it unlocked the second time. `expired` — signed out:
//! the sign-in shows its page, and a pasted code signs it in. `failed` — the
//! pasted code is refused. `missing` — Claude Code isn't installed.
//! `unknown` — it couldn't tell. `back` — signed in again, the turn carried
//! on. `checking` — still being looked at. `codex` — Codex's sign-in lost
//! instead (the code's way).

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use equipe_core::protocol::{AuthState, Command, Engine, Entry, Event, SignInFailure, SignInStep};
use tokio::sync::broadcast;

pub fn mode() -> Option<String> {
    std::env::var("FAKE_SIGNIN").ok().filter(|v| !v.is_empty())
}

pub fn on() -> bool {
    mode().is_some()
}

fn engine() -> Engine {
    if mode().as_deref() == Some("codex") { Engine::Codex } else { Engine::Claude }
}

/// The PM's turn, stopped by it: what was asked, and the error.
pub fn history() -> Vec<Entry> {
    let (message, next) = match engine() {
        Engine::Claude => ("Failed to authenticate: OAuth session expired and could not be refreshed", equipe_core::engine_auth::CLAUDE_HINT),
        Engine::Codex => ("Codex CLI에 로그인되어 있지 않아요. `codex login`으로 로그인하세요.", equipe_core::engine_auth::CODEX_HINT),
    };
    vec![Entry::User { text: "헤더를 조금 더 차분하게 바꿔 줘".into() }, Entry::Error { message: message.into(), next: Some(next.into()) }]
}

static CHECKS: AtomicU32 = AtomicU32::new(0);

fn first_state() -> AuthState {
    match mode().as_deref() {
        Some("outside") => AuthState::OutsideLogin { movable: true },
        Some("outside_stuck") => AuthState::OutsideLogin { movable: false },
        Some("locked") => AuthState::KeychainLocked,
        Some("missing") => AuthState::NotInstalled,
        Some("unknown") => AuthState::Unknown,
        Some("back") => AuthState::SignedIn { resumed: 1 },
        Some("checking") => AuthState::Checking,
        _ => AuthState::SignedOut,
    }
}

fn tell(events: &broadcast::Sender<Event>, state: AuthState) {
    let stopped = if matches!(state, AuthState::SignedIn { .. }) { vec![] } else { vec![1] };
    let detail = if state == AuthState::Unknown { "`claude auth status` didn't answer in 20s".into() } else { String::new() };
    let _ = events.send(Event::EngineAuth { engine: engine(), state, detail, stopped });
}

/// Signed in again: the stopped turn carries on, the conversation told.
fn back(events: &broadcast::Sender<Event>) {
    let name = if engine() == Engine::Claude { "Claude Code" } else { "Codex" };
    let _ = events.send(Event::Notice { session: 1, text: format!("{name}에 다시 로그인돼서, 멈췄던 일을 이어서 해요.") });
    tell(events, AuthState::SignedIn { resumed: 1 });
}

/// What a screen hears as it attaches.
pub fn attached(events: broadcast::Sender<Event>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        tell(&events, first_state());
    });
}

pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    if !on() {
        return false;
    }
    let events = events.clone();
    match command {
        Command::CheckEngine { .. } => {
            tokio::spawn(async move {
                tell(&events, AuthState::Checking);
                tokio::time::sleep(Duration::from_millis(700)).await;
                // A locked Keychain is found unlocked the second time it's looked at.
                if mode().as_deref() == Some("locked") && CHECKS.fetch_add(1, Ordering::SeqCst) >= 1 {
                    back(&events);
                } else {
                    tell(&events, first_state());
                }
            });
        }
        Command::MoveIntoLogin => {
            tokio::spawn(async move {
                tell(&events, AuthState::Moving);
                tokio::time::sleep(Duration::from_millis(2500)).await;
                back(&events);
            });
        }
        Command::SignIn { engine, device } => {
            let (engine, device) = (*engine, *device);
            tokio::spawn(async move {
                let step = |step| drop(events.send(Event::SignIn { engine, step }));
                step(SignInStep::Browser { url: None });
                tokio::time::sleep(Duration::from_millis(400)).await;
                match engine {
                    Engine::Claude => step(SignInStep::Paste { url: "https://claude.com/cai/oauth/authorize?code=true&client_id=example".into() }),
                    Engine::Codex if device => {
                        step(SignInStep::DeviceCode { url: "https://auth.openai.com/codex/device".into(), code: "WXYZ-12345".into() });
                        tokio::time::sleep(Duration::from_millis(4000)).await;
                        step(SignInStep::Done);
                        back(&events);
                    }
                    Engine::Codex => {
                        tokio::time::sleep(Duration::from_millis(2000)).await;
                        step(SignInStep::Done);
                        back(&events);
                    }
                }
            });
        }
        Command::SignInCode { engine, .. } => {
            let engine = *engine;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                if mode().as_deref() == Some("failed") {
                    let _ = events.send(Event::SignIn { engine, step: SignInStep::Failed { why: SignInFailure::Failed, detail: "Login failed: Request failed with status code 400".into() } });
                    return;
                }
                let _ = events.send(Event::SignIn { engine, step: SignInStep::Done });
                back(&events);
            });
        }
        Command::SignInCancel { engine } => {
            let _ = events.send(Event::SignIn { engine: *engine, step: SignInStep::Cancelled });
        }
        _ => return false,
    }
    true
}
