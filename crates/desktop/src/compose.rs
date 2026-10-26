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
use equipe_core::protocol::{Access, Command, Engine, Mode, SessionId, SessionSpec, TicketId, TicketStatus, new_session_id};
use equipe_core::roles;
use serde::{Deserialize, Serialize};

use crate::attach::Saved;
use crate::commands::{self, Choice};
use crate::files;
use crate::sheet::{After, Ask, Then};
use crate::sidecar::Job;
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

    /// The line sent as `message` (its words and what it attached), the
    /// latest such: what a message taken back puts back in the box.
    pub fn sent_as(&self, message: &str) -> Option<&Said> {
        self.lines.iter().rev().find(|said| {
            let tags: String = said.attachments.iter().map(|a| format!("{} ", a.ready.tag)).collect();
            crate::attach::message(&said.text, &tags) == message.trim()
        })
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
        squads: other_squads(model),
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
        local_folders: !model.remote,
    };
    commands::complete(text, &ctx)
}

/// Every squad on the daemon's machine but this one, and what each works on.
fn other_squads(model: &Model) -> Vec<(String, String)> {
    model
        .squad_list
        .iter()
        .filter(|s| s.name != model.squad)
        .map(|s| {
            let projects: Vec<String> = s.projects.iter().map(|p| equipe_core::board::project_name(p)).collect();
            (s.name.clone(), if projects.is_empty() { String::new() } else { format!("works on {}", projects.join(", ")) })
        })
        .collect()
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
    /// 디자인 시스템 (`/design`).
    ShowDesign,
    /// The board, narrowed to these words (`/find`).
    Find(String),
    /// Put this project (or all) in focus.
    Focus(Option<String>),
    /// Show another squad.
    SwitchSquad(String),
    /// The usage sheet, the help sheet, the conversation as a file, leaving the app.
    Usage,
    Help,
    Export,
    Quit,
    /// The squad's folder / memory: opened on this Mac when the daemon is here,
    /// else shown with its path (a window on the daemon's machine is no use).
    OpenHome,
    OpenMemory,
    /// A side command on the daemon's machine, and what to do with its answer.
    Job(Job, After),
    /// A question first.
    Ask(Ask),
    /// `/suggest` was switched.
    Suggest(bool),
    /// The 디스크 panel (`/disk`).
    Disk,
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
///
/// Words that plainly accept or merge handed-in work ("전부 수락", "T-1
/// 수락") also raise the card to confirm it in that conversation, at once —
/// sent before the words, so the member there finds it up. Only a card: the
/// person's click on it is what accepts (`crate::accept`).
pub fn plan(model: &Model, text: &str, later: bool) -> Plan {
    let mut out = said(model, text, later);
    let to = out.commands.iter().find_map(|c| match c {
        Command::Submit { session, text, .. } | Command::SubmitLater { session, text, .. } => Some((*session, text.clone())),
        _ => None,
    });
    if let Some((session, said)) = to
        && let Some(raise) = crate::accept::raise(&model.squad, session, &said, &model.tickets)
    {
        out.commands.insert(0, raise);
    }
    out
}

fn said(model: &Model, text: &str, later: bool) -> Plan {
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
        // Both engines read a steer at their next step (Codex through its own
        // CLI's `turn/steer`); a daemon that can't steer one leaves a memo.
        Presence::Working if later || target.engine.is_none() => Plan {
            commands: vec![Command::SubmitLater { session: target.session, text: text.to_owned(), about: Some(model.focus.clone()) }],
            // In sight, its waiting bubble says so already; to someone else, a line.
            flash: away.then(|| t!("compose.memo_left", name = target.name)),
            ..Plan::default()
        },
        // Steering: read at its next step, the work carrying on.
        Presence::Working => Plan {
            commands: vec![Command::Submit { session: target.session, text: text.to_owned(), about: Some(model.focus.clone()) }],
            flash: away.then(|| t!("compose.steered", name = target.name)),
            ..Plan::default()
        },
        Presence::Ready if !away && model.opens_lane() => Plan {
            commands: vec![Command::Submit { session: target.session, text: text.to_owned(), about: Some(model.focus.clone()) }],
            flash: Some(t!("compose.answers_alongside", name = target.name)),
            ..Plan::default()
        },
        Presence::Ready => Plan {
            commands: vec![Command::Submit { session: target.session, text: text.to_owned(), about: Some(model.focus.clone()) }],
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
        "disk" => Plan { effect: Some(Effect::Disk), ..Plan::default() },
        "design" => Plan { effect: Some(Effect::ShowDesign), ..Plan::default() },
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
                about: None,
            }),
            None => Plan::say(t!("compose.no_pm")),
        },
        "mode" => set_mode(model, rest),
        "close" => match model.selected_member().filter(|m| m.presence != Presence::Closed) {
            Some(m) => Plan {
                effect: Some(Effect::Ask(Ask {
                    title: format!("{}을(를) 스쿼드에서 내보낼까요?", m.name),
                    body: m.ticket.map(|t| format!("맡고 있던 T-{t}는 보드로 돌아갑니다.")).into_iter().collect(),
                    yes: "내보내기".into(),
                    then: Then::Send(vec![Command::Close { session: m.session }]),
                })),
                ..Plan::default()
            },
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
        "compact" => match if rest.is_empty() { model.selected_member() } else { member_named(rest) } {
            Some(m) => Plan::send(Command::Refresh { session: m.session, summary: true }),
            None => Plan::say("누구요? /compact <멤버>"),
        },
        "clear" => match if rest.is_empty() { model.selected_member() } else { member_named(rest) } {
            Some(m) => Plan {
                effect: Some(Effect::Ask(Ask {
                    title: format!("{}의 다음 대화를 새로 시작할까요?", m.name),
                    body: vec!["지금까지의 대화는 이어지지 않습니다. 기억과 보드는 그대로입니다.".into()],
                    yes: "새로 시작".into(),
                    then: Then::Send(vec![Command::Refresh { session: m.session, summary: false }]),
                })),
                ..Plan::default()
            },
            None => Plan::say("누구요? /clear <멤버>"),
        },
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
        // `/engine codex` (the member in view), `/engine backend codex`,
        // `/engine all claude`: moved at each one's next ticket boundary.
        "engine" => {
            let words: Vec<&str> = rest.split_whitespace().collect();
            let engine = words.last().and_then(|w| match w.to_ascii_lowercase().as_str() {
                "claude" | "claude-code" | "claudecode" => Some(Engine::Claude),
                "codex" => Some(Engine::Codex),
                _ => None,
            });
            let Some(engine) = engine else { return Plan::say(t!("compose.engine_usage")) };
            let named = &words[..words.len() - 1];
            let members: Vec<String> = match named {
                [] => match model.selected_member() {
                    Some(m) => vec![m.name.clone()],
                    None => return Plan::say(t!("compose.engine_usage")),
                },
                ["all"] | ["모두"] | ["전부"] => Vec::new(),
                names => {
                    if let Some(missing) = names.iter().find(|n| member_named(n).is_none()) {
                        return Plan::say(t!("compose.model_no_member", name = missing));
                    }
                    names.iter().map(|n| (*n).to_owned()).collect()
                }
            };
            let who = if members.is_empty() { t!("compose.engine_everyone").to_owned() } else { members.join("·") };
            Plan {
                commands: vec![Command::MoveEngine { squad, members, engine }],
                flash: Some(t!("compose.engine_moving", names = who, engine = crate::switching::engine_name(engine))),
                ..Plan::default()
            }
        }
        "engines" => match rest {
            "" => Plan::say(t!("engines.now", mode = crate::switching::mode_word(model.engines.mode), about = crate::switching::mode_about(model.engines.mode))),
            "setup" => Plan::send(Command::ShowEngineSetup { squad }),
            word => match equipe_core::protocol::engines::Switching::parse(word) {
                Some(mode) => Plan {
                    commands: vec![Command::SetEngineSwitching { squad, mode }],
                    flash: Some(t!("engines.set", mode = crate::switching::mode_word(mode), about = crate::switching::mode_about(mode))),
                    ..Plan::default()
                },
                None => Plan::say(t!("engines.cmd_usage")),
            },
        },
        "desktop" => match rest {
            "stop" => Plan { commands: vec![Command::Desktop { open: false }], flash: Some(t!("compose.desktop_stopped").into()), ..Plan::default() },
            "resume" => Plan { commands: vec![Command::Desktop { open: true }], flash: Some(t!("compose.desktop_resumed").into()), ..Plan::default() },
            _ => Plan::say("/desktop stop · /desktop resume"),
        },
        "browser" => {
            let words: Vec<&str> = rest.split_whitespace().collect();
            match words.as_slice() {
                [] | ["signin"] => Plan {
                    flash: Some("로그인 창을 엽니다 — 창을 열어 둔 채로 사이트에 로그인하세요 (데몬이 있는 맥의 화면에 뜹니다)".into()),
                    effect: Some(Effect::Job(Job::BrowserSignin, After::Flash)),
                    ..Plan::default()
                },
                ["max"] => Plan { effect: Some(Effect::Job(Job::BrowserMax(None), After::Flash)), ..Plan::default() },
                ["max", n] => Plan { effect: Some(Effect::Job(Job::BrowserMax(Some((*n).to_owned())), After::Flash)), ..Plan::default() },
                [verb @ ("show" | "hide")] => Plan::send(Command::ShowBrowser { squad, member: None, show: *verb == "show" }),
                [verb @ ("show" | "hide"), who] => match member_named(who) {
                    Some(m) => Plan::send(Command::ShowBrowser { squad, member: Some(m.name.clone()), show: *verb == "show" }),
                    None => Plan::say(t!("compose.no_member", who = who)),
                },
                _ => Plan::say(t!("compose.browser_usage")),
            }
        }
        "send-apps" => Plan {
            effect: Some(Effect::Job(Job::SendApps { squad, words: rest.split_whitespace().map(str::to_owned).collect() }, After::Flash)),
            ..Plan::default()
        },
        "suggest" => {
            let on = match rest {
                "" => !model.suggestions,
                "on" => true,
                "off" => false,
                _ => return Plan::say("/suggest on · /suggest off"),
            };
            Plan {
                commands: vec![Command::Suggestions { squad, on }],
                flash: Some(if on {
                    "멤버가 답한 뒤 다음에 할 말을 입력창에 흐리게 추측해 둡니다 — → 로 받고, 그냥 쓰면 무시됩니다".into()
                } else {
                    "다음 말 추측을 끕니다 · /suggest on 으로 다시 켭니다".into()
                }),
                effect: Some(Effect::Suggest(on)),
                ..Plan::default()
            }
        }
        "usage" => Plan { effect: Some(Effect::Usage), ..Plan::default() },
        "help" => Plan { effect: Some(Effect::Help), ..Plan::default() },
        "export" => Plan { effect: Some(Effect::Export), ..Plan::default() },
        "quit" | "exit" => Plan { effect: Some(Effect::Quit), ..Plan::default() },
        "mouse" => Plan::say("/mouse는 터미널에서 마우스를 넘기는 명령이라 앱에는 없습니다 — 글은 그냥 드래그해서 ⌘C로 복사하세요"),
        "add" => add(model, rest),
        "home" => home(model, rest),
        "memory" => match rest {
            "" => Plan { effect: Some(Effect::OpenMemory), ..Plan::default() },
            "changes" | "recent" => Plan {
                effect: Some(Effect::Job(
                    Job::Memory { squad, words: vec!["changes".into()] },
                    After::Changes { title: "스쿼드 기억의 변경".into(), undo: false },
                )),
                ..Plan::default()
            },
            _ => Plan::say("/memory 는 스쿼드가 기억하는 것을 엽니다 · /memory changes"),
        },
        "squad" => squad_command(model, rest),
        other => Plan::say(format!("/{other}은(는) 없는 명령입니다 — /help에 목록이 있습니다")),
    }
}

