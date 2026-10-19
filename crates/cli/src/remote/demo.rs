//! `equipe demo serve`'s squad: what a store reviewer's phone sees. No agent
//! and no engine — a script, played for each paired phone on its own: a board,
//! three members, a request for permission, the PM answering. Plain state; the
//! relay and the clock are `link::serve_demo`'s.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use equipe_core::protocol::rpc::Hello;
use equipe_core::protocol::{
    Access, Approval, Command, Decision, Engine, Entry, Event, Member, Mode, SessionId, SessionInfo, SessionState, Snapshot, Ticket,
    TicketId, TicketKind, TicketStatus, TurnOutcome, WorkInfo,
};

/// The one squad a demo phone follows.
pub const SQUAD: &str = "demo";
/// The name phones are told they paired with.
pub const NAME: &str = "equipe demo";
const PROJECTS: [&str; 2] = ["/demo/website", "/demo/server"];
const PM: SessionId = 1;
const FRONTEND: SessionId = 2;
const BACKEND: SessionId = 3;
/// Of a phone's message, this much is kept.
const MESSAGE: usize = 2000;
/// Lines of a conversation kept for a phone that attaches again.
const RECENT: usize = 40;
/// Tickets a phone may have the PM add.
const ADDED_TICKETS: u32 = 10;
/// Events a phone's copy may have waiting; past that, what it says goes unanswered.
const LATER: usize = 200;
/// After a request is answered, the next one comes this much later.
const NEXT_REQUEST_MS: u64 = 20_000;

/// Requests for permission, asked one after another — never more than one waits.
const REQUESTS: [(SessionId, Option<TicketId>, &str, &str, &str); 4] = [
    (FRONTEND, Some(3), "Bash", "npm run build", "Build the landing page to check it compiles before handing it in."),
    (BACKEND, Some(4), "WebFetch", "https://docs.stripe.com/api/payment_intents", "Read the payment provider's API reference."),
    (BACKEND, Some(2), "Bash", "git push origin equipe/backend-t2", "Push the sign-up branch so it can be reviewed."),
    (FRONTEND, Some(5), "Write", "src/pages/pricing.tsx", "Create the pricing page."),
];

/// One phone's copy of the squad.
struct Scene {
    sessions: Vec<SessionInfo>,
    tickets: Vec<Ticket>,
    approvals: Vec<Approval>,
    /// Events to happen, in order of when.
    later: VecDeque<(u64, Event)>,
    next_approval: u64,
    /// Which of [`REQUESTS`] comes next.
    request: usize,
    added: u32,
    replies: usize,
    /// When this phone last showed up — the least recent copy goes first.
    seen: u64,
}

/// Every demo phone's copy.
#[derive(Default)]
pub struct Demo {
    scenes: HashMap<String, Scene>,
}

impl Demo {
    /// A phone said `hello`: its squad as it now is, from the start the first time.
    pub fn hello(&mut self, device: &str, version: &str, now: u64) -> Hello {
        if !self.scenes.contains_key(device) && self.scenes.len() >= super::core::DEMO_DEVICES {
            let oldest = self.scenes.iter().min_by_key(|(_, s)| s.seen).map(|(d, _)| d.clone());
            oldest.map(|d| self.scenes.remove(&d));
        }
        let scene = self.scenes.entry(device.to_owned()).or_insert_with(Scene::new);
        scene.seen = now;
        Hello { version: version.to_owned(), snapshot: scene.snapshot() }
    }

    /// A command from a phone: what happens at once. More may follow, from [`Demo::due`].
    pub fn command(&mut self, device: &str, command: Command, now: u64) -> Vec<Event> {
        match self.scenes.get_mut(device) {
            Some(scene) => {
                scene.seen = now;
                let events = scene.command(command, now);
                events.into_iter().map(|e| scene.apply(e)).collect()
            }
            None => vec![],
        }
    }

    /// What has come due, for each phone.
    pub fn due(&mut self, now: u64) -> Vec<(String, Event)> {
        let mut out = Vec::new();
        for (device, scene) in &mut self.scenes {
            while scene.later.front().is_some_and(|(at, _)| *at <= now) {
                let (_, event) = scene.later.pop_front().expect("checked");
                out.push((device.clone(), scene.apply(event)));
            }
        }
        out
    }

