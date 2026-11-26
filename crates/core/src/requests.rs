//! The person's requests ([`equipe_protocol::requests`]): the rules, with no
//! runtime in them, so each is tested alone — where a request stands
//! ([`status`]), the line that says so ([`line`]), when that is news worth a
//! word in its conversation ([`news`]), what a member is told of them at the
//! start of its turn ([`context`]), and whether a message asks after one
//! ([`asked_after`]); which tickets cover one ([`covering`]) and when one is
//! left uncovered ([`standing`]); what a member is asked when it doesn't say
//! which request a ticket serves ([`which_request`]) — and where requests are
//! kept ([`Store`]).
//!
//! The runtime's side — recording a request, attaching tickets by their
//! lineage, telling the conversation — is `runtime::requests`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use equipe_protocol::requests::{Link, Milestone, Part, Phase, Request, RequestId, RequestTicket, Status, Step, Wait};
use equipe_protocol::title::TicketLabel;
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

/// A request no work ticket serves this long after it was asked is
/// uncovered: put to the PM, and shown so to the person (unix seconds).
pub const COVER_WAIT: u64 = 10 * 60;

/// [`COVER_WAIT`] — or, where `EQUIPE_REQUEST_COVER_WAIT` says, that many
/// seconds (tests).
pub fn cover_wait() -> u64 {
    std::env::var("EQUIPE_REQUEST_COVER_WAIT").ok().and_then(|v| v.parse().ok()).unwrap_or(COVER_WAIT)
}

/// `request` as a member gives it when a ticket serves none of the person's
/// requests (`request: 0`).
pub const NO_REQUEST: RequestId = 0;

/// A request's words, at most this long, where a member tells it apart
/// from another asked about the same time.
const WORDS_CHARS: usize = 90;

/// A first line shorter than this ("Hi", "Also:") doesn't name a request by
/// itself: the lines after it join it.
const SAYS_LITTLE: usize = 16;

/// At most this many characters of the person's words name a request.
const TITLE_CHARS: usize = 60;

/// At most this many characters of the person's words show on a request's
/// hover (a long paste stays a tooltip, not a page).
const HOVER_CHARS: usize = 400;

/// A part's title, and a wait's, at most this long in a status line.
const PART_CHARS: usize = 36;
const WAIT_CHARS: usize = 28;

/// At most this many parts, and waits, are named in a status line.
const PARTS_SHOWN: usize = 4;
const WAITS_SHOWN: usize = 3;

/// What the person said, as a request's name: its first line that says
/// something — joined by the next ones while it is too short to say much
/// alone — cut short.
pub fn title_of(words: &str) -> String {
    let mut line = String::new();
    for l in words.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(l);
        if line.chars().count() >= SAYS_LITTLE {
            break;
        }
    }
    cut(&line, TITLE_CHARS)
}

/// The person's words in one line, cut short: what tells two requests
/// asked minutes apart from each other.
pub fn words_line(words: &str) -> String {
    cut(&words.split_whitespace().collect::<Vec<_>>().join(" "), WORDS_CHARS)
}

/// The person's words in one line for a screen's hover, when the title cut
/// them short; empty when the title already says it all.
pub fn hover_words(words: &str, title: &str) -> String {
    let line = cut(&words.split_whitespace().collect::<Vec<_>>().join(" "), HOVER_CHARS);
    if line.is_empty() || line == title { String::new() } else { line }
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

/// Whether work on main is live: pushed is not live where a project goes
/// live by a deploy (its own go-live command, a production deployment, a
/// deploy workflow — `crate::go_live`, `crate::deploys`). Built by the
/// runtime from what it saw go live; the default knows of no deploy, so
/// work on main is live.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shipped {
    /// The tickets a deploy equipe saw succeed carried.
    pub live: HashSet<TicketId>,
    /// The projects (by path) whose work is live only once a deploy says so,
    /// and, while it can't, why (`deploy_failed`…).
    pub by_deploy: HashMap<String, Option<String>>,
}

impl Shipped {
    /// Work of `t` merged into main but not live yet: `Some(why)`. `None`
    /// when it is live, or isn't on main.
    pub fn not_live(&self, t: &Ticket) -> Option<Option<String>> {
        if !live(t) || self.live.contains(&t.id) {
            return None;
        }
        self.by_deploy.get(t.project.as_deref()?.to_string_lossy().as_ref()).cloned()
    }
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

/// Whether `ticket` — or a ticket of the request it came of — joined it as
/// equipe's own check.
fn through_check(links: &[RequestTicket], ticket: TicketId) -> bool {
    let mut at = ticket;
    // A loop of parents is cut where it closes.
    for _ in 0..=links.len() {
        let Some(link) = links.iter().find(|l| l.ticket == at) else { return false };
        if link.link == Link::Check {
            return true;
        }
        match link.parent {
            Some(parent) if parent != ticket => at = parent,
            _ => return false,
        }
    }
    false
}

/// The tickets of a request that cover it: work that serves it. Not
/// equipe's own checks — a `check` link or anything that came of one, a
/// design QA ticket — nor a meta ticket (labelled check). A ticket missing
/// from `tickets` covers nothing. A dropped one still does: its dropping
/// is told to the person as such.
pub fn covering(links: &[RequestTicket], tickets: &[Ticket]) -> Vec<TicketId> {
    links
        .iter()
        .filter(|l| !through_check(links, l.ticket))
        .filter_map(|l| tickets.iter().find(|t| t.id == l.ticket))
        .filter(|t| t.label != Some(TicketLabel::Check) && !tickets.iter().any(|o| o.design_qa == Some(t.id)))
        .map(|t| t.id)
        .collect()
}

/// Where a request stands: each part — a ticket it asked for and what came
/// of it — how far its work got; what of it waits on the person, apart; and
/// its tickets in order. `tickets` holds them (any others are ignored; one
/// missing is left out); `asking`, the tickets with a question open to the
/// person. equipe's own checks show in the timeline only.
pub fn status(links: &[RequestTicket], tickets: &[Ticket], asking: &[TicketId]) -> Status {
    shipped_status(links, tickets, asking, &Shipped::default())
}

/// [`status`], work on main live only as `shipped` says.
pub fn shipped_status(links: &[RequestTicket], tickets: &[Ticket], asking: &[TicketId], shipped: &Shipped) -> Status {
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
            merged: live(t) && shipped.not_live(t).is_none(),
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
        let (phase, who, since, not_live) = phase(&mine, asking, shipped);
        parts.push(Part { ticket: head.id, title: head.title.clone(), phase, who, since, not_live });
    }
    let cover = covering(links, tickets);
    let done = !cover.is_empty() && !parts.is_empty() && waits.is_empty() && parts.iter().all(|p| p.phase.is_finished());
    let decided = done && decided_only(&cover, tickets);
    Status { parts, waits, steps, done, decided, ..Status::default() }
}

