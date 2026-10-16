//! A member's pictures, drawn: each in one 4:3 box, whole (contain) on a
//! quiet light checkerboard (dimmer in the dark) so a transparent logo reads,
//! breathing while it comes and fading in when it does; a hand-in's gallery
//! over its note; a strip of small ones under a news line or on a ticket's
//! card; and the big view over the window — ←/→ through them, Esc away, and
//! for a decision's options the pick made right there (⏎, or its number).
//! What is known of each picture, and the layout rules, are `crate::visuals`'s.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use equipe_core::protocol::TicketId;
use equipe_core::t;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{CONTROL, ICON, MONO, Palette, R_LG, R_MD, R_SM};
use crate::tips::tip;
use crate::visuals::{self as rules, Look, Owner, Shot, Size, Viewer, Visuals};
use crate::Shell;

/// How long a picture that just came fades in.
const FADE: Duration = Duration::from_millis(240);
/// The checkerboard's squares, in points.
const SQUARE: u32 = 8;

/// What every picture of one frame draws with.
#[derive(Clone)]
pub struct Shots {
    pub p: Palette,
    shots: Rc<RefCell<Visuals>>,
    handle: WeakEntity<Shell>,
    reduce: bool,
}

impl Shots {
    pub fn of(shell: &Shell, handle: WeakEntity<Shell>, cx: &App) -> Self {
        Self { p: shell.palette, shots: shell.shots.clone(), handle, reduce: cx.reduce_motion() }
    }

    /// What `path` shows at `size`; the first time, it is asked for — once
    /// the frame is drawn, so nothing changes while it is.
    pub fn ask(&self, path: &str, size: Size, cx: &mut App) -> Look {
        let mut shots = self.shots.borrow_mut();
        let look = shots.look(path, size);
        if matches!(look, Look::Loading) && !shots.scheduled {
            shots.scheduled = true;
            let handle = self.handle.clone();
            cx.defer(move |cx| {
                handle.update(cx, |shell, cx| shell.load_shots(cx)).ok();
            });
        }
        look
    }

    pub fn handle(&self) -> WeakEntity<Shell> {
        self.handle.clone()
    }

    /// The grid `key` as last drawn: its width, or `guess` before it was.
    pub fn width(&self, key: u64, guess: f32) -> f32 {
        self.shots.borrow().width(key).unwrap_or(guess)
    }

    /// `el` tells how wide it was drawn, and is drawn again when that changed.
    pub fn measured<E: ParentElement + Sized + 'static>(&self, el: E, key: u64) -> E {
        use gpui_kit::base::ElementExt as _;
        let shots = self.shots.clone();
        el.on_prepaint(move |bounds, window, _| {
            if shots.borrow_mut().set_width(key, f32::from(bounds.size.width)) {
                window.request_animation_frame();
            }
        })
    }
}

/// Whether the palette is the dark one.
fn is_dark(p: Palette) -> bool {
    p.bg.r + p.bg.g + p.bg.b < 1.5
}

/// The checkerboards made, by size in points and theme.
type Boards = HashMap<(u32, u32, bool), Arc<Image>>;

/// A checkerboard `w`×`h` points, drawn at 2x in two quiet light
/// tones — made once for each size and theme.
fn checker(p: Palette, w: f32, h: f32) -> Option<Arc<Image>> {
    static MADE: OnceLock<Mutex<Boards>> = OnceLock::new();
    let (w, h, dark) = (w.round().max(1.) as u32, h.round().max(1.) as u32, is_dark(p));
    let made = MADE.get_or_init(Default::default);
    if let Some(image) = made.lock().ok()?.get(&(w, h, dark)) {
        return Some(image.clone());
    }
    // Light in both themes, as paper is: most marks are drawn dark on nothing, and must
    // read; in the dark a step dimmer, so a tile doesn't glare.
    let (a, b) = if dark { ([0xDA, 0xDA, 0xDF], [0xCC, 0xCC, 0xD3]) } else { ([0xFF, 0xFF, 0xFF], [0xEF, 0xEF, 0xF3]) };
    let board = image::RgbImage::from_fn(w * 2, h * 2, |x, y| image::Rgb(if (x / (SQUARE * 2) + y / (SQUARE * 2)).is_multiple_of(2) { a } else { b }));
    let mut png = std::io::Cursor::new(Vec::new());
    board.write_to(&mut png, image::ImageFormat::Png).ok()?;
    let image = Arc::new(Image::from_bytes(ImageFormat::Png, png.into_inner()));
    let mut made = made.lock().ok()?;
    // A window resized through many widths doesn't keep every one.
    if made.len() > 48 {
        made.clear();
    }
    made.insert((w, h, dark), image.clone());
    Some(image)
}

