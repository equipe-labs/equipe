//! The squad's autonomy policy at work ([`crate::autonomy`]): the person sets
//! it on their screen or the command line (the PM may only propose a change,
//! on a card — `change_setting`), grants and revokes standing allowances from
//! request cards, and changes the choices members took on their own. Here the
//! daemon keeps it, tells every screen, allows what an allowance or the
//! policy covers with a one-line note, and takes decide-and-tell choices on
//! their member's recommendation once their window has passed.

use super::*;
use crate::autonomy::{self as policy, Due, Request, Store};
use crate::protocol::autonomy::{self as au, AutoPick, Kind, Level, Op, Use};
use equipe_protocol::decide::{self, Answer, QuestionSet};

/// How long a member's own request waits for the person when the member
/// didn't stay for the answer: a day, then it is declined as unanswered.
pub(super) const LATER_WAIT: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a member waits on its own request before it is let go to other
/// work: most answers come within it (in one squad's two weeks, a yes took
/// four minutes on average and often under one), and none is worth more of
/// its idle time.
pub(super) const HOLD: Duration = Duration::from_secs(3 * 60);

impl Runtime {
    /// The squad's autonomy as its journal keeps it.
    pub(super) fn autonomy_store(&mut self, squad: &str) -> Store {
        self.journal(squad).map(Journal::autonomy).unwrap_or_default()
    }

    /// How the squad is let to work: [`au::Autonomy::Standard`] unless the person
    /// set it to full.
    pub(super) fn autonomy_mode(&self, squad: &str) -> au::Autonomy {
        self.journals.get(squad).map(|j| j.autonomy().mode).unwrap_or_default()
    }

    /// The person set the squad's autonomy mode.
    pub(super) fn set_autonomy_mode(&mut self, squad: &str, mode: au::Autonomy) {
        let mut store = self.autonomy_store(squad);
        if store.mode == mode {
            return;
        }
        store.mode = mode;
        crate::daemon_log!("equipe: {squad}'s autonomy mode is now {}", mode.code());
        self.save_autonomy(squad, store);
    }

    /// The squad's policy as it stands, its merge level read from `pm_accepts`.
    pub(super) fn autonomy_policy(&mut self, squad: &str) -> au::Policy {
        let pm_accepts = self.journals.get(squad).is_some_and(Journal::pm_accepts);
        self.autonomy_store(squad).state(pm_accepts).policy
    }

    /// Keep `store` (what has run its course let go) and tell every screen.
    pub(super) fn save_autonomy(&mut self, squad: &str, mut store: Store) {
        store.tidy(now_secs());
        if let Some(Err(err)) = self.journal(squad).map(|j| j.set_autonomy(&store)) {
            crate::daemon_log!("equipe: {squad}'s autonomy policy wasn't kept: {err}");
        }
        self.publish_autonomy(squad);
    }

    /// What screens show of the squad's autonomy.
    pub(super) fn autonomy_state(&mut self, squad: &str) -> au::State {
        let pm_accepts = self.journals.get(squad).is_none_or(Journal::pm_accepts);
        self.autonomy_store(squad).state(pm_accepts)
    }

    pub(super) fn publish_autonomy(&mut self, squad: &str) {
        let state = self.autonomy_state(squad);
        self.publish(Event::AutonomyChanged { squad: squad.to_owned(), state });
    }

    /// The person set how far members go with `kind`. Merges are the PM's
    /// acceptance: the same switch as "the PM accepts the team's work".
    pub(super) fn set_autonomy(&mut self, squad: &str, kind: Kind, level: Level) {
        let mut store = self.autonomy_store(squad);
        if kind == Kind::Merge {
            let on = level != Level::Ask;
            if let Some(journal) = self.journal(squad) {
                let _ = journal.set_pm_accepts(on);
            }
            store.policy.merge = level;
            self.publish(Event::PmAcceptsChanged { squad: squad.to_owned(), on });
        } else if store.policy.set(kind, level).is_err() {
            return self.squad_error(squad, say!(self, "autonomy.floor", kind = self.kind_words(kind)));
        }
        crate::daemon_log!("equipe: {squad}'s autonomy: {} is now {}", kind.code(), level.code());
        self.save_autonomy(squad, store);
    }

