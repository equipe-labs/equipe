//! `FAKE_TEAM=1`: the PM proposes one more frontend — T-61 waits while
//! frontend is on T-47 — and the change waits for the person's card.
//! Approved, frontend-2 joins as the daemon would bring it in, and the PM's
//! conversation says so; declined, it says that. `FAKE_TEAM=remove` proposes
//! letting designer go instead (it is on T-60); `FAKE_TEAM=elsewhere`
//! approves it from "another screen" after `FAKE_TEAM_AFTER` ms (default 5000).

use std::time::Duration;

use equipe_core::protocol::team::{Change, TeamChange};
use equipe_core::protocol::{Command, Engine, Event, Member};
use tokio::sync::broadcast;

const ID: u64 = 7001;
const PM: u64 = 1;

fn mode() -> Option<String> {
    std::env::var("FAKE_TEAM").ok().filter(|v| !v.is_empty() && v != "0")
}

/// The changes still waiting.
pub fn changes() -> Vec<TeamChange> {
    waiting().lock().unwrap().clone()
}

fn waiting() -> &'static std::sync::Mutex<Vec<TeamChange>> {
    static WAITING: std::sync::OnceLock<std::sync::Mutex<Vec<TeamChange>>> = std::sync::OnceLock::new();
    WAITING.get_or_init(|| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let add = TeamChange {
            id: ID,
            change: Change::Add,
            role: "frontend".into(),
            count: 1,
            member: None,
            engine: None,
            model: None,
            effort: None,
            reason: "frontend가 T-47 갤러리에 몇 시간 더 걸려서, 앱 쪽 T-61이 계속 밀리고 있어요. 한 명 더 있으면 바로 시작할 수 있어요.".into(),
            unblocks: vec![61],
            by: "pm".into(),
            session: PM,
            proposed_at: now,
            busy_on: None,
        };
        let out = match mode().as_deref() {
            None => vec![],
            Some("remove") => vec![TeamChange {
                change: Change::Remove,
                role: "designer".into(),
                member: Some("designer".into()),
                reason: "온보딩 시안이 끝나면 디자인 일이 당분간 없어요.".into(),
                unblocks: vec![],
                busy_on: Some(60),
                ..add
            }],
            Some(_) => vec![add],
        };
        std::sync::Mutex::new(out)
    })
}

/// On a hello: `FAKE_TEAM=elsewhere` approves it from another screen in a while.
pub fn on_hello(events: &broadcast::Sender<Event>) {
    if mode().as_deref() != Some("elsewhere") {
        return;
    }
    let events = events.clone();
    tokio::spawn(async move {
        let after = std::env::var("FAKE_TEAM_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(5000);
        tokio::time::sleep(Duration::from_millis(after)).await;
        decide(ID, true, &events);
    });
}

/// The person's answer, as the daemon makes it. Whether it was one.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    match command {
        Command::ApproveTeamChange { id, .. } => decide(*id, true, events),
        Command::DeclineTeamChange { id, .. } => decide(*id, false, events),
        _ => return false,
    }
    true
}

fn decide(id: u64, approve: bool, events: &broadcast::Sender<Event>) {
    let change = {
        let mut all = waiting().lock().unwrap();
        let Some(at) = all.iter().position(|c| c.id == id) else { return };
        all.remove(at)
    };
    let squad = || "demo".to_owned();
    let mut members = Vec::new();
    let text = match (approve, change.change) {
        (true, Change::Add) => {
            let name = format!("{}-2", change.role);
            let _ = events.send(Event::SessionStarted {
                session: 50,
                project: "/tmp/fake/site".into(),
                engine: Engine::Claude,
                model: Some("claude-opus-5-5".into()),
                workspace: None,
                member: Some(Member { role: change.role.clone(), name: name.clone() }),
                squad: Some(squad()),
            });
            members.push(name.clone());
            format!("팀에 {name}을(를) 추가했어요")
        }
        (true, Change::Remove) => {
            let name = change.member.clone().unwrap_or_default();
            let _ = events.send(Event::SessionClosed { session: 4 });
            members.push(name.clone());
            format!("{name}을(를) 팀에서 내보냈어요")
        }
        (false, Change::Add) => format!("팀 변경 제안을 거절했어요 · {} {}명 추가", change.role, change.count),
        (false, Change::Remove) => format!("팀 변경 제안을 거절했어요 · {} 내보내기", change.member.clone().unwrap_or_default()),
    };
    let _ = events.send(Event::TeamChangeClosed { squad: squad(), id, approved: approve, members });
    let _ = events.send(Event::Notice { session: PM, text });
}
