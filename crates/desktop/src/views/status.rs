//! Status: is the squad's work going well, at a glance, without reading logs.
//! From the top: the status line (a dot — green all good, amber going but
//! slowed, red stopped — one sentence on what slows or stops it, a muted line
//! of engine notes, and when it was worked out); four numbers; what is
//! blocked, one row per problem with its plain cause and what to do; the
//! engines' budgets beside the token-waste signals; and the tickets the
//! person watches.
//!
//! The daemon works the figures out (`squad_status`); the words are
//! [`crate::status`]'s. This only draws them, and polls (below).

use equipe_core::protocol::lanes::{MOST_PER_ENGINE, MOST_PER_MEMBER, Parallel, ParallelMode};
use equipe_core::protocol::status::{Block, EngineBudget, Goto, Handling, Mood, Severity, SquadStatus, Watched};
use equipe_core::protocol::{Command, Engine, TicketId};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::state::{Conn, short_span};
use crate::status::{self as words, Support, Tile};
use crate::theme::{ICON_SM, MONO, Palette, R_LG, R_MD, SMALL};
use crate::tips::TipExt as _;
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

/// The page's column, at its widest.
const COLUMN: f32 = 920.;
/// The least width of a card that sits beside another.
const CARD_MIN: f32 = 330.;

/// The dot's color for a mood.
pub fn mood_color(p: Palette, mood: Mood) -> Rgba {
    match mood {
        Mood::Good => p.good(),
        Mood::Slowed => p.amber,
        Mood::Stopped => p.danger(),
    }
}

fn severity_color(p: Palette, severity: Severity) -> Rgba {
    match severity {
        Severity::Red => p.danger(),
        Severity::Amber => p.amber,
        Severity::Info => p.muted,
    }
}

fn dot(color: Rgba, size: f32) -> Div {
    div().size(px(size)).flex_none().rounded_full().bg(color)
}

/// What the page shows: the figures, or why there are none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// Asked, no answer yet.
    Loading,
    /// The daemon predates the request.
    Unsupported,
    /// The last look failed, and why; nothing known from before.
    Failed(String),
    /// The squad's figures.
    Ready,
}

/// Which body `ui` calls for in `squad`'s window.
pub fn body_of(ui: &crate::status::StatusUi, squad: &str) -> Body {
    match (ui.of(squad), ui.support, &ui.failed) {
        (Some(_), _, _) => Body::Ready,
        (None, Support::No, _) => Body::Unsupported,
        (None, _, Some(why)) => Body::Failed(why.clone()),
        _ => Body::Loading,
    }
}

/// The status line, as it reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub mood: Mood,
    pub headline: String,
    pub detail: String,
    pub updated: String,
}

pub fn line_of(s: &SquadStatus, now: u64) -> Line {
    Line { mood: s.mood, headline: words::headline(s), detail: words::detail(s, now), updated: words::updated(s.at) }
}

/// A row of "What's blocked", as it reads: its cause, and its button or who handles it.
#[derive(Debug, Clone, PartialEq)]
pub struct RowModel {
    pub severity: Severity,
    pub text: String,
    pub label: &'static str,
    /// Where the button leads; `None`: the label only says who handles it.
    pub goto: Option<Goto>,
}

pub fn row_of(b: &Block, now: u64) -> RowModel {
    let goto = match b.handling {
        Handling::You { .. } => b.goto,
        _ => None,
    };
    RowModel { severity: b.severity, text: words::cause_text(b, now), label: words::handling_word(b.handling), goto }
}

/// A watched ticket's row, as it reads: its status word, and how long it has been so.
#[derive(Debug, Clone, PartialEq)]
pub struct WatchRow {
    pub ticket: TicketId,
    pub title: String,
    pub owner: Option<String>,
    pub status: String,
    pub paused: bool,
    pub how_long: Option<String>,
}

pub fn watch_row_of(w: &Watched, now: u64) -> WatchRow {
    WatchRow {
        ticket: w.ticket,
        title: w.title.clone(),
        owner: w.owner.clone(),
        status: if w.paused.is_some() { t!("run.paused").to_owned() } else { crate::state::status_word(w.status).to_owned() },
        paused: w.paused.is_some(),
        how_long: w.since.filter(|since| *since > 0 && *since <= now).map(|since| short_span(now - since)),
    }
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let header = ui::title_bar(h_flex())
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pr_3()
        .pl(px(shell.header_inset(16., cx)))
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("status.title")))
        .child(div().flex_1())
        .children(shell.status.of(&shell.model.squad).map(|s| div().text_size(px(12.)).text_color(p.muted).child(words::updated(s.at))));
    let body = match (body_of(&shell.status, &shell.model.squad), shell.status.of(&shell.model.squad)) {
        (Body::Ready, Some(status)) => page(shell, status, cx),
        (body, _) => note_page(shell, &body, cx),
    };
    v_flex()
        .id("status-page")
        .flex_1()
        .min_h_0()
        .bg(p.bg)
        .child(header)
        .child(div().id("status-scroll").flex_1().min_h_0().overflow_y_scroll().child(h_flex().w_full().justify_center().child(v_flex().w_full().max_w(px(COLUMN)).px_6().pt_4().pb_10().child(body))))
        .into_any_element()
}

