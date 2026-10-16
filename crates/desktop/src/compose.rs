//! What typing into the conversation does, without a window: the history that
//! ↑↓ walks, a draft kept per member and project, the guard that keeps a Korean
//! composition's Enter from sending, completion from the squad's state, and what
//! a sent line turns into — a message to a member, or one of the person's
//! commands. The same rules as the terminal console (`tui/app.rs` `submit`,
//! `slash`); the daemon commands are the ones it sends.

use equipe_core::i18n::label;
use equipe_core::t;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use equipe_core::board::parse_ticket_id;
use equipe_core::protocol::{Command, Engine, Mode, SessionId, TicketId, TicketStatus};
use serde::{Deserialize, Serialize};

use crate::attach::Saved;
use crate::commands::{self, Choice};
use crate::files;
use crate::state::{Model, Presence};

// ── history ──

/// What was sent, for ↑ to bring back: the words and what was attached.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Said {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Saved>,
}

impl Said {
    #[cfg(test)]
    pub fn text(text: &str) -> Self {
        Self { text: text.to_owned(), attachments: Vec::new() }
    }

    fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.attachments.is_empty()
    }
}

/// How many sent lines ↑ can reach.
const HISTORY: usize = 200;

/// The lines the person sent, for ↑ to bring back (only from an empty box).
#[derive(Debug, Default)]
pub struct History {
    lines: Vec<Said>,
    /// Where ↑↓ stands while walking, as an index into `lines`.
    at: Option<usize>,
}

impl History {
    /// A line was sent: remember it (not twice in a row), stop walking.
    pub fn push(&mut self, said: Said) {
        let said = Said { text: said.text.trim().to_owned(), ..said };
        if !said.is_empty() && self.lines.last() != Some(&said) {
            self.lines.push(said);
            if self.lines.len() > HISTORY {
                self.lines.remove(0);
            }
        }
        self.at = None;
    }

    /// ↑. `current` is what the box holds: walking starts only from empty, and
    /// goes on while the box still shows a recalled line.
    pub fn older(&mut self, current: &str) -> Option<&Said> {
        if self.lines.is_empty() || (self.at.is_none() && !current.is_empty()) {
            return None;
        }
        let i = match self.at {
            None => self.lines.len() - 1,
            Some(i) => i.saturating_sub(1),
        };
        self.at = Some(i);
        Some(&self.lines[i])
    }

    /// ↓: toward the newest line, then an empty box.
    pub fn newer(&mut self) -> Option<Said> {
        let i = self.at?;
        if i + 1 < self.lines.len() {
            self.at = Some(i + 1);
            Some(self.lines[i + 1].clone())
        } else {
            self.at = None;
            Some(Said::default())
        }
    }

    /// Whether ↑↓ is walking.
    pub fn walking(&self) -> bool {
        self.at.is_some()
    }

    /// The person typed: whatever is in the box is theirs now.
    pub fn stop(&mut self) {
        self.at = None;
    }
}

// ── drafts ──

/// What was half written to one member: the words, where the caret was, and
/// what was attached.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    #[serde(default)]
    pub text: String,
    /// The caret, as a byte offset into `text`.
    #[serde(default)]
    pub caret: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Saved>,
}

impl Draft {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.attachments.is_empty()
    }
}

/// How many files "최근 첨부" offers.
pub const RECENT: usize = 10;

/// What was half written to each member, per squad and project in focus,
/// and the files attached lately — kept on disk, so a restart loses nothing.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Drafts {
    #[serde(default)]
    drafts: HashMap<String, Draft>,
    /// Files attached lately, newest first.
    #[serde(default)]
    pub recent: Vec<PathBuf>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl Drafts {
    /// Where they are kept.
    pub fn file(home: &Path) -> PathBuf {
        home.join("desktop-composer.json")
    }

    /// The drafts kept at `path` (none when it can't be read).
    pub fn load(path: PathBuf) -> Self {
        let mut drafts: Self = std::fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        drafts.path = Some(path);
        drafts
    }

    /// The key a draft is kept under: by names, which outlive a daemon's
    /// session numbers.
    pub fn key(squad: &str, member: &str, project: Option<&str>) -> String {
        format!("{squad}/{member}/{}", project.unwrap_or(""))
    }

    pub fn put(&mut self, key: String, draft: Draft) {
        if draft.is_empty() {
            self.drafts.remove(&key);
        } else {
            self.drafts.insert(key, draft);
        }
    }

    pub fn get(&self, key: &str) -> Draft {
        self.drafts.get(key).cloned().unwrap_or_default()
    }

    /// A file was attached: it heads the recent ones.
    pub fn used(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_owned());
        self.recent.truncate(RECENT);
    }

    /// What `save` would write, and where; `None` for drafts kept nowhere.
    pub fn snapshot(&self) -> Option<(PathBuf, String)> {
        Some((self.path.clone()?, serde_json::to_string_pretty(self).ok()?))
    }

    /// Write them down (blocking; small).
    pub fn save(&self) {
        if let Some((path, text)) = self.snapshot() {
            write_private(&path, &text);
        }
    }
}

