//! What the window knows about the squad it follows: the members and how each
//! one stands, kept up to date from the daemon's snapshot and events alone.
//! No UI types here, so the rules can be tested without a window.

use std::fmt;
use std::path::Path;

use equipe_core::protocol::rpc::Hello;
use equipe_core::protocol::{Entry, Event, SessionId, SessionState, Ticket, TicketId, TicketStatus};

use crate::daemon::Update;

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
        reason: String,
        /// When we first noticed.
        since: Clock,
        /// Whether we ever held a snapshot: then the last state stays readable
        /// under a banner; otherwise the whole body says the daemon is off.
        had_snapshot: bool,
    },
}

#[derive(Debug, Clone)]
pub struct Model {
    pub squad: String,
    pub members: Vec<Member>,
    pub conn: Conn,
    tickets: Vec<(TicketId, TicketStatus, Option<String>)>,
    /// Open permission requests: (id, session, ticket).
    approvals: Vec<(u64, SessionId, Option<TicketId>)>,
}

impl Model {
    pub fn new(squad: &str) -> Self {
        Self { squad: squad.to_owned(), members: Vec::new(), conn: Conn::Connecting, tickets: Vec::new(), approvals: Vec::new() }
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
                let next = match &self.conn {
                    // Already down: only the reason may have moved on, and it
                    // is not shown while the banner is up.
                    Conn::Down { since, had_snapshot, .. } => Conn::Down { reason, since: *since, had_snapshot: *had_snapshot },
                    Conn::Connected { .. } => Conn::Down { reason, since: now, had_snapshot: true },
                    Conn::Connecting => Conn::Down { reason, since: now, had_snapshot: false },
                };
                let changed = !matches!((&self.conn, &next), (Conn::Down { since: a, had_snapshot: x, .. }, Conn::Down { since: b, had_snapshot: y, .. }) if a == b && x == y);
                self.conn = next;
                changed
            }
            Update::Event(event) => {
                let before = (self.members.clone(), self.ticket_hint());
                self.on_event(event);
                let mut changed = before != (self.members.clone(), self.ticket_hint());
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
        let snapshot = hello.snapshot;
        self.squad = snapshot.squad;
        self.tickets = snapshot.tickets.iter().map(|t| (t.id, t.status, t.owner.clone())).collect();
        self.approvals = snapshot.approvals.iter().map(|a| (a.id, a.session, a.ticket)).collect();
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
                }
            })
            .collect();
    }

    fn on_event(&mut self, event: Event) {
        match event {
            Event::TicketChanged { ticket, .. } => self.on_ticket(&ticket),
            Event::ApprovalRequested { approval, .. } => self.approvals.push((approval.id, approval.session, approval.ticket)),
            Event::ApprovalResolved { approval, .. } => self.approvals.retain(|(id, ..)| *id != approval),
            Event::SessionStarted { session, member, model, squad, .. } => {
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
        let entry = (ticket.id, ticket.status, ticket.owner.clone());
        match self.tickets.iter_mut().find(|(id, ..)| *id == ticket.id) {
            Some(slot) => *slot = entry,
            None => self.tickets.push(entry),
        }
    }

    /// What the sidebar derives from tickets and approvals, for change detection.
    fn ticket_hint(&self) -> (usize, Vec<Status>) {
        (self.for_you(), self.members.iter().map(|m| self.status(m)).collect())
    }

    /// Tickets waiting for the person: handed in for review, or put down for them.
    pub fn for_you(&self) -> usize {
        self.tickets.iter().filter(|(_, s, _)| matches!(s, TicketStatus::Review | TicketStatus::Waiting)).count()
    }

    pub fn status(&self, member: &Member) -> Status {
        let waiting = self
            .tickets
            .iter()
            .find(|(_, s, owner)| *s == TicketStatus::Waiting && owner.as_deref() == Some(member.name.as_str()))
            .map(|(id, ..)| *id);
        let asking = self.approvals.iter().find(|(_, session, _)| *session == member.session);
        if member.presence != Presence::Closed && (asking.is_some() || waiting.is_some()) {
            return Status::NeedsYou(asking.and_then(|(.., t)| *t).or(member.ticket).or(waiting));
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

/// The squads under `home`, by name — straight from the folder names, so the
/// list costs no database opens.
pub fn squad_names(home: &Path) -> Vec<String> {
    let Ok(dirs) = std::fs::read_dir(home.join("squads")) else { return Vec::new() };
    let mut names: Vec<String> =
        dirs.flatten().filter(|d| d.path().join("squad.db").exists()).map(|d| d.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
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
        assert!(model.apply(Update::Down("the daemon closed the connection".into()), T0));
        // A later reason is not news, and the time stays the first one.
        assert!(!model.apply(Update::Down("no daemon at /x".into()), T1));
        assert_eq!(model.conn, Conn::Down { reason: "no daemon at /x".into(), since: T0, had_snapshot: true });
        assert_eq!(model.members.len(), 1);
        // Back: the snapshot replaces the list.
        model.apply(hello(vec![]), T1);
        assert!(model.members.is_empty());
        assert!(matches!(model.conn, Conn::Connected { .. }));
    }

    #[test]
    fn a_daemon_that_was_never_there_has_no_snapshot() {
        let mut model = Model::new("acme");
        model.apply(Update::Down("no daemon at /x".into()), T0);
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
    fn context_shows_as_a_percentage() {
        assert_eq!(percent((63, 100)), Some(63));
        assert_eq!(percent((5, 0)), None);
    }
}
