//! The work screens: 나에게, 작업 중, 모든 일 and the saved views — a list of
//! 32px rows, the picked one peeking at the right (Space), opened as a page
//! (⏎), or its changes in the diff (D). Every need answers to the same keys:
//! A yes, ⇧A how, R in words, N no, C talk, M merge; j/k move and the peek
//! follows; after an action the next thing comes up, and for five seconds U
//! takes it back. The rules are in `crate::board`; this file is what is drawn
//! and what a click or a key does. The list changes only when the daemon says
//! so: every action here is a command.

use equipe_core::t;
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use equipe_core::protocol::{Command, Decision, Ticket, TicketId, TicketStatus};
use gpui_kit::component::input::{Input, InputEvent, InputState, TextareaState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::icons;
use crate::board::{self, DoneWindow, Filter, GroupBy, Menu, MergeWay, Row, SavedView, Tab, Tone, Undo, View};
use crate::diffs::{self, Fetched, Source, Stat};
use crate::state::{Conn, Model, Need, Presence, Verb, local_hm, short_span, status_word};
use crate::theme::{MONO, PROJECT_HUES, Palette, SMALL, look};
use crate::tips::TipExt as _;
use crate::{Screen, Shell};

/// A row of the list.
pub const ROW: f32 = 32.;
const HEADING: f32 = 30.;
/// The peek's width, and the window width under which it lies over the list.
pub const PEEK: f32 = 420.;
pub const PEEK_BESIDE_FROM: f32 = 1100.;

/// A question or a note being written about the pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// R: the words go with the action (changes asked for, what was done, a reply).
    Reply(TicketId),
    /// N on work under way or handed in: cancel it? (y / ⏎)
    Cancel(TicketId),
    /// The first A in a project: what accepting lands there from now on.
    Way { ticket: TicketId, remember: bool },
    /// ⇧A (and M): how to land this one, this once.
    Method(TicketId),
    /// A or N on several chosen tickets, said back once before it goes.
    Bulk { yes: bool },
    /// ⌥V: a name for what is listed now.
    SaveView,
}

/// A small menu open over the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Popover {
    Projects,
    /// The team, under the conversation's title.
    Members,
    Filter,
    Group,
}

type Seen = (Option<String>, String, Option<bool>);

pub struct BoardUi {
    pub view: View,
    pub search: Entity<InputState>,
    pub reply: Entity<TextareaState>,
    pub naming: Entity<InputState>,
    /// The project list's search, and which header opened it.
    pub slice_search: Entity<InputState>,
    pub slice_place: &'static str,
    pub flow: Option<Flow>,
    pub popover: Option<Popover>,
    pub list_scroll: ScrollHandle,
    pub peek_scroll: ScrollHandle,
    pub stats: HashMap<TicketId, (Seen, Stat)>,
    asked: HashMap<TicketId, Seen>,
    pub diff: Option<super::diffview::DiffUi>,
    pub fetched: tokio::sync::mpsc::UnboundedSender<Fetched>,
    /// The text to put in the search box at the next draw (which has a window).
    pending_search: Option<String>,
    pub undo: Undo,
    /// Checklist lines of a waiting ticket the person ticked off (1–9).
    pub ticked: HashMap<TicketId, BTreeSet<usize>>,
    /// `g` was pressed: the next key goes somewhere (i, l, b).
    chord: bool,
    /// The sidebar is away (⌘\), how wide it is, and its edge held to drag.
    pub sidebar_hidden: bool,
    pub sidebar_width: f32,
    pub sidebar_drag: Option<(f32, f32)>,
    /// When the sidebar last came or went, to slide it.
    pub sidebar_shown_at: Option<Instant>,
    /// The sidebar's live rows as last drawn, to ease them in and out.
    pub side_motion: std::cell::RefCell<super::sidebar::SideMotion>,
    /// A small menu out of the sidebar's foot.
    pub popup: Option<super::sidebar::Popup>,
    /// The language picked in settings for every screen; `None`: System.
    pub language: Option<equipe_core::i18n::Lang>,
    /// The language the inputs' placeholders were written in: when the one in
    /// use differs, they are written again ([`BoardUi::relocalize`]).
    pub words: equipe_core::i18n::Lang,
    /// Where the window has been, for back and forward.
    pub history: Vec<super::sidebar::Place>,
    pub history_at: usize,
    /// How far the sidebar's spinners have turned.
    pub spin: f32,
    /// The keys' sheet is open (?).
    pub keys_open: bool,
    /// The window is wide enough for the peek beside the list.
    pub wide: bool,
    /// How wide the list is, as last drawn: narrow lists drop columns.
    pub list_width: f32,
    /// How strong the live dots are now (0–1), stepped by the ticker.
    pub pulse: f32,
    pulse_step: u32,
    _ticker: Option<Task<()>>,
}

impl BoardUi {
    /// The language changed: the words kept in the inputs follow — the rest
    /// of the screen is drawn afresh from `t!` anyway.
    pub fn relocalize(&mut self, window: &mut Window, cx: &mut Context<Shell>) {
        let lang = crate::i18n::lang();
        if self.words == lang {
            return;
        }
        self.words = lang;
        self.search.update(cx, |s, cx| s.set_placeholder(t!("board.search_placeholder"), window, cx));
        self.reply.update(cx, |s, cx| s.set_placeholder(t!("board.reply_placeholder"), window, cx));
        self.naming.update(cx, |s, cx| s.set_placeholder(t!("board.view_name_placeholder"), window, cx));
        self.slice_search.update(cx, |s, cx| s.set_placeholder(t!("header.find_project"), window, cx));
        if let Some(diff) = &self.diff {
            diff.comment.update(cx, |s, cx| s.set_placeholder(t!("diff.comment_placeholder"), window, cx));
        }
    }

    pub fn new(window: &mut Window, cx: &mut Context<Shell>, fetched: tokio::sync::mpsc::UnboundedSender<Fetched>) -> Self {
        Self {
            view: View { peek: true, ..View::default() },
            search: cx.new(|cx| InputState::new(window, cx).placeholder(t!("board.search_placeholder"))),
            reply: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 8).submit_on_enter(false).placeholder(t!("board.reply_placeholder"))),
            naming: cx.new(|cx| InputState::new(window, cx).placeholder(t!("board.view_name_placeholder"))),
            slice_search: cx.new(|cx| InputState::new(window, cx).placeholder(t!("header.find_project"))),
            slice_place: "",
            flow: None,
            popover: None,
            list_scroll: ScrollHandle::new(),
            peek_scroll: ScrollHandle::new(),
            stats: HashMap::new(),
            asked: HashMap::new(),
            diff: None,
            fetched,
            pending_search: None,
            undo: Undo::default(),
            ticked: HashMap::new(),
            chord: false,
            sidebar_hidden: false,
            sidebar_width: super::sidebar::WIDTH,
            sidebar_drag: None,
            sidebar_shown_at: None,
            side_motion: Default::default(),
            popup: None,
            language: equipe_core::i18n::saved(&equipe_providers::credentials::equipe_home()),
            words: crate::i18n::lang(),
            history: Vec::new(),
            history_at: 0,
            spin: 0.,
            keys_open: false,
            wide: true,
            list_width: 900.,
            pulse: 1.,
            pulse_step: 0,
            _ticker: None,
        }
    }

    /// Whether one of the work screens' text boxes has the keyboard.
    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        self.search.read(cx).focus_handle(cx).is_focused(window)
            || self.reply.read(cx).focus_handle(cx).is_focused(window)
            || self.naming.read(cx).focus_handle(cx).is_focused(window)
            || self.slice_search.read(cx).focus_handle(cx).is_focused(window)
            || self.diff.as_ref().is_some_and(|d| d.comment.read(cx).focus_handle(cx).is_focused(window))
    }

    /// The stat of a ticket's branch, while it still looks as it did when asked.
    pub fn stat(&self, t: &Ticket) -> Option<&Stat> {
        self.stats.get(&t.id).filter(|(seen, _)| *seen == diffs::fingerprint(t)).map(|(_, s)| s)
    }
}

// ── what a click or a key does ──

impl Shell {
    pub fn can_act(&self) -> bool {
        matches!(self.model.conn, Conn::Connected { .. })
    }

    /// The views in the sidebar: the built-in ones, then the person's.
    pub fn saved_views(&self) -> Vec<SavedView> {
        let kept = self.remembered.views.iter().map(|v| SavedView { name: v.name.clone(), query: v.query.clone(), group: GroupBy::parse(&v.group), pinned: v.pinned });
        board::builtin_views().into_iter().chain(kept).collect()
    }

    /// What accepting lands in `project`, once the person said.
    pub fn merge_way_of(&self, project: &str) -> Option<MergeWay> {
        self.remembered.merge_way(project).and_then(MergeWay::parse)
    }

    pub fn board_rows(&self) -> Vec<Row> {
        board::rows(&self.model, &self.board.view, &self.saved_views())
    }

    /// The thing picked in the list.
    pub fn picked_need(&self) -> Option<Need> {
        board::selected(&self.board_rows(), &self.board.view)
    }

    /// The ticket picked (a request for permission's own, when it has one).
    pub fn picked(&self) -> Option<Ticket> {
        self.picked_need().and_then(|n| board::ticket_of(&self.model, n)).cloned()
    }

