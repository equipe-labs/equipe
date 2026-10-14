//! What typing into the conversation does, without a window: the history that
//! ↑↓ walks, a draft kept per member and project, the guard that keeps a Korean
//! composition's Enter from sending, completion from the squad's state, and what
//! a sent line turns into — a message to a member, or one of the person's
//! commands. The same rules as the terminal console (`tui/app.rs` `submit`,
//! `slash`); the daemon commands are the ones it sends.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use equipe_core::board::parse_ticket_id;
use equipe_core::protocol::{Command, Engine, Mode, SessionId, TicketId, TicketStatus};

use crate::commands::{self, Choice};
use crate::state::{Model, Presence};

// ── history ──

/// The lines the person sent, for ↑ to bring back (only from an empty box).
#[derive(Debug, Default)]
pub struct History {
    lines: Vec<String>,
    /// Where ↑↓ stands while walking, as an index into `lines`.
    at: Option<usize>,
}

impl History {
    /// A line was sent: remember it (not twice in a row), stop walking.
    pub fn push(&mut self, line: &str) {
        let line = line.trim();
        if !line.is_empty() && self.lines.last().map(String::as_str) != Some(line) {
            self.lines.push(line.to_owned());
        }
        self.at = None;
    }

    /// ↑. `current` is what the box holds: walking starts only from empty, and
    /// goes on while the box still shows a recalled line.
    pub fn older(&mut self, current: &str) -> Option<&str> {
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

    /// ↓: toward the newest line, then an empty box (`Some("")`).
    pub fn newer(&mut self) -> Option<&str> {
        let i = self.at?;
        if i + 1 < self.lines.len() {
            self.at = Some(i + 1);
            Some(&self.lines[i + 1])
        } else {
            self.at = None;
            Some("")
        }
    }

    /// The person typed: whatever is in the box is theirs now.
    pub fn stop(&mut self) {
        self.at = None;
    }
}

// ── drafts ──

/// What was half written to each member, per project in focus.
#[derive(Debug, Default)]
pub struct Drafts(HashMap<(SessionId, Option<String>), String>);

impl Drafts {
    pub fn put(&mut self, member: SessionId, project: Option<String>, text: String) {
        if text.is_empty() {
            self.0.remove(&(member, project));
        } else {
            self.0.insert((member, project), text);
        }
    }

