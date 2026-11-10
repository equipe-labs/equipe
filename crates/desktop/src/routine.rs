//! A routine the PM proposed (`propose_routine`), as the card the person turns
//! it on from — above the composer in the PM's conversation, and in Needs you
//! under "Decisions": which proposals wait, and the command the person's click
//! sends. No UI types here, so the rules are tested without a window. Turned
//! on, the daemon saves it as the person's own routine (`views::routines`
//! lists it from then on); declined, it is gone.

use equipe_core::protocol::Command;
use equipe_core::protocol::routines::RoutineProposal;

/// Proposals waiting for the person, oldest first. One decided here hides at
/// once, so a second click can't send it twice; the daemon's
/// `RoutineProposalClosed` takes it for good.
#[derive(Debug, Clone, Default)]
pub struct Proposals {
    proposals: Vec<RoutineProposal>,
    sent: Vec<u64>,
}

impl Proposals {
    /// The snapshot's proposals replace everything.
    pub fn load(&mut self, proposals: Vec<RoutineProposal>) {
        self.proposals = proposals;
        self.sent.clear();
    }

    /// A routine proposed. Whether it is new here.
    pub fn push(&mut self, proposal: RoutineProposal) -> bool {
        if self.proposals.iter().any(|p| p.id == proposal.id) {
            return false;
        }
        self.proposals.push(proposal);
        true
    }

    /// No longer waiting, whoever decided.
    pub fn close(&mut self, id: u64) {
        self.proposals.retain(|p| p.id != id);
        self.sent.retain(|s| *s != id);
    }

    /// What was sent into a dead link never came back: show it again.
    pub fn forget_sent(&mut self) {
        self.sent.clear();
    }

    /// Every proposal still waiting, oldest first.
    pub fn open(&self) -> impl Iterator<Item = &RoutineProposal> {
        self.proposals.iter().filter(|p| !self.sent.contains(&p.id))
    }

    pub fn get(&self, id: u64) -> Option<&RoutineProposal> {
        self.open().find(|p| p.id == id)
    }

    pub fn first(&self) -> Option<&RoutineProposal> {
        self.open().next()
    }

    pub fn ids(&self) -> Vec<u64> {
        self.open().map(|p| p.id).collect()
    }

    /// The person's answer to proposal `id`: the command to send, the card
    /// hidden at once. `None` when it no longer waits.
    pub fn decide(&mut self, squad: &str, id: u64, approve: bool) -> Option<Command> {
        self.get(id)?;
        self.sent.push(id);
        let squad = squad.to_owned();
        Some(if approve { Command::ApproveRoutine { squad, id } } else { Command::DeclineRoutine { squad, id } })
    }
}

/// A routine's run as its conversation has it (`[Routine · title]`, a blank
/// line, what it asks): the heading it is shown under, and what it asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ran<'a> {
    pub title: &'a str,
    pub asked: &'a str,
}

/// The routine run `text` is, if it is one (`routines::message` wrote it).
pub fn ran(text: &str) -> Option<Ran<'_>> {
    let rest = text.strip_prefix("[Routine · ")?;
    let (title, asked) = rest.split_once(']')?;
    let asked = asked.strip_prefix("\n\n").or_else(|| asked.is_empty().then_some(""))?;
    (!title.trim().is_empty() && !title.contains('\n')).then(|| Ran { title: title.trim(), asked: asked.trim() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::routines::{Routine, Schedule};

    #[test]
    fn a_routine_run_is_read_back_as_its_heading_and_what_it_asked() {
        let routine = Routine { id: 2, title: "디자인 리뷰".into(), prompt: "주간 디자인 리뷰예요.\n1. 어긋난 것".into(), when: Schedule::Weekly { weekday: 1, hour: 10, minute: 0 }, on: true, last_run: None, next_run: None };
        let sent = equipe_core::routines::message(&routine);
        assert_eq!(ran(&sent), Some(Ran { title: "디자인 리뷰", asked: "주간 디자인 리뷰예요.\n1. 어긋난 것" }));
        let nudge = equipe_core::design::review::nudge(equipe_core::i18n::Lang::Ko, &routine, None, None, equipe_core::protocol::design::Setup::Logo);
        assert_eq!(ran(&nudge).map(|r| r.title), Some("디자인 리뷰"), "the setup's nudge, under the same heading");
        // The person's own words that only look like it are theirs.
        assert_eq!(ran("[Routine · ] hi"), None);
        assert_eq!(ran("[Routine · 주간 보고] 이건 제 말이에요"), None);
        assert_eq!(ran("[Idle · 24h]\n\nThe board is empty"), None);
        assert_eq!(ran("루틴 돌려 줘"), None);
    }

    fn weekly() -> RoutineProposal {
        RoutineProposal {
            id: 3,
            routine: Routine { id: 0, title: "주간 보고".into(), prompt: "이번 주 한 일을 정리해 주세요".into(), when: Schedule::Weekly { weekday: 1, hour: 9, minute: 0 }, on: true, last_run: None, next_run: None },
            reason: "매주 같은 걸 물어보셔서요".into(),
            by: "pm".into(),
            session: 1,
            proposed_at: 0,
        }
    }

    #[test]
    fn a_proposal_decided_here_hides_at_once_and_is_sent_once() {
        let mut p = Proposals::default();
        p.load(vec![weekly(), RoutineProposal { id: 4, ..weekly() }]);
        assert!(!p.push(weekly()), "not twice");
        assert_eq!(p.decide("acme", 3, true), Some(Command::ApproveRoutine { squad: "acme".into(), id: 3 }));
        assert_eq!(p.decide("acme", 3, true), None, "hidden: no second answer");
        assert_eq!(p.ids(), vec![4]);
        assert_eq!(p.decide("acme", 4, false), Some(Command::DeclineRoutine { squad: "acme".into(), id: 4 }));
        assert!(p.first().is_none());
        // A lost link shows what never came back; the daemon's word takes it for good.
        p.forget_sent();
        assert_eq!(p.ids(), vec![3, 4]);
        p.close(3);
        assert_eq!(p.ids(), vec![4]);
        // The snapshot replaces everything.
        p.load(vec![]);
        assert!(p.first().is_none());
    }

    #[test]
    fn the_card_says_when_it_runs() {
        assert_eq!(crate::views::routines::schedule_words(weekly().routine.when), "매주 월 09:00");
        assert_eq!(crate::views::routines::schedule_words(Schedule::Daily { hour: 18, minute: 30 }), "매일 18:30");
    }
}
