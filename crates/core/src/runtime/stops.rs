//! The kinds of work the person stopped (`crate::stops` has the rules).
//! "Stop this" on a request or a ticket — or the person's words to a member,
//! who calls `stop_work` — cancels the open tickets of its chain and keeps a
//! stop in force for the squad: a member's new ticket of that kind (made
//! while working the chain, serving its request, named by its terms or
//! paths) is turned away with why, and a design review's fixes of it are not
//! filed, until the person lifts it. Every member's turn starts with the
//! stops in force; the screens list them under "Your requests".

use equipe_protocol::requests::{Link, RequestId, Stop, StopId};
use equipe_protocol::rpc::ToolResult;

use super::*;
use crate::stops as rules;

impl Runtime {
    /// `squad`'s stops in force, the oldest first.
    pub(super) fn stops_in_force(&self, squad: &str) -> Vec<Stop> {
        self.journals.get(squad).map(|j| j.stops().in_force()).unwrap_or_default()
    }

    fn publish_stops(&mut self, squad: &str) {
        let stops = self.stops_in_force(squad);
        self.publish(Event::StopsChanged { squad: squad.to_owned(), stops });
    }

    /// What every member's turn starts with while stops are in force.
    pub(super) fn stops_context(&self, squad: &str) -> Option<String> {
        rules::context(&self.stops_in_force(squad))
    }

    /// The chain a stop is made on: every ticket of `request`; or `ticket`
    /// with the rest of its part of a request when it serves one; each with
    /// the design follow-ups that came of it.
    fn stop_chain(&self, squad: &str, request: Option<RequestId>, ticket: Option<TicketId>) -> (Vec<TicketId>, Vec<TicketId>) {
        let Some(journal) = self.journals.get(squad) else { return (Vec::new(), Vec::new()) };
        let row = request.or_else(|| ticket.and_then(|t| journal.requests().of_ticket(t))).and_then(|r| journal.requests().get(r));
        let mut chain: Vec<TicketId> = Vec::new();
        let mut checks: Vec<TicketId> = Vec::new();
        match (&row, request, ticket) {
            (Some(row), Some(_), _) => chain.extend(row.tickets.iter().map(|l| l.ticket)),
            (Some(row), None, Some(t)) => {
                let part = crate::requests::part_of(&row.tickets, t);
                chain.extend(row.tickets.iter().filter(|l| crate::requests::part_of(&row.tickets, l.ticket) == part).map(|l| l.ticket));
            }
            (None, _, Some(t)) => chain.push(t),
            _ => {}
        }
        if let Some(row) = &row {
            checks.extend(row.tickets.iter().filter(|l| l.link == Link::Check).map(|l| l.ticket));
        }
        for t in chain.clone() {
            for f in journal.stops().follow_ups_of(t) {
                if !chain.contains(&f) {
                    chain.push(f);
                }
            }
        }
        (chain, checks)
    }

