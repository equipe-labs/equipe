//! Orchestration (`crate::orchestra`): who is doing what, who waits on whom,
//! how many subagents each member runs and where work is stuck, at a glance.
//! Across the top, what waits on the person, counted by kind (to Needs you);
//! then what is stuck, each in a sentence; then a lane per member — its
//! ticket and for how long, its subagents as small bars, a chip for each
//! wait ("Waits on backend · T-74") that lights the lane it waits on under
//! the pointer and goes to it on a click, and who it holds up; its plan load
//! of the last day, to Usage. Idle members are one short row. From the
//! sidebar under Progress, ⌘⇧L, or `orchestra` in the snapshot script.

use std::time::{Duration, Instant};

use equipe_core::protocol::plan_use::Span;
use equipe_core::protocol::Engine;
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::orchestra::{self, Doing, End, ForYou, Helper, HelperState, Hop, Lane, Overview, Stuck, Wait, Why};
use crate::state::{Section, short_span};
use crate::theme::{MONO, Palette, R_LG, R_MD, SMALL};
use crate::tips::TipExt as _;
use crate::{Screen, Shell, ui};

/// The page's column, at its widest.
const COLUMN: f32 = 880.;
/// The parts above the lanes, each one child of the scrolled list: the
/// person's strip, what is stuck, the lanes' heading.
const ABOVE_LANES: usize = 3;
/// At most this many waits show in a lane; the rest as "+N".
const WAITS_SHOWN: usize = 3;

/// What the screen remembers between frames: the lane lit (under the
/// pointer, or picked by a click) and where the list is scrolled.
#[derive(Default)]
pub struct OrchestraUi {
    /// The lane a chip's click went to: lit until another is picked.
    pub picked: Option<String>,
    /// The lane a chip under the pointer points at.
    pub hover: Option<String>,
    pub scroll: ScrollHandle,
}

impl OrchestraUi {
    pub fn lit(&self) -> Option<&str> {
        self.hover.as_deref().or(self.picked.as_deref())
    }
}

/// A short word for what the person owes: inside a wait's way.
pub fn why_words(why: Why) -> &'static str {
    match why {
        Why::Permission => t!("orchestra.why_permission"),
        Why::Decision => t!("orchestra.why_decision"),
        Why::YourTurn => t!("orchestra.why_turn"),
        Why::Review => t!("orchestra.why_review"),
    }
}

/// A wait on the person, as its chip says it.
pub fn on_you_words(why: Why) -> &'static str {
    match why {
        Why::Permission => t!("orchestra.on_you_permission"),
        Why::Decision => t!("orchestra.on_you_decision"),
        Why::YourTurn => t!("orchestra.on_you_turn"),
        Why::Review => t!("orchestra.on_you_review"),
    }
}

fn hop_words(h: &Hop) -> String {
    match &h.member {
        Some(m) => format!("{m} · T-{}", h.ticket),
        None => format!("T-{} ({})", h.ticket, t!("orchestra.way_nobody")),
    }
}

/// A wait's way, start to end, in arrows: "frontend-2 → backend · T-74 →
/// you · your part". Arrows read the same in every language.
pub fn way_words(member: &str, w: &Wait) -> String {
    let mut parts: Vec<String> = vec![member.to_owned()];
    parts.extend(w.hops.iter().filter(|h| h.member.is_some()).map(hop_words));
    match w.end {
        End::Person(why) => parts.push(format!("{} · {}", t!("orchestra.way_you"), why_words(why))),
        End::Nobody => parts.extend(w.hops.last().map(hop_words)),
        End::NotStarted => {
            if let Some(last) = parts.last_mut() {
                last.push_str(&format!(" ({})", t!("orchestra.way_not_started")));
            }
        }
        End::Moving | End::Cycle => {}
    }
    parts.join(" → ")
}

/// A loop's way: each ticket and who holds it, back to the first.
pub fn loop_words(hops: &[Hop]) -> String {
    hops.iter().map(|h| h.member.as_ref().map_or_else(|| format!("T-{}", h.ticket), |m| format!("{m} · T-{}", h.ticket))).collect::<Vec<_>>().join(" → ")
}

