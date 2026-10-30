//! A member's pictures — the options of a visual decision, what a hand-in
//! shows — and the one way a screen gets them. A screen may be on another
//! Mac (over SSH) or a phone (through the relay), so it can't read the
//! member's disk: it asks the daemon (`read_image`), which gives a picture
//! only when the squad shows that path now (a waiting decision names it, a
//! ticket's hand-in does), and only when it is an image file inside the
//! squad's folder or a ticket's worktree — links resolved, no `..`, no file
//! of another kind, nothing too big. Each is checked where it is named too,
//! so a member hears at once what won't show. A thumbnail is made here, so a
//! phone never gets the whole of a 6 MB render.
//!
//! A reply shows pictures too: each image its Markdown embeds, and the
//! pictures right in a folder it points at (`markdown`) — given only when a
//! line the daemon itself recorded for the squad names them, under the same
//! rules; a picture a line embeds by name may be in one of the squad's
//! projects or a temp folder too, where a quick chart or render lands. A ticket whose worktree went (accepted, its merge still to come)
//! still shows its hand-in: the pictures are read from its branch, or from
//! main once merged, by the paths the ticket recorded and no others.

pub mod markdown;

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use crate::protocol::decide::Question;
use crate::protocol::rpc::{self, ImageList, ImageRead, ListImages, ReadImage};
use crate::protocol::{Entry, Ticket};

/// The extensions a picture may have, and the type each is sent as.
const KINDS: [(&str, &str); 6] =
    [("png", "image/png"), ("jpg", "image/jpeg"), ("jpeg", "image/jpeg"), ("webp", "image/webp"), ("gif", "image/gif"), ("svg", "image/svg+xml")];
/// The largest picture read at all.
pub const FILE_MAX: u64 = 30 << 20;
/// The largest sent as it is; past it, a copy no wider than [`BIG_SIDE`].
pub const WHOLE_MAX: u64 = 6 << 20;
/// An SVG goes as it is, so it must be small.
pub const SVG_MAX: u64 = 1 << 20;
/// A copy of a picture too big to send whole: at most this many pixels on its longer side.
pub const BIG_SIDE: u32 = 2560;
/// The smallest thumbnail asked for.
const SMALLEST: u32 = 32;
/// The most pixels across or down a picture may have to be scaled here.
const DECODE_SIDE: u32 = 16_384;
/// What a phone may ask for at most: its frames go through the relay, 1 MB each.
pub const PHONE_SIDE: u32 = 720;
/// How many pictures an option, a question and a hand-in may show.
pub const PER_OPTION: usize = 4;
pub const PER_QUESTION: usize = 6;
pub const HAND_IN: usize = 12;
/// How many pictures of a folder a screen is given at most.
pub const LIST_MAX: usize = 12;
/// How many names in a folder are looked at, at most.
const LIST_SCAN: usize = 2000;
/// How many of the latest lines that name a path a reply's pictures are read from.
const SAID: u64 = 400;

/// The type a picture is sent as, by its name; `None` for anything else.
pub fn mime_of(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    KINDS.iter().find(|(e, _)| *e == ext).map(|(_, mime)| *mime)
}

/// Whether a file of this name is a picture a member may show.
pub fn is_image_name(name: &str) -> bool {
    mime_of(Path::new(name)).is_some()
}

/// What the bytes are, by their first few: a picture of one of the kinds
/// taken, or `None`.
fn sniff(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        let text = String::from_utf8_lossy(&head[..head.len().min(4096)]);
        let text = text.trim_start_matches('\u{feff}').trim_start();
        (text.starts_with('<') && text.contains("<svg")).then_some("image/svg+xml")
    }
}

/// An SVG that would reach past itself when drawn — a script, a page in it,
/// a link to a file or a site — is refused: only what it draws, and its own
/// `#parts` and `data:` pictures.
fn svg_is_closed(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.contains("<script") || lower.contains("<foreignobject") || lower.contains("<!entity") {
        return false;
    }
    for attr in ["href=", "src="] {
        let mut rest = lower.as_str();
        while let Some(at) = rest.find(attr) {
            let value = rest[at + attr.len()..].trim_start_matches(['"', '\'', ' ']);
            if !(value.starts_with('#') || value.starts_with("data:")) {
                return false;
            }
            rest = &rest[at + attr.len()..];
        }
    }
    !lower.contains("url(http") && !lower.contains("url(file") && !lower.contains("@import")
}

/// Why a path isn't a picture a screen may have.
fn refused(path: &Path, why: &str) -> String {
    format!("{}: {why}", path.display())
}

/// `path`, as it really is, when it is a picture a member may show: an
/// absolute path without `..`, to a file (links resolved) inside one of
/// `roots`, named as an image, no bigger than [`FILE_MAX`].
pub fn check(path: &Path, roots: &[PathBuf]) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err(refused(path, "give the picture's full path, without `..`"));
    }
    if mime_of(path).is_none() {
        return Err(refused(path, "not a picture — png, jpg, webp, gif or svg"));
    }
    std::fs::symlink_metadata(path).map_err(|_| refused(path, "no such file"))?;
    let real = std::fs::canonicalize(path).map_err(|_| refused(path, "no such file"))?;
    let roots: Vec<PathBuf> = roots.iter().filter_map(|r| std::fs::canonicalize(r).ok()).collect();
    if !roots.iter().any(|root| real.starts_with(root)) {
        return Err(refused(path, &format!("{} and the squad's folder", rpc::IMAGE_OUTSIDE)));
    }
    if mime_of(&real).is_none() {
        return Err(refused(path, "a link to something that isn't a picture"));
    }
    let meta = std::fs::metadata(&real).map_err(|_| refused(path, "no such file"))?;
    if !meta.is_file() {
        return Err(refused(path, "not a file"));
    }
    if meta.len() > FILE_MAX {
        return Err(refused(path, &format!("bigger than {} MB", FILE_MAX >> 20)));
    }
    Ok(real)
}

/// A path a member gave (relative to `cwd`, where it works, or absolute),
/// checked by [`check`], as the path screens ask for it by.
pub fn resolve(given: &str, cwd: &Path, roots: &[PathBuf]) -> Result<String, String> {
    let given = given.trim();
    let given = given.strip_prefix("file://").unwrap_or(given);
    let path = Path::new(given);
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(refused(path, "give the picture's path without `..`"));
    }
    let path = if path.is_absolute() { path.to_owned() } else { cwd.join(path) };
    let path: PathBuf = path.components().filter(|c| !matches!(c, Component::CurDir)).collect();
    check(&path, roots).map(|real| real.to_string_lossy().into_owned())
}