    /// Start over: every phone gets the squad from the start.
    pub fn reset(&mut self) {
        self.scenes.clear();
    }
}

fn ticket(id: TicketId, title: &str, status: TicketStatus, who: Option<&str>, note: Option<&str>) -> Ticket {
    Ticket {
        id,
        title: title.into(),
        body: String::new(),
        status,
        kind: TicketKind::Code,
        owner: who.map(|w| w.split('-').next().unwrap_or(w).to_owned()),
        assignee: who.filter(|_| status != TicketStatus::Todo && status != TicketStatus::Proposed).map(str::to_owned),
        project: Some(PathBuf::from(PROJECTS[if who == Some("backend") { 1 } else { 0 }])),
        depends_on: vec![],
        branch: None,
        worktree: None,
        merged: (status == TicketStatus::Done).then_some(true),
        off_main: None,
        design_qa: None,
        created_by: "pm".into(),
        note: note.map(str::to_owned),
        asks: vec![],
        remaining: Vec::new(), images: Vec::new(),
        updated_at: None,
        priority: Default::default(), rank: 0,
    }
}

fn session(id: SessionId, role: &str, project: &str, recent: Vec<Entry>, work: Option<TicketId>) -> SessionInfo {
    SessionInfo {
        session: id,
        project: project.into(),
        member: Some(Member { role: role.into(), name: role.into() }),
        engine: Engine::Claude,
        model: Some("demo".into()),
        access: if role == "pm" { Access::ReadOnly } else { Access::EditFiles },
        workspace: None,
        state: SessionState::Ready,
        title: None,
        tokens: 0,
        context: None,
        recent,
        queued: vec![],
        queued_project: None,
        queued_in: vec![],
        steering: vec![],
        steering_in: vec![],
        lane: None,
        estimate: None,
        work: work.map(|ticket| WorkInfo { ticket, project: project.into(), workspace: None }),
        mode: None,
        effort: None,
        subagents: Vec::new(),
    }
}

/// Whether the person wrote in Korean: the members answer in it.
fn korean(text: &str) -> bool {
    text.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c) || ('\u{3131}'..='\u{318E}').contains(&c))
}

