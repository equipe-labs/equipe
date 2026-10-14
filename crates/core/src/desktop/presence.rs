//! Is the person at the Mac? The person comes first: while they touch the mouse
//! or the keyboard — at the Mac, or through Screen Sharing or Jump Desktop — no
//! member's input goes in, and none until they have left it alone for a while.
//!
//! macOS says how long ago the last input event of any kind came
//! (`CGEventSourceSecondsSinceLastEventType`, combined session state). That
//! counts every source: the person's hands, a remote-desktop client, and the
//! members' own clicks. So the members' input is told apart by *when* it ran:
//! equipe knows when it posted, and an event that came at no such time was
//! somebody else's. Hardware-only counters would miss a person working over
//! Jump Desktop, whose input is synthetic too.
//!
//! Plain `std` on a caller's clock (seconds), so it is tested without a Mac and
//! the desktop probe can run this very file on one.

use std::collections::VecDeque;

/// How far an event may land outside the moment equipe posted it and still be
/// its own: the window server timestamps a little after the post.
pub const SLACK: f64 = 0.5;

/// What is known about the person's hands.
#[derive(Debug, Clone)]
pub struct Presence {
    /// How long the person must have left mouse and keyboard alone before a
    /// member may use them.
    quiet: f64,
    /// When equipe's own input ran: `(start, end)`, oldest first.
    own: VecDeque<(f64, f64)>,
    /// When the person last touched the Mac, as far as can be told.
    person_at: Option<f64>,
}

impl Presence {
    pub fn new(quiet: f64) -> Self {
        Self { quiet, own: VecDeque::new(), person_at: None }
    }

    pub fn quiet(&self) -> f64 {
        self.quiet
    }

    /// equipe posted input between `start` and `end`.
    pub fn posted(&mut self, start: f64, end: f64) {
        self.own.push_back((start, end.max(start)));
    }

    /// The last input event of any kind came `idle` seconds before `now`.
    /// Returns how many seconds the members must still wait, `None` when the
    /// person has left the Mac alone long enough.
    pub fn observe(&mut self, now: f64, idle: f64) -> Option<f64> {
        let last = now - idle.max(0.0);
        let ours = self.own.iter().any(|&(start, end)| last >= start - SLACK && last <= end + SLACK);
        if !ours {
            self.person_at = Some(self.person_at.map_or(last, |at| at.max(last)));
        }
        // Older posts can't explain anything that matters any more.
        let horizon = now - self.quiet - SLACK;
        while self.own.front().is_some_and(|&(_, end)| end < horizon) && self.own.len() > 1 {
            self.own.pop_front();
        }
        self.person_at.map(|at| at + self.quiet - now).filter(|&left| left > 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mac_nobody_touched_lately_is_free() {
        let mut p = Presence::new(60.0);
        assert_eq!(p.observe(1000.0, 300.0), None);
    }

    #[test]
    fn the_persons_input_holds_members_back_until_the_quiet_time_has_passed() {
        let mut p = Presence::new(60.0);
        let left = p.observe(1000.0, 5.0).expect("the person touched it 5 s ago");
        assert!((left - 55.0).abs() < 1e-9);
        // Still waiting 30 s later, though the last event is long gone.
        assert!(p.observe(1030.0, 35.0).is_some());
        assert_eq!(p.observe(1056.0, 61.0), None);
    }

    #[test]
    fn a_members_own_clicks_are_not_taken_for_the_person() {
        let mut p = Presence::new(60.0);
        assert_eq!(p.observe(1000.0, 300.0), None);
        p.posted(1001.0, 1001.2);
        // The last event is the click equipe just posted (timestamped a touch late).
        assert_eq!(p.observe(1002.0, 0.6), None);
        p.posted(1010.0, 1012.0);
        assert_eq!(p.observe(1013.0, 1.5), None);
    }

    #[test]
    fn the_person_moving_the_mouse_between_two_member_clicks_is_seen() {
        let mut p = Presence::new(60.0);
        p.posted(1001.0, 1001.2);
        assert_eq!(p.observe(1002.0, 0.8), None);
        // The pointer moved at 1004, when equipe posted nothing.
        assert!(p.observe(1005.0, 1.0).is_some());
        // Even if the next check only sees equipe's later click, the person stays seen.
        p.posted(1006.0, 1006.1);
        assert!(p.observe(1007.0, 0.9).is_some());
    }

    #[test]
    fn old_posts_are_forgotten_but_the_latest_is_kept() {
        let mut p = Presence::new(10.0);
        for i in 0..100 {
            p.posted(f64::from(i), f64::from(i) + 0.1);
        }
        assert_eq!(p.observe(100.0, 1.0 - 0.05), None, "the last post at 99 explains an event at 99.05");
        assert!(p.own.len() < 20);
    }
}
