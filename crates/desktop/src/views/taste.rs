//! The taste board, drawn: a question whose options are families of
//! references, browsed like a pinboard in the main area. Under the header, a
//! row of family filters ("All · Dense tool · Calm finance …") that stays put
//! while the pictures scroll; below it each family — its name, its one line
//! and "Pick this family" — over a masonry of its tiles, every one at its
//! picture's own shape. A tile shows where it is from and what to take from it
//! under the pointer (or the keys); a click opens it big in the picture viewer,
//! ←/→ through the board; the heart likes it, several allowed. At the foot, a
//! note and Send: the answer goes back as the family picked, the tiles liked
//! and the note. The rules are `crate::taste`'s; the likes and the pick live
//! in the question's draft, so the decision card shows the same.

use std::cell::Cell;
use std::rc::Rc;

use equipe_core::protocol::decide::{Question, QuestionSet, Tile};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::decide::Took;
use super::visuals::Shots;
use crate::icons::{Icon, icon};
use crate::render;
use crate::taste::{self as rules, Board, Effect, Key, Layout, TileRef};
use crate::theme::{META, MONO, R_LG, R_MD, SMALL};
use crate::ui::{self, Kind};
use crate::visuals::{Size, Viewer};
use crate::{Screen, Shell};

/// The board's side padding, and the room over its first family.
const PAD: f32 = 24.;
const TOP: f32 = 16.;
/// How wide the board is taken to be before it was drawn once.
const GUESS: f32 = 900.;
/// A tile's corners.
const RADIUS: f32 = R_LG;

/// The board's state that lives with the window.
pub struct TasteUi {
    pub board: Option<Board>,
    /// Where the person was when they opened it: Esc goes back there.
    pub from: Option<Screen>,
    pub scroll: ScrollHandle,
    /// The tile last brought into sight.
    seen: Rc<Cell<Option<TileRef>>>,
}

impl Default for TasteUi {
    fn default() -> Self {
        Self { board: None, from: None, scroll: ScrollHandle::new(), seen: Rc::default() }
    }
}

/// The key the board's width is kept under.
fn width_key() -> u64 {
    render::hash_of("taste-board-width")
}

/// The tiles' shapes, family by family, as their pictures are known now.
fn shapes(shell: &Shell, q: &Question) -> Vec<Vec<f32>> {
    let shots = shell.shots.borrow();
    q.options.iter().map(|c| c.board_tiles().iter().map(|t| rules::ratio_of(shots.dims(&t.image))).collect()).collect()
}

/// The board as it is laid out now, for the family filtered to (or all).
pub fn layout_of(shell: &Shell, q: &Question, only: Option<usize>) -> Layout {
    let width = shell.shots.borrow().width(width_key()).unwrap_or(GUESS);
    rules::layout(width, only, &shapes(shell, q))
}