/// `/home` opens the squad's folder; `/home changes` lists what changed there;
/// `/home undo [n]` takes a change back — after one question.
fn home(model: &Model, rest: &str) -> Plan {
    let squad = model.squad.clone();
    let words: Vec<&str> = rest.split_whitespace().collect();
    match words.as_slice() {
        [] => Plan { effect: Some(Effect::OpenHome), ..Plan::default() },
        ["changes"] | ["recent"] => Plan {
            effect: Some(Effect::Job(Job::Home { squad, words: vec!["changes".into()] }, After::Changes { title: "스쿼드 폴더의 변경".into(), undo: true })),
            ..Plan::default()
        },
        ["undo"] => undo(squad, 1),
        ["undo", n] => match n.trim_start_matches('#').parse::<usize>() {
            Ok(n) if n >= 1 => undo(squad, n),
            _ => Plan::say("어느 변경요? /home changes가 번호를 매깁니다: /home undo 2"),
        },
        _ => Plan::say("/home 은 스쿼드 폴더를 엽니다 · /home changes · /home undo [n]"),
    }
}

/// Take change `n` back (1 is the latest) — asked first, since it rewrites files.
pub fn undo(squad: String, n: usize) -> Plan {
    Plan {
        effect: Some(Effect::Ask(Ask {
            title: if n == 1 { "가장 최근 변경을 되돌릴까요?".into() } else { format!("{n}번 변경을 되돌릴까요?") },
            body: vec!["스쿼드 폴더의 파일이 그 변경 이전으로 돌아갑니다. 되돌림도 변경으로 남아서 /home undo 로 다시 되돌릴 수 있습니다.".into()],
            yes: "되돌리기".into(),
            then: Then::Job(Job::Home { squad, words: vec!["undo".into(), n.to_string()] }, After::Flash),
        })),
        ..Plan::default()
    }
}

