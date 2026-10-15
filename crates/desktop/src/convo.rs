//! One member's conversation as the window holds it: the transcript the daemon
//! sends (folded with the same `Transcript` every screen uses), what is waiting
//! behind it, and the blocks the conversation is drawn as. No UI types here, so
//! the rules — which thread shows, how tool steps gather and fold — can be tested
//! without a window. Behaviour follows the terminal console (`tui/app.rs`,
//! `tui/view.rs`).

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use equipe_core::protocol::{Entry, Event, SessionInfo, TicketId, ToolOutcome, Transcript, TurnOutcome};
use equipe_core::t;

use crate::state::Clock;

/// A run of tool lines longer than this folds to its first and last lines.
pub const RUN_FOLDS_AT: usize = 6;
/// How many targets a merged tool line names before it counts the rest.
pub const TARGETS_SHOWN: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sub {
    pub id: String,
    pub what: String,
    pub step: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Convo {
    pub transcript: Transcript,
    /// Shown as the work on this ticket only — the ticket open on screen.
    pub showing: Option<TicketId>,
    /// Memos waiting for the turn in progress to end.
    pub queued: Vec<String>,
    /// The project each memo was left about (`None`: the whole squad): each
    /// shows in that project's conversation.
    pub queued_in: Vec<Option<String>>,
    /// What the person said while it works, not read yet (steering).
    pub steering: Vec<String>,
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
}

/// The transcript line where reasoning ended: `THOUGHT` + its number + `:` +
/// seconds. A private-use character, so no other note starts so.
pub const THOUGHT: char = '\u{E000}';

/// The transcript line where a turn ended: `WORKED` + its seconds + `:` + the
/// time of day — worded when drawn, so in the language in use then.
pub const WORKED: char = '\u{E001}';

impl Convo {
    pub fn new(base: PathBuf, is_pm: bool) -> Self {
        Self {
            transcript: Transcript::default(),
            showing: None,
            queued: Vec::new(),
            queued_in: Vec::new(),
            steering: Vec::new(),
            suggestion: None,
            subagents: Vec::new(),
            desk: base.clone(),
            base,
            is_pm,
            lane: false,
            working_since: None,
            thinking: None,
            thoughts: Vec::new(),
        }
    }

    pub fn from_info(info: &SessionInfo, now: Instant) -> Self {
        let work_base = info.work.as_ref().map(|w| w.workspace.as_ref().map_or_else(|| w.project.clone(), |ws| ws.path.clone()));
        let mut convo = Self::new(work_base.unwrap_or_else(|| info.project.clone()), info.member.as_ref().is_some_and(|m| m.role == "pm"));
        convo.desk = info.project.clone();
        convo.transcript = Transcript::from_entries(info.recent.clone());
        convo.queued = info.queued.clone();
        // From an older daemon: one project for them all.
        convo.queued_in = if info.queued_in.len() == info.queued.len() { info.queued_in.clone() } else { vec![info.queued_project.clone(); info.queued.len()] };
        convo.steering = info.steering.clone();
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
            Event::SessionStarted { project, .. } => {
                self.base = project.clone();
                self.desk = project.clone();
            }
            Event::UserMessage { .. } => {
                self.working_since.get_or_insert(now);
                self.suggestion = None;
                // The memos of one project are delivered together, into its
                // conversation; another project's wait for the next turn's end.
                self.deliver_memos();
            }
            Event::SteerQueued { text, .. } => self.steering.push(text.clone()),
            Event::Steered { text, .. } => {
                if let Some(at) = self.steering.iter().position(|t| t == text) {
                    self.steering.remove(at);
                }
            }
            Event::Suggestion { text, .. } => {
                // Too late once it is at work again, or a memo is on its way.
                if self.working_since.is_none() && self.queued.is_empty() {
                    self.suggestion = Some(text.clone());
                }
            }
            Event::MessageQueued { text, project, .. } => {
                self.suggestion = None;
                self.queued.push(text.clone());
                self.queued_in.push(project.clone());
            }
            Event::WorkStarted { project, workspace, .. } => {
                self.suggestion = None;
                self.base = workspace.as_ref().map_or_else(|| project.clone(), |ws| ws.path.clone());
                self.working_since.get_or_insert(now);
            }
            Event::WorkEnded { .. } => {
                self.base = self.desk.clone();
            }
            Event::SubagentStarted { id, what, .. } => self.subagents.push(Sub { id: id.clone(), what: what.clone(), step: None }),
            Event::SubagentStep { id, step, .. } => {
                if let Some(sub) = self.subagents.iter_mut().find(|s| s.id == *id) {
                    sub.step = Some(step.clone());
                }
            }
            Event::SubagentEnded { id, .. } => self.subagents.retain(|s| s.id != *id),
            Event::TurnEnded { outcome, .. } => {
                self.subagents.clear();
                // Unread, it comes back as a memo, delivered now.
                self.steering.clear();
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

    /// What the reasoning numbered `n` was.
    pub fn thought(&self, n: usize) -> Option<&str> {
        self.thoughts.get(n).map(String::as_str)
    }

    /// The memos sent as its new turn starts: those of the first project one
    /// was left in.
    fn deliver_memos(&mut self) {
        let Some(project) = self.queued_in.first().cloned() else {
            self.queued.clear();
            return;
        };
        let (queued, queued_in) = std::mem::take(&mut self.queued).into_iter().zip(std::mem::take(&mut self.queued_in)).filter(|(_, about)| *about != project).unzip();
        (self.queued, self.queued_in) = (queued, queued_in);
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
                self.transcript.push_local(Entry::Note { text: format!("{WORKED}{}:{}", took.as_secs(), at.hm()) });
            }
        }
    }

    /// The conversation as it shows. A ticket open (`showing`): only its
    /// work. Otherwise everything — but the PM's own tickets' work, which each
    /// shows as its ticket. Never the markers.
    pub fn shown(&self) -> Vec<Cow<'_, Entry>> {
        let mut out = Vec::new();
        // The ticket whose work the lines are, while they are.
        let mut on: Option<TicketId> = None;
        for entry in self.transcript.entries() {
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
                out.push(Cow::Borrowed(entry));
            }
        }
        out
    }

    /// The blocks the conversation is drawn as: one per message, reply, run of
    /// steps, note — then each memo still waiting.
    pub fn blocks(&self) -> Vec<Block> {
        let mut blocks = build_blocks(&self.shown(), &self.base);
        let working = self.working_since.is_some();
        for block in &mut blocks {
            if let Block::Thought { n, text, .. } = block {
                *text = self.thought(*n).unwrap_or_default().to_owned();
            }
        }
        if working {
            // The reply still streaming, and the steps not answered yet.
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
        // Memos wait below the conversation until the turn ends.
        blocks.extend(self.queued.iter().map(|memo| Block::Memo(memo.clone())));
        blocks.extend(self.steering.iter().map(|text| Block::Steering(text.clone())));
        blocks
    }
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
}

