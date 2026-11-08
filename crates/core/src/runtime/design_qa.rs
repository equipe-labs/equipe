//! Design QA: a code ticket that changes what users see goes past a designer
//! before the person reviews it. Its engineer says so when handing in
//! (`changes_screens`, with screenshots); equipe puts a design QA ticket on
//! the board for the squad's designer, and the hand-in waits in review,
//! marked, until the designer passes it (the person is told then) or sends it
//! back to its engineer with what to change. A squad with no designer skips
//! the step. The person stays the gate: accepting, sending back or cancelling
//! the ticket meanwhile drops its design QA.

use super::*;

/// The role that checks screens.
pub(super) const DESIGNER: &str = "designer";

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
            merged: None,
            off_main: None,
            design_qa: None, plan: None,
            created_by: "equipe".to_owned(),
            note: None,
            asks: Vec::new(),
            remaining: Vec::new(),
            images: Vec::new(),
            updated_at: None,
            priority: ticket.priority,
            rank: 0,
        };
        let journal = self.journal(squad)?;
        let qa_id = journal.create_ticket(&qa).ok()?;
        let _ = journal.ticket_event(qa_id, "equipe", "created", &format!("design QA of T-{}", ticket.id));
        ticket.design_qa = Some(qa_id);
        let made = journal.ticket(qa_id).ok().flatten().unwrap_or(Ticket { id: qa_id, ..qa });
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: made });
        Some(qa_id)
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
        }
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: checked.clone() });
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
        let (cwd, roots) = self.picture_places(squad, id);
        let shown = crate::visuals::resolve_all(&board::arg_strs(args, "images"), &cwd, &roots, crate::visuals::HAND_IN)
            .map_err(|e| format!("{e}. Save the pictures in your worktree and give their paths, then finish again."))?;
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

/// What a designer on design QA ticket `qa` is told about the ticket it checks.
pub(super) fn qa_prompt(qa: TicketId, checked: &Ticket) -> String {
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
    out.push_str(&format!(
        "\nCheck it like a company designer, on the built screens themselves — the engineer's screenshots are where to \
start, never enough to pass. See it running: start its preview from its worktree as its project starts one (its \
README or package.json says how; another port than anyone's, and change no file there), then shoot it yourself — \
`equipe design render http://localhost:<port>/<page> --widths 390,768,1280 --dark` — every page it changed. Put \
each beside the spec or mockup it was built from, at the same width, and look: layout, spacing on the scale, type, \
colour, every state (empty, loading, error, long text, hover, focus), light and dark, and accessibility (contrast \
computed, not guessed; focus; touch size). Where you truly can't run it, ask {engineer} with comment for what you \
need — don't pass without it. Two kinds of finding: blockers, differences from the spec, the tokens or accessibility, \
which send it back; and polish, what keeps it from looking finished — rhythm, alignment, density, hierarchy, the \
first impression at a glance — never left out, even when it passes, with where and how to fix. Leave your comparison \
sheets under `outputs/T-{qa} …/`. Then finish with design_qa, not submit_for_review: a pass needs at least two of \
your own pictures in images, `compared`, what you `checked`, and no blocker."
    ));
    out
}
