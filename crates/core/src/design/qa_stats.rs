//! What design QA costs and decides, read from a squad's journal without
//! writing to it (`equipe design qa-stats`, docs/DESIGN-QA.md): each design
//! QA ticket — the ones from before the precheck too, found by their record —
//! with its verdict, round, gate, and what its turns used (the `turns`
//! table), summed overall and by model tier, so a squad can compare before
//! and after.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension as _, params};
use serde::Serialize;

/// One design QA ticket, as its record says.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct QaRow {
    pub qa: u64,
    /// The ticket it checked.
    pub checked: Option<u64>,
    /// When it was made, in unix seconds.
    pub created: i64,
    /// `pass`, `changes`, `cancelled`, or `open`.
    pub verdict: String,
    /// Who gave it: a designer, `equipe` (the gate), `you`.
    pub verdict_by: Option<String>,
    pub round: u32,
    /// `off`, `shadow`, `on`; `None` for one from before the precheck.
    pub gate: Option<String>,
    pub decision: Option<String>,
    pub judgment: Option<String>,
    pub acted: bool,
    /// The tier its lane started at, as kept.
    pub tier: Option<String>,
    pub models: Vec<String>,
    pub efforts: Vec<String>,
    pub turns: u32,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// What turns kept only as a sum, from before tokens were kept by kind.
    pub tokens_only: u64,
    pub calls: u64,
    /// Its turns' time, in seconds.
    pub secs: u64,
    pub precheck_secs: Option<u64>,
}

impl QaRow {
    pub fn tokens(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write + self.tokens_only
    }

    /// The tier it ran at: as kept, else read from the model its turns named.
    pub fn tier_key(&self) -> String {
        if let Some(tier) = &self.tier {
            return tier.clone();
        }
        let family = |m: &str| ["opus", "sonnet", "haiku", "fable", "sol", "luna", "astra"].into_iter().find(|f| m.contains(f));
        match self.models.iter().find_map(|m| family(m)) {
            Some(f) => format!("({f})"),
            None if self.turns == 0 => "-".into(),
            None => "?".into(),
        }
    }
}

fn read_only(db: &Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| format!("{}: {e}", db.display()))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    Ok(conn)
}

fn has_table(conn: &Connection, name: &str) -> bool {
    conn.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [name], |_| Ok(())).optional().ok().flatten().is_some()
}

