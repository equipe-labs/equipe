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
//! A design choice's options also name their aspects (type, colour, shape),
//! so the person can react in one click: this one, this one with one aspect
//! of another (a mix), or another round.

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
    /// What of it can be taken into another option — "type", "colour",
    /// "shape" — in the person's language; left out when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aspects: Vec<String>,
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
    /// The designer's weekly design review's proposal: screens show it as
    /// the review. Left out when not (and from an older daemon).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub review: bool,
}

impl QuestionSet {
    /// Every picture the set shows, question by question.
    pub fn images(&self) -> impl Iterator<Item = &str> {
        self.questions.iter().flat_map(Question::images)
    }
}

/// The person's answer to one question: the options they picked (by index),
/// and words of their own. Neither: skipped — the member's call. A design
/// choice may be answered with a reaction beyond a plain pick.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub picked: Vec<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
    /// A mix, or another round; left out for a plain pick (and from an
    /// older screen).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaction: Option<Reaction>,
}

/// How the person reacted to a design choice's options, beyond a pick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reaction {
    /// Option `base`, with option `from`'s `aspect` (one of its `aspects`).
    Mix { base: usize, from: usize, aspect: String },
    /// None of these: another round, why in the answer's own words.
    Again,
}

impl Answer {
    /// Nothing picked, nothing said.
    pub fn skipped() -> Self {
        Self::default()
    }

    /// Option `n`, as it is.
    pub fn pick(n: usize) -> Self {
        Self { picked: vec![n], ..Self::default() }
    }

    /// Option `base` with option `from`'s `aspect`: `base` is what is
    /// picked, so a reader that knows no mixes still sees the choice.
    pub fn mix(base: usize, from: usize, aspect: impl Into<String>) -> Self {
        Self { picked: vec![base], other: None, reaction: Some(Reaction::Mix { base, from, aspect: aspect.into() }) }
    }

    /// None of these: another round, with why if they said.
    pub fn again(why: Option<String>) -> Self {
        Self { picked: Vec::new(), other: why.map(|w| w.trim().to_owned()).filter(|w| !w.is_empty()), reaction: Some(Reaction::Again) }
    }

    pub fn is_skipped(&self) -> bool {
        self.reaction.is_none() && self.picked.is_empty() && self.other.as_deref().is_none_or(|o| o.trim().is_empty())
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
/// What a mix is written as: `mix — «A» with the type of «B»`.
const MIX: &str = "mix — «";
const MIX_WITH: &str = "» with the ";
const MIX_OF: &str = " of «";
/// What another round is written as; why beside it, as a note.
const AGAIN: &str = "another round — none of these";
/// What the member is told to do with a mix, and with another round.
const MIX_THEN: &str = "For a mix: make it, and ask again with it beside the two it came from.";
const AGAIN_THEN: &str = "For another round: bring options genuinely different from these, minding why.";
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
///
/// A design choice reacted to says so, and the member what to do next:
///
/// ```text
/// 1) Which logo? → mix — «Blue dot» with the type of «Grey dot»
/// 2) Which palette? → another round — none of these (+ note: too cold)
/// For a mix: make it, and ask again with it beside the two it came from.
/// For another round: …
/// ```
pub fn answer_message(set: &QuestionSet, answers: &[Answer]) -> String {
    let many = set.questions.len() > 1;
    let mut out = format!("{ANSWERS_HEAD}{}:", if many { "s" } else { "" });
    let (mut mixed, mut again) = (false, false);
    for (n, q) in set.questions.iter().enumerate() {
        let answer = answers.get(n).cloned().unwrap_or_default();
        let labels: Vec<String> = answer.picked.iter().filter_map(|&i| q.options.get(i)).map(|c| one_line(&c.label)).collect();
        let other = answer.other.as_deref().map(one_line).filter(|o| !o.is_empty());
        let mix = match &answer.reaction {
            Some(Reaction::Mix { base, from, aspect }) => q.options.get(*base).zip(q.options.get(*from)).map(|(b, f)| format!("{MIX}{}{MIX_WITH}{}{MIX_OF}{}»", one_line(&b.label), one_line(aspect), one_line(&f.label))),
            _ => None,
        };
        let said = match (answer.reaction == Some(Reaction::Again), mix, other) {
            (true, _, None) => AGAIN.to_owned(),
            (true, _, Some(why)) => format!("{AGAIN}{NOTE}{why})"),
            (false, Some(mix), None) => mix,
            (false, Some(mix), Some(other)) => format!("{mix}{NOTE}{other})"),
            (false, None, None) if labels.is_empty() => SKIPPED.to_owned(),
            (false, None, Some(other)) if labels.is_empty() => format!("{OWN}{other}"),
            (false, None, None) => labels.join(", "),
            (false, None, Some(other)) => format!("{}{NOTE}{other})", labels.join(", ")),
        };
        mixed |= said.starts_with(MIX);
        again |= said.starts_with(AGAIN);
        out.push_str(&format!("\n{}) {} → {said}", n + 1, one_line(&q.question)));
    }
    for (then, says) in [(mixed, MIX_THEN), (again, AGAIN_THEN)] {
        if then {
            out.push_str(&format!("\n{says}"));
        }
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
    /// A mix: the option taken, the one an aspect came from, and the aspect.
    pub mix: Option<Mixed>,
    /// Another round asked for (why in `note`).
    pub again: bool,
}

/// A mix, read back: the two labels and the aspect, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mixed {
    pub base: String,
    pub from: String,
    pub aspect: String,
}

/// `mix — «A» with the type of «B»`, read back.
fn read_mix(said: &str) -> Option<Mixed> {
    let rest = said.strip_prefix(MIX)?.strip_suffix('»')?;
    let (base, rest) = rest.split_once(MIX_WITH)?;
    let (aspect, from) = rest.split_once(MIX_OF)?;
    Some(Mixed { base: base.to_owned(), from: from.to_owned(), aspect: aspect.to_owned() })
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
        let (mix, again) = (read_mix(&answer), answer == AGAIN);
        out.push(Read { question: question.to_owned(), answer, note, skipped, mix, again });
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
            review: false,
        }
    }

