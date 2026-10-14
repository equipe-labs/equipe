//! The board's rules, with no UI types: which tickets show in which order,
//! under which heading, as a tree of what waits for what, narrowed by a
//! search. The same rules as the terminal board (`tui/app.rs` `board_order`,
//! `lanes`, `dependency_tree`), so both screens show the same board.

use std::path::PathBuf;

use equipe_core::protocol::{Ticket, TicketId, TicketKind, TicketStatus};

use crate::state::Model;

/// Todo and Proposed fold past this many (per section or lane) until shown whole.
pub const SECTION_ROWS: usize = 10;
/// How many finished tickets show without a search.
pub const DONE_SHOWN: usize = 5;

/// What the person has done to the board's view.
#[derive(Debug, Clone, Default)]
pub struct View {
    /// What waits for what, as a tree, instead of by state.
    pub graph: bool,
    /// Long sections shown whole.
    pub expanded: bool,
    /// What is typed in the search box.
    pub query: String,
    pub selected: Option<TicketId>,
}

/// How a heading is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tone {
    /// The person's call: accent.
    ForYou,
    /// A project's lane: its colour (the project's position in the squad).
    Lane(Option<usize>),
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Heading { label: String, count: usize, tone: Tone },
    /// A ticket, and how deep under what it waits for (tree view only).
    Ticket { id: TicketId, depth: usize },
    /// What a folded section left out.
    More { label: String, more: usize },
}

/// The words the board is narrowed to, lowercased.
pub fn query(view: &View) -> Option<String> {
    let q = view.query.trim().to_lowercase();
    (!q.is_empty()).then_some(q)
}

fn in_focus(model: &Model, t: &Ticket) -> bool {
    model.focus.is_none() || model.ticket_project_name(t) == model.focus
}

/// Whether open work goes in a lane per project: with no filter, and work in
/// more than one of them (the squad's own folder counts as one).
pub fn lanes(model: &Model) -> bool {
    let home_work = model.tickets.iter().any(|t| !t.status.is_closed() && model.ticket_project(t).is_none());
    model.focus.is_none() && model.projects.len() + usize::from(home_work) > 1
}

/// Open work as a tree of what waits for what. A ticket waiting on nothing
/// still open is a root; one waiting on several shows under the first.
pub fn dependency_tree(model: &Model) -> Vec<(TicketId, usize)> {
    let open: Vec<&Ticket> = model.tickets.iter().filter(|t| !t.status.is_closed() && in_focus(model, t)).collect();
    let is_open = |id: TicketId| open.iter().any(|t| t.id == id);
    let mut out: Vec<(TicketId, usize)> = Vec::new();
    let mut stack: Vec<(TicketId, usize)> = open.iter().filter(|t| !t.depends_on.iter().any(|d| is_open(*d))).map(|t| (t.id, 0)).collect();
    stack.sort_by_key(|(id, _)| std::cmp::Reverse(*id));
    while let Some((id, depth)) = stack.pop() {
        if out.iter().any(|(seen, _)| *seen == id) {
            continue;
        }
        out.push((id, depth));
        let mut next: Vec<TicketId> = open.iter().filter(|t| t.depends_on.contains(&id)).map(|t| t.id).collect();
        next.sort_unstable_by(|a, b| b.cmp(a));
        stack.extend(next.into_iter().map(|c| (c, depth + 1)));
    }
    // Anything left in a loop of waiting: listed, not lost.
    for t in &open {
        if !out.iter().any(|(id, _)| *id == t.id) {
            out.push((t.id, 0));
        }
    }
    out
}

fn matches(model: &Model, t: &Ticket, q: &str) -> bool {
    in_focus(model, t) && (t.title.to_lowercase().contains(q) || t.body.to_lowercase().contains(q) || format!("t-{}", t.id) == q)
}

