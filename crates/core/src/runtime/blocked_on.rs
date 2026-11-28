//! A ticket blocked on work that doesn't exist yet, as the daemon carries it
//! out (`crate::blocked_on` has the rules): the missing piece becomes a
//! ticket of its role under the same request — or the one on the board
//! already is — and the blocked ticket waits on it.

use equipe_protocol::requests::Link;

use super::*;
use crate::blocked_on::{self as rules, BlockedOn};

impl Runtime {
    /// Whether `role` — a role or a member's name — is on `squad`'s team.
    pub(super) fn staffed(&self, squad: &str, role: &str) -> bool {
        self.member_id(squad, role).is_some() || self.team(squad).iter().any(|(_, _, r)| r == role)
    }

    /// Check `missing` before anything is put down: its role is on the team,
    /// its project is one of the squad's.
    pub(super) fn check_blocked_on(&mut self, squad: &str, missing: &BlockedOn) -> Result<(), String> {
        if !self.staffed(squad, &missing.role) {
            return Err(format!(
                "Nobody on the squad is {role}, whose work `blocked_on` names. Hand it back as `not_my_role` with `suggest` {role} \
instead: the PM can propose adding one.",
                role = missing.role
            ));
        }
        if let Some(project) = &missing.project {
            let projects = self.projects(squad);
            if board::resolve_project(project, &projects).is_none() {
                return Err(format!("There is no project {project} in this squad. Leave `project` out when it is this ticket's own."));
            }
        }
        Ok(())
    }

    /// The ticket `blocked` waits on for `missing`: the one on the board that
    /// already is it, or a new one for its role, under `blocked`'s request,
    /// made by `by` with `note`. Its id, and whether it is new.
    pub(super) fn prerequisite_for(&mut self, squad: &str, blocked: &Ticket, missing: &BlockedOn, by: &str, note: &str) -> Result<(TicketId, bool), String> {
        self.check_blocked_on(squad, missing)?;
        let board = self.tickets(squad);
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        let request = journal.requests().of_ticket(blocked.id);
        let in_request: Vec<TicketId> = request.and_then(|r| journal.requests().get(r)).map(|row| row.tickets.iter().map(|t| t.ticket).collect()).unwrap_or_default();
        if let Some(found) = rules::find_prerequisite(&board, blocked, &in_request, missing) {
            if let Some(cycle) = board::dependency_loop(&board, blocked.id, &[found]) {
                let path: Vec<String> = cycle.iter().map(|t| format!("T-{t}")).collect();
                return Err(format!("T-{found} already waits on T-{}: {}. Hand it back as `cannot` and say why instead.", blocked.id, path.join(" → ")));
            }
            if let Some(request) = request
                && journal.requests().of_ticket(found).is_none()
            {
                let _ = journal.requests().link(request, found, Some(blocked.id), Link::Linked, now_secs());
            }
            return Ok((found, false));
        }
        let projects = self.projects(squad);
        let project = missing.project.as_deref().and_then(|p| board::resolve_project(p, &projects)).or_else(|| blocked.project.clone());
        let title = rules::title(&missing.what);
        let made = Ticket { summary: None,
            id: 0,
            title: title.clone(),
            body: rules::body(missing, blocked, by, note),
            status: TicketStatus::Todo,
            kind: blocked.kind,
            label: None,
            owner: Some(missing.role.clone()),
            assignee: None,
            project,
            depends_on: Vec::new(),
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            landing: false,
            created_by: by.to_owned(),
            note: None,
            asks: Vec::new(),
            remaining: Vec::new(),
            images: Vec::new(),
            // It holds up work under way: no lower than what it holds up.
            priority: blocked.priority.max(equipe_protocol::Priority::High),
            rank: 0,
            updated_at: None,
            started_at: None,
            app: None,
            design_qa: None,
            plan: None,
            model: None,
            flow: None,
        };
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        let id = journal.create_ticket(&made).map_err(|e| e.to_string())?;
        let _ = journal.ticket_event(id, by, "created", &title);
        let _ = journal.ticket_event(id, "equipe", rules::PREREQUISITE, &format!("for T-{}", blocked.id));
        if let Some(request) = request {
            let _ = journal.requests().link(request, id, Some(blocked.id), Link::Made, now_secs());
        }
        if let Some(made) = journal.ticket(id).ok().flatten() {
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: made });
        }
        crate::daemon_log!("equipe: {squad}: T-{} is blocked on missing work; made T-{id} for {}: {}", blocked.id, missing.role, missing.what);
        Ok((id, true))
    }

    /// Put `ticket` down to wait on `prerequisite`, back to `owner` once it
    /// lands: the record's line for it.
    pub(super) fn wait_on_prerequisite(ticket: &mut Ticket, prerequisite: TicketId, owner: &str, missing: &BlockedOn, note: &str) -> String {
        if !ticket.depends_on.contains(&prerequisite) {
            ticket.depends_on.push(prerequisite);
        }
        ticket.status = TicketStatus::Todo;
        ticket.assignee = None;
        ticket.owner = Some(owner.to_owned());
        ticket.note = Some(format!("{}T-{prerequisite} ({}: {}), which is closed now. {note}", board::WAITED_ON, missing.role, missing.what));
        format!("T-{prerequisite} ({}): {}", missing.role, missing.what)
    }

    /// A pass of `ticket` by `name` with `note` — a baton, a hand-back — that
    /// would be the relay on missing work (`crate::blocked_on`): its history
    /// says so. `None` when it isn't.
    pub(super) fn relay_caught(&mut self, squad: &str, ticket: TicketId, note: &str) -> Option<usize> {
        let entries = self.journal(squad)?.ticket_entries(ticket).ok()?;
        let history: Vec<(&str, &str)> = entries.iter().map(|e| (e.kind.as_str(), e.text.as_str())).collect();
        if !rules::relay_on_missing(note, history.iter().copied()) {
            return None;
        }
        Some(history.iter().filter(|(kind, _)| rules::PASSES.contains(kind)).count() + 1)
    }

    /// The PM's `update_ticket` with `blocked_on`: yes to the card — the
    /// missing ticket made or found, and `ticket` waiting on it.
    pub(super) fn pm_blocked_on(&mut self, squad: &str, pm: &str, ticket_id: TicketId, missing: &BlockedOn) -> Result<String, String> {
        let mut ticket = self.find_ticket(squad, ticket_id).ok_or_else(|| format!("There is no T-{ticket_id}."))?;
        if !matches!(ticket.status, TicketStatus::Proposed | TicketStatus::Todo | TicketStatus::Waiting) {
            return Err(format!("T-{ticket_id} is {} — only a ticket not under way waits on missing work.", ticket.status.label()));
        }
        let note = ticket.note.clone().unwrap_or_default();
        let (prerequisite, made) = self.prerequisite_for(squad, &ticket, missing, pm, &note)?;
        let owner = ticket.owner.clone().or_else(|| ticket.assignee.clone()).unwrap_or_default();
        let line = Self::wait_on_prerequisite(&mut ticket, prerequisite, &owner, missing, "");
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        journal.save_ticket(&ticket).map_err(|e| e.to_string())?;
        let _ = journal.ticket_event(ticket_id, pm, rules::BLOCKED_ON, &line);
        if let Some(saved) = journal.ticket(ticket_id).ok().flatten() {
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: saved });
        }
        let how = if made { "Made" } else { "Found" };
        Ok(format!(
            "{how} T-{prerequisite} for {role} ({what}) under the same request. T-{ticket_id} waits on it and goes back to {owner} once it lands.",
            role = missing.role,
            what = missing.what,
        ))
    }
}
