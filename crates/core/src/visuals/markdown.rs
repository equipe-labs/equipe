//! The pictures and folders a conversation's Markdown names: each image a
//! member embeds (`![평면도](/…/images/plan.png)`, a path relative to where
//! it works, a title after it, spaces in it as members write them), and each
//! folder it points at (`…/T-2 갤러리 디자인/images/`). The daemon reads
//! them from what it recorded, for what a screen may then ask of it; a screen
//! reads them from what it shows, for what to ask. Both read them here, so
//! both name a picture by the same path.

use std::borrow::Cow;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

/// One image the text embeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Embedded {
    /// What it is called (`![this](…)`).
    pub alt: String,
    /// Where it is, as written: escapes, `<…>` and the title taken off.
    pub path: String,
    pub title: Option<String>,
    /// The whole `![…](…)` in the text.
    pub span: Range<usize>,
    /// The destination as written, inside the parentheses.
    dest: Range<usize>,
    bracketed: bool,
}

/// How many folders one text may point at.
const FOLDERS_MAX: usize = 4;
/// How many words a path with spaces may run across in prose.
const WORDS_MAX: usize = 8;

/// The parts of `text` that are prose: fenced code left out, and with each
/// line's code spans (their ranges) for the caller to skip or read.
fn prose_lines(text: &str) -> Vec<(usize, &str, Vec<Range<usize>>)> {
    let mut out = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let start = at;
        at += line.len();
        let body = line.trim_end_matches(['\n', '\r']);
        let trimmed = body.trim_start();
        let indent = body.len() - trimmed.len();
        let run = |c: char| trimmed.chars().take_while(|x| *x == c).count();
        if indent <= 3 {
            match fence {
                Some((c, n)) if run(c) >= n && trimmed.trim_start_matches(c).trim().is_empty() => {
                    fence = None;
                    continue;
                }
                Some(_) => continue,
                None => {
                    if let Some(c) = ['`', '~'].into_iter().find(|c| run(*c) >= 3) {
                        fence = Some((c, run(c)));
                        continue;
                    }
                }
            }
        } else if fence.is_some() {
            continue;
        }
        out.push((start, body, code_spans(body)));
    }
    out
}

/// The code spans of one line: from a run of backticks to the next run as long.
fn code_spans(line: &str) -> Vec<Range<usize>> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let open = i;
        while i < bytes.len() && bytes[i] == b'`' {
            i += 1;
        }
        let n = i - open;
        let mut j = i;
        let mut closed = None;
        while j < bytes.len() {
            if bytes[j] == b'`' {
                let from = j;
                while j < bytes.len() && bytes[j] == b'`' {
                    j += 1;
                }
                if j - from == n {
                    closed = Some(j);
                    break;
                }
            } else {
                j += 1;
            }
        }
        match closed {
            Some(end) => {
                out.push(open..end);
                i = end;
            }
            None => break,
        }
    }
    out
}

