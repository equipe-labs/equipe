//! The terminal panel at the right: a shell on the daemon's machine
//! (`term.rs`), for what only the person types — `sudo`, a password, a
//! sign-in — without leaving the app. It opens from the header, from a
//! reply's shell block (typed in, not run), and on its own when a reply asks
//! the person to run something as the machine's administrator.

use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::Shell;
use crate::icons::{Icon, icon};
use crate::term::{Colors, Happened, Size, Terminal};
use crate::theme::{MONO, META, SMALL};
use crate::ui;

/// The text's size, and a line's height.
const FONT: f32 = 12.5;
const LINE: f32 = 17.;
/// Room around the text.
const PAD: f32 = 10.;

/// The open terminal.
pub struct TermUi {
    pub term: Terminal,
    pub focus: FocusHandle,
    pub open: bool,
    /// Where it runs, for its bar.
    pub place: String,
    /// A wheel's turn not yet a whole line: a trackpad's small steps add up.
    pub wheel: f32,
    _reading: Task<()>,
}

/// Under the conversation: how tall by default, and at least.
pub const BELOW: f32 = 280.;
const BELOW_MIN: f32 = 140.;
/// The bar over the text.
const BAR: f32 = 36.;

/// A cell's width in this font, measured once per window size.
fn cell_width(window: &Window) -> f32 {
    let system = window.text_system();
    let id = system.resolve_font(&font(MONO));
    system.advance(id, px(FONT), 'm').map(|s| f32::from(s.width)).unwrap_or(FONT * 0.6)
}

/// Columns and lines that fit `wide` × `tall` of text.
fn fits(window: &Window, (wide, tall): (f32, f32)) -> Size {
    let width = wide - 2. * PAD;
    let height = tall - 2. * PAD;
    // Whole pixels: the grid is drawn at the rounded width, so it is counted at it.
    let cw = cell_width(window).round().max(1.);
    Size { cols: (width / cw).floor().max(20.) as u16, lines: (height / LINE).floor().max(5.) as u16, cell_w: cw.round() as u16, cell_h: LINE as u16 }
}

impl Shell {
    /// Open the terminal (a new one if none runs), focused; `suggest` is typed
    /// at its prompt for the person to run — never run for them.
    pub fn open_terminal(&mut self, suggest: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.as_ref().is_none_or(|t| t.term.ended) {
            let dir = Some(crate::render::base_dir(self)).filter(|d| !d.as_os_str().is_empty());
            let program = crate::term::program(&self.endpoint, dir.as_deref());
            // A far machine's folder isn't this Mac's: ssh goes there itself.
            let here = if self.endpoint.is_remote() { None } else { dir.as_deref() };
            match Terminal::start(program, here, fits(window, self.term_box(window, cx))) {
                Ok((term, mut rx)) => {
                    let reading = cx.spawn(async move |this, cx| {
                        while let Some(happened) = rx.recv().await {
                            let alive = this.update(cx, |this, cx| {
                                if let Some(ui) = this.terminal.as_mut() {
                                    ui.term.heard(&happened);
                                }
                                if happened != Happened::Drew || this.terminal.as_ref().is_some_and(|t| t.open) {
                                    cx.notify();
                                }
                            });
                            if alive.is_err() {
                                break;
                            }
                        }
                    });
                    self.terminal = Some(TermUi { term, focus: cx.focus_handle(), open: true, place: self.endpoint.label(), wheel: 0., _reading: reading });
                }
                Err(e) => {
                    self.composer.flash = Some(t!("term.not_started", why = e.to_string()));
                    cx.notify();
                    return;
                }
            }
        }
        // The right side is the terminal's now.
        self.doc = None;
        let Some(ui) = self.terminal.as_mut() else { return };
        ui.open = true;
        if let Some(line) = suggest.filter(|l| !l.trim().is_empty()) {
            ui.term.suggest(&line);
        }
        window.focus(&ui.focus, cx);
        cx.notify();
    }

