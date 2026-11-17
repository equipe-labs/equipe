//! The person's messages on their way to the daemon, until it says it has
//! them. A message the composer sends is never just handed to the line and
//! forgotten: the line may be down (the daemon restarting, another Mac's SSH
//! link dropping), or drop it while it goes. So each one is kept here, shown
//! in its conversation as the person's own bubble, until its echo comes —
//! the daemon's word that it took it in ([`Event::UserMessage`], or the steer
//! or memo it became, or equipe's line that it answered a decision in words).
//!
//! - Not attached: it waits, and goes when the next hello comes, in the order
//!   it was said — a conversation it starts made first.
//! - Handed to a line that then went: the next hello's snapshot says whether
//!   it arrived; if not, it goes again. `Command::Say` carries the message's
//!   own key (`nonce`), the same every time, so a daemon that took it once
//!   takes it once.
//! - No word back in [`ANSWER_WAIT`], or refused: it stays on screen as not
//!   sent, for the person to send again (or take back into the box). Never
//!   dropped without a word.
//!
//! No UI types here, so the rules are tested without a window.

use std::time::{Duration, Instant};

use equipe_core::protocol::{Command, Entry, Event, SessionId, Snapshot};

/// How long a message handed to the line may go unanswered before it is
/// shown as not sent. A busy daemon answers late; a late echo still counts.
pub const ANSWER_WAIT: Duration = Duration::from_secs(30);

/// How far back a snapshot's conversation is looked through for a message
/// that went just before the line dropped.
const RECENT: usize = 12;

/// How a message on its way shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shown {
    /// Handed to the daemon; its word not here yet.
    Sending,
    /// Not attached: it goes when the line is back.
    Waiting,
    /// Not sent — no word back in time, or refused: the person's to retry.
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    /// Its key: the `nonce` its `say` carries, every time it is sent.
    pub id: u64,
    /// The conversation it is said in.
    pub session: SessionId,
    /// What was said, as sent: what its echo carries.
    pub text: String,
    /// What it goes as: the conversation it starts, any card it raises, the message.
    commands: Vec<Command>,
    pub shown: Shown,
    /// Handed to a line at least once: it may have arrived.
    handed: bool,
    sent_at: Option<Instant>,
    /// The daemon's own words, when it refused it.
    pub why: Option<String>,
}

/// The person's messages not yet taken in, oldest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outbox {
    items: Vec<Outgoing>,
}

/// The person's message among `commands` — the conversation it is said in, and its words.
pub fn message(commands: &[Command]) -> Option<(SessionId, String)> {
    commands.iter().find_map(|c| match c {
        Command::Say { session, text, .. } | Command::Submit { session, text, .. } | Command::SubmitLater { session, text, .. } => Some((*session, text.clone())),
        _ => None,
    })
}

impl Outbox {
    /// `commands`, the composer's for one line: when they carry the person's
    /// message, it is kept here (its `say` given its key) and what goes now
    /// is returned — nothing while not attached. Anything else (a slash
    /// command's) goes as it is.
    pub fn push(&mut self, mut commands: Vec<Command>, attached: bool, now: Instant) -> Vec<Command> {
        let Some((session, text)) = message(&commands) else { return commands };
        let id = equipe_core::protocol::new_session_id();
        for command in &mut commands {
            if let Command::Say { nonce, .. } = command {
                *nonce = Some(id);
            }
        }
        let mut item = Outgoing { id, session, text, commands, shown: Shown::Waiting, handed: false, sent_at: None, why: None };
        let out = if attached { item.send(now) } else { Vec::new() };
        self.items.push(item);
        out
    }

