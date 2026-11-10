//! The terminal panel at the right: a shell on the daemon's machine
//! (`term.rs`), for what only the person types — `sudo`, a password, a
//! sign-in — without leaving the app. It opens from the header, from a
//! reply's shell block (typed in, not run), and on its own when a reply asks
//! the person to run something as the machine's administrator.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::OnceLock;

use equipe_core::t;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::Shell;
use crate::icons::{Icon, icon};
use crate::term::{At, Colors, Happened, Mouse, Pick, Screen, Settle, Settled, Size, TermMode, Terminal};
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
    /// The pointer on the text: what a press there does, where the grid was drawn.
    pub mouse: Pointer,
    /// A new size waiting for the panel to stop changing.
    pub settle: Settle,
    _reading: Task<()>,
}

/// What the pointer is doing in the text, and where the grid was last drawn.
#[derive(Default)]
pub struct Pointer {
    /// A selection being dragged out.
    pub selecting: bool,
    /// Pressed while the program takes the mouse: its moves and release are its.
    pub reporting: bool,
    /// The cell the pointer was last reported at.
    last: Option<At>,
    /// The grid's top left in the window, as drawn.
    origin: Rc<Cell<Point<Pixels>>>,
}

/// Under the conversation: how tall by default, and at least.
pub const BELOW: f32 = 280.;
const BELOW_MIN: f32 = 140.;
/// The bar over the text.
const BAR: f32 = 36.;

/// Where a glyph the main font lacks comes from, in order: a Nerd Font's
/// (powerline arrows, prompt icons) — its symbols-only cut, else one-cell
/// "Mono" cut of any installed —, then the Mac's CJK faces ([`cjk_faces`]),
/// emoji, symbols.
pub fn fallback_chain(installed: &[String], lang: equipe_core::i18n::Lang) -> Vec<String> {
    let nerd = |name: &str| name.contains("Nerd Font") || name.ends_with(" NF") || name.contains(" NF ");
    let proportional = |name: &str| name.contains("Propo");
    let mut chain: Vec<String> = Vec::new();
    let mut add = |name: &str| {
        if !chain.iter().any(|c| c == name) {
            chain.push(name.to_owned());
        }
    };
    let mut installed: Vec<&str> = installed.iter().map(String::as_str).filter(|n| nerd(n) && !proportional(n)).collect();
    installed.sort_unstable();
    for symbols in ["Symbols Nerd Font Mono", "Symbols Nerd Font"] {
        if installed.contains(&symbols) {
            add(symbols);
        }
    }
    if let Some(mono) = installed.iter().find(|n| n.contains("Mono") || n.ends_with(" NF")) {
        add(mono);
    }
    if let Some(any) = installed.first() {
        add(any);
    }
    for name in cjk_faces(lang).iter().chain(&["Apple Color Emoji", "Apple Symbols"]) {
        add(name);
    }
    chain
}

/// The Mac's CJK faces, the one for `lang` first. Each has kana and Han too,
/// so with none of those the order gives every script its own: Hiragino
/// (kana, and the Han they share), PingFang (Han only Chinese writes), Apple
/// SD Gothic Neo (Hangul). Korean keeps the one face it always had.
fn cjk_faces(lang: equipe_core::i18n::Lang) -> &'static [&'static str] {
    use equipe_core::i18n::Lang;
    match lang {
        Lang::Ko => &["Apple SD Gothic Neo"],
        Lang::ZhHans => &["PingFang SC", "Apple SD Gothic Neo"],
        Lang::ZhHant => &["PingFang TC", "Apple SD Gothic Neo"],
        _ => &["Hiragino Sans", "PingFang SC", "Apple SD Gothic Neo"],
    }
}

/// The text's own font: Menlo, the Mac's terminal font — not the system's
/// monospaced one the rest of the app uses ([`MONO`]): CoreText takes no
/// fallback list for a system font, and a prompt's icons came out as boxes.
const TERM_FONT: &str = "Menlo";

/// The text's font: [`TERM_FONT`], then [`fallback_chain`].
fn term_font(window: &Window, bold: bool, italic: bool) -> Font {
    static CHAIN: OnceLock<FontFallbacks> = OnceLock::new();
    let fallbacks = CHAIN.get_or_init(|| FontFallbacks::from_fonts(fallback_chain(&window.text_system().all_font_names(), crate::i18n::lang()))).clone();
    Font {
        family: TERM_FONT.into(),
        features: FontFeatures::disable_ligatures(),
        fallbacks: Some(fallbacks),
        weight: if bold { FontWeight::BOLD } else { FontWeight::NORMAL },
        style: if italic { FontStyle::Italic } else { FontStyle::Normal },
    }
}

