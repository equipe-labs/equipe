//! A design session's page checked before it is sent to the person's screen
//! (`equipe design session show`, docs/DESIGN-SESSIONS.md): every local file
//! it names — stylesheets, scripts, pictures, CSS `@import`s and `url()`s —
//! must be where the page server serves it from, and the page, opened as the
//! person's screen opens it and photographed, must load, be styled and not
//! blank. What fails is told to the designer instead of "sent", with the
//! picture, so it never says "A and B are side by side" over a broken page.
//!
//! What the server reaches (its scope): the page's `design/` folder, the
//! nearest one above it, else the page's own folder — and the project
//! around that `design/` for an address from the root (`/assets/logo.svg`),
//! as [`root_absolute`] says and `crate::prototype` serves.

use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

/// Where a page's files are served from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// The folder served: the page's `design/`, else its own folder.
    pub served: PathBuf,
    /// The project around that `design/`: where an address from the root
    /// is looked for after the design folder.
    pub project: Option<PathBuf>,
}

impl Scope {
    /// The scope of a page served from `served`.
    pub fn of_served(served: &Path) -> Scope {
        let project = (served.file_name().is_some_and(|n| n == "design")).then(|| served.parent().map(Path::to_path_buf)).flatten();
        Scope { served: served.to_path_buf(), project }
    }

    /// The scope of `page` (canonical): its nearest `design/` above, else its folder.
    pub fn of_page(page: &Path) -> Scope {
        let own = page.parent().unwrap_or(page);
        let served = own.ancestors().find(|a| a.file_name().is_some_and(|n| n == "design")).unwrap_or(own);
        Scope::of_served(served)
    }

    fn reaches(&self, path: &Path) -> bool {
        path.starts_with(&self.served) || self.project.as_ref().is_some_and(|p| path.starts_with(p))
    }
}

/// A file named from the root (`/tokens.css`, `/design/x.css`,
/// `/assets/logo.svg`) as the page server finds it: in the served folder,
/// then in the project around it. `None` when it is in neither.
pub fn root_absolute(rel: &str, scope: &Scope) -> Option<PathBuf> {
    let rel = clean(rel.trim_start_matches('/'))?;
    std::iter::once(&scope.served).chain(scope.project.as_ref()).map(|base| base.join(&rel)).find(|p| p.is_file())
}

/// `rel` with no way out of where it is joined: `None` for `..` or an absolute part.
fn clean(rel: &str) -> Option<PathBuf> {
    let path = Path::new(rel);
    path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)).then(|| path.to_path_buf())
}

/// A local file a page names that the person's screen won't get.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    /// As the file names it.
    pub reference: String,
    /// The file that names it.
    pub from: PathBuf,
    /// Where it was looked for.
    pub looked: Option<PathBuf>,
    /// There, but outside what the page server serves.
    pub outside: bool,
}

impl Missing {
    pub fn words(&self) -> String {
        let from = self.from.file_name().map_or_else(|| self.from.display().to_string(), |n| n.to_string_lossy().into_owned());
        match (&self.looked, self.outside) {
            (Some(at), true) => format!("`{}` (in {from}) is at {}, outside the folder served to the person's screen", self.reference, at.display()),
            (Some(at), false) => format!("`{}` (in {from}) isn't there: no {}", self.reference, at.display()),
            (None, _) => format!("`{}` (in {from}) is in neither the design folder nor the project", self.reference),
        }
    }
}

/// The most files followed from one page (its stylesheets' imports included).
const FILES_MAX: usize = 64;

/// Every local file `page` (and the stylesheets it loads, in turn) names
/// that the page server can't serve it: not there, or outside its scope.
pub fn missing_assets(page: &Path, scope: &Scope) -> Vec<Missing> {
    let mut missing = Vec::new();
    let mut todo = vec![page.to_path_buf()];
    let mut seen: Vec<PathBuf> = Vec::new();
    while let Some(file) = todo.pop() {
        if seen.contains(&file) || seen.len() >= FILES_MAX {
            continue;
        }
        seen.push(file.clone());
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        let css = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("css"));
        let refs = if css { css_refs(&text) } else { html_refs(&text) };
        for reference in refs {
            let Some(target) = local_target(&reference) else { continue };
            let found = if let Some(rel) = target.strip_prefix('/') {
                root_absolute(rel, scope).ok_or(None)
            } else {
                let at = normalize(&file.parent().unwrap_or(&file).join(target));
                if at.is_file() { Ok(at) } else { Err(Some(at)) }
            };
            match found {
                Ok(at) => {
                    let real = std::fs::canonicalize(&at).unwrap_or(at);
                    if !scope.reaches(&real) {
                        missing.push(Missing { reference, from: file.clone(), looked: Some(real), outside: true });
                    } else if real.extension().is_some_and(|e| e.eq_ignore_ascii_case("css")) {
                        todo.push(real);
                    }
                }
                Err(looked) => missing.push(Missing { reference, from: file.clone(), looked, outside: false }),
            }
        }
    }
    missing.dedup_by(|a, b| a.reference == b.reference && a.looked == b.looked);
    missing
}

