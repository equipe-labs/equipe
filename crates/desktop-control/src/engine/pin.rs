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

/// The archives equipe knows, by target: macOS, Windows and Linux (glibc),
/// each on arm64 and x86_64. A Linux archive also carries
/// `agent-desktop-linux-clipboard-owner`, which agent-desktop runs to keep
/// the X11 clipboard after setting it.
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
    Archive {
        target: "x86_64-unknown-linux-gnu",
        sha256: "591ae7f4a38479dc045df97e27447c0c8b530441730ab6b990e27c813162af44",
        files: &[
            ("agent-desktop", "da52a1eb2152cdc21b756ab20e6208d242da8062c73cd20bc8a8ae2a0384392c"),
            ("agent-desktop-linux-clipboard-owner", "3e96f92b805c0d1f9de309935d62f4189b8e11e1385ad022bb878487bb3942ec"),
        ],
    },
    Archive {
        target: "aarch64-unknown-linux-gnu",
        sha256: "68e8ebbb4f80e44485d2f1cf972943a38ee43108e2e0ce56bdfa641fd1599f68",
        files: &[
            ("agent-desktop", "915de59a8b3d22c9ad33023740bbcc5fe0b4fa830de9587bbf761237eafeb76f"),
            ("agent-desktop-linux-clipboard-owner", "a6bfde6778dadcf9fd04ba94de17cdf205eb0b23b643c3f34073d264d2117cc3"),
        ],
    },
];

/// The archive for the machine this runs on, if equipe knows one.
pub fn this_machine() -> Option<&'static Archive> {
    for_machine(std::env::consts::OS, std::env::consts::ARCH, cfg!(target_env = "gnu"))
}

/// The archive for an `os` and `arch` (as `std::env::consts` names them).
/// Linux releases are built against glibc: a musl build of equipe (Alpine)
/// gets none.
pub fn for_machine(os: &str, arch: &str, glibc: bool) -> Option<&'static Archive> {
    let target = match (os, arch) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("linux", "x86_64") if glibc => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") if glibc => "aarch64-unknown-linux-gnu",
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
    fn linux_is_pinned_on_both_architectures_with_its_clipboard_owner() {
        for (arch, archive_sha) in [
            ("x86_64", "591ae7f4a38479dc045df97e27447c0c8b530441730ab6b990e27c813162af44"),
            ("aarch64", "68e8ebbb4f80e44485d2f1cf972943a38ee43108e2e0ce56bdfa641fd1599f68"),
        ] {
            let archive = for_machine("linux", arch, true).unwrap_or_else(|| panic!("no Linux {arch} pin"));
            assert_eq!(archive.target, format!("{arch}-unknown-linux-gnu"));
            assert_eq!(archive.sha256, archive_sha, "the release's checksums.txt");
            assert_eq!(archive.name(), format!("agent-desktop-v1.2.0-{arch}-unknown-linux-gnu.tar.gz"));
            let names: Vec<_> = archive.files.iter().map(|(name, _)| *name).collect();
            assert_eq!(names, ["agent-desktop", "agent-desktop-linux-clipboard-owner"], "the executable first");
            assert_ne!(archive.files[0].1, archive.files[1].1);
        }
        assert_ne!(for_machine("linux", "x86_64", true).map(|a| a.files), for_machine("linux", "aarch64", true).map(|a| a.files));
        assert_eq!(for_machine("linux", "x86_64", false), None, "glibc builds only: none for musl");
        assert_eq!(for_machine("linux", "riscv64", true), None);
        assert_eq!(for_machine("freebsd", "x86_64", true), None);
        assert_eq!(for_machine("macos", "aarch64", false).map(|a| a.target), Some("aarch64-apple-darwin"));
        assert_eq!(for_machine("windows", "x86_64", false).map(|a| a.target), Some("x86_64-pc-windows-msvc"));
        let targets: std::collections::BTreeSet<_> = ARCHIVES.iter().map(|a| a.target).collect();
        assert_eq!(targets.len(), ARCHIVES.len(), "one pin per target");
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
