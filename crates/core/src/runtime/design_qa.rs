//! Design QA: a code ticket that changes what users see goes past a designer
//! before the person reviews it. Its engineer says so when handing in
//! (`changes_screens`, with screenshots); equipe puts a design QA ticket on
//! the board for the squad's designer, and the hand-in waits in review,
//! marked, until the designer passes it (the person is told then) or sends it
//! back to its engineer with what to change. A squad with no designer skips
//! the step. The person stays the gate: accepting, sending back or cancelling
//! the ticket meanwhile drops its design QA.
//!
//! Before the designer's turn a deterministic precheck renders and checks
//! the screens the change touches (`crate::design::precheck`, run off the
//! loop as `equipe design precheck`); the design QA ticket is held off the
//! board meanwhile. Its gate (`crate::design_qa::gate`), when the squad has
//! it on, passes a change that changed no picture and is clean, or sends a
//! definite S1/S2 failure straight back — no designer turn; otherwise, and
//! always in shadow, the designer gets the report and looks at what it
//! flags. Each design QA ticket's way through is kept (`design_qa_runs`) for
//! `equipe design qa-stats`; its tier follows the squad's policy
//! (`crate::design_qa::tier_for`).
//!
//! In `risk_batch` mode (`crate::design_qa::Mode`, the default) the precheck
//! is also the engineer's self-check: a new S1/S2 finding refuses the hand-in
//! back to its engineer at once, unless waived with a reason; a low-risk
//! change goes on without a designer turn into the daily review
//! (`super::design_review`); only a high-risk one gets a designer turn of
//! its own, in a light session when the report shows enough.
//!
//! In `risk_batch` with a precheck to run, no design QA ticket is made at
//! hand-in: the change holds itself (its `design_qa` names its own id) while
//! its screens are prechecked, the result is recorded on it, and only a
//! change that needs a designer now gets a design QA ticket; the rest go on
//! into the daily review, counted (`Journal::design_qa_skipped_since`).
//!
//! A designer never starts on a change whose precheck is still under way —
//! not even after a restart, which starts it again. A precheck that couldn't
//! render on a loaded machine runs again later, waiting longer each time;
//! only after [`PRECHECK_TRIES`] such tries does a designer look, at the main
//! width and the changed states only. A design QA turn has a budget
//! ([`QA_TURN_CALLS`], [`QA_TURN_SECS`]): when it runs out the designer is
//! told to finish with what it has.
//!
//! Design QA stays one level deep: a designer's polish follow-up, or any
//! follow-up of a follow-up (`super::follow_ups`), goes into the daily
//! review instead of a designer turn of its own; its precheck still refuses
//! a new S1/S2.

use std::time::Duration;

use super::*;
use crate::design::precheck;
use crate::design_qa::{Decision, GateMode, Mode, Route, RouteCall};

/// The role that checks screens.
pub(super) const DESIGNER: &str = "designer";

/// The folder of a repository a designer's code ticket may change.
pub(super) const DESIGN_DIR: &str = "design";

/// Of the files a ticket changed, those outside the repository's design
/// folder — the first few, and how many more.
fn outside_design(files: &[String]) -> Vec<String> {
    const SHOWN: usize = 8;
    let outside: Vec<&String> = files.iter().filter(|f| f.strip_prefix(DESIGN_DIR).is_none_or(|rest| !rest.starts_with('/'))).collect();
    let mut named: Vec<String> = outside.iter().take(SHOWN).map(|f| format!("`{f}`")).collect();
    if outside.len() > SHOWN {
        named.push(format!("and {} more", outside.len() - SHOWN));
    }
    named
}

/// Who the precheck's gate passes or sends work back as, on its record.
pub(super) const PRECHECK_BY: &str = "equipe";

/// How long a precheck may run before it is stopped, and what it started with it.
const PRECHECK_LIMIT: Duration = Duration::from_secs(25 * 60);

/// How many days a squad's precheck reports are kept.
const PRECHECK_KEPT_DAYS: u64 = 30;

/// How many prechecks may fail to render on a loaded machine before a
/// designer looks anyway — at the main width and the changed states only.
pub(super) const PRECHECK_TRIES: u32 = 3;

/// The wait before a precheck runs again after it couldn't render on a
/// loaded machine; each next try waits twice as long, at most an hour.
const PRECHECK_RETRY_AFTER: Duration = Duration::from_secs(5 * 60);

/// How long a precheck waits for an overloaded machine to calm down before it runs anyway.
const PRECHECK_CALM_AT_MOST: Duration = Duration::from_secs(10 * 60);

/// How often a running precheck looks whether the machine is overloaded.
const PRECHECK_LOAD_EVERY: Duration = Duration::from_secs(30);

/// What a precheck stopped at [`PRECHECK_LIMIT`] says.
const PRECHECK_TIMED_OUT: &str = "ran past";

/// A design QA turn's budget: tool calls, and seconds. Past either, the
/// designer is told to finish with what it has.
pub(super) const QA_TURN_CALLS: u32 = 30;
pub(super) const QA_TURN_SECS: u64 = 15 * 60;
/// A daily design review's or audit's: one look over many changes.
pub(super) const REVIEW_TURN_CALLS: u32 = 60;
pub(super) const REVIEW_TURN_SECS: u64 = 30 * 60;

/// The wait before precheck try `tries` + 1 (`tries` failed so far).
pub(super) fn retry_delay(tries: u32) -> Duration {
    (PRECHECK_RETRY_AFTER * 2u32.saturating_pow(tries.saturating_sub(1))).min(Duration::from_secs(60 * 60))
}

/// Whether a precheck saw too little because the machine was loaded: it
/// gave no report or rendered (almost) nothing, and either the machine was
/// overloaded while it ran (or it waited for calm in vain) or it was stopped
/// at its time limit. Such a precheck runs again later instead of handing the
/// change to a designer.
pub(super) fn unseen_under_load(report: Option<&precheck::Report>, result: &Result<(), String>, loaded: bool) -> bool {
    use crate::design_qa::Judgment;
    let unseen = matches!(crate::design_qa::gate(report).judgment, Some(Judgment::NotRendered | Judgment::PrecheckFailed));
    let timed_out = result.as_ref().err().is_some_and(|why| why.contains(PRECHECK_TIMED_OUT));
    unseen && (loaded || timed_out)
}

/// What a designer looks at once the precheck couldn't render after
/// [`PRECHECK_TRIES`] tries: the main width, light, and the states the
/// change touched — not every width, scheme and text size.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Narrowed {
    pub width: u32,
    pub states: Vec<String>,
    pub files: Vec<String>,
}

impl Narrowed {
    /// Read from the last report, if any: its widest condition, its touched
    /// states and changed UI files off the map.
    pub fn of(report: Option<&precheck::Report>) -> Narrowed {
        let width = report
            .and_then(|r| r.conditions.iter().filter_map(|c| precheck::Condition::parse(c)).map(|c| c.width).max())
            .unwrap_or(precheck::WIDTHS[precheck::WIDTHS.len() - 1]);
        let states = report.map(|r| r.touched.clone()).unwrap_or_default();
        let files = report.map(|r| r.unmapped.clone()).unwrap_or_default();
        Narrowed { width, states, files }
    }
}

/// The pictures of a precheck worth reusing: those that rendered, of the
/// states it flags or the change changed first, at most `max`.
pub(super) fn reusable_pictures(report: &precheck::Report, max: usize) -> Vec<String> {
    let changed = report.changed();
    let mut out: Vec<String> = report.flagged();
    for s in report.shots.iter().filter(|s| s.error.is_none() && changed.contains(&s.key)) {
        if let Some(p) = &s.picture
            && !out.contains(p)
        {
            out.push(p.clone());
        }
    }
    out.truncate(max);
    out
}

/// Where design QA ticket `qa`'s precheck keeps its report and pictures:
/// beside the squad's journal, in no repository.
pub(super) fn precheck_dir(home: &Path, squad: &str, qa: TicketId) -> PathBuf {
    journal::squad_dir(home, squad).join("design-qa").join(format!("T-{qa}"))
}

/// What a designer on a design QA ticket is given of its precheck.
#[derive(Debug, Clone, Default)]
pub(super) struct PrecheckBrief {
    pub dir: PathBuf,
    pub summary: String,
    /// Its pictures to reuse instead of shooting them again.
    pub pictures: Vec<String>,
    /// It couldn't render after [`PRECHECK_TRIES`] tries: what to look at instead.
    pub narrowed: Option<Narrowed>,
}

/// Pictures of the precheck a design QA brief names to reuse.
const BRIEF_PICTURES: usize = 8;

/// How a design QA hand-in ends: the designer's word on the screens.
#[derive(Clone, Debug)]
pub(super) struct Verdict {
    pub passed: bool,
    pub findings: String,
}

/// How equipe ended a design QA ticket by itself, without a designer turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GateEnd {
    /// The gate passed it (`per_ticket` with the gate on).
    Passed,
    /// The gate sent it back (`per_ticket` with the gate on).
    SentBack,
    /// The engineer's self-check found new S1/S2s it didn't waive (`risk_batch`).
    Refused,
    /// Low risk: on to review, into the daily design review (`risk_batch`).
    Batched,
}

/// A design QA, review or audit turn's spend against its budget.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Budget {
    /// Tool calls so far this turn.
    pub calls: u32,
    /// Its budget: tool calls, seconds; `None`: not a design check.
    pub cap: Option<(u32, u64)>,
    /// Told to finish already.
    pub told: bool,
    /// A design setup step's turn (`crate::design_tier`), not a check.
    pub step: bool,
}

