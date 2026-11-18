//! The person's requests (`crate::requests` has the rules). A request is
//! recorded when a member puts the first ticket on the board in answer to
//! what the person said in a conversation — or when the person puts one
//! there themselves; tickets made while working on its tickets join it by
//! their lineage (made during a turn on one, a design follow-up of one, its
//! design QA), and a member can name it (`request`). Where each stands goes
//! to the screens whenever a ticket of it moves; a milestone — done, live,
//! waiting on the person, stopped, all done — is told once, in one line, in
//! the conversation it was asked in, a couple of minutes after the first
//! change so what moves together is said together; a member's turn there
//! starts with where each stands; and while the member is busy, a message
//! asking after one is answered from the board at once.

use equipe_protocol::requests::{Link, Request, RequestId};

use super::*;
use crate::requests::{self as rules, Row};

/// What the person last said in a conversation: a ticket a member puts on
/// the board in answer serves it — every ticket from the same words, the
/// same request.
#[derive(Debug, Clone)]
pub(super) struct Said {
    text: String,
    at: u64,
    request: Option<RequestId>,
    /// Their latest words there, whatever they were — asking after a request too.
    last: (String, u64),
}

impl Runtime {
    /// The person said `text` in conversation `session` (a member's, or a
    /// lane of it). Words asking after a request of theirs there aren't a new
    /// one: what they said before stays what a ticket made now answers.
    pub(super) fn person_said(&mut self, session: SessionId, text: &str) {
        let Some(squad) = self.sessions.get(&session).filter(|s| s.member.is_some()).and_then(|s| s.squad.clone()) else { return };
        if text.trim().is_empty() {
            return;
        }
        let now = now_secs();
        let asks_after = !rules::asked_after(text, &self.requests_in(&squad, session)).is_empty();
        let said = self.person_said.entry(session).or_insert_with(|| Said { text: String::new(), at: now, request: None, last: (String::new(), now) });
        said.last = (text.to_owned(), now);
        if !asks_after {
            *said = Said { text: text.to_owned(), at: now, request: None, last: (text.to_owned(), now) };
        }
    }

    /// What the person said to the member of session `member` lately, in
    /// any of its conversations: each with when.
    pub(super) fn person_said_to(&self, member: SessionId) -> Vec<(String, u64)> {
        self.person_said
            .iter()
            .filter(|(at, _)| self.member_session(**at) == member)
            .flat_map(|(_, s)| [(s.text.clone(), s.at), s.last.clone()])
            .filter(|(text, _)| !text.is_empty())
            .collect()
    }

    /// `request`, as a member names it on a ticket (`R-3` or `3`): checked
    /// before the ticket is made. `Ok(None)` when it names none.
    pub(super) fn request_arg(&mut self, squad: &str, args: &Value) -> Result<Option<RequestId>, String> {
        let named = match args.get("request") {
            None | Some(Value::Null) => return Ok(None),
            Some(Value::Number(n)) => n.as_u64(),
            Some(Value::String(s)) => s.trim().trim_start_matches(['R', 'r']).trim_start_matches('-').parse().ok(),
            Some(_) => None,
        };
        let Some(id) = named else { return Err("`request` is the R-number equipe gave the person's request, e.g. 3 for R-3.".into()) };
        if !self.journal(squad).is_some_and(|j| j.requests().exists(id)) {
            return Err(format!("There is no request R-{id}. Leave `request` out: tickets you make while working on a request's ticket join it by themselves."));
        }
        Ok(Some(id))
    }

    /// The member's own session of `id`: itself, or the member a lane or a run is of.
    fn member_session(&self, id: SessionId) -> SessionId {
        self.sessions.get(&id).and_then(|s| s.run_of.or(s.lane.as_ref().map(|(of, _)| *of))).unwrap_or(id)
    }

    /// Ticket `new`, just put on the board from session `id` (its
    /// `create_ticket`), joins a request: the one `named`; else the one the
    /// ticket `id` works serves; else — from a member's own conversation, not
    /// at work on a ticket — the one for what the person last said there,
    /// made now if it is the first ticket of those words. A line for the
    /// tool's reply when it joined one.
    pub(super) fn ticket_made(&mut self, squad: &str, id: SessionId, new: TicketId, named: Option<RequestId>) -> Option<String> {
        let now = now_secs();
        let working = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        let (request, parent, link) = if let Some(request) = named {
            (request, None, Link::Linked)
        } else if let Some(on) = working {
            let request = self.journal(squad)?.requests().of_ticket(on)?;
            (request, Some(on), Link::Made)
        } else {
            let request = self.asked_in(squad, id, now)?;
            (request, None, Link::Asked)
        };
        let journal = self.journal(squad)?;
        if let Err(err) = journal.requests().link(request, new, parent, link, now) {
            crate::daemon_log!("equipe: {squad}: T-{new} wasn't put with request R-{request}: {err}");
            return None;
        }
        self.request_moved(squad, request);
        Some(format!(" It serves the person's request R-{request}: they hear where it stands without asking."))
    }

