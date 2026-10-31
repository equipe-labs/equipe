//! A member's reply that ends asking the person several things in a list of
//! its own words ("알려 주세요: 1. … 2. … 3. …") instead of the tools made
//! for it: which list that is, what each item asks, the quick answers that
//! fit one (네 · 아니요, 같은 걸로), and the one message the answers go back
//! in, "1) 전화번호 → +82 …". Also the "1." the person starts typing by hand
//! after such a reply. No UI types here, so the rules are tested without a
//! window; the form is `views::asklist`'s.

use std::ops::Range;

/// A list in a reply that asks the person things, item by item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    /// Where the list is in the reply: from its first item's line to the end
    /// of its last line (without the line break).
    pub span: Range<usize>,
    pub items: Vec<Item>,
}

/// One thing asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The item as written (Markdown), its sub-lines joined on.
    pub text: String,
    /// Answers a tap gives, where one obviously fits.
    pub chips: Vec<Chip>,
}

/// A quick answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chip {
    Yes,
    No,
    /// "The same as above" — for an item asking whether what was given before will do.
    Same,
}

/// Lines that ask for what follows, in the languages equipe speaks.
fn leads() -> &'static [&'static str] {
    crate::i18n::every("match.ask_leads")
}

/// What says the work was done, not asked about (beside Korean's past
/// tense, which [`past_ko`] reads).
fn done() -> &'static [&'static str] {
    crate::i18n::every("match.done")
}

/// Endings of a sentence that asks or requests, punctuation aside (and the
/// question marks themselves).
fn asking_ends() -> impl Iterator<Item = &'static &'static str> {
    ["?", "？"].iter().chain(crate::i18n::every("match.asking_ends"))
}

/// Words that make a question more than a yes or a no.
fn open_words() -> &'static [&'static str] {
    crate::i18n::every("match.open_words")
}

/// Endings of a question a yes or a no answers — once it has none of
/// [`open_words`].
fn yes_no_ends() -> &'static [&'static str] {
    crate::i18n::every("match.yes_no_ends")
}

/// English (and the like) questions a yes or a no answers start so.
fn yes_no_starts() -> &'static [&'static str] {
    crate::i18n::every("match.yes_no_starts")
}

/// What asks whether what was given before will do.
fn same() -> &'static [&'static str] {
    crate::i18n::every("match.same")
}

/// How many characters may follow the list ("받으면 바로 입력합니다") for it to
/// still be what the reply ends on.
const TAIL: usize = 600;

/// The list in `reply` that asks the person things, if there is one: at
/// least two items, and either it follows a line asking for what follows
/// ("알려 주세요", "Questions") or most of its items ask ("…?", "…주세요").
/// A list of what was done, or of options laid out, isn't one. The last such
/// list wins, near the end of the reply.
pub fn find(reply: &str) -> Option<Asked> {
    let lists = lists(reply);
    for list in lists.iter().rev() {
        let tail = reply[list.span.end..].trim();
        if tail.chars().count() > TAIL {
            break;
        }
        if list.items.len() < 2 || !asks(reply, list) {
            continue;
        }
        let items = list.items.iter().map(|text| Item { chips: chips(text), text: text.clone() }).collect();
        return Some(Asked { span: list.span.clone(), items });
    }
    None
}

/// Whether a list asks: by the line it follows, or by its items.
fn asks(reply: &str, list: &List) -> bool {
    let n = list.items.len();
    let done = list.items.iter().filter(|i| is_done(i)).count();
    if done * 2 > n {
        return false;
    }
    let lead = reply[..list.span.start].lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    let lead = lead.to_lowercase();
    // "FAQ에 다음 질문을 넣었습니다:" — what follows was done.
    if is_done(&lead) {
        return false;
    }
    if leads().iter().any(|w| lead.contains(w)) {
        return true;
    }
    let asking = list.items.iter().filter(|i| is_asking(i)).count();
    asking >= 2 && asking * 2 >= n
}

/// An item's words, without Markdown's marks and on one line.
fn plain(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.replace("**", "").replace("__", "").replace('`', "")
}