/// A breathing wash while a picture comes.
fn waiting(p: Palette, id: ElementId, reduce: bool) -> AnyElement {
    let base = div().absolute().inset_0().bg(p.wash);
    if reduce {
        return base.into_any_element();
    }
    base.with_animation(id, Animation::new(Duration::from_millis(1400)).repeat().with_easing(bounce(ease_in_out)), |el, t| el.opacity(0.35 + 0.4 * t)).into_any_element()
}

/// Where a picture can't be had: the file's name, quietly.
fn gone(p: Palette, path: &str, roomy: bool) -> AnyElement {
    v_flex()
        .absolute()
        .inset_0()
        .p_2()
        .gap_1()
        .items_center()
        .justify_center()
        .text_color(p.muted)
        .child(icon(Icon::File, ICON, p.muted))
        .when(roomy, |c| c.child(div().max_w_full().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(11.5)).child(rules::name_of(path).to_owned())))
        .when(roomy, |c| c.child(div().text_size(px(11.)).child(t!("visuals.gone"))))
        .into_any_element()
}

/// One picture whole in a `w`×`h` box on the checkerboard, `pad` points
/// clear of its edge; `top_only` rounds only its top corners (a tile's,
/// with its caption under it).
#[allow(clippy::too_many_arguments)]
pub fn picture(shots: &Shots, id: u64, path: &str, look: Look, w: f32, h: f32, radius: f32, top_only: bool, pad: f32) -> Div {
    let p = shots.p;
    let corners = |el: Div| if top_only { el.rounded_t(px(radius)) } else { el.rounded(px(radius)) };
    let board = checker(p, w, h).map(|board| {
        let board = img(board).absolute().top_0().left_0().w(px(w)).h(px(h)).object_fit(ObjectFit::Fill);
        if top_only { board.rounded_t(px(radius)) } else { board.rounded(px(radius)) }
    });
    let face: AnyElement = match &look {
        Look::Ready(shot) => {
            let (iw, ih) = ((w - 2. * pad).max(1.), (h - 2. * pad).max(1.));
            let image = img(shot.image.clone()).absolute().top(px(pad)).left(px(pad)).w(px(iw)).h(px(ih)).object_fit(ObjectFit::Contain);
            if !shots.reduce && shot.at.elapsed() < FADE * 2 {
                motion::fade(image, ElementId::NamedInteger("shot-in".into(), id), FADE).into_any_element()
            } else {
                image.into_any_element()
            }
        }
        Look::Gone => gone(p, path, w >= 100.),
        Look::Loading => waiting(p, ElementId::NamedInteger("shot-wait".into(), id), shots.reduce),
    };
    corners(div().relative().flex_none().w(px(w)).h(px(h)).overflow_hidden().bg(p.wash)).children(board).child(face)
}

/// The big view opens at `path` among `viewer`'s pictures.
fn opens(handle: &WeakEntity<Shell>, viewer: Option<Viewer>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let handle = handle.clone();
    move |_, _, cx| {
        cx.stop_propagation();
        if let Some(viewer) = viewer.clone() {
            handle.update(cx, |shell, cx| shell.open_viewer(viewer, cx)).ok();
        }
    }
}

