//! A terminal in the window: a shell on the daemon's machine — this Mac's
//! own, or the far one's through the same SSH as the rest — for what only
//! the person may type (`sudo`, a password, a sign-in) without leaving the
//! app. Its grid and escape codes are `alacritty_terminal`'s, as Zed's are;
//! the panel (`views/terminal.rs`) draws what [`Terminal::screen`] gives.
//! No UI types here.

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::{Flags, LineLength as _};
pub use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::{Config, Term, viewport_to_point};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::target::Endpoint;

/// What the shell did that the window wants to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Happened {
    /// Something new to draw.
    Drew,
    Title(String),
    /// The shell ended.
    Ended,
}

#[derive(Clone)]
struct Listener {
    tx: UnboundedSender<Happened>,
    /// Replies the terminal owes the program (a cursor position asked).
    writer: Arc<std::sync::Mutex<Option<EventLoopSender>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        let said = match event {
            TermEvent::Wakeup => Happened::Drew,
            TermEvent::Title(title) => Happened::Title(title),
            TermEvent::Exit | TermEvent::ChildExit(_) => Happened::Ended,
            TermEvent::PtyWrite(text) => {
                if let Some(writer) = self.writer.lock().unwrap().as_ref() {
                    let _ = writer.send(Msg::Input(Cow::Owned(text.into_bytes())));
                }
                return;
            }
            _ => return,
        };
        let _ = self.tx.send(said);
    }
}

/// How many columns and lines, and how big a cell is in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub lines: u16,
    pub cell_w: u16,
    pub cell_h: u16,
}

impl Size {
    fn window(self) -> WindowSize {
        WindowSize { num_lines: self.lines, num_cols: self.cols, cell_width: self.cell_w, cell_height: self.cell_h }
    }
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines as usize
    }

    fn screen_lines(&self) -> usize {
        self.lines as usize
    }

    fn columns(&self) -> usize {
        self.cols as usize
    }
}

/// The program a terminal runs: the person's shell here, or a login on the
/// far machine through the shared SSH connection, in `dir` either way.
pub fn program(endpoint: &Endpoint, dir: Option<&Path>) -> Option<tty::Shell> {
    match endpoint {
        Endpoint::Local { .. } => None,
        Endpoint::Ssh(host) => {
            let launcher = crate::ssh::Launcher::system(None);
            let mut args = vec!["-t".to_owned()];
            args.extend(launcher.connection_options());
            args.extend(launcher.extra.iter().cloned());
            args.extend(host.args());
            let into = dir.map(|d| format!("cd {} 2>/dev/null; ", crate::ssh::quote(&d.to_string_lossy()))).unwrap_or_default();
            args.push(format!("{into}exec \"${{SHELL:-/bin/sh}}\" -l"));
            Some(tty::Shell::new("ssh".into(), args))
        }
    }
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    writer: EventLoopSender,
    pub size: Size,
    pub title: Option<String>,
    pub ended: bool,
    /// Typed in for the person once the shell has drawn its prompt — never
    /// with Enter: running it is theirs.
    pending: Option<String>,
    drew: bool,
}

