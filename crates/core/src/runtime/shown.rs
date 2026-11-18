//! What a member shows the person (`equipe design session show`, read from
//! its commands' output — `crate::design::session::markers`), made to reach
//! their screen wherever it is: a page saved where the member works is
//! served by the daemon's page server (`crate::prototype`), so the screen
//! opens it at a loopback address its live preview forwards from another
//! Mac — never a `file://` address only this Mac opens — and the member is
//! told at once when no screen of the person's follows the squad, so it
//! never says "it's beside you" when nothing is (docs/REMOTE.md).
//!
//! The command itself can't ask: inside an engine's sandbox a member's
//! commands don't reach the daemon. What it printed is read here, as the
//! person's screen reads it.

use super::*;
use crate::design::session::{self, Marker};

impl Runtime {
    /// A member's command ended with `outcome`: each page it showed served,
    /// and what the member must hear now — nobody to show it to, or a page
    /// that can't be served — as a note for it.
    pub(super) fn member_showed(&mut self, id: SessionId, outcome: &equipe_protocol::ToolOutcome) -> Option<String> {
        if !outcome.ok || !outcome.output.contains(session::MARK) {
            return None;
        }
        let urls: Vec<String> = session::markers(&outcome.output)
            .into_iter()
            .filter_map(|m| match m {
                Marker::Show(url) | Marker::Variant { url, .. } => Some(url),
                _ => None,
            })
            .collect();
        if urls.is_empty() {
            return None;
        }
        let session = self.sessions.get(&id).filter(|s| s.member.is_some())?;
        let squad = session.squad.clone()?;
        let cwd = session.cwd.clone();
        let mut said = Vec::new();
        for url in &urls {
            let Some(page) = session::file_path(url) else { continue };
            // Where it works, the squad's folder, and the squad's projects (a designer at its desk works in their `design/`).
            let (work, mut roots) = self.picture_places(&squad, id);
            roots.extend([work, cwd.clone()]);
            roots.extend(self.projects_cached(&squad));
            match serve_shown(&page, &roots) {
                Ok(served) => crate::daemon_log!("equipe: {squad}: a session page served for the person's screen at {served}"),
                Err(why) => said.push(why),
            }
        }
        let following = self.screens.following(&squad);
        if !following.can_show() {
            said.insert(0, session::nobody_line(&urls.join(", ")));
        }
        (!said.is_empty()).then(|| said.join(" "))
    }

    /// What a member and the person hear when something comes up on this
    /// Mac's own screen — a Chrome window, a system dialog — while the person
    /// follows `squad` from another machine: where it is, and how to reach
    /// it, so nobody waits on it silently. `None` when they're at this Mac.
    pub(super) fn on_this_screen(&self, squad: &str) -> Option<(String, String)> {
        if !self.screens.following(squad).remote() {
            return None;
        }
        let mac = crate::screens::machine_name();
        let member = format!(
            " The person follows from another machine: that is on {mac}'s own screen, not theirs. Tell them plainly it needs \
{mac}'s screen — through Screen Sharing — and don't wait on it silently."
        );
        Some((member, say!(self, "notice.on_squad_mac", host = mac)))
    }

    /// What member conversation `id`'s turn opens with when where the person
    /// looks from changed since it was last told ([`crate::screens::note`]).
    pub(super) fn screens_note(&mut self, id: SessionId) -> Option<String> {
        let squad = self.sessions.get(&id).filter(|s| s.member.is_some())?.squad.clone()?;
        let remote = self.screens.following(&squad).remote();
        let told = self.remote_told.insert(id, remote);
        crate::screens::note(remote, told, crate::screens::machine_name())
    }
}

/// Serve page `page` a member showed, when it is under one of `roots`
/// (where it works, the squad's folder and projects): from its design folder when it is
/// in one, else its own. Its address, or what the member must fix.
pub(super) fn serve_shown(page: &Path, roots: &[PathBuf]) -> Result<String, String> {
    let real = std::fs::canonicalize(page).map_err(|_| format!("{} isn't there any more, so the person's screen can't open it — save it and show it again.", page.display()))?;
    let roots: Vec<PathBuf> = roots.iter().filter_map(|r| std::fs::canonicalize(r).ok()).collect();
    let Some(root) = roots.iter().filter(|r| real.starts_with(r)).max_by_key(|r| r.components().count()) else {
        return Err(format!("{} is outside your worktree, the squad's projects and its folder, so the person's screen can't open it — save it there and show it again.", real.display()));
    };
    let scope = session::page_scope(&real, root);
    let server = crate::prototype::Server::now().ok_or("equipe's page server couldn't start, so the person's screen can't open the page; tell them where it is in words.")?;
    server.put_session(&real, &scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_shown_page_is_served_only_from_where_the_member_works() {
        let root = std::env::temp_dir().join(format!("equipe-shown-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let work = root.join("work");
        std::fs::create_dir_all(work.join("design/sessions/pricing")).unwrap();
        std::fs::write(work.join("design/sessions/pricing/a.html"), "A").unwrap();
        std::fs::write(root.join("outside.html"), "x").unwrap();
        let served = serve_shown(&work.join("design/sessions/pricing/a.html"), std::slice::from_ref(&work)).unwrap();
        assert!(served.starts_with("http://127.0.0.1:") && served.ends_with("/sessions/pricing/a.html"), "{served}");
        assert!(serve_shown(&root.join("outside.html"), std::slice::from_ref(&work)).unwrap_err().contains("outside your worktree"));
        assert!(serve_shown(&work.join("gone.html"), std::slice::from_ref(&work)).unwrap_err().contains("isn't there"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
