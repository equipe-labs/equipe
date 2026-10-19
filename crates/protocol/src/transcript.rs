//! Folding events into a conversation — the same way in the journal and on
//! every screen, so a screen that attaches later sees exactly what a screen that
//! watched live saw.

use crate::{Entry, Event, TurnOutcome};
#[cfg(test)]
use crate::ToolOutcome;

/// The lines the transcript writes itself, as they are kept and sent — in
/// English; a screen shows them in the person's language
/// (`equipe_core::i18n::note`).
pub mod notes {
    pub const STOPPED: &str = "Stopped.";
    pub const ENDED_EARLY: &str = "The engine ended early.";
    pub const NOTHING_CHANGED: &str = "No files changed, so no branch was kept.";
    /// Before the branch's name.
    pub const WORKING_ON: &str = "Working on branch ";
    /// After it.
    pub const WORKING_REST: &str = " in its own worktree. Your checkout stays untouched; closing the session lands the branch.";
    /// Between a ticket's header (`── T-12 · title`) and its branch.
    pub const ON_BRANCH: &str = " — on branch ";
}

/// Keep at most this many lines in memory; older ones live in the journal.
pub const IN_MEMORY_LINES: usize = 500;
/// How far back a tool's answer looks for its call (parallel calls answer
/// out of order, but not that far apart).
const TOOL_LOOKBACK: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transcript {
    entries: Vec<Entry>,
    /// The last entry is a reply still streaming.
    open_reply: bool,
}

impl Transcript {
    pub fn from_entries(entries: Vec<Entry>) -> Self {
        let mut t = Self { entries, open_reply: false };
        t.cap();
        t
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Add a line only this copy shows — a screen's own help text, say. It is
    /// not an event, so no journal or other screen ever sees it.
    pub fn push_local(&mut self, entry: Entry) {
        self.open_reply = false;
        self.entries.push(entry);
        self.cap();
    }

    /// Apply one event. Returns the lines it made final — what a journal stores.
    /// Streaming text joins the open reply and is final only once a line comes
    /// after it (a tool use, a message, news, an error) or the turn ends.
    pub fn apply(&mut self, event: &Event) -> Vec<Entry> {
        if let Event::TextDelta { text, .. } = event {
            match (self.open_reply, self.entries.last_mut()) {
                (true, Some(Entry::Reply { text: reply })) => reply.push_str(text),
                _ => {
                    self.entries.push(Entry::Reply { text: text.clone() });
                    self.open_reply = true;
                }
            }
            return Vec::new();
        }
        // A memo waits outside the conversation until it is delivered;
        // reasoning is never in it.
        if let Event::MessageQueued { .. } | Event::Thinking { .. } = event {
            return Vec::new();
        }
        // A tool's answer goes onto its line, which was final already: the
        // outcome lives in memory, for the screens, not in the journal.
        if let Event::ToolResult { tool: name, target: on, outcome, .. } = event {
            // One run in the background answers long after, past any lookback.
            let open = self.entries.iter_mut().rev().enumerate().find_map(|(back, entry)| match entry {
                Entry::Tool { tool, target, outcome: slot, background, .. } if tool == name && target == on && slot.is_none() && (back < TOOL_LOOKBACK || *background) => Some(slot),
                _ => None,
            });
            if let Some(slot) = open {
                *slot = Some(Box::new(outcome.clone()));
            }
            return Vec::new();
        }

        let new = match event {
            Event::SessionStarted { workspace: Some(ws), .. } => {
                Some(Entry::Note { text: format!("{}{}{}", notes::WORKING_ON, ws.branch, notes::WORKING_REST) })
            }
            // A message sent while it worked, where it was read.
            Event::UserMessage { text, .. } | Event::Steered { text, .. } => Some(Entry::User { text: text.clone() }),
            Event::Notice { text, .. } => Some(Entry::Note { text: text.clone() }),
            Event::Thread { project, .. } => Some(Entry::Thread { project: project.clone() }),
            Event::WorkStarted { ticket, title, workspace, .. } => Some(Entry::Note {
                text: match workspace {
                    Some(ws) => format!("── T-{ticket} · {title}{}{}", notes::ON_BRANCH, ws.branch),
                    None => format!("── T-{ticket} · {title}"),
                },
            }),
            Event::ToolUse { tool, target, label, background, .. } => Some(Entry::Tool { tool: tool.clone(), target: target.clone(), outcome: None, label: label.clone(), background: *background }),
            Event::TurnEnded { outcome: TurnOutcome::Interrupted, .. } => Some(Entry::Note { text: notes::STOPPED.into() }),
            Event::TurnEnded { outcome: TurnOutcome::Incomplete, .. } => {
                Some(Entry::Note { text: notes::ENDED_EARLY.into() })
            }
            Event::Error { message, next, .. } => Some(Entry::Error { message: message.clone(), next: next.clone() }),
            Event::BranchLanded { branch, stat, .. } => Some(Entry::Landed { branch: branch.clone(), stat: stat.clone() }),
            Event::NothingChanged { .. } => Some(Entry::Note { text: notes::NOTHING_CHANGED.into() }),
            _ => None,
        };
        // The reply ends where a line comes after it, or the turn ends. What
        // adds no line — a memo said meanwhile, a token count, a subagent's
        // step, a picture of the browser — leaves it streaming: split there,
        // its next chunk was a reply of its own, a line break mid-word.
        let ends = new.is_some() || matches!(event, Event::TurnEnded { .. } | Event::SessionClosed { .. } | Event::WorkEnded { .. });
        let mut done = Vec::new();
        if self.open_reply && ends {
            self.open_reply = false;
            if let Some(reply) = self.entries.last() {
                done.push(reply.clone());
            }
        }
        if let Some(entry) = new {
            self.entries.push(entry.clone());
            done.push(entry);
        }
        self.cap();
        done
    }

    fn cap(&mut self) {
        if self.entries.len() > IN_MEMORY_LINES {
            let extra = self.entries.len() - IN_MEMORY_LINES;
            self.entries.drain(..extra);
        }
    }
}

/// What a conversation is about, line by line. The PM keeps one conversation
/// for the whole squad; each line of it is about one project — or, `None`, the
/// whole squad. A [`Entry::Thread`] marker says what the lines after it are
/// about: the daemon leaves one before a message of the person's about another
/// project than the last (the project they looked at when they sent it), and
/// around news of another project's ticket. The member's replies and steps
/// follow the message they answer. A screen shows one project's slice of it
/// ([`slice`]); the engine always gets the whole.
pub mod scope {
    use std::borrow::Borrow;
    use std::ops::Range;

