//! The designer studio (`crate::studio`): watch a ticket's design work as it
//! happens. Under the title, a line that says honestly how it is going
//! ("Working · 12m", "Waiting for you: pick a direction", "Checks failing:
//! Contrast — …", "Handed in"); then the steps of the process — brief,
//! references, directions, build, checks, hand-in — each lit from what the
//! member did, with how much and when; in the middle, what it is making: the
//! latest picture it drew, at each of the seven widths a check drew it at;
//! under it, what it left on the way, newest first — references and what to
//! learn from them, directions and the pick, critiques, each check and second
//! look, each hand-in and why it was refused — any of them opened big in the
//! middle. At the right, the member's conversation docked (as on the design
//! screen, `views/design_talk.rs`), to say "make this calmer" while looking;
//! the live browser a click away while it browses. Opened from a ticket's
//! card, the member's lane in Orchestration, or its conversation's header;
//! `studio:<ticket>` in the snapshot script. Nothing here changes anything:
//! the folder is read through the daemon (`design_studio`), the pictures
//! through `read_image`.

use equipe_core::protocol::rpc;
use equipe_core::protocol::studio::{Studio, StudioAsk};
use equipe_core::protocol::{TicketId, TicketStatus};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::visuals::{Shots, picture};
use crate::design_talk::DesignContext;
use crate::icons::{Icon, icon};
use crate::state::Member;
use crate::studio::{self as rules, Centre, Item, ItemKind, Mark, Step, Tone, Watch};
use crate::theme::{META, MONO, Palette, R_LG, R_MD, SMALL, SMALL_LINE};
use crate::tips::TipExt as _;
use crate::visuals::{Size, Viewer};
use crate::{Screen, Shell, ui};

/// The page's side padding.
const PAD: f32 = 24.;
/// A timeline card's width, and its picture's height.
const CARD_W: f32 = 204.;
const CARD_PIC_H: f32 = 112.;
/// The centre's picture at most this tall.
const STAGE_H: f32 = 380.;
/// Under this, the steps sit two to a row.
pub const NARROW: f32 = 720.;

/// The colour a tone is said in.
fn tone_color(p: Palette, tone: Tone) -> Rgba {
    let tones = crate::render::tones(&p);
    match tone {
        Tone::Plain => p.muted,
        Tone::Good => tones.added,
        Tone::Bad => p.danger(),
        Tone::Wait => p.orange,
        Tone::Busy => p.amber,
    }
}

/// A step's mark in words, for its tooltip and a screen reader.
pub fn mark_words(mark: Mark) -> &'static str {
    match mark {
        Mark::Ahead => t!("studio.mark_ahead"),
        Mark::Skipped => t!("studio.mark_skipped"),
        Mark::Done => t!("studio.mark_done"),
        Mark::Now => t!("studio.mark_now"),
        Mark::Attention => t!("studio.mark_attention"),
        Mark::Waiting => t!("studio.mark_waiting"),
    }
}

/// A step as a screen reader says it: `Checks: Now — Floor 11/11`.
pub fn step_label(s: &Step) -> String {
    match &s.count {
        Some(count) => format!("{}: {} — {count}", s.kind.label(), mark_words(s.mark)),
        None => format!("{}: {}", s.kind.label(), mark_words(s.mark)),
    }
}

/// What the centre's caption says: the screen and when it was checked, or
/// the picture's name and when it was made.
pub fn caption(c: &Centre, now: u64) -> String {
    let when = crate::activity::ago(now.saturating_sub(c.at));
    match (&c.screen, &c.direction) {
        (Some(screen), _) => t!("studio.centre_check", screen = screen, when = when),
        (None, Some(name)) => t!("studio.centre_made", name = name, when = when),
        (None, None) => t!("studio.centre_made", name = crate::visuals::name_of(&c.shown), when = when),
    }
}