/// Write `text` at `path`, readable by its owner only, replacing what was there whole.
pub fn write_private(path: &Path, text: &str) {
    use std::os::unix::fs::OpenOptionsExt as _;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    let written = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()));
    if written.is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

// ── Korean composition ──

/// Keeps the Enter that ends a Korean composition from also sending.
///
/// macOS's Korean input source, asked to finish a syllable with Enter, commits
/// it and — depending on the app — hands the Enter on as well. Two rules
/// (T-54 §3.5): while text is being composed an Enter only commits; and an
/// Enter arriving in the same breath as the composition ending is that same
/// key press, not a second one.
#[derive(Debug, Default)]
pub struct EnterGuard {
    composing: bool,
    ended: Option<Instant>,
}

/// Longer than one key press takes to reach us, far shorter than typing twice.
const SAME_PRESS: Duration = Duration::from_millis(40);

impl EnterGuard {
    /// The box changed; `composing` is whether it holds text still being composed.
    pub fn observe(&mut self, composing: bool, now: Instant) {
        if self.composing && !composing {
            self.ended = Some(now);
        }
        self.composing = composing;
    }

    /// An Enter arrived. True when it should send; false when it was only the
    /// end of a composition.
    pub fn allows_send(&mut self, now: Instant) -> bool {
        if self.composing {
            return false;
        }
        !matches!(self.ended.take(), Some(at) if now.saturating_duration_since(at) < SAME_PRESS)
    }

    pub fn composing(&self) -> bool {
        self.composing
    }
}

// ── completion ──

/// How many rows `@` lists in each of its groups.
const PER_GROUP: usize = 6;

/// What `@` offers for the word being typed at the end of `text`: members
/// (only as the message's first word — `@name …` goes to them), the
/// projects' files, the tickets. Fuzzy, best first, each group headed.
pub fn mentions(model: &Model, text: &str, indexes: &[files::Index]) -> Vec<Choice> {
    let start = text.rfind(char::is_whitespace).map_or(0, |i| i + text[i..].chars().next().map_or(1, char::len_utf8));
    let Some(query) = text[start..].strip_prefix('@') else {
        return Vec::new();
    };
    let before = &text[..start];
    let mut out = Vec::new();
    if start == 0 {
        let members = model.members.iter().filter(|m| m.presence != Presence::Closed);
        let mut found: Vec<(i32, Choice)> = members
            .filter_map(|m| {
                let (score, hits) = commands::fuzzy(query, &m.name)?;
                let choice = Choice { hits: hits.into_iter().map(|h| h + 1).collect(), group: Some(t!("commands.group_members")), ..Choice::new(format!("@{} ", m.name), format!("@{}", m.name), m.role.clone()) };
                Some((score, choice))
            })
            .collect();
        found.sort_by_key(|(score, _)| -score);
        out.extend(found.into_iter().take(PER_GROUP).map(|(_, c)| c));
    }
    if !query.is_empty() {
        let mut found: Vec<(i32, Choice)> = indexes
            .iter()
            .flat_map(|index| index.files.iter().map(move |f| (index, f)))
            .filter_map(|(index, file)| {
                let (score, hits) = commands::fuzzy(query, file)?;
                // The file's own name counts more than its folders.
                let name_at = file.rfind('/').map_or(0, |i| i + 1);
                let in_name = hits.first().is_some_and(|h| *h >= file[..name_at].chars().count());
                let ext = Path::new(file).extension().map(|e| e.to_string_lossy().to_ascii_uppercase());
                let choice = Choice {
                    hits,
                    group: Some(t!("commands.group_files")),
                    badge: ext,
                    ..Choice::new(format!("{before}@{file} "), file.clone(), index.project.clone())
                };
                Some((score + if in_name { 40 } else { 0 } - file.len() as i32 / 8, choice))
            })
            .collect();
        found.sort_by_key(|(score, _)| -score);
        out.extend(found.into_iter().take(PER_GROUP).map(|(_, c)| c));
    }
    let mut tickets: Vec<(i32, Choice)> = model
        .tickets
        .iter()
        .filter(|t| t.status != TicketStatus::Done)
        .filter_map(|t| {
            let label = format!("T-{} {}", t.id, t.title);
            let (score, hits) = commands::fuzzy(query.trim_start_matches(['T', 't', '-']), &label).or_else(|| commands::fuzzy(query, &label))?;
            Some((score, Choice { hits, group: Some(t!("commands.group_tickets")), ..Choice::new(format!("{before}T-{} ", t.id), label, String::new()) }))
        })
        .collect();
    tickets.sort_by_key(|(score, _)| -score);
    out.extend(tickets.into_iter().take(PER_GROUP).map(|(_, c)| c));
    // Nothing more to offer once the word is the only choice, whole.
    match out.as_slice() {
        [only] if only.text.trim_end() == text.trim_end() => Vec::new(),
        _ => out,
    }
}

/// What completion offers for `text`, drawn from the squad as it stands.
#[cfg(test)]
pub fn choices(model: &Model, text: &str) -> Vec<Choice> {
    choices_with(model, text, &[])
}

