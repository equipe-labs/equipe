//! Status-only PM wakes that change nothing are answered in code, not by a
//! model turn.
//!
//! equipe hands its PM status notes as turns of their own — "nothing has run
//! for 30m", "these wait on the person". When the same note comes again over
//! the same board, a turn spent on it reads the board, finds what it found
//! last time and says "nothing changed" to the person: a model call, and a
//! message nobody needed. This gate keeps the second one out: it is counted,
//! and the PM reads one line about it with its next real turn.
//!
//! Only notes listed in [`Kind`] are ever held back, and only when all of
//! these hold:
//!
//! - the very same note (durations aside) woke the PM within [`DEDUPE_FOR`];
//! - the board (every ticket's status, owner and last change) and what waits
//!   on the person are as they were then;
//! - nothing else waits in the PM's briefing (an engine back, room on the
//!   disk, another finding);
//! - the PM has had no turn of another kind since (the person's message, an
//!   answer, a hand-in, a failure: [`Gate::forget`]).
//!
//! Everything else wakes the PM as before: the person's messages, answers and
//! approvals, blocked or failed work, hand-ins to accept, stops, routines,
//! errors and anything on a clock never come through here as a [`Kind`].

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};

/// How long a repeat of the same note over the same board is held back.
pub const DEDUPE_FOR: u64 = 6 * 3600;

/// The start of the line the PM reads about held-back notes.
pub const DIGEST_PREFIX: &str = "Status notes held back since your last turn";

/// The status notes the gate may hold back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// The squad's self-watch (`crate::health`): idle board, stalled work.
    SelfWatch,
    /// What has waited on the person for hours (`crate::importance::stale_message`).
    WaitingOnPerson,
}

impl Kind {
    /// Which held-back kind `text` is, by the tag equipe opens it with.
    pub fn of(text: &str) -> Option<Kind> {
        let t = text.trim_start();
        if t.starts_with("[equipe self-watch]") {
            Some(Kind::SelfWatch)
        } else if t.starts_with("[Waiting on the person") {
            Some(Kind::WaitingOnPerson)
        } else {
            None
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::SelfWatch => "self-watch",
            Kind::WaitingOnPerson => "waiting on the person",
        }
    }
}

