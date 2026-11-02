//! 지표 (`metrics.rs`): the product's numbers from the services it reports to
//! — a card per metric against the period before, a banner where a service
//! failed (its last value kept, dimmed), the form that connects one. Numbers
//! come from the daemon's `metrics.*` alone; a key typed here is hidden, goes
//! to the daemon in `metrics.connect` and is out of its box at once — never
//! on screen, in a message of the conversation or in the log.

use std::collections::BTreeMap;
use std::time::Duration;

use equipe_core::protocol::metrics::{self as wire, Connection, ConnectParams, DisconnectParams, ErrorCode, QueryParams, QueryResult, RefreshParams, SourceKind, SourceSpec, Sources, SourcesParams};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::metrics::{self, Banner, Body, Card, Change, Draft, Flow};
use crate::state::now_unix;
use crate::theme::{CONTROL, MONO, Palette, R_LG, R_MD, SMALL};
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

/// A card's width before it grows into its row, and the least it takes.
const CARD_BASIS: f32 = 320.;
const CARD_MIN: f32 = 260.;
/// How long a refresh may take: each service's fetch waits up to 90 s.
const REFRESH_WAIT: Duration = Duration::from_secs(100);
const ASK_WAIT: Duration = Duration::from_secs(20);

/// The boxes of the open form: one per field of the service, by its key.
#[derive(Default)]
pub struct Inputs {
    boxes: Vec<(String, Entity<InputState>)>,
    subscriptions: Vec<Subscription>,
    /// The connection whose "끊기" was pressed once.
    disconnecting: Option<String>,
}

impl Inputs {
    /// Whether one of the boxes has the keyboard: what is typed is the box's.
    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        self.boxes.iter().any(|(_, b)| b.read(cx).focus_handle(cx).is_focused(window))
    }

    fn values(&self, cx: &App) -> BTreeMap<String, String> {
        self.boxes.iter().map(|(k, b)| (k.clone(), b.read(cx).value().to_string())).collect()
    }

    /// Every box emptied: a key is never kept past the request.
    fn clear(&self, window: &mut Window, cx: &mut App) {
        for (_, b) in &self.boxes {
            b.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        }
    }
}

// ---- words ----

/// A word of the catalog for a name the daemon gave, else the daemon's own.
fn word(key: String, fallback: &str) -> String {
    crate::i18n::catalog().own(&key).map_or_else(|| fallback.to_owned(), str::to_owned)
}

fn service(kind: SourceKind, spec: Option<&SourceSpec>) -> String {
    word(format!("metrics.kind.{}", kind.as_str()), spec.map_or(kind.as_str(), |s| s.label.as_str()))
}

fn short(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Posthog => "PH",
        SourceKind::Github => "GH",
        SourceKind::AppStore => "AS",
        SourceKind::Fake => "··",
    }
}

fn code_word(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::Auth => "auth",
        ErrorCode::RateLimited => "rate_limited",
        ErrorCode::Network => "network",
        ErrorCode::BadResponse => "bad_response",
        ErrorCode::Keychain => "keychain",
        ErrorCode::Other => "other",
    }
}

fn lookup(key: String) -> String {
    crate::i18n::catalog().get(&key).to_owned()
}

fn fill(key: String, args: &[(&str, String)]) -> String {
    crate::i18n::catalog().fill(&key, args)
}

/// The colour a failure draws in: a key or a break in red, a wait in orange.
fn tone(p: Palette, code: ErrorCode) -> Rgba {
    match code {
        ErrorCode::RateLimited | ErrorCode::Network => wait(p),
        _ => p.red,
    }
}

/// The design tokens' `color.ok` and `color.wait`, light and dark.
fn dark(p: Palette) -> bool {
    p.bg.r < 0.5
}

fn ok(p: Palette) -> Rgba {
    if dark(p) { rgb(0x7FCB8E) } else { rgb(0x1F6B33) }
}

fn wait(p: Palette) -> Rgba {
    if dark(p) { rgb(0xEC9A62) } else { rgb(0xA04610) }
}

// ---- the screen ----

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let m = &shell.metrics;
    let body = match &m.flow {
        Flow::Connect(draft) => connect_page(shell, draft, cx),
        Flow::Dash => dash_page(shell, cx),
    };
    v_flex()
        .id("metrics")
        .flex_1()
        .min_h_0()
        .bg(p.bg)
        .child(header(shell, cx))
        .child(div().id("metrics-scroll").flex_1().min_h_0().overflow_y_scroll().child(v_flex().w_full().max_w(px(1100.)).p_6().gap_4().child(body)))
        .into_any_element()
}

