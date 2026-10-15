//! 티켓's rules, with no UI types: the list or the board (kanban), the board's
//! columns, what a card dropped somewhere means and sends, a ticket edited
//! here as it will look once the daemon has it, a new ticket's body, and the
//! edits several chosen tickets take at once. Every change is the person's
//! own command; the daemon checks it again (`equipe_core::runtime`'s
//! tickets) and tells the PM and the members it concerns.

use std::path::PathBuf;

use equipe_core::board::{self as rules, Move, Refusal};
use equipe_core::protocol::{Command, Priority, Ticket, TicketDraft, TicketEdit, TicketId, TicketKind, TicketStatus};
use equipe_core::t;

use crate::board::{Filter, who};
use crate::state::{Model, status_word};

/// How 티켓 lays its tickets out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// Rows, grouped: the default.
    #[default]
    List,
    /// Columns by status, cards to drag.
    Board,
}

impl Layout {
    pub fn code(self) -> &'static str {
        match self {
            Layout::List => "list",
            Layout::Board => "board",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        [Layout::List, Layout::Board].into_iter().find(|l| l.code() == code.trim())
    }

    pub fn other(self) -> Self {
        match self {
            Layout::List => Layout::Board,
            Layout::Board => Layout::List,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Layout::List => t!("tickets.layout_list"),
            Layout::Board => t!("tickets.layout_board"),
        }
    }
}

/// The board's columns, left to right; the last holds finished work.
pub const COLUMNS: [TicketStatus; 6] =
    [TicketStatus::Proposed, TicketStatus::Todo, TicketStatus::InProgress, TicketStatus::Waiting, TicketStatus::Review, TicketStatus::Done];

/// How far back the finished column reaches until "more" is asked for.
pub const DONE_DAYS: u64 = 7;

/// A column of the board: its status and its cards, in order; for the
/// finished one, how many older ones "more" would show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub status: TicketStatus,
    pub cards: Vec<TicketId>,
    pub hidden: usize,
}

/// The column a ticket's card sits in: the finished one for done and cancelled.
pub fn column_of(status: TicketStatus) -> TicketStatus {
    if status.is_closed() { TicketStatus::Done } else { status }
}

/// When a ticket last changed, as best known: the daemon's word, else when
/// this window saw it move.
pub fn changed_at(model: &Model, t: &Ticket) -> u64 {
    t.updated_at.filter(|at| *at > 0).unwrap_or_else(|| model.since.get(&t.id).copied().unwrap_or(0))
}

/// Open tickets in board order: as the person placed them, else by number.
pub fn in_board_order(mut tickets: Vec<&Ticket>) -> Vec<&Ticket> {
    rules::board_order(&mut tickets);
    tickets
}

/// The board, filtered as asked: a column per status, the finished one the
/// last week's (all of them with `all_done`), latest first.
pub fn columns(model: &Model, filter: &Filter, all_done: bool) -> Vec<Column> {
    let here: Vec<&Ticket> = model.tickets.iter().filter(|t| model.in_focus(t) && filter.matches(model, t)).collect();
    COLUMNS
        .into_iter()
        .map(|status| {
            if status == TicketStatus::Done {
                let cancelled = filter.status.contains(&TicketStatus::Cancelled);
                let mut done: Vec<&Ticket> = here.iter().copied().filter(|t| t.status == TicketStatus::Done || (cancelled && t.status == TicketStatus::Cancelled)).collect();
                done.sort_by_key(|t| (std::cmp::Reverse(changed_at(model, t)), std::cmp::Reverse(t.id)));
                let recent = |t: &&Ticket| {
                    let at = changed_at(model, t);
                    at == 0 || model.unix.saturating_sub(at) <= DONE_DAYS * 86_400
                };
                let shown: Vec<TicketId> = done.iter().filter(|t| all_done || recent(t)).map(|t| t.id).collect();
                let hidden = done.len() - shown.len();
                Column { status, cards: shown, hidden }
            } else {
                let open = in_board_order(here.iter().copied().filter(|t| t.status == status).collect());
                Column { status, cards: open.iter().map(|t| t.id).collect(), hidden: 0 }
            }
        })
        .collect()
}

// ── dropping a card ──

