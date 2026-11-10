//! Terminal presentation, following docs/BRAND.md: monochrome, one cool accent,
//! uppercase section labels, no emoji. Weight and dimming carry the hierarchy;
//! the accent marks only what succeeded.
//!
//! Plain text when the stream isn't a terminal, when `NO_COLOR` is set, or on a
//! dumb terminal, so piped output stays clean.

use std::io::IsTerminal as _;
use std::sync::OnceLock;

/// Whether a person is at the keyboard: both stdin and stderr are terminals.
pub fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// The selection-menu theme: the item under the cursor in bold with the accent
/// marker, the rest dimmed — the same hierarchy as the rest of the CLI.
pub fn select_theme() -> Box<dyn dialoguer::theme::Theme> {
    use console::{Style, style};
    use dialoguer::theme::{ColorfulTheme, SimpleTheme};

    if !err().color {
        return Box::new(SimpleTheme);
    }
    let accent = if err().truecolor { Style::new().true_color(180, 205, 230) } else { Style::new().color256(152) };
    Box::new(ColorfulTheme {
        prompt_prefix: style("?".to_owned()).for_stderr().dim(),
        prompt_style: Style::new().bold(),
        prompt_suffix: style(String::new()),
        success_prefix: accent.clone().for_stderr().apply_to("✓".to_owned()),
        success_suffix: style("·".to_owned()).for_stderr().dim(),
        values_style: Style::new().for_stderr().bold(),
        checked_item_prefix: accent.clone().for_stderr().apply_to("✓".to_owned()),
        unchecked_item_prefix: style("○".to_owned()).for_stderr().dim(),
        active_item_prefix: accent.for_stderr().apply_to("›".to_owned()),
        inactive_item_prefix: style(" ".to_owned()),
        active_item_style: Style::new().for_stderr().bold(),
        inactive_item_style: Style::new().for_stderr(),
        ..ColorfulTheme::default()
    })
}

/// Columns available, from whichever stream is a terminal; 80 when unknown.
pub fn width() -> usize {
    static W: OnceLock<usize> = OnceLock::new();
    *W.get_or_init(|| {
        terminal_size::terminal_size_of(std::io::stdout())
            .or_else(|| terminal_size::terminal_size_of(std::io::stderr()))
            .map(|(terminal_size::Width(w), _)| w as usize)
            .unwrap_or(80)
    })
}

/// Columns the text occupies, skipping ANSI style sequences. Wide characters —
/// Hangul, CJK, most emoji — take two columns, as the terminal draws them.
fn visible_len(text: &str) -> usize {
    use unicode_width::UnicodeWidthChar as _;
    let mut len = 0;
    let mut in_escape = false;
    for c in text.chars() {
        match (in_escape, c) {
            (false, '\x1b') => in_escape = true,
            (true, 'm') => in_escape = false,
            (true, _) => {}
            (false, _) => len += c.width().unwrap_or(0),
        }
    }
    len
}

/// `text` cut to `width` columns, ending in `…` when cut — for lines that must
/// stay on one row, like menu items a menu redraws in place.
/// Text with its enclosed numbers and letters (①, ⑵, ❸, Ⓐ) spelled out —
/// `(1)`, `(A)`: fonts draw them wider than the one cell a terminal gives
/// them, over the character after.
pub fn cell_safe(text: &str) -> std::borrow::Cow<'_, str> {
    let enclosed = |c: char| -> Option<String> {
        let n = c as u32;
        Some(match n {
            0x2460..=0x2473 => format!("({})", n - 0x2460 + 1),
            0x2474..=0x2487 => format!("({})", n - 0x2474 + 1),
            0x2488..=0x249B => format!("{}.", n - 0x2488 + 1),
            0x24B6..=0x24CF => format!("({})", char::from_u32(u32::from(b'A') + n - 0x24B6)?),
            0x24D0..=0x24E9 => format!("({})", char::from_u32(u32::from(b'a') + n - 0x24D0)?),
            0x24EA => "(0)".to_owned(),
            0x2776..=0x277F => format!("({})", n - 0x2776 + 1),
            0x2780..=0x2789 => format!("({})", n - 0x2780 + 1),
            0x278A..=0x2793 => format!("({})", n - 0x278A + 1),
            _ => return None,
        })
    };
    if !text.chars().any(|c| enclosed(c).is_some()) {
        return std::borrow::Cow::Borrowed(text);
    }
    std::borrow::Cow::Owned(text.chars().map(|c| enclosed(c).unwrap_or_else(|| c.to_string())).collect())
}

