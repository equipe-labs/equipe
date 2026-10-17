//! Handed-in work the person says in words they accept, merge or send back —
//! "전부 수락", "T-1이랑 T-3 수락해줘", "머지까지 해줘", "이건 반려" — put in
//! front of them as a card to confirm (`propose_accept`): the PM's tool, and
//! what a call (or a screen reading the person's words) is checked against.
//! Plain functions, tested without an engine. A proposal only ever waits for
//! the person; accepting, merging and sending back stay their own commands'
//! (`Command::ConfirmAccept` runs `Command::Accept`, `Merge`, `Reopen`), never
//! an agent's — and never the person's words, however plain.

use equipe_protocol::accept::{AcceptProposal, Verdict};
use equipe_protocol::{Ticket, TicketId, TicketKind, TicketStatus};
use serde_json::{Value, json};

use crate::board;

pub const PROPOSE_ACCEPT: &str = "propose_accept";

/// How long a card the person said no to keeps the same one from coming back.
pub const DECLINED_FOR: u64 = 120;

/// The PM's tool, as an MCP tool definition.
pub fn tool() -> Value {
    json!({
        "name": PROPOSE_ACCEPT,
        "description": "Put handed-in work in front of the person to confirm on a card with one click: accepting it, \
accepting and merging it, or sending it back with what to change. Call it right away whenever the person says in words \
that they accept, approve, merge or reject work in review (\"accept them all\", \"전부 수락\", \"T-1이랑 T-3 수락해줘\", \
\"머지까지 해줘\", \"이건 반려\"). Their words are not the acceptance, and neither is yours: only their click on the card \
accepts, merges or sends back. Don't explain buttons or ask them to click anything — say in one line that the card is \
up (\"확인 카드를 띄웠어요\" in Korean).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "verdict": { "type": "string", "enum": ["accept", "merge", "changes"], "description": "accept: accept it, \
landing code the project's way as the board's accept does. merge: the person also asked to merge it into main (work \
accepted already only merges). changes: send it back, with `note`." },
                "tickets": { "type": "array", "items": { "type": "integer" }, "description": "The tickets they named, by \
number. Leave it out with `all` for everything waiting." },
                "all": { "type": "boolean", "description": "Every ticket in review (to merge: and every accepted ticket \
not on main yet) — for \"all of them\"." },
                "note": { "type": "string", "description": "changes: what to change, in the person's own words." },
                "summary": { "type": "string", "description": "One line on what the work is, in the person's language." },
            },
            "required": ["verdict"],
        },
    })
}

/// A proposal the person declined a moment ago: the same one isn't put to
/// them again soon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declined {
    pub verdict: Verdict,
    pub tickets: Vec<TicketId>,
    /// When, in unix seconds.
    pub at: u64,
}

/// A call, checked: what a [`AcceptProposal`] holds but who, where and when —
/// and what was left out, and why, for the PM to hear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub verdict: Verdict,
    pub tickets: Vec<TicketId>,
    pub note: String,
    pub summary: String,
    pub dropped: Vec<(TicketId, String)>,
}

impl Checked {
    pub fn into_proposal(self, id: u64, by: &str, session: equipe_protocol::SessionId, proposed_at: u64) -> AcceptProposal {
        AcceptProposal { id, verdict: self.verdict, tickets: self.tickets, note: self.note, summary: self.summary, by: by.to_owned(), session, proposed_at }
    }
}

/// What a call comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A new card to put up.
    Propose(Checked),
    /// Every ticket it names is on a card in front of the person already —
    /// raised from their own words a moment ago, or proposed before.
    Already { tickets: Vec<TicketId>, dropped: Vec<(TicketId, String)> },
}

