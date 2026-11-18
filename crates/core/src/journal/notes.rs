//! Long ticket notes, kept apart from the board.
//!
//! A ticket's note is the latest word on it — a hand-in summary, why it was
//! reopened — and most are a few hundred bytes. A few are megabytes: a
//! hand-in that listed every file of a stale worktree. On a board of
//! thousands of tickets those few were most of its bytes, read with every
//! whole read of the board and sent to every screen that attached.
//!
//! So a long note is kept whole, compressed, in `note_archive`, and the
//! ticket keeps its start ([`equipe_protocol::note_summary`]) with
//! `note_full` saying how long the whole is. That happens when a note over
//! [`NOTE_WRITE_CAP`] is written, and — for a closed ticket's note over
//! [`ARCHIVE_OVER`] — once the ticket has stayed closed [`ARCHIVE_AFTER`]
//! ([`Journal::archive_old_notes`], run in the background). The whole note
//! is read back on demand ([`Journal::full_note`]).

use std::io::{Read, Write};

use equipe_protocol::{TicketId, note_summary};
use rusqlite::{Connection, OptionalExtension, params};

use super::{Journal, Result, now};

/// A closed ticket's note longer than this, in bytes, is kept apart once the
/// ticket has stayed closed [`ARCHIVE_AFTER`].
pub const ARCHIVE_OVER: usize = 8 * 1024;

/// How long a ticket stays closed before its long note is kept apart: long
/// enough for the person to read the hand-in they just accepted.
pub const ARCHIVE_AFTER: u64 = 24 * 3600;

/// The longest note a ticket keeps on the board, in bytes, open or closed: a
/// longer one is kept apart as it is written, the ticket keeping its start.
/// A member's hand-in summary runs to a few kilobytes at most.
pub const NOTE_WRITE_CAP: usize = 16 * 1024;

/// How much of a note over [`NOTE_WRITE_CAP`] an open ticket keeps, in
/// characters: the member's summary, which comes first.
pub const OPEN_NOTE_SHOWN: usize = 4_000;

/// The most of a whole note one answer carries, in characters: beyond, it
/// ends early and says so.
pub const NOTE_SENT_MAX: usize = 256 * 1024;

/// The table and the column, made once.
pub(super) fn create(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS note_archive (
           ticket INTEGER PRIMARY KEY,
           full BLOB NOT NULL,
           bytes INTEGER NOT NULL,
           summary TEXT NOT NULL,
           at INTEGER NOT NULL
         );",
    )?;
    let _ = conn.execute("ALTER TABLE tickets ADD COLUMN note_full INTEGER", []);
    Ok(())
}

fn pack(text: &str) -> Vec<u8> {
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = encoder.write_all(text.as_bytes());
    encoder.finish().unwrap_or_default()
}

fn unpack(data: &[u8]) -> Option<String> {
    let mut out = String::new();
    flate2::read::DeflateDecoder::new(data).read_to_string(&mut out).ok()?;
    Some(out)
}

/// How big a squad's board is, for `equipe usage` and the disk report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BoardSize {
    pub tickets: u64,
    pub closed: u64,
    /// The notes the board carries, in bytes.
    pub note_bytes: u64,
    /// Notes kept apart, how long they are whole, and what they take compressed.
    pub archived: u64,
    pub archived_bytes: u64,
    pub archived_stored: u64,
    /// Closed tickets' notes still waiting to be kept apart.
    pub to_archive: u64,
    /// The squad's journal on disk, in bytes.
    pub file_bytes: u64,
}