    /// Wire the work screens' text boxes to the window.
    pub fn board_subscriptions(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<Subscription> {
        let search = self.board.search.clone();
        let reply = self.board.reply.clone();
        let naming = self.board.naming.clone();
        let slice = self.board.slice_search.clone();
        vec![
            cx.subscribe_in(&slice, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&search, window, |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    if text != this.board.view.query {
                        this.board.view.query = text;
                        this.board_changed(cx);
                    }
                }
                InputEvent::PressEnter { .. } => {
                    window.blur(cx);
                    cx.notify();
                }
                _ => {}
            }),
            cx.subscribe_in(&reply, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { secondary: true, .. }) {
                    this.board_send_reply(window, cx);
                }
            }),
            cx.subscribe_in(&naming, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.board_save_view(window, cx);
                }
            }),
        ]
    }

    /// The list or the pick changed: the peek starts at its top, what the
    /// rows show of branches is asked of git once, the pick stays in sight.
    pub fn board_changed(&mut self, cx: &mut Context<Self>) {
        self.board.peek_scroll.set_offset(point(px(0.), px(0.)));
        if let Some(flow) = self.board.flow
            && let Some(id) = flow_ticket(flow)
            && self.picked().is_none_or(|t| t.id != id)
            && !matches!(flow, Flow::Way { .. })
        {
            self.board.flow = None;
        }
        self.board_sync(cx);
        self.scroll_to_pick();
        cx.notify();
    }

    fn scroll_to_pick(&self) {
        let rows = self.board_rows();
        let Some(pick) = board::selected(&rows, &self.board.view) else { return };
        if let Some(at) = rows.iter().position(|r| matches!(r, Row::Item { need, .. } if *need == pick)) {
            self.board.list_scroll.scroll_to_item(at);
        }
    }

    /// Bring what the work screens derive up to date: the stats of the
    /// branches listed (each asked once per look of its ticket), the diff
    /// open, a ticket a `T-n` link asked for.
    pub fn board_sync(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.board_ticket.take() {
            self.board_reveal(id);
        }
        if self.screen != Screen::Board {
            return;
        }
        let rows = self.board_rows();
        let mut wanted: Vec<Ticket> = board::items(&rows).into_iter().filter_map(|n| board::ticket_of(&self.model, n)).filter(|t| t.branch.is_some()).take(60).cloned().collect();
        if let Some(t) = self.picked() {
            wanted.push(t);
        }
        for t in wanted {
            let Some(source) = diffs::source(&self.model, &t) else { continue };
            let seen = diffs::fingerprint(&t);
            let have = self.board.stats.get(&t.id).is_some_and(|(s, _)| *s == seen);
            if !have && self.board.asked.get(&t.id) != Some(&seen) {
                self.board.asked.insert(t.id, seen);
                self.fetch(t.id, source, false);
            }
        }
        // A ticket the diff is open on went away or lost its branch.
        if let Some(d) = &self.board.diff
            && !self.model.tickets.iter().any(|t| t.id == d.ticket && t.branch.as_deref() == Some(d.branch.as_str()))
        {
            self.board.diff = None;
        }
        self.ensure_ticker(cx);
        cx.notify();
    }

    pub fn fetch(&self, ticket: TicketId, source: Source, full: bool) {
        let (endpoint, tx) = (self.endpoint.clone(), self.board.fetched.clone());
        std::thread::spawn(move || {
            let text = diffs::run(&endpoint, &source, full);
            let _ = tx.send(Fetched { ticket, branch: source.branch, full, text });
        });
    }

    pub fn board_fetched(&mut self, f: Fetched, cx: &mut Context<Self>) {
        let Some(t) = self.model.tickets.iter().find(|t| t.id == f.ticket) else { return };
        if t.branch.as_deref() != Some(f.branch.as_str()) {
            return;
        }
        if f.full {
            if let Some(d) = self.board.diff.as_mut().filter(|d| d.ticket == f.ticket && d.branch == f.branch) {
                d.loaded(f.text);
            }
        } else if let Ok(text) = f.text {
            let seen = diffs::fingerprint(t);
            self.board.stats.insert(f.ticket, (seen, diffs::parse_stat(&text)));
        }
        cx.notify();
    }

    /// Make ticket `id` the pick, peeking, in the list that holds it: 나에게
    /// when it waits for the person, else 모든 일 (searching for it when even
    /// that would not show it).
    pub fn board_reveal(&mut self, id: TicketId) {
        let need = Need::Ticket(id);
        let in_view = board::items(&self.board_rows()).contains(&need);
        if !in_view {
            let inbox = board::items(&board::inbox(&self.model, &Filter::default())).contains(&need);
            self.board.view.menu = if inbox { Menu::Inbox } else { Menu::All };
            self.board.view.query.clear();
            self.board.pending_search = Some(String::new());
            if !board::items(&self.board_rows()).contains(&need) {
                self.board.view.done_open = true;
                self.board.view.window = DoneWindow::All;
            }
            if !board::items(&self.board_rows()).contains(&need) {
                let q = format!("t-{id}");
                self.board.view.query = q.clone();
                self.board.pending_search = Some(q);
            }
        }
        self.board.view.selected = Some(need);
        self.board.view.peek = true;
        self.board.view.page = None;
        self.board.flow = None;
        self.board.diff = None;
        self.board.peek_scroll.set_offset(point(px(0.), px(0.)));
        self.scroll_to_pick();
    }

    /// Show a menu item: the work screen on it.
    pub fn open_side_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        if self.board.view.menu != menu {
            self.board.view.menu = menu;
            self.board.view.selected = None;
            self.board.view.page = None;
            self.board.view.chosen.clear();
            self.board.flow = None;
            self.board.diff = None;
        }
        self.screen = Screen::Board;
        self.board_changed(cx);
    }

    pub fn board_pick(&mut self, need: Need, cx: &mut Context<Self>) {
        self.board.view.selected = Some(need);
        self.composer.flash = None;
        self.board_changed(cx);
    }

    pub fn board_step(&mut self, by: isize, cx: &mut Context<Self>) {
        let rows = self.board_rows();
        board::step(&rows, &mut self.board.view, by);
        self.composer.flash = None;
        self.board_changed(cx);
    }

    /// ⇧↑↓: choose from where the range started to the pick.
    fn board_range(&mut self, by: isize, cx: &mut Context<Self>) {
        let rows = self.board_rows();
        let list: Vec<TicketId> = board::items(&rows).into_iter().filter_map(|n| if let Need::Ticket(id) = n { Some(id) } else { None }).collect();
        let Some(Need::Ticket(from)) = board::selected(&rows, &self.board.view) else { return };
        let anchor = *self.board.view.anchor.get_or_insert(from);
        board::step(&rows, &mut self.board.view, by);
        let Some(Need::Ticket(to)) = board::selected(&rows, &self.board.view) else { return };
        let (a, b) = (list.iter().position(|x| *x == anchor).unwrap_or(0), list.iter().position(|x| *x == to).unwrap_or(0));
        self.board.view.chosen = list[a.min(b)..=a.max(b)].iter().copied().collect();
        self.board_changed(cx);
    }

    /// `/find words`, from the conversation: 모든 일, narrowed.
    pub fn board_find(&mut self, words: &str, cx: &mut Context<Self>) {
        self.board.view.menu = Menu::All;
        self.board.view.query = words.to_owned();
        self.board.pending_search = Some(words.to_owned());
        self.show(Screen::Board, cx);
        self.board_changed(cx);
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.composer.flash = Some(text.into());
    }

    fn project_of(&self, t: &Ticket) -> String {
        self.model.ticket_project_name(t).unwrap_or_else(|| t!("board.squad_folder").into())
    }

    // ── yes ──

    /// A: yes to what is picked — or to everything chosen.
    pub fn board_yes(&mut self, cx: &mut Context<Self>) {
        if !self.board.view.chosen.is_empty() {
            self.board.flow = Some(Flow::Bulk { yes: true });
            cx.notify();
            return;
        }
        let Some(need) = self.picked_need() else { return };
        match need {
            Need::Permission(id) => {
                let before = board::items(&self.board_rows());
                self.decide_on(id, Decision::Allow, cx);
                self.advance_from(&before, need, cx);
            }
            Need::Ticket(id) => {
                let Some(t) = self.model.tickets.iter().find(|t| t.id == id).cloned() else { return };
                if t.not_on_main() {
                    self.board_merge(id, cx);
                } else if board::needs_way(&t) {
                    match self.merge_way_of(&self.project_of(&t)) {
                        Some(way) => self.land(&t, Some(way), cx),
                        None => {
                            self.board.flow = Some(Flow::Way { ticket: id, remember: true });
                            cx.notify();
                        }
                    }
                } else if t.status == TicketStatus::Waiting {
                    let note = self.done_note(&t, cx);
                    self.land_with(&t, None, &note, cx);
                } else if matches!(t.status, TicketStatus::Review | TicketStatus::Proposed) {
                    self.land(&t, None, cx);
                }
            }
            // Answered on its card, in the peek.
            Need::Questions(_) => {}
        }
    }

    /// ⇧A: how to land the pick, this once (for a request: always allow).
    pub fn board_method(&mut self, cx: &mut Context<Self>) {
        match self.picked_need() {
            Some(Need::Permission(id)) => {
                let before = board::items(&self.board_rows());
                self.decide_on(id, Decision::Always, cx);
                self.advance_from(&before, Need::Permission(id), cx);
            }
            Some(Need::Ticket(id)) => {
                if self.model.tickets.iter().any(|t| t.id == id && board::needs_way(t)) {
                    self.board.flow = Some(Flow::Method(id));
                    cx.notify();
                } else {
                    self.board_yes(cx);
                }
            }
            Some(Need::Questions(_)) | None => {}
        }
    }

    /// Merge accepted work into main the project's way — merge, or merge and
    /// push; with no such way set (asking, or "leave on branch"), ask how.
    pub fn board_merge(&mut self, id: TicketId, cx: &mut Context<Self>) {
        let Some(t) = self.model.tickets.iter().find(|t| t.id == id && t.not_on_main()).cloned() else { return };
        match self.merge_way_of(&self.project_of(&t)) {
            Some(way @ (MergeWay::Merge | MergeWay::Push)) => self.land(&t, Some(way), cx),
            _ => {
                self.board.flow = Some(Flow::Method(id));
                cx.notify();
            }
        }
    }

    /// Leave accepted work on its branch for good: out of 나에게, and the
    /// daemon remembers it was the person's choice.
    pub fn board_keep(&mut self, id: TicketId, cx: &mut Context<Self>) {
        if let Some(t) = self.model.tickets.iter().find(|t| t.id == id && t.waits_to_merge()).cloned() {
            self.land(&t, Some(MergeWay::Branch), cx);
        }
    }

    /// What a waiting ticket's "done" says: the lines ticked off.
    fn done_note(&self, t: &Ticket, cx: &App) -> String {
        // Its card's answers, numbered as asked, and the things ticked off.
        if !t.asks().is_empty() {
            return self.turn_message(t, cx);
        }
        let items = t.note.as_deref().map(board::checklist).unwrap_or_default();
        let ticked = self.board.ticked.get(&t.id);
        items.iter().enumerate().filter(|(i, _)| ticked.is_some_and(|s| s.contains(i))).map(|(_, l)| format!("✓ {l}")).collect::<Vec<_>>().join("\n")
    }

    /// The way chosen for the first A in a project: kept if asked, and used.
    pub fn board_choose_way(&mut self, way: MergeWay, cx: &mut Context<Self>) {
        let Some(flow) = self.board.flow.take() else { return };
        let (id, remember) = match flow {
            Flow::Way { ticket, remember } => (ticket, remember),
            Flow::Method(ticket) => (ticket, false),
            _ => return,
        };
        let Some(t) = self.model.tickets.iter().find(|t| t.id == id).cloned() else { return };
        if remember {
            self.remembered.set_merge_way(&self.project_of(&t), way.code());
            self.remembered.save(&equipe_providers::credentials::equipe_home());
        }
        self.land(&t, Some(way), cx);
    }

    fn land(&mut self, t: &Ticket, way: Option<MergeWay>, cx: &mut Context<Self>) {
        self.land_with(t, way, "", cx);
    }

    /// Hold back the commands that say yes to `t`, and move on.
    fn land_with(&mut self, t: &Ticket, way: Option<MergeWay>, note: &str, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        let commands = board::yes_commands(&self.model.squad, t, way, note);
        if commands.is_empty() {
            return;
        }
        let project = self.project_of(t);
        // Its card in the conversation says it is answered (until taken back).
        if t.status == TicketStatus::Waiting {
            self.model.answer_sent(t.id, note.to_owned());
        }
        let text = match t.status {
            TicketStatus::Waiting => t!("board.toast_resume", id = t.id),
            TicketStatus::Proposed => t!("board.toast_proposal", id = t.id),
            TicketStatus::Done if way == Some(MergeWay::Branch) => t!("board.toast_keep", id = t.id),
            TicketStatus::Done => t!("board.toast_merge", id = t.id, project = project, way = way.unwrap_or(MergeWay::Merge).label()),
            _ if Model::merges(t) => match way {
                Some(MergeWay::Branch) | None => t!("board.toast_accept_branch", id = t.id),
                Some(way) => t!("board.toast_accept_merge", id = t.id, project = project, way = way.label()),
            },
            _ => t!("board.toast_accept", id = t.id),
        };
        self.hold(t.id, text, commands, cx);
    }

    /// Act on a ticket in five seconds, unless taken back (U); out of the
    /// list at once, and on to the next thing.
    fn hold(&mut self, id: TicketId, text: String, commands: Vec<Command>, cx: &mut Context<Self>) {
        let before = board::items(&self.board_rows());
        self.model.act_on(id);
        self.board.undo.push(id, text, commands, Instant::now());
        self.board.flow = None;
        self.board.ticked.remove(&id);
        self.advance_from(&before, Need::Ticket(id), cx);
        self.ensure_ticker(cx);
    }

    /// After acting on `done`: the next thing comes up.
    fn advance_from(&mut self, before: &[Need], done: Need, cx: &mut Context<Self>) {
        let after = board::items(&self.board_rows());
        if self.board.view.selected.is_none_or(|s| s == done) || !after.contains(&self.board.view.selected.unwrap_or(done)) {
            self.board.view.selected = board::advance(before, &after, done).or(after.first().copied());
        }
        if self.board.view.selected.is_none() {
            self.board.view.page = None;
        }
        self.board_changed(cx);
    }

    /// U: take back the latest action held.
    pub fn board_undo(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.board.undo.undo() {
            self.model.unact(p.ticket);
            if self.model.answers.get(&p.ticket).is_some_and(|a| a.here && !a.settled) {
                self.model.answers.remove(&p.ticket);
                self.model.revision += 1;
            }
            self.board.view.selected = Some(Need::Ticket(p.ticket));
            self.say(t!("board.undone", id = p.ticket));
            self.board_changed(cx);
        }
    }

    /// Send what is due; step the live dots. True when the screen changed.
    fn tick(&mut self, now: Instant) -> bool {
        let due = self.board.undo.take_due(now);
        let mut changed = !due.is_empty() || !self.board.undo.pending.is_empty();
        for p in due {
            for command in p.commands {
                self.send_command(command);
            }
        }
        // The live dots pulse and the sidebar's spinners turn while a member works.
        if self.model.tickets.iter().any(|t| self.model.is_live(t)) || self.model.members.iter().any(|m| m.presence == Presence::Working) {
            self.board.pulse_step = self.board.pulse_step.wrapping_add(1);
            self.board.pulse = [1.0, 0.75, 0.45, 0.3, 0.45, 0.75][(self.board.pulse_step / 2) as usize % 6];
            self.board.spin = super::sidebar::spin(self.board.pulse_step);
            changed = true;
        }
        changed
    }

    /// A ticker runs while something is held back or something on screen is live.
    pub fn ensure_ticker(&mut self, cx: &mut Context<Self>) {
        if self.board._ticker.is_some() {
            return;
        }
        self.board._ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let keep = this.update(cx, |this, cx| {
                    let changed = this.tick(Instant::now());
                    if changed {
                        cx.notify();
                    }
                    let live = this.model.tickets.iter().any(|t| this.model.is_live(t)) || this.model.members.iter().any(|m| m.presence == Presence::Working);
                    let keep = !this.board.undo.pending.is_empty() || live;
                    if !keep {
                        this.board._ticker = None;
                    }
                    keep
                });
                if !matches!(keep, Ok(true)) {
                    break;
                }
            }
        }));
    }

    // ── no ──

    /// N: no to what is picked — decline, drop, or (asked once) cancel.
    pub fn board_no(&mut self, cx: &mut Context<Self>) {
        if !self.board.view.chosen.is_empty() {
            self.board.flow = Some(Flow::Bulk { yes: false });
            cx.notify();
            return;
        }
        match self.picked_need() {
            Some(Need::Permission(id)) => {
                let before = board::items(&self.board_rows());
                self.decide_on(id, Decision::Deny, cx);
                self.advance_from(&before, Need::Permission(id), cx);
            }
            Some(Need::Ticket(id)) => {
                let Some(t) = self.model.tickets.iter().find(|t| t.id == id).cloned() else { return };
                match t.status {
                    TicketStatus::Proposed => {
                        let command = Command::Cancel { squad: self.model.squad.clone(), ticket: id };
                        self.hold(id, t!("board.toast_drop", id = id), vec![command], cx);
                    }
                    TicketStatus::Done | TicketStatus::Cancelled => {}
                    _ => {
                        self.board.flow = Some(Flow::Cancel(id));
                        cx.notify();
                    }
                }
            }
            // Closed on its card (✕), where what it closes is in sight.
            Some(Need::Questions(_)) | None => {}
        }
    }

    pub fn board_cancel(&mut self, id: TicketId, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        let command = Command::Cancel { squad: self.model.squad.clone(), ticket: id };
        self.hold(id, t!("board.toast_cancel", id = id), vec![command], cx);
    }

    /// The bulk action, said back and confirmed.
    pub fn board_bulk(&mut self, yes: bool, cx: &mut Context<Self>) {
        self.board.flow = None;
        let chosen: Vec<TicketId> = std::mem::take(&mut self.board.view.chosen).into_iter().collect();
        self.board.view.anchor = None;
        for id in chosen {
            let Some(t) = self.model.tickets.iter().find(|t| t.id == id).cloned() else { continue };
            if yes {
                let set = self.merge_way_of(&self.project_of(&t));
                // Work accepted already is merged in bulk, never kept unasked.
                let way = if t.not_on_main() { set.filter(|w| *w != MergeWay::Branch) } else { board::needs_way(&t).then(|| set.unwrap_or(MergeWay::Branch)) };
                if matches!(t.status, TicketStatus::Review | TicketStatus::Proposed | TicketStatus::Waiting) || t.not_on_main() {
                    self.land(&t, way, cx);
                }
            } else if !t.status.is_closed() {
                let command = Command::Cancel { squad: self.model.squad.clone(), ticket: id };
                self.hold(id, t!("board.toast_cancel", id = id), vec![command], cx);
            }
        }
        cx.notify();
    }

    // ── in words ──

    /// R: the words box at the foot of the peek.
    pub fn board_open_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.picked() else { return };
        if self.picked_need().is_some_and(|n| matches!(n, Need::Permission(_))) {
            return;
        }
        // A wait's card has its own boxes: R goes to the first.
        if t.status == TicketStatus::Waiting && !t.asks().is_empty() {
            let id = t.id;
            self.board.view.peek = true;
            self.turn_focus(id, window, cx);
            return;
        }
        self.board.view.peek = true;
        self.board.flow = Some(Flow::Reply(t.id));
        self.board.reply.update(cx, |s, cx| {
            s.set_value(String::new(), window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// ⌘⏎ in the words box: changes asked for, what was done, a reply to the PM.
    pub fn board_send_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Flow::Reply(id)) = self.board.flow else { return };
        let Some(t) = self.model.tickets.iter().find(|t| t.id == id).cloned() else { return };
        let mut note = self.board.reply.read(cx).value().trim().to_owned();
        let comments = self.board.diff.as_ref().filter(|d| d.ticket == id).map(|d| d.comments_text()).unwrap_or_default();
        if !comments.is_empty() {
            note = if note.is_empty() { comments } else { format!("{note}\n\n{comments}") };
        }
        if !self.can_act() {
            return;
        }
        let squad = self.model.squad.clone();
        match t.status {
            TicketStatus::Review | TicketStatus::Done => {
                if note.is_empty() {
                    return;
                }
                self.board.diff = None;
                self.hold(id, t!("board.toast_send_back", id = id), vec![Command::Reopen { squad, ticket: id, note }], cx);
            }
            TicketStatus::Waiting => {
                let done = self.done_note(&t, cx);
                let note = [done, note].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
                self.hold(id, t!("board.toast_resume", id = id), vec![Command::Resume { squad, ticket: id, note }], cx);
            }
            _ => {
                if note.is_empty() {
                    return;
                }
                // A proposal's answer goes to the PM; work under way, to its member.
                let to = if t.status == TicketStatus::Proposed { self.model.pm() } else { self.model.member_on(id).map(|m| m.session).or(self.model.pm()) };
                if let Some(session) = to {
                    self.send_command(Command::Submit { session, text: format!("T-{id}: {note}") });
                    self.say(t!("board.replied", id = id));
                }
                self.board.flow = None;
            }
        }
        window.blur(cx);
        cx.notify();
    }

    /// C: talk to whoever has the pick, if still on the squad.
    pub fn board_talk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = match self.picked_need() {
            Some(Need::Permission(id)) => self.model.approvals.get(id).map(|a| a.session),
            Some(Need::Questions(id)) => {
                let member = self.model.asked.get(id).map(|s| s.member.clone());
                self.model.members.iter().find(|m| m.presence != Presence::Closed && Some(&m.name) == member.as_ref()).map(|m| m.session)
            }
            _ => {
                let Some(t) = self.picked() else { return };
                let Some(who) = board::who(&t).map(str::to_owned) else { return };
                self.model.members.iter().find(|m| m.presence != Presence::Closed && (m.name == who || m.role == who)).map(|m| m.session)
            }
        };
        if let Some(session) = session {
            self.select_member(session, window, cx);
        }
    }

    /// D: the pick's changes, file by file.
    pub fn board_open_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.picked() else { return };
        let Some(source) = diffs::source(&self.model, &t) else { return };
        self.board.diff = Some(super::diffview::DiffUi::new(t.id, source.branch.clone(), window, cx));
        self.fetch(t.id, source, true);
        cx.notify();
    }

    /// ⌥V: keep what is listed as a view of its own.
    pub fn board_save_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.board.naming.read(cx).value().trim().to_owned();
        if name.is_empty() {
            return;
        }
        let (query, group) = board::effective(&self.board.view, &self.saved_views());
        self.remembered.views.push(crate::target::KeptView { name: name.clone(), query, group: group.code().to_owned(), pinned: true });
        self.remembered.save(&equipe_providers::credentials::equipe_home());
        self.board.flow = None;
        let at = self.saved_views().len() - 1;
        self.board.view.query.clear();
        self.board.pending_search = Some(String::new());
        self.open_side_menu(Menu::View(at), cx);
        self.say(t!("board.view_saved", name = name));
        window.blur(cx);
    }

    /// Take a saved view out (the built-in ones stay).
    pub fn board_forget_view(&mut self, at: usize, cx: &mut Context<Self>) {
        let builtin = board::builtin_views().len();
        if at < builtin || at - builtin >= self.remembered.views.len() {
            return;
        }
        self.remembered.views.remove(at - builtin);
        self.remembered.save(&equipe_providers::credentials::equipe_home());
        if self.board.view.menu == Menu::View(at) {
            self.open_side_menu(Menu::All, cx);
        }
        cx.notify();
    }

    pub fn board_toggle_word(&mut self, word: &str, window: &mut Window, cx: &mut Context<Self>) {
        let q = board::toggle_word(&self.board.view.query, word);
        self.board.view.query = q.clone();
        self.board.search.update(cx, |s, cx| s.set_value(q, window, cx));
        self.board_changed(cx);
    }

    pub fn board_set_group(&mut self, group: GroupBy, cx: &mut Context<Self>) {
        self.board.view.group = group;
        self.board.popover = None;
        if let Menu::View(_) = self.board.view.menu {
            self.board.view.menu = Menu::All;
        }
        self.board_changed(cx);
    }

    /// Esc: a menu, a question or a note first, then the diff, the page, the
    /// choice, the search, the peek.
    pub fn board_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.board.keys_open {
            self.board.keys_open = false;
        } else if self.board.popover.is_some() {
            self.board.popover = None;
        } else if self.board.flow.is_some() {
            self.board.flow = None;
            window.blur(cx);
        } else if let Some(d) = self.board.diff.as_mut() {
            if !d.escape() {
                self.board.diff = None;
            }
        } else if self.board.view.page.is_some() {
            self.board.view.page = None;
        } else if !self.board.view.chosen.is_empty() {
            self.board.view.chosen.clear();
            self.board.view.anchor = None;
        } else if !self.board.view.query.is_empty() {
            self.board.view.query.clear();
            self.board.search.update(cx, |s, cx| s.set_value(String::new(), window, cx));
            window.blur(cx);
            self.board_changed(cx);
        } else if self.board.view.peek {
            self.board.view.peek = false;
        } else {
            window.blur(cx);
        }
        cx.notify();
    }

    /// A key on a work screen. True when it was taken.
    pub fn on_board_key(&mut self, key: &str, m: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.screen != Screen::Board {
            return false;
        }
        let plain = !(m.control || m.alt || m.platform || m.shift || m.function);
        if key == "escape" && plain {
            self.board_escape(window, cx);
            return true;
        }
        if self.board.typing(window, cx) {
            return false;
        }
        // The diff has keys of its own.
        if self.board.diff.is_some() && self.on_diff_key(key, m, window, cx) {
            return true;
        }
        let shift_only = m.shift && !(m.control || m.alt || m.platform || m.function);
        // Chosen keys of an open question.
        match (self.board.flow, key) {
            (Some(Flow::Way { .. } | Flow::Method(_)), "1" | "2" | "3") => {
                let way = MergeWay::ALL[key.parse::<usize>().unwrap_or(1) - 1];
                self.board_choose_way(way, cx);
                return true;
            }
            (Some(Flow::Way { ticket, remember }), "space") => {
                self.board.flow = Some(Flow::Way { ticket, remember: !remember });
                cx.notify();
                return true;
            }
            (Some(Flow::Cancel(id)), "y" | "enter") => {
                self.board_cancel(id, cx);
                return true;
            }
            (Some(Flow::Cancel(_)), "n") => {
                self.board.flow = None;
                cx.notify();
                return true;
            }
            (Some(Flow::Bulk { yes }), "y" | "enter") => {
                self.board_bulk(yes, cx);
                return true;
            }
            (Some(Flow::Bulk { .. }), "n") => {
                self.board.flow = None;
                cx.notify();
                return true;
            }
            _ => {}
        }
        if m.platform && key == "a" && !m.shift {
            let list: BTreeSet<TicketId> = board::items(&self.board_rows()).into_iter().filter_map(|n| if let Need::Ticket(id) = n { Some(id) } else { None }).collect();
            self.board.view.chosen = list;
            cx.notify();
            return true;
        }
        if m.alt && key == "v" {
            self.board.flow = Some(Flow::SaveView);
            self.board.naming.update(cx, |s, cx| {
                s.set_value(String::new(), window, cx);
                s.focus(window, cx);
            });
            cx.notify();
            return true;
        }
        if shift_only {
            match key {
                "a" => self.board_method(cx),
                "up" | "k" => self.board_range(-1, cx),
                "down" | "j" => self.board_range(1, cx),
                "/" | "?" => {
                    self.board.keys_open = !self.board.keys_open;
                    cx.notify();
                }
                _ => return false,
            }
            return true;
        }
        if !plain {
            return key == "?";
        }
        if std::mem::take(&mut self.board.chord) {
            let menu = match key {
                "i" => Some(Menu::Inbox),
                "l" => Some(Menu::Working),
                "b" => Some(Menu::All),
                _ => None,
            };
            if let Some(menu) = menu {
                self.open_side_menu(menu, cx);
                return true;
            }
        }
        // A waiting ticket's checklist: 1–9 tick its lines off.
        if let (Some(d @ 1..=9), Some(t)) = (key.parse::<usize>().ok(), self.picked().filter(|t| t.status == TicketStatus::Waiting)) {
            let set = self.board.ticked.entry(t.id).or_default();
            if !set.remove(&(d - 1)) {
                set.insert(d - 1);
            }
            cx.notify();
            return true;
        }
        match key {
            "up" | "k" => self.board_step(-1, cx),
            "down" | "j" => self.board_step(1, cx),
            "space" => {
                self.board.view.peek = !self.board.view.peek;
                cx.notify();
            }
            "enter" => {
                if self.picked_need().is_some_and(|n| matches!(n, Need::Ticket(_))) {
                    self.board.view.page = Some(Tab::Overview);
                    self.board.view.peek = true;
                    cx.notify();
                }
            }
            "/" => {
                self.board.search.update(cx, |s, cx| s.focus(window, cx));
                cx.notify();
            }
            "?" => {
                self.board.keys_open = !self.board.keys_open;
                cx.notify();
            }
            "f" => {
                self.board.popover = if self.board.popover == Some(Popover::Filter) { None } else { Some(Popover::Filter) };
                cx.notify();
            }
            "g" => self.board.chord = true,
            "x" => {
                if let Some(Need::Ticket(id)) = self.picked_need() {
                    if !self.board.view.chosen.remove(&id) {
                        self.board.view.chosen.insert(id);
                    }
                    self.board.view.anchor = Some(id);
                    cx.notify();
                }
            }
            "u" => self.board_undo(cx),
            "a" | "y" => self.board_yes(cx),
            "n" => self.board_no(cx),
            "r" => self.board_open_reply(window, cx),
            "c" => self.board_talk(window, cx),
            "d" => self.board_open_diff(window, cx),
            "m" => {
                if let Some(id) = self.picked().filter(Ticket::not_on_main).map(|t| t.id) {
                    self.board_merge(id, cx);
                }
            }
            _ => return false,
        }
        true
    }

    /// Dev tool (`snapshot.rs`): write in the open words box.
    #[cfg(feature = "snapshot")]
    pub fn board_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        match self.board.flow {
            Some(Flow::SaveView) => self.board.naming.update(cx, |s, cx| s.set_value(text.to_owned(), window, cx)),
            _ => self.board.reply.update(cx, |s, cx| s.set_value(text.to_owned(), window, cx)),
        }
        cx.notify();
    }

    /// Dev tool: the open question's main button.
    #[cfg(feature = "snapshot")]
    pub fn board_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.board.flow {
            Some(Flow::Reply(_)) => self.board_send_reply(window, cx),
            Some(Flow::SaveView) => self.board_save_view(window, cx),
            _ => {}
        }
    }

    /// Called from the window's draw: what needs a window and was asked for without one.
    pub fn board_prepare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.board.pending_search.take() {
            self.board.search.update(cx, |s, cx| s.set_value(text, window, cx));
        }
        let width = f32::from(window.viewport_size().width);
        self.board.wide = width >= PEEK_BESIDE_FROM;
        let peek = self.board.view.peek && self.board.wide && self.board.view.page.is_none();
        let side = if self.board.sidebar_hidden { 0. } else { self.board.sidebar_width };
        self.board.list_width = width - side - if peek { PEEK } else { 0. } - 16.;
        self.note_place();
    }
}

