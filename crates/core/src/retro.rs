//! The weekly retro: once a week the PM brings the person, in one message,
//! what shipped, what got stuck and why, where each member's time went, and
//! at most three lessons and three changes — the changes as one card the
//! person answers in a click each (`ask_decision`). The PM writes it from
//! evidence, not memory: equipe gathers the week's facts from the squad's
//! journal ([`gather`]) and hands them over with the routine ([`message`]).
//! A routine like any (`crate::routines`), added on once when a PM joins,
//! the person's to move or turn off. What the person confirms becomes a
//! lesson in the squad's memory ([`confirmed`], [`entry`]); the rest is let
//! go. Plain functions; the runtime keeps and sends (`runtime/retro.rs`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use equipe_protocol::decide::{Answer, QuestionSet, Retro};
use equipe_protocol::routines::{Routine, Schedule};
use equipe_protocol::{Ticket, TicketEntry, TicketId, TicketStatus};
use serde_json::Value;

use crate::i18n::{Lang, core};
use crate::routines::Clock;

/// The role the retro is the work of.
pub const ROLE: &str = crate::roles::PM;

/// Fridays at 17:00, the daemon machine's local time: the week's work is in,
/// the person still has the week in mind, and the changes are ready by Monday.
pub const WHEN: Schedule = Schedule::Weekly { weekday: 5, hour: 17, minute: 0 };

/// A week, the span the retro looks back over.
pub const WEEK: u64 = 7 * 24 * 60 * 60;

/// Lessons, and changes, at most.
pub const MOST: usize = 3;

/// How long after the retro is handed over a decision the PM puts is taken
/// for the retro's changes.
pub const ASK_WITHIN: u64 = 6 * 60 * 60;

/// A turn this long is told as stuck: half an hour without handing back.
pub const LONG_TURN: u64 = 30 * 60;

/// Past the time a member said it would be done by more than this, a ticket
/// is late.
pub const LATE_AFTER: u64 = 5 * 60;

/// A ticket's record of when its member said it would be done: the unix
/// seconds, as text. Kept for the retro; screens leave it out.
pub const ESTIMATED: &str = "estimated";

/// The facts' first line, which screens read the retro by.
pub const FACTS_HEAD: &str = "[Retro facts · ";
/// The heads of what shipped and what went wrong, each before its count.
const SHIPPED_HEAD: &str = "Shipped (";
const TROUBLES_HEAD: &str = "Stuck, failed, sent back, late (";

/// The squad memory's fact the confirmed lessons go into, and its line in the index.
pub const LESSONS: &str = "lessons";
pub const LESSONS_DESCRIPTION: &str = "What the squad learned in its weekly retros, as the person confirmed it, newest first — read it before planning the week's work.";

/// Entries in the lessons kept, newest first.
const KEEP: usize = 52;

/// The routine, on, in the person's language — theirs to change or turn off.
pub fn routine(lang: Lang, id: u32) -> Routine {
    Routine {
        id,
        title: core(lang).get("routine.retro.title").to_owned(),
        prompt: core(lang).get("routine.retro.prompt").to_owned(),
        when: WHEN,
        on: true,
        last_run: None,
        next_run: None,
    }
}

/// Where the week starts: the last retro, when it ran within two weeks,
/// else a week before `now`.
pub fn since(last_run: Option<u64>, now: u64) -> u64 {
    last_run.filter(|at| *at < now && now - at <= 2 * WEEK).unwrap_or(now.saturating_sub(WEEK))
}

// ── what the journal keeps ──

/// A member's turn, as the journal keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub member: String,
    pub role: String,
    /// The ticket it ran for; `None`: at its desk.
    pub ticket: Option<TicketId>,
    pub began: u64,
    pub ended: u64,
    /// `completed`, `incomplete`, `interrupted` or `failed`.
    pub outcome: String,
    /// Everything it processed, as its engine said.
    pub tokens: u64,
}

impl Turn {
    pub fn secs(&self) -> u64 {
        self.ended.saturating_sub(self.began)
    }
}

/// Something a member waited on the person for: a permission, a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wait {
    /// `permission` or `decision`.
    pub kind: String,
    pub member: String,
    pub ticket: Option<TicketId>,
    /// What it was: the action, the first question.
    pub what: String,
    pub asked: u64,
    /// `None`: still waiting.
    pub answered: Option<u64>,
    /// `allowed`, `declined`, `answered`, `closed`…
    pub outcome: String,
}

impl Wait {
    /// How long it waited, up to `now` for one still waiting.
    pub fn secs(&self, now: u64) -> u64 {
        self.answered.unwrap_or(now).saturating_sub(self.asked)
    }
}

/// A brief in a project (`docs/briefs/*.md`): what was expected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brief {
    pub project: String,
    /// Its path inside the project.
    pub file: String,
    pub title: String,
}

/// Where a project keeps its briefs.
pub const BRIEFS: &str = "docs/briefs";
/// Briefs listed, at most.
const BRIEFS_SHOWN: usize = 12;

/// The briefs of `projects`, by project then file.
pub fn briefs(projects: &[PathBuf]) -> Vec<Brief> {
    let mut out = Vec::new();
    for project in projects {
        let Ok(entries) = std::fs::read_dir(project.join(BRIEFS)) else { continue };
        let mut files: Vec<String> = entries.flatten().filter_map(|e| e.file_name().to_str().map(str::to_owned)).filter(|f| f.ends_with(".md") && !f.starts_with('.')).collect();
        files.sort();
        for file in files {
            let text = std::fs::read_to_string(project.join(BRIEFS).join(&file)).unwrap_or_default();
            let title = text.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty() && *l != "---").unwrap_or(file.trim_end_matches(".md")).to_owned();
            out.push(Brief { project: crate::board::project_name(project), file: format!("{BRIEFS}/{file}"), title: clip(&title, 80) });
        }
    }
    out.truncate(BRIEFS_SHOWN);
    out
}

