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
                title: "Weekly progress report".into(),
                prompt: "Sum up, by project, the tickets finished this past week and those still in progress, then tell me what is stuck and pick just three things to do first next week. Anything that needs my decision goes at the top.".into(),
                when: Schedule::Weekly { weekday: 1, hour: 9, minute: 0 },
                on: true,
                last_run: None,
                next_run: None,
            },
            reason: "You ask for last week's summary every Monday, so it may help to have it ready at a set time.".into(),
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
                    routine: Routine { title: "End-of-day check".into(), prompt: "Briefly sum up what finished today and what carries over to tomorrow.".into(), when: Schedule::Daily { hour: 18, minute: 30 }, ..weekly.routine.clone() },
                    reason: String::new(),
                    ..weekly
                },
            ],
            // A title and a prompt long enough to wrap and clamp.
            Some("long") => vec![RoutineProposal {
                routine: Routine {
                    title: "A weekly check every Monday morning that rounds up metrics, feedback and what to do next in one go".into(),
                    prompt: format!("{} Look at the store reviews, GitHub issues and last week's numbers on the PostHog dashboard too, and note briefly why anything changed. Propose at most three next steps as tickets, in line with the goals (North Star, OKRs), and put whatever I have to decide on one card.", weekly.routine.prompt),
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
        format!("Turned on the routine '{title}'")
    } else {
        format!("Declined the routine · {title}")
    };
    let _ = events.send(Event::RoutineProposalClosed { squad: squad(), id, approved: approve });
    let _ = events.send(Event::Notice { session: PM, text });
}
