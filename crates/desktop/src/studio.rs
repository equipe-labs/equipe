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
use equipe_core::protocol::studio::{Studio, StudioCheck, StudioLook, StudioPicture};
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
    /// Sent pictures to the other engine for a critique.
    Critiqued,
    /// Worked in the shared browser.
    Browsed,
    /// Handed in: taken, or refused with why.
    HandIn { ok: bool, why: Vec<String> },
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

/// The process's steps, in order, in the person's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepKind {
    Brief,
    References,
    Directions,
    Build,
    Checks,
    HandIn,
}

impl StepKind {
    pub const ALL: [StepKind; 6] = [StepKind::Brief, StepKind::References, StepKind::Directions, StepKind::Build, StepKind::Checks, StepKind::HandIn];

    pub fn label(self) -> &'static str {
        match self {
            StepKind::Brief => t!("studio.step_brief"),
            StepKind::References => t!("studio.step_references"),
            StepKind::Directions => t!("studio.step_directions"),
            StepKind::Build => t!("studio.step_build"),
            StepKind::Checks => t!("studio.step_checks"),
            StepKind::HandIn => t!("studio.step_handin"),
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
        }
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
    /// A question of the member's, in its words.
    Answer(String),
    /// The ticket waits on the person.
    Something,
}

/// Everything the studio is drawn from, gathered once a frame.
pub struct Watch<'a> {
    pub ticket: &'a Ticket,
    pub happenings: Vec<Happening>,
    pub files: Option<&'a Studio>,
    /// The member is at it right now.
    pub working: bool,
    pub waiting: Option<WaitFor>,
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

    let reached = [
        briefs > 0,
        board > 0 || lookups > 0,
        proposed > 0,
        !built.is_empty() || files.is_some_and(|f| !f.pending.is_empty() || !f.pictures.is_empty()),
        checked,
        handed || refusal_stands(w).is_some(),
    ];
    // Where it is now: the furthest step reached.
    let now = reached.iter().rposition(|r| *r);
    let failing = check.as_ref().is_some_and(|(_, s)| !s.passes()) || look.is_some_and(|(_, pass, _)| !pass) || refusal_stands(w).is_some();
    let mut out = Vec::new();
    for (ix, kind) in StepKind::ALL.into_iter().enumerate() {
        let mark = match now {
            _ if handed => if reached[ix] { Mark::Done } else { Mark::Skipped },
            Some(n) if ix < n => if reached[ix] { Mark::Done } else { Mark::Skipped },
            Some(n) if ix == n => match () {
                _ if w.waiting.is_some() => Mark::Waiting,
                _ if failing && ix >= 4 => Mark::Attention,
                _ => Mark::Now,
            },
            // Waiting on a pick before anything is built: the directions step waits.
            None if ix == 0 && w.working => Mark::Now,
            _ => Mark::Ahead,
        };
        let (count, at) = match kind {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    HandedIn { at: Option<u64> },
    Waiting(WaitFor),
    /// The last hand-in was refused, and why (its first line).
    Refused(String),
    /// The latest check fails the floor: the rule, in words, and what it saw.
    Failing { rule: String, evidence: String },
    /// The second look asked for this many fixes.
    Fixing(usize),
    Working { since: Option<u64> },
    Resting { last: Option<u64> },
    NotStarted,
}

/// How it is going, honestly: the most pressing thing first.
pub fn status(w: &Watch) -> Status {
    if handed_in(w) {
        return Status::HandedIn { at: last_hand_in(w).and_then(|h| h.0) };
    }
    if let Some(wait) = &w.waiting {
        return Status::Waiting(wait.clone());
    }
    // A refusal stands until the member has checked or looked again since.
    if let Some(why) = refusal_stands(w) {
        return Status::Refused(why.first().cloned().unwrap_or_default());
    }
    if let Some((_, s)) = latest_check(w).filter(|(_, s)| !s.passes())
        && let Some((id, evidence)) = s.failing.first()
    {
        return Status::Failing { rule: checks::rule_name(id), evidence: evidence.clone() };
    }
    if let Some((_, false, fixes)) = latest_look(w) {
        return Status::Fixing(fixes);
    }
    if w.working {
        return Status::Working { since: w.happenings.iter().find_map(|h| h.at) };
    }
    match w.happenings.iter().rev().find_map(|h| h.at) {
        Some(at) => Status::Resting { last: Some(at) },
        None if w.files.is_some_and(|f| f.dir.is_some()) => Status::Resting { last: None },
        None => Status::NotStarted,
    }
}

/// The status line's words.
pub fn status_words(s: &Status, now: u64) -> String {
    let ago = |at: Option<u64>| at.map(|a| crate::activity::ago(now.saturating_sub(a)));
    match s {
        Status::HandedIn { at } => match ago(*at) {
            Some(when) => t!("studio.status_handed_at", when = when),
            None => t!("studio.status_handed").to_owned(),
        },
        Status::Waiting(WaitFor::Pick) => t!("studio.status_waiting_pick").to_owned(),
        Status::Waiting(WaitFor::Answer(q)) => t!("studio.status_waiting_answer", what = q),
        Status::Waiting(WaitFor::Something) => t!("studio.status_waiting").to_owned(),
        Status::Refused(why) => t!("studio.status_refused", why = why),
        Status::Failing { rule, evidence } => t!("studio.status_failing", rule = rule, evidence = evidence),
        Status::Fixing(n) => t!("studio.status_fixing", count = n),
        Status::Working { since: Some(at) } => t!("studio.status_working", span = crate::state::short_span(now.saturating_sub(*at))),
        Status::Working { since: None } => t!("studio.status_working_now").to_owned(),
        Status::Resting { last: Some(at) } => t!("studio.status_resting", when = crate::activity::ago(now.saturating_sub(*at))),
        Status::Resting { last: None } => t!("studio.status_resting_now").to_owned(),
        Status::NotStarted => t!("studio.status_not_started").to_owned(),
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

pub fn status_tone(s: &Status) -> Tone {
    match s {
        Status::HandedIn { .. } => Tone::Good,
        Status::Waiting(_) => Tone::Wait,
        Status::Refused(_) | Status::Failing { .. } | Status::Fixing(_) => Tone::Bad,
        Status::Working { .. } => Tone::Busy,
        Status::Resting { .. } | Status::NotStarted => Tone::Plain,
    }
}

/// What the person is waited on for: a question of the member's on this
/// ticket — a pick, when directions are out and none was picked — or the
/// ticket itself waiting.
pub fn waiting_for(ticket: &Ticket, question: Option<&str>, files: Option<&Studio>, happenings: &[Happening]) -> Option<WaitFor> {
    let picked = files.is_some_and(|f| f.directions.iter().any(|d| d.chosen)) || happenings.iter().any(|h| matches!(h.did, Did::Chose(_)));
    let proposed = files.is_some_and(|f| !f.directions.is_empty()) || happenings.iter().any(|h| matches!(h.did, Did::Proposed(_)));
    match question {
        Some(_) if proposed && !picked => Some(WaitFor::Pick),
        Some(q) => Some(WaitFor::Answer(q.to_owned())),
        None if ticket.status == TicketStatus::Waiting => Some(if proposed && !picked { WaitFor::Pick } else { WaitFor::Something }),
        None => None,
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
}

/// The width shown when none is picked: the desktop one most look at.
pub const USUAL_WIDTH: u32 = 1280;

/// What the centre shows: the newest of a check's pictures and any other
/// picture the member made — the check's at the width picked (else the usual
/// one, else the widest).
pub fn centre(files: &Studio, picked: Option<u32>) -> Option<Centre> {
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
        Centre { screen: Some(c.screen.clone()), widths, shown, width, at, direction: None, choices: Vec::new() }
    });
    let from_other = other.map(|p| Centre { screen: None, widths: Vec::new(), shown: p.path.clone(), width: None, at: p.modified, direction: None, choices: Vec::new() });
    // A width picked keeps the check's pictures in the centre.
    match (from_check, from_other) {
        (Some(c), Some(o)) if picked.is_none() && o.at > c.at => Some(o),
        (Some(c), _) => Some(c),
        (None, o) => o.or_else(|| {
            let drawn: Vec<_> = files.directions.iter().filter(|d| !d.pictures.is_empty()).collect();
            let d = drawn.iter().rev().find(|d| d.chosen).or_else(|| drawn.last())?;
            let picked = drawn.iter().any(|d| d.chosen);
            let choices = if picked || drawn.len() < 2 { Vec::new() } else { drawn.iter().map(|d| (d.name.clone(), d.pictures[0].path.clone())).collect() };
            Some(Centre { screen: None, widths: Vec::new(), shown: d.pictures[0].path.clone(), width: None, at: d.at, direction: Some(d.name.clone()), choices })
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

fn look_lines(pass: bool, summary: &str, fixes: &[(String, String, String)], questions: &[(String, String)]) -> Vec<String> {
    let mut out = vec![if pass { t!("look.pass").to_owned() } else { t!("look.fixes", count = fixes.len()) }];
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
        for c in &f.critiques {
            let lines = c.text.lines().map(str::trim).filter(|l| !l.starts_with('#')).map(|l| l.trim_start_matches(['*', '-', ' ']).trim()).filter(|l| !l.is_empty()).map(str::to_owned).collect();
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
                out.push(Item { key: format!("look:{n}"), kind: ItemKind::Look, at: h.at.or(Some(verdict.at)), title, lines: look_lines(verdict.pass, &verdict.summary, &fixes, &questions), pictures: if last_run(screen, true) == Some(n) { picture_of(screen) } else { Vec::new() }, link: None, tone: if verdict.pass { Tone::Good } else { Tone::Bad } });
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
                    out.push(Item { key: format!("reclook:{}:{}", c.screen, l.at), kind: ItemKind::Look, at: Some(l.at), title: c.screen.clone(), lines: look_lines(l.pass, &l.summary, &fixes, questions), pictures: usual_first(&c.pictures), link: None, tone: if l.pass { Tone::Good } else { Tone::Bad } });
                }
            }
        }
    }
    out.sort_by_key(|i| std::cmp::Reverse(i.at.unwrap_or(0)));
    out
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
        Watch { ticket, happenings, files, working, waiting }
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
                })
                .collect();
            assert_eq!(did, ["skill", "brief", "looked", "looked", "proposed", "proposed", "chose", "built", "fail", "built", "pass", "refused", "look-fix", "built", "pass", "look-pass"], "{engine}");
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
    fn the_status_says_the_most_pressing_thing_first() {
        let t = ticket(TicketStatus::InProgress);
        let all = claude_work();
        let upto = |n: usize| all[..n].to_vec();
        // The first check fails: what failed, in words, and what it saw.
        let failing = upto(11);
        let w = watch(&t, &failing, None, true, None);
        assert_eq!(status(&w), Status::Failing { rule: "대비".into(), evidence: "the CTA is 2.47:1".into() });
        assert_eq!(status_words(&status(&w), 0), "검사 실패: 대비 — the CTA is 2.47:1");
        // Refused at the hand-in: why, until it checks again.
        let refused = upto(14);
        let w = watch(&t, &refused, None, true, None);
        assert_eq!(status(&w), Status::Refused("onboarding/welcome has had no second look: `equipe design second-look onboarding/welcome`.".into()));
        // The second look asked for a fix.
        let fixing = upto(15);
        assert_eq!(status(&watch(&t, &fixing, None, true, None)), Status::Fixing(1));
        // All passed, still at it: working, since its first step.
        let at_work = Watch { happenings: vec![Happening { at: Some(9_280), did: Did::Brief }], ..watch(&t, &all, None, true, None) };
        assert_eq!(status_words(&status(&at_work), 10_000), "작업 중 · 12분");
        // Waiting on the person to pick.
        let w = watch(&t, &all[..7], None, false, Some(WaitFor::Pick));
        assert_eq!(status(&w), Status::Waiting(WaitFor::Pick));
        assert_eq!(steps(&w)[2].mark, Mark::Waiting);
        // Handed in beats everything.
        let review = ticket(TicketStatus::Review);
        assert!(matches!(status(&watch(&review, &failing, None, false, None)), Status::HandedIn { .. }));
    }

    #[test]
    fn what_it_waits_on_is_a_pick_while_directions_are_out_and_none_is_picked() {
        let t = ticket(TicketStatus::InProgress);
        let proposed = vec![Happening { at: None, did: Did::Proposed("A".into()) }];
        assert_eq!(waiting_for(&t, Some("Which one?"), None, &proposed), Some(WaitFor::Pick));
        assert_eq!(waiting_for(&t, Some("Which one?"), None, &[]), Some(WaitFor::Answer("Which one?".into())));
        let picked = [proposed.clone(), vec![Happening { at: None, did: Did::Chose("A".into()) }]].concat();
        assert_eq!(waiting_for(&t, Some("The price?"), None, &picked), Some(WaitFor::Answer("The price?".into())));
        assert_eq!(waiting_for(&t, None, None, &picked), None);
        assert_eq!(waiting_for(&ticket(TicketStatus::Waiting), None, None, &proposed), Some(WaitFor::Pick));
    }

    fn pic(path: &str, width: Option<u32>, modified: u64) -> StudioPicture {
        StudioPicture { path: path.into(), modified, width }
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
        let c = centre(&f, None).unwrap();
        assert_eq!((c.screen.as_deref(), c.width), (Some("onboarding/welcome"), Some(1280)));
        assert_eq!(c.widths.len(), 7);
        assert_eq!(centre(&f, Some(360)).unwrap().shown, "/w/design/checks/onboarding/welcome/360.png");
        // A mockup made after the check: that is what it is making now.
        let mut newer = f.clone();
        newer.pictures[0].modified = 900;
        assert_eq!(centre(&newer, None).unwrap().shown, "/w/design/mockups/welcome-v1.png");
        assert_eq!(centre(&newer, Some(390)).unwrap().width, Some(390), "a width picked keeps the check's");
        // Nothing checked or made yet: the direction picked.
        let only = Studio { checks: vec![], pictures: vec![], ..f.clone() };
        let c = centre(&only, None).unwrap();
        assert_eq!((c.shown.as_str(), c.direction.as_deref()), ("/w/design/directions/calm-ledger/home.png", Some("Calm ledger")));
        assert!(c.choices.is_empty());
        // None picked yet: both, side by side, to pick from.
        let mut open = only.clone();
        open.directions[1].chosen = false;
        assert_eq!(centre(&open, None).unwrap().choices.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Poster", "Calm ledger"]);
        assert_eq!(centre(&Studio::default(), None), None);
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
        assert_eq!(status_words(&Status::Waiting(WaitFor::Pick), 0), "기다리는 중: 방향을 골라 주세요");
        assert_eq!(status_tone(&Status::Fixing(2)), Tone::Bad);
        assert!(browsing(&[Happening { at: None, did: Did::Browsed }], true));
        assert!(!browsing(&[Happening { at: None, did: Did::Browsed }], false));
    }
}
