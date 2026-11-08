//! One run of one task: a real `equipe daemon run` in a throwaway home, the
//! squad's members started over its socket the way a screen starts them, the
//! task's messages and tickets sent as the person sends them, and an
//! automatic person answering the cards members put up. What happened is
//! collected for the grader: the board, each ticket's record, what members
//! said, the daemon's events, and each member's Claude Code transcripts.
//!
//! The engine is the person's own installed CLI with its normal login:
//! `HOME` and every other variable are left as they are; only `EQUIPE_HOME`
//! points at the run's home. Nothing here reads, copies or passes on a
//! credential.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};
use std::time::{Duration, Instant};

use equipe_core::gateway::{self, Client, Commands};
use equipe_core::protocol::decide::{Answer, QuestionSet};
use equipe_core::protocol::{Access, Approval, Command, Decision, Engine, Event, SessionId, SessionSpec, Ticket, TicketDraft, TicketEntry, TicketId, TicketKind, Usage};
use equipe_core::quality::{self, Transcript};
use tokio::sync::mpsc::UnboundedReceiver;

/// The squad every run's members join.
pub const SQUAD: &str = "eval";

/// A build of equipe to run members with.
#[derive(Debug, Clone)]
pub struct Variant {
    /// `A` (the baseline) or `B` (the candidate).
    pub name: String,
    pub binary: PathBuf,
}

/// How members are run.
#[derive(Debug, Clone)]
pub struct Setup {
    /// `None`: each role's own default engine.
    pub engine: Option<Engine>,
    /// `None`: each role's own default model.
    pub model: Option<String>,
    /// Keep the run's home and Claude Code's transcript folders after it.
    pub keep: bool,
}

/// A run's root folder: short, under /tmp, so the daemon's socket path fits
/// and the daemon knows it for a temporary home (it leaves on its own if
/// this process dies).
pub fn root_for(tag: &str) -> PathBuf {
    let dir = PathBuf::from("/tmp").join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// What the automatic person does with members' questions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Questions {
    /// Pick the recommended option of each (else the first); a question
    /// without options gets "use your judgement".
    Recommended,
    /// Leave them open, as a person away from the screen would.
    Unanswered,
}

/// What the automatic person saw and did.
#[derive(Debug, Clone, Default)]
pub struct Person {
    pub approvals: Vec<Approval>,
    pub questions: Vec<QuestionSet>,
    pub team_changes: usize,
    pub secrets: usize,
    pub routines: usize,
    /// Each time a member put a ticket down for the person (`block_on_person`): the ticket and what it asked.
    pub blocks: Vec<(TicketId, String)>,
    /// Tickets put down more often than the person answers ([`ANSWERS_PER_TICKET`]): left waiting.
    pub left_waiting: Vec<TicketId>,
}

/// How often the automatic person answers one ticket's `block_on_person`
/// before it leaves the ticket waiting, as a person who has nothing more to give would.
pub const ANSWERS_PER_TICKET: usize = 2;

/// What the automatic person says to a ticket put down for them: it has
/// nothing more than the task gave, so the member goes on with what it has.
pub const NOTHING_MORE: &str = "I have nothing more to give you than the ticket and the project: use your judgement, go on, and say in your hand-in what you assumed.";

impl Person {
    /// What to do about `ticket` as it now is: `Some(note)` to resume it with,
    /// once for each time it is put down, up to [`ANSWERS_PER_TICKET`].
    pub fn on_ticket(&mut self, ticket: &Ticket) -> Option<&'static str> {
        if ticket.status != equipe_core::protocol::TicketStatus::Waiting || ticket.assignee.is_none() {
            return None;
        }
        let asked = ticket.note.clone().unwrap_or_default();
        if self.blocks.iter().any(|(id, a)| *id == ticket.id && *a == asked) || self.left_waiting.contains(&ticket.id) {
            return None;
        }
        if self.blocks.iter().filter(|(id, _)| *id == ticket.id).count() >= ANSWERS_PER_TICKET {
            self.left_waiting.push(ticket.id);
            return None;
        }
        self.blocks.push((ticket.id, asked));
        Some(NOTHING_MORE)
    }
}

