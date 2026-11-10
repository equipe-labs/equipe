//! Compare and review and Version history on the design screen (`crate::compare` holds
//! the rules). A draft in review is compared from the header's button: a
//! band under it names the ticket and how many screens changed, flips the
//! whole screen between Now and After, steps through the changed screens
//! only (←/→), and accepts the ticket or sends it back with words — the
//! board's own way, taken back with U. In the UI map the changed cards say
//! Changed and the open screen shows both sides at once when the window is
//! wide. Version history lists the accepted versions, each to be asked back: a
//! ticket for the designer, reviewed like any other change.

use equipe_core::protocol::Command;
use equipe_core::protocol::design::{DesignVersion, Flow, Screen};
use equipe_core::t;
use gpui_kit::component::input::Input;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::board;
use crate::compare::{self as cm, Side};
use crate::flows::{self as fm, Chip, Tab};
use crate::icons::Icon;
use crate::sheet::{Ask, Then};
use crate::theme::{FIELD, META, MONO, Palette, R_LG, R_MD, R_SM, SMALL};
use crate::tips::TipExt as _;
use crate::{Shell, motion, ui};

/// The open screen's picture, as `views/flows.rs` draws it.
const STAGE_H: f32 = 380.;

// ── the band ──

/// The band under the header while comparing.
pub fn bar(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui_ = &shell.design;
    let Some(draft) = ui_.draft_of(&ui_.source).and_then(|d| d.draft.clone()) else { return div().into_any_element() };
    let changes = ui_.compare_changes().unwrap_or_default();
    let on_map = shell.design_tab() == Tab::Map;
    let side = ui_.side();
    let n = changes.count();
    let ticket = draft.ticket;
    let sides = h_flex()
        .flex_none()
        .h(px(26.))
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(R_MD))
        .bg(p.wash)
        .child(side_button(p, Side::Now, side, cx))
        .child(side_button(p, Side::After, side, cx));
    let mut row = h_flex()
        .id("design-compare")
        .flex_none()
        .min_h(px(44.))
        .px(px(24.))
        .py_1p5()
        .gap_3()
        .flex_wrap()
        .items_center()
        .bg(Rgba { a: 0.10, ..p.amber })
        .border_b_1()
        .border_color(Rgba { a: 0.3, ..p.amber })
        .text_size(px(SMALL))
        .child(div().size(px(7.)).flex_none().rounded_full().bg(p.amber))
        .child(
            div()
                .id("design-compare-ticket")
                .flex_none()
                .font_family(MONO)
                .text_size(px(12.5))
                .text_color(p.text2)
                .cursor_pointer()
                .hover(move |s| s.text_color(p.accent).underline())
                .child(format!("T-{ticket}"))
                .on_click(cx.listener(move |this, _, _, cx| this.open_ticket(ticket, cx))),
        )
        .child(div().flex_1().min_w(px(120.)).truncate().text_color(p.text).font_weight(FontWeight::MEDIUM).child(draft.title.clone()))
        .child(div().flex_none().text_color(if n > 0 { p.text } else { p.muted }).child(if n > 0 { t!("design.compare_changed", count = n) } else { t!("design.compare_changed_none").to_owned() }))
        .child(sides);
    if on_map && n > 0 {
        row = row
            .child(ui::icon_button(p, "design-compare-prev", Icon::ArrowLeft, t!("design.compare_prev"), Some("←")).on_click(cx.listener(|this, _, _, cx| this.design_step_changed(false, cx))))
            .child(ui::icon_button(p, "design-compare-next", Icon::ArrowRight, t!("design.compare_next"), Some("→")).on_click(cx.listener(|this, _, _, cx| this.design_step_changed(true, cx))));
    }
    row = row
        .child(ui::button(p, "design-compare-redo", ui::Kind::Secondary, t!("design.compare_redo")).selected(ui_.redo_open).on_click(cx.listener(|this, _, window, cx| this.design_redo(!this.design.redo_open, window, cx))))
        .child(ui::button(p, "design-compare-accept", ui::Kind::Primary, t!("design.compare_accept")).icon(Icon::Check).on_click(cx.listener(|this, _, _, cx| this.design_accept_draft(cx))))
        .child(ui::icon_button(p, "design-compare-close", Icon::X, t!("design.compare_close"), Some("Esc")).on_click(cx.listener(|this, _, _, cx| this.design_compare(false, cx))));
    let typed = !shell.design_redo.read(cx).value().trim().is_empty();
    v_flex()
        .flex_none()
        .child(row)
        .when(ui_.redo_open, |col| {
            col.child(
                h_flex()
                    .px(px(24.))
                    .py_2()
                    .gap_2()
                    .items_center()
                    .bg(Rgba { a: 0.05, ..p.amber })
                    .border_b_1()
                    .border_color(p.hair)
                    .child(div().flex_1().min_w_0().h(px(FIELD)).px_3().flex().items_center().rounded(px(R_MD)).border_1().border_color(p.hair).bg(p.bg).text_size(px(SMALL)).child(Input::new(&shell.design_redo).appearance(false).bordered(false)))
                    .child(ui::button(p, "design-compare-redo-send", ui::Kind::Primary, t!("design.compare_redo_send")).field().key("⏎").disabled(!typed).on_click(cx.listener(|this, _, window, cx| this.design_redo_send(window, cx)))),
            )
        })
        .into_any_element()
}

