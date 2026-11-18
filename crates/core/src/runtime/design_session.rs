//! Design sessions and the person's taste signals, as the daemon runs them
//! (`crate::design::session`, `crate::taste::tuner`): a session started —
//! its kickoff to the designer as the person's message, every screen told —
//! the PM's proposal of one put on a card at most once per ticket or screen,
//! what the person chose there carried out, and what they do that tells their
//! taste (tuner picks, reactions, chosen and passed-over options, hearted
//! tiles) kept with the project's style profile.

use equipe_protocol::decide::{Answer, QuestionSet};
use equipe_protocol::design::TasteSignal;
use equipe_protocol::design_session::{SessionFrom, SessionProposal, page_name};
use equipe_protocol::{TicketDraft, TicketEdit, TicketKind, TicketStatus};

use super::*;
use crate::design::session::{self, Choose, Proposed};

impl Runtime {
    /// A design session on `page`: the kickoff to the designer, as the
    /// person's message about `project`, and every screen told so it shows
    /// the page live beside the conversation. Refused (said on the squad's
    /// screens) with no designer or no page.
    pub(super) async fn start_design_session(&mut self, squad: String, project: Option<String>, page: String, from: SessionFrom) {
        if !crate::squads::valid_name(&squad) {
            return;
        }
        let Some(page) = page_name(&page) else {
            let message = say!(self, "design_session.no_page");
            return self.squad_error(&squad, message);
        };
        let Some(designer) = self.designer_of(&squad) else {
            let message = say!(self, "design_session.no_designer");
            return self.squad_error(&squad, message);
        };
        let profile = crate::taste::store::profile_of(&self.config.home, &squad, project.as_deref());
        let words = say!(self, "design_session.kickoff", page = page.clone());
        let text = session::kickoff(&page, project.as_deref(), &from, profile.as_ref(), &words);
        crate::daemon_log!("equipe: {squad}: a design session on {page} with the designer");
        self.publish(Event::DesignSessionStarted { squad: squad.clone(), designer, project: project.clone(), page: page.clone(), from });
        Box::pin(self.handle(Command::Submit { session: designer, text, about: Some(project.clone()) })).await;
        // On from here: its later turns open with the round's rules (the kickoff says them itself).
        self.design_sessions.insert(designer, session::Active::new(&squad, project.as_deref(), &page, now_secs()));
    }

    /// The prototype the person picked for `ticket` with "Use this", when it was built from one.
    pub(super) fn approved_prototype(&mut self, squad: &str, ticket: TicketId) -> Option<String> {
        let entries = self.journal(squad)?.ticket_entries(ticket).ok()?;
        entries.into_iter().rev().find(|e| e.kind == session::APPROVED_PROTOTYPE).map(|e| e.text)
    }

    /// The person ended the session on its screen.
    pub(super) fn end_design_session(&mut self, squad: &str, designer: SessionId) {
        if self.design_sessions.get(&designer).is_some_and(|a| a.squad == squad) {
            self.design_sessions.remove(&designer);
            crate::daemon_log!("equipe: {squad}: the design session with the designer ended");
        }
    }

    /// The note `id`'s next turn opens with while a design session is on with it.
    pub(super) fn design_note(&mut self, id: SessionId) -> Option<String> {
        let now = now_secs();
        if self.design_sessions.get(&id).is_some_and(|a| !a.on(now)) {
            self.design_sessions.remove(&id);
        }
        self.design_sessions.get(&id).filter(|a| a.rounds(now)).map(session::Active::note)
    }

    /// The person's message `text` to `id` was just taken in: in a design
    /// session on with it, it comes before the designer's own next step — a
    /// command running is stopped so the step comes at once, and an engine
    /// that can't read mid-turn has its turn stopped so the message goes in
    /// now ([`session::preempt`]). Elsewhere, nothing changes.
    pub(super) async fn person_first(&mut self, id: SessionId, text: &str) {
        let now = now_secs();
        let Some(active) = self.design_sessions.get_mut(&id) else { return };
        if !active.on(now) {
            self.design_sessions.remove(&id);
            return;
        }
        active.last = now;
        if active.picked {
            return;
        }
        let squad = active.squad.clone();
        let Some(s) = self.sessions.get(&id).filter(|s| s.is_working()) else { return };
        let steered = s.steering.iter().any(|t| t == text);
        if !steered && !s.queued.iter().any(|t| t == text) {
            return;
        }
        match session::preempt(steered, self.command_running(id)) {
            session::Preempt::Nothing => {}
            session::Preempt::StopCommand => {
                crate::daemon_log!("equipe: {squad}: the person spoke in the design session: the designer's command is stopped so it reads them now");
                self.stop_command(id);
            }
            session::Preempt::Interrupt => {
                crate::daemon_log!("equipe: {squad}: the person spoke in the design session: the designer's turn is stopped so it reads them now");
                Box::pin(self.interrupt(id)).await;
            }
        }
    }

