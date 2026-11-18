//! What went live (`crate::deploys`): each minute every squad's projects are
//! looked at off the loop — where main stands, and for one with commits not
//! yet live, whether GitHub says they are. What went live is kept with what
//! it brought, in plain words, and shown on every screen
//! (`Event::DeploysChanged`); deploys close together are folded into one
//! note, told once it settles: a line in the PM's conversation, a line in
//! each conversation its work was asked in, and a notification
//! (`Event::Deployed`). A deploy that brought only internal work is listed,
//! never told.

use std::collections::BTreeMap;

use super::*;
use crate::deploys::{self as rules, Looked, Watch};
use equipe_protocol::deploys::Deploy;

impl Runtime {
    /// What went live lately in `squad`'s projects, the newest first.
    pub(super) fn deploy_views(&self, squad: &str) -> Vec<Deploy> {
        self.journals.get(squad).map(Journal::deploys).unwrap_or_default()
    }

    fn publish_deploys(&mut self, squad: &str) {
        let deploys = self.deploy_views(squad);
        self.publish(Event::DeploysChanged { squad: squad.to_owned(), deploys });
    }

    /// Each minute: news that settled is told, and every squad's projects are
    /// looked at again — one look at a time per squad, off the loop.
    pub(super) fn deploys_tick(&mut self) {
        let now = now_secs();
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            self.tell_deploys(&squad, now);
            if self.deploys_looking.contains(&squad) {
                continue;
            }
            let projects = self.projects(&squad);
            if projects.is_empty() {
                continue;
            }
            let watch: BTreeMap<String, Watch> = self.journals.get(&squad).map(Journal::deploy_watch).unwrap_or_default();
            // Projects that go live by a command of their own: what they deployed is told when it ran (`go_live`).
            let confirmed: HashSet<String> = self.lanes(&squad).iter().filter(|(_, l)| l.confirmed.is_some()).map(|(k, _)| k.clone()).collect();
            self.deploys_looking.insert(squad.clone());
            let internal = self.internal.clone();
            tokio::task::spawn_blocking(move || {
                let mut looks = Vec::new();
                let mut declared = Vec::new();
                for path in projects {
                    let Ok(repo) = worktree::repo_root(&path) else { continue };
                    let before = watch.get(&key_of(&path)).cloned().unwrap_or_default();
                    // A deploy file that can't be read still says the project goes live by one.
                    let file = !matches!(crate::go_live::declared_on_main(&repo), Ok(None));
                    declared.push((key_of(&path), file));
                    let by_command = confirmed.contains(&key_of(&path)) || file;
                    // Pushed isn't live for them: never noted as "on main" news, once watching started.
                    if by_command && before.live.is_some() {
                        continue;
                    }
                    let (after, looked) = rules::look(&repo, before.clone(), &rules::Gh, now_secs());
                    if after != before || looked != Looked::Nothing {
                        looks.push((path, after, looked));
                    }
                }
                let _ = internal.send(Internal::DeploysLooked { squad, looks, declared });
            });
        }
    }

    /// A look is back: each project's watch kept, and what went live noted —
    /// folded into a deploy of the same project close before, if one waits
    /// to be told.
    pub(super) fn deploys_looked(&mut self, squad: &str, looks: Vec<(PathBuf, Watch, Looked)>, declared: Vec<(String, bool)>) {
        self.deploys_looking.remove(squad);
        // Whether each project declares a deploy file, as just read: its work is live only by a deploy then.
        let mut changed = false;
        for (key, file) in declared {
            if self.lanes(squad).get(&key).is_some_and(|l| l.declared == file) || (!file && !self.lanes(squad).contains_key(&key)) {
                continue;
            }
            self.lanes(squad).entry(key).or_default().declared = file;
            changed = true;
        }
        if changed {
            self.keep_lanes(squad);
        }
        self.note_looks(squad, looks);
    }

    /// What went live, noted: each project's watch kept, and its deploy
    /// folded into one of the same project close before. A look's, or a
    /// project's own go-live command's (`go_live`).
    pub(super) fn note_looks(&mut self, squad: &str, looks: Vec<(PathBuf, Watch, Looked)>) {
        if looks.is_empty() {
            return;
        }
        let Some(journal) = self.journals.get(squad) else { return };
        let mut watch = journal.deploy_watch();
        let mut deploys = journal.deploys();
        let mut found = false;
        let mut carried: Vec<TicketId> = Vec::new();
        for (path, after, looked) in looks {
            watch.insert(key_of(&path), after);
            let Looked::Live { from, live, merges } = looked else { continue };
            if merges.is_empty() {
                continue;
            }
            let ids: Vec<TicketId> = merges.iter().flat_map(|m| m.tickets.iter().copied()).collect();
            let tickets: Vec<Ticket> = ids.iter().filter_map(|id| journal.ticket(*id).ok().flatten()).collect();
            let requests = journal.requests_of(&ids);
            let (items, internal, all) = rules::note(&merges, &tickets, &requests);
            let project = board::project_name(&path);
            crate::daemon_log!(
                "equipe: {squad}: {project} {} at {} ({} to see, {internal} internal)",
                if live.source.is_live() { "is live" } else { "has new commits on main" },
                &live.sha[..live.sha.len().min(9)],
                items.len()
            );
            let deploy = Deploy {
                id: 0,
                project,
                path,
                source: live.source,
                environment: live.environment,
                url: live.url,
                from: Some(from),
                to: live.sha,
                at: live.at,
                last_at: live.at,
                items,
                internal,
                tickets: all,
                told: false,
            };
            if deploy.source.is_live() {
                carried.extend(deploy.tickets.iter().copied());
            }
            rules::fold(&mut deploys, deploy);
            found = true;
        }
        if let Err(err) = journal.set_deploy_watch(&watch) {
            crate::daemon_log!("equipe: {squad}: where the projects' deploys stand wasn't kept: {err}");
        }
        if !found {
            return;
        }
        if let Err(err) = journal.set_deploys(&deploys) {
            crate::daemon_log!("equipe: {squad}: what went live wasn't kept: {err}");
            return;
        }
        self.publish_deploys(squad);
        self.tell_deploys(squad, now_secs());
        // The requests its work serves are live now.
        self.requests_of_moved(squad, &carried);
    }

    /// News of deploys that settled, told once: a line in the PM's
    /// conversation about the project, a line in each conversation its work
    /// was asked in (with only that work), and a notification. Marked told
    /// before it is told, so a failure to keep that never tells twice.
    fn tell_deploys(&mut self, squad: &str, now: u64) {
        let Some(journal) = self.journals.get(squad) else { return };
        let mut deploys = journal.deploys();
        let due: Vec<usize> = (0..deploys.len()).filter(|&i| rules::due(&deploys[i], now)).collect();
        if due.is_empty() {
            return;
        }
        for &i in &due {
            deploys[i].told = true;
        }
        if let Err(err) = journal.set_deploys(&deploys) {
            crate::daemon_log!("equipe: {squad}: a deploy's news wasn't kept as told, so it isn't told: {err}");
            return;
        }
        let words = crate::i18n::core(self.config.lang);
        let pm = self.pm_of(squad);
        // The oldest first, as they happened.
        for &i in due.iter().rev() {
            let d = deploys[i].clone();
            // Only work nobody sees: listed, not told.
            if d.items.is_empty() {
                continue;
            }
            let line = rules::line(&d, words, None);
            if let Some(pm) = pm {
                self.show_news_about(pm, line.clone(), Some(d.project.clone()));
            }
            for session in rules::sessions(&d) {
                if Some(session) == pm || !self.sessions.contains_key(&session) {
                    continue;
                }
                let text = rules::line(&d, words, Some(session));
                self.publish(Event::Notice { session, text, undo: None });
            }
            self.publish(Event::Deployed { squad: squad.to_owned(), deploy: d, line });
        }
        self.publish_deploys(squad);
    }
}

/// A project's key among the watches: its path.
pub(super) fn key_of(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