    #[test]
    fn the_answers_go_back_in_one_message_a_question_a_line() {
        let answers = [
            Answer::pick(0),
            Answer { picked: vec![0, 1], other: Some("SSO\nlater".into()), ..Default::default() },
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
        let message = answer_message(&one, &[Answer { other: Some("MySQL, we have one".into()), ..Default::default() }]);
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
        assert!(Answer { other: Some("  ".into()), ..Default::default() }.is_skipped());
        assert!(!Answer::pick(1).is_skipped());
        // On the wire: what is there, nothing more.
        assert_eq!(serde_json::to_value(Answer::skipped()).unwrap(), json!({}));
        assert_eq!(serde_json::to_value(Answer { picked: vec![1], other: Some("x".into()), ..Default::default() }).unwrap(), json!({ "picked": [1], "other": "x" }));
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
        assert!(!bare.review && wire_of(&s).get("review").is_none(), "not the design review: left out");
        let review = QuestionSet { review: true, ..set() };
        assert_eq!(wire_of(&review)["review"], true);
        assert_eq!(serde_json::from_value::<QuestionSet>(wire_of(&review)).unwrap(), review);
    }

    fn wire_of(set: &QuestionSet) -> serde_json::Value {
        serde_json::to_value(set).unwrap()
    }

    /// The designer's question: three screens, each with what can be mixed.
    fn design() -> QuestionSet {
        let option = |label: &str| Choice { label: label.into(), images: vec![format!("/w/{label}.png")], aspects: vec!["type".into(), "colour".into()], ..Default::default() };
        let mut s = set();
        s.questions = vec![Question { question: "Which direction?".into(), options: vec![option("Calm (Recommended)"), option("Bold"), option("Warm")], ..Default::default() }];
        s
    }

    #[test]
    fn a_mix_says_which_option_takes_what_from_which_and_reads_back() {
        let message = answer_message(&design(), &[Answer::mix(0, 2, "type")]);
        assert_eq!(
            message,
            "Answers to your question:\n\
             1) Which direction? → mix — «Calm (Recommended)» with the type of «Warm»\n\
             For a mix: make it, and ask again with it beside the two it came from."
        );
        let read = read_answers(&message).unwrap();
        assert_eq!(read.len(), 1, "the line on what to do isn't an answer");
        assert_eq!(read[0].mix, Some(Mixed { base: "Calm (Recommended)".into(), from: "Warm".into(), aspect: "type".into() }));
        assert!(!read[0].again && !read[0].skipped);
        // With words beside it.
        let mut with_words = Answer::mix(1, 0, "colour");
        with_words.other = Some("a bit softer".into());
        let read = read_answers(&answer_message(&design(), &[with_words])).unwrap();
        assert_eq!((read[0].mix.as_ref().map(|m| m.from.as_str()), read[0].note.as_deref()), (Some("Calm (Recommended)"), Some("a bit softer")));
    }

    #[test]
    fn another_round_says_none_of_these_and_why_and_reads_back() {
        let message = answer_message(&design(), &[Answer::again(Some(" too\ncold ".into()))]);
        assert_eq!(
            message,
            "Answers to your question:\n\
             1) Which direction? → another round — none of these (+ note: too cold)\n\
             For another round: bring options genuinely different from these, minding why."
        );
        let read = read_answers(&message).unwrap();
        assert!(read[0].again && read[0].mix.is_none() && !read[0].skipped);
        assert_eq!(read[0].note.as_deref(), Some("too cold"));
        // Without why; a plain pick says nothing more.
        let read = read_answers(&answer_message(&design(), &[Answer::again(Some("  ".into()))])).unwrap();
        assert!(read[0].again && read[0].note.is_none());
        assert_eq!(answer_message(&design(), &[Answer::pick(1)]), "Answers to your question:\n1) Which direction? → Bold");
    }

    #[test]
    fn a_reaction_is_not_a_skip_and_the_wire_is_as_before_without_one() {
        assert!(!Answer::again(None).is_skipped());
        assert_eq!(serde_json::to_value(Answer::pick(2)).unwrap(), json!({ "picked": [2] }));
        assert_eq!(serde_json::to_value(Answer::mix(0, 2, "type")).unwrap(), json!({ "picked": [0], "reaction": { "mix": { "base": 0, "from": 2, "aspect": "type" } } }));
        assert_eq!(serde_json::to_value(Answer::again(None)).unwrap(), json!({ "reaction": "again" }));
        assert_eq!(serde_json::from_value::<Answer>(json!({ "picked": [0], "reaction": { "mix": { "base": 0, "from": 2, "aspect": "type" } } })).unwrap(), Answer::mix(0, 2, "type"));
        // The options' aspects: on the wire when there are any.
        let wire = serde_json::to_value(design()).unwrap();
        assert_eq!(wire["questions"][0]["options"][0]["aspects"], json!(["type", "colour"]));
        assert!(serde_json::to_value(set()).unwrap()["questions"][0]["options"][0].get("aspects").is_none());
        assert_eq!(serde_json::from_value::<QuestionSet>(wire).unwrap(), design());
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
