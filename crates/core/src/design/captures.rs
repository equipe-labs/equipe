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
//! - **The main checkout holds them on disk where they always were**, so
//!   every screen and tool finds them at the same paths: a merge onto main
//!   puts what it changed there ([`publish`]). **A ticket's worktree holds
//!   only its own**: what its work made, and what the work it builds on made
//!   ([`after_checkout`]); the rest — unchanged from main — is shown from the
//!   main checkout ([`borrowed`], [`lent`]), so a worktree costs nothing for a
//!   big map.
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
//!
//! The same mechanism keeps what tasks hand over in a squad's folder out of
//! its history ([`OUTPUTS`]): pictures, videos and other generated files
//! under `outputs/` — gigabytes of them filled one squad's folder's git — are
//! listed in `outputs/.captures.json`, their bytes in the same store. Each
//! place is a [`Scope`]; [`DESIGN`] is the design folder's.

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
/// What a squad's folder keeps out of git under `outputs/`, by extension:
/// pictures, videos, sound, and the archives and documents tools generate —
/// office files and SVG drawings too: people open them, but git diffs none of
/// them, and a hand-in's few files in git are what people read ([`raw`]).
pub const OUTPUT_EXTS: [&str; 42] = [
    "png", "jpg", "jpeg", "webp", "gif", "heic", "avif", "tif", "tiff", "bmp", "psd", "svg", "mp4", "webm", "mov", "m4v", "mkv", "avi", "mp3", "wav", "m4a", "aac", "flac", "zip",
    "tgz", "7z", "pdf", "doc", "docx", "xls", "xlsx", "xlsm", "ppt", "pptx", "odt", "ods", "odp", "rtf", "key", "numbers", "pages", "epub",
];
/// The manifest's name in the design folder, and its path in a repository.
pub const MANIFEST: &str = "captures.json";
pub const MANIFEST_PATH: &str = "design/captures.json";
/// The ignore file equipe keeps its block in.
pub const IGNORE_PATH: &str = "design/.gitignore";
const BLOCK_START: &str = "# >>> equipe: design captures stay out of git";
const BLOCK_END: &str = "# <<< equipe: design captures";

/// A place in a repository whose generated files equipe keeps out of git:
/// on disk where they are, in its store, listed in a manifest beside them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    /// The folder, from the top of the repository.
    pub dir: &'static str,
    /// Its folders whose files are kept out; none: every file in it, at any depth.
    pub dirs: &'static [&'static str],
    /// What is kept out, by extension (any case).
    pub exts: &'static [&'static str],
    /// The manifest's name in the folder, and its path in a repository.
    pub manifest: &'static str,
    pub manifest_path: &'static str,
    /// The ignore file equipe keeps its block in.
    pub ignore_path: &'static str,
    block_start: &'static str,
    block_end: &'static str,
    /// What the block says it is, under its first line.
    block_about: &'static str,
    /// In a worktree's admin folder: what it was given when made ([`SEED`]).
    seed: &'static str,
    /// What its files are called, in the log.
    pub what: &'static str,
}

/// The design folder's captures (the module's doc).
pub const DESIGN: Scope = Scope {
    dir: super::DIR,
    dirs: &DIRS,
    exts: &EXTS,
    manifest: MANIFEST,
    manifest_path: MANIFEST_PATH,
    ignore_path: IGNORE_PATH,
    block_start: BLOCK_START,
    block_end: BLOCK_END,
    block_about: "# Pictures and videos equipe's design tools make stay on disk and in equipe's store;\n# captures.json lists them. equipe rewrites this block: edit outside it.\n",
    seed: SEED,
    what: "design captures",
};

/// What tasks hand over in a squad's folder (`outputs/T-12 …/`): its
/// pictures, videos and other generated files. Only in a squad's folder —
/// a project's own `outputs/` is left as it is.
pub const OUTPUTS: Scope = Scope {
    dir: "outputs",
    dirs: &[],
    exts: &OUTPUT_EXTS,
    manifest: ".captures.json",
    manifest_path: "outputs/.captures.json",
    ignore_path: "outputs/.gitignore",
    block_start: "# >>> equipe: handed-in pictures and files stay out of git",
    block_end: "# <<< equipe: handed-in pictures and files",
    block_about: "# Pictures, videos and other generated files tasks hand over stay in the folder and in\n# equipe's store; .captures.json lists them. equipe rewrites this block: edit outside it.\n",
    seed: "equipe-outputs-seed",
    what: "handed-in files",
};

/// Every place equipe keeps generated files out of git.
pub const SCOPES: [&Scope; 2] = [&DESIGN, &OUTPUTS];

impl Scope {
    /// Whether `rel` — a path from the top of a repository, `/`-separated —
    /// is one of this place's files.
    pub fn keeps(&self, rel: &str) -> bool {
        let parts: Vec<&str> = rel.split('/').collect();
        let deep = if self.dirs.is_empty() { parts.len() >= 2 } else { parts.len() >= 3 && self.dirs.contains(&parts[1]) };
        deep && parts[0] == self.dir && parts.iter().all(|p| !p.is_empty() && *p != "..") && self.keeps_name(parts[parts.len() - 1])
    }