/// The same, with the projects' files for `@`.
pub fn choices_with(model: &Model, text: &str, indexes: &[files::Index]) -> Vec<Choice> {
    if text.is_empty() {
        return Vec::new();
    }
    if !text.starts_with('/') {
        return mentions(model, text, indexes);
    }
    if text.contains('\n') {
        return Vec::new();
    }
    let tickets = |statuses: &[TicketStatus]| -> Vec<(TicketId, &str)> {
        model.tickets.iter().filter(|t| statuses.contains(&t.status)).map(|t| (t.id, t.title.as_str())).collect()
    };
    let selected_models = model.selected_member().map(|m| engine_models(m.engine, m.model.as_deref())).unwrap_or_default();
    let ctx = commands::Context {
        review: tickets(&[TicketStatus::Review]),
        reopenable: tickets(&[TicketStatus::Review, TicketStatus::Done]),
        waiting: tickets(&[TicketStatus::Waiting]),
        open: tickets(&[TicketStatus::Proposed, TicketStatus::Todo, TicketStatus::InProgress, TicketStatus::Waiting, TicketStatus::Review]),
        unmerged: model.tickets.iter().filter(|t| t.not_on_main()).map(|t| (t.id, t.title.as_str())).collect(),
        members: model.members.iter().filter(|m| m.presence != Presence::Closed).map(|m| (m.name.as_str(), m.role.as_str())).collect(),
        roles: equipe_core::ROLES.iter().map(|r| r.id).collect(),
        // Other squads are the sidebar's business here.
        squads: Vec::new(),
        mode: Some(model.mode),
        projects: model
            .project_names()
            .into_iter()
            .map(|name| {
                let about = if model.focus.as_deref() == Some(name.as_str()) { t!("compose.in_focus") } else { "" };
                (name, about.to_owned())
            })
            .collect(),
        member_modes: Vec::new(),
        member_models: model.members.iter().map(|m| (m.name.as_str(), engine_models(m.engine, m.model.as_deref()))).collect(),
        selected_models,
    };
    commands::complete(text, &ctx)
}

pub fn engine_models(engine: Option<Engine>, current: Option<&str>) -> Vec<String> {
    let mut models: Vec<String> = match engine {
        Some(Engine::Claude) => ["opus", "sonnet", "haiku"].iter().map(|m| (*m).to_owned()).collect(),
        _ => Vec::new(),
    };
    if let Some(current) = current.filter(|m| !models.iter().any(|x| x == m)) {
        models.push(current.to_owned());
    }
    models
}

// ── sending ──

/// What the window does besides sending commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Talk to this member (a bare `@name`).
    Select(SessionId),
    ShowBoard,
    /// The board, narrowed to these words (`/find`).
    Find(String),
    /// Put this project (or all) in focus.
    Focus(Option<String>),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub commands: Vec<Command>,
    pub flash: Option<String>,
    pub effect: Option<Effect>,
    /// Text that couldn't be sent, to go back into the box.
    pub restore: Option<String>,
}

impl Plan {
    fn say(text: impl Into<String>) -> Self {
        Self { flash: Some(text.into()), ..Self::default() }
    }

    fn send(command: Command) -> Self {
        Self { commands: vec![command], ..Self::default() }
    }
}

/// What a sent line comes to. `text` is the box's content; the selected
/// member is who it is for unless `@name` says otherwise.
///
/// `later` (Tab, or the memo button) leaves a member at work a memo for when
/// its turn ends instead of steering the work.
pub fn plan(model: &Model, text: &str, later: bool) -> Plan {
    let text = text.trim();
    if text.is_empty() {
        return Plan::default();
    }
    if let Some(command) = text.strip_prefix('/') {
        return slash(model, command);
    }
    let Some(selected) = model.selected_member() else {
        return Plan { restore: Some(text.to_owned()), ..Plan::say(t!("compose.no_member_yet")) };
    };
    // `@src/main.rs …` names a file, not a member: it is said to the one in view.
    let names_file = |mention: &str| {
        let name = mention.split(char::is_whitespace).next().unwrap_or("");
        model.open_member(name).is_none() && name.contains(['/', '.'])
    };
    let (target, text) = match text.strip_prefix('@').filter(|m| !names_file(m)) {
        // `@frontend fix the header` goes to frontend without leaving this view.
        Some(mention) => {
            let (name, rest) = mention.split_once(char::is_whitespace).unwrap_or((mention, ""));
            match model.open_member(name) {
                Some(member) if rest.trim().is_empty() => return Plan { effect: Some(Effect::Select(member.session)), ..Plan::default() },
                Some(member) => (member, rest.trim()),
                None => return Plan { restore: Some(text.to_owned()), ..Plan::say(t!("compose.no_member_mention", name = name)) },
            }
        }
        None => (selected, text),
    };
    // A ticket open: what is said steers its work, while its member is on it;
    // otherwise it goes to the PM, about the ticket.
    let about;
    let (target, text) = match model.opened {
        Some(ticket) if target.session == selected.session && target.ticket != Some(ticket) => {
            match model.members.iter().find(|m| Model::is_pm(m) && m.presence != Presence::Closed) {
                Some(pm) => {
                    about = format!("About T-{ticket}: {text}");
                    (pm, about.as_str())
                }
                None => (target, text),
            }
        }
        _ => (target, text),
    };
    let away = target.session != selected.session;
    // What is said to the one in view goes to its conversation about the
    // project in focus: at work there or in a lane, or free if the member is
    // at work in another project's — the daemon then opens a lane beside it.
    let presence = if away { model.presence_to_talk(target) } else { model.viewed_presence().unwrap_or(target.presence) };
    match presence {
        Presence::Closed => Plan { restore: Some(text.to_owned()), ..Plan::say(t!("compose.member_gone", name = target.name)) },
        // A memo: the core holds it until the turn ends. Only Claude can be steered.
        Presence::Working if later || target.engine != Some(Engine::Claude) => Plan {
            commands: vec![Command::SubmitLater { session: target.session, text: text.to_owned() }],
            flash: Some(t!("compose.memo_left", name = target.name)),
            ..Plan::default()
        },
        // Steering: read at its next step, the work carrying on.
        Presence::Working => Plan {
            commands: vec![Command::Submit { session: target.session, text: text.to_owned() }],
            flash: Some(t!("compose.steered", name = target.name)),
            ..Plan::default()
        },
        Presence::Ready if !away && model.opens_lane() => Plan {
            commands: vec![Command::Submit { session: target.session, text: text.to_owned() }],
            flash: Some(t!("compose.answers_alongside", name = target.name)),
            ..Plan::default()
        },
        Presence::Ready => Plan {
            commands: vec![Command::Submit { session: target.session, text: text.to_owned() }],
            flash: away.then(|| t!("compose.sent_to", name = target.name)),
            ..Plan::default()
        },
    }
}

