//! The wake gate (`equipe_core::wake_gate`) replayed over a week of real PM
//! wakes from working squads, with the model's part taken from what the PM
//! actually did then (no model runs here).
//!
//! `fixtures/pm_wakes_7d.json` holds, per squad, every turn the PM was
//! handed — the person's messages and equipe's own notes — in order: which
//! gate kind the note was (or none), a hash of its words with durations
//! taken out, how many board changes came before it, whether other news was
//! waiting for the PM, and what the PM did with it (replied, acted on the
//! board or a card, or failed). Nothing of the squads' words is kept.

use std::collections::BTreeMap;

use equipe_core::wake_gate::{Decision, Gate};
use serde::Deserialize;

#[derive(Deserialize)]
struct Squad {
    squad: String,
    events: Vec<Woken>,
}

#[derive(Deserialize)]
struct Woken {
    at: u64,
    kind: Option<String>,
    note: String,
    board: u64,
    news: u32,
    ok: bool,
    replied: bool,
    acted: bool,
}

#[derive(Debug, Default)]
struct Tally {
    handed: usize,
    gate_kinds: usize,
    woke: usize,
    held: usize,
    held_ran: usize,
    held_replied: usize,
    held_acted: usize,
    others: usize,
}

fn text(kind: &str, note: &str) -> String {
    match kind {
        "SelfWatch" => format!("[equipe self-watch] {note}"),
        "WaitingOnPerson" => format!("[Waiting on the person · 3h+] {note}"),
        other => panic!("unknown kind {other}"),
    }
}

fn replay() -> (Tally, BTreeMap<String, usize>) {
    let squads: Vec<Squad> = serde_json::from_str(include_str!("fixtures/pm_wakes_7d.json")).unwrap();
    let mut gate = Gate::default();
    let mut t = Tally::default();
    let mut held_by_squad = BTreeMap::new();
    for squad in &squads {
        for e in &squad.events {
            t.handed += 1;
            let Some(kind) = &e.kind else {
                // The person, a hand-in, a failure, a stop, a routine: never asked, always a turn.
                t.others += 1;
                gate.forget(&squad.squad);
                continue;
            };
            t.gate_kinds += 1;
            match gate.check(&squad.squad, &text(kind, &e.note), e.board, e.news > 0, e.at) {
                Decision::Wake => {
                    t.woke += 1;
                    gate.read(&squad.squad);
                }
                Decision::Skip { digest } => {
                    assert!(digest.starts_with(equipe_core::wake_gate::DIGEST_PREFIX));
                    t.held += 1;
                    *held_by_squad.entry(squad.squad.clone()).or_default() += 1;
                    if e.ok {
                        t.held_ran += 1;
                        t.held_replied += usize::from(e.replied);
                        t.held_acted += usize::from(e.acted);
                    }
                }
            }
        }
    }
    (t, held_by_squad)
}

#[test]
fn a_week_of_real_wakes_replayed() {
    let (t, by_squad) = replay();
    eprintln!("{t:?}\nheld back by squad: {by_squad:?}");
    assert_eq!(t.handed, t.gate_kinds + t.others);
    // Everything that isn't a status note went through: the gate never saw it.
    assert_eq!(t.woke + t.held, t.gate_kinds);
    // Most status notes were repeats over an unchanged board.
    assert!(t.held * 10 >= t.gate_kinds * 6, "held {} of {}", t.held, t.gate_kinds);
    // Of the repeats the PM ran (not failed), it acted on the board or a card in
    // few — re-filing the same card, mostly — and only said the same again in most.
    assert!(t.held_acted * 20 <= t.held_ran, "acted on {} of {} held", t.held_acted, t.held_ran);
}

#[test]
fn every_change_wakes_the_pm() {
    // Over the same fixture: a held-back note never follows a board change,
    // other news, or another kind of turn since the last wake of its kind.
    let squads: Vec<Squad> = serde_json::from_str(include_str!("fixtures/pm_wakes_7d.json")).unwrap();
    for squad in &squads {
        let mut gate = Gate::default();
        let mut last: BTreeMap<String, (String, u64, u64)> = BTreeMap::new();
        for e in &squad.events {
            let Some(kind) = &e.kind else {
                gate.forget(&squad.squad);
                last.clear();
                continue;
            };
            let decision = gate.check(&squad.squad, &text(kind, &e.note), e.board, e.news > 0, e.at);
            let same = last.get(kind).is_some_and(|(note, board, at)| *note == e.note && *board == e.board && e.at - at < equipe_core::wake_gate::DEDUPE_FOR);
            match decision {
                Decision::Skip { .. } => assert!(same && e.news == 0, "{}: held back a change at {}", squad.squad, e.at),
                Decision::Wake => {
                    assert!(!same || e.news > 0, "{}: woke for a repeat at {}", squad.squad, e.at);
                    last.insert(kind.clone(), (e.note.clone(), e.board, e.at));
                    gate.read(&squad.squad);
                }
            }
        }
    }
}