fn side_button(p: Palette, side: Side, on: Side, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let active = side == on;
    let label = match side {
        Side::Now => t!("design.compare_now"),
        Side::After => t!("design.compare_after"),
    };
    div()
        .id(match side {
            Side::Now => "design-side-now",
            Side::After => "design-side-after",
        })
        .h_full()
        .px_2p5()
        .flex()
        .items_center()
        .rounded(px(R_SM))
        .text_size(px(META))
        .cursor_pointer()
        .when(active, |d| d.bg(p.raised).text_color(p.text).shadow_sm())
        .when(!active, |d| d.text_color(p.text2).hover(move |s| s.bg(p.wash)))
        .child(ui::steady(label, FontWeight::MEDIUM, active, false))
        .on_click(cx.listener(move |this, _, _, cx| this.design_side(side, cx)))
}

/// The small mark on what changed.
pub fn badge(p: Palette, label: String) -> impl IntoElement {
    h_flex()
        .flex_none()
        .h(px(18.))
        .px_1p5()
        .gap_1()
        .items_center()
        .rounded(px(9.))
        .bg(Rgba { a: 0.18, ..p.amber })
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.text)
        .child(div().size(px(5.)).rounded_full().bg(p.amber))
        .child(label)
}

// ── the open screen, both sides ──

