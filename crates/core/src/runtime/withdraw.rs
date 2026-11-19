//! Cards that wait on the person, taken away when they are no longer needed
//! (`crate::withdraw`): by the PM for any card on its squad, by a member for
//! its own (`withdraw_card`, and `withdraw_decision` from before it), and by
//! equipe for a ticket's secret, sign-in and `ask_person` cards when the
//! ticket's approach changes or it closes. Each card closes the way the
//! person closing it would — its event to every screen, so it leaves the
//! composer and Needs you — with the reason said in a line where they read
//! it; the member that raised it hears it was withdrawn (never "the person
//! said no"). And a PM reply that says a card is withdrawn or to be ignored
//! while that card is still up brings a reminder with its next turn, once.

use crate::withdraw::{self as wd, Card, Kind};

use super::*;

/// Where an open card lives, to close it the way it closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Secret,
    /// A command-line sign-in card.
    Cli,
    /// An approval waiting in `pending`: an `ask_person` or a site's sign-in.
    Approval,
    Questions,
    Accept,
}

/// One open card, with what closing it needs.
#[derive(Debug, Clone)]
struct Open {
    card: Card,
    source: Source,
    /// The conversation it was asked from.
    session: SessionId,
}

/// Why equipe took a ticket's cards away by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TicketTurn {
    /// The ticket is done, cancelled or merged.
    Closed,
    /// Sent back with what to change.
    SentBack,
    /// Brought back by the person after its run stalled.
    Restarted,
    /// Its plan (body) changed before it started again.
    Replanned,
}

impl TicketTurn {
    fn key(self) -> &'static str {
        match self {
            TicketTurn::Closed => "withdraw.why_closed",
            TicketTurn::SentBack => "withdraw.why_sent_back",
            TicketTurn::Restarted => "withdraw.why_restarted",
            TicketTurn::Replanned => "withdraw.why_replanned",
        }
    }
}

impl Runtime {
    /// Every card open for the person on `squad` that a member raised, by kind then age.
    fn open_cards(&mut self, squad: &str) -> Vec<Open> {
        let line = |text: &str| clip(&super::decide::one_line(text), wd::QUOTED);
        let mut out = Vec::new();
        for a in self.secret_asks(squad).iter().filter(|a| !a.member.is_empty()) {
            let keys = a.fields.iter().map(|f| f.key.as_str()).collect::<Vec<_>>().join(", ");
            let what = if a.why.trim().is_empty() { keys } else { format!("{keys} — {}", a.why) };
            out.push(Open { card: Card { kind: Kind::Secret, id: a.id, member: a.member.clone(), ticket: a.ticket, what: line(&what) }, source: Source::Secret, session: a.session });
        }
        for c in self.cli_sign_in_open(squad).into_iter().filter(|c| !c.member.is_empty() && c.server.is_none()) {
            let what = format!("{} — {}", c.name, c.why);
            out.push(Open { card: Card { kind: Kind::SignIn, id: c.id, member: c.member.clone(), ticket: c.ticket, what: line(&what) }, source: Source::Cli, session: c.session });
        }
        let mut approvals: Vec<&Pending> = self.pending.values().filter(|p| p.squad == squad && (p.approval.tool == CONFIRM || p.approval.tool == SIGN_IN_TOOL)).collect();
        approvals.sort_by_key(|p| p.approval.id);
        for p in approvals {
            let a = &p.approval;
            let kind = if a.tool == SIGN_IN_TOOL { Kind::SignIn } else { Kind::Ask };
            out.push(Open { card: Card { kind, id: a.id, member: a.member.clone(), ticket: a.ticket, what: line(&a.action) }, source: Source::Approval, session: a.session });
        }
        for s in self.question_sets(squad).iter() {
            let what = s.questions.first().map(|q| q.question.as_str()).unwrap_or_default();
            out.push(Open { card: Card { kind: Kind::Decision, id: s.id, member: s.member.clone(), ticket: s.ticket, what: line(what) }, source: Source::Questions, session: s.session });
        }
        for p in self.accepts(squad).iter().filter(|p| !p.from_words()) {
            let ids = p.tickets.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
            let what = if p.summary.trim().is_empty() { format!("{:?} {ids}", p.verdict).to_lowercase() } else { p.summary.clone() };
            out.push(Open { card: Card { kind: Kind::Accept, id: p.id, member: p.by.clone(), ticket: p.tickets.first().copied(), what: line(&what) }, source: Source::Accept, session: p.session });
        }
        out.sort_by_key(|o| (o.card.kind, o.card.id));
        out
    }

