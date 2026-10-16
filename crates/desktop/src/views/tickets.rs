//! 티켓: every ticket, the person's to shape. A list (the default) or a board
//! of columns by status (V, remembered); in the list each row edits its
//! priority, owner and status in place; on the board a card dragged to
//! another column means what that column would (`crate::tickets::drop_card`)
//! and one dropped where it can't go says why. C writes a new ticket; the
//! page (⏎) edits every field and carries the ticket's record and comments.
//! Each change shows at once and is sent after the five seconds U can take
//! it back in; the daemon checks it again and tells the PM and the member.

use equipe_core::t;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use equipe_core::protocol::{Command, Priority, Ticket, TicketEdit, TicketEntry, TicketId, TicketKind, TicketStatus};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::{Flow, btn, dot, hue_of, kbd, member_color};
use super::icons;
use crate::board::{self as rules, Menu, Tab};
use crate::motion;
use crate::state::{Need, short_span, status_word};
use crate::theme::{GAP, MONO, Palette, R_MD, SMALL, look};
use crate::tickets::{self, Column, Drop, Layout, Sheet, Why};
use crate::tips::TipExt as _;
use crate::Shell;

/// A board column's width, and the gap between columns.
pub const COLUMN_W: f32 = 276.;
const COLUMN_GAP: f32 = 10.;
/// The room a dragged card opens where it would land.
const SLOT_H: f32 = 44.;

/// Which field a small menu edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditKind {
    Priority,
    Assignee,
    Status,
    Project,
    Kind,
    Depends,
}

/// A small menu open over the screen: what it edits, for which tickets (or
/// the new ticket sheet), and the control it opened from.
#[derive(Debug, Clone, PartialEq)]
pub struct EditMenu {
    pub kind: EditKind,
    pub targets: Vec<TicketId>,
    pub for_sheet: bool,
    pub anchor: Bounds<Pixels>,
    pub key: SharedString,
}

/// A card being dragged, and where it would land.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragOver {
    pub id: TicketId,
    pub status: TicketStatus,
    pub before: Option<TicketId>,
    /// The card the pointer is over that set `before`.
    pub by_card: Option<TicketId>,
    pub refused: Option<Why>,
}

/// What travels with a dragged card.
#[derive(Clone)]
pub struct CardDrag {
    pub id: TicketId,
    pub title: String,
    pub status: TicketStatus,
    pub priority: Priority,
    pub palette: Palette,
}

/// The card under the pointer while it is dragged.
struct Ghost(CardDrag);

impl Render for Ghost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let p = self.0.palette;
        let d = &self.0;
        v_flex()
            .w(px(COLUMN_W - 24.))
            .p_3()
            .gap_1()
            .rounded(px(10.))
            .bg(p.popover())
            .border_1()
            .border_color(p.accent)
            .shadow(motion::float_shadow(1.))
            .opacity(0.96)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(priority_icon(p, d.priority, 14.))
                    .child(div().font_family(MONO).text_size(px(11.)).text_color(p.muted).child(format!("T-{}", d.id)))
                    .child(icons::status(p, d.status, 12.)),
            )
            .child(div().text_size(px(SMALL)).line_height(px(19.)).text_color(p.text).line_clamp(2).child(d.title.clone()))
    }
}

pub struct TicketsUi {
    pub menu: Option<EditMenu>,
    /// Where each control that opens a menu was last drawn.
    pub anchors: Rc<RefCell<HashMap<SharedString, Bounds<Pixels>>>>,
    pub drag: Option<DragOver>,
    /// The finished column shows everything, not only the last week.
    pub all_done: bool,
    /// The new ticket sheet (C), with what is picked in it; the words are in the boxes.
    pub sheet: Option<Sheet>,
    pub sheet_title: Entity<InputState>,
    pub sheet_body: Entity<TextareaState>,
    pub sheet_criterion: Entity<InputState>,
    /// The dependency search, in its menu.
    pub dep_search: Entity<InputState>,
    /// A ticket made here, to open once the daemon has numbered it: its
    /// title, and the highest number before it.
    pub awaiting: Option<(String, TicketId)>,
    /// The page's title, and its description, being edited.
    pub title_edit: Option<TicketId>,
    pub title_input: Entity<InputState>,
    pub body_edit: Option<TicketId>,
    pub body_input: Entity<TextareaState>,
    pub comment: Entity<TextareaState>,
    /// The ticket whose record was asked for, as it looked then.
    pub history_asked: Option<(TicketId, u64)>,
    /// Edits sent and not answered yet: the tickets as they were, as shown
    /// meanwhile, and when — put back if the daemon refuses.
    pub sent: HashMap<TicketId, (Ticket, Ticket, Instant)>,
    pub errors_seen: u64,
    /// The views menu (⌥V).
    pub views_open: bool,
    /// Work under way given to someone else: asked once before it is held.
    pub confirm: Option<(Vec<TicketId>, TicketEdit)>,
    /// The quick line (C): a ticket in one line, `@who #project !priority after T-n`.
    pub quick: bool,
    pub quick_input: Entity<InputState>,
    pub board_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl TicketsUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        Self {
            menu: None,
            anchors: Rc::default(),
            drag: None,
            all_done: false,
            sheet: None,
            sheet_title: cx.new(|cx| InputState::new(window, cx).placeholder(t!("tickets.title_placeholder"))),
            sheet_body: cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 12).submit_on_enter(false).placeholder(t!("tickets.body_placeholder"))),
            sheet_criterion: cx.new(|cx| InputState::new(window, cx).placeholder(t!("tickets.criterion_placeholder"))),
            dep_search: cx.new(|cx| InputState::new(window, cx).placeholder(t!("tickets.depends_search"))),
            awaiting: None,
            title_edit: None,
            title_input: cx.new(|cx| InputState::new(window, cx)),
            body_edit: None,
            body_input: cx.new(|cx| TextareaState::new(window, cx).auto_grow(4, 24).submit_on_enter(false)),
            comment: cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 8).submit_on_enter(false).placeholder(t!("tickets.comment_placeholder"))),
            history_asked: None,
            sent: HashMap::new(),
            errors_seen: 0,
            views_open: false,
            confirm: None,
            quick: false,
            quick_input: cx.new(|cx| InputState::new(window, cx).placeholder(t!("tickets.quick_placeholder"))),
            board_scroll: ScrollHandle::new(),
            _subscriptions: Vec::new(),
        }
    }

    /// The language changed: the boxes' placeholders follow.
    pub fn relocalize(&self, window: &mut Window, cx: &mut Context<Shell>) {
        self.sheet_title.update(cx, |s, cx| s.set_placeholder(t!("tickets.title_placeholder"), window, cx));
        self.sheet_body.update(cx, |s, cx| s.set_placeholder(t!("tickets.body_placeholder"), window, cx));
        self.sheet_criterion.update(cx, |s, cx| s.set_placeholder(t!("tickets.criterion_placeholder"), window, cx));
        self.dep_search.update(cx, |s, cx| s.set_placeholder(t!("tickets.depends_search"), window, cx));
        self.comment.update(cx, |s, cx| s.set_placeholder(t!("tickets.comment_placeholder"), window, cx));
        self.quick_input.update(cx, |s, cx| s.set_placeholder(t!("tickets.quick_placeholder"), window, cx));
    }

    /// Whether one of 티켓's boxes has the keyboard.
    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        self.sheet_title.read(cx).focus_handle(cx).is_focused(window)
            || self.sheet_body.read(cx).focus_handle(cx).is_focused(window)
            || self.sheet_criterion.read(cx).focus_handle(cx).is_focused(window)
            || self.dep_search.read(cx).focus_handle(cx).is_focused(window)
            || self.title_input.read(cx).focus_handle(cx).is_focused(window)
            || self.body_input.read(cx).focus_handle(cx).is_focused(window)
            || self.comment.read(cx).focus_handle(cx).is_focused(window)
            || self.quick_input.read(cx).focus_handle(cx).is_focused(window)
    }
}

/// What a menu's row picks.
#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    Priority(Priority),
    Owner(Option<String>),
    Status(TicketStatus),
    Project(Option<PathBuf>),
    Kind(TicketKind),
    Depends(TicketId),
}

// ── what a click or a key does ──

impl Shell {
    /// 티켓 is on screen (모든 일's place, or a saved view of it).
    pub fn tickets_on(&self) -> bool {
        matches!(self.board.view.menu, Menu::All | Menu::View(_))
    }

    pub fn tickets_board(&self) -> bool {
        self.tickets_on() && self.board.view.layout == Layout::Board
    }