fn flow_ticket(flow: Flow) -> Option<TicketId> {
    match flow {
        Flow::Reply(t) | Flow::Cancel(t) | Flow::Method(t) => Some(t),
        Flow::Way { ticket, .. } => Some(ticket),
        Flow::Bulk { .. } | Flow::SaveView => None,
    }
}

// ── drawing ──

pub fn btn(p: Palette, id: impl Into<ElementId>, label: impl Into<SharedString>, key: Option<&'static str>, primary: bool, enabled: bool) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h(px(28.))
        .px_3()
        .gap_2()
        .items_center()
        .flex_none()
        .rounded(px(6.))
        .text_size(px(SMALL))
        .when(enabled, |b| b.cursor_pointer())
        .when(!enabled, |b| b.opacity(0.4))
        .when(primary, |b| b.bg(p.primary).text_color(p.on_primary).font_weight(FontWeight::SEMIBOLD))
        .when(!primary, |b| b.border_1().border_color(p.hair).when(enabled, |b| b.hover(move |s| s.bg(p.wash))))
        .child(label.into())
        .when_some(key, |b, key| b.child(div().text_size(px(11.)).opacity(0.6).child(key)))
}

pub fn kbd(p: Palette, key: &'static str) -> Div {
    div().px(px(5.)).h(px(18.)).flex().items_center().rounded(px(4.)).border_1().border_color(p.hair).text_size(px(11.)).text_color(p.muted).child(key)
}

