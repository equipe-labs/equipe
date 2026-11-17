//! What a ticket's title is, so the person can tell in a second what it is.
//!
//! A title is a short outcome in the squad's language: what changes, or what
//! is checked, for whom. It carries no ticket id, internal test code, bracketed
//! prefix or arrow chain (those belong in the body), and no sort-of-ticket
//! word (QA, verify, fix, deploy): that is the ticket's [`TicketLabel`], a chip
//! beside it. [`check`] is the rule, deterministic, with the problem said so
//! that whoever wrote it can write it again; the daemon runs it when a member
//! or the PM creates or renames a ticket. [`cut`] is how a screen shortens a
//! title that is longer anyway (one written before the rule).
//!
//! Which script the squad's language is written in is data, not code: each
//! locale says it (`title.script`), and [`check`] takes it as a name.

use serde::{Deserialize, Serialize};

/// A title this long (characters) or shorter reads at a glance.
pub const GOOD_CHARS: usize = 40;
/// ... and this many words.
pub const GOOD_WORDS: usize = 8;
/// A title longer than this many characters is refused.
pub const MAX_CHARS: usize = 60;
/// ... or than this many words.
pub const MAX_WORDS: usize = 12;
/// The most characters of the one-line ask a waiting ticket shows the person
/// (the effort comes after it).
pub const ASK_MAX_CHARS: usize = 50;

/// What sort of work a ticket is, in one word — the chip a list shows. Not the
/// ticket's `kind` (where its work happens): this is for the eye.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketLabel {
    /// Something new for its users.
    Feature,
    /// Something wrong made right.
    Fix,
    /// Looking at whether something is right: QA, verifying, reviewing.
    Check,
    /// Putting something live.
    Deploy,
    /// How something looks or is laid out.
    Design,
    /// Housekeeping nobody sees.
    Chore,
}

impl TicketLabel {
    pub const ALL: [TicketLabel; 6] = [TicketLabel::Feature, TicketLabel::Fix, TicketLabel::Check, TicketLabel::Deploy, TicketLabel::Design, TicketLabel::Chore];

    /// Its name on the wire and in a tool's argument; the locale key of its
    /// word is `ticket.label.<key>`.
    pub fn key(self) -> &'static str {
        match self {
            TicketLabel::Feature => "feature",
            TicketLabel::Fix => "fix",
            TicketLabel::Check => "check",
            TicketLabel::Deploy => "deploy",
            TicketLabel::Design => "design",
            TicketLabel::Chore => "chore",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        TicketLabel::ALL.into_iter().find(|l| l.key() == text)
    }
}

/// What is wrong with a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    Empty,
    TooLong { chars: usize },
    TooManyWords { words: usize },
    /// An id or an internal test code: `T-12`, `TP-3`, `#12`, `ABC-7`.
    Id(String),
    /// Starts with `[…]`, `【…】` or `(…)`.
    Bracket,
    /// An arrow: a chain of steps instead of an outcome.
    Arrow,
    /// Cut off in the middle (`…`).
    Cut,
    /// Not written in the squad's language: its name.
    Language(String),
}

impl Problem {
    /// Said to whoever wrote the title, so they can write it again.
    pub fn message(&self) -> String {
        match self {
            Problem::Empty => "The title is empty.".to_owned(),
            Problem::TooLong { chars } => format!("The title is {chars} characters; at most {MAX_CHARS}, and ideally {GOOD_CHARS} or fewer."),
            Problem::TooManyWords { words } => format!("The title is {words} words; at most {MAX_WORDS}, and ideally {GOOD_WORDS} or fewer."),
            Problem::Id(found) => format!("The title has `{found}`, an id or internal code: put it in the body; the board shows the ticket's number itself."),
            Problem::Bracket => "The title starts with a bracketed prefix: drop it, or give the ticket a `label` instead.".to_owned(),
            Problem::Arrow => "The title has an arrow: a title says the outcome, not the steps; put the steps in the body.".to_owned(),
            Problem::Cut => "The title is cut off with an ellipsis: write it whole, and shorter.".to_owned(),
            Problem::Language(language) => format!("The title isn't in the squad's language ({language}): write it in {language}."),
        }
    }
}

/// Names that look like an id (`UTF-8`) and are not.
const NOT_IDS: &[&str] = &["UTF", "ISO", "COVID", "SHA", "AES", "RSA", "MD", "USB", "GPT", "TLS", "SSL", "WPA", "MP", "H", "IPV", "RFC", "DB"];

const ARROWS: &[&str] = &["→", "⇒", "➔", "➜", "➡", "->", "=>", "-->", "⟶"];