/// What a card dropped in a column (before a card, or last) does.
#[derive(Debug, Clone, PartialEq)]
pub enum Drop {
    /// Nothing changes: dropped where it was, or where order means nothing.
    Stay,
    /// Send these; the ticket looks like `after` meanwhile.
    Send { commands: Vec<Command>, after: Box<Ticket>, what: Move },
    /// Cancelling: asked once before it is held.
    ConfirmCancel,
}

/// Why the person can't put a ticket there by hand. The statuses after
/// 할 일 belong to the members' work and to accepting (A), not to a drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// A rule of the board's own (`equipe_core::board::move_rule`).
    Core(Refusal),
    /// Done is accepting: A does it, with its question of how to land.
    AcceptWithA,
    /// Back from work under way or handed in: its buttons do it (R, A, N).
    NotByHand,
}

/// The rank that puts `id` before `before` in the column of `status`, as this
/// window sees it (the daemon spreads a crowded column out; here the
/// neighbour's own rank, nudged, is close enough to show).
pub fn local_rank(model: &Model, id: TicketId, status: TicketStatus, before: Option<TicketId>) -> i64 {
    let column = in_board_order(model.tickets.iter().filter(|t| t.status == status && t.id != id).collect());
    rules::rank_before(&column, before).unwrap_or_else(|| {
        let at = before.and_then(|b| column.iter().position(|t| t.id == b)).unwrap_or(0);
        column.get(at).map_or(0, |t| t.order_key() - 1)
    })
}

/// What the person may do to a ticket's status by hand — a drag, or the
/// status menu: take a proposal on, order what is ready (and the proposals),
/// cancel anything open. Into 진행 중, 내 차례, 검토 or 끝남 it never goes by
/// hand: members and accepting own those.
pub fn person_rule(t: &Ticket, to: TicketStatus) -> Result<Move, Why> {
    use TicketStatus::*;
    let from = column_of(t.status);
    if from == to {
        return Ok(Move::Reorder);
    }
    match (from, to) {
        (_, Cancelled) if t.status.is_closed() => Err(Why::Core(Refusal::Closed)),
        (_, Cancelled) => Ok(Move::Cancel),
        (Proposed, Todo) => Ok(Move::TakeOn),
        (_, InProgress) => Err(Why::Core(Refusal::StartsByItself)),
        (_, Waiting) => Err(Why::Core(Refusal::WaitsByMember)),
        (_, Review) => Err(Why::Core(Refusal::HandedInByMember)),
        (Review, Done) => Err(Why::AcceptWithA),
        (_, Done) => Err(Why::Core(Refusal::NotHandedIn)),
        _ => Err(Why::NotByHand),
    }
}

/// The words a refusal shows on the column the card is over.
pub fn why_words(why: Why) -> &'static str {
    match why {
        Why::Core(Refusal::StartsByItself) => t!("tickets.refuse_starts_by_itself"),
        Why::Core(Refusal::HandedInByMember) => t!("tickets.refuse_handed_in_by_member"),
        Why::Core(Refusal::WaitsByMember) => t!("tickets.refuse_waits_by_member"),
        Why::Core(Refusal::NotHandedIn) => t!("tickets.refuse_not_handed_in"),
        Why::Core(Refusal::UnderWay) => t!("tickets.refuse_under_way"),
        Why::Core(Refusal::Closed) => t!("tickets.refuse_closed"),
        Why::AcceptWithA => t!("tickets.refuse_accept_with_a"),
        Why::NotByHand => t!("tickets.refuse_not_by_hand"),
    }
}

/// What dropping `t` in the column of `to`, before `before` (`None`: last), sends.
pub fn drop_card(model: &Model, t: &Ticket, to: TicketStatus, before: Option<TicketId>) -> Result<Drop, Why> {
    let what = person_rule(t, to)?;
    if what == Move::Cancel {
        return Ok(Drop::ConfirmCancel);
    }
    // Order means something where work waits to be picked up: 할 일, and the proposals.
    let ordered = matches!(t.status, TicketStatus::Todo | TicketStatus::Proposed);
    let same_place = before == Some(t.id) || (what == Move::Reorder && next_in_column(model, t) == before && before.is_some());
    if what == Move::Reorder && (same_place || !ordered) {
        return Ok(Drop::Stay);
    }
    let status = (what != Move::Reorder).then_some(to);
    let commands = vec![Command::MoveTicket { squad: model.squad.clone(), ticket: t.id, status, before }];
    let mut after = t.clone();
    after.rank = local_rank(model, t.id, to, before);
    if what == Move::TakeOn {
        after.status = TicketStatus::Todo;
    }
    Ok(Drop::Send { commands, after: Box::new(after), what })
}

