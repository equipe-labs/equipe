//! Whether this Mac has the `equipe` command the app is a window onto. Opened
//! from Finder the app gets no shell PATH, so it also looks where install.sh,
//! Homebrew and cargo put it; when none has it, the window shows how to install.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The one line that installs equipe (README "Install").
pub const INSTALL_COMMAND: &str = "curl -fsSL https://github.com/equipe-labs/equipe/releases/latest/download/install.sh | sh";
/// Where the install page is.
pub const SITE: &str = "https://equipe.im";

/// The `equipe` binary on `path` (a PATH value) or in the usual install
/// folders under `home`, if any — else the one the app carries.
pub fn find_cli(home: &Path, path: Option<&OsStr>) -> Option<PathBuf> {
    let on_path = path.into_iter().flat_map(std::env::split_paths);
    let usual = [home.join(".local/bin"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin"), home.join(".cargo/bin")];
    on_path.chain(usual).map(|dir| dir.join("equipe")).find(|bin| bin.is_file()).or_else(bundled_cli)
}

/// The `equipe` this app carries (`equipe.app/Contents/Helpers/equipe`), the
/// same version as the app: what runs the daemon when the Mac has no other.
/// Not in `Contents/MacOS`, where an `equipe` is the desktop helper's copy.
pub fn bundled_cli() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.parent()?.join("Helpers").join("equipe");
    bin.is_file().then_some(bin)
}

/// The daemon off on this Mac: started with the `equipe` there is — the app's
/// own when the Mac has none — once per launch. The first time, from an app
/// that stays where it is, as a login service (`equipe daemon install`): up
/// again after a restart, with no app open. Opened from Finder, nobody has to
/// install equipe in a terminal first. Blocks: call it off the UI thread.
#[cfg_attr(test, allow(dead_code))]
pub fn wake_daemon() -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    static TRIED: AtomicBool = AtomicBool::new(false);
    if TRIED.swap(true, Ordering::SeqCst) {
        return false;
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return false };
    let Some(cli) = find_cli(&home, std::env::var_os("PATH").as_deref()) else { return false };
    let daemon = |action: &str| {
        std::process::Command::new(&cli)
            .args(["daemon", action])
            .env("EQUIPE_NO_OPEN", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    let as_service = installs_service(&cli, home.join("Library/LaunchAgents/im.equipe.daemon.plist").is_file(), std::env::var_os("EQUIPE_HOME").is_some());
    let started = (as_service && daemon("install")) || daemon("start");
    if Some(&cli) == bundled_cli().as_ref() {
        link_cli(&home, &cli);
    }
    started
}

/// Whether the daemon starts as a login service: not one yet, equipe's usual
/// home (not a test's `EQUIPE_HOME`), and an `equipe` that stays where it is —
/// never one on the disk image or in the copy macOS runs a downloaded app
/// from before it's moved (App Translocation), gone once the app is.
pub fn installs_service(cli: &Path, service_there: bool, home_set: bool) -> bool {
    let text = cli.to_string_lossy();
    !service_there && !home_set && !text.starts_with("/Volumes/") && !text.contains("/AppTranslocation/")
}

/// `~/.local/bin/equipe`, pointing at the app's own: `equipe` in a terminal
/// too, kept current by the app's updates. Never over a real file there (an
/// install.sh copy); a link left by an older place of the app is renewed.
pub fn link_cli(home: &Path, cli: &Path) {
    let dir = home.join(".local/bin");
    let link = dir.join("equipe");
    match std::fs::symlink_metadata(&link) {
        Ok(meta) if !meta.file_type().is_symlink() => return,
        Ok(_) => {
            if std::fs::read_link(&link).is_ok_and(|to| to == cli) {
                return;
            }
            let _ = std::fs::remove_file(&link);
        }
        Err(_) => {}
    }
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(cli, &link);
}

/// Whether this Mac can run git, which every squad keeps its work in. A new
/// Mac's `/usr/bin/git` is only a stub that asks to install Apple's developer
/// tools: it counts only once `xcode-select -p` finds them (asking that shows
/// no dialog); a git elsewhere on `path` (Homebrew's) counts as it is.
pub fn git_ready(path: Option<&OsStr>) -> bool {
    let elsewhere = path.into_iter().flat_map(std::env::split_paths).filter(|dir| dir != Path::new("/usr/bin")).any(|dir| dir.join("git").is_file());
    elsewhere || !cfg!(target_os = "macos") || std::process::Command::new("/usr/bin/xcode-select").arg("-p").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Ask macOS to install its developer tools (git with them): its own dialog
/// opens, and the install runs on its own. `false` when it couldn't be asked.
pub fn install_git() -> bool {
    std::process::Command::new("/usr/bin/xcode-select").arg("--install").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok()
}

/// Whether equipe is installed for the person running the app.
pub fn cli_installed() -> bool {
    let Some(home) = std::env::var_os("HOME") else { return true };
    find_cli(Path::new(&home), std::env::var_os("PATH").as_deref()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-desktop-install-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_it_on_path_or_where_install_sh_puts_it() {
        let home = temp("home");
        assert_eq!(find_cli(&home, Some(OsStr::new(""))).filter(|p| p.starts_with(&home)), None);

        let local = home.join(".local/bin");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join("equipe"), "").unwrap();
        assert_eq!(find_cli(&home, None), Some(local.join("equipe")));

        let elsewhere = temp("path");
        std::fs::write(elsewhere.join("equipe"), "").unwrap();
        let path = std::env::join_paths([&elsewhere]).unwrap();
        assert_eq!(find_cli(&home, Some(&path)), Some(elsewhere.join("equipe")));
    }

    #[test]
    fn the_daemon_becomes_a_login_service_once_from_an_app_that_stays() {
        let app = Path::new("/Applications/equipe.app/Contents/Helpers/equipe");
        assert!(installs_service(app, false, false));
        assert!(!installs_service(app, true, false), "once");
        assert!(!installs_service(app, false, true), "a test's EQUIPE_HOME never gets the person's service");
        assert!(!installs_service(Path::new("/Volumes/equipe/equipe.app/Contents/Helpers/equipe"), false, false), "the disk image");
        let moved = Path::new("/private/var/folders/x/T/AppTranslocation/ABC/d/equipe.app/Contents/Helpers/equipe");
        assert!(!installs_service(moved, false, false), "App Translocation");
    }

    #[test]
    fn a_git_beside_apples_stub_is_ready_as_it_is() {
        let brew = temp("brew");
        std::fs::write(brew.join("git"), "").unwrap();
        let path = std::env::join_paths([PathBuf::from("/usr/bin"), brew]).unwrap();
        assert!(git_ready(Some(&path)));
    }

    #[test]
    fn a_folder_named_equipe_is_not_the_command() {
        let home = temp("dir");
        std::fs::create_dir_all(home.join(".local/bin/equipe")).unwrap();
        assert_eq!(find_cli(&home, Some(OsStr::new(""))).filter(|p| p.starts_with(&home)), None);
    }

    #[test]
    fn the_apps_equipe_is_linked_where_a_terminal_finds_it_but_never_over_an_installed_one() {
        let home = temp("link");
        let app = temp("app").join("equipe");
        std::fs::write(&app, "").unwrap();
        link_cli(&home, &app);
        assert_eq!(std::fs::read_link(home.join(".local/bin/equipe")).unwrap(), app);
        // An install.sh copy stays.
        let other = temp("installed");
        std::fs::create_dir_all(other.join(".local/bin")).unwrap();
        std::fs::write(other.join(".local/bin/equipe"), "real").unwrap();
        link_cli(&other, &app);
        assert_eq!(std::fs::read_to_string(other.join(".local/bin/equipe")).unwrap(), "real");
    }
}