// ── the week's facts ──

/// What the week is gathered from.
#[derive(Debug, Clone, Default)]
pub struct Week<'a> {
    pub from: u64,
    pub to: u64,
    /// The board as it is now.
    pub tickets: &'a [Ticket],
    /// Ticket records since `from`, oldest first.
    pub events: &'a [(TicketId, TicketEntry)],
    pub turns: &'a [Turn],
    pub waits: &'a [Wait],
    pub briefs: Vec<Brief>,
}

/// A ticket that shipped this week.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shipped {
    pub id: TicketId,
    pub title: String,
    pub member: Option<String>,
    pub project: Option<String>,
    pub branch: Option<String>,
    /// Landed on main; `false`: accepted, still on its branch.
    pub on_main: bool,
    pub at: u64,
}

/// What went wrong with work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Snag {
    /// A turn failed, a merge failed.
    Failed,
    /// Sent back: by the person, by design QA, by equipe.
    SentBack,
    /// A turn ran long, or the person stopped one; it couldn't start.
    Stuck,
    /// Handed in (or still open) past when its member said.
    Late,
}

impl Snag {
    fn word(self) -> &'static str {
        match self {
            Snag::Failed => "failed",
            Snag::SentBack => "sent back",
            Snag::Stuck => "stuck",
            Snag::Late => "late",
        }
    }
}

/// One thing that got stuck or failed, and why as far as the journal says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trouble {
    pub snag: Snag,
    pub ticket: Option<TicketId>,
    /// The ticket's title, when it is on the board.
    pub title: Option<String>,
    pub member: Option<String>,
    pub why: String,
    pub at: u64,
}

/// A member's week: its turns and their time and usage, ticket by ticket.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Time {
    pub member: String,
    pub role: String,
    pub turns: usize,
    pub secs: u64,
    pub tokens: u64,
    /// By ticket (`None`: at its desk): turns and seconds, the most time first.
    pub tickets: Vec<(Option<TicketId>, usize, u64)>,
}

/// Where a member's week rubbed: counts of what went wrong, and its waits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Friction {
    pub member: String,
    pub role: String,
    pub snags: BTreeMap<Snag, usize>,
    pub waits: usize,
    pub waited: u64,
}

/// The week, gathered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub from: u64,
    pub to: u64,
    pub shipped: Vec<Shipped>,
    pub troubles: Vec<Trouble>,
    pub time: Vec<Time>,
    pub waits: Vec<Wait>,
    pub friction: Vec<Friction>,
    pub briefs: Vec<Brief>,
}

