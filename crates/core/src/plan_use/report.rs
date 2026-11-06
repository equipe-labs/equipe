//! Who used what: each hour of the ledger put down to the person's own
//! sessions outside equipe or to equipe's squads — a project, a ticket, a
//! member — and the report a screen shows of it.
//!
//! An hour is equipe's when its session is one equipe ran (a member's, by
//! the squads' journals), when it ran in equipe's folders (a ticket's
//! worktree `<home>/worktrees/<project>/t<N>-…` names its ticket), or when
//! the engine says equipe ran it; every other is the person's own, by the
//! folder it ran in.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::ledger::{Key, Used};
use super::weights;
use crate::protocol::plan_use::{Beat, Burn, Mostly, Pace, Report, Slice, Span, Spike, Tokens, Who};
use crate::protocol::{Engine, PlanWindow, TicketId};

const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;
const WEEK: i64 = 7 * DAY;

/// Whose an hour of equipe's was, as far as can be told.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Whose {
    pub squad: Option<String>,
    pub member: Option<String>,
    pub ticket: Option<TicketId>,
    /// The project's folder name.
    pub project: Option<String>,
    /// In the squad's own folder.
    pub home: bool,
}

/// Where an hour goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// The person's own, in this folder (as they'd write it: `~/code/site`).
    Outside(String),
    Equipe(Whose),
}

/// A ticket as the report names it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TicketInfo {
    pub title: String,
    pub assignee: Option<String>,
    pub project: Option<String>,
}

/// What the squads' journals say of equipe's sessions.
#[derive(Debug, Clone, Default)]
pub struct Squads {
    /// equipe's home.
    pub home: PathBuf,
    /// The person's home, to write folders from `~`.
    pub user_home: Option<String>,
    /// An engine's session (Claude Code's, Codex's thread) → whose.
    pub sessions: HashMap<String, Whose>,
    /// A project's folder name → its squad.
    pub folders: HashMap<String, String>,
    pub tickets: HashMap<(String, TicketId), TicketInfo>,
    /// A member → its role, by squad.
    pub roles: HashMap<(String, String), String>,
    /// Members' turns as the journals kept them, for the engines without a
    /// transcript on this machine.
    pub turns: Vec<KeptTurn>,
}

/// A member's turn with what it used, as its squad's journal kept it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptTurn {
    pub squad: String,
    pub member: String,
    pub ticket: Option<TicketId>,
    pub ended: i64,
    pub engine: Engine,
    pub model: String,
    pub session: Option<String>,
    pub used: Used,
}

fn folder_name(path: &str) -> Option<String> {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned())
}

