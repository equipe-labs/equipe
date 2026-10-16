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

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use crate::protocol::decide::Question;
use crate::protocol::rpc::{ImageRead, ReadImage};

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
        return Err(refused(path, "outside your worktree and the squad's folder"));
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
    use base64::Engine as _;
    use std::io::Read as _;
    let real = check(path, roots)?;
    let mut bytes = Vec::new();
    std::fs::File::open(&real).and_then(|f| f.take(FILE_MAX + 1).read_to_end(&mut bytes)).map_err(|_| refused(path, "couldn't be read"))?;
    let size = bytes.len() as u64;
    let named = mime_of(&real).unwrap_or_default();
    let mime = sniff(&bytes).ok_or_else(|| refused(path, "not a picture inside"))?;
    let svg = named == "image/svg+xml";
    if svg != (mime == "image/svg+xml") {
        return Err(refused(path, "its name and what it holds differ"));
    }
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

/// What a squad shows now: the pictures its waiting decisions and its
/// tickets name, and where they may be — its folder, its tickets' worktrees.
#[derive(Debug, Default)]
pub struct Shown {
    pub paths: HashSet<String>,
    pub roots: Vec<PathBuf>,
}

/// Read from the squad's journal; nothing for a squad that isn't there.
pub fn shown(home: &Path, squad: &str) -> Shown {
    let db = crate::journal::squad_dir(home, squad).join("squad.db");
    if !crate::squads::valid_name(squad) || !db.exists() {
        return Shown::default();
    }
    let Ok(journal) = crate::journal::Journal::open(&db) else { return Shown::default() };
    let mut out = Shown { roots: vec![crate::squad_home::path(home, squad)], ..Shown::default() };
    for set in journal.questions() {
        out.paths.extend(set.images().map(str::to_owned));
    }
    for ticket in journal.tickets().unwrap_or_default() {
        out.paths.extend(ticket.images.iter().cloned());
        out.roots.extend(ticket.worktree);
    }
    out
}

/// Answer a `read_image` for `ask.squad`, at most `cap` pixels on a side
/// when given (a phone's).
pub fn serve(home: &Path, ask: &ReadImage, cap: Option<u32>) -> Result<ImageRead, String> {
    let shown = shown(home, &ask.squad);
    if !shown.paths.contains(&ask.path) {
        return Err(format!("{} isn't a picture {} shows", ask.path, ask.squad));
    }
    let max = match (ask.max, cap) {
        (Some(max), Some(cap)) => Some(max.min(cap)),
        (max, cap) => max.or(cap),
    };
    read(Path::new(&ask.path), &shown.roots, max)
}

/// What a hand-in shows: the pictures its member named, then the images its
/// branch changed (`changed`, relative to `worktree`) that pass [`check`] —
/// at most [`HAND_IN`].
pub fn hand_in(named: &[String], changed: &[String], worktree: &Path, roots: &[PathBuf]) -> Vec<String> {
    let mut out: Vec<String> = named.to_vec();
    for rel in changed.iter().filter(|f| is_image_name(f)) {
        if let Ok(real) = check(&worktree.join(rel), roots) {
            let real = real.to_string_lossy().into_owned();
            if !out.contains(&real) {
                out.push(real);
            }
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
const VISUAL_WORDS: [&str; 44] = [
    "logo", "icon", "color", "colour", "palette", "mockup", "mock-up", "wireframe", "layout", "font", "typeface", "typography",
    "illustration", "banner", "visual", "로고", "아이콘", "색상", "컬러", "배색", "시안", "목업", "와이어프레임", "레이아웃",
    "폰트", "서체", "일러스트", "배너", "ロゴ", "アイコン", "配色", "フォント", "レイアウト", "图标", "標誌", "标志", "配色",
    "字体", "字體", "布局", "版面", "farbe", "couleur", "police",
];

/// The word a tool's answer adds when a question names something to look at
/// — a logo, colours, a layout — yet shows no picture: the person would be
/// choosing blind. Only ever a word to the member; the question still goes.
pub fn nudge(questions: &[Question]) -> Option<&'static str> {
    if questions.iter().any(|q| q.images().next().is_some()) {
        return None;
    }
    let names_visuals = questions.iter().filter(|q| !q.options.is_empty()).any(|q| {
        let text = std::iter::once(q.question.as_str()).chain(std::iter::once(q.header.as_str())).chain(q.options.iter().flat_map(|c| [c.label.as_str(), c.description.as_str()])).collect::<Vec<_>>().join(" ").to_lowercase();
        VISUAL_WORDS.iter().any(|w| text.contains(w))
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