/// Why work is stuck, in a sentence.
pub fn stuck_words(s: &Stuck) -> String {
    match s {
        Stuck::Cycle { hops } => t!("orchestra.stuck_cycle", way = loop_words(hops)),
        Stuck::Failed { member, ticket } => t!("orchestra.stuck_failed", member = member, ticket = format!("T-{ticket}")),
        Stuck::Chain { member, wait } => t!("orchestra.stuck_chain", way = way_words(member, wait)),
        Stuck::Nobody { member, ticket, on } => t!("orchestra.stuck_nobody", member = member, ticket = format!("T-{ticket}"), on = format!("T-{on}")),
        Stuck::Quiet { member, ticket, secs } => t!("orchestra.stuck_quiet", member = member, ticket = format!("T-{ticket}"), span = short_span(*secs)),
        Stuck::Stalled { member, ticket, secs } => t!("orchestra.stuck_stalled", member = member, ticket = format!("T-{ticket}"), span = short_span(*secs)),
        Stuck::Late { member, ticket, secs } => t!("orchestra.stuck_late", member = member, ticket = format!("T-{ticket}"), span = short_span(*secs)),
        Stuck::HelperQuiet { member, what, secs } => t!("orchestra.stuck_helper_quiet", member = member, what = what, span = short_span(*secs)),
        Stuck::HelperLong { member, what, secs } => t!("orchestra.stuck_helper_long", member = member, what = what, span = short_span(*secs)),
    }
}

/// A wait's chip: whom it waits on, in a few words.
pub fn chip_words(w: &Wait) -> String {
    if let Some(why) = w.on_person() {
        return on_you_words(why).to_owned();
    }
    match (w.end, w.hops.first()) {
        (End::Cycle, _) => t!("orchestra.waits_loop").to_owned(),
        (_, Some(Hop { member: Some(m), ticket })) => t!("orchestra.waits_on", member = m, ticket = format!("T-{ticket}")),
        (_, Some(Hop { member: None, ticket })) => t!("orchestra.waits_nobody", ticket = format!("T-{ticket}")),
        (_, None) => String::new(),
    }
}

pub fn doing_words(d: Doing) -> &'static str {
    match d {
        Doing::NeedsYou => t!("orchestra.doing_needs"),
        Doing::Working => t!("orchestra.doing_working"),
        Doing::Waiting => t!("orchestra.doing_waiting"),
        Doing::Idle => t!("orchestra.doing_idle"),
    }
}

/// A part of the person's strip, named.
pub fn part_words(section: Section) -> &'static str {
    match section {
        Section::Proposed | Section::NotOnMain => t!("orchestra.rest"),
        s => s.label(),
    }
}

/// What a lane says for the screen reader, in one go.
pub fn lane_label(l: &Lane) -> String {
    let mut out = vec![l.name.clone(), doing_words(l.doing).to_owned()];
    if let Some(c) = &l.current {
        out.push(format!("T-{} {}", c.id, c.title));
    }
    if l.helpers_running() > 0 {
        out.push(t!("orchestra.helpers", count = l.helpers_running()));
    }
    out.extend(l.waits.iter().map(chip_words));
    out.join(", ")
}

/// A subagent's bar: how much of [`orchestra::HELPER_LONG`] it has been at it (or took).
pub fn bar_fill(h: &Helper) -> f32 {
    (h.secs as f32 / orchestra::HELPER_LONG as f32).clamp(0.04, 1.)
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let o = orchestra::overview(&shell.model, Instant::now());
    let header = ui::title_bar(h_flex())
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pr_4()
        .pl(px(shell.header_inset(16., cx)))
        .gap_3()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(div().flex_none().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("orchestra.title")))
        .child(div().flex_1().min_w_0().truncate().text_size(px(SMALL)).text_color(p.muted).child(t!("orchestra.subtitle")));
    // Each part one child of the scrolled list, so a lane can be scrolled to.
    let place = |el: AnyElement, below: f32| div().flex_none().w_full().max_w(px(COLUMN)).px_6().pb(px(below)).child(el);
    let mut list = v_flex().id("orch-scroll").flex_1().min_h_0().w_full().items_center().pt_4().pb_10().overflow_y_scroll().track_scroll(&shell.board.orchestra.scroll);
    list = list
        .child(place(person_strip(shell, &o.for_you, cx), 20.))
        .child(place(stuck_part(shell, &o, cx), 20.))
        .child(place(heading(p, t!("orchestra.members"), o.lanes.len()).w_full().child(div().flex_1()).child(parallel_mode(shell)).into_any_element(), 6.));
    if o.lanes.is_empty() {
        list = list.child(place(div().py_6().text_size(px(13.)).text_color(p.muted).child(t!("orchestra.empty")).into_any_element(), 0.));
    }
    for (ix, l) in o.lanes.iter().enumerate() {
        list = list.child(place(lane_card(shell, ix, l, cx), 8.));
    }
    v_flex().id("orchestra-page").flex_1().min_h_0().bg(p.bg).child(header).child(list).into_any_element()
}

