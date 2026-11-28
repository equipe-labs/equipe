//! A member's word to the person, shown in the PM's conversation: the person
//! talks to the whole team in one place. The member says it with `tell_person`
//! (a plain reply stays in its own conversation); it is mirrored into the PM's
//! transcript as a [`Event::MemberNote`] — a line for screens, never a turn
//! for the PM's engine, and nothing waiting on the person, so it neither
//! escalates nor notifies on its own. The PM's journal keeps it, so a screen
//! that attaches later replays it.

use super::*;

/// The most characters of a note: it is a word to the person, not a report.
pub(super) const TEXT_MAX: usize = 1200;
/// The most characters of a choice's label.
const CHOICE_MAX: usize = 40;

/// The note's text, trimmed and cut to [`TEXT_MAX`] at a character.
pub(super) fn clean_text(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= TEXT_MAX {
        return text.to_owned();
    }
    let cut: String = text.chars().take(TEXT_MAX).collect();
    format!("{}…", cut.trim_end())
}

/// The note's choices: 2 to 4 distinct short labels, else none (a lone
/// option is no choice).
pub(super) fn clean_choices(args: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for label in board::arg_strs(args, "choices") {
        let label: String = label.chars().take(CHOICE_MAX).collect();
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&label)) {
            out.push(label);
        }
    }
    out.truncate(4);
    if out.len() < 2 {
        out.clear();
    }
    out
}

impl Runtime {
    /// `tell_person`: mirror the member's word into `squad`'s PM conversation.
    pub(super) fn tell_person(&mut self, squad: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let text = board::arg_str(args, "text").ok_or("Say something: give `text`.")?;
        let text = clean_text(text);
        let choices = clean_choices(args);
        // Shown as it is, with no PM between: in the person's language or not at all (inhwadoen,
        // 2026-11-28: "I made the recording smaller so it plays more easily…" reached a Korean founder).
        if let Some(want) = self.squad_language(squad)
            && std::iter::once(text.as_str()).chain(choices.iter().map(String::as_str)).any(|t| super::reply_guard::wrong_language(t, want))
        {
            return Err(format!(
                "Not shown: the person reads {lang}, and this isn't all in {lang}. Write it again in {lang} — every line and choice — in plain words.",
                lang = want.english_name()
            ));
        }
        let session = self.sessions.get(&id).ok_or("You're not on the squad.")?;
        let from = session.member.clone().ok_or("Only a member of the squad speaks to the person.")?;
        let ticket = session.work.as_ref().map(|w| w.ticket);
        let pm = self.pm_of(squad).ok_or("The squad has no PM conversation to show it in yet.")?;
        self.publish(Event::MemberNote { session: pm, from, text, choices: choices.clone(), ticket, at: now_secs(), source: id });
        Ok(if choices.is_empty() {
            "Shown to the person in the team conversation. Carry on.".into()
        } else {
            "Shown to the person with those choices; their tap comes to you as their next message. Carry on with what doesn't depend on it, or end your turn.".into()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_note_is_cut_at_a_character_not_a_byte() {
        let long = "가".repeat(TEXT_MAX + 50);
        let cut = clean_text(&long);
        assert_eq!(cut.chars().count(), TEXT_MAX + 1);
        assert!(cut.ends_with('…'));
        assert_eq!(clean_text("  hi  "), "hi");
    }

    #[test]
    fn choices_are_two_to_four_distinct_labels() {
        assert_eq!(clean_choices(&json!({ "choices": ["A", "B"] })), ["A", "B"]);
        assert_eq!(clean_choices(&json!({ "choices": ["A", "a", "B", "C", "D", "E"] })), ["A", "B", "C", "D"]);
        assert!(clean_choices(&json!({ "choices": ["Only"] })).is_empty());
        assert!(clean_choices(&json!({})).is_empty());
        assert_eq!(clean_choices(&json!({ "choices": ["x".repeat(90), "y"] }))[0].chars().count(), CHOICE_MAX);
    }
}
