//! Design sessions: the person and the designer on one page in short rounds,
//! the page live beside the conversation (`equipe_protocol::design_session`).
//! The rules here, with no runtime: the kickoff the designer reads, the pins
//! the person drops on the page as a block before their words, the lines
//! `equipe design session …` prints that screens read back (what is live, the
//! pins done), and the PM's proposals — kept so each ticket or screen is
//! proposed at most once.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;
use crate::protocol::decide::{Answer, Choice, Question};
use crate::protocol::design::{PickReason, StyleProfile};
use crate::protocol::design_session::{ProposalOutcome, SessionFrom, SessionProposal};
use crate::protocol::TicketId;

/// The kickoff's marks.
pub const OPEN: &str = "<design-session>";
pub const CLOSE: &str = "</design-session>";
/// The pins' marks.
pub const PINS_OPEN: &str = "<design-pins>";
pub const PINS_CLOSE: &str = "</design-pins>";
/// What `equipe design session …` prints before what it did, a line each.
pub const MARK: &str = "design-session:";
/// The most pins one message carries.
pub const PINS_MAX: usize = 12;
/// The longest a pin's note may be, in characters.
pub const NOTE_MAX: usize = 280;

/// What the designer is told to do with pins, the pins block's last line.
const PINS_HOW: &str = "Address each pin, then mark it with `equipe design session done <n> \"<what changed, one line>\"`.";

/// What the designer is told when a session starts, inside the kickoff block.
const HOW: &str = "This is a design session in prototype mode: work as your `design-session` skill says. Small steps \
of minutes, not hours. First, before any direction: pull 3-5 references for this page from the design MCPs you have \
connected (21st.dev, Mobbin; else the web) and put them on screen with `show_references`. Then build a runnable \
prototype in the session's folder (`design/sessions/<date>-<page>/` where you work) and show it live with `equipe design session show` (variants side by side with \
`--variant A|B|C`). End every turn with something visible and one focused question; never go quiet on a long build.";

/// What every later turn of a session opens with while it is on (the
/// runtime adds it): the rules of a prototype round, so a long conversation
/// never drifts back into ticket-sized work.
pub const ROUND_RULES: &str = "design session on — a prototype round: reply in a few lines, apply the person's \
feedback to the prototype within a couple of minutes and show it again (`equipe design session show`), add a line to \
the session's CHANGELOG.md. No heavy checks in a round: no width x theme x text-size matrix, no axe or contrast runs, \
no design QA, no new ticket per feedback item. Asked for new directions: references first (design MCPs, \
`show_references`). The full checks run once the person picks a variant with \"Use this\": equipe opens the \
implementation ticket then.";

/// Where a session's prototype files are, told each round: a designer that
/// lost track once searched the whole disk for its own prototype.
pub const WHERE_FILES: &str = "The session's files (prototypes, variants, CHANGELOG.md) are in its folder under \
`design/sessions/` where you work: `ls -t design/sessions/` lists the latest first. Never search the disk for them.";

fn from_words(from: &SessionFrom) -> String {
    match from {
        SessionFrom::DesignScreen => "the design system screen".into(),
        SessionFrom::Ticket { ticket } => format!("ticket T-{ticket}"),
        SessionFrom::Member => "the designer's row on the team".into(),
        SessionFrom::Pm { ticket: Some(t) } => format!("the PM's proposal on T-{t}"),
        SessionFrom::Pm { ticket: None } => "the PM".into(),
    }
}

/// The kickoff block: what the session is on, where it was started, how to
/// work, and whether the style profile is thin (so the first turn asks the
/// questions that fill it).
pub fn block(page: &str, project: Option<&str>, from: &SessionFrom, profile: Option<&StyleProfile>) -> String {
    let mut lines = vec![OPEN.to_owned(), format!("page: {}", one_line(page))];
    lines.extend(project.map(|p| format!("project: {p}")));
    lines.push(format!("started from: {}", from_words(from)));
    let thin = profile.is_none_or(StyleProfile::is_thin);
    lines.push(format!("style profile: {}", if thin { "thin or missing: learn it as you go, and record it after each round" } else { "read it first, at the top of design/taste.md" }));
    lines.push(HOW.to_owned());
    lines.push(CLOSE.to_owned());
    lines.join("\n")
}

/// The kickoff as the person's message: the block, then their words.
pub fn kickoff(page: &str, project: Option<&str>, from: &SessionFrom, profile: Option<&StyleProfile>, words: &str) -> String {
    format!("{}\n\n{}", block(page, project, from, profile), words.trim())
}

/// A kickoff read back: the page it is on, and the words after it.
pub fn split_kickoff(text: &str) -> Option<(String, String)> {
    let after = text.trim_start().strip_prefix(OPEN)?;
    let (inside, rest) = after.split_once(CLOSE)?;
    let page = inside.lines().find_map(|l| l.trim().strip_prefix("page:")).map(|p| p.trim().to_owned())?;
    Some((page, rest.trim_start_matches(['\n', ' ']).to_owned()))
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── a session the daemon keeps while it is on ──

/// A design session on, as the daemon keeps it: from its start on a screen
/// until the person ends it, picks a variant, or leaves it idle for
/// [`IDLE_ENDS_SECS`]. While it is on the designer's turns open with
/// [`ROUND_RULES`] and the person's words cut in ([`preempt`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Active {
    pub squad: String,
    pub project: Option<String>,
    pub page: String,
    /// Unix seconds it last heard from the person.
    pub last: u64,
    /// The implementation ticket "Use this" opened, updated by the next pick.
    pub implementation: Option<TicketId>,
    /// A variant was picked: the rounds are over (no round rules, no cutting
    /// in) until the person starts again; a later pick updates the ticket.
    pub picked: bool,
}

