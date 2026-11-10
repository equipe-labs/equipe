//! Handed-in work put to the person to confirm (`propose_accept`, or a screen
//! reading their own words): kept while it waits — in memory only, as what
//! the person said is about the board as it stands — and shown on every
//! screen. Confirmed by the person's own command, it runs exactly what the
//! board's buttons send: [`Command::Accept`], then [`Command::Merge`] when the
//! screen's way says so, or [`Command::Reopen`] with the note. Nothing here
//! accepts, merges or sends back any other way, and nobody's words confirm.

use equipe_protocol::accept::{AcceptProposal, Confirmed, Land, Verdict};

use super::*;
use crate::accept::{self as rules, Declined, Outcome};

impl Runtime {
    /// `squad`'s proposals waiting for the person, oldest first.
    pub(super) fn accepts(&mut self, squad: &str) -> &mut Vec<AcceptProposal> {
        self.accepts.entry(squad.to_owned()).or_default()
    }

    /// What the person said no to a moment ago, still fresh.
    fn declined_lately(&mut self, squad: &str) -> Vec<Declined> {
        let now = now_secs();
        let list = self.accepts_declined.entry(squad.to_owned()).or_default();
        list.retain(|d| now.saturating_sub(d.at) < rules::DECLINED_FOR);
        list.clone()
    }

    fn put_up(&mut self, squad: &str, proposal: AcceptProposal) {
        self.accepts(squad).push(proposal.clone());
        self.publish(Event::AcceptProposed { squad: squad.to_owned(), proposal });
    }

    /// `propose_accept`: checked against the board and what already waits,
    /// kept, and shown; the PM told the person confirms on a card.
    pub(super) fn propose_accept(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        if !self.sessions.get(&id).is_some_and(Session::is_pm) {
            return Err("Only the PM puts handed-in work to the person to confirm, and only the person accepts it. Don't take \
their words as an acceptance, and don't walk them through buttons: say in one line that a confirmation card puts it to \
them, and note what they said on the ticket with `comment`, for the PM."
                .into());
        }
        let tickets = self.tickets(squad);
        let open = self.accepts.get(squad).cloned().unwrap_or_default();
        let declined = self.declined_lately(squad);
        match rules::check(args, &tickets, &open, &declined, now_secs())? {
            Outcome::Already { tickets, dropped } => Ok(rules::already_reply(&tickets, &dropped)),
            // The person's own words already do it: being done, or read back to them.
            Outcome::Propose(checked) if self.words_cover(squad, &checked.tickets) => Ok(format!(
                "The person's own words already do this ({}): equipe is doing it, or reading it back to them to confirm. Put nothing to them; carry on.",
                crate::by_words::list(&checked.tickets)
            )),
            Outcome::Propose(checked) => {
                let reply = rules::proposed_reply(&checked, &tickets);
                let proposal = checked.into_proposal(equipe_protocol::new_session_id(), member, id, now_secs());
                self.put_up(squad, proposal);
                Ok(reply)
            }
        }
    }

