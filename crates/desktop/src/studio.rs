//! The designer studio's rules (`views/studio.rs` draws it): a ticket's design
//! work, watched — where the member is in its process, what it is making,
//! and what it left on the way — read from two places only, never guessed:
//! the member's own conversation on the ticket (the skills it opened, the
//! `equipe design` commands it ran and what they printed, its hand-ins), and
//! what its worktree's design folder holds (`equipe_core::protocol::studio`,
//! asked of the daemon while the studio is open). Works the same for a Claude
//! member and a Codex one — each engine's steps are read as they arrive — and
//! for any member's UI ticket, not only a designer's. No UI types here.

use std::path::Path;
use std::time::Instant;

use equipe_core::design::lint::Summary;
use equipe_core::design::lint::record::Verdict;
use equipe_core::protocol::engines::Reviewed;
use equipe_core::protocol::studio::{PictureFrom, Studio, StudioCheck, StudioLook, StudioPicture};
use equipe_core::protocol::{Entry, Ticket, TicketId, TicketStatus, Transcript};
use equipe_core::t;

use crate::checks::{self, Ran};

/// How often the design folder is asked about while the studio is open.
pub const EVERY: std::time::Duration = std::time::Duration::from_millis(2000);

/// The widths a screen is drawn at (`equipe design lint --render`).
pub const WIDTHS: [u32; 7] = equipe_core::design::lint::WIDTHS;

/// The studio's state this launch.
#[derive(Default)]
pub struct StudioUi {
    /// The ticket watched.
    pub ticket: Option<TicketId>,
    /// What its design folder held when last asked, and for which ticket.
    pub files: Option<Studio>,
    /// An ask is on its way.
    pub asking: bool,
    /// When the folder was last asked about.
    pub asked_at: Option<Instant>,
    /// The width picked in the centre; `None`: the usual one.
    pub width: Option<u32>,
    /// A timeline item opened in the centre, by its key.
    pub open: Option<String>,
    /// A picture picked from the strip of recent ones, shown in the centre
    /// instead of the newest; `None`: the newest, as it lands.
    pub picked: Option<String>,
    /// The asks' loop: dropped, it stops.
    pub poll: Option<gpui_kit::Task<()>>,
    pub scroll: gpui_kit::ScrollHandle,
    /// The studio's own width as last drawn: beside the sidebar, less the conversation docked.
    pub room: f32,
}

impl StudioUi {
    /// What the folder holds, for the ticket watched.
    pub fn files(&self) -> Option<&Studio> {
        self.files.as_ref().filter(|f| Some(f.ticket) == self.ticket)
    }

    /// Another ticket watched: nothing of the last one holds.
    pub fn watch(&mut self, ticket: TicketId) {
        if self.ticket != Some(ticket) {
            self.ticket = Some(ticket);
            self.files = None;
            self.width = None;
            self.open = None;
            self.picked = None;
        }
    }

    /// The pictures whose files changed since the last answer: shown again.
    pub fn arrived(&mut self, studio: Studio) -> Vec<String> {
        let before: std::collections::HashMap<String, u64> = self.files().map(|f| pictures_of(f).into_iter().map(|p| (p.path.clone(), p.modified)).collect()).unwrap_or_default();
        let changed = pictures_of(&studio).into_iter().filter(|p| before.get(&p.path).is_some_and(|m| *m != p.modified)).map(|p| p.path.clone()).collect();
        self.files = Some(studio);
        changed
    }
}

fn pictures_of(s: &Studio) -> Vec<&StudioPicture> {
    s.checks.iter().flat_map(|c| &c.pictures).chain(s.directions.iter().flat_map(|d| &d.pictures)).chain(s.references.iter().flat_map(|r| &r.pictures)).chain(&s.pictures).collect()
}

// ── the work, from the conversation ──

/// The lines of the member's conversation that are its work on `ticket`:
/// from that ticket's header to the next one (or a thread's start), each
/// with when it was said, if known.
pub fn work_of(transcript: &Transcript, ticket: TicketId) -> Vec<(&Entry, Option<u64>)> {
    let header = format!("── T-{ticket} ");
    let mut on = false;
    let mut out = Vec::new();
    for (ix, entry) in transcript.entries().iter().enumerate() {
        match entry {
            Entry::Note { text, .. } if text.starts_with("── T-") => on = text.starts_with(&header),
            Entry::Thread { .. } => on = false,
            _ => {}
        }
        if on {
            out.push((entry, transcript.at(ix)));
        }
    }
    out
}

/// What a step of the work did, read from its tool and target.
#[derive(Debug, Clone, PartialEq)]
pub enum Did {
    /// Opened a skill — a how-to — by its name (`design-process`; empty when the engine didn't say).
    Skill(String),
    /// Read the brief or the design system.
    Brief,
    /// Looked something up: the web, a page, the design library.
    Looked,
    /// Proposed a direction, by its name.
    Proposed(String),
    /// Chose one (the person's pick, recorded).
    Chose(String),
    /// Wrote a UI file (a mockup, a page, a style).
    Built(String),
    /// A design check, as it printed its summary.
    Checked(Box<Summary>),
    /// A second look, its verdict.
    Looked2 { screen: String, verdict: Box<Verdict> },
    /// Sent pictures to a fresh reviewer for a critique.
    Critiqued,
    /// Worked in the shared browser.
    Browsed,
    /// Handed in: taken, or refused with why.
    HandIn { ok: bool, why: Vec<String> },
    /// Opened a picture to look at it.
    Opened(String),
    /// Took a picture: a simulator's or an emulator's screenshot, a shot of a page.
    Shot,
    /// Worked the ticket's phone: `look`, `act`, `record`, `frames`…
    Phone(String),
    /// Read a file that isn't a picture: code, a note.
    Read(String),
    /// Looked at what a branch changed (`git diff`, `git show`, `git log`).
    Diff,
    /// Handed part of the work to a helper: fresh eyes, a search.
    Helper,
    /// Wrote a report into the hand-in folder.
    Report(String),
    /// Gave a design QA's verdict.
    Verdict,
    /// Asked the person something, in its words.
    Asked(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Happening {
    pub at: Option<u64>,
    pub did: Did,
}

/// The skill a step opens: Claude's Skill step (`equipe:design-process`), or
/// a read of its file, as a Codex member opens one (`…/skills/design-process/SKILL.md`).
pub fn skill_of(tool: &str, target: &str) -> Option<String> {
    if tool == "Skill" {
        return Some(target.trim().trim_start_matches("equipe:").to_owned());
    }
    if !matches!(tool, "Read" | "Bash") {
        return None;
    }
    let at = target.find("/skills/")? + "/skills/".len();
    let name: String = target[at..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    let rest = &target[at + name.len()..];
    (!name.is_empty() && (rest.is_empty() || rest.starts_with('/') || rest.starts_with(char::is_whitespace))).then_some(name)
}

/// A file a person sees drawn: a page, its styles, a component, a picture made by hand.
pub fn is_ui_file(path: &str) -> bool {
    const KINDS: [&str; 13] = ["html", "htm", "css", "scss", "sass", "less", "tsx", "jsx", "vue", "svelte", "astro", "svg", "swift"];
    const NOT: [&str; 5] = ["/design/checks/", "/design/critiques/", "/design/references/", "/node_modules/", "/design/decisions/"];
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    KINDS.contains(&ext.as_str()) && !NOT.iter().any(|n| path.contains(n))
}

/// The direction a `design direction propose|choose <name>` names.
fn direction_name(target: &str, verb: &str) -> Option<String> {
    let at = target.find(&format!("design direction {verb} "))? + format!("design direction {verb} ").len();
    let rest = target[at..].trim_start();
    let name = match rest.chars().next()? {
        q @ ('"' | '\'') => rest[1..].split(q).next()?.to_owned(),
        _ => rest.split_whitespace().next()?.to_owned(),
    };
    (!name.is_empty() && !name.starts_with('-')).then_some(name)
}

/// What a hand-in that was refused said: its lines that say what to do.
fn refusal(output: &str) -> Vec<String> {
    let listed: Vec<String> = output.lines().filter_map(|l| l.trim().strip_prefix("- ")).map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()).collect();
    if listed.is_empty() { output.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_owned).into_iter().collect() } else { listed }
}

/// A command that takes a picture: a simulator's or an emulator's
/// screenshot or recording, a page shot.
pub fn is_shot(command: &str) -> bool {
    let c = command.to_ascii_lowercase();
    (c.contains("simctl io") && (c.contains("screenshot") || c.contains("recordvideo"))) || c.contains("screencap") || c.contains("screenrecord") || c.contains("equipe shoot") || c.contains("screencapture")
}

/// What each step of the work did, in order — only the steps that say
/// something about the design process.
pub fn happenings(work: &[(&Entry, Option<u64>)]) -> Vec<Happening> {
    let mut out = Vec::new();
    for (entry, at) in work {
        let Entry::Tool { tool, target, outcome, .. } = entry else { continue };
        let target = target.as_deref().unwrap_or("");
        let output = outcome.as_ref().map(|o| o.output.as_str()).unwrap_or("");
        let done = outcome.as_ref().map(|o| o.ok);
        let did = if let Some(name) = skill_of(tool, target) {
            Some(Did::Skill(name))
        } else if tool == "Read" && equipe_core::visuals::is_image_name(target) {
            Some(Did::Opened(target.to_owned()))
        } else if let Some(ran) = outcome.as_ref().and_then(|o| checks::ran(tool, target, &o.output)) {
            Some(match ran {
                Ran::Check(summary) => Did::Checked(summary),
                Ran::Look { screen, verdict } => Did::Looked2 { screen, verdict },
            })
        } else if tool == "Bash" && target.contains("design direction propose") {
            direction_name(target, "propose").map(Did::Proposed)
        } else if tool == "Bash" && target.contains("design direction choose") {
            direction_name(target, "choose").map(Did::Chose)
        } else if tool == "Bash" && target.contains("design critique") {
            Some(Did::Critiqued)
        } else if tool == "mcp__equipe__submit_for_review" {
            done.map(|ok| Did::HandIn { ok, why: if ok { Vec::new() } else { refusal(output) } })
        } else if tool.starts_with("mcp__browser") {
            Some(Did::Browsed)
        } else if matches!(tool.as_str(), "WebSearch" | "WebFetch") || tool.contains("design_library") || tool == "Bash" && (target.contains("equipe shoot") || target.contains("design/references")) {
            Some(Did::Looked)
        } else if matches!(tool.as_str(), "Read" | "Bash") && ["brand.md", "guidelines.md", "tokens.json"].iter().any(|f| target.ends_with(f) || target.contains(&format!("/{f} "))) {
            Some(Did::Brief)
        } else if matches!(tool.as_str(), "Edit" | "Write" | "MultiEdit") && is_ui_file(target) {
            Some(Did::Built(target.to_owned()))
        } else if matches!(tool.as_str(), "Edit" | "Write") && target.contains("/design/references/") {
            Some(Did::Looked)
        } else if matches!(tool.as_str(), "Edit" | "Write") && target.contains("/outputs/") && target.ends_with(".md") {
            Some(Did::Report(target.to_owned()))
        } else if let Some(action) = tool.strip_prefix("mcp__phone__") {
            Some(Did::Phone(action.to_owned()))
        } else if tool == "Bash" && is_shot(target) {
            Some(Did::Shot)
        } else if tool == "Bash" && ["git diff", "git show", "git log"].iter().any(|g| target.contains(g)) {
            Some(Did::Diff)
        } else if tool == "Read" {
            Some(Did::Read(target.to_owned()))
        } else if matches!(tool.as_str(), "Agent" | "Task") {
            Some(Did::Helper)
        } else if tool == "mcp__equipe__design_qa" {
            Some(Did::Verdict)
        } else if tool == "mcp__equipe__ask_decision" {
            Some(Did::Asked(target.to_owned()))
        } else {
            None
        };
        if let Some(did) = did {
            out.push(Happening { at: *at, did });
        }
    }
    out
}

// ── the steps ──

/// What kind of design work a ticket is: the steps it goes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flow {
    /// A new design — a direction, a brand, a flow: the whole path.
    Design,
    /// Design QA, or any review of someone else's screens.
    Qa,
    /// Building screens in code: no references or directions to go through.
    Build,
}