/// A hand-in's pictures over its note: a grid of tiles, each its file's
/// name under it; one opens big. `guess`: how wide it is before it was drawn.
pub fn gallery(shell: &Shell, key: u64, ticket: TicketId, images: &[String], guess: f32, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if images.is_empty() {
        return None;
    }
    let shots = Shots::of(shell, cx.entity().downgrade(), cx);
    let p = shots.p;
    let width = shots.width(key, guess);
    let columns = rules::columns(width, images.len());
    let w = rules::tile_width(width, columns);
    let h = (w * rules::RATIO).floor();
    let tiles: Vec<AnyElement> = images
        .iter()
        .map(|path| {
            let id = key ^ rules_hash(path);
            let face = picture(&shots, id, path, shots.ask(path, Size::Thumb, cx), w, h, R_MD, true, 10.);
            let tile = v_flex()
                .id(("hand-in-shot", id))
                .relative()
                .w(px(w))
                .rounded(px(R_MD))
                .border_1()
                .border_color(p.hair)
                .bg(p.bg)
                .overflow_hidden()
                .cursor_pointer()
                .child(face)
                .child(div().px_2().py(px(6.)).border_t_1().border_color(p.hair).overflow_hidden().whitespace_nowrap().text_ellipsis().font_family(MONO).text_size(px(11.)).text_color(p.text2).child(rules::name_of(path).to_owned()))
                .on_click(opens(&shots.handle, Viewer::of_ticket(ticket, images, path)));
            lift(tile, ("hand-in-lift", id), p, false).into_any_element()
        })
        .collect();
    let grid = h_flex().w_full().flex_wrap().gap(px(rules::GAP)).children(tiles);
    let grid = shots.measured(grid, key);
    Some(
        v_flex()
            .gap_1p5()
            .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("visuals.pictures", count = images.len())))
            .child(grid)
            .into_any_element(),
    )
}

/// A few small pictures in a row — a news line's, a ticket card's — and
/// "+n" past [`rules::STRIP_MAX`]; one opens the big view. Without `cx`
/// (a view drawn while the window's state is read), what it asks for waits
/// for [`flush`].
pub fn strip(shots: &Shots, key: u64, ticket: TicketId, images: &[String], mut cx: Option<&mut App>) -> Option<AnyElement> {
    if images.is_empty() {
        return None;
    }
    let p = shots.p;
    let (w, h) = (rules::STRIP, (rules::STRIP * rules::RATIO).floor());
    let more = images.len().saturating_sub(rules::STRIP_MAX);
    let tiles: Vec<AnyElement> = images
        .iter()
        .take(rules::STRIP_MAX)
        .map(|path| {
            let id = key ^ rules_hash(path);
            let look = match cx.as_deref_mut() {
                Some(cx) => shots.ask(path, Size::Thumb, cx),
                None => shots.shots.borrow_mut().look(path, Size::Thumb),
            };
            let face = picture(shots, id, path, look, w, h, R_SM, false, 4.);
            let tile = div().id(("strip-shot", id)).rounded(px(R_SM)).border_1().border_color(p.hair).cursor_pointer().child(face).on_click(opens(&shots.handle, Viewer::of_ticket(ticket, images, path)));
            tip(tile, p, rules::name_of(path).to_owned(), None).into_any_element()
        })
        .collect();
    Some(
        h_flex()
            .gap_1p5()
            .items_center()
            .children(tiles)
            .when(more > 0, |r| r.child(div().text_size(px(11.5)).text_color(p.muted).child(format!("+{more}"))))
            .into_any_element(),
    )
}

/// What was asked for without a context, loaded: `shots` is the window's.
pub fn flush(shots: &Rc<RefCell<Visuals>>, shell: &WeakEntity<Shell>, cx: &mut App) {
    let mut held = shots.borrow_mut();
    if held.scheduled || !held.has_wanted() {
        return;
    }
    held.scheduled = true;
    let shell = shell.clone();
    cx.defer(move |cx| {
        shell.update(cx, |shell, cx| shell.load_shots(cx)).ok();
    });
}

/// A path's own number for element ids.
fn rules_hash(path: &str) -> u64 {
    crate::render::hash_of(path)
}

/// A tile that lifts a little under the pointer (and while `on`), its
/// shadow deepening with it; `ring`: the accent ring of one picked.
pub fn lift<E: IntoElement + Styled + 'static>(el: E, id: impl Into<ElementId>, p: Palette, ring: bool) -> motion::Hover<E> {
    let accent = p.accent;
    let dark = is_dark(p);
    motion::hover_with(el, id, false, move |el, t| {
        let mut shadows = Vec::new();
        if ring {
            shadows.push(BoxShadow { color: Rgba { a: 0.28, ..accent }.into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(3.), inset: false });
        }
        if t > 0. {
            let ink = if dark { 0.45 } else { 0.14 };
            shadows.push(BoxShadow { color: hsla(0., 0., 0., ink * t), offset: point(px(0.), px(6. * t)), blur_radius: px(16. * t), spread_radius: px(-4.), inset: false });
        }
        el.top(px(-2. * t)).shadow(shadows)
    })
}

// ── the big view ──

