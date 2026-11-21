//! Whether a decision is the person's to make. A decision card costs the
//! person attention, and they lead the squad: a choice they can judge and
//! that matters to the product — what users see, how it behaves, scope, cost,
//! risk — is theirs. A choice they can't perceive in the product is the
//! member's: which punctuation a developer note in a component gallery ends
//! on was put to a person who couldn't tell the options apart, nine cards
//! deep. `ask_decision` turns away what can be told structurally — options
//! that differ only in punctuation or spacing, or a question only about the
//! squad's internal files — with why, and the member decides it itself and
//! notes the choice on its ticket. The rest is the members' charters'.

use equipe_protocol::decide::{Choice, Question};

/// Why a question isn't the person's to answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotTheirs {
    /// Options `a` and `b` (from 0) differ only in punctuation or spacing.
    Punctuation { a: usize, b: usize },
    /// It is only about these files, which nobody sees in the product.
    Internal { paths: Vec<String> },
}

impl NotTheirs {
    /// Said to the member, for question `n` (from 1).
    pub fn said(&self, n: usize) -> String {
        let why = match self {
            NotTheirs::Punctuation { a, b } => format!("its options {} and {} differ only in punctuation or spacing", a + 1, b + 1),
            NotTheirs::Internal { paths } => format!("it is only about {}, which nobody sees in the product", paths.join(", ")),
        };
        format!(
            "Question {n} isn't the person's to decide: {why}. Decision cards are for choices they can judge and that matter to the \
product — what users see, how it behaves, scope, cost, risk. Wording or punctuation in internal docs, developer notes and test \
or QA records is yours: choose, say what you chose and why in a comment on your ticket, and go on."
        )
    }
}

/// Whether `q` asks for a fact only the person knows — the shop's name, its address, real prices
/// or hours: an open question (no options to pick from, no pictures, no board) that names nothing
/// on the autonomy floor (money, legal, security…, which always go to the person as asked).
/// Where a squad runs on its own, such a question never blocks its member: it goes to the PM,
/// which asks the person once for all of them, and the member works on marked placeholders.
pub fn is_business_fact(q: &Question) -> bool {
    q.options.len() < 2 && !q.board && q.images.is_empty() && equipe_protocol::autonomy::Kind::floor_named(&q.question, None).is_none()
}

/// What a member is told when its business-fact question went to the PM instead of the person.
pub const FACT_TO_PM: &str = "Not put to the person by you: a fact only they know (a name, an address, real prices or hours) never holds up work. \
It went to the PM, who asks them once for everything the team needs, in one friendly card. Go on now with a clearly marked placeholder \
(`[SHOP ADDRESS]`, `[PRICE]` — one that can't be mistaken for the real thing, easy to search for), say on your ticket which placeholders you \
used, and don't end your turn or put your ticket down over it. When the answer comes, the PM tells you and you replace them.";

/// Whether question `q` is one to put to the person; `Some(why)` when not.
/// Only a choice with options in words is judged: pictures, a taste board
/// or an open question are always theirs.
pub fn judge(q: &Question) -> Option<NotTheirs> {
    if q.options.len() < 2 || q.board || !q.images.is_empty() || q.options.iter().any(|o| !o.images.is_empty() || !o.tiles.is_empty()) {
        return None;
    }
    let texts: Vec<String> = q.options.iter().map(option_text).collect();
    for a in 0..texts.len() {
        for b in a + 1..texts.len() {
            let bare_a = bare(&texts[a]);
            if !bare_a.is_empty() && bare_a == bare(&texts[b]) {
                return Some(NotTheirs::Punctuation { a, b });
            }
        }
    }
    let mut paths = Vec::new();
    for text in std::iter::once(q.question.as_str()).chain(texts.iter().map(String::as_str)) {
        for path in file_paths(text) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    (!paths.is_empty() && paths.iter().all(|p| internal(p))).then_some(NotTheirs::Internal { paths })
}

fn option_text(o: &Choice) -> String {
    format!("{} {}", o.label, o.description)
}

/// Only its letters and digits, lowercased: what is left without punctuation and spacing.
fn bare(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The repository paths `text` names: tokens with a `/` that aren't links.
fn file_paths(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | '“' | '”' | '‘' | '’'))
        .map(|t| t.trim_end_matches(['.', ':', '!', '?']))
        .filter(|t| t.contains('/') && !t.contains("://") && t.chars().any(char::is_alphabetic))
        .map(|t| t.trim_start_matches("./").to_owned())
        .collect()
}

