//! Design captures kept out of git (docs/DESIGN-CAPTURES.md).
//!
//! A capture is a picture or a video equipe's design tools make under a
//! design folder — the UI map's shots and walkthroughs, design QA's and the
//! lint's renders, mockups, directions, reference shots, critiques' and
//! sessions' renders. Re-rendered again and again, committed they grew one
//! project's history past ten gigabytes. So, for every squad and project:
//!
//! - **Git never holds them.** `design/.gitignore` carries a block equipe
//!   keeps ([`ignore_block`]); every commit equipe makes in a ticket's
//!   worktree untracks any capture first ([`prepare_commit`]).
//! - **Git holds what they are.** `design/captures.json` ([`Manifest`]) lists
//!   each capture by its path in the design folder, its hash (git's blob id
//!   of its bytes), size, when it was made and the ticket that made it — small,
//!   reviewable, merged like code. It is the only thing a sync (equipe Hub)
//!   needs to exchange besides the bytes.
//! - **equipe's store holds the bytes**, content-addressed — one file per
//!   hash, identical renders kept once: `<git common dir>/equipe/captures/
//!   objects/<2 hex>/<rest>` ([`Store`]). Outside the working tree, never
//!   committed or pushed, shared by the main checkout and every worktree of
//!   the repository.
//! - **Checkouts hold them on disk where they always were**, so every screen
//!   and tool finds them at the same paths: a new worktree is filled from the
//!   store ([`after_checkout`]); a merge onto main puts what it changed into
//!   the main checkout ([`publish`]).
//! - **Landing guards it** ([`guard`]): a branch whose commits add captures
//!   anyway (`git add -f`, an older equipe) has its own unmerged commits
//!   rewritten without them before it merges — history stays small — and the
//!   member is told; a commit that adds more than [`HEAVY_COMMIT`] of other
//!   images is reported too. Two branches' manifests merge entry by entry
//!   ([`merge3`]), never as a conflict.
//! - **Upgrading** ([`migrate`]) is one normal commit on main that stops
//!   tracking the captures it holds (the files stay on disk and go into the
//!   store) — history is never rewritten; `scripts/purge-design-captures.sh`
//!   does that, only when a person runs it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::worktree::{self, Result};

/// The folders of a design folder whose pictures and videos are captures.
/// Its own assets (`assets/`, `icons/`, `components/`, …) are not: a logo is
/// made once and kept, in git.
pub const DIRS: [&str; 12] = ["flows", "qa", "checks", "mockups", "directions", "references", "critiques", "sessions", "audit", "palettes", "shots", "renders"];
/// What a capture is, by its extension (any case).
pub const EXTS: [&str; 8] = ["png", "jpg", "jpeg", "webp", "gif", "mp4", "webm", "mov"];
/// The manifest's name in the design folder, and its path in a repository.
pub const MANIFEST: &str = "captures.json";
pub const MANIFEST_PATH: &str = "design/captures.json";
/// The ignore file equipe keeps its block in.
pub const IGNORE_PATH: &str = "design/.gitignore";
const BLOCK_START: &str = "# >>> equipe: design captures stay out of git";
const BLOCK_END: &str = "# <<< equipe: design captures";
/// The manifest's format.
pub const VERSION: u32 = 1;
/// One commit adding more images than this to git is reported at landing.
pub const HEAVY_COMMIT: u64 = 10 << 20;
/// The store, in a repository's common git folder.
const STORE: &str = "equipe/captures";
/// In a worktree's admin folder: the captures it was given when made, one
/// path a line — what it lacks later, its member removed.
const SEED: &str = "equipe-captures-seed";
const DEPTH_MAX: usize = 8;
const FILES_MAX: usize = 200_000;
/// Objects no branch's manifest names are kept this long (seconds): work
/// not committed yet may name them soon.
const UNNAMED_KEPT: u64 = 7 * 86_400;

/// Whether `rel` — a path from the top of a repository, `/`-separated — is
/// a capture: `design/<one of DIRS>/…/<name>.<one of EXTS>`.
pub fn is_capture(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    parts.len() >= 3 && parts[0] == super::DIR && DIRS.contains(&parts[1]) && parts.iter().all(|p| !p.is_empty() && *p != "..") && is_media_name(parts[parts.len() - 1])
}

/// Whether a file's name is a picture's or a video's.
pub fn is_media_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && EXTS.iter().any(|e| e.eq_ignore_ascii_case(ext)))
}

/// `png` as `[pP][nN][gG]`: git matches ignore patterns by case only where
/// the filesystem doesn't.
fn any_case(ext: &str) -> String {
    ext.chars().map(|c| format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase())).collect()
}

/// The block equipe keeps in `design/.gitignore`.
pub fn ignore_block() -> String {
    let mut out = format!(
        "{BLOCK_START}\n# Pictures and videos equipe's design tools make stay on disk and in equipe's store;\n# captures.json lists them. equipe rewrites this block: edit outside it.\n"
    );
    for dir in DIRS {
        for ext in EXTS {
            out.push_str(&format!("/{dir}/**/*.{}\n", any_case(ext)));
        }
    }
    out.push_str(BLOCK_END);
    out.push('\n');
    out
}

/// `existing` (an ignore file's text) with equipe's block in it, as it is
/// now: replaced where it was, else added at the end. The rest is kept.
pub fn with_block(existing: &str) -> String {
    let block = ignore_block();
    if let Some(start) = existing.find(BLOCK_START)
        && let Some(len) = existing[start..].find(BLOCK_END)
    {
        let mut end = start + len + BLOCK_END.len();
        if existing[end..].starts_with('\n') {
            end += 1;
        }
        return format!("{}{block}{}", &existing[..start], &existing[end..]);
    }
    let mut out = existing.to_owned();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(&block);
    out
}

/// Whether `checkout`'s design folder is one equipe keeps: a design system
/// (`tokens.json`, `guidelines.md`) or a UI map. Another project's own
/// `design/` folder is left as it is.
pub fn applies(checkout: &Path) -> bool {
    let design = checkout.join(super::DIR);
    super::folder::is_design_dir(&design) || design.join(super::flows::DIR).join(super::flows::FILE).is_file()
}

/// [`applies`], of the tree at `rev` rather than a checkout.
fn tree_applies(repo: &Path, rev: &str) -> bool {
    let at = |rel: &str| worktree::git(repo, &["cat-file", "-e", &format!("{rev}:{rel}")]).is_ok();
    ["design/tokens.json", "design/guidelines.md", "design/flows/flows.json"].iter().any(|rel| at(rel))
}