impl Scene {
    fn new() -> Self {
        let pm = session(
            PM,
            "pm",
            PROJECTS[0],
            vec![
                Entry::User { text: "We launch the website this month. What's left?".into() },
                Entry::Reply {
                    text: "Here's the plan: the sign-up API (T-2) is handed in for your review, the landing page (T-3) is in progress, \
                           and the pricing page (T-5) follows it. The payment provider (T-4) waits on an API key only you can create."
                        .into(),
                },
            ],
            None,
        );
        let frontend = session(FRONTEND, "frontend", PROJECTS[0], vec![Entry::Tool { tool: "Edit".into(), target: Some("src/pages/index.tsx".into()), outcome: None }], Some(3));
        let backend = session(BACKEND, "backend", PROJECTS[1], vec![], None);
        let tickets = vec![
            ticket(1, "Set up the project", TicketStatus::Done, Some("backend"), Some("Repository, CI and a first deploy.")),
            ticket(2, "Sign-up API: email and password", TicketStatus::Review, Some("backend"), Some("Handed in: POST /signup with validation; 14 tests pass.")),
            ticket(3, "Landing page hero", TicketStatus::InProgress, Some("frontend"), None),
            ticket(4, "Connect the payment provider", TicketStatus::Waiting, Some("backend"), Some("Needs a test API key from you: create one in the provider's dashboard, then resume.")),
            Ticket { depends_on: vec![3], ..ticket(5, "Pricing page", TicketStatus::Todo, Some("frontend"), None) },
            Ticket { kind: TicketKind::Task, project: None, ..ticket(6, "Write the launch announcement", TicketStatus::Proposed, Some("pm"), None) },
        ];
        let mut scene = Self {
            sessions: vec![pm, frontend, backend],
            tickets,
            approvals: vec![],
            later: VecDeque::new(),
            next_approval: 1,
            request: 0,
            added: 0,
            replies: 0,
            seen: 0,
        };
        let first = scene.next_request();
        scene.apply(first);
        scene
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            squad: SQUAD.into(),
            projects: PROJECTS.iter().map(PathBuf::from).collect(),
            sessions: self.sessions.clone(),
            tickets: self.tickets.clone(),
            mode: Mode::Ask,
            approvals: self.approvals.clone(),
            ..Snapshot::default()
        }
    }

    fn member(&self, session: SessionId) -> Option<&str> {
        self.sessions.iter().find(|s| s.session == session).and_then(|s| s.member.as_ref()).map(|m| m.name.as_str())
    }

    fn find(&self, id: TicketId) -> Option<&Ticket> {
        self.tickets.iter().find(|t| t.id == id)
    }

    /// The next request for permission, its member at work.
    fn next_request(&mut self) -> Event {
        let (session, ticket, tool, action, reason) = REQUESTS[self.request % REQUESTS.len()];
        self.request += 1;
        let id = self.next_approval;
        self.next_approval += 1;
        let approval = Approval {
            id,
            session,
            member: self.member(session).unwrap_or("frontend").to_owned(),
            ticket,
            tool: tool.into(),
            action: action.into(),
            reason: Some(reason.into()),
            preview: None,
        };
        Event::ApprovalRequested { squad: SQUAD.into(), approval }
    }

    /// Keep what an event changes, so a phone attaching again sees it.
    fn apply(&mut self, event: Event) -> Event {
        match &event {
            Event::TicketChanged { ticket, .. } => match self.tickets.iter_mut().find(|t| t.id == ticket.id) {
                Some(t) => *t = ticket.clone(),
                None => self.tickets.push(ticket.clone()),
            },
            Event::ApprovalRequested { approval, .. } => {
                self.approvals.push(approval.clone());
                self.state(approval.session, SessionState::Working);
            }
            Event::ApprovalResolved { approval, .. } => self.approvals.retain(|a| a.id != *approval),
            Event::UserMessage { session, text } => {
                self.entry(*session, Entry::User { text: text.clone() });
                self.state(*session, SessionState::Working);
            }
            Event::TextDelta { session, text } => {
                if let Some(s) = self.sessions.iter_mut().find(|s| s.session == *session) {
                    match s.recent.last_mut() {
                        Some(Entry::Reply { text: so_far }) => so_far.push_str(text),
                        _ => s.recent.push(Entry::Reply { text: text.clone() }),
                    }
                    s.tokens += text.len() as u64 / 4;
                }
            }
            Event::ToolUse { session, tool, target } => self.entry(*session, Entry::Tool { tool: tool.clone(), target: target.clone(), outcome: None }),
            Event::Thread { session, project } => self.entry(*session, Entry::Thread { project: project.clone() }),
            Event::TurnEnded { session, .. } => self.state(*session, SessionState::Ready),
            _ => {}
        }
        event
    }

    fn entry(&mut self, session: SessionId, entry: Entry) {
        if let Some(s) = self.sessions.iter_mut().find(|s| s.session == session) {
            s.recent.push(entry);
            let over = s.recent.len().saturating_sub(RECENT);
            s.recent.drain(..over);
        }
    }

    fn state(&mut self, session: SessionId, state: SessionState) {
        if let Some(s) = self.sessions.iter_mut().find(|s| s.session == session) {
            s.state = state;
        }
    }

    /// `event` at `at`, kept in order.
    fn at(&mut self, at: u64, event: Event) {
        let i = self.later.iter().position(|(t, _)| *t > at).unwrap_or(self.later.len());
        self.later.insert(i, (at, event));
    }

    /// A member's turn: its words, a fragment at a time, from `at`.
    fn say(&mut self, session: SessionId, at: u64, text: &str) -> u64 {
        let words: Vec<&str> = text.split_inclusive(' ').collect();
        let mut t = at;
        for chunk in words.chunks(6) {
            self.at(t, Event::TextDelta { session, text: chunk.concat() });
            t += 250;
        }
        self.at(t, Event::TurnEnded { session, outcome: TurnOutcome::Completed, usage: None });
        t
    }

    fn changed(&self, ticket: Ticket) -> Event {
        Event::TicketChanged { squad: SQUAD.into(), ticket }
    }

    /// When a member's turn now under way ends, or `now`.
    fn free_at(&self, session: SessionId, now: u64) -> u64 {
        self.later.iter().filter(|(_, e)| e.session() == Some(session)).map(|(t, _)| *t + 300).max().unwrap_or(now).max(now)
    }

    fn command(&mut self, command: Command, now: u64) -> Vec<Event> {
        match command {
            Command::Decide { approval, decision, .. } => {
                let Some(asked) = self.approvals.iter().find(|a| a.id == approval).cloned() else { return vec![] };
                let resolved = Event::ApprovalResolved { squad: SQUAD.into(), approval, decision };
                let s = asked.session;
                let start = self.free_at(s, now) + 600;
                let end = if decision == Decision::Deny {
                    self.say(s, start, "Understood — I won't do that. I'll leave it for you and carry on with the rest.")
                } else {
                    self.at(start, Event::ToolUse { session: s, tool: asked.tool.clone(), target: Some(asked.action.clone()) });
                    let end = self.say(s, start + 900, "Done. It worked; I'm carrying on with the ticket.");
                    // The landing page, built, is handed in.
                    if let Some(t) = asked.ticket.and_then(|id| self.find(id)).filter(|t| t.id == 3 && t.status == TicketStatus::InProgress) {
                        let review = Ticket { status: TicketStatus::Review, note: Some("Handed in: the hero with headline, subline and one button; the build passes.".into()), ..t.clone() };
                        let event = self.changed(review);
                        self.at(end + 500, event);
                    }
                    end
                };
                let next = self.next_request();
                self.at(end + NEXT_REQUEST_MS, next);
                vec![resolved]
            }
            Command::Submit { session, text, .. } => {
                let Some(member) = self.member(session).map(str::to_owned).filter(|_| self.later.len() < LATER) else { return vec![] };
                let text: String = text.chars().take(MESSAGE).collect();
                let ko = korean(&text);
                let start = self.free_at(session, now) + 800;
                let reply = if member == "pm" { self.pm_reply(&text, ko, start) } else { member_reply(&member, ko) };
                self.say(session, start, &reply);
                vec![Event::UserMessage { session, text }]
            }
            Command::Interrupt { session } => {
                if self.sessions.iter().all(|s| s.session != session || s.state != SessionState::Working) {
                    return vec![];
                }
                let before = self.later.len();
                self.later.retain(|(_, e)| !matches!(e, Event::TextDelta { session: s, .. } | Event::ToolUse { session: s, .. } | Event::TurnEnded { session: s, .. } if *s == session));
                if self.later.len() == before && self.approvals.iter().any(|a| a.session == session) {
                    // Waiting on a request: that stays for the person to answer.
                    return vec![];
                }
                vec![Event::TurnEnded { session, outcome: TurnOutcome::Interrupted, usage: None }]
            }
            Command::Accept { ticket, .. } => {
                let Some(t) = self.find(ticket).filter(|t| t.status == TicketStatus::Review).cloned() else { return vec![] };
                let mut out = vec![self.changed(Ticket { status: TicketStatus::Done, merged: Some(true), ..t })];
                // With the landing page done, the pricing page starts.
                if ticket == 3
                    && let Some(next) = self.find(5).filter(|n| n.status == TicketStatus::Todo).cloned()
                {
                    out.push(self.changed(Ticket { status: TicketStatus::InProgress, assignee: Some("frontend".into()), ..next }));
                }
                out
            }
            Command::Reopen { ticket, note, .. } => self.back_to_work(ticket, TicketStatus::Review, note, now),
            Command::Resume { ticket, note, .. } => self.back_to_work(ticket, TicketStatus::Waiting, note, now),
            Command::Cancel { ticket, .. } => match self.find(ticket).filter(|t| !t.status.is_closed()).cloned() {
                Some(t) => vec![self.changed(Ticket { status: TicketStatus::Cancelled, ..t })],
                None => vec![],
            },
            Command::Focus { project, .. } => {
                let project = project.filter(|p| PROJECTS.iter().any(|known| known.ends_with(p.as_str())));
                vec![Event::Thread { session: PM, project }]
            }
            _ => vec![],
        }
    }

    /// A ticket the person sends back (`from` Review) or resumes (`from`
    /// Waiting): worked on, and handed in again a moment later.
    fn back_to_work(&mut self, ticket: TicketId, from: TicketStatus, note: String, now: u64) -> Vec<Event> {
        let Some(t) = self.find(ticket).filter(|t| t.status == from).cloned() else { return vec![] };
        let note: String = note.chars().take(MESSAGE).collect();
        let working = Ticket { status: TicketStatus::InProgress, note: (!note.is_empty()).then_some(note), ..t.clone() };
        let handed = Ticket { status: TicketStatus::Review, note: Some("Handed in again, with what you asked for.".into()), ..t };
        let event = self.changed(handed);
        self.at(now + 8000, event);
        vec![self.changed(working)]
    }

    /// The PM's answer. Asked for work, it puts a ticket on the board.
    fn pm_reply(&mut self, text: &str, ko: bool, at: u64) -> String {
        let lower = text.to_lowercase();
        let wants = ["add", "make", "create", "build", "ticket", "need", "추가", "만들", "티켓", "해줘", "해 줘"].iter().any(|w| lower.contains(w));
        if wants && self.added < ADDED_TICKETS {
            self.added += 1;
            let id = self.tickets.iter().map(|t| t.id).max().unwrap_or(0) + 1;
            let title: String = text.lines().next().unwrap_or(text).chars().take(60).collect();
            let event = self.changed(ticket(id, &title, TicketStatus::Todo, Some("frontend"), None));
            self.at(at + 400, event);
            return if ko {
                format!("알겠습니다. 보드에 T-{id}로 올리고 frontend에게 맡겼어요. 랜딩 페이지(T-3)가 끝나는 대로 시작합니다.")
            } else {
                format!("Got it — I've put that on the board as T-{id} for frontend. It starts as soon as the landing page (T-3) is done.")
            };
        }
        self.replies += 1;
        let en = [
            "Here's where we are: T-2 (sign-up API) waits for your review, T-3 (landing page) is in progress, and T-4 needs a test API key from you. Want me to add anything?",
            "Nothing is blocked except T-4, which needs the payment provider's test key. Everything else is moving; I'll tell you when something is ready to review.",
            "Good question. I'd ship the landing page and sign-up first, then pricing. Tell me what to add and I'll put it on the board.",
        ];
        let ko_replies = [
            "현재 상황입니다: T-2(가입 API)는 검토를 기다리고, T-3(랜딩 페이지)는 진행 중이며, T-4는 결제사의 테스트 API 키가 필요해요. 더 추가할 일이 있을까요?",
            "T-4만 결제사 테스트 키를 기다리고 있고, 나머지는 진행 중입니다. 검토할 것이 생기면 알려 드릴게요.",
            "랜딩 페이지와 가입을 먼저 내고, 그다음 가격 페이지를 하는 게 좋겠습니다. 추가할 일을 말씀해 주시면 보드에 올릴게요.",
        ];
        let i = (self.replies - 1) % en.len();
        (if ko { ko_replies[i] } else { en[i] }).to_owned()
    }
}

