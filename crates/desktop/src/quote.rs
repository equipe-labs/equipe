//! Asking about — and replying to — stretches of the conversation
//! (`views/quote.rs`): the words the person selected, trimmed into a quote;
//! the cards over the composer, each a passage and what the person says to
//! it; the message that carries them — each passage a Markdown blockquote
//! under a `<!-- reply n -->` mark with the person's words after it, then the
//! composer's own words under `<!-- reply -->` — and that message read back
//! for its bubble; where quoted words are in a text, for the way back to
//! them; and where the bar and the box float.

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// A passage of the conversation and what the person says to it: a card over
/// the composer until sent. Without words it is only the passage, asked about
/// ("Ask about this") — the composer's words are the question.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub quote: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub words: String,
}

impl Reply {
    pub fn asked(quote: impl Into<String>) -> Self {
        Self { quote: quote.into(), words: String::new() }
    }

    /// Whether the person said anything to it.
    pub fn answered(&self) -> bool {
        !self.words.trim().is_empty()
    }
}

/// The line that opens a reply's block: `<!-- reply 1 -->`, `<!-- reply 2 -->`,
/// …; the composer's own words after them open with `<!-- reply -->`.
fn mark(n: Option<usize>) -> String {
    match n {
        Some(n) => format!("<!-- reply {n} -->"),
        None => "<!-- reply -->".to_owned(),
    }
}

/// `line` read as a mark: `Some(Some(n))` a reply's, `Some(None)` the rest's.
fn read_mark(line: &str) -> Option<Option<usize>> {
    let inner = line.trim().strip_prefix("<!--")?.strip_suffix("-->")?.trim();
    let n = inner.strip_prefix("reply")?.trim();
    if n.is_empty() {
        return Some(None);
    }
    n.parse().ok().map(Some)
}

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

/// The message the cards and the composer's words make. Each passage
/// replied to: its mark, its blockquote, the person's words to it — in the
/// cards' order — then the composer's words under their own mark, so the
/// member reads which words answer which passage. Passages only asked
/// about: each its own blockquote, then the question, as plain Markdown.
pub fn message_all(cards: &[Reply], rest: &str) -> String {
    let rest = rest.trim();
    if !cards.iter().any(Reply::answered) {
        let quoted = cards.iter().map(|c| message(&c.quote, "")).filter(|q| !q.is_empty()).collect::<Vec<_>>().join("\n\n");
        return match (quoted.is_empty(), rest.is_empty()) {
            (true, _) => rest.to_owned(),
            (false, true) => quoted,
            (false, false) => format!("{quoted}\n\n{rest}"),
        };
    }
    let mut blocks: Vec<String> = cards.iter().enumerate().map(|(n, c)| format!("{}\n{}", mark(Some(n + 1)), message(&c.quote, &c.words))).collect();
    if !rest.is_empty() {
        blocks.push(format!("{}\n{rest}", mark(None)));
    }
    blocks.join("\n\n")
}

/// A message read back for its bubble: the cards it carries — passages
/// replied to (`<!-- reply n -->`), or quotes it starts with — and the rest.
pub fn read(text: &str) -> (Vec<Reply>, String) {
    if let Some(read) = read_marked(text) {
        return read;
    }
    let (quotes, rest) = split_all(text);
    (quotes.into_iter().map(Reply::asked).collect(), rest)
}

/// A message whose first line is a reply's mark, read block by block.
fn read_marked(text: &str) -> Option<(Vec<Reply>, String)> {
    let text = text.replace("\r\n", "\n");
    let text = text.trim_start();
    read_mark(text.lines().next()?)??;
    let mut blocks: Vec<(Option<usize>, Vec<&str>)> = Vec::new();
    for line in text.split('\n') {
        match (read_mark(line), blocks.last_mut()) {
            (Some(n), _) => blocks.push((n, Vec::new())),
            (None, Some((_, lines))) => lines.push(line),
            (None, None) => {}
        }
    }
    let (mut cards, mut rest) = (Vec::new(), Vec::new());
    for (n, lines) in blocks {
        let body = lines.join("\n");
        let body = body.trim_matches(['\n', ' ']);
        match n {
            Some(_) => match split(body) {
                Some((quote, words)) => cards.push(Reply { quote, words }),
                None if !body.is_empty() => cards.push(Reply { quote: String::new(), words: body.to_owned() }),
                None => {}
            },
            None if !body.is_empty() => rest.push(body.to_owned()),
            None => {}
        }
    }
    Some((cards, rest.join("\n\n")))
}

