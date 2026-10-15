//! Code in replies: the language a fence names (and the names people use for
//! it), the one highlighter every conversation shares, coloured from the
//! app's palette, and what a fence becomes before the Markdown is drawn — a
//! long one cut to its first lines, a one-line JSON laid out, a CSV a table.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

use gpui_kit::component::highlighter::{LanguageRegistry, SyntaxHighlighter};
use gpui_kit::{FontStyle, FontWeight, HighlightStyle, Hsla, rgb};

use super::links::CONTROL_SCHEME;

/// A fence longer than this shows its first [`CODE_SHOWN`] lines…
pub const CODE_CAPS_AT: usize = 30;
pub const CODE_SHOWN: usize = 24;
/// …and a CSV at most this many rows.
const TABLE_ROWS: usize = 100;

/// The grammar a fence's info string means: `shell`, `zsh`, `console` are
/// bash; `jsx` is JavaScript; `patch` a diff. `None`: no grammar for it.
pub fn canonical_lang(info: &str) -> Option<&'static str> {
    let word = first_word(info).to_ascii_lowercase();
    Some(match word.as_str() {
        "bash" | "sh" | "shell" | "zsh" | "console" | "sh-session" | "shellsession" | "terminal" | "fish" | "ksh" => "bash",
        "js" | "javascript" | "jsx" | "mjs" | "cjs" | "node" => "javascript",
        "ts" | "typescript" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "diff" | "patch" | "udiff" => "diff",
        "yaml" | "yml" => "yaml",
        "py" | "python" | "python3" | "py3" => "python",
        "rs" | "rust" => "rust",
        "go" | "golang" => "go",
        "toml" => "toml",
        "json" | "jsonc" | "json5" | "jsonl" => "json",
        "css" | "scss" | "less" => "css",
        "html" | "htm" | "xhtml" | "vue" | "svelte" => "html",
        "sql" | "psql" | "postgres" | "postgresql" | "mysql" | "sqlite" | "plsql" => "sql",
        "md" | "markdown" | "mdx" => "markdown",
        "swift" => "swift",
        _ => return None,
    })
}

/// The fence's first word, as written: `rust` of `rust,ignore`, `ts` of `{.ts}`.
pub fn first_word(info: &str) -> &str {
    info.trim().trim_start_matches(['{', '.']).split(|c: char| c.is_whitespace() || matches!(c, ',' | '{' | '}' | ':')).next().unwrap_or_default()
}

/// What a code block's header calls its language.
pub fn label(info: &str) -> Option<String> {
    let word = first_word(info);
    (!word.is_empty()).then(|| word.to_ascii_lowercase())
}

// ── highlighting ──

/// Syntax colours drawn from the app's palette, light or dark.
struct SyntaxPalette {
    comment: Hsla,
    keyword: Hsla,
    string: Hsla,
    number: Hsla,
    function: Hsla,
    kind: Hsla,
    property: Hsla,
    tag: Hsla,
    punctuation: Hsla,
    added: Hsla,
    removed: Hsla,
}

impl SyntaxPalette {
    fn new(dark: bool) -> Self {
        let c = |hex: u32| -> Hsla { rgb(hex).into() };
        if dark {
            Self {
                comment: c(0x6E7079),
                keyword: c(0xC9A8EE),
                string: c(0xA9CC8E),
                number: c(0xE6B673),
                function: c(0x9CC4E8),
                kind: c(0x7FCFC3),
                property: c(0xDDB89C),
                tag: c(0xE8999A),
                punctuation: c(0x8E9098),
                added: c(0x8CD096),
                removed: c(0xEC8A8A),
            }
        } else {
            Self {
                comment: c(0x8A8C93),
                keyword: c(0x7A3EB0),
                string: c(0x3A7D2A),
                number: c(0xA0590E),
                function: c(0x2E5C8A),
                kind: c(0x13786E),
                property: c(0x8A4B1E),
                tag: c(0xB0323A),
                punctuation: c(0x6A6C74),
                added: c(0x28803C),
                removed: c(0xB0323A),
            }
        }
    }
}