/// `t427-6acd…` → 427.
fn ticket_of_worktree(id: &str) -> Option<TicketId> {
    let digits: String = id.strip_prefix('t')?.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

impl Squads {
    /// Read every squad's journal under `home`, read-only. Blocking.
    pub fn load(home: &Path, since: i64) -> Self {
        let mut squads = Squads { home: home.to_owned(), user_home: std::env::var("HOME").ok().filter(|h| !h.is_empty()), ..Default::default() };
        let Ok(dirs) = std::fs::read_dir(home.join("squads")) else { return squads };
        for dir in dirs.flatten() {
            let db = dir.path().join("squad.db");
            if !db.exists() {
                continue;
            }
            let squad = dir.file_name().to_string_lossy().into_owned();
            let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
            let Ok(conn) = rusqlite::Connection::open_with_flags(&db, flags) else { continue };
            let _ = conn.busy_timeout(std::time::Duration::from_secs(2));
            squads.read_squad(&squad, &conn, since);
        }
        squads
    }

    fn read_squad(&mut self, squad: &str, conn: &rusqlite::Connection, since: i64) {
        // Each read on its own: a journal from before a column came has no such column.
        if let Ok(mut stmt) = conn.prepare("SELECT path FROM projects") {
            let paths: Vec<String> = stmt.query_map([], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default();
            for path in paths {
                if let Some(name) = folder_name(&path) {
                    self.folders.entry(name).or_insert_with(|| squad.to_owned());
                }
            }
        }
        if let Ok(mut stmt) = conn.prepare("SELECT engine_session, name, role FROM sessions WHERE name IS NOT NULL") {
            let rows: Vec<(Option<String>, String, Option<String>)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map(|rows| rows.flatten().collect()).unwrap_or_default();
            for (engine_session, name, role) in rows {
                if let Some(role) = role {
                    self.roles.insert((squad.to_owned(), name.clone()), role);
                }
                if let Some(id) = engine_session.filter(|s| !s.is_empty()) {
                    self.sessions.entry(id).or_insert(Whose { squad: Some(squad.to_owned()), member: Some(name), ..Default::default() });
                }
            }
        }
        if let Ok(mut stmt) = conn.prepare("SELECT id, title, assignee, project, engine_session FROM tickets") {
            type Row = (TicketId, String, Option<String>, Option<String>, Option<String>);
            let rows: Vec<Row> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).map(|rows| rows.flatten().collect()).unwrap_or_default();
            for (id, title, assignee, project, engine_session) in rows {
                let project = project.as_deref().and_then(folder_name);
                if let Some(session) = engine_session.filter(|s| !s.is_empty()) {
                    // A ticket's conversation is the ticket's, whoever's desk it began at.
                    self.sessions.insert(session, Whose { squad: Some(squad.to_owned()), member: assignee.clone(), ticket: Some(id), project: project.clone(), home: project.is_none() });
                }
                self.tickets.insert((squad.to_owned(), id), TicketInfo { title, assignee, project });
            }
        }
        if let Ok(mut stmt) = conn.prepare(
            "SELECT member, ticket, ended, engine, model, session, input, output, cache_write, cache_write_long, cache_read, calls
             FROM turns WHERE engine IS NOT NULL AND ended >= ?1",
        ) {
            let n = |r: &rusqlite::Row, i: usize| r.get::<_, Option<i64>>(i).map(|v| v.unwrap_or(0).max(0) as u64);
            let rows: Vec<KeptTurn> = stmt
                .query_map([since], |r| {
                    let engine: String = r.get(3)?;
                    Ok(KeptTurn {
                        squad: squad.to_owned(),
                        member: r.get(0)?,
                        ticket: r.get(1)?,
                        ended: r.get(2)?,
                        engine: if engine == "codex" { Engine::Codex } else { Engine::Claude },
                        model: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        session: r.get(5)?,
                        used: Used { tokens: Tokens { input: n(r, 6)?, output: n(r, 7)?, cache_write: n(r, 8)?, cache_read: n(r, 10)? }, long: n(r, 9)?, calls: n(r, 11)? },
                    })
                })
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default();
            for t in &rows {
                if let Some(session) = t.session.clone().filter(|s| !s.is_empty()) {
                    let project = t.ticket.and_then(|id| self.tickets.get(&(squad.to_owned(), id))).and_then(|i| i.project.clone());
                    self.sessions.entry(session).or_insert(Whose { squad: Some(squad.to_owned()), member: Some(t.member.clone()), ticket: t.ticket, project, home: false });
                }
            }
            self.turns.extend(rows);
        }
    }

    /// A folder as the person writes it: from `~`, a Claude Code worktree as
    /// the project it is of.
    pub fn folder_label(&self, cwd: &str) -> String {
        let cwd = cwd.split("/.claude/worktrees/").next().unwrap_or(cwd);
        match &self.user_home {
            Some(home) if cwd == home => "~".into(),
            Some(home) if cwd.starts_with(&format!("{home}/")) => format!("~{}", &cwd[home.len()..]),
            _ => cwd.to_owned(),
        }
    }

    /// Where one hour of the ledger goes.
    pub fn place(&self, key: &Key) -> Place {
        if let Some(whose) = self.sessions.get(&key.session) {
            let mut whose = whose.clone();
            if whose.project.is_none() && !whose.home {
                whose = self.by_folder(&key.cwd).map(|w| Whose { project: w.project, home: w.home, ..whose.clone() }).unwrap_or(whose);
            }
            return Place::Equipe(whose);
        }
        if let Some(whose) = self.by_folder(&key.cwd) {
            return Place::Equipe(whose);
        }
        if key.origin == "equipe" {
            return Place::Equipe(Whose::default());
        }
        Place::Outside(self.folder_label(&key.cwd))
    }

    /// Whose a folder of equipe's is, by its path.
    fn by_folder(&self, cwd: &str) -> Option<Whose> {
        let home = self.home.to_string_lossy();
        let rest = cwd.strip_prefix(home.as_ref())?;
        if !(rest.is_empty() || rest.starts_with('/')) {
            return None;
        }
        let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
        match parts.as_slice() {
            ["worktrees", folder, id, ..] => {
                let ticket = ticket_of_worktree(id);
                let (squad, project, home) = match folder.strip_suffix(".home") {
                    Some(squad) => (Some(squad.to_owned()), None, true),
                    None => (self.folders.get(*folder).cloned(), Some((*folder).to_owned()), false),
                };
                let member = squad.as_ref().zip(ticket).and_then(|(s, t)| self.tickets.get(&(s.clone(), t))).and_then(|i| i.assignee.clone());
                Some(Whose { squad, member, ticket, project, home })
            }
            ["squads", squad, ..] => Some(Whose { squad: Some((*squad).to_owned()), home: true, ..Default::default() }),
            _ => Some(Whose::default()),
        }
    }
}

/// One hour of use, put down to someone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub hour: i64,
    pub model: String,
    pub place: Place,
    pub used: Used,
}