/// Whether a turn `secs` in with `calls` tool calls is past `cap`.
pub(super) fn over_budget(calls: u32, secs: u64, (max_calls, max_secs): (u32, u64)) -> bool {
    calls >= max_calls || secs >= max_secs
}

impl Runtime {
    /// A tool call of `id`'s turn: on a design QA ticket, a daily review or
    /// an audit, counted against its budget — once past it, the member is
    /// told, once, to finish now with what it has (read at its next step).
    pub(super) fn watch_qa_budget(&mut self, id: SessionId) {
        let Some((squad, ticket, began, name)) = self.sessions.get(&id).and_then(|s| Some((s.squad.clone()?, s.work.as_ref()?.ticket, s.turn_began, s.name()?.to_owned()))) else { return };
        if !self.qa_budgets.contains_key(&id) {
            let (cap, step) = if self.checked_by(&squad, ticket).is_some() {
                (Some((QA_TURN_CALLS, QA_TURN_SECS)), false)
            } else if self.design_batch_kind(&squad, ticket).is_some() {
                (Some((REVIEW_TURN_CALLS, REVIEW_TURN_SECS)), false)
            } else {
                // A design setup step (the starter brand, a deep step) has its own turn cap: `crate::design_tier`.
                let cap = self.tickets(&squad).iter().find(|t| t.id == ticket).and_then(|t| crate::design_tier::classify(&t.title, &t.body)).map(|step| step.turn());
                (cap, cap.is_some())
            };
            self.qa_budgets.insert(id, Budget { cap, step, ..Budget::default() });
        }
        let Some(budget) = self.qa_budgets.get_mut(&id) else { return };
        let Some(cap) = budget.cap else { return };
        budget.calls += 1;
        let secs = began.map_or(0, |at| now_secs().saturating_sub(at));
        if budget.told || !over_budget(budget.calls, secs, cap) {
            return;
        }
        budget.told = true;
        let (calls, minutes) = (cap.0, cap.1 / 60);
        if budget.step {
            // A design setup step past its turn's budget: an early stop, with what it has.
            crate::daemon_log!("equipe: {squad}: {name}'s design step T-{ticket} used its turn budget ({} tool calls, {}s): told to finish", budget.calls, secs);
            let prompt = format!(
                "[equipe: this design step has used its budget ({calls} tool calls or {minutes} minutes). Stop here: commit what you have, \
hand it in now with what is done and what is left in one line each, then end your turn. Do not start anything new.]"
            );
            let notice = say!(self, "notice.design_step_budget", calls = calls.to_string(), minutes = minutes.to_string(), name = name);
            self.publish(Event::Notice { undo: None, session: id, text: notice });
            match self.sessions.get(&id).and_then(|s| s.steer.clone()) {
                Some(steer) => {
                    let _ = steer.send(prompt);
                }
                None => self.leave_memo(id, prompt),
            }
            return;
        }
        let tool = if cap.0 == QA_TURN_CALLS { board::DESIGN_QA } else { board::DESIGN_REVIEW };
        crate::daemon_log!("equipe: {squad}: {name}'s design check of T-{ticket} used its budget ({} tool calls, {}s): told to finish", budget.calls, secs);
        let prompt = format!(
            "[equipe: this check has used its budget ({calls} tool calls or {minutes} minutes). Stop looking further: finish now with `{tool}` on \
what you have — say in findings what you couldn't look at — then end your turn.]"
        );
        let notice = say!(self, "notice.design_qa_budget", calls = calls.to_string(), minutes = minutes.to_string(), name = name);
        self.publish(Event::Notice { undo: None, session: id, text: notice });
        match self.sessions.get(&id).and_then(|s| s.steer.clone()) {
            Some(steer) => {
                let _ = steer.send(prompt);
            }
            None => self.leave_memo(id, prompt),
        }
    }

    /// Whether `squad` has a designer to check screens.
    pub(super) fn has_designer(&self, squad: &str) -> bool {
        self.team(squad).iter().any(|(_, _, role)| role == DESIGNER)
    }

    /// The ticket design QA ticket `qa` checks, while it waits on it. A
    /// change held on itself for its precheck is no design QA ticket.
    pub(super) fn checked_by(&mut self, squad: &str, qa: TicketId) -> Option<Ticket> {
        self.tickets(squad).iter().find(|t| t.design_qa == Some(qa) && t.id != qa).cloned()
    }

    /// The change waiting on design QA run `qa`: the one its design QA ticket
    /// checks, or the change itself while held for its precheck.
    pub(super) fn waiting_on(&mut self, squad: &str, qa: TicketId) -> Option<Ticket> {
        self.tickets(squad).iter().find(|t| t.design_qa == Some(qa)).cloned()
    }

    /// What a designer's own hand-in needs (`design::lint::record::hand_in`):
    /// each UI file it edited drawn and checked, the floor passed, and a fresh
    /// second look that passed after the check. Refused with what to do;
    /// on the way through, the edits' ledger is cleared for the next ticket.
    /// Reads two or three small files in its worktree's `design/`.
    /// `Ok(Some(gap))`: refused the same way a second time, or its screens'
    /// looks asked for fixes [`crate::design::lint::record::LOOKS_BEFORE_THE_PERSON`]
    /// times in a row, it goes in with what stood in its way, to be marked for
    /// the person — a check that can't be cleared is the person's to see, not
    /// a loop for the member.
    pub(super) fn designer_hand_in(&mut self, id: SessionId) -> Result<Option<Vec<String>>, String> {
        let Some(session) = self.sessions.get_mut(&id) else { return Ok(None) };
        if session.member.as_ref().is_none_or(|m| m.role != DESIGNER) {
            return Ok(None);
        }
        let cwd = session.work.as_ref().map_or_else(|| session.cwd.clone(), |w| w.cwd.clone());
        let design = cwd.join(crate::design::DIR);
        let needs = crate::design::lint::record::hand_in(&design);
        let again = session.work.as_ref().and_then(|w| w.refused.as_ref()).is_some_and(|before| *before == needs);
        if !needs.is_empty() && !again {
            if let Some(work) = session.work.as_mut() {
                work.refused = Some(needs.clone());
            }
            return Err(format!(
                "Not yet — a screen you made goes to the person only checked and seen fresh:\n{}\nThen hand in again. If you hand in again the same way, it goes in with these marked for the person as not cleared.",
                needs.iter().map(|n| format!("- {n}")).collect::<Vec<_>>().join("\n")
            ));
        }
        let mut gap = needs;
        gap.extend(crate::design::lint::record::open_fixes(&design));
        crate::design::lint::record::handed_in(&design);
        if let Some(work) = session.work.as_mut() {
            work.refused = None;
        }
        Ok((!gap.is_empty()).then_some(gap))
    }

    /// What a designer's hand-in that changes a screen carries beyond its
    /// summary: its change note — before → after a point, why each is better,
    /// and what stayed as it was. It changes a screen when it says so
    /// (`changes_screens`), when it edited or checked a UI file since the
    /// hand-in before, or when its branch touches one outside `design/`.
    /// Refused with what to add; `Ok(Some(note))`, the note to put under the
    /// summary (given unasked, it is kept too).
    pub(super) fn designer_change_note(&self, id: SessionId, args: &Value) -> Result<Option<String>, String> {
        let Some(session) = self.sessions.get(&id) else { return Ok(None) };
        if session.member.as_ref().is_none_or(|m| m.role != DESIGNER) {
            return Ok(None);
        }
        let Some(work) = session.work.as_ref() else { return Ok(None) };
        let changes = board::changes_arg(args);
        let unchanged = board::arg_str(args, "unchanged").unwrap_or_default();
        let said = args.get("changes_screens").and_then(Value::as_bool) == Some(true);
        let ledger = crate::design::lint::record::read_pending(&work.cwd.join(crate::design::DIR));
        let touched = || work.sandbox.as_ref().and_then(|s| s.touched_files().ok()).unwrap_or_default();
        let screens = said || !ledger.edited.is_empty() || !ledger.checked.is_empty() || touched().iter().any(|f| !f.starts_with(&format!("{DESIGN_DIR}/")) && precheck::ui_file(f));
        if screens && let Some(gap) = board::change_note_gap(&changes, unchanged) {
            return Err(format!(
                "This hand-in changes a screen, so it says what changed and why it is better: add {gap}. Keep features, content and data as they were unless the ticket asked otherwise, and say so in `unchanged`. Then hand in again."
            ));
        }
        Ok((!changes.is_empty()).then(|| crate::protocol::decide::change_note(&changes, unchanged)))
    }

