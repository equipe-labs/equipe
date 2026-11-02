//! A ticket, on the pointer: rest on a `T-n` anywhere — a reply, a news or
//! tool row, your own message, a row's dependency chip, the sidebar — and
//! after a moment a small card shows what it is (where it stands, who has it,
//! the hand-in or what it asks, the change's size, what it waits on) and
//! what can be done about it now, by the Inbox peek's rules: a review is
//! accepted (A) or sent back (R), its changes looked at (D); accepted work
//! merged; a wait answered; a proposal taken or dropped; any ticket opened
//! beside the conversation (⏎). The card stays while the pointer moves into
//! it, goes a moment after it leaves, and Esc puts it away. It never opens
//! while the conversation is scrolling or text is being selected. A wheel on
//! it scrolls its text — and its middle, when the window is too short for all
//! of it — and never the conversation under it; a wheel off it scrolls the
//! conversation and the card goes.
//!
//! The timing ([`Hover`]), the placing ([`place`]) and what a ticket offers
//! ([`acts`]) are plain rules, tested below; the card is drawn over the
//! window by one view ([`TicketCard`]), which every link reports to.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use equipe_core::protocol::{Ticket, TicketId, TicketStatus};
use equipe_core::t;
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::{btn, dot, hue_of, member_color};
use super::icons;
use crate::board::{self as rules, MergeWay};
use crate::state::Model;
use crate::theme::{MONO, Palette, R_MD, SMALL, Sans, look};
use crate::{Screen, Shell};

/// How long the pointer rests on a link before its card shows.
pub const OPEN_AFTER: Duration = Duration::from_millis(350);
/// How long the card stays after the pointer left it (and its link).
pub const CLOSE_AFTER: Duration = Duration::from_millis(300);
/// No card opens this soon after the conversation scrolled.
pub const SCROLL_QUIET: Duration = Duration::from_millis(300);
/// How long the card says what was done before it goes.
pub const RESULT_FOR: Duration = Duration::from_millis(1400);
/// The card's width.
pub const WIDTH: f32 = 380.;
/// Between the link and the card.
const GAP: f32 = 6.;
/// The card keeps this far inside the window.
const MARGIN: f32 = 8.;
/// Slack around a link the pointer may wander in without leaving it.
const SLACK: f32 = 2.;
/// The size a card is taken to be before it was ever drawn.
const GUESS: (f32, f32) = (WIDTH, 230.);
/// The excerpt: this many lines, then it fades out.
const EXCERPT_LINES: f32 = 8.;
/// How tall the fade over a cut edge of the card's text is.
const FADE: f32 = 28.;

/// Whether the card's text goes on above and below what shows, scrolled
/// `down` of the `most` it can be.
fn fades(down: f32, most: f32) -> (bool, bool) {
    (down > 0.5, most - down > 0.5)
}

/// The card's height at most in a window `window_h` tall: the window less its
/// margins (past that its middle scrolls, its head and buttons kept in view).
pub fn tallest(window_h: f32) -> f32 {
    (window_h - 2. * MARGIN).max(TALLEST_MIN)
}

/// Below this the card isn't squeezed: a window that short shows what it can.
const TALLEST_MIN: f32 = 120.;

/// Whether a wheel turn by `dy` (up positive) on the card's text is the
/// text's: it was scrolled to `at` (zero or less) of the `most` it can be, and
/// can still go that way. At its end (or all of it showing) the turn goes on
/// to the card's middle, when the window is too short for the whole card —
/// never past the card.
fn text_takes(at: f32, dy: f32, most: f32) -> bool {
    (dy < 0. && at > -most + 0.5) || (dy > 0. && at < -0.5)
}
const EXCERPT_LINE: f32 = 19.;

// ── the rules ──

/// A rectangle in window points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    fn right(&self) -> f32 {
        self.x + self.w
    }

    fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }

    fn grown(&self, by: f32) -> Self {
        Self { x: self.x - by, y: self.y - by, w: self.w + 2. * by, h: self.h + 2. * by }
    }

    fn of(b: Bounds<Pixels>) -> Self {
        Self { x: f32::from(b.origin.x), y: f32::from(b.origin.y), w: f32::from(b.size.width), h: f32::from(b.size.height) }
    }
}

/// Which side of its link the card is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Below,
    Above,
    Right,
    Left,
}

/// Where the card goes for a link at `anchor`: its top-left corner and the
/// side it ended on — `prefer`, unless the window has no room there and more
/// on the other side; always inside the window, `MARGIN` from its edges.
pub fn place(anchor: Rect, card: (f32, f32), window: (f32, f32), prefer: Side) -> (f32, f32, Side) {
    let (w, h) = card;
    let (ww, wh) = window;
    let clamp = |v: f32, size: f32, room: f32| v.min(room - MARGIN - size).max(MARGIN);
    match prefer {
        Side::Below | Side::Above => {
            let below = wh - MARGIN - (anchor.bottom() + GAP);
            let above = anchor.y - GAP - MARGIN;
            let side = match prefer {
                Side::Below if h > below && above > below => Side::Above,
                Side::Above if h > above && below > above => Side::Below,
                _ => prefer,
            };
            let y = if side == Side::Below { anchor.bottom() + GAP } else { anchor.y - GAP - h };
            (clamp(anchor.x, w, ww), clamp(y, h, wh), side)
        }
        Side::Right | Side::Left => {
            let right = ww - MARGIN - (anchor.right() + GAP);
            let left = anchor.x - GAP - MARGIN;
            let side = match prefer {
                Side::Right if w > right && left > right => Side::Left,
                Side::Left if w > left && right > left => Side::Right,
                _ => prefer,
            };
            let x = if side == Side::Right { anchor.right() + GAP } else { anchor.x - GAP - w };
            (clamp(x, w, ww), clamp(anchor.y, h, wh), side)
        }
    }
}

/// The link, the card and the room between them: the way from one to the
/// other, however slanted, doesn't count as leaving — nor do links under it
/// as somewhere else to go.
fn corridor(anchor: Rect, card: Rect) -> Rect {
    let (x, y) = (anchor.x.min(card.x), anchor.y.min(card.y));
    Rect::new(x, y, anchor.right().max(card.right()) - x, anchor.bottom().max(card.bottom()) - y)
}

/// A link the pointer is on: which ticket, where it is, where its card goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub id: TicketId,
    pub anchor: Rect,
    pub prefer: Side,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Idle,
    /// On a link, not long enough yet.
    Waiting { target: Target, since: Instant },
    /// Shown. `inside`: the pointer is on the card; `left`: when it left both
    /// the card and the link; `keys`: opened from the keyboard, so the
    /// pointer's whereabouts don't close it.
    Open { target: Target, inside: bool, left: Option<Instant>, keys: bool },
    /// What was done, said on the card until `until`.
    Done { target: Target, until: Instant },
}

/// When the card shows and goes: the pointer's way over links and the card.
#[derive(Clone, Copy, Debug)]
pub struct Hover {
    pub phase: Phase,
    /// The card as last drawn.
    pub card: Option<Rect>,
    /// Bumped each time a card opens, to play its entrance once.
    pub epoch: u64,
    scrolled: Option<Instant>,
    dragging: bool,
    /// Where the pointer last was.
    pointer: Option<(f32, f32)>,
}

impl Default for Hover {
    fn default() -> Self {
        Self { phase: Phase::Idle, card: None, epoch: 0, scrolled: None, dragging: false, pointer: None }
    }
}

