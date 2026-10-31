//! `FAKE_ROUTINE=1`: the PM proposes a weekly report — every Monday at 09:00
//! — and the proposal waits for the person's card; `FAKE_ROUTINE=daily`
//! proposes a daily check at 18:30 too; `FAKE_ROUTINE=long`: one with a long
//! title and prompt. Turned on, it joins the squad's
//! routines as the daemon would save it, and the PM's conversation says so;
//! declined, it says that. `SetIdleWake` is answered as the daemon does.

use equipe_core::protocol::routines::{Routine, RoutineProposal, Schedule};
use equipe_core::protocol::{Command, Event};
use tokio::sync::broadcast;

const PM: u64 = 1;

fn mode() -> Option<String> {
    std::env::var("FAKE_ROUTINE").ok().filter(|v| !v.is_empty() && v != "0")
}

/// The proposals still waiting.
pub fn proposals() -> Vec<RoutineProposal> {
    waiting().lock().unwrap().clone()
}

/// The routines saved so far.
pub fn routines() -> Vec<Routine> {
    saved().lock().unwrap().clone()
}

/// Hours an empty board waits before the PM is woken.
pub fn idle_wake_hours() -> u32 {
    *idle().lock().unwrap()
}

fn idle() -> &'static std::sync::Mutex<u32> {
    static IDLE: std::sync::OnceLock<std::sync::Mutex<u32>> = std::sync::OnceLock::new();
    IDLE.get_or_init(|| std::sync::Mutex::new(equipe_core::protocol::routines::IDLE_WAKE_HOURS))
}

fn saved() -> &'static std::sync::Mutex<Vec<Routine>> {
    static SAVED: std::sync::OnceLock<std::sync::Mutex<Vec<Routine>>> = std::sync::OnceLock::new();
    SAVED.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

fn waiting() -> &'static std::sync::Mutex<Vec<RoutineProposal>> {
    static WAITING: std::sync::OnceLock<std::sync::Mutex<Vec<RoutineProposal>>> = std::sync::OnceLock::new();
    WAITING.get_or_init(|| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let weekly = RoutineProposal {
            id: 8001,
            routine: Routine {
                id: 0,
                title: "주간 진행 보고".into(),
                prompt: "지난 한 주 동안 끝난 티켓과 아직 진행 중인 티켓을 프로젝트별로 정리하고, 막혀 있는 것과 다음 주에 먼저 할 일을 세 가지만 골라서 알려 주세요. 사람에게 결정을 받아야 하는 게 있으면 맨 위에 적어 주세요.".into(),
                when: Schedule::Weekly { weekday: 1, hour: 9, minute: 0 },
                on: true,
                last_run: None,
                next_run: None,
            },
            reason: "월요일마다 지난주 정리를 물어보셔서, 정해진 시간에 먼저 정리해 드리면 좋을 것 같아요.".into(),
            by: "pm".into(),
            session: PM,
            proposed_at: now,
        };
        let out = match mode().as_deref() {
            None => vec![],
            Some("daily") => vec![
                weekly.clone(),
                RoutineProposal {
                    id: 8002,
                    routine: Routine { title: "하루 마무리 점검".into(), prompt: "오늘 끝난 일과 내일로 넘어가는 일을 짧게 정리해 주세요.".into(), when: Schedule::Daily { hour: 18, minute: 30 }, ..weekly.routine.clone() },
                    reason: String::new(),
                    ..weekly
                },
            ],
            // A title and a prompt long enough to wrap and clamp.
            Some("long") => vec![RoutineProposal {
                routine: Routine {
                    title: "매주 월요일 아침 지표·피드백·다음 할 일 한 번에 정리하는 주간 점검".into(),
                    prompt: format!("{} 스토어 리뷰와 GitHub 이슈, PostHog 대시보드의 지난주 숫자도 함께 보고, 달라진 점이 있으면 원인을 짧게 적어 주세요. 다음 할 일은 목표(North Star·OKR)에 맞춰 많아야 세 개만 제안 티켓으로 올리고, 제가 정해야 할 것은 카드 하나로 묶어 주세요.", weekly.routine.prompt),
                    ..weekly.routine.clone()
                },
                ..weekly
            }],
            Some(_) => vec![weekly],
        };
        std::sync::Mutex::new(out)
    })
}

/// The person's answer, as the daemon makes it. Whether it was one.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    match command {
        Command::ApproveRoutine { id, .. } => decide(*id, true, events),
        Command::DeclineRoutine { id, .. } => decide(*id, false, events),
        Command::SetIdleWake { squad, hours } => {
            *idle().lock().unwrap() = *hours;
            let _ = events.send(Event::IdleWakeChanged { squad: squad.clone(), hours: *hours });
        }
        _ => return false,
    }
    true
}

fn decide(id: u64, approve: bool, events: &broadcast::Sender<Event>) {
    let proposal = {
        let mut all = waiting().lock().unwrap();
        let Some(at) = all.iter().position(|p| p.id == id) else { return };
        all.remove(at)
    };
    let squad = || "demo".to_owned();
    let title = proposal.routine.title.clone();
    let text = if approve {
        let routines = {
            let mut saved = saved().lock().unwrap();
            let next = saved.iter().map(|r| r.id).max().unwrap_or(0) + 1;
            saved.push(Routine { id: next, on: true, ..proposal.routine });
            saved.clone()
        };
        let _ = events.send(Event::RoutinesChanged { squad: squad(), routines });
        format!("정기 업무 '{title}'을(를) 켰어요")
    } else {
        format!("정기 업무 제안을 거절했어요 · {title}")
    };
    let _ = events.send(Event::RoutineProposalClosed { squad: squad(), id, approved: approve });
    let _ = events.send(Event::Notice { session: PM, text });
}