    /// The request for what the person last said to the member of `id`, in
    /// its own conversation or a lane of it — the one whose turn answers
    /// them now first, else the latest said within [`rules::ASKED_WITHIN`] —
    /// made if it isn't yet.
    fn asked_in(&mut self, squad: &str, id: SessionId, now: u64) -> Option<RequestId> {
        let member = self.member_session(id);
        let name = self.sessions.get(&member)?.name()?.to_owned();
        let mut heard: Vec<(bool, u64, SessionId)> = self
            .person_said
            .iter()
            .filter(|(at, said)| self.member_session(**at) == member && !said.text.is_empty() && now.saturating_sub(said.at) <= rules::ASKED_WITHIN)
            .map(|(&at, said)| (self.sessions.get(&at).is_some_and(|s| s.is_working() && s.answers_person), said.at, at))
            .collect();
        heard.sort();
        let (_, _, conversation) = heard.pop()?;
        let said = self.person_said.get(&conversation)?.clone();
        if let Some(request) = said.request.filter(|r| self.journals.get(squad).is_some_and(|j| j.requests().exists(*r))) {
            return Some(request);
        }
        let title = rules::title_of(&said.text);
        let request = self.journal(squad)?.requests().create(&title, &said.text, Some(conversation), Some(&name), said.at).ok()?;
        if let Some(said) = self.person_said.get_mut(&conversation) {
            said.request = Some(request);
        }
        Some(request)
    }

    /// The person put ticket `id` on the board themselves: a request of its
    /// own, its news told in the PM's conversation, where the ticket is told.
    pub(super) fn person_put_on_board(&mut self, squad: &str, id: TicketId, title: &str, owner: Option<&str>) {
        let now = now_secs();
        let pm = self.pm_of(squad);
        let Some(journal) = self.journal(squad) else { return };
        let store = journal.requests();
        let Ok(request) = store.create(&rules::title_of(title), title, pm, owner, now) else { return };
        let _ = store.link(request, id, None, Link::Asked, now);
        self.request_moved(squad, request);
    }

    /// The PM puts ticket `ticket` with request `request` (`update_ticket`'s `request`).
    pub(super) fn link_to_request(&mut self, squad: &str, ticket: TicketId, request: RequestId) -> Result<(), String> {
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        if journal.ticket(ticket).ok().flatten().is_none() {
            return Err(format!("There is no T-{ticket}."));
        }
        let was = journal.requests().of_ticket(ticket);
        journal.requests().link(request, ticket, None, Link::Linked, now_secs()).map_err(|e| e.to_string())?;
        if let Some(was) = was.filter(|w| *w != request) {
            self.request_moved(squad, was);
        }
        self.request_moved(squad, request);
        Ok(())
    }

    /// A ticket changed: the request it serves moves with it. One not
    /// serving any joins its parent's when it is a design follow-up of a
    /// request's ticket; a request's ticket brings its design QA along, as
    /// equipe's own check.
    pub(super) fn request_ticket_changed(&mut self, squad: &str, ticket: TicketId) {
        let now = now_secs();
        let Some(journal) = self.journals.get(squad) else { return };
        let store = journal.requests();
        let mut request = store.of_ticket(ticket);
        if request.is_none()
            && let Some(f) = journal.follow_up(ticket)
            && let Some(of_parent) = store.of_ticket(f.parent)
            && store.link(of_parent, ticket, Some(f.parent), Link::FollowUp, now).is_ok()
        {
            request = Some(of_parent);
        }
        let Some(request) = request else { return };
        let qa = journal.ticket(ticket).ok().flatten().and_then(|t| t.design_qa);
        if let Some(qa) = qa.filter(|qa| store.of_ticket(*qa).is_none()) {
            let _ = store.link(request, qa, Some(ticket), Link::Check, now);
        }
        self.request_moved(squad, request);
    }

    /// A question put to the person from `ticket` — or one closed: its request waits on them, or no longer.
    pub(super) fn request_question_changed(&mut self, squad: &str, ticket: TicketId) {
        if let Some(request) = self.journals.get(squad).and_then(|j| j.requests().of_ticket(ticket)) {
            self.request_moved(squad, request);
        }
    }

    /// The tickets with a question open to the person in `squad`.
    fn asking(&self, squad: &str) -> Vec<TicketId> {
        self.questions.get(squad).map(|sets| sets.iter().filter_map(|q| q.ticket).collect()).unwrap_or_default()
    }

    /// `row`'s tickets as the board has them now.
    fn row_tickets(&self, squad: &str, row: &Row) -> Vec<Ticket> {
        let Some(journal) = self.journals.get(squad) else { return Vec::new() };
        row.tickets.iter().filter_map(|l| journal.ticket(l.ticket).ok().flatten()).collect()
    }