    /// "Use this" on a session's prototype: the implementation ticket opened
    /// with the picked variant as its spec — or the one this session opened
    /// before updated, while it hasn't started — the designer told the
    /// rounds are over, and the session ended.
    pub(super) async fn use_design_variant(&mut self, squad: &str, designer: SessionId, mut picked: session::Picked) {
        if !crate::squads::valid_name(squad) || !self.sessions.get(&designer).is_some_and(|s| s.squad.as_deref() == Some(squad)) {
            return;
        }
        let Some(page) = page_name(&picked.page) else {
            let message = say!(self, "design_session.no_page");
            return self.squad_error(squad, message);
        };
        picked.page = page.clone();
        picked.variant = picked.variant.as_deref().and_then(session::variant_label);
        picked.url = picked.url.filter(|u| session::is_page_address(u));
        let title = match &picked.variant {
            Some(variant) => say!(self, "design_session.use_title_variant", page = page.clone(), variant = variant.clone()),
            None => say!(self, "design_session.use_title", page = page.clone()),
        };
        let body = session::implementation_body(&say!(self, "design_session.use_intro"), &picked);
        let before = self.design_sessions.get(&designer).and_then(|a| a.implementation).and_then(|t| self.find_ticket(squad, t)).filter(|t| t.status == TicketStatus::Todo).map(|t| t.id);
        let (ticket, updated) = match before {
            Some(id) => {
                let edit = TicketEdit { title: Some(title), body: Some(body), ..TicketEdit::default() };
                Box::pin(self.person_edits(squad, id, edit)).await;
                (id, true)
            }
            None => {
                let frontend = self.sessions.values().any(|s| s.squad.as_deref() == Some(squad) && s.is_member_itself() && s.member.as_ref().is_some_and(|m| m.role == "frontend"));
                let draft = TicketDraft {
                    title,
                    body,
                    project: picked.project.clone().map(PathBuf::from),
                    assignee: frontend.then(|| "frontend".to_owned()),
                    kind: Some(TicketKind::Code),
                    ..TicketDraft::default()
                };
                match self.new_ticket(squad, draft) {
                    Ok(id) => (id, false),
                    Err(message) => return self.squad_error(squad, message),
                }
            }
        };
        crate::daemon_log!("equipe: {squad}: the person picked {} of the design session on {page}: T-{ticket}", picked.variant.as_deref().map_or("the prototype".to_owned(), |v| format!("variant {v}")));
        // Its design QA: the precheck alone, against what they picked (`design_qa::gate_prototype`).
        if let Some(journal) = self.journal(squad) {
            let _ = journal.ticket_event(ticket, "equipe", session::APPROVED_PROTOTYPE, picked.url.as_deref().unwrap_or(&page));
        }
        if let Some(session) = self.sessions.get_mut(&designer) {
            session.briefing.push(session::picked_line(&picked, ticket, updated));
        }
        self.save_waiting(designer);
        // The rounds are over: the designer's next turns are ticket work again;
        // a later pick in the same session updates this ticket while it hasn't started.
        let active = self.design_sessions.entry(designer).or_insert_with(|| session::Active::new(squad, picked.project.as_deref(), &page, now_secs()));
        active.implementation = Some(ticket);
        active.picked = true;
        self.publish(Event::DesignVariantUsed { squad: squad.to_owned(), designer, ticket, variant: picked.variant.clone(), updated });
        if !updated {
            self.dispatch(squad).await;
        }
    }

    /// What the person did that tells their taste, kept with its profile.
    pub(super) fn taste_signals(&mut self, squad: String, project: Option<String>, signals: Vec<TasteSignal>) {
        if !crate::squads::valid_name(&squad) {
            return;
        }
        let today = squad_memory::date(now_secs());
        match crate::taste::store::signals(&self.config.home, &squad, project.as_deref(), &signals, &today) {
            Ok(()) => self.look_at_design(&squad, true),
            Err(crate::taste::store::Refused::NoProject) => {
                let message = say!(self, "taste.no_project", name = project.unwrap_or_default());
                self.squad_error(&squad, message);
            }
            Err(crate::taste::store::Refused::Failed(why)) => {
                let message = say!(self, "taste.not_saved", why = why);
                self.squad_error(&squad, message);
            }
            Err(crate::taste::store::Refused::Invalid(_)) => {}
        }
    }

