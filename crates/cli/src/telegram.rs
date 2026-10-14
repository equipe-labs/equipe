//! Telegram: the squad from a phone. A bot the person makes with @BotFather,
//! paired with their own Telegram account, brings what needs them — requests
//! for permission, work to review, tickets waiting on them — with buttons to
//! answer, and carries their messages to the PM (or `@member`) and the
//! replies back. Telegram delivers the notifications.
//!
//! The bridge runs in the daemon, as one more screen on its socket. It answers
//! only the paired account, in its private chat with the bot. From a phone a
//! request is allowed once — never "always" — and merging stays at the console.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::gateway::{Client, Commands};
use equipe_core::protocol::{Approval, Command, Decision, Event, SessionId, SessionState, Snapshot, Ticket, TicketId, TicketStatus, TurnOutcome};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// The bot and who it answers. Kept where only the person can read it: the
/// token is the bot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub token: String,
    /// The bot's username, without `@`.
    pub bot: String,
    /// The person's Telegram account, and their private chat with the bot.
    pub user: i64,
    pub chat: i64,
}

pub fn config_path(home: &Path) -> PathBuf {
    home.join("telegram.json")
}

pub fn load(home: &Path) -> Option<Config> {
    serde_json::from_slice(&std::fs::read(config_path(home)).ok()?).ok()
}

pub fn save(home: &Path, config: &Config) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::create_dir_all(home)?;
    let path = config_path(home);
    let part = path.with_extension("json.part");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&part)?;
    file.write_all(&serde_json::to_vec_pretty(config).expect("config serializes"))?;
    std::fs::rename(part, path)
}

/// Longest message text Telegram takes is 4096; replies are cut below it.
const CHUNK: usize = 3500;
/// Of a command waiting for permission, this much shows.
const ACTION_SHOWN: usize = 1200;

/// What the bridge asks Telegram to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    /// A message (HTML), with buttons under it, or asking for a reply (`ask`:
    /// the placeholder in the person's text field). `key` names it, to edit
    /// or recognise replies to it later.
    Send { text: String, buttons: Vec<(String, String)>, ask: Option<String>, key: Option<Key> },
    /// A message sent earlier, rewritten — its buttons gone.
    Edit { message: i64, text: String },
    /// The little note a pressed button shows.
    Toast { callback: String, text: String },
    /// 👀 on the person's message: it reached who it was for.
    Seen { message: i64 },
}

/// What a message the bridge sent was about.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Approval(String, u64),
    Review(String, TicketId),
    Waiting(String, TicketId),
    /// "What should change?" — the reply sends the ticket back.
    Reopen(String, TicketId),
    /// "What did you do?" — the reply hands the ticket back to its member.
    Resume(String, TicketId),
    /// A member's answer: a reply to it goes on talking with them.
    Reply(String, SessionId),
}

/// What a button does.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Act {
    Decide(String, u64, Decision),
    Accept(String, TicketId),
    Reopen(String, TicketId),
    Resume(String, TicketId),
}

#[derive(Debug, Default)]
struct Squad {
    /// Members by session, and their roles.
    members: BTreeMap<SessionId, (String, String)>,
    working: HashSet<SessionId>,
    tickets: BTreeMap<TicketId, Ticket>,
    approvals: BTreeMap<u64, Approval>,
}

/// A member the person wrote to from Telegram: until its turn on their
/// message starts, then what it says, until the turn ends.
#[derive(Debug)]
enum Hearing {
    Waiting(String),
    Capturing(String),
}

/// The conversation with the person, as plain state: events and Telegram
/// updates in, commands for the squads and messages for Telegram out.
#[derive(Debug, Default)]
pub struct Bridge {
    squads: BTreeMap<String, Squad>,
    /// Where the person's messages go: a squad's name.
    current: Option<String>,
    buttons: HashMap<String, Act>,
    next_button: u64,
    sent: HashMap<Key, i64>,
    keys: HashMap<i64, Key>,
    hearing: HashMap<SessionId, (String, Hearing)>,
}

/// Text for Telegram's HTML.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    format!("{}…", text.chars().take(max).collect::<String>())
}

/// A long text in pieces Telegram takes, broken at lines where it can be.
fn chunks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut piece = String::new();
    for line in text.split_inclusive('\n') {
        if piece.chars().count() + line.chars().count() > CHUNK && !piece.is_empty() {
            out.push(std::mem::take(&mut piece));
        }
        if line.chars().count() > CHUNK {
            let chars: Vec<char> = line.chars().collect();
            for part in chars.chunks(CHUNK) {
                out.push(part.iter().collect());
            }
            continue;
        }
        piece.push_str(line);
    }
    if !piece.trim().is_empty() {
        out.push(piece);
    }
    out
}

impl Bridge {
    /// A message Telegram took, and the id it gave it.
    pub fn sent(&mut self, key: Key, message: i64) {
        self.sent.insert(key.clone(), message);
        self.keys.insert(message, key);
    }

    fn button(&mut self, label: &str, act: Act) -> (String, String) {
        self.next_button += 1;
        let data = format!("b{}", self.next_button);
        self.buttons.insert(data.clone(), act);
        (label.to_owned(), data)
    }

    /// " · squad", when there are several to tell apart.
    fn where_(&self, squad: &str) -> String {
        if self.squads.len() > 1 { format!(" · {}", escape(squad)) } else { String::new() }
    }

    fn current(&self) -> Option<String> {
        self.current.clone().filter(|s| self.squads.contains_key(s)).or_else(|| self.squads.keys().next().cloned())
    }

    /// Following a squad: what it is now, and the requests already waiting.
    pub fn attach(&mut self, snapshot: &Snapshot) -> Vec<Out> {
        let mut squad = Squad::default();
        for s in &snapshot.sessions {
            if let Some(member) = &s.member {
                squad.members.insert(s.session, (member.name.clone(), member.role.clone()));
                if s.state == SessionState::Working {
                    squad.working.insert(s.session);
                }
            }
        }
        squad.tickets = snapshot.tickets.iter().map(|t| (t.id, t.clone())).collect();
        self.squads.insert(snapshot.squad.clone(), squad);
        let mut outs = Vec::new();
        for approval in &snapshot.approvals {
            outs.extend(self.on_event(&snapshot.squad, &Event::ApprovalRequested { squad: snapshot.squad.clone(), approval: approval.clone() }));
        }
        outs
    }

    fn name_of(&self, squad: &str, session: SessionId) -> String {
        self.squads.get(squad).and_then(|s| s.members.get(&session)).map(|(name, _)| name.clone()).unwrap_or_else(|| "a member".into())
    }

