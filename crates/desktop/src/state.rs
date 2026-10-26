//! What the window knows about the squad it follows: the members and how each
//! one stands, kept up to date from the daemon's snapshot and events alone.
//! No UI types here, so the rules can be tested without a window.

use equipe_core::t;
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::time::Instant;

use equipe_core::protocol::rpc::{Hello, SquadEntry};
use equipe_core::protocol::{DesktopState, Engine, Entry, Estimate, Event, HumanCheck, Mode, OffMain, PlanUsage, SessionId, SessionState, Ticket, TicketEntry, TicketId, TicketKind, TicketStatus, TurnOutcome};

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
    /// When it expects what it is doing done, as it said (or equipe guessed).
    pub estimate: Option<Estimate>,
    /// What its latest tool was about: a file, a command.
    pub target: Option<String>,
    /// When it was last heard from (unix seconds; 0: not since the window opened).
    pub last_seen: u64,
    /// Its last turn failed, or it said something went wrong.
    pub failed: bool,
}

/// A member's conversation about one project, run beside its others. Lanes
/// aren't listed: choosing the member and the project shows one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    pub session: SessionId,
    /// The member's own session.
    pub of: SessionId,
    pub project: Option<String>,
    pub presence: Presence,
    pub verb: Verb,
}

/// What can be opened in the conversation's place: the PM (the one
/// conversation to talk to), a ticket's work, someone on the team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Pm,
    Ticket(TicketId),
    Member(SessionId),
}

/// The sections of what waits for the person, in their order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Section {
    /// A member asks for permission.
    Permission,
    /// A member puts a decision to the person: questions with options. 결정 필요.
    Decide,
    /// A ticket put down until the person does their part: 내 차례.
    Waiting,
    /// Work handed in: 검토.
    Review,
    /// Suggested work to take on or drop: 제안.
    Proposed,
    /// Accepted, but not on main yet — and not left on its branch by the
    /// person's choice: 수락했지만 아직 main에 안 합쳐짐.
    NotOnMain,
}

impl Section {
    pub const ALL: [Section; 6] = [Section::Permission, Section::Decide, Section::Waiting, Section::Review, Section::Proposed, Section::NotOnMain];
    /// The sections that list tickets.
    const TICKETS: [Section; 4] = [Section::Waiting, Section::Review, Section::Proposed, Section::NotOnMain];

    pub fn label(self) -> &'static str {
        match self {
            Section::Permission => t!("state.section_permission"),
            Section::Decide => t!("state.section_decide"),
            Section::Waiting => t!("state.status_waiting"),
            Section::Review => t!("state.status_review"),
            Section::Proposed => t!("state.status_proposed"),
            Section::NotOnMain => t!("state.not_on_main"),
        }
    }

    /// The line under a section's heading that says what it means, where its
    /// name alone doesn't.
    pub fn hint(self) -> Option<&'static str> {
        match self {
            Section::NotOnMain => Some(t!("state.not_on_main_hint")),
            _ => None,
        }
    }
}

/// One thing waiting for the person: a request for permission, a member's
/// questions, or a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Need {
    Permission(u64),
    /// A set of questions (`crate::decide`), by its id.
    Questions(u64),
    /// A change to the team the PM proposed (`crate::team`), by its id.
    Team(u64),
    /// Handed-in work the person said in words to accept, on a card to
    /// confirm (`crate::accept`), by its id.
    Accept(u64),
    Ticket(TicketId),
    /// A secret a member asked the person to type (`ask_secret`), by its id.
    Secret(u64),
    /// A member's browser page waiting for the person's own hand (a human
    /// check, a sign-in), by its page id ([`BrowserWait`]).
    Browser(u64),
}

/// What a status is called, everywhere: one word each.
pub fn status_word(status: TicketStatus) -> &'static str {
    match status {
        TicketStatus::Waiting => t!("state.status_waiting"),
        TicketStatus::Review => t!("state.status_review"),
        TicketStatus::Proposed => t!("state.status_proposed"),
        TicketStatus::Todo => t!("state.status_todo"),
        TicketStatus::InProgress => t!("state.status_in_progress"),
        TicketStatus::Done => t!("state.status_done"),
        TicketStatus::Cancelled => t!("state.status_cancelled"),
    }
}

/// How a ticket looked when the person acted on it: hidden from what waits
/// for them until it looks different — a merge that failed, say.
pub type Look = (TicketStatus, Option<bool>, Option<OffMain>);

pub fn look_of(t: &Ticket) -> Look {
    (t.status, t.merged, t.off_main.clone())
}

/// What a ticket's state is called where one ticket is shown: its status's
/// word — or, for accepted work not in main, that and where it is.
pub fn state_word(t: &Ticket) -> &'static str {
    match t.off_main {
        _ if t.in_design_qa() => t!("state.status_design_qa"),
        _ if !t.not_on_main() => status_word(t.status),
        Some(OffMain::Kept) => t!("state.accepted_kept"),
        _ => t!("state.accepted_not_on_main"),
    }
}

/// A member at work on a ticket is quiet past this many seconds without a word.
pub const QUIET_AFTER: u64 = 10 * 60;

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

/// What a member's turns have used, for `/usage`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Spent {
    /// Everything its turns processed, cached history included.
    pub tokens: u64,
    /// Of `seen.0` tokens, `seen.1` came from the prompt cache.
    pub seen: (u64, u64),
    /// (used, window) tokens of its context, once known.
    pub context: Option<(u64, u64)>,
}

type CardHint = (Vec<u64>, Vec<(TicketId, TicketStatus, Option<String>)>, Option<String>);

#[derive(Debug, Clone)]
pub struct Model {
    pub squad: String,
    pub members: Vec<Member>,
    /// Members' lanes, kept apart from `members`.
    pub lanes: Vec<Lane>,
    pub conn: Conn,
    pub tickets: Vec<Ticket>,
    /// The squad's projects, in the order they joined.
    pub projects: Vec<PathBuf>,
    /// The order the person dragged the projects into, by name (this Mac's).
    pub project_order: Vec<String>,
    /// The project that is the squad's HQ, where its operations live: first
    /// among the projects, wherever the others were dragged.
    pub hq: Option<PathBuf>,
    /// The squad's routines (`protocol::routines`), as the daemon has them.
    pub routines: Vec<equipe_core::protocol::routines::Routine>,
    /// The logins members sign in with, as the daemon keeps them (no passwords).
    pub logins: Vec<equipe_core::protocol::vault::Login>,
    /// The PM accepts the team's handed-in work itself: the person's inbox
    /// keeps only what is theirs.
    pub pm_accepts: bool,
    /// Secrets members asked the person to type, oldest first.
    pub secret_asks: Vec<equipe_core::protocol::secrets::SecretAsk>,
    /// The browser profiles besides the main one (other people's accounts).
    pub browser_profiles: Vec<equipe_core::protocol::BrowserProfile>,
    /// The squad's mode, for completion's sake.
    pub mode: Mode,
    /// The Claude plan's usage windows, as last reported.
    pub plan: Option<PlanUsage>,
    /// The daemon is on another machine (SSH): its folders aren't on this disk.
    pub remote: bool,
    /// Whether a guess at the next message is offered after an answer (`/suggest`).
    pub suggestions: bool,
    /// What each member's turns have used.
    pub spent: HashMap<SessionId, Spent>,
    /// The daemon machine's squads: where each one's folder is.
    pub squad_list: Vec<SquadEntry>,
    /// Open permission requests, oldest first.
    pub approvals: Queue,
    /// Members' questions waiting for the person, oldest first.
    pub asked: crate::decide::Asked,
    /// Changes to the team the PM proposed, waiting for the person, oldest first.
    pub team: crate::team::Proposals,
    /// Handed-in work put to the person to confirm, oldest first.
    pub accepts: crate::accept::Cards,
    /// Both engines' plans, the squad's switching and its cards (`engines`).
    pub engines: crate::switching::Engines,
    /// Things that need the person and have not been announced yet: the shell
    /// takes them and tells the person if the window is out of sight.
    alerts: Vec<Alert>,
    /// One quiet line the shell shows under the conversation, until taken.
    flash: Option<String>,
    /// Something the person asked for went wrong (a merge that didn't land):
    /// a notice of its own over the box, until taken.
    problem: Option<String>,
    /// Each member's conversation.
    pub convos: HashMap<SessionId, Convo>,
    /// Whose conversation is on screen.
    pub selected: Option<SessionId>,
    /// The ticket open on screen: its work in place of a conversation.
    pub opened: Option<TicketId>,
    /// The project in focus, by name; `None`: the whole squad.
    pub focus: Option<String>,
    /// Bumped whenever the selected conversation shows something new, so a
    /// screen can tell it must draw again.
    pub revision: u64,
    /// Now, in unix seconds: set by the shell before each update (never read here).
    pub unix: u64,
    /// When each ticket came to the state it is in, as seen from here.
    pub since: HashMap<TicketId, u64>,
    /// Tickets the person just acted on, as they looked then: out of what
    /// waits for them until the daemon says they changed.
    pub acted: HashMap<TicketId, Look>,
    /// Tickets that moved since the person last looked at them: news.
    pub news: std::collections::HashSet<TicketId>,
    /// The daemon's own version, as it said when the window attached.
    pub daemon_version: String,
    /// Each member's own mode, by session; absent: it follows the squad's.
    pub member_modes: HashMap<SessionId, Mode>,
    /// How hard each member thinks; absent: its engine's default.
    pub efforts: HashMap<SessionId, String>,
    /// The squad's "always allow" rules, oldest first.
    pub allowed: Vec<String>,
    /// The person turned off guesses at their next message.
    pub suggestions_off: bool,
    /// Every member's browser opens on screen.
    pub browsers_shown: bool,
    /// The person took the Mac's screen back from the members.
    pub desktop_stopped: bool,
    /// Who has the Mac's screen, and who waits for it.
    pub desktop: DesktopState,
    /// What the person answered tickets that waited for them with, here or on
    /// another screen (`crate::yourturn`), for the cards that asked.
    pub answers: HashMap<TicketId, crate::yourturn::Answered>,
    /// Members' browser pages waiting for the person — a "verify you are
    /// human" check, a sign-in, a code: in 나에게 until they pass or the
    /// browser is handed back.
    pub browser_waits: Vec<BrowserWait>,
    /// Members' browsers handed to the person (`show_my_browser`), by lane,
    /// with what to do there: waiting for their tabs to be told, then in
    /// `browser_waits`, until handed back.
    browser_asked: HashMap<String, String>,
    /// Each member's browser's tabs as last told, by lane.
    browser_tabs: HashMap<String, LaneTabs>,
    /// Tickets' records, as the daemon last sent them (a ticket page asks).
    pub histories: HashMap<TicketId, Vec<TicketEntry>>,
    /// Bumped whenever a ticket or a record changes, so the screens draw again.
    pub board_rev: u64,
    /// Bumped on every error the daemon says about the squad (a refused edit).
    pub squad_errors: u64,
}

/// A member's browser page that waits for the person. No member decides it,
/// and none can do it: a page that asks for a person (`check`), or a browser
/// its member handed them, saying what to do (`why`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserWait {
    pub page: u64,
    pub lane: String,
    pub session: Option<SessionId>,
    pub member: String,
    pub site: String,
    pub check: Option<HumanCheck>,
    /// Its member handed it over, for this — in its own words.
    pub why: Option<String>,
}

/// A member's browser's tabs, as last told.
#[derive(Debug, Clone, Default)]
struct LaneTabs {
    session: Option<SessionId>,
    member: Option<String>,
    pages: Vec<equipe_core::protocol::BrowserPage>,
    agent_page: Option<u64>,
}