/// The ticket a branch equipe made is for: `equipe/<label>-t<ticket>-<hex>-<n>`.
pub fn ticket_of_branch(branch: &str) -> Option<u64> {
    branch.rsplit(['-', '/']).find_map(|part| part.strip_prefix('t').filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())).and_then(|d| d.parse().ok()))
}

// ── the manifest ──

/// One capture, as `design/captures.json` lists it. Nothing in it is an
/// absolute path or names a machine: it means the same in any clone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Its path in the design folder: `flows/login/form/error.dark.390.png`.
    pub path: String,
    /// Git's blob id of its bytes (SHA-1, or SHA-256 in such a repository):
    /// what the store keeps it by.
    pub hash: String,
    pub size: u64,
    /// When it was made, in seconds since 1970.
    pub created_at: u64,
    /// The ticket whose work made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<u64>,
    /// A UI map picture's flow, screen, state, mode (`light`/`dark`) and width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
}

impl Entry {
    /// An entry for the capture at `path` (in the design folder), with what
    /// its name says of a UI map picture filled in.
    pub fn new(path: &str, hash: &str, size: u64, created_at: u64, ticket: Option<u64>) -> Entry {
        let mut entry = Entry { path: path.to_owned(), hash: hash.to_owned(), size, created_at, ticket, ..Entry::default() };
        if let [dir, flow, screen, file] = path.split('/').collect::<Vec<_>>()[..]
            && dir == super::flows::DIR
            && let Some((state, mode, width)) = super::flows::parse_picture_name(file)
        {
            entry.flow = Some(flow.to_owned());
            entry.screen = Some(screen.to_owned());
            entry.state = Some(state);
            entry.mode = Some(mode);
            entry.width = Some(width);
        }
        entry
    }
}

/// `design/captures.json`: every capture of a design folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// By path in the design folder.
    pub entries: BTreeMap<String, Entry>,
}

#[derive(Deserialize)]
struct ManifestFile {
    #[serde(default)]
    captures: Vec<Entry>,
}

impl Manifest {
    /// The manifest in `text`; `None` when it isn't one (a conflict's markers in it, say).
    pub fn parse(text: &str) -> Option<Manifest> {
        let file: ManifestFile = serde_json::from_str(text).ok()?;
        let entries = file.captures.into_iter().filter(|e| is_capture(&format!("{}/{}", super::DIR, e.path)) && is_hash(&e.hash)).map(|e| (e.path.clone(), e)).collect();
        Some(Manifest { entries })
    }

    /// As the file holds it: one capture a line, by path, so that git's diff
    /// and merge see one line per picture.
    pub fn render(&self) -> String {
        let lines: Vec<String> = self.entries.values().map(|e| serde_json::to_string(e).unwrap_or_default()).collect();
        if lines.is_empty() {
            return format!("{{\"version\": {VERSION}, \"captures\": []}}\n");
        }
        format!("{{\"version\": {VERSION}, \"captures\": [\n{}\n]}}\n", lines.join(",\n"))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn hash_of(&self, path: &str) -> Option<&str> {
        self.entries.get(path).map(|e| e.hash.as_str())
    }
}

fn is_hash(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Three manifests merged entry by entry: what one side changed since
/// `base` and the other didn't is taken; a capture both changed is the one
/// made last (a removal loses to a new render).
pub fn merge3(base: &Manifest, ours: &Manifest, theirs: &Manifest) -> Manifest {
    let paths: BTreeSet<&String> = base.entries.keys().chain(ours.entries.keys()).chain(theirs.entries.keys()).collect();
    let mut out = Manifest::default();
    for path in paths {
        let (b, o, t) = (base.hash_of(path), ours.hash_of(path), theirs.hash_of(path));
        let pick = if t == b || o == t {
            ours.entries.get(path)
        } else if o == b {
            theirs.entries.get(path)
        } else {
            match (ours.entries.get(path), theirs.entries.get(path)) {
                (Some(o), Some(t)) => Some(if o.created_at > t.created_at { o } else { t }),
                (one, other) => one.or(other),
            }
        };
        if let Some(entry) = pick {
            out.entries.insert(path.clone(), entry.clone());
        }
    }
    out
}

/// The manifest committed at `rev` in `repo`; empty when it has none.
pub fn manifest_at(repo: &Path, rev: &str) -> Manifest {
    worktree::git_bytes(repo, &["show", &format!("{rev}:{MANIFEST_PATH}")], None, None).ok().and_then(|b| Manifest::parse(&String::from_utf8_lossy(&b))).unwrap_or_default()
}

/// The manifest in `checkout`'s design folder on disk.
fn manifest_on_disk(checkout: &Path) -> Option<Manifest> {
    Manifest::parse(&std::fs::read_to_string(checkout.join(MANIFEST_PATH)).ok()?)
}

// ── hashing ──

/// Which hash a repository names its objects by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Sha1,
    Sha256,
}

/// Git's blob id of the file at `path` (`git hash-object`), read as a
/// stream; with its length. An error when it changes while read.
pub fn blob_id(path: &Path, kind: Kind) -> std::io::Result<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut digest = ring::digest::Context::new(match kind {
        Kind::Sha1 => &ring::digest::SHA1_FOR_LEGACY_USE_ONLY,
        Kind::Sha256 => &ring::digest::SHA256,
    });
    digest.update(format!("blob {len}\0").as_bytes());
    let mut buf = vec![0u8; 1 << 16];
    let mut read = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
        read += n as u64;
    }
    if read != len {
        return Err(std::io::Error::other("it changed while it was read"));
    }
    Ok((digest.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect(), len))
}

/// Blob ids worked out, by file and hash kind: its length and time then, and the id.
type Kept = HashMap<(PathBuf, Kind), (u64, Option<SystemTime>, String)>;
static KEPT: std::sync::Mutex<Option<Kept>> = std::sync::Mutex::new(None);

/// [`blob_id`], remembered while the file's length and time stay the same.
fn cached_id(path: &Path, kind: Kind) -> Option<(String, u64)> {
    let meta = std::fs::symlink_metadata(path).ok().filter(|m| m.is_file())?;
    let (len, modified) = (meta.len(), meta.modified().ok());
    let key = (path.to_owned(), kind);
    if let Some((_, _, id)) = KEPT.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get(&key).filter(|(l, m, _)| *l == len && *m == modified) {
        return Some((id.clone(), len));
    }
    let (id, len) = blob_id(path, kind).ok()?;
    remember(path, kind, &id);
    Some((id, len))
}