    /// `accept_work`: the PM accepts handed-in work itself — and merges it into
    /// main — while the person lets it (`SetPmAccepts`); what it checked is
    /// kept on each ticket, and the person sees it done in the PM's conversation.
    pub(super) async fn accept_work(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        if !self.sessions.get(&id).is_some_and(Session::is_pm) {
            return Err("Only the PM accepts the team's work. Hand yours in with submit_for_review.".into());
        }
        if !self.journal(squad).is_some_and(|j| j.pm_accepts()) {
            return Err("In this squad the person accepts work themselves: put it to them with propose_accept.".into());
        }
        let checked = board::arg_str(args, "checked").map(str::trim).unwrap_or_default().to_owned();
        if checked.is_empty() {
            return Err("Say in `checked` what you checked and found — it is kept on the ticket for the person.".into());
        }
        let push = args.get("push").and_then(Value::as_bool).unwrap_or(false);
        let mut named: Vec<TicketId> = args.get("tickets").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_u64()).map(|v| v as TicketId).collect()).unwrap_or_default();
        named.dedup();
        if named.is_empty() {
            return Err("Name the handed-in tickets you accept in `tickets`.".into());
        }
        let (mut accepted, mut left) = (Vec::new(), Vec::new());
        for ticket in named {
            let Some(t) = self.find_ticket(squad, ticket) else {
                left.push(format!("there is no T-{ticket}"));
                continue;
            };
            if let Some(why) = rules::unfit(&t, Verdict::Accept) {
                left.push(why);
                continue;
            }
            // Its own code goes to the person: nobody accepts their own work.
            if let Some(why) = rules::own_code(&t, member) {
                left.push(why);
                continue;
            }
            if !t.ready_for_review() {
                left.push(format!("T-{ticket} waits for its design QA"));
                continue;
            }
            // A branch only behind main merges with a merge commit: no reason to
            // send it back. A real conflict goes back to its member now, before
            // anything is accepted; one that can't be told is left to the merge.
            if !t.kind.in_home()
                && let (Some(branch), Some(project)) = (t.branch.clone(), t.project.clone())
            {
                let looked = tokio::task::spawn_blocking(move || worktree::repo_root(&project).and_then(|repo| worktree::merge_conflicts(&repo, &branch))).await;
                if let Ok(Ok(files)) = looked
                    && !files.is_empty()
                {
                    let files = files.join(", ");
                    let reason = format!(
                        "Its branch conflicts with main in {files}. Merge main into your branch in your worktree (git merge \
main; no rebase needed), resolve each conflict keeping what both sides meant, run the relevant tests, and hand it in again."
                    );
                    match self.send_back(squad, ticket, member, &reason, true) {
                        Ok(who) => left.push(format!("T-{ticket} conflicts with main in {files}: sent back to {who} to merge main in")),
                        Err(why) => left.push(why),
                    }
                    continue;
                }
            }
            if let Some(journal) = self.journal(squad) {
                let _ = journal.ticket_event(ticket, member, "accepted by the PM", &checked);
            }
            Box::pin(self.accept(squad, ticket, crate::telemetry::Accepter::Pm)).await;
            // Code lands on main at once: nothing left for the person to merge.
            if !t.kind.in_home() && t.branch.is_some() {
                self.merge(squad, ticket, push);
            }
            accepted.push(ticket);
        }
        if !accepted.is_empty() {
            let ids = accepted.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
            let text = say!(self, "notice.pm_accepted", ids = ids, checked = checked);
            self.publish(Event::Notice { undo: None, session: id, text });
        }
        let ids = accepted.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
        let mut reply = if accepted.is_empty() {
            "Nothing accepted.".to_owned()
        } else if push {
            format!("Accepted {ids}; code is merging into main and main is being pushed.")
        } else {
            format!("Accepted {ids}; code is merging into main.")
        };
        if !left.is_empty() {
            reply.push_str(&format!(" Left: {}.", left.join("; ")));
        }
        Ok(reply)
    }

    /// A screen read the person's own words as plainly accepting: the card
    /// goes up in that conversation, by the same rules; what doesn't fit is
    /// left to the member there to answer.
    pub(super) fn propose_accept_from_words(&mut self, squad: &str, session: SessionId, verdict: Verdict, named: &[TicketId], note: &str) {
        if !self.sessions.get(&session).is_some_and(|s| s.squad.as_deref() == Some(squad)) {
            return;
        }
        let tickets = self.tickets(squad);
        let open = self.accepts.get(squad).cloned().unwrap_or_default();
        let declined = self.declined_lately(squad);
        if let Ok(Outcome::Propose(checked)) = rules::check_tickets(verdict, named, note, &tickets, &open, &declined, now_secs()) {
            let proposal = checked.into_proposal(equipe_protocol::new_session_id(), equipe_protocol::accept::FROM_WORDS, session, now_secs());
            self.put_up(squad, proposal);
        }
    }

    /// Take proposal `id` out of what waits, if it still does.
    fn take_accept(&mut self, squad: &str, id: u64) -> Option<AcceptProposal> {
        let list = self.accepts(squad);
        let at = list.iter().position(|p| p.id == id)?;
        Some(list.remove(at))
    }

    /// The person's yes, for the tickets they kept on the card: each gets what
    /// the board's buttons send, the card closes everywhere. Decided already,
    /// elsewhere: nothing more.
    pub(super) async fn confirm_accept(&mut self, squad: &str, id: u64, picked: Vec<Confirmed>, said: Option<equipe_protocol::by_words::Said>) {
        let Some(proposal) = self.take_accept(squad, id) else { return };
        // Confirmed in words on a device that made sure of them itself: logged as said.
        if let Some(said) = &said {
            let kind = if proposal.verdict == Verdict::Merge { equipe_protocol::by_words::Kind::Merge } else { equipe_protocol::by_words::Kind::AcceptCode };
            self.words_said_elsewhere(squad, kind, proposal.tickets.clone(), said, equipe_protocol::by_words::Outcome::Done);
        }
        let mut acted = Vec::new();
        for pick in picked.iter().filter(|p| proposal.tickets.contains(&p.ticket)) {
            let ticket = pick.ticket;
            let Some(t) = self.journal(squad).and_then(|j| j.ticket(ticket).ok().flatten()) else { continue };
            if rules::unfit(&t, proposal.verdict).is_some() {
                continue;
            }
            let squad = squad.to_owned();
            match proposal.verdict {
                Verdict::Changes => {
                    // What R on the board sends.
                    Box::pin(self.handle(Command::Reopen { squad, ticket, note: proposal.note.clone() })).await;
                }
                Verdict::Accept | Verdict::Merge => {
                    // What A on the board sends: accept, then land it the screen's way.
                    if t.status == TicketStatus::Review {
                        Box::pin(self.handle(Command::Accept { squad: squad.clone(), ticket })).await;
                    }
                    if let Some(land) = pick.merge.filter(|_| !t.kind.in_home() && t.branch.is_some()) {
                        Box::pin(self.handle(Command::Merge { squad, ticket, push: land == Land::Push })).await;
                    }
                }
            }
            acted.push(ticket);
        }
        self.publish(Event::AcceptClosed { squad: squad.to_owned(), id, confirmed: true, tickets: acted.clone() });
        let left: Vec<TicketId> = proposal.tickets.iter().copied().filter(|t| !acted.contains(t)).collect();
        if !left.is_empty() {
            let ids = left.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
            self.brief_pm(squad, format!("The person left {ids} out when they confirmed the card: as it was, it waits for them."), None);
        }
    }

    /// The person's no: nothing changes; the PM is told, and the same card
    /// isn't put to them again for a while.
    pub(super) fn decline_accept(&mut self, squad: &str, id: u64) {
        let Some(proposal) = self.take_accept(squad, id) else { return };
        self.publish(Event::AcceptClosed { squad: squad.to_owned(), id, confirmed: false, tickets: Vec::new() });
        self.accepts_declined.entry(squad.to_owned()).or_default().push(Declined { verdict: proposal.verdict, tickets: proposal.tickets.clone(), at: now_secs() });
        let ids = proposal.tickets.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
        let (what_en, key) = match proposal.verdict {
            Verdict::Accept => (format!("accepting {ids}"), "news.accept_declined"),
            Verdict::Merge => (format!("accepting and merging {ids}"), "news.merge_declined"),
            Verdict::Changes => (format!("sending {ids} back"), "news.changes_declined"),
        };
        let line = format!("The person said no to the card ({what_en}): it stays as it is. Don't put it to them again unless they ask.");
        let shown = self.said(key, &[("tickets", ids)]);
        self.brief_pm(squad, line, Some(shown));
    }

    /// Work in review back to the member that handed it in — same branch,
    /// worktree and conversation — with `reason` as what to change: saved, on
    /// its record by `by`, and handed out again, so its member hears it when it
    /// picks it up (its sent-back work goes ahead of new). With `news`, the
    /// person reads it in the PM's conversation. Routing only: nothing is
    /// accepted or merged. The member it went back to.
    pub(super) fn send_back(&mut self, squad: &str, ticket: TicketId, by: &str, reason: &str, news: bool) -> Result<String, String> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err("Say what to change.".into());
        }
        let found = self.journal(squad).and_then(|j| j.ticket(ticket).ok().flatten()).ok_or_else(|| format!("There is no T-{ticket}."))?;
        if found.status != TicketStatus::Review {
            return Err(format!("T-{ticket} is {}, not in review: there is nothing to send back.", found.status.label()));
        }
        if self.worker_on(squad, ticket).is_some() {
            return Err(format!("T-{ticket} is being worked on."));
        }
        // Its design QA isn't needed any more: the work changes first.
        self.drop_design_qa(squad, ticket);
        let mut found = self.journal(squad).and_then(|j| j.ticket(ticket).ok().flatten()).ok_or_else(|| format!("There is no T-{ticket}."))?;
        let member = found.assignee.clone().or_else(|| found.owner.clone()).unwrap_or_default();
        found.owner = found.assignee.take().or(found.owner);
        found.status = TicketStatus::Todo;
        found.note = Some(format!("{}{reason}", rules::PM_SENT_BACK));
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        journal.save_ticket(&found).map_err(|e| e.to_string())?;
        let _ = journal.ticket_event(ticket, by, rules::SENT_BACK, reason);
        let shown = say!(self, "news.pm_sent_back", id = ticket, title = found.title, name = member);
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: found });
        if news && let Some(pm) = self.pm_of(squad) {
            self.show_news(pm, shown);
        }
        let _ = self.internal.send(Internal::Dispatch);
        Ok(member)
    }

    /// The PM's `comment` on `ticket`: on work in review it says whether it
    /// sends it back or only notes it — a change request is never left as a
    /// note its member doesn't hear.
    pub(super) fn pm_comment(&mut self, squad: &str, member: &str, ticket: &Ticket, text: &str, word: Option<rules::ReviewWord>) -> Result<String, String> {
        let id = ticket.id;
        let note = |rt: &mut Self| rt.journal(squad).ok_or_else(|| "The board can't be opened.".to_owned())?.ticket_event(id, member, board::COMMENT, text).map_err(|e| e.to_string());
        if ticket.status != TicketStatus::Review {
            note(self)?;
            return Ok(match word {
                Some(rules::ReviewWord::Changes) => format!("Noted on T-{id}. It is {}, not in review: nothing was sent back.", ticket.status.label()),
                _ => format!("Noted on T-{id}."),
            });
        }
        let to = ticket.assignee.clone().or_else(|| ticket.owner.clone()).unwrap_or_else(|| "its member".into());
        match word {
            None => Err(rules::say_which(id, &to)),
            Some(rules::ReviewWord::Note) => {
                note(self)?;
                Ok(format!("Noted on T-{id}; it still waits for review."))
            }
            Some(rules::ReviewWord::Changes) => {
                let who = self.send_back(squad, id, member, text, true)?;
                Ok(format!("Sent T-{id} back to {who} with what to change; they pick it up next, on the same branch. Carry on with what else waits."))
            }
        }
    }

    /// Once per squad, at start: before `comment` asked the PM which it
    /// meant, its change requests on work in review were plain notes their
    /// members never heard, and the work sat in review. Each ticket whose
    /// record ends so goes back to the member that handed it in, with what
    /// the PM asked; the person is told once.
    pub(super) fn recover_change_requests(&mut self) {
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            let Some(pm) = self.pm_of(&squad).and_then(|id| self.sessions.get(&id)).and_then(|s| s.name().map(str::to_owned)) else {
                continue;
            };
            let Some(journal) = self.journal(&squad) else { continue };
            if journal.change_requests_recovered() {
                continue;
            }
            let in_review: Vec<TicketId> = journal.tickets().unwrap_or_default().iter().filter(|t| t.status == TicketStatus::Review).map(|t| t.id).collect();
            let unheard: Vec<(TicketId, String)> = in_review
                .into_iter()
                .filter_map(|id| rules::unheard_change_request(&journal.ticket_entries(id).unwrap_or_default(), &pm).map(|asked| (id, asked)))
                .collect();
            // Marked first: whatever happens next, it runs once.
            if let Err(err) = journal.set_change_requests_recovered() {
                crate::daemon_log!("equipe: {squad}: unheard change requests not looked at: {err}");
                continue;
            }
            let mut sent = Vec::new();
            for (id, asked) in unheard {
                match self.send_back(&squad, id, &pm, &format!("{asked}{}", rules::UNHEARD_NOTE), false) {
                    Ok(who) => {
                        crate::daemon_log!("equipe: {squad}: T-{id} went back to {who} with the PM's change request it never heard");
                        sent.push(id);
                    }
                    Err(err) => crate::daemon_log!("equipe: {squad}: T-{id} kept in review: {err}"),
                }
            }
            if sent.is_empty() {
                continue;
            }
            let ids = sent.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
            let shown = say!(self, "news.change_requests_recovered", count = sent.len(), tickets = ids);
            let line = format!("equipe: your change requests on {ids} were plain comments their members never heard; each went back to its member with what you asked. From now on, `comment` on work in review takes `review: changes` to send it back.");
            self.brief_pm(&squad, line, Some(shown));
        }
    }

    /// `ticket` changed: a proposal it was on that has nothing left to
    /// confirm (decided on the board, sent back, dropped) no longer waits.
    /// Nobody is told — there is nothing to decide.
    pub(super) fn settle_accepts(&mut self, squad: &str, ticket: TicketId) {
        if !self.accepts.get(squad).is_some_and(|l| l.iter().any(|p| p.tickets.contains(&ticket))) {
            return;
        }
        let tickets = self.tickets(squad);
        let (kept, gone): (Vec<AcceptProposal>, Vec<AcceptProposal>) = std::mem::take(self.accepts(squad)).into_iter().partition(|p| rules::still_open(p, &tickets));
        *self.accepts(squad) = kept;
        for p in gone {
            self.publish(Event::AcceptClosed { squad: squad.to_owned(), id: p.id, confirmed: false, tickets: Vec::new() });
        }
    }
}
