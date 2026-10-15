//! The work screens' rules, with no UI types: what 나에게, 작업 중, 모든 일
//! and a saved view list, in which order, under which heading; the filter
//! words; moving through the list and on after an action; the actions held
//! back for a moment so they can be taken back; how accepting lands work.
//! The terminal board (`tui/app.rs`) follows the same order and words.

use equipe_core::t;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use equipe_core::protocol::{Command, OffMain, Priority, Ticket, TicketId, TicketKind, TicketStatus};

use crate::state::{Model, Need, Section, status_word};

/// The menu item the work screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Menu {
    /// What waits for the person: 나에게.
    #[default]
    Inbox,
    /// Work in progress: 작업 중.
    Working,
    /// Every ticket: 모든 일.
    All,
    /// A saved view, by its place in the list of views.
    View(usize),
}

/// How 모든 일 (and a saved view) groups its tickets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupBy {
    #[default]
    Status,
    Project,
    Member,
    Priority,
    /// A tree of what waits for what.
    Deps,
    None,
}

impl GroupBy {
    pub const ALL: [GroupBy; 6] = [GroupBy::Status, GroupBy::Project, GroupBy::Member, GroupBy::Priority, GroupBy::Deps, GroupBy::None];

    pub fn label(self) -> &'static str {
        match self {
            GroupBy::Status => t!("board.group_status"),
            GroupBy::Project => t!("board.group_project"),
            GroupBy::Member => t!("board.group_member"),
            GroupBy::Priority => t!("tickets.group_priority"),
            GroupBy::Deps => t!("board.group_deps"),
            GroupBy::None => t!("board.group_none"),
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            GroupBy::Status => "status",
            GroupBy::Project => "project",
            GroupBy::Member => "member",
            GroupBy::Priority => "priority",
            GroupBy::Deps => "deps",
            GroupBy::None => "none",
        }
    }

    pub fn parse(code: &str) -> Self {
        GroupBy::ALL.into_iter().find(|g| g.code() == code).unwrap_or_default()
    }
}

/// How far back the finished group reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DoneWindow {
    #[default]
    Week,
    Month,
    All,
}

impl DoneWindow {
    pub fn label(self) -> &'static str {
        match self {
            DoneWindow::Week => t!("board.window_week"),
            DoneWindow::Month => t!("board.window_month"),
            DoneWindow::All => t!("board.window_all"),
        }
    }

    pub fn next(self) -> Self {
        match self {
            DoneWindow::Week => DoneWindow::Month,
            DoneWindow::Month => DoneWindow::All,
            DoneWindow::All => DoneWindow::Week,
        }
    }

    fn seconds(self) -> Option<u64> {
        match self {
            DoneWindow::Week => Some(7 * 86_400),
            DoneWindow::Month => Some(30 * 86_400),
            DoneWindow::All => None,
        }
    }
}

/// A view the person keeps: a name, its filter words, its grouping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedView {
    pub name: String,
    pub query: String,
    pub group: GroupBy,
    pub pinned: bool,
}

/// The views every squad starts with: what is blocked, what finished lately.
pub fn builtin_views() -> Vec<SavedView> {
    vec![
        SavedView { name: t!("board.view_blocked").into(), query: "is:blocked".into(), group: GroupBy::Deps, pinned: true },
        SavedView { name: t!("board.view_recent_done").into(), query: "status:done".into(), group: GroupBy::None, pinned: true },
    ]
}

/// What the person has done to the work screen's view.
#[derive(Debug, Clone, Default)]
pub struct View {
    pub menu: Menu,
    pub group: GroupBy,
    /// Typed in the search box: words, and filter words (`project:web`).
    pub query: String,
    pub selected: Option<Need>,
    /// The picked item shows at the right.
    pub peek: bool,
    /// The picked ticket open as a page, on this tab.
    pub page: Option<Tab>,
    /// The finished group shown open, and how far back.
    pub done_open: bool,
    pub window: DoneWindow,
    /// Tickets chosen for one action together.
    pub chosen: BTreeSet<TicketId>,
    /// Where a ⇧↑↓ range started.
    pub anchor: Option<TicketId>,
    /// 티켓 as rows or as a board.
    pub layout: crate::tickets::Layout,
}

/// A ticket page's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Overview,
    Changes,
    Activity,
}

/// How a heading is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// The person's call: accent.
    ForYou,
    /// A project's: its dot (the project's position in the squad).
    Project(Option<usize>),
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A section's name and size — and, for one whose name alone doesn't say
    /// what it means, a line under it that does.
    Heading { label: String, count: usize, tone: Tone, hint: Option<String> },
    /// A thing listed, and how deep under what it waits for (the 의존 순서 grouping).
    Item { need: Need, depth: usize },
    /// The finished group's own line: `끝남 · 7일 12 ▸`.
    Done { count: usize, total: usize, open: bool, window: DoneWindow },
}

// ── filter words ──

/// The search box's words taken apart: filter words, and the rest to find.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub words: Vec<String>,
    pub project: Vec<String>,
    pub member: Vec<String>,
    pub status: Vec<TicketStatus>,
    pub kind: Vec<TicketKind>,
    pub blocked: bool,
    pub live: bool,
    pub quiet: bool,
    pub branch: bool,
    pub not_on_main: bool,
}