    /// Whether a file's name is of a kind this place keeps out of git.
    pub fn keeps_name(&self, name: &str) -> bool {
        name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && self.exts.iter().any(|e| e.eq_ignore_ascii_case(ext)))
    }

    /// The block equipe keeps in this place's ignore file.
    pub fn ignore_block(&self) -> String {
        let mut out = format!("{}\n{}", self.block_start, self.block_about);
        if self.dirs.is_empty() {
            for ext in self.exts {
                out.push_str(&format!("*.{}\n", any_case(ext)));
            }
        }
        for dir in self.dirs {
            for ext in self.exts {
                out.push_str(&format!("/{dir}/**/*.{}\n", any_case(ext)));
            }
        }
        out.push_str(self.block_end);
        out.push('\n');
        out
    }

    /// `existing` (an ignore file's text) with equipe's block in it, as it
    /// is now: replaced where it was, else added at the end. The rest is kept.
    pub fn with_block(&self, existing: &str) -> String {
        put_block(existing, self.block_start, self.block_end, &self.ignore_block())
    }

    /// Whether this place is one equipe keeps in `checkout`.
    fn applies(&self, checkout: &Path) -> bool {
        if self.dir == DESIGN.dir { applies(checkout) } else { in_squad_folder(checkout) }
    }

    /// [`Self::applies`], of the tree at `rev` rather than a checkout.
    fn tree_applies(&self, repo: &Path, rev: &str) -> bool {
        if self.dir == DESIGN.dir { tree_applies(repo, rev) } else { in_squad_folder(repo) }
    }

    /// The manifest's entry for the file at `path` (in this place's folder).
    fn entry(&self, path: &str, hash: &str, size: u64, created_at: u64, ticket: Option<u64>) -> Entry {
        if self.dir == DESIGN.dir {
            Entry::new(path, hash, size, created_at, ticket)
        } else {
            Entry { path: path.to_owned(), hash: hash.to_owned(), size, created_at, ticket, ..Entry::default() }
        }
    }

    /// `path` from the top of the repository, of one in this place's folder.
    fn full(&self, path: &str) -> String {
        format!("{}/{path}", self.dir)
    }

    /// `rel` from the top of the repository, in this place's folder.
    fn inside<'a>(&self, rel: &'a str) -> Option<&'a str> {
        rel.strip_prefix(self.dir)?.strip_prefix('/')
    }
}

/// `existing` (a file's text) with `block` — which starts with the line
/// `start` and ends with `end` — in place of the one it has, else added at
/// the end after a blank line. An empty `block` takes the one it has away.
/// The rest is kept.
fn put_block(existing: &str, start: &str, end: &str, block: &str) -> String {
    if let Some(at) = existing.find(start)
        && let Some(len) = existing[at..].find(end)
    {
        let mut to = at + len + end.len();
        if existing[to..].starts_with('\n') {
            to += 1;
        }
        let mut before = existing[..at].to_owned();
        if block.is_empty() && before.ends_with("\n\n") {
            before.pop();
        }
        return format!("{before}{block}{}", &existing[to..]);
    }
    if block.is_empty() {
        return existing.to_owned();
    }
    let mut out = existing.to_owned();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(block);
    out
}

/// Whether `dir` is a squad's folder (`…/squads/<squad>/home`) or a worktree of one.
fn in_squad_folder(dir: &Path) -> bool {
    worktree::is_squad_home(&main_checkout_of(dir).unwrap_or_else(|| dir.to_owned()))
}

/// The places of `repo` (a checkout, or a repository's main checkout) that
/// equipe keeps: the design folder's anywhere, a squad folder's hand-ins there.
fn scopes_of(repo: &Path) -> Vec<&'static Scope> {
    SCOPES.iter().copied().filter(|s| s.dir == DESIGN.dir || in_squad_folder(repo)).collect()
}
/// The manifest's format.
pub const VERSION: u32 = 1;
/// One commit adding more images than this to git is reported at landing.
pub const HEAVY_COMMIT: u64 = 10 << 20;
/// The store, in a repository's common git folder.
const STORE: &str = "equipe/captures";
/// In a worktree's admin folder: the captures it was given when made, one
/// path a line — what it lacks later, its member removed.
const SEED: &str = "equipe-captures-seed";
/// How long the checkout main is checked out in has to follow the upgrade's
/// commit: a squad's folder that tracked 130,000 handed-in files took more
/// than git's usual minute on a busy machine, failed, and tried again on
/// every sweep. It runs off the daemon's loop.
const MIGRATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);
const DEPTH_MAX: usize = 8;
const FILES_MAX: usize = 200_000;
/// Objects no branch's manifest names are kept this long (seconds): work
/// not committed yet may name them soon.
const UNNAMED_KEPT: u64 = 7 * 86_400;