/// That the file at `path`, as it is now, holds blob `id` — written by us
/// from the store, so the next look needn't read it.
fn remember(path: &Path, kind: Kind, id: &str) {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return };
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let kept = kept.get_or_insert_with(HashMap::new);
    if kept.len() >= 100_000 {
        kept.clear();
    }
    kept.insert((path.to_owned(), kind), (meta.len(), meta.modified().ok(), id.to_owned()));
}

fn secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}

fn now() -> u64 {
    secs(SystemTime::now())
}

// ── the store ──

/// equipe's store of a repository's captures: `objects/<2 hex>/<rest>`, one
/// file per hash, in the repository's common git folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    root: PathBuf,
    kind: Kind,
}

impl Store {
    /// The store of the repository `dir` is in (a worktree's is its repository's).
    pub fn of(dir: &Path) -> Option<Store> {
        let common = worktree::git(dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).ok()?;
        let kind = match worktree::git(dir, &["rev-parse", "--show-object-format"]) {
            Ok(format) if format == "sha256" => Kind::Sha256,
            _ => Kind::Sha1,
        };
        Some(Store { root: PathBuf::from(common).join(STORE), kind })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Where the bytes of `hash` are kept.
    pub fn object(&self, hash: &str) -> PathBuf {
        self.root.join("objects").join(&hash[..2]).join(&hash[2..])
    }

    pub fn has(&self, hash: &str) -> bool {
        is_hash(hash) && self.object(hash).is_file()
    }

    /// A file of ours beside `to`, not there yet, to be moved into place.
    fn staging(to: &Path) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        to.with_file_name(format!(".{name}.equipe-{}-{n}", std::process::id()))
    }

    /// Keep the file at `from`, whose hash is `hash`. A copy (a clone that
    /// shares its blocks on APFS), checked to hold those bytes before it is
    /// put in place.
    pub fn put_file(&self, from: &Path, hash: &str) -> Result<()> {
        if !is_hash(hash) {
            return Err(format!("{hash} isn't a hash"));
        }
        if self.has(hash) {
            return Ok(());
        }
        let to = self.object(hash);
        let parent = to.parent().expect("an object has a folder");
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        let staging = Self::staging(&to);
        let done = std::fs::copy(from, &staging).map_err(|e| format!("{}: {e}", from.display())).and_then(|_| {
            match blob_id(&staging, self.kind) {
                Ok((id, _)) if id == hash => std::fs::rename(&staging, &to).map_err(|e| format!("{}: {e}", to.display())),
                Ok(_) => Err(format!("{} changed while it was kept", from.display())),
                Err(e) => Err(format!("{}: {e}", staging.display())),
            }
        });
        if done.is_err() {
            let _ = std::fs::remove_file(&staging);
        }
        done
    }

    /// Keep blob `hash` of `repo`, read from git.
    pub fn put_blob(&self, repo: &Path, hash: &str) -> Result<()> {
        if !is_hash(hash) {
            return Err(format!("{hash} isn't a hash"));
        }
        if self.has(hash) {
            return Ok(());
        }
        let to = self.object(hash);
        let parent = to.parent().expect("an object has a folder");
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        let staging = Self::staging(&to);
        let done = worktree::git_to_file(repo, &["cat-file", "blob", hash], &staging).and_then(|()| match blob_id(&staging, self.kind) {
            Ok((id, _)) if id == hash => std::fs::rename(&staging, &to).map_err(|e| format!("{}: {e}", to.display())),
            _ => Err(format!("git's blob {hash} didn't read back as itself")),
        });
        if done.is_err() {
            let _ = std::fs::remove_file(&staging);
        }
        done
    }

    /// Put the bytes of `entry` at `to`, made at its time: written beside it
    /// and moved in whole, so `to` is the old file or the new, never half.
    pub fn place(&self, entry: &Entry, to: &Path) -> Result<()> {
        let from = self.object(&entry.hash);
        let parent = to.parent().ok_or_else(|| format!("{} has no folder", to.display()))?;
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        let staging = Self::staging(to);
        let done = std::fs::copy(&from, &staging)
            .map_err(|e| format!("{}: {e}", from.display()))
            .and_then(|_| {
                let when = UNIX_EPOCH + std::time::Duration::from_secs(entry.created_at);
                std::fs::File::options().write(true).open(&staging).and_then(|f| f.set_modified(when)).map_err(|e| format!("{}: {e}", staging.display()))
            })
            .and_then(|()| std::fs::rename(&staging, to).map_err(|e| format!("{}: {e}", to.display())));
        if done.is_err() {
            let _ = std::fs::remove_file(&staging);
        }
        done
    }

    fn report_path(&self, branch: &str) -> PathBuf {
        let name: String = branch.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
        self.root.join("reports").join(format!("{name}.json"))
    }
}

// ── on disk ──

/// The captures in `checkout`'s design folder: path in the design folder,
/// and where. Hidden files (a tool's half-written picture) and links are
/// passed over.
pub fn on_disk(checkout: &Path) -> Vec<(String, PathBuf)> {
    let design = checkout.join(super::DIR);
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, String, usize)> = DIRS.iter().map(|d| (design.join(d), (*d).to_owned(), 0)).collect();
    while let Some((at, rel, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else { continue };
        for entry in entries.flatten() {
            if out.len() >= FILES_MAX {
                return out;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            let path = format!("{rel}/{name}");
            if kind.is_dir() && depth < DEPTH_MAX {
                stack.push((entry.path(), path, depth + 1));
            } else if kind.is_file() && is_media_name(&name) {
                out.push((path, entry.path()));
            }
        }
    }
    out.sort();
    out
}

/// The paths a worktree was given when made ([`SEED`]).
fn read_seed(worktree: &Path) -> BTreeSet<String> {
    worktree::admin_dir(worktree).and_then(|a| std::fs::read_to_string(a.join(SEED)).ok()).map(|t| t.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect()).unwrap_or_default()
}

fn write_seed(worktree: &Path, paths: &BTreeSet<String>) {
    if let Some(admin) = worktree::admin_dir(worktree) {
        let text: String = paths.iter().map(|p| format!("{p}\n")).collect();
        let _ = std::fs::write(admin.join(SEED), text);
    }
}

/// The manifest of what is on disk in `checkout`, each capture kept in
/// `store`. An entry of `previous` with the same bytes keeps its time and
/// ticket; a new or changed one is made now by `ticket`. One of `previous`
/// no longer on disk goes only when `seed` says the checkout had it — its
/// member removed it; else it was never brought here, and stays.
fn from_disk(checkout: &Path, store: &Store, previous: &Manifest, seed: &BTreeSet<String>, ticket: Option<u64>) -> Manifest {
    let mut out = Manifest::default();
    for (rel, path) in on_disk(checkout) {
        let Some((hash, size)) = cached_id(&path, store.kind) else { continue };
        if let Err(err) = store.put_file(&path, &hash) {
            crate::daemon_log!("equipe: couldn't keep {rel} in the captures' store: {err}");
            continue;
        }
        let entry = match previous.entries.get(&rel) {
            Some(e) if e.hash == hash => e.clone(),
            _ => {
                let made = std::fs::metadata(&path).and_then(|m| m.modified()).map(secs).unwrap_or_else(|_| now());
                Entry::new(&rel, &hash, size, made, ticket)
            }
        };
        out.entries.insert(rel, entry);
    }
    for (rel, entry) in &previous.entries {
        if !out.entries.contains_key(rel) && !seed.contains(rel) {
            out.entries.insert(rel.clone(), entry.clone());
        }
    }
    out
}

/// What `materialize` did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Placed {
    pub placed: usize,
    /// Named by the manifest, but not in the store nor beside it: on another
    /// machine, for a sync to bring.
    pub missing: usize,
    /// The manifest's paths that are on disk now, as it says.
    pub present: BTreeSet<String>,
}

