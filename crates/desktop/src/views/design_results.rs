//! Design screen · Designs: what the squad's design tickets made, one card
//! each (`crate::designs`, read through the daemon from the project and the
//! ticket worktrees). The list shows each card's picture — the direction
//! chosen when a decision record names one, else the first mock — its title,
//! where it stands, when, and what it holds (directions, preview links,
//! change notes). A card opened shows the screen as it was beside each
//! direction at one scale (the decision card's side-by-side, the same big
//! view on a click), the change notes of `CHANGES.md` per direction, a button
//! for each live preview, the decision record and a way to the ticket. With
//! none yet, what the tab is for and how a render is found.

use equipe_core::protocol::TicketId;
use equipe_core::protocol::design::{DesignState, DesignWork};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::designs::{self, Line, Viewport};
use crate::icons::{Icon, icon};
use crate::render;
use crate::theme::{BODY, BODY_LINE, META, MONO, Palette, R_LG, R_MD, SMALL};
use crate::tips::TipExt as _;
use crate::ui::{self, Kind};
use crate::visuals::{self as visual, Ground, Item, Owner, Size, Viewer};
use crate::{Shell, motion};

use super::visuals::Shots;

const PAD: f32 = 32.;
const MAX_W: f32 = 1120.;
const CARD_W: f32 = 340.;
const HERO_H: f32 = 206.;
const STRIP_W: f32 = 132.;
const STRIP_H: f32 = 88.;

