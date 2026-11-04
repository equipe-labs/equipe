//! The person's design taste, learned as it shows: each design decision they
//! answer (what they chose, what they passed over, their own words) and each
//! piece of design work they send back with a reason, written by equipe into
//! one fact of the squad's memory — never left to the designer to remember.
//! It had kept two of more than six decisions. Newest first, the oldest let
//! go past a few dozen. No runtime types here.
//!
//! What it comes to is kept apart, for the person to see and edit on the
//! design screen ("Taste & decisions"): the notes the designer follows (`notes`),
//! and the decisions made, each with why (`decisions`) — where both live
//! and how they are read and written is `store`'s.

pub mod decisions;
pub mod evidence;
pub mod notes;
pub mod store;

pub use store::{NOTES_NAME, record};

use crate::protocol::decide::{Answer, QuestionSet, Reaction};

/// The fact's name in the squad's memory, and its line in the index.
pub const NAME: &str = "design taste";
pub const DESCRIPTION: &str = "What the person chose and passed over in design, and why in their own words — read it whole before any design.";
/// Entries kept, newest first.
const KEEP: usize = 40;

/// What a set of answered design questions says of the person's taste, as
/// one entry; `None` when they picked nothing and said nothing.
pub fn decided(set: &QuestionSet, answers: &[Answer], today: &str) -> Option<String> {
    let mut lines = Vec::new();
    for (question, answer) in set.questions.iter().zip(answers) {
        let label = |i: &usize| question.options.get(*i).map(|o| o.label.trim().trim_end_matches("(Recommended)").trim().to_owned());
        let mut chose: Vec<String> = answer.picked.iter().filter_map(label).collect();
        // A mix: the one taken, with what of another; another round: none.
        match &answer.reaction {
            Some(Reaction::Mix { base, from, aspect }) => chose = label(base).zip(label(from)).map(|(b, f)| format!("{b} with the {aspect} of {f}")).into_iter().collect(),
            Some(Reaction::Again) => {
                let all: Vec<String> = (0..question.options.len()).filter_map(|i| label(&i)).collect();
                let said = answer.other.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| format!("; in their words: \u{201c}{s}\u{201d}")).unwrap_or_default();
                lines.push(format!("- {} → none of {}: asked for another round{said}", question.question.trim(), all.join(", ")));
                continue;
            }
            None => {}
        }
        let passed: Vec<String> = (0..question.options.len()).filter(|i| !answer.picked.contains(i)).filter_map(|i| label(&i)).collect();
        let said = answer.other.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if chose.is_empty() && said.is_none() {
            continue;
        }
        let recommended = question.options.first().is_some_and(|o| o.label.contains("(Recommended)"));
        let mut line = format!("- {}", question.question.trim());
        if !chose.is_empty() {
            line.push_str(&format!(" → chose {}", chose.join(", ")));
            if recommended && answer.picked == [0] {
                line.push_str(" (the recommendation)");
            }
        }
        if !passed.is_empty() && !chose.is_empty() {
            line.push_str(&format!("; passed over {}", passed.join(", ")));
        }
        if let Some(said) = said {
            line.push_str(&format!("; in their words: \u{201c}{said}\u{201d}"));
        }
        lines.push(line);
    }
    (!lines.is_empty()).then(|| format!("{}\n{}", head(today, set.ticket, &set.member), lines.join("\n")))
}

/// A piece of design work the person sent back, in their words.
pub fn sent_back(ticket: u32, title: &str, words: &str, today: &str) -> Option<String> {
    let words = words.trim();
    (!words.is_empty()).then(|| format!("{}\n- Sent back \u{201c}{}\u{201d}: \u{201c}{words}\u{201d}", head(today, Some(ticket), "designer"), title.trim()))
}

fn head(today: &str, ticket: Option<u32>, member: &str) -> String {
    match ticket {
        Some(t) => format!("## {today} · T-{t} ({member})"),
        None => format!("## {today} ({member})"),
    }
}

