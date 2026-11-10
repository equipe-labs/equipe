//! A member's ticket runs ([`crate::RunOf`]) as a screen from before them
//! sees the squad.
//!
//! A screen that says `runs` in its hello lists each member once and its runs
//! under it. One that doesn't knows a member as one session with at most one
//! ticket ([`crate::SessionInfo::work`]), so the daemon projects what it sends
//! that screen through [`Legacy`]:
//!
//! - run sessions are never listed as members, and what happens in them is
//!   not streamed into the member's conversation;
//! - a member with exactly one run shows that ticket as its work, as before;
//! - a member with several shows no work, and a line in its conversation says
//!   the screen must be updated to follow them — never one run picked to stand
//!   for the others;
//! - a request for permission or a decision from a run names the member.
//!
//! What such a screen sends goes to the member's own session (its
//! conversation with the person), never to a run.

use std::collections::{BTreeMap, HashMap};

use crate::{Entry, Event, RunOf, SessionId, SessionState, Snapshot, TicketId, WorkInfo};

/// One screen's projection, fed the snapshot it attached with and then each
/// event in order.
pub struct Legacy {
    /// What a member with this many runs is told: the screen needs updating.
    update: Box<dyn Fn(usize) -> String + Send>,
    /// Every run session known, with its member and ticket.
    runs: HashMap<SessionId, RunOf>,
    /// Each run's ticket as its [`Event::WorkStarted`] said it, for when it
    /// becomes its member's only one again.
    started: HashMap<SessionId, Event>,
    /// What each member shows now: the ticket of its one run, or — several —
    /// that the screen must be updated.
    shown: HashMap<SessionId, Shown>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    Idle,
    One(TicketId),
    Several(usize),
}

impl Legacy {
    /// `update` says, for a member with that many runs, that this screen
    /// must be updated to follow them — in the screen's language.
    pub fn new(update: impl Fn(usize) -> String + Send + 'static) -> Self {
        Legacy { update: Box::new(update), runs: HashMap::new(), started: HashMap::new(), shown: HashMap::new() }
    }

    /// The snapshot as the screen knows it: run sessions folded into their
    /// members.
    pub fn snapshot(&mut self, mut snapshot: Snapshot) -> Snapshot {
        self.runs.clear();
        self.started.clear();
        self.shown.clear();
        let (runs, mut own): (Vec<_>, Vec<_>) = snapshot.sessions.into_iter().partition(|s| s.run.is_some());
        let mut by_member: BTreeMap<SessionId, Vec<&crate::SessionInfo>> = BTreeMap::new();
        for run in &runs {
            let of = run.run.expect("partitioned on it");
            self.runs.insert(run.session, of);
            if let Some(work) = &run.work {
                self.started.insert(
                    run.session,
                    Event::WorkStarted { session: run.session, ticket: work.ticket, title: String::new(), project: work.project.clone(), workspace: work.workspace.clone(), of: Some(of.of) },
                );
            }
            by_member.entry(of.of).or_default().push(run);
        }
        for member in &mut own {
            let mine = by_member.get(&member.session).map(Vec::as_slice).unwrap_or_default();
            match mine {
                [] => {
                    self.shown.insert(member.session, Shown::Idle);
                }
                [one] => {
                    let ticket = one.run.map(|r| r.ticket).unwrap_or_default();
                    member.work = Some(one.work.clone().unwrap_or(WorkInfo { ticket, project: one.project.clone(), workspace: one.workspace.clone() }));
                    if one.state == SessionState::Working {
                        member.state = SessionState::Working;
                    }
                    self.shown.insert(member.session, Shown::One(ticket));
                }
                several => {
                    member.work = None;
                    member.recent.push(Entry::Note { text: (self.update)(several.len()), undo: None });
                    if !member.recent_at.is_empty() {
                        member.recent_at.push(None);
                    }
                    self.shown.insert(member.session, Shown::Several(several.len()));
                }
            }
        }
        snapshot.sessions = own;
        for approval in &mut snapshot.approvals {
            if let Some(run) = self.runs.get(&approval.session) {
                approval.session = run.of;
            }
        }
        for set in &mut snapshot.questions {
            if let Some(run) = self.runs.get(&set.session) {
                set.session = run.of;
            }
        }
        snapshot
    }