impl Flow {
    /// Its steps, in order.
    pub fn steps(self) -> &'static [StepKind] {
        match self {
            Flow::Design => &[StepKind::Brief, StepKind::References, StepKind::Directions, StepKind::Build, StepKind::Checks, StepKind::HandIn],
            Flow::Qa => &[StepKind::Brief, StepKind::Change, StepKind::Shots, StepKind::Measure, StepKind::Findings, StepKind::Verdict],
            Flow::Build => &[StepKind::Brief, StepKind::Build, StepKind::Checks, StepKind::HandIn],
        }
    }

    /// Its name, as the studio's header says it.
    pub fn label(self) -> &'static str {
        match self {
            Flow::Design => t!("studio.flow_design"),
            Flow::Qa => t!("studio.flow_qa"),
            Flow::Build => t!("studio.flow_build"),
        }
    }
}

/// The kind of work `ticket` is: a review when another ticket waits on it
/// for design QA or the member opened the design QA guide for it; building
/// in code when it is a code ticket that went through no references or
/// directions; else the whole design path.
pub fn flow_of(ticket: &Ticket, reviews: bool, happenings: &[Happening]) -> Flow {
    let qa = reviews || happenings.iter().any(|h| matches!(&h.did, Did::Skill(s) if s == "design-qa") || matches!(h.did, Did::Verdict));
    let explored = happenings.iter().any(|h| matches!(h.did, Did::Proposed(_) | Did::Chose(_) | Did::Looked));
    if qa {
        Flow::Qa
    } else if ticket.kind == equipe_core::protocol::TicketKind::Code && !explored {
        Flow::Build
    } else {
        Flow::Design
    }
}

/// The process's steps, in the person's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepKind {
    Brief,
    References,
    Directions,
    Build,
    Checks,
    HandIn,
    /// A review: what the change is (its diff, its code).
    Change,
    /// A review: the screens seen for real — screenshots, the phone, pictures opened.
    Shots,
    /// A review: held to the rules and measured — checks, fresh eyes.
    Measure,
    /// A review: what it found, written down.
    Findings,
    /// A review: its word on the change.
    Verdict,
}

impl StepKind {
    pub fn label(self) -> &'static str {
        match self {
            StepKind::Brief => t!("studio.step_brief"),
            StepKind::References => t!("studio.step_references"),
            StepKind::Directions => t!("studio.step_directions"),
            StepKind::Build => t!("studio.step_build"),
            StepKind::Checks => t!("studio.step_checks"),
            StepKind::HandIn => t!("studio.step_handin"),
            StepKind::Change => t!("studio.step_change"),
            StepKind::Shots => t!("studio.step_shots"),
            StepKind::Measure => t!("studio.step_measure"),
            StepKind::Findings => t!("studio.step_findings"),
            StepKind::Verdict => t!("studio.step_verdict"),
        }
    }

    /// What the step is, in a sentence: its tooltip.
    pub fn about(self) -> &'static str {
        match self {
            StepKind::Brief => t!("studio.step_brief_tip"),
            StepKind::References => t!("studio.step_references_tip"),
            StepKind::Directions => t!("studio.step_directions_tip"),
            StepKind::Build => t!("studio.step_build_tip"),
            StepKind::Checks => t!("studio.step_checks_tip"),
            StepKind::HandIn => t!("studio.step_handin_tip"),
            StepKind::Change => t!("studio.step_change_tip"),
            StepKind::Shots => t!("studio.step_shots_tip"),
            StepKind::Measure => t!("studio.step_measure_tip"),
            StepKind::Findings => t!("studio.step_findings_tip"),
            StepKind::Verdict => t!("studio.step_verdict_tip"),
        }
    }

    /// A step where checks fail shows it.
    fn checks(self) -> bool {
        matches!(self, StepKind::Checks | StepKind::HandIn | StepKind::Measure | StepKind::Findings | StepKind::Verdict)
    }
}

/// Where a step stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Not reached yet.
    Ahead,
    /// Passed by without a trace: the work didn't need it.
    Skipped,
    Done,
    /// Where the member is now.
    Now,
    /// Where it is now, and something there fails.
    Attention,
    /// Where it is now, waiting on the person.
    Waiting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub kind: StepKind,
    pub mark: Mark,
    /// How much of it there is, in words: `3 guides`, `Floor 11/11 · 7/7 widths`.
    pub count: Option<String>,
    /// When it last moved, in unix seconds.
    pub at: Option<u64>,
    /// What it holds, named: the how-tos opened.
    pub detail: Option<String>,
}

/// What the person is waited on for, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitFor {
    /// Directions proposed, none picked.
    Pick,
    /// A question of the member's, in its words — on another ticket, which.
    Answer { what: String, ticket: Option<TicketId> },
    /// The person's OK to run something, as the member asked it.
    Permission(String),
    /// Another ticket of the member's waits on the person.
    Ticket(TicketId),
    /// This ticket waits on the person.
    Something,
}

impl WaitFor {
    /// About this ticket, not another.
    pub fn is_here(&self) -> bool {
        !matches!(self, WaitFor::Ticket(_) | WaitFor::Answer { ticket: Some(_), .. })
    }
}

/// Everything the studio is drawn from, gathered once a frame.
pub struct Watch<'a> {
    pub ticket: &'a Ticket,
    pub happenings: Vec<Happening>,
    pub files: Option<&'a Studio>,
    /// The member is at it right now.
    pub working: bool,
    pub waiting: Option<WaitFor>,
    pub flow: Flow,
}

/// The latest check, from what it printed, or from its record when the
/// conversation's lines don't reach back that far.
fn latest_check(w: &Watch) -> Option<(Option<u64>, Summary)> {
    let ran = w.happenings.iter().rev().find_map(|h| match &h.did {
        Did::Checked(s) => Some((h.at, (**s).clone())),
        _ => None,
    });
    let kept = w.files.and_then(|f| f.checks.first()).map(|c| (Some(c.at), summary_of(c)));
    match (ran, kept) {
        (Some(r), Some(k)) if k.0 > r.0.map(|a| a + 5) => Some(k),
        (Some(r), _) => Some(r),
        (None, k) => k,
    }
}

/// A check record's summary, as a run prints it.
pub fn summary_of(c: &StudioCheck) -> Summary {
    Summary {
        floor: (c.floor.0 as usize, c.floor.1 as usize),
        habits: c.habits as usize,
        finish: c.finish as usize,
        widths: c.pictures.iter().filter_map(|p| p.width).collect(),
        failing: c.failing.clone(),
        open: c.open.clone(),
        screen: Some(c.screen.clone()),
        ..Summary::default()
    }
}

/// The latest second look: its verdict, pass or how many fixes.
fn latest_look(w: &Watch) -> Option<(Option<u64>, bool, usize)> {
    let ran = w.happenings.iter().rev().find_map(|h| match &h.did {
        Did::Looked2 { verdict, .. } => Some((h.at.or(Some(verdict.at)), verdict.pass, verdict.fixes.len())),
        _ => None,
    });
    let kept = w.files.and_then(|f| f.checks.iter().filter_map(|c| c.look.as_ref()).max_by_key(|l| l.at)).map(|l| (Some(l.at), l.pass, l.fixes.len()));
    match (ran, kept) {
        (Some(r), Some(k)) if k.0 > r.0.map(|a| a + 5) => Some(k),
        (Some(r), _) => Some(r),
        (None, k) => k,
    }
}

/// The last hand-in, if one was tried.
fn last_hand_in<'a>(w: &'a Watch<'a>) -> Option<(Option<u64>, bool, &'a [String])> {
    w.happenings.iter().rev().find_map(|h| match &h.did {
        Did::HandIn { ok, why } => Some((h.at, *ok, why.as_slice())),
        _ => None,
    })
}

/// The last hand-in was refused, and the member hasn't checked or looked
/// again since: the refusal still stands.
fn refusal_stands<'a>(w: &'a Watch<'a>) -> Option<&'a [String]> {
    let refused = w.happenings.iter().rposition(|h| matches!(h.did, Did::HandIn { ok: false, .. }))?;
    let again = w.happenings[refused..].iter().any(|h| matches!(h.did, Did::Checked(_) | Did::Looked2 { .. } | Did::HandIn { ok: true, .. }));
    match &w.happenings[refused].did {
        Did::HandIn { why, .. } if !again => Some(why.as_slice()),
        _ => None,
    }
}

/// Handed in and taken: by a hand-in it saw, or by where the ticket is now.
pub fn handed_in(w: &Watch) -> bool {
    matches!(w.ticket.status, TicketStatus::Review | TicketStatus::Done) || last_hand_in(w).is_some_and(|(_, ok, _)| ok)
}