/// `/add designer frontend`, `/add qa codex`, `/add custom writer [--edits]`.
fn add(model: &Model, rest: &str) -> Plan {
    let mut words: Vec<&str> = rest.split_whitespace().collect();
    let custom_edits = words.contains(&"--edits");
    words.retain(|w| *w != "--edits");
    let engine = match words.last() {
        Some(&"codex") => Some(Engine::Codex),
        Some(&"claude") => Some(Engine::Claude),
        _ => None,
    };
    if engine.is_some() {
        words.pop();
    }
    let names = || roles::ROLES.iter().map(|r| r.id).collect::<Vec<_>>().join(" ");
    let wanted: Vec<String> = match words.as_slice() {
        [] => return Plan::say(format!("누구요? /add designer frontend · 역할: {}, 또는 custom <이름>", names())),
        ["custom"] => return Plan::say("역할 이름을 적어 주세요: /add custom writer"),
        ["custom", name, ..] => vec![name.to_lowercase()],
        _ if custom_edits => return Plan::say("--edits는 custom 역할용입니다: /add custom writer --edits"),
        ids => {
            let mut wanted = Vec::new();
            for id in ids {
                match roles::role(id) {
                    Some(role) => wanted.push(role.id.to_owned()),
                    None => return Plan::say(format!("'{id}'라는 역할이 없습니다. 역할: {} — 또는 /add custom {id}", names())),
                }
            }
            wanted
        }
    };
    let commands = starts(model, &wanted, engine, custom_edits);
    if commands.is_empty() {
        return Plan::say("이 스쿼드에는 프로젝트가 없습니다 — /squad add <폴더>로 먼저 넣으세요");
    }
    Plan { commands, flash: Some(format!("{}이(가) 합류합니다 · @이름으로 바로 말할 수 있습니다", wanted.join("·"))), ..Plan::default() }
}