/// The hours of `engine` from `since`: the ledger's, then the members'
/// turns whose session left no transcript here.
pub fn rows(engine: Engine, hours: Vec<(Key, Used)>, squads: &Squads, since: i64) -> Vec<Row> {
    let seen: HashSet<&str> = hours.iter().map(|(k, _)| k.session.as_str()).collect();
    let mut rows: Vec<Row> = hours.iter().map(|(k, u)| Row { hour: k.hour, model: k.model.clone(), place: squads.place(k), used: *u }).collect();
    for t in squads.turns.iter().filter(|t| t.engine == engine && t.ended >= since) {
        if t.session.as_deref().is_some_and(|s| seen.contains(s)) {
            continue;
        }
        let project = t.ticket.and_then(|id| squads.tickets.get(&(t.squad.clone(), id))).and_then(|i| i.project.clone());
        let whose = Whose { squad: Some(t.squad.clone()), member: Some(t.member.clone()), ticket: t.ticket, home: project.is_none(), project };
        rows.push(Row { hour: t.ended - t.ended.rem_euclid(HOUR), model: t.model.clone(), place: Place::Equipe(whose), used: t.used });
    }
    rows.sort_by_key(|r| r.hour);
    rows
}

// ── adding it up ──

#[derive(Debug, Clone, Copy, Default)]
struct Acc {
    load: u64,
    tokens: Tokens,
    calls: u64,
    /// Load by kind: input, output, cache writes, cache reads.
    parts: [u64; 4],
}

impl Acc {
    fn add(&mut self, row: &Weighed) {
        self.load += row.load;
        self.tokens.add(row.used.tokens);
        self.calls += row.used.calls;
        for (a, b) in self.parts.iter_mut().zip(row.parts) {
            *a += b;
        }
    }
}

/// What a share went mostly on, from its load by kind.
pub fn mostly(parts: [u64; 4]) -> Mostly {
    let [input, output, write, read] = parts;
    let most = parts.into_iter().max().unwrap_or(0);
    if most == read {
        Mostly::Rereading
    } else if most == write {
        Mostly::Loading
    } else if most == output {
        Mostly::Writing
    } else {
        let _ = input;
        Mostly::Input
    }
}

struct Weighed<'a> {
    row: &'a Row,
    used: Used,
    load: u64,
    parts: [u64; 4],
}

/// Who an hour shows as on its own, in a bar's list or a spike: its ticket,
/// else its member, else its project; outside, its folder.
fn headline(place: &Place) -> Who {
    match place {
        Place::Outside(folder) => Who::Outside { folder: Some(folder.clone()) },
        Place::Equipe(w) => {
            if let Some(ticket) = w.ticket {
                Who::Ticket { squad: w.squad.clone(), ticket }
            } else if let (Some(squad), Some(member)) = (&w.squad, &w.member) {
                Who::Member { squad: squad.clone(), member: member.clone() }
            } else {
                project_of(w)
            }
        }
    }
}

fn project_of(w: &Whose) -> Who {
    match (&w.squad, &w.project, w.home) {
        (_, Some(project), false) => Who::Project { squad: w.squad.clone(), project: project.clone() },
        (Some(squad), _, _) => Who::Home { squad: squad.clone() },
        _ => Who::Equipe,
    }
}

/// What a report is made from.
pub struct Input<'a> {
    pub engine: Engine,
    pub span: Span,
    pub now: i64,
    /// The plan's weekly window, as last read.
    pub week: Option<PlanWindow>,
    pub rows: &'a [Row],
    pub squads: &'a Squads,
    pub reading: bool,
}

/// Where the week's window starts, and the window if its reading still holds.
fn week_of(week: Option<PlanWindow>, now: i64) -> (i64, Option<PlanWindow>) {
    match week {
        Some(w) if w.resets_at > now => ((w.resets_at - WEEK).min(now), Some(w)),
        _ => (now - WEEK, None),
    }
}

/// `part` of `whole`, in basis points.
fn bp(part: u64, whole: u64) -> u32 {
    if whole == 0 { 0 } else { (part as u128 * 10_000 / whole as u128).min(10_000) as u32 }
}

