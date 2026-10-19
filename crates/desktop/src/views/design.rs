//! 디자인 시스템: the squad's design system as its designer keeps it
//! (`docs/DESIGN-SYSTEM.md`), read through the daemon. A header — the
//! project's when projects differ, a "검토 중 · T-n" pill for a draft in
//! review, what changed since the person last looked — over sections:
//! 개요 (logos on light and dark, the lead colours, a type sample), 색
//! (swatches by group, light and dark side by side, contrast graded; a click
//! copies), 타이포그래피 (live text in each style, a word when the font isn't
//! here), 간격 · 모서리 · 그림자 · 모션 (specimens; a curve plays under the
//! pointer), 에셋 (pictures on a checkerboard, big in a lightbox) and
//! 가이드라인 (guidelines.md with its contents). With none yet, what one is
//! and a way to ask the designer. ←/→ or 1–9 move between sections.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use equipe_core::design::contrast::{self, Grade};
use equipe_core::design::diff::Change;
use equipe_core::design::values::{self, Color};
use equipe_core::protocol::design::{AssetKind, DesignSystem, Token};
use equipe_core::protocol::Command;
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::design::{self as ds, Section};
use crate::visuals::{Look as Shown, Size};
use crate::icons::{Icon, icon};
use crate::render::view::Look;
use crate::theme::{BODY, BODY_LINE, CONTROL, ICON, ICON_SM, MEASURE, MONO, META, Palette, R_LG, R_MD, SMALL, UI};
use crate::tips::TipExt as _;
use crate::{Screen, Shell, motion, ui};

/// The page's side margin and the widest its content goes.
const PAD: f32 = 32.;
const MAX_W: f32 = 1120.;
/// A colour swatch's card.
const SWATCH_W: f32 = 200.;
const SWATCH_H: f32 = 84.;
/// An asset's tile.
const TILE_W: f32 = 208.;
const TILE_H: f32 = 148.;
/// How long a "copied" mark stays.
const COPIED_FOR: Duration = Duration::from_millis(1400);
/// How long the screen waits for a daemon that may not know `design`.
const ANSWER_WITHIN: Duration = Duration::from_secs(5);

fn color(c: Color) -> Rgba {
    rgba(c.rgba())
}

fn hsla_of(c: Color) -> Hsla {
    color(c).into()
}

/// A token's name the way the person refers to it.
fn ref_of(name: &str) -> String {
    format!("{{{name}}}")
}

// ── the screen ──

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.design;
    let body: AnyElement = match ui.current() {
        _ if !ui.loaded => waiting(shell),
        None => empty(shell, cx),
        Some(system) => page(shell, system, cx),
    };
    v_flex()
        .id("design")
        .flex_1()
        .min_h_0()
        .relative()
        .bg(p.bg)
        .child(header(shell, cx))
        .children(ui.loaded.then(|| ui.current().filter(|s| s.draft.is_some()).map(|s| draft_banner(shell, s, cx))).flatten())
        .children(ui.loaded.then(|| ui.current().map(|s| tabs(shell, s, cx))).flatten())
        .child(body)
        .children(changes_popover(shell, cx))
        .into_any_element()
}

/// The title bar: the screen's name, which system, its draft, what changed, its folder.
fn header(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let ui = &shell.design;
    let sources = ui.sources();
    let accepted = ui.accepted().len();
    let current = ui.current();
    let draft = current.and_then(|_| ui.draft_of(&ui.source)).and_then(|d| d.draft.clone());
    let only_draft = current.is_some_and(|s| s.draft.is_some()) && ui.systems.iter().all(|s| s.draft.is_some() || ds::base_key(s) != ui.source);
    let changes = current.map(|s| ui.changes(&shell.design_seen_key(s))).unwrap_or_default();
    let dir = current.map(|s| s.dir.clone()).unwrap_or_else(|| ui.folder.clone());
    let remote = shell.endpoint.is_remote();
    let mut bar = h_flex()
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pr_4()
        .pl(px(shell.header_inset(16., cx)))
        .gap_3()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .window_control_area(WindowControlArea::Drag)
        .child(div().flex_none().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("design.title")));
    // Projects with a look of their own: one segment each.
    if sources.len() > 1 || accepted > 1 {
        let mut seg = h_flex().flex_none().h(px(26.)).p(px(2.)).gap(px(2.)).rounded(px(R_MD)).bg(p.wash);
        for key in sources {
            let on = key == ui.source;
            let label = match key.strip_prefix("project:") {
                Some(name) => name.to_owned(),
                None => t!("design.source_squad").to_owned(),
            };
            let pick = key.clone();
            seg = seg.child(
                div()
                    .id(SharedString::from(format!("design-src-{key}")))
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
                    .on_click(cx.listener(move |this, _, _, cx| this.design_pick_source(&pick, cx))),
            );
        }
        bar = bar.child(seg);
    }
    if let Some(d) = draft {
        let viewing = current.is_some_and(|s| s.draft.is_some());
        let pill = h_flex()
            .id("design-draft-pill")
            .flex_none()
            .h(px(24.))
            .px_2()
            .gap_1p5()
            .items_center()
            .rounded(px(12.))
            .border_1()
            .border_color(Rgba { a: 0.55, ..p.amber })
            .when(viewing, |b| b.bg(Rgba { a: 0.14, ..p.amber }))
            .text_size(px(META))
            .text_color(p.text)
            .child(div().size(px(6.)).rounded_full().bg(p.amber))
            .child(t!("design.in_review", id = d.ticket))
            .when(!only_draft, |b| {
                b.cursor_pointer().hover(move |s| s.bg(Rgba { a: 0.1, ..p.amber })).on_click(cx.listener(|this, _, _, cx| this.design_toggle_draft(cx)))
            })
            .tip(p, if viewing { t!("design.back_to_accepted").to_owned() } else { t!("design.view_draft", title = d.title) }, None);
        bar = bar.child(pill);
    }
    bar = bar.child(div().flex_1());
    if !changes.is_empty() {
        let n = changes.len();
        bar = bar.child(
            h_flex()
                .id("design-changes")
                .flex_none()
                .h(px(26.))
                .px_2()
                .gap_1p5()
                .items_center()
                .rounded(px(R_MD))
                .bg(p.accent_bg)
                .text_size(px(SMALL))
                .text_color(p.accent)
                .cursor_pointer()
                .hover(move |s| s.bg(Rgba { a: p.accent_bg.a * 1.8, ..p.accent_bg }))
                .child(div().size(px(6.)).rounded_full().bg(p.accent))
                .child(t!("design.changed", count = n))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.design.changes_open = !this.design.changes_open;
                    cx.notify();
                })),
        );
    }
    if !dir.is_empty() {
        let tip = if remote { t!("design.copy_folder") } else { t!("design.open_folder") };
        bar = bar.child(
            ui::icon_button(p, "design-folder", Icon::Folder, tip, None).on_click(cx.listener(move |this, _, _, cx| this.design_open_folder(&dir, cx))),
        );
    }
    bar
}

/// The sections there is something in, a line under the title bar.
fn tabs(shell: &Shell, system: &DesignSystem, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let on = shell.design.section;
    let mut row = h_flex().id("design-tabs").flex_none().h(px(40.)).px(px(PAD)).gap_1().items_end().border_b_1().border_color(p.hair).overflow_x_scroll();
    for (n, section) in ds::sections(system).into_iter().enumerate() {
        let active = section == on;
        let count = ds::count(system, section).filter(|c| *c > 0 && section != Section::Guidelines);
        row = row.child(
            v_flex()
                .id(SharedString::from(format!("design-tab-{n}")))
                .flex_none()
                .h_full()
                .cursor_pointer()
                .child(
                    h_flex()
                        .flex_1()
                        .px_2()
                        .gap_1p5()
                        .items_center()
                        .rounded_t(px(6.))
                        .text_size(px(SMALL))
                        .text_color(if active { p.text } else { p.text2 })
                        .when(!active, |d| d.hover(move |s| s.text_color(p.text)))
                        .child(ui::steady(section.label(), FontWeight::MEDIUM, active, false))
                        .children(count.map(|c| div().text_size(px(META)).text_color(p.muted).child(c.to_string()))),
                )
                .child(div().h(px(2.)).mx_1().rounded(px(1.)).when(active, |d| d.bg(p.text)))
                .tip(p, section.label(), Some(digit(n)))
                .on_click(cx.listener(move |this, _, _, cx| this.design_section(section, cx))),
        );
    }
    row
}

fn digit(n: usize) -> &'static str {
    ["1", "2", "3", "4", "5", "6", "7", "8", "9"].get(n).copied().unwrap_or("")
}

