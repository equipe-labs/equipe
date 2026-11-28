//! The squad's self-watch: what a person keeping an eye on the squad would
//! notice by looking every so often, noticed by equipe itself
//! (docs/HEALTH.md). Each condition is found from the journal, the board and
//! a few git facts ([`Snapshot`]), told once to the PM (a line on its brief)
//! and to the person (news), and not again until it changes ([`Told`]).
//! `equipe health` prints the same findings.
//!
//! | finding | when |
//! |---|---|
//! | [`Finding::Stalled`] | a ticket in progress has had no turn for [`STALLED_AFTER`] and its member is free |
//! | [`Finding::Idle`] | nothing has run for [`IDLE_AFTER`] while tickets wait to be done — with what they wait on |
//! | [`Finding::Unpushed`] | a project's main has had commits origin lacks for [`UNPUSHED_AFTER`] |
//! | [`Finding::NotLive`] | a project that deploys has had commits on main not live for [`NOT_LIVE_AFTER`] |
//! | [`Finding::Waiting`] | things have waited on the person for [`WAITING_AFTER`] (the person's waiting list, `crate::importance`; the daemon tells that list itself, so the self-watch only shows it) |
//! | [`Finding::Bounced`] | one ticket went back [`BOUNCES`] times or more in [`BOUNCE_WINDOW`] |
//!
//! [`detect`] is pure over a snapshot, so each finding is tested on fixture
//! journals as of a later time. Git is read by [`repo_facts`], which blocks:
//! the daemon calls it off its loop.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use equipe_protocol::importance::{WaitKind, Waiting};
use equipe_protocol::{ExecutionState, Ticket, TicketId, TicketStatus};
use serde::{Deserialize, Serialize};

use crate::i18n::Catalog;
use crate::journal::Journal;
use crate::logins::span;

/// A ticket in progress with no turn this long, its member free, is stalled.
pub const STALLED_AFTER: u64 = 2 * HOUR;
/// Nothing running this long, with tickets to do, is an idle squad.
pub const IDLE_AFTER: u64 = 30 * 60;
/// Commits on main not pushed this long are told.
pub const UNPUSHED_AFTER: u64 = 10 * 60;
/// Commits on main not live this long are told, where a deploy is known.
pub const NOT_LIVE_AFTER: u64 = 30 * 60;
/// Things waiting on the person this long are told.
pub const WAITING_AFTER: u64 = 2 * HOUR;
/// A ticket sent or handed back this many times within [`BOUNCE_WINDOW`] is going in circles.
pub const BOUNCES: usize = 3;
pub const BOUNCE_WINDOW: u64 = 24 * HOUR;
/// How often the daemon looks.
pub const EVERY: u64 = 10 * 60;
/// The record kinds that send a ticket back: handed back by its member, sent back from review or design QA, reopened.
pub const BOUNCE_KINDS: [&str; 4] = ["handed back", "sent back", "design QA: changes", "reopened"];
/// The most tickets or items one finding names.
const NAMED: usize = 5;

const HOUR: u64 = 3600;

/// One project's main, as git has it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RepoFacts {
    /// As the board names it.
    pub project: String,
    pub path: PathBuf,
    /// Commits on the local main that origin's lacks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unpushed: Option<Ahead>,
    /// Commits on origin's main past what is live, where a deploy is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_live: Option<Ahead>,
}

/// Commits one ref has that another lacks, and since when (the oldest's commit time).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Ahead {
    pub commits: u32,
    pub since: u64,
}

/// What is running in the squad now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Live {
    /// Tickets a turn runs on now.
    pub running: HashSet<TicketId>,
    /// Members with a turn under way, or a run of theirs.
    pub busy: HashSet<String>,
    /// Any turn of the squad's under way, at a desk too — not the PM's off tickets.
    pub any: bool,
}

impl Live {
    /// What runs, as the journal alone can tell (no daemon to ask): tickets
    /// whose run holds a live lease and isn't paused.
    pub fn from_leases(tickets: &[Ticket], now: u64) -> Self {
        let mut live = Live::default();
        for t in tickets {
            let Some(e) = t.execution.as_ref() else { continue };
            if t.status == TicketStatus::InProgress && e.state == ExecutionState::Active && e.lease_expires_at > now && e.pause.is_none() {
                live.running.insert(t.id);
                if let Some(a) = &t.assignee {
                    live.busy.insert(a.clone());
                }
            }
        }
        live.any = !live.running.is_empty();
        live
    }
}

