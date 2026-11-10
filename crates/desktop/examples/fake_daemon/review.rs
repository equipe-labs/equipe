//! `FAKE_DESIGN_REVIEW=1`: the designer's weekly design review, come in —
//! its routine's message and the review it brought in the designer's
//! conversation, and its proposal waiting in the inbox as a visual question
//! (the pictures `fake_daemon/pictures.rs` draws).

use equipe_core::protocol::decide::{Choice, Question, QuestionSet};
use equipe_core::protocol::{Entry, ToolOutcome};

use super::pictures::at;

pub fn on() -> bool {
    std::env::var("FAKE_DESIGN_REVIEW").is_ok_and(|v| v == "1")
}

/// The review's proposal: the sign-in screen's two buttons, put as one
/// click — what is there now over the options, each its picture.
pub fn set(now: u64) -> QuestionSet {
    let option = |label: &str, description: &str, image: &str| Choice { label: label.into(), description: description.into(), images: vec![at(image)], ..Choice::default() };
    QuestionSet {
        id: 9005,
        session: 4,
        member: "designer".into(),
        ticket: None,
        questions: vec![Question {
            question: "Shall I tidy the sign-in screen's button hierarchy like this?".into(),
            header: "Button hierarchy".into(),
            images: vec![at("case-now.png")],
            options: vec![
                option("One primary button (Recommended)", "Google sign-in as a secondary button — one thing to do at first glance", "case-wide.png"),
                option("Both primary, as now", "Left as it is", "case-square.png"),
                option("Bolder: the whole card is the button", "A step past the recommendation — the touch target is half the screen", "case-circle-square.png"),
            ],
            ..Question::default()
        }],
        asked_at: now.saturating_sub(120),
        review: true,
        retro: None,
    }
}

/// The designer's conversation: the routine's message, the checks it ran,
/// and the review — its heading, the three things, the critic's word.
pub fn history() -> Vec<Entry> {
    let routine = equipe_core::design::review::routine(crate::lang(), 2);
    let ran = |target: &str| Entry::Tool { tool: "Bash".into(), target: Some(target.into()), outcome: Some(Box::new(ToolOutcome { ok: true, ..ToolOutcome::default() })), label: None, background: false };
    let review = format!(
        "## Weekly design review\n\n\
### 1. What drifted — 3 things\n\
1. **Sign-in › Error** — no dark-mode picture. Only the light one.\n\
2. **Orders › List › Empty** — the picture is out of date since last week's code change.\n\
3. **Settings › Notifications** — the toggle's colour is a hard-coded value (#3B82F6), not `color.action.primary`.\n\n\
![Sign-in › Error (light only)]({})\n\n\
### 2. A proposal to tidy — the sign-in buttons' hierarchy\n\
Today the email and Google buttons weigh the same, so there are two things to do at first glance. I drew one primary button over the real screen. Pick on the card below.\n\n\
> Critic (Codex): the recommendation's hierarchy is clear, but the secondary button's contrast is 3.9:1, just under AA — raising its border to `color.border.strong` fixes it.\n\n\
### 3. References worth a look\n\
- [Linear sign-in](https://linear.app/login) — one primary button, the rest stepped back to text links\n\
- [Stripe dashboard empty state](https://dashboard.stripe.com) — an empty state with one sentence on the one next thing to do\n\
- [GOV.UK Design System](https://design-system.service.gov.uk) — the spacing between a toggle and its description, and plain wording\n\n\
I won't change anything until you pick.",
        at("case-now.png")
    );
    vec![
        Entry::User { text: equipe_core::routines::message(&routine), kept: Vec::new() },
        ran("equipe design flows check"),
        ran("equipe design critique design/review/sign-in-a.png --reference design/references/linear/login.png"),
        Entry::Reply { text: review },
    ]
}
