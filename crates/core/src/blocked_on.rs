//! A ticket blocked on work that doesn't exist yet — another role's, or
//! another project's — turned into structure instead of a relay.
//!
//! Seen in production: a frontend ticket needed a server endpoint nobody
//! had made. Frontend, QA and the PM passed the ticket between them thirteen
//! times in under two hours, each note saying in prose that the server
//! contract had no such endpoint. Nobody made the endpoint.
//!
//! Now a member names the missing piece — `blocked_on: { role, what }` on
//! `hand_back` or `pass_baton` ([`parse`]) — and equipe makes that ticket for
//! the role, or finds the one already on the board ([`find_prerequisite`]),
//! under the same request; the blocked ticket waits on it (`depends_on`) and
//! comes back to its member once it lands. Nothing passes it meanwhile.
//!
//! When members say it only in prose, the relay is caught by its notes
//! ([`says_missing`]): the second hand-back or baton on one ticket whose note
//! says something it needs is missing — an API, an endpoint, a contract, a
//! permission — doesn't pass it on. The PM gets one card asking whether to
//! make the missing ticket ([`pm_card`]), once per ticket.
//!
//! The rules here are plain functions; the runtime's side is
//! `runtime::blocked_on`.

use equipe_protocol::{Ticket, TicketId, TicketStatus};
use serde_json::Value;

/// The ticket event a ticket put down on missing work gets: its text names
/// the prerequisite (`T-12 (backend): …`).
pub const BLOCKED_ON: &str = "blocked on";
/// The ticket event of a prerequisite equipe made: its text names the
/// ticket it unblocks.
pub const PREREQUISITE: &str = "prerequisite";
/// The ticket event of a relay caught by its notes, put to the PM — once.
pub const SUSPECTED: &str = "blocked on missing work?";

/// The ticket events that move a ticket away from its member without its
/// work done: the passes a relay is made of.
pub const PASSES: &[&str] = &[crate::lead_time::BATON, "handed back", "sent back", "design QA: changes"];

/// How many passes on one ticket whose notes say something is missing make
/// a relay: this one and one before it.
pub const RELAY_AT: usize = 2;

/// What a ticket waits on that doesn't exist yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedOn {
    /// The role (or member) whose work it is: `backend`.
    pub role: String,
    /// What is missing, in a line: "an admin-status endpoint that reads after a lost response".
    pub what: String,
    /// The project it is in, when not the blocked ticket's own.
    pub project: Option<String>,
}

/// `blocked_on` in a tool's arguments: `None` when it isn't given; an error
/// saying what to fix when it is, but incomplete.
pub fn parse(args: &Value) -> Result<Option<BlockedOn>, String> {
    let Some(given) = args.get("blocked_on").filter(|v| !v.is_null()) else { return Ok(None) };
    let field = |name: &str| given.get(name).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    let role = field("role").ok_or("`blocked_on` needs `role`: whose work the missing piece is (backend, a member's name).")?;
    let what = field("what").ok_or("`blocked_on` needs `what`: the missing piece in a line — the endpoint, contract, permission or data it needs.")?;
    Ok(Some(BlockedOn { role, what, project: field("project") }))
}

/// The `blocked_on` schema the tools that take it share.
pub fn schema() -> Value {
    serde_json::json!({
        "type": "object",
        "description": "Work another role or project must do first doesn't exist yet — an API or endpoint, a contract or schema, a \
permission, data. Name it here instead of passing the ticket on or explaining it in a note: equipe makes that ticket for the role \
(or finds the one already on the board) under the same request, and this ticket waits on it and comes back to you once it lands.",
        "properties": {
            "role": { "type": "string", "description": "Whose work it is: a role (backend) or a member's name." },
            "what": { "type": "string", "description": "The missing piece in a line, as its ticket's title: what it must provide, not why you need it." },
            "project": { "type": "string", "description": "The project it is in, when not this ticket's own." },
        },
        "required": ["role", "what"],
    })
}

/// The words of `text`, lower-case, three characters and longer, for
/// telling whether two lines say the same thing.
fn words(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut out: Vec<String> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3).map(str::to_owned).collect();
    out.sort();
    out.dedup();
    out
}

/// Whether `ticket` says what `what` asks for: most of its words are in
/// the ticket's title or body.
fn says_same(ticket: &Ticket, what: &str) -> bool {
    let wanted = words(what);
    if wanted.is_empty() {
        return false;
    }
    let have = words(&format!("{} {}", ticket.title, ticket.body));
    let found = wanted.iter().filter(|w| have.contains(w)).count();
    found * 3 >= wanted.len() * 2
}