/// Of the plan's week: `load`'s share of the week's load, times how much of
/// it is used.
fn week_bp(load: u64, week_load: u64, week: Option<PlanWindow>) -> Option<u32> {
    let w = week?;
    (week_load > 0).then(|| (load as u128 * u128::from(w.used_percent) * 100 / week_load as u128) as u32)
}

/// How long the week lasts at the pace of late: the last day's burn, as a
/// share of the week's load times its used percent; the week's own average
/// when nothing is known of the last day.
pub fn pace(used_percent: u8, week_start: i64, resets_at: i64, now: i64, recent: Option<(u64, u64, i64)>) -> Option<Pace> {
    let used_bp = f64::from(used_percent) * 100.0;
    let per_sec = match recent {
        Some((recent_load, week_load, secs)) if recent_load > 0 && week_load > 0 && secs > 0 => recent_load as f64 / week_load as f64 * used_bp / secs as f64,
        _ if now > week_start && used_percent > 0 => used_bp / (now - week_start) as f64,
        _ => return None,
    };
    if per_sec <= 0.0 {
        return None;
    }
    let left = (10_000.0 - used_bp).max(0.0);
    let runs_out_at = now + (left / per_sec).round() as i64;
    Some(Pace { runs_out_at, before_reset: runs_out_at < resets_at, per_day_bp: (per_sec * DAY as f64).round() as u32 })
}

/// A part of the plan worth a line, from what it added up to.
fn slice(who: Who, label: Option<String>, acc: &Acc, span_load: u64, week_load: u64, week: Option<PlanWindow>) -> Slice {
    Slice {
        who,
        label,
        load: acc.load,
        share_bp: bp(acc.load, span_load),
        week_bp: week_bp(acc.load, week_load, week),
        tokens: acc.tokens,
        calls: acc.calls,
        mostly: mostly(acc.parts),
        children: Vec::new(),
    }
}

fn sorted(map: HashMap<Who, Acc>, label: impl Fn(&Who) -> Option<String>, span_load: u64, week_load: u64, week: Option<PlanWindow>, at_most: usize) -> Vec<Slice> {
    let mut all: Vec<(Who, Acc)> = map.into_iter().filter(|(_, a)| a.load > 0).collect();
    all.sort_by(|a, b| b.1.load.cmp(&a.1.load).then_with(|| a.0.cmp(&b.0)));
    all.into_iter().take(at_most).map(|(who, acc)| slice(who.clone(), label(&who), &acc, span_load, week_load, week)).collect()
}

/// The label a part shows: a ticket's title, a member's role.
pub fn label_of(squads: &Squads, who: &Who) -> Option<String> {
    match who {
        Who::Ticket { squad: Some(squad), ticket } => squads.tickets.get(&(squad.clone(), *ticket)).map(|t| t.title.clone()),
        Who::Member { squad, member } => squads.roles.get(&(squad.clone(), member.clone())).cloned(),
        _ => None,
    }
}

/// The hours a spike spans.
pub const SPIKE_HOURS: i64 = 3;
/// A spike: a part that took at least this much of the week (basis points)
/// in [`SPIKE_HOURS`] — or, with the plan unread, of what the look saw.
const SPIKE_WEEK_BP: u32 = 150;
const SPIKE_SHARE_BP: u32 = 1_000;

