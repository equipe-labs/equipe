//! The board (T-54 §5, T-62 §2.5): the terminal board's whole job in a window.
//! Tickets by state or as a tree of what waits for what, a search, the picked
//! ticket's detail, its diff, and the person's calls — accept (then the merge
//! and push questions), send back, resume, cancel. The rules are in
//! `crate::board`, the git side in `crate::diffs`; this file is what is drawn
//! and what a click or a key does. The board changes only when the daemon says
//! so: every action here is a command, never an edit of the list.

use equipe_core::t;
use std::collections::{HashMap, HashSet};

use equipe_core::protocol::{Command, Ticket, TicketId, TicketStatus};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::board::{self, Row, Tone, View};
use crate::diffs::{self, Fetched, Kind, Source, Stat};
use crate::state::{Conn, Model, Presence};
use crate::target::Endpoint;
use crate::theme::{BODY, MONO, PROJECT_HUES, Palette, SMALL, look};
use crate::{Screen, Shell};

const ROW: f32 = 40.;
const HEADING: f32 = 32.;
const SPLIT_MIN: f32 = 96.;
const DETAIL_MIN: f32 = 200.;

/// A question or a note being written, in the detail panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// `Accepted T-n. Merge <branch> into main?`
    Merge(TicketId),
    /// `…and push main to origin too?`
    Push(TicketId),
    /// What to change: required.
    SendBack(TicketId),
    /// What the person did: optional.
    Resume(TicketId),
    Cancel(TicketId),
}

impl Flow {
    fn ticket(self) -> TicketId {
        match self {
            Flow::Merge(t) | Flow::Push(t) | Flow::SendBack(t) | Flow::Resume(t) | Flow::Cancel(t) => t,
        }
    }
}

type Seen = (Option<String>, String, Option<bool>);

/// The diff open on screen.
pub struct DiffUi {
    pub ticket: TicketId,
    pub branch: String,
    /// `None` while git is asked.
    pub files: Option<Vec<diffs::File>>,
    pub patch: String,
    pub error: Option<String>,
    pub current: usize,
    folded: HashSet<usize>,
    /// What the list shows: `(file, line)`; a file's own row has `usize::MAX`.
    flat: Vec<(usize, usize)>,
    scroll: UniformListScrollHandle,
}

impl DiffUi {
    fn rebuild(&mut self) {
        self.flat.clear();
        for (i, file) in self.files.iter().flatten().enumerate() {
            self.flat.push((i, usize::MAX));
            if !self.folded.contains(&i) {
                self.flat.extend((0..file.lines.len()).map(|l| (i, l)));
            }
        }
    }
}

pub struct BoardUi {
    pub view: View,
    pub search: Entity<InputState>,
    send_back: Entity<TextareaState>,
    resume: Entity<InputState>,
    pub flow: Option<Flow>,
    /// The list's height as the person left it (px); `None`: as tall as it needs.
    pub split: Option<f32>,
    /// Dragging the divider: where the pointer was and how tall the list was.
    drag: Option<(f32, f32)>,
    list_scroll: ScrollHandle,
    detail_scroll: ScrollHandle,
    stats: HashMap<TicketId, (Seen, Stat)>,
    asked: HashMap<TicketId, Seen>,
    pub diff: Option<DiffUi>,
    fetched: tokio::sync::mpsc::UnboundedSender<Fetched>,
    /// The text to put in the search box at the next draw (which has a window).
    pending_search: Option<String>,
}

impl BoardUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>, split: Option<f32>, fetched: tokio::sync::mpsc::UnboundedSender<Fetched>) -> Self {
        Self {
            view: View::default(),
            search: cx.new(|cx| InputState::new(window, cx).placeholder(t!("board.search_placeholder"))),
            send_back: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 8).submit_on_enter(false).placeholder(t!("board.send_back_placeholder"))),
            resume: cx.new(|cx| InputState::new(window, cx).placeholder(t!("board.resume_placeholder"))),
            flow: None,
            split,
            drag: None,
            list_scroll: ScrollHandle::new(),
            detail_scroll: ScrollHandle::new(),
            stats: HashMap::new(),
            asked: HashMap::new(),
            diff: None,
            fetched,
            pending_search: None,
        }
    }

    /// Whether one of the board's text boxes has the keyboard.
    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        self.search.read(cx).focus_handle(cx).is_focused(window)
            || self.send_back.read(cx).focus_handle(cx).is_focused(window)
            || self.resume.read(cx).focus_handle(cx).is_focused(window)
    }
}

// ── what a click or a key does ──

impl Shell {
    fn can_act(&self) -> bool {
        matches!(self.model.conn, Conn::Connected { .. })
    }

    fn picked(&self) -> Option<Ticket> {
        board::selected(&self.model, &self.board.view).cloned()
    }

