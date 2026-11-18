//! Long ticket notes kept apart from the board (`journal::notes`): written
//! over the cap, a day after closing, by the background pass — and read back
//! whole. In a process of its own: a lib test elsewhere makes every journal
//! of its process unable to grow, and these write megabytes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use equipe_core::journal::{self, ARCHIVE_AFTER, ARCHIVE_OVER, Journal, NOTE_WRITE_CAP};
use equipe_core::protocol::{CLOSED_NOTE_SHOWN, Ticket, TicketId, TicketKind, TicketStatus};

/// How much of a capped open note stays on the board (`notes::OPEN_NOTE_SHOWN`).
const OPEN_NOTE_SHOWN: usize = 4_000;

fn home() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("equipe-note-archive-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

fn db(home: &Path) -> PathBuf {
    journal::squad_dir(home, "acme").join("squad.db")
}

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

/// As if it had last changed `secs` ago.
fn age(home: &Path, id: TicketId, secs: u64) {
    let conn = rusqlite::Connection::open(db(home)).unwrap();
    conn.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
    conn.execute("UPDATE tickets SET updated_at = updated_at - ?1 WHERE id = ?2", rusqlite::params![secs as i64, id]).unwrap();
}

fn on_board(j: &Journal, id: TicketId) -> Ticket {
    j.board().unwrap().iter().find(|t| t.id == id).cloned().unwrap()
}

#[test]
fn a_huge_hand_in_keeps_its_start_on_the_board_and_the_whole_apart() {
    let j = Journal::for_squad(&home(), "acme").unwrap();
    let note = hand_in(5_000);
    assert!(note.len() > NOTE_WRITE_CAP);
    let id = put(&j, TicketStatus::Review, &note);
    let t = on_board(&j, id);
    let shown = t.note.clone().unwrap();
    assert!(shown.starts_with("Built the settings page; tests pass."), "{shown}");
    assert!(shown.chars().count() <= OPEN_NOTE_SHOWN + 2 && shown.ends_with('…'));
    assert_eq!(t.note_full, Some(note.len() as u64));
    assert_eq!(j.full_note(id).unwrap().as_deref(), Some(note.as_str()), "nothing lost");
    // Written back as read — the start, or the whole read before — it stays apart.
    j.save_ticket(&t).unwrap();
    assert_eq!(j.full_note(id).unwrap().as_deref(), Some(note.as_str()));
    assert_eq!(on_board(&j, id).note_full, Some(note.len() as u64));
    let whole = Ticket { note: Some(note.clone()), ..t.clone() };
    j.save_ticket(&whole).unwrap();
    assert_eq!(on_board(&j, id).note.as_deref(), Some(shown.as_str()));
    assert_eq!(j.full_note(id).unwrap().as_deref(), Some(note.as_str()));
    // A new note replaces it.
    let reopened = Ticket { status: TicketStatus::Todo, note: Some("Reopened: the header overlaps".into()), ..on_board(&j, id) };
    j.save_ticket(&reopened).unwrap();
    let back = on_board(&j, id);
    assert_eq!((back.note.as_deref(), back.note_full), (Some("Reopened: the header overlaps"), None));
    assert_eq!(j.full_note(id).unwrap().as_deref(), Some("Reopened: the header overlaps"));
    assert_eq!(j.board_size().unwrap().archived, 0);
}

#[test]
fn a_closed_tickets_long_note_is_kept_apart_a_day_after_it_closed() {
    let home = home();
    let j = Journal::for_squad(&home, "acme").unwrap();
    let note = hand_in(300);
    assert!(note.len() > ARCHIVE_OVER && note.len() < NOTE_WRITE_CAP);
    let fresh = put(&j, TicketStatus::Done, &note);
    let old = put(&j, TicketStatus::Done, &note);
    let open = put(&j, TicketStatus::Review, &note);
    let short = put(&j, TicketStatus::Done, "Done.");
    for id in [old, open, short] {
        age(&home, id, ARCHIVE_AFTER + 60);
    }
    let before = j.board_size().unwrap();
    assert_eq!(before.to_archive, 1);
    let (kept, freed) = j.archive_old_notes(now(), 10).unwrap();
    assert_eq!(kept, 1, "only the long note of a ticket closed a day ago");
    assert!(freed > 5_000);
    let fresh_journal = Journal::for_squad(&home, "acme").unwrap();
    let t = on_board(&fresh_journal, old);
    assert!(t.note.as_deref().unwrap().chars().count() <= CLOSED_NOTE_SHOWN + 2);
    assert_eq!(t.note_full, Some(note.len() as u64));
    assert_eq!(j.full_note(old).unwrap().as_deref(), Some(note.as_str()));
    for id in [fresh, open] {
        assert_eq!(on_board(&fresh_journal, id).note.as_deref(), Some(note.as_str()), "T-{id} as it was");
    }
    assert_eq!(on_board(&fresh_journal, short).note.as_deref(), Some("Done."));
    let after = j.board_size().unwrap();
    assert_eq!((after.archived, after.archived_bytes, after.to_archive), (1, note.len() as u64, 0));
    assert!(after.note_bytes < before.note_bytes && after.archived_stored < after.archived_bytes, "{after:?}");
    assert_eq!(journal::board_size_at(&db(&home)), Some(after), "read without writing, the same");
    assert_eq!(j.archive_old_notes(now(), 10).unwrap().0, 0, "once");
}

#[test]
fn the_background_pass_keeps_every_squads_due_notes_apart_and_gives_the_space_back() {
    let home = home();
    let j = Journal::for_squad(&home, "acme").unwrap();
    // Long, but under the cap a note is kept apart at as it is written.
    let ids: Vec<TicketId> = (0..20).map(|_| put(&j, TicketStatus::Done, &hand_in(300))).collect();
    assert_eq!(j.board_size().unwrap().archived, 0);
    for id in &ids {
        age(&home, *id, ARCHIVE_AFTER + 1);
    }
    let done = journal::archive_all_notes(&home, std::time::Duration::ZERO);
    assert_eq!(done.len(), 1, "{done:?}");
    assert_eq!((done[0].0.as_str(), done[0].1), ("acme", 20));
    // The daemon's own connection sees it, once it reads the board whole.
    let fresh = Journal::for_squad(&home, "acme").unwrap();
    assert!(fresh.board().unwrap().iter().all(|t| t.note_full.is_some()));
    assert!(journal::archive_all_notes(&home, std::time::Duration::ZERO).is_empty(), "nothing left to do");
}

#[test]
fn a_note_another_connection_shortened_is_left_alone() {
    let home = home();
    let j = Journal::for_squad(&home, "acme").unwrap();
    let id = put(&j, TicketStatus::Done, &hand_in(300));
    let other = Journal::open(&db(&home)).unwrap();
    let t = Ticket { note: Some("Changed".into()), ..other.board().unwrap().iter().find(|t| t.id == id).cloned().unwrap() };
    other.save_ticket(&t).unwrap();
    age(&home, id, ARCHIVE_AFTER + 1);
    assert_eq!(j.archive_old_notes(now(), 10).unwrap().0, 0);
    assert_eq!(j.full_note(id).unwrap().as_deref(), Some("Changed"));
}