/// Every design QA ticket in the journal at `db` made at or after `since`
/// (unix seconds), oldest first — read only.
pub fn read(db: &Path, since: i64) -> Result<Vec<QaRow>, String> {
    let conn = read_only(db)?;
    let e = |e: rusqlite::Error| e.to_string();
    // Every one, for the rounds; those since are kept.
    let mut stmt = conn
        .prepare(
            "SELECT t.id, t.status, t.created_at, e.text FROM tickets t
             JOIN ticket_events e ON e.ticket = t.id AND e.kind = 'created' AND e.by = 'equipe' AND e.text LIKE 'design QA of T-%'
             ORDER BY t.id",
        )
        .map_err(e)?;
    let all: Vec<(u64, String, i64, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(e)?.filter_map(Result::ok).collect();
    let runs = has_table(&conn, "design_qa_runs");
    let mut seen: BTreeMap<u64, u32> = BTreeMap::new();
    let mut out = Vec::new();
    for (qa, status, created, text) in all {
        let checked = text.strip_prefix("design QA of T-").and_then(|n| n.trim().parse::<u64>().ok());
        let round = checked.map_or(1, |c| {
            let n = seen.entry(c).or_default();
            *n += 1;
            *n
        });
        if created < since {
            continue;
        }
        let mut row = QaRow { qa, checked, created, round, ..QaRow::default() };
        let verdict: Option<String> = conn
            .query_row(
                "SELECT kind FROM ticket_events WHERE ticket = ?1 AND kind IN ('design QA passed', 'design QA: changes') ORDER BY seq DESC LIMIT 1",
                [qa],
                |r| r.get(0),
            )
            .optional()
            .map_err(e)?;
        row.verdict = match (verdict.as_deref(), status.as_str()) {
            (Some("design QA passed"), _) => "pass",
            (Some(_), _) => "changes",
            (None, "cancelled") => "cancelled",
            _ => "open",
        }
        .to_owned();
        if runs {
            conn.query_row(
                "SELECT round, gate, decision, judgment, acted, verdict, verdict_by, precheck_secs FROM design_qa_runs WHERE qa = ?1",
                [qa],
                |r| {
                    row.round = r.get::<_, u32>(0)?.max(1);
                    row.gate = Some(r.get(1)?);
                    row.decision = r.get(2)?;
                    row.judgment = r.get(3)?;
                    row.acted = r.get(4)?;
                    if let Some(v) = r.get::<_, Option<String>>(5)? {
                        row.verdict = v;
                    }
                    row.verdict_by = r.get(6)?;
                    row.precheck_secs = r.get::<_, Option<i64>>(7)?.map(|s| s.max(0) as u64);
                    Ok(())
                },
            )
            .optional()
            .map_err(e)?;
        }
        if has_table(&conn, "model_choices") {
            row.tier = conn
                .query_row("SELECT tier FROM model_choices WHERE ticket = ?1 AND what = 'lane' ORDER BY at DESC, rowid DESC LIMIT 1", [qa], |r| r.get::<_, Option<String>>(0))
                .optional()
                .map_err(e)?
                .flatten();
        }
        let used = conn
            .query_row(
                "SELECT COUNT(*), SUM(COALESCE(input, 0)), SUM(COALESCE(output, 0)), SUM(COALESCE(cache_read, 0)), SUM(COALESCE(cache_write, 0)),
                        SUM(CASE WHEN input IS NULL AND output IS NULL THEN tokens ELSE 0 END), SUM(COALESCE(calls, 0)), SUM(MAX(ended - began, 0)),
                        GROUP_CONCAT(DISTINCT model), GROUP_CONCAT(DISTINCT effort)
                 FROM turns WHERE ticket = ?1",
                params![qa],
                |r| {
                    let n = |i: usize| r.get::<_, Option<i64>>(i).map(|v| v.unwrap_or(0).max(0) as u64);
                    let list = |i: usize| r.get::<_, Option<String>>(i).map(|v| v.map(|s| s.split(',').map(str::to_owned).collect::<Vec<_>>()).unwrap_or_default());
                    Ok((r.get::<_, u32>(0)?, n(1)?, n(2)?, n(3)?, n(4)?, n(5)?, n(6)?, n(7)?, list(8)?, list(9)?))
                },
            )
            .map_err(e)?;
        (row.turns, row.input, row.output, row.cache_read, row.cache_write, row.tokens_only, row.calls, row.secs, row.models, row.efforts) = used;
        out.push(row);
    }
    Ok(out)
}

/// What a replay needs of the ticket a design QA checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checked {
    pub project: Option<std::path::PathBuf>,
    pub title: String,
    pub body: String,
}

/// Ticket `id` of the journal at `db`, read only.
pub fn checked(db: &Path, id: u64) -> Result<Option<Checked>, String> {
    let conn = read_only(db)?;
    conn.query_row("SELECT project, title, body FROM tickets WHERE id = ?1", [id], |r| {
        Ok(Checked { project: r.get::<_, Option<String>>(0)?.map(std::path::PathBuf::from), title: r.get(1)?, body: r.get(2)? })
    })
    .optional()
    .map_err(|e| e.to_string())
}

/// The commits design QA ticket `qa`'s precheck looked at — its head and
/// base — when it was kept; `(None, None)` for one from before.
pub fn commits(db: &Path, qa: u64) -> (Option<String>, Option<String>) {
    let Ok(conn) = read_only(db) else { return (None, None) };
    if !has_table(&conn, "design_qa_runs") {
        return (None, None);
    }
    conn.query_row("SELECT head, base FROM design_qa_runs WHERE qa = ?1", [qa], |r| Ok((r.get(0)?, r.get(1)?))).optional().ok().flatten().unwrap_or((None, None))
}

/// Sums over some design QA tickets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Sums {
    pub tickets: u32,
    pub passed: u32,
    pub changes: u32,
    pub cancelled: u32,
    pub open: u32,
    pub turns: u32,
    pub tokens: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub calls: u64,
    pub secs: u64,
}

impl Sums {
    fn add(&mut self, r: &QaRow) {
        self.tickets += 1;
        match r.verdict.as_str() {
            "pass" => self.passed += 1,
            "changes" => self.changes += 1,
            "cancelled" => self.cancelled += 1,
            _ => self.open += 1,
        }
        self.turns += r.turns;
        self.tokens += r.tokens();
        self.input += r.input;
        self.output += r.output;
        self.cache_read += r.cache_read;
        self.cache_write += r.cache_write;
        self.calls += r.calls;
        self.secs += r.secs;
    }

    /// Passed of those with a verdict, in percent; `None` with none.
    pub fn pass_rate(&self) -> Option<f64> {
        let judged = self.passed + self.changes;
        (judged > 0).then(|| f64::from(self.passed) * 100. / f64::from(judged))
    }

    /// Tokens per ticket.
    pub fn per_ticket(&self) -> u64 {
        if self.tickets == 0 { 0 } else { self.tokens / u64::from(self.tickets) }
    }
}