/// The pictures of an `ask_decision` call's or a hand-in's `images`: each
/// path resolved, at most `most`; what is wrong said so the member can call again.
pub fn resolve_all(given: &[String], cwd: &Path, roots: &[PathBuf], most: usize) -> Result<Vec<String>, String> {
    if given.len() > most {
        return Err(format!("At most {most} pictures there, not {}.", given.len()));
    }
    let mut out: Vec<String> = Vec::new();
    for g in given {
        let path = resolve(g, cwd, roots)?;
        if !out.contains(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

/// Read picture `path` for a screen — whole, or no bigger than `max`
/// pixels on its longer side. One too big to send whole goes as a copy no
/// bigger than [`BIG_SIDE`]; an SVG goes as it is.
pub fn read(path: &Path, roots: &[PathBuf], max: Option<u32>) -> Result<ImageRead, String> {
    use std::io::Read as _;
    let real = check(path, roots)?;
    let mut bytes = Vec::new();
    std::fs::File::open(&real).and_then(|f| f.take(FILE_MAX + 1).read_to_end(&mut bytes)).map_err(|_| refused(path, "couldn't be read"))?;
    picture(path, &real, bytes).and_then(|(bytes, mime)| sized(path, bytes, mime, max))
}

/// The bytes of picture `path` (named `named`, links resolved), when they
/// are what its name says: the bytes and their type.
fn picture(path: &Path, named: &Path, bytes: Vec<u8>) -> Result<(Vec<u8>, &'static str), String> {
    if bytes.len() as u64 > FILE_MAX {
        return Err(refused(path, &format!("bigger than {} MB", FILE_MAX >> 20)));
    }
    let named = mime_of(named).ok_or_else(|| refused(path, "not a picture — png, jpg, webp, gif or svg"))?;
    let mime = sniff(&bytes).ok_or_else(|| refused(path, "not a picture inside"))?;
    if (named == "image/svg+xml") != (mime == "image/svg+xml") {
        return Err(refused(path, "its name and what it holds differ"));
    }
    Ok((bytes, mime))
}

/// A picture's bytes for a screen: whole, or no bigger than `max` pixels on
/// its longer side; a copy no bigger than [`BIG_SIDE`] when too big to send
/// whole; an SVG as it is.
fn sized(path: &Path, bytes: Vec<u8>, mime: &'static str, max: Option<u32>) -> Result<ImageRead, String> {
    use base64::Engine as _;
    let size = bytes.len() as u64;
    let svg = mime == "image/svg+xml";
    let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    if svg {
        if size > SVG_MAX {
            return Err(refused(path, &format!("an SVG bigger than {} KB", SVG_MAX >> 10)));
        }
        if !svg_is_closed(&String::from_utf8_lossy(&bytes)) {
            return Err(refused(path, "an SVG that loads other files or runs a script"));
        }
        return Ok(ImageRead { mime: mime.into(), size, width: None, height: None, data: encode(&bytes), scaled: false });
    }
    let reader = || {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format().map_err(|_| refused(path, "couldn't be read"))?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(DECODE_SIDE);
        limits.max_image_height = Some(DECODE_SIDE);
        limits.max_alloc = Some(512 << 20);
        reader.limits(limits);
        Ok::<_, String>(reader)
    };
    let (width, height) = reader()?.into_dimensions().map_err(|_| refused(path, "not a picture it can read"))?;
    let side = match max.map(|m| m.clamp(SMALLEST, BIG_SIDE)) {
        Some(side) => Some(side),
        None if size > WHOLE_MAX => Some(BIG_SIDE),
        None => None,
    };
    let Some(side) = side.filter(|side| width > *side || height > *side || size > WHOLE_MAX) else {
        return Ok(ImageRead { mime: mime.into(), size, width: Some(width), height: Some(height), data: encode(&bytes), scaled: false });
    };
    let decoded = reader()?.decode().map_err(|_| refused(path, "not a picture it can read"))?;
    let small = decoded.thumbnail(side, side);
    let mut out = std::io::Cursor::new(Vec::new());
    let mime = if small.color().has_alpha() {
        small.to_rgba8().write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
        "image/png"
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 86).encode_image(&small.to_rgb8()).map_err(|e| e.to_string())?;
        "image/jpeg"
    };
    Ok(ImageRead { mime: mime.into(), size, width: Some(width), height: Some(height), data: encode(&out.into_inner()), scaled: true })
}

/// What a squad shows now: the pictures its waiting decisions, its tickets
/// and its design systems name, and where they may be — its folder, its
/// tickets' worktrees, its design folders (`crate::design`).
#[derive(Debug, Default)]
pub struct Shown {
    pub paths: HashSet<String>,
    pub roots: Vec<PathBuf>,
    /// Folders the conversation points at: the pictures right in them show too.
    pub folders: HashSet<String>,
    /// A ticket's pictures whose worktree is gone: where git still has each.
    pub kept: HashMap<String, Kept>,
    /// The pictures the conversation's lines embed by name: given from the
    /// squad's projects and the scratch folders too.
    pub said: HashSet<String>,
    /// The squad's projects.
    pub projects: Vec<PathBuf>,
}

/// Where git has a picture of a ticket whose worktree went: in `repo`, at
/// `rel` from its top, on the ticket's branch — or on main, once merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub repo: PathBuf,
    pub rel: PathBuf,
    pub branch: Option<String>,
    pub merged: bool,
}

/// Where a picture of a ticket's hand-in was in its worktree (gone now): the
/// repository and the path from its top — when it was in one of `repo`'s
/// worktrees under `home` at all.
fn in_worktree(image: &str, repo: &Path, home: &Path) -> Option<PathBuf> {
    let homes = [home.to_owned(), home.canonicalize().unwrap_or_else(|_| home.to_owned())];
    let rest = homes.iter().find_map(|h| Path::new(image).strip_prefix(crate::worktree::worktrees_of(repo, h)).ok())?;
    let mut parts = rest.components();
    let Some(Component::Normal(_)) = parts.next() else { return None };
    let rel: PathBuf = parts.collect();
    let clean = rel.components().all(|c| matches!(c, Component::Normal(_)));
    (clean && !rel.as_os_str().is_empty()).then_some(rel)
}

/// What git has of a ticket's pictures, once its worktree is gone.
fn kept_of(ticket: &Ticket, home: &Path, squad: &str) -> Vec<(String, Kept)> {
    if ticket.images.is_empty() || ticket.worktree.as_ref().is_some_and(|w| w.is_dir()) {
        return Vec::new();
    }
    let repo = if ticket.kind.in_home() { Some(crate::squad_home::path(home, squad)) } else { ticket.project.clone() };
    let Some(repo) = repo else { return Vec::new() };
    let merged = ticket.merged == Some(true) || (ticket.status == crate::protocol::TicketStatus::Done && ticket.branch.is_none());
    ticket
        .images
        .iter()
        .filter_map(|image| Some((image.clone(), Kept { repo: repo.clone(), rel: in_worktree(image, &repo, home)?, branch: ticket.branch.clone(), merged })))
        .collect()
}

/// Read from the squad's journal; nothing for a squad that isn't there.
pub fn shown(home: &Path, squad: &str) -> Shown {
    let db = crate::journal::squad_dir(home, squad).join("squad.db");
    if !crate::squads::valid_name(squad) || !db.exists() {
        return Shown::default();
    }
    let Ok(journal) = crate::journal::Journal::open(&db) else { return Shown::default() };
    let mut out = Shown { roots: vec![crate::squad_home::path(home, squad)], projects: journal.projects().unwrap_or_default(), ..Shown::default() };
    for set in journal.questions() {
        out.paths.extend(set.images().map(str::to_owned));
    }
    for ticket in journal.tickets().unwrap_or_default() {
        out.paths.extend(ticket.images.iter().cloned());
        out.kept.extend(kept_of(&ticket, home, squad));
        out.roots.extend(ticket.worktree);
    }
    let (design, folders) = crate::design::folder::shown(home, squad);
    out.paths.extend(design);
    out.roots.extend(folders);
    said(&journal, home, squad, &mut out);
    out
}

/// What the squad's latest lines show: each image their Markdown embeds (a
/// relative one taken from where its member works, or the squad's folder),
/// each folder they point at, and the member worktrees they were said in.
fn said(journal: &crate::journal::Journal, home: &Path, squad: &str, out: &mut Shown) {
    let folder = crate::squad_home::path(home, squad);
    let mut cwds: HashSet<PathBuf> = HashSet::new();
    for (cwd, entry) in journal.said_with_paths(SAID).unwrap_or_default() {
        let (Entry::User { text } | Entry::Reply { text } | Entry::Note { text }) = &entry else { continue };
        for image in markdown::local_images(text) {
            for base in [&cwd, &folder] {
                if let Some(path) = markdown::lexical(&image.path, base) {
                    out.said.insert(path.to_string_lossy().into_owned());
                    out.paths.insert(path.to_string_lossy().into_owned());
                }
            }
        }
        for named in markdown::folders(text) {
            if let Some(path) = markdown::lexical(&named, &cwd) {
                out.folders.insert(path.to_string_lossy().into_owned());
            }
        }
        cwds.insert(cwd);
    }
    // A member's worktree it spoke from is where its pictures may be.
    let Ok(worktrees) = home.join("worktrees").canonicalize() else { return };
    for cwd in cwds {
        if cwd.canonicalize().is_ok_and(|real| real.starts_with(&worktrees) && real.components().count() >= worktrees.components().count() + 2) && !out.roots.contains(&cwd) {
            out.roots.push(cwd);
        }
    }
}

/// Whether `path` is a picture right in a folder the squad's conversation points at.
fn in_folder(shown: &Shown, path: &str) -> bool {
    is_image_name(path) && Path::new(path).parent().is_some_and(|dir| shown.folders.contains(dir.to_string_lossy().as_ref()))
}

/// A picture of a ticket whose worktree is gone, read from git: the
/// project's checkout or main once merged, else the ticket's branch.
fn read_kept(path: &Path, kept: &Kept, max: Option<u32>) -> Result<ImageRead, String> {
    use crate::worktree::{self, Blob};
    if mime_of(path).is_none() {
        return Err(refused(path, "not a picture — png, jpg, webp, gif or svg"));
    }
    if kept.merged {
        let file = kept.repo.join(&kept.rel);
        if file.is_file()
            && let Ok(read) = read(&file, std::slice::from_ref(&kept.repo), max)
        {
            return Ok(read);
        }
    }
    let main = worktree::main_branch(&kept.repo).ok();
    let revs: Vec<&String> = if kept.merged { main.iter().chain(kept.branch.iter()).collect() } else { kept.branch.iter().chain(main.iter()).collect() };
    let mut failed = None;
    for rev in revs {
        match worktree::read_blob(&kept.repo, rev, &kept.rel, FILE_MAX) {
            Ok(bytes) => return picture(path, path, bytes).and_then(|(bytes, mime)| sized(path, bytes, mime, max)),
            Err(Blob::Missing) => {}
            Err(Blob::TooBig(_)) => return Err(refused(path, &format!("bigger than {} MB", FILE_MAX >> 20))),
            Err(Blob::Failed(why)) => failed = Some(why),
        }
    }
    match failed {
        Some(why) => Err(refused(path, &format!("couldn't be read from git: {why}"))),
        None => Err(refused(path, rpc::IMAGE_NOT_ON_BRANCH)),
    }
}

/// Answer a `read_image` for `ask.squad`, at most `cap` pixels on a side
/// when given (a phone's).
pub fn serve(home: &Path, ask: &ReadImage, cap: Option<u32>) -> Result<ImageRead, String> {
    serve_with(home, ask, cap, &scratch_folders())
}

/// The folders a member drops a quick render or chart in: this Mac's temp folders.
fn scratch_folders() -> Vec<PathBuf> {
    vec![std::env::temp_dir(), PathBuf::from("/tmp"), PathBuf::from("/private/tmp")]
}

/// [`serve`], with `scratch` as the scratch folders: a picture a line embeds
/// by name is given from the squad's projects and those too.
pub fn serve_with(home: &Path, ask: &ReadImage, cap: Option<u32>, scratch: &[PathBuf]) -> Result<ImageRead, String> {
    let mut shown = shown(home, &ask.squad);
    if !shown.paths.contains(&ask.path) && !in_folder(&shown, &ask.path) {
        return Err(format!("{} isn't a picture {} shows", ask.path, ask.squad));
    }
    if shown.said.contains(&ask.path) {
        shown.roots.extend(shown.projects.iter().cloned());
        shown.roots.extend(scratch.iter().cloned());
    }
    let max = match (ask.max, cap) {
        (Some(max), Some(cap)) => Some(max.min(cap)),
        (max, cap) => max.or(cap),
    };
    let path = Path::new(&ask.path);
    match read(path, &shown.roots, max) {
        // Its worktree went with its acceptance: git still has it.
        Err(why) if std::fs::symlink_metadata(path).is_err() => match shown.kept.get(&ask.path) {
            Some(kept) => read_kept(path, kept, max),
            None => Err(why),
        },
        read => read,
    }
}

/// Answer a `list_images`: the pictures right in a folder the squad's
/// conversation points at — inside its folder or a member's worktree, each
/// passing [`check`] — by name, at most [`LIST_MAX`].
pub fn list(home: &Path, ask: &ListImages) -> Result<ImageList, String> {
    let shown = shown(home, &ask.squad);
    let folder = markdown::lexical(&ask.folder, Path::new("/")).filter(|_| Path::new(ask.folder.trim()).is_absolute()).ok_or_else(|| format!("{}: give the folder's full path, without `..`", ask.folder))?;
    let key = folder.to_string_lossy().into_owned();
    if !shown.folders.contains(&key) {
        return Err(format!("{key} isn't a folder {} shows", ask.squad));
    }
    let real = folder.canonicalize().map_err(|_| format!("{key}: no such folder"))?;
    let roots: Vec<PathBuf> = shown.roots.iter().filter_map(|r| r.canonicalize().ok()).collect();
    if !roots.iter().any(|root| real.starts_with(root)) {
        return Err(format!("{key}: outside the members' worktrees and the squad's folder"));
    }
    let entries = std::fs::read_dir(&real).map_err(|_| format!("{key}: not a folder it can read"))?;
    let mut names: Vec<String> = entries.take(LIST_SCAN).filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| is_image_name(n) && !n.starts_with('.')).collect();
    names.sort_by_key(|n| n.to_lowercase());
    let images: Vec<String> = names.iter().map(|n| folder.join(n)).filter(|p| check(p, &shown.roots).is_ok()).map(|p| p.to_string_lossy().into_owned()).collect();
    let most = ask.max.map_or(LIST_MAX, |m| (m as usize).min(LIST_MAX));
    Ok(ImageList { folder: key, total: images.len() as u32, images: images.into_iter().take(most).collect() })
}