/// A backslash before punctuation is only the punctuation, as Markdown reads it.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(next) = chars.peek().copied()
            && next.is_ascii_punctuation()
        {
            out.push(next);
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// The image starting at `at` (`![`) in `line`, if it is one: its alt, its
/// destination and title, and where it ends.
fn image_at(line: &str, at: usize) -> Option<(String, Range<usize>, bool, Option<String>, usize)> {
    let bytes = line.as_bytes();
    // The alt, to its `]` (brackets inside it balanced).
    let mut depth = 0;
    let mut i = at + 2;
    let close = loop {
        match bytes.get(i)? {
            b'\\' => i += 1,
            b'[' => depth += 1,
            b']' if depth == 0 => break i,
            b']' => depth -= 1,
            _ => {}
        }
        i += 1;
    };
    if bytes.get(close + 1) != Some(&b'(') {
        return None;
    }
    let alt = unescape(&line[at + 2..close]);
    let mut i = close + 2;
    while bytes.get(i) == Some(&b' ') {
        i += 1;
    }
    // `<a path with spaces>`, as CommonMark has it.
    if bytes.get(i) == Some(&b'<') {
        let end = i + 1 + line[i + 1..].find('>')?;
        let dest = i + 1..end;
        let rest = &line[end + 1..];
        let close = rest.find(')')?;
        let title = title_of(rest[..close].trim());
        return Some((alt, dest, true, title, end + 1 + close + 1));
    }
    // Else to the `)` that closes it (parentheses inside balanced) — spaces
    // and all, as members write a folder's name; a title last, in quotes.
    let mut depth = 0;
    let mut j = i;
    let end = loop {
        match bytes.get(j)? {
            b'\\' => j += 1,
            b'(' => depth += 1,
            b')' if depth == 0 => break j,
            b')' => depth -= 1,
            _ => {}
        }
        j += 1;
    };
    let inner = &line[i..end];
    let (dest, title) = match split_title(inner) {
        Some((dest, title)) => (i..i + dest.len(), Some(title)),
        None => (i..i + inner.trim_end().len(), None),
    };
    (!dest.is_empty()).then_some((alt, dest, false, title, end + 1))
}

/// `"a title"` (or `'…'`, `(…)`) on its own, without its quotes.
fn title_of(text: &str) -> Option<String> {
    let first = text.chars().next()?;
    let last = text.chars().last()?;
    let paired = matches!((first, last), ('"', '"') | ('\'', '\'') | ('(', ')'));
    (paired && text.len() >= 2).then(|| unescape(&text[1..text.len() - 1]))
}

/// A destination with a title after it: the destination, and the title.
fn split_title(inner: &str) -> Option<(&str, String)> {
    let inner = inner.trim_end();
    let quote = inner.chars().last().filter(|c| matches!(c, '"' | '\''))?;
    let open = inner[..inner.len() - 1].rfind(quote)?;
    let dest = &inner[..open];
    if !dest.ends_with([' ', '\t']) || dest.trim().is_empty() {
        return None;
    }
    Some((dest.trim_end(), unescape(&inner[open + 1..inner.len() - 1])))
}

/// Every image `text` embeds, in order — outside its code.
pub fn images(text: &str) -> Vec<Embedded> {
    let mut out = Vec::new();
    for (start, line, code) in prose_lines(text) {
        let mut from = 0;
        while let Some(found) = line[from..].find("![") {
            let at = from + found;
            if let Some(span) = code.iter().find(|c| c.contains(&at)) {
                from = span.end;
                continue;
            }
            match image_at(line, at) {
                Some((alt, dest, bracketed, title, end)) => {
                    let path = unescape(line[dest.clone()].trim());
                    out.push(Embedded { alt, path, title, span: start + at..start + end, dest: start + dest.start..start + dest.end, bracketed });
                    from = end;
                }
                None => from = at + 2,
            }
        }
    }
    out
}

/// Whether `url` is a page's or inline data, not a file.
pub fn is_web(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["http://", "https://", "data:"].iter().any(|s| lower.starts_with(s))
}

/// The images of `text` that are files — on the machine the member works on.
pub fn local_images(text: &str) -> Vec<Embedded> {
    images(text).into_iter().filter(|i| !is_web(&i.path)).collect()
}

/// `text` with each image's destination that holds a space put in `<…>`, so
/// a Markdown reader takes it as one: members write `![a](/T-2 갤러리/a.png)`.
pub fn bracketed(text: &str) -> Cow<'_, str> {
    let loose: Vec<Embedded> = images(text).into_iter().filter(|i| !i.bracketed && text[i.dest.clone()].contains(char::is_whitespace)).collect();
    if loose.is_empty() {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + loose.len() * 2);
    let mut at = 0;
    for image in loose {
        out.push_str(&text[at..image.dest.start]);
        out.push('<');
        out.push_str(&text[image.dest.clone()]);
        out.push('>');
        at = image.dest.end;
    }
    out.push_str(&text[at..]);
    Cow::Owned(out)
}

/// `%20` and the like, as bytes; what doesn't decode stays as written.
fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = text.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

/// Where a path a member wrote is — `file://`, `%20`s and `./` read, a
/// relative one taken from `cwd` — as one string the daemon and every
/// screen agree on. `None` for one that climbs (`..`), or isn't a path.
pub fn lexical(written: &str, cwd: &Path) -> Option<PathBuf> {
    let written = written.trim().trim_start_matches('<').trim_end_matches('>');
    let written = match written.strip_prefix("file://") {
        Some(rest) => rest.strip_prefix("localhost").unwrap_or(rest),
        None => written,
    };
    if written.is_empty() || is_web(written) || written.starts_with('~') {
        return None;
    }
    let decoded = percent_decode(written);
    let path = Path::new(&decoded);
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    let path = if path.is_absolute() { path.to_owned() } else { cwd.join(path) };
    path.is_absolute().then(|| path.components().filter(|c| !matches!(c, Component::CurDir)).collect())
}