    /// What `submit_for_review` asks of an engineer's code ticket: whether
    /// users see the change, and if so fresh screenshots across the design QA
    /// range. A squad with a designer sends a complete hand-in to design QA.
    /// `Ok(true)`: it goes to design QA when the turn ends.
    pub(super) fn screens_handed_in(&mut self, squad: &str, id: SessionId, args: &Value, pictures: usize) -> Result<bool, String> {
        let Some(session) = self.sessions.get(&id) else {
            return Ok(false);
        };
        let Some((ticket, kind)) = session.work.as_ref().map(|w| (w.ticket, w.kind)) else {
            return Ok(false);
        };
        let role = session.member.as_ref().map(|m| m.role.clone()).unwrap_or_default();
        // A designer's code ticket changes the repository's design assets only.
        if kind == TicketKind::Code && role == DESIGNER {
            let touched = session.work.as_ref().and_then(|w| w.sandbox.as_ref()).and_then(|s| s.touched_files().ok()).unwrap_or_default();
            let outside = outside_design(&touched);
            if !outside.is_empty() {
                return Err(format!(
                    "On a code ticket a designer changes only the repository's `{DESIGN_DIR}/` folder, and these are outside it: {}. \
Undo them, propose that work as a ticket for an engineer (create_ticket), and hand in again.",
                    outside.join(", ")
                ));
            }
        }
        if self.checked_by(squad, ticket).is_some() {
            return Err(format!("T-{ticket} is a design QA ticket: finish it with design_qa — pass, or changes with your findings."));
        }
        if self.design_batch_kind(squad, ticket).is_some() {
            return Err(format!("T-{ticket} is a daily design review or audit: finish it with design_review, one entry per change."));
        }
        // A designer's own work has its stricter design ledger; non-code work
        // has no built screen to prove here.
        if kind != TicketKind::Code || role == DESIGNER {
            return Ok(false);
        }
        match args.get("changes_screens").and_then(Value::as_bool) {
            None => Err("Say whether this changes what users see (changes_screens: true or false). Then hand in again."
                .into()),
            Some(true) if pictures == 0 => Err("Screens changed: save fresh screenshots of each changed screen in \
your worktree, name them in images, and hand in again."
                .into()),
            Some(true) => {
                const ALL: [(&str, &str); 4] = [
                    ("languages", "languages"),
                    ("widths", "widths"),
                    ("light_dark", "light and dark"),
                    ("large_text", "large text"),
                ];
                let checked = board::arg_strs(args, "screen_checks");
                let missing = ALL.iter().filter(|(key, _)| !checked.iter().any(|got| got == key)).map(|(_, label)| *label).collect::<Vec<_>>();
                // A polish follow-up is shown where its findings are; the precheck renders the rest.
                let light = self.follow_up_of(squad, ticket).is_some_and(|f| f.light_check());
                if !missing.is_empty() && !light {
                    return Err(format!(
                        "Screens changed: the hand-in screenshots must cover the same range as design QA. Add every \
screen_checks value; missing: {}. Then hand in again.",
                        missing.join(", ")
                    ));
                }
                Ok(self.has_designer(squad))
            }
            Some(false) => Ok(false),
        }
    }