    /// Wire the board's text boxes to the window.
    pub fn board_subscriptions(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<Subscription> {
        let search = self.board.search.clone();
        let send_back = self.board.send_back.clone();
        let resume = self.board.resume.clone();
        vec![
            cx.subscribe_in(&search, window, |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    if text != this.board.view.query {
                        this.board.view.query = text;
                        this.board_changed(cx);
                    }
                }
            }),
            cx.subscribe_in(&send_back, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { secondary: true, .. }) {
                    this.board_send_back(window, cx);
                }
            }),
            cx.subscribe_in(&resume, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.board_resume(window, cx);
                }
            }),
        ]
    }

    /// The list or the pick changed: the detail starts at its top, and what the
    /// ticket changed is asked of git once.
    fn board_changed(&mut self, cx: &mut Context<Self>) {
        self.board.detail_scroll.set_offset(point(px(0.), px(0.)));
        if let Some(flow) = self.board.flow
            && self.picked().is_none_or(|t| t.id != flow.ticket())
            && !matches!(flow, Flow::Merge(_) | Flow::Push(_))
        {
            self.board.flow = None;
        }
        self.board_sync(cx);
        self.scroll_to_pick();
        cx.notify();
    }

    fn scroll_to_pick(&self) {
        let Some(pick) = board::selected(&self.model, &self.board.view).map(|t| t.id) else { return };
        let rows = board::rows(&self.model, &self.board.view);
        if let Some(at) = rows.iter().position(|r| matches!(r, Row::Ticket { id, .. } if *id == pick)) {
            self.board.list_scroll.scroll_to_item(at);
        }
    }

    /// Bring what the board derives up to date: the stat of the picked ticket's
    /// branch (asked once per look of the ticket), the diff open, a ticket a
    /// `T-n` link asked for.
    pub fn board_sync(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.board_ticket.take() {
            self.board_reveal(id);
        }
        if self.screen != Screen::Board {
            return;
        }
        if let Some(t) = self.picked()
            && let Some(source) = diffs::source(&self.model, &t)
        {
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
        cx.notify();
    }

    fn fetch(&self, ticket: TicketId, source: Source, full: bool) {
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
                match f.text {
                    Ok(patch) => {
                        let files = diffs::parse(&patch);
                        d.folded = if diffs::total_lines(&files) > diffs::BIG { (1..files.len()).collect() } else { HashSet::new() };
                        d.files = Some(files);
                        d.patch = patch;
                        d.rebuild();
                    }
                    Err(why) => d.error = Some(why),
                }
            }
        } else if let Ok(text) = f.text {
            let seen = diffs::fingerprint(t);
            self.board.stats.insert(f.ticket, (seen, diffs::parse_stat(&text)));
        }
        cx.notify();
    }

    /// Make `id` the pick, searching for it when the list would not show it.
    pub fn board_reveal(&mut self, id: TicketId) {
        self.board.view.selected = Some(id);
        if !board::order(&self.model, &self.board.view).contains(&id) {
            let q = format!("t-{id}");
            self.board.view.query = q.clone();
            self.board.pending_search = Some(q);
        }
        self.board.flow = None;
        self.board.diff = None;
        self.board.detail_scroll.set_offset(point(px(0.), px(0.)));
        self.scroll_to_pick();
    }

    pub fn board_pick(&mut self, id: TicketId, cx: &mut Context<Self>) {
        self.board.view.selected = Some(id);
        self.composer.flash = None;
        self.board_changed(cx);
    }

    pub fn board_step(&mut self, by: isize, cx: &mut Context<Self>) {
        board::step(&self.model, &mut self.board.view, by);
        self.composer.flash = None;
        self.board_changed(cx);
    }

    /// `/find words`, from the conversation: the board, narrowed.
    pub fn board_find(&mut self, words: &str, cx: &mut Context<Self>) {
        self.board.view.query = words.to_owned();
        self.board.pending_search = Some(words.to_owned());
        self.show(Screen::Board, cx);
        self.board_changed(cx);
    }

    pub fn set_graph(&mut self, graph: bool, cx: &mut Context<Self>) {
        self.board.view.graph = graph;
        self.board_changed(cx);
    }

    fn say(&mut self, text: impl Into<String>) {
        self.composer.flash = Some(text.into());
    }

    fn board_accept(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.picked().filter(|t| !t.status.is_closed()) else { return };
        if !self.can_act() {
            return;
        }
        self.send_command(Command::Accept { squad: self.model.squad.clone(), ticket: t.id });
        if board::asks_to_merge(&t) {
            self.board.flow = Some(Flow::Merge(t.id));
            self.say(t!("board.accepted_ask_merge", id = t.id));
        } else if t.kind.in_home() {
            self.say(t!("board.accepted_home", id = t.id));
        } else {
            self.say(t!("board.accepted", id = t.id));
        }
        board::next_review_after(&self.model, &mut self.board.view, t.id);
        self.board_changed(cx);
    }

    /// The merge question for work accepted but not on main (M).
    fn board_ask_merge(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.picked().filter(|t| t.not_on_main()) {
            self.board.flow = Some(Flow::Push(t.id));
            cx.notify();
        }
    }

    fn board_merge(&mut self, ticket: TicketId, push: bool, cx: &mut Context<Self>) {
        self.board.flow = None;
        self.send_command(Command::Merge { squad: self.model.squad.clone(), ticket, push });
        self.say(t!("board.merging", ticket = ticket));
        cx.notify();
    }

    fn board_decline_merge(&mut self, ticket: TicketId, cx: &mut Context<Self>) {
        self.board.flow = None;
        self.say(t!("board.stays_on_branch", ticket = ticket));
        cx.notify();
    }

    fn board_open_flow(&mut self, flow: Flow, window: &mut Window, cx: &mut Context<Self>) {
        self.board.flow = Some(flow);
        match flow {
            Flow::SendBack(_) => {
                self.board.send_back.update(cx, |s, cx| {
                    s.set_value(String::new(), window, cx);
                    s.focus(window, cx);
                });
            }
            Flow::Resume(_) => {
                self.board.resume.update(cx, |s, cx| {
                    s.set_value(String::new(), window, cx);
                    s.focus(window, cx);
                });
            }
            _ => {}
        }
        cx.notify();
    }

    fn board_close_flow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(flow) = self.board.flow.take() {
            if let Flow::Merge(t) | Flow::Push(t) = flow {
                self.say(t!("board.stays_on_branch", ticket = t));
            }
            window.blur(cx);
            cx.notify();
        }
    }

    fn board_send_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Flow::SendBack(id)) = self.board.flow else { return };
        let note = self.board.send_back.read(cx).value().trim().to_owned();
        if note.is_empty() || !self.can_act() {
            return;
        }
        self.send_command(Command::Reopen { squad: self.model.squad.clone(), ticket: id, note });
        self.board.flow = None;
        self.board.diff = None;
        self.say(t!("board.sent_back", id = id));
        window.blur(cx);
        cx.notify();
    }

    fn board_resume(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Flow::Resume(id)) = self.board.flow else { return };
        if !self.can_act() {
            return;
        }
        let note = self.board.resume.read(cx).value().trim().to_owned();
        self.send_command(Command::Resume { squad: self.model.squad.clone(), ticket: id, note });
        self.board.flow = None;
        self.say(t!("board.resumed", id = id));
        window.blur(cx);
        cx.notify();
    }

    fn board_cancel(&mut self, cx: &mut Context<Self>) {
        let Some(Flow::Cancel(id)) = self.board.flow else { return };
        if !self.can_act() {
            return;
        }
        self.send_command(Command::Cancel { squad: self.model.squad.clone(), ticket: id });
        self.board.flow = None;
        self.say(t!("board.cancelled", id = id));
        cx.notify();
    }

    /// Enter: talk to whoever has the ticket, if still on the squad.
    pub fn board_talk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.picked() else { return };
        let Some(who) = board::who(&t).map(str::to_owned) else { return };
        let target = self.model.members.iter().find(|m| m.presence != Presence::Closed && (m.name == who || m.role == who)).map(|m| m.session);
        if let Some(session) = target {
            self.select_member(session, window, cx);
        }
    }

    fn board_open_diff(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.picked() else { return };
        let Some(source) = diffs::source(&self.model, &t) else { return };
        self.board.diff = Some(DiffUi {
            ticket: t.id,
            branch: source.branch.clone(),
            files: None,
            patch: String::new(),
            error: None,
            current: 0,
            folded: HashSet::new(),
            flat: Vec::new(),
            scroll: UniformListScrollHandle::new(),
        });
        self.fetch(t.id, source, true);
        cx.notify();
    }

    fn board_look_in(&self, path: &std::path::Path) {
        if matches!(self.endpoint, Endpoint::Local { .. }) {
            let _ = std::process::Command::new("open").arg(path).spawn();
        }
    }

    /// Esc: a question or note first, then the diff, then the search, then the conversation.
    pub fn board_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.board.flow.is_some() {
            self.board_close_flow(window, cx);
        } else if self.board.diff.is_some() {
            self.board.diff = None;
            cx.notify();
        } else if !self.board.view.query.is_empty() {
            self.board.view.query.clear();
            self.board.search.update(cx, |s, cx| s.set_value(String::new(), window, cx));
            window.blur(cx);
            self.board_changed(cx);
        } else {
            window.blur(cx);
            self.show(Screen::Chat, cx);
            self.composer.focus(window, cx);
        }
    }

    /// A key on the board. True when it was taken.
    pub fn on_board_key(&mut self, key: &str, plain: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.screen != Screen::Board {
            return false;
        }
        if key == "escape" && plain {
            self.board_escape(window, cx);
            return true;
        }
        if self.board.typing(window, cx) {
            return false;
        }
        if !plain {
            return false;
        }
        let ticket = self.picked();
        // The questions have the keys while they are open.
        match (self.board.flow, key) {
            (Some(Flow::Merge(_)), "y") => {
                let id = self.board.flow.map(Flow::ticket).unwrap_or_default();
                self.board.flow = Some(Flow::Push(id));
                cx.notify();
                return true;
            }
            (Some(Flow::Merge(id)), "n") => {
                self.board_decline_merge(id, cx);
                return true;
            }
            (Some(Flow::Push(id)), "y") => {
                self.board_merge(id, true, cx);
                return true;
            }
            (Some(Flow::Push(id)), "n") => {
                self.board_merge(id, false, cx);
                return true;
            }
            (Some(Flow::Cancel(_)), "y") => {
                self.board_cancel(cx);
                return true;
            }
            (Some(Flow::Cancel(_)), "n") => {
                self.board.flow = None;
                cx.notify();
                return true;
            }
            _ => {}
        }
        match key {
            "up" | "k" => self.board_step(-1, cx),
            "down" | "j" => self.board_step(1, cx),
            "f" | "/" => {
                self.board.search.update(cx, |s, cx| s.focus(window, cx));
                cx.notify();
            }
            "e" => {
                self.board.view.expanded = !self.board.view.expanded;
                self.board_changed(cx);
            }
            "g" => {
                let graph = !self.board.view.graph;
                self.set_graph(graph, cx);
            }
            "a" => self.board_accept(cx),
            "m" => self.board_ask_merge(cx),
            "r" => {
                let Some(t) = ticket else { return false };
                match t.status {
                    TicketStatus::Waiting => self.board_open_flow(Flow::Resume(t.id), window, cx),
                    TicketStatus::Review | TicketStatus::Done => self.board_open_flow(Flow::SendBack(t.id), window, cx),
                    _ => return false,
                }
            }
            "d" => self.board_open_diff(cx),
            "enter" => self.board_talk(window, cx),
            _ => return false,
        }
        true
    }

    /// Dev tool (`snapshot.rs`): write in the open question's text box.
    #[cfg(feature = "snapshot")]
    pub fn board_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        match self.board.flow {
            Some(Flow::SendBack(_)) => self.board.send_back.update(cx, |s, cx| s.set_value(text.to_owned(), window, cx)),
            Some(Flow::Resume(_)) => self.board.resume.update(cx, |s, cx| s.set_value(text.to_owned(), window, cx)),
            _ => {}
        }
        cx.notify();
    }

    /// Dev tool: the t!("board.cancel_ellipsis") button.
    #[cfg(feature = "snapshot")]
    pub fn board_ask_cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.picked() {
            self.board.flow = Some(Flow::Cancel(t.id));
            cx.notify();
        }
    }

    /// Dev tool: the open question's main button.
    #[cfg(feature = "snapshot")]
    pub fn board_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.board.flow {
            Some(Flow::SendBack(_)) => self.board_send_back(window, cx),
            Some(Flow::Resume(_)) => self.board_resume(window, cx),
            _ => {}
        }
    }

    fn pending_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.board.pending_search.take() {
            self.board.search.update(cx, |s, cx| s.set_value(text, window, cx));
        }
    }
}

