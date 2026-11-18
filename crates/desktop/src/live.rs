//! What went live, as the window shows it — a project's latest deploy on its
//! sidebar line, every deploy on its page's "What's live" tab — and what an
//! update of the app brings: a few lines over "Restart to Update", and once
//! after the restart, what it brought. The daemon finds deploys and writes
//! their notes (`equipe_core::deploys`); the app's own notes are the release
//! feed (`equipe_core::release_notes`) beside its appcast.

use std::path::Path;

use equipe_core::protocol::deploys::Deploy;
use equipe_core::release_notes::{self, Feed};
use gpui_kit::Context;

use crate::Shell;

/// How long a project's latest deploy stays on its sidebar line.
pub const SIDEBAR_FOR: u64 = 24 * 3600;
/// The lines of an update's notes shown at once; the rest are counted.
pub const NOTES_SHOWN: usize = 3;

/// The deploys of the project at `path`, the newest first.
pub fn of_project<'a>(deploys: &'a [Deploy], path: &Path) -> Vec<&'a Deploy> {
    let mut found: Vec<&Deploy> = deploys.iter().filter(|d| d.path == path).collect();
    found.sort_by(|a, b| b.at.max(b.last_at).cmp(&a.at.max(a.last_at)).then(b.id.cmp(&a.id)));
    found
}

/// The project's latest deploy for its sidebar line: live for its users
/// (not only on main), with something the person can see, in the last day.
pub fn sidebar_line<'a>(deploys: &'a [Deploy], path: &Path, now: u64) -> Option<&'a Deploy> {
    of_project(deploys, path).into_iter().find(|d| d.source.is_live() && !d.items.is_empty() && now.saturating_sub(d.at.max(d.last_at)) < SIDEBAR_FOR)
}

/// A few lines of the app's notes, and how many more there are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notes {
    pub version: String,
    pub lines: Vec<String>,
    pub more: usize,
}

fn notes(feed: &Feed, after: Option<&str>, upto: &str) -> Option<Notes> {
    let releases = release_notes::between(feed, after, upto);
    let (lines, more) = release_notes::summary(&releases, NOTES_SHOWN);
    (!lines.is_empty()).then(|| Notes { version: upto.to_owned(), lines, more })
}

/// What the update waiting (`ready`) brings to this app (`ours`): every
/// release after ours up to it. `None` when the feed says nothing of it.
pub fn whats_new(feed: &Feed, ours: &str, ready: &str) -> Option<Notes> {
    if !crate::relaunch::newer(ready, ours) {
        return None;
    }
    notes(feed, Some(ours), ready)
}

/// What the last update brought, once, after the restart into it: from the
/// version this app last ran (`ran`) to this one. `None` the first time (no
/// version kept yet) and when nothing changed.
pub fn updated(feed: &Feed, ran: Option<&str>, ours: &str) -> Option<Notes> {
    let ran = ran?;
    if !crate::relaunch::newer(ours, ran) {
        return None;
    }
    notes(feed, Some(ran), ours)
}

/// This copy of the app's release feed, beside its appcast. `None` for a
/// build without one (`cargo run`).
fn feed_url() -> Option<String> {
    crate::relaunch::this_macs_feed().and_then(|appcast| release_notes::feed_url(&appcast))
}

