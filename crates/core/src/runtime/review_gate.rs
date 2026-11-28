//! Independent review before a builder's work lands (`crate::review_gate`
//! has the rules): when an engineer's code change is ready, the squad's
//! security reviewer and QA each get a review ticket of their own — a fresh
//! run that never saw it built — and the change waits in review for their
//! verdicts. A pass from each lets it go on to landing as before; a
//! `changes` sends it back to its engineer with the findings. At most
//! [`ROUNDS`] rounds, and a later round only for the reviewers that sent it
//! back: then it goes on to the PM with what is still open.
//!
//! The platform's `qa_check` is watched as members run it: its last answer
//! is kept on the ticket, a builder that deployed its work can't hand it in
//! until it passes, and the PM can't accept work whose last check failed.

use super::*;
use crate::review_gate::{self, QaSeen, ROUNDS, Reviewer};

impl Runtime {
    /// Whether `squad` has a member reviewing as `reviewer`.
    fn has_reviewer(&self, squad: &str, reviewer: Reviewer) -> bool {
        self.team(squad).iter().any(|(_, _, role)| roles::canonical(role) == reviewer.role())
    }

    /// Whether `squad` works with equipe's platform (a cloud team): `qa_check` is there.
    pub(super) fn on_platform(&self, squad: &str) -> bool {
        self.mcp_servers_of(squad).iter().any(|s| s.name == crate::build_ahead::PLATFORM_SERVER)
    }

    /// The role of the member who built `ticket`.
    fn builder_role(&self, squad: &str, ticket: &Ticket) -> String {
        let who = ticket.assignee.as_deref().or(ticket.owner.as_deref()).unwrap_or_default();
        self.team(squad).into_iter().find(|(_, name, _)| name == who).map_or_else(|| board::role_of(who).to_owned(), |(_, _, role)| role)
    }

    /// `ticket` is ready to land: held while its independent reviews run,
    /// or started now when it needs them. `true` while it is held.
    pub(super) fn hold_for_review(&mut self, squad: &str, ticket: TicketId) -> bool {
        let Some(found) = self.find_ticket(squad, ticket) else { return false };
        let Some(journal) = self.journal(squad) else { return false };
        let rows = journal.independent_reviews(ticket);
        let round = rows.iter().map(|r| r.round).max().unwrap_or(0);
        let current: Vec<&crate::journal::IndependentReview> = rows.iter().filter(|r| r.round == round).collect();
        if current.iter().any(|r| r.verdict.is_none()) {
            return true;
        }
        let sent_back: Vec<Reviewer> = current.iter().filter(|r| r.verdict.as_deref() == Some("changes")).filter_map(|r| Reviewer::parse(&r.reviewer)).collect();
        if round > 0 && sent_back.is_empty() {
            return false;
        }
        if round >= ROUNDS {
            // Its rounds are spent: on to the PM with what is still open, never round and round.
            let open: Vec<String> = current.iter().filter(|r| r.verdict.as_deref() == Some("changes")).map(|r| format!("{}: {}", r.reviewer, r.findings.as_deref().unwrap_or_default().lines().next().unwrap_or_default())).collect();
            let line = format!(
                "T-{ticket}'s reviews ran their {ROUNDS} rounds; its engineer worked on what they found and it goes on without a third. What the last round raised: {}. Check those before it lands, and tell the person plainly what is left, if anything.",
                open.join("; ")
            );
            self.brief_pm(squad, line, None);
            return false;
        }
        let who: Vec<Reviewer> = if round == 0 {
            let changed = self.handed_in_files.get(&(squad.to_owned(), ticket)).cloned().unwrap_or_default();
            let role = self.builder_role(squad, &found);
            review_gate::needed(&role, found.kind, &changed, |r| self.has_reviewer(squad, r))
        } else {
            sent_back
        };
        if who.is_empty() {
            return false;
        }
        self.start_reviews(squad, &found, &who, round + 1)
    }