/// The id or code in `title`, as written: `T-896`, `TP-1`, `#12`.
fn find_id(title: &str) -> Option<String> {
    let chars: Vec<char> = title.chars().collect();
    let boundary = |at: usize| at == 0 || !chars[at - 1].is_alphanumeric();
    for (at, &c) in chars.iter().enumerate() {
        if c == '#' && boundary(at) && chars.get(at + 1).is_some_and(char::is_ascii_digit) {
            let digits: String = chars[at + 1..].iter().take_while(|d| d.is_ascii_digit()).collect();
            return Some(format!("#{digits}"));
        }
        if c.is_ascii_alphabetic() && boundary(at) {
            let letters: String = chars[at..].iter().take_while(|l| l.is_ascii_alphabetic()).collect();
            let dash = at + letters.len();
            if letters.len() > 5 || chars.get(dash) != Some(&'-') || !chars.get(dash + 1).is_some_and(char::is_ascii_digit) {
                continue;
            }
            let upper = letters.chars().all(|l| l.is_ascii_uppercase());
            let ticket = letters.eq_ignore_ascii_case("t");
            if (upper && !NOT_IDS.contains(&letters.as_str())) || ticket {
                let digits: String = chars[dash + 1..].iter().take_while(|d| d.is_ascii_digit()).collect();
                return Some(format!("{letters}-{digits}"));
            }
        }
    }
    None
}

fn starts_with_bracket(title: &str) -> bool {
    let title = title.trim_start();
    if title.starts_with(['[', '【', '［', '〔']) {
        return true;
    }
    // `(QA) Check the form`: a short parenthesis, then the title proper.
    title.starts_with('(') && title.char_indices().take(18).find(|(_, c)| *c == ')').is_some_and(|(at, _)| !title[at + 1..].trim().is_empty())
}

#[derive(Default)]
struct Letters {
    latin: usize,
    hangul: usize,
    kana: usize,
    han: usize,
    other: usize,
}

fn letters(title: &str) -> Letters {
    let mut n = Letters::default();
    for c in title.chars().filter(|c| c.is_alphabetic()) {
        match c {
            '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' => n.hangul += 1,
            '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' => n.kana += 1,
            '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' => n.han += 1,
            c if (c as u32) <= 0x24F => n.latin += 1,
            _ => n.other += 1,
        }
    }
    n
}

/// Whether the letters of `title` are in the script a squad's language is
/// written in — `latin`, `hangul`, `japanese` (kana and kanji) or `han` — when
/// that is obvious: a Latin squad's title has no other script's letters (two
/// at most, for a name); any other's is mostly its own, the rest being names
/// and terms in Latin letters. A script not named here is not checked.
pub fn in_script(title: &str, script: &str) -> bool {
    let n = letters(title);
    let total = n.latin + n.hangul + n.kana + n.han + n.other;
    if total == 0 {
        return true;
    }
    let own = match script {
        "latin" => return total - n.latin < 2,
        "hangul" => n.hangul,
        "japanese" => n.kana + n.han,
        "han" => n.han,
        _ => return true,
    };
    own * 2 >= total
}

/// Everything wrong with `title` for a squad whose language is written in
/// `script` and is called `language` in English: nothing, when it is a title.
pub fn problems(title: &str, script: &str, language: &str) -> Vec<Problem> {
    let title = title.trim();
    if title.is_empty() {
        return vec![Problem::Empty];
    }
    let mut out = Vec::new();
    let chars = title.chars().count();
    if chars > MAX_CHARS {
        out.push(Problem::TooLong { chars });
    }
    let words = title.split_whitespace().count();
    if words > MAX_WORDS {
        out.push(Problem::TooManyWords { words });
    }
    if let Some(found) = find_id(title) {
        out.push(Problem::Id(found));
    }
    if starts_with_bracket(title) {
        out.push(Problem::Bracket);
    }
    if ARROWS.iter().any(|a| title.contains(a)) {
        out.push(Problem::Arrow);
    }
    if title.ends_with('…') || title.ends_with("...") {
        out.push(Problem::Cut);
    }
    if !in_script(title, script) {
        out.push(Problem::Language(language.to_owned()));
    }
    out
}

/// [`problems`] as one message to write the title again from, or `Ok`.
pub fn check(title: &str, script: &str, language: &str) -> Result<(), String> {
    let found = problems(title, script, language);
    if found.is_empty() {
        return Ok(());
    }
    let said: Vec<String> = found.iter().map(Problem::message).collect();
    Err(format!("{} A title is a short outcome in {language} — what changes or is checked, for whom — like \"Share the 3 sheets with the service account\".", said.join(" ")))
}