pub fn page(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let ui_ = &shell.design.results;
    let body: AnyElement = match &ui_.answer {
        None => loading(shell),
        Some(list) if list.is_empty() => empty(shell),
        Some(_) => match ui_.current() {
            Some(work) => detail(shell, work, cx),
            None => list(shell, cx),
        },
    };
    v_flex()
        .id("design-results")
        .flex_1()
        .min_h_0()
        .child(toolbar(shell, cx))
        .child(
            div()
                .id("design-results-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&ui_.scroll)
                .child(div().w_full().max_w(px(MAX_W)).mx_auto().px(px(PAD)).pt(px(8.)).pb(px(96.)).child(body)),
        )
        .into_any_element()
}

fn loading(shell: &Shell) -> AnyElement {
    let p = shell.palette;
    h_flex().w_full().py(px(80.)).gap_2().items_center().justify_center().text_size(px(SMALL)).text_color(p.muted).child(motion::spinner(16., p.muted)).child(t!("design.results_loading")).into_any_element()
}

/// What the list holds, and the way to read it again.
fn toolbar(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let count = shell.design.results.answer.as_ref().map_or(0, Vec::len);
    let fresh = shell.model.new_designs().count();
    h_flex()
        .flex_none()
        .w_full()
        .max_w(px(MAX_W))
        .mx_auto()
        .px(px(PAD))
        .pt(px(16.))
        .pb(px(8.))
        .gap_3()
        .items_center()
        .child(div().flex_none().text_size(px(SMALL)).text_color(p.muted).child(t!("design.results_count", count = count)))
        .when(fresh > 0, |bar| bar.child(chip(p, t!("design.results_new_count", count = fresh).to_string(), true)))
        .child(div().flex_1())
        .child(ui::icon_button(p, "design-results-refresh", Icon::Reload, t!("design.results_refresh"), None).on_click(cx.listener(|this, _, _, cx| this.designs_refresh(cx))))
}

/// A small label: where a card stands, "New".
fn chip(p: Palette, text: String, strong: bool) -> impl IntoElement {
    div().flex_none().h(px(20.)).px(px(8.)).flex().items_center().rounded(px(10.)).text_size(px(11.5)).font_weight(FontWeight::MEDIUM).when(strong, |c| c.bg(p.accent).text_color(p.on_accent)).when(!strong, |c| c.bg(p.wash).text_color(p.text2)).child(text)
}

/// Where a design ticket stands, as a pill in the colour of what it asks of the person.
fn state_pill(p: Palette, state: DesignState) -> impl IntoElement {
    let (word, tone, filled) = match state {
        DesignState::Working => (t!("design.results_state_working"), p.amber, false),
        DesignState::WaitingOnYou => (t!("design.results_state_waiting"), p.accent, true),
        DesignState::HandedIn => (t!("design.results_state_handed_in"), p.accent, false),
        DesignState::Accepted => (t!("design.results_state_accepted"), p.text2, false),
    };
    h_flex()
        .flex_none()
        .h(px(20.))
        .px(px(8.))
        .gap(px(5.))
        .items_center()
        .rounded(px(10.))
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .when(filled, |c| c.bg(tone).text_color(p.on_accent))
        .when(!filled, |c| c.border_1().border_color(p.hair).text_color(tone))
        .when(state == DesignState::Accepted, |c| c.child(icon(Icon::Check, 11., tone)))
        .child(word)
}

fn when_of(shell: &Shell, work: &DesignWork) -> Option<String> {
    (work.updated > 0).then(|| t!("design.results_updated", when = crate::activity::ago(shell.model.unix.saturating_sub(work.updated))).to_string())
}

// ── the cards ──

fn list(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let Some(answer) = shell.design.results.answer.as_ref() else { return loading(shell) };
    let cards: Vec<AnyElement> = answer.iter().map(|work| card(shell, work, cx)).collect();
    h_flex().w_full().flex_wrap().gap_4().items_start().children(cards).into_any_element()
}

fn card(shell: &Shell, work: &DesignWork, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ticket = work.ticket;
    let badges = designs::badges(work);
    let fresh = shell.model.new_designs().any(|n| n.ticket == ticket);
    let handle = cx.entity().downgrade();
    let shots = Shots::of(shell, handle, cx);
    let hero = work.hero.as_ref().map(|path| {
        let look = shots.ask(path, Size::Thumb, cx);
        super::visuals::picture_on(&shots, render::hash_of(("design-card", ticket)), path, look, (CARD_W - 2., HERO_H), R_LG - 1., true, 0., Ground::Surface)
    });
    let counts: Vec<String> = [
        (badges.directions > 0).then(|| t!("design.results_directions", count = badges.directions).to_string()),
        (badges.links > 0).then(|| t!("design.results_links", count = badges.links).to_string()),
        badges.notes.then(|| t!("design.results_notes_badge").to_string()),
    ]
    .into_iter()
    .flatten()
    .collect();
    v_flex()
        .id(SharedString::from(format!("design-card-{ticket}")))
        .w(px(CARD_W))
        .flex_none()
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(if fresh { p.accent } else { p.hair })
        .overflow_hidden()
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .child(
            div()
                .relative()
                .w_full()
                .h(px(HERO_H))
                .flex_none()
                .bg(p.bg)
                .border_b_1()
                .border_color(p.hair)
                .flex()
                .items_center()
                .justify_center()
                .children(hero)
                .when(work.hero.is_none(), |d| d.child(icon(Icon::Eye, 22., p.muted)))
                .when(fresh, |d| d.child(div().absolute().top(px(8.)).left(px(8.)).child(chip(p, t!("design.results_new").to_string(), true)))),
        )
        .child(
            v_flex()
                .p_3()
                .gap(px(6.))
                .child(div().text_size(px(BODY)).line_height(px(BODY_LINE)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).line_clamp(2).child(work.title.clone()))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(state_pill(p, work.state))
                        .child(div().flex_none().font_family(MONO).text_size(px(META)).text_color(p.muted).child(format!("T-{ticket}")))
                        .children(when_of(shell, work).map(|w| div().flex_1().min_w_0().truncate().text_size(px(META)).text_color(p.muted).child(w))),
                )
                .when(!counts.is_empty(), |c| c.child(div().text_size(px(META)).text_color(p.text2).child(counts.join(" · ")))),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.designs_open(ticket, cx)))
        .into_any_element()
}

// ── one card, opened ──