fn member_reply(member: &str, ko: bool) -> String {
    if ko {
        format!("{member}: 알겠습니다. 지금 하던 티켓에 반영할게요.")
    } else {
        format!("{member}: Noted — I'll take that into the ticket I'm on.")
    }
}

/// The demo's own place: its relay registration, its code, its devices.
pub fn home(given: Option<PathBuf>) -> PathBuf {
    given.unwrap_or_else(|| equipe_providers::credentials::equipe_home().join("demo"))
}

fn secret_path(home: &Path) -> PathBuf {
    home.join("demo.json")
}

/// The standing code's secret: made once, kept where only its owner reads it.
pub fn secret(home: &Path, renew: bool) -> std::io::Result<[u8; 32]> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let path = secret_path(home);
    if !renew
        && let Ok(bytes) = std::fs::read(&path)
        && let Ok(kept) = serde_json::from_slice::<serde_json::Value>(&bytes)
        && let Some(secret) = kept["secret"].as_str().and_then(|s| super::unb64(s).ok()).and_then(|b| <[u8; 32]>::try_from(b).ok())
    {
        return Ok(secret);
    }
    let secret = super::random::<32>();
    std::fs::create_dir_all(home)?;
    let part = path.with_extension("json.part");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&part)?;
    file.write_all(serde_json::json!({ "secret": super::b64(&secret) }).to_string().as_bytes())?;
    std::fs::rename(part, path)?;
    Ok(secret)
}