fn heading(p: Palette, label: &'static str, count: usize) -> Div {
    h_flex()
        .gap_1p5()
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.text2)
        .child(label)
        .child(div().font_weight(FontWeight::NORMAL).text_color(p.muted).child(count.to_string()))
}

/// How many run at once, beside the lanes' heading: the squad's parallel
/// work (Settings › AI platforms), max in the warning colour — it spends the
/// plan faster.
fn parallel_mode(shell: &Shell) -> AnyElement {
    let p = shell.palette;
    let parallel = shell.model.parallel;
    div()
        .flex_none()
        .text_size(px(12.))
        .font_weight(FontWeight::NORMAL)
        .text_color(if parallel.is_max() { p.orange } else { p.muted })
        .child(crate::switching::parallel_short(&parallel))
        .into_any_element()
}

/// What waits on the person, by kind: a dot and a quiet number each, the
/// whole strip leading to Needs you (one filled badge is the sidebar's).
fn person_strip(shell: &Shell, f: &ForYou, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let total = f.total();
    let parts = f.parts();
    let label = if total == 0 { t!("orchestra.for_you_none").to_owned() } else { format!("{}, {total}", t!("orchestra.for_you")) };
    let mut strip = h_flex()
        .id("orch-for-you")
        .w_full()
        .min_h(px(48.))
        .px_4()
        .py_2()
        .gap_x_4()
        .gap_y_1()
        .flex_wrap()
        .items_center()
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .role(Role::Button)
        .aria_label(label)
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .on_click(cx.listener(|this, _, _, cx| this.open_side_menu(crate::board::Menu::Inbox, cx)));
    if total == 0 {
        strip = strip.child(icon(Icon::Check, 14., p.muted)).child(div().text_size(px(SMALL)).text_color(p.text2).child(t!("orchestra.for_you_none")));
        return strip.into_any_element();
    }
    strip = strip.child(
        h_flex()
            .gap_2()
            .items_center()
            .child(super::board::dot(p.orange, 6.))
            .child(div().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text2).child(t!("orchestra.for_you")))
            .child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(total.to_string())),
    );
    for (section, n) in parts {
        strip = strip.child(
            h_flex()
                .gap_1p5()
                .items_baseline()
                .text_size(px(SMALL))
                .child(div().text_color(p.muted).child(part_words(section)))
                .child(div().text_color(p.text).font_weight(FontWeight::MEDIUM).child(n.to_string())),
        );
    }
    strip
        .child(div().flex_1())
        .child(h_flex().gap_1().items_center().text_size(px(12.)).text_color(p.accent).child(t!("orchestra.open_needs")).child(icon(Icon::ChevronRight, 12., p.accent)))
        .into_any_element()
}

