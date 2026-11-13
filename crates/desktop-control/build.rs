//! The helper's revision: a hash of this crate's sources. A desktop helper
//! bundle (the copy of `equipe` that holds the permissions when equipe.app
//! isn't installed) is rebuilt only when its revision changes — not on every
//! update of equipe — since replacing an app bundle is what macOS's App
//! Management guards.

use std::path::{Path, PathBuf};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let src = root.join("src");
    println!("cargo:rerun-if-changed=src");
    let mut all = Vec::new();
    files(&src, &mut all);
    all.sort();
    // FNV-1a over each file's path (relative) and contents.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for path in &all {
        eat(path.strip_prefix(&src).unwrap_or(path).to_string_lossy().replace('\\', "/").as_bytes());
        eat(&std::fs::read(path).unwrap_or_default());
    }
    println!("cargo:rustc-env=EQUIPE_HELPER_REVISION={hash:016x}");
}