fn header(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let m = &shell.metrics;
    let bar = ui::title_bar(h_flex()).h(px(super::header::HEIGHT)).flex_none().pr_4().pl(px(shell.header_inset(16., cx))).gap_3().items_center().border_b_1().border_color(p.hair);
    let bar = bar.child(div().flex_1());
    if !matches!(m.flow, Flow::Dash) || m.sources.is_none() {
        // Connecting: the way back to the numbers, once there are some.
        let back = (!m.is_empty() && m.sources.is_some() && !matches!(m.flow, Flow::Dash)).then(|| ui::button(p, "metrics-back-dash", Kind::Secondary, t!("metrics.see_dash")).on_click(cx.listener(|this, _, _, cx| this.metrics_show_dash(cx))));
        return bar.children(back);
    }
    let mut seg = h_flex().flex_none().h(px(26.)).p(px(2.)).gap(px(2.)).rounded(px(R_MD)).bg(p.wash);
    for days in metrics::PERIODS {
        let on = m.days == days;
        seg = seg.child(
            div()
                .id(SharedString::from(format!("metrics-days-{days}")))
                .h_full()
                .px_2()
                .flex()
                .items_center()
                .rounded(px(6.))
                .text_size(px(SMALL))
                .cursor_pointer()
                .when(on, |d| d.bg(p.raised).text_color(p.text).shadow_sm())
                .when(!on, |d| d.text_color(p.text2).hover(move |s| s.bg(p.wash)))
                .child(t!("metrics.period", n = days))
                .on_click(cx.listener(move |this, _, _, cx| this.metrics_period(days, cx))),
        );
    }
    bar.child(seg)
        .child(ui::button(p, "metrics-refresh", Kind::Secondary, if m.loading { t!("metrics.refreshing") } else { t!("metrics.refresh") }).disabled(m.loading).on_click(cx.listener(|this, _, _, cx| this.metrics_refresh(None, cx))))
        .child(ui::button(p, "metrics-manage", Kind::Secondary, t!("metrics.manage")).on_click(cx.listener(|this, _, window, cx| this.metrics_open_connect(None, None, window, cx))))
}

/// "지표 · equipe-site": whose numbers these are, with what follows it under.
fn title(shell: &Shell, under: Option<String>) -> Div {
    let p = shell.palette;
    let project = shell.model.focus.clone().or_else(|| shell.metrics.connections().iter().find_map(|c| c.project.clone()));
    let words = match project {
        Some(project) => t!("metrics.title_project", project = project),
        None => t!("metrics.title").to_owned(),
    };
    v_flex().gap_1().child(div().text_size(px(24.)).line_height(px(30.)).font_weight(FontWeight::BOLD).text_color(p.text).child(words)).children(under.map(|u| note(p, u)))
}

fn note(p: Palette, text: impl Into<SharedString>) -> Div {
    div().text_size(px(13.)).line_height(px(20.)).text_color(p.muted).child(text.into())
}

fn card_frame(p: Palette, border: Rgba) -> Div {
    v_flex().flex_1().flex_basis(px(CARD_BASIS)).min_w(px(CARD_MIN)).p_4().gap_3().rounded(px(R_LG)).border_1().border_color(border).bg(p.raised)
}

// ---- the dashboard ----

fn dash_page(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let m = &shell.metrics;
    let now = now_unix();
    let (Some(sources), query) = (&m.sources, &m.query) else {
        return match &m.failed {
            Some(why) => unreachable_card(p, why, cx),
            None => note(p, t!("metrics.loading")).into_any_element(),
        };
    };
    if sources.connections.is_empty() {
        return v_flex().gap_4().w_full().child(title(shell, query.as_ref().map(metrics::range_words))).child(empty_card(shell, sources, cx)).into_any_element();
    }
    let mut page = v_flex().gap_4().w_full().child(title(shell, query.as_ref().map(metrics::range_words)));
    for banner in metrics::banners(sources) {
        page = page.child(banner_row(shell, &banner, sources, now, cx));
    }
    if let Some(why) = &m.failed {
        page = page.child(unreachable_card(p, why, cx));
    }
    let cards = query.as_ref().map(|q| metrics::cards(q, sources)).unwrap_or_default();
    if cards.is_empty() && m.failed.is_none() {
        page = page.child(note(p, if m.loading { t!("metrics.loading") } else { t!("metrics.no_cards") }));
    }
    page.child(h_flex().w_full().flex_wrap().gap_4().items_stretch().children(cards.iter().enumerate().map(|(n, c)| metric_card(shell, n, c, sources, now))))
        .into_any_element()
}