/// Nothing to show yet: being asked, or not answered — with a way to ask again.
fn note_page(shell: &Shell, body: &Body, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let (title, body) = match body {
        Body::Unsupported => (t!("status.unsupported_title"), t!("status.unsupported").to_owned()),
        Body::Failed(why) => (t!("status.failed_title"), why.clone()),
        _ => (t!("status.loading_title"), t!("status.loading").to_owned()),
    };
    v_flex()
        .w_full()
        .py_10()
        .gap_1p5()
        .items_center()
        .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(title))
        .child(div().max_w(px(460.)).text_center().text_size(px(13.)).text_color(p.muted).child(body))
        .when(shell.status.failed.is_some(), |c| c.child(ui::button(p, "status-retry", Kind::Secondary, t!("status.retry")).on_click(cx.listener(|this, _, _, cx| this.status_ask(cx)))))
        .into_any_element()
}

fn page(shell: &Shell, s: &SquadStatus, cx: &mut Context<Shell>) -> AnyElement {
    let now = shell.model.unix;
    v_flex()
        .w_full()
        .gap_4()
        .child(status_line(shell, s, now))
        .child(tiles(shell, s, cx))
        .child(blocked(shell, s, now, cx))
        .children(speed_card(shell, s, now, cx))
        .children((!s.waiting.is_empty()).then(|| waiting(shell, s, cx)))
        .child(h_flex().w_full().flex_wrap().items_start().gap_4().child(budget(shell, s, now)).child(waste(shell, s)))
        .child(watched(shell, s, now, cx))
        .into_any_element()
}

/// A card: its heading, a hairline, its parts.
fn card(p: Palette, id: &'static str, heading: String, note: Option<String>) -> Stateful<Div> {
    v_flex()
        .id(id)
        .w_full()
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .child(
            h_flex()
                .px_3()
                .h(px(36.))
                .gap_2()
                .items_center()
                .border_b_1()
                .border_color(p.hair)
                .child(div().flex_none().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(heading))
                .children(note.map(|n| div().min_w_0().truncate().text_size(px(12.)).text_color(p.muted).child(n))),
        )
}

fn status_line(shell: &Shell, s: &SquadStatus, now: u64) -> AnyElement {
    let p = shell.palette;
    let Line { mood, headline, detail, updated } = line_of(s, now);
    let color = mood_color(p, mood);
    h_flex()
        .id("status-line")
        .w_full()
        .gap_3()
        .px_4()
        .py_3()
        .items_center()
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .role(Role::Status)
        .aria_label(format!("{}: {headline}", words::mood_word(mood)))
        .child(dot(color, 14.))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(headline))
                .when(!detail.is_empty(), |c| c.child(div().text_size(px(13.)).text_color(p.muted).child(detail))),
        )
        .child(div().flex_none().text_size(px(12.)).text_color(p.muted).child(updated))
        .into_any_element()
}

fn tiles(shell: &Shell, s: &SquadStatus, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let t = &s.tiles;
    let tile = |id: &'static str, n: u32, label: &'static str, kind: Tile, hot: Option<Rgba>| {
        v_flex()
            .id(id)
            .flex_1()
            .min_w(px(150.))
            .px_3()
            .py_2()
            .gap(px(2.))
            .rounded(px(R_MD))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .child(div().text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(hot.filter(|_| n > 0).unwrap_or(p.text)).child(n.to_string()))
            .child(div().text_size(px(12.)).text_color(p.text2).child(label))
            .child(div().text_size(px(11.5)).text_color(p.muted).child(words::tile_sub(s, kind)))
    };
    h_flex()
        .w_full()
        .flex_wrap()
        .gap_2()
        .child(tile("status-accepted", t.accepted_today, t!("status.tile_accepted"), Tile::Accepted, None))
        .child(tile("status-progress", t.in_progress, t!("status.tile_progress"), Tile::InProgress, None))
        .child(
            tile("status-needs", t.needs_you, t!("status.tile_needs"), Tile::NeedsYou, Some(p.orange))
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash))
                .on_click(cx.listener(|this, _, _, cx| this.open_side_menu(crate::board::Menu::Inbox, cx))),
        )
        .child(tile("status-waiting", t.waiting, t!("status.tile_waiting"), Tile::Waiting, None))
        .into_any_element()
}