impl Terminal {
    /// Start `program` (None: the person's own shell) in `dir`.
    pub fn start(program: Option<tty::Shell>, dir: Option<&Path>, size: Size) -> std::io::Result<(Self, UnboundedReceiver<Happened>)> {
        let mut env = std::collections::HashMap::new();
        env.insert("TERM".to_owned(), "xterm-256color".to_owned());
        env.insert("COLORTERM".to_owned(), "truecolor".to_owned());
        // An app opened from the Dock has no locale: the shell would be in C's,
        // print Hangul as `?` and count a prompt's width in bytes. ssh passes
        // it on to the far machine (`SendEnv LANG LC_*`).
        let var = |name: &str| std::env::var(name).ok();
        if let Some(lang) = utf8_locale(var("LANG").as_deref(), var("LC_ALL").as_deref(), var("LC_CTYPE").as_deref()) {
            env.insert("LANG".to_owned(), lang);
        }
        let options = tty::Options { shell: program, working_directory: dir.filter(|d| d.is_dir()).map(Path::to_owned), drain_on_exit: true, env, ..Default::default() };
        let pty = tty::new(&options, size.window(), 0)?;
        let (tx, rx) = unbounded_channel();
        let writer_slot = Arc::new(std::sync::Mutex::new(None));
        let listener = Listener { tx, writer: writer_slot.clone() };
        let term = Arc::new(FairMutex::new(Term::new(Config::default(), &size, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let writer = event_loop.channel();
        *writer_slot.lock().unwrap() = Some(event_loop.channel());
        event_loop.spawn();
        Ok((Self { term, writer, size, title: None, ended: false, pending: None, drew: false }, rx))
    }

    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let bytes: Vec<u8> = bytes.into();
        if !bytes.is_empty() {
            let _ = self.writer.send(Msg::Input(Cow::Owned(bytes)));
        }
        // Typing brings the view back to the prompt.
        self.term.lock().scroll_display(Scroll::Bottom);
    }

    /// A mouse event's bytes for the program that asked for them: unlike
    /// typing, it leaves the view where it is.
    pub fn report(&self, bytes: Vec<u8>) {
        let _ = self.writer.send(Msg::Input(Cow::Owned(bytes)));
    }

    /// Text pasted: as one block the program can tell from typing, when it asks so.
    pub fn paste(&self, text: &str) {
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        self.write(if bracketed { format!("\x1b[200~{text}\x1b[201~") } else { text });
    }

    /// `line` typed at the prompt, not run: as soon as the shell is ready.
    pub fn suggest(&mut self, line: &str) {
        let line = line.trim().replace('\n', " ");
        if self.drew {
            self.write(line);
        } else {
            self.pending = Some(line);
        }
    }

    /// Something happened; a suggestion waiting for the prompt goes in now.
    pub fn heard(&mut self, happened: &Happened) {
        match happened {
            Happened::Drew if !self.drew => {
                self.drew = true;
                if let Some(line) = self.pending.take() {
                    self.write(line);
                }
            }
            Happened::Title(title) => self.title = Some(title.clone()),
            Happened::Ended => self.ended = true,
            Happened::Drew => {}
        }
    }

    /// The grid as `size` says. Only a change of columns or lines reaches the
    /// shell: each one has it draw its prompt again.
    pub fn resize(&mut self, size: Size) {
        if size.cols == 0 || size.lines == 0 {
            return;
        }
        let changed = (size.cols, size.lines) != (self.size.cols, self.size.lines);
        self.size = size;
        if changed {
            self.term.lock().resize(size);
            let _ = self.writer.send(Msg::Resize(size.window()));
        }
    }

    pub fn scroll(&self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
    }

    pub fn app_cursor(&self) -> bool {
        self.term.lock().mode().contains(TermMode::APP_CURSOR)
    }

    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    /// A press at `at`: a selection begun there, by the character, the word
    /// or the line.
    pub fn select(&self, at: At, pick: Pick) {
        begin_selection(&mut self.term.lock(), at, pick);
    }

    /// The selection's moving end to `at` (a drag, or a shift-click).
    pub fn select_to(&self, at: At) {
        extend_selection(&mut self.term.lock(), at);
    }

    pub fn has_selection(&self) -> bool {
        self.term.lock().selection.as_ref().is_some_and(|s| !s.is_empty())
    }

    pub fn unselect(&self) {
        self.term.lock().selection = None;
    }

    pub fn select_all(&self) {
        select_all(&mut self.term.lock());
    }

    /// The words selected, a wrapped line as one.
    pub fn selected(&self) -> Option<String> {
        selected_text(&self.term.lock())
    }

    /// The web address under `at`, a wrapped one whole.
    pub fn url_at(&self, at: At) -> Option<String> {
        url_at(&self.term.lock(), at)
    }

    /// What is on screen, line by line, in runs of one look.
    pub fn screen(&self, palette: &Colors) -> Screen {
        screen_of(&self.term.lock(), self.size.lines as usize, palette)
    }
}

/// The locale a shell is given when the app has none that reads UTF-8.
pub fn utf8_locale(lang: Option<&str>, lc_all: Option<&str>, lc_ctype: Option<&str>) -> Option<String> {
    let utf8 = |v: Option<&str>| v.is_some_and(|v| v.to_ascii_lowercase().replace('-', "").contains("utf8"));
    if utf8(lc_all) || utf8(lc_ctype) || utf8(lang) {
        return None;
    }
    Some("en_US.UTF-8".to_owned())
}

/// A cell on screen — `col`, `row` from the top of what shows — and which
/// half of it the pointer is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct At {
    pub col: u16,
    pub row: u16,
    pub right: bool,
}

/// The cell under `(x, y)`, measured from the grid's top left, in cells
/// `cell_w` × `cell_h` of a grid `cols` × `lines`: outside it, the nearest.
pub fn cell_at(x: f32, y: f32, cell_w: f32, cell_h: f32, cols: u16, lines: u16) -> At {
    let (cols, lines) = (cols.max(1), lines.max(1));
    let x = x.max(0.);
    let col = ((x / cell_w).floor() as i64).clamp(0, i64::from(cols) - 1) as u16;
    let row = ((y / cell_h).floor() as i64).clamp(0, i64::from(lines) - 1) as u16;
    let right = x >= (f32::from(col) + 0.5) * cell_w;
    At { col, row, right }
}

/// What one, two or three clicks select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Chars,
    Word,
    Line,
}

impl Pick {
    pub fn of_clicks(clicks: usize) -> Self {
        match clicks {
            0 | 1 => Pick::Chars,
            2 => Pick::Word,
            _ => Pick::Line,
        }
    }
}

fn grid_point<T>(term: &Term<T>, at: At) -> (Point, Side) {
    let offset = term.grid().display_offset();
    let point = viewport_to_point(offset, Point::new(at.row as usize, Column(at.col as usize)));
    let point = Point::new(point.line.max(term.topmost_line()).min(term.bottommost_line()), point.column.min(term.last_column()));
    (point, if at.right { Side::Right } else { Side::Left })
}

pub fn begin_selection<T>(term: &mut Term<T>, at: At, pick: Pick) {
    let (point, side) = grid_point(term, at);
    let ty = match pick {
        Pick::Chars => SelectionType::Simple,
        Pick::Word => SelectionType::Semantic,
        Pick::Line => SelectionType::Lines,
    };
    term.selection = Some(Selection::new(ty, point, side));
}

