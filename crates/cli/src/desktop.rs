//! `equipe desktop`: the helper that lets members work this Mac's desktop apps
//! — installing it, checking what it needs, and who may use it.

use std::path::{Path, PathBuf};

use equipe_core::t;
use equipe_desktop_control::engine::pin;
use equipe_desktop_control::helper;
use equipe_desktop_control::install::{self, Signer};
use serde_json::Value;

use crate::ui;

pub enum Action {
    Setup { sign: Option<String> },
    Status,
    On { member: Option<String>, everyone: bool, squad: Option<String> },
    Off { member: Option<String>, everyone: bool, squad: Option<String> },
    Locked { state: Option<String> },
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
        Action::On { member, everyone, squad } => set(&home, member, everyone, true, squad),
        Action::Off { member, everyone, squad } => set(&home, member, everyone, false, squad),
        Action::Locked { state } => locked(&home, state.as_deref()),
        Action::Remove => {
            supported()?;
            #[cfg(target_os = "linux")]
            {
                let had = equipe_desktop_control::linux::stop_helper(&home);
                let _ = std::fs::remove_file(helper::socket_path(&home));
                println!("{}", ui::out().ok_line(if had { t!("desktop.removed") } else { t!("desktop.not_installed") }));
                return Ok(());
            }
            #[allow(unreachable_code)]
            let name = install::host().name();
            let had = install::uninstall(&home);
            let p = ui::out();
            println!("{}", p.ok_line(if had { t!("desktop.removed") } else { t!("desktop.not_installed") }));
            println!("{}", p.note(2, &t!("desktop.remove_note", name = name)));
            Ok(())
        }
        Action::Status => {
            supported()?;
            let p = ui::out();
            println!("{}\n", p.kicker("desktop"));
            let ready = if LINUX { report_linux(&home).await } else { report(&home).await };
            if !ready {
                println!("\n  {}", p.hint("equipe desktop setup"));
            }
            Ok(())
        }
        Action::Setup { sign } => setup(&home, sign).await,
    }
}

/// macOS, Windows and Linux run the desktop helper (see docs/DESKTOP-CONTROL.md).
fn supported() -> Result<(), String> {
    if cfg!(any(target_os = "macos", windows, target_os = "linux")) {
        Ok(())
    } else {
        Err("Desktop control works on macOS, Windows and Linux only.".into())
    }
}

/// Linux has no permission to grant: its checks are the session's (a
/// display, the session bus, AT-SPI), and the helper runs inside it.
const LINUX: bool = cfg!(target_os = "linux");

/// A Mac's checks (a login session, the login service, the helper's bundle)
/// mean nothing on Windows, whose helper needs no permission.
const MAC: bool = cfg!(target_os = "macos");

/// `equipe desktop locked on|off`: the person's "Work while the screen is locked".
fn locked(home: &Path, state: Option<&str>) -> Result<(), String> {
    let p = ui::out();
    let on = match state.map(str::to_ascii_lowercase).as_deref() {
        None => equipe_core::desktop::work_locked(home),
        Some("on" | "true" | "yes") => true,
        Some("off" | "false" | "no") => false,
        Some(_) => return Err(t!("desktop.locked_bad").to_owned()),
    };
    if state.is_some() {
        equipe_core::desktop::set_work_locked(home, on).map_err(|e| e.to_string())?;
    }
    println!("{}", p.ok_line(if on { t!("desktop.locked_on") } else { t!("desktop.locked_off") }));
    if on {
        println!("{}", p.note(2, t!("desktop.locked_note")));
    }
    Ok(())
}