/// The filter words offered (F), in order.
pub const FILTER_WORDS: [&str; 6] = ["is:blocked", "is:live", "is:quiet", "has:branch", "not-on-main", "kind:code"];

fn status_of(word: &str) -> Option<TicketStatus> {
    Some(match word {
        "review" | "검토" => TicketStatus::Review,
        "waiting" | "내차례" | "내_차례" => TicketStatus::Waiting,
        "proposed" | "제안" => TicketStatus::Proposed,
        "todo" | "할일" | "할_일" => TicketStatus::Todo,
        "progress" | "in_progress" | "working" | "작업중" | "작업_중" => TicketStatus::InProgress,
        "done" | "끝남" => TicketStatus::Done,
        "cancelled" | "취소" => TicketStatus::Cancelled,
        _ => return None,
    })
}

impl Filter {
    pub fn parse(text: &str) -> Self {
        let mut f = Filter::default();
        for word in text.split_whitespace() {
            let lower = word.to_lowercase();
            match lower.split_once(':') {
                Some(("project", v)) if !v.is_empty() => f.project.push(v.to_owned()),
                Some(("member", v)) if !v.is_empty() => f.member.push(v.to_owned()),
                Some(("status", v)) if status_of(v).is_some() => f.status.extend(status_of(v)),
                Some(("kind", "code")) => f.kind.push(TicketKind::Code),
                Some(("kind", "task")) => f.kind.push(TicketKind::Task),
                Some(("kind", "action")) => f.kind.push(TicketKind::Action),
                Some(("is", "blocked")) => f.blocked = true,
                Some(("is", "live")) => f.live = true,
                Some(("is", "quiet")) => f.quiet = true,
                Some(("has", "branch")) => f.branch = true,
                _ if lower == "not-on-main" => f.not_on_main = true,
                _ => f.words.push(lower),
            }
        }
        f
    }

    /// Whether it narrows anything at all.
    pub fn is_empty(&self) -> bool {
        *self == Filter::default()
    }

    /// Whether it asks for finished work (then the finished group opens).
    pub fn reaches_done(&self) -> bool {
        !self.words.is_empty() || self.status.iter().any(|s| s.is_closed()) || self.not_on_main
    }

    pub fn matches(&self, model: &Model, t: &Ticket) -> bool {
        let project = model.ticket_project_name(t).unwrap_or_default().to_lowercase();
        let who = who(t).unwrap_or_default().to_lowercase();
        let text = format!("{} {}", t.title.to_lowercase(), t.body.to_lowercase());
        let id = format!("t-{}", t.id);
        self.words.iter().all(|w| text.contains(w.as_str()) || *w == id)
            && (self.project.is_empty() || self.project.contains(&project))
            && (self.member.is_empty() || self.member.contains(&who))
            && (self.status.is_empty() || self.status.contains(&t.status))
            && (self.kind.is_empty() || self.kind.contains(&t.kind))
            && (!self.blocked || (t.status == TicketStatus::Todo && model.blocked_by(t).is_some()))
            && (!self.live || model.is_live(t))
            && (!self.quiet || model.quiet_for(t).is_some())
            && (!self.branch || t.branch.is_some())
            && (!self.not_on_main || t.not_on_main())
    }
}

/// The filter words in `query` (not the words to find), each a chip.
pub fn chips(query: &str) -> Vec<String> {
    query.split_whitespace().filter(|w| w.contains(':') || *w == "not-on-main").map(str::to_owned).collect()
}

/// `query` with one word added (if missing) or taken out.
pub fn toggle_word(query: &str, word: &str) -> String {
    let mut words: Vec<&str> = query.split_whitespace().collect();
    match words.iter().position(|w| *w == word) {
        Some(at) => {
            words.remove(at);
        }
        None => words.push(word),
    }
    words.join(" ")
}

// ── the lists ──

/// The query and grouping a menu item lists with.
pub fn effective(view: &View, views: &[SavedView]) -> (String, GroupBy) {
    match view.menu {
        Menu::View(at) => views.get(at).map_or((view.query.clone(), view.group), |v| (format!("{} {}", v.query, view.query).trim().to_owned(), v.group)),
        _ => (view.query.clone(), view.group),
    }
}

/// The rows of the menu item on screen.
pub fn rows(model: &Model, view: &View, views: &[SavedView]) -> Vec<Row> {
    match view.menu {
        Menu::Inbox => inbox(model, &Filter::parse(&view.query)),
        Menu::Working => working(model, &Filter::parse(&view.query)),
        Menu::All | Menu::View(_) => {
            let (query, group) = effective(view, views);
            all(model, view, &Filter::parse(&query), group)
        }
    }
}

/// 나에게: by section, in [`Model::needs_you`]'s order.
pub fn inbox(model: &Model, filter: &Filter) -> Vec<Row> {
    let mut out = Vec::new();
    for section in Section::ALL {
        let items: Vec<Need> = model
            .needs_you()
            .into_iter()
            .filter(|(s, need)| {
                *s == section
                    && match need {
                        Need::Ticket(id) => model.tickets.iter().find(|t| t.id == *id).is_some_and(|t| filter.matches(model, t)),
                        Need::Permission(_) => filter.is_empty(),
                    }
            })
            .map(|(_, need)| need)
            .collect();
        if !items.is_empty() {
            out.push(Row::Heading { label: section.label().to_owned(), count: items.len(), tone: Tone::ForYou, hint: section.hint().map(str::to_owned) });
            out.extend(items.into_iter().map(|need| Row::Item { need, depth: 0 }));
        }
    }
    out
}