impl ToolLine {
    pub fn new(verb: impl Into<String>, targets: Vec<String>, edits: bool) -> Self {
        Self { verb: verb.into(), targets, edits, status: Status::Unknown, ms: None, exit: None, output: None, diff: None, path: None }
    }

    /// `read  a.rs, b.rs, c.rs, +2` — what the line says after its mark.
    pub fn text(&self) -> String {
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

/// What a run of steps shows: all of it, or its start and end around a fold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunRow<'a> {
    Line(&'a ToolLine),
    Folded(usize),
}

pub fn run_rows(run: &[ToolLine], expanded: bool) -> Vec<RunRow<'_>> {
    if expanded || run.len() <= RUN_FOLDS_AT {
        return run.iter().map(RunRow::Line).collect();
    }
    let (head, tail) = (3, 2);
    let mut rows: Vec<RunRow<'_>> = run[..head].iter().map(RunRow::Line).collect();
    rows.push(RunRow::Folded(run.len() - head - tail));
    rows.extend(run[run.len() - tail..].iter().map(RunRow::Line));
    rows
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    User(String),
    /// A reply; `first` when it opens the member's turn (its name heads it);
    /// `live` while it is still streaming.
    Reply { text: String, first: bool, live: bool },
    Steps(Vec<ToolLine>),
    Note(String),
    /// Reasoning that ended: how long it took, and what it was.
    Thought { n: usize, secs: u64, text: String },
    /// Reasoning going on now: its latest line.
    Thinking(String),
    Error { message: String, next: Option<String> },
    Landed { branch: String, stat: String },
    /// A memo left while the member works.
    Memo(String),
    /// Said while it works: read at its next step, the work carrying on.
    Steering(String),
}

impl Block {
    /// The words a search finds in it.
    pub fn searchable(&self) -> Option<String> {
        match self {
            // Your words as shown: without the attachments' tags.
            Block::User(t) | Block::Memo(t) | Block::Steering(t) => Some(crate::sent::parts(t).text),
            Block::Reply { text: t, .. } | Block::Note(t) => Some(t.clone()),
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

/// The line a turn's end leaves, worded now: `─ worked 1m 05s · 14:02`.
fn worked_of(text: &str) -> Option<String> {
    let (secs, at) = text.strip_prefix(WORKED)?.split_once(':')?;
    Some(t!("chat.worked", took = elapsed(Duration::from_secs(secs.parse().ok()?)), at = at))
}

/// Gather entries into blocks: tool use into runs (merging looking-around
/// lines), everything else one to one.
pub fn build_blocks(entries: &[Cow<'_, Entry>], base: &Path) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut run: Vec<ToolLine> = Vec::new();
    let mut after_reply = false;
    for entry in entries {
        let entry = entry.as_ref();
        match entry {
            Entry::Thread { .. } => continue,
            // Claude Code loading tool descriptions: its business, not the person's.
            Entry::Tool { tool, .. } if tool == "ToolSearch" => continue,
            Entry::Tool { tool, target, outcome } => {
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
                    Some(line) if merges && line.verb == verb => {
                        if !shown.is_empty() && !line.targets.contains(&shown) {
                            line.targets.push(shown);
                        }
                        line.absorb(outcome);
                    }
                    _ => {
                        let mut line = ToolLine::new(verb, if shown.is_empty() { Vec::new() } else { vec![shown] }, edits);
                        if let Some(o) = outcome {
                            line.status = if o.ok { Status::Done } else { Status::Failed };
                            line.ms = o.ms;
                            line.exit = o.exit;
                            line.output = (!o.output.is_empty()).then(|| (o.output.clone(), o.skipped));
                            line.diff = o.diff.clone();
                        }
                        line.path = target.clone().filter(|_| matches!(tool.as_str(), "Read" | "Edit" | "MultiEdit" | "Write" | "NotebookEdit"));
                        run.push(line);
                    }
                }
                continue;
            }
            _ => {}
        }
        if !run.is_empty() {
            blocks.push(Block::Steps(std::mem::take(&mut run)));
        }
        match entry {
            Entry::User { text } => {
                after_reply = false;
                blocks.push(Block::User(text.clone()));
            }
            Entry::Reply { text } => {
                blocks.push(Block::Reply { text: text.clone(), first: !after_reply, live: false });
                after_reply = true;
            }
            Entry::Note { text } => match (thought_of(text), worked_of(text)) {
                (Some((n, secs)), _) => blocks.push(Block::Thought { n, secs, text: String::new() }),
                (None, Some(worked)) => blocks.push(Block::Note(worked)),
                (None, None) => blocks.push(Block::Note(equipe_core::i18n::note(crate::i18n::lang(), text).into_owned())),
            },
            Entry::Error { message, next } => blocks.push(Block::Error { message: message.clone(), next: next.clone() }),
            Entry::Landed { branch, stat } => blocks.push(Block::Landed { branch: branch.clone(), stat: stat.clone() }),
            Entry::Tool { .. } | Entry::Thread { .. } => {}
        }
        // Steps (and reasoning) between two replies don't make the second a new turn.
        let thought = matches!(entry, Entry::Note { text } if text.starts_with(THOUGHT));
        if !matches!(entry, Entry::Reply { .. }) && !thought {
            after_reply = false;
        }
    }
    if !run.is_empty() {
        blocks.push(Block::Steps(run));
    }
    blocks
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
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
    path.strip_prefix(base)
        .ok()
        .map(Path::to_owned)
        .or_else(|| canon(path).strip_prefix(canon(base)).ok().map(Path::to_owned))
        .filter(|rel| !rel.as_os_str().is_empty())
        .map(|rel| rel.display().to_string())
        .unwrap_or_else(|| target.to_owned())
}

/// The first `T-<digits>` in `text` that stands as a word: where, and how long.
pub fn ticket_ref(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(found) = text[from..].find("T-") {
        let at = from + found;
        let digits = bytes[at + 2..].iter().take_while(|b| b.is_ascii_digit()).count();
        let before_ok = at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'-');
        if digits > 0 && before_ok {
            return Some((at, 2 + digits));
        }
        from = at + 2;
    }
    None
}

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
        Event::ToolUse { session: 1, tool: name.into(), target: Some(target.into()) }
    }