impl Hover {
    /// The pointer came onto a link (a button not held). True when something changed.
    pub fn enter(&mut self, target: Target, now: Instant) -> bool {
        if self.dragging || self.scrolled.is_some_and(|at| now.saturating_duration_since(at) < SCROLL_QUIET) {
            return false;
        }
        match self.phase {
            // A link on the card itself (in its excerpt) is not another card.
            Phase::Open { inside: true, .. } | Phase::Open { keys: true, .. } => false,
            // On its way to the card: a link it passes over isn't where it's going.
            Phase::Open { target: shown, .. } if shown.id != target.id && self.heading_in(shown) => false,
            Phase::Open { target: shown, left, .. } if shown.id == target.id => {
                self.phase = Phase::Open { target: shown, inside: false, left: None, keys: false };
                left.is_some()
            }
            // From one card to the next link: the next card at once.
            Phase::Open { .. } | Phase::Done { .. } => {
                self.open(target, false);
                true
            }
            Phase::Waiting { target: on, since } if on.id == target.id => {
                self.phase = Phase::Waiting { target, since };
                false
            }
            Phase::Waiting { .. } | Phase::Idle => {
                self.phase = Phase::Waiting { target, since: now };
                true
            }
        }
    }

    /// Whether the pointer is between `shown`'s link and its card.
    fn heading_in(&self, shown: Target) -> bool {
        match (self.card, self.pointer) {
            (Some(card), Some((x, y))) => corridor(shown.anchor, card).grown(SLACK).contains(x, y),
            _ => false,
        }
    }

    fn open(&mut self, target: Target, keys: bool) {
        self.phase = Phase::Open { target, inside: false, left: None, keys };
        self.epoch += 1;
        self.card = None;
    }

    /// Open at once, from the keyboard (Space on a link that has the keys).
    pub fn open_now(&mut self, target: Target) {
        self.open(target, true);
    }

    /// The pointer moved to `x, y` (`pressed`: a button held — a selection
    /// being dragged). True when the card shows or goes, or a wait was dropped.
    pub fn moved(&mut self, x: f32, y: f32, pressed: bool, now: Instant) -> bool {
        self.dragging = pressed;
        self.pointer = Some((x, y));
        let before = self.phase;
        match self.phase {
            Phase::Waiting { .. } if pressed => self.phase = Phase::Idle,
            Phase::Waiting { target, .. } if !target.anchor.grown(SLACK).contains(x, y) => self.phase = Phase::Idle,
            Phase::Open { target, keys, left, .. } => {
                let inside = self.card.is_some_and(|c| c.contains(x, y));
                let on = inside || target.anchor.grown(SLACK).contains(x, y) || self.card.is_some_and(|c| corridor(target.anchor, c).grown(SLACK).contains(x, y));
                let left = if on || keys { None } else { left.or(Some(now)) };
                self.phase = Phase::Open { target, inside, left, keys };
            }
            _ => {}
        }
        // Only a wait dropped or the card going is something to draw; the
        // pointer's whereabouts on it are not.
        !matches!((before, self.phase), (Phase::Open { .. }, Phase::Open { .. })) && before != self.phase
    }

    /// Time passed: the card shows after its wait, goes after the pointer
    /// left, and its "done" ends. True when it changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        match self.phase {
            Phase::Waiting { target, since } if now.saturating_duration_since(since) >= OPEN_AFTER => {
                self.open(target, false);
                true
            }
            Phase::Open { left: Some(at), .. } if now.saturating_duration_since(at) >= CLOSE_AFTER => self.close(),
            Phase::Done { until, .. } if now >= until => self.close(),
            _ => false,
        }
    }

    /// How long until [`Hover::tick`] has something to do.
    pub fn next_wake(&self, now: Instant) -> Option<Duration> {
        let at = match self.phase {
            Phase::Waiting { since, .. } => since + OPEN_AFTER,
            Phase::Open { left: Some(at), .. } => at + CLOSE_AFTER,
            Phase::Done { until, .. } => until,
            _ => return None,
        };
        Some(at.saturating_duration_since(now))
    }

    /// The conversation scrolled: no card waits, and one shown goes (its
    /// link moved away) — unless the pointer is on it or the keys opened it.
    pub fn scrolled(&mut self, now: Instant) -> bool {
        self.scrolled = Some(now);
        match self.phase {
            Phase::Waiting { .. } | Phase::Open { inside: false, keys: false, .. } => self.close(),
            _ => false,
        }
    }

    /// A button went down at `x, y`: away from the card, the card goes.
    pub fn pressed(&mut self, x: f32, y: f32) -> bool {
        match self.phase {
            Phase::Idle => false,
            _ if self.card.is_some_and(|c| c.contains(x, y)) && self.shown().is_some() => false,
            _ => self.close(),
        }
    }

    /// Esc, or an action that went elsewhere: put it away. Whether it was there.
    pub fn close(&mut self) -> bool {
        let was = self.phase != Phase::Idle;
        self.phase = Phase::Idle;
        self.card = None;
        was
    }

    /// An action was taken on the card: it says so for a moment.
    pub fn acted(&mut self, now: Instant) {
        if let Some(target) = self.shown() {
            self.phase = Phase::Done { target, until: now + RESULT_FOR };
        }
    }

    /// The card on screen, if one is.
    pub fn shown(&self) -> Option<Target> {
        match self.phase {
            Phase::Open { target, .. } | Phase::Done { target, .. } => Some(target),
            _ => None,
        }
    }

    /// Whether a key goes to the card: the pointer is on it, or the keys
    /// opened it. (Resting on the link alone leaves typing alone.)
    pub fn takes_keys(&self) -> bool {
        matches!(self.phase, Phase::Open { inside: true, .. } | Phase::Open { keys: true, .. })
    }
}

/// What can be done about a ticket from its card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Accept,
    RequestChanges,
    Diff,
    Merge,
    Answer,
    Take,
    Drop,
    Open,
}

impl Act {
    /// Its key while the card is up.
    pub fn key(self) -> &'static str {
        match self {
            Act::Accept | Act::Merge | Act::Take => "A",
            Act::RequestChanges | Act::Answer => "R",
            Act::Diff => "D",
            Act::Drop => "N",
            Act::Open => "⏎",
        }
    }

    /// Whether it is the person's answer (only while connected).
    pub fn answers(self) -> bool {
        !matches!(self, Act::Diff | Act::Open)
    }

    fn label(self, way: Option<MergeWay>) -> String {
        match self {
            Act::Accept => t!("peek.accept").to_owned(),
            Act::RequestChanges => t!("peek.request_changes").to_owned(),
            Act::Diff => t!("card.see_changes").to_owned(),
            Act::Merge => match way {
                Some(MergeWay::Push) => t!("peek.merge_main_push").to_owned(),
                Some(MergeWay::Merge) => t!("peek.merge_main").to_owned(),
                _ => t!("peek.merge_main_ask").to_owned(),
            },
            Act::Answer => t!("card.answer").to_owned(),
            Act::Take => t!("card.take").to_owned(),
            Act::Drop => t!("peek.drop").to_owned(),
            Act::Open => t!("card.open").to_owned(),
        }
    }
}