/// A cell's width: the font's own advance, unrounded — the grid is drawn
/// at exactly this, so it is counted at it.
fn cell_width(window: &Window) -> f32 {
    let system = window.text_system();
    let id = system.resolve_font(&term_font(window, false, false));
    system.advance(id, px(FONT), 'm').map(|s| f32::from(s.width)).ok().filter(|w| *w > 1.).unwrap_or(FONT * 0.6)
}

/// Columns and lines that fit `wide` × `tall` of panel.
fn fits(window: &Window, (wide, tall): (f32, f32)) -> Size {
    let cw = cell_width(window);
    let (cols, lines) = crate::term::grid_fit(wide - 2. * PAD, tall - 2. * PAD, cw, LINE);
    Size { cols, lines, cell_w: cw.round() as u16, cell_h: LINE as u16 }
}

impl TermUi {
    /// The cell under a point of the window.
    fn at(&self, position: Point<Pixels>, cw: f32) -> At {
        let origin = self.mouse.origin.get();
        let size = self.term.size;
        crate::term::cell_at(f32::from(position.x - origin.x), f32::from(position.y - origin.y), cw, LINE, size.cols, size.lines)
    }

    /// Lines the pointer is past the grid's top (−) or bottom (+), if it is.
    fn beyond(&self, position: Point<Pixels>) -> i32 {
        let y = f32::from(position.y - self.mouse.origin.get().y);
        let tall = f32::from(self.term.size.lines) * LINE;
        if y < 0. { -1 } else if y > tall { 1 } else { 0 }
    }

    /// Whether the program takes the mouse now: unless Shift is held, as in
    /// other terminals, so text can still be selected.
    fn reports(&self, shift: bool) -> bool {
        !shift && self.term.mode().intersects(TermMode::MOUSE_MODE)
    }

