//! A squad's own record, measured over a window of time: how often handed-in
//! work was accepted the first time, how often it was sent back, how often
//! the person stepped in, what skills the situations called for and whether
//! members opened them, and the tool calls and tokens a ticket took.
//!
//! It only reads: the squad's journal (opened read-only) and, when given
//! Claude Code's projects folder, the transcripts of the squad's own working
//! directories — its folder and its tickets' worktrees. What it finds is for
//! the person who runs it, on their machine (`crates/core/examples/quality_metrics.rs`).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use super::{Rules, Transcript};

/// From `from` (included) to `to` (left out), in unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub from: u64,
    pub to: u64,
}

impl Window {
    pub fn holds(&self, at: u64) -> bool {
        at >= self.from && at < self.to
    }
}

/// What sends handed-in work back to its member.
const SENT_BACK: &[&str] = &["reopened", "sent back", "design QA: changes"];

/// A role's share of the window.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoleLive {
    pub handed_in: usize,
    pub accepted: usize,
    pub first_time: usize,
    pub reworked: usize,
    pub tokens: u64,
}

/// A window of a squad's record, measured.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Live {
    /// Tickets first handed in within the window.
    pub handed_in: usize,
    /// Of those, accepted by now (by the person or the PM).
    pub accepted: usize,
    /// Of those accepted, with nothing sent back between the hand-in and the acceptance.
    pub first_time: usize,
    /// Of those handed in, sent back at least once (reopened, sent back, design QA's changes).
    pub reworked: usize,
    /// The person's own reopens, ticket comments and messages to members other than the PM, in the window.
    pub person_reopens: usize,
    pub person_comments: usize,
    pub person_messages: usize,
    /// Tickets that had turns in the window, and their turns and tokens each.
    pub tickets_worked: usize,
    pub turns_per_ticket: Vec<u64>,
    pub tokens_per_ticket: Vec<u64>,
    /// Members' tool calls in the window (the PM's apart).
    pub tool_calls: usize,
    pub pm_tool_calls: usize,
    pub by_role: BTreeMap<String, RoleLive>,
    /// Transcripts read, and what they show.
    pub transcripts: usize,
    pub rules: Rules,
    /// Each skill: (opened in time, called for).
    pub skills: BTreeMap<String, (usize, usize)>,
}

impl Live {
    pub fn first_time_rate(&self) -> Option<f64> {
        ratio(self.first_time, self.accepted)
    }

    pub fn rework_rate(&self) -> Option<f64> {
        ratio(self.reworked, self.handed_in)
    }

    /// The person stepping in — reopens, comments, messages to members — per ticket handed in.
    pub fn corrections_per_ticket(&self) -> Option<f64> {
        (self.handed_in > 0).then(|| (self.person_reopens + self.person_comments + self.person_messages) as f64 / self.handed_in as f64)
    }
}

pub fn ratio(n: usize, of: usize) -> Option<f64> {
    (of > 0).then(|| n as f64 / of as f64)
}

/// The middle of `xs`.
pub fn median(xs: &[u64]) -> Option<u64> {
    let mut v = xs.to_vec();
    v.sort_unstable();
    (!v.is_empty()).then(|| v[v.len() / 2])
}

/// The role of a member's name: `backend-2` is a backend.
fn role_of(name: &str) -> String {
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit());
    trimmed.strip_suffix('-').unwrap_or(name).to_owned()
}

/// Whether a user line in a member's conversation is the person's own words,
/// not equipe's or the engine's on their behalf.
pub fn is_persons_own(text: &str) -> bool {
    let t = text.trim_start();
    !(t.is_empty() || t.starts_with('<') || t.starts_with('[') || ["Answers to your question", "The person ", "Another Claude"].iter().any(|p| t.starts_with(p)))
}