    /// Review tickets for `who`, round `round`, on `builder`: `true` when any was made.
    fn start_reviews(&mut self, squad: &str, builder: &Ticket, who: &[Reviewer], round: u32) -> bool {
        let engineer = builder.assignee.clone().or_else(|| builder.owner.clone()).unwrap_or_default();
        let platform = self.on_platform(squad);
        let worktree = builder.worktree.as_ref().map(|w| w.to_string_lossy().into_owned());
        let previous: Vec<String> = self
            .journal(squad)
            .map(|j| j.independent_reviews(builder.id))
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.verdict.as_deref() == Some("changes"))
            .filter_map(|r| r.findings)
            .collect();
        let mut made = Vec::new();
        for reviewer in who {
            let mut body = review_gate::brief(*reviewer, builder.id, &builder.title, &builder.body, &engineer, worktree.as_deref(), builder.branch.as_deref(), round, platform);
            if round > 1 && !previous.is_empty() {
                body.push_str(&format!("\n\nWhat the last round sent back (check these, and the flows they touch — not everything again):\n{}", previous.join("\n---\n")));
            }
            let title = match reviewer {
                Reviewer::Security => say!(self, "ticket.review_security_title", title = builder.title),
                Reviewer::Qa => say!(self, "ticket.review_qa_title", title = builder.title),
            };
            let review = Ticket {
                summary: None,
                id: 0,
                title,
                body,
                status: TicketStatus::Todo,
                kind: TicketKind::Task,
                owner: Some(reviewer.role().to_owned()),
                assignee: None,
                project: builder.project.clone(),
                depends_on: Vec::new(),
                branch: None,
                worktree: None,
                execution: None,
                merged: None,
                off_main: None,
                design_qa: None,
                landing: false,
                plan: None,
                model: None,
                flow: None,
                created_by: "equipe".to_owned(),
                note: None,
                asks: Vec::new(),
                label: None,
                remaining: Vec::new(),
                images: Vec::new(),
                updated_at: None,
                started_at: None,
                app: builder.app.clone(),
                priority: builder.priority,
                rank: 0,
            };
            let Some(journal) = self.journal(squad) else { return false };
            let Ok(id) = journal.create_ticket(&review) else { continue };
            let _ = journal.start_independent_review(id, builder.id, reviewer.code(), round);
            let _ = journal.ticket_event(id, "equipe", "created", &format!("{} of T-{}, round {round}", reviewer.what(), builder.id));
            let _ = journal.ticket_event(builder.id, "equipe", "review asked", &format!("{} on T-{id}, round {round}", reviewer.what()));
            let made_ticket = journal.ticket(id).ok().flatten().unwrap_or(Ticket { id, ..review });
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: made_ticket });
            made.push(format!("{} (T-{id})", reviewer.what()));
        }
        if made.is_empty() {
            return false;
        }
        crate::daemon_log!("equipe: {squad}: T-{} waits for its independent review, round {round}: {}", builder.id, made.join(", "));
        let line = format!(
            "T-{} is held for its independent review before it lands (round {round}): {}. Nothing for you to do: it comes back to you, or lands, on their verdicts.",
            builder.id,
            made.join(", ")
        );
        self.brief_pm(squad, line, None);
        true
    }

    /// `review_verdict`: a reviewer's word on the change its review ticket checks.
    pub(super) fn finish_review(&mut self, squad: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let on = self.sessions.get(&id).and_then(|s| s.work.as_ref()).map(|w| w.ticket).ok_or("You're not on a review — there is no verdict to give.")?;
        let row = self.journal(squad).and_then(|j| j.independent_review(on)).ok_or_else(|| format!("T-{on} isn't a review: hand it in with submit_for_review."))?;
        let reviewer = Reviewer::parse(&row.reviewer).ok_or("This review's kind isn't known.")?;
        let (cwd, roots) = self.picture_places(squad, id);
        let shown = crate::visuals::resolve_all(&board::arg_strs(args, "images"), &cwd, &roots, crate::visuals::HAND_IN)
            .map_err(|e| design_qa::pictures_refused(review_gate::REVIEW_VERDICT, args, &cwd, &roots, &e))?;
        crate::visuals::thumbs::warm(&self.config.home, shown.clone());
        let changed = self.handed_in_files.get(&(squad.to_owned(), row.ticket)).cloned().unwrap_or_default();
        let screens = changed.iter().any(|f| review_gate::screen_file(f));
        let platform = self.on_platform(squad);
        let work = self.sessions.get(&id).and_then(|s| s.work.as_ref()).ok_or("You're not on a ticket.")?;
        let verdict = review_gate::read(args, reviewer, shown.len(), screens, platform, work.qa.as_ref())?;
        let passed = verdict.passed;
        let work = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()).ok_or("You're not on a ticket.")?;
        work.handed_in = Some(verdict.findings.clone());
        work.shown = shown;
        work.waiting = None;
        work.verdict = Some(design_qa::Verdict { passed, findings: verdict.findings });
        Ok(if passed {
            format!("T-{} passes your {} when this turn ends. End your turn now with a short summary.", row.ticket, reviewer.what())
        } else {
            format!("T-{} goes back to its engineer with your findings when this turn ends. End your turn now with a short summary.", row.ticket)
        })
    }

    /// Review ticket `review` finished with `verdict` by `by`: kept, and the
    /// change it checked sent back to its engineer when it found defects (a
    /// pass lets the change go on once every review of its round passed —
    /// `work_ready` asks [`Self::hold_for_review`]). The change as it now is,
    /// and its engineer; `None` when `review` is no independent review.
    pub(super) fn review_checked(&mut self, squad: &str, review: &Ticket, by: &str, verdict: &design_qa::Verdict) -> Option<(Ticket, String)> {
        let journal = self.journal(squad)?;
        let row = journal.independent_review(review.id)?;
        let reviewer = Reviewer::parse(&row.reviewer)?;
        let word = if verdict.passed { "pass" } else { "changes" };
        let _ = journal.set_independent_verdict(review.id, word, &verdict.findings, by);
        let mut checked = journal.ticket(row.ticket).ok().flatten()?;
        let engineer = checked.assignee.clone().or_else(|| checked.owner.clone()).unwrap_or_default();
        let findings = verdict.findings.trim();
        // Sent back only while it still waits in review: the person may have taken it meanwhile.
        if !verdict.passed && checked.status == TicketStatus::Review {
            checked.owner = checked.assignee.take().or(checked.owner);
            checked.status = TicketStatus::Todo;
            checked.note = Some(review_gate::sent_back_note(reviewer, by, findings));
            checked.images.clear();
            let _ = journal.save_ticket(&checked);
            let _ = journal.ticket_event(checked.id, by, "review: changes", findings);
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: checked.clone() });
            self.after_send_back(squad, checked.id, by, equipe_protocol::tiers::SendBackReason::Correctness);
        } else {
            let _ = journal.ticket_event(checked.id, by, if verdict.passed { "review passed" } else { "review: changes (after it left review)" }, findings);
            if verdict.passed {
                for image in &review.images {
                    if !checked.images.contains(image) && checked.images.len() < crate::visuals::HAND_IN {
                        checked.images.push(image.clone());
                    }
                }
                let _ = journal.save_ticket(&checked);
                self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: checked.clone() });
            }
        }
        Some((checked, engineer))
    }

    /// Review `review` closed by the person or the PM rather than a verdict:
    /// it no longer holds the change.
    pub(super) fn review_closed(&mut self, squad: &str, review: TicketId) {
        if let Some(journal) = self.journal(squad)
            && journal.independent_review(review).is_some_and(|r| r.verdict.is_none())
        {
            let _ = journal.set_independent_verdict(review, "cancelled", "", "you");
        }
    }

    /// A tool's answer in a member's run: the platform's `qa_check` kept on
    /// its ticket and its run, a deploy noted (a check before it no longer counts).
    pub(super) fn platform_tool_seen(&mut self, id: SessionId, tool: &str, outcome: &equipe_protocol::ToolOutcome) {
        let deploy = review_gate::is_deploy(tool);
        if !deploy && !review_gate::is_qa_check(tool) {
            return;
        }
        let Some(session) = self.sessions.get_mut(&id) else { return };
        let squad = session.squad.clone();
        let Some(work) = session.work.as_mut() else { return };
        if deploy {
            if outcome.ok {
                work.deployed = true;
                work.qa = None;
            }
            return;
        }
        let seen = review_gate::read_qa_check(outcome.ok, &outcome.output);
        let ticket = work.ticket;
        work.qa = Some(seen.clone());
        // Kept for the PM's acceptance: the change itself, or the one a review checks.
        let (ok, detail) = match &seen {
            QaSeen::Passed => (true, String::new()),
            QaSeen::Failed(failed) => (false, failed.join("; ")),
            QaSeen::Unavailable(_) => return,
        };
        if let Some(squad) = squad
            && let Some(journal) = self.journal(&squad)
        {
            let on = journal.independent_review(ticket).map_or(ticket, |r| r.ticket);
            let _ = journal.record_qa_check(on, ok, &detail);
        }
    }

    /// Why `tickets` can't be accepted yet: a review still running, or a last
    /// `qa_check` that failed. `None`: nothing holds them.
    pub(super) fn not_yet_acceptable(&mut self, squad: &str, tickets: &[TicketId]) -> Option<String> {
        let journal = self.journal(squad)?;
        let mut why = Vec::new();
        for &t in tickets {
            if journal.independent_reviews(t).iter().any(|r| r.verdict.is_none()) {
                why.push(format!("T-{t}'s independent review is still running — it comes to you on its verdict"));
            } else if let Some((false, detail)) = journal.last_qa_check(t) {
                why.push(format!("T-{t}'s last qa_check failed ({detail}) — send it back with what failed"));
            }
        }
        (!why.is_empty()).then(|| format!("Not accepted: {}.", why.join("; ")))
    }
}