fn kind_icon(kind: ItemKind) -> Icon {
    match kind {
        ItemKind::Reference => Icon::Compass,
        ItemKind::Direction => Icon::PenTool,
        ItemKind::Critique => Icon::Message,
        ItemKind::Check => Icon::Check,
        ItemKind::Look => Icon::Eye,
        ItemKind::HandIn => Icon::Inbox,
    }
}

fn kind_words(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Reference => t!("studio.item_reference"),
        ItemKind::Direction => t!("studio.item_direction"),
        ItemKind::Critique => t!("studio.kind_critique"),
        ItemKind::Check => t!("check.title"),
        ItemKind::Look => t!("look.name"),
        ItemKind::HandIn => t!("studio.item_handin"),
    }
}

/// The screen.
pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let Some(id) = shell.studio.ticket else { return empty(shell, t!("studio.no_ticket").into(), cx) };
    let Some(ticket) = shell.model.tickets.iter().find(|t| t.id == id) else {
        return empty(shell, t!("studio.not_found", id = id), cx);
    };
    let now = crate::state::now_unix();
    let member = shell.studio_member();
    let files = shell.studio.files();
    let happenings = shell.studio_happenings();
    let working = member.is_some_and(|m| m.presence == crate::state::Presence::Working && m.ticket == Some(id));
    let question = shell.model.asked.open().find(|s| s.ticket == Some(id)).and_then(|s| s.questions.first()).map(|q| if q.header.trim().is_empty() { q.question.clone() } else { q.header.clone() });
    let waiting = rules::waiting_for(ticket, question.as_deref(), files, &happenings);
    let browsing = rules::browsing(&happenings, working);
    let w = Watch { ticket, happenings, files, working, waiting };
    let steps = rules::steps(&w);
    let status = rules::status(&w);
    let items = rules::items(&w.happenings, files);
    let narrow = shell.studio_room() < NARROW;

    // The title bar: what is watched, who is at it, and the ways out.
    let header = ui::title_bar(h_flex())
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pr_3()
        .pl(px(shell.header_inset(16., cx)))
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(icon(Icon::Eye, 15., p.text2))
        .child(div().flex_none().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("studio.title")))
        .child(div().flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{id}")))
        .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).text_color(p.text2).child(ticket.title.clone()))
        .children(member.map(|m| who(p, m)))
        .when(browsing, |bar| {
            bar.child(
                ui::button(p, "studio-browser", ui::Kind::Secondary, t!("studio.browser"))
                    .icon(Icon::Globe)
                    .tip(t!("studio.browser_tip"), Some("⌘⇧B"))
                    .on_click(cx.listener(|this, _, window, cx| this.studio_browser(window, cx))),
            )
        })
        .children(member.map(|m| {
            let session = m.session;
            ui::icon_button(p, "studio-chat", Icon::Message, t!("studio.conversation"), None).on_click(cx.listener(move |this, _, window, cx| this.select_member(session, window, cx)))
        }));

    // The line that says how it is going.
    let tone = rules::status_tone(&status);
    let pending = files.map_or(0, |f| f.pending.len());
    let status_line = h_flex()
        .id("studio-status")
        .flex_none()
        .w_full()
        .min_h(px(38.))
        .px(px(PAD))
        .py_2()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .role(Role::Status)
        .child(div().flex_none().size(px(8.)).rounded_full().bg(tone_color(p, tone)))
        .child(div().flex_1().min_w_0().text_size(px(SMALL)).line_height(px(SMALL_LINE)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(rules::status_words(&status, now)))
        .when(pending > 0, |row| row.child(div().flex_none().text_size(px(META)).text_color(p.muted).child(t!("studio.pending", count = pending))))
        .children(member.and_then(|m| m.engine).map(|e| super::switching::label(p, e)));

    let mut body = v_flex().id("studio-body").flex_1().min_h_0().w_full().overflow_y_scroll().track_scroll(&shell.studio.scroll).px(px(PAD)).pt_4().pb_10().gap_5();
    body = body.child(rail(p, &steps, narrow, now));
    body = body.child(stage(shell, id, files, &items, now, cx));
    body = body.child(timeline(shell, &items, now, cx));
    v_flex().id("studio-page").flex_1().min_w_0().min_h_0().bg(p.bg).child(header).child(status_line).child(body).into_any_element()
}