/// What is stuck, the worst first, a sentence each: a click opens its
/// ticket, or lights its member's lane.
fn stuck_part(shell: &Shell, o: &Overview, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let mut part = v_flex().w_full().gap_1().child(heading(p, t!("orchestra.stuck_title"), o.stuck.len()));
    if o.stuck.is_empty() {
        return part
            .child(h_flex().py_1().gap_2().items_center().child(icon(Icon::Check, 13., p.muted)).child(div().text_size(px(SMALL)).text_color(p.muted).child(t!("orchestra.stuck_none"))))
            .into_any_element();
    }
    let rows = o.stuck.iter().enumerate().map(|(ix, s)| {
        let words = stuck_words(s);
        let ticket = s.ticket();
        let member = s.member().map(str::to_owned);
        // Red for what can't move at all, orange for what waits on the person, amber for slow.
        let tone = match s {
            Stuck::Cycle { .. } | Stuck::Failed { .. } | Stuck::Nobody { .. } => p.danger(),
            Stuck::Chain { wait, .. } if matches!(wait.end, End::Person(_)) => p.orange,
            _ => p.amber,
        };
        h_flex()
            .id(("orch-stuck", ix))
            .w_full()
            .min_h(px(32.))
            .px_2()
            .py_1()
            .gap_2()
            .items_start()
            .rounded(px(R_MD))
            .cursor_pointer()
            .hover(move |st| st.bg(p.wash))
            .role(Role::Button)
            .aria_label(words.clone())
            .child(div().pt(px(6.)).child(super::board::dot(tone, 7.)))
            .child(div().flex_1().min_w_0().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text).child(words))
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| match ticket {
                Some(id) if !this.open_elsewhere(e, super::elsewhere::Goal::Ticket(id), window, cx) => this.open_ticket_row(id, window, cx),
                Some(_) => {}
                None => this.orchestra_go(member.clone(), cx),
            }))
    });
    part = part.child(v_flex().w_full().p_1().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).children(rows));
    part.into_any_element()
}

/// The lane's mark: a spinner at work, the wait dot, or a quiet ring.
fn doing_mark(shell: &Shell, d: Doing, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    match d {
        Doing::Working => div().size(px(12.)).flex_none().flex().items_center().justify_center().child(super::sidebar::icon(IconName::LoaderCircle, 12., p.amber).rotate(radians(shell.spin(cx)))).into_any_element(),
        Doing::NeedsYou => super::board::dot(p.orange, 7.).into_any_element(),
        Doing::Waiting => icon(Icon::Clock, 12., p.text2).into_any_element(),
        Doing::Idle => div().size(px(7.)).flex_none().rounded_full().border_1().border_color(p.muted).into_any_element(),
    }
}

fn lane_card(shell: &Shell, ix: usize, l: &Lane, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let lit = shell.board.orchestra.lit() == Some(l.name.as_str());
    let session = l.session;
    let engine_line = [l.engine.map(|e| crate::switching::engine_name(e).to_owned()), l.model.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let load = l.engine.and_then(|e| orchestra::plan_load(shell.plan_use.reports.get(&(e, Span::Day)), &shell.model.squad, &l.name).map(|load| (e, load)));
    let name_row = h_flex()
        .w_full()
        .gap_x_2()
        .gap_y_1()
        .flex_wrap()
        .items_center()
        .child(
            h_flex()
                .id(("orch-who", ix))
                .min_w_0()
                .gap_2()
                .items_center()
                .cursor_pointer()
                .role(Role::Button)
                .aria_label(t!("orchestra.open_member", member = &l.name))
                .on_click(cx.listener(move |this, _, window, cx| this.select_member(session, window, cx)))
                .child(crate::theme::avatar(p, &l.role, &l.name, 18.))
                .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(l.name.clone()))
                .when(l.role != l.name, |r| r.child(div().text_size(px(12.)).text_color(p.muted).child(l.role.clone()))),
        )
        .when(!engine_line.is_empty(), |r| r.child(div().min_w_0().truncate().text_size(px(12.)).text_color(p.muted).child(engine_line)))
        .child(div().flex_1())
        .children(load.map(|(engine, (bp, week))| plan_figure(shell, ix, engine, bp, week, cx)))
        .child(h_flex().flex_none().gap_1p5().items_center().child(doing_mark(shell, l.doing, cx)).child(div().text_size(px(12.)).text_color(p.text2).child(doing_words(l.doing))));
    let mut card = v_flex()
        .id(("orch-lane", ix))
        .w_full()
        .gap_2()
        .px_4()
        .py(px(if l.is_quiet() { 8. } else { 12. }))
        .rounded(px(R_LG))
        .bg(if lit { p.accent_bg } else { p.raised })
        .border_1()
        .border_color(if lit { p.accent } else { p.hair })
        .role(Role::Group)
        .aria_label(lane_label(l))
        .child(name_row);
    if l.is_quiet() {
        return card.into_any_element();
    }
    match &l.current {
        Some(c) => {
            let id = c.id;
            let state = [Some(crate::state::status_word(c.status).to_owned()), c.for_secs.map(|s| t!("orchestra.for_span", span = short_span(s)))].into_iter().flatten().collect::<Vec<_>>().join(" · ");
            let step = shell.model.members.iter().find(|m| m.session == session).and_then(|m| crate::board::step_words(m.estimate.as_ref().and_then(|e| e.what.as_deref()), m.target.as_deref()));
            card = card.child(
                h_flex()
                    .id(("orch-ticket", ix))
                    .w_full()
                    .gap_x_3()
                    .gap_y(px(2.))
                    .flex_wrap()
                    .items_center()
                    .rounded(px(R_MD))
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.wash))
                    .role(Role::Button)
                    .aria_label(format!("T-{id} {}", c.title))
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        if !this.open_elsewhere(e, super::elsewhere::Goal::Ticket(id), window, cx) {
                            this.open_ticket_row(id, window, cx);
                        }
                    }))
                    .child(div().flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{id}")))
                    .child(div().flex_1().min_w(px(160.)).truncate().text_size(px(14.)).text_color(p.text).child(c.title.clone()))
                    .child(div().flex_none().text_size(px(12.)).text_color(p.text2).child(state))
                    // A design ticket: watched in the designer studio.
                    .when(shell.studio_offered(id), |row| {
                        row.child(
                            ui::button(p, ("orch-studio", ix), ui::Kind::Secondary, t!("studio.open"))
                                .icon(Icon::Eye)
                                .tip(t!("studio.open_tip"), None)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.open_studio(id, window, cx);
                                })),
                        )
                    }),
            );
            if let Some(step) = step.filter(|_| l.doing == Doing::Working) {
                card = card.child(div().w_full().truncate().text_size(px(12.)).text_color(p.muted).child(step));
            }
        }
        None => card = card.child(div().text_size(px(12.)).text_color(p.muted).child(t!("orchestra.no_ticket"))),
    }
    if !l.helpers.is_empty() {
        card = card.child(helpers_part(shell, ix, l, cx));
    }
    if !l.waits.is_empty() || !l.holds.is_empty() || l.queued > 0 {
        card = card.child(relations(shell, ix, l, cx));
    }
    card.into_any_element()
}