pub fn fit(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar as _;
    let text = &*cell_safe(text);
    if visible_len(text) <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    format!("{}…", out.trim_end())
}

/// `text` padded with spaces to `width` columns, as the terminal draws it —
/// `format!("{:<9}")` counts characters, and Hangul takes two columns each.
pub fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(visible_len(text))))
}

/// Word-wrap `text` to `width` columns. The first line starts at column `first`
/// (text already printed before it); continuation lines are indented by `rest`.
/// A word longer than a line — a URL — stays whole so it still copies cleanly.
/// Japanese and Chinese, written without spaces, also break between two
/// characters — never after an opening bracket nor before closing punctuation
/// (kinsoku, as the desktop's `LineWrapper::may_break_between`). Hangul
/// breaks only at spaces.
pub fn wrap(text: &str, first: usize, rest: usize, width: usize) -> String {
    let mut out = String::new();
    let mut col = first;
    for (i, word) in text.split(' ').enumerate() {
        for (j, piece) in pieces(word).into_iter().enumerate() {
            let len = visible_len(piece);
            if i > 0 || j > 0 {
                let gap = usize::from(j == 0);
                if col + gap + len > width && col > rest {
                    out.push('\n');
                    out.push_str(&" ".repeat(rest));
                    col = rest;
                } else if gap == 1 {
                    out.push(' ');
                    col += 1;
                }
            }
            out.push_str(piece);
            col += len;
        }
    }
    out
}

/// A word cut where Japanese or Chinese may break inside it; style codes
/// stay with the character after them.
fn pieces(word: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev = None;
    let mut code_from = None;
    let mut in_code = false;
    for (at, c) in word.char_indices() {
        if in_code {
            in_code = !c.is_ascii_alphabetic() && c != '~';
            continue;
        }
        if c == '\x1b' {
            in_code = true;
            code_from.get_or_insert(at);
            continue;
        }
        let cut = code_from.take().unwrap_or(at);
        if let Some(p) = prev
            && (breaks_inside(p) || breaks_inside(c))
            && may_break_between(p, c)
            && cut > start
        {
            out.push(&word[start..cut]);
            start = cut;
        }
        prev = Some(c);
    }
    out.push(&word[start..]);
    out
}

/// Kana, Han and full-width punctuation: written without spaces.
fn breaks_inside(c: char) -> bool {
    matches!(c, '\u{3000}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{FF00}'..='\u{FF60}')
}

/// No break after an opening bracket nor before closing punctuation.
fn may_break_between(prev: char, c: char) -> bool {
    let opens = matches!(prev, '（' | '「' | '『' | '【' | '〈' | '《' | '〔' | '［' | '｛' | '〖' | '〘');
    let closes = matches!(c, '）' | '」' | '』' | '】' | '〉' | '》' | '〕' | '］' | '｝' | '〗' | '〙' | '、' | '。' | '，' | '．' | '；' | '：' | '！' | '？' | '・');
    !opens && !closes
}

#[derive(Clone, Copy)]
pub struct Painter {
    color: bool,
    truecolor: bool,
}

fn detect(is_tty: bool) -> Painter {
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let dumb = std::env::var("TERM").is_ok_and(|t| t == "dumb");
    let color = is_tty && !no_color && !dumb;
    let var = |name: &str| std::env::var(name).unwrap_or_default();
    let truecolor = truecolor(&var("COLORTERM"), &var("TERM"), &var("TERM_PROGRAM"), &var("LC_TERMINAL"));
    Painter { color, truecolor }
}