/// Everything the findings are made from, as of `now`.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub now: u64,
    pub tickets: Vec<Ticket>,
    /// When each open ticket's last turn ended.
    pub last_turn: HashMap<TicketId, u64>,
    /// When the squad's last turn ended.
    pub squad_last_turn: Option<u64>,
    pub live: Live,
    /// How many times each ticket went back within [`BOUNCE_WINDOW`].
    pub bounces: HashMap<TicketId, usize>,
    /// The person's waiting list ([`crate::importance::waiting_list`]).
    pub waiting: Vec<Waiting>,
    pub repos: Vec<RepoFacts>,
}

impl Snapshot {
    /// Gather a snapshot from the squad's journal, with `tickets` its board,
    /// `live` what runs, `asks` what waits on the person besides tickets and
    /// `repos` its projects' git facts.
    pub fn gather(journal: &Journal, tickets: Vec<Ticket>, now: u64, live: Live, asks: &[crate::importance::Ask], repos: Vec<RepoFacts>) -> Self {
        let open: Vec<TicketId> = tickets.iter().filter(|t| t.status == TicketStatus::InProgress).map(|t| t.id).collect();
        let last_turn = journal.ticket_last_turns(&open).unwrap_or_default();
        let squad_last_turn = journal.last_turn_ended().ok().flatten();
        let mut bounces: HashMap<TicketId, usize> = HashMap::new();
        for (ticket, e) in journal.ticket_events_since(now.saturating_sub(BOUNCE_WINDOW)).unwrap_or_default() {
            if BOUNCE_KINDS.contains(&e.kind.as_str()) && e.at <= now {
                *bounces.entry(ticket).or_default() += 1;
            }
        }
        let waiting = crate::importance::waiting_list(&tickets, asks, journal.pm_accepts());
        Snapshot { now, tickets, last_turn, squad_last_turn, live, bounces, waiting, repos }
    }
}

/// A ticket that holds up the squad's work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Holder {
    pub ticket: TicketId,
    pub title: String,
    pub status: TicketStatus,
    /// The tickets to do that wait on it.
    pub holds: usize,
}

/// One thing that waited on the person too long.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaitingItem {
    pub kind: WaitKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    pub title: String,
    /// How long it has waited.
    pub secs: u64,
}

/// One condition worth telling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Finding {
    Stalled { ticket: TicketId, title: String, member: Option<String>, quiet_secs: u64 },
    Idle { quiet_secs: u64, todo: usize, waits_on: Vec<Holder>, ready: usize },
    Unpushed { project: String, commits: u32, secs: u64 },
    NotLive { project: String, commits: u32, secs: u64 },
    Waiting { items: Vec<WaitingItem> },
    Bounced { ticket: TicketId, title: String, times: usize },
}

impl Finding {
    /// Which condition it is: one finding per key at a time.
    pub fn key(&self) -> String {
        match self {
            Finding::Stalled { ticket, .. } => format!("stalled:{ticket}"),
            Finding::Idle { .. } => "idle".to_owned(),
            Finding::Unpushed { project, .. } => format!("unpushed:{project}"),
            Finding::NotLive { project, .. } => format!("not_live:{project}"),
            Finding::Waiting { .. } => "waiting".to_owned(),
            Finding::Bounced { ticket, .. } => format!("bounced:{ticket}"),
        }
    }

    /// What, beyond its key, makes it news again when something new joins:
    /// what an idle squad waits on, what waits on the person.
    pub fn signature(&self) -> Vec<String> {
        match self {
            Finding::Idle { waits_on, ready, .. } => {
                let mut s: Vec<String> = waits_on.iter().map(|h| format!("T-{}", h.ticket)).collect();
                if *ready > 0 {
                    s.push("ready".to_owned());
                }
                s
            }
            Finding::Waiting { items } => items.iter().map(|i| format!("{:?}:{}:{}", i.kind, i.ticket.unwrap_or(0), i.title)).collect(),
            _ => Vec::new(),
        }
    }