/// Folders a hand-in's source assets are kept in: shown after its work.
const ASSET_DIRS: [&str; 8] = ["public", "assets", "textures", "static", "src", "res", "resources", "images"];

/// Whether a picture reads as one of the project's source assets — a texture,
/// a static file — rather than the work handed in (`outputs/`, a render).
pub fn is_asset(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    !parts.contains(&"outputs") && parts[..parts.len().saturating_sub(1)].iter().any(|p| ASSET_DIRS.contains(&p.to_ascii_lowercase().as_str()))
}

/// What a picture is whatever size it comes in — `earth_day_2048.jpg` and
/// `earth_day_4096.jpg` are one — and the size it is in (0: none said).
fn variant(path: &str) -> (String, u32) {
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let mut stem = stem.to_owned();
    let mut size = 0;
    if let Some(at) = stem.rfind('@').filter(|at| stem[at + 1..].strip_suffix('x').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))) {
        size = stem[at + 1..stem.len() - 1].parse().unwrap_or(0);
        stem.truncate(at);
    } else if let Some(at) = stem.rfind(['_', '-']) {
        let tail = &stem[at + 1..];
        let number = |t: &str| (3..=5).contains(&t.len()) && t.chars().all(|c| c.is_ascii_digit());
        let pixels = tail.split_once('x').filter(|(w, h)| number(w) && number(h));
        if number(tail) || pixels.is_some() {
            size = pixels.map_or(tail, |(w, _)| w).parse().unwrap_or(0);
            stem.truncate(at);
        }
    }
    (format!("{dir}/{stem}.{}", ext.to_ascii_lowercase()), size)
}