/// Put the captures of `manifest` that `checkout` lacks — or holds other
/// bytes of, where `changed` says the manifest changed them — in place from
/// the store; one the store lacks is taken from `fallback` (the main
/// checkout) when it holds those bytes there.
fn materialize(checkout: &Path, manifest: &Manifest, store: &Store, fallback: Option<&Path>, changed: &dyn Fn(&str) -> bool) -> Placed {
    let mut out = Placed::default();
    let design = checkout.join(super::DIR);
    for (rel, entry) in &manifest.entries {
        let to = design.join(rel);
        let there = cached_id(&to, store.kind).map(|(id, _)| id);
        if there.as_deref() == Some(entry.hash.as_str()) {
            out.present.insert(rel.clone());
            continue;
        }
        if there.is_some() && !changed(rel) {
            continue;
        }
        if !store.has(&entry.hash)
            && let Some(from) = fallback.map(|f| f.join(super::DIR).join(rel))
            && cached_id(&from, store.kind).is_some_and(|(id, _)| id == entry.hash)
        {
            let _ = store.put_file(&from, &entry.hash);
        }
        if !store.has(&entry.hash) {
            out.missing += 1;
            continue;
        }
        match store.place(entry, &to) {
            Ok(()) => {
                remember(&to, store.kind, &entry.hash);
                out.placed += 1;
                out.present.insert(rel.clone());
            }
            Err(err) => crate::daemon_log!("equipe: couldn't put the capture {rel} in {}: {err}", checkout.display()),
        }
    }
    out
}

// ── a ticket's worktree ──

/// Before equipe commits a worktree's work: `design/.gitignore` has equipe's
/// block, any capture git tracks is untracked (the file stays), and
/// `design/captures.json` lists what is on disk, each capture kept in the
/// store. Nothing when the worktree's design folder isn't one equipe keeps.
pub fn prepare_commit(worktree: &Path, branch: &str) -> Result<()> {
    if !applies(worktree) {
        return Ok(());
    }
    let store = Store::of(worktree).ok_or("not in a git repository")?;
    let ignore = worktree.join(IGNORE_PATH);
    let existing = std::fs::read_to_string(&ignore).unwrap_or_default();
    let wanted = with_block(&existing);
    if wanted != existing {
        std::fs::write(&ignore, wanted).map_err(|e| format!("{}: {e}", ignore.display()))?;
    }
    let tracked: Vec<String> = ls_files(worktree, None)?.into_iter().filter(|p| is_capture(p)).collect();
    if !tracked.is_empty() {
        untrack(worktree, &tracked, None)?;
    }
    let previous = manifest_on_disk(worktree).or_else(|| unmerged(worktree)).unwrap_or_else(|| manifest_at(worktree, "HEAD"));
    let manifest = from_disk(worktree, &store, &previous, &read_seed(worktree), ticket_of_branch(branch));
    let file = worktree.join(MANIFEST_PATH);
    if manifest.is_empty() && !file.exists() {
        return Ok(());
    }
    let text = manifest.render();
    if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(())
}

/// A merge stopped on the manifest in `worktree`: its three sides merged
/// entry by entry. `None` when it isn't unmerged.
fn unmerged(worktree: &Path) -> Option<Manifest> {
    let stage = |n: u8| worktree::git_bytes(worktree, &["show", &format!(":{n}:{MANIFEST_PATH}")], None, None).ok().and_then(|b| Manifest::parse(&String::from_utf8_lossy(&b)));
    let (ours, theirs) = (stage(2), stage(3));
    if ours.is_none() && theirs.is_none() {
        return None;
    }
    Some(merge3(&stage(1).unwrap_or_default(), &ours.unwrap_or_default(), &theirs.unwrap_or_default()))
}

/// A merge in `worktree` stopped on conflicts only in the manifest or in
/// captures: settled — the manifest merged entry by entry, the captures
/// untracked — and committed. `Ok(false)`: another file conflicts, nothing done.
pub fn settle_merge(worktree: &Path) -> Result<bool> {
    let conflicted = worktree::git(worktree, &["-c", "core.quotePath=false", "diff", "--name-only", "--diff-filter=U"])?;
    let files: Vec<&str> = conflicted.lines().filter(|l| !l.is_empty()).collect();
    if files.is_empty() || !files.iter().all(|f| *f == MANIFEST_PATH || is_capture(f)) {
        return Ok(false);
    }
    if let Some(merged) = unmerged(worktree) {
        let file = worktree.join(MANIFEST_PATH);
        std::fs::write(&file, merged.render()).map_err(|e| format!("{}: {e}", file.display()))?;
        worktree::git(worktree, &["add", "--", MANIFEST_PATH])?;
    }
    let captures: Vec<String> = files.iter().filter(|f| is_capture(f)).map(|f| (*f).to_owned()).collect();
    if !captures.is_empty() {
        untrack(worktree, &captures, None)?;
    }
    worktree::with_identity(worktree, &["commit", "-q", "--no-edit"])?;
    Ok(true)
}

/// Once a worktree of `repo` is checked out at `worktree`: the captures its
/// manifest names put in place, from the store or the main checkout, and
/// what it was given remembered.
pub fn after_checkout(repo: &Path, worktree: &Path) {
    if !applies(worktree) {
        return;
    }
    let Some(store) = Store::of(worktree) else { return };
    let Some(manifest) = manifest_on_disk(worktree) else { return };
    let main = worktree::main_branch(repo).ok().and_then(|m| worktree::checked_out_at(repo, &m)).unwrap_or_else(|| repo.to_owned());
    let placed = materialize(worktree, &manifest, &store, Some(&main), &|_| false);
    if placed.missing > 0 {
        crate::daemon_log!("equipe: {} captures named in {} aren't on this machine", placed.missing, worktree.join(MANIFEST_PATH).display());
    }
    write_seed(worktree, &placed.present);
}