    /// The PM's line, in English.
    pub fn pm_line(&self) -> String {
        match self {
            Finding::Stalled { ticket, title, member, quiet_secs } => format!(
                "T-{ticket} \"{title}\" is in progress but has had no turn for {}{}. Look at its record and branch: put it back to todo, \
give it to someone else, or cancel it.",
                span(*quiet_secs),
                member.as_deref().map(|m| format!(", and {m} is free")).unwrap_or_default()
            ),
            Finding::Idle { quiet_secs, todo, waits_on, ready } => {
                let mut line = format!("Nothing has run for {} while {todo} tickets wait to be done.", span(*quiet_secs));
                if !waits_on.is_empty() {
                    let named: Vec<String> = waits_on.iter().map(|h| format!("T-{} \"{}\" ({}, holds {})", h.ticket, h.title, status_words(h.status), h.holds)).collect();
                    line.push_str(&format!(" They wait on: {}.", named.join("; ")));
                }
                if *ready > 0 {
                    line.push_str(&format!(" {ready} are ready but didn't start: check their owners and the engines."));
                }
                line.push_str(" Unblock what you can — reorder, split or drop what waits — and bring what only the person can do to them first.");
                line
            }
            Finding::Unpushed { project, commits, secs } => {
                format!("{project}: main has {commits} commit(s) not pushed to origin, for {}. Pushing it is what makes it reach the team and go live.", span(*secs))
            }
            Finding::NotLive { project, commits, secs } => {
                format!("{project}: main has {commits} commit(s) pushed but not live, for {}. Check the project's deploy.", span(*secs))
            }
            Finding::Waiting { items } => {
                let named: Vec<String> = items.iter().map(|i| format!("{} ({}, {})", item_name(i), kind_words(i.kind), span(i.secs))).collect();
                format!("These have waited on the person for over {}: {}. Bring them up, most blocking first.", span(WAITING_AFTER), named.join("; "))
            }
            Finding::Bounced { ticket, title, times } => format!(
                "T-{ticket} \"{title}\" went back {times} times in the last day. Look at why before it goes round again: narrow it, \
reshape it, or put the question to the person."
            ),
        }
    }

    /// What the person reads, in their language.
    pub fn said(&self, words: &Catalog) -> String {
        match self {
            Finding::Stalled { ticket, title, member, quiet_secs } => match member {
                Some(m) => words.fill("health.stalled", &[("id", ticket.to_string()), ("title", title.clone()), ("ago", span(*quiet_secs)), ("name", m.clone())]),
                None => words.fill("health.stalled_unowned", &[("id", ticket.to_string()), ("title", title.clone()), ("ago", span(*quiet_secs))]),
            },
            Finding::Idle { quiet_secs, todo, waits_on, ready } => {
                let mut parts = vec![words.fill("health.idle", &[("ago", span(*quiet_secs)), ("count", todo.to_string())])];
                if !waits_on.is_empty() {
                    let named: Vec<String> = waits_on.iter().map(|h| format!("T-{} {}", h.ticket, h.title)).collect();
                    parts.push(words.fill("health.idle_waits_on", &[("tickets", named.join(", "))]));
                }
                if *ready > 0 {
                    parts.push(words.fill("health.idle_ready", &[("count", ready.to_string())]));
                }
                parts.join(" ")
            }
            Finding::Unpushed { project, commits, secs } => {
                words.fill("health.unpushed", &[("project", project.clone()), ("count", commits.to_string()), ("ago", span(*secs))])
            }
            Finding::NotLive { project, commits, secs } => {
                words.fill("health.not_live", &[("project", project.clone()), ("count", commits.to_string()), ("ago", span(*secs))])
            }
            Finding::Waiting { items } => {
                let named: Vec<String> = items.iter().map(|i| format!("{} ({})", item_name(i), span(i.secs))).collect();
                words.fill("health.waiting", &[("count", items.len().to_string()), ("ago", span(WAITING_AFTER)), ("items", named.join(", "))])
            }
            Finding::Bounced { ticket, title, times } => {
                words.fill("health.bounced", &[("id", ticket.to_string()), ("title", title.clone()), ("count", times.to_string())])
            }
        }
    }
}

/// What waits on the person, as the PM reads it.
fn kind_words(kind: WaitKind) -> &'static str {
    match kind {
        WaitKind::Permission => "a permission",
        WaitKind::Decision => "a decision",
        WaitKind::SignIn => "a sign-in",
        WaitKind::Ticket => "a ticket waiting on them",
        WaitKind::Stalled => "a stopped run",
        WaitKind::Review => "work to accept",
        WaitKind::Proposal => "a proposal",
        WaitKind::NotOnMain => "accepted work not on main",
        WaitKind::Other => "an ask",
    }
}

fn item_name(i: &WaitingItem) -> String {
    match i.ticket {
        Some(id) if !i.title.contains(&format!("T-{id}")) => format!("T-{id} {}", i.title),
        _ => i.title.clone(),
    }
}

/// A status as the PM reads it on an idle squad's holders.
fn status_words(status: TicketStatus) -> &'static str {
    match status {
        TicketStatus::Waiting => "waiting on the person",
        TicketStatus::Review => "in review",
        TicketStatus::Proposed => "a proposal not yet taken on",
        TicketStatus::Idea => "kept as an idea",
        TicketStatus::InProgress => "in progress",
        TicketStatus::Todo => "to do",
        TicketStatus::Done | TicketStatus::Cancelled => "closed",
    }
}