/// What a run's members did, as the daemon told it.
#[derive(Debug, Default)]
pub struct State {
    pub tickets: BTreeMap<TicketId, Ticket>,
    /// Each member's session, by role.
    pub sessions: HashMap<String, SessionId>,
    /// Each session's replies, one per finished turn.
    pub turns: HashMap<SessionId, Vec<String>>,
    /// The turn under way's words so far, per session.
    pub current: HashMap<SessionId, String>,
    /// Each turn's usage, per session.
    pub usage: HashMap<SessionId, Vec<Usage>>,
    pub person: Person,
    /// Every event, with the seconds since the run began.
    pub events: Vec<(f64, Event)>,
    pub errors: Vec<String>,
}

impl State {
    pub fn session(&self, role: &str) -> Option<SessionId> {
        self.sessions.get(role).copied()
    }

    /// The tickets `role` worked or owns.
    pub fn tickets_of<'a>(&'a self, role: &'a str) -> impl Iterator<Item = &'a Ticket> + 'a {
        self.tickets.values().filter(move |t| t.assignee.as_deref().is_some_and(|a| a == role || a.starts_with(&format!("{role}-"))) || t.owner.as_deref() == Some(role))
    }

    /// Whether `role`'s work is over for the grader: handed in, done or
    /// cancelled, or waiting on the person after they had nothing more to give.
    pub fn settled(&self, role: &str) -> bool {
        use equipe_core::protocol::TicketStatus as S;
        self.tickets_of(role).any(|t| matches!(t.status, S::Review | S::Done | S::Cancelled) || (t.status == S::Waiting && self.person.left_waiting.contains(&t.id)))
    }

    /// The replies `role` finished, oldest first.
    pub fn replies(&self, role: &str) -> &[String] {
        self.session(role).and_then(|s| self.turns.get(&s)).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Every turn `role` finished.
    pub fn turns_of(&self, role: &str) -> usize {
        self.replies(role).len()
    }
}

/// The daemon of one run.
pub struct Daemon {
    child: Child,
    pub socket: PathBuf,
}

