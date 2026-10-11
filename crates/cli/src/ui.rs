//! Terminal presentation, following docs/BRAND.md: monochrome, one cool accent,
//! uppercase section labels, no emoji. Weight and dimming carry the hierarchy;
//! the accent marks only what succeeded.
//!
//! Plain text when the stream isn't a terminal, when `NO_COLOR` is set, or on a
//! dumb terminal, so piped output stays clean.

use std::io::IsTerminal as _;
use std::sync::OnceLock;

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

    /// A `key  value` row inside a section.
    pub fn row(self, key: &str, value: &str) -> String {
        format!("  {}  {value}", self.dim(&format!("{key:<9}")))
    }

    /// A next step the user can type.
    pub fn hint(self, command: &str) -> String {
        format!("{} {}", self.dim("→"), self.bold(command))
    }
}
