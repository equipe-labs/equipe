//! The person's requests (`crate::requests` has the rules). A request is
//! recorded when the person asks a member for something in a conversation —
//! or when they put a ticket on the board themselves. A ticket a member puts
//! on the board from that conversation in answer serves it: the member names
//! which (`request`) whenever more than one could be it, and is asked to
//! when it doesn't. Tickets made while working on its tickets join it by
//! their lineage (made during a turn on one, a design follow-up of one, its
//! design QA), and follow their parent when it moves. Only work covers a
//! request: one no work ticket serves a few minutes after it was asked is
//! put to the PM and shown as such, and it closes only when its work is done
//! — or a member closes it with a reason the person sees. Where each stands
//! goes to the screens whenever a ticket of it moves; a milestone — done,
//! live, waiting on the person, stopped, all done — is told once, in one
//! line, in the conversation it was asked in, a couple of minutes after the
//! first change so what moves together is said together; a member's turn
//! there starts with where each stands; and while the member is busy, a
//! message asking after one is answered from the board at once.

use equipe_protocol::requests::{Link, Request, RequestId};

use super::*;
use crate::requests::{self as rules, NO_REQUEST, Row};

/// How equipe's and the platform's own messages to a member begin: never the person's words.
const PLATFORM_HEADS: [&str; 6] = ["[equipe monitor]", "[equipe:", "[Routine · ", "[Idle · ", "[Signal · ", crate::retro::FACTS_HEAD];

/// The person's own words in `text`: without a hosted team's leading
/// "[@handle · Name · role]" mark; `None` when nothing is left, or when it is
/// one of equipe's or the platform's own messages.
fn persons_words(text: &str) -> Option<&str> {
    let mut words = text.trim_start();
    if let Some(rest) = words.strip_prefix("[@")
        && let Some(end) = rest.find(']')
        && !rest[..end].contains('\n')
    {
        words = rest[end + 1..].trim_start();
    }
    let words = words.trim_end();
    (!words.is_empty() && !PLATFORM_HEADS.iter().any(|head| words.starts_with(head))).then_some(words)
}

/// What the person last said in a conversation.
#[derive(Debug, Clone)]
pub(super) struct Said {
    text: String,
    at: u64,
    /// Their latest words there, whatever they were — asking after a request too.
    last: (String, u64),
}

/// The request a ticket about to be put on the board serves: which, the
/// ticket of it the new one came of, and how it joins. `None`: none.
pub(super) type Serving = Option<(RequestId, Option<TicketId>, Link)>;

impl Runtime {
    /// The person said `text` in conversation `session` (a member's, or a
    /// lane of it). Words asking after a request of theirs there aren't a new
    /// one, nor is an acknowledgement or chit-chat ([`rules::small_talk`]);
    /// anything else they ask there is, from now on — so two things asked
    /// minutes apart are two requests, each a ticket must serve.
    ///
    /// What equipe or the platform said (a nudge, a routine, a monitor's
    /// line) is never the person's request; a hosted team's "[@who · Name ·
    /// role]" mark is the platform's, kept off the request's words.
    pub(super) fn person_said(&mut self, session: SessionId, text: &str) {
        let Some(squad) = self.sessions.get(&session).filter(|s| s.member.is_some()).and_then(|s| s.squad.clone()) else { return };
        if self.has_platform_said(text) {
            return;
        }
        let Some(text) = persons_words(text) else { return };
        let now = now_secs();
        let asks_after = !rules::asked_after(text, &self.requests_in(&squad, session)).is_empty();
        // "ok", "thanks!", an emoji: heard, but nothing to put on the board — no request for the PM to close.
        if !asks_after && !rules::small_talk(text) {
            self.record_request(&squad, session, text, now);
        }
        let said = self.person_said.entry(session).or_insert_with(|| Said { text: String::new(), at: now, last: (String::new(), now) });
        said.last = (text.to_owned(), now);
        if !asks_after {
            said.text = text.to_owned();
            said.at = now;
        }
    }