impl Journal {
    /// What ticket `id`'s note is written as: a note over
    /// [`NOTE_WRITE_CAP`] is kept apart and the ticket keeps its start;
    /// the start of a note kept apart, written back as it was read, stays
    /// so — as does the whole note, read before it was kept apart. Any other
    /// note replaces the one kept apart. `(note, note_full)`.
    pub(super) fn note_to_store(&self, id: TicketId, note: Option<&str>) -> Result<(Option<String>, Option<i64>)> {
        let kept: Option<(String, i64)> =
            self.conn.query_row("SELECT summary, bytes FROM note_archive WHERE ticket = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        if let (Some((summary, bytes)), Some(note)) = (&kept, note) {
            let start = summary.strip_suffix('…').unwrap_or(summary).trim_end();
            if note == summary || (note.len() as i64 == *bytes && note.starts_with(start)) {
                return Ok((Some(summary.clone()), Some(*bytes)));
            }
        }
        if kept.is_some() {
            self.conn.execute("DELETE FROM note_archive WHERE ticket = ?1", [id])?;
        }
        match note {
            Some(note) if note.len() > NOTE_WRITE_CAP => {
                let summary = note_summary(note, OPEN_NOTE_SHOWN).unwrap_or_else(|| note.to_owned());
                self.keep_apart(id, note, &summary)?;
                Ok((Some(summary), Some(note.len() as i64)))
            }
            note => Ok((note.map(str::to_owned), None)),
        }
    }

    fn keep_apart(&self, id: TicketId, note: &str, summary: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO note_archive (ticket, full, bytes, summary, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, pack(note), note.len() as i64, summary, now()],
        )?;
        Ok(())
    }

    /// Ticket `id`'s whole note: the one kept apart, else the note on the
    /// board. `None`: no such ticket, or no note.
    pub fn full_note(&self, id: TicketId) -> Result<Option<String>> {
        let kept: Option<Vec<u8>> = self.conn.query_row("SELECT full FROM note_archive WHERE ticket = ?1", [id], |r| r.get(0)).optional()?;
        if let Some(note) = kept.as_deref().and_then(unpack) {
            return Ok(Some(note));
        }
        Ok(self.conn.query_row("SELECT note FROM tickets WHERE id = ?1", [id], |r| r.get::<_, Option<String>>(0)).optional()?.flatten())
    }

    /// Keep apart up to `limit` long notes due ([`ARCHIVE_OVER`] on a ticket
    /// closed [`ARCHIVE_AFTER`] before `now`, or anything over
    /// [`NOTE_WRITE_CAP`]), each in its own short transaction so the daemon's
    /// writes never wait long. A note that changed meanwhile is left for the
    /// next round. `(notes kept apart, bytes they took off the board)`.
    pub fn archive_old_notes(&self, now: u64, limit: usize) -> Result<(usize, u64)> {
        let cutoff = now.saturating_sub(ARCHIVE_AFTER) as i64;
        let due: Vec<TicketId> = {
            let mut stmt = self.conn.prepare(
                "SELECT id FROM tickets
                 WHERE note_full IS NULL AND length(CAST(note AS BLOB)) > ?1
                   AND ((status IN ('done', 'cancelled') AND updated_at < ?2) OR length(CAST(note AS BLOB)) > ?3)
                 ORDER BY id LIMIT ?4",
            )?;
            stmt.query_map(params![ARCHIVE_OVER as i64, cutoff, NOTE_WRITE_CAP as i64, limit as i64], |r| r.get(0))?.collect::<Result<_>>()?
        };
        let (mut kept, mut freed) = (0, 0u64);
        for id in due {
            let tx = self.conn.unchecked_transaction()?;
            let Some((note, status)): Option<(String, String)> =
                tx.query_row("SELECT note, status FROM tickets WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?
            else {
                continue;
            };
            let shown = if matches!(status.as_str(), "done" | "cancelled") { equipe_protocol::CLOSED_NOTE_SHOWN } else { OPEN_NOTE_SHOWN };
            let Some(summary) = note_summary(&note, shown) else { continue };
            self.keep_apart(id, &note, &summary)?;
            // `updated_at` stays: keeping a note apart is no change to the ticket.
            let changed = tx.execute(
                "UPDATE tickets SET note = ?1, note_full = ?2 WHERE id = ?3 AND note = ?4",
                params![summary, note.len() as i64, id, note],
            )?;
            if changed == 1 {
                tx.commit()?;
                kept += 1;
                freed += (note.len() - summary.len()) as u64;
            }
        }
        Ok((kept, freed))
    }

    /// Give the file back up to `pages` of the space freed (the journal
    /// vacuums incrementally).
    pub fn reclaim(&self, pages: u32) -> Result<()> {
        self.conn.query_row(&format!("PRAGMA incremental_vacuum({pages})"), [], |_| Ok(())).optional()?;
        Ok(())
    }

    /// How big the board is ([`BoardSize`]).
    pub fn board_size(&self) -> Result<BoardSize> {
        board_size_in(&self.conn, &self.path)
    }
}

