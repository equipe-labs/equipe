//! The one agent-desktop release equipe runs, and how it gets onto the machine.
//!
//! equipe never builds or vendors agent-desktop: `equipe desktop setup`
//! downloads the upstream release archive for this machine, checks its
//! SHA-256 against the value pinned here (taken from the release's
//! `checksums.txt`, whose archives carry a Sigstore build attestation), unpacks
//! it with the system's `tar`, checks every unpacked file's SHA-256 too, and
//! keeps it in the helper's private folder beside a copy of its Apache-2.0
//! license. The helper checks the binary's hash again before it first runs it,
//! and again whenever the file changes, so a swapped binary never runs.
//!
//! To move to a new release: update [`VERSION`] and every hash below from the
//! release's `checksums.txt` and its unpacked archives (`shasum -a 256`), check
//! `gh attestation verify <archive> --repo vercel-labs/agent-desktop`, and run
//! the engine tests. docs/DESKTOP-CONTROL.md has the steps.

use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

/// The agent-desktop release equipe runs.
pub const VERSION: &str = "1.2.0";

/// Where releases are downloaded from.
pub const RELEASES: &str = "https://github.com/vercel-labs/agent-desktop/releases/download";

/// agent-desktop's license (Apache-2.0), as published with [`VERSION`]: kept
/// beside the installed binary. THIRD_PARTY_NOTICES.md says the same.
pub const LICENSE: &str = include_str!("../../../../licenses/agent-desktop-Apache-2.0.txt");

/// One platform's release archive and what it unpacks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Archive {
    /// The Rust target triple in the archive's name.
    pub target: &'static str,
    /// SHA-256 of the archive, from the release's `checksums.txt`.
    pub sha256: &'static str,
    /// Every file in it, with its SHA-256. The first is the executable.
    pub files: &'static [(&'static str, &'static str)],
}

impl Archive {
    pub fn name(&self) -> String {
        format!("agent-desktop-v{VERSION}-{}.tar.gz", self.target)
    }

    pub fn url(&self) -> String {
        format!("{RELEASES}/v{VERSION}/{}", self.name())
    }

    /// The executable's file name.
    pub fn executable(&self) -> &'static str {
        self.files[0].0
    }

    pub fn executable_sha256(&self) -> &'static str {
        self.files[0].1
    }
}

/// The archives equipe knows, by target. macOS and Windows: where the desktop
/// helper runs. (agent-desktop also ships Linux; equipe's helper doesn't run there yet.)
pub const ARCHIVES: &[Archive] = &[
    Archive {
        target: "aarch64-apple-darwin",
        sha256: "219813428bb1ad8decae172faed05c5964a2415170df402e2786ccfa6452ba82",
        files: &[
            ("agent-desktop", "dec75e0bba0d1f9b589bdb88188770ad1a199cde9e02cf0a5b94175491f738e9"),
            ("agent-desktop-macos-helper", "4a795a40ba7fc93b726b0834342f0225779c30d83e28f1f219db479860c38a78"),
        ],
    },
    Archive {
        target: "x86_64-apple-darwin",
        sha256: "d9dc50226f28d7fc86a2f5b0945f0c206dff6c31e418907749f493b6f9e773e6",
        files: &[
            ("agent-desktop", "5dae755fee4af37d1a245f058139b262d358352cce15aeb49d0100ff46b12156"),
            ("agent-desktop-macos-helper", "bdf060658b93f326efa06ac24d521e88025bf88109cecc257fa7fa40e65801cd"),
        ],
    },
    Archive {
        target: "x86_64-pc-windows-msvc",
        sha256: "a4199f741a441cedd349bf56c5eba01783b1261258b498efeb7ec9945220c3e8",
        files: &[("agent-desktop.exe", "d274667031322e50b9c2b1ab81c2ca46835d52cc932769a5c34bd4d481fd94c3")],
    },
    Archive {
        target: "aarch64-pc-windows-msvc",
        sha256: "d6d8643310c2c13eace2261444d9e4c0d16ca09581f38500983104903604caa6",
        files: &[("agent-desktop.exe", "f1577d9f8cb6c9b1e767584fed96c72fbe1591fefbd67100965607d1a337a2fb")],
    },
];

/// The archive for the machine this runs on, if equipe knows one.
pub fn this_machine() -> Option<&'static Archive> {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        _ => return None,
    };
    ARCHIVES.iter().find(|a| a.target == target)
}

/// Where the pinned release lives: `<home>/desktop/agent-desktop/<version>`,
/// inside the helper's folder, which members' sandboxed commands can't read or write.
pub fn install_dir(home: &Path) -> PathBuf {
    crate::helper::dir(home).join("agent-desktop").join(VERSION)
}

/// agent-desktop's state root (refs of each member's snapshots): private,
/// emptied whenever the helper starts.
pub fn state_dir(home: &Path) -> PathBuf {
    crate::helper::dir(home).join("agent-desktop").join("state")
}

/// The installed executable for `archive`.
pub fn executable(home: &Path, archive: &Archive) -> PathBuf {
    install_dir(home).join(archive.executable())
}