    /// How long a decide-and-tell choice waits for the person.
    pub(super) fn set_autonomy_window(&mut self, squad: &str, minutes: u32) {
        let mut store = self.autonomy_store(squad);
        store.policy.window_minutes = minutes.min(24 * 60);
        self.save_autonomy(squad, store);
    }

    /// "Allow for this kind" on request `approval`: the standing allowance
    /// its card offered, granted for `days`, and the request allowed. Only
    /// what the card offered — a request without one is left as it is.
    pub(super) fn allow_kind(&mut self, squad: &str, approval: u64, days: u32) {
        let Some(pending) = self.pending.get(&approval).filter(|p| p.squad == squad) else { return };
        let a = pending.approval.clone();
        let Some(ask) = a.ask.clone() else { return };
        let Some(scope) = ask.standing.clone().filter(|_| ask.kind.allowable()) else { return };
        let now = now_secs();
        let mut store = self.autonomy_store(squad);
        let Some(id) = store.grant(ask.kind, scope.clone(), &a.member, days, now) else { return };
        // This request is the allowance's first use: a record it makes can be cleaned up under it.
        store.record(Use { allowance: id, member: a.member.clone(), ticket: a.ticket, op: ask.op, what: first_line(&a.action).to_owned(), at: now });
        self.save_autonomy(squad, store);
        let days = days.clamp(1, au::MOST_DAYS);
        self.audit(squad, a.ticket, "you", "allowed for this kind", &format!("{} ({days} d)", scope.words));
        let text = say!(self, "autonomy.granted", scope = scope.words, days = days);
        self.publish(Event::Notice { undo: None, session: a.session, text });
        self.answer(approval, Decision::Allow, "");
    }

    /// The person took standing allowance `id` back.
    pub(super) fn revoke_allowance(&mut self, squad: &str, id: u64) {
        let mut store = self.autonomy_store(squad);
        if store.revoke(id, now_secs()) {
            let words = store.allowances.iter().find(|a| a.id == id).map(|a| a.scope.words.clone()).unwrap_or_default();
            self.audit(squad, None, "you", "revoked", &words);
            self.save_autonomy(squad, store);
        }
    }

    /// A member's request the squad lets through without the person: a
    /// standing allowance covers it, or the policy lets production data go
    /// ahead. The tool's answer then, with the one-line note posted; `None`:
    /// the person is asked. `on`: the tool and what it does.
    pub(super) fn allowed_standing(&mut self, squad: &str, session: SessionId, member: &str, ticket: Option<TicketId>, ask: &au::Ask, (tool, action): (&str, &str)) -> Option<String> {
        if ask.kind.is_floor() {
            return None;
        }
        let now = now_secs();
        let mut store = self.autonomy_store(squad);
        let request = Request { kind: ask.kind, member, tool, action, system: ask.standing.as_ref().and_then(|s| s.system.as_deref()), op: ask.op };
        let what = first_line(action).to_owned();
        let (line, audit) = if let Some(a) = policy::covering(&store.allowances, &store.uses, &request, now).cloned() {
            store.record(Use { allowance: a.id, member: member.to_owned(), ticket, op: ask.op, what: what.clone(), at: now });
            (say!(self, "autonomy.used", name = member, action = clip(&what, 160), scope = a.scope.words), format!("allowed under \"{}\"", a.scope.words))
        } else if policy::policy_allows(&store.policy, ask) {
            (say!(self, "autonomy.policy_used", name = member, action = clip(&what, 160)), "allowed by the squad's policy".to_owned())
        } else {
            return None;
        };
        self.save_autonomy(squad, store);
        self.audit(squad, ticket, "equipe", &audit, action);
        crate::daemon_log!("equipe: {squad}: {member}'s request {audit}");
        // A member's own request is told in one line; an engine's command under a rule only logged.
        if tool == CONFIRM {
            self.publish(Event::Notice { undo: None, session, text: line });
        }
        Some(format!("The person allows this kind of request standing ({audit}): go ahead, exactly as you described it: {what}"))
    }