impl gpui_kit::base::input::HighlightStyleResolver for SyntaxPalette {
    fn style(&self, name: &str) -> Option<HighlightStyle> {
        let color = |c: Hsla| Some(HighlightStyle { color: Some(c), ..Default::default() });
        let top = name.split('.').next().unwrap_or(name);
        match name {
            "comment" | "comment.doc" => Some(HighlightStyle { color: Some(self.comment), font_style: Some(FontStyle::Italic), ..Default::default() }),
            "string.escape" | "string.regex" | "string.special" | "string.special.symbol" => color(self.number),
            "punctuation.special" => color(self.keyword),
            "title" | "emphasis.strong" => Some(HighlightStyle { font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() }),
            "emphasis" => Some(HighlightStyle { font_style: Some(FontStyle::Italic), ..Default::default() }),
            "variable.special" => color(self.keyword),
            _ => match top {
                "keyword" | "preproc" | "label" => color(self.keyword),
                "string" | "text" => color(self.string),
                "number" | "boolean" | "constant" => color(self.number),
                "function" | "link_uri" => color(self.function),
                "type" | "constructor" | "enum" | "variant" | "embedded" => color(self.kind),
                "property" | "attribute" => color(self.property),
                "tag" => color(self.tag),
                "punctuation" | "operator" => color(self.punctuation),
                "link_text" => color(self.function),
                _ => None,
            },
        }
    }
}

pub type Highlighter = dyn Fn(&gpui_kit::base::text::CodeBlock) -> Vec<(Range<usize>, HighlightStyle)> + Send + Sync;

/// The highlighter for the palette's appearance — built once, so a code block
/// keeps its highlights from frame to frame (they are cached per highlighter).
pub fn highlighter(dark: bool) -> Arc<Highlighter> {
    static LIGHT: OnceLock<Arc<Highlighter>> = OnceLock::new();
    static DARK: OnceLock<Arc<Highlighter>> = OnceLock::new();
    let slot = if dark { &DARK } else { &LIGHT };
    slot.get_or_init(|| {
        let palette = SyntaxPalette::new(dark);
        Arc::new(move |block: &gpui_kit::base::text::CodeBlock| {
            let Some(lang) = block.lang().and_then(|l| canonical_lang(&l)) else {
                return diff_lines(&block.code(), &palette, block.lang().is_some_and(|l| first_word(&l).is_empty()));
            };
            let code = block.code();
            if lang == "diff" {
                // Whole lines read better than the grammar's spans.
                return diff_lines(&code, &palette, true);
            }
            highlight(lang, &code, &palette)
        })
    })
    .clone()
}

fn highlight(lang: &'static str, code: &str, palette: &SyntaxPalette) -> Vec<(Range<usize>, HighlightStyle)> {
    thread_local! {
        static HIGHLIGHTERS: std::cell::RefCell<HashMap<&'static str, SyntaxHighlighter>> = std::cell::RefCell::new(HashMap::new());
    }
    if LanguageRegistry::singleton().language(lang).is_none() {
        return Vec::new();
    }
    HIGHLIGHTERS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let highlighter = cache.entry(lang).or_insert_with(|| SyntaxHighlighter::new(lang));
        let rope = ropey::Rope::from_str(code);
        highlighter.update(None, &rope, None);
        highlighter.styles(&(0..code.len()), palette)
    })
}

/// A diff's `+` and `-` lines in their colours (also for a fence with no
/// language that is plainly a diff).
fn diff_lines(code: &str, palette: &SyntaxPalette, enabled: bool) -> Vec<(Range<usize>, HighlightStyle)> {
    if !enabled || !looks_like_diff(code) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut at = 0;
    for line in code.split_inclusive('\n') {
        let end = at + line.trim_end_matches('\n').len();
        let color = if line.starts_with("+++") || line.starts_with("---") {
            Some(palette.punctuation)
        } else if line.starts_with('+') {
            Some(palette.added)
        } else if line.starts_with('-') {
            Some(palette.removed)
        } else if line.starts_with("@@") {
            Some(palette.function)
        } else {
            None
        };
        if let Some(color) = color.filter(|_| end > at) {
            out.push((at..end, HighlightStyle { color: Some(color), ..Default::default() }));
        }
        at += line.len();
    }
    out
}

