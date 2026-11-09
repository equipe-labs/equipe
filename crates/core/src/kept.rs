//! Attachments the person sent, kept.
//!
//! A screen puts a file in `<home>/uploads` before the message that names it
//! is sent; the message carries only its tag (`equipe_protocol::tags`).
//! Uploads no sent message names — a draft given up — are swept after a week
//! ([`crate::gateway::sweep_uploads`]). Those a sent message names stay until
//! the person removes them (the disk panel): their names are kept in
//! `<home>/uploads.kept`, one per line, written only by the daemon when a
//! message is sent and read by whoever sweeps.
//!
//! An attached text file — a voice memo written down, a long paste — is also
//! kept as words with the message itself ([`words`], `Entry::User::kept`), so
//! the conversation, the members' history and a search still hold what was
//! said once the file is gone.
//!
//! [`rescue`] does both, once, for what a journal sent before this was kept.

use std::collections::HashSet;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use equipe_protocol::Entry;
use equipe_protocol::tags::{self, KEPT_FILE_MAX, KEPT_MESSAGE_MAX, Kept, TagKind};

use crate::gateway;
use crate::journal::{self, Journal};

/// The kept uploads' names, beside the uploads (never in them: a sweep or a
/// read back would take it for one).
pub const INDEX: &str = "uploads.kept";
/// A kept attachment sent longer ago than this is offered to the person to
/// remove (never removed without them).
pub const OLD_AFTER: Duration = Duration::from_secs(30 * 24 * 3600);

/// One writer at a time within the daemon: a send adds while the person removes.
static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn index(home: &Path) -> PathBuf {
    home.join(INDEX)
}

/// The names (in `<home>/uploads`) of the uploads sent messages keep.
pub fn names(home: &Path) -> HashSet<String> {
    std::fs::read_to_string(index(home)).unwrap_or_default().lines().map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned).collect()
}

/// The uploads of this machine `text`'s tags point at: each as its tag
/// names it, and its name in the uploads folder. A path outside the
/// uploads, through a link, or gone, is none of them.
fn uploads_in(home: &Path, text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for tag in tags::find(text) {
        for path in tag.files() {
            let Ok(real) = gateway::upload_file(home, Path::new(path)) else { continue };
            let Some(name) = real.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            if !out.iter().any(|(_, n)| *n == name) {
                out.push((path.to_owned(), name));
            }
        }
    }
    out
}

/// Keep what a sent message attaches: from now on it is never swept. How
/// many weren't kept already.
pub fn keep(home: &Path, text: &str) -> usize {
    let found = uploads_in(home, text);
    if found.is_empty() {
        return 0;
    }
    add(home, found.into_iter().map(|(_, name)| name))
}

fn add(home: &Path, names: impl IntoIterator<Item = String>) -> usize {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let known = self::names(home);
    let mut new: Vec<String> = Vec::new();
    for name in names {
        if !known.contains(&name) && !new.contains(&name) {
            new.push(name);
        }
    }
    if new.is_empty() {
        return 0;
    }
    let lines: String = new.iter().map(|n| format!("{n}\n")).collect();
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt as _; options.mode(0o600); }
    let written = options.open(index(home)).and_then(|mut f| f.write_all(lines.as_bytes()));
    match written {
        Ok(()) => new.len(),
        Err(err) => {
            crate::daemon_log!("equipe: attachments of a sent message couldn't be marked kept: {err}");
            0
        }
    }
}

/// Forget `gone` among the kept names: the person removed them.
fn forget(home: &Path, gone: &HashSet<String>) -> std::io::Result<()> {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let left: String = std::fs::read_to_string(index(home)).unwrap_or_default().lines().map(str::trim).filter(|n| !n.is_empty() && !gone.contains(*n)).map(|n| format!("{n}\n")).collect();
    let tmp = home.join(format!("{INDEX}.tmp"));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt as _; options.mode(0o600); }
    options.open(&tmp)?.write_all(left.as_bytes())?;
    std::fs::rename(&tmp, index(home))
}

/// The words of the text files `text` attaches, as they are now — to keep
/// with the message. Only a file of this machine's uploads (never a path
/// outside them or a link: [`gateway::upload_file`]), UTF-8 text, at most
/// [`KEPT_FILE_MAX`]; the message's together at most [`KEPT_MESSAGE_MAX`].
pub fn words(home: &Path, text: &str) -> Vec<Kept> {
    let mut kept: Vec<Kept> = Vec::new();
    let mut size = 0;
    for tag in tags::find(text) {
        if tag.kind != TagKind::File || kept.iter().any(|k| k.path == tag.path) {
            continue;
        }
        let Some(words) = text_of(home, &tag.path) else { continue };
        if size + words.len() > KEPT_MESSAGE_MAX {
            continue;
        }
        size += words.len();
        kept.push(Kept { path: tag.path.clone(), text: words });
    }
    kept
}

