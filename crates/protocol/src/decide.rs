//! A decision a member puts to the person (`ask_decision`): one to four
//! questions, each with two to four options to pick from (or several, where
//! it says so), and words of their own for anything the options miss — or
//! with no options at all, a plain question answered in their own words (a
//! phone number, an email). It doesn't hold the member up: the set waits
//! for the person, and their answers go back to the member as their next
//! message — the one message [`answer_message`] writes, which every screen
//! reads back ([`read_answers`]) to show it as "decided". A visual choice
//! carries its pictures: paths on the daemon's machine, in the member's
//! worktree or the squad's folder, which screens ask for (`read_image`).

use serde::{Deserialize, Serialize};

use crate::{SessionId, TicketId};

/// One option of a question: a short label, a line on what it means, and —
/// a visual choice — the pictures of it (renders, mockups).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Its pictures, by their paths on the daemon's machine; left out when
    /// none (and absent from an older daemon).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

/// One question of a set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// The question, in a sentence.
    pub question: String,
    /// A word or two for it: "Database", "Scope".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub header: String,
    /// What to pick from; none: answered in the person's own words only.
    #[serde(default)]
    pub options: Vec<Choice>,
    /// The person may pick several options, not just one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multi_select: bool,
    /// Pictures of the question as a whole (what is there now, say), by
    /// their paths on the daemon's machine; left out when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

impl Question {
    /// Whether any option is shown as a picture: then the options are a
    /// grid of pictures rather than a list.
    pub fn is_visual(&self) -> bool {
        self.options.iter().any(|c| !c.images.is_empty())
    }

    /// Its own pictures, then each option's, in order.
    pub fn images(&self) -> impl Iterator<Item = &str> {
        self.images.iter().chain(self.options.iter().flat_map(|c| &c.images)).map(String::as_str)
    }
}

/// A set of questions waiting for the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionSet {
    pub id: u64,
    /// The conversation that asked, where the answers go.
    pub session: SessionId,
    pub member: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    pub questions: Vec<Question>,
    /// When it was asked, in unix seconds.
    #[serde(default)]
    pub asked_at: u64,
}

impl QuestionSet {
    /// Every picture the set shows, question by question.
    pub fn images(&self) -> impl Iterator<Item = &str> {
        self.questions.iter().flat_map(Question::images)
    }
}

/// The person's answer to one question: the options they picked (by index),
/// and words of their own. Neither: skipped — the member's call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub picked: Vec<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
}

impl Answer {
    /// Nothing picked, nothing said.
    pub fn skipped() -> Self {
        Self::default()
    }

    pub fn is_skipped(&self) -> bool {
        self.picked.is_empty() && self.other.as_deref().is_none_or(|o| o.trim().is_empty())
    }
}

/// How many questions a set may have, and options a question that has any.
pub const QUESTIONS: std::ops::RangeInclusive<usize> = 1..=4;
pub const OPTIONS: std::ops::RangeInclusive<usize> = 2..=4;

/// The first line of the message the answers go back in.
pub const ANSWERS_HEAD: &str = "Answers to your question";
/// What a skipped question says.
pub const SKIPPED: &str = "skipped — your call";
/// What the person's own words are marked with when nothing was picked.
const OWN: &str = "their own answer: ";
const NOTE: &str = " (+ note: ";
/// The first words of the message a set the person closed goes back in.
pub const DISMISSED_HEAD: &str = "The person closed your question without answering";

/// On one line: a question or an answer is written in one.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The one message the person's answers go to the member in:
///
/// ```text
/// Answers to your questions:
/// 1) Which database? → Postgres (Recommended)
/// 2) Which features? → Auth, Payments (+ note: SSO later)
/// 3) Where to deploy? → skipped — your call
/// ```
pub fn answer_message(set: &QuestionSet, answers: &[Answer]) -> String {
    let many = set.questions.len() > 1;
    let mut out = format!("{ANSWERS_HEAD}{}:", if many { "s" } else { "" });
    for (n, q) in set.questions.iter().enumerate() {
        let answer = answers.get(n).cloned().unwrap_or_default();
        let labels: Vec<String> = answer.picked.iter().filter_map(|&i| q.options.get(i)).map(|c| one_line(&c.label)).collect();
        let other = answer.other.as_deref().map(one_line).filter(|o| !o.is_empty());
        let said = match (labels.is_empty(), other) {
            (true, None) => SKIPPED.to_owned(),
            (true, Some(other)) => format!("{OWN}{other}"),
            (false, None) => labels.join(", "),
            (false, Some(other)) => format!("{}{NOTE}{other})", labels.join(", ")),
        };
        out.push_str(&format!("\n{}) {} → {said}", n + 1, one_line(&q.question)));
    }
    out
}