fn detail(shell: &Shell, work: &DesignWork, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ticket = work.ticket;
    let when = when_of(shell, work);
    let head = v_flex()
        .gap_2()
        .child(h_flex().child(ui::button(p, "design-results-back", Kind::Ghost, t!("design.results_back")).icon(Icon::ArrowLeft).tip(t!("design.results_back_tip"), Some("Esc")).on_click(cx.listener(|this, _, _, cx| this.designs_back(cx)))))
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .child(div().min_w_0().text_size(px(20.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(work.title.clone()))
                .child(state_pill(p, work.state))
                .child(div().flex_1())
                .child(
                    ui::button(p, "design-results-ticket", Kind::Secondary, t!("design.results_open_ticket"))
                        .icon(Icon::Ticket)
                        .tip(t!("design.results_open_ticket_tip", id = ticket), None)
                        .on_click(cx.listener(move |this, _, window, cx| this.open_ticket_row(ticket, window, cx))),
                ),
        )
        .child(div().text_size(px(META)).text_color(p.muted).child([Some(format!("T-{ticket}")), work.project.clone(), when].into_iter().flatten().collect::<Vec<_>>().join(" · ")));
    v_flex()
        .w_full()
        .gap_5()
        .child(head)
        .children(summary(shell, work))
        .children(compare(shell, work, cx))
        .children(strip(shell, work, cx))
        .children(changes(shell, work))
        .children(previews(shell, work, cx))
        .children(decision(shell, work))
        .when(work.directions.is_empty() && work.before.is_empty() && work.pictures.is_empty(), |c| c.child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(t!("design.results_no_pictures"))))
        .into_any_element()
}

/// A section's heading.
fn section(p: Palette, title: impl Into<SharedString>) -> Div {
    v_flex().w_full().gap_2().child(div().text_size(px(META + 1.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(title.into()))
}

fn summary(shell: &Shell, work: &DesignWork) -> Option<AnyElement> {
    if work.summary.trim().is_empty() {
        return None;
    }
    let p = shell.palette;
    Some(section(p, t!("design.results_summary")).child(div().max_w(px(760.)).text_size(px(BODY)).line_height(px(BODY_LINE)).text_color(p.text2).child(work.summary.clone())).into_any_element())
}

/// The screen as it was beside each direction at one scale.
fn compare(shell: &Shell, work: &DesignWork, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let ui_ = &shell.design.results;
    let viewport = ui_.viewport_of(work);
    let columns = designs::columns(work, viewport);
    if columns.is_empty() {
        return None;
    }
    let ticket = work.ticket;
    let handle = cx.entity().downgrade();
    let shots = Shots::of(shell, handle.clone(), cx);
    let key = render::hash_of(("design-compare-width", ticket, matches!(viewport, Viewport::Mobile)));
    let guess = (shell.design.pane.min(MAX_W) - 2. * PAD).max(320.);
    let width = shots.width(key, guess);
    let dims: Vec<Option<(u32, u32)>> = columns.iter().map(|c| shots.best_dims(&c.shot.path)).collect();
    let boxes = visual::side_boxes(width, &dims);
    let items: Vec<Item> = columns
        .iter()
        .map(|c| Item { path: c.shot.path.clone(), choice: None, label: c.direction.as_ref().map_or_else(|| t!("design.results_before").to_string(), |k| direction_label(k)), tile: None, note: String::new(), before: c.direction.is_none() })
        .collect();
    let cells = columns.iter().zip(boxes).enumerate().map(|(n, (column, (w, h)))| {
        let k = render::hash_of(("design-compare", ticket, column.shot.path.as_str()));
        let look = shots.ask(&column.shot.path, Size::Inline, cx);
        let picture = super::visuals::picture_on(&shots, k, &column.shot.path, look, (w, h), R_MD, false, 0., Ground::Surface);
        let viewer = Viewer { owner: Owner::Ticket(ticket), items: items.clone(), at: n };
        let open_big = {
            let handle = handle.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                handle.update(cx, |shell, cx| shell.open_viewer(viewer.clone(), cx)).ok();
            }
        };
        let title = h_flex()
            .w(px(w))
            .min_h(px(24.))
            .gap_2()
            .items_center()
            .map(|t| match &column.direction {
                None => t.child(chip(p, t!("design.results_before").to_string(), false)),
                Some(key) => t.child(div().min_w_0().truncate().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(direction_label(key))),
            })
            .when(column.chosen, |t| t.child(chip(p, t!("design.results_chosen").to_string(), true)));
        let preview = column.direction.as_ref().and_then(|key| work.links.iter().find(|l| &l.direction == key)).and_then(|l| designs::live_link(&l.url)).map(|(url, host)| {
            ui::button(p, SharedString::from(format!("design-compare-preview-{ticket}-{n}")), Kind::Ghost, t!("design.results_open_preview")).icon(Icon::Globe).tip(t!("design.results_open_preview_tip", host = host), None).on_click(move |_, _, cx| cx.open_url(&url))
        });
        v_flex()
            .flex_none()
            .w(px(w))
            .gap_2()
            .child(title)
            .child(div().id(SharedString::from(format!("design-compare-shot-{ticket}-{n}"))).flex_none().rounded(px(R_MD + 1.)).border_1().border_color(if column.chosen { p.accent } else { p.hair }).overflow_hidden().cursor_pointer().child(picture).on_click(open_big).tip(p, t!("visuals.open"), None))
            .children(preview)
            .into_any_element()
    });
    let row = h_flex().w_full().gap(px(visual::SIDE_GAP)).items_start().children(cells.collect::<Vec<_>>());
    let widths = designs::viewports(work);
    let toggle = (widths.len() > 1).then(|| {
        let mut seg = h_flex().flex_none().h(px(26.)).p(px(2.)).gap(px(2.)).rounded(px(R_MD)).bg(p.wash);
        for v in widths {
            let on = v == viewport;
            let label = match v {
                Viewport::Desktop => t!("design.results_desktop"),
                Viewport::Mobile => t!("design.results_mobile"),
            };
            seg = seg.child(
                div()
                    .id(SharedString::from(format!("design-results-width-{}", matches!(v, Viewport::Mobile))))
                    .h_full()
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .text_size(px(SMALL))
                    .cursor_pointer()
                    .when(on, |d| d.bg(p.raised).text_color(p.text).shadow_sm())
                    .when(!on, |d| d.text_color(p.text2).hover(move |s| s.bg(p.wash)))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.designs_pick_viewport(v, cx))),
            );
        }
        seg
    });
    Some(
        section(p, t!("design.results_compare"))
            .child(h_flex().gap_3().items_center().child(div().text_size(px(SMALL)).text_color(p.text2).child(t!("design.results_compare_hint"))).children(toggle))
            .child(shots.measured(div().w_full().child(row), key))
            .into_any_element(),
    )
}