/// The QR code as an SVG, for the review forms and the docs.
pub fn qr_svg(text: &str) -> Option<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let width = code.width();
    let size = width + 8;
    let mut path = String::new();
    for (i, color) in code.to_colors().iter().enumerate() {
        if *color == qrcode::Color::Dark {
            path.push_str(&format!("M{},{}h1v1h-1z", i % width + 4, i / width + 4));
        }
    }
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" width=\"{px}\" height=\"{px}\" shape-rendering=\"crispEdges\">\
         <rect width=\"{size}\" height=\"{size}\" fill=\"#fff\"/><path fill=\"#000\" d=\"{path}\"/></svg>\n",
        px = size * 8
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello(demo: &mut Demo, device: &str) -> Snapshot {
        demo.hello(device, "test", 0).snapshot
    }

    #[test]
    fn a_phone_sees_a_board_a_waiting_request_and_the_pm_first() {
        let mut demo = Demo::default();
        let s = hello(&mut demo, "a");
        assert_eq!(s.squad, SQUAD);
        assert_eq!(s.sessions[0].member.as_ref().unwrap().role, "pm");
        assert_eq!(s.approvals.len(), 1);
        assert!(s.tickets.iter().any(|t| t.status == TicketStatus::Review));
        assert!(s.tickets.iter().any(|t| t.status == TicketStatus::Waiting));
    }

    #[test]
    fn allowing_the_request_lets_the_member_go_on_and_another_comes_later() {
        let mut demo = Demo::default();
        let id = hello(&mut demo, "a").approvals[0].id;
        let out = demo.command("a", Command::Decide { squad: SQUAD.into(), approval: id, decision: Decision::Allow }, 1000);
        assert_eq!(out, vec![Event::ApprovalResolved { squad: SQUAD.into(), approval: id, decision: Decision::Allow }]);
        assert!(hello(&mut demo, "a").approvals.is_empty());
        // The same answer again does nothing.
        assert!(demo.command("a", Command::Decide { squad: SQUAD.into(), approval: id, decision: Decision::Allow }, 1000).is_empty());
        let later: Vec<Event> = demo.due(1000 + 60_000).into_iter().map(|(_, e)| e).collect();
        assert!(later.iter().any(|e| matches!(e, Event::ToolUse { tool, .. } if tool == "Bash")));
        assert!(later.iter().any(|e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 3 && ticket.status == TicketStatus::Review)));
        assert!(later.iter().any(|e| matches!(e, Event::ApprovalRequested { approval, .. } if approval.id != id)));
        let s = hello(&mut demo, "a");
        assert_eq!(s.approvals.len(), 1, "one request waits at a time");
        assert_eq!(s.sessions[1].state, SessionState::Ready);
    }

    #[test]
    fn the_pm_answers_and_puts_asked_work_on_the_board() {
        let mut demo = Demo::default();
        hello(&mut demo, "a");
        let out = demo.command("a", Command::Submit { session: PM, text: "What's the status?".into(), about: None }, 0);
        assert_eq!(out, vec![Event::UserMessage { session: PM, text: "What's the status?".into() }]);
        let later: Vec<Event> = demo.due(30_000).into_iter().map(|(_, e)| e).collect();
        let said: String = later.iter().filter_map(|e| match e { Event::TextDelta { session: PM, text } => Some(text.as_str()), _ => None }).collect();
        assert!(said.contains("T-2"), "{said}");
        assert!(later.iter().any(|e| matches!(e, Event::TurnEnded { session: PM, outcome: TurnOutcome::Completed, .. })));
        // Asked in Korean for work: a ticket, and the answer in Korean.
        demo.command("a", Command::Submit { session: PM, text: "FAQ 페이지 추가해줘".into(), about: None }, 40_000);
        let later: Vec<Event> = demo.due(80_000).into_iter().map(|(_, e)| e).collect();
        assert!(later.iter().any(|e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 7 && ticket.title == "FAQ 페이지 추가해줘")));
        let said: String = later.iter().filter_map(|e| match e { Event::TextDelta { text, .. } => Some(text.as_str()), _ => None }).collect();
        assert!(said.contains("T-7") && korean(&said), "{said}");
        let s = hello(&mut demo, "a");
        assert!(matches!(s.sessions[0].recent.last(), Some(Entry::Reply { text }) if text.contains("T-7")));
    }

    #[test]
    fn each_phone_has_its_own_squad_and_reset_starts_over() {
        let mut demo = Demo::default();
        let id = hello(&mut demo, "a").approvals[0].id;
        hello(&mut demo, "b");
        demo.command("a", Command::Accept { squad: SQUAD.into(), ticket: 2 }, 0);
        demo.command("a", Command::Decide { squad: SQUAD.into(), approval: id, decision: Decision::Deny }, 0);
        assert!(demo.command("nobody", Command::Accept { squad: SQUAD.into(), ticket: 2 }, 0).is_empty());
        let (a, b) = (hello(&mut demo, "a"), hello(&mut demo, "b"));
        assert_eq!(a.tickets[1].status, TicketStatus::Done);
        assert_eq!(b.tickets[1].status, TicketStatus::Review);
        assert_eq!(b.approvals.len(), 1);
        assert!(demo.due(100_000).iter().all(|(d, _)| d == "a"));
        demo.reset();
        assert_eq!(hello(&mut demo, "a").tickets[1].status, TicketStatus::Review);
    }

    #[test]
    fn work_sent_back_or_resumed_comes_back_and_a_turn_can_be_stopped() {
        let mut demo = Demo::default();
        hello(&mut demo, "a");
        let out = demo.command("a", Command::Resume { squad: SQUAD.into(), ticket: 4, note: "key is in 1Password".into() }, 0);
        assert!(matches!(&out[..], [Event::TicketChanged { ticket, .. }] if ticket.status == TicketStatus::InProgress));
        assert!(demo.due(10_000).iter().any(|(_, e)| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 4 && ticket.status == TicketStatus::Review)));
        assert!(demo.command("a", Command::Reopen { squad: SQUAD.into(), ticket: 1, note: String::new() }, 0).is_empty(), "only work in review goes back");
        demo.command("a", Command::Submit { session: BACKEND, text: "hi".into(), about: None }, 20_000);
        let out = demo.command("a", Command::Interrupt { session: BACKEND }, 20_100);
        assert!(matches!(&out[..], [Event::TurnEnded { outcome: TurnOutcome::Interrupted, .. }]));
        assert!(demo.due(60_000).iter().all(|(_, e)| e.session() != Some(BACKEND)));
    }

    /// The whole way for a store reviewer, against a relay running locally
    /// (`npx wrangler dev` in `relay/`) and `equipe demo serve` pointed at it:
    /// `EQUIPE_E2E_RELAY=http://localhost:8787 EQUIPE_E2E_HOME=/tmp/eqdemo cargo test -p equipe e2e_demo -- --ignored`
    /// Two phones pair with the one code; each gets the request, allows it and
    /// talks to the PM.
    #[tokio::test]
    #[ignore]
    async fn e2e_demo_phones_pair_with_the_standing_code_allow_and_talk_to_the_pm() {
        use super::super::core::tests::Phone;
        use super::super::core::{Action, slot};
        use super::super::seal::{self, Kind};
        use futures_util::{SinkExt as _, StreamExt as _};
        use serde_json::{Value, json};
        use tokio_tungstenite::tungstenite::Message as Ws;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
        let home = PathBuf::from(std::env::var("EQUIPE_E2E_HOME").expect("EQUIPE_E2E_HOME"));
        let url = std::fs::read_to_string(home.join("pairing-url.txt")).unwrap().trim().to_owned();
        let query: HashMap<String, String> = url.split_once('?').unwrap().1.split('&').map(|kv| {
            let (k, v) = kv.split_once('=').unwrap();
            (k.to_owned(), v.to_owned())
        }).collect();
        let relay = std::env::var("EQUIPE_E2E_RELAY").expect("EQUIPE_E2E_RELAY");
        let ws_base = relay.replacen("http://", "ws://", 1).replacen("https://", "wss://", 1);
        let (daemon, daemon_key) = (query["d"].clone(), query["k"].clone());
        let secret = super::super::unb64(&query["s"]).unwrap();
        for n in 1..=2 {
            // Pair: nobody is asked.
            let mut phone = Phone::new();
            let (mut slot_ws, _) = tokio_tungstenite::connect_async(format!("{ws_base}/v1/daemons/{daemon}/pair/{}", slot(&secret))).await.unwrap();
            let request = json!({ "type": "pair_request", "name": format!("reviewer {n}"), "platform": "test", "key": phone.keys.public });
            let data = seal::seal(Kind::PairRequest, &daemon_key, None, Some(&secret), &daemon, request.to_string().as_bytes()).unwrap();
            slot_ws.send(Ws::text(json!({ "type": "send", "data": data }).to_string())).await.unwrap();
            let answer = loop {
                let next = tokio::time::timeout(std::time::Duration::from_secs(10), slot_ws.next()).await.expect("an answer in time");
                let Some(Ok(Ws::Text(text))) = next else { panic!("the slot closed without an answer") };
                let frame: Value = serde_json::from_str(&text).unwrap();
                if frame["type"] == "recv" {
                    break frame;
                }
            };
            let answer: Value = serde_json::from_slice(&seal::open(Kind::PairAnswer, &phone.keys, Some(&daemon_key), Some(&secret), &daemon, answer["data"].as_str().unwrap()).unwrap()).unwrap();
            assert_eq!(answer["type"], "paired");
            phone.id = answer["device"].as_str().unwrap().to_owned();
            let token = answer["token"].as_str().unwrap().to_owned();
            // The relay opens the slot again a moment later.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let mut request = format!("{ws_base}/v1/daemons/{daemon}/devices/{}/connect", phone.id).into_client_request().unwrap();
            request.headers_mut().insert("Authorization", format!("Bearer {token}").parse().unwrap());
            let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
            let config = super::super::Config { relay: relay.clone(), id: daemon.clone(), token: String::new(), private: String::new(), public: daemon_key.clone(), devices: vec![] };
            let ask = |phone: &mut Phone, rpc: Value| {
                let frame = phone.send(&config, Some(SQUAD), rpc, super::super::now_ms());
                Ws::text(json!({ "type": "send", "data": frame["data"] }).to_string())
            };
            ws.send(ask(&mut phone, json!({ "jsonrpc": "2.0", "id": 1, "method": "hello", "params": { "version": "e2e", "squad": SQUAD } }))).await.unwrap();
            let (mut resolved, mut said, mut ended) = (false, String::new(), false);
            while !(resolved && ended) {
                let next = tokio::time::timeout(std::time::Duration::from_secs(15), ws.next()).await.expect("the demo answers in time");
                let Some(Ok(Ws::Text(text))) = next else { continue };
                let frame: Value = serde_json::from_str(&text).unwrap();
                if frame["type"] != "recv" {
                    continue;
                }
                let rpc = phone.open(&config, &Action::Relay(frame))["rpc"].clone();
                if rpc["id"] == 1 {
                    let approval = &rpc["result"]["snapshot"]["approvals"][0];
                    assert_eq!(approval["member"], "frontend", "{rpc}");
                    let decide = json!({ "type": "decide", "squad": SQUAD, "approval": approval["id"], "decision": "allow" });
                    ws.send(ask(&mut phone, json!({ "jsonrpc": "2.0", "method": "command", "params": decide }))).await.unwrap();
                    let submit = json!({ "type": "submit", "session": PM, "text": "What's left before launch?" });
                    ws.send(ask(&mut phone, json!({ "jsonrpc": "2.0", "method": "command", "params": submit }))).await.unwrap();
                    continue;
                }
                let event = &rpc["params"];
                match (event["type"].as_str(), event["session"].as_u64()) {
                    (Some("approval_resolved"), _) => resolved = event["decision"] == "allow",
                    (Some("text_delta"), Some(PM)) => said.push_str(event["text"].as_str().unwrap()),
                    (Some("turn_ended"), Some(PM)) => ended = true,
                    _ => {}
                }
            }
            assert!(said.contains("T-2"), "{said}");
        }
    }

    #[test]
    fn a_phone_that_floods_the_pm_stops_being_answered() {
        let mut demo = Demo::default();
        hello(&mut demo, "a");
        let answered = (0..1000).filter(|_| !demo.command("a", Command::Submit { session: PM, text: "hi".into(), about: None }, 0).is_empty()).count();
        assert!(answered < 100, "{answered}");
        assert!(demo.scenes["a"].later.len() <= LATER + 20);
    }

    #[test]
    fn the_code_is_kept_and_its_picture_is_a_qr_code() {
        let home = std::env::temp_dir().join(format!("equipe-demo-{}", std::process::id()));
        let a = secret(&home, false).unwrap();
        assert_eq!(secret(&home, false).unwrap(), a, "the same code across restarts");
        assert_ne!(secret(&home, true).unwrap(), a, "a new code when asked");
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(secret_path(&home)).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(&home).unwrap();
        assert!(qr_svg("equipe://pair?r=x").unwrap().starts_with("<svg"));
    }
}