fn slash(model: &Model, command: &str) -> Plan {
    let (name, rest) = command.split_once(' ').unwrap_or((command, ""));
    let rest = rest.trim();
    let squad = model.squad.clone();
    let member_named = |n: &str| model.members.iter().find(|m| m.name == n && m.presence != Presence::Closed);
    match name {
        "accept" | "merge" | "reopen" | "resume" | "cancel" => decide(model, name, rest),
        "board" => Plan { effect: Some(Effect::ShowBoard), ..Plan::default() },
        "find" => Plan { effect: Some(Effect::Find(rest.to_owned())), ..Plan::default() },
        "project" => {
            let wanted = rest;
            let project = match wanted {
                "" | "all" => None,
                name => match model.project_names().into_iter().find(|p| p == name) {
                    Some(p) => Some(p),
                    None => {
                        return Plan::say(t!("compose.no_project", squad = model.squad, name = name, projects = model.project_names().join(", ")));
                    }
                },
            };
            Plan {
                commands: vec![Command::Focus { squad, project: project.clone() }],
                effect: Some(Effect::Focus(project)),
                ..Plan::default()
            }
        }
        "tidy" => match model.members.iter().find(|m| Model::is_pm(m) && m.presence != Presence::Closed) {
            Some(pm) => Plan::send(Command::Submit {
                session: pm.session,
                text: "Look the board over for tickets that have gone stale, overlap or no longer fit, and propose \
what to do with each — close, merge into another, or keep, and why. Don't cancel anything yourself; I'll decide."
                    .into(),
            }),
            None => Plan::say(t!("compose.no_pm")),
        },
        "mode" => set_mode(model, rest),
        "close" => match model.selected_member().filter(|m| m.presence != Presence::Closed) {
            Some(m) => Plan::send(Command::Close { session: m.session }),
            None => Plan::default(),
        },
        "scale" => match rest {
            "" => Plan::say(t!("compose.scale_usage")),
            word => match if word == "off" { Some(0) } else { word.parse::<usize>().ok().filter(|n| *n <= 6) } {
                Some(extra) => Plan {
                    commands: vec![Command::Scale { squad, extra }],
                    flash: Some(if extra == 0 { t!("compose.scale_off").into() } else { t!("compose.scale_set", extra = extra) }),
                    ..Plan::default()
                },
                None => Plan::say(t!("compose.scale_bad")),
            },
        },
        "compact" | "clear" => {
            let target = match rest {
                "" => model.selected_member(),
                n => member_named(n),
            };
            match target {
                Some(m) => Plan::send(Command::Refresh { session: m.session, summary: name == "compact" }),
                None => Plan::say(t!("compose.who", name = name)),
            }
        }
        "model" => {
            let words: Vec<&str> = rest.split_whitespace().collect();
            let (target, wanted) = match words.as_slice() {
                [n, m] => (member_named(n), *m),
                [m] => (model.selected_member(), *m),
                _ => return Plan::say(t!("compose.model_usage")),
            };
            match target {
                Some(t) => Plan {
                    commands: vec![Command::SetModel { session: t.session, model: wanted.to_owned() }],
                    flash: Some(t!("compose.model_set", name = t.name, wanted = wanted)),
                    ..Plan::default()
                },
                None => Plan::say(t!("compose.model_no_member", name = words[0])),
            }
        }
        "desktop" => match rest {
            "stop" => Plan { commands: vec![Command::Desktop { open: false }], flash: Some(t!("compose.desktop_stopped").into()), ..Plan::default() },
            "resume" => Plan { commands: vec![Command::Desktop { open: true }], flash: Some(t!("compose.desktop_resumed").into()), ..Plan::default() },
            _ => Plan::say("/desktop stop · /desktop resume"),
        },
        "browser" => {
            let words: Vec<&str> = rest.split_whitespace().collect();
            match words.as_slice() {
                [verb @ ("show" | "hide")] => Plan::send(Command::ShowBrowser { squad, member: None, show: *verb == "show" }),
                [verb @ ("show" | "hide"), who] => match member_named(who) {
                    Some(m) => Plan::send(Command::ShowBrowser { squad, member: Some(m.name.clone()), show: *verb == "show" }),
                    None => Plan::say(t!("compose.no_member", who = who)),
                },
                _ => Plan::say(t!("compose.browser_usage")),
            }
        }
        "help" => Plan::say(t!("compose.help")),
        other => Plan::say(t!("compose.not_in_desktop", other = other)),
    }
}