    use crate::Entry;

    /// How the daemon opens a message the person sent looking at one project
    /// (the project's name follows, then ` — `).
    pub const LOOKING_AT: &str = "[equipe: the person is looking at the project ";
    /// How it opens one sent looking at the whole squad.
    pub const WHOLE_SQUAD: &str = "[equipe: the person is looking at the whole squad";

    /// What `entry` says the lines from it on are about, when it says: a
    /// marker, or (in older history) a message that kept the daemon's prefix.
    pub fn marker(entry: &Entry) -> Option<Option<String>> {
        match entry {
            Entry::Thread { project } => Some(project.clone()),
            Entry::User { text } => said_about(text),
            _ => None,
        }
    }

    /// The project a message names in the daemon's prefix, if it kept one.
    pub fn said_about(text: &str) -> Option<Option<String>> {
        if let Some(rest) = text.strip_prefix(LOOKING_AT) {
            let name = rest.split_once(" — ").map_or_else(|| rest.split(']').next().unwrap_or_default(), |(name, _)| name).trim();
            return (!name.is_empty()).then(|| Some(name.to_owned()));
        }
        text.starts_with(WHOLE_SQUAD).then_some(None)
    }

    /// What each line is about, in order. Lines before any marker are about
    /// the whole squad.
    pub fn of(entries: &[Entry]) -> Vec<Option<String>> {
        let mut now: Option<String> = None;
        entries
            .iter()
            .map(|entry| {
                if let Some(about) = marker(entry) {
                    now = about;
                }
                now.clone()
            })
            .collect()
    }

    /// What the conversation is about at its end, if a marker said so.
    pub fn current(entries: &[Entry]) -> Option<Option<String>> {
        entries.iter().rev().find_map(marker)
    }