    /// Wire 티켓's boxes: Enter and ⌘⏎ where they mean something.
    pub fn tickets_subscriptions(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<Subscription> {
        let tk = &self.board.tk;
        let (title, body, criterion, dep, title_input, body_input, comment) =
            (tk.sheet_title.clone(), tk.sheet_body.clone(), tk.sheet_criterion.clone(), tk.dep_search.clone(), tk.title_input.clone(), tk.body_input.clone(), tk.comment.clone());
        let quick = tk.quick_input.clone();
        vec![
            cx.subscribe_in(&quick, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => cx.notify(),
                // ⇧⌘⏎: the PM shapes it; ⏎ or ⌘⏎: on the board as typed.
                InputEvent::PressEnter { secondary: true, shift: true } => this.shape_quick(window, cx),
                InputEvent::PressEnter { .. } => this.create_quick(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&title, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { secondary: true, .. } => this.create_from_sheet(window, cx),
                InputEvent::PressEnter { .. } => this.board.tk.sheet_body.update(cx, |s, cx| s.focus(window, cx)),
                _ => {}
            }),
            cx.subscribe_in(&body, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { secondary: true, .. }) {
                    this.create_from_sheet(window, cx);
                }
            }),
            cx.subscribe_in(&criterion, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { secondary: true, .. } => this.create_from_sheet(window, cx),
                InputEvent::PressEnter { .. } => this.add_criterion(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&dep, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => {
                    // The first found: added.
                    let words = this.board.tk.dep_search.read(cx).value().to_string();
                    let Some(menu) = this.board.tk.menu.clone() else { return };
                    let (except, have) = this.depends_context_for(&menu);
                    if let Some(id) = tickets::find_tickets(&this.model, &words, except, &have).first().map(|t| t.id) {
                        this.pick_from_menu(Choice::Depends(id), window, cx);
                    }
                }
                _ => {}
            }),
            cx.subscribe_in(&title_input, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_title(window, cx);
                }
            }),
            cx.subscribe_in(&body_input, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { secondary: true, .. }) {
                    this.save_body(window, cx);
                }
            }),
            cx.subscribe_in(&comment, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { secondary: true, .. }) {
                    this.send_comment(window, cx);
                }
            }),
        ]
    }

    /// V: the list or the board, kept for next time.
    pub fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.board.view.layout = layout;
        self.board.tk.menu = None;
        self.board.tk.drag = None;
        self.board.popover = None;
        if self.remembered.tickets_board != (layout == Layout::Board) {
            self.remembered.tickets_board = layout == Layout::Board;
            self.remembered.save(&equipe_providers::credentials::equipe_home());
        }
        self.board_changed(cx);
    }

    // ── menus ──

    /// Open (or, open already from the same control, close) the menu that
    /// edits `kind` of `targets` — or of the new ticket sheet.
    pub fn open_edit_menu(&mut self, kind: EditKind, targets: Vec<TicketId>, key: SharedString, for_sheet: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.board.tk.menu.as_ref().is_some_and(|m| m.key == key && m.kind == kind) {
            self.board.tk.menu = None;
            cx.notify();
            return;
        }
        let anchor = self.board.tk.anchors.borrow().get(&key).copied().unwrap_or_else(|| Bounds::new(point(px(200.), px(120.)), size(px(1.), px(1.))));
        self.board.tk.menu = Some(EditMenu { kind, targets, for_sheet, anchor, key });
        self.board.popover = None;
        self.board.tk.views_open = false;
        if kind == EditKind::Depends {
            self.board.tk.dep_search.update(cx, |s, cx| {
                s.set_value(String::new(), window, cx);
                s.focus(window, cx);
            });
        }
        cx.notify();
    }

    /// The menu's tickets, else the chosen ones, else the pick.
    pub fn edit_targets(&self) -> Vec<TicketId> {
        if !self.board.view.chosen.is_empty() {
            return self.board.view.chosen.iter().copied().collect();
        }
        match self.picked_need() {
            Some(Need::Ticket(id)) => vec![id],
            _ => Vec::new(),
        }
    }

    /// A row of the open menu picked.
    pub fn pick_from_menu(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.board.tk.menu.clone() else { return };
        // Dependencies are a list: the menu stays open while it is built.
        if !matches!(choice, Choice::Depends(_)) {
            self.board.tk.menu = None;
        }
        if menu.for_sheet {
            if let Some(sheet) = self.board.tk.sheet.as_mut() {
                match choice {
                    Choice::Priority(p) => sheet.priority = p,
                    Choice::Owner(o) => sheet.assignee = o,
                    Choice::Project(p) => sheet.project = p,
                    Choice::Kind(k) => sheet.kind = Some(k),
                    Choice::Depends(id) => {
                        if !sheet.depends_on.contains(&id) {
                            sheet.depends_on.push(id);
                        } else {
                            sheet.depends_on.retain(|d| *d != id);
                        }
                        self.board.tk.dep_search.update(cx, |s, cx| s.set_value(String::new(), window, cx));
                    }
                    Choice::Status(_) => {}
                }
            }
            cx.notify();
            return;
        }
        let targets = menu.targets.clone();
        let edit = match choice {
            Choice::Priority(p) => TicketEdit { priority: Some(p), ..Default::default() },
            Choice::Owner(o) => TicketEdit { assignee: Some(o), ..Default::default() },
            Choice::Project(p) => TicketEdit { project: Some(p), ..Default::default() },
            Choice::Kind(k) => TicketEdit { kind: Some(k), ..Default::default() },
            Choice::Depends(id) => {
                let Some(t) = targets.first().and_then(|first| self.model.tickets.iter().find(|t| t.id == *first)) else { return };
                let mut deps = t.depends_on.clone();
                if deps.contains(&id) {
                    deps.retain(|d| *d != id);
                } else {
                    deps.push(id);
                }
                self.board.tk.dep_search.update(cx, |s, cx| s.set_value(String::new(), window, cx));
                TicketEdit { depends_on: Some(deps), ..Default::default() }
            }
            Choice::Status(status) => {
                for id in targets {
                    self.tickets_move(id, status, None, window, cx);
                }
                return;
            }
        };
        self.tickets_edit(targets, edit, cx);
    }

    /// Change `ids` by `edit`: shown at once, sent in five seconds unless U.
    pub fn tickets_edit(&mut self, ids: Vec<TicketId>, edit: TicketEdit, cx: &mut Context<Self>) {
        self.tickets_edit_as(ids, edit, false, cx);
    }

    /// The edit, once the person said yes to stopping work under way.
    pub fn confirm_edit(&mut self, cx: &mut Context<Self>) {
        if let Some((ids, edit)) = self.board.tk.confirm.take() {
            self.tickets_edit_as(ids, edit, true, cx);
        }
    }

    fn tickets_edit_as(&mut self, ids: Vec<TicketId>, edit: TicketEdit, confirmed: bool, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        let (ok, refused) = tickets::bulk_edits(&self.model, &ids, &edit);
        if let Some(why) = refused.first() {
            self.say(why.clone());
        }
        if ok.is_empty() {
            cx.notify();
            return;
        }
        // Work under way going to someone else stops its member: asked first.
        if !confirmed && tickets::stops_work(&ok, &edit) {
            self.board.tk.confirm = Some((ok.iter().map(|(t, _)| t.id).collect(), edit));
            cx.notify();
            return;
        }
        let squad = self.model.squad.clone();
        let ids: Vec<TicketId> = ok.iter().map(|(t, _)| t.id).collect();
        // Several at once go as one, so the PM hears of them once.
        let commands = match ids.as_slice() {
            [one] => vec![Command::EditTicket { squad, ticket: *one, changes: edit.clone() }],
            many => vec![Command::EditTickets { squad, tickets: many.to_vec(), changes: edit.clone() }],
        };
        let text = tickets::edit_words(&ids, &edit);
        self.hold_edit(ok, text, commands, cx);
    }

    /// Show `pairs`' new look now; send `commands` when their time comes.
    pub fn hold_edit(&mut self, pairs: Vec<(Ticket, Ticket)>, text: String, commands: Vec<Command>, cx: &mut Context<Self>) {
        let Some(first) = pairs.first().map(|(t, _)| t.id) else { return };
        let mut before = Vec::new();
        for (was, now) in pairs {
            if let Some(slot) = self.model.tickets.iter_mut().find(|t| t.id == was.id) {
                *slot = now;
            }
            before.push(was);
        }
        self.model.board_rev += 1;
        self.board.undo.push_edit(first, text, commands, before, Instant::now());
        self.ensure_ticker(cx);
        cx.notify();
    }

    /// Move ticket `id` to `to`, before `before` (`None`: last) — what a drop
    /// on the board, or the status menu, does.
    pub fn tickets_move(&mut self, id: TicketId, to: TicketStatus, before: Option<TicketId>, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        let Some(t) = self.model.tickets.iter().find(|t| t.id == id).cloned() else { return };
        match tickets::drop_card(&self.model, &t, to, before) {
            Ok(Drop::Stay) => {}
            Ok(Drop::ConfirmCancel) => {
                // Cancelling is asked once, in the peek's foot.
                self.board.view.selected = Some(Need::Ticket(id));
                self.board.view.peek = true;
                self.board.flow = Some(Flow::Cancel(id));
                cx.notify();
            }
            Ok(Drop::Send { commands, after, what }) => {
                let text = tickets::move_words(&t, what, to);
                self.hold_edit(vec![(t, *after)], text, commands, cx);
            }
            Err(why) => {
                self.say(tickets::why_words(why));
                cx.notify();
            }
        }
    }

    /// A card dragged over column `status`, before card `before` when the
    /// pointer is over one (`by_card`).
    pub fn card_over(&mut self, id: TicketId, status: TicketStatus, before: Option<Option<TicketId>>, by_card: Option<TicketId>, cx: &mut Context<Self>) {
        let refused = self.model.tickets.iter().find(|t| t.id == id).and_then(|t| tickets::person_rule(t, status).err());
        let prev = self.board.tk.drag;
        let mut next = match prev {
            Some(d) if d.id == id && d.status == status => d,
            _ => DragOver { id, status, before: None, by_card: None, refused },
        };
        next.refused = refused;
        if let Some(before) = before {
            next.before = before;
            next.by_card = by_card;
        }
        if prev != Some(next) {
            self.board.tk.drag = Some(next);
            cx.notify();
        }
    }

    /// The pointer over (or off) card `card` of column `status` while `dragged`
    /// is dragged: over its upper half the drop goes before it, over its lower
    /// half before `next`. The room opened above a card for the drop counts as
    /// that card's, so the cards it pushes down don't make it jump.
    #[allow(clippy::too_many_arguments)]
    pub fn card_hover(&mut self, dragged: TicketId, status: TicketStatus, card: TicketId, next: Option<TicketId>, bounds: Bounds<Pixels>, at: Point<Pixels>, cx: &mut Context<Self>) {
        if self.in_cancel_zone(at) {
            return;
        }
        if bounds.contains(&at) {
            let before = if at.y < bounds.center().y { Some(card) } else { next };
            self.card_over(dragged, status, Some(before), Some(card), cx);
            return;
        }
        let hint = self.board.tk.drag;
        let in_room = hint.is_some_and(|d| d.status == status && d.before == Some(card))
            && at.x >= bounds.left()
            && at.x <= bounds.right()
            && at.y < bounds.top()
            && at.y >= bounds.top() - px(SLOT_H + 8.);
        if in_room {
            return;
        }
        // Off the card that placed the drop: last in the column, unless another card says.
        if let Some(d) = self.board.tk.drag.as_mut()
            && d.by_card == Some(card)
        {
            d.by_card = None;
            d.before = None;
            cx.notify();
        }
    }

    /// Whether the pointer is over the place to drop a card to cancel it.
    pub fn in_cancel_zone(&self, at: Point<Pixels>) -> bool {
        self.board.tk.drag.is_some() && self.board.tk.anchors.borrow().get("cancel-zone").is_some_and(|b| b.contains(&at))
    }

    /// A card let go over column `status`.
    pub fn card_dropped(&mut self, id: TicketId, status: TicketStatus, window: &mut Window, cx: &mut Context<Self>) {
        let hint = self.board.tk.drag.filter(|d| d.id == id);
        // Over the cancel place, the column under it doesn't count.
        let status = if hint.is_some_and(|d| d.status == TicketStatus::Cancelled) { TicketStatus::Cancelled } else { status };
        let before = hint.filter(|d| d.status == status).and_then(|d| d.before);
        self.board.tk.drag = None;
        self.board.tk.anchors.borrow_mut().remove("cancel-zone");
        self.board.view.selected = Some(Need::Ticket(id));
        self.tickets_move(id, status, before, window, cx);
        cx.notify();
    }

    // ── the new ticket sheet ──

    /// C: a new ticket — in the project in focus, for the PM to give an owner.
    pub fn open_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        let project = self.model.focus.as_ref().and_then(|name| self.model.projects.iter().find(|p| equipe_core::board::project_name(p) == *name).cloned());
        self.board.tk.sheet = Some(Sheet { project, ..Default::default() });
        self.board.tk.menu = None;
        self.board.popover = None;
        let tk = &self.board.tk;
        for input in [&tk.sheet_title, &tk.sheet_criterion] {
            input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        }
        tk.sheet_body.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        tk.sheet_title.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    pub fn close_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.board.tk.sheet = None;
        self.board.tk.menu = None;
        window.blur(cx);
        cx.notify();
    }

    /// ⏎ in the criterion box: one more line of what makes it done.
    pub fn add_criterion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line = self.board.tk.sheet_criterion.read(cx).value().trim().to_owned();
        if line.is_empty() {
            return;
        }
        if let Some(sheet) = self.board.tk.sheet.as_mut() {
            sheet.criteria.push(line);
        }
        self.board.tk.sheet_criterion.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        cx.notify();
    }

    /// ⌘⏎: the ticket goes on the board, and opens once the daemon numbers it.
    pub fn create_from_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut sheet) = self.board.tk.sheet.clone() else { return };
        sheet.title = self.board.tk.sheet_title.read(cx).value().to_string();
        sheet.description = self.board.tk.sheet_body.read(cx).value().to_string();
        // A criterion typed and not yet added still counts.
        let pending = self.board.tk.sheet_criterion.read(cx).value().trim().to_owned();
        if !pending.is_empty() {
            sheet.criteria.push(pending);
        }
        match sheet.command(&self.model.squad) {
            Ok(command) => {
                let highest = self.model.tickets.iter().map(|t| t.id).max().unwrap_or(0);
                self.board.tk.awaiting = Some((sheet.title.trim().to_owned(), highest));
                self.send_command(command);
                self.say(t!("tickets.creating", title = sheet.title.trim()));
                self.close_sheet(window, cx);
            }
            Err(why) => {
                self.say(why);
                self.board.tk.sheet_title.update(cx, |s, cx| s.focus(window, cx));
                cx.notify();
            }
        }
    }

    /// C: the quick line, at the top of 티켓.
    pub fn open_quick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        self.board.tk.quick = true;
        self.board.tk.menu = None;
        self.board.tk.quick_input.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    pub fn close_quick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.board.tk.quick = false;
        self.board.tk.quick_input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        window.blur(cx);
        cx.notify();
    }

    /// ⌘⏎ on the quick line: the ticket as typed.
    pub fn create_quick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line = self.board.tk.quick_input.read(cx).value().to_string();
        let (sheet, _) = tickets::parse_quick(&line, &self.model);
        match sheet.command(&self.model.squad) {
            Ok(command) => {
                let highest = self.model.tickets.iter().map(|t| t.id).max().unwrap_or(0);
                self.board.tk.awaiting = Some((sheet.title.trim().to_owned(), highest));
                self.send_command(command);
                self.say(t!("tickets.creating", title = sheet.title.trim()));
                self.close_quick(window, cx);
            }
            Err(why) => {
                self.say(why);
                cx.notify();
            }
        }
    }

    /// ⇧⌘⏎ on the quick line: the PM shapes it — criteria, a split — and
    /// brings it back as a proposal to take on.
    pub fn shape_quick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line = self.board.tk.quick_input.read(cx).value().trim().to_owned();
        let Some(pm) = self.model.pm().filter(|_| !line.is_empty() && self.can_act()) else { return };
        self.send_command(Command::Submit { session: pm, text: tickets::shape_request(&line), about: Some(self.model.focus.clone()) });
        self.say(t!("tickets.shaping"));
        self.close_quick(window, cx);
    }

    /// The quick line's words into the full sheet, for what one line can't say.
    pub fn quick_to_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line = self.board.tk.quick_input.read(cx).value().to_string();
        let (sheet, _) = tickets::parse_quick(&line, &self.model);
        self.close_quick(window, cx);
        self.open_sheet(window, cx);
        let title = sheet.title.clone();
        self.board.tk.sheet = Some(Sheet { title: String::new(), ..sheet });
        self.board.tk.sheet_title.update(cx, |s, cx| s.set_value(title, window, cx));
        cx.notify();
    }

    // ── the page, edited in place ──

    pub fn edit_title(&mut self, id: TicketId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.model.tickets.iter().find(|t| t.id == id) else { return };
        let title = t.title.clone();
        self.board.tk.title_edit = Some(id);
        self.board.tk.title_input.update(cx, |s, cx| {
            s.set_value(title, window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    pub fn save_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.board.tk.title_edit.take() else { return };
        let title = self.board.tk.title_input.read(cx).value().trim().to_owned();
        window.blur(cx);
        self.tickets_edit(vec![id], TicketEdit { title: Some(title), ..Default::default() }, cx);
        cx.notify();
    }

    pub fn edit_body(&mut self, id: TicketId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.model.tickets.iter().find(|t| t.id == id) else { return };
        let body = t.body.clone();
        self.board.tk.body_edit = Some(id);
        self.board.tk.body_input.update(cx, |s, cx| {
            s.set_value(body, window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    pub fn save_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.board.tk.body_edit.take() else { return };
        let body = self.board.tk.body_input.read(cx).value().to_string();
        window.blur(cx);
        self.tickets_edit(vec![id], TicketEdit { body: Some(body), ..Default::default() }, cx);
        cx.notify();
    }

    /// ⌘⏎ under the record: the person's words on the ticket, to its member and the PM.
    pub fn send_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.picked() else { return };
        let text = self.board.tk.comment.read(cx).value().trim().to_owned();
        if text.is_empty() || !self.can_act() {
            return;
        }
        self.send_command(Command::CommentTicket { squad: self.model.squad.clone(), ticket: t.id, text: text.clone() });
        // Shown in the record at once; the daemon's record replaces it.
        let entry = TicketEntry { at: self.model.unix, by: "you".into(), kind: "comment".into(), text };
        self.model.histories.entry(t.id).or_default().push(entry);
        self.model.board_rev += 1;
        self.board.tk.comment.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        cx.notify();
    }

    /// Bring 티켓's waiting things up to date: open a ticket just made, settle
    /// edits the daemon answered (or put back those it refused), ask for the
    /// record of the page open.
    pub fn tickets_sync(&mut self, cx: &mut Context<Self>) {
        if let Some((title, highest)) = self.board.tk.awaiting.clone()
            && let Some(id) = self.model.tickets.iter().find(|t| t.id > highest && t.created_by == "you" && t.title == title).map(|t| t.id)
        {
            self.board.tk.awaiting = None;
            self.board_reveal(id);
            self.board.view.page = Some(Tab::Overview);
            self.board.view.peek = true;
        }
        // Refused: what was shown goes back as it was.
        if self.model.squad_errors != self.board.tk.errors_seen {
            self.board.tk.errors_seen = self.model.squad_errors;
            for (id, (was, _, _)) in std::mem::take(&mut self.board.tk.sent) {
                if let Some(slot) = self.model.tickets.iter_mut().find(|t| t.id == id) {
                    *slot = was;
                }
            }
            self.model.board_rev += 1;
        }
        let model = &self.model;
        self.board.tk.sent.retain(|id, (_, shown, at)| at.elapsed().as_secs() < 15 && model.tickets.iter().find(|t| t.id == *id) == Some(shown));
        if let Some(t) = self.picked().filter(|_| self.board.view.page.is_some())
            && self.board.tk.history_asked != Some((t.id, t.updated_at.unwrap_or(0)))
        {
            self.board.tk.history_asked = Some((t.id, t.updated_at.unwrap_or(0)));
            self.send_command(Command::TicketHistory { squad: self.model.squad.clone(), ticket: t.id });
        }
        let _ = cx;
    }

    /// Edits whose time came: sent, and watched until the daemon answers.
    pub fn tickets_sent(&mut self, restore: Vec<Ticket>) {
        let now = Instant::now();
        for was in restore {
            if let Some(shown) = self.model.tickets.iter().find(|t| t.id == was.id).cloned() {
                self.board.tk.sent.insert(was.id, (was, shown, now));
            }
        }
    }

    /// U on an edit shown at once: the tickets as they were.
    pub fn tickets_restore(&mut self, restore: Vec<Ticket>) {
        for was in restore {
            if let Some(slot) = self.model.tickets.iter_mut().find(|t| t.id == was.id) {
                *slot = was;
            }
        }
        self.model.board_rev += 1;
    }

    /// The board's cards, column by column, as the filter and window show them.
    pub fn board_columns(&self) -> Vec<Column> {
        let (query, _) = rules::effective(&self.board.view, &self.saved_views());
        tickets::columns(&self.model, &rules::Filter::parse(&query), self.board.tk.all_done)
    }

    /// h j k l on the board: across columns, up and down a column.
    fn card_step(&mut self, dx: isize, dy: isize, cx: &mut Context<Self>) {
        let cols: Vec<Column> = self.board_columns().into_iter().filter(|c| !c.cards.is_empty()).collect();
        if cols.is_empty() {
            return;
        }
        let at = self.picked_need().and_then(|n| match n {
            Need::Ticket(id) => cols.iter().enumerate().find_map(|(c, col)| col.cards.iter().position(|x| *x == id).map(|r| (c, r))),
            _ => None,
        });
        let (c, r) = at.unwrap_or((0, 0));
        let c = (c as isize + dx).clamp(0, cols.len() as isize - 1) as usize;
        let r = if at.is_none() { 0 } else { (r as isize + dy).clamp(0, cols[c].cards.len() as isize - 1) as usize };
        let r = r.min(cols[c].cards.len() - 1);
        self.board.view.selected = Some(Need::Ticket(cols[c].cards[r]));
        self.board_changed(cx);
    }

    /// 티켓's own keys, before the work screens': its menus' digits, the
    /// sheet's ⌘⏎, V, C, P S O E, and the board's arrows. True when taken.
    pub fn on_tickets_key(&mut self, key: &str, m: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let plain = !(m.control || m.alt || m.platform || m.shift || m.function);
        // Stopping work under way, asked: ⏎ or Y gives it to them, N or Esc keeps it.
        if self.board.tk.confirm.is_some() {
            match key {
                "enter" | "y" => self.confirm_edit(cx),
                "n" | "escape" => {
                    self.board.tk.confirm = None;
                    cx.notify();
                }
                _ => {}
            }
            return true;
        }
        // A menu open: its rows by number.
        if let Some(menu) = self.board.tk.menu.clone()
            && plain
            && let Ok(n @ 1..=9) = key.parse::<usize>()
        {
            let words = self.board.tk.dep_search.read(cx).value().to_string();
            if let Some(choice) = menu_choices(self, &menu, &words).into_iter().filter(|(_, enabled)| *enabled).nth(n - 1).map(|(c, _)| c) {
                self.pick_from_menu(choice, window, cx);
            }
            return true;
        }
        if self.board.tk.sheet.is_some() {
            if m.platform && key == "enter" {
                self.create_from_sheet(window, cx);
            }
            return key != "escape";
        }
        if m.platform && key == "enter" && self.board.tk.body_edit.is_some() {
            self.save_body(window, cx);
            return true;
        }
        if !plain {
            return false;
        }
        // ⇧C: the full sheet; ⇧P: the project of the pick (or the chosen).
        if m.shift && !(m.control || m.alt || m.platform || m.function) {
            match key {
                "c" => {
                    self.open_sheet(window, cx);
                    return true;
                }
                "p" => {
                    let targets = self.edit_targets();
                    if targets.is_empty() {
                        return false;
                    }
                    let key = SharedString::from(format!("{}-{}", kind_code(EditKind::Project), targets[0]));
                    let key = if self.board.tk.anchors.borrow().contains_key(&key) { key } else { SharedString::from("toolbar-new") };
                    self.open_edit_menu(EditKind::Project, targets, key, false, window, cx);
                    return true;
                }
                _ => return false,
            }
        }
        if !plain {
            return false;
        }
        match key {
            "c" => {
                self.open_quick(window, cx);
                true
            }
            "v" if self.tickets_on() => {
                self.set_layout(self.board.view.layout.other(), cx);
                true
            }
            "p" | "s" | "m" | "b" => {
                let targets = self.edit_targets();
                if targets.is_empty() {
                    return false;
                }
                let kind = match key {
                    "p" => EditKind::Priority,
                    "s" => EditKind::Status,
                    "b" => EditKind::Depends,
                    _ => EditKind::Assignee,
                };
                // From the pick's own control when it shows, else the bar's.
                let key = SharedString::from(format!("{}-{}", kind_code(kind), targets[0]));
                let key = if self.board.tk.anchors.borrow().contains_key(&key) { key } else { SharedString::from("toolbar-new") };
                self.open_edit_menu(kind, targets, key, false, window, cx);
                true
            }
            "e" => {
                let Some(Need::Ticket(id)) = self.picked_need() else { return false };
                self.board.view.page = Some(Tab::Overview);
                self.board.view.peek = true;
                self.edit_title(id, window, cx);
                true
            }
            "h" | "left" if self.tickets_board() => {
                self.card_step(-1, 0, cx);
                true
            }
            "l" | "right" if self.tickets_board() => {
                self.card_step(1, 0, cx);
                true
            }
            "j" | "down" if self.tickets_board() && self.board.view.page.is_none() => {
                self.card_step(0, 1, cx);
                true
            }
            "k" | "up" if self.tickets_board() && self.board.view.page.is_none() => {
                self.card_step(0, -1, cx);
                true
            }
            _ => false,
        }
    }

    /// "분류 필요": the proposals alone, the first picked — or back to the list.
    pub fn set_triage(&mut self, on: bool, cx: &mut Context<Self>) {
        self.board.view.triage = on;
        self.board.view.selected = None;
        self.board.view.chosen.clear();
        self.board.flow = None;
        if on {
            self.board.view.layout = Layout::List;
            self.board.view.peek = true;
        }
        self.board_changed(cx);
    }

    /// Esc on 티켓: a menu, the sheet, an edit — true when one was closed.
    pub fn tickets_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.board.view.triage && self.board.flow.is_none() && self.board.tk.menu.is_none() {
            self.set_triage(false, cx);
            return true;
        }
        if self.board.tk.quick {
            self.close_quick(window, cx);
            return true;
        }
        let tk = &mut self.board.tk;
        if tk.confirm.take().is_some() || tk.menu.take().is_some() || std::mem::take(&mut tk.views_open) {
            cx.notify();
            return true;
        }
        if tk.sheet.is_some() {
            self.close_sheet(window, cx);
            return true;
        }
        if tk.title_edit.take().is_some() || tk.body_edit.take().is_some() {
            window.blur(cx);
            cx.notify();
            return true;
        }
        false
    }
}

fn kind_code(kind: EditKind) -> &'static str {
    match kind {
        EditKind::Priority => "prio",
        EditKind::Assignee => "who",
        EditKind::Status => "status",
        EditKind::Project => "project",
        EditKind::Kind => "kind",
        EditKind::Depends => "deps",
    }
}

/// Remember where a control that opens a menu is drawn.
fn anchored<E: gpui_kit::base::ElementExt + 'static>(el: E, anchors: &Rc<RefCell<HashMap<SharedString, Bounds<Pixels>>>>, key: SharedString) -> E {
    let anchors = anchors.clone();
    el.on_prepaint(move |bounds, _, _| {
        anchors.borrow_mut().insert(key.clone(), bounds);
    })
}

// ── drawing: small marks ──

/// A priority, Linear's way: three bars filled to how pressing it is; urgent
/// a filled square with "!".
pub fn priority_icon(p: Palette, priority: Priority, size: f32) -> AnyElement {
    if priority == Priority::Urgent {
        return div()
            .size(px(size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .bg(p.orange)
            .text_color(p.bg)
            .text_size(px(size * 0.72))
            .font_weight(FontWeight::BOLD)
            .child("!")
            .into_any_element();
    }
    let filled = match priority {
        Priority::High => 3,
        Priority::Normal => 2,
        _ => 1,
    };
    let bar = (size / 5.).max(2.);
    h_flex()
        .size(px(size))
        .flex_none()
        .gap(px((size - bar * 3.) / 4.))
        .items_end()
        .justify_center()
        .pb(px(size * 0.12))
        .children((1..=3).map(|i| {
            let h = size * (0.3 + 0.24 * (i as f32 - 1.));
            div().w(px(bar)).h(px(h)).rounded(px(1.)).bg(if i <= filled { p.text2 } else { p.hair })
        }))
        .into_any_element()
}

/// Who does a ticket, as a round mark in their colour; nobody: a dashed ring.
pub fn avatar(p: Palette, model: &crate::state::Model, who: Option<&str>, size: f32) -> AnyElement {
    match who {
        Some(name) => {
            let member = model.members.iter().find(|m| m.name == name || m.role == name);
            let hue = member.map_or_else(|| look(name, name).1, |m| look(&m.role, &m.name).1);
            let color = member.map_or_else(|| p.member(hue), |_| member_color(p, model, name));
            // Its initial, and a number for a second of a role: `F`, `F2`.
            let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            let number: String = name.rsplit_once('-').map(|(_, n)| n).filter(|n| n.chars().all(|c| c.is_ascii_digit())).unwrap_or("").to_owned();
            div()
                .size(px(size))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(Rgba { a: if p.is_dark() { 0.22 } else { 0.16 }, ..color })
                .text_color(color)
                .text_size(px(size * if number.is_empty() { 0.52 } else { 0.42 }))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("{initial}{number}"))
                .into_any_element()
        }
        None => div().size(px(size)).flex_none().rounded_full().border_1().border_color(p.muted).opacity(0.6).into_any_element(),
    }
}

/// The person, as a round mark: the first letter of what they are called here.
fn you_mark(p: Palette, size: f32) -> AnyElement {
    let initial: String = t!("tickets.you").chars().next().map(String::from).unwrap_or_default();
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(p.accent_bg)
        .text_color(p.accent)
        .text_size(px(size * 0.55))
        .font_weight(FontWeight::SEMIBOLD)
        .child(initial)
        .into_any_element()
}

/// A property's control: its look, clickable, opening its menu.
fn pill(p: Palette, id: SharedString, enabled: bool) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h(px(26.))
        .px(px(6.))
        .gap(px(6.))
        .items_center()
        .rounded(px(6.))
        .text_size(px(12.5))
        .text_color(p.text2)
        .when(enabled, |d| d.cursor_pointer().hover(move |s| s.bg(p.wash)))
}

// ── the list's rows ──

/// A ticket's row in 티켓's list, by what it is and where it stands: status
/// (pulsing while its member works) · T-n · title · what it waits on · project
/// · who, and what they are doing now or where it is in their queue · its
/// diff · priority · when it last changed. Status, owner and priority are
/// menus under the pointer; a row waiting for the person is lit.
pub fn row(shell: &Shell, id: TicketId, depth: usize, picked: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let Some(t) = model.tickets.iter().find(|t| t.id == id) else { return div().into_any_element() };
    let need = Need::Ticket(id);
    let group = SharedString::from(format!("trow-{id}"));
    let chosen = shell.board.view.chosen.contains(&id);
    let closed = t.status.is_closed();
    let compact = shell.board.list_width < 760.;
    let narrow = shell.board.list_width < 600.;
    let anchors = &shell.board.tk.anchors;
    let can = shell.can_act();
    let menu_open = |kind: EditKind| shell.board.tk.menu.as_ref().is_some_and(|m| m.kind == kind && m.targets == [id] && !m.for_sheet);
    let open = move |kind: EditKind| {
        move |this: &mut Shell, _: &ClickEvent, window: &mut Window, cx: &mut Context<Shell>| {
            cx.stop_propagation();
            let targets = if this.board.view.chosen.contains(&id) { this.board.view.chosen.iter().copied().collect() } else { vec![id] };
            this.open_edit_menu(kind, targets, SharedString::from(format!("{}-{id}", kind_code(kind))), false, window, cx);
        }
    };
    let control = |kind: EditKind, w: f32, child: AnyElement, cx: &mut Context<Shell>| {
        let key = SharedString::from(format!("{}-{id}", kind_code(kind)));
        let on = menu_open(kind);
        anchored(
            h_flex()
                .id(key.clone())
                .w(px(w))
                .h(px(24.))
                .flex_none()
                .px(px(3.))
                .gap(px(6.))
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .when(on, |d| d.bg(p.wash))
                .when(can, |d| d.cursor_pointer().hover(move |s| s.bg(p.wash)).on_click(cx.listener(open(kind))))
                .child(child),
            anchors,
            key,
        )
    };
    let flags = super::board::row_flags(shell, t);
    let who = rules::who(t).map(str::to_owned);
    let project = model.ticket_project_name(t);
    let changed = tickets::changed_at(model, t);
    let when = (changed > 0).then(|| short_span(model.unix.saturating_sub(changed)));
    let live = model.is_live(t);
    // Waiting for the person, and not acted on yet: lit, as in 나에게.
    let yours = model.needs_you().iter().any(|(_, n)| *n == need);
    // What its member is doing now, how long it has been at it — or its place in their queue.
    let doing: Option<(String, Rgba)> = match t.status {
        TicketStatus::InProgress => {
            let span = model.in_state_for(t).map(short_span);
            let activity = super::board::live_activity(model, t);
            match (activity, span) {
                (Some(a), Some(s)) => Some((format!("{a} · {s}"), p.text2)),
                (Some(a), None) => Some((a, p.text2)),
                (None, Some(s)) => Some((s, p.muted)),
                (None, None) => None,
            }
        }
        TicketStatus::Waiting => rules::first_need(t).map(|n| (n, p.accent)),
        TicketStatus::Todo => tickets::queue_place(model, t).map(|n| (t!("tickets.next_in_queue", n = n), p.muted)),
        _ => None,
    };
    let stat = shell.board.stat(t).filter(|s| !s.files.is_empty()).map(|s| (s.added, s.removed));
    let el = h_flex()
        .id(SharedString::from(format!("ticket-{id}")))
        .group(group.clone())
        .relative()
        .h(px(super::board::ROW + 4.))
        .flex_none()
        .pl_2()
        .pr_3()
        .gap(px(6.))
        .items_center()
        .rounded(px(6.))
        .text_size(px(13.))
        .cursor_pointer()
        .when(yours && !picked, |r| r.bg(p.accent_bg))
        .when(picked, |r| r.bg(p.wash))
        .when(!picked, |r| r.hover(move |s| s.bg(p.wash)))
        .when(yours, |r| r.child(div().absolute().left_0().top(px(8.)).bottom(px(8.)).w(px(2.)).rounded(px(1.)).bg(p.accent)))
        .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
            if e.modifiers().platform {
                if !this.board.view.chosen.remove(&id) {
                    this.board.view.chosen.insert(id);
                }
                cx.notify();
                return;
            }
            if e.click_count() >= 2 {
                this.board.view.page = Some(Tab::Overview);
            }
            this.board_pick(need, cx);
        }))
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
                    this.board.view.anchor = Some(id);
                    cx.notify();
                })),
        )
        .child(
            control(
                EditKind::Status,
                24.,
                div().relative().size(px(14.)).child(icons::status(p, t.status, 14.)).when(live, |d| d.child(div().absolute().top(px(-2.)).right(px(-3.)).child(icons::live_dot(p, shell.board.pulse)))).into_any_element(),
                cx,
            )
            .tip(p, t!("tickets.tip_status", status = crate::state::state_word(t)), Some("S")),
        )
        .child(div().w(px(46.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{id}")))
        .child(
            div()
                .flex_1()
                .min_w(px(120.))
                .truncate()
                .pl(px(depth as f32 * 18.))
                .text_color(if closed { p.muted } else { p.text })
                .when(yours, |d| d.font_weight(FontWeight::MEDIUM))
                .child(if depth > 0 { format!("↳ {}", t.title) } else { t.title.clone() }),
        )
        .child(
            h_flex().flex_none().max_w(px(if compact { 90. } else { 160. })).gap_1().justify_end().overflow_hidden().children(flags.into_iter().enumerate().map(|(i, (text, color, tip))| {
                let chip = div().id(("tflag", i)).flex_none().px(px(6.)).h(px(18.)).flex().items_center().rounded(px(4.)).bg(p.wash).text_size(px(11.)).text_color(color).child(text);
                match tip {
                    Some(tip) => chip.tip(p, tip, None).into_any_element(),
                    None => chip.into_any_element(),
                }
            })),
        )
        .when(!narrow, |r| {
            r.child(
                h_flex()
                    .w(px(if compact { 74. } else { 96. }))
                    .flex_none()
                    .gap(px(6.))
                    .items_center()
                    .child(dot(hue_of(p, model, t).unwrap_or(p.muted), 7.))
                    .child(div().min_w_0().truncate().text_size(px(12.)).text_color(p.muted).child(project.unwrap_or_else(|| t!("board.squad_folder").into()))),
            )
        })
        .child(
            control(
                EditKind::Assignee,
                if compact { 26. } else { 210. },
                h_flex()
                    .w_full()
                    .gap(px(6.))
                    .items_center()
                    .child(avatar(p, model, who.as_deref(), 20.))
                    .when(!compact, |h| {
                        h.child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().truncate().text_size(px(12.)).line_height(px(15.)).text_color(if who.is_some() { p.text2 } else { p.muted }).child(who.clone().unwrap_or_else(|| t!("tickets.pm_decides").into())))
                                .children(doing.clone().map(|(text, color)| div().truncate().text_size(px(11.)).line_height(px(14.)).text_color(color).child(text))),
                        )
                    })
                    .into_any_element(),
                cx,
            )
            .justify_start()
            .tip(p, who.clone().unwrap_or_else(|| t!("tickets.pm_decides").into()), Some("M")),
        )
        .when(!narrow, |r| {
            r.child(
                h_flex()
                    .w(px(64.))
                    .flex_none()
                    .gap_1()
                    .justify_end()
                    .text_size(px(11.5))
                    .font_family(MONO)
                    .children(stat.map(|(added, removed)| [div().text_color(p.accent).child(format!("+{added}")), div().text_color(p.muted).child(format!("−{removed}"))]).into_iter().flatten()),
            )
        })
        .child(control(EditKind::Priority, 24., priority_icon(p, t.priority, 14.), cx).tip(p, t!("tickets.tip_priority", priority = tickets::priority_word(t.priority)), Some("P")))
        .child(div().w(px(32.)).flex_none().text_align(TextAlign::Right).text_size(px(12.)).text_color(p.muted).child(when.unwrap_or_default()));
    el.into_any_element()
}