pub fn hue_of(p: Palette, model: &Model, t: &Ticket) -> Option<Rgba> {
    let name = model.ticket_project_name(t)?;
    model.project_names().iter().position(|q| *q == name).map(|ix| p.member(PROJECT_HUES[ix % PROJECT_HUES.len()]))
}

pub fn member_color(p: Palette, model: &Model, who: &str) -> Rgba {
    model.members.iter().find(|m| m.name == who || m.role == who).map_or(p.text2, |m| p.member(look(&m.role, &m.name).1))
}

/// A project's dot.
pub fn dot(color: Rgba, size: f32) -> Div {
    div().size(px(size)).flex_none().rounded_full().bg(color)
}

pub fn menu_title(menu: Menu, views: &[SavedView]) -> String {
    match menu {
        Menu::Inbox => t!("board.menu_inbox").into(),
        Menu::Working => t!("board.menu_working").into(),
        Menu::All => t!("board.menu_all").into(),
        Menu::View(at) => views.get(at).map_or_else(String::new, |v| v.name.clone()),
    }
}

/// "작업 중 4 · 다음 완료 ~14:30 · 조용함 1" (without the count: "다음 완료 ~14:30 · 조용함 1").
pub fn working_summary(model: &Model, count: bool) -> String {
    let working: Vec<&Ticket> = model.tickets.iter().filter(|t| t.status == TicketStatus::InProgress && model.in_focus(t)).collect();
    let mut parts = if count { vec![format!("{} {}", status_word(TicketStatus::InProgress), working.len())] } else { Vec::new() };
    if let Some(eta) = working.iter().filter_map(|t| model.eta(t)).filter(|e| e.at > model.unix).min_by_key(|e| e.at) {
        parts.push(t!("board.next_done", at = eta_text(eta.at, eta.guessed)));
    }
    let quiet = working.iter().filter(|t| model.quiet_for(t).is_some()).count();
    if quiet > 0 {
        parts.push(format!("{} {quiet}", t!("board.flag_quiet")));
    }
    parts.join(" · ")
}

