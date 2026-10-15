//! "휴대폰 연결": a sheet over the window with a QR code the equipe app scans,
//! three short steps, the time the code has left, "링크 복사" for when a
//! camera won't do, then the phone asking ("허용할까요?") and "연결됨". The
//! daemon — on whichever machine the window is a window onto — does the
//! pairing; the rules are `crate::pair`. Opened from settings (스쿼드와 기기),
//! the account menu and the app menu.

use std::time::{Duration, Instant};

use equipe_core::protocol::Command;
use equipe_core::t;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::pair::{self, Problem, Stage};
use crate::theme::{Palette, R_LG, R_MD, SMALL, UI};
use crate::ui::{self, Kind};
use crate::Shell;

/// The sheet's width, and the QR code's side within it.
const WIDTH: f32 = 560.;
const QR: f32 = 216.;
/// The body's height, the same at every stage so the sheet never jumps.
const BODY: f32 = 236.;
/// How long "복사됨" stays on the button.
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// The phone's kind as people say it.
pub fn platform_name(platform: &str) -> String {
    match platform.to_ascii_lowercase().as_str() {
        "ios" => "iPhone".into(),
        "ipados" => "iPad".into(),
        "android" => "Android".into(),
        "" => t!("pair.a_phone").into(),
        other => other.to_owned(),
    }
}

/// The QR code of `url`: black modules on white with a quiet zone of four,
/// whole points per module so every edge is sharp, in dark mode as in light.
fn qr_code(url: &str) -> AnyElement {
    const QUIET: usize = 4;
    let Some((width, dark)) = pair::qr(url) else {
        return div().size(px(QR)).into_any_element();
    };
    let runs = pair::runs(width, &dark);
    let total = width + 2 * QUIET;
    let module = (QR / total as f32).floor().max(1.);
    let side = module * total as f32;
    div()
        .id("pair-qr")
        .flex_none()
        .size(px(side))
        .rounded(px(R_MD))
        .bg(rgb(0xFFFFFF))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    // Each run lands on whole device pixels.
                    let scale = window.scale_factor();
                    let snap = |v: f32| (v * scale).round() / scale;
                    let (x0, y0) = (snap(f32::from(bounds.origin.x)), snap(f32::from(bounds.origin.y)));
                    let ink = rgb(0x000000);
                    for &(row, col, len) in &runs {
                        let at = point(px(x0 + (QUIET + col) as f32 * module), px(y0 + (QUIET + row) as f32 * module));
                        window.paint_quad(fill(Bounds::new(at, size(px(len as f32 * module), px(module))), ink));
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

/// A numbered step.
fn step(p: Palette, n: usize, text: &'static str) -> impl IntoElement {
    h_flex()
        .gap_2p5()
        .items_start()
        .child(
            div()
                .flex_none()
                .size(px(22.))
                .mt(px(-1.))
                .rounded_full()
                .bg(p.wash)
                .border_1()
                .border_color(p.hair)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text2)
                .child(n.to_string()),
        )
        .child(div().flex_1().min_w_0().text_size(px(UI)).line_height(px(20.)).text_color(p.text).child(text))
}

/// The code, the steps beside it, its time and the link to copy.
fn showing(shell: &Shell, url: &str, now: Instant, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let pairing = &shell.pairing;
    let left = pairing.seconds_left(now).unwrap_or(0);
    let time = if pairing.renews() { t!("pair.renews_in", time = pair::clock(left)) } else { t!("pair.left", time = pair::clock(left)) };
    let copied = pairing.copied.is_some_and(|at| now < at + COPIED_FOR);
    let copy = ui::button(p, "pair-copy", Kind::Secondary, if copied { t!("pair.copied") } else { t!("pair.copy_link") })
        .icon(if copied { Icon::Check } else { Icon::Copy })
        .on_click(cx.listener(|this, _, _, cx| this.copy_pair_link(cx)));
    h_flex()
        .h(px(BODY))
        .gap_6()
        .items_center()
        .child(motion::fade(div().flex_none().p(px(2.)).rounded(px(R_LG)).border_1().border_color(p.hair).child(qr_code(url)), ("pair-qr-in", pairing.turn as usize), motion::SLOW))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_3()
                .child(step(p, 1, t!("pair.step_open")))
                .child(step(p, 2, t!("pair.step_settings")))
                .child(step(p, 3, t!("pair.step_scan")))
                .child(
                    h_flex()
                        .pt_2()
                        .gap_2()
                        .items_center()
                        .child(icon(Icon::Clock, crate::theme::ICON_SM, p.muted))
                        .child(div().text_size(px(12.5)).text_color(p.muted).child(time)),
                )
                .child(h_flex().child(copy)),
        )
        .into_any_element()
}