/// Above 티켓's list: the proposals waiting to be sorted, a click into triage —
/// or, in triage, the way back.
pub fn triage_banner(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    if !shell.tickets_on() {
        return None;
    }
    if shell.board.view.triage {
        return Some(
            h_flex()
                .id("triage-back")
                .h(px(32.))
                .mt_1()
                .px_3()
                .gap_2()
                .items_center()
                .rounded(px(8.))
                .text_size(px(SMALL))
                .text_color(p.text2)
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash))
                .on_click(cx.listener(|this, _, _, cx| this.set_triage(false, cx)))
                .child("←")
                .child(t!("tickets.triage_back"))
                .child(div().flex_1())
                .child(kbd(p, "A"))
                .child(div().text_color(p.muted).child(t!("tickets.triage_take")))
                .child(kbd(p, "N"))
                .child(div().text_color(p.muted).child(t!("tickets.triage_drop")))
                .child(kbd(p, "Esc"))
                .into_any_element(),
        );
    }
    let (query, _) = rules::effective(&shell.board.view, &shell.saved_views());
    let n = rules::proposals(&shell.model, &rules::Filter::parse(&query)).len();
    (n > 0).then(|| {
        h_flex()
            .id("triage-banner")
            .h(px(36.))
            .mt_1()
            .mb_1()
            .px_3()
            .gap_2()
            .items_center()
            .rounded(px(8.))
            .bg(p.accent_bg)
            .text_size(px(SMALL))
            .cursor_pointer()
            .hover(move |s| s.border_1().border_color(p.accent))
            .on_click(cx.listener(|this, _, _, cx| this.set_triage(true, cx)))
            .child(icons::status(p, TicketStatus::Proposed, 14.))
            .child(div().font_weight(FontWeight::SEMIBOLD).text_color(p.accent).child(t!("tickets.triage_banner", n = n)))
            .child(div().text_color(p.text2).child(t!("tickets.triage_banner_hint")))
            .child(div().flex_1())
            .child(div().text_color(p.accent).child("→"))
            .into_any_element()
    })
}