/// What a ticket offers on its card, by where it stands — the Inbox peek's
/// answers, and always a way to open it.
pub fn acts(status: TicketStatus, not_on_main: bool, branch: bool) -> Vec<Act> {
    let mut out = match status {
        TicketStatus::Review if branch => vec![Act::Accept, Act::RequestChanges, Act::Diff],
        TicketStatus::Review => vec![Act::Accept, Act::RequestChanges],
        TicketStatus::Done if not_on_main => vec![Act::Merge],
        TicketStatus::Waiting => vec![Act::Answer],
        TicketStatus::Proposed => vec![Act::Take, Act::Drop],
        _ => Vec::new(),
    };
    out.push(Act::Open);
    out
}

/// The act a key asks for, among those offered.
pub fn act_for_key(key: &str, offered: &[Act]) -> Option<Act> {
    let want = match key {
        "enter" => "⏎",
        "a" => "A",
        "r" => "R",
        "d" => "D",
        "n" => "N",
        _ => return None,
    };
    offered.iter().copied().find(|a| a.key() == want)
}

/// What the card says once `act` was taken on a ticket that stood at
/// `status` (`merges`: accepting it lands code; `way`: how it landed).
pub fn done_words(act: Act, merges: bool, way: Option<MergeWay>) -> Option<&'static str> {
    Some(match act {
        Act::Accept if !merges => t!("card.done_accepted"),
        Act::Accept => match way {
            Some(MergeWay::Merge) => t!("card.done_accepted_merged"),
            Some(MergeWay::Push) => t!("card.done_accepted_pushed"),
            _ => t!("card.done_accepted_branch"),
        },
        Act::Merge => match way {
            Some(MergeWay::Push) => t!("card.done_pushed"),
            _ => t!("card.done_merged"),
        },
        Act::Take => t!("card.done_taken"),
        Act::Drop => t!("card.done_dropped"),
        _ => return None,
    })
}

// ── the card on screen ──

/// The window's card, drawn over everything by [`layer`].
pub struct TicketCard {
    shell: WeakEntity<Shell>,
    pub hover: Hover,
    timer: Option<Task<()>>,
    /// The card as last drawn: where it is, for the pointer, and its size, to place it.
    drawn: Rc<Cell<Option<Rect>>>,
    /// What was done, while the card says it.
    result: Option<SharedString>,
    /// Branches whose size was asked for, as they looked then.
    asked: HashMap<TicketId, (Option<String>, String, Option<bool>)>,
    /// Tickets acted on from a card: their undo shows off the work screens.
    acted: HashSet<TicketId>,
    /// Where the card's text, and its middle (when the window is too short
    /// for all of it), are scrolled to, for the card it opened as.
    said: (u64, ScrollHandle, ScrollHandle),
}

struct Layer(Entity<TicketCard>);

impl Global for Layer {}

/// The card's view, made the first time: every text view's links and the
/// app's own `T-n`s report to it.
pub fn layer(cx: &mut Context<Shell>) -> AnyView {
    if let Some(layer) = cx.try_global::<Layer>() {
        return layer.0.clone().into();
    }
    let shell = cx.entity().downgrade();
    let card = cx.new(|_| TicketCard { shell, hover: Hover::default(), timer: None, drawn: Rc::new(Cell::new(None)), result: None, asked: HashMap::new(), acted: HashSet::new(), said: (0, ScrollHandle::new(), ScrollHandle::new()) });
    cx.set_global(Layer(card.clone()));
    gpui_kit::base::text::on_link_hover(cx, |url, bounds, _, cx| {
        if let Some(id) = url.strip_prefix(crate::convo::TICKET_SCHEME).and_then(|n| n.parse().ok()) {
            enter(id, bounds, Side::Below, cx);
        }
    });
    card.into()
}

fn with_card<R>(cx: &mut App, f: impl FnOnce(&mut TicketCard, &mut Context<TicketCard>) -> R) -> Option<R> {
    let card = cx.try_global::<Layer>()?.0.clone();
    Some(card.update(cx, f))
}

/// Something opened over the window (a picture shown big): the card goes.
pub fn put_away(cx: &mut App) {
    with_card(cx, |card, cx| {
        if card.hover.close() {
            card.changed(cx);
        }
    });
}

/// The pointer came onto a link to ticket `id`, at `bounds`.
pub fn enter(id: TicketId, bounds: Bounds<Pixels>, prefer: Side, cx: &mut App) {
    with_card(cx, |card, cx| {
        if card.hover.enter(Target { id, anchor: Rect::of(bounds), prefer }, Instant::now()) {
            card.changed(cx);
        }
    });
}

/// Space on a link that has the keys: its card at once.
pub fn open_by_keys(id: TicketId, bounds: Bounds<Pixels>, prefer: Side, cx: &mut App) {
    with_card(cx, |card, cx| {
        card.hover.open_now(Target { id, anchor: Rect::of(bounds), prefer });
        card.result = None;
        card.changed(cx);
    });
}

/// `el`, a link to ticket `id` drawn by the app (a chip, a row): its card
/// on the pointer, on `prefer`'s side.
pub fn hover_link<E: StatefulInteractiveElement + ParentElement + 'static>(el: E, id: TicketId, prefer: Side) -> E {
    let bounds = Rc::new(Cell::new(Bounds::default()));
    let seen = bounds.clone();
    el.on_prepaint(move |b, _, _| seen.set(b)).on_hover(move |hovered, _, cx| {
        if *hovered {
            enter(id, bounds.get(), prefer, cx);
        }
    })
}

/// `el`, a row that has the keys and names ticket `id`: Space shows the
/// ticket's card (Enter still does what the row does).
pub fn space_opens<E: InteractiveElement + ParentElement + 'static>(el: E, id: Option<TicketId>) -> E {
    let Some(id) = id else { return el };
    let bounds = Rc::new(Cell::new(Bounds::default()));
    let seen = bounds.clone();
    el.on_prepaint(move |b, _, _| seen.set(b)).capture_key_down(move |event: &KeyDownEvent, _, cx| {
        let k = &event.keystroke;
        if k.key == "space" && !k.modifiers.modified() {
            open_by_keys(id, bounds.get(), Side::Below, cx);
            cx.stop_propagation();
        }
    })
}

/// A `T-n` in a line of text drawn by the app (`linked` rows): its card when
/// the pointer is on the number at `ix`. `ranges` are the numbers' places in
/// the text laid out as `layout`, `tickets` what they name.
pub fn text_hover(layout: &TextLayout, ranges: &[std::ops::Range<usize>], tickets: &[TicketId], ix: Option<usize>, event: &MouseMoveEvent, cx: &mut App) {
    if event.pressed_button.is_some() {
        return;
    }
    let Some((n, range)) = ix.and_then(|ix| ranges.iter().enumerate().find(|(_, r)| r.contains(&ix))) else { return };
    let Some(&id) = tickets.get(n) else { return };
    let (Some(start), Some(end)) = (layout.position_for_index(range.start), layout.position_for_index(range.end)) else { return };
    let line = layout.line_height();
    let bounds = Bounds::from_corners(start, point(end.x.max(start.x + px(1.)), start.y + line));
    enter(id, bounds, Side::Below, cx);
}

