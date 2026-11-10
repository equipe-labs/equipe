//! Diagrams in a reply, drawn here for a screen that can't draw them itself
//! (a phone): each ```` ```mermaid ```` block becomes two PNGs, light and
//! dark, kept in the squad's `visuals/` by a key of its source, and told to
//! the screens as `visuals_rendered` (docs/VISUALS.md). The desktop draws its
//! own from the same SVG ([`svg`]), so a diagram looks the same everywhere.
//! A block that won't draw is simply not told: every screen shows its source.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::protocol::{Entry, Rendered, RenderedImage};

/// The longest source drawn, in bytes.
pub const SOURCE_MAX: usize = 20_000;
/// How many blocks of one reply are drawn at most.
pub const PER_REPLY: usize = 8;
/// Drawn at twice its size, so it is sharp on a phone's screen…
const SCALE: f32 = 2.;
/// …but never more than this many pixels on its longer side.
pub const SIDE_MAX: u32 = 2048;
/// A PNG bigger than this isn't kept (a phone gets a smaller copy anyway).
pub const PNG_MAX: usize = 4 << 20;

/// The source of each Mermaid block of `text`, in order: a fence of three or
/// more backticks or tildes, `mermaid` its first word, up to its closing
/// fence. An unclosed block is still being written: not a diagram yet.
pub fn blocks(text: &str) -> Vec<String> {
    fenced(text, |info| info.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("mermaid")))
}

/// The source of each fenced block of `text` whose info string `wanted`
/// takes, in order, as CommonMark has fences: three or more backticks or
/// tildes, indented at most three spaces, closed by a fence of the same
/// character at least as long with nothing after it. Unclosed: not yet.
pub(crate) fn fenced(text: &str, wanted: impl Fn(&str) -> bool) -> Vec<String> {
    let mut found = Vec::new();
    let mut open: Option<(char, usize, bool, Vec<&str>)> = None;
    for line in text.lines() {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let fence = (indent <= 3).then(|| line.trim_start_matches(' ')).and_then(|rest| {
            let mark = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
            let n = rest.chars().take_while(|c| *c == mark).count();
            (n >= 3).then(|| (mark, n, rest[n..].trim()))
        });
        match (&mut open, fence) {
            (Some((mark, n, _, _)), Some((m, k, info))) if m == *mark && k >= *n && info.is_empty() => {
                let (_, _, keep, lines) = open.take().unwrap();
                if keep {
                    found.push(lines.join("\n"));
                }
            }
            (Some((_, _, _, lines)), _) => lines.push(line),
            (None, Some((mark, n, info))) => open = Some((mark, n, wanted(info), Vec::new())),
            (None, None) => {}
        }
    }
    found
}

/// A block's key: the first 8 bytes of the SHA-256 of its source, trimmed,
/// in lowercase hex. A screen makes the same of the block it shows.
pub fn key(source: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, source.trim().as_bytes());
    digest.as_ref()[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Where a squad's drawn diagrams are kept.
pub fn dir(home: &Path, squad: &str) -> PathBuf {
    crate::journal::squad_dir(home, squad).join("visuals")
}

pub(crate) fn file(home: &Path, squad: &str, key: &str, dark: bool) -> PathBuf {
    dir(home, squad).join(format!("{key}-{}.png", if dark { "dark" } else { "light" }))
}

/// Whether `path` is one of the squad's drawn diagrams, by its name alone:
/// right in its `visuals/`, a key and a tone.
pub fn is_drawn(home: &Path, squad: &str, path: &Path) -> bool {
    let named = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".png")).and_then(|n| n.strip_suffix("-light").or_else(|| n.strip_suffix("-dark")));
    named.is_some_and(|k| k.len() == 16 && k.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))) && path.parent() == Some(dir(home, squad).as_path())
}

