//! A red main never holds the squad's work back. When one of the project's
//! checks fails on main itself — equipe's checks ran it on main alone
//! (`crate::checks`), or a member found it failing on main before handing in
//! (`submit_for_review`'s `failing_on_main`) — the work that fails only that
//! still lands, recorded as "pre-existing on main", and equipe makes sure
//! exactly one top-priority "main is red" ticket exists to fix it: one per
//! failing check, never a second while the first is open, and never a ticket
//! the rest of the board waits on.

use super::*;

/// The most of a failing check's output put on its ticket.
const TAIL_KEPT: usize = 3000;

/// Which "main is red" ticket fixes `command` failing in `project`.
fn key(project: Option<&Path>, command: &str) -> String {
    format!("{}\n{}", project.map(|p| p.display().to_string()).unwrap_or_default(), command.split_whitespace().collect::<Vec<_>>().join(" "))
}

impl Runtime {
    /// The open "main is red" ticket of `squad`, if `ticket` is one.
    pub(super) fn is_main_red(&self, squad: &str, ticket: TicketId) -> bool {
        let Some(journal) = self.journals.get(squad) else { return false };
        journal.main_red().values().any(|t| *t == ticket) && journal.ticket(ticket).ok().flatten().is_some_and(|t| !t.status.is_closed())
    }

    /// Refused when `deps` names an open "main is red" ticket: nothing waits on one.
    pub(super) fn no_wait_on_main_red(&self, squad: &str, deps: &[TicketId]) -> Result<(), String> {
        match deps.iter().find(|d| self.is_main_red(squad, **d)) {
            Some(red) => Err(format!(
                "T-{red} fixes a check that fails on main itself, and nothing waits on it: work that fails only that check still \
lands, recorded as pre-existing on main. Leave T-{red} out — hand the work in, naming the check in `failing_on_main`."
            )),
            None => Ok(()),
        }
    }

    /// Make sure one top-priority ticket fixes `command` failing on main in
    /// `project`: the open one kept, noted with `seen_on`; else a new one at
    /// the front of `owner`'s queue. Its id.
    pub(super) fn ensure_main_red(&mut self, squad: &str, project: Option<&Path>, command: &str, tail: &str, seen_on: TicketId, owner: Option<&str>) -> Option<TicketId> {
        let command = command.trim();
        if command.is_empty() || !fixable_on_main(command) {
            return None;
        }
        let key = key(project, command);
        let journal = self.journal(squad)?;
        let mut red = journal.main_red();
        if let Some(open) = red.get(&key).copied().filter(|t| journal.ticket(*t).ok().flatten().is_some_and(|t| !t.status.is_closed())) {
            let _ = journal.ticket_event(open, "equipe", "seen again", &format!("Also failed on main while T-{seen_on} was checked."));
            return Some(open);
        }
        let shown_command = clip(command, 80);
        let title = say!(self, "ticket.main_red_title", command = shown_command);
        let tail = if tail.trim().is_empty() {
            String::new()
        } else {
            let mut kept = tail.trim().to_owned();
            if kept.len() > TAIL_KEPT {
                let mut cut = kept.len() - TAIL_KEPT;
                while !kept.is_char_boundary(cut) {
                    cut += 1;
                }
                kept = kept[cut..].to_owned();
            }
            format!("\n\nMain's output ended:\n```\n{kept}\n```")
        };
        let body = format!(
            "`{command}` fails on main itself: it failed on main alone, without any one change, so it is no change's fault. \
Work that fails only this keeps landing, recorded as pre-existing on main; this ticket gets main green again.{tail}\n\n\
Reproduce it on main, fix the cause (or the check, if the check itself is wrong), run the same command until it passes, \
and hand it in. Nothing else waits on this ticket — don't make other tickets depend on it."
        );
        let made = Ticket {
            id: 0,
            title: title.clone(),
            body,
            status: TicketStatus::Todo,
            kind: board::default_kind(project),
            label: Some(equipe_protocol::title::TicketLabel::Fix),
            owner: owner.map(str::to_owned),
            assignee: None,
            project: project.map(Path::to_path_buf),
            depends_on: Vec::new(),
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            landing: false,
            created_by: "equipe".to_owned(),
            note: None,
            asks: Vec::new(),
            remaining: Vec::new(),
            images: Vec::new(),
            priority: equipe_protocol::Priority::Urgent,
            rank: 0,
            updated_at: None,
            design_qa: None,
            plan: None,
            model: None,
            flow: None,
        };
        let journal = self.journal(squad)?;
        let id = journal.create_ticket(&made).ok()?;
        let _ = journal.ticket_event(id, "equipe", "created", &title);
        red.retain(|_, t| journal.ticket(*t).ok().flatten().is_some_and(|t| !t.status.is_closed()));
        red.insert(key, id);
        let _ = journal.set_main_red(&red);
        if let Some(made) = journal.ticket(id).ok().flatten() {
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: made });
        }
        crate::daemon_log!("equipe: {squad}: main fails `{command}` on its own: T-{id} opened to fix it");
        let line = format!(
            "Main fails `{command}` on its own (seen while T-{seen_on} was checked). equipe opened T-{id} \"{title}\" at the top of the board to fix \
it. Work that fails only this keeps landing — don't make anything depend on T-{id}."
        );
        self.brief_pm(squad, line, None);
        Some(id)
    }

    /// `submit_for_review`'s `failing_on_main`: the checks the member found
    /// failing on main just the same — each recorded on its ticket as
    /// pre-existing and given its one "main is red" ticket. What the
    /// member is told.
    pub(super) fn handed_in_failing_on_main(&mut self, squad: &str, ticket: TicketId, member: &str, failing: &[String]) -> String {
        let failing: Vec<&str> = failing.iter().map(|f| f.trim()).filter(|f| !f.is_empty()).take(5).collect();
        if failing.is_empty() {
            return String::new();
        }
        let project = self.find_ticket(squad, ticket).and_then(|t| t.project.clone());
        let mut fixed_by = Vec::new();
        for check in &failing {
            if let Some(journal) = self.journal(squad) {
                let _ = journal.ticket_event(ticket, member, "pre-existing on main", check);
            }
            if let Some(red) = self.ensure_main_red(squad, project.as_deref(), check, "", ticket, Some(member)) {
                fixed_by.push(format!("T-{red}"));
            }
        }
        fixed_by.dedup();
        format!(
            " Recorded as pre-existing on main, not this change's: {}. {} fixes main; this work doesn't wait for it.",
            failing.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", "),
            fixed_by.join(", ")
        )
    }
}

/// Whether a check failing on main is the project's to fix with an urgent
/// ticket. equipe's own advisory checks (`equipe design flows check`, …) are
/// not: a stale design map is refreshed by the daily design review, and in
/// production an urgent ticket for one looped between roles for hours.
fn fixable_on_main(command: &str) -> bool {
    let first = command.split_whitespace().next().unwrap_or_default();
    let program = first.rsplit('/').next().unwrap_or(first);
    program != "equipe"
}

#[cfg(test)]
mod fixable_tests {
    use super::fixable_on_main;

    #[test]
    fn equipes_own_checks_never_open_a_main_red_ticket() {
        assert!(!fixable_on_main("equipe design flows check --json"));
        assert!(!fixable_on_main("/Users/x/.local/bin/equipe design critique"));
        assert!(fixable_on_main("npm test"));
        assert!(fixable_on_main("cargo test --workspace"));
    }
}
