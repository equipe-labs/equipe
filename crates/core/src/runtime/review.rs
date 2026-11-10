//! The designer's weekly review (`crate::design::review`) handed over: to
//! the designer, not the PM — the review itself once every design folder's
//! setup is done, else a nudge to go on with the step it is at — and the
//! decision the designer puts next marked as the review's proposal, so
//! screens show it as the review.

use equipe_protocol::routines::Routine;

use super::*;
use crate::design::review::{self, Gate};

impl Runtime {
    /// `squad`'s designer, in its own conversation.
    fn designer_of(&self, squad: &str) -> Option<SessionId> {
        self.sessions
            .iter()
            .find(|(_, s)| s.squad.as_deref() == Some(squad) && s.lane.is_none() && s.member.as_ref().is_some_and(|m| m.role == review::ROLE))
            .map(|(&id, _)| id)
    }

    /// The review, or the setup's nudge, handed to the designer as the
    /// person's message — after its turn if one runs. No designer: nothing.
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
            Gate::Setup { project, dir, step } => {
                crate::daemon_log!("equipe: {squad}: design review waits for the setup ({step:?})");
                let lang = crate::i18n::Lang::resolve_in(&self.config.home, crate::i18n::Front::App);
                review::nudge(lang, routine, project.as_deref(), dir.as_deref(), step)
            }
        };
        match self.sessions.get(&designer) {
            Some(s) if s.is_working() => self.leave_memo(designer, text),
            _ => self.submit(designer, text).await,
        }
    }

    /// Whether the decision `id` puts now is its review's proposal — once.
    pub(super) fn review_asked(&mut self, id: SessionId, now: u64) -> bool {
        review::is_its_proposal(self.design_reviews.remove(&id), now)
    }
}