/// Folders only the squad reads: docs, tests, and design QA's own records.
const INTERNAL_DIRS: [&str; 10] = ["docs/", "doc/", "tests/", "test/", "__tests__/", "design/qa/", "design/checks/", "design/critiques/", "design/decisions/", "design/notes/"];

/// Whether `path` is a file nobody sees in the product.
fn internal(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    INTERNAL_DIRS.iter().any(|d| lower.starts_with(d) || lower.contains(&format!("/{d}"))) || [".test.", ".spec.", "_test."].iter().any(|k| name.contains(k))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_fact_only_the_person_knows_is_an_open_question_off_the_floor() {
        let open = |text: &str| Question { question: text.into(), ..Default::default() };
        assert!(is_business_fact(&open("What is the shop's address?")));
        assert!(is_business_fact(&open("What are your opening hours?")));
        // Money is on the floor: it is asked as it is.
        assert!(!is_business_fact(&open("Can we pay for the domain with your card?")));
        // A choice between options is not a fact.
        let choice = Question { question: "Which address?".into(), options: vec![Choice { label: "A".into(), ..Default::default() }, Choice { label: "B".into(), ..Default::default() }], ..Default::default() };
        assert!(!is_business_fact(&choice));
    }

    use super::*;

    fn choice(label: &str, description: &str) -> Choice {
        serde_json::from_value(serde_json::json!({ "label": label, "description": description })).unwrap()
    }

    fn asked(question: &str, options: Vec<Choice>) -> Question {
        let mut q: Question = serde_json::from_value(serde_json::json!({ "question": question })).unwrap();
        q.options = options;
        q
    }

    #[test]
    fn options_that_differ_only_in_punctuation_are_the_members_to_choose() {
        let dot = asked("Which wording for the processing rule?", vec![choice("Middle dot", "aria-disabled·aria-busy together"), choice("Comma", "aria-disabled, aria-busy together")]);
        assert_eq!(judge(&dot), None, "labels in words differ");
        let bare = asked(
            "Which wording for the processing rule?",
            vec![choice("aria-disabled·aria-busy together", ""), choice("aria-disabled and aria-busy together", ""), choice("aria-disabled, aria-busy together", "")],
        );
        assert_eq!(judge(&bare), Some(NotTheirs::Punctuation { a: 0, b: 2 }));
        assert!(NotTheirs::Punctuation { a: 0, b: 2 }.said(1).contains("options 1 and 3 differ only in punctuation"));
        let spaced = asked("Which label?", vec![choice("Sign in", ""), choice("Sign-in", "")]);
        assert_eq!(judge(&spaced), Some(NotTheirs::Punctuation { a: 0, b: 1 }));
    }

    #[test]
    fn a_real_choice_stays_the_persons() {
        let columns = asked("How many columns on the pricing page?", vec![choice("Keep 3 columns", ""), choice("Keep 4 columns", "")]);
        assert_eq!(judge(&columns), None, "a digit is a difference");
        let db = asked("Which database?", vec![choice("Postgres", "managed"), choice("SQLite", "on the box")]);
        assert_eq!(judge(&db), None);
        let yes = asked("Ship it today?", vec![choice("Yes", ""), choice("No", "")]);
        assert_eq!(judge(&yes), None);
        let mut pictured = asked("Which label?", vec![choice("Sign in", ""), choice("Sign-in", "")]);
        pictured.options[0].images = vec!["/tmp/a.png".into()];
        assert_eq!(judge(&pictured), None, "a choice shown in pictures is the person's to see");
    }

    #[test]
    fn a_question_only_about_internal_files_is_the_members() {
        let notes = asked("Which heading should docs/gallery.md and design/qa/T-12.md use?", vec![choice("Usage rules", ""), choice("How to use", "")]);
        assert_eq!(judge(&notes), Some(NotTheirs::Internal { paths: vec!["docs/gallery.md".into(), "design/qa/T-12.md".into()] }));
        let test = asked("Rename the case in src/app.test.ts?", vec![choice("renders", ""), choice("shows", "")]);
        assert!(matches!(judge(&test), Some(NotTheirs::Internal { .. })));
        let product = asked("Which copy for src/pages/pricing.tsx, also in docs/pricing.md?", vec![choice("Start free", ""), choice("Try it", "")]);
        assert_eq!(judge(&product), None, "a product file is named too");
        let link = asked("Which provider, see https://example.com/docs/a?", vec![choice("A", ""), choice("B", "")]);
        assert_eq!(judge(&link), None, "a link isn't a file");
    }
}
