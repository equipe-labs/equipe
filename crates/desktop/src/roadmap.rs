//! 로드맵: the squad's work as one picture (`views/roadmap.rs`) — a lane per
//! project (the HQ first), and across it what finished lately, what is under
//! way now, what comes next and what waits on something before it; an arrow
//! from each ticket to the ones waiting on it. Drawn from the board alone,
//! so it is always as the squad stands.

use std::collections::HashMap;

use equipe_core::protocol::{Ticket, TicketId, TicketStatus};

use crate::state::Model;

/// Where a ticket stands, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    /// Finished in the last two weeks.
    Done,
    /// Under way, handed in, or waiting on the person.
    Now,
    /// Ready to start: nothing before it unfinished; and proposals.
    Next,
    /// Waits on a ticket not finished yet.
    Later,
}

impl Stage {
    pub const ALL: [Stage; 4] = [Stage::Done, Stage::Now, Stage::Next, Stage::Later];

    pub fn column(self) -> usize {
        self as usize
    }
}

/// What finished lately enough to show.
pub const DONE_WINDOW: u64 = 14 * 24 * 60 * 60;
/// Finished tickets a lane shows, the latest.
pub const DONE_SHOWN: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    /// `None`: work outside the projects.
    pub name: Option<String>,
    /// Its place among the projects: its colour.
    pub at: Option<usize>,
    pub hq: bool,
    /// The tallest of its stages, in cards.
    pub rows: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Card {
    pub id: TicketId,
    pub stage: Stage,
    pub lane: usize,
    /// Its place down its lane's stage.
    pub row: usize,
}

/// A ticket waiting on another: `from` must finish before `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: TicketId,
    pub to: TicketId,
    /// `from` isn't finished: `to` is held up by it.
    pub holding: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub lanes: Vec<Lane>,
    pub cards: Vec<Card>,
    pub edges: Vec<Edge>,
}

impl Layout {
    pub fn count(&self, stage: Stage) -> usize {
        self.cards.iter().filter(|c| c.stage == stage).count()
    }

    pub fn card(&self, id: TicketId) -> Option<&Card> {
        self.cards.iter().find(|c| c.id == id)
    }
}

fn stage_of(model: &Model, t: &Ticket, now: u64) -> Option<Stage> {
    match t.status {
        TicketStatus::Cancelled => None,
        TicketStatus::Done => t.updated_at.is_some_and(|at| now.saturating_sub(at) < DONE_WINDOW).then_some(Stage::Done),
        TicketStatus::InProgress | TicketStatus::Review | TicketStatus::Waiting => Some(Stage::Now),
        TicketStatus::Proposed => Some(Stage::Next),
        TicketStatus::Todo => Some(if model.blocked_by(t).is_some() { Stage::Later } else { Stage::Next }),
    }
}

