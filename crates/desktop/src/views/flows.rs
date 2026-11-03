//! 화면 지도: the service's every flow, its screens in order and each screen
//! in every state, as the designer keeps them (`crate::flows`) — the design
//! screen's first tab. A line of counts (화면 · 상태 · 빠진 상태 · 오래됨) over
//! the flows on the left and the one picked: its walkthrough to watch, its
//! screens left to right as cards joined by arrows, and the one clicked
//! open below them — big, in each state (the missing ones dashed), light or
//! dark, at each width, its notes, and a box to talk about it with the
//! designer or the PM, the picture going with the words. ←/→ move between
//! screens, ↑/↓ between flows, Esc closes. With no map yet, what one is and
//! a way to ask the designer.

use std::path::Path;
use std::sync::Arc;

use equipe_core::protocol::Command;
use equipe_core::protocol::design::{Flow, FlowMap, Screen, Walkthrough};
use equipe_core::t;
use gpui_kit::component::input::Input;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::flows::{self as fm, Chip, Counts, Tab};
use crate::icons::{Icon, icon};
use crate::state::Presence;
use crate::theme::{BODY, BODY_LINE, FIELD, ICON_SM, META, MONO, Palette, R_LG, R_MD, R_SM, SMALL, UI};
use crate::tips::TipExt as _;
use crate::visuals::Size;
use crate::{Shell, motion, ui};

/// The side margins, and the flows' column.
const PAD: f32 = 24.;
const LIST_W: f32 = 208.;
/// A screen's card in the strip, and its picture's box.
const CARD_W: f32 = 168.;
const THUMB_H: f32 = 212.;
/// The open screen's picture.
const STAGE_H: f32 = 440.;
/// A walkthrough's poster.
const POSTER_W: f32 = 104.;
const POSTER_H: f32 = 64.;

// ── the tabs ──

/// 화면 지도 | 디자인 시스템, where the screen's title was.
pub fn tabs(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let on = shell.design_tab();
    let mut row = h_flex().flex_none().gap_4().items_center();
    for tab in Tab::ALL {
        let active = tab == on;
        row = row.child(
            div()
                .id(SharedString::from(format!("design-tab-top-{}", tab.code())))
                .flex_none()
                .cursor_pointer()
                .text_size(px(15.))
                .text_color(if active { p.text } else { p.muted })
                .when(!active, |d| d.hover(move |s| s.text_color(p.text2)))
                .child(ui::steady(tab.label(), FontWeight::SEMIBOLD, active, false))
                .on_click(cx.listener(move |this, _, _, cx| this.design_set_tab(tab, cx))),
        );
    }
    row
}

// ── the page ──

pub fn page(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let map = shell.design.current().and_then(|s| s.flows.as_ref());
    match map {
        Some(map) if !map.flows.is_empty() => {
            v_flex()
                .id("flows")
                .flex_1()
                .min_h_0()
                .child(counts_bar(shell, map))
                .child(h_flex().flex_1().min_h_0().items_start().child(flow_list(shell, map, cx)).child(main(shell, map, cx)))
                .into_any_element()
        }
        _ => empty(shell, map, cx),
    }
}

/// 화면 12 · 상태 31 · 빠진 상태 4 · 오래됨 1.
fn counts_bar(shell: &Shell, map: &FlowMap) -> impl IntoElement + use<> {
    let p = shell.palette;
    let Counts { screens, states, missing, stale } = fm::counts(map);
    let part = |label: String, n: usize, tone: Rgba| h_flex().gap_1().items_baseline().child(div().text_color(p.text2).child(label)).child(div().font_weight(FontWeight::SEMIBOLD).text_color(tone).child(n.to_string()));
    let dot = || div().text_color(p.muted).child("·");
    h_flex()
        .id("flows-counts")
        .flex_none()
        .h(px(40.))
        .px(px(PAD))
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .text_size(px(SMALL))
        .overflow_x_scroll()
        .child(part(t!("flows.count_screens").into(), screens, p.text))
        .child(dot())
        .child(part(t!("flows.count_states").into(), states, p.text))
        .child(dot())
        .child(part(t!("flows.count_missing").into(), missing, if missing > 0 { p.danger() } else { p.text }))
        .child(dot())
        .child(part(t!("flows.count_stale").into(), stale, p.text))
}