    /// How tall it is under the conversation: as left, within the window.
    pub fn term_height(&self, window: &Window) -> f32 {
        let most = (f32::from(window.viewport_size().height) - 240.).max(BELOW_MIN);
        self.remembered.term_height.unwrap_or(BELOW).clamp(BELOW_MIN, most)
    }

    /// The room the text has, wherever the terminal sits.
    pub fn term_box(&self, window: &Window, cx: &App) -> (f32, f32) {
        let viewport = window.viewport_size();
        if self.remembered.term_below {
            let side = self.board.sidebar_width * self.sidebar_shown(cx);
            (f32::from(viewport.width) - side, self.term_height(window) - BAR)
        } else {
            (self.side_width(window), f32::from(viewport.height) - super::header::HEIGHT)
        }
    }

    /// At the right, or under the conversation: kept for next time.
    pub fn move_terminal(&mut self, cx: &mut Context<Self>) {
        self.remembered.term_below = !self.remembered.term_below;
        self.remembered.save(&equipe_providers::credentials::equipe_home());
        cx.notify();
    }

    pub fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.terminal.as_mut() {
            Some(ui) if ui.open && !ui.term.ended => {
                ui.open = false;
                self.composer.focus(window, cx);
                cx.notify();
            }
            _ => self.open_terminal(None, window, cx),
        }
    }

    /// Closed: the shell ends with it.
    pub fn close_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.terminal = None;
        self.composer.focus(window, cx);
        cx.notify();
    }
}