    /// One event from the daemon: whether a message on its way changed.
    pub fn on_event(&mut self, event: &Event) -> bool {
        match event {
            // Taken in: as a message, a steer or a memo.
            Event::UserMessage { session, text, .. } | Event::SteerQueued { session, text, .. } | Event::MessageQueued { session, text, .. } => self.taken(*session, |said| said == text.trim()),
            // An answer to a decision in words: equipe's line says so, quoting it.
            Event::Notice { session, text, .. } => self.taken(*session, |said| text.contains(said)),
            // A new decision in words read from it: the daemon has it.
            Event::WordsLive { live, .. } => self.taken(live.session, |said| said == live.said.trim()),
            // The conversation it starts is made: not asked for again.
            Event::LaneOpened { session, .. } => {
                for item in self.items.iter_mut().filter(|i| i.session == *session) {
                    item.commands.retain(|c| !matches!(c, Command::NewConversation { .. }));
                }
                false
            }
            // Refused where it was said (no such conversation, an engine that
            // can't sign in): not sent, and why.
            Event::Error { session: Some(session), message, .. } => match self.items.iter_mut().find(|i| i.session == *session && i.shown == Shown::Sending) {
                Some(item) => {
                    item.shown = Shown::Failed;
                    item.why = Some(message.clone());
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    /// The first message in `session` whose words `matches` accepts goes: taken in.
    fn taken(&mut self, session: SessionId, matches: impl Fn(&str) -> bool) -> bool {
        let Some(at) = self.items.iter().position(|i| i.session == session && !i.text.trim().is_empty() && matches(i.text.trim())) else { return false };
        self.items.remove(at);
        true
    }

    /// The line went: what was handed to it waits for the next hello.
    pub fn down(&mut self) -> bool {
        let mut changed = false;
        for item in self.items.iter_mut().filter(|i| i.shown == Shown::Sending) {
            item.shown = Shown::Waiting;
            changed = true;
        }
        changed
    }

    /// Attached anew, with `snapshot`: what arrived before the line went is
    /// taken in; what is waiting goes now, in order — each conversation it
    /// starts asked for once, and not at all when the daemon has it.
    pub fn attached(&mut self, snapshot: &Snapshot, now: Instant) -> Vec<Command> {
        self.items.retain(|item| !(item.handed && arrived(snapshot, item.session, &item.text)));
        let mut out = Vec::new();
        let mut asked: Vec<SessionId> = snapshot.sessions.iter().map(|s| s.session).collect();
        for item in &mut self.items {
            if asked.contains(&item.session) {
                item.commands.retain(|c| !matches!(c, Command::NewConversation { .. }));
            }
            if item.shown != Shown::Failed {
                out.extend(item.send(now));
            }
            if item.commands.iter().any(|c| matches!(c, Command::NewConversation { .. })) {
                asked.push(item.session);
            }
        }
        out
    }

    /// Time passed: what went unanswered too long, attached, shows as not sent.
    pub fn tick(&mut self, now: Instant, attached: bool) -> bool {
        if !attached {
            return false;
        }
        let mut changed = false;
        for item in self.items.iter_mut().filter(|i| i.shown == Shown::Sending) {
            if item.sent_at.is_some_and(|at| now.duration_since(at) >= ANSWER_WAIT) {
                item.shown = Shown::Failed;
                changed = true;
            }
        }
        changed
    }

    /// The person's retry on message `id`: what goes now (nothing while not
    /// attached — it waits, and goes with the next hello).
    pub fn retry(&mut self, id: u64, attached: bool, now: Instant) -> Vec<Command> {
        let Some(item) = self.items.iter_mut().find(|i| i.id == id) else { return Vec::new() };
        item.why = None;
        if attached {
            item.send(now)
        } else {
            item.shown = Shown::Waiting;
            Vec::new()
        }
    }

    /// Message `id` taken back, its words for the box. One already handed to
    /// a line may still arrive; its echo then shows it as said.
    pub fn take_back(&mut self, id: u64) -> Option<String> {
        let at = self.items.iter().position(|i| i.id == id)?;
        Some(self.items.remove(at).text)
    }

    /// The messages on their way in `session`, oldest first.
    pub fn in_session(&self, session: SessionId) -> impl Iterator<Item = &Outgoing> {
        self.items.iter().filter(move |i| i.session == session)
    }

    /// Whether any is still to be heard back from.
    pub fn sending(&self) -> bool {
        self.items.iter().any(|i| i.shown == Shown::Sending)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl Outgoing {
    /// Handed to the line now: what goes.
    fn send(&mut self, now: Instant) -> Vec<Command> {
        self.shown = Shown::Sending;
        self.handed = true;
        self.sent_at = Some(now);
        self.commands.clone()
    }
}

/// Whether `snapshot` shows `text` said in `session` lately — taken in as a
/// message, or waiting there as a steer or a memo.
fn arrived(snapshot: &Snapshot, session: SessionId, text: &str) -> bool {
    let text = text.trim();
    let Some(info) = snapshot.sessions.iter().find(|s| s.session == session) else { return false };
    if info.steering.iter().chain(&info.queued).any(|t| t.trim() == text) {
        return true;
    }
    info.recent.iter().rev().take(RECENT).any(|e| match e {
        Entry::User { text: said, .. } => said.trim() == text,
        Entry::Note { text: note, .. } => note.contains(text),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::by_words::Via;
    use equipe_core::protocol::SessionInfo;

    fn say(session: SessionId, text: &str) -> Command {
        Command::Say { squad: "acme".into(), session, text: text.into(), via: Via::Typed, all: false, later: false, pasted: false, about: None, device: None, nonce: None }
    }

    fn new(session: SessionId) -> Command {
        Command::NewConversation { squad: "acme".into(), session, project: None }
    }

    fn nonce(c: &Command) -> Option<u64> {
        match c {
            Command::Say { nonce, .. } => *nonce,
            _ => None,
        }
    }

    fn info(session: SessionId, recent: Vec<Entry>) -> SessionInfo {
        let mut info: SessionInfo = serde_json::from_value(serde_json::json!({ "session": session, "project": "/w", "engine": "claude", "access": "read_only", "state": "ready", "tokens": 0, "recent": [] })).unwrap();
        info.recent = recent;
        info
    }

    fn snapshot(sessions: Vec<SessionInfo>) -> Snapshot {
        Snapshot { squad: "acme".into(), sessions, ..Snapshot::default() }
    }

    #[test]
    fn a_message_said_while_the_line_is_down_waits_and_goes_after_the_next_hello_in_order_once() {
        let mut out = Outbox::default();
        let t0 = Instant::now();
        // The daemon restarting: a new conversation's first message, then another.
        assert!(out.push(vec![new(9), say(9, "what waits for me to accept?")], false, t0).is_empty(), "nothing goes while down");
        assert!(out.push(vec![new(9), say(9, "and the board?")], false, t0).is_empty());
        let shown: Vec<(&str, Shown)> = out.in_session(9).map(|i| (i.text.as_str(), i.shown)).collect();
        assert_eq!(shown, [("what waits for me to accept?", Shown::Waiting), ("and the board?", Shown::Waiting)], "on screen, never lost");
        // Time passes, still down: still waiting — not failed, not dropped.
        assert!(!out.tick(t0 + ANSWER_WAIT * 4, false));
        // Back: the conversation made once, then both messages in the order said, each with its key.
        let sent = out.attached(&snapshot(vec![info(1, vec![])]), t0);
        assert_eq!(sent.len(), 3, "{sent:?}");
        assert_eq!(sent[0], new(9));
        assert!(matches!(&sent[1], Command::Say { text, .. } if text == "what waits for me to accept?"));
        assert!(matches!(&sent[2], Command::Say { text, .. } if text == "and the board?"));
        let keys: Vec<Option<u64>> = sent.iter().skip(1).map(nonce).collect();
        assert!(keys.iter().all(Option::is_some) && keys[0] != keys[1], "each its own key: {keys:?}");
        assert!(out.in_session(9).all(|i| i.shown == Shown::Sending));
        // Their echoes: taken in, gone from here.
        assert!(!out.on_event(&Event::LaneOpened { session: 9, of: 1, project: None, kept: true }), "made: not a word on the messages yet");
        assert!(out.on_event(&Event::UserMessage { session: 9, text: "what waits for me to accept?".into(), kept: vec![] }));
        assert!(out.on_event(&Event::UserMessage { session: 9, text: "and the board?".into(), kept: vec![] }));
        assert!(out.is_empty());
        // Another hello: nothing goes twice.
        assert!(out.attached(&snapshot(vec![info(1, vec![]), info(9, vec![])]), t0).is_empty());
    }

    #[test]
    fn a_message_handed_to_a_line_that_went_goes_again_only_if_the_daemon_never_had_it() {
        let mut out = Outbox::default();
        let t0 = Instant::now();
        let first = out.push(vec![say(1, "fix the header")], true, t0);
        let key = nonce(&first[0]);
        assert!(key.is_some());
        out.push(vec![say(1, "and the footer")], true, t0);
        assert!(out.down());
        assert!(out.in_session(1).all(|i| i.shown == Shown::Waiting));
        // The new daemon's snapshot has the first (it arrived before the line went), not the second.
        let had = info(1, vec![Entry::User { text: "fix the header".into(), kept: vec![] }]);
        let sent = out.attached(&snapshot(vec![had]), t0);
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(matches!(&sent[0], Command::Say { text, .. } if text == "and the footer"));
        assert_eq!(out.in_session(1).count(), 1);
        // Sent again, the same key: a daemon that had it takes it once.
        let again = out.attached(&snapshot(vec![info(1, vec![])]), t0);
        assert_eq!(again.iter().map(nonce).collect::<Vec<_>>(), sent.iter().map(nonce).collect::<Vec<_>>());
    }

    #[test]
    fn a_new_conversation_never_acknowledged_shows_not_sent_and_its_retry_goes_again_with_its_key() {
        let mut out = Outbox::default();
        let t0 = Instant::now();
        let sent = out.push(vec![new(9), say(9, "what waits for me?")], true, t0);
        assert_eq!(sent.len(), 2);
        assert!(out.sending());
        assert!(!out.tick(t0 + ANSWER_WAIT / 2, true), "not yet");
        // Nothing back: not sent — said so on its bubble, kept for the person.
        assert!(out.tick(t0 + ANSWER_WAIT, true));
        assert_eq!(out.in_session(9).map(|i| i.shown).collect::<Vec<_>>(), [Shown::Failed]);
        // A hello doesn't send a failed one by itself: the person decides.
        assert!(out.attached(&snapshot(vec![]), t0).is_empty());
        let id = out.in_session(9).next().unwrap().id;
        let retried = out.retry(id, true, t0 + ANSWER_WAIT);
        assert_eq!(retried.len(), 2, "the conversation asked for again, then the message: {retried:?}");
        assert_eq!(nonce(&retried[1]), nonce(&sent[1]), "the same key: taken once however often it goes");
        // A late echo still counts.
        assert!(out.on_event(&Event::UserMessage { session: 9, text: "what waits for me?".into(), kept: vec![] }));
        assert!(out.is_empty());
    }

    #[test]
    fn refused_where_it_was_said_it_shows_why_and_taken_back_its_words_return() {
        let mut out = Outbox::default();
        let t0 = Instant::now();
        out.push(vec![say(4, "hello?")], true, t0);
        assert!(out.on_event(&Event::Error { session: Some(4), message: "no such session 4".into(), next: None }));
        let item = out.in_session(4).next().unwrap();
        assert_eq!((item.shown, item.why.as_deref()), (Shown::Failed, Some("no such session 4")));
        let id = item.id;
        // Retried while down: it waits for the line.
        assert!(out.retry(id, false, t0).is_empty());
        assert_eq!(out.in_session(4).next().unwrap().shown, Shown::Waiting);
        assert_eq!(out.take_back(id).as_deref(), Some("hello?"));
        assert!(out.is_empty());
    }

    #[test]
    fn a_message_taken_as_an_answer_in_words_is_heard_back_too_and_other_commands_go_as_they_are() {
        let mut out = Outbox::default();
        let t0 = Instant::now();
        out.push(vec![say(1, "ㄱㄱ")], true, t0);
        assert!(!out.on_event(&Event::UserMessage { session: 2, text: "ㄱㄱ".into(), kept: vec![] }), "another conversation's");
        assert!(out.on_event(&Event::Notice { session: 1, undo: None, text: "“ㄱㄱ” was taken as your answer".into() }));
        assert!(out.is_empty());
        let slash = vec![Command::Focus { squad: "acme".into(), project: None }];
        assert_eq!(out.push(slash.clone(), false, t0), slash, "not a message: not kept");
        assert!(out.is_empty());
    }
}