    /// The lines a screen sends from `from` on, said about what they are about
    /// where they start: the marker in force there goes first, so a screen
    /// that gets only the latest lines reads them as the daemon does.
    pub fn from(entries: &[Entry], from: usize) -> Vec<Entry> {
        let from = from.min(entries.len());
        let mut out = Vec::with_capacity(entries.len() - from + 1);
        let opens_with_marker = entries.get(from).is_some_and(|e| matches!(e, Entry::Thread { .. }));
        if !opens_with_marker && let Some(about) = current(&entries[..from]) {
            out.push(Entry::Thread { project: about });
        }
        out.extend_from_slice(&entries[from..]);
        out
    }

    /// A stretch of the conversation as one project's slice shows it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Part {
        /// Lines about the project: shown.
        Here(Range<usize>),
        /// Lines about other projects or the whole squad, folded into one row
        /// saying how many things were said there. `kept`: lines of the
        /// project's own within it — news only, that would otherwise split
        /// the row in several — shown after the row while it is folded.
        Elsewhere { lines: Range<usize>, said: usize, kept: Vec<Range<usize>> },
    }

    /// Whether a line is something said — a message, a reply, news, an error,
    /// a commit — rather than a step, a marker or a screen's own line (those
    /// open with a private-use character).
    pub fn is_said(entry: &Entry) -> bool {
        match entry {
            Entry::User { .. } | Entry::Reply { .. } | Entry::Error { .. } | Entry::Landed { .. } => true,
            Entry::Note { text } => !text.chars().next().is_some_and(|c| ('\u{E000}'..='\u{F8FF}').contains(&c)),
            Entry::Tool { .. } | Entry::Thread { .. } => false,
        }
    }

    /// `lines` (each with what it is about) as `project`'s slice shows them:
    /// its own lines, and between them the rest folded. A reply about the
    /// whole squad still streaming at the end (`live`) shows too: the person
    /// is waiting for it. Folds with nothing said in them are left out.
    pub fn slice<E: Borrow<Entry>>(lines: &[(E, Option<String>)], project: &str, live: bool) -> Vec<Part> {
        let last = lines.len().checked_sub(1);
        let here = |ix: usize| {
            let (entry, about) = &lines[ix];
            about.as_deref() == Some(project) || (live && Some(ix) == last && about.is_none() && matches!(entry.borrow(), Entry::Reply { .. }))
        };
        let mut parts: Vec<Part> = Vec::new();
        let mut ix = 0;
        while ix < lines.len() {
            let shown = here(ix);
            let start = ix;
            while ix < lines.len() && here(ix) == shown {
                ix += 1;
            }
            if shown {
                parts.push(Part::Here(start..ix));
            } else {
                let said = lines[start..ix].iter().filter(|(e, _)| is_said(e.borrow())).count();
                if said > 0 {
                    parts.push(Part::Elsewhere { lines: start..ix, said, kept: Vec::new() });
                }
            }
        }
        // Folds with only the project's news between them are one fold: a
        // row each, a news line between, read as clutter.
        let news_only = |range: &Range<usize>| lines[range.clone()].iter().all(|(e, _)| !matches!(e.borrow(), Entry::User { .. } | Entry::Reply { .. } | Entry::Error { .. } | Entry::Landed { .. }));
        let mut merged: Vec<Part> = Vec::with_capacity(parts.len());
        let mut rest = parts.into_iter().peekable();
        while let Some(part) = rest.next() {
            let between = match (merged.last(), &part) {
                (Some(Part::Elsewhere { .. }), Part::Here(range)) if news_only(range) && matches!(rest.peek(), Some(Part::Elsewhere { .. })) => Some(range.clone()),
                _ => None,
            };
            let Some(between) = between else {
                merged.push(part);
                continue;
            };
            let Some(Part::Elsewhere { lines: next, said: more, kept: next_kept }) = rest.next() else { unreachable!("peeked") };
            if let Some(Part::Elsewhere { lines: range, said, kept }) = merged.last_mut() {
                range.end = next.end;
                *said += more;
                kept.push(between);
                kept.extend(next_kept);
            }
        }
        merged
    }
}