/// Heading and tone for a ticket in the by-state list.
fn heading(model: &Model, t: &Ticket, lanes: bool) -> (String, Tone) {
    let project_index = |t: &Ticket| model.ticket_project(t).and_then(|p| model.projects.iter().position(|q| *q == p));
    match t.status {
        TicketStatus::Review => ("내 검토".into(), Tone::ForYou),
        TicketStatus::Waiting => ("내 차례".into(), Tone::ForYou),
        TicketStatus::InProgress | TicketStatus::Todo | TicketStatus::Proposed if lanes => {
            (model.ticket_project_name(t).unwrap_or_else(|| "스쿼드 폴더".into()), Tone::Lane(project_index(t)))
        }
        TicketStatus::InProgress => ("진행 중".into(), Tone::Plain),
        TicketStatus::Todo => ("할 일".into(), Tone::Plain),
        TicketStatus::Proposed => ("제안".into(), Tone::Plain),
        TicketStatus::Done | TicketStatus::Cancelled => ("끝남".into(), Tone::Plain),
    }
}

/// The tickets listed, in order: the person's reviews, what waits on them,
/// open work (a lane per project when there are several), the latest few done.
pub fn order(model: &Model, view: &View) -> Vec<TicketId> {
    if let Some(q) = query(view) {
        // What matches: open work first, then finished, the latest first.
        let hit = |t: &&Ticket| matches(model, t, &q);
        let mut open: Vec<TicketId> = model.tickets.iter().filter(hit).filter(|t| !t.status.is_closed()).map(|t| t.id).collect();
        open.sort_unstable();
        let mut closed: Vec<TicketId> = model.tickets.iter().filter(hit).filter(|t| t.status.is_closed()).map(|t| t.id).collect();
        closed.sort_unstable_by(|a, b| b.cmp(a));
        open.extend(closed);
        return open;
    }
    if view.graph {
        return dependency_tree(model).into_iter().map(|(id, _)| id).collect();
    }
    let fold = |mut ids: Vec<TicketId>, status: TicketStatus| {
        if !view.expanded && matches!(status, TicketStatus::Todo | TicketStatus::Proposed) {
            ids.truncate(SECTION_ROWS);
        }
        ids
    };
    let group = |statuses: &[TicketStatus]| -> Vec<TicketId> {
        model.tickets.iter().filter(|t| statuses.contains(&t.status) && in_focus(model, t)).map(|t| t.id).collect()
    };
    let mut out = group(&[TicketStatus::Review]);
    out.extend(group(&[TicketStatus::Waiting]));
    let open = [TicketStatus::InProgress, TicketStatus::Todo, TicketStatus::Proposed];
    if lanes(model) {
        // A ticket about a folder the squad no longer lists goes in the last lane, not nowhere.
        let lane_of = |id: &TicketId| -> Option<PathBuf> {
            model.tickets.iter().find(|t| t.id == *id).and_then(|t| model.ticket_project(t)).filter(|p| model.projects.contains(p))
        };
        for lane in model.projects.iter().map(Some).chain([None]) {
            for status in open {
                let in_lane: Vec<TicketId> = group(&[status]).into_iter().filter(|id| lane_of(id).as_ref() == lane).collect();
                out.extend(fold(in_lane, status));
            }
        }
    } else {
        for status in open {
            out.extend(fold(group(&[status]), status));
        }
    }
    let mut done = group(&[TicketStatus::Done, TicketStatus::Cancelled]);
    done.reverse();
    done.truncate(DONE_SHOWN);
    out.extend(done);
    out
}