    /// Stop a kind of work: its chain's open tickets cancelled, a stop kept,
    /// the PM told, the screens shown. `terms` left empty are taken from the
    /// chain's titles (else the words).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn stop_work(
        &mut self,
        squad: &str,
        by: &str,
        words: &str,
        terms: Vec<String>,
        paths: Vec<String>,
        request: Option<RequestId>,
        ticket: Option<TicketId>,
    ) -> Result<Stop, String> {
        let (chain, checks) = self.stop_chain(squad, request, ticket);
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        let tickets: Vec<Ticket> = chain.iter().filter_map(|t| journal.ticket(*t).ok().flatten()).collect();
        let words = match words.trim() {
            "" => request
                .and_then(|r| journal.requests().get(r).map(|row| row.title))
                .or_else(|| ticket.and_then(|t| tickets.iter().find(|x| x.id == t).map(|x| x.title.clone())))
                .ok_or("Say what to stop.")?,
            words => words.to_owned(),
        };
        let terms: Vec<String> = terms.into_iter().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()).collect();
        let terms = if !terms.is_empty() || !paths.is_empty() {
            terms
        } else {
            let titles: Vec<&str> = tickets.iter().filter(|t| !checks.contains(&t.id)).map(|t| t.title.as_str()).collect();
            if titles.is_empty() { rules::terms_of(&[words.as_str()]) } else { rules::terms_of(&titles) }
        };
        let mut stop = Stop { id: 0, words, terms, paths, request, tickets: chain, by: by.to_owned(), since: now_secs(), cancelled: Vec::new(), refused: 0 };
        stop.id = journal.stops().create(&stop).map_err(|e| e.to_string())?;
        // Its open tickets go now: whoever works one stops.
        let open: Vec<TicketId> = tickets.iter().filter(|t| !t.status.is_closed()).map(|t| t.id).collect();
        for t in &open {
            self.cancel(squad, *t).await;
        }
        stop.cancelled = open;
        if let Some(journal) = self.journals.get(squad) {
            let _ = journal.stops().set_cancelled(stop.id, &stop.cancelled);
        }
        let cancelled = if stop.cancelled.is_empty() {
            "nothing was open".to_owned()
        } else {
            format!("cancelled {}", stop.cancelled.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", "))
        };
        let line = format!(
            "The person stopped “{}” (S-{}): {cancelled}. equipe turns away new tickets of it until they lift the stop — don't plan it, file it, or follow it up.",
            stop.words, stop.id
        );
        let shown = say!(self, "news.stopped", words = stop.words, count = stop.cancelled.len());
        self.brief_pm(squad, line, Some(shown));
        crate::daemon_log!("equipe: {squad}: stop S-{} by {by}: “{}”, terms {:?}, {} ticket(s) cancelled", stop.id, stop.words, stop.terms, stop.cancelled.len());
        self.publish_stops(squad);
        Ok(stop)
    }

    /// The person's "Stop this" ([`Command::StopWork`]).
    pub(super) async fn person_stops(&mut self, squad: &str, request: Option<RequestId>, ticket: Option<TicketId>, words: &str) {
        if request.is_none() && ticket.is_none() && words.trim().is_empty() {
            return self.squad_error(squad, say!(self, "error.stop_what"));
        }
        if let Err(err) = self.stop_work(squad, "you", words, Vec::new(), Vec::new(), request, ticket).await {
            self.squad_error(squad, err);
        }
    }

    /// The person lifts stop `id` ([`Command::LiftStop`]): that work may be filed again.
    pub(super) fn lift_stop(&mut self, squad: &str, id: StopId) {
        let Some(journal) = self.journal(squad) else { return };
        let Some(stop) = journal.stops().get(id) else { return };
        if !journal.stops().lift(id, now_secs()).unwrap_or(false) {
            return;
        }
        let line = format!("The person lifted the stop on “{}” (S-{id}): that work may be put on the board again when it is asked for.", stop.words);
        let shown = say!(self, "news.stop_lifted", words = stop.words);
        self.brief_pm(squad, line, Some(shown));
        self.publish_stops(squad);
    }

    /// The stop a new ticket would break, if any — counted, and the screens
    /// told. `lineage`: the tickets it comes of; `request`: the one it serves.
    pub(super) fn stop_for(&mut self, squad: &str, title: &str, body: &str, lineage: &[TicketId], request: Option<RequestId>) -> Option<Stop> {
        let stops = self.stops_in_force(squad);
        let stop = rules::stopping(&stops, title, body, lineage, request)?.clone();
        self.stop_refused(squad, &stop, title);
        Some(stop)
    }

    /// `stop` turned away a ticket titled `title`: counted, and the screens told.
    pub(super) fn stop_refused(&mut self, squad: &str, stop: &Stop, title: &str) {
        if let Some(journal) = self.journals.get(squad) {
            let _ = journal.stops().refused(stop.id);
        }
        crate::daemon_log!("equipe: {squad}: “{title}” turned away by stop S-{} (“{}”)", stop.id, stop.words);
        self.publish_stops(squad);
    }

    /// Whether the person asked for this again, in their own words to the
    /// member of `id` since `stop` began: then its ticket goes on the board.
    pub(super) fn asked_again(&self, id: SessionId, stop: &Stop) -> bool {
        let member = self.sessions.get(&id).and_then(|s| s.run_of.or(s.lane.as_ref().map(|(of, _)| *of))).unwrap_or(id);
        self.person_said_to(member).into_iter().any(|(text, at)| at > stop.since && rules::named_by(stop, &text, &text))
    }

    /// `stop_work`: the person told this member to stop a kind of work.
    pub(super) async fn stop_work_tool(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        let answer = self.stop_work_answer(&call).await;
        let _ = reply.send(match answer {
            Ok(text) => ToolResult::ok(text),
            Err(text) => ToolResult::error(text),
        });
    }

    async fn stop_work_answer(&mut self, call: &ToolCall) -> Result<String, String> {
        let id = self.caller_id(call).ok_or_else(|| format!("{} isn't on the squad {}.", call.member, call.squad))?;
        let member = self.sessions.get(&id).and_then(|s| s.run_of.or(s.lane.as_ref().map(|(of, _)| *of))).unwrap_or(id);
        let now = now_secs();
        if !self.person_said_to(member).iter().any(|(_, at)| now.saturating_sub(*at) <= crate::requests::ASKED_WITHIN) {
            return Err("Only the person stops a kind of work, in their own words to you. If you think something should stop, ask them.".into());
        }
        let args = &call.arguments;
        let words = board::arg_str(args, "words").map(str::trim).filter(|w| !w.is_empty()).ok_or("Give `words`: what the person said to stop, in their words.")?;
        let terms = board::arg_strs(args, "terms");
        let paths = board::arg_strs(args, "paths");
        let request = self.request_arg(&call.squad, args)?;
        let ticket = board::arg_id(args, "ticket");
        if let Some(t) = ticket
            && self.find_ticket(&call.squad, t).is_none()
        {
            return Err(format!("There is no T-{t}."));
        }
        if terms.is_empty() && paths.is_empty() && request.is_none() && ticket.is_none() {
            return Err("Say what it covers: `terms` (words its tickets' titles have, e.g. \"component gallery\"), `paths`, a `ticket` of it or its `request`.".into());
        }
        let stop = self.stop_work(&call.squad, &call.member, words, terms, paths, request, ticket).await?;
        let cancelled = if stop.cancelled.is_empty() {
            "No ticket of it was open".to_owned()
        } else {
            format!("Cancelled {}", stop.cancelled.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", "))
        };
        Ok(format!(
            "Stopped as S-{}. {cancelled}; new tickets of it are turned away (titles with {}) until the person lifts the stop under Your requests. Tell them so in one line.",
            stop.id,
            if stop.terms.is_empty() { "—".to_owned() } else { stop.terms.iter().map(|t| format!("“{t}”")).collect::<Vec<_>>().join(" or ") }
        ))
    }
}