/// The open state's picture as it is now and after — side by side when the
/// window is wide, else the side on show — each saying which it is.
pub fn stage(shell: &Shell, flow: &Flow, screen: &Screen, chip: Option<&Chip>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui_ = &shell.design;
    let Some((now, after)) = ui_.compared_maps() else { return div().into_any_element() };
    let mode = ui_.map.mode(p.is_dark());
    let state = chip.and_then(|c| c.state).map(|s| s.id.clone()).or_else(|| chip.map(|c| c.key.clone())).unwrap_or_default();
    let changed = ui_.compare_changes().is_some_and(|c| c.state(&flow.id, &screen.id, &state));
    let picture = |map: &equipe_core::protocol::design::FlowMap| cm::counterpart(map, &flow.id, &screen.id, &state).and_then(|s| fm::pick(s, &mode, ui_.map.width)).map(|pic| pic.path.clone());
    let sides: Vec<(Side, Option<String>)> = if ui_.wide { vec![(Side::Now, picture(&now)), (Side::After, picture(&after))] } else { vec![(ui_.side(), picture(if ui_.side() == Side::Now { &now } else { &after }))] };
    let mut row = h_flex().w_full().gap_4().items_start();
    for (side, path) in sides {
        let label = match side {
            Side::Now => t!("design.compare_now"),
            Side::After => t!("design.compare_after"),
        };
        let frame = div().w_full().h(px(STAGE_H)).rounded(px(R_MD)).border_1().overflow_hidden();
        let frame: AnyElement = match path {
            Some(path) => frame
                .id(match side {
                    Side::Now => "compare-stage-now",
                    Side::After => "compare-stage-after",
                })
                .p_3()
                .bg(p.wash)
                .border_color(if changed && side == Side::After { Rgba { a: 0.7, ..p.amber } } else { p.hair })
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .child(super::design::picture(shell, &path, p.muted))
                .tip(p, t!("flows.zoom"), None)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.design.show_side(side);
                    this.flows_zoom(true, cx);
                }))
                .into_any_element(),
            None => frame
                .border_dashed()
                .border_color(p.hair)
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .child(div().max_w(px(260.)).text_center().text_size(px(SMALL)).text_color(p.muted).child(match side {
                    Side::Now => t!("design.compare_new"),
                    Side::After => t!("design.compare_gone"),
                }))
                .into_any_element(),
        };
        row = row.child(
            v_flex()
                .flex_1()
                .min_w(px(240.))
                .max_w(px(560.))
                .gap_1p5()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_size(px(META))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if side == Side::After { p.text } else { p.text2 })
                        .child(label)
                        .when(changed && side == Side::After, |r| r.child(badge(p, t!("design.compare_badge").into()))),
                )
                .child(frame),
        );
    }
    row.into_any_element()
}

/// The other side's picture of the state open, to load beside the one on show.
pub fn other_picture(shell: &Shell) -> Option<String> {
    let ui_ = &shell.design;
    if !ui_.compare || !ui_.wide || shell.design_tab() != Tab::Map {
        return None;
    }
    let map = ui_.current()?.flows.as_ref()?;
    let flow = ui_.map.flow(map)?;
    let screen = ui_.map.screen(map)?;
    let state = ui_.map.chip(screen)?.state?.id.clone();
    let (now, after) = ui_.compared_maps()?;
    let other = if ui_.side() == Side::Now { after } else { now };
    let s = cm::counterpart(&other, &flow.id, &screen.id, &state)?;
    fm::pick(s, &ui_.map.mode(shell.palette.is_dark()), ui_.map.width).map(|p| p.path.clone())
}

// ── Version history ──

pub fn versions_popover(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let ui_ = &shell.design;
    if !ui_.versions_open {
        return None;
    }
    let p = shell.palette;
    let body: AnyElement = match ui_.shown_versions() {
        None => h_flex().py_4().gap_2().justify_center().text_size(px(SMALL)).text_color(p.muted).child(motion::spinner(14., p.muted)).child(t!("design.versions_loading")).into_any_element(),
        Some([]) => div().py_3().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(t!("design.versions_empty")).into_any_element(),
        Some(versions) => v_flex().id("design-versions-list").max_h(px(440.)).overflow_y_scroll().children(versions.iter().enumerate().map(|(n, v)| version_row(shell, versions, n, v, cx))).into_any_element(),
    };
    let panel = v_flex()
        .id("design-versions-panel")
        .w(px(540.))
        .p_3()
        .gap_2()
        .rounded(px(R_LG))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            v_flex()
                .gap_0p5()
                .child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("design.versions_title")))
                .child(div().text_size(px(META)).text_color(p.muted).child(t!("design.versions_lead"))),
        )
        .child(body);
    Some(div().absolute().top(px(super::header::HEIGHT + 6.)).right(px(16.)).child(motion::pop(panel, "design-versions-in", false)).into_any_element())
}