/// `.` and `..` worked out, without asking the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The local file part of a reference — no query or fragment, `%XX` read —
/// or `None` for what isn't a local file (the web, `data:`, a fragment).
fn local_target(reference: &str) -> Option<String> {
    let r = reference.trim();
    let lower = r.to_ascii_lowercase();
    if r.is_empty() || r.starts_with('#') || r.starts_with("//") || r.contains("{{") || lower.contains(':') && !lower.starts_with("file:") {
        return None;
    }
    let r = r.strip_prefix("file://").unwrap_or(r);
    let r = r.split(['?', '#']).next().unwrap_or_default();
    (!r.is_empty()).then(|| percent_decoded(r))
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%' && i + 2 < bytes.len()).then(|| std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())).flatten();
        match hex {
            Some(b) => {
                out.push(b);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The files an HTML page loads: `<link href>`, `src` and `srcset` of
/// scripts, pictures and media, SVG `<use href>`, and what its inline styles
/// import or `url()`. Links a person follows (`<a href>`) are not loaded.
pub fn html_refs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut i = 0;
    while let Some(at) = lower[i..].find('<') {
        let start = i + at + 1;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        let tag = &html[start..end];
        let name: String = tag.chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        let attrs = attributes(tag);
        let attr = |key: &str| attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        match name.as_str() {
            "link" => {
                let rel = attr("rel").unwrap_or_default().to_ascii_lowercase();
                if !rel.split_whitespace().any(|r| matches!(r, "canonical" | "alternate" | "preconnect" | "dns-prefetch" | "manifest")) {
                    out.extend(attr("href"));
                }
            }
            "script" | "img" | "source" | "video" | "audio" | "iframe" | "embed" | "track" | "input" => out.extend(attr("src")),
            "use" | "image" => out.extend(attr("href").or_else(|| attr("xlink:href"))),
            "style" => {
                let close = lower[end..].find("</style").map_or(lower.len(), |c| end + c);
                out.extend(css_refs(&html[(end + 1).min(close)..close]));
            }
            _ => {}
        }
        if let Some(srcset) = attr("srcset") {
            out.extend(srcset.split(',').filter_map(|c| c.split_whitespace().next().map(str::to_owned)));
        }
        if let Some(style) = attr("style") {
            out.extend(css_refs(&style));
        }
        i = end.max(start);
    }
    out
}

/// A tag's attributes, names lowercased, quotes taken off.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let chars: Vec<char> = tag.chars().collect();
    let mut i = chars.iter().take_while(|c| !c.is_whitespace()).count();
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == '/') {
            i += 1;
        }
        let name_start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' && chars[i] != '/' {
            i += 1;
        }
        let name: String = chars[name_start..i].iter().collect::<String>().to_ascii_lowercase();
        if name.is_empty() {
            i += 1;
            continue;
        }
        if chars.get(i) != Some(&'=') {
            out.push((name, String::new()));
            continue;
        }
        i += 1;
        let value = match chars.get(i) {
            Some(&q) if q == '"' || q == '\'' => {
                let start = i + 1;
                let end = chars[start..].iter().position(|&c| c == q).map_or(chars.len(), |e| start + e);
                i = end + 1;
                chars[start..end].iter().collect()
            }
            _ => {
                let start = i;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                chars[start..i].iter().collect()
            }
        };
        out.push((name, value));
    }
    out
}

/// The files CSS loads: `@import "x"`, `@import url(x)` and every `url(x)`.
pub fn css_refs(css: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = css;
    while let Some(at) = rest.find("url(") {
        let after = &rest[at + 4..];
        let end = after.find(')').unwrap_or(after.len());
        out.push(after[..end].trim().trim_matches(['"', '\'']).to_owned());
        rest = &after[end..];
    }
    let mut rest = css;
    while let Some(at) = rest.find("@import") {
        let after = rest[at + 7..].trim_start();
        if let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') {
            let inner = &after[1..];
            out.push(inner[..inner.find(q).unwrap_or(inner.len())].to_owned());
        }
        rest = after;
    }
    out
}

// ── the page opened as the person's screen opens it ──

/// The line `pagecheck.mjs` ends with: this word, then what it saw as JSON.
pub const MARK: &str = "equipe-page-check:";