/// The ticket after `t` in its column, in board order.
fn next_in_column(model: &Model, t: &Ticket) -> Option<TicketId> {
    let column = in_board_order(model.tickets.iter().filter(|x| x.status == t.status).collect());
    let at = column.iter().position(|x| x.id == t.id)?;
    column.get(at + 1).map(|x| x.id)
}

/// The toast a move says, held for a moment: `T-12 → 할 일`.
pub fn move_words(t: &Ticket, what: Move, to: TicketStatus) -> String {
    match what {
        Move::Reorder => t!("tickets.toast_reordered", id = t.id),
        Move::Cancel => t!("board.toast_cancel", id = t.id),
        Move::TakeOn => t!("board.toast_proposal", id = t.id),
        _ => t!("tickets.toast_moved", id = t.id, status = status_word(to)),
    }
}

// ── editing ──

/// `t` as it will look once the daemon has made `edit`.
pub fn edited(t: &Ticket, edit: &TicketEdit) -> Ticket {
    let mut out = t.clone();
    if let Some(title) = &edit.title {
        out.title = title.trim().to_owned();
    }
    if let Some(body) = &edit.body {
        out.body = body.trim().to_owned();
    }
    if let Some(project) = &edit.project {
        out.project = project.clone();
    }
    if let Some(owner) = &edit.assignee {
        out.owner = owner.clone();
        if t.status != TicketStatus::InProgress {
            out.assignee = None;
        }
    }
    if let Some(priority) = edit.priority {
        out.priority = priority;
    }
    if let Some(deps) = &edit.depends_on {
        out.depends_on = deps.clone();
    }
    if let Some(kind) = edit.kind {
        out.kind = kind;
    }
    out
}

/// Why `edit` can't be made to `t`, in the person's words — what the daemon
/// would refuse, said before anything is sent.
pub fn edit_refused(model: &Model, t: &Ticket, edit: &TicketEdit) -> Option<String> {
    if edit.title.as_deref().is_some_and(|s| s.trim().is_empty()) {
        return Some(t!("tickets.need_title").to_owned());
    }
    let started = t.branch.is_some() || t.status == TicketStatus::InProgress;
    if edit.project.as_ref().is_some_and(|p| *p != t.project) && started {
        return Some(t!("tickets.project_under_way", id = t.id));
    }
    if edit.kind.is_some_and(|k| k != t.kind) && started {
        return Some(t!("tickets.kind_under_way", id = t.id));
    }
    if let Some(owner) = &edit.assignee
        && owner.as_deref() != who(t)
    {
        if t.status.is_closed() {
            return Some(t!("tickets.owner_closed", id = t.id));
        }
        if owner.is_none() && t.status == TicketStatus::InProgress {
            return Some(t!("tickets.owner_under_way", id = t.id));
        }
    }
    if let Some(deps) = &edit.depends_on {
        if deps.contains(&t.id) {
            return Some(t!("tickets.waits_on_itself", id = t.id));
        }
        if let Some(path) = rules::dependency_loop(&model.tickets, t.id, deps) {
            let path: Vec<String> = path.iter().map(|id| format!("T-{id}")).collect();
            return Some(t!("tickets.dependency_loop", path = path.join(" → ")));
        }
    }
    None
}

/// Whether `edit` changes anything of `t`.
pub fn changes_anything(t: &Ticket, edit: &TicketEdit) -> bool {
    edited(t, edit) != *t
}

/// The toast an edit says: `T-12 우선순위 → 긴급`.
pub fn edit_words(ids: &[TicketId], edit: &TicketEdit) -> String {
    let what = if let Some(priority) = edit.priority {
        format!("{} → {}", t!("tickets.prop_priority"), priority_word(priority))
    } else if let Some(owner) = &edit.assignee {
        format!("{} → {}", t!("tickets.prop_assignee"), owner.clone().unwrap_or_else(|| t!("tickets.pm_decides").into()))
    } else if let Some(project) = &edit.project {
        let name = project.as_deref().map_or_else(|| t!("board.squad_folder").to_owned(), equipe_core::board::project_name);
        format!("{} → {name}", t!("tickets.prop_project"))
    } else if let Some(kind) = edit.kind {
        format!("{} → {}", t!("tickets.prop_kind"), equipe_core::i18n::label::kind(crate::i18n::lang(), kind))
    } else if let Some(deps) = &edit.depends_on {
        let list = if deps.is_empty() { "—".to_owned() } else { deps.iter().map(|d| format!("T-{d}")).collect::<Vec<_>>().join(", ") };
        format!("{} → {list}", t!("tickets.prop_depends"))
    } else if edit.title.is_some() {
        t!("tickets.prop_title").to_owned()
    } else {
        t!("tickets.prop_description").to_owned()
    };
    match ids {
        [one] => t!("tickets.toast_edited", id = one, what = what),
        many => t!("tickets.toast_edited_many", count = many.len(), what = what),
    }
}

