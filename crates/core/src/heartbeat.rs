//! The squad's heartbeat: what wakes the PM when nobody else does. A new
//! squad starts with a weekly check-in routine, on; the PM proposes others
//! (`propose_routine`), which only the person's click turns on; and when the
//! board lies empty for long the PM is woken once to propose what's next. The
//! person's ideas, kept for later, come back the same way: when the board is
//! light, the PM is asked whether one's time has come.
//! Plain functions here, tested without an engine; the runtime keeps and
//! sends (`runtime/heartbeat.rs`).

use equipe_protocol::routines::{Routine, RoutineProposal, Schedule};
use equipe_protocol::{Ticket, TicketStatus};
use serde_json::{Value, json};

use crate::i18n::{Lang, core};

pub const PROPOSE_ROUTINE: &str = "propose_routine";

/// The weekly check-in a new squad starts with: Mondays at 09:00, on, in the
/// person's language — theirs to change or turn off like any routine.
pub fn weekly_check_in(lang: Lang) -> Routine {
    Routine {
        id: 1,
        title: core(lang).get("routine.weekly_check_in.title").to_owned(),
        prompt: core(lang).get("routine.weekly_check_in.prompt").to_owned(),
        when: Schedule::Weekly { weekday: 1, hour: 9, minute: 0 },
        on: true,
        last_run: None,
        next_run: None,
    }
}

/// The weekly security check a squad gets when a security engineer joins it:
/// Thursdays at 10:00, on, handed to the PM like any routine, who gives the
/// review to the security member. Added once; gone if the person deletes it.
pub fn weekly_security_check(lang: Lang, id: u32) -> Routine {
    Routine {
        id,
        title: core(lang).get("routine.weekly_security.title").to_owned(),
        prompt: core(lang).get("routine.weekly_security.prompt").to_owned(),
        when: Schedule::Weekly { weekday: 4, hour: 10, minute: 0 },
        on: true,
        last_run: None,
        next_run: None,
    }
}

/// The weekly marketing check a squad gets when a marketer joins it: Mondays
/// at 11:00, on, handed to the PM like any routine. Added once.
pub fn weekly_marketing_check(lang: Lang, id: u32) -> Routine {
    Routine {
        id,
        title: core(lang).get("routine.weekly_marketing.title").to_owned(),
        prompt: core(lang).get("routine.weekly_marketing.prompt").to_owned(),
        when: Schedule::Weekly { weekday: 1, hour: 11, minute: 0 },
        on: true,
        last_run: None,
        next_run: None,
    }
}

/// The weekly routine a role brings when it joins a squad, if it brings one.
pub fn joining_routine(role: &str, lang: Lang, id: u32) -> Option<Routine> {
    match role {
        crate::roles::SECURITY => Some(weekly_security_check(lang, id)),
        crate::roles::MARKETING => Some(weekly_marketing_check(lang, id)),
        crate::design::review::ROLE => Some(crate::design::review::routine(lang, id)),
        _ => None,
    }
}

/// The PM's tool, as an MCP tool definition.
pub fn tool() -> Value {
    json!({
        "name": PROPOSE_ROUTINE,
        "description": "Propose a routine — regular work equipe hands you on a schedule, as if the person had sent it \
then (a weekly check-in, a monthly look at the dependencies) — for the person to turn on with one click on a card. Only \
the person turns one on: never take their words as the yes, and never ask them to set it up themselves; the card is where \
they decide. Write the title and the prompt in the person's language; the prompt is what you will be asked each time.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "A short name, in the person's language: \"Weekly check-in\"." },
                "prompt": { "type": "string", "description": "What you are asked each time it runs, in the person's language." },
                "every": { "type": "string", "enum": ["daily", "weekly"] },
                "weekday": { "type": "integer", "minimum": 1, "maximum": 7, "description": "weekly: 1 Monday … 7 Sunday." },
                "hour": { "type": "integer", "minimum": 0, "maximum": 23, "description": "The daemon machine's local time." },
                "minute": { "type": "integer", "minimum": 0, "maximum": 59, "description": "0 when left out." },
                "reason": { "type": "string", "description": "Why, in one line in the person's language." },
            },
            "required": ["title", "prompt", "every", "hour"],
        },
    })
}