/// Nothing to watch: why, quietly.
fn empty(shell: &Shell, text: String, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let header = ui::title_bar(h_flex()).h(px(super::header::HEIGHT)).flex_none().pl(px(shell.header_inset(16., cx))).items_center().border_b_1().border_color(p.hair).child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("studio.title")));
    v_flex().flex_1().min_h_0().bg(p.bg).child(header).child(div().flex_1().flex().items_center().justify_center().px_6().text_size(px(SMALL)).text_color(p.muted).child(text)).into_any_element()
}

/// Who is at it: their face and name.
fn who(p: Palette, m: &Member) -> impl IntoElement + use<> {
    let (_, hue) = crate::theme::look(&m.role, &m.name);
    h_flex().flex_none().gap_1p5().items_center().child(crate::theme::avatar(p, &m.role, &m.name, 18.)).child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(p.member(hue)).child(m.name.clone()))
}

/// The steps, left to right; in a narrow room, two to a row.
fn rail(p: Palette, steps: &[Step], narrow: bool, now: u64) -> AnyElement {
    let mut row = h_flex().id("studio-rail").w_full().gap_2().flex_wrap().items_stretch().role(Role::List).aria_label(t!("studio.steps"));
    for (n, s) in steps.iter().enumerate() {
        let (fill, edge, ink) = match s.mark {
            Mark::Done => (p.raised, p.hair, p.text),
            Mark::Now => (p.accent_bg, p.accent, p.text),
            Mark::Attention => (p.danger_bg(), p.danger(), p.text),
            Mark::Waiting => (p.raised, p.orange, p.text),
            Mark::Ahead | Mark::Skipped => (p.bg, p.hair, p.muted),
        };
        let glyph: AnyElement = match s.mark {
            Mark::Done => icon(Icon::Check, 12., tone_color(p, Tone::Good)).into_any_element(),
            Mark::Now => div().size(px(8.)).rounded_full().bg(p.accent).into_any_element(),
            Mark::Attention => icon(Icon::Alert, 12., p.danger()).into_any_element(),
            Mark::Waiting => div().size(px(8.)).rounded_full().bg(p.orange).into_any_element(),
            Mark::Ahead => div().size(px(6.)).rounded_full().border_1().border_color(p.muted).into_any_element(),
            Mark::Skipped => div().w(px(8.)).h(px(1.5)).bg(p.muted).into_any_element(),
        };
        let when = s.at.map(|at| crate::activity::ago(now.saturating_sub(at)));
        let card = v_flex()
            .id(("studio-step", n))
            .flex_1()
            .min_w(px(if narrow { 140. } else { 112. }))
            .when(narrow, |c| c.flex_basis(px(160.)))
            .px_3()
            .py_2()
            .gap(px(2.))
            .rounded(px(R_MD))
            .bg(fill)
            .border_1()
            .border_color(edge)
            .role(Role::ListItem)
            .aria_label(step_label(s))
            .tip(p, s.detail.as_ref().map_or_else(|| s.kind.about().to_owned(), |d| format!("{} — {d}", s.kind.about())), None)
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(div().flex_none().size(px(14.)).flex().items_center().justify_center().child(glyph))
                    .child(div().text_size(px(META)).font_weight(FontWeight::SEMIBOLD).text_color(ink).child(format!("{}. {}", n + 1, s.kind.label()))),
            )
            .child(div().min_w_0().text_size(px(META)).line_height(px(16.)).text_color(if matches!(s.mark, Mark::Ahead | Mark::Skipped) { p.muted } else { p.text2 }).child(s.count.clone().unwrap_or_else(|| mark_words(s.mark).to_owned())))
            .children(when.map(|w| div().text_size(px(11.)).text_color(p.muted).child(w)));
        row = row.child(card);
    }
    row.into_any_element()
}

