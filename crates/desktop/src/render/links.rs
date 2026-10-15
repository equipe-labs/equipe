//! What a link in a reply points at — a ticket, a web page, a file (with a
//! line), or one of the conversation's own controls — and the paths a member
//! writes in code spans, which show as file chips. No UI types here.

use std::path::{Path, PathBuf};

use crate::convo::TICKET_SCHEME;

/// The scheme the conversation's own controls use (a long code block's
/// "show the rest" link, say).
pub const CONTROL_SCHEME: &str = "equipe:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Ticket(u32),
    /// Opened in the browser.
    Web(String),
    /// A file on the daemon's machine, and the line it is about.
    File { path: PathBuf, line: Option<u32> },
    /// `equipe:<what>`: a control drawn as a link.
    Control(String),
    /// Something else: left alone.
    Other(String),
}

/// What `url` points at. Relative paths are taken from `base`, where the
/// member works.
pub fn classify(url: &str, base: &Path) -> Link {
    let url = url.trim();
    if let Some(n) = url.strip_prefix(TICKET_SCHEME).and_then(|n| n.parse().ok()) {
        return Link::Ticket(n);
    }
    if let Some(what) = url.strip_prefix(CONTROL_SCHEME) {
        return Link::Control(what.to_owned());
    }
    let lower = url.to_ascii_lowercase();
    if ["http://", "https://", "mailto:"].iter().any(|s| lower.starts_with(s)) {
        return Link::Web(url.to_owned());
    }
    if let Some(rest) = url.strip_prefix("file://") {
        // `file:///a/b` or `file://localhost/a/b`.
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        let decoded = percent_decode(rest);
        let (path, line) = path_and_line(&decoded);
        return Link::File { path: PathBuf::from(path), line };
    }
    // Any other scheme (`vscode:`, `x-apple…`) is not ours to read as a path.
    if has_scheme(url) {
        return Link::Other(url.to_owned());
    }
    if looks_like_path(url) {
        let decoded = percent_decode(url);
        let (path, line) = path_and_line(&decoded);
        return Link::File { path: resolve(path, base), line };
    }
    Link::Other(url.to_owned())
}

fn has_scheme(url: &str) -> bool {
    match url.split_once(':') {
        Some((scheme, _)) => scheme.len() > 1 && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')),
        None => false,
    }
}

/// `~/x`, `/x`, or `x` under `base`.
pub fn resolve(path: &str, base: &Path) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return Path::new(&home).join(rest);
    }
    let path = Path::new(path);
    if path.is_absolute() { path.to_owned() } else { base.join(path.strip_prefix("./").unwrap_or(path)) }
}

/// `src/a.rs:12:5` → `src/a.rs`, line 12. `#L12` works too.
pub fn path_and_line(text: &str) -> (&str, Option<u32>) {
    if let Some((path, line)) = text.rsplit_once("#L")
        && let Ok(n) = line.split('-').next().unwrap_or_default().parse()
    {
        return (path, Some(n));
    }
    let mut parts = text.rsplitn(3, ':');
    let last = parts.next().unwrap_or_default();
    let before = parts.next();
    match (before, last.parse::<u32>()) {
        // `a.rs:12:5`
        (Some(mid), Ok(_)) if mid.parse::<u32>().is_ok() => {
            let path = &text[..text.len() - last.len() - mid.len() - 2];
            (path, mid.parse().ok())
        }
        (Some(_), Ok(n)) => (&text[..text.len() - last.len() - 1], Some(n)),
        _ => (text, None),
    }
}

/// Whether a code span's text reads as a path: no spaces, a slash or a file
/// extension, nothing that only code would have.
pub fn looks_like_path(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || text.len() > 240 || text.chars().any(char::is_whitespace) {
        return false;
    }
    if text.contains("://") || text.starts_with('-') || text.contains("::") || text.contains("..") && !text.starts_with("../") {
        return false;
    }
    if text.chars().any(|c| matches!(c, '(' | ')' | '{' | '}' | '<' | '>' | '=' | '*' | '$' | '"' | '\'' | '|' | ';' | ',' | '`' | '[' | ']' | '&' | '!' | '?' | '\\')) {
        return false;
    }
    if has_scheme(text) && !text.contains('/') {
        // `a.rs:12` has a "scheme" of `a.rs`; `std:fmt` doesn't read as a file.
        let (path, line) = path_and_line(text);
        if line.is_none() || path.contains(':') {
            return false;
        }
    }
    let (path, _) = path_and_line(text);
    let name = path.rsplit('/').next().unwrap_or(path);
    let slashed = path.contains('/') && path.split('/').filter(|p| !p.is_empty()).count() >= 2;
    let extension = name.rsplit_once('.').filter(|(stem, ext)| {
        // `1.2.3`, `e.g`, `x.` — not files.
        !stem.is_empty() && !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()) && ext.chars().any(|c| c.is_ascii_alphabetic()) && !stem.chars().all(|c| c.is_ascii_digit() || c == '.')
    });
    // Without a folder, `self.chat` reads as code: only a known kind of file counts.
    let has_extension = extension.is_some_and(|(_, ext)| slashed || path.starts_with('/') || KNOWN_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()));
    let dotfile = name.starts_with('.') && name.len() > 1 && !name[1..].contains('.');
    let absolute = path.starts_with('/') || path.starts_with("~/");
    let folder = path.len() > 1 && path.ends_with('/') && path.chars().any(char::is_alphanumeric);
    // `a/b` alone is as likely a ratio or a branch as a path; want a name.
    has_extension || dotfile || folder || (slashed && absolute)
}

