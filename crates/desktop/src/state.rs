//! What the window knows about the squad it follows: the members and how each
//! one stands, kept up to date from the daemon's snapshot and events alone.
//! No UI types here, so the rules can be tested without a window.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::time::Instant;

use equipe_core::protocol::rpc::Hello;
use equipe_core::protocol::{Engine, Entry, Event, Mode, SessionId, SessionState, Ticket, TicketId, TicketStatus};

use crate::convo::Convo;
use crate::daemon::Update;
use crate::failure::Failure;
use crate::notify::Alert;
use crate::views::approvals::Queue;

/// A wall-clock time of day, as shown ("14:02"). Passed in, never read here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Clock {
    pub fn now() -> Self {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        let secs = unsafe { libc::time(std::ptr::null_mut()) };
        unsafe { libc::localtime_r(&secs, &mut tm) };
        Self { hour: tm.tm_hour as u8, minute: tm.tm_min as u8, second: tm.tm_sec as u8 }
    }

    /// "14:02", for what is stamped per event: it changes once a minute.
    pub fn hm(self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }
}

impl fmt::Display for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }
}

/// What the member is busy with, from the latest thing it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Thinking,
    Reading,
    Editing,
    Running,
    Searching,
    Writing,
    Working,
}

impl Verb {
    fn of_tool(tool: &str) -> Self {
        match tool {
            "Read" => Self::Reading,
            "Edit" | "MultiEdit" | "NotebookEdit" | "Write" => Self::Editing,
            "Bash" => Self::Running,
            "Grep" | "Glob" | "WebSearch" => Self::Searching,
            _ => Self::Working,
        }
    }

    fn of_entry(entry: Option<&Entry>) -> Self {
        match entry {
            Some(Entry::Tool { tool, .. }) => Self::of_tool(tool),
            Some(Entry::Reply { .. }) => Self::Writing,
            _ => Self::Thinking,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Ready,
    Working,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub session: SessionId,
    pub name: String,
    pub role: String,
    pub presence: Presence,
    pub verb: Verb,
    pub ticket: Option<TicketId>,
    /// Memos waiting for the turn in progress to end.
    pub memos: usize,
    /// How full its context is, in percent, once known.
    pub context: Option<u8>,
    pub model: Option<String>,
    pub engine: Option<Engine>,
}

/// How a member shows in the list. The most pressing thing wins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Waits for the person: a permission to give, or a ticket put down for them.
    NeedsYou(Option<TicketId>),
    Working { ticket: Option<TicketId>, verb: Verb },
    Ready { ticket: Option<TicketId>, model: Option<String> },
    Closed,
}

/// The line to the daemon, as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conn {
    /// Not heard from yet.
    Connecting,
    Connected {
        /// When the last event came, to the minute.
        last_event: Option<Clock>,
    },
    Down {
        reason: Failure,
        /// When we first noticed.
        since: Clock,
        /// Whether we ever held a snapshot: then the last state stays readable
        /// under a banner; otherwise the whole body says the daemon is off.
        had_snapshot: bool,
    },
}

type CardHint = (Vec<u64>, Vec<(TicketId, TicketStatus, Option<String>)>, Option<String>);

#[derive(Debug, Clone)]
pub struct Model {
    pub squad: String,
    pub members: Vec<Member>,
    pub conn: Conn,
    pub tickets: Vec<Ticket>,
    /// The squad's projects, in the order they joined.
    pub projects: Vec<PathBuf>,
    /// The squad's mode, for completion's sake.
    pub mode: Mode,
    /// Open permission requests, oldest first.
    pub approvals: Queue,
    /// Things that need the person and have not been announced yet: the shell
    /// takes them and tells the person if the window is out of sight.
    alerts: Vec<Alert>,
    /// One quiet line the shell shows under the conversation, until taken.
    flash: Option<String>,
    /// Each member's conversation.
    pub convos: HashMap<SessionId, Convo>,
    /// Whose conversation is on screen.
    pub selected: Option<SessionId>,
    /// The project in focus, by name; `None`: the whole squad.
    pub focus: Option<String>,
    /// Bumped whenever the selected conversation shows something new, so a
    /// screen can tell it must draw again.
    pub revision: u64,
}

