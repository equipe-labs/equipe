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
/// own when the Mac has none — as `equipe daemon start` does (a login service
/// from then on), once per launch. Opened from Finder, nobody has to install
/// equipe in a terminal first. Blocks: call it off the UI thread.
#[cfg_attr(test, allow(dead_code))]
pub fn wake_daemon() -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    static TRIED: AtomicBool = AtomicBool::new(false);
    if TRIED.swap(true, Ordering::SeqCst) {
        return false;
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return false };
    let Some(cli) = find_cli(&home, std::env::var_os("PATH").as_deref()) else { return false };
    let started = std::process::Command::new(&cli)
        .args(["daemon", "start"])
        .env("EQUIPE_NO_OPEN", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if Some(&cli) == bundled_cli().as_ref() {
        link_cli(&home, &cli);
    }
    started
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
