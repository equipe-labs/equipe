//! The taste tuner as the screen runs it (`views/tuner.rs` draws it): two
//! real product screens from the project's set side by side, the person
//! picking the one they like more (← or →, S to skip), with one-tap reasons;
//! about twenty picks give a first style profile, more refine it. Which
//! pairs come is the core's (`equipe_core::taste::tuner::next_pairs`: one or
//! two axes varied at a time, the axes asked least first, one pair in six a
//! contrast probe); each pick goes to the daemon at once
//! (`Command::TasteSignals`), which works the profile out again. No UI types
//! here.

use equipe_core::protocol::Command;
use equipe_core::protocol::design::{PickReason, PickSide, TUNER_FIRST, Taste, TasteSignal, TunerItem};
use equipe_core::taste::tuner::{self, Pair};

/// Pairs worked out at a time.
const BATCH: usize = 12;

/// The tuner on screen.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TunerUi {
    /// Open, for this project (`Some(None)`: the squad's own).
    pub open: Option<Option<String>>,
    pub pairs: Vec<Pair>,
    pub at: usize,
    /// The reasons tapped for the pair on screen.
    pub reasons: Vec<PickReason>,
    /// Picked (or skipped) this time, as `(a, b)` pictures.
    pub done: Vec<(String, String)>,
    pub picks: u32,
    pub seed: u64,
}

/// What a key does in the tuner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Pick(PickSide),
    Skip,
    Close,
}

pub fn key(name: &str) -> Option<Key> {
    match name {
        "left" => Some(Key::Pick(PickSide::A)),
        "right" => Some(Key::Pick(PickSide::B)),
        "s" => Some(Key::Skip),
        "escape" => Some(Key::Close),
        _ => None,
    }
}

impl TunerUi {
    pub fn open(&mut self, project: Option<String>, seed: u64) {
        *self = TunerUi { open: Some(project), seed, ..TunerUi::default() };
    }

    pub fn close(&mut self) {
        self.open = None;
    }

    /// The pair on screen: its two pictures, and whether it is a probe;
    /// more are worked out when these run out. `None`: the set has no more.
    pub fn current<'a>(&mut self, taste: &'a Taste) -> Option<(&'a TunerItem, &'a TunerItem, bool)> {
        if self.at >= self.pairs.len() {
            let mut tuned = taste.tuned.clone();
            tuned.extend(self.done.iter().cloned());
            let leans = taste.profile.as_ref().map(|p| p.leans.clone()).unwrap_or_default();
            let made = taste.profile.as_ref().map_or(0, |p| p.picks as usize) + self.done.len();
            self.pairs = tuner::next_pairs(&taste.tuner, &tuned, &leans, made, BATCH, self.seed);
            self.at = 0;
        }
        let pair = self.pairs.get(self.at)?;
        Some((taste.tuner.get(pair.a)?, taste.tuner.get(pair.b)?, pair.probe))
    }

    /// Tap a reason on or off.
    pub fn toggle(&mut self, reason: PickReason) {
        if let Some(at) = self.reasons.iter().position(|r| *r == reason) {
            self.reasons.remove(at);
        } else {
            self.reasons.push(reason);
        }
    }

    /// The person picked (`Some`) or skipped the pair on screen: the command
    /// that keeps it, and the next pair comes.
    pub fn answer(&mut self, squad: &str, a: &str, b: &str, chose: Option<PickSide>) -> Command {
        self.done.push((a.to_owned(), b.to_owned()));
        if chose.is_some() {
            self.picks += 1;
        }
        self.at += 1;
        let reasons = std::mem::take(&mut self.reasons);
        Command::TasteSignals { squad: squad.to_owned(), project: self.open.clone().flatten(), signals: vec![TasteSignal::Pick { a: a.to_owned(), b: b.to_owned(), chose, reasons }] }
    }
}

/// Where the person stands: picks made in all, and how many more until a
/// first profile (0: it is there, and more refine it).
pub fn progress(taste: &Taste, this_time: u32) -> (u32, u32) {
    let all = taste.profile.as_ref().map_or(0, |p| p.picks) + this_time;
    (all, TUNER_FIRST.saturating_sub(all))
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::design::{StyleProfile, TasteAxis};

    fn set() -> Vec<TunerItem> {
        let mut out = vec![TunerItem { picture: "/d/base.png".into(), ..TunerItem::default() }];
        for axis in TasteAxis::ALL {
            for v in [-70, 70] {
                out.push(TunerItem { picture: format!("/d/{}{v}.png", axis.key()), source: axis.key().into(), axes: vec![(axis, v)] });
            }
        }
        out
    }

    #[test]
    fn picks_go_out_one_by_one_with_their_reasons_and_never_repeat_a_pair() {
        let taste = Taste { project: Some("web".into()), tuner: set(), ..Taste::default() };
        let mut ui = TunerUi::default();
        ui.open(Some("web".into()), 3);
        let mut seen = Vec::new();
        for n in 0..20 {
            let (a, b, _) = ui.current(&taste).map(|(a, b, p)| (a.picture.clone(), b.picture.clone(), p)).expect("a pair");
            assert!(!seen.contains(&(a.clone(), b.clone())) && !seen.contains(&(b.clone(), a.clone())), "pair {n} again");
            seen.push((a.clone(), b.clone()));
            if n == 0 {
                ui.toggle(PickReason::TooDense);
                ui.toggle(PickReason::Type);
                ui.toggle(PickReason::Type);
            }
            let chose = if n % 5 == 4 { None } else { Some(PickSide::A) };
            let command = ui.answer("acme", &a, &b, chose);
            let Command::TasteSignals { project, signals, .. } = command else { panic!() };
            assert_eq!(project.as_deref(), Some("web"));
            if n == 0 {
                assert_eq!(signals, [TasteSignal::Pick { a: a.clone(), b: b.clone(), chose: Some(PickSide::A), reasons: vec![PickReason::TooDense] }]);
            }
            assert!(ui.reasons.is_empty(), "reasons are the pair's");
        }
        assert_eq!(ui.picks, 16, "skips aren't picks");
        assert_eq!(progress(&taste, ui.picks), (16, 4));
        let refined = Taste { profile: Some(StyleProfile { picks: 30, ..StyleProfile::default() }), ..taste.clone() };
        assert_eq!(progress(&refined, 0), (30, 0));
        assert_eq!(key("left"), Some(Key::Pick(PickSide::A)));
        assert_eq!(key("s"), Some(Key::Skip));
        assert_eq!(key("x"), None);
        let empty = Taste::default();
        assert!(TunerUi::default().current(&empty).is_none(), "no set, no pairs");
    }
}