/// A conversation as Markdown, for a file the person can search and copy —
/// the same text on every screen that exports one.
pub fn markdown(entries: &[Entry], heading: &str) -> String {
    let mut out = format!("# {heading}\n\n");
    for entry in entries {
        match entry {
            Entry::User { text } => out.push_str(&format!("**You:** {text}\n\n")),
            Entry::Reply { text } => out.push_str(&format!("{text}\n\n")),
            Entry::Tool { tool, target, .. } => {
                out.push_str(&format!("- {} {}\n", tool_verb(tool), target.as_deref().unwrap_or("")));
            }
            Entry::Note { text } => out.push_str(&format!("_{text}_\n\n")),
            Entry::Thread { project } => out.push_str(&format!("## {}\n\n", project.as_deref().unwrap_or("The whole squad"))),
            Entry::Error { message, next } => {
                out.push_str(&format!("> ✗ {message}{}\n\n", next.as_ref().map(|n| format!(" → `{n}`")).unwrap_or_default()))
            }
            Entry::Landed { branch, stat } => out.push_str(&format!("✓ Landed on `{branch}`\n\n```\n{stat}\n```\n\n")),
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Usage;

    mod scopes {
        use super::super::scope::{self, Part};
        use crate::Entry;

        fn user(text: &str) -> Entry {
            Entry::User { text: text.into() }
        }
        fn reply(text: &str) -> Entry {
            Entry::Reply { text: text.into() }
        }
        fn about(project: Option<&str>) -> Entry {
            Entry::Thread { project: project.map(str::to_owned) }
        }
        fn tool() -> Entry {
            Entry::Tool { tool: "Read".into(), target: None, outcome: None, label: None, background: false }
        }

        #[test]
        fn a_turn_its_reply_and_its_steps_are_about_the_project_it_was_sent_in() {
            let lines = vec![user("old"), reply("old answer"), about(Some("app")), user("fix it"), tool(), reply("fixed"), about(Some("site")), user("and here")];
            let s = scope::of(&lines);
            assert_eq!(s[..2], [None, None], "before any marker: the whole squad");
            assert!(s[2..6].iter().all(|p| p.as_deref() == Some("app")), "{s:?}");
            assert_eq!(s[7].as_deref(), Some("site"));
            assert_eq!(scope::current(&lines), Some(Some("site".into())));
            assert_eq!(scope::current(&lines[..2]), None, "no marker yet");
        }

        #[test]
        fn news_is_about_its_ticket_and_the_turn_goes_on_after_it() {
            let news = Entry::Note { text: "frontend handed in T-4".into() };
            let lines = vec![about(Some("app")), user("go"), reply("on it"), about(Some("site")), news, about(Some("app")), reply("done")];
            let s = scope::of(&lines);
            assert_eq!(s[4].as_deref(), Some("site"));
            assert_eq!(s[6].as_deref(), Some("app"), "the reply after the news is still the turn's");
        }

        #[test]
        fn older_history_is_read_from_the_prefix_it_kept_else_the_whole_squad() {
            let lines = vec![user("plain"), user("[equipe: the person is looking at the project web — new work is for it unless they say otherwise]\n\nhi"), reply("hello"), user("[equipe: the person is looking at the whole squad — you oversee every project.]\n\nall")];
            let s = scope::of(&lines);
            assert_eq!(s, vec![None, Some("web".into()), Some("web".into()), None]);
            assert_eq!(scope::said_about("[equipe: the person is looking at the project x]"), Some(Some("x".into())));
            assert_eq!(scope::said_about("nothing"), None);
        }

        #[test]
        fn the_latest_lines_carry_the_marker_in_force_where_they_start() {
            let lines = vec![about(Some("app")), user("a"), reply("b"), user("c")];
            assert_eq!(scope::from(&lines, 2), vec![about(Some("app")), reply("b"), user("c")]);
            assert_eq!(scope::from(&lines, 0), lines, "a marker first already");
            assert_eq!(scope::from(&[user("x"), reply("y")], 1), vec![reply("y")], "no marker, nothing to add");
        }

        #[test]
        fn a_slice_shows_its_project_and_folds_the_rest_counting_what_was_said() {
            let lines = vec![
                about(Some("app")), user("a1"), tool(), reply("a2"),
                about(Some("site")), user("s1"), tool(), tool(), reply("s2"),
                about(None), user("all"), reply("all answer"),
                about(Some("app")), user("a3"), reply("a4"),
            ];
            let s = scope::of(&lines);
            let pairs: Vec<(&Entry, Option<String>)> = lines.iter().zip(s).collect();
            assert_eq!(
                scope::slice(&pairs, "app", false),
                vec![Part::Here(0..4), Part::Elsewhere { lines: 4..12, said: 4, kept: vec![] }, Part::Here(12..15)],
                "site's turn and the squad-wide one fold together: four things said"
            );
            assert_eq!(scope::slice(&pairs, "site", false), vec![Part::Elsewhere { lines: 0..4, said: 2, kept: vec![] }, Part::Here(4..9), Part::Elsewhere { lines: 9..15, said: 4, kept: vec![] }]);
            assert_eq!(scope::slice(&pairs, "web", false), vec![Part::Elsewhere { lines: 0..15, said: 8, kept: vec![] }]);
        }

        #[test]
        fn folds_with_only_the_projects_news_between_are_one() {
            let news = |t: &str| Entry::Note { text: t.into() };
            let lines = vec![
                about(Some("app")), user("a1"), reply("a2"),
                about(Some("site")), news("T-4 handed in"),
                about(Some("app")), user("a3"), reply("a4"),
                about(Some("site")), news("T-5 started"), news("T-6 started"),
                about(None), user("all"), reply("all answer"),
                about(Some("site")), user("s1"), reply("s2"),
            ];
            let s = scope::of(&lines);
            let pairs: Vec<(&Entry, Option<String>)> = lines.iter().zip(s).collect();
            assert_eq!(
                scope::slice(&pairs, "site", false),
                vec![Part::Elsewhere { lines: 0..14, said: 6, kept: vec![3..5, 8..11] }, Part::Here(14..17)],
                "one row for the others, the site's news after it"
            );
            // Something said between them keeps them apart.
            let lines = vec![about(Some("app")), user("a1"), about(Some("site")), user("s1"), about(Some("app")), user("a2")];
            let s = scope::of(&lines);
            let pairs: Vec<(&Entry, Option<String>)> = lines.iter().zip(s).collect();
            assert_eq!(scope::slice(&pairs, "site", false).len(), 3);
        }

        #[test]
        fn a_squad_wide_reply_streaming_now_shows_in_any_slice() {
            let lines = vec![about(Some("app")), user("a"), reply("b"), about(None), user("all?"), reply("stream")];
            let s = scope::of(&lines);
            let pairs: Vec<(&Entry, Option<String>)> = lines.iter().zip(s).collect();
            assert_eq!(scope::slice(&pairs, "app", true), vec![Part::Here(0..3), Part::Elsewhere { lines: 3..5, said: 1, kept: vec![] }, Part::Here(5..6)]);
            assert_eq!(scope::slice(&pairs, "app", false), vec![Part::Here(0..3), Part::Elsewhere { lines: 3..6, said: 2, kept: vec![] }]);
            // Another project's reply streaming stays folded.
            let lines = vec![about(Some("site")), user("s"), reply("stream")];
            let pairs: Vec<(&Entry, Option<String>)> = lines.iter().zip(scope::of(&lines)).collect();
            assert_eq!(scope::slice(&pairs, "app", true), vec![Part::Elsewhere { lines: 0..3, said: 2, kept: vec![] }]);
        }

        #[test]
        fn a_fold_with_nothing_said_is_left_out() {
            let lines = vec![about(Some("site")), tool(), Entry::Note { text: "\u{E001}3:10:00@1".into() }, about(Some("app")), user("a")];
            let pairs: Vec<(&Entry, Option<String>)> = lines.iter().zip(scope::of(&lines)).collect();
            assert_eq!(scope::slice(&pairs, "app", false), vec![Part::Here(3..5)]);
        }
    }

    fn delta(text: &str) -> Event {
        Event::TextDelta { session: 1, text: text.into() }
    }

    #[test]
    fn streaming_joins_one_reply_and_is_final_when_the_turn_ends() {
        let mut t = Transcript::default();
        assert!(t.apply(&Event::UserMessage { session: 1, text: "hi".into() }).len() == 1);
        assert!(t.apply(&delta("Hel")).is_empty());
        assert!(t.apply(&delta("lo")).is_empty());
        assert_eq!(t.entries().last(), Some(&Entry::Reply { text: "Hello".into() }));
        let done = t.apply(&Event::TurnEnded {
            session: 1,
            outcome: TurnOutcome::Completed,
            usage: Some(Usage { input_tokens: Some(1), output_tokens: Some(1), ..Default::default() }),
        });
        assert_eq!(done, vec![Entry::Reply { text: "Hello".into() }]);
    }

    #[test]
    fn a_tool_use_closes_the_reply_and_starts_a_new_one_after() {
        let mut t = Transcript::default();
        t.apply(&delta("Let me look."));
        let done = t.apply(&Event::ToolUse { session: 1, tool: "Read".into(), target: Some("a.rs".into()), label: None, background: false });
        assert_eq!(done, vec![Entry::Reply { text: "Let me look.".into() }, Entry::Tool { tool: "Read".into(), target: Some("a.rs".into()), outcome: None, label: None, background: false }]);
        t.apply(&delta("Done."));
        assert_eq!(t.entries().len(), 3);
    }

    #[test]
    fn what_adds_no_line_leaves_the_reply_streaming_whole() {
        // A memo said while it answers, a token count, a subagent's step, a
        // picture of the browser: none is a line, so the reply goes on — split
        // there, its next chunk was a reply of its own, as a line break in the
        // middle of a word ("지" / "금 보드를").
        let quiet = [
            Event::SteerQueued { session: 1, text: "그리고 하나 더".into(), project: None },
            Event::Output { session: 1, tokens: 12 },
            Event::ModelKnown { session: 1, model: "claude-opus-5-5".into() },
            Event::Suggestion { session: 1, text: "다음".into() },
            Event::SubagentStep { session: 1, id: "a".into(), step: "Read a.rs".into() },
            Event::Estimated { session: 1, estimate: None },
            Event::BrowserFrame { session: 1, image: String::new(), url: None },
            Event::QueueSent { session: 1, texts: vec![] },
        ];
        for event in &quiet {
            let mut t = Transcript::default();
            t.apply(&delta("네, 확인했습니다. 지"));
            assert!(t.apply(event).is_empty(), "{event:?} makes nothing final");
            t.apply(&delta("금 보드를 살펴보겠습니다."));
            assert_eq!(t.entries(), &[Entry::Reply { text: "네, 확인했습니다. 지금 보드를 살펴보겠습니다.".into() }], "after {event:?}");
            let done = t.apply(&Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None });
            assert_eq!(done, vec![Entry::Reply { text: "네, 확인했습니다. 지금 보드를 살펴보겠습니다.".into() }]);
        }
        // A line between ends it: a message read mid-turn, news.
        let mut t = Transcript::default();
        t.apply(&delta("first"));
        t.apply(&Event::Notice { session: 1, text: "frontend handed in T-4".into() });
        t.apply(&delta("second"));
        assert_eq!(t.entries().len(), 3);
    }

    #[test]
    fn a_steer_read_ends_the_reply_so_far() {
        let mut t = Transcript::default();
        t.apply(&delta("Look"));
        assert!(t.apply(&Event::SteerQueued { session: 1, text: "skip the footer".into(), project: None }).is_empty());
        assert!(t.apply(&Event::SubagentStep { session: 1, id: "a".into(), step: "Read x".into() }).is_empty());
        assert!(t.apply(&Event::ModelKnown { session: 1, model: "m".into() }).is_empty());
        t.apply(&delta("ing."));
        assert_eq!(t.entries(), &[Entry::Reply { text: "Looking.".into() }]);
        // Read where it was: the reply so far is final, the steer follows it.
        let done = t.apply(&Event::Steered { session: 1, text: "skip the footer".into() });
        assert_eq!(done, vec![Entry::Reply { text: "Looking.".into() }, Entry::User { text: "skip the footer".into() }]);
        // A token count leaves the next text in the same reply; the turn's end closes it.
        t.apply(&delta("Done."));
        assert!(t.apply(&Event::Output { session: 1, tokens: 40 }).is_empty());
        t.apply(&delta(" Also."));
        let done = t.apply(&Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None });
        assert_eq!(done, vec![Entry::Reply { text: "Done. Also.".into() }]);
    }

    #[test]
    fn a_tool_result_lands_on_its_call_without_splitting_the_reply() {
        let mut t = Transcript::default();
        let call = |target: &str| Event::ToolUse { session: 1, tool: "Bash".into(), target: Some(target.into()), label: None, background: false };
        let result = |target: &str, ok: bool| Event::ToolResult {
            session: 1,
            tool: "Bash".into(),
            target: Some(target.into()),
            outcome: ToolOutcome { ok, exit: (!ok).then_some(1), ..Default::default() },
        };
        t.apply(&call("ls"));
        t.apply(&call("make"));
        // Parallel calls may answer out of order.
        assert!(t.apply(&result("make", false)).is_empty());
        t.apply(&delta("So far"));
        assert!(t.apply(&result("ls", true)).is_empty());
        t.apply(&delta(" good."));
        let outcome = |i: usize| match &t.entries()[i] {
            Entry::Tool { outcome, .. } => outcome.as_ref().map(|o| (o.ok, o.exit)),
            _ => panic!(),
        };
        assert_eq!(outcome(0), Some((true, None)));
        assert_eq!(outcome(1), Some((false, Some(1))));
        assert_eq!(t.entries()[2], Entry::Reply { text: "So far good.".into() }, "the reply goes on");
        // An answer with no call is dropped.
        t.apply(&result("rm", true));
        assert_eq!(t.entries().len(), 3);
    }

    #[test]
    fn output_keeps_its_end() {
        let text: String = (0..300).map(|n| format!("line {n}\n")).collect();
        let (tail, skipped) = ToolOutcome::tail(&text);
        assert_eq!(skipped, 100);
        assert_eq!(tail.lines().count(), ToolOutcome::OUTPUT_LINES);
        assert!(tail.starts_with("line 100") && tail.ends_with("line 299"));
        assert_eq!(ToolOutcome::tail("\n\n"), (String::new(), 0));
        let wide = "가".repeat(10_000);
        let (tail, skipped) = ToolOutcome::tail(&wide);
        assert_eq!(skipped, 0);
        assert!(tail.starts_with('…') && tail.len() <= ToolOutcome::OUTPUT_BYTES + 4);
    }

    #[test]
    fn an_interrupted_turn_says_so() {
        let mut t = Transcript::default();
        t.apply(&delta("partial"));
        let done = t.apply(&Event::TurnEnded { session: 1, outcome: TurnOutcome::Interrupted, usage: None });
        assert_eq!(done, vec![Entry::Reply { text: "partial".into() }, Entry::Note { text: "Stopped.".into() }]);
    }

    #[test]
    fn a_memo_is_not_a_line_until_it_is_delivered() {
        let mut t = Transcript::default();
        t.apply(&delta("working"));
        assert_eq!(t.apply(&Event::MessageQueued { session: 1, text: "also this".into(), project: None }), Vec::<Entry>::new());
        t.apply(&delta(" still"));
        assert_eq!(t.entries(), &[Entry::Reply { text: "working still".into() }], "a memo doesn't split the reply");
    }

    #[test]
    fn a_notice_is_a_note() {
        let mut t = Transcript::default();
        let done = t.apply(&Event::Notice { session: 1, text: "↪ You → qa: check it".into() });
        assert_eq!(done, vec![Entry::Note { text: "↪ You → qa: check it".into() }]);
    }

    #[test]
    fn memory_keeps_only_the_latest_lines() {
        let mut t = Transcript::default();
        for i in 0..(IN_MEMORY_LINES + 20) {
            t.apply(&Event::UserMessage { session: 1, text: i.to_string() });
        }
        assert_eq!(t.entries().len(), IN_MEMORY_LINES);
        assert_eq!(t.entries().last(), Some(&Entry::User { text: (IN_MEMORY_LINES + 19).to_string() }));
    }
}