fn unreachable_card(p: Palette, why: &str, cx: &mut Context<Shell>) -> AnyElement {
    h_flex()
        .w_full()
        .gap_3()
        .items_center()
        .p_4()
        .rounded(px(R_LG))
        .border_1()
        .border_color(p.red)
        .bg(p.raised)
        .child(v_flex().flex_1().min_w_0().gap_1().child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("metrics.failed"))).child(note(p, why.to_owned())))
        .child(ui::button(p, "metrics-retry-load", Kind::Secondary, t!("metrics.retry")).on_click(cx.listener(|this, _, _, cx| this.metrics_load(cx))))
        .into_any_element()
}

fn empty_card(shell: &Shell, sources: &Sources, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let buttons = sources.kinds.iter().enumerate().map(|(n, spec)| {
        let kind = spec.kind;
        let name = service(kind, Some(spec));
        let label = if n == 0 { t!("metrics.connect_kind", service = name) } else { name };
        ui::button(p, ("metrics-empty-connect", n), if n == 0 { Kind::Primary } else { Kind::Secondary }, label).on_click(cx.listener(move |this, _, window, cx| this.metrics_open_connect(Some(kind), None, window, cx)))
    });
    v_flex()
        .w_full()
        .p_6()
        .gap_3()
        .rounded(px(R_LG))
        .border_1()
        .border_color(p.hair)
        .bg(p.raised)
        .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("metrics.empty_title")))
        .child(note(p, t!("metrics.empty_body")).max_w(px(640.)))
        .child(h_flex().gap_2().flex_wrap().children(buttons))
        .into_any_element()
}

fn banner_row(shell: &Shell, b: &Banner, sources: &Sources, now: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let name = service(b.kind, sources.kinds.iter().find(|k| k.kind == b.kind));
    let code = code_word(b.error.code);
    let color = tone(p, b.error.code);
    let when = metrics::moment(b.at, now);
    let last = b.last_ok.map(|t| metrics::moment(t, now)).unwrap_or_default();
    let body = match (b.error.code, b.error.retry_at) {
        (ErrorCode::RateLimited, Some(at)) => fill("metrics.err.rate_limited.body".into(), &[("retry", metrics::moment(at, now)), ("last", last)]),
        (ErrorCode::RateLimited, None) => fill("metrics.err.rate_limited.body_soon".into(), &[("last", last)]),
        _ => fill(format!("metrics.err.{code}.body"), &[("when", when), ("last", last)]),
    };
    let id = b.connection.clone();
    let kind = b.kind;
    let key = metrics::needs_key(b.error.code);
    let action = if key {
        ui::button(p, SharedString::from(format!("metrics-banner-key-{id}")), Kind::Primary, t!("metrics.err.new_key")).on_click(cx.listener(move |this, _, window, cx| this.metrics_open_connect(Some(kind), Some(id.clone()), window, cx)))
    } else {
        ui::button(p, SharedString::from(format!("metrics-banner-retry-{id}")), Kind::Secondary, t!("metrics.retry")).on_click(cx.listener(move |this, _, _, cx| this.metrics_refresh(Some(id.clone()), cx)))
    };
    h_flex()
        .id(SharedString::from(format!("metrics-banner-{}", b.connection)))
        .w_full()
        .gap_3()
        .items_center()
        .px_4()
        .py_3()
        .rounded(px(R_LG))
        .border_1()
        .border_color(Rgba { a: 0.45, ..color })
        .bg(Rgba { a: 0.10, ..color })
        .child(icon(if b.error.code == ErrorCode::RateLimited { Icon::Clock } else { Icon::Warning }, 18., color))
        .child(v_flex().flex_1().min_w_0().gap(px(2.)).child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(fill(format!("metrics.err.{code}.title"), &[("service", name)]))).child(div().text_size(px(13.)).line_height(px(19.)).text_color(p.text2).child(body)))
        .child(action)
        .into_any_element()
}