/// 작업 중: work in progress, the soonest done first.
pub fn working(model: &Model, filter: &Filter) -> Vec<Row> {
    let mut tickets: Vec<&Ticket> = model.tickets.iter().filter(|t| t.status == TicketStatus::InProgress && model.in_focus(t) && filter.matches(model, t)).collect();
    tickets.sort_by_key(|t| (model.eta(t).map_or(u64::MAX, |e| e.at), t.id));
    tickets.into_iter().map(|t| Row::Item { need: Need::Ticket(t.id), depth: 0 }).collect()
}

/// The order statuses are listed in, the person's calls first.
const STATUS_ORDER: [TicketStatus; 6] = [TicketStatus::Review, TicketStatus::Waiting, TicketStatus::InProgress, TicketStatus::Todo, TicketStatus::Proposed, TicketStatus::Done];

fn status_rank(s: TicketStatus) -> usize {
    STATUS_ORDER.iter().position(|x| *x == s).unwrap_or(STATUS_ORDER.len() - 1)
}

/// Finished tickets within the window, the latest first.
fn finished<'a>(model: &'a Model, tickets: &[&'a Ticket], window: DoneWindow) -> Vec<&'a Ticket> {
    let mut done: Vec<&Ticket> = tickets
        .iter()
        .copied()
        .filter(|t| t.status.is_closed())
        .filter(|t| {
            let at = crate::tickets::changed_at(model, t);
            window.seconds().is_none_or(|w| at > 0 && model.unix.saturating_sub(at) <= w)
        })
        .collect();
    done.sort_by_key(|t| (std::cmp::Reverse(crate::tickets::changed_at(model, t)), std::cmp::Reverse(t.id)));
    done
}

/// 모든 일 and the saved views: grouped as asked; finished work in one group
/// that opens on a click, or by itself when the search reaches it.
pub fn all(model: &Model, view: &View, filter: &Filter, group: GroupBy) -> Vec<Row> {
    let here: Vec<&Ticket> = model.tickets.iter().filter(|t| model.in_focus(t) && filter.matches(model, t)).collect();
    let open_done = view.done_open || filter.reaches_done();
    let window = if filter.reaches_done() && !view.done_open { DoneWindow::All } else { view.window };
    let mut out = Vec::new();
    let item = |t: &Ticket, depth: usize| Row::Item { need: Need::Ticket(t.id), depth };
    // Within a status, as the person ordered the board.
    let by_status = |list: &mut Vec<&Ticket>| list.sort_by_key(|t| (status_rank(t.status), t.order_key(), t.id));
    let open: Vec<&Ticket> = here.iter().copied().filter(|t| !t.status.is_closed()).collect();
    match group {
        GroupBy::Status => {
            for status in &STATUS_ORDER[..5] {
                let list = crate::tickets::in_board_order(open.iter().copied().filter(|t| t.status == *status).collect());
                if !list.is_empty() {
                    let tone = if matches!(status, TicketStatus::Review | TicketStatus::Waiting) { Tone::ForYou } else { Tone::Plain };
                    out.push(Row::Heading { label: status_word(*status).to_owned(), count: list.len(), tone, hint: None });
                    out.extend(list.into_iter().map(|t| item(t, 0)));
                }
            }
        }
        GroupBy::Project => {
            let mut names: Vec<Option<String>> = model.project_names().into_iter().map(Some).collect();
            names.push(None);
            for name in names {
                let mut list: Vec<&Ticket> = open.iter().copied().filter(|t| model.ticket_project_name(t) == name).collect();
                if !list.is_empty() {
                    by_status(&mut list);
                    let at = name.as_ref().and_then(|n| model.project_names().iter().position(|x| x == n));
                    out.push(Row::Heading { label: name.unwrap_or_else(|| t!("board.squad_folder").into()), count: list.len(), tone: Tone::Project(at), hint: None });
                    out.extend(list.into_iter().map(|t| item(t, 0)));
                }
            }
        }
        GroupBy::Member => {
            let mut whos: Vec<String> = open.iter().map(|t| who(t).unwrap_or_default().to_owned()).collect();
            whos.sort();
            whos.dedup();
            for name in whos {
                let mut list: Vec<&Ticket> = open.iter().copied().filter(|t| who(t).unwrap_or_default() == name).collect();
                by_status(&mut list);
                let label = if name.is_empty() { t!("board.nobody").to_owned() } else { name };
                out.push(Row::Heading { label, count: list.len(), tone: Tone::Plain, hint: None });
                out.extend(list.into_iter().map(|t| item(t, 0)));
            }
        }
        GroupBy::Priority => {
            for priority in Priority::ALL {
                let mut list: Vec<&Ticket> = open.iter().copied().filter(|t| t.priority == priority).collect();
                if !list.is_empty() {
                    by_status(&mut list);
                    out.push(Row::Heading { label: crate::tickets::priority_word(priority).to_owned(), count: list.len(), tone: Tone::Plain, hint: None });
                    out.extend(list.into_iter().map(|t| item(t, 0)));
                }
            }
        }
        GroupBy::Deps => {
            let tree = dependency_tree(model, &open);
            if !tree.is_empty() {
                out.push(Row::Heading { label: t!("board.group_deps").into(), count: tree.len(), tone: Tone::Plain, hint: None });
                out.extend(tree.into_iter().map(|(id, depth)| Row::Item { need: Need::Ticket(id), depth }));
            }
        }
        GroupBy::None => {
            let mut list = open.clone();
            by_status(&mut list);
            out.extend(list.into_iter().map(|t| item(t, 0)));
        }
    }
    let done = finished(model, &here, window);
    let total = here.iter().filter(|t| t.status.is_closed()).count();
    if total > 0 {
        out.push(Row::Done { count: done.len(), total, open: open_done, window });
        if open_done {
            out.extend(done.into_iter().map(|t| item(t, 0)));
        }
    }
    out
}

/// Open work as a tree of what waits for what. A ticket waiting on nothing
/// still open is a root; one waiting on several shows under the first.
pub fn dependency_tree(model: &Model, open: &[&Ticket]) -> Vec<(TicketId, usize)> {
    let _ = model;
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
    for t in open {
        if !out.iter().any(|(id, _)| *id == t.id) {
            out.push((t.id, 0));
        }
    }
    out
}

/// The sidebar's groups: a project each (the squad's folder last), its open
/// tickets — what waits for the person first, then work under way, to do,
/// proposed; the latest first in each — and its finished ones apart.
pub struct ProjectGroup<'a> {
    /// `None`: the squad's own folder.
    pub name: Option<String>,
    pub at: Option<usize>,
    pub open: Vec<&'a Ticket>,
    pub done: Vec<&'a Ticket>,
}