/// The member's load on its plan over the last day, to Usage.
fn plan_figure(shell: &Shell, ix: usize, engine: Engine, bp: u32, week: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let pct = crate::usage::percent(bp);
    let words = if week { t!("orchestra.plan_week", pct = &pct) } else { t!("orchestra.plan_look", pct = &pct) };
    h_flex()
        .id(("orch-plan", ix))
        .flex_none()
        .h(px(22.))
        .px_1p5()
        .gap_1()
        .items_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .role(Role::Link)
        .aria_label(format!("{words} — {}", t!("orchestra.plan_tip")))
        .tip(p, t!("orchestra.plan_tip"), None)
        .on_click(cx.listener(move |this, _, _, cx| this.orchestra_usage(engine, cx)))
        .child(icon(Icon::Chart, 12., p.muted))
        .child(div().text_size(px(12.)).text_color(p.text2).child(words))
        .into_any_element()
}

/// The member's subagents: how many at work, then a small bar each.
fn helpers_part(shell: &Shell, ix: usize, l: &Lane, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let member = l.session;
    let running = l.helpers_running();
    let mut part = v_flex().w_full().gap_1().child(
        div().text_size(px(12.)).text_color(p.text2).child(if running > 0 { t!("orchestra.helpers", count = running) } else { t!("orchestra.helpers_done").to_owned() }),
    );
    for (n, h) in l.helpers.iter().enumerate() {
        let (tone, mark, after): (Rgba, AnyElement, String) = match h.state {
            HelperState::Running if h.long() => (p.danger(), icon(Icon::Clock, 11., p.danger()).into_any_element(), shell.counting(Duration::from_secs(h.secs), cx)),
            HelperState::Running => (p.amber, super::sidebar::icon(IconName::LoaderCircle, 11., p.muted).rotate(radians(shell.spin(cx))).into_any_element(), shell.counting(Duration::from_secs(h.secs), cx)),
            HelperState::Quiet(q) => (p.danger(), icon(Icon::Warning, 11., p.danger()).into_any_element(), t!("orchestra.helper_quiet", span = short_span(q))),
            HelperState::Finished { failed: true } => (p.danger(), icon(Icon::XCircle, 11., p.danger()).into_any_element(), t!("orchestra.helper_failed", span = short_span(h.secs))),
            HelperState::Finished { failed: false } => (p.muted, icon(Icon::Check, 11., p.muted).into_any_element(), t!("orchestra.helper_done", span = short_span(h.secs))),
        };
        let fill = bar_fill(h);
        let (session, id) = (h.session, h.id.clone());
        let open = h.running();
        part = part.child(
            h_flex()
                .id(("orch-helper", ix * 100 + n))
                .w_full()
                .h(px(22.))
                .gap_2()
                .items_center()
                .rounded(px(6.))
                .when(open, |r| r.cursor_pointer().hover(move |s| s.bg(p.wash)).on_click(cx.listener(move |this, _, window, cx| this.open_subagent(member, session, &id, window, cx))))
                .role(Role::ListItem)
                .aria_label(format!("{} · {after}", h.what))
                .child(div().w(px(12.)).flex_none().flex().justify_center().child(mark))
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(if open { p.text } else { p.muted }).child(h.what.clone()))
                .child(
                    div()
                        .w(px(96.))
                        .h(px(4.))
                        .flex_none()
                        .rounded_full()
                        .bg(p.wash)
                        .child(div().h_full().w(relative(fill)).rounded_full().bg(tone)),
                )
                .child(div().w(px(84.)).flex_none().truncate().text_size(px(11.)).text_color(p.muted).child(after)),
        );
    }
    part.into_any_element()
}

