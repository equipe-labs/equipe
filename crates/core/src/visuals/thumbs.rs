//! Scaled copies of members' pictures, kept on disk so a screen's thumbnail
//! is a file read rather than a decode and an encode.
//!
//! A decision card asks for a dozen pictures at once, each a render of a few
//! megabytes; decoding and scaling each took a second or more on a busy Mac,
//! and the card showed placeholders meanwhile. A copy is kept under
//! `<home>/cache/thumbs/` by the picture's real path, its size and modified
//! time, and the side asked for — so a picture changed in place is scaled
//! afresh, and nothing kept is ever given for another file. Only what
//! [`super::serve_with`] already allowed is read through here; the cache
//! decides nothing about what a screen may see.
//!
//! When a member names pictures (`ask_decision`, a hand-in), [`warm`] makes
//! the thumbnail and the tiny preview ahead of any screen, off the daemon's
//! loop. Past [`FILES_MAX`] files or [`BYTES_MAX`] bytes, the copies used
//! longest ago go.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::protocol::rpc::ImageRead;

/// A screen's thumbnail: its longer side in pixels (a tile drawn at 2x).
pub const THUMB_SIDE: u32 = 640;
/// A tiny preview, shown blurred the moment a card opens while the thumbnail comes.
pub const TINY_SIDE: u32 = 48;
/// The most copies kept…
pub const FILES_MAX: usize = 4000;
/// …and the most bytes.
pub const BYTES_MAX: u64 = 512 << 20;
/// A prune runs after this many copies are written.
const PRUNE_EVERY: usize = 64;

/// Where the copies are kept.
pub fn dir(home: &Path) -> PathBuf {
    home.join("cache").join("thumbs")
}

/// The name a copy of `real` scaled to `side` is kept under: its path, its
/// size and modified time, and the side — a file changed in place is another
/// key, so a stale copy is never given.
fn key(real: &Path, meta: &std::fs::Metadata, side: u32) -> String {
    let mut h = DefaultHasher::new();
    real.hash(&mut h);
    meta.len().hash(&mut h);
    meta.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos()).hash(&mut h);
    side.hash(&mut h);
    // A second, differently seeded hash: 128 bits, so two pictures never share a name.
    let mut h2 = DefaultHasher::new();
    (side, "equipe-thumb", real).hash(&mut h2);
    meta.len().hash(&mut h2);
    format!("{:016x}{:016x}", h.finish(), h2.finish())
}

/// The kept copy's header: what [`ImageRead`] says besides its bytes.
#[derive(serde::Serialize, serde::Deserialize)]
struct Header {
    mime: String,
    size: u64,
    width: Option<u32>,
    height: Option<u32>,
    scaled: bool,
}

/// The copy of `real` at `side` kept earlier, if there is one: a header
/// line, then the bytes.
pub fn get(home: &Path, real: &Path, side: u32) -> Option<ImageRead> {
    use base64::Engine as _;
    let meta = std::fs::metadata(real).ok()?;
    let file = dir(home).join(key(real, &meta, side));
    let bytes = std::fs::read(&file).ok()?;
    let split = bytes.iter().position(|b| *b == b'\n')?;
    let header: Header = serde_json::from_slice(&bytes[..split]).ok()?;
    super::cache::touch(&file);
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes[split + 1..]);
    Some(ImageRead { mime: header.mime, size: header.size, width: header.width, height: header.height, data, scaled: header.scaled })
}

/// Copies written since the last prune.
static WRITTEN: AtomicUsize = AtomicUsize::new(0);
/// One prune at a time.
static PRUNING: Mutex<()> = Mutex::new(());

/// Keep `read` (the copy of `real` at `side`, its bytes `raw`) for next
/// time. Written whole or not at all: a half-written copy is never read.
pub fn put(home: &Path, real: &Path, side: u32, read: &ImageRead, raw: &[u8]) {
    let Ok(meta) = std::fs::metadata(real) else { return };
    let dir = dir(home);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let name = key(real, &meta, side);
    let header = Header { mime: read.mime.clone(), size: read.size, width: read.width, height: read.height, scaled: read.scaled };
    let Ok(line) = serde_json::to_vec(&header) else { return };
    let part = dir.join(format!("{name}.{}.part", std::process::id()));
    let written = std::fs::File::create(&part).and_then(|mut f| {
        f.write_all(&line)?;
        f.write_all(b"\n")?;
        f.write_all(raw)
    });
    if written.is_err() || std::fs::rename(&part, dir.join(&name)).is_err() {
        let _ = std::fs::remove_file(&part);
        return;
    }
    if WRITTEN.fetch_add(1, Ordering::Relaxed) + 1 >= PRUNE_EVERY {
        WRITTEN.store(0, Ordering::Relaxed);
        prune(home, FILES_MAX, BYTES_MAX);
    }
}