/// An object's time is when it came into the store, which [`gc`] counts
/// from: a copy keeps its file's time, and an upgrade that failed half-way
/// saw every file it had kept — months old — go again at once, to be copied
/// and hashed anew on the next sweep (7 GB each time, in one squad's folder).
fn stored_now(object: &Path) {
    if let Ok(file) = std::fs::File::options().write(true).open(object) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// Whether `rel` — a path from the top of a repository, `/`-separated — is
/// a capture: `design/<one of DIRS>/…/<name>.<one of EXTS>`.
pub fn is_capture(rel: &str) -> bool {
    DESIGN.keeps(rel)
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
    DESIGN.ignore_block()
}

/// `existing` (an ignore file's text) with equipe's block in it, as it is
/// now: replaced where it was, else added at the end. The rest is kept.
pub fn with_block(existing: &str) -> String {
    DESIGN.with_block(existing)
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
        Manifest::parse_in(&DESIGN, text)
    }

    /// [`Manifest::parse`], of `scope`'s manifest.
    pub fn parse_in(scope: &Scope, text: &str) -> Option<Manifest> {
        let file: ManifestFile = serde_json::from_str(text).ok()?;
        let entries = file.captures.into_iter().filter(|e| scope.keeps(&scope.full(&e.path)) && is_hash(&e.hash)).map(|e| (e.path.clone(), e)).collect();
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
    manifest_at_in(&DESIGN, repo, rev)
}

/// [`manifest_at`], of `scope`'s manifest.
pub fn manifest_at_in(scope: &Scope, repo: &Path, rev: &str) -> Manifest {
    worktree::git_bytes(repo, &["show", &format!("{rev}:{}", scope.manifest_path)], None, None).ok().and_then(|b| Manifest::parse_in(scope, &String::from_utf8_lossy(&b))).unwrap_or_default()
}

/// `scope`'s manifest in `checkout`, on disk.
fn manifest_on_disk(scope: &Scope, checkout: &Path) -> Option<Manifest> {
    Manifest::parse_in(scope, &std::fs::read_to_string(checkout.join(scope.manifest_path)).ok()?)
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
                Ok((id, _)) if id == hash => {
                    stored_now(&staging);
                    std::fs::rename(&staging, &to).map_err(|e| format!("{}: {e}", to.display()))
                }
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
            Ok((id, _)) if id == hash => {
                stored_now(&staging);
                std::fs::rename(&staging, &to).map_err(|e| format!("{}: {e}", to.display()))
            }
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
    on_disk_in(&DESIGN, checkout)
}

/// [`on_disk`], of `scope`'s folder: path in that folder, and where.
pub fn on_disk_in(scope: &Scope, checkout: &Path) -> Vec<(String, PathBuf)> {
    let root = checkout.join(scope.dir);
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, String, usize)> =
        if scope.dirs.is_empty() { vec![(root.clone(), String::new(), 0)] } else { scope.dirs.iter().map(|d| (root.join(d), (*d).to_owned(), 0)).collect() };
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
            let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            if kind.is_dir() && depth < DEPTH_MAX {
                stack.push((entry.path(), path, depth + 1));
            } else if kind.is_file() && scope.keeps_name(&name) {
                out.push((path, entry.path()));
            }
        }
    }
    out.sort();
    out
}

/// The paths of `scope` a worktree was given when made ([`SEED`]).
fn read_seed(scope: &Scope, worktree: &Path) -> BTreeSet<String> {
    worktree::admin_dir(worktree).and_then(|a| std::fs::read_to_string(a.join(scope.seed)).ok()).map(|t| t.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect()).unwrap_or_default()
}

fn write_seed(scope: &Scope, worktree: &Path, paths: &BTreeSet<String>) {
    if let Some(admin) = worktree::admin_dir(worktree) {
        let text: String = paths.iter().map(|p| format!("{p}\n")).collect();
        let _ = std::fs::write(admin.join(scope.seed), text);
    }
}