pub fn extend_selection<T>(term: &mut Term<T>, at: At) {
    let (point, side) = grid_point(term, at);
    if let Some(selection) = term.selection.as_mut() {
        selection.update(point, side);
    }
}

/// Everything, from the oldest line kept to the last one written.
pub fn select_all<T>(term: &mut Term<T>) {
    let cursor = term.grid().cursor.point.line;
    let bottom = term.bottommost_line();
    let mut last = cursor;
    let mut line = bottom;
    while line > cursor {
        if term.grid()[line].line_length().0 > 0 {
            last = line;
            break;
        }
        line -= 1;
    }
    let mut selection = Selection::new(SelectionType::Simple, Point::new(term.topmost_line(), Column(0)), Side::Left);
    selection.update(Point::new(last, term.last_column()), Side::Right);
    term.selection = Some(selection);
}

/// The selection's text: a line the terminal wrapped joins the next without a
/// break, a wide character is itself once.
pub fn selected_text<T>(term: &Term<T>) -> Option<String> {
    term.selection_to_string().filter(|s| !s.is_empty())
}

/// The web address under `at`: the logical line around it (rows the
/// terminal wrapped, joined), cut at spaces and quotes.
pub fn url_at<T>(term: &Term<T>, at: At) -> Option<String> {
    let (point, _) = grid_point(term, at);
    let grid = term.grid();
    let last_col = term.last_column();
    let wrapped = |line: Line| grid[line][last_col].flags.contains(Flags::WRAPLINE);
    let mut first = point.line;
    while first > term.topmost_line() && wrapped(first - 1) {
        first -= 1;
    }
    let mut last = point.line;
    while last < term.bottommost_line() && wrapped(last) {
        last += 1;
    }
    let mut text = Vec::new();
    let mut here = None;
    let mut line = first;
    while line <= last {
        for col in 0..term.columns() {
            let cell = &grid[line][Column(col)];
            let spacer = cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
            if line == point.line && col == point.column.0 {
                here = Some(if spacer { text.len().saturating_sub(1) } else { text.len() });
            }
            if !spacer {
                text.push(if cell.c == '\0' { ' ' } else { cell.c });
            }
        }
        line += 1;
    }
    url_in(&text, here?)
}

/// The http(s) address that the character `at` of `text` is part of.
pub fn url_in(text: &[char], at: usize) -> Option<String> {
    let stop = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>' | '│' | '|');
    if at >= text.len() || stop(text[at]) {
        return None;
    }
    let mut from = at;
    while from > 0 && !stop(text[from - 1]) {
        from -= 1;
    }
    let mut to = at + 1;
    while to < text.len() && !stop(text[to]) {
        to += 1;
    }
    let word: String = text[from..to].iter().collect();
    let start = ["https://", "http://"].iter().filter_map(|s| word.find(s)).min()?;
    // Clicked before the address began (a bracket, a label): not on it.
    if word[..start].chars().count() > at - from {
        return None;
    }
    let mut url = &word[start..];
    loop {
        let trimmed = url.trim_end_matches(['.', ',', ';', ':', '!', '?', ']', '}']);
        let trimmed = if trimmed.ends_with(')') && trimmed.matches('(').count() < trimmed.matches(')').count() { &trimmed[..trimmed.len() - 1] } else { trimmed };
        if trimmed == url {
            break;
        }
        url = trimmed;
    }
    (url.len() > "https://".len()).then(|| url.to_owned())
}

/// The bytes a mouse event sends a program that asked for it (`mode`), at
/// cell `at`; `None`: the program didn't ask for this one.
pub fn mouse_report(mode: TermMode, event: Mouse, at: At, shift: bool, alt: bool, ctrl: bool) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return None;
    }
    let (button, release, motion) = match event {
        Mouse::Down(b) => (b, false, false),
        Mouse::Up(b) => (b, true, false),
        Mouse::Drag(b) if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) => (b, false, true),
        Mouse::Drag(_) => return None,
        Mouse::WheelUp => (64, false, false),
        Mouse::WheelDown => (65, false, false),
    };
    let mods = 4 * u8::from(shift) + 8 * u8::from(alt) + 16 * u8::from(ctrl);
    let code = button + mods + if motion { 32 } else { 0 };
    let (col, row) = (u32::from(at.col) + 1, u32::from(at.row) + 1);
    if mode.contains(TermMode::SGR_MOUSE) {
        return Some(format!("\x1b[<{code};{col};{row}{}", if release { 'm' } else { 'M' }).into_bytes());
    }
    // The old encoding has no button for a release: 3, whichever it was.
    let code = if release { 3 + mods } else { code };
    let mut bytes = b"\x1b[M".to_vec();
    bytes.push(32 + code);
    for v in [col, row] {
        if mode.contains(TermMode::UTF8_MOUSE) {
            let c = char::from_u32(32 + v)?;
            let mut buf = [0; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        } else {
            bytes.push(u8::try_from(32 + v).ok()?);
        }
    }
    Some(bytes)
}

/// A mouse event a program may ask for: buttons 0 (left), 1 (middle), 2 (right).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mouse {
    Down(u8),
    Up(u8),
    Drag(u8),
    WheelUp,
    WheelDown,
}