fn metric_card(shell: &Shell, n: usize, c: &Card, sources: &Sources, now: u64) -> AnyElement {
    let p = shell.palette;
    let spec = sources.kinds.iter().find(|k| k.kind == c.kind);
    let name = service(c.kind, spec);
    let label = word(format!("metrics.metric.{}", c.metric), &c.label);
    let unit = word(format!("metrics.unit.{}", c.unit), &c.unit);
    let stale = c.error.is_some();
    let dot = c.error.map_or_else(|| ok(p), |e| tone(p, e));
    // "PostHog · 07:00", or "PostHog · 키 만료" while it is failing.
    let source = match (c.error, c.fetched) {
        (Some(e), _) => t!("metrics.source_error", service = name, what = lookup(format!("metrics.err.{}.short", code_word(e)))),
        (None, Some(at)) => t!("metrics.source", service = name, time = metrics::moment(at, now)),
        (None, None) => name.clone(),
    };
    let head = h_flex().gap_2().items_center().child(div().flex_1().min_w_0().truncate().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(label)).child(div().flex_none().size(px(7.)).rounded_full().bg(dot)).child(div().flex_none().max_w(px(240.)).truncate().text_size(px(12.)).text_color(p.muted).child(source));
    let border = c.error.map_or(p.hair, |e| tone(p, e));
    let mut card = card_frame(p, border).id(("metrics-card", n)).child(head);
    card = match &c.body {
        Body::Number { value, change, series, previous } => {
            let shown = value.clone().unwrap_or_else(|| "—".into());
            let numbers = h_flex().gap_2().items_baseline().child(div().text_size(px(32.)).line_height(px(38.)).font_weight(FontWeight::SEMIBOLD).text_color(if stale { p.text2 } else { p.text }).child(shown)).children(value.is_some().then(|| div().text_size(px(13.)).text_color(p.muted).child(unit.clone())));
            card.child(numbers).children(change.as_ref().map(|ch| change_row(p, ch))).child(sparkline(p, series.clone(), previous.clone(), stale))
        }
        Body::Bars(bars) => card.child(v_flex().gap_2().children(bars.iter().take(6).map(|b| {
            h_flex()
                .gap_3()
                .items_center()
                .child(div().w(px(96.)).flex_none().truncate().text_size(px(13.)).text_color(if stale { p.text2 } else { p.text }).child(b.label.clone()))
                .child(div().flex_1().h(px(8.)).rounded_full().bg(p.wash).child(div().h_full().w(relative(b.fill.clamp(0.02, 1.))).rounded_full().bg(if stale { Rgba { a: 0.5, ..p.accent } } else { p.accent })))
                .child(div().w(px(52.)).flex_none().text_size(px(13.)).text_color(p.text).text_right().child(b.value.clone()))
        }))),
    };
    if stale && let Some(at) = c.last_ok {
        card = card.child(note(p, t!("metrics.stale_last", when = metrics::moment(at, now))).text_size(px(12.)));
        if let Some(retry) = c.retry_at {
            card = card.child(note(p, t!("metrics.card_retry", time = metrics::moment(retry, now))).text_size(px(12.)));
        }
    } else if let Some(day) = c.as_of.as_ref().filter(|d| c.as_of.is_some() && Some(d.as_str()) < today_day(now).as_deref()) {
        card = card.child(note(p, t!("metrics.as_of", date = metrics::day_words(day))).text_size(px(12.)));
    }
    if c.days_known < c.days && c.days_known > 0 {
        card = card.child(note(p, t!("metrics.days_known", known = c.days_known, days = c.days)).text_size(px(12.)));
    }
    if let Some(keeps) = c.keeps_days.filter(|k| *k < c.days) {
        card = card.child(note(p, t!("metrics.keeps_days", service = name, days = keeps)).text_size(px(12.)));
    }
    card.into_any_element()
}

fn today_day(now: u64) -> Option<String> {
    Some(metrics::local_day_time(now).0)
}

fn change_row(p: Palette, ch: &Change) -> impl IntoElement + use<> {
    let (mark, color) = match ch.up {
        Some(true) => ("▲", ok(p)),
        Some(false) => ("▼", wait(p)),
        None => ("–", p.muted),
    };
    h_flex().gap_1p5().items_baseline().text_size(px(13.)).child(div().text_color(color).font_weight(FontWeight::SEMIBOLD).child(format!("{mark} {}", ch.text))).child(div().text_color(p.text2).child(t!("metrics.vs_previous")))
}

/// This period's line in the accent, the one before beside it, fainter.
fn sparkline(p: Palette, series: Vec<Option<f64>>, previous: Vec<Option<f64>>, stale: bool) -> impl IntoElement {
    const H: f32 = 52.;
    let (now_color, then_color) = (Rgba { a: if stale { 0.5 } else { 1. }, ..p.accent }, Rgba { a: 0.8, ..p.muted });
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let w: f32 = bounds.size.width.into();
            let (line, before) = metrics::spark(&series, &previous, w, H - 4.);
            let o = bounds.origin;
            // The period before is dashed: told from this one by more than its colour.
            for pair in before.windows(2) {
                let (Some((x0, y0)), Some((x1, y1))) = (pair[0], pair[1]) else { continue };
                let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt().max(0.1);
                let mut at = 0.;
                while at < len {
                    let end = (at + 4.).min(len);
                    let place = |d: f32| gpui_kit::point(o.x + px(x0 + (x1 - x0) * d / len), o.y + px(y0 + (y1 - y0) * d / len + 2.));
                    let mut dash = PathBuilder::stroke(px(1.5));
                    dash.move_to(place(at));
                    dash.line_to(place(end));
                    if let Ok(dash) = dash.build() {
                        window.paint_path(dash, then_color);
                    }
                    at += 7.;
                }
            }
            for (points, color, width) in [(&line, now_color, 2.)] {
                let mut path = PathBuilder::stroke(px(width));
                let mut pen_down = false;
                let mut any = false;
                for point in points.iter() {
                    match point {
                        Some((x, y)) => {
                            let at = gpui_kit::point(o.x + px(*x), o.y + px(*y + 2.));
                            if pen_down { path.line_to(at) } else { path.move_to(at) }
                            pen_down = true;
                            any = true;
                        }
                        None => pen_down = false,
                    }
                }
                if any && let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
        },
    )
    .w_full()
    .h(px(H))
}

