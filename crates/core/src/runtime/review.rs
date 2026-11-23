//! The designer's weekly review (`crate::design::review`) handed over: to
//! the designer, not the PM — the review itself once every design folder's
//! setup is done; while a setup is unfinished nothing (the setup is the PM's
//! or the person's to propose, never started by a routine) — and the
//! decision the designer puts next marked as the review's proposal, so
//! screens show it as the review.

use equipe_protocol::routines::Routine;

use super::*;
use crate::design::review::{self, Gate};

impl Runtime {
    /// `squad`'s designer, in its own conversation.
    pub(super) fn designer_of(&self, squad: &str) -> Option<SessionId> {
        self.sessions
            .iter()
            .find(|(_, s)| s.squad.as_deref() == Some(squad) && s.is_member_itself() && s.member.as_ref().is_some_and(|m| m.role == review::ROLE))
            .map(|(&id, _)| id)
    }

    /// The review handed to the designer as equipe's message — after its turn
    /// if one runs. No designer, or a design setup not finished: nothing. A
    /// routine never sets the designer on the setup: that is several tickets
    /// of costly work, often not needed (a small, finished site), so only the
    /// PM or the person starts it.
    pub(super) async fn run_design_review(&mut self, squad: &str, routine: &Routine, now: u64) {
        let Some(designer) = self.designer_of(squad) else {
            crate::daemon_log!("equipe: {squad}: no designer for the design review");
            return;
        };
        let projects = self.projects(squad);
        let places = review::places(&squad_home::path(&self.config.home, squad), &projects);
        let text = match review::gate(&places, &projects) {
            Gate::Review => {
                crate::daemon_log!("equipe: {squad}: design review {} handed to the designer", routine.id);
                self.design_reviews.insert(designer, now);
                review::message(routine, &places)
            }
            Gate::Setup { step, .. } => {
                crate::daemon_log!("equipe: {squad}: design review skipped: the design setup isn't done ({step:?}), and a routine doesn't start it");
                return;
            }
        };
        self.hand_as_platform(designer, text).await;
    }

    /// Whether the decision `id` puts now is its review's proposal — once.
    pub(super) fn review_asked(&mut self, id: SessionId, now: u64) -> bool {
        review::is_its_proposal(self.design_reviews.remove(&id), now)
    }
}
