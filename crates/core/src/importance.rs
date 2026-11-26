//! How much each thing that waits on the person matters, so the ones that
//! hold the most up are handled first.
//!
//! Many things pile up for the person: requests for permission, questions,
//! tickets put down for them, sign-ins, runs to bring back, proposals, work
//! accepted but not in main. By kind and by time, the minor ones get handled
//! first and the ones that block a lot are missed. This scores each by what
//! it **holds up**, in a rule short enough to explain:
//!
//! ```text
//! score = BLOCKED_EACH × tickets blocked (up to BLOCKED_CAP)
//!       + the highest priority among what it blocks and its own ticket
//!       + DEPLOY_HOLD, when deploy or release work waits on it
//!       + IDLE_MEMBER × members parked because of it (up to IDLE_CAP)
//! ```
//!
//! *Blocked* means open work tickets (to do, in progress, waiting, in review)
//! that wait on it through `depends_on`, directly or through other tickets.
//! The score never includes age: age only breaks a tie (the older first), so
//! the order is stable while the person works through it.
//!
//! Everything here is pure: tickets and waiting things in, [`Impact`]s out,
//! so every rule is tested on fixtures. The daemon uses it for the Status
//! list and for what the PM is told; the desktop, for Needs you's order.

use std::collections::{HashMap, HashSet, VecDeque};

use equipe_protocol::importance::{HeldRequest, Impact, WaitKind, Waiting};
use equipe_protocol::{ExecutionState, Priority, Ticket, TicketId, TicketStatus};

/// Points for each ticket blocked.
pub const BLOCKED_EACH: u32 = 10;
/// Tickets blocked that count: past this, more doesn't make it matter more.
pub const BLOCKED_CAP: u32 = 10;
/// Points for the highest priority among what it blocks (and its own ticket).
pub const PRIORITY_URGENT: u32 = 60;
pub const PRIORITY_HIGH: u32 = 30;
/// A normal ticket blocked counts a little; one's own normal ticket counts nothing.
pub const PRIORITY_NORMAL_BLOCKED: u32 = 5;
/// Points when deploy or release work waits on it, or the PM said to hold deploys for it.
pub const DEPLOY_HOLD: u32 = 50;
/// Points for each member parked or idle because of it.
pub const IDLE_MEMBER: u32 = 12;
/// Members that count.
pub const IDLE_CAP: u32 = 3;

/// One thing that waits on the person, as the scoring needs to know it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiter {
    pub kind: WaitKind,
    /// The tickets it is about (a permission's ticket, a waiting ticket, the
    /// tickets of a card to confirm): what it blocks is what waits on them.
    pub tickets: Vec<TicketId>,
    /// Tickets it holds up directly, besides what waits on `tickets` (the
    /// tickets a change to the team would put hands on).
    pub holding: Vec<TicketId>,
    /// Since when it has waited (unix seconds); 0: not known, which is last among equals.
    pub since: u64,
    /// Members parked or idle because of it: a run paused on a request, a lane blocked.
    pub idle_members: u32,
    /// It holds a merge or deploy by its nature: accepted work not in main.
    pub holds_deploy: bool,
}

impl Waiter {
    pub fn new(kind: WaitKind) -> Self {
        Waiter { kind, tickets: Vec::new(), holding: Vec::new(), since: 0, idle_members: 0, holds_deploy: false }
    }
}

/// Whether a ticket is work a person wants done: held up, it costs them.
/// Ideas and proposals are not yet work.
fn is_work(t: &Ticket) -> bool {
    matches!(t.status, TicketStatus::Todo | TicketStatus::InProgress | TicketStatus::Waiting | TicketStatus::Review)
}

/// Commands and paths that make a ticket deploy work whatever language it is
/// written in: they are what the work touches or runs, not words of a
/// language. Matched in the ticket's title, body and the PM's note.
const DEPLOY_MARKERS: [&str; 18] = [
    ".github/workflows/deploy",
    ".github/workflows/release",
    ".github/workflows/publish",
    "cdk deploy",
    "terraform apply",
    "kubectl apply",
    "helm upgrade",
    "serverless deploy",
    "sls deploy",
    "vercel deploy",
    "vercel --prod",
    "fly deploy",
    "wrangler deploy",
    "firebase deploy",
    "npm publish",
    "cargo publish",
    "docker push",
    "deploy.sh",
];

/// Whether `word` is written without spaces between words (Han, kana,
/// Hangul): it is looked for anywhere in the text, not at the start of a word.
fn unspaced(word: &str) -> bool {
    word.chars().any(|c| matches!(c, '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}' | '\u{3040}'..='\u{30FF}' | '\u{4E00}'..='\u{9FFF}'))
}

/// Whether `text` holds one of `words` (a locale's `|`-separated lists, every
/// language's together): a word of a spaced language at the start of a word
/// of the text (`shipping`, `Deployments`); a phrase, or a word of an unspaced
/// language, anywhere in it.
fn has_word(text: &str, words: &[&str]) -> bool {
    let text = text.to_lowercase();
    let tokens: Vec<&str> = text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    words.iter().map(|w| w.to_lowercase()).any(|w| if unspaced(&w) || w.chars().any(|c| !c.is_alphanumeric()) { text.contains(&w) } else { tokens.iter().any(|t| t.starts_with(&w)) })
}