/// A hand-in's pictures in the order they are worth a look: its work first,
/// as it came — what its member named, what is under `outputs/`, a render —
/// then the project's source assets ([`is_asset`]); a picture in several
/// sizes once, its biggest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Arranged {
    pub work: Vec<String>,
    pub assets: Vec<String>,
}

pub fn arrange(images: &[String]) -> Arranged {
    let mut kept: Vec<(String, String, u32)> = Vec::new();
    for image in images {
        let (key, size) = variant(image);
        match kept.iter_mut().find(|(_, k, _)| *k == key) {
            Some(slot) if size > slot.2 => *slot = (image.clone(), key, size),
            Some(_) => {}
            None => kept.push((image.clone(), key, size)),
        }
    }
    let (assets, work): (Vec<_>, Vec<_>) = kept.into_iter().map(|(image, ..)| image).partition(|i| is_asset(i));
    Arranged { work, assets }
}

/// What a hand-in shows: the pictures its member named, then the images its
/// branch changed (`changed`, relative to `worktree`) that pass [`check`] —
/// at most [`HAND_IN`].
pub fn hand_in(named: &[String], changed: &[String], worktree: &Path, roots: &[PathBuf]) -> Vec<String> {
    let mut out: Vec<String> = named.to_vec();
    let found: Vec<String> = changed.iter().filter(|f| is_image_name(f)).filter_map(|rel| check(&worktree.join(rel), roots).ok()).map(|real| real.to_string_lossy().into_owned()).collect();
    // Its work, then its assets; a picture in several sizes once.
    let arranged = arrange(&found);
    for real in arranged.work.into_iter().chain(arranged.assets) {
        if !out.iter().any(|o| *o == real || variant(o).0 == variant(&real).0) {
            out.push(real);
        }
    }
    out.truncate(HAND_IN);
    out
}

/// Pictures that were in `from` (a worktree whose work landed in `to`, the
/// squad's folder), where they are now.
pub fn moved(images: &[String], from: &Path, to: &Path) -> Vec<String> {
    images
        .iter()
        .map(|image| match Path::new(image).strip_prefix(from) {
            Ok(rest) => to.join(rest).to_string_lossy().into_owned(),
            Err(_) => image.clone(),
        })
        .collect()
}

/// Words a question names something to look at by, in the languages the
/// person may write in.
fn visual_words() -> &'static [&'static str] {
    crate::i18n::every("match.visual_words")
}

