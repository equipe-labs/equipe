//! Each ticket's model and effort, made so ([`crate::tiers`] has the rules):
//! picked when a lane starts, kept on the ticket with why, raised one tier
//! when work comes back wrong (a send-back, a failed check or design QA, a
//! loop, a member that hands its ticket back as beyond it), and set by the
//! person whenever they like. Everything is logged, and every raise is told on the ticket and in
//! the PM's conversation.

use equipe_protocol::tiers::{ModelChoice, ModelTier, SendBackReason, TicketModel};

use super::*;
use crate::tiers::{self as rules, Next};

/// What a handoff note carries of each earlier hand-in and send-back.
const HANDOFF_CLIP: usize = 600;

impl Runtime {
    /// What `ticket`'s next lane on member `id` starts on, under the squad's
    /// model choice; `None` when `id` isn't a member of `squad`.
    pub(super) fn choose_model(&mut self, squad: &str, id: SessionId, ticket: &Ticket) -> Option<rules::Choice> {
        let batch = self.design_batch_kind(squad, ticket.id);
        let design_qa = batch.is_some() || self.checked_by(squad, ticket.id).is_some();
        let choice = self.journal(squad).map(Journal::model_choice).unwrap_or_default();
        let (name, role, engine) = self.sessions.get(&id).and_then(|s| s.member.as_ref().map(|m| (m.name.clone(), m.role.clone(), s.engine)))?;
        // Only a model the person chose is a floor: the member's own `/model`, or a model they set for its role.
        let person = self.journal(squad).is_some_and(|j| j.person_models().contains(&name) || j.role_models().iter().any(|r| r.role == role && r.engine == engine && r.model.is_some()));
        let qa_tier = match (&batch, design_qa) {
            // A daily review on the cheap tier, its audit on the top one.
            (Some(kind), _) => self.batch_tier(squad, kind, &name, &role, engine),
            (None, true) => self.qa_tier(squad, ticket, &name, &role, engine),
            (None, false) => None,
        };
        let s = self.sessions.get(&id)?;
        let effort = effort_of(s);
        let infos: Vec<codex_app::ModelInfo> = match s.engine {
            Engine::Codex => self.codex_models.as_ref().map(|(_, models)| models.clone()).unwrap_or_default(),
            Engine::Claude => Vec::new(),
        };
        let catalog: Vec<rules::Offer<'_>> = infos.iter().map(|m| rules::Offer { id: &m.id, efforts: &m.efforts, default_effort: m.default_effort.as_deref() }).collect();
        let member = rules::Member { engine: s.engine, model: s.model.as_deref(), effort: effort.as_deref(), catalog: &catalog, person };
        let work = rules::Work { role: &role, kind: ticket.kind, title: &ticket.title, body: &ticket.body, design_qa, qa_tier, edits: s.edits() };
        let rule = rules::rule(&work);
        Some(rules::choose(ticket.model.as_ref(), rule.as_ref(), &member, choice))
    }

    /// `ticket` was sent back by `by` for `why`: kept, and — while the squad
    /// lets equipe choose models — each of the first two correctness
    /// send-backs raises its next lane one tier, the third puts it to the PM.
    pub(super) fn after_send_back(&mut self, squad: &str, ticket: TicketId, by: &str, why: SendBackReason) {
        let Some(journal) = self.journal(squad) else { return };
        if let Err(err) = journal.record_send_back(ticket, by, why) {
            crate::daemon_log!("equipe: {squad}: T-{ticket}'s send-back not kept: {err}");
        }
        if journal.model_choice() == ModelChoice::Off {
            return;
        }
        let times = journal.send_backs(ticket, SendBackReason::Correctness).unwrap_or(0);
        match rules::after_send_back(why, times) {
            Next::Nothing => {}
            Next::StepUp => self.escalate(squad, ticket, &format!("sent back for correctness {times} times")),
            Next::PmDecides => {
                let Some(t) = self.find_ticket(squad, ticket) else { return };
                if let Some(journal) = self.journal(squad) {
                    let _ = journal.ticket_event(ticket, "equipe", "model", &format!("Sent back for correctness {times} times: a stronger model isn't the answer now; the PM decides."));
                }
                let line = format!(
                    "equipe: T-{ticket} ({}) has come back wrong {times} times, once already on a stronger model. Don't send it round again as it is: \
decide — split it into smaller tickets, rewrite what it asks and how it is checked, or ask the person what they meant.",
                    t.title
                );
                let shown = say!(self, "news.model_pm_decides", id = ticket, title = t.title, times = times);
                crate::daemon_log!("equipe: {squad}: T-{ticket} sent back {times} times for correctness: the PM decides");
                self.brief_pm(squad, line, Some(shown));
            }
        }
    }