/// The faces a diagram's words are drawn in, by the language it is written
/// in: Helvetica for Latin, then the Mac's face for its script — Korean in
/// Apple SD Gothic Neo, Japanese in Hiragino Sans, Chinese in PingFang. Each of
/// those also has kana and Han, so a diagram in no one language lists them in
/// the order that gives every script its own: Hiragino (kana, and the Han they
/// share), PingFang (Han only Chinese writes), Apple SD Gothic Neo (Hangul).
pub fn font_family(source: &str) -> &'static str {
    match crate::language::detect(source) {
        Some("Korean") => "\"Helvetica Neue\", \"Apple SD Gothic Neo\", sans-serif",
        Some("Japanese") => "\"Helvetica Neue\", \"Hiragino Sans\", sans-serif",
        Some("Chinese") => "\"Helvetica Neue\", \"PingFang SC\", sans-serif",
        _ => "\"Helvetica Neue\", \"Hiragino Sans\", \"PingFang SC\", \"Apple SD Gothic Neo\", sans-serif",
    }
}

/// The diagram's SVG, in colours that sit on the conversation's background.
pub fn svg(source: &str, dark: bool) -> Result<String, String> {
    use mermaid_rs_renderer::{RenderOptions, Theme, render_with_options};
    let mut theme = if dark { Theme::dark() } else { Theme::modern() };
    theme.font_family = font_family(source).into();
    theme.font_size = 14.;
    if dark {
        theme.background = "transparent".into();
        theme.primary_color = "#1b1d22".into();
        theme.primary_border_color = "#7c7e85".into();
        theme.primary_text_color = "#efede7".into();
        theme.text_color = "#cfccc4".into();
        theme.line_color = "#9a9ca3".into();
        theme.cluster_background = "#141417".into();
        theme.edge_label_background = "#141417".into();
    } else {
        theme.background = "transparent".into();
    }
    let options = RenderOptions { theme, ..RenderOptions::default() };
    // The renderer panics on some malformed input; a bad diagram is its source.
    let svg = std::panic::catch_unwind(|| render_with_options(source, options)).map_err(|_| "mermaid failed".to_owned())?.map_err(|e| e.to_string())?;
    Ok(if dark { quadrants_in_the_dark(&svg) } else { svg })
}

/// A quadrant chart's own colours are fixed light ones whatever the theme —
/// pale squares, near-black words — so on a dark screen its axis labels go
/// unread: they take the theme's tones instead.
fn quadrants_in_the_dark(svg: &str) -> String {
    const LIGHT_TO_DARK: [(&str, &str); 6] = [
        ("\"#ECECFF\"", "\"#262830\""),
        ("\"#f1f1ff\"", "\"#22242b\""),
        ("\"#f6f6ff\"", "\"#1e2026\""),
        ("\"#fbfbff\"", "\"#1b1c21\""),
        ("\"#c7c7f1\"", "\"#4a4c55\""),
        ("\"#131300\"", "\"#cfccc4\""),
    ];
    LIGHT_TO_DARK.iter().fold(svg.to_owned(), |svg, (light, dark)| svg.replace(light, dark))
}

/// The Mac's fonts, read once: a diagram's words are drawn in them.
fn fonts() -> Arc<resvg::usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = resvg::usvg::fontdb::Database::new();
            db.load_system_fonts();
            db.set_sans_serif_family("Helvetica Neue");
            Arc::new(db)
        })
        .clone()
}

/// `svg` as a PNG on a transparent ground, and its size in pixels.
pub fn png_of_svg(svg: &str) -> Result<(Vec<u8>, u32, u32), String> {
    use resvg::{tiny_skia, usvg};
    let options = usvg::Options { fontdb: fonts(), ..usvg::Options::default() };
    let tree = usvg::Tree::from_str(svg, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    let scale = SCALE.min(SIDE_MAX as f32 / size.width().max(size.height()).max(1.));
    let (width, height) = ((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32);
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or("empty picture")?;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut()))).map_err(|_| "drawing failed".to_owned())?;
    let png = pixmap.encode_png().map_err(|e| e.to_string())?;
    if png.len() > PNG_MAX {
        return Err(format!("{} bytes, bigger than {PNG_MAX}", png.len()));
    }
    Ok((png, width, height))
}