/// Whether the covering tickets `cover`, all finished, put nothing live: no
/// code of them merged or waits to merge, and none is a deploy — the request
/// was a decision or an instruction answered, whose production lead time
/// doesn't apply (`crate::lead_time` reads it the same way).
fn decided_only(cover: &[TicketId], tickets: &[Ticket]) -> bool {
    let covering: Vec<&Ticket> = cover.iter().filter_map(|id| tickets.iter().find(|t| t.id == *id)).collect();
    let ships = |t: &Ticket| (t.kind == TicketKind::Code && t.status == TicketStatus::Done && t.merged.is_some()) || t.label == Some(TicketLabel::Deploy);
    // All of it dropped is dropped, not answered.
    covering.iter().any(|t| t.status == TicketStatus::Done) && !covering.iter().any(|t| ships(t))
}

/// Where the request kept as `row` stands at `now`: [`status`], and — still
/// open, with no ticket covering it [`cover_wait`] after it was asked —
/// uncovered; closed by a member, why.
pub fn standing(row: &Row, tickets: &[Ticket], asking: &[TicketId], shipped: &Shipped, now: u64) -> Status {
    let mut status = shipped_status(&row.tickets, tickets, asking, shipped);
    status.uncovered = row.closed_at.is_none() && covering(&row.tickets, tickets).is_empty() && now >= row.asked_at.saturating_add(cover_wait());
    status.closed_why = row.closed_why.clone();
    status
}

/// How far a part's tickets got: the open ones that don't wait on the
/// person say it — stopped, worked on, in review, next — and once none is
/// open, what was done says it: live, on main and not live yet (and why),
/// accepted, done.
fn phase(tickets: &[&Ticket], asking: &[TicketId], shipped: &Shipped) -> (Phase, Option<String>, Option<u64>, Option<String>) {
    let (phase, who, since) = how_far(tickets, asking);
    if phase != Phase::Live {
        return (phase, who, since, None);
    }
    // Live only once every piece of its merged work is.
    let waiting: Vec<Option<String>> = tickets.iter().filter_map(|t| shipped.not_live(t)).collect();
    if waiting.is_empty() {
        return (phase, who, since, None);
    }
    (Phase::OnMain, who, since, waiting.into_iter().flatten().next())
}