// ── drawing ──

fn btn(p: Palette, id: impl Into<ElementId>, label: impl Into<SharedString>, key: Option<&'static str>, primary: bool, enabled: bool) -> Stateful<Div> {
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
        .when_some(key, |b, key| b.child(div().text_size(px(12.)).opacity(0.6).child(key)))
}

fn kicker(p: Palette, text: impl Into<SharedString>) -> Div {
    div().text_size(px(11.)).text_color(p.muted).font_weight(FontWeight::MEDIUM).child(text.into())
}

fn hue_of(p: Palette, model: &Model, t: &Ticket) -> Option<Rgba> {
    let path = model.ticket_project(t)?;
    model.projects.iter().position(|q| *q == path).map(|ix| p.member(PROJECT_HUES[ix % PROJECT_HUES.len()]))
}

fn member_color(p: Palette, model: &Model, who: &str) -> Rgba {
    model.members.iter().find(|m| m.name == who || m.role == who).map_or(p.text2, |m| p.member(look(&m.role, &m.name).1))
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let ui = &shell.board;
    let body = match &ui.diff {
        Some(diff) => diff_view(shell, diff, cx).into_any_element(),
        None => split_view(shell, cx).into_any_element(),
    };
    v_flex()
        .id("board")
        .flex_1()
        .min_h_0()
        .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
            if let Some((y0, h0)) = this.board.drag {
                if e.pressed_button != Some(MouseButton::Left) {
                    this.board_end_drag();
                } else {
                    let max = (f32::from(window.viewport_size().height) - 40. - 52. - 44. - DETAIL_MIN).max(SPLIT_MIN);
                    this.board.split = Some((h0 + f32::from(e.position.y) - y0).clamp(SPLIT_MIN, max));
                }
                cx.notify();
            }
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.board_end_drag();
            cx.notify();
        }))
        .child(toolbar(shell, cx))
        // The merge questions are about a ticket that may no longer be the pick (the next review came up).
        .children(shell.board.flow.filter(|f| matches!(f, Flow::Merge(_) | Flow::Push(_))).and_then(|flow| {
            let t = shell.model.tickets.iter().find(|t| t.id == flow.ticket())?;
            Some(div().px_5().pb_2().child(flow_card(shell, flow, t, shell.can_act(), cx)))
        }))
        .children(shell.composer.flash.clone().map(|text| div().px_5().pb_1().text_size(px(SMALL)).text_color(p.muted).child(text)))
        .child(body)
}