pub fn build(input: &Input) -> Report {
    let Input { engine, span, now, week, rows, squads, reading } = *input;
    let (week_start, week) = week_of(week, now);
    let from = match span {
        Span::Week => week_start,
        Span::Day => now - DAY,
    };
    let weighed: Vec<Weighed> = rows
        .iter()
        .filter(|r| r.hour + HOUR > week_start.min(from) && r.hour <= now)
        .map(|row| Weighed { row, used: row.used, load: weights::load(engine, &row.model, row.used.tokens, row.used.long), parts: weights::parts(engine, &row.model, row.used.tokens, row.used.long) })
        .collect();
    let week_load: u64 = weighed.iter().filter(|w| w.row.hour + HOUR > week_start).map(|w| w.load).sum();
    let in_span: Vec<&Weighed> = weighed.iter().filter(|w| w.row.hour + HOUR > from).collect();
    let mut all = Acc::default();
    let mut outside = Acc::default();
    let mut folders: HashMap<Who, Acc> = HashMap::new();
    let mut projects: HashMap<Who, Acc> = HashMap::new();
    let mut tickets: HashMap<Who, Acc> = HashMap::new();
    let mut members: HashMap<Who, Acc> = HashMap::new();
    let mut hours: BTreeMap<i64, (Acc, u64, HashMap<Who, Acc>)> = BTreeMap::new();
    for w in &in_span {
        all.add(w);
        let hour = hours.entry(w.row.hour).or_default();
        hour.0.add(w);
        hour.2.entry(headline(&w.row.place)).or_default().add(w);
        match &w.row.place {
            Place::Outside(folder) => {
                outside.add(w);
                hour.1 += w.load;
                folders.entry(Who::Outside { folder: Some(folder.clone()) }).or_default().add(w);
            }
            Place::Equipe(whose) => {
                projects.entry(project_of(whose)).or_default().add(w);
                if let Some(ticket) = whose.ticket {
                    tickets.entry(Who::Ticket { squad: whose.squad.clone(), ticket }).or_default().add(w);
                }
                if let (Some(squad), Some(member)) = (&whose.squad, &whose.member) {
                    members.entry(Who::Member { squad: squad.clone(), member: member.clone() }).or_default().add(w);
                }
            }
        }
    }
    let span_load = all.load;
    let label = |who: &Who| label_of(squads, who);
    let outside_slice = (outside.load > 0).then(|| {
        let mut s = slice(Who::Outside { folder: None }, None, &outside, span_load, week_load, week);
        s.children = sorted(folders, |_| None, span_load, week_load, week, 12);
        s
    });
    // Every hour of the look, burned or not, so a quiet hour reads as one.
    let first = from - from.rem_euclid(HOUR);
    let beats: Vec<Beat> = (0..)
        .map(|i| first + i * HOUR)
        .take_while(|h| *h <= now)
        .map(|at| match hours.get(&at) {
            Some((acc, out, parts)) => Beat { at, load: acc.load, outside: *out, calls: acc.calls, parts: sorted(parts.clone(), label, acc.load, week_load, week, 5) },
            None => Beat { at, load: 0, outside: 0, calls: 0, parts: Vec::new() },
        })
        .collect();
    let spikes = spikes(&in_span, squads, span_load, week_load, week);
    let recent_secs = (now - week_start).clamp(0, DAY);
    let recent_load: u64 = weighed.iter().filter(|w| w.row.hour + HOUR > now - recent_secs).map(|w| w.load).sum();
    let pace = week.and_then(|w| pace(w.used_percent, week_start, w.resets_at, now, Some((recent_load, week_load, recent_secs.max(HOUR)))));
    Report {
        engine,
        span,
        from,
        to: now,
        week,
        pace,
        load: span_load,
        week_load,
        tokens: all.tokens,
        calls: all.calls,
        outside: outside_slice,
        projects: sorted(projects, label, span_load, week_load, week, 40),
        tickets: sorted(tickets, label, span_load, week_load, week, 40),
        members: sorted(members, label, span_load, week_load, week, 40),
        beats,
        spikes,
        reading,
    }
}

/// Where one part burned fastest: for each, its heaviest [`SPIKE_HOURS`]
/// in a row, if heavy enough; the five heaviest of those.
fn spikes(rows: &[&Weighed], squads: &Squads, span_load: u64, week_load: u64, week: Option<PlanWindow>) -> Vec<Spike> {
    let mut by_who: HashMap<Who, BTreeMap<i64, Acc>> = HashMap::new();
    for w in rows {
        by_who.entry(headline(&w.row.place)).or_default().entry(w.row.hour).or_default().add(w);
    }
    let mut found: Vec<Spike> = Vec::new();
    for (who, hours) in by_who {
        let mut best: Option<(i64, Acc)> = None;
        for &start in hours.keys() {
            let mut acc = Acc::default();
            for (_, a) in hours.range(start..start + SPIKE_HOURS * HOUR) {
                acc.load += a.load;
                acc.tokens.add(a.tokens);
                acc.calls += a.calls;
                for (x, y) in acc.parts.iter_mut().zip(a.parts) {
                    *x += y;
                }
            }
            if best.as_ref().is_none_or(|(_, b)| acc.load > b.load) {
                best = Some((start, acc));
            }
        }
        let Some((start, acc)) = best else { continue };
        let heavy = match week_bp(acc.load, week_load, week) {
            Some(bp) => bp >= SPIKE_WEEK_BP,
            None => bp(acc.load, span_load) >= SPIKE_SHARE_BP,
        };
        if !heavy {
            continue;
        }
        // From its first hour of use to the end of its last, in the window.
        let last = hours.range(start..start + SPIKE_HOURS * HOUR).next_back().map_or(start, |(h, _)| *h);
        found.push(Spike { from: start, to: last + HOUR, slice: slice(who.clone(), label_of(squads, &who), &acc, span_load, week_load, week) });
    }
    found.sort_by(|a, b| b.slice.load.cmp(&a.slice.load).then_with(|| a.from.cmp(&b.from)));
    found.truncate(5);
    found
}