/// A ticket's draft on screen: whose, and how to get back.
fn draft_banner(shell: &Shell, system: &DesignSystem, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let d = system.draft.clone().expect("a draft");
    let accepted = shell.design.accepted().iter().any(|a| ds::base_key(a) == ds::base_key(system));
    let ticket = d.ticket;
    h_flex()
        .flex_none()
        .min_h(px(40.))
        .px(px(PAD))
        .py_1()
        .gap_3()
        .items_center()
        .bg(Rgba { a: 0.10, ..p.amber })
        .border_b_1()
        .border_color(Rgba { a: 0.3, ..p.amber })
        .text_size(px(SMALL))
        .child(div().size(px(7.)).flex_none().rounded_full().bg(p.amber))
        .child(div().flex_1().min_w_0().text_color(p.text).child(t!("design.draft_banner", id = d.ticket, title = d.title)))
        .child(ui::button(p, "design-open-ticket", ui::Kind::Secondary, t!("design.open_ticket")).icon(Icon::Ticket).on_click(cx.listener(move |this, _, _, cx| this.open_ticket(ticket, cx))))
        .when(accepted, |b| b.child(ui::button(p, "design-accepted", ui::Kind::Ghost, t!("design.back_to_accepted")).on_click(cx.listener(|this, _, _, cx| this.design_toggle_draft(cx)))))
}

/// The daemon hasn't answered: a moment, or a daemon that predates the screen.
fn waiting(shell: &Shell) -> AnyElement {
    let p = shell.palette;
    let old = shell.design.asked.is_some_and(|at| at.elapsed() > ANSWER_WITHIN);
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .text_size(px(SMALL))
        .text_color(p.muted)
        .when(!old, |d| d.child(motion::spinner(16., p.muted)).child(t!("design.loading")))
        .when(old, |d| d.child(div().max_w(px(420.)).text_center().child(t!("design.daemon_too_old"))))
        .into_any_element()
}

/// None yet: what a design system is, and a way to ask for one.
fn empty(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let folder = shell.design.folder.clone();
    let has_designer = shell.model.members.iter().any(|m| m.role == "designer");
    let point = |text: String| h_flex().gap_2().items_start().child(div().mt(px(9.)).size(px(4.)).flex_none().rounded_full().bg(p.muted)).child(div().flex_1().child(text));
    v_flex()
        .id("design-empty")
        .flex_1()
        .overflow_y_scroll()
        .items_center()
        .justify_center()
        .p(px(PAD))
        .child(
            motion::rise(
                v_flex()
                    .max_w(px(520.))
                    .gap_4()
                    .child(
                        div()
                            .size(px(44.))
                            .rounded(px(R_LG))
                            .bg(p.raised)
                            .border_1()
                            .border_color(p.hair)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(super::sidebar::icon(IconName::Palette, 22., p.text2)),
                    )
                    .child(div().text_size(px(20.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("design.empty_title")))
                    .child(div().text_size(px(BODY)).line_height(px(BODY_LINE)).text_color(p.text2).child(t!("design.empty_text")))
                    .child(
                        v_flex()
                            .gap_1()
                            .text_size(px(SMALL))
                            .line_height(px(22.))
                            .text_color(p.text2)
                            .child(point(t!("design.empty_tokens").into()))
                            .child(point(t!("design.empty_guidelines").into()))
                            .child(point(t!("design.empty_assets").into())),
                    )
                    .child(
                        h_flex().pt_2().gap_2().child(
                            ui::button(p, "design-ask", ui::Kind::Primary, if has_designer { t!("design.ask_designer") } else { t!("design.ask_pm") })
                                .icon(Icon::Message)
                                .on_click(cx.listener(|this, _, window, cx| this.design_ask_designer(window, cx))),
                        ),
                    )
                    .when(!folder.is_empty(), |d| {
                        d.child(
                            h_flex()
                                .gap_1p5()
                                .text_size(px(META))
                                .text_color(p.muted)
                                .child(t!("design.lives_in"))
                                .child(div().font_family(MONO).text_color(p.text2).child(short_home(&folder))),
                        )
                    }),
                "design-empty-in",
            ),
        )
        .into_any_element()
}

/// `~/…` for a path under the person's home.
fn short_home(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_owned(),
    }
}

/// The system on screen, its section.
fn page(shell: &Shell, system: &DesignSystem, cx: &mut Context<Shell>) -> AnyElement {
    let section = shell.design.section;
    if section == Section::Guidelines {
        return guidelines(shell, system, cx);
    }
    let content: AnyElement = match section {
        Section::Overview => overview(shell, system, cx),
        Section::Colors => colors(shell, system, cx),
        Section::Typography => typography(shell, system, cx),
        Section::Spacing => spacing(shell, system, cx),
        Section::Radius => radius(shell, system, cx),
        Section::Shadow => shadows(shell, system, cx),
        Section::Motion => motion_section(shell, system, cx),
        Section::Assets => assets(shell, system, cx),
        Section::Guidelines => unreachable!(),
    };
    div()
        .id("design-body")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&shell.design.scroll)
        .child(div().w_full().flex().justify_center().child(div().w_full().max_w(px(MAX_W + 2. * PAD)).px(px(PAD)).pt(px(24.)).pb(px(64.)).child(content)))
        .into_any_element()
}

/// A section's heading and what it says, over its content.
fn heading(p: Palette, title: String, note: Option<String>) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .mb_3()
        .child(div().text_size(px(UI)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title))
        .children(note.map(|n| div().text_size(px(SMALL)).text_color(p.muted).child(n)))
}

/// A group's name over its tokens: `color.brand`.
fn group_label(p: Palette, name: &str) -> impl IntoElement + use<> {
    div().mt_5().mb_2().font_family(MONO).text_size(px(META)).text_color(p.muted).child(if name.is_empty() { "—".to_owned() } else { name.to_owned() })
}

/// A token's name: a click copies it as `{color.bg}`, and says so.
fn name_button(shell: &Shell, token: &Token, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let reference = ref_of(&token.name);
    let copied = shell.design_copied(&reference);
    let id = SharedString::from(format!("design-name-{}", token.name));
    h_flex()
        .id(id)
        .min_w_0()
        .gap_1()
        .items_center()
        .rounded(px(4.))
        .cursor_pointer()
        .font_family(MONO)
        .text_size(px(11.5))
        .text_color(if copied { p.accent } else { p.muted })
        .hover(move |s| s.text_color(p.text))
        .child(div().min_w_0().truncate().child(token.name.clone()))
        .when(copied, |b| b.child(icon(Icon::Check, 11., p.accent)))
        .tip(p, t!("design.copy_name", name = reference.clone()), None)
        .on_click(cx.listener(move |this, _, _, cx| this.design_copy(reference.clone(), cx)))
        .into_any_element()
}

/// A contrast grade, as a small badge: AAA, AA, 큰 글자, or 미달 — with the ratio.
fn grade_badge(p: Palette, ratio: f64, what: String, id: SharedString) -> AnyElement {
    let tones = crate::render::tones(&p);
    let grade = contrast::grade(ratio);
    let (label, fg, bg) = match grade {
        Grade::Aaa => ("AAA".to_owned(), tones.added, tones.added_bg),
        Grade::Aa => ("AA".to_owned(), tones.added, tones.added_bg),
        Grade::Large => (t!("design.grade_large").to_owned(), p.amber, Rgba { a: 0.14, ..p.amber }),
        Grade::Fail => (t!("design.grade_fail").to_owned(), p.danger(), p.danger_bg()),
    };
    h_flex()
        .id(id)
        .flex_none()
        .h(px(18.))
        .px(px(5.))
        .gap_1()
        .items_center()
        .rounded(px(4.))
        .bg(bg)
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg)
        .child(label)
        .child(div().font_weight(FontWeight::NORMAL).child(format!("{ratio:.1}")))
        .tip(p, what, None)
        .into_any_element()
}

// ── 개요 ──

