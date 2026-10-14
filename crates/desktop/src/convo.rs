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

use equipe_core::protocol::{Entry, Event, SessionInfo, Transcript, TurnOutcome};

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
    /// For the PM: the thread on screen — the project in focus, `None` for the
    /// whole squad's. A member's conversation is narrowed to that project too.
    pub thread_view: Option<String>,
    /// Memos waiting for the turn in progress to end.
    pub queued: Vec<String>,
    /// The project each memo was left about (`None`: the whole squad): each
    /// shows in that project's conversation.
    pub queued_in: Vec<Option<String>>,
    /// What the person said while it works, not read yet (steering).
    pub steering: Vec<String>,
    /// The greyed guess at what the person says next, while it rests.
    pub suggestion: Option<String>,
    /// Working on a ticket: it has one conversation, so every memo is for it.
    on_ticket: bool,
    /// Subagents it handed work to this turn and that haven't handed back.
    pub subagents: Vec<Sub>,
    /// Where it works now; tool paths are shown relative to it.
    pub base: PathBuf,
    desk: PathBuf,
    pub is_pm: bool,
    working_since: Option<Instant>,
}

impl Convo {
    pub fn new(base: PathBuf, is_pm: bool) -> Self {
        Self {
            transcript: Transcript::default(),
            thread_view: None,
            queued: Vec::new(),
            queued_in: Vec::new(),
            steering: Vec::new(),
            suggestion: None,
            on_ticket: false,
            subagents: Vec::new(),
            desk: base.clone(),
            base,
            is_pm,
            working_since: None,
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
        convo.on_ticket = info.work.is_some();
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
                self.on_ticket = true;
                self.suggestion = None;
                self.base = workspace.as_ref().map_or_else(|| project.clone(), |ws| ws.path.clone());
                self.working_since.get_or_insert(now);
            }
            Event::WorkEnded { .. } => {
                self.on_ticket = false;
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

    /// Whether what belongs to `project`'s conversation (`None`: the squad's)
    /// shows in the one in view.
    fn in_view(&self, project: Option<&str>) -> bool {
        project == self.thread_view.as_deref() || (self.thread_view.is_none() && !self.is_pm)
    }

    /// Its turn ended: leave a quiet line saying how long it took.
    fn stop_working(&mut self, now: Instant, at: Clock, _outcome: TurnOutcome) {
        if let Some(since) = self.working_since.take() {
            let took = now.saturating_duration_since(since);
            if took >= Duration::from_secs(1) {
                self.transcript.push_local(Entry::Note { text: format!("─ worked {} · {}", elapsed(took), at.hm()) });
            }
        }
    }

    /// The conversation as it shows, through the project in focus. The PM's is
    /// its thread for that project (what came before any thread was the whole
    /// squad's). A member's is everything with the whole squad in view — each
    /// ticket headed with its project — or that project's part, with a line
    /// where other projects' parts were left out. Never the markers.
    pub fn shown(&self) -> Vec<Cow<'_, Entry>> {
        let entries = self.transcript.entries();
        let pm = self.is_pm;
        let mut projects = HashSet::new();
        for entry in entries {
            if let Entry::Thread { project: Some(p) } = entry {
                projects.insert(p.as_str());
            }
        }
        let several = projects.len() > 1;
        let mut current: Option<&str> = None;
        let mut out = Vec::new();
        // Left out in a row: whether anything was, and how many tickets.
        let mut left_out: Option<usize> = None;
        let say_left_out = |out: &mut Vec<Cow<'_, Entry>>, left: Option<usize>| {
            let text = match left {
                None => return,
                Some(0) => "· 다른 프로젝트의 이전 대화는 숨겼습니다 — 프로젝트 칩 ‘All’로 보입니다".to_owned(),
                Some(n) => format!("· 다른 프로젝트의 티켓 {n}개는 숨겼습니다 — 프로젝트 칩 ‘All’로 보입니다"),
            };
            out.push(Cow::Owned(Entry::Note { text }));
        };
        for entry in entries {
            if let Entry::Thread { project } = entry {
                current = project.as_deref();
                continue;
            }
            let ticket_header = matches!(entry, Entry::Note { text } if text.starts_with("── T-"));
            let visible = if pm { current == self.thread_view.as_deref() } else { self.thread_view.is_none() || current == self.thread_view.as_deref() };
            if !visible {
                if !pm {
                    let n = left_out.get_or_insert(0);
                    *n += usize::from(ticket_header);
                }
                continue;
            }
            say_left_out(&mut out, left_out.take());
            match (entry, current) {
                (Entry::Note { text }, Some(project)) if ticket_header && !pm && self.thread_view.is_none() && several => {
                    out.push(Cow::Owned(Entry::Note { text: format!("── {project} · {}", &text["── ".len()..]) }));
                }
                _ => out.push(Cow::Borrowed(entry)),
            }
        }
        say_left_out(&mut out, left_out);
        out
    }

