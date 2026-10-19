//! 현황: every project of the squad at a glance — what is under way, what is
//! stuck, what waits for the person's review, what finished lately and the
//! latest release — the HQ first. The screen is `views/overview.rs`.

use std::path::{Path, PathBuf};

use equipe_core::protocol::{Ticket, TicketId, TicketStatus};

use crate::target::Endpoint;
use crate::state::Model;

/// A row's worth per section: the rest is a count.
pub const ROWS: usize = 3;
/// What "lately" is for finished work.
pub const WEEK: u64 = 7 * 24 * 60 * 60;

/// One project's card.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Card {
    /// `None`: work outside any project (the squad's folder).
    pub name: Option<String>,
    pub path: Option<PathBuf>,
    /// Its place among the projects: its colour.
    pub at: Option<usize>,
    pub hq: bool,
    /// Waiting on the person, or on a ticket not done yet.
    pub stuck: Vec<TicketId>,
    /// Handed in, for the person's review.
    pub review: Vec<TicketId>,
    pub working: Vec<TicketId>,
    pub todo: usize,
    /// Finished in the last week, newest first.
    pub shipped: Vec<TicketId>,
}

impl Card {
    pub fn is_quiet(&self) -> bool {
        self.stuck.is_empty() && self.review.is_empty() && self.working.is_empty() && self.todo == 0 && self.shipped.is_empty()
    }
}

/// The whole squad's numbers, over the cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Totals {
    pub working: usize,
    pub review: usize,
    pub stuck: usize,
    pub shipped: usize,
}

pub fn totals(cards: &[Card]) -> Totals {
    cards.iter().fold(Totals::default(), |t, c| Totals {
        working: t.working + c.working.len(),
        review: t.review + c.review.len(),
        stuck: t.stuck + c.stuck.len(),
        shipped: t.shipped + c.shipped.len(),
    })
}

/// A card per project in the sidebar's order (the HQ first), then one for
/// work outside them if there is any. `now`: unix seconds.
pub fn cards(model: &Model, now: u64) -> Vec<Card> {
    let names = model.project_names();
    let mut out = Vec::new();
    for (at, name) in names.iter().cloned().enumerate().map(|(i, n)| (Some(i), Some(n))).chain([(None, None)]) {
        let here: Vec<&Ticket> = model.tickets.iter().filter(|t| model.ticket_project_name(t) == name).collect();
        let stuck = |t: &Ticket| t.status == TicketStatus::Waiting || (!t.status.is_closed() && t.status != TicketStatus::Review && model.blocked_by(t).is_some());
        let card = Card {
            path: name.as_ref().and_then(|n| model.projects.iter().find(|p| equipe_core::board::project_name(p) == *n).cloned()),
            hq: name.as_deref().is_some_and(|n| model.is_hq(n)),
            name,
            at,
            stuck: newest(&here, |t| stuck(t)),
            review: newest(&here, |t| t.status == TicketStatus::Review),
            working: newest(&here, |t| t.status == TicketStatus::InProgress && !stuck(t)),
            todo: here.iter().filter(|t| t.status == TicketStatus::Todo && !stuck(t)).count(),
            shipped: newest(&here, |t| t.status == TicketStatus::Done && t.updated_at.is_some_and(|at| now.saturating_sub(at) < WEEK)),
        };
        if card.name.is_none() && card.is_quiet() {
            continue;
        }
        out.push(card);
    }
    out
}

/// Those `keep` takes, the latest change first, then the higher number.
fn newest(tickets: &[&Ticket], keep: impl Fn(&Ticket) -> bool) -> Vec<TicketId> {
    let mut v: Vec<&Ticket> = tickets.iter().copied().filter(|t| keep(t)).collect();
    v.sort_by_key(|t| (std::cmp::Reverse(t.updated_at.unwrap_or(0)), std::cmp::Reverse(t.id)));
    v.into_iter().map(|t| t.id).collect()
}

/// A project's latest release: its newest tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    /// Unix seconds.
    pub at: u64,
}

/// What finds each project's newest tag, on whichever machine runs it.
fn releases_script(projects: &[PathBuf]) -> String {
    let dirs: Vec<String> = projects.iter().map(|p| crate::ssh::quote(&p.to_string_lossy())).collect();
    format!(
        "for d in {}; do t=$(git -C \"$d\" for-each-ref --sort=-creatordate --count=1 --format='%(refname:short)\t%(creatordate:unix)' refs/tags 2>/dev/null); printf '%s\\t%s\\n' \"$d\" \"$t\"; done",
        dirs.join(" ")
    )
}