    /// `ticket`, just handed in with changed screens, gets its design QA: a
    /// task for the squad's designer, which it waits on in review.
    pub(super) fn ask_design_qa(&mut self, squad: &str, ticket: &mut Ticket, engineer: &str, waived: Option<&str>) -> Option<TicketId> {
        let qa = self.design_qa_ticket(ticket, engineer);
        let can_run = self.config.equipe_binary.is_some();
        let journal = self.journal(squad)?;
        let gate = journal.design_qa_gate();
        let mode = journal.design_qa_settings().mode;
        let runs = gate != GateMode::Off && can_run && ticket.worktree.as_deref().is_some_and(Path::exists) && ticket.project.is_some();
        // Risk-based with a precheck to run: no design QA ticket yet. The change holds itself while
        // its screens are prechecked; a design QA ticket is made only if a designer is needed now.
        if mode == Mode::RiskBatch && runs {
            let round = journal.design_qa_rounds(ticket.id).unwrap_or(0) + 1;
            if let Err(err) = journal.start_held_design_qa_run(ticket.id, round, gate.code(), "running") {
                crate::daemon_log!("equipe: {squad}: T-{}'s design QA run not kept: {err}", ticket.id);
            }
            let _ = journal.set_design_qa_mode_of(ticket.id, mode.code(), waived);
            ticket.design_qa = Some(ticket.id);
            self.start_precheck(squad, ticket.id, ticket, Duration::ZERO);
            return Some(ticket.id);
        }
        // Risk-based with no precheck to run (the gate is off, or there is nothing to run it on):
        // nothing is known of the change, and a gate that saw nothing never passes it — a designer
        // looks at it now, through its design QA ticket below, as when the precheck rendered nothing.
        let qa_id = journal.create_ticket(&qa).ok()?;
        let _ = journal.ticket_event(qa_id, "equipe", "created", &format!("design QA of T-{}", ticket.id));
        ticket.design_qa = Some(qa_id);
        let made = journal.ticket(qa_id).ok().flatten().unwrap_or(Ticket { id: qa_id, ..qa });
        // Its precheck first, unless the squad has it off or there is nothing to run it on.
        let round = journal.design_qa_rounds(ticket.id).unwrap_or(1).max(1);
        if let Err(err) = journal.start_design_qa_run(qa_id, ticket.id, round, gate.code(), if runs { "running" } else { "skipped" }) {
            crate::daemon_log!("equipe: {squad}: T-{qa_id}'s design QA run not kept: {err}");
        }
        let _ = journal.set_design_qa_mode_of(qa_id, mode.code(), waived);
        if mode == Mode::RiskBatch && !runs {
            let _ = journal.set_design_qa_route(qa_id, Route::PerTicket.code(), crate::design_qa::Risk::Unverified.code(), false);
        }
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: made });
        if runs {
            self.start_precheck(squad, qa_id, ticket, Duration::ZERO);
        }
        Some(qa_id)
    }

    /// Whether design QA ticket `ticket` waits on its precheck: nobody takes it meanwhile.
    pub(super) fn prechecking(&self, squad: &str, ticket: TicketId) -> bool {
        self.prechecks.keys().any(|(q, t)| q == squad && *t == ticket)
    }

    /// Open design QA tickets nobody has started that no longer hold their
    /// change under relaxed risk-based routing — made before it, or for a
    /// risk that now waits for the daily review: closed into the batch, their
    /// change on to landing. Those for new screens, waived S1/S2 findings or
    /// a look the person asked for stay. Before each dispatch, so no designer
    /// takes one first.
    pub(super) fn release_design_qa(&mut self, squad: &str) {
        let Some(journal) = self.journal(squad) else { return };
        let settings = journal.design_qa_settings();
        if settings.mode != Mode::RiskBatch || settings.tight {
            return;
        }
        let open: Vec<Ticket> = self.tickets(squad).iter().filter(|t| t.status == TicketStatus::Todo && t.kind == TicketKind::Task && t.owner.as_deref() == Some(DESIGNER)).cloned().collect();
        for qa in open {
            if self.prechecking(squad, qa.id) || self.worker_on(squad, qa.id).is_some() || self.checked_by(squad, qa.id).is_none() {
                continue;
            }
            let Some(journal) = self.journal(squad) else { return };
            // Only those made under risk-based routing: a squad's per-ticket design QA stays as it was.
            let Some(run) = journal.design_qa_run(qa.id).filter(|r| r.mode.as_deref() == Some(Mode::RiskBatch.code())) else { continue };
            if run.precheck == "running" || run.risk.as_deref().is_some_and(crate::design_qa::Risk::codes_need_designer_now) {
                continue;
            }
            let risk = run.risk.clone().unwrap_or_default();
            let _ = journal.set_design_qa_route(qa.id, Route::Batch.code(), &risk, false);
            crate::daemon_log!("equipe: {squad}: design QA T-{} no longer holds its change: into the daily design review", qa.id);
            let findings = "**No longer held for a designer before landing: its screens go into the daily design review.**".to_owned();
            self.precheck_verdict(squad, qa, Verdict { passed: true, findings }, GateEnd::Batched);
        }
    }

    /// The design QA runs of `squad` whose precheck is under way by the
    /// journal — held off the board even when this daemon didn't start it
    /// (a restart ended the one that ran).
    pub(super) fn prechecks_pending(&mut self, squad: &str) -> HashSet<TicketId> {
        let mut out: HashSet<TicketId> = self.prechecks.keys().filter(|(q, _)| q == squad).map(|(_, t)| *t).collect();
        if let Some(journal) = self.journal(squad) {
            out.extend(journal.design_qa_prechecks_running().into_iter().map(|r| r.qa));
        }
        out
    }

    /// Prechecks the journal says are under way but nothing runs — a restart
    /// ended them: started again while their change still waits on them, else
    /// put down as failed. On the minute tick.
    pub(super) fn resume_prechecks(&mut self) {
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        let can_run = self.config.equipe_binary.is_some();
        for squad in squads {
            let Some(journal) = self.journal(&squad) else { continue };
            let orphans: Vec<journal::DesignQaRun> = journal.design_qa_prechecks_running();
            for run in orphans {
                if self.prechecking(&squad, run.qa) {
                    continue;
                }
                let Some(journal) = self.journal(&squad) else { break };
                let checked = journal.ticket(run.checked).ok().flatten().filter(|c| c.design_qa == Some(run.qa) && c.status == TicketStatus::Review);
                // A design QA ticket of its own must still be open; a change held on itself waits in review.
                let open = run.qa == run.checked || journal.ticket(run.qa).ok().flatten().is_some_and(|t| t.status == TicketStatus::Todo);
                match checked.filter(|_| open) {
                    Some(c) if can_run && c.worktree.as_deref().is_some_and(Path::exists) && c.project.is_some() => {
                        crate::daemon_log!("equipe: {squad}: design QA T-{}'s precheck was cut short (a restart); it runs again", run.qa);
                        self.start_precheck(&squad, run.qa, &c, Duration::ZERO);
                    }
                    Some(_) => {
                        // Nothing to run it on any more: on as a precheck that failed.
                        let _ = self.internal.send(Internal::Prechecked { squad: squad.clone(), qa: run.qa, result: Err("it was cut short and can't run again".into()), secs: 0, loaded: false });
                        self.prechecks.insert((squad.clone(), run.qa), std::time::Instant::now());
                    }
                    None => {
                        let mut gone = run.clone();
                        gone.precheck = "failed".into();
                        let _ = journal.set_design_qa_precheck(&gone);
                    }
                }
            }
        }
    }

    /// `equipe design precheck` on `checked`'s worktree, off the loop, for
    /// design QA ticket `qa`, held off the board until it ends
    /// ([`Internal::Prechecked`]) — stopped, with what it started, after
    /// [`PRECHECK_LIMIT`].
    fn start_precheck(&mut self, squad: &str, qa: TicketId, checked: &Ticket, after: Duration) {
        let (Some(bin), Some(worktree), Some(project)) = (self.config.equipe_binary.clone(), checked.worktree.clone(), checked.project.clone()) else { return };
        let dir = precheck_dir(&self.config.home, squad, qa);
        self.prechecks.insert((squad.to_owned(), qa), std::time::Instant::now());
        let (internal, squad, id, busy) = (self.internal.clone(), squad.to_owned(), checked.id, self.machine.busy.clone());
        crate::daemon_log!("equipe: {squad}: prechecking T-{id}'s screens for design QA T-{qa}{}", if after.is_zero() { String::new() } else { format!(" in {} min", after.as_secs() / 60) });
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            // Its browsers wait a while for an overloaded machine to calm down — not long: the change
            // waits on it, and a precheck that can't render under load lands it for the daily review.
            super::machine::wait_for_calm(&busy, PRECHECK_CALM_AT_MOST).await;
            let started = std::time::Instant::now();
            // Whether the machine was overloaded at any time it ran: a picture that timed out then says nothing of the change.
            let mut loaded = busy.load(std::sync::atomic::Ordering::Relaxed);
            let run = run_precheck(&bin, &worktree, &project, &dir, id);
            tokio::pin!(run);
            let result = loop {
                tokio::select! {
                    result = &mut run => break result,
                    () = tokio::time::sleep(PRECHECK_LOAD_EVERY) => loaded |= busy.load(std::sync::atomic::Ordering::Relaxed),
                }
            };
            loaded |= busy.load(std::sync::atomic::Ordering::Relaxed);
            let _ = internal.send(Internal::Prechecked { squad, qa, result, secs: started.elapsed().as_secs(), loaded });
        });
    }

    /// A precheck ended: its report kept with the run and on the design QA
    /// ticket's record (or the change's, held on itself), and — when the
    /// squad's gate is on and the ticket still waits — acted on; else the
    /// designer takes it, with the report. One that couldn't render on a
    /// loaded machine runs again later instead ([`unseen_under_load`]).
    pub(super) async fn on_prechecked(&mut self, squad: &str, qa: TicketId, result: Result<(), String>, secs: u64, loaded: bool) {
        self.prechecks.remove(&(squad.to_owned(), qa));
        let dir = precheck_dir(&self.config.home, squad, qa);
        let report = match &result {
            Ok(()) => precheck::read(&dir),
            Err(why) => {
                crate::daemon_log!("equipe: {squad}: design QA T-{qa}'s precheck failed: {why}");
                None
            }
        };
        // The change waiting on it: through its design QA ticket, or held on itself (`risk_batch`).
        let checked_now = self.waiting_on(squad, qa);
        let held = checked_now.as_ref().is_some_and(|c| c.id == qa);
        // What the precheck couldn't see goes to a designer: on a loaded machine it runs again
        // later first, so a picture it can take saves that designer turn.
        if let Some(c) = checked_now.as_ref().filter(|_| unseen_under_load(report.as_ref(), &result, loaded)) {
            let tries = self.journal(squad).and_then(|j| j.add_design_qa_precheck_try(qa).ok()).unwrap_or(PRECHECK_TRIES);
            let waiting = held || (self.journal(squad).and_then(|j| j.ticket(qa).ok().flatten()).is_some_and(|t| t.status == TicketStatus::Todo) && self.worker_on(squad, qa).is_none());
            if tries < PRECHECK_TRIES && waiting {
                let after = retry_delay(tries);
                let why = result.as_ref().err().cloned().unwrap_or_else(|| "it rendered too little".to_owned());
                crate::daemon_log!("equipe: {squad}: design QA T-{qa}'s precheck couldn't render on a loaded machine ({why}); again in {} min (try {} of {PRECHECK_TRIES})", after.as_secs() / 60, tries + 1);
                if let Some(journal) = self.journal(squad) {
                    let line = format!("The precheck couldn't render on a loaded machine ({why}); it runs again in {} min, when the machine is calm (try {} of {PRECHECK_TRIES}).", after.as_secs() / 60, tries + 1);
                    let _ = journal.ticket_event(qa, "equipe", "precheck", &line);
                }
                let c = c.clone();
                self.start_precheck(squad, qa, &c, after);
                return;
            }
        }
        // Built from a live prototype the person picked — or one UI slice, its prototype
        // and build on one branch (`crate::flow`): the precheck alone decides, whatever the squad's gate.
        let slice = checked_now.as_ref().is_some_and(|c| c.flow.as_ref().is_some_and(|f| f.slice));
        let picked = checked_now.as_ref().and_then(|c| self.approved_prototype(squad, c.id));
        let prototype = picked.clone().or_else(|| slice.then(String::new));
        let call = if prototype.is_some() { crate::design_qa::gate_prototype(report.as_ref()) } else { crate::design_qa::gate(report.as_ref()) };
        let Some(journal) = self.journal(squad) else { return };
        let mut run = journal.design_qa_run(qa).unwrap_or(journal::DesignQaRun { qa, ..Default::default() });
        let mode = GateMode::parse(&run.gate).unwrap_or_default();
        let tight = journal.design_qa_settings().tight;
        let recheck = journal.design_qa_before(run.checked, qa).is_some_and(|(verdict, by)| verdict.as_deref() == Some("changes") && by.as_deref().is_some_and(|b| b != PRECHECK_BY));
        let risk_batch = run.mode.as_deref() == Some(Mode::RiskBatch.code());
        let open = if held { checked_now.clone() } else { journal.ticket(qa).ok().flatten().filter(|t| t.status == TicketStatus::Todo) };
        let checked = self.waiting_on(squad, qa);
        let free = open.is_some() && checked.is_some() && (held || self.worker_on(squad, qa).is_none());
        // In risk_batch the route decides: refused, a designer now, or the daily review.
        let routed: Option<RouteCall> = match (risk_batch && prototype.is_none(), &checked) {
            (true, Some(c)) => {
                let spec = self.spec_of(c);
                let follow_up = self.follow_up_of(squad, c.id).is_some_and(|f| f.quiet_qa());
                let asked = crate::design_qa::person_asked(&c.created_by, c.label);
                Some(crate::design_qa::route(&crate::design_qa::RouteAsk { report: report.as_ref(), waived: run.waived.is_some(), spec, recheck, tight, follow_up, asked }))
            }
            _ => None,
        };
        let act = match &routed {
            Some(r) => free && r.route != Route::PerTicket,
            None if prototype.is_some() => call.decision == Decision::AutoPass && free,
            None => mode == GateMode::On && call.decision != Decision::Designer && free,
        };
        let headline = match &routed {
            Some(r) => route_headline(r, &call),
            None => call.headline(act),
        };
        let flagged = report.as_ref().map(precheck::Report::flagged).unwrap_or_default();
        if let Some(r) = &report {
            let _ = precheck::write(&dir, r, &r.summary(&headline));
        }
        run.precheck = if report.is_some() { "done" } else { "failed" }.to_owned();
        run.decision = Some(call.decision.code().to_owned());
        run.judgment = call.judgment.map(|j| j.code().to_owned());
        run.acted = act;
        run.report = report.is_some().then(|| dir.to_string_lossy().into_owned());
        run.head = report.as_ref().and_then(|r| r.head.clone());
        run.base = report.as_ref().and_then(|r| r.base.clone());
        run.precheck_secs = Some(secs);
        let Some(journal) = self.journal(squad) else { return };
        let _ = journal.set_design_qa_precheck(&run);
        if let Some(r) = &routed {
            let _ = journal.set_design_qa_route(qa, r.route.code(), &r.risk_codes(), r.lite && r.route == Route::PerTicket);
        }
        let said = match (&report, &result) {
            (Some(_), _) => format!("{headline} Report: {}", dir.join(precheck::REPORT_MD).display()),
            (None, Err(why)) => format!("The precheck couldn't run ({why}); the designer checks it as before."),
            (None, Ok(())) => "The precheck wrote no report; the designer checks it as before.".to_owned(),
        };
        let _ = journal.ticket_event(qa, "equipe", "precheck", &said);
        match &routed {
            Some(r) => crate::daemon_log!("equipe: {squad}: design QA T-{qa} precheck: {} [{}] ({secs}s)", r.route.code(), r.risk_codes()),
            None => crate::daemon_log!("equipe: {squad}: design QA T-{qa} precheck: {} ({}, {secs}s)", call.decision.code(), if act { "acted" } else { mode.code() }),
        }
        let checked_id = checked.as_ref().map(|c| c.id);
        let (Some(mut qa_ticket), Some(checked)) = (open, checked) else { return };
        // Held on itself and a designer is needed now: only now its design QA ticket.
        if held && !act {
            self.promote_held(squad, checked, flagged);
            self.dispatch(squad).await;
            return;
        }
        // What it flags, shown on the design QA ticket (never over the change's own pictures).
        if !flagged.is_empty() && !held {
            qa_ticket.images = flagged.iter().take(crate::visuals::HAND_IN).cloned().collect();
            if let Some(journal) = self.journal(squad) {
                let _ = journal.save_ticket(&qa_ticket);
            }
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: qa_ticket.clone() });
        }
        if act {
            let report_line = format!("Report: {}", dir.join(precheck::REPORT_MD).display());
            let blockers = |r: &precheck::Report| -> String {
                r.new_blockers().iter().take(8).map(|f| format!("- {} on {} [{}]: {} {}", f.rule, f.key, f.conditions.join(", "), f.selector, f.evidence)).collect::<Vec<_>>().join("\n")
            };
            let (how, findings) = match (routed.as_ref().map(|r| r.route), call.decision, &report) {
                (Some(Route::Refused), _, Some(r)) => (
                    GateEnd::Refused,
                    format!(
                        "{headline}\n{}\nFix each, then hand in again. If one isn't a real problem for the person (a decorative label, a deliberate \
truncation with the whole text available), hand in again with `waive_findings` saying why — a designer then judges it.\n{report_line}",
                        blockers(r)
                    ),
                ),
                (Some(_), _, _) => (GateEnd::Batched, format!("{headline}\n{report_line}")),
                (None, Decision::SendBack, Some(r)) => (GateEnd::SentBack, format!("{headline}\n{}\n{report_line}", blockers(r))),
                (None, Decision::SendBack, None) => (GateEnd::SentBack, format!("{headline}\n{report_line}")),
                // Built from the picked prototype: the person sees the result at once.
                (None, _, Some(r)) if prototype.is_some() => {
                    let shots: Vec<&str> = r.shots.iter().filter_map(|s| s.picture.as_deref()).take(3).collect();
                    let see = picked.as_deref().map(|p| format!("\nThe prototype it was built from: {p}")).unwrap_or_default();
                    (GateEnd::Passed, format!("{headline}\nSee the result: {}{see}\n{report_line}", shots.join(", ")))
                }
                (None, _, _) => (GateEnd::Passed, format!("{headline}\n{report_line}")),
            };
            self.precheck_verdict(squad, qa_ticket, Verdict { passed: matches!(how, GateEnd::Passed | GateEnd::Batched), findings }, how);
            // Built from the picked prototype and passed: the built screens on the ticket, one tap from the person's review.
            if let (true, GateEnd::Passed, Some(id), Some(r)) = (prototype.is_some(), how, checked_id, &report) {
                self.show_built_screens(squad, id, r);
            }
        }
        self.dispatch(squad).await;
    }

    /// A change held on itself for its precheck needs a designer now: its
    /// design QA ticket is made, its run and report move to it, and the change
    /// waits on it as on any design QA. The new ticket, if made.
    fn promote_held(&mut self, squad: &str, mut checked: Ticket, flagged: Vec<String>) -> Option<TicketId> {
        let engineer = checked.assignee.clone().or_else(|| checked.owner.clone()).unwrap_or_default();
        let mut qa = self.design_qa_ticket(&checked, &engineer);
        qa.images = flagged.into_iter().take(crate::visuals::HAND_IN).collect();
        let home = self.config.home.clone();
        let journal = self.journal(squad)?;
        let qa_id = journal.create_ticket(&qa).ok()?;
        let _ = journal.ticket_event(qa_id, "equipe", "created", &format!("design QA of T-{} after its precheck", checked.id));
        let _ = journal.rekey_design_qa_run(checked.id, qa_id);
        let (from, to) = (precheck_dir(&home, squad, checked.id), precheck_dir(&home, squad, qa_id));
        move_precheck(&from, &to);
        if let Some(mut run) = journal.design_qa_run(qa_id).filter(|r| r.report.is_some()) {
            run.report = Some(to.to_string_lossy().into_owned());
            let _ = journal.set_design_qa_precheck(&run);
        }
        checked.design_qa = Some(qa_id);
        let _ = journal.save_ticket(&checked);
        let _ = journal.ticket_event(checked.id, "equipe", "design QA", &format!("Its precheck asks for a designer: T-{qa_id}."));
        let made = journal.ticket(qa_id).ok().flatten().unwrap_or(Ticket { id: qa_id, ..qa });
        crate::daemon_log!("equipe: {squad}: T-{} needs a designer after its precheck: design QA T-{qa_id}", checked.id);
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: checked.clone() });
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: made });
        let line = format!("T-{}'s precheck asks for a designer: the designer checks its screens on T-{qa_id} before the person reviews it", checked.id);
        let shown = say!(self, "news.design_qa_asked", name = engineer, id = checked.id, to = DESIGNER, title = checked.title);
        self.brief_pm(squad, line, Some(shown));
        Some(qa_id)
    }

    /// A design QA ticket for `ticket`, which `engineer` handed in, not yet on the board.
    fn design_qa_ticket(&self, ticket: &Ticket, engineer: &str) -> Ticket {
        Ticket {
            id: 0,
            title: say!(self, "ticket.design_qa_title", id = ticket.id, title = ticket.title),
            body: say!(self, "ticket.design_qa_body", id = ticket.id, name = engineer),
            status: TicketStatus::Todo,
            kind: TicketKind::Task,
            owner: Some(DESIGNER.to_owned()),
            assignee: None,
            project: ticket.project.clone(),
            depends_on: Vec::new(),
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            design_qa: None, landing: false, plan: None, model: None,
            flow: None,
            created_by: "equipe".to_owned(),
            note: None,
            asks: Vec::new(),
            label: None, remaining: Vec::new(),
            images: Vec::new(),
            updated_at: None,
            started_at: None,
            app: None,
            priority: ticket.priority,
            rank: 0,
        }
    }

    /// The pictures the precheck took of `ticket`'s built screens, on the
    /// ticket when it has none of its own: its review shows the result.
    fn show_built_screens(&mut self, squad: &str, ticket: TicketId, report: &precheck::Report) {
        let shots: Vec<String> = report.shots.iter().filter(|s| s.error.is_none()).filter_map(|s| s.picture.clone()).take(crate::visuals::HAND_IN).collect();
        let Some(journal) = self.journal(squad) else { return };
        let Some(mut t) = journal.ticket(ticket).ok().flatten().filter(|t| t.images.is_empty()) else { return };
        if shots.is_empty() {
            return;
        }
        t.images = shots;
        let _ = journal.save_ticket(&t);
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: t });
    }

    /// The gate's verdict on design QA ticket `qa`, without a designer turn:
    /// it closes, and the ticket it checked goes on as a designer's verdict
    /// would send it — to the person's review, or back to its engineer.
    /// A change held on itself (`qa` is the change) has no design QA ticket
    /// to close: the verdict is recorded on the change alone.
    fn precheck_verdict(&mut self, squad: &str, mut qa: Ticket, verdict: Verdict, how: GateEnd) {
        let held = qa.design_qa == Some(qa.id);
        if !held {
            qa.status = TicketStatus::Done;
            qa.note = Some(verdict.findings.clone());
        }
        let event = if verdict.passed { "design QA passed" } else { "design QA: changes" };
        if let Some(journal) = self.journal(squad) {
            if !held {
                let _ = journal.save_ticket(&qa);
                let _ = journal.ticket_event(qa.id, PRECHECK_BY, event, &verdict.findings);
            }
            // Its own verdict first: the first one kept stays.
            match how {
                GateEnd::Refused => {
                    let _ = journal.set_design_qa_verdict(qa.id, "refused", PRECHECK_BY);
                }
                GateEnd::Batched => {
                    let _ = journal.set_design_qa_verdict(qa.id, "batched", PRECHECK_BY);
                    // Counted, to weigh later what skipping a designer's own look cost.
                    let week = journal.design_qa_skipped_since(now_secs() as i64 - 7 * 24 * 60 * 60);
                    crate::daemon_log!("equipe: {squad}: T-{} skipped its own design QA (low risk by its precheck; into the daily review) — {week} skipped in the last 7 days", qa.id);
                }
                GateEnd::Passed | GateEnd::SentBack => {}
            }
        }
        if !held {
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: qa.clone() });
        }
        // A self-check refusal is a lint, not a send-back: it never raises the engineer's model.
        let Some((checked, engineer)) = self.design_qa_checked(squad, &qa, PRECHECK_BY, &verdict, how != GateEnd::Refused) else { return };
        let first = verdict.findings.lines().next().unwrap_or_default();
        let on = if held { "its own precheck".to_owned() } else { format!("T-{}", qa.id) };
        let (line, shown) = match how {
            GateEnd::Passed => (
                format!("equipe's precheck passed the design QA of T-{} ({on}): no screen changed and nothing new was found; it is ready for the person's review", checked.id),
                say!(self, "news.design_qa_precheck_passed", id = checked.id, title = checked.title),
            ),
            GateEnd::SentBack => (
                format!("equipe's precheck sent T-{} back to {engineer} without a designer turn ({on}): {first}", checked.id),
                say!(self, "news.design_qa_precheck_changes", id = checked.id, title = checked.title, to = engineer),
            ),
            GateEnd::Refused => (
                format!("equipe's self-check refused T-{}'s hand-in back to {engineer} without a designer turn ({on}): {first}", checked.id),
                say!(self, "news.design_qa_refused", id = checked.id, title = checked.title, to = engineer),
            ),
            GateEnd::Batched => (
                format!("T-{}'s screens are low risk by equipe's precheck ({on}): no designer turn now, it goes into the daily design review; it is ready for the person's review", checked.id),
                say!(self, "news.design_qa_batched", id = checked.id, title = checked.title),
            ),
        };
        self.brief_pm(squad, line, Some(shown));
        // Where the PM accepts, work ready for review lands now: by its member
        // when nothing in it needs a person (`runtime::flow`), else the PM acts on it.
        if verdict.passed && checked.ready_for_review() && self.journal(squad).is_some_and(|j| j.pm_accepts()) {
            let how = if how == GateEnd::Batched { "is low risk by equipe's precheck (its screens go into the daily design review)" } else { "passed design QA by equipe's precheck" };
            let prompt = format!(
                "[equipe: T-{} {how} and is ready for you. Check it and accept it with `accept_work`, send it back with `comment` and `review: changes`, or put it to the person with `propose_accept`.]",
                checked.id
            );
            self.work_ready(squad, checked.id, prompt);
        }
    }

    /// What the designer on design QA ticket `qa` is given of its precheck:
    /// its report's summary, when there is one.
    pub(super) fn precheck_brief(&mut self, squad: &str, qa: TicketId) -> Option<PrecheckBrief> {
        let home = self.config.home.clone();
        let journal = self.journal(squad)?;
        let run = journal.design_qa_run(qa)?;
        let tries = journal.design_qa_precheck_tries(qa);
        let dir = run.report.as_deref().map_or_else(|| precheck_dir(&home, squad, qa), PathBuf::from);
        let report = run.report.as_ref().and_then(|_| precheck::read(&dir));
        let summary = run.report.as_ref().and_then(|_| std::fs::read_to_string(dir.join(precheck::REPORT_MD)).ok());
        // It couldn't render after its tries: a narrowed look instead of the whole matrix.
        let unseen = run.precheck == "failed" || run.judgment.as_deref().and_then(crate::design_qa::Judgment::parse).is_some_and(|j| matches!(j, crate::design_qa::Judgment::NotRendered | crate::design_qa::Judgment::PrecheckFailed));
        let narrowed = (unseen && tries >= PRECHECK_TRIES).then(|| Narrowed::of(report.as_ref()));
        if summary.is_none() && narrowed.is_none() {
            return None;
        }
        let pictures = report.as_ref().map(|r| reusable_pictures(r, BRIEF_PICTURES)).unwrap_or_default();
        Some(PrecheckBrief { dir, summary: summary.unwrap_or_default(), pictures, narrowed })
    }

    /// Design QA ticket `qa`'s tier as the squad's policy reads it, for
    /// `member` (of `role`, on `engine`): the cheap tier for routine QA; the
    /// top tier for a redesign, a third round or more, S1/S2 findings the
    /// precheck leaves to the designer's judgment, or no precheck at all;
    /// below the member's own model only where the person set none for it.
    pub(super) fn qa_tier(&mut self, squad: &str, qa: &Ticket, member: &str, role: &str, engine: Engine) -> Option<crate::tiers::QaTier> {
        let checked = self.checked_by(squad, qa.id);
        let journal = self.journal(squad)?;
        let policy = journal.design_qa_tiers();
        let run = journal.design_qa_run(qa.id);
        let round = run.as_ref().map(|r| r.round).or_else(|| checked.as_ref().and_then(|c| journal.design_qa_rounds(c.id).ok())).unwrap_or(1);
        // Where a precheck ran, the top tier only for S1/S2 findings it leaves to a designer's judgment
        // (a waiver, a send-back the gate didn't act on): what it couldn't render is a narrowed look, not
        // a harder one. With no precheck at all, a change that isn't small (copy, a label, a tweak) still
        // gets the top tier: nothing was measured for it.
        let small = checked.as_ref().is_some_and(|c| crate::tiers::small_change(&c.title));
        let prechecked = run.as_ref().is_some_and(|r| matches!(r.precheck.as_str(), "done" | "failed"));
        let flagged = run.as_ref().and_then(|r| r.report.as_deref()).and_then(|d| precheck::read(Path::new(d))).is_some_and(|r| !r.new_blockers().is_empty());
        let judgment = if prechecked { flagged } else { !small };
        let person = journal.person_models().iter().any(|m| m == member) || journal.role_models().iter().any(|r| r.role == role && r.engine == engine && r.model.is_some());
        // A light per-ticket session: the cheap tier, the top one from a third round on.
        if run.as_ref().is_some_and(|r| r.lite) {
            return Some(crate::tiers::QaTier { tier: crate::design_qa::lite_tier(policy, round), below_floor: !person });
        }
        let redesign = checked.as_ref().is_some_and(|c| crate::design_qa::redesign(&c.title, &c.body));
        let (tier, _) = crate::design_qa::tier_for(policy, crate::design_qa::TierAsk { round, redesign, judgment });
        Some(crate::tiers::QaTier { tier, below_floor: !person })
    }

    /// A daily review's tier (the cheap one) or an audit's (the top one),
    /// below the member's own model only where the person set none for it.
    pub(super) fn batch_tier(&mut self, squad: &str, kind: &str, member: &str, role: &str, engine: Engine) -> Option<crate::tiers::QaTier> {
        let journal = self.journal(squad)?;
        let policy = journal.design_qa_tiers();
        let person = journal.person_models().iter().any(|m| m == member) || journal.role_models().iter().any(|r| r.role == role && r.engine == engine && r.model.is_some());
        let tier = if kind == super::design_review::AUDIT { policy.top } else { policy.cheap };
        Some(crate::tiers::QaTier { tier, below_floor: !person })
    }

    /// The designer's verdict on design QA ticket `qa`: the ticket it checked
    /// goes on to the person's review, or back to its engineer. The ticket as
    /// it now is, and who had it.
    pub(super) fn design_qa_checked(&mut self, squad: &str, qa: &Ticket, designer: &str, verdict: &Verdict, escalate: bool) -> Option<(Ticket, String)> {
        let mut checked = self.waiting_on(squad, qa.id)?;
        checked.design_qa = None;
        let engineer = checked.assignee.clone().or_else(|| checked.owner.clone()).unwrap_or_default();
        let findings = verdict.findings.trim();
        // Held on itself, the change has no design QA ticket to name.
        let on = if qa.id == checked.id { String::new() } else { format!(" (T-{})", qa.id) };
        let event = if verdict.passed {
            let summary = checked.note.take().unwrap_or_default();
            checked.note = Some(format!("{summary}\n\nDesign QA by {designer}{on}: passed.\n{findings}").trim().to_owned());
            // The designer's pictures follow the engineer's.
            for image in &qa.images {
                if !checked.images.contains(image) && checked.images.len() < crate::visuals::HAND_IN {
                    checked.images.push(image.clone());
                }
            }
            "design QA passed"
        } else {
            // Back to its engineer, on the same branch and worktree, like the person's send-back.
            checked.owner = checked.assignee.take().or(checked.owner);
            checked.status = TicketStatus::Todo;
            checked.note = Some(format!("Design QA by {designer}{on}: {findings}"));
            checked.images.clear();
            "design QA: changes"
        };
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&checked);
            let _ = journal.ticket_event(checked.id, designer, event, findings);
            let _ = journal.set_design_qa_verdict(qa.id, if verdict.passed { "pass" } else { "changes" }, designer);
        }
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: checked.clone() });
        // Blockers are differences from the spec: the work isn't right yet.
        if !verdict.passed && escalate {
            self.after_send_back(squad, checked.id, designer, equipe_protocol::tiers::SendBackReason::Correctness);
        }
        Some((checked, engineer))
    }

    /// The person takes `ticket` over while its design QA is open — accepting,
    /// sending back or cancelling it: the design QA isn't needed any more.
    pub(super) fn drop_design_qa(&mut self, squad: &str, ticket: TicketId) {
        let Some(mut found) = self.journal(squad).and_then(|j| j.ticket(ticket).ok().flatten()) else {
            return;
        };
        let Some(qa) = found.design_qa.take() else {
            return;
        };
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&found);
            let _ = journal.set_design_qa_verdict(qa, "cancelled", "you");
        }
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: found });
        // Held on itself for its precheck: no design QA ticket to close, and
        // the change, sent back meanwhile, isn't held off the board by it.
        if qa == ticket {
            self.prechecks.remove(&(squad.to_owned(), ticket));
            return;
        }
        let Some(open) = self.journal(squad).and_then(|j| j.ticket(qa).ok().flatten()).filter(|t| !t.status.is_closed()) else {
            return;
        };
        match self.worker_on(squad, qa) {
            Some(id) => {
                self.stop_turn(id);
                self.end_work(id, WorkEnd::Cancelled);
            }
            None => self.close_ticket(squad, open, TicketStatus::Cancelled),
        }
    }

    /// Design QA ticket `qa` closed by the person, not by a verdict: the
    /// ticket it held goes on to their review.
    pub(super) fn design_qa_closed(&mut self, squad: &str, qa: TicketId) {
        let Some(mut held) = self.checked_by(squad, qa) else {
            return;
        };
        held.design_qa = None;
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&held);
            let _ = journal.set_design_qa_verdict(qa, "cancelled", "you");
        }
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: held });
    }

    /// `design_qa`: a designer's verdict on the design QA ticket it is on,
    /// handed in when the turn ends.
    pub(super) fn finish_design_qa(&mut self, squad: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let on = self.sessions.get(&id).and_then(|s| s.work.as_ref()).map(|w| w.ticket).ok_or("You're not on a ticket — there is no design QA to finish.")?;
        let checked = self.checked_by(squad, on).ok_or_else(|| format!("T-{on} isn't a design QA ticket: hand it in with submit_for_review."))?;
        let passed = match board::arg_str(args, "verdict") {
            Some("pass") => true,
            Some("changes") => false,
            _ => return Err("Give a verdict: pass, or changes.".into()),
        };
        let summary = board::arg_str(args, "findings").ok_or("Say what you looked at and how it stands.")?.to_owned();
        let (cwd, mut roots) = self.picture_places(squad, id);
        // The precheck's pictures count as the designer's own.
        roots.push(precheck_dir(&self.config.home, squad, on));
        let shown = crate::visuals::resolve_all(&board::arg_strs(args, "images"), &cwd, &roots, crate::visuals::HAND_IN)
            .map_err(|e| format!("{e}. Save the pictures in your worktree and give their paths, then finish again."))?;
        crate::visuals::thumbs::warm(&self.config.home, shown.clone());
        // Its own look, not the engineer's screenshots handed on.
        let own = shown.iter().filter(|s| !checked.images.contains(s)).count();
        let proof = crate::design_qa::Proof::read(args, own);
        if let Some(why) = if passed { proof.refuses_pass() } else { proof.refuses_changes() } {
            return Err(why);
        }
        let findings = proof.report(&summary);
        let work = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()).ok_or("You're not on a ticket.")?;
        work.handed_in = Some(findings.clone());
        work.shown = shown;
        work.waiting = None;
        work.verdict = Some(Verdict { passed, findings });
        Ok(if passed {
            format!("T-{} passes design QA when this turn ends and goes to the person's review. End your turn now with a short summary.", checked.id)
        } else {
            format!("T-{} goes back to its engineer with your findings when this turn ends. End your turn now with a short summary.", checked.id)
        })
    }
}