/// The steps, each lit from what happened.
pub fn steps(w: &Watch) -> Vec<Step> {
    let of = |f: &dyn Fn(&Did) -> bool| -> (usize, Option<u64>, Option<u64>) {
        let hits: Vec<&Happening> = w.happenings.iter().filter(|h| f(&h.did)).collect();
        (hits.len(), hits.iter().filter_map(|h| h.at).min(), hits.iter().filter_map(|h| h.at).max())
    };
    let files = w.files;
    // Brief: the guides opened, and the brief read.
    let mut guides: Vec<&str> = w.happenings.iter().filter_map(|h| if let Did::Skill(s) = &h.did { Some(s.as_str()) } else { None }).collect();
    guides.sort_unstable();
    guides.dedup();
    let (briefs, _, brief_last) = of(&|d| matches!(d, Did::Skill(_) | Did::Brief));
    // References: the board, else the look-ups.
    let board = files.map_or(0, |f| f.references.len());
    let (lookups, _, looked_last) = of(&|d| matches!(d, Did::Looked));
    let board_last = files.and_then(|f| f.references.iter().map(|r| r.modified).max());
    // Directions: proposed and picked.
    let proposed = files.map_or(0, |f| f.directions.len()).max(of(&|d| matches!(d, Did::Proposed(_))).0);
    let chosen = files.and_then(|f| f.directions.iter().rev().find(|d| d.chosen).map(|d| d.name.clone())).or_else(|| {
        w.happenings.iter().rev().find_map(|h| if let Did::Chose(n) = &h.did { Some(n.clone()) } else { None })
    });
    let (_, _, directions_last) = of(&|d| matches!(d, Did::Proposed(_) | Did::Chose(_)));
    let directions_last = directions_last.or_else(|| files.and_then(|f| f.directions.iter().map(|d| d.at).max()));
    // Build: the UI files written.
    let mut built: Vec<&str> = w.happenings.iter().filter_map(|h| if let Did::Built(f) = &h.did { Some(f.as_str()) } else { None }).collect();
    built.sort_unstable();
    built.dedup();
    let (_, _, built_last) = of(&|d| matches!(d, Did::Built(_)));
    // Checks: the runs, the widths drawn, the looks.
    // A critique can come at any step (directions are critiqued too): it lights none on its own.
    let (runs, _, checks_last) = of(&|d| matches!(d, Did::Checked(_) | Did::Looked2 { .. }));
    let check = latest_check(w);
    let look = latest_look(w);
    let checked = runs > 0 || files.is_some_and(|f| !f.checks.is_empty());
    let hand_in = last_hand_in(w);
    let handed = handed_in(w);

    // A review's own steps: the change looked at, the screens seen, measured, found, judged.
    let (reads, _, change_last) = of(&|d| matches!(d, Did::Read(_) | Did::Diff));
    let shots_seen = w.happenings.iter().filter(|h| matches!(&h.did, Did::Shot | Did::Opened(_)) || matches!(&h.did, Did::Phone(a) if matches!(a.as_str(), "look" | "frames" | "record"))).count();
    let shots_kept = files.map_or(0, |f| f.pictures.iter().filter(|p| matches!(p.from, PictureFrom::Recording | PictureFrom::Opened | PictureFrom::Made | PictureFrom::Output)).count());
    let (_, _, shots_last) = of(&|d| matches!(d, Did::Shot | Did::Opened(_) | Did::Phone(_)));
    let (helpers, _, helper_last) = of(&|d| matches!(d, Did::Helper | Did::Critiqued));
    let found = files.map(findings).unwrap_or_default();
    let (reported, _, report_last) = of(&|d| matches!(d, Did::Report(_)));
    let report_last = report_last.or_else(|| files.and_then(|f| f.reports.first().map(|r| r.at)));
    let (verdicts, _, verdict_last) = of(&|d| matches!(d, Did::Verdict | Did::HandIn { ok: true, .. }));
    let steps = w.flow.steps();
    let reached: Vec<bool> = steps
        .iter()
        .map(|kind| match kind {
            StepKind::Brief => briefs > 0,
            StepKind::References => board > 0 || lookups > 0,
            StepKind::Directions => proposed > 0,
            StepKind::Build => !built.is_empty() || files.is_some_and(|f| !f.pending.is_empty()),
            StepKind::Checks => checked,
            StepKind::HandIn => handed || refusal_stands(w).is_some(),
            StepKind::Change => reads > 0,
            StepKind::Shots => shots_seen > 0 || shots_kept > 0,
            StepKind::Measure => checked || helpers > 0,
            StepKind::Findings => reported > 0 || !found.is_empty(),
            StepKind::Verdict => handed || verdicts > 0,
        })
        .collect();
    // Where it is now: the furthest step reached.
    let now = reached.iter().rposition(|r| *r);
    let failing = check.as_ref().is_some_and(|(_, s)| !s.passes()) || look.is_some_and(|(_, pass, _)| !pass) || refusal_stands(w).is_some();
    let mut out = Vec::new();
    for (ix, kind) in steps.iter().copied().enumerate() {
        let mark = match now {
            _ if handed => if reached[ix] { Mark::Done } else { Mark::Skipped },
            Some(n) if ix < n => if reached[ix] { Mark::Done } else { Mark::Skipped },
            Some(n) if ix == n => match () {
                // Only a wait about this ticket holds its step up; one on another ticket doesn't.
                _ if w.waiting.as_ref().is_some_and(WaitFor::is_here) => Mark::Waiting,
                _ if failing && kind.checks() => Mark::Attention,
                _ => Mark::Now,
            },
            // Waiting on a pick before anything is built: the directions step waits.
            None if ix == 0 && w.working => Mark::Now,
            _ => Mark::Ahead,
        };
        let (count, at) = match kind {
            StepKind::Change => ((reads > 0).then(|| t!("studio.count_read_files", count = reads)), change_last),
            StepKind::Shots => ((shots_seen.max(shots_kept) > 0).then(|| t!("studio.count_pictures", count = shots_seen.max(shots_kept))), shots_last),
            StepKind::Measure => (match (check.as_ref(), helpers) {
                (Some((_, s)), 0) => Some(checks::floor(s)),
                (Some((_, s)), n) => Some(format!("{} · {}", checks::floor(s), t!("studio.count_helpers", count = n))),
                (None, 0) => None,
                (None, n) => Some(t!("studio.count_helpers", count = n)),
            }, checks_last.max(helper_last)),
            StepKind::Findings => ((!found.is_empty() || reported > 0).then(|| if found.is_empty() { t!("studio.count_report").to_owned() } else { t!("studio.count_findings", count = found.len()) }), report_last),
            StepKind::Verdict => ((handed || verdicts > 0).then(|| t!("studio.count_verdict").to_owned()), verdict_last.or(hand_in.and_then(|h| h.0))),
            StepKind::Brief => ((briefs > 0).then(|| if guides.is_empty() { t!("studio.count_read").to_owned() } else { t!("studio.count_guides", count = guides.len()) }), brief_last),
            StepKind::References => (match (board, lookups) {
                (0, 0) => None,
                (0, n) => Some(t!("studio.count_lookups", count = n)),
                (n, _) => Some(t!("studio.count_references", count = n)),
            }, board_last.max(looked_last)),
            StepKind::Directions => ((proposed > 0).then(|| match &chosen {
                Some(name) => format!("{} · {}", t!("studio.count_directions", count = proposed), t!("studio.count_picked", name = name)),
                None => t!("studio.count_directions", count = proposed),
            }), directions_last),
            StepKind::Build => ((!built.is_empty()).then(|| t!("studio.count_files", count = built.len())), built_last),
            StepKind::Checks => (check.as_ref().map(|(_, s)| {
                let widths = s.widths.iter().filter(|w| WIDTHS.contains(w)).count();
                let mut parts = vec![checks::floor(s)];
                if widths > 0 {
                    parts.push(t!("studio.count_widths", n = widths, total = WIDTHS.len()));
                }
                if let Some((_, pass, fixes)) = look {
                    parts.push(if pass { t!("look.pass").to_owned() } else { t!("look.fixes", count = fixes) });
                }
                parts.join(" · ")
            }), checks_last.max(check.as_ref().and_then(|c| c.0))),
            StepKind::HandIn => (match hand_in {
                Some((_, true, _)) => Some(t!("studio.count_handed").to_owned()),
                _ if handed => Some(t!("studio.count_handed").to_owned()),
                Some((_, false, _)) => Some(t!("studio.count_refused", count = w.happenings.iter().filter(|h| matches!(h.did, Did::HandIn { ok: false, .. })).count())),
                None => None,
            }, hand_in.and_then(|h| h.0)),
        };
        let detail = (kind == StepKind::Brief && !guides.is_empty()).then(|| guides.iter().map(|g| skill_words(g)).collect::<Vec<_>>().join(" · "));
        out.push(Step { kind, mark, count, at, detail });
    }
    out
}

// ── the line that says how it is going ──

/// Where the member is with the ticket, as the sidebar would say it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presence {
    Waiting(WaitFor),
    HandedIn { at: Option<u64> },
    /// At it since it began the ticket.
    Working { since: Option<u64> },
    /// Not at it now: its last step then.
    Idle { last: Option<u64> },
    NotStarted,
}

/// Something the work is held up by, said beside where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// The last hand-in was refused, and why (its first line).
    Refused(String),
    /// The latest check fails the floor: the rule, in words, and what it saw.
    Failing { rule: String, evidence: String },
    /// The second look asked for this many fixes.
    Fixing(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub presence: Presence,
    pub issue: Option<Issue>,
}

/// How it is going, honestly: where the member is — waiting for the person
/// first, as the sidebar says it — and what holds the work up, if anything.
pub fn status(w: &Watch, first: Option<u64>, last: Option<u64>) -> Status {
    let presence = if let Some(wait) = &w.waiting {
        Presence::Waiting(wait.clone())
    } else if handed_in(w) {
        Presence::HandedIn { at: last_hand_in(w).and_then(|h| h.0) }
    } else if w.working {
        Presence::Working { since: first }
    } else if last.is_some() || w.files.is_some_and(|f| f.dir.is_some() || !f.pictures.is_empty()) {
        Presence::Idle { last }
    } else {
        Presence::NotStarted
    };
    let issue = if handed_in(w) {
        None
    } else if let Some(why) = refusal_stands(w) {
        // A refusal stands until the member has checked or looked again since.
        Some(Issue::Refused(why.first().cloned().unwrap_or_default()))
    } else if let Some((_, s)) = latest_check(w).filter(|(_, s)| !s.passes())
        && let Some((id, evidence)) = s.failing.first()
    {
        Some(Issue::Failing { rule: checks::rule_name(id), evidence: evidence.clone() })
    } else if let Some((_, false, fixes)) = latest_look(w) {
        Some(Issue::Fixing(fixes))
    } else {
        None
    };
    Status { presence, issue }
}

/// Where it is, in words.
pub fn presence_words(p: &Presence, now: u64) -> String {
    let ago = |at: u64| crate::activity::ago(now.saturating_sub(at));
    match p {
        Presence::HandedIn { at: Some(at) } => t!("studio.status_handed_at", when = ago(*at)),
        Presence::HandedIn { at: None } => t!("studio.status_handed").to_owned(),
        Presence::Waiting(WaitFor::Pick) => t!("studio.status_waiting_pick").to_owned(),
        Presence::Waiting(WaitFor::Answer { what, ticket: None }) => t!("studio.status_waiting_answer", what = what),
        Presence::Waiting(WaitFor::Answer { what, ticket: Some(id) }) => t!("studio.status_waiting_elsewhere", id = id, what = what),
        Presence::Waiting(WaitFor::Permission(what)) => t!("studio.status_waiting_ok", what = what),
        Presence::Waiting(WaitFor::Ticket(id)) => t!("studio.status_waiting_ticket", id = id),
        Presence::Waiting(WaitFor::Something) => t!("studio.status_waiting").to_owned(),
        Presence::Working { since: Some(at) } => t!("studio.status_working", span = crate::state::short_span(now.saturating_sub(*at))),
        Presence::Working { since: None } => t!("studio.status_working_now").to_owned(),
        Presence::Idle { last: Some(at) } => t!("studio.status_resting", when = ago(*at)),
        Presence::Idle { last: None } => t!("studio.status_resting_now").to_owned(),
        Presence::NotStarted => t!("studio.status_not_started").to_owned(),
    }
}

/// What holds it up, in words.
pub fn issue_words(i: &Issue) -> String {
    match i {
        Issue::Refused(why) => t!("studio.status_refused", why = why),
        Issue::Failing { rule, evidence } => t!("studio.status_failing", rule = rule, evidence = evidence),
        Issue::Fixing(n) => t!("studio.status_fixing", count = n),
    }
}

/// The whole line, as a screen reader says it.
pub fn status_words(s: &Status, now: u64) -> String {
    match &s.issue {
        Some(issue) => format!("{} · {}", presence_words(&s.presence, now), issue_words(issue)),
        None => presence_words(&s.presence, now),
    }
}

/// What tone the line is said in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Good,
    Bad,
    Wait,
    Busy,
}