/// Columns and lines that fit `wide` × `tall` points of cells `cell_w` × `cell_h`.
pub fn grid_fit(wide: f32, tall: f32, cell_w: f32, cell_h: f32) -> (u16, u16) {
    // A hair of slack: a width that is exactly n cells is n, not n − 1.
    let n = |room: f32, cell: f32| ((room + 0.01) / cell.max(1.)).floor().clamp(2., 1000.) as u16;
    (n(wide, cell_w), n(tall, cell_h))
}

/// The grid's new size, once the panel has stopped changing: while its edge
/// is held, or the window is being resized, the shell isn't told each step —
/// every one would have it draw its prompt again.
#[derive(Debug, Default)]
pub struct Settle {
    pending: Option<(Size, Instant)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// Nothing to do.
    Same,
    /// The edge is held: once let go.
    Held,
    /// Ask again after this long.
    Later(Duration),
    /// Resize now.
    Now(Size),
}

impl Settle {
    pub const QUIET: Duration = Duration::from_millis(150);

    pub fn want(&mut self, current: Size, wanted: Size, holding: bool, now: Instant) -> Settled {
        if (wanted.cols, wanted.lines) == (current.cols, current.lines) {
            self.pending = None;
            return Settled::Same;
        }
        let since = match self.pending {
            Some((size, at)) if size == wanted => now.saturating_duration_since(at),
            _ => {
                self.pending = Some((wanted, now));
                Duration::ZERO
            }
        };
        if holding {
            return Settled::Held;
        }
        if since >= Self::QUIET {
            self.pending = None;
            return Settled::Now(wanted);
        }
        Settled::Later(Self::QUIET - since)
    }
}

/// What is on screen, line by line, in runs of one look; a wide character is
/// a run of its own, two cells wide.
pub fn screen_of<T: EventListener>(term: &Term<T>, lines: usize, palette: &Colors) -> Screen {
    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    let mut rows: Vec<Vec<Run>> = vec![Vec::new(); lines];
    let mut selected: Vec<Vec<(u16, u16)>> = vec![Vec::new(); lines];
    let mut cursor_wide = false;
    let cursor = content.cursor.point;
    for item in content.display_iter {
        let row = item.point.line.0 + offset;
        let Some(r) = usize::try_from(row).ok().filter(|r| *r < lines) else { continue };
        let cell = &item.cell;
        let col = item.point.column.0 as u16;
        let wide = cell.flags.contains(Flags::WIDE_CHAR);
        if let Some(range) = content.selection {
            let picked = range.contains(item.point) || (wide && range.contains(Point::new(item.point.line, item.point.column + 1)));
            if picked {
                let span = &mut selected[r];
                let cells = if wide { 2 } else { 1 };
                match span.last_mut() {
                    Some((_, to)) if *to + 1 >= col => *to = (*to).max(col + cells - 1),
                    _ => span.push((col, col + cells - 1)),
                }
            }
        }
        if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        if item.point == cursor {
            cursor_wide = wide;
        }
        let (mut fg, mut bg) = (palette.of(cell.fg, content.colors), palette.of(cell.bg, content.colors));
        if cell.flags.contains(Flags::DIM) {
            fg = palette.dim(fg);
        }
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        let look = Look { fg, bg: (bg != palette.background).then_some(bg), bold: cell.flags.contains(Flags::BOLD), italic: cell.flags.contains(Flags::ITALIC), underline: cell.flags.intersects(Flags::ALL_UNDERLINES) };
        let mut text = String::from(if cell.c == '\0' { ' ' } else { cell.c });
        text.extend(cell.zerowidth().into_iter().flatten());
        let line = &mut rows[r];
        match line.last_mut() {
            Some(run) if !wide && !run.wide && run.look == look && run.col + run.cells == col => {
                run.text.push_str(&text);
                run.cells += 1;
            }
            _ => line.push(Run { col, cells: if wide { 2 } else { 1 }, text, look, wide }),
        }
    }
    // The blank rest of a line: nothing to draw.
    for line in &mut rows {
        if let Some(run) = line.last_mut().filter(|r| !r.wide && r.look.bg.is_none() && !r.look.underline) {
            let kept = run.text.trim_end_matches(' ').len();
            run.cells -= (run.text.len() - kept) as u16;
            run.text.truncate(kept);
            if run.text.is_empty() {
                line.pop();
            }
        }
    }
    let row = cursor.line.0 + offset;
    let hidden = !content.mode.contains(TermMode::SHOW_CURSOR) || offset > 0;
    let cursor = (row >= 0 && (row as usize) < lines && !hidden).then_some(Cursor { col: cursor.column.0 as u16, row: row as u16, wide: cursor_wide });
    Screen { lines: rows, cursor, selected }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.writer.send(Msg::Shutdown);
    }
}

/// How a run of cells looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    pub fg: u32,
    /// `None`: the panel's own background.
    pub bg: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// Cells side by side that look alike, from column `col`, `cells` wide: one
/// character each, or one wide one (Hangul, most emoji) over two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub col: u16,
    pub cells: u16,
    pub text: String,
    pub look: Look,
    pub wide: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub col: u16,
    pub row: u16,
    /// On a wide character: two cells.
    pub wide: bool,
}

pub struct Screen {
    pub lines: Vec<Vec<Run>>,
    /// Where the cursor is, when it shows.
    pub cursor: Option<Cursor>,
    /// Each line's selected cells, as spans of columns (first, last).
    pub selected: Vec<Vec<(u16, u16)>>,
}