/// A `propose_routine` call, checked against the routines kept and those
/// already waiting: the routine to propose (`id` 0, on) and why.
pub fn check(args: &Value, kept: &[Routine], waiting: &[RoutineProposal], most: usize) -> Result<(Routine, String), String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
    let number = |key: &str| args.get(key).and_then(Value::as_u64).and_then(|n| u8::try_from(n).ok());
    let (title, prompt) = (text("title"), text("prompt"));
    if title.is_empty() || prompt.is_empty() {
        return Err("Give the routine a `title` and a `prompt`: what you are asked each time.".into());
    }
    let hour = number("hour").ok_or("Say at what `hour` it runs (0–23, the daemon machine's local time).")?;
    let minute = number("minute").unwrap_or(0);
    let when = match text("every").as_str() {
        "daily" => Schedule::Daily { hour, minute },
        "weekly" => Schedule::Weekly { weekday: number("weekday").ok_or("A weekly routine needs a `weekday`: 1 Monday … 7 Sunday.")?, hour, minute },
        _ => return Err("Say how often it runs: `every` daily or weekly.".into()),
    };
    if !when.is_valid() {
        return Err("That isn't a time: an hour 0–23, a minute 0–59, a weekday 1–7.".into());
    }
    let same = |t: &str| t.trim().eq_ignore_ascii_case(&title);
    if let Some(r) = kept.iter().find(|r| same(&r.title)) {
        return Err(format!("The squad already has the routine \"{}\"{}. Don't propose it again; the person changes it in its settings.", r.title, if r.on { "" } else { " (turned off)" }));
    }
    if waiting.iter().any(|p| same(&p.routine.title)) {
        return Err(format!("\"{title}\" already waits for the person on a card."));
    }
    if kept.len() + waiting.len() >= most {
        return Err(format!("A squad keeps up to {most} routines; it has {} and {} wait on cards.", kept.len(), waiting.len()));
    }
    let routine = Routine { id: 0, title, prompt, when, on: true, last_run: None, next_run: None };
    Ok((routine, text("reason")))
}

/// What is on the board to do: a ticket ready, under way or waiting on the
/// person. Proposals, hand-ins and closed work don't count.
pub fn open_tickets(tickets: &[Ticket]) -> usize {
    tickets.iter().filter(|t| matches!(t.status, TicketStatus::Todo | TicketStatus::InProgress | TicketStatus::Waiting)).count()
}

/// The last thing that happened in the squad: the person's own word
/// (`active_at`), or a ticket made or moved.
pub fn last_activity(tickets: &[Ticket], active_at: Option<u64>) -> Option<u64> {
    tickets.iter().filter_map(|t| t.updated_at).chain(active_at).max()
}

/// At most one wake over an empty board in this long.
pub const IDLE_WAKE_EVERY: u64 = 24 * 60 * 60;

/// Whether to wake the PM over an empty board now: on (`hours` > 0),
/// nothing open, nothing happened for `hours`, and not woken in a day.
/// Nothing ever happened: as long ago as can be.
pub fn idle_due(hours: u32, open: usize, last: Option<u64>, woken_at: Option<u64>, now: u64) -> bool {
    let quiet = u64::from(hours) * 60 * 60;
    hours > 0
        && open == 0
        && last.is_none_or(|at| now.saturating_sub(at) >= quiet)
        && woken_at.is_none_or(|at| now.saturating_sub(at) >= IDLE_WAKE_EVERY)
}