/// The set and question the board is on, if both are still there and asked as a board.
fn context(shell: &Shell) -> Option<(QuestionSet, Question)> {
    let board = shell.taste.board.as_ref()?;
    let set = shell.model.asked.get(board.set)?.clone();
    let step = shell.decide.drafts.get(&board.set).map_or(board.step, |d| d.step);
    let q = set.questions.get(step).filter(|q| q.wants_board())?.clone();
    Some((set, q))
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let handle = cx.entity().downgrade();
    let back = ui::button(p, "taste-back", Kind::Ghost, t!("taste.back")).icon(Icon::ArrowLeft).tip(t!("taste.back_tip"), Some("Esc")).on_click(cx.listener(|this, _, window, cx| this.close_taste(window, cx)));
    let header = |title: String, extra: Option<AnyElement>| {
        ui::title_bar(h_flex())
            .h(px(super::header::HEIGHT))
            .flex_none()
            .pr_4()
            .pl(px(shell.header_inset(16., cx)))
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(p.hair)
            .child(div().flex_none().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("taste.title")))
            .child(div().flex_1().min_w_0().truncate().text_size(px(SMALL)).text_color(p.muted).child(title))
            .children(extra)
    };
    let page = |header: Div, body: AnyElement| v_flex().id("taste-page").flex_1().min_h_0().bg(p.bg).child(header).child(body).into_any_element();
    let (Some(board), Some((set, q))) = (shell.taste.board.as_ref(), context(shell)) else {
        // Answered or closed meanwhile, or not a board: the way back.
        let body = v_flex().flex_1().items_center().justify_center().gap_3().child(div().text_size(px(13.)).text_color(p.muted).child(t!("taste.gone"))).child(back).into_any_element();
        return page(header(String::new(), None), body);
    };
    let draft = shell.decide.drafts.get(&set.id).cloned().unwrap_or_else(|| crate::decide::Draft::new(&set));
    let shots = Shots::of(shell, handle.clone(), cx);
    let layout = layout_of(shell, &q, board.filter);
    let liked = draft.liked_count();
    let others = shell.decide_others(set.id, cx);
    let answered = draft.answered(&others);
    let (words, _) = q.shown();

    // The header: where from, the question, how many liked, Send.
    let count = (liked > 0).then(|| h_flex().flex_none().gap_1p5().items_center().text_size(px(SMALL)).text_color(p.accent).child(icon(Icon::Heart, 14., p.accent)).child(t!("taste.liked_count", count = liked)).into_any_element());
    let bar = header(words, None).children(count).child(div().flex_none().child(back));

    // The filter row: it stays while the pictures scroll.
    let chip = |id: &'static str, n: usize, label: String, on: bool, only: Option<usize>| {
        div()
            .id((id, n))
            .flex_none()
            .h(px(28.))
            .px_3()
            .flex()
            .items_center()
            .rounded_full()
            .border_1()
            .cursor_pointer()
            .text_size(px(SMALL))
            .font_weight(FontWeight::MEDIUM)
            .map(|c| if on { c.bg(p.accent_bg).border_color(p.accent).text_color(p.accent) } else { c.border_color(p.hair).text_color(p.text2).hover(move |s| s.bg(p.wash)) })
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.taste_filter(only, cx)))
    };
    let mut chips = h_flex().flex_1().min_w_0().gap_2().items_center().flex_wrap().child(chip("taste-all", 0, t!("taste.all").to_string(), board.filter.is_none(), None));
    for (n, option) in q.options.iter().enumerate() {
        chips = chips.child(chip("taste-family", n, option.shown().0, board.filter == Some(n), Some(n)));
    }
    let filters = h_flex()
        .flex_none()
        .w_full()
        .px(px(PAD))
        .py_2()
        .gap_3()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .bg(p.bg)
        .child(chips)
        .child(div().flex_none().max_w(px(360.)).truncate().text_size(px(META)).text_color(p.muted).child(t!("taste.keys")));

    // The board: each family's header and tiles where the layout puts them.
    let mut items: Vec<AnyElement> = Vec::new();
    for group in &layout.groups {
        items.push(family_header(shell, &q, group.option, group.y, &draft, cx));
    }
    for placed in &layout.tiles {
        let (option, tile) = (placed.at.option, placed.at.tile);
        let Some(t) = q.options.get(option).and_then(|c| c.board_tiles().get(tile).cloned()) else { continue };
        items.push(tile_view(shell, &shots, set.id, placed.at, placed.rect, &t, draft.liked(option, tile), board.cursor == Some(placed.at), cx));
    }
    follow(&shell.taste.scroll, &shell.taste.seen, board.cursor, &layout);
    let surface = shots.measured(div().relative().w_full().h(px(layout.height)), width_key()).children(items);
    let scroller = div().id("taste-scroll").flex_1().min_h_0().w_full().overflow_y_scroll().track_scroll(&shell.taste.scroll).child(div().w_full().px(px(PAD)).pt(px(TOP)).pb(px(32.)).child(surface));

    // The foot: a note beside the answer, and Send again in reach of the thumb.
    let field = shell.decide.fields.get(&(set.id, draft.step)).cloned();
    let typing = field.as_ref().is_some_and(|f| !f.read(cx).value().trim().is_empty());
    let note = field.map(|field| {
        h_flex()
            .id(("taste-note", set.id))
            .flex_1()
            .min_w_0()
            .px_3()
            .py(px(5.))
            .gap_2()
            .items_center()
            .rounded(px(R_MD))
            .border_1()
            .border_color(if typing { p.accent } else { p.hair })
            .when(typing, |r| r.bg(p.accent_bg))
            .child(icon(Icon::Pencil, 13., p.muted))
            .child(div().flex_1().min_w_0().text_size(px(14.)).child(super::composer::answer_box(&field, super::composer::AttachTo::Answer(set.id, draft.step), handle.clone())))
            .child(super::decide::key_badge(p, "Tab", false))
            .into_any_element()
    });
    let picked = q.options.iter().enumerate().filter(|(n, _)| draft.picked(*n)).map(|(_, c)| c.shown().0).collect::<Vec<_>>().join(", ");
    let foot = h_flex()
        .flex_none()
        .w_full()
        .px(px(PAD))
        .py_3()
        .gap_3()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .bg(p.bg)
        .children((!picked.is_empty()).then(|| div().flex_none().max_w(px(220.)).truncate().text_size(px(SMALL)).text_color(p.text2).child(t!("taste.picked_family", family = picked))))
        .children(note)
        .child(send_again(shell, answered, cx));
    let body = v_flex().flex_1().min_h_0().child(filters).child(scroller).child(foot).into_any_element();
    page(bar, body)
}