pub fn project_groups(model: &Model) -> Vec<ProjectGroup<'_>> {
    let rank = |t: &Ticket| match t.status {
        TicketStatus::Review | TicketStatus::Waiting => 0,
        TicketStatus::InProgress => 1,
        TicketStatus::Todo => 2,
        _ => 3,
    };
    let names = model.project_names();
    let mut out = Vec::new();
    for (at, name) in names.iter().cloned().map(Some).enumerate().map(|(i, n)| (Some(i), n)).chain([(None, None)]) {
        let here: Vec<&Ticket> = model.tickets.iter().filter(|t| model.ticket_project_name(t) == name).collect();
        let latest = |t: &&Ticket| (std::cmp::Reverse(model.since.get(&t.id).copied().unwrap_or(0)), std::cmp::Reverse(t.id));
        let mut open: Vec<&Ticket> = here.iter().copied().filter(|t| !t.status.is_closed()).collect();
        open.sort_by_key(|t| (rank(t), latest(t)));
        let mut done: Vec<&Ticket> = here.iter().copied().filter(|t| t.status.is_closed()).collect();
        done.sort_by_key(latest);
        if name.is_some() || !open.is_empty() || !done.is_empty() {
            out.push(ProjectGroup { name, at, open, done });
        }
    }
    out
}

/// The things listed, in order.
pub fn items(rows: &[Row]) -> Vec<Need> {
    rows.iter().filter_map(|r| if let Row::Item { need, .. } = r { Some(*need) } else { None }).collect()
}

/// The pick: the one chosen if it still shows, else the first listed.
pub fn selected(rows: &[Row], view: &View) -> Option<Need> {
    let list = items(rows);
    view.selected.filter(|n| list.contains(n)).or_else(|| list.first().copied())
}

/// Move the pick by `by` through what is listed, stopping at the ends.
pub fn step(rows: &[Row], view: &mut View, by: isize) {
    let list = items(rows);
    if list.is_empty() {
        return;
    }
    let at = selected(rows, view).and_then(|n| list.iter().position(|x| *x == n)).unwrap_or(0) as isize;
    view.selected = Some(list[(at + by).clamp(0, list.len() as isize - 1) as usize]);
}

/// After acting on `done`, what comes up: the next thing after it as the list
/// was — the rest of its section, then the next section — or, at the end, the
/// one before it. Only what still shows.
pub fn advance(before: &[Need], after: &[Need], done: Need) -> Option<Need> {
    let at = before.iter().position(|n| *n == done)?;
    before[at + 1..].iter().chain(before[..at].iter().rev()).copied().find(|n| *n != done && after.contains(n))
}

/// The ticket a thing listed is about.
pub fn ticket_of(model: &Model, need: Need) -> Option<&Ticket> {
    match need {
        Need::Ticket(id) => model.tickets.iter().find(|t| t.id == id),
        Need::Permission(id) => {
            let ticket = model.approvals.get(id)?.ticket?;
            model.tickets.iter().find(|t| t.id == ticket)
        }
    }
}

/// The person who does a ticket's work: whom C talks to.
pub fn who(t: &Ticket) -> Option<&str> {
    t.assignee.as_deref().or(t.owner.as_deref())
}

/// The steps a waiting ticket's note lists for the person: its checklist
/// lines (`- [ ] …`, `- …`, `1. …`), else nothing.
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

/// What a waiting ticket asks first, in a few words.
pub fn first_need(t: &Ticket) -> Option<String> {
    let note = t.note.as_deref()?;
    equipe_core::board::waiting_needs(note).first().map(|s| (*s).to_owned()).or_else(|| checklist(note).into_iter().next())
}

// ── what accepting does ──

/// What accepting lands, per project: merge and push, merge only, or leave
/// the work on its branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeWay {
    Push,
    Merge,
    Branch,
}

