//! A member changing its own work from what the person says in conversation,
//! rather than asking them to answer a card a certain way or to edit the
//! board themselves: its open decision cards taken back
//! ([`board::WITHDRAW_DECISION`]), and its own tickets that haven't started
//! re-scoped or dropped ([`board::REVISE_TICKET`]). Each change is said in a
//! line in its conversation, and the PM hears of it. While a member has cards
//! open, the person's next message to it carries a note of them
//! ([`Runtime::open_cards_note`]): a message that settles one has the member
//! withdraw or replace it.

use super::*;

/// The longest a card's question is quoted in a note or a line.
const QUOTED: usize = 100;

impl Runtime {
    /// `withdraw_decision`: `member`'s own open cards — the one `id` names,
    /// those about `ticket`, else all of them — taken back, said in its
    /// conversation `session`.
    pub(super) fn withdraw_decision(&mut self, squad: &str, member: &str, session: SessionId, args: &Value) -> Result<String, String> {
        let id = args.get("id").and_then(Value::as_u64);
        let ticket = board::arg_id(args, "ticket");
        let why = board::arg_str(args, "why").map(str::trim).filter(|w| !w.is_empty()).map(str::to_owned);
        let mine: Vec<equipe_protocol::decide::QuestionSet> = self
            .question_sets(squad)
            .iter()
            .filter(|s| s.member == member && id.is_none_or(|i| s.id == i) && ticket.is_none_or(|t| s.ticket == Some(t)))
            .cloned()
            .collect();
        if mine.is_empty() {
            return Err(match (id, ticket) {
                (Some(id), _) => format!("You have no open card #{id}. Your open cards are listed with the person's message."),
                (None, Some(t)) => format!("You have no open card about T-{t}."),
                (None, None) => "You have no open card to withdraw.".into(),
            });
        }
        let ids: Vec<u64> = mine.iter().map(|s| s.id).collect();
        self.drop_questions(squad, |s| ids.contains(&s.id));
        let mut said = Vec::new();
        for set in &mine {
            let question = clip(&super::decide::one_line(&set.questions[0].question), QUOTED);
            if let (Some(t), Some(journal)) = (set.ticket, self.journal(squad)) {
                let _ = journal.ticket_event(t, member, "withdrew", &question);
            }
            let shown = match set.ticket {
                Some(t) => say!(self, "news.card_withdrawn_on", name = member, id = t, question = question.clone()),
                None => say!(self, "news.card_withdrawn", name = member, question = question.clone()),
            };
            self.publish(Event::Notice { undo: None, session, text: shown });
            said.push(question);
        }
        let reason = why.map(|w| format!(" ({w})")).unwrap_or_default();
        self.brief_pm(squad, format!("{member} withdrew its card(s) after the person's word{reason}: {}", said.join("; ")), None);
        crate::daemon_log!("equipe: {squad}: {member} withdrew {} card(s)", mine.len());
        Ok(format!(
            "Withdrawn: the person no longer sees {}. Say in a line what you took back and why. If something is still \
theirs to choose, ask again with ask_decision — with the options as they stand now.",
            if mine.len() == 1 { "it".to_owned() } else { format!("these {}", mine.len()) }
        ))
    }