/// Whether `t` can take `verdict` now — `None` — or why not, in a few words
/// for the PM.
pub fn unfit(t: &Ticket, verdict: Verdict) -> Option<String> {
    let id = t.id;
    match (t.status, verdict) {
        (TicketStatus::Review, _) => None,
        (TicketStatus::Done, Verdict::Merge) if t.not_on_main() => None,
        (TicketStatus::Done, Verdict::Merge) if t.kind != TicketKind::Code => Some(format!("T-{id} is accepted, its work in the squad's folder: nothing to merge")),
        (TicketStatus::Done, Verdict::Merge) => Some(format!("T-{id} is accepted and on main already")),
        (TicketStatus::Done, Verdict::Accept) if t.not_on_main() => Some(format!("T-{id} is accepted already, not on main yet: propose `merge` to land it")),
        (TicketStatus::Done, _) => Some(format!("T-{id} is accepted already")),
        (TicketStatus::Cancelled, _) => Some(format!("T-{id} was dropped")),
        (TicketStatus::Proposed, _) => Some(format!("T-{id} is a proposal, not handed-in work: take it on with update_ticket (status todo)")),
        (status, _) => Some(format!("T-{id} is {}, not handed in yet", status.label())),
    }
}

/// The tickets "all" means for `verdict`: every one in review — and, to
/// merge, every accepted one still asking to be merged.
pub fn all_of(tickets: &[Ticket], verdict: Verdict) -> Vec<TicketId> {
    let mut ids: Vec<TicketId> = tickets.iter().filter(|t| t.status == TicketStatus::Review || (verdict == Verdict::Merge && t.waits_to_merge())).map(|t| t.id).collect();
    ids.sort_unstable();
    ids
}

/// Whether a proposal still has something for the person to confirm: one of
/// its tickets still takes its verdict.
pub fn still_open(proposal: &AcceptProposal, tickets: &[Ticket]) -> bool {
    proposal.tickets.iter().any(|id| tickets.iter().any(|t| t.id == *id && unfit(t, proposal.verdict).is_none()))
}

/// Tickets named, checked against the board and what already waits — the
/// same rules for the PM's tool and a screen reading the person's words.
/// What can't be proposed is left out with why; nothing left, an error.
pub fn check_tickets(verdict: Verdict, named: &[TicketId], note: &str, tickets: &[Ticket], open: &[AcceptProposal], declined: &[Declined], now: u64) -> Result<Outcome, String> {
    let note = note.trim().to_owned();
    if verdict == Verdict::Changes && note.is_empty() {
        return Err("Say what to change in `note`, in the person's own words.".into());
    }
    let mut seen: Vec<TicketId> = Vec::new();
    let mut fit: Vec<TicketId> = Vec::new();
    let mut waiting: Vec<TicketId> = Vec::new();
    let mut dropped: Vec<(TicketId, String)> = Vec::new();
    for &id in named {
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let Some(t) = tickets.iter().find(|t| t.id == id) else {
            dropped.push((id, format!("there is no T-{id}")));
            continue;
        };
        if let Some(why) = unfit(t, verdict) {
            dropped.push((id, why));
            continue;
        }
        if open.iter().any(|p| p.verdict == verdict && p.tickets.contains(&id)) {
            waiting.push(id);
            continue;
        }
        fit.push(id);
    }
    if fit.is_empty() && !waiting.is_empty() {
        return Ok(Outcome::Already { tickets: waiting, dropped });
    }
    if fit.is_empty() {
        let why: Vec<String> = dropped.iter().map(|(_, w)| w.clone()).collect();
        return Err(match why.as_slice() {
            [] => "Name the tickets (`tickets`), or say `all` for everything waiting.".into(),
            _ => format!("Nothing to put to the person: {}.", why.join("; ")),
        });
    }
    let just_said_no = declined.iter().find(|d| d.verdict == verdict && now.saturating_sub(d.at) < DECLINED_FOR && fit.iter().all(|id| d.tickets.contains(id)));
    if just_said_no.is_some() {
        return Err("The person said no to that very card a moment ago: don't put it to them again unless they ask.".into());
    }
    // Already on a card: said so, not proposed twice.
    for id in waiting {
        dropped.push((id, format!("T-{id} is on a card already")));
    }
    Ok(Outcome::Propose(Checked { verdict, tickets: fit, note, summary: String::new(), dropped }))
}

