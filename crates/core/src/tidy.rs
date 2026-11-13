//! Tidying the person's conversations with the PM, as the Claude app's
//! history does it, by itself: one left alone for days is put away — kept,
//! listed, to be taken up again, nothing deleted.
//!
//! A put-away is only for a conversation nobody is waiting on. Everything
//! that would make it matter blocks it: a turn running or queued, a question,
//! permission, proposal or decision in words it put to the person, helpers at
//! work for it, and the person's own pin.
//!
//! There is no link from a conversation to the tickets it made: a ticket
//! records the member that created it (`created_by`: the PM), not which of the
//! PM's conversations. So "no ticket it created is still open" cannot be
//! checked per conversation; the other conditions stand on their own, and the
//! board keeps showing every open ticket whichever conversation made it.

use std::time::Duration;

/// A conversation with no activity for this long is put away.
pub const IDLE_AFTER: u64 = 3 * 24 * 60 * 60;

/// One the person never said anything in (an empty draft) goes after this long.
pub const EMPTY_DRAFT_AFTER: u64 = 24 * 60 * 60;

/// How often the daemon looks (and once at start).
pub const EVERY: Duration = Duration::from_secs(60 * 60);

/// One still titled by nobody, whose first exchange never finished, takes its
/// title from the person's first words once it has been quiet this long.
pub const TITLE_AFTER: u64 = 60 * 60;

/// What decides whether a conversation is put away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Facts {
    /// When something last happened in it (unix seconds).
    pub last_active: u64,
    pub pinned: bool,
    /// The person has said something in it.
    pub said: bool,
    /// A turn is running or preparing, memos or steering wait for one, or
    /// helpers work for it.
    pub busy: bool,
    /// Something it put to the person is waiting for them: a question, a
    /// permission, a proposal, a decision in words.
    pub asked: bool,
}

/// Whether a conversation is put away now.
pub fn due(f: &Facts, now: u64) -> bool {
    if f.pinned || f.busy || f.asked {
        return false;
    }
    let quiet = now.saturating_sub(f.last_active);
    if f.said { quiet >= IDLE_AFTER } else { quiet >= EMPTY_DRAFT_AFTER }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn old() -> Facts {
        Facts { last_active: NOW - IDLE_AFTER, said: true, ..Facts::default() }
    }

    #[test]
    fn a_conversation_quiet_for_three_days_is_put_away() {
        assert!(due(&old(), NOW));
        assert!(!due(&Facts { last_active: NOW - IDLE_AFTER + 1, ..old() }, NOW), "a second short of three days");
    }

    #[test]
    fn each_thing_that_matters_blocks_it() {
        for blocked in [Facts { pinned: true, ..old() }, Facts { busy: true, ..old() }, Facts { asked: true, ..old() }] {
            assert!(!due(&blocked, NOW), "{blocked:?}");
        }
    }

    #[test]
    fn an_empty_draft_goes_after_a_day_unless_pinned_or_busy() {
        let draft = Facts { last_active: NOW - EMPTY_DRAFT_AFTER, said: false, ..Facts::default() };
        assert!(due(&draft, NOW));
        assert!(!due(&Facts { last_active: NOW - EMPTY_DRAFT_AFTER + 1, ..draft }, NOW));
        assert!(!due(&Facts { pinned: true, ..draft }, NOW));
        assert!(!due(&Facts { busy: true, ..draft }, NOW));
    }

    #[test]
    fn a_conversation_the_person_spoke_in_waits_the_full_three_days() {
        assert!(!due(&Facts { last_active: NOW - 2 * 24 * 60 * 60, said: true, ..Facts::default() }, NOW));
    }
}
