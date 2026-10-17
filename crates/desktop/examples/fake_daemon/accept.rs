//! `FAKE_ACCEPT=1`: the person told the PM "전부 수락할게", and the PM put
//! T-42 and T-71 (both in review) to them on a card to confirm. Confirmed,
//! each ticket gets what the board's buttons send — accept, then merge when
//! the card says so — as the daemon would; declined, the PM's conversation
//! says so. `FAKE_ACCEPT=merge` asks to merge them too; `FAKE_ACCEPT=words`
//! has the card raised from the person's own words rather than by the PM.

use equipe_core::protocol::accept::{AcceptProposal, FROM_WORDS, Land, Verdict};
use equipe_core::protocol::{Command, Event};
use tokio::sync::broadcast;

const ID: u64 = 8001;
const PM: u64 = 1;

fn mode() -> Option<String> {
    std::env::var("FAKE_ACCEPT").ok().filter(|v| !v.is_empty() && v != "0")
}

/// The cards still waiting.
pub fn cards() -> Vec<AcceptProposal> {
    waiting().lock().unwrap().clone()
}

fn waiting() -> &'static std::sync::Mutex<Vec<AcceptProposal>> {
    static WAITING: std::sync::OnceLock<std::sync::Mutex<Vec<AcceptProposal>>> = std::sync::OnceLock::new();
    WAITING.get_or_init(|| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let card = AcceptProposal {
            id: ID,
            verdict: Verdict::Accept,
            tickets: vec![42, 71],
            note: String::new(),
            summary: "결제 수단 선택과 요청 수 제한 — 둘 다 테스트를 통과했어요.".into(),
            by: "pm".into(),
            session: PM,
            proposed_at: now,
        };
        let out = match mode().as_deref() {
            None => vec![],
            Some("merge") => vec![AcceptProposal { verdict: Verdict::Merge, ..card }],
            Some("words") => vec![AcceptProposal { by: FROM_WORDS.into(), summary: String::new(), ..card }],
            Some(_) => vec![card],
        };
        std::sync::Mutex::new(out)
    })
}

/// The person's answer, as the daemon takes it: the commands the board's
/// buttons send, for the fake board to run. `None`: not one.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> Option<Vec<Command>> {
    let (id, confirmed) = match command {
        Command::ConfirmAccept { id, tickets, .. } => (*id, Some(tickets.clone())),
        Command::DeclineAccept { id, .. } => (*id, None),
        Command::ProposeAccept { session, verdict, tickets, note, .. } => {
            // Raised from the person's words on the screen: the card, at once.
            let card = AcceptProposal { id: ID + 1 + tickets.len() as u64, verdict: *verdict, tickets: tickets.clone(), note: note.clone(), summary: String::new(), by: FROM_WORDS.into(), session: *session, proposed_at: 0 };
            let mut all = waiting().lock().unwrap();
            if !all.iter().any(|c| c.verdict == *verdict && tickets.iter().all(|t| c.tickets.contains(t))) {
                all.push(card.clone());
                let _ = events.send(Event::AcceptProposed { squad: "demo".into(), proposal: card });
            }
            return Some(Vec::new());
        }
        _ => return None,
    };
    let card = {
        let mut all = waiting().lock().unwrap();
        let at = all.iter().position(|c| c.id == id)?;
        all.remove(at)
    };
    let squad = || "demo".to_owned();
    let mut out = Vec::new();
    let mut acted = Vec::new();
    match confirmed {
        Some(picked) => {
            for pick in picked.iter().filter(|p| card.tickets.contains(&p.ticket)) {
                match card.verdict {
                    Verdict::Changes => out.push(Command::Reopen { squad: squad(), ticket: pick.ticket, note: card.note.clone() }),
                    Verdict::Accept | Verdict::Merge => {
                        out.push(Command::Accept { squad: squad(), ticket: pick.ticket });
                        if let Some(land) = pick.merge {
                            out.push(Command::Merge { squad: squad(), ticket: pick.ticket, push: land == Land::Push });
                        }
                    }
                }
                acted.push(pick.ticket);
            }
            let _ = events.send(Event::AcceptClosed { squad: squad(), id, confirmed: true, tickets: acted.clone() });
            for ticket in acted {
                let _ = events.send(Event::Notice { session: PM, text: format!("T-{ticket}을(를) 수락했습니다.") });
            }
        }
        None => {
            let _ = events.send(Event::AcceptClosed { squad: squad(), id, confirmed: false, tickets: Vec::new() });
            let list = card.tickets.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
            let _ = events.send(Event::Notice { session: PM, text: format!("{list} 수락하지 않았어요 — 검토 대기로 그대로 둬요") });
        }
    }
    Some(out)
}