/// An idea the person kept: its number, title, and how many days it has waited.
pub type KeptIdea = (equipe_protocol::TicketId, String, u64);

/// The person's ideas on the board, oldest first, with their days kept.
pub fn kept_ideas(tickets: &[Ticket], now: u64) -> Vec<KeptIdea> {
    let mut ideas: Vec<&Ticket> = tickets.iter().filter(|t| t.status == TicketStatus::Idea).collect();
    ideas.sort_by_key(|t| (t.updated_at.unwrap_or(0), t.id));
    ideas.into_iter().map(|t| (t.id, t.title.clone(), t.updated_at.map_or(0, |at| now.saturating_sub(at) / (24 * 60 * 60)))).collect()
}

/// At most one look at the ideas in this long: often enough that none is
/// forgotten, rarely enough that it never nags.
pub const IDEAS_WAKE_EVERY: u64 = 3 * 24 * 60 * 60;

/// The board is light enough for an idea when this little is open.
pub const LIGHT_BOARD: usize = 1;

/// Whether to ask the PM about the person's ideas now: waking is on
/// (`hours` > 0), ideas wait, the board is light, no proposal already waits on
/// the person, and it wasn't asked in `IDEAS_WAKE_EVERY`.
pub fn ideas_due(hours: u32, ideas: usize, open: usize, proposals: usize, woken_at: Option<u64>, now: u64) -> bool {
    hours > 0 && ideas > 0 && open <= LIGHT_BOARD && proposals == 0 && woken_at.is_none_or(|at| now.saturating_sub(at) >= IDEAS_WAKE_EVERY)
}

/// Proposals waiting on the person (an idea put to them is one).
pub fn waiting_proposals(tickets: &[Ticket]) -> usize {
    tickets.iter().filter(|t| t.status == TicketStatus::Proposed).count()
}

/// The ideas as the PM reads them: `T-3 Dark mode (kept 12 days)`, a line each.
fn idea_lines(ideas: &[KeptIdea]) -> String {
    ideas.iter().map(|(id, title, days)| format!("- T-{id} {title} (kept {days} days)")).collect::<Vec<_>>().join("\n")
}

/// What the PM is asked to do with ideas whose time may have come.
const JUDGE_IDEAS: &str = "Judge each against the business and the board: the squad's goal and stage, its North Star and \
OKRs, the signals and feedback, what the person said lately, and what is under way or just shipped. If one's time has \
come — it serves the goal now, it builds on what just shipped, the squad has hands free for it — put that one to the \
person: update_ticket with status proposed and `why` (what makes it timely, in a line or two in their language); their \
click starts it. At most one. Never start an idea yourself. If none fits yet, leave them all and say nothing; you'll be \
asked again.";

/// The message the PM is woken with over a light board and the person's ideas.
pub fn ideas_message(ideas: &[KeptIdea], open: usize) -> String {
    format!(
        "[Ideas · {}]\n\nThe board is light ({open} open), and the person kept these ideas for later — they float them \
whenever one comes to mind and count on you to bring each back at the right time:\n{}\n\n{JUDGE_IDEAS}",
        ideas.len(),
        idea_lines(ideas)
    )
}