/// A precheck's folder moved from `from` to `to`, the paths its report
/// names moved with it.
fn move_precheck(from: &Path, to: &Path) {
    if !from.is_dir() {
        return;
    }
    let _ = std::fs::remove_dir_all(to);
    if let Some(parent) = to.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::rename(from, to).is_err() {
        return;
    }
    let (a, b) = (from.to_string_lossy().into_owned(), to.to_string_lossy().into_owned());
    let quoted = |p: &str| serde_json::to_string(p).map(|q| q.trim_matches('"').to_owned()).unwrap_or_default();
    for name in [precheck::REPORT_JSON, precheck::REPORT_MD] {
        let path = to.join(name);
        let Ok(mut text) = std::fs::read_to_string(&path) else { continue };
        for sep in ["/", "\\"] {
            text = text.replace(&format!("{a}{sep}"), &format!("{b}{sep}"));
            text = text.replace(&format!("{}{}", quoted(&a), quoted(sep)), &format!("{}{}", quoted(&b), quoted(sep)));
        }
        let _ = std::fs::write(&path, text);
    }
}

/// `equipe design precheck` for the ticket `id` checked out at `worktree`
/// (of the repository `project`), writing into `dir`; what it says goes to
/// `dir/precheck.log`. `Ok` once its report is there.
async fn run_precheck(bin: &Path, worktree: &Path, project: &Path, dir: &Path, id: TicketId) -> Result<(), String> {
    let (repo, parent) = (project.to_path_buf(), dir.parent().map(Path::to_path_buf));
    let main = tokio::task::spawn_blocking(move || {
        // Old reports go after a month.
        if let Some(parent) = parent {
            precheck::prune(&parent, PRECHECK_KEPT_DAYS);
        }
        worktree::main_branch(&repo).ok()
    })
    .await
    .ok()
    .flatten();
    precheck_process(bin, worktree, dir, Some(id), main.as_deref()).await
}

