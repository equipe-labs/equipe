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
/// folders under `home`, if any.
pub fn find_cli(home: &Path, path: Option<&OsStr>) -> Option<PathBuf> {
    let on_path = path.into_iter().flat_map(std::env::split_paths);
    let usual = [home.join(".local/bin"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin"), home.join(".cargo/bin")];
    on_path.chain(usual).map(|dir| dir.join("equipe")).find(|bin| bin.is_file())
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
}
