//! The screen's lease: one member at a time, the others in line.
//!
//! A member gets it with its first desktop tool call, or waits in line behind
//! whoever has it. It ends when the member's turn ends, when it lets go, or
//! when it hasn't used the desktop for `idle` seconds; the next in line gets it
//! then. The person can take it back from everyone (`stop`) and open it again
//! (`resume`). Kept in memory: a restarted daemon starts with a free screen.

use std::collections::VecDeque;

/// What a member asking for the screen is told.
#[derive(Debug, Clone, PartialEq)]
pub enum Turn<W> {
    /// It has the screen.
    Yours,
    /// `holder` has it; the asker is `place`th in line (1: next).
    Waiting { holder: W, place: usize },
    /// The person took the screen back.
    Stopped,
}

#[derive(Debug, Clone)]
pub struct Lease<W> {
    /// Who has it, and when it last used the desktop.
    holder: Option<(W, f64)>,
    queue: VecDeque<W>,
    stopped: bool,
    idle: f64,
}

impl<W: Clone + PartialEq> Lease<W> {
    pub fn new(idle: f64) -> Self {
        Self { holder: None, queue: VecDeque::new(), stopped: false, idle }
    }

    pub fn holder(&self) -> Option<&W> {
        self.holder.as_ref().map(|(w, _)| w)
    }

    pub fn queue(&self) -> impl Iterator<Item = &W> {
        self.queue.iter()
    }

    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// `who` wants the screen at `now`: it has it, gets it, or waits in line.
    pub fn ask(&mut self, who: &W, now: f64) -> Turn<W> {
        if self.stopped {
            return Turn::Stopped;
        }
        self.expire(now);
        match &mut self.holder {
            Some((holder, used)) if holder == who => {
                *used = now;
                return Turn::Yours;
            }
            Some(_) => {}
            None => {
                // The screen is free: the next in line has it before anyone new.
                match self.queue.front() {
                    Some(next) if next != who => {}
                    _ => {
                        self.queue.retain(|w| w != who);
                        self.holder = Some((who.clone(), now));
                        return Turn::Yours;
                    }
                }
            }
        }
        if !self.queue.contains(who) {
            self.queue.push_back(who.clone());
        }
        let place = self.queue.iter().position(|w| w == who).map_or(0, |i| i + 1);
        let holder = self.holder().or(self.queue.front()).cloned().expect("someone is ahead");
        Turn::Waiting { holder, place }
    }

    /// `who` used the desktop at `now` (a call it made has ended).
    pub fn used(&mut self, who: &W, now: f64) {
        if let Some((holder, used)) = &mut self.holder
            && holder == who
        {
            *used = now;
        }
    }

    /// `who` is done with the screen: it lets go, or leaves the line. Returns
    /// whether anything changed.
    pub fn leave(&mut self, who: &W, now: f64) -> bool {
        let before = self.queue.len();
        self.queue.retain(|w| w != who);
        let mut changed = before != self.queue.len();
        if self.holder() == Some(who) {
            self.holder = None;
            self.hand_on(now);
            changed = true;
        }
        changed
    }

    /// Let go of a holder that hasn't used the desktop for too long. Returns
    /// whether anything changed.
    pub fn expire(&mut self, now: f64) -> bool {
        match &self.holder {
            Some((_, used)) if now - used >= self.idle => {
                self.holder = None;
                self.hand_on(now);
                true
            }
            _ => false,
        }
    }

    /// The person takes the screen back: nobody has it, nobody waits.
    pub fn stop(&mut self) -> bool {
        let changed = !self.stopped || self.holder.is_some() || !self.queue.is_empty();
        self.stopped = true;
        self.holder = None;
        self.queue.clear();
        changed
    }

    /// The person opens the screen to the members again.
    pub fn resume(&mut self) -> bool {
        std::mem::replace(&mut self.stopped, false)
    }

    /// The next in line gets the screen, its idle time counted from now.
    fn hand_on(&mut self, now: f64) {
        if let Some(next) = self.queue.pop_front() {
            self.holder = Some((next, now));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_to_ask_gets_it_and_the_others_wait_in_line() {
        let mut lease = Lease::new(120.0);
        assert_eq!(lease.ask(&"frontend", 0.0), Turn::Yours);
        assert_eq!(lease.ask(&"backend", 1.0), Turn::Waiting { holder: "frontend", place: 1 });
        assert_eq!(lease.ask(&"designer", 2.0), Turn::Waiting { holder: "frontend", place: 2 });
        // Asking again keeps one's place.
        assert_eq!(lease.ask(&"backend", 3.0), Turn::Waiting { holder: "frontend", place: 1 });
        assert_eq!(lease.ask(&"frontend", 4.0), Turn::Yours);
    }

    #[test]
    fn letting_go_hands_it_to_the_next_in_line_not_to_whoever_asks_first() {
        let mut lease = Lease::new(120.0);
        lease.ask(&"frontend", 0.0);
        lease.ask(&"backend", 1.0);
        assert!(lease.leave(&"frontend", 2.0));
        assert_eq!(lease.holder(), Some(&"backend"));
        assert_eq!(lease.ask(&"designer", 3.0), Turn::Waiting { holder: "backend", place: 1 });
        assert_eq!(lease.ask(&"backend", 4.0), Turn::Yours);
    }

    #[test]
    fn leaving_the_line_moves_the_others_up() {
        let mut lease = Lease::new(120.0);
        lease.ask(&"a", 0.0);
        lease.ask(&"b", 0.0);
        lease.ask(&"c", 0.0);
        assert!(lease.leave(&"b", 1.0));
        assert_eq!(lease.ask(&"c", 2.0), Turn::Waiting { holder: "a", place: 1 });
        assert!(!lease.leave(&"nobody", 3.0));
    }

    #[test]
    fn an_idle_holder_loses_it() {
        let mut lease = Lease::new(120.0);
        lease.ask(&"frontend", 0.0);
        lease.used(&"frontend", 50.0);
        assert!(!lease.expire(169.0));
        lease.ask(&"backend", 100.0);
        assert!(lease.expire(170.0));
        assert_eq!(lease.holder(), Some(&"backend"), "the idle time counts from its last use");
        // The one handed it has its own idle time from then.
        assert!(!lease.expire(289.0));
        assert!(lease.expire(290.0));
        assert_eq!(lease.holder(), None);
    }

    #[test]
    fn a_free_screen_goes_to_whoever_asks() {
        let mut lease = Lease::new(10.0);
        lease.ask(&"a", 0.0);
        assert_eq!(lease.ask(&"b", 20.0), Turn::Yours, "a went idle");
    }

    #[test]
    fn stop_takes_it_from_everyone_until_resume() {
        let mut lease = Lease::new(120.0);
        lease.ask(&"frontend", 0.0);
        lease.ask(&"backend", 0.0);
        assert!(lease.stop());
        assert_eq!(lease.holder(), None);
        assert_eq!(lease.queue().count(), 0);
        assert_eq!(lease.ask(&"frontend", 1.0), Turn::Stopped);
        assert!(!lease.stop());
        assert!(lease.resume());
        assert_eq!(lease.ask(&"backend", 2.0), Turn::Yours);
    }
}