/// The fact's body with `entry` added first, the oldest let go past [`KEEP`].
pub fn with(previous: Option<&str>, entry: &str) -> String {
    let mut entries = vec![entry.trim().to_owned()];
    if let Some(previous) = previous {
        let text = format!("\n{}", previous.trim());
        entries.extend(text.split("\n## ").map(str::trim).filter(|e| !e.is_empty()).map(|e| if e.starts_with("## ") { e.to_owned() } else { format!("## {e}") }));
    }
    entries.truncate(KEEP);
    entries.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::decide::{Choice, Question};

    fn set() -> QuestionSet {
        let choice = |label: &str| Choice { label: label.into(), ..Default::default() };
        QuestionSet {
            id: 1,
            session: 1,
            member: "designer".into(),
            ticket: Some(272),
            questions: vec![
                Question { question: "Which hero layout?".into(), options: vec![choice("B: airy, one column (Recommended)"), choice("A: dense, two columns"), choice("C: full-bleed image")], ..Default::default() },
                Question { question: "Accent colour?".into(), options: vec![choice("Blue"), choice("Clay")], ..Default::default() },
                Question { question: "Anything else?".into(), ..Default::default() },
            ],
            asked_at: 0,
            review: false,
            retro: None,
        }
    }

    #[test]
    fn what_the_person_chose_passed_over_and_said_is_kept_as_their_taste() {
        let answers = vec![
            Answer { picked: vec![0], other: Some("좀 더 숨 쉬게".into()), ..Answer::default() },
            Answer::pick(1),
            Answer { other: Some("  ".into()), ..Answer::default() },
        ];
        let entry = decided(&set(), &answers, "2026-11-03").unwrap();
        assert!(entry.starts_with("## 2026-11-03 · T-272 (designer)"), "{entry}");
        assert!(entry.contains("Which hero layout? → chose B: airy, one column (the recommendation); passed over A: dense, two columns, C: full-bleed image; in their words: \u{201c}좀 더 숨 쉬게\u{201d}"), "{entry}");
        assert!(entry.contains("Accent colour? → chose Clay; passed over Blue"), "{entry}");
        assert!(!entry.contains("Anything else?"), "nothing said, nothing kept");
        assert_eq!(decided(&set(), &[Answer::default(), Answer::default(), Answer::default()], "2026-11-03"), None);
    }

    #[test]
    fn a_mix_and_another_round_are_kept_as_what_they_were() {
        let answers = vec![Answer::mix(1, 2, "type"), Answer::again(Some("너무 차가워".into())), Answer::default()];
        let entry = decided(&set(), &answers, "2026-11-04").unwrap();
        assert!(entry.contains("Which hero layout? → chose A: dense, two columns with the type of C: full-bleed image; passed over B: airy, one column, C: full-bleed image"), "{entry}");
        assert!(entry.contains("Accent colour? → none of Blue, Clay: asked for another round; in their words: \u{201c}너무 차가워\u{201d}"), "{entry}");
    }

    #[test]
    fn a_send_back_is_kept_in_their_words_and_the_newest_come_first() {
        let back = sent_back(336, "Pricing page", "못생겼어. 너무 답답해", "2026-11-03").unwrap();
        assert!(back.contains("Sent back \u{201c}Pricing page\u{201d}: \u{201c}못생겼어. 너무 답답해\u{201d}"), "{back}");
        assert_eq!(sent_back(336, "x", " ", "2026-11-03"), None);
        let body = with(None, "## 2026-11-01 · T-1 (designer)\n- a");
        let body = with(Some(&body), "## 2026-11-03 · T-2 (designer)\n- b");
        assert!(body.starts_with("## 2026-11-03 · T-2"), "{body}");
        assert!(body.contains("\n\n## 2026-11-01 · T-1 (designer)\n- a"), "{body}");
        let mut many = String::new();
        for i in 0..50 {
            many = with(Some(&many), &format!("## day {i}\n- x"));
        }
        assert_eq!(many.matches("## day").count(), KEEP);
        assert!(many.starts_with("## day 49"));
    }
}