/// A key while a card is up, from the window's root (before what has the
/// keys): Esc puts it away; with the pointer on it (or the keys having
/// opened it) A, R, D, N and ⏎ do what its buttons say. True when it was the card's.
pub fn key(shell: &mut Shell, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Shell>) -> bool {
    let Some(card) = cx.try_global::<Layer>().map(|l| l.0.clone()) else { return false };
    let (shown, keys, done) = {
        let c = card.read(cx);
        (c.hover.shown(), c.hover.takes_keys(), matches!(c.hover.phase, Phase::Done { .. }))
    };
    let Some(target) = shown else { return false };
    let k = &event.keystroke;
    if k.key == "escape" {
        card.update(cx, |c, cx| {
            c.hover.close();
            c.changed(cx);
        });
        return true;
    }
    if !keys || done || k.modifiers.modified() {
        return false;
    }
    let Some(t) = shell.model.tickets.iter().find(|t| t.id == target.id) else { return false };
    let offered = acts(t.status, t.not_on_main(), t.branch.is_some());
    let Some(act) = act_for_key(&k.key, &offered) else { return false };
    if act.answers() && !shell.can_act() {
        return true;
    }
    let said = shell.card_act(target.id, act, window, cx);
    card.update(cx, |c, cx| c.after(target.id, said, cx));
    true
}

impl TicketCard {
    /// The state changed: draw again, and wake when the next thing is due.
    fn changed(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.hover.phase, Phase::Done { .. }) {
            self.result = None;
        }
        self.schedule(cx);
        cx.notify();
    }

    fn schedule(&mut self, cx: &mut Context<Self>) {
        let Some(wait) = self.hover.next_wake(Instant::now()) else {
            self.timer = None;
            return;
        };
        self.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |card, cx| {
                if card.hover.tick(Instant::now()) {
                    card.changed(cx);
                } else {
                    card.schedule(cx);
                }
            });
        }));
    }

    /// An action ran: what it did, said on the card (`said`), or the card
    /// goes — the action went to the ticket's page, the board or the panel.
    fn after(&mut self, id: TicketId, said: Option<String>, cx: &mut Context<Self>) {
        match said {
            Some(words) => {
                self.acted.insert(id);
                self.hover.acted(Instant::now());
                self.result = Some(words.into());
            }
            None => {
                self.hover.close();
            }
        }
        self.changed(cx);
    }

    fn on_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        self.hover.card = self.drawn.get();
        let (x, y) = (f32::from(event.position.x), f32::from(event.position.y));
        let was = self.hover.next_wake(Instant::now()).is_some();
        if self.hover.moved(x, y, event.pressed_button.is_some(), Instant::now()) {
            self.changed(cx);
        } else if self.hover.next_wake(Instant::now()).is_some() != was {
            self.schedule(cx);
        }
    }
}

impl Render for TicketCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        // The pointer, the wheel and a press, wherever they happen.
        let listen = {
            let this = this.clone();
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let (moved, wheel, down) = (this.clone(), this.clone(), this.clone());
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            let _ = moved.update(cx, |c, cx| c.on_move(e, cx));
                        }
                    });
                    window.on_mouse_event(move |_: &ScrollWheelEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            let _ = wheel.update(cx, |c, cx| {
                                if c.hover.scrolled(Instant::now()) {
                                    c.changed(cx);
                                }
                            });
                        }
                    });
                    window.on_mouse_event(move |e: &MouseDownEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            let _ = down.update(cx, |c, cx| {
                                c.hover.card = c.drawn.get();
                                if c.hover.pressed(f32::from(e.position.x), f32::from(e.position.y)) {
                                    c.changed(cx);
                                }
                            });
                        }
                    });
                },
            )
            .absolute()
            .size_full()
        };
        let root = div().absolute().top_0().left_0().size_full().child(listen);
        let Some(shell) = self.shell.upgrade() else { return root };
        // Its undo counts the seconds down (`beat.rs`).
        let shell_now = shell.read(cx);
        if let Some(at) = shell_now.board.undo.latest().filter(|p| shell_now.screen != Screen::Board && self.acted.contains(&p.ticket)).map(|p| rules::Undo::next_count(p, Instant::now())) {
            crate::beat::Beat::wake(cx.entity_id(), at, cx);
        }
        let shell = shell.read(cx);
        let root = root.children(undo_toast(shell, &self.acted, &self.shell));
        let Some(target) = self.hover.shown() else {
            self.drawn.set(None);
            return root;
        };
        let p = shell.palette;
        let found = shell.model.tickets.iter().find(|t| t.id == target.id);
        // The size of the branch, asked of git once per look of the ticket.
        if let Some(t) = found.filter(|t| t.branch.is_some() && shell.board.stat(t).is_none()) {
            let seen = crate::diffs::fingerprint(t);
            if self.asked.get(&t.id) != Some(&seen)
                && let Some(source) = crate::diffs::source(&shell.model, t)
            {
                self.asked.insert(t.id, seen);
                shell.fetch(t.id, source, false);
            }
        }
        let shots = super::visuals::Shots::of(shell, self.shell.clone(), cx);
        let held = shell.shots.clone();
        let body = match found {
            Some(t) => {
                // Each card opens at the top of its text.
                if self.said.0 != self.hover.epoch {
                    self.said = (self.hover.epoch, ScrollHandle::new(), ScrollHandle::new());
                }
                content(shell, t, self.result.clone(), &self.shell, &this, &shots, (&self.said.1, &self.said.2))
            }
            None => missing(p, target.id),
        };
        let found = found.is_some();
        // Its pictures, asked for once the window's state is let go.
        super::visuals::flush(&held, &self.shell, cx);
        // Placed by its size as last drawn; drawn again when that changes.
        let viewport = window.viewport_size();
        let size = self.drawn.get().map_or(if found { GUESS } else { (200., 36.) }, |r| (r.w, r.h));
        let (x, y, side) = place(target.anchor, size, (f32::from(viewport.width), f32::from(viewport.height)), target.prefer);
        let drawn = self.drawn.clone();
        let card = div()
            .id(("ticket-card", target.id as u64))
            .relative()
            .occlude()
            .w(px(if found { WIDTH } else { 220. }))
            .rounded(px(10.))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .shadow(crate::motion::float_shadow(1.))
            .text_color(p.text)
            .cursor_default()
            // No taller than the window: its middle scrolls instead.
            .flex()
            .flex_col()
            .max_h(px(tallest(f32::from(viewport.height))))
            .child(body)
            // A wheel on the card is the card's, at its ends too: what is
            // under it (the conversation) never scrolls from here.
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .on_prepaint(move |b, window, _| {
                let r = Rect::of(b);
                let before = drawn.replace(Some(r));
                if before.is_none_or(|o| (o.w - r.w).abs() > 0.5 || (o.h - r.h).abs() > 0.5) {
                    window.request_animation_frame();
                }
            });
        let card = div().relative().child(crate::motion::pop(card, ("ticket-card-in", self.hover.epoch), side == Side::Above));
        root.child(div().absolute().left(px(x)).top(px(y)).child(card))
    }
}

/// "T-999 · 찾을 수 없어요": a number the squad has no ticket for.
fn missing(p: Palette, id: TicketId) -> AnyElement {
    h_flex()
        .h(px(34.))
        .px_3()
        .gap_2()
        .items_center()
        .text_size(px(SMALL))
        .child(div().font_family(MONO).text_size(px(12.)).text_color(p.text2).child(format!("T-{id}")))
        .child(div().text_color(p.muted).child("·"))
        .child(div().text_color(p.muted).child(t!("card.not_found")))
        .into_any_element()
}