pub fn status_tone(p: &Presence) -> Tone {
    match p {
        Presence::HandedIn { .. } => Tone::Good,
        Presence::Waiting(_) => Tone::Wait,
        Presence::Working { .. } => Tone::Busy,
        Presence::Idle { .. } | Presence::NotStarted => Tone::Plain,
    }
}

/// What the person is waited on for, in the order the sidebar weighs it
/// (`Model::status`): the member's ask for an OK, then a ticket of its own
/// waiting on the person, then a question of its own — each, on this
/// ticket or another. Directions out with none picked make this ticket's
/// wait a pick.
pub fn waiting_for(this: TicketId, ticket: &Ticket, permission: Option<String>, question: Option<(String, Option<TicketId>)>, waiting: Option<TicketId>, files: Option<&Studio>, happenings: &[Happening]) -> Option<WaitFor> {
    let picked = files.is_some_and(|f| f.directions.iter().any(|d| d.chosen)) || happenings.iter().any(|h| matches!(h.did, Did::Chose(_)));
    let proposed = files.is_some_and(|f| !f.directions.is_empty()) || happenings.iter().any(|h| matches!(h.did, Did::Proposed(_)));
    let pick = proposed && !picked;
    if let Some(what) = permission {
        return Some(WaitFor::Permission(what));
    }
    let here = waiting == Some(this) || ticket.status == TicketStatus::Waiting;
    match (question, waiting) {
        (Some((_, on)), _) if pick && on.is_none_or(|t| t == this) => Some(WaitFor::Pick),
        (Some((what, on)), _) => Some(WaitFor::Answer { what, ticket: on.filter(|t| *t != this) }),
        (None, _) if here => Some(if pick { WaitFor::Pick } else { WaitFor::Something }),
        (None, Some(other)) => Some(WaitFor::Ticket(other)),
        (None, None) => None,
    }
}

// ── the centre: what it is making ──

/// The picture in the centre: a check's, at a width, or the latest one made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Centre {
    /// The screen it is of, when a check drew it.
    pub screen: Option<String>,
    /// The widths there are, each its picture (a check's), narrowest first.
    pub widths: Vec<(u32, String)>,
    pub shown: String,
    pub width: Option<u32>,
    /// When it was made or checked.
    pub at: u64,
    /// The direction it shows, by name, when it shows one.
    pub direction: Option<String>,
    /// Directions out and none picked yet, nothing else drawn: each one's
    /// name and picture, side by side for the person to pick from.
    pub choices: Vec<(String, String)>,
    /// Where the picture came from, when not a check's or a direction's.
    pub from: Option<PictureFrom>,
}

/// The width shown when none is picked: the desktop one most look at.
pub const USUAL_WIDTH: u32 = 1280;

/// What the centre shows: the newest of a check's pictures and any other
/// picture the member made — the check's at the width picked (else the usual
/// one, else the widest).
/// A picture the person picked from the strip shows instead of the newest,
/// while it is still there.
pub fn centre_of(files: &Studio, picked: Option<u32>, path: Option<&str>) -> Option<Centre> {
    if let Some(path) = path {
        if let Some(c) = files.checks.iter().find(|c| c.pictures.iter().any(|p| p.path == path)) {
            let width = c.pictures.iter().find(|p| p.path == path).and_then(|p| p.width);
            let widths: Vec<(u32, String)> = c.pictures.iter().filter_map(|p| p.width.map(|w| (w, p.path.clone()))).collect();
            return Some(Centre { screen: Some(c.screen.clone()), widths, shown: path.to_owned(), width, at: c.at, direction: None, choices: Vec::new(), from: None });
        }
        if let Some(p) = files.pictures.iter().find(|p| p.path == path) {
            return Some(Centre { screen: None, widths: Vec::new(), shown: p.path.clone(), width: None, at: p.modified, direction: None, choices: Vec::new(), from: Some(p.from) });
        }
    }
    let check = files.checks.iter().filter(|c| !c.pictures.is_empty()).max_by_key(|c| c.at);
    let other = files.pictures.iter().max_by_key(|p| p.modified);
    let from_check = check.map(|c| {
        let widths: Vec<(u32, String)> = c.pictures.iter().filter_map(|p| p.width.map(|w| (w, p.path.clone()))).collect();
        let want = picked.or(Some(USUAL_WIDTH));
        let pick = widths.iter().find(|(w, _)| Some(*w) == want).or_else(|| widths.iter().rfind(|(w, _)| *w <= USUAL_WIDTH)).or(widths.last());
        let (width, shown) = match pick {
            Some((w, p)) => (Some(*w), p.clone()),
            None => (None, c.pictures[0].path.clone()),
        };
        let at = c.pictures.iter().map(|p| p.modified).max().unwrap_or(c.at).max(c.at);
        Centre { screen: Some(c.screen.clone()), widths, shown, width, at, direction: None, choices: Vec::new(), from: None }
    });
    let from_other = other.map(|p| Centre { screen: None, widths: Vec::new(), shown: p.path.clone(), width: None, at: p.modified, direction: None, choices: Vec::new(), from: Some(p.from) });
    // A width picked keeps the check's pictures in the centre.
    match (from_check, from_other) {
        (Some(c), Some(o)) if picked.is_none() && o.at > c.at => Some(o),
        (Some(c), _) => Some(c),
        (None, o) => o.or_else(|| {
            let drawn: Vec<_> = files.directions.iter().filter(|d| !d.pictures.is_empty()).collect();
            let d = drawn.iter().rev().find(|d| d.chosen).or_else(|| drawn.last())?;
            let picked = drawn.iter().any(|d| d.chosen);
            let choices = if picked || drawn.len() < 2 { Vec::new() } else { drawn.iter().map(|d| (d.name.clone(), d.pictures[0].path.clone())).collect() };
            Some(Centre { screen: None, widths: Vec::new(), shown: d.pictures[0].path.clone(), width: None, at: d.at, direction: Some(d.name.clone()), choices, from: None })
        }),
    }
}

// ── the timeline: what it left on the way ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Reference,
    Direction,
    Critique,
    Check,
    Look,
    HandIn,
    Report,
}

/// One thing on the timeline, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Stays the same while it is there: what opens it in the centre.
    pub key: String,
    pub kind: ItemKind,
    pub at: Option<u64>,
    pub title: String,
    /// A line or two under its title on the strip; all of them in the centre.
    pub lines: Vec<String>,
    pub pictures: Vec<String>,
    pub link: Option<String>,
    pub tone: Tone,
}

fn check_lines(s: &Summary) -> Vec<String> {
    let mut out = vec![[vec![checks::floor(s)], checks::counts(s)].concat().join(" · ")];
    for d in checks::details(s) {
        let kind = match d.kind {
            checks::Kind::Fails => t!("check.kind_fails"),
            checks::Kind::Habit => t!("check.kind_habit"),
            checks::Kind::Kept => t!("check.kind_kept"),
            checks::Kind::Note => t!("check.kind_note"),
            checks::Kind::Skipped => t!("check.kind_skipped"),
        };
        if matches!(d.kind, checks::Kind::Fails | checks::Kind::Habit) {
            out.push(if d.said.is_empty() { format!("{kind} · {}", d.rule) } else { format!("{kind} · {} — {}", d.rule, d.said) });
        }
    }
    if !s.widths.is_empty() {
        out.push(format!("{} {}", t!("check.drawn"), checks::widths(s)));
    }
    out
}

fn look_lines(pass: bool, summary: &str, fixes: &[(String, String, String)], questions: &[(String, String)], reviewer: Option<&Reviewed>) -> Vec<String> {
    let mut out = vec![if pass { t!("look.pass").to_owned() } else { t!("look.fixes", count = fixes.len()) }];
    // Who looked, beside the verdict: what the timeline shows of it.
    out.extend(reviewer.map(checks::reviewed));
    if !summary.trim().is_empty() {
        out.push(summary.trim().to_owned());
    }
    for (place, what, why) in fixes {
        out.push(if why.is_empty() { format!("{place} — {what}") } else { format!("{place} — {what} ({why})") });
    }
    for (what, wording) in questions {
        out.push(t!("studio.look_question", what = what, wording = wording));
    }
    out
}

/// A fix as its three parts: where, what, why.
type FixParts = (String, String, String);

fn studio_look(l: &StudioLook) -> (Vec<FixParts>, &[(String, String)]) {
    (l.fixes.iter().map(|f| (f.place.clone(), f.what.clone(), f.why.clone())).collect(), &l.questions)
}

/// A check's pictures, the usual width first (what a card shows), then the rest narrowest first.
fn usual_first(pictures: &[StudioPicture]) -> Vec<String> {
    let mut out: Vec<String> = pictures.iter().map(|p| p.path.clone()).collect();
    if let Some(at) = pictures.iter().position(|p| p.width == Some(USUAL_WIDTH)) {
        let usual = out.remove(at);
        out.insert(0, usual);
    }
    out
}

