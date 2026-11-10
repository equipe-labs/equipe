//! The phone tools (`crate::phone`), as the daemon runs them: the runtime
//! says whether a member may — a QA or mobile member of a squad with a
//! mobile app, on a ticket — and where its files go; the device work runs off
//! the loop. Installing AXe asks the person on a card first; a report goes on
//! the ticket's record and into the member's conversation.

use super::*;
use crate::phone;

pub(super) enum PhoneRequest {
    /// May this member use the phone tools now: its ticket and places, or why not.
    Allowed(ToolCall, oneshot::Sender<Result<phone::Call, ToolResult>>),
    /// Ask the person to let equipe install AXe; answered once they have.
    Install(ToolCall, oneshot::Sender<ToolResult>),
    /// A member's report: on the ticket, and in the conversation.
    Report(ToolCall, Box<phone::report::Report>),
}

impl Handle {
    /// A member's phone tool call: allowed by the runtime, run off its loop —
    /// booting a simulator takes a minute and holds no one else up.
    pub async fn phone(&self, call: ToolCall) -> ToolResult {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::Phone(PhoneRequest::Allowed(call.clone(), tx)));
        let ctx = match rx.await {
            Ok(Ok(ctx)) => ctx,
            Ok(Err(refused)) => return refused,
            Err(_) => return ToolResult::error("equipe is shutting down"),
        };
        let run = |ctx: phone::Call, call: ToolCall| async move {
            tokio::task::spawn_blocking(move || phone::call(&phone::PHONES, &phone::exec::Real, &ctx, &call.name, &call.arguments))
                .await
                .unwrap_or_else(|_| phone::Outcome::Done(ToolResult::error("The phone tool stopped unexpectedly.")))
        };
        let mut outcome = run(ctx.clone(), call.clone()).await;
        if matches!(outcome, phone::Outcome::NeedsAxe) {
            let (tx, rx) = oneshot::channel();
            let _ = self.requests.send(Request::Phone(PhoneRequest::Install(call.clone(), tx)));
            let answer = rx.await.unwrap_or_else(|_| ToolResult::error("equipe is shutting down"));
            if answer.is_error {
                return answer;
            }
            let home = ctx.equipe_home.clone();
            let installed = tokio::task::spawn_blocking(move || phone::axe::install(&phone::exec::Real, &home)).await;
            match installed {
                Ok(Ok(_)) => {}
                Ok(Err(why)) => return ToolResult::error(format!("The person said yes, but AXe couldn't be installed: {why}")),
                Err(_) => return ToolResult::error("AXe's install stopped unexpectedly."),
            }
            outcome = run(ctx, call.clone()).await;
        }
        match outcome {
            phone::Outcome::Done(result) => result,
            phone::Outcome::NeedsAxe => ToolResult::error(phone::NO_AXE),
            phone::Outcome::Report(report, result) => {
                let _ = self.requests.send(Request::Phone(PhoneRequest::Report(call, report)));
                result
            }
        }
    }
}

impl Runtime {
    pub(super) fn on_phone(&mut self, request: PhoneRequest) {
        match request {
            PhoneRequest::Allowed(call, reply) => {
                let _ = reply.send(self.phone_allowed(&call));
            }
            PhoneRequest::Install(call, reply) => self.ask_phone_install(call, reply),
            PhoneRequest::Report(call, report) => self.phone_report(&call, &report),
        }
    }

    /// The platforms of `squad`'s apps, as its projects' files say.
    pub(super) fn phone_platforms(&self, squad: &str) -> phone::Platforms {
        phone::detect::platforms_cached(&self.projects_cached(squad))
    }

    /// Whether a member of `role` in `squad`, in `mode`, gets the phone tools.
    pub(super) fn phone_offered(&self, squad: &str, role: &str, mode: Mode) -> bool {
        phone::for_role(role) && mode != Mode::ReadOnly && self.phone_platforms(squad).any()
    }

    fn phone_allowed(&self, call: &ToolCall) -> Result<phone::Call, ToolResult> {
        let refuse = |why: &str| Err(ToolResult::error(why));
        let Some(id) = self.caller_id(call) else { return refuse("You're not on the squad.") };
        let Some(session) = self.sessions.get(&id) else { return refuse("You're not on the squad.") };
        let Some(member) = session.member.as_ref() else { return refuse("You're not on the squad.") };
        let mode = session.mode.or_else(|| self.journals.get(&call.squad).map(Journal::mode)).unwrap_or_default();
        if !self.phone_offered(&call.squad, &member.role, mode) {
            return refuse("The phone tools are for QA and mobile members of a squad with a mobile app, in a mode that may change things.");
        }
        let Some(work) = session.work.as_ref() else {
            return refuse("The phone is a ticket's: devices and recordings are made for one and deleted with it. Work on a ticket to use it.");
        };
        let home = squad_home::path(&self.config.home, &call.squad);
        Ok(phone::Call {
            squad: call.squad.clone(),
            member: call.member.clone(),
            ticket: work.ticket,
            platforms: self.phone_platforms(&call.squad),
            recordings: phone::recordings_dir(&home, work.ticket),
            equipe_home: self.config.home.clone(),
            cwd: work.cwd.clone(),
        })
    }