/// The flows, a row each: its title, and what it lacks or how many screens.
fn flow_list(shell: &Shell, map: &FlowMap, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let on = shell.design.map.flow(map).map(|f| f.id.clone());
    let mut col = v_flex()
        .id("flows-list")
        .w(px(LIST_W))
        .flex_none()
        .h_full()
        .overflow_y_scroll()
        .px_2()
        .pt_3()
        .pb_8()
        .gap(px(1.))
        .border_r_1()
        .border_color(p.hair)
        .child(div().px_2().pb_1p5().text_size(px(META)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child(t!("flows.flows")));
    for flow in &map.flows {
        let active = on.as_deref() == Some(flow.id.as_str());
        let c = fm::flow_counts(flow);
        let id = flow.id.clone();
        let side: AnyElement = if c.missing > 0 {
            div().flex_none().text_size(px(META)).text_color(p.danger()).child(t!("flows.missing_n", count = c.missing)).into_any_element()
        } else {
            div().flex_none().text_size(px(META)).text_color(p.muted).child(c.screens.to_string()).into_any_element()
        };
        col = col.child(
            h_flex()
                .id(SharedString::from(format!("flows-flow-{}", flow.id)))
                .h(px(crate::theme::ROW))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(R_MD))
                .cursor_pointer()
                .when(active, |d| d.bg(p.wash))
                .when(!active, |d| d.hover(move |s| s.bg(p.wash)))
                .text_size(px(SMALL))
                .child(div().flex_1().min_w_0().truncate().text_color(if active { p.text } else { p.text2 }).when(active, |d| d.font_weight(FontWeight::MEDIUM)).child(flow.title.clone()))
                .child(side)
                .on_click(cx.listener(move |this, _, _, cx| this.flows_pick_flow(&id, cx))),
        );
    }
    col
}

/// The flow picked: its title and walkthrough, its screens, the one open.
fn main(shell: &Shell, map: &FlowMap, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let ui_ = &shell.design.map;
    let flow = ui_.flow(map).expect("a map with flows");
    let open = ui_.screen(map).map(|s| s.id.clone());
    let mut col = v_flex().px(px(PAD)).pt(px(20.)).pb(px(64.)).gap_4();
    if !map.problems.is_empty() {
        col = col.child(super::design::problems(shell, &map.problems, true, cx));
    }
    col = col.child(flow_head(shell, flow, cx));
    // The screens, left to right, an arrow between each two.
    let mut strip = h_flex().id("flows-strip").w_full().pb_2().gap_2().items_start().overflow_x_scroll().track_scroll(&ui_.strip);
    for (n, screen) in flow.screens.iter().enumerate() {
        if n > 0 {
            strip = strip.child(div().flex_none().h(px(THUMB_H)).flex().items_center().child(icon(Icon::ArrowRight, ICON_SM, p.muted)));
        }
        strip = strip.child(card(shell, n, screen, open.as_deref() == Some(screen.id.as_str()), cx));
    }
    col = col.child(strip);
    match ui_.screen(map) {
        Some(screen) => {
            let n = flow.screens.iter().position(|s| s.id == screen.id).unwrap_or(0);
            col = col.child(detail(shell, flow, n, screen, cx));
        }
        None => col = col.child(div().text_size(px(SMALL)).text_color(p.muted).child(t!("flows.pick_hint"))),
    }
    div().id("flows-main").flex_1().min_w_0().h_full().overflow_y_scroll().child(col)
}