/// Every finding in `s`, in a stable order.
pub fn detect(s: &Snapshot) -> Vec<Finding> {
    let mut out = Vec::new();
    out.extend(stalled(s));
    out.extend(idle(s));
    for repo in &s.repos {
        if let Some(a) = repo.unpushed.filter(|a| a.commits > 0 && s.now.saturating_sub(a.since) >= UNPUSHED_AFTER) {
            out.push(Finding::Unpushed { project: repo.project.clone(), commits: a.commits, secs: s.now.saturating_sub(a.since) });
        }
        if let Some(a) = repo.not_live.filter(|a| a.commits > 0 && s.now.saturating_sub(a.since) >= NOT_LIVE_AFTER) {
            out.push(Finding::NotLive { project: repo.project.clone(), commits: a.commits, secs: s.now.saturating_sub(a.since) });
        }
    }
    out.extend(waiting(s));
    out.extend(bounced(s));
    out
}

/// Tickets in progress with no turn for [`STALLED_AFTER`], none running and
/// their member free. A run that stopped is the person's to bring back, and
/// on their waiting list already; so is a ticket waiting on them.
pub fn stalled(s: &Snapshot) -> Vec<Finding> {
    let mut out = Vec::new();
    for t in s.tickets.iter().filter(|t| t.status == TicketStatus::InProgress) {
        if s.live.running.contains(&t.id) || t.execution.as_ref().is_some_and(|e| matches!(e.state, ExecutionState::Stalled | ExecutionState::Waiting) || e.pause.is_some()) {
            continue;
        }
        if t.assignee.as_ref().is_some_and(|a| s.live.busy.contains(a)) {
            continue;
        }
        let last = s.last_turn.get(&t.id).copied().max(t.updated_at).unwrap_or(0);
        let quiet = s.now.saturating_sub(last);
        if last > 0 && quiet >= STALLED_AFTER {
            out.push(Finding::Stalled { ticket: t.id, title: t.title.clone(), member: t.assignee.clone(), quiet_secs: quiet });
        }
    }
    out
}

/// Nothing running for [`IDLE_AFTER`] while tickets wait to be done: what
/// they wait on — the tickets, not to do themselves, that hold them through
/// their dependencies — and how many were ready but didn't start.
pub fn idle(s: &Snapshot) -> Option<Finding> {
    if s.live.any || !s.live.running.is_empty() {
        return None;
    }
    let todo: Vec<&Ticket> = s.tickets.iter().filter(|t| t.status == TicketStatus::Todo).collect();
    if todo.is_empty() {
        return None;
    }
    // Quiet since the last turn ended, or — never a turn — since the board last changed.
    let since = s.squad_last_turn.or_else(|| s.tickets.iter().filter_map(|t| t.updated_at).max()).unwrap_or(0);
    let quiet = s.now.saturating_sub(since);
    if since == 0 || quiet < IDLE_AFTER {
        return None;
    }
    let by_id: HashMap<TicketId, &Ticket> = s.tickets.iter().map(|t| (t.id, t)).collect();
    let mut holders: BTreeMap<TicketId, usize> = BTreeMap::new();
    let mut ready = 0;
    for t in &todo {
        let roots = holding(t, &by_id);
        if roots.is_empty() {
            ready += 1;
        }
        for r in roots {
            *holders.entry(r).or_default() += 1;
        }
    }
    let mut waits_on: Vec<Holder> = holders
        .into_iter()
        .filter_map(|(id, holds)| by_id.get(&id).map(|t| Holder { ticket: id, title: t.title.clone(), status: t.status, holds }))
        .collect();
    waits_on.sort_by(|a, b| b.holds.cmp(&a.holds).then(a.ticket.cmp(&b.ticket)));
    waits_on.truncate(NAMED);
    Some(Finding::Idle { quiet_secs: quiet, todo: todo.len(), waits_on, ready })
}

/// The open tickets, not to do themselves, that `t` waits on through its
/// dependencies — directly, or through other tickets to do. A cycle ends.
fn holding(t: &Ticket, by_id: &HashMap<TicketId, &Ticket>) -> BTreeSet<TicketId> {
    let mut roots = BTreeSet::new();
    let mut seen: HashSet<TicketId> = HashSet::from([t.id]);
    let mut queue: Vec<TicketId> = t.depends_on.clone();
    while let Some(id) = queue.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(dep) = by_id.get(&id) else { continue };
        if dep.status.is_closed() {
            continue;
        }
        if dep.status == TicketStatus::Todo {
            queue.extend(dep.depends_on.iter().copied());
        } else {
            roots.insert(id);
        }
    }
    roots
}