/// `<path>\t<tag>\t<unix>` a line; a project without tags has its path alone.
pub fn parse_releases(text: &str) -> Vec<(PathBuf, Release)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let (path, tag, at) = (parts.next()?, parts.next()?.trim(), parts.next()?.trim().parse().ok()?);
            (!tag.is_empty()).then(|| (PathBuf::from(path), Release { tag: tag.to_owned(), at }))
        })
        .collect()
}

/// Each project's newest tag, here or on the daemon's machine. Blocks: call
/// it on a thread.
pub fn releases(endpoint: &Endpoint, projects: &[PathBuf]) -> Result<Vec<(PathBuf, Release)>, String> {
    if projects.is_empty() {
        return Ok(Vec::new());
    }
    let script = releases_script(projects);
    let text = match endpoint {
        Endpoint::Local { .. } => {
            let out = std::process::Command::new("sh").arg("-c").arg(&script).stdin(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
        Endpoint::Ssh(host) => {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
            let bytes = runtime.block_on(crate::ssh::Launcher::system(None).run(host, &script)).map_err(|f| f.guide(endpoint).short)?;
            String::from_utf8_lossy(&bytes).into_owned()
        }
    };
    Ok(parse_releases(&text))
}

/// The release of the project at `path`, among those found.
pub fn release_of<'a>(found: &'a [(PathBuf, Release)], path: &Path) -> Option<&'a Release> {
    found.iter().find(|(p, _)| p == path).map(|(_, r)| r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(id: TicketId, project: &str, status: TicketStatus, updated: u64) -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": format!("t{id}"), "body": "", "status": status, "owner": "backend", "assignee": null,
            "project": format!("/c/{project}"), "depends_on": [], "branch": null, "created_by": "pm", "note": null, "updated_at": updated
        }))
        .unwrap()
    }

    #[test]
    fn each_project_says_what_is_stuck_under_way_in_review_and_shipped() {
        let mut model = Model::new("acme");
        model.projects = vec!["/c/web".into(), "/c/hq".into(), "/c/api".into()];
        model.hq = Some("/c/hq".into());
        model.order_projects();
        let now = 10 * WEEK;
        let mut blocked = ticket(5, "web", TicketStatus::Todo, now - 50);
        blocked.depends_on = vec![1];
        model.tickets = vec![
            ticket(1, "web", TicketStatus::InProgress, now - 100),
            ticket(2, "web", TicketStatus::Waiting, now - 10),
            ticket(3, "web", TicketStatus::Review, now - 20),
            ticket(4, "web", TicketStatus::Done, now - 60),
            blocked,
            ticket(6, "web", TicketStatus::Done, now - 2 * WEEK),
            ticket(7, "web", TicketStatus::Todo, now - 5),
            ticket(8, "hq", TicketStatus::InProgress, now - 30),
            Ticket { project: None, kind: equipe_core::protocol::TicketKind::Task, ..ticket(9, "-", TicketStatus::InProgress, now) },
        ];
        let cards = cards(&model, now);
        let names: Vec<_> = cards.iter().map(|c| c.name.clone()).collect();
        assert_eq!(names, [Some("hq".into()), Some("web".into()), Some("api".into()), None], "the HQ first; work outside the projects last");
        assert!(cards[0].hq && !cards[1].hq);
        let web = &cards[1];
        assert_eq!(web.stuck, [2, 5], "waiting on the person, then waiting on T-1");
        assert_eq!(web.review, [3]);
        assert_eq!(web.working, [1]);
        assert_eq!(web.todo, 1);
        assert_eq!(web.shipped, [4], "a week back at most");
        assert!(cards[2].is_quiet());
        assert_eq!(totals(&cards), Totals { working: 3, review: 1, stuck: 2, shipped: 1 });
    }

    #[test]
    fn a_projects_newest_tag_is_its_latest_release() {
        let text = "/c/web\tv1.2.0\t1700000000\n/c/api\t\n/c/odd\tv1\tsoon\n";
        assert_eq!(parse_releases(text), [(PathBuf::from("/c/web"), Release { tag: "v1.2.0".into(), at: 1_700_000_000 })]);
        let script = releases_script(&["/c/my app".into()]);
        assert!(script.contains("'/c/my app'"), "{script}");
    }
}