    /// What the person sees of a decision kind, in their language.
    pub(super) fn kind_words(&self, kind: Kind) -> String {
        crate::i18n::core(self.config.lang).get(&kind.words_key()).to_owned()
    }

    /// What the person sees of a level, in their language.
    pub(super) fn level_words(&self, level: Level) -> String {
        crate::i18n::core(self.config.lang).get(&format!("autonomy.level.{}", level.code())).to_owned()
    }

    /// A question set under the policy, before it is put to the person:
    /// refused (the member's own call, just-do), taken at once on the
    /// member's recommendation, or left to be taken at its due time — or
    /// asked as ever. `declared`: each question's `kind`, as the member said.
    pub(super) fn decide_under_policy(&mut self, squad: &str, set: &QuestionSet, role: &str, declared: &[Option<Kind>], low_impact: bool) -> PolicyOutcome {
        if set.review || set.retro.is_some() {
            return PolicyOutcome::Ask;
        }
        let store = self.autonomy_store(squad);
        let kind = policy::set_kind(set, role, declared);
        let level = store.policy.level(kind);
        if !matches!(kind, Kind::Design | Kind::Internal) || level == Level::Ask {
            return PolicyOutcome::Ask;
        }
        let recommended = au::recommended_answers(set);
        if kind == Kind::Internal && level == Level::JustDo {
            return PolicyOutcome::Yours;
        }
        // A choice between options with no recommendation marked: the member is asked for one first.
        let choices = set.questions.iter().all(|q| q.options.len() >= 2 && !q.board);
        let Some(answers) = recommended else { return if kind == Kind::Design && choices { PolicyOutcome::Unmarked } else { PolicyOutcome::Ask } };
        let due = au::due_at(set.asked_at, &store.policy, low_impact || level == Level::JustDo);
        if due <= set.asked_at { PolicyOutcome::Now(kind, answers) } else { PolicyOutcome::Later(kind, due) }
    }

    /// Keep set `id` to be taken on its recommendation at `at`.
    pub(super) fn keep_due(&mut self, squad: &str, set: u64, kind: Kind, at: u64) {
        let mut store = self.autonomy_store(squad);
        store.due.retain(|d| d.set != set);
        store.due.push(Due { set, kind, at });
        self.save_autonomy(squad, store);
    }

    /// Set `id` no longer waits to be taken on its recommendation: the person settled it.
    pub(super) fn forget_due(&mut self, squad: &str, id: u64) {
        let mut store = self.autonomy_store(squad);
        if store.due.iter().any(|d| d.set == id) {
            store.due.retain(|d| d.set != id);
            self.save_autonomy(squad, store);
        }
    }

    /// The member took `set` on its recommendation (`answers`): kept a week
    /// for the person to change, noted on its ticket, told in one line. The
    /// answers' message, as the member reads it.
    pub(super) fn record_pick(&mut self, squad: &str, set: &QuestionSet, answers: &[Answer], kind: Kind) -> String {
        let now = now_secs();
        let message = decide::answer_message(set, answers);
        if let (Some(ticket), Some(journal)) = (set.ticket, self.journal(squad)) {
            let _ = journal.ticket_event(ticket, "equipe", "decided", &format!("Taken on {}'s recommendation (decide-and-tell):\n{message}", set.member));
        }
        let wait = crate::retro::Wait { kind: "decision".into(), member: set.member.clone(), ticket: set.ticket, what: set.questions.first().map(|q| q.question.clone()).unwrap_or_default(), asked: set.asked_at, answered: Some(now), outcome: "auto-picked".into() };
        if let Some(journal) = self.journal(squad) {
            let _ = journal.record_wait(&wait);
        }
        let pick = AutoPick { set: set.clone(), answers: answers.to_vec(), kind, at: now, reopened: false };
        let view = pick.view();
        let mut store = self.autonomy_store(squad);
        store.due.retain(|d| d.set != set.id);
        store.picks.retain(|p| p.set.id != set.id);
        store.picks.push(pick);
        self.save_autonomy(squad, store);
        let why = if view.why.is_empty() { String::new() } else { format!(" ({})", clip(&view.why, 120)) };
        let text = say!(self, "autonomy.picked", name = set.member, picked = view.picked, why = why);
        self.publish(Event::Notice { undo: None, session: set.session, text: text.clone() });
        self.brief_pm(squad, format!("{} took its recommendation on \"{}\" (decide-and-tell): {}", set.member, view.question, view.picked), Some(text));
        message
    }