/// A `propose_accept` call checked against the board and what already
/// waits; what is wrong is said so the PM can call again.
pub fn check(args: &Value, tickets: &[Ticket], open: &[AcceptProposal], declined: &[Declined], now: u64) -> Result<Outcome, String> {
    let verdict = match board::arg_str(args, "verdict").map(str::to_lowercase).as_deref() {
        Some("accept") | Some("approve") => Verdict::Accept,
        Some("merge") => Verdict::Merge,
        Some("changes") | Some("reject") | Some("reopen") | Some("request_changes") => Verdict::Changes,
        _ => return Err("Say which `verdict`: `accept`, `merge` or `changes`.".into()),
    };
    let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
    let named = match (board::arg_ids(args, "tickets").unwrap_or_default(), all) {
        (ids, false) if ids.is_empty() => return Err("Name the tickets (`tickets`), or say `all` for everything waiting.".into()),
        (ids, false) => ids,
        (_, true) => {
            let ids = all_of(tickets, verdict);
            if ids.is_empty() {
                return Err("Nothing waits for the person to accept right now.".into());
            }
            ids
        }
    };
    let note = board::arg_str(args, "note").unwrap_or_default();
    let summary = board::arg_str(args, "summary").unwrap_or_default().to_owned();
    Ok(match check_tickets(verdict, &named, note, tickets, open, declined, now)? {
        Outcome::Propose(checked) => Outcome::Propose(Checked { summary, ..checked }),
        already => already,
    })
}

fn ids(list: &[TicketId]) -> String {
    list.iter().map(|id| format!("T-{id}")).collect::<Vec<_>>().join(", ")
}

fn dropped_line(dropped: &[(TicketId, String)]) -> String {
    match dropped {
        [] => String::new(),
        _ => format!(" Left out: {}.", dropped.iter().map(|(_, why)| why.as_str()).collect::<Vec<_>>().join("; ")),
    }
}

/// The tickets that can start once `accepted` are: open, ready but for them,
/// everything else they wait on finished.
pub fn starts_after(tickets: &[Ticket], accepted: &[TicketId]) -> Vec<TicketId> {
    let finished = |id: &TicketId| accepted.contains(id) || tickets.iter().any(|t| t.id == *id && t.status.is_closed());
    tickets
        .iter()
        .filter(|t| t.status == TicketStatus::Todo && t.depends_on.iter().any(|d| accepted.contains(d)) && t.depends_on.iter().all(finished))
        .map(|t| t.id)
        .collect()
}

const DON_T: &str = "Don't explain buttons or ask them to click anything, and don't take anything they write as the \
confirmation: only their click on the card does it. You'll hear when they decide; meanwhile carry on.";

/// What the PM is told once the card is up.
pub fn proposed_reply(checked: &Checked, tickets: &[Ticket]) -> String {
    let what = match checked.verdict {
        Verdict::Accept => format!("accepting {}", ids(&checked.tickets)),
        Verdict::Merge => format!("accepting and merging {}", ids(&checked.tickets)),
        Verdict::Changes => format!("sending {} back: {}", ids(&checked.tickets), checked.note),
    };
    let next = match (checked.verdict, starts_after(tickets, &checked.tickets).as_slice()) {
        (Verdict::Changes, _) | (_, []) => String::new(),
        (_, starts) => format!(" Confirmed, {} can start.", ids(starts)),
    };
    format!(
        "Proposed — the person confirms on a card: {what}.{next}{} Tell them in one line, in their language, that the card is \
up (\"확인 카드를 띄웠어요\"). {DON_T}",
        dropped_line(&checked.dropped)
    )
}