// ---- connecting ----

fn connect_page(shell: &Shell, draft: &Draft, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let m = &shell.metrics;
    let Some(sources) = &m.sources else {
        return note(p, t!("metrics.loading")).into_any_element();
    };
    let mut row = h_flex().w_full().flex_wrap().gap_4().items_start().child(pick_card(shell, sources, draft, cx));
    if let Some(kind) = draft.kind {
        row = row.child(key_card(shell, sources, kind, draft, cx));
    }
    if let Some(kind) = draft.done {
        row = row.child(done_card(shell, sources, kind, cx));
    }
    v_flex()
        .gap_4()
        .child(v_flex().gap_1().child(div().text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("metrics.connect_title"))).child(note(p, t!("metrics.connect_sub"))))
        .child(row)
        .into_any_element()
}

fn step(p: Palette, text: String) -> Div {
    div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(text)
}

/// Step 1: the services, each to connect — or, once connected, to re-key or disconnect.
fn pick_card(shell: &Shell, sources: &Sources, draft: &Draft, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let rows = sources.kinds.iter().enumerate().map(|(n, spec)| {
        let kind = spec.kind;
        let connected = sources.connections.iter().find(|c| c.kind == kind);
        let name = service(kind, Some(spec));
        let about = lookup(format!("metrics.kind_about.{}", kind.as_str()));
        let on = draft.kind == Some(kind);
        let mut buttons = h_flex().flex_none().gap_1p5().items_center();
        match connected {
            Some(c) => {
                let (id, sure) = (c.id.clone(), shell.metrics_ui.disconnecting.as_deref() == Some(c.id.as_str()));
                let id2 = id.clone();
                buttons = buttons
                    .child(ui::button(p, ("metrics-rekey", n), Kind::Secondary, t!("metrics.change_key")).on_click(cx.listener(move |this, _, window, cx| this.metrics_open_connect(Some(kind), Some(id.clone()), window, cx))))
                    .child(ui::button(p, ("metrics-disconnect", n), if sure { Kind::Danger } else { Kind::Ghost }, if sure { t!("metrics.disconnect_sure") } else { t!("metrics.disconnect") }).on_click(cx.listener(move |this, _, _, cx| this.metrics_disconnect(&id2, cx))));
            }
            None => {
                buttons = buttons.child(ui::button(p, ("metrics-connect", n), if on { Kind::Secondary } else { Kind::Primary }, t!("metrics.connect")).on_click(cx.listener(move |this, _, window, cx| this.metrics_open_connect(Some(kind), None, window, cx))));
            }
        }
        h_flex()
            .gap_3()
            .items_center()
            .py_3()
            .border_t_1()
            .border_color(p.hair)
            .child(div().flex_none().size(px(40.)).flex().items_center().justify_center().rounded(px(R_MD)).bg(p.wash).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(short(kind)))
            .child(v_flex().flex_1().min_w_0().child(h_flex().gap_2().items_center().child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(name)).children(connected.map(|_| div().px_1p5().rounded_full().bg(Rgba { a: 0.12, ..ok(p) }).text_size(px(11.)).text_color(ok(p)).child(t!("metrics.connected_badge"))))).child(div().text_size(px(12.5)).text_color(p.muted).child(about)))
            .child(buttons)
    });
    card_frame(p, p.hair).child(step(p, t!("metrics.step1").into())).child(v_flex().children(rows)).child(note(p, t!("metrics.all_optional"))).into_any_element()
}