    /// What the screen is sent for `event`: nothing for a run's own, the
    /// member's change of work for a run starting or ending, else the event.
    pub fn event(&mut self, event: Event) -> Vec<Event> {
        match event {
            Event::RunOpened { session, of, ticket, generation } => {
                self.runs.insert(session, RunOf { of, ticket, generation });
                self.settle(of)
            }
            Event::WorkStarted { session, .. } if self.runs.contains_key(&session) => {
                let of = self.runs[&session].of;
                self.started.insert(session, event);
                self.settle(of)
            }
            Event::SessionClosed { session } if self.runs.contains_key(&session) => {
                let of = self.runs.remove(&session).map(|r| r.of).expect("checked");
                self.started.remove(&session);
                self.settle(of)
            }
            Event::ApprovalRequested { squad, mut approval } => {
                if let Some(run) = self.runs.get(&approval.session) {
                    approval.session = run.of;
                }
                vec![Event::ApprovalRequested { squad, approval }]
            }
            Event::QuestionsAsked { squad, mut questions } => {
                if let Some(run) = self.runs.get(&questions.session) {
                    questions.session = run.of;
                }
                vec![Event::QuestionsAsked { squad, questions }]
            }
            other => match other.session() {
                Some(id) if self.runs.contains_key(&id) => Vec::new(),
                _ => vec![other],
            },
        }
    }

    /// Member `of`'s runs changed: what its screen shows follows, said only
    /// where it changes.
    fn settle(&mut self, of: SessionId) -> Vec<Event> {
        let mine: Vec<SessionId> = {
            let mut mine: Vec<SessionId> = self.runs.iter().filter(|(_, r)| r.of == of).map(|(&s, _)| s).collect();
            mine.sort_unstable();
            mine
        };
        let before = self.shown.get(&of).copied().unwrap_or(Shown::Idle);
        let now = match mine.as_slice() {
            [] => Shown::Idle,
            [one] => Shown::One(self.runs[one].ticket),
            several => Shown::Several(several.len()),
        };
        let mut out = Vec::new();
        // The one it showed is no longer the one it works.
        if let Shown::One(ticket) = before
            && now != before
        {
            out.push(Event::WorkEnded { session: of, ticket, of: None });
        }
        match now {
            Shown::One(_) => {
                // Shown once its run says where it works.
                let Some(Event::WorkStarted { ticket, title, project, workspace, .. }) = mine.first().and_then(|s| self.started.get(s)).cloned() else {
                    self.shown.insert(of, if matches!(before, Shown::One(_)) { Shown::Idle } else { before });
                    return out;
                };
                if before != now {
                    out.push(Event::WorkStarted { session: of, ticket, title, project, workspace, of: None });
                }
            }
            Shown::Several(n) if !matches!(before, Shown::Several(_)) => {
                out.push(Event::Notice { undo: None, session: of, text: (self.update)(n) });
            }
            _ => {}
        }
        self.shown.insert(of, now);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Access, Engine, Member, SessionInfo};
    use std::path::PathBuf;

    fn session(id: SessionId, name: &str, run: Option<RunOf>, ticket: Option<TicketId>) -> SessionInfo {
        SessionInfo {
            session: id,
            project: PathBuf::from("/home"),
            member: Some(Member { name: name.into(), role: "frontend".into() }),
            engine: Engine::Claude,
            model: None,
            asked_model: None,
            access: Access::EditFiles,
            workspace: None,
            state: if run.is_some() { SessionState::Working } else { SessionState::Ready },
            title: None,
            tokens: 0,
            context: None,
            recent: Vec::new(),
            recent_at: Vec::new(),
            queued: Vec::new(),
            queued_project: None,
            queued_in: Vec::new(),
            steering: Vec::new(),
            steering_in: Vec::new(),
            lane: None,
            run,
            estimate: None,
            work: ticket.map(|ticket| WorkInfo { ticket, project: PathBuf::from("/site"), workspace: None }),
            mode: None,
            effort: None,
            subagents: Vec::new(),
            compacting: None,
            visuals: Vec::new(),
        }
    }

    fn run(of: SessionId, ticket: TicketId) -> Option<RunOf> {
        Some(RunOf { of, ticket, generation: 1 })
    }

    fn legacy() -> Legacy {
        Legacy::new(|n| format!("update to follow {n} tickets"))
    }