fn how_far(tickets: &[&Ticket], asking: &[TicketId]) -> (Phase, Option<String>, Option<u64>) {
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

/// Where a request stands, in one line in `words`' language: why a member
/// closed it, or that no ticket serves it yet; each part and how far it
/// got, then what waits on the person — or that it is all done.
/// `clock` says a time.
pub fn line(status: &Status, words: &Catalog, clock: &dyn Fn(u64) -> String) -> String {
    let mut out: Vec<String> = Vec::new();
    if let Some(why) = &status.closed_why {
        out.push(words.fill("request.closed", &[("why", cut(why, WORDS_CHARS))]));
    }
    if status.uncovered {
        out.push(words.get("request.uncovered").to_owned());
    }
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
            Phase::OnMain if part.not_live.is_some() => "request.phase.not_live",
            Phase::OnMain if time.is_empty() => "request.phase.on_main_now",
            Phase::OnMain => "request.phase.on_main",
            Phase::Live if time.is_empty() => "request.phase.live_now",
            Phase::Live => "request.phase.live",
            Phase::Done => "request.phase.done",
            Phase::Cancelled => "request.phase.cancelled",
        };
        let why = part.not_live.as_deref().map(|code| words.get(&format!("request.why.{code}")).to_owned()).unwrap_or_default();
        out.push(words.fill(key, &[("title", title), ("who", part.who.clone().unwrap_or_default()), ("time", time), ("why", why)]));
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
    if status.decided {
        out.push(words.get("request.decided").to_owned());
    } else if status.done {
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
/// waits on them, `b` its run stopped, `l` its work is live, `m` on main
/// and not live yet, `x` on main and its deploy failed, `d` done. Being
/// worked on, in review, sent back and handed in again: no news.
pub fn mark(t: &Ticket, asking: bool, shipped: &Shipped) -> Option<char> {
    match t.status {
        TicketStatus::Waiting => Some('w'),
        _ if asking && !t.status.is_closed() => Some('w'),
        _ if stalled(t) => Some('b'),
        TicketStatus::Done if live(t) => match shipped.not_live(t) {
            None => Some('l'),
            Some(None) => Some('m'),
            Some(Some(_)) => Some('x'),
        },
        TicketStatus::Done => Some('d'),
        _ => None,
    }
}

/// The marks a mark implies were passed on the way: live work was on main,
/// and done.
fn with_passed(mark: char) -> &'static [char] {
    match mark {
        'l' => &['l', 'm', 'd'],
        'x' => &['x', 'm', 'd'],
        'm' => &['m', 'd'],
        'd' => &['d'],
        'w' => &['w'],
        'b' => &['b'],
        _ => &[],
    }
}

/// Whether a request has news since `told`: its milestone — the most
/// telling of what is new — and what is told once it is said. Each mark of
/// each ticket is told once; equipe's own checks never are; "all done" once.
pub fn news(told: &Told, links: &[RequestTicket], tickets: &[Ticket], asking: &[TicketId], shipped: &Shipped, status: &Status) -> Option<(Milestone, Told)> {
    let mut next = told.clone();
    let mut new: Vec<char> = Vec::new();
    for link in links.iter().filter(|l| l.link != Link::Check) {
        let Some(t) = tickets.iter().find(|t| t.id == link.ticket) else { continue };
        let Some(m) = mark(t, asking.contains(&t.id), shipped) else { continue };
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
    } else if new.contains(&'x') {
        Some(Milestone::NotLive)
    } else if new.contains(&'l') {
        Some(Milestone::Live)
    } else if new.contains(&'m') {
        Some(Milestone::OnMain)
    } else if new.contains(&'d') {
        Some(Milestone::Done)
    } else {
        None
    };
    milestone.map(|m| (m, next))
}

// ── what a member is told ──

/// One request as a member reads it, in one line: its number, its name in
/// the person's words, the tickets that serve it, when it was asked — and,
/// `with_line`, where it stands.
fn member_line(r: &Request, now: u64, with_line: bool) -> String {
    let tickets: Vec<String> = r.tickets.iter().filter(|l| l.link != Link::Check).map(|l| format!("T-{}", l.ticket)).collect();
    let tickets = if tickets.is_empty() { "no ticket yet".to_owned() } else { tickets.join(", ") };
    let head = format!("R-{} “{}” ({tickets}) · asked {}", r.id, r.title, clock(r.asked_at, now));
    if !with_line {
        return head;
    }
    let words = crate::i18n::core(crate::i18n::Lang::En);
    format!("{head}: {}", line(&r.status, words, &|at| clock(at, now)))
}

/// What a member is told of the person's open requests in the conversation
/// its turn is in, at the turn's start (English, as every word to a model):
/// so "how is X going?" is answered at once, and right, and each gets a
/// ticket that serves it. `elsewhere`: the PM's — requests asked of other
/// members that no ticket covers yet. `None` without any.
pub fn context(requests: &[Request], elsewhere: &[Request], now: u64) -> Option<String> {
    let open: Vec<&Request> = requests.iter().filter(|r| r.closed_at.is_none()).collect();
    let away: Vec<&Request> = elsewhere.iter().filter(|r| r.closed_at.is_none() && !open.iter().any(|o| o.id == r.id)).collect();
    if open.is_empty() && away.is_empty() {
        return None;
    }
    let mut text = String::from("[equipe: ");
    if !open.is_empty() {
        let lines: Vec<String> = open.iter().map(|r| format!("- {}", member_line(r, now, true))).collect();
        text.push_str(&format!(
            "what the person asked for in this conversation, and where each stands now (from the board):\n{}\n\
When they ask how one is going, answer from this at once — never \"at the next step\" — and name what waits on them. \
Each needs a ticket that serves it: give its R-number as `request` on `create_ticket` (with more than one open here, equipe \
asks which; 0 when a ticket serves none). One that asks for no work — thanks, a question you answered, an answer to yours — \
close with `close_request`, saying why in a few words: the person sees them. equipe closes a request itself only once the \
tickets serving it are done.",
            lines.join("\n")
        ));
    }
    if !away.is_empty() {
        let lines: Vec<String> = away.iter().map(|r| format!("- {} — asked of {}", member_line(r, now, false), r.member.as_deref().unwrap_or("the squad"))).collect();
        if !open.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!(
            "the person's requests no ticket serves yet, asked of other members — see each gets one (`create_ticket` with its \
`request`), or is closed with why (`close_request`):\n{}",
            lines.join("\n")
        ));
    }
    text.push(']');
    Some(text)
}