/// The word a tool's answer adds when a question names something to look at
/// — a logo, colours, a layout — yet shows no picture: the person would be
/// choosing blind. Only ever a word to the member; the question still goes.
pub fn nudge(questions: &[Question]) -> Option<&'static str> {
    if questions.iter().any(|q| q.images().next().is_some()) {
        return None;
    }
    let names_visuals = questions.iter().filter(|q| !q.options.is_empty()).any(|q| {
        let text = std::iter::once(q.question.as_str()).chain(std::iter::once(q.header.as_str())).chain(q.options.iter().flat_map(|c| [c.label.as_str(), c.description.as_str()])).collect::<Vec<_>>().join(" ").to_lowercase();
        visual_words().iter().any(|w| text.contains(w))
    });
    names_visuals.then_some(
        "This reads like a visual choice, yet no option shows a picture: the person is choosing blind. Render or mock up \
each option first (PNG, JPG, WebP, GIF or SVG, in your worktree or the squad's folder) and give each option its \
`images` — the person then sees them side by side. If you can make them now, do, and ask again with the pictures.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::decide::Choice;

    /// The daemon's own temp folders hold every test's files: none here.
    fn serve(home: &Path, ask: &ReadImage, cap: Option<u32>) -> Result<ImageRead, String> {
        serve_with(home, ask, cap, &[])
    }
    use std::os::unix::fs::symlink;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-visuals-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn png(path: &Path, w: u32, h: u32, alpha: bool) {
        if alpha {
            image::RgbaImage::from_pixel(w, h, image::Rgba([20, 90, 250, 0])).save(path).unwrap();
        } else {
            image::RgbImage::from_pixel(w, h, image::Rgb([20, 90, 250])).save(path).unwrap();
        }
    }

    fn decoded(read: &ImageRead) -> image::DynamicImage {
        use base64::Engine as _;
        image::load_from_memory(&base64::engine::general_purpose::STANDARD.decode(&read.data).unwrap()).unwrap()
    }

    #[test]
    fn only_a_picture_inside_the_worktree_or_the_squads_folder_is_given() {
        let root = dir("check");
        let (work, home, outside) = (root.join("work"), root.join("home"), root.join("outside"));
        for d in [&work, &home, &outside] {
            std::fs::create_dir_all(d.join("brand")).unwrap();
        }
        let roots = [work.clone(), home.clone()];
        png(&work.join("brand/a.png"), 4, 4, false);
        png(&home.join("logo.png"), 4, 4, false);
        png(&outside.join("secret.png"), 4, 4, false);
        std::fs::write(work.join("notes.txt"), "hi").unwrap();
        assert_eq!(check(&work.join("brand/a.png"), &roots).unwrap(), work.join("brand/a.png"));
        assert!(check(&home.join("logo.png"), &roots).is_ok());
        // Not a picture; outside; climbing out; relative; gone; a folder.
        assert!(check(&work.join("notes.txt"), &roots).unwrap_err().contains("not a picture"));
        assert!(check(&outside.join("secret.png"), &roots).unwrap_err().contains("outside"));
        assert!(check(&work.join("../outside/secret.png"), &roots).unwrap_err().contains("`..`"));
        assert!(check(Path::new("brand/a.png"), &roots).is_err(), "only absolute paths");
        assert!(check(&work.join("gone.png"), &roots).unwrap_err().contains("no such file"));
        std::fs::create_dir_all(work.join("dir.png")).unwrap();
        assert!(check(&work.join("dir.png"), &roots).unwrap_err().contains("not a file"));
        // Links: one out of the worktree, one to a file that isn't a picture, one that stays inside.
        symlink(outside.join("secret.png"), work.join("out.png")).unwrap();
        assert!(check(&work.join("out.png"), &roots).unwrap_err().contains("outside"), "a link is followed to where it goes");
        symlink(&outside, work.join("up")).unwrap();
        assert!(check(&work.join("up/secret.png"), &roots).unwrap_err().contains("outside"));
        symlink(work.join("notes.txt"), work.join("notes.png")).unwrap();
        assert!(check(&work.join("notes.png"), &roots).unwrap_err().contains("isn't a picture"));
        symlink(work.join("brand/a.png"), work.join("same.png")).unwrap();
        assert_eq!(check(&work.join("same.png"), &roots).unwrap(), work.join("brand/a.png"));
        // Too big.
        let big = std::fs::File::create(work.join("big.png")).unwrap();
        big.set_len(FILE_MAX + 1).unwrap();
        assert!(check(&work.join("big.png"), &roots).unwrap_err().contains("MB"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_path_a_member_gives_is_read_from_where_it_works() {
        let root = dir("resolve");
        let work = root.join("work");
        std::fs::create_dir_all(work.join("brand")).unwrap();
        png(&work.join("brand/a.png"), 4, 4, false);
        let roots = [work.clone()];
        let real = work.join("brand/a.png").to_string_lossy().into_owned();
        assert_eq!(resolve("brand/a.png", &work, &roots).unwrap(), real);
        assert_eq!(resolve("./brand/a.png", &work, &roots).unwrap(), real);
        assert_eq!(resolve(&format!("file://{real}"), &root, &roots).unwrap(), real);
        assert!(resolve("../work/brand/a.png", &work.join("brand"), &roots).unwrap_err().contains("`..`"));
        assert!(resolve_all(&["brand/a.png".into(), "brand/a.png".into()], &work, &roots, 4).unwrap().len() == 1, "the same one once");
        assert!(resolve_all(&vec!["brand/a.png".to_owned(); 5], &work, &roots, 4).unwrap_err().contains("At most 4"));
        assert!(resolve_all(&["brand/nope.png".into()], &work, &roots, 4).unwrap_err().contains("brand/nope.png"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_picture_goes_whole_or_as_a_thumbnail_and_bytes_must_be_what_the_name_says() {
        let root = dir("read");
        let roots = [root.clone()];
        png(&root.join("logo.png"), 1200, 600, true);
        let whole = read(&root.join("logo.png"), &roots, None).unwrap();
        assert_eq!((whole.mime.as_str(), whole.width, whole.height, whole.scaled), ("image/png", Some(1200), Some(600), false));
        let thumb = read(&root.join("logo.png"), &roots, Some(300)).unwrap();
        assert!(thumb.scaled && thumb.mime == "image/png", "transparency kept: {thumb:?}");
        assert_eq!((decoded(&thumb).width(), decoded(&thumb).height()), (300, 150));
        assert_eq!((thumb.width, thumb.height), (Some(1200), Some(600)), "the size it really is");
        // Opaque: a JPEG; small enough already: as it is.
        png(&root.join("photo.png"), 800, 800, false);
        assert_eq!(read(&root.join("photo.png"), &roots, Some(200)).unwrap().mime, "image/jpeg");
        assert!(!read(&root.join("photo.png"), &roots, Some(2000)).unwrap().scaled);
        // A text file named .png, and a PNG named .svg.
        std::fs::write(root.join("fake.png"), "not a picture").unwrap();
        assert!(read(&root.join("fake.png"), &roots, None).unwrap_err().contains("not a picture inside"));
        std::fs::copy(root.join("logo.png"), root.join("logo.svg")).unwrap();
        assert!(read(&root.join("logo.svg"), &roots, None).unwrap_err().contains("differ"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_svg_goes_as_it_is_unless_it_reaches_past_itself() {
        let root = dir("svg");
        let roots = [root.clone()];
        let drawn = r##"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><defs><circle id="d" r="2"/></defs><use href="#d"/></svg>"##;
        std::fs::write(root.join("e.svg"), drawn).unwrap();
        let read_svg = read(&root.join("e.svg"), &roots, Some(64)).unwrap();
        assert_eq!((read_svg.mime.as_str(), read_svg.width, read_svg.scaled), ("image/svg+xml", None, false));
        for bad in [
            r#"<svg><script>alert(1)</script></svg>"#,
            r#"<svg><image href="file:///etc/passwd"/></svg>"#,
            r#"<svg><image xlink:href="https://example.com/x.png"/></svg>"#,
            r#"<svg><foreignObject><div/></foreignObject></svg>"#,
            r#"<svg><style>@import url(http://x)</style></svg>"#,
        ] {
            std::fs::write(root.join("bad.svg"), bad).unwrap();
            assert!(read(&root.join("bad.svg"), &roots, None).is_err(), "{bad}");
        }
        std::fs::write(root.join("data.svg"), r#"<svg><image href="data:image/png;base64,AA=="/></svg>"#).unwrap();
        assert!(read(&root.join("data.svg"), &roots, None).is_ok(), "a picture inside itself is fine");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_hand_in_shows_what_its_member_named_then_the_images_its_branch_changed() {
        let root = dir("hand-in");
        std::fs::create_dir_all(root.join("brand")).unwrap();
        for f in ["brand/a.png", "brand/b.png", "brand/c.webp"] {
            png(&root.join(f), 4, 4, false);
        }
        std::fs::rename(root.join("brand/c.webp"), root.join("brand/c.png")).unwrap();
        let p = |f: &str| root.join(f).to_string_lossy().into_owned();
        let named = vec![p("brand/b.png")];
        let changed = ["src/app.css".to_owned(), "brand/a.png".into(), "brand/b.png".into(), "brand/gone.png".into()];
        assert_eq!(hand_in(&named, &changed, &root, std::slice::from_ref(&root)), [p("brand/b.png"), p("brand/a.png")], "named first, once each; what is gone left out");
        let many: Vec<String> = (0..20).map(|_| "brand/a.png".to_owned()).collect();
        assert!(hand_in(&[], &many, &root, std::slice::from_ref(&root)).len() == 1);
        // Landed in the squad's folder: where they are now.
        let moved = moved(&[p("brand/a.png"), "/elsewhere/x.png".into()], &root, Path::new("/h/squads/acme/home"));
        assert_eq!(moved, ["/h/squads/acme/home/brand/a.png", "/elsewhere/x.png"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_screen_gets_only_what_the_squad_shows() {
        let home = dir("serve");
        let journal = crate::journal::Journal::for_squad(&home, "acme").unwrap();
        let folder = crate::squad_home::path(&home, "acme");
        std::fs::create_dir_all(folder.join("brand")).unwrap();
        png(&folder.join("brand/a.png"), 40, 20, false);
        png(&folder.join("brand/other.png"), 4, 4, false);
        let shown_path = folder.join("brand/a.png").canonicalize().unwrap().to_string_lossy().into_owned();
        let choice = Choice { label: "A".into(), images: vec![shown_path.clone()], ..Default::default() };
        let set = crate::protocol::decide::QuestionSet {
            id: 1,
            session: 1,
            member: "designer".into(),
            ticket: None,
            questions: vec![Question { question: "Which logo?".into(), options: vec![choice, Choice { label: "B".into(), ..Default::default() }], ..Default::default() }],
            asked_at: 0,
        };
        journal.set_questions(&[set]).unwrap();
        let ask = |path: &str, max| ReadImage { squad: "acme".into(), path: path.into(), max };
        let got = serve(&home, &ask(&shown_path, None), None).unwrap();
        assert_eq!(got.width, Some(40));
        let other = folder.join("brand/other.png").canonicalize().unwrap().to_string_lossy().into_owned();
        assert!(serve(&home, &ask(&other, None), None).unwrap_err().contains("isn't a picture"), "a picture in the folder no one shows");
        assert!(serve(&home, &ReadImage { squad: "zeta".into(), ..ask(&shown_path, None) }, None).is_err(), "another squad");
        assert!(serve(&home, &ReadImage { squad: "../acme".into(), ..ask(&shown_path, None) }, None).is_err());
        // A phone's cap holds whatever it asks.
        let capped = serve(&home, &ask(&shown_path, Some(4000)), Some(16)).unwrap();
        assert!(capped.scaled && decoded(&capped).width() <= 32, "never below the smallest, never above the cap asked: {capped:?}");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_screen_gets_a_design_systems_pictures_and_nothing_else_of_its_folder() {
        let home = dir("design");
        crate::journal::Journal::for_squad(&home, "acme").unwrap();
        let design = crate::squad_home::path(&home, "acme").join("design");
        std::fs::create_dir_all(design.join("assets/logo")).unwrap();
        std::fs::write(design.join("tokens.json"), "{}").unwrap();
        png(&design.join("assets/logo/mark.png"), 20, 10, true);
        png(&design.join("loose.png"), 4, 4, false);
        png(&home.join("secret.png"), 4, 4, false);
        std::os::unix::fs::symlink(home.join("secret.png"), design.join("assets/logo/sneaky.png")).unwrap();
        let ask = |path: &Path| ReadImage { squad: "acme".into(), path: path.canonicalize().map_or_else(|_| path.to_string_lossy().into_owned(), |p| p.to_string_lossy().into_owned()), max: None };
        assert_eq!(serve(&home, &ask(&design.join("assets/logo/mark.png")), None).unwrap().width, Some(20));
        for refused in [design.join("loose.png"), design.join("tokens.json"), home.join("secret.png"), design.join("assets/logo/sneaky.png"), design.join("assets/logo/../../loose.png")] {
            assert!(serve(&home, &ask(&refused), None).is_err(), "{}", refused.display());
        }
        let mut climbing = ask(&design.join("assets/logo/mark.png"));
        climbing.path = format!("{}/assets/logo/../logo/mark.png", design.display());
        assert!(serve(&home, &climbing, None).is_err(), "asked by another name, with `..`");
        std::fs::remove_dir_all(home).unwrap();
    }

    /// A session of `squad` working in `cwd` that said `lines`.
    fn conversation(journal: &crate::journal::Journal, id: u64, cwd: &Path, lines: &[Entry]) {
        use crate::protocol::{Access, Engine};
        let record = crate::journal::SessionRecord { id, member: None, mode: None, engine: Engine::Claude, model: None, access: Access::EditFiles, cwd: cwd.to_owned(), repo: None, branch: None, base: None, engine_session: None, title: None, tokens: 0, open: true };
        journal.record_session(&record).unwrap();
        for line in lines {
            journal.append(id, line).unwrap();
        }
    }

    fn reply(text: &str) -> Entry {
        Entry::Reply { text: text.into() }
    }

    #[test]
    fn a_reply_shows_the_pictures_it_embeds_and_nothing_it_doesnt() {
        let home = dir("said");
        let journal = crate::journal::Journal::for_squad(&home, "acme").unwrap();
        let worktree = home.join("worktrees/photo-studio.home/t2-6ad338ba");
        let images = worktree.join("outputs/T-2 갤러리 디자인/images");
        std::fs::create_dir_all(&images).unwrap();
        for name in ["평면도.png", "배치도.png", "단면.png", "unsaid.png"] {
            png(&images.join(name), 40, 30, false);
        }
        std::fs::write(images.join("notes.txt"), "hi").unwrap();
        let outside = home.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        png(&outside.join("secret.png"), 4, 4, false);
        std::os::unix::fs::symlink(outside.join("secret.png"), images.join("link.png")).unwrap();
        let abs = images.join("평면도.png").to_string_lossy().into_owned();
        let text = format!(
            "시안입니다.\n\n![평면도]({abs})\n![배치도](outputs/T-2 갤러리 디자인/images/배치도.png \"A안\")\n![x]({}/secret.png)\n![l](outputs/T-2 갤러리 디자인/images/link.png)\n![t](outputs/T-2 갤러리 디자인/images/notes.txt)\n![up](../t2-6ad338ba/outputs/T-2 갤러리 디자인/images/단면.png)",
            outside.display()
        );
        conversation(&journal, 1, &worktree, &[Entry::User { text: "보여 주세요".into() }, reply(&text)]);
        let ask = |path: &Path| ReadImage { squad: "acme".into(), path: path.to_string_lossy().into_owned(), max: Some(64) };
        // Absolute, and relative to where the member works: given.
        assert_eq!(serve(&home, &ask(&images.join("평면도.png")), None).unwrap().width, Some(40));
        assert!(serve(&home, &ask(&images.join("배치도.png")), None).is_ok(), "relative, with spaces and a title");
        // Never said: refused, though it is right there.
        assert!(serve(&home, &ask(&images.join("unsaid.png")), None).unwrap_err().contains("isn't a picture"));
        // Said, but outside every root; a link out; not a picture; climbing.
        assert!(serve(&home, &ask(&outside.join("secret.png")), None).unwrap_err().contains("outside"));
        assert!(serve(&home, &ask(&images.join("link.png")), None).unwrap_err().contains("outside"), "a link is followed");
        assert!(serve(&home, &ask(&images.join("notes.txt")), None).is_err());
        assert!(serve(&home, &ask(&images.join("단면.png")), None).is_err(), "`..` is never read");
        let climbing = ReadImage { path: format!("{}/../t2-6ad338ba/outputs/T-2 갤러리 디자인/images/단면.png", worktree.display()), ..ask(&images) };
        assert!(serve(&home, &climbing, None).is_err());
        // Another squad's conversation names nothing of this one's.
        assert!(serve(&home, &ReadImage { squad: "zeta".into(), ..ask(&images.join("평면도.png")) }, None).is_err());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_reply_picture_in_one_of_the_squads_projects_or_a_scratch_folder_shows_and_nothing_else_there() {
        let home = dir("scratch");
        let journal = crate::journal::Journal::for_squad(&home, "acme").unwrap();
        let project = home.join("code/atinum-mcp");
        let scratch = home.join("tmp");
        let private = home.join("Pictures");
        for d in [&project, &scratch, &private] {
            std::fs::create_dir_all(d.join("docs")).unwrap();
        }
        journal.add_project(&project).unwrap();
        png(&project.join("docs/roadmap-erp.png"), 40, 30, false);
        png(&project.join("docs/unsaid.png"), 4, 4, false);
        png(&scratch.join("roadmap.png"), 40, 30, false);
        png(&private.join("passport.png"), 4, 4, false);
        let text = format!(
            "![ERP 로드맵]({})\n![r]({})\n![p]({})",
            project.join("docs/roadmap-erp.png").display(),
            scratch.join("roadmap.png").display(),
            private.join("passport.png").display()
        );
        conversation(&journal, 1, &home.join("squads/acme/home"), &[reply(&text)]);
        let ask = |path: &Path| ReadImage { squad: "acme".into(), path: path.to_string_lossy().into_owned(), max: Some(64) };
        let serve = |path: &Path| serve_with(&home, &ask(path), None, std::slice::from_ref(&scratch));
        // A reply's picture in one of the squad's projects, or in a scratch folder: shown.
        assert_eq!(serve(&project.join("docs/roadmap-erp.png")).unwrap().width, Some(40));
        assert_eq!(serve(&scratch.join("roadmap.png")).unwrap().width, Some(40));
        // Never said, though right there; said, but somewhere else of the person's.
        assert!(serve(&project.join("docs/unsaid.png")).unwrap_err().contains("isn't a picture"));
        let outside = serve(&private.join("passport.png")).unwrap_err();
        assert_eq!(rpc::ImageWhy::of(&outside), rpc::ImageWhy::Outside, "{outside}");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_folder_a_reply_points_at_lists_its_pictures_and_they_show() {
        let home = dir("folder");
        let journal = crate::journal::Journal::for_squad(&home, "acme").unwrap();
        let worktree = home.join("worktrees/photo-studio.home/t2-6ad338ba");
        let images = worktree.join("outputs/T-2 갤러리 디자인/images");
        std::fs::create_dir_all(images.join("sub")).unwrap();
        for n in 0..15 {
            png(&images.join(format!("{n:02}.png")), 4, 4, false);
        }
        png(&images.join("sub/deeper.png"), 4, 4, false);
        std::fs::write(images.join("readme.md"), "#").unwrap();
        std::fs::write(images.join(".hidden.png"), "x").unwrap();
        let elsewhere = worktree.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        png(&elsewhere.join("a.png"), 4, 4, false);
        let folder = images.to_string_lossy().into_owned();
        conversation(&journal, 1, &worktree, &[reply(&format!("이미지가 화면에 안 보이면 이 폴더를 열어 보세요: {folder}/"))]);
        let list_of = |folder: &str, max| list(&home, &ListImages { squad: "acme".into(), folder: folder.into(), max });
        let listed = list_of(&folder, None).unwrap();
        assert_eq!((listed.total, listed.images.len()), (15, LIST_MAX), "pictures only, at most twelve");
        assert_eq!(listed.images[0], format!("{folder}/00.png"));
        assert_eq!(list_of(&format!("{folder}/"), Some(3)).unwrap().images.len(), 3, "its slash, and fewer asked");
        // Each listed picture shows; one in a folder under it doesn't, nor one in a folder never named.
        let ask = |path: String| ReadImage { squad: "acme".into(), path, max: Some(32) };
        assert!(serve(&home, &ask(listed.images[11].clone()), None).is_ok());
        assert!(serve(&home, &ask(format!("{folder}/14.png")), None).is_ok(), "past the twelve, still in the folder");
        assert!(serve(&home, &ask(format!("{folder}/sub/deeper.png")), None).is_err());
        assert!(serve(&home, &ask(elsewhere.join("a.png").to_string_lossy().into_owned()), None).is_err());
        // A folder never named, one climbing, one relative.
        assert!(list_of(&elsewhere.to_string_lossy(), None).unwrap_err().contains("isn't a folder"));
        assert!(list_of(&format!("{folder}/../images"), None).is_err());
        assert!(list_of("outputs/images", None).is_err());
        // Named, but outside every root.
        let outside = home.join("outside/pics");
        std::fs::create_dir_all(&outside).unwrap();
        png(&outside.join("a.png"), 4, 4, false);
        conversation(&journal, 2, &home, &[reply(&format!("see {}/", outside.display()))]);
        assert!(list_of(&outside.to_string_lossy(), None).unwrap_err().contains("outside"));
        std::fs::remove_dir_all(home).unwrap();
    }

    /// Run git in `dir`, as a test does.
    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// A project with a ticket's branch whose worktree is gone: its pictures
    /// recorded where the worktree was. Returns the project and those paths.
    fn accepted_unmerged(home: &Path) -> (PathBuf, Vec<String>) {
        let project = home.join("code/gallery-y");
        std::fs::create_dir_all(&project).unwrap();
        git(&project, &["init", "-q", "-b", "main"]);
        git(&project, &["config", "user.name", "T"]);
        git(&project, &["config", "user.email", "t@example.com"]);
        std::fs::write(project.join("README.md"), "#").unwrap();
        git(&project, &["add", "."]);
        git(&project, &["commit", "-qm", "start"]);
        let worktree = crate::worktree::worktrees_of(&project, home).join("t4-6ad33ac5");
        std::fs::create_dir_all(worktree.parent().unwrap()).unwrap();
        git(&project, &["worktree", "add", "-q", "-b", "equipe/frontend-t4-6ad33ac5", &worktree.to_string_lossy()]);
        std::fs::create_dir_all(worktree.join("outputs")).unwrap();
        std::fs::create_dir_all(worktree.join("public/textures/earth")).unwrap();
        png(&worktree.join("outputs/T-4-globe.png"), 60, 30, false);
        png(&worktree.join("public/textures/earth/earth_day_2048.png"), 20, 10, false);
        std::fs::write(worktree.join("outputs/fake.png"), "not a picture").unwrap();
        git(&worktree, &["add", "."]);
        git(&worktree, &["commit", "-qm", "T-4"]);
        let images = ["outputs/T-4-globe.png", "public/textures/earth/earth_day_2048.png", "outputs/fake.png"].map(|f| worktree.canonicalize().unwrap().join(f).to_string_lossy().into_owned()).to_vec();
        git(&project, &["worktree", "remove", "--force", &worktree.to_string_lossy()]);
        assert!(!worktree.exists());
        (project, images)
    }

    fn save_ticket(home: &Path, project: &Path, images: &[String], merged: Option<bool>, branch: Option<&str>) {
        let journal = crate::journal::Journal::for_squad(home, "acme").unwrap();
        let mut ticket = Ticket {
            id: 0,
            title: "Globe".into(),
            body: "".into(),
            status: crate::protocol::TicketStatus::Done,
            kind: crate::protocol::TicketKind::Code,
            owner: Some("frontend".into()),
            assignee: Some("frontend".into()),
            project: Some(project.to_owned()),
            depends_on: vec![],
            branch: branch.map(str::to_owned),
            worktree: None,
            merged,
            off_main: None,
            design_qa: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            remaining: vec![],
            images: vec![],
            priority: Default::default(),
            rank: 0,
            updated_at: None,
        };
        ticket.id = journal.create_ticket(&ticket).unwrap();
        ticket.images = images.to_vec();
        journal.save_ticket(&ticket).unwrap();
    }

    #[test]
    fn an_accepted_tickets_pictures_come_from_its_branch_once_its_worktree_is_gone() {
        let home = dir("kept");
        let (project, images) = accepted_unmerged(&home);
        save_ticket(&home, &project, &images, Some(false), Some("equipe/frontend-t4-6ad33ac5"));
        let ask = |path: &str| ReadImage { squad: "acme".into(), path: path.into(), max: Some(32) };
        let globe = serve(&home, &ask(&images[0]), None).unwrap();
        assert_eq!((globe.width, globe.height, globe.scaled), (Some(60), Some(30), true), "from the branch, scaled as ever");
        assert!(serve(&home, &ask(&images[1]), None).is_ok());
        // The type is still checked on what git holds.
        assert!(serve(&home, &ask(&images[2]), None).unwrap_err().contains("not a picture inside"));
        // A path of that worktree the ticket never recorded: refused, though the branch has it.
        let readme = images[0].replace("outputs/T-4-globe.png", "README.md");
        assert!(serve(&home, &ask(&readme), None).unwrap_err().contains("isn't a picture"));
        let unrecorded = images[0].replace("T-4-globe", "T-4-other");
        assert!(serve(&home, &ask(&unrecorded), None).unwrap_err().contains("isn't a picture"));
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_merged_tickets_pictures_come_from_main_or_the_checkout_and_a_missing_one_says_so() {
        let home = dir("merged");
        let (project, images) = accepted_unmerged(&home);
        git(&project, &["merge", "-q", "--no-ff", "-m", "T-4", "equipe/frontend-t4-6ad33ac5"]);
        git(&project, &["branch", "-q", "-D", "equipe/frontend-t4-6ad33ac5"]);
        let mut recorded = images.clone();
        recorded.push(images[0].replace("T-4-globe", "T-4-gone"));
        save_ticket(&home, &project, &recorded, Some(true), None);
        let ask = |path: &str| ReadImage { squad: "acme".into(), path: path.into(), max: None };
        assert_eq!(serve(&home, &ask(&recorded[0]), None).unwrap().width, Some(60), "the checkout's file");
        // Gone from the checkout, still on main.
        std::fs::remove_file(project.join("outputs/T-4-globe.png")).unwrap();
        assert_eq!(serve(&home, &ask(&recorded[0]), None).unwrap().width, Some(60), "main's");
        let missing = serve(&home, &ask(&recorded[3]), None).unwrap_err();
        assert_eq!(rpc::ImageWhy::of(&missing), rpc::ImageWhy::Missing, "{missing}");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_hand_in_shows_its_work_before_its_assets_and_each_asset_once() {
        let at = |f: &str| format!("/w/t4/{f}");
        let images: Vec<String> = [
            "public/textures/earth/earth_day_4096.jpg",
            "outputs/T-4-globe.png",
            "public/textures/earth/earth_day_2048.jpg",
            "public/textures/earth/earth_night_2048.jpg",
            "public/textures/earth/earth_night_4096.jpg",
            "outputs/T-4-flying.png",
            "src/assets/logo@2x.png",
            "src/assets/logo.png",
        ]
        .map(at)
        .to_vec();
        let arranged = arrange(&images);
        assert_eq!(arranged.work, [at("outputs/T-4-globe.png"), at("outputs/T-4-flying.png")]);
        assert_eq!(arranged.assets, [at("public/textures/earth/earth_day_4096.jpg"), at("public/textures/earth/earth_night_4096.jpg"), at("src/assets/logo@2x.png")], "the biggest of each");
        assert!(!is_asset("/w/t4/outputs/textures/a.png") && is_asset("/w/t4/static/a.png") && !is_asset("/w/t4/a.png"));
        assert_eq!(variant("/w/a_1024x768.png"), ("/w/a.png".to_owned(), 1024));
        assert_eq!(variant("/w/plan-2.png").1, 0, "a number that's no size stays");
    }

    #[test]
    fn a_visual_question_without_pictures_gets_a_word_to_the_member() {
        let q = |question: &str, labels: &[&str]| Question {
            question: question.into(),
            options: labels.iter().map(|l| Choice { label: (*l).into(), ..Default::default() }).collect(),
            ..Default::default()
        };
        let logo = q("로고 마스터는 어느 안으로 할까요?", &["e. + 파란 원 점 (추천)", "e. 회색 점 유지"]);
        let text = nudge(std::slice::from_ref(&logo)).unwrap();
        assert!(text.contains("`images`") && text.contains("Render or mock up"), "{text}");
        assert!(nudge(&[q("Which colour palette?", &["Warm", "Cool"])]).is_some());
        assert!(nudge(&[q("Which database?", &["Postgres", "SQLite"])]).is_none());
        assert!(nudge(&[q("주문 검색을 넣을까요?", &["네", "아니요"])]).is_none(), "검색 is no colour");
        assert!(nudge(&[q("Which logo file name?", &[])]).is_none(), "nothing to pick from: nothing to show");
        let mut shown = logo;
        shown.options[0].images = vec!["/w/a.png".into()];
        assert!(nudge(&[shown]).is_none());
    }
}