// ── burns going on now ──

/// A burn: one ticket, or the use outside equipe, took at least this much
/// of the week (basis points) in the last [`BURN_HOURS`].
pub const BURN_WEEK_BP: u32 = 500;
pub const BURN_HOURS: i64 = 3;

/// The burns going on at `now`, on `engine`'s plan. None while the plan's
/// week isn't read: a share of nothing known says nothing.
pub fn burns(engine: Engine, rows: &[Row], squads: &Squads, week: Option<PlanWindow>, now: i64) -> Vec<Burn> {
    let (week_start, week) = week_of(week, now);
    if week.is_none() {
        return Vec::new();
    }
    let since = now - BURN_HOURS * HOUR;
    let mut week_load = 0;
    let mut recent: HashMap<Who, Acc> = HashMap::new();
    for row in rows.iter().filter(|r| r.hour + HOUR > week_start && r.hour <= now) {
        let w = Weighed { row, used: row.used, load: weights::load(engine, &row.model, row.used.tokens, row.used.long), parts: weights::parts(engine, &row.model, row.used.tokens, row.used.long) };
        week_load += w.load;
        if row.hour + HOUR <= since {
            continue;
        }
        let who = match &row.place {
            Place::Outside(_) => Who::Outside { folder: None },
            Place::Equipe(Whose { ticket: Some(ticket), squad, .. }) => Who::Ticket { squad: squad.clone(), ticket: *ticket },
            Place::Equipe(_) => continue,
        };
        recent.entry(who).or_default().add(&w);
    }
    let mut burns: Vec<Burn> = recent
        .into_iter()
        .filter_map(|(who, acc)| {
            let share = week_bp(acc.load, week_load, week)?;
            (share >= BURN_WEEK_BP).then(|| Burn {
                id: format!("{}:{}:{}", engine_word(engine), who_key(&who), now - now.rem_euclid(HOUR)),
                engine,
                label: label_of(squads, &who),
                who,
                week_bp: share,
                hours: BURN_HOURS as u32,
                at: now,
                calls: acc.calls,
                mostly: mostly(acc.parts),
            })
        })
        .collect();
    burns.sort_by_key(|b| std::cmp::Reverse(b.week_bp));
    burns
}

pub fn engine_word(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "claude",
        Engine::Codex => "codex",
    }
}