/// Whether a title is longer than reads at a glance, though it passes.
pub fn is_long(title: &str) -> bool {
    title.chars().count() > GOOD_CHARS || title.split_whitespace().count() > GOOD_WORDS
}

/// `title` cut to at most `max` characters, at a word's end when there is one
/// near, with `…` — never in the middle of a word or after a dangling mark.
pub fn cut(title: &str, max: usize) -> String {
    let title = title.trim();
    if title.chars().count() <= max {
        return title.to_owned();
    }
    let kept: String = title.chars().take(max.saturating_sub(1)).collect();
    // A word's end, if it is in the last third (languages without spaces cut where they are).
    let at = kept.char_indices().rev().find(|(_, c)| c.is_whitespace()).map(|(i, _)| i).filter(|i| kept[..*i].chars().count() * 3 >= max * 2);
    let head = at.map_or(kept.as_str(), |i| &kept[..i]);
    let head = head.trim_end_matches(|c: char| c.is_whitespace() || ",;:-–—(/·&+".contains(c));
    format!("{head}…")
}

/// What a waiting ticket asks of the person in one line: `ask`, then how long
/// it takes (`effort`, in their language: "1 min"), the way the headline reads.
pub fn headline(ask: &str, effort: &str) -> String {
    format!("{} · {effort}", ask.trim().trim_end_matches(['.', '。']))
}