/// Bring the kept copies within `files` and `bytes`, the ones used longest
/// ago going first; half-written ones an hour old go too.
pub fn prune(home: &Path, files: usize, bytes: u64) {
    let _one = PRUNING.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(read) = std::fs::read_dir(dir(home)) else { return };
    let now = SystemTime::now();
    let mut kept: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for entry in read.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let used = meta.modified().unwrap_or(UNIX_EPOCH);
        if entry.file_name().to_string_lossy().ends_with(".part") {
            if now.duration_since(used).is_ok_and(|age| age.as_secs() > 3600) {
                let _ = std::fs::remove_file(entry.path());
            }
            continue;
        }
        kept.push((used, meta.len(), entry.path()));
    }
    kept.sort_by_key(|k| std::cmp::Reverse(k.0));
    let mut total = 0u64;
    for (n, (_, len, path)) in kept.into_iter().enumerate() {
        total += len;
        if n >= files || total > bytes {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The pictures being decoded now; told when one is done.
static DECODING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static DECODED: Condvar = Condvar::new();

/// Held while picture `real` is decoded and scaled: another ask for it
/// waits, then finds the copy kept rather than decoding it again.
pub struct Decoding(PathBuf);

/// Mark picture `real` as being decoded, waiting while another ask decodes it.
pub fn decoding(real: &Path) -> Decoding {
    let mut now = DECODING.lock().unwrap_or_else(|e| e.into_inner());
    while now.iter().any(|p| p == real) {
        now = DECODED.wait(now).unwrap_or_else(|e| e.into_inner());
    }
    now.push(real.to_owned());
    Decoding(real.to_owned())
}

impl Drop for Decoding {
    fn drop(&mut self) {
        DECODING.lock().unwrap_or_else(|e| e.into_inner()).retain(|p| p != &self.0);
        DECODED.notify_all();
    }
}

/// Pictures being warmed now, so a burst of the same names warms each once.
static WARMING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Make the thumbnail and the tiny preview of each of `paths` (pictures a
/// member just named, each already checked against where it works) on a
/// thread of their own, so the first screen to ask reads a file.
pub fn warm(home: &Path, paths: Vec<String>) {
    let paths: Vec<String> = {
        let mut warming = WARMING.lock().unwrap_or_else(|e| e.into_inner());
        let fresh: Vec<String> = paths.into_iter().filter(|p| !warming.contains(p)).collect();
        warming.extend(fresh.iter().cloned());
        fresh
    };
    if paths.is_empty() {
        return;
    }
    let home = home.to_owned();
    let spawned = std::thread::Builder::new().name("equipe-thumbs".into()).spawn({
        let paths = paths.clone();
        move || {
            for path in &paths {
                let path = Path::new(path);
                // Checked against the picture's own folder: it was checked where it was named.
                let Some(root) = path.parent().map(Path::to_path_buf) else { continue };
                for side in [TINY_SIDE, THUMB_SIDE] {
                    let _ = super::read_cached(&home, path, std::slice::from_ref(&root), Some(side));
                }
            }
            WARMING.lock().unwrap_or_else(|e| e.into_inner()).retain(|p| !paths.contains(p));
        }
    });
    if spawned.is_err() {
        WARMING.lock().unwrap_or_else(|e| e.into_inner()).retain(|p| !paths.contains(p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_for(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-thumbs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn a_kept_copy_is_given_back_until_its_picture_changes() {
        let home = dir_for("keep");
        let picture = home.join("a.png");
        image::RgbImage::from_pixel(1200, 900, image::Rgb([200, 30, 30])).save(&picture).unwrap();
        let first = super::super::read_cached(&home, &picture, std::slice::from_ref(&home), Some(THUMB_SIDE)).unwrap();
        assert!(first.scaled && first.mime == "image/jpeg", "{}", first.mime);
        assert_eq!(std::fs::read_dir(dir(&home)).unwrap().count(), 2, "kept, and its tiny preview made from the same decode");
        assert!(get(&home, &picture, TINY_SIDE).is_some_and(|tiny| tiny.mime == "image/jpeg" && tiny.width == Some(1200)));
        let again = get(&home, &picture, THUMB_SIDE).expect("read back");
        assert_eq!((again.data, again.width, again.height), (first.data, Some(1200), Some(900)));
        // Changed in place: another key, so the old copy is never given for it.
        std::thread::sleep(std::time::Duration::from_millis(20));
        image::RgbImage::from_pixel(800, 800, image::Rgb([30, 200, 30])).save(&picture).unwrap();
        assert!(get(&home, &picture, THUMB_SIDE).is_none());
        let fresh = super::super::read_cached(&home, &picture, std::slice::from_ref(&home), Some(THUMB_SIDE)).unwrap();
        assert_eq!(fresh.width, Some(800));
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_prune_keeps_the_copies_used_last() {
        let home = dir_for("prune");
        let kept = dir(&home);
        std::fs::create_dir_all(&kept).unwrap();
        for n in 0..5u64 {
            let file = kept.join(format!("{n:032x}"));
            std::fs::write(&file, vec![0u8; 100]).unwrap();
            std::fs::File::open(&file).unwrap().set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + n)).unwrap();
        }
        prune(&home, 3, u64::MAX);
        let mut left: Vec<String> = std::fs::read_dir(&kept).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, [format!("{:032x}", 2), format!("{:032x}", 3), format!("{:032x}", 4)]);
        prune(&home, 10, 150);
        assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 1, "within the bytes");
        std::fs::remove_dir_all(home).unwrap();
    }
}
