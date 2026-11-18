//! The person's requests ([`equipe_protocol::requests`]): the rules, with no
//! runtime in them, so each is tested alone — where a request stands
//! ([`status`]), the line that says so ([`line`]), when that is news worth a
//! word in its conversation ([`news`]), what a member is told of them at the
//! start of its turn ([`context`]), and whether a message asks after one
//! ([`asked_after`]) — and where requests are kept ([`Store`]).
//!
//! The runtime's side — recording a request, attaching tickets by their
//! lineage, telling the conversation — is `runtime::requests`.

use std::collections::{BTreeMap, BTreeSet};

use equipe_protocol::requests::{Link, Milestone, Part, Phase, Request, RequestId, RequestTicket, Status, Step, Wait};
use equipe_protocol::{ExecutionState, SessionId, Ticket, TicketId, TicketKind, TicketStatus};
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::i18n::Catalog;

/// News waits this long after the first change it tells of, so whatever
/// else moves meanwhile goes into the same one line (unix seconds).
pub const SETTLE: u64 = 120;

/// [`SETTLE`] — or, where `EQUIPE_REQUEST_SETTLE` says, that many seconds
/// (tests, which can't wait two minutes).
pub fn settle() -> u64 {
    std::env::var("EQUIPE_REQUEST_SETTLE").ok().and_then(|v| v.parse().ok()).unwrap_or(SETTLE)
}

/// A ticket a member puts on the board this long after the person spoke to
/// it, between turns, still answers what they said.
pub const ASKED_WITHIN: u64 = 20 * 60;

/// A finished request stays listed this long.
pub const LISTED_DONE: u64 = 24 * 60 * 60;

/// At most this many characters of the person's words name a request.
const TITLE_CHARS: usize = 60;

/// A part's title, and a wait's, at most this long in a status line.
const PART_CHARS: usize = 36;
const WAIT_CHARS: usize = 28;

/// At most this many parts, and waits, are named in a status line.
const PARTS_SHOWN: usize = 4;
const WAITS_SHOWN: usize = 3;

/// What the person said, as a request's name: its first line that says
/// something, cut short.
pub fn title_of(words: &str) -> String {
    let line = words.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    cut(line, TITLE_CHARS)
}