/// Where a new member works: the squad's first project, else its folder.
fn desk(model: &Model) -> Option<std::path::PathBuf> {
    model.projects.first().cloned().or_else(|| model.squad_list.iter().find(|s| s.name == model.squad).map(|s| s.home.clone()))
}

/// The commands that bring `roles` onto the squad. None when there is nowhere for them to work.
pub fn starts(model: &Model, roles: &[String], engine: Option<Engine>, custom_edits: bool) -> Vec<Command> {
    let Some(cwd) = desk(model) else { return Vec::new() };
    roles
        .iter()
        .map(|role| {
            let edits = equipe_core::roles::role_or_custom(role).edits || custom_edits;
            Command::Start {
                session: new_session_id(),
                spec: SessionSpec {
                    engine,
                    model: None,
                    access: if edits { Access::EditFiles } else { Access::ReadOnly },
                    cwd: cwd.clone(),
                    role: Some(role.clone()),
                    squad: Some(model.squad.clone()),
                },
            }
        })
        .collect()
}

/// A squad opened with nobody on it gets its PM first — the main contact —
/// then the team it was started with.
pub fn team(model: &Model, joining: Vec<String>) -> Vec<Command> {
    let mut roles = joining;
    roles.retain(|r| r != roles::PM);
    roles.insert(0, roles::PM.to_owned());
    starts(model, &roles, None, false)
}

/// `/squad` lists, `/squad <name>` switches, `/squad new <name> [folder]`
/// starts one, `/squad add <folder>` and `/squad remove <project>` change what
/// this one works on. Adding and starting are checked on the daemon's machine,
/// then asked about; removing is asked about.
fn squad_command(model: &Model, rest: &str) -> Plan {
    let words: Vec<&str> = rest.split_whitespace().collect();
    let squad = model.squad.clone();
    let job = |words: &[&str], dry_run: bool| Job::Squad { squad: squad.clone(), words: words.iter().map(|w| (*w).to_owned()).collect(), dry_run };
    match words.as_slice() {
        [] => {
            let others: Vec<String> = other_squads(model).into_iter().map(|(n, _)| n).collect();
            Plan::say(match others.as_slice() {
                [] => format!("{squad}뿐입니다 · /squad new <이름> [폴더]로 새 스쿼드를 시작합니다"),
                names => format!("지금 {squad} · 다른 스쿼드: {} · /squad <이름>으로 전환, /squad new로 새로 시작", names.join(", ")),
            })
        }
        ["add", folder] => Plan {
            effect: Some(Effect::Job(
                job(&["add", folder], true),
                After::Confirm {
                    title: format!("{squad}에 이 폴더를 추가할까요?"),
                    yes: "추가".into(),
                    then: Box::new((job(&["add", folder], false), After::Squads { open: None, say: format!("{squad}이(가) {folder}도 작업합니다 · 멤버는 다음 턴부터 봅니다"), })),
                },
            )),
            ..Plan::default()
        },
        ["remove", name] => {
            if !model.project_names().iter().any(|p| p == name) {
                return Plan::say(format!("{squad}은(는) {name}을(를) 작업하지 않습니다 · {}", model.project_names().join(", ")));
            }
            Plan {
                effect: Some(Effect::Ask(Ask {
                    title: format!("{name}을(를) {squad}에서 뺄까요?"),
                    body: vec!["티켓과 브랜치는 그대로 남습니다. 폴더는 지워지지 않습니다.".into()],
                    yes: "빼기".into(),
                    then: Then::Job(job(&["remove", name], false), After::Squads { open: None, say: format!("{squad}은(는) 이제 {name}을(를) 작업하지 않습니다 — 티켓과 브랜치는 남습니다") }),
                })),
                ..Plan::default()
            }
        }
        ["add"] | ["remove"] => Plan::say("/squad add <폴더> · /squad remove <프로젝트>"),
        ["new"] => Plan::say("이름, 그리고 작업할 폴더가 있으면: /squad new shop ~/Projects/shop"),
        ["new", name, rest @ ..] if rest.len() <= 1 => {
            let mut args = vec!["new", name];
            args.extend(rest.iter().copied());
            Plan {
                effect: Some(Effect::Job(
                    job(&args, true),
                    After::Confirm {
                        title: format!("스쿼드 {name}을(를) 시작할까요?"),
                        yes: "시작".into(),
                        then: Box::new((job(&args, false), After::Squads { open: Some((*name).to_owned()), say: format!("{name} 스쿼드를 시작했습니다: PM이 먼저 합류하고 팀이 따라옵니다 · PM에게 만들 것을 말하세요") })),
                    },
                )),
                ..Plan::default()
            }
        }
        [name] if *name == squad => Plan::say(format!("이미 {name}입니다")),
        [name] => {
            if model.squad_list.iter().any(|s| s.name == *name) {
                Plan { effect: Some(Effect::SwitchSquad((*name).to_owned())), ..Plan::default() }
            } else {
                Plan::say(format!("{name}라는 스쿼드가 없습니다 · /squad new {name}으로 시작합니다"))
            }
        }
        _ => Plan::say("/squad <이름>으로 전환 · /squad new <이름> [폴더] · /squad add <폴더> · /squad remove <프로젝트>"),
    }
}