fn overview(shell: &Shell, system: &DesignSystem, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let (light_bg, dark_bg) = ds::backgrounds(system);
    let logos: Vec<(usize, &equipe_core::protocol::design::Asset)> = ds::DesignUi::logos(system).into_iter().take(3).map(|i| (i, &system.assets[i])).collect();
    let mut col = v_flex().gap_8();
    if !system.problems.is_empty() {
        col = col.child(problems(shell, &system.problems, cx));
    }
    if !logos.is_empty() {
        let mut row = h_flex().gap_3().flex_wrap();
        for (ix, logo) in &logos {
            let (on_light, on_dark) = ds::logo_sides(&logo.name);
            for (on, bg, mode) in [(on_light, light_bg, "light"), (on_dark, dark_bg, "dark")] {
                if !on {
                    continue;
                }
                row = row.child(logo_tile(shell, *ix, &logo.path, bg, mode, cx));
            }
        }
        col = col.child(v_flex().child(heading(p, t!("design.logos").into(), None)).child(row));
    }
    let leads = ds::primary_colors(system, 7);
    if !leads.is_empty() {
        let mut row = h_flex().gap_3().flex_wrap();
        for token in leads {
            row = row.child(lead_chip(shell, system, token, cx));
        }
        col = col.child(v_flex().child(heading(p, t!("design.lead_colors").into(), None)).child(row));
    }
    if let Some(style) = ds::display_style(system) {
        col = col.child(v_flex().child(heading(p, t!("design.type_sample_title").into(), None)).child(type_hero(shell, system, style)));
    }
    // Every section, how much is in it.
    let mut cards = h_flex().gap_3().flex_wrap();
    for section in ds::sections(system).into_iter().filter(|s| *s != Section::Overview) {
        let n = ds::count(system, section).unwrap_or(0);
        cards = cards.child(
            v_flex()
                .id(SharedString::from(format!("design-count-{}", section.key())))
                .w(px(150.))
                .p_3()
                .gap_0p5()
                .rounded(px(R_LG))
                .bg(p.raised)
                .border_1()
                .border_color(p.hair)
                .cursor_pointer()
                .hover(move |s| s.border_color(p.muted))
                .child(div().text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(n.to_string()))
                .child(div().text_size(px(SMALL)).text_color(p.text2).child(section.label()))
                .on_click(cx.listener(move |this, _, _, cx| this.design_section(section, cx))),
        );
    }
    let any = ds::sections(system).into_iter().any(|s| s != Section::Overview && ds::count(system, s).unwrap_or(0) > 0);
    col = col.child(v_flex().child(heading(p, t!("design.at_a_glance").into(), None)).child(if any {
        cards.into_any_element()
    } else {
        div().text_size(px(SMALL)).text_color(p.muted).child(t!("design.nothing_yet")).into_any_element()
    }));
    col.into_any_element()
}

/// What couldn't be read: one line, its count, with the way to have it fixed
/// beside it; the list itself only when asked for.
fn problems(shell: &Shell, problems: &[String], cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let open = shell.design.problems_open;
    let shown = 8;
    let toggle = ui::button(p, "design-problems-toggle", ui::Kind::Ghost, if open { t!("design.problems_less") } else { t!("design.problems_more") })
        .on_click(cx.listener(|this, _, _, cx| {
            this.design.problems_open = !this.design.problems_open;
            cx.notify();
        }));
    let listed: Vec<String> = problems.to_vec();
    let fix = ui::button(p, "design-problems-fix", ui::Kind::Secondary, t!("design.ask_fix")).on_click(cx.listener(move |this, _, window, cx| this.design_ask_fix(&listed, window, cx)));
    v_flex()
        .p_3()
        .gap_1()
        .rounded(px(R_LG))
        .border_1()
        .border_color(Rgba { a: 0.45, ..p.amber })
        .bg(Rgba { a: 0.07, ..p.amber })
        .text_size(px(SMALL))
        .child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .items_center()
                .child(
                    h_flex()
                        .flex_1()
                        .min_w(px(ui::WORDS_AT_LEAST))
                        .gap_2()
                        .items_center()
                        .text_color(p.text)
                        .font_weight(FontWeight::MEDIUM)
                        .child(icon(Icon::Alert, ICON_SM, p.amber))
                        .child(t!("design.problems", count = problems.len())),
                )
                .child(h_flex().flex_none().ml_auto().gap_2().child(toggle).child(fix)),
        )
        .when(!open, |c| c.child(div().pl(px(22.)).text_color(p.text2).child(t!("design.problems_hint"))))
        .when(open, |c| {
            c.children(problems.iter().take(shown).map(|text| div().pl(px(22.)).text_color(p.text2).child(text.clone())))
                .when(problems.len() > shown, |c| c.child(div().pl(px(22.)).text_color(p.muted).child(t!("design.problems_rest", count = problems.len() - shown))))
        })
        .into_any_element()
}

/// A logo on the system's own light or dark background.
fn logo_tile(shell: &Shell, ix: usize, path: &str, bg: Color, mode: &str, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let face = picture(shell, path, if ds::is_dark(bg) { rgba(0xFFFFFF99) } else { rgba(0x00000080) });
    v_flex()
        .gap_1p5()
        .child(
            div()
                .id(SharedString::from(format!("design-logo-{ix}-{mode}")))
                .w(px(260.))
                .h(px(156.))
                .p(px(32.))
                .rounded(px(R_LG))
                .bg(color(bg))
                .border_1()
                .border_color(p.hair)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .child(face)
                .on_click(cx.listener(move |this, _, _, cx| this.design_lightbox(Some(ix), cx))),
        )
        .child(div().text_size(px(META)).text_color(p.muted).child(if mode == "dark" { t!("design.on_dark") } else { t!("design.on_light") }))
        .into_any_element()
}

/// A lead colour: light over dark when it has both, its name and value.
fn lead_chip(shell: &Shell, system: &DesignSystem, token: &Token, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let modes = modes_of(system, token);
    let mut block = h_flex().h(px(64.)).rounded_t(px(R_LG)).overflow_hidden();
    for (mode, c) in &modes {
        block = block.child(color_cell(shell, &token.name, mode.as_deref(), *c, cx));
    }
    let hex = modes.first().map(|(_, c)| c.hex()).unwrap_or_default();
    v_flex()
        .w(px(132.))
        .rounded(px(R_LG))
        .border_1()
        .border_color(p.hair)
        .bg(p.raised)
        .child(block)
        .child(v_flex().px_2p5().py_2().gap_0p5().child(div().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).truncate().child(token.leaf().to_owned())).child(div().font_family(MONO).text_size(px(11.)).text_color(p.muted).child(hex)))
        .into_any_element()
}

/// The biggest type style, set in its own font, in the person's language and English.
fn type_hero(shell: &Shell, system: &DesignSystem, style: &Token) -> AnyElement {
    let p = shell.palette;
    let t = values::typography(&style.value).unwrap_or_default();
    let (family, exact) = ds::resolve_font(&t.family, shell.design.fonts.as_ref().unwrap_or(&HashSet::new()));
    let size = t.size.unwrap_or(32.).clamp(20., 56.) as f32;
    let body = system.tokens.iter().find(|x| x.kind == "typography" && x.name.to_ascii_lowercase().contains("body"));
    v_flex()
        .p_6()
        .gap_3()
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .child(
            div()
                .font_family(SharedString::from(family.clone()))
                .text_size(px(size))
                .line_height(px(size * 1.2))
                .font_weight(FontWeight(f32::from(t.weight.unwrap_or(600))))
                .text_color(p.text)
                .child(t!("design.sample_title")),
        )
        .children(body.and_then(|b| values::typography(&b.value)).map(|b| {
            let (family, _) = ds::resolve_font(&b.family, shell.design.fonts.as_ref().unwrap_or(&HashSet::new()));
            let size = b.size.unwrap_or(15.) as f32;
            div()
                .max_w(px(MEASURE))
                .font_family(SharedString::from(family))
                .text_size(px(size))
                .line_height(px(b.line.map_or(size * 1.6, |l| l as f32)))
                .font_weight(FontWeight(f32::from(b.weight.unwrap_or(400))))
                .text_color(p.text2)
                .child(t!("design.sample_body"))
        }))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .text_size(px(META))
                .text_color(p.muted)
                .child(div().font_family(MONO).child(format!("{} · {}", style.name, values::describe(&style.kind, &style.value))))
                .when(!exact, |d| d.child(font_missing(p, t.family.first().cloned().unwrap_or_default(), family))),
        )
        .into_any_element()
}

/// "Inter 없음 · Helvetica Neue로 표시" — a style's font isn't on this Mac.
fn font_missing(p: Palette, wanted: String, shown: String) -> impl IntoElement {
    h_flex()
        .flex_none()
        .h(px(20.))
        .px_1p5()
        .gap_1()
        .items_center()
        .rounded(px(4.))
        .bg(Rgba { a: 0.14, ..p.amber })
        .text_size(px(11.))
        .text_color(p.text2)
        .child(icon(Icon::Alert, 11., p.amber))
        .child(t!("design.font_missing", wanted = wanted, shown = shown))
}

// ── 색 ──

/// A colour token's value in each mode it has (or its one value), parsed.
fn modes_of(system: &DesignSystem, token: &Token) -> Vec<(Option<String>, Color)> {
    if token.modes.is_empty() {
        return values::color(&token.value).map(|c| vec![(None, c)]).unwrap_or_default();
    }
    system.modes.iter().filter_map(|m| values::color(token.value_in(Some(m))).map(|c| (Some(m.clone()), c))).collect()
}

