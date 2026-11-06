//! `FAKE_RETRO=1`: the PM's weekly retro, come in — last week's and this
//! week's runs in the PM's conversation (the facts as the daemon gathers
//! them, and the retro the PM wrote from them), and this week's changes
//! waiting in Needs you as one card, each with the lesson behind it.

use equipe_core::protocol::decide::{Choice, Question, QuestionSet, Retro};
use equipe_core::protocol::{Entry, Ticket, TicketEntry, TicketKind, TicketStatus};
use equipe_core::retro::{self, Turn, Wait, Week};

const DAY: u64 = 24 * 60 * 60;
const HOUR: u64 = 60 * 60;

pub fn on() -> bool {
    std::env::var("FAKE_RETRO").is_ok_and(|v| v == "1")
}

/// This week's changes, one click each: the change first, the last "not this week".
pub fn set(now: u64) -> QuestionSet {
    let option = |label: &str, description: &str| Choice { label: label.into(), description: description.into(), ..Choice::default() };
    let change = |question: &str, header: &str, first: (&str, &str)| Question {
        question: question.into(),
        header: header.into(),
        options: vec![option(first.0, first.1), option("Keep the lesson only", "Change nothing, just remember what we learned"), option("Not this week", "")],
        ..Question::default()
    };
    QuestionSet {
        id: 9006,
        session: 1,
        member: "pm".into(),
        ticket: None,
        questions: vec![
            change("Lesson: tickets longer than half a day stalled. → Split tickets to half a day from next week?", "Ticket size", ("Split to half a day (Recommended)", "A big ticket like T-70 goes up as two")),
            change("Lesson: an approval waited overnight. → Gather approval requests twice a day, morning and afternoon?", "Approvals", ("Twice a day, gathered (Recommended)", "Only urgent ones at once, the rest at 10:00 and 16:00")),
        ],
        asked_at: now.saturating_sub(600),
        review: false,
        retro: Some(Retro { lessons: vec!["Tickets over half a day stall — split them small".into(), "Gather approvals".into()] }),
    }
}

fn ticket(id: u32, title: &str, status: TicketStatus, who: &str, project: &str) -> Ticket {
    Ticket {
        id,
        title: title.into(),
        body: String::new(),
        status,
        kind: TicketKind::Code,
        owner: Some(who.into()),
        assignee: None,
        project: Some(format!("/tmp/fake/{project}").into()),
        depends_on: vec![],
        branch: Some(format!("equipe/t-{id}")),
        worktree: None,
        merged: Some(status == TicketStatus::Done),
        off_main: None,
        design_qa: None,
        created_by: "pm".into(),
        note: None,
        plan: None,
        asks: vec![],
        remaining: vec![],
        images: Vec::new(),
        updated_at: None,
        priority: Default::default(),
        rank: 0,
    }
}

/// The week up to `to`, gathered as the daemon gathers it.
fn facts(to: u64) -> String {
    let from = to - retro::WEEK;
    let at = |days: u64, hours: u64| from + days * DAY + hours * HOUR;
    let event = |id: u32, when: u64, by: &str, kind: &str, text: &str| (id, TicketEntry { at: when, by: by.into(), kind: kind.into(), text: text.into() });
    let tickets = vec![
        ticket(47, "Product detail image gallery", TicketStatus::Done, "frontend", "web"),
        ticket(52, "Payment failure message", TicketStatus::Done, "frontend", "web"),
        ticket(70, "Orders API pagination", TicketStatus::InProgress, "backend", "api"),
    ];
    let events = vec![
        event(47, at(1, 3), "you", "accepted", ""),
        event(47, at(1, 4), "you", "merged", "main"),
        event(52, at(3, 2), "you", "accepted", ""),
        event(52, at(3, 2), "you", "merged", "main"),
        event(70, at(2, 1), "backend", retro::ESTIMATED, &at(2, 5).to_string()),
        event(70, at(2, 9), "backend", "handed in", ""),
        event(70, at(3, 1), "you", "reopened", "Reopened: the last page comes back as an empty list"),
    ];
    let turn = |member: &str, ticket: Option<u32>, day: u64, hours: u64, mins: u64, outcome: &str, tokens: u64| Turn { member: member.into(), role: member.into(), ticket, began: at(day, hours), ended: at(day, hours) + mins * 60, outcome: outcome.into(), tokens };
    let turns = vec![
        turn("frontend", Some(47), 0, 2, 25, "completed", 310_000),
        turn("frontend", Some(52), 2, 3, 18, "completed", 120_000),
        turn("backend", Some(70), 2, 1, 55, "completed", 840_000),
        turn("backend", Some(70), 2, 3, 6, "failed", 30_000),
        turn("pm", None, 0, 1, 12, "completed", 90_000),
        turn("pm", None, 4, 1, 9, "completed", 60_000),
    ];
    let waits = vec![Wait { kind: "permission".into(), member: "backend".into(), ticket: Some(70), what: "git push origin equipe/t-70".into(), asked: at(2, 10), answered: Some(at(3, 0)), outcome: "allowed".into() }];
    let gathered = retro::gather(&Week { from, to, tickets: &tickets, events: &events, turns: &turns, waits: &waits, briefs: vec![retro::Brief { project: "web".into(), file: "docs/briefs/checkout.md".into(), title: "Polish the checkout flow".into() }] });
    retro::facts_text(&gathered, &equipe_core::routines::Local)
}