/// "Direction A", "Calm ledger": a direction as the person reads it.
fn direction_label(key: &str) -> String {
    if key.is_empty() {
        return t!("design.results_design").to_string();
    }
    let name = designs::name_of(key);
    if name.chars().count() == 1 { t!("design.results_direction", name = name).to_string() } else { name }
}

/// The pictures the hand-in showed besides the renders.
fn strip(shell: &Shell, work: &DesignWork, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if work.pictures.is_empty() {
        return None;
    }
    let p = shell.palette;
    let ticket = work.ticket;
    let handle = cx.entity().downgrade();
    let shots = Shots::of(shell, handle.clone(), cx);
    let items: Vec<Item> = work.pictures.iter().map(|s| Item { path: s.path.clone(), choice: None, label: s.name.clone(), tile: None, note: String::new(), before: false }).collect();
    let tiles: Vec<AnyElement> = work
        .pictures
        .iter()
        .enumerate()
        .map(|(n, shot)| {
            let look = shots.ask(&shot.path, Size::Thumb, cx);
            let picture = super::visuals::picture_on(&shots, render::hash_of(("design-strip", ticket, n)), &shot.path, look, (STRIP_W, STRIP_H), R_MD, false, 0., Ground::Surface);
            let viewer = Viewer { owner: Owner::Ticket(ticket), items: items.clone(), at: n };
            let handle = handle.clone();
            div()
                .id(SharedString::from(format!("design-strip-{ticket}-{n}")))
                .flex_none()
                .rounded(px(R_MD + 1.))
                .border_1()
                .border_color(p.hair)
                .overflow_hidden()
                .cursor_pointer()
                .child(picture)
                .on_click(move |_, _, cx| {
                    handle.update(cx, |shell, cx| shell.open_viewer(viewer.clone(), cx)).ok();
                })
                .tip(p, shot.name.clone(), None)
                .into_any_element()
        })
        .collect();
    Some(section(p, t!("design.results_pictures")).child(h_flex().w_full().flex_wrap().gap_2().children(tiles)).into_any_element())
}