/// The one-line ask checked: short, no id, arrow or bracket prefix.
pub fn check_ask(ask: &str) -> Result<(), String> {
    let ask = ask.trim();
    if ask.is_empty() {
        return Err("`ask` is empty: say in one line what the person has to do.".into());
    }
    let chars = ask.chars().count();
    let mut bad = Vec::new();
    if chars > ASK_MAX_CHARS {
        bad.push(format!("`ask` is {chars} characters; at most {ASK_MAX_CHARS}."));
    }
    if ask.contains('\n') {
        bad.push("`ask` is one line.".to_owned());
    }
    if let Some(found) = find_id(ask) {
        bad.push(format!("`ask` has `{found}`, an id or internal code: the person doesn't read those."));
    }
    if starts_with_bracket(ask) {
        bad.push("`ask` starts with a bracketed prefix: drop it.".to_owned());
    }
    if ARROWS.iter().any(|a| ask.contains(a)) {
        bad.push("`ask` has an arrow: say what to do, not the steps.".to_owned());
    }
    if bad.is_empty() { Ok(()) } else { Err(format!("{} Write it as what the person does, like \"Share the 3 sheets with the service account\"; the checklist (`needs`) has the steps.", bad.join(" "))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ko(title: &str) -> Vec<Problem> {
        problems(title, "hangul", "Korean")
    }

    fn en(title: &str) -> Vec<Problem> {
        problems(title, "latin", "English")
    }

    #[test]
    fn a_short_outcome_in_the_squads_language_passes() {
        assert!(en("Share the 3 sheets with the service account").is_empty());
        assert!(ko("구글시트 3개를 서비스 계정에 공유").is_empty());
        assert!(ko("Nightly 동기화 실패 원인 확인").is_empty(), "a name in Latin letters is fine");
        assert!(problems("本番環境のフォームを確認", "japanese", "Japanese").is_empty());
        assert!(problems("检查所有表单的保存流程", "han", "Chinese").is_empty());
        assert!(en("Check UTF-8 and ISO-8601 dates").is_empty(), "standard names are not ids");
    }

    #[test]
    fn ids_and_internal_codes_are_refused() {
        assert_eq!(en("Fix login (T-896)"), [Problem::Id("T-896".into())]);
        assert_eq!(en("Run TP-1~24 again"), [Problem::Id("TP-1".into())]);
        assert_eq!(en("t-12 follow-up"), [Problem::Id("t-12".into())]);
        assert_eq!(en("Close ticket #45"), [Problem::Id("#45".into())]);
        assert_eq!(en("Sync ABC-7 export"), [Problem::Id("ABC-7".into())]);
        assert!(en("Roll out v2-beta and Wi-Fi 6").is_empty());
        assert!(en("Costs for 2024-05 report").is_empty(), "a date is not an id");
    }

    #[test]
    fn a_bracketed_prefix_is_refused() {
        assert!(en("[QA] Check the forms").contains(&Problem::Bracket));
        assert!(en("【重要】Check the forms").contains(&Problem::Bracket));
        assert!(en("(QA) Check the forms").contains(&Problem::Bracket));
        assert!(en("Check the forms (all six)").is_empty(), "a bracket later is just words");
    }

    #[test]
    fn arrows_are_refused() {
        assert!(en("Save → edit → submit").contains(&Problem::Arrow));
        assert!(en("Save -> submit").contains(&Problem::Arrow));
        assert!(en("Save => submit").contains(&Problem::Arrow));
        assert!(en("Check the sign-in flow").is_empty(), "a hyphen is not an arrow");
    }

    #[test]
    fn too_long_and_cut_off_are_refused() {
        let long = "Verify all four exit-plan sheets after deployment and publish the fund-map report";
        assert!(en(long).iter().any(|p| matches!(p, Problem::TooLong { .. })));
        let words = "a b c d e f g h i j k l m";
        assert!(en(words).iter().any(|p| matches!(p, Problem::TooManyWords { words: 13 })));
        assert!(en("Publish the fund-map re…").contains(&Problem::Cut));
        assert!(en("Publish the fund-map...").contains(&Problem::Cut));
        assert_eq!(problems("  ", "latin", "English"), [Problem::Empty]);
        assert!(is_long("Check that every single form saves and then submits cleanly"));
        assert!(!is_long("Check the forms"));
    }

    #[test]
    fn the_title_is_in_the_squads_language() {
        // The real titles from a Korean squad: a mixed one, an English one.
        assert!(ko("Nightly 사업계획 회수계획 동기화 workflow failing two nights running (ECS task failed)").contains(&Problem::Language("Korean".into())));
        assert!(ko("Verify the exit plan sheets").contains(&Problem::Language("Korean".into())));
        assert!(en("구글시트를 서비스 계정에 공유").contains(&Problem::Language("English".into())));
        assert!(problems("Check the forms", "japanese", "Japanese").contains(&Problem::Language("Japanese".into())));
        assert!(problems("Check the forms", "han", "Chinese").contains(&Problem::Language("Chinese".into())));
        assert!(problems("Check the forms", "unknown", "Elvish").is_empty(), "a script not named is not checked");
        assert!(ko("T-1 123").contains(&Problem::Id("T-1".into())) && !ko("123 45").contains(&Problem::Language("Korean".into())), "no letters: no language");
    }

    #[test]
    fn the_real_titles_are_all_refused_with_their_reasons() {
        for title in [
            "Tinumi 전체 운영 QA: TP-1~24 + 시나리오 10개(T-896) 판정",
            "Nightly 사업계획 회수계획 동기화 workflow failing two nights running (ECS task failed)",
            "[T-1326 이어가기] ERP 기안 전 과정 운영 시험: 6개 양식 저장→수정→결재선 자동→상신→회수→삭제",
            "Verify all four exit-plan sheets after deployment and publish fund-map re…",
        ] {
            assert!(!ko(title).is_empty(), "{title}");
        }
        let said = check("[T-1326 이어가기] ERP 양식 저장→수정", "hangul", "Korean").unwrap_err();
        for part in ["T-1326", "bracketed prefix", "arrow", "short outcome in Korean"] {
            assert!(said.contains(part), "{part}: {said}");
        }
    }

    #[test]
    fn a_long_title_is_cut_at_a_word_with_an_ellipsis() {
        assert_eq!(cut("Short", 40), "Short");
        assert_eq!(cut("Verify all four exit-plan sheets after deployment today", 40), "Verify all four exit-plan sheets after…");
        assert!(cut("공유 시트 접근 권한을 서비스 계정에게 모두 부여하고 확인한다", 12).chars().count() <= 12);
        assert_eq!(cut("aaaaaaaaaaaaaaaaaaaaaaaa", 10), "aaaaaaaaa…");
        assert_eq!(cut("Sync the data, then, and then more words here", 21), "Sync the data…");
    }

    #[test]
    fn the_ask_is_one_short_line_with_no_codes_then_its_effort() {
        assert!(check_ask("Share the 3 sheets with the service account").is_ok());
        assert!(check_ask("").is_err());
        assert!(check_ask(&"x".repeat(51)).unwrap_err().contains("at most 50"));
        assert!(check_ask("Share T-12's sheets").unwrap_err().contains("T-12"));
        assert!(check_ask("Open → share").unwrap_err().contains("arrow"));
        assert!(check_ask("a\nb").unwrap_err().contains("one line"));
        assert_eq!(headline("Share the sheets.", "1 min"), "Share the sheets · 1 min");
    }

    #[test]
    fn labels_are_named_and_parsed() {
        for label in TicketLabel::ALL {
            assert_eq!(TicketLabel::parse(label.key()), Some(label));
            assert_eq!(serde_json::to_value(label).unwrap(), label.key());
        }
        assert_eq!(TicketLabel::parse(" Fix "), Some(TicketLabel::Fix));
        assert_eq!(TicketLabel::parse("qa"), None);
    }
}
