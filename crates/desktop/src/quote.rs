//! Asking about a stretch of the conversation (`views/quote.rs`): the words
//! the person selected, trimmed into a quote; the message that carries it — a
//! Markdown blockquote, then the question — and that message read back for
//! its bubble; where quoted words are in a text, for the way back to them; and
//! where the button and the box float, by the selection's ends.

use std::ops::Range;

/// The most of a selection a quote keeps; past it, an ellipsis.
pub const MAX_QUOTE: usize = 4000;

/// The selected words as a quote: line ends made `\n`, each line's trailing
/// space and a text view's non-breaking indentation undone, blank lines at
/// either end dropped and runs of them made one. `None`: nothing but space.
pub fn trim(selected: &str) -> Option<String> {
    let text = selected.replace("\r\n", "\n").replace('\r', "\n").replace('\u{A0}', " ");
    let mut lines: Vec<&str> = Vec::new();
    for line in text.split('\n').map(str::trim_end) {
        if line.trim().is_empty() && lines.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        lines.push(if line.trim().is_empty() { "" } else { line });
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    // A selection begun mid-line starts at a space, not at indentation; under
    // it the common indentation goes, a list's own indentation under that stays.
    if let Some(first) = lines.first_mut() {
        *first = first.trim_start();
    }
    let indent = lines.iter().skip(1).filter(|l| !l.is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let mut out = lines.iter().enumerate().map(|(n, l)| if l.is_empty() || n == 0 { *l } else { &l[indent..] }).collect::<Vec<_>>().join("\n");
    if out.is_empty() {
        return None;
    }
    if out.chars().count() > MAX_QUOTE {
        out = out.chars().take(MAX_QUOTE).collect::<String>().trim_end().to_owned();
        out.push('…');
    }
    Some(out)
}

/// The message: the quote as a Markdown blockquote (`> ` on every line, a
/// bare `>` on a blank one), a blank line, then the question.
pub fn message(quote: &str, question: &str) -> String {
    let quoted = quote.lines().map(|l| if l.trim().is_empty() { ">".to_owned() } else { format!("> {l}") }).collect::<Vec<_>>().join("\n");
    let question = question.trim();
    if question.is_empty() { quoted } else { format!("{quoted}\n\n{question}") }
}

/// A message with several quotes, each its own blockquote, then the words.
pub fn message_all(quotes: &[String], question: &str) -> String {
    let quoted = quotes.iter().map(|q| message(q, "")).filter(|q| !q.is_empty()).collect::<Vec<_>>().join("

");
    let question = question.trim();
    match (quoted.is_empty(), question.is_empty()) {
        (true, _) => question.to_owned(),
        (false, true) => quoted,
        (false, false) => format!("{quoted}

{question}"),
    }
}

/// A message read back: every quote it starts with, each a blockquote of
/// its own, and the words after them.
pub fn split_all(text: &str) -> (Vec<String>, String) {
    let mut quotes = Vec::new();
    let mut rest = text.to_owned();
    while let Some((quote, after)) = split(rest.trim_start_matches(['\n', ' '])) {
        quotes.push(quote);
        rest = after;
    }
    (quotes, rest.trim().to_owned())
}

/// A message read back: the quote it starts with (its `>` lines, one level
/// of marks taken off) and the words after it. `None`: it starts with none.
pub fn split(text: &str) -> Option<(String, String)> {
    let mut quote: Vec<&str> = Vec::new();
    let mut rest = text;
    while let Some(line) = rest.split('\n').next().filter(|l| l.trim_start().starts_with('>')) {
        let mark = line.trim_start().strip_prefix('>').unwrap_or("");
        quote.push(mark.strip_prefix(' ').unwrap_or(mark).trim_end());
        rest = rest.get(line.len() + 1..).unwrap_or("");
    }
    while quote.last().is_some_and(|l| l.is_empty()) {
        quote.pop();
    }
    if quote.is_empty() {
        return None;
    }
    Some((quote.join("\n"), rest.trim_start_matches(['\n', ' ']).trim_end().to_owned()))
}

/// Text as compared for finding a quote: letters and digits only, lower case
/// — so the Markdown a reply was written in (`**`, `` ` ``, list marks) and
/// how lines broke don't keep the words it showed from being found.
fn squashed(text: &str) -> impl Iterator<Item = (usize, char)> + '_ {
    text.char_indices().filter(|(_, c)| c.is_alphanumeric()).flat_map(|(at, c)| c.to_lowercase().map(move |l| (at, l)))
}

/// Whether `text` (a reply's Markdown, a call's output) holds the quote.
pub fn holds(text: &str, quote: &str) -> bool {
    let needle: String = squashed(quote).map(|(_, c)| c).collect();
    !needle.is_empty() && squashed(text).map(|(_, c)| c).collect::<String>().contains(&needle)
}

/// Where in `text` (a text view's rendered words) the quote is, by bytes of
/// `text`: from its first letter to the end of its last.
pub fn locate(text: &str, quote: &str) -> Option<Range<usize>> {
    let needle: Vec<char> = squashed(quote).map(|(_, c)| c).collect();
    if needle.is_empty() {
        return None;
    }
    let hay: Vec<(usize, char)> = squashed(text).collect();
    let start = hay.windows(needle.len()).position(|w| w.iter().map(|(_, c)| *c).eq(needle.iter().copied()))?;
    let (first, _) = hay[start];
    let (last, _) = hay[start + needle.len() - 1];
    let end = last + text[last..].chars().next().map_or(0, char::len_utf8);
    Some(first..end)
}

/// A box on screen, in the window's points: left, top, width, height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// How far what floats keeps from the window's edges.
pub const EDGE: f32 = 12.;

/// The small button: just over the selection's start, its left at it —
/// never on the words being read; under the selection's end when there is
/// no room above. Kept inside the window.
pub fn button_at(start: Rect, end: Rect, size: (f32, f32), window: (f32, f32)) -> (f32, f32) {
    let x = start.x.clamp(EDGE, (window.0 - size.0 - EDGE).max(EDGE));
    let above = start.y - size.1 - 6.;
    let y = if above >= EDGE { above } else { (end.bottom() + 6.).min(window.1 - size.1 - EDGE).max(EDGE) };
    (x, y)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_is_trimmed_to_its_words() {
        assert_eq!(trim("  \n\n  첫 줄  \n\n\n  둘째 줄\n  \n").as_deref(), Some("첫 줄\n\n둘째 줄"));
        assert_eq!(trim("one\r\ntwo\r\n").as_deref(), Some("one\ntwo"));
        // A text view's indentation is non-breaking space.
        assert_eq!(trim("x\n\u{A0}\u{A0}a\n\u{A0}\u{A0}\u{A0}\u{A0}b").as_deref(), Some("x\na\n  b"));
        // Begun mid-line, after a space.
        assert_eq!(trim(" 보드를 살펴보고\n진행 순서").as_deref(), Some("보드를 살펴보고\n진행 순서"));
        assert_eq!(trim(" \n\t\n"), None);
        assert_eq!(trim(""), None);
    }

    #[test]
    fn a_long_selection_keeps_its_start() {
        let long = "가".repeat(MAX_QUOTE + 50);
        let quote = trim(&long).unwrap();
        assert_eq!(quote.chars().count(), MAX_QUOTE + 1);
        assert!(quote.ends_with('…'));
    }

    #[test]
    fn the_message_quotes_every_line_then_asks() {
        assert_eq!(message("첫 줄\n\n둘째 줄", "  이게 무슨 뜻이에요? "), "> 첫 줄\n>\n> 둘째 줄\n\n이게 무슨 뜻이에요?");
        assert_eq!(message("one", ""), "> one");
    }

    #[test]
    fn a_message_reads_back_as_its_quote_and_words() {
        let sent = message("T-12를 먼저\n\n- 헤더 정리", "왜 먼저죠?\n둘째 줄");
        assert_eq!(split(&sent), Some(("T-12를 먼저\n\n- 헤더 정리".to_owned(), "왜 먼저죠?\n둘째 줄".to_owned())));
        // Typed by hand, without the space; nested marks keep their inner level.
        assert_eq!(split(">a\n>> b\nwhy"), Some(("a\n> b".to_owned(), "why".to_owned())));
        assert_eq!(split("> only a quote"), Some(("only a quote".to_owned(), String::new())));
        assert_eq!(split("no quote\n> later"), None);
        assert_eq!(split(">\n>\nwords"), None);
        assert_eq!(split(""), None);
    }

    #[test]
    fn a_quote_is_found_whatever_the_markdown() {
        let reply = "네, 확인했습니다. 지금 보드를 살펴보고 **T-12**를 먼저\n처리하겠습니다.";
        assert!(holds(reply, "T-12를 먼저 처리하겠습니다"));
        assert!(!holds(reply, "T-13"));
        assert!(!holds(reply, "  ·  "));
    }

    #[test]
    fn a_quote_is_located_in_rendered_words() {
        let shown = "진행 순서는 다음과 같습니다.\n헤더 컴포넌트 정리\n입력창 한글 조합 확인";
        let range = locate(shown, "헤더 컴포넌트 정리 입력창").unwrap();
        assert_eq!(&shown[range], "헤더 컴포넌트 정리\n입력창");
        let range = locate("Hello, World!", "world").unwrap();
        assert_eq!(range, 7..12);
        assert_eq!(locate("abc", "d"), None);
        assert_eq!(locate("abc", "…"), None);
    }

    #[test]
    fn the_button_sits_over_the_selection_never_on_the_words_read() {
        let start = Rect { x: 100., y: 200., w: 8., h: 20. };
        let end = Rect { x: 300., y: 240., w: 8., h: 20. };
        assert_eq!(button_at(start, end, (120., 28.), (1000., 700.)), (100., 166.), "over its start");
        assert_eq!(button_at(Rect { x: 990., ..start }, end, (120., 28.), (1000., 700.)), (868., 166.), "inside the window");
        let top = Rect { y: 20., ..start };
        assert_eq!(button_at(top, end, (120., 28.), (1000., 700.)), (100., 266.), "no room above: under its end");
    }

    #[test]
    fn several_quotes_go_as_blockquotes_of_their_own_and_read_back_so() {
        let quotes = vec!["첫 인용".to_owned(), "둘째\n\n- 목록".to_owned()];
        let sent = message_all(&quotes, "둘 다 왜요?");
        assert_eq!(sent, "> 첫 인용\n\n> 둘째\n>\n> - 목록\n\n둘 다 왜요?");
        assert_eq!(split_all(&sent), (quotes.clone(), "둘 다 왜요?".to_owned()));
        assert_eq!(split_all(&message_all(&quotes[..1], "")), (vec!["첫 인용".to_owned()], String::new()));
        assert_eq!(split_all("그냥 말"), (Vec::new(), "그냥 말".to_owned()));
        assert_eq!(message_all(&[], "그냥 말"), "그냥 말");
    }

}