fn eta_text(at: u64, guessed: bool) -> String {
    if guessed { format!("~{}", local_hm(at)) } else { local_hm(at) }
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    if let Some(diff) = &shell.board.diff {
        return v_flex().flex_1().min_h_0().child(super::diffview::render(shell, diff, cx)).children(toast(shell, cx)).into_any_element();
    }
    let view = &shell.board.view;
    let page = view.page.zip(shell.picked());
    let body: AnyElement = match page {
        Some((tab, t)) => super::peek::page(shell, &t, tab, cx).into_any_element(),
        None => {
            let rows = shell.board_rows();
            let pick = board::selected(&rows, view);
            let list = list(shell, &rows, pick, cx);
            let peek = pick.filter(|_| view.peek).map(|need| super::peek::panel(shell, need, cx));
            match (peek, shell.board.wide) {
                (Some(peek), true) => h_flex().flex_1().min_h_0().items_start().child(list).child(div().w(px(PEEK)).h_full().flex_none().border_l_1().border_color(p.hair).child(peek)).into_any_element(),
                (Some(peek), false) => div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(list)
                    .child(div().absolute().top_0().right_0().bottom_0().w(px(PEEK)).bg(p.bg).border_l_1().border_color(p.hair).shadow_lg().child(peek))
                    .into_any_element(),
                (None, _) => list.into_any_element(),
            }
        }
    };
    v_flex()
        .id("board")
        .flex_1()
        .min_h_0()
        .relative()
        .child(toolbar(shell, cx))
        .children(super::peek::save_view_bar(shell, cx))
        .child(body)
        .children(bulk_bar(shell, cx))
        .children(toast(shell, cx))
        .children(shell.board.keys_open.then(|| keys_sheet(p, cx)))
        .into_any_element()
}