impl Shell {
    /// The app's release notes, read once a window needs them — an update is
    /// ready, or the app was just updated — and again when a newer update is
    /// ready than the feed knows.
    pub fn ask_release_notes(&mut self, cx: &mut Context<Self>) {
        let wanted = self.update.ready().map(str::to_owned).unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
        if self.release_feed_for.as_deref() == Some(wanted.as_str()) {
            return;
        }
        let Some(url) = feed_url() else { return };
        self.release_feed_for = Some(wanted);
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(release_notes::fetch(&url));
        });
        cx.spawn(async move |this, cx| {
            let feed = match rx.await {
                Ok(Ok(feed)) => feed,
                Ok(Err(why)) => {
                    crate::relaunch::log(&format!("the release notes weren't read: {why}"));
                    return;
                }
                Err(_) => return,
            };
            let _ = this.update(cx, |this, cx| {
                this.release_feed = Some(feed);
                this.note_updated();
                cx.notify();
            });
        })
        .detach();
    }

    /// At launch: the version last run is kept, and if this one is newer the
    /// window says once what the update brought — as soon as the notes are read.
    pub fn note_launch(&mut self, cx: &mut Context<Self>) {
        let ours = env!("CARGO_PKG_VERSION");
        if self.remembered.ran.as_deref() == Some(ours) {
            return;
        }
        self.updated_from = self.remembered.ran.clone();
        self.remembered.ran = Some(ours.to_owned());
        self.remembered.save(&crate::equipe_home());
        if self.updated_from.is_some() {
            self.ask_release_notes(cx);
        }
    }

    /// The "Updated to …" card, from the version this app ran before, once the notes are here.
    fn note_updated(&mut self) {
        let Some(feed) = &self.release_feed else { return };
        if let Some(from) = self.updated_from.take() {
            self.updated_card = updated(feed, Some(&from), env!("CARGO_PKG_VERSION"));
        }
    }

    /// What the update waiting brings, for the lines over "Restart to Update".
    pub fn whats_new(&self) -> Option<Notes> {
        whats_new(self.release_feed.as_ref()?, env!("CARGO_PKG_VERSION"), self.update.ready()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::deploys::{DeploySource, NoteItem};
    use equipe_core::release_notes::Release;

    fn deploy(id: u64, path: &str, source: DeploySource, at: u64, items: &[&str]) -> Deploy {
        Deploy {
            id,
            project: "site".into(),
            path: path.into(),
            source,
            environment: None,
            url: Some("https://example.com".into()),
            from: None,
            to: "abc".into(),
            at,
            last_at: at,
            items: items.iter().map(|t| NoteItem { title: t.to_string(), tickets: vec![], request: None, request_title: None, session: None }).collect(),
            internal: 0,
            tickets: vec![],
            told: true,
        }
    }

    #[test]
    fn a_projects_sidebar_line_is_its_latest_deploy_people_see_within_a_day() {
        let now = 100_000;
        let deploys = vec![
            deploy(1, "/c/site", DeploySource::Deployment, now - 5000, &["Old"]),
            deploy(2, "/c/site", DeploySource::Deployment, now - 600, &[]),
            deploy(3, "/c/site", DeploySource::Main, now - 60, &["Only on main"]),
            deploy(4, "/c/app", DeploySource::Workflow, now - 30, &["App"]),
        ];
        assert_eq!(of_project(&deploys, Path::new("/c/site")).iter().map(|d| d.id).collect::<Vec<_>>(), [3, 2, 1], "the newest first");
        assert_eq!(sidebar_line(&deploys, Path::new("/c/site"), now).map(|d| d.id), Some(1), "not only internal work, not only on main");
        assert_eq!(sidebar_line(&deploys, Path::new("/c/site"), now - 5000 + SIDEBAR_FOR), None, "a day on, the line goes");
        assert_eq!(sidebar_line(&deploys, Path::new("/c/app"), now).map(|d| d.id), Some(4));
        assert_eq!(sidebar_line(&deploys, Path::new("/c/none"), now), None);
    }

    fn feed() -> Feed {
        let release = |v: &str, notes: &[&str]| Release { version: v.into(), date: String::new(), notes: notes.iter().map(|n| n.to_string()).collect() };
        Feed { releases: vec![release("0.5.0", &["A", "B", "C", "D"]), release("0.4.0", &["E"]), release("0.3.0", &[]), release("0.2.0", &["F"])] }
    }

    #[test]
    fn an_update_waiting_says_what_it_brings_and_after_the_restart_what_it_brought() {
        let feed = feed();
        assert_eq!(whats_new(&feed, "0.3.0", "0.5.0"), Some(Notes { version: "0.5.0".into(), lines: vec!["A".into(), "B".into(), "C".into()], more: 2 }));
        assert_eq!(whats_new(&feed, "0.5.0", "0.5.0"), None, "nothing to bring");
        assert_eq!(whats_new(&feed, "0.2.0", "0.3.0"), None, "a release with no notes says nothing");
        assert_eq!(updated(&feed, Some("0.3.0"), "0.4.0").map(|n| n.lines), Some(vec!["E".to_owned()]));
        assert_eq!(updated(&feed, None, "0.4.0"), None, "the first launch that keeps a version tells nothing");
        assert_eq!(updated(&feed, Some("0.4.0"), "0.4.0"), None);
        assert_eq!(updated(&feed, Some("0.5.0"), "0.4.0"), None, "an older build run again tells nothing");
    }
}