/// The send button of the foot (the header's is the same).
fn send_again(shell: &Shell, answered: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    ui::button(p, "taste-send-foot", Kind::Primary, t!("taste.send")).key("⌘↵").disabled(!answered || !shell.can_act()).on_click(cx.listener(|this, _, window, cx| this.taste_send(window, cx))).into_any_element()
}

/// A family's header: its name, whether it is the one recommended or the one
/// picked, its line, and "Pick this family".
fn family_header(shell: &Shell, q: &Question, option: usize, y: f32, draft: &crate::decide::Draft, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let Some(choice) = q.options.get(option) else { return div().into_any_element() };
    let (label, recommended) = choice.shown();
    let picked = draft.picked(option);
    let count = choice.board_tiles().len();
    let likes = draft.likes.get(&draft.step).map_or(0, |l| l.iter().filter(|l| l.option == option).count());
    let pick = ui::button(p, ("taste-pick", option), if picked { Kind::Primary } else { Kind::Secondary }, if picked { t!("taste.family_picked") } else { t!("taste.pick_family") })
        .icon(if picked { Icon::Check } else { Icon::Plus })
        .key("P")
        .disabled(!shell.can_act())
        .on_click(cx.listener(move |this, _, _, cx| this.taste_pick(option, cx)));
    h_flex()
        .absolute()
        .left_0()
        .top(px(y))
        .w_full()
        .h(px(rules::HEADER))
        .gap_3()
        .items_center()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().min_w_0().truncate().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(label))
                        .when(recommended, |r| r.child(super::visuals::badge(p, false)))
                        .child(div().flex_none().text_size(px(META)).text_color(p.muted).child(if likes > 0 { format!("{} · {}", t!("taste.tiles", count = count), t!("taste.liked_count", count = likes)) } else { t!("taste.tiles", count = count).to_string() })),
                )
                .children((!choice.description.trim().is_empty()).then(|| div().text_size(px(13.)).line_height(px(18.)).text_color(p.text2).line_clamp(2).text_ellipsis().child(choice.description.trim().to_owned()))),
        )
        .child(div().flex_none().child(pick))
        .into_any_element()
}