/// The block drawn in one tone, kept in the squad's `visuals/` (once: a
/// key drawn before is read back).
fn draw_one(home: &Path, squad: &str, key: &str, source: &str, dark: bool) -> Result<RenderedImage, String> {
    let path = file(home, squad, key, dark);
    if let Some(kept) = read_size(&path) {
        super::cache::touch(&path);
        return Ok(kept);
    }
    let (png, width, height) = png_of_svg(&svg(source, dark)?)?;
    std::fs::create_dir_all(dir(home, squad)).map_err(|e| e.to_string())?;
    // Whole or not at all: a screen never reads half a picture.
    let partial = path.with_extension("part");
    std::fs::write(&partial, &png).and_then(|()| std::fs::rename(&partial, &path)).map_err(|e| e.to_string())?;
    Ok(RenderedImage { path: path.to_string_lossy().into_owned(), width, height })
}

/// A kept PNG's path and size, read from its header.
pub(crate) fn read_size(path: &Path) -> Option<RenderedImage> {
    use std::io::Read;
    let mut head = [0u8; 24];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    if !head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let width = u32::from_be_bytes(head[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(head[20..24].try_into().ok()?);
    Some(RenderedImage { path: path.to_string_lossy().into_owned(), width, height })
}

/// Each block of `text` that drew, in both tones. A block too long, past
/// [`PER_REPLY`], or one either tone failed on, is left out: its source is shown.
pub fn draw(home: &Path, squad: &str, text: &str) -> Vec<Rendered> {
    let mut drawn: Vec<Rendered> = Vec::new();
    for source in blocks(text).into_iter().take(PER_REPLY) {
        let key = key(&source);
        if source.trim().is_empty() || source.len() > SOURCE_MAX || drawn.iter().any(|r| r.key == key) {
            continue;
        }
        match draw_one(home, squad, &key, &source, false).and_then(|light| Ok((light, draw_one(home, squad, &key, &source, true)?))) {
            Ok((light, dark)) => drawn.push(Rendered { kind: "mermaid".into(), key, light, dark, page: None }),
            Err(why) => crate::daemon_log!("equipe: a diagram of {squad} wasn't drawn ({why}); its source shows"),
        }
    }
    drawn
}

/// The blocks of these lines' replies drawn before, read from the squad's
/// `visuals/` without drawing anything: for a screen that attaches later.
pub fn kept(home: &Path, squad: &str, lines: &[Entry]) -> Vec<Rendered> {
    let mut found: Vec<Rendered> = Vec::new();
    for line in lines {
        let Entry::Reply { text } = line else { continue };
        for source in blocks(text).into_iter().take(PER_REPLY) {
            let key = key(&source);
            if found.iter().any(|r| r.key == key) {
                continue;
            }
            if let (Some(light), Some(dark)) = (read_size(&file(home, squad, &key, false)), read_size(&file(home, squad, &key, true))) {
                found.push(Rendered { kind: "mermaid".into(), key, light, dark, page: None });
            }
        }
    }
    found
}

/// Whether a reply has anything here to draw.
pub fn has_blocks(text: &str) -> bool {
    text.contains("mermaid") && !blocks(text).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOW: &str = "flowchart LR\n  A[Ask] --> B[Draw]\n  B --> C[Show]";

    #[test]
    fn a_diagrams_words_take_the_face_of_their_own_script() {
        let face = |source: &str| font_family(source).split(", ").nth(1).unwrap().trim_matches('"').to_owned();
        assert_eq!(face("flowchart LR\n  A[요청] --> B[그리기]"), "Apple SD Gothic Neo", "Korean as it always was");
        assert_eq!(face("flowchart LR\n  A[見積もり] --> B[確認する]"), "Hiragino Sans");
        assert_eq!(face("flowchart LR\n  A[需求] --> B[设计]"), "PingFang SC");
        assert_eq!(face(FLOW), "Hiragino Sans", "no one language: each script to its own, Hangul last");
        assert!(font_family(FLOW).ends_with("\"Apple SD Gothic Neo\", sans-serif"));
        assert!(svg("flowchart LR\n  A[見積もり] --> B[確認する]", false).unwrap().contains("Hiragino Sans"));
    }

    #[test]
    fn mermaid_blocks_are_found_by_their_fence_and_others_left() {
        let text = format!("Here:\n\n```mermaid\n{FLOW}\n```\n\n```rust\nfn main() {{}}\n```\n~~~~ Mermaid\npie\n  \"a\" : 1\n~~~~\n```mermaid\nflowchart TD\n  still --> writing");
        assert_eq!(blocks(&text), vec![FLOW.to_owned(), "pie\n  \"a\" : 1".to_owned()]);
        assert!(blocks("```\nmermaid\n```").is_empty());
        // A shorter fence inside doesn't close it.
        assert_eq!(blocks("````mermaid\nflowchart LR\n```\nA-->B\n````"), vec!["flowchart LR\n```\nA-->B".to_owned()]);
    }

    #[test]
    fn a_key_is_its_trimmed_sources_and_sixteen_hex_digits() {
        assert_eq!(key(FLOW), key(&format!("\n{FLOW}\n  ")));
        assert_ne!(key(FLOW), key("pie"));
        assert_eq!(key(FLOW).len(), 16);
        // The same on every screen: SHA-256("pie"), its first 8 bytes.
        assert_eq!(key("pie"), "558211ed72b2d696");
    }

    #[test]
    fn a_diagram_is_drawn_light_and_dark_kept_and_read_back() {
        let home = tempfile_dir("draw");
        let text = format!("The flow:\n```mermaid\n{FLOW}\n```\nThat's all.");
        let drawn = draw(&home, "acme", &text);
        assert_eq!(drawn.len(), 1, "{drawn:?}");
        let r = &drawn[0];
        assert_eq!((r.kind.as_str(), r.key.as_str()), ("mermaid", key(FLOW).as_str()));
        for image in [&r.light, &r.dark] {
            let bytes = std::fs::read(&image.path).unwrap();
            assert!(bytes.starts_with(b"\x89PNG"));
            assert!(image.width > 100 && image.height > 20 && image.width.max(image.height) <= SIDE_MAX);
            assert!(is_drawn(&home, "acme", Path::new(&image.path)));
            assert!(!is_drawn(&home, "other", Path::new(&image.path)));
        }
        assert_ne!(std::fs::read(&r.light.path).unwrap(), std::fs::read(&r.dark.path).unwrap());
        assert_eq!(kept(&home, "acme", &[Entry::Reply { text }]), drawn);
        assert!(kept(&home, "acme", &[Entry::Reply { text: "```mermaid\npie\n```".into() }]).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_diagram_that_wont_draw_is_left_out() {
        let home = tempfile_dir("fail");
        assert!(draw(&home, "acme", "```mermaid\nthis is not a diagram {{{ --> \n```").is_empty());
        assert!(draw(&home, "acme", &format!("```mermaid\nflowchart LR\n{}\n```", "A-->B\n".repeat(SOURCE_MAX / 6 + 1))).is_empty());
        assert!(draw(&home, "acme", "no diagram here").is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn nothing_but_a_drawn_diagrams_name_counts_as_one() {
        let home = Path::new("/h");
        let dir = dir(home, "acme");
        assert!(is_drawn(home, "acme", &dir.join("0123456789abcdef-dark.png")));
        assert!(!is_drawn(home, "acme", &dir.join("0123456789ABCDEF-dark.png")));
        assert!(!is_drawn(home, "acme", &dir.join("0123456789abcdef.png")));
        assert!(!is_drawn(home, "acme", &dir.join("sub/0123456789abcdef-dark.png")));
        assert!(!is_drawn(home, "acme", &dir.join("../0123456789abcdef-dark.png")));
    }

    #[test]
    fn a_quadrant_chart_on_a_dark_screen_is_drawn_in_its_tones_and_read_in_light_words() {
        let source = "quadrantChart\n  title Depth vs system\n  x-axis Low --> High\n  y-axis Shallow --> Deep\n  rea: [0.25, 0.7]\n  ours: [0.85, 0.85]\n";
        let dark = svg(source, true).expect("drawn");
        for light in ["#ECECFF", "#f1f1ff", "#f6f6ff", "#fbfbff", "#131300"] {
            assert!(!dark.contains(light), "{light} left in the dark");
        }
        assert!(dark.contains("#cfccc4"), "the axis labels in the theme's words");
        assert!(svg(source, false).expect("drawn").contains("#131300"), "a light screen keeps them");
    }

    fn tempfile_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-diagram-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }
}