/// A session nobody spoke in for this long is over (seconds).
pub const IDLE_ENDS_SECS: u64 = 4 * 3600;

impl Active {
    pub fn new(squad: &str, project: Option<&str>, page: &str, now: u64) -> Self {
        Self { squad: squad.to_owned(), project: project.map(str::to_owned), page: page.to_owned(), last: now, implementation: None, picked: false }
    }

    /// Whether it is still on at `now`: not idle too long. (Its rounds may
    /// be over — [`Self::rounds`].)
    pub fn on(&self, now: u64) -> bool {
        now.saturating_sub(self.last) < IDLE_ENDS_SECS
    }

    /// Whether its turns are prototype rounds at `now`: on, and nothing picked yet.
    pub fn rounds(&self, now: u64) -> bool {
        self.on(now) && !self.picked
    }

    /// The note the designer's next turn opens with: the rules, the page,
    /// and where its files are — so it never searches the disk for them.
    pub fn note(&self) -> String {
        format!("{ROUND_RULES} (page: {}) {WHERE_FILES}", one_line(&self.page))
    }
}

/// What the person's message does to the designer's turn under way in a
/// session: the person comes first, not the designer's next step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preempt {
    /// Taken in as a steer with no command running: read at its next step, soon.
    Nothing,
    /// Taken in as a steer while a command runs (a build, a capture run):
    /// that command is stopped so the step comes at once.
    StopCommand,
    /// The engine can't take words mid-turn (left as a memo): the turn is
    /// stopped, and the memo goes in as the next turn.
    Interrupt,
}

/// [`Preempt`] for a message said while the designer's turn runs: whether
/// the engine took it as a steer, and whether a command runs now.
pub fn preempt(steered: bool, command_running: bool) -> Preempt {
    match (steered, command_running) {
        (false, _) => Preempt::Interrupt,
        (true, true) => Preempt::StopCommand,
        (true, false) => Preempt::Nothing,
    }
}

/// The kind of the ticket record that marks an implementation ticket "Use
/// this" opened from a live prototype the person picked; its text is the
/// prototype's address. Design QA of such a ticket gets no designer turn
/// unless its precheck fails (`crate::design_qa::gate_prototype`).
pub const APPROVED_PROTOTYPE: &str = "approved prototype";

/// The variant the person picked with "Use this", as the implementation
/// ticket takes it for its spec.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picked {
    pub page: String,
    pub project: Option<String>,
    /// `A`, `B` or `C`; `None`: the one page shown.
    pub variant: Option<String>,
    /// The prototype's address as the designer showed it.
    pub url: Option<String>,
}

/// The implementation ticket's body: what the person picked and where it
/// is, the session's change log, and that the full checks run here. `intro`
/// is its first line, in the squad's language.
pub fn implementation_body(intro: &str, picked: &Picked) -> String {
    let mut lines = vec![intro.trim().to_owned(), String::new()];
    lines.push(format!("- Page: {}", one_line(&picked.page)));
    lines.extend(picked.project.as_ref().map(|p| format!("- Project: {p}")));
    lines.extend(picked.variant.as_ref().map(|v| format!("- Variant: {v}")));
    lines.extend(picked.url.as_ref().map(|u| format!("- Prototype: {u}")));
    lines.push("- Spec: the prototype as it is now, and the session's CHANGELOG.md beside it (design/sessions/).".into());
    lines.push(String::new());
    lines.push("Build it in the product from the project's tokens and components. The full design checks run on this ticket \
(every width, light and dark, larger text, accessibility, design QA) — none of them ran during the session's rounds."
        .into());
    lines.join("\n")
}

/// What the designer is told once the person picked a variant.
pub fn picked_line(picked: &Picked, ticket: TicketId, updated: bool) -> String {
    let which = picked.variant.as_ref().map_or_else(|| "the prototype".to_owned(), |v| format!("variant {v} of the prototype"));
    let what = if updated { "updated" } else { "opened" };
    format!(
        "The person picked {which} for {} with \"Use this\": equipe {what} T-{ticket} to build it, with the prototype as its spec. \
The session's rounds on this page are over; the full checks run on T-{ticket}. Make sure the prototype and its CHANGELOG.md are saved in design/sessions/.",
        one_line(&picked.page)
    )
}

// ── equipe's framing, apart from the person's words ──

/// How a note of equipe's framing a turn opens, so the model knows whose it is.
const FRAMING_HEAD: &str = "From equipe, the app the person leads this squad in — equipe's own note, not the person's words. \
Their message follows.";