/// How the gate's call compares with the designer's verdict, where both are known.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Agreement {
    /// Would pass, and the designer passed.
    pub pass_agreed: u32,
    /// Would pass, and the designer sent it back: the risky miss.
    pub pass_missed: u32,
    /// Would send back, and the designer sent it back.
    pub back_agreed: u32,
    /// Would send back, and the designer passed: a needless send-back.
    pub back_wrong: u32,
    /// Left to the designer: nothing to compare.
    pub deferred: u32,
    /// The gate acted itself (on): passed or sent back without a designer.
    pub acted_pass: u32,
    pub acted_back: u32,
}

impl Agreement {
    /// Of the calls a designer's verdict can check, how many it agreed with, in percent.
    pub fn rate(&self) -> Option<f64> {
        let all = self.pass_agreed + self.pass_missed + self.back_agreed + self.back_wrong;
        (all > 0).then(|| f64::from(self.pass_agreed + self.back_agreed) * 100. / f64::from(all))
    }
}

/// Everything `equipe design qa-stats` says.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Stats {
    pub all: Sums,
    /// By the tier each ran at.
    pub by_tier: BTreeMap<String, Sums>,
    /// By gate mode (`before` for those from before the precheck).
    pub by_gate: BTreeMap<String, Sums>,
    pub agreement: Agreement,
    /// Rounds: how many were a ticket's first design QA, second, third or more.
    pub rounds: BTreeMap<u32, u32>,
}

pub fn summarize(rows: &[QaRow]) -> Stats {
    let mut s = Stats::default();
    for r in rows {
        s.all.add(r);
        s.by_tier.entry(r.tier_key()).or_default().add(r);
        s.by_gate.entry(r.gate.clone().unwrap_or_else(|| "before".into())).or_default().add(r);
        *s.rounds.entry(r.round.min(3)).or_default() += 1;
        let a = &mut s.agreement;
        match (r.decision.as_deref(), r.acted, r.verdict.as_str()) {
            (Some("auto_pass"), true, _) => a.acted_pass += 1,
            (Some("send_back"), true, _) => a.acted_back += 1,
            (Some("auto_pass"), false, "pass") => a.pass_agreed += 1,
            (Some("auto_pass"), false, "changes") => a.pass_missed += 1,
            (Some("send_back"), false, "changes") => a.back_agreed += 1,
            (Some("send_back"), false, "pass") => a.back_wrong += 1,
            (Some("designer"), _, _) => a.deferred += 1,
            _ => {}
        }
    }
    s
}

