//! A terminal in the window: a shell on the daemon's machine — this Mac's
//! own, or the far one's through the same SSH as the rest — for what only
//! the person may type (`sudo`, a password, a sign-in) without leaving the
//! app. Its grid and escape codes are `alacritty_terminal`'s, as Zed's are;
//! the panel (`views/terminal.rs`) draws what [`Terminal::screen`] gives.
//! No UI types here.

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
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
            let control = launcher.control_dir.join("c-%C");
            let mut args: Vec<String> = ["-t", "-o", "ControlMaster=auto", "-o"].iter().map(|s| (*s).to_owned()).collect();
            args.push(format!("ControlPath={}", control.display()));
            args.extend(["-o".to_owned(), "ControlPersist=600".to_owned()]);
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
        let options = tty::Options { shell: program, working_directory: dir.filter(|d| d.is_dir()).map(Path::to_owned), drain_on_exit: true, env };
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

    pub fn resize(&mut self, size: Size) {
        if size == self.size || size.cols == 0 || size.lines == 0 {
            return;
        }
        self.size = size;
        self.term.lock().resize(size);
        let _ = self.writer.send(Msg::Resize(size.window()));
    }

    pub fn scroll(&self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
    }

    pub fn app_cursor(&self) -> bool {
        self.term.lock().mode().contains(TermMode::APP_CURSOR)
    }

    /// What is on screen, line by line, in runs of one look.
    pub fn screen(&self, palette: &Colors) -> Screen {
        let term = self.term.lock();
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let mut lines: Vec<Vec<Run>> = vec![Vec::new(); self.size.lines as usize];
        for item in content.display_iter {
            let row = item.point.line.0 + offset;
            let Some(line) = usize::try_from(row).ok().and_then(|r| lines.get_mut(r)) else { continue };
            let cell = &item.cell;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let wide = cell.flags.contains(Flags::WIDE_CHAR);
            let inverse = cell.flags.contains(Flags::INVERSE);
            let (mut fg, mut bg) = (palette.of(cell.fg, content.colors), palette.of(cell.bg, content.colors));
            if cell.flags.contains(Flags::DIM) {
                fg = palette.dim(fg);
            }
            if inverse {
                std::mem::swap(&mut fg, &mut bg);
            }
            let look = Look { fg, bg: (bg != palette.background).then_some(bg), bold: cell.flags.contains(Flags::BOLD), italic: cell.flags.contains(Flags::ITALIC), underline: cell.flags.intersects(Flags::ALL_UNDERLINES) };
            let col = item.point.column.0 as u16;
            let ch = if cell.c == '\0' { ' ' } else { cell.c };
            match line.last_mut() {
                Some(run) if run.look == look && run.col + run.cells == col => {
                    run.text.push(ch);
                    run.cells += if wide { 2 } else { 1 };
                }
                _ => line.push(Run { col, cells: if wide { 2 } else { 1 }, text: ch.to_string(), look }),
            }
        }
        let cursor = content.cursor.point;
        let row = cursor.line.0 + offset;
        let hidden = !content.mode.contains(TermMode::SHOW_CURSOR) || offset > 0;
        Screen { lines, cursor: (row >= 0 && !hidden).then_some((cursor.column.0 as u16, row as u16)) }
    }
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

/// Cells side by side that look alike, from column `col`, `cells` wide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub col: u16,
    pub cells: u16,
    pub text: String,
    pub look: Look,
}

pub struct Screen {
    pub lines: Vec<Vec<Run>>,
    /// Where the cursor is (column, line), when it shows.
    pub cursor: Option<(u16, u16)>,
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

    #[test]
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
}