/// A priority, in a word.
pub fn priority_word(priority: Priority) -> &'static str {
    match priority {
        Priority::Urgent => t!("tickets.priority_urgent"),
        Priority::High => t!("tickets.priority_high"),
        Priority::Normal => t!("tickets.priority_normal"),
        Priority::Low => t!("tickets.priority_low"),
    }
}

/// Who a ticket can be for: every member, the PM first, by name and role —
/// the daemon's own list of who exists.
pub fn owners(model: &Model) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = model
        .members
        .iter()
        .filter(|m| m.presence != crate::state::Presence::Closed)
        .map(|m| (m.name.clone(), m.role.clone()))
        .collect();
    out.sort_by_key(|(name, role)| (role != "pm", name.clone()));
    out.dedup();
    out
}

/// The statuses the status menu offers, each with why not when it can't be picked.
pub fn status_choices(t: &Ticket) -> Vec<(TicketStatus, Option<Why>)> {
    [TicketStatus::Proposed, TicketStatus::Todo, TicketStatus::InProgress, TicketStatus::Waiting, TicketStatus::Review, TicketStatus::Done, TicketStatus::Cancelled]
        .into_iter()
        .map(|s| (s, if s == t.status { None } else { person_rule(t, s).err() }))
        .collect()
}

// ── a new ticket ──

/// What the person wrote in the new ticket sheet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sheet {
    pub title: String,
    pub description: String,
    /// What makes it done, one line each.
    pub criteria: Vec<String>,
    pub project: Option<PathBuf>,
    /// `None`: the PM decides.
    pub assignee: Option<String>,
    pub priority: Priority,
    pub depends_on: Vec<TicketId>,
    pub kind: Option<TicketKind>,
}

/// The body a new ticket gets: its description, then its acceptance criteria
/// as a checklist — what members and the review read as "done when".
pub fn compose_body(description: &str, criteria: &[String]) -> String {
    let items: Vec<String> = criteria.iter().map(|c| c.trim()).filter(|c| !c.is_empty()).map(|c| format!("- [ ] {c}")).collect();
    let description = description.trim();
    match (description.is_empty(), items.is_empty()) {
        (_, true) => description.to_owned(),
        (true, false) => format!("__{}__\n{}", t!("tickets.criteria_heading"), items.join("\n")),
        (false, false) => format!("{description}\n\n__{}__\n{}", t!("tickets.criteria_heading"), items.join("\n")),
    }
}

impl Sheet {
    /// The command it makes, or why not yet.
    pub fn command(&self, squad: &str) -> Result<Command, &'static str> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err(t!("tickets.need_title"));
        }
        Ok(Command::CreateTicket {
            squad: squad.to_owned(),
            draft: TicketDraft {
                title: title.to_owned(),
                body: compose_body(&self.description, &self.criteria),
                project: self.project.clone(),
                assignee: self.assignee.clone(),
                priority: self.priority,
                depends_on: self.depends_on.clone(),
                kind: self.kind,
            },
        })
    }
}

/// What a word of the quick line (C) set, shown as a chip beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Owner(String),
    Project(String),
    Priority(Priority),
    After(TicketId),
}