/// Its waits as chips (each lighting, and going to, the lane it waits on),
/// then who waits on it, and its other tickets.
fn relations(shell: &Shell, ix: usize, l: &Lane, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let mut row = h_flex().w_full().gap_2().flex_wrap().items_center();
    for (n, w) in l.waits.iter().take(WAITS_SHOWN).enumerate() {
        let target = w.on_member().filter(|m| *m != l.name).map(str::to_owned);
        let person = w.on_person().is_some() || matches!(w.end, End::Person(_));
        let dot = match w.end {
            End::Cycle | End::Nobody => p.danger(),
            _ if person => p.orange,
            _ => p.muted,
        };
        let words = chip_words(w);
        let own = w.ticket.filter(|t| l.current.as_ref().is_none_or(|c| c.id != *t));
        let label = match own {
            Some(t) => format!("T-{t} · {words}"),
            None => words.clone(),
        };
        let chip = h_flex()
            .id(("orch-wait", ix * 100 + n))
            .h(px(24.))
            .px_2()
            .gap_1p5()
            .items_center()
            .rounded(px(12.))
            .border_1()
            .border_color(p.hair)
            .bg(p.bg)
            .text_size(px(12.))
            .child(super::board::dot(dot, 6.))
            .children(own.map(|t| div().font_family(MONO).text_color(p.muted).child(format!("T-{t}"))))
            .child(div().text_color(p.text2).child(words));
        let chip = match (target, w.on_person().is_some()) {
            (Some(m), _) => {
                let on = m.clone();
                chip.cursor_pointer()
                    .hover(move |s| s.bg(p.wash))
                    .role(Role::Button)
                    .aria_label(format!("{label} — {}", t!("orchestra.show_lane", member = &m)))
                    // No tip: a tip takes the element's one hover, which lights the lane instead.
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        let want = hovered.then(|| on.clone());
                        if this.board.orchestra.hover != want {
                            this.board.orchestra.hover = want;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.orchestra_go(Some(m.clone()), cx)))
                    .into_any_element()
            }
            // A wait on the person goes where they answer it.
            (None, true) => chip
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash))
                .role(Role::Button)
                .aria_label(format!("{label} — {}", t!("orchestra.open_needs")))
                .on_click(cx.listener(|this, _, _, cx| this.open_side_menu(crate::board::Menu::Inbox, cx)))
                .into_any_element(),
            (None, false) => chip.aria_label(label).into_any_element(),
        };
        row = row.child(chip);
    }
    if l.waits.len() > WAITS_SHOWN {
        row = row.child(div().text_size(px(12.)).text_color(p.muted).child(format!("+{}", l.waits.len() - WAITS_SHOWN)));
    }
    for (n, (who, ticket)) in l.holds.iter().enumerate() {
        let m = who.clone();
        let on = who.clone();
        let words = t!("orchestra.holds", member = who, ticket = format!("T-{ticket}"));
        row = row.child(
            h_flex()
                .id(("orch-holds", ix * 100 + n))
                .h(px(24.))
                .px_2()
                .gap_1p5()
                .items_center()
                .rounded(px(12.))
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash))
                .text_size(px(12.))
                .text_color(p.muted)
                .role(Role::Button)
                .aria_label(format!("{words} — {}", t!("orchestra.show_lane", member = who)))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    let want = hovered.then(|| on.clone());
                    if this.board.orchestra.hover != want {
                        this.board.orchestra.hover = want;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| this.orchestra_go(Some(m.clone()), cx)))
                .child(icon(Icon::ArrowLeft, 11., p.muted))
                .child(words),
        );
    }
    if l.queued > 0 {
        row = row.child(div().text_size(px(12.)).text_color(p.muted).child(t!("orchestra.queued", count = l.queued)));
    }
    row.into_any_element()
}