fn version_row(shell: &Shell, versions: &[DesignVersion], n: usize, v: &DesignVersion, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let parts = v.parts.iter().map(|part| div().flex_none().px_1p5().rounded(px(4.)).bg(p.wash).text_size(px(10.5)).text_color(p.text2).child(crate::i18n::catalog().get(cm::part_key(*part)).to_owned()));
    let subject = if v.subject.is_empty() { cm::short(v) } else { v.subject.clone() };
    let side: AnyElement = if cm::can_restore(versions, n) {
        ui::button(p, ("design-version-restore", n), ui::Kind::Ghost, t!("design.versions_restore")).icon(Icon::Retry).on_click(cx.listener(move |this, _, _, cx| this.design_restore_ask(n, cx))).into_any_element()
    } else {
        div().flex_none().px_2().h(px(22.)).flex().items_center().rounded(px(11.)).bg(p.accent_bg).text_size(px(META)).text_color(p.accent).child(t!("design.versions_now")).into_any_element()
    };
    let ticket = v.ticket;
    h_flex()
        .id(("design-version", n))
        .py_2()
        .gap_3()
        .items_center()
        .when(n > 0, |d| d.border_t_1().border_color(p.hair))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .items_baseline()
                        .children(ticket.map(|id| {
                            div()
                                .id(("design-version-ticket", n))
                                .flex_none()
                                .font_family(MONO)
                                .text_size(px(12.))
                                .text_color(p.text2)
                                .cursor_pointer()
                                .hover(move |s| s.text_color(p.accent).underline())
                                .child(format!("T-{id}"))
                                .on_click(cx.listener(move |this, _, _, cx| this.open_ticket(id, cx)))
                        }))
                        .child(div().min_w_0().truncate().text_size(px(SMALL)).text_color(p.text).child(subject)),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .flex_wrap()
                        .items_center()
                        .text_size(px(META))
                        .text_color(p.muted)
                        .child(cm::local_date(v.at))
                        .child(div().font_family(MONO).child(cm::short(v)))
                        .children(parts),
                ),
        )
        .child(side)
        .into_any_element()
}

// ── what it does ──

impl Shell {
    /// Compare and review begun (on After, in the UI map when there is a map, at
    /// the first changed screen) or ended.
    pub fn design_compare(&mut self, on: bool, cx: &mut Context<Self>) {
        if on {
            if !self.design.begin_compare() {
                return;
            }
            self.design.versions_open = false;
            self.design.changes_open = false;
            if self.design.current().is_some_and(|s| s.flows.is_some()) && self.design_tab() != Tab::Map {
                self.design_set_tab(Tab::Map, cx);
            }
            if self.design.map.screen.is_none() {
                self.design.step_changed(true);
            }
        } else {
            self.design.end_compare();
        }
        cx.notify();
    }

    /// Now or After.
    pub fn design_side(&mut self, side: Side, cx: &mut Context<Self>) {
        self.design.show_side(side);
        cx.notify();
    }

    /// ←/→ while comparing: the next changed screen, brought into view.
    pub fn design_step_changed(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.design.step_changed(forward) {
            let map = self.design.current().and_then(|s| s.flows.clone());
            if let Some(map) = map
                && let Some(flow) = self.design.map.flow(&map)
                && let Some(at) = flow.screens.iter().position(|s| Some(&s.id) == self.design.map.screen.as_ref())
            {
                self.design.map.strip.scroll_to_item(at * 2);
                let problems = usize::from(!fm::problems(&map).is_empty());
                self.design.map.body.scroll_to_item(problems + 2);
            }
        }
        cx.notify();
    }

    /// The draft's ticket.
    fn compared_ticket(&self) -> Option<equipe_core::protocol::Ticket> {
        let id = self.design.draft_of(&self.design.source)?.draft.as_ref()?.ticket;
        self.model.tickets.iter().find(|t| t.id == id).cloned()
    }