/// The captures a worktree's branch changed since `base`, by path from the
/// top of the repository: what its hand-in shows beside the files it changed.
pub fn changed(worktree: &Path, base: &str) -> Vec<String> {
    let (before, after) = (manifest_at(worktree, base), manifest_at(worktree, "HEAD"));
    after.entries.values().filter(|e| before.hash_of(&e.path) != Some(e.hash.as_str())).map(|e| format!("{}/{}", super::DIR, e.path)).collect()
}

// ── main ──

/// What [`publish`] did in the main checkout.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Published {
    pub placed: usize,
    pub removed: usize,
    pub missing: usize,
}

/// Main moved from `before` to `after` (a merge landed): the captures its
/// manifest changed put in the checkout main is checked out in (else
/// `repo`'s own), those it removed taken away where nothing changed them
/// since — and any it names that the checkout lacks put there too.
pub fn publish(repo: &Path, main: &str, before: &str, after: &str) -> Result<Published> {
    let (old, new) = (manifest_at(repo, before), manifest_at(repo, after));
    if old.is_empty() && new.is_empty() {
        return Ok(Published::default());
    }
    let store = Store::of(repo).ok_or("not in a git repository")?;
    let checkout = worktree::checked_out_at(repo, main).unwrap_or_else(|| repo.to_owned());
    if !checkout.join(super::DIR).is_dir() {
        return Ok(Published::default());
    }
    let placed = materialize(&checkout, &new, &store, None, &|rel| old.hash_of(rel) != new.hash_of(rel));
    let mut out = Published { placed: placed.placed, missing: placed.missing, removed: 0 };
    for (rel, entry) in &old.entries {
        if new.entries.contains_key(rel) {
            continue;
        }
        let path = checkout.join(super::DIR).join(rel);
        if cached_id(&path, store.kind).is_some_and(|(id, _)| id == entry.hash) && std::fs::remove_file(&path).is_ok() {
            out.removed += 1;
        }
    }
    Ok(out)
}

/// What [`migrate`] did: one commit on main.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migrated {
    pub commit: String,
    /// Captures no longer tracked, and their size.
    pub untracked: usize,
    pub bytes: u64,
}

/// One entry of `git ls-tree -r -l -z`: path, blob id, size.
fn tree_blobs(repo: &Path, rev: &str, under: &str) -> Result<Vec<(String, String, u64)>> {
    let out = worktree::git_bytes(repo, &["-c", "core.quotePath=false", "ls-tree", "-r", "-l", "-z", "--full-tree", rev, "--", under], None, None)?;
    let mut blobs = Vec::new();
    for item in out.split(|b| *b == 0).filter(|i| !i.is_empty()) {
        let item = String::from_utf8_lossy(item);
        let Some((head, path)) = item.split_once('\t') else { continue };
        let fields: Vec<&str> = head.split_whitespace().collect();
        if let [_, "blob", id, size] = fields[..] {
            blobs.push((path.to_owned(), id.to_owned(), size.parse().unwrap_or_default()));
        }
    }
    Ok(blobs)
}

/// The files `index` (else the checkout's own) lists, by path.
fn ls_files(dir: &Path, index: Option<&Path>) -> Result<Vec<String>> {
    let out = worktree::git_bytes(dir, &["-c", "core.quotePath=false", "ls-files", "-z"], index, None)?;
    Ok(out.split(|b| *b == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).collect())
}

/// `paths` out of `index` (else the checkout's own), the files left alone.
fn untrack(dir: &Path, paths: &[String], index: Option<&Path>) -> Result<()> {
    let list = scratch_file(dir, "untrack", paths.join("\0").as_bytes())?;
    let done = worktree::git_bytes(dir, &["update-index", "--force-remove", "-z", "--stdin"], index, Some(&list)).map(|_| ());
    let _ = std::fs::remove_file(&list);
    done
}

/// A file of ours holding `bytes`, in the store's scratch folder.
fn scratch_file(dir: &Path, what: &str, bytes: &[u8]) -> Result<PathBuf> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = Store::of(dir).map(|s| s.root.join("tmp")).unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    let path = root.join(format!("{what}-{}-{n}", std::process::id()));
    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Write `bytes` into `repo` as a blob; its id.
fn write_blob(repo: &Path, bytes: &[u8]) -> Result<String> {
    let file = scratch_file(repo, "blob", bytes)?;
    let id = worktree::git(repo, &["hash-object", "-w", "--", &file.to_string_lossy()]);
    let _ = std::fs::remove_file(&file);
    id
}

/// A scratch index of `repo`, read from `tree`, changed by `edit` and
/// written back: the new tree's id.
fn edit_tree(repo: &Path, tree: &str, edit: impl FnOnce(&Path) -> Result<()>) -> Result<String> {
    let index = scratch_file(repo, "index", b"")?;
    let _ = std::fs::remove_file(&index);
    let done = (|| {
        worktree::git_bytes(repo, &["read-tree", tree], Some(&index), None)?;
        edit(&index)?;
        let out = worktree::git_bytes(repo, &["write-tree"], Some(&index), None)?;
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    })();
    let _ = std::fs::remove_file(&index);
    done
}

/// Put `bytes` at `rel` in the scratch `index`.
fn set_in_index(repo: &Path, index: &Path, rel: &str, bytes: &[u8]) -> Result<()> {
    let blob = write_blob(repo, bytes)?;
    worktree::git_bytes(repo, &["update-index", "--add", "--cacheinfo", &format!("100644,{blob},{rel}")], Some(index), None).map(|_| ())
}