/// Every item, newest first: the board's references, the directions with
/// their pick, the critiques, each check and second look as it ran (or as
/// its record keeps it, when the conversation's lines don't reach back), and
/// each hand-in.
pub fn items(happenings: &[Happening], files: Option<&Studio>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    if let Some(f) = files {
        for r in &f.references {
            let mut lines: Vec<String> = r.notes.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
            if let Some(link) = &r.link {
                lines.push(t!("studio.source", link = link));
            }
            out.push(Item { key: format!("ref:{}", r.name), kind: ItemKind::Reference, at: Some(r.modified), title: r.name.clone(), lines, pictures: r.pictures.iter().map(|p| p.path.clone()).collect(), link: r.link.clone(), tone: Tone::Plain });
        }
        for d in &f.directions {
            let dims = [("studio.dir_layout", &d.layout), ("studio.dir_type", &d.type_), ("studio.dir_palette", &d.palette), ("studio.dir_density", &d.density), ("studio.dir_motion", &d.motion)];
            let mut lines: Vec<String> = Vec::new();
            if d.chosen {
                lines.push(t!("studio.direction_picked").to_owned());
            }
            lines.extend(dims.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| format!("{} · {}", crate::i18n::catalog().get(k), v.trim())));
            out.push(Item { key: format!("dir:{}", d.name), kind: ItemKind::Direction, at: Some(d.at), title: d.name.clone(), lines, pictures: d.pictures.iter().map(|p| p.path.clone()).collect(), link: None, tone: if d.chosen { Tone::Good } else { Tone::Plain } });
        }
        for r in &f.reports {
            let pictures = f.pictures.iter().filter(|p| p.from == PictureFrom::Output).map(|p| p.path.clone()).collect();
            let mut lines: Vec<String> = report_lead(&r.text).into_iter().collect();
            lines.extend(report_items(&r.text));
            out.push(Item { key: format!("report:{}", r.path), kind: ItemKind::Report, at: Some(r.at), title: r.title.clone(), lines, pictures, link: None, tone: Tone::Plain });
        }
        for c in &f.critiques {
            let said = c.reviewer.as_ref().map(checks::reviewed);
            let lines = said.into_iter().chain(c.text.lines().map(str::trim).filter(|l| !l.starts_with('#')).map(|l| l.trim_start_matches(['*', '-', ' ']).trim()).filter(|l| !l.is_empty()).map(str::to_owned)).collect();
            out.push(Item { key: format!("crit:{}", c.path), kind: ItemKind::Critique, at: Some(c.at), title: t!("studio.item_critique").to_owned(), lines, pictures: Vec::new(), link: None, tone: Tone::Plain });
        }
    }
    // Each check and look as it ran.
    let mut seen_checks: Vec<String> = Vec::new();
    let mut seen_looks: Vec<String> = Vec::new();
    let picture_of = |screen: &str| files.and_then(|f| f.checks.iter().find(|c| c.screen == screen)).map(|c| usual_first(&c.pictures)).unwrap_or_default();
    // Only the latest run of a screen goes with its pictures: they are what it drew last.
    let last_run = |screen: &str, look: bool| {
        happenings.iter().rposition(|h| match &h.did {
            Did::Checked(s) if !look => s.screen.as_deref().unwrap_or("") == screen,
            Did::Looked2 { screen: sc, .. } if look => sc == screen,
            _ => false,
        })
    };
    for (n, h) in happenings.iter().enumerate() {
        match &h.did {
            Did::Checked(s) => {
                let screen = s.screen.clone().unwrap_or_default();
                seen_checks.push(screen.clone());
                let title = if screen.is_empty() { t!("check.title").to_owned() } else { screen.clone() };
                let pictures = if last_run(&screen, false) == Some(n) { picture_of(&screen) } else { Vec::new() };
                out.push(Item { key: format!("run:{n}"), kind: ItemKind::Check, at: h.at, title, lines: check_lines(s), pictures, link: None, tone: if s.passes() { Tone::Good } else { Tone::Bad } });
            }
            Did::Looked2 { screen, verdict } => {
                seen_looks.push(screen.clone());
                let fixes: Vec<(String, String, String)> = verdict.fixes.iter().map(|f| (f.place.clone(), f.what.clone(), f.why.clone())).collect();
                let questions: Vec<(String, String)> = verdict.questions.iter().map(|q| (q.what.clone(), q.wording.clone())).collect();
                let title = screen.clone();
                out.push(Item { key: format!("look:{n}"), kind: ItemKind::Look, at: h.at.or(Some(verdict.at)), title, lines: look_lines(verdict.pass, &verdict.summary, &fixes, &questions, verdict.reviewer.as_ref()), pictures: if last_run(screen, true) == Some(n) { picture_of(screen) } else { Vec::new() }, link: None, tone: if verdict.pass { Tone::Good } else { Tone::Bad } });
            }
            Did::HandIn { ok, why } => {
                let title = if *ok { t!("studio.item_handed") } else { t!("studio.item_refused") }.to_owned();
                out.push(Item { key: format!("hand:{n}"), kind: ItemKind::HandIn, at: h.at, title, lines: why.clone(), pictures: Vec::new(), link: None, tone: if *ok { Tone::Good } else { Tone::Bad } });
            }
            _ => {}
        }
    }
    // What the records keep that the conversation's lines don't show.
    if let Some(f) = files {
        for c in &f.checks {
            if !seen_checks.contains(&c.screen) {
                let s = summary_of(c);
                out.push(Item { key: format!("rec:{}", c.screen), kind: ItemKind::Check, at: Some(c.at), title: c.screen.clone(), lines: check_lines(&s), pictures: usual_first(&c.pictures), link: None, tone: if s.passes() { Tone::Good } else { Tone::Bad } });
            }
            if !seen_looks.contains(&c.screen) {
                for l in [&c.first, &c.look].into_iter().flatten() {
                    let (fixes, questions) = studio_look(l);
                    out.push(Item { key: format!("reclook:{}:{}", c.screen, l.at), kind: ItemKind::Look, at: Some(l.at), title: c.screen.clone(), lines: look_lines(l.pass, &l.summary, &fixes, questions, l.reviewer.as_ref()), pictures: usual_first(&c.pictures), link: None, tone: if l.pass { Tone::Good } else { Tone::Bad } });
                }
            }
        }
    }
    out.sort_by_key(|i| std::cmp::Reverse(i.at.unwrap_or(0)));
    out
}

/// Only what this ticket's work made: a shared design folder (the squad's
/// own, which every ticket works in) keeps every ticket's checks, critiques
/// and references — those from before the work began are another's.
pub fn scoped(files: &Studio, since: Option<u64>) -> Studio {
    let mut out = files.clone();
    if let Some(since) = since.map(|s| s.saturating_sub(60)) {
        out.checks.retain(|c| c.at >= since);
        out.critiques.retain(|c| c.at >= since);
        out.references.retain(|r| r.modified >= since);
        out.pictures.retain(|p| p.from != PictureFrom::Design || p.modified >= since);
        out.reports.retain(|r| r.at >= since);
    }
    out
}

/// The pictures of the work, newest first: each check's at the usual width,
/// and every other picture found — at most `most`.
pub fn recent(files: &Studio, most: usize) -> Vec<StudioPicture> {
    let mut out: Vec<StudioPicture> = files
        .checks
        .iter()
        .filter_map(|c| c.pictures.iter().find(|p| p.width == Some(USUAL_WIDTH)).or_else(|| c.pictures.last()))
        .cloned()
        .chain(files.pictures.iter().cloned())
        .collect();
    out.sort_by_key(|p| std::cmp::Reverse(p.modified));
    out.dedup_by(|a, b| a.path == b.path);
    out.truncate(most);
    out
}

/// Where a picture came from, in a word.
pub fn from_words(from: PictureFrom) -> &'static str {
    match from {
        PictureFrom::Design => t!("studio.from_design"),
        PictureFrom::Recording => t!("studio.from_recording"),
        PictureFrom::Output => t!("studio.from_output"),
        PictureFrom::Opened => t!("studio.from_opened"),
        PictureFrom::Made => t!("studio.from_made"),
        PictureFrom::HandIn => t!("studio.from_hand_in"),
    }
}

/// Markdown's emphasis and code marks taken off a line.
fn plain(line: &str) -> String {
    line.replace("**", "").replace('`', "").trim().to_owned()
}

/// A report's first paragraph after its title: where it stands, said up front.
pub fn report_lead(text: &str) -> Option<String> {
    let mut lines = text.lines().map(str::trim).skip_while(|l| l.is_empty() || l.starts_with("# "));
    let lead: Vec<&str> = lines.by_ref().take_while(|l| !l.is_empty() && !l.starts_with('#')).collect();
    (!lead.is_empty() && !lead[0].starts_with(['-', '*', '|'])).then(|| plain(&lead.join(" ")))
}

/// What a report found, a line each: its numbered items — the findings a
/// report counts — else its bullets; never inside code or a table.
pub fn report_items(text: &str) -> Vec<String> {
    let mut numbered = Vec::new();
    let mut bullets = Vec::new();
    let mut code = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with("```") {
            code = !code;
            continue;
        }
        if code || l.starts_with('|') || line.starts_with("    ") || line.starts_with('\t') {
            continue;
        }
        let digits = l.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && l[digits..].starts_with(". ") {
            numbered.push(plain(&l[digits + 2..]));
        } else if let Some(rest) = l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")) {
            bullets.push(plain(rest));
        }
    }
    let mut out = if numbered.is_empty() { bullets } else { numbered };
    out.retain(|l| !l.is_empty());
    out.truncate(12);
    out
}

/// Every finding its reports hold, the newest report's first.
pub fn findings(files: &Studio) -> Vec<String> {
    files.reports.iter().flat_map(|r| report_items(&r.text)).collect()
}

// ── what it is doing now, from its conversation ──

/// What a line of the feed is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedKind {
    /// A step it took.
    Step,
    /// Something it said.
    Said,
    /// Something the person said.
    You,
    /// A line from equipe: stopped, started again.
    Note,
}

/// One line of what it is doing, in plain words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedLine {
    pub at: Option<u64>,
    pub words: String,
    pub kind: FeedKind,
    /// Still running: no answer yet.
    pub running: bool,
    pub tone: Tone,
}

/// Steps that say nothing to the person.
const QUIET_TOOLS: [&str; 6] = ["ToolSearch", "TodoWrite", "ListAgents", "mcp__equipe__estimate", "mcp__equipe__board", "TaskOutput"];

/// A file's name, from a path.
fn file_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

/// A command as the person reads it: past the `cd somewhere;` it starts
/// with, cut short.
pub fn command_words(command: &str) -> String {
    let mut rest = command.lines().next().unwrap_or("").trim();
    while let Some(after) = rest.strip_prefix("cd ") {
        match after.find([';', '&']) {
            Some(at) => rest = after[at..].trim_start_matches([';', '&', ' ']),
            None => break,
        }
    }
    let mut out: String = rest.chars().take(64).collect();
    if rest.chars().count() > 64 {
        out.push('…');
    }
    out
}

/// What a phone step did, in a word.
fn phone_words(action: &str) -> String {
    let key = format!("studio.phone_{action}");
    let word = crate::i18n::catalog().get(&key);
    if word == key { action.to_owned() } else { word.to_owned() }
}

/// A step, in plain words: what the check or the look printed, what a known
/// tool does, else the member's own words for it (its label), else the tool.
pub fn step_words(tool: &str, target: &str, label: Option<&str>, outcome: Option<&equipe_core::protocol::ToolOutcome>, background: bool) -> (String, Tone) {
    if let Some(ran) = outcome.and_then(|o| checks::ran(tool, target, &o.output)) {
        return match ran {
            Ran::Check(s) => (t!("studio.do_check", screen = s.screen.clone().unwrap_or_default(), result = checks::floor(&s)), if s.passes() { Tone::Good } else { Tone::Bad }),
            Ran::Look { screen, verdict } => (t!("studio.do_look", screen = screen, result = checks::verdict(&verdict)), if verdict.pass { Tone::Good } else { Tone::Bad }),
        };
    }
    let ok = outcome.is_none_or(|o| o.ok);
    let plain = Tone::Plain;
    if let Some(name) = skill_of(tool, target) {
        return (t!("studio.do_skill", name = skill_words(&name)), plain);
    }
    let words = match tool {
        "Read" if equipe_core::visuals::is_image_name(target) => t!("studio.do_looked_at", name = file_name(target)),
        "Read" => t!("studio.do_read", name = file_name(target)),
        "Write" => t!("studio.do_wrote", name = file_name(target)),
        "Edit" | "MultiEdit" => t!("studio.do_edited", name = file_name(target)),
        "WebSearch" => t!("studio.do_search", what = target),
        "WebFetch" => t!("studio.do_fetch", what = target),
        "Agent" | "Task" => t!("studio.do_helper", what = label.unwrap_or(target)),
        "Monitor" => t!("studio.do_wait", what = command_words(label.unwrap_or(target))),
        "mcp__equipe__ask_decision" => t!("studio.do_asked", what = target),
        "mcp__equipe__submit_for_review" if ok => t!("studio.do_handed").to_owned(),
        "mcp__equipe__submit_for_review" => t!("studio.do_refused", why = outcome.map(|o| refusal(&o.output).into_iter().next().unwrap_or_default()).unwrap_or_default()),
        "mcp__equipe__design_qa" => t!("studio.do_verdict").to_owned(),
        "Bash" if target.contains("design direction propose") => t!("studio.do_propose", name = direction_name(target, "propose").unwrap_or_default()),
        "Bash" if target.contains("design direction choose") => t!("studio.do_choose", name = direction_name(target, "choose").unwrap_or_default()),
        "Bash" if target.contains("design critique") => t!("studio.do_critique").to_owned(),
        "Bash" if is_shot(target) => t!("studio.do_shot").to_owned(),
        "Bash" if ["git diff", "git show"].iter().any(|g| target.contains(g)) => t!("studio.do_diff").to_owned(),
        _ if tool.starts_with("mcp__phone__") => t!("studio.do_phone", what = [phone_words(&tool["mcp__phone__".len()..]), target.to_owned()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" · ")),
        _ if tool.starts_with("mcp__browser") => t!("studio.do_browse", what = target),
        // The member's own words for the step, when it gave them.
        _ if label.is_some_and(|l| !l.trim().is_empty()) => label.unwrap_or_default().trim().to_owned(),
        "Bash" if background => t!("studio.do_started", command = command_words(target)),
        "Bash" => t!("studio.do_ran", command = command_words(target)),
        _ => t!("studio.do_tool", name = tool.rsplit("__").next().unwrap_or(tool)),
    };
    (words, if ok { plain } else { Tone::Bad })
}