/// The picture of the board at `now` (unix seconds).
pub fn layout(model: &Model, now: u64) -> Layout {
    let names = model.project_names();
    let mut lanes: Vec<Lane> = names.iter().enumerate().map(|(i, n)| Lane { name: Some(n.clone()), at: Some(i), hq: model.is_hq(n), rows: 0 }).collect();
    lanes.push(Lane { name: None, at: None, hq: false, rows: 0 });
    let lane_of = |t: &Ticket| -> usize {
        match model.ticket_project_name(t) {
            Some(name) => names.iter().position(|n| *n == name).unwrap_or(names.len()),
            None => names.len(),
        }
    };
    // Each lane's stage, in order: the work's own order (rank, then number);
    // what finished, the latest first and only the last few.
    let mut groups: HashMap<(usize, Stage), Vec<&Ticket>> = HashMap::new();
    for t in &model.tickets {
        if let Some(stage) = stage_of(model, t, now) {
            groups.entry((lane_of(t), stage)).or_default().push(t);
        }
    }
    let mut cards = Vec::new();
    for ((lane, stage), mut list) in groups {
        if stage == Stage::Done {
            list.sort_by_key(|t| std::cmp::Reverse(t.updated_at.unwrap_or(0)));
            list.truncate(DONE_SHOWN);
        } else {
            list.sort_by_key(|t| (t.order_key(), t.id));
        }
        for (row, t) in list.into_iter().enumerate() {
            cards.push(Card { id: t.id, stage, lane, row });
        }
    }
    cards.sort_by_key(|c| (c.lane, c.stage, c.row));
    for (i, lane) in lanes.iter_mut().enumerate() {
        lane.rows = cards.iter().filter(|c| c.lane == i).map(|c| c.row + 1).max().unwrap_or(0);
    }
    // Lanes with nothing to show stay out — but every project keeps a lane when all are empty.
    let shown: Vec<usize> = (0..lanes.len()).filter(|&i| lanes[i].rows > 0).collect();
    if !shown.is_empty() {
        let renumber: HashMap<usize, usize> = shown.iter().enumerate().map(|(new, old)| (*old, new)).collect();
        lanes = shown.iter().map(|&i| lanes[i].clone()).collect();
        for c in &mut cards {
            c.lane = renumber[&c.lane];
        }
    } else {
        lanes.pop();
    }
    let on = |id: TicketId| cards.iter().any(|c| c.id == id);
    let edges = model
        .tickets
        .iter()
        .filter(|t| on(t.id))
        .flat_map(|t| t.depends_on.iter().filter(|d| on(**d)).map(move |d| (*d, t.id)))
        .map(|(from, to)| Edge { from, to, holding: model.tickets.iter().find(|x| x.id == from).is_some_and(|x| x.status != TicketStatus::Done) })
        .collect();
    Layout { lanes, cards, edges }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(id: TicketId, project: &str, status: &str, deps: &[TicketId], updated: u64) -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": format!("t{id}"), "body": "", "status": status, "owner": "backend", "assignee": null,
            "project": format!("/c/{project}"), "depends_on": deps, "branch": null, "created_by": "pm", "note": null, "updated_at": updated
        }))
        .unwrap()
    }

    const NOW: u64 = 100 * 24 * 60 * 60;

    #[test]
    fn the_board_reads_left_to_right_a_lane_a_project_with_what_waits_on_what() {
        let mut model = Model::new("acme");
        model.projects = vec!["/c/web".into(), "/c/hq".into(), "/c/api".into()];
        model.hq = Some("/c/hq".into());
        model.order_projects();
        model.tickets = vec![
            ticket(1, "web", "done", &[], NOW - 3600),
            ticket(2, "web", "in_progress", &[1], NOW),
            ticket(3, "web", "todo", &[2], NOW),
            ticket(4, "web", "todo", &[], NOW),
            ticket(5, "web", "done", &[], NOW - 30 * 24 * 3600),
            ticket(6, "hq", "review", &[], NOW),
            ticket(7, "web", "cancelled", &[], NOW),
            ticket(8, "web", "proposed", &[], NOW),
        ];
        let l = layout(&model, NOW);
        let lanes: Vec<_> = l.lanes.iter().map(|x| (x.name.clone(), x.hq)).collect();
        assert_eq!(lanes, [(Some("hq".into()), true), (Some("web".into()), false)], "the HQ first; api, with nothing, left out");
        let at = |id| l.card(id).map(|c| (c.stage, c.lane, c.row));
        assert_eq!(at(1), Some((Stage::Done, 1, 0)));
        assert_eq!(at(5), None, "finished a month ago");
        assert_eq!(at(7), None, "cancelled");
        assert_eq!(at(2), Some((Stage::Now, 1, 0)));
        assert_eq!(at(3), Some((Stage::Later, 1, 0)), "waits on T-2");
        assert_eq!(at(4).map(|a| a.0), Some(Stage::Next));
        assert_eq!(at(8).map(|a| a.0), Some(Stage::Next), "a proposal comes next");
        assert_eq!(at(6), Some((Stage::Now, 0, 0)));
        assert_eq!(l.lanes[1].rows, 2, "web's tallest stage: next, with T-4 and T-8");
        assert_eq!(l.edges, [Edge { from: 1, to: 2, holding: false }, Edge { from: 2, to: 3, holding: true }]);
        assert_eq!((l.count(Stage::Done), l.count(Stage::Now), l.count(Stage::Next), l.count(Stage::Later)), (1, 2, 2, 1));
    }

    #[test]
    fn a_lane_shows_only_the_last_few_it_finished() {
        let mut model = Model::new("acme");
        model.projects = vec!["/c/web".into()];
        model.tickets = (1..=6).map(|i| ticket(i, "web", "done", &[], NOW - i as u64 * 60)).collect();
        let l = layout(&model, NOW);
        let ids: Vec<_> = l.cards.iter().map(|c| c.id).collect();
        assert_eq!(ids, [1, 2, 3, 4], "the latest first");
    }
}