/// The card's inside: where it stands and whose it is, its title, what was
/// said of it, its size and links, and what can be done now.
fn content(shell: &Shell, t: &Ticket, result: Option<SharedString>, handle: &WeakEntity<Shell>, card: &WeakEntity<TicketCard>, shots: &super::visuals::Shots, scrolls: (&ScrollHandle, &ScrollHandle)) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let id = t.id;
    // Status, number, project; whose it is, on the right.
    let pill = h_flex()
        .flex_none()
        .h(px(20.))
        .px(px(7.))
        .gap(px(5.))
        .items_center()
        .rounded(px(10.))
        .bg(p.wash)
        .child(icons::status(p, t.status, 11.))
        .child(div().text_size(px(11.5)).text_color(icons::color(p, t.status)).child(crate::state::state_word(t)));
    let project = model.ticket_project_name(t).map(|name| {
        h_flex()
            .min_w_0()
            .gap(px(5.))
            .items_center()
            .text_size(px(11.5))
            .text_color(p.muted)
            .child(dot(hue_of(p, model, t).unwrap_or(p.muted), 6.))
            .child(div().truncate().child(name))
    });
    let who = rules::who(t).map(|w| {
        let member = model.members.iter().find(|m| m.name == w || m.role == w);
        let (mark, _) = member.map_or_else(|| look(w, w), |m| look(&m.role, &m.name));
        h_flex().flex_none().gap_1().text_size(px(11.5)).text_color(member_color(p, model, w)).child(mark).child(w.to_owned())
    });
    let head = h_flex()
        .gap_2()
        .items_center()
        .child(pill)
        .child(div().flex_none().font_family(MONO).text_size(px(11.5)).text_color(p.muted).child(format!("T-{id}")))
        .children(project)
        .child(div().flex_1())
        .children(who);
    let title = div().text_size(px(14.)).line_height(px(20.)).font_weight(FontWeight::SEMIBOLD).line_clamp(2).child(t.title.clone());
    // What was said of it: the hand-in, what it asks, else what it is.
    let said = t.note.as_deref().filter(|n| !n.trim().is_empty() && !matches!(t.status, TicketStatus::Todo | TicketStatus::InProgress | TicketStatus::Proposed)).unwrap_or(&t.body);
    let (scroll, body) = scrolls;
    // Over a cut edge with more beyond it.
    let fade = |top: bool| {
        let solid = Hsla::from(p.raised);
        let clear = Hsla { a: 0., ..solid };
        let (from, to) = if top { (solid, clear) } else { (clear, solid) };
        div().absolute().left_0().right_0().h(px(FADE)).when(top, |d| d.top_0()).when(!top, |d| d.bottom_0()).bg(linear_gradient(180., linear_color_stop(from, 0.), linear_color_stop(to, 1.)))
    };
    let excerpt = (!said.trim().is_empty()).then(|| {
        // All of it, scrolled on the card past its first lines; an edge with
        // more beyond it fades out.
        let (above, below) = fades(-f32::from(scroll.offset().y), f32::from(scroll.max_offset().y));
        let (card, text) = (card.clone(), scroll.clone());
        div()
            .relative()
            .flex_none()
            .child(
                div()
                    .id(("card-said-scroll", id as u64))
                    .max_h(px(EXCERPT_LINES * EXCERPT_LINE))
                    .overflow_y_scroll()
                    .track_scroll(scroll)
                    .on_scroll_wheel(move |event, window, cx| {
                        // The text has just been moved by the turn (its own
                        // listener runs first, a sideways turn taken as down);
                        // where it was says whether it could.
                        let d = event.delta.pixel_delta(window.line_height());
                        let dy = f32::from(if d.y.is_zero() { d.x } else { d.y });
                        if text_takes(f32::from(text.offset().y) - dy, dy, f32::from(text.max_offset().y)) {
                            cx.stop_propagation();
                        }
                        let _ = card.update(cx, |_, cx| cx.notify());
                    })
                    .child(super::chat::markdown(p, ("card-said", id as u64), said, 13., EXCERPT_LINE, p.text2, handle)),
            )
            .when(above, |d| d.child(fade(true)))
            .when(below, |d| d.child(fade(false)))
    });
    // Its size, what it waits on and holds up, how long it has stood so.
    let mut facts: Vec<AnyElement> = Vec::new();
    if t.branch.is_some() && !t.kind.in_home() {
        facts.push(match shell.board.stat(t) {
            Some(stat) => h_flex()
                .gap_1()
                .font_family(MONO)
                .child(div().text_color(p.accent).child(format!("+{}", stat.added)))
                .child(div().child(format!("−{}", stat.removed)))
                .child(div().sans().child(format!("· {}", t!("card.files", count = stat.files.len()))))
                .into_any_element(),
            None => div().child("…").into_any_element(),
        });
    }
    let open_deps: Vec<String> = t.depends_on.iter().filter(|d| model.tickets.iter().find(|o| o.id == **d).is_none_or(|o| o.status != TicketStatus::Done)).map(|d| format!("T-{d}")).collect();
    if !open_deps.is_empty() && !t.status.is_closed() {
        facts.push(div().text_color(p.orange).child(t!("card.blocked_by", list = open_deps.join(", "))).into_any_element());
    }
    let unblocks: Vec<String> = model.unblocks(id).into_iter().map(|d| format!("T-{d}")).collect();
    if !unblocks.is_empty() {
        facts.push(div().child(t!("card.unblocks", list = unblocks.join(", "))).into_any_element());
    }
    if let Some(since) = model.in_state_for(t) {
        facts.push(div().child(crate::activity::ago(since)).into_any_element());
    }
    let facts = (!facts.is_empty()).then(|| h_flex().flex_wrap().gap_x_3().gap_y_1().text_size(px(11.5)).text_color(p.muted).children(facts));
    // What can be done, or what was.
    let foot = match result {
        Some(words) => h_flex()
            .h(px(28.))
            .gap_2()
            .items_center()
            .text_size(px(SMALL))
            .text_color(p.text)
            .child(crate::icons::icon(crate::icons::Icon::Check, 13., p.accent))
            .child(words)
            .into_any_element(),
        None => {
            let can = shell.can_act();
            let way = shell.merge_way_of(&shell.project_of(t));
            let offered = acts(t.status, t.not_on_main(), t.branch.is_some());
            let first = offered.first().copied();
            h_flex()
                .flex_wrap()
                .gap_2()
                .children(offered.into_iter().enumerate().map(|(i, act)| {
                    let enabled = !act.answers() || can;
                    let label = match (act, rules::needs_way(t), way) {
                        (Act::Accept, true, Some(w)) => t!("peek.accept_way", way = w.label()),
                        _ => act.label(way),
                    };
                    let (handle, card) = (handle.clone(), card.clone());
                    btn(p, ("card-act", i), label, Some(act.key()), Some(act) == first && act != Act::Open, enabled).h(px(26.)).when(enabled, |b| {
                        b.on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            let Some(shell) = handle.upgrade() else { return };
                            let said = shell.update(cx, |s, cx| s.card_act(id, act, window, cx));
                            let _ = card.update(cx, |c, cx| c.after(id, said, cx));
                        })
                    })
                }))
                .into_any_element()
        }
    };
    // A hand-in's pictures, small, over what was said of it.
    let pictures = matches!(t.status, TicketStatus::Review | TicketStatus::Done).then(|| super::visuals::strip(shots, crate::render::hash_of(("card-shots", id)), id, &t.images, None)).flatten();
    // Its middle: the pictures, what was said, the facts — scrolled when the
    // window is too short for the whole card; its head and buttons stay.
    let middle: Vec<AnyElement> = pictures.map(|s| div().flex_none().child(s).into_any_element()).into_iter().chain(excerpt.map(IntoElement::into_any_element)).chain(facts.map(|f| f.flex_none().into_any_element())).collect();
    v_flex()
        .min_h_0()
        .p_3()
        .gap_2()
        .child(div().flex_none().child(head))
        .child(div().flex_none().child(title))
        .when(!middle.is_empty(), |c| {
            let (above, below) = fades(-f32::from(body.offset().y), f32::from(body.max_offset().y));
            let card = card.clone();
            let middle = v_flex()
                .id(("card-middle", id as u64))
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(body)
                .on_scroll_wheel(move |_, _, cx| {
                    let _ = card.update(cx, |_, cx| cx.notify());
                })
                .gap_2()
                .children(middle);
            c.child(v_flex().relative().min_h_0().child(middle).when(above, |d| d.child(fade(true))).when(below, |d| d.child(fade(false))))
        })
        .child(div().flex_none().pt_1().child(foot))
        .into_any_element()
}