/// One mode's side of a swatch: its colour; a click copies its hex.
fn color_cell(shell: &Shell, name: &str, mode: Option<&str>, c: Color, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let hex = c.hex();
    let copied = shell.design_copied(&hex);
    let ink = if ds::is_dark(contrast::over(c, if p.is_dark() { Color::rgb(0x16171A) } else { Color::WHITE })) { rgba(0xFFFFFFE6) } else { rgba(0x000000B3) };
    let group = SharedString::from(format!("cell-{name}-{}", mode.unwrap_or("")));
    div()
        .id(SharedString::from(format!("design-cell-{name}-{}", mode.unwrap_or(""))))
        .group(group.clone())
        .flex_1()
        .h_full()
        .relative()
        .bg(color(c))
        .cursor_pointer()
        .child(
            div()
                .absolute()
                .bottom(px(6.))
                .left(px(8.))
                .text_size(px(10.5))
                .font_family(MONO)
                .text_color(ink)
                .when(!copied, |d| d.invisible().group_hover(group, |s| s.visible()))
                .child(if copied { t!("design.copied").to_owned() } else { t!("design.copy").to_owned() }),
        )
        .tip(p, t!("design.copy_hex", hex = hex.clone()), None)
        .on_click(cx.listener(move |this, _, _, cx| this.design_copy(hex.clone(), cx)))
        .into_any_element()
}

fn swatch(shell: &Shell, system: &DesignSystem, token: &Token, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let modes = modes_of(system, token);
    let mut block = h_flex().h(px(SWATCH_H)).border_b_1().border_color(p.hair);
    for (mode, c) in &modes {
        block = block.child(color_cell(shell, &token.name, mode.as_deref(), *c, cx));
    }
    let mut rows = v_flex().gap_1();
    for (mode, c) in &modes {
        let dark = mode.as_deref().map_or_else(|| ds::is_dark(contrast::background(&system.tokens, None, Color::WHITE)), |m| m.to_ascii_lowercase().contains("dark"));
        let surface = contrast::is_surface(token);
        let base = if dark { Color::rgb(0x0A0A0C) } else { Color::WHITE };
        let on = contrast::drawn_on(&system.tokens, token).and_then(|t| values::color(t.value_in(mode.as_deref())).map(|c| (t.leaf().to_owned(), c)));
        let against = if surface { contrast::text(&system.tokens, mode.as_deref(), if dark { Color::rgb(0xEFEDE7) } else { Color::rgb(0x15161A) }) } else { contrast::background(&system.tokens, mode.as_deref(), base) };
        let what = |ratio: f64| {
            let ratio = format!("{ratio:.2}");
            match &on {
                Some((name, c)) => t!("design.contrast_on", name = name, hex = c.hex(), ratio = ratio),
                None if surface => t!("design.contrast_text", hex = against.hex(), ratio = ratio),
                None => t!("design.contrast_bg", hex = against.hex(), ratio = ratio),
            }
        };
        let ratio = contrast::measure(&system.tokens, token, mode.as_deref(), dark);
        let label = match mode.as_deref() {
            Some(m) if m.eq_ignore_ascii_case("light") => t!("design.mode_light").to_owned(),
            Some(m) if m.eq_ignore_ascii_case("dark") => t!("design.mode_dark").to_owned(),
            Some(m) => m.to_owned(),
            None => String::new(),
        };
        let same_as_bg = !surface && on.is_none() && contrast::background(&system.tokens, mode.as_deref(), base) == contrast::over(*c, base);
        rows = rows.child(
            h_flex()
                .gap_1p5()
                .items_center()
                .when(!label.is_empty(), |r| r.child(div().w(px(30.)).flex_none().text_size(px(10.5)).text_color(p.muted).child(label)))
                .child(div().flex_1().font_family(MONO).text_size(px(11.5)).text_color(p.text2).child(c.hex()))
                .children(ratio.filter(|_| !same_as_bg).map(|r| grade_badge(p, r, what(r), SharedString::from(format!("design-grade-{}-{}", token.name, mode.as_deref().unwrap_or("")))))),
        );
    }
    let changed = shell.design_changed(&token.name);
    v_flex()
        .w(px(SWATCH_W))
        .rounded(px(R_LG))
        .overflow_hidden()
        .border_1()
        .border_color(if changed { p.accent } else { p.hair })
        .bg(p.raised)
        .child(block)
        .child(
            v_flex()
                .px_2p5()
                .pt_2()
                .pb_2p5()
                .gap_1p5()
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .child(div().flex_1().min_w_0().truncate().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(token.leaf().to_owned()))
                        .when(changed, |r| r.child(div().size(px(6.)).rounded_full().bg(p.accent))),
                )
                .child(name_button(shell, token, cx))
                .child(rows)
                .children(token.alias.as_ref().map(|a| div().font_family(MONO).text_size(px(10.5)).text_color(p.muted).truncate().child(format!("→ {a}")))),
        )
        .into_any_element()
}

fn colors(shell: &Shell, system: &DesignSystem, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let tokens = ds::tokens_in(system, Section::Colors);
    let mut col = v_flex().child(heading(p, t!("design.section_colors").into(), Some(t!("design.colors_note").into())));
    for (group, list) in ds::groups(&tokens) {
        let mut grid = h_flex().gap_3().flex_wrap().items_start();
        for token in list {
            grid = grid.child(swatch(shell, system, token, cx));
        }
        col = col.child(group_label(p, &group)).child(grid);
    }
    col.into_any_element()
}

// ── 타이포그래피 ──

fn typography(shell: &Shell, system: &DesignSystem, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let empty = HashSet::new();
    let fonts = shell.design.fonts.as_ref().unwrap_or(&empty);
    let local = t!("design.sample_line");
    let english = "The quick brown fox jumps over the lazy dog — 0123456789";
    let english_lang = crate::i18n::lang().code() == "en";
    let mut col = v_flex().child(heading(p, t!("design.section_typography").into(), Some(t!("design.type_note").into())));
    // Type styles first, then sizes, then the families and weights they are made of.
    let mut tokens = ds::tokens_in(system, Section::Typography);
    tokens.sort_by_key(|t| ["typography", "dimension", "fontFamily", "fontWeight"].iter().position(|k| *k == t.kind).unwrap_or(4));
    for (group, list) in ds::groups(&tokens) {
        col = col.child(group_label(p, &group));
        for token in list {
            let (family, weight, size, line, wanted) = match token.kind.as_str() {
                "typography" => {
                    let t = values::typography(&token.value).unwrap_or_default();
                    let size = t.size.unwrap_or(16.) as f32;
                    (t.family.clone(), t.weight.unwrap_or(400), size, t.line.map_or(size * 1.4, |l| l as f32), t.family.first().cloned())
                }
                "fontFamily" => {
                    let stack = values::font_family(&token.value).unwrap_or_default();
                    let first = stack.first().cloned();
                    (stack, 400, 24., 32., first)
                }
                "fontWeight" => (Vec::new(), values::font_weight(&token.value).unwrap_or(400), 24., 32., None),
                _ => {
                    let size = values::dimension(&token.value).unwrap_or(16.) as f32;
                    (Vec::new(), 400, size, size * 1.4, None)
                }
            };
            let (shown, exact) = ds::resolve_font(&family, fonts);
            let size = size.clamp(8., 96.);
            let line = line.max(size);
            let sample = |text: String| {
                div().font_family(SharedString::from(shown.clone())).font_weight(FontWeight(f32::from(weight))).text_size(px(size)).line_height(px(line)).text_color(p.text).truncate().child(text)
            };
            let changed = shell.design_changed(&token.name);
            col = col.child(
                h_flex()
                    .py_4()
                    .gap_6()
                    .items_start()
                    .border_b_1()
                    .border_color(p.hair)
                    .child(
                        v_flex()
                            .w(px(240.))
                            .flex_none()
                            .gap_1()
                            .child(h_flex().gap_1p5().items_center().child(div().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(token.leaf().to_owned())).when(changed, |r| r.child(div().size(px(6.)).rounded_full().bg(p.accent))))
                            .child(name_button(shell, token, cx))
                            .child(div().font_family(MONO).text_size(px(11.5)).text_color(p.text2).child(values::describe(&token.kind, &token.value)))
                            .when(!exact && wanted.is_some(), |c| c.child(font_missing(p, wanted.clone().unwrap_or_default(), shown.clone())))
                            .children(token.description.clone().map(|d| div().text_size(px(META)).text_color(p.muted).child(d))),
                    )
                    .child(v_flex().flex_1().min_w_0().gap_1().overflow_hidden().child(sample(local.to_owned())).when(!english_lang, |c| c.child(sample(english.to_owned())))),
            );
        }
    }
    col.into_any_element()
}

