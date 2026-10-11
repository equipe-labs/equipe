use std::ops::Range;
use std::path::Path;

use rusqlite::{Connection, params};

pub use rusqlite::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    Text,
    Tool,
    DiffAdd,
    DiffDel,
}

impl LogKind {
    fn to_i64(self) -> i64 {
        match self {
            LogKind::Text => 0,
            LogKind::Tool => 1,
            LogKind::DiffAdd => 2,
            LogKind::DiffDel => 3,
        }
    }

    fn from_i64(v: i64) -> Self {
        match v {
            1 => LogKind::Tool,
            2 => LogKind::DiffAdd,
            3 => LogKind::DiffDel,
            _ => LogKind::Text,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub seq: u64,
    pub kind: LogKind,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TicketStatus {
    Todo,
    InProgress,
    Review,
    Done,
}

impl TicketStatus {
    pub fn label(self) -> &'static str {
        match self {
            TicketStatus::Todo => "todo",
            TicketStatus::InProgress => "in progress",
            TicketStatus::Review => "review",
            TicketStatus::Done => "done",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "in_progress" => TicketStatus::InProgress,
            "review" => TicketStatus::Review,
            "done" => TicketStatus::Done,
            _ => TicketStatus::Todo,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Ticket {
    pub id: i64,
    pub title: String,
    pub role: String,
    pub status: TicketStatus,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let conn = Connection::open(path)?;
        // WAL lets the agent writers and the UI reader run without blocking each
        // other; a small page cache keeps each connection's footprint bounded.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -2000;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS log_lines (
               agent TEXT NOT NULL,
               seq   INTEGER NOT NULL,
               kind  INTEGER NOT NULL,
               text  TEXT NOT NULL,
               PRIMARY KEY (agent, seq)
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS tickets (
               id     INTEGER PRIMARY KEY,
               title  TEXT NOT NULL,
               role   TEXT NOT NULL,
               status TEXT NOT NULL
             );",
        )?;
        Ok(Self { conn })
    }

    /// Next free `seq` for an agent (0 for a fresh agent).
    pub fn next_seq(&self, agent: &str) -> Result<u64, Error> {
        self.conn.query_row(
            "SELECT COALESCE(MAX(seq) + 1, 0) FROM log_lines WHERE agent = ?1",
            [agent],
            |r| r.get::<_, i64>(0).map(|v| v as u64),
        )
    }

    /// Append lines in one transaction, numbering them from `start_seq`.
    pub fn append(
        &mut self,
        agent: &str,
        start_seq: u64,
        lines: &[(LogKind, String)],
    ) -> Result<(), Error> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO log_lines (agent, seq, kind, text) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (i, (kind, text)) in lines.iter().enumerate() {
                stmt.execute(params![agent, (start_seq + i as u64) as i64, kind.to_i64(), text])?;
            }
        }
        tx.commit()
    }

    /// The rows for `range` of an agent's log: what a virtual list draws.
    pub fn lines(&self, agent: &str, range: Range<u64>) -> Result<Vec<LogLine>, Error> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT seq, kind, text FROM log_lines
             WHERE agent = ?1 AND seq >= ?2 AND seq < ?3 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![agent, range.start as i64, range.end as i64], |r| {
            Ok(LogLine {
                seq: r.get::<_, i64>(0)? as u64,
                kind: LogKind::from_i64(r.get(1)?),
                text: r.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn tickets(&self) -> Result<Vec<Ticket>, Error> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, title, role, status FROM tickets ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Ticket {
                id: r.get(0)?,
                title: r.get(1)?,
                role: r.get(2)?,
                status: TicketStatus::from_str(&r.get::<_, String>(3)?),
            })
        })?;
        rows.collect()
    }

    pub fn seed_tickets_if_empty(&mut self) -> Result<(), Error> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM tickets", [], |r| r.get(0))?;
        if n > 0 {
            return Ok(());
        }
        let seed = [
            ("Write spec for login flow", "po", "done"),
            ("Design board empty state", "designer", "review"),
            ("Board view: virtualized ticket list", "frontend", "in_progress"),
            ("ACP client: session/update stream", "backend", "in_progress"),
            ("SQLite log store with paging", "backend", "in_progress"),
            ("Memory budget check in CI", "devops", "todo"),
            ("E2E: three agents streaming", "qa", "todo"),
            ("Korean IME regression tests", "qa", "todo"),
        ];
        let tx = self.conn.transaction()?;
        for (title, role, status) in seed {
            tx.execute(
                "INSERT INTO tickets (title, role, status) VALUES (?1, ?2, ?3)",
                params![title, role, status],
            )?;
        }
        tx.commit()
    }
}