/// The list as rows: a heading per section, a ticket per row, and a line where
/// a section was folded.
pub fn rows(model: &Model, view: &View) -> Vec<Row> {
    let ids = order(model, view);
    let q = query(view);
    let lanes = lanes(model);
    let depth: std::collections::HashMap<TicketId, usize> = if view.graph && q.is_none() { dependency_tree(model).into_iter().collect() } else { Default::default() };
    let label_of = |t: &Ticket| -> (String, Tone) {
        if q.is_some() {
            return ("찾은 티켓".into(), Tone::Plain);
        }
        if view.graph {
            return ("무엇이 무엇을 기다리나".into(), Tone::Plain);
        }
        heading(model, t, lanes)
    };
    // Each section's whole size, so a folded one says what it left out.
    let mut totals: std::collections::HashMap<String, usize> = Default::default();
    if q.is_none() && !view.graph {
        for t in model.tickets.iter().filter(|t| in_focus(model, t)) {
            *totals.entry(label_of(t).0).or_default() += 1;
        }
    }
    let shown = |label: &str| ids.iter().filter_map(|id| model.tickets.iter().find(|t| t.id == *id)).filter(|t| label_of(t).0 == label).count();
    let folded = |label: &str| {
        let more = totals.get(label).copied().unwrap_or(0).saturating_sub(shown(label));
        (more > 0).then(|| Row::More { label: label.to_owned(), more })
    };
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    for id in &ids {
        let Some(t) = model.tickets.iter().find(|t| t.id == *id) else { continue };
        let (label, tone) = label_of(t);
        if current.as_ref() != Some(&label) {
            out.extend(current.as_deref().and_then(folded));
            let count = totals.get(&label).copied().unwrap_or_else(|| shown(&label));
            out.push(Row::Heading { label: label.clone(), count, tone });
            current = Some(label);
        }
        out.push(Row::Ticket { id: t.id, depth: depth.get(&t.id).copied().unwrap_or(0) });
    }
    out.extend(current.as_deref().and_then(folded));
    out
}

/// The ticket picked: the one chosen if it still shows, else the first listed.
pub fn selected<'a>(model: &'a Model, view: &View) -> Option<&'a Ticket> {
    let order = order(model, view);
    let id = view.selected.filter(|id| order.contains(id)).or_else(|| order.first().copied())?;
    model.tickets.iter().find(|t| t.id == id)
}

/// Move the pick by `step` rows through what is listed.
pub fn step(model: &Model, view: &mut View, step: isize) {
    let order = order(model, view);
    if order.is_empty() {
        return;
    }
    let at = selected(model, view).and_then(|t| order.iter().position(|id| *id == t.id)).unwrap_or(0) as isize;
    view.selected = Some(order[(at + step).clamp(0, order.len() as isize - 1) as usize]);
}

/// After a review is accepted, the next one comes up (or the one before, when
/// it was the last), so reviews go one after another.
pub fn next_review_after(model: &Model, view: &mut View, done: TicketId) {
    let list: Vec<TicketId> = order(model, view).into_iter().filter(|id| model.tickets.iter().any(|t| t.id == *id && t.status == TicketStatus::Review)).collect();
    let at = list.iter().position(|&id| id == done);
    let rest: Vec<TicketId> = list.into_iter().filter(|&id| id != done).collect();
    let next = match at {
        Some(at) => rest.get(at).or(rest.last()),
        None => rest.first(),
    };
    if let Some(&next) = next {
        view.selected = Some(next);
    }
}

/// The short state column: what is waiting on whom.
pub fn state_text(t: &Ticket) -> String {
    match t.status {
        TicketStatus::Review => "내 검토 대기".into(),
        TicketStatus::Waiting => "내가 할 일".into(),
        TicketStatus::InProgress => "작업 중".into(),
        TicketStatus::Todo if !t.depends_on.is_empty() => format!("after {}", t.depends_on.iter().map(|d| format!("T-{d}")).collect::<Vec<_>>().join(", ")),
        TicketStatus::Todo => "할 일".into(),
        TicketStatus::Proposed => "제안됨".into(),
        TicketStatus::Done if t.not_on_main() => "main에 없음".into(),
        TicketStatus::Done => "끝남".into(),
        TicketStatus::Cancelled => "취소됨".into(),
    }
}

/// Tickets open that wait for `id`.
pub fn unblocks(model: &Model, id: TicketId) -> Vec<TicketId> {
    model.tickets.iter().filter(|o| o.depends_on.contains(&id) && !o.status.is_closed()).map(|o| o.id).collect()
}

/// The person who does a ticket's work: whom Enter talks to.
pub fn who(t: &Ticket) -> Option<&str> {
    t.assignee.as_deref().or(t.owner.as_deref())
}