/// The flow's title and summary, and its walkthrough.
fn flow_head(shell: &Shell, flow: &Flow, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let title = v_flex()
        .flex_1()
        .min_w(px(ui::WORDS_AT_LEAST))
        .gap_0p5()
        .child(div().text_size(px(18.)).line_height(px(26.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(flow.title.clone()))
        .children(flow.summary.clone().map(|s| div().text_size(px(SMALL)).text_color(p.text2).child(s)));
    let id = flow.id.clone();
    let watch: AnyElement = match fm::video(flow) {
        Some(video) => walkthrough(shell, flow, video, cx),
        None => ui::button(p, "flows-ask-video", ui::Kind::Secondary, t!("flows.video_ask"))
            .icon(Icon::Play)
            .on_click(cx.listener(move |this, _, window, cx| this.flows_ask_video(&id, window, cx)))
            .into_any_element(),
    };
    h_flex().flex_wrap().gap_3().items_start().child(title).child(div().flex_none().max_w_full().child(watch))
}

/// 영상으로 보기: the walkthrough to watch — its first frame, its length and
/// age, the screens it passes and when — and the older ones.
fn walkthrough(shell: &Shell, flow: &Flow, video: &Walkthrough, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let now = now_secs();
    let older = fm::older_videos(flow);
    let open = shell.design.map.older_open;
    let poster: AnyElement = match shell.design.map.posters.get(&video.path).cloned().flatten() {
        Some(image) => img(image).size_full().object_fit(ObjectFit::Cover).into_any_element(),
        None => div().into_any_element(),
    };
    let path = video.path.clone();
    let mut meta = vec![fm::length(video), crate::activity::ago(now.saturating_sub(video.taken))];
    meta.push(mode_word(&video.mode));
    meta.push(format!("{}px", video.width));
    let marks = fm::marks(flow, video);
    let card = h_flex()
        .id("flows-video")
        .gap_3()
        .p_2()
        .pr_3()
        .items_center()
        .rounded(px(R_LG))
        .border_1()
        .border_color(p.hair)
        .bg(p.raised)
        .cursor_pointer()
        .hover(move |s| s.border_color(p.muted))
        .child(
            div()
                .w(px(POSTER_W))
                .h(px(POSTER_H))
                .flex_none()
                .relative()
                .rounded(px(R_MD))
                .overflow_hidden()
                .bg(p.wash)
                .child(poster)
                .child(div().absolute().inset_0().flex().items_center().justify_center().child(div().size(px(26.)).rounded_full().bg(rgba(0x0000008C)).flex().items_center().justify_center().child(icon(Icon::Play, 13., rgb(0xFFFFFF))))),
        )
        .child(
            v_flex()
                .min_w_0()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(t!("flows.video_watch")))
                        .when(!video.current, |r| r.child(div().text_size(px(META)).text_color(p.muted).child(t!("flows.video_old")))),
                )
                .child(div().text_size(px(META)).text_color(p.muted).child(meta.join(" · ")))
                .when(!marks.is_empty(), |c| {
                    c.child(h_flex().flex_wrap().gap_x_2().gap_y_0p5().max_w(px(420.)).text_size(px(11.5)).children(marks.into_iter().map(|(title, at)| {
                        h_flex().gap_1().child(div().text_color(p.text2).child(title)).child(div().font_family(MONO).text_color(p.muted).child(at))
                    })))
                }),
        )
        .tip(p, t!("flows.video_play_tip"), None)
        .on_click(cx.listener(move |this, _, _, cx| this.flows_play(&path, cx)));
    let mut col = v_flex().gap_1().items_end().child(card);
    if !older.is_empty() {
        col = col.child(
            ui::button(p, "flows-older", ui::Kind::Ghost, t!("flows.video_older", count = older.len()))
                .icon(if open { Icon::ChevronUp } else { Icon::ChevronDown })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.design.map.older_open = !this.design.map.older_open;
                    cx.notify();
                })),
        );
        if open {
            let mut list = v_flex().gap(px(1.)).min_w(px(220.));
            for (n, v) in older.iter().enumerate() {
                let path = v.path.clone();
                list = list.child(
                    h_flex()
                        .id(("flows-older-row", n))
                        .h(px(crate::theme::ROW))
                        .px_2()
                        .gap_3()
                        .items_center()
                        .rounded(px(R_MD))
                        .cursor_pointer()
                        .hover(move |s| s.bg(p.wash))
                        .text_size(px(META))
                        .child(icon(Icon::Play, 12., p.muted))
                        .child(div().flex_1().text_color(p.text2).child(crate::activity::ago(now.saturating_sub(v.taken))))
                        .child(div().font_family(MONO).text_color(p.muted).child(fm::length(v)))
                        .on_click(cx.listener(move |this, _, _, cx| this.flows_play(&path, cx))),
                );
            }
            col = col.child(list);
        }
    }
    col.into_any_element()
}

fn mode_word(mode: &str) -> String {
    match mode {
        "light" => t!("design.mode_light").to_owned(),
        "dark" => t!("design.mode_dark").to_owned(),
        other => other.to_owned(),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A screen in the strip: its default state's picture, its number and name,
/// how many states and what it lacks.
fn card(shell: &Shell, n: usize, screen: &Screen, open: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui_ = &shell.design.map;
    let mode = ui_.mode(p.is_dark());
    let shot = fm::default_state(screen).and_then(|s| fm::pick(s, &mode, ui_.width));
    let missing = fm::chips(screen).iter().filter(|c| c.missing).count();
    let face: AnyElement = match shot {
        Some(pic) => super::design::picture(shell, &pic.path, p.muted),
        None => div().text_size(px(META)).text_color(p.muted).child(t!("flows.no_picture")).into_any_element(),
    };
    let id = screen.id.clone();
    v_flex()
        .id(SharedString::from(format!("flows-card-{}", screen.id)))
        .w(px(CARD_W))
        .flex_none()
        .rounded(px(R_LG))
        .overflow_hidden()
        .border_1()
        .border_color(if open { p.accent } else { p.hair })
        .bg(p.raised)
        .cursor_pointer()
        .when(!open, |d| d.hover(move |s| s.border_color(p.muted)))
        .child(div().w_full().h(px(THUMB_H)).p_2().bg(p.wash).flex().items_center().justify_center().child(face))
        .child(
            v_flex()
                .px_2p5()
                .py_2()
                .gap_0p5()
                .border_t_1()
                .border_color(p.hair)
                .child(div().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).truncate().child(format!("{}. {}", n + 1, screen.title)))
                .child(
                    h_flex()
                        .gap_1()
                        .text_size(px(META))
                        .text_color(p.muted)
                        .child(t!("flows.states_n", count = screen.states.len()))
                        .when(missing > 0, |r| r.child("·").child(div().text_color(p.danger()).child(t!("flows.missing_short"))))
                        .when(screen.stale, |r| r.child("·").child(t!("flows.stale"))),
                ),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.flows_toggle_screen(&id, cx)))
        .into_any_element()
}