/// The quick line (C): one line, its words read as they are typed.
pub fn quick_bar(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    if !shell.board.tk.quick {
        return None;
    }
    let line = shell.board.tk.quick_input.read(cx).value().to_string();
    let (sheet, tokens) = tickets::parse_quick(&line, &shell.model);
    let chip = |lead: AnyElement, text: String| h_flex().h(px(22.)).px(px(7.)).gap(px(5.)).items_center().rounded(px(6.)).bg(p.wash).text_size(px(12.)).text_color(p.text2).child(lead).child(text);
    let chips: Vec<AnyElement> = tokens
        .iter()
        .map(|t| match t {
            tickets::Token::Owner(name) => chip(avatar(p, &shell.model, Some(name), 14.), name.clone()).into_any_element(),
            tickets::Token::Project(name) => {
                let at = shell.model.project_names().iter().position(|x| x == name);
                chip(dot(at.map_or(p.muted, |i| p.member(crate::theme::PROJECT_HUES[i % crate::theme::PROJECT_HUES.len()])), 7.).into_any_element(), name.clone()).into_any_element()
            }
            tickets::Token::Priority(pr) => chip(priority_icon(p, *pr, 12.), tickets::priority_word(*pr).to_owned()).into_any_element(),
            tickets::Token::After(id) => chip(div().text_color(p.orange).child("⛔").into_any_element(), format!("T-{id}")).into_any_element(),
        })
        .collect();
    let ready = !sheet.title.trim().is_empty() && shell.can_act();
    Some(
        v_flex()
            .mx_3()
            .mt_2()
            .p_2()
            .gap_2()
            .rounded(px(10.))
            .bg(p.raised)
            .border_1()
            .border_color(p.accent)
            .shadow(motion::float_shadow(0.4))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(super::sidebar::icon(IconName::Plus, 15., p.accent))
                    .child(div().flex_1().text_size(px(14.)).child(Input::new(&shell.board.tk.quick_input).appearance(false).bordered(false))),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .children(chips)
                    .when(tokens.is_empty(), |h| h.child(div().text_size(px(12.)).text_color(p.muted).child(t!("tickets.quick_hint"))))
                    .child(div().flex_1())
                    .child(btn(p, "quick-more", t!("tickets.quick_more"), Some("⇧C"), false, true).on_click(cx.listener(|this, _, window, cx| this.quick_to_sheet(window, cx))))
                    .child(btn(p, "quick-shape", t!("tickets.quick_shape"), Some("⇧⌘⏎"), false, ready && shell.model.pm().is_some()).on_click(cx.listener(|this, _, window, cx| this.shape_quick(window, cx))))
                    .child(btn(p, "quick-create", t!("tickets.create"), Some("⌘⏎"), true, ready).on_click(cx.listener(|this, _, window, cx| this.create_quick(window, cx)))),
            )
            .into_any_element(),
    )
}