/// The header's line about the Mac's screen; `None` when nobody is on it.
pub fn screen_line(desktop: &DesktopState, here: &str) -> Option<String> {
    let name = |m: &equipe_core::protocol::MemberRef| if m.squad == here { m.member.clone() } else { m.to_string() };
    if desktop.stopped {
        return Some(t!("header.screen_stopped").into());
    }
    let holder = desktop.holder.as_ref().map(|m| t!("header.screen_holder", name = name(m)));
    let waiting = match desktop.queue.as_slice() {
        [] => None,
        [one] => Some(t!("header.screen_waiting_one", name = name(one))),
        [first, rest @ ..] => Some(t!("header.screen_waiting_more", name = name(first), n = rest.len())),
    };
    let parts: Vec<String> = holder.into_iter().chain(waiting).collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

impl Model {
    pub fn new(squad: &str) -> Self {
        Self {
            squad: squad.to_owned(),
            members: Vec::new(),
            lanes: Vec::new(),
            conn: Conn::Connecting,
            tickets: Vec::new(),
            projects: Vec::new(),
            project_order: Vec::new(),
            hq: None,
            routines: Vec::new(),
            logins: Vec::new(),
            pm_accepts: false,
            secret_asks: Vec::new(),
            browser_profiles: Vec::new(),
            mode: Mode::default(),
            plan: None,
            remote: false,
            suggestions: true,
            spent: HashMap::new(),
            squad_list: Vec::new(),
            approvals: Queue::default(),
            asked: Default::default(),
            team: Default::default(),
            accepts: Default::default(),
            engines: Default::default(),
            alerts: Vec::new(),
            flash: None,
            problem: None,
            convos: HashMap::new(),
            selected: None,
            focus: None,
            opened: None,
            revision: 0,
            unix: 0,
            since: HashMap::new(),
            acted: HashMap::new(),
            news: Default::default(),
            daemon_version: String::new(),
            member_modes: HashMap::new(),
            efforts: HashMap::new(),
            allowed: Vec::new(),
            suggestions_off: false,
            browsers_shown: false,
            desktop_stopped: false,
            desktop: DesktopState::default(),
            answers: HashMap::new(),
            browser_waits: Vec::new(),
            browser_asked: HashMap::new(),
            browser_tabs: HashMap::new(),
            histories: HashMap::new(),
            board_rev: 0,
            squad_errors: 0,
        }
    }

    /// The member on screen.
    pub fn selected_member(&self) -> Option<&Member> {
        self.selected.and_then(|id| self.members.iter().find(|m| m.session == id))
    }

    /// The session whose conversation shows: a ticket's member's, with the
    /// ticket open; else the selected member's — or its conversation beside
    /// its work, when the PM runs one.
    pub fn viewed(&self) -> Option<SessionId> {
        if let Some(ticket) = self.opened {
            return self.convos.iter().find(|(_, c)| c.showing == Some(ticket)).map(|(&id, _)| id);
        }
        let selected = self.selected?;
        Some(self.lane_of(selected).map_or(selected, |l| l.session))
    }

    fn lane_of(&self, member: SessionId) -> Option<&Lane> {
        self.lanes.iter().find(|l| l.of == member)
    }

    /// The team as the sidebar lists it: the PM first, then whoever needs
    /// the person, who works, who rests, who left.
    pub fn team(&self) -> Vec<SessionId> {
        let pm = self.pm();
        pm.into_iter().chain(self.ordered().into_iter().map(|m| m.session).filter(|s| Some(*s) != pm)).collect()
    }

    /// The PM's session, or whoever came first without one.
    pub fn pm(&self) -> Option<SessionId> {
        self.members.iter().find(|m| Self::is_pm(m)).or(self.members.first()).map(|m| m.session)
    }

    /// Whether a ticket shows under the project in focus.
    pub fn in_focus(&self, ticket: &Ticket) -> bool {
        self.focus.is_none() || self.ticket_project_name(ticket) == self.focus
    }

    /// The sidebar's line open now.
    pub fn side_open(&self) -> Option<Side> {
        if let Some(ticket) = self.opened {
            return Some(Side::Ticket(ticket));
        }
        let selected = self.selected_member()?;
        Some(if Self::is_pm(selected) { Side::Pm } else { Side::Member(selected.session) })
    }

    /// Open a line of the sidebar: the PM's conversation, someone's, or a
    /// ticket — its member's work on it, while there is any to show. False for
    /// a ticket with none: it is the board's to show.
    pub fn open(&mut self, side: Side) -> bool {
        for convo in self.convos.values_mut() {
            convo.showing = None;
        }
        self.opened = None;
        self.revision += 1;
        match side {
            Side::Pm => {
                self.selected = self.members.iter().find(|m| Self::is_pm(m)).or(self.members.first()).map(|m| m.session);
                true
            }
            Side::Member(session) => {
                self.selected = Some(session);
                true
            }
            Side::Ticket(id) => {
                // Whose work it is: the member on it now, else who did it.
                let assignee = self.tickets.iter().find(|t| t.id == id).and_then(|t| t.assignee.clone());
                let who = self
                    .members
                    .iter()
                    .find(|m| m.ticket == Some(id))
                    .or_else(|| self.members.iter().find(|m| assignee.as_deref() == Some(m.name.as_str())))
                    .map(|m| m.session);
                let Some(who) = who.filter(|s| self.convos.get(s).is_some_and(|c| c.has_work_on(id))) else {
                    return false;
                };
                self.selected = Some(who);
                self.opened = Some(id);
                if let Some(convo) = self.convos.get_mut(&who) {
                    convo.showing = Some(id);
                }
                true
            }
        }
    }

    /// How many of a member's lanes are at work now.
    pub fn lanes_working(&self, member: SessionId) -> usize {
        self.lanes.iter().filter(|l| l.of == member && l.presence == Presence::Working).count()
    }

    /// The subagents a member handed work to and that are at it now — from
    /// its own conversation and its lanes' — oldest first, each with the
    /// conversation it works for.
    pub fn subagents_of(&self, member: SessionId) -> Vec<(SessionId, &crate::convo::Sub)> {
        let lanes = self.lanes.iter().filter(|l| l.of == member).map(|l| l.session);
        let mut subs: Vec<(SessionId, &crate::convo::Sub)> =
            std::iter::once(member).chain(lanes).filter_map(|s| self.convos.get(&s).map(|c| (s, c))).flat_map(|(s, c)| c.subagents.iter().map(move |sub| (s, sub))).collect();
        subs.sort_by_key(|(_, sub)| sub.since);
        subs
    }

    /// How many of a member's conversations are at work at once.
    pub fn at_once(&self, member: &Member) -> usize {
        self.lanes_working(member.session) + usize::from(member.presence == Presence::Working)
    }

    /// Whether the conversation in view is at work. The PM at work on a ticket
    /// of its own is free to talk: what is said opens its conversation beside
    /// the work.
    pub fn viewed_presence(&self) -> Option<Presence> {
        let member = self.selected_member()?;
        if self.opened.is_some() {
            return Some(member.presence);
        }
        Some(self.presence_to_talk(member))
    }

    /// How a member stands for what is said to it: its conversation beside
    /// its work, when one runs or the PM's would open.
    pub fn presence_to_talk(&self, member: &Member) -> Presence {
        if let Some(lane) = self.lane_of(member.session) {
            return lane.presence;
        }
        let beside = Self::is_pm(member) && member.presence == Presence::Working && member.ticket.is_some();
        if beside { Presence::Ready } else { member.presence }
    }

    /// Whether what is said now would open the PM's conversation beside its work.
    pub fn opens_lane(&self) -> bool {
        self.selected_member().is_some_and(|m| m.presence == Presence::Working) && self.viewed_presence() == Some(Presence::Ready)
    }

    pub fn convo(&self) -> Option<&Convo> {
        self.convos.get(&self.viewed()?)
    }

    /// The slice of the conversation in view the person sees: the project in
    /// focus (`Some(None)`: every project) — when they want the conversation
    /// to follow the project (`follow`) and it is the PM's. `None`: whole, as
    /// it always was.
    pub fn chat_slice(&self, follow: bool) -> Option<Option<String>> {
        self.convo().filter(|c| follow && c.sliceable()).map(|_| self.focus.clone())
    }

    /// The conversation in view as it shows (see [`Convo::sliced`]): with a
    /// project in focus, its thread and the squad's — never another project's.
    pub fn conversation(&self, follow: bool) -> crate::convo::Sliced {
        let Some(convo) = self.convo() else { return Default::default() };
        match self.chat_slice(follow) {
            Some(slice) => convo.sliced(slice.as_deref()),
            None => {
                let blocks = convo.blocks();
                crate::convo::Sliced { about: vec![None; blocks.len()], blocks }
            }
        }
    }

    pub fn convo_mut(&mut self) -> Option<&mut Convo> {
        let id = self.viewed()?;
        self.convos.get_mut(&id)
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
        if (self.selected == Some(session) && self.opened.is_none()) || !self.members.iter().any(|m| m.session == session) {
            return false;
        }
        for convo in self.convos.values_mut() {
            convo.showing = None;
        }
        self.opened = None;
        self.selected = Some(session);
        self.revision += 1;
        true
    }

    /// The names of the squad's projects, as chips show them.
    /// The projects in the person's order: those they placed first, as
    /// placed; the rest after, as the daemon lists them.
    pub fn order_projects(&mut self) {
        let (order, hq) = (&self.project_order, &self.hq);
        let rank = |p: &PathBuf| (Some(p) != hq.as_ref(), order.iter().position(|n| *n == equipe_core::board::project_name(p)).unwrap_or(usize::MAX));
        self.projects.sort_by_key(rank);
    }

    /// Project `name` moved to where `before` is (`None`: the end). The new
    /// order, by name, to keep.
    pub fn move_project(&mut self, name: &str, before: Option<&str>) -> Vec<String> {
        let mut names = self.project_names();
        let Some(from) = names.iter().position(|n| n == name) else { return names };
        let moved = names.remove(from);
        let to = before.and_then(|b| names.iter().position(|n| n == b)).unwrap_or(names.len());
        names.insert(to, moved);
        self.project_order = names.clone();
        self.order_projects();
        self.revision += 1;
        names
    }

    /// Whether the project called `name` is the squad's HQ.
    pub fn is_hq(&self, name: &str) -> bool {
        self.hq.as_ref().is_some_and(|hq| equipe_core::board::project_name(hq) == name)
    }

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
                self.asked.forget_sent();
                self.team.forget_sent();
                self.accepts.forget_sent();
                self.engines.forget_sent();
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
            // The shell reads the bridge's greeting itself; the squad list is kept for
            // `/home` (where the folder is) and `/squad` (what to switch to).
            Update::Squads(list) => {
                self.squad_list = list.squads;
                false
            }
            // The connecting screen's, kept by the shell.
            Update::Bridge(_) | Update::Leg(_) | Update::Door(_) => false,
            Update::Event(event) => {
                let before = (self.members.clone(), self.lanes.clone(), self.ticket_hint(), self.subagents_hint(), self.revision, self.card_hint(), self.settings_hint());
                self.on_event(event, now);
                let mut changed = before != (self.members.clone(), self.lanes.clone(), self.ticket_hint(), self.subagents_hint(), self.revision, self.card_hint(), self.settings_hint());
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

    /// What settings show of the squad, to tell when it changed.
    fn settings_hint(&self) -> (Mode, Vec<(SessionId, Mode)>, Vec<String>, bool) {
        let mut modes: Vec<(SessionId, Mode)> = self.member_modes.iter().map(|(s, m)| (*s, *m)).collect();
        modes.sort_by_key(|(s, _)| *s);
        (self.mode, modes, self.allowed.clone(), self.desktop_stopped)
    }

    fn load(&mut self, hello: Hello) {
        // The daemon tells its browser's tabs again right after.
        self.browser_waits.clear();
        self.browser_asked.clear();
        self.browser_tabs.clear();
        let version = hello.version;
        let snapshot = hello.snapshot;
        self.engines.load(&snapshot, self.unix as i64);
        self.squad = snapshot.squad;
        // When each came to where it stands: what the journal says, unless it
        // was seen here already in that state.
        for t in &snapshot.tickets {
            let seen = self.tickets.iter().find(|o| o.id == t.id).is_some_and(|o| o.status == t.status) && self.since.contains_key(&t.id);
            if let (false, Some(at)) = (seen, t.updated_at) {
                self.since.insert(t.id, at);
            }
        }
        self.tickets = snapshot.tickets.clone();
        self.projects = snapshot.projects.clone();
        self.hq = snapshot.hq.clone();
        self.pm_accepts = snapshot.pm_accepts;
        self.secret_asks = snapshot.secret_asks.clone();
        self.routines = snapshot.routines.clone();
        self.order_projects();
        self.mode = snapshot.mode;
        self.plan = snapshot.plan;
        self.suggestions = !snapshot.suggestions_off;
        self.spent = snapshot
            .sessions
            .iter()
            .filter(|s| s.lane.is_none())
            .map(|s| (s.session, Spent { tokens: s.tokens, seen: (0, 0), context: s.context }))
            .collect();
        self.approvals.load(snapshot.approvals.clone());
        self.asked.load(snapshot.questions.clone());
        self.team.load(snapshot.team_changes.clone());
        self.accepts.load(snapshot.accepts.clone());
        // An older daemon: the banner over the conversation says so, with its button.
        self.daemon_version = version;
        self.member_modes = snapshot.sessions.iter().filter_map(|s| Some((s.session, s.mode?))).collect();
        self.efforts = snapshot.sessions.iter().filter_map(|s| Some((s.session, s.effort.clone()?))).collect();
        self.allowed = snapshot.allowed.clone();
        self.suggestions_off = snapshot.suggestions_off;
        self.browsers_shown = snapshot.browsers_shown;
        self.desktop_stopped = snapshot.desktop.stopped;
        self.desktop = snapshot.desktop.clone();
        let (lanes, own): (Vec<&_>, Vec<&_>) = snapshot.sessions.iter().partition(|s| s.lane.is_some());
        self.lanes = lanes
            .iter()
            .filter_map(|s| {
                let lane = s.lane.as_ref()?;
                Some(Lane {
                    session: s.session,
                    of: lane.of,
                    project: lane.project.clone(),
                    presence: if s.state == SessionState::Working { Presence::Working } else { Presence::Ready },
                    verb: Verb::of_entry(s.recent.iter().rev().find(|e| matches!(e, Entry::Tool { .. } | Entry::Reply { .. } | Entry::User { .. }))),
                })
            })
            .collect();
        self.members = own
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
                    estimate: s.estimate.clone(),
                    target: s.recent.iter().rev().find_map(|e| match e {
                        Entry::Tool { target, .. } => target.clone(),
                        _ => None,
                    }),
                    last_seen: self.unix,
                    failed: false,
                }
            })
            .collect();
        let now = Instant::now();
        self.convos = snapshot
            .sessions
            .iter()
            .map(|s| {
                let mut convo = Convo::from_info(s, now);
                convo.lane = s.lane.is_some();
                (s.session, convo)
            })
            .collect();
        self.keep_selection();
        self.revision += 1;
    }

    fn on_event(&mut self, event: Event, now: Clock) {
        match &event {
            Event::PlanUsage { usage, engine } => {
                if *engine == Engine::Claude {
                    self.plan = Some(*usage);
                }
                self.engines.read(*engine, *usage, self.unix as i64);
            }
            Event::EngineProposed { squad, proposal } if *squad == self.squad => {
                self.engines.propose(proposal.clone());
            }
            Event::EngineProposalClosed { squad, id, .. } if *squad == self.squad => self.engines.close(*id),
            Event::EngineSwitchingChanged { squad, mode } if *squad == self.squad => self.engines.mode = *mode,
            Event::SecretAsked { squad, ask } if *squad == self.squad => {
                if !self.secret_asks.iter().any(|a| a.id == ask.id) {
                    self.secret_asks.push(ask.clone());
                    self.revision += 1;
                }
            }
            Event::SecretClosed { squad, id, .. } if *squad == self.squad => {
                self.secret_asks.retain(|a| a.id != *id);
                self.revision += 1;
            }
            Event::PmAcceptsChanged { squad, on } if *squad == self.squad => {
                self.pm_accepts = *on;
                self.revision += 1;
            }
            Event::RoutinesChanged { squad, routines } if *squad == self.squad => self.routines = routines.clone(),
            Event::Logins { logins } => self.logins = logins.clone(),
            Event::BrowserProfiles { profiles } => self.browser_profiles = profiles.clone(),
            Event::ProjectsChanged { squad, projects, hq } if *squad == self.squad => {
                self.hq = hq.clone();
                // The one just added, said under the box.
                if let Some(new) = projects.iter().find(|p| !self.projects.contains(p)) {
                    self.flash = Some(t!("header.project_added", name = equipe_core::board::project_name(new)));
                }
                self.projects = projects.clone();
                self.order_projects();
            }
            Event::TurnEnded { session, usage: Some(u), .. } if !self.lanes.iter().any(|l| l.session == *session) => {
                let spent = self.spent.entry(*session).or_default();
                spent.tokens += u.total();
                spent.seen.0 += u.total();
                spent.seen.1 += u.cache_tokens.unwrap_or(0);
                if let (Some(used), Some(window)) = (u.context_tokens, u.context_window) {
                    spent.context = Some((used, window));
                }
            }
            _ => {}
        }
        // The conversation first: what the selected one shows decides redraws.
        if let Some(id) = event.session() {
            if let Event::SessionStarted { squad, member, project, .. } = &event {
                let ours = squad.as_ref().is_none_or(|s| *s == self.squad);
                if ours && !self.convos.contains_key(&id) {
                    let convo = Convo::new(project.clone(), member.as_ref().is_some_and(|m| m.role == "pm"));
                    self.convos.insert(id, convo);
                }
            }
            // The conversation on screen: the selected member's — or its lane,
            // the PM's conversation beside its work, whose events are not the
            // member's session's (a steer said to it showed nothing).
            let viewed = self.selected == Some(id) || self.viewed() == Some(id);
            if let Some(convo) = self.convos.get_mut(&id)
                && convo.on_event(&event, Instant::now(), now)
                && viewed
            {
                self.revision += 1;
            }
            // An answer: the shell tells it if it is to what the person last wrote.
            if let Event::TurnEnded { outcome: TurnOutcome::Completed | TurnOutcome::Incomplete, .. } = &event
                && let Some(line) = self.convos.get(&id).and_then(|c| crate::notify::reply_line(c.transcript.entries()))
                && let Some(member) = self.members.iter().find(|m| m.session == id).or_else(|| self.lanes.iter().find(|l| l.session == id).and_then(|l| self.members.iter().find(|m| m.session == l.of)))
            {
                self.alerts.push(Alert::Reply { session: id, member: member.name.clone(), line });
            }
        }
        self.on_member_event(event);
        self.keep_selection();
    }

    fn on_member_event(&mut self, event: Event) {
        match event {
            Event::TicketChanged { ticket, .. } => self.on_ticket(&ticket),
            Event::TicketHistory { squad, ticket, entries } if squad == self.squad => {
                if self.histories.get(&ticket) != Some(&entries) {
                    self.histories.insert(ticket, entries);
                    self.board_rev += 1;
                }
            }
            Event::ApprovalRequested { squad, approval } => {
                if squad == self.squad && !self.approvals.has(approval.id) {
                    self.alerts.push(Alert::Approval { approval: approval.clone(), squad });
                    self.approvals.push(approval);
                }
            }
            Event::ApprovalResolved { approval, .. } => self.approvals.resolve(approval),
            Event::QuestionsAsked { squad, questions } if squad == self.squad => {
                if self.asked.get(questions.id).is_none() {
                    let session = self.members.iter().find(|m| m.name == questions.member).map_or(questions.session, |m| m.session);
                    self.alerts.push(Alert::Decision { session, member: questions.member.clone(), count: questions.questions.len() });
                }
                self.asked.push(questions);
            }
            Event::QuestionsClosed { squad, id, .. } if squad == self.squad => {
                self.asked.close(id);
            }
            // A change to the team waits for the person: announced as a decision is.
            Event::TeamChangeProposed { squad, change } if squad == self.squad => {
                let session = self.members.iter().find(|m| m.name == change.by).map_or(change.session, |m| m.session);
                let member = change.by.clone();
                if self.team.push(change) {
                    self.alerts.push(Alert::Decision { session, member, count: 1 });
                }
            }
            Event::TeamChangeClosed { squad, id, .. } if squad == self.squad => self.team.close(id),
            // Work to confirm: announced as a decision is — unless the person's own words raised it.
            Event::AcceptProposed { squad, proposal } if squad == self.squad => {
                let (session, member, words) = (proposal.session, proposal.by.clone(), proposal.from_words());
                if self.accepts.push(proposal) && !words {
                    self.alerts.push(Alert::Decision { session, member, count: 1 });
                }
            }
            Event::AcceptClosed { squad, id, .. } if squad == self.squad => self.accepts.close(id),
            Event::ModeChanged { squad, member, mode } if squad == self.squad => match member {
                None => self.mode = mode.unwrap_or_default(),
                Some(name) => {
                    if let Some(session) = self.members.iter().find(|m| m.name == name).map(|m| m.session) {
                        match mode {
                            Some(mode) => self.member_modes.insert(session, mode),
                            None => self.member_modes.remove(&session),
                        };
                    }
                }
            },
            Event::Allowed { squad, rules } if squad == self.squad => self.allowed = rules,
            Event::Desktop { desktop } => {
                self.desktop_stopped = desktop.stopped;
                self.desktop = desktop;
            }
            Event::LaneOpened { session, of, project } => self.open_lane(session, of, project),
            // Taken back, or sent at once: the member's count follows its conversation's.
            Event::MemoWithdrawn { session, .. } | Event::QueueSent { session, .. } => {
                let memos = self.convos.get(&session).map(|c| c.queue.memo_count());
                if let (Some(member), Some(memos)) = (self.members.iter_mut().find(|m| m.session == session), memos) {
                    member.memos = memos;
                }
            }
            Event::SessionClosed { session } if self.lanes.iter().any(|l| l.session == session) => self.fold_lane(session),
            Event::Error { session: None, message, .. } => {
                self.problem = Some(message);
                self.squad_errors += 1;
                self.board_rev += 1;
            }
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
                    estimate: None,
                    target: None,
                    last_seen: self.unix,
                    failed: false,
                });
            }
            other => {
                let Some(id) = other.session() else { return };
                if let Some(lane) = self.lanes.iter_mut().find(|l| l.session == id) {
                    match other {
                        Event::UserMessage { .. } => (lane.presence, lane.verb) = (Presence::Working, Verb::Thinking),
                        Event::ToolUse { tool, .. } => (lane.presence, lane.verb) = (Presence::Working, Verb::of_tool(&tool)),
                        Event::TextDelta { .. } => (lane.presence, lane.verb) = (Presence::Working, Verb::Writing),
                        Event::TurnEnded { .. } => lane.presence = Presence::Ready,
                        _ => {}
                    }
                    return;
                }
                let unix = self.unix;
                let Some(member) = self.members.iter_mut().find(|m| m.session == id) else { return };
                if !matches!(other, Event::Output { .. }) {
                    member.last_seen = unix;
                }
                match other {
                    Event::UserMessage { .. } => {
                        member.presence = Presence::Working;
                        member.verb = Verb::Thinking;
                        member.memos = 0;
                        member.failed = false;
                    }
                    Event::Estimated { estimate, .. } => member.estimate = estimate,
                    Event::Error { .. } => member.failed = true,
                    Event::MessageQueued { .. } => member.memos += 1,
                    Event::WorkStarted { ticket, .. } => {
                        member.failed = false;
                        member.ticket = Some(ticket);
                        member.presence = Presence::Working;
                        member.verb = Verb::Thinking;
                    }
                    Event::WorkEnded { .. } => member.ticket = None,
                    Event::ToolUse { tool, target, .. } => {
                        member.presence = Presence::Working;
                        member.verb = Verb::of_tool(&tool);
                        member.target = target;
                    }
                    Event::TextDelta { .. } => {
                        member.presence = Presence::Working;
                        member.verb = Verb::Writing;
                    }
                    Event::ModelKnown { model, .. } => member.model = Some(model),
                    Event::TurnEnded { usage, outcome, .. } => {
                        member.failed = outcome == TurnOutcome::Failed;
                        if member.failed {
                            self.alerts.push(Alert::Failed { session: member.session, member: member.name.clone() });
                        }
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

    /// A session just started turns out to be a lane: it leaves the list.
    fn open_lane(&mut self, session: SessionId, of: SessionId, project: Option<String>) {
        let Some(at) = self.members.iter().position(|m| m.session == session) else { return };
        let lane = self.members.remove(at);
        let main = self.convos.get(&of).cloned();
        if let Some(convo) = self.convos.get_mut(&session) {
            match &main {
                Some(main) => convo.open_lane(main),
                None => convo.lane = true,
            }
        }
        self.lanes.push(Lane { session, of, project, presence: lane.presence, verb: lane.verb });
        self.revision += 1;
    }

    /// A lane goes back into its member: what was said there stays, in the
    /// member's conversation about that project.
    fn fold_lane(&mut self, session: SessionId) {
        let Some(at) = self.lanes.iter().position(|l| l.session == session) else { return };
        let lane = self.lanes.remove(at);
        if let Some(convo) = self.convos.remove(&session)
            && let Some(main) = self.convos.get_mut(&lane.of)
        {
            main.take_back(lane.project, &convo);
        }
        self.revision += 1;
    }

    fn on_ticket(&mut self, ticket: &Ticket) {
        let was = self.tickets.iter().find(|t| t.id == ticket.id).map(|t| t.status);
        // Past its design QA: ready for the person now, as if just handed in.
        let qa_passed = ticket.ready_for_review() && self.tickets.iter().any(|t| t.id == ticket.id && t.in_design_qa());
        if let Some(alert) = self.tickets.iter().find(|t| t.id == ticket.id).and_then(|old| crate::notify::merge_alert(old, ticket, self.ticket_project_name(ticket))) {
            self.alerts.push(alert);
        }
        if was != Some(ticket.status) {
            self.since.insert(ticket.id, self.unix);
            if was.is_some() {
                self.news.insert(ticket.id);
            }
        }
        // What the person acted on came back changed: its look is news again.
        if self.acted.get(&ticket.id).is_some_and(|look| *look != look_of(ticket)) {
            self.acted.remove(&ticket.id);
        }
        if was.is_some() && (was != Some(ticket.status) || qa_passed) {
            match ticket.status {
                // A designer checks it first: the person hears once that passes.
                TicketStatus::Review if ticket.in_design_qa() => {}
                TicketStatus::Review => {
                    self.flash = Some(t!("state.review_flash", id = ticket.id));
                    self.alerts.push(Alert::Review { ticket: ticket.id, title: ticket.title.clone(), by: ticket.assignee.clone().or(ticket.owner.clone()), project: self.ticket_project_name(ticket) });
                }
                TicketStatus::Waiting => {
                    // Its card is in the conversation on screen: no need to say it twice.
                    let by = ticket.assignee.clone().or(ticket.owner.clone());
                    let here = self.selected_member().is_some_and(|m| by.as_deref() == Some(m.name.as_str()));
                    if !here {
                        let name = by.clone().unwrap_or_default();
                        self.flash = Some(t!("state.waiting_flash", id = ticket.id, name = name, count = ticket.asks().len().max(1)));
                    }
                    self.alerts.push(Alert::Waiting { ticket: ticket.id, by });
                }
                _ => {}
            }
        }
        // A card asking the person, answered here or elsewhere: it shows so.
        let asked = was == Some(TicketStatus::Waiting) || ticket.status == TicketStatus::Waiting;
        let answered = crate::yourturn::on_ticket(&mut self.answers, was, ticket, self.unix);
        if asked || answered {
            self.revision += 1;
        }
        match self.tickets.iter_mut().find(|t| t.id == ticket.id) {
            Some(slot) if slot == ticket => {}
            Some(slot) => {
                *slot = ticket.clone();
                self.board_rev += 1;
            }
            None => {
                self.tickets.push(ticket.clone());
                self.board_rev += 1;
            }
        }
    }

    /// The person sent a card's answers from here: it says so until the
    /// daemon moves the ticket.
    pub fn answer_sent(&mut self, id: TicketId, text: String) {
        crate::yourturn::sent(&mut self.answers, id, text, self.unix);
        self.revision += 1;
    }

    /// The subagents under each member, which the sidebar lists.
    fn subagents_hint(&self) -> Vec<(SessionId, Vec<crate::convo::Sub>)> {
        self.members
            .iter()
            .map(|m| (m.session, self.subagents_of(m.session).into_iter().map(|(_, s)| s.clone()).collect::<Vec<_>>()))
            .filter(|(_, subs)| !subs.is_empty())
            .collect()
    }

    /// What the cards above the composer show, for change detection.
    fn card_hint(&self) -> CardHint {
        (
            self.approvals.ids().into_iter().chain(self.asked.ids()).chain(self.team.ids()).chain(self.accepts.ids()).collect(),
            self.for_you_here().iter().map(|t| (t.id, t.status, t.note.clone())).collect(),
            self.flash.clone(),
        )
    }

    /// What the sidebar derives from tickets and approvals, for change detection.
    fn ticket_hint(&self) -> (usize, Vec<Status>, u64) {
        (self.needs_you_count(), self.members.iter().map(|m| self.status(m)).collect(), self.board_rev)
    }

    /// One of the daemon's browser events (the panel's, told here too): a
    /// member's page that starts waiting for the person, or a browser its
    /// member hands them, goes into 나에게 and is announced; one that passed,
    /// or a browser handed back, leaves.
    pub fn on_browser(&mut self, event: &Event) -> bool {
        let before = self.browser_waits.clone();
        match event {
            Event::BrowserTabs { squad, lane, session, member, pages, agent_page, .. } => {
                if lane.is_empty() || squad.as_deref().is_some_and(|s| s != self.squad) {
                    return false;
                }
                if pages.is_empty() {
                    self.browser_tabs.remove(lane);
                } else {
                    self.browser_tabs.insert(lane.clone(), LaneTabs { session: *session, member: member.clone(), pages: pages.clone(), agent_page: *agent_page });
                }
                // Handed over by its member: that says what waits, below.
                if !self.browser_asked.contains_key(lane) {
                    let waiting = pages.iter().filter(|p| p.check.is_some()).min_by_key(|p| Some(p.id) != *agent_page);
                    let kept = waiting.and_then(|p| self.browser_waits.iter().find(|w| w.lane == *lane && w.page == p.id).cloned());
                    self.browser_waits.retain(|w| w.lane != *lane);
                    if let Some(page) = waiting {
                        let name = self.lane_member(lane);
                        let site = crate::browse::site(&page.url).unwrap_or_default();
                        let wait = BrowserWait { page: page.id, lane: lane.clone(), session: *session, member: name, site, check: page.check, why: None };
                        if kept.is_none() {
                            self.alerts.push(Alert::Browser { page: wait.page, session: wait.session, member: wait.member.clone(), site: wait.site.clone(), check: wait.check, why: None });
                        }
                        self.browser_waits.push(wait);
                    }
                }
            }
            // A member hands the person its browser, saying what for.
            Event::BrowserControl { squad, lane, by: Some(_), why: Some(why), .. } if squad.as_deref().is_none_or(|s| s == self.squad) && !lane.is_empty() => {
                self.browser_asked.insert(lane.clone(), why.clone());
            }
            // Handed back to its member: not the person's to do any more.
            Event::BrowserControl { squad, lane, by: None, .. } if squad.as_deref().is_none_or(|s| s == self.squad) => {
                self.browser_asked.remove(lane);
                self.browser_waits.retain(|w| w.lane != *lane);
            }
            _ => {}
        }
        self.settle_handed();
        before != self.browser_waits
    }

    /// The browsers handed to the person whose tabs are known: in 나에게, on
    /// the tab its member works in, announced the first time.
    fn settle_handed(&mut self) {
        let mut asked: Vec<(String, String)> = self.browser_asked.iter().map(|(l, w)| (l.clone(), w.clone())).collect();
        asked.sort();
        for (lane, why) in asked {
            let Some(tabs) = self.browser_tabs.get(&lane) else { continue };
            let Some(page) = tabs.agent_page.and_then(|a| tabs.pages.iter().find(|p| p.id == a)).or(tabs.pages.last()).cloned() else { continue };
            let session = tabs.session;
            let site = crate::browse::site(&page.url).unwrap_or_default();
            let had = self.browser_waits.iter().any(|w| w.lane == lane && w.why.is_some());
            let member = self.lane_member(&lane);
            self.browser_waits.retain(|w| w.lane != lane);
            let wait = BrowserWait { page: page.id, lane, session, member, site, check: page.check, why: Some(why.clone()) };
            if !had {
                self.alerts.push(Alert::Browser { page: wait.page, session: wait.session, member: wait.member.clone(), site: wait.site.clone(), check: wait.check, why: Some(why) });
            }
            self.browser_waits.push(wait);
        }
    }

    /// Whose a member's browser is, by name: its conversation's member, else
    /// the host's label (`pm (site)`), else the lane's key.
    fn lane_member(&self, lane: &str) -> String {
        let tabs = self.browser_tabs.get(lane);
        tabs.and_then(|t| t.session)
            .and_then(|s| self.members.iter().find(|m| m.session == s).or_else(|| self.lanes.iter().find(|l| l.session == s).and_then(|l| self.members.iter().find(|m| m.session == l.of))))
            .map(|m| m.name.clone())
            .or_else(|| tabs.and_then(|t| t.member.as_deref()).map(|m| m.split_once(" (").map_or(m, |(n, _)| n).to_owned()))
            .unwrap_or_else(|| {
                let key = lane.rsplit('/').next().unwrap_or(lane);
                key.split_once('@').map_or(key, |(n, _)| n).to_owned()
            })
    }

    pub fn take_alerts(&mut self) -> Vec<Alert> {
        std::mem::take(&mut self.alerts)
    }

    pub fn take_flash(&mut self) -> Option<String> {
        self.flash.take()
    }

    /// Connected to the daemon, and told how things stand.
    pub fn attached(&self) -> bool {
        matches!(self.conn, Conn::Connected { .. })
    }

    pub fn take_problem(&mut self) -> Option<String> {
        self.problem.take()
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

    /// Everything waiting for the person, in one list, by section: requests
    /// for permission, tickets put down for them, work handed in, proposals,
    /// work accepted but not on main. The one definition the sidebar's count,
    /// 나에게 and notifications all read. Tickets follow the project in focus
    /// and leave once the person acted on them; requests always show.
    pub fn needs_you(&self) -> Vec<(Section, Need)> {
        let mut out: Vec<(Section, Need)> = self.approvals.open_items().map(|a| (Section::Permission, Need::Permission(a.id))).collect();
        out.extend(self.asked.open().map(|s| (Section::Decide, Need::Questions(s.id))));
        out.extend(self.team.open().map(|c| (Section::Decide, Need::Team(c.id))));
        out.extend(self.accepts.open().map(|c| (Section::Decide, Need::Accept(c.id))));
        // A secret a member waits for: theirs to type.
        out.extend(self.secret_asks.iter().map(|a| (Section::Waiting, Need::Secret(a.id))));
        // A page that waits for the person's own hand: their turn, first.
        out.extend(self.browser_waits.iter().map(|w| (Section::Waiting, Need::Browser(w.page))));
        for section in &Section::TICKETS {
            let mut tickets: Vec<&Ticket> = self.tickets.iter().filter(|t| self.section_of(t) == Some(*section) && self.in_focus(t) && !self.is_acted(t)).collect();
            let since = |t: &Ticket| self.since.get(&t.id).copied().unwrap_or(0);
            match section {
                // What it unblocks first, then the longest waiting.
                Section::Review => tickets.sort_by_key(|t| (std::cmp::Reverse(self.unblocks(t.id).len()), since(t), t.id)),
                _ => tickets.sort_by_key(|t| (since(t), t.id)),
            }
            out.extend(tickets.into_iter().map(|t| (*section, Need::Ticket(t.id))));
        }
        out
    }

    /// How many things wait for the person.
    pub fn needs_you_count(&self) -> usize {
        self.needs_you().len()
    }

    /// Whether a ticket waits for the person, and under which section.
    pub fn section_of(&self, t: &Ticket) -> Option<Section> {
        match t.status {
            TicketStatus::Waiting => Some(Section::Waiting),
            // A designer checks its screens first.
            TicketStatus::Review if t.in_design_qa() => None,
            // Where the PM accepts, handed-in work is the PM's: the person sees
            // only what it puts to them, on a card.
            TicketStatus::Review if self.pm_accepts => None,
            TicketStatus::Review => Some(Section::Review),
            // …and so are members' proposals; the PM's own are for the person.
            TicketStatus::Proposed if self.pm_accepts && (!self.is_pm_name(&t.created_by) || t.could_not_start()) => None,
            TicketStatus::Proposed => Some(Section::Proposed),
            // A conflict its owner is resolving comes back through the PM.
            TicketStatus::Done if self.pm_accepts && matches!(t.off_main, Some(equipe_core::protocol::OffMain::Conflict { .. })) => None,
            TicketStatus::Done if t.waits_to_merge() => Some(Section::NotOnMain),
            _ => None,
        }
    }

    /// Whether `name` is the squad's PM (or one of them).
    fn is_pm_name(&self, name: &str) -> bool {
        self.members.iter().any(|m| m.name == name && m.role == equipe_core::roles::PM) || name == equipe_core::roles::PM || name.starts_with("pm-")
    }

    pub fn is_acted(&self, t: &Ticket) -> bool {
        self.acted.get(&t.id).is_some_and(|look| *look == look_of(t))
    }

    /// The person acted on a ticket: out of what waits for them until it changes.
    pub fn act_on(&mut self, id: TicketId) {
        if let Some(t) = self.tickets.iter().find(|t| t.id == id) {
            self.acted.insert(id, look_of(t));
            self.revision += 1;
        }
    }

    /// The person looked at a ticket: no longer news.
    pub fn seen(&mut self, id: TicketId) {
        self.news.remove(&id);
    }

    /// An action taken back: the ticket waits for the person again.
    pub fn unact(&mut self, id: TicketId) {
        if self.acted.remove(&id).is_some() {
            self.revision += 1;
        }
    }

    /// Open tickets that wait for `id`.
    pub fn unblocks(&self, id: TicketId) -> Vec<TicketId> {
        self.tickets.iter().filter(|o| o.depends_on.contains(&id) && !o.status.is_closed()).map(|o| o.id).collect()
    }

    /// The first thing a ticket waits for that is not done yet.
    pub fn blocked_by(&self, t: &Ticket) -> Option<TicketId> {
        if t.status.is_closed() {
            return None;
        }
        t.depends_on.iter().copied().find(|d| self.tickets.iter().find(|o| o.id == *d).is_none_or(|o| o.status != TicketStatus::Done))
    }

    /// The member on a ticket now.
    pub fn member_on(&self, id: TicketId) -> Option<&Member> {
        self.members.iter().find(|m| m.ticket == Some(id) && m.presence != Presence::Closed)
    }

    /// Whether the member on a ticket is in the middle of a turn.
    pub fn is_live(&self, t: &Ticket) -> bool {
        t.status == TicketStatus::InProgress && self.member_on(t.id).is_some_and(|m| m.presence == Presence::Working)
    }

    /// How long, in seconds, the member on a ticket in progress has said nothing
    /// — once past [`QUIET_AFTER`].
    pub fn quiet_for(&self, t: &Ticket) -> Option<u64> {
        if t.status != TicketStatus::InProgress {
            return None;
        }
        let member = self.member_on(t.id)?;
        let quiet = self.unix.saturating_sub(member.last_seen);
        (member.last_seen > 0 && quiet >= QUIET_AFTER).then_some(quiet)
    }

    /// Whether the member on a ticket in progress hit an error.
    pub fn failed(&self, t: &Ticket) -> bool {
        t.status == TicketStatus::InProgress && self.member_on(t.id).is_some_and(|m| m.failed)
    }

    /// When the work on a ticket is expected done: the estimate of the member on it.
    pub fn eta(&self, t: &Ticket) -> Option<Eta> {
        if t.status != TicketStatus::InProgress {
            return None;
        }
        let estimate = self.member_on(t.id)?.estimate.as_ref()?;
        Some(Eta { at: estimate.done_at?, guessed: estimate.guessed })
    }

    /// How many tickets were finished since `midnight` (unix seconds, the
    /// local day's start: [`local_midnight`]) — done, not cancelled.
    pub fn done_since(&self, midnight: u64) -> usize {
        self.tickets.iter().filter(|t| t.status == TicketStatus::Done && self.since.get(&t.id).is_some_and(|at| *at >= midnight)).count()
    }

    /// How many tickets were finished today, local time.
    pub fn done_today(&self) -> usize {
        self.done_since(local_midnight(self.unix))
    }

    /// How long a ticket has been where it is, in seconds, when known.
    pub fn in_state_for(&self, t: &Ticket) -> Option<u64> {
        self.since.get(&t.id).filter(|s| **s > 0).map(|s| self.unix.saturating_sub(*s))
    }

    /// Tickets put down for the person or handed in, under the project in focus:
    /// reviews, then waits.
    pub fn for_you_here(&self) -> Vec<&Ticket> {
        let mut out: Vec<&Ticket> = Vec::new();
        for status in [TicketStatus::Review, TicketStatus::Waiting] {
            let mut group: Vec<&Ticket> = self.tickets.iter().filter(|t| t.status == status && !t.in_design_qa() && self.in_focus(t)).collect();
            group.sort_by_key(|t| t.id);
            out.extend(group);
        }
        out
    }

    /// Whether a ticket's work changes a repository and lands by a merge.
    pub fn merges(t: &Ticket) -> bool {
        t.kind == TicketKind::Code && t.branch.is_some()
    }

    pub fn status(&self, member: &Member) -> Status {
        let waiting = self
            .tickets
            .iter()
            .find(|t| t.status == TicketStatus::Waiting && t.owner.as_deref() == Some(member.name.as_str()))
            .map(|t| t.id);
        let asking = self.approvals.first_for(member.session);
        let deciding = self.asked.from(&member.name).is_some();
        if member.presence != Presence::Closed && (asking.is_some() || waiting.is_some() || deciding) {
            return Status::NeedsYou(asking.and_then(|a| a.ticket).or(member.ticket).or(waiting));
        }
        match member.presence {
            Presence::Closed => Status::Closed,
            Presence::Working => Status::Working { ticket: member.ticket, verb: member.verb },
            Presence::Ready if self.lanes_working(member.session) > 0 => {
                let verb = self.lanes.iter().find(|l| l.of == member.session && l.presence == Presence::Working).map_or(Verb::Working, |l| l.verb);
                Status::Working { ticket: member.ticket, verb }
            }
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

/// When work is expected done (unix seconds), and whether that is equipe's guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Eta {
    pub at: u64,
    pub guessed: bool,
}

/// `14:30` in local time.
pub fn local_hm(unix: u64) -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let secs = unix as libc::time_t;
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

/// The start of the local day `unix` falls in (midnight, as the Mac's clock
/// and time zone have it), in unix seconds.
pub fn local_midnight(unix: u64) -> u64 {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let secs = unix as libc::time_t;
    unsafe { libc::localtime_r(&secs, &mut tm) };
    tm.tm_hour = 0;
    tm.tm_min = 0;
    tm.tm_sec = 0;
    // Daylight saving as that day has it.
    tm.tm_isdst = -1;
    let midnight = unsafe { libc::mktime(&mut tm) };
    if midnight < 0 || midnight as u64 > unix { unix - unix % 86_400 } else { midnight as u64 }
}

/// Now, in unix seconds: for the shell to hand the model.
pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A span of time, short: `45s`, `25m`, `3h`, `2d`.
pub fn short_span(secs: u64) -> String {
    match secs {
        0..=59 => t!("time.span_secs", n = secs),
        60..=3599 => t!("time.span_mins", n = secs / 60),
        3600..=86_399 => t!("time.span_hours", n = secs / 3600),
        _ => t!("time.span_days", n = secs / 86_400),
    }
}

fn percent((used, window): (u64, u64)) -> Option<u8> {
    (window > 0).then(|| (used * 100 / window).min(100) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::rpc::Hello;
    use equipe_core::protocol::{Access, Approval, Command, Engine, Member as Who, SessionInfo, Snapshot, SubagentInfo, WorkInfo};
    use std::time::Duration;

    const T0: Clock = Clock { hour: 14, minute: 2, second: 10 };
    const T1: Clock = Clock { hour: 14, minute: 3, second: 12 };

    #[test]
    fn projects_keep_the_order_they_were_dragged_into() {
        let mut model = Model::new("acme");
        model.projects = vec!["/c/mcp".into(), "/c/website".into(), "/c/hq".into()];
        assert_eq!(model.move_project("hq", Some("mcp")), ["hq", "mcp", "website"]);
        assert_eq!(model.project_names(), ["hq", "mcp", "website"]);
        assert_eq!(model.move_project("hq", None), ["mcp", "website", "hq"], "dropped at the end");
        // A list from the daemon keeps the order; a new project goes after.
        model.projects = vec!["/c/hq".into(), "/c/new".into(), "/c/website".into(), "/c/mcp".into()];
        model.order_projects();
        assert_eq!(model.project_names(), ["mcp", "website", "hq", "new"]);
    }

    #[test]
    fn the_hq_comes_first_wherever_the_others_were_dragged() {
        let mut model = Model::new("acme");
        model.projects = vec!["/c/mcp".into(), "/c/website".into(), "/c/hq".into()];
        model.project_order = vec!["website".into(), "mcp".into(), "hq".into()];
        model.hq = Some("/c/hq".into());
        model.order_projects();
        assert_eq!(model.project_names(), ["hq", "website", "mcp"]);
        assert!(model.is_hq("hq") && !model.is_hq("mcp"));
        // Dragged below another, it stays on top: the HQ is the squad's, not a slot.
        model.move_project("hq", None);
        assert_eq!(model.project_names(), ["hq", "website", "mcp"]);
        // The mark taken off: back where it was dragged.
        model.apply(Update::Event(Event::ProjectsChanged { squad: "acme".into(), projects: model.projects.clone(), hq: None }), T0);
        assert_eq!(model.project_names(), ["website", "mcp", "hq"]);
    }

    #[test]
    fn a_members_page_waiting_for_the_person_is_theirs_to_do_until_it_passes() {
        use equipe_core::protocol::{BrowserPage, HumanCheck};
        let mut model = Model::new("acme");
        let tabs = |check: Option<HumanCheck>| Event::BrowserTabs {
            squad: Some("acme".into()),
            lane: "acme/backend@squad".into(),
            session: None,
            member: Some("backend (the whole squad)".into()),
            color: None,
            pages: vec![BrowserPage { id: 9_000_001, url: "https://dash.cloudflare.com/login".into(), title: "Just a moment...".into(), check, ..Default::default() }],
            agent_page: Some(9_000_001),
        };
        assert!(!model.on_browser(&tabs(None)));
        assert!(model.on_browser(&tabs(Some(HumanCheck::Challenge))));
        assert_eq!(model.browser_waits[0].site, "dash.cloudflare.com");
        assert_eq!(model.browser_waits[0].member, "backend");
        assert!(model.needs_you().contains(&(Section::Waiting, Need::Browser(9_000_001))), "in 나에게, as the person's turn");
        assert!(matches!(&model.take_alerts()[..], [Alert::Browser { page: 9_000_001, .. }]), "announced once");
        // Told again (a title changed): not announced again.
        assert!(!model.on_browser(&tabs(Some(HumanCheck::Challenge))));
        assert!(model.take_alerts().is_empty());
        // Passed: it leaves.
        assert!(model.on_browser(&tabs(None)));
        assert!(model.needs_you().is_empty());
        // Handed back while it still waits: not the person's any more.
        model.on_browser(&tabs(Some(HumanCheck::Captcha)));
        assert!(model.on_browser(&Event::BrowserControl { squad: Some("acme".into()), lane: "acme/backend@squad".into(), by: None, why: None, check: None }));
        assert!(model.browser_waits.is_empty());
        // Another squad's, or the person's own tabs: none.
        let mut other = tabs(Some(HumanCheck::SignIn));
        if let Event::BrowserTabs { squad, .. } = &mut other {
            *squad = Some("side".into());
        }
        assert!(!model.on_browser(&other));
    }

    #[test]
    fn a_browser_its_member_hands_over_is_the_persons_to_do_until_it_comes_back() {
        use equipe_core::protocol::{BrowserPage, HumanCheck};
        let mut model = Model::new("acme");
        let why = "Umbriles Inc. 계정 소유자 Apple ID로 App Store Connect에 로그인해 주세요";
        let tabs = |url: &str, check: Option<HumanCheck>| Event::BrowserTabs {
            squad: Some("acme".into()),
            lane: "acme/pm".into(),
            session: None,
            member: Some("pm".into()),
            color: None,
            pages: vec![BrowserPage { id: 7_000_001, url: url.into(), title: "App Store Connect".into(), check, ..Default::default() }],
            agent_page: Some(7_000_001),
        };
        let handed = Event::BrowserControl { squad: Some("acme".into()), lane: "acme/pm".into(), by: Some("person".into()), why: Some(why.into()), check: None };
        // Handed over before its tabs are told (the lane just opened): waits for them.
        assert!(!model.on_browser(&handed));
        assert!(model.needs_you().is_empty() && model.take_alerts().is_empty());
        assert!(model.on_browser(&tabs("https://appstoreconnect.apple.com/", None)));
        assert!(model.needs_you().contains(&(Section::Waiting, Need::Browser(7_000_001))), "in 나에게");
        let wait = &model.browser_waits[0];
        assert_eq!((wait.member.as_str(), wait.why.as_deref(), wait.check), ("pm", Some(why), None));
        assert!(matches!(&model.take_alerts()[..], [Alert::Browser { page: 7_000_001, why: Some(_), .. }]), "announced once");
        // On to Apple's sign-in: the same hand-over, now known to be a sign-in — not announced again.
        model.on_browser(&tabs("https://idmsa.apple.com/appleauth/auth/signin", Some(HumanCheck::SignIn)));
        assert_eq!(model.browser_waits.len(), 1);
        assert_eq!((model.browser_waits[0].check, model.browser_waits[0].site.as_str()), (Some(HumanCheck::SignIn), "idmsa.apple.com"));
        assert!(model.take_alerts().is_empty());
        // The host saying the person has it (no words this time) changes nothing.
        assert!(!model.on_browser(&Event::BrowserControl { squad: Some("acme".into()), lane: "acme/pm".into(), by: Some("person".into()), why: None, check: None }));
        // Signed in, the page waits no more — but it is still theirs until handed back.
        model.on_browser(&tabs("https://appstoreconnect.apple.com/apps", None));
        assert_eq!(model.needs_you().len(), 1);
        assert!(model.on_browser(&Event::BrowserControl { squad: Some("acme".into()), lane: "acme/pm".into(), by: None, why: None, check: None }));
        assert!(model.needs_you().is_empty(), "handed back: gone from 나에게");
        model.on_browser(&tabs("https://appstoreconnect.apple.com/apps", None));
        assert!(model.needs_you().is_empty(), "and it stays gone");
    }

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
            steering_in: vec![],
            lane: None,
            estimate: None,
            work: None,
            mode: None,
            effort: None,
            subagents: Vec::new(),
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

    /// A PM whose history went over the app, then the site.
    fn pm_in_two_projects() -> Model {
        let mut model = Model::new("acme");
        let mut pm = session(1, "pm", "pm", SessionState::Ready);
        let about = |p: &str| Entry::Thread { project: Some(p.into()) };
        let said = |t: &str| Entry::User { text: t.into() };
        let reply = |t: &str| Entry::Reply { text: t.into() };
        pm.recent = vec![about("app"), said("app q"), reply("app a"), about("site"), said("site q"), reply("site a")];
        model.apply(hello(vec![pm, session(2, "frontend", "frontend", SessionState::Ready)]), T0);
        model
    }

    fn said(sliced: &crate::convo::Sliced) -> Vec<String> {
        use crate::convo::Block;
        sliced.blocks.iter().filter_map(|b| if let Block::User(t) = b { Some(t.clone()) } else { None }).collect()
    }

    #[test]
    fn a_project_in_focus_shows_its_own_thread_and_never_another_projects() {
        let mut model = pm_in_two_projects();
        // Every project: everything, each stretch saying its project.
        assert_eq!(said(&model.conversation(true)), ["app q", "site q"]);
        let before = model.revision;
        assert!(model.set_focus(Some("app".into())));
        assert!(model.revision > before, "a draw is due");
        assert_eq!(model.chat_slice(true), Some(Some("app".into())));
        // The app's thread alone: the site's lines are not there at all —
        // no fold row, no pill pointing at them.
        let app = model.conversation(true);
        assert_eq!(said(&app), ["app q"]);
        assert!(!app.blocks.iter().any(|b| matches!(b, crate::convo::Block::Reply { text, .. } if text == "site a")));
        model.set_focus(Some("site".into()));
        let site = model.conversation(true);
        assert_eq!(said(&site), ["site q"]);
        assert!(matches!(site.blocks.last(), Some(crate::convo::Block::Reply { text, .. }) if text == "site a"), "its latest line last");
        // Following the project off in settings: the whole conversation.
        assert_eq!(said(&model.conversation(false)), ["app q", "site q"]);
    }

    #[test]
    fn what_was_said_to_the_whole_squad_shows_in_every_project() {
        let mut model = Model::new("acme");
        let mut pm = session(1, "pm", "pm", SessionState::Ready);
        let about = |p: Option<&str>| Entry::Thread { project: p.map(Into::into) };
        let said = |t: &str| Entry::User { text: t.into() };
        let reply = |t: &str| Entry::Reply { text: t.into() };
        pm.recent = vec![about(None), said("plan the week"), reply("ok"), about(Some("site")), said("site q"), reply("site a"), about(Some("app")), said("app q"), reply("app a")];
        model.apply(hello(vec![pm]), T0);
        model.set_focus(Some("app".into()));
        assert_eq!(self::said(&model.conversation(true)), ["plan the week", "app q"], "the squad's own lines are every project's; the site's are not");
        // A project nothing was said about yet: only what was said to everyone.
        model.projects.push("/c/new".into());
        model.set_focus(Some("new".into()));
        assert_eq!(self::said(&model.conversation(true)), ["plan the week"]);
    }

    #[test]
    fn with_the_setting_off_the_conversation_is_as_it_always_was() {
        let mut model = pm_in_two_projects();
        let whole = model.convo().unwrap().blocks();
        for focus in [None, Some("app"), Some("site")] {
            model.set_focus(focus.map(str::to_owned));
            assert_eq!(model.chat_slice(false), None);
            let shown = model.conversation(false);
            assert_eq!(shown.blocks, whole);
            assert!(shown.about.iter().all(Option::is_none));
        }
        // A member's conversation never follows the project.
        model.set_focus(Some("app".into()));
        model.select(2);
        assert_eq!(model.chat_slice(true), None);
    }

    #[test]
    fn the_snapshot_fills_the_list_and_connects() {
        let mut model = Model::new("acme");
        let mut busy = session(2, "frontend", "frontend", SessionState::Working);
        busy.work = Some(WorkInfo { ticket: 61, project: "/p".into(), workspace: None });
        busy.recent = vec![Entry::Tool { tool: "Edit".into(), target: None, outcome: None, label: None, background: false }];
        assert!(model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), busy]), T0));
        assert_eq!(model.conn, Conn::Connected { last_event: None });
        assert_eq!(model.status(&model.members[1]), Status::Working { ticket: Some(61), verb: Verb::Editing });
        assert_eq!(model.status(&model.members[0]), Status::Ready { ticket: None, model: Some("opus".into()) });
    }

    #[test]
    fn what_settings_show_of_the_squad_follows_the_snapshot_and_the_daemon() {
        let mut model = Model::new("acme");
        let mut backend = session(2, "backend", "backend", SessionState::Ready);
        backend.mode = Some(Mode::Ask);
        backend.effort = Some("high".into());
        let snapshot = Snapshot {
            squad: "acme".into(),
            sessions: vec![session(1, "pm", "pm", SessionState::Ready), backend],
            mode: Mode::ReadOnly,
            allowed: vec!["Bash(gh:*)".into()],
            suggestions_off: true,
            browsers_shown: true,
            ..Default::default()
        };
        model.apply(Update::Attached(Hello { version: "0.37.0+1".into(), snapshot }), T0);
        assert_eq!(model.daemon_version, "0.37.0+1");
        assert_eq!((model.mode, model.member_modes.get(&2), model.member_modes.get(&1)), (Mode::ReadOnly, Some(&Mode::Ask), None));
        assert_eq!(model.efforts.get(&2).map(String::as_str), Some("high"));
        assert_eq!(model.allowed, ["Bash(gh:*)"]);
        assert!(model.suggestions_off && model.browsers_shown && !model.desktop_stopped);

        let event = |e: Event| Update::Event(e);
        assert!(model.apply(event(Event::ModeChanged { squad: "acme".into(), member: None, mode: Some(Mode::Full) }), T0), "a mode changed redraws");
        assert!(model.apply(event(Event::ModeChanged { squad: "acme".into(), member: Some("backend".into()), mode: None }), T0));
        assert_eq!((model.mode, model.member_modes.get(&2)), (Mode::Full, None), "the member follows the squad again");
        // Another squad's word changes nothing here.
        model.apply(event(Event::Allowed { squad: "other".into(), rules: Vec::new() }), T0);
        assert_eq!(model.allowed, ["Bash(gh:*)"]);
        assert!(model.apply(event(Event::Allowed { squad: "acme".into(), rules: Vec::new() }), T0));
        assert!(model.allowed.is_empty());
        let stopped = equipe_core::protocol::DesktopState { stopped: true, ..Default::default() };
        assert!(model.apply(event(Event::Desktop { desktop: stopped }), T0));
        assert!(model.desktop_stopped);
    }

    #[test]
    fn a_turn_that_failed_is_announced() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(2, "backend", "backend", SessionState::Working)]), T0);
        let ended = |outcome| Update::Event(Event::TurnEnded { session: 2, outcome, usage: None });
        model.apply(ended(equipe_core::protocol::TurnOutcome::Interrupted), T0);
        assert!(model.take_alerts().is_empty(), "stopped by the person: nothing to say");
        model.apply(ended(equipe_core::protocol::TurnOutcome::Failed), T0);
        assert_eq!(model.take_alerts(), vec![Alert::Failed { session: 2, member: "backend".into() }]);
    }

    #[test]
    fn an_answer_and_a_merge_are_news_for_the_shell_to_tell() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready)]), T0);
        let event = |e: Event| Update::Event(e);
        model.apply(event(Event::UserMessage { session: 1, text: "어떻게 됐어?".into() }), T0);
        model.apply(event(Event::TextDelta { session: 1, text: "**다 됐어요.** 테스트도\n통과".into() }), T0);
        assert!(model.take_alerts().is_empty(), "not before the turn ends");
        model.apply(event(Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None }), T0);
        assert_eq!(model.take_alerts(), vec![Alert::Reply { session: 1, member: "pm".into(), line: "다 됐어요. 테스트도".into() }]);
        model.apply(event(Event::UserMessage { session: 1, text: "멈춰".into() }), T0);
        model.apply(event(Event::TurnEnded { session: 1, outcome: TurnOutcome::Interrupted, usage: None }), T0);
        assert!(model.take_alerts().is_empty(), "stopped, and nothing said since the person spoke");

        let code = |status: &str, merged: Option<bool>| -> Ticket {
            serde_json::from_value(serde_json::json!({"id": 4, "title": "로그인", "body": "", "status": status, "owner": "backend", "assignee": "backend", "project": null, "depends_on": [], "branch": "eq/t4", "merged": merged, "created_by": "pm", "note": null})).unwrap()
        };
        model.apply(event(Event::TicketChanged { squad: "acme".into(), ticket: code("done", Some(true)) }), T0);
        assert!(model.take_alerts().is_empty(), "a ticket seen for the first time isn't news");
        model.apply(event(Event::TicketChanged { squad: "acme".into(), ticket: code("review", None) }), T0);
        model.take_alerts();
        model.apply(event(Event::TicketChanged { squad: "acme".into(), ticket: code("done", Some(true)) }), T0);
        assert!(matches!(&model.take_alerts()[..], [Alert::Merged { ticket: 4, .. }]));
    }

    #[test]
    fn events_move_a_member_between_states() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready)]), T0);

        assert!(model.apply(Update::Event(Event::WorkStarted { session: 1, ticket: 5, title: "t".into(), project: "/p".into(), workspace: None }), T0));
        assert_eq!(model.status(&model.members[0]), Status::Working { ticket: Some(5), verb: Verb::Thinking });
        model.apply(Update::Event(Event::ToolUse { session: 1, tool: "Read".into(), target: None, label: None, background: false }), T0);
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
        assert_eq!(model.needs_you(), vec![(Section::Waiting, Need::Ticket(3))]);
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
        // backend's conversation is on screen: its card says it there, the flash doesn't.
        assert_eq!(model.take_flash().as_deref(), Some("T-3 검토 · 나에게에서 A 수락, R 변경 요청"));
        assert!(model.take_alerts().is_empty());
    }

    #[test]
    fn a_wait_elsewhere_is_one_short_line_and_its_card_follows_where_it_was_answered() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), session(2, "backend", "backend", SessionState::Working)]), T0);
        model.select(1);
        let tick = |status, note: Option<&str>| {
            let mut t = ticket(3, status, note);
            t.assignee = Some("backend".into());
            Update::Event(Event::TicketChanged { squad: "acme".into(), ticket: t })
        };
        model.apply(tick("in_progress", None), T0);
        model.apply(tick("waiting", Some("Waiting on you:\n- [ ] 토큰을 알려 주세요.\n- [ ] 일정을 정해 주세요.")), T0);
        assert_eq!(model.take_flash().as_deref(), Some("T-3 내 차례 — backend이(가) 2가지를 묻습니다"), "no asks run together, no keys to learn");
        // Answered on another screen: the card learns it as the ticket moves.
        let revision = model.revision;
        model.unix = 1_000;
        model.apply(tick("todo", Some("Resumed: 1. 토큰을 알려 주세요.\n→ ghp_x")), T0);
        assert!(model.revision > revision, "the conversation draws its card again");
        let answered = &model.answers[&3];
        assert_eq!((answered.here, answered.settled, answered.at), (false, true, Some(1_000)));
        assert_eq!(answered.text, "1. 토큰을 알려 주세요.\n→ ghp_x");
        // Asked again, then answered from here.
        model.apply(tick("waiting", Some("Waiting on you:\n- [ ] 다시 알려 주세요.")), T0);
        assert!(!model.answers.contains_key(&3), "a new round");
        model.answer_sent(3, "1. 다시 알려 주세요.\n→ ok".into());
        let t = model.tickets.iter().find(|t| t.id == 3).cloned();
        assert_eq!(crate::yourturn::phase(t.as_ref(), model.answers.get(&3)), crate::yourturn::Phase::Sending);
        model.apply(tick("todo", Some("Resumed: 1. 다시 알려 주세요.\n→ ok")), T0);
        let t = model.tickets.iter().find(|t| t.id == 3).cloned();
        assert_eq!(crate::yourturn::phase(t.as_ref(), model.answers.get(&3)), crate::yourturn::Phase::Answered { here: true });
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
    fn a_project_in_focus_narrows_the_tickets_not_the_conversation() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready)]), T0);
        model.projects = vec!["/code/site".into(), "/code/app".into()];
        let mut site = ticket(3, "review", None);
        site.project = Some("/code/site".into());
        let mut app = ticket(4, "todo", None);
        app.project = Some("/code/app".into());
        model.tickets = vec![site, app];
        assert!(model.set_focus(Some("site".into())));
        assert!(!model.set_focus(Some("site".into())));
        assert_eq!(model.viewed(), Some(1));
        assert_eq!(model.needs_you(), vec![(Section::Review, Need::Ticket(3))]);
        assert!(!model.in_focus(&model.tickets[1].clone()));
        model.set_focus(Some("app".into()));
        assert!(model.needs_you().is_empty(), "the review is site's");
        assert!(model.set_focus(None));
        assert!(model.in_focus(&model.tickets[1].clone()));
    }

    #[test]
    fn the_sidebar_opens_the_pm_a_tickets_work_or_sends_a_ticket_without_any_to_the_board() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), session(2, "frontend", "frontend", SessionState::Working)]), T0);
        let mut working = ticket(7, "in_progress", None);
        working.assignee = Some("frontend".into());
        model.tickets = vec![working, ticket(8, "todo", None)];
        model.members[1].ticket = Some(7);
        model.apply(Update::Event(Event::Notice { session: 2, text: "── T-7 · Hero — on branch b".into() }), T0);
        model.apply(Update::Event(Event::UserMessage { session: 2, text: "hero work".into() }), T0);
        assert_eq!(model.team(), [1, 2], "the PM heads the team");
        // A ticket at work: its member's work on it, and what is typed steers it.
        assert!(model.open(Side::Ticket(7)));
        assert_eq!((model.viewed(), model.side_open()), (Some(2), Some(Side::Ticket(7))));
        let plan = crate::compose::plan(&model, "skip the footer", false);
        assert_eq!(plan.commands, vec![equipe_core::protocol::Command::Submit { session: 2, text: "skip the footer".into(), about: Some(None) }]);
        // A ticket with no work to show: the board's to show.
        assert!(!model.open(Side::Ticket(8)));
        assert_eq!(model.opened, None);
        // Back to the PM.
        assert!(model.open(Side::Pm));
        assert_eq!(model.viewed(), Some(1));
    }

    #[test]
    fn a_closed_selection_falls_back_to_the_pm() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), session(2, "qa", "qa", SessionState::Ready)]), T0);
        model.select(2);
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready)]), T1);
        assert_eq!(model.selected, Some(1));
    }

    /// The PM at work on a ticket of its own; the daemon then opens its
    /// conversation (session 77) beside the work.
    fn with_lane() -> Model {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), session(2, "frontend", "frontend", SessionState::Ready)]), T0);
        model.projects = vec!["/code/site".into(), "/code/app".into()];
        model.apply(Update::Event(Event::UserMessage { session: 1, text: "사이트는?".into() }), T0);
        model.members[0].ticket = Some(9);
        model.apply(Update::Event(Event::SessionStarted { session: 77, project: "/p".into(), engine: Engine::Claude, model: None, workspace: None, member: Some(Who { role: "pm".into(), name: "pm".into() }), squad: Some("acme".into()) }), T0);
        model.apply(Update::Event(Event::LaneOpened { session: 77, of: 1, project: None }), T0);
        model
    }

    #[test]
    fn a_lane_leaves_the_member_list_and_is_what_shows_about_its_project() {
        let mut model = with_lane();
        assert_eq!(names(&model), ["pm", "frontend"], "the lane is no row of its own");
        assert_eq!(model.lanes.len(), 1);
        assert_eq!(model.selected, Some(1));
        assert_eq!(model.viewed(), Some(77));
        model.apply(Update::Event(Event::UserMessage { session: 77, text: "앱은?".into() }), T0);
        model.apply(Update::Event(Event::TextDelta { session: 77, text: "앱은 괜찮습니다.".into() }), T0);
        // What the member said before, then the lane's own.
        assert_eq!(model.convo().unwrap().blocks().len(), 3);
        // Whatever project is in focus: one conversation.
        model.set_focus(Some("site".into()));
        assert_eq!(model.viewed(), Some(77));
    }

    /// What the conversation on screen draws: its pending bubbles and the
    /// person's own messages, as the chat pane gets them.
    fn on_screen(model: &Model) -> Vec<crate::convo::Block> {
        model.conversation(true).blocks
    }

    #[test]
    fn what_is_said_to_the_pms_lane_at_work_shows_at_once_in_every_slice() {
        // The user's report: the PM's conversation beside its work (a lane)
        // at work; a steer said to it never showed — its events drew nothing,
        // being the lane's, not the selected member's.
        for focus in [None, Some("site".to_owned())] {
            let mut model = with_lane();
            model.set_focus(focus.clone());
            let before = model.revision;
            // As the daemon says it: the PM's desk marks the project the message is about.
            model.apply(Update::Event(Event::Thread { session: 77, project: focus.clone() }), T0);
            assert!(model.apply(Update::Event(Event::UserMessage { session: 77, text: "앱은?".into() }), T0), "the person's message in the lane is a redraw");
            assert!(model.revision > before, "the chat pane syncs only on a new revision");
            assert!(on_screen(&model).iter().any(|b| matches!(b, crate::convo::Block::User(t) if t == "앱은?")), "their bubble shows at once ({focus:?})");
            let before = model.revision;
            assert!(model.apply(Update::Event(Event::SteerQueued { session: 77, text: "그리고 웹도".into(), project: focus.clone() }), T0), "a steer queued is a redraw");
            assert!(model.revision > before);
            assert!(
                on_screen(&model).iter().any(|b| matches!(b, crate::convo::Block::Pending { text, steer: true, .. } if text == "그리고 웹도")),
                "the pending steer shows ({focus:?}): {:?}",
                on_screen(&model)
            );
            // Read at its next step: the bubble is the conversation's.
            let before = model.revision;
            model.apply(Update::Event(Event::Steered { session: 77, text: "그리고 웹도".into() }), T0);
            assert!(model.revision > before);
            assert!(!on_screen(&model).iter().any(|b| matches!(b, crate::convo::Block::Pending { .. })));
            // And the lane's reply streams in as it comes.
            let before = model.revision;
            model.apply(Update::Event(Event::TextDelta { session: 77, text: "웹도 봤어요.".into() }), T0);
            assert!(model.revision > before, "a reply streaming in the lane draws");
        }
    }

    #[test]
    fn a_lane_at_work_not_in_view_draws_nothing() {
        let mut model = with_lane();
        model.select(2);
        let before = model.revision;
        model.apply(Update::Event(Event::SteerQueued { session: 77, text: "x".into(), project: None }), T0);
        assert_eq!(model.revision, before, "another conversation's steer doesn't redraw this one");
    }

    #[test]
    fn the_member_row_reads_how_many_conversations_work_at_once() {
        let mut model = with_lane();
        // The lane is at work (the PM too, in site): two at once.
        model.apply(Update::Event(Event::UserMessage { session: 77, text: "앱은?".into() }), T0);
        assert_eq!(model.at_once(&model.members[0]), 2);
        // Only the lane at work: the member counts as working too.
        model.apply(Update::Event(Event::TurnEnded { session: 1, outcome: equipe_core::protocol::TurnOutcome::Completed, usage: None }), T0);
        assert_eq!(model.at_once(&model.members[0]), 1);
        assert!(matches!(model.status(&model.members[0]), Status::Working { .. }));
        model.apply(Update::Event(Event::TurnEnded { session: 77, outcome: equipe_core::protocol::TurnOutcome::Completed, usage: None }), T0);
        assert!(matches!(model.status(&model.members[0]), Status::Ready { .. }));
    }

    #[test]
    fn stopping_reaches_only_the_conversation_in_view() {
        let mut model = with_lane();
        model.apply(Update::Event(Event::UserMessage { session: 77, text: "앱은?".into() }), T0);
        assert_eq!((model.viewed(), model.viewed_presence()), (Some(77), Some(Presence::Working)));
        // Without its conversation beside, the PM at work on its ticket is free to talk: one would open.
        model.apply(Update::Event(Event::SessionClosed { session: 77 }), T0);
        assert_eq!((model.viewed(), model.viewed_presence()), (Some(1), Some(Presence::Ready)));
        assert!(model.opens_lane());
    }

    #[test]
    fn a_lane_that_closes_goes_back_into_its_member() {
        let mut model = with_lane();
        model.apply(Update::Event(Event::UserMessage { session: 77, text: "앱은?".into() }), T0);
        model.apply(Update::Event(Event::TextDelta { session: 77, text: "괜찮습니다.".into() }), T0);
        model.apply(Update::Event(Event::TurnEnded { session: 77, outcome: equipe_core::protocol::TurnOutcome::Completed, usage: None }), T0);
        model.apply(Update::Event(Event::SessionClosed { session: 77 }), T0);
        assert!(model.lanes.is_empty());
        assert!(!model.convos.contains_key(&77));
        assert_eq!(model.members.len(), 2);
        assert!(model.members.iter().all(|m| m.presence != Presence::Closed), "a lane closing is not a member leaving");
        // What was said in it stays, in the member's one conversation.
        assert_eq!(model.viewed(), Some(1));
        let shown = model.convo().unwrap().shown();
        assert!(shown.iter().any(|e| matches!(e.as_ref(), Entry::User { text } if text == "앱은?")), "{shown:?}");
        assert!(shown.iter().any(|e| matches!(e.as_ref(), Entry::User { text } if text == "사이트는?")));
    }

    #[test]
    fn a_snapshot_keeps_lanes_apart_from_members() {
        let mut lane = session(9, "pm", "pm", SessionState::Working);
        lane.lane = Some(equipe_core::protocol::Lane { of: 1, project: Some("app".into()) });
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), lane]), T0);
        assert_eq!(model.members.len(), 1);
        assert_eq!(model.lanes.len(), 1);
        assert_eq!(model.at_once(&model.members[0]), 1);
        model.set_focus(Some("app".into()));
        assert_eq!(model.viewed(), Some(9));
        // Whatever project is in focus, it is the one shown.
        model.set_focus(Some("site".into()));
        assert_eq!(model.viewed(), Some(9));
    }

    fn subs(model: &Model, member: SessionId) -> Vec<(SessionId, String, Option<String>)> {
        model.subagents_of(member).into_iter().map(|(s, sub)| (s, sub.what.clone(), sub.step.clone())).collect()
    }

    #[test]
    fn a_members_subagents_start_step_and_end_under_it_and_each_is_a_redraw() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), session(2, "frontend", "frontend", SessionState::Working)]), T0);
        // Not the conversation on screen, yet the sidebar draws again each time.
        assert!(model.apply(Update::Event(Event::SubagentStarted { session: 2, id: "a".into(), what: "Read the logs".into() }), T0));
        assert!(model.apply(Update::Event(Event::SubagentStarted { session: 2, id: "b".into(), what: "Run the tests".into() }), T0));
        assert!(model.apply(Update::Event(Event::SubagentStep { session: 2, id: "b".into(), step: "Bash cargo test".into() }), T0));
        assert_eq!(subs(&model, 2), [(2, "Read the logs".into(), None), (2, "Run the tests".into(), Some("Bash cargo test".into()))]);
        assert!(subs(&model, 1).is_empty());
        assert!(model.apply(Update::Event(Event::SubagentEnded { session: 2, id: "a".into() }), T0));
        assert_eq!(subs(&model, 2), [(2, "Run the tests".into(), Some("Bash cargo test".into()))]);
        // Someone else's subagent ending changes nothing here.
        assert!(!model.apply(Update::Event(Event::SubagentEnded { session: 2, id: "zz".into() }), T0));
    }

    #[test]
    fn attaching_again_shows_the_subagents_at_work_now_and_not_those_that_ended_meanwhile() {
        let mut model = Model::new("acme");
        let mut frontend = session(2, "frontend", "frontend", SessionState::Working);
        frontend.subagents = vec![SubagentInfo { id: "a".into(), what: "Read the logs".into(), step: Some("Grep error".into()), secs: 30 }];
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), frontend.clone()]), T0);
        assert_eq!(subs(&model, 2), [(2, "Read the logs".into(), Some("Grep error".into()))]);
        let (_, sub) = model.subagents_of(2)[0];
        assert!(sub.since.elapsed() >= Duration::from_secs(30), "its clock picks up where the daemon's was");
        // The link went down; meanwhile one handed back and another started.
        model.apply(Update::Down(Failure::Closed), T0);
        frontend.subagents = vec![SubagentInfo { id: "c".into(), what: "Compare flows".into(), step: None, secs: 2 }];
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), frontend]), T0);
        assert_eq!(subs(&model, 2), [(2, "Compare flows".into(), None)]);
    }

    #[test]
    fn a_lanes_subagents_are_its_members_oldest_first() {
        let mut model = with_lane();
        model.apply(Update::Event(Event::SubagentStarted { session: 1, id: "a".into(), what: "own".into() }), T0);
        std::thread::sleep(Duration::from_millis(2));
        model.apply(Update::Event(Event::SubagentStarted { session: 77, id: "b".into(), what: "lane's".into() }), T0);
        assert_eq!(subs(&model, 1), [(1, "own".into(), None), (77, "lane's".into(), None)]);
    }

    fn needs_model() -> Model {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), session(2, "backend", "backend", SessionState::Working)]), T0);
        let mut merged_not = ticket(9, "done", None);
        merged_not.branch = Some("b".into());
        merged_not.merged = Some(false);
        let mut unblocker = ticket(6, "review", None);
        unblocker.branch = Some("b6".into());
        let mut blocked = ticket(7, "todo", None);
        blocked.depends_on = vec![6];
        model.tickets = vec![ticket(5, "review", None), unblocker, blocked, ticket(4, "proposed", None), ticket(3, "waiting", None), merged_not, ticket(2, "in_progress", None), ticket(1, "done", None)];
        model.approvals.push(approval(11, 2));
        model
    }

    #[test]
    fn a_members_questions_wait_under_their_own_heading_mark_it_and_go_once_answered_anywhere() {
        use equipe_core::protocol::decide::{Choice, Question, QuestionSet};
        let mut model = needs_model();
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), session(2, "backend", "backend", SessionState::Working)]), T0);
        model.take_alerts();
        let q = Question { question: "Which?".into(), header: String::new(), options: vec![Choice { label: "a".into(), description: String::new(), ..Choice::default() }, Choice { label: "b".into(), description: String::new(), ..Choice::default() }], multi_select: false, ..Question::default() };
        let set = QuestionSet { id: 40, session: 2, member: "backend".into(), ticket: None, questions: vec![q.clone(), q], asked_at: 1 };
        assert!(model.apply(Update::Event(Event::QuestionsAsked { squad: "acme".into(), questions: set.clone() }), T0), "the card shows: a redraw");
        assert_eq!(model.take_alerts(), vec![Alert::Decision { session: 2, member: "backend".into(), count: 2 }]);
        // Another squad's, or the same again: nothing.
        model.apply(Update::Event(Event::QuestionsAsked { squad: "other".into(), questions: QuestionSet { id: 41, ..set.clone() } }), T0);
        model.apply(Update::Event(Event::QuestionsAsked { squad: "acme".into(), questions: set.clone() }), T0);
        assert!(model.take_alerts().is_empty());
        let decide: Vec<Need> = model.needs_you().into_iter().filter(|(s, _)| *s == Section::Decide).map(|(_, n)| n).collect();
        assert_eq!(decide, [Need::Questions(40)]);
        assert_eq!(model.needs_you()[0].0, Section::Decide, "right after requests for permission, before tickets");
        assert_eq!(model.status(&model.members[1]), Status::NeedsYou(None));
        // Answered from here: hidden at once; answered anywhere: gone.
        model.asked.sent(40);
        assert!(!model.needs_you().iter().any(|(s, _)| *s == Section::Decide));
        model.asked.forget_sent();
        assert!(model.apply(Update::Event(Event::QuestionsClosed { squad: "acme".into(), id: 40, answers: None }), T0), "answered on another screen: it goes, live");
        assert!(model.asked.ids().is_empty());
        assert!(matches!(model.status(&model.members[1]), Status::Working { .. }));
    }

    #[test]
    fn a_change_to_the_team_waits_to_be_decided_and_goes_once_decided_anywhere() {
        use equipe_core::protocol::team::{Change, TeamChange};
        let mut model = needs_model();
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready), session(2, "mobile", "mobile", SessionState::Working)]), T0);
        model.take_alerts();
        let change = TeamChange {
            id: 70,
            change: Change::Add,
            role: "mobile".into(),
            count: 1,
            member: None,
            engine: None,
            model: None,
            effort: None,
            reason: "T-7 waits".into(),
            unblocks: vec![7],
            by: "pm".into(),
            session: 1,
            proposed_at: 1,
            busy_on: None,
        };
        assert!(model.apply(Update::Event(Event::TeamChangeProposed { squad: "acme".into(), change: change.clone() }), T0), "the card shows: a redraw");
        assert_eq!(model.take_alerts(), vec![Alert::Decision { session: 1, member: "pm".into(), count: 1 }]);
        model.apply(Update::Event(Event::TeamChangeProposed { squad: "other".into(), change: TeamChange { id: 71, ..change.clone() } }), T0);
        model.apply(Update::Event(Event::TeamChangeProposed { squad: "acme".into(), change: change.clone() }), T0);
        assert!(model.take_alerts().is_empty(), "another squad's, or the same again: nothing");
        let decide: Vec<Need> = model.needs_you().into_iter().filter(|(s, _)| *s == Section::Decide).map(|(_, n)| n).collect();
        assert_eq!(decide, [Need::Team(70)], "결정 필요");
        // Approved here: the person's command, and hidden at once; decided anywhere: gone, live.
        assert_eq!(model.team.decide("acme", 70, true), Some(Command::ApproveTeamChange { squad: "acme".into(), id: 70 }));
        assert!(!model.needs_you().iter().any(|(_, n)| *n == Need::Team(70)));
        model.team.forget_sent();
        assert!(model.apply(Update::Event(Event::TeamChangeClosed { squad: "acme".into(), id: 70, approved: true, members: vec!["mobile-2".into()] }), T0));
        assert!(model.team.ids().is_empty());
    }

    #[test]
    fn a_card_to_confirm_waits_to_be_decided_announced_only_when_a_member_raised_it() {
        use equipe_core::protocol::accept::{AcceptProposal, FROM_WORDS, Verdict};
        let mut model = needs_model();
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready)]), T0);
        model.take_alerts();
        let card = AcceptProposal { id: 80, verdict: Verdict::Accept, tickets: vec![7], note: String::new(), summary: String::new(), by: "pm".into(), session: 1, proposed_at: 1 };
        assert!(model.apply(Update::Event(Event::AcceptProposed { squad: "acme".into(), proposal: card.clone() }), T0), "the card shows: a redraw");
        assert_eq!(model.take_alerts(), vec![Alert::Decision { session: 1, member: "pm".into(), count: 1 }]);
        // Raised from the person's own words a moment ago: they know; nothing to announce.
        model.apply(Update::Event(Event::AcceptProposed { squad: "acme".into(), proposal: AcceptProposal { id: 81, by: FROM_WORDS.into(), ..card.clone() } }), T0);
        model.apply(Update::Event(Event::AcceptProposed { squad: "other".into(), proposal: AcceptProposal { id: 82, ..card.clone() } }), T0);
        assert!(model.take_alerts().is_empty());
        let decide: Vec<Need> = model.needs_you().into_iter().filter(|(s, _)| *s == Section::Decide).map(|(_, n)| n).collect();
        assert_eq!(decide, [Need::Accept(80), Need::Accept(81)], "결정 필요");
        assert_eq!(model.accepts.shown_in(1).map(|c| c.id), Some(80));
        // Confirmed here: the person's command, hidden at once; closed anywhere: gone, live.
        assert!(matches!(model.accepts.decide("acme", 80, true, |_| None), Some(Command::ConfirmAccept { id: 80, .. })));
        assert!(!model.needs_you().iter().any(|(_, n)| *n == Need::Accept(80)));
        model.accepts.forget_sent();
        assert!(model.apply(Update::Event(Event::AcceptClosed { squad: "acme".into(), id: 80, confirmed: true, tickets: vec![7] }), T0));
        assert_eq!(model.accepts.ids(), [81]);
    }

    #[test]
    fn what_needs_you_is_one_list_in_one_order() {
        let model = needs_model();
        let sections: Vec<Section> = model.needs_you().iter().map(|(s, _)| *s).collect();
        assert_eq!(sections, [Section::Permission, Section::Waiting, Section::Review, Section::Review, Section::Proposed, Section::NotOnMain]);
        let needs: Vec<Need> = model.needs_you().into_iter().map(|(_, n)| n).collect();
        assert_eq!(needs, [Need::Permission(11), Need::Ticket(3), Need::Ticket(6), Need::Ticket(5), Need::Ticket(4), Need::Ticket(9)], "the review that unblocks work first");
        assert_eq!(model.needs_you_count(), 6);
    }

    #[test]
    fn a_hand_in_in_design_qa_waits_for_the_designer_then_comes_to_the_person() {
        let mut model = needs_model();
        model.take_alerts();
        let five = model.tickets.iter().find(|t| t.id == 5).unwrap().clone();
        let held = Ticket { design_qa: Some(12), ..five.clone() };
        model.apply(Update::Event(Event::TicketChanged { squad: "acme".into(), ticket: held.clone() }), T0);
        assert_eq!(state_word(&held), "디자인 QA 중");
        assert_eq!(model.section_of(&held), None, "not the person's yet");
        assert!(!model.for_you_here().iter().any(|t| t.id == 5));
        assert!(!model.take_alerts().iter().any(|a| matches!(a, Alert::Review { ticket: 5, .. })));
        // Passed: the person hears it is ready, as with any hand-in.
        model.apply(Update::Event(Event::TicketChanged { squad: "acme".into(), ticket: five }), T0);
        assert!(model.take_alerts().iter().any(|a| matches!(a, Alert::Review { ticket: 5, .. })));
        assert_eq!(model.section_of(model.tickets.iter().find(|t| t.id == 5).unwrap()), Some(Section::Review));
    }

    #[test]
    fn accepted_work_asks_to_be_merged_only_while_there_is_something_to_merge_and_the_person_hasnt_kept_it() {
        let mut model = needs_model();
        assert_eq!(state_word(model.tickets.iter().find(|t| t.id == 9).unwrap()), "수락함 · main에 안 합쳐짐");
        let not_on_main = |m: &Model| m.needs_you().into_iter().filter(|(s, _)| *s == Section::NotOnMain).map(|(_, n)| n).collect::<Vec<_>>();
        assert_eq!(not_on_main(&model), [Need::Ticket(9)]);
        let nine = model.tickets.iter().find(|t| t.id == 9).unwrap().clone();
        // Merging it was tried and failed: back in the inbox, its reason with it.
        model.act_on(9);
        assert!(not_on_main(&model).is_empty());
        let clash = Ticket { off_main: Some(OffMain::Conflict { detail: "a.rs".into() }), ..nine.clone() };
        model.apply(Update::Event(Event::TicketChanged { squad: "acme".into(), ticket: clash }), T0);
        assert_eq!(not_on_main(&model), [Need::Ticket(9)]);
        // Kept on its branch by the person: out of the inbox, still not on main.
        let kept = Ticket { off_main: Some(OffMain::Kept), ..nine.clone() };
        model.apply(Update::Event(Event::TicketChanged { squad: "acme".into(), ticket: kept.clone() }), T0);
        assert!(not_on_main(&model).is_empty() && kept.not_on_main());
        assert_eq!(model.section_of(&kept), None);
        assert_eq!(state_word(&kept), "수락함 · 브랜치에 둠");
        // Merged after all: back to its status's word.
        assert_eq!(state_word(&Ticket { merged: Some(true), off_main: None, ..kept.clone() }), "끝남");
        // +0 −0, or in main another way: the daemon finds nothing to merge and
        // says it is merged — done, not waiting.
        let mut empty = ticket(10, "done", None);
        empty.branch = Some("equipe/t-10".into());
        empty.merged = Some(false);
        model.apply(Update::Event(Event::TicketChanged { squad: "acme".into(), ticket: empty.clone() }), T0);
        assert_eq!(not_on_main(&model), [Need::Ticket(10)]);
        model.apply(Update::Event(Event::TicketChanged { squad: "acme".into(), ticket: Ticket { merged: Some(true), ..empty } }), T0);
        assert!(not_on_main(&model).is_empty());
        // A task's work lands in the squad's folder: never asks.
        let mut task = ticket(11, "done", None);
        task.kind = equipe_core::protocol::TicketKind::Task;
        task.branch = Some("b".into());
        task.merged = Some(false);
        assert_eq!(model.section_of(&task), None);
    }

    #[test]
    fn reviews_that_unblock_as_much_go_oldest_first() {
        let mut model = needs_model();
        model.since.insert(6, 500);
        model.since.insert(5, 100);
        model.tickets.retain(|t| t.id != 7);
        let reviews: Vec<Need> = model.needs_you().into_iter().filter(|(s, _)| *s == Section::Review).map(|(_, n)| n).collect();
        assert_eq!(reviews, [Need::Ticket(5), Need::Ticket(6)]);
    }

    #[test]
    fn what_the_person_acted_on_leaves_until_it_changes_and_comes_back_when_taken_back() {
        let mut model = needs_model();
        model.act_on(5);
        assert!(!model.needs_you().contains(&(Section::Review, Need::Ticket(5))));
        model.unact(5);
        assert!(model.needs_you().contains(&(Section::Review, Need::Ticket(5))));
        model.act_on(5);
        // The daemon says it is done now: news again, and no longer waiting.
        let mut done = ticket(5, "done", None);
        done.owner = None;
        model.apply(Update::Event(Event::TicketChanged { squad: "acme".into(), ticket: done }), T0);
        assert!(model.acted.is_empty());
    }

    #[test]
    fn a_status_change_is_stamped_and_reads_as_time_in_state() {
        let mut model = needs_model();
        model.unix = 1_000;
        model.apply(Update::Event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(2, "review", None) }), T0);
        assert_eq!(model.since.get(&2), Some(&1_000));
        model.unix = 1_000 + 25 * 60;
        assert_eq!(model.in_state_for(&model.tickets.iter().find(|t| t.id == 2).unwrap().clone()), Some(25 * 60));
        assert_eq!(short_span(25 * 60), "25분");
        assert_eq!(short_span(3 * 3600), "3시간");
        assert_eq!(short_span(2 * 86_400), "2일");
    }

    #[test]
    fn work_under_way_knows_its_blocker_eta_quiet_and_error() {
        let mut model = needs_model();
        let t = |m: &Model, id: u32| m.tickets.iter().find(|t| t.id == id).unwrap().clone();
        assert_eq!(model.blocked_by(&t(&model, 7)), Some(6));
        assert_eq!(model.unblocks(6), vec![7]);
        model.unix = 10_000;
        model.members[1].ticket = Some(2);
        model.members[1].last_seen = 10_000 - 25 * 60;
        model.members[1].estimate = Some(Estimate { done_at: Some(10_600), guessed: true, ..Default::default() });
        let working = t(&model, 2);
        assert_eq!(model.quiet_for(&working), Some(25 * 60));
        assert_eq!(model.eta(&working), Some(Eta { at: 10_600, guessed: true }));
        assert!(model.is_live(&working));
        assert!(!model.failed(&working));
        model.apply(Update::Event(Event::TurnEnded { session: 2, outcome: TurnOutcome::Failed, usage: None }), T0);
        assert!(model.failed(&working));
        assert!(model.quiet_for(&working).is_none(), "just heard from");
    }

    #[test]
    fn every_status_has_one_word() {
        let words: Vec<&str> = [TicketStatus::Waiting, TicketStatus::Review, TicketStatus::Proposed, TicketStatus::Todo, TicketStatus::InProgress, TicketStatus::Done, TicketStatus::Cancelled].into_iter().map(status_word).collect();
        assert_eq!(words, ["내 차례", "검토", "제안", "할 일", "작업 중", "끝남", "취소"]);
        assert_eq!(Section::NotOnMain.label(), "수락했지만 아직 main에 안 합쳐짐");
        assert!(Section::NotOnMain.hint().is_some_and(|h| h.contains("main")) && Section::Review.hint().is_none());
        assert_eq!(Section::Permission.label(), "허락 요청");
    }

    #[test]
    fn context_shows_as_a_percentage() {
        assert_eq!(percent((63, 100)), Some(63));
        assert_eq!(percent((5, 0)), None);
    }

    #[test]
    fn a_memo_taken_back_leaves_the_conversation_and_the_count() {
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Working)]), T0);
        for text in ["a", "b", "a"] {
            model.apply(Update::Event(Event::MessageQueued { session: 1, text: text.into(), project: None }), T0);
        }
        assert_eq!(model.members[0].memos, 3);
        assert!(model.apply(Update::Event(Event::MemoWithdrawn { session: 1, text: "a".into() }), T0));
        assert_eq!(model.convo().unwrap().queue.texts(), ["b", "a"], "only the first with that text");
        assert_eq!(model.members[0].memos, 2);
        // Sent at once: none waits.
        model.apply(Update::Event(Event::QueueSent { session: 1, texts: vec!["b".into(), "a".into()] }), T0);
        assert!(model.convo().unwrap().queue.is_empty());
        assert_eq!(model.members[0].memos, 0);
    }

    #[test]
    fn the_screen_line_says_who_is_on_it() {
        use equipe_core::protocol::MemberRef;
        let m = |squad: &str, member: &str| MemberRef { squad: squad.into(), member: member.into() };
        let mut model = Model::new("acme");
        model.apply(hello(vec![session(1, "pm", "pm", SessionState::Ready)]), T0);
        assert_eq!(screen_line(&model.desktop, "acme"), None);
        let busy = DesktopState { holder: Some(m("acme", "frontend")), queue: vec![m("tax", "pm"), m("acme", "qa")], stopped: false };
        assert!(model.apply(Update::Event(Event::Desktop { desktop: busy }), T0));
        let line = screen_line(&model.desktop, "acme").unwrap();
        assert!(line.contains("frontend") && line.contains("tax/pm"), "{line}");
        assert!(!line.contains("acme"), "a member of this squad goes by name alone: {line}");
        let stopped = DesktopState { stopped: true, ..Default::default() };
        assert!(screen_line(&stopped, "acme").is_some());
    }

}