impl Shell {
    fn board_end_drag(&mut self) {
        if self.board.drag.take().is_some()
            && let Some(h) = self.board.split
        {
            self.remembered.board_split = Some(h);
            self.remembered.save(&equipe_providers::credentials::equipe_home());
        }
    }

    /// Called from the window's draw: what needs a window and was asked for without one.
    pub fn board_prepare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_search(window, cx);
    }
}

fn toolbar(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let model = &shell.model;
    let review = model.tickets.iter().filter(|t| t.status == TicketStatus::Review).count();
    let waiting = model.tickets.iter().filter(|t| t.status == TicketStatus::Waiting).count();
    let summary = match (review, waiting) {
        (0, 0) => String::new(),
        (r, 0) => t!("board.summary_review", r = r),
        (0, w) => t!("board.summary_waiting", w = w),
        (r, w) => t!("board.summary_both", r = r, w = w),
    };
    let graph = shell.board.view.graph;
    let seg = |id: &'static str, label: &'static str, on: bool, graph_value: bool, cx: &mut Context<Shell>| {
        div()
            .id(id)
            .h(px(28.))
            .px_3()
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(SMALL))
            .cursor_pointer()
            .when(on, |d| d.bg(p.wash).font_weight(FontWeight::SEMIBOLD))
            .when(!on, |d| d.text_color(p.text2).hover(move |s| s.bg(p.wash)))
            .on_click(cx.listener(move |this, _, _, cx| this.set_graph(graph_value, cx)))
            .child(label)
    };
    h_flex()
        .h(px(44.))
        .flex_none()
        .px_5()
        .gap_3()
        .items_center()
        .child(h_flex().gap_1().child(seg("by-state", t!("board.by_state"), !graph, false, cx)).child(seg("by-tree", t!("board.by_tree"), graph, true, cx)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .max_w(px(380.))
                .px_3()
                .py(px(3.))
                .rounded(px(8.))
                .bg(p.code_bg)
                .border_1()
                .border_color(p.hair)
                .text_size(px(SMALL))
                .text_color(p.text)
                .child(Input::new(&shell.board.search).appearance(false).bordered(false)),
        )
        .child(div().flex_1())
        .child(div().text_size(px(SMALL)).text_color(if review + waiting > 0 { p.accent } else { p.muted }).child(summary))
}

fn split_view(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let rows = board::rows(&shell.model, &shell.board.view);
    let need: f32 = rows.iter().map(|r| if matches!(r, Row::Ticket { .. }) { ROW } else { HEADING + 8. }).sum::<f32>() + 8.;
    let height = shell.board.split.unwrap_or(need.clamp(SPLIT_MIN, 340.));
    let dragging = shell.board.drag.is_some();
    let empty = rows.is_empty();
    let list = v_flex()
        .id("board-list")
        .h(px(height))
        .flex_none()
        .overflow_y_scroll()
        .track_scroll(&shell.board.list_scroll)
        .px_3()
        .pb_2()
        .children(rows.iter().enumerate().map(|(ix, row)| list_row(shell, ix, row, cx)))
        .when(empty, |l| l.child(empty_note(shell)));
    v_flex()
        .flex_1()
        .min_h_0()
        .child(list)
        .child(
            div()
                .id("board-split")
                .h(px(9.))
                .flex_none()
                .flex()
                .items_center()
                .cursor_row_resize()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        this.board.drag = Some((f32::from(e.position.y), this.board.split.unwrap_or(height)));
                        this.board.split.get_or_insert(height);
                        cx.notify();
                    }),
                )
                .child(div().w_full().h(px(if dragging { 2. } else { 1. })).bg(if dragging { p.accent } else { p.hair })),
        )
        .child(detail(shell, cx))
}

fn empty_note(shell: &Shell) -> Div {
    let p = shell.palette;
    let text = if let Some(q) = board::query(&shell.board.view) {
        t!("board.empty_query", q = q)
    } else if let Some(focus) = shell.model.focus.as_ref().filter(|_| !shell.model.tickets.is_empty()) {
        t!("board.empty_focus", focus = focus)
    } else {
        t!("board.empty").to_owned()
    };
    div().py_6().text_size(px(BODY)).text_color(p.muted).child(text)
}