// ── the board ──

/// The board: a column per status, cards to drag between them.
pub fn board(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let cols = shell.board_columns();
    let dragging = shell.board.tk.drag.filter(|_| cx.has_active_drag());
    let pick = shell.picked_need();
    let columns = h_flex()
        .id("ticket-board")
        .size_full()
        .items_start()
        .overflow_x_scroll()
        .track_scroll(&shell.board.tk.board_scroll)
        .px_3()
        .pt_2()
        .gap(px(COLUMN_GAP))
        .children(cols.into_iter().map(|col| column(shell, col, dragging, pick, cx)))
        .child(div().w(px(8.)).flex_none());
    div().flex_1().min_w_0().h_full().relative().bg(p.bg).child(columns).children(dragging.map(|d| cancel_zone(shell, d, cx))).into_any_element()
}

/// While a card is dragged: a place at the board's foot to drop it to cancel
/// it (asked once, after).
fn cancel_zone(shell: &Shell, drag: DragOver, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let over = drag.status == TicketStatus::Cancelled;
    let refused = drag.refused.filter(|_| over);
    let zone = h_flex()
        .id("cancel-zone")
        .w(px(380.))
        .h(px(52.))
        .gap_2()
        .items_center()
        .justify_center()
        .rounded(px(12.))
        .border_1()
        .border_color(if over { p.danger() } else { p.hair })
        .bg(if over { p.danger_bg() } else { p.popover() })
        .shadow(motion::float_shadow(0.8))
        // Its drop is its own, not the column's under it.
        .occlude()
        .text_size(px(SMALL))
        .text_color(if over { p.danger() } else { p.text2 })
        .on_drag_move::<CardDrag>(cx.listener(move |this, e: &DragMoveEvent<CardDrag>, _, cx| {
            if e.bounds.contains(&e.event.position) {
                let id = e.drag(cx).id;
                this.card_over(id, TicketStatus::Cancelled, Some(None), None, cx);
            }
        }))
        .on_drop(cx.listener(move |this, d: &CardDrag, window, cx| this.card_dropped(d.id, TicketStatus::Cancelled, window, cx)))
        .child(icons::status(p, TicketStatus::Cancelled, 14.))
        .child(refused.map_or_else(|| t!("tickets.drop_to_cancel").to_owned(), |why| tickets::why_words(why).to_owned()));
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(20.))
        .flex()
        .justify_center()
        .child(anchored(zone, &shell.board.tk.anchors, "cancel-zone".into()))
        .into_any_element()
}

fn column(shell: &Shell, col: Column, dragging: Option<DragOver>, pick: Option<Need>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let status = col.status;
    let over = dragging.filter(|d| d.status == status);
    let refused = over.and_then(|d| d.refused);
    let ok_over = over.filter(|d| d.refused.is_none());
    let label = status_word(status);
    let count = col.cards.len() + col.hidden;
    let cards: Vec<TicketId> = col.cards.clone();
    let mut body: Vec<AnyElement> = Vec::new();
    for (i, id) in cards.iter().enumerate() {
        if ok_over.is_some_and(|d| d.before == Some(*id) && d.id != *id) {
            body.push(slot(p, status, Some(*id)));
        }
        let next = cards.get(i + 1).copied();
        body.push(card(shell, *id, status, next, dragging, pick == Some(Need::Ticket(*id)), cx));
    }
    if ok_over.is_some_and(|d| d.before.is_none() || !cards.contains(&d.before.unwrap_or(0))) && ok_over.is_some_and(|d| cards.last() != Some(&d.id) || d.before.is_some()) {
        body.push(slot(p, status, None));
    }
    let empty = cards.is_empty();
    v_flex()
        .id(SharedString::from(format!("col-{}", status.label())))
        .w(px(COLUMN_W))
        .flex_none()
        .h_full()
        .relative()
        .rounded(px(10.))
        .bg(if p.is_dark() { rgba(0xFFFFFF06) } else { rgba(0x0000000A) })
        .when(ok_over.is_some(), |c| c.bg(p.accent_bg))
        .when(refused.is_some(), |c| c.bg(p.danger_bg()))
        .on_drag_move::<CardDrag>(cx.listener(move |this, e: &DragMoveEvent<CardDrag>, _, cx| {
            if e.bounds.contains(&e.event.position) && !this.in_cancel_zone(e.event.position) {
                let id = e.drag(cx).id;
                this.card_over(id, status, None, None, cx);
            }
        }))
        .on_drop(cx.listener(move |this, d: &CardDrag, window, cx| this.card_dropped(d.id, status, window, cx)))
        .child(
            h_flex()
                .h(px(38.))
                .px_3()
                .gap_2()
                .items_center()
                .flex_none()
                .child(icons::status(p, status, 14.))
                .child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(label))
                .child(div().text_size(px(12.)).text_color(p.muted).child(count.to_string()))
                .child(div().flex_1())
                .when(status == TicketStatus::Todo || status == TicketStatus::Proposed, |h| {
                    h.child(
                        div()
                            .id(SharedString::from(format!("col-new-{}", status.label())))
                            .size(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .hover(move |s| s.bg(p.wash))
                            .on_click(cx.listener(|this, _, window, cx| this.open_sheet(window, cx)))
                            .child(super::sidebar::icon(IconName::Plus, 14., p.muted))
                            .tip(p, t!("tickets.new"), Some("C")),
                    )
                }),
        )
        // Why it can't go here: over the column's foot, where the card being
        // dragged doesn't hide it, and without pushing the cards about.
        .children(refused.map(|r| {
            div()
                .absolute()
                .left(px(10.))
                .right(px(10.))
                .bottom(px(16.))
                .px_3()
                .py_2()
                .rounded(px(8.))
                .bg(p.tip_bg())
                .shadow(motion::float_shadow(0.6))
                .text_size(px(12.5))
                .line_height(px(18.))
                .text_color(p.tip_text())
                .child(format!("⊘ {}", tickets::why_words(r)))
        }))
        .child(
            v_flex()
                .id(SharedString::from(format!("col-cards-{}", status.label())))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(6.))
                .pb_3()
                .children(body)
                .when(empty && over.is_none(), |c| c.child(div().pt_2().px_2().text_size(px(12.)).text_color(p.muted).child(t!("tickets.column_empty"))))
                .when(col.hidden > 0, |c| {
                    c.child(
                        div()
                            .id("done-more")
                            .mt_1()
                            .mx_1()
                            .h(px(28.))
                            .px_2()
                            .flex()
                            .items_center()
                            .rounded(px(6.))
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .cursor_pointer()
                            .hover(move |s| s.bg(p.wash))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.board.tk.all_done = true;
                                cx.notify();
                            }))
                            .child(t!("tickets.more_done", n = col.hidden)),
                    )
                }),
        )
        .into_any_element()
}

/// Where a dragged card would land: room opening between the cards.
fn slot(p: Palette, status: TicketStatus, before: Option<TicketId>) -> AnyElement {
    let id = SharedString::from(format!("slot-{}-{}", status.label(), before.unwrap_or(0)));
    div()
        .py(px(3.))
        .child(
            div()
                .h(px(SLOT_H))
                .rounded(px(9.))
                .border_1()
                .border_color(p.accent)
                .bg(p.accent_bg)
                .with_animation(id, Animation::new(motion::BASE).with_easing(motion::ease_out), |d, t| d.h(px(SLOT_H * t)).opacity(t)),
        )
        .into_any_element()
}

/// A card: its priority, number and owner; its title; its project and what
/// it waits on or holds up. Dragged, its place dims and a copy follows.
fn card(shell: &Shell, id: TicketId, status: TicketStatus, next: Option<TicketId>, dragging: Option<DragOver>, picked: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let Some(t) = model.tickets.iter().find(|t| t.id == id) else { return div().into_any_element() };
    let lifted = dragging.is_some_and(|d| d.id == id);
    let who = rules::who(t).map(str::to_owned);
    let project = model.ticket_project_name(t);
    let blocked = model.blocked_by(t).filter(|_| matches!(t.status, TicketStatus::Todo | TicketStatus::Proposed));
    let unblocks = model.unblocks(id).len();
    let live = model.is_live(t);
    let chosen = shell.board.view.chosen.contains(&id);
    let drag = CardDrag { id, title: t.title.clone(), status: t.status, priority: t.priority, palette: p };
    let anchors = &shell.board.tk.anchors;
    let key = SharedString::from(format!("prio-{id}"));
    let who_key = SharedString::from(format!("who-{id}"));
    let activity = super::board::live_activity(model, t);
    // The wrapper's padding is the gap: the pointer is always over a card or the column's end.
    div()
        .id(SharedString::from(format!("card-wrap-{id}")))
        .py(px(3.))
        .on_drag_move::<CardDrag>(cx.listener(move |this, e: &DragMoveEvent<CardDrag>, _, cx| {
            let dragged = e.drag(cx).id;
            this.card_hover(dragged, status, id, next, e.bounds, e.event.position, cx);
        }))
        .child(
            v_flex()
                .id(SharedString::from(format!("card-{id}")))
                .p(px(10.))
                .gap(px(6.))
                .rounded(px(9.))
                .bg(p.raised)
                .border_1()
                .border_color(if picked || chosen { p.accent } else { p.hair })
                .when(picked, |c| c.shadow(motion::float_shadow(0.35)))
                .when(lifted, |c| c.opacity(0.35))
                .cursor_grab()
                .hover(move |s| s.border_color(p.muted))
                .on_drag(drag, |d, _, _, cx| cx.new(|_| Ghost(d.clone())))
                .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                    if e.modifiers().platform {
                        if !this.board.view.chosen.remove(&id) {
                            this.board.view.chosen.insert(id);
                        }
                        cx.notify();
                        return;
                    }
                    if e.click_count() >= 2 {
                        this.board.view.page = Some(Tab::Overview);
                    }
                    this.board_pick(Need::Ticket(id), cx);
                }))
                .child(
                    h_flex()
                        .gap(px(6.))
                        .items_center()
                        .child(anchored(
                            div()
                                .id(key.clone())
                                .size(px(20.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.))
                                .cursor_pointer()
                                .hover(move |s| s.bg(p.wash))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.open_edit_menu(EditKind::Priority, vec![id], SharedString::from(format!("prio-{id}")), false, window, cx);
                                }))
                                .child(priority_icon(p, t.priority, 13.)),
                            anchors,
                            key,
                        ))
                        .child(div().font_family(MONO).text_size(px(11.)).text_color(p.muted).child(format!("T-{id}")))
                        .when(live, |h| h.child(icons::live_dot(p, shell.board.pulse)))
                        .child(div().flex_1())
                        .child(anchored(
                            div()
                                .id(who_key.clone())
                                .rounded_full()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.open_edit_menu(EditKind::Assignee, vec![id], SharedString::from(format!("who-{id}")), false, window, cx);
                                }))
                                .child(avatar(p, model, who.as_deref(), 20.))
                                .tip(p, who.clone().unwrap_or_else(|| t!("tickets.pm_decides").into()), Some("O")),
                            anchors,
                            who_key,
                        )),
                )
                .child(div().text_size(px(SMALL)).line_height(px(19.)).text_color(if t.status.is_closed() { p.muted } else { p.text }).line_clamp(3).child(t.title.clone()))
                .children(activity.filter(|_| t.status == TicketStatus::InProgress).map(|a| div().text_size(px(11.5)).text_color(p.text2).truncate().child(a)))
                .child(
                    h_flex()
                        .gap(px(6.))
                        .items_center()
                        .flex_wrap()
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .when(t.status.is_closed() && status == TicketStatus::Done, |h| h.child(icons::status(p, t.status, 11.)))
                        .child(h_flex().gap(px(5.)).items_center().min_w_0().child(dot(hue_of(p, model, t).unwrap_or(p.muted), 6.)).child(div().truncate().child(project.unwrap_or_else(|| t!("board.squad_folder").into()))))
                        .when_some(blocked, |h, dep| h.child(super::ticket_card::hover_link(div().id(("card-blocked", id as u64)).px(px(5.)).rounded(px(4.)).bg(p.wash).text_color(p.orange).child(format!("⛔ T-{dep}")), dep, super::ticket_card::Side::Below)))
                        .when(unblocks > 0 && !t.status.is_closed(), |h| h.child(div().px(px(5.)).rounded(px(4.)).bg(p.wash).child(format!("→ {unblocks}"))))
                        .when(t.not_on_main(), |h| h.child(div().px(px(5.)).rounded(px(4.)).bg(p.wash).text_color(p.accent).child(t!("state.chip_not_on_main")))),
                ),
        )
        .into_any_element()
}