/// What the page looked like, opened and photographed (`pagecheck.mjs`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Seen {
    /// It didn't open at all, and why.
    pub error: Option<String>,
    /// Requests that failed or answered 400 and up: address and why.
    pub failed: Vec<String>,
    /// Stylesheets the page links or carries (`<link rel=stylesheet>`, `<style>`).
    pub linked: u32,
    /// CSS rules that applied, across them.
    pub rules: u32,
    /// The body's font family, as computed.
    pub font: String,
    /// Characters of text on the page.
    pub text: u32,
    /// Pictures, canvases and videos that take room.
    pub visible: u32,
    /// Pictures drawn far larger than the window: what each is.
    pub huge: Vec<String>,
}

impl Seen {
    /// What it says, read from `pagecheck.mjs`'s output.
    pub fn read(output: &str) -> Option<Seen> {
        output.lines().rev().find_map(|l| l.trim().strip_prefix(MARK)).and_then(|json| serde_json::from_str(json.trim()).ok())
    }

    /// What is wrong with the page as the person would see it, in words;
    /// empty when it looks right.
    pub fn problems(&self) -> Vec<String> {
        if let Some(error) = &self.error {
            return vec![format!("it didn't open: {error}")];
        }
        let mut out = Vec::new();
        let failed: Vec<&String> = self.failed.iter().filter(|f| !f.contains("favicon.ico")).collect();
        if !failed.is_empty() {
            out.push(format!("these didn't load: {}", failed.iter().map(|f| f.as_str()).collect::<Vec<_>>().join(", ")));
        }
        let default_font = {
            let f = self.font.trim().trim_matches('"').to_ascii_lowercase();
            f.is_empty() || f == "times" || f == "times new roman" || f == "serif"
        };
        if self.linked > 0 && self.rules == 0 {
            out.push("its styles didn't apply: no CSS rule reached the page".into());
        } else if self.linked > 0 && default_font && self.text > 0 {
            out.push(format!("its text is in the browser's default font ({}): its stylesheet likely didn't load", self.font.trim()));
        }
        if self.text == 0 && self.visible == 0 {
            out.push("it shows nothing: no text and no pictures".into());
        }
        if !self.huge.is_empty() {
            out.push(format!(
                "pictures drawn far larger than the window (an SVG with no width or height, whose CSS didn't load): {}",
                self.huge.join(", ")
            ));
        }
        out
    }
}