/// `text`, at most `max` characters, `…` where it was cut.
fn cut(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

fn who(t: &Ticket) -> Option<String> {
    t.assignee.clone().or_else(|| t.owner.clone())
}

fn stalled(t: &Ticket) -> bool {
    t.status == TicketStatus::InProgress && t.execution.as_ref().is_some_and(|e| e.state == ExecutionState::Stalled)
}

/// Its work is on the main branch.
fn live(t: &Ticket) -> bool {
    t.status == TicketStatus::Done && t.kind == TicketKind::Code && t.merged == Some(true)
}

/// The request's ticket each of `links` belongs under: its first ticket
/// whose parent isn't in the request (or that has none) — the part it is in.
pub fn part_of(links: &[RequestTicket], ticket: TicketId) -> TicketId {
    let mut at = ticket;
    // A loop of parents is cut where it closes.
    for _ in 0..links.len() {
        match links.iter().find(|l| l.ticket == at).and_then(|l| l.parent).filter(|p| links.iter().any(|l| l.ticket == *p)) {
            Some(parent) if parent != ticket => at = parent,
            _ => break,
        }
    }
    at
}

/// Where a request stands: each part — a ticket it asked for and what came
/// of it — how far its work got; what of it waits on the person, apart; and
/// its tickets in order. `tickets` holds them (any others are ignored; one
/// missing is left out); `asking`, the tickets with a question open to the
/// person. equipe's own checks show in the timeline only.
pub fn status(links: &[RequestTicket], tickets: &[Ticket], asking: &[TicketId]) -> Status {
    let find = |id: TicketId| tickets.iter().find(|t| t.id == id);
    let mut steps = Vec::new();
    for link in links {
        let Some(t) = find(link.ticket) else { continue };
        steps.push(Step {
            ticket: t.id,
            title: t.title.clone(),
            status: t.status,
            link: link.link,
            parent: link.parent,
            who: who(t),
            at: t.updated_at,
            merged: live(t),
        });
    }
    let counted: Vec<(&RequestTicket, &Ticket)> = links.iter().filter(|l| l.link != Link::Check).filter_map(|l| find(l.ticket).map(|t| (l, t))).collect();
    let waits: Vec<Wait> = counted
        .iter()
        .filter(|(_, t)| t.status == TicketStatus::Waiting || (!t.status.is_closed() && asking.contains(&t.id)))
        .map(|(_, t)| Wait { ticket: t.id, title: t.title.clone(), who: who(t) })
        .collect();
    let mut roots: Vec<TicketId> = Vec::new();
    for (l, _) in &counted {
        let root = part_of(links, l.ticket);
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    let mut parts = Vec::new();
    for root in roots {
        let mine: Vec<&Ticket> = counted.iter().filter(|(l, _)| part_of(links, l.ticket) == root).map(|(_, t)| *t).collect();
        let Some(head) = find(root).or(mine.first().copied()) else { continue };
        let (phase, who, since) = phase(&mine, asking);
        parts.push(Part { ticket: head.id, title: head.title.clone(), phase, who, since });
    }
    let done = !parts.is_empty() && waits.is_empty() && parts.iter().all(|p| p.phase.is_finished());
    Status { parts, waits, steps, done }
}

/// How far a part's tickets got: the open ones that don't wait on the
/// person say it — stopped, worked on, in review, next — and once none is
/// open, what was done says it: live, accepted, done.
fn phase(tickets: &[&Ticket], asking: &[TicketId]) -> (Phase, Option<String>, Option<u64>) {
    let kept: Vec<&&Ticket> = tickets.iter().filter(|t| t.status != TicketStatus::Cancelled).collect();
    if kept.is_empty() {
        return (Phase::Cancelled, None, tickets.iter().filter_map(|t| t.updated_at).max());
    }
    let waits = |t: &Ticket| t.status == TicketStatus::Waiting || asking.contains(&t.id);
    let open: Vec<&&Ticket> = kept.iter().copied().filter(|t| !t.status.is_closed() && !waits(t)).collect();
    if let Some(t) = open.iter().find(|t| stalled(t)) {
        return (Phase::Blocked, who(t), t.updated_at);
    }
    if let Some(t) = open.iter().find(|t| t.status == TicketStatus::InProgress) {
        return (Phase::Working, who(t), None);
    }
    if let Some(t) = open.iter().find(|t| t.status == TicketStatus::Review) {
        return (Phase::Review, who(t), t.updated_at);
    }
    if let Some(t) = open.first() {
        return (Phase::Queued, who(t), None);
    }
    let done: Vec<&&Ticket> = kept.iter().copied().filter(|t| t.status == TicketStatus::Done).collect();
    if done.is_empty() {
        return (Phase::Waiting, None, None);
    }
    let latest = |list: &[&&Ticket]| list.iter().filter_map(|t| t.updated_at).max();
    let code: Vec<&&Ticket> = done.iter().copied().filter(|t| t.kind == TicketKind::Code).collect();
    if code.iter().any(|t| t.merged == Some(false)) {
        return (Phase::Accepted, None, latest(&code));
    }
    let merged: Vec<&&Ticket> = code.iter().copied().filter(|t| t.merged == Some(true)).collect();
    if !merged.is_empty() {
        return (Phase::Live, None, latest(&merged));
    }
    (Phase::Done, None, latest(&done))
}

// ── the line ──

/// A time as a status line says it: `07:58` today, else with its day.
pub fn clock(unix: u64, now: u64) -> String {
    let Some(tm) = equipe_providers::platform::local_time(unix) else { return String::new() };
    let today = equipe_providers::platform::local_time(now).is_some_and(|n| (n.year, n.month, n.day) == (tm.year, tm.month, tm.day));
    if today { format!("{:02}:{:02}", tm.hour, tm.minute) } else { format!("{:02}-{:02} {:02}:{:02}", tm.month, tm.day, tm.hour, tm.minute) }
}

/// Where a request stands, in one line in `words`' language: each part and
/// how far it got, then what waits on the person — or that it is all done.
/// `clock` says a time.
pub fn line(status: &Status, words: &Catalog, clock: &dyn Fn(u64) -> String) -> String {
    let mut out: Vec<String> = Vec::new();
    for part in status.parts.iter().take(PARTS_SHOWN) {
        let title = cut(&part.title, PART_CHARS);
        let time = part.since.map(clock).unwrap_or_default();
        let key = match part.phase {
            Phase::Queued => "request.phase.queued",
            Phase::Working if part.who.is_some() => "request.phase.working",
            Phase::Working => "request.phase.working_nobody",
            Phase::Review => "request.phase.review",
            Phase::Blocked => "request.phase.blocked",
            Phase::Waiting => "request.phase.waiting",
            Phase::Accepted => "request.phase.accepted",
            Phase::Live if time.is_empty() => "request.phase.live_now",
            Phase::Live => "request.phase.live",
            Phase::Done => "request.phase.done",
            Phase::Cancelled => "request.phase.cancelled",
        };
        out.push(words.fill(key, &[("title", title), ("who", part.who.clone().unwrap_or_default()), ("time", time)]));
    }
    if status.parts.len() > PARTS_SHOWN {
        out.push(words.fill("request.more", &[("count", (status.parts.len() - PARTS_SHOWN).to_string())]));
    }
    if !status.waits.is_empty() {
        let mut list: Vec<String> = status.waits.iter().take(WAITS_SHOWN).map(|w| cut(&w.title, WAIT_CHARS)).collect();
        if status.waits.len() > WAITS_SHOWN {
            list.push("…".into());
        }
        out.push(words.fill("request.waits", &[("count", status.waits.len().to_string()), ("list", list.join(", "))]));
    }
    if status.done {
        out.push(words.get("request.all_done").to_owned());
    }
    if out.is_empty() {
        return words.get("request.nothing_yet").to_owned();
    }
    out.join(" · ")
}

// ── news ──

/// What has been told of a request's tickets: for each, the marks
/// ([`mark`]) its conversation heard, and whether "all done" was told —
/// kept as `12:wd,14:l,*`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Told {
    pub marks: BTreeMap<TicketId, BTreeSet<char>>,
    pub all_done: bool,
}

impl Told {
    pub fn parse(text: &str) -> Self {
        let mut told = Told::default();
        for item in text.split(',').map(str::trim).filter(|i| !i.is_empty()) {
            if item == "*" {
                told.all_done = true;
            } else if let Some((id, marks)) = item.split_once(':')
                && let Ok(id) = id.parse()
            {
                told.marks.entry(id).or_default().extend(marks.chars());
            }
        }
        told
    }

    pub fn code(&self) -> String {
        let mut items: Vec<String> = self.marks.iter().filter(|(_, m)| !m.is_empty()).map(|(id, m)| format!("{id}:{}", m.iter().collect::<String>())).collect();
        if self.all_done {
            items.push("*".into());
        }
        items.join(",")
    }
}

/// What a ticket's state says that is worth telling the person: `w` it
/// waits on them, `b` its run stopped, `l` its work is on main, `d` done.
/// Being worked on, in review, sent back and handed in again: no news.
pub fn mark(t: &Ticket, asking: bool) -> Option<char> {
    match t.status {
        TicketStatus::Waiting => Some('w'),
        _ if asking && !t.status.is_closed() => Some('w'),
        _ if stalled(t) => Some('b'),
        TicketStatus::Done if live(t) => Some('l'),
        TicketStatus::Done => Some('d'),
        _ => None,
    }
}

/// The marks a mark implies were passed on the way: live work was done.
fn with_passed(mark: char) -> &'static [char] {
    match mark {
        'l' => &['l', 'd'],
        'd' => &['d'],
        'w' => &['w'],
        'b' => &['b'],
        _ => &[],
    }
}

