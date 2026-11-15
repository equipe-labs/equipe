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

/// What the designer is told when a session starts, inside the kickoff block.
const HOW: &str = "This is a design session: work as your `design-session` skill says. Small steps of minutes, \
not hours. In this first turn: 3-6 quick reference picks and one rough take, shown live with `equipe design session \
show`. End every turn with something visible and one focused question; never go quiet on a long build.";

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
}

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
        out.extend(pin.crop.map(|c| format!("  crop: attached picture {c}")));
    }
    out.push("Address each pin, then mark it with `equipe design session done <n> \"<what changed, one line>\"`.".to_owned());
    out.push(PINS_CLOSE.to_owned());
    out.join("\n")
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

/// An address the live pane may open: a web page (`http(s)://`) or a page
/// file (`file:///…`), with no space in it.
pub fn is_page_address(url: &str) -> bool {
    !url.contains(char::is_whitespace) && ["http://", "https://", "file:///"].iter().any(|s| url.len() > s.len() && url.to_ascii_lowercase().starts_with(s))
}

/// What `equipe design session show <target>` makes of its target: a web
/// address or `localhost:port` as it is, an HTML file as its `file://`
/// address, a picture as a render. `cwd` resolves a relative path.
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
    fn the_kickoff_names_the_page_and_how_to_work_and_reads_back() {
        let text = kickoff("Pricing\npage", Some("web"), &SessionFrom::Ticket { ticket: 12 }, None, "Let's work on it together.");
        assert!(text.starts_with("<design-session>\npage: Pricing page\nproject: web\nstarted from: ticket T-12\nstyle profile: thin"), "{text}");
        assert!(text.contains("`design-session` skill") && text.contains("3-6 quick reference picks") && text.contains("one focused question"));
        assert_eq!(split_kickoff(&text), Some(("Pricing page".into(), "Let's work on it together.".into())));
        let full = StyleProfile { density: "airy".into(), tone: "calm".into(), typography: "serif".into(), ..Default::default() };
        assert!(block("Home", None, &SessionFrom::Pm { ticket: None }, Some(&full)).contains("style profile: read it first"));
        assert_eq!(split_kickoff("just words"), None);
    }

    #[test]
    fn pins_go_before_the_words_with_where_and_why_and_their_crop() {
        let pins = [
            Pin { n: 1, x: 812, y: 1340, width: 1440, note: "  Too\nloud ".into(), selector: Some("main > h1".into()), reasons: vec![PickReason::Colors], crop: Some(1) },
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
        let _ = std::fs::remove_dir_all(&dir);
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