    fn copy(&self, cx: &mut App) -> bool {
        let Some(text) = self.term.selected() else { return false };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    fn paste(&self, cx: &mut App) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            self.term.paste(&text);
        }
    }
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
                    self.terminal = Some(TermUi { term, focus: cx.focus_handle(), open: true, place: self.endpoint.label(), wheel: 0., mouse: Pointer::default(), settle: Settle::default(), _reading: reading });
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
            // Less its left border.
            (self.side_width(window) - 1., f32::from(viewport.height) - super::header::HEIGHT)
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
    let below = shell.remembered.term_below;
    let cw = cell_width(window);
    let screen = ui.term.screen(&colors);
    let focused = ui.focus.is_focused(window);
    let title = ui.term.title.clone().filter(|t| !t.trim().is_empty());
    // One line, as the conversation's header and the document's bar: where it
    // runs, then what runs in it, muted — the same height, the same middle.
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
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(8.))
                .items_baseline()
                .child(div().flex_none().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).whitespace_nowrap().child(t!("term.title", place = ui.place.clone())))
                .children(title.map(|t| div().flex_1().min_w_0().text_size(px(META)).text_color(p.muted).font_family(MONO).overflow_hidden().whitespace_nowrap().text_ellipsis().child(t))),
        )
        .when(ui.term.ended, |bar| bar.child(ui::icon_button(p, "term-again", Icon::Reload, t!("term.again"), None).on_click(cx.listener(|this, _, window, cx| this.open_terminal(None, window, cx)))))
        .child(
            ui::icon_button(p, "term-move", if below { Icon::ArrowRight } else { Icon::ArrowDown }, if below { t!("term.to_right") } else { t!("term.to_bottom") }, None)
                .on_click(cx.listener(|this, _, _, cx| this.move_terminal(cx))),
        )
        .child(ui::icon_button(p, "term-hide", Icon::X, t!("term.close"), None).on_click(cx.listener(|this, _, window, cx| this.close_terminal(window, cx))));
    let bg: Hsla = rgb(colors.background).into();
    let picked: Hsla = Hsla { a: if dark { 0.42 } else { 0.28 }, ..p.accent.into() };
    let grid = grid(screen, colors, picked, cw, focused, ui.mouse.origin.clone());
    let body = div()
        .id("term-body")
        .track_focus(&ui.focus)
        .flex_1()
        .min_h_0()
        .relative()
        .overflow_hidden()
        .p(px(PAD))
        .bg(bg)
        .cursor_text()
        .on_mouse_down(MouseButton::Left, cx.listener(pressed))
        .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
            let Some(ui) = this.terminal.as_ref() else { return };
            let k = &e.keystroke;
            let m = k.modifiers;
            if m.platform && !m.shift && !m.alt && !m.control {
                let handled = match k.key.as_str() {
                    "v" => {
                        ui.paste(cx);
                        true
                    }
                    // Nothing selected: ⌘C is the window's, as anywhere.
                    "c" => ui.copy(cx),
                    "a" => {
                        ui.term.select_all();
                        cx.notify();
                        true
                    }
                    _ => false,
                };
                if handled {
                    cx.stop_propagation();
                }
                return;
            }
            if let Some(bytes) = crate::term::key_bytes(&k.key, k.key_char.as_deref(), m.control, m.alt, m.platform, m.shift, ui.term.app_cursor()) {
                // Typed: what was selected is let go.
                ui.term.unselect();
                ui.term.write(bytes);
                cx.stop_propagation();
            }
        }))
        // The Edit menu's, while the terminal has the keys.
        .on_action(cx.listener(|this, _: &input::Copy, _, cx| {
            if let Some(ui) = this.terminal.as_ref() {
                ui.copy(cx);
            }
        }))
        .on_action(cx.listener(|this, _: &input::Paste, _, cx| {
            if let Some(ui) = this.terminal.as_ref() {
                ui.paste(cx);
            }
        }))
        .on_action(cx.listener(|this, _: &input::SelectAll, _, cx| {
            if let Some(ui) = this.terminal.as_ref() {
                ui.term.select_all();
                cx.notify();
            }
        }))
        // Back through what scrolled by: a trackpad's small steps add up to
        // lines. A program that takes the mouse hears the wheel; a full-screen
        // one that doesn't (less, man) gets arrows, as other terminals send.
        .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, window, cx| {
            let cw = cell_width(window);
            if let Some(ui) = this.terminal.as_mut() {
                ui.wheel += f32::from(e.delta.pixel_delta(px(LINE)).y);
                let lines = (ui.wheel / LINE).trunc();
                if lines != 0. {
                    ui.wheel -= lines * LINE;
                    let mode = ui.term.mode();
                    let up = lines > 0.;
                    let steps = lines.abs() as usize;
                    if ui.reports(e.modifiers.shift) {
                        let at = ui.at(e.position, cw);
                        let event = if up { Mouse::WheelUp } else { Mouse::WheelDown };
                        for _ in 0..steps {
                            if let Some(bytes) = crate::term::mouse_report(mode, event, at, false, e.modifiers.alt, e.modifiers.control) {
                                ui.term.report(bytes);
                            }
                        }
                    } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
                        let arrow = match (mode.contains(TermMode::APP_CURSOR), up) {
                            (true, true) => "\x1bOA",
                            (true, false) => "\x1bOB",
                            (false, true) => "\x1b[A",
                            (false, false) => "\x1b[B",
                        };
                        ui.term.report(arrow.repeat(steps).into_bytes());
                    } else {
                        ui.term.scroll(lines as i32);
                    }
                    cx.notify();
                }
                cx.stop_propagation();
            }
        }))
        .child(grid)
        // Copy, paste, select all: the Edit menu's words.
        .context_menu(|menu, _, cx| {
            let selected = cx.try_global::<crate::render::ShellRef>().and_then(|s| s.0.upgrade()).is_some_and(|shell| shell.read(cx).terminal.as_ref().is_some_and(|t| t.term.has_selection()));
            let act = |what: u8| {
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    crate::render::with_shell(cx, |shell, cx| {
                        let Some(ui) = shell.terminal.as_ref() else { return };
                        match what {
                            0 => {
                                ui.copy(cx);
                            }
                            1 => ui.paste(cx),
                            _ => ui.term.select_all(),
                        }
                        cx.notify();
                    });
                }
            };
            menu.item(PopupMenuItem::new(t!("main.menu_copy")).disabled(!selected).on_click(act(0)))
                .item(PopupMenuItem::new(t!("main.menu_paste")).on_click(act(1)))
                .separator()
                .item(PopupMenuItem::new(t!("main.menu_select_all")).on_click(act(2)))
        });
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