/// `equipe design precheck` on `worktree` against `base`, writing into
/// `dir` (emptied first); what it says goes to `dir/precheck.log`. `Ok`
/// once its report is there; stopped, with what it started, after
/// [`PRECHECK_LIMIT`].
pub(super) async fn precheck_process(bin: &Path, worktree: &Path, dir: &Path, id: Option<TicketId>, main: Option<&str>) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let log = std::fs::File::create(dir.join("precheck.log")).map_err(|e| e.to_string())?;
    let mut command = tokio::process::Command::new(bin);
    command.args(["design", "precheck", "--worktree"]).arg(worktree).arg("--out").arg(dir);
    if let Some(id) = id {
        command.args(["--ticket", &id.to_string()]);
    }
    if let Some(main) = main {
        command.args(["--base", main]);
    }
    command.current_dir(worktree).stdin(std::process::Stdio::null()).stdout(log.try_clone().map_err(|e| e.to_string())?).stderr(log).kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|e| format!("{}: {e}", bin.display()))?;
    let pid = child.id();
    match tokio::time::timeout(PRECHECK_LIMIT, child.wait()).await {
        Ok(Ok(status)) if status.success() => {}
        Ok(Ok(status)) => return Err(format!("it stopped ({status}){}; see {}", precheck::log_head(&dir.join("precheck.log")), dir.join("precheck.log").display())),
        Ok(Err(err)) => return Err(err.to_string()),
        Err(_) => {
            if let Some(pid) = pid {
                crate::group::end(pid, crate::group::Signal::Terminate);
                tokio::time::sleep(Duration::from_secs(2)).await;
                crate::group::end(pid, crate::group::Signal::Kill);
            }
            return Err(format!("it {PRECHECK_TIMED_OUT} {} minutes and was stopped", PRECHECK_LIMIT.as_secs() / 60));
        }
    }
    if dir.join(precheck::REPORT_JSON).is_file() { Ok(()) } else { Err(format!("it wrote no report{}", precheck::log_head(&dir.join("precheck.log")))) }
}