/// What the designer is told when the page isn't sent: each problem, and
/// the picture of what the person would have seen.
pub fn not_sent(page: &Path, problems: &[String], picture: Option<&Path>) -> String {
    let mut text = format!("Not sent: {} would reach the person's screen broken —\n", page.display());
    for p in problems {
        text.push_str(&format!("- {p}\n"));
    }
    if let Some(picture) = picture {
        text.push_str(&format!("What they would see: {} (open it to look).\n", picture.display()));
    }
    text.push_str("Fix it, then show it again. The page is served from its design folder: a file it names must be inside it (or, from the root, `/…`, in the project).");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("equipe-page-check-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::canonicalize(&d).unwrap()
    }

    #[test]
    fn a_page_reaching_up_its_design_folder_and_from_the_root_is_fine_and_whats_missing_is_named() {
        let project = dir("refs");
        let design = project.join("design");
        let session = design.join("sessions/2026-11-18-pricing");
        let sibling = design.join("sessions/2026-11-17-hero");
        for d in [&session, &sibling, &design.join("src"), &design.join("assets/logo"), &project.join("public")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(design.join("src/tokens.css"), "@import './type.css'; :root{--ink:#111} .a{background:url(../assets/logo/mark.svg)}").unwrap();
        std::fs::write(design.join("src/type.css"), "body{font-family:Inter} .b{background:url(gone.png)}").unwrap();
        std::fs::write(design.join("assets/logo/mark.svg"), "<svg/>").unwrap();
        std::fs::write(sibling.join("mascot.css"), ".m{}").unwrap();
        std::fs::write(project.join("public/og.png"), "png").unwrap();
        let page = session.join("index-a.html");
        std::fs::write(
            &page,
            r##"<!doctype html><link rel="stylesheet" href="../../src/tokens.css"><link rel=stylesheet href='../2026-11-17-hero/mascot.css'>
<link rel="preconnect" href="https://fonts.gstatic.com"><link href="https://fonts.googleapis.com/css2?family=Inter" rel="stylesheet">
<style>@import url("/src/type.css"); .hero{background:url('/public/og.png')}</style>
<img src="../../assets/logo/mark.svg" srcset="../../assets/logo/mark.svg 1x, ../../assets/logo/mark@2x.svg 2x">
<img src="data:image/png;base64,AAA"><a href="../other.html">next</a>
<script src="./app.js"></script><img src="/assets/missing.svg"><svg><use href="#icon"/></svg>"##,
        )
        .unwrap();
        let scope = Scope::of_page(&page);
        assert_eq!(scope, Scope { served: design.clone(), project: Some(project.clone()) });
        let missing: Vec<String> = missing_assets(&page, &scope).iter().map(|m| m.reference.clone()).collect();
        let mut sorted = missing.clone();
        sorted.sort();
        assert_eq!(sorted, ["../../assets/logo/mark@2x.svg", "./app.js", "/assets/missing.svg", "gone.png"], "only what isn't there: {missing:?}");
        let words = missing_assets(&page, &scope).iter().map(Missing::words).collect::<Vec<_>>().join("\n");
        assert!(words.contains("`./app.js` (in index-a.html) isn't there") && words.contains("`gone.png` (in type.css)"), "{words}");
        // From the root: the design folder first, then the project.
        assert_eq!(root_absolute("/src/tokens.css", &scope), Some(design.join("src/tokens.css")));
        assert_eq!(root_absolute("/design/src/tokens.css", &scope), Some(design.join("src/tokens.css")));
        assert_eq!(root_absolute("/public/og.png", &scope), Some(project.join("public/og.png")));
        assert_eq!(root_absolute("/../secret.css", &scope), None, "never out of the project");
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn a_file_reached_outside_the_served_folder_is_named() {
        let root = dir("outside");
        let page_dir = root.join("prototypes");
        std::fs::create_dir_all(&page_dir).unwrap();
        std::fs::write(root.join("shared.css"), "x").unwrap();
        std::fs::write(page_dir.join("a.html"), r#"<link rel="stylesheet" href="../shared.css">"#).unwrap();
        let scope = Scope::of_page(&page_dir.join("a.html"));
        assert_eq!(scope, Scope { served: page_dir.clone(), project: None }, "no design folder: its own");
        let missing = missing_assets(&page_dir.join("a.html"), &scope);
        assert_eq!(missing.len(), 1);
        assert!(missing[0].outside && missing[0].words().contains("outside the folder served"), "{:?}", missing[0]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_page_seen_broken_says_how() {
        let fine = Seen { linked: 2, rules: 40, font: "Inter, sans-serif".into(), text: 300, visible: 2, ..Seen::default() };
        assert!(fine.problems().is_empty());
        let unstyled = Seen { linked: 2, rules: 0, font: "Times".into(), text: 300, visible: 1, huge: vec!["../../assets/logo/mark.svg".into()], failed: vec!["http://127.0.0.1:9/t/k/src/tokens.css (404)".into(), "http://127.0.0.1:9/favicon.ico (404)".into()], ..Seen::default() };
        let said = unstyled.problems().join("\n");
        for part in ["didn't load: http://127.0.0.1:9/t/k/src/tokens.css (404)", "styles didn't apply", "far larger than the window", "mark.svg"] {
            assert!(said.contains(part), "{part}: {said}");
        }
        assert!(!said.contains("favicon"), "a missing icon is no problem");
        let default_font = Seen { linked: 1, rules: 3, font: "\"Times New Roman\"".into(), text: 20, visible: 0, ..Seen::default() };
        assert!(default_font.problems()[0].contains("default font"));
        assert!(Seen::default().problems()[0].contains("shows nothing"));
        assert!(Seen { error: Some("net::ERR_CONNECTION_REFUSED".into()), ..Seen::default() }.problems()[0].starts_with("it didn't open"));
        let out = format!("noise\n{MARK} {{\"failed\":[],\"linked\":1,\"rules\":5,\"font\":\"Inter\",\"text\":10,\"visible\":0,\"huge\":[]}}\n");
        assert_eq!(Seen::read(&out), Some(Seen { linked: 1, rules: 5, font: "Inter".into(), text: 10, ..Seen::default() }));
        let told = not_sent(Path::new("/w/design/sessions/p/a.html"), &unstyled.problems(), Some(Path::new("/tmp/a.png")));
        assert!(told.starts_with("Not sent:") && told.contains("What they would see: /tmp/a.png"), "{told}");
    }

    #[test]
    fn references_are_read_from_tags_and_css() {
        assert_eq!(css_refs("@import 'a.css'; @import url(\"b.css\"); .x{background:url( c.png )}"), ["b.css", "c.png", "a.css"]);
        assert_eq!(local_target("../x.css?v=2#y").as_deref(), Some("../x.css"));
        assert_eq!(local_target("b%20b.png").as_deref(), Some("b b.png"));
        for remote in ["https://x.com/a.css", "//cdn/x.js", "data:image/png;base64,A", "#icon", "mailto:a@b", "javascript:void(0)"] {
            assert_eq!(local_target(remote), None, "{remote}");
        }
    }
}