/// What a member is told when it puts a ticket on the board without saying
/// which of the person's open requests it serves, and more than one could
/// be it: each in one line (English, for the member).
pub fn which_request(open: &[Request], now: u64) -> String {
    let lines: Vec<String> = open.iter().map(|r| format!("- {}", member_line(r, now, false))).collect();
    format!(
        "Which of the person's requests does this ticket serve? More than one is open here:\n{}\nGive its number as `request` \
(e.g. {} for R-{}) — or 0 when it serves none of them — and create it again.",
        lines.join("\n"),
        open.first().map_or(1, |r| r.id),
        open.first().map_or(1, |r| r.id)
    )
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
pub(crate) fn ticket_numbers(text: &str) -> Vec<TicketId> {
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

// ── words that ask for nothing ──

/// A message longer than this is always heard as asking for something.
const SMALL_TALK_CHARS: usize = 16;

/// Whether `text` is an acknowledgement or chit-chat that asks for nothing
/// — "ok", "thanks!", an emoji — so it opens no request: no question mark,
/// short, and either no letter or digit at all, or only words of
/// `requests.small_talk` (every language's, in the locale files).
pub fn small_talk(text: &str) -> bool {
    small_talk_in(text, crate::i18n::every("requests.small_talk"))
}

fn small_talk_in(text: &str, words: &[&str]) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return true;
    }
    if text.contains(['?', '？']) || text.chars().count() > SMALL_TALK_CHARS {
        return false;
    }
    if !text.chars().any(char::is_alphanumeric) {
        return true;
    }
    let mut list: Vec<String> = words.iter().map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect();
    list.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));
    let lower: String = text.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' }).collect();
    let mut rest = format!(" {} ", lower.split_whitespace().collect::<Vec<_>>().join(" "));
    // Phrases first ("thank you", "alles klar"), the longest first.
    for phrase in list.iter().filter(|w| w.contains(' ')) {
        let spaced = format!(" {phrase} ");
        while rest.contains(&spaced) {
            rest = rest.replacen(&spaced, " ", 1);
        }
    }
    let single: Vec<&str> = list.iter().filter(|w| !w.contains(' ')).map(String::as_str).collect();
    rest.split_whitespace().all(|token| said_little(token, &single))
}

/// `token` with each run of one character kept once ("hahaha" stays, "okkk" is "ok").
fn squeezed(token: &str) -> String {
    let mut out = String::new();
    for c in token.chars() {
        if !out.ends_with(c) {
            out.push(c);
        }
    }
    out
}

/// Whether one word of a message is one of `words`, its repeated letters
/// aside — or, in a script written without spaces, several of them run
/// together ("好的哈哈").
fn said_little(token: &str, words: &[&str]) -> bool {
    let candidates = [token.to_owned(), squeezed(token)];
    if candidates.iter().any(|t| words.contains(&t.as_str())) {
        return true;
    }
    !token.is_ascii() && candidates.iter().any(|t| runs_together(t, words))
}

/// Whether `text` is nothing but `words` run together.
fn runs_together(text: &str, words: &[&str]) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut reach = vec![false; chars.len() + 1];
    reach[0] = true;
    for start in 0..chars.len() {
        if !reach[start] {
            continue;
        }
        for word in words {
            let w: Vec<char> = word.chars().collect();
            if !w.is_empty() && chars[start..].starts_with(&w) {
                reach[start + w.len()] = true;
            }
        }
    }
    reach[chars.len()]
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
    /// Closed by a member with its work not done: why.
    pub closed_why: Option<String>,
    /// Recorded from the person's words in a conversation (not a ticket they
    /// put on the board): a ticket made there in answer may serve it.
    pub said: bool,
    /// When the PM was told no ticket covers it.
    pub surfaced_at: Option<u64>,
    pub tickets: Vec<RequestTicket>,
}

impl Row {
    /// Whether a screen lists it: once a ticket is with it, or it was put
    /// to the PM as uncovered. Words no work came of, closed with a reason
    /// before then — thanks, a question answered — are told in their
    /// conversation, not listed.
    pub fn shown(&self) -> bool {
        !self.tickets.is_empty() || self.surfaced_at.is_some()
    }

    /// As a screen gets it, with where it stands.
    pub fn view(&self, status: Status) -> Request {
        Request {
            id: self.id,
            title: self.title.clone(),
            words: hover_words(&self.words, &self.title),
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
    )?;
    // Kept since a journal made before them: added, once.
    for column in ["closed_why TEXT", "said INTEGER NOT NULL DEFAULT 0", "surfaced_at INTEGER"] {
        let _ = conn.execute(&format!("ALTER TABLE requests ADD COLUMN {column}"), []);
    }
    Ok(())
}

/// The squad's requests in its journal.
pub struct Store<'a>(pub(crate) &'a Connection);

