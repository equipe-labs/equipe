//! One member's conversation as the window holds it: the transcript the daemon
//! sends (folded with the same `Transcript` every screen uses), what is waiting
//! behind it, and the blocks the conversation is drawn as. No UI types here, so
//! the rules — which thread shows, how tool steps gather and fold — can be tested
//! without a window. Behaviour follows the terminal console (`tui/app.rs`,
//! `tui/view.rs`).

use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use equipe_core::protocol::transcript::scope;
use equipe_core::protocol::{Entry, Event, SessionInfo, TicketId, ToolOutcome, Transcript, TurnOutcome};
use equipe_core::t;

use crate::activity::News;
use crate::state::Clock;

/// How many targets a merged tool line names before it counts the rest.
pub const TARGETS_SHOWN: usize = 3;

/// A subagent the member handed work to, at it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sub {
    pub id: String,
    /// What it was handed: the delegation's task, its first 80 characters.
    pub what: String,
    /// Its latest step, once it has taken one.
    pub step: Option<String>,
    /// Since when it works (attached mid-way: as long ago as the daemon said).
    pub since: Instant,
}

impl Sub {
    /// Whether `target` — a delegation step's task — is what this one was handed.
    pub fn handed(&self, target: &str) -> bool {
        let first: String = target.chars().take(80).collect();
        first == self.what || (!target.is_empty() && self.what.starts_with(target))
    }
}

#[derive(Debug, Clone)]
pub struct Convo {
    pub transcript: Transcript,
    /// Shown as the work on this ticket only — the ticket open on screen.
    pub showing: Option<TicketId>,
    /// What waits for it while it works: steers not read yet and memos for
    /// when its turn ends, in order, each with its project.
    pub queue: crate::queue::Queue,
    /// The greyed guess at what the person says next, while it rests.
    pub suggestion: Option<String>,
    /// Subagents it handed work to this turn and that haven't handed back.
    pub subagents: Vec<Sub>,
    /// Where it works now; tool paths are shown relative to it.
    pub base: PathBuf,
    desk: PathBuf,
    pub is_pm: bool,
    /// A lane: one project's conversation beside its member's others. Like a
    /// ticket's, it is one conversation, so every memo is for it.
    pub lane: bool,
    working_since: Option<Instant>,
    /// The model reasoning right now: since when, and what it has shown.
    thinking: Option<(Instant, String)>,
    /// Reasoning that ended, by the number its line in the transcript carries.
    thoughts: Vec<String>,
    /// Since when the call under way has run: its line counts the seconds.
    step_since: Option<Instant>,
}

/// The transcript line where reasoning ended: `THOUGHT` + its number + `:` +
/// seconds. A private-use character, so no other note starts so.
pub const THOUGHT: char = '\u{E000}';

/// The transcript line where a turn ended: `WORKED` + its seconds + `:` + the
/// time of day + `@` + the unix second — worded when drawn, so in the
/// language in use then.
pub const WORKED: char = '\u{E001}';

impl Convo {
    pub fn new(base: PathBuf, is_pm: bool) -> Self {
        Self {
            transcript: Transcript::default(),
            showing: None,
            queue: crate::queue::Queue::default(),
            suggestion: None,
            subagents: Vec::new(),
            desk: base.clone(),
            base,
            is_pm,
            lane: false,
            working_since: None,
            thinking: None,
            thoughts: Vec::new(),
            step_since: None,
        }
    }

    pub fn from_info(info: &SessionInfo, now: Instant) -> Self {
        let work_base = info.work.as_ref().map(|w| w.workspace.as_ref().map_or_else(|| w.project.clone(), |ws| ws.path.clone()));
        let mut convo = Self::new(work_base.unwrap_or_else(|| info.project.clone()), info.member.as_ref().is_some_and(|m| m.role == "pm"));
        convo.desk = info.project.clone();
        convo.transcript = Transcript::from_entries(info.recent.clone());
        convo.queue = crate::queue::Queue::from_info(info);
        convo.subagents = info
            .subagents
            .iter()
            .map(|s| Sub { id: s.id.clone(), what: s.what.clone(), step: s.step.clone(), since: now.checked_sub(Duration::from_secs(s.secs)).unwrap_or(now) })
            .collect();
        // Attached mid-turn: the clock starts now, not when the turn did.
        if info.state == equipe_core::protocol::SessionState::Working {
            convo.working_since = Some(now);
        }
        convo
    }

    /// Fold one event of this session in. True when what the conversation
    /// shows may differ (the token counter's ticks don't count).
    pub fn on_event(&mut self, event: &Event, now: Instant, at: Clock) -> bool {
        if matches!(event, Event::Output { .. } | Event::ModelKnown { .. }) {
            return false;
        }
        if let Event::Thinking { text, .. } = event {
            self.thinking.get_or_insert_with(|| (now, String::new())).1.push_str(text);
            return true;
        }
        // Anything else ends the reasoning: it stays as a line where it was.
        if matches!(event, Event::TextDelta { .. } | Event::ToolUse { .. } | Event::TurnEnded { .. } | Event::Error { .. }) {
            self.end_thinking(now);
        }
        self.transcript.apply(event);
        match event {
            Event::ToolUse { .. } => self.step_since = Some(now),
            Event::ToolResult { .. } | Event::TextDelta { .. } | Event::TurnEnded { .. } => self.step_since = None,
            _ => {}
        }
        match event {
            Event::SessionStarted { project, .. } => {
                self.base = project.clone();
                self.desk = project.clone();
            }
            Event::UserMessage { .. } => {
                self.working_since.get_or_insert(now);
                self.suggestion = None;
                // The memos of one project are delivered together, into its
                // conversation; another project's wait for the next turn's end.
                self.queue.delivered();
            }
            Event::SteerQueued { text, project, .. } => self.queue.steer(text, project.clone(), crate::state::now_unix()),
            Event::Steered { text, .. } => {
                self.queue.read(text);
            }
            Event::MemoWithdrawn { text, .. } => {
                self.queue.withdraw(text);
            }
            Event::QueueSent { texts, .. } => self.queue.sent(texts),
            Event::Suggestion { text, .. } => {
                // Too late once it is at work again, or a memo is on its way.
                if self.working_since.is_none() && !self.queue.has_memo() {
                    self.suggestion = Some(text.clone());
                }
            }
            Event::MessageQueued { text, project, .. } => {
                self.suggestion = None;
                self.queue.memo(text, project.clone(), crate::state::now_unix());
            }
            Event::WorkStarted { project, workspace, .. } => {
                self.suggestion = None;
                self.base = workspace.as_ref().map_or_else(|| project.clone(), |ws| ws.path.clone());
                self.working_since.get_or_insert(now);
            }
            Event::WorkEnded { .. } => {
                self.base = self.desk.clone();
            }
            // Told twice (attached as it started): it is the one already here.
            Event::SubagentStarted { id, .. } if self.subagents.iter().any(|s| s.id == *id) => {}
            Event::SubagentStarted { id, what, .. } => self.subagents.push(Sub { id: id.clone(), what: what.clone(), step: None, since: now }),
            Event::SubagentStep { id, step, .. } => {
                if let Some(sub) = self.subagents.iter_mut().find(|s| s.id == *id) {
                    sub.step = Some(step.clone());
                }
            }
            Event::SubagentEnded { id, .. } => self.subagents.retain(|s| s.id != *id),
            Event::TurnEnded { outcome, .. } => {
                self.subagents.clear();
                // Unread, it comes back as a memo, delivered now.
                self.queue.turn_ended();
                self.stop_working(now, at, *outcome);
            }
            _ => {}
        }
        true
    }

    /// The reasoning in progress, finished: a line saying how long it took,
    /// which opens to what it was.
    fn end_thinking(&mut self, now: Instant) {
        let Some((since, text)) = self.thinking.take() else { return };
        let secs = now.saturating_duration_since(since).as_secs().max(1);
        self.thoughts.push(text);
        self.transcript.push_local(Entry::Note { text: format!("{THOUGHT}{}:{secs}", self.thoughts.len() - 1) });
    }

    /// Since when the latest call has run, while it does.
    /// How long the turn has run, while it does.
    pub fn working_for(&self) -> Option<std::time::Duration> {
        self.working_since.map(|since| since.elapsed())
    }

    pub fn step_since(&self) -> Option<Instant> {
        self.step_since.filter(|_| self.working_since.is_some())
    }

    /// What the reasoning numbered `n` was.
    pub fn thought(&self, n: usize) -> Option<&str> {
        self.thoughts.get(n).map(String::as_str)
    }