/// The grid, painted cell by cell: every character at its column × the
/// cell's width — a glyph from a fallback font, wider or narrower than the
/// main one, can't push the rest of its line along — and a wide one centred
/// in its two cells.
fn grid(screen: Screen, colors: Colors, picked: Hsla, cw: f32, focused: bool, origin: Rc<Cell<Point<Pixels>>>) -> impl IntoElement {
    canvas(
        move |bounds, _, _| origin.set(bounds.origin),
        move |bounds, (), window, cx| {
            let at = |col: u16, row: usize| point(bounds.origin.x + px(f32::from(col) * cw), bounds.origin.y + px(row as f32 * LINE));
            let cells = |n: u16| size(px(f32::from(n) * cw), px(LINE));
            for (row, runs) in screen.lines.iter().enumerate() {
                for run in runs {
                    if let Some(bg) = run.look.bg {
                        window.paint_quad(fill(Bounds::new(at(run.col, row), cells(run.cells)), rgb(bg)));
                    }
                }
            }
            for (row, spans) in screen.selected.iter().enumerate() {
                for &(from, to) in spans {
                    window.paint_quad(fill(Bounds::new(at(from, row), cells(to + 1 - from)), picked));
                }
            }
            let fg: Hsla = rgb(colors.foreground).into();
            if let Some(cursor) = screen.cursor.filter(|_| !focused) {
                window.paint_quad(outline(Bounds::new(at(cursor.col, cursor.row as usize), cells(if cursor.wide { 2 } else { 1 })), fg.opacity(0.6), BorderStyle::Solid));
            }
            for (row, runs) in screen.lines.iter().enumerate() {
                for run in runs {
                    if run.text.trim().is_empty() {
                        continue;
                    }
                    let style = TextRun {
                        len: run.text.len(),
                        font: term_font(window, run.look.bold, run.look.italic),
                        color: rgb(run.look.fg).into(),
                        background_color: None,
                        underline: run.look.underline.then_some(UnderlineStyle { thickness: px(1.), color: None, wavy: false }),
                        strikethrough: None,
                    };
                    let system = window.text_system().clone();
                    let mut origin = at(run.col, row);
                    let line = if run.wide {
                        let line = system.shape_line(run.text.clone().into(), px(FONT), &[style], None);
                        let room = px(2. * cw);
                        if line.width < room {
                            origin.x += (room - line.width) / 2.;
                        }
                        line
                    } else {
                        system.shape_line(run.text.clone().into(), px(FONT), &[style], Some(px(cw)))
                    };
                    let _ = line.paint(origin, px(LINE), TextAlign::Left, None, window, cx);
                }
            }
            if let Some(cursor) = screen.cursor.filter(|_| focused) {
                window.paint_quad(fill(Bounds::new(at(cursor.col, cursor.row as usize), cells(if cursor.wide { 2 } else { 1 })), fg.opacity(0.55)));
            }
        },
    )
    .size_full()
}

/// A press in the text: ⌘ on a web address opens it; a program that takes
/// the mouse hears it; else a selection begins — by the character, the
/// word (two clicks) or the line (three), or the one there grows (⇧).
fn pressed(this: &mut Shell, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Shell>) {
    let cw = cell_width(window);
    let Some(ui) = this.terminal.as_mut() else { return };
    window.focus(&ui.focus, cx);
    let at = ui.at(e.position, cw);
    let m = e.modifiers;
    if m.platform
        && let Some(url) = ui.term.url_at(at)
    {
        cx.open_url(&url);
        cx.stop_propagation();
        return;
    }
    if ui.reports(m.shift) {
        if let Some(bytes) = crate::term::mouse_report(ui.term.mode(), Mouse::Down(0), at, false, m.alt, m.control) {
            ui.term.report(bytes);
        }
        ui.mouse.reporting = true;
        ui.mouse.last = Some(at);
    } else {
        if m.shift && ui.term.has_selection() {
            ui.term.select_to(at);
        } else {
            ui.term.select(at, Pick::of_clicks(e.click_count));
        }
        ui.mouse.selecting = true;
    }
    cx.stop_propagation();
    cx.notify();
}

/// The pointer moved anywhere in the window (the root hears it): a selection
/// being dragged follows it — past the top or bottom, the text scrolls —
/// and a program that took the press hears the drag.
pub fn dragged(shell: &mut Shell, e: &MouseMoveEvent, window: &Window, cx: &mut Context<Shell>) {
    let Some(ui) = shell.terminal.as_mut().filter(|t| t.mouse.selecting || t.mouse.reporting) else { return };
    if e.pressed_button != Some(MouseButton::Left) {
        released(shell, e.position, window, cx);
        return;
    }
    let at = ui.at(e.position, cell_width(window));
    if ui.mouse.reporting {
        if ui.mouse.last != Some(at) {
            ui.mouse.last = Some(at);
            if let Some(bytes) = crate::term::mouse_report(ui.term.mode(), Mouse::Drag(0), at, false, e.modifiers.alt, e.modifiers.control) {
                ui.term.report(bytes);
            }
        }
        return;
    }
    let beyond = ui.beyond(e.position);
    if beyond != 0 {
        ui.term.scroll(-beyond);
    }
    ui.term.select_to(at);
    cx.notify();
}

