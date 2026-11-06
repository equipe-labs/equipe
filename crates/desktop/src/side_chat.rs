//! Side chats, as the screen keeps and reads them (`views/side_chat.rs`
//! draws them): a passage of a member's reply talked about beside the
//! conversation, the way Claude's apps "Start a side chat". The daemon runs
//! it on the member's engine, read-only, searching the web where the engine
//! can (`equipe_core::side_chat`); nothing said in it reaches the
//! conversation. Here: what one side chat holds, how its steps read
//! ("Searched", the pages found, "Read"), its last answer for "Bring to
//! main", and how wide its panel is.

use equipe_core::protocol::{Entry, Event, SessionId, ToolOutcome, Transcript};
#[cfg(test)]
use equipe_core::protocol::TurnOutcome;

/// One side chat, as the screen knows it.
#[derive(Debug, Clone, PartialEq)]
pub struct SideConvo {
    /// The conversation it is about.
    pub session: SessionId,
    pub passage: String,
    /// Whether it searches the web, once the daemon said (`None`: not yet).
    pub search: Option<bool>,
    pub transcript: Transcript,
    /// A turn of it is under way.
    pub working: bool,
    /// The daemon let it go (closed, or idle): its words stay here; the next
    /// message starts it over.
    pub gone: bool,
}

impl SideConvo {
    pub fn new(session: SessionId, passage: impl Into<String>) -> Self {
        Self { session, passage: passage.into(), search: None, transcript: Transcript::default(), working: false, gone: false }
    }

    /// One of its events ([`Event::SideChat`]'s `event`).
    pub fn on_event(&mut self, event: &Event) {
        match event {
            Event::UserMessage { .. } => self.working = true,
            Event::TurnEnded { .. } => self.working = false,
            _ => {}
        }
        self.transcript.apply(event);
    }

    /// Its latest answer, for "Bring to main": the last reply after the
    /// person's last message, once that turn ended.
    pub fn last_answer(&self) -> Option<String> {
        if self.working {
            return None;
        }
        self.transcript.entries().iter().rev().take_while(|e| !matches!(e, Entry::User { .. })).find_map(|e| match e {
            Entry::Reply { text } if !text.trim().is_empty() => Some(text.trim().to_owned()),
            _ => None,
        })
    }

    /// Whether anything was said in it yet.
    pub fn said_anything(&self) -> bool {
        self.transcript.entries().iter().any(|e| matches!(e, Entry::User { .. }))
    }
}

/// An answer's Markdown as its words read, for a quote over the composer —
/// as words selected in a reply are: bold, italic underscores and code
/// marks taken off; lists and lines kept.
pub fn plain(markdown: &str) -> String {
    markdown.replace("**", "").replace("__", "").replace('`', "")
}

/// A page a web search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub title: String,
    pub url: String,
}

impl Source {
    /// Where it is, as a person reads it: `blog.rust-lang.org`.
    pub fn site(&self) -> String {
        site(&self.url)
    }
}

/// A URL's host, without `www.`.
pub fn site(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.strip_prefix("www.").unwrap_or(host).to_owned()
}

/// The pages a search's outcome lists, one Markdown link a line
/// (`- [title](url)`, as the daemon writes them).
pub fn sources(output: &str) -> Vec<Source> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim().strip_prefix("- ").unwrap_or(line.trim());
            let (title, rest) = line.strip_prefix('[')?.split_once("](")?;
            let url = rest.strip_suffix(')')?;
            url.starts_with("http").then(|| Source { title: title.trim().to_owned(), url: url.to_owned() })
        })
        .collect()
}

/// A side chat's step, as its panel says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A web search: what it looked for, and the pages it found.
    Searched { query: String, sources: Vec<Source> },
    /// A page read.
    Read { url: String },
    /// A look at the member's files: the tool, and what it looked at.
    Looked { tool: String, target: String },
}

/// A tool line of a side chat, as a step. A Codex search that opened a page
/// names the page instead of words.
pub fn step(tool: &str, target: Option<&str>, outcome: Option<&ToolOutcome>) -> Step {
    let target = target.unwrap_or_default().to_owned();
    match tool {
        "WebFetch" => Step::Read { url: target },
        "WebSearch" if target.starts_with("http") => Step::Read { url: target },
        "WebSearch" => Step::Searched { query: target, sources: outcome.map(|o| sources(&o.output)).unwrap_or_default() },
        _ => Step::Looked { tool: tool.to_owned(), target },
    }
}