// ── 간격 · 모서리 · 그림자 ──

fn specimen_meta(shell: &Shell, token: &Token, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    v_flex()
        .gap_0p5()
        .min_w_0()
        .child(div().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).truncate().child(token.leaf().to_owned()))
        .child(name_button(shell, token, cx))
        .child(div().font_family(MONO).text_size(px(11.5)).text_color(p.text2).child(values::describe(&token.kind, &token.value)))
}

fn spacing(shell: &Shell, system: &DesignSystem, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let mut col = v_flex().child(heading(p, t!("design.section_spacing").into(), Some(t!("design.spacing_note").into())));
    let tokens = ds::tokens_in(system, Section::Spacing);
    for (group, list) in ds::groups(&tokens) {
        col = col.child(group_label(p, &group));
        for token in list {
            let px_value = values::dimension(&token.value).unwrap_or(0.) as f32;
            col = col.child(
                h_flex()
                    .py_2()
                    .gap_6()
                    .items_center()
                    .border_b_1()
                    .border_color(p.hair)
                    .child(div().w(px(240.)).flex_none().child(specimen_meta(shell, token, cx)))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .child(div().h(px(20.)).w(px(px_value.clamp(1., 640.))).rounded(px(3.)).bg(Rgba { a: 0.22, ..p.accent }).border_1().border_color(Rgba { a: 0.6, ..p.accent })),
                    ),
            );
        }
    }
    col.into_any_element()
}

fn radius(shell: &Shell, system: &DesignSystem, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let mut grid = h_flex().gap_4().flex_wrap().items_start();
    for token in ds::tokens_in(system, Section::Radius) {
        let r = values::dimension(&token.value).unwrap_or(0.) as f32;
        grid = grid.child(
            v_flex()
                .w(px(150.))
                .gap_2()
                .child(div().size(px(120.)).rounded(px(r.min(60.))).bg(p.raised).border_2().border_color(Rgba { a: 0.7, ..p.accent }))
                .child(specimen_meta(shell, token, cx)),
        );
    }
    v_flex().child(heading(p, t!("design.section_radius").into(), None)).child(grid).into_any_element()
}

fn shadows(shell: &Shell, system: &DesignSystem, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    // Shadows read on light: the system's own light background, whatever the window's.
    let (light_bg, _) = ds::backgrounds(system);
    let mut grid = h_flex().gap_4().flex_wrap().items_start();
    for token in ds::tokens_in(system, Section::Shadow) {
        let layers: Vec<BoxShadow> = values::shadows(&token.value)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !s.inset)
            .map(|s| BoxShadow { color: hsla_of(s.color), offset: point(px(s.x as f32), px(s.y as f32)), blur_radius: px(s.blur as f32), spread_radius: px(s.spread as f32), inset: false })
            .collect();
        grid = grid.child(
            v_flex()
                .w(px(232.))
                .gap_2()
                .child(div().w_full().p(px(28.)).rounded(px(R_LG)).bg(color(light_bg)).border_1().border_color(p.hair).child(div().w_full().h(px(96.)).rounded(px(R_LG)).bg(rgb(0xFFFFFF)).shadow(layers)))
                .child(specimen_meta(shell, token, cx)),
        );
    }
    v_flex().child(heading(p, t!("design.section_shadow").into(), Some(t!("design.shadow_note").into()))).child(grid).into_any_element()
}

// ── 모션 ──

/// A token's timing: how long, and on what curve — a curve alone plays
/// for the system's first duration, a duration alone on its first curve.
fn timing(system: &DesignSystem, token: &Token) -> (f64, [f64; 4]) {
    let first_curve = system.tokens.iter().find_map(|t| (t.kind == "cubicBezier").then(|| values::cubic_bezier(&t.value)).flatten()).unwrap_or([0.2, 0., 0., 1.]);
    let first_duration = system.tokens.iter().find_map(|t| (t.kind == "duration").then(|| values::duration_ms(&t.value)).flatten()).unwrap_or(400.);
    match token.kind.as_str() {
        "duration" => (values::duration_ms(&token.value).unwrap_or(first_duration), first_curve),
        "cubicBezier" => (first_duration.max(300.), values::cubic_bezier(&token.value).unwrap_or(first_curve)),
        _ => values::transition(&token.value).map(|(d, _, c)| (d, c)).unwrap_or((first_duration, first_curve)),
    }
}

/// The curve, drawn as dots in a small square.
fn curve_plot(p: Palette, curve: [f64; 4]) -> impl IntoElement {
    const SIDE: f32 = 56.;
    let mut plot = div().relative().size(px(SIDE)).flex_none().rounded(px(6.)).bg(p.code_bg).border_1().border_color(p.hair);
    let [x1, y1, x2, y2] = curve;
    let bez = |a: f64, b: f64, s: f64| 3. * a * (1. - s).powi(2) * s + 3. * b * (1. - s) * s * s + s.powi(3);
    for i in 0..=28 {
        let s = f64::from(i) / 28.;
        let (x, y) = (bez(x1, x2, s), bez(y1, y2, s));
        let inner = SIDE - 12.;
        plot = plot.child(div().absolute().left(px(5. + x as f32 * inner)).top(px(5. + (1. - y as f32).clamp(-0.2, 1.2) * inner)).size(px(2.5)).rounded_full().bg(p.accent));
    }
    plot
}

fn motion_section(shell: &Shell, system: &DesignSystem, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let reduce = cx.reduce_motion();
    let mut col = v_flex().child(heading(p, t!("design.section_motion").into(), Some(t!("design.motion_note").into())));
    const TRACK: f32 = 320.;
    const DOT: f32 = 16.;
    for token in ds::tokens_in(system, Section::Motion) {
        let (ms, curve) = timing(system, token);
        let playing = shell.design.playing.as_ref().filter(|(name, _)| *name == token.name).map(|(_, n)| *n);
        let dot = div().absolute().top(px(8.)).size(px(DOT)).rounded_full().bg(p.accent).shadow_sm();
        let dot: AnyElement = match playing {
            Some(_) if reduce => dot.left(px(TRACK - DOT)).into_any_element(),
            Some(n) => dot
                .left(px(0.))
                .with_animation(
                    ElementId::Name(format!("design-play-{}-{n}", token.name).into()),
                    Animation::new(Duration::from_millis(ms.clamp(16., 5000.) as u64)).with_easing(move |t| values::ease(curve, f64::from(t)) as f32),
                    move |el, t| el.left(px((TRACK - DOT) * t)),
                )
                .into_any_element(),
            None => dot.left(px(0.)).into_any_element(),
        };
        let name = token.name.clone();
        col = col.child(
            h_flex()
                .id(SharedString::from(format!("design-motion-{}", token.name)))
                .py_3()
                .gap_6()
                .items_center()
                .border_b_1()
                .border_color(p.hair)
                .child(div().w(px(240.)).flex_none().child(specimen_meta(shell, token, cx)))
                .child(curve_plot(p, curve))
                .child(
                    div()
                        .relative()
                        .w(px(TRACK))
                        .h(px(32.))
                        .flex_none()
                        .rounded(px(16.))
                        .bg(p.wash)
                        .child(div().absolute().left(px(DOT / 2.)).right(px(DOT / 2.)).top(px(15.5)).h(px(1.)).bg(p.hair))
                        .child(dot),
                )
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.design.plays += 1;
                        this.design.playing = Some((name.clone(), this.design.plays));
                    } else if this.design.playing.as_ref().is_some_and(|(n, _)| *n == name) {
                        this.design.playing = None;
                    }
                    cx.notify();
                })),
        );
    }
    col.into_any_element()
}

// ── 에셋 ──

/// A checkerboard to show transparency on, light or dark: made once.
fn checkerboard(dark: bool) -> Arc<Image> {
    static BOARDS: OnceLock<[Arc<Image>; 2]> = OnceLock::new();
    let boards = BOARDS.get_or_init(|| {
        let make = |a: u32, b: u32| {
            let (w, h, cell) = (416u32, 296u32, 8u32);
            let rgb = |c: u32| image::Rgb([(c >> 16) as u8, (c >> 8) as u8, c as u8]);
            let board = image::RgbImage::from_fn(w, h, |x, y| if ((x / cell) + (y / cell)) % 2 == 0 { rgb(a) } else { rgb(b) });
            let mut out = std::io::Cursor::new(Vec::new());
            let _ = board.write_to(&mut out, image::ImageFormat::Png);
            Arc::new(Image::from_bytes(ImageFormat::Png, out.into_inner()))
        };
        [make(0xFFFFFF, 0xEFEEEA), make(0x26272B, 0x1E1F22)]
    });
    boards[usize::from(dark)].clone()
}