    /// Signals equipe saw itself (a designer's options chosen and passed
    /// over, tiles hearted): kept quietly, a failure logged.
    pub(super) fn learn_signals(&mut self, squad: &str, project: Option<&str>, records: &[crate::taste::tuner::Record]) {
        if records.is_empty() {
            return;
        }
        let today = squad_memory::date(now_secs());
        if let Err(why) = crate::taste::store::learn(&self.config.home, squad, project, records, &[], &today) {
            crate::daemon_log!("equipe: {squad}: taste signals weren't kept: {why:?}");
        }
    }

    /// Where the PM's proposals are kept: the squad's own data folder.
    fn proposals_dir(&self, squad: &str) -> PathBuf {
        crate::journal::squad_dir(&self.config.home, squad)
    }

    /// `propose_design_session`: the PM proposes a session on a card — once
    /// per ticket or screen, never in the person's quiet hours — or, when the
    /// person asked for one, starts it.
    pub(super) async fn propose_design_session(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        if !self.sessions.get(&id).is_some_and(Session::is_pm) {
            return Err("Only the PM proposes design sessions; tell the PM.".into());
        }
        let screen = board::arg_str(args, "screen").and_then(page_name).ok_or("Which page or screen? Give its `screen`.")?;
        let ticket = board::arg_id(args, "ticket");
        let project = board::arg_str(args, "project").map(str::to_owned).or_else(|| ticket.and_then(|t| self.find_ticket(squad, t)).and_then(|t| t.project).map(|p| board::project_name(&p)));
        if self.designer_of(squad).is_none() {
            return Err("The squad has no designer: propose bringing one in first (`propose_team_change`).".into());
        }
        if args.get("asked").and_then(Value::as_bool) == Some(true) {
            Box::pin(self.start_design_session(squad.to_owned(), project, screen.clone(), SessionFrom::Pm { ticket })).await;
            return Ok(format!("Started: the designer has the session on {screen} and the person sees it live. Tell them in one line."));
        }
        let key = session::key(ticket, project.as_deref(), &screen);
        let dir = self.proposals_dir(squad);
        let mut made = session::read(&dir);
        session::may_propose(&made, &key)?;
        if crate::escalation::is_quiet(&self.notify_settings(squad), now_secs()) {
            return Err("It's the person's quiet hours: don't propose a session now. Propose it when they are back, if it still fits.".into());
        }
        let proposal = SessionProposal { ticket, screen, project: project.clone(), why: board::arg_str(args, "why").unwrap_or_default().to_owned() };
        let tune = crate::taste::store::profile_of(&self.config.home, squad, project.as_deref()).is_none_or(|p| p.is_thin());
        let question = session::question(&proposal, tune, self.config.lang);
        let set = QuestionSet { id: equipe_protocol::new_session_id(), session: id, member: member.to_owned(), ticket, questions: vec![question], asked_at: now_secs(), review: false, retro: None };
        made.push(Proposed { key, proposal, set: set.id, at: set.asked_at, tune, outcome: None });
        session::write(&dir, &made)?;
        equipe_protocol::decide::add(self.question_sets(squad), set.clone());
        self.save_questions(squad);
        self.publish(Event::QuestionsAsked { squad: squad.to_owned(), questions: set });
        Ok("Proposed: the person sees the card and answers when they can; the answer comes as their next message. \
Don't hold the ticket for it — if they say \"Not now\" or let the designer go ahead, carry on as planned."
            .into())
    }

    /// The person answered (or put away) a set: when it was a design session
    /// proposal, what they chose is kept and carried out — a session started,
    /// or the taste tuner opened on their screens.
    pub(super) async fn design_proposal_settled(&mut self, squad: &str, set: &QuestionSet, answers: Option<&[Answer]>) {
        let dir = self.proposals_dir(squad);
        let mut made = session::read(&dir);
        let Some(at) = made.iter().position(|p| p.set == set.id) else { return };
        let choice = session::chosen(made[at].tune, answers.and_then(|a| a.first()));
        made[at].outcome = Some(choice.outcome());
        let proposal = made[at].proposal.clone();
        if let Err(why) = session::write(&dir, &made) {
            crate::daemon_log!("equipe: {squad}: a design session proposal's answer wasn't kept: {why}");
        }
        match choice {
            Choose::Start => Box::pin(self.start_design_session(squad.to_owned(), proposal.project, proposal.screen, SessionFrom::Pm { ticket: proposal.ticket })).await,
            Choose::Tune => self.publish(Event::OpenTasteTuner { squad: squad.to_owned(), project: proposal.project }),
            Choose::NotNow | Choose::GoAhead => {}
        }
    }
}