impl MergeWay {
    pub const ALL: [MergeWay; 3] = [MergeWay::Push, MergeWay::Merge, MergeWay::Branch];

    pub fn label(self) -> &'static str {
        match self {
            MergeWay::Push => t!("board.way_push"),
            MergeWay::Merge => t!("board.way_merge"),
            MergeWay::Branch => t!("board.way_branch"),
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            MergeWay::Push => "push",
            MergeWay::Merge => "merge",
            MergeWay::Branch => "branch",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        MergeWay::ALL.into_iter().find(|w| w.code() == code)
    }
}

/// The commands A sends for a ticket that waits for the person: accept (and
/// land its branch `way`), resume, take on a proposal, merge what is not on
/// main — or, the way being "leave on branch", keep it there and stop asking.
pub fn yes_commands(squad: &str, t: &Ticket, way: Option<MergeWay>, note: &str) -> Vec<Command> {
    let squad = squad.to_owned();
    let merge = |way: Option<MergeWay>| match way {
        Some(MergeWay::Push) => Some(Command::Merge { squad: squad.clone(), ticket: t.id, push: true }),
        Some(MergeWay::Merge) => Some(Command::Merge { squad: squad.clone(), ticket: t.id, push: false }),
        Some(MergeWay::Branch) | None => None,
    };
    match t.status {
        TicketStatus::Waiting => vec![Command::Resume { squad, ticket: t.id, note: note.to_owned() }],
        TicketStatus::Done if t.waits_to_merge() && way == Some(MergeWay::Branch) => vec![Command::KeepOnBranch { squad, ticket: t.id }],
        TicketStatus::Done if t.not_on_main() => merge(way).into_iter().collect(),
        TicketStatus::Review if Model::merges(t) => std::iter::once(Command::Accept { squad: squad.clone(), ticket: t.id }).chain(merge(way)).collect(),
        TicketStatus::Done | TicketStatus::Cancelled => Vec::new(),
        _ => vec![Command::Accept { squad, ticket: t.id }],
    }
}

/// Whether A on this ticket lands code, so the project's way is asked for.
pub fn needs_way(t: &Ticket) -> bool {
    (t.status == TicketStatus::Review && Model::merges(t)) || t.not_on_main()
}

/// Why accepted work isn't in main, in the person's words: what equipe
/// recorded (kept by choice, a conflict, a failed merge), else what the
/// project's way (`way`, this app's setting) says. `None` when it is on main.
pub fn off_main_why(t: &Ticket, way: Option<MergeWay>) -> Option<String> {
    if !t.not_on_main() {
        return None;
    }
    Some(match (&t.off_main, way) {
        (Some(OffMain::Kept), _) => t!("peek.why_kept").to_owned(),
        (Some(OffMain::Conflict { detail }), _) => t!("peek.why_conflict", detail = detail),
        (Some(OffMain::Failed { detail }), _) => t!("peek.why_failed", detail = detail),
        (None, Some(MergeWay::Branch)) => t!("peek.why_way_branch").to_owned(),
        (None, None) => t!("peek.why_way_ask").to_owned(),
        (None, Some(_)) => t!("peek.why_accepted_only").to_owned(),
    })
}

// ── taking an action back ──

/// An action held back for a moment: sent when its time comes, unless taken back.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub ticket: TicketId,
    /// What the toast says: `T-42 수락 · web에 병합+push 예정`.
    pub text: String,
    pub commands: Vec<Command>,
    pub due: Instant,
    /// Tickets as they were before an edit shown at once: put back if taken back.
    pub restore: Vec<Ticket>,
}

/// Actions held back, latest last.
#[derive(Debug, Clone, Default)]
pub struct Undo {
    pub pending: Vec<Pending>,
}

impl Undo {
    /// How long an action waits before it is sent.
    pub const DELAY: Duration = Duration::from_secs(5);

    pub fn push(&mut self, ticket: TicketId, text: String, commands: Vec<Command>, now: Instant) {
        self.pending.push(Pending { ticket, text, commands, due: now + Self::DELAY, restore: Vec::new() });
    }

    /// An edit already shown: `restore` is how the tickets were, put back if
    /// it is taken back.
    pub fn push_edit(&mut self, ticket: TicketId, text: String, commands: Vec<Command>, restore: Vec<Ticket>, now: Instant) {
        self.pending.push(Pending { ticket, text, commands, due: now + Self::DELAY, restore });
    }