/// One of a few, as the header's sources are: a wash with the one picked raised.
fn segment(p: Palette, id: &str, options: Vec<(String, bool, Box<dyn Fn(&mut Shell, &mut Context<Shell>)>)>, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let mut seg = h_flex().flex_none().h(px(26.)).p(px(2.)).gap(px(2.)).rounded(px(R_MD)).bg(p.wash);
    for (n, (label, on, pick)) in options.into_iter().enumerate() {
        seg = seg.child(
            div()
                .id(SharedString::from(format!("{id}-{n}")))
                .h_full()
                .px_2()
                .flex()
                .items_center()
                .rounded(px(R_SM))
                .text_size(px(META))
                .cursor_pointer()
                .when(on, |d| d.bg(p.raised).text_color(p.text).shadow_sm())
                .when(!on, |d| d.text_color(p.text2).hover(move |s| s.bg(p.wash)))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| pick(this, cx))),
        );
    }
    seg
}

/// The screen open below the strip.
fn detail(shell: &Shell, flow: &Flow, n: usize, screen: &Screen, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui_ = &shell.design.map;
    let mode = ui_.mode(p.is_dark());
    let modes = fm::modes(screen);
    let widths = fm::widths(screen);
    let shown = ui_.chip(screen);
    let width = ui_.width.or_else(|| widths.first().copied());
    let picture = shown.as_ref().and_then(|c| c.state).and_then(|s| fm::pick(s, &mode, ui_.width));
    // The head: its name, the mode and the width, a way to close it.
    let mut head = h_flex()
        .flex_wrap()
        .gap_2()
        .items_center()
        .child(div().flex_1().min_w(px(160.)).text_size(px(UI)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(format!("{}. {}", n + 1, screen.title)));
    if modes.len() > 1 {
        let options = modes
            .iter()
            .map(|m| {
                let m2 = m.clone();
                let pick: Box<dyn Fn(&mut Shell, &mut Context<Shell>)> = Box::new(move |this, cx| this.flows_mode(&m2, cx));
                (mode_word(m), picture.is_some_and(|pic| &pic.mode == m) || (picture.is_none() && &mode == m), pick)
            })
            .collect();
        head = head.child(segment(p, "flows-mode", options, cx));
    }
    if widths.len() > 1 {
        let options = widths
            .iter()
            .map(|w| {
                let w = *w;
                let pick: Box<dyn Fn(&mut Shell, &mut Context<Shell>)> = Box::new(move |this, cx| this.flows_width(w, cx));
                (format!("{w}"), picture.map_or(width == Some(w), |pic| pic.width == w), pick)
            })
            .collect();
        head = head.child(segment(p, "flows-width", options, cx));
    }
    head = head.child(ui::icon_button(p, "flows-close", Icon::X, t!("flows.close"), Some("Esc")).on_click(cx.listener(|this, _, _, cx| this.flows_close(cx))));
    // A chip each state; those it lacks, dashed.
    let mut chips = h_flex().flex_wrap().gap_1p5();
    for chip in fm::chips(screen) {
        let on = shown.as_ref().is_some_and(|s| s.key == chip.key);
        let key = chip.key.clone();
        let danger = p.danger();
        let face = h_flex()
            .id(SharedString::from(format!("flows-chip-{}", chip.key)))
            .h(px(24.))
            .px_2p5()
            .gap_1()
            .items_center()
            .rounded(px(12.))
            .border_1()
            .text_size(px(META))
            .cursor_pointer()
            .when(chip.missing, |d| d.border_dashed().border_color(Rgba { a: 0.7, ..danger }).text_color(danger).when(on, |d| d.bg(p.danger_bg())).child("+"))
            .when(!chip.missing, |d| {
                d.border_color(if on { Rgba { a: 0.75, ..p.accent } } else { p.hair }).text_color(if on { p.text } else { p.text2 }).when(on, |d| d.bg(crate::theme::mix(p.raised, p.accent, 0.16)))
            })
            .when(!on, |d| d.hover(move |s| s.bg(p.wash)))
            .child(ui::steady(chip.label.clone(), FontWeight::MEDIUM, on, false))
            .when(chip.missing, |d| d.tip(p, t!("flows.missing_tip"), None))
            .on_click(cx.listener(move |this, _, _, cx| this.flows_chip(&key, cx)));
        chips = chips.child(face);
    }
    let about = shown.as_ref().map_or_else(|| fm::context(flow, screen, ""), |c| fm::context(flow, screen, &c.label));
    v_flex()
        .id("flows-detail")
        .p_4()
        .gap_3()
        .rounded(px(R_LG))
        .border_1()
        .border_color(p.hair)
        .bg(p.raised)
        .child(head)
        .child(chips)
        .child(h_flex().flex_wrap().gap_4().items_start().child(stage(shell, flow, screen, shown.as_ref(), picture.map(|p| p.path.clone()), cx)).child(notes(shell, screen, shown.as_ref(), picture.map(|p| p.taken), cx)))
        .child(talk(shell, about, cx))
        .into_any_element()
}

/// The state's picture, big — a click shows it bigger; for a state it
/// lacks, the dashed place it would be and a way to have it drawn.
fn stage(shell: &Shell, flow: &Flow, screen: &Screen, chip: Option<&Chip>, path: Option<String>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let frame = div().flex_1().min_w(px(280.)).max_w(px(620.)).h(px(STAGE_H)).rounded(px(R_MD)).border_1().overflow_hidden();
    match (path, chip) {
        (Some(path), _) => frame
            .id("flows-stage")
            .p_3()
            .bg(p.wash)
            .border_color(p.hair)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .child(super::design::picture(shell, &path, p.muted))
            .tip(p, t!("flows.zoom"), None)
            .on_click(cx.listener(|this, _, _, cx| this.flows_zoom(true, cx)))
            .into_any_element(),
        (None, chip) => {
            let label = chip.map(|c| c.label.clone()).unwrap_or_default();
            let about = fm::context(flow, screen, &label);
            frame
                .id("flows-stage")
                .border_dashed()
                .border_color(Rgba { a: 0.6, ..p.danger() })
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .child(div().max_w(px(320.)).text_center().text_size(px(SMALL)).text_color(p.text2).child(t!("flows.missing_stage", state = label)))
                .child(ui::button(p, "flows-ask-draw", ui::Kind::Secondary, t!("flows.ask_draw")).icon(Icon::PenTool).on_click(cx.listener(move |this, _, window, cx| this.flows_ask_draw(&about, window, cx))))
                .into_any_element()
        }
    }
}

/// What the state and the screen are for, and where they come from.
fn notes(shell: &Shell, screen: &Screen, chip: Option<&Chip>, taken: Option<u64>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let block = |title: String, text: String| v_flex().gap_0p5().child(div().text_size(px(META)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child(title)).child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(text));
    let mut col = v_flex().flex_1().min_w(px(220.)).max_w(px(420.)).gap_4();
    if let Some(chip) = chip {
        let kind = chip.state.map(|s| fm::kind_label(&s.kind)).filter(|k| *k != chip.label);
        col = col.child(
            v_flex()
                .gap_0p5()
                .child(h_flex().gap_2().items_baseline().child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(chip.label.clone())).children(kind.map(|k| div().text_size(px(META)).text_color(p.muted).child(k))))
                .children(chip.state.and_then(|s| s.note.clone()).map(|n| div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(n)))
                .when(chip.missing, |c| c.child(div().text_size(px(SMALL)).text_color(p.danger()).child(t!("flows.missing_note")))),
        );
    }
    if let Some(note) = &screen.note {
        col = col.child(block(t!("flows.screen_note").into(), note.clone()));
    }
    // T-n, stale, when taken, where it runs.
    let mut meta = v_flex().gap_1().text_size(px(META)).text_color(p.muted);
    if let Some(ticket) = screen.ticket {
        meta = meta.child(
            h_flex().gap_1p5().child(t!("flows.ticket")).child(
                div()
                    .id("flows-ticket")
                    .font_family(MONO)
                    .text_size(px(12.5))
                    .text_color(p.text2)
                    .cursor_pointer()
                    .hover(move |s| s.text_color(p.accent).underline())
                    .child(format!("T-{ticket}"))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_ticket(ticket, cx))),
            ),
        );
    }
    if screen.stale {
        meta = meta.child(h_flex().gap_1p5().items_center().child(icon(Icon::Clock, 12., p.text2)).child(div().text_color(p.text2).child(t!("flows.stale_note"))));
    }
    if let Some(at) = taken.filter(|t| *t > 0) {
        meta = meta.child(div().child(t!("flows.taken", ago = crate::activity::ago(now_secs().saturating_sub(at)))));
    }
    if let Some(url) = &screen.url {
        meta = meta.child(div().font_family(MONO).truncate().child(url.clone()));
    }
    col.child(meta).into_any_element()
}