/// A stage that isn't the code: an icon or a spinner, a line, words, buttons.
fn centered(p: Palette, id: (&'static str, u64), mark: AnyElement, title: String, text: Option<String>, detail: Option<String>, buttons: Option<AnyElement>) -> AnyElement {
    let inner = v_flex()
        .w_full()
        .items_center()
        .gap_2()
        .child(mark)
        .child(div().pt_1().text_size(px(16.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).text_center().child(title))
        .children(text.map(|t| div().max_w(px(420.)).text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).text_center().child(t)))
        .children(detail.map(|d| div().max_w(px(420.)).text_size(px(12.)).line_height(px(17.)).text_color(p.muted).text_center().child(d)))
        .children(buttons.map(|b| div().pt_3().child(b)));
    v_flex().h(px(BODY)).justify_center().child(motion::rise(inner, ElementId::NamedInteger(id.0.into(), id.1))).into_any_element()
}

/// A round mark around an icon.
fn badge(glyph: Icon, ink: Rgba, ground: Rgba) -> AnyElement {
    div().size(px(56.)).rounded_full().bg(ground).flex().items_center().justify_center().child(icon(glyph, 26., ink)).into_any_element()
}

fn problem_words(problem: Problem, host: &str) -> (String, String) {
    match problem {
        Problem::Refused => (t!("pair.refused_title").into(), t!("pair.refused_text").into()),
        Problem::Expired => (t!("pair.expired_title").into(), t!("pair.expired_text").into()),
        Problem::Busy => (t!("pair.busy_title").into(), t!("pair.busy_text").into()),
        Problem::Relay => (t!("pair.relay_title").into(), t!("pair.relay_text").into()),
        Problem::Unavailable => (t!("pair.unavailable_title").into(), t!("pair.unavailable_text", host = host)),
        Problem::TooOld => (t!("pair.too_old_title").into(), t!("pair.too_old_text", host = host)),
        Problem::Disconnected => (t!("pair.disconnected_title").into(), t!("pair.disconnected_text", host = host)),
        Problem::Failed => (t!("pair.failed_title").into(), t!("pair.failed_text").into()),
    }
}

/// The body for the stage the pairing is at.
fn body(shell: &Shell, now: Instant, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let turn = shell.pairing.turn;
    let host = shell.endpoint.label();
    match &shell.pairing.stage {
        Stage::Showing { url, .. } => showing(shell, url, now, cx),
        Stage::Opening { .. } => {
            let mark = div().size(px(56.)).flex().items_center().justify_center().child(Spinner::new().with_size(px(22.)).color(Hsla::from(p.muted))).into_any_element();
            centered(p, ("pair-opening", turn), mark, t!("pair.opening").into(), Some(t!("pair.opening_text", host = host)), None, None)
        }
        Stage::Asked { name, platform } => {
            let buttons = h_flex()
                .gap_2()
                .child(ui::button(p, "pair-no", Kind::Secondary, t!("pair.deny")).field().on_click(cx.listener(|this, _, _, cx| this.answer_pairing(false, cx))))
                .child(ui::button(p, "pair-yes", Kind::Primary, t!("pair.allow")).field().on_click(cx.listener(|this, _, _, cx| this.answer_pairing(true, cx))))
                .into_any_element();
            centered(p, ("pair-asked", turn), badge(Icon::Phone, p.text, p.wash), t!("pair.asked_title", name = name, platform = platform_name(platform)), Some(t!("pair.asked_text").into()), None, Some(buttons))
        }
        Stage::Approving { name } => {
            let mark = div().size(px(56.)).flex().items_center().justify_center().child(Spinner::new().with_size(px(22.)).color(Hsla::from(p.muted))).into_any_element();
            centered(p, ("pair-approving", turn), mark, t!("pair.approving", name = name), None, None, None)
        }
        Stage::Paired { name, .. } => {
            let mark = motion::pop(div().child(badge(Icon::Done, p.on_primary, p.accent)), ("pair-done", turn as usize), false).into_any_element();
            centered(p, ("pair-paired", turn), mark, t!("pair.paired_title", name = name), Some(t!("pair.paired_text").into()), None, None)
        }
        Stage::Ended { problem, message } => {
            let (title, text) = problem_words(*problem, &host);
            let retry_label = if matches!(problem, Problem::Expired | Problem::Refused) { t!("pair.new_code") } else { t!("pair.retry") };
            let buttons = h_flex()
                .gap_2()
                .child(ui::button(p, "pair-retry", Kind::Primary, retry_label).icon(Icon::Retry).field().on_click(cx.listener(|this, _, _, cx| this.retry_pairing(cx))))
                .into_any_element();
            let mark = badge(Icon::Warning, if *problem == Problem::Refused { p.text2 } else { p.danger() }, if *problem == Problem::Refused { p.wash } else { p.danger_bg() });
            // The daemon's own words, when they add something.
            let detail = (!message.is_empty() && *problem != Problem::Refused).then(|| message.clone());
            centered(p, ("pair-ended", turn), mark, title, Some(text), detail, Some(buttons))
        }
    }
}

/// The sheet, while it's open.
pub fn sheet(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if !shell.pairing.open {
        return None;
    }
    let p = shell.palette;
    let now = Instant::now();
    let host = shell.endpoint.label();
    let close = ui::icon_button(p, "pair-close", Icon::X, t!("pair.close"), Some("Esc")).on_click(cx.listener(|this, _, window, cx| this.close_pairing(window, cx)));
    let header = h_flex()
        .gap_3()
        .items_start()
        .child(div().mt(px(2.)).child(badge_small(p)))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_size(px(17.)).line_height(px(24.)).font_weight(FontWeight::SEMIBOLD).child(t!("pair.title")))
                .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(t!("pair.subtitle", host = host))),
        )
        .child(close);
    let showing = matches!(shell.pairing.stage, Stage::Showing { .. } | Stage::Opening { .. });
    // Two lines tall whatever it says, so the sheet keeps its place between stages.
    let foot = h_flex()
        .min_h(px(36. + 12.))
        .gap_3()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .pt_3()
        .child(icon(Icon::Info, crate::theme::ICON_SM, p.muted))
        .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(if showing { t!("pair.no_app") } else { t!("pair.private") }));
    Some(crate::views::connect::modal_w(p, "pair", WIDTH, v_flex().gap_4().child(header).child(body(shell, now, cx)).child(foot)))
}