/// Measure `window` of the squad whose journal is `db`. `projects`: Claude
/// Code's projects folder, to read the squad's transcripts too; `squad_home`:
/// the squad's folder, members' desk.
pub fn measure(db: &Path, window: Window, projects: Option<&Path>, squad_home: Option<&Path>) -> rusqlite::Result<Live> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let mut live = Live::default();

    // Tickets: who worked each, where.
    let mut tickets: HashMap<i64, (Option<String>, Option<String>)> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT id, assignee, owner, worktree FROM tickets")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?)))?;
        for row in rows {
            let (id, assignee, owner, worktree) = row?;
            tickets.insert(id, (assignee.or(owner).map(|a| role_of(&a)), worktree));
        }
    }

    // Each ticket's record, in order.
    let mut events: BTreeMap<i64, Vec<(u64, String, String)>> = BTreeMap::new();
    {
        let mut stmt = conn.prepare("SELECT ticket, at, by, kind FROM ticket_events ORDER BY ticket, seq")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?.max(0) as u64, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
        for row in rows {
            let (ticket, at, by, kind) = row?;
            if by == "you" && window.holds(at) {
                match kind.as_str() {
                    "comment" => live.person_comments += 1,
                    "reopened" => live.person_reopens += 1,
                    _ => {}
                }
            }
            events.entry(ticket).or_default().push((at, by, kind));
        }
    }
    for (ticket, record) in &events {
        let Some(at) = record.iter().position(|(_, _, k)| k == "handed in") else { continue };
        if !window.holds(record[at].0) {
            continue;
        }
        let after = &record[at..];
        let accepted_at = after.iter().position(|(_, _, k)| k.starts_with("accepted"));
        let sent_back = |range: &[(u64, String, String)]| range.iter().any(|(_, _, k)| SENT_BACK.contains(&k.as_str()));
        let role = tickets.get(ticket).and_then(|(r, _)| r.clone()).unwrap_or_else(|| "?".into());
        let r = live.by_role.entry(role).or_default();
        live.handed_in += 1;
        r.handed_in += 1;
        if sent_back(after) {
            live.reworked += 1;
            r.reworked += 1;
        }
        if let Some(acc) = accepted_at {
            live.accepted += 1;
            r.accepted += 1;
            if !sent_back(&after[..acc]) {
                live.first_time += 1;
                r.first_time += 1;
            }
        }
    }

    // Turns and tokens by ticket.
    {
        let mut per: BTreeMap<i64, (u64, u64)> = BTreeMap::new();
        let mut stmt = conn.prepare("SELECT ticket, role, ended, tokens FROM turns WHERE ticket IS NOT NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?.max(0) as u64, r.get::<_, i64>(3)?.max(0) as u64)))?;
        for row in rows {
            let (ticket, role, ended, tokens) = row?;
            if !window.holds(ended) {
                continue;
            }
            let e = per.entry(ticket).or_default();
            e.0 += 1;
            e.1 += tokens;
            live.by_role.entry(role_of(&role)).or_default().tokens += tokens;
        }
        live.tickets_worked = per.len();
        live.turns_per_ticket = per.values().map(|(n, _)| *n).collect();
        live.tokens_per_ticket = per.values().map(|(_, t)| *t).collect();
    }

    // Members' conversations: tool calls, and the person's own messages.
    let mut roles: HashMap<i64, String> = HashMap::new();
    {
        let has_role = conn.prepare("SELECT role, name FROM sessions LIMIT 0").is_ok();
        let sql = if has_role { "SELECT id, COALESCE(role, name, '') FROM sessions" } else { "SELECT id, '' FROM sessions" };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, role) = row?;
            roles.insert(id, role_of(&role));
        }
    }
    {
        let mut stmt = conn.prepare("SELECT session, body FROM entries WHERE at >= ?1 AND at < ?2")?;
        let rows = stmt.query_map([window.from as i64, window.to as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (session, body) = row?;
            let Ok(entry) = serde_json::from_str::<Value>(&body) else { continue };
            let pm = roles.get(&session).is_some_and(|r| r == crate::roles::PM);
            match entry.get("kind").and_then(Value::as_str) {
                Some("tool") if pm => live.pm_tool_calls += 1,
                Some("tool") => live.tool_calls += 1,
                Some("user") if !pm && entry.get("text").and_then(Value::as_str).is_some_and(is_persons_own) => live.person_messages += 1,
                _ => {}
            }
        }
    }

    // Transcripts of the squad's own working directories.
    if let Some(projects) = projects {
        let mut dirs: Vec<(String, Option<String>)> = tickets.values().filter_map(|(role, wt)| wt.as_ref().map(|w| (super::claude_project_dir_name(Path::new(w)), role.clone()))).collect();
        if let Some(home) = squad_home {
            dirs.push((super::claude_project_dir_name(home), None));
        }
        dirs.sort();
        dirs.dedup();
        for (dir, role) in dirs {
            for path in super::transcripts_under(&projects.join(&dir)) {
                let Ok(t) = Transcript::read(&path) else { continue };
                if !t.first_at.as_deref().and_then(unix_of).is_some_and(|at| window.holds(at)) {
                    continue;
                }
                live.transcripts += 1;
                live.rules.add(&super::rules(&t.steps));
                if !t.sidechain {
                    for c in super::skill_checks(&t, role.as_deref().or(t.role.as_deref())) {
                        let e = live.skills.entry(c.skill).or_default();
                        e.1 += 1;
                        if c.opened.in_time() {
                            e.0 += 1;
                        }
                    }
                }
            }
        }
    }
    Ok(live)
}