/// The words with what closes a sentence taken off its end.
fn bare(text: &str) -> String {
    plain(text).trim_end_matches(['.', '。', '…', ')', '）', '"', '\'', '”', '’', '!', ' ', ':', '：']).to_owned()
}

/// Whether an item asks or requests something.
pub fn is_asking(text: &str) -> bool {
    let words = plain(text);
    let end = bare(text);
    let lower = words.to_lowercase();
    words.trim_end().ends_with(['?', '？']) || asking_ends().any(|e| end.ends_with(e)) || lower.starts_with("please ")
}

/// Whether an item says what was done.
fn is_done(text: &str) -> bool {
    let end = bare(text);
    if past_ko(&end) || done().iter().any(|d| end.ends_with(d)) {
        return true;
    }
    // "Fixed the header", "Added a test".
    let first = end.split_whitespace().next().unwrap_or_default();
    first.len() > 3 && first.chars().next().is_some_and(char::is_uppercase) && first.ends_with("ed") && !end.ends_with('?')
}

/// Whether Korean words end in the past tense: "고쳤습니다", "했어요" — a
/// syllable closing on ㅆ before the ending ("있습니다" is the present).
fn past_ko(end: &str) -> bool {
    let Some(stem) = crate::i18n::every("match.past_endings").iter().find_map(|e| end.strip_suffix(e)) else { return false };
    let Some(last) = stem.chars().last() else { return false };
    let code = last as u32;
    let present = crate::i18n::every("match.past_not");
    (0xAC00..=0xD7A3).contains(&code) && (code - 0xAC00) % 28 == 20 && !present.iter().any(|p| p.chars().eq(std::iter::once(last)))
}

/// Whether `text` (lowercase) has one of `words`: an ASCII one as a whole
/// word ("how" isn't in "show"), any other anywhere.
fn has_any(text: &str, words: &[&str]) -> bool {
    let tokens: Vec<&str> = text.split(|c: char| !c.is_alphanumeric() && c != '\'').filter(|t| !t.is_empty()).collect();
    words.iter().any(|w| {
        let w = w.trim();
        if w.is_ascii() { tokens.contains(&w) } else { text.contains(w) }
    })
}

/// The quick answers that fit an item.
fn chips(text: &str) -> Vec<Chip> {
    let lower = plain(text).to_lowercase();
    if same().iter().any(|w| lower.contains(w)) {
        return vec![Chip::Same, Chip::No];
    }
    if !is_asking(text) {
        return Vec::new();
    }
    // What comes after "label:" is the question.
    let question = match lower.split_once(": ").or_else(|| lower.split_once('：')) {
        Some((_, rest)) => rest.to_owned(),
        None => lower.clone(),
    };
    if has_any(&question, open_words()) {
        return Vec::new();
    }
    let end = bare(&question);
    let end = end.trim_end_matches(['?', '？']).trim_end();
    let start = question.trim_start_matches('¿');
    if yes_no_ends().iter().any(|e| end.ends_with(e)) || yes_no_starts().iter().any(|s| start.starts_with(s.trim_start_matches('¿'))) {
        return vec![Chip::Yes, Chip::No];
    }
    Vec::new()
}

/// An item, short, for the message the answers go in: its label when it is
/// one ("전화번호: 국가번호 포함" → "전화번호"), else the whole of it,
/// clipped.
pub fn summary(text: &str) -> String {
    let words = plain(text);
    let split = words.split_once(": ").or_else(|| words.split_once('：'));
    if let Some((label, rest)) = split {
        let label = label.trim();
        if !label.is_empty() && label.chars().count() <= 40 && !label.contains(['?', '？']) && !is_asking(rest) {
            return label.to_owned();
        }
    }
    clip(words.trim(), 90)
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max - 1).collect();
    format!("{}…", cut.trim_end())
}