/// Whether `owner` is `role`'s: the role itself, or a member of it (`backend-2`).
fn of_role(owner: Option<&str>, role: &str) -> bool {
    owner.is_some_and(|o| o == role || crate::board::role_of(o) == crate::board::role_of(role))
}

/// The open ticket on the board that already is what `blocked` waits on:
/// one it depends on already, of that role; one equipe made for it before;
/// or one of its request, of that role, that says the same. `request` holds
/// the tickets of `blocked`'s request.
pub fn find_prerequisite(board: &[Ticket], blocked: &Ticket, request: &[TicketId], missing: &BlockedOn) -> Option<TicketId> {
    let open = |t: &&Ticket| t.id != blocked.id && !t.status.is_closed() && t.status != TicketStatus::Idea;
    let mine = |t: &&Ticket| of_role(t.owner.as_deref().or(t.assignee.as_deref()), &missing.role);
    let marker = unblocks_line(blocked.id);
    let candidates: Vec<&Ticket> = board.iter().filter(open).filter(mine).collect();
    candidates
        .iter()
        .find(|t| blocked.depends_on.contains(&t.id))
        .or_else(|| candidates.iter().find(|t| t.body.contains(&marker) && says_same(t, &missing.what)))
        .or_else(|| candidates.iter().find(|t| request.contains(&t.id) && says_same(t, &missing.what)))
        .map(|t| t.id)
}

/// The line a prerequisite's body names the ticket it unblocks with.
pub fn unblocks_line(blocked: TicketId) -> String {
    format!("Unblocks T-{blocked}")
}

/// A prerequisite's title: `what`, its first line, cut short.
pub fn title(what: &str) -> String {
    let line = what.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    if line.chars().count() <= 60 {
        return line.to_owned();
    }
    let kept: String = line.chars().take(59).collect();
    format!("{}…", kept.trim_end())
}

/// A prerequisite's body: what it must provide, and what waits on it.
pub fn body(missing: &BlockedOn, blocked: &Ticket, by: &str, note: &str) -> String {
    let note = note.trim();
    let why = if note.is_empty() { String::new() } else { format!("\n\n{by}'s note: {note}") };
    format!(
        "{what}\n\n{line} — \"{title}\": it can't go on without this, and carries on once this lands.{why}",
        what = missing.what,
        line = unblocks_line(blocked.id),
        title = blocked.title,
    )
}

/// Whether a pass's note says something the work needs doesn't exist: a
/// word for a missing thing and a word for an interface or a right, both
/// from `blocked.missing_terms` and `blocked.dependency_terms` (every
/// language's, in the locale files — members write in the person's).
pub fn says_missing(note: &str) -> bool {
    says_missing_in(note, crate::i18n::every("blocked.missing_terms"), crate::i18n::every("blocked.dependency_terms"))
}

fn says_missing_in(note: &str, missing: &[&str], dependency: &[&str]) -> bool {
    let lower = note.to_lowercase();
    let has = |terms: &[&str]| terms.iter().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).any(|t| contains_term(&lower, &t));
    has(missing) && has(dependency)
}