fn looks_like_diff(code: &str) -> bool {
    let mut lines = code.lines().filter(|l| !l.is_empty());
    let marked = code.lines().filter(|l| l.starts_with('+') || l.starts_with('-') || l.starts_with("@@")).count();
    lines.next().is_some_and(|l| l.starts_with("diff ") || l.starts_with("--- ") || l.starts_with("@@") || l.starts_with('+') || l.starts_with('-')) && marked * 2 >= code.lines().count().max(1)
}

// ── fences before drawing ──

/// The Markdown to draw, and the whole code of each fence shown cut (keyed by
/// the code shown), for its copy button.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Fences {
    pub text: String,
    pub full: HashMap<String, String>,
}

/// The words the cut fences' controls say.
pub struct Words<'a> {
    /// "N줄 더 보기", for N hidden lines.
    pub more: &'a dyn Fn(usize) -> String,
    /// "접기".
    pub less: &'a str,
    /// "… N행 더" under a table cut short.
    pub rows: &'a dyn Fn(usize) -> String,
}

/// Rewrite `markdown`'s fences: a long one cut to its first lines with a link
/// to the rest (unless `expanded(n)`, n counting fences), a one-line JSON laid
/// out, a closed CSV or TSV made a table.
pub fn prepare(markdown: &str, expanded: &dyn Fn(usize) -> bool, words: &Words<'_>) -> Fences {
    let mut out = Fences { text: String::with_capacity(markdown.len() + 64), full: HashMap::new() };
    let lines: Vec<&str> = markdown.split('\n').collect();
    let mut i = 0;
    let mut ordinal = 0;
    while i < lines.len() {
        let Some(fence) = open_fence(lines[i]) else {
            push_line(&mut out.text, lines[i]);
            i += 1;
            continue;
        };
        let body_from = i + 1;
        let mut j = body_from;
        let mut closed = false;
        while j < lines.len() {
            if fence.closes(lines[j]) {
                closed = true;
                break;
            }
            j += 1;
        }
        let body = &lines[body_from..j];
        let close_line = closed.then(|| lines[j]);
        emit_fence(&mut out, &fence, lines[i], body, close_line, ordinal, expanded(ordinal), words);
        ordinal += 1;
        i = if closed { j + 1 } else { j };
    }
    out
}

fn push_line(text: &mut String, line: &str) {
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(line);
}

struct Fence<'a> {
    indent: &'a str,
    marker: char,
    len: usize,
    info: &'a str,
}

impl Fence<'_> {
    fn closes(&self, line: &str) -> bool {
        let trimmed = line.trim_start();
        let run = trimmed.chars().take_while(|c| *c == self.marker).count();
        run >= self.len && trimmed[run * self.marker.len_utf8()..].trim().is_empty()
    }
}

fn open_fence(line: &str) -> Option<Fence<'_>> {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    let marker = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = trimmed.chars().take_while(|c| *c == marker).count();
    if len < 3 {
        return None;
    }
    let info = trimmed[len..].trim();
    // A backtick fence's info string has no backticks (that is inline code).
    if marker == '`' && info.contains('`') {
        return None;
    }
    Some(Fence { indent, marker, len, info })
}