/// What has waited on the person for [`WAITING_AFTER`] or more, most blocking first.
pub fn waiting(s: &Snapshot) -> Option<Finding> {
    let items: Vec<WaitingItem> = s
        .waiting
        .iter()
        .filter_map(|w| {
            let secs = s.now.saturating_sub(w.since?);
            (secs >= WAITING_AFTER).then(|| WaitingItem { kind: w.kind, ticket: w.ticket, title: w.title.clone(), secs })
        })
        .take(NAMED * 2)
        .collect();
    (!items.is_empty()).then_some(Finding::Waiting { items })
}

/// Open tickets that went back [`BOUNCES`] times or more within [`BOUNCE_WINDOW`].
pub fn bounced(s: &Snapshot) -> Vec<Finding> {
    let mut out: Vec<Finding> = s
        .tickets
        .iter()
        .filter(|t| !t.status.is_closed())
        .filter_map(|t| s.bounces.get(&t.id).filter(|n| **n >= BOUNCES).map(|n| Finding::Bounced { ticket: t.id, title: t.title.clone(), times: *n }))
        .collect();
    out.sort_by_key(|f| match f {
        Finding::Bounced { ticket, .. } => *ticket,
        _ => 0,
    });
    out
}

/// What was told, by finding key, with its signature then. A finding is told
/// again only after it went away, or when something new joins it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Told(pub BTreeMap<String, Vec<String>>);

impl Told {
    /// Of `found`, what is news; what went away is forgotten.
    pub fn fresh(&mut self, found: &[Finding]) -> Vec<Finding> {
        let keys: HashSet<String> = found.iter().map(Finding::key).collect();
        self.0.retain(|k, _| keys.contains(k));
        let mut out = Vec::new();
        for f in found {
            let (key, signature) = (f.key(), f.signature());
            let news = match self.0.get(&key) {
                None => true,
                Some(before) => signature.iter().any(|s| !before.contains(s)),
            };
            self.0.insert(key, signature);
            if news {
                out.push(f.clone());
            }
        }
        out
    }
}

// ── git ──

/// Where a project's main last went live, as equipe knows it: the commit, when
/// the project deploys some way equipe can see. Behind a trait so a better
/// record of going live plugs in without the finding changing.
pub trait LiveState {
    fn live_commit(&self, path: &Path) -> Option<String>;
}

/// What equipe keeps of how projects go live: a project's own go-live
/// command (`crate::go_live`) and the last commit it deployed, else the
/// deploy watch (`crate::deploys`) for a project that reports production
/// deployments or has a deploy workflow. Neither: nothing is known live.
pub struct DeployWatch<'a> {
    pub watch: &'a BTreeMap<String, crate::deploys::Watch>,
    pub lanes: &'a BTreeMap<String, crate::go_live::Lane>,
}

impl LiveState for DeployWatch<'_> {
    fn live_commit(&self, path: &Path) -> Option<String> {
        let key = path.to_string_lossy();
        if let Some(deployed) = self.lanes.get(key.as_ref()).and_then(|l| l.deployed.clone()) {
            return Some(deployed);
        }
        let watch = self.watch.get(key.as_ref())?;
        match watch.channel.as_ref()? {
            crate::deploys::Channel::Deployments | crate::deploys::Channel::Workflows { .. } => watch.live.clone(),
            crate::deploys::Channel::Main => None,
        }
    }
}

/// The first-parent commits of `range` in `repo`: how many, and the oldest's commit time.
fn ahead(repo: &Path, range: &str) -> Option<Ahead> {
    let text = crate::worktree::git(repo, &["log", "--first-parent", "--format=%ct", range]).ok()?;
    let times: Vec<u64> = text.lines().filter_map(|l| l.trim().parse().ok()).collect();
    let since = *times.last()?;
    Some(Ahead { commits: times.len() as u32, since })
}

/// A project's main as git has it: what origin lacks, and what isn't live.
/// Runs git: off the daemon's loop.
pub fn repo_facts(path: &Path, live: &dyn LiveState) -> RepoFacts {
    let mut facts = RepoFacts { project: crate::board::project_name(path), path: path.to_owned(), unpushed: None, not_live: None };
    let Ok(repo) = crate::worktree::repo_root(path) else { return facts };
    let Ok(main) = crate::worktree::main_branch(&repo) else { return facts };
    let theirs = format!("refs/remotes/origin/{main}");
    if !crate::worktree::has_origin(&repo) || crate::worktree::git(&repo, &["rev-parse", "-q", "--verify", &format!("{theirs}^{{commit}}")]).is_err() {
        return facts;
    }
    facts.unpushed = ahead(&repo, &format!("{theirs}..refs/heads/{main}"));
    if let Some(sha) = live.live_commit(path) {
        facts.not_live = ahead(&repo, &format!("{sha}..{theirs}"));
    }
    facts
}

