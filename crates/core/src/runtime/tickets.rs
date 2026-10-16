//! The person's own hand on the board: tickets they write, change, move and
//! comment on. Each is the person's command — never a member's word — and
//! every change is told to the PM and the members it concerns: a line in
//! their conversation, and a few words with their next message, so plans
//! stay in step with the board.

use super::*;
use equipe_protocol::{Priority, TicketDraft, TicketEdit};

/// What a change was, said twice: in English for the models, in the person's
/// language for their screen.
struct Change {
    /// Which field: `title`, `body`, `priority`, `kind`, `project`, `depends`, `owner`.
    field: &'static str,
    en: String,
    shown: String,
}

/// A ticket the person changed, to be told about.
struct Edited {
    id: TicketId,
    title: String,
    said: Vec<Change>,
    /// The members it concerns: who had it, who has it.
    concerned: Vec<Option<String>>,
}

impl Runtime {
    /// A ticket's owner, as the person names it: a member of the squad, or a
    /// role one of them holds.
    fn known_owner(&self, squad: &str, name: &str) -> bool {
        self.team(squad).iter().any(|(_, member, role)| member == name || role == name)
    }

    /// The project the person picked, among the squad's: by path or by name.
    fn squads_project(&mut self, squad: &str, picked: &Path) -> Option<PathBuf> {
        let projects = self.projects(squad);
        projects.iter().find(|p| p.as_path() == picked).cloned().or_else(|| board::resolve_project(&picked.to_string_lossy(), &projects))
    }

    /// Where a code ticket naming no project works: the squad's only project;
    /// otherwise the person is asked to pick one.
    fn only_project(&mut self, squad: &str) -> Result<PathBuf, String> {
        let projects = self.projects(squad);
        match projects.as_slice() {
            [one] => Ok(one.clone()),
            [] => Err(say!(self, "error.ticket_no_projects")),
            _ => {
                let names: Vec<String> = projects.iter().map(|p| board::project_name(p)).collect();
                Err(say!(self, "error.ticket_name_project", names = names.join(", ")))
            }
        }
    }

    /// Dependencies the person gave: every one on the board, none the ticket
    /// itself, and no loop of waiting.
    fn check_dependencies(&mut self, squad: &str, ticket: TicketId, depends_on: &[TicketId]) -> Result<(), String> {
        let tickets = self.tickets(squad);
        if let Some(missing) = depends_on.iter().find(|d| !tickets.iter().any(|t| t.id == **d)) {
            return Err(say!(self, "error.no_ticket", id = missing));
        }
        if depends_on.contains(&ticket) {
            return Err(say!(self, "error.waits_on_itself", id = ticket));
        }
        if let Some(path) = board::dependency_loop(&tickets, ticket, depends_on) {
            let path: Vec<String> = path.iter().map(|id| format!("T-{id}")).collect();
            return Err(say!(self, "error.dependency_loop", path = path.join(" → ")));
        }
        Ok(())
    }

    /// Tell member `name` (or the member holding role `name`) about a change
    /// to its ticket: a line in its conversation and words with its next
    /// message. The PM is told on its own.
    fn brief_member(&mut self, squad: &str, name: &str, line: &str, shown: &str) {
        let by_name = self.member_id(squad, name);
        let by_role = || {
            self.sessions
                .iter()
                .filter(|(_, s)| s.squad.as_deref() == Some(squad) && s.lane.is_none() && s.member.as_ref().is_some_and(|m| m.role == name))
                .min_by_key(|(_, s)| (s.name() != Some(name), s.joined))
                .map(|(&id, _)| id)
        };
        let Some(id) = by_name.or_else(by_role) else {
            return;
        };
        if self.pm_of(squad) == Some(id) {
            return;
        }
        if let Some(session) = self.sessions.get_mut(&id) {
            session.briefing.push(line.to_owned());
        }
        self.save_waiting(id);
        self.publish(Event::Notice { session: id, text: shown.to_owned() });
    }

