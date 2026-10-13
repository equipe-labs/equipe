//! `equipe desktop`: the helper that lets members work this Mac's desktop apps
//! — installing it, checking what it needs, and who may use it.

use std::path::{Path, PathBuf};

use equipe_desktop_control::helper;
use equipe_desktop_control::install::{self, Signer};
use serde_json::Value;

use crate::ui;

pub enum Action {
    Setup { sign: Option<String> },
    Status,
    On { member: String, experimental: bool, squad: Option<String> },
    Off { member: String, squad: Option<String> },
    Remove,
    Serve { home: PathBuf },
}

/// The daemon's login service label (`equipe daemon install`).
const DAEMON_LABEL: &str = "im.equipe.daemon";

const SCREEN_PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
const ACCESSIBILITY_PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

/// Whether this process is the helper's copy of equipe, inside its bundle:
/// that copy holds the person's grants and runs nothing but the helper.
pub fn is_helper_copy() -> bool {
    std::env::current_exe().is_ok_and(|exe| exe.ends_with(Path::new("Contents/MacOS").join(install::EXECUTABLE)))
}

pub async fn run(action: Option<Action>) -> Result<(), String> {
    let home = equipe_providers::credentials::equipe_home();
    match action.unwrap_or(Action::Status) {
        Action::Serve { home } => equipe_desktop_control::serve(&home).map_err(|e| e.to_string()),
        Action::On { member, experimental, squad } => {
            if !experimental {
                return Err(
                    "Desktop control is experimental: a member clicks and types on this Mac as you, with no screen lease or send approval yet. Add --experimental to turn it on anyway."
                        .into(),
                );
            }
            set(&home, &member, true, squad)
        }
        Action::Off { member, squad } => set(&home, &member, false, squad),
        Action::Remove => {
            mac_only()?;
            let had = install::uninstall(&home);
            let p = ui::out();
            println!("{}", p.ok_line(if had { "The desktop helper is stopped and removed." } else { "No desktop helper was installed." }));
            println!("{}", p.note(2, "To take its permissions back too: System Settings › Privacy & Security › Screen & System Audio Recording and Accessibility, remove \"equipe Desktop Helper\" with −."));
            Ok(())
        }
        Action::Status => {
            mac_only()?;
            let p = ui::out();
            println!("{}\n", p.kicker("desktop"));
            let ready = report(&home).await;
            if !ready {
                println!("\n  {}", p.hint("equipe desktop setup"));
            }
            Ok(())
        }
        Action::Setup { sign } => setup(&home, sign).await,
    }
}

fn mac_only() -> Result<(), String> {
    if cfg!(target_os = "macos") { Ok(()) } else { Err("Desktop control works on macOS only.".into()) }
}

fn set(home: &Path, member: &str, on: bool, squad: Option<String>) -> Result<(), String> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let squad = crate::home::squad_for(home, &cwd, squad).ok_or("No squad yet. Run equipe to start one.")?;
    let journal = equipe_core::journal::Journal::for_squad(home, &squad).map_err(|e| e.to_string())?;
    let team: Vec<String> = journal
        .open_sessions()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|s| s.member.map(|m| m.name))
        .collect();
    if !team.iter().any(|m| m == member) {
        return Err(format!("No one called {member} on {squad}. On the squad: {}.", team.join(", ")));
    }
    journal.set_desktop(member, on).map_err(|e| e.to_string())?;
    let p = ui::out();
    if on {
        println!("{}", p.ok_line(&format!("{member} works this Mac's desktop from its next turn (experimental).")));
        println!("{}", p.note(2, "It clicks and types as you. Each click is checked against the app it means, and typing stops when focus moves; leave the Mac alone while it works."));
        println!("  {}", p.hint(&format!("equipe desktop off {member}")));
    } else {
        println!("{}", p.ok_line(&format!("{member} no longer works the desktop, from its next turn.")));
    }
    Ok(())
}

/// The helper's own report, or `None` when it doesn't answer.
async fn helper_status(home: &Path, ask: &str) -> Option<Value> {
    let result = helper::call(home, "setup", ask, &Value::Null).await;
    if result.is_error {
        return None;
    }
    serde_json::from_str(&result.text).ok()
}

fn line(ok: bool, label: &str, detail: &str) {
    let p = ui::out();
    if ok {
        println!("  {}", p.ok(label));
    } else {
        println!("  {}{}", p.fail(label), if detail.is_empty() { String::new() } else { p.dim(&format!(" — {detail}")) });
    }
}

