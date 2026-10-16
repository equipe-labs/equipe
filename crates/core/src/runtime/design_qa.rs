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

    /// What `submit_for_review` asks of a code ticket in a squad with a
    /// designer: whether users see the change, and if so its screenshots.
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
        // A designer's own work, and what isn't code, has no one else to check it.
        if kind != TicketKind::Code || role == DESIGNER || !self.has_designer(squad) {
            return Ok(false);
        }
        match args.get("changes_screens").and_then(Value::as_bool) {
            None => Err("Say whether this changes what users see (changes_screens: true or false): the squad's designer \
checks changed screens before the person reviews. Then hand in again."
                .into()),
            Some(true) if pictures == 0 => Err("Screens changed: the designer checks them from screenshots. Save a \
screenshot of each changed screen in your worktree, name them in images, and hand in again."
                .into()),
            Some(changed) => Ok(changed),
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
            design_qa: None,
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
        let findings = board::arg_str(args, "findings").ok_or("Say what you checked and found.")?.to_owned();
        let (cwd, roots) = self.picture_places(squad, id);
        let shown = crate::visuals::resolve_all(&board::arg_strs(args, "images"), &cwd, &roots, crate::visuals::HAND_IN)
            .map_err(|e| format!("{e}. Save the pictures in your worktree and give their paths, then finish again."))?;
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
        "\nCheck it like a company designer: against the spec or mockups it was built from and the project's design \
system (tokens, components under its design/ folder) — layout, spacing, type, colour, every state (empty, loading, \
error, long text), light and dark, the widths it supports, and accessibility (contrast computed, not guessed; focus; \
touch size). Take more screenshots yourself where you can; ask {engineer} with comment when one you need is missing. \
Count only differences from the spec, the tokens and accessibility as defects, not taste — a new idea is a proposed \
ticket. Leave your comparison under `outputs/T-{qa} …/`. Then finish with design_qa, not submit_for_review: verdict \
pass, or changes with each difference numbered (where, the spec's value, what was built, how much it matters), and \
your comparison pictures in images."
    ));
    out
}