/// Step 2: the boxes of the service's fields. The keys are hidden.
fn key_card(shell: &Shell, sources: &Sources, kind: SourceKind, draft: &Draft, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let spec = sources.kinds.iter().find(|k| k.kind == kind);
    let name = service(kind, spec);
    let replacing = draft.replacing.is_some();
    let boxes = spec.into_iter().flat_map(|s| s.fields.iter()).filter_map(|f| {
        let (_, input) = shell.metrics_ui.boxes.iter().find(|(k, _)| *k == f.key)?;
        let label = word(format!("metrics.field.{}", f.key), &f.label);
        let hint = f.hint.as_deref().map(|h| word(format!("metrics.hint.{}", f.key), h));
        let focus = input.read(cx).focus_handle(cx);
        let ring = BoxShadow { color: p.accent.into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: false };
        let empty = draft.missing && f.required && input.read(cx).value().trim().is_empty() && !(replacing && f.secret);
        Some(
            v_flex()
                .gap_1()
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(label))
                .child(div().w_full().h(px(CONTROL + 4.)).px_3().flex().items_center().rounded(px(R_MD)).border_1().border_color(if empty { p.red } else { p.hair }).id(SharedString::from(format!("metrics-box-{}", f.key))).track_focus(&focus).in_focus(move |s| s.border_color(ring.color).shadow(vec![ring])).bg(p.code_bg).text_size(px(13.)).font_family(if f.secret { MONO } else { MONO }).child(Input::new(input).appearance(false).bordered(false)))
                .children(hint.map(|h| div().text_size(px(12.)).text_color(p.muted).child(h))),
        )
    });
    let how = lookup(format!("metrics.how.{}", kind.as_str()));
    let go = ui::button(p, "metrics-confirm", Kind::Primary, if draft.sending { t!("metrics.connecting") } else { t!("metrics.confirm") }).disabled(draft.sending).on_click(cx.listener(|this, _, window, cx| this.metrics_submit(window, cx)));
    let back = ui::button(p, "metrics-back", Kind::Ghost, t!("metrics.back")).on_click(cx.listener(|this, _, window, cx| this.metrics_pick_kind(None, None, window, cx)));
    card_frame(p, p.hair)
        .child(step(p, t!("metrics.step2", service = name)))
        .child(note(p, how).text_color(p.text2))
        .child(v_flex().gap_3().children(boxes))
        .children(draft.missing.then(|| div().text_size(px(13.)).text_color(p.red).child(t!("metrics.fill_required"))))
        .children(draft.failed.as_ref().map(|why| div().text_size(px(13.)).line_height(px(19.)).text_color(p.red).child(t!("metrics.connect_failed", why = why))))
        .child(h_flex().gap_2().items_center().child(go).child(back))
        .child(h_flex().gap_1p5().items_start().child(div().pt(px(3.)).child(icon(Icon::Lock, 12., p.muted))).child(note(p, t!("metrics.key_where")).text_size(px(12.5))))
        .into_any_element()
}

/// Step 3: connected; the first numbers follow in a few seconds.
fn done_card(shell: &Shell, sources: &Sources, kind: SourceKind, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let name = service(kind, sources.kinds.iter().find(|k| k.kind == kind));
    let next = sources.kinds.iter().find(|k| k.kind != kind && !sources.connections.iter().any(|c| c.kind == k.kind));
    let mut actions = h_flex().gap_2().flex_wrap().child(ui::button(p, "metrics-see-dash", Kind::Primary, t!("metrics.see_dash")).on_click(cx.listener(|this, _, _, cx| this.metrics_show_dash(cx))));
    if let Some(spec) = next {
        let k = spec.kind;
        actions = actions.child(ui::button(p, "metrics-next", Kind::Secondary, t!("metrics.connect_kind", service = service(k, Some(spec)))).on_click(cx.listener(move |this, _, window, cx| this.metrics_open_connect(Some(k), None, window, cx))));
    }
    card_frame(p, p.hair)
        .child(step(p, t!("metrics.step3").into()))
        .child(h_flex().gap_2().items_center().child(icon(Icon::Check, 16., ok(p))).child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("metrics.done_title", service = name))))
        .child(note(p, t!("metrics.done_body")))
        .child(actions)
        .child(note(p, t!("metrics.key_delete_note")).text_size(px(12.5)))
        .into_any_element()
}

// ---- asking the daemon ----