/// The week's facts from what the journal keeps: nothing asked of anyone.
pub fn gather(week: &Week<'_>) -> Facts {
    let inside = |at: u64| at >= week.from && at < week.to;
    let ticket = |id: TicketId| week.tickets.iter().find(|t| t.id == id);
    let worker = |id: TicketId| ticket(id).and_then(|t| t.assignee.clone().or(t.owner.clone()));
    let events: Vec<&(TicketId, TicketEntry)> = week.events.iter().filter(|(_, e)| inside(e.at)).collect();

    // Shipped: accepted or merged this week, and done now.
    let mut shipped: Vec<Shipped> = Vec::new();
    for (id, e) in &events {
        if !matches!(e.kind.as_str(), "accepted" | "accepted by the PM" | "merged") {
            continue;
        }
        let Some(t) = ticket(*id).filter(|t| t.status == TicketStatus::Done) else { continue };
        let merged = e.kind == "merged";
        match shipped.iter_mut().find(|s| s.id == *id) {
            Some(s) => {
                s.on_main |= merged;
                s.at = s.at.max(e.at);
            }
            None => shipped.push(Shipped {
                id: *id,
                title: t.title.clone(),
                member: worker(*id),
                project: t.project.as_deref().map(crate::board::project_name),
                branch: t.branch.clone(),
                on_main: merged || t.merged == Some(true),
                at: e.at,
            }),
        }
    }
    shipped.sort_by_key(|s| (s.at, s.id));

    // What went wrong: the ticket records first.
    let mut troubles: Vec<Trouble> = Vec::new();
    for (id, e) in &events {
        let said = one_line(&e.text);
        let (snag, why) = match e.kind.as_str() {
            "reopened" => (Snag::SentBack, format!("sent back by you{}", quoted(said.trim_start_matches("Reopened").trim_start_matches(':').trim()))),
            "design QA: changes" => (Snag::SentBack, format!("sent back by design QA ({}){}", e.by, quoted(&said))),
            "sent back" => (Snag::SentBack, format!("sent back by equipe{}", quoted(&said))),
            "not merged" => (Snag::Failed, format!("merging failed{}", quoted(&said))),
            "blocked" if e.by == "equipe" => (Snag::Stuck, format!("couldn't go on{}", quoted(&said))),
            _ => continue,
        };
        troubles.push(Trouble { snag, ticket: Some(*id), title: ticket(*id).map(|t| t.title.clone()), member: worker(*id), why, at: e.at });
    }
    // Turns that failed, were stopped, or ran long — once per member, ticket and
    // kind: how many, the longest, the last.
    type Run = (String, Option<TicketId>, &'static str);
    let turns: Vec<&Turn> = week.turns.iter().filter(|t| inside(t.ended)).collect();
    let mut runs: BTreeMap<Run, (usize, u64, u64)> = BTreeMap::new();
    for t in &turns {
        let kind = match t.outcome.as_str() {
            "failed" => "failed",
            "interrupted" => "stopped",
            _ if t.secs() >= LONG_TURN => "long",
            _ => continue,
        };
        let run = runs.entry((t.member.clone(), t.ticket, kind)).or_default();
        run.0 += 1;
        run.1 = run.1.max(t.secs());
        run.2 = run.2.max(t.ended);
    }
    for ((member, ticket, kind), (n, longest, at)) in runs {
        let times = if n > 1 { format!(" ({n}×)") } else { String::new() };
        let (snag, why) = match kind {
            "failed" => (Snag::Failed, format!("a turn failed{times}")),
            "stopped" => (Snag::Stuck, format!("you stopped a turn{times}")),
            _ => (Snag::Stuck, format!("a turn ran {}{times}", span(longest))),
        };
        troubles.push(Trouble { snag, ticket, title: ticket.and_then(|id| week.tickets.iter().find(|t| t.id == id)).map(|t| t.title.clone()), member: Some(member), why, at });
    }
    // Late: handed in past the time its member gave, or still open past it.
    let mut estimated: BTreeMap<TicketId, u64> = BTreeMap::new();
    let mut late: BTreeMap<TicketId, (u64, u64, bool)> = BTreeMap::new();
    for (id, e) in week.events.iter().filter(|(_, e)| e.at < week.to) {
        match e.kind.as_str() {
            ESTIMATED => {
                if let Ok(at) = e.text.trim().parse::<u64>() {
                    estimated.insert(*id, at);
                }
            }
            "handed in" | "waiting" if inside(e.at) => {
                if let Some(eta) = estimated.remove(id).filter(|eta| e.at > eta + LATE_AFTER) {
                    late.entry(*id).or_insert((e.at - eta, e.at, true));
                }
            }
            _ => {}
        }
    }
    for (id, eta) in estimated {
        if ticket(id).is_some_and(|t| t.status == TicketStatus::InProgress) && week.to > eta + LATE_AFTER && eta >= week.from {
            late.entry(id).or_insert((week.to - eta, week.to, false));
        }
    }
    for (id, (over, at, handed)) in late {
        let why = if handed { format!("handed in {} after it said", span(over)) } else { format!("still open, {} past when it said", span(over)) };
        troubles.push(Trouble { snag: Snag::Late, ticket: Some(id), title: ticket(id).map(|t| t.title.clone()), member: worker(id), why, at });
    }
    troubles.sort_by_key(|t| (t.snag, t.at, t.ticket));

    // Time by member, then by ticket.
    let mut time: Vec<Time> = Vec::new();
    for t in &turns {
        let at = match time.iter().position(|m| m.member == t.member) {
            Some(at) => at,
            None => {
                time.push(Time { member: t.member.clone(), role: t.role.clone(), ..Time::default() });
                time.len() - 1
            }
        };
        let m = &mut time[at];
        m.turns += 1;
        m.secs += t.secs();
        m.tokens += t.tokens;
        match m.tickets.iter_mut().find(|(id, _, _)| *id == t.ticket) {
            Some(row) => {
                row.1 += 1;
                row.2 += t.secs();
            }
            None => m.tickets.push((t.ticket, 1, t.secs())),
        }
    }
    for m in &mut time {
        m.tickets.sort_by_key(|(id, _, secs)| (std::cmp::Reverse(*secs), id.unwrap_or(0)));
    }
    time.sort_by_key(|m| (std::cmp::Reverse(m.secs), m.member.clone()));

    // The waits on the person: answered this week, or waiting still.
    let mut waits: Vec<Wait> = week.waits.iter().filter(|w| w.answered.map_or(w.asked < week.to, inside)).cloned().collect();
    waits.sort_by_key(|w| (std::cmp::Reverse(w.secs(week.to)), w.asked));

    // Friction, member by member: what went wrong and what it waited for.
    let mut friction: Vec<Friction> = Vec::new();
    let role_of = |member: &str| time.iter().find(|m| m.member == member).map(|m| m.role.clone()).unwrap_or_else(|| crate::eta::role_of(member).to_owned());
    let rub = |friction: &mut Vec<Friction>, member: &str| -> usize {
        match friction.iter().position(|f| f.member == member) {
            Some(at) => at,
            None => {
                friction.push(Friction { member: member.to_owned(), role: role_of(member), ..Friction::default() });
                friction.len() - 1
            }
        }
    };
    for t in &troubles {
        if let Some(member) = &t.member {
            let at = rub(&mut friction, member);
            *friction[at].snags.entry(t.snag).or_default() += 1;
        }
    }
    for w in &waits {
        let at = rub(&mut friction, &w.member);
        friction[at].waits += 1;
        friction[at].waited += w.secs(week.to);
    }
    friction.sort_by_key(|f| (std::cmp::Reverse(f.snags.values().sum::<usize>() + f.waits), f.member.clone()));

    Facts { from: week.from, to: week.to, shipped, troubles, time, waits, friction, briefs: week.briefs.clone() }
}

// ── the facts, as the PM reads them ──

/// The week's facts as the PM is handed them: a section each, the evidence
/// the retro is written from. Dates as `clock` reads them.
pub fn facts_text(facts: &Facts, clock: &impl Clock) -> String {
    let day = |at: u64| {
        let p = clock.parts(at);
        format!("{:04}-{:02}-{:02}", p.year, p.month, p.day)
    };
    let ticket = |id: Option<TicketId>| id.map_or_else(|| "at its desk".to_owned(), |id| format!("T-{id}"));
    let mut out = format!("{FACTS_HEAD}{} – {}]\nGathered by equipe from the squad's journal, {} to {}: the evidence to write from. Nothing here is guessed; what it doesn't say, equipe doesn't know.", day(facts.from), day(facts.to), day(facts.from), day(facts.to));

    out.push_str(&format!("\n\n{SHIPPED_HEAD}{}):", facts.shipped.len()));
    if facts.shipped.is_empty() {
        out.push_str("\n- nothing was accepted or merged this week");
    }
    for s in &facts.shipped {
        let mut line = format!("\n- T-{} {}", s.id, s.title);
        let about: Vec<String> = [s.member.clone(), s.project.clone()].into_iter().flatten().collect();
        if !about.is_empty() {
            line.push_str(&format!(" — {}", about.join(", ")));
        }
        line.push_str(if s.on_main { ", on main" } else { ", accepted, still on its branch" });
        if let Some(branch) = &s.branch {
            line.push_str(&format!(" ({branch})"));
        }
        out.push_str(&line);
    }

    out.push_str(&format!("\n\n{TROUBLES_HEAD}{}):", facts.troubles.len()));
    if facts.troubles.is_empty() {
        out.push_str("\n- nothing");
    }
    for t in &facts.troubles {
        let who = t.member.as_deref().map(|m| format!(" · {m}")).unwrap_or_default();
        let title = t.title.as_deref().map(|title| format!(" {title}")).unwrap_or_default();
        let on = t.ticket.map(|id| format!("T-{id}{title}")).unwrap_or_else(|| "at the desk".to_owned());
        out.push_str(&format!("\n- {} · {on}{who}: {}", t.snag.word(), t.why));
    }

    let waited: u64 = facts.waits.iter().map(|w| w.secs(facts.to)).sum();
    out.push_str(&format!("\n\nWaits on the person ({}{}):", facts.waits.len(), if facts.waits.is_empty() { String::new() } else { format!(", {} in all", span(waited)) }));
    if facts.waits.is_empty() {
        out.push_str("\n- none");
    }
    for w in &facts.waits {
        let on = w.ticket.map(|id| format!(" · T-{id}")).unwrap_or_default();
        let how = match w.answered {
            Some(_) => format!("{}, {}", span(w.secs(facts.to)), w.outcome),
            None => format!("still waiting, {}", span(w.secs(facts.to))),
        };
        out.push_str(&format!("\n- {} · {}{on} · {} — {how}", w.kind, w.member, clip(&one_line(&w.what), 100)));
    }

    out.push_str("\n\nTime by member (turns · time · usage), then by ticket:");
    if facts.time.is_empty() {
        out.push_str("\n- no turns were kept this week");
    }
    for m in &facts.time {
        let usage = if m.tokens > 0 { format!(" · {} tokens", tokens(m.tokens)) } else { String::new() };
        let tickets: Vec<String> = m.tickets.iter().take(5).map(|(id, n, secs)| format!("{} {n} turn{} {}", ticket(*id), if *n == 1 { "" } else { "s" }, span(*secs))).collect();
        let more = if m.tickets.len() > 5 { format!("; and {} more", m.tickets.len() - 5) } else { String::new() };
        out.push_str(&format!("\n- {} ({}): {} turn{} · {}{usage} — {}{more}", m.member, m.role, m.turns, if m.turns == 1 { "" } else { "s" }, span(m.secs), tickets.join("; ")));
    }

    out.push_str("\n\nFriction by member:");
    if facts.friction.is_empty() {
        out.push_str("\n- none");
    }
    for f in &facts.friction {
        let mut parts: Vec<String> = f.snags.iter().map(|(snag, n)| format!("{n} {}", snag.word())).collect();
        if f.waits > 0 {
            parts.push(format!("waited on the person {}× ({})", f.waits, span(f.waited)));
        }
        out.push_str(&format!("\n- {} ({}): {}", f.member, f.role, parts.join(", ")));
    }

    if !facts.briefs.is_empty() {
        out.push_str("\n\nBriefs — what was expected (read each, and set it beside what happened):");
        for b in &facts.briefs {
            out.push_str(&format!("\n- {}: {} — {}", b.project, b.file, b.title));
        }
    }
    out
}

/// What the PM is handed: the routine as any is, then the week's facts.
pub fn message(routine: &Routine, facts: &str) -> String {
    format!("{}\n\n{facts}", crate::routines::message(routine))
}

/// The week a retro's message covers, `(from, to)` as its facts say: what
/// screens tell a retro by. `None`: any other message.
pub fn week_of(text: &str) -> Option<(&str, &str)> {
    let line = text.lines().find_map(|l| l.strip_prefix(FACTS_HEAD))?;
    let (from, to) = line.strip_suffix(']')?.split_once(" – ")?;
    (!from.is_empty() && !to.is_empty()).then_some((from, to))
}

/// What a retro's facts count: `(shipped, stuck or failed)`. `None`: no facts.
pub fn counts(text: &str) -> Option<(usize, usize)> {
    let count = |head: &str| text.lines().find_map(|l| l.strip_prefix(head)).and_then(|rest| rest.split(')').next()).and_then(|n| n.parse().ok());
    week_of(text)?;
    Some((count(SHIPPED_HEAD)?, count(TROUBLES_HEAD)?))
}

/// Whether a decision the PM puts at `now` is the retro's changes: the
/// retro was handed to it at `handed`, at most [`ASK_WITHIN`] before.
pub fn is_its_proposal(handed: Option<u64>, now: u64) -> bool {
    handed.is_some_and(|at| now.saturating_sub(at) <= ASK_WITHIN)
}

// ── the lessons ──

/// The lessons a retro's `ask_decision` gives, question by question (empty:
/// none): a line each, at most [`MOST`] given.
pub fn lessons_of(args: &Value, questions: usize) -> Retro {
    let items: Vec<Value> = match args.get("questions") {
        Some(Value::Array(items)) => items.clone(),
        Some(one @ Value::Object(_)) => vec![one.clone()],
        _ => Vec::new(),
    };
    let mut given = 0;
    let lessons = (0..questions)
        .map(|n| {
            let lesson = items.get(n).and_then(|q| q.get("lesson")).and_then(Value::as_str).map(one_line).unwrap_or_default();
            if lesson.is_empty() || given >= MOST {
                return String::new();
            }
            given += 1;
            clip(&lesson, 200)
        })
        .collect();
    Retro { lessons }
}

/// The lessons the person confirmed: those of the changes they took — any
/// option but the last ("not this week"), picked. Skipped, declined, or
/// answered only in their own words: not confirmed.
pub fn confirmed(set: &QuestionSet, answers: &[Answer]) -> Vec<String> {
    let Some(retro) = &set.retro else { return Vec::new() };
    set.questions
        .iter()
        .zip(answers)
        .zip(&retro.lessons)
        .filter(|((q, a), lesson)| {
            let last = q.options.len().checked_sub(1);
            !lesson.trim().is_empty() && q.options.len() >= 2 && !a.picked.is_empty() && !a.picked.iter().any(|i| Some(*i) == last)
        })
        .map(|(_, lesson)| lesson.trim().to_owned())
        .take(MOST)
        .collect()
}

/// The confirmed lessons as one entry of the memory, under the day:
/// `## 2026-11-06` then a line each. `None`: none confirmed.
pub fn entry(today: &str, lessons: &[String]) -> Option<String> {
    (!lessons.is_empty()).then(|| format!("## {today}\n{}", lessons.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n")))
}

/// The lessons' body with `entry` added first, the oldest let go past a year's.
pub fn with(previous: Option<&str>, entry: &str) -> String {
    let mut entries = vec![entry.trim().to_owned()];
    if let Some(previous) = previous {
        let text = format!("\n{}", previous.trim());
        entries.extend(text.split("\n## ").map(str::trim).filter(|e| !e.is_empty()).map(|e| if e.starts_with("## ") { e.to_owned() } else { format!("## {e}") }));
    }
    entries.truncate(KEEP);
    entries.join("\n\n")
}

// ── words ──

/// A span of seconds, short: `40m`, `3h 05m`, `2d 4h`.
pub fn span(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    match (d, h) {
        (0, 0) => format!("{}m", m.max(u64::from(secs > 0))),
        (0, h) => format!("{h}h {m:02}m"),
        (d, h) => format!("{d}d {h}h"),
    }
}

/// Tokens, short: `900`, `34k`, `1.2M`.
pub fn tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{}k", n / 1_000),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn quoted(text: &str) -> String {
    let text = clip(text.trim(), 140);
    if text.is_empty() { String::new() } else { format!(": \u{201c}{text}\u{201d}") }
}

fn clip(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        return text.to_owned();
    }
    let kept: String = text.chars().take(most - 1).collect();
    format!("{}…", kept.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routines::{Parts, Utc, next_run};
    use equipe_protocol::decide::{Choice, Question};

    /// Friday 2026-11-06 17:00 UTC.
    const FRI_5: u64 = 1_793_984_400;
    const HOUR: u64 = 3600;
    const DAY: u64 = 24 * HOUR;
    const MON: u64 = FRI_5 - 4 * DAY - 8 * HOUR; // Monday 09:00

    fn ticket(id: TicketId, title: &str, status: TicketStatus, who: &str) -> Ticket {
        Ticket {
            id,
            title: title.into(),
            body: String::new(),
            status,
            kind: equipe_protocol::TicketKind::Code,
            owner: Some(who.into()),
            assignee: None,
            project: Some(PathBuf::from("/p/site")),
            depends_on: vec![],
            branch: Some(format!("equipe/t-{id}")),
            worktree: None,
            merged: None,
            off_main: None,
            design_qa: None,
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

    fn event(id: TicketId, at: u64, by: &str, kind: &str, text: &str) -> (TicketId, TicketEntry) {
        (id, TicketEntry { at, by: by.into(), kind: kind.into(), text: text.into() })
    }

    fn turn(member: &str, ticket: Option<TicketId>, began: u64, secs: u64, outcome: &str, tokens: u64) -> Turn {
        Turn { member: member.into(), role: member.into(), ticket, began, ended: began + secs, outcome: outcome.into(), tokens }
    }

    type Fixture = (Vec<Ticket>, Vec<(TicketId, TicketEntry)>, Vec<Turn>, Vec<Wait>);

    /// A week of the squad: two shipped (one still on its branch), one sent
    /// back, one late, one merge that failed; turns long, failed and plain;
    /// a permission and a decision waited on, and one waiting still.
    fn fixture() -> Fixture {
        let mut on_branch = ticket(13, "Login", TicketStatus::Done, "backend");
        on_branch.merged = Some(false);
        let tickets = vec![
            ticket(12, "Pricing page", TicketStatus::Done, "frontend"),
            on_branch,
            ticket(14, "Search", TicketStatus::InProgress, "backend"),
            ticket(15, "Onboarding", TicketStatus::InProgress, "frontend"),
            ticket(9, "Old work", TicketStatus::Done, "frontend"),
        ];
        let events = vec![
            event(9, MON - 3 * DAY, "you", "merged", "main"),
            event(12, MON + HOUR, "equipe", "assigned", "frontend"),
            event(12, MON + 2 * HOUR, "frontend", "estimated", &(MON + 5 * HOUR).to_string()),
            event(12, MON + 4 * HOUR, "frontend", "handed in", ""),
            event(12, MON + DAY, "you", "accepted", ""),
            event(12, MON + DAY + 60, "you", "merged", "main"),
            event(13, MON + 2 * DAY, "you", "accepted", ""),
            event(14, MON + DAY, "backend", "estimated", &(MON + DAY + 2 * HOUR).to_string()),
            event(14, MON + DAY + 5 * HOUR, "backend", "handed in", ""),
            event(14, MON + 2 * DAY, "you", "reopened", "Reopened: the results are in the wrong order"),
            event(14, MON + 3 * DAY, "equipe", "not merged", "conflict in search.rs"),
            event(15, MON + 3 * DAY, "frontend", "estimated", &(MON + 3 * DAY + HOUR).to_string()),
        ];
        let turns = vec![
            turn("frontend", Some(12), MON + HOUR, 25 * 60, "completed", 400_000),
            turn("frontend", Some(15), MON + 3 * DAY, 20 * 60, "completed", 50_000),
            turn("backend", Some(14), MON + DAY, 50 * 60, "completed", 900_000),
            turn("backend", Some(14), MON + DAY + 2 * HOUR, 10 * 60, "failed", 20_000),
            turn("backend", Some(14), MON + DAY + 3 * HOUR, 5 * 60, "failed", 10_000),
            turn("pm", None, MON, 15 * 60, "completed", 80_000),
            turn("pm", None, MON - 4 * DAY, 15 * 60, "completed", 80_000),
        ];
        let waits = vec![
            Wait { kind: "permission".into(), member: "backend".into(), ticket: Some(14), what: "git push origin equipe/t-14".into(), asked: MON + DAY, answered: Some(MON + DAY + 3 * HOUR), outcome: "allowed".into() },
            Wait { kind: "decision".into(), member: "pm".into(), ticket: None, what: "Which database?".into(), asked: MON + 2 * DAY, answered: Some(MON + 2 * DAY + 40 * 60), outcome: "answered".into() },
            Wait { kind: "decision".into(), member: "designer".into(), ticket: None, what: "Which logo?".into(), asked: FRI_5 - 2 * DAY, answered: None, outcome: String::new() },
            Wait { kind: "permission".into(), member: "backend".into(), ticket: None, what: "last week's".into(), asked: MON - 3 * DAY, answered: Some(MON - 3 * DAY + 60), outcome: "allowed".into() },
        ];
        (tickets, events, turns, waits)
    }

    fn week_facts() -> Facts {
        let (tickets, events, turns, waits) = fixture();
        let briefs = vec![Brief { project: "site".into(), file: "docs/briefs/pricing.md".into(), title: "Pricing that converts".into() }];
        gather(&Week { from: FRI_5 - WEEK, to: FRI_5, tickets: &tickets, events: &events, turns: &turns, waits: &waits, briefs })
    }

    #[test]
    fn the_retro_comes_on_friday_evenings_to_the_pm() {
        let r = routine(Lang::Ko, 3);
        assert_eq!((r.id, r.on, r.when), (3, true, Schedule::Weekly { weekday: 5, hour: 17, minute: 0 }));
        assert_eq!(r.title, "주간 회고");
        assert_eq!(ROLE, "pm");
        assert_eq!(Utc.parts(FRI_5), Parts { year: 2026, month: 11, day: 6, weekday: 5, hour: 17, minute: 0 });
        // Monday: this Friday at five. Friday at five sharp: next week's.
        assert_eq!(next_run(r.when, MON, &Utc), FRI_5);
        assert_eq!(next_run(r.when, FRI_5, &Utc), FRI_5 + WEEK);
        // The week looks back to the last retro, or a week.
        assert_eq!(since(None, FRI_5), FRI_5 - WEEK);
        assert_eq!(since(Some(FRI_5 - 5 * DAY), FRI_5), FRI_5 - 5 * DAY, "moved to Sunday, then back");
        assert_eq!(since(Some(FRI_5 - 30 * DAY), FRI_5), FRI_5 - WEEK, "off for a month: a week");
    }

    #[test]
    fn what_shipped_is_what_was_accepted_or_merged_this_week_and_is_done() {
        let facts = week_facts();
        let shipped: Vec<(TicketId, bool, Option<&str>)> = facts.shipped.iter().map(|s| (s.id, s.on_main, s.member.as_deref())).collect();
        assert_eq!(shipped, [(12, true, Some("frontend")), (13, false, Some("backend"))], "T-9 was last week's; T-14 was sent back");
        assert_eq!(facts.shipped[0].project.as_deref(), Some("site"));
    }

    #[test]
    fn what_got_stuck_failed_was_sent_back_or_late_and_why() {
        let facts = week_facts();
        let got: Vec<(Snag, Option<TicketId>, Option<&str>, &str)> = facts.troubles.iter().map(|t| (t.snag, t.ticket, t.member.as_deref(), t.why.as_str())).collect();
        assert_eq!(
            got,
            [
                (Snag::Failed, Some(14), Some("backend"), "a turn failed (2×)"),
                (Snag::Failed, Some(14), Some("backend"), "merging failed: \u{201c}conflict in search.rs\u{201d}"),
                (Snag::SentBack, Some(14), Some("backend"), "sent back by you: \u{201c}the results are in the wrong order\u{201d}"),
                (Snag::Stuck, Some(14), Some("backend"), "a turn ran 50m"),
                (Snag::Late, Some(14), Some("backend"), "handed in 3h 00m after it said"),
                (Snag::Late, Some(15), Some("frontend"), "still open, 1d 7h past when it said"),
            ]
        );
        // T-12 was handed in an hour before it said, its turn short: nothing went wrong.
        assert!(!facts.troubles.iter().any(|t| t.ticket == Some(12)));
        assert_eq!(facts.troubles[2].title.as_deref(), Some("Search"));
    }

    #[test]
    fn each_members_time_turns_and_usage_by_ticket_and_where_it_rubbed() {
        let facts = week_facts();
        let backend = facts.time.iter().find(|m| m.member == "backend").unwrap();
        assert_eq!((backend.turns, backend.secs, backend.tokens), (3, 65 * 60, 930_000));
        assert_eq!(backend.tickets, [(Some(14), 3, 65 * 60)]);
        let pm = facts.time.iter().find(|m| m.member == "pm").unwrap();
        assert_eq!((pm.turns, pm.tickets.clone()), (1, vec![(None, 1, 15 * 60)]), "last week's turn left out");
        assert_eq!(facts.time[0].member, "backend", "the most time first");
        // Waits: answered this week or waiting still, the longest first; last week's left out.
        let waits: Vec<(&str, u64)> = facts.waits.iter().map(|w| (w.what.as_str(), w.secs(FRI_5))).collect();
        assert_eq!(waits, [("Which logo?", 2 * DAY), ("git push origin equipe/t-14", 3 * HOUR), ("Which database?", 40 * 60)]);
        let backend = facts.friction.iter().find(|f| f.member == "backend").unwrap();
        assert_eq!(backend.snags, BTreeMap::from([(Snag::Failed, 2), (Snag::SentBack, 1), (Snag::Stuck, 1), (Snag::Late, 1)]));
        assert_eq!((backend.waits, backend.waited), (1, 3 * HOUR));
        assert_eq!(facts.friction[0].member, "backend", "the most friction first");
        assert!(facts.friction.iter().any(|f| f.member == "designer" && f.waits == 1));
    }

    #[test]
    fn the_facts_read_as_evidence_section_by_section() {
        let text = facts_text(&week_facts(), &Utc);
        assert!(text.starts_with("[Retro facts · 2026-10-30 – 2026-11-06]\n"), "{text}");
        for must in [
            "Shipped (2):\n- T-12 Pricing page — frontend, site, on main (equipe/t-12)\n- T-13 Login — backend, site, accepted, still on its branch (equipe/t-13)",
            "- sent back · T-14 Search · backend: sent back by you: \u{201c}the results are in the wrong order\u{201d}",
            "- late · T-15 Onboarding · frontend: still open, 1d 7h past when it said",
            "Waits on the person (3, 2d 3h in all):",
            "- permission · backend · T-14 · git push origin equipe/t-14 — 3h 00m, allowed",
            "- decision · designer · Which logo? — still waiting, 2d 0h",
            "- backend (backend): 3 turns · 1h 05m · 930k tokens — T-14 3 turns 1h 05m",
            "- pm (pm): 1 turn · 15m · 80k tokens — at its desk 1 turn 15m",
            "- backend (backend): 2 failed, 1 sent back, 1 stuck, 1 late, waited on the person 1× (3h 00m)",
            "Briefs — what was expected",
            "- site: docs/briefs/pricing.md — Pricing that converts",
        ] {
            assert!(text.contains(must), "{must}\n---\n{text}");
        }
        // An empty week says so, and lists no briefs.
        let empty = facts_text(&gather(&Week { from: FRI_5 - WEEK, to: FRI_5, ..Week::default() }), &Utc);
        for must in ["Shipped (0):\n- nothing was accepted or merged this week", "late (0):\n- nothing", "Waits on the person (0):\n- none", "- no turns were kept this week"] {
            assert!(empty.contains(must), "{must}\n---\n{empty}");
        }
        assert!(!empty.contains("Briefs"));
    }

    #[test]
    fn the_pm_is_handed_the_routine_then_the_facts_and_screens_tell_it_by_them() {
        let r = routine(Lang::En, 3);
        let facts = facts_text(&week_facts(), &Utc);
        let handed = message(&r, &facts);
        assert!(handed.starts_with("[Routine · Weekly retro]\n\n"), "{handed}");
        assert!(handed.ends_with(&facts));
        assert_eq!(week_of(&handed), Some(("2026-10-30", "2026-11-06")));
        assert_eq!(week_of(&crate::routines::message(&r)), None, "no facts: no retro");
        assert_eq!(week_of("[Retro facts · ]"), None);
        assert_eq!(counts(&handed), Some((2, 6)));
        assert_eq!(counts(&crate::routines::message(&r)), None);
        assert!(is_its_proposal(Some(FRI_5), FRI_5 + 30 * 60));
        assert!(!is_its_proposal(Some(FRI_5), FRI_5 + ASK_WITHIN + 1));
        assert!(!is_its_proposal(None, FRI_5));
    }

    #[test]
    fn the_prompt_asks_for_one_message_from_the_facts_and_proposes_without_changing_anything() {
        for lang in Lang::ALL {
            let prompt = routine(lang, 1).prompt;
            for must in ["[Retro facts · …]", "ask_decision", "`lesson`", "docs/briefs/", "propose_routine", "\"lessons\"", "T-"] {
                assert!(prompt.contains(must), "{lang:?} lacks {must}: {prompt}");
            }
            assert!(prompt.matches("\n1. ").count() == 1 && prompt.contains("\n2. ") && prompt.contains("\n3. ") && !prompt.contains("\n4. "), "{lang:?}: three parts, numbered");
            if lang != Lang::Ko {
                assert!(!prompt.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c)), "{lang:?}");
                assert!(!routine(lang, 1).title.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c)), "{lang:?}");
            }
        }
        let en = routine(Lang::En, 1).prompt;
        for must in ["one message", "Weekly retro", "at most three lessons", "at most three changes", "one click", "last option", "Change nothing yourself", "invent nothing", "expected"] {
            assert!(en.contains(must), "{must}: {en}");
        }
        let ko = routine(Lang::Ko, 1).prompt;
        assert!(ko.contains("주간 회고") && ko.contains("최대 세 가지"), "{ko}");
    }

    fn change(question: &str) -> Question {
        let option = |label: &str| Choice { label: label.into(), ..Choice::default() };
        Question { question: question.into(), options: vec![option("Do it (Recommended)"), option("Lesson only"), option("Not this week")], ..Question::default() }
    }

    fn retro_set(lessons: &[&str]) -> QuestionSet {
        QuestionSet {
            id: 1,
            session: 1,
            member: "pm".into(),
            ticket: None,
            questions: vec![change("Split tickets?"), change("Review twice a day?"), change("Freeze Fridays?"), change("Ask sooner?")],
            asked_at: 0,
            review: false,
            retro: Some(Retro { lessons: lessons.iter().map(|l| (*l).to_owned()).collect() }),
        }
    }

    #[test]
    fn only_the_lessons_of_changes_the_person_took_are_filed_under_the_day() {
        let set = retro_set(&["Big tickets stall", "Approvals wait overnight", "", "Ask before guessing"]);
        let answers = [Answer::pick(0), Answer::pick(2), Answer::pick(0), Answer { other: Some("maybe later".into()), ..Answer::default() }];
        // Taken: the first; declined: the second; the third has no lesson; the fourth only words.
        assert_eq!(confirmed(&set, &answers), ["Big tickets stall"]);
        assert_eq!(confirmed(&set, &[Answer::pick(1)]), ["Big tickets stall"], "the lesson only, kept");
        assert!(confirmed(&set, &[Answer::skipped(), Answer::skipped()]).is_empty(), "skipped: not confirmed");
        assert!(confirmed(&QuestionSet { retro: None, ..set.clone() }, &[Answer::pick(0)]).is_empty(), "not the retro");
        let entry_text = entry("2026-11-06", &confirmed(&set, &answers)).unwrap();
        assert_eq!(entry_text, "## 2026-11-06\n- Big tickets stall");
        assert_eq!(entry("2026-11-06", &[]), None, "none confirmed: nothing filed");
        let body = with(Some("## 2026-10-30\n- Earlier"), &entry_text);
        assert_eq!(body, "## 2026-11-06\n- Big tickets stall\n\n## 2026-10-30\n- Earlier");
        assert_eq!(with(None, &entry_text), entry_text);
    }

    #[test]
    fn the_lessons_come_from_the_retros_questions_a_line_each_three_at_most() {
        let args = serde_json::json!({ "questions": [
            { "question": "a?", "lesson": "  Big tickets\nstall " },
            { "question": "b?" },
            { "question": "c?", "lesson": "Two" },
            { "question": "d?", "lesson": "Three" },
        ]});
        assert_eq!(lessons_of(&args, 4).lessons, ["Big tickets stall", "", "Two", "Three"]);
        let four = serde_json::json!({ "questions": [{ "lesson": "1" }, { "lesson": "2" }, { "lesson": "3" }, { "lesson": "4" }] });
        assert_eq!(lessons_of(&four, 4).lessons, ["1", "2", "3", ""], "three at most");
        assert_eq!(lessons_of(&serde_json::json!({}), 2).lessons, ["", ""]);
    }

    #[test]
    fn briefs_are_read_from_each_projects_docs_briefs() {
        let root = std::env::temp_dir().join(format!("equipe-retro-briefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (site, api) = (root.join("site"), root.join("api"));
        std::fs::create_dir_all(site.join(BRIEFS)).unwrap();
        std::fs::create_dir_all(&api).unwrap();
        std::fs::write(site.join(BRIEFS).join("pricing.md"), "\n# Pricing that converts\n\nMore.").unwrap();
        std::fs::write(site.join(BRIEFS).join("notes.txt"), "not a brief").unwrap();
        let found = briefs(&[site.clone(), api]);
        assert_eq!(found, [Brief { project: "site".into(), file: "docs/briefs/pricing.md".into(), title: "Pricing that converts".into() }]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn spans_and_tokens_read_short() {
        assert_eq!((span(0), span(30), span(40 * 60), span(3 * HOUR + 5 * 60), span(2 * DAY + 4 * HOUR)), ("0m".into(), "1m".into(), "40m".into(), "3h 05m".into(), "2d 4h".into()));
        assert_eq!((tokens(900), tokens(34_500), tokens(1_250_000)), ("900".into(), "34k".into(), "1.2M".into()));
    }
}