    /// The person's words `text` in conversation `session`, kept as a request
    /// — unless they are about a ticket already on the board ("T-12: …",
    /// "merge T-3"), or said beside a ticket's run, whose words are about that
    /// ticket, or in a design session's rounds, whose words steer its prototype.
    fn record_request(&mut self, squad: &str, session: SessionId, text: &str, now: u64) -> Option<RequestId> {
        if self.sessions.get(&session)?.work.is_some() || self.design_sessions.get(&session).is_some_and(|a| a.rounds(now)) {
            return None;
        }
        let name = self.sessions.get(&self.member_session(session))?.name()?.to_owned();
        let journal = self.journal(squad)?;
        if rules::ticket_numbers(text).into_iter().any(|t| journal.ticket(t).ok().flatten().is_some()) {
            return None;
        }
        match journal.requests().create(&rules::title_of(text), text, Some(session), Some(&name), now, true) {
            Ok(request) => Some(request),
            Err(err) => {
                crate::daemon_log!("equipe: {squad}: what the person asked {name} wasn't kept as a request: {err}");
                None
            }
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

    /// `request`, as a member names it on a ticket (`R-3` or `3`; `0`, none
    /// of them: [`NO_REQUEST`]): checked before the ticket is made.
    /// `Ok(None)` when it names none.
    pub(super) fn request_arg(&mut self, squad: &str, args: &Value) -> Result<Option<RequestId>, String> {
        let named = match args.get("request") {
            None | Some(Value::Null) => return Ok(None),
            Some(Value::Number(n)) => n.as_u64(),
            Some(Value::String(s)) if s.trim().eq_ignore_ascii_case("none") => Some(NO_REQUEST),
            Some(Value::String(s)) => s.trim().trim_start_matches(['R', 'r']).trim_start_matches('-').parse().ok(),
            Some(_) => None,
        };
        let Some(id) = named else { return Err("`request` is the R-number equipe gave the person's request, e.g. 3 for R-3 (0 for none).".into()) };
        if id == NO_REQUEST {
            return Ok(Some(id));
        }
        if !self.journal(squad).is_some_and(|j| j.requests().exists(id)) {
            return Err(format!("There is no request R-{id}. Leave `request` out: tickets you make while working on a request's ticket join it by themselves."));
        }
        Ok(Some(id))
    }

    /// The member's own session of `id`: itself, or the member a lane or a run is of.
    fn member_session(&self, id: SessionId) -> SessionId {
        self.sessions.get(&id).and_then(|s| s.run_of.or(s.lane.as_ref().map(|(of, _)| *of))).unwrap_or(id)
    }

    /// The request a ticket session `id` is about to put on the board serves
    /// — checked before it is made. The one `named` (none for
    /// [`NO_REQUEST`]); else the one the ticket `id` works serves; else —
    /// from a member's own conversation — the one request of the person's
    /// that it could answer there. When more than one could be it, the
    /// member is asked which, each in one line: a ticket is never put with
    /// whichever was asked last.
    pub(super) fn serving(&self, squad: &str, id: SessionId, named: Option<RequestId>) -> Result<Serving, String> {
        let Some(journal) = self.journals.get(squad) else { return Ok(None) };
        let store = journal.requests();
        let working = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        Ok(match (named, working) {
            (Some(NO_REQUEST), _) => None,
            (Some(request), Some(on)) if store.of_ticket(on) == Some(request) => Some((request, Some(on), Link::Made)),
            (Some(request), Some(_)) => Some((request, None, Link::Linked)),
            // Put on the board in answer to the person's words, named.
            (Some(request), None) => Some((request, None, Link::Asked)),
            (None, Some(on)) => store.of_ticket(on).map(|request| (request, Some(on), Link::Made)),
            (None, None) => {
                let now = now_secs();
                let open = self.answerable(squad, id, now);
                match open.as_slice() {
                    [] => None,
                    [(only, true)] => Some((only.id, None, Link::Asked)),
                    _ => return Err(rules::which_request(&open.into_iter().map(|(r, _)| r).collect::<Vec<_>>(), now)),
                }
            }
        })
    }

    /// The person's open requests a ticket put on the board from session
    /// `id` could answer: asked in any conversation of its member, not all
    /// done yet, and asked within [`rules::ASKED_WITHIN`] (`true`) or still
    /// without a ticket covering it (`false`) — the oldest first.
    fn answerable(&self, squad: &str, id: SessionId, now: u64) -> Vec<(Request, bool)> {
        let Some(journal) = self.journals.get(squad) else { return Vec::new() };
        let member = self.member_session(id);
        let asking = self.asking(squad);
        let shipped = self.shipped(squad);
        journal
            .requests()
            .open()
            .into_iter()
            .filter(|row| row.said && row.session.is_some_and(|s| self.member_session(s) == member))
            .filter_map(|row| {
                let tickets = self.row_tickets(squad, &row);
                let status = rules::standing(&row, &tickets, &asking, &shipped, now);
                let recent = now.saturating_sub(row.asked_at) <= rules::ASKED_WITHIN;
                let uncovered = rules::covering(&row.tickets, &tickets).is_empty();
                (!status.done && (recent || uncovered)).then(|| (row.view(status), recent))
            })
            .collect()
    }

    /// The product a ticket is for ([`Ticket::app`]) by what it came of: the
    /// app the person's request `request` was written about, else that of
    /// ticket `parent` (the one it was made while working, or a follow-up of).
    pub(super) fn app_for(&self, squad: &str, request: Option<RequestId>, parent: Option<TicketId>) -> Option<String> {
        let journal = self.journals.get(squad)?;
        request
            .and_then(|r| journal.requests().get(r))
            .and_then(|r| rules::app_of(&r.words))
            .or_else(|| parent.and_then(|p| journal.ticket(p).ok().flatten()).and_then(|t| t.app))
    }

    /// Ticket `ticket` joined request `request` (of `parent`): the product it
    /// is for, when it had none, from them ([`Runtime::app_for`]).
    fn fill_app(&self, squad: &str, ticket: TicketId, request: Option<RequestId>, parent: Option<TicketId>) {
        let Some(app) = self.app_for(squad, request, parent) else { return };
        if let Some(journal) = self.journals.get(squad) {
            let _ = journal.set_ticket_app(ticket, &app);
        }
    }

    /// Ticket `new`, just put on the board, joins the request [`Runtime::serving`]
    /// found for it. A line for the tool's reply when it joined one, naming it
    /// in the person's words.
    pub(super) fn ticket_made(&mut self, squad: &str, new: TicketId, serving: Serving) -> Option<String> {
        let (request, parent, link) = serving?;
        let journal = self.journal(squad)?;
        if let Err(err) = journal.requests().link(request, new, parent, link, now_secs()) {
            crate::daemon_log!("equipe: {squad}: T-{new} wasn't put with request R-{request}: {err}");
            return None;
        }
        let title = journal.requests().get(request).map(|r| r.title).unwrap_or_default();
        self.request_moved(squad, request);
        Some(format!(" It serves the person's request R-{request} “{title}”: they hear where it stands without asking."))
    }

    /// The person put ticket `id` on the board themselves: a request of its
    /// own, its news told in the PM's conversation, where the ticket is told.
    pub(super) fn person_put_on_board(&mut self, squad: &str, id: TicketId, title: &str, owner: Option<&str>) {
        let now = now_secs();
        let pm = self.pm_of(squad);
        let Some(journal) = self.journal(squad) else { return };
        let store = journal.requests();
        let Ok(request) = store.create(&rules::title_of(title), title, pm, owner, now, false) else { return };
        let _ = store.link(request, id, None, Link::Asked, now);
        self.request_moved(squad, request);
    }

    /// The PM puts ticket `ticket` with request `request` (`update_ticket`'s
    /// `request`; [`NO_REQUEST`]: with none) — and what came of it, with it.
    pub(super) fn link_to_request(&mut self, squad: &str, ticket: TicketId, request: RequestId) -> Result<(), String> {
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        if journal.ticket(ticket).ok().flatten().is_none() {
            return Err(format!("There is no T-{ticket}."));
        }
        let was = journal.requests().of_ticket(ticket);
        if request == NO_REQUEST {
            journal.requests().unlink(ticket).map_err(|e| e.to_string())?;
            if let Some(was) = was {
                self.request_moved(squad, was);
            }
            return Ok(());
        }
        journal.requests().link(request, ticket, None, Link::Linked, now_secs()).map_err(|e| e.to_string())?;
        self.fill_app(squad, ticket, Some(request), None);
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
            self.fill_app(squad, ticket, request, Some(f.parent));
        }
        let Some(request) = request else { return };
        let qa = journal.ticket(ticket).ok().flatten().and_then(|t| t.design_qa);
        if let Some(qa) = qa.filter(|qa| store.of_ticket(*qa).is_none()) {
            let _ = store.link(request, qa, Some(ticket), Link::Check, now);
            self.fill_app(squad, qa, Some(request), Some(ticket));
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
        row.view(rules::standing(row, &tickets, &self.asking(squad), &self.shipped(squad), now_secs()))
    }

    /// Request `request` moved: the screens are told where it stands, once
    /// they list it, and news of it waits [`rules::SETTLE`] to be told,
    /// unless it waits already.
    fn request_moved(&mut self, squad: &str, request: RequestId) {
        let Some(row) = self.journals.get(squad).and_then(|j| j.requests().get(request)) else { return };
        let tickets = self.row_tickets(squad, &row);
        let asking = self.asking(squad);
        let shipped = self.shipped(squad);
        let status = rules::standing(&row, &tickets, &asking, &shipped, now_secs());
        if rules::news(&row.told, &row.tickets, &tickets, &asking, &shipped, &status).is_some()
            && let Some(journal) = self.journals.get(squad)
        {
            let _ = journal.requests().set_due(request, now_secs() + rules::settle());
        }
        if row.shown() {
            self.publish(Event::RequestChanged { squad: squad.to_owned(), request: row.view(status) });
        }
    }

    /// The requests a screen lists: those under way, and those finished in
    /// the last day, the latest first — once a ticket is with them, or they
    /// were put to the PM as uncovered ([`Row::shown`]).
    pub(super) fn requests_listed(&self, squad: &str) -> Vec<Request> {
        let Some(journal) = self.journals.get(squad) else { return Vec::new() };
        let since = now_secs().saturating_sub(rules::LISTED_DONE);
        journal.requests().listed(since).iter().filter(|row| row.shown()).map(|row| self.request_view(squad, row)).collect()
    }

    /// The person's open requests asked in conversation `id`, listed or not yet.
    fn requests_in(&self, squad: &str, id: SessionId) -> Vec<Request> {
        let Some(journal) = self.journals.get(squad) else { return Vec::new() };
        journal.requests().open().iter().filter(|row| row.session == Some(id)).map(|row| self.request_view(squad, row)).collect()
    }

    /// What a member's turn in conversation `id` starts with: where each of
    /// the person's open requests there stands — and, for the PM, those asked
    /// of other members that no ticket covers yet.
    pub(super) fn requests_context(&self, id: SessionId) -> Option<String> {
        let squad = self.sessions.get(&id)?.squad.clone()?;
        let elsewhere: Vec<Request> = if self.pm_of(&squad) == Some(self.member_session(id)) {
            let journal = self.journals.get(&squad)?;
            journal.requests().open().iter().filter(|row| row.session != Some(id)).map(|row| self.request_view(&squad, row)).filter(|r| r.status.uncovered).collect()
        } else {
            Vec::new()
        };
        let context = rules::context(&self.requests_in(&squad, id), &elsewhere, now_secs());
        // What each of the person's requests cost (`scope_gate`: about, from the work's tokens): the PM answers with it only
        // when they ask what something cost, and never volunteers it.
        let costs = if self.pm_of(&squad) == Some(self.member_session(id)) { self.request_costs(&squad) } else { None };
        match (context, costs) {
            (Some(c), Some(k)) => Some(format!("{c}\n{k}")),
            (c, k) => c.or(k),
        }
    }

    /// What each of the person's requests listed now has cost so far, in one bracketed line for the PM.
    fn request_costs(&self, squad: &str) -> Option<String> {
        let journal = self.journals.get(squad)?;
        let since = now_secs().saturating_sub(rules::LISTED_DONE);
        let rows = journal.requests().listed(since);
        let ids: Vec<TicketId> = rows.iter().flat_map(|r| r.tickets.iter().map(|l| l.ticket)).collect();
        let spent = journal.ticket_spend(&ids).ok()?;
        let lines: Vec<String> = rows
            .iter()
            .filter_map(|r| {
                let tokens: u64 = r.tickets.iter().filter_map(|l| spent.get(&l.ticket)).map(|(_, t)| *t).sum();
                (tokens > 0).then(|| format!("R-{} “{}”: about {}{}", r.id, r.title, crate::scope_gate::said(crate::scope_gate::usd(tokens)), if r.closed_at.is_some() { "" } else { " so far" }))
            })
            .take(12)
            .collect();
        (!lines.is_empty()).then(|| format!("[equipe: what the team's work on each request has cost, the team's own bookkeeping: never tell the person unless they ask what something cost, then in one short line: {}]", lines.join("; ")))
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
            let shipped = self.shipped(squad);
            let status = rules::standing(&row, &tickets, &asking, &shipped, now);
            let Some((milestone, told)) = rules::news(&row.told, &row.tickets, &tickets, &asking, &shipped, &status) else {
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

impl Runtime {
    /// Each minute: the person's open requests that no work ticket covers
    /// [`rules::cover_wait`] after they were asked are put to the PM — once
    /// each, all in one message — and listed for the person as such. Until
    /// one covers them they stay open, and in the PM's turns.
    pub(super) async fn surface_uncovered(&mut self, squad: &str) {
        let now = now_secs();
        let Some(rows) = self.journals.get(squad).map(|j| j.requests().open()) else { return };
        let asking = self.asking(squad);
        let mut lines = Vec::new();
        for row in rows.into_iter().filter(|r| r.surfaced_at.is_none()) {
            let tickets = self.row_tickets(squad, &row);
            let status = rules::standing(&row, &tickets, &asking, &self.shipped(squad), now);
            if !status.uncovered {
                continue;
            }
            // Answered with a question for the person (a link that needs their sign-in, "what do you do?"): it waits
            // on them, not on the board. Nudged, the member only said the same again (my-team-qjba, 2026-11-26).
            if self.asked_back(squad, &row) {
                continue;
            }
            if let Some(journal) = self.journals.get(squad)
                && let Err(err) = journal.requests().surfaced(row.id, now)
            {
                crate::daemon_log!("equipe: {squad}: request R-{} is uncovered, but that wasn't kept: {err}", row.id);
                continue;
            }
            lines.push(format!(
                "- R-{} — asked of {} at {}: “{}”",
                row.id,
                row.member.as_deref().unwrap_or("the squad"),
                rules::clock(row.asked_at, now),
                rules::words_line(&row.words)
            ));
            let row = Row { surfaced_at: Some(now), ..row };
            self.publish(Event::RequestChanged { squad: squad.to_owned(), request: row.view(status) });
        }
        if lines.is_empty() {
            return;
        }
        crate::daemon_log!("equipe: {squad}: {} of the person's requests have no ticket serving them; the PM is told", lines.len());
        let Some(pm) = self.pm_of(squad) else { return };
        let minutes = rules::cover_wait() / 60;
        let text = format!(
            "[Uncovered requests] No ticket serves these requests of the person's, {minutes} minutes or more after they asked:\n{}\n\n\
Settle each first: put its work on the board (`create_ticket` with its number as `request`, or `update_ticket` with `request` \
on a ticket that already does it) — or, when it asks for no work, `close_request` with why, which the person sees. Until then \
it stays open, shown to the person as not on the board. Settling it is bookkeeping: don't write to the person again to repeat \
what you already told them.",
            lines.join("\n")
        );
        self.hand_to_pm(squad, pm, text).await;
    }

    /// Whether the member `row` was asked of answered it with a question for
    /// the person, in the conversation it was asked in, after it was asked.
    fn asked_back(&self, squad: &str, row: &Row) -> bool {
        let (Some(session), Some(journal)) = (row.session, self.journals.get(squad)) else { return false };
        let Ok((entries, at)) = journal.recent_at(session, 60) else { return false };
        entries.iter().zip(at).any(|(entry, at)| matches!(entry, Entry::Reply { text } if at.is_some_and(|at| at >= row.asked_at) && rules::asks_person(text)))
    }

    /// `close_request`: the PM — or the member it was asked of — closes one
    /// of the person's requests with its work not done, saying why; the
    /// person is told in its conversation.
    pub(super) fn close_request_tool(&mut self, call: ToolCall, reply: oneshot::Sender<equipe_protocol::rpc::ToolResult>) {
        let answer = self.close_request(&call);
        let _ = reply.send(match answer {
            Ok(text) => equipe_protocol::rpc::ToolResult::ok(text),
            Err(text) => equipe_protocol::rpc::ToolResult::error(text),
        });
    }

    fn close_request(&mut self, call: &ToolCall) -> Result<String, String> {
        let squad = call.squad.clone();
        let member = call.member.clone();
        let id = self.caller_id(call).ok_or_else(|| format!("{member} isn't on the squad {squad}."))?;
        let args = &call.arguments;
        let request = self.request_arg(&squad, args)?.filter(|r| *r != NO_REQUEST).ok_or("Which request? Give its number as `request` (3 for R-3).")?;
        let why = board::arg_str(args, "why").map(str::trim).filter(|w| !w.is_empty()).ok_or("Say `why` in a few words, in the person's language: they see it.")?.to_owned();
        let is_pm = self.sessions.get(&self.member_session(id)).is_some_and(Session::is_pm);
        let journal = self.journal(&squad).ok_or("The board can't be opened.")?;
        let row = journal.requests().get(request).ok_or_else(|| format!("There is no request R-{request}."))?;
        if row.closed_at.is_some() {
            return Err(format!("R-{request} is closed already."));
        }
        if !is_pm && row.member.as_deref() != Some(member.as_str()) {
            return Err(format!("R-{request} was asked of {}: only they or the PM close it.", row.member.as_deref().unwrap_or("the squad")));
        }
        let now = now_secs();
        journal.requests().close(request, &why, now).map_err(|e| e.to_string())?;
        let row = journal.requests().get(request).ok_or("The request can't be read back.")?;
        let tickets = self.row_tickets(&squad, &row);
        let status = rules::standing(&row, &tickets, &self.asking(&squad), &self.shipped(&squad), now);
        let line = rules::line(&status, crate::i18n::core(self.config.lang), &|at| rules::clock(at, now));
        crate::daemon_log!("equipe: {squad}: {member} closed request R-{request}: {why}");
        let to = row.session.filter(|s| self.sessions.contains_key(s)).or_else(|| row.member.as_deref().and_then(|m| self.member_id(&squad, m)));
        if let Some(session) = to {
            let text = say!(self, "request.news", title = row.title, line = line);
            self.publish(Event::Notice { session, text, undo: None });
        }
        if row.shown() {
            self.publish(Event::RequestChanged { squad: squad.clone(), request: row.view(status) });
        }
        let open: Vec<String> = tickets.iter().filter(|t| !t.status.is_closed()).map(|t| format!("T-{}", t.id)).collect();
        let left = if open.is_empty() { String::new() } else { format!(" Its open tickets ({}) stay on the board: cancel them if they go too.", open.join(", ")) };
        Ok(format!("Closed R-{request}. The person is told: “{line}”.{left}"))
    }
}

impl Runtime {
    /// `depends_on` checked for ticket `ticket` (`None`: being made, to serve
    /// what `serves` says — `Some(None)`: no request yet), with words `text`: a ticket of the person's request
    /// isn't chained behind open work outside it unless `depends_why` says
    /// why; a dependency whose places don't overlap is pointed out. The note
    /// for the reply.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn check_depends(
        &mut self,
        squad: &str,
        ticket: Option<TicketId>,
        serves: Option<Option<RequestId>>,
        title: &str,
        text: &str,
        deps: &[TicketId],
        args: &Value,
    ) -> Result<String, String> {
        if deps.is_empty() {
            return Ok(String::new());
        }
        self.no_wait_on_main_red(squad, deps)?;
        let why = board::arg_str(args, "depends_why").map(str::trim).filter(|w| !w.is_empty());
        let Some(journal) = self.journals.get(squad) else { return Ok(String::new()) };
        let serves = match ticket {
            Some(t) => journal.requests().of_ticket(t).map(Some),
            None => serves,
        };
        let mut note = String::new();
        for dep in deps {
            let Some(d) = journal.ticket(*dep).ok().flatten() else { continue };
            if let (Some(serves), None) = (serves, why)
                && let Some(refusal) = rules::chain_refusal(ticket, serves, &d, journal.requests().of_ticket(*dep))
            {
                return Err(refusal);
            }
            if why.is_none()
                && let Some(warning) = rules::overlap_warning(&format!("{title} {text}"), &d)
            {
                note.push_str(&warning);
            }
        }
        Ok(note)
    }

    /// `unblock`: a member takes dependencies off its own ticket, not started
    /// yet, saying why — logged, the PM told — and it can start; otherwise
    /// the PM is asked to, at its next turn, as something to settle first.
    pub(super) async fn unblock_tool(&mut self, call: ToolCall, reply: oneshot::Sender<equipe_protocol::rpc::ToolResult>) {
        let answer = self.unblock(&call).await;
        let _ = reply.send(match answer {
            Ok(text) => equipe_protocol::rpc::ToolResult::ok(text),
            Err(text) => equipe_protocol::rpc::ToolResult::error(text),
        });
    }

    async fn unblock(&mut self, call: &ToolCall) -> Result<String, String> {
        let squad = call.squad.clone();
        let member = call.member.clone();
        let id = self.caller_id(call).ok_or_else(|| format!("{member} isn't on the squad {squad}."))?;
        let args = &call.arguments;
        let ticket_id = board::arg_id(args, "id").ok_or("Which ticket waits? Give its id.")?;
        let why = board::arg_str(args, "why").map(str::trim).filter(|w| !w.is_empty()).ok_or("Say `why` its work doesn't need them: what each touches.")?.to_owned();
        let mut ticket = self.find_ticket(&squad, ticket_id).ok_or_else(|| format!("There is no T-{ticket_id}."))?;
        let on: Vec<TicketId> = match board::arg_ids(args, "on") {
            Some(ids) => ids.into_iter().filter(|d| ticket.depends_on.contains(d)).collect(),
            None => ticket.depends_on.clone(),
        };
        if on.is_empty() {
            return Err(format!("T-{ticket_id} doesn't wait on those (it waits on {}).", if ticket.depends_on.is_empty() { "nothing".to_owned() } else { crate::by_words::list(&ticket.depends_on) }));
        }
        let listed = crate::by_words::list(&on);
        let role = self.sessions.get(&self.member_session(id)).and_then(|s| s.member.as_ref().map(|m| m.role.clone())).unwrap_or_default();
        let is_pm = self.sessions.get(&self.member_session(id)).is_some_and(Session::is_pm);
        let own = ticket.assignee.as_deref() == Some(member.as_str())
            || ticket.owner.as_deref() == Some(member.as_str())
            || (ticket.owner.as_deref() == Some(role.as_str()) && ticket.assignee.as_deref().is_none_or(|a| a == member));
        let not_started = matches!(ticket.status, TicketStatus::Todo | TicketStatus::Proposed) && ticket.branch.is_none();
        if (own || is_pm) && not_started {
            ticket.depends_on.retain(|d| !on.contains(d));
            let journal = self.journal(&squad).ok_or("The board can't be opened.")?;
            journal.save_ticket(&ticket).map_err(|e| e.to_string())?;
            let _ = journal.ticket_event(ticket_id, &member, "dependency dropped", &format!("{listed}: {why}"));
            self.publish(Event::TicketChanged { squad: squad.clone(), ticket });
            if !is_pm {
                let line = format!("{member} took {listed} off T-{ticket_id}'s depends_on: {why} — put it back with update_ticket if it truly must wait.");
                let shown = say!(self, "news.unblocked", name = member, id = ticket_id, deps = listed);
                self.brief_pm(&squad, line, Some(shown));
            }
            self.dispatch(&squad).await;
            return Ok(format!("T-{ticket_id} no longer waits on {listed}; it starts when its owner is free.{}", if is_pm { "" } else { " The PM is told." }));
        }
        let Some(pm) = self.pm_of(&squad) else { return Err("There's no PM to ask.".into()) };
        let because = if own { format!("it has started ({})", ticket.status.label()) } else { "it isn't yours".to_owned() };
        let text = format!(
            "[Unblock · {member}] {member} asks you to unblock T-{ticket_id} ({}): it waits on {listed}. Its word: {why}\n\nSettle this \
first: if the work overlaps nothing there, drop the dependency now (update_ticket with depends_on); if it must wait, tell \
{member} why in a comment on T-{ticket_id}.",
            ticket.title
        );
        self.hand_to_pm(&squad, pm, text).await;
        Ok(format!("Asked the PM to unblock T-{ticket_id} ({because}): it takes it up at its next turn. Go on with other work meanwhile."))
    }
}


#[cfg(test)]
mod said_tests {
    use super::persons_words;

    #[test]
    fn the_platforms_mark_and_its_own_nudges_are_never_the_persons_request() {
        assert_eq!(persons_words("[@dana · Dana Kim · admin] add dark mode"), Some("add dark mode"));
        assert_eq!(persons_words("  add dark mode \n"), Some("add dark mode"));
        assert_eq!(persons_words("[@dana · Dana Kim · admin]   "), None);
        assert_eq!(persons_words("[Routine · Weekly report]\n\nSum up the week"), None);
        assert_eq!(persons_words("[Idle · 24h]\n\nThe board is empty"), None);
        assert_eq!(persons_words("[equipe monitor] the team looks stuck"), None);
        assert_eq!(persons_words("[@dana · Dana Kim · admin] [Signal · uptime] down"), None, "a mark can't make a nudge the person's");
        assert_eq!(persons_words("[link](https://x.y) looks broken"), Some("[link](https://x.y) looks broken"));
    }
}