/// The steps a waiting ticket's note lists for the person: its checklist
/// lines (`- [ ] …`, `- …`, `1. …`), else the whole note's lines.
pub fn checklist(note: &str) -> Vec<String> {
    let strip = |line: &str| -> Option<String> {
        let line = line.trim();
        let line = line.strip_prefix(['-', '*', '•']).map(str::trim_start).or_else(|| {
            let digits = line.chars().take_while(char::is_ascii_digit).count();
            line[digits..].strip_prefix(['.', ')']).filter(|_| digits > 0).map(str::trim_start)
        })?;
        let line = line.strip_prefix("[ ]").or_else(|| line.strip_prefix("[x]")).or_else(|| line.strip_prefix("[X]")).unwrap_or(line).trim();
        (!line.is_empty()).then(|| line.to_owned())
    };
    note.lines().filter_map(strip).collect()
}

/// Whether the ticket has a branch to look at.
pub fn has_branch(t: &Ticket) -> bool {
    t.branch.is_some()
}

/// Whether accepting a code ticket goes on to the merge question.
pub fn asks_to_merge(t: &Ticket) -> bool {
    t.kind == TicketKind::Code && t.branch.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::Update;
    use crate::state::Clock;
    use equipe_core::protocol::rpc::Hello;
    use equipe_core::protocol::Snapshot;

    fn ticket(id: u32, status: &str, project: &str, deps: &[u32]) -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": format!("title {id}"), "body": "", "status": status, "owner": "backend", "assignee": null,
            "project": project, "depends_on": deps, "branch": null, "created_by": "pm", "note": null
        }))
        .unwrap()
    }

    fn model(tickets: Vec<Ticket>, projects: &[&str]) -> Model {
        let mut m = Model::new("acme");
        m.apply(
            Update::Attached(Hello {
                version: "x".into(),
                snapshot: Snapshot { squad: "acme".into(), tickets, projects: projects.iter().map(PathBuf::from).collect(), ..Default::default() },
            }),
            Clock { hour: 9, minute: 0, second: 0 },
        );
        m
    }

    fn ids(model: &Model, view: &View) -> Vec<u32> {
        order(model, view)
    }

    #[test]
    fn sections_run_review_waiting_progress_todo_proposed_then_the_latest_five_done() {
        let mut tickets = vec![
            ticket(1, "proposed", "/p", &[]),
            ticket(2, "todo", "/p", &[]),
            ticket(3, "in_progress", "/p", &[]),
            ticket(4, "waiting", "/p", &[]),
            ticket(5, "review", "/p", &[]),
        ];
        tickets.extend((10..18).map(|i| ticket(i, "done", "/p", &[])));
        let m = model(tickets, &["/p"]);
        assert_eq!(ids(&m, &View::default()), vec![5, 4, 3, 2, 1, 17, 16, 15, 14, 13]);
    }

    #[test]
    fn todo_and_proposed_fold_past_ten_until_shown_whole() {
        let tickets: Vec<Ticket> = (1..=14).map(|i| ticket(i, "todo", "/p", &[])).collect();
        let m = model(tickets, &["/p"]);
        let mut view = View::default();
        assert_eq!(ids(&m, &view).len(), 10);
        let rows = rows(&m, &view);
        assert_eq!(rows.last(), Some(&Row::More { label: "할 일".into(), more: 4 }));
        view.expanded = true;
        assert_eq!(ids(&m, &view).len(), 14);
    }

    #[test]
    fn several_projects_without_a_filter_give_each_a_lane_and_a_filter_takes_them_away() {
        let tickets = vec![ticket(1, "todo", "/a", &[]), ticket(2, "todo", "/b", &[]), ticket(3, "in_progress", "/a", &[]), ticket(4, "review", "/b", &[])];
        let mut m = model(tickets, &["/a", "/b"]);
        assert!(lanes(&m));
        // Review first; then a's lane (in progress before todo), then b's.
        assert_eq!(ids(&m, &View::default()), vec![4, 3, 1, 2]);
        let labels: Vec<String> = rows(&m, &View::default()).into_iter().filter_map(|r| if let Row::Heading { label, .. } = r { Some(label) } else { None }).collect();
        assert_eq!(labels, vec!["내 검토", "a", "b"]);
        m.set_focus(Some("a".into()));
        assert!(!lanes(&m));
        assert_eq!(ids(&m, &View::default()), vec![3, 1]);
    }

    #[test]
    fn the_tree_puts_each_ticket_under_what_it_waits_for() {
        let tickets = vec![ticket(1, "in_progress", "/p", &[]), ticket(2, "todo", "/p", &[1]), ticket(3, "todo", "/p", &[2]), ticket(4, "todo", "/p", &[]), ticket(5, "done", "/p", &[])];
        let m = model(tickets, &["/p"]);
        assert_eq!(dependency_tree(&m), vec![(1, 0), (2, 1), (3, 2), (4, 0)]);
        let view = View { graph: true, ..Default::default() };
        assert_eq!(ids(&m, &view), vec![1, 2, 3, 4]);
        assert!(rows(&m, &view).contains(&Row::Ticket { id: 3, depth: 2 }));
    }

    #[test]
    fn a_loop_of_waiting_is_listed_not_lost() {
        let m = model(vec![ticket(1, "todo", "/p", &[2]), ticket(2, "todo", "/p", &[1])], &["/p"]);
        assert_eq!(dependency_tree(&m).len(), 2);
    }

    #[test]
    fn finding_reaches_title_body_number_and_finished_tickets() {
        let mut a = ticket(1, "todo", "/p", &[]);
        a.title = "Login page".into();
        let mut b = ticket(2, "done", "/p", &[]);
        b.body = "fix the login redirect".into();
        let c = ticket(3, "done", "/p", &[]);
        let m = model(vec![a, b, c], &["/p"]);
        let find = |q: &str| ids(&m, &View { query: q.into(), ..Default::default() });
        assert_eq!(find("LOGIN"), vec![1, 2], "open first, then done");
        assert_eq!(find("t-3"), vec![3]);
        assert_eq!(find("nothing"), Vec::<u32>::new());
        assert_eq!(find("  "), ids(&m, &View::default()), "blank is no search");
    }

    #[test]
    fn the_pick_stays_while_it_shows_and_moves_with_the_arrows() {
        let m = model((1..=3).map(|i| ticket(i, "todo", "/p", &[])).collect(), &["/p"]);
        let mut view = View::default();
        assert_eq!(selected(&m, &view).map(|t| t.id), Some(1));
        step(&m, &mut view, 1);
        step(&m, &mut view, 1);
        step(&m, &mut view, 1);
        assert_eq!(selected(&m, &view).map(|t| t.id), Some(3), "stops at the end");
        view.selected = Some(99);
        assert_eq!(selected(&m, &view).map(|t| t.id), Some(1), "a ticket that went away falls back to the first");
    }

    #[test]
    fn accepting_a_review_brings_up_the_next_one_or_the_one_before() {
        let m = model(vec![ticket(3, "review", "/p", &[]), ticket(4, "review", "/p", &[]), ticket(5, "review", "/p", &[])], &["/p"]);
        let mut view = View::default();
        next_review_after(&m, &mut view, 4);
        assert_eq!(view.selected, Some(5));
        next_review_after(&m, &mut view, 5);
        assert_eq!(view.selected, Some(4));
    }

    #[test]
    fn the_state_column_says_what_it_waits_for() {
        assert_eq!(state_text(&ticket(1, "todo", "/p", &[53, 22])), "after T-53, T-22");
        assert_eq!(state_text(&ticket(1, "review", "/p", &[])), "내 검토 대기");
        let mut t = ticket(1, "done", "/p", &[]);
        t.branch = Some("b".into());
        t.merged = Some(false);
        assert_eq!(state_text(&t), "main에 없음");
    }

    #[test]
    fn a_waiting_note_becomes_a_checklist() {
        let note = "Needs your part:\n- [ ] Create the API key\n- add it to .env.local\n2. restart\n\nthanks";
        assert_eq!(checklist(note), vec!["Create the API key", "add it to .env.local", "restart"]);
        assert!(checklist("nothing listed").is_empty());
    }
}
