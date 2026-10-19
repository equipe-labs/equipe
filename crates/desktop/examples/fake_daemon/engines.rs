//! The AI platforms in settings, as the daemon would tell them — no `claude`
//! or `codex` is run, and nothing is signed in. `FAKE_ENGINES`:
//! (unset) or `signed_out` — Claude Code on a Max plan with the PM and the
//! designer on it, Codex signed out; a sign-in shows the browser's way (the
//! code's, from another Mac or with `FAKE_ENGINES_CODE=1`) and is signed in
//! 2.5 s later. `signing_in` — the same, but the browser never comes back.
//! `signed_in` — Codex too, on Plus with backend on it. `failed` — a sign-in
//! fails: with a code, as an account with device codes turned off.
//! `missing` — neither `claude` nor `codex` is installed. `claude_out` —
//! Claude Code signed out, Codex signed in. `legacy` — Codex's own CLI
//! signed out, equipe's older login kept beside it.

use std::time::Duration;

use equipe_core::protocol::{Command, Engine, EngineState, EngineStatus, Event, MemberRef, PlanUsage, PlanWindow, SignInFailure, SignInStep};
use tokio::sync::broadcast;

fn mode() -> String {
    std::env::var("FAKE_ENGINES").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "signed_out".into())
}

/// Codex's state, as signing in and out moves it.
fn codex() -> &'static std::sync::Mutex<EngineState> {
    static CODEX: std::sync::OnceLock<std::sync::Mutex<EngineState>> = std::sync::OnceLock::new();
    CODEX.get_or_init(|| {
        std::sync::Mutex::new(match mode().as_str() {
            "signed_in" | "claude_out" => EngineState::SignedIn,
            "missing" => EngineState::NotInstalled,
            _ => EngineState::SignedOut,
        })
    })
}

fn signing_in() -> &'static std::sync::Mutex<Option<tokio::task::JoinHandle<()>>> {
    static TASK: std::sync::OnceLock<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>> = std::sync::OnceLock::new();
    TASK.get_or_init(Default::default)
}

/// The Claude plan's usage, as Claude Code would have reported it.
pub fn plan() -> Option<PlanUsage> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    (mode() != "missing" && mode() != "claude_out").then_some(PlanUsage {
        five_hour: Some(PlanWindow { used_percent: 42, resets_at: now + 2 * 3600 + 10 * 60 }),
        seven_day: Some(PlanWindow { used_percent: 76, resets_at: now + 3 * 86_400 }),
    })
}

fn statuses() -> Vec<EngineStatus> {
    let member = |name: &str| MemberRef { squad: "demo".into(), member: name.into() };
    let mut claude = EngineStatus::new(Engine::Claude, EngineState::SignedIn);
    match mode().as_str() {
        "missing" => claude.state = EngineState::NotInstalled,
        "claude_out" => claude.state = EngineState::SignedOut,
        _ => {
            claude.plan = Some("max".into());
            claude.members = vec![member("pm"), member("designer"), member("frontend")];
        }
    }
    let mut codex = EngineStatus::new(Engine::Codex, *self::codex().lock().unwrap());
    if codex.state == EngineState::SignedIn {
        codex.plan = Some("plus".into());
        codex.account = Some("abel@example.com".into());
        codex.members = vec![member("backend")];
    }
    codex.signing_in = signing_in().lock().unwrap().as_ref().is_some_and(|t| !t.is_finished());
    codex.legacy = mode() == "legacy";
    vec![claude, codex]
}

pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    let tell = |engine, step| drop(events.send(Event::SignIn { engine, step }));
    match command {
        Command::Engines => {
            let events = events.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let _ = events.send(Event::Engines { engines: statuses() });
            });
        }
        Command::SignIn { engine: Engine::Claude, .. } => tell(Engine::Claude, SignInStep::Failed { why: SignInFailure::Unsupported, detail: String::new() }),
        Command::SignIn { engine, device } => {
            let (engine, device) = (*engine, *device || std::env::var("FAKE_ENGINES_CODE").is_ok_and(|v| v == "1"));
            let said = events.clone();
            let task = tokio::spawn(async move {
                let tell = |step| drop(said.send(Event::SignIn { engine, step }));
                tokio::time::sleep(Duration::from_millis(500)).await;
                if *codex().lock().unwrap() == EngineState::NotInstalled {
                    return tell(SignInStep::Failed { why: SignInFailure::NotInstalled, detail: String::new() });
                }
                if device {
                    tell(SignInStep::DeviceCode { url: "https://auth.openai.com/codex/device".into(), code: "K7QD-M4XP9".into() });
                } else {
                    tell(SignInStep::Browser { url: None });
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    tell(SignInStep::Browser { url: Some("https://auth.openai.com/oauth/authorize?client_id=fake&state=fake-for-screenshots".into()) });
                }
                match mode().as_str() {
                    "signing_in" => std::future::pending::<()>().await,
                    "failed" => {
                        tokio::time::sleep(Duration::from_millis(900)).await;
                        let (why, detail) = if device {
                            (SignInFailure::DeviceCodeOff, "Error logging in with device code: device code login is not enabled for this Codex server. Use the browser login or verify the server URL.")
                        } else {
                            (SignInFailure::Failed, "Error logging in: the browser closed the sign-in page")
                        };
                        tell(SignInStep::Failed { why, detail: detail.into() });
                    }
                    _ => {
                        tokio::time::sleep(Duration::from_millis(2500)).await;
                        *codex().lock().unwrap() = EngineState::SignedIn;
                        tell(SignInStep::Done);
                    }
                }
            });
            if let Some(old) = signing_in().lock().unwrap().replace(task) {
                old.abort();
            }
            // Told once the step is out, as the daemon tells it after the tool ends.
            let events = events.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    if signing_in().lock().unwrap().as_ref().is_none_or(|t| t.is_finished()) {
                        break;
                    }
                }
                let _ = events.send(Event::Engines { engines: statuses() });
            });
        }
        Command::SignInCancel { engine } => {
            if let Some(task) = signing_in().lock().unwrap().take() {
                task.abort();
                tell(*engine, SignInStep::Cancelled);
                let _ = events.send(Event::Engines { engines: statuses() });
            }
        }
        Command::SignOut { .. } => {
            let events = events.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(600)).await;
                *codex().lock().unwrap() = EngineState::SignedOut;
                let _ = events.send(Event::Engines { engines: statuses() });
            });
        }
        _ => return false,
    }
    true
}
