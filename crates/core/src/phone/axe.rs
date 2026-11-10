//! AXe (<https://github.com/cameroncooke/AXe>, MIT): how members read and touch
//! the iOS Simulator.
//!
//! `simctl` boots, installs, launches, screenshots and records, but it can't
//! tap or read what is on the screen. Of the ways to do that, AXe needs the
//! least: one self-contained download (about 8 MB: the `axe` program and the
//! simulator frameworks of Meta's idb it is built on), no root, no Python, no
//! companion daemon, no Accessibility or Screen Recording permission, no
//! Simulator.app window — it talks to the simulator through Xcode's own
//! private frameworks. idb needs a Python client and a companion process;
//! WebDriverAgent needs an XCUITest runner built with `xcodebuild` (minutes,
//! and DerivedData on disk) for every Xcode.
//!
//! equipe never installs it unasked: the first member that needs it asks the
//! person on a card (once; a "no" holds until the daemon restarts), then
//! equipe downloads the pinned release, checks its SHA-256 and unpacks it
//! under `~/.equipe/tools/axe/<version>/`. An `axe` the person installed
//! (Homebrew) is used as it is.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::exec::Exec;

pub const VERSION: &str = "1.8.0";
pub const URL: &str = "https://github.com/cameroncooke/AXe/releases/download/v1.8.0/AXe-macOS-v1.8.0-universal.tar.gz";
pub const SHA256: &str = "7b76340b72e90d0f211bc7c4636f15009076eff07acef2f2b632b175debd8834";
/// What the person is told it weighs.
pub const SIZE_MB: u32 = 8;

/// Where equipe keeps its copy.
pub fn dir(equipe_home: &Path) -> PathBuf {
    equipe_home.join("tools").join("axe").join(VERSION)
}

/// The `axe` to run: equipe's copy, else one the person installed.
pub fn find(equipe_home: &Path) -> Option<PathBuf> {
    let ours = dir(equipe_home).join("axe");
    if ours.is_file() {
        return Some(ours);
    }
    ["/opt/homebrew/bin/axe", "/usr/local/bin/axe"].iter().map(PathBuf::from).find(|p| p.is_file())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Download, check and unpack the pinned release: the `axe` to run.
/// Unpacked beside its place and moved in whole, so a half-done install is never used.
pub fn install(exec: &dyn Exec, equipe_home: &Path) -> Result<PathBuf, String> {
    let dest = dir(equipe_home);
    if dest.join("axe").is_file() {
        return Ok(dest.join("axe"));
    }
    let parent = dest.parent().ok_or("no folder for tools")?.to_owned();
    std::fs::create_dir_all(&parent).map_err(|e| format!("{} couldn't be made: {e}", parent.display()))?;
    let staging = parent.join(format!(".{VERSION}.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let archive = staging.join("axe.tar.gz");
    let result = (|| {
        let got = exec.run(
            "curl",
            &["-fsSL", "--retry", "2", "--max-time", "300", "-o", &archive.to_string_lossy(), URL].iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
            None,
            Duration::from_secs(330),
        );
        if !got.ok {
            return Err(format!("AXe couldn't be downloaded: {}", got.why()));
        }
        let bytes = std::fs::read(&archive).map_err(|e| e.to_string())?;
        let sum = sha256_hex(&bytes);
        if sum != SHA256 {
            return Err(format!("AXe's download isn't the release equipe expects (SHA-256 {sum}): nothing was installed."));
        }
        let unpacked = staging.join("unpacked");
        std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
        let out = exec.run("tar", &["-xzf".into(), archive.to_string_lossy().into_owned(), "-C".into(), unpacked.to_string_lossy().into_owned()], None, Duration::from_secs(120));
        if !out.ok || !unpacked.join("axe").is_file() {
            return Err(format!("AXe's archive couldn't be unpacked: {}", out.why()));
        }
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&unpacked, &dest).map_err(|e| format!("AXe couldn't be put in place: {e}"))?;
        Ok(dest.join("axe"))
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phone::exec::fake::Fake;

    #[test]
    fn a_download_that_isn_t_the_pinned_release_is_never_installed() {
        let home = std::env::temp_dir().join(format!("equipe-axe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        // The fake curl writes nothing; the archive is then missing or wrong.
        let fake = Fake::default();
        let err = install(&fake, &home).unwrap_err();
        assert!(err.contains("isn't the release") || err.contains("No such file") || err.contains("os error"), "{err}");
        assert!(!dir(&home).exists(), "nothing left in place");
        assert!(fake.called("curl -fsSL"), "{:?}", fake.calls());
        assert!(!fake.called("tar"), "never unpacked unchecked");
        let leftovers: Vec<_> = std::fs::read_dir(dir(&home).parent().unwrap()).unwrap().flatten().collect();
        assert!(leftovers.is_empty(), "the staging folder goes too");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_hash_is_sha256_in_hex() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(SHA256.len(), 64);
        assert!(URL.contains(VERSION));
    }
}