/// A part, as a burn told already is known again.
pub fn who_key(who: &Who) -> String {
    match who {
        Who::Outside { .. } => "outside".into(),
        Who::Ticket { squad, ticket } => format!("{}/T-{ticket}", squad.as_deref().unwrap_or("")),
        Who::Member { squad, member } => format!("{squad}/{member}"),
        Who::Project { squad, project } => format!("{}/{project}", squad.as_deref().unwrap_or("")),
        Who::Home { squad } => format!("{squad}/home"),
        Who::Equipe => "equipe".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_793_635_200; // on the hour
    const HOME: &str = "/u/.equipe";

    fn squads() -> Squads {
        let mut s = Squads { home: HOME.into(), user_home: Some("/u".into()), ..Default::default() };
        s.folders.insert("web".into(), "acme".into());
        s.tickets.insert(("acme".into(), 427), TicketInfo { title: "Checkout".into(), assignee: Some("frontend".into()), project: Some("web".into()) });
        s.roles.insert(("acme".into(), "frontend".into()), "frontend".into());
        s.roles.insert(("acme".into(), "pm".into()), "pm".into());
        s.sessions.insert("pm-desk".into(), Whose { squad: Some("acme".into()), member: Some("pm".into()), ..Default::default() });
        s
    }

    fn key(session: &str, cwd: &str, origin: &str) -> Key {
        Key { hour: NOW, engine: "claude".into(), model: "claude-opus-5-5".into(), session: session.into(), cwd: cwd.into(), origin: origin.into() }
    }

    #[test]
    fn a_worktree_names_its_project_and_ticket_and_the_rest_is_the_persons_own() {
        let s = squads();
        let worktree = s.place(&key("x", "/u/.equipe/worktrees/web/t427-6acd1234", ""));
        assert_eq!(worktree, Place::Equipe(Whose { squad: Some("acme".into()), member: Some("frontend".into()), ticket: Some(427), project: Some("web".into()), home: false }));
        let home = s.place(&key("y", "/u/.equipe/worktrees/acme.home/t3-6acf", ""));
        assert_eq!(home, Place::Equipe(Whose { squad: Some("acme".into()), ticket: Some(3), home: true, ..Default::default() }));
        assert_eq!(s.place(&key("z", "/u/.equipe/squads/acme/home", "")), Place::Equipe(Whose { squad: Some("acme".into()), home: true, ..Default::default() }));
        // A member's desk conversation, wherever it ran.
        assert!(matches!(s.place(&key("pm-desk", "/u/code/site", "")), Place::Equipe(Whose { member: Some(m), .. }) if m == "pm"));
        // Codex says equipe ran it.
        assert_eq!(s.place(&key("w", "/u/code/site", "equipe")), Place::Equipe(Whose::default()));
        assert_eq!(s.place(&key("me", "/u/Abel", "")), Place::Outside("~/Abel".into()));
        assert_eq!(s.place(&key("me", "/u/Abel/equipe/.claude/worktrees/agent-1", "")), Place::Outside("~/Abel/equipe".into()), "a Claude Code worktree is its project's");
        assert_eq!(s.place(&key("me", "/u/.equipe-other/x", "")), Place::Outside("~/.equipe-other/x".into()), "only equipe's own folder");
    }

    fn row(hours_ago: i64, model: &str, place: Place, tokens: Tokens, calls: u64) -> Row {
        Row { hour: NOW - hours_ago * HOUR, model: model.into(), place, used: Used { tokens, long: 0, calls } }
    }

    fn ticket(n: TicketId) -> Place {
        Place::Equipe(Whose { squad: Some("acme".into()), member: Some("frontend".into()), ticket: Some(n), project: Some("web".into()), home: false })
    }

    fn reread(n: u64) -> Tokens {
        Tokens { input: 0, output: n / 100, cache_write: n / 50, cache_read: n }
    }

    #[test]
    fn shares_are_of_load_and_of_the_week_and_the_heavy_come_first() {
        let s = squads();
        let rows = vec![
            row(30, "claude-opus-5-5", Place::Outside("~/Abel".into()), reread(50_000_000), 400),
            row(2, "claude-opus-5-5", ticket(427), reread(20_000_000), 300),
            row(1, "claude-opus-5-5", ticket(427), reread(20_000_000), 300),
            // Many tokens, but on Haiku: little load.
            row(1, "claude-haiku-4-5", Place::Equipe(Whose { squad: Some("acme".into()), member: Some("pm".into()), home: true, ..Default::default() }), reread(80_000_000), 50),
        ];
        let week = PlanWindow { used_percent: 40, resets_at: NOW + 3 * DAY };
        let r = build(&Input { engine: Engine::Claude, span: Span::Week, now: NOW, week: Some(week), rows: &rows, squads: &s, reading: false });
        assert_eq!(r.from, NOW + 3 * DAY - WEEK);
        let outside = r.outside.as_ref().unwrap();
        assert_eq!(outside.children[0].who, Who::Outside { folder: Some("~/Abel".into()) });
        assert_eq!(r.tickets[0].who, Who::Ticket { squad: Some("acme".into()), ticket: 427 });
        assert_eq!(r.tickets[0].label.as_deref(), Some("Checkout"));
        assert_eq!(r.tickets[0].calls, 600);
        assert_eq!(r.members[0].label.as_deref(), Some("frontend"), "a member shows with its role");
        assert_eq!(r.members.last().unwrap().who, Who::Member { squad: "acme".into(), member: "pm".into() }, "the most tokens, the least load");
        // The week's shares add up to what of the week is used.
        let parts = outside.week_bp.unwrap() + r.projects.iter().map(|p| p.week_bp.unwrap()).sum::<u32>();
        assert!((3_990..=4_000).contains(&parts), "{parts}");
        assert!(r.outside.as_ref().unwrap().share_bp > 4_000);
        assert_eq!(r.tickets[0].mostly, Mostly::Rereading);
        assert_eq!(r.beats.last().unwrap().at, NOW);
        assert_eq!(r.beats.len() as i64, (NOW - r.from) / HOUR + 1, "every hour, a quiet one too");
        let beat = r.beats.iter().find(|b| b.at == NOW - 2 * HOUR).unwrap();
        assert_eq!(beat.parts[0].who, Who::Ticket { squad: Some("acme".into()), ticket: 427 }, "what ran then");
    }

    #[test]
    fn a_spike_is_a_parts_heaviest_three_hours_said_in_its_share_of_the_week() {
        let s = squads();
        let mut rows: Vec<Row> = (0..40).map(|h| row(h + 5, "claude-opus-5-5", Place::Outside("~/Abel".into()), reread(1_000_000), 10)).collect();
        rows.extend([row(3, "claude-opus-5-5", ticket(427), reread(30_000_000), 500), row(2, "claude-opus-5-5", ticket(427), reread(30_000_000), 704)]);
        let week = PlanWindow { used_percent: 30, resets_at: NOW + 3 * DAY };
        let r = build(&Input { engine: Engine::Claude, span: Span::Week, now: NOW, week: Some(week), rows: &rows, squads: &s, reading: false });
        let spike = &r.spikes[0];
        assert_eq!(spike.slice.who, Who::Ticket { squad: Some("acme".into()), ticket: 427 });
        assert_eq!((spike.from, spike.to), (NOW - 3 * HOUR, NOW - HOUR));
        assert_eq!(spike.slice.calls, 1204);
        assert!(spike.slice.week_bp.unwrap() > 1_500, "{:?}", spike.slice.week_bp);
        assert!(r.spikes.iter().all(|s| s.slice.week_bp.unwrap() >= SPIKE_WEEK_BP));
    }

    #[test]
    fn the_pace_of_the_last_day_says_when_the_week_runs_out() {
        let start = NOW - 4 * DAY;
        let reset = start + WEEK;
        // 60% used in four days, but the last day took half the week's load: 30% a day.
        let p = pace(60, start, reset, NOW, Some((500, 1000, DAY))).unwrap();
        assert_eq!(p.per_day_bp, 3_000);
        assert_eq!(p.runs_out_at, NOW + DAY * 4 / 3);
        assert!(p.before_reset);
        // A quiet last day: it lasts past the reset.
        let calm = pace(60, start, reset, NOW, Some((10, 1000, DAY))).unwrap();
        assert!(!calm.before_reset);
        // Nothing known of the last day: the week's average.
        assert_eq!(pace(60, start, reset, NOW, None).unwrap().per_day_bp, 1_500);
        assert_eq!(pace(0, start, reset, NOW, None), None, "nothing used, no pace");
        assert_eq!(pace(100, start, reset, NOW, None).unwrap().runs_out_at, NOW, "used up: now");
    }

    #[test]
    fn a_burn_is_told_when_one_ticket_or_the_outside_takes_much_of_the_week_fast() {
        let s = squads();
        let mut rows: Vec<Row> = (10..60).map(|h| row(h, "claude-opus-5-5", ticket(1), reread(4_000_000), 10)).collect();
        rows.push(row(1, "claude-opus-5-5", ticket(427), reread(30_000_000), 900));
        rows.push(row(1, "claude-opus-5-5", Place::Outside("~/Abel".into()), reread(1_000_000), 5));
        let week = PlanWindow { used_percent: 50, resets_at: NOW + 2 * DAY };
        let burns = burns(Engine::Claude, &rows, &s, Some(week), NOW);
        assert_eq!(burns.len(), 1, "{burns:?}");
        assert_eq!(burns[0].who, Who::Ticket { squad: Some("acme".into()), ticket: 427 });
        assert_eq!(burns[0].label.as_deref(), Some("Checkout"));
        assert!(burns[0].week_bp >= BURN_WEEK_BP);
        assert_eq!(burns[0].id, format!("claude:acme/T-427:{NOW}"));
        assert!(super::burns(Engine::Claude, &rows, &s, None, NOW).is_empty(), "the plan unread: nothing to measure by");
        let stale = PlanWindow { used_percent: 50, resets_at: NOW - 1 };
        assert!(super::burns(Engine::Claude, &rows, &s, Some(stale), NOW).is_empty(), "a reading from before the reset says nothing of this week");
    }

    #[test]
    fn turns_kept_by_the_journal_count_only_where_no_transcript_was_read() {
        let mut s = squads();
        let used = Used { tokens: Tokens { input: 100, output: 10, ..Default::default() }, long: 0, calls: 1 };
        let turn = |session: &str| KeptTurn { squad: "acme".into(), member: "backend".into(), ticket: Some(9), ended: NOW, engine: Engine::Codex, model: "gpt-5".into(), session: Some(session.into()), used };
        s.turns = vec![turn("read-here"), turn("api-only")];
        let hours = vec![(Key { engine: "codex".into(), model: "gpt-5".into(), ..key("read-here", "/u/.equipe/worktrees/web/t9-a", "") }, used)];
        let rows = rows(Engine::Codex, hours, &s, NOW - DAY);
        assert_eq!(rows.len(), 2, "the transcript's hour, and the turn that left none");
        assert!(rows.iter().all(|r| matches!(&r.place, Place::Equipe(w) if w.ticket == Some(9))));
    }
}
