//! What happened, kept on disk: one SQLite database per squad.
//!
//! The journal holds the squad's *projects*, its *board* (tickets and an
//! append-only record of what happened to each), the *record* of its sessions
//! (engine, branch, title, outcome, tokens) and their *conversation* (messages,
//! finished replies, one line per tool use). It never holds raw material — file
//! contents, command output, images, thinking, diffs: git has the changes and
//! each engine keeps its own transcript.
//!
//! Conversations are kept for a window and then pruned; the session record stays.
//! The database is created in incremental-vacuum mode, so pruning gives the
//! space back to the disk instead of leaving the file at its high-water mark.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use equipe_protocol::{Access, Engine, Entry, Member, Mode, Priority, SessionId, Ticket, TicketEntry, TicketId, TicketKind, TicketStatus};
use rusqlite::{Connection, OptionalExtension as _, params};

pub use rusqlite::Error;
pub type Result<T> = std::result::Result<T, Error>;

/// Whether `err` says there is no room left: the disk is full (SQLite's
/// "database or disk is full"), or a file couldn't be written for it.
pub fn is_full(err: &Error) -> bool {
    match err {
        Error::SqliteFailure(e, message) => {
            e.code == rusqlite::ErrorCode::DiskFull || message.as_deref().is_some_and(|m| m.contains("disk is full"))
        }
        Error::ToSqlConversionFailure(e) => {
            e.downcast_ref::<std::io::Error>().is_some_and(|e| e.raw_os_error() == Some(libc::ENOSPC))
        }
        _ => false,
    }
}

static NO_ROOM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// From now on, journals opened in this process can't grow — as on a full
/// disk: for tests of what equipe does then.
#[doc(hidden)]
pub fn no_room_for_tests() {
    NO_ROOM.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// How long conversations are kept before they are pruned to the record.
pub const DEFAULT_RETENTION_DAYS: u64 = 90;

/// A session as the journal keeps it — enough to resume it after a restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: SessionId,
    /// The role it plays on the team, if any.
    pub member: Option<Member>,
    /// Its own mode, when it doesn't follow the squad's.
    pub mode: Option<Mode>,
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

/// `<home>/squads/<squad>`: everything a squad keeps.
pub fn squad_dir(home: &Path, squad: &str) -> PathBuf {
    home.join("squads").join(squad)
}

/// Where a role's members keep what they learn. It belongs to the role, not to
/// a project — like a person, a member carries what it learned from one project
/// to the next — nor to one member: `backend-2`, brought in for a rush, learns
/// for `backend` too, and what it learned stays when it leaves. What a project
/// needs everyone to know belongs in the repository (`CLAUDE.md`, `AGENTS.md`).
pub fn member_memory_dir(home: &Path, squad: &str, role: &str) -> PathBuf {
    squad_dir(home, squad).join("members").join(role).join("memory")
}

/// Where a member's builds go (`CARGO_TARGET_DIR`), kept from ticket to ticket.
pub fn build_dir(home: &Path, squad: &str, member: &str) -> PathBuf {
    home.join("cache").join("build").join(squad).join(member)
}

/// Fold what members like `backend-3` learned, from when each kept its own,
/// into their role's memory: lessons it lacks move over, a lesson of the same
/// name but other words keeps both, and the index gains their lines. Returns
/// how many lessons moved.
pub fn fold_member_memories(home: &Path, squad: &str) -> std::io::Result<usize> {
    let members = squad_dir(home, squad).join("members");
    let Ok(dirs) = std::fs::read_dir(&members) else {
        return Ok(0);
    };
    let mut moved = 0;
    for dir in dirs.flatten() {
        let name = dir.file_name().to_string_lossy().into_owned();
        let Some((role, n)) = name.rsplit_once('-') else {
            continue;
        };
        if role.is_empty() || n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let from = dir.path().join("memory");
        let to = members.join(role).join("memory");
        if from.is_dir() {
            std::fs::create_dir_all(&to)?;
            moved += fold_memory(&from, &to, &name)?;
        }
        let _ = std::fs::remove_dir_all(dir.path());
    }
    Ok(moved)
}

fn fold_memory(from: &Path, to: &Path, who: &str) -> std::io::Result<usize> {
    const INDEX: &str = "MEMORY.md";
    let mut index = std::fs::read_to_string(from.join(INDEX)).unwrap_or_default();
    let mut moved = 0;
    for entry in std::fs::read_dir(from)?.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if file == INDEX || !entry.path().is_file() {
            continue;
        }
        let text = std::fs::read(entry.path())?;
        let mut target = to.join(&file);
        if target.exists() {
            if std::fs::read(&target)? == text {
                continue;
            }
            let (stem, ext) = file.rsplit_once('.').unwrap_or((&file, ""));
            let renamed = if ext.is_empty() { format!("{stem}-from-{who}") } else { format!("{stem}-from-{who}.{ext}") };
            index = index.replace(&format!("({file})"), &format!("({renamed})"));
            target = to.join(renamed);
        }
        std::fs::write(&target, text)?;
        moved += 1;
    }
    let mut kept = std::fs::read_to_string(to.join(INDEX)).unwrap_or_default();
    let new: Vec<&str> = index.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#') && !kept.contains(*l)).collect();
    if !new.is_empty() {
        if !kept.is_empty() && !kept.ends_with('\n') {
            kept.push('\n');
        }
        for line in new {
            kept.push_str(line);
            kept.push('\n');
        }
        std::fs::write(to.join(INDEX), kept)?;
    }
    Ok(moved)
}