/// `/accept T-3`, `/reopen T-3 what to change`, `/resume T-3 what I did`,
/// `/cancel T-3`, `/merge T-3 [push]`: the person's calls, straight to equipe —
/// never through a member.
fn decide(model: &Model, verb: &str, rest: &str) -> Plan {
    let (id, note) = rest.split_once(' ').unwrap_or((rest, ""));
    let note = note.trim();
    let Some(ticket) = parse_ticket_id(id).filter(|_| !id.is_empty()) else {
        let (status, what) = match verb {
            "resume" => (TicketStatus::Waiting, t!("compose.waiting_on_person")),
            _ => (TicketStatus::Review, t!("compose.waiting_for_review")),
        };
        let waiting: Vec<String> = model.tickets.iter().filter(|t| t.status == status).map(|t| format!("T-{}", t.id)).collect();
        return Plan::say(match waiting.as_slice() {
            [] => t!("compose.which_ticket", verb = verb),
            ids => t!("compose.which_ticket_of", what = what, tickets = ids.join(", ")),
        });
    };
    let squad = model.squad.clone();
    match verb {
        "accept" => {
            let merges = model.tickets.iter().find(|t| t.id == ticket).is_some_and(|t| !t.kind.in_home() && t.branch.is_some());
            Plan {
                commands: vec![Command::Accept { squad, ticket }],
                flash: Some(if merges { t!("compose.accepted_merge", ticket = ticket) } else { t!("compose.accepted", ticket = ticket) }),
                ..Plan::default()
            }
        }
        "merge" => {
            let push = note == "push";
            Plan {
                commands: vec![Command::Merge { squad, ticket, push }],
                flash: Some(if push { t!("compose.merging_push", ticket = ticket) } else { t!("compose.merging", ticket = ticket) }),
                ..Plan::default()
            }
        }
        // Sent back with no word, the member can only guess what to change.
        "reopen" if note.is_empty() => Plan {
            restore: Some(format!("/reopen T-{ticket} ")),
            ..Plan::say(t!("compose.reopen_needs_note", ticket = ticket))
        },
        "reopen" => Plan::send(Command::Reopen { squad, ticket, note: note.to_owned() }),
        "resume" => Plan::send(Command::Resume { squad, ticket, note: note.to_owned() }),
        _ => Plan::send(Command::Cancel { squad, ticket }),
    }
}