    /// `row` as a screen gets it, with where it stands now.
    fn request_view(&self, squad: &str, row: &Row) -> Request {
        let tickets = self.row_tickets(squad, row);
        row.view(rules::status(&row.tickets, &tickets, &self.asking(squad)))
    }

    /// Request `request` moved: the screens are told where it stands, and
    /// news of it waits [`rules::SETTLE`] to be told, unless it waits already.
    fn request_moved(&mut self, squad: &str, request: RequestId) {
        let Some(row) = self.journals.get(squad).and_then(|j| j.requests().get(request)) else { return };
        let tickets = self.row_tickets(squad, &row);
        let asking = self.asking(squad);
        let status = rules::status(&row.tickets, &tickets, &asking);
        if rules::news(&row.told, &row.tickets, &tickets, &asking, &status).is_some()
            && let Some(journal) = self.journals.get(squad)
        {
            let _ = journal.requests().set_due(request, now_secs() + rules::settle());
        }
        self.publish(Event::RequestChanged { squad: squad.to_owned(), request: row.view(status) });
    }

    /// The requests a screen lists: those under way, and those finished in
    /// the last day, the latest first.
    pub(super) fn requests_listed(&self, squad: &str) -> Vec<Request> {
        let Some(journal) = self.journals.get(squad) else { return Vec::new() };
        let since = now_secs().saturating_sub(rules::LISTED_DONE);
        journal.requests().listed(since).iter().map(|row| self.request_view(squad, row)).collect()
    }

    /// The person's open requests asked in conversation `id`.
    fn requests_in(&self, squad: &str, id: SessionId) -> Vec<Request> {
        self.requests_listed(squad).into_iter().filter(|r| r.session == Some(id) && r.closed_at.is_none()).collect()
    }

    /// What a member's turn in conversation `id` starts with: where each of
    /// the person's open requests there stands.
    pub(super) fn requests_context(&self, id: SessionId) -> Option<String> {
        let squad = self.sessions.get(&id)?.squad.clone()?;
        rules::context(&self.requests_in(&squad, id), now_secs())
    }

    /// The person wrote `text` to conversation `id` while its member works:
    /// when it asks after a request of theirs there, equipe answers at once
    /// from the board, marked as its own answer — the member reads the
    /// message at its next step all the same.
    pub(super) fn answer_from_board(&mut self, id: SessionId, text: &str) {
        let Some(session) = self.sessions.get(&id).filter(|s| s.is_working()) else { return };
        let (Some(squad), Some(name)) = (session.squad.clone(), session.name().map(str::to_owned)) else { return };
        let open = self.requests_in(&squad, id);
        let now = now_secs();
        let lines: Vec<String> = rules::asked_after(text, &open)
            .into_iter()
            .take(3)
            .map(|r| {
                let line = rules::line(&r.status, crate::i18n::core(self.config.lang), &|at| rules::clock(at, now));
                say!(self, "request.answer", name = name, title = r.title, line = line)
            })
            .collect();
        for text in lines {
            self.publish(Event::Notice { session: id, text, undo: None });
        }
    }

    /// Each minute: news of `squad`'s requests whose changes have settled is
    /// told — once per milestone, in one line, in the conversation each was
    /// asked in (else its member's own), with a notification on the screens.
    pub(super) fn tell_request_news(&mut self, squad: &str) {
        let now = now_secs();
        let Some(due) = self.journals.get(squad).map(|j| j.requests().due(now)) else { return };
        for row in due {
            let tickets = self.row_tickets(squad, &row);
            let asking = self.asking(squad);
            let status = rules::status(&row.tickets, &tickets, &asking);
            let Some((milestone, told)) = rules::news(&row.told, &row.tickets, &tickets, &asking, &status) else {
                // What moved settled back: nothing to tell.
                if let Some(journal) = self.journals.get(squad) {
                    let _ = journal.requests().told(row.id, &row.told, None);
                }
                continue;
            };
            let closed = status.done.then_some(now);
            if let Some(journal) = self.journals.get(squad)
                && let Err(err) = journal.requests().told(row.id, &told, closed)
            {
                crate::daemon_log!("equipe: {squad}: request R-{}'s news wasn't kept, so it isn't told: {err}", row.id);
                continue;
            }
            let line = rules::line(&status, crate::i18n::core(self.config.lang), &|at| rules::clock(at, now));
            let to = row
                .session
                .filter(|s| self.sessions.contains_key(s))
                .or_else(|| row.member.as_deref().and_then(|m| self.member_id(squad, m)));
            if let Some(session) = to {
                let text = say!(self, "request.news", title = row.title, line = line);
                self.publish(Event::Notice { session, text, undo: None });
            }
            self.publish(Event::RequestNews {
                squad: squad.to_owned(),
                request: row.id,
                title: row.title.clone(),
                session: to,
                member: row.member.clone(),
                milestone,
                line,
            });
            let row = Row { told, closed_at: closed.or(row.closed_at), due_at: None, ..row };
            self.publish(Event::RequestChanged { squad: squad.to_owned(), request: row.view(status) });
        }
    }
}