/// The words that say deploy or release work, in every language equipe speaks
/// (`importance.deploy_words` in the locale files).
fn deploy_word(text: &str) -> bool {
    has_word(text, crate::i18n::every("importance.deploy_words"))
}

fn deploy_marker(text: &str) -> bool {
    let text = text.to_lowercase();
    DEPLOY_MARKERS.iter().any(|m| text.contains(m))
}

/// Whether a ticket is deploy or release work: it touches a deploy workflow or
/// runs a deploy command (in its title or body, in any language), or its title
/// says so in any of the languages equipe speaks.
pub fn is_deploy_work(t: &Ticket) -> bool {
    deploy_marker(&t.title) || deploy_marker(&t.body) || deploy_word(&t.title)
}

/// Whether the PM's note on a ticket says to hold deploys for it ("Hold
/// every deploy until the migration is approved"), in any language
/// (`importance.hold_words`).
pub fn pm_holds_deploys(t: &Ticket) -> bool {
    t.note.as_deref().is_some_and(|n| (deploy_word(n) || deploy_marker(n)) && has_word(n, crate::i18n::every("importance.hold_words")))
}

fn priority_points(priority: Priority, own: bool) -> u32 {
    match priority {
        Priority::Urgent => PRIORITY_URGENT,
        Priority::High => PRIORITY_HIGH,
        Priority::Normal if !own => PRIORITY_NORMAL_BLOCKED,
        Priority::Normal | Priority::Low => 0,
    }
}

/// The board as a graph of what waits on what, built once for many scores.
pub struct Graph<'a> {
    by_id: HashMap<TicketId, &'a Ticket>,
    /// Ticket → the tickets that list it in `depends_on`.
    dependents: HashMap<TicketId, Vec<TicketId>>,
}

impl<'a> Graph<'a> {
    pub fn new(tickets: &'a [Ticket]) -> Self {
        let mut dependents: HashMap<TicketId, Vec<TicketId>> = HashMap::new();
        for t in tickets {
            for dep in &t.depends_on {
                dependents.entry(*dep).or_default().push(t.id);
            }
        }
        Graph { by_id: tickets.iter().map(|t| (t.id, t)).collect(), dependents }
    }

    /// Every open work ticket that waits on any of `roots`, directly or
    /// through others, once each, `roots` themselves left out; and `held`,
    /// held up as they are, with what waits on them. A cycle ends.
    pub fn blocked(&self, roots: &[TicketId], held: &[TicketId]) -> Vec<&'a Ticket> {
        let mut seen: HashSet<TicketId> = roots.iter().copied().collect();
        let mut queue: VecDeque<TicketId> = roots.iter().copied().collect();
        let mut out = Vec::new();
        for id in held {
            if let Some(t) = self.by_id.get(id).copied()
                && seen.insert(*id)
                && !t.status.is_closed()
            {
                queue.push_back(*id);
                if is_work(t) {
                    out.push(t);
                }
            }
        }
        while let Some(id) = queue.pop_front() {
            for next in self.dependents.get(&id).into_iter().flatten() {
                let Some(t) = self.by_id.get(next).copied() else { continue };
                // A finished or dropped ticket no longer waits; what hangs from it hangs from something else.
                if !seen.insert(*next) || t.status.is_closed() {
                    continue;
                }
                queue.push_back(*next);
                if is_work(t) {
                    out.push(t);
                }
            }
        }
        out
    }

    /// What `waiter` holds up.
    pub fn impact(&self, waiter: &Waiter) -> Impact {
        let blocked = self.blocked(&waiter.tickets, &waiter.holding);
        let own: Vec<&Ticket> = waiter.tickets.iter().filter_map(|id| self.by_id.get(id).copied()).filter(|t| !t.status.is_closed()).collect();
        let blocks = blocked.len() as u32;
        let holds_deploy = waiter.holds_deploy || blocked.iter().any(|t| is_deploy_work(t)) || own.iter().any(|t| pm_holds_deploys(t));
        let priority = blocked.iter().chain(own.iter()).map(|t| t.priority).min();
        let points = blocked
            .iter()
            .map(|t| priority_points(t.priority, false))
            .chain(own.iter().map(|t| priority_points(t.priority, true)))
            .max()
            .unwrap_or(0);
        let score = BLOCKED_EACH * blocks.min(BLOCKED_CAP) + points + if holds_deploy { DEPLOY_HOLD } else { 0 } + IDLE_MEMBER * waiter.idle_members.min(IDLE_CAP);
        let mut impact = Impact { score, blocks, priority, holds_deploy, idle_members: waiter.idle_members, reason: String::new() };
        impact.reason = reason(&impact);
        impact
    }
}

/// The facts of an impact in one short English line, for the PM.
pub fn reason(impact: &Impact) -> String {
    let mut parts = Vec::new();
    if impact.blocks > 0 {
        let urgent = impact.priority.filter(|p| matches!(p, Priority::Urgent | Priority::High)).map(|p| format!(", {}", p.label())).unwrap_or_default();
        parts.push(format!("blocks {} ticket{}{urgent}", impact.blocks, if impact.blocks == 1 { "" } else { "s" }));
    } else if let Some(p) = impact.priority.filter(|p| matches!(p, Priority::Urgent | Priority::High)) {
        parts.push(format!("is {} itself", p.label()));
    }
    if impact.holds_deploy {
        parts.push("holds deploys".to_owned());
    }
    if impact.idle_members > 0 {
        parts.push(format!("{} member{} idle", impact.idle_members, if impact.idle_members == 1 { "" } else { "s" }));
    }
    if parts.is_empty() { "blocks nothing".to_owned() } else { parts.join(", ") }
}