fn set(home: &Path, member: Option<String>, everyone: bool, on: bool, squad: Option<String>) -> Result<(), String> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let squad = crate::home::squad_for(home, &cwd, squad).ok_or("No squad yet. Run equipe to start one.")?;
    let journal = equipe_core::journal::Journal::for_squad(home, &squad).map_err(|e| e.to_string())?;
    let p = ui::out();
    if everyone {
        journal.set_desktop_everyone(on).map_err(|e| e.to_string())?;
        println!("{}", p.ok_line(&if on { t!("desktop.on_everyone", squad = squad) } else { t!("desktop.off_everyone", squad = squad) }));
        return Ok(());
    }
    let Some(member) = member else { return Err(t!("desktop.needs_member").to_owned()) };
    let member = member.as_str();
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
    if on {
        println!("{}", p.ok_line(&t!("desktop.on_member", member = member)));
        println!("{}", p.note(2, t!("desktop.on_note")));
        println!("  {}", p.hint(&format!("equipe desktop off {member}")));
    } else {
        println!("{}", p.ok_line(&t!("desktop.off_member", member = member)));
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

/// Print every check; `true` when the desktop is ready to use. Accessibility
/// is what members need (the tree, ref actions, input); Screen Recording is
/// only for screenshots and the picture shown when the person is asked.
async fn report(home: &Path) -> bool {
    let gui = !MAC || install::gui_session();
    if MAC {
        line(gui, t!("desktop.check_session"), t!("desktop.fix_session"));
        line(install::daemon_in_aqua(DAEMON_LABEL), t!("desktop.check_daemon"), t!("desktop.fix_daemon"));
    }
    engine_line(home);
    let loaded = !MAC || install::loaded();
    if MAC {
        line(loaded, t!("desktop.check_helper"), "equipe desktop setup");
    }
    let status = if loaded { helper_status(home, helper::STATUS).await } else { None };
    if loaded && status.is_none() {
        line(false, t!("desktop.check_answers"), &t!("desktop.fix_answers", path = helper::log_path(home).display()));
    }
    let Some(status) = status else { return false };
    let yes = |key: &str| status[key].as_bool().unwrap_or(false);
    let name = equipe_core::desktop::ready::helper_name(Some(&status));
    if MAC {
        host_lines(&status, &name);
    }
    line(yes("accessibility"), t!("desktop.check_accessibility"), &t!("desktop.fix_accessibility", name = name));
    line(yes("post_events"), t!("desktop.check_post_events"), t!("desktop.fix_post_events"));
    // The engine, run by the helper: proof that its grant reaches agent-desktop.
    if let Some(state) = status["engine"]["accessibility"].as_str() {
        line(state == "granted", t!("desktop.check_engine_grant"), &t!("desktop.fix_engine_grant", name = name));
    }
    optional(yes("screen"), t!("desktop.check_screen"), &t!("desktop.fix_screen", name = name));
    match status["display"].as_object() {
        Some(d) => {
            let (w, h, s) = (d["width"].as_f64().unwrap_or(0.0), d["height"].as_f64().unwrap_or(0.0), d["scale"].as_f64().unwrap_or(1.0));
            line(true, &t!("desktop.check_display", width = w, height = h, scale = s), "");
        }
        None => line(false, t!("desktop.check_no_display"), t!("desktop.fix_no_display")),
    }
    let work_locked = equipe_core::desktop::work_locked(home);
    if status["session"]["locked"].as_bool() == Some(true) {
        if work_locked {
            optional(false, t!("desktop.check_unlocked"), t!("desktop.fix_unlocked_working"));
        } else {
            line(false, t!("desktop.check_unlocked"), t!("desktop.fix_unlocked"));
        }
    }
    let p = ui::out();
    println!("\n{}", p.note(2, &members_line(home, yes("accessibility"))));
    println!("{}", p.note(2, if work_locked { t!("desktop.locked_on") } else { t!("desktop.locked_off_hint") }));
    gui && yes("accessibility") && yes("post_events")
}

/// Which process holds the permission, and — once equipe.app does — that the
/// older helper's row can go.
fn host_lines(status: &Value, name: &str) {
    let p = ui::out();
    match status["role"].as_str() {
        Some("app") => println!("  {}", p.ok(t!("desktop.check_host_app"))),
        Some("bundle") if status["handed_over"].as_bool() == Some(true) => {
            optional(false, t!("desktop.check_host_app"), &t!("desktop.fix_host_handed", name = name));
        }
        Some("bundle") => println!("  {}", p.ok(&t!("desktop.check_host_bundle", name = name))),
        _ => {}
    }
    if status["role"].as_str() == Some("app") && install::bundle_executable().is_some() {
        println!("{}", p.note(2, t!("desktop.old_row")));
    }
}

/// Who works the desktop, across squads: everyone by default once the switch is on.
fn members_line(home: &Path, granted: bool) -> String {
    let mut named = Vec::new();
    let mut off = Vec::new();
    let mut everyone_off = Vec::new();
    for squad in equipe_core::squads::list(home) {
        let Ok(j) = equipe_core::journal::Journal::for_squad(home, &squad.name) else { continue };
        named.extend(j.desktop_members().into_iter().map(|m| format!("{}/{m}", squad.name)));
        off.extend(j.desktop_off().into_iter().map(|m| format!("{}/{m}", squad.name)));
        if j.desktop_off_for_everyone() {
            everyone_off.push(squad.name.clone());
        }
    }
    off.extend(everyone_off.iter().map(|s| format!("{s}/*")));
    match (granted, off.is_empty()) {
        (false, _) if named.is_empty() => t!("desktop.members_until_granted").to_owned(),
        (false, _) => t!("desktop.members", names = named.join(", ")),
        (true, true) => t!("desktop.members_everyone").to_owned(),
        (true, false) => t!("desktop.members_except", names = off.join(", ")),
    }
}

/// A check that doesn't hold the desktop back when it fails.
fn optional(ok: bool, label: &str, detail: &str) {
    let p = ui::out();
    if ok {
        println!("  {}", p.ok(label));
    } else {
        println!("  {}{}", p.dim(&format!("○ {label}")), p.dim(&format!(" — {detail}")));
    }
}

/// The accessibility engine on disk: there, and the pinned release.
fn engine_line(home: &Path) {
    let version = pin::VERSION;
    match pin::check(home, pin::this_machine()) {
        pin::Installed::Ready(_) => line(true, &t!("desktop.check_engine", version = version), ""),
        pin::Installed::Missing => line(false, &t!("desktop.check_engine_missing", version = version), t!("desktop.fix_engine_missing")),
        pin::Installed::Tampered(_) => line(false, &t!("desktop.check_engine_missing", version = version), t!("desktop.fix_engine_tampered")),
        pin::Installed::Unsupported => {}
    }
}

/// Download the pinned agent-desktop release when it isn't installed (or
/// doesn't verify), check it against its pinned checksums, keep it in the
/// helper's folder. The daemon does the same by itself in the background; this
/// is the same step, now. A failure leaves the screenshot tools working.
async fn install_engine(home: &Path) {
    let p = ui::out();
    if pin::this_machine().is_none() {
        return;
    }
    let version = pin::VERSION;
    if matches!(pin::check(home, pin::this_machine()), pin::Installed::Ready(_)) {
        println!("{}", p.ok_line(&t!("desktop.engine_ready", version = version)));
        return;
    }
    println!("{}", p.note(2, &t!("desktop.engine_installing", version = version)));
    match equipe_core::desktop::auto::install_engine(home).await {
        Ok(()) => println!("{}", p.ok_line(&t!("desktop.engine_installed", version = version))),
        Err(why) => println!("{}", p.fail_line(&t!("desktop.engine_failed", why = why.detail))),
    }
}

async fn setup(home: &Path, sign: Option<String>) -> Result<(), String> {
    supported()?;
    let p = ui::out();
    println!("{}\n", p.kicker("desktop setup"));
    println!("{}\n", p.note(2, t!("desktop.auto_note")));
    if LINUX {
        return setup_linux(home).await;
    }
    if !MAC {
        return setup_windows(home).await;
    }
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
    let installed = install::install(home, &exe, env!("CARGO_PKG_VERSION"), &signer).map_err(|why| match why.strip_prefix(install::SIGNING_FAILED) {
        Some(detail) => t!("desktop.signing_failed", detail = detail.trim()),
        None => why,
    })?;
    match (&installed.host, installed.changed) {
        (install::Host::App(_), _) => println!("{}", p.ok_line(t!("desktop.installed_app"))),
        (install::Host::Bundle, true) => println!("{}", p.ok_line(&format!("Installed the desktop helper, signed with {}.", signer.describe()))),
        (install::Host::Bundle, false) => println!("{}", p.ok_line("The desktop helper is up to date.")),
    }
    println!("{}", p.note(2, &p.dim(&installed.requirement)));
    // The helper looks for the engine at each call: no restart needed.
    install_engine(home).await;
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
    // Accessibility is what members need; Screen Recording is asked for
    // too, but only screenshots wait for it.
    let granted = |s: &Value| ["accessibility", "post_events"].iter().all(|k| s[*k].as_bool() == Some(true));
    if !granted(&status) {
        // Asking puts the helper in both lists (and shows macOS's prompts).
        let _ = helper_status(home, helper::REQUEST).await;
        guide(&status, &equipe_core::desktop::ready::helper_name(Some(&status)));
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
                    Some(now) => guide(&now, &equipe_core::desktop::ready::helper_name(Some(&now))),
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

/// Windows needs no permission: the engine, then the helper, started if it isn't running.
async fn setup_windows(home: &Path) -> Result<(), String> {
    use equipe_core::desktop::auto::{Ops as _, ThisMachine};
    let p = ui::out();
    install_engine(home).await;
    let machine = ThisMachine { home: home.to_owned() };
    let answering = helper_status(home, helper::STATUS).await.is_some();
    if !answering {
        machine.sync_helper(false).await.map_err(|f| f.detail)?;
        for _ in 0..40 {
            if helper_status(home, helper::STATUS).await.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
    println!();
    if report(home).await {
        println!("\n{}", p.ok_line("The desktop is ready."));
        Ok(())
    } else {
        Err(format!("Not ready yet: see what's marked ✗ (the helper's log: {}).", helper::log_path(home).display()))
    }
}

/// The session as this terminal sees it: display, bus, AT-SPI. A terminal
/// inside the desktop session sees what the helper will.
#[cfg(target_os = "linux")]
async fn linux_session() -> equipe_desktop_control::linux::Readiness {
    tokio::task::spawn_blocking(equipe_desktop_control::linux::check).await.unwrap_or_else(|e| equipe_desktop_control::linux::Readiness {
        display: Err(e.to_string()),
        bus: Err(e.to_string()),
        atspi: Err(e.to_string()),
        toolkits: None,
    })
}

/// Print Linux's checks; `true` when the desktop is ready to use: the
/// session reachable, AT-SPI answering, the engine installed, and the
/// helper (inside the session) able to read apps through it.
#[cfg(target_os = "linux")]
async fn report_linux(home: &Path) -> bool {
    let p = ui::out();
    let session = linux_session().await;
    let err = |r: Result<(), &String>| r.err().cloned().unwrap_or_default();
    line(session.display.is_ok(), t!("desktop.check_linux_display"), &err(session.display.as_ref().map(drop)));
    line(session.bus.is_ok(), t!("desktop.check_linux_bus"), &err(session.bus.as_ref().map(drop)));
    line(session.atspi.is_ok(), t!("desktop.check_linux_atspi"), &err(session.atspi.as_ref().map(drop)));
    if session.atspi.is_ok()
        && let Some(on) = session.toolkits
    {
        optional(on, t!("desktop.check_linux_toolkits"), t!("desktop.fix_linux_toolkits"));
    }
    engine_line(home);
    let status = helper_status(home, helper::STATUS).await;
    let engine_ready = matches!(pin::check(home, pin::this_machine()), pin::Installed::Ready(_));
    let Some(status) = status else {
        if session.ok() {
            line(false, t!("desktop.check_answers"), &t!("desktop.fix_answers", path = helper::log_path(home).display()));
        }
        return false;
    };
    let helper_ok = status["accessibility"].as_bool() == Some(true);
    let missing: Vec<&str> = status["missing"].as_array().map(|m| m.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    line(helper_ok, t!("desktop.check_linux_helper"), &missing.join(" "));
    let engine_grant = status["engine"]["accessibility"].as_str();
    if let Some(state) = engine_grant {
        line(state == "granted", t!("desktop.check_linux_engine"), t!("desktop.fix_linux_engine"));
    }
    let work_locked = equipe_core::desktop::work_locked(home);
    if status["session"]["locked"].as_bool() == Some(true) {
        if work_locked {
            optional(false, t!("desktop.check_unlocked"), t!("desktop.fix_unlocked_working"));
        } else {
            line(false, t!("desktop.check_unlocked"), t!("desktop.fix_unlocked"));
        }
    }
    println!("\n{}", p.note(2, &members_line(home, helper_ok)));
    println!("{}", p.note(2, t!("desktop.linux_tree_only")));
    println!("{}", p.note(2, if work_locked { t!("desktop.locked_on") } else { t!("desktop.locked_off_hint") }));
    helper_ok && engine_ready && engine_grant != Some("denied")
}

#[cfg(not(target_os = "linux"))]
async fn report_linux(home: &Path) -> bool {
    report(home).await
}

/// Linux: the engine, then the helper started inside this terminal's
/// session — or, without one, exactly what the session lacks.
#[cfg(target_os = "linux")]
async fn setup_linux(home: &Path) -> Result<(), String> {
    use equipe_core::desktop::auto::{Ops as _, ThisMachine};
    let p = ui::out();
    install_engine(home).await;
    let session = linux_session().await;
    if session.session() {
        let machine = ThisMachine { home: home.to_owned() };
        let status = helper_status(home, helper::STATUS).await;
        // A helper left running from an earlier session (or an older equipe)
        // doesn't see this one's AT-SPI: started again, inside this session.
        let stale = status.as_ref().is_some_and(|s| s["accessibility"].as_bool() != Some(true) && session.ok());
        if stale && equipe_desktop_control::linux::stop_helper(home) {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        if status.is_none() || stale {
            machine.sync_helper(false).await.map_err(|f| f.detail)?;
            for _ in 0..40 {
                if helper_status(home, helper::STATUS).await.is_some() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        }
    }
    println!();
    if report_linux(home).await {
        println!("\n{}", p.ok_line("The desktop is ready."));
        Ok(())
    } else {
        Err(t!("desktop.linux_not_ready").to_owned())
    }
}

#[cfg(not(target_os = "linux"))]
async fn setup_linux(home: &Path) -> Result<(), String> {
    setup_windows(home).await
}

/// Exactly what the person switches on, opening the panes for them.
fn guide(status: &Value, name: &str) {
    let p = ui::out();
    let yes = |key: &str| status[key].as_bool() == Some(true);
    println!("\n{}", p.note(2, &t!("desktop.guide_intro", name = name)));
    let mut step = 1;
    if !yes("accessibility") || !yes("post_events") {
        println!("{}", p.note(2, &t!("desktop.guide_accessibility", step = step, name = name)));
        let _ = equipe_providers::platform::open_status(ACCESSIBILITY_PANE);
        step += 1;
    }
    if !yes("screen") {
        println!("{}", p.note(2, &t!("desktop.guide_screen", step = step, name = name)));
        let _ = equipe_providers::platform::open_status(SCREEN_PANE);
    }
    println!("{}", p.note(2, t!("desktop.guide_signed")));
}
