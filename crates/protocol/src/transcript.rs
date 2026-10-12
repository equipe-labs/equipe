//! Folding events into a conversation — the same way in the journal and on
//! every screen, so a screen that attaches later sees exactly what a screen that
//! watched live saw.

use crate::{Entry, Event, TurnOutcome};

/// Keep at most this many lines in memory; older ones live in the journal.
pub const IN_MEMORY_LINES: usize = 500;

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
    /// Streaming text joins the open reply and is final only once something else
    /// happens (a tool use, the turn ending, an error).
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
        // A memo waits outside the conversation until it is delivered.
        if let Event::MessageQueued { .. } = event {
            return Vec::new();
        }

        let mut done = Vec::new();
        if self.open_reply {
            self.open_reply = false;
            if let Some(reply) = self.entries.last() {
                done.push(reply.clone());
            }
        }
        let new = match event {
            Event::SessionStarted { workspace: Some(ws), .. } => Some(Entry::Note {
                text: format!(
                    "Working on branch {} in its own worktree. Your checkout stays untouched; \
closing the session lands the branch.",
                    ws.branch
                ),
            }),
            Event::UserMessage { text, .. } => Some(Entry::User { text: text.clone() }),
            Event::Notice { text, .. } => Some(Entry::Note { text: text.clone() }),
            Event::WorkStarted { ticket, title, workspace, .. } => Some(Entry::Note {
                text: match workspace {
                    Some(ws) => format!("── T-{ticket} · {title} — on branch {}", ws.branch),
                    None => format!("── T-{ticket} · {title}"),
                },
            }),
            Event::ToolUse { tool, target, .. } => Some(Entry::Tool { tool: tool.clone(), target: target.clone() }),
            Event::TurnEnded { outcome: TurnOutcome::Interrupted, .. } => Some(Entry::Note { text: "Stopped.".into() }),
            Event::TurnEnded { outcome: TurnOutcome::Incomplete, .. } => {
                Some(Entry::Note { text: "The engine ended early.".into() })
            }
            Event::Error { message, next, .. } => Some(Entry::Error { message: message.clone(), next: next.clone() }),
            Event::BranchLanded { branch, stat, .. } => Some(Entry::Landed { branch: branch.clone(), stat: stat.clone() }),
            Event::NothingChanged { .. } => Some(Entry::Note { text: "No files changed, so no branch was kept.".into() }),
            _ => None,
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Usage;

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
            usage: Some(Usage { input_tokens: Some(1), output_tokens: Some(1) }),
        });
        assert_eq!(done, vec![Entry::Reply { text: "Hello".into() }]);
    }

    #[test]
    fn a_tool_use_closes_the_reply_and_starts_a_new_one_after() {
        let mut t = Transcript::default();
        t.apply(&delta("Let me look."));
        let done = t.apply(&Event::ToolUse { session: 1, tool: "Read".into(), target: Some("a.rs".into()) });
        assert_eq!(done, vec![Entry::Reply { text: "Let me look.".into() }, Entry::Tool { tool: "Read".into(), target: Some("a.rs".into()) }]);
        t.apply(&delta("Done."));
        assert_eq!(t.entries().len(), 3);
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
        assert_eq!(t.apply(&Event::MessageQueued { session: 1, text: "also this".into() }), Vec::<Entry>::new());
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