impl Shell {
    /// One request off the UI thread; `done` gets the answer on it.
    fn metrics_ask<T: serde::de::DeserializeOwned + Send + 'static>(&mut self, method: &'static str, params: impl serde::Serialize + Send + 'static, wait: Duration, cx: &mut Context<Self>, done: impl FnOnce(&mut Shell, Result<T, String>, &mut Context<Shell>) + 'static) {
        let Some(door) = self.link.as_ref().map(|l| l.uploader()) else {
            done(self, Err(t!("daemon.not_connected").into()), cx);
            return;
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let answer = door.call(method, params, wait).and_then(|v| serde_json::from_value::<T>(v).map_err(|_| t!("failure.answer_unreadable").to_string()));
            let _ = tx.send(answer);
        });
        cx.spawn(async move |this, cx| {
            let answer = rx.await.unwrap_or_else(|_| Err(t!("metrics.no_answer").into()));
            let _ = this.update(cx, |this, cx| done(this, answer, cx));
        })
        .detach();
    }

    /// 지표, opened: the numbers are asked for again.
    pub fn open_metrics(&mut self, cx: &mut Context<Self>) {
        self.show(Screen::Metrics, cx);
        self.metrics.flow = Flow::Dash;
        self.metrics_load(cx);
    }

    pub fn metrics_show_dash(&mut self, cx: &mut Context<Self>) {
        self.metrics.flow = Flow::Dash;
        self.metrics_load(cx);
        cx.notify();
    }

    /// The services connected, and the period's numbers.
    pub fn metrics_load(&mut self, cx: &mut Context<Self>) {
        if self.metrics.demo {
            cx.notify();
            return;
        }
        let (squad, project, days) = (self.model.squad.clone(), self.model.focus.clone(), self.metrics.days);
        self.metrics.loading = true;
        self.metrics.failed = None;
        cx.notify();
        let params = SourcesParams { squad: squad.clone(), project: project.clone() };
        self.metrics_ask::<Sources>(wire::SOURCES, params, ASK_WAIT, cx, move |this, answer, cx| {
            match answer {
                Ok(sources) => {
                    let any = !sources.connections.is_empty();
                    this.metrics.sources = Some(sources);
                    this.metrics.failed = None;
                    if !any {
                        this.metrics.query = None;
                        this.metrics.loading = false;
                    }
                }
                Err(why) => {
                    this.metrics.failed = Some(why);
                    this.metrics.loading = false;
                }
            }
            cx.notify();
        });
        let params = QueryParams { squad, project, days: Some(days), bucket: metrics::bucket(days), ..QueryParams::default() };
        self.metrics_ask::<QueryResult>(wire::QUERY, params, ASK_WAIT, cx, move |this, answer, cx| {
            this.metrics.loading = false;
            // A period picked since this was asked is not the one on screen.
            if this.metrics.days == days {
                match answer {
                    Ok(query) => this.metrics.query = Some(query),
                    Err(why) => this.metrics.failed = Some(why),
                }
            }
            cx.notify();
        });
    }

    pub fn metrics_period(&mut self, days: u32, cx: &mut Context<Self>) {
        if self.metrics.days == days {
            return;
        }
        self.metrics.days = days;
        if self.metrics.demo {
            cx.notify();
            return;
        }
        self.metrics_load(cx);
    }

    /// "새로고침" / "지금 다시 시도": the services are asked now, then the numbers again.
    pub fn metrics_refresh(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if self.metrics.demo {
            return;
        }
        let params = RefreshParams { squad: self.model.squad.clone(), project: self.model.focus.clone(), id };
        self.metrics.loading = true;
        cx.notify();
        self.metrics_ask::<Sources>(wire::REFRESH, params, REFRESH_WAIT, cx, |this, answer, cx| {
            match answer {
                Ok(sources) => this.metrics.sources = Some(sources),
                Err(why) => this.metrics.failed = Some(why),
            }
            this.metrics_load(cx);
        });
    }

    /// The form: `kind` picked (with `replacing` its connection, for a new
    /// key), or — `None` — the list of services alone.
    pub fn metrics_open_connect(&mut self, kind: Option<SourceKind>, replacing: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.show(Screen::Metrics, cx);
        if self.metrics.sources.is_none() {
            self.metrics_load(cx);
        }
        self.metrics_pick_kind(kind, replacing, window, cx);
    }

    pub fn metrics_pick_kind(&mut self, kind: Option<SourceKind>, replacing: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.metrics_ui.disconnecting = None;
        self.metrics_ui.boxes.clear();
        self.metrics_ui.subscriptions.clear();
        self.metrics.flow = Flow::Connect(Draft { kind, replacing: replacing.clone(), ..Draft::default() });
        if let Some(kind) = kind {
            let spec = self.metrics.spec(kind).cloned();
            let existing = replacing.as_ref().and_then(|id| self.metrics.connections().iter().find(|c| c.id == *id)).cloned();
            for field in spec.iter().flat_map(|s| s.fields.iter()) {
                let placeholder = word(format!("metrics.field.{}", field.key), &field.label);
                let secret = field.secret;
                let kept = existing.as_ref().is_some_and(|c| c.keys.contains(&field.key));
                let input = cx.new(|cx| {
                    let mut state = InputState::new(window, cx).masked(secret).placeholder(if kept { t!("metrics.key_kept").to_owned() } else { placeholder });
                    if let Some(value) = existing.as_ref().and_then(|c| c.settings.get(&field.key)) {
                        state.set_value(value.clone(), window, cx);
                    }
                    state
                });
                // ⏎ in a box connects.
                let subscription = cx.subscribe_in(&input, window, |shell, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        shell.metrics_submit(window, cx);
                    }
                });
                self.metrics_ui.boxes.push((field.key.clone(), input));
                self.metrics_ui.subscriptions.push(subscription);
            }
            if let Some((_, first)) = self.metrics_ui.boxes.first() {
                first.update(cx, |s, cx| s.focus(window, cx));
            }
        }
        cx.notify();
    }

    /// 확인하고 연결: every required box filled, the values to the daemon — and out of the boxes at once.
    pub fn metrics_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Flow::Connect(draft) = &self.metrics.flow else { return };
        let (Some(kind), true) = (draft.kind, !draft.sending) else { return };
        let Some(spec) = self.metrics.spec(kind).cloned() else { return };
        let replacing = draft.replacing.clone();
        let kept: Vec<String> = replacing.as_ref().and_then(|id| self.metrics.connections().iter().find(|c| c.id == *id)).map(|c| c.keys.clone()).unwrap_or_default();
        let values = self.metrics_ui.values(cx);
        let empty = metrics::missing(&spec, &values, &kept);
        if let Some(first) = empty.first() {
            if let Flow::Connect(d) = &mut self.metrics.flow {
                d.missing = true;
            }
            if let Some((_, b)) = self.metrics_ui.boxes.iter().find(|(k, _)| k == first) {
                b.update(cx, |s, cx| s.focus(window, cx));
            }
            cx.notify();
            return;
        }
        let params = ConnectParams { squad: self.model.squad.clone(), project: self.model.focus.clone(), kind: replacing.is_none().then_some(kind), id: replacing, settings: metrics::settings(&values) };
        drop(values);
        self.metrics_ui.clear(window, cx);
        if let Flow::Connect(d) = &mut self.metrics.flow {
            d.sending = true;
            d.missing = false;
            d.failed = None;
        }
        cx.notify();
        if self.metrics.demo {
            return;
        }
        self.metrics_ask::<Connection>(wire::CONNECT, params, ASK_WAIT, cx, move |this, answer, cx| {
            let Flow::Connect(d) = &mut this.metrics.flow else { return };
            d.sending = false;
            match answer {
                Ok(connection) => {
                    d.done = Some(kind);
                    d.kind = None;
                    let id = connection.id.clone();
                    // The first fetch follows on its own: the numbers are asked after it.
                    this.metrics_ui.boxes.clear();
                    this.metrics_ui.subscriptions.clear();
                    this.metrics_refresh(Some(id), cx);
                }
                Err(why) => d.failed = Some(why),
            }
            cx.notify();
        });
    }

    /// "연결 끊기": asked twice — the key leaves the Keychain.
    pub fn metrics_disconnect(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.metrics_ui.disconnecting.as_deref() != Some(id) {
            self.metrics_ui.disconnecting = Some(id.to_owned());
            cx.notify();
            return;
        }
        self.metrics_ui.disconnecting = None;
        if self.metrics.demo {
            return;
        }
        let params = DisconnectParams { squad: self.model.squad.clone(), id: id.to_owned(), forget: false };
        self.metrics_ask::<wire::Disconnected>(wire::DISCONNECT, params, ASK_WAIT, cx, |this, answer, cx| {
            if let Err(why) = answer {
                this.metrics.failed = Some(why);
            }
            this.metrics_load(cx);
        });
        cx.notify();
    }

    /// The screen on a sample of the daemon's answers, no daemon asked
    /// (`--features snapshot`): `dash`, `error`, `empty`, `connect`, `done`.
    #[cfg(feature = "snapshot")]
    pub fn metrics_demo(&mut self, which: &str, window: &mut Window, cx: &mut Context<Self>) {
        use metrics::demo;
        self.metrics.demo = true;
        self.show(Screen::Metrics, cx);
        self.metrics.flow = Flow::Dash;
        let (sources, query) = match which {
            "error" => demo::failing(),
            "empty" => (demo::nothing(), demo::healthy().1),
            "done" => {
                let (s, q) = demo::healthy();
                self.metrics.sources = Some(s);
                self.metrics.query = Some(q);
                self.metrics.flow = Flow::Connect(Draft { done: Some(SourceKind::Posthog), ..Draft::default() });
                cx.notify();
                return;
            }
            _ => demo::healthy(),
        };
        self.metrics.sources = Some(sources);
        self.metrics.query = Some(query);
        if matches!(which, "connect" | "key") {
            let kind = (which == "key").then_some(SourceKind::Posthog);
            self.metrics.sources = Some(demo::nothing());
            self.metrics_pick_kind(kind, None, window, cx);
        }
        cx.notify();
    }

    /// Type into the open form's box `n` (`--features snapshot`).
    #[cfg(feature = "snapshot")]
    pub fn metrics_type(&mut self, n: usize, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((_, b)) = self.metrics_ui.boxes.get(n) {
            b.update(cx, |s, cx| s.set_value(text.to_owned(), window, cx));
        }
    }
}
