//! What a squad's `visuals/` may hold: its drawn diagrams and cards are kept
//! by their key, and past [`KEYS_MAX`] keys or [`BYTES_MAX`] bytes the ones
//! used longest ago go — both tones and a card's page together — but never
//! one a conversation still shows (docs/VISUALS.md, *Limits*). "Used" is a
//! file's modified time: set as it is drawn, and again as it is read back or
//! given to a screen ([`touch`]).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// The most keys a squad's `visuals/` keeps…
pub const KEYS_MAX: usize = 1000;
/// …and the most bytes.
pub const BYTES_MAX: u64 = 256 << 20;
/// A half-written file (`.part`) older than this was left by a drawing that
/// never finished: it goes.
const PART_AGE: Duration = Duration::from_secs(60);

/// How much a `visuals/` may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    pub keys: usize,
    pub bytes: u64,
}

impl Default for Quota {
    fn default() -> Self {
        Self { keys: KEYS_MAX, bytes: BYTES_MAX }
    }
}

/// What a `visuals/` holds, in keys and bytes (half-written files left out).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Held {
    pub keys: usize,
    pub bytes: u64,
}

/// Marks a kept file used now, so it is the last to go.
pub fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// The key a file of `visuals/` is kept under, and whether it is half
/// written: `<key>-light.png`, `<key>-dark.png`, `<key>.html`, or their
/// `.part`. Anything else isn't ours: left alone.
fn key_of(name: &str) -> Option<(&str, bool)> {
    let key = name.get(..16).filter(|k| k.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))?;
    match &name[16..] {
        "-light.png" | "-dark.png" | ".html" => Some((key, false)),
        "-light.part" | "-dark.part" | ".part" => Some((key, true)),
        _ => None,
    }
}

/// One drawing at a time decides what goes.
static PRUNING: Mutex<()> = Mutex::new(());

/// Brings `dir` within `quota`: the keys used longest ago go first, whole,
/// none of `keep` (what the squad's conversations show, and what was just
/// drawn). What `keep` holds stays even past the quota. What it holds after.
pub fn prune(dir: &Path, keep: &HashSet<String>, quota: Quota) -> Held {
    let _one = PRUNING.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(read) = std::fs::read_dir(dir) else { return Held::default() };
    // Each key: its files, their bytes, when it was last used.
    let mut keys: HashMap<String, (Vec<std::path::PathBuf>, u64, SystemTime)> = HashMap::new();
    let now = SystemTime::now();
    for entry in read.flatten() {
        let name = entry.file_name();
        let Some((key, partial)) = name.to_str().and_then(key_of) else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let used = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if partial {
            if now.duration_since(used).is_ok_and(|age| age > PART_AGE) {
                let _ = std::fs::remove_file(entry.path());
            }
            continue;
        }
        let held = keys.entry(key.to_owned()).or_insert_with(|| (Vec::new(), 0, SystemTime::UNIX_EPOCH));
        held.0.push(entry.path());
        held.1 += meta.len();
        held.2 = held.2.max(used);
    }
    let mut held = Held { keys: keys.len(), bytes: keys.values().map(|k| k.1).sum() };
    let mut oldest: Vec<_> = keys.into_iter().filter(|(key, _)| !keep.contains(key)).collect();
    oldest.sort_by_key(|(_, (_, _, used))| *used);
    for (_, (files, bytes, _)) in oldest {
        if held.keys <= quota.keys && held.bytes <= quota.bytes {
            break;
        }
        for file in files {
            let _ = std::fs::remove_file(file);
        }
        held.keys -= 1;
        held.bytes -= bytes;
    }
    held
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-cachetest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn key(n: usize) -> String {
        format!("{n:016x}")
    }

    /// A key's files, used `ago` seconds ago.
    fn put(dir: &Path, n: usize, bytes: usize, ago: u64, card: bool) {
        let mut names = vec![format!("{}-light.png", key(n)), format!("{}-dark.png", key(n))];
        if card {
            names.push(format!("{}.html", key(n)));
        }
        for name in names {
            let path = dir.join(name);
            std::fs::write(&path, vec![0u8; bytes]).unwrap();
            std::fs::File::open(&path).unwrap().set_modified(SystemTime::now() - Duration::from_secs(ago)).unwrap();
        }
    }

    #[test]
    fn the_oldest_keys_go_whole_and_what_is_shown_stays() {
        let dir = temp_dir("lru");
        // Key 0 is the oldest, 9 the newest; 1 is shown in a conversation.
        for n in 0..10 {
            put(&dir, n, 100, 1000 - n as u64 * 10, n % 2 == 0);
        }
        std::fs::write(dir.join("notes.txt"), "not ours").unwrap();
        let keep: HashSet<String> = [key(1)].into();
        let held = prune(&dir, &keep, Quota { keys: 4, bytes: u64::MAX });
        assert_eq!(held.keys, 4);
        let left: HashSet<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        for n in 0..10 {
            let kept = left.contains(&format!("{}-light.png", key(n)));
            assert_eq!(kept, n == 1 || n >= 7, "key {n}");
            // Never half a key: its tones and page go together.
            assert_eq!(kept, left.contains(&format!("{}-dark.png", key(n))), "key {n}");
            assert!(n % 2 == 1 || kept == left.contains(&format!("{}.html", key(n))), "key {n}");
        }
        assert!(left.contains("notes.txt"));
        // By bytes too: what is used again is kept over what isn't.
        touch(&dir.join(format!("{}-dark.png", key(7))));
        let held = prune(&dir, &keep, Quota { keys: usize::MAX, bytes: 500 });
        assert!(held.bytes <= 500, "{held:?}");
        assert!(dir.join(format!("{}-light.png", key(1))).is_file() && dir.join(format!("{}-light.png", key(7))).is_file());
        assert!(!dir.join(format!("{}-light.png", key(8))).exists());
        // What is shown stays, even past the quota.
        let held = prune(&dir, &keep, Quota { keys: 0, bytes: 0 });
        assert_eq!(held.keys, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_half_written_file_left_behind_goes_and_one_being_written_stays() {
        let dir = temp_dir("part");
        let stale = dir.join(format!("{}-light.part", key(1)));
        let fresh = dir.join(format!("{}.part", key(2)));
        std::fs::write(&stale, "x").unwrap();
        std::fs::File::open(&stale).unwrap().set_modified(SystemTime::now() - PART_AGE * 2).unwrap();
        std::fs::write(&fresh, "x").unwrap();
        assert_eq!(prune(&dir, &HashSet::new(), Quota::default()), Held::default());
        assert!(!stale.exists() && fresh.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