/// Who to talk to: the designer when the squad has one, else the PM.
pub fn recipients(shell: &Shell) -> (Option<&crate::state::Member>, Option<&crate::state::Member>) {
    let designer = shell.model.members.iter().find(|m| m.role == "designer" && m.presence != Presence::Closed);
    let pm = shell.model.members.iter().find(|m| crate::state::Model::is_pm(m));
    (designer, pm)
}

/// The box under the open screen: where it is, who to, the words.
fn talk(shell: &Shell, about: String, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let (designer, pm) = recipients(shell);
    let to_pm = shell.design.map.to_pm || designer.is_none();
    let typed = !shell.flow_talk.read(cx).value().trim().is_empty();
    let who = |m: &crate::state::Member, label: String, on: bool, pm: bool| {
        div()
            .id(SharedString::from(format!("flows-to-{}", if pm { "pm" } else { "designer" })))
            .h_full()
            .px_2()
            .flex()
            .gap_1p5()
            .items_center()
            .rounded(px(R_SM))
            .text_size(px(META))
            .cursor_pointer()
            .when(on, |d| d.bg(p.raised).text_color(p.text).shadow_sm())
            .when(!on, |d| d.text_color(p.text2).hover(move |s| s.bg(p.wash)))
            .child(crate::theme::avatar(p, &m.role, &m.name, 16.))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.design.map.to_pm = pm;
                cx.notify();
            }))
    };
    let mut to = h_flex().flex_none().h(px(26.)).p(px(2.)).gap(px(2.)).rounded(px(R_MD)).bg(p.wash);
    if let Some(d) = designer {
        to = to.child(who(d, t!("flows.to_designer").into(), !to_pm, false));
    }
    if let Some(m) = pm {
        to = to.child(who(m, t!("flows.to_pm").into(), to_pm, true));
    }
    v_flex()
        .pt_3()
        .gap_2()
        .border_t_1()
        .border_color(p.hair)
        .child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .items_center()
                .child(
                    h_flex()
                        .min_w_0()
                        .h(px(22.))
                        .px_2()
                        .gap_1p5()
                        .items_center()
                        .rounded(px(R_SM))
                        .bg(p.wash)
                        .text_size(px(META))
                        .text_color(p.text2)
                        .child(icon(Icon::Message, 12., p.muted))
                        .child(div().min_w_0().truncate().child(about)),
                )
                .child(div().flex_1())
                .when(designer.is_some() || pm.is_some(), |r| r.child(to)),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().flex_1().min_w_0().h(px(FIELD)).px_3().flex().items_center().rounded(px(R_MD)).border_1().border_color(p.hair).bg(p.bg).text_size(px(SMALL)).child(Input::new(&shell.flow_talk).appearance(false).bordered(false)))
                .child(
                    ui::button(p, "flows-send", ui::Kind::Primary, t!("flows.send"))
                        .field()
                        .key("⏎")
                        .disabled(!typed || (designer.is_none() && pm.is_none()))
                        .on_click(cx.listener(|this, _, window, cx| this.flows_send(window, cx))),
                ),
        )
        .into_any_element()
}