    /// Put installing AXe to the person, once: a card with what it is, how big,
    /// and where it goes. A second ask while one waits is told to wait.
    fn ask_phone_install(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        if phone::PHONES.axe_declined() {
            let _ = reply.send(ToolResult::error(phone::NO_AXE));
            return;
        }
        if self.pending.values().any(|p| p.approval.tool == phone::INSTALL) {
            let _ = reply.send(ToolResult::error("The person is being asked to let equipe install AXe. Wait for their answer, then try again."));
            return;
        }
        let Some(id) = self.caller_id(&call) else {
            let _ = reply.send(ToolResult::error("You're not on the squad."));
            return;
        };
        let ticket = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        let folder = phone::axe::dir(&self.config.home);
        let approval_id = self.next_approval;
        self.next_approval += 1;
        let approval = Approval {
            id: approval_id,
            session: id,
            member: call.member.clone(),
            ticket,
            tool: phone::INSTALL.to_owned(),
            action: say!(self, "phone.install_action", version = phone::axe::VERSION, size = phone::axe::SIZE_MB, folder = folder.display()),
            reason: Some(say!(self, "phone.install_why", name = call.member)),
            preview: None,
        };
        self.pending.insert(approval_id, Pending { squad: call.squad.clone(), approval: approval.clone(), input: Value::Null, reply });
        self.publish(Event::ApprovalRequested { squad: call.squad, approval });
        let internal = self.internal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(APPROVAL_WAIT).await;
            let _ = internal.send(Internal::ApprovalTimedOut(approval_id));
        });
    }

    /// The person's answer to installing AXe: a yes lets the call go on; a no
    /// holds until the daemon restarts, so nobody asks again meanwhile.
    pub(super) fn phone_install_answered(decision: Decision, why: &str) -> ToolResult {
        match decision {
            Decision::Allow | Decision::Always => ToolResult::ok(String::new()),
            Decision::Deny => {
                // A request nobody answered isn't a no: asked again next time.
                if why != "Nobody answered in time." {
                    phone::PHONES.decline_axe();
                }
                ToolResult::error(format!("{} ({why})", phone::NO_AXE))
            }
        }
    }

    /// A ticket done with: its members' devices put away (a recording still
    /// running saved), its emulators deleted, and the squad's recordings of
    /// tickets done with — but one an open ticket names — deleted. Off the loop.
    /// (Its simulators go with `disk::clear_simulators`.)
    pub(super) fn clear_finished_phones(&mut self, squad: &str, id: TicketId) {
        let tickets: Vec<(TicketId, bool, String)> = self.tickets(squad).into_iter().map(|t| (t.id, !t.status.is_closed(), format!("{}\n{}", t.title, t.body))).collect();
        let shared = self.journals.keys().filter(|s| s.as_str() != squad).cloned().collect::<Vec<_>>().iter().any(|other| {
            self.journal(other).and_then(|j| j.tickets().ok()).is_some_and(|ts| ts.iter().any(|t| t.id == id && !t.status.is_closed()))
        });
        let (owner, home) = (squad.to_owned(), squad_home::path(&self.config.home, squad));
        tokio::task::spawn_blocking(move || {
            let mut lines: Vec<String> = phone::PHONES.release_ticket(&phone::exec::Real, &owner, id).into_iter().map(|n| format!("put away {n}")).collect();
            lines.extend(crate::disk::clear_avds(id, &owner, shared));
            lines.extend(crate::disk::sweep_recordings(&home, &tickets));
            for line in lines {
                crate::daemon_log!("equipe: {line} (its ticket is done with)");
            }
        });
    }

    /// A report: on the ticket's record, and told in the member's conversation
    /// with the recording and its key frames.
    fn phone_report(&mut self, call: &ToolCall, report: &phone::report::Report) {
        let args: Vec<(&str, String)> = phone::report::head_args(report);
        let head = crate::i18n::core(self.config.lang).fill("notice.qa_report", &args);
        let text = phone::report::note(&head, report);
        if let Some(journal) = self.journal(&call.squad) {
            let _ = journal.ticket_event(report.ticket, &call.member, "qa report", &text);
        }
        if let Some(id) = self.caller_id(call) {
            self.publish(Event::Notice { session: id, text, undo: None });
        }
    }
}