/// The sixteen colours and the panel's own, as RGB.
#[derive(Debug, Clone)]
pub struct Colors {
    pub foreground: u32,
    pub background: u32,
    pub ansi: [u32; 16],
}

impl Colors {
    pub fn dark() -> Self {
        Self {
            foreground: 0xD8D8DC,
            background: 0x0E0E10,
            ansi: [
                0x1C1C1F, 0xEC8A8A, 0x8CD096, 0xE8BE78, 0x8DB8E8, 0xB6A2F2, 0x7CC8C8, 0xC8C8CC, 0x5A5A60, 0xF2A0A0, 0xA4DCAC, 0xF0CE90, 0xA6C8F0, 0xC8B8F6, 0x96D6D6, 0xF2F2F4,
            ],
        }
    }

    pub fn light() -> Self {
        Self {
            foreground: 0x1E1E22,
            background: 0xFBFBFC,
            ansi: [
                0x1E1E22, 0xB0323A, 0x28803C, 0x9A6410, 0x1E68AA, 0x6446BE, 0x1C8080, 0x6E6E74, 0x8A8A90, 0xC8424A, 0x34944A, 0xB07818, 0x2E7CC0, 0x7656D0, 0x249494, 0x3A3A40,
            ],
        }
    }

    fn of(&self, color: Color, set: &alacritty_terminal::term::color::Colors) -> u32 {
        let rgb = |c: Rgb| (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b);
        match color {
            Color::Spec(c) => rgb(c),
            Color::Indexed(i) => set[i as usize].map(rgb).unwrap_or_else(|| self.indexed(i)),
            Color::Named(name) => set[name].map(rgb).unwrap_or_else(|| self.named(name)),
        }
    }

    fn named(&self, name: NamedColor) -> u32 {
        match name {
            NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => self.foreground,
            NamedColor::Background => self.background,
            NamedColor::DimForeground => self.dim(self.foreground),
            other => {
                let n = other as usize;
                if n < 16 { self.ansi[n] } else if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&n) { self.dim(self.ansi[n - NamedColor::DimBlack as usize]) } else { self.foreground }
            }
        }
    }

    /// The 256 colours: the sixteen, a 6×6×6 cube, a ramp of greys.
    fn indexed(&self, i: u8) -> u32 {
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let i = i - 16;
                let step = |v: u8| if v == 0 { 0 } else { 55 + 40 * u32::from(v) };
                (step(i / 36) << 16) | (step((i / 6) % 6) << 8) | step(i % 6)
            }
            _ => {
                let v = 8 + 10 * u32::from(i - 232);
                (v << 16) | (v << 8) | v
            }
        }
    }

    fn dim(&self, c: u32) -> u32 {
        let mix = |a: u32, b: u32| (a * 2 + b) / 3;
        let (r, g, b) = (c >> 16 & 0xFF, c >> 8 & 0xFF, c & 0xFF);
        let (br, bg, bb) = (self.background >> 16 & 0xFF, self.background >> 8 & 0xFF, self.background & 0xFF);
        (mix(r, br) << 16) | (mix(g, bg) << 8) | mix(b, bb)
    }
}

/// The bytes a key sends, as a terminal sends them. `None`: not the
/// terminal's (⌘ shortcuts stay the window's).
pub fn key_bytes(key: &str, ch: Option<&str>, ctrl: bool, alt: bool, cmd: bool, shift: bool, app_cursor: bool) -> Option<Vec<u8>> {
    if cmd {
        return None;
    }
    let arrow = |c: char| if app_cursor { format!("\x1bO{c}") } else { format!("\x1b[{c}") };
    let named: Option<String> = match key {
        "enter" => Some("\r".into()),
        "backspace" => Some(if alt { "\x1b\x7f".into() } else { "\x7f".into() }),
        "tab" if shift => Some("\x1b[Z".into()),
        "tab" => Some("\t".into()),
        "escape" => Some("\x1b".into()),
        "up" => Some(arrow('A')),
        "down" => Some(arrow('B')),
        "right" if alt => Some("\x1bf".into()),
        "left" if alt => Some("\x1bb".into()),
        "right" => Some(arrow('C')),
        "left" => Some(arrow('D')),
        "home" => Some("\x1b[H".into()),
        "end" => Some("\x1b[F".into()),
        "pageup" => Some("\x1b[5~".into()),
        "pagedown" => Some("\x1b[6~".into()),
        "delete" => Some("\x1b[3~".into()),
        "space" if ctrl => Some("\0".into()),
        _ => None,
    };
    if let Some(named) = named {
        return Some(named.into_bytes());
    }
    if ctrl {
        let c = key.chars().next().filter(|_| key.chars().count() == 1)?.to_ascii_lowercase();
        let byte = match c {
            'a'..='z' => c as u8 - b'a' + 1,
            '[' => 0x1b,
            '\\' => 0x1c,
            ']' => 0x1d,
            '^' | '6' => 0x1e,
            '_' | '-' => 0x1f,
            '@' | '2' => 0,
            _ => return None,
        };
        return Some(if alt { vec![0x1b, byte] } else { vec![byte] });
    }
    let text = ch.filter(|c| !c.is_empty())?;
    let mut bytes = if alt { vec![0x1b] } else { Vec::new() };
    bytes.extend_from_slice(text.as_bytes());
    Some(bytes)
}

