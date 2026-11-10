//! The squad's briefs and plans on a screen (`crate::briefs`): asked for with
//! `Command::ProjectDocs`, read off the loop and sent as `Event::ProjectDocs`
//! — and sent again whenever one is written (`write_brief`, and the plan
//! step's `write_plan` through [`Runtime::docs_changed`]). The PM writes a
//! brief with `write_brief`: into the squad's own folder, never a project's
//! repository, saved there as the PM's change off the loop, its answer sent
//! when it is in. A screen searches them (`Command::SearchDocs`) and, for one
//! saved under more than one project, keeps one copy (`Command::KeepDocCopy`):
//! the others go from the squad's folder as the person's change.

use super::*;
use crate::briefs::{self, Kind, Target, brief_args, brief_text};

impl Runtime {
    /// A screen asked: read them off the loop, and send them.
    pub(super) fn docs_asked(&mut self, squad: String) {
        if crate::squads::valid_name(&squad) {
            self.docs_changed(&squad);
        }
    }

    /// `squad`'s briefs and plans read again off the loop, and sent — after
    /// one is written, or when a screen asks.
    pub(crate) fn docs_changed(&mut self, squad: &str) {
        let (home, internal, squad) = (self.config.home.clone(), self.internal.clone(), squad.to_owned());
        tokio::task::spawn_blocking(move || {
            let catalog = briefs::catalog(&home, &squad);
            let _ = internal.send(Internal::Docs(squad, catalog));
        });
    }

    pub(super) fn on_docs(&mut self, squad: String, catalog: briefs::Catalog) {
        let briefs::Catalog { docs, copies, in_repos } = catalog;
        self.publish(Event::ProjectDocs { squad, docs, copies, in_repos });
    }

    /// A screen searches the briefs and plans: read off the loop, in the
    /// squad's folder only, and answered.
    pub(super) fn docs_searched(&mut self, squad: String, query: String) {
        if !crate::squads::valid_name(&squad) {
            return;
        }
        let (home, internal) = (self.config.home.clone(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let found = briefs::search(&squad_home::path(&home, &squad), &briefs::docs(&home, &squad), &query);
            let _ = internal.send(Internal::DocsFound(squad, query, found));
        });
    }

    /// The person keeps one copy of a brief or plan saved under more than one
    /// project: the others go from the squad's folder off the loop, as their
    /// change — told in `session`'s conversation, which must be the squad's.
    pub(super) fn keep_doc_copy(&mut self, squad: String, session: SessionId, keep: String) {
        if self.sessions.get(&session).is_none_or(|s| s.squad.as_deref() != Some(squad.as_str())) {
            return;
        }
        let (home, internal) = (self.config.home.clone(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let folder = squad_home::path(&home, &squad);
            let kept = briefs::keep_copy(&folder, &briefs::docs(&home, &squad), &keep);
            let _ = internal.send(Internal::DocCopyKept(squad.clone(), session, kept));
            let _ = internal.send(Internal::Docs(squad.clone(), briefs::catalog(&home, &squad)));
        });
    }

    pub(super) fn on_doc_copy_kept(&mut self, squad: String, session: SessionId, kept: Result<briefs::Kept, String>) {
        match kept {
            Ok(kept) => {
                let undo = kept.change.as_ref().map(|c| c.id.clone());
                if let Some(change) = &kept.change {
                    let text = say!(self, "notice.doc_copy_kept", title = kept.title, what = change.said(self.config.lang));
                    self.publish(Event::Notice { session, text, undo: undo.clone() });
                }
                self.publish(Event::DocCopyKept { squad, kept: kept.kept, removed: kept.removed, undo });
            }
            Err(err) => self.error(Some(session), say!(self, "error.doc_copy_not_kept", err = err), None),
        }
    }

    /// The PM writes a brief: checked here, written off the loop, answered
    /// once it is in.
    pub(super) fn write_brief(&mut self, call: &ToolCall, reply: oneshot::Sender<ToolResult>) {
        let fail = |reply: oneshot::Sender<ToolResult>, text: String| drop(reply.send(ToolResult { text, is_error: true, image: None }));
        let squad = call.squad.clone();
        let is_pm = self.member_id(&squad, &call.member).and_then(|id| self.sessions.get(&id)).is_some_and(Session::is_pm);
        if !is_pm {
            return fail(reply, "Only the PM writes a brief; tell the PM what you'd add to it.".into());
        }
        let brief = match brief_args(&call.arguments) {
            Ok(brief) => brief,
            Err(text) => return fail(reply, text),
        };
        let projects = self.projects(&squad);
        let folder = squad_home::path(&self.config.home, &squad);
        let target = match &brief.project {
            Some(arg) => match board::resolve_project(arg, &projects).map(|p| Target::project(&folder, &board::project_name(&p))) {
                Some(Ok(target)) => target,
                Some(Err(text)) => return fail(reply, text),
                None => {
                    let names: Vec<String> = projects.iter().map(|p| board::project_name(p)).collect();
                    return fail(reply, format!("No project called {arg}. The squad's projects: {}.", names.join(", ")));
                }
            },
            None => Target::squad(&folder),
        };
        let by = call.member.clone();
        let (home, internal) = (self.config.home.clone(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let today = squad_memory::date(now_secs());
            // A rewrite keeps the date it was first written on.
            let date = briefs::find(&target, Kind::Brief, &brief.slug).and_then(|n| briefs::parse_name(&n).map(|(d, _)| d.to_owned())).unwrap_or_else(|| today.clone());
            let result = briefs::save(&target, Kind::Brief, &brief.slug, &brief_text(&brief, &date), &today, &by);
            let _ = reply.send(match result {
                Ok(path) => ToolResult {
                    text: format!(
                        "Wrote {} ({}), in the squad's own folder — not in any repository. It shows on the project's page. \
Give the person its path in backticks in one line; {}",
                        path.display(),
                        brief.status,
                        if brief.status == "settled" { "the plan comes next." } else { "it is settled only when they say so." }
                    ),
                    is_error: false,
                    image: None,
                },
                Err(text) => ToolResult { text: format!("The brief wasn't written: {text}"), is_error: true, image: None },
            });
            let catalog = briefs::catalog(&home, &squad);
            let _ = internal.send(Internal::Docs(squad, catalog));
        });
    }
}