impl Daemon {
    /// `binary daemon run` with its home at `home`, and nothing else changed
    /// in its environment but the language equipe speaks, telemetry off, and
    /// the variant's own `equipe` first on `PATH` (as an installed equipe is),
    /// so a member that runs `equipe …` gets this variant, not whichever
    /// equipe this Mac has installed.
    pub fn start(binary: &Path, home: &Path, log: &Path) -> Self {
        let out = std::fs::File::create(log).unwrap();
        let bin = home.parent().unwrap_or(home).join("bin");
        let path = variant_path(binary, &bin, std::env::var_os("PATH").as_deref());
        let own = std::env::var_os("ZDOTDIR").filter(|d| !d.is_empty()).or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
        let zdot = variant_zdotdir(&bin, &home.parent().unwrap_or(home).join("zdot"), &own);
        // Run from the run's own folder: a member's command that loses its
        // way (a relative path, outside its sandbox) lands there, never in
        // the checkout the eval was started from.
        let child = Process::new(binary)
            .current_dir(home.parent().unwrap_or(home))
            .args(["daemon", "run"])
            .env("PATH", path)
            .env("ZDOTDIR", zdot)
            .env("EQUIPE_HOME", home)
            .env("EQUIPE_LANG", "en")
            .env("EQUIPE_TELEMETRY", "off")
            .env("EQUIPE_TRIAL_IDLE_SECS", "900")
            .stdin(Stdio::null())
            .stdout(out.try_clone().unwrap())
            .stderr(out)
            .spawn()
            .unwrap_or_else(|e| panic!("{}: {e}", binary.display()));
        let socket = gateway::socket_path(home);
        let started = Instant::now();
        while !socket.exists() && started.elapsed() < Duration::from_secs(60) {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(socket.exists(), "the daemon came up: see {}", log.display());
        Self { child, socket }
    }

    pub async fn stop(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(20), gateway::shutdown(&self.socket)).await;
        for _ in 0..100 {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Daemon {
    /// A run that panicked leaves no daemon behind.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `PATH` with `bin` first, where `bin/equipe` is made a link to `binary`.
pub fn variant_path(binary: &Path, bin: &Path, path: Option<&std::ffi::OsStr>) -> std::ffi::OsString {
    std::fs::create_dir_all(bin).unwrap();
    let link = bin.join("equipe");
    let _ = std::fs::remove_file(&link);
    let target = binary.canonicalize().unwrap_or_else(|_| binary.to_owned());
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let rest: Vec<PathBuf> = path.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    std::env::join_paths(std::iter::once(bin.to_owned()).chain(rest)).unwrap()
}

/// A zsh startup folder that reads the person's own files and then puts
/// `bin` first again. Codex runs a member's commands in a login shell
/// (`zsh -lc`), whose profile can put an installed equipe's folder before
/// the variant's: without this, a Codex member's `equipe …` ran this Mac's
/// installed equipe, not the variant under test.
/// `own` is where the person's own files are (their `ZDOTDIR`, else home).
pub fn variant_zdotdir(bin: &Path, dir: &Path, own: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    for name in [".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
        let own = own.join(name);
        let text = format!("[ -f '{}' ] && . '{}'\nexport PATH='{}':\"$PATH\"\n", own.display(), own.display(), bin.display());
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir.to_owned()
}

/// A run under way.
pub struct Run {
    pub root: PathBuf,
    pub tag: String,
    pub setup: Setup,
    daemon: Option<Daemon>,
    home: PathBuf,
    socket: PathBuf,
    commands: Commands,
    events: UnboundedReceiver<Event>,
    started: Instant,
    pub questions: Questions,
    pub state: State,
}

impl Run {
    /// The squad, made on disk with its first project as the onboarding
    /// makes it, and the daemon attached to.
    pub async fn start(variant: &Variant, root: &Path, tag: &str, projects: &[PathBuf], setup: Setup) -> Self {
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        // equipe set to English, as a person picks it: what the squad starts
        // with (its weekly check-in's words) doesn't follow this Mac's language.
        equipe_core::i18n::save(&home, Some(equipe_core::i18n::Lang::En)).unwrap();
        match projects.split_first() {
            Some((first, rest)) => {
                equipe_core::squads::create(&home, SQUAD, first).unwrap();
                for p in rest {
                    equipe_core::squads::add_project(&home, SQUAD, p).unwrap();
                }
            }
            None => equipe_core::squads::create_without_project(&home, SQUAD).unwrap(),
        }
        let daemon = Daemon::start(&variant.binary, &home, &root.join("daemon.log"));
        let mut attached = None;
        for _ in 0..50 {
            match Client::connect(&daemon.socket, "quality-gate", SQUAD).await {
                Ok(c) => {
                    attached = Some(c);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
        let (client, _hello) = attached.expect("attached to the daemon");
        let (commands, events) = client.split();
        // Nobody reads the next-message guesses a screen shows the person.
        commands.send(Command::Suggestions { squad: SQUAD.into(), on: false });
        let socket = daemon.socket.clone();
        Self { root: root.to_owned(), tag: tag.to_owned(), setup, daemon: Some(daemon), home, socket, commands, events, started: Instant::now(), questions: Questions::Recommended, state: State::default() }
    }

    pub fn send(&self, command: Command) {
        self.commands.send(command);
    }

    /// Bring a member onto the squad as a screen's `/add` does, and wait until it has started.
    pub async fn add(&mut self, role: &str, cwd: &Path) -> SessionId {
        let (engine, model) = (self.setup.engine, self.setup.model.clone());
        self.add_on(role, cwd, engine, model).await
    }

    /// The same, on `engine` and `model` whatever the run's setup says: a
    /// squad that starts where the task needs it.
    pub async fn add_on(&mut self, role: &str, cwd: &Path, engine: Option<Engine>, model: Option<String>) -> SessionId {
        let id = equipe_core::protocol::new_session_id();
        let edits = equipe_core::roles::role_or_custom(role).edits;
        let spec = SessionSpec {
            engine,
            model,
            access: if edits { Access::EditFiles } else { Access::ReadOnly },
            cwd: cwd.to_owned(),
            role: Some(role.to_owned()),
            squad: Some(SQUAD.into()),
        };
        self.send(Command::Start { session: id, spec });
        let ok = self.wait_for(Duration::from_secs(120), |s| s.sessions.values().any(|v| *v == id)).await;
        assert!(ok, "{role} started; errors: {:?}", self.state.errors);
        // A second member of a role is known by its own name (`mobile-2`), from its start.
        self.state.sessions.entry(role.to_owned()).or_insert(id);
        id
    }

    /// The person's message to `role`.
    pub fn say(&mut self, role: &str, text: &str) {
        let session = self.state.session(role).unwrap_or_else(|| panic!("no {role} on the squad"));
        self.state.current.remove(&session);
        self.send(Command::Submit { session, text: text.to_owned(), about: None });
    }

    /// A ticket of the person's, for `owner`.
    pub fn ticket(&self, owner: &str, project: Option<&Path>, kind: TicketKind, title: &str, body: &str) {
        let draft = TicketDraft { title: title.into(), body: body.into(), project: project.map(Path::to_owned), assignee: Some(owner.into()), kind: Some(kind), ..Default::default() };
        self.send(Command::CreateTicket { squad: SQUAD.into(), draft });
    }

    /// Start `role`'s next conversation afresh, carrying a summary of the last.
    pub fn refresh(&self, role: &str) {
        let session = self.state.session(role).unwrap();
        self.send(Command::Refresh { session, summary: true });
    }

    /// Follow the daemon until `done` holds, answering what members put to
    /// the person meanwhile; `false` after `timeout`.
    pub async fn wait_for(&mut self, timeout: Duration, done: impl Fn(&State) -> bool) -> bool {
        let wait = async {
            while !done(&self.state) {
                match self.events.recv().await {
                    Some(event) => self.on_event(event),
                    None => return false,
                }
            }
            true
        };
        tokio::time::timeout(timeout, wait).await.unwrap_or(false)
    }

    /// Follow the daemon until `done` holds with nobody mid-turn and nothing
    /// waiting for the person, answering what members put to the person
    /// meanwhile — or until nobody has been at work for two minutes (the
    /// members are done with what they had: whether `done` holds then is the
    /// answer). `false` after `timeout`.
    pub async fn settle(&mut self, timeout: Duration, done: impl Fn(&State) -> bool) -> bool {
        let end = Instant::now() + timeout;
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let mut idle_since: Option<Instant> = None;
        loop {
            tokio::select! {
                event = self.events.recv() => match event {
                    Some(event) => self.on_event(event),
                    None => {
                        self.state.errors.push("the daemon closed the connection".into());
                        return false;
                    }
                },
                _ = tick.tick() => {
                    if Instant::now() > end {
                        return false;
                    }
                    let busy = gateway::busy(&self.socket).await.ok().flatten();
                    let at_rest = busy.as_ref().is_some_and(|b| b.working.is_empty() && b.approvals == 0);
                    if at_rest && done(&self.state) {
                        return true;
                    }
                    if !at_rest {
                        idle_since = None;
                    } else if idle_since.get_or_insert_with(Instant::now).elapsed() > Duration::from_secs(120) {
                        return done(&self.state);
                    }
                }
            }
        }
    }

    fn on_event(&mut self, event: Event) {
        let s = &mut self.state;
        match &event {
            Event::SessionStarted { session, member: Some(m), .. } => {
                // A member's first session is the one it is known by; a
                // conversation it opens beside it later (a lane) isn't.
                s.sessions.entry(m.role.clone()).or_insert(*session);
                s.sessions.entry(m.name.to_string()).or_insert(*session);
            }
            Event::TicketChanged { ticket, .. } => {
                s.tickets.insert(ticket.id, ticket.clone());
                if let Some(note) = s.person.on_ticket(ticket) {
                    self.commands.send(Command::Resume { squad: SQUAD.into(), ticket: ticket.id, note: note.into() });
                }
            }
            Event::TextDelta { session, text } => s.current.entry(*session).or_default().push_str(text),
            Event::TurnEnded { session, usage, .. } => {
                let said = s.current.remove(session).unwrap_or_default();
                s.turns.entry(*session).or_default().push(said);
                if let Some(u) = usage {
                    s.usage.entry(*session).or_default().push(*u);
                }
            }
            Event::Error { message, .. } => s.errors.push(message.clone()),
            Event::ApprovalRequested { approval, .. } => {
                s.person.approvals.push(approval.clone());
                self.commands.send(Command::Decide { squad: SQUAD.into(), approval: approval.id, decision: Decision::Allow, words: None, said: None });
            }
            Event::QuestionsAsked { questions, .. } => {
                s.person.questions.push(questions.clone());
                if self.questions == Questions::Recommended {
                    let answers = questions
                        .questions
                        .iter()
                        .map(|q| {
                            if q.options.is_empty() {
                                Answer { picked: vec![], other: Some("No preference — use your judgement.".into()), reaction: None }
                            } else {
                                let pick = q.options.iter().position(|o| o.shown().1).unwrap_or(0);
                                Answer { picked: vec![pick], other: None, reaction: None }
                            }
                        })
                        .collect();
                    self.commands.send(Command::AnswerQuestions { squad: SQUAD.into(), id: questions.id, answers, about: None });
                }
            }
            Event::TeamChangeProposed { change, .. } => {
                s.person.team_changes += 1;
                self.commands.send(Command::DeclineTeamChange { squad: SQUAD.into(), id: change.id });
            }
            Event::SecretAsked { ask, .. } => {
                s.person.secrets += 1;
                self.commands.send(Command::DismissSecret { squad: SQUAD.into(), id: ask.id });
            }
            Event::RoutineProposed { proposal, .. } => {
                s.person.routines += 1;
                self.commands.send(Command::DeclineRoutine { squad: SQUAD.into(), id: proposal.id });
            }
            _ => {}
        }
        let at = self.started.elapsed().as_secs_f64();
        self.state.events.push((at, event));
    }

    /// Everything the run left, collected, and the daemon stopped.
    pub async fn finish(mut self, out: &Path) -> Outcome {
        // What arrived after the last wait.
        while let Ok(event) = self.events.try_recv() {
            self.on_event(event);
        }
        let mut histories = BTreeMap::new();
        if let Ok((client, hello)) = Client::connect(&self.socket, "quality-gate", SQUAD).await {
            for t in hello.snapshot.tickets {
                self.state.tickets.insert(t.id, t);
            }
            let (to, mut events) = client.split();
            for id in self.state.tickets.keys() {
                to.send(Command::TicketHistory { squad: SQUAD.into(), ticket: *id });
            }
            let wanted = self.state.tickets.len();
            let _ = tokio::time::timeout(Duration::from_secs(15), async {
                while histories.len() < wanted {
                    match events.recv().await {
                        Some(Event::TicketHistory { ticket, entries, .. }) => {
                            histories.insert(ticket, entries);
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
            })
            .await;
        }
        let wall = self.started.elapsed();
        if let Some(daemon) = self.daemon.take() {
            daemon.stop().await;
        }
        let mut transcripts = collect_transcripts(&self.tag, &out.join("transcripts"), self.setup.keep);
        // Who each engine session was, as the journal recorded it: surer than a path.
        let roles = engine_roles(&self.home);
        for t in &mut transcripts {
            let parent = if t.sidechain { t.path.parent().and_then(Path::parent).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()) } else { None };
            if let Some(role) = roles.get(&t.session).or_else(|| parent.as_ref().and_then(|p| roles.get(p))) {
                t.role = Some(role.clone());
            }
        }
        std::fs::create_dir_all(out).unwrap();
        let events: String = self.state.events.iter().map(|(at, e)| format!("{{\"at\":{at:.1},\"event\":{}}}\n", serde_json::to_string(e).unwrap_or_default())).collect();
        std::fs::write(out.join("events.jsonl"), events).unwrap();
        let _ = std::fs::copy(self.root.join("daemon.log"), out.join("daemon.log"));
        Outcome { state: self.state, histories, transcripts, wall, root: self.root, home_dir: self.home }
    }
}

/// Each engine session the squad's journal knows, by the role that ran it:
/// members' sessions, their tickets' and the PM's conversations.
fn engine_roles(home: &Path) -> HashMap<String, String> {
    let mut roles = HashMap::new();
    let Ok(journal) = equipe_core::journal::Journal::for_squad(home, SQUAD) else { return roles };
    for s in journal.open_sessions().unwrap_or_default() {
        if let (Some(engine), Some(member)) = (s.engine_session, s.member) {
            roles.insert(engine, member.role.clone());
        }
    }
    for t in journal.tickets().unwrap_or_default() {
        let role = t.assignee.as_deref().map(|a| a.trim_end_matches(|c: char| c.is_ascii_digit() || c == '-').to_owned());
        if let (Some(role), Ok(work)) = (role, journal.ticket_work(t.id))
            && let Some(engine) = work.engine_session
        {
            roles.insert(engine, role);
        }
    }
    for c in journal.conversations().unwrap_or_default() {
        if let Some(engine) = c.engine_session {
            roles.insert(engine, "pm".into());
        }
    }
    roles
}

/// What a finished run left, for its grader.
pub struct Outcome {
    pub state: State,
    pub histories: BTreeMap<TicketId, Vec<TicketEntry>>,
    /// Members' Claude Code transcripts, subagents' included.
    pub transcripts: Vec<Transcript>,
    pub wall: Duration,
    pub root: PathBuf,
    pub home_dir: PathBuf,
}

impl Outcome {
    /// The transcripts of `role`'s sessions: the ones it ran on its tickets
    /// and at its desk, by what equipe's paths in them say.
    pub fn transcripts_of(&self, role: &str) -> Vec<&Transcript> {
        self.transcripts.iter().filter(|t| t.role.as_deref() == Some(role)).collect()
    }

    /// Every word `role` wrote: its replies, its tickets and comments, its hand-in summaries.
    pub fn words_of(&self, role: &str) -> String {
        let mut all: Vec<String> = self.state.replies(role).to_vec();
        for t in self.transcripts_of(role) {
            for (name, input) in t.tools() {
                if name.starts_with("mcp__") {
                    all.push(input.to_string());
                }
            }
        }
        for entries in self.histories.values() {
            all.extend(entries.iter().filter(|e| e.by == role || e.by.starts_with(&format!("{role}-"))).map(|e| e.text.clone()));
        }
        all.join("\n")
    }

    /// Where a ticket's work is: its worktree while there, else the squad's
    /// folder (a task's outputs land there once accepted).
    pub fn work_dir(&self, ticket: &Ticket) -> Option<PathBuf> {
        ticket.worktree.clone().filter(|w| w.exists())
    }

    pub fn squad_home(&self) -> PathBuf {
        equipe_core::squad_home::path(&self.home_dir, SQUAD)
    }

    /// Whether the ticket's record shows it handed in by its member.
    pub fn handed_in(&self, id: TicketId) -> bool {
        self.histories.get(&id).is_some_and(|h| h.iter().any(|e| e.kind == "handed in"))
            || self.state.tickets.get(&id).is_some_and(|t| matches!(t.status, equipe_core::protocol::TicketStatus::Review | equipe_core::protocol::TicketStatus::Done))
    }

    /// Tokens spent, by role — from the transcripts, subagents' with their member's.
    pub fn tokens_by_role(&self) -> BTreeMap<String, quality::Tokens> {
        let mut by: BTreeMap<String, quality::Tokens> = BTreeMap::new();
        for t in &self.transcripts {
            by.entry(t.role.clone().unwrap_or_else(|| "other".into())).or_default().add(t.tokens);
        }
        by
    }

    /// The cost Claude Code put on the run's sessions, at API prices.
    pub fn cost_usd(&self) -> f64 {
        self.transcripts.iter().filter_map(|t| t.cost_usd).sum()
    }
}

/// Claude Code's transcript folders of this run's sessions — those whose
/// working directory was under the run's root — read, copied to `keep_in`,
/// and (unless `keep`) removed, since they are the run's own.
fn collect_transcripts(tag: &str, keep_in: &Path, keep: bool) -> Vec<Transcript> {
    let Some(projects) = quality::claude_projects() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&projects) else { return Vec::new() };
    let marker = format!("-{tag}-");
    let mut found = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !(name.contains(&marker) || name.ends_with(&format!("-{tag}"))) {
            continue;
        }
        let dir = e.path();
        for path in quality::transcripts_under(&dir) {
            let Ok(mut t) = Transcript::read(&path) else { continue };
            let rel = path.strip_prefix(&projects).unwrap_or(&path);
            let copy = keep_in.join(rel);
            let _ = std::fs::create_dir_all(copy.parent().unwrap());
            let _ = std::fs::copy(&path, &copy);
            // A subagent's transcript plays its parent's role.
            if t.role.is_none() && t.sidechain {
                let parent = path.parent().and_then(Path::parent).map(|p| p.with_extension("jsonl"));
                if let Some(Ok(p)) = parent.as_deref().map(Transcript::read) {
                    t.role = p.role;
                }
            }
            found.push(t);
        }
        if !keep {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    found
}

/// Run `program args` in `dir`: whether it succeeded, and what it printed.
pub fn run_in(dir: &Path, program: &str, args: &[&str], env: &[(&str, &str)]) -> (bool, String) {
    let mut c = Process::new(program);
    c.args(args).current_dir(dir).stdin(Stdio::null());
    for (k, v) in env {
        c.env(k, v);
    }
    match c.output() {
        Ok(o) => (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))),
        Err(e) => (false, e.to_string()),
    }
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let (ok, out) = run_in(dir, "git", args, &[]);
    assert!(ok, "git {args:?} in {}: {out}", dir.display());
    out.trim().to_owned()
}

/// A fresh repository at `dir` with `files`, committed on `main`: its commit.
pub fn repo(dir: &Path, files: &[(&str, &str)]) -> String {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Quality Gate"]);
    git(dir, &["config", "user.email", "quality-gate@example.com"]);
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
    }
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "Start"]);
    git(dir, &["rev-parse", "HEAD"])
}

/// The files a worktree changed since `base`: committed or not, new ones too.
/// Bytecode Python leaves when the tests run (`__pycache__/`, `*.pyc`) is the
/// grader's own trace, not the member's work — left out, whatever the
/// machine's global gitignore says.
pub fn changed_files(worktree: &Path, base: &str) -> Vec<String> {
    let mut files: Vec<String> = git(worktree, &["diff", "--name-only", base]).lines().map(str::to_owned).collect();
    files.extend(git(worktree, &["ls-files", "--others", "--exclude-standard"]).lines().map(str::to_owned));
    files.sort();
    files.dedup();
    files.retain(|f| !f.is_empty() && !f.ends_with(".pyc") && !f.split('/').any(|part| part == "__pycache__"));
    files
}

/// A ticket's work checked out on its own, when its worktree is gone: its branch, in a new worktree.
pub fn checkout(repo: &Path, branch: &str, at: &Path) -> Option<PathBuf> {
    let (ok, _) = run_in(repo, "git", &["worktree", "add", "-q", "--detach", &at.to_string_lossy(), branch], &[]);
    ok.then(|| at.to_owned())
}