    /// Accept: the ticket accepted the board's way — landed the project's way
    /// when one is set, else accepted and its merge asked for after.
    pub fn design_accept_draft(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.compared_ticket() else {
            let id = self.design.draft_of(&self.design.source).and_then(|d| d.draft.as_ref()).map_or(0, |d| d.ticket);
            self.composer.flash = Some(t!("design.compare_no_ticket", id = id));
            cx.notify();
            return;
        };
        let way = if board::needs_way(&t) { self.merge_way_of(&self.project_of(&t)) } else { None };
        self.land(&t, way, cx);
        self.design.end_compare();
        cx.notify();
    }

    /// Ask again's box, opened (focused) or closed.
    pub fn design_redo(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.design.redo_open = open;
        if open {
            self.design_redo.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    /// The words sent: the ticket back to its member with them — about the
    /// screen open, when one is — as the board sends one back.
    pub fn design_redo_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let words = self.design_redo.read(cx).value().trim().to_owned();
        if words.is_empty() || !self.can_act() {
            return;
        }
        let Some(t) = self.compared_ticket() else { return };
        let about = (self.design_tab() == Tab::Map)
            .then(|| {
                let map = self.design.current()?.flows.as_ref()?;
                let (flow, screen) = (self.design.map.flow(map)?, self.design.map.screen(map)?);
                let state = self.design.map.chip(screen).map(|c| c.label).unwrap_or_default();
                Some(fm::context(flow, screen, &state))
            })
            .flatten();
        let note = match about {
            Some(about) => crate::quote::message(&about, &words),
            None => words,
        };
        self.hold(t.id, t!("board.toast_send_back", id = t.id), vec![Command::Reopen { squad: self.model.squad.clone(), ticket: t.id, note }], cx);
        self.design_redo.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        self.design.end_compare();
        cx.notify();
    }

    /// Version history opened — the daemon asked for the system on screen's — or closed.
    pub fn design_versions(&mut self, open: bool, cx: &mut Context<Self>) {
        self.design.versions_open = open;
        if open {
            self.design.changes_open = false;
            self.design_ask_versions();
        }
        cx.notify();
    }

    fn design_ask_versions(&mut self) {
        if self.model.squad.is_empty() {
            return;
        }
        self.design.versions_asked = Some(self.design.source.clone());
        self.send_command(Command::DesignVersions { squad: self.model.squad.clone(), project: self.design.source_project() });
    }

    /// The daemon's list: kept; asked again while it is open and the folder changed.
    pub fn design_versions_arrived(&mut self, squad: String, project: Option<String>, versions: Vec<DesignVersion>) -> bool {
        if squad != self.model.squad {
            return false;
        }
        self.design.versions_arrived(project, versions);
        true
    }

    /// The folder changed while the list is open: it is read again.
    pub fn design_versions_follow(&mut self) {
        if self.design.versions_open && self.design.versions_asked.is_none() {
            self.design_ask_versions();
        }
    }

    /// "Ask to go back to this": asked first, saying it becomes a change for review.
    pub fn design_restore_ask(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some(versions) = self.design.shown_versions() else { return };
        if !cm::can_restore(versions, at) {
            return;
        }
        let v = versions[at].clone();
        let subject = match v.ticket {
            Some(id) => format!("T-{id} {}", v.subject).trim().to_owned(),
            None => v.subject.clone(),
        };
        let command = Command::RestoreDesign { squad: self.model.squad.clone(), project: self.design.source_project(), commit: v.commit.clone() };
        self.confirm = Some(Ask {
            title: t!("design.versions_confirm_title", version = cm::short(&v)),
            body: vec![t!("design.versions_confirm_body", date = cm::local_date(v.at), subject = subject), t!("design.versions_confirm_review").to_owned()],
            yes: t!("design.versions_confirm_yes").to_owned(),
            then: Then::Send(vec![command]),
            option: None,
        });
        self.design.versions_open = false;
        cx.notify();
    }
}