// ── the menus ──

/// The rows a menu offers, each with whether it can be picked.
fn menu_choices(shell: &Shell, menu: &EditMenu, words: &str) -> Vec<(Choice, bool)> {
    let model = &shell.model;
    match menu.kind {
        EditKind::Priority => Priority::ALL.into_iter().map(|p| (Choice::Priority(p), true)).collect(),
        EditKind::Assignee => std::iter::once((Choice::Owner(None), true)).chain(tickets::owners(model).into_iter().map(|(name, _)| (Choice::Owner(Some(name)), true))).collect(),
        EditKind::Status => {
            let Some(t) = menu.targets.first().and_then(|id| model.tickets.iter().find(|t| t.id == *id)) else { return Vec::new() };
            tickets::status_choices(t).into_iter().map(|(s, why)| (Choice::Status(s), why.is_none())).collect()
        }
        EditKind::Project => model.projects.iter().cloned().map(|p| (Choice::Project(Some(p)), true)).chain([(Choice::Project(None), true)]).collect(),
        EditKind::Kind => [TicketKind::Code, TicketKind::Task, TicketKind::Action].into_iter().map(|k| (Choice::Kind(k), true)).collect(),
        EditKind::Depends => {
            let (except, have) = shell.depends_context_for(menu);
            let mut out: Vec<(Choice, bool)> = have.iter().map(|d| (Choice::Depends(*d), true)).collect();
            out.extend(tickets::find_tickets(model, words, except, &have).into_iter().map(|t| (Choice::Depends(t.id), true)));
            out
        }
    }
}

impl Shell {
    /// The ticket a dependency menu edits, and what it waits on already.
    fn depends_context_for(&self, menu: &EditMenu) -> (Option<TicketId>, Vec<TicketId>) {
        if menu.for_sheet {
            return (None, self.board.tk.sheet.as_ref().map(|s| s.depends_on.clone()).unwrap_or_default());
        }
        let first = menu.targets.first().copied();
        let have = first.and_then(|id| self.model.tickets.iter().find(|t| t.id == id)).map(|t| t.depends_on.clone()).unwrap_or_default();
        (first, have)
    }
}

/// Is `choice` what the menu's ticket (or the sheet) has now?
fn current(shell: &Shell, menu: &EditMenu, choice: &Choice) -> bool {
    if menu.for_sheet {
        let Some(s) = &shell.board.tk.sheet else { return false };
        return match choice {
            Choice::Priority(p) => s.priority == *p,
            Choice::Owner(o) => s.assignee == *o,
            Choice::Project(p) => s.project == *p,
            Choice::Kind(k) => s.kind == Some(*k),
            Choice::Depends(id) => s.depends_on.contains(id),
            Choice::Status(_) => false,
        };
    }
    let all: Vec<&Ticket> = menu.targets.iter().filter_map(|id| shell.model.tickets.iter().find(|t| t.id == *id)).collect();
    !all.is_empty()
        && all.iter().all(|t| match choice {
            Choice::Priority(p) => t.priority == *p,
            Choice::Owner(o) => rules::who(t) == o.as_deref(),
            Choice::Project(p) => t.project == *p,
            Choice::Kind(k) => t.kind == *k,
            Choice::Depends(id) => t.depends_on.contains(id),
            Choice::Status(s) => t.status == *s,
        })
}

/// The menu open, at its control.
pub fn edit_menu(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let menu = shell.board.tk.menu.clone()?;
    let words = shell.board.tk.dep_search.read(cx).value().to_string();
    let choices = menu_choices(shell, &menu, &words);
    let model = &shell.model;
    let status_of = menu.targets.first().and_then(|id| model.tickets.iter().find(|t| t.id == *id)).cloned();
    let mut n = 0;
    let rows: Vec<AnyElement> = choices
        .into_iter()
        .enumerate()
        .map(|(ix, (choice, enabled))| {
            if enabled {
                n += 1;
            }
            let number = enabled.then_some(n).filter(|n| *n <= 9);
            let on = current(shell, &menu, &choice);
            let (lead, label, hint): (AnyElement, String, Option<String>) = match &choice {
                Choice::Priority(pr) => (priority_icon(p, *pr, 14.), tickets::priority_word(*pr).to_owned(), None),
                Choice::Owner(None) => (avatar(p, model, None, 18.), t!("tickets.pm_decides").to_owned(), None),
                Choice::Owner(Some(name)) => {
                    let role = model.members.iter().find(|m| m.name == *name).map(|m| m.role.clone()).filter(|r| r != name);
                    (avatar(p, model, Some(name), 18.), name.clone(), role)
                }
                Choice::Status(s) => {
                    let why = status_of.as_ref().and_then(|t| tickets::person_rule(t, *s).err().filter(|_| t.status != *s));
                    (icons::status(p, *s, 14.).into_any_element(), status_word(*s).to_owned(), why.map(|r| tickets::why_words(r).to_owned()))
                }
                Choice::Project(Some(path)) => {
                    let name = equipe_core::board::project_name(path);
                    let at = model.project_names().iter().position(|x| *x == name);
                    (dot(at.map_or(p.muted, |i| p.member(crate::theme::PROJECT_HUES[i % crate::theme::PROJECT_HUES.len()])), 8.).into_any_element(), name, None)
                }
                Choice::Project(None) => (dot(p.muted, 8.).into_any_element(), t!("board.squad_folder").to_owned(), None),
                Choice::Kind(k) => (div().w(px(14.)).into_any_element(), equipe_core::i18n::label::kind(crate::i18n::lang(), *k).to_owned(), Some(kind_hint(*k).to_owned())),
                Choice::Depends(id) => {
                    let t = model.tickets.iter().find(|t| t.id == *id);
                    (t.map_or_else(|| div().into_any_element(), |t| icons::status(p, t.status, 12.).into_any_element()), format!("T-{id}  {}", t.map(|t| t.title.clone()).unwrap_or_default()), None)
                }
            };
            let pick = choice.clone();
            h_flex()
                .id(("edit-menu-row", ix))
                .min_h(px(30.))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(6.))
                .text_size(px(SMALL))
                .when(!enabled, |r| r.opacity(0.45))
                .when(enabled, |r| r.cursor_pointer().hover(move |s| s.bg(p.wash)).on_click(cx.listener(move |this, _, window, cx| this.pick_from_menu(pick.clone(), window, cx))))
                .child(div().w(px(18.)).flex_none().flex().justify_center().child(lead))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .py(px(4.))
                        .child(div().truncate().text_color(p.text).child(label))
                        .children(hint.map(|h| div().text_size(px(11.5)).line_height(px(15.)).text_color(p.muted).child(h))),
                )
                .child(div().w(px(14.)).flex_none().text_color(p.accent).child(if on { "✓" } else { "" }))
                .children(number.map(|n| div().w(px(14.)).flex_none().text_size(px(11.)).text_color(p.muted).child(n.to_string())))
                .into_any_element()
        })
        .collect();
    let title = match menu.kind {
        EditKind::Priority => t!("tickets.prop_priority"),
        EditKind::Assignee => t!("tickets.prop_assignee"),
        EditKind::Status => t!("tickets.prop_status"),
        EditKind::Project => t!("tickets.prop_project"),
        EditKind::Kind => t!("tickets.prop_kind"),
        EditKind::Depends => t!("tickets.prop_depends"),
    };
    let anchor = menu.anchor;
    let many = menu.targets.len() > 1;
    let surface = v_flex()
        .id("edit-menu")
        .w(px(if menu.kind == EditKind::Depends || menu.kind == EditKind::Status { 300. } else { 240. }))
        .max_h(px(420.))
        .overflow_y_scroll()
        .p_1()
        .rounded(px(R_MD))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .text_color(p.text)
        .child(
            div()
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child(if many { format!("{title} · {}", t!("board.chosen", n = menu.targets.len())) } else { title.to_owned() }),
        )
        .when(menu.kind == EditKind::Depends, |s| {
            s.child(div().mx_1().mb_1().px_2().h(px(30.)).flex().items_center().rounded(px(6.)).bg(p.code_bg).border_1().border_color(p.hair).text_size(px(SMALL)).child(Input::new(&shell.board.tk.dep_search).appearance(false).bordered(false)))
        })
        .children(rows)
        .on_mouse_down_out(cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            if !anchor.contains(&e.position) {
                this.board.tk.menu = None;
                cx.notify();
            }
        }));
    let positioner = gpui_kit::base::Positioner::side(anchor).placement(gpui_kit::base::Placement::Bottom).align(gpui_kit::base::Align::Start).offset(px(GAP)).margin(px(8.)).occlude();
    Some(deferred(positioner.child(motion::pop(surface, ("edit-menu-in", menu.kind as u64), false))).with_priority(3).into_any_element())
}

fn kind_hint(kind: TicketKind) -> &'static str {
    match kind {
        TicketKind::Code => t!("tickets.kind_code_hint"),
        TicketKind::Task => t!("tickets.kind_task_hint"),
        TicketKind::Action => t!("tickets.kind_action_hint"),
    }
}

// ── the toolbar's parts ──

/// List | Board, the layout in use pressed.
pub fn layout_toggle(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let now = shell.board.view.layout;
    let seg = |layout: Layout, icon: IconName, cx: &mut Context<Shell>| {
        let on = now == layout;
        h_flex()
            .id(SharedString::from(format!("layout-{}", layout.code())))
            .h(px(24.))
            .px_2()
            .gap(px(5.))
            .items_center()
            .rounded(px(5.))
            .text_size(px(12.5))
            .when(on, |d| d.bg(p.raised).text_color(p.text).shadow_xs())
            .when(!on, |d| d.text_color(p.muted).cursor_pointer().hover(move |s| s.text_color(p.text)))
            .on_click(cx.listener(move |this, _, _, cx| this.set_layout(layout, cx)))
            .child(super::sidebar::icon(icon, 13., if on { p.text } else { p.muted }))
            .child(layout.label())
            .tip(p, t!("tickets.tip_layout"), Some("V"))
    };
    h_flex()
        .flex_none()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(7.))
        .bg(p.wash)
        .child(seg(Layout::List, IconName::Menu, cx))
        .child(seg(Layout::Board, IconName::LayoutDashboard, cx))
        .into_any_element()
}

/// The views menu's button (⌥V): every view, kept ones taken out with ×,
/// and keeping what is listed now.
pub fn views_button(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let views = shell.saved_views();
    let name = match shell.board.view.menu {
        Menu::View(at) => views.get(at).map(|v| v.name.clone()).unwrap_or_default(),
        _ => t!("tickets.all_tickets").to_owned(),
    };
    h_flex()
        .id("views")
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
        .when(shell.board.tk.views_open, |d| d.bg(p.wash))
        .on_click(cx.listener(|this, _, _, cx| {
            this.board.tk.views_open = !this.board.tk.views_open;
            this.board.tk.menu = None;
            this.board.popover = None;
            cx.notify();
        }))
        .child(div().text_color(p.muted).child(t!("tickets.view")))
        .child(div().max_w(px(140.)).truncate().child(name))
        .child(super::sidebar::icon(IconName::ChevronDown, 12., p.muted))
        .tip(p, t!("tickets.tip_views"), Some("⌥V"))
        .into_any_element()
}