/// The undo for an action taken on a card, where the work screens' own
/// isn't drawn: at the top, under the header, for its five seconds.
fn undo_toast(shell: &Shell, acted: &HashSet<TicketId>, handle: &WeakEntity<Shell>) -> Option<AnyElement> {
    if shell.screen == Screen::Board {
        return None;
    }
    let pending = shell.board.undo.latest().filter(|p| acted.contains(&p.ticket))?;
    let p = shell.palette;
    let left = rules::Undo::seconds_left(pending, Instant::now());
    let handle = handle.clone();
    Some(
        h_flex()
            .absolute()
            .top(px(60.))
            .left_0()
            .right_0()
            .justify_center()
            .child(crate::motion::rise(
                h_flex()
                    .id("card-undo")
                    .occlude()
                    .h(px(36.))
                    .pl_4()
                    .pr_2()
                    .gap_3()
                    .items_center()
                    .rounded(px(R_MD))
                    .bg(p.primary)
                    .text_color(p.on_primary)
                    .shadow(crate::motion::float_shadow(1.))
                    .text_size(px(SMALL))
                    .child(pending.text.clone())
                    .child(
                        h_flex()
                            .id("card-undo-button")
                            .h(px(26.))
                            .px_2()
                            .gap_2()
                            .items_center()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.8))
                            .on_click(move |_, _, cx| {
                                let _ = handle.update(cx, |s, cx| s.board_undo(cx));
                            })
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(t!("board.undo")))
                            .child(div().opacity(0.6).child(format!("({left})"))),
                    ),
                ("card-undo-in", pending.ticket as u64),
            ))
            .into_any_element(),
    )
}

// ── what the buttons do ──

impl Shell {
    /// Do `act` on ticket `id` from its card, by the Inbox peek's rules.
    /// What it did, for the card to say — or `None` when it went to the
    /// ticket (the panel beside the conversation, the board's peek, its
    /// diff, a question to answer first).
    pub fn card_act(&mut self, id: TicketId, act: Act, window: &mut Window, cx: &mut Context<Self>) -> Option<String> {
        let t = self.model.tickets.iter().find(|t| t.id == id).cloned()?;
        let way = self.merge_way_of(&self.project_of(&t));
        let merges = Model::merges(&t);
        match act {
            Act::Accept | Act::Take if self.can_act() => {
                if act == Act::Accept && rules::needs_way(&t) && way.is_none() {
                    // The first A in a project asks what accepting lands: on the board.
                    self.card_to_board(id, cx);
                    self.board.flow = Some(super::board::Flow::Way { ticket: id, remember: true });
                    cx.notify();
                    return None;
                }
                let way = if rules::needs_way(&t) { way } else { None };
                self.land(&t, way, cx);
                done_words(act, merges, way).map(str::to_owned)
            }
            Act::Merge if self.can_act() => match way {
                Some(w @ (MergeWay::Merge | MergeWay::Push)) => {
                    self.land(&t, Some(w), cx);
                    done_words(act, merges, Some(w)).map(str::to_owned)
                }
                _ => {
                    self.card_to_board(id, cx);
                    self.board.flow = Some(super::board::Flow::Method(id));
                    cx.notify();
                    None
                }
            },
            Act::Drop if self.can_act() => {
                let command = equipe_core::protocol::Command::Cancel { squad: self.model.squad.clone(), ticket: id };
                self.hold(id, t!("board.toast_drop", id = id), vec![command], cx);
                done_words(act, merges, None).map(str::to_owned)
            }
            Act::RequestChanges => {
                self.card_to_board(id, cx);
                self.board_open_reply(window, cx);
                None
            }
            Act::Diff => {
                self.card_to_board(id, cx);
                self.board_open_diff(window, cx);
                None
            }
            // A wait's card is answered beside the conversation; without asks, on the board.
            Act::Answer if self.screen == Screen::Chat && !t.asks().is_empty() => {
                self.side_open(id, cx);
                self.turn_focus(id, window, cx);
                None
            }
            Act::Answer => {
                self.card_to_board(id, cx);
                self.board_open_reply(window, cx);
                None
            }
            Act::Open if self.screen == Screen::Chat => {
                self.side_open(id, cx);
                None
            }
            Act::Open => {
                self.card_to_board(id, cx);
                None
            }
            _ => None,
        }
    }

    /// The ticket picked on the board, peeking.
    fn card_to_board(&mut self, id: TicketId, cx: &mut Context<Self>) {
        self.board_reveal(id);
        self.show(Screen::Board, cx);
        self.board_changed(cx);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use equipe_core::protocol::{TicketId, TicketStatus};

    use super::{Act, CLOSE_AFTER, Hover, OPEN_AFTER, Phase, RESULT_FOR, Rect, SCROLL_QUIET, Side, Target, act_for_key, acts, done_words, place};
    use crate::board::MergeWay;

    const WIN: (f32, f32) = (1200., 800.);

    fn link(x: f32, y: f32) -> Rect {
        Rect::new(x, y, 40., 20.)
    }

    fn target(id: TicketId) -> Target {
        Target { id, anchor: link(100., 100.), prefer: Side::Below }
    }

    #[test]
    fn the_card_goes_below_its_link_and_flips_where_there_is_no_room() {
        assert_eq!(place(link(100., 100.), (380., 230.), WIN, Side::Below), (100., 126., Side::Below));
        // Near the bottom: above, its foot a gap over the link.
        assert_eq!(place(link(100., 700.), (380., 230.), WIN, Side::Below), (100., 700. - 6. - 230., Side::Above));
        // Asked above near the top: below.
        assert_eq!(place(link(100., 40.), (380., 230.), WIN, Side::Above), (100., 66., Side::Below));
        // Near the right edge: pulled in, the margin kept.
        assert_eq!(place(link(1100., 100.), (380., 230.), WIN, Side::Below).0, 1200. - 8. - 380.);
        // No room either way: the side with more, kept inside the window.
        let (_, y, side) = place(link(100., 420.), (380., 900.), WIN, Side::Below);
        assert_eq!((y, side), (8., Side::Above));
    }

    #[test]
    fn a_sidebar_row_has_its_card_to_the_right_unless_the_window_ends() {
        let row = Rect::new(8., 300., 240., 32.);
        assert_eq!(place(row, (380., 230.), WIN, Side::Right), (254., 300., Side::Right));
        let near_edge = Rect::new(900., 700., 240., 32.);
        let (x, y, side) = place(near_edge, (380., 230.), WIN, Side::Right);
        assert_eq!(side, Side::Left);
        assert_eq!(x, 900. - 6. - 380.);
        assert_eq!(y, 800. - 8. - 230., "kept inside the window's foot");
    }

    #[test]
    fn the_card_shows_after_the_pointer_rests_and_goes_after_it_leaves() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        assert!(h.enter(target(5), t0));
        assert!(!h.tick(t0 + Duration::from_millis(200)), "not yet");
        assert_eq!(h.next_wake(t0 + Duration::from_millis(200)), Some(Duration::from_millis(150)));
        assert!(h.tick(t0 + OPEN_AFTER));
        assert_eq!(h.shown().map(|t| t.id), Some(5));
        h.card = Some(Rect::new(100., 126., 380., 230.));
        // Down into the card, across the gap: still there.
        assert!(!h.moved(110., 123., false, t0 + Duration::from_millis(400)));
        assert!(!h.moved(200., 200., false, t0 + Duration::from_millis(420)));
        assert!(h.takes_keys(), "the pointer on the card: its keys work");
        // Out: it goes a moment later, not at once.
        assert!(!h.moved(700., 600., false, t0 + Duration::from_millis(500)));
        assert!(!h.tick(t0 + Duration::from_millis(600)));
        assert!(h.tick(t0 + Duration::from_millis(500) + CLOSE_AFTER));
        assert_eq!(h.phase, Phase::Idle);
    }

