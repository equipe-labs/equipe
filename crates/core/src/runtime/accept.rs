//! Handed-in work put to the person to confirm (`propose_accept`, or a screen
//! reading their own words): kept while it waits — in memory only, as what
//! the person said is about the board as it stands — and shown on every
//! screen. Confirmed by the person's own command, it runs exactly what the
//! board's buttons send: [`Command::Accept`], then [`Command::Merge`] when the
//! screen's way says so, or [`Command::Reopen`] with the note. Nothing here
//! accepts, merges or sends back any other way, and nobody's words confirm.

use equipe_protocol::accept::{AcceptProposal, Confirmed, Land, Verdict};
use equipe_protocol::tiers::SendBackReason;

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
                // Small work is the PM's to decide; only what is weighty goes up, with why (`person_gate`).
                let why = match self.gate_card(squad, member, &checked.tickets, args)? {
                    super::person_gate::Gate::Pm(reply) => return Ok(reply),
                    super::person_gate::Gate::Card(why) => why,
                };
                let reply = rules::proposed_reply(&checked, &tickets);
                let mut proposal = checked.into_proposal(equipe_protocol::new_session_id(), member, id, now_secs());
                if !why.is_empty() {
                    proposal.summary = [proposal.summary.trim(), why.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                }
                self.put_up(squad, proposal);
                Ok(reply)
            }
        }
    }

    /// `accept_work`: the PM accepts handed-in work itself — and merges it into
    /// main — while the person lets it (`SetPmAccepts`); what it checked is
    /// kept on each ticket, and the person sees it done in the PM's conversation.
    ///
    /// What the board says is checked here, on the loop; the git side — a
    /// look for conflicts with main, the project's checks file, a worktree
    /// released or a task's work landed in the squad's folder — runs off it,
    /// as it can take a minute on a large repository, and
    /// [`Runtime::accept_worked`] takes it from there, in the order the tickets
    /// were named. The PM's call is answered then, with what really happened.
    pub(super) async fn accept_work(&mut self, squad: &str, member: &str, id: SessionId, args: &Value, reply: oneshot::Sender<ToolResult>) {
        let (checked, push, plan) = match self.accept_plan(squad, member, id, args) {
            Ok(planned) => planned,
            Err(text) => {
                self.dispatch(squad).await;
                let _ = reply.send(ToolResult { text, is_error: true, image: None });
                return;
            }
        };
        let mut worked = AcceptWorked { squad: squad.to_owned(), member: member.to_owned(), session: id, checked, push, named: Vec::new(), reply };
        // Nothing for git to do: answered at once.
        if !plan.iter().any(|p| matches!(p, Plan::Look(_))) {
            worked.named = plan.into_iter().map(|p| match p {
                Plan::Left(why) => Named::Left(why),
                Plan::Look(look) => Named::Worked { ticket: look.ticket, code: look.code, found: Found::Failed },
            }).collect();
            return Box::pin(self.accept_worked(worked)).await;
        }
        // Should the look die, each ticket it was on is said to be left as it was.
        let fallback: Vec<Named> = plan
            .iter()
            .map(|p| match p {
                Plan::Left(why) => Named::Left(why.clone()),
                Plan::Look(look) => Named::Worked { ticket: look.ticket.clone(), code: look.code, found: Found::Failed },
            })
            .collect();
        let (home, internal) = (self.config.home.clone(), self.internal.clone());
        let (squad, member) = (squad.to_owned(), member.to_owned());
        tokio::spawn(async move {
            let looked = tokio::task::spawn_blocking(move || look_off_loop(&home, &squad, &member, plan)).await;
            worked.named = looked.unwrap_or(fallback);
            let _ = internal.send(Internal::Accepting(Box::new(worked)));
        });
    }

    /// What `accept_work` asks, checked against the board: what the PM said
    /// it checked, whether main is pushed, and each ticket named, in order —
    /// left out and why, or to be looked at off the loop. The tickets to look
    /// at are under way from here on (`accepting`).
    fn accept_plan(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<(String, bool, Vec<Plan>), String> {
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
        let mut seen = HashSet::new();
        named.retain(|t| seen.insert(*t));
        if named.is_empty() {
            return Err("Name the handed-in tickets you accept in `tickets`.".into());
        }
        watchdog::mark("accept_work: the board looked at");
        let mut plan = Vec::new();
        for ticket in named {
            // Asked again while the first is worked out: that one answers.
            if self.accepting.contains(&(squad.to_owned(), ticket)) {
                plan.push(Plan::Left(format!("T-{ticket} is already being accepted: you hear how it went")));
                continue;
            }
            let Some(t) = self.find_ticket(squad, ticket) else {
                plan.push(Plan::Left(format!("there is no T-{ticket}")));
                continue;
            };
            if let Some(why) = rules::unfit(&t, Verdict::Accept) {
                plan.push(Plan::Left(why));
                continue;
            }
            // Its own code goes to the person: nobody accepts their own work.
            if let Some(why) = rules::own_code(&t, member) {
                plan.push(Plan::Left(why));
                continue;
            }
            if !t.ready_for_review() {
                plan.push(Plan::Left(format!("T-{ticket} waits for its design QA")));
                continue;
            }
            if self.gating(squad, ticket) {
                plan.push(Plan::Left(format!("T-{ticket}'s checks are running already: you hear how they went")));
                continue;
            }
            let code = !t.kind.in_home() && t.branch.is_some();
            // What closing it does in git, read now and done off the loop once
            // it is fit to accept; a member on it closes it its own way.
            let close = if self.worker_on(squad, ticket).is_none() {
                let saved = self.journal(squad).and_then(|j| j.ticket_work(ticket).ok()).unwrap_or_default();
                self.close_git(squad, &t, &saved, TicketStatus::Done)
            } else {
                None
            };
            self.accepting.insert((squad.to_owned(), ticket));
            plan.push(Plan::Look(Look { ticket: Box::new(t), code, close }));
        }
        Ok((checked, push, plan))
    }

    /// The git side of `accept_work` is through: each ticket, in the order
    /// named, is sent back for a conflict, put to its project's checks, or
    /// accepted and merged — then the PM hears what happened.
    pub(super) async fn accept_worked(&mut self, worked: AcceptWorked) {
        let AcceptWorked { squad, member, session: id, checked, push, named, reply } = worked;
        let (squad, member) = (squad.as_str(), member.as_str());
        let (mut accepted, mut left) = (Vec::new(), Vec::new());
        // What a designer made, to put in front of the person with the acceptance.
        let mut made: Vec<String> = Vec::new();
        // Code merged with no checks run (the project declares none), and code
        // whose checks run first.
        let (mut unchecked, mut checking) = (Vec::new(), Vec::new());
        for named in named {
            let (t, code, found) = match named {
                Named::Left(why) => {
                    left.push(why);
                    continue;
                }
                Named::Worked { ticket, code, found } => (ticket, code, found),
            };
            let ticket = t.id;
            self.accepting.remove(&(squad.to_owned(), ticket));
            match found {
                Found::Failed => left.push(format!("T-{ticket} wasn't accepted: equipe couldn't look at its branch. Accept it again")),
                // A branch only behind main merges with a merge commit: no reason to
                // send it back. A real conflict goes back to its member now, before
                // anything is accepted; one that can't be told is left to the merge.
                Found::Conflicts(files) => {
                    watchdog::mark("accept_work: sent back");
                    let files = files.join(", ");
                    let reason = format!(
                        "Its branch conflicts with main in {files}. Merge main into your branch in your worktree (git merge \
main; no rebase needed), resolve each conflict keeping what both sides meant, run the relevant tests, and hand it in again."
                    );
                    match self.send_back(squad, ticket, member, &reason, SendBackReason::Environment, true) {
                        Ok(who) => left.push(format!("T-{ticket} conflicts with main in {files}: sent back to {who} to merge main in")),
                        Err(why) => left.push(why),
                    }
                }
                // Checked on its merge first, off the loop: accepted and landed only if they pass.
                Found::Checks { repo, branch } => {
                    if self.gating(squad, ticket) {
                        left.push(format!("T-{ticket}'s checks are running already: you hear how they went"));
                        continue;
                    }
                    let title = t.title.clone();
                    self.queue_gate(gate::Gate { squad: squad.to_owned(), ticket, title, by: member.to_owned(), session: id, checked: checked.clone(), push, repo, branch, tries: 0 });
                    checking.push(ticket);
                }
                // A checks file that can't be read holds the merge back: never guessed past.
                Found::Unreadable(err) => left.push(format!("T-{ticket} wasn't accepted: {err}. Get that file fixed on main, then accept it again")),
                Found::Accept { made: work, unchecked: no_checks, closed } => {
                    // Moved meanwhile (the person accepted or cancelled it on the board): nothing more.
                    let status = self.journal(squad).and_then(|j| j.ticket(ticket).ok().flatten()).map(|t| t.status);
                    if status != Some(TicketStatus::Review) {
                        let now = status.map_or("gone", TicketStatus::label);
                        crate::daemon_log!("equipe: {squad}: T-{ticket} changed while the PM's acceptance was worked out (it is {now} now): not accepted");
                        left.push(format!("T-{ticket} changed while it was being accepted (it is {now} now): equipe did nothing more with it"));
                        continue;
                    }
                    made.extend(work);
                    if no_checks {
                        unchecked.push(ticket);
                    }
                    if let Some(journal) = self.journal(squad) {
                        let _ = journal.ticket_event(ticket, member, "accepted by the PM", &checked);
                    }
                    if let Some(closed) = closed {
                        self.closing.insert((squad.to_owned(), ticket), closed);
                    }
                    watchdog::mark("accept_work: accepted");
                    Box::pin(self.accept(squad, ticket, crate::telemetry::Accepter::Pm)).await;
                    // Closed another way (a member was on it): nothing kept for later.
                    self.closing.remove(&(squad.to_owned(), ticket));
                    // Code lands on main at once: nothing left for the person to merge.
                    if code {
                        self.merge(squad, ticket, push);
                    }
                    accepted.push(ticket);
                }
            }
        }
        watchdog::mark("accept_work: answered");
        let list = |ids: &[TicketId]| ids.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
        // What the person reads: work merged with no checks run says so.
        let (unchecked_now, rest): (Vec<TicketId>, Vec<TicketId>) = accepted.iter().partition(|t| unchecked.contains(t));
        if !rest.is_empty() {
            let text = say!(self, "notice.pm_accepted", ids = list(&rest), checked = checked);
            self.publish(Event::Notice { undo: None, session: id, text });
        }
        if !unchecked_now.is_empty() {
            let text = say!(self, "notice.pm_accepted_unchecked", ids = list(&unchecked_now), checked = checked, file = crate::checks::FILE);
            self.publish(Event::Notice { undo: None, session: id, text });
        }
        let ids = list(&accepted);
        let mut text = if accepted.is_empty() && checking.is_empty() {
            "Nothing accepted.".to_owned()
        } else if accepted.is_empty() {
            String::new()
        } else if push {
            format!("Accepted {ids}; code is merging into main and main is being pushed.")
        } else {
            format!("Accepted {ids}; code is merging into main.")
        };
        if !unchecked_now.is_empty() {
            text.push_str(&format!(" No project checks ran for {}: the project declares none in {}.", list(&unchecked_now), crate::checks::FILE));
        }
        if !checking.is_empty() {
            text.push_str(&format!(
                " Checking {} first: equipe runs the project's checks on it merged with main, off to the side, and accepts and merges \
it only if they pass; if they fail it goes back to its member with the output, and main stays as it is. You hear how it went; \
carry on meanwhile.",
                list(&checking)
            ));
        }
        let mut text = text.trim_start().to_owned();
        if !left.is_empty() {
            text.push_str(&format!(" Left: {}.", left.join("; ")));
        }
        if !made.is_empty() {
            text.push_str(&format!("\n\nThe person has not seen what the designer made. Put these pictures in your message to them as they are, one to a line, before you describe anything:\n{}", made.join("\n")));
        }
        self.dispatch(squad).await;
        let _ = reply.send(ToolResult { text, is_error: false, image: None });
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
    /// accepted or merged. `why` is what kind of problem it is, which decides
    /// whether its model goes up ([`Runtime::after_send_back`]). The member it
    /// went back to.
    pub(super) fn send_back(&mut self, squad: &str, ticket: TicketId, by: &str, reason: &str, why: SendBackReason, news: bool) -> Result<String, String> {
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
        self.after_send_back(squad, ticket, by, why);
        let _ = self.internal.send(Internal::Dispatch);
        Ok(member)
    }

    /// The PM's `comment` on `ticket`: on work in review it says whether it
    /// sends it back or only notes it — a change request is never left as a
    /// note its member doesn't hear.
    pub(super) fn pm_comment(&mut self, squad: &str, member: &str, ticket: &Ticket, text: &str, word: Option<rules::ReviewWord>, why: SendBackReason) -> Result<String, String> {
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
                let who = self.send_back(squad, id, member, text, why, true)?;
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
                match self.send_back(&squad, id, &pm, &format!("{asked}{}", rules::UNHEARD_NOTE), SendBackReason::Correctness, false) {
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

/// The PM's `accept_work`, its git side worked out off the loop: who asked,
/// what for, and each ticket named, in order — left out on the loop, or
/// looked at off it. The PM's call waits on `reply`; the loop doesn't.
pub(super) struct AcceptWorked {
    squad: String,
    member: String,
    session: SessionId,
    checked: String,
    push: bool,
    named: Vec<Named>,
    reply: oneshot::Sender<ToolResult>,
}

/// A ticket `accept_work` named, as the loop planned it.
enum Plan {
    /// Left out, and why.
    Left(String),
    /// To be looked at off the loop.
    Look(Look),
}

struct Look {
    ticket: Box<Ticket>,
    /// Code on a branch: looked at for conflicts and checks, merged once accepted.
    code: bool,
    /// What closing it does in git, done off the loop once it is fit to accept.
    close: Option<CloseGit>,
}

/// A ticket `accept_work` named, once looked at.
enum Named {
    Left(String),
    Worked { ticket: Box<Ticket>, code: bool, found: Found },
}

/// What the look off the loop found for a ticket.
enum Found {
    /// Its branch conflicts with main in these files: back to its member.
    Conflicts(Vec<String>),
    /// Its project declares checks: they run on its merge before it is accepted.
    Checks { repo: PathBuf, branch: String },
    /// Its project's checks file can't be read: nothing merges.
    Unreadable(String),
    /// Fit to accept: what a designer made, to show; whether its code merges
    /// with no checks run; and its closing git work, done.
    Accept { made: Option<String>, unchecked: bool, closed: Option<Closed> },
    /// The look itself died: nothing was done.
    Failed,
}

/// The git side of `accept_work`, for each ticket in turn: blocking, and run
/// off the loop. How long each part took is logged when it all took long,
/// as the loop's own slow steps are.
fn look_off_loop(home: &Path, squad: &str, member: &str, plan: Vec<Plan>) -> Vec<Named> {
    let began = std::time::Instant::now();
    let mut at = began;
    let mut parts: Vec<(String, std::time::Duration)> = Vec::new();
    let mut lap = |what: String| {
        let now = std::time::Instant::now();
        parts.push((what, now.saturating_duration_since(at)));
        at = now;
    };
    let mut out = Vec::new();
    for item in plan {
        let look = match item {
            Plan::Left(why) => {
                out.push(Named::Left(why));
                continue;
            }
            Plan::Look(look) => look,
        };
        let Look { ticket, code, close } = look;
        let id = ticket.id;
        let mut unchecked = false;
        let found = 'found: {
            // A real conflict goes back before anything is accepted; and what
            // the project declares as its checks, on main as it stands.
            if code && let (Some(branch), Some(project)) = (ticket.branch.clone(), ticket.project.clone()) {
                let repo = worktree::repo_root(&project).ok();
                let conflicts = repo.as_deref().map(|repo| worktree::merge_conflicts(repo, &branch));
                lap(format!("T-{id} conflicts with main looked for"));
                let declared = repo.as_deref().map(crate::checks::declared_on_main);
                lap(format!("T-{id} checks file read"));
                if let Some(Ok(files)) = conflicts
                    && !files.is_empty()
                {
                    break 'found Found::Conflicts(files);
                }
                match (declared, repo) {
                    (Some(Ok(Some(_))), Some(repo)) => break 'found Found::Checks { repo, branch },
                    (Some(Err(err)), _) => break 'found Found::Unreadable(err),
                    _ => unchecked = true,
                }
            }
            // Read before it is closed: the worktree is still there to say what it made.
            let made = crate::design::results::for_ticket(home, squad, id).filter(|w| w.hero.is_some()).map(|w| crate::design::results::embed(&w));
            lap(format!("T-{id} design work read"));
            let in_home = ticket.kind.in_home();
            let closed = close.and_then(CloseGit::run);
            lap(format!("T-{id} {}", if in_home { "landed in the squad's folder" } else { "worktree released" }));
            Found::Accept { made, unchecked, closed }
        };
        out.push(Named::Worked { ticket, code, found });
    }
    let took = began.elapsed();
    if took >= watchdog::SLOW_LOGGED {
        parts.sort_by_key(|p| std::cmp::Reverse(p.1));
        let slowest: Vec<String> = parts.iter().filter(|(_, d)| d.as_millis() >= 50).take(4).map(|(what, d)| format!("{what} {:.1}s", d.as_secs_f64())).collect();
        crate::daemon_log!("equipe: accept_work for {squad}/{member} worked out off the loop in {:.1}s ({})", took.as_secs_f64(), slowest.join(", "));
    }
    out
}