/// The one message the answers go back in: an item a line, "1) question →
/// answer" (`skipped` for one left blank), and the person's own words under
/// them.
pub fn message(items: &[Item], answers: &[String], note: &str, skipped: &str) -> String {
    let mut lines: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(n, item)| {
            let answer = answers.get(n).map(|a| a.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" / ")).unwrap_or_default();
            let answer = if answer.is_empty() { skipped.to_owned() } else { answer };
            format!("{}) {} → {answer}", n + 1, summary(&item.text))
        })
        .collect();
    let note = note.trim();
    if !note.is_empty() {
        lines.push(String::new());
        lines.push(note.to_owned());
    }
    lines.join("\n")
}

/// Whether there is anything to send: an answer, or words of their own.
pub fn can_send(answers: &[String], note: &str) -> bool {
    answers.iter().any(|a| !a.trim().is_empty()) || !note.trim().is_empty()
}

/// What follows the "1." (or "1)") the person started their message with,
/// when they started it so: they are answering a list by hand.
pub fn started(typed: &str) -> Option<&str> {
    let text = typed.trim_start();
    let rest = text.strip_prefix('1')?;
    let rest = rest.trim_start_matches(' ');
    let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim())
}

// ── the lists in a reply ──

/// A list as written: where it is, and its items.
struct List {
    span: Range<usize>,
    items: Vec<String>,
}

/// What kind of item marker a line starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    Number,
    Bullet,
}

/// An item's line: its marker, how far in it starts, and its words.
fn item_line(line: &str) -> Option<(Marker, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if (1..=2).contains(&digits) {
        let after = &rest[digits..];
        let words = after.strip_prefix(". ").or_else(|| after.strip_prefix(") "))?;
        return Some((Marker::Number, indent, words.trim()));
    }
    for bullet in ["- ", "* ", "+ ", "• "] {
        if let Some(words) = rest.strip_prefix(bullet) {
            return Some((Marker::Bullet, indent, words.trim()));
        }
    }
    None
}

