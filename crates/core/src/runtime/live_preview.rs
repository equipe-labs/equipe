//! Live previews the person's screens open on members' pages
//! (`crate::live_preview`): while the person looks, the member serving the
//! page is told — with each of its turns — to keep its server running, and
//! told again once they closed it. Nothing here starts a turn.

use super::*;

impl Runtime {
    /// A screen opened or closed a preview of `url` that `session`'s member
    /// serves. Only a loopback page of this machine, of a conversation of
    /// `squad`'s, counts.
    pub(super) fn live_preview(&mut self, squad: String, session: Option<SessionId>, url: String, open: bool) {
        let Ok(target) = crate::live_preview::loopback(&url) else { return };
        let Some(id) = session.filter(|id| self.sessions.get(id).is_some_and(|s| s.squad.as_deref() == Some(squad.as_str()))) else { return };
        let page = target.page();
        let line = if open {
            if !self.previews.open(id, &page, now_secs()) {
                return;
            }
            crate::daemon_log!("equipe: {squad}: the person opened a live preview of {page}");
            crate::live_preview::opened_line(&page)
        } else {
            if !self.previews.close(id, &page) {
                return;
            }
            crate::daemon_log!("equipe: {squad}: the person closed the live preview of {page}");
            crate::live_preview::closed_line(&page)
        };
        if let Some(session) = self.sessions.get_mut(&id) {
            session.briefing.push(line);
        }
        self.save_waiting(id);
    }

    /// What `id`'s next turn opens with while the person previews its pages.
    pub(super) fn preview_note(&self, id: SessionId) -> Option<String> {
        crate::live_preview::open_note(&self.previews.open_now(id, now_secs()))
    }
}