/// The bar over the list: what it is, the project slice, the search, the filters.
fn toolbar(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let view = &shell.board.view;
    let views = shell.saved_views();
    let rows = shell.board_rows();
    let count = board::items(&rows).len();
    let title = menu_title(view.menu, &views);
    let summary = match view.menu {
        Menu::Working => Some(working_summary(&shell.model, false)),
        _ => None,
    };
    let grouping = matches!(view.menu, Menu::All | Menu::View(_));
    let (_, group) = board::effective(view, &views);
    let chips: Vec<String> = board::chips(&view.query);
    let saved_chips: Vec<String> = match view.menu {
        Menu::View(at) => views.get(at).map(|v| board::chips(&v.query)).unwrap_or_default(),
        _ => Vec::new(),
    };
    let chip = |word: String, removable: bool, cx: &mut Context<Shell>| {
        let w = word.clone();
        h_flex()
            .id(SharedString::from(format!("chip-{word}")))
            .h(px(22.))
            .px_2()
            .gap_1()
            .items_center()
            .rounded(px(11.))
            .bg(p.wash)
            .text_size(px(12.))
            .text_color(p.text2)
            .child(word)
            .when(removable, |c| {
                c.cursor_pointer().child(div().text_color(p.muted).child("×")).on_click(cx.listener(move |this, _, window, cx| this.board_toggle_word(&w, window, cx)))
            })
    };
    let has_chips = !chips.is_empty() || !saved_chips.is_empty();
    // The title row is the title bar: its height and hairline the sidebar's
    // strip's, the chips (when any) a row of their own under it.
    v_flex()
        .flex_none()
        .when(has_chips, |v| v.border_b_1().border_color(p.hair))
        .child(
            h_flex()
                .h(px(super::header::HEIGHT))
                .pr_4()
                .pl(px(shell.header_inset(16., cx)))
                .gap_3()
                .items_center()
                .border_b_1()
                .border_color(p.hair)
                .window_control_area(WindowControlArea::Drag)
                .child(h_flex().gap_2().items_baseline().flex_none().child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(title)).child(div().text_size(px(SMALL)).text_color(p.muted).child(count.to_string())))
                .child(super::header::slice_control(shell, "board", cx))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .max_w(px(360.))
                        .h(px(28.))
                        .px_2()
                        .gap_2()
                        .items_center()
                        .rounded(px(6.))
                        .bg(p.code_bg)
                        .border_1()
                        .border_color(p.hair)
                        .text_size(px(SMALL))
                        .child(div().flex_1().min_w_0().child(Input::new(&shell.board.search).appearance(false).bordered(false)))
                        .child(kbd(p, "/")),
                )
                .child(
                    h_flex()
                        .id("filter")
                        .h(px(28.))
                        .px_2()
                        .gap_1()
                        .items_center()
                        .rounded(px(6.))
                        .text_size(px(SMALL))
                        .text_color(p.text2)
                        .cursor_pointer()
                        .hover(move |s| s.bg(p.wash))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.board.popover = if this.board.popover == Some(Popover::Filter) { None } else { Some(Popover::Filter) };
                            cx.notify();
                        }))
                        .child(t!("board.filter"))
                        .child(kbd(p, "F")),
                )
                .child(div().flex_1())
                .children(summary.map(|s| div().flex_none().text_size(px(SMALL)).text_color(p.muted).child(s)))
                .when(grouping, |bar| {
                    bar.child(
                        h_flex()
                            .id("group")
                            .h(px(28.))
                            .px_2()
                            .gap_1()
                            .items_center()
                            .flex_none()
                            .rounded(px(6.))
                            .text_size(px(SMALL))
                            .text_color(p.text2)
                            .cursor_pointer()
                            .hover(move |s| s.bg(p.wash))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.board.popover = if this.board.popover == Some(Popover::Group) { None } else { Some(Popover::Group) };
                                cx.notify();
                            }))
                            .child(div().text_color(p.muted).child(t!("board.group_by")))
                            .child(group.label())
                            .child(super::sidebar::icon(IconName::ChevronDown, 12., p.muted)),
                    )
                })
                .children(popover(shell, cx)),
        )
        .when(has_chips, |bar| {
            let mut row = h_flex().px_4().py_2().gap_1().flex_wrap();
            for word in saved_chips {
                row = row.child(chip(word, false, cx));
            }
            for word in chips {
                row = row.child(chip(word, true, cx));
            }
            bar.child(row)
        })
}

/// A small menu over the bar: projects, filter words, grouping.
fn popover(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let which = shell.board.popover?;
    let item = |id: SharedString, label: String, on: bool| {
        h_flex()
            .id(id)
            .h(px(28.))
            .px_3()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(SMALL))
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .child(div().w(px(12.)).text_color(p.accent).child(if on { "✓" } else { "" }))
            .child(label)
    };
    let body: Vec<AnyElement> = match which {
        Popover::Projects | Popover::Members => return None,
        Popover::Group => {
            let mut rows: Vec<AnyElement> = GroupBy::ALL
                .into_iter()
                .map(|g| {
                    let on = board::effective(&shell.board.view, &shell.saved_views()).1 == g;
                    item(SharedString::from(format!("group-{}", g.code())), g.label().to_owned(), on).on_click(cx.listener(move |this, _, _, cx| this.board_set_group(g, cx))).into_any_element()
                })
                .collect();
            // The views kept, reachable from here now that the sidebar lists live work only.
            rows.push(div().h(px(1.)).my_1().bg(p.hair).into_any_element());
            rows.push(div().px_3().pt_1().pb(px(2.)).text_size(px(11.)).text_color(p.muted).child(t!("sidebar.views")).into_any_element());
            for (at, v) in shell.saved_views().into_iter().enumerate() {
                let on = shell.board.view.menu == Menu::View(at);
                rows.push(
                    item(SharedString::from(format!("view-{at}")), v.name, on)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.board.popover = None;
                            this.open_side_menu(if this.board.view.menu == Menu::View(at) { Menu::All } else { Menu::View(at) }, cx);
                        }))
                        .into_any_element(),
                );
            }
            rows
        }
        Popover::Filter => {
            let mut words: Vec<String> = shell.model.project_names().into_iter().map(|n| format!("project:{n}")).collect();
            let mut members: Vec<String> = shell.model.members.iter().filter(|m| !Model::is_pm(m)).map(|m| format!("member:{}", m.name)).collect();
            members.dedup();
            words.extend(members);
            words.extend(["status:review", "status:waiting", "status:in_progress", "status:todo", "status:proposed", "status:done"].map(String::from));
            words.extend(board::FILTER_WORDS.map(String::from));
            words.extend(["kind:task", "kind:action"].map(String::from));
            let have = board::chips(&shell.board.view.query);
            words
                .into_iter()
                .map(|w| {
                    let on = have.contains(&w);
                    let word = w.clone();
                    item(SharedString::from(format!("word-{w}")), w, on).on_click(cx.listener(move |this, _, window, cx| this.board_toggle_word(&word, window, cx))).into_any_element()
                })
                .collect()
        }
    };
    let right = which == Popover::Group;
    Some(
        deferred(
            v_flex()
                .id("popover")
                .absolute()
                .top(px(44.))
                .when(right, |d| d.right(px(16.)))
                .when(!right, |d| d.left(px(420.)))
                .w(px(240.))
                .max_h(px(420.))
                .overflow_y_scroll()
                .p_1()
                .rounded(px(8.))
                .bg(p.raised)
                .border_1()
                .border_color(p.hair)
                .shadow_lg()
                .occlude()
                .children(body),
        )
        .into_any_element(),
    )
}

fn list(shell: &Shell, rows: &[Row], pick: Option<Need>, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let empty = board::items(rows).is_empty() && !rows.iter().any(|r| matches!(r, Row::Done { .. }));
    v_flex()
        .id("board-list")
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .track_scroll(&shell.board.list_scroll)
        .px_2()
        .pt_1()
        .pb_6()
        .children(rows.iter().enumerate().map(|(ix, row)| list_row(shell, ix, row, pick, cx)))
        .when(empty, |l| l.child(empty_note(shell, p, cx)))
}

fn empty_note(shell: &Shell, p: Palette, cx: &mut Context<Shell>) -> AnyElement {
    let view = &shell.board.view;
    let (title, text, clear) = if !Filter::parse(&view.query).is_empty() {
        (t!("board.empty_filtered_title").to_owned(), t!("board.empty_filtered").to_owned(), true)
    } else {
        match view.menu {
            Menu::Inbox => (t!("board.empty_inbox_title").to_owned(), working_summary(&shell.model, true), false),
            Menu::Working => (t!("board.empty_working_title").to_owned(), t!("board.empty_working").to_owned(), false),
            _ if shell.model.focus.is_some() && !shell.model.tickets.is_empty() => (t!("board.empty_focus_title").to_owned(), t!("board.empty_focus", focus = shell.model.focus.clone().unwrap_or_default()), false),
            _ => (t!("board.empty_title").to_owned(), t!("board.empty").to_owned(), false),
        }
    };
    v_flex()
        .pt(px(96.))
        .items_center()
        .gap_2()
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title))
        .child(div().max_w(px(420.)).text_center().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(text))
        .when(clear, |c| {
            c.child(btn(p, "clear-filter", t!("board.clear_filter"), Some("Esc"), false, true).on_click(cx.listener(|this, _, window, cx| {
                this.board.view.query.clear();
                this.board.search.update(cx, |s, cx| s.set_value(String::new(), window, cx));
                this.board_changed(cx);
            })))
        })
        .into_any_element()
}

fn list_row(shell: &Shell, ix: usize, row: &Row, pick: Option<Need>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    match row {
        Row::Heading { label, count, tone, hint } => {
            let color = match tone {
                Tone::ForYou => p.accent,
                _ => p.muted,
            };
            let heading = h_flex()
                .h(px(HEADING))
                .mt(px(if ix == 0 { 4. } else { 10. }))
                .px_3()
                .gap_2()
                .items_center()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .when_some(match tone {
                    Tone::Project(Some(i)) => Some(p.member(PROJECT_HUES[*i % PROJECT_HUES.len()])),
                    _ => None,
                }, |h, c| h.child(dot(c, 7.)))
                .child(label.clone())
                .child(div().font_weight(FontWeight::NORMAL).text_color(p.muted).child(count.to_string()));
            match hint {
                // What the section means, in one quiet line under its name.
                Some(hint) => v_flex()
                    .child(heading)
                    .child(div().px_3().pb_1().mt(px(-6.)).text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(hint.clone()))
                    .into_any_element(),
                None => heading.into_any_element(),
            }
        }
        Row::Done { count, total, open, window } => {
            let (open, window) = (*open, *window);
            h_flex()
                .id("done-group")
                .h(px(HEADING))
                .mt(px(10.))
                .px_3()
                .gap_2()
                .items_center()
                .rounded(px(6.))
                .text_size(px(12.))
                .text_color(p.muted)
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.board.view.done_open = !open;
                    this.board_changed(cx);
                }))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(status_word(TicketStatus::Done)))
                .child("·")
                .child(
                    div()
                        .id("done-window")
                        .px_1()
                        .rounded(px(4.))
                        .hover(move |s| s.bg(p.hair))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.board.view.window = window.next();
                            this.board.view.done_open = true;
                            this.board_changed(cx);
                        }))
                        .child(window.label()),
                )
                .child(count.to_string())
                .when(window != DoneWindow::All && total > count, |r| r.child(div().opacity(0.7).child(t!("board.done_total", n = total))))
                .child(super::sidebar::icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, 12., p.muted))
                .into_any_element()
        }
        Row::Item { need, depth } => item_row(shell, *need, *depth, pick == Some(*need), cx),
    }
}