/// The quick line read: `@frontend #web !high after T-38 결제 오류 문구` — who,
/// which project, how pressing, what it waits on; the other words are its
/// title. A word naming nobody (or no project) stays in the title.
pub fn parse_quick(text: &str, model: &Model) -> (Sheet, Vec<Token>) {
    let owners = owners(model);
    let mut sheet = Sheet::default();
    let mut tokens = Vec::new();
    let mut title: Vec<&str> = Vec::new();
    let mut words = text.split_whitespace().peekable();
    let ticket_id = |w: &str| equipe_core::board::parse_ticket_id(w.trim_end_matches(','));
    while let Some(word) = words.next() {
        if let Some(name) = word.strip_prefix('@').filter(|n| owners.iter().any(|(m, r)| m == n || r == n)) {
            sheet.assignee = Some(name.to_owned());
            tokens.push(Token::Owner(name.to_owned()));
        } else if let Some(path) = word.strip_prefix('#').and_then(|n| model.projects.iter().find(|p| equipe_core::board::project_name(p) == n)) {
            sheet.project = Some(path.clone());
            tokens.push(Token::Project(equipe_core::board::project_name(path)));
        } else if let Some(priority) = word.strip_prefix('!').and_then(priority_of) {
            sheet.priority = priority;
            tokens.push(Token::Priority(priority));
        } else if let Some(id) = word.strip_prefix("after:").and_then(ticket_id) {
            sheet.depends_on.push(id);
            tokens.push(Token::After(id));
        } else if (word == "after" || word == "선행") && words.peek().is_some_and(|w| ticket_id(w).is_some()) {
            let id = words.next().and_then(ticket_id).unwrap_or_default();
            sheet.depends_on.push(id);
            tokens.push(Token::After(id));
        } else {
            title.push(word);
        }
    }
    sheet.title = title.join(" ");
    (sheet, tokens)
}

/// `!high`, `!긴급`, `!1` (urgent) … `!4` (low).
fn priority_of(word: &str) -> Option<Priority> {
    Priority::parse(word).or(match word {
        "긴급" | "1" | "u" => Some(Priority::Urgent),
        "높음" | "2" | "h" => Some(Priority::High),
        "보통" | "3" | "n" => Some(Priority::Normal),
        "낮음" | "4" | "l" => Some(Priority::Low),
        _ => None,
    })
}

/// What the PM is asked when the person hands it a quick line to shape (⇧⌘⏎).
pub fn shape_request(line: &str) -> String {
    format!(
        "The person wrote a ticket in a line and wants you to shape it: “{}”. Write what to do and its acceptance criteria, \
split it into several tickets if it is several pieces of work, keep what the line says (@owner, #project, !priority, after T-n), \
and put it on the board as a proposal (create_ticket with proposal: true) for them to take on.",
        line.trim()
    )
}

/// Tickets the dependency search finds for `words`, open ones first, never
/// `except` (the ticket being edited) or those already listed.
pub fn find_tickets<'a>(model: &'a Model, words: &str, except: Option<TicketId>, have: &[TicketId]) -> Vec<&'a Ticket> {
    let words = words.trim().to_lowercase();
    let mut found: Vec<&Ticket> = model
        .tickets
        .iter()
        .filter(|t| Some(t.id) != except && !have.contains(&t.id))
        .filter(|t| words.is_empty() || t.title.to_lowercase().contains(&words) || format!("t-{}", t.id) == words || t.id.to_string() == words.trim_start_matches("t-"))
        .collect();
    found.sort_by_key(|t| (t.status.is_closed(), std::cmp::Reverse(t.id)));
    found.truncate(8);
    found
}

/// Whether `edit` takes work under way from the member on it — what the
/// person is asked about first (its member stops; its worktree and branch stay).
pub fn stops_work(pairs: &[(Ticket, Ticket)], edit: &TicketEdit) -> bool {
    edit.assignee.is_some() && pairs.iter().any(|(was, now)| was.status == TicketStatus::InProgress && who(was) != now.owner.as_deref())
}

/// Where a ticket waiting to be picked up stands in its owner's queue: 1 is
/// next. The board's order decides, priority only between equals — as the
/// daemon picks (`equipe_core::board::assignments`).
pub fn queue_place(model: &Model, t: &Ticket) -> Option<usize> {
    if t.status != TicketStatus::Todo {
        return None;
    }
    let owner = who(t)?;
    let mut queue: Vec<&Ticket> = model.tickets.iter().filter(|x| x.status == TicketStatus::Todo && who(x) == Some(owner)).collect();
    queue.sort_by_key(|x| (x.order_key(), x.priority, x.id));
    queue.iter().position(|x| x.id == t.id).map(|at| at + 1)
}

// ── several at once ──