/// No map yet: what it is, and a way to ask for one.
fn empty(shell: &Shell, map: Option<&FlowMap>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let has_designer = recipients(shell).0.is_some();
    let point = |text: String| h_flex().gap_2().items_start().child(div().mt(px(9.)).size(px(4.)).flex_none().rounded_full().bg(p.muted)).child(div().flex_1().child(text));
    let problems = map.map(|m| m.problems.clone()).unwrap_or_default();
    v_flex()
        .id("flows-empty")
        .flex_1()
        .overflow_y_scroll()
        .items_center()
        .justify_center()
        .p(px(PAD))
        .gap_6()
        .when(!problems.is_empty(), |d| d.child(div().w_full().max_w(px(560.)).child(super::design::problems(shell, &problems, true, cx))))
        .child(
            motion::rise(
                v_flex()
                    .max_w(px(520.))
                    .gap_4()
                    .child(div().size(px(44.)).rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).flex().items_center().justify_center().child(icon(Icon::AppWindow, 22., p.text2)))
                    .child(div().text_size(px(20.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("flows.empty_title")))
                    .child(div().text_size(px(BODY)).line_height(px(BODY_LINE)).text_color(p.text2).child(t!("flows.empty_text")))
                    .child(
                        v_flex()
                            .gap_1()
                            .text_size(px(SMALL))
                            .line_height(px(22.))
                            .text_color(p.text2)
                            .child(point(t!("flows.empty_flows").into()))
                            .child(point(t!("flows.empty_states").into()))
                            .child(point(t!("flows.empty_talk").into())),
                    )
                    .child(
                        h_flex().pt_2().gap_2().child(
                            ui::button(p, "flows-ask", ui::Kind::Primary, if has_designer { t!("flows.ask_designer") } else { t!("design.ask_pm") })
                                .icon(Icon::Message)
                                .on_click(cx.listener(|this, _, window, cx| this.flows_ask_map(window, cx))),
                        ),
                    ),
                "flows-empty-in",
            ),
        )
        .into_any_element()
}