/// The picture shown big over the window, its name and where it is among
/// the others, ←/→ to them; a decision's option, with its pick.
pub fn viewer(shell: &Shell, window: &Window, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let viewer = shell.viewer.as_ref()?;
    let item = viewer.item().clone();
    let (at, n) = (viewer.at, viewer.items.len());
    let shots = Shots::of(shell, cx.entity().downgrade(), cx);
    let p = shell.palette;
    // Whole, asked for once it is opened; meanwhile its thumbnail.
    let whole = shots.ask(&item.path, Size::Full, cx);
    let thumb = shots.ask(&item.path, Size::Thumb, cx);
    let shown: Option<Shot> = shell.shots.borrow().best(&item.path);
    let viewport = window.viewport_size();
    let room = ((f32::from(viewport.width) - 96.).max(120.), (f32::from(viewport.height) - 150.).max(120.));
    let size = match shown.as_ref().and_then(|s| s.dims) {
        Some(dims) => rules::contain(dims, (room.0.min(dims.0 as f32 / 2. * 1.5).max(240.), room.1.min(dims.1 as f32 / 2. * 1.5).max(180.))),
        None => {
            let side = room.0.min(room.1).min(720.);
            (side, side)
        }
    };
    let face: AnyElement = match (&shown, &thumb) {
        (Some(shot), _) => {
            let board = checker(p, size.0, size.1).map(|b| img(b).absolute().top_0().left_0().w(px(size.0)).h(px(size.1)).object_fit(ObjectFit::Fill).rounded(px(10.)));
            div()
                .relative()
                .w(px(size.0))
                .h(px(size.1))
                .rounded(px(10.))
                .overflow_hidden()
                .children(board)
                .child(img(shot.image.clone()).absolute().top_0().left_0().w(px(size.0)).h(px(size.1)).object_fit(ObjectFit::Contain))
                // The whole picture still coming: a turning mark in its corner.
                .when(matches!(whole, Look::Loading), |d| d.child(div().absolute().right(px(10.)).bottom(px(10.)).child(motion::spinner(14., rgba(0x00000080)))))
                .into_any_element()
        }
        (None, Look::Gone) => v_flex().size(px(220.)).gap_2().rounded(px(R_LG)).bg(rgba(0xFFFFFF14)).items_center().justify_center().text_size(px(13.)).text_color(rgba(0xFFFFFFB3)).child(icon(Icon::File, 28., rgba(0xFFFFFFB3))).child(t!("visuals.gone")).into_any_element(),
        _ => div().size(px(220.)).flex().items_center().justify_center().child(motion::spinner(20., rgba(0xFFFFFFB3))).into_any_element(),
    };
    let round = |b: Stateful<Div>, glyph: Icon, label: &'static str, keys: &'static str| {
        tip(b.size(px(CONTROL)).flex().items_center().justify_center().rounded_full().cursor_pointer().hover(|s| s.bg(rgba(0xFFFFFF1F))).child(icon(glyph, ICON, rgb(0xFFFFFF))), p, label, Some(keys))
    };
    // A decision's option: its number, what it is called, and its pick.
    let decision = match (&viewer.owner, item.choice) {
        (Owner::Decision { set, step }, Some(choice)) => shell.model.asked.get(*set).and_then(|s| s.questions.get(*step)).map(|q| (*set, *step, choice, q.multi_select, q.options.get(choice).is_some_and(|o| rules::recommended(&o.label).1))),
        _ => None,
    };
    let picked = decision.is_some_and(|(set, step, choice, ..)| shell.decide.drafts.get(&set).is_some_and(|d| d.step == step && d.picked(choice)));
    let label = h_flex()
        .gap_2()
        .items_center()
        .min_w_0()
        .children(decision.map(|(_, _, choice, ..)| div().flex_none().min_w(px(20.)).h(px(20.)).px(px(5.)).flex().items_center().justify_center().rounded(px(5.)).border_1().border_color(rgba(0xFFFFFF4D)).font_family(MONO).text_size(px(11.)).text_color(rgb(0xFFFFFF)).child((choice + 1).to_string())))
        .child(div().min_w_0().max_w(px(360.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(rgb(0xFFFFFF)).child(item.label.clone()))
        .when(decision.is_some_and(|d| d.4), |r| r.child(badge(p, true)))
        .when(n > 1, |r| r.child(div().flex_none().text_size(px(12.)).text_color(rgba(0xFFFFFFB3)).child(format!("{}/{n}", at + 1))));
    let pick = decision.map(|(set, _, choice, multi, _)| {
        let label = if picked { t!("visuals.picked") } else { t!("visuals.pick") };
        div()
            .id("viewer-pick")
            .flex_none()
            .h(px(CONTROL))
            .px_3()
            .flex()
            .items_center()
            .gap_1p5()
            .rounded_full()
            .cursor_pointer()
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .map(|b| if picked { b.bg(p.accent).text_color(p.on_accent) } else { b.bg(rgb(0xFFFFFF)).text_color(rgb(0x111114)).hover(|s| s.bg(rgba(0xFFFFFFE0))) })
            .when(picked, |b| b.child(icon(Icon::Check, 13., p.on_accent)))
            .child(label)
            .child(div().font_family(MONO).text_size(px(11.)).opacity(0.6).child("⏎"))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.viewer_pick(set, choice, multi, window, cx);
            }))
    });
    let bar = h_flex()
        .id("viewer-bar")
        .flex_none()
        .h(px(44.))
        .max_w_full()
        .px_2()
        .gap_3()
        .items_center()
        .rounded(px(22.))
        .bg(rgba(0x2A2A2EF2))
        .border_1()
        .border_color(rgba(0xFFFFFF1F))
        .on_click(|_, _, cx| cx.stop_propagation())
        .when(n > 1, |b| {
            let prev = div().id("viewer-prev").on_click(cx.listener(|this, _, _, cx| this.step_viewer(false, cx)));
            b.child(round(prev, Icon::ArrowLeft, t!("composer.aria_previous"), "←"))
        })
        .child(label)
        .children(pick)
        .when(n > 1, |b| {
            let next = div().id("viewer-next").on_click(cx.listener(|this, _, _, cx| this.step_viewer(true, cx)));
            b.child(round(next, Icon::ArrowRight, t!("composer.aria_next"), "→"))
        })
        .child(round(div().id("viewer-close").on_click(cx.listener(|this, _, _, cx| this.close_viewer(cx))), Icon::X, t!("composer.aria_close"), "Esc"));
    let backdrop = div()
        .id("viewer")
        .absolute()
        .inset_0()
        .bg(rgba(0x000000EB))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_4()
        .p_8()
        .occlude()
        .on_click(cx.listener(|this, _, _, cx| this.close_viewer(cx)))
        .child(motion::rise(
            div().id("viewer-picture").flex_1().min_h_0().w_full().flex().items_center().justify_center().child(div().id("viewer-face").on_click(|_, _, cx| cx.stop_propagation()).child(face)),
            ElementId::Name(format!("viewer-{}", item.path).into()),
        ))
        .child(bar);
    Some(motion::fade(backdrop, "viewer-in", motion::BASE).into_any_element())
}