/// SHA-256 of a file, lower-case hex.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What is installed, as a check finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installed {
    /// No release known for this machine.
    Unsupported,
    /// Not downloaded yet.
    Missing,
    /// There, but its files don't hash to the pinned values: it won't run.
    Tampered(String),
    /// There and verified.
    Ready(PathBuf),
}

/// Check what is installed for `archive`, hashing every file.
pub fn check(home: &Path, archive: Option<&Archive>) -> Installed {
    let Some(archive) = archive else {
        return Installed::Unsupported;
    };
    let dir = install_dir(home);
    if !dir.join(archive.executable()).exists() {
        return Installed::Missing;
    }
    for (file, want) in archive.files {
        match sha256_file(&dir.join(file)) {
            Ok(got) if got == *want => {}
            Ok(_) => return Installed::Tampered(format!("{file} doesn't match agent-desktop {VERSION}'s published checksum")),
            Err(err) => return Installed::Tampered(format!("{file}: {err}")),
        }
    }
    Installed::Ready(dir.join(archive.executable()))
}

/// Install `archive` from its downloaded `bytes`: checks the archive's hash,
/// unpacks it with the system `tar` in a scratch folder beside the install,
/// checks every file's hash, then moves the folder into place. Nothing is
/// left behind when a check fails.
pub fn install_from_archive(home: &Path, archive: &Archive, bytes: &[u8]) -> Result<PathBuf, String> {
    let got = sha256_bytes(bytes);
    if got != archive.sha256 {
        return Err(format!(
            "The downloaded {} doesn't match agent-desktop {VERSION}'s published checksum (got {got}): nothing installed.",
            archive.name()
        ));
    }
    crate::helper::ensure_dir(home).map_err(|e| e.to_string())?;
    let parent = install_dir(home).parent().map(Path::to_path_buf).ok_or("no install folder")?;
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let scratch = parent.join(format!(".unpack-{}", crate::helper::random_hex(6).map_err(|e| e.to_string())?));
    let result = unpack_and_check(archive, bytes, &scratch).and_then(|()| {
        std::fs::write(scratch.join("LICENSE"), LICENSE).map_err(|e| e.to_string())?;
        let dir = install_dir(home);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&scratch, &dir).map_err(|e| e.to_string())?;
        Ok(dir.join(archive.executable()))
    });
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn unpack_and_check(archive: &Archive, bytes: &[u8], scratch: &Path) -> Result<(), String> {
    std::fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
    let tarball = scratch.join("archive.tar.gz");
    std::fs::write(&tarball, bytes).map_err(|e| e.to_string())?;
    // Only the files the pin names are taken out of the archive.
    let mut tar = std::process::Command::new("tar");
    tar.arg("-xzf").arg(&tarball).arg("-C").arg(scratch);
    for (file, _) in archive.files {
        tar.arg(file);
    }
    let status = tar.status().map_err(|e| format!("couldn't run tar: {e}"))?;
    let _ = std::fs::remove_file(&tarball);
    if !status.success() {
        return Err(format!("tar couldn't unpack {}", archive.name()));
    }
    for (file, want) in archive.files {
        let path = scratch.join(file);
        let meta = std::fs::symlink_metadata(&path).map_err(|_| format!("{} has no {file}", archive.name()))?;
        if !meta.file_type().is_file() {
            return Err(format!("{file} in {} isn't a plain file", archive.name()));
        }
        let got = sha256_file(&path).map_err(|e| e.to_string())?;
        if got != *want {
            return Err(format!("{file} in {} doesn't match its pinned checksum: nothing installed.", archive.name()));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pin_is_a_sha256_and_names_its_executable_first() {
        for archive in ARCHIVES {
            assert_eq!(archive.sha256.len(), 64, "{}", archive.target);
            assert!(archive.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
            assert!(archive.executable().starts_with("agent-desktop"));
            for (_, sha) in archive.files {
                assert_eq!(sha.len(), 64);
            }
            assert_eq!(
                archive.url(),
                format!("https://github.com/vercel-labs/agent-desktop/releases/download/v{VERSION}/agent-desktop-v{VERSION}-{}.tar.gz", archive.target)
            );
        }
        assert!(LICENSE.contains("Apache License") && LICENSE.contains("Version 2.0"));
    }

    #[test]
    fn a_download_that_doesnt_match_its_pin_installs_nothing() {
        let home = std::env::temp_dir().join(format!("eq-pin-{}", std::process::id()));
        let archive = &ARCHIVES[0];
        let refused = install_from_archive(&home, archive, b"not the release").unwrap_err();
        assert!(refused.contains("doesn't match"), "{refused}");
        assert_eq!(check(&home, Some(archive)), Installed::Missing);
        assert_eq!(check(&home, None), Installed::Unsupported);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn an_installed_binary_that_changed_is_found_out() {
        let home = std::env::temp_dir().join(format!("eq-pin-t-{}", std::process::id()));
        let fake = Archive { target: "test", sha256: "", files: &[("agent-desktop", "00")] };
        let dir = install_dir(&home);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("agent-desktop"), b"#!/bin/sh\n").unwrap();
        assert!(matches!(check(&home, Some(&fake)), Installed::Tampered(_)));
        let _ = std::fs::remove_dir_all(&home);
    }
}