/// What a row says at its right, column by column.
struct Meta {
    /// Each chip: its words, its color, and what it means on the pointer.
    flags: Vec<(String, Rgba, Option<String>)>,
    activity: Option<(String, Rgba)>,
    eta: Option<(String, Rgba)>,
    time: Option<String>,
}

fn verb_word(verb: Verb) -> &'static str {
    match verb {
        Verb::Thinking => t!("sidebar.thinking"),
        Verb::Reading => t!("sidebar.reading"),
        Verb::Editing => t!("sidebar.editing"),
        Verb::Running => t!("sidebar.running"),
        Verb::Searching => t!("sidebar.searching"),
        Verb::Writing => t!("sidebar.writing"),
        Verb::Working => t!("sidebar.working"),
    }
}

/// The last part of a path or the start of a command: what a step was about.
pub fn short_target(target: &str) -> String {
    let first = target.lines().next().unwrap_or("").trim();
    let leaf = if first.contains('/') && !first.contains(' ') { first.rsplit('/').next().unwrap_or(first) } else { first };
    leaf.chars().take(28).collect()
}

/// What a member is doing now: `수정 중 checkout.tsx`.
pub fn live_activity(model: &Model, t: &Ticket) -> Option<String> {
    let m = model.member_on(t.id)?;
    if m.presence != Presence::Working {
        return None;
    }
    let target = m.target.as_deref().filter(|_| matches!(m.verb, Verb::Reading | Verb::Editing | Verb::Running | Verb::Searching)).map(short_target);
    Some(match target {
        Some(target) if !target.is_empty() => format!("{} {target}", verb_word(m.verb)),
        _ => verb_word(m.verb).to_owned(),
    })
}

fn meta(shell: &Shell, t: &Ticket) -> Meta {
    let p = shell.palette;
    let model = &shell.model;
    let mut flags = Vec::new();
    if let Some(dep) = model.blocked_by(t).filter(|_| matches!(t.status, TicketStatus::Todo | TicketStatus::Proposed)) {
        flags.push((format!("⛓T-{dep}"), p.orange, None));
    }
    let unblocks = model.unblocks(t.id).len();
    if unblocks > 0 && !t.status.is_closed() {
        flags.push((format!("→{unblocks}"), p.muted, None));
    }
    if t.not_on_main() {
        let branch = t.branch.clone().unwrap_or_default();
        flags.push(if t.waits_to_merge() {
            (t!("state.chip_not_on_main").to_owned(), p.accent, Some(t!("state.chip_not_on_main_tip", branch = branch)))
        } else {
            (t!("state.chip_kept").to_owned(), p.muted, Some(t!("state.chip_kept_tip", branch = branch)))
        });
    }
    if let Some(quiet) = model.quiet_for(t) {
        flags.push((format!("{} {}", t!("board.flag_quiet"), short_span(quiet)), p.amber, None));
    }
    if model.failed(t) {
        flags.push((t!("board.flag_error").to_owned(), p.red, None));
    }
    let activity = match t.status {
        TicketStatus::InProgress => live_activity(model, t).map(|a| (a, p.text2)),
        TicketStatus::Waiting => board::first_need(t).map(|n| (n, p.accent)),
        TicketStatus::Proposed => Some((t!("board.proposed_by", who = t.created_by), p.muted)),
        TicketStatus::Review | TicketStatus::Done if t.kind.in_home() => Some((t!("board.kind_label", kind = equipe_core::i18n::label::kind(crate::i18n::lang(), t.kind)), p.muted)),
        TicketStatus::Review | TicketStatus::Done => shell.board.stat(t).map(|s| (s.short(), p.muted)),
        _ => None,
    };
    let eta = model.eta(t).map(|e| {
        if e.at <= model.unix {
            (t!("board.eta_passed", span = short_span(model.unix - e.at)), p.amber)
        } else {
            (eta_text(e.at, e.guessed), p.muted)
        }
    });
    let time = model.in_state_for(t).map(short_span);
    Meta { flags, activity, eta, time }
}