/// The panel at its widest, and at its narrowest beside the conversation.
pub const WIDEST: f32 = 420.;
pub const NARROWEST: f32 = 300.;
/// Under this much room, the conversation would be too narrow beside it:
/// the panel covers the conversation instead, whole.
pub const BESIDE_LEAST: f32 = 680.;

/// How wide the panel is in `room` (the conversation's pane, panel
/// included), and whether it covers the conversation rather than stand
/// beside it.
pub fn width(room: f32) -> (f32, bool) {
    if room < BESIDE_LEAST {
        return (room.max(0.), true);
    }
    ((room * 0.4).clamp(NARROWEST, WIDEST), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(convo: &mut SideConvo, events: &[Event]) {
        for e in events {
            convo.on_event(e);
        }
    }

    #[test]
    fn it_works_from_the_persons_message_to_the_turns_end_and_its_answer_is_the_last_reply() {
        let mut c = SideConvo::new(1, "the header");
        assert!(!c.said_anything() && c.last_answer().is_none());
        said(&mut c, &[Event::UserMessage { session: 9, text: "why?".into(), kept: Vec::new() }, Event::TextDelta { session: 9, text: "Because ".into() }]);
        assert!(c.working && c.last_answer().is_none(), "not while it answers");
        said(&mut c, &[
            Event::ToolUse { session: 9, tool: "WebSearch".into(), target: Some("q".into()), label: None, background: false },
            Event::TextDelta { session: 9, text: "it was **first**.".into() },
            Event::TurnEnded { session: 9, outcome: TurnOutcome::Completed, usage: None },
        ]);
        assert!(!c.working);
        assert_eq!(c.last_answer().as_deref(), Some("it was **first**."));
        said(&mut c, &[Event::UserMessage { session: 9, text: "and?".into(), kept: Vec::new() }, Event::TurnEnded { session: 9, outcome: TurnOutcome::Failed, usage: None }]);
        assert_eq!(c.last_answer(), None, "the last message has no answer");
    }

    #[test]
    fn an_answer_brought_to_main_reads_as_its_words_as_a_selection_would() {
        assert_eq!(plain("It goes **first**: `ends` is read __later__."), "It goes first: ends is read later.");
        assert_eq!(plain("- one\n- `two`"), "- one\n- two", "lists stay lists");
        assert_eq!(plain("a * b"), "a * b", "a lone star is a star");
    }

    #[test]
    fn a_search_reads_as_what_it_looked_for_and_the_pages_it_found() {
        let outcome = ToolOutcome { ok: true, exit: None, ms: None, output: "- [Rust Blog](https://blog.rust-lang.org/x)\n- [www](https://www.example.com/a?b)\nnot a link".into(), skipped: 0, diff: None };
        let Step::Searched { query, sources } = step("WebSearch", Some("rust 2027"), Some(&outcome)) else { panic!() };
        assert_eq!(query, "rust 2027");
        assert_eq!(sources.iter().map(|s| (s.title.as_str(), s.site())).collect::<Vec<_>>(), [("Rust Blog", "blog.rust-lang.org".to_owned()), ("www", "example.com".to_owned())]);
        assert_eq!(step("WebSearch", Some("q"), None), Step::Searched { query: "q".into(), sources: vec![] }, "still searching");
        assert_eq!(step("WebFetch", Some("https://a.dev/x"), None), Step::Read { url: "https://a.dev/x".into() });
        assert_eq!(step("WebSearch", Some("https://a.dev/x"), None), Step::Read { url: "https://a.dev/x".into() }, "Codex opening a page");
        assert_eq!(step("Read", Some("/p/a.rs"), None), Step::Looked { tool: "Read".into(), target: "/p/a.rs".into() });
    }

    #[test]
    fn the_panel_stands_beside_the_conversation_and_covers_it_when_the_window_is_narrow() {
        assert_eq!(width(1400.), (WIDEST, false));
        assert_eq!(width(800.), (320., false));
        assert_eq!(width(700.), (NARROWEST, false));
        assert_eq!(width(600.), (600., true), "too narrow for both: it covers the conversation");
        assert_eq!(width(-5.), (0., true));
    }
}