impl Model {
    pub fn new(squad: &str) -> Self {
        Self {
            squad: squad.to_owned(),
            members: Vec::new(),
            conn: Conn::Connecting,
            tickets: Vec::new(),
            projects: Vec::new(),
            mode: Mode::default(),
            approvals: Queue::default(),
            alerts: Vec::new(),
            flash: None,
            convos: HashMap::new(),
            selected: None,
            focus: None,
            revision: 0,
        }
    }

    /// The member on screen.
    pub fn selected_member(&self) -> Option<&Member> {
        self.selected.and_then(|id| self.members.iter().find(|m| m.session == id))
    }

    pub fn convo(&self) -> Option<&Convo> {
        self.convos.get(&self.selected?)
    }

    pub fn convo_mut(&mut self) -> Option<&mut Convo> {
        self.convos.get_mut(&self.selected?)
    }

    pub fn is_pm(member: &Member) -> bool {
        member.role == "pm"
    }

    /// A member by the name the person types after `@`, if it is still on the squad.
    pub fn open_member(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.name == name && m.presence != Presence::Closed)
    }

    /// Show a member's conversation. True when that changed what is on screen.
    pub fn select(&mut self, session: SessionId) -> bool {
        if self.selected == Some(session) || !self.members.iter().any(|m| m.session == session) {
            return false;
        }
        self.selected = Some(session);
        self.revision += 1;
        true
    }

    /// The names of the squad's projects, as chips show them.
    pub fn project_names(&self) -> Vec<String> {
        self.projects.iter().map(|p| equipe_core::board::project_name(p)).collect()
    }

    /// Put a project (by name; `None`: the whole squad) in focus. Every
    /// conversation follows: the PM's thread for it, each member's part of its
    /// work there. True when it changed.
    pub fn set_focus(&mut self, project: Option<String>) -> bool {
        if self.focus == project {
            return false;
        }
        for convo in self.convos.values_mut() {
            convo.thread_view = project.clone();
        }
        self.focus = project;
        self.revision += 1;
        true
    }

    fn keep_selection(&mut self) {
        let valid = self.selected.is_some_and(|id| self.members.iter().any(|m| m.session == id));
        if !valid {
            self.selected = self.members.iter().find(|m| Self::is_pm(m)).or(self.members.first()).map(|m| m.session);
        }
    }

    /// Apply one update from the link. True when something on screen changed.
    pub fn apply(&mut self, update: Update, now: Clock) -> bool {
        match update {
            Update::Attached(hello) => {
                self.load(hello);
                self.conn = Conn::Connected { last_event: None };
                true
            }
            Update::Down(reason) => {
                // Answers sent into a dead link never came back as resolved.
                self.approvals.forget_answers();
                let next = match &self.conn {
                    // Already down: only the reason may have moved on, and it
                    // is not shown while the banner is up.
                    Conn::Down { since, had_snapshot, .. } => Conn::Down { reason, since: *since, had_snapshot: *had_snapshot },
                    Conn::Connected { .. } => Conn::Down { reason, since: now, had_snapshot: true },
                    Conn::Connecting => Conn::Down { reason, since: now, had_snapshot: false },
                };
                // With a snapshot on screen only the banner shows, and it doesn't
                // say why; without one, the whole body is the reason's guide.
                let changed = !matches!((&self.conn, &next), (Conn::Down { since: a, had_snapshot: x, reason: r }, Conn::Down { since: b, had_snapshot: y, reason: q }) if a == b && x == y && (*x || r == q));
                self.conn = next;
                changed
            }
            // The shell reads these two itself: the squad list and the bridge's greeting.
            Update::Squads(_) | Update::Bridge(_) => false,
            Update::Event(event) => {
                let before = (self.members.clone(), self.ticket_hint(), self.subagents_hint(), self.revision, self.card_hint());
                self.on_event(event, now);
                let mut changed = before != (self.members.clone(), self.ticket_hint(), self.subagents_hint(), self.revision, self.card_hint());
                // The footer's "last event" moves when the minute does.
                if let Conn::Connected { last_event } = &mut self.conn {
                    let stamp = Some(now);
                    if last_event.map(Clock::hm) != stamp.map(Clock::hm) {
                        changed = true;
                    }
                    *last_event = stamp;
                }
                changed
            }
        }
    }

    fn load(&mut self, hello: Hello) {
        let version = hello.version;
        let snapshot = hello.snapshot;
        self.squad = snapshot.squad;
        self.tickets = snapshot.tickets.clone();
        self.projects = snapshot.projects.clone();
        self.mode = snapshot.mode;
        self.approvals.load(snapshot.approvals.clone());
        if let Some(note) = crate::notify::stale_daemon(&version, env!("CARGO_PKG_VERSION")) {
            self.flash = Some(note);
        }
        self.members = snapshot
            .sessions
            .iter()
            .map(|s| {
                let (name, role) = s.member.as_ref().map_or(("session".to_owned(), String::new()), |m| (m.name.clone(), m.role.clone()));
                Member {
                    session: s.session,
                    name,
                    role,
                    presence: match s.state {
                        SessionState::Working => Presence::Working,
                        SessionState::Ready => Presence::Ready,
                    },
                    verb: Verb::of_entry(s.recent.iter().rev().find(|e| matches!(e, Entry::Tool { .. } | Entry::Reply { .. } | Entry::User { .. }))),
                    ticket: s.work.as_ref().map(|w| w.ticket),
                    memos: s.queued.len(),
                    context: s.context.and_then(percent),
                    model: s.model.clone(),
                    engine: Some(s.engine),
                }
            })
            .collect();
        let now = Instant::now();
        let focus = self.focus.clone();
        self.convos = snapshot
            .sessions
            .iter()
            .map(|s| {
                let mut convo = Convo::from_info(s, now);
                convo.thread_view = focus.clone();
                (s.session, convo)
            })
            .collect();
        self.keep_selection();
        self.revision += 1;
    }

    fn on_event(&mut self, event: Event, now: Clock) {
        // The conversation first: what the selected one shows decides redraws.
        if let Some(id) = event.session() {
            if let Event::SessionStarted { squad, member, project, .. } = &event {
                let ours = squad.as_ref().is_none_or(|s| *s == self.squad);
                if ours && !self.convos.contains_key(&id) {
                    let mut convo = Convo::new(project.clone(), member.as_ref().is_some_and(|m| m.role == "pm"));
                    convo.thread_view = self.focus.clone();
                    self.convos.insert(id, convo);
                }
            }
            if let Some(convo) = self.convos.get_mut(&id)
                && convo.on_event(&event, Instant::now(), now)
                && self.selected == Some(id)
            {
                self.revision += 1;
            }
        }
        self.on_member_event(event);
        self.keep_selection();
    }

    fn on_member_event(&mut self, event: Event) {
        match event {
            Event::TicketChanged { ticket, .. } => self.on_ticket(&ticket),
            Event::ApprovalRequested { squad, approval } => {
                if squad == self.squad && !self.approvals.has(approval.id) {
                    self.alerts.push(Alert::Approval { approval: approval.clone(), squad });
                    self.approvals.push(approval);
                }
            }
            Event::ApprovalResolved { approval, .. } => self.approvals.resolve(approval),
            Event::Error { session: None, message, .. } => self.flash = Some(message),
            Event::SessionStarted { session, member, model, squad, engine, .. } => {
                if squad.is_some_and(|s| s != self.squad) || self.members.iter().any(|m| m.session == session) {
                    return;
                }
                let (name, role) = member.map_or(("session".to_owned(), String::new()), |m| (m.name, m.role));
                self.members.push(Member {
                    session,
                    name,
                    role,
                    presence: Presence::Ready,
                    verb: Verb::Thinking,
                    ticket: None,
                    memos: 0,
                    context: None,
                    model,
                    engine: Some(engine),
                });
            }
            other => {
                let Some(id) = other.session() else { return };
                let Some(member) = self.members.iter_mut().find(|m| m.session == id) else { return };
                match other {
                    Event::UserMessage { .. } => {
                        member.presence = Presence::Working;
                        member.verb = Verb::Thinking;
                        member.memos = 0;
                    }
                    Event::MessageQueued { .. } => member.memos += 1,
                    Event::WorkStarted { ticket, .. } => {
                        member.ticket = Some(ticket);
                        member.presence = Presence::Working;
                        member.verb = Verb::Thinking;
                    }
                    Event::WorkEnded { .. } => member.ticket = None,
                    Event::ToolUse { tool, .. } => {
                        member.presence = Presence::Working;
                        member.verb = Verb::of_tool(&tool);
                    }
                    Event::TextDelta { .. } => {
                        member.presence = Presence::Working;
                        member.verb = Verb::Writing;
                    }
                    Event::ModelKnown { model, .. } => member.model = Some(model),
                    Event::TurnEnded { usage, .. } => {
                        if member.presence == Presence::Working {
                            member.presence = Presence::Ready;
                        }
                        if let Some(u) = usage
                            && let (Some(used), Some(window)) = (u.context_tokens, u.context_window)
                        {
                            member.context = percent((used, window));
                        }
                    }
                    Event::SessionClosed { .. } => member.presence = Presence::Closed,
                    _ => {}
                }
            }
        }
    }

    fn on_ticket(&mut self, ticket: &Ticket) {
        let was = self.tickets.iter().find(|t| t.id == ticket.id).map(|t| t.status);
        if was.is_some() && was != Some(ticket.status) {
            match ticket.status {
                TicketStatus::Review => {
                    self.flash = Some(format!("T-{0} 검토 대기 · /accept T-{0} 또는 /reopen T-{0} <바꿀 것>", ticket.id));
                    self.alerts.push(Alert::Review { ticket: ticket.id, title: ticket.title.clone(), by: ticket.assignee.clone().or(ticket.owner.clone()), project: self.ticket_project_name(ticket) });
                }
                TicketStatus::Waiting => {
                    let needs = ticket.note.as_deref().map(equipe_core::board::waiting_needs).unwrap_or_default().join(", ");
                    self.flash = Some(format!("T-{0}: 당신을 기다립니다 — {needs} · 끝나면 /resume T-{0}", ticket.id));
                    self.alerts.push(Alert::Waiting { ticket: ticket.id, by: ticket.assignee.clone().or(ticket.owner.clone()) });
                }
                _ => {}
            }
        }
        match self.tickets.iter_mut().find(|t| t.id == ticket.id) {
            Some(slot) => *slot = ticket.clone(),
            None => self.tickets.push(ticket.clone()),
        }
    }

    /// The subagents under each member, which the sidebar lists.
    fn subagents_hint(&self) -> Vec<(SessionId, Vec<crate::convo::Sub>)> {
        self.members.iter().filter_map(|m| self.convos.get(&m.session).filter(|c| !c.subagents.is_empty()).map(|c| (m.session, c.subagents.clone()))).collect()
    }

    /// What the cards above the composer show, for change detection.
    fn card_hint(&self) -> CardHint {
        (
            self.approvals.ids(),
            self.for_you_here().iter().map(|t| (t.id, t.status, t.note.clone())).collect(),
            self.flash.clone(),
        )
    }

    /// What the sidebar derives from tickets and approvals, for change detection.
    fn ticket_hint(&self) -> (usize, Vec<Status>) {
        (self.for_you(), self.members.iter().map(|m| self.status(m)).collect())
    }

    pub fn take_alerts(&mut self) -> Vec<Alert> {
        std::mem::take(&mut self.alerts)
    }

    pub fn take_flash(&mut self) -> Option<String> {
        self.flash.take()
    }

    /// The project a ticket is about: its own; for code, else the squad's first.
    /// A task or action with none is the squad's own, not a project's.
    pub fn ticket_project(&self, ticket: &Ticket) -> Option<PathBuf> {
        if ticket.kind.in_home() {
            return ticket.project.clone();
        }
        ticket.project.clone().or_else(|| self.projects.first().cloned())
    }

    pub fn ticket_project_name(&self, ticket: &Ticket) -> Option<String> {
        self.ticket_project(ticket).map(|p| equipe_core::board::project_name(&p))
    }

    /// What waits for the person under the project in focus: reviews first,
    /// then tickets put down for them, each in ticket order.
    pub fn for_you_here(&self) -> Vec<&Ticket> {
        let here = |t: &&Ticket| self.focus.is_none() || self.ticket_project_name(t) == self.focus;
        let mut out: Vec<&Ticket> = Vec::new();
        for status in [TicketStatus::Review, TicketStatus::Waiting] {
            let mut group: Vec<&Ticket> = self.tickets.iter().filter(|t| t.status == status).filter(here).collect();
            group.sort_by_key(|t| t.id);
            out.extend(group);
        }
        out
    }

    /// Tickets waiting for the person: handed in for review, or put down for them.
    pub fn for_you(&self) -> usize {
        self.tickets.iter().filter(|t| matches!(t.status, TicketStatus::Review | TicketStatus::Waiting)).count()
    }

    pub fn status(&self, member: &Member) -> Status {
        let waiting = self
            .tickets
            .iter()
            .find(|t| t.status == TicketStatus::Waiting && t.owner.as_deref() == Some(member.name.as_str()))
            .map(|t| t.id);
        let asking = self.approvals.first_for(member.session);
        if member.presence != Presence::Closed && (asking.is_some() || waiting.is_some()) {
            return Status::NeedsYou(asking.and_then(|a| a.ticket).or(member.ticket).or(waiting));
        }
        match member.presence {
            Presence::Closed => Status::Closed,
            Presence::Working => Status::Working { ticket: member.ticket, verb: member.verb },
            Presence::Ready => Status::Ready { ticket: member.ticket, model: member.model.clone() },
        }
    }

    /// Members in list order: the PM first, then whoever needs the person, who
    /// is working, who is resting, and who has left — each in the order they joined.
    pub fn ordered(&self) -> Vec<&Member> {
        let rank = |m: &Member| match self.status(m) {
            _ if m.role == "pm" => 0,
            Status::NeedsYou(_) => 1,
            Status::Working { .. } => 2,
            Status::Ready { .. } => 3,
            Status::Closed => 4,
        };
        let mut list: Vec<&Member> = self.members.iter().collect();
        list.sort_by_key(|m| rank(m));
        list
    }
}