fn list_row(shell: &Shell, ix: usize, row: &Row, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    match row {
        Row::Heading { label, count, tone } => {
            let color = match tone {
                Tone::ForYou => p.accent,
                Tone::Lane(Some(i)) => p.member(PROJECT_HUES[*i % PROJECT_HUES.len()]),
                Tone::Lane(None) | Tone::Plain => p.muted,
            };
            h_flex()
                .h(px(HEADING))
                .mt_2()
                .px_2()
                .gap_2()
                .items_end()
                .pb_1()
                .text_size(px(12.))
                .text_color(color)
                .font_weight(if *tone == Tone::ForYou { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                .child(label.clone())
                .child(count.to_string())
                .into_any_element()
        }
        Row::More { more, .. } => div()
            .id(("more", ix))
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .text_size(px(SMALL))
            .text_color(p.muted)
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.board.view.expanded = true;
                this.board_changed(cx);
            }))
            .child(t!("board.more_rows", more = more))
            .into_any_element(),
        Row::Ticket { id, depth } => {
            let Some(t) = model.tickets.iter().find(|t| t.id == *id) else { return div().into_any_element() };
            let picked = board::selected(model, &shell.board.view).is_some_and(|s| s.id == *id);
            let closed = t.status.is_closed();
            let for_you = matches!(t.status, TicketStatus::Review | TicketStatus::Waiting);
            let who = board::who(t).unwrap_or("").to_owned();
            let id = *id;
            h_flex()
                .id(("ticket", id as usize))
                .h(px(ROW))
                .px_2()
                .gap_3()
                .items_center()
                .rounded(px(6.))
                .text_size(px(14.))
                .cursor_pointer()
                .when(picked, |r| r.bg(p.wash))
                .when(!picked, |r| r.hover(move |s| s.bg(p.wash)))
                .on_click(cx.listener(move |this, _, _, cx| this.board_pick(id, cx)))
                .child(div().w(px(2.)).h(px(22.)).flex_none().rounded(px(1.)).when(for_you, |d| d.bg(p.accent)))
                .child(div().w(px(48.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{}", t.id)))
                .child(
                    div()
                        .w(px(84.))
                        .flex_none()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(hue_of(p, model, t).unwrap_or(p.muted))
                        .child(model.ticket_project_name(t).unwrap_or_default()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .pl(px(*depth as f32 * 16.))
                        .text_color(if closed { p.muted } else { p.text })
                        .when(picked, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .child(if *depth > 0 { format!("↳ {}", t.title) } else { t.title.clone() }),
                )
                .child(div().w(px(84.)).flex_none().truncate().text_size(px(13.)).text_color(if closed { p.muted } else { member_color(p, model, &who) }).child(who))
                .child(div().w(px(110.)).flex_none().truncate().text_align(TextAlign::Right).text_size(px(12.)).text_color(if for_you { p.accent } else { p.muted }).child(board::state_text(t)))
                .into_any_element()
        }
    }
}

fn detail(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let model = &shell.model;
    let column = v_flex().id("board-detail").flex_1().min_h_0().overflow_y_scroll().track_scroll(&shell.board.detail_scroll).px_5().pt_3().pb_6();
    let Some(t) = board::selected(model, &shell.board.view) else {
        return column.child(div().text_size(px(BODY)).text_color(p.muted).child(t!("board.pick_ticket")));
    };
    let can = shell.can_act();
    let id = t.id;
    let who = board::who(t).map(str::to_owned);
    let kind = t.kind.label();
    let status = match t.status {
        TicketStatus::Review => t!("board.detail_review"),
        TicketStatus::Waiting => t!("board.detail_waiting"),
        TicketStatus::InProgress => t!("board.state_in_progress"),
        TicketStatus::Todo => t!("board.todo"),
        TicketStatus::Proposed => t!("board.state_proposed"),
        TicketStatus::Done if t.not_on_main() => t!("board.detail_not_on_main"),
        TicketStatus::Done => t!("board.done"),
        TicketStatus::Cancelled => t!("board.state_cancelled"),
    };
    let project = model.ticket_project_name(t).filter(|_| model.projects.len() > 1 || t.kind.in_home());

    // The buttons: the one call the person is here to make is the main one.
    let mut buttons: Vec<AnyElement> = Vec::new();
    let for_you = matches!(t.status, TicketStatus::Review | TicketStatus::Waiting);
    let accept_label = match t.status {
        TicketStatus::Proposed => t!("board.accept_proposal"),
        TicketStatus::Review => t!("board.accept"),
        _ => t!("board.mark_done"),
    };
    let mut primary = Some(());
    let mut take_primary = || primary.take().is_some();
    match t.status {
        TicketStatus::Review => {
            buttons.push(btn(p, "accept", accept_label, Some("A"), take_primary(), can).on_click(cx.listener(|this, _, _, cx| this.board_accept(cx))).into_any_element());
            buttons.push(btn(p, "send-back", t!("board.request_changes"), Some("R"), false, can).on_click(cx.listener(move |this, _, w, cx| this.board_open_flow(Flow::SendBack(id), w, cx))).into_any_element());
        }
        TicketStatus::Waiting => {
            buttons.push(btn(p, "resume", t!("board.resume_done"), Some("R"), take_primary(), can).on_click(cx.listener(move |this, _, w, cx| this.board_open_flow(Flow::Resume(id), w, cx))).into_any_element());
            buttons.push(btn(p, "accept", t!("board.mark_done"), Some("A"), false, can).on_click(cx.listener(|this, _, _, cx| this.board_accept(cx))).into_any_element());
        }
        TicketStatus::Done => {
            if t.not_on_main() {
                buttons.push(btn(p, "merge", t!("board.merge_into_main"), Some("M"), take_primary(), can).on_click(cx.listener(|this, _, _, cx| this.board_ask_merge(cx))).into_any_element());
            }
            buttons.push(btn(p, "reopen", t!("board.reopen"), Some("R"), false, can).on_click(cx.listener(move |this, _, w, cx| this.board_open_flow(Flow::SendBack(id), w, cx))).into_any_element());
        }
        TicketStatus::Proposed | TicketStatus::Todo | TicketStatus::InProgress => {
            buttons.push(btn(p, "accept", accept_label, Some("A"), take_primary() && t.status == TicketStatus::Proposed, can).on_click(cx.listener(|this, _, _, cx| this.board_accept(cx))).into_any_element());
        }
        TicketStatus::Cancelled => {}
    }
    if board::has_branch(t) {
        buttons.push(btn(p, "diff", t!("board.view_changes"), Some("D"), false, true).on_click(cx.listener(|this, _, _, cx| this.board_open_diff(cx))).into_any_element());
    }
    if let Some(who) = &who {
        let open = model.members.iter().any(|m| m.presence != Presence::Closed && (m.name == *who || m.role == *who));
        buttons.push(btn(p, "talk", t!("board.talk_to", who = who), Some("⏎"), false, open).on_click(cx.listener(|this, _, w, cx| this.board_talk(w, cx))).into_any_element());
    }
    if !t.status.is_closed() {
        buttons.push(btn(p, "cancel", t!("board.cancel_ellipsis"), None, false, can).on_click(cx.listener(move |this, _, _, cx| {
            this.board.flow = Some(Flow::Cancel(id));
            cx.notify();
        })).into_any_element());
    }

    let after: Vec<AnyElement> = t.depends_on.iter().map(|d| named(shell, *d, cx)).collect();
    let unblocks: Vec<AnyElement> = board::unblocks(model, t.id).into_iter().map(|d| named(shell, d, cx)).collect();

    let checklist = if t.status == TicketStatus::Waiting { t.note.as_deref().map(board::checklist).unwrap_or_default() } else { Vec::new() };
    let text = if t.status == TicketStatus::Waiting && !checklist.is_empty() {
        None
    } else {
        t.note.as_deref().filter(|n| !n.trim().is_empty()).or(Some(t.body.as_str()).filter(|b| !b.trim().is_empty()))
    };
    let handle = cx.entity().downgrade();

    let stat = shell.board.stats.get(&t.id).filter(|(seen, _)| *seen == diffs::fingerprint(t)).map(|(_, s)| s.clone());
    let branch = t.branch.clone().filter(|_| !t.kind.in_home());
    let worktree = t.worktree.clone();

    column
        .gap_3()
        .child(
            h_flex()
                .gap_3()
                .items_baseline()
                .child(div().flex_none().font_family(MONO).text_size(px(13.)).text_color(p.muted).child(format!("T-{}", t.id)))
                .child(div().min_w_0().text_size(px(18.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).child(t.title.clone())),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap_x(px(6.))
                .text_size(px(SMALL))
                .text_color(p.muted)
                .child(kind)
                .children(who.clone().map(|w| h_flex().gap_1().child("·").child(div().text_color(member_color(p, model, &w)).child(w))))
                .children(project.map(|pr| h_flex().gap_1().child("·").child(div().text_color(hue_of(p, model, t).unwrap_or(p.muted)).child(pr))))
                .child(format!("· {status}")),
        )
        .child(h_flex().flex_wrap().gap_2().children(buttons))
        .children(shell.board.flow.filter(|f| f.ticket() == t.id && !matches!(f, Flow::Merge(_) | Flow::Push(_))).map(|flow| flow_card(shell, flow, t, can, cx)))
        .children(checklist.is_empty().not_then(|| {
            v_flex()
                .gap_1()
                .child(kicker(p, t!("board.kicker_for_me")))
                .children(checklist.iter().map(|item| h_flex().gap_2().items_start().text_size(px(BODY)).line_height(px(24.)).text_color(p.text2).child(div().flex_none().text_color(if for_you { p.accent } else { p.muted }).child("□")).child(div().flex_1().min_w_0().child(item.clone()))))
        }))
        .children(text.map(|text| div().child(super::chat::markdown(p, ("board-text", id as u64), text, 14., 22., p.text2, &handle))))
        .children((!after.is_empty()).then(|| v_flex().child(kicker(p, t!("board.kicker_after"))).children(after)))
        .children((!unblocks.is_empty()).then(|| v_flex().child(kicker(p, t!("board.kicker_unblocks"))).children(unblocks)))
        .child(
            v_flex()
                .gap_1()
                .text_size(px(SMALL))
                .child(kicker(p, t!("board.kicker_where")))
                .children(match (&stat, board::has_branch(t)) {
                    (Some(s), _) if !s.summary.is_empty() => {
                        let more = if s.files.len() > 4 { format!(" +{}", s.files.len() - 4) } else { String::new() };
                        vec![
                            h_flex().gap_3().child(div().w(px(56.)).text_color(p.muted).child(t!("board.where_changes"))).child(div().text_color(p.text2).child(s.summary.clone())).into_any_element(),
                            h_flex()
                                .gap_3()
                                .child(div().w(px(56.)).flex_none())
                                .child(div().min_w_0().truncate().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("{}{more}", s.files.iter().take(4).cloned().collect::<Vec<_>>().join(" · "))))
                                .into_any_element(),
                        ]
                    }
                    (None, true) => vec![h_flex().gap_3().child(div().w(px(56.)).text_color(p.muted).child(t!("board.where_changes"))).child(div().text_color(p.muted).child("…")).into_any_element()],
                    _ => Vec::new(),
                })
                .children(branch.map(|b| h_flex().gap_3().child(div().w(px(56.)).text_color(p.muted).child(t!("board.where_branch"))).child(div().min_w_0().truncate().font_family(MONO).text_size(px(12.)).text_color(p.text2).child(b))))
                .children(worktree.map(|w| {
                    let local = matches!(shell.endpoint, Endpoint::Local { .. });
                    let path = w.clone();
                    h_flex()
                        .id("look-in")
                        .gap_3()
                        .when(local, |d| d.cursor_pointer().on_click(cx.listener(move |this, _, _, _| this.board_look_in(&path))))
                        .child(div().w(px(56.)).flex_none().text_color(p.muted).child(t!("board.where_worktree")))
                        .child(div().min_w_0().truncate().font_family(MONO).text_size(px(12.)).text_color(p.text2).child(w.display().to_string()))
                })),
        )
}

fn named(shell: &Shell, dep: TicketId, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
        let found = model.tickets.iter().find(|x| x.id == dep);
        let (mark, title, state) = match found {
            Some(d) => (if matches!(d.status, TicketStatus::Done) { "✓" } else if d.status == TicketStatus::Cancelled { "×" } else { "" }, d.title.clone(), board::state_text(d)),
            None => ("", String::new(), String::new()),
        };
        h_flex()
            .id(("dep", dep as usize))
            .gap_3()
            .py(px(3.))
            .text_size(px(SMALL))
            .cursor_pointer()
            .hover(move |s| s.text_color(p.text))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.board_reveal(dep);
                this.board_changed(cx);
            }))
            .child(div().w(px(48.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{dep}")))
            .child(div().flex_1().min_w_0().truncate().text_color(p.text2).child(format!("{mark} {title}").trim().to_owned()))
            .child(div().flex_none().text_size(px(12.)).text_color(p.muted).child(state))
            .into_any_element()
    }

/// `bool::then`, the other way round: for "show this when the list is not empty".
trait NotThen {
    fn not_then<T>(self, f: impl FnOnce() -> T) -> Option<T>;
}
impl NotThen for bool {
    fn not_then<T>(self, f: impl FnOnce() -> T) -> Option<T> {
        (!self).then(f)
    }
}

fn flow_card(shell: &Shell, flow: Flow, t: &Ticket, can: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = t.id;
    let branch = t.branch.clone().unwrap_or_default();
    let card = || v_flex().gap_2().p_3().rounded(px(8.)).bg(p.accent_bg).border_1().border_color(p.hair);
    let ask = |text: String| div().text_size(px(BODY)).line_height(px(24.)).text_color(p.text).child(text);
    match flow {
        Flow::Merge(_) => card()
            .child(ask(t!("board.ask_merge", id = id, branch = branch)))
            .child(
                h_flex()
                    .gap_2()
                    .child(btn(p, "merge-yes", t!("board.merge"), Some("Y"), true, can).on_click(cx.listener(move |this, _, _, cx| {
                        this.board.flow = Some(Flow::Push(id));
                        cx.notify();
                    })))
                    .child(btn(p, "merge-no", t!("board.not_now"), Some("N"), false, true).on_click(cx.listener(move |this, _, _, cx| this.board_decline_merge(id, cx)))),
            )
            .into_any_element(),
        Flow::Push(_) => card()
            .child(ask(t!("board.ask_push", id = id)))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(btn(p, "push-yes", t!("board.merge_and_push"), Some("Y"), true, can).on_click(cx.listener(move |this, _, _, cx| this.board_merge(id, true, cx))))
                    .child(btn(p, "push-no", t!("board.merge_only"), Some("N"), false, can).on_click(cx.listener(move |this, _, _, cx| this.board_merge(id, false, cx))))
                    .child(btn(p, "push-skip", t!("board.dont_merge"), Some("Esc"), false, true).on_click(cx.listener(|this, _, w, cx| this.board_close_flow(w, cx)))),
            )
            .into_any_element(),
        Flow::SendBack(_) => {
            let empty = shell.board.send_back.read(cx).value().trim().is_empty();
            card()
                .child(ask(t!("board.ask_send_back", who = board::who(t).unwrap_or(t!("board.assignee")), id = id)))
                .child(
                    div()
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(px(8.))
                        .bg(p.code_bg)
                        .border_1()
                        .border_color(p.hair)
                        .text_size(px(BODY))
                        .text_color(p.text)
                        .child(Textarea::new(&shell.board.send_back).appearance(false).bordered(false)),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(btn(p, "send-cancel", t!("board.cancel"), Some("Esc"), false, true).on_click(cx.listener(|this, _, w, cx| this.board_close_flow(w, cx))))
                        .child(btn(p, "send-back-go", t!("board.send_back"), Some("⌘⏎"), true, can && !empty).on_click(cx.listener(|this, _, w, cx| this.board_send_back(w, cx)))),
                )
                .into_any_element()
        }
        Flow::Resume(_) => card()
            .child(ask(t!("board.ask_resume", id = id)))
            .child(
                div()
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .bg(p.code_bg)
                    .border_1()
                    .border_color(p.hair)
                    .text_size(px(BODY))
                    .text_color(p.text)
                    .child(Input::new(&shell.board.resume).appearance(false).bordered(false)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(btn(p, "resume-cancel", t!("board.cancel"), Some("Esc"), false, true).on_click(cx.listener(|this, _, w, cx| this.board_close_flow(w, cx))))
                    .child(btn(p, "resume-go", t!("board.carry_on"), Some("⏎"), true, can).on_click(cx.listener(|this, _, w, cx| this.board_resume(w, cx)))),
            )
            .into_any_element(),
        Flow::Cancel(_) => card()
            .child(ask(t!("board.ask_cancel", id = id)))
            .child(
                h_flex()
                    .gap_2()
                    .child(btn(p, "cancel-yes", t!("board.cancel_ticket"), Some("Y"), true, can).on_click(cx.listener(|this, _, _, cx| this.board_cancel(cx))))
                    .child(btn(p, "cancel-no", t!("board.keep"), Some("N"), false, true).on_click(cx.listener(|this, _, _, cx| {
                        this.board.flow = None;
                        cx.notify();
                    }))),
            )
            .into_any_element(),
    }
}

// ── the diff ──

fn diff_view(shell: &Shell, diff: &DiffUi, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let model = &shell.model;
    let t = model.tickets.iter().find(|t| t.id == diff.ticket);
    let can = shell.can_act();
    let id = diff.ticket;
    let review = t.is_some_and(|t| t.status == TicketStatus::Review);
    let patch = diff.patch.clone();
    let stat = diff.files.as_ref().map(|f| {
        let (a, r) = f.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
        t!("board.diff_stat", n = f.len(), a = a, r = r)
    });
    let bar = h_flex()
        .h(px(44.))
        .flex_none()
        .px_5()
        .gap_3()
        .items_center()
        .border_t_1()
        .border_b_1()
        .border_color(p.hair)
        .child(btn(p, "diff-back", t!("board.back_to_ticket"), Some("Esc"), false, true).on_click(cx.listener(|this, _, _, cx| {
            this.board.diff = None;
            cx.notify();
        })))
        .child(div().flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{id}")))
        .child(div().flex_1().min_w_0().truncate().text_size(px(SMALL)).text_color(p.text2).child(format!("{} · {}", diff.branch, stat.unwrap_or_default())))
        .child(btn(p, "diff-copy", t!("board.copy_patch"), Some("⌘⇧C"), false, !patch.is_empty()).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(patch.clone()))))
        .children(review.then(|| btn(p, "diff-accept", t!("board.accept"), Some("A"), true, can).on_click(cx.listener(|this, _, _, cx| {
            this.board.diff = None;
            this.board_accept(cx);
        }))))
        .children(review.then(|| btn(p, "diff-ask", t!("board.request_changes"), Some("R"), false, can).on_click(cx.listener(move |this, _, w, cx| {
            this.board.diff = None;
            this.board_open_flow(Flow::SendBack(id), w, cx);
        }))));
    let body: AnyElement = match (&diff.files, &diff.error) {
        (_, Some(why)) => div().p_6().text_size(px(BODY)).text_color(p.muted).child(t!("board.diff_failed", why = why)).into_any_element(),
        (None, _) => div().p_6().text_size(px(BODY)).text_color(p.muted).child(t!("board.diff_loading")).into_any_element(),
        (Some(files), _) if files.is_empty() => div().p_6().text_size(px(BODY)).text_color(p.muted).child(t!("board.diff_none")).into_any_element(),
        (Some(files), _) => {
            let list = v_flex()
                .id("diff-files")
                .w(px(260.))
                .flex_none()
                .h_full()
                .overflow_y_scroll()
                .border_r_1()
                .border_color(p.hair)
                .p_2()
                .children(files.iter().enumerate().map(|(i, f)| {
                    h_flex()
                        .id(("diff-file", i))
                        .gap_2()
                        .px_2()
                        .py(px(5.))
                        .rounded(px(6.))
                        .text_size(px(12.))
                        .cursor_pointer()
                        .when(diff.current == i, |d| d.bg(p.wash))
                        .hover(move |s| s.bg(p.wash))
                        .on_click(cx.listener(move |this, _, _, cx| this.diff_goto(i, cx)))
                        .child(div().flex_1().min_w_0().truncate().font_family(MONO).text_color(p.text2).child(f.path.clone()))
                        .child(div().flex_none().text_color(p.muted).child(format!("+{} −{}", f.added, f.removed)))
                }));
            let count = diff.flat.len();
            let lines = uniform_list("diff-lines", count, cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                let p = this.palette;
                let Some(d) = this.board.diff.as_ref() else { return Vec::new() };
                let Some(files) = d.files.as_ref() else { return Vec::new() };
                range.filter_map(|ix| d.flat.get(ix).copied().map(|(f, l)| diff_row(p, files, d, f, l, cx))).collect()
            }))
            .track_scroll(&diff.scroll)
            .flex_1()
            .min_w_0()
            .h_full();
            h_flex().flex_1().min_h_0().child(list).child(lines).into_any_element()
        }
    };
    v_flex().flex_1().min_h_0().child(bar).child(body)
}

const LINE: f32 = 20.;

fn diff_row(p: Palette, files: &[diffs::File], d: &DiffUi, f: usize, l: usize, cx: &mut Context<Shell>) -> AnyElement {
    let file = &files[f];
    if l == usize::MAX {
        let folded = d.folded.contains(&f);
        return h_flex()
            .id(("diff-head", f))
            .w_full()
            .h(px(LINE + 8.))
            .px_4()
            .gap_2()
            .items_center()
            .bg(p.raised)
            .border_b_1()
            .border_color(p.hair)
            .font_family(MONO)
            .text_size(px(12.))
            .text_color(p.text)
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.diff_fold(f, cx)))
            .child(if folded { "▸" } else { "▾" })
            .child(file.path.clone())
            .child(div().text_color(p.muted).font_weight(FontWeight::NORMAL).child(format!("+{} −{}", file.added, file.removed)))
            .into_any_element();
    }
    let line = &file.lines[l];
    let number = |n: Option<u32>| div().w(px(44.)).flex_none().text_align(TextAlign::Right).text_color(p.muted).child(n.map(|n| n.to_string()).unwrap_or_default());
    let (glyph, bg, color) = match line.kind {
        Kind::Add => ("+", Some(p.accent_bg), p.text),
        Kind::Del => ("−", Some(p.wash), p.muted),
        Kind::Hunk => ("", Some(p.wash), p.muted),
        Kind::Context => (" ", None, p.text2),
        Kind::Note => ("", None, p.muted),
    };
    h_flex()
        .w_full()
        .h(px(LINE))
        .gap_2()
        .px_2()
        .items_center()
        .font_family(MONO)
        .text_size(px(12.))
        .when_some(bg, |d, bg| d.bg(bg))
        .child(number(line.old))
        .child(number(line.new))
        .child(div().w(px(10.)).flex_none().text_color(color).child(glyph))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_color(color).when(line.kind == Kind::Del, |d| d.line_through()).child(if line.kind == Kind::Hunk || line.kind == Kind::Note { line.text.clone() } else { line.text.replace('\t', "    ") }))
        .into_any_element()
}

impl Shell {
    fn diff_goto(&mut self, file: usize, cx: &mut Context<Self>) {
        if let Some(d) = self.board.diff.as_mut() {
            d.current = file;
            d.folded.remove(&file);
            d.rebuild();
            if let Some(at) = d.flat.iter().position(|(f, l)| *f == file && *l == usize::MAX) {
                d.scroll.scroll_to_item(at, ScrollStrategy::Top);
            }
        }
        cx.notify();
    }

    fn diff_fold(&mut self, file: usize, cx: &mut Context<Self>) {
        if let Some(d) = self.board.diff.as_mut() {
            if !d.folded.remove(&file) {
                d.folded.insert(file);
            }
            d.rebuild();
        }
        cx.notify();
    }
}