/// Whether a request has news since `told`: its milestone — the most
/// telling of what is new — and what is told once it is said. Each mark of
/// each ticket is told once; equipe's own checks never are; "all done" once.
pub fn news(told: &Told, links: &[RequestTicket], tickets: &[Ticket], asking: &[TicketId], status: &Status) -> Option<(Milestone, Told)> {
    let mut next = told.clone();
    let mut new: Vec<char> = Vec::new();
    for link in links.iter().filter(|l| l.link != Link::Check) {
        let Some(t) = tickets.iter().find(|t| t.id == link.ticket) else { continue };
        let Some(m) = mark(t, asking.contains(&t.id)) else { continue };
        let seen = next.marks.entry(t.id).or_default();
        if !seen.contains(&m) {
            new.push(m);
            seen.extend(with_passed(m));
        }
    }
    let milestone = if status.done && !told.all_done {
        next.all_done = true;
        Some(Milestone::AllDone)
    } else if new.contains(&'w') {
        Some(Milestone::NeedsYou)
    } else if new.contains(&'b') {
        Some(Milestone::Blocked)
    } else if new.contains(&'l') {
        Some(Milestone::Live)
    } else if new.contains(&'d') {
        Some(Milestone::Done)
    } else {
        None
    };
    milestone.map(|m| (m, next))
}

// ── what a member is told ──

/// What a member is told of the person's open requests in the conversation
/// its turn is in, at the turn's start (English, as every word to a model):
/// so "how is X going?" is answered at once, and right. `None` without any.
pub fn context(requests: &[Request], now: u64) -> Option<String> {
    let open: Vec<&Request> = requests.iter().filter(|r| r.closed_at.is_none()).collect();
    if open.is_empty() {
        return None;
    }
    let words = crate::i18n::core(crate::i18n::Lang::En);
    let lines: Vec<String> = open
        .iter()
        .map(|r| {
            let tickets: Vec<String> = r.tickets.iter().filter(|l| l.link != Link::Check).map(|l| format!("T-{}", l.ticket)).collect();
            format!("- R-{} “{}” ({}): {}", r.id, r.title, tickets.join(", "), line(&r.status, words, &|at| clock(at, now)))
        })
        .collect();
    Some(format!(
        "[equipe: what the person asked for in this conversation, and where each stands now (from the board):\n{}\n\
When they ask how one is going, answer from this at once — never \"at the next step\" — and name what waits on them. \
A ticket you put on the board for one of these that equipe wouldn't link by itself: give its R-number as `request`.]",
        lines.join("\n")
    ))
}

// ── a message asking after a request ──

/// Han, Hangul and Katakana: scripts written without spaces between words,
/// matched two characters at a time.
fn dense(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xAC00..=0xD7A3 | 0x30A0..=0x30FF)
}

/// Whether `text` and `target` share a term: two characters of a dense
/// script side by side, or a word's first four letters.
fn shares_term(text: &str, target: &str) -> bool {
    let target = target.to_lowercase();
    let target_words: Vec<&str> = target.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    for word in text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        let chars: Vec<char> = word.chars().collect();
        if chars.windows(2).any(|pair| pair.iter().all(|c| dense(*c)) && target.contains(&pair.iter().collect::<String>())) {
            return true;
        }
        let letters: Vec<char> = chars.iter().copied().filter(|c| !dense(*c)).collect();
        if letters.len() >= 4 && letters.iter().all(|c| c.is_alphabetic()) {
            let stem: String = letters[..4].iter().collect();
            if target_words.iter().any(|w| w.starts_with(&stem)) {
                return true;
            }
        }
    }
    false
}

/// The ticket numbers `text` names (`T-12`).
fn ticket_numbers(text: &str) -> Vec<TicketId> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    for i in 0..chars.len() {
        let starts = matches!(chars[i], 'T' | 't') && chars.get(i + 1) == Some(&'-') && (i == 0 || !chars[i - 1].is_alphanumeric());
        if starts {
            let digits: String = chars[i + 2..].iter().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse() {
                out.push(n);
            }
        }
    }
    out
}