    /// [`Self::record_pick`] for a set that waited on the person's screen
    /// past its window: closed there, and the answers sent to its member.
    pub(super) async fn auto_pick(&mut self, squad: &str, set: QuestionSet, answers: Vec<Answer>, kind: Kind) {
        self.publish(Event::QuestionsClosed { squad: squad.to_owned(), id: set.id, answers: Some(answers.clone()) });
        let message = self.record_pick(squad, &set, &answers, kind);
        let told = format!(
            "{message}\n(The person didn't answer within the squad's window, so equipe took your recommendation under its \
autonomy policy. They were told in one line and may still change it: if they do, their pick comes to you as a message.)"
        );
        if self.resume_asked(squad, &set, &told).await {
            return;
        }
        let to = if self.sessions.contains_key(&set.session) { Some(set.session) } else { self.member_id(squad, &set.member).map(|m| self.route(m)) };
        if let Some(to) = to {
            Box::pin(self.submit(to, told)).await;
        }
    }

    /// Every minute: the decide-and-tell choices whose window has passed are
    /// taken on their recommendation, unless the person answered or the
    /// policy changed since.
    pub(super) async fn autonomy_due(&mut self, now: u64) {
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            let store = self.autonomy_store(&squad);
            if store.due.is_empty() {
                continue;
            }
            let (ripe, waiting): (Vec<Due>, Vec<Due>) = store.due.iter().cloned().partition(|d| d.at <= now);
            if ripe.is_empty() {
                continue;
            }
            let mut kept = store.clone();
            kept.due = waiting;
            self.save_autonomy(&squad, kept);
            for due in ripe {
                if store.policy.level(due.kind) == Level::Ask {
                    continue;
                }
                let Some(set) = self.question_sets(&squad).iter().find(|s| s.id == due.set).cloned() else { continue };
                let Some(answers) = au::recommended_answers(&set) else { continue };
                let Some(set) = self.take_questions(&squad, set.id) else { continue };
                self.auto_pick(&squad, set, answers, due.kind).await;
            }
        }
    }

    /// The person opens a choice a member took on its own again, to choose
    /// themselves: the same card, back in front of them.
    pub(super) fn reopen_auto_pick(&mut self, squad: &str, set: u64) {
        let mut store = self.autonomy_store(squad);
        let Some(pick) = store.picks.iter_mut().find(|p| p.set.id == set && !p.reopened) else { return };
        pick.reopened = true;
        let mut again = pick.set.clone();
        self.save_autonomy(squad, store);
        if self.question_sets(squad).iter().any(|s| s.id == set) {
            return;
        }
        again.asked_at = now_secs();
        self.question_sets(squad).push(again.clone());
        self.save_questions(squad);
        self.publish(Event::QuestionsAsked { squad: squad.to_owned(), questions: again });
    }

    /// The person answered a choice they opened again: what the member is told.
    pub(super) fn reopened_message(&mut self, squad: &str, set: &QuestionSet, answers: &[Answer], message: &str) -> Option<String> {
        let store = self.autonomy_store(squad);
        let pick = store.picks.iter().find(|p| p.set.id == set.id && p.reopened)?;
        Some(if pick.answers == answers {
            format!("The person looked at the choice you took on your own and kept it:\n{message}\nNothing to change.")
        } else {
            format!(
                "The person changed the choice you took on your own (decide-and-tell):\n{message}\nRework what you built on \
your recommendation to their pick, and say what changed in a line."
            )
        })
    }

    /// A member's own request has waited [`HOLD`]: the member is told to
    /// carry on with other work; the card stays, and the answer reaches it as
    /// a message ([`Self::later_answered`]).
    pub(super) fn release_ask(&mut self, id: u64) {
        let Some(pending) = self.pending.get_mut(&id) else { return };
        let Some(ask) = pending.approval.ask.as_mut().filter(|a| !a.later) else { return };
        ask.later = true;
        let (kept, _) = oneshot::channel();
        let reply = std::mem::replace(&mut pending.reply, kept);
        self.approvals_turn.remove(&id);
        let _ = reply.send(ToolResult::ok(STILL_ASKED));
    }

    /// The person answered a request the member didn't wait for: back to its
    /// ticket if that waits on it alone, else to the member as a message.
    pub(super) async fn later_answered(&mut self, squad: &str, approval: Approval, said: String) {
        let action = clip(first_line(&approval.action), 300);
        let on_ticket = approval.ticket.is_some();
        let prompt = later_prompt(&action, &said, on_ticket);
        if let Some(ticket) = approval.ticket
            && self.resume_waiting_on(squad, ticket, &[super::decide::one_line(&approval.action)], &prompt).await
        {
            return;
        }
        let to = if self.sessions.contains_key(&approval.session) { Some(approval.session) } else { self.member_id(squad, &approval.member).map(|m| self.route(m)) };
        if let Some(to) = to {
            self.set_going(to, prompt).await;
        }
    }
}