    /// The PM and the members a change concerns hear of it.
    fn tell_about(&mut self, squad: &str, members: &[Option<String>], line: String, shown: String) {
        let mut told: Vec<&str> = Vec::new();
        for name in members.iter().flatten() {
            if !told.contains(&name.as_str()) {
                told.push(name);
                self.brief_member(squad, name, &line, &shown);
            }
        }
        self.brief_pm(squad, line, Some(shown));
    }

    fn priority_word(&self, priority: Priority) -> &'static str {
        crate::i18n::label::priority(self.config.lang, priority)
    }

    // ── create ──

    /// The person writes a ticket: on the board ready, for its owner — or for
    /// the PM to give one.
    pub(super) async fn person_creates(&mut self, squad: &str, draft: TicketDraft) {
        match self.new_ticket(squad, draft) {
            Ok(()) => self.dispatch(squad).await,
            Err(message) => self.squad_error(squad, message),
        }
    }

    fn new_ticket(&mut self, squad: &str, draft: TicketDraft) -> Result<(), String> {
        let title = draft.title.trim().to_owned();
        if title.is_empty() {
            return Err(say!(self, "error.ticket_title"));
        }
        let named = match &draft.project {
            Some(picked) => Some(self.squads_project(squad, picked).ok_or_else(|| say!(self, "error.no_project", name = picked.display()))?),
            None => None,
        };
        let project = match (draft.kind, named) {
            (_, Some(project)) => Some(project),
            (Some(kind), None) if kind.in_home() => None,
            (Some(_), None) => Some(self.only_project(squad)?),
            (None, None) => {
                let projects = self.projects(squad);
                if projects.len() == 1 { projects.first().cloned() } else { None }
            }
        };
        let kind = draft.kind.unwrap_or_else(|| board::default_kind(project.as_deref()));
        let owner = draft.assignee.map(|a| a.trim().to_owned()).filter(|a| !a.is_empty());
        if let Some(owner) = &owner
            && !self.known_owner(squad, owner)
        {
            return Err(say!(self, "error.no_member", name = owner));
        }
        let tickets = self.tickets(squad);
        if let Some(missing) = draft.depends_on.iter().find(|d| !tickets.iter().any(|t| t.id == **d)) {
            return Err(say!(self, "error.no_ticket", id = missing));
        }
        // What is ready goes in the board's order; an urgent one goes on top of it.
        let rank = if draft.priority == Priority::Urgent {
            let mut ready: Vec<&Ticket> = tickets.iter().filter(|t| t.status == TicketStatus::Todo).collect();
            board::board_order(&mut ready);
            ready.first().map_or(0, |first| board::rank_before(&ready, Some(first.id)).unwrap_or(first.order_key() - 1))
        } else {
            0
        };
        let ticket = Ticket {
            id: 0,
            title: title.clone(),
            body: draft.body.trim().to_owned(),
            status: TicketStatus::Todo,
            kind,
            owner: owner.clone(),
            assignee: None,
            project,
            depends_on: draft.depends_on,
            branch: None,
            worktree: None,
            merged: None,
            off_main: None,
            created_by: "you".into(),
            note: None,
            asks: Vec::new(),
            remaining: Vec::new(),
            priority: draft.priority,
            rank,
            updated_at: None,
        };
        let no_squad = say!(self, "error.no_squad", squad = squad);
        let journal = self.journal(squad).ok_or(no_squad)?;
        let id = journal.create_ticket(&ticket).map_err(|e| e.to_string())?;
        let _ = journal.ticket_event(id, "you", "created", &title);
        let ticket = journal.ticket(id).ok().flatten().unwrap_or(Ticket { id, ..ticket });
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
        let urgency = if draft.priority.is_normal() { String::new() } else { format!(", {} priority", draft.priority.label()) };
        let (line, shown) = match &owner {
            Some(owner) => (
                format!("The person put T-{id} on the board for {owner}{urgency}: {title}. It starts when {owner} is free and what it waits for is done."),
                say!(self, "news.person_created", id = id, title = title, name = owner),
            ),
            None => (
                format!("The person put T-{id} on the board{urgency}, with no owner yet: {title}. Give it one (update_ticket) — or tell them who should do it."),
                say!(self, "news.person_created_unowned", id = id, title = title),
            ),
        };
        self.tell_about(squad, &[owner], line, shown);
        Ok(())
    }

    // ── edit ──

    /// The person changes a ticket. All or nothing: a change that doesn't
    /// make sense refuses the lot, with why.
    pub(super) async fn person_edits(&mut self, squad: &str, id: TicketId, changes: TicketEdit) {
        if changes.is_empty() {
            return;
        }
        let Some(found) = self.find_ticket(squad, id) else {
            return;
        };
        match self.apply_edit(squad, found, changes) {
            Ok(Some(edited)) => {
                let line = format!("The person changed T-{} ({}): {}. Plan with the board as it is now.", edited.id, edited.title, Self::en_of(&edited));
                let shown = say!(self, "news.edited", id = edited.id, what = Self::shown_of(&edited));
                self.tell_about(squad, &edited.concerned, line, shown);
                self.dispatch(squad).await;
            }
            Ok(None) => {}
            Err(message) => self.squad_error(squad, message),
        }
    }

    /// The same change to several tickets: each made as it can be, and the
    /// PM and the members told once — "6 tickets changed: owner 3, priority 3".
    pub(super) async fn person_edits_many(&mut self, squad: &str, ids: Vec<TicketId>, changes: TicketEdit) {
        if changes.is_empty() || ids.is_empty() {
            return;
        }
        let mut done: Vec<Edited> = Vec::new();
        let mut refused: Option<String> = None;
        for id in ids {
            let Some(found) = self.find_ticket(squad, id) else { continue };
            match self.apply_edit(squad, found, changes.clone()) {
                Ok(Some(edited)) => done.push(edited),
                Ok(None) => {}
                Err(message) => {
                    refused.get_or_insert(message);
                }
            }
        }
        if let Some(message) = refused {
            self.squad_error(squad, message);
        }
        match done.len() {
            0 => return,
            1 => {
                let edited = &done[0];
                let line = format!("The person changed T-{} ({}): {}. Plan with the board as it is now.", edited.id, edited.title, Self::en_of(edited));
                let shown = say!(self, "news.edited", id = edited.id, what = Self::shown_of(edited));
                let concerned = edited.concerned.clone();
                self.tell_about(squad, &concerned, line, shown);
            }
            n => {
                // What changed, counted by field, in the order they come.
                let mut fields: Vec<(&'static str, usize)> = Vec::new();
                for change in done.iter().flat_map(|e| e.said.iter()) {
                    match fields.iter_mut().find(|(f, _)| *f == change.field) {
                        Some((_, count)) => *count += 1,
                        None => fields.push((change.field, 1)),
                    }
                }
                let shown_what = fields.iter().map(|(f, count)| format!("{} {count}", self.said(&format!("field.{f}"), &[]))).collect::<Vec<_>>().join(" · ");
                let en_each = done.iter().map(|e| format!("T-{} ({}): {}", e.id, e.title, Self::en_of(e))).collect::<Vec<_>>().join("; ");
                let line = format!("The person changed {n} tickets at once — {en_each}. Plan with the board as it is now.");
                let shown = say!(self, "news.edited_many", count = n, what = shown_what);
                let concerned: Vec<Option<String>> = done.iter().flat_map(|e| e.concerned.iter().cloned()).collect();
                self.tell_about(squad, &concerned, line, shown);
            }
        }
        self.dispatch(squad).await;
    }

    fn en_of(edited: &Edited) -> String {
        edited.said.iter().map(|c| c.en.as_str()).collect::<Vec<_>>().join("; ")
    }

    fn shown_of(edited: &Edited) -> String {
        edited.said.iter().map(|c| c.shown.as_str()).collect::<Vec<_>>().join(" · ")
    }

    /// Change `found` as asked, save and publish it — its member stopping, when
    /// its work went to someone else. What changed, to tell; `None`: nothing did.
    fn apply_edit(&mut self, squad: &str, found: Ticket, changes: TicketEdit) -> Result<Option<Edited>, String> {
        let id = found.id;
        let mut t = found.clone();
        let mut said: Vec<Change> = Vec::new();
        if let Some(title) = changes.title.map(|s| s.trim().to_owned()).filter(|s| *s != t.title) {
            if title.is_empty() {
                return Err(say!(self, "error.ticket_title"));
            }
            said.push(Change { field: "title", en: format!("title → “{title}”"), shown: say!(self, "edit.title", title = title) });
            t.title = title;
        }
        if let Some(body) = changes.body.map(|s| s.trim().to_owned()).filter(|s| *s != t.body) {
            said.push(Change { field: "body", en: format!("description → {}", clip(&body, 400)), shown: say!(self, "edit.body") });
            t.body = body;
        }
        if let Some(priority) = changes.priority.filter(|p| *p != t.priority) {
            let (from, to) = (self.priority_word(t.priority), self.priority_word(priority));
            said.push(Change { field: "priority", en: format!("priority {} → {}", t.priority.label(), priority.label()), shown: say!(self, "edit.priority", from = from, to = to) });
            t.priority = priority;
        }
        let started = t.branch.is_some() || t.status == TicketStatus::InProgress;
        if let Some(kind) = changes.kind.filter(|k| *k != t.kind) {
            if started {
                return Err(say!(self, "error.kind_under_way", id = id));
            }
            let (from, to) = (crate::i18n::label::kind(self.config.lang, t.kind), crate::i18n::label::kind(self.config.lang, kind));
            said.push(Change { field: "kind", en: format!("kind {} → {}", t.kind.label(), kind.label()), shown: say!(self, "edit.kind", from = from, to = to) });
            t.kind = kind;
        }
        if let Some(project) = changes.project {
            let project = match project {
                Some(picked) => Some(self.squads_project(squad, &picked).ok_or_else(|| say!(self, "error.no_project", name = picked.display()))?),
                None => None,
            };
            if project != t.project {
                if started {
                    return Err(say!(self, "error.project_under_way", id = id));
                }
                let name = |p: &Option<PathBuf>| p.as_deref().map_or_else(|| "—".to_owned(), board::project_name);
                said.push(Change {
                    field: "project",
                    en: format!("project {} → {}", name(&t.project), name(&project)),
                    shown: say!(self, "edit.project", from = name(&t.project), to = name(&project)),
                });
                t.project = project;
            }
        }
        // Code needs its repository: none named, the squad's only one.
        if !t.kind.in_home() && t.project.is_none() {
            t.project = Some(self.only_project(squad)?);
        }
        if let Some(deps) = changes.depends_on.filter(|d| *d != t.depends_on) {
            self.check_dependencies(squad, id, &deps)?;
            let list = |ids: &[TicketId]| if ids.is_empty() { "—".to_owned() } else { ids.iter().map(|d| format!("T-{d}")).collect::<Vec<_>>().join(", ") };
            said.push(Change { field: "depends", en: format!("waits for {}", if deps.is_empty() { "nothing".into() } else { list(&deps) }), shown: say!(self, "edit.depends", list = list(&deps)) });
            t.depends_on = deps;
        }
        // Who does it. Work under way goes to them from its branch.
        let before = board_who(&found);
        let mut reassign: Option<(SessionId, String)> = None;
        if let Some(owner) = changes.assignee.map(|o| o.map(|o| o.trim().to_owned()).filter(|o| !o.is_empty()))
            && owner != before
        {
            if t.status.is_closed() {
                return Err(say!(self, "error.owner_closed", id = id));
            }
            if let Some(name) = &owner
                && !self.known_owner(squad, name)
            {
                return Err(say!(self, "error.no_member", name = name));
            }
            let nobody = say!(self, "edit.nobody");
            let (from, to) = (before.clone().unwrap_or_else(|| nobody.clone()), owner.clone().unwrap_or(nobody));
            said.push(Change { field: "owner", en: format!("owner {} → {}", before.as_deref().unwrap_or("nobody"), owner.as_deref().unwrap_or("nobody (the PM gives it one)")), shown: say!(self, "edit.owner", from = from, to = to) });
            match (t.status, self.worker_on(squad, id)) {
                (TicketStatus::InProgress, Some(worker)) => match &owner {
                    Some(name) => reassign = Some((worker, name.clone())),
                    None => return Err(say!(self, "error.owner_under_way", id = id)),
                },
                _ => {
                    t.owner = owner;
                    t.assignee = None;
                }
            }
        }
        if said.is_empty() {
            return Ok(None);
        }
        let shown_what = said.iter().map(|c| c.shown.as_str()).collect::<Vec<_>>().join(" · ");
        if let Some(journal) = self.journal(squad) {
            journal.save_ticket(&t).map_err(|e| e.to_string())?;
            // Its record says what the person changed, in their words.
            let _ = journal.ticket_event(id, "you", "edited", &shown_what);
        }
        let concerned = vec![found.assignee.clone(), found.owner.clone(), t.owner.clone()];
        let title = t.title.clone();
        match reassign {
            Some((worker, to)) => {
                // Its member stops; the ticket goes back on the board for `to`.
                self.stop_turn(worker);
                self.end_work(worker, WorkEnd::Reassigned { to });
            }
            None => self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: t }),
        }
        Ok(Some(Edited { id, title, said, concerned }))
    }

    // ── move ──

    /// The person moves a ticket on the board: to another status (what each
    /// move means is [`board::move_rule`]'s) and to a place in its column.
    pub(super) async fn person_moves(&mut self, squad: &str, id: TicketId, status: Option<TicketStatus>, before: Option<TicketId>) {
        let Some(found) = self.find_ticket(squad, id) else {
            return;
        };
        let to = status.unwrap_or(found.status);
        let rule = match board::move_rule(found.status, to) {
            Ok(rule) => rule,
            Err(refusal) => {
                let message = self.said(&format!("error.refuse_{}", refusal.code()), &[("id", id.to_string())]);
                return self.squad_error(squad, message);
            }
        };
        // Its place in the column it goes to, the rest spread out when there's no room.
        let rank = self.place(squad, id, to, before);
        let mut t = found.clone();
        t.rank = rank;
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&t);
        }
        let status_word = crate::i18n::label::status(self.config.lang, to);
        let concerned = vec![t.assignee.clone(), t.owner.clone()];
        match rule {
            board::Move::Reorder => {
                let after = before.map_or_else(|| "at the bottom".to_owned(), |b| format!("before T-{b}"));
                let line = format!("The person moved T-{id} ({}) {after} in {} — it goes in that order.", t.title, to.label());
                let shown = say!(self, "news.reordered", id = id, status = status_word);
                self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: t });
                self.brief_pm(squad, line, Some(shown));
            }
            board::Move::TakeOn => return self.take_on(squad, t).await,
            board::Move::BackToProposal | board::Move::Restore(_) => {
                t.status = to;
                t.assignee = None;
                let (event, line, shown) = if rule == board::Move::BackToProposal {
                    ("back to proposal", format!("The person put T-{id} ({}) back to a proposal: it isn't to be done for now.", t.title), say!(self, "news.back_to_proposal", id = id, title = t.title))
                } else {
                    t.note = Some("Restored by the person.".into());
                    ("restored", format!("The person brought T-{id} ({}) back, as {}.", t.title, to.label()), say!(self, "news.restored", id = id, title = t.title, status = status_word))
                };
                if let Some(journal) = self.journal(squad) {
                    let _ = journal.save_ticket(&t);
                    let _ = journal.ticket_event(id, "you", event, status_word);
                }
                self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: t });
                self.tell_about(squad, &concerned, line, shown);
                self.dispatch(squad).await;
            }
            board::Move::Accept => self.accept(squad, id).await,
            board::Move::Cancel => self.cancel(squad, id).await,
            board::Move::Resume => self.resume(squad, id, String::new()).await,
            board::Move::SendBack => self.reopen(squad, id, String::new()).await,
        }
    }

    /// The rank that puts ticket `id` before `before` among the tickets of
    /// `status` (`None`: last). When its neighbours sit too close, the
    /// column is spread out first, each ticket that moved saved and published.
    fn place(&mut self, squad: &str, id: TicketId, status: TicketStatus, before: Option<TicketId>) -> i64 {
        let tickets = self.tickets(squad);
        let mut column: Vec<&Ticket> = tickets.iter().filter(|t| t.status == status && t.id != id).collect();
        board::board_order(&mut column);
        if let Some(rank) = board::rank_before(&column, before) {
            return rank;
        }
        let spread = board::spread(&column);
        let mut moved: Vec<Ticket> = Vec::new();
        for (tid, rank) in &spread {
            if let Some(t) = tickets.iter().find(|t| t.id == *tid).filter(|t| t.rank != *rank) {
                moved.push(Ticket { rank: *rank, ..t.clone() });
            }
        }
        if let Some(journal) = self.journal(squad) {
            for t in &moved {
                let _ = journal.save_ticket(t);
            }
        }
        for t in moved {
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: t });
        }
        let tickets: Vec<Ticket> = tickets.iter().map(|t| spread.iter().find(|(tid, _)| *tid == t.id).map_or_else(|| t.clone(), |(_, r)| Ticket { rank: *r, ..t.clone() })).collect();
        let mut column: Vec<&Ticket> = tickets.iter().filter(|t| t.status == status && t.id != id).collect();
        board::board_order(&mut column);
        board::rank_before(&column, before).unwrap_or(0)
    }

    /// A proposal taken on: ready for its owner (the person's yes — never a member's).
    pub(super) async fn take_on(&mut self, squad: &str, mut ticket: Ticket) {
        ticket.status = TicketStatus::Todo;
        ticket.note = None;
        let id = ticket.id;
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&ticket);
            let _ = journal.ticket_event(id, "you", "taken on", "");
        }
        let line = format!("The person took on T-{id} ({}): it is ready for its owner.", ticket.title);
        let shown = say!(self, "news.taken_on", id = id, title = ticket.title);
        let concerned = vec![ticket.owner.clone(), Some(ticket.created_by.clone())];
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
        self.tell_about(squad, &concerned, line, shown);
        self.dispatch(squad).await;
    }

    // ── comment ──

    /// The person writes on a ticket: kept in its record; the member on it
    /// reads it as a message from them, else its owner and the PM get it with
    /// their next one.
    pub(super) async fn person_comments(&mut self, squad: &str, id: TicketId, text: String) {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return;
        }
        let Some(found) = self.find_ticket(squad, id) else {
            return;
        };
        if let Some(journal) = self.journal(squad) {
            let _ = journal.ticket_event(id, "you", "comment", &text);
        }
        self.send_history(squad, id);
        match self.worker_on(squad, id) {
            // On it now: it's the person talking to it (the PM hears, as always).
            Some(worker) => self.submit(worker, format!("[T-{id}] {text}")).await,
            None => {
                let line = format!("The person commented on T-{id} ({}): {text}", found.title);
                let shown = say!(self, "news.commented", id = id, text = clip(&text, 120));
                self.tell_about(squad, &[found.assignee.clone(), found.owner.clone()], line, shown);
            }
        }
    }

    /// A ticket's record, to the screens following its squad.
    pub(super) fn send_history(&mut self, squad: &str, id: TicketId) {
        let Some(entries) = self.journal(squad).and_then(|j| j.ticket_entries(id).ok()) else {
            return;
        };
        self.publish_in(Some(squad), Event::TicketHistory { squad: squad.to_owned(), ticket: id, entries });
    }
}

/// Who does a ticket: the member on it, else its owner.
fn board_who(t: &Ticket) -> Option<String> {
    t.assignee.clone().or_else(|| t.owner.clone())
}