/// The open requests `text` — a message the person sent while its member
/// works — asks after: one it names by a ticket or R-number, or by a term
/// of its words or its parts', in a question (or a message short enough to
/// be one). Empty when it asks after none.
pub fn asked_after<'a>(text: &str, requests: &'a [Request]) -> Vec<&'a Request> {
    let text = text.trim();
    let numbers = ticket_numbers(text);
    let questionish = text.contains(['?', '？']) || text.chars().count() <= 60;
    requests
        .iter()
        .filter(|r| r.closed_at.is_none())
        .filter(|r| {
            let named = r.tickets.iter().any(|l| numbers.contains(&l.ticket)) || text.contains(&format!("R-{}", r.id));
            named || (questionish && {
                let target = std::iter::once(r.title.as_str()).chain(r.status.parts.iter().map(|p| p.title.as_str())).collect::<Vec<_>>().join(" ");
                shares_term(text, &target)
            })
        })
        .collect()
}

// ── what a request's ticket may wait on ──

/// The places a ticket's words name — `design/directions/`, `src/app/x.tsx`
/// — each by its first folder (or its name, for a file at the top).
pub fn areas(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .split(|c: char| c.is_whitespace() || matches!(c, '`' | '(' | ')' | '[' | ']' | ',' | ';' | '"' | '\'' | '“' | '”'))
        .map(|w| w.trim_end_matches(['.', ':', '!', '?']))
        .filter(|w| w.contains('/') && !w.contains("://") && w.chars().any(char::is_alphabetic) && !w.starts_with('/'))
        .filter_map(|w| w.trim_start_matches("./").split('/').find(|p| !p.is_empty()).map(|p| p.to_lowercase()))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Whether two tickets' words name overlapping places: `None` when either
/// names none — nothing to tell by.
pub fn overlap(a: &str, b: &str) -> Option<bool> {
    let (a, b) = (areas(a), areas(b));
    if a.is_empty() || b.is_empty() {
        return None;
    }
    Some(a.iter().any(|x| b.contains(x)))
}

/// Why a ticket can't wait on `dep`, if it can't: one serving the person's
/// request `serves` isn't chained behind open work outside that request
/// without a reason. (English, for the member who asked.)
pub fn chain_refusal(ticket: Option<TicketId>, serves: Option<RequestId>, dep: &Ticket, dep_serves: Option<RequestId>) -> Option<String> {
    if dep.status.is_closed() || (serves.is_some() && dep_serves == serves) {
        return None;
    }
    let what = ticket.map_or_else(|| "This ticket".to_owned(), |t| format!("T-{t}"));
    let request = serves.map_or_else(|| "what the person just asked for".to_owned(), |r| format!("the person's request R-{r}"));
    Some(format!(
        "{what} serves {request}; T-{} ({}) is work outside it, still open. A request's ticket isn't chained behind other \
work without a reason: leave T-{} out of depends_on — or, if it truly must wait, say why in `depends_why`.",
        dep.id, dep.title, dep.id
    ))
}

/// What a member is told when a dependency names places that don't
/// overlap with its ticket's (`None` when they do, or can't be told).
pub fn overlap_warning(text: &str, dep: &Ticket) -> Option<String> {
    (overlap(text, &format!("{} {}", dep.title, dep.body)) == Some(false)).then(|| {
        format!(
            " Note: it now waits on T-{}, whose work is in {} while this one's is in {} — nothing overlaps. If it needn't wait, drop the dependency (update_ticket, or `unblock` on your own ticket).",
            dep.id,
            areas(&format!("{} {}", dep.title, dep.body)).join(", "),
            areas(text).join(", ")
        )
    })
}

// ── where requests are kept ──

/// A request as kept, before its status is worked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: RequestId,
    pub title: String,
    pub words: String,
    pub session: Option<SessionId>,
    pub member: Option<String>,
    pub asked_at: u64,
    pub told: Told,
    /// News waits to be told from then on.
    pub due_at: Option<u64>,
    pub closed_at: Option<u64>,
    pub tickets: Vec<RequestTicket>,
}

impl Row {
    /// As a screen gets it, with where it stands.
    pub fn view(&self, status: Status) -> Request {
        Request {
            id: self.id,
            title: self.title.clone(),
            session: self.session,
            member: self.member.clone(),
            asked_at: self.asked_at,
            tickets: self.tickets.clone(),
            status,
            closed_at: self.closed_at,
        }
    }
}

/// The tables, made when the journal opens.
pub fn create_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS requests (
           id INTEGER PRIMARY KEY,
           title TEXT NOT NULL,
           words TEXT NOT NULL,
           session INTEGER,
           member TEXT,
           asked_at INTEGER NOT NULL,
           told TEXT NOT NULL DEFAULT '',
           due_at INTEGER,
           closed_at INTEGER
         );
         CREATE TABLE IF NOT EXISTS request_tickets (
           ticket INTEGER PRIMARY KEY,
           request INTEGER NOT NULL,
           parent INTEGER,
           link TEXT NOT NULL,
           at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS request_tickets_request ON request_tickets(request);",
    )
}

/// The squad's requests in its journal.
pub struct Store<'a>(pub(crate) &'a Connection);

