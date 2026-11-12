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

use std::time::Duration;

use super::*;
use crate::design::precheck;
use crate::design_qa::{Decision, GateMode};

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

/// Where design QA ticket `qa`'s precheck keeps its report and pictures:
/// beside the squad's journal, in no repository.
pub(super) fn precheck_dir(home: &Path, squad: &str, qa: TicketId) -> PathBuf {
    journal::squad_dir(home, squad).join("design-qa").join(format!("T-{qa}"))
}

/// What a designer on a design QA ticket is given of its precheck.
#[derive(Debug, Clone)]
pub(super) struct PrecheckBrief {
    pub dir: PathBuf,
    pub summary: String,
}

/// How a design QA hand-in ends: the designer's word on the screens.
#[derive(Clone, Debug)]
pub(super) struct Verdict {
    pub passed: bool,
    pub findings: String,
}

impl Runtime {
    /// Whether `squad` has a designer to check screens.
    pub(super) fn has_designer(&self, squad: &str) -> bool {
        self.team(squad).iter().any(|(_, _, role)| role == DESIGNER)
    }

    /// The ticket design QA ticket `qa` checks, while it waits on it.
    pub(super) fn checked_by(&mut self, squad: &str, qa: TicketId) -> Option<Ticket> {
        self.tickets(squad).into_iter().find(|t| t.design_qa == Some(qa))
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
                if !missing.is_empty() {
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
    pub(super) fn ask_design_qa(&mut self, squad: &str, ticket: &mut Ticket, engineer: &str) -> Option<TicketId> {
        let qa = Ticket {
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
            design_qa: None, plan: None, model: None,
            created_by: "equipe".to_owned(),
            note: None,
            asks: Vec::new(),
            remaining: Vec::new(),
            images: Vec::new(),
            updated_at: None,
            priority: ticket.priority,
            rank: 0,
        };
        let can_run = self.config.equipe_binary.is_some();
        let journal = self.journal(squad)?;
        let qa_id = journal.create_ticket(&qa).ok()?;
        let _ = journal.ticket_event(qa_id, "equipe", "created", &format!("design QA of T-{}", ticket.id));
        ticket.design_qa = Some(qa_id);
        let made = journal.ticket(qa_id).ok().flatten().unwrap_or(Ticket { id: qa_id, ..qa });
        // Its precheck first, unless the squad has it off or there is nothing to run it on.
        let gate = journal.design_qa_gate();
        let round = journal.design_qa_rounds(ticket.id).unwrap_or(1).max(1);
        let runs = gate != GateMode::Off && can_run && ticket.worktree.as_deref().is_some_and(Path::exists) && ticket.project.is_some();
        if let Err(err) = journal.start_design_qa_run(qa_id, ticket.id, round, gate.code(), if runs { "running" } else { "skipped" }) {
            crate::daemon_log!("equipe: {squad}: T-{qa_id}'s design QA run not kept: {err}");
        }
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: made });
        if runs {
            self.start_precheck(squad, qa_id, ticket);
        }
        Some(qa_id)
    }

    /// Whether design QA ticket `ticket` waits on its precheck: nobody takes it meanwhile.
    pub(super) fn prechecking(&self, squad: &str, ticket: TicketId) -> bool {
        self.prechecks.keys().any(|(q, t)| q == squad && *t == ticket)
    }