fn badge_small(p: Palette) -> AnyElement {
    div().size(px(32.)).rounded(px(R_MD)).bg(p.wash).border_1().border_color(p.hair).flex().items_center().justify_center().child(icon(Icon::Phone, 17., p.text)).into_any_element()
}

/// How a paired phone's line reads under its name: its kind, and whether
/// it's there now or when it was last.
pub fn device_line(device: &equipe_core::protocol::PairedDevice, now_unix: u64) -> String {
    let kind = platform_name(&device.platform);
    let when = if device.online {
        t!("pair.online").to_owned()
    } else if let Some(seen) = device.seen {
        t!("pair.seen_ago", span = crate::state::short_span(now_unix.saturating_sub(seen)))
    } else {
        t!("pair.added_ago", span = crate::state::short_span(now_unix.saturating_sub(device.added)))
    };
    format!("{kind} · {when}")
}

// ── what the sheet does ──

impl Shell {
    /// "휴대폰 연결…": the sheet opens and asks the daemon for a code.
    pub fn open_pairing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::tips::hide(cx);
        self.board.popup = None;
        if self.pairing.open {
            return;
        }
        for command in self.pairing.open(Instant::now()) {
            self.send_command(command);
        }
        // The countdown, the code's renewal, the close after "연결됨".
        self._pair_tick = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let open = this.update(cx, |this, cx| {
                    for command in this.pairing.tick(Instant::now()) {
                        this.send_command(command);
                    }
                    cx.notify();
                    this.pairing.open
                });
                if !matches!(open, Ok(true)) {
                    break;
                }
            }
        }));
        cx.notify();
    }

    /// Esc, ×: the code stops working and leaves the window.
    pub fn close_pairing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for command in self.pairing.close() {
            self.send_command(command);
        }
        self._pair_tick = None;
        if self.screen != crate::Screen::Settings {
            self.composer.focus(window, cx);
        }
        cx.notify();
    }

    pub fn answer_pairing(&mut self, approve: bool, cx: &mut Context<Self>) {
        for command in self.pairing.answer(approve) {
            self.send_command(command);
        }
        cx.notify();
    }

    pub fn retry_pairing(&mut self, cx: &mut Context<Self>) {
        for command in self.pairing.retry(Instant::now()) {
            self.send_command(command);
        }
        cx.notify();
    }

    /// "링크 복사": the code as a link, for the app's "paste a link instead".
    pub fn copy_pair_link(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = self.pairing.url() {
            cx.write_to_clipboard(ClipboardItem::new_string(url.to_owned()));
            self.pairing.copied = Some(Instant::now());
            cx.notify();
        }
    }

    /// The paired phones, from the daemon; the page says so if it never answers.
    pub fn ask_devices(&mut self, cx: &mut Context<Self>) {
        // Not attached, nothing would answer: asked again on attaching.
        if !matches!(self.model.conn, crate::state::Conn::Connected { .. }) {
            return;
        }
        self.send_command(Command::Devices);
        if self.pairing.devices.is_none() && self.pairing.devices_asked.is_none() {
            self.pairing.devices_asked = Some(Instant::now());
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(pair::NO_ANSWER).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
    }

    /// Settings' "연결 끊기": first a question on the row, then the daemon forgets it.
    pub fn forget_device(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.forgetting.as_deref() == Some(id) {
            self.forgetting = None;
            self.send_command(Command::Unpair { device: id.to_owned() });
        } else {
            self.forgetting = Some(id.to_owned());
        }
        cx.notify();
    }
}