impl Shell {
    /// Orchestration, with a fresh look at each engine's last day of plan use for the lanes' figures.
    pub fn open_orchestra(&mut self, cx: &mut Context<Self>) {
        let mut engines: Vec<Engine> = self.model.members.iter().filter_map(|m| m.engine).collect();
        engines.sort_by_key(|e| crate::switching::engine_name(*e));
        engines.dedup();
        let asks: Vec<_> = engines.into_iter().flat_map(|e| self.plan_use.ask_for(e, Span::Day)).collect();
        self.plan_use_send(asks, cx);
        self.show(Screen::Orchestration, cx);
        cx.notify();
    }

    /// A chip pressed, or something stuck about no one ticket: light that
    /// member's lane and bring it into sight.
    pub fn orchestra_go(&mut self, member: Option<String>, cx: &mut Context<Self>) {
        let ui = &mut self.board.orchestra;
        ui.picked = if ui.picked == member { None } else { member.clone() };
        ui.hover = None;
        if let Some(name) = member {
            let lanes = orchestra::overview(&self.model, Instant::now()).lanes;
            if let Some(at) = lanes.iter().position(|l| l.name == name) {
                self.board.orchestra.scroll.scroll_to_item(ABOVE_LANES + at);
            }
        }
        cx.notify();
    }

    /// A lane's plan figure: Usage, at that engine's last day, by member.
    pub fn orchestra_usage(&mut self, engine: Engine, cx: &mut Context<Self>) {
        let mut asks = self.plan_use.pick_engine(engine);
        asks.extend(self.plan_use.pick_span(Span::Day));
        self.plan_use.cut = crate::usage::Cut::Members;
        self.plan_use_send(asks, cx);
        self.open_usage(cx);
    }

