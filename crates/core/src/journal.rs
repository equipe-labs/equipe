//! What happened, kept on disk: one SQLite database per project.
//!
//! The journal holds the *record* (sessions: engine, branch, title, outcome,
//! tokens) and the *conversation* (messages, finished replies, one line per tool
//! use). It never holds raw material — file contents, command output, images,
//! thinking, diffs: git has the changes and each engine keeps its own transcript.
//!
//! Conversations are kept for a window and then pruned; the session record stays.
//! The database is created in incremental-vacuum mode, so pruning gives the
//! space back to the disk instead of leaving the file at its high-water mark.

use std::path::{Path, PathBuf};

use equipe_protocol::{Access, Engine, Entry, Member, SessionId};
use rusqlite::{Connection, OptionalExtension as _, params};

pub use rusqlite::Error;
pub type Result<T> = std::result::Result<T, Error>;

/// How long conversations are kept before they are pruned to the record.
pub const DEFAULT_RETENTION_DAYS: u64 = 90;

/// A session as the journal keeps it — enough to resume it after a restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: SessionId,
    /// The role it plays on the team, if any.
    pub member: Option<Member>,
    pub engine: Engine,
    pub model: Option<String>,
    pub access: Access,
    /// Where turns run: the worktree for an editing session.
    pub cwd: PathBuf,
    /// For an editing session: the repository its branch belongs to.
    pub repo: Option<PathBuf>,
    pub branch: Option<String>,
    pub base: Option<String>,
    /// The engine's own conversation id (Claude Code `--resume`).
    pub engine_session: Option<String>,
    pub title: Option<String>,
    pub tokens: u64,
    pub open: bool,
}

/// Sizes and counts, for `equipe storage`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub bytes: u64,
    pub sessions: u64,
    pub open_sessions: u64,
    pub entries: u64,
    /// Unix seconds of the oldest kept conversation line.
    pub oldest_entry: Option<i64>,
}

pub struct Journal {
    conn: Connection,
    path: PathBuf,
}

/// `<home>/projects/<name>-<hash>`: one directory per project.
pub fn project_dir(home: &Path, project: &Path) -> PathBuf {
    // FNV-1a over the path: stable across runs and Rust versions.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in project.to_string_lossy().bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let name = project
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "root".into());
    home.join("projects").join(format!("{name}-{:08x}", hash as u32))
}

/// Where a member keeps what it learns about a project. Tied to the member's
/// name, so a `frontend` closed and opened again remembers.
pub fn member_memory_dir(home: &Path, project: &Path, name: &str) -> PathBuf {
    project_dir(home, project).join("members").join(name).join("memory")
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn engine_str(engine: Engine) -> &'static str {
    match engine {
        Engine::Codex => "codex",
        Engine::Claude => "claude",
    }
}

fn engine_from(s: &str) -> Engine {
    if s == "codex" { Engine::Codex } else { Engine::Claude }
}

fn access_str(access: Access) -> &'static str {
    match access {
        Access::ReadOnly => "read_only",
        Access::EditFiles => "edit_files",
    }
}

impl Journal {
    /// Open (or create) the journal for `project` under `home`.
    pub fn for_project(home: &Path, project: &Path) -> Result<Self> {
        let dir = project_dir(home, project);
        std::fs::create_dir_all(&dir).map_err(|e| Error::ToSqlConversionFailure(Box::new(e)))?;
        let journal = Self::open(&dir.join("journal.db"))?;
        journal
            .conn
            .execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('project', ?1)", [project.to_string_lossy()])?;
        Ok(journal)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        // Must precede the first table to take effect on a new database.
        conn.execute_batch(
            "PRAGMA auto_vacuum = INCREMENTAL;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS sessions (
               id INTEGER PRIMARY KEY,
               engine TEXT NOT NULL,
               model TEXT,
               access TEXT NOT NULL,
               cwd TEXT NOT NULL,
               repo TEXT,
               branch TEXT,
               base TEXT,
               engine_session TEXT,
               title TEXT,
               outcome TEXT,
               tokens INTEGER NOT NULL DEFAULT 0,
               open INTEGER NOT NULL DEFAULT 1,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS entries (
               session INTEGER NOT NULL,
               seq INTEGER NOT NULL,
               at INTEGER NOT NULL,
               body TEXT NOT NULL,
               PRIMARY KEY (session, seq)
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS entries_at ON entries(at);",
        )?;
        // Columns added after the first release; a journal that has them already
        // refuses the ALTER, which is fine.
        for column in ["role TEXT", "name TEXT", "joined INTEGER"] {
            let _ = conn.execute(&format!("ALTER TABLE sessions ADD COLUMN {column}"), []);
        }
        Ok(Self { conn, path: path.to_owned() })
    }

    /// The project path this journal belongs to.
    pub fn project(&self) -> Option<PathBuf> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = 'project'", [], |r| r.get::<_, String>(0))
            .optional()
            .ok()
            .flatten()
            .map(PathBuf::from)
    }