/// The middle: what it is making — or an item opened big.
fn stage(shell: &Shell, id: TicketId, files: Option<&Studio>, items: &[Item], now: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let shots = Shots::of(shell, cx.entity().downgrade(), cx);
    let frame = || v_flex().id("studio-stage").w_full().gap_3().p_4().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair);
    // An item opened: it, big, and the way back to the live picture.
    if let Some(item) = shell.studio.open.as_ref().and_then(|k| items.iter().find(|i| &i.key == k)) {
        let head = h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(icon(kind_icon(item.kind), 14., tone_color(p, item.tone)))
            .child(div().flex_none().text_size(px(META)).text_color(p.muted).child(kind_words(item.kind)))
            .child(div().flex_1().min_w_0().truncate().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(item.title.clone()))
            .child(ui::button(p, "studio-back", ui::Kind::Ghost, t!("studio.back_live")).icon(Icon::ArrowLeft).on_click(cx.listener(|this, _, _, cx| this.studio_open_item(None, cx))));
        let room = shots.width(rules_key("stage"), 720.) - 32.;
        let n = item.pictures.len().min(3);
        let tile_w = if n == 0 { 0. } else { ((room - 12. * (n as f32 - 1.)) / n as f32).max(120.) };
        let tile_h = if n == 1 { (tile_w * 0.62).min(STAGE_H) } else { 200. };
        let pictures = h_flex().w_full().gap_3().flex_wrap().children(item.pictures.iter().take(3).enumerate().map(|(k, path)| {
            let look = shots.ask(path, if n == 1 { Size::Full } else { Size::Thumb }, cx);
            let viewer = viewer_of(id, &item.pictures, path, &item.title);
            div()
                .id(("studio-open-pic", k))
                .cursor_pointer()
                .child(picture(&shots, crate::render::hash_of(("studio-open", path)), path, look, tile_w, tile_h, R_MD, false, 6.))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(v) = viewer.clone() {
                        this.open_viewer(v, cx);
                    }
                }))
        }));
        let lines = v_flex().w_full().gap_1().children(item.lines.iter().map(|l| div().text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.text2).child(l.clone())));
        let when = item.at.map(|at| crate::activity::ago(now.saturating_sub(at)));
        return shots.measured(frame(), rules_key("stage")).child(head).children(when.map(|w| div().text_size(px(META)).text_color(p.muted).child(w))).when(n > 0, |f| f.child(pictures)).child(lines).into_any_element();
    }
    let Some(files) = files else {
        let words = if shell.studio.asking || shell.studio.asked_at.is_none() { t!("studio.reading") } else { t!("studio.centre_none") };
        return frame().child(div().py_10().w_full().text_center().text_size(px(SMALL)).text_color(p.muted).child(words)).into_any_element();
    };
    let Some(centre) = rules::centre(files, shell.studio.width) else {
        let words = if files.dir.is_none() { t!("studio.no_folder") } else { t!("studio.centre_none") };
        return frame().child(div().py_10().w_full().text_center().text_size(px(SMALL)).text_color(p.muted).child(words)).into_any_element();
    };
    // The widths a check drew it at, to switch between.
    let widths = (centre.widths.len() > 1).then(|| {
        let mut seg = h_flex().id("studio-widths").flex_none().h(px(26.)).p(px(2.)).gap(px(2.)).rounded(px(R_MD)).bg(p.wash).role(Role::RadioGroup).aria_label(t!("studio.widths"));
        for (w, _) in &centre.widths {
            let (w, on) = (*w, centre.width == Some(*w));
            seg = seg.child(
                div()
                    .id(("studio-width", w as usize))
                    .h_full()
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(crate::theme::R_SM))
                    .text_size(px(META))
                    .cursor_pointer()
                    .role(Role::RadioButton)
                    .aria_toggled(if on { Toggled::True } else { Toggled::False })
                    .aria_label(t!("studio.width_tip", n = w))
                    .when(on, |d| d.bg(p.raised).text_color(p.text).shadow_sm())
                    .when(!on, |d| d.text_color(p.text2).hover(move |s| s.bg(p.wash)))
                    .child(w.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| this.studio_width(Some(w), cx))),
            );
        }
        seg
    });
    let working = shell.studio_member().is_some_and(|m| m.presence == crate::state::Presence::Working && m.ticket == Some(id));
    let live = h_flex()
        .flex_none()
        .gap_1()
        .items_center()
        .child(div().size(px(6.)).rounded_full().bg(if working { tone_color(p, Tone::Busy) } else { p.muted }))
        .child(div().text_size(px(META)).text_color(p.muted).child(if working { t!("studio.live") } else { t!("studio.latest") }));
    // Directions out, none picked: each, side by side, to pick from.
    if !centre.choices.is_empty() {
        let n = centre.choices.len().min(3);
        let room = (shots.width(rules_key("stage"), 720.) - 32.).max(200.);
        let tile_w = ((room - 12. * (n as f32 - 1.)) / n as f32).max(140.);
        let tile_h = (tile_w * 0.62).min(STAGE_H);
        let all: Vec<String> = centre.choices.iter().map(|(_, p)| p.clone()).collect();
        let row = h_flex().w_full().gap_3().flex_wrap().children(centre.choices.iter().take(3).enumerate().map(|(k, (name, path))| {
            let look = shots.ask(path, Size::Full, cx);
            let viewer = viewer_of(id, &all, path, name);
            v_flex()
                .id(("studio-choice", k))
                .gap_1p5()
                .cursor_pointer()
                .child(picture(&shots, crate::render::hash_of(("studio-choice", path)), path, look, tile_w, tile_h, R_MD, false, 0.))
                .child(div().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(name.clone()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(v) = viewer.clone() {
                        this.open_viewer(v, cx);
                    }
                }))
        }));
        let head = h_flex().w_full().gap_3().items_center().child(live).child(div().flex_1().min_w_0().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(t!("studio.choices", count = centre.choices.len())));
        return shots.measured(frame(), rules_key("stage")).child(head).child(row).into_any_element();
    }
    let rel = files.dir.as_deref().and_then(|d| centre.shown.strip_prefix(d)).map(|r| r.trim_start_matches('/').to_owned()).unwrap_or_else(|| crate::visuals::name_of(&centre.shown).to_owned());
    let talk = super::design_talk::say(p, "studio-say", "studio-stage-pic", DesignContext::Asset { name: rel, path: centre.shown.clone() }, cx);
    let head = h_flex()
        .w_full()
        .gap_3()
        .flex_wrap()
        .items_center()
        .child(live)
        .child(div().flex_1().min_w(px(160.)).truncate().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(caption(&centre, now)))
        .children(widths);
    let room = (shots.width(rules_key("stage"), 720.) - 32.).max(200.);
    let dims = shots.best_dims(&centre.shown).unwrap_or((1280, 800));
    let (w, h) = crate::visuals::contain(dims, (room, STAGE_H));
    let look = shots.ask(&centre.shown, Size::Full, cx);
    let viewer = viewer_of(id, &centre.widths.iter().map(|(_, p)| p.clone()).collect::<Vec<_>>(), &centre.shown, &caption(&centre, now)).or_else(|| viewer_of(id, std::slice::from_ref(&centre.shown), &centre.shown, &caption(&centre, now)));
    let pic = div()
        .id("studio-stage-pic")
        .group("studio-stage-pic")
        .relative()
        .w_full()
        .flex()
        .justify_center()
        .cursor_pointer()
        .child(picture(&shots, crate::render::hash_of(("studio-stage", &centre.shown)), &centre.shown, look, w.max(80.), h.max(60.), R_MD, false, 0.))
        .child(div().absolute().top_2().right_2().child(talk))
        .tip(p, t!("studio.see_big"), None)
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(v) = viewer.clone() {
                this.open_viewer(v, cx);
            }
        }));
    shots.measured(frame(), rules_key("stage")).child(head).child(pic).into_any_element()
}