/// The mark of the option a member recommends: "추천" — on a picture
/// (`on_dark`: over the big view's dark) or a tile.
pub fn badge(p: Palette, on_dark: bool) -> impl IntoElement {
    div()
        .flex_none()
        .h(px(18.))
        .px(px(6.))
        .flex()
        .items_center()
        .rounded(px(9.))
        .bg(if on_dark { Rgba { a: 0.9, ..p.accent } } else { p.accent })
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.on_accent)
        .child(t!("visuals.recommended"))
}

impl Shell {
    /// Load what was asked about: each picture off the UI thread, from the
    /// daemon on this Mac or another — it gives only what the squad shows.
    pub fn load_shots(&mut self, cx: &mut Context<Self>) {
        let wanted = self.shots.borrow_mut().take_wanted();
        if wanted.is_empty() {
            return;
        }
        let generation = self.shots.borrow().generation;
        let Some(uploader) = self.link.as_ref().map(|l| l.uploader()) else {
            // Not on the line: asked again once it is.
            let mut shots = self.shots.borrow_mut();
            for (path, size) in wanted {
                shots.put(&path, size, Look::Gone);
            }
            return;
        };
        let squad = self.model.squad.clone();
        for (path, size) in wanted {
            let (uploader, squad, asked) = (uploader.clone(), squad.clone(), path.clone());
            let loaded = cx.background_spawn(async move {
                match uploader.read_image(&squad, &asked, Some(size.side())) {
                    Ok(read) => rules::shot(&read).map_or(Look::Gone, Look::Ready),
                    Err(_) => Look::Gone,
                }
            });
            cx.spawn(async move |this, cx| {
                let look = loaded.await;
                this.update(cx, |this, cx| this.shot_arrived(generation, &path, size, look, cx)).ok();
            })
            .detach();
        }
    }