    /// The snapshot tool's `orchestra[:<member>]` (`snapshot.rs`): the screen, a member's lane lit.
    pub fn snapshot_orchestra(&mut self, what: &str, cx: &mut Context<Self>) {
        self.open_orchestra(cx);
        if !what.is_empty() {
            self.orchestra_go(Some(what.to_owned()), cx);
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's own `test` would stand in for the standard one.
    use super::{bar_fill, chip_words, doing_words, lane_label, loop_words, on_you_words, part_words, stuck_words, way_words};
    use crate::orchestra::{Current, Doing, End, Helper, HelperState, Hop, Lane, Stuck, Wait, Why};
    use crate::state::Section;
    use equipe_core::protocol::TicketStatus;

    fn hop(ticket: u32, member: Option<&str>) -> Hop {
        Hop { ticket, member: member.map(str::to_owned) }
    }

    #[test]
    fn a_waits_chip_names_whom_it_waits_on() {
        assert_eq!(chip_words(&Wait { ticket: Some(90), hops: vec![hop(74, Some("backend"))], end: End::Moving }), "backend · T-74 기다리는 중");
        assert_eq!(chip_words(&Wait { ticket: Some(55), hops: vec![], end: End::Person(Why::YourTurn) }), on_you_words(Why::YourTurn));
        assert_eq!(on_you_words(Why::Review), "내 검토를 기다리는 중");
        assert_eq!(chip_words(&Wait { ticket: Some(45), hops: vec![hop(12, None)], end: End::Nobody }), "T-12 기다리는 중 · 맡은 멤버 없음");
        assert_eq!(chip_words(&Wait { ticket: Some(91), hops: vec![hop(92, Some("backend")), hop(91, Some("frontend"))], end: End::Cycle }), "서로 기다리는 중");
    }

    /// A chain reads start to end in arrows, the person last when it ends on them.
    #[test]
    fn a_chain_reads_in_arrows_to_the_person() {
        let w = Wait { ticket: Some(90), hops: vec![hop(74, Some("backend")), hop(73, Some("backend"))], end: End::Person(Why::YourTurn) };
        assert_eq!(way_words("frontend-2", &w), "frontend-2 → backend · T-74 → backend · T-73 → 나 · 내 할 일");
        assert_eq!(stuck_words(&Stuck::Chain { member: "frontend-2".into(), wait: w }), "기다림이 이어져 있어요: frontend-2 → backend · T-74 → backend · T-73 → 나 · 내 할 일");
        let nobody = Wait { ticket: Some(45), hops: vec![hop(12, Some("backend")), hop(9, None)], end: End::Nobody };
        assert_eq!(way_words("frontend", &nobody), "frontend → backend · T-12 → T-9 (맡은 멤버 없음)");
        let later = Wait { ticket: Some(45), hops: vec![hop(12, Some("backend")), hop(9, Some("data"))], end: End::NotStarted };
        assert_eq!(way_words("frontend", &later), "frontend → backend · T-12 → data · T-9 (시작 전)");
        assert_eq!(loop_words(&[hop(91, Some("frontend")), hop(92, Some("backend")), hop(91, Some("frontend"))]), "frontend · T-91 → backend · T-92 → frontend · T-91");
    }

    #[test]
    fn stuck_work_is_said_in_plain_words() {
        assert_eq!(stuck_words(&Stuck::Stalled { member: "designer".into(), ticket: 60, secs: 3 * 3600 }), "T-60: 3시간째 진행이 없고 designer 멤버는 지금 쉬고 있어요");
        assert_eq!(stuck_words(&Stuck::HelperLong { member: "backend".into(), what: "Run the tests".into(), secs: 25 * 60 }), "backend 멤버의 서브에이전트가 25분째 일하고 있어요: Run the tests");
        assert_eq!(stuck_words(&Stuck::Nobody { member: "frontend".into(), ticket: 45, on: 12 }), "frontend · T-45 → T-12: 맡은 멤버가 없어요");
        assert!(stuck_words(&Stuck::Cycle { hops: vec![hop(1, Some("a")), hop(2, Some("b")), hop(1, Some("a"))] }).contains("a · T-1 → b · T-2 → a · T-1"));
    }

    /// The person's strip names each kind as Needs you does; proposals and
    /// work not on main yet go together.
    #[test]
    fn the_strip_names_each_kind() {
        assert_eq!(part_words(Section::Permission), Section::Permission.label());
        assert_eq!(part_words(Section::Proposed), part_words(Section::NotOnMain));
        assert_eq!(doing_words(Doing::Waiting), "기다리는 중");
    }

    #[test]
    fn a_lane_reads_as_one_line_and_a_bar_grows_to_its_limit() {
        let helper = |secs: u64, state: HelperState| Helper { session: 3, id: "a".into(), what: "Read the logs".into(), secs, state };
        let lane = Lane {
            session: 3,
            name: "backend".into(),
            role: "backend".into(),
            engine: None,
            model: None,
            doing: Doing::Working,
            current: Some(Current { id: 70, title: "Orders API pagination".into(), status: TicketStatus::InProgress, for_secs: Some(600) }),
            helpers: vec![helper(60, HelperState::Running), helper(30, HelperState::Finished { failed: false })],
            waits: vec![Wait { ticket: Some(73), hops: vec![], end: End::Person(Why::YourTurn) }],
            holds: vec![],
            queued: 0,
        };
        assert_eq!(lane_label(&lane), "backend, 작업 중, T-70 Orders API pagination, 서브에이전트 1개 작업 중, 내 차례를 기다리는 중");
        assert!(bar_fill(&helper(0, HelperState::Running)) > 0., "a sliver even at the start");
        assert_eq!(bar_fill(&helper(10 * 60, HelperState::Running)), 0.5);
        assert_eq!(bar_fill(&helper(3 * 3600, HelperState::Running)), 1.);
    }
}
