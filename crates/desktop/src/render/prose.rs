//! Two fix-ups to a reply's Markdown before the text view parses it, both
//! made on the parse (the same parser and options as the text view's), so
//! code, links and URLs are never touched:
//!
//! - A ticket number in text becomes a link to the ticket. Done on the raw
//!   string it went into URLs and indented code, and broke the emphasis
//!   around it.
//! - Emphasis written against Korean, Chinese or Japanese closes as written.
//!   CommonMark won't close `**` that follows punctuation and is followed by
//!   a letter, so `**T-114(…)**를` showed its asterisks; the same holds for
//!   `은**(…)` opening. Where the asterisks (or tildes) stayed literal for that
//!   reason alone, the pair is written as `<strong>`/`<em>`/`<del>`, which the
//!   text view draws the same. Elsewhere — English, code — CommonMark's rule
//!   stands.
//!
//! No UI types here.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use markdown::mdast::Node;
use markdown::{ParseOptions, to_mdast};

use super::plugins::looks_like_math;
use crate::convo::{TICKET_SCHEME, ticket_ref};

/// The Markdown as the text view should get it. The same text comes in each
/// frame a reply is in sight, so what was worked out is kept a while.
pub fn fix(markdown: &str) -> String {
    thread_local! {
        static DONE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    }
    if ticket_ref(markdown).is_none() && !may_be_stuck(markdown) {
        return markdown.to_owned();
    }
    if let Some(done) = DONE.with_borrow(|done| done.get(markdown).cloned()) {
        return done;
    }
    let fixed = close_cjk_emphasis(&link_tickets(markdown));
    DONE.with_borrow_mut(|done| {
        if done.len() >= 128 {
            done.clear();
        }
        done.insert(markdown.to_owned(), fixed.clone());
    });
    fixed
}

/// The text view's parse options: GFM, with math.
fn options() -> ParseOptions {
    let mut options = ParseOptions::gfm();
    options.constructs.math_text = true;
    options.constructs.math_flow = true;
    options
}

/// A replacement: this range of the source becomes this text.
type Edit = (Range<usize>, String);

fn span(node: &Node) -> Option<Range<usize>> {
    node.position().map(|p| p.start.offset..p.end.offset)
}

/// `source` with `edits` (not overlapping) made.
fn splice(source: &str, edits: &mut [Edit]) -> String {
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = String::with_capacity(source.len() + edits.len() * 16);
    let mut at = 0;
    for (range, with) in edits.iter() {
        if range.start < at {
            continue;
        }
        out.push_str(&source[at..range.start]);
        out.push_str(with);
        at = range.end;
    }
    out.push_str(&source[at..]);
    out
}

/// What holds no prose: code, math, raw HTML, an image, a definition.
fn not_prose(node: &Node) -> bool {
    matches!(
        node,
        Node::Code(_) | Node::InlineCode(_) | Node::Html(_) | Node::Math(_) | Node::Image(_) | Node::ImageReference(_) | Node::Definition(_) | Node::Yaml(_) | Node::Toml(_) | Node::FootnoteReference(_)
    )
}

// ── ticket numbers ──

/// Markdown with every ticket number in text made a link: `T-8` becomes
/// `[T-8](ticket:8)`, which a click turns into the ticket on the board.
/// Emphasis around just the number goes inside the link (`[**T-8**](…)`).
pub fn link_tickets(markdown: &str) -> String {
    if ticket_ref(markdown).is_none() {
        return markdown.to_owned();
    }
    let mut edits = Vec::new();
    tickets_in(markdown, 0, &options(), &mut edits);
    splice(markdown, &mut edits)
}

/// Edits (from `base` on) linking the tickets in `source`.
fn tickets_in(source: &str, base: usize, options: &ParseOptions, edits: &mut Vec<Edit>) {
    if let Ok(root) = to_mdast(source, options) {
        walk_tickets(&root, source, base, edits);
    }
}