/// What the route made of a precheck, in a sentence: the report's headline
/// in `risk_batch`.
fn route_headline(r: &RouteCall, call: &crate::design_qa::GateCall) -> String {
    let why = call.reasons.join(" ");
    match r.route {
        Route::Refused => format!("**Refused back to the engineer by its self-check, without a designer turn.** {why}"),
        Route::Batch => format!("**Low risk: on to review without a designer turn; its screens go into the daily design review.** {why}"),
        Route::PerTicket => {
            let risks: Vec<&str> = r.risks.iter().map(|k| k.said()).collect();
            format!("**A designer checks this change now: {}.** {why}", risks.join("; "))
        }
    }
}

/// What a designer on design QA ticket `qa` is told about the ticket it
/// checks — with its precheck's report, when there is one: then it starts
/// from what the report flags instead of shooting every condition again.
pub(super) fn qa_prompt(qa: TicketId, checked: &Ticket, precheck: Option<&PrecheckBrief>) -> String {
    let engineer = checked.assignee.as_deref().or(checked.owner.as_deref()).unwrap_or("its engineer");
    let mut out = format!(
        "\nThis is design QA of T-{} ({}), which {engineer} handed in with changed screens. It waits in review: \
the person is told it is ready only once you pass it.",
        checked.id, checked.title
    );
    if let Some(branch) = &checked.branch {
        out.push_str(&format!(" Its work is on branch {branch}"));
        match &checked.worktree {
            Some(path) => out.push_str(&format!(", checked out at {} — read it there; never change it.", path.display())),
            None => out.push('.'),
        }
    }
    out.push_str(&format!("\n\nWhat T-{} asked for:\n{}\n", checked.id, checked.body.trim()));
    if let Some(note) = &checked.note {
        out.push_str(&format!("\n{engineer}'s hand-in:\n{}\n", note.trim()));
    }
    if !checked.images.is_empty() {
        out.push_str(&format!("\nIts screenshots (look at each with Read):\n- {}\n", checked.images.join("\n- ")));
    }
    let blockers = "Two kinds of finding: blockers (S1/S2) — function or data broken, layout broken at a major width, a WCAG 2.2 A/AA \
failure such as content or a control lost at 200% text or at 320px reflow, or a clear difference from the spec — which send it back; \
and polish (S3/S4), what keeps it from looking finished — rhythm, alignment, density, hierarchy — never left out, even when it \
passes, with where and how to fix, and never a reason to send it back. Then the quality bar (your `design-references` skill): \
the changed screens beside the references of their patterns in the project's `design/references/`, at the same width, and say in \
findings whether they stand in their tier — a gap the build opened against its spec is a blocker, one the spec itself has is \
polish and one design ticket to raise it. Run your `ui-review` skill's checklists on them too: the kind checklist, and \
the checklist of each part they change (forms, tables and charts, motion, copy).";
    let budget = format!(
        " Your budget for this check is about {QA_TURN_CALLS} tool calls and {} minutes: spend it on what only you can judge, and \
when it runs out finish with design_qa on what you have, saying in findings what you couldn't look at.",
        QA_TURN_SECS / 60
    );
    if let Some(n) = precheck.and_then(|b| b.narrowed.as_ref()) {
        let states = if n.states.is_empty() { "the pages its changed files show".to_owned() } else { n.states.join(", ") };
        let files = if n.files.is_empty() { String::new() } else { format!(" (changed UI files no screen lists: {})", n.files.join(", ")) };
        out.push_str(&format!(
            "\nequipe's precheck couldn't render this change after {PRECHECK_TRIES} tries on a loaded machine, so your look is narrowed: \
don't shoot every width, scheme and text size. Start its preview from its worktree as its project starts one (another port than \
anyone's, change no file there) and render only the main width, light — `equipe design render http://localhost:<port>/<page> \
--widths {}` — of the states it touched: {states}{files}. Add one more condition only where a state needs it to be judged.",
            n.width
        ));
    }
    match precheck.filter(|b| !b.summary.trim().is_empty()) {
        Some(brief) => out.push_str(&format!(
            "\nequipe's precheck already rendered the screens this change touches — each width, light and dark, at normal size, \
with text at 200% and zoomed to 200% — measured them (text cut off, text over text, sideways scroll, axe-core's WCAG rules) and \
compared each with the UI map's picture at the base. Its report is below; the full one and every picture are in {dir}.\n\n{summary}\n\
Start from it, not from scratch: look at its flagged pictures and the changed states (Read each), and don't shoot every condition \
again. What it measured is settled — don't re-measure it; a finding it marks as there before isn't this change's. Spend your look \
on what it can't judge: whether the changed screens match the spec or mockup they were built from and what the ticket asked, their \
states (empty, loading, error, long text, hover, focus), and whether a flagged finding truly costs the person something. Render \
yourself only what it couldn't — pictures that failed, states it left out, changed UI files no screen lists — or a state you need \
to see: start the preview from its worktree as its project starts one (another port than anyone's, change no file there) and \
`equipe design render http://localhost:<port>/<page> --widths 390,768,1280 --dark`. Its pictures count as your own in `images`. \
{blockers} Put the changed screens beside their spec at the same width; leave any comparison sheets under `outputs/T-{qa} …/`. \
Then finish with design_qa, not submit_for_review: a pass needs at least two pictures in images (the precheck's or yours), \
`compared`, what you `checked`, and no blocker.{reuse}{budget}",
            dir = brief.dir.display(),
            summary = brief.summary.trim(),
            reuse = if brief.pictures.is_empty() {
                String::new()
            } else {
                format!(" Its pictures to reuse — Read these instead of shooting them again, and name them in `images`:\n- {}\n", brief.pictures.join("\n- "))
            },
        )),
        // Narrowed: only what was said above, put beside its spec.
        None if precheck.is_some_and(|b| b.narrowed.is_some()) => out.push_str(&format!(
            "\nPut each picture beside the spec or mockup it was built from, at the same width, and judge whether it matches \
what the ticket asked and its states hold up. {blockers} Then finish with design_qa, not submit_for_review: a pass needs at \
least two of your own pictures in images, `compared`, what you `checked`, and no blocker.{budget}"
        )),
        None => out.push_str(&format!(
            "\nCheck it like a company designer, on the built screens themselves — the engineer's screenshots are where to \
start, never enough to pass. See it running: start its preview from its worktree as its project starts one (its \
README or package.json says how; another port than anyone's, and change no file there), then shoot it yourself — \
`equipe design render http://localhost:<port>/<page> --widths 390,768,1280 --dark` — every page it changed. Put \
each beside the spec or mockup it was built from, at the same width, and look: layout, spacing on the scale, type, \
colour, every state (empty, loading, error, long text, hover, focus), light and dark, and accessibility (contrast \
computed, not guessed; focus; touch size). Where you truly can't run it, ask {engineer} with comment for what you \
need — don't pass without it. {blockers} Leave your comparison sheets under `outputs/T-{qa} …/`. Then finish with \
design_qa, not submit_for_review: a pass needs at least two of your own pictures in images, `compared`, what you \
`checked`, and no blocker.{budget}"
        )),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Narrowed, PrecheckBrief, QA_TURN_CALLS, QA_TURN_SECS, move_precheck, outside_design, over_budget, qa_prompt, retry_delay, reusable_pictures, unseen_under_load};
    use crate::design::precheck::{self, Report, Shot};
    use equipe_protocol::{Ticket, TicketKind, TicketStatus};
    use std::time::Duration;

    fn shot(key: &str, condition: &str, picture: Option<&str>, error: bool) -> Shot {
        Shot { key: key.into(), condition: condition.into(), picture: picture.map(str::to_owned), error: error.then(|| "timed out".into()), ..Shot::default() }
    }

    #[test]
    fn a_precheck_that_saw_nothing_on_a_loaded_machine_runs_again_later_waiting_longer_each_time() {
        let failed = Report { touched: vec!["app/home".into()], shots: vec![shot("app/home/default", "390.light.normal", None, true), shot("app/home/default", "1280.light.normal", None, true)], ..Report::default() };
        // Nothing rendered while the machine was overloaded, or stopped at its limit: again later.
        assert!(unseen_under_load(Some(&failed), &Ok(()), true));
        assert!(unseen_under_load(None, &Err("it ran past 25 minutes and was stopped".into()), false));
        // The same on a calm machine says something about the change or its setup: no retry.
        assert!(!unseen_under_load(Some(&failed), &Ok(()), false));
        assert!(!unseen_under_load(None, &Err("it stopped (exit 1)".into()), false));
        // Rendered fine under load: nothing to retry.
        let seen = Report { touched: vec!["app/home".into()], shots: vec![shot("app/home/default", "390.light.normal", Some("/p/a.png"), false)], ..Report::default() };
        assert!(!unseen_under_load(Some(&seen), &Ok(()), true));
        assert_eq!((retry_delay(1), retry_delay(2)), (Duration::from_secs(5 * 60), Duration::from_secs(10 * 60)));
        assert_eq!(retry_delay(9), Duration::from_secs(60 * 60), "never more than an hour");
    }

    #[test]
    fn after_its_tries_a_designer_looks_at_the_main_width_and_the_touched_states_only() {
        let report = Report { touched: vec!["app/home".into()], unmapped: vec!["src/Hero.tsx".into()], conditions: vec!["390.light.normal".into(), "1280.dark.normal".into()], ..Report::default() };
        assert_eq!(Narrowed::of(Some(&report)), Narrowed { width: 1280, states: vec!["app/home".into()], files: vec!["src/Hero.tsx".into()] });
        assert_eq!(Narrowed::of(None).width, 1280);
    }

    #[test]
    fn the_precheck_pictures_of_flagged_and_changed_states_are_reused() {
        let changed = Shot { diff: Some(precheck::Diff { changed: true, share: 0.2, ..precheck::Diff::default() }), ..shot("app/home/default", "390.light.normal", Some("/p/home.png"), false) };
        let same = Shot { diff: Some(precheck::Diff::default()), ..shot("app/about/default", "390.light.normal", Some("/p/about.png"), false) };
        let report = Report { shots: vec![changed, same], ..Report::default() };
        assert_eq!(reusable_pictures(&report, 8), ["/p/home.png"]);
    }

    #[test]
    fn a_design_check_past_its_calls_or_its_time_is_over_budget() {
        let cap = (QA_TURN_CALLS, QA_TURN_SECS);
        assert!(!over_budget(QA_TURN_CALLS - 1, QA_TURN_SECS - 1, cap));
        assert!(over_budget(QA_TURN_CALLS, 10, cap) && over_budget(3, QA_TURN_SECS, cap));
    }

    #[test]
    fn a_precheck_moved_to_its_design_qa_ticket_takes_the_paths_its_report_names_along() {
        let root = std::env::temp_dir().join(format!("equipe-move-precheck-{}", std::process::id()));
        let (from, to) = (root.join("T-7"), root.join("T-12"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&from).unwrap();
        let pic = format!("{}/shots/home.png", from.display());
        let other = format!("{}0/shots/x.png", from.display());
        std::fs::write(from.join(precheck::REPORT_JSON), serde_json::json!({ "pictures": [pic, other] }).to_string()).unwrap();
        std::fs::write(from.join(precheck::REPORT_MD), format!("- {pic}\n")).unwrap();
        move_precheck(&from, &to);
        assert!(!from.exists() && to.is_dir());
        let json = std::fs::read_to_string(to.join(precheck::REPORT_JSON)).unwrap();
        assert!(json.contains(&format!("{}/shots/home.png", to.display())) && json.contains(&other), "only its own folder's paths move: {json}");
        assert!(std::fs::read_to_string(to.join(precheck::REPORT_MD)).unwrap().contains(&format!("{}/shots/home.png", to.display())));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_designer_with_a_precheck_report_starts_from_it_instead_of_shooting_everything() {
        let checked = Ticket {
            id: 4,
            title: "Add a pricing page".into(),
            body: "Build the pricing page from the mockup.".into(),
            status: TicketStatus::Review,
            kind: TicketKind::Code,
            owner: Some("frontend".into()),
            assignee: Some("frontend".into()),
            project: None,
            depends_on: vec![],
            branch: Some("equipe/frontend-t4".into()),
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            landing: false,
            design_qa: Some(9),
            plan: None,
            model: None,
            flow: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            label: None, remaining: vec![],
            images: vec![],
            updated_at: None,
            started_at: None,
            app: None,
            priority: Default::default(),
            rank: 0,
        };
        let plain = qa_prompt(9, &checked, None);
        assert!(plain.contains("shoot it yourself") && plain.contains("S1/S2") && !plain.contains("precheck"), "{plain}");
        let brief = PrecheckBrief {
            dir: "/h/squads/acme/design-qa/T-9".into(),
            summary: "# Design QA precheck\n\n- Changed states: pricing/plans/default".into(),
            pictures: vec!["/h/squads/acme/design-qa/T-9/shots/plans.png".into()],
            narrowed: None,
        };
        let given = qa_prompt(9, &checked, Some(&brief));
        assert!(given.contains("/h/squads/acme/design-qa/T-9") && given.contains("Changed states: pricing/plans/default"), "{given}");
        assert!(given.contains("don't shoot every condition") && given.contains("count as your own"), "{given}");
        assert!(given.contains("instead of shooting them again") && given.contains("T-9/shots/plans.png"), "its pictures are reused: {given}");
        for text in [&plain, &given] {
            assert!(text.contains(&format!("about {QA_TURN_CALLS} tool calls")), "a budget: {text}");
        }
        // The precheck couldn't render after its tries: a narrowed look, not the whole matrix.
        let narrowed = PrecheckBrief { narrowed: Some(super::Narrowed { width: 1280, states: vec!["pricing/plans".into()], files: vec![] }), ..PrecheckBrief::default() };
        let narrow = qa_prompt(9, &checked, Some(&narrowed));
        assert!(narrow.contains("--widths 1280`") && narrow.contains("pricing/plans") && narrow.contains("narrowed"), "{narrow}");
        assert!(!narrow.contains("390,768,1280 --dark"), "not every width and scheme: {narrow}");
        assert!(given.contains("never a reason to send it back"), "polish never blocks");
        for text in [&plain, &given] {
            assert!(text.contains("`design-references`") && text.contains("stand in their tier") && text.contains("a gap the build opened against its spec is a blocker"), "the quality bar: {text}");
            assert!(text.contains("`ui-review` skill's checklists"), "the review checklists: {text}");
        }
    }

    #[test]
    fn only_the_design_folder_is_a_designers_to_change() {
        let files = ["design/tokens.json", "design/assets/logo/mark.svg", "designs.md", "design", "src/app.css"].map(str::to_owned);
        assert_eq!(outside_design(&files), ["`designs.md`", "`design`", "`src/app.css`"]);
        let many: Vec<String> = (0..10).map(|n| format!("src/{n}.rs")).collect();
        assert_eq!(outside_design(&many).last().map(String::as_str), Some("and 2 more"));
    }
}