/// The squad's routines: the weekly check-in and the retro, as a PM brings it.
pub fn routines() -> Vec<equipe_core::protocol::routines::Routine> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut check_in = equipe_core::heartbeat::weekly_check_in(crate::lang());
    let mut weekly = retro::routine(crate::lang(), 2);
    for r in [&mut check_in, &mut weekly] {
        r.next_run = Some(equipe_core::routines::next_run(r.when, now, &equipe_core::routines::Local));
    }
    weekly.last_run = Some(now.saturating_sub(HOUR));
    vec![check_in, weekly]
}

/// Last week's run and its retro, then this week's: the PM's conversation.
pub fn history(now: u64) -> Vec<Entry> {
    let routine = retro::routine(crate::lang(), 2);
    // This week's ran an hour ago; last week's, a week before.
    let this = now.saturating_sub(HOUR);
    let last = this - retro::WEEK;
    let earlier = "## Weekly retro\n\nFour things reached main this week, and one was stuck.\n\n### 1. Shipped\n- **T-31** Sign-up email verification — frontend\n- **T-33** Order history sorting — backend".to_owned();
    let week = "## Weekly retro\n\n\
Two things reached main this week, and the orders API (T-70) has been stuck for three days.\n\n\
### 1. Shipped\n\
- **T-47** Product detail image gallery — frontend, web\n\
- **T-52** Payment failure message — frontend, web\n\n\
### 2. What got stuck, and why\n\
- **T-70** Orders API pagination — handed in 4 hours later than said, and you sent it back for the empty list on the last page. One turn failed (the test database connection).\n\
- A `git push` approval waited 14 hours overnight.\n\n\
### 3. Time by member\n\
| Member | Turns | Time | Usage |\n|---|---|---|---|\n| backend | 2 | 1h 1m | 870k |\n| frontend | 2 | 43m | 430k |\n| pm | 2 | 21m | 150k |\n\n\
The friction fell on backend: 1 failure, 1 sent back, 1 late, 1 wait for approval.\n\n\
**Against the brief** — `docs/briefs/checkout.md` expected three checkout flows; this week only the failure message went out.\n\n\
### Lessons and what to change\n\
1. Tickets over half a day stall → tickets sized to half a day\n\
2. Gather approvals → twice a day, gathered\n\n\
Pick one on each card below. I won't change anything before you do."
        .to_owned();
    vec![
        Entry::User { text: retro::message(&routine, &facts(last)), kept: Vec::new() },
        Entry::Reply { text: earlier },
        Entry::User { text: "Great, same again next week please".into(), kept: Vec::new() },
        Entry::Reply { text: "Sure, I'll bring it again on Friday.".into() },
        Entry::User { text: retro::message(&routine, &facts(this)), kept: Vec::new() },
        Entry::Reply { text: week },
    ]
}