    #[test]
    fn coming_back_in_time_keeps_it() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        h.enter(target(5), t0);
        h.tick(t0 + OPEN_AFTER);
        h.card = Some(Rect::new(100., 126., 380., 230.));
        h.moved(700., 600., false, t0 + Duration::from_millis(400));
        h.moved(110., 105., false, t0 + Duration::from_millis(450));
        assert!(!h.tick(t0 + Duration::from_millis(900)));
        assert!(h.shown().is_some());
        assert!(!h.takes_keys(), "resting on the link leaves typing alone");
    }

    #[test]
    fn leaving_before_the_wait_is_over_shows_nothing() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        h.enter(target(5), t0);
        assert!(h.moved(300., 300., false, t0 + Duration::from_millis(100)));
        assert!(!h.tick(t0 + OPEN_AFTER));
        assert_eq!(h.phase, Phase::Idle);
    }

    #[test]
    fn from_one_card_to_the_next_link_is_at_once() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        h.enter(target(5), t0);
        h.tick(t0 + OPEN_AFTER);
        let epoch = h.epoch;
        assert!(h.enter(Target { id: 6, anchor: link(300., 100.), prefer: Side::Below }, t0 + Duration::from_millis(500)));
        assert_eq!(h.shown().map(|t| t.id), Some(6));
        assert_eq!(h.epoch, epoch + 1, "its own entrance");
    }

    #[test]
    fn nothing_opens_while_scrolling_or_selecting() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        h.scrolled(t0);
        assert!(!h.enter(target(5), t0 + Duration::from_millis(100)), "the conversation is moving under the pointer");
        assert!(h.enter(target(5), t0 + SCROLL_QUIET + Duration::from_millis(1)));
        assert!(h.scrolled(t0 + Duration::from_millis(700)), "a wait is dropped by a scroll");
        // A selection being dragged across a link.
        let t1 = t0 + Duration::from_secs(2);
        h.moved(100., 100., true, t1);
        assert!(!h.enter(target(5), t1));
        h.moved(110., 105., false, t1 + Duration::from_millis(10));
        assert!(h.enter(target(5), t1 + Duration::from_millis(20)));
        // A drag that starts while it waits drops the wait.
        assert!(h.moved(112., 105., true, t1 + Duration::from_millis(30)));
        assert_eq!(h.phase, Phase::Idle);
    }

    #[test]
    fn a_shown_card_goes_on_scroll_esc_or_a_press_away_from_it() {
        let t0 = Instant::now();
        let open = || {
            let mut h = Hover::default();
            h.enter(target(5), t0);
            h.tick(t0 + OPEN_AFTER);
            h.card = Some(Rect::new(100., 126., 380., 230.));
            h
        };
        let mut h = open();
        assert!(h.scrolled(t0 + Duration::from_secs(1)));
        let mut h2 = open();
        assert!(h2.close());
        assert!(!h2.close(), "already away");
        let mut h3 = open();
        assert!(!h3.pressed(200., 200.), "a press on the card is its own");
        assert!(h3.pressed(700., 700.));
        // The pointer on it: a scroll there leaves it.
        let mut h4 = open();
        h4.moved(200., 200., false, t0 + Duration::from_millis(400));
        assert!(!h4.scrolled(t0 + Duration::from_millis(410)));
    }

    #[test]
    fn opened_from_the_keyboard_it_stays_where_the_pointer_is_not() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        h.open_now(target(9));
        assert!(h.takes_keys());
        h.card = Some(Rect::new(100., 126., 380., 230.));
        h.moved(900., 700., false, t0);
        assert!(!h.tick(t0 + Duration::from_secs(1)));
        assert_eq!(h.shown().map(|t| t.id), Some(9));
    }

    #[test]
    fn after_an_action_it_says_so_then_goes() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        h.enter(target(5), t0);
        h.tick(t0 + OPEN_AFTER);
        h.acted(t0 + Duration::from_secs(1));
        assert!(matches!(h.phase, Phase::Done { .. }));
        assert!(!h.takes_keys(), "no second action by a second key");
        assert!(!h.tick(t0 + Duration::from_secs(2)));
        assert!(h.tick(t0 + Duration::from_secs(1) + RESULT_FOR));
        assert_eq!(h.phase, Phase::Idle);
    }

    #[test]
    fn each_state_offers_the_peeks_answers_and_always_open() {
        use Act::*;
        assert_eq!(acts(TicketStatus::Review, false, true), [Accept, RequestChanges, Diff, Open]);
        assert_eq!(acts(TicketStatus::Review, false, false), [Accept, RequestChanges, Open], "no branch, no diff");
        assert_eq!(acts(TicketStatus::Done, true, true), [Merge, Open], "accepted, not in main");
        assert_eq!(acts(TicketStatus::Done, false, true), [Open]);
        assert_eq!(acts(TicketStatus::Waiting, false, false), [Answer, Open]);
        assert_eq!(acts(TicketStatus::Proposed, false, false), [Take, Drop, Open]);
        assert_eq!(acts(TicketStatus::InProgress, false, true), [Open]);
        assert_eq!(acts(TicketStatus::Todo, false, false), [Open]);
        assert!(!Diff.answers() && !Open.answers() && Accept.answers() && Drop.answers());
    }

    #[test]
    fn keys_pick_among_what_is_offered() {
        let review = acts(TicketStatus::Review, false, true);
        assert_eq!(act_for_key("a", &review), Some(Act::Accept));
        assert_eq!(act_for_key("r", &review), Some(Act::RequestChanges));
        assert_eq!(act_for_key("d", &review), Some(Act::Diff));
        assert_eq!(act_for_key("enter", &review), Some(Act::Open));
        assert_eq!(act_for_key("n", &review), None);
        let proposed = acts(TicketStatus::Proposed, false, false);
        assert_eq!(act_for_key("a", &proposed), Some(Act::Take));
        assert_eq!(act_for_key("n", &proposed), Some(Act::Drop));
        assert_eq!(act_for_key("a", &acts(TicketStatus::Done, true, true)), Some(Act::Merge));
        assert_eq!(act_for_key("r", &acts(TicketStatus::Waiting, false, false)), Some(Act::Answer));
        assert_eq!(act_for_key("x", &review), None);
    }

    #[test]
    fn the_card_says_what_was_done() {
        assert_eq!(done_words(Act::Accept, true, Some(MergeWay::Merge)), Some("수락했어요 · main에 합쳤어요"));
        assert_eq!(done_words(Act::Accept, false, None), Some("수락했어요"));
        assert_eq!(done_words(Act::Merge, true, Some(MergeWay::Push)), Some("main에 합치고 push했어요"));
        assert_eq!(done_words(Act::Open, false, None), None);
    }
    #[test]
    fn on_the_way_to_the_card_other_links_underneath_dont_take_it_away() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        h.enter(target(5), t0);
        h.tick(t0 + OPEN_AFTER);
        // The card sits wider than its link and a little off to the right.
        h.card = Some(Rect::new(100., 126., 380., 230.));
        // Diagonally across, past the link's own width, over another T-n.
        assert!(!h.moved(300., 118., false, t0 + Duration::from_millis(400)));
        assert!(!h.enter(Target { id: 9, anchor: Rect::new(290., 112., 40., 14.), prefer: Side::Below }, t0 + Duration::from_millis(400)));
        assert_eq!(h.shown().map(|t| t.id), Some(5), "still the card it was heading for");
        assert!(!h.tick(t0 + Duration::from_millis(400) + CLOSE_AFTER), "the way in isn't leaving");
        assert!(!h.moved(320., 200., false, t0 + Duration::from_millis(450)));
        assert!(h.takes_keys());
    }

    #[test]
    fn a_link_away_from_the_card_still_shows_its_own() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        h.enter(target(5), t0);
        h.tick(t0 + OPEN_AFTER);
        h.card = Some(Rect::new(100., 126., 380., 230.));
        h.moved(600., 40., false, t0 + Duration::from_millis(400));
        assert!(h.enter(Target { id: 9, anchor: Rect::new(590., 34., 40., 14.), prefer: Side::Below }, t0 + Duration::from_millis(400)));
        assert_eq!(h.shown().map(|t| t.id), Some(9));
    }

    #[test]
    fn resting_on_the_card_keeps_it_however_long() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        h.enter(target(5), t0);
        h.tick(t0 + OPEN_AFTER);
        h.card = Some(Rect::new(100., 126., 380., 230.));
        h.moved(200., 200., false, t0 + Duration::from_millis(400));
        assert!(!h.tick(t0 + Duration::from_secs(60)));
        assert!(h.shown().is_some());
        // A wheel on the card scrolls it; the card stays.
        assert!(!h.scrolled(t0 + Duration::from_secs(61)));
        assert!(h.shown().is_some());
    }

    #[test]
    fn a_wheel_on_the_card_keeps_it_and_one_off_it_puts_it_away() {
        let t0 = Instant::now();
        let open = || {
            let mut h = Hover::default();
            h.enter(target(5), t0);
            h.tick(t0 + OPEN_AFTER);
            h.card = Some(Rect::new(100., 126., 380., 230.));
            h
        };
        // On the card, anywhere on it — its text, its head, its buttons: the
        // wheel scrolls it, however many turns, and it stays.
        for (x, y) in [(200., 200.), (101., 127.), (479., 355.)] {
            let mut h = open();
            h.moved(x, y, false, t0 + Duration::from_millis(400));
            for turn in 0..20 {
                assert!(!h.scrolled(t0 + Duration::from_millis(410 + turn * 16)), "a wheel at {x},{y}");
            }
            assert!(h.shown().is_some() && h.takes_keys());
            assert!(!h.tick(t0 + Duration::from_secs(5)));
        }
        // Off it, over the conversation: the conversation scrolls, its link
        // moves away, and the card goes at once.
        let mut h = open();
        h.moved(900., 600., false, t0 + Duration::from_millis(400));
        assert!(h.scrolled(t0 + Duration::from_millis(410)));
        assert_eq!(h.phase, Phase::Idle);
        // Resting on its link (not the card): the same.
        let mut h = open();
        h.moved(110., 105., false, t0 + Duration::from_millis(400));
        assert!(h.scrolled(t0 + Duration::from_millis(410)));
    }

    #[test]
    fn a_card_above_its_link_is_reached_by_going_up_into_it() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        let anchor = Rect::new(860., 690., 22., 25.);
        h.enter(Target { id: 23, anchor, prefer: Side::Below }, t0);
        h.tick(t0 + OPEN_AFTER);
        // A tall card: no room below, so above, clamped to the window's top —
        // its head over the window's header and banner.
        let (x, y, side) = place(anchor, (380., 676.), WIN, Side::Below);
        assert_eq!((y, side), (8., Side::Above));
        assert!(y + 676. <= anchor.y, "its foot over the link");
        h.card = Some(Rect::new(x, y, 380., 676.));
        // Up from the link, across the gap, into the card, up to its head.
        let mut at = t0 + Duration::from_millis(400);
        for py in [700., 690., 686., 680., 600., 400., 200., 60., 20., 10.] {
            at += Duration::from_millis(16);
            assert!(!h.moved(870., py, false, at), "moving up through {py}");
            assert!(matches!(h.phase, Phase::Open { left: None, .. }), "not leaving at {py}");
        }
        assert!(h.takes_keys(), "on the card");
        assert!(!h.tick(at + CLOSE_AFTER + Duration::from_millis(1)));
        assert_eq!(h.shown().map(|t| t.id), Some(23));
        // A wheel on its head: still there.
        assert!(!h.scrolled(at + Duration::from_millis(500)));
        assert!(h.shown().is_some());
    }

    #[test]
    fn a_card_taller_than_the_window_is_held_inside_it() {
        assert_eq!(super::tallest(800.), 800. - 16.);
        assert_eq!(super::tallest(40.), 120., "a window that short isn't squeezed to nothing");
        // Clamped to the window, the card is placed whole inside it, whichever side.
        let h = super::tallest(WIN.1);
        for anchor in [link(100., 100.), link(100., 400.), link(100., 760.)] {
            let (_, y, _) = place(anchor, (380., h), WIN, Side::Below);
            assert!(y >= 8. && y + h <= WIN.1 - 8. + 0.01, "{anchor:?}: {y}");
        }
    }

    #[test]
    fn the_cards_text_takes_the_wheel_until_its_end() {
        use super::text_takes;
        // At the top of 120 more: down is the text's, up is not (nowhere to go).
        assert!(text_takes(0., -40., 120.));
        assert!(!text_takes(0., 40., 120.));
        // In the middle: both ways.
        assert!(text_takes(-60., -40., 120.) && text_takes(-60., 40., 120.));
        // At the end: down goes on to the card's middle, up is the text's.
        assert!(!text_takes(-120., -40., 120.));
        assert!(text_takes(-120., 40., 120.));
        // Turns piled up past the end within a frame: still on to the middle.
        assert!(!text_takes(-200., -40., 120.));
        // Text that all shows never takes it.
        assert!(!text_takes(0., -40., 0.) && !text_takes(0., 40., 0.));
    }

    #[test]
    fn a_cut_edge_fades_and_an_end_reached_does_not() {
        assert_eq!(super::fades(0., 0.), (false, false), "all of it shows");
        assert_eq!(super::fades(0., 120.), (false, true), "more below, at the top");
        assert_eq!(super::fades(60., 120.), (true, true), "in the middle");
        assert_eq!(super::fades(120., 120.), (true, false), "at the end");
    }
}