/// Main stops tracking its captures: one commit, made apart from any
/// checkout, that untracks them, puts equipe's block in `design/.gitignore`
/// and lists them in `design/captures.json`; each is kept in the store. The
/// checkout main is checked out in follows (fast-forward) and keeps every
/// file on disk. History stays as it was. `Ok(None)` when there is nothing
/// to do — done before, or main has no design folder equipe keeps.
pub fn migrate(repo: &Path) -> Result<Option<Migrated>> {
    let main = worktree::main_branch(repo)?;
    let tip = worktree::git(repo, &["rev-parse", &format!("refs/heads/{main}")])?;
    if !tree_applies(repo, &tip) {
        return Ok(None);
    }
    let tracked: Vec<(String, String, u64)> = tree_blobs(repo, &tip, super::DIR)?.into_iter().filter(|(p, ..)| is_capture(p)).collect();
    let ignore_now = worktree::git_bytes(repo, &["show", &format!("{tip}:{IGNORE_PATH}")], None, None).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let ignore = with_block(&ignore_now);
    if tracked.is_empty() && ignore == ignore_now {
        return Ok(None);
    }
    let store = Store::of(repo).ok_or("not in a git repository")?;
    let checkout = worktree::checked_out_at(repo, &main);
    let committed: u64 = worktree::git(repo, &["log", "-1", "--format=%ct", &tip])?.parse().unwrap_or_else(|_| now());
    let mut manifest = manifest_at(repo, &tip);
    for (path, id, size) in &tracked {
        let on_disk = checkout.as_ref().map(|c| c.join(path)).filter(|p| cached_id(p, store.kind).is_some_and(|(h, _)| h == *id));
        let kept = match &on_disk {
            Some(file) => store.put_file(file, id),
            None => store.put_blob(repo, id),
        };
        if let Err(err) = kept {
            return Err(format!("couldn't keep {path} in the captures' store, so nothing was untracked: {err}"));
        }
        let made = on_disk.and_then(|f| std::fs::metadata(f).and_then(|m| m.modified()).ok()).map(secs).unwrap_or(committed);
        let rel = path.strip_prefix("design/").unwrap_or(path);
        if manifest.hash_of(rel) != Some(id.as_str()) {
            manifest.entries.insert(rel.to_owned(), Entry::new(rel, id, *size, made, None));
        }
    }
    let tree = worktree::git(repo, &["rev-parse", &format!("{tip}^{{tree}}")])?;
    let paths: Vec<String> = tracked.iter().map(|(p, ..)| p.clone()).collect();
    let new_tree = edit_tree(repo, &tree, |index| {
        if !paths.is_empty() {
            untrack(repo, &paths, Some(index))?;
        }
        set_in_index(repo, index, IGNORE_PATH, ignore.as_bytes())?;
        if !manifest.is_empty() {
            set_in_index(repo, index, MANIFEST_PATH, manifest.render().as_bytes())?;
        }
        Ok(())
    })?;
    let bytes: u64 = tracked.iter().map(|(_, _, s)| s).sum();
    let message = format!(
        "equipe: keep design captures out of git\n\n{} pictures and videos ({}) under design/ stop being tracked: they stay on disk and in\nequipe's store, listed in design/captures.json. History is unchanged; equipe's\ndocs/DESIGN-CAPTURES.md says how a person can shrink it.",
        tracked.len(),
        size_words(bytes)
    );
    let commit = worktree::with_identity(repo, &["commit-tree", &new_tree, "-p", &tip, "-m", &message])?;
    // A fast-forward deletes the files main stops tracking: each is held by
    // a second link meanwhile and put back.
    let held = checkout.as_ref().map(|c| hold(&store, c, &paths)).unwrap_or_default();
    let moved = worktree::move_main(repo, &main, &tip, &commit, "untracked");
    for (path, at) in held {
        if std::fs::symlink_metadata(&path).is_err() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::rename(&at, &path).is_ok() {
                continue;
            }
        }
        let _ = std::fs::remove_file(&at);
    }
    moved?;
    Ok(Some(Migrated { commit, untracked: tracked.len(), bytes }))
}

/// A second link (else a copy) of each of `paths` in `checkout`, in the
/// store's holding folder: (the file, the link).
fn hold(store: &Store, checkout: &Path, paths: &[String]) -> Vec<(PathBuf, PathBuf)> {
    let dir = store.root.join("hold").join(format!("{}-{}", std::process::id(), now()));
    if std::fs::create_dir_all(&dir).is_err() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (n, rel) in paths.iter().enumerate() {
        let path = checkout.join(rel);
        if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            continue;
        }
        let at = dir.join(n.to_string());
        if std::fs::hard_link(&path, &at).is_ok() || std::fs::copy(&path, &at).is_ok() {
            out.push((path, at));
        }
    }
    out
}

// ── landing ──

/// A commit that adds many images to git.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heavy {
    pub commit: String,
    pub subject: String,
    pub bytes: u64,
}

/// What landing a branch kept out of git, for its member to hear.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// Captures its commits added, taken out of them (still on disk, kept in
    /// the store, listed in the manifest), and their size.
    pub kept_out: usize,
    pub bytes: u64,
    pub heavy: Vec<Heavy>,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        self.kept_out == 0 && self.heavy.is_empty()
    }

    /// What its member is told, in English.
    pub fn line(&self, ticket: u64) -> String {
        let mut out = Vec::new();
        if self.kept_out > 0 {
            out.push(format!(
                "When T-{ticket} landed, equipe took {} design pictures or videos ({}) out of its commits: captures under design/ (the UI map, QA, checks, mockups, directions, references…) never go into git. Leave them on disk and never `git add` them, not with -f either: equipe lists them in design/captures.json and keeps them in its store when it commits your work.",
                self.kept_out,
                size_words(self.bytes)
            ));
        }
        for heavy in &self.heavy {
            out.push(format!(
                "T-{ticket}'s commit {} (\"{}\") adds {} of images to git. Keep big pictures out of the repository: a capture goes under design/, where equipe keeps it out of git.",
                &heavy.commit[..heavy.commit.len().min(10)],
                heavy.subject,
                size_words(heavy.bytes)
            ));
        }
        out.join("\n")
    }
}