/// The edit chosen tickets take together: each one's, without those it
/// wouldn't change or the daemon would refuse (those are said, and skipped).
pub fn bulk_edits(model: &Model, chosen: &[TicketId], edit: &TicketEdit) -> (Vec<(Ticket, Ticket)>, Vec<String>) {
    let mut ok = Vec::new();
    let mut refused = Vec::new();
    for id in chosen {
        let Some(t) = model.tickets.iter().find(|t| t.id == *id) else { continue };
        match edit_refused(model, t, edit) {
            Some(why) => refused.push(why),
            None if changes_anything(t, edit) => ok.push((t.clone(), edited(t, edit))),
            None => {}
        }
    }
    (ok, refused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::Update;
    use crate::state::Clock;
    use equipe_core::protocol::Snapshot;
    use equipe_core::protocol::rpc::Hello;

    fn ticket(id: u32, status: &str) -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": format!("title {id}"), "body": "", "status": status, "owner": "backend", "assignee": null,
            "project": "/p", "depends_on": [], "branch": null, "created_by": "pm", "note": null
        }))
        .unwrap()
    }

    fn model(tickets: Vec<Ticket>) -> Model {
        let mut m = Model::new("acme");
        m.apply(
            Update::Attached(Hello { version: "x".into(), snapshot: Snapshot { squad: "acme".into(), tickets, projects: vec!["/p".into()], ..Default::default() } }),
            Clock { hour: 9, minute: 0, second: 0 },
        );
        m.unix = 100 * 86_400;
        m
    }

    #[test]
    fn the_layout_is_remembered_by_its_code_and_list_is_the_default() {
        assert_eq!(Layout::default(), Layout::List);
        assert_eq!(Layout::parse("board"), Some(Layout::Board));
        assert_eq!(Layout::parse("kanban"), None);
        assert_eq!(Layout::List.other(), Layout::Board);
    }

    #[test]
    fn the_board_has_a_column_per_status_in_the_persons_order_and_the_last_week_finished() {
        let mut moved = ticket(3, "todo");
        moved.rank = 1;
        let mut old = ticket(8, "done");
        old.updated_at = Some(100 * 86_400 - 30 * 86_400);
        let mut fresh = ticket(9, "done");
        fresh.updated_at = Some(100 * 86_400 - 3600);
        let mut cancelled = ticket(10, "cancelled");
        cancelled.updated_at = Some(100 * 86_400 - 60);
        let m = model(vec![ticket(1, "proposed"), ticket(2, "todo"), moved, ticket(4, "in_progress"), ticket(5, "waiting"), ticket(6, "review"), old, fresh, cancelled]);
        let cols = columns(&m, &Filter::default(), false);
        let ids: Vec<Vec<u32>> = cols.iter().map(|c| c.cards.clone()).collect();
        assert_eq!(ids, vec![vec![1], vec![3, 2], vec![4], vec![5], vec![6], vec![9]], "cancelled work out of sight");
        assert_eq!(cols[5].hidden, 1, "older finished work behind more");
        assert_eq!(columns(&m, &Filter::default(), true)[5].cards, vec![9, 8]);
        assert_eq!(columns(&m, &Filter::parse("status:cancelled"), false)[5].cards, vec![10], "unless asked for");
        assert_eq!(columns(&m, &Filter::parse("title 2"), false)[1].cards, vec![2], "the filter narrows the board too");
    }

    #[test]
    fn by_hand_a_proposal_is_taken_on_ready_work_reordered_and_anything_cancelled_nothing_else() {
        let mut review = ticket(6, "review");
        review.branch = Some("equipe/x".into());
        let m = model(vec![ticket(1, "proposed"), ticket(2, "todo"), ticket(3, "todo"), review.clone(), ticket(7, "in_progress"), ticket(8, "done")]);
        let t = |id| m.tickets.iter().find(|t| t.id == id).unwrap().clone();
        // A proposal to 할 일: taken on, sent as a move, shown there at once.
        let Ok(Drop::Send { commands, after, what }) = drop_card(&m, &t(1), TicketStatus::Todo, Some(2)) else { panic!() };
        assert_eq!(what, Move::TakeOn);
        assert_eq!(commands, vec![Command::MoveTicket { squad: "acme".into(), ticket: 1, status: Some(TicketStatus::Todo), before: Some(2) }]);
        assert_eq!(after.status, TicketStatus::Todo);
        assert!(after.order_key() < t(2).order_key(), "above T-2");
        // Within 할 일: only its place.
        let Ok(Drop::Send { commands, .. }) = drop_card(&m, &t(3), TicketStatus::Todo, Some(2)) else { panic!() };
        assert_eq!(commands, vec![Command::MoveTicket { squad: "acme".into(), ticket: 3, status: None, before: Some(2) }]);
        assert_eq!(drop_card(&m, &t(2), TicketStatus::Todo, Some(3)), Ok(Drop::Stay), "where it was");
        assert_eq!(drop_card(&m, &t(7), TicketStatus::InProgress, None), Ok(Drop::Stay), "order means nothing under way");
        // Cancelling anything open: asked first.
        assert_eq!(drop_card(&m, &t(7), TicketStatus::Cancelled, None), Ok(Drop::ConfirmCancel));
        assert_eq!(drop_card(&m, &t(8), TicketStatus::Cancelled, None), Err(Why::Core(Refusal::Closed)));
        // Members' and accepting's statuses: never by hand, each said why.
        assert_eq!(drop_card(&m, &review, TicketStatus::Done, None), Err(Why::AcceptWithA), "A accepts");
        assert_eq!(drop_card(&m, &t(2), TicketStatus::InProgress, None), Err(Why::Core(Refusal::StartsByItself)));
        assert_eq!(drop_card(&m, &t(7), TicketStatus::Review, None), Err(Why::Core(Refusal::HandedInByMember)));
        assert_eq!(drop_card(&m, &t(2), TicketStatus::Waiting, None), Err(Why::Core(Refusal::WaitsByMember)));
        assert_eq!(drop_card(&m, &review, TicketStatus::Todo, None), Err(Why::NotByHand), "sent back with R");
        assert_eq!(drop_card(&m, &t(2), TicketStatus::Proposed, None), Err(Why::NotByHand));
        assert!(!why_words(Why::AcceptWithA).is_empty());
        // The status menu offers the same.
        let choices = status_choices(&t(1));
        assert!(choices.iter().any(|(s, why)| *s == TicketStatus::Todo && why.is_none()));
        assert!(choices.iter().any(|(s, why)| *s == TicketStatus::Done && why.is_some()));
    }

    #[test]
    fn an_edit_shows_at_once_and_what_the_daemon_would_refuse_is_said_first() {
        let mut a = ticket(1, "todo");
        a.depends_on = vec![2];
        let b = ticket(2, "todo");
        let mut working = ticket(3, "in_progress");
        working.branch = Some("equipe/b".into());
        let m = model(vec![a.clone(), b.clone(), working.clone()]);
        let edit = TicketEdit { priority: Some(Priority::Urgent), assignee: Some(Some("frontend".into())), ..Default::default() };
        let after = edited(&b, &edit);
        assert_eq!((after.priority, after.owner.as_deref()), (Priority::Urgent, Some("frontend")));
        assert!(changes_anything(&b, &edit));
        assert!(!changes_anything(&b, &TicketEdit { priority: Some(Priority::Normal), ..Default::default() }));
        let loop_ = TicketEdit { depends_on: Some(vec![1]), ..Default::default() };
        assert!(edit_refused(&m, &b, &loop_).unwrap().contains("T-2 → T-1 → T-2"));
        let moved = TicketEdit { project: Some(Some("/q".into())), ..Default::default() };
        assert!(edit_refused(&m, &working, &moved).is_some(), "work under way stays in its project");
        assert!(edit_refused(&m, &working, &TicketEdit { assignee: Some(None), ..Default::default() }).is_some());
        assert!(edit_refused(&m, &working, &TicketEdit { assignee: Some(Some("qa".into())), ..Default::default() }).is_none(), "given to someone else: fine");
        assert_eq!(edit_words(&[2], &TicketEdit { priority: Some(Priority::High), ..Default::default() }), "T-2 우선순위 → 높음");
    }

    #[test]
    fn several_chosen_take_one_edit_skipping_what_wouldnt_change_or_would_be_refused() {
        let mut done = ticket(3, "done");
        done.updated_at = Some(1);
        let mut urgent = ticket(2, "todo");
        urgent.priority = Priority::Urgent;
        let m = model(vec![ticket(1, "todo"), urgent, done]);
        let edit = TicketEdit { priority: Some(Priority::Urgent), ..Default::default() };
        let (ok, refused) = bulk_edits(&m, &[1, 2, 3], &edit);
        assert_eq!(ok.iter().map(|(t, _)| t.id).collect::<Vec<_>>(), vec![1, 3], "T-2 is urgent already");
        assert!(refused.is_empty());
        let give = TicketEdit { assignee: Some(Some("qa".into())), ..Default::default() };
        let (ok, refused) = bulk_edits(&m, &[1, 3], &give);
        assert_eq!((ok.len(), refused.len()), (1, 1), "finished work has no one to give it to");
        assert_eq!(edit_words(&[1, 2], &edit), "티켓 2개 우선순위 → 긴급");
        // Taking work under way from its member is asked about first.
        let mut working = ticket(5, "in_progress");
        working.assignee = Some("backend".into());
        let m = model(vec![working]);
        let (ok, _) = bulk_edits(&m, &[5], &give);
        assert!(stops_work(&ok, &give));
        assert!(!stops_work(&ok, &edit), "its priority isn't");
    }

    #[test]
    fn a_new_tickets_body_carries_its_acceptance_criteria_as_a_checklist() {
        assert_eq!(compose_body("  Do it  ", &["works offline".into(), " ".into(), "has tests".into()]), "Do it\n\n__완료 기준__\n- [ ] works offline\n- [ ] has tests");
        assert_eq!(compose_body("", &[]), "");
        assert_eq!(compose_body("x", &[]), "x");
        let sheet = Sheet { title: "  Hero ".into(), assignee: Some("frontend".into()), priority: Priority::High, ..Default::default() };
        let Ok(Command::CreateTicket { draft, .. }) = sheet.command("acme") else { panic!() };
        assert_eq!((draft.title.as_str(), draft.assignee.as_deref(), draft.priority), ("Hero", Some("frontend"), Priority::High));
        assert!(Sheet::default().command("acme").is_err(), "a title first");
        // The checklist reads back as what to check in review.
        assert_eq!(crate::board::checklist(&compose_body("x", &["a".into()])), vec!["a"]);
    }

    #[test]
    fn a_quick_line_reads_who_where_how_pressing_and_what_it_waits_on() {
        let mut m = model(vec![ticket(38, "todo")]);
        m.projects = vec!["/code/web".into(), "/code/app".into()];
        let started = equipe_core::protocol::Event::SessionStarted {
            session: 7,
            project: "/code/web".into(),
            engine: equipe_core::protocol::Engine::Claude,
            model: None,
            workspace: None,
            member: Some(equipe_core::protocol::Member { role: "frontend".into(), name: "frontend".into() }),
            squad: Some("acme".into()),
        };
        m.apply(Update::Event(started), Clock { hour: 9, minute: 0, second: 0 });
        let (sheet, tokens) = parse_quick("@frontend #web !high after T-38 결제 오류 문구 정리", &m);
        assert_eq!(sheet.title, "결제 오류 문구 정리");
        assert_eq!((sheet.assignee.as_deref(), sheet.priority, sheet.depends_on.clone()), (Some("frontend"), Priority::High, vec![38]));
        assert_eq!(sheet.project.as_deref(), Some(std::path::Path::new("/code/web")));
        assert_eq!(tokens, vec![Token::Owner("frontend".into()), Token::Project("web".into()), Token::Priority(Priority::High), Token::After(38)]);
        // Naming nobody, it is just a word of the title.
        let (sheet, tokens) = parse_quick("@nobody fix #nowhere !긴급 after:38", &m);
        assert_eq!((sheet.title.as_str(), sheet.priority), ("@nobody fix #nowhere", Priority::Urgent));
        assert_eq!(tokens.len(), 2);
        assert!(shape_request(" x ").contains("“x”"));
    }

    #[test]
    fn the_dependency_search_finds_by_words_or_number_open_first() {
        let mut closed = ticket(4, "done");
        closed.title = "Login form".into();
        let mut open = ticket(2, "todo");
        open.title = "Login API".into();
        let m = model(vec![ticket(1, "todo"), open, closed]);
        assert_eq!(find_tickets(&m, "login", None, &[]).iter().map(|t| t.id).collect::<Vec<_>>(), vec![2, 4]);
        assert_eq!(find_tickets(&m, "T-1", None, &[]).iter().map(|t| t.id).collect::<Vec<_>>(), vec![1]);
        assert!(find_tickets(&m, "login", Some(2), &[4]).is_empty());
    }
}
