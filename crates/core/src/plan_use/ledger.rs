//! What the engines' transcripts on this machine say was used, hour by hour:
//! `<home>/plan-use.db`. Only numbers are kept — tokens by kind and requests,
//! by hour, engine, model, the engine's session and the folder it ran in —
//! never a word of what was said, and nothing leaves the machine. Each file
//! read is remembered by how far it was read, so it is never read twice.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Result, params};

use crate::protocol::plan_use::Tokens;

/// How long the hours are kept: the week a plan counts, and some.
pub const KEPT: i64 = 10 * 24 * 60 * 60;

pub struct Ledger {
    conn: Connection,
}

/// One hour of one session's use of one model.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    /// Its hour's start (Unix seconds).
    pub hour: i64,
    /// `claude` or `codex`.
    pub engine: String,
    pub model: String,
    /// The engine's session (Claude Code's, Codex's root thread).
    pub session: String,
    /// The folder it ran in.
    pub cwd: String,
    /// `equipe` when the engine says equipe ran it (Codex's originator).
    pub origin: String,
}

/// What an hour of it used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Used {
    pub tokens: Tokens,
    /// Of `tokens.cache_write`, written to be kept an hour.
    pub long: u64,
    pub calls: u64,
}

impl Used {
    pub fn add(&mut self, other: Used) {
        self.tokens.add(other.tokens);
        self.long += other.long;
        self.calls += other.calls;
    }
}

impl Ledger {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS cursors (path TEXT PRIMARY KEY, offset INTEGER NOT NULL, state TEXT);
             CREATE TABLE IF NOT EXISTS hours (
               hour INTEGER NOT NULL,
               engine TEXT NOT NULL,
               model TEXT NOT NULL,
               session TEXT NOT NULL,
               cwd TEXT NOT NULL,
               origin TEXT NOT NULL DEFAULT '',
               input INTEGER NOT NULL DEFAULT 0,
               output INTEGER NOT NULL DEFAULT 0,
               cache_write INTEGER NOT NULL DEFAULT 0,
               cache_write_long INTEGER NOT NULL DEFAULT 0,
               cache_read INTEGER NOT NULL DEFAULT 0,
               calls INTEGER NOT NULL DEFAULT 0,
               PRIMARY KEY (hour, engine, model, session, cwd, origin)
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS seen (key TEXT PRIMARY KEY, output INTEGER NOT NULL, at INTEGER NOT NULL) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS seen_at ON seen(at);",
        )?;
        Ok(Self { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// How far `path` was read, and what its reading carried over.
    pub fn cursor(&self, path: &str) -> Result<Option<(u64, Option<String>)>> {
        self.conn
            .query_row("SELECT offset, state FROM cursors WHERE path = ?1", [path], |r| Ok((r.get::<_, i64>(0)?.max(0) as u64, r.get(1)?)))
            .optional()
    }

    pub fn set_cursor(&self, path: &str, offset: u64, state: Option<&str>) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO cursors (path, offset, state) VALUES (?1, ?2, ?3)", params![path, offset as i64, state])?;
        Ok(())
    }

    /// A request counted once, however often it is written: `None` when it
    /// is new (now kept), else the output counted for it so far.
    pub fn seen(&self, key: &str, output: u64, at: i64) -> Result<Option<u64>> {
        let had: Option<i64> = self.conn.query_row("SELECT output FROM seen WHERE key = ?1", [key], |r| r.get(0)).optional()?;
        match had {
            None => {
                self.conn.execute("INSERT INTO seen (key, output, at) VALUES (?1, ?2, ?3)", params![key, output as i64, at])?;
                Ok(None)
            }
            Some(before) => {
                if (before.max(0) as u64) < output {
                    self.conn.execute("UPDATE seen SET output = ?2 WHERE key = ?1", params![key, output as i64])?;
                }
                Ok(Some(before.max(0) as u64))
            }
        }
    }

    pub fn add(&self, key: &Key, used: Used) -> Result<()> {
        self.conn.execute(
            "INSERT INTO hours (hour, engine, model, session, cwd, origin, input, output, cache_write, cache_write_long, cache_read, calls)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT (hour, engine, model, session, cwd, origin) DO UPDATE SET
               input = input + excluded.input, output = output + excluded.output,
               cache_write = cache_write + excluded.cache_write, cache_write_long = cache_write_long + excluded.cache_write_long,
               cache_read = cache_read + excluded.cache_read, calls = calls + excluded.calls",
            params![
                key.hour,
                key.engine,
                key.model,
                key.session,
                key.cwd,
                key.origin,
                used.tokens.input as i64,
                used.tokens.output as i64,
                used.tokens.cache_write as i64,
                used.long as i64,
                used.tokens.cache_read as i64,
                used.calls as i64
            ],
        )?;
        Ok(())
    }

    /// `engine`'s hours from `since`, oldest first.
    pub fn hours(&self, engine: &str, since: i64) -> Result<Vec<(Key, Used)>> {
        let mut stmt = self.conn.prepare(
            "SELECT hour, engine, model, session, cwd, origin, input, output, cache_write, cache_write_long, cache_read, calls
             FROM hours WHERE engine = ?1 AND hour >= ?2 ORDER BY hour",
        )?;
        let n = |r: &rusqlite::Row, i: usize| r.get::<_, i64>(i).map(|v| v.max(0) as u64);
        let rows = stmt.query_map(params![engine, since], |r| {
            Ok((
                Key { hour: r.get(0)?, engine: r.get(1)?, model: r.get(2)?, session: r.get(3)?, cwd: r.get(4)?, origin: r.get(5)? },
                Used { tokens: Tokens { input: n(r, 6)?, output: n(r, 7)?, cache_write: n(r, 8)?, cache_read: n(r, 10)? }, long: n(r, 9)?, calls: n(r, 11)? },
            ))
        })?;
        rows.collect()
    }

    pub fn meta(&self, key: &str) -> Option<String> {
        self.conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)", [key, value])?;
        Ok(())
    }

    /// Let go of what is older than [`KEPT`].
    pub fn prune(&self, now: i64) -> Result<()> {
        self.conn.execute("DELETE FROM hours WHERE hour < ?1", [now - KEPT])?;
        self.conn.execute("DELETE FROM seen WHERE at < ?1", [now - KEPT])?;
        Ok(())
    }

    pub fn begin(&self) -> Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")
    }

    pub fn commit(&self) -> Result<()> {
        self.conn.execute_batch("COMMIT")
    }
}