    fn approval_text(&self, squad: &str, a: &Approval) -> String {
        let verb = match a.tool.as_str() {
            "Bash" => "to run",
            "Write" | "Edit" | "MultiEdit" => "to edit",
            "WebFetch" => "to fetch",
            "Confirm" => "to",
            equipe_core::permissions::BROWSER_CODE => "to run code in its browser",
            _ => "to use",
        };
        let ticket = a.ticket.map(|t| format!(" · T-{t}")).unwrap_or_default();
        let mut text = format!("🔐 <b>{}</b> asks {verb}{ticket}{}\n<pre>{}</pre>", escape(&a.member), self.where_(squad), escape(&cut(&a.action, ACTION_SHOWN)));
        if let Some(reason) = &a.reason {
            text.push_str(&format!("\n<i>{}</i>", escape(reason)));
        }
        text
    }

    fn review_text(&self, squad: &str, t: &Ticket) -> String {
        let who = t.assignee.as_ref().or(t.owner.as_ref()).cloned().unwrap_or_else(|| "A member".into());
        let mut text = format!("📝 <b>T-{}</b> {}{}\n{} handed it in for your review.", t.id, escape(&t.title), self.where_(squad), escape(&who));
        if let Some(note) = t.note.as_deref().filter(|n| !n.trim().is_empty()) {
            text.push_str(&format!("\n\n{}", escape(&cut(note, 1500))));
        }
        text.push_str("\n\n<i>Reply to this message to ask for changes.</i>");
        text
    }

    fn waiting_text(&self, squad: &str, t: &Ticket) -> String {
        let who = t.assignee.as_ref().or(t.owner.as_ref()).cloned().unwrap_or_else(|| "A member".into());
        let mut text = format!("✋ <b>T-{}</b> {}{}\n{} needs you", t.id, escape(&t.title), self.where_(squad), escape(&who));
        match t.note.as_deref().filter(|n| !n.trim().is_empty()) {
            Some(note) => text.push_str(&format!(":\n\n{}", escape(&cut(note, 1500)))),
            None => text.push('.'),
        }
        text.push_str("\n\n<i>Reply to this message with what you did — they carry on from it.</i>");
        text
    }