/// What the policy makes of a question set.
pub(super) enum PolicyOutcome {
    /// Put to the person as ever.
    Ask,
    /// The member's own call: not sent.
    Yours,
    /// Taken on its recommendation at once.
    Now(Kind, Vec<Answer>),
    /// Put to the person, and taken on its recommendation at this time
    /// unless they answer first.
    Later(Kind, u64),
    /// A design or product choice without a recommended option: the member
    /// marks the one it would take, so it can carry on with it.
    Unmarked,
}

/// Why a choice comes back to its member to mark a recommendation.
pub(super) const UNMARKED: &str = "Not sent yet: this is a design or product choice, and under the squad's autonomy policy \
you decide and tell — so mark the option you would take with `recommended` and ask again. Then carry on with it: it is taken \
unless the person picks otherwise, and they can change it later. (Asked again exactly as it is, it goes to the person as a plain \
question.)";

/// Why an internal question isn't sent under just-do: the call's own result.
pub(super) const YOURS: &str = "Not sent: under the squad's autonomy policy, questions about the squad's own process \
(how to file or title a ticket, whether to close a duplicate, which follow-up to register first) are yours to settle. \
Decide, note your choice and why on the ticket in one line, and carry on. Nothing was sent to the person.";

/// The message a member gets when the person answers a request it didn't wait for.
fn later_prompt(action: &str, said: &str, on_ticket: bool) -> String {
    let mut prompt = format!("[equipe] The person answered your request (\u{201c}{action}\u{201d}): {said}\n");
    prompt.push_str(if on_ticket {
        "Carry on in light of their answer — look at where things stand first, and don't redo what is done. Then end the \
turn in one of the ways to finish: `submit_for_review` when it's done, `block_on_person` when only the person can do \
what's left, or `hand_back`."
    } else {
        "Carry on in light of their answer."
    });
    prompt
}

/// The ops a request says it does.
pub(super) fn op_of(args: &Value) -> Option<Op> {
    args.get("op").and_then(Value::as_str).and_then(Op::parse)
}