/// A shell code block's command, for the terminal: its lines joined, the
/// prompt marks people paste with taken off.
pub fn command_of(code: &str) -> String {
    code.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.strip_prefix("$ ").unwrap_or(l))
        .collect::<Vec<_>>()
        .join(" && ")
}

/// Commands only the person can see through: a sign-in that shows a code or
/// opens a page, a password, a key of their own.
const THEIRS: &[&str] = &[
    "gh auth login", "gh auth refresh", "aws sso login", "aws configure", "gcloud auth", "gcloud init", "npm login", "npm adduser", "pnpm login",
    "yarn npm login", "docker login", "firebase login", "vercel login", "netlify login", "supabase login", "fly auth login", "flyctl auth login",
    "heroku login", "az login", "op signin", "codex login", "claude login", "wrangler login", "eas login", "expo login", "ssh-keygen", "ssh-copy-id",
    "passwd", "security unlock-keychain", "notarytool store-credentials", "gpg --full-generate-key",
];

/// Whether a reply's shell block is one for the person to run themselves:
/// it asks for the machine's administrator, or signs them in somewhere.
pub fn wants_person(code: &str) -> bool {
    let words: Vec<&str> = code.split(|c: char| c.is_whitespace() || c == ';' || c == '&' || c == '|').filter(|w| !w.is_empty()).collect();
    let joined = words.join(" ");
    words.contains(&"sudo") || THEIRS.iter().any(|t| joined.contains(t))
}