/// File kinds a bare name (no folder) is recognised by.
const KNOWN_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "json", "jsonc", "toml", "yaml", "yml", "md", "mdx", "txt", "py", "go", "swift", "kt", "kts", "java", "c", "h", "cc", "cpp", "hpp", "m", "mm", "css", "scss", "html", "htm", "sh", "zsh", "bash", "sql", "lock", "png", "jpg", "jpeg",
    "gif", "svg", "webp", "pdf", "csv", "tsv", "xml", "env", "ini", "cfg", "conf", "log", "rb", "php", "vue", "svelte", "plist", "gradle", "dart", "lua", "zig", "ex", "exs", "proto", "graphql", "gql", "ipynb", "pbxproj", "storyboard", "xib", "strings", "entitlements", "dockerfile", "makefile",
];

/// `%20` and friends, as a path in a URL writes them.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(b) = text.get(i + 1..i + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "/work/site";

    fn file(path: &str, line: Option<u32>) -> Link {
        Link::File { path: PathBuf::from(path), line }
    }

    #[test]
    fn links_are_told_apart() {
        let base = Path::new(BASE);
        assert_eq!(classify("ticket:8", base), Link::Ticket(8));
        assert_eq!(classify("https://example.com/a?b", base), Link::Web("https://example.com/a?b".into()));
        assert_eq!(classify("HTTP://X", base), Link::Web("HTTP://X".into()));
        assert_eq!(classify("mailto:a@b.c", base), Link::Web("mailto:a@b.c".into()));
        assert_eq!(classify("file:///Users/a/My%20Notes.md", base), file("/Users/a/My Notes.md", None));
        assert_eq!(classify("file://localhost/tmp/x.rs", base), file("/tmp/x.rs", None));
        assert_eq!(classify("/abs/a.rs:42", base), file("/abs/a.rs", Some(42)));
        assert_eq!(classify("src/main.rs:12:5", base), file("/work/site/src/main.rs", Some(12)));
        assert_eq!(classify("./README.md", base), file("/work/site/README.md", None));
        assert_eq!(classify("src/a.rs#L30-L40", base), file("/work/site/src/a.rs", Some(30)));
        assert_eq!(classify("equipe:expand/2", base), Link::Control("expand/2".into()));
        assert_eq!(classify("vscode://file/x", base), Link::Other("vscode://file/x".into()));
        assert_eq!(classify("hello", base), Link::Other("hello".into()));
    }

    #[test]
    fn code_spans_that_read_as_paths() {
        for yes in ["src/main.rs", "crates/desktop/src/views/chat.rs:120", "Cargo.toml", "/tmp/out.txt", "~/notes/todo.md", ".env", "../a/b.ts", "a.rs:3:9", "docs/", "index.tsx"] {
            assert!(looks_like_path(yes), "{yes}");
        }
        for no in ["cargo test", "Vec::new()", "x == 1", "1.2.3", "e.g.", "a/b", "v0.7", "foo()", "--release", "std:fmt", "T-12", "$HOME/x", "a, b", "Ok(())", "https://x.y", "self.chat"] {
            assert!(!looks_like_path(no), "{no}");
        }
    }

    #[test]
    fn a_line_number_comes_off_the_path() {
        assert_eq!(path_and_line("a.rs"), ("a.rs", None));
        assert_eq!(path_and_line("a.rs:7"), ("a.rs", Some(7)));
        assert_eq!(path_and_line("a.rs:7:2"), ("a.rs", Some(7)));
        assert_eq!(path_and_line("C:x"), ("C:x", None));
    }
}