fn blocked(shell: &Shell, s: &SquadStatus, now: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let mut card = card(p, "status-blocked", t!("status.blocked").to_owned(), None);
    if s.blocked.is_empty() {
        return card.child(div().px_3().py_3().text_size(px(13.)).text_color(p.muted).child(t!("status.blocked_none"))).into_any_element();
    }
    for (ix, b) in s.blocked.iter().enumerate() {
        card = card.child(blocked_row(shell, b, ix, now, cx));
    }
    if s.blocked_more > 0 {
        card = card.child(div().px_3().py_2().text_size(px(12.)).text_color(p.muted).child(t!("status.more", count = s.blocked_more)));
    }
    card.into_any_element()
}

fn blocked_row(shell: &Shell, b: &Block, ix: usize, now: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let RowModel { severity, text, mut label, mut goto } = row_of(b, now);
    // The Mac in front of the person is the daemon's: nothing to open, they unlock it where they sit.
    if goto == Some(Goto::UnlockMac) && !shell.endpoint.is_remote() {
        goto = None;
        label = t!("status.act_unlock_here");
    }
    let action: AnyElement = match goto {
        Some(goto) => ui::button(p, SharedString::from(format!("status-act-{ix}")), Kind::Secondary, label).on_click(cx.listener(move |this, _, window, cx| this.status_go(goto, window, cx))).into_any_element(),
        None => div().flex_none().text_size(px(12.)).text_color(p.muted).child(label).into_any_element(),
    };
    h_flex()
        .id(("status-block", ix))
        .w_full()
        .min_h(px(40.))
        .px_3()
        .py_1()
        .gap_3()
        .items_center()
        .when(ix > 0, |r| r.border_t_1().border_color(p.hair))
        .child(dot(severity_color(p, severity), 8.).id(("status-sev", ix)).tip(p, words::severity_word(severity), None))
        .child(div().flex_1().min_w_0().text_size(px(SMALL)).line_height(px(19.)).text_color(p.text).child(text))
        .child(action)
        .into_any_element()
}

/// One engine's weekly bar, how used, and how far it lasts.
fn engine_row(p: Palette, e: &EngineBudget, now: u64, holding: bool) -> AnyElement {
    let used = e.weekly.map(|w| w.used_percent.min(100));
    let color = if e.out_until.is_some() { p.danger() } else if used.is_some_and(|u| u >= 85) { p.orange } else if used.is_some_and(|u| u >= 70) { p.amber } else { p.good() };
    let bar = div().h(px(6.)).w_full().rounded_full().bg(p.hair).child(div().h_full().w(relative(f32::from(used.unwrap_or(0)) / 100.)).rounded_full().bg(color));
    let name = crate::switching::engine_name(e.engine);
    v_flex()
        .id(match e.engine {
            Engine::Claude => "status-engine-claude",
            Engine::Codex => "status-engine-codex",
        })
        .w_full()
        .gap(px(4.))
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_baseline()
                .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(name))
                .children(e.plan_type.clone().map(|t| div().text_size(px(11.5)).text_color(p.muted).child(t)))
                .child(div().flex_1())
                .child(div().text_size(px(13.)).font_family(MONO).text_color(if used.is_some() { p.text } else { p.muted }).child(used.map_or_else(|| "—".to_owned(), |u| t!("status.used", percent = u)))),
        )
        .child(bar)
        .child(div().text_size(px(12.)).text_color(p.text2).child(words::budget_line(e, now)))
        // Amber only when running fewer at once holds a ready ticket back.
        .children(e.pacing_at_once.map(|n| div().text_size(px(12.)).text_color(if holding { p.amber } else { p.muted }).child(t!("status.budget_pacing", count = n))))
        .children(e.stale.then(|| div().text_size(px(11.5)).text_color(p.muted).child(t!("status.budget_stale"))))
        .into_any_element()
}