/// Print every check; `true` when the desktop is ready to use.
async fn report(home: &Path) -> bool {
    let gui = install::gui_session();
    line(gui, "Someone is logged in at the Mac's screen", "log in at the Mac (or over Screen Sharing / Jump Desktop): the helper only runs in that session");
    line(
        install::daemon_in_aqua(DAEMON_LABEL),
        "The daemon runs as a login service in that session",
        "equipe daemon install (while logged in at the Mac)",
    );
    let loaded = install::loaded();
    line(loaded, "The desktop helper is installed and running", "equipe desktop setup");
    let status = if loaded { helper_status(home, helper::STATUS).await } else { None };
    if loaded && status.is_none() {
        line(false, "The helper answers", &format!("see {}", helper::log_path(home).display()));
    }
    let Some(status) = status else { return false };
    let yes = |key: &str| status[key].as_bool().unwrap_or(false);
    line(yes("screen"), "Screen & System Audio Recording", "switch on \"equipe Desktop Helper\"");
    line(yes("accessibility"), "Accessibility", "switch on \"equipe Desktop Helper\" and confirm with your password or Touch ID");
    line(yes("post_events"), "Sending input (comes with Accessibility)", "switch Accessibility on, then run setup again");
    match status["display"].as_object() {
        Some(d) => {
            let (w, h, s) = (d["width"].as_f64().unwrap_or(0.0), d["height"].as_f64().unwrap_or(0.0), d["scale"].as_f64().unwrap_or(1.0));
            line(true, &format!("Display {w}×{h} pt at {s}×"), "");
        }
        None => line(false, "A display", "no screen attached: plug in a monitor or an HDMI dummy plug, or use a virtual display"),
    }
    if status["session"]["locked"].as_bool() == Some(true) {
        line(false, "The screen is unlocked", "the Mac is locked: members can't work until it is unlocked");
    }
    let members = desktop_members(home);
    let p = ui::out();
    if members.is_empty() {
        println!("\n{}", p.note(2, "No member works the desktop yet: equipe desktop on <member> --experimental"));
    } else {
        println!("\n{}", p.note(2, &format!("Working the desktop: {}", members.join(", "))));
    }
    gui && yes("screen") && yes("accessibility") && yes("post_events")
}

fn desktop_members(home: &Path) -> Vec<String> {
    equipe_core::squads::list(home)
        .into_iter()
        .filter_map(|s| equipe_core::journal::Journal::for_squad(home, &s.name).ok().map(|j| (s.name, j.desktop_members())))
        .flat_map(|(squad, members)| members.into_iter().map(move |m| format!("{squad}/{m}")))
        .collect()
}

async fn setup(home: &Path, sign: Option<String>) -> Result<(), String> {
    mac_only()?;
    let p = ui::out();
    println!("{}\n", p.kicker("desktop setup"));
    if !install::gui_session() {
        return Err("Nobody is logged in at this Mac's screen. Log in there (or over Screen Sharing / Jump Desktop), then run equipe desktop setup again.".into());
    }
    let signer = match sign.as_deref() {
        None => Signer::chosen(home),
        Some("self") => Signer::SelfSigned,
        Some(identity) => Signer::Identity(identity.to_owned()),
    };
    signer.remember(home).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let stamp = crate::daemon::build_id();
    let installed = install::install(home, &exe, &stamp, env!("CARGO_PKG_VERSION"), &signer)?;
    if installed.changed {
        println!("{}", p.ok_line(&format!("Installed the desktop helper, signed with {}.", signer.describe())));
    } else {
        println!("{}", p.ok_line("The desktop helper is up to date."));
    }
    println!("{}", p.note(2, &p.dim(&installed.requirement)));
    // The helper takes a moment to start listening.
    let mut status = None;
    for _ in 0..40 {
        status = helper_status(home, helper::STATUS).await;
        if status.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let Some(status) = status else {
        return Err(format!("The helper didn't start: see {}", helper::log_path(home).display()));
    };
    let granted = |s: &Value| ["screen", "accessibility", "post_events"].iter().all(|k| s[*k].as_bool() == Some(true));
    if !granted(&status) {
        // Asking puts the helper in both lists (and shows macOS's prompts).
        let _ = helper_status(home, helper::REQUEST).await;
        guide(&status);
        if ui::interactive() {
            loop {
                println!("\n  {}", p.dim("Press Enter once you've switched them on (q to stop here)…"));
                let mut answer = String::new();
                if std::io::stdin().read_line(&mut answer).is_err() || answer.trim().eq_ignore_ascii_case("q") {
                    break;
                }
                // A permission given while a process runs is seen by its next one.
                let _ = install::restart();
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                match helper_status(home, helper::STATUS).await {
                    Some(now) if granted(&now) => break,
                    Some(now) => guide(&now),
                    None => println!("{}", p.fail_line("The helper didn't come back after a restart.")),
                }
            }
        }
    }
    println!();
    let ready = report(home).await;
    if ready {
        println!("\n{}", p.ok_line("The desktop is ready."));
        Ok(())
    } else {
        Err("Not ready yet: switch on what's marked ✗, then run equipe desktop setup again.".into())
    }
}

/// Exactly what the person switches on, opening the panes for them.
fn guide(status: &Value) {
    let p = ui::out();
    let yes = |key: &str| status[key].as_bool() == Some(true);
    println!("\n{}", p.note(2, "macOS asks you — at the Mac's own screen — to allow \"equipe Desktop Helper\". Only it gets these permissions; equipe and its members never do."));
    let mut step = 1;
    if !yes("screen") {
        println!("{}", p.note(2, &format!("{step}. System Settings › Privacy & Security › Screen & System Audio Recording: switch on \"equipe Desktop Helper\". If macOS asks whether it may bypass the private window picker, choose Allow.")));
        let _ = std::process::Command::new("open").arg(SCREEN_PANE).status();
        step += 1;
    }
    if !yes("accessibility") || !yes("post_events") {
        println!("{}", p.note(2, &format!("{step}. System Settings › Privacy & Security › Accessibility: switch on \"equipe Desktop Helper\". The switch only takes once you confirm with your password or Touch ID.")));
        let _ = std::process::Command::new("open").arg(ACCESSIBILITY_PANE).status();
    }
    println!("{}", p.note(2, "Already listed and switched on, but still ✗? It was signed differently before: remove it with −, then run equipe desktop setup again."));
}