    /// `equipe design precheck` on `checked`'s worktree, off the loop, for
    /// design QA ticket `qa`, held off the board until it ends
    /// ([`Internal::Prechecked`]) — stopped, with what it started, after
    /// [`PRECHECK_LIMIT`].
    fn start_precheck(&mut self, squad: &str, qa: TicketId, checked: &Ticket) {
        let (Some(bin), Some(worktree), Some(project)) = (self.config.equipe_binary.clone(), checked.worktree.clone(), checked.project.clone()) else { return };
        let dir = precheck_dir(&self.config.home, squad, qa);
        self.prechecks.insert((squad.to_owned(), qa), std::time::Instant::now());
        let (internal, squad, id) = (self.internal.clone(), squad.to_owned(), checked.id);
        crate::daemon_log!("equipe: {squad}: prechecking T-{id}'s screens for design QA T-{qa}");
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            let result = run_precheck(&bin, &worktree, &project, &dir, id).await;
            let _ = internal.send(Internal::Prechecked { squad, qa, result, secs: started.elapsed().as_secs() });
        });
    }

    /// A precheck ended: its report kept with the run and on the design QA
    /// ticket's record, and — when the squad's gate is on and the ticket
    /// still waits — acted on; else the designer takes it, with the report.
    pub(super) async fn on_prechecked(&mut self, squad: &str, qa: TicketId, result: Result<(), String>, secs: u64) {
        self.prechecks.remove(&(squad.to_owned(), qa));
        let dir = precheck_dir(&self.config.home, squad, qa);
        let report = match &result {
            Ok(()) => precheck::read(&dir),
            Err(why) => {
                crate::daemon_log!("equipe: {squad}: design QA T-{qa}'s precheck failed: {why}");
                None
            }
        };
        let call = crate::design_qa::gate(report.as_ref());
        let Some(journal) = self.journal(squad) else { return };
        let mut run = journal.design_qa_run(qa).unwrap_or(journal::DesignQaRun { qa, ..Default::default() });
        let mode = GateMode::parse(&run.gate).unwrap_or_default();
        let open = journal.ticket(qa).ok().flatten().filter(|t| t.status == TicketStatus::Todo);
        let checked = self.checked_by(squad, qa);
        let act = mode == GateMode::On && call.decision != Decision::Designer && open.is_some() && checked.is_some() && self.worker_on(squad, qa).is_none();
        let headline = call.headline(act);
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
        let said = match (&report, &result) {
            (Some(_), _) => format!("{headline} Report: {}", dir.join(precheck::REPORT_MD).display()),
            (None, Err(why)) => format!("The precheck couldn't run ({why}); the designer checks it as before."),
            (None, Ok(())) => "The precheck wrote no report; the designer checks it as before.".to_owned(),
        };
        let _ = journal.ticket_event(qa, "equipe", "precheck", &said);
        crate::daemon_log!("equipe: {squad}: design QA T-{qa} precheck: {} ({}, {secs}s)", call.decision.code(), if act { "acted" } else { mode.code() });
        let (Some(mut qa_ticket), Some(_)) = (open, checked) else { return };
        // What it flags, shown on the design QA ticket.
        if !flagged.is_empty() {
            qa_ticket.images = flagged.iter().take(crate::visuals::HAND_IN).cloned().collect();
            if let Some(journal) = self.journal(squad) {
                let _ = journal.save_ticket(&qa_ticket);
            }
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: qa_ticket.clone() });
        }
        if act {
            let findings = match (call.decision, &report) {
                (Decision::SendBack, Some(r)) => {
                    let lines: Vec<String> = r.new_blockers().iter().take(8).map(|f| format!("- {} on {} [{}]: {} {}", f.rule, f.key, f.conditions.join(", "), f.selector, f.evidence)).collect();
                    format!("{headline}\n{}\nReport: {}", lines.join("\n"), dir.join(precheck::REPORT_MD).display())
                }
                _ => format!("{headline}\nReport: {}", dir.join(precheck::REPORT_MD).display()),
            };
            self.precheck_verdict(squad, qa_ticket, Verdict { passed: call.decision == Decision::AutoPass, findings });
        }
        self.dispatch(squad).await;
    }

    /// The gate's verdict on design QA ticket `qa`, without a designer turn:
    /// it closes, and the ticket it checked goes on as a designer's verdict
    /// would send it — to the person's review, or back to its engineer.
    fn precheck_verdict(&mut self, squad: &str, mut qa: Ticket, verdict: Verdict) {
        qa.status = TicketStatus::Done;
        qa.note = Some(verdict.findings.clone());
        let event = if verdict.passed { "design QA passed" } else { "design QA: changes" };
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&qa);
            let _ = journal.ticket_event(qa.id, PRECHECK_BY, event, &verdict.findings);
        }
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: qa.clone() });
        let Some((checked, engineer)) = self.design_qa_checked(squad, &qa, PRECHECK_BY, &verdict) else { return };
        let (line, shown) = if verdict.passed {
            (
                format!("equipe's precheck passed the design QA of T-{} (T-{}): no screen changed and nothing new was found; it is ready for the person's review", checked.id, qa.id),
                say!(self, "news.design_qa_precheck_passed", id = checked.id, title = checked.title),
            )
        } else {
            (
                format!("equipe's precheck sent T-{} back to {engineer} without a designer turn (design QA T-{}): {}", checked.id, qa.id, verdict.findings.lines().next().unwrap_or_default()),
                say!(self, "news.design_qa_precheck_changes", id = checked.id, title = checked.title, to = engineer),
            )
        };
        self.brief_pm(squad, line, Some(shown));
        // Where the PM accepts, work ready for review is its to act on now.
        if verdict.passed
            && checked.ready_for_review()
            && self.journal(squad).is_some_and(|j| j.pm_accepts())
            && let Some(pm) = self.pm_of(squad)
        {
            let pm_name = self.sessions.get(&pm).and_then(|s| s.name().map(str::to_owned)).unwrap_or_default();
            if crate::accept::own_code(&checked, &pm_name).is_none() {
                let prompt = format!(
                    "[equipe: T-{} passed design QA by equipe's precheck and is ready for you. Check it and accept it with `accept_work`, send it back with `comment` and `review: changes`, or put it to the person with `propose_accept`.]",
                    checked.id
                );
                let _ = self.internal.send(Internal::Wake { session: pm, prompt });
            }
        }
    }

    /// What the designer on design QA ticket `qa` is given of its precheck:
    /// its report's summary, when there is one.
    pub(super) fn precheck_brief(&mut self, squad: &str, qa: TicketId) -> Option<PrecheckBrief> {
        let run = self.journal(squad)?.design_qa_run(qa)?;
        let dir = PathBuf::from(run.report?);
        let summary = std::fs::read_to_string(dir.join(precheck::REPORT_MD)).ok()?;
        Some(PrecheckBrief { dir, summary })
    }

    /// Design QA ticket `qa`'s tier as the squad's policy reads it, for
    /// `member` (of `role`, on `engine`): the top tier for a redesign, a
    /// second round or more, or a precheck that left judgment to the
    /// designer (none at all included); below the member's own model only
    /// where the person set none for it.
    pub(super) fn qa_tier(&mut self, squad: &str, qa: &Ticket, member: &str, role: &str, engine: Engine) -> Option<crate::tiers::QaTier> {
        let checked = self.checked_by(squad, qa.id);
        let journal = self.journal(squad)?;
        let policy = journal.design_qa_tiers();
        let run = journal.design_qa_run(qa.id);
        let round = run.as_ref().map(|r| r.round).or_else(|| checked.as_ref().and_then(|c| journal.design_qa_rounds(c.id).ok())).unwrap_or(1);
        // The top tier for what leaves too little seen; what only left some states unseen stays cheap.
        let judgment = run.as_ref().is_none_or(|r| r.precheck != "done" || r.judgment.as_deref().is_some_and(crate::design_qa::Judgment::text_needs_top));
        let person = journal.person_models().iter().any(|m| m == member) || journal.role_models().iter().any(|r| r.role == role && r.engine == engine && r.model.is_some());
        let redesign = checked.as_ref().is_some_and(|c| crate::design_qa::redesign(&c.title, &c.body));
        let (tier, _) = crate::design_qa::tier_for(policy, crate::design_qa::TierAsk { round, redesign, judgment });
        Some(crate::tiers::QaTier { tier, below_floor: !person })
    }

    /// The designer's verdict on design QA ticket `qa`: the ticket it checked
    /// goes on to the person's review, or back to its engineer. The ticket as
    /// it now is, and who had it.
    pub(super) fn design_qa_checked(&mut self, squad: &str, qa: &Ticket, designer: &str, verdict: &Verdict) -> Option<(Ticket, String)> {
        let mut checked = self.checked_by(squad, qa.id)?;
        checked.design_qa = None;
        let engineer = checked.assignee.clone().or_else(|| checked.owner.clone()).unwrap_or_default();
        let findings = verdict.findings.trim();
        let event = if verdict.passed {
            let summary = checked.note.take().unwrap_or_default();
            checked.note = Some(format!("{summary}\n\nDesign QA by {designer} (T-{}): passed.\n{findings}", qa.id).trim().to_owned());
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
            checked.note = Some(format!("Design QA by {designer} (T-{}): {findings}", qa.id));
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
        if !verdict.passed {
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
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let log = std::fs::File::create(dir.join("precheck.log")).map_err(|e| e.to_string())?;
    let mut command = tokio::process::Command::new(bin);
    command.args(["design", "precheck", "--worktree"]).arg(worktree).arg("--out").arg(dir).args(["--ticket", &id.to_string()]);
    if let Some(main) = main {
        command.args(["--base", &main]);
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
            return Err(format!("it ran past {} minutes and was stopped", PRECHECK_LIMIT.as_secs() / 60));
        }
    }
    if dir.join(precheck::REPORT_JSON).is_file() { Ok(()) } else { Err(format!("it wrote no report{}", precheck::log_head(&dir.join("precheck.log")))) }
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
passes, with where and how to fix, and never a reason to send it back.";
    match precheck {
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
`compared`, what you `checked`, and no blocker.",
            dir = brief.dir.display(),
            summary = brief.summary.trim(),
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
`checked`, and no blocker."
        )),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{PrecheckBrief, outside_design, qa_prompt};
    use equipe_protocol::{Ticket, TicketKind, TicketStatus};

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
            design_qa: Some(9),
            plan: None,
            model: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            remaining: vec![],
            images: vec![],
            updated_at: None,
            priority: Default::default(),
            rank: 0,
        };
        let plain = qa_prompt(9, &checked, None);
        assert!(plain.contains("shoot it yourself") && plain.contains("S1/S2") && !plain.contains("precheck"), "{plain}");
        let brief = PrecheckBrief { dir: "/h/squads/acme/design-qa/T-9".into(), summary: "# Design QA precheck\n\n- Changed states: pricing/plans/default".into() };
        let given = qa_prompt(9, &checked, Some(&brief));
        assert!(given.contains("/h/squads/acme/design-qa/T-9") && given.contains("Changed states: pricing/plans/default"), "{given}");
        assert!(given.contains("don't shoot every condition") && given.contains("count as your own"), "{given}");
        assert!(given.contains("never a reason to send it back"), "polish never blocks");
    }

    #[test]
    fn only_the_design_folder_is_a_designers_to_change() {
        let files = ["design/tokens.json", "design/assets/logo/mark.svg", "designs.md", "design", "src/app.css"].map(str::to_owned);
        assert_eq!(outside_design(&files), ["`designs.md`", "`design`", "`src/app.css`"]);
        let many: Vec<String> = (0..10).map(|n| format!("src/{n}.rs")).collect();
        assert_eq!(outside_design(&many).last().map(String::as_str), Some("and 2 more"));
    }
}