/// Tab among the cards: from the box (`None`) to the first (back: the
/// last), on to the next, and past either end back to the box.
pub fn tab(at: Option<usize>, cards: usize, back: bool) -> Option<usize> {
    if cards == 0 {
        return None;
    }
    match (at.map(|i| i.min(cards - 1)), back) {
        (None, false) => Some(0),
        (None, true) => Some(cards - 1),
        (Some(i), false) => (i + 1 < cards).then_some(i + 1),
        (Some(i), true) => i.checked_sub(1),
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

/// The reply box's width, room permitting.
pub const BOX_WIDTH: f32 = 380.;

/// The reply box `height` tall, by what it answers (a selection's ends, or a
/// card): its width, where it goes, and whether that is above — over the
/// start, as the bar was; under the end when there is no room. Kept inside
/// the window.
pub fn box_at(start: Rect, end: Rect, height: f32, window: (f32, f32)) -> (f32, (f32, f32), bool) {
    let width = BOX_WIDTH.min(window.0 - 2. * EDGE).max(200.);
    let x = start.x.clamp(EDGE, (window.0 - width - EDGE).max(EDGE));
    let above = start.y - 8. - height;
    if above >= EDGE {
        return (width, (x, above), true);
    }
    let below = (end.bottom() + 8.).min(window.1 - height - EDGE).max(EDGE);
    (width, (x, below), false)
}

/// Where the bar's selection is: as drawn now; gone (a press cleared it, the
/// bar's own included), where it was last drawn — the bar doesn't jump from
/// under the pointer; never drawn (a double-clicked word), where the button
/// let go.
pub fn place(now: Option<(Rect, Rect)>, before: Option<(Rect, Rect)>, released: Rect) -> (Rect, Rect) {
    now.or(before).unwrap_or((released, released))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_stays_where_it_was_when_a_press_clears_the_selection() {
        let r = |x: f32| Rect { x, y: 10., w: 1., h: 1. };
        let drawn = (r(100.), r(300.));
        assert_eq!(place(Some(drawn), None, r(5.)), drawn, "as drawn now");
        assert_eq!(place(None, Some(drawn), r(5.)), drawn, "cleared by the press: where it was, not where the button let go");
        assert_eq!(place(None, None, r(5.)), (r(5.), r(5.)), "never drawn: where the button let go");
    }

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

    fn reply(quote: &str, words: &str) -> Reply {
        Reply { quote: quote.into(), words: words.into() }
    }

    #[test]
    fn replies_go_each_marked_with_its_passage_then_the_rest() {
        let cards = vec![reply("T-12를 먼저\n처리하겠습니다", " 왜 먼저죠? "), reply("헤더 정리", "좋아요\n둘째 줄")];
        assert_eq!(
            message_all(&cards, " 나머지는 내일 봐요 "),
            "<!-- reply 1 -->\n> T-12를 먼저\n> 처리하겠습니다\n\n왜 먼저죠?\n\n<!-- reply 2 -->\n> 헤더 정리\n\n좋아요\n둘째 줄\n\n<!-- reply -->\n나머지는 내일 봐요"
        );
        // Nothing typed in the composer itself: no block for the rest.
        assert_eq!(message_all(&cards[..1], "  "), "<!-- reply 1 -->\n> T-12를 먼저\n> 처리하겠습니다\n\n왜 먼저죠?");
        // A passage only asked about, among replies: its block is the passage alone.
        assert_eq!(message_all(&[reply("a", ""), reply("b", "yes")], ""), "<!-- reply 1 -->\n> a\n\n<!-- reply 2 -->\n> b\n\nyes");
    }

    #[test]
    fn passages_only_asked_about_go_as_before() {
        let asked = vec![reply("첫 인용", ""), reply("둘째", "  ")];
        assert_eq!(message_all(&asked, "둘 다 왜요?"), "> 첫 인용\n\n> 둘째\n\n둘 다 왜요?");
        assert_eq!(message_all(&[], "그냥 말"), "그냥 말");
    }

    #[test]
    fn a_message_of_replies_reads_back_as_its_cards_and_the_rest() {
        let cards = vec![reply("T-12를 먼저\n\n- 헤더 정리", "왜 먼저죠?"), reply("둘째", "네\n\n그리고"), reply("셋째", "")];
        let sent = message_all(&cards, "끝으로 하나 더");
        assert_eq!(read(&sent), (cards.clone(), "끝으로 하나 더".to_owned()));
        assert_eq!(read(&message_all(&cards, "")), (cards, String::new()));
        // As the person writes it by hand: spaces in the marks, CRLF, a passage that holds a mark.
        let typed = "<!--reply 1-->\r\n> a\r\n> <!-- reply 2 -->\r\n\r\nmine\r\n<!-- reply -->\r\nrest";
        assert_eq!(read(typed), (vec![reply("a\n<!-- reply 2 -->", "mine")], "rest".to_owned()));
    }

    #[test]
    fn other_messages_read_back_as_before() {
        assert_eq!(read("> 첫 인용\n\n둘 다 왜요?"), (vec![reply("첫 인용", "")], "둘 다 왜요?".to_owned()));
        assert_eq!(read("그냥 말"), (Vec::new(), "그냥 말".to_owned()));
        // A mark that isn't the message's first line is only words.
        assert_eq!(read("hi\n<!-- reply 1 -->\n> a"), (Vec::new(), "hi\n<!-- reply 1 -->\n> a".to_owned()));
        assert_eq!(read("<!-- replying -->\nx"), (Vec::new(), "<!-- replying -->\nx".to_owned()));
    }

    #[test]
    fn tab_walks_the_cards_and_comes_back_to_the_box() {
        assert_eq!(tab(None, 3, false), Some(0), "from the box, the first");
        assert_eq!(tab(None, 3, true), Some(2), "back from the box, the last");
        assert_eq!(tab(Some(0), 3, false), Some(1));
        assert_eq!(tab(Some(2), 3, false), None, "past the last, the box");
        assert_eq!(tab(Some(0), 3, true), None, "before the first, the box");
        assert_eq!(tab(None, 0, false), None, "no cards");
        assert_eq!(tab(Some(5), 2, true), Some(0), "a card gone meanwhile: from the last");
    }

    #[test]
    fn the_reply_box_floats_over_what_it_answers_and_inside_the_window() {
        let start = Rect { x: 100., y: 300., w: 8., h: 20. };
        let end = Rect { x: 400., y: 340., w: 8., h: 20. };
        assert_eq!(box_at(start, end, 90., (1000., 700.)), (BOX_WIDTH, (100., 202.), true), "above its start");
        assert_eq!(box_at(Rect { y: 40., ..start }, end, 90., (1000., 700.)), (BOX_WIDTH, (100., 368.), false), "no room above: under its end");
        let (w, (x, _), _) = box_at(Rect { x: 900., ..start }, end, 90., (1000., 700.));
        assert_eq!(x + w, 1000. - EDGE, "kept inside the window");
        assert_eq!(box_at(start, end, 90., (300., 700.)).0, 300. - 2. * EDGE, "narrower in a narrow window");
    }

    #[test]
    fn several_quotes_go_as_blockquotes_of_their_own_and_read_back_so() {
        let quotes = vec!["첫 인용".to_owned(), "둘째\n\n- 목록".to_owned()];
        let sent = message_all(&quotes.iter().map(|q| reply(q, "")).collect::<Vec<_>>(), "둘 다 왜요?");
        assert_eq!(sent, "> 첫 인용\n\n> 둘째\n>\n> - 목록\n\n둘 다 왜요?");
        assert_eq!(split_all(&sent), (quotes.clone(), "둘 다 왜요?".to_owned()));
        assert_eq!(split_all(&message_all(&[reply(&quotes[0], "")], "")), (vec!["첫 인용".to_owned()], String::new()));
        assert_eq!(split_all("그냥 말"), (Vec::new(), "그냥 말".to_owned()));
        assert_eq!(message_all(&[], "그냥 말"), "그냥 말");
    }

}