/// The open state's picture as the lightbox shows it: its path, its name,
/// what the bar says of it, and whether there are others to step to.
pub fn zoomed(shell: &Shell) -> Option<(String, String, String, bool)> {
    let ui_ = &shell.design.map;
    if !ui_.zoomed || shell.design_tab() != Tab::Map {
        return None;
    }
    let map = shell.design.current()?.flows.as_ref()?;
    let flow = ui_.flow(map)?;
    let screen = ui_.screen(map)?;
    let chip = ui_.chip(screen)?;
    let picture = fm::pick(chip.state?, &ui_.mode(shell.palette.is_dark()), ui_.width)?;
    let name = picture.path.rsplit('/').next().unwrap_or(&picture.path).to_owned();
    let said = format!("{} · {} · {}px", fm::context(flow, screen, &chip.label), mode_word(&picture.mode), picture.width);
    let many = fm::chips(screen).iter().filter(|c| !c.missing).count() > 1;
    Some((picture.path.clone(), name, said, many))
}

// ── what the tab does ──

impl Shell {
    /// The design screen's tab on show.
    pub fn design_tab(&self) -> Tab {
        let picked = self.remembered.design_tab.as_deref().and_then(Tab::parse);
        Tab::shown(picked, self.design.current().is_some_and(|s| s.flows.is_some()))
    }