/// A message about to go to an engine that takes equipe's framing apart
/// from the person's words (Codex, as a developer message): equipe's own
/// lines in it — a design session's kickoff block with the notes of equipe's
/// that open it, the pins' line on what to do with them — taken out, and
/// those lines as one note. OpenAI's safety check reads instructions sitting
/// in the person's message that the person didn't write as possibly
/// unintended activity, and refuses the turn. The person's words and the pins
/// themselves (where they pointed, what they said) stay their message.
/// `None`: nothing to take apart — a message with no session in it, or one
/// that would be left empty.
pub fn framing_apart(message: &str) -> (String, Option<String>) {
    let mut notes: Vec<String> = Vec::new();
    let mut words = message.to_owned();
    if let Some(start) = words.find(OPEN)
        && let Some(len) = words[start..].find(CLOSE)
    {
        // What opens the message before the block: equipe's bracketed notes
        // (the project the person looks at, what changed since) go with it.
        let (before, kept) = leading_notes(&words[..start]);
        notes.extend(before);
        let inside = words[start + OPEN.len()..start + len].trim().to_owned();
        notes.push(format!("The person started a design session with you from equipe's screen:\n{inside}"));
        words = format!("{kept}{}", &words[start + len + CLOSE.len()..]);
    }
    if words.contains(PINS_OPEN) && words.contains(PINS_HOW) {
        words = words.replacen(&format!("\n{PINS_HOW}"), "", 1).replacen(PINS_HOW, "", 1);
        notes.push(format!("The person's message carries pins they dropped on the live page ({PINS_OPEN}). {PINS_HOW}"));
    }
    while words.contains("\n\n\n") {
        words = words.replace("\n\n\n", "\n\n");
    }
    let words = words.trim().to_owned();
    if notes.is_empty() || words.is_empty() {
        return (message.to_owned(), None);
    }
    (words, Some(format!("{FRAMING_HEAD}\n\n{}", notes.join("\n\n"))))
}

/// equipe's bracketed notes opening `text` (`[equipe: …]`, `[equipe] …`),
/// each whole — brackets inside them, a Markdown link say, balanced — and
/// what is left of `text` after them.
fn leading_notes(text: &str) -> (Vec<String>, String) {
    let mut notes = Vec::new();
    let mut rest = text.trim_start();
    while rest.starts_with("[equipe") {
        let mut depth = 0usize;
        let mut end = None;
        for (at, c) in rest.char_indices() {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = Some(at + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        // Unbalanced, or more of the line after it: not a note of equipe's alone.
        let Some(end) = end.filter(|&e| rest[e..].starts_with('\n') || rest[e..].trim().is_empty()) else { break };
        notes.push(rest[..end].to_owned());
        rest = rest[end..].trim_start();
    }
    (notes, rest.to_owned())
}

// ── pins ──

/// A spot on the live page the person pointed at, with their note: where it
/// is in the page (CSS pixels from its top left, scrolling included) at the
/// width it was drawn, the element there when known, and whether a crop of
/// it goes with the message as a picture.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    pub n: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    #[serde(default)]
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<PickReason>,
    /// The crop goes with the message, as its `crop`th picture (1-based).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<usize>,
    /// The element's own words, as the page shows them (trimmed, one line).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The element's box in the page: x, y, width, height in CSS pixels
    /// from the page's top left (scrolling included).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[u32; 4]>,
}

/// The longest an element's words in a pin may be, in characters.
pub const TEXT_MAX: usize = 120;

impl Pin {
    /// Its note made fit to send: one line, at most [`NOTE_MAX`] characters.
    pub fn tidy(note: &str) -> String {
        let one = one_line(note);
        if one.chars().count() > NOTE_MAX { one.chars().take(NOTE_MAX).collect::<String>().trim_end().to_owned() } else { one }
    }
}

/// The pins as the block the designer reads before the person's words:
/// the page they are on, then each pin — where, the element, what the person
/// said and why, and which picture is its crop. Empty: no pins.
pub fn pins_block(page: &str, pins: &[Pin]) -> String {
    if pins.is_empty() {
        return String::new();
    }
    let width = pins.first().map_or(0, |p| p.width);
    let mut out = vec![PINS_OPEN.to_owned(), format!("on: {} at {width} wide", one_line(page))];
    for pin in pins.iter().take(PINS_MAX) {
        let reasons = if pin.reasons.is_empty() { String::new() } else { format!(" ({})", pin.reasons.iter().map(|r| r.words()).collect::<Vec<_>>().join(", ")) };
        out.push(format!("- pin {} at {},{}: {}{reasons}", pin.n, pin.x, pin.y, Pin::tidy(&pin.note)));
        out.extend(pin.selector.as_ref().map(|s| format!("  element: {s}")));
        out.extend(pin.text.as_deref().map(element_words).filter(|t| !t.is_empty()).map(|t| format!("  text: \"{t}\"")));
        out.extend(pin.bounds.map(|[x, y, w, h]| format!("  box: {x},{y} {w}x{h}")));
        out.extend(pin.crop.map(|c| format!("  crop: attached picture {c}")));
    }
    out.push(PINS_HOW.to_owned());
    out.push(PINS_CLOSE.to_owned());
    out.join("\n")
}

/// An element's words made fit for a pin: one line, quotes made plain, at
/// most [`TEXT_MAX`] characters.
pub fn element_words(text: &str) -> String {
    let one = one_line(text).replace('"', "'");
    if one.chars().count() > TEXT_MAX { format!("{}…", one.chars().take(TEXT_MAX).collect::<String>().trim_end()) } else { one }
}

/// The person's message with its pins before it.
pub fn with_pins(page: &str, pins: &[Pin], words: &str) -> String {
    let block = pins_block(page, pins);
    match (block.is_empty(), words.trim().is_empty()) {
        (true, _) => words.trim().to_owned(),
        (false, true) => block,
        (false, false) => format!("{block}\n\n{}", words.trim()),
    }
}

/// A message read back: its pins (number and note) and the rest.
pub fn split_pins(text: &str) -> (Vec<(u32, String)>, String) {
    let Some(after) = text.trim_start().strip_prefix(PINS_OPEN) else { return (Vec::new(), text.to_owned()) };
    let Some((inside, rest)) = after.split_once(PINS_CLOSE) else { return (Vec::new(), text.to_owned()) };
    let pins = inside
        .lines()
        .filter_map(|l| l.strip_prefix("- pin "))
        .filter_map(|l| {
            let (n, rest) = l.split_once(' ')?;
            let note = rest.split_once(": ").map(|(_, n)| n).unwrap_or_default();
            Some((n.parse().ok()?, note.trim().to_owned()))
        })
        .collect();
    (pins, rest.trim_start_matches(['\n', ' ']).to_owned())
}

// ── what the designer shows ──

/// A line `equipe design session …` printed, read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Marker {
    /// A page live: an address the shared browser opens (`http(s)://` or a
    /// `file://` page in the worktree).
    Show(String),
    /// One of the prototype's variants live, by its label (`A`, `B`, `C`):
    /// the screens put the variants side by side.
    Variant { label: String, url: String },
    /// A static render: a picture on the daemon's machine.
    Render(String),
    /// Pin `n` addressed, and what changed.
    Done { n: u32, what: String },
}