pub fn views_menu(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    if !shell.board.tk.views_open {
        return None;
    }
    let views = shell.saved_views();
    let builtin = rules::builtin_views().len();
    let menu = shell.board.view.menu;
    let item = |id: SharedString, label: String, on: bool| {
        h_flex()
            .id(id.clone())
            .group(id)
            .h(px(30.))
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(SMALL))
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .child(div().w(px(12.)).text_color(p.accent).child(if on { "✓" } else { "" }))
            .child(div().flex_1().truncate().child(label))
    };
    let mut rows: Vec<AnyElement> = vec![item("view-all".into(), t!("tickets.all_tickets").into(), menu == Menu::All)
        .on_click(cx.listener(|this, _, _, cx| {
            this.board.tk.views_open = false;
            this.open_side_menu(Menu::All, cx);
        }))
        .into_any_element()];
    for (at, v) in views.iter().enumerate() {
        let kept = at >= builtin;
        let group = SharedString::from(format!("view-{at}"));
        rows.push(
            item(group.clone(), v.name.clone(), menu == Menu::View(at))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.board.tk.views_open = false;
                    this.open_side_menu(Menu::View(at), cx);
                }))
                .when(kept, |r| {
                    r.child(
                        div()
                            .id(SharedString::from(format!("view-forget-{at}")))
                            .px_1()
                            .rounded(px(4.))
                            .text_color(p.muted)
                            .invisible()
                            .group_hover(group.clone(), |s| s.visible())
                            .hover(move |s| s.bg(p.hair))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.board_forget_view(at, cx);
                            }))
                            .child("×"),
                    )
                })
                .into_any_element(),
        );
    }
    rows.push(div().my_1().h(px(1.)).bg(p.hair).into_any_element());
    rows.push(
        item("view-save".into(), t!("tickets.save_view").into(), false)
            .child(kbd(p, "⏎"))
            .on_click(cx.listener(|this, _, window, cx| {
                this.board.tk.views_open = false;
                this.board.flow = Some(Flow::SaveView);
                this.board.naming.update(cx, |s, cx| {
                    s.set_value(String::new(), window, cx);
                    s.focus(window, cx);
                });
                cx.notify();
            }))
            .into_any_element(),
    );
    Some(
        deferred(
            v_flex()
                .id("views-menu")
                .absolute()
                .top(px(44.))
                .left(px(shell.header_inset(16., cx) + 120.))
                .w(px(240.))
                .p_1()
                .rounded(px(R_MD))
                .bg(p.popover())
                .border_1()
                .border_color(p.hair)
                .shadow(motion::float_shadow(1.))
                .occlude()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.board.tk.views_open = false;
                    cx.notify();
                }))
                .children(rows),
        )
        .into_any_element(),
    )
}

/// "+ 새 티켓".
pub fn new_button(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    anchored(
        btn(p, "toolbar-new", t!("tickets.new"), Some("C"), true, shell.can_act()).on_click(cx.listener(|this, _, window, cx| this.open_quick(window, cx))),
        &shell.board.tk.anchors,
        "toolbar-new".into(),
    )
    .into_any_element()
}

/// The bulk bar's edits: owner, project, priority, status, cancel.
pub fn bulk_tools(shell: &Shell, cx: &mut Context<Shell>) -> Vec<AnyElement> {
    let p = shell.palette;
    let can = shell.can_act();
    let tool = |kind: EditKind, label: &'static str, cx: &mut Context<Shell>| {
        let key = SharedString::from(format!("bulk-{}", kind_code(kind)));
        anchored(
            btn(p, key.clone(), label, None, false, can).on_click(cx.listener(move |this, _, window, cx| {
                let targets: Vec<TicketId> = this.board.view.chosen.iter().copied().collect();
                this.open_edit_menu(kind, targets, SharedString::from(format!("bulk-{}", kind_code(kind))), false, window, cx);
            })),
            &shell.board.tk.anchors,
            key,
        )
        .into_any_element()
    };
    vec![
        tool(EditKind::Assignee, t!("tickets.prop_assignee"), cx),
        tool(EditKind::Project, t!("tickets.prop_project"), cx),
        tool(EditKind::Priority, t!("tickets.prop_priority"), cx),
        tool(EditKind::Status, t!("tickets.prop_status"), cx),
        btn(p, "bulk-cancel", t!("tickets.bulk_cancel"), Some("N"), false, can).on_click(cx.listener(|this, _, _, cx| this.board_no(cx))).into_any_element(),
    ]
}

// ── the new ticket sheet ──

pub fn sheet(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let s = shell.board.tk.sheet.clone()?;
    let model = &shell.model;
    let anchors = &shell.board.tk.anchors;
    let prop = |kind: EditKind, lead: AnyElement, label: String, cx: &mut Context<Shell>| {
        let key = SharedString::from(format!("sheet-{}", kind_code(kind)));
        anchored(
            pill(p, key.clone(), true)
                .border_1()
                .border_color(p.hair)
                .on_click(cx.listener(move |this, _, window, cx| this.open_edit_menu(kind, Vec::new(), SharedString::from(format!("sheet-{}", kind_code(kind))), true, window, cx)))
                .child(lead)
                .child(label),
            anchors,
            key,
        )
        .into_any_element()
    };
    let project_name = s.project.as_deref().map(equipe_core::board::project_name);
    let project_dot = project_name.as_ref().and_then(|n| model.project_names().iter().position(|x| x == n)).map_or(p.muted, |i| p.member(crate::theme::PROJECT_HUES[i % crate::theme::PROJECT_HUES.len()]));
    let deps_label = if s.depends_on.is_empty() { t!("tickets.prop_depends").to_owned() } else { s.depends_on.iter().map(|d| format!("T-{d}")).collect::<Vec<_>>().join(", ") };
    let kind_label = s.kind.map_or_else(|| t!("tickets.kind_auto").to_owned(), |k| equipe_core::i18n::label::kind(crate::i18n::lang(), k).to_owned());
    let criteria: Vec<AnyElement> = s
        .criteria
        .iter()
        .enumerate()
        .map(|(i, c)| {
            h_flex()
                .id(("criterion", i))
                .group(SharedString::from(format!("criterion-{i}")))
                .h(px(26.))
                .gap_2()
                .items_center()
                .text_size(px(SMALL))
                .child(div().size(px(13.)).flex_none().rounded(px(3.)).border_1().border_color(p.muted))
                .child(div().flex_1().min_w_0().truncate().text_color(p.text).child(c.clone()))
                .child(
                    div()
                        .id(("criterion-x", i))
                        .px_1()
                        .rounded(px(4.))
                        .text_color(p.muted)
                        .cursor_pointer()
                        .invisible()
                        .group_hover(SharedString::from(format!("criterion-{i}")), |s| s.visible())
                        .hover(move |s| s.bg(p.wash))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(s) = this.board.tk.sheet.as_mut()
                                && i < s.criteria.len()
                            {
                                s.criteria.remove(i);
                            }
                            cx.notify();
                        }))
                        .child("×"),
                )
                .into_any_element()
        })
        .collect();
    let panel = v_flex()
        .id("ticket-sheet")
        .w(px(640.))
        .max_h(px(620.))
        .rounded(px(14.))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .occlude()
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            h_flex()
                .px_5()
                .pt_4()
                .gap_2()
                .items_center()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(div().px(px(6.)).py(px(2.)).rounded(px(5.)).bg(p.wash).child(model.squad.clone()))
                .child("›")
                .child(t!("tickets.new_title"))
                .child(div().flex_1())
                .child(
                    div()
                        .id("sheet-close")
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(p.wash))
                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx)))
                        .child(super::sidebar::icon(IconName::Close, 14., p.muted)),
                ),
        )
        .child(
            v_flex()
                .id("sheet-body")
                .max_h(px(460.))
                .flex_none()
                .overflow_y_scroll()
                .px_5()
                .pt_2()
                .pb_3()
                .gap_3()
                .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(Input::new(&shell.board.tk.sheet_title).appearance(false).bordered(false).text_size(px(19.))))
                .child(div().text_size(px(14.)).line_height(px(22.)).text_color(p.text2).child(Textarea::new(&shell.board.tk.sheet_body).appearance(false).bordered(false)))
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("tickets.criteria_heading")))
                        .children(criteria)
                        .child(
                            h_flex()
                                .h(px(28.))
                                .gap_2()
                                .items_center()
                                .text_size(px(SMALL))
                                .child(super::sidebar::icon(IconName::Plus, 13., p.muted))
                                .child(div().flex_1().child(Input::new(&shell.board.tk.sheet_criterion).appearance(false).bordered(false))),
                        ),
                ),
        )
        .child(
            h_flex()
                .px_5()
                .py_3()
                .gap_2()
                .flex_wrap()
                .border_t_1()
                .border_color(p.hair)
                .child(prop(EditKind::Priority, priority_icon(p, s.priority, 13.), tickets::priority_word(s.priority).to_owned(), cx))
                .child(prop(EditKind::Assignee, avatar(p, model, s.assignee.as_deref(), 16.), s.assignee.clone().unwrap_or_else(|| t!("tickets.pm_decides").into()), cx))
                .child(prop(EditKind::Project, dot(project_dot, 7.).into_any_element(), project_name.unwrap_or_else(|| t!("tickets.project_auto").into()), cx))
                .child(prop(EditKind::Depends, div().text_color(p.muted).child("⛓").into_any_element(), deps_label, cx))
                .child(prop(EditKind::Kind, div().w(px(0.)).into_any_element(), kind_label, cx)),
        )
        .child(
            h_flex()
                .px_5()
                .py_3()
                .gap_2()
                .items_center()
                .border_t_1()
                .border_color(p.hair)
                .child(div().flex_1().text_size(px(12.)).text_color(p.muted).child(t!("tickets.sheet_hint")))
                .child(btn(p, "sheet-cancel", t!("peek.cancel"), Some("Esc"), false, true).on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))))
                .child(btn(p, "sheet-create", t!("tickets.create"), Some("⌘⏎"), true, shell.can_act()).on_click(cx.listener(|this, _, window, cx| this.create_from_sheet(window, cx)))),
        );
    Some(
        deferred(
            div()
                .id("sheet-scrim")
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .flex()
                .justify_center().items_start()
                .pt(px(72.))
                .bg(rgba(0x0000006B))
                .occlude()
                .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx)))
                .child(motion::dialog(panel, "sheet-in")),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

// ── the page: properties, record, comments ──