/// Whether a path's last part reads as a file's name (`a.png`), not a folder's.
fn has_extension(path: &str) -> bool {
    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// What may end a path in prose and isn't part of it.
const TRAILING: &[char] = &['.', ',', ';', ':', '!', '?', ')', ']', '"', '\'', '」', '』', '。', '、', '）', '*', '_'];

/// The absolute path starting at `at` in `line` (prose) that ends in `/`:
/// over a few words, since a folder's name may hold spaces, never into the
/// next path. `None` when no stretch of it ends in a slash.
fn folder_at(line: &str, at: usize) -> Option<String> {
    let mut best = None;
    let mut end = at;
    for (n, word) in line[at..].split(' ').enumerate() {
        if n >= WORDS_MAX || (n > 0 && (word.starts_with('/') || word.contains("://") || word.contains('`') || word.starts_with("!["))) {
            break;
        }
        end += word.len();
        let candidate = line[at..end].trim_end_matches(TRAILING);
        if candidate.ends_with('/') && candidate.matches('/').count() >= 3 {
            best = Some(candidate.to_owned());
        }
        end += 1;
        if end > line.len() {
            break;
        }
    }
    best
}

/// Every folder `text` points at, in order, at most a few: an absolute path
/// ending in `/` in prose or a link, or an absolute path in a code span that
/// has no file's extension — outside its code blocks and its images.
pub fn folders(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let add = |path: &str, out: &mut Vec<String>| {
        let path = path.trim();
        let path = path.strip_prefix("file://").unwrap_or(path);
        if path.starts_with('/') && path.len() > 1 && !out.iter().any(|p| p.trim_end_matches('/') == path.trim_end_matches('/')) && out.len() < FOLDERS_MAX {
            out.push(path.to_owned());
        }
    };
    let embedded = images(text);
    for (start, line, code) in prose_lines(text) {
        let in_image = |at: usize| embedded.iter().any(|i| i.span.contains(&(start + at)));
        for span in &code {
            let inner = line[span.clone()].trim_matches('`').trim();
            if inner.starts_with('/') && !inner.contains('\n') && !has_extension(inner) && !inner.contains("://") {
                add(inner, &mut out);
            }
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'/' || code.iter().any(|c| c.contains(&i)) || in_image(i) {
                i += 1;
                continue;
            }
            let before = line[..i].chars().next_back();
            let starts = before.is_none_or(|c| c.is_whitespace() || matches!(c, '(' | ':' | '"' | '\'' | '「' | '『' | '[' | '*' | '<' | '（'));
            if !starts || bytes.get(i + 1).is_none_or(|b| *b == b'/' || *b == b' ') {
                i += 1;
                continue;
            }
            match folder_at(line, i) {
                Some(found) => {
                    i += found.len();
                    add(&found, &mut out);
                }
                None => i += 1,
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESIGN: &str = "/Users/abel/.equipe/worktrees/photo-studio.home/t2-6ad338ba/outputs/T-2 갤러리 디자인/images";

    fn paths(text: &str) -> Vec<String> {
        images(text).into_iter().map(|i| i.path).collect()
    }

    #[test]
    fn an_image_is_read_absolute_or_relative_with_its_alt_and_title() {
        let text = "디자이너 시안입니다.\n\n![평면도](/w/images/plan.png)\n![배치도](images/site.png \"배치도 A안\")\n![](./a.png)";
        let found = images(text);
        assert_eq!(found.iter().map(|i| (i.alt.as_str(), i.path.as_str(), i.title.as_deref())).collect::<Vec<_>>(), [
            ("평면도", "/w/images/plan.png", None),
            ("배치도", "images/site.png", Some("배치도 A안")),
            ("", "./a.png", None),
        ]);
        assert_eq!(&text[found[0].span.clone()], "![평면도](/w/images/plan.png)");
    }

    #[test]
    fn a_path_with_spaces_is_one_path_bracketed_or_not() {
        let plain = format!("![단면·입면]({DESIGN}/section.png)");
        assert_eq!(paths(&plain), [format!("{DESIGN}/section.png")]);
        let titled = format!("![단면·입면]({DESIGN}/section.png 'B안')");
        assert_eq!(images(&titled)[0].title.as_deref(), Some("B안"));
        assert_eq!(images(&titled)[0].path, format!("{DESIGN}/section.png"));
        let bracketed_text = format!("![평면도](<{DESIGN}/plan.png> \"A\")");
        assert_eq!(paths(&bracketed_text), [format!("{DESIGN}/plan.png")]);
        // Put in brackets for a Markdown reader, once; others left as they are.
        let fixed = bracketed(&plain);
        assert_eq!(fixed, format!("![단면·입면](<{DESIGN}/section.png>)"));
        assert_eq!(bracketed(&fixed), fixed, "already bracketed");
        assert!(matches!(bracketed("![a](/w/a.png) and text"), Cow::Borrowed(_)));
        assert_eq!(bracketed(&titled), format!("![단면·입면](<{DESIGN}/section.png> 'B안')"));
    }

    #[test]
    fn code_and_links_and_odd_brackets_are_not_images() {
        assert!(paths("`![a](/w/a.png)` is how").is_empty(), "in a code span");
        assert!(paths("```md\n![a](/w/a.png)\n```\n").is_empty(), "in a fence");
        assert_eq!(paths("```\nx\n```\n![b](/w/b.png)"), ["/w/b.png"], "after the fence closes");
        assert!(paths("[a link](/w/a.png) ! [not](/w/b.png) ![open](").is_empty());
        assert_eq!(paths(r"![a \[1\]](/w/a\_b.png)"), ["/w/a_b.png"], "escapes read");
        assert_eq!(images(r"![a \[1\]](/w/a.png)")[0].alt, "a [1]");
        assert_eq!(paths("![x](/w/plan (1).png)"), ["/w/plan (1).png"], "parentheses inside");
        assert_eq!(paths("![web](https://x.io/a.png) ![f](/w/a.png)").len(), 2);
        assert_eq!(local_images("![web](https://x.io/a.png) ![d](data:image/png;base64,AA) ![f](/w/a.png)").len(), 1);
    }

    #[test]
    fn a_path_is_where_the_member_works_unless_it_is_absolute_and_never_climbs() {
        let cwd = Path::new("/h/worktrees/site/t2");
        assert_eq!(lexical("images/a.png", cwd).unwrap(), Path::new("/h/worktrees/site/t2/images/a.png"));
        assert_eq!(lexical("./images/./a.png", cwd).unwrap(), Path::new("/h/worktrees/site/t2/images/a.png"));
        assert_eq!(lexical("/w/T-2%20갤러리/a.png", cwd).unwrap(), Path::new("/w/T-2 갤러리/a.png"));
        assert_eq!(lexical("file:///w/a.png", cwd).unwrap(), Path::new("/w/a.png"));
        assert_eq!(lexical("<images/a b.png>", cwd).unwrap(), Path::new("/h/worktrees/site/t2/images/a b.png"));
        assert_eq!(lexical(&format!("{DESIGN}/"), cwd).unwrap().to_string_lossy(), DESIGN, "a folder without its last slash");
        for refused in ["../secret.png", "/w/../etc/a.png", "https://x.io/a.png", "~/a.png", ""] {
            assert!(lexical(refused, cwd).is_none(), "{refused}");
        }
    }

    #[test]
    fn a_folder_is_named_in_prose_with_spaces_or_in_code() {
        let reply = format!("이미지가 화면에 안 보이면 이 폴더를 열어 보세요: {DESIGN}/");
        assert_eq!(folders(&reply), [format!("{DESIGN}/")]);
        assert_eq!(folders(&format!("폴더: {DESIGN}/ 를 보세요.")), [format!("{DESIGN}/")]);
        assert_eq!(folders(&format!("({DESIGN}/).")), [format!("{DESIGN}/")]);
        assert_eq!(folders(&format!("`{DESIGN}`")), [DESIGN.to_owned()], "in code, no slash needed");
        assert_eq!(folders(&format!("[폴더]({DESIGN}/)")), [format!("{DESIGN}/")]);
        // Two in a line stay two.
        assert_eq!(folders("/w/a/b/ 그리고 /w/c/d/"), ["/w/a/b/", "/w/c/d/"]);
        // Not folders: files, URLs, short or relative paths, images, code blocks.
        for text in ["/w/a/b.png", "https://x.io/a/b/", "/a/", "images/", "`/w/a/b.md`", "![a](/w/a/b/)", "```\n/w/a/b/\n```"] {
            assert!(folders(text).is_empty(), "{text}: {:?}", folders(text));
        }
        // The same folder once, and a few at most.
        assert_eq!(folders("/w/a/b/ and `/w/a/b`").len(), 1);
        let many: String = (0..9).map(|n| format!("/w/f/{n}/\n")).collect();
        assert_eq!(folders(&many).len(), FOLDERS_MAX);
    }
}
