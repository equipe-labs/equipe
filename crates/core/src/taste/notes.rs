//! Taste notes as their files keep them: a Markdown bullet a note, where it
//! came from and when in italics after it —
//! `- 모서리는 둥글게 _(sent back on T-336, 2026-11-03)_`, and for the
//! designer's, the other tickets it was seen on and whether the person
//! confirmed it — `_(noticed on T-301, 2026-11-03, seen on T-272 T-288,
//! confirmed)_`; without `confirmed` it is "확인 전". A line the person
//! typed by hand, with nothing after it, is theirs. Whatever else the file
//! holds (a heading, a word on what it is, the dated entries the old
//! template kept there) is left as it is. Pure: no disk here.

use crate::protocol::design::{NOTE_MAX, TasteChange, TasteFrom, TasteNote};

/// The most notes kept: a list to read before every proposal, not a book.
pub const MAX: usize = 60;

/// Why a note can't be written as asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalid {
    Empty,
    TooLong,
    /// More than one line.
    Lines,
    /// It looks like a secret value: an account or card number, a key.
    Secret,
    /// The same note is there already.
    Already,
    /// The note named isn't there (any more).
    Missing,
    TooMany,
}

impl Invalid {
    /// Its word in the core's locale files.
    pub fn key(self) -> &'static str {
        match self {
            Invalid::Empty => "taste.invalid_empty",
            Invalid::TooLong => "taste.invalid_long",
            Invalid::Lines => "taste.invalid_lines",
            Invalid::Secret => "taste.invalid_secret",
            Invalid::Already => "taste.invalid_already",
            Invalid::Missing => "taste.invalid_missing",
            Invalid::TooMany => "taste.invalid_many",
        }
    }
}

/// Whether `s` is a date as the files write one: `2026-11-03`.
pub fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
}

fn from_word(from: TasteFrom) -> String {
    match from {
        TasteFrom::Person => "the person".into(),
        TasteFrom::SentBack { ticket } => format!("sent back on T-{ticket}"),
        TasteFrom::Answered { ticket } => format!("answered on T-{ticket}"),
        TasteFrom::Noticed { ticket: Some(t) } => format!("noticed on T-{t}"),
        TasteFrom::Noticed { ticket: None } => "noticed".into(),
    }
}

fn parse_from(word: &str) -> Option<TasteFrom> {
    let word = word.trim().to_ascii_lowercase();
    let ticket = |rest: &str| rest.trim().trim_start_matches("t-").parse::<u32>().ok();
    if matches!(word.as_str(), "the person" | "person" | "you") {
        return Some(TasteFrom::Person);
    }
    if word == "noticed" {
        return Some(TasteFrom::Noticed { ticket: None });
    }
    if let Some(rest) = word.strip_prefix("sent back on ") {
        return ticket(rest).map(|ticket| TasteFrom::SentBack { ticket });
    }
    if let Some(rest) = word.strip_prefix("answered on ") {
        return ticket(rest).map(|ticket| TasteFrom::Answered { ticket });
    }
    word.strip_prefix("noticed on ").and_then(ticket).map(|t| TasteFrom::Noticed { ticket: Some(t) })
}

/// A note as its line in the file.
pub fn line(note: &TasteNote) -> String {
    let mut meta = vec![from_word(note.from)];
    meta.extend(note.date.clone());
    if !note.seen.is_empty() {
        meta.push(format!("seen on {}", note.seen.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(" ")));
    }
    if note.confirmed && note.by_designer() {
        meta.push("confirmed".into());
    }
    format!("- {} _({})_", note.text, meta.join(", "))
}

/// A bullet line read as a note; `None` for an empty one.
fn parse_line(bullet: &str) -> Option<TasteNote> {
    let text = bullet.trim();
    let parsed = text.strip_suffix(")_").and_then(|head| {
        let (words, meta) = head.rsplit_once(" _(")?;
        let mut parts = meta.split(',').map(str::trim);
        let from = parse_from(parts.next()?)?;
        let mut note = TasteNote { text: words.trim().to_owned(), from, ..TasteNote::default() };
        for part in parts {
            if is_date(part) {
                note.date = Some(part.to_owned());
            } else if part.eq_ignore_ascii_case("confirmed") {
                note.confirmed = true;
            } else if let Some(tickets) = part.strip_prefix("seen on ") {
                note.seen = tickets.split_whitespace().filter_map(|t| t.trim_start_matches("T-").parse().ok()).collect();
            }
        }
        Some(note)
    });
    let mut note = parsed.unwrap_or_else(|| TasteNote { text: text.to_owned(), ..TasteNote::default() });
    // The person's own is theirs to stand behind.
    note.confirmed |= !note.by_designer();
    (!note.text.is_empty()).then_some(note)
}