fn budget(shell: &Shell, s: &SquadStatus, now: u64) -> AnyElement {
    let p = shell.palette;
    let mut body = v_flex().px_3().py_3().gap_3();
    if s.engines.is_empty() {
        body = body.child(div().text_size(px(13.)).text_color(p.muted).child(t!("status.budget_empty")));
    }
    let holding = s.flow.as_ref().is_some_and(|f| f.held_by_cap > 0);
    for e in &s.engines {
        body = body.child(engine_row(p, e, now, holding));
    }
    div().flex_1().min_w(px(CARD_MIN)).child(card(p, "status-budget", t!("status.budget").to_owned(), None).child(body)).into_any_element()
}

/// One side of a stepper: an icon that sets the number to `to`.
fn step_button(p: Palette, id: SharedString, glyph: Icon, to: u32, enabled: bool, set: impl Fn(&mut Shell, u32, &mut Context<Shell>) + 'static, cx: &mut Context<Shell>) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h_full()
        .w(px(24.))
        .justify_center()
        .items_center()
        .rounded(px(R_MD))
        .when(!enabled, |s| s.opacity(0.4))
        .when(enabled, |s| s.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| set(this, to, cx))))
        .child(icon(glyph, ICON_SM, p.text2))
}

/// A small number with − and + either side, as the zoom control's pill.
fn stepper(shell: &Shell, id: &'static str, label: &'static str, value: u32, least: u32, most: u32, step: u32, set: impl Fn(&mut Shell, u32, &mut Context<Shell>) + Clone + 'static, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let live = matches!(shell.model.conn, Conn::Connected { .. });
    let less = step_button(p, SharedString::from(format!("{id}-less")), Icon::Minus, value.saturating_sub(step).max(least), live && value > least, set.clone(), cx);
    let more = step_button(p, SharedString::from(format!("{id}-more")), Icon::Plus, (value + step).min(most), live && value < most, set, cx);
    h_flex()
        .id(id)
        .flex_none()
        .gap_2()
        .items_center()
        .child(div().text_size(px(12.)).text_color(p.text2).child(label))
        .child(
            h_flex()
                .h(px(26.))
                .p(px(2.))
                .gap(px(2.))
                .items_center()
                .rounded(px(R_MD))
                .bg(if p.is_dark() { p.wash } else { p.code_bg })
                .child(less)
                .child(div().min_w(px(26.)).text_center().font_family(MONO).text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(value.to_string()))
                .child(more),
        )
        .into_any_element()
}

/// The Speed card: how many tickets run at once, what each speed does to the
/// budget, and why the work isn't faster than it is.
fn speed_card(shell: &Shell, s: &SquadStatus, now: u64, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if s.speed.is_none() && s.flow.is_none() {
        return None;
    }
    let p = shell.palette;
    let connected = matches!(shell.model.conn, Conn::Connected { .. });
    let parallel = shell.model.parallel;
    let mut body = v_flex().px_3().py_3().gap_3();

    // The control, and one line on what each speed means.
    let control = if connected {
        super::settings::segmented(shell, "status-speed", words::speed_modes(), parallel.mode, |shell, mode, _, cx| shell.set_speed(Parallel { mode, ..shell.model.parallel }, cx), cx)
    } else {
        div().text_size(px(13.)).text_color(p.text2).child(crate::switching::parallel_word(parallel.mode)).into_any_element()
    };
    let about = |id: &'static str, mode: ParallelMode, name: String| {
        let on = parallel.mode == mode;
        h_flex()
            .id(id)
            .gap_2()
            .items_baseline()
            .child(div().flex_none().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(if on { p.text } else { p.text2 }).child(name))
            .child(div().min_w_0().text_size(px(12.5)).text_color(if on { p.text2 } else { p.muted }).child(words::speed_about(mode)))
    };
    let set_member = |shell: &mut Shell, n: u32, cx: &mut Context<Shell>| shell.set_speed(Parallel { per_member: n, ..shell.model.parallel }, cx);
    let set_engine = |shell: &mut Shell, n: u32, cx: &mut Context<Shell>| shell.set_speed(Parallel { per_engine: n, ..shell.model.parallel }, cx);
    body = body
        .child(h_flex().w_full().items_center().child(control))
        .child(about("status-speed-full", ParallelMode::Max, t!("status.speed_full").to_owned()))
        .child(
            h_flex()
                .id("status-speed-caps")
                .gap_4()
                .pl_3()
                .flex_wrap()
                .opacity(if parallel.is_max() { 1. } else { 0.7 })
                .child(stepper(shell, "status-speed-member", t!("status.speed_per_member"), parallel.per_member, 1, MOST_PER_MEMBER, 1, set_member, cx))
                .child(stepper(shell, "status-speed-engine", t!("status.speed_per_engine"), parallel.per_engine, 1, MOST_PER_ENGINE, 2, set_engine, cx)),
        )
        .child(about("status-speed-balanced", ParallelMode::Auto, t!("status.speed_balanced").to_owned()));
    for (ix, line) in words::speed_lines(s, now).into_iter().enumerate() {
        body = body.child(div().id(("status-speed-outlook", ix)).text_size(px(12.5)).text_color(p.text).child(line));
    }

    // Why not faster?
    if let Some(flow) = &s.flow {
        body = body.child(div().h(px(1.)).w_full().bg(p.hair)).child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(t!("status.why")));
        if let Some(why) = words::why_of(s, &parallel) {
            let button: Option<AnyElement> = why.act.map(|act| match act {
                words::WhyAct::Answer => ui::button(p, "status-why-act", Kind::Secondary, t!("status.act_answer")).on_click(cx.listener(|this, _, window, cx| this.status_go(Goto::NeedsYou, window, cx))).into_any_element(),
                words::WhyAct::FullSpeed => ui::button(p, "status-why-act", Kind::Secondary, t!("status.speed_full")).disabled(!connected).on_click(cx.listener(|this, _, _, cx| this.set_speed(Parallel { mode: ParallelMode::Max, ..this.model.parallel }, cx))).into_any_element(),
            });
            body = body.child(
                h_flex()
                    .id("status-why")
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(why.text))
                            .children(why.sub.map(|sub| div().text_size(px(12.)).text_color(p.muted).child(sub))),
                    )
                    .children(button),
            );
        }
        let mut rows = v_flex().w_full().gap(px(2.));
        for (ix, (label, n)) in words::flow_rows(flow).into_iter().enumerate() {
            rows = rows.child(
                h_flex()
                    .id(("status-flow", ix))
                    .w_full()
                    .min_h(px(24.))
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().min_w_0().text_size(px(12.5)).text_color(p.text2).child(label))
                    .child(div().flex_none().font_family(MONO).text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(if n > 0 { p.text } else { p.muted }).child(n.to_string())),
            );
        }
        body = body.child(rows);
    }
    Some(card(p, "status-speed-card", t!("status.speed").to_owned(), None).child(body).into_any_element())
}