/// The order of the more important first: higher score, then the longer
/// waiting (an unknown time last), then as given.
pub fn order(waiters: &[Waiter], impacts: &[Impact]) -> Vec<usize> {
    let mut at: Vec<usize> = (0..waiters.len()).collect();
    at.sort_by_key(|&i| (std::cmp::Reverse(impacts[i].score), if waiters[i].since == 0 { u64::MAX } else { waiters[i].since }, i));
    at
}

/// Every waiter's impact, and the order of importance as indices into `waiters`.
pub fn rank(tickets: &[Ticket], waiters: &[Waiter]) -> (Vec<Impact>, Vec<usize>) {
    let graph = Graph::new(tickets);
    let impacts: Vec<Impact> = waiters.iter().map(|w| graph.impact(w)).collect();
    let by = order(waiters, &impacts);
    (impacts, by)
}

// ── the daemon's list ──

/// A thing waiting on the person that is not a ticket on its own: the
/// daemon's runtime gathers these (requests, questions, cards, secrets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub kind: WaitKind,
    /// What is asked, in a line.
    pub label: String,
    /// The tickets it is about.
    pub tickets: Vec<TicketId>,
    /// The tickets it holds up directly ([`Waiter::holding`]).
    pub holding: Vec<TicketId>,
    pub since: u64,
    pub idle_members: u32,
}

/// The kind a ticket waits for the person as, if it does: the same rule as
/// the Status screen's count, and the unmerged accepted work besides.
fn ticket_kind(t: &Ticket, pm_accepts: bool) -> Option<WaitKind> {
    match t.status {
        TicketStatus::Waiting => Some(WaitKind::Ticket),
        TicketStatus::InProgress if t.execution.as_ref().is_some_and(|e| e.state == ExecutionState::Stalled) => Some(WaitKind::Stalled),
        TicketStatus::Review if !pm_accepts && t.ready_for_review() => Some(WaitKind::Review),
        // Where the PM sorts the team's proposals, only its own are the person's: not told here.
        TicketStatus::Proposed if !pm_accepts => Some(WaitKind::Proposal),
        TicketStatus::Done if t.waits_to_merge() => Some(WaitKind::NotOnMain),
        _ => None,
    }
}

/// Everything waiting on the person, the most blocking first: `tickets` that
/// wait on them, and `asks` the runtime holds. All of it, scored.
pub fn waiting_list(tickets: &[Ticket], asks: &[Ask], pm_accepts: bool) -> Vec<Waiting> {
    let mut waiters = Vec::new();
    let mut shown: Vec<(Option<TicketId>, String)> = Vec::new();
    for t in tickets {
        let Some(kind) = ticket_kind(t, pm_accepts) else { continue };
        waiters.push(Waiter {
            kind,
            tickets: vec![t.id],
            holding: Vec::new(),
            since: t.updated_at.unwrap_or(0),
            // A run that stopped leaves its member with nothing to do on it.
            idle_members: u32::from(kind == WaitKind::Stalled),
            holds_deploy: kind == WaitKind::NotOnMain,
        });
        shown.push((Some(t.id), t.title.clone()));
    }
    for a in asks {
        waiters.push(Waiter { kind: a.kind, tickets: a.tickets.clone(), holding: a.holding.clone(), since: a.since, idle_members: a.idle_members, holds_deploy: false });
        shown.push((a.tickets.first().copied(), a.label.clone()));
    }
    let (impacts, by) = rank(tickets, &waiters);
    by.into_iter()
        .map(|i| {
            let (ticket, title) = shown[i].clone();
            let found = ticket.and_then(|id| tickets.iter().find(|t| t.id == id));
            let status = found.map(|t| t.status);
            // A ticket put down for the person says what they have to do, in plain words, and item by item.
            let waits = found.filter(|t| t.status == TicketStatus::Waiting && waiters[i].kind == WaitKind::Ticket);
            let title = waits.and_then(Ticket::ask_headline).unwrap_or(title);
            let asks = waits.map(Ticket::asks).unwrap_or_default();
            Waiting { kind: waiters[i].kind, ticket, status, title, since: Some(waiters[i].since).filter(|s| *s > 0), impact: impacts[i].clone(), asks, request: None }
        })
        .collect()
}

/// Something waiting on the person this long is stale: the PM brings it up
/// with them now, or finds a way around it (unix seconds).
pub const STALE_AFTER: u64 = 3 * 3600;

/// At most this many waiting things are named to the PM, and to the person
/// in one line.
pub const PM_SHOWN: usize = 8;
pub const PERSON_SHOWN: usize = 4;

/// Each item of `list` whose ticket serves one of the person's requests
/// names it (`request_of`): what the person asked for that it holds up.
pub fn hold_requests(list: &mut [Waiting], request_of: impl Fn(TicketId) -> Option<HeldRequest>) {
    for w in list.iter_mut() {
        w.request = w.ticket.and_then(&request_of);
    }
}

fn cut(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    format!("{}…", text.chars().take(max.saturating_sub(1)).collect::<String>().trim_end())
}