/// A picture of the system's as it stands: drawn, still coming, or why not.
fn picture(shell: &Shell, path: &str, quiet: Rgba) -> AnyElement {
    let shots = shell.shots.borrow();
    match shots.best(path) {
        Some(shot) => img(shot.image).max_w_full().max_h_full().object_fit(ObjectFit::Contain).into_any_element(),
        None if shots.is_gone(path) => icon(Icon::Alert, ICON, quiet).into_any_element(),
        None => motion::spinner(14., quiet),
    }
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1 << 20 { format!("{:.1} MB", bytes as f64 / f64::from(1 << 20)) } else { format!("{} KB", bytes.div_ceil(1024)) }
}

fn assets(shell: &Shell, system: &DesignSystem, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let remote = shell.endpoint.is_remote();
    let mut col = v_flex().child(heading(p, t!("design.section_assets").into(), Some(t!("design.assets_note").into())));
    let kinds = [
        (AssetKind::Logo, "design.kind_logo"),
        (AssetKind::Icon, "design.kind_icon"),
        (AssetKind::Illustration, "design.kind_illustration"),
        (AssetKind::Component, "design.kind_component"),
        (AssetKind::Other, "design.kind_other"),
    ];
    for (kind, key) in kinds {
        let list: Vec<usize> = (0..system.assets.len()).filter(|i| system.assets[*i].kind == kind).collect();
        if list.is_empty() {
            continue;
        }
        let mut grid = h_flex().gap_3().flex_wrap().items_start();
        for ix in list {
            let asset = &system.assets[ix];
            let group = SharedString::from(format!("asset-{ix}"));
            let (path, name) = (asset.path.clone(), asset.name.rsplit('/').next().unwrap_or(&asset.name).to_owned());
            let (download_path, download_name) = (path.clone(), name.clone());
            let action = |id: &str, glyph: Icon, label: String| {
                div()
                    .id(SharedString::from(format!("{id}-{ix}")))
                    .size(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .bg(p.popover())
                    .border_1()
                    .border_color(p.hair)
                    .shadow_sm()
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.raised))
                    .child(icon(glyph, ICON_SM, p.text2))
                    .tip(p, label, None)
            };
            // A logo for one background is shown on that one's checkerboard.
            let stage_dark = match ds::logo_sides(&asset.name) {
                (true, false) => false,
                (false, true) => true,
                _ => p.is_dark(),
            };
            let tile = div()
                .id(SharedString::from(format!("design-asset-{ix}")))
                .group(group.clone())
                .relative()
                .w(px(TILE_W))
                .h(px(TILE_H))
                .rounded_t(px(R_LG))
                .overflow_hidden()
                .cursor_pointer()
                .child(img(checkerboard(stage_dark)).absolute().inset_0().size_full().object_fit(ObjectFit::Cover))
                .child(div().absolute().inset_0().p_5().flex().items_center().justify_center().child(picture(shell, &path, p.muted)))
                .child(
                    h_flex()
                        .absolute()
                        .top(px(8.))
                        .right(px(8.))
                        .gap_1()
                        .invisible()
                        .group_hover(group, |s| s.visible())
                        .child(action("design-dl", Icon::ArrowDown, t!("design.download").into()).on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.design_download(&download_path, &download_name, cx);
                        })))
                        .when(!remote, |b| {
                            let reveal = path.clone();
                            b.child(action("design-reveal", Icon::Folder, t!("design.reveal").into()).on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                let _ = std::process::Command::new("open").arg("-R").arg(&reveal).spawn();
                            }))
                        }),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.design_lightbox(Some(ix), cx)));
            let dims = match shell.shots.borrow().best(&asset.path).and_then(|s| s.dims) {
                Some((w, h)) => format!(" · {w}×{h}"),
                None => String::new(),
            };
            grid = grid.child(
                v_flex()
                    .w(px(TILE_W))
                    .rounded(px(R_LG))
                    .border_1()
                    .border_color(p.hair)
                    .bg(p.raised)
                    .child(tile)
                    .child(
                        v_flex()
                            .px_2p5()
                            .py_2()
                            .gap_0p5()
                            .border_t_1()
                            .border_color(p.hair)
                            .child(div().text_size(px(SMALL)).text_color(p.text).truncate().child(name))
                            .child(div().text_size(px(META)).text_color(p.muted).child(format!("{}{dims}", human_size(asset.size)))),
                    ),
            );
        }
        col = col.child(div().mt_4().mb_2().text_size(px(META)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child(crate::i18n::catalog().get(key))).child(grid);
    }
    col.into_any_element()
}

/// An asset shown big over the window: ←/→ through the others, Esc closes.
pub fn lightbox(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Design {
        return None;
    }
    let at = shell.design.lightbox?;
    let system = shell.design.current()?;
    let asset = system.assets.get(at)?;
    let n = system.assets.len();
    let p = shell.palette;
    let remote = shell.endpoint.is_remote();
    let name = asset.name.rsplit('/').next().unwrap_or(&asset.name).to_owned();
    let dark_stage = match ds::logo_sides(&asset.name) {
        (true, false) => false,
        (false, true) => true,
        _ => p.is_dark(),
    };
    let round = |b: Stateful<Div>, glyph: Icon, label: String, keys: Option<&'static str>| {
        b.size(px(CONTROL)).flex().items_center().justify_center().rounded_full().cursor_pointer().hover(|s| s.bg(rgba(0xFFFFFF1F))).child(icon(glyph, ICON, rgb(0xFFFFFF))).tip(p, label, keys)
    };
    let (path, dl_name) = (asset.path.clone(), name.clone());
    let reveal = asset.path.clone();
    let backdrop = div()
        .id("design-lightbox")
        .absolute()
        .inset_0()
        .bg(rgba(0x000000F0))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .p_8()
        .on_click(cx.listener(|this, _, _, cx| this.design_lightbox(None, cx)))
        .child(motion::rise(
            div()
                .id("design-lightbox-picture")
                .flex_1()
                .min_h_0()
                .w_full()
                .max_w(px(1200.))
                .relative()
                .rounded(px(R_LG))
                .overflow_hidden()
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(img(checkerboard(dark_stage)).absolute().inset_0().size_full().object_fit(ObjectFit::Cover))
                .child(div().absolute().inset_0().p_8().flex().items_center().justify_center().child(picture(shell, &asset.path, rgba(0xFFFFFFB3)))),
            ElementId::Name(format!("design-lightbox-{}", asset.path).into()),
        ))
        .child(
            h_flex()
                .id("design-lightbox-bar")
                .flex_none()
                .h(px(44.))
                .px_2()
                .gap_3()
                .items_center()
                .rounded(px(22.))
                .bg(rgba(0x2A2A2EF2))
                .border_1()
                .border_color(rgba(0xFFFFFF1F))
                .on_click(|_, _, cx| cx.stop_propagation())
                .when(n > 1, |b| b.child(round(div().id("design-lb-prev").on_click(cx.listener(|this, _, _, cx| this.design_step(false, cx))), Icon::ArrowLeft, t!("composer.aria_previous").into(), Some("←"))))
                .child(div().text_size(px(13.)).text_color(rgb(0xFFFFFF)).child(name))
                .child(div().text_size(px(12.)).text_color(rgba(0xFFFFFFB3)).child(format!("{}/{n} · {}", at + 1, human_size(asset.size))))
                .child(round(div().id("design-lb-dl").on_click(cx.listener(move |this, _, _, cx| this.design_download(&path, &dl_name, cx))), Icon::ArrowDown, t!("design.download").into(), None))
                .when(!remote, |b| {
                    b.child(round(
                        div().id("design-lb-reveal").on_click(move |_, _, _| {
                            let _ = std::process::Command::new("open").arg("-R").arg(&reveal).spawn();
                        }),
                        Icon::Folder,
                        t!("design.reveal").into(),
                        None,
                    ))
                })
                .when(n > 1, |b| b.child(round(div().id("design-lb-next").on_click(cx.listener(|this, _, _, cx| this.design_step(true, cx))), Icon::ArrowRight, t!("composer.aria_next").into(), Some("→"))))
                .child(round(div().id("design-lb-close").on_click(cx.listener(|this, _, _, cx| this.design_lightbox(None, cx))), Icon::X, t!("composer.aria_close").into(), Some("Esc"))),
        );
    Some(motion::fade(backdrop, "design-lightbox-in", motion::BASE).into_any_element())
}

// ── 가이드라인 ──