/// The page's (and the peek's) properties, each a menu.
pub fn properties(shell: &Shell, t: &Ticket, compact: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let id = t.id;
    let anchors = &shell.board.tk.anchors;
    let can = shell.can_act();
    let row = |label: &'static str, value: AnyElement| {
        h_flex()
            .min_h(px(30.))
            .gap_2()
            .items_center()
            .child(div().w(px(if compact { 72. } else { 88. })).flex_none().text_size(px(12.)).text_color(p.muted).child(label))
            .child(div().flex_1().min_w_0().child(value))
    };
    let prop = |kind: EditKind, lead: AnyElement, label: String, cx: &mut Context<Shell>| {
        let key = SharedString::from(format!("page-{}-{id}", kind_code(kind)));
        anchored(
            pill(p, key.clone(), can)
                .when(can, |d| d.on_click(cx.listener(move |this, _, window, cx| this.open_edit_menu(kind, vec![id], SharedString::from(format!("page-{}-{id}", kind_code(kind))), false, window, cx))))
                .child(lead)
                .child(div().truncate().child(label)),
            anchors,
            key,
        )
        .into_any_element()
    };
    let who = rules::who(t).map(str::to_owned);
    let project = model.ticket_project_name(t);
    let mut rows = vec![
        row(t!("tickets.prop_status"), prop(EditKind::Status, icons::status(p, t.status, 13.).into_any_element(), crate::state::state_word(t).to_owned(), cx)),
        row(t!("tickets.prop_priority"), prop(EditKind::Priority, priority_icon(p, t.priority, 13.), tickets::priority_word(t.priority).to_owned(), cx)),
        row(t!("tickets.prop_assignee"), prop(EditKind::Assignee, avatar(p, model, who.as_deref(), 16.), who.unwrap_or_else(|| t!("tickets.pm_decides").into()), cx)),
        row(t!("tickets.prop_project"), prop(EditKind::Project, dot(hue_of(p, model, t).unwrap_or(p.muted), 7.).into_any_element(), project.unwrap_or_else(|| t!("board.squad_folder").into()), cx)),
    ];
    if !compact {
        rows.push(row(t!("tickets.prop_kind"), prop(EditKind::Kind, div().into_any_element(), equipe_core::i18n::label::kind(crate::i18n::lang(), t.kind).to_owned(), cx)));
    }
    let link = |dep: TicketId, removable: bool, cx: &mut Context<Shell>| {
        let found = model.tickets.iter().find(|x| x.id == dep);
        let row = h_flex().id(SharedString::from(format!("prop-link-{id}-{dep}-{removable}")));
        super::ticket_card::hover_link(row, dep, super::ticket_card::Side::Below)
            .group(SharedString::from(format!("prop-link-{dep}")))
            .h(px(26.))
            .px(px(6.))
            .gap(px(6.))
            .items_center()
            .rounded(px(6.))
            .text_size(px(12.5))
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.board_reveal(dep);
                this.board_changed(cx);
            }))
            .children(found.map(|d| icons::status(p, d.status, 12.)))
            .child(div().flex_none().font_family(MONO).text_size(px(11.5)).text_color(p.muted).child(format!("T-{dep}")))
            .child(div().flex_1().min_w_0().truncate().text_color(p.text2).child(found.map(|d| d.title.clone()).unwrap_or_default()))
            .when(removable && can, |r| {
                r.child(
                    div()
                        .id(SharedString::from(format!("prop-unlink-{id}-{dep}")))
                        .px_1()
                        .rounded(px(4.))
                        .text_color(p.muted)
                        .invisible()
                        .group_hover(SharedString::from(format!("prop-link-{dep}")), |s| s.visible())
                        .hover(move |s| s.bg(p.hair))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            let Some(t) = this.model.tickets.iter().find(|t| t.id == id) else { return };
                            let deps: Vec<TicketId> = t.depends_on.iter().copied().filter(|d| *d != dep).collect();
                            this.tickets_edit(vec![id], TicketEdit { depends_on: Some(deps), ..Default::default() }, cx);
                        }))
                        .child("×"),
                )
            })
            .into_any_element()
    };
    let deps: Vec<AnyElement> = t.depends_on.iter().map(|d| link(*d, true, cx)).collect();
    let add_key = SharedString::from(format!("page-deps-{id}"));
    let add = anchored(
        pill(p, add_key.clone(), can)
            .text_color(p.muted)
            .when(can, |d| d.on_click(cx.listener(move |this, _, window, cx| this.open_edit_menu(EditKind::Depends, vec![id], SharedString::from(format!("page-deps-{id}")), false, window, cx))))
            .child(super::sidebar::icon(IconName::Plus, 12., p.muted))
            .child(t!("tickets.add_dependency")),
        anchors,
        add_key,
    );
    let blocks: Vec<AnyElement> = model.unblocks(id).into_iter().map(|d| link(d, false, cx)).collect();
    v_flex()
        .gap_1()
        .children(rows)
        .child(div().h(px(1.)).my_2().bg(p.hair))
        .child(div().text_size(px(12.)).text_color(p.muted).child(t!("tickets.prop_depends")))
        .children(deps)
        .child(add)
        .when(!blocks.is_empty(), |c| c.child(div().mt_2().text_size(px(12.)).text_color(p.muted).child(t!("tickets.prop_blocks"))).children(blocks))
        .when(!compact, |c| {
            c.child(div().h(px(1.)).my_2().bg(p.hair))
                .children(t.branch.clone().filter(|_| !t.kind.in_home()).map(|b| row(t!("tickets.prop_branch"), div().font_family(MONO).text_size(px(11.5)).text_color(p.text2).truncate().child(b).into_any_element())))
                .child(row(t!("tickets.prop_created_by"), div().text_size(px(12.5)).text_color(p.text2).child(if t.created_by == "you" { t!("tickets.you").to_owned() } else { t.created_by.clone() }).into_any_element()))
        })
        .into_any_element()
}

/// The title, as text to click (or E) — or the box it is edited in.
pub fn title(shell: &Shell, t: &Ticket, size: f32, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = t.id;
    if shell.board.tk.title_edit == Some(id) {
        return div()
            .px_2()
            .py_1()
            .mx(px(-8.))
            .rounded(px(6.))
            .border_1()
            .border_color(p.accent)
            .text_size(px(size))
            .font_weight(FontWeight::SEMIBOLD)
            .child(Input::new(&shell.board.tk.title_input).appearance(false).bordered(false).text_size(px(size)))
            .into_any_element();
    }
    div()
        .id(SharedString::from(format!("title-{id}")))
        .px_2()
        .py_1()
        .mx(px(-8.))
        .rounded(px(6.))
        .text_size(px(size))
        .line_height(px(size * 1.4))
        .font_weight(FontWeight::SEMIBOLD)
        .when(shell.can_act(), |d| d.cursor_text().hover(move |s| s.bg(p.wash)).on_click(cx.listener(move |this, _, window, cx| this.edit_title(id, window, cx))))
        .child(t.title.clone())
        .tip(p, t!("tickets.tip_edit_title"), Some("E"))
        .into_any_element()
}

/// The description: Markdown to read, a click to edit (⌘⏎ keeps it).
pub fn description(shell: &Shell, t: &Ticket, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = t.id;
    if shell.board.tk.body_edit == Some(id) {
        return v_flex()
            .gap_2()
            .child(div().px_3().py_2().rounded(px(8.)).bg(p.code_bg).border_1().border_color(p.accent).text_size(px(14.)).line_height(px(22.)).child(Textarea::new(&shell.board.tk.body_input).appearance(false).bordered(false)))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().text_size(px(12.)).text_color(p.muted).child(t!("tickets.markdown_hint")))
                    .child(btn(p, "body-cancel", t!("peek.cancel"), Some("Esc"), false, true).on_click(cx.listener(|this, _, window, cx| {
                        this.board.tk.body_edit = None;
                        window.blur(cx);
                        cx.notify();
                    })))
                    .child(btn(p, "body-save", t!("tickets.save"), Some("⌘⏎"), true, shell.can_act()).on_click(cx.listener(|this, _, window, cx| this.save_body(window, cx)))),
            )
            .into_any_element();
    }
    let handle = cx.entity().downgrade();
    let empty = t.body.trim().is_empty();
    div()
        .id(SharedString::from(format!("body-{id}")))
        .px_3()
        .py_2()
        .mx(px(-12.))
        .rounded(px(8.))
        .min_h(px(40.))
        .when(shell.can_act(), |d| d.cursor_text().hover(move |s| s.bg(p.wash)).on_click(cx.listener(move |this, _, window, cx| this.edit_body(id, window, cx))))
        .when(empty, |d| d.text_size(px(14.)).text_color(p.muted).child(t!("tickets.add_description")))
        .when(!empty, |d| d.child(super::chat::markdown(p, ("page-body", id as u64), &t.body, 14., 22., p.text2, &handle)))
        .into_any_element()
}

/// What a line of the record says happened.
fn entry_words(kind: &str) -> String {
    match kind {
        "created" => t!("tickets.ev_created").into(),
        "assigned" => t!("tickets.ev_assigned").into(),
        "handed in" => t!("tickets.ev_handed_in").into(),
        "waiting" => t!("tickets.ev_waiting").into(),
        "accepted" => t!("tickets.ev_accepted").into(),
        "cancelled" => t!("tickets.ev_cancelled").into(),
        "reopened" | "sent back" => t!("tickets.ev_reopened").into(),
        "resumed" => t!("tickets.ev_resumed").into(),
        "merged" => t!("tickets.ev_merged").into(),
        "not merged" => t!("tickets.ev_not_merged").into(),
        "kept on branch" => t!("tickets.ev_kept").into(),
        "edited" | "updated" => t!("tickets.ev_edited").into(),
        "reassigned" => t!("tickets.ev_reassigned").into(),
        "taken on" => t!("tickets.ev_taken_on").into(),
        "back to proposal" => t!("tickets.ev_back_to_proposal").into(),
        "restored" => t!("tickets.ev_restored").into(),
        "blocked" => t!("tickets.ev_blocked").into(),
        "released" => t!("tickets.ev_released").into(),
        other => other.to_owned(),
    }
}

/// The ticket's record, oldest first, and the box to comment in.
pub fn timeline(shell: &Shell, t: &Ticket, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let entries = model.histories.get(&t.id).cloned().unwrap_or_default();
    let who_word = |by: &str| match by {
        "you" => t!("tickets.you").to_owned(),
        other => other.to_owned(),
    };
    let lines: Vec<AnyElement> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let when = if e.at > 0 { short_span(model.unix.saturating_sub(e.at)) } else { String::new() };
            let by_color = if e.by == "you" { p.accent } else if e.by == "equipe" { p.muted } else { member_color(p, model, &e.by) };
            if e.kind == "comment" {
                return v_flex()
                    .id(("entry", i))
                    .gap_1()
                    .p_3()
                    .rounded(px(10.))
                    .bg(p.raised)
                    .border_1()
                    .border_color(p.hair)
                    .child(h_flex().gap_2().items_center().text_size(px(12.)).child(if e.by == "you" { you_mark(p, 16.) } else { avatar(p, model, Some(e.by.as_str()), 16.) }).child(div().font_weight(FontWeight::SEMIBOLD).text_color(by_color).child(who_word(&e.by))).child(div().text_color(p.muted).child(when)))
                    .child(div().text_size(px(13.5)).line_height(px(21.)).text_color(p.text).child(e.text.clone()))
                    .into_any_element();
            }
            let detail = match e.kind.as_str() {
                "updated" => String::new(),
                _ => e.text.lines().next().unwrap_or_default().chars().take(140).collect(),
            };
            h_flex()
                .id(("entry", i))
                .min_h(px(26.))
                .gap_2()
                .items_center()
                .text_size(px(12.5))
                .child(div().w(px(16.)).flex_none().flex().justify_center().child(dot(by_color, 6.)))
                .child(div().flex_none().font_weight(FontWeight::MEDIUM).text_color(p.text2).child(who_word(&e.by)))
                .child(div().flex_none().text_color(p.muted).child(entry_words(&e.kind)))
                .child(div().flex_1().min_w_0().truncate().text_color(p.text2).child(detail))
                .child(div().flex_none().text_color(p.muted).child(when))
                .into_any_element()
        })
        .collect();
    let can = shell.can_act();
    let empty = shell.board.tk.comment.read(cx).value().trim().is_empty();
    v_flex()
        .gap_2()
        .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("tickets.activity")))
        .when(lines.is_empty(), |c| c.child(div().text_size(px(12.)).text_color(p.muted).child(t!("tickets.no_activity"))))
        .children(lines)
        .child(
            v_flex()
                .mt_2()
                .gap_2()
                .p_3()
                .rounded(px(10.))
                .bg(p.code_bg)
                .border_1()
                .border_color(p.hair)
                .child(div().text_size(px(14.)).line_height(px(21.)).child(Textarea::new(&shell.board.tk.comment).appearance(false).bordered(false)))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().flex_1().text_size(px(11.5)).text_color(p.muted).child(t!("tickets.comment_hint")))
                        .child(btn(p, "comment-send", t!("tickets.comment_send"), Some("⌘⏎"), true, can && !empty).on_click(cx.listener(|this, _, window, cx| this.send_comment(window, cx)))),
                ),
        )
        .into_any_element()
}

/// "Stop T-47 and give it to backend?" — asked before work under way changes hands.
pub fn confirm_card(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let (ids, edit) = shell.board.tk.confirm.clone()?;
    let to = edit.assignee.clone().flatten().unwrap_or_default();
    let working: Vec<&Ticket> = ids.iter().filter_map(|id| shell.model.tickets.iter().find(|t| t.id == *id)).filter(|t| t.status == TicketStatus::InProgress).collect();
    let names: Vec<String> = working.iter().map(|t| format!("T-{}", t.id)).collect();
    let from: Vec<String> = working.iter().filter_map(|t| rules::who(t).map(str::to_owned)).collect();
    let card = v_flex()
        .id("confirm-reassign")
        .w(px(440.))
        .p_5()
        .gap_3()
        .rounded(px(14.))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .occlude()
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("tickets.confirm_title", ids = names.join(", "), to = to)))
        .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(t!("tickets.confirm_body", from = from.join(", "), to = to)))
        .child(
            h_flex()
                .gap_2()
                .child(div().flex_1())
                .child(btn(p, "confirm-keep", t!("board.keep"), Some("Esc"), false, true).on_click(cx.listener(|this, _, _, cx| {
                    this.board.tk.confirm = None;
                    cx.notify();
                })))
                .child(btn(p, "confirm-give", t!("tickets.confirm_give"), Some("⏎"), true, shell.can_act()).on_click(cx.listener(|this, _, _, cx| this.confirm_edit(cx)))),
        );
    Some(
        deferred(
            div()
                .id("confirm-scrim")
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(120.))
                .bg(rgba(0x0000005C))
                .occlude()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.board.tk.confirm = None;
                    cx.notify();
                }))
                .child(motion::dialog(card, "confirm-in")),
        )
        .with_priority(3)
        .into_any_element(),
    )
}