/// `/accept T-3`, `/reopen T-3 what to change`, `/resume T-3 what I did`,
/// `/cancel T-3`, `/merge T-3 [push]`: the person's calls, straight to equipe —
/// never through a member.
fn decide(model: &Model, verb: &str, rest: &str) -> Plan {
    // `/accept T-1 T-3`: each, as the board's accept of each.
    let many: Vec<TicketId> = rest.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).map_while(parse_ticket_id).collect();
    if verb == "accept" && many.len() > 1 && many.len() == rest.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).count() {
        let squad = model.squad.clone();
        let tickets = many.iter().map(|id| format!("T-{id}")).collect::<Vec<_>>().join(", ");
        return Plan { commands: many.into_iter().map(|ticket| Command::Accept { squad: squad.clone(), ticket }).collect(), flash: Some(t!("compose.accepted_many", tickets = tickets)), ..Plan::default() };
    }
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
        _ => {
            let title = model.tickets.iter().find(|t| t.id == ticket).map(|t| t.title.clone()).unwrap_or_default();
            Plan {
                effect: Some(Effect::Ask(Ask {
                    title: format!("T-{ticket}을(를) 취소할까요?"),
                    body: vec![title, "맡은 멤버는 하던 일을 멈춥니다. 브랜치는 남습니다.".into()],
                    yes: "취소하기".into(),
                    then: Then::Send(vec![Command::Cancel { squad, ticket }]),
                })),
                ..Plan::default()
            }
        }
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
    use equipe_core::protocol::rpc::SquadEntry;
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
            steering_in: vec![],
            lane: None,
            estimate: None,
            compacting: None,
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

    fn team_for(model: &Model, joining: Vec<String>) -> Vec<Command> {
        super::team(model, joining)
    }

    fn submit(session: SessionId, text: &str) -> Command {
        Command::Submit { session, text: text.into(), about: Some(None) }
    }

    #[test]
    fn a_message_says_the_project_this_screen_shows() {
        let mut model = team();
        model.focus = Some("site".into());
        assert_eq!(plan(&model, "how is it going?").commands, vec![Command::Submit { session: 1, text: "how is it going?".into(), about: Some(Some("site".into())) }]);
    }

    #[test]
    fn a_message_taken_back_finds_what_it_attached() {
        let mut history = History::default();
        let shot = crate::attach::saved_of("[image: /up/1792073093606-paste.png]");
        assert_eq!((shot.len(), shot[0].kind, shot[0].name.as_str()), (1, crate::attach::Kind::Image, "paste.png"));
        history.push(Said { text: "look".into(), attachments: shot });
        history.push(Said::text("plain"));
        assert_eq!(history.sent_as("look [image: /up/1792073093606-paste.png]").map(|s| s.text.as_str()), Some("look"));
        assert_eq!(history.sent_as("plain").map(|s| s.attachments.len()), Some(0));
        assert!(history.sent_as("never sent").is_none());
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
        // The PM at work at its desk: steered — its bubble says "read at the
        // next step" and the box says Tab leaves a memo: no third line saying it.
        let p = plan(&model, "이거 해");
        assert_eq!(p.commands, vec![submit(1, "이거 해")]);
        assert_eq!(p.flash, None);
        // On a ticket of its own: free to talk, beside the work.
        model.members[0].ticket = Some(4);
        let p = plan(&model, "앱은?");
        assert_eq!(p.commands, vec![submit(1, "앱은?")]);
        assert!(p.flash.unwrap().contains("나란히"));
        // A member on a ticket: what is said steers it.
        model.members[1].ticket = Some(5);
        model.select(2);
        let p = plan(&model, "푸터는 빼");
        assert_eq!(p.commands, vec![submit(2, "푸터는 빼")]);
        assert_eq!(p.flash, None);
        // A memo to the one in view: its bubble says so too.
        assert_eq!(super::plan(&model, "끝나면 이것도", true).flash, None);
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
        assert_eq!(p.commands, vec![Command::SubmitLater { session: 3, text: "이것도".into(), about: Some(None) }]);
        assert!(p.flash.unwrap().contains("메모를 남겼"));
        // A member at rest is simply told: there is nothing to wait for.
        assert_eq!(super::plan(&model, "@frontend 안녕", true).commands, vec![submit(2, "안녕")]);
    }

    #[test]
    fn a_codex_member_at_work_is_steered_like_a_claude_one() {
        let mut codex = session(3, "backend", SessionState::Working);
        codex.engine = Engine::Codex;
        let model = squad(vec![session(1, "pm", SessionState::Ready), codex]);
        let p = plan(&model, "@backend 이것도");
        assert_eq!(p.commands, vec![Command::Submit { session: 3, text: "이것도".into(), about: Some(None) }]);
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
        // Dropping a ticket is asked about once.
        let cancel = plan(&model, "/cancel 3");
        assert!(cancel.commands.is_empty());
        assert_eq!(asked(&cancel).then, Then::Send(vec![Command::Cancel { squad: "acme".into(), ticket: 3 }]));
        assert_eq!(plan(&model, "/reopen T-3 헤더가 깨짐").commands, vec![Command::Reopen { squad: "acme".into(), ticket: 3, note: "헤더가 깨짐".into() }]);
        assert_eq!(plan(&model, "/resume T-4 토큰 넣음").commands, vec![Command::Resume { squad: "acme".into(), ticket: 4, note: "토큰 넣음".into() }]);
        assert_eq!(plan(&model, "/merge T-3 push").commands, vec![Command::Merge { squad: "acme".into(), ticket: 3, push: true }]);
        assert_eq!(plan(&model, "/merge T-3").commands, vec![Command::Merge { squad: "acme".into(), ticket: 3, push: false }]);
        // Sent back with no word: the number stays in the box for the person to finish.
        let bare = plan(&model, "/reopen T-3");
        assert!(bare.commands.is_empty());
        assert_eq!(bare.restore.as_deref(), Some("/reopen T-3 "));
        assert!(plan(&model, "/accept").commands.is_empty());
        // Several at once: each accepted, as from the board.
        let accept = |ticket| Command::Accept { squad: "acme".into(), ticket };
        assert_eq!(plan(&model, "/accept T-1 T-3").commands, vec![accept(1), accept(3)]);
        assert_eq!(plan(&model, "/accept T-1, 3").commands, vec![accept(1), accept(3)]);
    }

    #[test]
    fn plain_words_of_acceptance_raise_the_card_before_the_member_hears_them() {
        use equipe_core::protocol::accept::Verdict;
        let mut model = team();
        let review: equipe_core::protocol::Ticket = serde_json::from_value(serde_json::json!({ "id": 1, "title": "로그인", "body": "", "status": "review", "kind": "code", "branch": "equipe/t-1", "depends_on": [], "created_by": "pm" })).unwrap();
        model.apply(Update::Event(Event::TicketChanged { squad: "acme".into(), ticket: review }), T0);
        model.selected = Some(1);
        let p = plan(&model, "전부다 수락할게.");
        assert_eq!(
            p.commands,
            vec![
                Command::ProposeAccept { squad: "acme".into(), session: 1, verdict: Verdict::Accept, tickets: vec![1], note: String::new() },
                Command::Submit { session: 1, text: "전부다 수락할게.".into(), about: Some(None) },
            ],
            "the card first, then the words, to the PM as ever"
        );
        // Said to another member, the card is in that conversation.
        assert!(matches!(plan(&model, "@frontend T-1 수락").commands.first(), Some(Command::ProposeAccept { session: 2, .. })));
        // Anything less plain is only words.
        assert_eq!(plan(&model, "전부 수락할까?").commands.len(), 1);
        assert_eq!(plan(&model, "T-1 수락하지 마").commands.len(), 1);
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

    fn asked(plan: &Plan) -> &Ask {
        match &plan.effect {
            Some(Effect::Ask(ask)) => ask,
            other => panic!("no question asked: {other:?}"),
        }
    }

    fn job(plan: &Plan) -> (&Job, &After) {
        match &plan.effect {
            Some(Effect::Job(job, after)) => (job, after),
            other => panic!("no side command: {other:?}"),
        }
    }

    #[test]
    fn compact_model_and_desktop_go_to_the_daemon() {
        let model = team();
        assert_eq!(plan(&model, "/compact").commands, vec![Command::Refresh { session: 1, summary: true }]);
        assert_eq!(plan(&model, "/model backend sonnet").commands, vec![Command::SetModel { session: 3, model: "sonnet".into() }]);
        assert_eq!(plan(&model, "/model haiku").commands, vec![Command::SetModel { session: 1, model: "haiku".into() }]);
        assert_eq!(plan(&model, "/desktop stop").commands, vec![Command::Desktop { open: false }]);
        assert_eq!(plan(&model, "/browser hide backend").commands, vec![Command::ShowBrowser { squad: "acme".into(), member: Some("backend".into()), show: false }]);
        assert_eq!(plan(&model, "/scale 3").commands, vec![Command::Scale { squad: "acme".into(), extra: 3 }]);
    }

    #[test]
    fn what_is_hard_to_take_back_is_asked_about_once() {
        let model = team();
        let clear = plan(&model, "/clear frontend");
        assert!(clear.commands.is_empty(), "nothing is sent before the yes");
        assert_eq!(asked(&clear).then, Then::Send(vec![Command::Refresh { session: 2, summary: false }]));
        assert!(asked(&clear).title.contains("frontend"));
        let close = plan(&model, "/close");
        assert_eq!(asked(&close).then, Then::Send(vec![Command::Close { session: 1 }]));
        assert!(asked(&close).title.starts_with("pm"), "the member is named in the question");
        // Undo asks too, and says what it will do.
        let undo = plan(&model, "/home undo 2");
        assert_eq!(asked(&undo).then, Then::Job(Job::Home { squad: "acme".into(), words: vec!["undo".into(), "2".into()] }, After::Flash));
        assert_eq!(asked(&plan(&model, "/home undo")).then, Then::Job(Job::Home { squad: "acme".into(), words: vec!["undo".into(), "1".into()] }, After::Flash));
        assert!(plan(&model, "/home undo x").flash.unwrap().contains("/home changes"));
    }

    #[test]
    fn tidy_asks_the_pm_and_the_screens_have_no_daemon_command() {
        let model = team();
        let tidy = plan(&model, "/tidy");
        assert!(matches!(tidy.commands.as_slice(), [Command::Submit { session: 1, .. }]));
        assert_eq!(plan(&model, "/board").effect, Some(Effect::ShowBoard));
        assert_eq!(plan(&model, "/design").effect, Some(Effect::ShowDesign));
        assert_eq!(plan(&model, "/find 로그인 화면").effect, Some(Effect::Find("로그인 화면".into())));
        for (line, effect) in [("/usage", Effect::Usage), ("/help", Effect::Help), ("/export", Effect::Export), ("/quit", Effect::Quit), ("/home", Effect::OpenHome), ("/memory", Effect::OpenMemory)] {
            let p = plan(&model, line);
            assert_eq!((p.commands.is_empty(), p.effect), (true, Some(effect)), "{line}");
        }
        assert!(plan(&model, "/mouse").flash.unwrap().contains("터미널"));
        assert!(plan(&model, "/nope").flash.unwrap().contains("/help"));
    }

    #[test]
    fn the_folders_and_memory_are_read_on_the_daemons_machine() {
        let model = team();
        let p = plan(&model, "/home changes");
        let (j, after) = job(&p);
        assert_eq!(j, &Job::Home { squad: "acme".into(), words: vec!["changes".into()] });
        assert!(matches!(after, After::Changes { undo: true, .. }), "each change has its way back");
        let p = plan(&model, "/memory changes");
        let (j, after) = job(&p);
        assert_eq!(j, &Job::Memory { squad: "acme".into(), words: vec!["changes".into()] });
        assert!(matches!(after, After::Changes { undo: false, .. }), "memory's list numbers its own changes");
        let p = plan(&model, "/send-apps add Slack");
        let (j, _) = job(&p);
        assert_eq!(j, &Job::SendApps { squad: "acme".into(), words: vec!["add".into(), "Slack".into()] });
        assert_eq!(job(&plan(&model, "/browser max 4")).0, &Job::BrowserMax(Some("4".into())));
        assert_eq!(job(&plan(&model, "/browser max")).0, &Job::BrowserMax(None));
        assert_eq!(job(&plan(&model, "/browser")).0, &Job::BrowserSignin);
        assert_eq!(job(&plan(&model, "/browser signin")).0, &Job::BrowserSignin);
    }

    #[test]
    fn squads_are_switched_started_and_changed_with_a_question_where_it_matters() {
        let mut model = team();
        model.squad_list = vec![
            SquadEntry { name: "acme".into(), projects: vec!["/code/site".into()], home: "/h/acme".into() },
            SquadEntry { name: "ios".into(), projects: vec!["/code/app".into()], home: "/h/ios".into() },
        ];
        assert_eq!(plan(&model, "/squad ios").effect, Some(Effect::SwitchSquad("ios".into())));
        assert!(plan(&model, "/squad acme").flash.unwrap().contains("이미"));
        assert!(plan(&model, "/squad nope").flash.unwrap().contains("/squad new nope"));
        assert!(plan(&model, "/squad").flash.unwrap().contains("ios"));
        // Add: checked on the daemon's machine, then asked about, then done.
        let add = plan(&model, "/squad add ~/Projects/shop");
        let (check, after) = job(&add);
        assert_eq!(check, &Job::Squad { squad: "acme".into(), words: vec!["add".into(), "~/Projects/shop".into()], dry_run: true });
        let After::Confirm { then, .. } = after else { panic!("{after:?}") };
        assert_eq!(then.0, Job::Squad { squad: "acme".into(), words: vec!["add".into(), "~/Projects/shop".into()], dry_run: false });
        // New: the same, and the squad it makes is opened afterwards.
        let new = plan(&model, "/squad new shop ~/Projects/shop");
        let After::Confirm { then, .. } = job(&new).1 else { panic!() };
        assert!(matches!(&then.1, After::Squads { open: Some(name), .. } if name == "shop"));
        // Remove: asked, and only a project the squad works on.
        let remove = plan(&model, "/squad remove site");
        assert_eq!(asked(&remove).then, Then::Job(Job::Squad { squad: "acme".into(), words: vec!["remove".into(), "site".into()], dry_run: false }, After::Squads { open: None, say: "acme은(는) 이제 site을(를) 작업하지 않습니다 — 티켓과 브랜치는 남습니다".into() }));
        assert!(plan(&model, "/squad remove ghost").effect.is_none());
    }

    #[test]
    fn add_brings_in_roles_with_their_access_and_needs_somewhere_to_work() {
        let model = team();
        let started = |p: &Plan| -> Vec<(Option<String>, Access, Option<Engine>)> {
            p.commands.iter().map(|c| match c {
                Command::Start { spec, .. } => (spec.role.clone(), spec.access, spec.engine),
                other => panic!("{other:?}"),
            }).collect()
        };
        let access = |role: &str| if roles::role(role).unwrap().edits { Access::EditFiles } else { Access::ReadOnly };
        assert_eq!(
            started(&plan(&model, "/add designer qa codex")),
            vec![(Some("designer".into()), access("designer"), Some(Engine::Codex)), (Some("qa".into()), access("qa"), Some(Engine::Codex))]
        );
        assert_eq!(started(&plan(&model, "/add custom writer")), vec![(Some("writer".into()), Access::ReadOnly, None)]);
        assert_eq!(started(&plan(&model, "/add custom writer --edits")), vec![(Some("writer".into()), Access::EditFiles, None)]);
        assert!(plan(&model, "/add --edits").commands.is_empty());
        assert!(plan(&model, "/add wizard").flash.unwrap().contains("wizard"));
        let nowhere = Model::new("empty");
        assert!(plan(&nowhere, "/add qa").flash.unwrap().contains("프로젝트가 없습니다"));
    }

    #[test]
    fn engines_sets_how_members_move_between_engines_and_shows_the_setup_again() {
        use equipe_core::protocol::engines::Switching;
        let model = team();
        let auto = plan(&model, "/engines auto");
        assert_eq!(auto.commands, vec![Command::SetEngineSwitching { squad: "acme".into(), mode: Switching::Auto }]);
        assert!(auto.flash.unwrap().contains("자동"));
        assert_eq!(plan(&model, "/engines off").commands, vec![Command::SetEngineSwitching { squad: "acme".into(), mode: Switching::Off }]);
        assert_eq!(plan(&model, "/engines setup").commands, vec![Command::ShowEngineSetup { squad: "acme".into() }]);
        assert!(plan(&model, "/engines").flash.unwrap().contains("제안"), "suggest unless the person chose");
        assert!(plan(&model, "/engines sometimes").commands.is_empty());
    }

    #[test]
    fn suggest_toggles_and_a_new_squad_opens_with_its_pm_first() {
        let mut model = team();
        assert_eq!(plan(&model, "/suggest off").commands, vec![Command::Suggestions { squad: "acme".into(), on: false }]);
        assert_eq!(plan(&model, "/suggest").effect, Some(Effect::Suggest(false)), "on now, so a bare /suggest turns it off");
        model.suggestions = false;
        assert_eq!(plan(&model, "/suggest").effect, Some(Effect::Suggest(true)));
        let roles_of = |commands: Vec<Command>| -> Vec<String> {
            commands.into_iter().map(|c| match c { Command::Start { spec, .. } => spec.role.unwrap(), other => panic!("{other:?}") }).collect()
        };
        assert_eq!(roles_of(team_for(&model, vec!["designer".into(), "frontend".into()])), ["pm", "designer", "frontend"]);
        assert_eq!(roles_of(team_for(&model, vec!["pm".into(), "qa".into()])), ["pm", "qa"], "the PM once, first");
        assert_eq!(roles_of(team_for(&model, vec![])), ["pm"]);
        assert!(team_for(&Model::new("x"), vec![]).is_empty(), "no folder to work in");
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
    fn over_ssh_folders_are_not_listed_from_this_disk_and_squads_come_from_the_daemon() {
        let mut model = team();
        model.squad_list = vec![SquadEntry { name: "ios".into(), projects: vec!["/code/app".into()], home: "/h".into() }, SquadEntry { name: "acme".into(), projects: vec![], home: "/h".into() }];
        let texts = |model: &Model, text: &str| choices(model, text).into_iter().map(|c| c.text).collect::<Vec<_>>();
        assert_eq!(texts(&model, "/squad i"), ["/squad ios"], "this squad isn't offered as a switch");
        assert!(!texts(&model, "/squad add /").is_empty(), "a Mac's own folders complete");
        model.remote = true;
        assert!(texts(&model, "/squad add /").is_empty(), "another machine's can't be listed here");
        assert_eq!(texts(&model, "/squad rem"), ["/squad remove "]);
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