/// Unix seconds of a UTC date or time as people and Claude Code write them:
/// `2026-11-06`, `2026-11-06T09:30`, `2026-11-06T09:30:12.345Z`; or unix
/// seconds themselves.
pub fn unix_of(text: &str) -> Option<u64> {
    let text = text.trim();
    if let Ok(secs) = text.parse::<u64>() {
        return Some(secs);
    }
    let (date, time) = text.split_once(['T', ' ']).unwrap_or((text, ""));
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    let time = time.trim_end_matches('Z');
    let time = time.split(['+', '.']).next().unwrap_or("");
    let mut t = time.split(':').filter(|p| !p.is_empty()).map(|p| p.parse::<i64>().ok());
    let (hh, mm, ss) = (t.next().flatten().unwrap_or(0), t.next().flatten().unwrap_or(0), t.next().flatten().unwrap_or(0));
    // Days from 1970-01-01, by Howard Hinnant's civil-from-days inverse.
    let (y, m) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

/// `secs` as a UTC date and time: `2026-11-06 09:30`.
pub fn date_of(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}", rest / 3600, rest % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Journal;

    #[test]
    fn dates_read_as_unix_seconds() {
        assert_eq!(unix_of("1970-01-01"), Some(0));
        assert_eq!(unix_of("2026-11-06"), Some(1_793_923_200));
        assert_eq!(unix_of("2026-11-06T09:30"), Some(1_793_923_200 + 9 * 3600 + 30 * 60));
        assert_eq!(unix_of("2026-11-06T09:30:12.345Z"), Some(1_793_923_200 + 9 * 3600 + 30 * 60 + 12));
        assert_eq!(unix_of("2024-02-29"), Some(1_709_164_800), "a leap day");
        assert_eq!(unix_of("1793923200"), Some(1_793_923_200));
        assert_eq!(unix_of("2026-13-01"), None);
        assert_eq!(unix_of("soon"), None);
        assert_eq!(date_of(1_793_923_200 + 9 * 3600 + 30 * 60), "2026-11-06 09:30");
        assert_eq!(date_of(1_709_164_800), "2024-02-29 00:00");
        assert_eq!(date_of(0), "1970-01-01 00:00");
    }

    #[test]
    fn the_persons_own_words_are_told_from_equipes() {
        assert!(is_persons_own("Can you make the button blue?"));
        for not in ["<task-notification>…", "[equipe: the person took back…]", "Answers to your question: 1) …", "The person finished the step", ""] {
            assert!(!is_persons_own(not), "{not}");
        }
        assert_eq!(role_of("backend-2"), "backend");
        assert_eq!(role_of("qa"), "qa");
    }

    #[test]
    fn a_window_counts_first_time_acceptance_rework_and_the_person_stepping_in() {
        let home = std::env::temp_dir().join(format!("equipe-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let j = Journal::for_squad(&home, "acme").unwrap();
        let ticket = |title: &str, who: &str| {
            let t: crate::protocol::Ticket = serde_json::from_value(serde_json::json!({
                "id": 0, "title": title, "body": "", "status": "todo", "owner": who, "assignee": who, "project": null,
                "depends_on": [], "branch": null, "created_by": "you", "note": null,
            }))
            .unwrap();
            j.create_ticket(&t).unwrap()
        };
        let (a, b, c) = (ticket("A", "backend"), ticket("B", "frontend-2"), ticket("C", "backend"));
        let db = crate::journal::squad_dir(&home, "acme").join("squad.db");
        let conn = Connection::open(&db).unwrap();
        let event = |t: i64, seq: i64, at: i64, by: &str, kind: &str| {
            conn.execute("INSERT OR REPLACE INTO ticket_events (ticket, seq, at, by, kind, text) VALUES (?1, ?2, ?3, ?4, ?5, '')", rusqlite::params![t, seq, at, by, kind]).unwrap();
        };
        // A: handed in, accepted. B: handed in, reopened, handed in, accepted. C: handed in before the window.
        event(a as i64, 100, 1000, "backend", "handed in");
        event(a as i64, 101, 1100, "pm", "accepted by the PM");
        event(b as i64, 100, 1200, "frontend-2", "handed in");
        event(b as i64, 101, 1300, "you", "reopened");
        event(b as i64, 102, 1400, "frontend-2", "handed in");
        event(b as i64, 103, 1500, "you", "accepted");
        event(b as i64, 104, 1350, "you", "comment");
        event(c as i64, 100, 10, "backend", "handed in");
        conn.execute("INSERT INTO turns (member, role, ticket, began, ended, outcome, tokens) VALUES ('backend', 'backend', ?1, 990, 1000, 'completed', 5000)", [a as i64]).unwrap();
        conn.execute("INSERT INTO turns (member, role, ticket, began, ended, outcome, tokens) VALUES ('frontend-2', 'frontend', ?1, 1190, 1200, 'completed', 3000)", [b as i64]).unwrap();
        conn.execute("INSERT INTO turns (member, role, ticket, began, ended, outcome, tokens) VALUES ('frontend-2', 'frontend', ?1, 1390, 1400, 'completed', 1000)", [b as i64]).unwrap();
        drop(conn);
        let live = measure(&db, Window { from: 500, to: 2000 }, None, None).unwrap();
        assert_eq!((live.handed_in, live.accepted, live.first_time, live.reworked), (2, 2, 1, 1));
        assert_eq!(live.first_time_rate(), Some(0.5));
        assert_eq!((live.person_reopens, live.person_comments), (1, 1));
        assert_eq!(live.corrections_per_ticket(), Some(1.0));
        assert_eq!(live.tickets_worked, 2);
        let mut tokens = live.tokens_per_ticket.clone();
        tokens.sort();
        assert_eq!(tokens, [4000, 5000]);
        assert_eq!(live.by_role["frontend"], RoleLive { handed_in: 1, accepted: 1, first_time: 0, reworked: 1, tokens: 4000 });
        assert_eq!(measure(&db, Window { from: 0, to: 500 }, None, None).unwrap().handed_in, 1, "C, in its own window");
        let _ = std::fs::remove_dir_all(&home);
    }
}