fn waste(shell: &Shell, s: &SquadStatus) -> AnyElement {
    let p = shell.palette;
    let w = &s.waste;
    let mut body = v_flex().px_3().py_2().gap(px(2.));
    if w.tokens_day == 0 && w.metrics.iter().all(|m| m.day.is_none()) {
        body = body.child(div().py_1().text_size(px(13.)).text_color(p.muted).child(t!("status.waste_none")));
    } else {
        for m in &w.metrics {
            let quality = m.key == equipe_core::protocol::status::WasteKey::FirstHandInAccept;
            let color = if m.bad { p.orange } else { p.text };
            body = body
                .when(quality, |b| b.child(div().h(px(1.)).my_1().bg(p.hair)))
                .child(
                    h_flex()
                        .w_full()
                        .min_h(px(28.))
                        .gap_2()
                        .items_center()
                        .child(div().flex_1().min_w_0().text_size(px(12.5)).text_color(p.text2).child(words::waste_label(m.key)))
                        .child(div().flex_none().font_family(MONO).text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(color).child(words::waste_value(m.key, m.day)))
                        .child(div().flex_none().w(px(76.)).text_right().font_family(MONO).text_size(px(11.5)).text_color(p.muted).child(t!("status.waste_week", value = words::waste_value(m.key, m.week)))),
                );
        }
    }
    div().flex_1().min_w(px(CARD_MIN)).child(card(p, "status-waste", t!("status.waste").to_owned(), None).child(body)).into_any_element()
}

/// What waits on the person, the most blocking first — the order Needs you
/// lists them in — each with what it holds up.
fn waiting(shell: &Shell, s: &SquadStatus, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let mut card = card(p, "status-waiting", t!("status.waiting_title").to_owned(), None);
    for (ix, w) in s.waiting.iter().enumerate() {
        let ticket = w.ticket;
        let badge = crate::board::impact_words(&w.impact);
        let color = if w.impact.is_pressing() { p.red } else { p.amber };
        card = card.child(
            h_flex()
                .id(("status-waiting-row", ix))
                .w_full()
                .min_h(px(40.))
                .px_3()
                .py_1()
                .gap_3()
                .items_center()
                .cursor_pointer()
                .when(ix > 0, |r| r.border_t_1().border_color(p.hair))
                .hover(move |s| s.bg(p.wash))
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| match ticket {
                    Some(id) if !this.open_elsewhere(e, super::elsewhere::Goal::Ticket(id), window, cx) => this.open_ticket_row(id, window, cx),
                    Some(_) => {}
                    None => this.open_side_menu(crate::board::Menu::Inbox, cx),
                }))
                .child(div().w(px(52.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(ticket.map(|id| format!("T-{id}")).unwrap_or_default()))
                .child(div().flex_1().min_w_0().truncate().text_size(px(SMALL)).text_color(p.text).child(w.title.clone()))
                .child(div().flex_none().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(color).child(badge)),
        );
    }
    if s.waiting_more > 0 {
        card = card.child(div().px_3().py_2().text_size(px(12.)).text_color(p.muted).child(t!("status.more", count = s.waiting_more)));
    }
    card.into_any_element()
}