    /// A tab picked: kept for next time, on this Mac.
    pub fn design_set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.remembered.design_tab.as_deref() != Some(tab.code()) {
            self.remembered.design_tab = Some(tab.code().to_owned());
            self.remembered.save(&equipe_providers::credentials::equipe_home());
        }
        self.design.map.zoomed = false;
        self.design.lightbox = None;
        cx.notify();
    }

    fn flow_map(&self) -> Option<&FlowMap> {
        self.design.current().and_then(|s| s.flows.as_ref())
    }

    pub fn flows_pick_flow(&mut self, id: &str, cx: &mut Context<Self>) {
        self.design.map.pick_flow(id);
        self.design.map.strip.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    pub fn flows_toggle_screen(&mut self, id: &str, cx: &mut Context<Self>) {
        self.design.map.toggle_screen(id);
        cx.notify();
    }

    pub fn flows_chip(&mut self, key: &str, cx: &mut Context<Self>) {
        self.design.map.chip = Some(key.to_owned());
        cx.notify();
    }

    pub fn flows_mode(&mut self, mode: &str, cx: &mut Context<Self>) {
        self.design.map.mode = Some(mode.to_owned());
        cx.notify();
    }

    pub fn flows_width(&mut self, width: u32, cx: &mut Context<Self>) {
        self.design.map.width = Some(width);
        cx.notify();
    }

    pub fn flows_close(&mut self, cx: &mut Context<Self>) {
        self.design.map.screen = None;
        self.design.map.chip = None;
        self.design.map.zoomed = false;
        cx.notify();
    }

    pub fn flows_zoom(&mut self, on: bool, cx: &mut Context<Self>) {
        self.design.map.zoomed = on;
        cx.notify();
    }

    /// The zoomed picture's ←/→: the state before or after.
    pub fn flows_step_state(&mut self, forward: bool, cx: &mut Context<Self>) {
        let screen = self.flow_map().and_then(|m| self.design.map.screen(m)).cloned();
        if let Some(screen) = screen {
            self.design.map.step_state(&screen, forward);
        }
        cx.notify();
    }

    /// The pictures the tab shows: each card's thumbnail, the open state's
    /// picture at the column's size, whole when zoomed; and the walkthrough's
    /// first frame, read on this Mac off the UI thread.
    pub fn flows_load_pictures(&mut self, cx: &mut Context<Self>) {
        if self.design_tab() != Tab::Map {
            return;
        }
        let Some(map) = self.flow_map() else { return };
        let ui_ = &self.design.map;
        let mode = ui_.mode(self.palette.is_dark());
        let mut thumbs: Vec<String> = Vec::new();
        let mut inline: Option<String> = None;
        let mut poster: Option<String> = None;
        if let Some(flow) = ui_.flow(map) {
            for screen in &flow.screens {
                if let Some(pic) = fm::default_state(screen).and_then(|s| fm::pick(s, &mode, ui_.width)) {
                    thumbs.push(pic.path.clone());
                }
            }
            if let Some(screen) = ui_.screen(map)
                && let Some(pic) = ui_.chip(screen).and_then(|c| c.state).and_then(|s| fm::pick(s, &mode, ui_.width))
            {
                inline = Some(pic.path.clone());
            }
            poster = fm::video(flow).map(|v| v.path.clone()).filter(|path| !ui_.posters.contains_key(path));
        }
        let zoomed = zoomed(self).map(|(path, ..)| path);
        let wanted = {
            let mut shots = self.shots.borrow_mut();
            for path in &thumbs {
                let _ = shots.look(path, Size::Thumb);
            }
            if let Some(path) = &inline {
                let _ = shots.look(path, Size::Thumb);
                let _ = shots.look(path, Size::Inline);
            }
            if let Some(path) = &zoomed {
                let _ = shots.look(path, Size::Full);
            }
            shots.has_wanted() && !shots.scheduled
        };
        if wanted {
            self.load_shots(cx);
        }
        if let Some(path) = poster {
            self.design.map.posters.insert(path.clone(), None);
            // Only a file on this Mac: a daemon elsewhere keeps its videos there.
            if self.endpoint.is_remote() || !Path::new(&path).is_file() {
                return;
            }
            let read = cx.background_spawn({
                let path = path.clone();
                async move {
                    let video = crate::media::video(Path::new(&path)).ok()?;
                    let frame = video.frames.into_iter().next()?;
                    let png = crate::media::png(&crate::media::fit(frame.image, 480)).ok()?;
                    Some(Arc::new(Image::from_bytes(ImageFormat::Png, png)))
                }
            });
            cx.spawn(async move |this, cx| {
                let image = read.await;
                let _ = this.update(cx, |this, cx| {
                    this.design.map.posters.insert(path, image);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// A walkthrough opened in the Mac's player; one on another machine plays there.
    pub fn flows_play(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.endpoint.is_remote() || !Path::new(path).is_file() {
            cx.write_to_clipboard(ClipboardItem::new_string(path.to_owned()));
            self.composer.flash = Some(t!("flows.video_elsewhere", path = path));
        } else {
            cx.open_with_system(Path::new(path));
        }
        cx.notify();
    }

    /// The words in the box, about the state on show, to the designer or
    /// the PM — the picture shown with them — and their conversation opened.
    pub fn flows_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let words = self.flow_talk.read(cx).value().to_string();
        let Some(map) = self.flow_map() else { return };
        let ui_ = &self.design.map;
        let (Some(flow), Some(screen)) = (ui_.flow(map), ui_.screen(map)) else { return };
        let Some(chip) = ui_.chip(screen) else { return };
        let picture = chip.state.and_then(|s| fm::pick(s, &ui_.mode(self.palette.is_dark()), ui_.width));
        let about = match picture {
            Some(pic) => t!("flows.talk_about", place = fm::context(flow, screen, &chip.label), look = format!("{} · {}px", mode_word(&pic.mode), pic.width)),
            None => t!("flows.talk_about_missing", place = fm::context(flow, screen, &chip.label)),
        };
        let Some(text) = fm::talk_message(&about, &words, picture.map(|p| p.path.as_str())) else { return };
        let (designer, pm) = recipients(self);
        let to = if ui_.to_pm || designer.is_none() { pm } else { designer };
        let Some(to) = to.map(|m| m.session) else {
            self.composer.flash = Some(t!("design.no_one_to_ask").into());
            cx.notify();
            return;
        };
        self.send_command(Command::Submit { session: to, text, about: Some(self.model.focus.clone()) });
        self.flow_talk.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        self.select_member(to, window, cx);
    }

    /// "+ 빠진 상태"'s way to have it drawn: the request goes to the designer
    /// (or the PM), the way the empty screen's ask does.
    pub fn flows_ask_draw(&mut self, about: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = t!("flows.ask_draw_message", place = about);
        self.design_ask_with(text.clone(), text, window, cx);
    }

    /// No map yet: the designer asked to make one (or the PM to bring one in).
    pub fn flows_ask_map(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.design_ask_with(t!("flows.ask_designer_message").into(), t!("flows.ask_pm_message").into(), window, cx);
    }

    /// No walkthrough of the flow yet: the designer asked to record one.
    pub fn flows_ask_video(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(flow) = self.flow_map().and_then(|m| m.flows.iter().find(|f| f.id == id)) else { return };
        let text = t!("flows.video_ask_message", flow = flow.title.clone(), flow_id = flow.id.clone());
        self.design_ask_with(text.clone(), text, window, cx);
    }

    /// 화면 지도's keys: ←/→ between screens, ↑/↓ between flows, Esc
    /// closes the open one — not while the box has the words.
    pub fn on_flows_key(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.design_tab() != Tab::Map || self.flow_talk.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        let Some(map) = self.flow_map().cloned() else { return false };
        if map.flows.is_empty() {
            return false;
        }
        match key {
            "left" | "right" => {
                if let Some(at) = self.design.map.step_screen(&map, key == "right") {
                    // The card and its arrow before it.
                    self.design.map.strip.scroll_to_item(at * 2);
                }
            }
            "up" | "down" => {
                self.design.map.step_flow(&map, key == "down");
                self.design.map.strip.set_offset(point(px(0.), px(0.)));
            }
            "escape" if self.design.map.screen.is_some() => self.flows_close(cx),
            _ => return false,
        }
        cx.notify();
        true
    }
}