    fn feed(c: &mut Convo, events: &[Event]) {
        let now = Instant::now();
        for e in events {
            c.on_event(e, now, T);
        }
    }

    #[test]
    fn streaming_joins_one_reply_and_a_tool_use_closes_it() {
        let mut c = convo(true);
        feed(&mut c, &[Event::UserMessage { session: 1, text: "hi".into() }, Event::TextDelta { session: 1, text: "Hel".into() }, Event::TextDelta { session: 1, text: "lo".into() }]);
        assert_eq!(c.blocks(), vec![Block::User("hi".into()), Block::Reply { text: "Hello".into(), first: true, live: true }]);
        feed(&mut c, &[tool("Read", "/work/site/a.rs"), Event::TextDelta { session: 1, text: "Done.".into() }]);
        let blocks = c.blocks();
        assert_eq!(blocks.len(), 4);
        let mut read = ToolLine::new("read", vec!["a.rs".into()], false);
        read.path = Some("/work/site/a.rs".into());
        assert_eq!(blocks[2], Block::Steps(vec![read]));
        assert_eq!(blocks[3], Block::Reply { text: "Done.".into(), first: false, live: true }, "steps between replies are one turn");
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
        feed(&mut c, &[done("Bash", "cargo test", false, "error[E0308]", None), tool("Read", "/work/site/b.rs"), done("Read", "/work/site/a.rs", true, "", None)]);
        let Block::Steps(lines) = &c.blocks()[1] else { panic!() };
        assert_eq!((lines[0].status, lines[0].exit, lines[0].output.clone()), (Status::Failed, Some(2), Some(("error[E0308]".into(), 0))));
        assert_eq!(lines[1].status, Status::Running, "one read of the two still out");
        feed(&mut c, &[done("Read", "/work/site/b.rs", true, "", None), tool("Edit", "/work/site/a.rs"), done("Edit", "/work/site/a.rs", true, "", Some("diff --git a/a b/a\n"))]);
        let Block::Steps(lines) = &c.blocks()[1] else { panic!() };
        assert_eq!((lines[1].status, lines[1].ms), (Status::Done, Some(3000)), "a merged line adds its steps up");
        assert_eq!(lines[2].diff.as_deref(), Some("diff --git a/a b/a\n"));
        assert_eq!(lines[2].path.as_deref(), Some("/work/site/a.rs"));
        // Turn over: what never answered is just a step.
        feed(&mut c, &[tool("Bash", "ls"), Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None }]);
        let Block::Steps(lines) = &c.blocks()[1] else { panic!() };
        assert_eq!(lines[3].status, Status::Unknown);
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
        assert_eq!(blocks[2], Block::Reply { text: "Done".into(), first: true, live: true }, "a thought doesn't start a new turn's header twice");
    }