    /// The conversation its turns go into now (`None`: the whole squad's).
    pub fn current_thread(&self) -> Option<&str> {
        self.transcript.entries().iter().rev().find_map(|e| match e {
            Entry::Thread { project } => Some(project.as_deref()),
            _ => None,
        })?
    }

    /// Whether its conversation holds work on `ticket`, to show.
    pub fn has_work_on(&self, ticket: TicketId) -> bool {
        let header = format!("── T-{ticket} ");
        self.transcript.entries().iter().any(|e| matches!(e, Entry::Note { text } if text.starts_with(&header)))
    }

    /// What the person said last in it, its first line.
    pub fn last_said(&self) -> Option<String> {
        self.transcript.entries().iter().rev().find_map(|e| match e {
            Entry::User { text } => Some(crate::sent::summary(text).lines().next().unwrap_or_default().trim().to_owned()),
            _ => None,
        })
    }

    /// A lane of this member that goes back into it: what was said there stays,
    /// in this member's conversation about `project`.
    pub fn take_back(&mut self, project: Option<String>, lane: &Convo) {
        let back = self.current_thread().map(str::to_owned);
        self.transcript.push_local(Entry::Thread { project });
        for entry in lane.transcript.entries() {
            self.transcript.push_local(entry.clone());
        }
        self.transcript.push_local(Entry::Thread { project: back });
    }

    /// Start a lane's conversation with what the member said about its project.
    pub fn open_lane(&mut self, main: &Convo) {
        self.transcript = Transcript::from_entries(main.shown().into_iter().map(Cow::into_owned).collect());
        self.lane = true;
    }

    /// Its turn ended: leave a quiet line saying how long it took.
    fn stop_working(&mut self, now: Instant, at: Clock, _outcome: TurnOutcome) {
        if let Some(since) = self.working_since.take() {
            let took = now.saturating_duration_since(since);
            if took >= Duration::from_secs(1) {
                self.transcript.push_local(Entry::Note { text: format!("{WORKED}{}:{}@{}", took.as_secs(), at.hm(), crate::state::now_unix()) });
            }
        }
    }

    /// The conversation as it shows. A ticket open (`showing`): only its
    /// work. Otherwise everything — but the PM's own tickets' work, which each
    /// shows as its ticket. Never the markers.
    pub fn shown(&self) -> Vec<Cow<'_, Entry>> {
        self.shown_about().into_iter().map(|(entry, _)| entry).collect()
    }

    /// The same, each line with the project it is about (`None`: the whole
    /// squad; see `transcript::scope`).
    pub fn shown_about(&self) -> Vec<(Cow<'_, Entry>, Option<String>)> {
        let entries = self.transcript.entries();
        let mut about = scope::of(entries).into_iter();
        let mut out = Vec::new();
        // The ticket whose work the lines are, while they are.
        let mut on: Option<TicketId> = None;
        for entry in entries {
            let here = about.next().flatten();
            match entry {
                Entry::Thread { .. } => {
                    on = None;
                    continue;
                }
                Entry::Note { text } if text.starts_with("── T-") => {
                    on = text["── T-".len()..].split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse().ok());
                }
                _ => {}
            }
            let keep = match self.showing {
                Some(ticket) => on == Some(ticket),
                None => on.is_none() || !self.is_pm,
            };
            if keep {
                out.push((Cow::Borrowed(entry), here));
            }
        }
        out
    }

    /// The blocks the conversation is drawn as: one per message, reply, run of
    /// steps, note — then what waits for it, as the person said it.
    pub fn blocks(&self) -> Vec<Block> {
        let mut blocks = build_blocks(&self.shown(), &self.base);
        self.finish(&mut blocks, true);
        blocks.extend(self.queue.items().iter().map(Block::pending));
        blocks
    }

    /// The thoughts' words in place; while it works, the reply streaming and
    /// the steps not answered yet — when the end of the conversation is in
    /// sight (`end_shown`) — and the reasoning going on.
    fn finish(&self, blocks: &mut Vec<Block>, end_shown: bool) {
        for block in blocks.iter_mut() {
            if let Block::Thought { n, text, .. } = block {
                *text = self.thought(*n).unwrap_or_default().to_owned();
            }
        }
        self.mark_delegations(blocks);
        // A command left running in the background runs on while the turn does,
        // wherever its line is.
        if self.working_since.is_some() {
            for block in blocks.iter_mut() {
                if let Block::Steps(lines) = block {
                    for line in lines.iter_mut().filter(|l| l.background && l.status == Status::Unknown) {
                        line.status = Status::Running;
                    }
                }
            }
        }
        if !end_shown {
            return;
        }
        if self.working_since.is_some() {
            let thinking = self.thinking.is_some();
            match blocks.last_mut() {
                Some(Block::Reply { live, .. }) if !thinking => *live = true,
                Some(Block::Steps(lines)) => {
                    for line in lines.iter_mut().filter(|l| l.status == Status::Unknown) {
                        line.status = Status::Running;
                    }
                }
                _ => {}
            }
        }
        if let Some((_, text)) = &self.thinking {
            blocks.push(Block::Thinking(text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default().to_owned()));
        }
    }

    /// A delegation whose subagent is still at it runs, wherever it is — one
    /// sent to the background answered at once, but its work goes on. The
    /// latest delegation of each task is its subagent's.
    fn mark_delegations(&self, blocks: &mut [Block]) {
        for sub in &self.subagents {
            if let Some((at, line)) = delegation_of(blocks, sub)
                && let Block::Steps(lines) = &mut blocks[at]
            {
                lines[line].status = Status::Running;
            }
        }
    }

    /// Whether the conversation can be shown a project at a time: the PM's
    /// (or its lane's), not a ticket's work.
    pub fn sliceable(&self) -> bool {
        self.is_pm && self.showing.is_none()
    }

    /// The conversation as the person looks at it with `slice` in focus
    /// (`None`: every project). The PM's shows that project's lines and what
    /// was said to the whole squad — another project's lines are not there
    /// at all, folded or pointed at. Every project's: the lines all, each
    /// block with the project it is about. Any other conversation shows whole.
    pub fn sliced(&self, slice: Option<&str>) -> Sliced {
        if !self.sliceable() {
            let blocks = self.blocks();
            return Sliced { about: vec![None; blocks.len()], blocks };
        }
        let lines = self.shown_about();
        let here = |about: &Option<String>| slice.is_none_or(|project| about.as_deref().is_none_or(|a| a == project));
        let mut out = Sliced::default();
        // A block per run of lines about one project, so each says which.
        let mut at = 0;
        while at < lines.len() {
            let about = lines[at].1.clone();
            let end = (at..lines.len()).find(|&i| lines[i].1 != about).unwrap_or(lines.len());
            if here(&about) {
                let run: Vec<Cow<'_, Entry>> = lines[at..end].iter().map(|(e, _)| Cow::Borrowed(e.as_ref())).collect();
                let blocks = build_blocks(&run, &self.base);
                out.about.extend(std::iter::repeat_n(about, blocks.len()));
                out.blocks.extend(blocks);
            }
            at = end;
        }
        // Only the first reply in sight names who speaks.
        let mut named = false;
        for block in &mut out.blocks {
            if let Block::Reply { first, .. } = block {
                *first = !named;
                named = true;
            }
        }
        let end_shown = lines.last().is_none_or(|(_, about)| here(about));
        let before = out.blocks.len();
        self.finish(&mut out.blocks, end_shown);
        out.about.extend(std::iter::repeat_n(slice.map(str::to_owned), out.blocks.len() - before));
        // What waits shows below the conversation it goes into, as sent messages do.
        for item in self.queue.items().iter().filter(|item| here(&item.project)) {
            out.blocks.push(Block::pending(item));
            out.about.push(item.project.clone());
        }
        out
    }
}

/// Where `sub` was handed its work in `blocks`: the block, and the line in
/// it — the latest delegation of that task.
pub fn delegation_of(blocks: &[Block], sub: &Sub) -> Option<(usize, usize)> {
    blocks.iter().enumerate().rev().find_map(|(at, block)| match block {
        Block::Steps(lines) => lines.iter().rposition(|l| l.verb == "delegate" && l.targets.first().is_some_and(|t| sub.handed(t))).map(|line| (at, line)),
        _ => None,
    })
}

/// The conversation as a project's slice shows it ([`Convo::sliced`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sliced {
    pub blocks: Vec<Block>,
    /// What each block is about (`None`: the whole squad, or not said).
    pub about: Vec<Option<String>>,
}

/// `1m 05s`, `42s`, `1h 02m` — how long something has been going.
pub fn elapsed(took: Duration) -> String {
    let s = took.as_secs();
    match s {
        0..=59 => t!("time.secs", s = s),
        60..=3599 => t!("time.mins_secs", m = s / 60, s = format!("{:02}", s % 60)),
        _ => t!("time.hours_mins", h = s / 3600, m = format!("{:02}", (s % 3600) / 60)),
    }
}