/// An upload's words, when it is text small enough to keep.
fn text_of(home: &Path, path: &str) -> Option<String> {
    let real = gateway::upload_file(home, Path::new(path)).ok()?;
    let file = std::fs::File::open(real).ok()?;
    if file.metadata().ok()?.len() > KEPT_FILE_MAX {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(KEPT_FILE_MAX + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > KEPT_FILE_MAX || bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok().filter(|t| !t.trim().is_empty())
}

/// What the kept attachments take: all of them, and those sent over
/// [`OLD_AFTER`] ago (by `now`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub files: usize,
    pub bytes: u64,
    pub old_files: usize,
    pub old_bytes: u64,
}

/// The kept files there are, each with the room it takes on disk and
/// whether it is old.
fn kept_files(home: &Path, now: SystemTime) -> Vec<(String, u64, bool)> {
    let kept = names(home);
    let Ok(entries) = std::fs::read_dir(gateway::uploads_dir(home)) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta = entry.metadata().ok().filter(|m| m.file_type().is_file())?;
            kept.contains(&name).then(|| {
                let old = meta.modified().ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > OLD_AFTER);
                (name, allocated_bytes(&meta), old)
            })
        })
        .collect()
}

#[cfg(unix)]
fn allocated_bytes(meta: &std::fs::Metadata) -> u64 { use std::os::unix::fs::MetadataExt as _; meta.blocks() * 512 }
#[cfg(windows)]
fn allocated_bytes(meta: &std::fs::Metadata) -> u64 { meta.len() }

pub fn usage(home: &Path, now: SystemTime) -> Usage {
    let mut usage = Usage::default();
    for (_, bytes, old) in kept_files(home, now) {
        usage.files += 1;
        usage.bytes += bytes;
        if old {
            usage.old_files += 1;
            usage.old_bytes += bytes;
        }
    }
    usage
}

/// Remove the kept attachments sent over [`OLD_AFTER`] ago — on the
/// person's yes only. Their words kept with the messages stay. How many
/// files, and their bytes.
pub fn remove_old(home: &Path, now: SystemTime) -> (usize, u64) {
    let dir = gateway::uploads_dir(home);
    let mut gone: HashSet<String> = HashSet::new();
    let mut bytes = 0;
    for (name, size, old) in kept_files(home, now) {
        if old && std::fs::remove_file(dir.join(&name)).is_ok() {
            bytes += size;
            gone.insert(name);
        }
    }
    if !gone.is_empty()
        && let Err(err) = forget(home, &gone)
    {
        crate::daemon_log!("equipe: removed attachments are still listed as kept: {err}");
    }
    (gone.len(), bytes)
}

/// What [`rescue`] did for one journal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rescued {
    /// Attachments of sent messages, still there, now kept.
    pub kept: usize,
    /// Text files whose words are now kept with their message.
    pub words: usize,
    /// Attachments sent messages name that were gone already.
    pub gone: usize,
}