/// `bytes` as a person reads them: `1.2 GB`, `340 KB`.
pub fn size_words(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

/// Before `branch` merges into main: main stops tracking captures if it
/// hasn't yet ([`migrate`]); the branch's own commits — those main lacks —
/// that add captures are rewritten without them, each kept in the store and
/// listed in the branch's manifest, so none reaches main's history; a
/// commit adding more than [`HEAVY_COMMIT`] of other images is noted. What
/// the member should hear is kept for [`take_report`]. Only a local branch
/// other than main; nothing for a remote's.
pub fn guard(repo: &Path, branch: &str) -> Result<Report> {
    let main = worktree::main_branch(repo)?;
    let local = format!("refs/heads/{branch}");
    if branch == main || worktree::git(repo, &["rev-parse", "--verify", "-q", &local]).is_err() {
        return Ok(Report::default());
    }
    // Copying a big map into the store is no work for the daemon's loop
    // (a task landing in the squad's folder): its hourly sweep does it then.
    if !crate::runtime::on_core_loop()
        && let Err(err) = migrate(repo)
    {
        crate::daemon_log!("equipe: {} still tracks design captures: {err}", repo.display());
    }
    let tip = worktree::git(repo, &["rev-parse", &format!("refs/heads/{main}")])?;
    let head = worktree::git(repo, &["rev-parse", &local])?;
    let range = format!("{tip}..{head}");
    let names = |args: &[&str]| -> Result<Vec<String>> {
        let out = worktree::git_bytes(repo, args, None, None)?;
        Ok(out.split(|b| *b == 0 || *b == b'\n').filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).filter(|p| is_capture(p)).collect())
    };
    let mut strip: BTreeSet<String> = names(&["-c", "core.quotePath=false", "log", "--no-merges", "--format=", "--name-only", "-z", "--diff-filter=AMT", &range])?.into_iter().collect();
    strip.extend(names(&["-c", "core.quotePath=false", "diff", "--name-only", "-z", "--diff-filter=AMT", &format!("{tip}...{head}")])?);
    let mut report = Report::default();
    let mut new_head = head.clone();
    if !strip.is_empty() {
        let store = Store::of(repo).ok_or("not in a git repository")?;
        let at_head: Vec<(String, String, u64)> = tree_blobs(repo, &head, super::DIR)?.into_iter().filter(|(p, ..)| strip.contains(p)).collect();
        let committed: u64 = worktree::git(repo, &["log", "-1", "--format=%ct", &head])?.parse().unwrap_or_else(|_| now());
        let ticket = ticket_of_branch(branch);
        let mut manifest = manifest_at(repo, &head);
        for (path, id, size) in &at_head {
            store.put_blob(repo, id).map_err(|e| format!("couldn't keep {path} in the captures' store, so {branch} wasn't merged: {e}"))?;
            let rel = path.strip_prefix("design/").unwrap_or(path);
            if manifest.hash_of(rel) != Some(id.as_str()) {
                manifest.entries.insert(rel.to_owned(), Entry::new(rel, id, *size, committed, ticket));
            }
        }
        report.kept_out = at_head.len();
        report.bytes = at_head.iter().map(|(_, _, s)| s).sum();
        new_head = rewrite(repo, &tip, &head, &strip, &manifest)?;
        let checked_out = worktree::checked_out_at(repo, branch);
        worktree::git(repo, &["update-ref", &local, &new_head, &head])?;
        if let Some(worktree) = checked_out {
            // Its index follows the branch; the files stay as they are, and
            // the two equipe wrote match what is committed.
            let _ = worktree::git(&worktree, &["reset", "-q"]);
            for rel in [MANIFEST_PATH, IGNORE_PATH] {
                if let Ok(bytes) = worktree::git_bytes(&worktree, &["show", &format!("HEAD:{rel}")], None, None) {
                    let _ = std::fs::write(worktree.join(rel), bytes);
                }
            }
        }
    }
    report.heavy = heavy_commits(repo, &tip, &new_head)?;
    if !report.is_empty()
        && let Some(store) = Store::of(repo)
    {
        let path = store.report_path(branch);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, serde_json::to_vec(&report).unwrap_or_default());
    }
    Ok(report)
}

/// What [`guard`] kept for `branch`'s member, once: taken away as it is read.
pub fn take_report(repo: &Path, branch: &str) -> Option<Report> {
    let path = Store::of(repo)?.report_path(branch);
    let report = serde_json::from_slice(&std::fs::read(&path).ok()?).ok();
    let _ = std::fs::remove_file(&path);
    report.filter(|r: &Report| !r.is_empty())
}

/// The commits of `tip..head` again, without `strip`'s paths, and with
/// `manifest` and equipe's ignore block in the last one; their authors,
/// times and messages as they were (a signature can't stay). Commits that
/// lose nothing, and whose parents didn't change, stay as they are. The new head.
fn rewrite(repo: &Path, tip: &str, head: &str, strip: &BTreeSet<String>, manifest: &Manifest) -> Result<String> {
    let listed = worktree::git(repo, &["rev-list", "--reverse", "--topo-order", "--parents", &format!("{tip}..{head}")])?;
    let mut map: HashMap<String, String> = HashMap::new();
    for line in listed.lines().filter(|l| !l.is_empty()) {
        let mut ids = line.split_whitespace();
        let Some(commit) = ids.next() else { continue };
        let parents: Vec<String> = ids.map(str::to_owned).collect();
        let new_parents: Vec<String> = parents.iter().map(|p| map.get(p).cloned().unwrap_or_else(|| p.clone())).collect();
        let tree = worktree::git(repo, &["rev-parse", &format!("{commit}^{{tree}}")])?;
        let last = commit == head;
        let new_tree = edit_tree(repo, &tree, |index| {
            let gone: Vec<String> = ls_files(repo, Some(index))?.into_iter().filter(|p| strip.contains(p)).collect();
            if !gone.is_empty() {
                untrack(repo, &gone, Some(index))?;
            }
            if last {
                let ignore = worktree::git_bytes(repo, &["show", &format!("{commit}:{IGNORE_PATH}")], None, None).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
                set_in_index(repo, index, IGNORE_PATH, with_block(&ignore).as_bytes())?;
                set_in_index(repo, index, MANIFEST_PATH, manifest.render().as_bytes())?;
            }
            Ok(())
        })?;
        if new_tree == tree && new_parents == parents {
            map.insert(commit.to_owned(), commit.to_owned());
            continue;
        }
        let raw = worktree::git_bytes(repo, &["cat-file", "commit", commit], None, None)?;
        let raw = String::from_utf8_lossy(&raw).into_owned();
        let (headers, message) = raw.split_once("\n\n").unwrap_or((raw.as_str(), ""));
        let mut out = format!("tree {new_tree}\n");
        for parent in &new_parents {
            out.push_str(&format!("parent {parent}\n"));
        }
        let mut skipping = false;
        for header in headers.lines() {
            if header.starts_with(' ') {
                if !skipping {
                    out.push_str(header);
                    out.push('\n');
                }
                continue;
            }
            skipping = header.starts_with("tree ") || header.starts_with("parent ") || header.starts_with("gpgsig") || header.starts_with("mergetag ");
            if !skipping {
                out.push_str(header);
                out.push('\n');
            }
        }
        out.push('\n');
        out.push_str(message);
        let file = scratch_file(repo, "commit", out.as_bytes())?;
        let made = worktree::git(repo, &["hash-object", "-t", "commit", "-w", "--", &file.to_string_lossy()]);
        let _ = std::fs::remove_file(&file);
        map.insert(commit.to_owned(), made?);
    }
    Ok(map.get(head).cloned().unwrap_or_else(|| head.to_owned()))
}