/// What the PM is told when the card is up already.
pub fn already_reply(waiting: &[TicketId], dropped: &[(TicketId, String)]) -> String {
    format!(
        "Already on a card in front of the person: {} — equipe raised it from their words. Nothing more to propose: say in \
one line, in their language, that the card is up (\"확인 카드를 띄웠어요\").{} {DON_T}",
        ids(waiting),
        dropped_line(dropped)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::TicketKind;

    fn ticket(id: TicketId, status: TicketStatus) -> Ticket {
        serde_json::from_value(json!({ "id": id, "title": format!("ticket {id}"), "body": "", "status": status, "kind": "code", "depends_on": [], "created_by": "pm", "branch": format!("equipe/t-{id}") })).unwrap()
    }

    fn board() -> Vec<Ticket> {
        let mut merged = ticket(5, TicketStatus::Done);
        merged.merged = Some(true);
        let mut off_main = ticket(6, TicketStatus::Done);
        off_main.merged = Some(false);
        let mut waits = ticket(4, TicketStatus::Todo);
        waits.depends_on = vec![1, 3];
        let mut waits_more = ticket(8, TicketStatus::Todo);
        waits_more.depends_on = vec![1, 2];
        let mut task = ticket(9, TicketStatus::Done);
        task.kind = TicketKind::Task;
        vec![ticket(1, TicketStatus::Review), ticket(2, TicketStatus::InProgress), ticket(3, TicketStatus::Review), waits, merged, off_main, ticket(7, TicketStatus::Proposed), waits_more, task]
    }

    fn call(args: Value) -> Result<Outcome, String> {
        check(&args, &board(), &[], &[], 1000)
    }

    fn proposed(args: Value) -> Checked {
        match call(args).unwrap() {
            Outcome::Propose(c) => c,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_tickets_named_in_review_are_put_to_the_person_once_each() {
        let c = proposed(json!({ "verdict": "accept", "tickets": [1, "T-3", 1], "summary": " 로그인과 결제 " }));
        assert_eq!((c.verdict, c.tickets.as_slice(), c.summary.as_str(), c.dropped.len()), (Verdict::Accept, &[1, 3][..], "로그인과 결제", 0), "a duplicate is one");
        // "All of them": everything in review — to merge, accepted work off main too.
        assert_eq!(proposed(json!({ "verdict": "accept", "all": true })).tickets, [1, 3]);
        assert_eq!(proposed(json!({ "verdict": "merge", "all": true })).tickets, [1, 3, 6]);
        assert_eq!(proposed(json!({ "verdict": "merge", "tickets": [6] })).tickets, [6], "accepted, not on main: merges");
        let back = proposed(json!({ "verdict": "changes", "tickets": [3], "note": "버튼을 더 크게" }));
        assert_eq!((back.verdict, back.note.as_str()), (Verdict::Changes, "버튼을 더 크게"));
    }

    #[test]
    fn what_cant_be_confirmed_is_left_out_with_why_and_nothing_left_is_an_error() {
        let c = proposed(json!({ "verdict": "accept", "tickets": [1, 2, 99, 5, 7] }));
        assert_eq!(c.tickets, [1]);
        let why: Vec<&str> = c.dropped.iter().map(|(_, w)| w.as_str()).collect();
        assert_eq!(why, ["T-2 is in progress, not handed in yet", "there is no T-99", "T-5 is accepted already", "T-7 is a proposal, not handed-in work: take it on with update_ticket (status todo)"]);
        let err = |args: Value| call(args).unwrap_err();
        assert!(err(json!({ "verdict": "accept", "tickets": [99] })).contains("there is no T-99"));
        assert!(err(json!({ "verdict": "accept", "tickets": [2] })).contains("T-2 is in progress, not handed in yet"));
        assert!(err(json!({ "verdict": "merge", "tickets": [5] })).contains("T-5 is accepted and on main already"));
        assert!(err(json!({ "verdict": "merge", "tickets": [9] })).contains("work in the squad's folder: nothing to merge"));
        assert!(err(json!({ "verdict": "accept", "tickets": [6] })).contains("propose `merge` to land it"));
        assert!(err(json!({ "tickets": [1] })).contains("`accept`, `merge` or `changes`"));
        assert!(err(json!({ "verdict": "accept" })).contains("Name the tickets"));
        assert!(err(json!({ "verdict": "accept", "tickets": [] })).contains("Name the tickets"));
        assert!(err(json!({ "verdict": "changes", "tickets": [1] })).contains("what to change in `note`"));
        assert!(check(&json!({ "verdict": "accept", "all": true }), &[ticket(2, TicketStatus::InProgress)], &[], &[], 0).unwrap_err().contains("Nothing waits"));
    }

    #[test]
    fn a_card_up_already_is_not_proposed_twice_and_one_declined_doesnt_come_back_soon() {
        let up = proposed(json!({ "verdict": "accept", "tickets": [1, 3] })).into_proposal(8, equipe_protocol::accept::FROM_WORDS, 1, 990);
        let open = std::slice::from_ref(&up);
        // The PM, a moment after a screen raised it from the person's words: nothing twice.
        assert_eq!(check(&json!({ "verdict": "accept", "all": true }), &board(), open, &[], 1000), Ok(Outcome::Already { tickets: vec![1, 3], dropped: vec![] }));
        // Part of it on a card: the rest goes up, the PM told which was there.
        let more = check(&json!({ "verdict": "merge", "tickets": [1, 6] }), &board(), open, &[], 1000).unwrap();
        assert!(matches!(&more, Outcome::Propose(c) if c.tickets == [1, 6]), "another verdict is another card: {more:?}");
        let up6 = proposed(json!({ "verdict": "merge", "tickets": [6] })).into_proposal(9, "pm", 1, 990);
        let more = check(&json!({ "verdict": "merge", "tickets": [1, 6] }), &board(), std::slice::from_ref(&up6), &[], 1000).unwrap();
        assert!(matches!(&more, Outcome::Propose(c) if c.tickets == [1] && c.dropped == [(6, "T-6 is on a card already".to_owned())]), "{more:?}");
        // Said no to: not again for a while, then it may be.
        let no = [Declined { verdict: Verdict::Accept, tickets: vec![1, 3], at: 990 }];
        assert!(check(&json!({ "verdict": "accept", "tickets": [3] }), &board(), &[], &no, 1000).unwrap_err().contains("said no to that very card"));
        assert!(check(&json!({ "verdict": "accept", "tickets": [3] }), &board(), &[], &no, 990 + DECLINED_FOR).is_ok());
        assert!(check(&json!({ "verdict": "changes", "tickets": [3], "note": "x" }), &board(), &[], &no, 1000).is_ok(), "another verdict");
    }

    #[test]
    fn the_pm_hears_what_starts_after_what_was_left_out_and_not_to_lecture() {
        let c = proposed(json!({ "verdict": "accept", "tickets": [1, 3, 2] }));
        let said = proposed_reply(&c, &board());
        assert!(said.starts_with("Proposed — the person confirms on a card: accepting T-1, T-3. Confirmed, T-4 can start."), "{said}");
        assert!(said.contains("Left out: T-2 is in progress, not handed in yet.") && said.contains("확인 카드를 띄웠어요") && said.contains("Don't explain buttons"), "{said}");
        assert!(said.contains("only their click on the card does it"), "{said}");
        let back = proposed(json!({ "verdict": "changes", "tickets": [3], "note": "헤더" }));
        assert!(proposed_reply(&back, &board()).contains("sending T-3 back: 헤더.") && !proposed_reply(&back, &board()).contains("can start"));
        let already = already_reply(&[1, 3], &[]);
        assert!(already.starts_with("Already on a card in front of the person: T-1, T-3") && already.contains("확인 카드를 띄웠어요"), "{already}");
        assert_eq!(starts_after(&board(), &[1]), Vec::<TicketId>::new(), "T-4 still waits on T-3, T-8 on T-2");
        assert_eq!(starts_after(&board(), &[1, 3]), [4]);
        assert_eq!(tool()["name"], PROPOSE_ACCEPT);
        assert_eq!(tool()["inputSchema"]["required"], json!(["verdict"]));
    }

    #[test]
    fn a_proposal_stays_open_while_one_of_its_tickets_still_takes_it() {
        let p = proposed(json!({ "verdict": "accept", "tickets": [1, 3] })).into_proposal(1, "pm", 1, 0);
        assert!(still_open(&p, &board()));
        let mut decided = board();
        decided.iter_mut().filter(|t| t.id == 1 || t.id == 3).for_each(|t| t.status = TicketStatus::Done);
        assert!(!still_open(&p, &decided));
    }
}