pub fn panel(shell: &Shell, window: &mut Window, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let ui = shell.terminal.as_ref().filter(|t| t.open)?;
    let p = shell.palette;
    let dark = shell.palette.bg.r + shell.palette.bg.g + shell.palette.bg.b < 1.5;
    let colors = if dark { Colors::dark() } else { Colors::light() };
    // Kept the panel's size: a window resized, the shell hears it.
    let below = shell.remembered.term_below;
    let size = fits(window, shell.term_box(window, cx));
    let cw = f32::from(size.cell_w);
    let screen = ui.term.screen(&colors);
    let focused = ui.focus.is_focused(window);
    let title = ui.term.title.clone().filter(|t| !t.trim().is_empty());
    let bar = h_flex()
        .h(px(if below { BAR } else { super::header::HEIGHT }))
        .flex_none()
        .px_3()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(icon(Icon::Terminal, 15., p.muted))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).truncate().child(t!("term.title", place = ui.place.clone())))
                .children(title.map(|t| div().text_size(px(META)).text_color(p.muted).font_family(MONO).truncate().child(t))),
        )
        .when(ui.term.ended, |bar| bar.child(ui::icon_button(p, "term-again", Icon::Reload, t!("term.again"), None).on_click(cx.listener(|this, _, window, cx| this.open_terminal(None, window, cx)))))
        .child(
            ui::icon_button(p, "term-move", if below { Icon::ArrowRight } else { Icon::ArrowDown }, if below { t!("term.to_right") } else { t!("term.to_bottom") }, None)
                .on_click(cx.listener(|this, _, _, cx| this.move_terminal(cx))),
        )
        .child(ui::icon_button(p, "term-hide", Icon::X, t!("term.close"), None).on_click(cx.listener(|this, _, window, cx| this.close_terminal(window, cx))));
    let bg: Hsla = rgb(colors.background).into();
    let lines = screen.lines.into_iter().enumerate().map(|(row, runs)| {
        div().h(px(LINE)).relative().children(runs.into_iter().map(move |run| {
            div()
                .absolute()
                .top_0()
                .left(px(f32::from(run.col) * cw))
                .w(px(f32::from(run.cells) * cw))
                .h(px(LINE))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(rgb(run.look.fg))
                .when_some(run.look.bg, |d, bg| d.bg(rgb(bg)))
                .when(run.look.bold, |d| d.font_weight(FontWeight::BOLD))
                .when(run.look.italic, |d| d.italic())
                .when(run.look.underline, |d| d.underline())
                .child(run.text)
        }))
        .id(("term-line", row))
    });
    let cursor = screen.cursor.map(|(col, row)| {
        let at = div().absolute().top(px(PAD + f32::from(row) * LINE)).left(px(PAD + f32::from(col) * cw)).w(px(cw)).h(px(LINE));
        if focused { at.bg(rgb(colors.foreground)).opacity(0.55) } else { at.border_1().border_color(rgb(colors.foreground)).opacity(0.6) }
    });
    let body = div()
        .id("term-body")
        .track_focus(&ui.focus)
        .flex_1()
        .min_h_0()
        .relative()
        .overflow_hidden()
        .p(px(PAD))
        .bg(bg)
        .font_family(MONO)
        .text_size(px(FONT))
        .line_height(px(LINE))
        .cursor_text()
        .on_click(cx.listener(|this, _, window, cx| {
            if let Some(ui) = this.terminal.as_ref() {
                window.focus(&ui.focus, cx);
                cx.notify();
            }
        }))
        .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
            let Some(ui) = this.terminal.as_ref() else { return };
            let k = &e.keystroke;
            let m = k.modifiers;
            if m.platform && k.key == "v" {
                if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    ui.term.paste(&text);
                }
                cx.stop_propagation();
                return;
            }
            if let Some(bytes) = crate::term::key_bytes(&k.key, k.key_char.as_deref(), m.control, m.alt, m.platform, m.shift, ui.term.app_cursor()) {
                ui.term.write(bytes);
                cx.stop_propagation();
            }
        }))
        // Back through what scrolled by: a trackpad's small steps add up to lines.
        .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| {
            if let Some(ui) = this.terminal.as_mut() {
                ui.wheel += f32::from(e.delta.pixel_delta(px(LINE)).y);
                let lines = (ui.wheel / LINE).trunc();
                if lines != 0. {
                    ui.wheel -= lines * LINE;
                    ui.term.scroll(lines as i32);
                    cx.notify();
                }
                cx.stop_propagation();
            }
        }))
        .children(lines)
        .children(cursor);
    if below {
        let dragging = shell.term_drag.is_some();
        // Its top edge: drag to make it taller or shorter, double-click for the usual height.
        let edge = div()
            .id("term-edge")
            .group("term-edge")
            .absolute()
            .left_0()
            .right_0()
            .top(px(-4.))
            .h(px(8.))
            .cursor_row_resize()
            .child(div().absolute().left_0().right_0().top(px(4.)).h(px(1.)).bg(if dragging { p.accent } else { p.hair }).group_hover("term-edge", move |s| s.bg(p.accent).opacity(0.7)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    if e.click_count >= 2 {
                        this.remembered.term_height = None;
                        this.remembered.save(&equipe_providers::credentials::equipe_home());
                    } else {
                        this.term_drag = Some((f32::from(e.position.y), this.term_height(window)));
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            );
        return Some(
            v_flex()
                .id("term-panel")
                .relative()
                .w_full()
                .h(px(shell.term_height(window)))
                .flex_none()
                .min_h_0()
                .bg(p.bg)
                .child(bar)
                .child(body)
                .child(edge)
                .into_any_element(),
        );
    }
    Some(
        v_flex()
            .id("term-panel")
            .relative()
            .w(px(shell.side_width(window)))
            .flex_none()
            .h_full()
            .min_h_0()
            .border_l_1()
            .border_color(p.hair)
            .bg(p.bg)
            .child(bar)
            .child(body)
            .child(super::doc::edge(shell, cx))
            .into_any_element(),
    )
}

/// The size the panel wants now; the shell is told when it changed.
pub fn fit(shell: &mut Shell, window: &Window, cx: &App) {
    let room = shell.term_box(window, cx);
    if let Some(ui) = shell.terminal.as_mut().filter(|t| t.open) {
        ui.term.resize(fits(window, room));
    }
}