/// The message a set the person closed without answering goes back in.
pub fn dismissed_message(set: &QuestionSet) -> String {
    let asked: Vec<String> = set.questions.iter().map(|q| one_line(&q.question)).collect();
    format!(
        "{DISMISSED_HEAD} ({}). Don't wait on it: go on with your best judgement and say what you chose, or ask again in your own words.",
        asked.join(" / ")
    )
}

/// One answered question, as a screen shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub question: String,
    /// What was picked, or said; empty when skipped.
    pub answer: String,
    /// Their words beside what they picked.
    pub note: Option<String>,
    pub skipped: bool,
}

/// The answers a message of [`answer_message`]'s carries, question by
/// question; `None` when it isn't one.
pub fn read_answers(message: &str) -> Option<Vec<Read>> {
    let mut lines = message.lines();
    let head = lines.next()?.trim();
    if !(head == format!("{ANSWERS_HEAD}:") || head == format!("{ANSWERS_HEAD}s:")) {
        return None;
    }
    let mut out = Vec::new();
    for line in lines {
        let digits = line.chars().take_while(char::is_ascii_digit).count();
        let Some(rest) = line[digits..].strip_prefix(") ").filter(|_| digits > 0) else { continue };
        let Some((question, said)) = rest.split_once(" → ") else { continue };
        let skipped = said == SKIPPED;
        let (answer, note) = match said.strip_suffix(')').and_then(|s| s.split_once(NOTE)) {
            Some((picked, note)) => (picked.to_owned(), Some(note.to_owned())),
            None if skipped => (String::new(), None),
            None => (said.strip_prefix(OWN).unwrap_or(said).to_owned(), None),
        };
        out.push(Read { question: question.to_owned(), answer, note, skipped });
    }
    (!out.is_empty()).then_some(out)
}