/// The commit `branch` is at in the repository whose common git folder is
/// `common` — read from its files, no git run: loose ref first, then packed.
pub fn branch_tip(common: &Path, branch: &str) -> Option<String> {
    if let Ok(sha) = std::fs::read_to_string(common.join("refs").join("heads").join(branch)) {
        return Some(sha.trim().to_owned()).filter(|s| !s.is_empty());
    }
    let packed = std::fs::read_to_string(common.join("packed-refs")).ok()?;
    let wanted = format!("refs/heads/{branch}");
    packed.lines().find_map(|line| {
        let (sha, name) = line.split_once(' ')?;
        (name.trim() == wanted).then(|| sha.trim().to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retro::Turn;
    use equipe_protocol::{Ticket, TicketKind};

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn scratch(name: &str) -> PathBuf {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("equipe-health-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ticket(title: &str, status: TicketStatus, owner: &str) -> Ticket {
        Ticket { summary: None,
            id: 0,
            title: title.into(),
            body: "Do it.".into(),
            status,
            kind: TicketKind::Code,
            owner: Some(owner.into()),
            assignee: (status == TicketStatus::InProgress).then(|| owner.to_owned()),
            project: None,
            depends_on: vec![],
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            design_qa: None,
            landing: false,
            plan: None,
            model: None,
            flow: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            label: None,
            remaining: vec![],
            priority: Default::default(),
            rank: 0,
            updated_at: None,
            started_at: None,
            app: None,
            images: Default::default(),
        }
    }

    /// A fixture journal: tickets made `at`, with the given turns and record lines.
    struct Fixture {
        journal: Journal,
        at: u64,
    }

    impl Fixture {
        fn new() -> Self {
            let journal = Journal::open(&scratch("journal").join("squad.db")).unwrap();
            Fixture { journal, at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() }
        }

        fn add(&self, t: Ticket) -> TicketId {
            let id = self.journal.create_ticket(&t).unwrap();
            let mut saved = self.journal.ticket(id).unwrap().unwrap();
            saved.status = t.status;
            saved.assignee = t.assignee.clone();
            saved.depends_on = t.depends_on.clone();
            self.journal.save_ticket(&saved).unwrap();
            self.journal.backdate_ticket(id, self.at);
            id
        }

        fn turn(&self, member: &str, ticket: Option<TicketId>, ended: u64) {
            let t = Turn { member: member.into(), role: member.into(), ticket, began: ended - 60, ended, outcome: "completed".into(), tokens: 1000 };
            self.journal.record_turn(&t).unwrap();
        }

        /// The snapshot `later` seconds after the fixture was made.
        fn snapshot(&self, later: u64, live: Live, repos: Vec<RepoFacts>) -> Snapshot {
            Snapshot::gather(&self.journal, self.journal.tickets().unwrap(), self.at + later, live, &[], repos)
        }
    }

    #[test]
    fn a_ticket_in_progress_with_no_turn_for_two_hours_and_its_member_free_is_stalled() {
        let f = Fixture::new();
        let quiet = f.add(ticket("Wire the export button", TicketStatus::InProgress, "frontend"));
        let busy = f.add(ticket("Cache the report", TicketStatus::InProgress, "backend"));
        f.turn("frontend", Some(quiet), f.at);
        f.turn("backend", Some(busy), f.at);
        // An hour on: nothing yet.
        assert!(stalled(&f.snapshot(HOUR, Live::default(), vec![])).is_empty());
        // Two hours on: the one whose member is free is stalled; the busy member's isn't.
        let live = Live { busy: HashSet::from(["backend".to_owned()]), ..Live::default() };
        let found = stalled(&f.snapshot(2 * HOUR + 60, live, vec![]));
        assert_eq!(found, vec![Finding::Stalled { ticket: quiet, title: "Wire the export button".into(), member: Some("frontend".into()), quiet_secs: 2 * HOUR + 60 }]);
        // Running now: not stalled, however quiet its record.
        let live = Live { running: HashSet::from([quiet]), busy: HashSet::from(["frontend".to_owned()]), any: true };
        assert!(stalled(&f.snapshot(5 * HOUR, live, vec![])).iter().all(|f| f.key() != format!("stalled:{quiet}")));
        let said = found[0].said(crate::i18n::core(crate::i18n::Lang::En));
        assert!(said.contains("Wire the export button") && said.contains("frontend"), "{said}");
    }

    #[test]
    fn an_idle_squad_with_work_to_do_says_what_the_work_waits_on() {
        let f = Fixture::new();
        let sign_in = f.add(ticket("Sign in to the store console", TicketStatus::Waiting, "backend"));
        let review = f.add(ticket("Payment provider switch", TicketStatus::Review, "backend"));
        let chained = f.add(Ticket { depends_on: vec![sign_in], ..ticket("Upload the build", TicketStatus::Todo, "backend") });
        for title in ["Store listing copy", "Screenshots for the listing"] {
            f.add(Ticket { depends_on: vec![chained], ..ticket(title, TicketStatus::Todo, "frontend") });
        }
        f.add(Ticket { depends_on: vec![review], ..ticket("Receipts by email", TicketStatus::Todo, "backend") });
        f.turn("pm", None, f.at);
        // Twenty minutes quiet: not yet.
        assert_eq!(idle(&f.snapshot(20 * 60, Live::default(), vec![])), None);
        // Something running: never idle.
        assert_eq!(idle(&f.snapshot(3 * HOUR, Live { any: true, ..Live::default() }, vec![])), None);
        let found = idle(&f.snapshot(2 * HOUR, Live::default(), vec![])).expect("idle");
        let Finding::Idle { quiet_secs, todo, waits_on, ready } = &found else { panic!("{found:?}") };
        assert_eq!((*quiet_secs, *todo, *ready), (2 * HOUR, 4, 0));
        assert_eq!(waits_on.iter().map(|h| (h.ticket, h.holds, h.status)).collect::<Vec<_>>(), vec![(sign_in, 3, TicketStatus::Waiting), (review, 1, TicketStatus::Review)]);
        let line = found.pm_line();
        assert!(line.contains("Sign in to the store console") && line.contains("waiting on the person") && line.contains("holds 3"), "{line}");
        // Ready work that didn't start is said too.
        f.add(ticket("Fix the footer link", TicketStatus::Todo, "frontend"));
        let Some(Finding::Idle { ready, .. }) = idle(&f.snapshot(2 * HOUR, Live::default(), vec![])) else { panic!() };
        assert_eq!(ready, 1);
    }

    #[test]
    fn main_ahead_of_origin_or_of_whats_live_is_told_after_its_wait() {
        let f = Fixture::new();
        let repo = |unpushed: Option<Ahead>, not_live: Option<Ahead>| RepoFacts { project: "web".into(), path: "/p/web".into(), unpushed, not_live };
        let at = f.at;
        let five_min = repo(Some(Ahead { commits: 2, since: at }), None);
        assert!(detect(&f.snapshot(5 * 60, Live::default(), vec![five_min.clone()])).is_empty());
        let found = detect(&f.snapshot(11 * 60, Live::default(), vec![five_min]));
        assert_eq!(found, vec![Finding::Unpushed { project: "web".into(), commits: 2, secs: 11 * 60 }]);
        let pending = repo(None, Some(Ahead { commits: 3, since: at }));
        assert!(detect(&f.snapshot(20 * 60, Live::default(), vec![pending.clone()])).is_empty(), "under half an hour");
        let found = detect(&f.snapshot(31 * 60, Live::default(), vec![pending]));
        assert_eq!(found, vec![Finding::NotLive { project: "web".into(), commits: 3, secs: 31 * 60 }]);
        assert!(found[0].said(crate::i18n::core(crate::i18n::Lang::En)).contains("web"));
    }

    #[test]
    fn what_waits_on_the_person_past_two_hours_is_told() {
        let f = Fixture::new();
        let card = f.add(ticket("Approve deleting the old bucket", TicketStatus::Waiting, "backend"));
        f.turn("backend", Some(card), f.at);
        let asks = [crate::importance::Ask { kind: WaitKind::Decision, label: "Which plan to buy".into(), tickets: vec![], holding: vec![], since: f.at + HOUR, idle_members: 0 }];
        let snap = |later: u64| Snapshot::gather(&f.journal, f.journal.tickets().unwrap(), f.at + later, Live::default(), &asks, vec![]);
        assert_eq!(waiting(&snap(HOUR)), None);
        let Some(Finding::Waiting { items }) = waiting(&snap(2 * HOUR + 1)) else { panic!("waiting") };
        assert_eq!(items.iter().map(|i| (i.ticket, i.secs)).collect::<Vec<_>>(), vec![(Some(card), 2 * HOUR + 1)], "the question is an hour younger");
        let Some(Finding::Waiting { items }) = waiting(&snap(3 * HOUR + 1)) else { panic!("waiting") };
        assert_eq!(items.len(), 2);
        let said = Finding::Waiting { items }.said(crate::i18n::core(crate::i18n::Lang::En));
        assert!(said.contains("Approve deleting the old bucket") && said.contains("Which plan to buy"), "{said}");
    }

    #[test]
    fn a_ticket_sent_back_three_times_in_a_day_is_going_in_circles() {
        let f = Fixture::new();
        let id = f.add(ticket("Verify the checkout flow", TicketStatus::Todo, "qa"));
        f.journal.ticket_event_at(id, f.at - 30 * HOUR, "pm", "sent back", "older than a day");
        f.journal.ticket_event_at(id, f.at, "qa", "handed back", "can't reproduce");
        f.journal.ticket_event_at(id, f.at + 60, "pm", "sent back", "the login step fails");
        assert!(bounced(&f.snapshot(120, Live::default(), vec![])).is_empty(), "two in the day");
        f.journal.ticket_event_at(id, f.at + 90, "equipe", "design QA: changes", "");
        let found = bounced(&f.snapshot(120, Live::default(), vec![]));
        assert_eq!(found, vec![Finding::Bounced { ticket: id, title: "Verify the checkout flow".into(), times: 3 }]);
        // A day on, the first has aged out.
        assert!(bounced(&f.snapshot(BOUNCE_WINDOW + 30, Live::default(), vec![])).is_empty());
    }

    #[test]
    fn each_finding_is_told_once_until_it_changes() {
        let stalled = Finding::Stalled { ticket: 3, title: "A".into(), member: None, quiet_secs: 7200 };
        let idle = |on: &[TicketId]| Finding::Idle {
            quiet_secs: 1800,
            todo: 4,
            waits_on: on.iter().map(|t| Holder { ticket: *t, title: String::new(), status: TicketStatus::Waiting, holds: 1 }).collect(),
            ready: 0,
        };
        let mut told = Told::default();
        assert_eq!(told.fresh(&[stalled.clone(), idle(&[1])]).len(), 2);
        // The same again, however long it lasts: nothing.
        assert!(told.fresh(&[Finding::Stalled { ticket: 3, title: "A".into(), member: None, quiet_secs: 9000 }, idle(&[1])]).is_empty());
        // A new holder joins: the idle squad is news again; one leaving isn't.
        assert_eq!(told.fresh(&[stalled.clone(), idle(&[1, 2])]), vec![idle(&[1, 2])]);
        assert!(told.fresh(&[stalled.clone(), idle(&[2])]).is_empty());
        // Gone, then back: told again.
        assert!(told.fresh(&[idle(&[2])]).is_empty());
        assert_eq!(told.fresh(&[stalled.clone(), idle(&[2])]), vec![stalled]);
        let kept: Told = serde_json::from_str(&serde_json::to_string(&told).unwrap()).unwrap();
        assert_eq!(kept, told, "kept across a restart");
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    struct LiveAt(Option<String>);

    impl LiveState for LiveAt {
        fn live_commit(&self, _: &Path) -> Option<String> {
            self.0.clone()
        }
    }

    #[test]
    fn git_says_what_main_has_that_origin_and_the_live_site_lack() {
        let origin = scratch("origin");
        git(&origin, &["init", "-q", "--bare", "-b", "main"]);
        let repo = scratch("repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "first"]);
        git(&repo, &["push", "-q", "origin", "main"]);
        let live = git(&repo, &["rev-parse", "HEAD"]);
        // All pushed, all live: nothing to say.
        let facts = repo_facts(&repo, &LiveAt(Some(live.clone())));
        assert_eq!((facts.unpushed, facts.not_live), (None, None));
        for n in 0..2 {
            std::fs::write(repo.join("b.txt"), n.to_string()).unwrap();
            git(&repo, &["add", "-A"]);
            git(&repo, &["commit", "-q", "-m", "more"]);
        }
        let facts = repo_facts(&repo, &LiveAt(Some(live.clone())));
        assert_eq!(facts.unpushed.map(|a| a.commits), Some(2), "two commits origin lacks");
        assert_eq!(facts.not_live, None, "origin's main is what is live");
        git(&repo, &["push", "-q", "origin", "main"]);
        let facts = repo_facts(&repo, &LiveAt(Some(live)));
        assert_eq!((facts.unpushed, facts.not_live.map(|a| a.commits)), (None, Some(2)));
        assert_eq!(repo_facts(&repo, &LiveAt(None)).not_live, None, "no deploy known: nothing about live");
        // The branch's tip, read from git's files.
        let (_, common) = crate::worktree::git_dirs(&repo).unwrap();
        assert_eq!(branch_tip(&common, "main"), Some(git(&repo, &["rev-parse", "HEAD"])));
        git(&repo, &["pack-refs", "--all"]);
        assert_eq!(branch_tip(&common, "main"), Some(git(&repo, &["rev-parse", "HEAD"])), "packed too");
        assert_eq!(branch_tip(&common, "nope"), None);
    }
}