/// What it said, in a line: its first sentence or line, cut short.
fn said_words(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let mut out: String = first.chars().take(160).collect();
    if first.chars().count() > 160 {
        out.push('…');
    }
    out
}

/// The last `most` lines of what it did and said on the ticket, newest
/// last: its steps in plain words, what it said, what the person said —
/// never the engine's own notices (a background task ending).
pub fn feed(work: &[(&Entry, Option<u64>)], most: usize) -> Vec<FeedLine> {
    let mut out: Vec<FeedLine> = Vec::new();
    let last_tool = work.iter().rposition(|(e, _)| matches!(e, Entry::Tool { .. }));
    for (ix, (entry, at)) in work.iter().enumerate() {
        let line = match entry {
            Entry::Tool { tool, target, outcome, label, background } if !QUIET_TOOLS.contains(&tool.as_str()) => {
                let (words, tone) = step_words(tool, target.as_deref().unwrap_or(""), label.as_deref(), outcome.as_deref(), *background);
                let running = outcome.is_none() && Some(ix) == last_tool && !*background;
                FeedLine { at: *at, words, kind: FeedKind::Step, running, tone }
            }
            Entry::Reply { text } if !text.trim().is_empty() => FeedLine { at: *at, words: said_words(text), kind: FeedKind::Said, running: false, tone: Tone::Plain },
            Entry::User { text, .. } if !text.trim_start().starts_with('<') => FeedLine { at: *at, words: t!("studio.do_you", words = said_words(&crate::design_talk::split(text).1)), kind: FeedKind::You, running: false, tone: Tone::Plain },
            Entry::Note { text, .. } if !text.starts_with("── T-") && !text.trim().is_empty() => FeedLine { at: *at, words: equipe_core::i18n::note(crate::i18n::lang(), text).into_owned(), kind: FeedKind::Note, running: false, tone: Tone::Plain },
            _ => continue,
        };
        out.push(line);
    }
    if out.len() > most {
        out.drain(..out.len() - most);
    }
    out
}

/// What it said along the way, newest last: the notes a review writes as it
/// goes, before its report.
pub fn said_so_far(work: &[(&Entry, Option<u64>)], most: usize) -> Vec<(Option<u64>, String)> {
    let mut out: Vec<(Option<u64>, String)> = work.iter().filter_map(|(e, at)| if let Entry::Reply { text } = e { Some((*at, said_words(text))) } else { None }).filter(|(_, w)| !w.is_empty()).collect();
    if out.len() > most {
        out.drain(..out.len() - most);
    }
    out
}

/// When the work on the ticket began, and when it last moved.
pub fn span_of(work: &[(&Entry, Option<u64>)]) -> (Option<u64>, Option<u64>) {
    (work.iter().filter_map(|(_, at)| *at).min(), work.iter().filter_map(|(_, at)| *at).max())
}

/// The skills' names in the person's words, as far as the studio knows them.
pub fn skill_words(name: &str) -> String {
    let key = format!("studio.skill.{name}");
    let word = crate::i18n::catalog().get(&key);
    if word == key { name.to_owned() } else { word.to_owned() }
}

/// The member is browsing now: its latest step was in the shared browser.
pub fn browsing(happenings: &[Happening], working: bool) -> bool {
    working && happenings.last().is_some_and(|h| matches!(h.did, Did::Browsed))
}

