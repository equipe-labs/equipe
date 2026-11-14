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

use equipe_protocol::{
    Access, Engine, Entry, ExecutionState, Member, Mode, Priority, SessionId, Ticket, TicketEntry, TicketExecution, TicketId, TicketKind, TicketStatus,
};
use rusqlite::{Connection, OptionalExtension as _, params};

pub use rusqlite::Error;
pub type Result<T> = std::result::Result<T, Error>;

/// What [`Journal::reserve_requeue`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requeued {
    /// Queued for its owner, under a new generation.
    Reserved,
    /// The same operation came before: nothing more.
    AlreadyApplied,
    /// Its run isn't stalled (or queued): nothing changed.
    NotStalled,
}

/// Whether `err` says there is no room left: the disk is full (SQLite's
/// "database or disk is full"), or a file couldn't be written for it.
pub fn is_full(err: &Error) -> bool {
    match err {
        Error::SqliteFailure(e, message) => {
            e.code == rusqlite::ErrorCode::DiskFull || message.as_deref().is_some_and(|m| m.contains("disk is full"))
        }
        Error::ToSqlConversionFailure(e) => {
            e.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == std::io::ErrorKind::StorageFull)
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

/// A conversation with the PM the person opened ([`Journal::record_conversation`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRecord {
    pub id: SessionId,
    pub project: Option<String>,
    pub engine_session: Option<String>,
    pub title: Option<String>,
}

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
    /// The board as last read whole, and the [`BoardVersion`] it was read at:
    /// [`Self::tickets`] reads it again only once a ticket or a run changed.
    board: std::cell::RefCell<Option<(BoardVersion, Vec<Ticket>)>>,
}

/// Where a squad's board stands: the changes this connection made to its
/// tickets and their runs (counted by temporary triggers), and SQLite's
/// `data_version`, which moves when another connection writes. The same
/// version, the same board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BoardVersion {
    ours: i64,
    others: i64,
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

/// A lane's own temporary folder (`TMPDIR`): `<home>/tmp/<squad>/t<ticket>` for
/// the session working a ticket, `<home>/tmp/<squad>/<member>` for a member at
/// its desk. Ticket numbers are per squad, so the squad is part of the path:
/// what is cleared when a ticket ends is exactly that squad's.
pub fn lane_tmp_dir(home: &Path, squad: &str, ticket: Option<TicketId>, member: &str) -> PathBuf {
    let base = home.join("tmp").join(squad);
    match ticket {
        Some(id) => base.join(format!("t{id}")),
        None => base.join(member),
    }
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

/// The most lines of a ticket's work [`Journal::ticket_lines`] reads.
pub const TICKET_WORK_MOST: usize = 4000;

/// The reason an active run carries while it is still starting: its
/// workspace is being made off the daemon's loop, and no session runs it yet
/// ([`Journal::set_execution_starting`]).
pub const STARTING: &str = "starting";

/// How the operation key of a run the board started (not a recovery the
/// person approved, nor a restart) begins.
pub const START_OPERATION: &str = "start:";

/// A ticket's work as its conversation kept it ([`Journal::ticket_lines`]).
#[derive(Debug, Clone, PartialEq)]
pub struct TicketWorkLines {
    pub session: SessionId,
    /// Where the conversation works, as last recorded.
    pub cwd: Option<PathBuf>,
    pub lines: Vec<(Entry, Option<u64>)>,
}

/// Where an editing member's ticket work is checked out, so it can be resumed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TicketWork {
    pub base: Option<String>,
    pub worktree: Option<PathBuf>,
    /// The engine's conversation for this ticket (Claude Code `--resume`).
    pub engine_session: Option<String>,
}

/// What a member's turn used: its engine and model, the engine's session it
/// ran in, and its tokens by kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnUse {
    pub engine: Engine,
    pub model: Option<String>,
    pub session: Option<String>,
    pub tokens: crate::protocol::plan_use::Tokens,
    /// Of `tokens.cache_write`, written to be kept an hour.
    pub cache_write_long: u64,
    pub calls: u64,
    /// The effort it ran at; `None`: the engine's own.
    pub effort: Option<String>,
}

impl TurnUse {
    /// From a turn's usage, as its engine reported it.
    pub fn of(engine: Engine, model: Option<String>, session: Option<String>, u: &crate::protocol::Usage) -> Self {
        // An engine that said only the sum: read from the cache, as most of it is.
        let (write, read) = match (u.cache_write_tokens, u.cache_read_tokens) {
            (None, None) => (0, u.cache_tokens.unwrap_or(0)),
            (write, read) => (write.unwrap_or(0), read.unwrap_or(0)),
        };
        TurnUse {
            engine,
            model,
            session,
            tokens: crate::protocol::plan_use::Tokens { input: u.input_tokens.unwrap_or(0), output: u.output_tokens.unwrap_or(0), cache_write: write, cache_read: read },
            cache_write_long: u.cache_write_long_tokens.unwrap_or(0).min(write),
            calls: u.calls.unwrap_or(1),
            effort: None,
        }
    }
}

/// A model choice kept for a ticket ([`Journal::record_model_choice`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoiceKept {
    /// `lane`, `escalated` or `person`.
    pub what: String,
    pub tier: Option<String>,
    pub by: String,
    /// The tier the rules asked for, where the ticket ran above it.
    pub shadow: Option<String>,
    /// What the rules made of the ticket by itself (`code`, `small`, `risk`…),
    /// whatever it ran as in the end; `None` for the PM and rows from before.
    pub classified: Option<String>,
}

/// A design QA ticket's way through the precheck, its gate and the
/// designer ([`Journal::design_qa_run`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesignQaRun {
    pub qa: TicketId,
    /// The ticket it checks.
    pub checked: TicketId,
    /// 1 for the first design QA of the ticket it checks.
    pub round: u32,
    /// `off`, `shadow` or `on`, as the squad's gate stood when it was made.
    pub gate: String,
    /// `skipped`, `running`, `done` or `failed`.
    pub precheck: String,
    /// What the gate decided (`auto_pass`, `send_back`, `designer`) — or, in
    /// shadow, would have.
    pub decision: Option<String>,
    /// Why the designer's judgment is needed ([`crate::design_qa::Judgment`]).
    pub judgment: Option<String>,
    /// The gate's decision was carried out.
    pub acted: bool,
    /// The report's folder.
    pub report: Option<String>,
    pub head: Option<String>,
    pub base: Option<String>,
    pub precheck_secs: Option<u64>,
    /// `pass`, `changes` or `cancelled`, once it ended.
    pub verdict: Option<String>,
    /// Who gave it: a designer, `equipe` (the gate), `you`.
    pub verdict_by: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    /// `per_ticket` or `risk_batch`, as the squad's mode stood when it was made.
    pub mode: Option<String>,
    /// In `risk_batch`: `refused`, `per_ticket` or `batch` ([`crate::design_qa::Route`]).
    pub route: Option<String>,
    /// Why it is high-risk, comma-separated ([`crate::design_qa::Risk`] codes).
    pub risk: Option<String>,
    /// The designer judged it in a light session: the report, no repository.
    pub lite: bool,
    /// The engineer's reason for waiving the precheck's findings.
    pub waived: Option<String>,
    /// The daily review that covered it.
    pub batch: Option<TicketId>,
    /// The audit that sampled it.
    pub audit: Option<TicketId>,
    /// What the audit found: `ok`, or `miss` (an S1/S2 the routing let through).
    pub audit_verdict: Option<String>,
}

/// A daily design review or audit ticket, as kept (`design_qa_batches`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesignQaBatch {
    pub ticket: TicketId,
    /// `review` or `audit`.
    pub kind: String,
    pub created_at: i64,
    /// How many changes it covers.
    pub items: u32,
    /// Its folder: main's precheck, the pictures handed to it.
    pub dir: Option<String>,
    /// Main's commit it looked at, per project: `project=commit` lines.
    pub heads: Option<String>,
    pub finished_at: Option<i64>,
    pub blockers: Option<u32>,
    pub fixes: Option<u32>,
    pub polish: Option<u32>,
}

/// What a turn ran on, as kept ([`Journal::ticket_turns`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRan {
    pub engine: Option<String>,
    pub model: Option<String>,
    /// `None`: the engine's own.
    pub effort: Option<String>,
}