#[allow(clippy::too_many_arguments)]
fn emit_fence(out: &mut Fences, fence: &Fence<'_>, open: &str, body: &[&str], close: Option<&str>, ordinal: usize, expanded: bool, words: &Words<'_>) {
    let word = first_word(fence.info).to_ascii_lowercase();
    let closing = close.map(str::to_owned).unwrap_or_else(|| format!("{}{}", fence.indent, fence.marker.to_string().repeat(fence.len)));
    // A finished CSV or TSV is a table.
    if close.is_some() && matches!(word.as_str(), "csv" | "tsv") {
        let separator = if word == "tsv" { '\t' } else { ',' };
        let rows = parse_csv(&body.join("\n"), separator);
        if rows.len() >= 2 {
            push_line(&mut out.text, &csv_table(&rows, fence.indent, words));
            return;
        }
    }
    // A finished JSON on one long line, laid out.
    if close.is_some() && word == "json" && body.len() <= 2 {
        let raw = body.join("\n");
        if raw.len() > 80 && serde_json::from_str::<serde_json::Value>(&raw).is_ok() {
            push_line(&mut out.text, open);
            for line in pretty_json(&raw).lines() {
                push_line(&mut out.text, &format!("{}{line}", fence.indent));
            }
            push_line(&mut out.text, &closing);
            return;
        }
    }
    let special = matches!(word.as_str(), "mermaid" | "math" | "latex" | "tex");
    let long = body.len() > CODE_CAPS_AT && !special;
    push_line(&mut out.text, open);
    if long && !expanded {
        let shown = &body[..CODE_SHOWN];
        for line in shown {
            push_line(&mut out.text, line);
        }
        push_line(&mut out.text, &closing);
        out.full.insert(dedent(shown, fence.indent), dedent(body, fence.indent));
        push_line(&mut out.text, "");
        push_line(&mut out.text, &format!("{}[{}]({CONTROL_SCHEME}expand/{ordinal})", fence.indent, (words.more)(body.len() - CODE_SHOWN)));
        push_line(&mut out.text, "");
        return;
    }
    for line in body {
        push_line(&mut out.text, line);
    }
    if close.is_some() || long {
        push_line(&mut out.text, &closing);
    }
    if long {
        push_line(&mut out.text, "");
        push_line(&mut out.text, &format!("{}[{}]({CONTROL_SCHEME}collapse/{ordinal})", fence.indent, words.less));
        push_line(&mut out.text, "");
    }
}

/// The code as the code block holds it: its lines without the fence's indent.
fn dedent(lines: &[&str], indent: &str) -> String {
    lines.iter().map(|l| l.strip_prefix(indent).unwrap_or(l)).collect::<Vec<_>>().join("\n")
}