/// What a line of the file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Note,
    Other,
}

/// Each line of `text` and whether it is a note: a bullet outside code and
/// outside a dated section (`## 2026-11-03 · …`, a decision the old
/// template kept here, or its `## YYYY-MM-DD` example).
fn kinds(text: &str) -> Vec<(&str, Kind)> {
    let mut out = Vec::new();
    let mut fence = false;
    let mut dated = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let kind = if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = !fence;
            Kind::Other
        } else if fence {
            Kind::Other
        } else if trimmed.starts_with('#') {
            let heading = trimmed.trim_start_matches('#').trim();
            dated = heading.get(..10).is_some_and(is_date) || heading.starts_with("YYYY");
            Kind::Other
        } else if !dated && (trimmed.starts_with("- ") || trimmed.starts_with("* ")) && !trimmed[2..].trim().is_empty() {
            Kind::Note
        } else {
            Kind::Other
        };
        out.push((line, kind));
    }
    out
}

/// The notes in a file, in its order.
pub fn parse(text: &str) -> Vec<TasteNote> {
    kinds(text).into_iter().filter(|(_, k)| *k == Kind::Note).filter_map(|(l, _)| parse_line(&l.trim_start()[2..])).take(MAX).collect()
}

/// `previous` with its notes made `notes`: where its first note was (else
/// before its first dated section, else at the end), all else as it was.
pub fn write(previous: &str, notes: &[TasteNote]) -> String {
    let lines = kinds(previous);
    let first_note = lines.iter().position(|(_, k)| *k == Kind::Note);
    let first_dated = lines.iter().position(|(l, _)| {
        let t = l.trim_start();
        t.starts_with("## ") && (t[3..].trim().get(..10).is_some_and(is_date) || t[3..].trim().starts_with("YYYY"))
    });
    let mut kept: Vec<String> = Vec::new();
    let mut at = None;
    for (i, (line, kind)) in lines.iter().enumerate() {
        if Some(i) == first_note || (first_note.is_none() && Some(i) == first_dated) {
            at = Some(kept.len());
        }
        if *kind == Kind::Other {
            kept.push((*line).to_owned());
        }
    }
    let at = at.unwrap_or(kept.len());
    let mut block: Vec<String> = notes.iter().map(line).collect();
    // A blank line between the notes and what is around them.
    if at > 0 && !kept[at - 1].trim().is_empty() {
        block.insert(0, String::new());
    }
    if at < kept.len() && !kept[at].trim().is_empty() {
        block.push(String::new());
    }
    if notes.is_empty() {
        block.clear();
    }
    kept.splice(at..at, block);
    // Removing notes leaves no gap wider than a blank line.
    let mut out: Vec<String> = Vec::new();
    for line in kept {
        if line.trim().is_empty() && out.last().is_none_or(|l: &String| l.trim().is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    if out.is_empty() { String::new() } else { format!("{}\n", out.join("\n")) }
}

/// Whether a word looks like a key or token: long, letters and digits mixed,
/// or a known key's start.
fn looks_like_key(text: &str) -> bool {
    text.split_whitespace().any(|w| {
        let w = w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-');
        let known = ["sk-", "sk_", "ghp_", "gho_", "xoxb-", "xoxp-", "AKIA", "AIza"].iter().any(|p| w.starts_with(p) && w.len() >= 16);
        let long = w.len() >= 24 && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') && w.chars().any(|c| c.is_ascii_digit()) && w.chars().any(|c| c.is_ascii_alphabetic());
        known || long
    })
}

/// A note as the person typed it, made fit to keep: one short line, no
/// bullet of its own, nothing secret.
pub fn validate(text: &str) -> Result<String, Invalid> {
    let text = text.trim();
    if text.contains(['\n', '\r']) {
        return Err(Invalid::Lines);
    }
    let text = text.strip_prefix("- ").or_else(|| text.strip_prefix("* ")).unwrap_or(text).trim();
    if text.is_empty() {
        return Err(Invalid::Empty);
    }
    if text.chars().count() > NOTE_MAX {
        return Err(Invalid::TooLong);
    }
    if crate::squad_memory::sensitive(text).is_some() || looks_like_key(text) {
        return Err(Invalid::Secret);
    }
    Ok(text.to_owned())
}

/// A note's words, as two notes are told apart: case and spacing aside.
fn same(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    norm(a) == norm(b)
}

/// `notes` with the person's change made. A new note is theirs, dated
/// `today`; a rewritten one keeps where it came from, confirmed.
pub fn change(notes: &[TasteNote], change: &TasteChange, today: &str) -> Result<Vec<TasteNote>, Invalid> {
    let mut out = notes.to_vec();
    let find = |words: &str| out.iter().position(|n| same(&n.text, words)).ok_or(Invalid::Missing);
    match change {
        TasteChange::Add { text } => {
            let text = validate(text)?;
            if out.iter().any(|n| same(&n.text, &text)) {
                return Err(Invalid::Already);
            }
            if out.len() >= MAX {
                return Err(Invalid::TooMany);
            }
            out.push(TasteNote { text, from: TasteFrom::Person, date: Some(today.to_owned()), confirmed: true, ..TasteNote::default() });
        }
        TasteChange::Edit { was, text } => {
            let at = find(was)?;
            let text = validate(text)?;
            if out.iter().enumerate().any(|(i, n)| i != at && same(&n.text, &text)) {
                return Err(Invalid::Already);
            }
            out[at].text = text;
            // Corrected by the person: theirs to stand behind now.
            out[at].confirmed = true;
        }
        TasteChange::Confirm { text } => {
            let at = find(text)?;
            out[at].confirmed = true;
        }
        TasteChange::Remove { text } => {
            let at = find(text)?;
            out.remove(at);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(text: &str, from: TasteFrom, date: Option<&str>) -> TasteNote {
        TasteNote { text: text.into(), from, date: date.map(str::to_owned), confirmed: from == TasteFrom::Person, ..TasteNote::default() }
    }

    #[test]
    fn a_note_says_where_it_came_from_and_a_hand_typed_one_is_the_persons() {
        let notes = [
            note("모서리는 둥글게", TasteFrom::Person, Some("2026-11-01")),
            note("채도 높은 파랑은 싫어함", TasteFrom::SentBack { ticket: 336 }, Some("2026-11-03")),
            note("여백은 넉넉하게", TasteFrom::Answered { ticket: 272 }, None),
            note("숫자는 고정폭", TasteFrom::Noticed { ticket: Some(301) }, Some("2026-11-04")),
            note("사진보다 일러스트", TasteFrom::Noticed { ticket: None }, None),
        ];
        let text = notes.iter().map(line).collect::<Vec<_>>().join("\n");
        assert!(text.starts_with("- 모서리는 둥글게 _(the person, 2026-11-01)_\n- 채도 높은 파랑은 싫어함 _(sent back on T-336, 2026-11-03)_\n"), "{text}");
        assert_eq!(parse(&text), notes);
        assert!(notes[1..].iter().all(|n| !n.confirmed), "the designer's wait for the person's yes");
        // A pattern: the other tickets it was seen on, and the person's yes.
        let pattern = TasteNote { seen: vec![272, 288], confirmed: true, ..note("숫자는 고정폭", TasteFrom::Noticed { ticket: Some(301) }, Some("2026-11-04")) };
        assert_eq!(line(&pattern), "- 숫자는 고정폭 _(noticed on T-301, 2026-11-04, seen on T-272 T-288, confirmed)_");
        assert_eq!(parse(&line(&pattern)), [pattern]);
        // By hand, in Finder: no source, the person's; a bullet `*` too; an unknown source is words.
        let typed = parse("* 그림자는 거의 없게\n- 둥근 버튼 _(my mood, 2026-11-01)_\n-   \n");
        assert_eq!(typed, [note("그림자는 거의 없게", TasteFrom::Person, None), note("둥근 버튼 _(my mood, 2026-11-01)_", TasteFrom::Person, None)]);
    }

    #[test]
    fn the_old_templates_dated_entries_and_code_are_not_notes() {
        let old = "# Taste\n\nWhat the person chose — read before every new direction.\n\n- 여백은 넉넉하게\n\n## YYYY-MM-DD · what was decided\n- Chose: (the option)\n\n## 2026-10-02 · Hero\n- Chose: B\n- In their words: \"airy\"\n\n## Notes\n```md\n- not a note\n```\n- 선은 얇게\n";
        let notes: Vec<String> = parse(old).into_iter().map(|n| n.text).collect();
        assert_eq!(notes, ["여백은 넉넉하게", "선은 얇게"]);
    }

    #[test]
    fn writing_notes_keeps_the_rest_of_the_file_where_it_was() {
        let file = "# Taste\n\nThe person's taste.\n\n- a _(the person)_\n- b _(the person)_\n\n## 2026-10-02 · Hero\n- Chose: B\n";
        let notes = vec![note("a", TasteFrom::Person, None), note("c", TasteFrom::Person, Some("2026-11-04"))];
        let written = write(file, &notes);
        assert_eq!(written, "# Taste\n\nThe person's taste.\n\n- a _(the person)_\n- c _(the person, 2026-11-04)_\n\n## 2026-10-02 · Hero\n- Chose: B\n");
        assert_eq!(parse(&written), notes);
        // None left: the file stays, without a gap.
        assert_eq!(write(file, &[]), "# Taste\n\nThe person's taste.\n\n## 2026-10-02 · Hero\n- Chose: B\n");
        // No note yet: before the first dated entry, else at the end.
        assert_eq!(write("# Taste\n\n## 2026-10-02 · Hero\n- Chose: B\n", &notes[..1]), "# Taste\n\n- a _(the person)_\n\n## 2026-10-02 · Hero\n- Chose: B\n");
        assert_eq!(write("# Taste\n\nIntro.\n", &notes[..1]), "# Taste\n\nIntro.\n\n- a _(the person)_\n");
        assert_eq!(write("", &notes[..1]), "- a _(the person)_\n");
    }

    #[test]
    fn a_note_is_one_short_line_with_nothing_secret_in_it() {
        assert_eq!(validate("  - 모서리는 둥글게 "), Ok("모서리는 둥글게".into()));
        assert_eq!(validate("   "), Err(Invalid::Empty));
        assert_eq!(validate("둥글게\n그리고 밝게"), Err(Invalid::Lines));
        assert_eq!(validate(&"가".repeat(NOTE_MAX + 1)), Err(Invalid::TooLong));
        assert!(validate(&"가".repeat(NOTE_MAX)).is_ok());
        for secret in ["카드 4111 1111 1111 1111", "키는 sk-proj-abcdef1234567890", "token ghp_16C7e42F292c6912E7710c838347Ae178B4a", "a3f9c2e8b7d6a5f4e3d2c1b0a9f8e7d6"] {
            assert_eq!(validate(secret), Err(Invalid::Secret), "{secret}");
        }
        for fine in ["#1D4ED8 같은 파랑은 피하기", "Pretendard Variable 우선", "radius 12px, 2026-11-03부터", "line-height 1.6"] {
            assert!(validate(fine).is_ok(), "{fine}");
        }
    }

    #[test]
    fn the_person_adds_rewrites_and_removes_notes_by_their_words() {
        let notes = vec![note("모서리는 둥글게", TasteFrom::SentBack { ticket: 336 }, Some("2026-11-03"))];
        let added = change(&notes, &TasteChange::Add { text: "여백은 넉넉하게".into() }, "2026-11-04").unwrap();
        assert_eq!(added[1], note("여백은 넉넉하게", TasteFrom::Person, Some("2026-11-04")));
        assert_eq!(change(&added, &TasteChange::Add { text: "  여백은   넉넉하게 ".into() }, "2026-11-04"), Err(Invalid::Already));
        let edited = change(&added, &TasteChange::Edit { was: "모서리는 둥글게".into(), text: "모서리는 둥글게, 12px".into() }, "2026-11-05").unwrap();
        assert_eq!(edited[0], TasteNote { confirmed: true, ..note("모서리는 둥글게, 12px", TasteFrom::SentBack { ticket: 336 }, Some("2026-11-03")) }, "where it came from stays; corrected, it is confirmed");
        assert!(!added[0].confirmed);
        let confirmed = change(&added, &TasteChange::Confirm { text: "모서리는 둥글게".into() }, "x").unwrap();
        assert!(confirmed[0].confirmed && confirmed[0].text == "모서리는 둥글게");
        assert_eq!(change(&added, &TasteChange::Confirm { text: "gone".into() }, "x"), Err(Invalid::Missing));
        assert_eq!(change(&edited, &TasteChange::Edit { was: "모서리는 둥글게, 12px".into(), text: "여백은 넉넉하게".into() }, "x"), Err(Invalid::Already));
        assert_eq!(change(&edited, &TasteChange::Edit { was: "gone".into(), text: "x".into() }, "x"), Err(Invalid::Missing));
        let removed = change(&edited, &TasteChange::Remove { text: "여백은 넉넉하게".into() }, "x").unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(change(&removed, &TasteChange::Remove { text: "여백은 넉넉하게".into() }, "x"), Err(Invalid::Missing));
        let full: Vec<TasteNote> = (0..MAX).map(|i| note(&format!("n{i}"), TasteFrom::Person, None)).collect();
        assert_eq!(change(&full, &TasteChange::Add { text: "one more".into() }, "x"), Err(Invalid::TooMany));
    }
}