/// Where an editing member's ticket work is checked out, so it can be resumed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TicketWork {
    pub base: Option<String>,
    pub worktree: Option<PathBuf>,
    /// The engine's conversation for this ticket (Claude Code `--resume`).
    pub engine_session: Option<String>,
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

fn mode_str(mode: Mode) -> &'static str {
    match mode {
        Mode::ReadOnly => "read-only",
        Mode::Ask => "ask",
        Mode::Auto => "auto",
        Mode::Full => "full",
    }
}

fn status_str(status: TicketStatus) -> &'static str {
    match status {
        TicketStatus::Proposed => "proposed",
        TicketStatus::Todo => "todo",
        TicketStatus::InProgress => "in_progress",
        TicketStatus::Waiting => "waiting",
        TicketStatus::Review => "review",
        TicketStatus::Done => "done",
        TicketStatus::Cancelled => "cancelled",
    }
}

fn status_from(s: &str) -> TicketStatus {
    match s {
        "proposed" => TicketStatus::Proposed,
        "in_progress" => TicketStatus::InProgress,
        "waiting" => TicketStatus::Waiting,
        "review" => TicketStatus::Review,
        "done" => TicketStatus::Done,
        "cancelled" => TicketStatus::Cancelled,
        _ => TicketStatus::Todo,
    }
}

fn ids_str(ids: &[TicketId]) -> String {
    ids.iter().map(ToString::to_string).collect::<Vec<_>>().join(",")
}

/// What a ticket leaves open, as kept: a JSON list, or nothing.
fn remaining_str(asks: &[String]) -> Option<String> {
    (!asks.is_empty()).then(|| serde_json::to_string(asks).unwrap_or_default())
}

fn ids_from(s: &str) -> Vec<TicketId> {
    s.split(',').filter_map(|id| id.trim().parse().ok()).collect()
}

fn access_str(access: Access) -> &'static str {
    match access {
        Access::ReadOnly => "read_only",
        Access::EditFiles => "edit_files",
    }
}

impl Journal {
    /// Open (or create) the journal of `squad` under `home`.
    pub fn for_squad(home: &Path, squad: &str) -> Result<Self> {
        let dir = squad_dir(home, squad);
        std::fs::create_dir_all(&dir).map_err(|e| Error::ToSqlConversionFailure(Box::new(e)))?;
        let journal = Self::open(&dir.join("squad.db"))?;
        journal.conn.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('squad', ?1)", [squad])?;
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
             CREATE INDEX IF NOT EXISTS entries_at ON entries(at);
             CREATE TABLE IF NOT EXISTS projects (
               path TEXT PRIMARY KEY,
               added_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS tickets (
               id INTEGER PRIMARY KEY,
               title TEXT NOT NULL,
               body TEXT NOT NULL,
               status TEXT NOT NULL,
               owner TEXT,
               assignee TEXT,
               project TEXT,
               depends_on TEXT NOT NULL DEFAULT '',
               branch TEXT,
               base TEXT,
               worktree TEXT,
               engine_session TEXT,
               created_by TEXT NOT NULL,
               note TEXT,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS ticket_events (
               ticket INTEGER NOT NULL,
               seq INTEGER NOT NULL,
               at INTEGER NOT NULL,
               by TEXT NOT NULL,
               kind TEXT NOT NULL,
               text TEXT NOT NULL,
               PRIMARY KEY (ticket, seq)
             ) WITHOUT ROWID;",
        )?;
        // Columns added after the first release; a journal that has them already
        // refuses the ALTER, which is fine.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS waiting (
               session INTEGER NOT NULL,
               kind TEXT NOT NULL,
               seq INTEGER NOT NULL,
               text TEXT NOT NULL,
               PRIMARY KEY (session, kind, seq)
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS turn_times (
               role TEXT NOT NULL,
               seconds INTEGER NOT NULL,
               at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS allowed (
               rule TEXT PRIMARY KEY,
               by TEXT NOT NULL,
               added_at INTEGER NOT NULL
             );",
        )?;
        for column in ["role TEXT", "name TEXT", "joined INTEGER", "mode TEXT"] {
            let _ = conn.execute(&format!("ALTER TABLE sessions ADD COLUMN {column}"), []);
        }
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN merged INTEGER", []);
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN off_main TEXT", []);
        // The person's say on how soon and in which order: normal and by number
        // for tickets from before.
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN priority TEXT NOT NULL DEFAULT 'normal'", []);
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN rank INTEGER NOT NULL DEFAULT 0", []);
        // What the person left open when they answered a wait (JSON list).
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN remaining TEXT", []);
        // Tickets from before kinds: `code` with a project, `task` without — once,
        // when the column arrives.
        if conn.execute("ALTER TABLE tickets ADD COLUMN kind TEXT NOT NULL DEFAULT 'code'", []).is_ok() {
            conn.execute("UPDATE tickets SET kind = 'task' WHERE project IS NULL", [])?;
        }
        if NO_ROOM.load(std::sync::atomic::Ordering::Relaxed) {
            // No bigger than it is now: SQLite answers SQLITE_FULL, as on a full disk.
            conn.query_row("PRAGMA max_page_count = 1", [], |_| Ok(()))?;
        }
        Ok(Self { conn, path: path.to_owned() })
    }

    /// The squad this journal belongs to.
    pub fn squad(&self) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = 'squad'", [], |r| r.get::<_, String>(0))
            .optional()
            .ok()
            .flatten()
    }

    // ── modes and permissions ──

    /// How much the squad's members may do on their own, by default.
    pub fn mode(&self) -> Mode {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = 'mode'", [], |r| r.get::<_, String>(0))
            .optional()
            .ok()
            .flatten()
            .and_then(|m| Mode::stored(&m))
            .unwrap_or_default()
    }