    /// `ticket`'s next lane starts one tier up, in a fresh conversation with
    /// a handoff note — its branch, worktree and record kept. `why`, in a few
    /// English words, goes on its record and in the log; the person reads it
    /// in the PM's conversation. A ticket the person pinned stays as it is.
    pub(super) fn escalate(&mut self, squad: &str, ticket: TicketId, why: &str) {
        let Some(t) = self.find_ticket(squad, ticket) else { return };
        let now = t.model.clone().unwrap_or_default();
        let Some(up) = rules::step_up(&now) else {
            if let Some(journal) = self.journal(squad) {
                let _ = journal.ticket_event(ticket, "equipe", "model", &format!("{why}; kept at the tier you pinned."));
            }
            crate::daemon_log!("equipe: {squad}: T-{ticket} {why}, pinned by the person: not raised");
            return;
        };
        let note = self.handoff_note(squad, &t, up.tier, up.tier == now.tier);
        let Some(journal) = self.journal(squad) else { return };
        if let Err(err) = journal.set_ticket_model(ticket, Some(&up)) {
            return crate::daemon_log!("equipe: {squad}: T-{ticket} not raised: {err}");
        }
        let _ = journal.record_model_choice(ticket, "escalated", None, &up);
        let tier = up.tier.map_or("its member's own model", ModelTier::code);
        // At the top already (or on a model equipe can't place): a fresh start is all that's left.
        let top = up.tier == now.tier;
        let what = if top { format!("{why}: already at {tier}, so its next lane starts fresh, with a handoff note.") } else { format!("{why}: its next lane starts on {tier}, in a fresh conversation with a handoff note.") };
        let _ = journal.ticket_event(ticket, "equipe", "model raised", &what);
        let _ = journal.ticket_event(ticket, "equipe", "handoff", &note);
        crate::daemon_log!("equipe: {squad}: T-{ticket} {} ({why})", if top { format!("starts fresh at {tier}") } else { format!("raised to {tier}") });
        let shown = if top {
            say!(self, "news.model_fresh", id = ticket, title = t.title)
        } else {
            say!(self, "news.model_raised", id = ticket, title = t.title, tier = self.tier_word(up.tier))
        };
        let line = if top {
            format!("equipe: T-{ticket} ({}) was {why}; it is on the strongest model and effort already, so its next lane starts fresh with a handoff note. If it comes back wrong again, decide what to do with it.", t.title)
        } else {
            format!("equipe: T-{ticket} ({}) was {why}, so its next lane starts one step stronger ({tier}), fresh, with a handoff note. Nothing to do about it.", t.title)
        };
        self.brief_pm(squad, line, Some(shown));
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: Ticket { model: Some(up), ..t } });
    }

    /// A tier as the person reads it, in their language.
    fn tier_word(&self, tier: Option<ModelTier>) -> String {
        let key = match tier {
            Some(ModelTier::Light) => "tier.light",
            Some(ModelTier::Standard) => "tier.standard",
            Some(ModelTier::Strong) => "tier.strong",
            Some(ModelTier::Deep) => "tier.deep",
            Some(ModelTier::Max) => "tier.max",
            None => "tier.own",
        };
        crate::i18n::core(self.config.lang).get(key).to_owned()
    }

    /// What a ticket's fresh lane on a stronger tier starts from, instead of
    /// the conversation before: where the work is, what was handed in, and
    /// what came back. In English, for the member.
    fn handoff_note(&mut self, squad: &str, t: &Ticket, tier: Option<ModelTier>, same: bool) -> String {
        let journal = self.journal(squad);
        let history = journal.and_then(|j| j.ticket_history(t.id).ok()).unwrap_or_default();
        let base = journal.and_then(|j| j.ticket_work(t.id).ok()).and_then(|w| w.base);
        let tier = tier.map(|t| format!(" ({})", t.code())).unwrap_or_default();
        let mut out = if same {
            format!("Handoff: T-{} starts again{tier} in a fresh conversation — the one before isn't carried over. Start from here, not from scratch.", t.id)
        } else {
            format!("Handoff: T-{} moves to a stronger model{tier}, in a fresh conversation — the one before isn't carried over. Start from here, not from scratch.", t.id)
        };
        if let Some(branch) = &t.branch {
            out.push_str(&format!("\n- The work so far is on branch {branch}, in your worktree"));
            match base {
                Some(base) => out.push_str(&format!(": read `git log {base}..HEAD` and `git diff {base}...HEAD` first.")),
                None => out.push_str(": read its log and diff first."),
            }
        }
        let handed: Vec<&(String, String, String)> = history.iter().filter(|(_, kind, _)| kind == "handed in").collect();
        if let Some((_, _, text)) = handed.last() {
            out.push_str(&format!("\n- Last handed in: {}", clip(text, HANDOFF_CLIP)));
        }
        let back: Vec<String> = history
            .iter()
            .filter(|(_, kind, _)| matches!(kind.as_str(), "sent back" | "design QA: changes" | "checks failed" | "reopened"))
            .map(|(by, kind, text)| format!("{kind} by {by}: {}", clip(text, HANDOFF_CLIP)))
            .collect();
        if !back.is_empty() {
            out.push_str("\n- What came back, oldest first:");
            for line in &back[back.len().saturating_sub(4)..] {
                out.push_str(&format!("\n  - {line}"));
            }
        }
        out.push_str("\nFix what came back, check it the way the ticket says, and keep what was right.");
        out
    }

    /// The latest handoff note on `ticket`'s record, for its fresh lane.
    pub(super) fn latest_handoff(&mut self, squad: &str, ticket: TicketId) -> Option<String> {
        let history = self.journal(squad)?.ticket_history(ticket).ok()?;
        history.into_iter().rev().find(|(_, kind, _)| kind == "handoff").map(|(_, _, text)| text)
    }

    /// The person sets `ticket`'s tier: kept for its next lanes — lower than
    /// its member's own included — and shown at once with the model it
    /// comes to on its member's engine.
    pub(super) fn set_ticket_tier(&mut self, squad: &str, ticket: TicketId, tier: Option<ModelTier>, pinned: bool) {
        let Some(t) = self.find_ticket(squad, ticket) else { return };
        let set = rules::set_by_person(t.model.as_ref(), tier, pinned);
        let set = self.shown_model(squad, &t, set);
        let Some(journal) = self.journal(squad) else { return };
        if let Err(err) = journal.set_ticket_model(ticket, Some(&set)) {
            return self.squad_error(squad, format!("{err:#}"));
        }
        let _ = journal.record_model_choice(ticket, "person", None, &set);
        let what = match (tier, pinned) {
            (Some(tier), true) => format!("Set to {} and pinned by you.", tier.code()),
            (Some(tier), false) => format!("Set to {} by you.", tier.code()),
            (None, true) => "Pinned by you.".to_owned(),
            (None, false) => "Given back to equipe's choice by you.".to_owned(),
        };
        let _ = journal.ticket_event(ticket, "you", "model", &what);
        crate::daemon_log!("equipe: {squad}: T-{ticket}'s tier: {what}");
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: Ticket { model: Some(set), ..t } });
    }

    /// `set`, a ticket's model as the person just set it, with the model and
    /// effort it comes to on the member that has the ticket, or would take it.
    fn shown_model(&mut self, squad: &str, t: &Ticket, set: TicketModel) -> TicketModel {
        let holder = t.assignee.clone().or_else(|| t.owner.clone());
        let member = holder.and_then(|name| {
            self.sessions.iter().find(|(_, s)| s.squad.as_deref() == Some(squad) && s.is_member_itself() && s.member.as_ref().is_some_and(|m| m.name == name || m.role == name)).map(|(&id, _)| id)
        });
        match member.and_then(|id| self.choose_model(squad, id, &Ticket { model: Some(set.clone()), ..t.clone() })) {
            Some(chosen) => TicketModel { model: chosen.model, effort: chosen.effort, engine: chosen.ticket.engine, tier: set.tier.or(chosen.ticket.tier), reason: chosen.ticket.reason, ..set },
            // Nobody holds it yet: what the person picked is what it will run on.
            None => TicketModel { model: set.pick.as_ref().map(|p| p.model.clone()).or(set.model.clone()), ..set },
        }
    }

    /// The person picks the model of `ticket` alone (or lets the pick go):
    /// every lane and run of this ticket from its next one uses it — the
    /// premium family included — and nothing else does. It starts a fresh
    /// conversation, a model change mid-session would throw its cache away.
    pub(super) fn set_ticket_pick(&mut self, squad: &str, ticket: TicketId, model: Option<String>) {
        let Some(t) = self.find_ticket(squad, ticket) else { return };
        if t.status.is_closed() {
            let message = say!(self, "error.ticket_closed", id = ticket);
            return self.squad_error(squad, message);
        }
        let set = match rules::set_pick(t.model.as_ref(), model.as_deref(), "you", now_secs() as i64) {
            Ok(set) => set,
            Err(why) => return self.squad_error(squad, why),
        };
        let set = self.shown_model(squad, &t, set);
        let Some(journal) = self.journal(squad) else { return };
        if let Err(err) = journal.set_ticket_model(ticket, Some(&set)) {
            return self.squad_error(squad, format!("{err:#}"));
        }
        let _ = journal.record_model_choice(ticket, "person", None, &set);
        let what = match set.pick.as_ref() {
            Some(p) if rules::is_premium(Engine::Claude, &p.model) => format!("Model set to {} (premium: several times Opus's cost) for this ticket alone by you; its next lanes use it.", p.model),
            Some(p) => format!("Model set to {} for this ticket alone by you; its next lanes use it.", p.model),
            None => "Model pick for this ticket let go by you; its tier decides again.".to_owned(),
        };
        let _ = journal.ticket_event(ticket, "you", "model", &what);
        crate::daemon_log!("equipe: {squad}: T-{ticket}'s model: {what}");
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: Ticket { model: Some(set), ..t } });
    }

    /// The squad's model choice, kept and told.
    pub(super) fn set_model_choice(&mut self, squad: &str, choice: ModelChoice) {
        match self.journal(squad).map(|j| j.set_model_choice(choice)) {
            Some(Ok(())) => {}
            Some(Err(err)) => return self.squad_error(squad, format!("{err:#}")),
            None => return,
        }
        crate::daemon_log!("equipe: {squad}'s model choice is now {}", choice.code());
        self.publish(Event::ModelChoiceChanged { squad: squad.to_owned(), choice });
    }

    /// A run's tool call, watched for a loop: the same call
    /// [`rules::LOOP_REPEATS`] times in a row raises its ticket's next lane.
    pub(super) fn watch_loop(&mut self, id: SessionId, tool: &str, target: &str) {
        let Some((squad, ticket)) = self.sessions.get(&id).and_then(|s| Some((s.squad.clone()?, s.work.as_ref()?.ticket))) else { return };
        let call = format!("{tool}\u{1f}{target}");
        let seen = self.tool_repeats.entry(id).or_insert_with(|| (call.clone(), 0));
        if seen.0 != call {
            *seen = (call, 0);
        }
        seen.1 += 1;
        if seen.1 == rules::LOOP_REPEATS && self.journal(&squad).is_some_and(|j| j.model_choice() != ModelChoice::Off) {
            self.escalate(&squad, ticket, &format!("caught in a loop ({tool} on the same target {} times in a row)", rules::LOOP_REPEATS));
        }
    }
}