fn walk_tickets(node: &Node, source: &str, base: usize, edits: &mut Vec<Edit>) {
    let target = |id: &str| format!("({TICKET_SCHEME}{})", &id[2..]);
    match node {
        // Already a link, or an address (`https://x.y/T-8`).
        Node::Link(_) | Node::LinkReference(_) => {}
        Node::InlineMath(math) => {
            // Two dollars around prose ("$5 for T-8 and $10"): the text view
            // reads it as text, and so do we.
            let Some(range) = span(node) else { return };
            if !looks_like_math(&math.value, source[range.end..].chars().next()) {
                let mut prose = options();
                prose.constructs.math_text = false;
                prose.constructs.math_flow = false;
                tickets_in(&source[range.clone()], base + range.start, &prose, edits);
            }
        }
        _ if not_prose(node) => {}
        Node::Strong(_) | Node::Emphasis(_) | Node::Delete(_)
            if let Some(range) = span(node)
                && let [Node::Text(text)] = node.children().map(Vec::as_slice).unwrap_or_default()
                && ticket_ref(&text.value) == Some((0, text.value.len())) =>
        {
            edits.push((base + range.start..base + range.end, format!("[{}]{}", &source[range], target(&text.value))));
        }
        Node::Text(_) => {
            let Some(range) = span(node) else { return };
            let mut from = range.start;
            while let Some((at, len)) = ticket_ref(&source[from..range.end]) {
                let at = from + at;
                // A word that runs on from the node before.
                let joined = at == range.start && source[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '-');
                if !joined {
                    let id = &source[at..at + len];
                    edits.push((base + at..base + at + len, format!("[{id}]{}", target(id))));
                }
                from = at + len;
            }
        }
        _ => {
            for child in node.children().into_iter().flatten() {
                walk_tickets(child, source, base, edits);
            }
        }
    }
}

// ── emphasis against CJK ──

/// What goes in, for a trial parse, between a delimiter and punctuation.
const MARK: char = '\u{200B}';
const MARK_LEN: usize = MARK.len_utf8();

/// Emphasis, strikethrough and bold that CommonMark leaves open next to
/// Korean, Chinese or Japanese, closed. A closing `)**를` (or opening
/// `은**(`) is retried with a zero-width space between the delimiter and the
/// punctuation, which lets it close (open); a pair that forms only so is
/// written as HTML tags, and the spaces go again. Repeated, for emphasis
/// inside emphasis.
pub fn close_cjk_emphasis(markdown: &str) -> String {
    let mut text = markdown.to_owned();
    for _ in 0..4 {
        match close_once(&text) {
            Some(next) => text = next,
            None => break,
        }
    }
    text
}

fn close_once(text: &str) -> Option<String> {
    if !may_be_stuck(text) {
        return None;
    }
    let root = to_mdast(text, &options()).ok()?;
    let mut marks = Vec::new();
    stuck_in(&root, text, &mut marks);
    if marks.is_empty() {
        return None;
    }
    marks.sort_unstable();
    marks.dedup();
    let mut marked = String::with_capacity(text.len() + marks.len() * MARK_LEN);
    let mut at = 0;
    for &mark in &marks {
        marked.push_str(&text[at..mark]);
        marked.push(MARK);
        at = mark;
    }
    marked.push_str(&text[at..]);
    // Where each mark sits in `marked`.
    let placed: Vec<usize> = marks.iter().enumerate().map(|(n, at)| at + n * MARK_LEN).collect();
    let root = to_mdast(&marked, &options()).ok()?;
    let mut edits = Vec::new();
    closed_by_marks(&root, &marked, &placed, &mut edits);
    if edits.is_empty() {
        return None;
    }
    let unmarked = |at: usize| at - MARK_LEN * placed.iter().filter(|p| **p < at).count();
    let mut edits: Vec<Edit> = edits.into_iter().map(|(range, tag)| (unmarked(range.start)..unmarked(range.end), tag)).collect();
    Some(splice(text, &mut edits))
}

/// Whether a run of `*` or `~` anywhere in `text` sits between punctuation
/// and a CJK letter: a cheap look before parsing.
fn may_be_stuck(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if matches!(bytes[i], b'*' | b'~') {
            let run = run_at(text, i);
            if stuck(text, &run).is_some() {
                return true;
            }
            i = run.end;
        } else {
            i += 1;
        }
    }
    false
}

/// The whole run of the delimiter at `at`.
fn run_at(text: &str, at: usize) -> Range<usize> {
    let bytes = text.as_bytes();
    let c = bytes[at];
    let start = at - bytes[..at].iter().rev().take_while(|b| **b == c).count();
    let end = at + bytes[at..].iter().take_while(|b| **b == c).count();
    start..end
}

/// Where a mark would let this run open or close: between it and the
/// punctuation on one side, when a CJK letter is on the other.
fn stuck(text: &str, run: &Range<usize>) -> Option<usize> {
    let before = text[..run.start].chars().next_back();
    let after = text[run.end..].chars().next();
    let punctuation = |c: Option<char>| c.is_some_and(|c| !c.is_whitespace() && !c.is_alphanumeric() && c != MARK);
    let cjk = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() && is_cjk(c));
    if punctuation(before) && cjk(after) {
        Some(run.start)
    } else if cjk(before) && punctuation(after) {
        Some(run.end)
    } else {
        None
    }
}

/// Hangul, kana, Han, and their full- and half-width forms.
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF | 0x3040..=0x30FF | 0x3130..=0x318F | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0xFF10..=0xFF5A | 0xFF66..=0xFFDC | 0x20000..=0x3FFFF)
}