    /// The language the person writes in, as last detected.
    pub fn language(&self) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = 'language'", [], |r| r.get::<_, String>(0))
            .optional()
            .ok()
            .flatten()
    }

    /// Whose browser opens on screen: everyone's, or the members named.
    fn shown_browsers(&self) -> (bool, Vec<String>) {
        let value: Option<String> =
            self.conn.query_row("SELECT value FROM meta WHERE key = 'browser_shown'", [], |r| r.get(0)).optional().ok().flatten();
        let value: serde_json::Value = value.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        let members = value["members"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(str::to_owned)).collect();
        (value["all"].as_bool().unwrap_or(false), members)
    }

    /// Whether every member's browser opens on screen.
    pub fn browsers_all_shown(&self) -> bool {
        self.shown_browsers().0
    }

    /// Whether `member`'s browser opens on screen rather than hidden.
    pub fn browser_shown(&self, member: &str) -> bool {
        let (all, members) = self.shown_browsers();
        all || members.iter().any(|m| m == member)
    }

    /// Show or hide one member's browser, or everyone's; `team` is who is on
    /// the squad, so hiding one of "everyone" leaves the others shown.
    pub fn show_browser(&self, member: Option<&str>, show: bool, team: &[String]) -> Result<()> {
        let (all, mut members) = self.shown_browsers();
        let (all, members) = match (member, show) {
            (None, show) => (show, Vec::new()),
            (Some(name), true) => {
                if !members.iter().any(|m| m == name) {
                    members.push(name.to_owned());
                }
                (all, members)
            }
            (Some(name), false) => {
                let shown: Vec<String> = if all { team.to_vec() } else { members };
                (false, shown.into_iter().filter(|m| m != name).collect())
            }
        };
        let value = serde_json::json!({ "all": all, "members": members }).to_string();
        self.conn.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('browser_shown', ?1)", [value])?;
        Ok(())
    }

    /// The members allowed the Mac's desktop (experimental; nobody by default).
    pub fn desktop_members(&self) -> Vec<String> {
        let value: serde_json::Value = self.meta("desktop_members").and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        value.as_array().into_iter().flatten().filter_map(|m| m.as_str().map(str::to_owned)).collect()
    }

    /// Whether `member` works the Mac's desktop.
    pub fn desktop_enabled(&self, member: &str) -> bool {
        self.desktop_members().iter().any(|m| m == member)
    }

    /// Allow `member` the desktop, or take it back.
    pub fn set_desktop(&self, member: &str, on: bool) -> Result<()> {
        let mut members: Vec<String> = self.desktop_members().into_iter().filter(|m| m != member).collect();
        if on {
            members.push(member.to_owned());
        }
        self.set_meta("desktop_members", &serde_json::json!(members).to_string())
    }

    fn meta(&self, key: &str) -> Option<String> {
        self.conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)", [key, value])?;
        Ok(())
    }

    /// Members' questions waiting for the person (`ask_decision`), oldest
    /// first: kept so a restart doesn't lose them.
    pub fn questions(&self) -> Vec<equipe_protocol::decide::QuestionSet> {
        self.meta("questions").and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    pub fn set_questions(&self, sets: &[equipe_protocol::decide::QuestionSet]) -> Result<()> {
        self.set_meta("questions", &serde_json::to_string(sets).unwrap_or_default())
    }

    /// The PM's conversation threads: the project its desk conversation is
    /// about now (`None`: the whole squad), and each other thread's
    /// conversation id, put aside — keyed by project, `""` for the squad's.
    pub fn desk_threads(&self, id: SessionId) -> (Option<String>, HashMap<String, String>) {
        let value: serde_json::Value = self.meta(&format!("threads:{id}")).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        let current = value["current"].as_str().map(str::to_owned);
        let others = value["others"].as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect()).unwrap_or_default();
        (current, others)
    }

    pub fn set_desk_threads(&self, id: SessionId, current: Option<&str>, others: &HashMap<String, String>) -> Result<()> {
        self.set_meta(&format!("threads:{id}"), &serde_json::json!({ "current": current, "others": others }).to_string())
    }

    /// How many extra members of a role equipe may bring in when its work
    /// waits; 2 unless the person set it.
    pub fn scale(&self) -> usize {
        self.meta("scale").and_then(|v| v.parse().ok()).unwrap_or(2)
    }

    pub fn set_scale(&self, extra: usize) -> Result<()> {
        self.set_meta("scale", &extra.to_string())
    }

    /// The person turned off guesses at their next message.
    pub fn suggestions_off(&self) -> bool {
        self.meta("suggestions").as_deref() == Some("off")
    }

    pub fn set_suggestions(&self, on: bool) -> Result<()> {
        self.set_meta("suggestions", if on { "on" } else { "off" })
    }

    /// The members equipe brought in to help, which it lets go when idle.
    pub fn helpers(&self) -> Vec<SessionId> {
        self.meta("helpers").and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    pub fn set_helpers(&self, helpers: &[SessionId]) -> Result<()> {
        self.set_meta("helpers", &serde_json::to_string(helpers).unwrap_or_default())
    }

    /// The apps where a member's input may send in the person's name, so a
    /// send waits for their yes: by name or bundle id. The defaults until the
    /// person changes them.
    pub fn messaging_apps(&self) -> Vec<String> {
        self.meta("messaging_apps")
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_else(|| crate::desktop_gate::DEFAULT_APPS.iter().map(|a| (*a).to_owned()).collect())
    }

    pub fn set_messaging_apps(&self, apps: &[String]) -> Result<()> {
        self.set_meta("messaging_apps", &serde_json::to_string(apps).unwrap_or_default())
    }

    pub fn set_language(&self, language: &str) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('language', ?1)", [language])?;
        Ok(())
    }

    pub fn set_mode(&self, mode: Mode) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('mode', ?1)", [mode_str(mode)])?;
        Ok(())
    }

    /// A member's own mode; `None` follows the squad's.
    pub fn set_member_mode(&self, id: SessionId, mode: Option<Mode>) -> Result<()> {
        self.touch(id, "mode", mode.map(mode_str))
    }

    /// The squad's "always allow" rules, oldest first.
    pub fn allowed(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT rule FROM allowed ORDER BY added_at, rowid")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    pub fn allow(&self, rule: &str, by: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO allowed (rule, by, added_at) VALUES (?1, ?2, ?3)",
            params![rule, by, now()],
        )?;
        Ok(())
    }

    /// Take an "always allow" rule back. False when the squad had no such rule.
    pub fn disallow(&self, rule: &str) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM allowed WHERE rule = ?1", [rule])? > 0)
    }

    // ── projects ──

    /// The squad's projects, in the order they joined.
    pub fn projects(&self) -> Result<Vec<PathBuf>> {
        let mut stmt = self.conn.prepare("SELECT path FROM projects ORDER BY added_at, rowid")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|p| p.ok().map(PathBuf::from)).collect())
    }

    pub fn add_project(&self, path: &Path) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO projects (path, added_at) VALUES (?1, ?2)",
            params![path.to_string_lossy(), now()],
        )?;
        Ok(())
    }

    /// Stop working on a project. Its tickets stay on the board, as they were.
    pub fn remove_project(&self, path: &Path) -> Result<()> {
        self.conn.execute("DELETE FROM projects WHERE path = ?1", params![path.to_string_lossy()])?;
        Ok(())
    }

    // ── the board ──

    /// Put a ticket on the board. Its id is the next free number; the one given is ignored.
    pub fn create_ticket(&self, t: &Ticket) -> Result<TicketId> {
        let id: i64 = self.conn.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM tickets", [], |r| r.get(0))?;
        let at = now();
        self.conn.execute(
            "INSERT INTO tickets (id, title, body, status, owner, assignee, project, depends_on, branch, created_by,
                                  note, created_at, updated_at, kind, priority, rank)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, ?13, ?14, ?15)",
            params![
                id,
                t.title,
                t.body,
                status_str(t.status),
                t.owner,
                t.assignee,
                t.project.as_ref().map(|p| p.to_string_lossy().into_owned()),
                ids_str(&t.depends_on),
                t.branch,
                t.created_by,
                t.note,
                at,
                t.kind.label(),
                t.priority.label(),
                t.rank,
            ],
        )?;
        Ok(id as TicketId)
    }

    /// Write a ticket's fields back.
    pub fn save_ticket(&self, t: &Ticket) -> Result<()> {
        self.conn.execute(
            "UPDATE tickets SET title = ?1, body = ?2, status = ?3, owner = ?4, assignee = ?5, project = ?6,
                                depends_on = ?7, branch = ?8, note = ?9, updated_at = ?10, merged = ?12, kind = ?13,
                                off_main = ?14, priority = ?15, rank = ?16, remaining = ?17
             WHERE id = ?11",
            params![
                t.title,
                t.body,
                status_str(t.status),
                t.owner,
                t.assignee,
                t.project.as_ref().map(|p| p.to_string_lossy().into_owned()),
                ids_str(&t.depends_on),
                t.branch,
                t.note,
                now(),
                t.id,
                t.merged,
                t.kind.label(),
                t.off_main.as_ref().and_then(|o| serde_json::to_string(o).ok()),
                t.priority.label(),
                t.rank,
                remaining_str(&t.remaining),
            ],
        )?;
        Ok(())
    }

    pub fn ticket(&self, id: TicketId) -> Result<Option<Ticket>> {
        Ok(self.query_tickets("WHERE id = ?1", params![id])?.into_iter().next())
    }

    /// Every ticket, oldest first.
    /// Work not yet started — to do, or proposed — that hasn't moved in
    /// `older_than` seconds: id, title, and how long it has sat.
    pub fn stale_tickets(&self, older_than: i64) -> Result<Vec<(TicketId, String, i64)>> {
        let now = now();
        let mut stmt = self.conn.prepare(
            "SELECT id, title, updated_at FROM tickets WHERE status IN ('todo', 'proposed') AND updated_at < ?1 ORDER BY updated_at",
        )?;
        let rows = stmt.query_map([now - older_than], |r| Ok((r.get::<_, TicketId>(0)?, r.get::<_, String>(1)?, now - r.get::<_, i64>(2)?)))?;
        rows.collect()
    }

    /// When the PM was last asked to look stale work over.
    pub fn tidied_at(&self) -> i64 {
        self.meta("tidied_at").and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    pub fn set_tidied_at(&self) -> Result<()> {
        self.set_meta("tidied_at", &now().to_string())
    }

    pub fn tickets(&self) -> Result<Vec<Ticket>> {
        self.query_tickets("", [])
    }

    fn query_tickets(&self, filter: &str, args: impl rusqlite::Params) -> Result<Vec<Ticket>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, title, body, status, owner, assignee, project, depends_on, branch, created_by, note, worktree, merged, kind, off_main, updated_at,
                    priority, rank, remaining
             FROM tickets {filter} ORDER BY id"
        ))?;
        let rows = stmt.query_map(args, |r| {
            let mut t = Ticket {
                id: r.get(0)?,
                title: r.get(1)?,
                body: r.get(2)?,
                status: status_from(&r.get::<_, String>(3)?),
                owner: r.get(4)?,
                assignee: r.get(5)?,
                project: r.get::<_, Option<String>>(6)?.map(PathBuf::from),
                depends_on: ids_from(&r.get::<_, String>(7)?),
                branch: r.get(8)?,
                created_by: r.get(9)?,
                note: r.get(10)?,
                worktree: r.get::<_, Option<String>>(11)?.map(PathBuf::from),
                merged: r.get(12)?,
                kind: TicketKind::parse(&r.get::<_, String>(13)?).unwrap_or_default(),
                asks: Vec::new(),
                remaining: r.get::<_, Option<String>>(18)?.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default(),
                priority: Priority::parse(&r.get::<_, String>(16)?).unwrap_or_default(),
                rank: r.get(17)?,
                off_main: r.get::<_, Option<String>>(14)?.and_then(|o| serde_json::from_str(&o).ok()),
                updated_at: r.get::<_, Option<i64>>(15)?.map(|at| at.max(0) as u64),
            };
            // A waiting ticket's asks are kept in its note; they travel as a list.
            t.asks = t.asks();
            Ok(t)
        })?;
        rows.collect()
    }

    pub fn ticket_work(&self, id: TicketId) -> Result<TicketWork> {
        self.conn
            .query_row("SELECT base, worktree, engine_session FROM tickets WHERE id = ?1", [id], |r| {
                Ok(TicketWork {
                    base: r.get(0)?,
                    worktree: r.get::<_, Option<String>>(1)?.map(PathBuf::from),
                    engine_session: r.get(2)?,
                })
            })
            .optional()
            .map(Option::unwrap_or_default)
    }

    pub fn set_ticket_work(&self, id: TicketId, work: &TicketWork) -> Result<()> {
        self.conn.execute(
            "UPDATE tickets SET base = ?1, worktree = ?2, engine_session = ?3 WHERE id = ?4",
            params![work.base, work.worktree.as_ref().map(|p| p.to_string_lossy().into_owned()), work.engine_session, id],
        )?;
        Ok(())
    }

    /// Add to a ticket's record — kept forever, a few hundred bytes a line.
    pub fn ticket_event(&self, id: TicketId, by: &str, kind: &str, text: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO ticket_events (ticket, seq, at, by, kind, text)
             VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM ticket_events WHERE ticket = ?1), ?2, ?3, ?4, ?5)",
            params![id, now(), by, kind, text],
        )?;
        Ok(())
    }

    /// How long tickets took, from first being assigned to first being handed
    /// in (or put down for the person): `(member, kind, seconds)`, newest first.
    pub fn ticket_times(&self) -> Result<Vec<(String, TicketKind, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.text, t.kind, h.at - a.at FROM tickets t
             JOIN (SELECT ticket, text, at, MIN(seq) FROM ticket_events WHERE kind = 'assigned' GROUP BY ticket) a ON a.ticket = t.id
             JOIN (SELECT ticket, MIN(at) AS at FROM ticket_events WHERE kind IN ('handed in', 'waiting') GROUP BY ticket) h ON h.ticket = t.id
             WHERE h.at >= a.at ORDER BY h.at DESC LIMIT 500",
        )?;
        let rows = stmt.query_map([], |r| {
            let kind: String = r.get(1)?;
            Ok((r.get::<_, String>(0)?, TicketKind::parse(&kind).unwrap_or_default(), r.get::<_, i64>(2)?.max(0) as u64))
        })?;
        rows.collect()
    }

    /// Keep how long a turn of `role`'s at its desk took.
    pub fn record_turn_time(&self, role: &str, seconds: u64) -> Result<()> {
        self.conn.execute("INSERT INTO turn_times (role, seconds, at) VALUES (?1, ?2, ?3)", params![role, seconds as i64, now()])?;
        // The latest few hundred are plenty to go by.
        self.conn.execute("DELETE FROM turn_times WHERE rowid NOT IN (SELECT rowid FROM turn_times ORDER BY at DESC LIMIT 500)", [])?;
        Ok(())
    }

    /// How long `role`'s desk turns took, newest first, in seconds.
    pub fn turn_times(&self, role: &str) -> Result<Vec<u64>> {
        let mut stmt = self.conn.prepare("SELECT seconds FROM turn_times WHERE role = ?1 ORDER BY at DESC, rowid DESC LIMIT 50")?;
        let rows = stmt.query_map([role], |r| Ok(r.get::<_, i64>(0)?.max(0) as u64))?;
        rows.collect()
    }

    /// A ticket's record, oldest first: `(by, kind, text)`.
    pub fn ticket_history(&self, id: TicketId) -> Result<Vec<(String, String, String)>> {
        Ok(self.ticket_entries(id)?.into_iter().map(|e| (e.by, e.kind, e.text)).collect())
    }

    /// A ticket's record, oldest first, each line with when.
    pub fn ticket_entries(&self, id: TicketId) -> Result<Vec<TicketEntry>> {
        let mut stmt = self.conn.prepare("SELECT at, by, kind, text FROM ticket_events WHERE ticket = ?1 ORDER BY seq")?;
        let rows = stmt.query_map([id], |r| Ok(TicketEntry { at: r.get::<_, i64>(0)?.max(0) as u64, by: r.get(1)?, kind: r.get(2)?, text: r.get(3)? }))?;
        rows.collect()
    }

    pub fn record_session(&self, r: &SessionRecord) -> Result<()> {
        let t = now();
        self.conn.execute(
            "INSERT OR REPLACE INTO sessions
               (id, engine, model, access, cwd, repo, branch, base, engine_session, title, tokens, open, created_at, updated_at,
                role, name, joined, mode)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13, ?14, ?15,
                     (SELECT COALESCE(MAX(joined), 0) + 1 FROM sessions), ?16)",
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
                r.mode.map(mode_str),
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

    /// Where a session's desk turns run from now on.
    pub fn set_cwd(&self, id: SessionId, cwd: &Path) -> Result<()> {
        self.touch(id, "cwd", cwd.to_string_lossy())
    }

    /// Forget the engine's conversation: the next turn starts a fresh one.
    pub fn clear_engine_session(&self, id: SessionId) -> Result<()> {
        self.touch(id, "engine_session", None::<String>)
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
        self.conn.execute("DELETE FROM waiting WHERE session = ?1", params![id as i64])?;
        self.touch(id, "open", false)
    }

    /// What waits for a member's next turn, as it stands now: `memo`s from the
    /// person, the PM's `briefing`. Kept so a restart loses none of it.
    pub fn set_waiting(&self, id: SessionId, kind: &str, texts: &[String]) -> Result<()> {
        self.conn.execute("DELETE FROM waiting WHERE session = ?1 AND kind = ?2", params![id as i64, kind])?;
        for (seq, text) in texts.iter().enumerate() {
            self.conn.execute(
                "INSERT INTO waiting (session, kind, seq, text) VALUES (?1, ?2, ?3, ?4)",
                params![id as i64, kind, seq as i64, text],
            )?;
        }
        Ok(())
    }

    pub fn waiting(&self, id: SessionId, kind: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT text FROM waiting WHERE session = ?1 AND kind = ?2 ORDER BY seq")?;
        let rows = stmt.query_map(params![id as i64, kind], |r| r.get::<_, String>(0))?;
        rows.collect()
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
            "SELECT id, engine, model, access, cwd, repo, branch, base, engine_session, title, tokens, open, role, name, mode
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
                mode: r.get::<_, Option<String>>(14)?.as_deref().and_then(Mode::stored),
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

    #[test]
    fn a_journal_with_no_room_says_so_and_still_reads() {
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
        j.record_session(&record(1)).unwrap();
        let pages: i64 = j.conn.query_row("PRAGMA max_page_count = 1", [], |r| r.get(0)).unwrap();
        assert!(pages >= 1);
        let err = j.append(1, &Entry::User { text: "x".repeat(200_000) }).unwrap_err();
        assert!(is_full(&err), "{err:?}");
        assert!(!is_full(&Error::QueryReturnedNoRows));
        // What it holds is still there to read; a lock left behind would fail this.
        assert_eq!(j.open_sessions().unwrap().len(), 1);
        j.set_title_if_empty(1, "small enough to fit").unwrap();
    }

    fn record(id: SessionId) -> SessionRecord {
        SessionRecord {
            id,
            member: None,
            mode: None,
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
            let j = Journal::for_squad(&home, "acme").unwrap();
            j.record_session(&record(7)).unwrap();
            j.set_title_if_empty(7, "add a docstring").unwrap();
            j.set_title_if_empty(7, "not this one").unwrap();
            j.set_engine_session(7, "sess-1").unwrap();
            j.add_tokens(7, 40).unwrap();
            j.append(7, &Entry::User { text: "add a docstring".into() }).unwrap();
            j.append(7, &Entry::Reply { text: "done".into() }).unwrap();
        }
        let j = Journal::for_squad(&home, "acme").unwrap();
        let open = j.open_sessions().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].title.as_deref(), Some("add a docstring"));
        assert_eq!(open[0].engine_session.as_deref(), Some("sess-1"));
        assert_eq!(open[0].tokens, 40);
        assert_eq!(
            j.conversation(7).unwrap(),
            vec![Entry::User { text: "add a docstring".into() }, Entry::Reply { text: "done".into() }]
        );
        assert_eq!(j.squad().as_deref(), Some("acme"));
    }

    #[test]
    fn members_are_restored_in_the_order_they_joined() {
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
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
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
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
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
        j.record_session(&record(1)).unwrap();
        j.record_session(&record(2)).unwrap();
        j.set_outcome(1, "landed equipe/try-1").unwrap();
        j.close_session(1).unwrap();
        let ids: Vec<_> = j.open_sessions().unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, [2]);
    }

    #[test]
    fn pruning_drops_old_conversation_keeps_the_record_and_returns_space() {
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
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
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
        let mode: i64 = j.conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, 2, "2 = INCREMENTAL");
    }

    #[test]
    fn recent_lines_survive_pruning() {
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
        j.record_session(&record(1)).unwrap();
        j.append(1, &Entry::Note { text: "old".into() }).unwrap();
        j.backdate_entries(1, 1_000);
        j.append(1, &Entry::Note { text: "new".into() }).unwrap();
        j.prune(now() - 86_400).unwrap();
        assert_eq!(j.conversation(1).unwrap(), vec![Entry::Note { text: "new".into() }]);
    }

    #[test]
    fn a_squad_keeps_its_projects_in_the_order_they_joined() {
        let j = Journal::for_squad(&temp_home(), "acme");
        let j = j.unwrap();
        j.add_project(Path::new("/code/site")).unwrap();
        j.add_project(Path::new("/code/app")).unwrap();
        j.add_project(Path::new("/code/site")).unwrap();
        assert_eq!(j.projects().unwrap(), [PathBuf::from("/code/site"), PathBuf::from("/code/app")]);
    }

    #[test]
    fn tickets_that_have_not_moved_in_a_while_are_found() {
        let home = temp_home();
        let j = Journal::for_squad(&home, "acme").unwrap();
        let old = j.create_ticket(&ticket("Old idea")).unwrap();
        let fresh = j.create_ticket(&ticket("New idea")).unwrap();
        let _ = fresh;
        j.conn.execute("UPDATE tickets SET updated_at = updated_at - 9 * 86400 WHERE id = ?1", [old]).unwrap();
        let stale = j.stale_tickets(7 * 86400).unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!((stale[0].0, stale[0].1.as_str(), stale[0].2 / 86400), (old, "Old idea", 9));
        std::fs::remove_dir_all(&home).unwrap();
    }

    fn ticket(title: &str) -> Ticket {
        Ticket {
            id: 0,
            title: title.into(),
            body: "do it".into(),
            status: TicketStatus::Todo,
            owner: Some("frontend".into()),
            assignee: None,
            project: Some("/code/site".into()),
            depends_on: vec![],
            branch: None,
            worktree: None,
            merged: None,
            off_main: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            remaining: Vec::new(),
            updated_at: None,
            priority: Default::default(), rank: 0,
            kind: TicketKind::Code,
        }
    }

    #[test]
    fn a_ticket_keeps_its_kind_and_old_tickets_get_one_from_their_project() {
        let home = temp_home();
        let path = squad_dir(&home, "acme").join("squad.db");
        {
            let j = Journal::for_squad(&home, "acme").unwrap();
            j.create_ticket(&Ticket { kind: TicketKind::Action, project: None, ..ticket("Reply") }).unwrap();
            let mut t = j.ticket(1).unwrap().unwrap();
            assert_eq!(t.kind, TicketKind::Action);
            t.kind = TicketKind::Task;
            j.save_ticket(&t).unwrap();
            assert_eq!(j.ticket(1).unwrap().unwrap().kind, TicketKind::Task);
            // As a journal from before kinds would have them.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "ALTER TABLE tickets DROP COLUMN kind;
                 INSERT INTO tickets (id, title, body, status, created_by, created_at, updated_at, project)
                   VALUES (2, 'Hero', '-', 'todo', 'pm', 1, 1, '/code/site');
                 INSERT INTO tickets (id, title, body, status, created_by, created_at, updated_at)
                   VALUES (3, 'Research', '-', 'todo', 'pm', 1, 1);",
            )
            .unwrap();
        }
        let j = Journal::for_squad(&home, "acme").unwrap();
        let kinds: Vec<TicketKind> = j.tickets().unwrap().iter().map(|t| t.kind).collect();
        assert_eq!(kinds, [TicketKind::Task, TicketKind::Code, TicketKind::Task]);
    }

    #[test]
    fn what_the_person_left_open_is_kept() {
        let home = temp_home();
        let j = Journal::for_squad(&home, "acme").unwrap();
        j.create_ticket(&ticket("App Store")).unwrap();
        let mut t = j.ticket(1).unwrap().unwrap();
        assert!(t.remaining.is_empty());
        t.remaining = vec!["EU DSA 상태를 trader로 바꿔 주세요.".into()];
        j.save_ticket(&t).unwrap();
        assert_eq!(j.ticket(1).unwrap().unwrap().remaining, t.remaining);
        t.remaining.clear();
        j.save_ticket(&t).unwrap();
        assert!(j.ticket(1).unwrap().unwrap().remaining.is_empty());
    }

    #[test]
    fn a_tickets_priority_and_place_are_kept_and_old_ones_are_normal_by_number() {
        let home = temp_home();
        let path = squad_dir(&home, "acme").join("squad.db");
        {
            let j = Journal::for_squad(&home, "acme").unwrap();
            j.create_ticket(&Ticket { priority: Priority::Urgent, ..ticket("Fire") }).unwrap();
            let mut t = j.ticket(1).unwrap().unwrap();
            assert_eq!((t.priority, t.rank), (Priority::Urgent, 0));
            assert!(t.updated_at.is_some_and(|at| at > 0), "when it last changed travels with it");
            t.rank = -512;
            t.priority = Priority::Low;
            j.save_ticket(&t).unwrap();
            let t = j.ticket(1).unwrap().unwrap();
            assert_eq!((t.priority, t.rank), (Priority::Low, -512));
            // As a journal from before priorities would have them.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "ALTER TABLE tickets DROP COLUMN priority;
                 ALTER TABLE tickets DROP COLUMN rank;
                 INSERT INTO tickets (id, title, body, status, created_by, created_at, updated_at, kind)
                   VALUES (2, 'Old', '-', 'todo', 'pm', 1, 1, 'task');",
            )
            .unwrap();
        }
        let j = Journal::for_squad(&home, "acme").unwrap();
        let tickets = j.tickets().unwrap();
        assert_eq!(tickets.iter().map(|t| (t.priority, t.order_key())).collect::<Vec<_>>(), [(Priority::Normal, 1024), (Priority::Normal, 2048)]);
        j.ticket_event(2, "you", "edited", "priority").unwrap();
        let entries = j.ticket_entries(2).unwrap();
        assert_eq!((entries[0].by.as_str(), entries[0].kind.as_str()), ("you", "edited"));
        assert!(entries[0].at > 0);
    }

    #[test]
    fn tickets_are_numbered_kept_and_updated() {
        let home = temp_home();
        {
            let j = Journal::for_squad(&home, "acme").unwrap();
            assert_eq!(j.create_ticket(&ticket("Hero")).unwrap(), 1);
            let id = j.create_ticket(&Ticket { depends_on: vec![1], ..ticket("Pricing") }).unwrap();
            assert_eq!(id, 2);
            let mut t = j.ticket(2).unwrap().unwrap();
            t.status = TicketStatus::InProgress;
            t.assignee = Some("frontend".into());
            t.merged = Some(false);
            t.off_main = Some(equipe_protocol::OffMain::Conflict { detail: "a.rs".into() });
            j.save_ticket(&t).unwrap();
            j.set_ticket_work(2, &TicketWork { base: Some("abc".into()), worktree: Some("/w/2".into()), engine_session: Some("s".into()) }).unwrap();
            j.ticket_event(2, "pm", "created", "Pricing").unwrap();
            j.ticket_event(2, "equipe", "assigned", "frontend").unwrap();
        }
        let j = Journal::for_squad(&home, "acme").unwrap();
        let tickets = j.tickets().unwrap();
        assert_eq!(tickets.len(), 2);
        assert_eq!((tickets[1].status, tickets[1].depends_on.clone()), (TicketStatus::InProgress, vec![1]));
        assert_eq!((tickets[0].merged, tickets[1].merged), (None, Some(false)));
        assert!(tickets.iter().all(|t| t.updated_at.is_some_and(|at| at > 0)), "when each last changed travels with it");
        assert_eq!((tickets[0].off_main.clone(), tickets[1].off_main.clone()), (None, Some(equipe_protocol::OffMain::Conflict { detail: "a.rs".into() })));
        assert_eq!(j.ticket_work(2).unwrap().worktree, Some(PathBuf::from("/w/2")));
        assert_eq!(j.ticket_history(2).unwrap()[1], ("equipe".into(), "assigned".into(), "frontend".into()));
    }

    #[test]
    fn modes_and_always_allow_rules_are_kept() {
        let home = temp_home();
        {
            let j = Journal::for_squad(&home, "acme").unwrap();
            assert_eq!(j.mode(), Mode::Auto, "auto until the person says otherwise");
            j.set_mode(Mode::Ask).unwrap();
            j.record_session(&record(3)).unwrap();
            j.set_member_mode(3, Some(Mode::Full)).unwrap();
            j.allow("Bash(npm install:*)", "you").unwrap();
            j.allow("Bash(npm install:*)", "you").unwrap();
        }
        let j = Journal::for_squad(&home, "acme").unwrap();
        assert_eq!(j.mode(), Mode::Ask);
        assert_eq!(j.open_sessions().unwrap()[0].mode, Some(Mode::Full));
        assert_eq!(j.allowed().unwrap(), ["Bash(npm install:*)"]);
    }

    #[test]
    fn an_always_allow_rule_taken_back_is_gone_and_the_others_stay() {
        let home = temp_home();
        let j = Journal::for_squad(&home, "acme").unwrap();
        j.allow("Bash(npm install:*)", "you").unwrap();
        j.allow("Bash(gh:*)", "you").unwrap();
        assert!(j.disallow("Bash(npm install:*)").unwrap());
        assert!(!j.disallow("Bash(npm install:*)").unwrap(), "only once");
        assert_eq!(j.allowed().unwrap(), ["Bash(gh:*)"]);
        assert!(!j.browsers_all_shown());
        j.show_browser(None, true, &[]).unwrap();
        assert!(j.browsers_all_shown());
    }

    #[test]
    fn each_squad_gets_its_own_directory() {
        let home = Path::new("/h");
        assert_eq!(squad_dir(home, "acme"), PathBuf::from("/h/squads/acme"));
        assert_eq!(member_memory_dir(home, "acme", "frontend"), PathBuf::from("/h/squads/acme/members/frontend/memory"));
    }

    #[test]
    fn what_extra_members_learned_folds_into_their_roles_memory() {
        let home = std::env::temp_dir().join(format!("equipe-fold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let write = |member: &str, file: &str, text: &str| {
            let dir = member_memory_dir(&home, "acme", member);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(file), text).unwrap();
        };
        write("backend", "MEMORY.md", "- [Cargo](cargo.md) — not on PATH\n");
        write("backend", "cargo.md", "cargo lives in ~/.cargo/bin");
        write("backend-3", "MEMORY.md", "- [Cargo](cargo.md) — not on PATH\n- [Hold](hold.md) — hold means stop\n");
        write("backend-3", "cargo.md", "cargo lives in ~/.cargo/bin");
        write("backend-3", "hold.md", "hold means stop");
        write("mobile-2", "MEMORY.md", "- [Android](android.md) — the SDK path\n");
        write("mobile-2", "android.md", "ANDROID_HOME is set");
        write("frontend-2", "MEMORY.md", "- [Cargo](cargo.md) — a different lesson\n");
        write("frontend-2", "cargo.md", "something else");
        write("frontend", "cargo.md", "frontend's own");
        write("web-app", "notes.md", "a role whose name has a dash");

        assert_eq!(fold_member_memories(&home, "acme").unwrap(), 3);
        let read = |member: &str, file: &str| std::fs::read_to_string(member_memory_dir(&home, "acme", member).join(file)).unwrap();
        assert_eq!(read("backend", "hold.md"), "hold means stop");
        assert_eq!(read("backend", "MEMORY.md"), "- [Cargo](cargo.md) — not on PATH\n- [Hold](hold.md) — hold means stop\n");
        // A role with no member of its own name yet gets one's memory.
        assert_eq!(read("mobile", "android.md"), "ANDROID_HOME is set");
        assert_eq!(read("frontend", "cargo.md"), "frontend's own");
        assert_eq!(read("frontend", "cargo-from-frontend-2.md"), "something else");
        assert!(read("frontend", "MEMORY.md").contains("(cargo-from-frontend-2.md)"));
        assert!(!home.join("squads/acme/members/backend-3").exists());
        assert_eq!(read("web-app", "notes.md"), "a role whose name has a dash");
        assert_eq!(fold_member_memories(&home, "acme").unwrap(), 0, "once is enough");
        let _ = std::fs::remove_dir_all(&home);
    }
}