fn guidelines(shell: &Shell, system: &DesignSystem, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let text = system.guidelines.clone().unwrap_or_default();
    let (sections, heads) = ds::split_guidelines(&text);
    let mut toc = v_flex().id("design-toc").w(px(232.)).flex_none().h_full().overflow_y_scroll().pl(px(PAD)).pr_3().pt(px(24.)).pb_8().gap(px(1.)).child(
        div().mb_2().text_size(px(META)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child(t!("design.contents")),
    );
    for (i, h) in heads.iter().enumerate() {
        let on = shell.design.toc_at == i;
        let section = h.section;
        toc = toc.child(
            div()
                .id(SharedString::from(format!("design-toc-{i}")))
                .py_1()
                .pl(px(if h.level > 2 { 18. } else { 8. }))
                .pr_2()
                .rounded(px(6.))
                .text_size(px(if h.level > 2 { 12.5 } else { SMALL }))
                .line_height(px(18.))
                .text_color(if on { p.text } else if h.level > 2 { p.muted } else { p.text2 })
                .when(on, |d| d.bg(p.wash))
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash))
                .child(h.title.clone())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.design.toc_at = i;
                    this.design.scroll.scroll_to_item(section);
                    cx.notify();
                })),
        );
    }
    let look = Look { size: BODY, line: BODY_LINE, color: p.prose, live: false };
    let mut body = v_flex().id("design-body").flex_1().min_w_0().h_full().overflow_y_scroll().track_scroll(&shell.design.scroll).pt(px(24.)).pb(px(96.)).pr(px(PAD));
    for (i, section) in sections.iter().enumerate() {
        body = body.child(div().max_w(px(MEASURE + 160.)).pb_2().child(crate::render::view::nested(p, ("design-guide", crate::render::hash_of((i, section))), section, look)));
    }
    h_flex().flex_1().min_h_0().items_start().child(toc).child(body).into_any_element()
}

// ── what changed ──

fn changes_popover(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if !shell.design.changes_open {
        return None;
    }
    let p = shell.palette;
    let system = shell.design.current()?;
    let changes = shell.design.changes(&shell.design_seen_key(system));
    let row = |change: &Change| {
        let (badge, tone, line) = match change {
            Change::Added { now, .. } => (t!("design.change_added"), crate::render::tones(&p).added, now.clone()),
            Change::Removed { was, .. } => (t!("design.change_removed"), p.danger(), was.clone()),
            Change::Changed { was, now, .. } => (t!("design.change_changed"), p.accent, format!("{was} → {now}")),
        };
        let swatches: Vec<AnyElement> = match change {
            Change::Changed { was, now, .. } => [was, now].into_iter().filter_map(|h| values::color(&serde_json::Value::String(h.clone()))).map(|c| div().size(px(12.)).rounded(px(3.)).border_1().border_color(p.hair).bg(color(c)).into_any_element()).collect(),
            Change::Added { now: h, .. } | Change::Removed { was: h, .. } => values::color(&serde_json::Value::String(h.clone())).map(|c| div().size(px(12.)).rounded(px(3.)).border_1().border_color(p.hair).bg(color(c)).into_any_element()).into_iter().collect(),
        };
        v_flex()
            .py_1p5()
            .gap_0p5()
            .child(h_flex().gap_2().items_center().child(div().flex_none().px_1().rounded(px(3.)).bg(Rgba { a: 0.14, ..tone }).text_size(px(10.5)).text_color(tone).child(badge)).child(div().min_w_0().truncate().font_family(MONO).text_size(px(12.)).text_color(p.text).child(change.name().to_owned())))
            .child(h_flex().pl(px(2.)).gap_1().items_center().children(swatches).child(div().min_w_0().truncate().font_family(MONO).text_size(px(11.5)).text_color(p.muted).child(line)))
    };
    let list = v_flex().id("design-changes-list").max_h(px(360.)).overflow_y_scroll().children(changes.iter().take(80).map(row));
    let panel = v_flex()
        .id("design-changes-panel")
        .relative()
        .w(px(360.))
        .p_3()
        .gap_2()
        .rounded(px(R_LG))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("design.changed_since")))
        .child(list)
        .child(h_flex().justify_end().child(ui::button(p, "design-seen", ui::Kind::Secondary, t!("design.mark_seen")).icon(Icon::Check).on_click(cx.listener(|this, _, _, cx| this.design_mark_seen(cx)))));
    // Placed by its frame, so the pop's travel (which moves `top`) is its own.
    Some(div().absolute().top(px(super::header::HEIGHT + 6.)).right(px(16.)).child(motion::pop(panel, "design-changes-in", false)).into_any_element())
}

// ── what the screen does ──

impl Shell {
    /// Where what the person saw of `system` is kept: by machine, squad and system.
    pub fn design_seen_key(&self, system: &DesignSystem) -> String {
        format!("{}|{}|{}", self.endpoint.label(), self.model.squad, system.key())
    }

    fn design_copied(&self, what: &str) -> bool {
        self.design.copied.as_ref().is_some_and(|(c, at)| c == what && at.elapsed() < COPIED_FOR)
    }

    /// Whether a token changed since the person last looked.
    fn design_changed(&self, name: &str) -> bool {
        self.design.current().is_some_and(|s| self.design.changes(&self.design_seen_key(s)).iter().any(|c| c.name() == name))
    }

    /// 디자인 시스템 (the sidebar, `/design`): the screen, and the daemon asked.
    pub fn open_design(&mut self, cx: &mut Context<Self>) {
        self.screen = Screen::Design;
        if self.design.asked.is_none() {
            self.design_ask();
        }
        self.board_sync(cx);
        cx.notify();
    }

    fn design_ask(&mut self) {
        if self.model.squad.is_empty() {
            return;
        }
        self.design.asked = Some(Instant::now());
        self.send_command(Command::Design { squad: self.model.squad.clone() });
    }

    /// The daemon said what the squad's design systems are.
    pub fn design_arrived(&mut self, squad: String, systems: Vec<DesignSystem>, folder: String) -> bool {
        if squad != self.model.squad {
            return false;
        }
        if !self.design.seen_loaded {
            self.design.seen = ds::load_seen(&equipe_providers::credentials::equipe_home());
            self.design.seen_loaded = true;
        }
        let (label, squad) = (self.endpoint.label(), self.model.squad.clone());
        let key = move |s: &DesignSystem| format!("{label}|{squad}|{}", s.key());
        self.design.arrived(systems, folder, &key);
        true
    }

    /// Each draw: the fonts this Mac has, the pictures on screen asked for,
    /// a daemon that never answered said so — and a look that ended kept.
    pub fn design_prepare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let on = self.screen == Screen::Design;
        if on != self.design.on_screen {
            self.design.on_screen = on;
            let (label, squad) = (self.endpoint.label(), self.model.squad.clone());
            let key = move |s: &DesignSystem| format!("{label}|{squad}|{}", s.key());
            if on {
                self.design.begin_look(&key);
            } else {
                self.design_end_look();
                self.design.lightbox = None;
                self.design.changes_open = false;
            }
        }
        if !on {
            return;
        }
        if self.design.fonts.is_none() {
            self.design.fonts = Some(window.text_system().all_font_names().into_iter().map(|f| f.to_ascii_lowercase()).collect());
        }
        if self.design.asked.is_none() {
            self.design_ask();
        }
        // Not answered yet: look again when it would be late, to say so.
        if !self.design.loaded
            && let Some(at) = self.design.asked
            && at.elapsed() <= ANSWER_WITHIN
        {
            let left = ANSWER_WITHIN.saturating_sub(at.elapsed()) + Duration::from_millis(50);
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(left).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
        self.design_load_pictures(cx);
    }

    /// The person looked: what is on screen is what they have seen.
    fn design_end_look(&mut self) {
        let (label, squad) = (self.endpoint.label(), self.model.squad.clone());
        let key = move |s: &DesignSystem| format!("{label}|{squad}|{}", s.key());
        if self.design.looked(&key) {
            ds::save_seen(&equipe_providers::credentials::equipe_home(), &self.design.seen);
        }
    }

    pub fn design_mark_seen(&mut self, cx: &mut Context<Self>) {
        self.design_end_look();
        self.design.changes_open = false;
        cx.notify();
    }

    /// Ask for the pictures the system on screen shows — a thumbnail each,
    /// the one in the lightbox whole — through the window's pictures
    /// (`crate::visuals`), which read them through the daemon off the UI thread.
    fn design_load_pictures(&mut self, cx: &mut Context<Self>) {
        let Some(system) = self.design.current() else { return };
        let big = self.design.lightbox.and_then(|at| system.assets.get(at)).map(|a| a.path.clone());
        let wanted = {
            let mut shots = self.shots.borrow_mut();
            for asset in &system.assets {
                let _: Shown = shots.look(&asset.path, Size::Thumb);
            }
            if let Some(path) = &big {
                let _ = shots.look(path, Size::Full);
            }
            shots.has_wanted() && !shots.scheduled
        };
        if wanted {
            self.load_shots(cx);
        }
    }