fn item_row(shell: &Shell, need: Need, depth: usize, picked: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let group = SharedString::from(format!("row-{need:?}"));
    let base = |id: SharedString, cx: &mut Context<Shell>| {
        h_flex()
            .id(id)
            .group(group.clone())
            .relative()
            .h(px(ROW))
            .flex_none()
            .pr_3()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(13.))
            .cursor_pointer()
            .when(picked, |r| r.bg(p.wash))
            .when(!picked, |r| r.hover(move |s| s.bg(p.wash)))
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                if e.click_count() >= 2 {
                    this.board.view.page = Some(Tab::Overview);
                }
                this.board_pick(need, cx);
            }))
    };
    // One grid for every row of every section: check · mark · T-n · project ·
    // title · flags · detail · when · time.
    let compact = shell.board.list_width < 720.;
    let show_project = model.focus.is_none() && model.projects.len() > 1;
    let (flags_w, detail_w, eta_w) = if compact { (72., 120., 44.) } else { (120., 176., 56.) };
    let project_col = |t: Option<&Ticket>| {
        let color = t.and_then(|t| hue_of(p, model, t));
        let name = t.map(|t| model.ticket_project_name(t).unwrap_or_else(|| t!("board.squad_folder").into()));
        h_flex()
            .w(px(if compact { 10. } else { 84. }))
            .flex_none()
            .gap(px(6.))
            .items_center()
            .children(t.map(|_| dot(color.unwrap_or(p.muted), 6.)))
            .when(!compact, |d| d.child(div().min_w_0().truncate().text_size(px(12.)).text_color(p.muted).child(name.unwrap_or_default())))
    };
    let cell = |w: f32| div().w(px(w)).flex_none().truncate().text_align(TextAlign::Right).text_size(px(12.));
    match need {
        Need::Permission(id) => {
            let Some(a) = model.approvals.get(id) else { return div().into_any_element() };
            let card = super::approvals::Card::of(a);
            let hue = look(model.members.iter().find(|m| m.session == a.session).map_or("", |m| m.role.as_str()), &a.member).1;
            let ticket = a.ticket.and_then(|t| model.tickets.iter().find(|x| x.id == t));
            base(SharedString::from(format!("ask-{id}")), cx)
                .pl_2()
                .child(div().w(px(14.)).flex_none())
                .child(div().size(px(14.)).flex_none().flex().items_center().justify_center().text_size(px(12.)).text_color(p.accent).child("⚑"))
                .child(div().w(px(44.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(a.ticket.map(|t| format!("T-{t}")).unwrap_or_default()))
                .when(show_project, |r| r.child(project_col(ticket)))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w(px(120.))
                        .gap_2()
                        .overflow_hidden()
                        .child(div().flex_none().text_color(p.text).child(t!("approvals.allow_question", what = card.noun)))
                        .child(div().flex_1().min_w_0().truncate().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(card.lines.first().cloned().unwrap_or_default())),
                )
                .child(div().w(px(flags_w)).flex_none())
                .child(cell(detail_w).text_color(p.member(hue)).child(a.member.clone()))
                .child(cell(eta_w))
                .child(cell(34.))
                .into_any_element()
        }
        Need::Questions(id) => {
            let Some((title, ticket_id, name, color)) = super::decide::row_text(shell, id) else { return div().into_any_element() };
            let ticket = ticket_id.and_then(|t| model.tickets.iter().find(|x| x.id == t));
            base(SharedString::from(format!("decide-{id}")), cx)
                .pl_2()
                .child(div().w(px(14.)).flex_none())
                .child(super::decide::mark(p))
                .child(div().w(px(44.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(ticket_id.map(|t| format!("T-{t}")).unwrap_or_default()))
                .when(show_project, |r| r.child(project_col(ticket)))
                .child(div().flex_1().min_w(px(120.)).truncate().text_color(p.text).child(title))
                .child(div().w(px(flags_w)).flex_none())
                .child(cell(detail_w).text_color(color).child(name))
                .child(cell(eta_w))
                .child(cell(34.))
                .into_any_element()
        }
        Need::Ticket(id) => {
            let Some(t) = model.tickets.iter().find(|t| t.id == id) else { return div().into_any_element() };
            let m = meta(shell, t);
            let chosen = shell.board.view.chosen.contains(&id);
            let closed = t.status.is_closed();
            let live = model.is_live(t);
            let can = shell.can_act();
            let quick = |id: &'static str, label: &'static str, cx: &mut Context<Shell>, act: fn(&mut Shell, &mut Window, &mut Context<Shell>)| {
                div()
                    .id(SharedString::from(format!("{id}-{need:?}")))
                    .h(px(22.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(5.))
                    .bg(p.raised)
                    .border_1()
                    .border_color(p.hair)
                    .text_size(px(11.))
                    .text_color(p.text2)
                    .hover(move |s| s.bg(p.wash))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.board.view.selected = Some(need);
                        act(this, window, cx);
                    }))
                    .child(label)
            };
            let mut actions: Vec<AnyElement> = Vec::new();
            if can && (matches!(t.status, TicketStatus::Review | TicketStatus::Waiting | TicketStatus::Proposed) || t.not_on_main()) {
                actions.push(quick("q-a", "A", cx, |this, _, cx| this.board_yes(cx)).into_any_element());
            }
            if can && matches!(t.status, TicketStatus::Review | TicketStatus::Waiting | TicketStatus::Proposed | TicketStatus::Done) {
                actions.push(quick("q-r", "R", cx, |this, w, cx| this.board_open_reply(w, cx)).into_any_element());
            }
            if t.branch.is_some() {
                actions.push(quick("q-d", "D", cx, |this, w, cx| this.board_open_diff(w, cx)).into_any_element());
            }
            let has_actions = !actions.is_empty();
            base(SharedString::from(format!("ticket-{id}")), cx)
                .pl_2()
                .child(
                    div()
                        .id(SharedString::from(format!("check-{id}")))
                        .size(px(14.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(3.))
                        .border_1()
                        .border_color(if chosen { p.accent } else { p.hair })
                        .when(chosen, |c| c.bg(p.accent).text_color(p.on_accent).text_size(px(10.)).child("✓"))
                        .when(!chosen, |c| c.invisible().group_hover(group.clone(), |s| s.visible()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if !this.board.view.chosen.remove(&id) {
                                this.board.view.chosen.insert(id);
                            }
                            cx.notify();
                        })),
                )
                .child(div().relative().size(px(14.)).flex_none().child(icons::status(p, t.status, 14.)).when(live, |d| {
                    d.child(div().absolute().top(px(-2.)).right(px(-3.)).child(icons::live_dot(p, shell.board.pulse)))
                }))
                .child(div().w(px(44.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{id}")))
                .when(show_project, |r| r.child(project_col(Some(t))))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(120.))
                        .truncate()
                        .pl(px(depth as f32 * 18.))
                        .text_color(if closed { p.muted } else { p.text })
                        .child(if depth > 0 { format!("↳ {}", t.title) } else { t.title.clone() }),
                )
                .child(
                    h_flex()
                        .w(px(flags_w))
                        .flex_none()
                        .gap_1()
                        .justify_end()
                        .overflow_hidden()
                        .children(m.flags.into_iter().enumerate().map(|(i, (text, color, tip))| {
                            let chip = div().id(("flag", i)).flex_none().px(px(6.)).h(px(18.)).flex().items_center().rounded(px(4.)).bg(p.wash).text_size(px(11.)).text_color(color).child(text);
                            match tip {
                                Some(tip) => chip.tip(p, tip, None).into_any_element(),
                                None => chip.into_any_element(),
                            }
                        })),
                )
                .child(
                    h_flex()
                        .flex_none()
                        .gap_2()
                        .items_center()
                        .when(has_actions, |d| d.group_hover(group.clone(), |s| s.opacity(0.)))
                        .child(cell(detail_w).text_color(m.activity.as_ref().map_or(p.muted, |a| a.1)).child(m.activity.map(|a| a.0).unwrap_or_default()))
                        .child(cell(eta_w).text_color(m.eta.as_ref().map_or(p.muted, |e| e.1)).child(m.eta.map(|e| e.0).unwrap_or_default()))
                        .child(cell(34.).text_color(p.muted).child(m.time.unwrap_or_default())),
                )
                .when(has_actions, |r| r.child(h_flex().absolute().right(px(12.)).gap_1().invisible().group_hover(group.clone(), |s| s.visible()).children(actions)))
                .into_any_element()
        }
    }
}

/// The floating bar while several are chosen.
fn bulk_bar(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let n = shell.board.view.chosen.len();
    if n == 0 {
        return None;
    }
    let confirm = match shell.board.flow {
        Some(Flow::Bulk { yes }) => Some(yes),
        _ => None,
    };
    let merging = shell.board.view.chosen.iter().filter(|id| shell.model.tickets.iter().any(|t| t.id == **id && board::needs_way(t))).count();
    let beside = shell.board.view.peek && shell.board.wide && shell.board.view.page.is_none() && shell.picked_need().is_some();
    let bar = h_flex()
        .absolute()
        .bottom(px(20.))
        .left_0()
        .right(px(if beside { PEEK } else { 0. }))
        .justify_center()
        .child(
            h_flex()
                .h(px(40.))
                .px_3()
                .gap_2()
                .items_center()
                .rounded(px(10.))
                .bg(p.raised)
                .border_1()
                .border_color(p.hair)
                .shadow_lg()
                .text_size(px(SMALL))
                .child(div().px_1().font_weight(FontWeight::SEMIBOLD).child(t!("board.chosen", n = n)))
                .children(match confirm {
                    Some(yes) => vec![
                        div().text_color(p.text2).child(if yes { t!("board.bulk_yes_confirm", n = n, merging = merging) } else { t!("board.bulk_no_confirm", n = n) }).into_any_element(),
                        btn(p, "bulk-go", t!("board.confirm"), Some("⏎"), true, shell.can_act()).on_click(cx.listener(move |this, _, _, cx| this.board_bulk(yes, cx))).into_any_element(),
                        btn(p, "bulk-back", t!("board.keep"), Some("Esc"), false, true).on_click(cx.listener(|this, _, _, cx| {
                            this.board.flow = None;
                            cx.notify();
                        })).into_any_element(),
                    ],
                    None => vec![
                        btn(p, "bulk-yes", t!("board.bulk_accept"), Some("A"), true, shell.can_act()).on_click(cx.listener(|this, _, _, cx| this.board_yes(cx))).into_any_element(),
                        btn(p, "bulk-no", t!("board.bulk_drop"), Some("N"), false, shell.can_act()).on_click(cx.listener(|this, _, _, cx| this.board_no(cx))).into_any_element(),
                        btn(p, "bulk-clear", t!("board.clear_choice"), Some("Esc"), false, true).on_click(cx.listener(|this, _, _, cx| {
                            this.board.view.chosen.clear();
                            cx.notify();
                        })).into_any_element(),
                    ],
                }),
        );
    Some(bar.into_any_element())
}

/// "T-42 수락 · web에 병합+push 예정  U 되돌리기 (5)".
pub fn toast(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let pending = shell.board.undo.latest()?;
    let left = Undo::seconds_left(pending, Instant::now());
    let more = shell.board.undo.pending.len().saturating_sub(1);
    let lift = if shell.board.view.chosen.is_empty() { 20. } else { 72. };
    // Over the list, not the peek beside it.
    let beside = shell.board.view.peek && shell.board.wide && shell.board.diff.is_none() && shell.board.view.page.is_none() && shell.picked_need().is_some();
    Some(
        h_flex()
            .absolute()
            .bottom(px(lift))
            .left_0()
            .right(px(if beside { PEEK } else { 0. }))
            .justify_center()
            .child(
                h_flex()
                    .h(px(40.))
                    .pl_4()
                    .pr_2()
                    .gap_3()
                    .items_center()
                    .rounded(px(10.))
                    .bg(p.primary)
                    .text_color(p.on_primary)
                    .shadow_lg()
                    .text_size(px(SMALL))
                    .child(pending.text.clone())
                    .when(more > 0, |d| d.child(div().opacity(0.6).child(format!("+{more}"))))
                    .child(
                        h_flex()
                            .id("undo")
                            .h(px(28.))
                            .px_2()
                            .gap_2()
                            .items_center()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.8))
                            .on_click(cx.listener(|this, _, _, cx| this.board_undo(cx)))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("U {}", t!("board.undo"))))
                            .child(div().opacity(0.6).child(format!("({left})"))),
                    ),
            )
            .into_any_element(),
    )
}

/// The keys, on ?.
fn keys_sheet(p: Palette, cx: &mut Context<Shell>) -> AnyElement {
    let rows: [(&str, &str); 18] = [
        ("A", t!("board.key_yes")),
        ("⇧A", t!("board.key_method")),
        ("R", t!("board.key_reply")),
        ("N", t!("board.key_no")),
        ("D", t!("board.key_diff")),
        ("C", t!("board.key_talk")),
        ("M", t!("board.key_merge")),
        ("U", t!("board.key_undo")),
        ("J K ↑↓", t!("board.key_move")),
        ("Space", t!("board.key_peek")),
        ("⏎", t!("board.key_page")),
        ("/", t!("board.key_search")),
        ("F", t!("board.key_filter")),
        ("X ⇧↑↓ ⌘A", t!("board.key_choose")),
        ("⌥V", t!("board.key_save_view")),
        ("G I · G L · G B", t!("board.key_go")),
        ("⌘0–9", t!("board.key_projects")),
        ("Esc", t!("board.key_back")),
    ];
    deferred(
        div()
            .id("keys-sheet")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000040))
            .occlude()
            .on_click(cx.listener(|this, _, _, cx| {
                this.board.keys_open = false;
                cx.notify();
            }))
            .child(
                v_flex()
                    .w(px(440.))
                    .p_5()
                    .gap_1()
                    .rounded(px(12.))
                    .bg(p.raised)
                    .border_1()
                    .border_color(p.hair)
                    .shadow_lg()
                    .child(div().pb_2().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("board.keys_title")))
                    .children(rows.into_iter().map(|(k, what)| {
                        h_flex().h(px(26.)).gap_3().items_center().text_size(px(SMALL)).child(div().w(px(120.)).font_family(MONO).text_size(px(12.)).text_color(p.text2).child(k)).child(div().text_color(p.text).child(what))
                    })),
            ),
    )
    .into_any_element()
}