    /// `withdraw_card` (and `withdraw_decision`, `legacy`: decisions only,
    /// neither `id` nor `ticket` meaning all of the member's own): the PM
    /// takes back any open card on its squad, a member its own.
    pub(super) fn withdraw_card(&mut self, squad: &str, member: &str, session: SessionId, args: &Value, is_pm: bool, legacy: bool) -> Result<String, String> {
        let kind = match board::arg_str(args, "kind").map(str::trim).filter(|k| !k.is_empty()) {
            Some(word) => Some(Kind::parse(word).ok_or_else(|| format!("No card kind \"{word}\": say secret, sign_in, ask, decision or accept."))?),
            None if legacy => Some(Kind::Decision),
            None => None,
        };
        let id = args.get("id").and_then(Value::as_u64);
        let ticket = board::arg_id(args, "ticket");
        let why = board::arg_str(args, "why").map(str::trim).filter(|w| !w.is_empty()).map(str::to_owned);
        let mine_only = !is_pm || legacy;
        let reachable: Vec<Open> = self.open_cards(squad).into_iter().filter(|o| !mine_only || o.card.member == member).collect();
        let whose = if mine_only { "of yours" } else { "on the squad" };
        // Neither which card nor which ticket: the list of what may be withdrawn.
        if id.is_none() && ticket.is_none() && !legacy {
            let listed: Vec<Card> = reachable.iter().filter(|o| kind.is_none_or(|k| o.card.kind == k)).map(|o| o.card.clone()).collect();
            if listed.is_empty() {
                return Ok(format!("No card {whose} is open for the person now."));
            }
            return Ok(format!(
                "The cards {whose} open for the person now:\n{}\nWithdraw one with its `kind` and `id` (or a ticket's with `ticket`), and a one-line `why`.",
                wd::listing(&listed)
            ));
        }
        let picked: Vec<Open> = reachable
            .iter()
            .filter(|o| kind.is_none_or(|k| o.card.kind == k) && id.is_none_or(|i| o.card.id == i) && ticket.is_none_or(|t| o.card.ticket == Some(t)))
            .cloned()
            .collect();
        if picked.is_empty() {
            let open: Vec<Card> = reachable.iter().map(|o| o.card.clone()).collect();
            let named = match (kind, id, ticket) {
                (_, Some(id), _) => format!("No card {id} {whose} is open"),
                (Some(k), None, Some(t)) => format!("No {} card {whose} about T-{t} is open", k.word()),
                (None, None, Some(t)) => format!("No card {whose} about T-{t} is open"),
                (_, None, None) => format!("No card {whose} is open"),
            };
            return Err(if open.is_empty() { format!("{named}; none is.") } else { format!("{named}. Open now:\n{}", wd::listing(&open)) });
        }
        if id.is_some() && kind.is_none() && picked.iter().any(|o| o.card.kind != picked[0].card.kind) {
            return Err(format!("More than one card has the number {}: say which with `kind`.\n{}", id.unwrap_or_default(), wd::listing(&picked.iter().map(|o| o.card.clone()).collect::<Vec<_>>())));
        }
        let why = match why {
            Some(why) => why,
            None if legacy => String::new(),
            None => return Err("Say in one line why it is no longer needed (`why`): the person reads it where the card was.".into()),
        };
        let waiting: Vec<TicketId> = {
            let tickets = self.tickets(squad);
            let mut ids: Vec<TicketId> = picked.iter().filter_map(|o| o.card.ticket).filter(|t| tickets.iter().any(|x| x.id == *t && x.status == TicketStatus::Waiting)).collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        let gone: Vec<Card> = picked.iter().map(|o| o.card.clone()).collect();
        self.close_cards(squad, picked, member, Some(session), &why, true);
        let them = if gone.len() == 1 { "it".to_owned() } else { format!("these {}", gone.len()) };
        let still_waits = if is_pm && !waiting.is_empty() {
            let ids = waiting.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
            format!(" {ids} still waits on the person for its checklist: if that is moot too, `unblock` it.")
        } else {
            String::new()
        };
        Ok(format!(
            "Withdrawn: the person no longer sees {them} ({}), and reads your reason where it was. Say in a line what you took \
back and why.{still_waits} If something is still theirs to choose, ask anew — with the options as they stand now.",
            gone.iter().map(|c| format!("{} {}", c.kind.word(), c.id)).collect::<Vec<_>>().join(", ")
        ))
    }

    /// `ticket`'s secret, sign-in and `ask_person` cards taken away by equipe:
    /// its approach changed or it closed (`turn`). The member that raised
    /// each hears it with its next turn; nobody is woken for it.
    pub(super) fn withdraw_ticket_cards(&mut self, squad: &str, ticket: TicketId, turn: TicketTurn) -> usize {
        let open: Vec<Open> = self.open_cards(squad).into_iter().filter(|o| o.card.ticket == Some(ticket) && o.card.kind.follows_its_ticket()).collect();
        if open.is_empty() {
            return 0;
        }
        let count = open.len();
        let why = crate::i18n::core(self.config.lang).get(turn.key()).to_owned();
        self.close_cards(squad, open, "equipe", None, &why, false);
        crate::daemon_log!("equipe: {squad}: {count} card(s) about T-{ticket} withdrawn ({turn:?})");
        count
    }

    /// Close `cards` as `by` (a member, or `equipe`), said from conversation
    /// `from`; `wake`: a member held up on one is set going again with the
    /// news, rather than hearing it at its next turn.
    fn close_cards(&mut self, squad: &str, cards: Vec<Open>, by: &str, from: Option<SessionId>, why: &str, wake: bool) {
        let mut said = Vec::new();
        for open in cards {
            let Open { card, source, session } = open;
            match source {
                Source::Secret => {
                    self.secret_asks(squad).retain(|a| a.id != card.id);
                    self.keep_secret_asks(squad);
                    self.publish(Event::SecretClosed { squad: squad.to_owned(), id: card.id, saved: false });
                }
                Source::Cli => {
                    self.close_card(squad, card.id);
                }
                Source::Approval => {
                    // Its member hears it in the answer, as it hears a no.
                    let reason = if why.is_empty() { format!("{by} withdrew this card.") } else { format!("{by} withdrew this card: {why}.") };
                    self.answer(card.id, Decision::Deny, &format!("{reason} Don't do it as asked; go on the way the PM or the person says."));
                }
                Source::Questions => {
                    self.drop_questions(squad, |s| s.id == card.id);
                    self.forget_due(squad, card.id);
                }
                Source::Accept => {
                    if self.take_accept(squad, card.id).is_some() {
                        self.publish(Event::AcceptClosed { squad: squad.to_owned(), id: card.id, confirmed: false, tickets: Vec::new() });
                    }
                }
            }
            if let (Some(t), Some(journal)) = (card.ticket, self.journal(squad)) {
                let detail = if why.is_empty() { card.what.clone() } else { format!("{} — {why}", card.what) };
                let _ = journal.ticket_event(t, by, "withdrew", &detail);
            }
            // The person reads it where the card was asked, and where it was withdrawn from.
            let own = card.member == by;
            let shown = match (card.ticket, why.is_empty(), own) {
                (Some(t), true, _) => say!(self, "news.card_withdrawn_on", name = by, id = t, question = card.what.clone()),
                (None, true, _) => say!(self, "news.card_withdrawn", name = by, question = card.what.clone()),
                (Some(t), false, true) => say!(self, "news.card_withdrawn_on_why", name = by, id = t, what = card.what.clone(), why = why),
                (None, false, true) => say!(self, "news.card_withdrawn_why", name = by, what = card.what.clone(), why = why),
                (Some(t), false, false) => say!(self, "news.card_withdrawn_for_on", name = by, member = card.member.clone(), id = t, what = card.what.clone(), why = why),
                (None, false, false) => say!(self, "news.card_withdrawn_for", name = by, member = card.member.clone(), what = card.what.clone(), why = why),
            };
            let asked_in = Some(session).filter(|s| self.sessions.contains_key(s)).or_else(|| self.member_id(squad, &card.member));
            let mut told: Vec<SessionId> = Vec::new();
            for to in [from, asked_in].into_iter().flatten() {
                if !told.contains(&to) {
                    told.push(to);
                    self.publish(Event::Notice { undo: None, session: to, text: shown.clone() });
                }
            }
            // The member that raised it hears it was withdrawn — not that the person said no.
            if !own && source != Source::Approval && source != Source::Accept {
                let because = if why.is_empty() { String::new() } else { format!(": {why}") };
                let line = format!(
                    "{by} withdrew your card asking the person for {}{because}. It is no longer in front of them — don't ask for it again \
unless the PM or the person says so; go on without it.",
                    card.what
                );
                match asked_in {
                    Some(to) if wake && matches!(source, Source::Secret | Source::Cli) => {
                        let _ = self.internal.send(Internal::Wake { session: to, prompt: format!("[equipe: {line}]") });
                    }
                    Some(to) => {
                        if let Some(s) = self.sessions.get_mut(&to) {
                            s.briefing.push(line);
                        }
                        self.save_waiting(to);
                    }
                    None => {}
                }
            }
            said.push(card.line());
        }
        // The PM hears what it didn't do itself.
        if self.pm_of(squad).is_some_and(|pm| from != Some(pm)) && !said.is_empty() {
            let reason = if why.is_empty() { String::new() } else { format!(" ({why})") };
            self.brief_pm(squad, format!("{by} withdrew card(s){reason}: {}", said.join("; ")), None);
        }
        crate::daemon_log!("equipe: {squad}: {by} withdrew {} card(s)", said.len());
    }

    /// The PM's turn ended with `reply`: when it tells the person a card is
    /// withdrawn, not needed or to be ignored while cards are still up, its
    /// next turn is reminded to take them away — once for each card.
    pub(super) fn check_said_withdrawn(&mut self, squad: &str, reply: &str) {
        if !wd::says_withdrawn(reply) {
            return;
        }
        let open: Vec<Card> = self.open_cards(squad).into_iter().map(|o| o.card).collect();
        let named: Vec<Card> = open.iter().filter(|c| c.named_in(reply)).cloned().collect();
        let suspects = if named.is_empty() { open } else { named };
        let reminded = self.withdraw_reminded.entry(squad.to_owned()).or_default();
        let fresh: Vec<Card> = suspects.into_iter().filter(|c| !reminded.contains(&(c.kind, c.id))).collect();
        if fresh.is_empty() {
            return;
        }
        reminded.extend(fresh.iter().map(|c| (c.kind, c.id)));
        crate::daemon_log!("equipe: {squad}: the PM said a card was withdrawn while {} still up; reminded", fresh.len());
        self.brief_pm(squad, wd::reminder(&fresh), None);
    }
}