/// The button let go, wherever it is.
pub fn released(shell: &mut Shell, position: Point<Pixels>, window: &Window, cx: &mut Context<Shell>) {
    let Some(ui) = shell.terminal.as_mut().filter(|t| t.mouse.selecting || t.mouse.reporting) else { return };
    if ui.mouse.reporting {
        let at = ui.at(position, cell_width(window));
        if let Some(bytes) = crate::term::mouse_report(ui.term.mode(), Mouse::Up(0), at, false, false, false) {
            ui.term.report(bytes);
        }
    }
    ui.mouse.selecting = false;
    ui.mouse.reporting = false;
    ui.mouse.last = None;
    cx.notify();
}

/// The size the panel wants now: the shell is told once it settles — not
/// while its edge is dragged or the window resized, each step of which would
/// have it draw its prompt again.
pub fn fit(shell: &mut Shell, window: &Window, cx: &mut Context<Shell>) {
    let room = shell.term_box(window, cx);
    let holding = shell.term_drag.is_some() || shell.side_drag.is_some();
    let Some(ui) = shell.terminal.as_mut().filter(|t| t.open) else { return };
    let wanted = fits(window, room);
    match ui.settle.want(ui.term.size, wanted, holding, std::time::Instant::now()) {
        Settled::Now(size) => ui.term.resize(size),
        Settled::Later(wait) => {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
        Settled::Same | Settled::Held => {}
    }
}

#[cfg(test)]
mod tests {
    use super::fallback_chain;
    use equipe_core::i18n::Lang;

    #[test]
    fn missing_glyphs_come_from_a_nerd_font_then_hangul_and_emoji() {
        let installed: Vec<String> = ["Menlo", "MesloLGS Nerd Font", "MesloLGS Nerd Font Mono", "MesloLGS Nerd Font Propo", "Helvetica"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(fallback_chain(&installed, Lang::Ko), ["MesloLGS Nerd Font Mono", "MesloLGS Nerd Font", "Apple SD Gothic Neo", "Apple Color Emoji", "Apple Symbols"]);
        // The symbols-only one first, when there is one.
        let installed: Vec<String> = ["Symbols Nerd Font Mono", "Hack Nerd Font Mono"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(fallback_chain(&installed, Lang::En)[..2], ["Symbols Nerd Font Mono", "Hack Nerd Font Mono"]);
        // p10k's own, by its old name.
        assert_eq!(fallback_chain(&["MesloLGS NF".to_owned()], Lang::En)[0], "MesloLGS NF");
        // None installed: the Mac's own.
        assert_eq!(fallback_chain(&[], Lang::Ko), ["Apple SD Gothic Neo", "Apple Color Emoji", "Apple Symbols"], "Korean as it always was");
    }

    #[test]
    fn wide_letters_come_from_the_face_of_the_language_in_use() {
        // Every CJK face has kana and Han too: the one for the language first.
        let emoji = ["Apple Color Emoji", "Apple Symbols"];
        assert_eq!(fallback_chain(&[], Lang::Ja), [&["Hiragino Sans", "PingFang SC", "Apple SD Gothic Neo"][..], &emoji[..]].concat());
        assert_eq!(fallback_chain(&[], Lang::ZhHans), [&["PingFang SC", "Apple SD Gothic Neo"][..], &emoji[..]].concat());
        assert_eq!(fallback_chain(&[], Lang::ZhHant), [&["PingFang TC", "Apple SD Gothic Neo"][..], &emoji[..]].concat());
        // Another language: each script to its own — kana and shared Han to Hiragino, Han only Chinese writes to PingFang, Hangul last.
        for lang in [Lang::En, Lang::De, Lang::Fr, Lang::Es] {
            assert_eq!(fallback_chain(&[], lang), [&["Hiragino Sans", "PingFang SC", "Apple SD Gothic Neo"][..], &emoji[..]].concat(), "{lang:?}");
        }
    }
}