/// One tile: its picture at its own shape, a heart to like it, and — under
/// the pointer or the keys — where it is from and what to take.
fn tile_view(shell: &Shell, shots: &Shots, set: u64, at: TileRef, rect: rules::Rect, tile: &Tile, liked: bool, cursor: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let key = render::hash_of(("taste-tile", set, at.option, at.tile));
    let group: SharedString = format!("taste-tile-{}-{}", at.option, at.tile).into();
    let look = shots.ask(&tile.image, Size::Thumb, cx);
    let face = super::visuals::picture(shots, key, &tile.image, look, rect.w, rect.h, RADIUS, false, 0.);
    let host = tile.host();
    let said = !host.is_empty() || !tile.take.trim().is_empty();
    let caption = said.then(|| {
        v_flex()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .px(px(10.))
            .pt(px(22.))
            .pb(px(9.))
            .gap(px(2.))
            .bg(linear_gradient(180., linear_color_stop(rgba(0x00000000), 0.), linear_color_stop(rgba(0x000000CC), 1.)))
            .when(!cursor, |c| c.invisible().group_hover(group.clone(), |s| s.visible()))
            .children((!host.is_empty()).then(|| div().truncate().font_family(MONO).text_size(px(11.)).text_color(rgba(0xFFFFFFCC)).child(host)))
            .children((!tile.take.trim().is_empty()).then(|| div().text_size(px(12.5)).line_height(px(17.)).font_weight(FontWeight::MEDIUM).text_color(rgb(0xFFFFFF)).line_clamp(3).text_ellipsis().child(tile.take.trim().to_owned())))
    });
    let heart = div()
        .id(("taste-like", key))
        .absolute()
        .top(px(8.))
        .right(px(8.))
        .size(px(30.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .map(|h| if liked { h.bg(p.accent) } else { h.bg(rgba(0x0000008C)).hover(|s| s.bg(rgba(0x000000BF))) })
        .when(!liked && !cursor, |h| h.invisible().group_hover(group.clone(), |s| s.visible()))
        .child(icon(Icon::Heart, 15., if liked { p.on_accent } else { rgb(0xFFFFFF).into() }))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.taste_like(at, cx);
        }));
    let heart = crate::tips::tip(heart, p, if liked { t!("taste.unlike") } else { t!("taste.like") }, Some("Space"));
    div()
        .id(("taste-tile", key))
        .group(group)
        .absolute()
        .left(px(rect.x))
        .top(px(rect.y))
        .w(px(rect.w))
        .h(px(rect.h))
        .rounded(px(RADIUS))
        .overflow_hidden()
        .cursor_pointer()
        .child(face)
        .children(caption)
        .child(heart)
        // The keys' tile, ringed over the picture; a liked one, quietly.
        .child(div().absolute().inset_0().rounded(px(RADIUS)).border_2().border_color(if cursor { p.accent } else { rgba(0x00000000) }))
        .on_click(cx.listener(move |this, _, _, cx| this.taste_open_tile(at, cx)))
        .into_any_element()
}

/// The board scrolled to the tile the keys moved to, if it isn't in sight.
fn follow(scroll: &ScrollHandle, seen: &Cell<Option<TileRef>>, cursor: Option<TileRef>, layout: &Layout) {
    let before = seen.replace(cursor);
    if before == cursor {
        return;
    }
    let Some(rect) = cursor.and_then(|c| layout.rect_of(c)) else { return };
    let offset = scroll.offset();
    let view = scroll.bounds();
    let (room, scrolled, margin) = (f32::from(view.size.height), -f32::from(offset.y), 12.);
    if room <= 0. {
        return;
    }
    let top = rect.y + TOP;
    let to = if top - margin < scrolled {
        top - margin
    } else if top + rect.h + margin > scrolled + room {
        top + rect.h + margin - room
    } else {
        return;
    };
    scroll.set_offset(point(offset.x, px(-to.max(0.))));
}

impl Shell {
    /// Open the board of set `id`'s question on show, over the screen the
    /// person is on. Nothing when that question isn't one that can be browsed.
    pub fn open_taste(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(set) = self.model.asked.get(id) else { return };
        let step = self.decide.drafts.get(&id).map_or(0, |d| d.step);
        if !set.questions.get(step).is_some_and(|q| q.wants_board()) {
            return;
        }
        if let Some(draft) = self.decide.drafts.get_mut(&id) {
            draft.collapsed = false;
        }
        let from = if self.screen == Screen::Taste { self.taste.from } else { Some(self.screen) };
        self.taste = TasteUi { board: Some(Board::new(id, step)), from, ..TasteUi::default() };
        self.show(Screen::Taste, cx);
        window.blur(cx);
        cx.notify();
    }