    /// `revise_ticket`: one of `member`'s own tickets that hasn't started —
    /// one it proposed, owns or is on — re-scoped (title, body, waits) or
    /// dropped, as the person said in conversation.
    pub(super) fn revise_ticket(&mut self, squad: &str, member: &str, session: SessionId, args: &Value) -> Result<String, String> {
        let ticket_id = board::arg_id(args, "id").ok_or("Which ticket? Give its id.")?;
        let role = self.sessions.get(&session).and_then(|s| s.member.as_ref()).map(|m| m.role.clone()).unwrap_or_default();
        let lang = self.squad_language(squad);
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        let mut ticket = journal.ticket(ticket_id).ok().flatten().ok_or_else(|| format!("There is no T-{ticket_id}."))?;
        let theirs = |who: &Option<String>| who.as_deref().is_some_and(|w| w == member || w == role);
        if !(ticket.created_by == member || theirs(&ticket.owner) || theirs(&ticket.assignee)) {
            return Err(format!("T-{ticket_id} isn't yours — ask the PM to change it."));
        }
        if !matches!(ticket.status, TicketStatus::Proposed | TicketStatus::Idea | TicketStatus::Todo | TicketStatus::Waiting) {
            return Err(format!("T-{ticket_id} is {} — from here it's the person's to change.", ticket.status.label()));
        }
        let mut changed: Vec<&'static str> = Vec::new();
        if let Some(title) = board::arg_str(args, "title").map(str::trim).filter(|t| !t.is_empty() && *t != ticket.title) {
            glance::check_title(lang, title)?;
            let _ = journal.ticket_event(ticket_id, member, glance::RETITLED, &ticket.title);
            ticket.title = title.to_owned();
            changed.push("title");
        }
        if let Some(body) = board::arg_str(args, "body").filter(|b| *b != ticket.body) {
            ticket.body = body.to_owned();
            changed.push("body");
        }
        let cancel = match board::arg_str(args, "status") {
            None => false,
            Some("cancelled") => true,
            Some(other) => return Err(format!("A ticket of yours can only be dropped (status cancelled), not set to {other}: the PM moves tickets.")),
        };
        if let Some(waits) = board::arg_ids(args, "depends_on").filter(|w| *w != ticket.depends_on) {
            self.check_dependencies(squad, ticket_id, &waits)?;
            ticket.depends_on = waits;
            changed.push("waits");
        }
        if cancel {
            if ticket.status == TicketStatus::Waiting || self.worker_on(squad, ticket_id).is_some() {
                return Err(format!("T-{ticket_id} has work on it — ask the PM or the person to cancel it."));
            }
            ticket.status = TicketStatus::Cancelled;
            changed.push("cancelled");
        }
        if changed.is_empty() {
            return Ok(format!("T-{ticket_id} already says that; nothing changed."));
        }
        let why = board::arg_str(args, "why").map(str::trim).filter(|w| !w.is_empty()).unwrap_or_default().to_owned();
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        journal.save_ticket(&ticket).map_err(|e| e.to_string())?;
        let _ = journal.ticket_event(ticket_id, member, "revised", &format!("{}{}", changed.join(", "), if why.is_empty() { String::new() } else { format!(": {why}") }));
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: ticket.clone() });
        let shown = if cancel {
            say!(self, "news.member_dropped", name = member, id = ticket_id, title = clip(&ticket.title, 80))
        } else {
            say!(self, "news.member_revised", name = member, id = ticket_id, title = clip(&ticket.title, 80))
        };
        self.publish(Event::Notice { undo: None, session, text: shown });
        self.brief_pm(squad, format!("{member} changed its own T-{ticket_id} ({}) after the person's word: {}{}", ticket.title, changed.join(", "), if why.is_empty() { String::new() } else { format!(" — {why}") }), None);
        Ok(format!("T-{ticket_id} changed ({}). Say in a line what you changed.", changed.join(", ")))
    }

    /// The note the person's next message to `id` opens with while its
    /// member has decision cards open: each card, and that a message which
    /// settles one has it withdrawn or replaced — never answered for them.
    pub(super) fn open_cards_note(&mut self, id: SessionId) -> Option<String> {
        let session = self.sessions.get(&id)?;
        let (squad, member) = (session.squad.clone()?, session.name()?.to_owned());
        let open: Vec<(u64, Option<TicketId>, String)> = self
            .question_sets(&squad)
            .iter()
            .filter(|s| s.member == member)
            .map(|s| (s.id, s.ticket, clip(&super::decide::one_line(&s.questions[0].question), QUOTED)))
            .collect();
        cards_note(&open)
    }
}

/// [`Runtime::open_cards_note`]'s words, of the cards open: (id, ticket, question).
pub(super) fn cards_note(open: &[(u64, Option<TicketId>, String)]) -> Option<String> {
    if open.is_empty() {
        return None;
    }
    let cards: Vec<String> = open
        .iter()
        .take(6)
        .map(|(id, ticket, question)| match ticket {
            Some(t) => format!("#{id} on T-{t}: \"{question}\""),
            None => format!("#{id}: \"{question}\""),
        })
        .collect();
    Some(format!(
        "your decision cards still open for the person: {}. When their message settles one, makes it moot or changes \
what it asks, withdraw it (withdraw_decision) or ask anew with ask_decision, and change your own tickets to match \
(revise_ticket) — say in a line what you changed. Never ask them to answer a card a certain way.",
        cards.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_members_open_cards_go_with_the_persons_message_and_say_to_withdraw_not_steer() {
        assert_eq!(cards_note(&[]), None);
        let note = cards_note(&[(7, Some(3567), "Which left menu?".into()), (9, None, "Dark or light?".into())]).unwrap();
        assert!(note.contains("#7 on T-3567: \"Which left menu?\"; #9: \"Dark or light?\""), "{note}");
        assert!(note.contains("withdraw_decision") && note.contains("revise_ticket") && note.contains("Never ask them to answer a card a certain way"), "{note}");
    }
}