/// Whether the studio is offered for a ticket: a designer's, or any member's
/// whose work on it checks screens.
pub fn offered(ticket: &Ticket, role: Option<&str>, happenings: &[Happening]) -> bool {
    !ticket.status.is_closed() && role == Some("designer") || happenings.iter().any(|h| matches!(h.did, Did::Checked(_) | Did::Looked2 { .. }))
        || ticket.owner.as_deref() == Some("designer") || ticket.assignee.as_deref().is_some_and(|a| a.starts_with("designer"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::design::lint::{self, record::Fix};
    use equipe_core::design::second_look;
    use equipe_core::protocol::ToolOutcome;
    use equipe_core::protocol::studio::{StudioDirection, StudioFix, StudioReference};

    fn tool(tool: &str, target: &str, ok: bool, output: &str) -> Entry {
        Entry::Tool { tool: tool.into(), target: Some(target.into()), outcome: Some(Box::new(ToolOutcome { ok, output: output.into(), ..ToolOutcome::default() })), label: None, background: false }
    }

    fn summary(pass: bool) -> Summary {
        Summary {
            floor: if pass { (11, 11) } else { (10, 11) },
            widths: WIDTHS.to_vec(),
            failing: if pass { vec![] } else { vec![("contrast".into(), "the CTA is 2.47:1".into())] },
            screen: Some("onboarding/welcome".into()),
            ..Summary::default()
        }
    }

    fn lint_run(pass: bool) -> Entry {
        tool("Bash", "equipe design lint design/mockups/welcome.html --render --screen onboarding/welcome", pass, &format!("Drawing…\n{}", lint::mark_line(&summary(pass))))
    }

    fn look_run(pass: bool) -> Entry {
        let v = Verdict { pass, fixes: if pass { vec![] } else { vec![Fix { place: "the CTA at 360".into(), what: "give it room".into(), why: String::new() }] }, ..Verdict::default() };
        tool("Bash", "equipe design second-look onboarding/welcome", pass, &second_look::mark_line("onboarding/welcome", &v))
    }

    fn ticket(status: TicketStatus) -> Ticket {
        let mut t: Ticket = serde_json::from_value(serde_json::json!({ "id": 205, "title": "Onboarding", "body": "", "status": "in_progress", "kind": "task", "depends_on": [], "assignee": "designer", "created_by": "pm" })).unwrap();
        t.status = status;
        t
    }

    /// A Claude designer's work, as its conversation keeps it.
    fn claude_work() -> Vec<Entry> {
        vec![
            Entry::Note { undo: None, text: "── T-205 · Onboarding — on branch equipe/designer-t205".into() },
            tool("Skill", "equipe:design-process", true, ""),
            tool("Read", "/w/design/brand.md", true, ""),
            tool("WebSearch", "calm onboarding", true, ""),
            tool("Write", "/w/design/references/linear/notes.md", true, ""),
            tool("Bash", "equipe design direction propose \"Calm ledger\" --layout one --ticket T-205", true, ""),
            tool("Bash", "equipe design direction propose Poster --layout two --ticket T-205", true, ""),
            tool("mcp__equipe__ask_decision", "", true, ""),
            tool("Bash", "equipe design direction choose \"Calm ledger\" --ticket T-205", true, ""),
            tool("Write", "/w/design/mockups/welcome.html", true, ""),
            lint_run(false),
            tool("Edit", "/w/design/mockups/welcome.css", true, ""),
            lint_run(true),
            tool("mcp__equipe__submit_for_review", "T-205", false, "Not yet — a screen you made goes to the person only checked and seen fresh:\n- onboarding/welcome has had no second look: `equipe design second-look onboarding/welcome`.\nThen hand in again."),
            look_run(false),
            tool("Edit", "/w/design/mockups/welcome.html", true, ""),
            lint_run(true),
            look_run(true),
        ]
    }

    /// The same work by a Codex designer: skills opened by reading their
    /// files, commands as its shell ran them.
    fn codex_work() -> Vec<Entry> {
        let mut out = claude_work();
        out[1] = tool("Read", "/Users/x/.equipe/skills/design-process/SKILL.md", true, "");
        out[3] = tool("Bash", "equipe shoot https://linear.app --out design/references/linear/shot.png", true, "");
        out
    }

    fn watch<'a>(ticket: &'a Ticket, entries: &'a [Entry], files: Option<&'a Studio>, working: bool, waiting: Option<WaitFor>) -> Watch<'a> {
        let t = Transcript::from_entries(entries.to_vec());
        let happenings = happenings(&work_of(&t, ticket.id));
        Watch { ticket, happenings, files, working, waiting, flow: Flow::Design }
    }

    #[test]
    fn only_the_tickets_own_work_is_read() {
        let mut entries = vec![Entry::Note { undo: None, text: "── T-9 · Other — on branch b".into() }, lint_run(false)];
        entries.extend(claude_work());
        entries.push(Entry::Note { undo: None, text: "── T-2050 · Not it — on branch c".into() });
        entries.push(lint_run(false));
        let t = Transcript::from_entries(entries);
        let work = work_of(&t, 205);
        assert_eq!(work.len(), claude_work().len(), "from its header to the next, and T-2050 is not T-205");
    }

    #[test]
    fn each_step_of_a_claude_designer_and_a_codex_one_is_read_the_same() {
        for (engine, entries) in [("claude", claude_work()), ("codex", codex_work())] {
            let t = Transcript::from_entries(entries);
            let did: Vec<&'static str> = happenings(&work_of(&t, 205))
                .iter()
                .map(|h| match &h.did {
                    Did::Skill(_) => "skill",
                    Did::Brief => "brief",
                    Did::Looked => "looked",
                    Did::Proposed(_) => "proposed",
                    Did::Chose(_) => "chose",
                    Did::Built(_) => "built",
                    Did::Checked(s) if s.passes() => "pass",
                    Did::Checked(_) => "fail",
                    Did::Looked2 { verdict, .. } if verdict.pass => "look-pass",
                    Did::Looked2 { .. } => "look-fix",
                    Did::Critiqued => "critique",
                    Did::Browsed => "browsed",
                    Did::HandIn { ok: true, .. } => "handed",
                    Did::HandIn { .. } => "refused",
                    Did::Opened(_) => "opened",
                    Did::Shot => "shot",
                    Did::Phone(_) => "phone",
                    Did::Read(_) => "read",
                    Did::Diff => "diff",
                    Did::Helper => "helper",
                    Did::Report(_) => "report",
                    Did::Verdict => "verdict",
                    Did::Asked(_) => "asked",
                })
                .collect();
            assert_eq!(did, ["skill", "brief", "looked", "looked", "proposed", "proposed", "asked", "chose", "built", "fail", "built", "pass", "refused", "look-fix", "built", "pass", "look-pass"], "{engine}");
        }
        assert_eq!(skill_of("Skill", "equipe:design-process").as_deref(), Some("design-process"));
        assert_eq!(skill_of("Bash", "cat /h/skills/ui-floor/SKILL.md | head").as_deref(), Some("ui-floor"));
        assert_eq!(skill_of("Read", "/h/skills-old/x.md"), None);
        assert_eq!(direction_name("equipe design direction propose 'Bold one' --ticket T-1", "propose").as_deref(), Some("Bold one"));
        assert_eq!(direction_name("equipe design direction propose --help", "propose"), None);
        assert!(is_ui_file("/w/src/App.tsx") && !is_ui_file("/w/design/checks/a/360.png") && !is_ui_file("/w/README.md"));
    }

    #[test]
    fn the_rail_lights_each_step_from_what_happened() {
        let t = ticket(TicketStatus::InProgress);
        let entries = claude_work();
        let w = watch(&t, &entries, None, true, None);
        let rail = steps(&w);
        assert_eq!(rail.iter().map(|s| s.mark).collect::<Vec<_>>(), [Mark::Done, Mark::Done, Mark::Done, Mark::Done, Mark::Now, Mark::Ahead], "refused, it went back to its checks, which pass now");
        assert_eq!(rail[0].detail.as_deref(), Some("디자인 작업 순서"));
        assert_eq!(rail[2].count.as_deref(), Some("방향 2개 · 고른 것: Calm ledger"));
        assert_eq!(rail[4].count.as_deref(), Some("기준선 11/11 · 폭 7/7 · 통과"));
        assert_eq!(rail[5].count.as_deref(), Some("거절 1회"));
        // Refused and nothing since: the hand-in needs a fix.
        let refused: Vec<Entry> = claude_work().into_iter().take(14).collect();
        let w = watch(&t, &refused, None, true, None);
        assert_eq!(steps(&w)[5].mark, Mark::Attention);
        // Only the brief so far: the brief is where it is; the rest ahead.
        let early: Vec<Entry> = claude_work().into_iter().take(3).collect();
        let w = watch(&t, &early, None, true, None);
        assert_eq!(steps(&w).iter().map(|s| s.mark).collect::<Vec<_>>(), [Mark::Now, Mark::Ahead, Mark::Ahead, Mark::Ahead, Mark::Ahead, Mark::Ahead]);
        // Straight to building, no directions: those are skipped, not missing.
        let quick = vec![claude_work()[0].clone(), tool("Write", "/w/src/a.css", true, "")];
        let w = watch(&t, &quick, None, true, None);
        assert_eq!(steps(&w).iter().map(|s| s.mark).collect::<Vec<_>>(), [Mark::Skipped, Mark::Skipped, Mark::Skipped, Mark::Now, Mark::Ahead, Mark::Ahead]);
        // Handed in: every step reached is done.
        let mut handed = claude_work();
        handed.push(tool("mcp__equipe__submit_for_review", "T-205", true, ""));
        let review = ticket(TicketStatus::Review);
        let w = watch(&review, &handed, None, false, None);
        assert!(steps(&w).iter().all(|s| s.mark == Mark::Done));
        assert_eq!(steps(&w)[5].count.as_deref(), Some("넘김"));
    }

    #[test]
    fn the_status_says_where_it_is_and_what_holds_it_up() {
        let t = ticket(TicketStatus::InProgress);
        let all = claude_work();
        let upto = |n: usize| all[..n].to_vec();
        // The first check fails: working, and what failed, in words, and what it saw.
        let failing = upto(11);
        let w = watch(&t, &failing, None, true, None);
        let s = status(&w, Some(9_280), Some(9_900));
        assert_eq!(s.presence, Presence::Working { since: Some(9_280) });
        assert_eq!(s.issue, Some(Issue::Failing { rule: "대비".into(), evidence: "the CTA is 2.47:1".into() }));
        assert_eq!(status_words(&s, 10_000), "작업 중 · 12분 · 검사 실패: 대비 — the CTA is 2.47:1");
        // Refused at the hand-in: why, until it checks again.
        let refused = upto(14);
        let w = watch(&t, &refused, None, true, None);
        assert_eq!(status(&w, None, None).issue, Some(Issue::Refused("onboarding/welcome has had no second look: `equipe design second-look onboarding/welcome`.".into())));
        // The second look asked for a fix.
        let fixing = upto(15);
        assert_eq!(status(&watch(&t, &fixing, None, true, None), None, None).issue, Some(Issue::Fixing(1)));
        // Not at it: idle since its last step, never "working".
        let s = status(&watch(&t, &all, None, false, None), Some(1_000), Some(9_820));
        assert_eq!(s.presence, Presence::Idle { last: Some(9_820) });
        assert_eq!(presence_words(&s.presence, 10_000), "쉬는 중 · 마지막 단계 3분 전");
        // Waiting on the person beats working, as the sidebar says it.
        let w = watch(&t, &all[..7], None, true, Some(WaitFor::Pick));
        assert_eq!(status(&w, None, None).presence, Presence::Waiting(WaitFor::Pick));
        assert_eq!(steps(&w)[2].mark, Mark::Waiting);
        // Handed in: no issue left to say.
        let review = ticket(TicketStatus::Review);
        let s = status(&watch(&review, &failing, None, false, None), None, None);
        assert!(matches!(s.presence, Presence::HandedIn { .. }) && s.issue.is_none());
    }

    #[test]
    fn it_waits_for_the_person_exactly_when_the_sidebar_says_so() {
        let t = ticket(TicketStatus::InProgress);
        let proposed = vec![Happening { at: None, did: Did::Proposed("A".into()) }];
        let wf = |permission: Option<&str>, question: Option<(&str, Option<TicketId>)>, waiting: Option<TicketId>, hs: &[Happening]| waiting_for(205, &t, permission.map(str::to_owned), question.map(|(q, on)| (q.to_owned(), on)), waiting, None, hs);
        // The order Model::status weighs it: an OK asked, a ticket waiting, a question.
        assert_eq!(wf(Some("Bash · xcodebuild test"), Some(("Which one?", Some(205))), None, &[]), Some(WaitFor::Permission("Bash · xcodebuild test".into())));
        assert_eq!(wf(None, Some(("Which one?", Some(205))), None, &proposed), Some(WaitFor::Pick));
        assert_eq!(wf(None, Some(("Which one?", Some(205))), None, &[]), Some(WaitFor::Answer { what: "Which one?".into(), ticket: None }));
        // On another ticket: said which.
        assert_eq!(wf(None, Some(("The price?", Some(499))), None, &[]), Some(WaitFor::Answer { what: "The price?".into(), ticket: Some(499) }));
        assert_eq!(wf(None, None, Some(499), &[]), Some(WaitFor::Ticket(499)));
        assert!(!WaitFor::Ticket(499).is_here() && WaitFor::Pick.is_here(), "a wait on another ticket doesn't hold this one's steps");
        assert_eq!(wf(None, None, None, &proposed), None);
        assert_eq!(waiting_for(205, &ticket(TicketStatus::Waiting), None, None, None, None, &proposed), Some(WaitFor::Pick));
        assert_eq!(presence_words(&Presence::Waiting(WaitFor::Answer { what: "The price?".into(), ticket: Some(499) }), 0), "T-499에서 기다리는 중: The price?");
        assert_eq!(presence_words(&Presence::Waiting(WaitFor::Permission("Bash · xcodebuild test".into())), 0), "허락을 기다리는 중: Bash · xcodebuild test");
    }

    /// A design QA ticket's conversation, in the shape the journal keeps it
    /// (a review of another ticket's screens on a phone simulator), paths made up.
    fn qa_work() -> Vec<(Entry, Option<u64>)> {
        let line = |json: &str| serde_json::from_str::<Entry>(json).unwrap();
        [
            r#"{"kind":"note","text":"── T-502 · Design QA: T-498 the composer's glyphs at AX5 — on branch equipe/designer-t502-1"}"#,
            r#"{"kind":"tool","tool":"Skill","target":"equipe:design-qa"}"#,
            r#"{"kind":"tool","tool":"Bash","target":"cd /w/t498; git diff main...HEAD --stat"}"#,
            r#"{"kind":"tool","tool":"mcp__equipe__estimate","target":null}"#,
            r#"{"kind":"tool","tool":"Bash","target":"cd /w/t498/ios/Features; sed -n 780,880p ChatScreen.swift"}"#,
            r#"{"kind":"reply","text":"Building a simulator copy while I read the Android composer."}"#,
            r#"{"kind":"tool","tool":"Bash","target":"/tmp/t502/run.sh > /tmp/t502/run.log 2>&1","background":true}"#,
            r#"{"kind":"tool","tool":"Read","target":"/w/t498/design/flows/chat/large.dark.360.png"}"#,
            r#"{"kind":"user","text":"<task-notification>\n<summary>Monitor event</summary>\n</task-notification>"}"#,
            r#"{"kind":"tool","tool":"Bash","target":"xcrun simctl io booted screenshot /tmp/t502/ax5.png"}"#,
            r#"{"kind":"tool","tool":"Read","target":"/tmp/t502/ios-ax5-17e.png"}"#,
            r#"{"kind":"reply","text":"At AX5 the dictation cancel glyph spills out of its circle — the fix missed that button."}"#,
            r#"{"kind":"tool","tool":"Agent","target":"Fresh-eyes look at composer glyphs"}"#,
            r#"{"kind":"note","text":"Stopped."}"#,
            r#"{"kind":"tool","tool":"Write","target":"/h/outputs/T-502 Design QA/report.md"}"#,
            r#"{"kind":"tool","tool":"Bash","target":"tail -c 300 /tmp/t502/build-touch.log","label":"Peek build progress"}"#,
        ]
        .iter()
        .enumerate()
        .map(|(n, j)| (line(j), Some(1_000 + 60 * n as u64)))
        .collect()
    }

    #[test]
    fn a_design_qa_ticket_shows_its_own_steps_from_its_real_shape() {
        let mut t = ticket(TicketStatus::InProgress);
        t.id = 502;
        let lines = qa_work();
        let work: Vec<(&Entry, Option<u64>)> = lines.iter().map(|(e, at)| (e, *at)).collect();
        let hs = happenings(&work);
        assert_eq!(flow_of(&t, false, &hs), Flow::Qa, "the design QA guide opened");
        assert_eq!(flow_of(&t, true, &[]), Flow::Qa, "another ticket waits on it for design QA");
        assert_eq!(flow_of(&t, false, &[]), Flow::Design);
        let report = "# T-502 Design QA — the composer\n\nBranch `b` (80a7af9). Verdict: **fix and again**.\n\n## Shot\n- iOS on two simulators\n\n## Blocking\n1. **The dictation cancel X still spills at AX5.** `DictationViews.swift` misses `.composerGlyph()`.\n2. The + touch area reads 36pt.\n\n| glyph | main |\n|---|---|\n| + | 43pt |\n";
        let files = Studio {
            ticket: 502,
            pictures: vec![StudioPicture { path: "/tmp/t502/ios-ax5-17e.png".into(), modified: 1_600, width: None, from: PictureFrom::Opened }, StudioPicture { path: "/h/outputs/T-502 Design QA/1-ax5.png".into(), modified: 1_900, width: None, from: PictureFrom::Output }],
            reports: vec![equipe_core::protocol::studio::StudioReport { path: "/h/outputs/T-502 Design QA/report.md".into(), at: 1_850, title: "T-502 Design QA — the composer".into(), text: report.into() }],
            ..Studio::default()
        };
        let w = Watch { ticket: &t, happenings: hs, files: Some(&files), working: true, waiting: None, flow: Flow::Qa };
        let rail = steps(&w);
        assert_eq!(rail.iter().map(|s| s.kind).collect::<Vec<_>>(), [StepKind::Brief, StepKind::Change, StepKind::Shots, StepKind::Measure, StepKind::Findings, StepKind::Verdict]);
        assert_eq!(rail.iter().map(|s| s.mark).collect::<Vec<_>>(), [Mark::Done, Mark::Done, Mark::Done, Mark::Done, Mark::Now, Mark::Ahead], "nothing 'not needed' on a review");
        assert_eq!(rail[4].count.as_deref(), Some("찾은 것 2개"));
        assert_eq!(findings(&files), ["The dictation cancel X still spills at AX5. DictationViews.swift misses .composerGlyph().", "The + touch area reads 36pt."], "numbered items, marks taken off, never the table");
        assert_eq!(report_lead(report).as_deref(), Some("Branch b (80a7af9). Verdict: fix and again."));
        // What it is doing, in plain words, the engine's notices and quiet steps left out.
        let f = feed(&work, 10);
        let words: Vec<&str> = f.iter().map(|l| l.words.as_str()).collect();
        assert_eq!(words.last(), Some(&"Peek build progress"), "its own words for a step");
        assert!(words.contains(&"스크린샷을 찍음"));
        assert!(words.contains(&"ios-ax5-17e.png 봄"));
        assert!(words.contains(&"도우미에게 맡김: Fresh-eyes look at composer glyphs"));
        assert!(words.contains(&"At AX5 the dictation cancel glyph spills out of its circle — the fix missed that button."));
        assert!(!words.iter().any(|w| w.contains("estimate") || w.contains("task-notification")));
        assert_eq!(f.len(), 10);
        let all = feed(&work, 99);
        assert!(all.iter().any(|l| l.words == "실행: sed -n 780,880p ChatScreen.swift"), "past the cd it starts with: {:?}", all.iter().map(|l| &l.words).collect::<Vec<_>>());
        assert!(all.iter().any(|l| l.words == "백그라운드에서 시작: /tmp/t502/run.sh > /tmp/t502/run.log 2>&1"));
        assert!(all.iter().any(|l| l.words == "안내 열어 봄: 디자인 QA"));
        assert!(all.iter().any(|l| l.kind == FeedKind::Note), "a stop shows");
        assert_eq!(span_of(&work), (Some(1_000), Some(1_900)));
        assert_eq!(said_so_far(&work, 1).len(), 1);
    }

    #[test]
    fn the_latest_step_still_running_says_so() {
        let lines = [Entry::Note { undo: None, text: "── T-1 · A — on branch b".into() }, Entry::Tool { tool: "mcp__phone__look".into(), target: Some("Composer".into()), outcome: None, label: None, background: false }];
        let work: Vec<(&Entry, Option<u64>)> = lines.iter().map(|e| (e, Some(5))).collect();
        let f = feed(&work, 10);
        assert_eq!(f.len(), 1);
        assert!(f[0].running);
        assert_eq!(f[0].words, "폰: 화면을 봄 · Composer");
    }

    #[test]
    fn a_shared_design_folder_shows_only_what_this_work_made() {
        let mut f = files();
        f.checks[0].at = 50;
        let s = scoped(&f, Some(400));
        assert!(s.checks.is_empty(), "a check from before the work began is another ticket's");
        assert!(s.references.is_empty());
        assert_eq!(scoped(&f, None).checks.len(), 1, "no start known: all of it");
    }

    fn pic(path: &str, width: Option<u32>, modified: u64) -> StudioPicture {
        StudioPicture { path: path.into(), modified, width, from: PictureFrom::Design }
    }

    fn files() -> Studio {
        Studio {
            ticket: 205,
            dir: Some("/w/design".into()),
            checks: vec![StudioCheck {
                screen: "onboarding/welcome".into(),
                at: 500,
                rendered: true,
                floor: (11, 11),
                pictures: WIDTHS.iter().map(|w| pic(&format!("/w/design/checks/onboarding/welcome/{w}.png"), Some(*w), 500)).collect(),
                first: Some(StudioLook { pass: true, at: 300, ..StudioLook::default() }),
                look: Some(StudioLook { pass: false, at: 520, fixes: vec![StudioFix { place: "the CTA".into(), what: "darker".into(), why: String::new() }], ..StudioLook::default() }),
                ..StudioCheck::default()
            }],
            directions: vec![
                StudioDirection { name: "Poster".into(), at: 100, pictures: vec![pic("/w/design/directions/poster/home.png", None, 100)], ..StudioDirection::default() },
                StudioDirection { name: "Calm ledger".into(), at: 110, chosen: true, layout: "one column".into(), pictures: vec![pic("/w/design/directions/calm-ledger/home.png", None, 110)], ..StudioDirection::default() },
            ],
            references: vec![StudioReference { name: "linear".into(), link: Some("https://linear.app".into()), notes: "Take: the quiet density.".into(), modified: 50, pictures: vec![pic("/w/design/references/linear/shot.png", None, 50)] }],
            pictures: vec![pic("/w/design/mockups/welcome-v1.png", None, 200)],
            ..Studio::default()
        }
    }

    #[test]
    fn the_centre_shows_the_newest_picture_at_the_usual_width_or_the_one_picked() {
        let f = files();
        let c = centre_of(&f, None, None).unwrap();
        assert_eq!((c.screen.as_deref(), c.width), (Some("onboarding/welcome"), Some(1280)));
        assert_eq!(c.widths.len(), 7);
        assert_eq!(centre_of(&f, Some(360), None).unwrap().shown, "/w/design/checks/onboarding/welcome/360.png");
        // A mockup made after the check: that is what it is making now.
        let mut newer = f.clone();
        newer.pictures[0].modified = 900;
        assert_eq!(centre_of(&newer, None, None).unwrap().shown, "/w/design/mockups/welcome-v1.png");
        assert_eq!(centre_of(&newer, Some(390), None).unwrap().width, Some(390), "a width picked keeps the check's");
        // Nothing checked or made yet: the direction picked.
        let only = Studio { checks: vec![], pictures: vec![], ..f.clone() };
        let c = centre_of(&only, None, None).unwrap();
        assert_eq!((c.shown.as_str(), c.direction.as_deref()), ("/w/design/directions/calm-ledger/home.png", Some("Calm ledger")));
        assert!(c.choices.is_empty());
        // None picked yet: both, side by side, to pick from.
        let mut open = only.clone();
        open.directions[1].chosen = false;
        assert_eq!(centre_of(&open, None, None).unwrap().choices.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Poster", "Calm ledger"]);
        assert_eq!(centre_of(&Studio::default(), None, None), None);
    }

    #[test]
    fn the_timeline_holds_each_thing_it_left_newest_first() {
        let f = files();
        let t = Transcript::from_entries(claude_work());
        let mut hs = happenings(&work_of(&t, 205));
        for (n, h) in hs.iter_mut().enumerate() {
            h.at = Some(1_000 + n as u64);
        }
        let list = items(&hs, Some(&f));
        let kinds: Vec<ItemKind> = list.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, [ItemKind::Look, ItemKind::Check, ItemKind::Look, ItemKind::HandIn, ItemKind::Check, ItemKind::Check, ItemKind::Direction, ItemKind::Direction, ItemKind::Reference]);
        let failing = &list[5];
        assert_eq!(failing.tone, Tone::Bad);
        assert_eq!(failing.lines[..2], ["기준선 10/11".to_owned(), "실패 · 대비 — the CTA is 2.47:1".to_owned()]);
        assert!(failing.pictures.is_empty(), "an earlier run doesn't wear the pictures drawn since");
        let latest = &list[1];
        assert_eq!(latest.pictures.len(), 7, "the latest run goes with the screen's pictures");
        assert!(latest.pictures[0].ends_with("/1280.png"), "its card shows the usual width");
        let refused = &list[3];
        assert_eq!(refused.lines, ["onboarding/welcome has had no second look: `equipe design second-look onboarding/welcome`."]);
        let picked = list.iter().find(|i| i.title == "Calm ledger").unwrap();
        assert_eq!(picked.tone, Tone::Good);
        assert!(picked.lines.iter().any(|l| l.contains("one column")));
        let reference = list.last().unwrap();
        assert_eq!(reference.link.as_deref(), Some("https://linear.app"));
        assert!(reference.lines[0].contains("quiet density"));
        // The conversation's lines gone (a long day): the records still tell.
        let kept = items(&[], Some(&f));
        assert_eq!(kept.iter().filter(|i| i.kind == ItemKind::Look).count(), 2, "the first screenful's and the whole's");
        assert_eq!(kept.iter().filter(|i| i.kind == ItemKind::Check).count(), 1);
        let keys: std::collections::HashSet<&str> = list.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys.len(), list.len(), "each its own key");
    }

    #[test]
    fn the_timeline_says_which_engine_reviewed_and_why_that_one() {
        use equipe_core::protocol::Engine;
        use equipe_core::protocol::engines::{ReviewWhy, Window};
        let room = Reviewed { engine: Engine::Codex, member: Some(Engine::Claude), why: ReviewWhy::Room { used: Some(40) } };
        let near = Reviewed { engine: Engine::Claude, member: Some(Engine::Claude), why: ReviewWhy::NearLimit { window: Window::Weekly, used: 93 } };
        let mut f = files();
        f.critiques = vec![
            equipe_core::protocol::studio::StudioCritique { path: "/w/design/critiques/600.md".into(), at: 600, text: "## Whole\nCramped at 360.".into(), reviewer: Some(near) },
            equipe_core::protocol::studio::StudioCritique { path: "/w/design/critiques/90.md".into(), at: 90, text: "Flat.".into(), reviewer: None },
        ];
        f.checks[0].look.as_mut().unwrap().reviewer = Some(room);
        let list = items(&[], Some(&f));
        let critique = list.iter().find(|i| i.key.ends_with("600.md")).unwrap();
        assert_eq!(critique.lines, ["Claude 새 세션 검토 · Codex 주간 93% 사용", "Cramped at 360."], "who, first: what the timeline shows");
        assert_eq!(list.iter().find(|i| i.key.ends_with("90.md")).unwrap().lines, ["Flat."], "one kept before it: as it was");
        let look = list.iter().find(|i| i.kind == ItemKind::Look && i.at == Some(520)).unwrap();
        assert_eq!(look.lines[..2], [t!("look.fixes", count = 1), "Codex 검토 · Codex 여유 있음 — 주간 40% 사용".to_owned()]);
    }

    #[test]
    fn a_picture_drawn_again_under_its_name_is_shown_again() {
        let mut ui = StudioUi::default();
        ui.watch(205);
        assert!(ui.arrived(files()).is_empty(), "the first answer: nothing to show again");
        let mut again = files();
        again.checks[0].pictures[0].modified = 900;
        assert_eq!(ui.arrived(again), ["/w/design/checks/onboarding/welcome/360.png"]);
        ui.watch(206);
        assert!(ui.files().is_none(), "another ticket: nothing of the last");
    }

    #[test]
    fn its_words_name_each_skill_and_say_when_it_waits() {
        assert_eq!(skill_words("design-process"), "디자인 작업 순서");
        assert_eq!(skill_words("not-a-skill"), "not-a-skill");
        assert_eq!(presence_words(&Presence::Waiting(WaitFor::Pick), 0), "기다리는 중: 방향을 골라 주세요");
        assert_eq!(status_tone(&Presence::Waiting(WaitFor::Pick)), Tone::Wait);
        assert_eq!(issue_words(&Issue::Fixing(2)), "새 시선이 고칠 점 2개를 요청했어요");
        assert!(browsing(&[Happening { at: None, did: Did::Browsed }], true));
        assert!(!browsing(&[Happening { at: None, did: Did::Browsed }], false));
    }
}