impl Store<'_> {
    /// Keep a new request; its number.
    pub fn create(&self, title: &str, words: &str, session: Option<SessionId>, member: Option<&str>, asked_at: u64) -> rusqlite::Result<RequestId> {
        let words: String = words.chars().take(4000).collect();
        self.0.execute(
            "INSERT INTO requests (title, words, session, member, asked_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![title, words, session.map(|s| s as i64), member, asked_at as i64],
        )?;
        Ok(self.0.last_insert_rowid() as RequestId)
    }

    /// `ticket` serves `request` now (a ticket serves one at most), and the
    /// request is open again if it was closed.
    pub fn link(&self, request: RequestId, ticket: TicketId, parent: Option<TicketId>, link: Link, at: u64) -> rusqlite::Result<()> {
        self.0.execute(
            "INSERT OR REPLACE INTO request_tickets (ticket, request, parent, link, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ticket, request as i64, parent, link.code(), at as i64],
        )?;
        self.0.execute("UPDATE requests SET closed_at = NULL WHERE id = ?1", [request as i64])?;
        Ok(())
    }

    /// The open tickets the person's open requests are waiting for — what they
    /// asked, what members made for it, what was linked to it; not equipe's
    /// checks nor design follow-ups — for the queue to take first.
    pub fn serving(&self) -> Vec<TicketId> {
        let Ok(mut stmt) = self.0.prepare(
            "SELECT rt.ticket FROM request_tickets rt JOIN requests r ON r.id = rt.request
             WHERE r.closed_at IS NULL AND rt.link IN ('asked', 'made', 'linked') ORDER BY r.asked_at, rt.at",
        ) else {
            return Vec::new();
        };
        stmt.query_map([], |r| r.get(0)).and_then(|rows| rows.collect()).unwrap_or_default()
    }

    /// How `ticket` joined the request it serves, if it serves one.
    pub fn link_of(&self, ticket: TicketId) -> Option<Link> {
        self.0.query_row("SELECT link FROM request_tickets WHERE ticket = ?1", [ticket], |r| r.get::<_, String>(0)).optional().ok().flatten().and_then(|l| Link::parse(&l))
    }

    /// The request `ticket` serves, if any.
    pub fn of_ticket(&self, ticket: TicketId) -> Option<RequestId> {
        self.0.query_row("SELECT request FROM request_tickets WHERE ticket = ?1", [ticket], |r| r.get::<_, i64>(0)).optional().ok().flatten().map(|n| n as RequestId)
    }

    pub fn exists(&self, request: RequestId) -> bool {
        self.0.query_row("SELECT 1 FROM requests WHERE id = ?1", [request as i64], |_| Ok(())).optional().ok().flatten().is_some()
    }

    fn links(&self, request: RequestId) -> Vec<RequestTicket> {
        let Ok(mut stmt) = self.0.prepare_cached("SELECT ticket, parent, link FROM request_tickets WHERE request = ?1 ORDER BY at, ticket") else { return Vec::new() };
        stmt.query_map([request as i64], |r| {
            Ok(RequestTicket { ticket: r.get(0)?, parent: r.get(1)?, link: Link::parse(&r.get::<_, String>(2)?).unwrap_or(Link::Linked) })
        })
        .and_then(|rows| rows.collect())
        .unwrap_or_default()
    }

    fn rows(&self, filter: &str, args: impl rusqlite::Params) -> Vec<Row> {
        let Ok(mut stmt) = self.0.prepare(&format!("SELECT id, title, words, session, member, asked_at, told, due_at, closed_at FROM requests {filter}")) else {
            return Vec::new();
        };
        let rows: Vec<Row> = stmt
            .query_map(args, |r| {
                Ok(Row {
                    id: r.get::<_, i64>(0)? as RequestId,
                    title: r.get(1)?,
                    words: r.get(2)?,
                    session: r.get::<_, Option<i64>>(3)?.map(|s| s as SessionId),
                    member: r.get(4)?,
                    asked_at: r.get::<_, i64>(5)?.max(0) as u64,
                    told: Told::parse(&r.get::<_, String>(6)?),
                    due_at: r.get::<_, Option<i64>>(7)?.map(|s| s.max(0) as u64),
                    closed_at: r.get::<_, Option<i64>>(8)?.map(|s| s.max(0) as u64),
                    tickets: Vec::new(),
                })
            })
            .and_then(|rows| rows.collect())
            .unwrap_or_default();
        rows.into_iter().map(|row| Row { tickets: self.links(row.id), ..row }).collect()
    }

    pub fn get(&self, request: RequestId) -> Option<Row> {
        self.rows("WHERE id = ?1", [request as i64]).into_iter().next()
    }

    /// Those still open, and those closed since `since`, the latest first.
    pub fn listed(&self, since: u64) -> Vec<Row> {
        self.rows("WHERE closed_at IS NULL OR closed_at >= ?1 ORDER BY asked_at DESC, id DESC", [since as i64])
    }

    /// Those with news waiting to be told by `now`.
    pub fn due(&self, now: u64) -> Vec<Row> {
        self.rows("WHERE due_at IS NOT NULL AND due_at <= ?1 ORDER BY due_at, id", [now as i64])
    }

    /// News waits to be told from `at`, unless it waits already.
    pub fn set_due(&self, request: RequestId, at: u64) -> rusqlite::Result<()> {
        self.0.execute("UPDATE requests SET due_at = ?2 WHERE id = ?1 AND due_at IS NULL", params![request as i64, at as i64])?;
        Ok(())
    }

    /// The tickets the PM accepted (`true`) or sent back (`false`) since
    /// `since` — or their own member landed, its gates passed (accepted,
    /// with who) — the latest first: each once, at its latest.
    pub fn decided_by_pm(&self, since: u64) -> Vec<(TicketId, u64, bool, Option<String>)> {
        let Ok(mut stmt) = self.0.prepare(
            "SELECT ticket, MAX(at), kind, by FROM ticket_events
             WHERE at >= ?1 AND (kind = 'accepted by the PM' OR kind = ?2 OR (kind = 'sent back' AND by != 'equipe' AND by != 'you'))
             GROUP BY ticket ORDER BY MAX(at) DESC LIMIT 50",
        ) else {
            return Vec::new();
        };
        stmt.query_map(params![since as i64, crate::lead_time::SELF_LANDED], |r| {
            let kind: String = r.get(2)?;
            let landed = (kind == crate::lead_time::SELF_LANDED).then(|| r.get::<_, String>(3)).transpose()?;
            Ok((r.get::<_, TicketId>(0)?, r.get::<_, i64>(1)?.max(0) as u64, kind != "sent back", landed))
        })
        .and_then(|rows| rows.collect())
        .unwrap_or_default()
    }

    /// What was told, and that nothing waits to be; `closed`: when it was
    /// all done and told.
    pub fn told(&self, request: RequestId, told: &Told, closed: Option<u64>) -> rusqlite::Result<()> {
        self.0.execute(
            "UPDATE requests SET told = ?2, due_at = NULL, closed_at = COALESCE(?3, closed_at) WHERE id = ?1",
            params![request as i64, told.code(), closed.map(|c| c as i64)],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, core};

    fn ticket(id: TicketId, title: &str, status: TicketStatus) -> Ticket {
        Ticket {
            id,
            title: title.into(),
            body: String::new(),
            status,
            kind: TicketKind::Code,
            label: None,
            owner: Some("frontend".into()),
            assignee: None,
            project: None,
            depends_on: vec![],
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            created_by: "designer".into(),
            note: None,
            asks: vec![],
            remaining: vec![],
            images: vec![],
            priority: Default::default(),
            rank: 0,
            updated_at: Some(1_000),
            design_qa: None,
            landing: false,
            plan: None,
            model: None,
            flow: None,
        }
    }

    fn merged(mut t: Ticket, at: u64) -> Ticket {
        t.merged = Some(true);
        t.updated_at = Some(at);
        t
    }

    fn link(ticket: TicketId, parent: Option<TicketId>, link: Link) -> RequestTicket {
        RequestTicket { ticket, parent, link }
    }

    /// The owner's morning: the composer's spec and its build, a licence
    /// check, a phone check and a legal question waiting on the person, two
    /// design QAs; the left menu's spec and build.
    fn morning() -> (Vec<RequestTicket>, Vec<Ticket>) {
        let links = vec![
            link(3565, None, Link::Asked),
            link(3567, None, Link::Asked),
            link(3613, Some(3565), Link::Made),
            link(3614, Some(3565), Link::Made),
            link(3616, Some(3565), Link::Made),
            link(3619, Some(3565), Link::Made),
            link(3639, Some(3613), Link::Check),
            link(3591, Some(3567), Link::Made),
        ];
        let tickets = vec![
            merged(ticket(3565, "Spec the Tinumi composer", TicketStatus::Done), 900),
            ticket(3567, "Left menu, three options", TicketStatus::Done),
            merged(ticket(3613, "Apply the composer to Tinumi chat", TicketStatus::Done), 1_100),
            ticket(3614, "Confirm the composer source licence", TicketStatus::Done),
            ticket(3616, "Check the new composer on real phones", TicketStatus::Waiting),
            ticket(3619, "Decide if the layout needs legal review", TicketStatus::Waiting),
            ticket(3639, "Design QA: T-3613", TicketStatus::InProgress),
            ticket(3591, "Make the left menu a sidebar", TicketStatus::InProgress),
        ];
        (links, tickets)
    }

    #[test]
    fn a_request_reads_as_its_parts_and_what_waits_on_the_person() {
        let (links, tickets) = morning();
        let s = status(&links, &tickets, &[]);
        assert_eq!(s.parts.iter().map(|p| (p.ticket, p.phase)).collect::<Vec<_>>(), [(3565, Phase::Live), (3567, Phase::Working)]);
        assert_eq!(s.parts[0].since, Some(1_100), "live since its last merge");
        assert_eq!(s.waits.iter().map(|w| w.ticket).collect::<Vec<_>>(), [3616, 3619]);
        assert!(!s.done);
        assert_eq!(s.steps.len(), 8, "the design QA shows in the timeline");
        let words = core(Lang::En);
        let line = line(&s, words, &|at| format!("@{at}"));
        assert_eq!(
            line,
            "Spec the Tinumi composer — on main since @1100 · Left menu, three options — frontend is on it · 2 things wait for you (Check the new composer on r…, Decide if the layout needs…)"
        );
    }

    #[test]
    fn a_design_qa_never_moves_a_part_nor_is_news() {
        let (links, mut tickets) = morning();
        // Only the composer's chain: everything closed but its design QA, still running.
        tickets.retain(|t| [3565, 3613, 3639].contains(&t.id));
        let s = status(&links, &tickets, &[]);
        assert_eq!(s.parts.len(), 1);
        assert_eq!(s.parts[0].phase, Phase::Live, "{s:?}");
        assert!(s.done, "a running check doesn't hold the request open");
    }

    #[test]
    fn a_part_is_stopped_worked_reviewed_or_next_by_its_open_tickets() {
        let links = vec![link(1, None, Link::Asked), link(2, Some(1), Link::Made)];
        let mut a = ticket(1, "Spec", TicketStatus::Done);
        let mut b = ticket(2, "Build", TicketStatus::Todo);
        assert_eq!(status(&links, &[a.clone(), b.clone()], &[]).parts[0].phase, Phase::Queued);
        b.status = TicketStatus::Review;
        assert_eq!(status(&links, &[a.clone(), b.clone()], &[]).parts[0].phase, Phase::Review);
        b.status = TicketStatus::InProgress;
        b.assignee = Some("frontend-2".into());
        let s = status(&links, &[a.clone(), b.clone()], &[]);
        assert_eq!((s.parts[0].phase, s.parts[0].who.as_deref()), (Phase::Working, Some("frontend-2")));
        b.execution = Some(Box::new(equipe_protocol::TicketExecution {
            state: ExecutionState::Stalled,
            session: 1,
            operation: "work".into(),
            generation: 1,
            heartbeat_at: 0,
            lease_expires_at: 0,
            reason: None,
            pause: None,
            head: None,
            dirty: None,
            waited_for: None,
        }));
        assert_eq!(status(&links, &[a.clone(), b.clone()], &[]).parts[0].phase, Phase::Blocked);
        b.execution = None;
        b.status = TicketStatus::Done;
        b.merged = Some(false);
        assert_eq!(status(&links, &[a.clone(), b.clone()], &[]).parts[0].phase, Phase::Accepted);
        b.status = TicketStatus::Cancelled;
        a.status = TicketStatus::Cancelled;
        let s = status(&links, &[a, b], &[]);
        assert_eq!(s.parts[0].phase, Phase::Cancelled);
        assert!(s.done);
    }

    #[test]
    fn a_question_open_to_the_person_is_a_wait() {
        let links = vec![link(1, None, Link::Asked)];
        let t = ticket(1, "Claude icon in the picker", TicketStatus::InProgress);
        let s = status(&links, std::slice::from_ref(&t), &[1]);
        assert_eq!(s.waits.len(), 1);
        assert_eq!(s.parts[0].phase, Phase::Waiting);
        assert_eq!(mark(&t, true), Some('w'));
    }

    #[test]
    fn each_milestone_is_news_once_and_a_retry_is_none() {
        let links = vec![link(1, None, Link::Asked), link(2, Some(1), Link::Made), link(3, Some(2), Link::Check)];
        let mut tickets = vec![ticket(1, "Spec", TicketStatus::InProgress), ticket(2, "Build", TicketStatus::Todo), ticket(3, "Design QA", TicketStatus::Todo)];
        let check = |told: &Told, tickets: &[Ticket]| news(told, &links, tickets, &[], &status(&links, tickets, &[]));
        let told = Told::default();
        assert_eq!(check(&told, &tickets), None, "work under way is no news");
        tickets[0].status = TicketStatus::Done;
        let (m, told) = check(&told, &tickets).unwrap();
        assert_eq!(m, Milestone::Done);
        assert_eq!(check(&told, &tickets), None, "told once");
        // Built, handed in, sent back, handed in again; its design QA done: no news.
        tickets[1].status = TicketStatus::Review;
        tickets[2].status = TicketStatus::Done;
        assert_eq!(check(&told, &tickets), None);
        tickets[1].status = TicketStatus::InProgress;
        assert_eq!(check(&told, &tickets), None);
        tickets[1].status = TicketStatus::Waiting;
        let (m, told) = check(&told, &tickets).unwrap();
        assert_eq!(m, Milestone::NeedsYou);
        tickets[1].status = TicketStatus::InProgress;
        assert_eq!(check(&told, &tickets), None);
        tickets[1].status = TicketStatus::Waiting;
        assert_eq!(check(&told, &tickets), None, "the same ticket waiting again isn't told twice");
        tickets[1] = merged(ticket(2, "Build", TicketStatus::Done), 2_000);
        let (m, told) = check(&told, &tickets).unwrap();
        assert_eq!(m, Milestone::AllDone, "the last piece landing is all done");
        assert!(told.all_done);
        assert_eq!(check(&told, &tickets), None);
        assert_eq!(Told::parse(&told.code()), told);
    }

    #[test]
    fn what_moved_together_is_one_milestone_the_most_telling() {
        let links = vec![link(1, None, Link::Asked), link(2, None, Link::Asked), link(3, None, Link::Asked)];
        let tickets = vec![merged(ticket(1, "A", TicketStatus::Done), 5), ticket(2, "B", TicketStatus::Waiting), ticket(3, "C", TicketStatus::InProgress)];
        let s = status(&links, &tickets, &[]);
        let (m, told) = news(&Told::default(), &links, &tickets, &[], &s).unwrap();
        assert_eq!(m, Milestone::NeedsYou);
        assert_eq!(told.code(), "1:dl,2:w");
    }

    #[test]
    fn a_member_is_told_each_open_request_with_its_tickets_and_line() {
        let (links, tickets) = morning();
        let row = Row {
            id: 7,
            title: "새 입력창이랑 왼쪽 메뉴".into(),
            words: String::new(),
            session: Some(1),
            member: Some("designer".into()),
            asked_at: 0,
            told: Told::default(),
            due_at: None,
            closed_at: None,
            tickets: links.clone(),
        };
        let open = row.view(status(&links, &tickets, &[]));
        let closed = Request { id: 8, closed_at: Some(5), ..open.clone() };
        let text = context(&[open, closed], 2_000).unwrap();
        assert!(text.contains("R-7 “새 입력창이랑 왼쪽 메뉴” (T-3565, T-3567, T-3613, T-3614, T-3616, T-3619, T-3591)"), "{text}");
        assert!(text.contains("2 things wait for you"), "{text}");
        assert!(!text.contains("R-8"), "a finished one isn't");
        assert!(!text.contains("T-3639"), "nor equipe's checks");
        assert_eq!(context(&[], 0), None);
    }

    #[test]
    fn a_question_about_a_request_finds_it_in_any_script() {
        let (links, tickets) = morning();
        let base = Row {
            id: 3,
            title: "새 입력창 만들어 줘".into(),
            words: String::new(),
            session: Some(1),
            member: None,
            asked_at: 0,
            told: Told::default(),
            due_at: None,
            closed_at: None,
            tickets: links.clone(),
        };
        let composer = base.view(status(&links, &tickets, &[]));
        let menu = Request { id: 4, title: "メニューを新しく".into(), tickets: vec![link(77, None, Link::Asked)], status: Status::default(), ..composer.clone() };
        let both = [composer, menu];
        let ids = |text: &str| asked_after(text, &both).iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids("입력창은 어떻게 되고 있어?"), [3]);
        assert_eq!(ids("how's the composer going?"), [3], "by a part's title");
        assert_eq!(ids("what about T-3567?"), [3], "by a ticket");
        assert_eq!(ids("T-77 は？"), [4]);
        assert_eq!(ids("メニューはどう？"), [4]);
        assert!(ids("please also add dark mode to the settings page, when you get to it, no rush at all").is_empty());
        assert!(ids("ok thanks!").is_empty());
    }

    #[test]
    fn a_requests_ticket_waits_on_other_work_only_with_a_reason_and_overlap_is_told() {
        let mut sidebar = ticket(3672, "Sidebar collapse code", TicketStatus::Todo);
        sidebar.body = "Collapse the rail in src/components/Sidebar.tsx".into();
        assert!(chain_refusal(Some(3674), Some(7), &sidebar, None).unwrap().contains("R-7"));
        assert!(chain_refusal(None, None, &sidebar, Some(7)).unwrap().contains("what the person just asked for"), "a request about to be made");
        assert_eq!(chain_refusal(Some(3674), Some(7), &sidebar, Some(7)), None, "within the same request");
        sidebar.status = TicketStatus::Done;
        assert_eq!(chain_refusal(Some(3674), Some(7), &sidebar, None), None, "done work holds nothing up");
        assert_eq!(areas("A live prototype in `design/directions/linear-claude/` and ./design/x.md"), ["design"]);
        assert_eq!(areas("see https://linear.app/x and 3/4 of it"), Vec::<String>::new());
        let proto = "Live prototype in design/directions/linear-claude/";
        assert_eq!(overlap(proto, &sidebar.body), Some(false));
        assert!(overlap_warning(proto, &sidebar).unwrap().contains("in src while this one's is in design"), "{:?}", overlap_warning(proto, &sidebar));
        assert_eq!(overlap(proto, "Tokens in design/tokens.json"), Some(true));
        assert_eq!(overlap(proto, "No paths at all"), None);
    }

    #[test]
    fn a_title_is_the_first_line_that_says_something_cut_short() {
        assert_eq!(title_of("\n  Make the input box new\nand the left menu"), "Make the input box new");
        let long = "x".repeat(100);
        assert_eq!(title_of(&long).chars().count(), TITLE_CHARS);
        assert!(title_of(&long).ends_with('…'));
    }

    #[test]
    fn requests_are_kept_with_their_tickets_and_what_was_told() {
        let conn = Connection::open_in_memory().unwrap();
        create_table(&conn).unwrap();
        let store = Store(&conn);
        let id = store.create("Input box", "Input box, please", Some(9), Some("designer"), 100).unwrap();
        store.link(id, 5, None, Link::Asked, 101).unwrap();
        store.link(id, 6, Some(5), Link::Made, 102).unwrap();
        assert_eq!(store.of_ticket(6), Some(id));
        assert!(store.exists(id) && !store.exists(id + 1));
        store.set_due(id, 300).unwrap();
        store.set_due(id, 400).unwrap();
        assert!(store.due(299).is_empty());
        let due = store.due(300);
        assert_eq!(due.len(), 1, "the first change sets when");
        assert_eq!(due[0].tickets, vec![link(5, None, Link::Asked), link(6, Some(5), Link::Made)]);
        let told = Told::parse("5:d,*");
        store.told(id, &told, Some(500)).unwrap();
        let row = store.get(id).unwrap();
        assert_eq!((row.told, row.due_at, row.closed_at, row.session), (told, None, Some(500), Some(9)));
        assert_eq!(store.listed(400).len(), 1, "closed since: still listed");
        assert!(store.listed(10_000).is_empty(), "closed before: not listed");
        store.link(id, 7, None, Link::Linked, 600).unwrap();
        assert_eq!(store.get(id).unwrap().closed_at, None, "a new ticket opens it again");
        store.link(id, 8, Some(5), Link::Check, 601).unwrap();
        store.link(id, 9, Some(5), Link::FollowUp, 602).unwrap();
        assert_eq!(store.serving(), [5, 6, 7], "what the person asked for goes first; checks and follow-ups don't");
    }
}