fn rules_key(name: &str) -> u64 {
    crate::render::hash_of(("studio", name))
}

/// The big view over `pictures`, open at `at`, each called `label` and its width.
fn viewer_of(id: TicketId, pictures: &[String], at: &str, label: &str) -> Option<Viewer> {
    let items: Vec<crate::visuals::Item> = pictures
        .iter()
        .map(|p| {
            let width = equipe_core::design::studio::width_of(p).map(|w| format!(" · {w}px")).unwrap_or_default();
            crate::visuals::Item { path: p.clone(), choice: None, label: format!("{label}{width}") }
        })
        .collect();
    let at = items.iter().position(|i| i.path == at)?;
    Some(Viewer { owner: crate::visuals::Owner::Ticket(id), items, at })
}

/// What it left on the way, newest first.
fn timeline(shell: &Shell, items: &[Item], now: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let shots = Shots::of(shell, cx.entity().downgrade(), cx);
    let head = h_flex()
        .gap_1p5()
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.text2)
        .child(t!("studio.timeline"))
        .when(!items.is_empty(), |h| h.child(div().text_color(p.muted).font_weight(FontWeight::NORMAL).child(items.len().to_string())));
    if items.is_empty() {
        return v_flex().gap_2().child(head).child(div().text_size(px(SMALL)).text_color(p.muted).child(t!("studio.timeline_empty"))).into_any_element();
    }
    // Newest first, wrapping: everything it left is in sight as the page scrolls.
    let mut strip = h_flex().id("studio-strip").w_full().gap_3().flex_wrap().items_start().role(Role::List).aria_label(t!("studio.timeline"));
    for (n, item) in items.iter().enumerate() {
        let open = shell.studio.open.as_deref() == Some(item.key.as_str());
        let key = item.key.clone();
        let pic = item.pictures.first().map(|path| {
            let look = shots.ask(path, Size::Thumb, cx);
            picture(&shots, crate::render::hash_of(("studio-card", path)), path, look, CARD_W, CARD_PIC_H, R_MD, true, 4.)
        });
        let when = item.at.map(|at| crate::activity::ago(now.saturating_sub(at)));
        let card = v_flex()
            .id(("studio-item", n))
            .flex_none()
            .w(px(CARD_W))
            .rounded(px(R_MD))
            .bg(p.raised)
            .border_1()
            .border_color(if open { p.accent } else { p.hair })
            .cursor_pointer()
            .hover(move |s| s.border_color(p.accent))
            .role(Role::ListItem)
            .aria_label(format!("{} · {}", kind_words(item.kind), item.title))
            .children(pic)
            .child(
                v_flex()
                    .p_2p5()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .child(icon(if item.tone == Tone::Bad { Icon::Alert } else { kind_icon(item.kind) }, 12., tone_color(p, item.tone)))
                            .child(div().flex_1().min_w_0().truncate().text_size(px(META)).text_color(p.muted).child(kind_words(item.kind)))
                            .children(when.map(|w| div().flex_none().text_size(px(11.)).text_color(p.muted).child(w))),
                    )
                    .child(div().min_w_0().truncate().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(item.title.clone()))
                    .children(item.lines.iter().take(2).map(|l| div().min_w_0().truncate().text_size(px(META)).text_color(p.text2).child(l.clone()))),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.studio_open_item(Some(key.clone()), cx)));
        strip = strip.child(card);
    }
    v_flex().w_full().gap_2().child(head).child(strip).into_any_element()
}