/// `text` with its durations taken out ("for 31m", "waited 2h", "3d"):
/// the same note an hour on is the same note.
pub fn normalized(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            let mut j = i;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            // A duration: digits, one unit letter, then no more letters ("31m", "2h", "3d", "45s").
            let unit = chars.get(j).is_some_and(|c| matches!(c, 'm' | 'h' | 'd' | 's'));
            let ends = chars.get(j + 1).is_none_or(|c| !c.is_alphanumeric());
            if unit && ends {
                out.push('#');
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// A fingerprint of what the PM acts on: any change makes the next note news.
pub fn fingerprint<T: Hash>(parts: &T) -> u64 {
    let mut h = DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Woke {
    note: u64,
    board: u64,
    at: u64,
}

/// What the gate decided about one note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Wake the PM, as before.
    Wake,
    /// Held back: the PM's briefing line now reads `digest`.
    Skip { digest: String },
}

/// The gate's memory, per squad: what last woke the PM, by kind, and what
/// was held back since its last turn. In memory only — after a restart the
/// first note wakes the PM again, as it always did.
#[derive(Debug, Default)]
pub struct Gate {
    woke: HashMap<(String, Kind), Woke>,
    held: HashMap<String, BTreeMap<Kind, u32>>,
}

impl Gate {
    /// Whether `text`, about to wake `squad`'s PM over a board fingerprinted
    /// `board`, wakes it. `news`: something else waits for the PM to read
    /// (its briefing) — it wakes, whatever the note.
    pub fn check(&mut self, squad: &str, text: &str, board: u64, news: bool, now: u64) -> Decision {
        let Some(kind) = Kind::of(text) else { return Decision::Wake };
        let note = fingerprint(&normalized(text));
        let key = (squad.to_owned(), kind);
        let repeat = !news && self.woke.get(&key).is_some_and(|w| w.note == note && w.board == board && now.saturating_sub(w.at) < DEDUPE_FOR);
        if !repeat {
            self.woke.insert(key, Woke { note, board, at: now });
            return Decision::Wake;
        }
        let held = self.held.entry(squad.to_owned()).or_default();
        *held.entry(kind).or_default() += 1;
        Decision::Skip { digest: digest(held) }
    }

    /// The PM took a turn: what was held back was read with it.
    pub fn read(&mut self, squad: &str) {
        self.held.remove(squad);
    }

    /// The PM took a turn of another kind (the person, an answer, a hand-in):
    /// the next note wakes it whatever it says.
    pub fn forget(&mut self, squad: &str) {
        self.woke.retain(|(s, _), _| s != squad);
        self.read(squad);
    }
}

/// The one line the PM reads about what was held back.
pub fn digest(held: &BTreeMap<Kind, u32>) -> String {
    let parts: Vec<String> = held.iter().map(|(k, n)| format!("{} ×{n}", k.label())).collect();
    format!(
        "{DIGEST_PREFIX}, the board and the waits unchanged: {}. You already saw these; act only on what changed.",
        parts.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: &str = "[equipe self-watch] Nothing has run for 31m while 4 tickets wait to be done. They wait on: T-62 \"Gallery\" (in progress, holds 2).";

    #[test]
    fn kinds_by_tag() {
        assert_eq!(Kind::of(IDLE), Some(Kind::SelfWatch));
        assert_eq!(Kind::of("[Waiting on the person · 3h+] These have waited"), Some(Kind::WaitingOnPerson));
        for never in [
            "[Stopped by the person] T-3",
            "[equipe: handed-in work still waits for your review: T-4]",
            "[Stale waits · 21]",
            "[Routine · Weekly report]",
            "[equipe: T-9 couldn't start for kai: build failed. Settle it now]",
            "Can you change the headline?",
        ] {
            assert_eq!(Kind::of(never), None, "{never}");
        }
    }

    #[test]
    fn durations_are_not_news() {
        let later = IDLE.replace("31m", "2h");
        assert_eq!(normalized(IDLE), normalized(&later));
        // Ticket numbers and counts are.
        assert_ne!(normalized(IDLE), normalized(&IDLE.replace("T-62", "T-63")));
        assert_ne!(normalized(IDLE), normalized(&IDLE.replace("4 tickets", "5 tickets")));
        assert_ne!(normalized(IDLE), normalized(&IDLE.replace("holds 2", "holds 3")));
    }

    #[test]
    fn a_repeat_over_the_same_board_is_held_once_then_counted() {
        let mut g = Gate::default();
        assert_eq!(g.check("s", IDLE, 1, false, 0), Decision::Wake);
        let Decision::Skip { digest } = g.check("s", &IDLE.replace("31m", "1h"), 1, false, 1800) else { panic!() };
        assert!(digest.contains("self-watch ×1"), "{digest}");
        let Decision::Skip { digest } = g.check("s", IDLE, 1, false, 3600) else { panic!() };
        assert!(digest.contains("self-watch ×2"), "{digest}");
        // Another squad is its own.
        assert_eq!(g.check("t", IDLE, 1, false, 3600), Decision::Wake);
    }

    #[test]
    fn anything_changed_wakes() {
        let mut g = Gate::default();
        assert_eq!(g.check("s", IDLE, 1, false, 0), Decision::Wake);
        assert_eq!(g.check("s", IDLE, 2, false, 60), Decision::Wake, "the board moved");
        assert_eq!(g.check("s", &IDLE.replace("T-62", "T-70"), 2, false, 120), Decision::Wake, "the note says something new");
        assert_eq!(g.check("s", &IDLE.replace("T-62", "T-70"), 2, false, 120 + DEDUPE_FOR), Decision::Wake, "long enough ago");
        assert_eq!(g.check("s", &IDLE.replace("T-62", "T-70"), 2, true, 125 + DEDUPE_FOR), Decision::Wake, "news waits for the PM");
        g.forget("s");
        assert_eq!(g.check("s", &IDLE.replace("T-62", "T-70"), 2, false, 130 + DEDUPE_FOR), Decision::Wake, "the PM had another turn");
    }

    #[test]
    fn notes_outside_the_kinds_always_wake() {
        let mut g = Gate::default();
        let hand_in = "[equipe: handed-in work still waits for your review: T-4 \"Hero\"]";
        for at in 0..5 {
            assert_eq!(g.check("s", hand_in, 1, false, at * 60), Decision::Wake);
        }
    }

    #[test]
    fn digest_counts_by_kind_and_clears_when_read() {
        let mut g = Gate::default();
        let waits = "[Waiting on the person · 3h+] These have waited on the person 3h or more:\n- T-5 Domain";
        g.check("s", IDLE, 1, false, 0);
        g.check("s", waits, 1, false, 0);
        g.check("s", IDLE, 1, false, 10);
        let Decision::Skip { digest } = g.check("s", waits, 1, false, 20) else { panic!() };
        assert!(digest.starts_with(DIGEST_PREFIX));
        assert!(digest.contains("self-watch ×1, waiting on the person ×1"), "{digest}");
        g.read("s");
        let Decision::Skip { digest } = g.check("s", IDLE, 1, false, 30) else { panic!() };
        assert!(digest.contains("self-watch ×1") && !digest.contains("waiting"), "{digest}");
    }
}