    /// The actions whose time has come, to send now.
    pub fn take_due(&mut self, now: Instant) -> Vec<Pending> {
        let (due, keep): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.pending).into_iter().partition(|p| p.due <= now);
        self.pending = keep;
        due
    }

    /// Take back the latest action.
    pub fn undo(&mut self) -> Option<Pending> {
        self.pending.pop()
    }

    pub fn latest(&self) -> Option<&Pending> {
        self.pending.last()
    }

    /// Whole seconds left before `p` is sent, rounded up.
    pub fn seconds_left(p: &Pending, now: Instant) -> u64 {
        let left = p.due.saturating_duration_since(now);
        left.as_millis().div_ceil(1000) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::Update;
    use crate::state::Clock;
    use equipe_core::protocol::rpc::Hello;
    use equipe_core::protocol::{Approval, Snapshot};
    use std::path::PathBuf;

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

    fn ids(rows: &[Row]) -> Vec<u32> {
        items(rows).into_iter().filter_map(|n| if let Need::Ticket(id) = n { Some(id) } else { None }).collect()
    }

    fn headings(rows: &[Row]) -> Vec<String> {
        rows.iter().filter_map(|r| if let Row::Heading { label, .. } = r { Some(label.clone()) } else { None }).collect()
    }

    #[test]
    fn all_work_groups_by_status_with_the_finished_folded_in_one_line() {
        let mut tickets = vec![ticket(1, "proposed", "/p", &[]), ticket(2, "todo", "/p", &[]), ticket(3, "in_progress", "/p", &[]), ticket(4, "waiting", "/p", &[]), ticket(5, "review", "/p", &[])];
        tickets.extend((10..18).map(|i| ticket(i, "done", "/p", &[])));
        let m = model(tickets, &["/p"]);
        let view = View { menu: Menu::All, ..Default::default() };
        let rows = all(&m, &view, &Filter::default(), GroupBy::Status);
        assert_eq!(ids(&rows), vec![5, 4, 3, 2, 1], "no cap of five: the finished stay folded");
        assert_eq!(headings(&rows), vec!["검토", "내 차례", "작업 중", "할 일", "제안"]);
        // Nothing finished within the week is known: the line says so and opens to the window.
        assert_eq!(rows.last(), Some(&Row::Done { count: 0, total: 8, open: false, window: DoneWindow::Week }));
        let open = View { done_open: true, window: DoneWindow::All, ..view };
        assert_eq!(ids(&all(&m, &open, &Filter::default(), GroupBy::Status)).len(), 13);
    }

    #[test]
    fn the_finished_window_reaches_back_as_far_as_asked() {
        let mut m = model(vec![ticket(1, "done", "/p", &[]), ticket(2, "done", "/p", &[]), ticket(3, "done", "/p", &[])], &["/p"]);
        m.unix = 100 * 86_400;
        m.since.insert(1, m.unix - 86_400);
        m.since.insert(2, m.unix - 20 * 86_400);
        let view = |window| View { menu: Menu::All, done_open: true, window, ..Default::default() };
        assert_eq!(ids(&all(&m, &view(DoneWindow::Week), &Filter::default(), GroupBy::Status)), vec![1]);
        assert_eq!(ids(&all(&m, &view(DoneWindow::Month), &Filter::default(), GroupBy::Status)), vec![1, 2]);
        assert_eq!(ids(&all(&m, &view(DoneWindow::All), &Filter::default(), GroupBy::Status)), vec![1, 2, 3]);
    }

    #[test]
    fn grouping_by_project_member_and_dependency_order() {
        let tickets = vec![ticket(1, "todo", "/a", &[]), ticket(2, "todo", "/b", &[1]), ticket(3, "in_progress", "/a", &[]), ticket(4, "todo", "/b", &[2])];
        let m = model(tickets, &["/a", "/b"]);
        let view = View { menu: Menu::All, ..Default::default() };
        let rows = all(&m, &view, &Filter::default(), GroupBy::Project);
        assert_eq!(headings(&rows), vec!["a", "b"]);
        assert_eq!(ids(&rows), vec![3, 1, 2, 4], "in progress before to do, within a project");
        let rows = all(&m, &view, &Filter::default(), GroupBy::Deps);
        assert!(rows.contains(&Row::Item { need: Need::Ticket(4), depth: 2 }));
        assert_eq!(headings(&all(&m, &view, &Filter::default(), GroupBy::Member)), vec!["backend"]);
        assert!(headings(&all(&m, &view, &Filter::default(), GroupBy::None)).is_empty());
    }

    #[test]
    fn filter_words_narrow_and_finding_reaches_finished_work() {
        let mut a = ticket(1, "todo", "/a", &[9]);
        a.title = "Login page".into();
        let mut b = ticket(2, "done", "/a", &[]);
        b.body = "fix the login redirect".into();
        let c = ticket(3, "todo", "/b", &[]);
        let m = model(vec![a, b, c, ticket(9, "in_progress", "/a", &[])], &["/a", "/b"]);
        let view = View { menu: Menu::All, ..Default::default() };
        let find = |q: &str| ids(&all(&m, &view, &Filter::parse(q), GroupBy::Status));
        assert_eq!(find("LOGIN"), vec![1, 2], "open first, then the finished, opened");
        assert_eq!(find("t-3"), vec![3]);
        assert_eq!(find("is:blocked"), vec![1]);
        assert_eq!(find("project:b"), vec![3]);
        assert_eq!(find("status:todo project:a"), vec![1]);
        assert_eq!(find("nothing"), Vec::<u32>::new());
        let f = Filter::parse("project:web is:quiet words here not-on-main");
        assert_eq!((f.project, f.quiet, f.not_on_main, f.words.len()), (vec!["web".to_string()], true, true, 2));
        assert_eq!(chips("project:web hello is:live"), vec!["project:web", "is:live"]);
        assert_eq!(toggle_word("project:web", "is:live"), "project:web is:live");
        assert_eq!(toggle_word("project:web is:live", "project:web"), "is:live");
    }

    #[test]
    fn the_inbox_runs_permission_waiting_review_proposed_not_on_main() {
        let mut merged_not = ticket(7, "done", "/p", &[]);
        merged_not.branch = Some("b".into());
        merged_not.merged = Some(false);
        let mut m = model(vec![ticket(5, "review", "/p", &[]), ticket(6, "review", "/p", &[]), ticket(8, "todo", "/p", &[6]), ticket(3, "proposed", "/p", &[]), ticket(4, "waiting", "/p", &[]), merged_not], &["/p"]);
        let ask: Approval = serde_json::from_value(serde_json::json!({"id": 1, "session": 2, "member": "m", "ticket": 9, "tool": "Bash", "action": "ls", "reason": null})).unwrap();
        m.approvals.push(ask);
        let rows = inbox(&m, &Filter::default());
        assert_eq!(headings(&rows), vec!["허락 요청", "내 차례", "검토", "제안", "수락했지만 아직 main에 안 합쳐짐"]);
        let hints: Vec<Option<String>> = rows.iter().filter_map(|r| if let Row::Heading { hint, .. } = r { Some(hint.clone()) } else { None }).collect();
        assert_eq!(hints.iter().filter(|h| h.is_some()).count(), 1, "only the not-merged heading explains itself");
        assert_eq!(hints.last().cloned().flatten().as_deref(), Some("수락한 작업이 멤버 브랜치에만 있어요. 합치면 main에 들어가요."));
        assert_eq!(items(&rows)[0], Need::Permission(1));
        // The review that unblocks something comes first.
        assert_eq!(ids(&rows), vec![4, 6, 5, 3, 7]);
        assert_eq!(m.needs_you_count(), 6);
    }

    #[test]
    fn after_an_action_the_next_in_the_section_then_the_next_section_comes_up() {
        let list = [Need::Ticket(1), Need::Ticket(2), Need::Ticket(3)];
        assert_eq!(advance(&list, &[Need::Ticket(1), Need::Ticket(3)], Need::Ticket(2)), Some(Need::Ticket(3)));
        assert_eq!(advance(&list, &[Need::Ticket(1), Need::Ticket(2)], Need::Ticket(3)), Some(Need::Ticket(2)), "the last: the one before");
        assert_eq!(advance(&list, &[], Need::Ticket(1)), None);
        // Acting hides it from the inbox until the daemon says it changed.
        let mut m = model(vec![ticket(1, "review", "/p", &[]), ticket(2, "review", "/p", &[])], &["/p"]);
        let before = items(&inbox(&m, &Filter::default()));
        m.act_on(1);
        let after = items(&inbox(&m, &Filter::default()));
        assert_eq!(advance(&before, &after, Need::Ticket(1)), Some(Need::Ticket(2)));
        m.unact(1);
        assert_eq!(items(&inbox(&m, &Filter::default())).len(), 2, "taken back: it waits again");
    }

    #[test]
    fn the_pick_stays_while_it_shows_and_moves_with_the_arrows() {
        let m = model((1..=3).map(|i| ticket(i, "todo", "/p", &[])).collect(), &["/p"]);
        let mut view = View { menu: Menu::All, ..Default::default() };
        let rows = rows(&m, &view, &[]);
        assert_eq!(selected(&rows, &view), Some(Need::Ticket(1)));
        for _ in 0..3 {
            step(&rows, &mut view, 1);
        }
        assert_eq!(selected(&rows, &view), Some(Need::Ticket(3)), "stops at the end");
        view.selected = Some(Need::Ticket(99));
        assert_eq!(selected(&rows, &view), Some(Need::Ticket(1)), "one that went away falls back to the first");
    }

    #[test]
    fn yes_lands_work_the_projects_way() {
        let mut review = ticket(1, "review", "/p", &[]);
        review.branch = Some("b".into());
        let accept = Command::Accept { squad: "s".into(), ticket: 1 };
        assert_eq!(yes_commands("s", &review, Some(MergeWay::Push), ""), vec![accept.clone(), Command::Merge { squad: "s".into(), ticket: 1, push: true }]);
        assert_eq!(yes_commands("s", &review, Some(MergeWay::Merge), ""), vec![accept.clone(), Command::Merge { squad: "s".into(), ticket: 1, push: false }]);
        assert_eq!(yes_commands("s", &review, Some(MergeWay::Branch), ""), vec![accept]);
        assert!(needs_way(&review));
        let waiting = ticket(2, "waiting", "/p", &[]);
        assert_eq!(yes_commands("s", &waiting, None, "did it"), vec![Command::Resume { squad: "s".into(), ticket: 2, note: "did it".into() }]);
        assert!(!needs_way(&waiting));
        let mut task = ticket(3, "review", "/p", &[]);
        task.kind = TicketKind::Task;
        task.branch = Some("b".into());
        assert!(!needs_way(&task), "a task's work goes into the squad's folder unasked");
        assert_eq!(MergeWay::parse("merge"), Some(MergeWay::Merge));
    }

    #[test]
    fn work_not_in_main_merges_the_projects_way_or_stays_on_its_branch_for_good() {
        let mut done = ticket(4, "done", "/p", &[]);
        done.branch = Some("equipe/t-4".into());
        done.merged = Some(false);
        assert_eq!(yes_commands("s", &done, Some(MergeWay::Push), ""), vec![Command::Merge { squad: "s".into(), ticket: 4, push: true }]);
        assert_eq!(yes_commands("s", &done, Some(MergeWay::Merge), ""), vec![Command::Merge { squad: "s".into(), ticket: 4, push: false }]);
        assert_eq!(yes_commands("s", &done, Some(MergeWay::Branch), ""), vec![Command::KeepOnBranch { squad: "s".into(), ticket: 4 }], "kept: it stops asking");
        assert!(yes_commands("s", &done, None, "").is_empty(), "no way yet: ask first");
        let kept = Ticket { off_main: Some(OffMain::Kept), ..done.clone() };
        assert!(yes_commands("s", &kept, Some(MergeWay::Branch), "").is_empty(), "kept already");
        assert_eq!(yes_commands("s", &kept, Some(MergeWay::Merge), "").len(), 1, "and it can still be merged");
    }

    #[test]
    fn why_work_isnt_in_main_says_what_is_known() {
        let mut done = ticket(4, "done", "/p", &[]);
        done.branch = Some("equipe/t-4".into());
        done.merged = Some(false);
        assert_eq!(off_main_why(&done, None).as_deref(), Some(t!("peek.why_way_ask")));
        assert_eq!(off_main_why(&done, Some(MergeWay::Branch)).as_deref(), Some(t!("peek.why_way_branch")));
        assert_eq!(off_main_why(&done, Some(MergeWay::Push)).as_deref(), Some(t!("peek.why_accepted_only")));
        let clash = Ticket { off_main: Some(OffMain::Conflict { detail: "README.md".into() }), ..done.clone() };
        let why = off_main_why(&clash, Some(MergeWay::Merge)).unwrap();
        assert!(why.contains("충돌") && why.contains("README.md"), "{why}");
        let failed = Ticket { off_main: Some(OffMain::Failed { detail: "main is checked out".into() }), ..done.clone() };
        assert!(off_main_why(&failed, None).unwrap().contains("main is checked out"));
        let kept = Ticket { off_main: Some(OffMain::Kept), ..done.clone() };
        assert_eq!(off_main_why(&kept, Some(MergeWay::Push)).as_deref(), Some(t!("peek.why_kept")));
        assert_eq!(off_main_why(&Ticket { merged: Some(true), ..done }, None), None, "on main: nothing to explain");
    }

    #[test]
    fn an_action_waits_five_seconds_and_can_be_taken_back() {
        let t0 = Instant::now();
        let mut undo = Undo::default();
        undo.push(1, "T-1".into(), vec![Command::Cancel { squad: "s".into(), ticket: 1 }], t0);
        undo.push(2, "T-2".into(), vec![Command::Cancel { squad: "s".into(), ticket: 2 }], t0 + Duration::from_secs(2));
        assert_eq!(Undo::seconds_left(undo.latest().unwrap(), t0 + Duration::from_secs(2)), 5);
        assert_eq!(Undo::seconds_left(undo.latest().unwrap(), t0 + Duration::from_millis(2500)), 5, "rounded up");
        assert!(undo.take_due(t0 + Duration::from_secs(4)).is_empty());
        let due = undo.take_due(t0 + Duration::from_secs(5));
        assert_eq!(due.iter().map(|p| p.ticket).collect::<Vec<_>>(), vec![1]);
        assert_eq!(undo.undo().map(|p| p.ticket), Some(2), "U takes back the latest");
        assert!(undo.take_due(t0 + Duration::from_secs(60)).is_empty(), "taken back: never sent");
    }

    #[test]
    fn a_waiting_note_becomes_a_checklist_and_its_first_need() {
        let note = "Needs your part:\n- [ ] Create the API key\n- add it to .env.local\n2. restart\n\nthanks";
        assert_eq!(checklist(note), vec!["Create the API key", "add it to .env.local", "restart"]);
        assert!(checklist("nothing listed").is_empty());
        let mut t = ticket(1, "waiting", "/p", &[]);
        t.note = Some(note.into());
        assert_eq!(first_need(&t).as_deref(), Some("Create the API key"));
    }

    #[test]
    fn the_sidebar_groups_tickets_by_project_what_waits_first_the_finished_apart() {
        let mut task = ticket(9, "todo", "/a", &[]);
        task.kind = TicketKind::Task;
        task.project = None;
        let m = model(vec![ticket(1, "todo", "/a", &[]), ticket(2, "review", "/a", &[]), ticket(3, "in_progress", "/a", &[]), ticket(4, "done", "/a", &[]), ticket(5, "todo", "/b", &[]), task], &["/a", "/b"]);
        let groups = project_groups(&m);
        let names: Vec<Option<String>> = groups.iter().map(|g| g.name.clone()).collect();
        assert_eq!(names, [Some("a".to_string()), Some("b".to_string()), None], "the squad's folder last");
        let open: Vec<u32> = groups[0].open.iter().map(|t| t.id).collect();
        assert_eq!(open, [2, 3, 1]);
        assert_eq!(groups[0].done.iter().map(|t| t.id).collect::<Vec<_>>(), [4]);
        assert_eq!(groups[0].at, Some(0));
    }

    #[test]
    fn saved_views_list_with_their_words_and_grouping() {
        let mut m = model(vec![ticket(1, "todo", "/p", &[2]), ticket(2, "in_progress", "/p", &[]), ticket(3, "todo", "/p", &[])], &["/p"]);
        m.unix = 1000;
        let views = builtin_views();
        let view = View { menu: Menu::View(0), ..Default::default() };
        assert_eq!(ids(&rows(&m, &view, &views)), vec![1], "막힌 일");
        assert_eq!(effective(&view, &views).1, GroupBy::Deps);
        assert_eq!(GroupBy::parse("member"), GroupBy::Member);
    }
}
