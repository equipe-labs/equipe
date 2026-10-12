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
pub fn fit(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar as _;
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

/// Word-wrap `text` to `width` columns. The first line starts at column `first`
/// (text already printed before it); continuation lines are indented by `rest`.
/// A word longer than a line — a URL — stays whole so it still copies cleanly.
pub fn wrap(text: &str, first: usize, rest: usize, width: usize) -> String {
    let mut out = String::new();
    let mut col = first;
    for (i, word) in text.split(' ').enumerate() {
        let len = visible_len(word);
        if i > 0 {
            if col + 1 + len > width && col > rest {
                out.push('\n');
                out.push_str(&" ".repeat(rest));
                col = rest;
            } else {
                out.push(' ');
                col += 1;
            }
        }
        out.push_str(word);
        col += len;
    }
    out
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
    let truecolor = std::env::var("COLORTERM").is_ok_and(|v| v == "truecolor" || v == "24bit");
    Painter { color, truecolor }
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
            self.dim(&format!("{key:<9}")),
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
    use super::*;

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
    fn style_codes_do_not_count_as_width() {
        let styled = format!("\x1b[1m{}\x1b[0m", "abc");
        assert_eq!(visible_len(&styled), 3);
        assert_eq!(wrap(&format!("{styled} {styled}"), 0, 0, 7), format!("{styled} {styled}"));
    }
}