/// Whether the terminal takes 24-bit colour. `COLORTERM` says so, but SSH
/// doesn't carry it over; iTerm2's `LC_TERMINAL` does (ssh sends `LC_*`), and
/// some terminals name themselves in `TERM` or `TERM_PROGRAM`.
fn truecolor(colorterm: &str, term: &str, program: &str, lc_terminal: &str) -> bool {
    matches!(colorterm, "truecolor" | "24bit")
        || term.contains("truecolor")
        || term.contains("direct")
        || matches!(term, "xterm-kitty" | "xterm-ghostty" | "wezterm" | "alacritty")
        || matches!(program, "iTerm.app" | "WezTerm" | "ghostty" | "vscode")
        || matches!(lc_terminal, "iTerm2")
}

/// For text written to stdout.
pub fn out() -> Painter {
    static P: OnceLock<Painter> = OnceLock::new();
    *P.get_or_init(|| detect(std::io::stdout().is_terminal()))
}

/// For text written to stderr.
pub fn err() -> Painter {
    static P: OnceLock<Painter> = OnceLock::new();
    *P.get_or_init(|| detect(std::io::stderr().is_terminal()))
}

impl Painter {
    fn wrap(self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    /// Whether this stream gets colour, and whether it gets 24-bit colour.
    pub fn palette(self) -> (bool, bool) {
        (self.color, self.truecolor)
    }

    pub fn bold(self, text: &str) -> String {
        self.wrap("1", text)
    }

    /// Secondary text: labels, hints, metadata. Brand "Muted" #7C7E85.
    pub fn dim(self, text: &str) -> String {
        if self.truecolor {
            self.wrap("38;2;124;126;133", text)
        } else {
            self.wrap("2", text)
        }
    }

    /// The one accent: a cool rim on chrome. Brand rgba(180,205,230).
    pub fn accent(self, text: &str) -> String {
        self.wrap(if self.truecolor { "38;2;180;205;230" } else { "38;5;152" }, text)
    }

    /// Uppercase section label, set like a film credit.
    pub fn kicker(self, text: &str) -> String {
        self.dim(&text.to_uppercase())
    }

    pub fn ok(self, text: &str) -> String {
        format!("{} {text}", self.accent("✓"))
    }

    pub fn fail(self, text: &str) -> String {
        format!("{} {text}", self.bold("✗"))
    }

    /// The wordmark: lowercase, bold, with the chrome-grey period.
    pub fn wordmark(self) -> String {
        format!("{}{}", self.bold("equipe"), self.dim("."))
    }

    /// A `key  value` row inside a section; long values wrap under the value column.
    pub fn row(self, key: &str, value: &str) -> String {
        const VALUE_COL: usize = 13;
        format!(
            "  {}  {}",
            self.dim(&pad(key, 9)),
            wrap(value, VALUE_COL, VALUE_COL, width())
        )
    }

    /// Plain prose, wrapped at `indent`.
    pub fn para(self, indent: usize, text: &str) -> String {
        format!("{}{}", " ".repeat(indent), wrap(text, indent, indent, width()))
    }

    /// Secondary prose, wrapped at `indent`, every line dimmed.
    pub fn note(self, indent: usize, text: &str) -> String {
        wrap(text, indent, indent, width())
            .lines()
            .enumerate()
            .map(|(i, line)| {
                let pad = if i == 0 { " ".repeat(indent) } else { String::new() };
                let line = line.trim_start();
                let lead = if i == 0 { String::new() } else { " ".repeat(indent) };
                format!("{pad}{lead}{}", self.dim(line))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A line led by ✓, wrapped with continuation under the text.
    pub fn ok_line(self, text: &str) -> String {
        wrap(&self.ok(text), 0, 2, width())
    }

    /// A line led by ✗, wrapped with continuation under the text.
    pub fn fail_line(self, text: &str) -> String {
        wrap(&self.fail(text), 0, 2, width())
    }

    /// A section heading: label, then a dim description that wraps under it.
    pub fn section(self, label: &str, description: &str) -> String {
        let head = format!("{}  {}", self.kicker(label), self.dim(description));
        wrap(&head, 0, 2, width())
    }

    /// A next step the user can type.
    pub fn hint(self, command: &str) -> String {
        format!("{} {}", self.dim("→"), self.bold(command))
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn iterm_over_ssh_still_has_truecolor() {
        assert!(truecolor("", "xterm-256color", "", "iTerm2"), "LC_TERMINAL crosses ssh");
        assert!(truecolor("truecolor", "xterm-256color", "", ""));
        assert!(!truecolor("", "xterm-256color", "Apple_Terminal", ""));
    }

    use super::*;

    #[test]
    fn enclosed_numbers_are_spelled_out_so_they_keep_to_their_cell() {
        assert_eq!(cell_safe("Android MVP ①: QR"), "Android MVP (1): QR");
        assert_eq!(cell_safe("❷ and ⓑ"), "(2) and (b)");
        assert!(matches!(cell_safe("plain"), std::borrow::Cow::Borrowed(_)));
        assert_eq!(fit("MVP ② 스쿼드", 40), "MVP (2) 스쿼드");
    }

    #[test]
    fn fit_cuts_to_the_width_counting_wide_characters() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("builds the interface", 12), "builds the…");
        assert_eq!(visible_len(&fit("에이전트가 파일을 고쳤어요", 9)), 9);
    }

    #[test]
    fn wraps_between_words_never_inside_one() {
        let out = wrap("Claude Code keeps its own login; `claude auth logout` signs it out", 2, 2, 30);
        for line in out.lines() {
            assert!(visible_len(line) <= 30, "{line:?}");
        }
        assert!(out.contains("`claude auth logout`") || out.contains("`claude\n"));
        assert!(!out.contains("au\n"));
    }

    #[test]
    fn continuation_lines_keep_their_indent() {
        let out = wrap("one two three four five six", 4, 4, 14);
        for line in out.lines().skip(1) {
            assert!(line.starts_with("    ") && !line.starts_with("     "), "{line:?}");
        }
    }

    #[test]
    fn a_long_url_stays_whole() {
        let url = "https://auth.openai.com/oauth/authorize?response_type=code&client_id=x";
        assert_eq!(wrap(url, 0, 0, 20), url);
    }

    #[test]
    fn hangul_counts_two_columns_per_syllable() {
        assert_eq!(visible_len("안녕"), 4);
        let out = wrap("에이전트가 파일을 고쳤어요 이제 리뷰해 주세요", 0, 0, 20);
        for line in out.lines() {
            assert!(visible_len(line) <= 20, "{line:?} is {} columns", visible_len(line));
        }
    }

    #[test]
    fn japanese_and_chinese_wrap_between_characters_keeping_brackets() {
        let opens = ['（', '「'];
        let closes = ['）', '」', '、', '。', '，', '；'];
        for text in [
            "自分のワークツリーのサンドボックス内で編集・インストール・ビルド・テストを自分で行い、それ以外（ネットワーク、プッシュ、あなたのアカウント）は先に確認する",
            "在自己工作树的沙盒里自行编辑、安装、构建和测试；超出范围的事（网络、推送、你的账号）先询问",
            "在自己工作樹的沙盒裡自行編輯、安裝、建置和測試；超出範圍的事（網路、推送、你的帳號）先詢問",
        ] {
            for width in [20, 31, 40, 57, 66, 80, 100, 140] {
                let out = wrap(text, 4, 4, width);
                assert_eq!(out.replace(['\n', ' '], ""), text);
                let lines: Vec<&str> = out.lines().collect();
                for (n, line) in lines.iter().enumerate() {
                    assert!(visible_len(line) <= width, "at {width}: {line:?}");
                    let body = line.trim_start();
                    assert!(n == 0 || !closes.contains(&body.chars().next().unwrap()), "at {width}: {out}");
                    assert!(n + 1 == lines.len() || !opens.contains(&body.chars().last().unwrap()), "at {width}: {out}");
                }
            }
        }
        // Hangul still breaks only at spaces; a style code stays with its word.
        assert!(!wrap("가나다라마바사아자차카타파하", 0, 0, 10).contains('\n'));
        let styled = "\x1b[1m確認\x1b[0mする";
        assert_eq!(wrap(styled, 0, 0, 80), styled);
    }

    #[test]
    fn style_codes_do_not_count_as_width() {
        let styled = format!("\x1b[1m{}\x1b[0m", "abc");
        assert_eq!(visible_len(&styled), 3);
        assert_eq!(wrap(&format!("{styled} {styled}"), 0, 0, 7), format!("{styled} {styled}"));
    }
}