    /// Something happened in `squad`.
    pub fn on_event(&mut self, squad: &str, event: &Event) -> Vec<Out> {
        let mut outs = Vec::new();
        self.squads.entry(squad.to_owned()).or_default();
        macro_rules! state {
            () => {
                self.squads.get_mut(squad).expect("entered above")
            };
        }
        match event {
            Event::SessionStarted { session, member: Some(member), .. } => {
                state!().members.insert(*session, (member.name.clone(), member.role.clone()));
            }
            Event::SessionClosed { session } => {
                state!().members.remove(session);
                state!().working.remove(session);
            }
            Event::UserMessage { session, text } => {
                state!().working.insert(*session);
                if let Some((_, hearing)) = self.hearing.get_mut(session)
                    && matches!(hearing, Hearing::Waiting(sent) if text.contains(sent.as_str()))
                {
                    *hearing = Hearing::Capturing(String::new());
                }
            }
            Event::MessageQueued { session, text, .. } => {
                if matches!(self.hearing.get(session), Some((_, Hearing::Waiting(sent))) if sent == text) {
                    let name = self.name_of(squad, *session);
                    outs.push(Out::Send {
                        text: format!("⏳ {} is mid-turn — your message goes in when it's done.", escape(&name)),
                        buttons: vec![],
                        ask: None,
                        key: None,
                    });
                }
            }
            Event::TextDelta { session, text } => {
                if let Some((_, Hearing::Capturing(said))) = self.hearing.get_mut(session) {
                    said.push_str(text);
                }
            }
            Event::ToolUse { session, .. } => {
                if let Some((_, Hearing::Capturing(said))) = self.hearing.get_mut(session)
                    && !said.is_empty()
                    && !said.ends_with('\n')
                {
                    said.push_str("\n\n");
                }
            }
            Event::TurnEnded { session, outcome, .. } => {
                state!().working.remove(session);
                if matches!(self.hearing.get(session), Some((_, Hearing::Capturing(_))))
                    && let Some((_, Hearing::Capturing(said))) = self.hearing.remove(session)
                {
                    let name = self.name_of(squad, *session);
                    let said = said.trim();
                    if said.is_empty() {
                        let why = match outcome {
                            TurnOutcome::Failed => "its turn failed — the console says why",
                            TurnOutcome::Interrupted => "it was stopped",
                            _ => "it finished without a word",
                        };
                        outs.push(Out::Send { text: format!("<b>{}</b>: {why}", escape(&name)), buttons: vec![], ask: None, key: None });
                    } else {
                        let pieces = chunks(said);
                        let last = pieces.len() - 1;
                        for (i, piece) in pieces.into_iter().enumerate() {
                            let head = if i == 0 { format!("<b>{}</b>{}\n", escape(&name), self.where_(squad)) } else { String::new() };
                            outs.push(Out::Send {
                                text: format!("{head}{}", escape(&piece)),
                                buttons: vec![],
                                ask: None,
                                key: (i == last).then(|| Key::Reply(squad.to_owned(), *session)),
                            });
                        }
                    }
                }
            }
            Event::Error { session: Some(session), message, .. } if self.hearing.contains_key(session) => {
                self.hearing.remove(session);
                let name = self.name_of(squad, *session);
                outs.push(Out::Send { text: format!("⚠️ <b>{}</b>: {}", escape(&name), escape(message)), buttons: vec![], ask: None, key: None });
            }
            Event::ApprovalRequested { approval, .. } => {
                let key = Key::Approval(squad.to_owned(), approval.id);
                state!().approvals.insert(approval.id, approval.clone());
                if !self.sent.contains_key(&key) {
                    let text = self.approval_text(squad, approval);
                    let allow = self.button("Allow", Act::Decide(squad.to_owned(), approval.id, Decision::Allow));
                    let decline = self.button("Decline", Act::Decide(squad.to_owned(), approval.id, Decision::Deny));
                    outs.push(Out::Send { text, buttons: vec![allow, decline], ask: None, key: Some(key) });
                }
            }
            Event::ApprovalResolved { approval, decision, .. } => {
                if let Some(a) = state!().approvals.remove(approval)
                    && let Some(message) = self.sent.get(&Key::Approval(squad.to_owned(), *approval)).copied()
                {
                    let answer = match decision {
                        Decision::Allow | Decision::Always => "✅ Allowed",
                        Decision::Deny => "❌ Declined",
                    };
                    outs.push(Out::Edit { message, text: format!("{}\n\n<b>{answer}</b>", self.approval_text(squad, &a)) });
                }
            }
            Event::TicketChanged { ticket, .. } => {
                let before = state!().tickets.insert(ticket.id, ticket.clone()).map(|t| t.status);
                if before == Some(ticket.status) {
                    return outs;
                }
                let review = Key::Review(squad.to_owned(), ticket.id);
                let waiting = Key::Waiting(squad.to_owned(), ticket.id);
                // What it was waiting for is settled: the message says how.
                if before == Some(TicketStatus::Review)
                    && let Some(message) = self.sent.get(&review).copied()
                {
                    let how = match ticket.status {
                        TicketStatus::Done => "✅ Accepted",
                        TicketStatus::Cancelled => "Cancelled",
                        _ => "↩ Sent back",
                    };
                    let text = self.review_text(squad, ticket).replace("\n\n<i>Reply to this message to ask for changes.</i>", "");
                    outs.push(Out::Edit { message, text: format!("{text}\n\n<b>{how}</b>") });
                }
                if before == Some(TicketStatus::Waiting)
                    && let Some(message) = self.sent.get(&waiting).copied()
                {
                    let text = self.waiting_text(squad, ticket).replace("\n\n<i>Reply to this message with what you did — they carry on from it.</i>", "");
                    outs.push(Out::Edit { message, text: format!("{text}\n\n<b>↪ Back with them</b>") });
                }
                match ticket.status {
                    TicketStatus::Review => {
                        let text = self.review_text(squad, ticket);
                        let accept = self.button("Accept", Act::Accept(squad.to_owned(), ticket.id));
                        let reopen = self.button("Ask for changes", Act::Reopen(squad.to_owned(), ticket.id));
                        outs.push(Out::Send { text, buttons: vec![accept, reopen], ask: None, key: Some(review) });
                    }
                    TicketStatus::Waiting => {
                        let text = self.waiting_text(squad, ticket);
                        let done = self.button("Done — carry on", Act::Resume(squad.to_owned(), ticket.id));
                        outs.push(Out::Send { text, buttons: vec![done], ask: None, key: Some(waiting) });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        outs
    }

    /// The person pressed a button. Returns commands for squads, and what to tell them.
    pub fn on_button(&mut self, callback: &str, data: &str) -> (Vec<(String, Command)>, Vec<Out>) {
        let toast = |text: &str| Out::Toast { callback: callback.to_owned(), text: text.to_owned() };
        let Some(act) = self.buttons.get(data).cloned() else {
            return (vec![], vec![toast("That's no longer waiting")]);
        };
        match act {
            Act::Decide(squad, id, decision) => {
                if !self.squads.get(&squad).is_some_and(|s| s.approvals.contains_key(&id)) {
                    return (vec![], vec![toast("Already answered")]);
                }
                let said = if decision == Decision::Deny { "Declined" } else { "Allowed" };
                (vec![(squad.clone(), Command::Decide { squad, approval: id, decision })], vec![toast(said)])
            }
            Act::Accept(squad, ticket) => {
                if !self.is(&squad, ticket, TicketStatus::Review) {
                    return (vec![], vec![toast("It isn't waiting for review any more")]);
                }
                (vec![(squad.clone(), Command::Accept { squad, ticket })], vec![toast("Accepted")])
            }
            Act::Reopen(squad, ticket) => {
                if !self.is(&squad, ticket, TicketStatus::Review) {
                    return (vec![], vec![toast("It isn't waiting for review any more")]);
                }
                let ask = Out::Send {
                    text: format!("What should change in <b>T-{ticket}</b>?"),
                    buttons: vec![],
                    ask: Some("what to change".into()),
                    key: Some(Key::Reopen(squad, ticket)),
                };
                (vec![], vec![toast(""), ask])
            }
            Act::Resume(squad, ticket) => {
                if !self.is(&squad, ticket, TicketStatus::Waiting) {
                    return (vec![], vec![toast("It isn't waiting on you any more")]);
                }
                let ask = Out::Send {
                    text: format!("What did you do for <b>T-{ticket}</b>? They carry on from it."),
                    buttons: vec![],
                    ask: Some("what you did".into()),
                    key: Some(Key::Resume(squad, ticket)),
                };
                (vec![], vec![toast(""), ask])
            }
        }
    }

    fn is(&self, squad: &str, ticket: TicketId, status: TicketStatus) -> bool {
        self.squads.get(squad).and_then(|s| s.tickets.get(&ticket)).is_some_and(|t| t.status == status)
    }

    fn say(text: String) -> Out {
        Out::Send { text, buttons: vec![], ask: None, key: None }
    }

    /// Ask `session` in `squad`, and hear its answer here.
    fn submit(&mut self, squad: &str, session: SessionId, text: &str, message: i64) -> (Vec<(String, Command)>, Vec<Out>) {
        self.hearing.insert(session, (squad.to_owned(), Hearing::Waiting(text.to_owned())));
        (vec![(squad.to_owned(), Command::Submit { session, text: text.to_owned() })], vec![Out::Seen { message }])
    }

    /// The person wrote `text` (message `message`), maybe in reply to `reply_to`.
    pub fn on_text(&mut self, text: &str, reply_to: Option<i64>, message: i64) -> (Vec<(String, Command)>, Vec<Out>) {
        let text = text.trim();
        if text.is_empty() {
            return (vec![], vec![]);
        }
        if let Some(key) = reply_to.and_then(|id| self.keys.get(&id)).cloned() {
            match key {
                Key::Reopen(squad, ticket) | Key::Review(squad, ticket) => {
                    if !self.is(&squad, ticket, TicketStatus::Review) {
                        return (vec![], vec![Self::say(format!("T-{ticket} isn't waiting for review any more."))]);
                    }
                    let command = Command::Reopen { squad: squad.clone(), ticket, note: text.to_owned() };
                    return (vec![(squad, command)], vec![Out::Seen { message }]);
                }
                Key::Resume(squad, ticket) | Key::Waiting(squad, ticket) => {
                    if !self.is(&squad, ticket, TicketStatus::Waiting) {
                        return (vec![], vec![Self::say(format!("T-{ticket} isn't waiting on you any more."))]);
                    }
                    let command = Command::Resume { squad: squad.clone(), ticket, note: text.to_owned() };
                    return (vec![(squad, command)], vec![Out::Seen { message }]);
                }
                Key::Reply(squad, session) => {
                    if self.squads.get(&squad).is_some_and(|s| s.members.contains_key(&session)) {
                        return self.submit(&squad, session, text, message);
                    }
                }
                Key::Approval(..) => return (vec![], vec![Self::say("Answer it with its buttons.".into())]),
            }
        }
        let (word, rest) = text.split_once(char::is_whitespace).map_or((text, ""), |(w, r)| (w, r.trim()));
        // `/command@bot` in a group-style mention is the same command.
        let word = word.split('@').next().filter(|w| w.starts_with('/')).unwrap_or(word);
        match word {
            "/start" | "/help" => return (vec![], vec![Self::say(self.help())]),
            "/status" => return (vec![], vec![Self::say(self.status())]),
            "/board" => return (vec![], vec![Self::say(self.board())]),
            "/squad" => {
                if rest.is_empty() {
                    let names: Vec<String> = self.squads.keys().map(|s| escape(s)).collect();
                    let here = self.current().map(|s| escape(&s)).unwrap_or_default();
                    return (vec![], vec![Self::say(format!("Writing to <b>{here}</b>. Squads: {} · /squad &lt;name&gt; switches", names.join(", ")))]);
                }
                if !self.squads.contains_key(rest) {
                    return (vec![], vec![Self::say(format!("No squad {} · /squad lists them", escape(rest)))]);
                }
                self.current = Some(rest.to_owned());
                return (vec![], vec![Self::say(format!("Now writing to <b>{}</b>", escape(rest)))]);
            }
            _ if word.starts_with('/') => return (vec![], vec![Self::say(format!("No command {} · /help", escape(word)))]),
            _ => {}
        }
        let Some(squad) = self.current() else {
            return (vec![], vec![Self::say("No squad yet — start one with equipe in a project.".into())]);
        };
        let members = &self.squads[&squad].members;
        // `@name …` is for that member; anything else for the PM.
        let (session, said) = match word.strip_prefix('@') {
            Some(name) => match members.iter().find(|(_, (n, _))| n == name) {
                Some((id, _)) if !rest.is_empty() => (*id, rest),
                Some(_) => return (vec![], vec![Self::say(format!("What should I tell {}?", escape(name)))]),
                None => return (vec![], vec![Self::say(format!("No one called @{} in {}", escape(name), escape(&squad)))]),
            },
            None => match members.iter().find(|(_, (_, role))| role == equipe_core::roles::PM) {
                Some((id, _)) => (*id, text),
                None => return (vec![], vec![Self::say(format!("{} has no PM right now · @name writes to a member", escape(&squad)))]),
            },
        };
        self.submit(&squad, session, said, message)
    }

    fn help(&self) -> String {
        let here = self.current().map(|s| format!(" of <b>{}</b>", escape(&s))).unwrap_or_default();
        format!(
            "Write here to talk to the PM{here}; <code>@name …</code> writes to someone else. Reply to a member's message to keep talking with them.\n\n\
Requests for permission, work to review and tickets that need you arrive here, with buttons.\n\n\
/status — where every squad stands\n/board — the board\n/squad — which squad you write to"
        )
    }

    fn status(&self) -> String {
        if self.squads.is_empty() {
            return "No squads yet.".into();
        }
        let here = self.current();
        let mut out = Vec::new();
        for (name, s) in &self.squads {
            let mark = if here.as_deref() == Some(name.as_str()) && self.squads.len() > 1 { " ← you write here" } else { "" };
            let mut lines = vec![format!("<b>{}</b>{mark}", escape(name))];
            let working: Vec<String> = s
                .working
                .iter()
                .filter_map(|id| s.members.get(id))
                .map(|(member, _)| {
                    let ticket = s.tickets.values().find(|t| t.status == TicketStatus::InProgress && t.assignee.as_deref() == Some(member.as_str()));
                    match ticket {
                        Some(t) => format!("{} (T-{})", escape(member), t.id),
                        None => escape(member),
                    }
                })
                .collect();
            lines.push(if working.is_empty() { "nobody working".into() } else { format!("working: {}", working.join(", ")) });
            let review = s.tickets.values().filter(|t| t.status == TicketStatus::Review).count();
            let waiting = s.tickets.values().filter(|t| t.status == TicketStatus::Waiting).count();
            let mut yours = Vec::new();
            if review > 0 {
                yours.push(format!("{review} to review"));
            }
            if waiting > 0 {
                yours.push(format!("{waiting} waiting on you"));
            }
            if !s.approvals.is_empty() {
                yours.push(format!("{} requests", s.approvals.len()));
            }
            if !yours.is_empty() {
                lines.push(format!("for you: {}", yours.join(" · ")));
            }
            out.push(lines.join("\n"));
        }
        out.join("\n\n")
    }

    fn board(&self) -> String {
        let Some(name) = self.current() else {
            return "No squad yet.".into();
        };
        let s = &self.squads[&name];
        let mut out = vec![format!("<b>{}</b>", escape(&name))];
        for (heading, statuses) in [
            ("For you", &[TicketStatus::Review, TicketStatus::Waiting][..]),
            ("In progress", &[TicketStatus::InProgress][..]),
            ("To do", &[TicketStatus::Todo][..]),
        ] {
            let tickets: Vec<String> = s
                .tickets
                .values()
                .filter(|t| statuses.contains(&t.status))
                .map(|t| {
                    let who = t.assignee.as_ref().or(t.owner.as_ref()).map(|w| format!(" · {}", escape(w))).unwrap_or_default();
                    format!("T-{} {}{who}", t.id, escape(&cut(&t.title, 60)))
                })
                .collect();
            if !tickets.is_empty() {
                out.push(format!("\n<b>{heading}</b>\n{}", tickets.join("\n")));
            }
        }
        if out.len() == 1 {
            out.push("Nothing open.".into());
        }
        out.join("\n")
    }
}

/// Telegram's Bot API, for one bot.
#[derive(Clone)]
struct Api {
    http: reqwest::Client,
    base: String,
}

impl Api {
    fn new(token: &str) -> Self {
        let base = std::env::var("EQUIPE_TELEGRAM_API").unwrap_or_else(|_| "https://api.telegram.org".into());
        Self { http: reqwest::Client::new(), base: format!("{base}/bot{token}") }
    }

    /// Call `method`; its result, or why not. The token never shows in an error.
    async fn call(&self, method: &str, body: Value) -> Result<Value, String> {
        let response = self
            .http
            .post(format!("{}/{method}", self.base))
            .json(&body)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|err| err.without_url().to_string())?;
        let value: Value = response.json().await.map_err(|err| err.without_url().to_string())?;
        if value["ok"] == true {
            Ok(value["result"].clone())
        } else {
            Err(value["description"].as_str().unwrap_or("Telegram said no").to_owned())
        }
    }

    async fn send(&self, chat: i64, text: &str, markup: Option<Value>) -> Result<i64, String> {
        let mut body = json!({ "chat_id": chat, "text": text, "parse_mode": "HTML", "link_preview_options": { "is_disabled": true } });
        if let Some(markup) = markup {
            body["reply_markup"] = markup;
        }
        let sent = self.call("sendMessage", body).await?;
        Ok(sent["message_id"].as_i64().unwrap_or_default())
    }
}

/// Do what the bridge asked, telling it the ids of what was sent.
async fn deliver(api: &Api, chat: i64, bridge: &mut Bridge, outs: Vec<Out>) {
    for out in outs {
        let done = match out {
            Out::Send { text, buttons, ask, key } => {
                let markup = if !buttons.is_empty() {
                    let row: Vec<Value> = buttons.iter().map(|(label, data)| json!({ "text": label, "callback_data": data })).collect();
                    Some(json!({ "inline_keyboard": [row] }))
                } else {
                    ask.map(|placeholder| json!({ "force_reply": true, "input_field_placeholder": placeholder }))
                };
                match api.send(chat, &text, markup).await {
                    Ok(id) => {
                        if let Some(key) = key {
                            bridge.sent(key, id);
                        }
                        Ok(())
                    }
                    Err(err) => Err(err),
                }
            }
            Out::Edit { message, text } => {
                api.call("editMessageText", json!({ "chat_id": chat, "message_id": message, "text": text, "parse_mode": "HTML" })).await.map(|_| ())
            }
            Out::Toast { callback, text } => api.call("answerCallbackQuery", json!({ "callback_query_id": callback, "text": text })).await.map(|_| ()),
            Out::Seen { message } => api
                .call("setMessageReaction", json!({ "chat_id": chat, "message_id": message, "reaction": [{ "type": "emoji", "emoji": "👀" }] }))
                .await
                .map(|_| ()),
        };
        if let Err(err) = done {
            eprintln!("telegram: {err}");
        }
    }
}

/// An update from Telegram, from the paired person or no one: their message
/// in their chat with the bot, or a button they pressed.
fn on_update(config: &Config, bridge: &mut Bridge, update: &Value) -> (Vec<(String, Command)>, Vec<Out>) {
    if let Some(press) = update.get("callback_query") {
        if press["from"]["id"].as_i64() != Some(config.user) {
            return (vec![], vec![]);
        }
        let id = press["id"].as_str().unwrap_or_default();
        return bridge.on_button(id, press["data"].as_str().unwrap_or_default());
    }
    let Some(message) = update.get("message") else {
        return (vec![], vec![]);
    };
    if message["from"]["id"].as_i64() != Some(config.user) || message["chat"]["id"].as_i64() != Some(config.chat) {
        return (vec![], vec![]);
    }
    let id = message["message_id"].as_i64().unwrap_or_default();
    let reply_to = message["reply_to_message"]["message_id"].as_i64();
    match message["text"].as_str() {
        Some(text) => bridge.on_text(text, reply_to, id),
        None => (vec![], vec![Bridge::say("Only text for now — screenshots go in at the console.".into())]),
    }
}

/// Follow `squad`: its snapshot into the bridge, its events into `events`.
async fn follow(socket: &Path, version: &str, squad: &str, bridge: &mut Bridge, events: &mpsc::UnboundedSender<(String, Event)>) -> std::io::Result<(Commands, Vec<Out>)> {
    let (client, hello) = Client::connect(socket, version, squad).await?;
    let outs = bridge.attach(&hello.snapshot);
    let (commands, mut stream) = client.split();
    let events = events.clone();
    let name = squad.to_owned();
    tokio::spawn(async move {
        while let Some(event) = stream.recv().await {
            if events.send((name.clone(), event)).is_err() {
                break;
            }
        }
    });
    Ok((commands, outs))
}

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// In the daemon: the bridge, whenever Telegram is set up — picked up within
/// seconds of `equipe telegram setup`, dropped as soon as it's turned off.
pub async fn serve_forever(home: PathBuf, socket: PathBuf, version: String) {
    loop {
        if let Some(config) = load(&home)
            && let Err(why) = run(&home, &socket, &version, config).await
        {
            eprintln!("telegram: {why}");
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn run(home: &Path, socket: &Path, version: &str, config: Config) -> Result<(), String> {
    let api = Api::new(&config.token);
    let since = modified(&config_path(home));
    let mut bridge = Bridge::default();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<(String, Event)>();
    let mut squads: HashMap<String, Commands> = HashMap::new();
    let (update_tx, mut update_rx) = mpsc::unbounded_channel::<Value>();
    let poll = api.clone();
    let poller = tokio::spawn(async move {
        let mut offset = 0i64;
        loop {
            let asked = json!({ "offset": offset, "timeout": 30, "allowed_updates": ["message", "callback_query"] });
            match poll.call("getUpdates", asked).await {
                Ok(Value::Array(updates)) => {
                    for update in updates {
                        offset = offset.max(update["update_id"].as_i64().unwrap_or_default() + 1);
                        if update_tx.send(update).is_err() {
                            return;
                        }
                    }
                }
                Ok(_) => {}
                Err(err) => {
                    eprintln!("telegram: {err}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    });
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    let result = loop {
        tokio::select! {
            Some((squad, event)) = event_rx.recv() => {
                let outs = bridge.on_event(&squad, &event);
                deliver(&api, config.chat, &mut bridge, outs).await;
            }
            Some(update) = update_rx.recv() => {
                let (commands, outs) = on_update(&config, &mut bridge, &update);
                for (squad, command) in commands {
                    if let Some(sender) = squads.get(&squad) {
                        sender.send(command);
                    }
                }
                deliver(&api, config.chat, &mut bridge, outs).await;
            }
            _ = tick.tick() => {
                // Turned off, or set up again: start over with what's there now.
                if modified(&config_path(home)) != since {
                    break Ok(());
                }
                // Squads started since: follow them too.
                for squad in equipe_core::squads::list(home) {
                    if squads.contains_key(&squad.name) {
                        continue;
                    }
                    match follow(socket, version, &squad.name, &mut bridge, &event_tx).await {
                        Ok((sender, outs)) => {
                            squads.insert(squad.name.clone(), sender);
                            deliver(&api, config.chat, &mut bridge, outs).await;
                        }
                        Err(err) => {
                            eprintln!("telegram: following {}: {err}", squad.name);
                            break;
                        }
                    }
                }
            }
        }
    };
    poller.abort();
    result
}

/// `equipe telegram setup`: make the bot this person's.
pub async fn setup(home: &Path) -> Result<String, String> {
    use std::io::Write as _;
    if let Some(config) = load(home) {
        return Err(format!("Already connected to @{} — `equipe telegram off` first to connect another bot", config.bot));
    }
    println!("Talk to your squad from Telegram: requests, reviews and questions come to your phone, with buttons to answer.\n");
    println!("1. In Telegram, open @BotFather, send /newbot and pick a name. It gives you a token.");
    println!("   (Messages go through Telegram's servers, encrypted to them — not end to end.)\n");
    let pasted = dialoguer::Password::new()
        .with_prompt("2. Paste the token here (or BotFather's whole message)")
        .interact()
        .map_err(|err| err.to_string())?;
    let Some(token) = token_in(&pasted) else {
        return Err(format!(
            "No bot token in what was pasted ({} characters) — it looks like 123456789:AAH… ; in @BotFather, /mybots › your bot › API Token shows it",
            pasted.chars().count()
        ));
    };
    let api = Api::new(&token);
    let me = api.call("getMe", json!({})).await.map_err(|why| match why.as_str() {
        "Not Found" | "Unauthorized" => format!("Telegram doesn't know that token ({why}) — copy it again from @BotFather: /mybots › your bot › API Token"),
        _ => format!("Couldn't reach Telegram: {why}"),
    })?;
    let bot = me["username"].as_str().ok_or("Telegram didn't say the bot's name")?.to_owned();
    let code = pairing_code();
    println!("\n3. Open https://t.me/{bot}?start={code} on your phone and press Start");
    println!("   (or send /start {code} to @{bot}). Waiting up to 10 minutes…");
    let _ = std::io::stdout().flush();
    let deadline = std::time::Instant::now() + Duration::from_secs(600);
    let mut offset = 0i64;
    while std::time::Instant::now() < deadline {
        let updates = api.call("getUpdates", json!({ "offset": offset, "timeout": 20, "allowed_updates": ["message"] })).await?;
        for update in updates.as_array().into_iter().flatten() {
            offset = offset.max(update["update_id"].as_i64().unwrap_or_default() + 1);
            let message = &update["message"];
            let text = message["text"].as_str().unwrap_or_default().trim();
            let private = message["chat"]["type"] == "private";
            let said = text.strip_prefix("/start").or_else(|| text.strip_prefix("/pair")).map(str::trim);
            if !private || said != Some(code.as_str()) {
                continue;
            }
            let (Some(user), Some(chat)) = (message["from"]["id"].as_i64(), message["chat"]["id"].as_i64()) else {
                continue;
            };
            // Past this update, so the daemon doesn't see the pairing again.
            let _ = api.call("getUpdates", json!({ "offset": offset, "timeout": 0 })).await;
            let config = Config { token, bot: bot.clone(), user, chat };
            save(home, &config).map_err(|err| format!("Couldn't keep the connection: {err}"))?;
            let commands = json!({ "commands": [
                { "command": "status", "description": "Where every squad stands" },
                { "command": "board", "description": "The board" },
                { "command": "squad", "description": "Which squad you write to" },
                { "command": "help", "description": "How this works" },
            ]});
            let _ = api.call("setMyCommands", commands).await;
            let host = hostname();
            let _ = api
                .send(chat, &format!("✅ Paired with equipe on <b>{}</b>.\n\n{}", escape(&host), Bridge::default().help()), None)
                .await;
            return Ok(format!("Paired with @{bot}. Requests, reviews and replies come to your Telegram from now on."));
        }
    }
    Err("Nobody sent the code in time — run `equipe telegram setup` again".into())
}

/// `equipe telegram off`.
pub fn off(home: &Path) -> Result<String, String> {
    match load(home) {
        Some(config) => {
            std::fs::remove_file(config_path(home)).map_err(|err| err.to_string())?;
            Ok(format!("Disconnected from @{}. The bot stays in Telegram until you delete it with @BotFather.", config.bot))
        }
        None => Ok("Telegram isn't connected".into()),
    }
}

/// `equipe telegram`: whether it's connected.
pub fn status(home: &Path) -> String {
    match load(home) {
        Some(config) => format!("Connected: @{} — it answers only your Telegram account. `equipe telegram off` disconnects.", config.bot),
        None => "Not connected. `equipe telegram setup` connects a bot of your own.".into(),
    }
}

/// The bot token in what was pasted — `<digits>:<letters, digits, _ and ->` —
/// wherever it sits: alone, inside BotFather's whole message, after `bot`,
/// or between the marks a terminal wraps a paste in.
fn token_in(pasted: &str) -> Option<String> {
    let chars: Vec<char> = pasted.chars().collect();
    let tail = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    for (colon, c) in chars.iter().enumerate() {
        if *c != ':' {
            continue;
        }
        let start = (0..colon).rev().take_while(|&i| chars[i].is_ascii_digit()).last().unwrap_or(colon);
        let end = (colon + 1..chars.len()).take_while(|&i| tail(chars[i])).last().map_or(colon + 1, |i| i + 1);
        if colon - start >= 5 && end - colon > 30 {
            return Some(chars[start..end].iter().collect());
        }
    }
    None
}

/// Six digits from the system's randomness.
fn pairing_code() -> String {
    use std::io::Read as _;
    let mut bytes = [0u8; 4];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    format!("{:06}", u32::from_le_bytes(bytes) % 1_000_000)
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "this machine".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::{Engine, Member, SessionInfo, Access};

    fn session(id: SessionId, name: &str, role: &str) -> SessionInfo {
        SessionInfo {
            session: id,
            project: "/repo".into(),
            member: Some(Member { role: role.into(), name: name.into() }),
            engine: Engine::Claude,
            model: None,
            access: Access::ReadOnly,
            workspace: None,
            state: SessionState::Ready,
            title: None,
            tokens: 0,
            context: None,
            recent: vec![],
            queued: vec![],
            queued_project: None,
            queued_in: vec![],
            steering: vec![],
            lane: None,
            work: None,
            mode: None,
        }
    }

    fn ticket(id: TicketId, status: TicketStatus) -> Ticket {
        Ticket {
            id,
            title: "Sign-in page".into(),
            body: String::new(),
            status,
            owner: Some("frontend".into()),
            assignee: Some("frontend".into()),
            project: None,
            depends_on: vec![],
            branch: None,
            worktree: None,
            merged: None,
            kind: equipe_core::protocol::TicketKind::Code,
            created_by: "pm".into(),
            note: Some("Done: <form> & tests".into()),
        }
    }

    /// A squad "acme" with its PM (1) and a frontend engineer (2).
    fn bridge() -> Bridge {
        let mut bridge = Bridge::default();
        let snapshot = Snapshot { squad: "acme".into(), sessions: vec![session(1, "pm", "pm"), session(2, "frontend", "frontend")], ..Snapshot::default() };
        assert!(bridge.attach(&snapshot).is_empty());
        bridge
    }

    fn buttons(out: &Out) -> Vec<(String, String)> {
        match out {
            Out::Send { buttons, .. } => buttons.clone(),
            other => panic!("not a message: {other:?}"),
        }
    }

    fn text(out: &Out) -> String {
        match out {
            Out::Send { text, .. } | Out::Edit { text, .. } => text.clone(),
            other => panic!("no text: {other:?}"),
        }
    }

    #[test]
    fn a_request_comes_with_buttons_and_says_how_it_was_answered() {
        let mut b = bridge();
        let approval = Approval { id: 7, session: 2, member: "frontend".into(), ticket: Some(3), tool: "Bash".into(), action: "npm i <pkg>".into(), reason: None, preview: None };
        let outs = b.on_event("acme", &Event::ApprovalRequested { squad: "acme".into(), approval });
        assert_eq!(outs.len(), 1);
        assert!(text(&outs[0]).contains("<b>frontend</b> asks to run · T-3") && text(&outs[0]).contains("npm i &lt;pkg&gt;"), "{outs:?}");
        let labels: Vec<String> = buttons(&outs[0]).into_iter().map(|(l, _)| l).collect();
        // Once, or not: never "always" from a phone.
        assert_eq!(labels, ["Allow", "Decline"]);
        b.sent(Key::Approval("acme".into(), 7), 100);
        let allow = buttons(&outs[0])[0].1.clone();
        let (commands, said) = b.on_button("cb", &allow);
        assert_eq!(commands, vec![("acme".to_owned(), Command::Decide { squad: "acme".into(), approval: 7, decision: Decision::Allow })]);
        assert_eq!(said, vec![Out::Toast { callback: "cb".into(), text: "Allowed".into() }]);
        let outs = b.on_event("acme", &Event::ApprovalResolved { squad: "acme".into(), approval: 7, decision: Decision::Allow });
        assert!(matches!(&outs[..], [Out::Edit { message: 100, text }] if text.ends_with("<b>✅ Allowed</b>")), "{outs:?}");
        // Pressed again, it was already answered.
        let (commands, said) = b.on_button("cb2", &allow);
        assert!(commands.is_empty());
        assert_eq!(said, vec![Out::Toast { callback: "cb2".into(), text: "Already answered".into() }]);
    }

    #[test]
    fn work_to_review_is_accepted_or_sent_back_from_the_phone() {
        let mut b = bridge();
        b.on_event("acme", &Event::TicketChanged { squad: "acme".into(), ticket: ticket(3, TicketStatus::InProgress) });
        let outs = b.on_event("acme", &Event::TicketChanged { squad: "acme".into(), ticket: ticket(3, TicketStatus::Review) });
        assert!(text(&outs[0]).contains("<b>T-3</b> Sign-in page") && text(&outs[0]).contains("Done: &lt;form&gt; &amp; tests"), "{outs:?}");
        let accept = buttons(&outs[0])[0].1.clone();
        b.sent(Key::Review("acme".into(), 3), 200);
        // A reply to it asks for changes.
        let (commands, _) = b.on_text("Make the button bigger", Some(200), 201);
        assert_eq!(commands, vec![("acme".to_owned(), Command::Reopen { squad: "acme".into(), ticket: 3, note: "Make the button bigger".into() })]);
        // Or the button accepts it.
        let (commands, _) = b.on_button("cb", &accept);
        assert_eq!(commands, vec![("acme".to_owned(), Command::Accept { squad: "acme".into(), ticket: 3 })]);
        let outs = b.on_event("acme", &Event::TicketChanged { squad: "acme".into(), ticket: ticket(3, TicketStatus::Done) });
        assert!(matches!(&outs[..], [Out::Edit { message: 200, text }] if text.ends_with("<b>✅ Accepted</b>")), "{outs:?}");
        // Done, its buttons do nothing.
        assert!(b.on_button("cb", &accept).0.is_empty());
    }

    #[test]
    fn a_ticket_waiting_on_the_person_goes_on_with_their_reply() {
        let mut b = bridge();
        let outs = b.on_event("acme", &Event::TicketChanged { squad: "acme".into(), ticket: ticket(4, TicketStatus::Waiting) });
        assert!(text(&outs[0]).contains("frontend needs you"), "{outs:?}");
        let done = buttons(&outs[0])[0].1.clone();
        let (_, asked) = b.on_button("cb", &done);
        let Some(Out::Send { ask: Some(_), key: Some(key), .. }) = asked.last() else { panic!("{asked:?}") };
        b.sent(key.clone(), 300);
        let (commands, said) = b.on_text("Added the DNS record", Some(300), 301);
        assert_eq!(commands, vec![("acme".to_owned(), Command::Resume { squad: "acme".into(), ticket: 4, note: "Added the DNS record".into() })]);
        assert_eq!(said, vec![Out::Seen { message: 301 }]);
    }

    #[test]
    fn messages_go_to_the_pm_or_a_named_member_and_their_answer_comes_back() {
        let mut b = bridge();
        let (commands, said) = b.on_text("Ship the site today?", None, 10);
        assert_eq!(commands, vec![("acme".to_owned(), Command::Submit { session: 1, text: "Ship the site today?".into() })]);
        assert_eq!(said, vec![Out::Seen { message: 10 }]);
        // What the PM was saying before the person's message isn't the answer.
        b.on_event("acme", &Event::TextDelta { session: 1, text: "earlier work".into() });
        b.on_event("acme", &Event::UserMessage { session: 1, text: "Ship the site today?".into() });
        b.on_event("acme", &Event::TextDelta { session: 1, text: "Checking the board.".into() });
        b.on_event("acme", &Event::ToolUse { session: 1, tool: "board".into(), target: None });
        b.on_event("acme", &Event::TextDelta { session: 1, text: "Yes — after T-3.".into() });
        let outs = b.on_event("acme", &Event::TurnEnded { session: 1, outcome: TurnOutcome::Completed, usage: None });
        assert_eq!(outs.len(), 1);
        assert_eq!(text(&outs[0]), "<b>pm</b>\nChecking the board.\n\nYes — after T-3.");
        let Out::Send { key: Some(key), .. } = &outs[0] else { panic!() };
        b.sent(key.clone(), 11);
        // A reply to it goes on talking with the PM; `@name` writes to a member.
        assert_eq!(b.on_text("Great", Some(11), 12).0, vec![("acme".to_owned(), Command::Submit { session: 1, text: "Great".into() })]);
        assert_eq!(b.on_text("@frontend how's T-3?", None, 13).0, vec![("acme".to_owned(), Command::Submit { session: 2, text: "how's T-3?".into() })]);
        assert!(b.on_text("@nobody hi", None, 14).0.is_empty());
        // A turn the person didn't start from here isn't sent.
        assert!(b.on_event("acme", &Event::TurnEnded { session: 2, outcome: TurnOutcome::Completed, usage: None }).is_empty());
    }

    #[test]
    fn a_message_to_a_busy_member_says_it_waits() {
        let mut b = bridge();
        b.on_text("One more thing", None, 10);
        let outs = b.on_event("acme", &Event::MessageQueued { session: 1, text: "One more thing".into(), project: None });
        assert!(text(&outs[0]).contains("pm is mid-turn"), "{outs:?}");
    }

    #[test]
    fn squads_and_status_and_commands() {
        let mut b = bridge();
        b.attach(&Snapshot { squad: "zeta".into(), sessions: vec![session(5, "pm", "pm")], ..Snapshot::default() });
        b.on_event("acme", &Event::TicketChanged { squad: "acme".into(), ticket: ticket(3, TicketStatus::Review) });
        let status = text(&b.on_text("/status", None, 1).1[0]);
        assert!(status.contains("<b>acme</b> ← you write here") && status.contains("1 to review"), "{status}");
        assert!(text(&b.on_text("/squad zeta", None, 2).1[0]).contains("Now writing to <b>zeta</b>"));
        assert_eq!(b.on_text("hello", None, 3).0, vec![("zeta".to_owned(), Command::Submit { session: 5, text: "hello".into() })]);
        assert!(text(&b.on_text("/board@my_bot", None, 4).1[0]).contains("<b>zeta</b>"));
        assert!(text(&b.on_text("/nope", None, 5).1[0]).contains("No command /nope"));
    }

    #[test]
    fn only_the_paired_person_is_answered() {
        let config = Config { token: "t".into(), bot: "b".into(), user: 42, chat: 42 };
        let mut b = bridge();
        let from = |user: i64, chat: i64| json!({ "message": { "message_id": 1, "from": { "id": user }, "chat": { "id": chat }, "text": "hi" } });
        assert!(!on_update(&config, &mut b, &from(42, 42)).0.is_empty());
        assert!(on_update(&config, &mut b, &from(7, 42)).0.is_empty());
        assert!(on_update(&config, &mut b, &from(42, 7)).0.is_empty(), "the person, but in a group");
        let press = json!({ "callback_query": { "id": "c", "from": { "id": 7 }, "data": "b1" } });
        assert_eq!(on_update(&config, &mut b, &press), (vec![], vec![]));
    }

    /// A stand-in for Telegram: answers each request with `reply`, and hands
    /// back the requests it got (path and body).
    async fn fake_telegram(reply: &'static str) -> (String, tokio::sync::mpsc::UnboundedReceiver<(String, Value)>) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut got = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = stream.read(&mut buf).await.unwrap();
                    got.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&got).into_owned();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length = head.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length: ").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                        if body.len() >= length {
                            let path = head.split_whitespace().nth(1).unwrap().to_owned();
                            let _ = tx.send((path, serde_json::from_str(body).unwrap_or(Value::Null)));
                            break;
                        }
                    }
                }
                let answer = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len());
                stream.write_all(answer.as_bytes()).await.unwrap();
            }
        });
        (base, rx)
    }

    #[tokio::test]
    async fn requests_reach_telegram_with_their_buttons_and_errors_never_show_the_token() {
        let (base, mut got) = fake_telegram(r#"{"ok":true,"result":{"message_id":55}}"#).await;
        let api = Api { http: reqwest::Client::new(), base: format!("{base}/bot123:secret") };
        let mut b = bridge();
        let outs = b.on_event("acme", &Event::TicketChanged { squad: "acme".into(), ticket: ticket(3, TicketStatus::Review) });
        deliver(&api, 42, &mut b, outs).await;
        let (path, body) = got.recv().await.unwrap();
        assert_eq!(path, "/bot123:secret/sendMessage");
        assert_eq!(body["chat_id"], 42);
        assert_eq!(body["parse_mode"], "HTML");
        assert_eq!(body["reply_markup"]["inline_keyboard"][0][0]["text"], "Accept");
        // The id Telegram gave it is what a reply refers to.
        assert_eq!(b.on_text("Bigger", Some(55), 56).0.len(), 1);

        let (base, _) = fake_telegram(r#"{"ok":false,"description":"Unauthorized"}"#).await;
        let api = Api { http: reqwest::Client::new(), base: format!("{base}/bot123:secret") };
        assert_eq!(api.call("getMe", json!({})).await, Err("Unauthorized".into()));
        let api = Api { http: reqwest::Client::new(), base: "http://127.0.0.1:1/bot123:secret".into() };
        let err = api.call("getMe", json!({})).await.unwrap_err();
        assert!(!err.contains("secret"), "{err}");
    }

    #[test]
    fn long_replies_are_cut_into_pieces_telegram_takes() {
        let long = "line\n".repeat(2000);
        let pieces = chunks(&long);
        assert!(pieces.len() > 1 && pieces.iter().all(|p| p.chars().count() <= CHUNK));
        assert_eq!(pieces.concat(), long);
        assert_eq!(chunks(&"x".repeat(8000)).len(), 3);
    }

    #[test]
    fn the_token_is_found_in_whatever_was_pasted() {
        let token = "7412345678:AAHk3jX_9d-Qw2LmNoPqRsTuVwXyZ012345";
        assert_eq!(token_in(token).as_deref(), Some(token));
        assert_eq!(token_in(&format!("  {token}\n")).as_deref(), Some(token));
        // Wrapped by the terminal's paste marks, or after "bot".
        assert_eq!(token_in(&format!("\x1b[200~{token}\x1b[201~")).as_deref(), Some(token));
        assert_eq!(token_in(&format!("bot{token}")).as_deref(), Some(token));
        // BotFather's whole message.
        let message = format!("Done! Congratulations on your new bot.\nUse this token to access the HTTP API:\n{token}\nKeep your token secure");
        assert_eq!(token_in(&message).as_deref(), Some(token));
        assert_eq!(token_in("hello"), None);
        assert_eq!(token_in("12:34"), None);
    }

    #[test]
    fn the_connection_is_kept_where_only_the_person_reads_it() {
        use std::os::unix::fs::PermissionsExt as _;
        let home = std::env::temp_dir().join(format!("equipe-telegram-{}", std::process::id()));
        let config = Config { token: "123:abc".into(), bot: "acme_bot".into(), user: 1, chat: 1 };
        save(&home, &config).unwrap();
        assert_eq!(load(&home), Some(config));
        assert_eq!(std::fs::metadata(config_path(&home)).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(off(&home).unwrap().contains("@acme_bot"));
        assert_eq!(load(&home), None);
        std::fs::remove_dir_all(&home).unwrap();
        assert_eq!(pairing_code().len(), 6);
    }
}