/// RFC 4180, forgivingly: quoted fields with `""` and line breaks inside.
pub fn parse_csv(text: &str, separator: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if field.is_empty() => quoted = true,
            c if c == separator && !quoted => row.push(std::mem::take(&mut field)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                if !(row.len() == 1 && row[0].is_empty()) {
                    rows.push(std::mem::take(&mut row));
                }
                row.clear();
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn csv_table(rows: &[Vec<String>], indent: &str, words: &Words<'_>) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let cell = |s: &str| s.replace('|', "\\|").replace(['\n', '\r'], " ").trim().to_owned();
    let line = |row: &[String]| {
        let cells: Vec<String> = (0..columns).map(|c| cell(row.get(c).map_or("", String::as_str))).collect();
        format!("{indent}| {} |", cells.join(" | "))
    };
    let mut out = vec![line(&rows[0]), format!("{indent}|{}", " --- |".repeat(columns))];
    let body = &rows[1..];
    out.extend(body.iter().take(TABLE_ROWS).map(|r| line(r)));
    if body.len() > TABLE_ROWS {
        out.push(String::new());
        out.push(format!("{indent}{}", (words.rows)(body.len() - TABLE_ROWS)));
    }
    out.join("\n")
}

/// JSON laid out two spaces deep, its keys in the order written.
pub fn pretty_json(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() * 2);
    let mut depth = 0usize;
    let (mut in_string, mut escaped) = (false, false);
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
    };
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '{' | '[' => {
                out.push(c);
                // `{}` and `[]` stay closed up.
                let close = if c == '{' { '}' } else { ']' };
                while chars.peek().is_some_and(|n| n.is_whitespace()) {
                    chars.next();
                }
                if chars.peek() == Some(&close) {
                    out.push(close);
                    chars.next();
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(c);
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prep(md: &str, expanded: bool) -> Fences {
        let more = |n: usize| format!("{n} more");
        let rows = |n: usize| format!("{n} rows more");
        prepare(md, &|_| expanded, &Words { more: &more, less: "less", rows: &rows })
    }

    #[test]
    fn fence_names_mean_their_grammar() {
        for (info, lang) in [("shell", "bash"), ("zsh", "bash"), ("console", "bash"), ("sh-session", "bash"), ("jsx", "javascript"), ("patch", "diff"), ("yml", "yaml"), ("rust,ignore", "rust"), ("{.ts}", "typescript"), ("TSX", "tsx"), ("py title=a.py", "python")] {
            assert_eq!(canonical_lang(info), Some(lang), "{info}");
        }
        assert_eq!(canonical_lang("brainfuck"), None);
        assert_eq!(canonical_lang(""), None);
        assert_eq!(label("rust,ignore"), Some("rust".into()));
        assert_eq!(label(""), None);
    }

    #[test]
    fn a_long_fence_shows_its_start_and_a_way_to_the_rest() {
        let code: Vec<String> = (1..=40).map(|n| format!("line {n}")).collect();
        let md = format!("Before\n\n```rust\n{}\n```\n\nAfter", code.join("\n"));
        let cut = prep(&md, false);
        assert!(cut.text.contains("line 24\n```\n\n[16 more](equipe:expand/0)\n") && cut.text.ends_with("\nAfter"), "{}", cut.text);
        assert!(!cut.text.contains("line 25"));
        let shown = code[..24].join("\n");
        assert_eq!(cut.full.get(&shown), Some(&code.join("\n")), "copy takes all of it");
        let whole = prep(&md, true);
        assert!(whole.text.contains("line 40\n```\n\n[less](equipe:collapse/0)"), "{}", whole.text);
        // Short fences and the text around them are left as they are.
        let short = "a\n\n```\nx\n```\nb";
        assert_eq!(prep(short, false).text, short);
    }

    #[test]
    fn a_streaming_fence_is_cut_and_closed_while_it_grows() {
        let code: Vec<String> = (1..=35).map(|n| format!("{n}")).collect();
        let md = format!("~~~~py\n{}", code.join("\n"));
        let cut = prep(&md, false);
        assert!(cut.text.starts_with("~~~~py\n1\n") && cut.text.contains("\n24\n~~~~\n"), "{}", cut.text);
        // An open short fence stays open, as Markdown reads it.
        assert_eq!(prep("```\nx", false).text, "```\nx");
    }

    #[test]
    fn csv_is_a_table_and_json_is_laid_out() {
        let md = "```csv\nname,\"note, with comma\",n\na,\"say \"\"hi\"\"\",1\nb|c,x,2\n```";
        assert_eq!(prep(md, false).text, "| name | note, with comma | n |\n| --- | --- | --- |\n| a | say \"hi\" | 1 |\n| b\\|c | x | 2 |");
        // Still streaming: code until it closes.
        assert!(prep("```csv\na,b\n1,2", false).text.starts_with("```csv"));
        let json = r#"{"name":"equipe","tags":["a","b"],"nested":{"deep":true,"empty":{}},"text":"a, b: {c}","list":[]}"#;
        let laid = prep(&format!("```json\n{json}\n```"), false).text;
        assert!(laid.starts_with("```json\n{\n  \"name\": \"equipe\",\n  \"tags\": [\n    \"a\",\n    \"b\"\n  ],"), "{laid}");
        assert!(laid.contains("\"empty\": {}") && laid.contains("\"text\": \"a, b: {c}\"") && laid.contains("\"list\": []"), "{laid}");
        assert!(serde_json::from_str::<serde_json::Value>(&pretty_json(json)).is_ok());
        // Not JSON after all: as written.
        assert_eq!(prep("```json\n{nope\n```", false).text, "```json\n{nope\n```");
    }

    #[test]
    fn diffs_colour_whole_lines() {
        let palette = SyntaxPalette::new(true);
        let spans = diff_lines("@@ -1 +1 @@\n-old\n+new\n same", &palette, true);
        assert_eq!(spans.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>(), [0..11, 12..16, 17..21]);
        assert!(diff_lines("just text", &palette, true).is_empty());
    }
}