/// The shell blocks in `markdown`: their code, in order.
pub fn shell_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut lines = markdown.lines();
    while let Some(line) = lines.next() {
        let fence = line.trim_start();
        let Some(lang) = fence.strip_prefix("```") else { continue };
        let shell = matches!(lang.trim(), "" | "sh" | "bash" | "zsh" | "shell" | "console" | "fish");
        let mut code = Vec::new();
        for inner in lines.by_ref() {
            if inner.trim_start().starts_with("```") {
                break;
            }
            code.push(inner);
        }
        if shell {
            blocks.push(code.join("\n"));
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_send_what_a_terminal_sends() {
        let k = |key: &str, ch: Option<&str>, ctrl, alt| key_bytes(key, ch, ctrl, alt, false, false, false);
        assert_eq!(k("enter", None, false, false), Some(b"\r".to_vec()));
        assert_eq!(k("c", Some("c"), true, false), Some(vec![3]));
        assert_eq!(k("a", Some("a"), false, false), Some(b"a".to_vec()));
        assert_eq!(k("ㅎ", Some("한"), false, false), Some("한".as_bytes().to_vec()));
        assert_eq!(k("up", None, false, false), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes("up", None, false, false, false, false, true), Some(b"\x1bOA".to_vec()), "a full-screen program's arrows");
        assert_eq!(k("b", Some("b"), false, true), Some(b"\x1bb".to_vec()));
        assert_eq!(key_bytes("v", Some("v"), false, false, true, false, false), None, "⌘V is the window's");
    }

    #[test]
    fn a_replys_admin_command_is_found_and_joined() {
        let reply = "이렇게 하세요.\n\n```bash\n# 잠들지 않게\n$ sudo pmset -a sleep 0\nsudo pmset -a womp 1\n```\n\n```rust\nfn main() {}\n```";
        let blocks = shell_blocks(reply);
        assert_eq!(blocks.len(), 1);
        assert!(wants_person(&blocks[0]));
        assert_eq!(command_of(&blocks[0]), "sudo pmset -a sleep 0 && sudo pmset -a womp 1");
        assert!(!wants_person("echo pseudo"));
        assert!(wants_person("gh  auth login --web"), "a sign-in that shows a code");
        assert!(wants_person("cd ~/app && npx vercel login"));
        assert!(!wants_person("gh pr list"));
    }

    #[test]
    fn the_256_colours_are_the_usual_ones() {
        let c = Colors::dark();
        assert_eq!(c.indexed(16), 0x000000);
        assert_eq!(c.indexed(231), 0xFFFFFF);
        assert_eq!(c.indexed(232), 0x080808);
        assert_eq!(c.indexed(1), c.ansi[1]);
    }

    /// A grid `cols` × `lines` that has been sent `bytes`, as from a program.
    fn grid(cols: u16, lines: u16, bytes: &str) -> Term<alacritty_terminal::event::VoidListener> {
        let size = Size { cols, lines, cell_w: 8, cell_h: 16 };
        let mut term = Term::new(Config::default(), &size, alacritty_terminal::event::VoidListener);
        let mut parser: alacritty_terminal::vte::ansi::Processor = alacritty_terminal::vte::ansi::Processor::new();
        parser.advance(&mut term, bytes.as_bytes());
        term
    }

    fn at(col: u16, row: u16) -> At {
        At { col, row, right: false }
    }

    fn drag(term: &mut Term<alacritty_terminal::event::VoidListener>, from: At, to: At) -> Option<String> {
        begin_selection(term, from, Pick::Chars);
        extend_selection(term, to);
        selected_text(term)
    }

    #[test]
    fn a_wrapped_url_copies_as_one_line() {
        let url = "https://claude.ai/oauth/authorize?code=true&client_id=9d1c250a&state=abc";
        let mut term = grid(20, 8, &format!("open:\r\n{url}\r\n$ "));
        // Over four rows on screen…
        let screen = screen_of(&term, 8, &Colors::dark());
        assert_eq!(screen.lines[1][0].text, &url[..20]);
        // …and one line copied, from its first cell to its last.
        let last_row = 1 + (url.len() as u16 - 1) / 20;
        let last_col = (url.len() as u16 - 1) % 20;
        assert_eq!(drag(&mut term, at(0, 1), At { col: last_col, row: last_row, right: true }).as_deref(), Some(url));
        // A ⌘-click anywhere on it opens all of it.
        assert_eq!(url_at(&term, at(5, 3)).as_deref(), Some(url));
        assert_eq!(url_at(&term, at(2, 0)), None, "the line before");
    }

    #[test]
    fn lines_the_program_broke_keep_their_break() {
        let mut term = grid(20, 6, "one\r\ntwo\r\n");
        assert_eq!(drag(&mut term, at(0, 0), At { col: 2, row: 1, right: true }).as_deref(), Some("one\ntwo"));
    }

    #[test]
    fn wide_characters_take_two_cells_and_copy_once() {
        let mut term = grid(20, 4, "a한글b 🚀!\r\n");
        let screen = screen_of(&term, 4, &Colors::dark());
        let runs: Vec<(u16, u16, &str, bool)> = screen.lines[0].iter().map(|r| (r.col, r.cells, r.text.as_str(), r.wide)).collect();
        assert_eq!(runs, vec![(0, 1, "a", false), (1, 2, "한", true), (3, 2, "글", true), (5, 2, "b ", false), (7, 2, "🚀", true), (9, 1, "!", false)]);
        // Started on a wide character's right half, it is whole.
        assert_eq!(drag(&mut term, at(2, 0), At { col: 5, row: 0, right: true }).as_deref(), Some("한글b"));
        // The selection drawn covers both its cells.
        let screen = screen_of(&term, 4, &Colors::dark());
        assert_eq!(screen.selected[0], vec![(1, 5)]);
    }

    #[test]
    fn the_cursor_on_a_wide_character_is_two_cells() {
        let term = grid(20, 4, "x한\x1b[2D");
        let cursor = screen_of(&term, 4, &Colors::dark()).cursor.unwrap();
        assert_eq!((cursor.col, cursor.row, cursor.wide), (1, 0, true));
    }

    #[test]
    fn a_double_click_takes_a_word_and_a_triple_one_the_line() {
        let mut term = grid(30, 4, "cd ~/projects/equipe now\r\n");
        begin_selection(&mut term, at(6, 0), Pick::Word);
        assert_eq!(selected_text(&term).as_deref(), Some("~/projects/equipe"));
        begin_selection(&mut term, at(6, 0), Pick::Line);
        assert_eq!(selected_text(&term).as_deref(), Some("cd ~/projects/equipe now\n"));
        // A shift-click moves the end.
        begin_selection(&mut term, at(0, 0), Pick::Chars);
        extend_selection(&mut term, At { col: 1, row: 0, right: true });
        assert_eq!(selected_text(&term).as_deref(), Some("cd"));
        // A click without a drag selects nothing.
        begin_selection(&mut term, at(4, 0), Pick::Chars);
        assert_eq!(selected_text(&term), None);
    }

    #[test]
    fn select_all_ends_at_the_last_line_written() {
        let mut term = grid(20, 6, "first\r\nsecond\r\n$ ");
        select_all(&mut term);
        assert_eq!(selected_text(&term).as_deref(), Some("first\nsecond\n$"));
    }

    #[test]
    fn a_url_is_found_where_clicked_and_trimmed() {
        let chars = |s: &str| s.chars().collect::<Vec<_>>();
        let text = chars("see (https://example.com/a_(b)) now.");
        assert_eq!(url_in(&text, 10).as_deref(), Some("https://example.com/a_(b)"));
        assert_eq!(url_in(&text, 4), None, "the bracket before it");
        assert_eq!(url_in(&text, 33), None);
        let text = chars("go https://x.io/login.");
        assert_eq!(url_in(&text, 3).as_deref(), Some("https://x.io/login"));
        assert_eq!(url_in(&chars("https://"), 2), None);
    }

    #[test]
    fn a_point_is_the_cell_under_it() {
        assert_eq!(cell_at(0., 0., 7.5, 17., 80, 24), At { col: 0, row: 0, right: false });
        assert_eq!(cell_at(11.3, 17.5, 7.5, 17., 80, 24), At { col: 1, row: 1, right: true });
        // Past the edges: the nearest cell.
        assert_eq!(cell_at(-4., -9., 7.5, 17., 80, 24), At { col: 0, row: 0, right: false });
        assert_eq!(cell_at(9000., 9000., 7.5, 17., 80, 24), At { col: 79, row: 23, right: true });
    }

    #[test]
    fn the_grid_is_counted_at_the_width_it_is_drawn() {
        // SF Mono's 12.5 pt advance: 7.5 points, not 8 rounded.
        assert_eq!(grid_fit(600., 340., 7.5, 17.), (80, 20));
        assert_eq!(grid_fit(599.9, 339.9, 7.5, 17.), (79, 19));
        assert_eq!(grid_fit(5., 5., 7.5, 17.), (2, 2));
    }

    #[test]
    fn the_shell_hears_a_new_size_once_it_settles() {
        let size = |cols| Size { cols, lines: 24, cell_w: 8, cell_h: 17 };
        let t0 = Instant::now();
        let mut settle = Settle::default();
        assert_eq!(settle.want(size(80), size(80), false, t0), Settled::Same);
        // Dragged: nothing while held, whatever the size passes through…
        assert_eq!(settle.want(size(80), size(70), true, t0), Settled::Held);
        assert_eq!(settle.want(size(80), size(60), true, t0 + Duration::from_millis(40)), Settled::Held);
        // …then the last one, let go and quiet.
        assert_eq!(settle.want(size(80), size(60), false, t0 + Duration::from_millis(60)), Settled::Later(Duration::from_millis(130)));
        assert_eq!(settle.want(size(80), size(60), false, t0 + Duration::from_millis(200)), Settled::Now(size(60)));
        // A window resized: each new size waits again.
        assert_eq!(settle.want(size(60), size(61), false, t0), Settled::Later(Settle::QUIET));
        assert_eq!(settle.want(size(60), size(62), false, t0 + Duration::from_millis(100)), Settled::Later(Settle::QUIET));
        assert_eq!(settle.want(size(60), size(62), false, t0 + Duration::from_millis(260)), Settled::Now(size(62)));
        // Back to what it was: nothing to tell.
        assert_eq!(settle.want(size(62), size(62), false, t0 + Duration::from_millis(300)), Settled::Same);
    }

    #[test]
    fn a_shell_with_no_locale_gets_a_utf8_one() {
        assert_eq!(utf8_locale(None, None, None).as_deref(), Some("en_US.UTF-8"));
        assert_eq!(utf8_locale(Some("C"), None, None).as_deref(), Some("en_US.UTF-8"));
        assert_eq!(utf8_locale(Some("ko_KR.UTF-8"), None, None), None);
        assert_eq!(utf8_locale(None, None, Some("UTF-8")), None);
        assert_eq!(utf8_locale(None, Some("en_US.utf8"), None), None);
    }

    #[test]
    fn mouse_reports_go_to_programs_that_ask() {
        let a = At { col: 4, row: 2, right: false };
        assert_eq!(mouse_report(TermMode::empty(), Mouse::Down(0), a, false, false, false), None, "nobody asked");
        let sgr = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(mouse_report(sgr, Mouse::Down(0), a, false, false, false), Some(b"\x1b[<0;5;3M".to_vec()));
        assert_eq!(mouse_report(sgr, Mouse::Up(0), a, false, false, false), Some(b"\x1b[<0;5;3m".to_vec()));
        assert_eq!(mouse_report(sgr, Mouse::Drag(0), a, false, false, false), None, "clicks only");
        assert_eq!(mouse_report(sgr | TermMode::MOUSE_DRAG, Mouse::Drag(0), a, false, false, true), Some(b"\x1b[<48;5;3M".to_vec()));
        assert_eq!(mouse_report(sgr, Mouse::WheelUp, a, false, false, false), Some(b"\x1b[<64;5;3M".to_vec()));
        let old = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(mouse_report(old, Mouse::Down(0), a, false, false, false), Some(vec![0x1b, b'[', b'M', 32, 37, 35]));
        assert_eq!(mouse_report(old, Mouse::Up(0), a, false, false, false), Some(vec![0x1b, b'[', b'M', 35, 37, 35]));
        assert_eq!(mouse_report(old, Mouse::Down(0), At { col: 300, row: 0, right: false }, false, false, false), None, "too far for the old encoding");
    }

    #[test]
    #[cfg(unix)]
    fn a_shell_starts_draws_and_echoes() {
        let size = Size { cols: 40, lines: 6, cell_w: 8, cell_h: 16 };
        let (mut term, mut rx) = Terminal::start(Some(tty::Shell::new("/bin/sh".into(), vec![])), None, size).unwrap();
        term.write("echo equipe-$((40+2))\r");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let colors = Colors::dark();
        loop {
            while let Ok(h) = rx.try_recv() {
                term.heard(&h);
            }
            let text: String = term.screen(&colors).lines.iter().map(|l| l.iter().map(|r| r.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n");
            if text.contains("equipe-42") {
                break;
            }
            assert!(std::time::Instant::now() < until, "{text}");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        term.resize(Size { cols: 60, ..size });
        assert_eq!(term.size.cols, 60);
    }

    #[test]
    #[cfg(windows)]
    fn conpty_starts_powershell_draws_and_echoes() {
        let size = Size {
            cols: 50,
            lines: 8,
            cell_w: 8,
            cell_h: 16,
        };
        let shell = tty::Shell::new(
            "powershell.exe".into(),
            vec!["-NoLogo".into(), "-NoProfile".into()],
        );
        let (mut term, mut rx) = Terminal::start(Some(shell), None, size).unwrap();
        term.write("$v='equipe-conpty'; Write-Output ($v + '-42')\r");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let colors = Colors::dark();
        loop {
            while let Ok(happened) = rx.try_recv() {
                term.heard(&happened);
            }
            let text = term
                .screen(&colors)
                .lines
                .iter()
                .map(|line| line.iter().map(|run| run.text.as_str()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            if text.contains("equipe-conpty-42") {
                break;
            }
            assert!(std::time::Instant::now() < until, "{text}");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}