impl Marker {
    /// Its line, as the command prints it.
    pub fn line(&self) -> String {
        match self {
            Marker::Show(url) => format!("{MARK} show {url}"),
            Marker::Variant { label, url } => format!("{MARK} variant {label} {url}"),
            Marker::Render(path) => format!("{MARK} render {path}"),
            Marker::Done { n, what } => format!("{MARK} done {n} {}", one_line(what)),
        }
    }
}

/// The markers in a command's output, in order.
pub fn markers(output: &str) -> Vec<Marker> {
    output
        .lines()
        .filter_map(|l| l.trim().strip_prefix(MARK))
        .filter_map(|rest| {
            let (verb, arg) = rest.trim().split_once(' ')?;
            let arg = arg.trim();
            match verb {
                "show" if is_page_address(arg) => Some(Marker::Show(arg.to_owned())),
                "variant" => {
                    let (label, url) = arg.split_once(' ')?;
                    let url = url.trim();
                    (variant_label(label).is_some() && is_page_address(url)).then(|| Marker::Variant { label: label.to_ascii_uppercase(), url: url.to_owned() })
                }
                "render" if crate::visuals::is_image_name(arg) && arg.starts_with('/') => Some(Marker::Render(arg.to_owned())),
                "done" => {
                    let (n, what) = arg.split_once(' ').unwrap_or((arg, ""));
                    Some(Marker::Done { n: n.parse().ok()?, what: what.trim().to_owned() })
                }
                _ => None,
            }
        })
        .collect()
}

/// The most variants shown side by side.
pub const VARIANTS_MAX: usize = 3;

/// A variant's label as the screens show it: one letter, `A` to `C`.
pub fn variant_label(raw: &str) -> Option<String> {
    let label = raw.trim().to_ascii_uppercase();
    (label.len() == 1 && ('A'..=(b'A' + VARIANTS_MAX as u8 - 1) as char).contains(&label.chars().next()?)).then_some(label)
}

/// An address the live pane may open: a web page (`http(s)://`) or a page
/// file (`file:///…`), with no space in it.
pub fn is_page_address(url: &str) -> bool {
    !url.contains(char::is_whitespace) && ["http://", "https://", "file:///"].iter().any(|s| url.len() > s.len() && url.to_ascii_lowercase().starts_with(s))
}

/// What `equipe design session show <target>` makes of its target: a web
/// address or `localhost:port` as it is, an HTML file (or a folder with an
/// `index.html`) as its `file://` address — which the daemon serves to the
/// person's screen, wherever it is ([`file_path`], `crate::prototype`) — a
/// picture as a render. `cwd` resolves a relative path.
pub fn target(raw: &str, cwd: &Path) -> Result<Marker, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Show what? A page's address (http://localhost:5173/pricing), an HTML file, or a rendered picture.".into());
    }
    let lower = raw.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return if is_page_address(raw) { Ok(Marker::Show(raw.to_owned())) } else { Err(format!("{raw} isn't an address the browser can open")) };
    }
    if lower.starts_with("localhost:") || lower.starts_with("127.0.0.1:") {
        return Ok(Marker::Show(format!("http://{raw}")));
    }
    let path = if Path::new(raw).is_absolute() { Path::new(raw).to_path_buf() } else { cwd.join(raw) };
    let path = std::fs::canonicalize(&path).map_err(|_| format!("{} isn't there", path.display()))?;
    let path = if path.is_dir() && path.join("index.html").is_file() { path.join("index.html") } else { path };
    if !path.is_file() {
        return Err(format!("{} isn't a file", path.display()));
    }
    let shown = path.to_string_lossy().into_owned();
    if crate::visuals::is_image_name(&shown) {
        return Ok(Marker::Render(shown));
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    if matches!(ext.as_str(), "html" | "htm") {
        return Ok(Marker::Show(format!("file://{}", shown.replace(' ', "%20"))));
    }
    Err(format!("{shown} is neither a page (.html) nor a picture"))
}