/// The manifest of what is on disk in `checkout`, each capture kept in
/// `store`. An entry of `previous` with the same bytes keeps its time and
/// ticket; a new or changed one is made now by `ticket`. One of `previous`
/// no longer on disk goes only when `seed` says the checkout had it — its
/// member removed it; else it was never brought here, and stays.
fn from_disk(scope: &Scope, checkout: &Path, store: &Store, previous: &Manifest, seed: &BTreeSet<String>, ticket: Option<u64>) -> Manifest {
    let mut out = Manifest::default();
    for (rel, path) in on_disk_in(scope, checkout) {
        let Some((hash, size)) = cached_id(&path, store.kind) else { continue };
        if let Err(err) = store.put_file(&path, &hash) {
            crate::daemon_log!("equipe: couldn't keep {rel} in the captures' store: {err}");
            continue;
        }
        let entry = match previous.entries.get(&rel) {
            Some(e) if e.hash == hash => e.clone(),
            _ => {
                let made = std::fs::metadata(&path).and_then(|m| m.modified()).map(secs).unwrap_or_else(|_| now());
                scope.entry(&rel, &hash, size, made, ticket)
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
fn materialize(scope: &Scope, checkout: &Path, manifest: &Manifest, store: &Store, fallback: Option<&Path>, changed: &dyn Fn(&str) -> bool) -> Placed {
    let mut out = Placed::default();
    let design = checkout.join(scope.dir);
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
            && let Some(from) = fallback.map(|f| f.join(scope.dir).join(rel))
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
/// In a squad's folder, the same for what its tasks hand over ([`OUTPUTS`]).
pub fn prepare_commit(worktree: &Path, branch: &str) -> Result<()> {
    let mut failed = Vec::new();
    for scope in SCOPES {
        if let Err(err) = prepare_commit_in(scope, worktree, branch) {
            failed.push(format!("{}: {err}", scope.what));
        }
    }
    // And of what is left there, only what people read (`raw`).
    if let Err(err) = raw::prepare(worktree) {
        failed.push(format!("{}: {err}", raw::WHAT));
    }
    if failed.is_empty() { Ok(()) } else { Err(failed.join("; ")) }
}

/// [`prepare_commit`] for one place.
fn prepare_commit_in(scope: &Scope, worktree: &Path, branch: &str) -> Result<()> {
    if !scope.applies(worktree) {
        return Ok(());
    }
    let store = Store::of(worktree).ok_or("not in a git repository")?;
    let ignore = worktree.join(scope.ignore_path);
    let existing = std::fs::read_to_string(&ignore).unwrap_or_default();
    let wanted = scope.with_block(&existing);
    if wanted != existing {
        if let Some(parent) = ignore.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&ignore, wanted).map_err(|e| format!("{}: {e}", ignore.display()))?;
    }
    // Only what this checkout holds: a sparse one (a squad folder's) leaves
    // other tickets' files out, and those stay as main has them.
    let tracked: Vec<String> = ls_files(worktree, None)?.into_iter().filter(|p| scope.keeps(p) && worktree.join(p).is_file()).collect();
    if !tracked.is_empty() {
        untrack(worktree, &tracked, None)?;
    }
    let previous = manifest_on_disk(scope, worktree).or_else(|| unmerged(scope, worktree)).unwrap_or_else(|| manifest_at_in(scope, worktree, "HEAD"));
    let mut manifest = from_disk(scope, worktree, &store, &previous, &read_seed(scope, worktree), ticket_of_branch(branch));
    // A UI map picture of a state the map no longer lists goes with it (a
    // state renamed or removed): its file was never in this worktree to delete.
    let listed: BTreeSet<(String, String, String)> =
        if scope.dir == DESIGN.dir { super::flows::shots(&worktree.join(super::DIR)).into_iter().map(|s| (s.flow, s.screen, s.state)).collect() } else { BTreeSet::new() };
    if !listed.is_empty() {
        manifest.entries.retain(|_, e| match (&e.flow, &e.screen, &e.state) {
            (Some(f), Some(sc), Some(st)) => listed.contains(&(f.clone(), sc.clone(), st.clone())),
            _ => true,
        });
    }
    let file = worktree.join(scope.manifest_path);
    if manifest.is_empty() && !file.exists() {
        return Ok(());
    }
    let text = manifest.render();
    if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(())
}

/// A merge stopped on `scope`'s manifest in `worktree`: its three sides
/// merged entry by entry. `None` when it isn't unmerged.
fn unmerged(scope: &Scope, worktree: &Path) -> Option<Manifest> {
    let stage = |n: u8| {
        worktree::git_bytes(worktree, &["show", &format!(":{n}:{}", scope.manifest_path)], None, None).ok().and_then(|b| Manifest::parse_in(scope, &String::from_utf8_lossy(&b)))
    };
    let (ours, theirs) = (stage(2), stage(3));
    if ours.is_none() && theirs.is_none() {
        return None;
    }
    Some(merge3(&stage(1).unwrap_or_default(), &ours.unwrap_or_default(), &theirs.unwrap_or_default()))
}

/// A merge in `worktree` stopped on conflicts only in the manifests or in
/// the files they list: settled — each manifest merged entry by entry, the
/// files untracked — and committed. `Ok(false)`: another file conflicts,
/// nothing done.
pub fn settle_merge(worktree: &Path) -> Result<bool> {
    let conflicted = worktree::git(worktree, &["-c", "core.quotePath=false", "diff", "--name-only", "--diff-filter=U"])?;
    let files: Vec<&str> = conflicted.lines().filter(|l| !l.is_empty()).collect();
    if files.is_empty() || !files.iter().all(|f| settles_itself(worktree, f)) {
        return Ok(false);
    }
    let mut kept: Vec<String> = Vec::new();
    for scope in scopes_of(worktree) {
        if files.contains(&scope.manifest_path)
            && let Some(merged) = unmerged(scope, worktree)
        {
            let file = worktree.join(scope.manifest_path);
            std::fs::write(&file, merged.render()).map_err(|e| format!("{}: {e}", file.display()))?;
            worktree::git(worktree, &["add", "--", scope.manifest_path])?;
        }
        kept.extend(files.iter().filter(|f| scope.keeps(f)).map(|f| (*f).to_owned()));
    }
    if !kept.is_empty() {
        untrack(worktree, &kept, None)?;
    }
    worktree::with_identity(worktree, &["commit", "-q", "--no-edit"])?;
    Ok(true)
}

/// Once a worktree of `repo` is checked out at `worktree`: the captures its
/// manifest names that the main checkout doesn't hold as they are — the
/// work of this ticket or of those it builds on — put in place from the
/// store; the rest stay in the main checkout, shown from there
/// ([`borrowed`]). What it was given is remembered. In a squad's folder,
/// its tasks' hand-ins likewise.
pub fn after_checkout(repo: &Path, worktree: &Path) {
    for scope in SCOPES {
        after_checkout_in(scope, repo, worktree);
    }
}

fn after_checkout_in(scope: &Scope, repo: &Path, worktree: &Path) {
    if !scope.applies(worktree) {
        return;
    }
    let Some(store) = Store::of(worktree) else { return };
    let Some(manifest) = manifest_on_disk(scope, worktree) else { return };
    let main = worktree::main_branch(repo).ok().and_then(|m| worktree::checked_out_at(repo, &m)).unwrap_or_else(|| repo.to_owned());
    let accepted = manifest_on_disk(scope, &main).unwrap_or_default();
    let own = Manifest { entries: manifest.entries.into_iter().filter(|(rel, e)| accepted.hash_of(rel) != Some(e.hash.as_str())).collect() };
    let placed = materialize(scope, worktree, &own, &store, Some(&main), &|_| false);
    if placed.missing > 0 {
        crate::daemon_log!("equipe: {} {} named in {} aren't on this machine", placed.missing, scope.what, worktree.join(scope.manifest_path).display());
    }
    write_seed(scope, worktree, &placed.present);
}

/// The captures a worktree's branch changed since `base`, by path from the
/// top of the repository: what its hand-in shows beside the files it changed.
/// In a squad's folder, the handed-in files too.
pub fn changed(worktree: &Path, base: &str) -> Vec<String> {
    let mut out = Vec::new();
    for scope in scopes_of(worktree) {
        let (before, after) = (manifest_at_in(scope, worktree, base), manifest_at_in(scope, worktree, "HEAD"));
        out.extend(after.entries.values().filter(|e| before.hash_of(&e.path) != Some(e.hash.as_str())).map(|e| scope.full(&e.path)));
    }
    out
}

/// The main checkout of the repository whose linked worktree `checkout` is —
/// read from git's files, no git run. `None` for a main checkout itself.
pub fn main_checkout_of(checkout: &Path) -> Option<PathBuf> {
    let admin = worktree::admin_dir(checkout)?;
    let common = std::fs::read_to_string(admin.join("commondir")).ok()?;
    let common = std::fs::canonicalize(admin.join(common.trim())).ok()?;
    if common.file_name()? != ".git" {
        return None;
    }
    common.parent().map(Path::to_path_buf)
}

/// Kept parsed while a manifest file's length and time stay the same: a
/// draft's map is read often.
fn manifest_cached(design: &Path) -> Option<Manifest> {
    use std::sync::Mutex;
    type Kept = HashMap<PathBuf, (u64, Option<SystemTime>, Manifest)>;
    static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
    let file = design.join(MANIFEST);
    let meta = std::fs::metadata(&file).ok()?;
    let (len, modified) = (meta.len(), meta.modified().ok());
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let kept = kept.get_or_insert_with(HashMap::new);
    if let Some((_, _, m)) = kept.get(&file).filter(|(l, t, _)| *l == len && *t == modified) {
        return Some(m.clone());
    }
    let manifest = Manifest::parse(&std::fs::read_to_string(&file).ok()?)?;
    if kept.len() > 64 {
        kept.clear();
    }
    kept.insert(file, (len, modified, manifest.clone()));
    Some(manifest)
}

/// The captures a worktree's design folder `design` lists but doesn't hold
/// because they are unchanged from main: each one's path in the design
/// folder, where the main checkout holds it, and that checkout's design
/// folder (what it must stay inside). Nothing for a main checkout, or a
/// folder equipe doesn't keep.
pub fn borrowed(design: &Path) -> Vec<(String, PathBuf, PathBuf)> {
    let Some(checkout) = design.parent() else { return Vec::new() };
    let Some(main) = main_checkout_of(checkout) else { return Vec::new() };
    let Some(name) = design.file_name() else { return Vec::new() };
    let lender = main.join(name);
    let (Some(mine), Some(theirs)) = (manifest_cached(design), manifest_cached(&lender)) else { return Vec::new() };
    let lender = std::fs::canonicalize(&lender).unwrap_or(lender);
    mine.entries
        .values()
        .filter(|e| theirs.hash_of(&e.path) == Some(e.hash.as_str()))
        .filter(|e| std::fs::symlink_metadata(design.join(&e.path)).is_err())
        .map(|e| (e.path.clone(), lender.join(&e.path)))
        .filter(|(_, path)| path.is_file())
        .map(|(rel, path)| (rel, path, lender.clone()))
        .collect()
}

/// Where the capture at `rel` (from the top of the repository) of
/// `checkout` is to be read: there, else — unchanged from main and so not
/// copied into a ticket's worktree — the main checkout's.
pub fn lent(checkout: &Path, rel: &str) -> Option<PathBuf> {
    let here = checkout.join(rel);
    if here.is_file() {
        return Some(here);
    }
    let inside = rel.strip_prefix("design/")?;
    borrowed(&checkout.join(super::DIR)).into_iter().find(|(r, ..)| r == inside).map(|(_, path, _)| path)
}

/// A capture as `rev`'s manifest names it (`rel` from the top of the
/// repository `dir` is in): the store's copy, else the main checkout's when
/// it holds those bytes. For what used to be read from git (`git show`).
pub fn file_at(dir: &Path, rev: &str, rel: &str) -> Option<PathBuf> {
    let scope = SCOPES.iter().copied().find(|s| s.keeps(rel))?;
    let entry = manifest_at_in(scope, dir, rev).entries.remove(scope.inside(rel)?)?;
    let store = Store::of(dir)?;
    if store.has(&entry.hash) {
        return Some(store.object(&entry.hash));
    }
    let main = main_checkout_of(dir).unwrap_or_else(|| dir.to_owned()).join(rel);
    cached_id(&main, store.kind).filter(|(id, _)| *id == entry.hash).map(|_| main)
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
    let mut out = Published::default();
    for scope in scopes_of(repo) {
        let done = publish_in(scope, repo, main, before, after)?;
        out = Published { placed: out.placed + done.placed, removed: out.removed + done.removed, missing: out.missing + done.missing };
    }
    Ok(out)
}

/// [`publish`] for one place.
fn publish_in(scope: &Scope, repo: &Path, main: &str, before: &str, after: &str) -> Result<Published> {
    let (old, new) = (manifest_at_in(scope, repo, before), manifest_at_in(scope, repo, after));
    if old.is_empty() && new.is_empty() {
        return Ok(Published::default());
    }
    let store = Store::of(repo).ok_or("not in a git repository")?;
    let checkout = worktree::checked_out_at(repo, main).unwrap_or_else(|| repo.to_owned());
    if !checkout.join(scope.dir).is_dir() {
        return Ok(Published::default());
    }
    let placed = materialize(scope, &checkout, &new, &store, None, &|rel| old.hash_of(rel) != new.hash_of(rel));
    let mut out = Published { placed: placed.placed, missing: placed.missing, removed: 0 };
    for (rel, entry) in &old.entries {
        if new.entries.contains_key(rel) {
            continue;
        }
        let path = checkout.join(scope.dir).join(rel);
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
    migrate_in(&DESIGN, repo)
}

/// [`migrate`] for `scope`: in a squad's folder, [`OUTPUTS`] stops being
/// tracked the same way — one commit, the files kept on disk and in the store.
pub fn migrate_in(scope: &Scope, repo: &Path) -> Result<Option<Migrated>> {
    let main = worktree::main_branch(repo)?;
    let tip = worktree::git(repo, &["rev-parse", &format!("refs/heads/{main}")])?;
    if !scope.tree_applies(repo, &tip) {
        return Ok(None);
    }
    let tracked: Vec<(String, String, u64)> = tree_blobs(repo, &tip, scope.dir)?.into_iter().filter(|(p, ..)| scope.keeps(p)).collect();
    let ignore_now = worktree::git_bytes(repo, &["show", &format!("{tip}:{}", scope.ignore_path)], None, None).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let ignore = scope.with_block(&ignore_now);
    if tracked.is_empty() && ignore == ignore_now {
        return Ok(None);
    }
    let store = Store::of(repo).ok_or("not in a git repository")?;
    let checkout = worktree::checked_out_at(repo, &main);
    let committed: u64 = worktree::git(repo, &["log", "-1", "--format=%ct", &tip])?.parse().unwrap_or_else(|_| now());
    let mut manifest = manifest_at_in(scope, repo, &tip);
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
        let rel = scope.inside(path).unwrap_or(path);
        if manifest.hash_of(rel) != Some(id.as_str()) {
            manifest.entries.insert(rel.to_owned(), scope.entry(rel, id, *size, made, None));
        }
    }
    let tree = worktree::git(repo, &["rev-parse", &format!("{tip}^{{tree}}")])?;
    let paths: Vec<String> = tracked.iter().map(|(p, ..)| p.clone()).collect();
    let new_tree = edit_tree(repo, &tree, |index| {
        if !paths.is_empty() {
            untrack(repo, &paths, Some(index))?;
        }
        set_in_index(repo, index, scope.ignore_path, ignore.as_bytes())?;
        if !manifest.is_empty() {
            set_in_index(repo, index, scope.manifest_path, manifest.render().as_bytes())?;
        }
        Ok(())
    })?;
    let bytes: u64 = tracked.iter().map(|(_, _, s)| s).sum();
    let message = format!(
        "equipe: keep {} out of git\n\n{} files ({}) under {}/ stop being tracked: they stay on disk and in\nequipe's store, listed in {}. History is unchanged; equipe's\ndocs/DESIGN-CAPTURES.md says how a person can shrink it.",
        scope.what,
        tracked.len(),
        size_words(bytes),
        scope.dir,
        scope.manifest_path
    );
    let commit = untrack_on_main(repo, &main, &tip, &new_tree, &message, &paths)?;
    Ok(Some(Migrated { commit, untracked: tracked.len(), bytes }))
}

/// Commit `tree` — `tip`'s, with `paths` no longer tracked — on `main` with
/// `message`, made apart from any checkout, and move main there. The checkout
/// main is checked out in keeps every one of `paths` on disk. The commit.
fn untrack_on_main(repo: &Path, main: &str, tip: &str, tree: &str, message: &str, paths: &[String]) -> Result<String> {
    let store = Store::of(repo).ok_or("not in a git repository")?;
    let checkout = worktree::checked_out_at(repo, main);
    let commit = worktree::with_identity(repo, &["commit-tree", tree, "-p", tip, "-m", message])?;
    // A fast-forward deletes the files main stops tracking: each is held by
    // a second link meanwhile and put back.
    let held = checkout.as_ref().map(|c| hold(&store, c, paths)).unwrap_or_default();
    let moved = worktree::move_main_within(repo, main, tip, &commit, "untracked", MIGRATE_TIMEOUT);
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
    Ok(commit)
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
    /// Files its commits added under a squad folder's `outputs/` ([`OUTPUTS`]),
    /// taken out of them likewise, and their size.
    #[serde(default)]
    pub handed_in_kept_out: usize,
    #[serde(default)]
    pub handed_in_bytes: u64,
    /// Machine output its commits added under a squad folder's `outputs/` —
    /// or more than a hand-in keeps in git ([`raw`]) — taken out of them; the
    /// files stay in the folder.
    #[serde(default)]
    pub raw_kept_out: usize,
    #[serde(default)]
    pub raw_bytes: u64,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        self.kept_out == 0 && self.handed_in_kept_out == 0 && self.raw_kept_out == 0 && self.heavy.is_empty()
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
        if self.handed_in_kept_out > 0 {
            out.push(format!(
                "When T-{ticket} landed, equipe took {} pictures, videos or other generated files ({}) under outputs/ out of its commits: they stay in the squad's folder and in equipe's store, listed in {}, and never go into the folder's history. Leave them where they are and never `git add` them, not with -f either.",
                self.handed_in_kept_out,
                size_words(self.handed_in_bytes),
                OUTPUTS.manifest_path
            ));
        }
        if self.raw_kept_out > 0 {
            out.push(raw::told(ticket, self.raw_kept_out, self.raw_bytes));
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
    let mut report = Report::default();
    let first_head = worktree::git(repo, &["rev-parse", &local])?;
    for scope in scopes_of(repo) {
        // Copying a big map into the store is no work for the daemon's loop
        // (a task landing in the squad's folder): its hourly sweep does it then.
        if !crate::runtime::on_core_loop()
            && let Err(err) = migrate_in(scope, repo)
        {
            crate::daemon_log!("equipe: {} still tracks {}: {err}", repo.display(), scope.what);
        }
        let (count, bytes) = guard_in(scope, repo, branch, &main)?;
        if scope.dir == DESIGN.dir {
            (report.kept_out, report.bytes) = (count, bytes);
        } else {
            (report.handed_in_kept_out, report.handed_in_bytes) = (count, bytes);
        }
    }
    // A squad's folder: of its hand-ins, git keeps what people read.
    if in_squad_folder(repo) {
        if !crate::runtime::on_core_loop()
            && let Err(err) = raw::migrate(repo)
        {
            crate::daemon_log!("equipe: {} still tracks {}: {err}", repo.display(), raw::WHAT);
        }
        (report.raw_kept_out, report.raw_bytes) = raw::guard_in(repo, branch, &main)?;
    }
    let tip = worktree::git(repo, &["rev-parse", &format!("refs/heads/{main}")])?;
    let new_head = worktree::git(repo, &["rev-parse", &local]).unwrap_or(first_head);
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

/// [`guard`] for one place: `branch`'s own commits that add its files
/// rewritten without them, each kept in the store and listed in the
/// branch's manifest. How many it took out, and their size.
fn guard_in(scope: &Scope, repo: &Path, branch: &str, main: &str) -> Result<(usize, u64)> {
    let local = format!("refs/heads/{branch}");
    let tip = worktree::git(repo, &["rev-parse", &format!("refs/heads/{main}")])?;
    let head = worktree::git(repo, &["rev-parse", &local])?;
    let range = format!("{tip}..{head}");
    let names = |args: &[&str]| -> Result<Vec<String>> {
        let out = worktree::git_bytes(repo, args, None, None)?;
        Ok(out.split(|b| *b == 0 || *b == b'\n').filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).filter(|p| scope.keeps(p)).collect())
    };
    let mut strip: BTreeSet<String> = names(&["-c", "core.quotePath=false", "log", "--no-merges", "--format=", "--name-only", "-z", "--diff-filter=AMT", &range])?.into_iter().collect();
    strip.extend(names(&["-c", "core.quotePath=false", "diff", "--name-only", "-z", "--diff-filter=AMT", &format!("{tip}...{head}")])?);
    if strip.is_empty() {
        return Ok((0, 0));
    }
    let store = Store::of(repo).ok_or("not in a git repository")?;
    let at_head: Vec<(String, String, u64)> = tree_blobs(repo, &head, scope.dir)?.into_iter().filter(|(p, ..)| strip.contains(p)).collect();
    let committed: u64 = worktree::git(repo, &["log", "-1", "--format=%ct", &head])?.parse().unwrap_or_else(|_| now());
    let ticket = ticket_of_branch(branch);
    let mut manifest = manifest_at_in(scope, repo, &head);
    for (path, id, size) in &at_head {
        store.put_blob(repo, id).map_err(|e| format!("couldn't keep {path} in the captures' store, so {branch} wasn't merged: {e}"))?;
        let rel = scope.inside(path).unwrap_or(path);
        if manifest.hash_of(rel) != Some(id.as_str()) {
            manifest.entries.insert(rel.to_owned(), scope.entry(rel, id, *size, committed, ticket));
        }
    }
    let new_head = rewrite(scope, repo, &tip, &head, &strip, &manifest)?;
    let checked_out = worktree::checked_out_at(repo, branch);
    worktree::git(repo, &["update-ref", &local, &new_head, &head])?;
    if let Some(worktree) = checked_out {
        // Its index follows the branch; the files stay as they are, and
        // the two equipe wrote match what is committed.
        let _ = worktree::git(&worktree, &["reset", "-q"]);
        for rel in [scope.manifest_path, scope.ignore_path] {
            if let Ok(bytes) = worktree::git_bytes(&worktree, &["show", &format!("HEAD:{rel}")], None, None) {
                let _ = std::fs::write(worktree.join(rel), bytes);
            }
        }
    }
    Ok((at_head.len(), at_head.iter().map(|(_, _, s)| s).sum()))
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
fn rewrite(scope: &Scope, repo: &Path, tip: &str, head: &str, strip: &BTreeSet<String>, manifest: &Manifest) -> Result<String> {
    rewrite_commits(repo, tip, head, strip, &|index, commit| {
        let ignore = worktree::git_bytes(repo, &["show", &format!("{commit}:{}", scope.ignore_path)], None, None).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        set_in_index(repo, index, scope.ignore_path, scope.with_block(&ignore).as_bytes())?;
        set_in_index(repo, index, scope.manifest_path, manifest.render().as_bytes())
    })
}

/// The commits of `tip..head` again, without `strip`'s paths, the last one
/// changed by `last` too (given its scratch index and the commit it was); as
/// [`rewrite`] says. The new head.
fn rewrite_commits(repo: &Path, tip: &str, head: &str, strip: &BTreeSet<String>, last: &dyn Fn(&Path, &str) -> Result<()>) -> Result<String> {
    let listed = worktree::git(repo, &["rev-list", "--reverse", "--topo-order", "--parents", &format!("{tip}..{head}")])?;
    let mut map: HashMap<String, String> = HashMap::new();
    for line in listed.lines().filter(|l| !l.is_empty()) {
        let mut ids = line.split_whitespace();
        let Some(commit) = ids.next() else { continue };
        let parents: Vec<String> = ids.map(str::to_owned).collect();
        let new_parents: Vec<String> = parents.iter().map(|p| map.get(p).cloned().unwrap_or_else(|| p.clone())).collect();
        let tree = worktree::git(repo, &["rev-parse", &format!("{commit}^{{tree}}")])?;
        let is_last = commit == head;
        let new_tree = edit_tree(repo, &tree, |index| {
            let gone: Vec<String> = ls_files(repo, Some(index))?.into_iter().filter(|p| strip.contains(p)).collect();
            if !gone.is_empty() {
                untrack(repo, &gone, Some(index))?;
            }
            if is_last {
                last(index, commit)?;
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
    if conflicted.is_empty() || !conflicted.iter().all(|f| settles_itself(repo, f)) {
        return None;
    }
    let base = worktree::git(repo, &["merge-base", tip, head]).ok();
    let scopes = scopes_of(repo);
    let manifests: Vec<(&Scope, Manifest)> = scopes
        .iter()
        .filter(|s| conflicted.contains(&s.manifest_path))
        .map(|s| (*s, merge3(&base.as_ref().map(|b| manifest_at_in(s, repo, b)).unwrap_or_default(), &manifest_at_in(s, repo, tip), &manifest_at_in(s, repo, head))))
        .collect();
    let kept: Vec<String> = conflicted.iter().filter(|f| scopes.iter().any(|s| s.keeps(f))).map(|f| (*f).to_owned()).collect();
    edit_tree(repo, merged, |index| {
        let listed: BTreeSet<String> = ls_files(repo, Some(index))?.into_iter().collect();
        let gone: Vec<String> = kept.iter().filter(|c| listed.contains(*c)).cloned().collect();
        if !gone.is_empty() {
            untrack(repo, &gone, Some(index))?;
        }
        for (scope, manifest) in &manifests {
            set_in_index(repo, index, scope.manifest_path, manifest.render().as_bytes())?;
        }
        Ok(())
    })
    .ok()
}

/// Whether a file merging reports conflicting in `repo` is one equipe
/// settles itself: a manifest, or a file one lists.
pub fn settles_itself(repo: &Path, path: &str) -> bool {
    scopes_of(repo).iter().any(|s| path == s.manifest_path || s.keeps(path))
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
        let mut log = vec!["log".to_owned(), "--since=30.days".into(), "--format=%H".into(), format!("refs/heads/{main}"), "--".into()];
        log.extend(SCOPES.iter().map(|s| s.manifest_path.to_owned()));
        let log: Vec<&str> = log.iter().map(String::as_str).collect();
        let recent = worktree::git(repo, &log).unwrap_or_default();
        revs.extend(recent.lines().map(str::to_owned));
    }
    // Every place's manifests: the store is shared by all of them.
    for rev in revs.iter().filter(|r| !r.is_empty()) {
        for scope in SCOPES {
            named.extend(manifest_at_in(scope, repo, rev).entries.into_values().map(|e| e.hash));
        }
    }
    // Worktrees' manifests on disk: work committed soon.
    for checkout in worktree::listed(repo) {
        for scope in SCOPES {
            if let Some(m) = manifest_on_disk(scope, &checkout) {
                named.extend(m.entries.into_values().map(|e| e.hash));
            }
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

pub mod raw;

#[cfg(test)]
mod tests;