/// Where marks go: beside each stuck run of `*` or `~` the parse left as text.
fn stuck_in(node: &Node, text: &str, marks: &mut Vec<usize>) {
    match node {
        _ if not_prose(node) || matches!(node, Node::InlineMath(_)) => {}
        // An address (`https://…`, `<https://…>`) is no place for emphasis.
        Node::Link(_) if span(node).is_some_and(|r| !text[r.start..].starts_with('[')) => {}
        Node::Text(_) => {
            let Some(range) = span(node) else { return };
            let bytes = text.as_bytes();
            let mut i = range.start;
            while i < range.end {
                if !matches!(bytes[i], b'*' | b'~') {
                    i += 1;
                    continue;
                }
                let run = run_at(text, i);
                let escaped = bytes[..run.start].iter().rev().take_while(|b| **b == b'\\').count() % 2 == 1;
                if !escaped && let Some(mark) = stuck(text, &run) {
                    marks.push(mark);
                }
                i = run.end;
            }
        }
        _ => {
            for child in node.children().into_iter().flatten() {
                stuck_in(child, text, marks);
            }
        }
    }
}

/// The delimiters, as HTML tags, of each pair that a mark let form.
fn closed_by_marks(node: &Node, marked: &str, placed: &[usize], edits: &mut Vec<Edit>) {
    let tag = match node {
        Node::Strong(_) => Some(("strong", 2)),
        Node::Emphasis(_) => Some(("em", 1)),
        Node::Delete(_) => span(node).map(|r| ("del", marked[r].bytes().take_while(|b| *b == b'~').count())),
        _ => None,
    };
    if let (Some((name, n)), Some(r)) = (tag, span(node)) {
        let marked_at = |at: Option<usize>| at.is_some_and(|at| placed.contains(&at));
        let (opener, closer) = (r.start..r.start + n, r.end - n..r.end);
        let touched = marked_at(opener.start.checked_sub(MARK_LEN)) || marked_at(Some(opener.end)) || marked_at(closer.start.checked_sub(MARK_LEN)) || marked_at(Some(closer.end));
        if touched {
            edits.push((opener, format!("<{name}>")));
            edits.push((closer, format!("</{name}>")));
        }
    }
    if !not_prose(node) {
        for child in node.children().into_iter().flatten() {
            closed_by_marks(child, marked, placed, edits);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use markdown::mdast::Node;

    fn options() -> markdown::ParseOptions {
        let mut options = markdown::ParseOptions::gfm();
        options.constructs.math_text = true;
        options.constructs.math_flow = true;
        options
    }

    /// What the text view shows for `markdown`, marks spelled out: `<b>`,
    /// `<i>`, `<s>`, links as `[text](url)`, code in backticks.
    fn shown(markdown: &str) -> String {
        let fixed = fix(markdown);
        let root = markdown::to_mdast(&fixed, &options()).unwrap();
        let mut out = String::new();
        flatten(&root, &mut out);
        out
    }

    fn flatten(node: &Node, out: &mut String) {
        let wrap = |open: &str, close: &str, out: &mut String| {
            out.push_str(open);
            for child in node.children().into_iter().flatten() {
                flatten(child, out);
            }
            out.push_str(close);
        };
        match node {
            Node::Text(t) => out.push_str(&t.value),
            Node::InlineCode(c) => out.push_str(&format!("`{}`", c.value)),
            Node::Code(c) => out.push_str(&format!("```\n{}\n```", c.value)),
            Node::Html(h) => out.push_str(match h.value.as_str() {
                "<strong>" => "<b>",
                "</strong>" => "</b>",
                "<em>" => "<i>",
                "</em>" => "</i>",
                "<del>" => "<s>",
                "</del>" => "</s>",
                other => other,
            }),
            Node::Strong(_) => wrap("<b>", "</b>", out),
            Node::Emphasis(_) => wrap("<i>", "</i>", out),
            Node::Delete(_) => wrap("<s>", "</s>", out),
            Node::Link(l) => {
                let url = l.url.clone();
                wrap("[", &format!("]({url})"), out);
            }
            Node::Paragraph(_) if !out.is_empty() => wrap("\n", "", out),
            _ => wrap("", "", out),
        }
    }

    #[test]
    fn bold_around_a_ticket_closes_before_a_korean_particle() {
        assert_eq!(
            shown("참고로 backend가 **T-114(이 기기 지우기, 릴레이·데몬 쪽)**를 검토에 올렸습니다."),
            "참고로 backend가 <b>[T-114](ticket:114)(이 기기 지우기, 릴레이·데몬 쪽)</b>를 검토에 올렸습니다."
        );
        assert_eq!(shown("**T-9**부터"), "[<b>T-9</b>](ticket:9)부터");
        assert_eq!(shown("*T-1*이"), "[<i>T-1</i>](ticket:1)이");
        assert_eq!(shown("~~T-2~~는"), "[<s>T-2</s>](ticket:2)는");
        assert_eq!(shown("이건 **T-3**와 **T-4(로그인)**을 묶습니다"), "이건 [<b>T-3</b>](ticket:3)와 <b>[T-4](ticket:4)(로그인)</b>을 묶습니다");
    }

    #[test]
    fn emphasis_next_to_korean_reads_as_written() {
        assert_eq!(shown("**한국어**입니다"), "<b>한국어</b>입니다");
        assert_eq!(shown("**[링크](https://x.y)**은 열립니다"), "<b>[링크](https://x.y)</b>은 열립니다");
        assert_eq!(shown("이것은**(중요)**입니다"), "이것은<b>(중요)</b>입니다");
        assert_eq!(shown("**`cargo test`**를 돌렸고"), "<b>`cargo test`</b>를 돌렸고");
        assert_eq!(shown("**「인용」**이다"), "<b>「인용」</b>이다");
        assert_eq!(shown("*기울임(x)*과 ~~지움(y)~~도"), "<i>기울임(x)</i>과 <s>지움(y)</s>도");
        assert_eq!(shown("日本語の**「強調」**です"), "日本語の<b>「強調」</b>です");
        let nested = shown("***중요(x)***를");
        assert!(!nested.contains('*') && nested.ends_with("를"), "{nested}");
        assert_eq!(shown("**굵게 *기울임(x)***를"), "<b>굵게 <i>기울임(x)</i></b>를");
        assert_eq!(shown("- **설정(Settings)**에서\n- 둘째"), "<b>설정(Settings)</b>에서\n둘째");
        assert_eq!(shown("## **T-7(앱)**의 범위"), "<b>[T-7](ticket:7)(앱)</b>의 범위");
        assert_eq!(shown("| **값(%)**은 |\n|---|"), "<b>값(%)</b>은");
    }

    #[test]
    fn english_and_code_are_left_alone() {
        assert_eq!(shown("**bold** text and *it* too"), "<b>bold</b> text and <i>it</i> too");
        assert_eq!(shown("see **T-3** now"), "see [<b>T-3</b>](ticket:3) now");
        // CommonMark's own rule, kept where no Korean, Chinese or Japanese is next to it.
        assert_eq!(shown("**foo(bar)**s"), "**foo(bar)**s");
        assert_eq!(shown("2 ** 3 and a*(b)*c"), "2 ** 3 and a*(b)*c");
        assert_eq!(shown("run `2 ** (3)**가` now"), "run `2 ** (3)**가` now");
        assert_eq!(shown("```\n**T-1(x)**를\n```"), "```\n**T-1(x)**를\n```");
        assert_eq!(shown("see https://x.y/T-8 and <https://x.y/T-9>"), "see [https://x.y/T-8](https://x.y/T-8) and [https://x.y/T-9](https://x.y/T-9)");
        assert_eq!(shown("[T-8](ticket:8) and [docs](https://x.y/T-8)"), "[T-8](ticket:8) and [docs](https://x.y/T-8)");
        assert_eq!(shown(r"escaped \*\*(x)\*\*가"), "escaped **(x)**가");
    }

    #[test]
    fn the_fixed_markdown_is_clean_and_stable() {
        for text in ["참고로 **T-114(이 기기 지우기)**를 검토", "**[link](url)**은", "**한국어**입니다", "a **b** c", "이것은**(중요)**입니다"] {
            let once = fix(text);
            assert!(!once.contains('\u{200B}'), "no zero-width marks left in {once:?}");
            assert_eq!(fix(&once), once, "fixing twice changes nothing: {text}");
        }
        assert_eq!(fix("nothing to do here"), "nothing to do here");
    }

    #[test]
    fn ticket_numbers_become_links_outside_code() {
        assert_eq!(fix("see T-8 and T-12."), "see [T-8](ticket:8) and [T-12](ticket:12).");
        assert_eq!(fix("**T-9**부터"), "[**T-9**](ticket:9)부터", "bold goes inside the link");
        assert_eq!(fix("run `T-8` now"), "run `T-8` now", "inline code stays");
        assert_eq!(fix("```\nT-8\n```\nT-9"), "```\nT-8\n```\n[T-9](ticket:9)", "fenced code stays");
        assert_eq!(fix("    T-8 indented code\n\nT-9"), "    T-8 indented code\n\n[T-9](ticket:9)", "indented code stays");
        assert_eq!(fix("[T-8](ticket:8)"), "[T-8](ticket:8)", "already a link");
        assert_eq!(fix("MAT-8 and T-"), "MAT-8 and T-", "only whole words");
        assert_eq!(fix("| a | T-5 |\n|---|---|\n| T-6 | b |"), "| a | [T-5](ticket:5) |\n|---|---|\n| [T-6](ticket:6) | b |");
    }
}