/// How long turns and waits are kept for the retros: a quarter.
const KEPT_FOR_RETROS: i64 = 92 * 24 * 60 * 60;

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
        TicketStatus::Idea => "idea",
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
        "idea" => TicketStatus::Idea,
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
             );
             CREATE TABLE IF NOT EXISTS reviews (
               ticket INTEGER PRIMARY KEY,
               body TEXT
             );
             CREATE TABLE IF NOT EXISTS turns (
               member TEXT NOT NULL,
               role TEXT NOT NULL,
               ticket INTEGER,
               began INTEGER NOT NULL,
               ended INTEGER NOT NULL,
               outcome TEXT NOT NULL,
               tokens INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS turns_ended ON turns(ended);
             CREATE TABLE IF NOT EXISTS waits (
               kind TEXT NOT NULL,
               member TEXT NOT NULL,
               ticket INTEGER,
               what TEXT NOT NULL,
               asked INTEGER NOT NULL,
               answered INTEGER NOT NULL,
               outcome TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS conversations (
               id INTEGER PRIMARY KEY,
               project TEXT,
               engine_session TEXT,
               title TEXT,
               open INTEGER NOT NULL DEFAULT 1,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               cwd TEXT
             );
             CREATE TABLE IF NOT EXISTS ticket_executions (
               ticket INTEGER PRIMARY KEY,
               state TEXT NOT NULL,
               session INTEGER NOT NULL,
               operation TEXT NOT NULL UNIQUE,
               generation INTEGER NOT NULL,
               heartbeat_at INTEGER NOT NULL,
               lease_expires_at INTEGER NOT NULL,
               reason TEXT,
               updated_at INTEGER NOT NULL,
               head TEXT,
               dirty INTEGER
             );
             CREATE INDEX IF NOT EXISTS ticket_executions_lease
               ON ticket_executions(state, lease_expires_at);
             CREATE TABLE IF NOT EXISTS execution_operations (
               operation TEXT PRIMARY KEY,
               ticket INTEGER NOT NULL,
               generation INTEGER NOT NULL,
               at INTEGER NOT NULL
             );",
        )?;
        for column in ["role TEXT", "name TEXT", "joined INTEGER", "mode TEXT"] {
            let _ = conn.execute(&format!("ALTER TABLE sessions ADD COLUMN {column}"), []);
        }
        // Why an active run is paused, while it is (JSON, `RunPause`).
        for column in ["head TEXT", "dirty INTEGER", "pause TEXT"] {
            let _ = conn.execute(&format!("ALTER TABLE ticket_executions ADD COLUMN {column}"), []);
        }
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN merged INTEGER", []);
        let _ = conn.execute("ALTER TABLE conversations ADD COLUMN cwd TEXT", []);
        // Pinned by the person; put away by equipe itself ([`crate::tidy`]).
        for column in ["pinned INTEGER NOT NULL DEFAULT 0", "auto INTEGER NOT NULL DEFAULT 0"] {
            let _ = conn.execute(&format!("ALTER TABLE conversations ADD COLUMN {column}"), []);
        }
        // What a member's turn used, by kind, on which engine and model, in
        // which of the engine's sessions (`crate::plan_use`).
        for column in [
            "engine TEXT",
            "model TEXT",
            "session TEXT",
            "input INTEGER",
            "output INTEGER",
            "cache_write INTEGER",
            "cache_write_long INTEGER",
            "cache_read INTEGER",
            "calls INTEGER",
        ] {
            let _ = conn.execute(&format!("ALTER TABLE turns ADD COLUMN {column}"), []);
        }
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN off_main TEXT", []);
        // The person's say on how soon and in which order: normal and by number
        // for tickets from before.
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN priority TEXT NOT NULL DEFAULT 'normal'", []);
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN rank INTEGER NOT NULL DEFAULT 0", []);
        // What the person left open when they answered a wait (JSON list).
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN remaining TEXT", []);
        // The pictures a hand-in shows (JSON list of paths).
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN images TEXT", []);
        // The design QA ticket a hand-in waits on before the person's review.
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN design_qa INTEGER", []);
        // The plan it was cut from, or reviews: its path.
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN plan TEXT", []);
        // The model and effort its lanes run on, and why (JSON, `tiers::TicketModel`).
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN model TEXT", []);
        // What sort of work it is, in a word (`protocol::title::TicketLabel`).
        let _ = conn.execute("ALTER TABLE tickets ADD COLUMN label TEXT", []);
        // The effort a member's turn ran at; `NULL`: the engine's own, or a turn from before.
        let _ = conn.execute("ALTER TABLE turns ADD COLUMN effort TEXT", []);
        // How turns on tickets ended when they didn't end as they should: no
        // hand-in, a failed engine, a plan's limit (`crate::runtime` counts them).
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS turn_endings (at INTEGER NOT NULL, kind TEXT NOT NULL, ticket INTEGER, member TEXT)",
            [],
        );
        // Conversations that went on in a fresh engine session at a context
        // budget (`crate::runtime`'s `hygiene`): counted for the usage report.
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS context_rotations (at INTEGER NOT NULL, session INTEGER NOT NULL, member TEXT, ticket INTEGER, kind TEXT NOT NULL, tokens INTEGER NOT NULL)",
            [],
        );
        // Work sent back, and why: what decides whether a stronger model would
        // help (`tiers::after_send_back`). Send-backs from before have none:
        // none counted.
        // Every model choice a lane started with, the cheaper tier the rules
        // would have picked beside it, and every escalation: kept to see which
        // defaults are too low or too high (`crate::tiers`).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS send_backs (
               ticket INTEGER NOT NULL,
               at INTEGER NOT NULL,
               by TEXT NOT NULL,
               reason TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS send_backs_ticket ON send_backs(ticket);
             CREATE TABLE IF NOT EXISTS model_choices (
               ticket INTEGER NOT NULL,
               at INTEGER NOT NULL,
               what TEXT NOT NULL,
               member TEXT,
               role TEXT,
               engine TEXT,
               tier TEXT,
               model TEXT,
               effort TEXT,
               by TEXT NOT NULL,
               reason TEXT NOT NULL,
               shadow TEXT
             );
             CREATE INDEX IF NOT EXISTS model_choices_ticket ON model_choices(ticket);",
        )?;
        // What the rules made of a ticket by itself, beside the reason it ran on its tier
        // (an escalation, the person's choice, the member's floor): kept to see which kinds
        // of tickets run on which tier.
        let _ = conn.execute("ALTER TABLE model_choices ADD COLUMN classified TEXT", []);
        // Each design QA ticket's way through the precheck, its gate and the
        // designer, for measuring them (`crate::design::qa_stats`).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS design_qa_runs (
               qa INTEGER PRIMARY KEY,
               checked INTEGER NOT NULL,
               round INTEGER NOT NULL,
               gate TEXT NOT NULL,
               precheck TEXT NOT NULL,
               decision TEXT,
               judgment TEXT,
               acted INTEGER NOT NULL DEFAULT 0,
               report TEXT,
               head TEXT,
               base TEXT,
               precheck_secs INTEGER,
               verdict TEXT,
               verdict_by TEXT,
               created_at INTEGER NOT NULL,
               finished_at INTEGER
             );",
        )?;
        // Risk-based design QA (`crate::design_qa::route`): how each was routed, and the reviews and audits.
        for column in ["mode TEXT", "route TEXT", "risk TEXT", "lite INTEGER NOT NULL DEFAULT 0", "waived TEXT", "batch INTEGER", "audit INTEGER", "audit_verdict TEXT"] {
            let _ = conn.execute(&format!("ALTER TABLE design_qa_runs ADD COLUMN {column}"), []);
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS design_qa_batches (
               ticket INTEGER PRIMARY KEY,
               kind TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               items INTEGER NOT NULL,
               dir TEXT,
               heads TEXT,
               finished_at INTEGER,
               blockers INTEGER,
               fixes INTEGER,
               polish INTEGER
             );",
        )?;
        // Why the person was asked to sign in, and sign-ins finished (`crate::logins`).
        crate::logins::create_table(&conn)?;
        // Tickets from before kinds: `code` with a project, `task` without — once,
        // when the column arrives.
        if conn.execute("ALTER TABLE tickets ADD COLUMN kind TEXT NOT NULL DEFAULT 'code'", []).is_ok() {
            conn.execute("UPDATE tickets SET kind = 'task' WHERE project IS NULL", [])?;
        }
        if NO_ROOM.load(std::sync::atomic::Ordering::Relaxed) {
            // No bigger than it is now: SQLite answers SQLITE_FULL, as on a full disk.
            conn.query_row("PRAGMA max_page_count = 1", [], |_| Ok(()))?;
        }
        // This connection's changes to the board, counted (`BoardVersion`):
        // temporary, so nothing is written to the squad's file. Without them
        // the board is read afresh every time, as before.
        let mut counted = String::from(
            "CREATE TEMP TABLE IF NOT EXISTS board_writes (n INTEGER NOT NULL);
             INSERT INTO board_writes SELECT 0 WHERE NOT EXISTS (SELECT 1 FROM board_writes);",
        );
        for table in ["tickets", "ticket_executions"] {
            for change in ["INSERT", "UPDATE", "DELETE"] {
                counted.push_str(&format!(
                    "CREATE TEMP TRIGGER IF NOT EXISTS board_{table}_{name} AFTER {change} ON main.{table} BEGIN UPDATE board_writes SET n = n + 1; END;",
                    name = change.to_ascii_lowercase()
                ));
            }
        }
        let _ = conn.execute_batch(&counted);
        Ok(Self { conn, path: path.to_owned(), board: std::cell::RefCell::new(None) })
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

    /// The members the person turned the Mac's desktop on for by name
    /// (before it was everyone's by default, or before the switch was on).
    pub fn desktop_members(&self) -> Vec<String> {
        self.names("desktop_members")
    }

    /// The members the person turned the desktop off for.
    pub fn desktop_off(&self) -> Vec<String> {
        self.names("desktop_off")
    }

    /// Whether the person turned the desktop off for the whole squad.
    pub fn desktop_off_for_everyone(&self) -> bool {
        self.meta("desktop_everyone").as_deref() == Some("off")
    }

    fn names(&self, key: &str) -> Vec<String> {
        let value: serde_json::Value = self.meta(key).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        value.as_array().into_iter().flatten().filter_map(|m| m.as_str().map(str::to_owned)).collect()
    }

    /// Whether `member` works the Mac's desktop. Everyone does by default
    /// once the person's Accessibility switch is on (`granted`), unless the
    /// person turned it off for them or for the squad; a member turned on by
    /// name does either way (the switch is then asked for).
    pub fn desktop_enabled(&self, member: &str, granted: bool) -> bool {
        if self.desktop_members().iter().any(|m| m == member) {
            return true;
        }
        granted && !self.desktop_off_for_everyone() && !self.desktop_off().iter().any(|m| m == member)
    }

    /// Turn the desktop on or off for `member`.
    pub fn set_desktop(&self, member: &str, on: bool) -> Result<()> {
        let mut members: Vec<String> = self.desktop_members().into_iter().filter(|m| m != member).collect();
        let mut off: Vec<String> = self.desktop_off().into_iter().filter(|m| m != member).collect();
        if on {
            members.push(member.to_owned());
        } else {
            off.push(member.to_owned());
        }
        self.set_meta("desktop_members", &serde_json::json!(members).to_string())?;
        self.set_meta("desktop_off", &serde_json::json!(off).to_string())
    }

    /// Turn the desktop on or off for everyone on the squad: off, only members
    /// turned on by name work it; on again, everyone does, and the names
    /// turned off one by one are forgotten.
    pub fn set_desktop_everyone(&self, on: bool) -> Result<()> {
        if on {
            self.set_meta("desktop_off", "[]")?;
        }
        self.set_meta("desktop_everyone", if on { "on" } else { "off" })
    }

    /// The members the person rested (`Command::SetResting`), by name: they
    /// take no new work until woken. Nobody by default.
    pub fn resting(&self) -> Vec<String> {
        self.meta("resting").and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    /// Whether `member` rests.
    pub fn is_resting(&self, member: &str) -> bool {
        self.resting().iter().any(|m| m == member)
    }

    /// Rest `member`, or wake it; the members resting after.
    pub fn set_resting(&self, member: &str, on: bool) -> Result<Vec<String>> {
        let mut members: Vec<String> = self.resting().into_iter().filter(|m| m != member).collect();
        if on {
            members.push(member.to_owned());
        }
        self.set_meta("resting", &serde_json::json!(members).to_string())?;
        Ok(members)
    }

    /// The tickets the person pinned to watch on the Status screen, in the
    /// order they pinned them.
    pub fn watched(&self) -> Vec<TicketId> {
        self.meta("watched").and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    /// Pin `ticket` to watch, or let it go; the tickets pinned after.
    pub fn set_watched(&self, ticket: TicketId, on: bool) -> Result<Vec<TicketId>> {
        let mut pinned: Vec<TicketId> = self.watched().into_iter().filter(|t| *t != ticket).collect();
        if on {
            pinned.push(ticket);
        }
        self.set_meta("watched", &serde_json::json!(pinned).to_string())?;
        Ok(pinned)
    }

    /// When the PM was last asked about the person's stale waits (unix seconds).
    pub fn stale_waits_asked_at(&self) -> Option<u64> {
        self.meta("stale_waits_asked_at").and_then(|at| at.parse().ok())
    }

    pub fn set_stale_waits_asked_at(&self, at: u64) -> Result<()> {
        self.set_meta("stale_waits_asked_at", &at.to_string())
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

    /// How much debug information members' Rust builds carry: lean unless
    /// the person said (`equipe disk debug`).
    pub fn build_debug(&self) -> crate::disk::BuildDebug {
        self.meta("build_debug").and_then(|v| crate::disk::BuildDebug::parse(&v)).unwrap_or_default()
    }

    pub fn set_build_debug(&self, debug: crate::disk::BuildDebug) -> Result<()> {
        self.set_meta("build_debug", debug.word())
    }

    /// The looks the squad's screens told, a machine each (`crate::looks`).
    pub fn screen_looks(&self) -> crate::looks::Looks {
        self.meta(crate::looks::KEY).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    pub fn set_screen_looks(&self, looks: &crate::looks::Looks) -> Result<()> {
        self.set_meta(crate::looks::KEY, &serde_json::to_string(looks).unwrap_or_default())
    }

    /// The person turned off guesses at their next message.
    pub fn suggestions_off(&self) -> bool {
        self.meta("suggestions").as_deref() == Some("off")
    }

    pub fn set_suggestions(&self, on: bool) -> Result<()> {
        self.set_meta("suggestions", if on { "on" } else { "off" })
    }

    /// How the squad moves members between engines as their plans run low.
    pub fn engine_switching(&self) -> equipe_protocol::engines::Switching {
        self.meta("engine_switching").and_then(|v| equipe_protocol::engines::Switching::parse(&v)).unwrap_or_default()
    }

    pub fn set_engine_switching(&self, mode: equipe_protocol::engines::Switching) -> Result<()> {
        self.set_meta("engine_switching", mode.code())
    }

    /// Whether the PM accepts and merges the team's handed-in work itself —
    /// unless the person said no, it does.
    /// Secrets members asked the person for and still wait, as JSON.
    pub fn secret_asks(&self) -> Option<String> {
        self.meta("secret_asks")
    }

    pub fn set_secret_asks(&self, json: &str) -> Result<()> {
        self.set_meta("secret_asks", json)
    }

    /// The facts of the keys, tokens and passwords this squad's members kept
    /// (`crate::squad_secrets`), as JSON — never a value, those live in the
    /// secret store.
    pub fn squad_secrets(&self) -> Option<String> {
        self.meta("squad_secrets")
    }

    pub fn set_squad_secrets(&self, json: &str) -> Result<()> {
        self.set_meta("squad_secrets", json)
    }

    /// Every use of a kept key, newest last, as JSON: who, on which ticket
    /// and project, when — never the value.
    pub fn secret_uses(&self) -> Option<String> {
        self.meta("secret_uses")
    }

    pub fn set_secret_uses(&self, json: &str) -> Result<()> {
        self.set_meta("secret_uses", json)
    }

    /// Questions put to the person about a kept key's first use on a project
    /// that still wait, as JSON (`crate::squad_secrets::Grant`).
    pub fn secret_grants(&self) -> Option<String> {
        self.meta("secret_grants")
    }

    pub fn set_secret_grants(&self, json: &str) -> Result<()> {
        self.set_meta("secret_grants", json)
    }

    pub fn pm_accepts(&self) -> bool {
        self.meta("pm_accepts").as_deref() != Some("off")
    }

    pub fn set_pm_accepts(&self, on: bool) -> Result<()> {
        self.set_meta("pm_accepts", if on { "on" } else { "off" })
    }

    /// "All by words": the person's own words decide high-risk requests too,
    /// read back and confirmed. Off unless they turned it on, at the Mac.
    pub fn words_all(&self) -> bool {
        self.meta("words_all").as_deref() == Some("on")
    }

    pub fn set_words_all(&self, on: bool) -> Result<()> {
        self.set_meta("words_all", if on { "on" } else { "off" })
    }

    /// How many subagents each member runs at once, where its engine has
    /// them, as the person set it; `None`: each engine's own.
    pub fn subagents_at_once(&self) -> Option<u32> {
        self.meta("subagents_at_once").and_then(|n| n.parse().ok()).map(equipe_protocol::subagents_at_once)
    }

    pub fn set_subagents_at_once(&self, at_once: Option<u32>) -> Result<()> {
        self.set_meta("subagents_at_once", &at_once.map(|n| equipe_protocol::subagents_at_once(n).to_string()).unwrap_or_default())
    }

    /// How many tickets the squad works at once: auto (by the plans) unless
    /// the person set max, its caps within bounds.
    pub fn parallel(&self) -> equipe_protocol::lanes::Parallel {
        self.meta("parallel").and_then(|v| serde_json::from_str::<equipe_protocol::lanes::Parallel>(&v).ok()).map(|p| p.clamped()).unwrap_or_default()
    }

    pub fn set_parallel(&self, parallel: equipe_protocol::lanes::Parallel) -> Result<()> {
        self.set_meta("parallel", &serde_json::to_string(&parallel.clamped()).unwrap_or_default())
    }

    /// Which engine reviews the designer's work; auto unless the person set it.
    pub fn design_reviewer(&self) -> equipe_protocol::engines::Reviewer {
        self.meta("design_reviewer").and_then(|v| equipe_protocol::engines::Reviewer::parse(&v)).unwrap_or_default()
    }

    pub fn set_design_reviewer(&self, reviewer: equipe_protocol::engines::Reviewer) -> Result<()> {
        self.set_meta("design_reviewer", reviewer.code())
    }

    /// What new members of a role start with on an engine, where the person
    /// set it; every other role takes equipe's recommendation.
    pub fn role_models(&self) -> Vec<equipe_protocol::RoleModel> {
        self.meta("role_models").and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    /// Keep `default` for its role and engine: one with neither a model nor
    /// an effort goes back to the recommendation.
    pub fn set_role_model(&self, default: equipe_protocol::RoleModel) -> Result<Vec<equipe_protocol::RoleModel>> {
        let mut all = self.role_models();
        all.retain(|d| (d.role.as_str(), d.engine) != (default.role.as_str(), default.engine));
        if default.model.is_some() || default.effort.is_some() {
            all.push(default);
        }
        all.sort_by(|a, b| (a.role.as_str(), a.engine as u8).cmp(&(b.role.as_str(), b.engine as u8)));
        self.set_meta("role_models", &serde_json::to_string(&all).unwrap_or_default())?;
        Ok(all)
    }

    /// What engine switching keeps for the squad (`runtime/engines.rs`), as JSON.
    pub fn engines_record(&self) -> Option<String> {
        self.meta("engines")
    }

    pub fn set_engines_record(&self, json: &str) -> Result<()> {
        self.set_meta("engines", json)
    }

    /// The hand-over packet session `id` opened with, after a move to the
    /// other engine (`runtime/engines.rs`), as it was sent.
    pub fn hand_over_packet(&self, id: SessionId) -> Option<String> {
        self.meta(&format!("hand_over:{id}"))
    }

    pub fn set_hand_over_packet(&self, id: SessionId, packet: &str) -> Result<()> {
        self.set_meta(&format!("hand_over:{id}"), packet)
    }

    /// A packet no move on record names any more.
    pub fn forget_hand_over_packet(&self, id: SessionId) -> Result<()> {
        self.conn.execute("DELETE FROM meta WHERE key = ?1", params![format!("hand_over:{id}")])?;
        Ok(())
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

    /// The latest durable runtime generation for a ticket.
    pub fn execution(&self, ticket: TicketId) -> Result<Option<TicketExecution>> {
        self.conn
            .query_row(
                "SELECT state, session, operation, generation, heartbeat_at, lease_expires_at, reason, head, dirty, pause
                 FROM ticket_executions WHERE ticket = ?1",
                [ticket],
                |r| {
                    Ok(TicketExecution {
                        state: ExecutionState::parse(&r.get::<_, String>(0)?),
                        session: r.get::<_, i64>(1)?.max(0) as u64,
                        operation: r.get(2)?,
                        generation: r.get::<_, i64>(3)?.max(0) as u64,
                        heartbeat_at: r.get::<_, i64>(4)?.max(0) as u64,
                        lease_expires_at: r.get::<_, i64>(5)?.max(0) as u64,
                        reason: r.get(6)?,
                        pause: r.get::<_, Option<String>>(9)?.and_then(|p| serde_json::from_str(&p).ok()),
                        head: r.get(7)?,
                        dirty: r.get::<_, Option<i64>>(8)?.map(|n| n.clamp(0, i64::from(u32::MAX)) as u32),
                    })
                },
            )
            .optional()
    }

    /// Start a run of `ticket` for runtime `session`: its generation, or
    /// `None` when another run holds a valid lease or `operation` was used
    /// already. A recovery the person approved ([`Self::reserve_requeue`])
    /// starts with its own operation and keeps the generation it reserved;
    /// any other start takes the next one.
    pub fn acquire_execution(&self, ticket: TicketId, session: SessionId, operation: &str, at: u64, lease_for: u64) -> Result<Option<u64>> {
        let tx = self.conn.unchecked_transaction()?;
        let lease = at.saturating_add(lease_for) as i64;
        let queued: Option<i64> = tx
            .query_row(
                "SELECT generation FROM ticket_executions WHERE ticket = ?1 AND operation = ?2 AND state = 'queued'",
                params![ticket, operation],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(generation) = queued {
            tx.execute(
                "UPDATE ticket_executions SET state = 'active', session = ?1, heartbeat_at = ?2,
                   lease_expires_at = ?3, reason = NULL, head = NULL, dirty = NULL, pause = NULL, updated_at = ?2
                 WHERE ticket = ?4 AND operation = ?5 AND state = 'queued'",
                params![session as i64, at as i64, lease, ticket, operation],
            )?;
            tx.commit()?;
            return Ok(Some(generation.max(0) as u64));
        }
        let used = tx.query_row("SELECT 1 FROM execution_operations WHERE operation = ?1", [operation], |_| Ok(())).optional()?.is_some();
        let busy = tx
            .query_row(
                "SELECT 1 FROM ticket_executions WHERE ticket = ?1 AND state = 'active' AND lease_expires_at > ?2",
                params![ticket, at as i64],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if used || busy {
            tx.commit()?;
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO ticket_executions
               (ticket, state, session, operation, generation, heartbeat_at, lease_expires_at, reason, updated_at, head, dirty)
             VALUES (?1, 'active', ?2, ?3,
               COALESCE((SELECT generation + 1 FROM ticket_executions WHERE ticket = ?1), 1), ?4, ?5, NULL, ?4, NULL, NULL)
             ON CONFLICT(ticket) DO UPDATE SET state = excluded.state, session = excluded.session,
               operation = excluded.operation, generation = excluded.generation,
               heartbeat_at = excluded.heartbeat_at, lease_expires_at = excluded.lease_expires_at,
               reason = NULL, updated_at = excluded.updated_at, head = NULL, dirty = NULL, pause = NULL",
            params![ticket, session as i64, operation, at as i64, lease],
        )?;
        let generation: i64 = tx.query_row("SELECT generation FROM ticket_executions WHERE ticket = ?1", [ticket], |r| r.get(0))?;
        tx.execute(
            "INSERT OR IGNORE INTO execution_operations (operation, ticket, generation, at) VALUES (?1, ?2, ?3, ?4)",
            params![operation, ticket, generation, at as i64],
        )?;
        tx.commit()?;
        Ok(Some(generation.max(0) as u64))
    }

    /// Reserve a recovery the person approved, in one transaction: the run is
    /// queued under a new generation, and the ticket goes back to `todo` for
    /// `owner` (left as it is when `None`). The ticket's note — the latest
    /// send-back or design QA reason, which the next run is told first — its
    /// open blockers, branch and worktree are kept as they are. Only a stalled
    /// run, or one already queued, is recovered; an operation used before
    /// changes nothing, however late it comes.
    pub fn reserve_requeue(&self, ticket: TicketId, operation: &str, owner: Option<&str>, at: u64) -> Result<Requeued> {
        let tx = self.conn.unchecked_transaction()?;
        if tx.query_row("SELECT 1 FROM execution_operations WHERE operation = ?1", [operation], |_| Ok(())).optional()?.is_some() {
            tx.commit()?;
            return Ok(Requeued::AlreadyApplied);
        }
        let state: Option<(String, String)> = tx
            .query_row(
                "SELECT e.state, t.status FROM ticket_executions e JOIN tickets t ON t.id = e.ticket WHERE e.ticket = ?1",
                [ticket],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let recoverable = matches!(state.as_ref().map(|(e, t)| (e.as_str(), t.as_str())), Some(("stalled", "in_progress")) | Some(("queued", "todo")));
        if !recoverable {
            tx.commit()?;
            return Ok(Requeued::NotStalled);
        }
        tx.execute(
            "UPDATE ticket_executions SET state = 'queued', session = 0, operation = ?1,
               generation = generation + 1, heartbeat_at = ?2, lease_expires_at = 0,
               reason = NULL, pause = NULL, updated_at = ?2 WHERE ticket = ?3",
            params![operation, at as i64, ticket],
        )?;
        tx.execute(
            "UPDATE tickets SET status = 'todo', assignee = NULL, owner = COALESCE(?1, owner), updated_at = ?2 WHERE id = ?3",
            params![owner, at as i64, ticket],
        )?;
        tx.execute(
            "INSERT INTO execution_operations (operation, ticket, generation, at)
             VALUES (?1, ?2, (SELECT generation FROM ticket_executions WHERE ticket = ?2), ?3)",
            params![operation, ticket, at as i64],
        )?;
        tx.commit()?;
        Ok(Requeued::Reserved)
    }

    /// Renew only the exact generation/session that emitted the heartbeat.
    pub fn heartbeat_execution(&self, ticket: TicketId, generation: u64, session: SessionId, at: u64, lease_for: u64) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE ticket_executions SET heartbeat_at = ?1, lease_expires_at = ?2, updated_at = ?1
             WHERE ticket = ?3 AND generation = ?4 AND session = ?5 AND state = 'active'",
            params![at as i64, at.saturating_add(lease_for) as i64, ticket, generation as i64, session as i64],
        )? == 1)
    }

    /// Close generation `generation` of `ticket`'s run as the workflow ended
    /// it; a newer generation is left alone.
    pub fn finish_execution(&self, ticket: TicketId, generation: u64, state: ExecutionState, reason: Option<&str>) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE ticket_executions SET state = ?1, lease_expires_at = 0, reason = ?2, pause = NULL, updated_at = ?3
             WHERE ticket = ?4 AND generation = ?5",
            params![state.code(), reason, now(), ticket, generation as i64],
        )? == 1)
    }

    /// The minute's check: active runs whose runtime session isn't among
    /// `live` and whose lease ran out by `at` are stalled. A live session is
    /// never stalled, whatever the clock says. Returns the tickets stalled,
    /// for the runtime to let go of; their work, checkout and files are
    /// untouched.
    pub fn recover_executions(&self, live: &[SessionId], at: u64) -> Result<Vec<TicketId>> {
        let tx = self.conn.unchecked_transaction()?;
        let stale: Vec<(TicketId, i64)> = {
            let mut stmt = tx.prepare("SELECT ticket, session FROM ticket_executions WHERE state = 'active' AND lease_expires_at <= ?1")?;
            let rows = stmt.query_map([at as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        let mut stalled = Vec::new();
        for (ticket, session) in stale {
            if live.contains(&(session.max(0) as u64)) {
                continue;
            }
            tx.execute(
                "UPDATE ticket_executions SET state = 'stalled', lease_expires_at = 0, reason = ?1, pause = NULL, updated_at = ?2
                 WHERE ticket = ?3 AND state = 'active'",
                params![equipe_protocol::stall_reason::LEASE_EXPIRED, at as i64, ticket],
            )?;
            stalled.push(ticket);
        }
        tx.commit()?;
        Ok(stalled)
    }

    /// A daemon start: no runtime session outlives the process that ran it,
    /// so every active run is stalled (`not_resumed`), and the runtime starts
    /// a new generation for each one it carries on
    /// ([`Self::acquire_execution`]). In-progress tickets from before runs
    /// were kept are recorded the same way (`migrated`, generation 0). Safe
    /// to run again; tickets, their work and their checkouts are untouched.
    pub fn stall_all_executions(&self, at: u64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE ticket_executions SET state = 'stalled', lease_expires_at = 0, reason = ?1, pause = NULL, updated_at = ?2 WHERE state = 'active'",
            params![equipe_protocol::stall_reason::NOT_RESUMED, at as i64],
        )?;
        tx.execute(
            "INSERT INTO ticket_executions
               (ticket, state, session, operation, generation, heartbeat_at, lease_expires_at, reason, updated_at)
             SELECT id, 'stalled', 0, 'migration:' || id, 0, ?1, 0, ?2, ?1
             FROM tickets WHERE status = 'in_progress'
               AND NOT EXISTS (SELECT 1 FROM ticket_executions e WHERE e.ticket = tickets.id)",
            params![at as i64, equipe_protocol::stall_reason::MIGRATED],
        )?;
        tx.commit()
    }

    /// `ticket`'s run, still active although nothing runs it any more — its
    /// member left the squad — stalled now (`reason`), so that it can be
    /// requeued ([`Self::reserve_requeue`]). Whether it was active.
    pub fn stall_execution(&self, ticket: TicketId, reason: &str, at: u64) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE ticket_executions SET state = 'stalled', lease_expires_at = 0, reason = ?1, pause = NULL, updated_at = ?2
             WHERE ticket = ?3 AND state = 'active'",
            params![reason, at as i64, ticket],
        )? == 1)
    }

    /// Say why generation `generation` of `ticket`'s active run is paused —
    /// or, with `None`, that it goes on. Whether that very run was active.
    pub fn set_execution_pause(&self, ticket: TicketId, generation: u64, pause: Option<&equipe_protocol::RunPause>) -> Result<bool> {
        let pause = pause.and_then(|p| serde_json::to_string(p).ok());
        Ok(self.conn.execute(
            "UPDATE ticket_executions SET pause = ?1, updated_at = ?2 WHERE ticket = ?3 AND generation = ?4 AND state = 'active'",
            params![pause, now(), ticket, generation as i64],
        )? == 1)
    }

    /// Count a turn on a ticket that ended other than as it should (`kind`:
    /// [`crate::runtime::turn_ending`]'s codes), for the squad's numbers.
    pub fn count_turn_ending(&self, kind: &str, ticket: Option<TicketId>, member: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO turn_endings (at, kind, ticket, member) VALUES (?1, ?2, ?3, ?4)",
            params![now(), kind, ticket, member],
        )?;
        self.conn.execute("DELETE FROM turn_endings WHERE at < ?1", [now() - KEPT_FOR_RETROS])?;
        Ok(())
    }

    /// How many turns ended each way since `since`, by kind, most first.
    pub fn turn_endings_since(&self, since: u64) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare("SELECT kind, COUNT(*) FROM turn_endings WHERE at >= ?1 GROUP BY kind ORDER BY COUNT(*) DESC, kind")?;
        let rows = stmt.query_map([since as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64)))?;
        rows.collect()
    }

    /// Whether the squad's long conversations go on in a fresh engine session
    /// at a context budget (`equipe context-config --rotation`); on unless set off.
    pub fn context_rotation(&self) -> bool {
        self.meta("context_rotation").is_none_or(|v| v != "off")
    }

    pub fn set_context_rotation(&self, on: bool) -> Result<()> {
        self.set_meta("context_rotation", if on { "on" } else { "off" })
    }

    /// Whether equipe's own tools cap very long answers
    /// (`equipe context-config --tool-caps`); on unless set off.
    pub fn context_tool_caps(&self) -> bool {
        self.meta("context_tool_caps").is_none_or(|v| v != "off")
    }

    pub fn set_context_tool_caps(&self, on: bool) -> Result<()> {
        self.set_meta("context_tool_caps", if on { "on" } else { "off" })
    }

    /// Whether a ticket whose blockers were cancelled, or that repeats another
    /// open one, is held for the PM before it starts, and a run is stopped when
    /// its ticket is cancelled (`equipe context-config --obsolete-check`); on unless set off.
    pub fn context_obsolete_check(&self) -> bool {
        self.meta("context_obsolete_check").is_none_or(|v| v != "off")
    }

    pub fn set_context_obsolete_check(&self, on: bool) -> Result<()> {
        self.set_meta("context_obsolete_check", if on { "on" } else { "off" })
    }

    /// Whether the engines' own auto-compaction is asked to start at the
    /// squad's context budgets (`equipe context-config --native-compact`); on unless set off.
    pub fn context_native_compact(&self) -> bool {
        self.meta("context_native_compact").is_none_or(|v| v != "off")
    }

    pub fn set_context_native_compact(&self, on: bool) -> Result<()> {
        self.set_meta("context_native_compact", if on { "on" } else { "off" })
    }

    /// Whether members may run on a 1M-token context window: `false` (the
    /// standard window, so the engine's own compaction comes early) unless the
    /// squad said `large` (`equipe context-config --window large`). A ticket
    /// can ask for it alone ([`crate::runtime::wants_large_window`]).
    pub fn context_window_large(&self) -> bool {
        self.meta("context_window").is_some_and(|v| v == "large")
    }

    pub fn set_context_window_large(&self, large: bool) -> Result<()> {
        self.set_meta("context_window", if large { "large" } else { "standard" })
    }

    /// Keep a rotation: conversation `session` went on in a fresh engine
    /// session at `tokens` of context (`kind`: `desk` or `ticket`).
    pub fn record_rotation(&self, session: SessionId, member: Option<&str>, ticket: Option<TicketId>, kind: &str, tokens: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO context_rotations (at, session, member, ticket, kind, tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![now(), session as i64, member, ticket, kind, tokens as i64],
        )?;
        self.conn.execute("DELETE FROM context_rotations WHERE at < ?1", [now() - KEPT_FOR_RETROS])?;
        Ok(())
    }

    /// The rotations since `since`, by kind: how many, and the context they
    /// started from, in all.
    pub fn rotations_since(&self, since: u64) -> Result<Vec<(String, u64, u64)>> {
        let mut stmt = self.conn.prepare("SELECT kind, COUNT(*), COALESCE(SUM(tokens), 0) FROM context_rotations WHERE at >= ?1 GROUP BY kind ORDER BY kind")?;
        let rows = stmt.query_map([since as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64, r.get::<_, i64>(2)?.max(0) as u64)))?;
        rows.collect()
    }

    /// Mark generation `generation` of `ticket`'s active run as still starting
    /// — its workspace being made off the daemon's loop ([`STARTING`]) — or,
    /// with `starting` false, as started. Whether that very run was active.
    pub fn set_execution_starting(&self, ticket: TicketId, generation: u64, starting: bool) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE ticket_executions SET reason = ?1, updated_at = ?2 WHERE ticket = ?3 AND generation = ?4 AND state = 'active'",
            params![starting.then_some(STARTING), now(), ticket, generation as i64],
        )? == 1)
    }

    /// A daemon start, before anything else looks at runs: a run still
    /// starting when the daemon stopped never had a session, so nothing
    /// carries it on. Its ticket goes back on the board (`todo`, no
    /// assignee; owner, note, branch and work kept), and the run is let go:
    /// a recovery the person approved is queued again under the operation and
    /// generation it reserved, to start exactly once; any other start is
    /// stopped (`not_started`). What its workspace held half-made is left to
    /// [`crate::worktree::tidy`]. Returns the tickets put back.
    pub fn release_starting_executions(&self, at: u64) -> Result<Vec<TicketId>> {
        let tx = self.conn.unchecked_transaction()?;
        let starting: Vec<(TicketId, String)> = {
            let mut stmt = tx.prepare("SELECT ticket, operation FROM ticket_executions WHERE state = 'active' AND reason = ?1")?;
            let rows = stmt.query_map([STARTING], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        for (ticket, operation) in &starting {
            if operation.starts_with(START_OPERATION) {
                tx.execute(
                    "UPDATE ticket_executions SET state = 'stopped', lease_expires_at = 0, reason = 'not_started', updated_at = ?1 WHERE ticket = ?2",
                    params![at as i64, ticket],
                )?;
            } else {
                tx.execute(
                    "UPDATE ticket_executions SET state = 'queued', session = 0, lease_expires_at = 0, reason = NULL, updated_at = ?1 WHERE ticket = ?2",
                    params![at as i64, ticket],
                )?;
            }
            tx.execute(
                "UPDATE tickets SET status = 'todo', assignee = NULL, updated_at = ?1 WHERE id = ?2 AND status = 'in_progress'",
                params![at as i64, ticket],
            )?;
        }
        tx.commit()?;
        Ok(starting.into_iter().map(|(ticket, _)| ticket).collect())
    }

    /// What a stalled run left in its checkout — its commit and how many
    /// files are changed or untracked — for the person's recovery card.
    pub fn set_stalled_checkout(&self, ticket: TicketId, head: Option<&str>, dirty: Option<u32>) -> Result<()> {
        self.conn.execute(
            "UPDATE ticket_executions SET head = ?1, dirty = ?2 WHERE ticket = ?3 AND state = 'stalled'",
            params![head, dirty.map(i64::from), ticket],
        )?;
        Ok(())
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

    /// The squad's HQ: the project its operations live in — the plan across
    /// the products, decisions, runbooks, reports. Only while it is one of
    /// the squad's projects.
    pub fn hq(&self) -> Option<PathBuf> {
        let hq = PathBuf::from(self.meta("hq").filter(|h| !h.is_empty())?);
        self.projects().ok()?.contains(&hq).then_some(hq)
    }

    /// The squad's routines, by number.
    pub fn routines(&self) -> Vec<crate::protocol::routines::Routine> {
        self.meta("routines").and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
    }

    pub fn set_routines(&self, routines: &[crate::protocol::routines::Routine]) -> Result<()> {
        self.set_meta("routines", &serde_json::to_string(routines).unwrap_or_else(|_| "[]".into()))
    }

    /// The squad's signals (`crate::signals`), in the order turned on.
    pub fn signals(&self) -> Vec<crate::signals::Signal> {
        self.meta("signals").and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
    }

    pub fn set_signals(&self, signals: &[crate::signals::Signal]) -> Result<()> {
        self.set_meta("signals", &serde_json::to_string(signals).unwrap_or_else(|_| "[]".into()))
    }

    /// Each signal's readings and whether its failing was told, by its key.
    pub fn signal_states(&self) -> std::collections::BTreeMap<String, crate::signals::State> {
        self.meta("signal_states").and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
    }

    pub fn set_signal_states(&self, states: &std::collections::BTreeMap<String, crate::signals::State>) -> Result<()> {
        self.set_meta("signal_states", &serde_json::to_string(states).unwrap_or_else(|_| "{}".into()))
    }

    /// Hours an empty board waits before the PM is woken; 0: never.
    pub fn idle_wake_hours(&self) -> u32 {
        self.meta("idle_wake_hours").and_then(|h| h.parse().ok()).unwrap_or(crate::protocol::routines::IDLE_WAKE_HOURS)
    }

    pub fn set_idle_wake_hours(&self, hours: u32) -> Result<()> {
        self.set_meta("idle_wake_hours", &hours.to_string())
    }

    /// How this squad escalates what stays a bottleneck, as the person set it
    /// for it; `None`: they did not, and their default (or the built-in one) stands.
    pub fn notify_settings(&self) -> Option<crate::protocol::importance::NotifySettings> {
        self.meta("notify").and_then(|json| serde_json::from_str(&json).ok())
    }

    pub fn set_notify_settings(&self, settings: &crate::protocol::importance::NotifySettings) -> Result<()> {
        self.set_meta("notify", &serde_json::to_string(settings).unwrap_or_default())
    }

    /// When the person last did something in the squad — made it, wrote to a
    /// member — in unix seconds; the board's own times count besides.
    /// `None`: not since this was kept.
    pub fn active_at(&self) -> Option<u64> {
        self.meta("active_at").and_then(|at| at.parse().ok())
    }

    pub fn set_active_at(&self, at: u64) -> Result<()> {
        self.set_meta("active_at", &at.to_string())
    }

    /// Whether the weekly routine `role` brings was ever added (it comes once).
    pub fn role_routine_added(&self, role: &str) -> bool {
        self.meta(&format!("{role}_check_added")).is_some()
    }

    pub fn set_role_routine_added(&self, role: &str) -> Result<()> {
        self.set_meta(&format!("{role}_check_added"), "1")
    }

    /// The designer's weekly review among the routines (`crate::design::review`),
    /// by its id: added with the designer, gone once the person deletes it.
    pub fn design_review_routine(&self) -> Option<u32> {
        self.meta("design_review_routine").and_then(|id| id.parse().ok())
    }

    pub fn set_design_review_routine(&self, id: Option<u32>) -> Result<()> {
        self.set_meta("design_review_routine", &id.map(|id| id.to_string()).unwrap_or_default())
    }

    /// The PM's weekly retro among the routines (`crate::retro`), by its id:
    /// added with the PM, gone once the person deletes it.
    pub fn retro_routine(&self) -> Option<u32> {
        self.meta("retro_routine").and_then(|id| id.parse().ok())
    }

    pub fn set_retro_routine(&self, id: Option<u32>) -> Result<()> {
        self.set_meta("retro_routine", &id.map(|id| id.to_string()).unwrap_or_default())
    }

    /// When the PM was last woken over an empty board, in unix seconds.
    pub fn idle_woken_at(&self) -> Option<u64> {
        self.meta("idle_woken_at").and_then(|at| at.parse().ok())
    }

    pub fn set_idle_woken_at(&self, at: u64) -> Result<()> {
        self.set_meta("idle_woken_at", &at.to_string())
    }

    /// When the PM was last asked to look at the person's ideas, in unix seconds.
    pub fn ideas_woken_at(&self) -> Option<u64> {
        self.meta("ideas_woken_at").and_then(|at| at.parse().ok())
    }

    pub fn set_ideas_woken_at(&self, at: u64) -> Result<()> {
        self.set_meta("ideas_woken_at", &at.to_string())
    }

    pub fn set_hq(&self, path: Option<&Path>) -> Result<()> {
        self.set_meta("hq", &path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default())
    }

    // ── the board ──

    /// Put a ticket on the board. Its id is the next free number; the one given is ignored.
    pub fn create_ticket(&self, t: &Ticket) -> Result<TicketId> {
        let id: i64 = self.conn.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM tickets", [], |r| r.get(0))?;
        let at = now();
        self.conn.execute(
            "INSERT INTO tickets (id, title, body, status, owner, assignee, project, depends_on, branch, created_by,
                                  note, created_at, updated_at, kind, priority, rank, plan, label)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, ?13, ?14, ?15, ?16, ?17)",
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
                t.plan,
                t.label.map(|l| l.key()),
            ],
        )?;
        Ok(id as TicketId)
    }

    /// Write a ticket's fields back.
    pub fn save_ticket(&self, t: &Ticket) -> Result<()> {
        self.conn.execute(
            "UPDATE tickets SET title = ?1, body = ?2, status = ?3, owner = ?4, assignee = ?5, project = ?6,
                                depends_on = ?7, branch = ?8, note = ?9, updated_at = ?10, merged = ?12, kind = ?13,
                                off_main = ?14, priority = ?15, rank = ?16, remaining = ?17, images = ?18, design_qa = ?19, plan = ?20, label = ?21
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
                remaining_str(&t.images),
                t.design_qa,
                t.plan,
                t.label.map(|l| l.key()),
            ],
        )?;
        Ok(())
    }

    /// Set the model and effort a ticket's lanes run on ([`Ticket::model`]).
    /// Kept apart from [`Journal::save_ticket`], so a copy of the ticket read
    /// before the choice never writes it away.
    pub fn set_ticket_model(&self, id: TicketId, model: Option<&equipe_protocol::tiers::TicketModel>) -> Result<()> {
        let text = model.and_then(|m| serde_json::to_string(m).ok());
        self.conn.execute("UPDATE tickets SET model = ?1 WHERE id = ?2", params![text, id])?;
        Ok(())
    }

    /// Keep that `ticket` was sent back by `by`, and why.
    pub fn record_send_back(&self, ticket: TicketId, by: &str, reason: equipe_protocol::tiers::SendBackReason) -> Result<()> {
        self.conn.execute("INSERT INTO send_backs (ticket, at, by, reason) VALUES (?1, ?2, ?3, ?4)", params![ticket, now(), by, reason.code()])?;
        Ok(())
    }

    /// How many times `ticket` was sent back for `reason`.
    pub fn send_backs(&self, ticket: TicketId, reason: equipe_protocol::tiers::SendBackReason) -> Result<u32> {
        self.conn.query_row("SELECT COUNT(*) FROM send_backs WHERE ticket = ?1 AND reason = ?2", params![ticket, reason.code()], |r| r.get(0))
    }

    /// Keep a model choice for `ticket`: `what` happened (`lane`: a lane
    /// started on it; `escalated`; `person`), on whom, the tier the rules
    /// asked where the ticket ran above it, and what the rules made of the
    /// ticket by itself.
    pub fn record_model_choice(&self, ticket: TicketId, what: &str, member: Option<(&str, &str)>, m: &equipe_protocol::tiers::TicketModel) -> Result<()> {
        let word = |v: serde_json::Result<serde_json::Value>| v.ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
        self.conn.execute(
            "INSERT INTO model_choices (ticket, at, what, member, role, engine, tier, model, effort, by, reason, shadow, classified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                ticket,
                now(),
                what,
                member.map(|(name, _)| name),
                member.map(|(_, role)| role),
                m.engine.map(engine_str),
                m.tier.map(|t| t.code()),
                m.model,
                m.effort,
                word(serde_json::to_value(m.by)),
                word(serde_json::to_value(m.reason)),
                m.shadow.map(|t| t.code()),
                m.classified.map(|r| word(serde_json::to_value(r))),
            ],
        )?;
        Ok(())
    }

    /// The model choices kept for `ticket`, oldest first: what happened, the
    /// tier, by whom, and the cheaper tier recorded beside it.
    pub fn model_choices(&self, ticket: TicketId) -> Result<Vec<ModelChoiceKept>> {
        let mut stmt = self.conn.prepare("SELECT what, tier, by, shadow, classified FROM model_choices WHERE ticket = ?1 ORDER BY at, rowid")?;
        let rows = stmt.query_map([ticket], |r| Ok(ModelChoiceKept { what: r.get(0)?, tier: r.get(1)?, by: r.get(2)?, shadow: r.get(3)?, classified: r.get(4)? }))?;
        rows.collect()
    }

    /// Whether equipe picks each ticket's model: quality first unless the person said off.
    pub fn model_choice(&self) -> equipe_protocol::tiers::ModelChoice {
        self.meta("model_choice").and_then(|v| equipe_protocol::tiers::ModelChoice::parse(&v)).unwrap_or_default()
    }

    pub fn set_model_choice(&self, choice: equipe_protocol::tiers::ModelChoice) -> Result<()> {
        self.set_meta("model_choice", choice.code())
    }

    /// Whether the squad's design QA precheck runs and its gate acts: shadow
    /// unless the person said otherwise.
    pub fn design_qa_gate(&self) -> crate::design_qa::GateMode {
        self.meta("design_qa_gate").and_then(|v| crate::design_qa::GateMode::parse(&v)).unwrap_or_default()
    }

    pub fn set_design_qa_gate(&self, mode: crate::design_qa::GateMode) -> Result<()> {
        self.set_meta("design_qa_gate", mode.code())
    }

    /// The tiers design QA runs on: economy unless the person said otherwise.
    pub fn design_qa_tiers(&self) -> crate::design_qa::TierPolicy {
        self.meta("design_qa_tiers").and_then(|v| crate::design_qa::TierPolicy::parse(&v)).unwrap_or_default()
    }

    pub fn set_design_qa_tiers(&self, policy: crate::design_qa::TierPolicy) -> Result<()> {
        self.set_meta("design_qa_tiers", &policy.code())
    }

    /// How the squad runs design QA, its daily review and audits:
    /// `risk_batch` with the defaults unless the person said otherwise.
    pub fn design_qa_settings(&self) -> crate::design_qa::Settings {
        use crate::design_qa::{Mode, Settings, parse_time};
        let d = Settings::default();
        Settings {
            mode: self.meta("design_qa_mode").and_then(|v| Mode::parse(&v)).unwrap_or(d.mode),
            batch_at: self.meta("design_qa_batch_at").and_then(|v| parse_time(&v)).unwrap_or(d.batch_at),
            batch_size: self.meta("design_qa_batch_size").and_then(|v| v.parse().ok()).filter(|n| *n > 0).unwrap_or(d.batch_size),
            sample: self.meta("design_qa_sample").and_then(|v| v.parse().ok()).map(|n: u8| n.min(100)).unwrap_or(d.sample),
            audit_miss: self.meta("design_qa_audit_miss").and_then(|v| v.parse().ok()).map(|n: u8| n.min(100)).unwrap_or(d.audit_miss),
            tight: self.meta("design_qa_tight").is_some_and(|v| v == "1"),
        }
    }

    /// Whether the person (or `qa-config`) set the squad's design QA mode:
    /// one never set follows equipe's default, and is told when it changes.
    pub fn design_qa_mode_set(&self) -> bool {
        self.meta("design_qa_mode").is_some()
    }

    pub fn set_design_qa_mode(&self, mode: crate::design_qa::Mode) -> Result<()> {
        self.set_meta("design_qa_mode", mode.code())
    }

    pub fn set_design_qa_batch_at(&self, at: (u8, u8)) -> Result<()> {
        self.set_meta("design_qa_batch_at", &crate::design_qa::time_code(at))
    }

    pub fn set_design_qa_batch_size(&self, n: usize) -> Result<()> {
        self.set_meta("design_qa_batch_size", &n.max(1).to_string())
    }

    pub fn set_design_qa_sample(&self, percent: u8) -> Result<()> {
        self.set_meta("design_qa_sample", &percent.min(100).to_string())
    }

    pub fn set_design_qa_audit_miss(&self, percent: u8) -> Result<()> {
        self.set_meta("design_qa_audit_miss", &percent.min(100).to_string())
    }

    /// Tighten design QA's routing (`true`, as of now) or relax it.
    pub fn set_design_qa_tight(&self, tight: bool) -> Result<()> {
        if tight {
            self.set_meta("design_qa_tight_at", &now().to_string())?;
        }
        self.set_meta("design_qa_tight", if tight { "1" } else { "0" })
    }

    /// When the routing was last tightened, in unix seconds.
    pub fn design_qa_tight_at(&self) -> i64 {
        self.meta("design_qa_tight_at").and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    /// A one-time flag kept in the squad's journal under `key`: `true` the
    /// first time it is asked for, `false` ever after.
    pub fn first_time(&self, key: &str) -> bool {
        let key = format!("once:{key}");
        if self.meta(&key).is_some() {
            return false;
        }
        self.set_meta(&key, &now().to_string()).is_ok()
    }

    /// The turns, tickets and turn endings that [`crate::usage`] measures,
    /// for the turns that ended in `since..until`.
    pub fn usage_input(&self, since: u64, until: u64) -> Result<crate::usage::Input> {
        crate::usage::select(&self.conn, since, until)
    }

    /// Whether the squad gets the daily usage note (`equipe usage digest`):
    /// on unless the person switched it off.
    pub fn usage_digest_on(&self) -> bool {
        self.meta("usage_digest").is_none_or(|v| v != "off")
    }

    pub fn set_usage_digest(&self, on: bool) -> Result<()> {
        self.set_meta("usage_digest", if on { "on" } else { "off" })
    }

    /// The UTC day (`YYYY-MM-DD`) the daily usage note was last looked at,
    /// said or not; `None` for never.
    pub fn usage_digest_day(&self) -> Option<String> {
        self.meta("usage_digest_day")
    }

    pub fn set_usage_digest_day(&self, day: &str) -> Result<()> {
        self.set_meta("usage_digest_day", day)
    }

    /// When the last daily design review was made, in unix seconds; 0 for never.
    pub fn design_qa_last_batch(&self) -> i64 {
        self.meta("design_qa_last_batch").and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    /// Main's commit in `project` the last daily review looked at.
    pub fn design_qa_batch_base(&self, project: &Path) -> Option<String> {
        self.meta(&format!("design_qa_batch_base:{}", project.display()))
    }

    pub fn set_design_qa_batch_base(&self, project: &Path, commit: &str) -> Result<()> {
        self.set_meta(&format!("design_qa_batch_base:{}", project.display()), commit)
    }

    /// Keep how design QA ticket `qa` was routed in `risk_batch`.
    pub fn set_design_qa_route(&self, qa: TicketId, route: &str, risk: &str, lite: bool) -> Result<()> {
        self.conn.execute("UPDATE design_qa_runs SET route = ?2, risk = ?3, lite = ?4 WHERE qa = ?1", params![qa, route, (!risk.is_empty()).then_some(risk), lite])?;
        Ok(())
    }

    /// Keep design QA ticket `qa`'s mode and the engineer's waiver, if any.
    pub fn set_design_qa_mode_of(&self, qa: TicketId, mode: &str, waived: Option<&str>) -> Result<()> {
        self.conn.execute("UPDATE design_qa_runs SET mode = ?2, waived = ?3 WHERE qa = ?1", params![qa, mode, waived])?;
        Ok(())
    }

    /// The last design QA of `checked` before `qa`: its verdict and who gave it.
    pub fn design_qa_before(&self, checked: TicketId, qa: TicketId) -> Option<(Option<String>, Option<String>)> {
        self.conn
            .query_row("SELECT verdict, verdict_by FROM design_qa_runs WHERE checked = ?1 AND qa < ?2 ORDER BY qa DESC LIMIT 1", params![checked, qa], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .ok()
            .flatten()
    }

    /// Whether the squad has had any design QA under the precheck.
    pub fn has_design_qa_runs(&self) -> bool {
        self.conn.query_row("SELECT 1 FROM design_qa_runs LIMIT 1", [], |_| Ok(())).optional().ok().flatten().is_some()
    }

    /// The changes waiting for the daily review: let through without a
    /// designer turn and not reviewed yet, oldest first.
    pub fn design_qa_waiting_for_batch(&self) -> Vec<DesignQaRun> {
        let ids: Vec<TicketId> = self
            .conn
            .prepare("SELECT qa FROM design_qa_runs WHERE route = 'batch' AND batch IS NULL ORDER BY qa")
            .and_then(|mut stmt| stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<TicketId>>>())
            .unwrap_or_default();
        ids.into_iter().filter_map(|qa| self.design_qa_run(qa)).collect()
    }

    /// Keep a daily review or audit ticket and the changes it covers.
    pub fn start_design_qa_batch(&self, batch: &DesignQaBatch, covers: &[TicketId]) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO design_qa_batches (ticket, kind, created_at, items, dir, heads) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![batch.ticket, batch.kind, now(), covers.len() as i64, batch.dir, batch.heads],
        )?;
        let column = if batch.kind == "audit" { "audit" } else { "batch" };
        for qa in covers {
            self.conn.execute(&format!("UPDATE design_qa_runs SET {column} = ?2 WHERE qa = ?1"), params![qa, batch.ticket])?;
        }
        if batch.kind != "audit" {
            self.set_meta("design_qa_last_batch", &now().to_string())?;
        }
        Ok(())
    }

    /// Keep main's commits a review looked at, once its precheck ran.
    pub fn set_design_qa_batch_heads(&self, ticket: TicketId, heads: &str) -> Result<()> {
        self.conn.execute("UPDATE design_qa_batches SET heads = ?2 WHERE ticket = ?1", params![ticket, heads])?;
        Ok(())
    }

    /// Keep how a review or audit ended.
    pub fn finish_design_qa_batch(&self, ticket: TicketId, blockers: u32, fixes: u32, polish: u32) -> Result<()> {
        self.conn.execute(
            "UPDATE design_qa_batches SET finished_at = ?2, blockers = ?3, fixes = ?4, polish = ?5 WHERE ticket = ?1 AND finished_at IS NULL",
            params![ticket, now(), blockers, fixes, polish],
        )?;
        Ok(())
    }

    /// Daily review or audit ticket `ticket`, if it is one.
    pub fn design_qa_batch(&self, ticket: TicketId) -> Option<DesignQaBatch> {
        self.conn
            .query_row("SELECT ticket, kind, created_at, items, dir, heads, finished_at, blockers, fixes, polish FROM design_qa_batches WHERE ticket = ?1", [ticket], |r| {
                Ok(DesignQaBatch {
                    ticket: r.get(0)?,
                    kind: r.get(1)?,
                    created_at: r.get(2)?,
                    items: r.get(3)?,
                    dir: r.get(4)?,
                    heads: r.get(5)?,
                    finished_at: r.get(6)?,
                    blockers: r.get(7)?,
                    fixes: r.get(8)?,
                    polish: r.get(9)?,
                })
            })
            .optional()
            .ok()
            .flatten()
    }

    /// The changes daily review or audit `ticket` covers, oldest first.
    pub fn design_qa_batch_items(&self, ticket: TicketId) -> Vec<DesignQaRun> {
        let ids: Vec<TicketId> = self
            .conn
            .prepare("SELECT qa FROM design_qa_runs WHERE batch = ?1 OR audit = ?1 ORDER BY qa")
            .and_then(|mut stmt| stmt.query_map([ticket], |r| r.get(0))?.collect::<rusqlite::Result<Vec<TicketId>>>())
            .unwrap_or_default();
        ids.into_iter().filter_map(|qa| self.design_qa_run(qa)).collect()
    }

    /// The daily reviews and audits nobody has finished.
    pub fn design_qa_open_batches(&self) -> Vec<TicketId> {
        self.conn
            .prepare("SELECT ticket FROM design_qa_batches WHERE finished_at IS NULL ORDER BY ticket")
            .and_then(|mut stmt| stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<TicketId>>>())
            .unwrap_or_default()
    }

    /// Whether a daily review or audit is still open: nobody has finished it.
    pub fn design_qa_batch_open(&self) -> bool {
        self.conn.query_row("SELECT 1 FROM design_qa_batches WHERE finished_at IS NULL LIMIT 1", [], |_| Ok(())).optional().ok().flatten().is_some()
    }

    /// Keep what an audit found of the change design QA ticket `qa` let through.
    pub fn set_design_qa_audit_verdict(&self, qa: TicketId, verdict: &str) -> Result<()> {
        self.conn.execute("UPDATE design_qa_runs SET audit_verdict = ?2 WHERE qa = ?1", params![qa, verdict])?;
        Ok(())
    }

    /// The latest audited changes, newest first: `true` for an S1/S2 miss;
    /// only those audited after `after` (unix seconds) when given.
    pub fn design_qa_audits(&self, after: i64, max: usize) -> Vec<bool> {
        self.conn
            .prepare(
                "SELECT r.audit_verdict FROM design_qa_runs r JOIN design_qa_batches b ON b.ticket = r.audit
                 WHERE r.audit_verdict IS NOT NULL AND COALESCE(b.finished_at, 0) >= ?1 ORDER BY b.finished_at DESC, r.qa DESC LIMIT ?2",
            )
            .and_then(|mut stmt| stmt.query_map(params![after, max as i64], |r| Ok(r.get::<_, String>(0)? == "miss"))?.collect::<rusqlite::Result<Vec<bool>>>())
            .unwrap_or_default()
    }

    /// The members whose model the person set (`/model`, a move they made):
    /// equipe's policies never run them below it.
    pub fn person_models(&self) -> Vec<String> {
        self.meta("person_models").and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    /// `member`'s model is no longer the person's choice: equipe moved it to
    /// the other engine on its own, onto a model of its own.
    pub fn unmark_person_model(&self, member: &str) -> Result<()> {
        let mut all = self.person_models();
        if all.iter().any(|m| m == member) {
            all.retain(|m| m != member);
            self.set_meta("person_models", &serde_json::to_string(&all).unwrap_or_default())?;
        }
        Ok(())
    }

    pub fn mark_person_model(&self, member: &str) -> Result<()> {
        let mut all = self.person_models();
        if !all.iter().any(|m| m == member) {
            all.push(member.to_owned());
            self.set_meta("person_models", &serde_json::to_string(&all).unwrap_or_default())?;
        }
        Ok(())
    }

    /// How many design QA tickets `checked` has had, the ones from before
    /// this table included.
    pub fn design_qa_rounds(&self, checked: TicketId) -> Result<u32> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM ticket_events WHERE kind = 'created' AND by = 'equipe' AND text = ?1",
            [format!("design QA of T-{checked}")],
            |r| r.get(0),
        )
    }

    /// Keep that design QA ticket `qa` of `checked` was made, in its `round`,
    /// under gate `gate`, its precheck `precheck`.
    pub fn start_design_qa_run(&self, qa: TicketId, checked: TicketId, round: u32, gate: &str, precheck: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO design_qa_runs (qa, checked, round, gate, precheck, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![qa, checked, round, gate, precheck, now()],
        )?;
        Ok(())
    }

    /// Keep how `qa`'s precheck went: `run` with its precheck, decision,
    /// judgment, whether it acted, report, commits and time filled in.
    pub fn set_design_qa_precheck(&self, run: &DesignQaRun) -> Result<()> {
        self.conn.execute(
            "UPDATE design_qa_runs SET precheck = ?2, decision = ?3, judgment = ?4, acted = ?5, report = ?6, head = ?7, base = ?8, precheck_secs = ?9 WHERE qa = ?1",
            params![run.qa, run.precheck, run.decision, run.judgment, run.acted, run.report, run.head, run.base, run.precheck_secs.map(|s| s as i64)],
        )?;
        Ok(())
    }

    /// Keep how `qa` ended: `pass`, `changes` or `cancelled`, and by whom.
    /// The first verdict stays.
    pub fn set_design_qa_verdict(&self, qa: TicketId, verdict: &str, by: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE design_qa_runs SET verdict = ?2, verdict_by = ?3, finished_at = ?4 WHERE qa = ?1 AND verdict IS NULL",
            params![qa, verdict, by, now()],
        )?;
        Ok(())
    }

    /// Design QA ticket `qa`'s run, if it was kept.
    pub fn design_qa_run(&self, qa: TicketId) -> Option<DesignQaRun> {
        self.conn
            .query_row(
                "SELECT qa, checked, round, gate, precheck, decision, judgment, acted, report, head, base, precheck_secs, verdict, verdict_by, created_at, finished_at,
                        mode, route, risk, lite, waived, batch, audit, audit_verdict
                 FROM design_qa_runs WHERE qa = ?1",
                [qa],
                |r| {
                    Ok(DesignQaRun {
                        qa: r.get(0)?,
                        checked: r.get(1)?,
                        round: r.get(2)?,
                        gate: r.get(3)?,
                        precheck: r.get(4)?,
                        decision: r.get(5)?,
                        judgment: r.get(6)?,
                        acted: r.get(7)?,
                        report: r.get(8)?,
                        head: r.get(9)?,
                        base: r.get(10)?,
                        precheck_secs: r.get::<_, Option<i64>>(11)?.map(|s| s.max(0) as u64),
                        verdict: r.get(12)?,
                        verdict_by: r.get(13)?,
                        created_at: r.get(14)?,
                        finished_at: r.get(15)?,
                        mode: r.get(16)?,
                        route: r.get(17)?,
                        risk: r.get(18)?,
                        lite: r.get(19)?,
                        waived: r.get(20)?,
                        batch: r.get(21)?,
                        audit: r.get(22)?,
                        audit_verdict: r.get(23)?,
                    })
                },
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn ticket(&self, id: TicketId) -> Result<Option<Ticket>> {
        Ok(self.query_tickets("WHERE t.id = ?1", params![id])?.into_iter().next())
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

    /// Every ticket, oldest first. Read whole once, then given again from
    /// memory until a ticket or a run changes ([`BoardVersion`]): a big
    /// board is megabytes, and the daemon's loop asks for it many times a
    /// minute — on a busy disk each read took seconds.
    pub fn tickets(&self) -> Result<Vec<Ticket>> {
        let version = self.board_version();
        if let (Some(version), Some((at, board))) = (version, self.board.borrow().as_ref())
            && *at == version
        {
            return Ok(board.clone());
        }
        let board = self.query_tickets("", [])?;
        // What the read itself might have moved is looked at again: kept only when nothing did.
        if let Some(version) = version.filter(|v| self.board_version() == Some(*v)) {
            *self.board.borrow_mut() = Some((version, board.clone()));
        }
        Ok(board)
    }

    /// Where the board stands now; `None` when it can't be told (no triggers).
    fn board_version(&self) -> Option<BoardVersion> {
        let ours = self.conn.query_row("SELECT n FROM temp.board_writes", [], |r| r.get(0)).ok()?;
        let others = self.conn.query_row("PRAGMA data_version", [], |r| r.get(0)).ok()?;
        Some(BoardVersion { ours, others })
    }

    fn query_tickets(&self, filter: &str, args: impl rusqlite::Params) -> Result<Vec<Ticket>> {
        // Each ticket with its run, in one read.
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT t.id, t.title, t.body, t.status, t.owner, t.assignee, t.project, t.depends_on, t.branch, t.created_by, t.note, t.worktree, t.merged, t.kind, t.off_main, t.updated_at,
                    t.priority, t.rank, t.remaining, t.images, t.design_qa, t.plan, t.model,
                    e.state, e.session, e.operation, e.generation, e.heartbeat_at, e.lease_expires_at, e.reason, e.head, e.dirty, e.pause, t.label
             FROM tickets t LEFT JOIN ticket_executions e ON e.ticket = t.id {filter} ORDER BY t.id"
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
                execution: None,
                merged: r.get(12)?,
                kind: TicketKind::parse(&r.get::<_, String>(13)?).unwrap_or_default(),
                label: r.get::<_, Option<String>>(33)?.and_then(|l| equipe_protocol::title::TicketLabel::parse(&l)),
                asks: Vec::new(),
                remaining: r.get::<_, Option<String>>(18)?.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default(),
                images: r.get::<_, Option<String>>(19)?.and_then(|i| serde_json::from_str(&i).ok()).unwrap_or_default(),
                priority: Priority::parse(&r.get::<_, String>(16)?).unwrap_or_default(),
                rank: r.get(17)?,
                off_main: r.get::<_, Option<String>>(14)?.and_then(|o| serde_json::from_str(&o).ok()),
                updated_at: r.get::<_, Option<i64>>(15)?.map(|at| at.max(0) as u64),
                design_qa: r.get(20)?,
                plan: r.get(21)?,
                // Unreadable (a newer daemon's, or damaged): its member's own model, as before.
                model: r.get::<_, Option<String>>(22)?.and_then(|m| serde_json::from_str(&m).ok()),
            };
            // A waiting ticket's asks are kept in its note; they travel as a list.
            t.asks = t.asks();
            t.execution = match r.get::<_, Option<String>>(23)? {
                Some(state) => Some(Box::new(TicketExecution {
                    state: ExecutionState::parse(&state),
                    session: r.get::<_, i64>(24)?.max(0) as u64,
                    operation: r.get(25)?,
                    generation: r.get::<_, i64>(26)?.max(0) as u64,
                    heartbeat_at: r.get::<_, i64>(27)?.max(0) as u64,
                    lease_expires_at: r.get::<_, i64>(28)?.max(0) as u64,
                    reason: r.get(29)?,
                    pause: r.get::<_, Option<String>>(32)?.and_then(|p| serde_json::from_str(&p).ok()),
                    head: r.get(30)?,
                    dirty: r.get::<_, Option<i64>>(31)?.map(|n| n.clamp(0, i64::from(u32::MAX)) as u32),
                })),
                None => None,
            };
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

    /// Keep Nemotron's review of its ticket, in place of one before.
    pub fn save_review(&self, review: &equipe_protocol::review::CodeReview) -> Result<()> {
        let body = serde_json::to_string(review).map_err(|e| Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn.execute("INSERT OR REPLACE INTO reviews (ticket, body) VALUES (?1, ?2)", params![review.ticket, body])?;
        Ok(())
    }

    /// Forget `ticket`'s review: its work changed since.
    pub fn forget_review(&self, ticket: TicketId) -> Result<()> {
        self.conn.execute("DELETE FROM reviews WHERE ticket = ?1", [ticket])?;
        Ok(())
    }

    /// The reviews of `tickets` that were kept, in their order.
    pub fn reviews(&self, tickets: &[TicketId]) -> Result<Vec<equipe_protocol::review::CodeReview>> {
        let mut stmt = self.conn.prepare("SELECT body FROM reviews WHERE ticket = ?1")?;
        let mut found = Vec::new();
        for id in tickets {
            let body: Option<String> = stmt.query_row([id], |r| r.get(0)).optional()?;
            found.extend(body.and_then(|b| serde_json::from_str(&b).ok()));
        }
        Ok(found)
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

    /// Keep a member's turn — who, on what, how long, how it ended, what it
    /// used — for the weekly retro. A quarter's worth is kept.
    pub fn record_turn(&self, t: &crate::retro::Turn) -> Result<()> {
        self.record_turn_use(t, None)
    }

    /// Keep a member's turn with what it used, by kind ([`TurnUse`]).
    pub fn record_turn_use(&self, t: &crate::retro::Turn, used: Option<&TurnUse>) -> Result<()> {
        let n = |f: fn(&TurnUse) -> u64| used.map(|u| f(u) as i64);
        self.conn.execute(
            "INSERT INTO turns (member, role, ticket, began, ended, outcome, tokens, engine, model, session, input, output, cache_write, cache_write_long, cache_read, calls, effort)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                t.member,
                t.role,
                t.ticket,
                t.began as i64,
                t.ended as i64,
                t.outcome,
                t.tokens as i64,
                used.map(|u| engine_str(u.engine)),
                used.and_then(|u| u.model.clone()),
                used.and_then(|u| u.session.clone()),
                n(|u| u.tokens.input),
                n(|u| u.tokens.output),
                n(|u| u.tokens.cache_write),
                n(|u| u.cache_write_long),
                n(|u| u.tokens.cache_read),
                n(|u| u.calls),
                used.and_then(|u| u.effort.clone())
            ],
        )?;
        self.conn.execute("DELETE FROM turns WHERE ended < ?1", [now() - KEPT_FOR_RETROS])?;
        Ok(())
    }

    /// What each turn on `ticket` ran on, oldest first: its engine, the model
    /// the engine named and the effort it ran at (`None`: the engine's own).
    pub fn ticket_turns(&self, ticket: TicketId) -> Result<Vec<TurnRan>> {
        let mut stmt = self.conn.prepare("SELECT engine, model, effort FROM turns WHERE ticket = ?1 ORDER BY ended, rowid")?;
        let rows = stmt.query_map([ticket], |r| Ok(TurnRan { engine: r.get(0)?, model: r.get(1)?, effort: r.get(2)? }))?;
        rows.collect()
    }

    /// The turns that ended at or after `since`, oldest first.
    pub fn turns_since(&self, since: u64) -> Result<Vec<crate::retro::Turn>> {
        let mut stmt = self.conn.prepare("SELECT member, role, ticket, began, ended, outcome, tokens FROM turns WHERE ended >= ?1 ORDER BY ended, rowid")?;
        let rows = stmt.query_map([since as i64], |r| {
            Ok(crate::retro::Turn {
                member: r.get(0)?,
                role: r.get(1)?,
                ticket: r.get(2)?,
                began: r.get::<_, i64>(3)?.max(0) as u64,
                ended: r.get::<_, i64>(4)?.max(0) as u64,
                outcome: r.get(5)?,
                tokens: r.get::<_, i64>(6)?.max(0) as u64,
            })
        })?;
        rows.collect()
    }

    /// Keep what a member waited on the person for, once answered.
    pub fn record_wait(&self, w: &crate::retro::Wait) -> Result<()> {
        let answered = w.answered.unwrap_or(w.asked);
        self.conn.execute(
            "INSERT INTO waits (kind, member, ticket, what, asked, answered, outcome) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![w.kind, w.member, w.ticket, w.what, w.asked as i64, answered as i64, w.outcome],
        )?;
        self.conn.execute("DELETE FROM waits WHERE answered < ?1", [now() - KEPT_FOR_RETROS])?;
        Ok(())
    }

    /// Keep a sign-in handoff, card or sign-in (`crate::logins`): hosts,
    /// names and causes only.
    pub fn record_login(&self, entry: &crate::logins::Entry) -> Result<()> {
        crate::logins::insert(&self.conn, entry, now() - KEPT_FOR_RETROS)?;
        Ok(())
    }

    /// The sign-in record since `since`, oldest first.
    pub fn logins_since(&self, since: u64) -> Result<Vec<crate::logins::Entry>> {
        crate::logins::select(&self.conn, since)
    }

    /// Whether `site` was signed in to before, as far as the record goes.
    pub fn signed_in_before(&self, site: &str) -> bool {
        crate::logins::signed_in_before(&self.conn, site)
    }

    /// The waits answered at or after `since`, oldest first.
    pub fn waits_since(&self, since: u64) -> Result<Vec<crate::retro::Wait>> {
        let mut stmt = self.conn.prepare("SELECT kind, member, ticket, what, asked, answered, outcome FROM waits WHERE answered >= ?1 ORDER BY answered, rowid")?;
        let rows = stmt.query_map([since as i64], |r| {
            Ok(crate::retro::Wait {
                kind: r.get(0)?,
                member: r.get(1)?,
                ticket: r.get(2)?,
                what: r.get(3)?,
                asked: r.get::<_, i64>(4)?.max(0) as u64,
                answered: Some(r.get::<_, i64>(5)?.max(0) as u64),
                outcome: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    /// Every ticket's record since `since`, oldest first, each with its ticket.
    pub fn ticket_events_since(&self, since: u64) -> Result<Vec<(TicketId, TicketEntry)>> {
        let mut stmt = self.conn.prepare("SELECT ticket, at, by, kind, text FROM ticket_events WHERE at >= ?1 ORDER BY at, ticket, seq")?;
        let rows = stmt.query_map([since as i64], |r| Ok((r.get(0)?, TicketEntry { at: r.get::<_, i64>(1)?.max(0) as u64, by: r.get(2)?, kind: r.get(3)?, text: r.get(4)? })))?;
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
        self.touch_conversation(id, "engine_session", engine_session)?;
        self.touch(id, "engine_session", engine_session)
    }

    /// Where a session's desk turns run from now on.
    pub fn set_cwd(&self, id: SessionId, cwd: &Path) -> Result<()> {
        self.touch(id, "cwd", cwd.to_string_lossy())
    }

    /// Forget the engine's conversation: the next turn starts a fresh one.
    pub fn clear_engine_session(&self, id: SessionId) -> Result<()> {
        self.touch_conversation(id, "engine_session", None::<String>)?;
        self.touch(id, "engine_session", None::<String>)
    }

    fn touch_conversation(&self, id: SessionId, set: &str, value: impl rusqlite::ToSql) -> Result<()> {
        self.conn.execute(&format!("UPDATE conversations SET {set} = ?1, updated_at = ?2 WHERE id = ?3"), params![value, now(), id as i64])?;
        Ok(())
    }

    /// A conversation the person opened with the PM, about `project`
    /// (`None`: the whole squad). Its lines are kept as any session's.
    /// A conversation with the PM, held where the PM works (`cwd`), which
    /// its replies' pictures are read from.
    pub fn record_conversation(&self, id: SessionId, project: Option<&str>, cwd: &Path) -> Result<()> {
        let t = now();
        self.conn.execute(
            "INSERT OR REPLACE INTO conversations (id, project, open, created_at, updated_at, cwd) VALUES (?1, ?2, 1, ?3, ?3, ?4)",
            params![id as i64, project, t, cwd.to_string_lossy()],
        )?;
        Ok(())
    }

    pub fn set_conversation_project(&self, id: SessionId, project: Option<&str>) -> Result<()> {
        self.touch_conversation(id, "project", project)
    }

    /// The person names a conversation: its own title and its session's,
    /// both or neither. What equipe names later ([`Journal::set_title_if_empty`])
    /// leaves it be. False when there is no such conversation.
    pub fn rename_conversation(&self, id: SessionId, title: &str) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        if tx.execute("UPDATE conversations SET title = ?1, updated_at = ?2 WHERE id = ?3", params![title, now(), id as i64])? == 0 {
            return Ok(false);
        }
        tx.execute("UPDATE sessions SET title = ?1 WHERE id = ?2", params![title, id as i64])?;
        tx.commit()?;
        Ok(true)
    }

    /// Put away: it leaves the list; its lines stay as long as any.
    pub fn close_conversation(&self, id: SessionId) -> Result<()> {
        self.conn.execute("UPDATE conversations SET open = 0, auto = 0, updated_at = ?1 WHERE id = ?2", params![now(), id as i64])?;
        Ok(())
    }

    /// Put away by equipe itself, for lack of use: the same, and recorded as
    /// its doing ([`equipe_protocol::PutAway::auto`]).
    pub fn close_conversation_auto(&self, id: SessionId) -> Result<()> {
        self.conn.execute("UPDATE conversations SET open = 0, auto = 1, updated_at = ?1 WHERE id = ?2", params![now(), id as i64])?;
        Ok(())
    }

    /// The person pins a conversation (or lets it go): a pinned one is never
    /// put away by itself. Not an activity of its own.
    pub fn set_conversation_pinned(&self, id: SessionId, pinned: bool) -> Result<bool> {
        Ok(self.conn.execute("UPDATE conversations SET pinned = ?1 WHERE id = ?2", params![pinned, id as i64])? > 0)
    }

    /// For each open conversation: when something last happened in it (its
    /// latest line, or when it was made or last changed) and whether it is
    /// pinned.
    pub fn conversation_marks(&self) -> Result<std::collections::HashMap<SessionId, (u64, bool)>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, MAX(c.updated_at, COALESCE((SELECT MAX(e.at) FROM entries e WHERE e.session = c.id), 0)), c.pinned
             FROM conversations c WHERE c.open = 1",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)? as u64, (r.get::<_, i64>(1)?.max(0) as u64, r.get::<_, bool>(2)?))))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Automatic tidying of conversations: on unless the person turned it off.
    pub fn auto_put_away(&self) -> bool {
        self.meta("auto_put_away").as_deref() != Some("off")
    }

    pub fn set_auto_put_away(&self, on: bool) -> Result<()> {
        self.set_meta("auto_put_away", if on { "on" } else { "off" })
    }

    /// The conversations put away, the latest first: at most `limit`.
    pub fn put_away(&self, limit: usize) -> Result<Vec<equipe_protocol::PutAway>> {
        let mut stmt = self.conn.prepare("SELECT id, project, title, updated_at, auto FROM conversations WHERE open = 0 ORDER BY updated_at DESC, id DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(equipe_protocol::PutAway { session: r.get::<_, i64>(0)? as u64, project: r.get(1)?, title: r.get(2)?, at: r.get(3)?, auto: r.get(4)? })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// A conversation put away, taken up again: back on the list, as it was.
    /// None if there is no such conversation, or it is open.
    pub fn reopen_conversation(&self, id: SessionId) -> Result<Option<ConversationRecord>> {
        let changed = self.conn.execute("UPDATE conversations SET open = 1, updated_at = ?1 WHERE id = ?2 AND open = 0", params![now(), id as i64])?;
        self.conn.execute("UPDATE conversations SET auto = 0 WHERE id = ?1", params![id as i64])?;
        if changed == 0 {
            return Ok(None);
        }
        Ok(self.conversations()?.into_iter().find(|c| c.id == id))
    }

    /// The conversations with the PM still open, oldest first.
    pub fn conversations(&self) -> Result<Vec<ConversationRecord>> {
        let mut stmt = self.conn.prepare("SELECT id, project, engine_session, title FROM conversations WHERE open = 1 ORDER BY created_at, id")?;
        let rows = stmt.query_map([], |r| {
            Ok(ConversationRecord { id: r.get::<_, i64>(0)? as u64, project: r.get(1)?, engine_session: r.get(2)?, title: r.get(3)? })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// The first finished title wins; later attempts never rename it.
    pub fn set_title_if_empty(&self, id: SessionId, title: &str) -> Result<bool> {
        let mut changed = false;
        for table in ["sessions", "conversations"] {
            changed |= self.conn.execute(&format!("UPDATE {table} SET title = ?1 WHERE id = ?2 AND title IS NULL"), params![title, id as i64])? > 0;
        }
        Ok(changed)
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

    /// The person's messages whose kept line holds `needle`, each with where
    /// it is kept (session, line): for a look through what was sent
    /// (`crate::kept::rescue`).
    pub fn messages_holding(&self, needle: &str) -> Result<Vec<(SessionId, i64, Entry)>> {
        let mut stmt = self.conn.prepare("SELECT session, seq, body FROM entries WHERE instr(body, ?1) > 0 ORDER BY session, seq")?;
        let rows = stmt.query_map([needle], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?;
        Ok(rows
            .filter_map(|row| row.ok())
            .filter_map(|(session, seq, body)| Some((session as SessionId, seq, serde_json::from_str::<Entry>(&body).ok()?)))
            .filter(|(.., entry)| matches!(entry, Entry::User { .. }))
            .collect())
    }

    /// Line `seq` of `session`'s conversation, as `entry` now has it — when it
    /// was said stays.
    pub fn rewrite(&self, session: SessionId, seq: i64, entry: &Entry) -> Result<()> {
        let body = serde_json::to_string(entry).map_err(|e| Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn.execute("UPDATE entries SET body = ?1 WHERE session = ?2 AND seq = ?3", params![body, session as i64, seq])?;
        Ok(())
    }

    /// Everything waiting for members' next turns, of every kind.
    pub fn all_waiting(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT text FROM waiting ORDER BY session, kind, seq")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    /// Whether this journal's sent attachments were looked through once
    /// already ([`crate::kept::rescue`]).
    pub fn kept_rescued(&self) -> bool {
        self.meta("kept_rescued").is_some()
    }

    pub fn set_kept_rescued(&self) -> Result<()> {
        self.set_meta("kept_rescued", "1")
    }

    /// Whether the PM's change requests left as plain comments on work in
    /// review were sent back to their members once already.
    pub fn change_requests_recovered(&self) -> bool {
        self.meta("change_requests_recovered").is_some()
    }

    pub fn set_change_requests_recovered(&self) -> Result<()> {
        self.set_meta("change_requests_recovered", "1")
    }

    fn entries(&self, id: SessionId, limit: Option<u64>) -> Result<Vec<Entry>> {
        Ok(self.timed_entries(id, limit)?.0)
    }

    /// The lines, and when each was kept (unix seconds; a time that can't be
    /// one, `None`) — a message when it came, a reply when it was done.
    fn timed_entries(&self, id: SessionId, limit: Option<u64>) -> Result<(Vec<Entry>, Vec<Option<u64>>)> {
        let mut stmt = self.conn.prepare(
            "SELECT body, at FROM (SELECT seq, body, at FROM entries WHERE session = ?1 ORDER BY seq DESC LIMIT ?2)
             ORDER BY seq ASC",
        )?;
        let limit = limit.map_or(-1, |l| l as i64);
        let rows = stmt.query_map(params![id as i64, limit], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        Ok(rows
            .filter_map(|row| row.ok().and_then(|(body, at)| Some((serde_json::from_str::<Entry>(&body).ok()?, u64::try_from(at).ok().filter(|&at| at > 0)))))
            .unzip())
    }

    /// The latest work on `ticket` in whichever conversation did it: the
    /// session, where it works, and its lines from the ticket's header
    /// (`── T-<n> · …`) to the next ticket's or a thread's start, each with
    /// when it was kept — at most [`TICKET_WORK_MOST`] lines. `None`: no
    /// conversation has worked on it.
    pub fn ticket_lines(&self, ticket: TicketId) -> Result<Option<TicketWorkLines>> {
        let header = format!("── T-{ticket} ");
        let found: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT session, MAX(seq) FROM entries WHERE instr(body, ?1) > 0 AND instr(body, '\"kind\":\"note\"') > 0 GROUP BY session ORDER BY MAX(at) DESC LIMIT 1",
                [&header],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((session, from)) = found else { return Ok(None) };
        let cwd: Option<String> = self.conn.query_row("SELECT cwd FROM sessions WHERE id = ?1", [session], |r| r.get(0)).optional()?;
        let mut stmt = self.conn.prepare("SELECT body, at FROM entries WHERE session = ?1 AND seq >= ?2 ORDER BY seq ASC LIMIT ?3")?;
        let rows = stmt.query_map(params![session, from, TICKET_WORK_MOST as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut lines = Vec::new();
        for (body, at) in rows.filter_map(|row| row.ok()) {
            let Ok(entry) = serde_json::from_str::<Entry>(&body) else { continue };
            let ends = match &entry {
                Entry::Note { text, .. } => text.starts_with("── T-") && !text.starts_with(&header),
                Entry::Thread { .. } => true,
                _ => false,
            };
            if ends && !lines.is_empty() {
                break;
            }
            lines.push((entry, u64::try_from(at).ok().filter(|&at| at > 0)));
        }
        Ok(Some(TicketWorkLines { session: session as SessionId, cwd: cwd.map(PathBuf::from), lines }))
    }

    /// The last `limit` lines of a session's conversation, oldest first.
    pub fn recent(&self, id: SessionId, limit: u64) -> Result<Vec<Entry>> {
        self.entries(id, Some(limit))
    }

    /// Every kept line of a session's conversation, oldest first.
    pub fn conversation(&self, id: SessionId) -> Result<Vec<Entry>> {
        self.entries(id, None)
    }

    /// The same, with when each line was said, one per line.
    pub fn conversation_at(&self, id: SessionId) -> Result<(Vec<Entry>, Vec<Option<u64>>)> {
        self.timed_entries(id, None)
    }

    /// The latest `limit` lines of the squad's conversations that name a path
    /// (a `/` in them), newest first, each with where its session works — what
    /// a reply's pictures and folders are read from (`crate::visuals`). The
    /// person's conversations with the PM count, from where the PM works (one
    /// opened before that was kept: where the open PM works now).
    pub fn said_with_paths(&self, limit: u64) -> Result<Vec<(PathBuf, Entry)>> {
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(s.cwd, c.cwd, (SELECT p.cwd FROM sessions p WHERE p.role = 'pm' AND p.open = 1 ORDER BY p.id LIMIT 1)), e.body
             FROM entries e LEFT JOIN sessions s ON s.id = e.session LEFT JOIN conversations c ON c.id = e.session
             WHERE (e.body LIKE '%/%' OR e.body LIKE '%![%') AND (s.id IS NOT NULL OR c.id IS NOT NULL) ORDER BY e.at DESC, e.seq DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)))?;
        Ok(rows.filter_map(|row| row.ok()).filter_map(|(cwd, body)| Some((PathBuf::from(cwd?), serde_json::from_str(&body).ok()?))).collect())
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

    /// Every member that was ever on the squad, open or closed, as
    /// `(name, role)`, each name once with the role it held last.
    pub fn members_ever(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, role FROM sessions s WHERE name IS NOT NULL AND role IS NOT NULL
               AND id = (SELECT id FROM sessions t WHERE t.name = s.name AND t.role IS NOT NULL ORDER BY t.updated_at DESC, t.id DESC LIMIT 1)
             ORDER BY name",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
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
    fn design_qa_settings_default_to_risk_batch_and_reviews_audits_are_kept() {
        use crate::design_qa::{Mode, Settings};
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
        assert_eq!(j.design_qa_settings(), Settings::default());
        assert!(!j.design_qa_mode_set() && !j.has_design_qa_runs());
        j.set_design_qa_mode(Mode::PerTicket).unwrap();
        j.set_design_qa_batch_at((9, 30)).unwrap();
        j.set_design_qa_sample(20).unwrap();
        let s = j.design_qa_settings();
        assert_eq!((s.mode, s.batch_at, s.sample, j.design_qa_mode_set()), (Mode::PerTicket, (9, 30), 20, true));
        assert!(j.first_time("x") && !j.first_time("x"), "once");
        // Two changes let through, one reviewed and audited.
        for (qa, checked) in [(10, 1), (11, 2)] {
            j.start_design_qa_run(qa, checked, 1, "shadow", "done").unwrap();
            j.set_design_qa_route(qa, "batch", "", true).unwrap();
        }
        assert_eq!(j.design_qa_waiting_for_batch().iter().map(|r| r.qa).collect::<Vec<_>>(), [10, 11]);
        j.start_design_qa_batch(&DesignQaBatch { ticket: 20, kind: "review".into(), ..DesignQaBatch::default() }, &[10, 11]).unwrap();
        j.start_design_qa_batch(&DesignQaBatch { ticket: 21, kind: "audit".into(), ..DesignQaBatch::default() }, &[11]).unwrap();
        assert!(j.design_qa_waiting_for_batch().is_empty() && j.design_qa_batch_open() && j.design_qa_last_batch() > 0);
        assert_eq!(j.design_qa_batch_items(21).iter().map(|r| r.qa).collect::<Vec<_>>(), [11]);
        j.set_design_qa_audit_verdict(11, "miss").unwrap();
        j.finish_design_qa_batch(21, 1, 1, 0).unwrap();
        j.finish_design_qa_batch(20, 0, 0, 2).unwrap();
        assert!(!j.design_qa_batch_open());
        assert_eq!(j.design_qa_audits(0, 20), [true]);
        assert_eq!(j.design_qa_run(11).unwrap().audit, Some(21));
        j.set_design_qa_tight(true).unwrap();
        assert!(j.design_qa_settings().tight && j.design_qa_tight_at() > 0);
        assert_eq!(j.design_qa_before(2, 30), Some((None, None)));
    }

    #[test]
    fn a_journal_with_no_room_says_so_and_still_reads() {
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
        j.record_session(&record(1)).unwrap();
        let pages: i64 = j.conn.query_row("PRAGMA max_page_count = 1", [], |r| r.get(0)).unwrap();
        assert!(pages >= 1);
        let err = j.append(1, &Entry::User { text: "x".repeat(200_000), kept: Vec::new() }).unwrap_err();
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
    fn a_squad_keeps_how_many_subagents_a_member_runs_at_once_within_what_engines_take() {
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
        assert_eq!(j.subagents_at_once(), None, "unset: each engine's own");
        j.set_subagents_at_once(Some(4)).unwrap();
        assert_eq!(j.subagents_at_once(), Some(4));
        // Never none (both engines refuse 0), never past what equipe offers.
        j.set_subagents_at_once(Some(0)).unwrap();
        assert_eq!(j.subagents_at_once(), Some(1));
        j.set_subagents_at_once(Some(500)).unwrap();
        assert_eq!(j.subagents_at_once(), Some(equipe_protocol::MOST_SUBAGENTS_AT_ONCE));
        // Back to each engine's own.
        j.set_subagents_at_once(None).unwrap();
        assert_eq!(j.subagents_at_once(), None);
    }

    #[test]
    fn a_squad_keeps_its_parallel_work_and_max_comes_back_as_it_was_left() {
        use equipe_protocol::lanes::{Parallel, ParallelMode};
        let home = temp_home();
        let j = Journal::for_squad(&home, "acme").unwrap();
        assert_eq!(j.parallel(), Parallel::default(), "never set: auto");
        j.set_parallel(Parallel::max(8, 20)).unwrap();
        assert_eq!(Journal::for_squad(&home, "acme").unwrap().parallel(), Parallel::max(8, 20), "kept across opening");
        // Back on auto, the caps stay for the next time max is chosen.
        j.set_parallel(Parallel { mode: ParallelMode::Auto, ..Parallel::max(8, 20) }).unwrap();
        assert_eq!(j.parallel(), Parallel { mode: ParallelMode::Auto, per_member: 8, per_engine: 20 });
        // Never past the bounds, whatever was written.
        j.set_meta("parallel", r#"{"mode":"max","per_member":400,"per_engine":0}"#).unwrap();
        assert_eq!(j.parallel(), Parallel::max(12, 1));
        j.set_meta("parallel", "not json").unwrap();
        assert_eq!(j.parallel(), Parallel::default(), "unreadable: auto");
    }

    #[test]
    fn a_squad_keeps_who_reviews_its_designers_work_auto_until_set() {
        use equipe_protocol::engines::Reviewer;
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
        assert_eq!(j.design_reviewer(), Reviewer::Auto);
        j.set_design_reviewer(Reviewer::Same).unwrap();
        assert_eq!(j.design_reviewer(), Reviewer::Same);
        j.set_design_reviewer(Reviewer::Other).unwrap();
        assert_eq!(j.design_reviewer(), Reviewer::Other);
        j.set_design_reviewer(Reviewer::Auto).unwrap();
        assert_eq!(j.design_reviewer(), Reviewer::Auto);
    }

    #[test]
    fn what_a_conversation_with_the_pm_says_is_read_from_where_the_pm_works() {
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
        let pm = SessionRecord { cwd: "/squad/pm".into(), member: Some(Member { role: "pm".into(), name: "pm".into() }), ..record(1) };
        j.record_session(&pm).unwrap();
        j.record_conversation(50, Some("web"), Path::new("/squad/pm")).unwrap();
        j.append(50, &Entry::Reply { text: "![shot](shots/home.png)".into() }).unwrap();
        // One kept before conversations knew where they were held.
        j.record_conversation(51, None, Path::new("/elsewhere")).unwrap();
        j.conn.execute("UPDATE conversations SET cwd = NULL WHERE id = 51", []).unwrap();
        j.append(51, &Entry::Reply { text: "see /tmp/a.png".into() }).unwrap();
        // A line of nothing known is left out.
        j.append(99, &Entry::Reply { text: "/nowhere".into() }).unwrap();
        let said = j.said_with_paths(10).unwrap();
        let cwds: Vec<_> = said.iter().map(|(cwd, _)| cwd.to_string_lossy().into_owned()).collect();
        assert_eq!(cwds, ["/squad/pm", "/squad/pm"], "{said:?}");
    }

    #[test]
    fn a_conversation_put_away_is_kept_listed_and_taken_up_again() {
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
        j.record_conversation(50, Some("web"), Path::new("/squad/pm")).unwrap();
        j.record_conversation(51, None, Path::new("/squad/pm")).unwrap();
        j.record_conversation(52, Some("app"), Path::new("/squad/pm")).unwrap();
        j.set_title_if_empty(50, "the pricing page").unwrap();
        j.append(50, &Entry::User { text: "the pricing page".into(), kept: Vec::new() }).unwrap();
        j.close_conversation(50).unwrap();
        j.close_conversation(52).unwrap();
        j.conn.execute("UPDATE conversations SET updated_at = 100 WHERE id = 50", []).unwrap();
        j.conn.execute("UPDATE conversations SET updated_at = 200 WHERE id = 52", []).unwrap();
        let away = j.put_away(10).unwrap();
        assert_eq!(away.iter().map(|c| c.session).collect::<Vec<_>>(), [52, 50], "the latest first, open ones not");
        assert_eq!(away[1].title.as_deref(), Some("the pricing page"));
        assert_eq!(away[1].project.as_deref(), Some("web"));
        assert_eq!(away[1].at, 100);
        assert_eq!(j.put_away(1).unwrap().len(), 1);
        let back = j.reopen_conversation(50).unwrap().expect("taken up");
        assert_eq!((back.id, back.project.as_deref(), back.title.as_deref()), (50, Some("web"), Some("the pricing page")));
        assert_eq!(j.conversation(50).unwrap(), [Entry::User { text: "the pricing page".into(), kept: Vec::new() }], "its lines, as they were");
        assert_eq!(j.put_away(10).unwrap().iter().map(|c| c.session).collect::<Vec<_>>(), [52]);
        assert!(j.reopen_conversation(50).unwrap().is_none(), "open already");
        assert!(j.reopen_conversation(99).unwrap().is_none(), "no such one");
    }

    #[test]
    fn a_name_the_person_gives_is_kept_whole_and_never_named_over() {
        let home = temp_home();
        let id = 71;
        {
            let j = Journal::for_squad(&home, "acme").unwrap();
            j.record_session(&record(id)).unwrap();
            j.record_conversation(id, Some("desktop"), Path::new("/squad/pm")).unwrap();
            j.set_title_if_empty(id, "fix the login page").unwrap();
            assert!(j.rename_conversation(id, "Login copy").unwrap());
            // What equipe names later (the first message, a made-up topic) leaves it be.
            j.set_title_if_empty(id, "something else").unwrap();
            assert!(!j.rename_conversation(999, "nobody").unwrap(), "no such conversation");
            j.close_conversation(id).unwrap();
        }
        let j = Journal::for_squad(&home, "acme").unwrap();
        assert_eq!(j.put_away(10).unwrap()[0].title.as_deref(), Some("Login copy"));
        let back = j.reopen_conversation(id).unwrap().expect("back");
        assert_eq!(back.title.as_deref(), Some("Login copy"));
    }

    #[test]
    fn a_generated_topic_is_written_once_and_survives_more_turns_restart_and_put_away() {
        let home = temp_home();
        let id = 70;
        {
            let j = Journal::for_squad(&home, "acme").unwrap();
            j.record_conversation(id, Some("desktop"), Path::new("/squad/pm")).unwrap();
            let question = "대화 첫 메시지를 그대로 제목으로 쓰지 않게 해 줘";
            j.append(id, &Entry::User { text: question.into(), kept: Vec::new() }).unwrap();
            j.append(id, &Entry::Reply { text: "첫 교환을 요약해 한 번 저장하겠습니다.".into() }).unwrap();
            let title = crate::conversation_title::resolve(Some("세션 주제 제목"), question);
            assert_ne!(title, question);
            assert!(j.set_title_if_empty(id, &title).unwrap());
            j.append(id, &Entry::User { text: "두 번째 질문".into(), kept: Vec::new() }).unwrap();
            j.append(id, &Entry::Reply { text: "두 번째 답변".into() }).unwrap();
            assert!(!j.set_title_if_empty(id, "바뀌면 안 되는 제목").unwrap());
            j.close_conversation(id).unwrap();
        }
        let j = Journal::for_squad(&home, "acme").unwrap();
        assert_eq!(j.put_away(10).unwrap()[0].title.as_deref(), Some("세션 주제 제목"));
        let back = j.reopen_conversation(id).unwrap().expect("conversation back");
        assert_eq!(back.title.as_deref(), Some("세션 주제 제목"));
        assert_eq!(j.conversation(id).unwrap().len(), 4);
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
            j.append(7, &Entry::User { text: "add a docstring".into(), kept: Vec::new() }).unwrap();
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
            vec![Entry::User { text: "add a docstring".into(), kept: Vec::new() }, Entry::Reply { text: "done".into() }]
        );
        assert_eq!(j.squad().as_deref(), Some("acme"));
    }

    #[test]
    fn each_line_comes_back_with_when_it_was_said() {
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
        j.record_session(&record(7)).unwrap();
        let before = now() as u64;
        j.append(7, &Entry::User { text: "add a docstring".into(), kept: Vec::new() }).unwrap();
        j.append(7, &Entry::Reply { text: "done".into() }).unwrap();
        let (lines, times) = j.conversation_at(7).unwrap();
        assert_eq!(lines, j.conversation(7).unwrap());
        assert_eq!(times.len(), 2);
        assert!(times.iter().all(|at| at.is_some_and(|at| at >= before && at <= now() as u64)), "{times:?}");
        // Kept a while ago: that time, not now.
        j.backdate_entries(7, 1_762_300_000);
        assert_eq!(j.conversation_at(7).unwrap().1, vec![Some(1_762_300_000); 2]);
        // A time that can't be one (a row written wrong): not known.
        j.backdate_entries(7, 0);
        assert_eq!(j.conversation_at(7).unwrap().1, vec![None, None]);
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
            j.append(1, &Entry::Note { undo: None, text: i.to_string() }).unwrap();
        }
        let texts: Vec<_> = j
            .recent(1, 3)
            .unwrap()
            .into_iter()
            .map(|e| match e {
                Entry::Note { text, .. } => text,
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
        j.append(1, &Entry::Note { undo: None, text: "old".into() }).unwrap();
        j.backdate_entries(1, 1_000);
        j.append(1, &Entry::Note { undo: None, text: "new".into() }).unwrap();
        j.prune(now() - 86_400).unwrap();
        assert_eq!(j.conversation(1).unwrap(), vec![Entry::Note { undo: None, text: "new".into() }]);
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
            execution: None,
            merged: None,
            off_main: None,
            design_qa: None, plan: None, model: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            label: None, remaining: Vec::new(), images: Vec::new(),
            updated_at: None,
            priority: Default::default(), rank: 0,
            kind: TicketKind::Code,
        }
    }

    #[test]
    fn the_board_read_from_memory_follows_every_change_to_it() {
        let home = temp_home();
        let j = Journal::for_squad(&home, "acme").unwrap();
        let a = j.create_ticket(&ticket("Checkout")).unwrap();
        assert_eq!(j.tickets().unwrap().len(), 1);
        // Read again unchanged: the same board.
        assert_eq!(j.tickets().unwrap(), j.tickets().unwrap());
        // Changed through this connection: a ticket, then its run.
        let mut t = j.ticket(a).unwrap().unwrap();
        t.status = TicketStatus::InProgress;
        j.save_ticket(&t).unwrap();
        assert_eq!(j.tickets().unwrap()[0].status, TicketStatus::InProgress);
        assert!(j.tickets().unwrap()[0].execution.is_none());
        let generation = j.acquire_execution(a, 7, "op-1", 100, 60).unwrap().expect("acquired");
        let run = j.tickets().unwrap()[0].execution.clone().expect("its run");
        assert_eq!((run.session, run.generation, run.lease_expires_at), (7, generation, 160));
        assert!(j.heartbeat_execution(a, generation, 7, 130, 60).unwrap());
        assert_eq!(j.tickets().unwrap()[0].execution.as_ref().unwrap().lease_expires_at, 190);
        // Changed through another connection, as another process would.
        let other = Journal::for_squad(&home, "acme").unwrap();
        other.create_ticket(&ticket("Search")).unwrap();
        let mut first = other.ticket(a).unwrap().unwrap();
        first.title = "Checkout, again".into();
        other.save_ticket(&first).unwrap();
        let board = j.tickets().unwrap();
        assert_eq!(board.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Checkout, again", "Search"]);
        assert_eq!(board, other.tickets().unwrap(), "both see one board");
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
    fn a_hand_ins_pictures_are_kept() {
        let home = temp_home();
        let j = Journal::for_squad(&home, "acme").unwrap();
        j.create_ticket(&ticket("Logo")).unwrap();
        let mut t = j.ticket(1).unwrap().unwrap();
        assert!(t.images.is_empty());
        t.images = vec!["/h/worktrees/acme.home/1/brand/a.png".into(), "/h/worktrees/acme.home/1/brand/b.svg".into()];
        j.save_ticket(&t).unwrap();
        assert_eq!(j.ticket(1).unwrap().unwrap().images, t.images);
        t.images.clear();
        j.save_ticket(&t).unwrap();
        assert!(j.ticket(1).unwrap().unwrap().images.is_empty());
    }

    #[test]
    fn the_plan_a_ticket_was_cut_from_is_kept_from_its_making() {
        let home = temp_home();
        let j = Journal::for_squad(&home, "acme").unwrap();
        let plan = Some("docs/plans/2026-11-04-sign-in.md".to_owned());
        j.create_ticket(&Ticket { plan: plan.clone(), ..ticket("Magic link") }).unwrap();
        let mut t = j.ticket(1).unwrap().unwrap();
        assert_eq!(t.plan, plan);
        t.plan = None;
        j.save_ticket(&t).unwrap();
        assert_eq!(j.ticket(1).unwrap().unwrap().plan, None);
    }

    #[test]
    fn a_turn_keeps_what_it_used_by_kind_and_a_journal_from_before_gains_the_columns() {
        let home = temp_home();
        let path = squad_dir(&home, "acme").join("squad.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        {
            // The turns table as it first was.
            let old = Connection::open(&path).unwrap();
            old.execute_batch("CREATE TABLE turns (member TEXT NOT NULL, role TEXT NOT NULL, ticket INTEGER, began INTEGER NOT NULL, ended INTEGER NOT NULL, outcome TEXT NOT NULL, tokens INTEGER NOT NULL DEFAULT 0);
                INSERT INTO turns VALUES ('pm', 'pm', NULL, 1, 2, 'completed', 50);").unwrap();
        }
        let j = Journal::for_squad(&home, "acme").unwrap();
        let usage = crate::protocol::Usage { input_tokens: Some(10), output_tokens: Some(200), cache_tokens: Some(1300), cache_read_tokens: Some(1000), cache_write_tokens: Some(300), cache_write_long_tokens: Some(300), calls: Some(4), ..Default::default() };
        let used = TurnUse::of(Engine::Claude, Some("claude-opus-5-5".into()), Some("s-1".into()), &usage);
        assert_eq!((used.tokens.cache_read, used.tokens.cache_write, used.cache_write_long, used.calls), (1000, 300, 300, 4));
        let now = now() as u64;
        let turn = crate::retro::Turn { member: "frontend".into(), role: "frontend".into(), ticket: Some(7), began: now - 60, ended: now, outcome: "completed".into(), tokens: 1510 };
        j.record_turn_use(&turn, Some(&used)).unwrap();
        let kept: (String, String, String, i64, i64, i64, i64) = j
            .conn
            .query_row("SELECT engine, model, session, output, cache_read, cache_write_long, calls FROM turns WHERE member = 'frontend'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))
            .unwrap();
        assert_eq!(kept, ("claude".into(), "claude-opus-5-5".into(), "s-1".into(), 200, 1000, 300, 4));
        // The retro reads them as before.
        assert_eq!(j.turns_since(now - 100).unwrap(), std::slice::from_ref(&turn));
        // A sum alone: read from the cache.
        let summed = TurnUse::of(Engine::Codex, None, None, &crate::protocol::Usage { cache_tokens: Some(900), ..Default::default() });
        assert_eq!((summed.tokens.cache_read, summed.tokens.cache_write, summed.calls), (900, 0, 1));
        // The effort it ran at is kept beside the model; the turn from before has none.
        let thought = TurnUse { effort: Some("high".into()), ..used };
        j.record_turn_use(&crate::retro::Turn { member: "backend".into(), ..turn }, Some(&thought)).unwrap();
        let efforts: Vec<(String, Option<String>)> =
            j.conn.prepare("SELECT member, effort FROM turns ORDER BY rowid").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        assert_eq!(efforts, [("frontend".into(), None), ("backend".into(), Some("high".into()))]);
    }

    #[test]
    fn a_tickets_model_send_backs_and_choices_are_kept_and_old_tickets_read_as_before() {
        use equipe_protocol::tiers::{ModelChoice, ModelTier, SendBackReason, TicketModel, TierBy, TierReason};
        let home = temp_home();
        let j = Journal::for_squad(&home, "acme").unwrap();
        let id = j.create_ticket(&ticket("Sign-in")).unwrap();
        assert_eq!(j.ticket(id).unwrap().unwrap().model, None, "none chosen: the member's own model");
        let m = TicketModel { tier: Some(ModelTier::Deep), by: TierBy::Escalation, reason: TierReason::Escalated, escalations: 1, model: Some("opus".into()), ..TicketModel::default() };
        j.set_ticket_model(id, Some(&m)).unwrap();
        // A copy read before the choice doesn't write it away.
        let mut stale = j.ticket(id).unwrap().unwrap();
        stale.model = None;
        stale.title = "Sign-in, again".into();
        j.save_ticket(&stale).unwrap();
        assert_eq!(j.ticket(id).unwrap().unwrap().model, Some(m.clone()));
        // Damaged or from a newer daemon: read as none.
        j.conn.execute("UPDATE tickets SET model = 'not json' WHERE id = ?1", [id]).unwrap();
        assert_eq!(j.ticket(id).unwrap().unwrap().model, None);
        // Send-backs by reason.
        assert_eq!(j.send_backs(id, SendBackReason::Correctness).unwrap(), 0);
        j.record_send_back(id, "pm", SendBackReason::Correctness).unwrap();
        j.record_send_back(id, "pm", SendBackReason::Style).unwrap();
        j.record_send_back(id, "equipe", SendBackReason::Correctness).unwrap();
        assert_eq!((j.send_backs(id, SendBackReason::Correctness).unwrap(), j.send_backs(id, SendBackReason::Style).unwrap()), (2, 1));
        // Every choice, with the cheaper tier beside it.
        j.record_model_choice(id, "lane", Some(("backend", "backend")), &TicketModel { tier: Some(ModelTier::Max), reason: TierReason::MemberFloor, shadow: Some(ModelTier::Strong), classified: Some(TierReason::Code), ..TicketModel::default() }).unwrap();
        j.record_model_choice(id, "escalated", None, &m).unwrap();
        let kept = |what: &str, tier: &str, by: &str, shadow: Option<&str>, classified: Option<&str>| ModelChoiceKept { what: what.into(), tier: Some(tier.into()), by: by.into(), shadow: shadow.map(str::to_owned), classified: classified.map(str::to_owned) };
        assert_eq!(j.model_choices(id).unwrap(), [kept("lane", "max", "rule", Some("strong"), Some("code")), kept("escalated", "deep", "escalation", None, None)]);
        // The squad's say: quality first until set.
        assert_eq!(j.model_choice(), ModelChoice::QualityFirst);
        j.set_model_choice(ModelChoice::Off).unwrap();
        assert_eq!(Journal::for_squad(&home, "acme").unwrap().model_choice(), ModelChoice::Off);
    }

    #[test]
    fn a_resting_member_is_kept_by_name_across_a_restart() {
        let home = temp_home();
        let j = Journal::for_squad(&home, "acme").unwrap();
        assert!(j.resting().is_empty(), "nobody rests by default");
        assert_eq!(j.set_resting("designer", true).unwrap(), ["designer"]);
        assert_eq!(j.set_resting("qa", true).unwrap(), ["designer", "qa"]);
        assert_eq!(j.set_resting("designer", true).unwrap(), ["qa", "designer"], "once each");
        drop(j);
        let j = Journal::for_squad(&home, "acme").unwrap();
        assert!(j.is_resting("designer") && j.is_resting("qa") && !j.is_resting("backend"));
        assert_eq!(j.set_resting("designer", false).unwrap(), ["qa"]);
        assert!(!Journal::for_squad(&home, "acme").unwrap().is_resting("designer"), "woken: kept too");
        assert_eq!(j.stale_waits_asked_at(), None);
        j.set_stale_waits_asked_at(1_000).unwrap();
        assert_eq!(Journal::for_squad(&home, "acme").unwrap().stale_waits_asked_at(), Some(1_000));
    }

    #[test]
    fn the_design_qa_a_hand_in_waits_on_is_kept() {
        let home = temp_home();
        let j = Journal::for_squad(&home, "acme").unwrap();
        j.create_ticket(&ticket("Settings screen")).unwrap();
        let mut t = j.ticket(1).unwrap().unwrap();
        assert_eq!(t.design_qa, None);
        t.design_qa = Some(2);
        j.save_ticket(&t).unwrap();
        assert_eq!(j.ticket(1).unwrap().unwrap().design_qa, Some(2));
        t.design_qa = None;
        j.save_ticket(&t).unwrap();
        assert_eq!(j.ticket(1).unwrap().unwrap().design_qa, None);
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
    fn turns_waits_and_the_weeks_ticket_records_are_kept_for_the_retro() {
        let j = Journal::for_squad(&temp_home(), "acme").unwrap();
        let at = now() as u64;
        let turn = |member: &str, ended: u64| crate::retro::Turn { member: member.into(), role: "backend".into(), ticket: Some(4), began: ended - 600, ended, outcome: "completed".into(), tokens: 1200 };
        let failed = crate::retro::Turn { ticket: None, outcome: "failed".into(), ..turn("pm", at - 60) };
        j.record_turn(&turn("backend", at - 3600)).unwrap();
        j.record_turn(&failed).unwrap();
        j.record_turn(&turn("old", at - 200 * 24 * 3600)).unwrap();
        assert_eq!(j.turns_since(at - 7200).unwrap(), [turn("backend", at - 3600), failed]);
        assert!(j.turns_since(0).unwrap().iter().all(|t| t.member != "old"), "past a quarter: let go");
        let wait = crate::retro::Wait { kind: "permission".into(), member: "backend".into(), ticket: Some(4), what: "git push".into(), asked: at - 900, answered: Some(at - 300), outcome: "allowed".into() };
        j.record_wait(&wait).unwrap();
        assert_eq!(j.waits_since(at - 600).unwrap(), [wait]);
        assert!(j.waits_since(at - 200).unwrap().is_empty());
        j.ticket_event(4, "you", "reopened", "Reopened: wrong order").unwrap();
        j.ticket_event(5, "equipe", "assigned", "frontend").unwrap();
        let events = j.ticket_events_since(at.saturating_sub(10)).unwrap();
        assert_eq!(events.iter().map(|(id, e)| (*id, e.kind.as_str())).collect::<Vec<_>>(), [(4, "reopened"), (5, "assigned")]);
        assert!(j.ticket_events_since(at + 3600).unwrap().is_empty());
        assert_eq!(j.retro_routine(), None);
        j.set_retro_routine(Some(3)).unwrap();
        assert_eq!(j.retro_routine(), Some(3));
        j.set_retro_routine(None).unwrap();
        assert_eq!(j.retro_routine(), None);
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
    fn what_a_role_starts_with_is_kept_per_engine_and_cleared_back_to_the_recommendation() {
        use equipe_protocol::{Engine, RoleModel};
        let home = temp_home();
        let set = |role: &str, engine, model: Option<&str>, effort: Option<&str>| RoleModel { role: role.into(), engine, model: model.map(Into::into), effort: effort.map(Into::into) };
        {
            let j = Journal::for_squad(&home, "acme").unwrap();
            assert!(j.role_models().is_empty(), "the recommendation until the person sets one");
            j.set_role_model(set("frontend", Engine::Claude, Some("claude-opus-5-5"), None)).unwrap();
            j.set_role_model(set("frontend", Engine::Codex, None, Some("high"))).unwrap();
            j.set_role_model(set("designer", Engine::Claude, Some("fable"), Some("max"))).unwrap();
            // Set again: the newer replaces it.
            j.set_role_model(set("frontend", Engine::Claude, Some("sonnet"), None)).unwrap();
        }
        let j = Journal::for_squad(&home, "acme").unwrap();
        assert_eq!(
            j.role_models(),
            [set("designer", Engine::Claude, Some("fable"), Some("max")), set("frontend", Engine::Codex, None, Some("high")), set("frontend", Engine::Claude, Some("sonnet"), None)]
        );
        let left = j.set_role_model(set("frontend", Engine::Codex, None, None)).unwrap();
        assert_eq!(left.len(), 2, "nothing set is the recommendation again");
        assert_eq!(j.role_models(), left);
    }

    #[test]
    fn each_squad_gets_its_own_directory() {
        let home = Path::new("/h");
        assert_eq!(squad_dir(home, "acme"), PathBuf::from("/h/squads/acme"));
        assert_eq!(member_memory_dir(home, "acme", "frontend"), PathBuf::from("/h/squads/acme/members/frontend/memory"));
    }

    #[test]
    fn execution_generation_fences_late_heartbeats_and_requeue_is_idempotent() {
        let j = Journal::for_squad(&temp_home(), "lease").unwrap();
        j.conn.execute(
            "INSERT INTO tickets (id,title,body,status,depends_on,created_by,created_at,updated_at,kind)
             VALUES (7,'recover','', 'in_progress','', 'you',1,1,'code')",
            [],
        ).unwrap();
        assert_eq!(j.acquire_execution(7, 11, "start:one", 100, 30).unwrap(), Some(1));
        assert_eq!(j.acquire_execution(7, 12, "start:two", 101, 30).unwrap(), None, "valid lease prevents duplicate work");
        let first = j.execution(7).unwrap().unwrap();
        assert_eq!(first.generation, 1);
        assert_eq!(j.recover_executions(&[], 120).unwrap(), Vec::<TicketId>::new(), "a lease not yet run out is left alone");
        assert_eq!(j.recover_executions(&[], 131).unwrap(), vec![7], "past its lease, with no live session: stalled");
        assert_eq!(j.execution(7).unwrap().unwrap().reason.as_deref(), Some(equipe_protocol::stall_reason::LEASE_EXPIRED));
        assert_eq!(j.reserve_requeue(7, "approved:one", None, 133).unwrap(), Requeued::Reserved);
        assert_eq!(j.reserve_requeue(7, "approved:one", None, 134).unwrap(), Requeued::AlreadyApplied, "same approval is harmless");
        assert_eq!(j.execution(7).unwrap().unwrap().state, ExecutionState::Queued);
        assert_eq!(j.acquire_execution(7, 22, "approved:one", 135, 30).unwrap(), Some(2), "the queued recovery starts under its own generation");
        let second = j.execution(7).unwrap().unwrap();
        assert_eq!((second.generation, second.state, second.session), (2, ExecutionState::Active, 22));
        assert!(!j.heartbeat_execution(7, first.generation, 11, 136, 30).unwrap(), "old runtime cannot extend the new lease");
        assert!(j.heartbeat_execution(7, second.generation, 22, 136, 30).unwrap());
        assert_eq!(j.recover_executions(&[22], 10_000).unwrap(), Vec::<TicketId>::new(), "an observed live session is never called stale");
        assert!(!j.finish_execution(7, first.generation, ExecutionState::Stopped, None).unwrap(), "an old generation can't close the new run");
        // Stalled again much later: the first approval, replayed, still changes nothing.
        assert_eq!(j.recover_executions(&[], 10_000).unwrap(), vec![7]);
        assert_eq!(j.reserve_requeue(7, "approved:one", None, 10_001).unwrap(), Requeued::AlreadyApplied);
        assert_eq!(j.execution(7).unwrap().unwrap().state, ExecutionState::Stalled);
        assert_eq!(j.acquire_execution(7, 33, "approved:one", 10_002, 30).unwrap(), None, "a used operation never starts another run");
    }

    #[test]
    fn a_run_still_starting_when_the_daemon_stopped_goes_back_on_the_board() {
        let j = Journal::for_squad(&temp_home(), "starting").unwrap();
        for id in [1, 2, 3] {
            j.conn.execute(
                "INSERT INTO tickets (id,title,body,status,depends_on,created_by,created_at,updated_at,kind,assignee,owner,note)
                 VALUES (?1,'t','','in_progress','','you',1,1,'code','frontend','frontend','Reopened: again')",
                params![id],
            ).unwrap();
        }
        // T-2: a recovery the person approved, still making its workspace.
        j.acquire_execution(2, 12, "start:two", 100, 30).unwrap();
        j.recover_executions(&[], 200).unwrap();
        assert_eq!(j.reserve_requeue(2, "approved:two", None, 201).unwrap(), Requeued::Reserved);
        assert_eq!(j.acquire_execution(2, 22, "approved:two", 202, 30).unwrap(), Some(2));
        assert!(j.set_execution_starting(2, 2, true).unwrap());
        assert!(!j.set_execution_starting(2, 1, true).unwrap(), "only the very generation");
        // T-1: a board start, still making its workspace (after T-2's stall, which would stall it too).
        assert_eq!(j.acquire_execution(1, 11, &format!("{START_OPERATION}one"), 202, 30).unwrap(), Some(1));
        assert!(j.set_execution_starting(1, 1, true).unwrap());
        assert_eq!(j.execution(1).unwrap().unwrap().reason.as_deref(), Some(STARTING));
        // T-3: started; its session ran.
        j.acquire_execution(3, 13, "start:three", 100, 30).unwrap();
        j.set_execution_starting(3, 1, true).unwrap();
        assert!(j.set_execution_starting(3, 1, false).unwrap());
        assert_eq!(j.execution(3).unwrap().unwrap().reason, None);

        assert_eq!(j.release_starting_executions(300).unwrap(), vec![1, 2]);
        let one = j.ticket(1).unwrap().unwrap();
        assert_eq!((one.status, one.assignee.as_deref(), one.owner.as_deref(), one.note.as_deref()), (TicketStatus::Todo, None, Some("frontend"), Some("Reopened: again")));
        let run = j.execution(1).unwrap().unwrap();
        assert_eq!((run.state, run.reason.as_deref()), (ExecutionState::Stopped, Some("not_started")));
        let two = j.execution(2).unwrap().unwrap();
        assert_eq!((two.state, two.generation, two.operation.as_str()), (ExecutionState::Queued, 2, "approved:two"), "the approval starts exactly once");
        assert_eq!(j.acquire_execution(2, 32, "approved:two", 301, 30).unwrap(), Some(2), "under the generation it reserved");
        assert_eq!(j.ticket(3).unwrap().unwrap().status, TicketStatus::InProgress, "a run that started is the restart's to carry on");
        assert_eq!(j.release_starting_executions(302).unwrap(), Vec::<TicketId>::new(), "safe to run again");
    }

    #[test]
    fn only_a_stalled_run_is_requeued() {
        let j = Journal::for_squad(&temp_home(), "requeue-states").unwrap();
        for (id, status) in [(1, "in_progress"), (2, "review"), (3, "waiting")] {
            j.conn.execute(
                "INSERT INTO tickets (id,title,body,status,depends_on,created_by,created_at,updated_at,kind) VALUES (?1,'t','',?2,'','you',1,1,'code')",
                params![id, status],
            ).unwrap();
            j.acquire_execution(id, 10 + u64::from(id), &format!("start:{id}"), 100, 30).unwrap();
        }
        j.finish_execution(2, 1, ExecutionState::Stopped, Some("handed_in")).unwrap();
        j.finish_execution(3, 1, ExecutionState::Waiting, Some("waiting_for_person")).unwrap();
        assert_eq!(j.reserve_requeue(1, "op:1", None, 101).unwrap(), Requeued::NotStalled, "an active run is not taken from its member");
        assert_eq!(j.reserve_requeue(2, "op:2", None, 101).unwrap(), Requeued::NotStalled, "work in review stays in review");
        assert_eq!(j.reserve_requeue(3, "op:3", None, 101).unwrap(), Requeued::NotStalled, "a wait is the person's Resume");
        let statuses: Vec<TicketStatus> = [1, 2, 3].iter().map(|id| j.ticket(*id).unwrap().unwrap().status).collect();
        assert_eq!(statuses, [TicketStatus::InProgress, TicketStatus::Review, TicketStatus::Waiting]);
        // A queued recovery can be pointed elsewhere before it starts.
        j.recover_executions(&[], 200).unwrap();
        assert_eq!(j.reserve_requeue(1, "op:1", Some("backend"), 201).unwrap(), Requeued::Reserved);
        assert_eq!(j.reserve_requeue(1, "op:1b", Some("backend-3"), 202).unwrap(), Requeued::Reserved);
        assert_eq!(j.ticket(1).unwrap().unwrap().owner.as_deref(), Some("backend-3"));
        assert_eq!(j.execution(1).unwrap().unwrap().generation, 3);
    }

    /// T-563 review: a requeue overwrote the note, so the next run lost the
    /// latest send-back or design QA reason it must be told first.
    #[test]
    fn a_requeue_keeps_the_latest_send_back_reason_and_open_blockers() {
        let j = Journal::for_squad(&temp_home(), "requeue-note").unwrap();
        for (id, note) in [(4, "Reopened by the PM: the header overlaps the logo at 760px."), (5, "Design QA by designer: the button radius is 6, not 8.")] {
            j.conn.execute(
                "INSERT INTO tickets (id,title,body,status,depends_on,branch,owner,assignee,note,created_by,created_at,updated_at,kind)
                 VALUES (?1,'t','','in_progress','','topic','backend','backend-2',?2,'you',1,1,'code')",
                params![id, note],
            ).unwrap();
            let mut t = j.ticket(id).unwrap().unwrap();
            t.remaining = vec!["The API token for staging".into()];
            j.save_ticket(&t).unwrap();
            j.acquire_execution(id, 10 + u64::from(id), &format!("start:{id}"), 100, 30).unwrap();
            j.recover_executions(&[], 200).unwrap();
            assert_eq!(j.reserve_requeue(id, &format!("op:{id}"), Some("backend-3"), 201).unwrap(), Requeued::Reserved);
            let back = j.ticket(id).unwrap().unwrap();
            assert_eq!(back.note.as_deref(), Some(note), "the latest reason is kept for the next run");
            assert_eq!(back.remaining, ["The API token for staging"], "open blockers are kept");
            assert_eq!(
                (back.status, back.assignee.as_deref(), back.owner.as_deref(), back.branch.as_deref()),
                (TicketStatus::Todo, None, Some("backend-3"), Some("topic"))
            );
        }
    }

    #[test]
    fn legacy_in_progress_ticket_migrates_without_touching_ticket_work() {
        let j = Journal::for_squad(&temp_home(), "migration").unwrap();
        j.conn.execute(
            "INSERT INTO tickets (id,title,body,status,depends_on,branch,base,worktree,created_by,created_at,updated_at,kind)
             VALUES (8,'old','', 'in_progress','','topic','main','/tmp/kept','you',1,1,'code')",
            [],
        ).unwrap();
        j.stall_all_executions(50).unwrap();
        let migrated = j.execution(8).unwrap().unwrap();
        assert_eq!((migrated.state, migrated.generation, migrated.reason.as_deref()), (ExecutionState::Stalled, 0, Some(equipe_protocol::stall_reason::MIGRATED)));
        assert_eq!(j.ticket_work(8).unwrap(), TicketWork { base: Some("main".into()), worktree: Some("/tmp/kept".into()), engine_session: None });
        j.stall_all_executions(60).unwrap();
        assert_eq!(j.execution(8).unwrap().unwrap(), migrated, "safe to run again");
        // Carried on after the restart: a new generation, active.
        assert_eq!(j.acquire_execution(8, 9, "restart:1", 61, 30).unwrap(), Some(1));
        j.set_stalled_checkout(8, Some("abc1234"), Some(3)).unwrap();
        assert_eq!(j.execution(8).unwrap().unwrap().head, None, "only a stalled run keeps its checkout's facts");
        j.stall_all_executions(70).unwrap();
        j.set_stalled_checkout(8, Some("abc1234"), Some(3)).unwrap();
        let stalled = j.execution(8).unwrap().unwrap();
        assert_eq!((stalled.reason.as_deref(), stalled.head.as_deref(), stalled.dirty), (Some(equipe_protocol::stall_reason::NOT_RESUMED), Some("abc1234"), Some(3)));
    }

    #[test]
    fn the_context_settings_default_on_and_each_can_be_turned_off() {
        let home = std::env::temp_dir().join(format!("equipe-ctx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let j = Journal::for_squad(&home, "acme").unwrap();
        assert!(j.context_rotation() && j.context_tool_caps() && j.context_obsolete_check());
        j.set_context_rotation(false).unwrap();
        assert!(!j.context_rotation() && j.context_tool_caps() && j.context_obsolete_check());
        j.set_context_tool_caps(false).unwrap();
        j.set_context_obsolete_check(false).unwrap();
        j.set_context_rotation(true).unwrap();
        assert!(j.context_rotation() && !j.context_tool_caps() && !j.context_obsolete_check());
    }

    #[test]
    fn rotations_are_kept_and_counted_by_kind() {
        let home = std::env::temp_dir().join(format!("equipe-rot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let j = Journal::for_squad(&home, "acme").unwrap();
        assert!(j.rotations_since(0).unwrap().is_empty());
        j.record_rotation(1, Some("pm"), None, "desk", 260_000).unwrap();
        j.record_rotation(1, Some("pm"), None, "desk", 270_000).unwrap();
        j.record_rotation(5, Some("backend"), Some(42), "ticket", 310_000).unwrap();
        assert_eq!(j.rotations_since(0).unwrap(), vec![("desk".to_owned(), 2, 530_000), ("ticket".to_owned(), 1, 310_000)]);
        assert!(j.rotations_since(now() as u64 + 10).unwrap().is_empty());
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