    #[test]
    fn a_tool_line_counts_the_targets_past_three() {
        let line = ToolLine::new("read", (1..=5).map(|n| format!("f{n}")).collect(), false);
        assert_eq!(line.text(), "f1, f2, f3, +2");
    }

    #[test]
    fn a_long_run_folds_to_three_then_two_and_unfolds() {
        let run: Vec<ToolLine> = (0..10).map(|n| ToolLine::new("run", vec![format!("c{n}")], false)).collect();
        let rows = run_rows(&run, false);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[3], RunRow::Folded(5));
        assert_eq!(run_rows(&run, true).len(), 10);
        assert_eq!(run_rows(&run[..6], false).len(), 6, "six still fits");
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
        assert!(matches!(c.blocks().last(), Some(Block::Note(t)) if t == "─ 1분 05초 작업함 · 14:02"), "{:?}", c.blocks());
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
        assert_eq!(c.blocks().last(), Some(&Block::Memo("also this".into())));
        feed(&mut c, &[Event::UserMessage { session: 1, text: "also this".into() }]);
        assert!(c.queued.is_empty());
        assert!(!c.blocks().iter().any(|b| matches!(b, Block::Memo(_))));
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
        let memos = |c: &Convo| c.blocks().into_iter().filter_map(|b| if let Block::Memo(t) = b { Some(t) } else { None }).collect::<Vec<_>>();
        assert_eq!(memos(&c), ["for site", "for app"]);
        // The next turn takes the first project's; the other waits.
        feed(&mut c, &[Event::UserMessage { session: 1, text: "for site".into() }]);
        assert_eq!(c.queued, ["for app"]);
    }

    #[test]
    fn steering_shows_as_read_next_step_until_read_or_the_turn_ends() {
        let mut c = convo(false);
        feed(&mut c, &[Event::UserMessage { session: 1, text: "work".into() }, Event::SteerQueued { session: 1, text: "skip the footer".into() }]);
        assert_eq!(c.blocks().last(), Some(&Block::Steering("skip the footer".into())));
        feed(&mut c, &[Event::Steered { session: 1, text: "skip the footer".into() }]);
        assert!(c.steering.is_empty());
        feed(&mut c, &[Event::SteerQueued { session: 1, text: "and the nav".into() }, Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None }]);
        assert!(!c.blocks().iter().any(|b| matches!(b, Block::Steering(_))));
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
        feed(&mut c, &[Event::SubagentStarted { session: 1, id: "a".into(), what: "look".into() }, Event::SubagentStep { session: 1, id: "a".into(), step: "Read x".into() }]);
        assert_eq!(c.subagents, [Sub { id: "a".into(), what: "look".into(), step: Some("Read x".into()) }]);
        feed(&mut c, &[Event::SubagentEnded { session: 1, id: "a".into() }]);
        assert!(c.subagents.is_empty());
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
        assert_eq!(c.queued, ["m"]);
        c.on_event(&Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None }, now + Duration::from_secs(3), T);
        assert!(c.blocks().iter().any(|b| matches!(b, Block::Note(t) if t.starts_with("─ 3초 작업함"))), "{:?}", c.blocks());
    }

    #[test]
    fn tool_verbs_read_as_what_they_do() {
        assert_eq!(tool_verb("Bash"), "run");
        assert_eq!(tool_verb("mcp__equipe__submit_for_review"), "hand in");
        assert_eq!(tool_verb("mcp__browser__browser_click"), "browse");
        assert_eq!(tool_verb("Odd"), "Odd");
    }
}