    fn started(session: SessionId, ticket: TicketId) -> Event {
        Event::WorkStarted { session, ticket, title: format!("T{ticket}"), project: "/site".into(), workspace: None, of: None }
    }

    #[test]
    fn no_run_one_run_and_several_each_show_as_an_older_screen_reads_them() {
        let snapshot = Snapshot {
            sessions: vec![
                session(1, "pm", None, None),
                session(2, "frontend", None, None),
                session(20, "frontend", run(2, 7), Some(7)),
                session(3, "backend", None, None),
                session(30, "backend", run(3, 8), Some(8)),
                session(31, "backend", run(3, 9), Some(9)),
            ],
            ..Default::default()
        };
        let seen = legacy().snapshot(snapshot);
        let ids: Vec<SessionId> = seen.sessions.iter().map(|s| s.session).collect();
        assert_eq!(ids, [1, 2, 3], "runs are never listed as members");
        assert_eq!(seen.sessions[0].work, None);
        assert_eq!(seen.sessions[1].work.as_ref().map(|w| w.ticket), Some(7), "one run: its ticket is the member's work");
        assert_eq!(seen.sessions[1].state, SessionState::Working);
        assert_eq!(seen.sessions[2].work, None, "several: never one picked to stand for the others");
        assert!(matches!(seen.sessions[2].recent.last(), Some(Entry::Note { text, .. }) if text == "update to follow 2 tickets"));
    }

    #[test]
    fn a_runs_coming_and_going_is_its_members_work_while_it_is_the_only_one() {
        let mut view = legacy();
        view.snapshot(Snapshot { sessions: vec![session(2, "frontend", None, None)], ..Default::default() });
        assert!(view.event(Event::RunOpened { session: 20, of: 2, ticket: 7, generation: 1 }).is_empty(), "nothing until it says where it works");
        assert!(view.event(Event::SessionStarted { session: 20, project: "/site".into(), engine: Engine::Claude, model: None, workspace: None, member: None, squad: None }).is_empty());
        assert_eq!(view.event(started(20, 7)), vec![started(2, 7)]);
        assert!(view.event(Event::TextDelta { session: 20, text: "hi".into() }).is_empty(), "a run's words stay out of the member's conversation");
        // A second run: the screen can't follow both.
        view.event(Event::RunOpened { session: 21, of: 2, ticket: 8, generation: 1 });
        let more = view.event(started(21, 8));
        assert_eq!(more, Vec::<Event>::new(), "already said when the second opened");
        let mut view2 = legacy();
        view2.snapshot(Snapshot { sessions: vec![session(2, "frontend", None, None)], ..Default::default() });
        view2.event(Event::RunOpened { session: 20, of: 2, ticket: 7, generation: 1 });
        view2.event(started(20, 7));
        let second = view2.event(Event::RunOpened { session: 21, of: 2, ticket: 8, generation: 1 });
        assert_eq!(second, vec![Event::WorkEnded { session: 2, ticket: 7, of: None }, Event::Notice { undo: None, session: 2, text: "update to follow 2 tickets".into() }]);
        // One ends: the other is the member's work again, as it said.
        assert_eq!(view2.event(started(21, 8)), Vec::<Event>::new());
        assert_eq!(view2.event(Event::SessionClosed { session: 20 }), vec![started(2, 8)]);
        assert_eq!(view2.event(Event::SessionClosed { session: 21 }), vec![Event::WorkEnded { session: 2, ticket: 8, of: None }]);
        // The member's own events pass as they are.
        assert_eq!(view2.event(Event::TextDelta { session: 2, text: "x".into() }).len(), 1);
    }

    #[test]
    fn a_runs_request_names_its_member() {
        let mut view = legacy();
        view.snapshot(Snapshot { sessions: vec![session(2, "frontend", None, None), session(20, "frontend", run(2, 7), Some(7))], ..Default::default() });
        let approval = crate::Approval { id: 1, session: 20, member: "frontend".into(), ticket: Some(7), tool: "Bash".into(), action: "ls".into(), reason: None, preview: None };
        let out = view.event(Event::ApprovalRequested { squad: "acme".into(), approval });
        assert!(matches!(&out[..], [Event::ApprovalRequested { approval, .. }] if approval.session == 2));
    }
}