/// How long `w` has waited at `now`, when known.
fn waited(w: &Waiting, now: u64) -> Option<u64> {
    w.since.map(|s| now.saturating_sub(s))
}

/// Whether `w` has waited [`STALE_AFTER`] or more.
pub fn is_stale(w: &Waiting, now: u64) -> bool {
    waited(w, now).is_some_and(|d| d >= STALE_AFTER)
}

/// One waiting thing as the PM reads it: what, what it holds up, how long
/// it waited (and that it is stale), the request it holds up, what it asks.
fn pm_line(w: &Waiting, now: u64) -> String {
    let what = cut(&w.ticket.map_or_else(|| w.title.clone(), |id| format!("T-{id} {}", w.title)), 90);
    let mut facts = vec![w.impact.reason.clone()];
    if let Some(d) = waited(w, now) {
        facts.push(format!("waiting {}{}", ago(d), if d >= STALE_AFTER { " — STALE" } else { "" }));
    }
    if let Some(r) = &w.request {
        facts.push(format!("holds up R-{} “{}”", r.id, r.title));
    }
    if !w.asks.is_empty() {
        facts.push(format!("asks: {}", w.asks.iter().map(|a| cut(a.lines().next().unwrap_or_default(), 60)).collect::<Vec<_>>().join("; ")));
    }
    format!("- {what} — {}", facts.into_iter().filter(|f| !f.is_empty()).collect::<Vec<_>>().join(", "))
}

/// The PM's note of everything waiting on the person, for it to remind them
/// of the right things: the first `n` (the most blocking first), each with
/// what it holds up, how long it waited, the request it holds up and what it
/// asks; those waiting [`STALE_AFTER`] or more marked, to bring up now or
/// get around. `None` when nothing waits.
pub fn pm_note(list: &[Waiting], n: usize, now: u64) -> Option<String> {
    if list.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = list.iter().take(n).map(|w| pm_line(w, now)).collect();
    if list.len() > n {
        lines.push(format!("- … and {} more (`board` with `status: waiting` lists them)", list.len() - n));
    }
    let mut text = format!(
        "Waiting on the person ({} in all), the most blocking first. When you remind them, lead with these and say in plain words what each holds up:\n{}",
        list.len(),
        lines.join("\n")
    );
    let stale = list.iter().filter(|w| is_stale(w, now)).count();
    if stale > 0 {
        text.push_str(&format!(
            "\n{stale} of them waited {} or more (STALE): bring each up with the person now, in plain words and with what it holds up — \
or find a way around it (another route, a sensible default they can change later) so the work doesn't stay stuck.",
            ago(STALE_AFTER)
        ));
    }
    Some(text)
}

/// What is told to the PM about the things that just went stale: each in a
/// line, and what to do. `None` when none did.
pub fn stale_message(newly: &[&Waiting], now: u64) -> Option<String> {
    if newly.is_empty() {
        return None;
    }
    let lines: Vec<String> = newly.iter().map(|w| pm_line(w, now)).collect();
    Some(format!(
        "[Waiting on the person · {}+] These have waited on the person {} or more:\n{}\n\nBring each up with them now, in plain words: \
what to do, and what it holds up. Where you can, find a way around it instead — another route, or a sensible default they can change \
later — and tell them what you chose.",
        ago(STALE_AFTER),
        ago(STALE_AFTER),
        lines.join("\n")
    ))
}

/// The items of `list` stale at `now` that `told` doesn't hold yet (by key).
pub fn newly_stale<'a>(list: &'a [Waiting], told: &HashSet<String>, now: u64) -> Vec<&'a Waiting> {
    list.iter().filter(|w| is_stale(w, now) && !told.contains(&key(w))).collect()
}

/// A waiting thing's key, the same as escalation's: its ticket, else its kind and words.
pub fn key(w: &Waiting) -> String {
    w.ticket.map_or_else(|| format!("{:?}:{}", w.kind, w.title), |id| format!("t{id}"))
}

/// Everything waiting on the person in one line, in `words`' language: how
/// many, then the first few in plain words with how long each waited.
/// `None` when nothing waits.
pub fn person_line(list: &[Waiting], words: &crate::i18n::Catalog, now: u64) -> Option<String> {
    if list.is_empty() {
        return None;
    }
    let mut items: Vec<String> = list
        .iter()
        .take(PERSON_SHOWN)
        .map(|w| {
            let what = cut(&w.title, 48);
            match waited(w, now) {
                Some(d) => format!("{what} ({})", ago(d)),
                None => what,
            }
        })
        .collect();
    if list.len() > PERSON_SHOWN {
        items.push(words.fill("request.more", &[("count", (list.len() - PERSON_SHOWN).to_string())]));
    }
    Some(words.fill("waiting.headline", &[("count", list.len().to_string()), ("list", items.join(" · "))]))
}

/// Whether the person is told what waits on them now: something new waits
/// that wasn't told (by key), and the last time was [`TELL_EVERY`] ago or
/// more — or never.
pub fn tell_person(list: &[Waiting], told: &HashSet<String>, last: Option<u64>, now: u64) -> bool {
    list.iter().any(|w| !told.contains(&key(w))) && last.is_none_or(|at| now.saturating_sub(at) >= TELL_EVERY)
}