/// How big the board of the journal at `db` is ([`BoardSize`]), read
/// without opening it for writing; `None` when it can't be read.
pub fn board_size_at(db: &std::path::Path) -> Option<BoardSize> {
    use rusqlite::OpenFlags;
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).ok()?;
    board_size_in(&conn, db).ok()
}

fn board_size_in(conn: &Connection, path: &std::path::Path) -> Result<BoardSize> {
    {
        let cutoff = (now() as u64).saturating_sub(ARCHIVE_AFTER) as i64;
        // A journal from before notes were kept apart has no `note_full` yet: every note is on the board.
        let kept_apart = conn.prepare("SELECT note_full FROM tickets LIMIT 0").is_ok();
        let not_apart = if kept_apart { "note_full IS NULL" } else { "1" };
        let (tickets, closed, note_bytes, to_archive): (i64, i64, i64, i64) = conn.query_row(&format!(
            "SELECT COUNT(*), COALESCE(SUM(status IN ('done', 'cancelled')), 0), COALESCE(SUM(length(CAST(note AS BLOB))), 0),
                    COALESCE(SUM({not_apart} AND length(CAST(note AS BLOB)) > ?1 AND status IN ('done', 'cancelled') AND updated_at < ?2), 0)
             FROM tickets"),
            params![ARCHIVE_OVER as i64, cutoff],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        let (archived, archived_bytes, archived_stored): (i64, i64, i64) = conn
            .query_row("SELECT COUNT(*), COALESCE(SUM(bytes), 0), COALESCE(SUM(length(full)), 0) FROM note_archive", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap_or((0, 0, 0));
        let file_bytes = ["", "-wal"]
            .iter()
            .filter_map(|suffix| std::fs::metadata(format!("{}{suffix}", path.display())).ok())
            .map(|m| m.len())
            .sum();
        let n = |v: i64| v.max(0) as u64;
        Ok(BoardSize {
            tickets: n(tickets),
            closed: n(closed),
            note_bytes: n(note_bytes),
            archived: n(archived),
            archived_bytes: n(archived_bytes),
            archived_stored: n(archived_stored),
            to_archive: n(to_archive),
            file_bytes,
        })
    }
}

/// Keep apart the long notes due on every squad's board under `home`
/// ([`Journal::archive_old_notes`]), a few at a time with a pause between,
/// each squad through its own connection: off the daemon's loop, never
/// holding the squad's file for long. Then give the space back. What was
/// done, per squad, `(squad, notes, bytes)`.
pub fn archive_all(home: &std::path::Path, pause: std::time::Duration) -> Vec<(String, usize, u64)> {
    const BATCH: usize = 8;
    let mut done = Vec::new();
    let Ok(dirs) = std::fs::read_dir(home.join("squads")) else { return done };
    for dir in dirs.flatten() {
        let db = dir.path().join("squad.db");
        if !db.is_file() {
            continue;
        }
        let Ok(journal) = Journal::open(&db) else { continue };
        let (mut notes, mut bytes) = (0, 0);
        while let Ok((n, b)) = journal.archive_old_notes(now() as u64, BATCH) {
            notes += n;
            bytes += b;
            if n < BATCH {
                break;
            }
            std::thread::sleep(pause);
        }
        if notes > 0 {
            // A few hundred pages at a time: each step holds the file briefly.
            for _ in 0..200 {
                let before = journal.conn.query_row("PRAGMA freelist_count", [], |r| r.get::<_, i64>(0)).unwrap_or(0);
                if before == 0 || journal.reclaim(256).is_err() {
                    break;
                }
                std::thread::sleep(pause);
            }
            done.push((dir.file_name().to_string_lossy().into_owned(), notes, bytes));
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::{Ticket, TicketKind, TicketStatus};

    fn ticket() -> Ticket {
        Ticket {
            id: 0,
            title: "Settings page".into(),
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
            design_qa: None,
            landing: false,
            note_full: None,
            plan: None,
            model: None,
            flow: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            label: None,
            remaining: Vec::new(),
            images: Vec::new(),
            updated_at: None,
            priority: Default::default(),
            rank: 0,
            kind: TicketKind::Code,
        }
    }

    fn journal() -> Journal {
        let dir = std::env::temp_dir().join(format!("equipe-notes-{}-{}", std::process::id(), rand_suffix()));
        Journal::for_squad(&dir, "acme").unwrap()
    }

    fn rand_suffix() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed) + std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos() as u64
    }

    fn hand_in(files: usize) -> String {
        let mut note = String::from("Built the settings page; tests pass.\n\nIn the squad's folder:");
        for i in 0..files {
            note.push_str(&format!("\n- outputs/T-1 settings/screens/shot-{i:05}.png"));
        }
        note
    }

    fn put(j: &Journal, status: TicketStatus, note: &str) -> TicketId {
        let mut t = ticket();
        t.status = status;
        t.note = Some(note.to_owned());
        let id = j.create_ticket(&t).unwrap();
        t.id = id;
        j.save_ticket(&t).unwrap();
        id
    }

    fn age(j: &Journal, id: TicketId, secs: u64) {
        j.conn.execute("UPDATE tickets SET updated_at = updated_at - ?1 WHERE id = ?2", params![secs as i64, id]).unwrap();
    }

    fn on_board(j: &Journal, id: TicketId) -> Ticket {
        j.board().unwrap().iter().find(|t| t.id == id).cloned().unwrap()
    }

    #[test]
    fn a_huge_hand_in_keeps_its_start_on_the_board_and_the_whole_apart() {
        let j = journal();
        let note = hand_in(5_000);
        assert!(note.len() > NOTE_WRITE_CAP);
        let id = put(&j, TicketStatus::Review, &note);
        let t = on_board(&j, id);
        let shown = t.note.as_deref().unwrap();
        assert!(shown.starts_with("Built the settings page; tests pass."), "{shown}");
        assert!(shown.chars().count() <= OPEN_NOTE_SHOWN + 2 && shown.ends_with('…'));
        assert_eq!(t.note_full, Some(note.len() as u64));
        assert_eq!(j.full_note(id).unwrap().as_deref(), Some(note.as_str()), "nothing lost");
        // Written back as read — the start, or the whole read before — it stays apart.
        j.save_ticket(&t).unwrap();
        assert_eq!(j.full_note(id).unwrap().as_deref(), Some(note.as_str()));
        assert_eq!(on_board(&j, id).note_full, Some(note.len() as u64));
        let mut whole = t.clone();
        whole.note = Some(note.clone());
        j.save_ticket(&whole).unwrap();
        assert_eq!(on_board(&j, id).note.as_deref(), Some(shown));
        assert_eq!(j.full_note(id).unwrap().as_deref(), Some(note.as_str()));
        // A new note replaces it.
        let mut reopened = on_board(&j, id);
        reopened.status = TicketStatus::Todo;
        reopened.note = Some("Reopened: the header overlaps".into());
        j.save_ticket(&reopened).unwrap();
        let back = on_board(&j, id);
        assert_eq!((back.note.as_deref(), back.note_full), (Some("Reopened: the header overlaps"), None));
        assert_eq!(j.full_note(id).unwrap().as_deref(), Some("Reopened: the header overlaps"));
        assert_eq!(j.board_size().unwrap().archived, 0);
    }

    #[test]
    fn a_closed_tickets_long_note_is_kept_apart_a_day_after_it_closed() {
        let j = journal();
        let note = hand_in(300);
        assert!(note.len() > ARCHIVE_OVER && note.len() < NOTE_WRITE_CAP);
        let fresh = put(&j, TicketStatus::Done, &note);
        let old = put(&j, TicketStatus::Done, &note);
        let open = put(&j, TicketStatus::Review, &note);
        let short = put(&j, TicketStatus::Done, "Done.");
        for id in [old, open, short] {
            age(&j, id, ARCHIVE_AFTER + 60);
        }
        let before = j.board_size().unwrap();
        assert_eq!(before.to_archive, 1);
        let (kept, freed) = j.archive_old_notes(now() as u64, 10).unwrap();
        assert_eq!(kept, 1, "only the long note of a ticket closed a day ago");
        assert!(freed > 5_000);
        let t = on_board(&j, old);
        assert!(t.note.as_deref().unwrap().chars().count() <= equipe_protocol::CLOSED_NOTE_SHOWN + 2);
        assert_eq!(t.note_full, Some(note.len() as u64));
        assert_eq!(j.full_note(old).unwrap().as_deref(), Some(note.as_str()));
        for id in [fresh, open] {
            assert_eq!(on_board(&j, id).note.as_deref(), Some(note.as_str()), "T-{id} as it was");
        }
        assert_eq!(on_board(&j, short).note.as_deref(), Some("Done."));
        let after = j.board_size().unwrap();
        assert_eq!((after.archived, after.archived_bytes, after.to_archive), (1, note.len() as u64, 0));
        assert!(after.note_bytes < before.note_bytes && after.archived_stored < after.archived_bytes, "{after:?}");
        assert_eq!(j.archive_old_notes(now() as u64, 10).unwrap().0, 0, "once");
    }

    #[test]
    fn the_background_pass_keeps_every_squads_due_notes_apart_and_gives_the_space_back() {
        let home = std::env::temp_dir().join(format!("equipe-notes-home-{}-{}", std::process::id(), rand_suffix()));
        let j = Journal::for_squad(&home, "acme").unwrap();
        let ids: Vec<TicketId> = (0..20).map(|_| put(&j, TicketStatus::Done, &hand_in(400))).collect();
        for id in &ids {
            age(&j, *id, ARCHIVE_AFTER + 1);
        }
        let done = archive_all(&home, std::time::Duration::ZERO);
        assert_eq!(done.len(), 1);
        assert_eq!((done[0].0.as_str(), done[0].1), ("acme", 20));
        // The daemon's own connection sees it, once it reads the board whole.
        let fresh = Journal::for_squad(&home, "acme").unwrap();
        assert!(fresh.board().unwrap().iter().all(|t| t.note_full.is_some()));
        assert!(archive_all(&home, std::time::Duration::ZERO).is_empty(), "nothing left to do");
    }

    #[test]
    fn a_note_another_connection_shortened_is_left_alone() {
        let j = journal();
        let id = put(&j, TicketStatus::Done, &hand_in(300));
        age(&j, id, ARCHIVE_AFTER + 1);
        let other = Journal::open(&j.path).unwrap();
        let mut t = other.board().unwrap().iter().find(|t| t.id == id).cloned().unwrap();
        t.note = Some("Changed".into());
        other.save_ticket(&t).unwrap();
        age(&j, id, ARCHIVE_AFTER + 1);
        assert_eq!(j.archive_old_notes(now() as u64, 10).unwrap().0, 0);
        assert_eq!(j.full_note(id).unwrap().as_deref(), Some("Changed"));
    }
}