/// Once per journal, for what was sent before attachments were kept: every
/// upload its messages (and what still waits for a member) name that is
/// still there is kept, and a text file's words go into its message's line.
/// `None` when it was done already. Doing it again changes nothing.
pub fn rescue(home: &Path, journal: &Journal) -> journal::Result<Option<Rescued>> {
    if journal.kept_rescued() {
        return Ok(None);
    }
    let mut done = Rescued::default();
    let mut names: Vec<String> = Vec::new();
    let mut seen_gone: HashSet<String> = HashSet::new();
    let mut note = |text: &str, names: &mut Vec<String>, done: &mut Rescued| {
        let found = uploads_in(home, text);
        for tag in tags::find(text) {
            for path in tag.files() {
                let is_upload = Path::new(path).parent().and_then(Path::file_name).is_some_and(|d| d == "uploads");
                if is_upload && !found.iter().any(|(p, _)| p == path) && seen_gone.insert(path.to_owned()) {
                    done.gone += 1;
                }
            }
        }
        names.extend(found.into_iter().map(|(_, name)| name));
    };
    for (session, seq, entry) in journal.messages_holding("/uploads/")? {
        let Entry::User { text, mut kept } = entry else { continue };
        note(&text, &mut names, &mut done);
        let mut changed = false;
        let mut size: usize = kept.iter().map(|k| k.text.len()).sum();
        for found in words(home, &text) {
            if tags::kept_of(&kept, &found.path).is_none() && size + found.text.len() <= KEPT_MESSAGE_MAX {
                size += found.text.len();
                kept.push(found);
                done.words += 1;
                changed = true;
            }
        }
        if changed {
            journal.rewrite(session, seq, &Entry::User { text, kept })?;
        }
    }
    for text in journal.all_waiting()? {
        note(&text, &mut names, &mut done);
    }
    done.kept = add(home, names);
    journal.set_kept_rescued()?;
    Ok(Some(done))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-kept-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn upload(home: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let (mut file, path) = gateway::create_upload(home, name, Some(equipe_protocol::rpc::UploadKind::File)).unwrap();
        file.write_all(bytes).unwrap();
        path
    }

    fn sent(home: &Path) -> (PathBuf, PathBuf, String) {
        let memo = upload(home, "memo.m4a", b"\0\0audio");
        let words = upload(home, "memo-transcript.txt", "Ship the pricing page by Friday.\n".as_bytes());
        let text = format!("listen [audio: {} · 0:31] [file: {} · 1 line]", memo.display(), words.display());
        (memo, words, text)
    }

    #[test]
    fn a_sent_messages_attachments_are_kept_once_and_its_text_files_words_read() {
        let home = home("keep");
        let (memo, words, text) = sent(&home);
        assert_eq!(keep(&home, &text), 2);
        assert_eq!(keep(&home, &text), 0, "kept once");
        let names = names(&home);
        assert!(names.contains(memo.file_name().unwrap().to_str().unwrap()) && names.contains(words.file_name().unwrap().to_str().unwrap()));
        assert_eq!(super::words(&home, &text), [Kept { path: words.display().to_string(), text: "Ship the pricing page by Friday.\n".into() }], "the sound isn't text");
    }

    #[test]
    fn only_text_within_the_limits_is_kept_as_words() {
        let home = home("limits");
        let big = upload(&home, "big.txt", &vec![b'a'; KEPT_FILE_MAX as usize + 1]);
        let edge = upload(&home, "edge.txt", &vec![b'b'; KEPT_FILE_MAX as usize]);
        let binary = upload(&home, "x.txt", b"one\0two");
        let latin = upload(&home, "y.txt", &[0xff, 0xfe, 0x41]);
        let tag = |p: &Path| format!("[file: {}]", p.display());
        assert!(super::words(&home, &tag(&big)).is_empty(), "past the most for a file");
        assert_eq!(super::words(&home, &tag(&edge))[0].text.len(), KEPT_FILE_MAX as usize, "right at it: kept");
        assert!(super::words(&home, &format!("{} {}", tag(&binary), tag(&latin))).is_empty(), "not UTF-8 text");
        // Two files at the most each: the message keeps what fits.
        let other = upload(&home, "other.txt", &vec![b'c'; KEPT_FILE_MAX as usize]);
        let both = super::words(&home, &format!("{} {}", tag(&edge), tag(&other)));
        assert_eq!(both.len(), 1, "a message keeps at most {KEPT_MESSAGE_MAX} bytes");
        // The big one is still kept as a file.
        assert_eq!(keep(&home, &tag(&big)), 1);
    }

    #[test]
    fn nothing_outside_the_uploads_is_read_or_kept() {
        use std::os::unix::fs::symlink;
        let home = home("safe");
        let secret = home.join("credentials.txt");
        std::fs::write(&secret, "token").unwrap();
        upload(&home, "a.txt", b"x");
        let uploads = gateway::uploads_dir(&home);
        symlink(&secret, uploads.join("link.txt")).unwrap();
        for path in [secret.clone(), uploads.join("../credentials.txt"), uploads.join("link.txt"), uploads.join("gone.txt")] {
            let text = format!("[file: {}]", path.display());
            assert!(super::words(&home, &text).is_empty(), "{}", path.display());
            assert_eq!(keep(&home, &text), 0, "{}", path.display());
        }
        assert!(!names(&home).contains("credentials.txt"));
    }

    #[test]
    fn old_kept_attachments_go_only_when_asked_and_their_names_with_them() {
        let home = home("old");
        let (memo, words, text) = sent(&home);
        keep(&home, &text);
        let draft = upload(&home, "draft.png", b"png");
        let now = SystemTime::now();
        let u = usage(&home, now);
        assert_eq!((u.files, u.old_files), (2, 0), "the draft isn't kept");
        use std::os::unix::fs::MetadataExt as _;
        assert_eq!(u.bytes, (std::fs::metadata(&memo).unwrap().blocks() + std::fs::metadata(&words).unwrap().blocks()) * 512, "the room they take");
        let later = now + OLD_AFTER + Duration::from_secs(3600);
        assert_eq!(usage(&home, later).old_files, 2);
        assert_eq!(remove_old(&home, now), (0, 0), "nothing old yet");
        let (files, _) = remove_old(&home, later);
        assert_eq!(files, 2);
        assert!(!memo.exists() && !words.exists() && draft.exists(), "only the kept old ones");
        assert!(names(&home).is_empty());
    }
}
