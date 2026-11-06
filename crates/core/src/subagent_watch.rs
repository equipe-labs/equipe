//! Subagents gone quiet: the watch shared by equipe's helpers
//! ([`crate::helpers`]) and Codex's own subagents on OpenAI.
//!
//! A command's watch looks at its processes ([`crate::command_watch`]); a
//! subagent has none of its own to look at, only what it says — each step,
//! each word, Codex telling of it on its own thread. One heard from in the
//! last [`STUCK_AFTER`] is at work, however long it takes; one silent that
//! long looks stuck, and the person may stop it. It is never stopped for
//! looking stuck: only the person stops a subagent of the engine's own (a
//! helper also has its time limit).

use std::time::Duration;

/// A subagent unheard this long looks stuck.
pub const STUCK_AFTER: Duration = Duration::from_secs(5 * 60);

static AFTER: std::sync::Mutex<Duration> = std::sync::Mutex::new(STUCK_AFTER);

/// How long a subagent goes unheard before it looks stuck, now.
pub fn stuck_after() -> Duration {
    *AFTER.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Another wait, for this whole process: tests, and the fake daemon's screens.
#[doc(hidden)]
pub fn set_stuck_after(after: Duration) {
    *AFTER.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = after;
}

/// What the screens are told of a subagent, as of a look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// It looks stuck now, quiet this many seconds.
    Stuck(u64),
    /// It was marked stuck and has been heard from since.
    Moving,
    /// Nothing new to say.
    Same,
}

/// A subagent quiet for `quiet`, marked stuck already or not, as of a wait of `after`.
pub fn look(quiet: Duration, marked: bool, after: Duration) -> Change {
    match (quiet >= after, marked) {
        (true, false) => Change::Stuck(quiet.as_secs()),
        (false, true) => Change::Moving,
        _ => Change::Same,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_long_enough_it_looks_stuck_once_and_heard_again_it_moves() {
        assert_eq!(look(Duration::from_secs(60), false, STUCK_AFTER), Change::Same, "a minute's thought is work");
        assert_eq!(look(STUCK_AFTER, false, STUCK_AFTER), Change::Stuck(300));
        assert_eq!(look(STUCK_AFTER * 2, true, STUCK_AFTER), Change::Same, "said once");
        assert_eq!(look(Duration::from_secs(2), true, STUCK_AFTER), Change::Moving);
    }
}