/// `7d`, `24h`, `30m`, or a day (`2026-11-01`), as unix seconds back from
/// `now`; `None` for what isn't one.
pub fn since(text: &str, now: i64) -> Option<i64> {
    let text = text.trim();
    let unit = |u: char| match u {
        'd' => Some(86_400),
        'h' => Some(3_600),
        'm' => Some(60),
        'w' => Some(7 * 86_400),
        _ => None,
    };
    if let Some(last) = text.chars().last()
        && let Some(secs) = unit(last)
        && let Ok(n) = text[..text.len() - 1].parse::<i64>()
    {
        return Some(now - n * secs);
    }
    // A calendar day, from its midnight UTC.
    let mut parts = text.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (Some(Some(y)), Some(Some(m)), Some(Some(d))) = (parts.next(), parts.next(), parts.next()) else { return None };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_span_or_a_day_reads_as_where_the_stats_start() {
        assert_eq!(since("7d", 1_000_000), Some(1_000_000 - 7 * 86_400));
        assert_eq!(since("24h", 100_000), Some(100_000 - 86_400));
        assert_eq!(since("2026-11-01", 0), Some(1_793_491_200));
        assert_eq!(since("1970-01-02", 0), Some(86_400));
        assert_eq!(since("soon", 0), None);
        assert_eq!(since("2026-13-01", 0), None);
    }

    fn row(qa: u64, verdict: &str, gate: Option<&str>, decision: Option<&str>, acted: bool, tier: &str, tokens: u64) -> QaRow {
        QaRow { qa, verdict: verdict.into(), gate: gate.map(str::to_owned), decision: decision.map(str::to_owned), acted, tier: Some(tier.into()), turns: 1, cache_read: tokens, round: 1, ..QaRow::default() }
    }

    #[test]
    fn stats_count_pass_rate_agreement_and_tokens_by_tier() {
        let rows = vec![
            row(1, "pass", None, None, false, "deep", 9_000),
            row(2, "changes", None, None, false, "deep", 7_000),
            row(3, "pass", Some("shadow"), Some("auto_pass"), false, "standard", 1_000),
            row(4, "changes", Some("shadow"), Some("auto_pass"), false, "standard", 2_000),
            row(5, "changes", Some("shadow"), Some("send_back"), false, "deep", 3_000),
            row(6, "pass", Some("shadow"), Some("designer"), false, "standard", 1_000),
            row(7, "pass", Some("on"), Some("auto_pass"), true, "-", 0),
        ];
        let s = summarize(&rows);
        assert_eq!((s.all.tickets, s.all.passed, s.all.changes), (7, 4, 3));
        assert!((s.all.pass_rate().unwrap() - 400. / 7.).abs() < 1e-9);
        assert_eq!(s.by_tier["deep"].tokens, 19_000);
        assert_eq!(s.by_tier["standard"].per_ticket(), 4_000 / 3);
        assert_eq!(s.by_gate["before"].tickets, 2);
        let a = &s.agreement;
        assert_eq!((a.pass_agreed, a.pass_missed, a.back_agreed, a.back_wrong, a.deferred, a.acted_pass), (1, 1, 1, 0, 1, 1));
        assert!((a.rate().unwrap() - 200. / 3.).abs() < 1e-9);
        assert_eq!(QaRow { models: vec!["claude-sonnet-5".into()], turns: 2, ..QaRow::default() }.tier_key(), "(sonnet)");
    }

    #[test]
    fn the_journal_is_read_without_writing_with_old_and_new_design_qa_tickets() {
        let home = std::env::temp_dir().join(format!("equipe-qa-stats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let j = crate::journal::Journal::for_squad(&home, "acme").unwrap();
        let ticket = |title: &str| equipe_protocol::Ticket {
            id: 0,
            title: title.into(),
            body: String::new(),
            status: equipe_protocol::TicketStatus::Todo,
            kind: equipe_protocol::TicketKind::Task,
            owner: Some("designer".into()),
            assignee: None,
            project: None,
            depends_on: vec![],
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            design_qa: None,
            plan: None,
            model: None,
            created_by: "equipe".into(),
            note: None,
            asks: vec![],
            remaining: vec![],
            images: vec![],
            updated_at: None,
            priority: Default::default(),
            rank: 0,
        };
        let checked = j.create_ticket(&ticket("Add a page")).unwrap();
        // One from before the precheck, sent back; then one with a run, passed.
        let old = j.create_ticket(&ticket("Design QA: old")).unwrap();
        j.ticket_event(old, "equipe", "created", &format!("design QA of T-{checked}")).unwrap();
        j.ticket_event(old, "designer", "design QA: changes", "Title cut off").unwrap();
        let new = j.create_ticket(&ticket("Design QA: new")).unwrap();
        j.ticket_event(new, "equipe", "created", &format!("design QA of T-{checked}")).unwrap();
        assert_eq!(j.design_qa_rounds(checked).unwrap(), 2);
        j.start_design_qa_run(new, checked, 2, "shadow", "running").unwrap();
        j.set_design_qa_precheck(&crate::journal::DesignQaRun { qa: new, precheck: "done".into(), decision: Some("auto_pass".into()), ..Default::default() }).unwrap();
        j.set_design_qa_verdict(new, "pass", "designer").unwrap();
        j.set_design_qa_verdict(new, "changes", "you").unwrap();
        assert_eq!(j.design_qa_run(new).unwrap().verdict.as_deref(), Some("pass"), "the first verdict stays");
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let used = crate::journal::TurnUse::of(equipe_protocol::Engine::Claude, Some("claude-sonnet-5".into()), None, &equipe_protocol::Usage { input_tokens: Some(10), output_tokens: Some(20), cache_read_tokens: Some(300), cache_write_tokens: Some(40), calls: Some(5), ..Default::default() });
        let turn = crate::retro::Turn { member: "designer".into(), role: "designer".into(), ticket: Some(new), began: now as u64 - 60, ended: now as u64, outcome: "completed".into(), tokens: 370 };
        j.record_turn_use(&turn, Some(&used)).unwrap();
        drop(j);
        let db = crate::journal::squad_dir(&home, "acme").join("squad.db");
        let rows = read(&db, 0).unwrap();
        assert_eq!(rows.iter().map(|r| (r.qa, r.verdict.as_str(), r.round)).collect::<Vec<_>>(), [(u64::from(old), "changes", 1), (u64::from(new), "pass", 2)]);
        let r = &rows[1];
        assert_eq!((r.gate.as_deref(), r.decision.as_deref(), r.tokens(), r.calls, r.secs, r.turns), (Some("shadow"), Some("auto_pass"), 370, 5, 60, 1));
        assert!(read(&db, now + 10).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(home);
    }
}