// ── what the screen does ──

impl Shell {
    /// The member at work on the ticket watched: the one on it now, else
    /// whoever it is assigned to.
    pub fn studio_member(&self) -> Option<&Member> {
        let id = self.studio.ticket?;
        let ticket = self.model.tickets.iter().find(|t| t.id == id);
        let on = self.model.members.iter().find(|m| m.ticket == Some(id));
        let named = ticket.and_then(|t| t.assignee.as_deref().or(t.owner.as_deref())).and_then(|n| self.model.members.iter().find(|m| m.name == n));
        let worked = self.model.convos.iter().find(|(_, c)| c.has_work_on(id)).and_then(|(s, _)| self.model.members.iter().find(|m| m.session == *s));
        on.or(worked).or(named)
    }

    /// What the member did on the ticket, from its conversation.
    pub fn studio_happenings(&self) -> Vec<rules::Happening> {
        let (Some(id), Some(m)) = (self.studio.ticket, self.studio_member()) else { return Vec::new() };
        self.model.convos.get(&m.session).map(|c| rules::happenings(&rules::work_of(&c.transcript, id))).unwrap_or_default()
    }

    /// The room beside the sidebar, less the conversation docked at its side.
    pub fn studio_room(&self) -> f32 {
        self.studio.room
    }