    pub fn get(&self, member: SessionId, project: &Option<String>) -> String {
        self.0.get(&(member, project.clone())).cloned().unwrap_or_default()
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

/// What completion offers for `text`, drawn from the squad as it stands.
pub fn choices(model: &Model, text: &str) -> Vec<Choice> {
    if text.is_empty() || text.contains('\n') {
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
                let about = if model.focus.as_deref() == Some(name.as_str()) { "in focus" } else { "" };
                (name, about.to_owned())
            })
            .collect(),
        member_modes: Vec::new(),
        member_models: model.members.iter().map(|m| (m.name.as_str(), engine_models(m.engine, m.model.as_deref()))).collect(),
        selected_models,
    };
    commands::complete(text, &ctx)
}

fn engine_models(engine: Option<Engine>, current: Option<&str>) -> Vec<String> {
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
pub fn plan(model: &Model, text: &str) -> Plan {
    let text = text.trim();
    if text.is_empty() {
        return Plan::default();
    }
    if let Some(command) = text.strip_prefix('/') {
        return slash(model, command);
    }
    let Some(selected) = model.selected_member() else {
        return Plan { restore: Some(text.to_owned()), ..Plan::say("대화할 멤버가 아직 없습니다") };
    };
    let (target, text) = match text.strip_prefix('@') {
        // `@frontend fix the header` goes to frontend without leaving this view.
        Some(mention) => {
            let (name, rest) = mention.split_once(char::is_whitespace).unwrap_or((mention, ""));
            match model.open_member(name) {
                Some(member) if rest.trim().is_empty() => return Plan { effect: Some(Effect::Select(member.session)), ..Plan::default() },
                Some(member) => (member, rest.trim()),
                None => return Plan { restore: Some(text.to_owned()), ..Plan::say(format!("스쿼드에 @{name}라는 멤버가 없습니다")) },
            }
        }
        None => (selected, text),
    };
    let away = target.session != selected.session;
    match target.presence {
        Presence::Closed => Plan { restore: Some(text.to_owned()), ..Plan::say(format!("{}은(는) 더 이상 스쿼드에 없습니다", target.name)) },
        Presence::Working => Plan {
            commands: vec![Command::Submit { session: target.session, text: text.to_owned() }],
            // Left as a memo: the core holds it until the turn ends.
            flash: Some(format!("{}에게 메모를 남겼습니다 — 이번 턴이 끝나면 읽습니다. Esc로 멈추면 바로 읽습니다.", target.name)),
            ..Plan::default()
        },
        Presence::Ready => Plan {
            commands: vec![Command::Submit { session: target.session, text: text.to_owned() }],
            flash: away.then(|| format!("{}에게 보냈습니다", target.name)),
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
                        return Plan::say(format!("{}에 {name}라는 프로젝트가 없습니다 · {}", model.squad, model.project_names().join(", ")));
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
            None => Plan::say("물어볼 PM이 스쿼드에 없습니다"),
        },
        "mode" => set_mode(model, rest),
        "close" => match model.selected_member().filter(|m| m.presence != Presence::Closed) {
            Some(m) => Plan::send(Command::Close { session: m.session }),
            None => Plan::default(),
        },
        "scale" => match rest {
            "" => Plan::say("/scale <0-6> · /scale off — 한 역할의 일이 밀릴 때 equipe가 추가로 들이는 멤버 수"),
            word => match if word == "off" { Some(0) } else { word.parse::<usize>().ok().filter(|n| *n <= 6) } {
                Some(extra) => Plan {
                    commands: vec![Command::Scale { squad, extra }],
                    flash: Some(if extra == 0 { "추가 멤버 없음: 각 역할의 일은 그 역할 멤버를 기다립니다".into() } else { format!("일이 밀리면 한 역할에 최대 {extra}명 더") }),
                    ..Plan::default()
                },
                None => Plan::say("0에서 6 사이 숫자: /scale 3 · /scale off"),
            },
        },
        "compact" | "clear" => {
            let target = match rest {
                "" => model.selected_member(),
                n => member_named(n),
            };
            match target {
                Some(m) => Plan::send(Command::Refresh { session: m.session, summary: name == "compact" }),
                None => Plan::say(format!("누구요? /{name} <멤버>")),
            }
        }
        "model" => {
            let words: Vec<&str> = rest.split_whitespace().collect();
            let (target, wanted) = match words.as_slice() {
                [n, m] => (member_named(n), *m),
                [m] => (model.selected_member(), *m),
                _ => return Plan::say("/model [멤버] <모델> — 예: /model mobile opus"),
            };
            match target {
                Some(t) => Plan {
                    commands: vec![Command::SetModel { session: t.session, model: wanted.to_owned() }],
                    flash: Some(format!("{}은(는) 다음 턴부터 {wanted}로 일합니다", t.name)),
                    ..Plan::default()
                },
                None => Plan::say(format!("{}라는 멤버가 없습니다 · /model [멤버] <모델>", words[0])),
            }
        }
        "desktop" => match rest {
            "stop" => Plan { commands: vec![Command::Desktop { open: false }], flash: Some("화면을 회수했습니다: /desktop resume 전까지 멤버는 클릭도 입력도 못 합니다".into()), ..Plan::default() },
            "resume" => Plan { commands: vec![Command::Desktop { open: true }], flash: Some("멤버가 화면을 다시 쓸 수 있습니다".into()), ..Plan::default() },
            _ => Plan::say("/desktop stop · /desktop resume"),
        },
        "browser" => {
            let words: Vec<&str> = rest.split_whitespace().collect();
            match words.as_slice() {
                [verb @ ("show" | "hide")] => Plan::send(Command::ShowBrowser { squad, member: None, show: *verb == "show" }),
                [verb @ ("show" | "hide"), who] => match member_named(who) {
                    Some(m) => Plan::send(Command::ShowBrowser { squad, member: Some(m.name.clone()), show: *verb == "show" }),
                    None => Plan::say(format!("{who}라는 멤버가 없습니다")),
                },
                _ => Plan::say("/browser show|hide [멤버] — 로그인과 개수 제한은 터미널의 /browser에서"),
            }
        }
        "help" => Plan::say("입력창에서 / 를 누르면 명령 목록, @ 를 누르면 멤버 목록이 나옵니다"),
        other => Plan::say(format!("/{other}은(는) 아직 데스크톱에 없습니다 — 터미널의 equipe에서 쓰세요")),
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
            "resume" => (TicketStatus::Waiting, "사람을 기다리는 일"),
            _ => (TicketStatus::Review, "검토를 기다리는 일"),
        };
        let waiting: Vec<String> = model.tickets.iter().filter(|t| t.status == status).map(|t| format!("T-{}", t.id)).collect();
        return Plan::say(match waiting.as_slice() {
            [] => format!("어느 티켓요? /{verb} T-3"),
            ids => format!("어느 티켓요? {what}: {}", ids.join(", ")),
        });
    };
    let squad = model.squad.clone();
    match verb {
        "accept" => {
            let merges = model.tickets.iter().find(|t| t.id == ticket).is_some_and(|t| !t.kind.in_home() && t.branch.is_some());
            Plan {
                commands: vec![Command::Accept { squad, ticket }],
                flash: Some(if merges { format!("T-{ticket} 수락 · main에 합치려면 /merge T-{ticket}") } else { format!("T-{ticket} 수락 · 되돌리려면 /reopen T-{ticket} <바꿀 것>") }),
                ..Plan::default()
            }
        }
        "merge" => {
            let push = note == "push";
            Plan {
                commands: vec![Command::Merge { squad, ticket, push }],
                flash: Some(format!("T-{ticket}를 main에 합칩니다{}…", if push { " (origin에 푸시까지)" } else { " (푸시는 /merge T-n push)" })),
                ..Plan::default()
            }
        }
        // Sent back with no word, the member can only guess what to change.
        "reopen" if note.is_empty() => Plan {
            restore: Some(format!("/reopen T-{ticket} ")),
            ..Plan::say(format!("T-{ticket}에서 무엇을 바꿔야 하나요? /reopen T-{ticket} 뒤에 적어 주세요 — 멤버는 그 글로 일합니다"))
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
        let modes: Vec<String> = Mode::ALL.iter().map(|m| format!("{} ({})", m.label(), m.short())).collect();
        format!("모드: {} · /mode <모드> 는 스쿼드 전체, /mode <멤버> <모드> 는 한 명", modes.join(" · "))
    };
    let squad = model.squad.clone();
    if words.contains(&"plan") {
        return Plan::say("읽기만 하는 모드의 이름은 read-only입니다: /mode read-only");
    }
    match words.as_slice() {
        [] => Plan::say(format!("스쿼드는 {} 모드입니다. {}", model.mode.label(), usage())),
        [mode] => match Mode::parse(mode) {
            Some(mode) => Plan::send(Command::SetMode { squad, member: None, mode: Some(mode) }),
            None => Plan::say(usage()),
        },
        [name, mode, ..] => {
            if model.open_member(name).is_none() {
                return Plan::say(format!("스쿼드에 {name}라는 멤버가 없습니다"));
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
    fn a_message_to_a_working_member_is_left_as_a_memo() {
        let model = team();
        let p = plan(&model, "@backend 이것도");
        assert_eq!(p.commands, vec![submit(3, "이것도")]);
        assert!(p.flash.unwrap().contains("메모"));
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
        assert_eq!(h.older(""), None, "nothing sent yet");
        h.push("one");
        h.push("two");
        h.push("two");
        assert_eq!(h.older("draft"), None, "a box with text is the person's");
        assert_eq!(h.older(""), Some("two"));
        assert_eq!(h.older("two"), Some("one"));
        assert_eq!(h.older("one"), Some("one"), "stops at the oldest");
        assert_eq!(h.newer(), Some("two"));
        assert_eq!(h.newer(), Some(""));
        assert_eq!(h.newer(), None);
            }

    #[test]
    fn a_draft_is_kept_per_member_and_project() {
        let mut d = Drafts::default();
        d.put(1, None, "all".into());
        d.put(1, Some("site".into()), "site draft".into());
        assert_eq!(d.get(1, &None), "all");
        assert_eq!(d.get(1, &Some("site".into())), "site draft");
        assert_eq!(d.get(2, &None), "");
        d.put(1, None, String::new());
        assert_eq!(d.get(1, &None), "");
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