impl Store<'_> {
    /// Keep a new request; its number. `said`: from the person's words in a
    /// conversation.
    pub fn create(&self, title: &str, words: &str, session: Option<SessionId>, member: Option<&str>, asked_at: u64, said: bool) -> rusqlite::Result<RequestId> {
        let words: String = words.chars().take(4000).collect();
        self.0.execute(
            "INSERT INTO requests (title, words, session, member, asked_at, said) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![title, words, session.map(|s| s as i64), member, asked_at as i64, said],
        )?;
        Ok(self.0.last_insert_rowid() as RequestId)
    }

    /// `ticket` serves `request` now (a ticket serves one at most): what
    /// came of it — made while working it, its follow-ups, its checks —
    /// moves with it from the request it served before. The request is open
    /// again if it was closed, unless all that joined is one of equipe's checks.
    pub fn link(&self, request: RequestId, ticket: TicketId, parent: Option<TicketId>, link: Link, at: u64) -> rusqlite::Result<()> {
        let was = self.of_ticket(ticket);
        self.0.execute(
            "INSERT OR REPLACE INTO request_tickets (ticket, request, parent, link, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ticket, request as i64, parent, link.code(), at as i64],
        )?;
        if was.is_some_and(|w| w != request) {
            for child in self.descendants(ticket) {
                self.0.execute("UPDATE request_tickets SET request = ?2 WHERE ticket = ?1", params![child, request as i64])?;
            }
        }
        if link != Link::Check {
            self.reopen(request)?;
        }
        Ok(())
    }

    /// `ticket` serves no request now, nor does what came of it.
    pub fn unlink(&self, ticket: TicketId) -> rusqlite::Result<()> {
        for t in std::iter::once(ticket).chain(self.descendants(ticket)) {
            self.0.execute("DELETE FROM request_tickets WHERE ticket = ?1", [t])?;
        }
        Ok(())
    }

    /// The tickets that came of `ticket` — by their `parent`, all the way down.
    fn descendants(&self, ticket: TicketId) -> Vec<TicketId> {
        let Ok(mut stmt) = self.0.prepare_cached("SELECT ticket FROM request_tickets WHERE parent = ?1") else { return Vec::new() };
        let mut out: Vec<TicketId> = Vec::new();
        let mut next = vec![ticket];
        while let Some(at) = next.pop() {
            let children: Vec<TicketId> = stmt.query_map([at], |r| r.get(0)).and_then(|rows| rows.collect()).unwrap_or_default();
            for child in children {
                if child != ticket && !out.contains(&child) {
                    out.push(child);
                    next.push(child);
                }
            }
        }
        out
    }

    /// A closed request open again: its "all done" is to be told anew when
    /// it is, and the reason it was closed no longer holds.
    fn reopen(&self, request: RequestId) -> rusqlite::Result<()> {
        let Some(row) = self.get(request).filter(|r| r.closed_at.is_some()) else { return Ok(()) };
        let told = Told { all_done: false, ..row.told };
        self.0.execute("UPDATE requests SET closed_at = NULL, closed_why = NULL, told = ?2 WHERE id = ?1", params![request as i64, told.code()])?;
        Ok(())
    }

    /// Closed by a member at `at`, its work not done: `why`, as the person is told.
    pub fn close(&self, request: RequestId, why: &str, at: u64) -> rusqlite::Result<()> {
        self.0.execute("UPDATE requests SET closed_at = ?2, closed_why = ?3, due_at = NULL WHERE id = ?1", params![request as i64, at as i64, why])?;
        Ok(())
    }

    /// The PM was told at `at` that no ticket covers it.
    pub fn surfaced(&self, request: RequestId, at: u64) -> rusqlite::Result<()> {
        self.0.execute("UPDATE requests SET surfaced_at = ?2 WHERE id = ?1", params![request as i64, at as i64])?;
        Ok(())
    }

    /// Those still open, the oldest first.
    pub fn open(&self) -> Vec<Row> {
        self.rows("WHERE closed_at IS NULL ORDER BY asked_at, id", ())
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
        let Ok(mut stmt) = self.0.prepare(&format!("SELECT id, title, words, session, member, asked_at, told, due_at, closed_at, closed_why, said, surfaced_at FROM requests {filter}")) else {
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
                    closed_why: r.get(9)?,
                    said: r.get(10)?,
                    surfaced_at: r.get::<_, Option<i64>>(11)?.map(|s| s.max(0) as u64),
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
            "Spec the Tinumi composer — live since @1100 · Left menu, three options — frontend is on it · 2 things wait for you (Check the new composer on r…, Decide if the layout needs…)"
        );
    }

    /// "Go with B": its ticket done and nothing of it merged or deployed —
    /// answered, with nothing to ship; never "not live yet".
    #[test]
    fn a_request_whose_work_merged_nothing_reads_as_answered_not_waiting_to_go_live() {
        let words = core(Lang::En);
        let clock = |at: u64| format!("@{at}");
        let links = vec![link(7, None, Link::Asked)];
        let mut decision = ticket(7, "Go with option B", TicketStatus::Done);
        decision.kind = TicketKind::Task;
        let s = status(&links, &[decision.clone()], &[]);
        assert!(s.done && s.decided, "{s:?}");
        assert_eq!(line(&s, words, &clock), "Go with option B — done · answered — nothing to ship");
        // Its code merged: shipped, all done as before.
        let shipped = merged(ticket(7, "Go with option B", TicketStatus::Done), 1_200);
        let s = status(&links, &[shipped], &[]);
        assert!(s.done && !s.decided);
        assert!(line(&s, words, &clock).ends_with("all done"));
        // Accepted and waiting to merge, or a deploy: not a decision.
        let mut accepted = ticket(7, "Go with option B", TicketStatus::Done);
        accepted.merged = Some(false);
        assert!(!status(&links, &[accepted], &[]).decided);
        let mut deploy = decision;
        deploy.label = Some(TicketLabel::Deploy);
        assert!(!status(&links, &[deploy], &[]).decided);
        // All of it dropped: dropped, not answered.
        let s = status(&links, &[ticket(7, "Go with option B", TicketStatus::Cancelled)], &[]);
        assert!(s.done && !s.decided);
        assert!(line(&s, words, &clock).ends_with("all done"));
        // Still open: neither.
        assert!(!status(&links, &[ticket(7, "Go with option B", TicketStatus::InProgress)], &[]).decided);
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
        assert_eq!(mark(&t, true, &Shipped::default()), Some('w'));
    }

    #[test]
    fn each_milestone_is_news_once_and_a_retry_is_none() {
        let links = vec![link(1, None, Link::Asked), link(2, Some(1), Link::Made), link(3, Some(2), Link::Check)];
        let mut tickets = vec![ticket(1, "Spec", TicketStatus::InProgress), ticket(2, "Build", TicketStatus::Todo), ticket(3, "Design QA", TicketStatus::Todo)];
        let check = |told: &Told, tickets: &[Ticket]| news(told, &links, tickets, &[], &Shipped::default(), &status(&links, tickets, &[]));
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
    fn work_on_main_closes_a_request_only_once_its_deploy_went_live() {
        let links = vec![link(1, None, Link::Asked)];
        let mut t = merged(ticket(1, "Hero banner", TicketStatus::Done), 50);
        t.project = Some("/c/site".into());
        let tickets = vec![t];
        let words = core(Lang::En);
        // The project goes live by a deploy nobody has seen succeed yet: on main, not live, open.
        let mut shipped = Shipped { by_deploy: HashMap::from([("/c/site".to_owned(), None)]), ..Shipped::default() };
        let s = shipped_status(&links, &tickets, &[], &shipped);
        assert_eq!((s.parts[0].phase, s.parts[0].not_live.as_deref(), s.done), (Phase::OnMain, None, false));
        assert!(!s.steps[0].merged, "not shown as landed live");
        assert_eq!(line(&s, words, &|at| format!("@{at}")), "Hero banner — on main since @50, not live yet");
        let (m, told) = news(&Told::default(), &links, &tickets, &[], &shipped, &s).unwrap();
        assert_eq!((m, told.code()), (Milestone::OnMain, "1:dm".to_owned()));
        // Its deploy failed: still open, and why — told once.
        shipped.by_deploy.insert("/c/site".into(), Some("deploy_failed".into()));
        let s = shipped_status(&links, &tickets, &[], &shipped);
        assert_eq!((s.parts[0].phase, s.parts[0].not_live.as_deref(), s.done), (Phase::OnMain, Some("deploy_failed"), false));
        assert_eq!(line(&s, words, &|_| String::new()), "Hero banner — on main, not live: its deploy failed");
        let (m, told) = news(&told, &links, &tickets, &[], &shipped, &s).unwrap();
        assert_eq!(m, Milestone::NotLive);
        assert_eq!(news(&told, &links, &tickets, &[], &shipped, &s), None, "the same failure isn't told twice");
        // A deploy carried it: live, and the request is all done.
        shipped.live.insert(1);
        let s = shipped_status(&links, &tickets, &[], &shipped);
        assert_eq!((s.parts[0].phase, s.done), (Phase::Live, true));
        assert_eq!(news(&told, &links, &tickets, &[], &shipped, &s).map(|(m, _)| m), Some(Milestone::AllDone));
        // A project that doesn't deploy: on main is live, as before.
        let s = shipped_status(&links, &tickets, &[], &Shipped::default());
        assert_eq!((s.parts[0].phase, s.done), (Phase::Live, true));
    }

    #[test]
    fn what_moved_together_is_one_milestone_the_most_telling() {
        let links = vec![link(1, None, Link::Asked), link(2, None, Link::Asked), link(3, None, Link::Asked)];
        let tickets = vec![merged(ticket(1, "A", TicketStatus::Done), 5), ticket(2, "B", TicketStatus::Waiting), ticket(3, "C", TicketStatus::InProgress)];
        let s = status(&links, &tickets, &[]);
        let (m, told) = news(&Told::default(), &links, &tickets, &[], &Shipped::default(), &s).unwrap();
        assert_eq!(m, Milestone::NeedsYou);
        assert_eq!(told.code(), "1:dlm,2:w");
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
            closed_why: None,
            said: true,
            surfaced_at: None,
            tickets: links.clone(),
        };
        let open = row.view(status(&links, &tickets, &[]));
        let closed = Request { id: 8, closed_at: Some(5), ..open.clone() };
        let text = context(&[open, closed], &[], 2_000).unwrap();
        assert!(text.contains("R-7 “새 입력창이랑 왼쪽 메뉴” (T-3565, T-3567, T-3613, T-3614, T-3616, T-3619, T-3591) · asked "), "{text}");
        assert!(text.contains("2 things wait for you"), "{text}");
        assert!(text.contains("`close_request`"), "words that ask for no work are closed, with why: {text}");
        assert!(!text.contains("R-8"), "a finished one isn't");
        assert!(!text.contains("T-3639"), "nor equipe's checks");
        assert_eq!(context(&[], &[], 0), None);
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
            closed_why: None,
            said: true,
            surfaced_at: None,
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

    fn row(id: RequestId, asked_at: u64, tickets: Vec<RequestTicket>) -> Row {
        Row {
            id,
            title: format!("Request {id}"),
            words: format!("Request {id}, please"),
            session: Some(1),
            member: Some("pm".into()),
            asked_at,
            told: Told::default(),
            due_at: None,
            closed_at: None,
            closed_why: None,
            said: true,
            surfaced_at: None,
            tickets,
        }
    }

    #[test]
    fn only_work_covers_a_request_never_equipes_checks_nor_meta_tickets() {
        let mut qa = ticket(10, "Design QA: T-1 sidebar toggle", TicketStatus::Done);
        qa.kind = TicketKind::Task;
        let mut work = ticket(1, "Sidebar toggle", TicketStatus::InProgress);
        work.design_qa = Some(10);
        let mut meta = ticket(3, "Verify the toggle on staging", TicketStatus::Todo);
        meta.label = Some(TicketLabel::Check);
        let under_check = ticket(11, "Fix the toggle radius", TicketStatus::Todo);
        let all = [work.clone(), qa.clone(), meta.clone(), under_check.clone()];
        // A request whose only link is a check — what was left of a ticket moved elsewhere.
        let only_check = vec![link(10, Some(1), Link::Check)];
        assert!(covering(&only_check, &all).is_empty());
        let s = status(&only_check, &all, &[]);
        assert!(!s.done, "a done check doesn't finish a request: {s:?}");
        // Made while working the check: came of a check, covers nothing.
        let from_check = vec![link(1, None, Link::Asked), link(10, Some(1), Link::Check), link(11, Some(10), Link::Made)];
        assert_eq!(covering(&from_check, &all), [1]);
        // A meta ticket, and a design QA joined any other way, cover nothing either.
        assert!(covering(&[link(3, None, Link::Asked), link(10, None, Link::Linked)], &all).is_empty());
        // A ticket not on the board covers nothing.
        assert!(covering(&[link(99, None, Link::Asked)], &all).is_empty());
    }

    #[test]
    fn a_request_closes_on_its_own_work_only() {
        // The incident: the dark theme's tickets landed; the MCP-usage request had none of its own.
        let dark = merged(ticket(3766, "Apply the dark theme", TicketStatus::Done), 2_000);
        let mut qa = ticket(3767, "Design QA: T-3766 dark theme", TicketStatus::Done);
        qa.kind = TicketKind::Task;
        let tickets = [dark.clone(), qa];
        let mcp = vec![link(3767, Some(3766), Link::Check)];
        let s = status(&mcp, &tickets, &[]);
        assert!(!s.done && s.parts.is_empty(), "{s:?}");
        assert_eq!(news(&Told::default(), &mcp, &tickets, &[], &Shipped::default(), &s), None, "nothing to tell, nothing to close");
        // Its own work, done: all done.
        let own = vec![link(3766, None, Link::Asked), link(3767, Some(3766), Link::Check)];
        let s = status(&own, &tickets, &[]);
        assert!(s.done, "{s:?}");
        assert_eq!(news(&Told::default(), &own, &tickets, &[], &Shipped::default(), &s).map(|(m, _)| m), Some(Milestone::AllDone));
    }

    #[test]
    fn a_request_no_work_covers_is_uncovered_after_a_while_and_says_so() {
        let words = core(Lang::En);
        let clock = |at: u64| format!("@{at}");
        let bare = row(3, 1_000, vec![]);
        assert!(!standing(&bare, &[], &[], &Shipped::default(), 1_000 + COVER_WAIT - 1).uncovered, "a few minutes to put it on the board");
        let s = standing(&bare, &[], &[], &Shipped::default(), 1_000 + COVER_WAIT);
        assert!(s.uncovered && !s.done, "{s:?}");
        assert_eq!(line(&s, words, &clock), "no ticket serves it yet — the PM is told");
        // Only a check with it: still uncovered.
        let mut qa = ticket(10, "Design QA: T-1 sidebar toggle", TicketStatus::InProgress);
        qa.kind = TicketKind::Task;
        let checked = row(3, 1_000, vec![link(10, Some(1), Link::Check)]);
        assert!(standing(&checked, std::slice::from_ref(&qa), &[], &Shipped::default(), 1_000 + COVER_WAIT).uncovered);
        // Work with it: covered, whenever.
        let worked = row(3, 1_000, vec![link(1, None, Link::Asked)]);
        let t = ticket(1, "Fix the login error", TicketStatus::Todo);
        let s = standing(&worked, std::slice::from_ref(&t), &[], &Shipped::default(), 1_000 + 10 * COVER_WAIT);
        assert!(!s.uncovered, "{s:?}");
        // Closed by the PM with why: not uncovered, and the person reads why.
        let closed = Row { closed_at: Some(2_000), closed_why: Some("answered in chat".into()), ..bare };
        let s = standing(&closed, &[], &[], &Shipped::default(), 1_000 + 10 * COVER_WAIT);
        assert!(!s.uncovered && !s.done, "{s:?}");
        assert_eq!(line(&s, words, &clock), "closed: answered in chat");
    }

    #[test]
    fn a_member_is_asked_which_request_each_in_one_line_and_the_pm_sees_the_uncovered() {
        let login = Request { title: "Login and logout show an error".into(), ..row(3, 1_000, vec![]).view(Status::default()) };
        let dark = Request { id: 4, title: "Dark mode, a black terminal style".into(), tickets: vec![link(3766, None, Link::Asked)], ..login.clone() };
        let text = which_request(&[login.clone(), dark.clone()], 2_000);
        let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].starts_with("- R-3 “Login and logout show an error” (no ticket yet) · asked "), "{text}");
        assert!(lines[1].starts_with("- R-4 “Dark mode, a black terminal style” (T-3766) · asked "), "{text}");
        assert!(text.contains("`request`") && text.contains("0 when it serves none"), "{text}");
        // The PM's turn: those asked of others that nothing covers, apart.
        let away = Request { id: 5, member: Some("designer".into()), title: "Admin page of MCP calls".into(), session: Some(2), ..login.clone() };
        let text = context(&[dark], &[away], 2_000).unwrap();
        assert!(text.contains("R-4 “Dark mode") && text.contains("R-5 “Admin page of MCP calls” (no ticket yet) · asked ") && text.contains("asked of designer"), "{text}");
        assert!(context(&[], &[login], 2_000).unwrap().contains("no ticket serves yet"), "the PM alone");
    }

    #[test]
    fn what_came_of_a_ticket_follows_it_and_a_reopened_request_closes_again() {
        let conn = Connection::open_in_memory().unwrap();
        create_table(&conn).unwrap();
        let store = Store(&conn);
        let login = store.create("Login error", "Login error", Some(1), Some("pm"), 100, true).unwrap();
        let sidebar = store.create("Sidebar toggle", "Sidebar toggle", Some(1), Some("pm"), 101, true).unwrap();
        // The sidebar's ticket put with the wrong request, its build and its design QA after it.
        store.link(login, 3763, None, Link::Asked, 102).unwrap();
        store.link(login, 3770, Some(3763), Link::Made, 103).unwrap();
        store.link(login, 3780, Some(3763), Link::Check, 104).unwrap();
        store.link(login, 3790, Some(3770), Link::Made, 105).unwrap();
        // Moved where it belongs: all that came of it goes with it, nothing is left behind.
        store.link(sidebar, 3763, None, Link::Linked, 106).unwrap();
        let of = |request| {
            let mut ids: Vec<TicketId> = store.get(request).unwrap().tickets.iter().map(|l| l.ticket).collect();
            ids.sort_unstable();
            ids
        };
        assert!(of(login).is_empty(), "{:?}", of(login));
        assert_eq!(of(sidebar), [3763, 3770, 3780, 3790]);
        // Put with none: it and what came of it serve none.
        store.unlink(3770).unwrap();
        assert_eq!(of(sidebar), [3763, 3780]);
        // All done and closed; a check joining after doesn't open it again — new work does,
        // and its "all done" is told anew when that is done.
        store.told(sidebar, &Told::parse("3763:d,*"), Some(200)).unwrap();
        store.link(sidebar, 3800, Some(3763), Link::Check, 201).unwrap();
        assert_eq!(store.get(sidebar).unwrap().closed_at, Some(200));
        store.link(sidebar, 3801, None, Link::Linked, 202).unwrap();
        let row = store.get(sidebar).unwrap();
        assert_eq!((row.closed_at, row.told.all_done), (None, false), "{row:?}");
        // Closed by the PM with why; surfaced to it once.
        store.close(login, "fixed by T-3700 already", 300).unwrap();
        let row = store.get(login).unwrap();
        assert_eq!((row.closed_at, row.closed_why.as_deref(), row.said), (Some(300), Some("fixed by T-3700 already"), true));
        assert!(!row.shown(), "words no work came of aren't listed");
        store.surfaced(sidebar, 400).unwrap();
        assert_eq!(store.get(sidebar).unwrap().surfaced_at, Some(400));
        assert_eq!(store.open().iter().map(|r| r.id).collect::<Vec<_>>(), [sidebar]);
    }

    #[test]
    fn a_journal_from_before_gets_the_new_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE requests (id INTEGER PRIMARY KEY, title TEXT NOT NULL, words TEXT NOT NULL, session INTEGER, member TEXT,
               asked_at INTEGER NOT NULL, told TEXT NOT NULL DEFAULT '', due_at INTEGER, closed_at INTEGER);
             INSERT INTO requests (title, words, asked_at) VALUES ('Old', 'Old one', 5);",
        )
        .unwrap();
        create_table(&conn).unwrap();
        create_table(&conn).unwrap();
        let row = Store(&conn).get(1).unwrap();
        assert_eq!((row.title.as_str(), row.said, row.closed_why, row.surfaced_at), ("Old", false, None, None));
    }

    #[test]
    fn an_acknowledgement_in_any_language_asks_for_nothing_and_a_request_does() {
        for said in ["ok", "OK!", "thanks", "Thank you!", "ok thanks 🙏", "👍", "🙏🙏", "...", "고마워", "고마워요!", "ㅇㅋ", "ㅋㅋㅋ", "좋아요ㅎㅎ", "넵", "ありがとう", "了解です", "谢谢", "好的", "收到", "謝謝", "danke", "alles klar", "merci", "d'accord", "gracias", "vale"] {
            assert!(small_talk(said), "{said:?} asks for nothing");
        }
        for said in ["fix the login", "ok, deploy it", "thanks, now add dark mode", "고마워 근데 로그인 에러", "ok?", "고마워?", "재발방지 ㄱㄱ", "로그인 에러", "dark mode", "wok", "ありがとう、次は", "好的，部署吧", "merci, et le bouton", "gracias pero falla el login", "a b c d e f g h i j"] {
            assert!(!small_talk(said), "{said:?} is heard");
        }
        assert!(small_talk("   "));
        assert!(!small_talk("thanks thanks thanks"), "longer than a word of thanks is heard all the same");
    }

    #[test]
    fn a_title_is_the_first_line_that_says_something_cut_short() {
        assert_eq!(title_of("\n  Make the input box new\nand the left menu"), "Make the input box new");
        let long = "x".repeat(100);
        assert_eq!(title_of(&long).chars().count(), TITLE_CHARS);
        assert!(title_of(&long).ends_with('…'));
        assert_eq!(title_of("Hi!\nThe login page errors when I log out\n[image]"), "Hi! The login page errors when I log out");
        assert_eq!(words_line("Hi!\nThe login page   errors\n[image]"), "Hi! The login page errors [image]");
    }

    #[test]
    fn hover_words_say_more_than_the_title_or_nothing() {
        let said = "항상 의사를 만날수없는 환자들이 진단을 받고나서 집에서도 약을 어떻게 먹어야 하는지 바로 볼 수 있는 페이지를 만들어줘";
        let title = title_of(said);
        assert!(title.ends_with('…'));
        assert_eq!(hover_words(said, &title), said);
        assert_eq!(hover_words("Fix the footer", &title_of("Fix the footer")), "");
        assert_eq!(hover_words("Hi!\nThe login page   errors", "Hi! The login page errors"), "");
        let long = "y ".repeat(500);
        assert_eq!(hover_words(&long, &title_of(&long)).chars().count(), HOVER_CHARS);
    }

    #[test]
    fn requests_are_kept_with_their_tickets_and_what_was_told() {
        let conn = Connection::open_in_memory().unwrap();
        create_table(&conn).unwrap();
        let store = Store(&conn);
        let id = store.create("Input box", "Input box, please", Some(9), Some("designer"), 100, true).unwrap();
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