/// Every top-level list in `reply`, outside code fences, in order.
fn lists(reply: &str) -> Vec<List> {
    // Each line with where it starts.
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut at = 0;
    for line in reply.split('\n') {
        lines.push((at, line));
        at += line.len() + 1;
    }
    let mut out: Vec<List> = Vec::new();
    let mut fence: Option<&str> = None;
    let mut current: Option<(Marker, List)> = None;
    // A blank line inside a list: it goes on only when an item comes next.
    let mut gap = false;
    for &(start, raw) in &lines {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(if trimmed.starts_with("```") { "```" } else { "~~~" });
            if let Some((_, list)) = current.take() {
                out.push(list);
            }
            continue;
        }
        let end = start + line.len();
        if trimmed.is_empty() {
            gap = current.is_some();
            continue;
        }
        let indent = line.len() - trimmed.len();
        // Under a list, an item set in is a sub-item of the one before.
        let item = item_line(line).filter(|(_, at, _)| *at <= if current.is_some() { 1 } else { 3 });
        match (item, current.as_mut()) {
            // An item of the list under way, at its level.
            (Some((marker, _, words)), Some((kind, list))) if *kind == marker => {
                list.items.push(words.to_owned());
                list.span.end = end;
                gap = false;
            }
            // A new list.
            (Some((marker, _, words)), _) => {
                if let Some((_, list)) = current.take() {
                    out.push(list);
                }
                current = Some((marker, List { span: start..end, items: vec![words.to_owned()] }));
                gap = false;
            }
            // A line under an item: its sub-items and the rest of its words.
            (_, Some((_, list))) if indent >= 2 || !gap => {
                if let Some(last) = list.items.last_mut() {
                    last.push(' ');
                    last.push_str(trimmed.trim_start_matches(['-', '*', '+', '•']).trim());
                }
                list.span.end = end;
                gap = false;
            }
            _ => {
                if let Some((_, list)) = current.take() {
                    out.push(list);
                }
                gap = false;
            }
        }
    }
    if let Some((_, list)) = current.take() {
        out.push(list);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reply the person saw: a heading, three things to tell, a line after.
    const SCREENSHOT: &str = "EU 디지털 서비스법(DSA) 때문에 EU 스토어에 앱을 내려면 판매자 연락처를 공개해야 합니다. \
App Store Connect에 아직 비어 있어서 **T-117** 제출이 여기서 멈춰 있어요.\n\n\
## 알려 주세요\n\n\
1. EU 스토어에 공개할 전화번호: 국가번호 포함 (예: +82 10-1234-5678)\n\
2. 공개할 이메일: 스토어 페이지에 그대로 보입니다\n\
3. T-117 심사 연락처: 위와 같은 번호와 메일을 써도 되는지\n\n\
받으면 바로 입력합니다.";

    #[test]
    fn the_screenshots_reply_asks_three_things() {
        let asked = find(SCREENSHOT).expect("a list that asks");
        assert_eq!(asked.items.len(), 3);
        assert!(SCREENSHOT[asked.span.clone()].starts_with("1. EU 스토어"), "{:?}", &SCREENSHOT[asked.span.clone()]);
        assert!(SCREENSHOT[asked.span.clone()].ends_with("써도 되는지"));
        assert_eq!(asked.items[0].text, "EU 스토어에 공개할 전화번호: 국가번호 포함 (예: +82 10-1234-5678)");
        assert_eq!(asked.items[0].chips, vec![]);
        assert_eq!(asked.items[1].chips, vec![]);
        assert_eq!(asked.items[2].chips, vec![Chip::Same, Chip::No]);
        assert_eq!(summary(&asked.items[0].text), "EU 스토어에 공개할 전화번호");
        assert_eq!(summary(&asked.items[1].text), "공개할 이메일");
        assert_eq!(summary(&asked.items[2].text), "T-117 심사 연락처: 위와 같은 번호와 메일을 써도 되는지");
    }

    #[test]
    fn the_answers_go_back_in_one_message_an_item_a_line() {
        let asked = find(SCREENSHOT).unwrap();
        let answers = ["+82 10-8123-4567".to_owned(), String::new(), "같은 걸로".to_owned()];
        let sent = message(&asked.items, &answers, "  주말엔 메일로만 연락 주세요 ", "(건너뜀)");
        assert_eq!(
            sent,
            "1) EU 스토어에 공개할 전화번호 → +82 10-8123-4567\n2) 공개할 이메일 → (건너뜀)\n3) T-117 심사 연락처: 위와 같은 번호와 메일을 써도 되는지 → 같은 걸로\n\n주말엔 메일로만 연락 주세요"
        );
        assert_eq!(message(&asked.items[..1], &["a\n\n b ".into()], "", "-"), "1) EU 스토어에 공개할 전화번호 → a / b");
        assert!(can_send(&answers, ""));
        assert!(!can_send(&[" ".into(), String::new()], " "));
        assert!(can_send(&[String::new()], "just this"));
    }

    #[test]
    fn questions_in_english_and_other_ways_of_writing_them() {
        let english = "Almost there. Before I submit the build:\n\n**Questions**\n\n- Which phone number should the EU listing show?\n- Should the review contact be the same as above?\n- Can I use your Apple ID email for the support URL?\n\nOnce I have these I'll fill them in.";
        let asked = find(english).expect("english questions");
        assert_eq!(asked.items.len(), 3);
        assert_eq!(asked.items[0].chips, vec![], "which… is open");
        assert_eq!(asked.items[1].chips, vec![Chip::Same, Chip::No]);
        assert_eq!(asked.items[2].chips, vec![Chip::Yes, Chip::No]);

        // No heading: the items themselves ask.
        let bare = "배포 전에 두 가지만요.\n\n1) 도메인은 equipe.im 그대로 쓸까요?\n2) 출시일을 정해 주세요\n";
        let asked = find(bare).expect("items that ask");
        assert_eq!(asked.items.len(), 2);
        assert_eq!(asked.items[0].chips, vec![Chip::Yes, Chip::No]);

        // "Please tell me" over numbered lines, a sub-line folded into its item.
        let tell = "Please tell me:\n1. The app's display name\n   - as it shows under the icon\n2. The support email\n";
        let asked = find(tell).expect("please tell me");
        assert_eq!(asked.items[0].text, "The app's display name as it shows under the icon");

        // A loose list: blank lines between the items.
        let loose = "확인해 주세요.\n\n1. 스크린샷 다섯 장이 맞는지\n\n2. 앱 설명 문구가 괜찮은지\n\n3. 가격은 무료로 둘지\n";
        let asked = find(loose).expect("a loose list");
        assert_eq!(asked.items.len(), 3);
        assert_eq!(asked.items[0].chips, vec![Chip::Yes, Chip::No]);
        assert_eq!(asked.items[1].chips, vec![Chip::Yes, Chip::No]);
    }

    #[test]
    fn steps_done_options_laid_out_and_lone_items_are_not_questions() {
        // What the member did.
        let done = "끝났습니다. 한 일은 다음과 같습니다.\n\n1. 헤더 컴포넌트를 정리했습니다\n2. 입력창 한글 조합을 고쳤습니다\n3. 스트리밍 표시를 점검했습니다\n\n검토 부탁드립니다.";
        assert_eq!(find(done), None);
        let done_en = "Done:\n\n1. Fixed the header\n2. Added a test for composition\n3. Updated the README\n";
        assert_eq!(find(done_en), None);
        // Even under "확인해 주세요", a list of what was done is a report.
        let check_done = "확인해 주세요:\n- 헤더를 고쳤습니다\n- 테스트를 추가했습니다\n";
        assert_eq!(find(check_done), None);
        // Options laid out, and a recommendation.
        let options = "두 가지 방법이 있습니다.\n\n1. Postgres — 관계형, Supabase에서 운영\n2. SQLite — 파일 하나, 서버 없음\n\n저는 Postgres를 추천합니다.";
        assert_eq!(find(options), None);
        // A choice put as one question over its options: one answer, not one each.
        let choice = "어느 쪽으로 할까요?\n\n- Postgres\n- SQLite\n";
        assert_eq!(find(choice), None);
        // A plan, a bulleted summary.
        let plan = "진행 순서는 다음과 같습니다.\n\n- 헤더 컴포넌트 정리\n- 입력창 한글 조합 확인\n- 스트리밍 표시 점검 (`T-13`)\n\n끝나면 다시 알려 드리겠습니다.";
        assert_eq!(find(plan), None);
        // One thing asked is no list.
        assert_eq!(find("알려 주세요:\n1. 전화번호\n"), None);
        // A FAQ written, its questions and all.
        let faq = "FAQ 페이지에 다음 질문을 넣었습니다:\n- 배송은 얼마나 걸리나요?\n- 환불은 어떻게 하나요?\n";
        assert_eq!(find(faq), None);
        // Questions in a code block are code.
        let code = "예시입니다:\n\n```\n1. 이름을 알려 주세요?\n2. 나이는요?\n```\n";
        assert_eq!(find(code), None);
    }

    #[test]
    fn a_list_far_from_the_end_is_not_what_the_reply_asks() {
        let long = format!("알려 주세요\n1. 이름은?\n2. 나이는?\n\n{}", "그리고 긴 설명이 이어집니다. ".repeat(60));
        assert_eq!(find(&long), None);
        // The questions after a list of what was done.
        let both = "한 일:\n1. 헤더를 고쳤습니다\n2. 테스트를 추가했습니다\n\n남은 질문:\n1. 배포는 오늘 할까요?\n2. 공지 문구는 누가 쓰나요?\n";
        let asked = find(both).expect("the questions");
        assert!(asked.items[0].text.starts_with("배포는"));
    }

    #[test]
    fn a_message_begun_with_one_dot_answers_by_hand() {
        assert_eq!(started("1. +82 10-8"), Some("+82 10-8"));
        assert_eq!(started("1."), Some(""));
        assert_eq!(started("  1) 네"), Some("네"));
        assert_eq!(started("1.5배로 해 줘"), None);
        assert_eq!(started("10. 열 번째"), None);
        assert_eq!(started("안녕"), None);
    }
}