/// Whether a message is the one a closed set went back in.
pub fn is_dismissed(message: &str) -> bool {
    message.starts_with(DISMISSED_HEAD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set() -> QuestionSet {
        let choice = |label: &str, description: &str| Choice { label: label.into(), description: description.into(), ..Default::default() };
        QuestionSet {
            id: 7,
            session: 42,
            member: "backend".into(),
            ticket: Some(12),
            questions: vec![
                Question {
                    question: "Which database should the API use?".into(),
                    header: "Database".into(),
                    options: vec![choice("Postgres (Recommended)", "Relational, runs on Supabase"), choice("SQLite", "One file, no server")],
                    multi_select: false,
                    ..Default::default()
                },
                Question {
                    question: "Which features ship\nfirst?".into(),
                    header: "Scope".into(),
                    options: vec![choice("Auth", ""), choice("Payments", ""), choice("Search", "")],
                    multi_select: true,
                    ..Default::default()
                },
                Question { question: "Where to deploy?".into(), header: String::new(), options: vec![choice("Vercel", ""), choice("Fly", "")], multi_select: false, ..Default::default() },
            ],
            asked_at: 1_760_000_000,
        }
    }

    #[test]
    fn the_answers_go_back_in_one_message_a_question_a_line() {
        let answers = [
            Answer { picked: vec![0], other: None },
            Answer { picked: vec![0, 1], other: Some("SSO\nlater".into()) },
            Answer::skipped(),
        ];
        let message = answer_message(&set(), &answers);
        assert_eq!(
            message,
            "Answers to your questions:\n\
             1) Which database should the API use? → Postgres (Recommended)\n\
             2) Which features ship first? → Auth, Payments (+ note: SSO later)\n\
             3) Where to deploy? → skipped — your call"
        );
        let read = read_answers(&message).unwrap();
        assert_eq!(read.len(), 3);
        assert_eq!((read[0].question.as_str(), read[0].answer.as_str()), ("Which database should the API use?", "Postgres (Recommended)"));
        assert_eq!((read[1].answer.as_str(), read[1].note.as_deref()), ("Auth, Payments", Some("SSO later")));
        assert!(read[2].skipped && read[2].answer.is_empty());
        // Only their own words; one question.
        let mut one = set();
        one.questions.truncate(1);
        let message = answer_message(&one, &[Answer { picked: vec![], other: Some("MySQL, we have one".into()) }]);
        assert_eq!(message, "Answers to your question:\n1) Which database should the API use? → their own answer: MySQL, we have one");
        assert_eq!(read_answers(&message).unwrap()[0].answer, "MySQL, we have one");
        assert_eq!(read_answers("hello"), None);
        assert_eq!(read_answers("Answers to your question:\nnothing numbered"), None);
    }

    #[test]
    fn a_set_closed_unanswered_says_so_and_what_it_was() {
        let message = dismissed_message(&set());
        assert!(is_dismissed(&message) && message.contains("Which database should the API use? / Which features ship first?"), "{message}");
        assert!(!is_dismissed("Answers to your question:"));
    }

    #[test]
    fn a_skipped_answer_is_nothing_picked_and_nothing_said() {
        assert!(Answer::skipped().is_skipped());
        assert!(Answer { picked: vec![], other: Some("  ".into()) }.is_skipped());
        assert!(!Answer { picked: vec![1], other: None }.is_skipped());
        // On the wire: what is there, nothing more.
        assert_eq!(serde_json::to_value(Answer::skipped()).unwrap(), json!({}));
        assert_eq!(serde_json::to_value(Answer { picked: vec![1], other: Some("x".into()) }).unwrap(), json!({ "picked": [1], "other": "x" }));
        assert_eq!(serde_json::from_value::<Answer>(json!({ "picked": [0, 2] })).unwrap().picked, [0, 2]);
    }

    #[test]
    fn a_set_round_trips_and_leaves_out_what_it_lacks() {
        let s = set();
        let wire = serde_json::to_value(&s).unwrap();
        assert_eq!(wire["questions"][1]["multi_select"], true);
        assert!(wire["questions"][0].get("multi_select").is_none());
        assert!(wire["questions"][1]["options"][0].get("description").is_none());
        assert!(wire["questions"][2].get("header").is_none());
        assert_eq!(serde_json::from_value::<QuestionSet>(wire).unwrap(), s);
        let bare: QuestionSet = serde_json::from_value(json!({ "id": 1, "session": 2, "member": "pm", "questions": [{ "question": "?", "options": [{ "label": "a" }, { "label": "b" }] }] })).unwrap();
        assert_eq!((bare.ticket, bare.asked_at, bare.questions[0].multi_select), (None, 0, false));
        assert!(bare.images().next().is_none() && !bare.questions[0].is_visual(), "an older daemon's set: no pictures");
    }

    #[test]
    fn a_visual_choice_carries_its_pictures_and_a_plain_one_the_wire_as_before() {
        let mut s = set();
        assert!(serde_json::to_value(&s).unwrap()["questions"][0].get("images").is_none());
        assert!(serde_json::to_value(&s).unwrap()["questions"][0]["options"][0].get("images").is_none());
        s.questions[0].images = vec!["/w/t-144/now.png".into()];
        s.questions[0].options[0].images = vec!["/w/t-144/a.png".into(), "/w/t-144/a-dark.png".into()];
        s.questions[0].options[1].images = vec!["/w/t-144/b.svg".into()];
        let wire = serde_json::to_value(&s).unwrap();
        assert_eq!(wire["questions"][0]["images"], json!(["/w/t-144/now.png"]));
        assert_eq!(wire["questions"][0]["options"][0]["images"], json!(["/w/t-144/a.png", "/w/t-144/a-dark.png"]));
        assert_eq!(serde_json::from_value::<QuestionSet>(wire).unwrap(), s);
        assert!(s.questions[0].is_visual() && !s.questions[1].is_visual(), "a question's own picture alone doesn't make its options a grid");
        assert_eq!(s.images().collect::<Vec<_>>(), ["/w/t-144/now.png", "/w/t-144/a.png", "/w/t-144/a-dark.png", "/w/t-144/b.svg"]);
    }
}