/// `term` in `text` as a word of its own where the term is spelled in
/// letters with spaces between words (`api`, not `rapid`); anywhere for a
/// term in a script without spaces.
fn contains_term(text: &str, term: &str) -> bool {
    let spaced = term.chars().all(|c| c.is_ascii() || !c.is_alphabetic() || is_latin(c));
    if !spaced {
        return text.contains(term);
    }
    let mut from = 0;
    while let Some(at) = text[from..].find(term) {
        let start = from + at;
        let end = start + term.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        // Next to a letter of a script without spaces (a particle right after "API") it is a word all the same.
        let edge = |c: Option<char>| c.is_none_or(|c| !(c.is_ascii_digit() || is_latin(c)));
        if edge(before) && edge(after) {
            return true;
        }
        from = start + term.chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// A letter of a script written with spaces between words, as far as the
/// terms go: Latin, with its accents.
fn is_latin(c: char) -> bool {
    matches!(c, 'a'..='z' | 'A'..='Z' | '\u{00C0}'..='\u{024F}')
}

/// Whether a pass of `ticket` now — its note `note` — makes a relay on
/// missing work: it says something is missing, and at least one pass before
/// it did, with no prerequisite named and no card put to the PM yet.
/// `history` is its ticket events, `(kind, text)`, oldest first.
pub fn relay_on_missing<'a>(note: &str, history: impl IntoIterator<Item = (&'a str, &'a str)>) -> bool {
    if !says_missing(note) {
        return false;
    }
    let mut before = 0;
    for (kind, text) in history {
        if kind == BLOCKED_ON || kind == SUSPECTED {
            return false;
        }
        if PASSES.contains(&kind) && says_missing(text) {
            before += 1;
        }
    }
    before + 1 >= RELAY_AT
}

/// The role a relay's notes most likely wait on, of those the squad has:
/// the backend, for an API, an endpoint or a contract, when it has one.
pub fn likely_role(roles: &[String]) -> Option<String> {
    [crate::roles::ENGINEER, "data"].into_iter().find(|r| roles.iter().any(|have| crate::board::role_of(have) == *r)).map(str::to_owned)
}

/// What the PM is told of a relay caught by its notes: one card — make the
/// missing ticket, or reshape or cancel this one.
pub fn pm_card(ticket: &Ticket, passes: usize, note: &str, role: Option<&str>) -> String {
    let role = role.unwrap_or("the role whose work it is");
    format!(
        "T-{id} \"{title}\" is blocked on missing work: it changed hands {passes} times, each note saying something it needs doesn't \
exist yet — the last: \"{note}\". It is not passed on again; it waits for you. Create the missing ticket? Yes: `update_ticket` \
with `id` {id} and `blocked_on` {{ \"role\": \"{role}\", \"what\": \"<the missing piece in a line>\" }} — equipe makes that ticket \
(or finds it on the board) under the same request, and T-{id} waits on it and goes back to its member once it lands. No: reshape \
T-{id} so it doesn't need it (`update_ticket`, then status todo), or cancel it.",
        id = ticket.id,
        title = ticket.title,
        note = note.trim(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::TicketKind;
    use serde_json::json;

    fn ticket(id: TicketId, title: &str, owner: &str, status: TicketStatus) -> Ticket {
        Ticket { summary: None,
            id,
            title: title.into(),
            body: String::new(),
            status,
            kind: TicketKind::Code,
            label: None,
            owner: Some(owner.into()),
            assignee: None,
            project: None,
            depends_on: vec![],
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            remaining: vec![],
            images: vec![],
            priority: Default::default(),
            rank: 0,
            updated_at: None,
            started_at: None,
            app: None,
            design_qa: None,
            landing: false,
            plan: None,
            model: None,
            flow: None,
        }
    }

    fn missing(role: &str, what: &str) -> BlockedOn {
        BlockedOn { role: role.into(), what: what.into(), project: None }
    }

    #[test]
    fn blocked_on_names_a_role_and_what_is_missing_or_says_what_to_fix() {
        assert_eq!(parse(&json!({ "reason": "cannot" })), Ok(None));
        assert_eq!(parse(&json!({ "blocked_on": null })), Ok(None));
        assert_eq!(
            parse(&json!({ "blocked_on": { "role": " backend ", "what": "Admin status endpoint", "project": "api" } })),
            Ok(Some(BlockedOn { role: "backend".into(), what: "Admin status endpoint".into(), project: Some("api".into()) }))
        );
        assert!(parse(&json!({ "blocked_on": { "what": "x" } })).unwrap_err().contains("`role`"));
        assert!(parse(&json!({ "blocked_on": { "role": "backend", "what": "  " } })).unwrap_err().contains("`what`"));
        assert_eq!(schema()["required"], json!(["role", "what"]));
    }

    #[test]
    fn the_prerequisite_on_the_board_is_found_before_another_is_made() {
        let mut blocked = ticket(10, "Admin-only MCP delete", "frontend", TicketStatus::InProgress);
        let want = missing("backend", "Read-only admin status endpoint after a lost response");
        let mut endpoint = ticket(11, "Admin status endpoint, read-only", "backend-2", TicketStatus::Todo);
        endpoint.body = "Expose a read-only admin status endpoint, read again after a lost response.".into();
        let unrelated = ticket(12, "Rate limits on login", "backend", TicketStatus::Todo);
        let done = ticket(13, "Read-only admin status endpoint after a lost response", "backend", TicketStatus::Done);
        let frontend = ticket(14, "Read-only admin status endpoint after a lost response", "frontend", TicketStatus::Todo);
        let board = vec![blocked.clone(), endpoint.clone(), unrelated.clone(), done, frontend];
        // In the same request and saying the same: that one.
        assert_eq!(find_prerequisite(&board, &blocked, &[10, 11, 12], &want), Some(11));
        // Outside the request, nothing said it was for this ticket: a new one is made.
        assert_eq!(find_prerequisite(&board, &blocked, &[10], &want), None);
        // Made for it before: found wherever it is.
        let mut made = endpoint.clone();
        made.body = format!("Read-only admin status endpoint after a lost response\n\n{} — \"Admin-only MCP delete\"", unblocks_line(10));
        let board = vec![blocked.clone(), made, unrelated.clone()];
        assert_eq!(find_prerequisite(&board, &blocked, &[10], &want), Some(11));
        // What it waits on already, of that role: that one, whatever it says.
        blocked.depends_on = vec![12];
        assert_eq!(find_prerequisite(&[blocked.clone(), unrelated], &blocked, &[], &want), Some(12));
    }

    #[test]
    fn a_prerequisite_reads_as_what_it_provides_and_what_waits_on_it() {
        let blocked = ticket(10, "Admin-only MCP delete", "frontend", TicketStatus::InProgress);
        let want = missing("backend", "Read-only admin status endpoint\nso the UI can recover");
        assert_eq!(title(&want.what), "Read-only admin status endpoint");
        assert_eq!(title(&"x".repeat(80)).chars().count(), 60);
        let body = body(&want, &blocked, "frontend", "The contract has no such endpoint.");
        assert!(body.starts_with("Read-only admin status endpoint") && body.contains("Unblocks T-10") && body.contains("frontend's note: The contract has no such endpoint."), "{body}");
    }

    #[test]
    fn a_note_saying_an_interface_is_missing_is_told_from_one_that_isnt() {
        let missing = ["missing", "no such", "doesn't exist", "does not exist", "not exist"];
        let dependency = ["api", "endpoint", "contract", "permission", "route"];
        let says = |note: &str| says_missing_in(note, &missing, &dependency);
        assert!(says("The server contract has no such endpoint for the admin status."));
        assert!(says("Blocked: the API for this doesn't exist yet."));
        assert!(says("Missing permission: only owners may delete."));
        assert!(!says("The endpoint works; I added tests."), "nothing missing");
        assert!(!says("Missing a test for the empty state."), "nothing of an interface");
        assert!(!says("Rapid fix: no such issue in the contracts folder"), "`rapid` is no `api`, `contracts` no `contract`");
        // A script without spaces between words: anywhere in the text.
        assert!(says_missing_in("服务器没有这个接口", &["没有"], &["接口"]));
        // A Latin term against another script's letters is a word of its own.
        assert!(says_missing_in("서버 계약에 해당 API가 없음", &["없음"], &["api"]));
        assert!(says_missing("서버 계약에 해당 엔드포인트가 없습니다"), "the locale files' own words");
    }

    #[test]
    fn the_second_pass_that_says_something_is_missing_is_a_relay_once() {
        let missing = "The server contract has no such endpoint.";
        // The locale files' words, every language's.
        assert!(says_missing(missing), "the locale files know the words");
        let first: Vec<(&str, &str)> = vec![("created", "Admin-only MCP delete"), ("assigned", "frontend")];
        assert!(!relay_on_missing(missing, first.clone()), "the first pass is a pass");
        let passed = format!("qa · {missing}");
        let mut second = first.clone();
        second.push(("baton", &passed));
        assert!(relay_on_missing(missing, second.clone()));
        assert!(!relay_on_missing("Done with the layout, over to QA.", second.clone()), "a pass that says nothing is missing");
        // Once named, or once put to the PM: never again.
        let mut named = second.clone();
        named.push((BLOCKED_ON, "T-12 (backend): admin status endpoint"));
        assert!(!relay_on_missing(missing, named));
        let mut asked = second;
        asked.push((SUSPECTED, missing));
        assert!(!relay_on_missing(missing, asked));
    }

    #[test]
    fn the_pm_card_offers_the_missing_ticket_with_the_role_the_squad_has() {
        let blocked = ticket(10, "Admin-only MCP delete", "frontend", TicketStatus::Todo);
        assert_eq!(likely_role(&["pm".into(), "frontend".into(), "backend-2".into()]), Some("engineer".into()), "the software engineer builds the API");
        assert_eq!(likely_role(&["pm".into(), "designer".into()]), None);
        let card = pm_card(&blocked, 3, "The contract has no such endpoint.", Some("backend"));
        for wanted in ["T-10", "3 times", "Create the missing ticket?", "\"role\": \"backend\"", "`blocked_on`", "cancel it"] {
            assert!(card.contains(wanted), "{wanted} in: {card}");
        }
        assert!(pm_card(&blocked, 2, "x", None).contains("the role whose work it is"));
    }
}