fn watched(shell: &Shell, s: &SquadStatus, now: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let list = &s.watched;
    let note = (!list.pinned).then(|| t!("status.watched_default").to_owned());
    let mut card = card(p, "status-watched", t!("status.watched").to_owned(), note);
    if list.tickets.is_empty() {
        return card.child(div().px_3().py_3().text_size(px(13.)).text_color(p.muted).child(t!("status.watched_none"))).into_any_element();
    }
    for (ix, w) in list.tickets.iter().enumerate() {
        card = card.child(watched_row(shell, w, ix, list.pinned, now, cx));
    }
    if list.more > 0 {
        card = card.child(div().px_3().py_2().text_size(px(12.)).text_color(p.muted).child(t!("status.more", count = list.more)));
    }
    card.into_any_element()
}

fn watched_row(shell: &Shell, w: &Watched, ix: usize, pinned: bool, now: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let WatchRow { ticket: id, title, owner, status, paused, how_long } = watch_row_of(w, now);
    h_flex()
        .id(("status-watch", ix))
        .w_full()
        .min_h(px(40.))
        .px_3()
        .py_1()
        .gap_3()
        .items_center()
        .cursor_pointer()
        .when(ix > 0, |r| r.border_t_1().border_color(p.hair))
        .hover(move |s| s.bg(p.wash))
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Ticket(id), window, cx) {
                this.open_ticket_row(id, window, cx);
            }
        }))
        .child(div().w(px(52.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{id}")))
        .child(div().flex_1().min_w_0().truncate().text_size(px(SMALL)).text_color(p.text).child(title))
        .children(owner.map(|o| div().flex_none().max_w(px(120.)).truncate().text_size(px(12.)).text_color(p.text2).child(o)))
        .child(
            h_flex()
                .flex_none()
                .gap_1()
                .text_size(px(12.))
                .text_color(if paused { p.amber } else { p.text2 })
                .child(status)
                .children(how_long.map(|h| div().text_color(p.muted).child(format!("· {h}")))),
        )
        .when(pinned, |r| {
            r.child(
                crate::ui::x_box(SharedString::from(format!("status-unwatch-{id}")), p.muted, t!("status.unwatch"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.watch_ticket(id, false, cx);
                    })),
            )
        })
        .into_any_element()
}

impl Shell {
    /// Status, from its row in the sidebar or `/status`.
    pub fn open_status(&mut self, cx: &mut Context<Self>) {
        self.show(Screen::Status, cx);
        self.status_ask(cx);
        cx.notify();
    }

    /// The Speed card's choice, from a click: sent as it is for settings and `/parallel`,
    /// and the status asked for again a moment after, for the budget outlook.
    pub fn set_speed(&mut self, parallel: Parallel, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        self.set_parallel(parallel, cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(600)).await;
            this.update(cx, |this, cx| this.status_ask(cx)).ok();
        })
        .detach();
    }

    /// A row's button: where it leads.
    pub fn status_go(&mut self, goto: Goto, window: &mut Window, cx: &mut Context<Self>) {
        match goto {
            Goto::Ticket(id) => self.open_ticket_row(id, window, cx),
            Goto::NeedsYou => self.open_side_menu(crate::board::Menu::Inbox, cx),
            Goto::Logins => self.open_logins(cx),
            Goto::Progress => self.open_progress(cx),
            // Screen Sharing to the daemon's Mac, as the card's "Unlock it" does.
            Goto::UnlockMac => {
                let url = self.unlock.url(&self.mac_address());
                if !cfg!(feature = "snapshot") {
                    cx.open_url(&url);
                }
            }
        }
    }

    /// Whether the window can ask the daemon now: attached to a squad.
    fn status_can_ask(&self) -> bool {
        matches!(self.model.conn, Conn::Connected { .. }) && !self.model.squad.is_empty() && !self.status.demo
    }

    /// Ask the daemon for the squad's status, off the main thread; what
    /// comes back is the squad's the window shows still.
    pub fn status_ask(&mut self, cx: &mut Context<Self>) {
        if !self.status_can_ask() || self.status.loading {
            return;
        }
        let Some(uploader) = self.link.as_ref().map(|l| l.uploader()) else { return };
        let squad = self.model.squad.clone();
        self.status.loading = true;
        let ask = equipe_core::protocol::status::StatusAsk { squad: squad.clone(), utc_offset_secs: words::utc_offset_secs(self.model.unix) };
        let look = cx.background_spawn(async move { uploader.call(equipe_core::protocol::rpc::SQUAD_STATUS, ask, words::ASK_WAIT).and_then(|v| serde_json::from_value::<SquadStatus>(v).map_err(|e| e.to_string())) });
        cx.spawn(async move |this, cx| {
            let answer = look.await;
            this.update(cx, |this, cx| {
                if this.model.squad != squad {
                    this.status.loading = false;
                    return;
                }
                this.status.answered(answer);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Ask now and then, for as long as the window lives: sooner while the
    /// screen is open. Never from the real view's own daemon beyond a read.
    pub fn status_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let wait = match this.update_in(cx, |this, _, cx| {
                    this.status_ask(cx);
                    if this.screen == Screen::Status { words::POLL_OPEN } else { words::POLL_CLOSED }
                }) {
                    Ok(wait) => wait,
                    Err(_) => break,
                };
                cx.background_executor().timer(wait).await;
            }
        })
        .detach();
    }

    /// Pin a ticket to watch, or let it go: the daemon keeps it; the screen reads it at once.
    pub fn watch_ticket(&mut self, ticket: TicketId, watch: bool, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        self.status.flip(ticket, watch);
        self.send_command(Command::WatchTicket { squad: self.model.squad.clone(), ticket, watch });
        cx.notify();
        // The daemon drops its kept answer on the command; ask again a moment after it.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(600)).await;
            this.update(cx, |this, cx| this.status_ask(cx)).ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::status::{Action, Cause, Note, Tiles, WatchedList};
    use equipe_core::protocol::{Engine, Priority, TicketStatus};
    use gpui_kit::WindowAppearance;

    const NOW: u64 = 1_800_000_000;

    fn block(cause: Cause, severity: Severity, count: u32, handling: Handling) -> Block {
        Block { cause, severity, count, why: None, engine: None, at: None, detail: None, tickets: vec![], handling, goto: None }
    }

    fn status(mood: Mood, blocked: Vec<Block>) -> SquadStatus {
        SquadStatus { squad: "acme".into(), at: NOW, mood, blocked, tiles: Tiles::default(), watched: WatchedList::default(), ..SquadStatus::default() }
    }

    fn contrast(a: Rgba, b: Rgba) -> f32 {
        let lum = |c: Rgba| {
            let f = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b)
        };
        let (x, y) = (lum(a) + 0.05, lum(b) + 0.05);
        x.max(y) / x.min(y)
    }

    #[::core::prelude::v1::test]
    fn the_page_is_the_figures_or_says_why_there_are_none() {
        let mut ui = crate::status::StatusUi::default();
        assert_eq!(body_of(&ui, "acme"), Body::Loading, "asked, not answered");
        ui.failed = Some("the daemon didn't answer".into());
        assert_eq!(body_of(&ui, "acme"), Body::Failed("the daemon didn't answer".into()));
        ui.answered(Err("unknown method".into()));
        assert_eq!(body_of(&ui, "acme"), Body::Unsupported, "an older daemon");
        ui.answered(Ok(status(Mood::Good, vec![])));
        assert_eq!(body_of(&ui, "acme"), Body::Ready);
        assert_eq!(body_of(&ui, "other"), Body::Loading, "another squad's figures aren't this window's");
        ui.answered(Err("the daemon didn't answer".into()));
        assert_eq!(body_of(&ui, "acme"), Body::Ready, "a failed look keeps what was known");
    }

    #[::core::prelude::v1::test]
    fn a_healthy_squad_reads_green_with_no_rows() {
        let s = status(Mood::Good, vec![]);
        let line = line_of(&s, NOW);
        assert_eq!(line.mood, Mood::Good);
        assert_eq!(line.headline, t!("status.good"));
        assert!(!line.detail.is_empty() && line.updated.contains(&words::clock(NOW)), "{line:?}");
    }

    #[::core::prelude::v1::test]
    fn a_slowed_squad_reads_amber_with_the_engine_notes_and_a_button_where_the_person_can_act() {
        let mut s = status(
            Mood::Slowed,
            vec![
                Block { engine: Some(Engine::Codex), at: Some(NOW + 3 * 86_400), ..block(Cause::EngineOut, Severity::Amber, 12, Handling::Fixed) },
                Block { goto: Some(Goto::Ticket(61)), tickets: vec![61], ..block(Cause::HandedBack, Severity::Amber, 1, Handling::You { action: Action::Look }) },
                block(Cause::Failing, Severity::Amber, 4, Handling::Resolving),
            ],
        );
        s.notes = vec![Note::Out { engine: Engine::Codex, back_at: Some(NOW + 3 * 86_400) }, Note::Pacing { engine: Engine::Claude, at_once: 4 }];
        let line = line_of(&s, NOW);
        assert_eq!(line.mood, Mood::Slowed);
        assert!(line.headline.contains("Codex") && line.detail.contains("Claude") && line.detail.contains("Codex"), "{line:?}");
        let rows: Vec<RowModel> = s.blocked.iter().map(|b| row_of(b, NOW)).collect();
        assert_eq!(rows[0].label, t!("status.act_fixed"));
        assert_eq!(rows[0].goto, None, "equipe handles it: nothing to press");
        assert!(rows[0].text.contains("12") && rows[0].text.contains("Codex"), "{:?}", rows[0]);
        assert_eq!((rows[1].label, rows[1].goto), (t!("status.act_look"), Some(Goto::Ticket(61))));
        assert_eq!((rows[2].label, rows[2].goto), (t!("status.act_resolving"), None));
    }

    #[::core::prelude::v1::test]
    fn a_stopped_squad_reads_red_and_says_what_stopped_it_and_since_when() {
        let s = status(Mood::Stopped, vec![Block { detail: Some("No space left on device".into()), at: Some(NOW - 25 * 60), ..block(Cause::BreakerOpen, Severity::Red, 3, Handling::Resolving) }]);
        let line = line_of(&s, NOW);
        assert_eq!((line.mood, line.headline.as_str()), (Mood::Stopped, t!("status.stop_breaker")));
        let row = row_of(&s.blocked[0], NOW);
        assert!(row.text.contains("No space left on device") && row.text.contains("25"), "{row:?}");
        assert_eq!((row.severity, row.goto), (Severity::Red, None));
    }

    #[::core::prelude::v1::test]
    fn a_button_only_leads_where_the_daemon_said() {
        let no_way = Block { goto: None, ..block(Cause::Decisions, Severity::Info, 2, Handling::You { action: Action::Answer }) };
        assert_eq!(row_of(&no_way, NOW).goto, None, "no destination: the label alone");
        let handled = Block { goto: Some(Goto::NeedsYou), ..block(Cause::EngineOut, Severity::Amber, 0, Handling::Fixed) };
        assert_eq!(row_of(&handled, NOW).goto, None, "equipe handles it: even a destination isn't a button");
    }

    #[::core::prelude::v1::test]
    fn a_watched_ticket_reads_with_its_owner_status_and_how_long() {
        let w = Watched { ticket: 47, title: "Gallery".into(), status: TicketStatus::InProgress, owner: Some("frontend".into()), since: Some(NOW - 95 * 60), priority: Priority::Urgent, paused: None };
        let row = watch_row_of(&w, NOW);
        assert_eq!((row.ticket, row.owner.as_deref(), row.paused), (47, Some("frontend"), false));
        assert_eq!(row.status, crate::state::status_word(TicketStatus::InProgress));
        assert_eq!(row.how_long, Some(short_span(95 * 60)));
        let paused = watch_row_of(&Watched { paused: Some(equipe_core::protocol::PauseWhy::EngineLimit), since: None, ..w }, NOW);
        assert_eq!((paused.status.as_str(), paused.paused, paused.how_long), (t!("run.paused"), true, None));
    }

    #[::core::prelude::v1::test]
    fn the_three_dots_are_told_apart_and_read_on_both_appearances() {
        for appearance in [WindowAppearance::Dark, WindowAppearance::Light] {
            let p = Palette::new(appearance);
            let colors = [mood_color(p, Mood::Good), mood_color(p, Mood::Slowed), mood_color(p, Mood::Stopped)];
            assert!(colors[0] != colors[1] && colors[1] != colors[2] && colors[0] != colors[2], "{appearance:?}");
            for c in colors {
                for surface in [p.bg, p.raised, p.side] {
                    assert!(contrast(c, surface) >= 3., "a dot is a graphic: 3:1 on its surface ({appearance:?}: {:.2})", contrast(c, surface));
                }
            }
        }
    }
}