fn percent((used, window): (u64, u64)) -> Option<u8> {
    (window > 0).then(|| (used * 100 / window).min(100) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::rpc::Hello;
    use equipe_core::protocol::{Access, Approval, Engine, Member as Who, SessionInfo, Snapshot, WorkInfo};

    const T0: Clock = Clock { hour: 14, minute: 2, second: 10 };
    const T1: Clock = Clock { hour: 14, minute: 3, second: 12 };

    fn session(id: SessionId, role: &str, name: &str, state: SessionState) -> SessionInfo {
        SessionInfo {
            session: id,
            project: "/p".into(),
            member: Some(Who { role: role.into(), name: name.into() }),
            engine: Engine::Claude,
            model: Some("opus".into()),
            access: Access::EditFiles,
            workspace: None,
            state,
            title: None,
            tokens: 0,
            context: None,
            recent: vec![],
            queued: vec![],
            queued_project: None,
            queued_in: vec![],
            steering: vec![],
            lane: None,
            estimate: None,
            work: None,
            mode: None,
        }
    }

    fn hello(sessions: Vec<SessionInfo>) -> Update {
        Update::Attached(Hello { version: "x".into(), snapshot: Snapshot { squad: "acme".into(), sessions, ..Default::default() } })
    }

    fn approval(id: u64, session: SessionId) -> Approval {
        serde_json::from_value(serde_json::json!({"id": id, "session": session, "member": "m", "ticket": 9, "tool": "Bash", "action": "ls", "reason": null})).unwrap()
    }

    fn names(model: &Model) -> Vec<&str> {
        model.ordered().iter().map(|m| m.name.as_str()).collect()
    }

    #[test]
    fn the_snapshot_fills_the_list_and_connects() {
        let mut model = Model::new("acme");
        let mut busy = session(2, "frontend", "frontend", SessionState::Working);
        busy.work = Some(WorkInfo { ticket: 61, project: "/p".into(), workspace: None });
        busy.recent = vec![Entry::Tool { tool: "Edit".into(), target: None }];
        assert!(model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), busy]), T0));
        assert_eq!(model.conn, Conn::Connected { last_event: None });
        assert_eq!(model.status(&model.members[1]), Status::Working { ticket: Some(61), verb: Verb::Editing });
        assert_eq!(model.status(&model.members[0]), Status::Ready { ticket: None, model: Some("opus".into()) });
    }

    #[test]
    fn events_move_a_member_between_states() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready)]), T0);

        assert!(model.apply(Update::Event(Event::WorkStarted { session: 1, ticket: 5, title: "t".into(), project: "/p".into(), workspace: None }), T0));
        assert_eq!(model.status(&model.members[0]), Status::Working { ticket: Some(5), verb: Verb::Thinking });
        model.apply(Update::Event(Event::ToolUse { session: 1, tool: "Read".into(), target: None }), T0);
        assert_eq!(model.status(&model.members[0]), Status::Working { ticket: Some(5), verb: Verb::Reading });
        model.apply(Update::Event(Event::TurnEnded { session: 1, outcome: equipe_core::protocol::TurnOutcome::Completed, usage: None }), T0);
        assert!(matches!(model.status(&model.members[0]), Status::Ready { ticket: Some(5), .. }));
        model.apply(Update::Event(Event::SessionClosed { session: 1 }), T0);
        assert_eq!(model.status(&model.members[0]), Status::Closed);
    }

    #[test]
    fn a_permission_request_marks_the_member_until_it_is_answered() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), session(2, "backend", "backend", SessionState::Working)]), T0);
        model.apply(Update::Event(Event::ApprovalRequested { squad: "acme".into(), approval: approval(7, 2) }), T0);
        assert_eq!(model.status(&model.members[1]), Status::NeedsYou(Some(9)));
        model.apply(Update::Event(Event::ApprovalResolved { squad: "acme".into(), approval: 7, decision: equipe_core::protocol::Decision::Allow }), T0);
        assert!(matches!(model.status(&model.members[1]), Status::Working { .. }));
    }

    #[test]
    fn a_ticket_put_down_for_the_person_marks_its_owner() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(2, "backend", "backend", SessionState::Ready)]), T0);
        let ticket: Ticket = serde_json::from_value(serde_json::json!({"id": 3, "title": "t", "body": "", "status": "waiting", "owner": "backend", "assignee": null, "project": null, "depends_on": [], "branch": null, "created_by": "pm", "note": null})).unwrap();
        model.apply(Update::Event(Event::TicketChanged { squad: "acme".into(), ticket }), T0);
        assert_eq!(model.status(&model.members[0]), Status::NeedsYou(Some(3)));
        assert_eq!(model.for_you(), 1);
    }

    fn ticket(id: u32, status: &str, note: Option<&str>) -> Ticket {
        serde_json::from_value(serde_json::json!({"id": id, "title": "t", "body": "", "status": status, "owner": "backend", "assignee": null, "project": null, "depends_on": [], "branch": null, "created_by": "pm", "note": note})).unwrap()
    }

    #[test]
    fn a_new_request_or_a_ticket_turning_to_review_or_waiting_is_announced_once() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(2, "backend", "backend", SessionState::Working)]), T0);
        assert!(model.take_alerts().is_empty(), "what the snapshot holds is not news");
        model.apply(Update::Event(Event::ApprovalRequested { squad: "acme".into(), approval: approval(7, 2) }), T0);
        // The same request again changes nothing.
        model.apply(Update::Event(Event::ApprovalRequested { squad: "acme".into(), approval: approval(7, 2) }), T0);
        let tick = |status, note| Update::Event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(3, status, note) });
        model.apply(tick("in_progress", None), T0);
        assert!(model.apply(tick("review", None), T0));
        model.apply(tick("review", None), T0);
        assert!(model.apply(tick("waiting", Some("Waiting on you:\n- [ ] GitHub token")), T0));
        let alerts = model.take_alerts();
        assert_eq!(alerts.len(), 3);
        assert!(matches!(alerts[0], Alert::Approval { .. }));
        assert!(matches!(alerts[1], Alert::Review { ticket: 3, .. }));
        assert!(matches!(alerts[2], Alert::Waiting { ticket: 3, .. }));
        assert!(model.take_flash().unwrap().contains("GitHub token"));
        assert!(model.take_alerts().is_empty());
    }

    #[test]
    fn the_card_lists_reviews_before_waits_and_follows_the_project_in_focus() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![]), T0);
        for t in [ticket(5, "waiting", None), ticket(9, "review", None), ticket(2, "review", None), ticket(1, "in_progress", None)] {
            model.apply(Update::Event(Event::TicketChanged { squad: "acme".into(), ticket: t }), T0);
        }
        let ids: Vec<u32> = model.for_you_here().iter().map(|t| t.id).collect();
        assert_eq!(ids, [2, 9, 5]);
        model.set_focus(Some("elsewhere".into()));
        assert!(model.for_you_here().is_empty());
    }

    #[test]
    fn the_list_puts_the_pm_first_then_who_needs_you_then_who_works() {
        let mut model = Model::new("acme");
        model.apply(
            hello(vec![
                session(4, "qa", "qa", SessionState::Ready),
                session(3, "frontend", "frontend", SessionState::Working),
                session(2, "backend", "backend", SessionState::Ready),
                session(1, "pm", "pm", SessionState::Ready),
            ]),
            T0,
        );
        model.apply(Update::Event(Event::ApprovalRequested { squad: "acme".into(), approval: approval(1, 2) }), T0);
        assert_eq!(names(&model), ["pm", "backend", "frontend", "qa"]);
    }

    #[test]
    fn going_down_keeps_the_list_and_the_first_time_noticed() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready)]), T0);
        assert!(model.apply(Update::Down(Failure::Closed), T0));
        // A later reason is not news, and the time stays the first one.
        assert!(!model.apply(Update::Down(Failure::NoAnswer), T1));
        assert_eq!(model.conn, Conn::Down { reason: Failure::NoAnswer, since: T0, had_snapshot: true });
        assert_eq!(model.members.len(), 1);
        // Back: the snapshot replaces the list.
        model.apply(hello(vec![]), T1);
        assert!(model.members.is_empty());
        assert!(matches!(model.conn, Conn::Connected { .. }));
    }

    #[test]
    fn a_daemon_that_was_never_there_has_no_snapshot() {
        let mut model = Model::new("acme");
        model.apply(Update::Down(Failure::NoAnswer), T0);
        assert!(matches!(model.conn, Conn::Down { had_snapshot: false, .. }));
    }

    #[test]
    fn an_event_that_changes_nothing_in_the_same_minute_draws_nothing() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready)]), T0);
        // First event stamps the footer: a change.
        assert!(model.apply(Update::Event(Event::Output { session: 1, tokens: 5 }), T0));
        assert!(!model.apply(Update::Event(Event::Output { session: 1, tokens: 9 }), Clock { second: 50, ..T0 }));
        assert!(model.apply(Update::Event(Event::Output { session: 1, tokens: 9 }), T1));
    }

    #[test]
    fn the_pm_is_on_screen_first_and_selecting_a_member_changes_what_shows() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(2, "frontend", "frontend", SessionState::Ready), session(1, "pm", "pm", SessionState::Ready)]), T0);
        assert_eq!(model.selected, Some(1));
        let before = model.revision;
        assert!(model.select(2));
        assert!(model.revision > before, "a draw is due");
        assert!(!model.select(2), "already there");
        assert!(!model.select(99), "nobody by that session");
        assert_eq!(model.selected_member().map(|m| m.name.as_str()), Some("frontend"));
    }

    #[test]
    fn only_what_the_selected_conversation_shows_asks_for_a_draw() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), session(2, "frontend", "frontend", SessionState::Ready)]), T0);
        // Another member's reply is not on screen: no draw for the text itself.
        let busy = Event::TextDelta { session: 2, text: "x".into() };
        let first = model.apply(Update::Event(busy.clone()), T0);
        assert!(first, "its row says it is writing now");
        assert!(!model.apply(Update::Event(busy), Clock { second: 50, ..T0 }), "more of the same changes nothing the person sees");
        // The selected one's every piece does.
        let delta = |t: &str| Update::Event(Event::TextDelta { session: 1, text: t.into() });
        assert!(model.apply(delta("a"), T0));
        assert!(model.apply(delta("b"), T0));
        assert_eq!(model.convo().unwrap().blocks().len(), 1);
        // The token counter is not a change.
        assert!(!model.apply(Update::Event(Event::Output { session: 1, tokens: 5 }), Clock { second: 50, ..T0 }));
    }

    #[test]
    fn a_project_in_focus_gives_every_conversation_its_thread() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready)]), T0);
        assert!(model.set_focus(Some("site".into())));
        assert!(!model.set_focus(Some("site".into())));
        assert_eq!(model.convo().unwrap().thread_view.as_deref(), Some("site"));
        // A member who joins afterwards follows the focus too, and so does a reattach.
        model.apply(Update::Event(Event::SessionStarted { session: 5, project: "/p".into(), engine: Engine::Claude, model: None, workspace: None, member: Some(Who { role: "qa".into(), name: "qa".into() }), squad: Some("acme".into()) }), T0);
        assert_eq!(model.convos[&5].thread_view.as_deref(), Some("site"));
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready)]), T1);
        assert_eq!(model.convo().unwrap().thread_view.as_deref(), Some("site"));
        assert!(model.set_focus(None));
        assert_eq!(model.convo().unwrap().thread_view, None);
    }

    #[test]
    fn a_closed_selection_falls_back_to_the_pm() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), session(2, "qa", "qa", SessionState::Ready)]), T0);
        model.select(2);
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready)]), T1);
        assert_eq!(model.selected, Some(1));
    }

    #[test]
    fn context_shows_as_a_percentage() {
        assert_eq!(percent((63, 100)), Some(63));
        assert_eq!(percent((5, 0)), None);
    }
}
