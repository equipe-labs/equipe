//! `FAKE_ORCHESTRA=1`: a squad for the Orchestration screen — who waits on
//! whom, subagents at work, and work that is stuck.
//!
//! - frontend is at T-47 with two subagents, one at it for 26 minutes
//!   (unusually long); its T-62 waits on the designer's T-60.
//! - backend is at T-70 with a subagent gone quiet and one that hands back
//!   two seconds after a screen attaches; its T-74 waits on its own T-73,
//!   which waits on the person (the production DB password).
//! - frontend-2 waits on backend's T-74 for its order history page: a chain
//!   that ends on the person.
//! - the designer's T-60 hasn't moved for hours and the designer isn't at it.
//! - qa's T-93 and data's T-94 wait on each other: a loop.
//!
//! Not with `FAKE_EMPTY` (T-73 is part of the default board) nor `FAKE_BUSY`.

use std::time::Duration;

use equipe_core::protocol::{Event, Member, SessionId, SessionInfo, SubagentInfo, Ticket};
use tokio::sync::broadcast;

pub fn on() -> bool {
    std::env::var("FAKE_ORCHESTRA").is_ok_and(|v| v == "1")
}

/// The members it adds: (session, name, the ticket each is on).
const MEMBERS: [(SessionId, &str, u32); 3] = [(11, "frontend-2", 90), (12, "qa", 93), (13, "data", 94)];

/// Backend's subagent that hands back once a screen has attached.
const HANDS_BACK: &str = "o-b2";

/// The tickets it adds to the board.
pub fn tickets(ticket: impl Fn(u32, &str, &str, &str, &str, &[u32]) -> Ticket) -> Vec<Ticket> {
    vec![
        ticket(74, "todo", "api", "backend", "Order history API endpoint", &[73]),
        ticket(90, "todo", "web", "frontend-2", "Order history page", &[74]),
        ticket(93, "todo", "web", "qa", "Checkout regression suite", &[94]),
        ticket(94, "todo", "web", "data", "Checkout funnel events", &[93]),
    ]
}

fn sub(id: &str, what: &str, step: &str, secs: u64, stuck_secs: Option<u64>) -> SubagentInfo {
    SubagentInfo { id: id.into(), what: what.into(), step: Some(step.into()), secs, brief: format!("{what}. Hand back what you find, with file paths."), entries: Vec::new(), stuck_secs }
}

/// The squad as a snapshot shows it: frontend's and backend's subagents,
/// and the three members it adds, each on its ticket and resting.
pub fn dress(sessions: &mut Vec<SessionInfo>, info: impl Fn(SessionId, &str) -> SessionInfo, on: impl Fn(SessionInfo, u32, &str, &str) -> SessionInfo) {
    for s in sessions.iter_mut() {
        match s.member.as_ref().map(|m| m.name.as_str()) {
            Some("frontend") => {
                s.subagents = vec![
                    sub("o-f1", "Run the gallery's visual regression tests", "Bash npm run test:visual -- gallery", 26 * 60, None),
                    sub("o-f2", "Check swipe on small screens", "Read src/swipe.ts", 3 * 60 + 10, None),
                ];
            }
            Some("backend") => {
                s.subagents = vec![
                    sub("o-b1", "Compare cursor encodings", "WebFetch RFC 4648", 7 * 60, Some(5 * 60 + 30)),
                    sub(HANDS_BACK, "Find every caller of encode_cursor", "Grep encode_cursor", 2 * 60 + 20, None),
                ];
            }
            _ => {}
        }
    }
    for (session, name, ticket) in MEMBERS {
        let role = name.split('-').next().unwrap_or(name);
        let (project, title) = match ticket {
            90 => ("web", "Order history page"),
            93 => ("web", "Checkout regression suite"),
            _ => ("web", "Checkout funnel events"),
        };
        let mut member = on(info(session, role), ticket, project, title);
        member.member = Some(Member { role: role.into(), name: name.into() });
        sessions.push(member);
    }
}

/// Backend's second subagent hands back, as a screen looks on.
pub async fn play(events: broadcast::Sender<Event>) {
    tokio::time::sleep(Duration::from_millis(2000)).await;
    let result = Some("`api/cursor.rs:18`, `api/orders.rs:142` and `api/refunds.rs:77` call it.".to_owned());
    let _ = events.send(Event::SubagentEnded { session: 3, id: HANDS_BACK.into(), result, failed: false });
}