    /// Watch `ticket`'s design work: the studio, its folder asked about, the
    /// conversation beside it the member's.
    pub fn open_studio(&mut self, ticket: TicketId, window: &mut Window, cx: &mut Context<Self>) {
        self.studio.watch(ticket);
        self.show(Screen::Studio, cx);
        self.studio_ask(cx);
        if self.design_talk_open(window, cx) {
            self.design_talk_follow(window, cx);
        }
        cx.notify();
    }

    /// Each draw: the folder asked about again while the studio is open, and
    /// the conversation beside it the member's.
    pub fn studio_prepare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen != Screen::Studio {
            self.studio.poll = None;
            return;
        }
        let room = self.design_talk_room(window, cx);
        let docked = if self.design_talk_open(window, cx) { crate::design_talk::width(self.remembered.design_talk_width, room) } else { 0. };
        self.studio.room = room - docked;
        if self.studio.poll.is_none() {
            let poll = cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(rules::EVERY).await;
                    let on = this.update(cx, |this, cx| {
                        let on = this.screen == Screen::Studio;
                        if on {
                            this.studio_ask(cx);
                        }
                        on
                    });
                    if !matches!(on, Ok(true)) {
                        break;
                    }
                }
            });
            self.studio.poll = Some(poll);
        }
        if self.design_talk_open(window, cx) && self.design_talk_behind() {
            cx.defer_in(window, |this, window, cx| this.design_talk_follow(window, cx));
        }
    }

    /// Ask the daemon what the ticket's design folder holds now.
    pub fn studio_ask(&mut self, cx: &mut Context<Self>) {
        let (Some(ticket), false) = (self.studio.ticket, self.studio.asking) else { return };
        let Some(uploader) = self.link.as_ref().map(|l| l.uploader()) else { return };
        self.studio.asking = true;
        self.studio.asked_at = Some(std::time::Instant::now());
        let squad = self.model.squad.clone();
        let asked = cx.background_spawn(async move {
            uploader.call(rpc::DESIGN_STUDIO, StudioAsk { squad, ticket }, std::time::Duration::from_secs(20)).and_then(|v| serde_json::from_value::<Studio>(v).map_err(|e| e.to_string()))
        });
        cx.spawn(async move |this, cx| {
            let answer = asked.await;
            this.update(cx, |this, cx| {
                this.studio.asking = false;
                if let Ok(studio) = answer
                    && this.studio.ticket == Some(studio.ticket)
                {
                    this.studio_arrived(studio, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn studio_arrived(&mut self, studio: Studio, cx: &mut Context<Self>) {
        let same = self.studio.files().is_some_and(|f| f.stamp == studio.stamp);
        let changed = self.studio.arrived(studio);
        if !changed.is_empty() {
            self.shots.borrow_mut().refresh(&changed);
        }
        if !same {
            cx.notify();
        }
    }

    pub fn studio_width(&mut self, width: Option<u32>, cx: &mut Context<Self>) {
        self.studio.width = width;
        cx.notify();
    }

    /// A timeline item opened in the middle, or put back (`None`).
    pub fn studio_open_item(&mut self, key: Option<String>, cx: &mut Context<Self>) {
        self.studio.open = if self.studio.open == key { None } else { key };
        if self.studio.open.is_some() {
            self.studio.scroll.set_offset(point(px(0.), px(0.)));
        }
        cx.notify();
    }

    /// The live browser, where the member is browsing: its conversation, the panel open.
    pub fn studio_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.studio_member().map(|m| m.session) {
            self.model.select(session);
            self.refresh(window, cx);
        }
        if !self.browser.read(cx).open {
            self.toggle_browser(window, cx);
        } else {
            self.show(Screen::Chat, cx);
        }
    }

    /// Whether the studio is offered for `ticket` (its card, its lane, its conversation's header).
    pub fn studio_offered(&self, ticket: TicketId) -> bool {
        let Some(t) = self.model.tickets.iter().find(|t| t.id == ticket) else { return false };
        let role = t.assignee.as_deref().or(t.owner.as_deref()).and_then(|n| self.model.members.iter().find(|m| m.name == n)).map(|m| m.role.as_str());
        let worked = self.model.convos.values().find(|c| c.has_work_on(ticket)).map(|c| rules::happenings(&rules::work_of(&c.transcript, ticket))).unwrap_or_default();
        t.status != TicketStatus::Cancelled && rules::offered(t, role.or(t.owner.as_deref()), &worked)
    }

    /// The snapshot tool's `studio:<ticket>` (`snapshot.rs`); `studio:width:<n>`,
    /// `studio:item:<n>` (the nth on the timeline, opened), `studio:back`.
    pub fn snapshot_studio(&mut self, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        match what.split_once(':') {
            Some(("width", n)) => self.studio_width(n.parse().ok(), cx),
            Some(("item", n)) => {
                let files = self.studio.files().cloned();
                let items = rules::items(&self.studio_happenings(), files.as_ref());
                let key = n.parse::<usize>().ok().and_then(|n| items.get(n)).map(|i| i.key.clone());
                self.studio.open = None;
                self.studio_open_item(key, cx);
            }
            _ if what == "back" => self.studio_open_item(None, cx),
            _ => {
                if let Ok(id) = what.trim_start_matches("T-").parse() {
                    self.open_studio(id, window, cx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{caption, mark_words, step_label};
    use crate::studio::{Centre, Mark, Step, StepKind};

    #[test]
    fn a_step_says_where_it_stands_and_how_much_there_is() {
        let s = Step { kind: StepKind::Checks, mark: Mark::Attention, count: Some("기준선 10/11".into()), at: None, detail: None };
        assert_eq!(step_label(&s), "검사: 고칠 것 있음 — 기준선 10/11");
        let ahead = Step { kind: StepKind::HandIn, mark: Mark::Ahead, count: None, at: None, detail: None };
        assert_eq!(step_label(&ahead), "넘기기: 아직");
        for mark in [Mark::Ahead, Mark::Skipped, Mark::Done, Mark::Now, Mark::Attention, Mark::Waiting] {
            assert!(!mark_words(mark).is_empty());
        }
    }

    #[test]
    fn the_centre_says_what_it_shows_and_how_fresh_it_is() {
        let check = Centre { screen: Some("onboarding/welcome".into()), widths: vec![], shown: "/w/design/checks/onboarding/welcome/1280.png".into(), width: Some(1280), at: 1_000, direction: None, choices: vec![] };
        assert_eq!(caption(&check, 1_000 + 180), "onboarding/welcome · 3분 전 점검");
        let made = Centre { screen: None, widths: vec![], shown: "/w/design/mockups/welcome-v2.png".into(), width: None, at: 1_000, direction: None, choices: vec![] };
        assert_eq!(caption(&made, 1_000), "welcome-v2.png · 방금");
    }

    /// Nothing here changes anything: the studio only reads.
    #[test]
    fn the_studio_sends_no_command_and_asks_only_what_reads() {
        let source = include_str!("studio.rs");
        let code = &source[..source.find("#[cfg(test)]").unwrap()];
        for never in ["send_command(", "Command::", "self.send(", "send_as("] {
            assert!(!code.contains(never), "the studio uses {never}");
        }
        assert!(crate::readonly::reads_method(equipe_core::protocol::rpc::DESIGN_STUDIO), "the real view may ask it");
    }
}