/// Where a step stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Not answered, and the member at work.
    Running,
    Done,
    Failed,
    /// Nothing said (an older daemon, or a turn that ended without).
    Unknown,
}

/// One line of the conversation's tool activity. Consecutive reads, searches
/// and fetches merge into one line, so looking around reads as one step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLine {
    pub verb: String,
    pub targets: Vec<String>,
    pub edits: bool,
    pub status: Status,
    /// How long it took (all of a merged line's steps together).
    pub ms: Option<u64>,
    pub exit: Option<i32>,
    /// What a command printed: its last lines, and how many came before.
    pub output: Option<(String, u32)>,
    /// An edit's change, as a unified diff.
    pub diff: Option<String>,
    /// The file it acts on, whole, for its chip.
    pub path: Option<String>,
    /// What it acted on, whole: a command's every line, a comment's words.
    pub full: Option<String>,
    /// What it does in the agent's words (a command's description): shown
    /// over what it acted on, which shows when it is opened.
    pub label: Option<String>,
    /// A command left running on in the background.
    pub background: bool,
}

impl ToolLine {
    pub fn new(verb: impl Into<String>, targets: Vec<String>, edits: bool) -> Self {
        Self { verb: verb.into(), targets, edits, status: Status::Unknown, ms: None, exit: None, output: None, diff: None, path: None, full: None, label: None, background: false }
    }

    /// `read  a.rs, b.rs, c.rs, +2` — what the line says after its mark.
    pub fn text(&self) -> String {
        if let Some(label) = self.label.as_ref().filter(|_| self.targets.len() <= 1) {
            return label.clone();
        }
        let mut named: Vec<String> = self.targets.iter().take(TARGETS_SHOWN).cloned().collect();
        if self.targets.len() > TARGETS_SHOWN {
            named.push(format!("+{}", self.targets.len() - TARGETS_SHOWN));
        }
        named.join(", ")
    }

