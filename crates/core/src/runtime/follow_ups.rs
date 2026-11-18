//! A designer's follow-ups (`crate::design_qa::follow_up` has the rules):
//! a ticket a designer files from the ticket it is on keeps where it came
//! from; filed from a finding it says its severity, waits for the pass it
//! came from and goes into the open follow-up of the same work while nobody
//! has started it, and polish found
//! while working a follow-up — or past the squad's cap — goes into the
//! root's design backlog, an idea for the person, the PM told.

use super::*;
use crate::design_qa::follow_up::{self as rules, FollowUp, Kind, Lineage, Place, Severity, Why};

/// Where a ticket a designer files from its ticket goes.
#[derive(Debug, Clone, Copy)]
pub(super) struct Filing {
    /// The ticket its designer is on: a new fix or polish ticket waits for it,
    /// so every finding of the same pass goes into one.
    on: TicketId,
    lineage: Lineage,
    place: Place,
    severity: Option<Severity>,
}

impl Runtime {
    /// A designer's `create_ticket` from the ticket it is on: its lineage
    /// and where it goes. `Ok(None)`: not a designer, or on no ticket — the
    /// ticket is filed as before. Refused when it is filed from a finding
    /// without its severity.
    pub(super) fn follow_up_filing(&mut self, squad: &str, id: SessionId, args: &Value) -> Result<Option<Filing>, String> {
        let Some(session) = self.sessions.get(&id) else { return Ok(None) };
        if session.member.as_ref().is_none_or(|m| m.role != design_qa::DESIGNER) {
            return Ok(None);
        }
        let Some(on) = session.work.as_ref().map(|w| w.ticket) else { return Ok(None) };
        let severity = match board::arg_str(args, "severity").map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => Some(Severity::parse(s).ok_or_else(|| format!("`severity` is S1, S2, S3 or S4, not {s}."))?),
            None => None,
        };
        // A design QA's findings are about the work it checks; a review's, about the review.
        let (parent, finding, context) = match self.checked_by(squad, on) {
            Some(checked) => (checked.id, true, format!("T-{on} is the design QA of T-{}", checked.id)),
            None if self.design_batch_kind(squad, on).is_some() => (on, true, format!("T-{on} is a design review")),
            None => (on, false, String::new()),
        };
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        let of_parent = journal.follow_up(parent);
        let lineage = rules::lineage(parent, of_parent.as_ref(), finding);
        // Polish is no ticket to run (`crate::flow`): the backlog keeps it for the person.
        let place = match rules::place(&lineage, severity, journal.open_polish()) {
            Place::Polish => Place::Backlog(rules::Why::Budget),
            place => place,
        };
        if place == Place::NeedsSeverity {
            let context = if context.is_empty() { format!("T-{on} is itself a follow-up (of T-{})", of_parent.map_or(parent, |p| p.parent)) } else { context };
            return Err(format!(
                "{context}, so what you file from it is a finding: say its `severity`. S1/S2 — function or data broken, layout broken \
at a major width, a WCAG A/AA failure, a clear difference from the spec — is a ticket now. S3/S4 — what keeps it from looking \
finished — is polish: one ticket for the work it is about, and found while working a follow-up it goes to the design backlog \
for the person. Then create it again."
            ));
        }
        Ok(Some(Filing { on, lineage, place, severity }))
    }

    /// `ticket`, as `create_ticket` built it, filed as `filing` says.
    /// `Ok(Some(reply))`: it went into an open follow-up or the design
    /// backlog — no new ticket the usual way. `Ok(None)`: `ticket` is shaped
    /// as its kind of follow-up and goes on the board the usual way; keep its
    /// record with [`Runtime::keep_follow_up`] once it has a number.
    pub(super) fn file_follow_up(&mut self, squad: &str, member: &str, filing: &Filing, ticket: &mut Ticket) -> Result<Option<String>, String> {
        let Some(kind) = filing.place.kind() else { return Ok(None) };
        let Lineage { parent, root, generation, .. } = filing.lineage;
        if kind == Kind::Plain {
            return Ok(None);
        }
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        let into = journal.open_follow_up(if kind == Kind::Backlog { root } else { parent }, kind);
        if let Some(into) = into.and_then(|t| journal.ticket(t).ok().flatten()).filter(rules::takes_more) {
            let mut into = into;
            into.body = rules::appended(&into.body, &ticket.title, filing.severity, parent, &ticket.body);
            journal.save_ticket(&into).map_err(|e| e.to_string())?;
            let _ = journal.ticket_event(into.id, member, "finding added", &ticket.title);
            let n = into.id;
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: into });
            self.brief_pm(squad, format!("{member} added a finding of T-{parent} to its open follow-up T-{n}: {}", ticket.title), None);
            let what = match kind {
                Kind::Fix => "fix",
                Kind::Polish => "polish follow-up",
                _ => "design backlog",
            };
            return Ok(Some(format!(
                "Added to T-{n}, the open {what} of T-{}: findings about the same work go together, so no new ticket. Say it in your hand-in and go on.",
                if kind == Kind::Backlog { root } else { parent }
            )));
        }
        if let Some(priority) = rules::priority(kind) {
            ticket.priority = priority;
        }
        if matches!(kind, Kind::Fix | Kind::Polish) && !ticket.depends_on.contains(&filing.on) {
            ticket.depends_on.push(filing.on);
        }
        let severity = filing.severity.map(Severity::label).unwrap_or_default();
        match filing.place {
            Place::Fix => {
                ticket.body = format!("{}\n\n{}", say!(self, "ticket.follow_up_fix", id = parent, severity = severity), ticket.body.trim());
                Ok(None)
            }
            Place::Polish => {
                ticket.body = format!("{}\n\n{}", say!(self, "ticket.follow_up_polish", id = parent, severity = severity), ticket.body.trim());
                Ok(None)
            }
            Place::Backlog(why) => {
                let head = say!(self, "ticket.design_backlog_body", id = root);
                let backlog = Ticket {
                    title: say!(self, "ticket.design_backlog_title", id = root),
                    body: rules::appended(&head, &ticket.title, filing.severity, parent, &ticket.body),
                    status: TicketStatus::Idea,
                    owner: ticket.owner.clone().or_else(|| Some(member.to_owned())),
                    ..ticket.clone()
                };
                let journal = self.journal(squad).ok_or("The board can't be opened.")?;
                let made = journal.create_ticket(&backlog).map_err(|e| e.to_string())?;
                let _ = journal.ticket_event(made, member, "created", &format!("design backlog of T-{root}"));
                let _ = journal.keep_follow_up(&FollowUp { ticket: made, parent, root, generation, kind, severity: filing.severity });
                self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: Ticket { id: made, ..backlog } });
                let because = match why {
                    Why::Depth => format!("polish found while working a follow-up of T-{root}"),
                    Why::Cap => format!("{} polish follow-ups are open already", rules::POLISH_CAP),
                    Why::Budget => "S3/S4 polish is never a ticket of its own: in scope, add it to the change's checklist".to_owned(),
                };
                let line = format!("{member} kept S3/S4 polish of T-{root} in the design backlog T-{made} ({because}): an idea nothing starts — put it to the person when its time comes.");
                let shown = say!(self, "news.design_backlog", name = member, id = root, backlog = made);
                self.brief_pm(squad, line, Some(shown));
                Ok(Some(format!(
                    "Kept in T-{made}, the design backlog of T-{root}, not as a ticket of its own: {because}. The person starts it when they want it. Say it in your hand-in and go on."
                )))
            }
            Place::NeedsSeverity | Place::Plain => Ok(None),
        }
    }

    /// The record of follow-up `ticket`, just made from `filing`.
    pub(super) fn keep_follow_up(&mut self, squad: &str, ticket: TicketId, filing: &Filing) {
        let Some(kind) = filing.place.kind() else { return };
        let Lineage { parent, root, generation, .. } = filing.lineage;
        if let Some(journal) = self.journal(squad) {
            let _ = journal.keep_follow_up(&FollowUp { ticket, parent, root, generation, kind, severity: filing.severity });
        }
    }

    /// The record of a fix or polish ticket a daily review filed about `about`.
    pub(super) fn keep_review_follow_up(&mut self, squad: &str, ticket: TicketId, about: TicketId, kind: Kind, severity: Severity) {
        let Some(journal) = self.journal(squad) else { return };
        let lineage = rules::lineage(about, journal.follow_up(about).as_ref(), true);
        let _ = journal.keep_follow_up(&FollowUp { ticket, parent: about, root: lineage.root, generation: lineage.generation, kind, severity: Some(severity) });
    }

    /// Where `ticket` came from, if it is a design follow-up.
    pub(super) fn follow_up_of(&mut self, squad: &str, ticket: TicketId) -> Option<FollowUp> {
        self.journal(squad)?.follow_up(ticket)
    }
}