/// `/mode auto` for the squad, `/mode frontend ask` for one member,
/// `/mode frontend squad` to put it back on the squad's.
fn set_mode(model: &Model, rest: &str) -> Plan {
    let words: Vec<&str> = rest.split_whitespace().collect();
    let usage = || {
        // The word typed for each, and what it means in the person's language.
        let lang = crate::i18n::lang();
        let modes: Vec<String> = Mode::ALL.iter().zip(commands::MODES).map(|(m, word)| format!("{word} ({})", label::mode_short(lang, *m))).collect();
        t!("compose.mode_usage", modes = modes.join(" · "))
    };
    let squad = model.squad.clone();
    if words.contains(&"plan") {
        return Plan::say(t!("compose.mode_plan"));
    }
    match words.as_slice() {
        [] => Plan::say(t!("compose.mode_current", mode = label::mode(crate::i18n::lang(), model.mode), usage = usage())),
        [mode] => match Mode::parse(mode) {
            Some(mode) => Plan::send(Command::SetMode { squad, member: None, mode: Some(mode) }),
            None => Plan::say(usage()),
        },
        [name, mode, ..] => {
            if model.open_member(name).is_none() {
                return Plan::say(t!("compose.no_member_in_squad", name = name));
            }
            let mode = match *mode {
                "squad" | "default" => None,
                other => match Mode::parse(other) {
                    Some(mode) => Some(mode),
                    None => return Plan::say(usage()),
                },
            };
            Plan::send(Command::SetMode { squad, member: Some((*name).to_owned()), mode })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::Update;
    use crate::state::Clock;
    use equipe_core::protocol::rpc::Hello;
    use equipe_core::protocol::{Access, Event, Member as Who, SessionInfo, SessionState, Snapshot};

    const T0: Clock = Clock { hour: 14, minute: 2, second: 10 };

    fn session(id: SessionId, role: &str, state: SessionState) -> SessionInfo {
        SessionInfo {
            session: id,
            project: "/p".into(),
            member: Some(Who { role: role.into(), name: role.into() }),
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
            work: None,
            mode: None,
            effort: None,
            subagents: Vec::new(),
            queued_project: None,
            queued_in: vec![],
            steering: vec![],
            lane: None,
            estimate: None,
        }
    }

    fn squad(sessions: Vec<SessionInfo>) -> Model {
        let mut model = Model::new("acme");
        let snapshot = Snapshot { squad: "acme".into(), sessions, projects: vec!["/code/site".into(), "/code/app".into()], ..Default::default() };
        model.apply(Update::Attached(Hello { version: "x".into(), snapshot }), T0);
        model
    }

    fn team() -> Model {
        squad(vec![session(1, "pm", SessionState::Ready), session(2, "frontend", SessionState::Ready), session(3, "backend", SessionState::Working)])
    }

    fn plan(model: &Model, text: &str) -> Plan {
        super::plan(model, text, false)
    }

    fn submit(session: SessionId, text: &str) -> Command {
        Command::Submit { session, text: text.into() }
    }

    #[test]
    fn a_line_goes_to_the_member_on_screen() {
        let model = team();
        assert_eq!(model.selected, Some(1), "the PM is where the window starts");
        assert_eq!(plan(&model, "  안녕하세요  "), Plan { commands: vec![submit(1, "안녕하세요")], ..Plan::default() });
        assert_eq!(plan(&model, "   "), Plan::default());
    }

    #[test]
    fn the_pm_on_a_ticket_of_its_own_answers_beside_it_and_others_are_steered() {
        let mut model = squad(vec![session(1, "pm", SessionState::Working), session(2, "frontend", SessionState::Working)]);
        // The PM at work at its desk: steered.
        assert!(plan(&model, "이거 해").flash.unwrap().contains("다음 단계에서 읽습니다"));
        // On a ticket of its own: free to talk, beside the work.
        model.members[0].ticket = Some(4);
        let p = plan(&model, "앱은?");
        assert_eq!(p.commands, vec![submit(1, "앱은?")]);
        assert!(p.flash.unwrap().contains("나란히"));
        // A member on a ticket: what is said steers it.
        model.members[1].ticket = Some(5);
        model.select(2);
        assert!(plan(&model, "푸터는 빼").flash.unwrap().contains("다음 단계에서 읽습니다"));
    }

    #[test]
    fn a_mention_sends_to_that_member_without_leaving() {
        let model = team();
        let p = plan(&model, "@frontend 헤더 고쳐 줘");
        assert_eq!(p.commands, vec![submit(2, "헤더 고쳐 줘")]);
        assert_eq!(p.flash.as_deref(), Some("frontend에게 보냈습니다"));
        assert_eq!(p.effect, None);
    }

    #[test]
    fn a_bare_mention_goes_to_talk_to_that_member() {
        let model = team();
        assert_eq!(plan(&model, "@frontend"), Plan { effect: Some(Effect::Select(2)), ..Plan::default() });
    }

    #[test]
    fn a_mention_of_nobody_keeps_the_draft() {
        let model = team();
        let p = plan(&model, "@ghost hi");
        assert!(p.commands.is_empty());
        assert_eq!(p.restore.as_deref(), Some("@ghost hi"));
        assert!(p.flash.unwrap().contains("@ghost"));
    }

    #[test]
    fn a_message_to_a_working_member_steers_it_and_does_not_claim_a_memo() {
        let model = team();
        let p = plan(&model, "@backend 이것도");
        assert_eq!(p.commands, vec![submit(3, "이것도")]);
        let flash = p.flash.unwrap();
        assert!(flash.contains("다음 단계") && !flash.contains("메모를 남겼"), "{flash}");
    }

    #[test]
    fn tab_leaves_a_working_member_a_memo() {
        let model = team();
        let p = super::plan(&model, "@backend 이것도", true);
        assert_eq!(p.commands, vec![Command::SubmitLater { session: 3, text: "이것도".into() }]);
        assert!(p.flash.unwrap().contains("메모를 남겼"));
        // A member at rest is simply told: there is nothing to wait for.
        assert_eq!(super::plan(&model, "@frontend 안녕", true).commands, vec![submit(2, "안녕")]);
    }

    #[test]
    fn a_codex_member_at_work_always_gets_a_memo() {
        let mut codex = session(3, "backend", SessionState::Working);
        codex.engine = Engine::Codex;
        let model = squad(vec![session(1, "pm", SessionState::Ready), codex]);
        let p = plan(&model, "@backend 이것도");
        assert_eq!(p.commands, vec![Command::SubmitLater { session: 3, text: "이것도".into() }]);
    }

    #[test]
    fn a_member_who_left_gets_nothing() {
        let mut model = team();
        model.apply(Update::Event(Event::SessionClosed { session: 2 }), T0);
        let p = plan(&model, "@frontend hi");
        assert!(p.commands.is_empty() && p.restore.is_some());
    }

    #[test]
    fn the_persons_calls_go_straight_to_equipe() {
        let model = team();
        assert_eq!(plan(&model, "/accept T-8").commands, vec![Command::Accept { squad: "acme".into(), ticket: 8 }]);
        assert_eq!(plan(&model, "/cancel 3").commands, vec![Command::Cancel { squad: "acme".into(), ticket: 3 }]);
        assert_eq!(plan(&model, "/reopen T-3 헤더가 깨짐").commands, vec![Command::Reopen { squad: "acme".into(), ticket: 3, note: "헤더가 깨짐".into() }]);
        assert_eq!(plan(&model, "/resume T-4 토큰 넣음").commands, vec![Command::Resume { squad: "acme".into(), ticket: 4, note: "토큰 넣음".into() }]);
        assert_eq!(plan(&model, "/merge T-3 push").commands, vec![Command::Merge { squad: "acme".into(), ticket: 3, push: true }]);
        assert_eq!(plan(&model, "/merge T-3").commands, vec![Command::Merge { squad: "acme".into(), ticket: 3, push: false }]);
        // Sent back with no word: the number stays in the box for the person to finish.
        let bare = plan(&model, "/reopen T-3");
        assert!(bare.commands.is_empty());
        assert_eq!(bare.restore.as_deref(), Some("/reopen T-3 "));
        assert!(plan(&model, "/accept").commands.is_empty());
    }

    #[test]
    fn project_puts_a_project_in_focus_and_tells_the_daemon() {
        let model = team();
        let p = plan(&model, "/project site");
        assert_eq!(p.commands, vec![Command::Focus { squad: "acme".into(), project: Some("site".into()) }]);
        assert_eq!(p.effect, Some(Effect::Focus(Some("site".into()))));
        assert_eq!(plan(&model, "/project all").effect, Some(Effect::Focus(None)));
        assert!(plan(&model, "/project nope").commands.is_empty());
    }

    #[test]
    fn modes_are_set_for_the_squad_or_one_member() {
        let model = team();
        assert_eq!(plan(&model, "/mode ask").commands, vec![Command::SetMode { squad: "acme".into(), member: None, mode: Some(Mode::Ask) }]);
        assert_eq!(plan(&model, "/mode frontend full").commands, vec![Command::SetMode { squad: "acme".into(), member: Some("frontend".into()), mode: Some(Mode::Full) }]);
        assert_eq!(plan(&model, "/mode frontend squad").commands, vec![Command::SetMode { squad: "acme".into(), member: Some("frontend".into()), mode: None }]);
        assert!(plan(&model, "/mode ghost ask").commands.is_empty());
        assert!(plan(&model, "/mode plan").flash.unwrap().contains("read-only"));
    }

    #[test]
    fn compact_clear_model_scale_close_and_desktop() {
        let model = team();
        assert_eq!(plan(&model, "/compact").commands, vec![Command::Refresh { session: 1, summary: true }]);
        assert_eq!(plan(&model, "/clear frontend").commands, vec![Command::Refresh { session: 2, summary: false }]);
        assert_eq!(plan(&model, "/model backend sonnet").commands, vec![Command::SetModel { session: 3, model: "sonnet".into() }]);
        assert_eq!(plan(&model, "/model haiku").commands, vec![Command::SetModel { session: 1, model: "haiku".into() }]);
        assert_eq!(plan(&model, "/scale off").commands, vec![Command::Scale { squad: "acme".into(), extra: 0 }]);
        assert!(plan(&model, "/scale 9").commands.is_empty());
        assert_eq!(plan(&model, "/close").commands, vec![Command::Close { session: 1 }]);
        assert_eq!(plan(&model, "/desktop stop").commands, vec![Command::Desktop { open: false }]);
        assert_eq!(plan(&model, "/browser hide backend").commands, vec![Command::ShowBrowser { squad: "acme".into(), member: Some("backend".into()), show: false }]);
    }

    #[test]
    fn tidy_asks_the_pm_and_a_command_not_here_says_so() {
        let model = team();
        let tidy = plan(&model, "/tidy");
        assert!(matches!(tidy.commands.as_slice(), [Command::Submit { session: 1, .. }]));
        let p = plan(&model, "/squad new x");
        assert!(p.commands.is_empty() && p.flash.unwrap().contains("터미널"));
        assert_eq!(plan(&model, "/board").effect, Some(Effect::ShowBoard));
        assert_eq!(plan(&model, "/find 로그인 화면").effect, Some(Effect::Find("로그인 화면".into())));
    }

    #[test]
    fn completion_draws_on_the_squad() {
        let model = team();
        let texts = |text: &str| choices(&model, text).into_iter().map(|c| c.text).collect::<Vec<_>>();
        assert_eq!(texts("@f"), ["@frontend "]);
        assert_eq!(texts("/proj"), ["/project "]);
        assert_eq!(texts("/project "), ["/project site", "/project app", "/project all"]);
        assert_eq!(texts("/model "), ["/model pm ", "/model frontend ", "/model backend ", "/model opus", "/model sonnet", "/model haiku"]);
        assert!(texts("hello").is_empty());
        assert!(texts("/mode\nx").is_empty(), "a multi-line draft is never a command");
    }

    #[test]
    fn up_and_down_walk_the_history_only_from_an_empty_box() {
        let mut h = History::default();
        let text = |s: Option<&Said>| s.map(|s| s.text.clone());
        assert_eq!(h.older(""), None, "nothing sent yet");
        h.push(Said::text("one"));
        h.push(Said::text("two"));
        h.push(Said::text("two"));
        assert_eq!(h.older("draft"), None, "a box with text is the person's");
        assert_eq!(text(h.older("")), Some("two".into()));
        assert_eq!(text(h.older("two")), Some("one".into()));
        assert_eq!(text(h.older("one")), Some("one".into()), "stops at the oldest");
        assert_eq!(h.newer().map(|s| s.text), Some("two".into()));
        assert_eq!(h.newer(), Some(Said::default()));
        assert_eq!(h.newer(), None);
    }

    fn saved(path: &str) -> Saved {
        use crate::attach::{Kind, Meta, Ready};
        Saved { kind: Kind::Pdf, name: "spec.pdf".into(), size: 10, meta: Meta { pages: Some(2), ..Meta::default() }, ready: Ready { path: path.into(), tag: format!("[pdf: {path} · 2 pages]") }, thumb: Some(vec![1, 2, 3]), origin: None }
    }

    #[test]
    fn up_brings_back_what_was_attached_too() {
        let mut h = History::default();
        h.push(Said { text: String::new(), attachments: vec![saved("/u/a.pdf")] });
        h.push(Said { text: "  ".into(), attachments: vec![] });
        let back = h.older("").unwrap();
        assert_eq!(back.attachments, vec![saved("/u/a.pdf")], "attachments alone were a message");
    }

    #[test]
    fn a_draft_is_kept_per_member_and_project_and_survives_a_restart() {
        let path = std::env::temp_dir().join(format!("equipe-drafts-{}/composer.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut d = Drafts::load(path.clone());
        let (all, site) = (Drafts::key("acme", "pm", None), Drafts::key("acme", "pm", Some("site")));
        d.put(all.clone(), Draft { text: "all".into(), caret: 1, attachments: vec![] });
        d.put(site.clone(), Draft { text: "site draft".into(), caret: 4, attachments: vec![saved("/u/b.pdf")] });
        assert_eq!(d.get(&all).text, "all");
        assert_eq!(d.get(&Drafts::key("acme", "frontend", None)), Draft::default());
        d.used(Path::new("/x/a.png"));
        d.used(Path::new("/x/b.png"));
        d.used(Path::new("/x/a.png"));
        d.save();
        let again = Drafts::load(path.clone());
        assert_eq!(again.get(&site), Draft { text: "site draft".into(), caret: 4, attachments: vec![saved("/u/b.pdf")] });
        assert_eq!(again.recent, vec![PathBuf::from("/x/a.png"), PathBuf::from("/x/b.png")]);
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        d.put(all.clone(), Draft::default());
        assert_eq!(d.get(&all), Draft::default(), "an empty draft is forgotten");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn at_finds_members_first_files_and_tickets_anywhere() {
        let mut model = team();
        model.tickets = vec![serde_json::from_value(serde_json::json!({"id": 12, "title": "헤더 정리", "body": "", "status": "todo", "owner": "pm", "assignee": null, "project": "/code/site", "depends_on": [], "branch": null, "created_by": "pm", "note": null})).unwrap()];
        let index = files::Index { project: "site".into(), files: vec!["src/main.rs".into(), "src/views/header.rs".into(), "README.md".into()] };
        let rows = choices_with(&model, "@hea", std::slice::from_ref(&index));
        let texts: Vec<&str> = rows.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["@src/views/header.rs "], "no member is called that; the file's name ranks");
        assert_eq!(choices_with(&model, "@T-12", &[]).iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["T-12 "]);
        assert_eq!(rows[0].group, Some("파일"));
        let mid = choices_with(&model, "고쳐 줘 @main", std::slice::from_ref(&index));
        assert_eq!(mid[0].text, "고쳐 줘 @src/main.rs ", "mid-sentence, the word is replaced");
        assert!(mid.iter().all(|c| c.group != Some("멤버")), "a member only heads a message");
        let members = choices_with(&model, "@", &[]);
        assert_eq!(members.iter().filter(|c| c.group == Some("멤버")).count(), 3);
    }

    #[test]
    fn a_file_mention_is_said_to_the_one_in_view() {
        let model = team();
        assert_eq!(plan(&model, "@src/main.rs 이거 봐 줘").commands, vec![submit(1, "@src/main.rs 이거 봐 줘")]);
    }

    #[test]
    fn enter_while_composing_only_commits_and_its_echo_is_dropped() {
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        let mut g = EnterGuard::default();
        // (a) "안녕" is being composed: Enter does not send.
        g.observe(true, t);
        assert!(!g.allows_send(ms(1)));
        // The composition ends with that key press; the Enter that follows in the
        // same breath is the same press.
        g.observe(false, ms(2));
        assert!(!g.allows_send(ms(3)), "the echo of the committing Enter");
        // (b) The next Enter, a person's moment later, sends.
        assert!(g.allows_send(ms(300)));
    }

    #[test]
    fn a_plain_enter_sends_and_a_composition_ended_long_ago_is_forgotten() {
        let t = Instant::now();
        let mut g = EnterGuard::default();
        assert!(g.allows_send(t), "nothing was ever composing");
        g.observe(true, t);
        g.observe(false, t + Duration::from_millis(5));
        assert!(g.allows_send(t + Duration::from_millis(400)), "the composition ended with another key");
        assert!(g.allows_send(t + Duration::from_millis(401)));
    }
}