    /// Fold another step's outcome into this line's.
    fn absorb(&mut self, outcome: Option<&ToolOutcome>) {
        let status = match outcome {
            Some(o) if o.ok => Status::Done,
            Some(_) => Status::Failed,
            None => Status::Unknown,
        };
        self.status = match (self.status, status) {
            (Status::Failed, _) | (_, Status::Failed) => Status::Failed,
            (Status::Unknown, _) | (_, Status::Unknown) => Status::Unknown,
            _ => Status::Done,
        };
        if let Some(ms) = outcome.and_then(|o| o.ms) {
            self.ms = Some(self.ms.unwrap_or(0) + ms);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    User(String),
    /// A reply; `first` when it is the conversation's first (its speaker
    /// named over it); `live` while it is still streaming; `worked`, when it
    /// ended its turn: how long the turn took, and when.
    Reply { text: String, first: bool, live: bool, worked: Option<Worked> },
    /// A group of tool calls one after another (a failed one alone).
    Steps(Vec<ToolLine>),
    /// equipe's news, one after another.
    News(Vec<News>),
    /// A ticket put down for the person: the card they answer it on
    /// (`crate::yourturn`). `news` is the line it came as; `live`, whether it
    /// is the ticket's latest in this conversation (an earlier one is past);
    /// `look`, what the card shows now, so a change draws it again.
    YourTurn { ticket: u32, news: String, live: bool, look: u64 },
    Note(String),
    /// A turn's end with no reply to end it.
    Worked(Worked),
    /// Reasoning that ended: how long it took, and what it was.
    Thought { n: usize, secs: u64, text: String },
    /// Reasoning going on now: its latest line.
    Thinking(String),
    Error { message: String, next: Option<String> },
    Landed { branch: String, stat: String },
    /// Said while the member works, waiting: a steer (read at its next
    /// step, the work carrying on) or a memo (read when its turn ends); when
    /// it was said, in unix seconds (0: not known).
    Pending { text: String, steer: bool, at: u64 },
    /// A member handed the person its browser, asking `why`: a row to open
    /// it from, and give it back; `live` on the member's latest (an earlier
    /// one is past).
    HandOver { name: String, why: String, live: bool },
}

impl Block {
    fn pending(item: &crate::queue::Pending) -> Self {
        Block::Pending { text: item.text.clone(), steer: item.kind == crate::queue::Kind::Steer, at: item.at }
    }

    /// The words a search finds in it.
    pub fn searchable(&self) -> Option<String> {
        match self {
            // Your words as shown: without the attachments' tags.
            Block::User(t) | Block::Pending { text: t, .. } => Some(crate::sent::parts(t).text),
            Block::Reply { text: t, .. } | Block::Note(t) => Some(t.clone()),
            Block::News(lines) => Some(lines.iter().map(|n| n.text.clone()).collect::<Vec<_>>().join("\n")),
            Block::YourTurn { news, .. } => Some(news.clone()),
            Block::HandOver { name, why, .. } => Some(format!("{name}\n{why}")),
            Block::Worked(_) => None,
            Block::Steps(lines) => Some(lines.iter().map(|l| format!("{} {}", verb_word(&l.verb), l.targets.join(" "))).collect::<Vec<_>>().join("\n")),
            Block::Error { message, .. } => Some(message.clone()),
            Block::Landed { branch, stat } => Some(format!("{branch}\n{stat}")),
            Block::Thought { .. } | Block::Thinking(_) => None,
        }
    }
}

/// `n:secs` of a thought's transcript line.
fn thought_of(text: &str) -> Option<(usize, u64)> {
    let (n, secs) = text.strip_prefix(THOUGHT)?.split_once(':')?;
    Some((n.parse().ok()?, secs.parse().ok()?))
}

/// How long a turn worked, and when it ended: the time of day it was then,
/// and (kept by this app since it learned to) the moment, for "5분 전".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worked {
    pub secs: u64,
    pub at: String,
    pub unix: Option<u64>,
}

impl Worked {
    /// `1분 05초 작업함`.
    pub fn took(&self) -> String {
        t!("chat.worked_for", took = elapsed(Duration::from_secs(self.secs)))
    }
}

/// A turn's end, as its transcript line keeps it: `WORKED` secs `:` hh:mm,
/// then `@` unix seconds.
fn worked_of(text: &str) -> Option<Worked> {
    let rest = text.strip_prefix(WORKED)?;
    let (rest, unix) = match rest.rsplit_once('@') {
        Some((rest, unix)) => (rest, unix.parse().ok()),
        None => (rest, None),
    };
    let (secs, at) = rest.split_once(':')?;
    Some(Worked { secs: secs.parse().ok()?, at: at.to_owned(), unix })
}

/// Gather entries into blocks: tool use into groups (merging looking-around
/// lines, a failed call on its own), news into runs, a turn's end onto the
/// reply that ended it; everything else one to one.
pub fn build_blocks(entries: &[Cow<'_, Entry>], base: &Path) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut run: Vec<ToolLine> = Vec::new();
    let mut replied = false;
    for entry in entries {
        let entry = entry.as_ref();
        match entry {
            Entry::Thread { .. } => continue,
            // Claude Code loading tool descriptions: its business, not the person's.
            Entry::Tool { tool, .. } if tool == "ToolSearch" => continue,
            Entry::Tool { tool, target, outcome, label, background } => {
                let (verb, shown, edits) = match target {
                    // A member writing down what it learned, in its own memory.
                    Some(t) if is_memory(tool, t) => {
                        let note = Path::new(t).file_name().map_or_else(|| t.clone(), |n| n.to_string_lossy().into_owned());
                        ("recall".to_owned(), note, false)
                    }
                    _ => {
                        let verb = tool_verb(tool).to_owned();
                        let edits = matches!(verb.as_str(), "edit" | "write");
                        (verb, target.as_deref().map(|t| relative(t, base)).unwrap_or_default(), edits)
                    }
                };
                let outcome = outcome.as_deref();
                let merges = matches!(verb.as_str(), "read" | "search" | "fetch" | "recall" | "browse");
                match run.last_mut() {
                    Some(line) if merges && line.verb == verb && line.status != Status::Failed && outcome.is_none_or(|o| o.ok) => {
                        if !shown.is_empty() && !line.targets.contains(&shown) {
                            line.targets.push(shown);
                        }
                        line.absorb(outcome);
                    }
                    _ => {
                        // One line on screen: a command's first, the rest when opened.
                        let first = shown.lines().next().unwrap_or_default().to_owned();
                        let mut line = ToolLine::new(verb, if shown.is_empty() { Vec::new() } else { vec![first.clone()] }, edits);
                        if let Some(o) = outcome {
                            line.status = if o.ok { Status::Done } else { Status::Failed };
                            line.ms = o.ms;
                            line.exit = o.exit;
                            line.output = (!o.output.is_empty()).then(|| (o.output.clone(), o.skipped));
                            line.diff = o.diff.clone();
                        }
                        line.label = label.clone();
                        line.background = *background;
                        line.path = target.clone().filter(|_| matches!(tool.as_str(), "Read" | "Edit" | "MultiEdit" | "Write" | "NotebookEdit"));
                        if line.path.is_none() && !shown.is_empty() {
                            line.full = Some(shown);
                        }
                        run.push(line);
                    }
                }
                continue;
            }
            _ => {}
        }
        if !run.is_empty() {
            blocks.extend(crate::activity::groups(std::mem::take(&mut run)).into_iter().map(Block::Steps));
        }
        match entry {
            // What equipe once told a member as if the person had said it (its
            // browser handed back): a line of equipe's, never their bubble.
            Entry::User { text } if crate::activity::handed_back_line(text) => blocks.push(Block::Note(t!("chat.browser_given_back").to_owned())),
            Entry::User { text } => blocks.push(Block::User(text.clone())),
            Entry::Reply { text } => {
                blocks.push(Block::Reply { text: text.clone(), first: !replied, live: false, worked: None });
                replied = true;
            }
            Entry::Note { text } => match (thought_of(text), worked_of(text)) {
                (Some((n, secs)), _) => blocks.push(Block::Thought { n, secs, text: String::new() }),
                (None, Some(worked)) => match blocks.last_mut() {
                    Some(Block::Reply { worked: slot, .. }) => *slot = Some(worked),
                    _ => blocks.push(Block::Worked(worked)),
                },
                (None, None) if crate::activity::handed_of(text).is_some() => {
                    let (name, why) = crate::activity::handed_of(text).unwrap_or_default();
                    blocks.push(Block::HandOver { name, why, live: false });
                }
                (None, None) => match crate::activity::news_of(text) {
                    // A ticket waiting for the person: their card, not a line.
                    Some(news) if news.kind == crate::activity::NewsKind::Waiting && ticket_ref(&news.text).is_some() => {
                        let (at, len) = ticket_ref(&news.text).unwrap_or_default();
                        let ticket = news.text[at + 2..at + len].parse().unwrap_or_default();
                        blocks.push(Block::YourTurn { ticket, news: news.text, live: false, look: 0 });
                    }
                    Some(news) => match blocks.last_mut() {
                        Some(Block::News(run)) => run.push(news),
                        _ => blocks.push(Block::News(vec![news])),
                    },
                    None => blocks.push(Block::Note(equipe_core::i18n::note(crate::i18n::lang(), text).into_owned())),
                },
            },
            Entry::Error { message, next } => blocks.push(Block::Error { message: message.clone(), next: next.clone() }),
            Entry::Landed { branch, stat } => blocks.push(Block::Landed { branch: branch.clone(), stat: stat.clone() }),
            Entry::Tool { .. } | Entry::Thread { .. } => {}
        }
    }
    if !run.is_empty() {
        blocks.extend(crate::activity::groups(run).into_iter().map(Block::Steps));
    }
    blocks
}

/// The conversation's hand-over rows brought up to date: each member's
/// latest is live (it can be opened and given back), the rest past.
pub fn mark_handed(blocks: &mut [Block]) {
    let mut seen = HashSet::new();
    for block in blocks.iter_mut().rev() {
        if let Block::HandOver { name, live, .. } = block {
            *live = seen.insert(name.clone());
        }
    }
}

/// The member's auto memory, which equipe keeps under the squad's members.
fn is_memory(tool: &str, target: &str) -> bool {
    matches!(tool, "Write" | "Edit" | "MultiEdit") && target.contains("/.equipe/") && target.contains("/members/") && target.contains("/memory/")
}

/// A step's verb ([`tool_verb`]) as the person reads it; a tool without a
/// word of its own reads as its name.
pub fn verb_word(verb: &str) -> &str {
    match verb {
        "read" => t!("chat.verb_read"),
        "edit" => t!("chat.verb_edit"),
        "write" => t!("chat.verb_write"),
        "run" => t!("chat.verb_run"),
        "search" => t!("chat.verb_search"),
        "fetch" => t!("chat.verb_fetch"),
        "delegate" => t!("chat.verb_delegate"),
        "board" => t!("chat.verb_board"),
        "ticket" => t!("chat.verb_ticket"),
        "comment" => t!("chat.verb_comment"),
        "hand in" => t!("chat.verb_hand_in"),
        "wait on you" => t!("chat.verb_wait_on_you"),
        "ask you" => t!("chat.verb_ask_you"),
        "browse" => t!("chat.verb_browse"),
        "recall" => t!("chat.verb_recall"),
        other => other,
    }
}

/// How a tool use reads in one word.
pub fn tool_verb(name: &str) -> &str {
    match name {
        "Read" => "read",
        "Edit" | "MultiEdit" | "NotebookEdit" => "edit",
        "Write" => "write",
        "Bash" => "run",
        "Glob" | "Grep" => "search",
        "WebFetch" | "WebSearch" => "fetch",
        // A subagent the member handed side work to.
        "Agent" | "Task" => "delegate",
        other => match other.strip_prefix("mcp__equipe__") {
            // equipe's own board tools read as what they do.
            Some("board") => "board",
            Some("create_ticket") => "ticket",
            Some("update_ticket") => "ticket",
            Some("comment") => "comment",
            Some("submit_for_review") => "hand in",
            Some("block_on_person") => "wait on you",
            Some("ask_person") => "ask you",
            // The member's browser: every step reads as browsing.
            _ if other.starts_with("mcp__browser__") => "browse",
            _ => other,
        },
    }
}

/// A tool's path, relative to where the agent works when it's inside it.
pub fn relative(target: &str, base: &Path) -> String {
    let path = Path::new(target);
    // macOS reports temp dirs under /private; compare canonical forms.
    // A file not there (yet, or on another machine) compares as written.
    path.strip_prefix(base)
        .ok()
        .map(Path::to_owned)
        .or_else(|| canonical(path).strip_prefix(canonical(base)).ok().map(Path::to_owned))
        .filter(|rel| !rel.as_os_str().is_empty())
        .map(|rel| rel.display().to_string())
        .unwrap_or_else(|| target.to_owned())
}

/// `p`'s canonical form (or `p`, when it isn't there), as the disk had it in
/// the last couple of seconds. Every call of every tool in the conversation is
/// made relative each time it is drawn anew — while a reply streams, ten
/// times a second — and asking the disk each time cost a long day's
/// conversation a few thousand `realpath`s a tick (`docs/PERF-2026-10.md`).
fn canonical(p: &Path) -> PathBuf {
    use std::cell::RefCell;
    use std::collections::HashMap;
    const FRESH: Duration = Duration::from_secs(2);
    thread_local! {
        static KNOWN: RefCell<(Option<Instant>, HashMap<PathBuf, PathBuf>)> = RefCell::new((None, HashMap::new()));
    }
    KNOWN.with_borrow_mut(|(since, known)| {
        let now = Instant::now();
        if since.is_none_or(|at| now.duration_since(at) > FRESH) || known.len() > 8192 {
            known.clear();
            *since = Some(now);
        }
        known.entry(p.to_owned()).or_insert_with(|| p.canonicalize().unwrap_or_else(|_| p.to_owned())).clone()
    })
}

/// Ticket numbers in text that stand on their own (not in a path, a file
/// name or an address): the core's rule, the console's too.
pub use equipe_core::board::{ticket_ref, ticket_ref_from};

/// The link target a ticket number carries in markdown.
pub const TICKET_SCHEME: &str = "ticket:";

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::SessionState;

    const T: Clock = Clock { hour: 14, minute: 2, second: 0 };

    fn convo(pm: bool) -> Convo {
        Convo::new(PathBuf::from("/work/site"), pm)
    }

    fn tool(name: &str, target: &str) -> Event {
        Event::ToolUse { session: 1, tool: name.into(), target: Some(target.into()), label: None, background: false }
    }

    fn feed(c: &mut Convo, events: &[Event]) {
        let now = Instant::now();
        for e in events {
            c.on_event(e, now, T);
        }
    }

    #[test]
    fn a_reply_streams_whole_through_what_adds_no_line() {
        // Said while it answered: once, the chunk after it started a reply of
        // its own, and "지" ended one row, "금 보드를" began the next.
        let mut c = convo(true);
        let delta = |text: &str| Event::TextDelta { session: 1, text: text.into() };
        feed(&mut c, &[Event::UserMessage { session: 1, text: "보드 확인해 줘".into() }, delta("네, 확인했습니다. 지")]);
        feed(&mut c, &[Event::SteerQueued { session: 1, text: "그리고 하나 더".into(), project: None }, Event::SubagentStep { session: 1, id: "a".into(), step: "Read a.rs".into() }]);
        feed(&mut c, &[delta("금 보드를 살펴보겠습니다.")]);
        let replies: Vec<String> = c.blocks().into_iter().filter_map(|b| if let Block::Reply { text, .. } = b { Some(text) } else { None }).collect();
        assert_eq!(replies, ["네, 확인했습니다. 지금 보드를 살펴보겠습니다."]);
    }

    #[test]
    fn streaming_joins_one_reply_and_a_tool_use_closes_it() {
        let mut c = convo(true);
        feed(&mut c, &[Event::UserMessage { session: 1, text: "hi".into() }, Event::TextDelta { session: 1, text: "Hel".into() }, Event::TextDelta { session: 1, text: "lo".into() }]);
        assert_eq!(c.blocks(), vec![Block::User("hi".into()), Block::Reply { text: "Hello".into(), first: true, live: true, worked: None }]);
        feed(&mut c, &[tool("Read", "/work/site/a.rs"), Event::TextDelta { session: 1, text: "Done.".into() }]);
        let blocks = c.blocks();
        assert_eq!(blocks.len(), 4);
        let mut read = ToolLine::new("read", vec!["a.rs".into()], false);
        read.path = Some("/work/site/a.rs".into());
        assert_eq!(blocks[2], Block::Steps(vec![read]));
        assert_eq!(blocks[3], Block::Reply { text: "Done.".into(), first: false, live: true, worked: None }, "only the conversation's first reply names who speaks");
    }

    #[test]
    fn looking_around_merges_and_edits_stay_apart() {
        let mut c = convo(false);
        feed(&mut c, &[tool("Read", "/work/site/a.rs"), tool("Read", "/work/site/b.rs"), tool("Read", "/work/site/a.rs"), tool("Edit", "/work/site/a.rs"), tool("Edit", "/work/site/b.rs")]);
        let Block::Steps(lines) = &c.blocks()[0] else { panic!() };
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].verb, "read");
        assert_eq!(lines[0].targets, ["a.rs", "b.rs"], "a repeat adds nothing");
        assert!(lines[1].edits && lines[2].edits, "each edit is its own step");
    }

    #[test]
    fn steps_say_how_they_ended_and_run_while_unanswered() {
        let mut c = convo(false);
        let done = |tool: &str, target: &str, ok: bool, output: &str, diff: Option<&str>| Event::ToolResult {
            session: 1,
            tool: tool.into(),
            target: Some(target.into()),
            outcome: ToolOutcome { ok, exit: (!ok).then_some(2), ms: Some(1500), output: output.into(), skipped: 0, diff: diff.map(Into::into) },
        };
        feed(&mut c, &[Event::UserMessage { session: 1, text: "go".into() }, tool("Bash", "cargo test"), tool("Read", "/work/site/a.rs")]);
        let Block::Steps(lines) = &c.blocks()[1] else { panic!() };
        assert!(lines.iter().all(|l| l.status == Status::Running), "unanswered while it works");
        assert_eq!(crate::activity::status(lines), Status::Running);
        // The command fails: it stands apart from the reads after it, so it stays in sight.
        feed(&mut c, &[done("Bash", "cargo test", false, "error[E0308]", None), tool("Read", "/work/site/b.rs"), done("Read", "/work/site/a.rs", true, "", None)]);
        let blocks = c.blocks();
        let Block::Steps(failed) = &blocks[1] else { panic!() };
        assert_eq!((failed.len(), failed[0].status, failed[0].exit, failed[0].output.clone()), (1, Status::Failed, Some(2), Some(("error[E0308]".into(), 0))));
        let Block::Steps(reads) = &blocks[2] else { panic!("{blocks:?}") };
        assert_eq!(reads[0].status, Status::Running, "one read of the two still out");
        feed(&mut c, &[done("Read", "/work/site/b.rs", true, "", None), tool("Edit", "/work/site/a.rs"), done("Edit", "/work/site/a.rs", true, "", Some("diff --git a/a b/a\n"))]);
        let Block::Steps(lines) = &c.blocks()[2] else { panic!() };
        assert_eq!((lines[0].status, lines[0].ms), (Status::Done, Some(3000)), "a merged line adds its steps up");
        assert_eq!(lines[1].diff.as_deref(), Some("diff --git a/a b/a\n"));
        assert_eq!(lines[1].path.as_deref(), Some("/work/site/a.rs"));
        assert_eq!(crate::activity::status(lines), Status::Done);
        // Turn over: what never answered is just a step.
        feed(&mut c, &[tool("Bash", "ls"), Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None }]);
        let Block::Steps(lines) = &c.blocks()[2] else { panic!() };
        assert_eq!(lines[2].status, Status::Unknown);
    }

    #[test]
    fn a_long_command_shows_its_first_line_and_keeps_the_rest() {
        let mut c = convo(false);
        feed(&mut c, &[tool("Bash", "cat <<EOF > a\nhello\nEOF")]);
        let Block::Steps(lines) = &c.blocks()[0] else { panic!() };
        assert_eq!(lines[0].targets, ["cat <<EOF > a"]);
        assert_eq!(lines[0].full.as_deref(), Some("cat <<EOF > a\nhello\nEOF"));
        assert!(crate::activity::has_more(&lines[0]));
    }

    #[test]
    fn news_lines_gather_and_other_notes_stay_notes() {
        let mut c = convo(true);
        feed(
            &mut c,
            &[
                Event::Notice { session: 1, text: "backend handed in T-118 for your review · 주문 API.".into() },
                Event::Notice { session: 1, text: "You accepted T-58 · 헤더.".into() },
                Event::Notice { session: 1, text: "plain".into() },
                Event::Notice { session: 1, text: "pm이(가) T-9을(를) 제안했습니다: 로그인".into() },
            ],
        );
        let blocks = c.blocks();
        let Block::News(run) = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(run.iter().map(|n| n.kind).collect::<Vec<_>>(), [crate::activity::NewsKind::HandedIn, crate::activity::NewsKind::Accepted]);
        assert_eq!(blocks[1], Block::Note("plain".into()));
        assert!(matches!(&blocks[2], Block::News(run) if run.len() == 1));
    }

    #[test]
    fn a_browser_handed_over_is_a_row_to_act_on_in_any_language_its_latest_live() {
        let why = "App Store Connect(Umbriles Inc. 계정 소유자 Apple ID)에 로그인해 주세요. 끝나면 DSA '거래자' 신고는 제가 이어서 합니다.";
        let mut c = convo(true);
        feed(
            &mut c,
            &[
                // From a daemon that spoke English, popping the window on its own Mac.
                Event::Notice { session: 1, text: format!("🖥 pm's browser comes up on this Mac's screen: {why}") },
                Event::Notice { session: 1, text: "You accepted T-58 · 헤더.".into() },
                // Into the panel, worded in Korean.
                Event::Notice { session: 1, text: "🖥 pm의 브라우저가 브라우저 패널에 열립니다: 2단계 인증 코드를 입력해 주세요".into() },
            ],
        );
        let mut blocks = c.blocks();
        mark_handed(&mut blocks);
        assert_eq!(blocks[0], Block::HandOver { name: "pm".into(), why: why.into(), live: false }, "{blocks:?}");
        assert!(matches!(&blocks[1], Block::News(_)));
        assert_eq!(blocks[2], Block::HandOver { name: "pm".into(), why: "2단계 인증 코드를 입력해 주세요".into(), live: true });
        assert_eq!(blocks[2].searchable().as_deref(), Some("pm\n2단계 인증 코드를 입력해 주세요"));
    }

    #[test]
    fn a_browser_handed_back_long_ago_reads_as_equipes_line_not_the_persons_bubble() {
        let mut c = convo(true);
        feed(
            &mut c,
            &[
                Event::UserMessage { session: 1, text: "The person finished in the browser.".into() },
                Event::UserMessage { session: 1, text: "The person handled the page that needed them in your browser (a human check or a sign-in) and handed it back. Carry on where you were.".into() },
                Event::UserMessage { session: 1, text: "The person finished the report".into() },
            ],
        );
        let blocks = c.blocks();
        assert_eq!(blocks[0], Block::Note(t!("chat.browser_given_back").to_owned()));
        assert_eq!(blocks[1], Block::Note(t!("chat.browser_given_back").to_owned()));
        assert_eq!(blocks[2], Block::User("The person finished the report".into()), "words of the person's own stay theirs");
    }

    #[test]
    fn a_ticket_waiting_for_the_person_is_a_card_not_a_news_line() {
        let mut c = convo(true);
        feed(
            &mut c,
            &[
                Event::Notice { session: 1, text: "You accepted T-58 · 헤더.".into() },
                Event::Notice { session: 1, text: "T-117 waits for you — ios needs something only you can do · 앱스토어 제출.".into() },
                Event::Notice { session: 1, text: "You accepted T-59 · 푸터.".into() },
            ],
        );
        let blocks = c.blocks();
        assert!(matches!(&blocks[0], Block::News(run) if run.len() == 1), "{blocks:?}");
        let Block::YourTurn { ticket, news, live, .. } = &blocks[1] else { panic!("{blocks:?}") };
        assert_eq!((*ticket, *live), (117, false));
        assert!(news.contains("T-117") && news.contains("앱스토어 제출"), "{news}");
        assert!(matches!(&blocks[2], Block::News(_)), "the news after it starts a run of its own");
    }

    #[test]
    fn reasoning_shows_while_it_goes_and_stays_as_a_line() {
        let mut c = convo(false);
        let t0 = Instant::now();
        c.on_event(&Event::UserMessage { session: 1, text: "go".into() }, t0, T);
        c.on_event(&Event::Thinking { session: 1, text: "First, the header.\nThen the ".into() }, t0, T);
        assert_eq!(c.blocks().last(), Some(&Block::Thinking("Then the ".into())));
        c.on_event(&Event::Thinking { session: 1, text: "footer.".into() }, t0, T);
        c.on_event(&Event::TextDelta { session: 1, text: "Done".into() }, t0 + Duration::from_secs(4), T);
        let blocks = c.blocks();
        assert_eq!(blocks[1], Block::Thought { n: 0, secs: 4, text: "First, the header.\nThen the footer.".into() });
        assert_eq!(blocks[2], Block::Reply { text: "Done".into(), first: true, live: true, worked: None });
    }

    #[test]
    fn a_tool_line_counts_the_targets_past_three() {
        let line = ToolLine::new("read", (1..=5).map(|n| format!("f{n}")).collect(), false);
        assert_eq!(line.text(), "f1, f2, f3, +2");
    }

    #[test]
    fn the_pm_keeps_one_conversation_and_its_tickets_work_shows_as_the_ticket() {
        let mut c = convo(true);
        feed(
            &mut c,
            &[
                Event::UserMessage { session: 1, text: "whole squad".into() },
                Event::Thread { session: 1, project: Some("site".into()) },
                Event::UserMessage { session: 1, text: "about the site".into() },
                Event::Notice { session: 1, text: "── T-4 · Research".into() },
                Event::UserMessage { session: 1, text: "ticket work".into() },
                Event::Thread { session: 1, project: None },
                Event::UserMessage { session: 1, text: "after".into() },
            ],
        );
        let texts = |c: &Convo| c.blocks().iter().filter_map(|b| if let Block::User(t) = b { Some(t.clone()) } else { None }).collect::<Vec<_>>();
        assert_eq!(texts(&c), ["whole squad", "about the site", "after"]);
        assert!(c.has_work_on(4) && !c.has_work_on(5));
        c.showing = Some(4);
        assert_eq!(texts(&c), ["ticket work"]);
    }

    /// Two turns about the app around one about the site, news of the site,
    /// and one about the whole squad.
    fn two_projects() -> Convo {
        let mut c = convo(true);
        let say = |text: &str| Event::UserMessage { session: 1, text: text.into() };
        let reply = |text: &str| Event::TextDelta { session: 1, text: text.into() };
        let end = Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None };
        let about = |p: Option<&str>| Event::Thread { session: 1, project: p.map(str::to_owned) };
        feed(
            &mut c,
            &[
                about(Some("app")), say("app one"), tool("Read", "/work/site/a.rs"), reply("app answer"), end.clone(),
                about(Some("site")), say("site one"), reply("site answer"), end.clone(),
                Event::Notice { session: 1, text: "frontend이(가) T-4을(를) 검토해 달라고 넘겼습니다 · 히어로.".into() },
                about(None), say("everyone"), reply("squad answer"), end.clone(),
                about(Some("app")), say("app two"), reply("app again"), end,
            ],
        );
        c
    }

    fn users(blocks: &[Block]) -> Vec<String> {
        blocks.iter().filter_map(|b| if let Block::User(t) = b { Some(t.clone()) } else { None }).collect()
    }

    #[test]
    fn a_project_in_focus_shows_its_own_lines_and_the_squads_never_another_projects() {
        let c = two_projects();
        let app = c.sliced(Some("app"));
        assert_eq!(users(&app.blocks), ["app one", "everyone", "app two"]);
        assert!(app.about.iter().all(|a| a.as_deref() != Some("site")), "nothing of the site's, not even a row saying it's there");
        assert!(!app.blocks.iter().any(|b| b.searchable().is_some_and(|t| t.contains("site answer") || t.contains("T-4"))));
        let first: Vec<bool> = app.blocks.iter().filter_map(|b| if let Block::Reply { first, .. } = b { Some(*first) } else { None }).collect();
        assert_eq!(first, [true, false, false], "who speaks is named once");
        let site = c.sliced(Some("site"));
        assert_eq!(users(&site.blocks), ["site one", "everyone"]);
        assert!(site.blocks.iter().any(|b| matches!(b, Block::News(n) if n.len() == 1)), "news of the site's ticket shows in the site");
        // A project with nothing said yet: what was said to everyone.
        assert_eq!(users(&c.sliced(Some("web")).blocks), ["everyone"]);
    }

    #[test]
    fn every_project_shows_everything_saying_what_each_message_was_about() {
        let c = two_projects();
        let all = c.sliced(None);
        assert_eq!(users(&all.blocks), ["app one", "site one", "everyone", "app two"]);
        let said_in: Vec<Option<&str>> = all.blocks.iter().zip(&all.about).filter_map(|(b, a)| matches!(b, Block::User(_)).then_some(a.as_deref())).collect();
        assert_eq!(said_in, [Some("app"), Some("site"), None, Some("app")]);
        let news = all.blocks.iter().position(|b| matches!(b, Block::News(_))).unwrap();
        assert_eq!(all.about[news].as_deref(), Some("site"), "news carries the project of the turn it came in");
        // Unchanged from the whole conversation otherwise.
        assert_eq!(all.blocks, c.blocks());
    }

    #[test]
    fn another_projects_reply_streaming_stays_in_its_project() {
        let mut c = two_projects();
        feed(&mut c, &[Event::Thread { session: 1, project: Some("site".into()) }, Event::UserMessage { session: 1, text: "site two".into() }, Event::TextDelta { session: 1, text: "stream".into() }]);
        let app = c.sliced(Some("app"));
        assert!(!app.blocks.iter().any(|b| matches!(b, Block::Reply { text, .. } if text == "stream")), "{:?}", app.blocks);
        assert!(!users(&app.blocks).contains(&"site two".to_owned()));
        let site = c.sliced(Some("site"));
        assert!(matches!(site.blocks.last(), Some(Block::Reply { live: true, .. })));
    }

    #[test]
    fn a_squad_wide_reply_streaming_shows_in_the_project_in_focus() {
        let mut c = two_projects();
        feed(&mut c, &[Event::Thread { session: 1, project: None }, Event::UserMessage { session: 1, text: "all of you".into() }, Event::TextDelta { session: 1, text: "stream".into() }]);
        let app = c.sliced(Some("app"));
        assert!(matches!(app.blocks.last(), Some(Block::Reply { text, live: true, .. }) if text == "stream"), "{:?}", app.blocks);
        assert!(users(&app.blocks).contains(&"all of you".to_owned()));
    }

    #[test]
    fn memos_show_in_the_project_they_were_left_about() {
        let mut c = two_projects();
        feed(
            &mut c,
            &[
                Event::UserMessage { session: 1, text: "work".into() },
                Event::MessageQueued { session: 1, text: "for site".into(), project: Some("site".into()) },
                Event::MessageQueued { session: 1, text: "for app".into(), project: Some("app".into()) },
            ],
        );
        let memos = |s: &Sliced| waiting(&s.blocks).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
        assert_eq!(memos(&c.sliced(Some("app"))), ["for app"]);
        assert_eq!(memos(&c.sliced(None)), ["for site", "for app"]);
    }

    /// What waits, as the conversation shows it: its words, and whether a steer.
    fn waiting(blocks: &[Block]) -> Vec<(String, bool)> {
        blocks.iter().filter_map(|b| if let Block::Pending { text, steer, .. } = b { Some((text.clone(), *steer)) } else { None }).collect()
    }

    #[test]
    fn steers_show_in_the_project_they_were_said_about_and_the_squads() {
        let mut c = two_projects();
        feed(
            &mut c,
            &[
                Event::UserMessage { session: 1, text: "work".into() },
                Event::SteerQueued { session: 1, text: "mobile한 명 더 추가하는거 승인할게.".into(), project: None },
                Event::SteerQueued { session: 1, text: "우리 웹사이트는 어떻게 되어가는중?".into(), project: Some("site".into()) },
                Event::MessageQueued { session: 1, text: "app memo".into(), project: Some("app".into()) },
            ],
        );
        let site = c.sliced(Some("site"));
        assert_eq!(waiting(&site.blocks), [("mobile한 명 더 추가하는거 승인할게.".to_owned(), true), ("우리 웹사이트는 어떻게 되어가는중?".to_owned(), true)], "{:?}", site.blocks);
        // Every project: all of them, each with its project.
        let all = c.sliced(None);
        assert_eq!(waiting(&all.blocks).len(), 3);
        assert_eq!(all.about[all.about.len() - 3..], [None, Some("site".into()), Some("app".into())]);
    }

    #[test]
    fn a_members_conversation_and_a_tickets_work_are_never_sliced() {
        let mut c = two_projects();
        c.is_pm = false;
        assert_eq!(c.sliced(Some("app")).blocks, c.blocks());
        let mut c = two_projects();
        c.showing = Some(4);
        assert_eq!(c.sliced(Some("app")).blocks, c.blocks());
    }

    #[test]
    fn a_command_shows_its_description_and_one_in_the_background_runs_while_the_turn_does() {
        let mut c = convo(true);
        let steps = |c: &Convo| -> Vec<(String, String, Status, bool)> {
            c.blocks().iter().filter_map(|b| if let Block::Steps(lines) = b { Some(lines.clone()) } else { None }).flatten().map(|l| (l.verb.clone(), l.text(), l.status, l.background)).collect()
        };
        let watch = "until git log -1 | grep -q x; do sleep 30; done";
        feed(
            &mut c,
            &[
                Event::UserMessage { session: 1, text: "배포 지켜봐 줘".into() },
                Event::ToolUse { session: 1, tool: "Bash".into(), target: Some(watch.into()), label: Some("새 커밋·배포 감시".into()), background: true },
                Event::TextDelta { session: 1, text: "지켜보고 있어요.".into() },
            ],
        );
        // Its words over the command, which is under it when opened; running
        // in the background while the turn goes on, though a reply came after.
        assert_eq!(steps(&c), [("run".to_owned(), "새 커밋·배포 감시".to_owned(), Status::Running, true)]);
        let line = c.blocks().iter().find_map(|b| if let Block::Steps(lines) = b { lines.first().cloned() } else { None }).unwrap();
        assert_eq!(line.full.as_deref(), Some(watch));
        assert!(crate::activity::has_more(&line), "the command opens under its description");
        assert_eq!(crate::activity::line_phrase(&line, true), "백그라운드에서 실행 중");
        // Its task ended: done.
        let outcome = equipe_core::protocol::ToolOutcome { ok: true, exit: None, ms: Some(240_000), output: "new commit".into(), skipped: 0, diff: None };
        feed(&mut c, &[Event::ToolResult { session: 1, tool: "Bash".into(), target: Some(watch.into()), outcome }]);
        assert_eq!(steps(&c)[0].2, Status::Done);
        // A turn that ended before it did: no longer shown as running.
        let mut c = convo(true);
        feed(
            &mut c,
            &[
                Event::UserMessage { session: 1, text: "x".into() },
                Event::ToolUse { session: 1, tool: "Bash".into(), target: Some(watch.into()), label: None, background: true },
                Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None },
            ],
        );
        assert_eq!(steps(&c)[0].2, Status::Unknown);
        assert_eq!(steps(&c)[0].1, watch, "no description: the command");
    }

    #[test]
    fn a_members_conversation_shows_all_its_work() {
        let mut c = convo(false);
        feed(
            &mut c,
            &[
                Event::Thread { session: 1, project: Some("site".into()) },
                Event::Notice { session: 1, text: "── T-1 · Hero".into() },
                Event::Thread { session: 1, project: Some("app".into()) },
                Event::Notice { session: 1, text: "── T-2 · Login".into() },
                Event::Notice { session: 1, text: "plain".into() },
            ],
        );
        let notes: Vec<String> = c.blocks().into_iter().filter_map(|b| if let Block::Note(t) = b { Some(t) } else { None }).collect();
        assert_eq!(notes, ["── T-1 · Hero", "── T-2 · Login", "plain"]);
        c.showing = Some(2);
        let notes: Vec<String> = c.blocks().into_iter().filter_map(|b| if let Block::Note(t) = b { Some(t) } else { None }).collect();
        assert_eq!(notes, ["── T-2 · Login", "plain"]);
    }

    #[test]
    fn a_turn_leaves_how_long_it_took() {
        let mut c = convo(true);
        let t0 = Instant::now();
        c.on_event(&Event::UserMessage { session: 1, text: "go".into() }, t0, T);
        c.on_event(&Event::TextDelta { session: 1, text: "ok".into() }, t0, T);
        c.on_event(&Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None }, t0 + Duration::from_secs(65), T);
        let Some(Block::Reply { worked: Some(worked), .. }) = c.blocks().last().cloned() else { panic!("{:?}", c.blocks()) };
        assert_eq!((worked.took(), worked.at.as_str()), ("1분 05초 작업함".to_owned(), "14:02"), "on the reply that ended the turn");
        assert!(worked.unix.is_some());
        // A turn ending on a step: a line of its own.
        let mut steps = convo(true);
        steps.on_event(&Event::UserMessage { session: 1, text: "go".into() }, t0, T);
        steps.on_event(&tool("Bash", "ls"), t0, T);
        steps.on_event(&Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None }, t0 + Duration::from_secs(5), T);
        assert!(matches!(steps.blocks().last(), Some(Block::Worked(w)) if w.secs == 5));
        // Lines kept before the moment was: still read.
        assert_eq!(worked_of("\u{E001}65:14:02"), Some(Worked { secs: 65, at: "14:02".into(), unix: None }));
        // A turn under a second says nothing.
        let mut quick = convo(true);
        quick.on_event(&Event::UserMessage { session: 1, text: "go".into() }, t0, T);
        quick.on_event(&Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None }, t0, T);
        assert_eq!(quick.blocks().len(), 1);
    }

    #[test]
    fn a_memo_waits_below_the_conversation_until_the_turn_delivers_it() {
        let mut c = convo(false);
        feed(&mut c, &[Event::UserMessage { session: 1, text: "work".into() }, Event::MessageQueued { session: 1, text: "also this".into(), project: None }]);
        assert_eq!(waiting(&c.blocks()), [("also this".to_owned(), false)]);
        assert!(matches!(c.blocks().last(), Some(Block::Pending { .. })));
        feed(&mut c, &[Event::UserMessage { session: 1, text: "also this".into() }]);
        assert!(c.queue.is_empty());
        assert!(waiting(&c.blocks()).is_empty());
    }

    #[test]
    fn memos_wait_below_the_one_conversation_and_go_in_a_project_at_a_time() {
        let mut c = convo(false);
        feed(
            &mut c,
            &[
                Event::UserMessage { session: 1, text: "work".into() },
                Event::MessageQueued { session: 1, text: "for site".into(), project: Some("site".into()) },
                Event::MessageQueued { session: 1, text: "for app".into(), project: Some("app".into()) },
            ],
        );
        let memos = |c: &Convo| waiting(&c.blocks()).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
        assert_eq!(memos(&c), ["for site", "for app"]);
        // The next turn takes the first project's; the other waits.
        feed(&mut c, &[Event::UserMessage { session: 1, text: "for site".into() }]);
        assert_eq!(c.queue.texts(), ["for app"]);
    }

    #[test]
    fn steering_shows_as_read_next_step_until_read_or_the_turn_ends() {
        let mut c = convo(false);
        feed(&mut c, &[Event::UserMessage { session: 1, text: "work".into() }, Event::SteerQueued { session: 1, text: "skip the footer".into(), project: None }]);
        assert_eq!(waiting(&c.blocks()), [("skip the footer".to_owned(), true)]);
        // Read: it is the conversation's, where it was read.
        feed(&mut c, &[Event::Steered { session: 1, text: "skip the footer".into() }]);
        assert!(c.queue.is_empty());
        assert!(matches!(c.blocks().last(), Some(Block::User(t)) if t == "skip the footer"));
        // Unread at the turn's end: a memo in its place, delivered with the next turn.
        feed(&mut c, &[Event::SteerQueued { session: 1, text: "and the nav".into(), project: None }, Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None }]);
        assert_eq!(waiting(&c.blocks()), [("and the nav".to_owned(), false)]);
        feed(&mut c, &[Event::MessageQueued { session: 1, text: "and the nav".into(), project: None }]);
        assert_eq!(waiting(&c.blocks()).len(), 1, "once");
        feed(&mut c, &[Event::UserMessage { session: 1, text: "and the nav".into() }]);
        assert!(c.queue.is_empty());
    }

    #[test]
    fn a_suggestion_waits_for_rest_and_goes_when_work_or_a_memo_comes() {
        let mut c = convo(false);
        feed(&mut c, &[Event::UserMessage { session: 1, text: "work".into() }, Event::Suggestion { session: 1, text: "too early".into() }]);
        assert_eq!(c.suggestion, None, "still at work");
        feed(&mut c, &[Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None }, Event::Suggestion { session: 1, text: "다른 페이지도".into() }]);
        assert_eq!(c.suggestion.as_deref(), Some("다른 페이지도"));
        feed(&mut c, &[Event::UserMessage { session: 1, text: "다른 페이지도".into() }]);
        assert_eq!(c.suggestion, None);
        feed(&mut c, &[Event::MessageQueued { session: 1, text: "m".into(), project: None }, Event::Suggestion { session: 1, text: "late".into() }]);
        assert_eq!(c.suggestion, None, "a memo is on its way");
    }

    #[test]
    fn subagents_come_and_go_with_their_steps() {
        let mut c = convo(false);
        let start = Instant::now();
        c.on_event(&Event::SubagentStarted { session: 1, id: "a".into(), what: "look".into() }, start, T);
        c.on_event(&Event::SubagentStep { session: 1, id: "a".into(), step: "Read x".into() }, start + Duration::from_secs(5), T);
        assert_eq!(c.subagents, [Sub { id: "a".into(), what: "look".into(), step: Some("Read x".into()), since: start }], "its clock runs from when it started");
        // Told again (attached as it started): still one.
        feed(&mut c, &[Event::SubagentStarted { session: 1, id: "a".into(), what: "look".into() }, Event::SubagentStarted { session: 1, id: "b".into(), what: "more".into() }]);
        assert_eq!(c.subagents.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["a", "b"], "in the order they started");
        feed(&mut c, &[Event::SubagentEnded { session: 1, id: "a".into() }, Event::SubagentStep { session: 1, id: "a".into(), step: "late".into() }]);
        assert_eq!(c.subagents.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["b"], "a step after its end brings nothing back");
        feed(&mut c, &[Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None }]);
        assert!(c.subagents.is_empty(), "the turn's end takes the rest with it");
    }

    #[test]
    fn attaching_mid_turn_lists_the_subagents_at_work_with_their_clocks() {
        let info: SessionInfo = serde_json::from_value(serde_json::json!({
            "session": 2, "project": "/p", "member": {"role": "frontend", "name": "frontend"}, "engine": "claude", "model": null,
            "access": "read_only", "workspace": null, "state": "working", "title": null, "tokens": 0,
            "recent": [{"kind": "tool", "tool": "Agent", "target": "Read the logs"}],
            "subagents": [{"id": "t1", "what": "Read the logs", "step": "Grep error", "secs": 90}]
        }))
        .unwrap();
        let now = Instant::now();
        let c = Convo::from_info(&info, now);
        assert_eq!(c.subagents, [Sub { id: "t1".into(), what: "Read the logs".into(), step: Some("Grep error".into()), since: now - Duration::from_secs(90) }]);
        let Block::Steps(lines) = &c.blocks()[0] else { panic!("{:?}", c.blocks()) };
        assert_eq!((lines[0].verb.as_str(), lines[0].status), ("delegate", Status::Running));
    }

    #[test]
    fn a_delegation_runs_while_its_subagent_does_even_answered_or_further_up() {
        let mut c = convo(false);
        feed(
            &mut c,
            &[
                Event::UserMessage { session: 1, text: "go".into() },
                tool("Agent", "Read the logs"),
                Event::SubagentStarted { session: 1, id: "t1".into(), what: "Read the logs".into() },
                // Sent to the background: answered at once, at work still.
                Event::ToolResult { session: 1, tool: "Agent".into(), target: Some("Read the logs".into()), outcome: ToolOutcome { ok: true, exit: None, ms: Some(5), output: String::new(), skipped: 0, diff: None } },
                Event::TextDelta { session: 1, text: "Meanwhile…".into() },
                tool("Read", "/work/site/a.rs"),
            ],
        );
        let blocks = c.blocks();
        let sub = c.subagents[0].clone();
        let (at, line) = delegation_of(&blocks, &sub).expect("its delegation");
        assert_eq!(at, 1, "{blocks:?}");
        let Block::Steps(lines) = &blocks[at] else { panic!() };
        assert_eq!(lines[line].status, Status::Running);
        feed(&mut c, &[Event::SubagentEnded { session: 1, id: "t1".into() }]);
        let Block::Steps(lines) = &c.blocks()[1] else { panic!() };
        assert_eq!(lines[0].status, Status::Done, "handed back: done");
    }

    #[test]
    fn a_subagent_knows_its_task_cut_at_eighty_characters() {
        let long = "a".repeat(100);
        let sub = Sub { id: "x".into(), what: "a".repeat(80), step: None, since: Instant::now() };
        assert!(sub.handed(&long) && !sub.handed("b") && !sub.handed(""));
    }

    #[test]
    fn the_token_counter_is_not_a_change() {
        let mut c = convo(false);
        assert!(!c.on_event(&Event::Output { session: 1, tokens: 9 }, Instant::now(), T));
        assert!(c.on_event(&Event::TextDelta { session: 1, text: "x".into() }, Instant::now(), T));
    }

    #[test]
    fn errors_and_landed_branches_are_blocks_of_their_own() {
        let mut c = convo(false);
        feed(
            &mut c,
            &[
                Event::Error { session: Some(1), message: "boom".into(), next: Some("equipe login".into()) },
                Event::BranchLanded { session: 1, branch: "b".into(), base: "main".into(), stat: " a | 1 +".into() },
            ],
        );
        assert_eq!(c.blocks()[0], Block::Error { message: "boom".into(), next: Some("equipe login".into()) });
        assert_eq!(c.blocks()[1], Block::Landed { branch: "b".into(), stat: " a | 1 +".into() });
    }

    #[test]
    fn attaching_mid_turn_starts_the_clock_now() {
        let info: SessionInfo = serde_json::from_value(serde_json::json!({
            "session": 1, "project": "/p", "member": {"role": "pm", "name": "pm"}, "engine": "claude", "model": null,
            "access": "read_only", "workspace": null, "state": "working", "title": null, "tokens": 0,
            "recent": [{"kind": "reply", "text": "so far"}], "queued": ["m"]
        }))
        .unwrap();
        assert_eq!(info.state, SessionState::Working);
        let now = Instant::now();
        let mut c = Convo::from_info(&info, now);
        assert!(c.is_pm);
        assert_eq!(c.queue.texts(), ["m"]);
        c.on_event(&Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None }, now + Duration::from_secs(3), T);
        assert!(c.blocks().iter().any(|b| matches!(b, Block::Reply { worked: Some(w), .. } if w.took() == "3초 작업함")), "{:?}", c.blocks());
    }

    #[test]
    fn tool_verbs_read_as_what_they_do() {
        assert_eq!(tool_verb("Bash"), "run");
        assert_eq!(tool_verb("mcp__equipe__submit_for_review"), "hand in");
        assert_eq!(tool_verb("mcp__browser__browser_click"), "browse");
        assert_eq!(tool_verb("Odd"), "Odd");
    }

    #[test]
    fn a_path_is_shown_from_where_the_agent_works_however_it_is_spelled() {
        let dir = std::env::temp_dir().join(format!("equipe-relative-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "").unwrap();
        let base = dir.canonicalize().unwrap();
        // As written, and through a link to the same place (macOS's temp dir is under /private).
        assert_eq!(relative(&base.join("src/a.rs").display().to_string(), &base), "src/a.rs");
        let linked = dir.join("src/a.rs").display().to_string();
        for _ in 0..2 {
            assert_eq!(relative(&linked, &base), "src/a.rs");
        }
        // Outside it, a command, the place itself: as written.
        assert_eq!(relative("/elsewhere/b.rs", &base), "/elsewhere/b.rs");
        assert_eq!(relative("cargo test -p api", &base), "cargo test -p api");
        assert_eq!(relative(&base.display().to_string(), &base), base.display().to_string());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