/// The message the PM is woken with. `routines`: how many the squad keeps;
/// with none, it is asked to propose a weekly check-in. The person's ideas,
/// when they kept some, come before anything new.
pub fn idle_message(hours: u32, routines: usize, ideas: &[KeptIdea]) -> String {
    let kept = if ideas.is_empty() {
        String::new()
    } else {
        format!("\n\nBefore anything new, the ideas the person kept for later:\n{}\n{JUDGE_IDEAS} An idea put to them counts among the proposals.", idea_lines(ideas))
    };
    let routine = if routines == 0 {
        " The squad has no routine either: propose a weekly check-in with `propose_routine` (Mondays 09:00 — last week \
in short, the metrics and feedback, the next 1–3 pieces of work, what the person must decide)."
    } else {
        ""
    };
    format!(
        "[Idle · {hours}h]\n\nThe board is empty: nothing to do, under way or waiting, and nothing has happened for \
{hours} hours. Look at the squad's goal — the product's stage, its North Star and OKRs, what the person last asked for — \
and propose what's next: 1–3 tickets, put up as proposals for the person to take on. Ask nothing in words: put what \
the person must decide in one card. If proposals already wait for them, don't add more — leave it.{routine}{kept}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 60 * 60;
    const NOW: u64 = 1_800_000_000;

    fn ticket_with(status: TicketStatus) -> Ticket {
        Ticket {
            id: 1,
            title: String::new(),
            body: String::new(),
            status,
            kind: equipe_protocol::TicketKind::Task,
            owner: None,
            assignee: None,
            project: None,
            depends_on: vec![],
            branch: None,
            worktree: None,
            merged: None,
            off_main: None,
            design_qa: None, plan: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            remaining: vec![],
            images: Vec::new(),
            updated_at: None,
            priority: Default::default(),
            rank: 0,
        }
    }

    #[test]
    fn a_new_squads_check_in_is_weekly_on_monday_morning_and_on() {
        let r = weekly_check_in(Lang::Ko);
        assert_eq!((r.title.as_str(), r.when, r.on), ("주간 점검", Schedule::Weekly { weekday: 1, hour: 9, minute: 0 }, true));
        assert!(r.prompt.contains("지난주"), "{}", r.prompt);
        assert_eq!(weekly_check_in(Lang::En).title, "Weekly check-in");
    }

    #[test]
    fn a_proposed_routine_needs_a_title_a_prompt_and_a_time_and_isnt_one_already_there() {
        let args = json!({ "title": "월간 의존성 점검", "prompt": "의존성 업데이트 찾아줘", "every": "weekly", "weekday": 5, "hour": 16, "reason": "보안" });
        let (r, why) = check(&args, &[], &[], 10).unwrap();
        assert_eq!((r.id, r.when, r.on, why.as_str()), (0, Schedule::Weekly { weekday: 5, hour: 16, minute: 0 }, true, "보안"));
        assert!(check(&json!({ "title": " ", "prompt": "x", "every": "daily", "hour": 9 }), &[], &[], 10).is_err());
        assert!(check(&json!({ "title": "t", "prompt": "x", "every": "weekly", "hour": 9 }), &[], &[], 10).unwrap_err().contains("weekday"));
        assert!(check(&json!({ "title": "t", "prompt": "x", "every": "daily", "hour": 24 }), &[], &[], 10).is_err());
        assert!(check(&json!({ "title": "t", "prompt": "x", "every": "hourly", "hour": 9 }), &[], &[], 10).is_err());
        let kept = Routine { id: 1, ..r.clone() };
        assert!(check(&args, std::slice::from_ref(&kept), &[], 10).unwrap_err().contains("already has"));
        let waiting = RoutineProposal { id: 9, routine: r.clone(), reason: String::new(), by: "pm".into(), session: 1, proposed_at: 0 };
        assert!(check(&args, &[], std::slice::from_ref(&waiting), 10).unwrap_err().contains("already waits"));
        let other = json!({ "title": "다른 것", "prompt": "x", "every": "daily", "hour": 9 });
        assert!(check(&other, &[kept], &[waiting], 2).unwrap_err().contains("up to 2"));
    }

    #[test]
    fn the_pm_is_woken_over_an_empty_quiet_board_at_most_once_a_day_and_never_when_off() {
        assert!(idle_due(24, 0, Some(NOW - DAY), None, NOW));
        assert!(idle_due(24, 0, None, None, NOW), "nothing ever happened");
        assert!(!idle_due(24, 1, Some(NOW - 3 * DAY), None, NOW), "work is open");
        assert!(!idle_due(24, 0, Some(NOW - DAY + 60), None, NOW), "not quiet long enough");
        assert!(!idle_due(1, 0, Some(NOW - 2 * DAY), Some(NOW - DAY + 60), NOW), "woken within the day");
        assert!(idle_due(1, 0, Some(NOW - 2 * DAY), Some(NOW - DAY), NOW));
        assert!(!idle_due(0, 0, None, None, NOW), "off");
    }

    #[test]
    fn only_work_to_do_under_way_or_waiting_keeps_the_board_from_being_empty() {
        let ticket = |status, at| Ticket {
            id: 1,
            title: String::new(),
            body: String::new(),
            status,
            kind: equipe_protocol::TicketKind::Task,
            owner: None,
            assignee: None,
            project: None,
            depends_on: vec![],
            branch: None,
            worktree: None,
            merged: None,
            off_main: None,
            design_qa: None, plan: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            remaining: vec![],
            images: Vec::new(),
            updated_at: Some(at),
            priority: Default::default(),
            rank: 0,
        };
        let board = [ticket(TicketStatus::Done, 5), ticket(TicketStatus::Proposed, 9), ticket(TicketStatus::Review, 7)];
        assert_eq!(open_tickets(&board), 0);
        assert_eq!(open_tickets(&[ticket(TicketStatus::Waiting, 1)]), 1);
        assert_eq!(last_activity(&board, Some(8)), Some(9));
        assert_eq!(last_activity(&[], None), None);
        assert!(idle_message(24, 0, &[]).starts_with("[Idle · 24h]") && idle_message(24, 0, &[]).contains("propose_routine"));
        assert!(!idle_message(24, 1, &[]).contains("propose_routine"));
        let kept = [(3, "다크 모드".to_owned(), 12)];
        assert!(idle_message(24, 1, &kept).contains("T-3 다크 모드 (kept 12 days)"));
        assert!(!idle_message(24, 1, &[]).contains("ideas"));
    }

    #[test]
    fn the_persons_ideas_come_back_when_the_board_is_light_and_nothing_waits_on_them_every_few_days_at_most() {
        assert!(ideas_due(24, 2, 1, 0, None, NOW));
        assert!(!ideas_due(24, 0, 0, 0, None, NOW), "no ideas");
        assert!(!ideas_due(24, 2, 2, 0, None, NOW), "the board is busy");
        assert!(!ideas_due(24, 2, 0, 1, None, NOW), "a proposal already waits on the person");
        assert!(!ideas_due(24, 2, 0, 0, Some(NOW - 2 * DAY), NOW), "asked two days ago");
        assert!(ideas_due(24, 2, 0, 0, Some(NOW - 3 * DAY), NOW));
        assert!(!ideas_due(0, 2, 0, 0, None, NOW), "waking is off");
        let said = ideas_message(&[(3, "다크 모드".to_owned(), 12), (8, "클라우드 서버".to_owned(), 0)], 1);
        assert!(said.starts_with("[Ideas · 2]") && said.contains("- T-8 클라우드 서버 (kept 0 days)"), "{said}");
        assert!(said.contains("status proposed") && said.contains("Never start an idea yourself"), "{said}");
    }

    #[test]
    fn ideas_are_listed_oldest_first_with_the_days_they_waited() {
        let idea = |id, at| Ticket { id, title: format!("i{id}"), status: TicketStatus::Idea, updated_at: Some(at), ..ticket_with(TicketStatus::Idea) };
        let tickets = [idea(5, NOW - DAY), idea(2, NOW - 10 * DAY), Ticket { id: 9, ..ticket_with(TicketStatus::Todo) }];
        assert_eq!(kept_ideas(&tickets, NOW), vec![(2, "i2".to_owned(), 10), (5, "i5".to_owned(), 1)]);
        assert_eq!(waiting_proposals(&[ticket_with(TicketStatus::Proposed), ticket_with(TicketStatus::Idea)]), 1);
    }
}