    /// Back to where the board was opened from.
    pub fn close_taste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen != Screen::Taste {
            return;
        }
        let back = self.taste.from.filter(|s| *s != Screen::Taste).unwrap_or(Screen::Chat);
        self.taste.board = None;
        self.viewer = None;
        self.show(back, cx);
        if back == Screen::Chat {
            self.composer.focus(window, cx);
        }
        cx.notify();
    }

    /// One family's pictures only, or all of them: the keys on the first on show.
    pub fn taste_filter(&mut self, only: Option<usize>, cx: &mut Context<Self>) {
        let Some((_, q)) = context(self) else { return };
        let layout = layout_of(self, &q, only);
        if let Some(board) = self.taste.board.as_mut() {
            board.filter_to(only, &layout.tiles);
        }
        let offset = self.taste.scroll.offset();
        self.taste.scroll.set_offset(point(offset.x, px(0.)));
        self.taste.seen.set(None);
        cx.notify();
    }

    /// Like tile `at`, or take the like back.
    pub fn taste_like(&mut self, at: TileRef, cx: &mut Context<Self>) {
        let Some((set, _)) = context(self) else { return };
        if let Some(draft) = self.decide.drafts.get_mut(&set.id) {
            draft.like(&set, at.option, at.tile);
        }
        if let Some(board) = self.taste.board.as_mut() {
            board.cursor = Some(at);
        }
        cx.notify();
    }

    /// Pick family `option`, or take the pick back.
    pub fn taste_pick(&mut self, option: usize, cx: &mut Context<Self>) {
        let Some((set, _)) = context(self) else { return };
        if let Some(draft) = self.decide.drafts.get_mut(&set.id) {
            draft.toggle(&set, option);
        }
        cx.notify();
    }

    /// Tile `at` big, with ←/→ through the tiles on show.
    pub fn taste_open_tile(&mut self, at: TileRef, cx: &mut Context<Self>) {
        let Some((set, q)) = context(self) else { return };
        let only = self.taste.board.as_ref().and_then(|b| b.filter);
        let order: Vec<(usize, usize)> = layout_of(self, &q, only).order().into_iter().map(|t| (t.option, t.tile)).collect();
        let step = self.decide.drafts.get(&set.id).map_or(0, |d| d.step);
        if let Some(board) = self.taste.board.as_mut() {
            board.cursor = Some(at);
        }
        if let Some(viewer) = Viewer::of_board(set.id, step, &q, &order, (at.option, at.tile)) {
            self.open_viewer(viewer, cx);
        }
    }

    /// The answer, as it stands: the family picked, the tiles liked, the note.
    /// From the last question it goes to the member and the board closes; from
    /// another, the next question shows on its card.
    pub fn taste_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((set, _)) = context(self) else { return };
        if !self.can_act() {
            return;
        }
        let others = self.decide_others(set.id, cx);
        let Some(draft) = self.decide.drafts.get_mut(&set.id) else { return };
        let before = draft.step;
        let sent = draft.next(&set, &others);
        let moved = draft.step != before;
        if sent.is_none() && !moved {
            return;
        }
        self.close_taste(window, cx);
        if let Some(answers) = sent {
            self.decide_send(set.id, answers, window, cx);
        }
        cx.notify();
    }

    /// A keystroke, before anyone else sees it, while the board is on screen.
    pub fn on_taste_key(&mut self, key: &str, m: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> Took {
        if self.screen != Screen::Taste {
            return Took::Pass;
        }
        let Some(board) = self.taste.board.clone() else { return Took::Pass };
        if m.control || m.alt || m.function {
            return Took::Pass;
        }
        let in_note = self.decide.fields.get(&(board.set, board.step)).is_some_and(|f| f.read(cx).focus_handle(cx).is_focused(window));
        let Some(k) = rules::key_of(key, m.platform, m.shift) else {
            return if in_note { Took::Typing } else { Took::Pass };
        };
        if in_note {
            match k {
                // Out of the box, back to the pictures.
                Key::Note | Key::Back => window.blur(cx),
                Key::Send => self.taste_send(window, cx),
                _ => return Took::Typing,
            }
            cx.notify();
            return Took::Taken;
        }
        self.taste_key(k, window, cx);
        Took::Taken
    }

    /// Key `k` on the board: what it does, done.
    pub fn taste_key(&mut self, k: Key, window: &mut Window, cx: &mut Context<Self>) {
        let Some((set, q)) = context(self) else {
            if k == Key::Back {
                self.close_taste(window, cx);
            }
            return;
        };
        let Some(mut board) = self.taste.board.clone() else { return };
        let layout = layout_of(self, &q, board.filter);
        let effect = board.key(k, &layout.tiles, q.options.len());
        self.taste.board = Some(board);
        match effect {
            Effect::Nothing => {}
            Effect::Moved => cx.notify(),
            Effect::Like(at) => self.taste_like(at, cx),
            Effect::Open(at) => self.taste_open_tile(at, cx),
            Effect::Pick(option) => self.taste_pick(option, cx),
            Effect::Send => self.taste_send(window, cx),
            Effect::Back => self.close_taste(window, cx),
            Effect::Filter(only) => self.taste_filter(only, cx),
            Effect::Note => {
                if let Some(field) = self.decide.fields.get(&(set.id, self.decide.drafts.get(&set.id).map_or(0, |d| d.step))).cloned() {
                    field.update(cx, |s, cx| s.focus(window, cx));
                }
            }
        }
    }

    /// Dev tool (`snapshot.rs`): a key by name — `left`, `space`, `3`, `cmd-enter`.
    #[cfg(feature = "snapshot")]
    pub fn taste_script_key(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (cmd, name) = match name.strip_prefix("cmd-") {
            Some(rest) => (true, rest),
            None => (false, name),
        };
        if let Some(k) = rules::key_of(name, cmd, false) {
            self.taste_key(k, window, cx);
        }
    }

    /// Dev tool (`snapshot.rs`): the board of the card on show, or the set picked in Needs you.
    #[cfg(feature = "snapshot")]
    pub fn taste_script_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = match self.picked_need() {
            Some(crate::state::Need::Questions(id)) if self.screen == Screen::Board => Some(id),
            _ => match self.board.view.selected {
                Some(crate::state::Need::Questions(id)) => Some(id),
                _ => self.decide_shown().or_else(|| self.model.asked.open().find(|s| s.questions.iter().any(|q| q.wants_board())).map(|s| s.id)),
            },
        };
        if let Some(id) = id {
            self.open_taste(id, window, cx);
        }
    }

    /// Dev tool (`snapshot.rs`): the nth tile on show (from 1) liked, or opened big.
    #[cfg(feature = "snapshot")]
    pub fn taste_script_tile(&mut self, n: usize, open: bool, cx: &mut Context<Self>) {
        let Some((_, q)) = context(self) else { return };
        let only = self.taste.board.as_ref().and_then(|b| b.filter);
        let Some(at) = layout_of(self, &q, only).order().get(n.saturating_sub(1)).copied() else { return };
        if open {
            self.taste_open_tile(at, cx);
        } else {
            self.taste_like(at, cx);
        }
    }

    /// Dev tool (`snapshot.rs`): family `n` (from 1) picked.
    #[cfg(feature = "snapshot")]
    pub fn taste_script_pick(&mut self, n: usize, cx: &mut Context<Self>) {
        self.taste_pick(n.saturating_sub(1), cx);
    }

    /// The viewer's like: the tile on show liked, or the like taken back.
    pub fn viewer_like(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.viewer.as_ref() else { return };
        let item = viewer.item();
        if let (crate::visuals::Owner::Board { .. }, Some(option), Some(tile)) = (&viewer.owner, item.choice, item.tile) {
            self.taste_like(TileRef { option, tile }, cx);
        }
    }

    /// The viewer's "Pick this family".
    pub fn viewer_pick_family(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.viewer.as_ref() else { return };
        if let (crate::visuals::Owner::Board { .. }, Some(option)) = (&viewer.owner, viewer.item().choice) {
            self.taste_pick(option, cx);
        }
    }
}