    pub fn design_section(&mut self, section: Section, cx: &mut Context<Self>) {
        if self.design.section != section {
            self.design.section = section;
            self.design.scroll.set_offset(point(px(0.), px(0.)));
            self.design.toc_at = 0;
            self.design.playing = None;
        }
        cx.notify();
    }

    pub fn design_pick_source(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.design.source != key {
            self.design_end_look();
            self.design.source = key.to_owned();
            self.design.draft = false;
            let (label, squad) = (self.endpoint.label(), self.model.squad.clone());
            let seen_key = move |s: &DesignSystem| format!("{label}|{squad}|{}", s.key());
            let systems = std::mem::take(&mut self.design.systems);
            let folder = self.design.folder.clone();
            self.design.arrived(systems, folder, &seen_key);
            self.design.scroll.set_offset(point(px(0.), px(0.)));
        }
        cx.notify();
    }

    pub fn design_toggle_draft(&mut self, cx: &mut Context<Self>) {
        self.design.draft = !self.design.draft;
        self.design.lightbox = None;
        if let Some(system) = self.design.current()
            && ds::count(system, self.design.section).is_none()
        {
            self.design.section = Section::Overview;
        }
        cx.notify();
    }

    fn design_copy(&mut self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        self.design.copied = Some((text, Instant::now()));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
        cx.notify();
    }

    pub fn design_lightbox(&mut self, at: Option<usize>, cx: &mut Context<Self>) {
        self.design.lightbox = at;
        cx.notify();
    }

    fn design_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let n = self.design.current().map_or(0, |s| s.assets.len());
        if let Some(at) = self.design.lightbox.filter(|_| n > 0) {
            self.design.lightbox = Some(if forward { (at + 1) % n } else { (at + n - 1) % n });
        }
        cx.notify();
    }

    /// The folder: opened in Finder on this Mac; its path copied from another.
    fn design_open_folder(&mut self, dir: &str, cx: &mut Context<Self>) {
        if self.endpoint.is_remote() {
            cx.write_to_clipboard(ClipboardItem::new_string(dir.to_owned()));
            self.composer.flash = Some(t!("design.folder_copied", path = dir));
        } else if std::path::Path::new(dir).is_dir() {
            let _ = std::process::Command::new("open").arg(dir).spawn();
        } else if let Some(parent) = std::path::Path::new(dir).parent() {
            let _ = std::process::Command::new("open").arg(parent).spawn();
        }
        cx.notify();
    }

    /// A picture saved in Downloads, whole — read through the daemon, so from
    /// another Mac too — and shown there in Finder.
    fn design_download(&mut self, path: &str, name: &str, cx: &mut Context<Self>) {
        let Some(uploader) = self.link.as_ref().map(|l| l.uploader()) else { return };
        let (squad, path, name, notices) = (self.model.squad.clone(), path.to_owned(), name.to_owned(), self.notices.clone());
        std::thread::spawn(move || {
            use base64::Engine as _;
            let saved = uploader.read_image(&squad, &path, None).and_then(|read| {
                let bytes = base64::engine::general_purpose::STANDARD.decode(read.data.as_bytes()).map_err(|e| e.to_string())?;
                let dir = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Downloads");
                let name = if read.scaled { format!("{}-preview.{}", name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s), if read.mime == "image/png" { "png" } else { "jpg" }) } else { name.clone() };
                let mut target = dir.join(&name);
                let mut n = 2;
                while target.exists() {
                    let (stem, ext) = name.rsplit_once('.').unwrap_or((name.as_str(), ""));
                    target = dir.join(format!("{stem} {n}.{ext}"));
                    n += 1;
                }
                std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
                Ok(target)
            });
            let _ = notices.send(match saved {
                Ok(target) => {
                    let _ = std::process::Command::new("open").arg("-R").arg(&target).spawn();
                    t!("design.downloaded", name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                }
                Err(why) => t!("design.download_failed", why = why),
            });
        });
        cx.notify();
    }

    /// "designer에게 만들어 달라고 하기": the request goes as the person's
    /// message, the way the composer sends one — to the designer, or to the
    /// PM to bring one in when the team has none — and the conversation opens.
    pub fn design_ask_designer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let designer = self.model.members.iter().find(|m| m.role == "designer" && m.presence != crate::state::Presence::Closed).map(|m| m.session);
        let pm = self.model.members.iter().find(|m| crate::state::Model::is_pm(m)).map(|m| m.session);
        let (to, text) = match (designer, pm) {
            (Some(d), _) => (d, t!("design.ask_designer_message").to_owned()),
            (None, Some(pm)) => (pm, t!("design.ask_pm_message").to_owned()),
            (None, None) => {
                self.composer.flash = Some(t!("design.no_one_to_ask").into());
                cx.notify();
                return;
            }
        };
        self.select_member(to, window, cx);
        self.composer.set_text(&text, window, cx);
        self.send(window, cx);
    }

    /// "designer에게 고쳐 달라고 하기": what couldn't be read goes to the
    /// designer (or the PM) as the person's message, a few of them named.
    pub fn design_ask_fix(&mut self, problems: &[String], window: &mut Window, cx: &mut Context<Self>) {
        let designer = self.model.members.iter().find(|m| m.role == "designer" && m.presence != crate::state::Presence::Closed).map(|m| m.session);
        let pm = self.model.members.iter().find(|m| crate::state::Model::is_pm(m)).map(|m| m.session);
        let Some(to) = designer.or(pm) else {
            self.composer.flash = Some(t!("design.no_one_to_ask").into());
            cx.notify();
            return;
        };
        let examples = problems.iter().take(5).cloned().collect::<Vec<_>>().join("; ");
        let text = t!("design.fix_message", count = problems.len(), examples = examples);
        self.select_member(to, window, cx);
        self.composer.set_text(&text, window, cx);
        self.send(window, cx);
    }

    /// The design screen's keys: ←/→ and 1–9 move between sections; in the
    /// lightbox ←/→ step and Esc closes; Esc puts what changed away.
    pub fn on_design_key(&mut self, key: &str, m: &Modifiers, cx: &mut Context<Self>) -> bool {
        if self.screen != Screen::Design || m.platform || m.control || m.alt || m.function {
            return false;
        }
        if self.design.lightbox.is_some() {
            match key {
                "left" => self.design_step(false, cx),
                "right" => self.design_step(true, cx),
                "escape" => self.design_lightbox(None, cx),
                _ => return false,
            }
            return true;
        }
        if key == "escape" && (self.design.changes_open || self.design.sources_open) {
            self.design.changes_open = false;
            self.design.sources_open = false;
            cx.notify();
            return true;
        }
        let Some(system) = self.design.current() else { return false };
        let sections = ds::sections(system);
        let at = sections.iter().position(|s| *s == self.design.section).unwrap_or(0);
        let to = match key {
            "left" => at.checked_sub(1),
            "right" => Some(at + 1).filter(|n| *n < sections.len()),
            d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() && d != "0" => Some(usize::from(d.as_bytes()[0] - b'1')).filter(|n| *n < sections.len()),
            _ => return false,
        };
        if let Some(to) = to {
            self.design_section(sections[to], cx);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{human_size, ref_of, short_home, timing};
    use equipe_core::protocol::design::{DesignSystem, Token};

    #[test]
    fn a_path_under_home_reads_short_and_sizes_read_as_people_say_them() {
        if let Ok(home) = std::env::var("HOME") {
            assert_eq!(short_home(&format!("{home}/.equipe/squads/a/home/design")), "~/.equipe/squads/a/home/design");
        }
        assert_eq!(short_home("/srv/design"), "/srv/design");
        assert_eq!(human_size(1_204), "2 KB");
        assert_eq!(human_size(3 << 20), "3.0 MB");
        assert_eq!(ref_of("color.bg"), "{color.bg}");
    }

    #[test]
    fn timing_borrows_what_a_token_lacks_from_the_system() {
        let t = |name: &str, kind: &str, v: serde_json::Value| Token { name: name.into(), kind: kind.into(), value: v, modes: Default::default(), description: None, alias: None };
        let system = DesignSystem { tokens: vec![t("motion.fast", "duration", serde_json::json!("120ms")), t("ease.out", "cubicBezier", serde_json::json!([0, 0, 0.58, 1]))], ..Default::default() };
        assert_eq!(timing(&system, &system.tokens[0]), (120., [0., 0., 0.58, 1.]));
        assert_eq!(timing(&system, &system.tokens[1]), (300., [0., 0., 0.58, 1.]), "a curve plays long enough to see");
        let none = DesignSystem::default();
        assert_eq!(timing(&none, &t("x", "duration", serde_json::json!("200ms"))).0, 200.);
    }
}