    fn shot_arrived(&mut self, generation: u64, path: &str, size: Size, look: Look, cx: &mut Context<Self>) {
        let mut shots = self.shots.borrow_mut();
        if shots.generation != generation {
            return;
        }
        shots.put(path, size, look);
        drop(shots);
        cx.notify();
    }

    pub fn open_viewer(&mut self, viewer: Viewer, cx: &mut Context<Self>) {
        self.viewer = Some(viewer);
        super::ticket_card::put_away(cx);
        cx.notify();
    }

    /// Dev tool (`snapshot.rs`): the big view at option `n`'s picture of the
    /// card on show (or picked in 나에게), or at `ticket`'s nth hand-in picture.
    #[cfg(feature = "snapshot")]
    pub fn view_script(&mut self, ticket: Option<TicketId>, n: usize, cx: &mut Context<Self>) {
        let viewer = match ticket {
            Some(id) => self.model.tickets.iter().find(|t| t.id == id).and_then(|t| Viewer::of_ticket(id, &t.images, t.images.get(n.saturating_sub(1))?)),
            None => {
                let id = match self.picked_need() {
                    Some(crate::state::Need::Questions(id)) if self.screen == crate::Screen::Board => Some(id),
                    _ => self.decide_shown(),
                };
                id.and_then(|id| {
                    let step = self.decide.drafts.get(&id).map_or(0, |d| d.step);
                    let q = self.model.asked.get(id)?.questions.get(step)?.clone();
                    let path = q.options.get(n.saturating_sub(1))?.images.first()?.clone();
                    Viewer::of_question(id, step, &q, &path)
                })
            }
        };
        if let Some(viewer) = viewer {
            self.open_viewer(viewer, cx);
        }
    }

    pub fn close_viewer(&mut self, cx: &mut Context<Self>) {
        self.viewer = None;
        cx.notify();
    }

    pub fn step_viewer(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.step(forward);
            cx.notify();
        }
    }

    /// The option on show picked from the big view — the person's click or
    /// key, never anyone else's. One pick only: the view closes on it.
    pub fn viewer_pick(&mut self, set: u64, choice: usize, multi: bool, window: &mut Window, cx: &mut Context<Self>) {
        let on_step = self.decide.drafts.get(&set).map(|d| d.step);
        let Some(Owner::Decision { step, .. }) = self.viewer.as_ref().map(|v| v.owner.clone()) else { return };
        if on_step != Some(step) {
            return;
        }
        self.decide_pick(set, choice, cx);
        if !multi {
            self.viewer = None;
            // Back where the person answers: the keys go on there.
            if self.screen == crate::Screen::Chat {
                self.composer.focus(window, cx);
            }
        }
        cx.notify();
    }

    /// While a picture is shown big, the keys are its own: ←/→ through the
    /// others, ⏎ or a number picks a decision's option, Esc and Space close.
    /// Every plain key is taken, so nothing is typed underneath.
    pub fn on_viewer_key(&mut self, key: &str, m: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(viewer) = self.viewer.clone() else { return false };
        if m.platform || m.control || m.alt {
            return false;
        }
        let pick = |n: usize| match viewer.owner {
            Owner::Decision { set, step } => self.model.asked.get(set).and_then(|s| s.questions.get(step)).filter(|q| n < q.options.len()).map(|q| (set, n, q.multi_select)),
            Owner::Ticket(_) => None,
        };
        match key {
            "left" | "right" => self.step_viewer(key == "right", cx),
            "escape" | "space" => self.close_viewer(cx),
            "enter" => {
                if let Some((set, n, multi)) = viewer.item().choice.and_then(pick) {
                    self.viewer_pick(set, n, multi, window, cx);
                }
            }
            digit if digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit()) && digit != "0" => {
                let n = digit.parse::<usize>().unwrap_or(1) - 1;
                if let Some((set, n, multi)) = pick(n) {
                    if let Some(viewer) = self.viewer.as_mut() {
                        viewer.go_to_choice(n);
                    }
                    self.viewer_pick(set, n, multi, window, cx);
                }
            }
            _ => {}
        }
        true
    }
}