/// The file a `file://` page address names, `%XX` read back; `None` for
/// any other address.
pub fn file_path(url: &str) -> Option<std::path::PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%' && i + 2 < bytes.len()).then(|| std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())).flatten();
        match hex {
            Some(b) => {
                out.push(b);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    let path = std::path::PathBuf::from(String::from_utf8(out).ok()?);
    path.is_absolute().then_some(path)
}

/// The folder a session page `page` (under `root`, where its member works)
/// is served from: the nearest `design` folder it is in, inside `root`, so
/// it may reach `../tokens.css` there; else its own folder.
pub fn page_scope(page: &Path, root: &Path) -> std::path::PathBuf {
    let own = page.parent().unwrap_or(page);
    own.ancestors().take_while(|a| a.starts_with(root)).find(|a| a.file_name().is_some_and(|n| n == "design")).unwrap_or(own).to_path_buf()
}

/// What a member hears when what it showed (`urls`) reached no screen.
pub fn nobody_line(urls: &str) -> String {
    format!(
        "No screen of the person's follows the squad in equipe's app right now, so what you just showed ({urls}) reached \
nobody. Don't tell them it's on their screen or beside the session: say in words what changed and where it is; it shows \
there once their app is back."
    )
}

// ── the PM's proposals ──

/// The PM's tool: propose a design session to the person, or start one they asked for.
pub const PROPOSE: &str = "propose_design_session";

/// [`PROPOSE`]'s definition, offered to the PM.
pub fn tool() -> serde_json::Value {
    serde_json::json!({
        "name": PROPOSE,
        "description": "Propose a short design session (the person and the designer on one page, live, in rounds of minutes) \
before design work goes to the designer alone — as your `working-with-design` skill says when. The person gets a card: \
start a 20-30 min session, not now, or let the designer go ahead (and, when their style profile is thin, tune their \
taste first). At most once per ticket or screen, never in their quiet hours; equipe refuses a second one. When the \
person themselves asks for a design session, set `asked`: it starts at once, no card.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "screen": { "type": "string", "description": "The page or screen it is on: \"Pricing page\"." },
                "why": { "type": "string", "description": "One line on why a session now, in the person's language." },
                "ticket": { "type": "integer", "description": "The designer's ticket it is about, if any." },
                "project": { "type": "string", "description": "The project, by name, when the squad has several." },
                "asked": { "type": "boolean", "description": "The person asked for a session: start it now, no card." },
            },
            "required": ["screen"],
        },
    })
}

/// The proposals made, in the squad's own data folder.
pub const PROPOSALS_FILE: &str = "design-session-proposals.json";

/// One proposal made: what it was about, the card it went out on, when, and
/// what the person did with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposed {
    pub key: String,
    #[serde(flatten)]
    pub proposal: SessionProposal,
    /// The question set it went out as.
    pub set: u64,
    pub at: u64,
    /// The card offered the taste tuner first (the profile was thin).
    #[serde(default)]
    pub tune: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ProposalOutcome>,
}

/// What a proposal is about, for "at most once": its ticket, else its
/// project and screen, case and spacing aside.
pub fn key(ticket: Option<TicketId>, project: Option<&str>, screen: &str) -> String {
    match ticket {
        Some(t) => format!("ticket:{t}"),
        None => format!("screen:{}/{}", project.unwrap_or(""), one_line(screen).to_lowercase()),
    }
}

/// Whether a proposal on `key` may go out: never twice for the same ticket
/// or screen, whatever came of the first — "Not now" holds it back for good.
pub fn may_propose(made: &[Proposed], key: &str) -> Result<(), String> {
    match made.iter().find(|p| p.key == key) {
        None => Ok(()),
        Some(p) => Err(match p.outcome {
            Some(ProposalOutcome::NotNow) => "The person said \"Not now\" to a design session for this; don't propose it again — carry on as planned.".into(),
            Some(ProposalOutcome::GoAhead) => "The person already let the designer go ahead on this alone; don't propose a session again.".into(),
            Some(ProposalOutcome::Started) => "A design session on this was already started.".into(),
            None => "A design session for this is already on the person's card; wait for their answer.".into(),
        }),
    }
}