    pub fn record_session(&self, r: &SessionRecord) -> Result<()> {
        let t = now();
        self.conn.execute(
            "INSERT OR REPLACE INTO sessions
               (id, engine, model, access, cwd, repo, branch, base, engine_session, title, tokens, open, created_at, updated_at,
                role, name, joined)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13, ?14, ?15,
                     (SELECT COALESCE(MAX(joined), 0) + 1 FROM sessions))",
            params![
                r.id as i64,
                engine_str(r.engine),
                r.model,
                access_str(r.access),
                r.cwd.to_string_lossy(),
                r.repo.as_ref().map(|p| p.to_string_lossy().into_owned()),
                r.branch,
                r.base,
                r.engine_session,
                r.title,
                r.tokens as i64,
                r.open,
                t,
                r.member.as_ref().map(|m| m.role.clone()),
                r.member.as_ref().map(|m| m.name.clone()),
            ],
        )?;
        Ok(())
    }

    fn touch(&self, id: SessionId, set: &str, value: impl rusqlite::ToSql) -> Result<()> {
        self.conn.execute(
            &format!("UPDATE sessions SET {set} = ?1, updated_at = ?2 WHERE id = ?3"),
            params![value, now(), id as i64],
        )?;
        Ok(())
    }

    pub fn set_model(&self, id: SessionId, model: &str) -> Result<()> {
        self.touch(id, "model", model)
    }

    pub fn set_engine_session(&self, id: SessionId, engine_session: &str) -> Result<()> {
        self.touch(id, "engine_session", engine_session)
    }

    /// The first message names the session; later ones don't.
    pub fn set_title_if_empty(&self, id: SessionId, title: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET title = ?1 WHERE id = ?2 AND title IS NULL",
            params![title, id as i64],
        )?;
        Ok(())
    }

    pub fn add_tokens(&self, id: SessionId, tokens: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET tokens = tokens + ?1, updated_at = ?2 WHERE id = ?3",
            params![tokens as i64, now(), id as i64],
        )?;
        Ok(())
    }

    pub fn set_outcome(&self, id: SessionId, outcome: &str) -> Result<()> {
        self.touch(id, "outcome", outcome)
    }

    pub fn close_session(&self, id: SessionId) -> Result<()> {
        self.touch(id, "open", false)
    }

    /// Append one conversation line.
    pub fn append(&self, id: SessionId, entry: &Entry) -> Result<()> {
        let body = serde_json::to_string(entry).map_err(|e| Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn.execute(
            "INSERT INTO entries (session, seq, at, body)
             VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM entries WHERE session = ?1), ?2, ?3)",
            params![id as i64, now(), body],
        )?;
        Ok(())
    }

    fn entries(&self, id: SessionId, limit: Option<u64>) -> Result<Vec<Entry>> {
        let mut stmt = self.conn.prepare(
            "SELECT body FROM (SELECT seq, body FROM entries WHERE session = ?1 ORDER BY seq DESC LIMIT ?2)
             ORDER BY seq ASC",
        )?;
        let limit = limit.map_or(-1, |l| l as i64);
        let rows = stmt.query_map(params![id as i64, limit], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|b| b.ok().and_then(|b| serde_json::from_str(&b).ok())).collect())
    }

    /// The last `limit` lines of a session's conversation, oldest first.
    pub fn recent(&self, id: SessionId, limit: u64) -> Result<Vec<Entry>> {
        self.entries(id, Some(limit))
    }

    /// Every kept line of a session's conversation, oldest first.
    pub fn conversation(&self, id: SessionId) -> Result<Vec<Entry>> {
        self.entries(id, None)
    }

    pub fn open_sessions(&self) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, engine, model, access, cwd, repo, branch, base, engine_session, title, tokens, open, role, name
             FROM sessions WHERE open = 1 ORDER BY COALESCE(joined, 0), created_at, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SessionRecord {
                id: r.get::<_, i64>(0)? as u64,
                engine: engine_from(&r.get::<_, String>(1)?),
                model: r.get(2)?,
                access: if r.get::<_, String>(3)? == "edit_files" { Access::EditFiles } else { Access::ReadOnly },
                cwd: PathBuf::from(r.get::<_, String>(4)?),
                repo: r.get::<_, Option<String>>(5)?.map(PathBuf::from),
                branch: r.get(6)?,
                base: r.get(7)?,
                engine_session: r.get(8)?,
                title: r.get(9)?,
                tokens: r.get::<_, i64>(10)? as u64,
                open: r.get(11)?,
                member: match (r.get::<_, Option<String>>(12)?, r.get::<_, Option<String>>(13)?) {
                    (Some(role), Some(name)) => Some(Member { role, name }),
                    _ => None,
                },
            })
        })?;
        rows.collect()
    }

    /// Drop conversation lines older than `cutoff` (unix seconds) and give the
    /// space back. Session records are kept. Returns the lines removed.
    pub fn prune(&self, cutoff: i64) -> Result<usize> {
        let removed = self.conn.execute("DELETE FROM entries WHERE at < ?1", [cutoff])?;
        if removed > 0 {
            self.give_back_space()?;
        }
        Ok(removed)
    }

    /// Return free pages to the disk. `incremental_vacuum` frees pages as its rows
    /// are stepped, so it has to be read to the end; then the WAL is folded into
    /// the main file so the truncation actually lands.
    fn give_back_space(&self) -> Result<()> {
        let mut vacuum = self.conn.prepare("PRAGMA incremental_vacuum")?;
        let mut rows = vacuum.query([])?;
        while rows.next()?.is_some() {}
        drop(rows);
        drop(vacuum);
        self.checkpoint()
    }

    fn checkpoint(&self) -> Result<()> {
        self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
    }

    pub fn stats(&self) -> Result<Stats> {
        let count = |sql: &str| self.conn.query_row(sql, [], |r| r.get::<_, i64>(0)).map(|n| n as u64);
        let file = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let wal = PathBuf::from(format!("{}-wal", self.path.display()));
        Ok(Stats {
            bytes: file(&self.path) + file(&wal),
            sessions: count("SELECT COUNT(*) FROM sessions")?,
            open_sessions: count("SELECT COUNT(*) FROM sessions WHERE open = 1")?,
            entries: count("SELECT COUNT(*) FROM entries")?,
            oldest_entry: self.conn.query_row("SELECT MIN(at) FROM entries", [], |r| r.get(0))?,
        })
    }

    #[cfg(test)]
    fn backdate_entries(&self, id: SessionId, at: i64) {
        self.conn.execute("UPDATE entries SET at = ?1 WHERE session = ?2", params![at, id as i64]).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temp_home() -> PathBuf {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("equipe-journal-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(id: SessionId) -> SessionRecord {
        SessionRecord {
            id,
            member: None,
            engine: Engine::Claude,
            model: None,
            access: Access::EditFiles,
            cwd: "/w/1".into(),
            repo: Some("/repo".into()),
            branch: Some("equipe/try-1".into()),
            base: Some("abc".into()),
            engine_session: None,
            title: None,
            tokens: 0,
            open: true,
        }
    }

    #[test]
    fn a_session_and_its_conversation_survive_reopening() {
        let home = temp_home();
        {
            let j = Journal::for_project(&home, Path::new("/repo")).unwrap();
            j.record_session(&record(7)).unwrap();
            j.set_title_if_empty(7, "add a docstring").unwrap();
            j.set_title_if_empty(7, "not this one").unwrap();
            j.set_engine_session(7, "sess-1").unwrap();
            j.add_tokens(7, 40).unwrap();
            j.append(7, &Entry::User { text: "add a docstring".into() }).unwrap();
            j.append(7, &Entry::Reply { text: "done".into() }).unwrap();
        }
        let j = Journal::for_project(&home, Path::new("/repo")).unwrap();
        let open = j.open_sessions().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].title.as_deref(), Some("add a docstring"));
        assert_eq!(open[0].engine_session.as_deref(), Some("sess-1"));
        assert_eq!(open[0].tokens, 40);
        assert_eq!(
            j.conversation(7).unwrap(),
            vec![Entry::User { text: "add a docstring".into() }, Entry::Reply { text: "done".into() }]
        );
        assert_eq!(j.project().as_deref(), Some(Path::new("/repo")));
    }

    #[test]
    fn members_are_restored_in_the_order_they_joined() {
        let j = Journal::for_project(&temp_home(), Path::new("/p")).unwrap();
        // Ids are random, so the order is when they joined, not the id.
        for (id, name) in [(90, "pm"), (10, "frontend")] {
            j.record_session(&SessionRecord { member: Some(Member { role: name.into(), name: name.into() }), ..record(id) }).unwrap();
        }
        let names: Vec<_> = j.open_sessions().unwrap().into_iter().map(|r| r.member.unwrap().name).collect();
        assert_eq!(names, ["pm", "frontend"]);
    }

    #[test]
    fn a_journal_from_before_members_gains_the_columns() {
        let home = temp_home();
        let path = home.join("old.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE sessions (id INTEGER PRIMARY KEY, engine TEXT NOT NULL, model TEXT, access TEXT NOT NULL,
                   cwd TEXT NOT NULL, repo TEXT, branch TEXT, base TEXT, engine_session TEXT, title TEXT, outcome TEXT,
                   tokens INTEGER NOT NULL DEFAULT 0, open INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL);
                 INSERT INTO sessions (id, engine, access, cwd, created_at, updated_at) VALUES (5, 'claude', 'read_only', '/r', 1, 1);",
            )
            .unwrap();
        }
        let j = Journal::open(&path).unwrap();
        let open = j.open_sessions().unwrap();
        assert_eq!((open[0].id, open[0].member.clone()), (5, None));
    }

    #[test]
    fn recent_returns_the_latest_lines_in_order() {
        let j = Journal::for_project(&temp_home(), Path::new("/p")).unwrap();
        j.record_session(&record(1)).unwrap();
        for i in 0..10 {
            j.append(1, &Entry::Note { text: i.to_string() }).unwrap();
        }
        let texts: Vec<_> = j
            .recent(1, 3)
            .unwrap()
            .into_iter()
            .map(|e| match e {
                Entry::Note { text } => text,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(texts, ["7", "8", "9"]);
    }

    #[test]
    fn closed_sessions_are_not_restored() {
        let j = Journal::for_project(&temp_home(), Path::new("/p")).unwrap();
        j.record_session(&record(1)).unwrap();
        j.record_session(&record(2)).unwrap();
        j.set_outcome(1, "landed equipe/try-1").unwrap();
        j.close_session(1).unwrap();
        let ids: Vec<_> = j.open_sessions().unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, [2]);
    }

    #[test]
    fn pruning_drops_old_conversation_keeps_the_record_and_returns_space() {
        let j = Journal::for_project(&temp_home(), Path::new("/p")).unwrap();
        j.record_session(&record(1)).unwrap();
        let big = "x".repeat(4000);
        for _ in 0..500 {
            j.append(1, &Entry::Reply { text: big.clone() }).unwrap();
        }
        j.checkpoint().unwrap();
        let before = j.stats().unwrap().bytes;
        j.backdate_entries(1, 1_000);
        let removed = j.prune(now() - 90 * 86_400).unwrap();
        assert_eq!(removed, 500);
        let after = j.stats().unwrap();
        assert_eq!(after.entries, 0);
        assert_eq!(after.sessions, 1, "the record stays");
        assert!(after.bytes < before / 4, "space returned: {before} → {}", after.bytes);
    }

    #[test]
    fn new_journals_are_created_in_incremental_vacuum_mode() {
        let j = Journal::for_project(&temp_home(), Path::new("/p")).unwrap();
        let mode: i64 = j.conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, 2, "2 = INCREMENTAL");
    }

    #[test]
    fn recent_lines_survive_pruning() {
        let j = Journal::for_project(&temp_home(), Path::new("/p")).unwrap();
        j.record_session(&record(1)).unwrap();
        j.append(1, &Entry::Note { text: "old".into() }).unwrap();
        j.backdate_entries(1, 1_000);
        j.append(1, &Entry::Note { text: "new".into() }).unwrap();
        j.prune(now() - 86_400).unwrap();
        assert_eq!(j.conversation(1).unwrap(), vec![Entry::Note { text: "new".into() }]);
    }

    #[test]
    fn each_project_gets_its_own_directory() {
        let home = Path::new("/h");
        let a = project_dir(home, Path::new("/code/app"));
        let b = project_dir(home, Path::new("/other/app"));
        assert_ne!(a, b);
        assert!(a.file_name().unwrap().to_string_lossy().starts_with("app-"));
        assert_eq!(a, project_dir(home, Path::new("/code/app")), "stable");
    }
}