/// The change notes, per direction, as `CHANGES.md` wrote them.
fn changes(shell: &Shell, work: &DesignWork) -> Option<AnyElement> {
    let keys = designs::note_keys(work);
    if keys.is_empty() {
        return None;
    }
    let p = shell.palette;
    let blocks: Vec<AnyElement> = keys
        .iter()
        .map(|key| {
            let heading = if key.is_empty() { t!("design.results_changes_general").to_string() } else { direction_label(key) };
            let chosen = designs::direction_in(work, key).is_some_and(|d| d.chosen);
            let body: Vec<AnyElement> = designs::notes_for(work, key).into_iter().flat_map(|c| designs::lines(&c.text)).map(|line| line_of(p, line)).collect();
            v_flex()
                .flex_1()
                .min_w(px(300.))
                .gap(px(6.))
                .p_3()
                .rounded(px(R_LG))
                .bg(p.raised)
                .border_1()
                .border_color(if chosen { p.accent } else { p.hair })
                .child(h_flex().gap_2().items_center().child(div().text_size(px(SMALL + 0.5)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(heading)).when(chosen, |h| h.child(chip(p, t!("design.results_chosen").to_string(), true))))
                .children(body)
                .into_any_element()
        })
        .collect();
    Some(section(p, t!("design.results_changes")).child(h_flex().w_full().flex_wrap().gap_3().items_start().children(blocks)).into_any_element())
}

fn line_of(p: Palette, line: Line) -> AnyElement {
    match line {
        Line::Heading(text) => div().pt(px(4.)).text_size(px(META + 0.5)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(text).into_any_element(),
        Line::Bullet(text) => h_flex()
            .gap_2()
            .items_start()
            .child(div().mt(px(8.)).size(px(4.)).flex_none().rounded_full().bg(p.muted))
            .child(div().flex_1().min_w_0().text_size(px(SMALL)).line_height(px(19.)).text_color(p.text2).child(text))
            .into_any_element(),
        Line::Text(text) => div().text_size(px(SMALL)).line_height(px(19.)).text_color(p.text2).child(text).into_any_element(),
    }
}

/// A button for each live preview the ticket's words gave.
fn previews(shell: &Shell, work: &DesignWork, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let links: Vec<(String, String, String)> = work.links.iter().filter_map(|l| designs::live_link(&l.url).map(|(url, host)| (url, host, l.direction.clone()))).collect();
    if links.is_empty() {
        return None;
    }
    let p = shell.palette;
    let ticket = work.ticket;
    let buttons: Vec<AnyElement> = links
        .into_iter()
        .enumerate()
        .map(|(n, (url, host, direction))| {
            let label = if direction.is_empty() { t!("design.results_open_preview").to_string() } else { t!("design.results_open_preview_of", name = direction_label(&direction)).to_string() };
            let tip = t!("design.results_open_preview_tip", host = host);
            let go = {
                let url = url.clone();
                cx.listener(move |this, _: &ClickEvent, window, cx| this.designs_open_link(&url, window, cx))
            };
            ui::button(p, SharedString::from(format!("design-preview-{ticket}-{n}")), Kind::Secondary, label).icon(Icon::Globe).tip(tip, None).on_click(go).into_any_element()
        })
        .collect();
    Some(section(p, t!("design.results_previews")).child(h_flex().w_full().flex_wrap().gap_2().children(buttons)).into_any_element())
}

/// The decision record that names what was chosen.
fn decision(shell: &Shell, work: &DesignWork) -> Option<AnyElement> {
    let d = work.decision.as_ref()?;
    let p = shell.palette;
    // A record is a line to a field (`Why: …`): each line stays its own.
    let body: Vec<AnyElement> = d.text.lines().flat_map(designs::lines).filter(|l| !matches!(l, Line::Heading(h) if *h == d.title)).take(40).map(|l| line_of(p, l)).collect();
    Some(
        section(p, t!("design.results_decision"))
            .child(
                v_flex()
                    .gap(px(6.))
                    .p_3()
                    .rounded(px(R_LG))
                    .bg(p.raised)
                    .border_1()
                    .border_color(p.hair)
                    .child(h_flex().gap_2().items_center().child(div().text_size(px(SMALL + 0.5)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(d.title.clone())).child(div().font_family(MONO).text_size(px(META)).text_color(p.muted).child(d.file.clone())))
                    .when(!d.chose.is_empty(), |c| c.child(div().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(t!("design.results_decision_chose", what = d.chose.clone()))))
                    .children(body),
            )
            .into_any_element(),
    )
}

// ── nothing yet ──

fn empty(shell: &Shell) -> AnyElement {
    let p = shell.palette;
    let point = |text: String| h_flex().gap_2().items_start().child(div().mt(px(9.)).size(px(4.)).flex_none().rounded_full().bg(p.muted)).child(div().flex_1().child(text));
    v_flex()
        .id("design-results-empty")
        .w_full()
        .pt(px(56.))
        .items_center()
        .child(motion::rise(
            v_flex()
                .max_w(px(520.))
                .gap_4()
                .child(div().size(px(44.)).rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).flex().items_center().justify_center().child(icon(Icon::Eye, 22., p.text2)))
                .child(div().text_size(px(20.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("design.results_empty_title")))
                .child(div().text_size(px(BODY)).line_height(px(BODY_LINE)).text_color(p.text2).child(t!("design.results_empty_text")))
                .child(
                    v_flex()
                        .gap_1()
                        .text_size(px(SMALL))
                        .line_height(px(22.))
                        .text_color(p.text2)
                        .child(point(t!("design.results_empty_names").into()))
                        .child(point(t!("design.results_empty_notes").into()))
                        .child(point(t!("design.results_empty_links").into())),
                ),
            "design-results-empty-in",
        ))
        .into_any_element()
}

// ── what the person does ──

impl Shell {
    /// Ask the daemon what the design tickets made when the answer is missing
    /// or stale (the Designs tab, the Needs-you item and the Status line read it).
    pub fn designs_watch(&mut self) {
        if !matches!(self.model.conn, crate::state::Conn::Connected { .. }) || self.model.squad.is_empty() {
            return;
        }
        if let Some(ask) = self.design.results.wanted(&self.model.squad, std::time::Instant::now()) {
            self.send_command(ask);
        }
    }

    /// What the daemon said the design tickets made: the tab's cards, and which
    /// of them are new for the person.
    pub fn designs_arrived(&mut self, squad: String, designs: Vec<DesignWork>) -> bool {
        if squad != self.model.squad {
            return false;
        }
        self.model.design_seen = self.remembered.designs_seen_in(&squad);
        self.model.design_news = designs::news(&designs, &self.model.design_seen, self.model.unix);
        let changed = self.design.results.arrived(designs, std::time::Instant::now());
        if changed {
            self.model.revision += 1;
        }
        changed
    }

    /// The person looked at ticket `id`'s design: it is not new any more, here or after a restart.
    pub fn designs_see(&mut self, id: TicketId) {
        if self.model.design_seen.insert(id) {
            self.remembered.see_design(&self.model.squad, id);
            self.remembered.save(&equipe_providers::credentials::equipe_home());
            self.model.revision += 1;
        }
    }

    /// Show the card of ticket `id` on the design screen's Designs tab.
    pub fn designs_open(&mut self, id: TicketId, cx: &mut Context<Self>) {
        self.design.results.open = Some(id);
        self.design.results.viewport = None;
        self.design.results.scroll.set_offset(point(px(0.), px(0.)));
        self.designs_see(id);
        cx.notify();
    }

    /// The link of a card, or the Needs-you item, in front of the card: the
    /// design screen on its Designs tab, the card open.
    pub fn designs_show(&mut self, id: TicketId, cx: &mut Context<Self>) {
        self.open_design(cx);
        self.design_set_tab(crate::flows::Tab::Designs, cx);
        self.designs_open(id, cx);
    }

    /// Back to the list.
    pub fn designs_back(&mut self, cx: &mut Context<Self>) {
        self.design.results.open = None;
        self.viewer = None;
        self.design.results.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    pub fn designs_pick_viewport(&mut self, viewport: Viewport, cx: &mut Context<Self>) {
        self.design.results.viewport = Some(viewport);
        cx.notify();
    }

    /// Read again: a designer may have handed in more.
    pub fn designs_refresh(&mut self, cx: &mut Context<Self>) {
        self.design.results.dirty = true;
        self.designs_watch();
        cx.notify();
    }

    /// A preview opened where the person is — the browser they chose, on this Mac, not the daemon's.
    pub fn designs_open_link(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if designs::live_link(url).is_some() {
            self.open_link(url, false, window, cx);
        }
    }

    /// Each draw, while the tab is on show: the cards asked for when they are not here yet.
    pub fn designs_prepare(&mut self) {
        self.designs_watch();
    }
}