/// The commits of `tip..head` that each add more than [`HEAVY_COMMIT`] of
/// pictures or videos to git.
fn heavy_commits(repo: &Path, tip: &str, head: &str) -> Result<Vec<Heavy>> {
    let out = worktree::git(repo, &["-c", "core.quotePath=false", "log", "--no-merges", "--raw", "--no-abbrev", "--no-renames", "--diff-filter=AM", "--format=%x1e%H%x1f%s", &format!("{tip}..{head}")])?;
    let mut commits: Vec<(String, String, Vec<String>)> = Vec::new();
    for record in out.split('\u{1e}').filter(|r| !r.trim().is_empty()) {
        let mut lines = record.lines();
        let Some((commit, subject)) = lines.next().and_then(|l| l.split_once('\u{1f}')) else { continue };
        // `:100644 100644 <old> <new> A\t<path>`
        let blobs: Vec<String> = lines
            .filter_map(|l| l.strip_prefix(':'))
            .filter_map(|l| l.split_once('\t'))
            .filter(|(_, path)| path.rsplit('/').next().is_some_and(is_media_name))
            .filter_map(|(head, _)| head.split_whitespace().nth(3).map(str::to_owned))
            .collect();
        if !blobs.is_empty() {
            commits.push((commit.to_owned(), subject.to_owned(), blobs));
        }
    }
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<&str> = commits.iter().flat_map(|(_, _, b)| b.iter().map(String::as_str)).collect();
    let list = scratch_file(repo, "sizes", format!("{}\n", ids.join("\n")).as_bytes())?;
    let sizes = worktree::git_bytes(repo, &["cat-file", "--batch-check=%(objectname) %(objectsize)"], None, Some(&list));
    let _ = std::fs::remove_file(&list);
    let sizes: HashMap<String, u64> = String::from_utf8_lossy(&sizes?).lines().filter_map(|l| l.split_once(' ')).filter_map(|(id, size)| Some((id.to_owned(), size.parse().ok()?))).collect();
    Ok(commits
        .into_iter()
        .filter_map(|(commit, subject, blobs)| {
            let bytes: u64 = blobs.iter().filter_map(|b| sizes.get(b)).sum();
            (bytes > HEAVY_COMMIT).then_some(Heavy { commit, subject, bytes })
        })
        .collect())
}

/// Conflicts between main's tree and a branch's that only the manifest and
/// captures have: the tree git's merge made (`merged`, its conflicted files
/// marked) with the manifest merged entry by entry and the captures out.
/// `None` when another file conflicts.
pub fn settle_tree(repo: &Path, merged: &str, tip: &str, head: &str, conflicted: &[&str]) -> Option<String> {
    if conflicted.is_empty() || !conflicted.iter().all(|f| *f == MANIFEST_PATH || is_capture(f)) {
        return None;
    }
    let base = worktree::git(repo, &["merge-base", tip, head]).ok();
    let manifest = merge3(&base.map(|b| manifest_at(repo, &b)).unwrap_or_default(), &manifest_at(repo, tip), &manifest_at(repo, head));
    let captures: Vec<String> = conflicted.iter().filter(|f| is_capture(f)).map(|f| (*f).to_owned()).collect();
    edit_tree(repo, merged, |index| {
        let listed: BTreeSet<String> = ls_files(repo, Some(index))?.into_iter().collect();
        let gone: Vec<String> = captures.iter().filter(|c| listed.contains(*c)).cloned().collect();
        if !gone.is_empty() {
            untrack(repo, &gone, Some(index))?;
        }
        if conflicted.contains(&MANIFEST_PATH) {
            set_in_index(repo, index, MANIFEST_PATH, manifest.render().as_bytes())?;
        }
        Ok(())
    })
    .ok()
}

/// Whether a file merging reports conflicting is one equipe settles itself.
pub fn settles_itself(path: &str) -> bool {
    path == MANIFEST_PATH || is_capture(path)
}

// ── keeping the store small ──

/// Objects of `repo`'s store no manifest names any more — not at any local
/// branch's tip, nor on main in the last month — and older than a week go,
/// as do leftovers of work that stopped half-way. What went, a line.
pub fn gc(repo: &Path) -> Vec<String> {
    let Some(store) = Store::of(repo) else { return Vec::new() };
    let objects = store.root.join("objects");
    if !objects.is_dir() {
        return Vec::new();
    }
    let mut named: BTreeSet<String> = BTreeSet::new();
    let tips = worktree::git(repo, &["for-each-ref", "--format=%(objectname)", "refs/heads"]).unwrap_or_default();
    let mut revs: Vec<String> = tips.lines().map(str::to_owned).collect();
    if let Ok(main) = worktree::main_branch(repo) {
        let recent = worktree::git(repo, &["log", "--since=30.days", "--format=%H", &format!("refs/heads/{main}"), "--", MANIFEST_PATH]).unwrap_or_default();
        revs.extend(recent.lines().map(str::to_owned));
    }
    for rev in revs.iter().filter(|r| !r.is_empty()) {
        named.extend(manifest_at(repo, rev).entries.into_values().map(|e| e.hash));
    }
    // Worktrees' manifests on disk: work committed soon.
    for checkout in worktree::listed(repo) {
        if let Some(m) = manifest_on_disk(&checkout) {
            named.extend(m.entries.into_values().map(|e| e.hash));
        }
    }
    let cutoff = now().saturating_sub(UNNAMED_KEPT);
    let mut gone = (0usize, 0u64);
    for dir in std::fs::read_dir(&objects).into_iter().flatten().flatten() {
        let prefix = dir.file_name().to_string_lossy().into_owned();
        for file in std::fs::read_dir(dir.path()).into_iter().flatten().flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            let Ok(meta) = file.metadata() else { continue };
            let old = meta.modified().map(secs).unwrap_or_default() < cutoff;
            let hash = format!("{prefix}{name}");
            if old && (name.starts_with('.') || !named.contains(&hash)) && std::fs::remove_file(file.path()).is_ok() {
                gone = (gone.0 + 1, gone.1 + meta.len());
            }
        }
    }
    for leftover in ["tmp", "hold"] {
        for entry in std::fs::read_dir(store.root.join(leftover)).into_iter().flatten().flatten() {
            if entry.metadata().and_then(|m| m.modified()).map(secs).unwrap_or_default() < now().saturating_sub(86_400) {
                let _ = std::fs::remove_dir_all(entry.path()).or_else(|_| std::fs::remove_file(entry.path()));
            }
        }
    }
    if gone.0 == 0 { Vec::new() } else { vec![format!("{} captures no manifest names any more ({}) left {}'s store", gone.0, size_words(gone.1), repo.display())] }
}

#[cfg(test)]
mod tests;