    /// The blocks the conversation is drawn as: one per message, reply, run of
    /// steps, note — then each memo still waiting.
    pub fn blocks(&self) -> Vec<Block> {
        let mut blocks = build_blocks(&self.shown(), &self.base);
        // Memos wait below the conversation they go into until the turn ends —
        // each only in the project it was left in. On a ticket there is one
        // conversation, the ticket's: every memo is for it.
        for (memo, _) in self.queued.iter().zip(&self.queued_in).filter(|(_, about)| self.on_ticket || self.in_view(about.as_deref())) {
            blocks.push(Block::Memo(memo.clone()));
        }
        blocks.extend(self.steering.iter().map(|text| Block::Steering(text.clone())));
        blocks
    }
}

/// `1m 05s`, `42s`, `1h 02m` — how long something has been going.
pub fn elapsed(took: Duration) -> String {
    let s = took.as_secs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, (s % 3600) / 60),
    }
}

/// One line of the conversation's tool activity. Consecutive reads, searches
/// and fetches merge into one line, so looking around reads as one step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLine {
    pub verb: String,
    pub targets: Vec<String>,
    pub edits: bool,
}

impl ToolLine {
    /// `read  a.rs, b.rs, c.rs, +2` — what the line says after its mark.
    pub fn text(&self) -> String {
        let mut named: Vec<String> = self.targets.iter().take(TARGETS_SHOWN).cloned().collect();
        if self.targets.len() > TARGETS_SHOWN {
            named.push(format!("+{}", self.targets.len() - TARGETS_SHOWN));
        }
        named.join(", ")
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
    /// A reply; `first` when it opens the member's turn (its name heads it).
    Reply { text: String, first: bool },
    Steps(Vec<ToolLine>),
    Note(String),
    Error { message: String, next: Option<String> },
    Landed { branch: String, stat: String },
    /// A memo left while the member works.
    Memo(String),
    /// Said while it works: read at its next step, the work carrying on.
    Steering(String),
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
            Entry::Tool { tool, target } => {
                let (verb, target, edits) = match target {
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
                let merges = matches!(verb.as_str(), "read" | "search" | "fetch" | "recall" | "browse");
                match run.last_mut() {
                    Some(line) if merges && line.verb == verb => {
                        if !target.is_empty() && !line.targets.contains(&target) {
                            line.targets.push(target);
                        }
                    }
                    _ => run.push(ToolLine { verb, targets: if target.is_empty() { Vec::new() } else { vec![target] }, edits }),
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
                blocks.push(Block::Reply { text: text.clone(), first: !after_reply });
                after_reply = true;
            }
            Entry::Note { text } => blocks.push(Block::Note(text.clone())),
            Entry::Error { message, next } => blocks.push(Block::Error { message: message.clone(), next: next.clone() }),
            Entry::Landed { branch, stat } => blocks.push(Block::Landed { branch: branch.clone(), stat: stat.clone() }),
            Entry::Tool { .. } | Entry::Thread { .. } => {}
        }
        // Steps between two replies don't make the second a new turn.
        if !matches!(entry, Entry::Reply { .. }) {
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
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
    canon(path)
        .strip_prefix(canon(base))
        .ok()
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

/// Markdown with every ticket number outside code made a link: `T-8` becomes
/// `[T-8](ticket:8)`, which a click turns into the ticket on the board.
pub fn link_tickets(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len() + 16);
    let mut fenced = false;
    for (n, line) in markdown.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
            out.push_str(line);
        } else if fenced || !line.contains("T-") {
            out.push_str(line);
        } else {
            link_line(line, &mut out);
        }
    }
    out
}

fn link_line(line: &str, out: &mut String) {
    // Inline code is every second piece when split on backticks.
    for (i, piece) in line.split('`').enumerate() {
        if i > 0 {
            out.push('`');
        }
        if i % 2 == 1 {
            out.push_str(piece);
            continue;
        }
        let mut rest = piece;
        while let Some((at, len)) = ticket_ref(rest) {
            let mut before = &rest[..at];
            let mut after = &rest[at + len..];
            let id = &rest[at..at + len];
            // Not inside an existing link's text or target.
            if before.ends_with('[') || before.ends_with("](") {
                out.push_str(before);
                out.push_str(id);
                rest = after;
                continue;
            }
            // `**T-8**` links as a whole: a link closing right before a Korean
            // particle would otherwise leave the bold unclosed (CommonMark
            // wants a letter, not a bracket, before a closing `**`).
            let mut delim = "";
            for d in ["**", "__", "~~", "*", "_"] {
                if before.ends_with(d) && after.starts_with(d) {
                    delim = d;
                    break;
                }
            }
            before = &before[..before.len() - delim.len()];
            after = &after[delim.len()..];
            out.push_str(before);
            out.push_str(&format!("[{delim}{id}{delim}]({TICKET_SCHEME}{})", &id[2..]));
            rest = after;
        }
        out.push_str(rest);
    }
}

/// Which ticket a clicked link means.
pub fn ticket_of_link(url: &str) -> Option<u32> {
    url.strip_prefix(TICKET_SCHEME)?.parse().ok()
}

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
        assert_eq!(c.blocks(), vec![Block::User("hi".into()), Block::Reply { text: "Hello".into(), first: true }]);
        feed(&mut c, &[tool("Read", "/work/site/a.rs"), Event::TextDelta { session: 1, text: "Done.".into() }]);
        let blocks = c.blocks();
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[2], Block::Steps(vec![ToolLine { verb: "read".into(), targets: vec!["a.rs".into()], edits: false }]));
        assert_eq!(blocks[3], Block::Reply { text: "Done.".into(), first: false }, "steps between replies are one turn");
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
    fn a_tool_line_counts_the_targets_past_three() {
        let line = ToolLine { verb: "read".into(), targets: (1..=5).map(|n| format!("f{n}")).collect(), edits: false };
        assert_eq!(line.text(), "f1, f2, f3, +2");
    }

    #[test]
    fn a_long_run_folds_to_three_then_two_and_unfolds() {
        let run: Vec<ToolLine> = (0..10).map(|n| ToolLine { verb: "run".into(), targets: vec![format!("c{n}")], edits: false }).collect();
        let rows = run_rows(&run, false);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[3], RunRow::Folded(5));
        assert_eq!(run_rows(&run, true).len(), 10);
        assert_eq!(run_rows(&run[..6], false).len(), 6, "six still fits");
    }

    #[test]
    fn the_pm_shows_the_thread_of_the_project_in_focus() {
        let mut c = convo(true);
        feed(
            &mut c,
            &[
                Event::UserMessage { session: 1, text: "whole squad".into() },
                Event::Thread { session: 1, project: Some("site".into()) },
                Event::UserMessage { session: 1, text: "about the site".into() },
                Event::Thread { session: 1, project: Some("app".into()) },
                Event::UserMessage { session: 1, text: "about the app".into() },
            ],
        );
        let texts = |c: &Convo| c.blocks().iter().filter_map(|b| if let Block::User(t) = b { Some(t.clone()) } else { None }).collect::<Vec<_>>();
        c.thread_view = None;
        assert_eq!(texts(&c), ["whole squad"]);
        c.thread_view = Some("site".into());
        assert_eq!(texts(&c), ["about the site"]);
        c.thread_view = Some("app".into());
        assert_eq!(texts(&c), ["about the app"]);
    }

    #[test]
    fn a_members_conversation_says_what_other_projects_left_out() {
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
        c.thread_view = Some("site".into());
        let notes: Vec<String> = c.blocks().into_iter().filter_map(|b| if let Block::Note(t) = b { Some(t) } else { None }).collect();
        assert_eq!(notes[0], "── T-1 · Hero");
        assert!(notes[1].contains("티켓 1개"), "{notes:?}");
        // With every project in view, tickets are headed with theirs.
        c.thread_view = None;
        let notes: Vec<String> = c.blocks().into_iter().filter_map(|b| if let Block::Note(t) = b { Some(t) } else { None }).collect();
        assert_eq!(notes[0], "── site · T-1 · Hero");
    }

    #[test]
    fn a_turn_leaves_how_long_it_took() {
        let mut c = convo(true);
        let t0 = Instant::now();
        c.on_event(&Event::UserMessage { session: 1, text: "go".into() }, t0, T);
        c.on_event(&Event::TextDelta { session: 1, text: "ok".into() }, t0, T);
        c.on_event(&Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None }, t0 + Duration::from_secs(65), T);
        assert!(matches!(c.blocks().last(), Some(Block::Note(t)) if t == "─ worked 1m 05s · 14:02"), "{:?}", c.blocks());
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
    fn memos_show_only_in_the_project_they_were_left_in() {
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
        c.thread_view = Some("site".into());
        assert_eq!(memos(&c), ["for site"]);
        c.thread_view = Some("app".into());
        assert_eq!(memos(&c), ["for app"]);
        // A member's whole-squad view has them all.
        c.thread_view = None;
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
    fn ticket_numbers_become_links_outside_code() {
        assert_eq!(link_tickets("see T-8 and T-12."), "see [T-8](ticket:8) and [T-12](ticket:12).");
        assert_eq!(link_tickets("**T-9**부터"), "[**T-9**](ticket:9)부터", "bold goes inside the link");
        assert_eq!(link_tickets("run `T-8` now"), "run `T-8` now", "inline code stays");
        assert_eq!(link_tickets("```\nT-8\n```\nT-9"), "```\nT-8\n```\n[T-9](ticket:9)", "fenced code stays");
        assert_eq!(link_tickets("[T-8](ticket:8)"), "[T-8](ticket:8)", "already a link");
        assert_eq!(link_tickets("MAT-8 and T-"), "MAT-8 and T-", "only whole words");
        assert_eq!(ticket_of_link("ticket:8"), Some(8));
        assert_eq!(ticket_of_link("https://x"), None);
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
        assert!(c.blocks().iter().any(|b| matches!(b, Block::Note(t) if t.starts_with("─ worked 3s"))), "{:?}", c.blocks());
    }

    #[test]
    fn tool_verbs_read_as_what_they_do() {
        assert_eq!(tool_verb("Bash"), "run");
        assert_eq!(tool_verb("mcp__equipe__submit_for_review"), "hand in");
        assert_eq!(tool_verb("mcp__browser__browser_click"), "browse");
        assert_eq!(tool_verb("Odd"), "Odd");
    }
}