/// The person is told what waits on them at most this often (unix seconds).
pub const TELL_EVERY: u64 = 30 * 60;

fn ago(secs: u64) -> String {
    match secs {
        s if s >= 86_400 => format!("{}d", s / 86_400),
        s if s >= 3_600 => format!("{}h", s / 3_600),
        s => format!("{}m", s / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(id: TicketId, title: &str, status: TicketStatus, priority: Priority, depends_on: &[TicketId]) -> Ticket {
        Ticket {
            id,
            title: title.into(),
            body: String::new(),
            status,
            kind: Default::default(),
            owner: None,
            assignee: None,
            project: None,
            depends_on: depends_on.to_vec(),
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            created_by: "you".into(),
            note: None,
            asks: Vec::new(),
            label: None, remaining: Vec::new(),
            images: Vec::new(),
            priority,
            rank: 0,
            updated_at: None,
            started_at: None,
            app: None,
            design_qa: None, landing: false,
            plan: None,
            model: None,
            flow: None,
        }
    }

    fn todo(id: TicketId, deps: &[TicketId]) -> Ticket {
        ticket(id, &format!("work {id}"), TicketStatus::Todo, Priority::Normal, deps)
    }

    fn on(ids: &[TicketId]) -> Waiter {
        Waiter { tickets: ids.to_vec(), ..Waiter::new(WaitKind::Ticket) }
    }

    #[test]
    fn what_waits_on_it_counts_through_other_tickets_once_each() {
        // 2 and 3 wait on 1; 4 on 2 and on 3 (once); 5 on 4; 6 doesn't.
        let tickets = vec![ticket(1, "sign in", TicketStatus::Waiting, Priority::Normal, &[]), todo(2, &[1]), todo(3, &[1]), todo(4, &[2, 3]), todo(5, &[4]), todo(6, &[])];
        let impact = Graph::new(&tickets).impact(&on(&[1]));
        assert_eq!(impact.blocks, 4, "{impact:?}");
        assert_eq!(impact.score, 4 * BLOCKED_EACH + PRIORITY_NORMAL_BLOCKED);
        assert!(impact.is_blocking());
    }

    #[test]
    fn finished_dropped_and_not_yet_work_do_not_count_and_a_cycle_ends() {
        let tickets = vec![
            ticket(1, "sign in", TicketStatus::Waiting, Priority::Normal, &[3]),
            ticket(2, "done", TicketStatus::Done, Priority::Urgent, &[1]),
            ticket(3, "loops back", TicketStatus::Todo, Priority::Normal, &[1]),
            ticket(4, "idea", TicketStatus::Idea, Priority::Normal, &[1]),
            ticket(5, "proposed", TicketStatus::Proposed, Priority::Normal, &[1]),
            ticket(6, "dropped", TicketStatus::Cancelled, Priority::Normal, &[1]),
        ];
        let impact = Graph::new(&tickets).impact(&on(&[1]));
        assert_eq!(impact.blocks, 1, "only 3: {impact:?}");
    }

    #[test]
    fn the_highest_priority_among_what_it_blocks_and_its_own_counts_once() {
        let tickets = vec![
            ticket(1, "own", TicketStatus::Waiting, Priority::Normal, &[]),
            ticket(2, "a", TicketStatus::Todo, Priority::Low, &[1]),
            ticket(3, "b", TicketStatus::Todo, Priority::High, &[1]),
        ];
        let impact = Graph::new(&tickets).impact(&on(&[1]));
        assert_eq!((impact.priority, impact.blocks), (Some(Priority::High), 2));
        assert_eq!(impact.score, 2 * BLOCKED_EACH + PRIORITY_HIGH);
        assert!(impact.is_pressing());
        let urgent_itself = vec![ticket(1, "own", TicketStatus::Waiting, Priority::Urgent, &[])];
        let impact = Graph::new(&urgent_itself).impact(&on(&[1]));
        assert_eq!((impact.blocks, impact.score, impact.is_blocking()), (0, PRIORITY_URGENT, true), "an urgent ticket waiting is itself pressing");
    }

    #[test]
    fn deploy_work_behind_it_or_a_pm_hold_note_holds_deploys() {
        let tickets = vec![
            ticket(1, "approve the production migration", TicketStatus::Waiting, Priority::Normal, &[]),
            ticket(2, "Deploy the API to production", TicketStatus::Todo, Priority::Normal, &[1]),
            ticket(3, "approve a design", TicketStatus::Waiting, Priority::Normal, &[]),
            todo(4, &[3]),
        ];
        let graph = Graph::new(&tickets);
        let migration = graph.impact(&on(&[1]));
        assert!(migration.holds_deploy);
        assert_eq!(migration.score, BLOCKED_EACH + PRIORITY_NORMAL_BLOCKED + DEPLOY_HOLD);
        assert!(!graph.impact(&on(&[3])).holds_deploy);
        let mut held = tickets.clone();
        held[2].note = Some("Hold every release until this is picked.".into());
        assert!(Graph::new(&held).impact(&on(&[3])).holds_deploy, "the PM's hold note");
        let unmerged = Waiter { holds_deploy: true, ..Waiter::new(WaitKind::NotOnMain) };
        assert!(graph.impact(&unmerged).holds_deploy);
        assert!(!is_deploy_work(&todo(9, &[])) && deploy_word("Ship it") && deploy_word("v2 rollout") && !deploy_word("relation"));
    }

    #[test]
    fn a_parked_member_counts_and_a_request_that_blocks_nothing_is_least() {
        let tickets = vec![ticket(1, "pick", TicketStatus::Waiting, Priority::Normal, &[]), todo(2, &[1])];
        let graph = Graph::new(&tickets);
        let parked = Waiter { idle_members: 2, ..Waiter::new(WaitKind::Permission) };
        assert_eq!(graph.impact(&parked).score, 2 * IDLE_MEMBER);
        assert!(!graph.impact(&parked).is_blocking(), "a parked member is not blocked work");
        assert_eq!(graph.impact(&Waiter { idle_members: 9, ..parked.clone() }).score, IDLE_CAP * IDLE_MEMBER, "capped");
        assert_eq!(graph.impact(&Waiter::new(WaitKind::Decision)).score, 0);
        assert!(graph.impact(&on(&[1])).score > graph.impact(&Waiter { idle_members: 1, ..Waiter::new(WaitKind::Permission) }).score, "one blocked ticket beats one idle member");
    }

    #[test]
    fn the_squads_example_orders_the_sign_in_and_the_migration_before_the_design_picks() {
        let mut tickets = vec![
            ticket(1, "AWS admin sign-in", TicketStatus::Waiting, Priority::Normal, &[]),
            ticket(2, "Approve the production DB migration", TicketStatus::Waiting, Priority::Normal, &[]),
            ticket(3, "Pick the hero image", TicketStatus::Waiting, Priority::Normal, &[]),
            ticket(4, "Pick the font", TicketStatus::Waiting, Priority::Normal, &[]),
            ticket(5, "Deploy the servers", TicketStatus::Todo, Priority::Normal, &[2]),
        ];
        tickets.extend((10..15).map(|id| todo(id, &[1])));
        tickets.extend((20..23).map(|id| todo(id, &[2])));
        tickets.push(todo(30, &[3]));
        tickets.push(todo(31, &[4]));
        let waiters = vec![on(&[3]), on(&[1]), Waiter::new(WaitKind::Permission), on(&[2]), on(&[4])];
        let (impacts, by) = rank(&tickets, &waiters);
        assert_eq!(by, vec![3, 1, 0, 4, 2], "migration (holds deploys), sign-in, the picks by age/as given, the approval last: {impacts:?}");
        assert_eq!(impacts[3].blocks, 4);
        assert_eq!(impacts[1].blocks, 5);
    }

    #[test]
    fn a_change_to_the_team_holds_the_tickets_it_would_put_hands_on_and_what_waits_on_them() {
        let tickets = vec![todo(1, &[]), todo(2, &[1]), todo(3, &[]), ticket(4, "idea", TicketStatus::Idea, Priority::Normal, &[])];
        let team = Waiter { holding: vec![1, 3, 4, 99], ..Waiter::new(WaitKind::Decision) };
        let impact = Graph::new(&tickets).impact(&team);
        assert_eq!(impact.blocks, 3, "1, 3 and what waits on 1: not an idea, not a ticket that isn't there");
    }

    #[test]
    fn a_tie_goes_to_the_one_waiting_longest_and_an_unknown_time_goes_last() {
        let tickets = vec![todo(1, &[])];
        let w = |since| Waiter { since, ..Waiter::new(WaitKind::Decision) };
        let waiters = vec![w(500), w(0), w(100), w(300)];
        let (_, by) = rank(&tickets, &waiters);
        assert_eq!(by, vec![2, 3, 0, 1]);
    }

    #[test]
    fn the_daemons_list_scores_tickets_and_asks_together_and_says_why() {
        let mut tickets = vec![
            ticket(1, "AWS admin sign-in", TicketStatus::Waiting, Priority::Normal, &[]),
            todo(2, &[1]),
            todo(3, &[1]),
            ticket(4, "handed in", TicketStatus::Review, Priority::Normal, &[]),
            ticket(5, "stopped run", TicketStatus::InProgress, Priority::Normal, &[]),
            ticket(6, "accepted, unmerged", TicketStatus::Done, Priority::Normal, &[]),
        ];
        tickets[1].updated_at = Some(1_000);
        tickets[3].updated_at = Some(500);
        tickets[3].branch = Some("b".into());
        tickets[4].execution = Some(Box::new(equipe_protocol::TicketExecution {
            state: ExecutionState::Stalled,
            session: 1,
            operation: String::new(),
            generation: 1,
            heartbeat_at: 0,
            lease_expires_at: 0,
            reason: None,
            pause: None,
            head: None,
            dirty: None,
            waited_for: None,
        }));
        tickets[5].branch = Some("b".into());
        tickets[5].merged = Some(false);
        let asks = vec![Ask { kind: WaitKind::Permission, label: "run rm".into(), tickets: vec![], holding: vec![], since: 10, idle_members: 1 }];
        let list = waiting_list(&tickets, &asks, false);
        let kinds: Vec<WaitKind> = list.iter().map(|w| w.kind).collect();
        assert_eq!(list.iter().map(|w| w.kind).collect::<Vec<_>>(), vec![WaitKind::NotOnMain, WaitKind::Ticket, WaitKind::Permission, WaitKind::Stalled, WaitKind::Review], "{list:#?}");
        assert_eq!(list[1].ticket, Some(1));
        assert_eq!(list[1].impact.reason, "blocks 2 tickets");
        assert!(kinds.contains(&WaitKind::Review) && kinds.contains(&WaitKind::Stalled) && kinds.contains(&WaitKind::NotOnMain) && kinds.contains(&WaitKind::Permission));
        let unmerged = list.iter().find(|w| w.kind == WaitKind::NotOnMain).unwrap();
        assert!(unmerged.impact.holds_deploy && unmerged.impact.reason.contains("holds deploys"));
        // Where the PM accepts, handed-in work is not the person's.
        assert!(!waiting_list(&tickets, &[], true).iter().any(|w| w.kind == WaitKind::Review));
    }

    #[test]
    fn the_pms_note_names_the_top_few_and_what_holds_nothing_up_too() {
        let tickets = vec![ticket(1, "AWS admin sign-in", TicketStatus::Waiting, Priority::Normal, &[]), todo(2, &[1]), ticket(3, "Pick a font", TicketStatus::Waiting, Priority::Normal, &[]), todo(4, &[3])];
        let mut list = waiting_list(&tickets, &[], false);
        list[0].since = Some(1_000);
        let note = pm_note(&list, 1, 1_000 + 3 * 86_400).unwrap();
        assert!(note.contains("T-1 AWS admin sign-in — blocks 1 ticket, waiting 3d — STALE") && !note.contains("Pick a font"), "{note}");
        assert!(note.contains("(2 in all)") && note.contains("… and 1 more") && note.contains("1 of them waited 3h or more"), "{note}");
        // Holding nothing up, it is still named: the person has it to do all the same.
        let quiet = waiting_list(&[ticket(9, "pick", TicketStatus::Waiting, Priority::Normal, &[])], &[], false);
        assert!(pm_note(&quiet, 3, 0).unwrap().contains("T-9 pick — blocks nothing"));
        assert_eq!(pm_note(&[], 3, 0), None);
    }

    /// The incident's morning: tickets put down for the person with their checklists, a card to
    /// accept, questions, a sign-in — all of it gathered, in plain words, the most blocking first.
    fn morning() -> (Vec<Ticket>, Vec<Ask>) {
        let mut aws = ticket(1, "Infra: AWS console access", TicketStatus::Waiting, Priority::Normal, &[]);
        aws.asks = vec!["Sign in to the AWS console in the browser card".into()];
        aws.note = Some(equipe_protocol::asks::waiting_note_headed(&aws.asks, None, Some("Sign in to AWS")));
        aws.updated_at = Some(1_000);
        let mut draft = ticket(2, "ERP drafts cleanup", TicketStatus::Waiting, Priority::Normal, &[]);
        // From an older daemon: the checklist only in its note.
        draft.note = Some("Waiting on you:\n- [ ] Delete the test ERP draft\n- [ ] Say which approval line to use".into());
        draft.updated_at = Some(5_000);
        let mut unlock = ticket(3, "Desktop run", TicketStatus::Waiting, Priority::Normal, &[]);
        unlock.asks = vec!["Unlock the Mac's screen".into()];
        unlock.updated_at = Some(9_000);
        let build = todo(4, &[3]);
        let asks = vec![
            Ask { kind: WaitKind::Decision, label: "Accept “Collection run”".into(), tickets: vec![5], holding: vec![], since: 2_000, idle_members: 0 },
            Ask { kind: WaitKind::SignIn, label: "Log in to the preview".into(), tickets: vec![], holding: vec![], since: 3_000, idle_members: 1 },
        ];
        (vec![aws, draft, unlock, build], asks)
    }

    #[test]
    fn everything_waiting_on_the_person_is_gathered_in_plain_words_with_what_it_asks() {
        let (tickets, asks) = morning();
        let mut list = waiting_list(&tickets, &asks, true);
        assert_eq!(list.len(), 5, "{list:#?}");
        let aws = list.iter().find(|w| w.ticket == Some(1)).unwrap();
        assert_eq!(aws.title, "Sign in to AWS", "its one-line ask, not the member's title");
        assert_eq!(aws.asks, ["Sign in to the AWS console in the browser card"]);
        let draft = list.iter().find(|w| w.ticket == Some(2)).unwrap();
        assert_eq!(draft.asks, ["Delete the test ERP draft", "Say which approval line to use"], "an older note's checklist");
        assert!(list.iter().any(|w| w.kind == WaitKind::Decision && w.title.contains("Collection run")));
        // The request each holds up.
        hold_requests(&mut list, |t| (t == 3).then(|| HeldRequest { id: 7, title: "Personal dashboards".into() }));
        assert_eq!(list.iter().find(|w| w.ticket == Some(3)).unwrap().request, Some(HeldRequest { id: 7, title: "Personal dashboards".into() }));
        let note = pm_note(&list, PM_SHOWN, 9_000 + STALE_AFTER).unwrap();
        assert!(note.contains("T-3 Unlock the Mac's screen") || note.contains("T-3 Desktop run"), "{note}");
        assert!(note.contains("holds up R-7 “Personal dashboards”") && note.contains("asks: Delete the test ERP draft; Say which approval line to use"), "{note}");
    }

    #[test]
    fn what_blocks_comes_first_then_the_longest_waiting() {
        let (tickets, asks) = morning();
        let list = waiting_list(&tickets, &asks, true);
        let order: Vec<String> = list.iter().map(key).collect();
        // T-3 holds T-4 up; the sign-in holds a member; then the rest, the oldest first.
        assert_eq!(order[0], "t3", "{order:?}");
        assert_eq!(order[1], "SignIn:Log in to the preview", "{order:?}");
        assert_eq!(&order[2..], ["t1", "t5", "t2"], "the card, by the ticket it is about: {order:?}");
    }

    #[test]
    fn the_person_reads_it_in_one_line_and_is_told_again_only_when_something_new_waits() {
        let (tickets, asks) = morning();
        let list = waiting_list(&tickets, &asks, true);
        let line = person_line(&list, crate::i18n::core(crate::i18n::Lang::En), 1_000 + 5 * 3600).unwrap();
        assert!(line.starts_with("Waiting on you (5): "), "{line}");
        assert!(line.contains("Sign in to AWS (5h)") && line.ends_with("+1 more"), "{line}");
        assert_eq!(person_line(&[], crate::i18n::core(crate::i18n::Lang::En), 0), None);
        let mut told: HashSet<String> = HashSet::new();
        assert!(tell_person(&list, &told, None, 10_000), "never told");
        told.extend(list.iter().map(key));
        assert!(!tell_person(&list, &told, Some(10_000), 10_000 + 2 * TELL_EVERY), "nothing new");
        told.remove("t2");
        assert!(!tell_person(&list, &told, Some(10_000), 10_000 + 60), "not twice within the half hour");
        assert!(tell_person(&list, &told, Some(10_000), 10_000 + TELL_EVERY), "something new, a while after");
    }

    #[test]
    fn what_waits_hours_goes_stale_once_for_the_pm() {
        let (tickets, asks) = morning();
        let list = waiting_list(&tickets, &asks, true);
        let now = 2_000 + STALE_AFTER;
        // T-1 (since 1000) and the card (2000) are stale; T-2, T-3 and the sign-in not yet.
        let stale: Vec<String> = newly_stale(&list, &HashSet::new(), now).into_iter().map(key).collect();
        assert_eq!(stale, ["t1", "t5"]);
        let told: HashSet<String> = stale.iter().cloned().collect();
        assert!(newly_stale(&list, &told, now).is_empty(), "told once");
        let message = stale_message(&newly_stale(&list, &HashSet::new(), now), now).unwrap();
        assert!(message.starts_with("[Waiting on the person · 3h+]") && message.contains("find a way around"), "{message}");
        assert_eq!(stale_message(&[], now), None);
    }

    fn titled(title: &str) -> Ticket {
        ticket(1, title, TicketStatus::Todo, Priority::Normal, &[])
    }

    #[test]
    fn deploy_titles_count_in_every_language_equipe_speaks() {
        for title in [
            "Deploy the API",
            "v2 rollout",
            "운영 배포",
            "배포",
            "릴리스 준비",
            "운영배포 일정",
            "デプロイ作業",
            "リリース",
            "部署到生产环境",
            "发布 2.0",
            "發佈新版本",
            "Bereitstellung der API",
            "Deployment vorbereiten",
            "Release-Notes veröffentlichen",
            "Déploiement du site",
            "Mise en production vendredi",
            "Despliegue a producción",
            "Lanzamiento de la versión 2",
        ] {
            assert!(is_deploy_work(&titled(title)), "{title}");
        }
        for title in ["Pick a font", "relation table", "글꼴 고르기", "フォントを選ぶ", "选择字体", "Schriftart wählen", "Choisir une police", "Elegir una fuente"] {
            assert!(!is_deploy_work(&titled(title)), "{title}");
        }
    }

    #[test]
    fn the_work_itself_says_it_deploys_whatever_the_title_is_written_in() {
        let mut t = titled("인프라 정리");
        assert!(!is_deploy_work(&t));
        t.body = "`.github/workflows/deploy-prod.yml` 수정, 그 다음 `cdk deploy`".into();
        assert!(is_deploy_work(&t));
        let mut u = titled("設定の見直し");
        u.body = "Runs npm publish at the end".into();
        assert!(is_deploy_work(&u));
    }

    #[test]
    fn a_pm_hold_note_works_in_any_language_and_needs_both_a_hold_and_a_deploy() {
        for note in [
            "Hold every release until this is picked.",
            "이게 끝날 때까지 모든 배포 보류",
            "デプロイは保留にしてください",
            "在此之前暂缓发布",
            "Bereitstellung bitte anhalten",
            "Suspendre le déploiement en attendant",
            "Retener el despliegue hasta que se apruebe",
        ] {
            let mut t = titled("x");
            t.note = Some(note.into());
            assert!(pm_holds_deploys(&t), "{note}");
        }
        for note in ["Done, merged.", "배포 완료", "Hold the font choice"] {
            let mut t = titled("x");
            t.note = Some(note.into());
            assert!(!pm_holds_deploys(&t), "{note}");
        }
    }

    #[test]
    fn every_language_lists_its_own_deploy_and_hold_words() {
        for lang in crate::i18n::Lang::ALL {
            for key in ["importance.deploy_words", "importance.hold_words"] {
                let words = crate::i18n::core(lang).own(key).unwrap_or_default();
                assert!(words.split('|').filter(|w| !w.is_empty()).count() >= 3, "{}: {key}", lang.code());
            }
        }
    }
}