pub fn read(dir: &Path) -> Vec<Proposed> {
    std::fs::read(dir.join(PROPOSALS_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn write(dir: &Path, made: &[Proposed]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(made).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(PROPOSALS_FILE), text).map_err(|e| e.to_string())
}

/// What the person chose on a proposal's card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choose {
    Start,
    /// Tune their taste first (offered when the profile was thin).
    Tune,
    NotNow,
    GoAhead,
}

impl Choose {
    pub fn outcome(self) -> ProposalOutcome {
        match self {
            Choose::Start | Choose::Tune => ProposalOutcome::Started,
            Choose::NotNow => ProposalOutcome::NotNow,
            Choose::GoAhead => ProposalOutcome::GoAhead,
        }
    }
}

/// The card's options, in order.
fn options(tune: bool) -> Vec<Choose> {
    let mut out = vec![Choose::Start];
    if tune {
        out.push(Choose::Tune);
    }
    out.extend([Choose::NotNow, Choose::GoAhead]);
    out
}

/// The card a proposal goes out on, in the person's language: one line on
/// why, "Start a 20-30 min session", "Not now", "Let the designer go
/// ahead" — and first "Tune my taste (2 min)" when the profile is thin.
pub fn question(proposal: &SessionProposal, tune: bool, lang: Lang) -> Question {
    let words = crate::i18n::core(lang);
    let label = |c: Choose| match c {
        Choose::Start => words.get("design_session.start").to_owned(),
        Choose::Tune => words.get("design_session.tune").to_owned(),
        Choose::NotNow => words.get("design_session.not_now").to_owned(),
        Choose::GoAhead => words.get("design_session.go_ahead").to_owned(),
    };
    let screen = one_line(&proposal.screen);
    Question {
        question: words.fill("design_session.question", &[("screen", screen)]),
        header: words.get("design_session.header").to_owned(),
        options: options(tune).into_iter().map(|c| Choice { label: label(c), recommended: c == Choose::Start, ..Choice::default() }).collect(),
        ..Question::default()
    }
    .with_why(&proposal.why)
}

trait WithWhy {
    fn with_why(self, why: &str) -> Self;
}

impl WithWhy for Question {
    fn with_why(mut self, why: &str) -> Self {
        let why = one_line(why);
        if !why.is_empty() {
            self.question = format!("{} {why}", self.question);
        }
        self
    }
}

/// What the person chose, from their answer to the card (`None`: dismissed
/// or skipped — taken as "Not now").
pub fn chosen(tune: bool, answer: Option<&Answer>) -> Choose {
    answer.and_then(|a| a.picked.first()).and_then(|i| options(tune).get(*i).copied()).unwrap_or(Choose::NotNow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kickoff_goes_as_equipes_note_with_the_notes_opening_it_and_the_persons_words_stay_theirs() {
        let text = kickoff("Pricing", Some("web"), &SessionFrom::DesignScreen, None, "Let's work on Pricing together.");
        let prompt = format!(
            "[equipe: the person is looking at the project web — new work is for it unless they say otherwise]\n\n[equipe: since your last turn —\n- frontend handed in [the page](http://localhost:3000/x)]\n\n{text}"
        );
        let (words, framing) = framing_apart(&prompt);
        assert_eq!(words, "Let's work on Pricing together.");
        let framing = framing.expect("equipe's note");
        assert!(framing.starts_with(FRAMING_HEAD), "{framing}");
        assert!(framing.contains("the person is looking at the project web") && framing.contains("[the page](http://localhost:3000/x)"), "{framing}");
        assert!(framing.contains("page: Pricing") && framing.contains("work as your `design-session` skill says"), "{framing}");
        assert!(!framing.contains(OPEN) && !words.contains(OPEN));
    }

    #[test]
    fn pins_stay_the_persons_and_only_equipes_line_on_them_moves() {
        let pins = [Pin { n: 1, x: 10, y: 20, width: 1440, note: "tighter".into(), ..Pin::default() }];
        let message = with_pins("Pricing", &pins, "and the header too");
        let (words, framing) = framing_apart(&message);
        assert!(words.starts_with(PINS_OPEN) && words.contains("pin 1 at 10,20: tighter") && words.ends_with("and the header too"), "{words}");
        assert!(words.contains(PINS_CLOSE) && !words.contains("equipe design session done"), "{words}");
        assert!(framing.expect("equipe's note").contains("equipe design session done <n>"));
        assert_eq!(split_pins(&words).0, vec![(1, "tighter".to_owned())], "the pins read back as before");
    }

    #[test]
    fn a_message_without_equipes_framing_goes_as_it_is() {
        let plain = "[equipe: since your last turn — x]\n\nfix it";
        assert_eq!(framing_apart(plain), (plain.to_owned(), None));
        let bare = block("Pricing", None, &SessionFrom::DesignScreen, None);
        assert_eq!(framing_apart(&bare), (bare.clone(), None), "nothing would be left to say");
    }

    #[test]
    fn a_session_on_tells_each_round_to_stay_quick_and_light() {
        let active = Active::new("acme", Some("site"), "Pricing\npage", 100);
        let note = active.note();
        for rule in ["prototype round", "within a couple of minutes", "no width x theme x text-size matrix", "no axe", "no design QA", "no new ticket per feedback item", "CHANGELOG.md", "show_references", "Use this"] {
            assert!(note.contains(rule), "{rule}: {note}");
        }
        assert!(note.contains("(page: Pricing page)"));
        // Where its files are, each round: never a search of the disk.
        assert!(note.ends_with(WHERE_FILES) && note.contains("ls -t design/sessions/") && note.contains("Never search the disk"), "{note}");
        assert!(active.on(100 + IDLE_ENDS_SECS - 1) && !active.on(100 + IDLE_ENDS_SECS));
        assert!(active.rounds(100) && !Active { picked: true, ..active.clone() }.rounds(100), "a pick ends the rounds");
        let kickoff = block("Pricing", None, &SessionFrom::DesignScreen, None);
        assert!(kickoff.contains("21st.dev, Mobbin") && kickoff.contains("show_references") && kickoff.contains("--variant A|B|C") && kickoff.contains("design/sessions/<date>-<page>/"), "{kickoff}");
    }

    #[test]
    fn the_persons_words_cut_into_a_session_turn() {
        assert_eq!(preempt(true, false), Preempt::Nothing, "read at its next step");
        assert_eq!(preempt(true, true), Preempt::StopCommand, "the command running is stopped so that step comes now");
        assert_eq!(preempt(false, false), Preempt::Interrupt, "an engine that can't take a steer stops its turn");
        assert_eq!(preempt(false, true), Preempt::Interrupt);
    }

    #[test]
    fn a_picked_variant_becomes_the_implementation_spec() {
        let picked = Picked { page: "Pricing".into(), project: Some("site".into()), variant: Some("B".into()), url: Some("http://localhost:5173/proto/b".into()) };
        let body = implementation_body("Picked in a design session.", &picked);
        assert!(body.starts_with("Picked in a design session.\n\n- Page: Pricing\n- Project: site\n- Variant: B\n- Prototype: http://localhost:5173/proto/b\n"), "{body}");
        assert!(body.contains("full design checks run on this ticket"));
        let told = picked_line(&picked, 12, false);
        assert!(told.contains("variant B") && told.contains("opened T-12") && told.contains("full checks run on T-12"), "{told}");
        assert!(picked_line(&Picked { variant: None, ..picked }, 12, true).contains("the prototype for Pricing") );
    }

    #[test]
    fn variants_are_shown_by_their_letter() {
        assert_eq!(variant_label(" b "), Some("B".into()));
        assert_eq!(variant_label("D"), None);
        assert_eq!(variant_label("AB"), None);
        let out = "design-session: variant b http://localhost:5173/a\ndesign-session: variant D http://localhost:5173/d\ndesign-session: variant C file:///tmp/c.html";
        assert_eq!(markers(out), vec![Marker::Variant { label: "B".into(), url: "http://localhost:5173/a".into() }, Marker::Variant { label: "C".into(), url: "file:///tmp/c.html".into() }]);
        assert_eq!(Marker::Variant { label: "A".into(), url: "http://x/a".into() }.line(), "design-session: variant A http://x/a");
    }

    #[test]
    fn a_pin_carries_the_elements_words_and_box() {
        let pin = Pin { n: 1, x: 10, y: 20, width: 1440, note: "tighter".into(), selector: Some("main > button".into()), text: Some("  Start \"free\"\n trial ".into()), bounds: Some([4, 12, 120, 40]), crop: Some(1), ..Pin::default() };
        let block = pins_block("http://localhost:5173/", &[pin]);
        assert!(block.contains("- pin 1 at 10,20: tighter\n  element: main > button\n  text: \"Start 'free' trial\"\n  box: 4,12 120x40\n  crop: attached picture 1"), "{block}");
        assert_eq!(element_words(&"x".repeat(200)).chars().count(), TEXT_MAX + 1);
    }

    #[test]
    fn the_kickoff_names_the_page_and_how_to_work_and_reads_back() {
        let text = kickoff("Pricing\npage", Some("web"), &SessionFrom::Ticket { ticket: 12 }, None, "Let's work on it together.");
        assert!(text.starts_with("<design-session>\npage: Pricing page\nproject: web\nstarted from: ticket T-12\nstyle profile: thin"), "{text}");
        assert!(text.contains("`design-session` skill") && text.contains("3-5 references") && text.contains("one focused question"));
        assert_eq!(split_kickoff(&text), Some(("Pricing page".into(), "Let's work on it together.".into())));
        let full = StyleProfile { density: "airy".into(), tone: "calm".into(), typography: "serif".into(), ..Default::default() };
        assert!(block("Home", None, &SessionFrom::Pm { ticket: None }, Some(&full)).contains("style profile: read it first"));
        assert_eq!(split_kickoff("just words"), None);
    }

    #[test]
    fn pins_go_before_the_words_with_where_and_why_and_their_crop() {
        let pins = [
            Pin { n: 1, x: 812, y: 1340, width: 1440, note: "  Too\nloud ".into(), selector: Some("main > h1".into()), reasons: vec![PickReason::Colors], crop: Some(1), ..Pin::default() },
            Pin { n: 2, x: 40, y: 90, width: 1440, note: "logo smaller".into(), ..Pin::default() },
        ];
        let text = with_pins("http://localhost:5173/pricing", &pins, " and the footer? ");
        assert_eq!(
            text,
            "<design-pins>\non: http://localhost:5173/pricing at 1440 wide\n\
             - pin 1 at 812,1340: Too loud (the colors)\n  element: main > h1\n  crop: attached picture 1\n\
             - pin 2 at 40,90: logo smaller\n\
             Address each pin, then mark it with `equipe design session done <n> \"<what changed, one line>\"`.\n\
             </design-pins>\n\nand the footer?"
        );
        let (back, rest) = split_pins(&text);
        assert_eq!(back, [(1, "Too loud (the colors)".to_owned()), (2, "logo smaller".to_owned())]);
        assert_eq!(rest, "and the footer?");
        assert_eq!(with_pins("p", &[], " hi "), "hi");
        assert_eq!(split_pins("plain"), (vec![], "plain".to_owned()));
        assert_eq!(Pin::tidy(&"a".repeat(400)).chars().count(), NOTE_MAX);
    }

    #[test]
    fn what_the_designer_shows_and_marks_done_is_read_from_the_commands_output() {
        let out = format!(
            "Showing it.\n{}\n{}\n{}\n{MARK} show not an address\n{MARK} render relative.png\n",
            Marker::Show("http://localhost:5173/pricing".into()).line(),
            Marker::Done { n: 3, what: "Quieter accent  on the CTA".into() }.line(),
            Marker::Render("/w/design/sessions/r1-390.png".into()).line()
        );
        assert_eq!(
            markers(&out),
            [Marker::Show("http://localhost:5173/pricing".into()), Marker::Done { n: 3, what: "Quieter accent on the CTA".into() }, Marker::Render("/w/design/sessions/r1-390.png".into())]
        );
        let dir = std::env::temp_dir().join(format!("equipe-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("page.html"), "<h1>x</h1>").unwrap();
        std::fs::write(dir.join("r.png"), b"x").unwrap();
        std::fs::write(dir.join("notes.md"), "x").unwrap();
        let real = std::fs::canonicalize(&dir).unwrap();
        assert_eq!(target("page.html", &dir), Ok(Marker::Show(format!("file://{}/page.html", real.display()))));
        assert_eq!(target("r.png", &dir), Ok(Marker::Render(format!("{}/r.png", real.display()))));
        assert_eq!(target("localhost:5173/pricing", &dir), Ok(Marker::Show("http://localhost:5173/pricing".into())));
        assert_eq!(target("https://example.com/a", &dir), Ok(Marker::Show("https://example.com/a".into())));
        assert!(target("notes.md", &dir).is_err() && target("missing.html", &dir).is_err() && target(" ", &dir).is_err());
        // A folder: its index.
        std::fs::create_dir_all(dir.join("proto b")).unwrap();
        std::fs::write(dir.join("proto b/index.html"), "<h1>b</h1>").unwrap();
        let Ok(Marker::Show(url)) = target("proto b", &dir) else { panic!("a folder with an index is a page") };
        assert_eq!(url, format!("file://{}/proto%20b/index.html", real.display()));
        assert_eq!(file_path(&url), Some(real.join("proto b/index.html")), "its file, read back");
        assert_eq!(file_path("http://localhost:5173/"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_session_page_is_served_from_its_design_folder_inside_where_the_member_works() {
        let root = Path::new("/w/t12");
        assert_eq!(page_scope(Path::new("/w/t12/design/sessions/pricing/a.html"), root), Path::new("/w/t12/design"));
        assert_eq!(page_scope(Path::new("/w/t12/site/a.html"), root), Path::new("/w/t12/site"));
        assert_eq!(page_scope(Path::new("/design/t12/x/a.html"), Path::new("/design/t12")), Path::new("/design/t12/x"), "never above where it works");
        assert!(nobody_line("http://127.0.0.1:1/a").contains("Don't tell them it's on their screen"));
    }

    #[test]
    fn a_ticket_or_screen_is_proposed_once_and_not_now_holds_for_good() {
        let proposal = SessionProposal { ticket: Some(7), screen: "Pricing".into(), project: Some("web".into()), why: "A new screen.".into() };
        assert_eq!(key(Some(7), Some("web"), "x"), "ticket:7");
        assert_eq!(key(None, Some("web"), "  Pricing   Page "), "screen:web/pricing page");
        let mut made = vec![];
        assert!(may_propose(&made, "ticket:7").is_ok());
        made.push(Proposed { key: "ticket:7".into(), proposal: proposal.clone(), set: 9, at: 1, tune: false, outcome: None });
        assert!(may_propose(&made, "ticket:7").unwrap_err().contains("already on the person's card"));
        made[0].outcome = Some(ProposalOutcome::NotNow);
        assert!(may_propose(&made, "ticket:7").unwrap_err().contains("Not now"));
        assert!(may_propose(&made, "ticket:8").is_ok());
        let dir = std::env::temp_dir().join(format!("equipe-proposals-{}", std::process::id()));
        write(&dir, &made).unwrap();
        assert_eq!(read(&dir), made);
        let _ = std::fs::remove_dir_all(&dir);
        // The card: why in a line, the three choices; the tuner first when the profile is thin.
        let q = question(&proposal, false, Lang::En);
        assert!(q.question.ends_with("A new screen."), "{}", q.question);
        assert_eq!(q.options.len(), 3);
        assert!(q.options[0].recommended);
        assert_eq!(question(&proposal, true, Lang::En).options.len(), 4);
        assert_eq!(chosen(false, Some(&Answer::pick(0))), Choose::Start);
        assert_eq!(chosen(true, Some(&Answer::pick(1))), Choose::Tune);
        assert_eq!(chosen(true, Some(&Answer::pick(3))), Choose::GoAhead);
        assert_eq!(chosen(false, Some(&Answer::pick(1))), Choose::NotNow);
        assert_eq!(chosen(false, None), Choose::NotNow, "dismissed: not now");
    }
}
