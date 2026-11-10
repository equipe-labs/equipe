//! What a call says about the board: the answer to "Where are we?" and the
//! briefing a call opens with. A fast model puts it in a sentence or two for
//! the ear; without it (or when it fails), a template does.

use equipe_protocol::{Ticket, TicketStatus};

use super::judge::{LlmConfig, ask_model};

/// What is being asked of the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// "Where are we?" — what is going on now.
    Status,
    /// The call's opening: what changed since the last call.
    Briefing,
}

/// Without a last call, the briefing looks back this far (s).
pub const BRIEFING_LOOKBACK_S: u64 = 3 * 60 * 60;
/// "Recently done", for a status (s).
const RECENT_S: u64 = 24 * 60 * 60;
/// Most tickets of each kind told.
const MOST: usize = 4;

/// One ticket as a call tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    pub who: Option<String>,
}

/// The board, as much as a call tells.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub in_progress: Vec<Item>,
    pub review: Vec<Item>,
    pub done: Vec<Item>,
}

impl Snapshot {
    /// For a status: all in progress and in review, done in the last day.
    /// For a briefing: only what changed since `since` (unix s).
    pub fn of(tickets: &[Ticket], ask: Ask, since: u64, now: u64) -> Snapshot {
        let mut t: Vec<&Ticket> = tickets.iter().collect();
        t.sort_by_key(|t| std::cmp::Reverse(t.updated_at.unwrap_or(0)));
        let changed = |t: &Ticket, after: u64| t.updated_at.is_some_and(|u| u >= after);
        let pick = |status: TicketStatus, after: Option<u64>| -> Vec<Item> {
            t.iter()
                .filter(|t| t.status == status && after.is_none_or(|a| changed(t, a)))
                .take(MOST)
                .map(|t| Item { title: t.title.clone(), who: t.assignee.clone().or_else(|| t.owner.clone()) })
                .collect()
        };
        match ask {
            Ask::Status => Snapshot {
                in_progress: pick(TicketStatus::InProgress, None),
                review: pick(TicketStatus::Review, None),
                done: pick(TicketStatus::Done, Some(now.saturating_sub(RECENT_S))),
            },
            Ask::Briefing => Snapshot {
                in_progress: pick(TicketStatus::InProgress, Some(since)),
                review: pick(TicketStatus::Review, Some(since)),
                done: pick(TicketStatus::Done, Some(since)),
            },
        }
    }

    pub fn is_empty(&self) -> bool {
        self.in_progress.is_empty() && self.review.is_empty() && self.done.is_empty()
    }
}

/// What ends each kind of answer.
pub fn closing(ask: Ask) -> &'static str {
    match ask {
        Ask::Status => "Anything else?",
        Ask::Briefing => "What do you need?",
    }
}

/// "2 in progress, 1 waiting for review." — no model needed.
pub fn template(s: &Snapshot, ask: Ask) -> String {
    let mut parts = Vec::new();
    match ask {
        Ask::Status => {
            if !s.in_progress.is_empty() {
                parts.push(format!("{} in progress", s.in_progress.len()));
            }
            if !s.review.is_empty() {
                parts.push(format!("{} waiting for review", s.review.len()));
            }
            if !s.done.is_empty() {
                parts.push(format!("{} done today", s.done.len()));
            }
            if parts.is_empty() {
                return "Nothing is in progress right now.".to_owned();
            }
            format!("{}.", parts.join(", "))
        }
        Ask::Briefing => {
            if !s.done.is_empty() {
                parts.push(format!("{} finished", s.done.len()));
            }
            if !s.review.is_empty() {
                parts.push(format!("{} waiting for review", s.review.len()));
            }
            if !s.in_progress.is_empty() {
                parts.push(format!("{} started", s.in_progress.len()));
            }
            if parts.is_empty() {
                return "Nothing new since our last call.".to_owned();
            }
            format!("Since our last call, {}.", parts.join(", "))
        }
    }
}

fn prompt(s: &Snapshot, ask: Ask) -> (String, String) {
    let what = match ask {
        Ask::Status => "The person asked where things stand. Say what is in progress, what waits for their review, and what was done recently.",
        Ask::Briefing => "The person just started a call. Start with \"Since our last call,\" and say what finished, what waits for their review and what started.",
    };
    let system = format!(
        "You speak for a software squad on a voice call with the person who leads it. {what} \
Answer in one or two short spoken sentences of plain English: no lists, no ticket numbers, no markdown, no question at the end. \
Name who did what by role (Designer, Backend…)."
    );
    let mut user = String::new();
    for (label, items) in [("In progress", &s.in_progress), ("Waiting for review", &s.review), ("Done", &s.done)] {
        user.push_str(&format!("{label}:\n"));
        if items.is_empty() {
            user.push_str("- (none)\n");
        }
        for i in items {
            user.push_str(&format!("- {} ({})\n", i.title, i.who.as_deref().unwrap_or("unassigned")));
        }
    }
    (system, user)
}

/// The words for `ask`, closing included: the model's, else the template's.
pub async fn speak(s: &Snapshot, ask: Ask, llm: &LlmConfig) -> String {
    let mut body = None;
    if let Some(key) = &llm.key
        && !s.is_empty()
    {
        let (system, user) = prompt(s, ask);
        match tokio::time::timeout(llm.timeout, ask_model(llm, key, &system, &user)).await {
            Ok(Ok(text)) if !text.trim().is_empty() => body = Some(text.trim().to_owned()),
            Ok(Ok(_)) => eprintln!("call status: empty model answer"),
            Ok(Err(e)) => eprintln!("call status: model failed: {e}"),
            Err(_) => eprintln!("call status: model timed out after {:?}", llm.timeout),
        }
    }
    let body = body.unwrap_or_else(|| template(s, ask));
    format!("{body} {}", closing(ask))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(id: u64, title: &str, status: TicketStatus, who: &str, updated: u64) -> Ticket {
        let mut t: Ticket = serde_json::from_value(serde_json::json!({
            "id": id, "title": title, "body": "", "status": "todo", "owner": null, "assignee": who,
            "project": null, "depends_on": [], "branch": null, "created_by": "you", "note": null,
        }))
        .unwrap();
        t.status = status;
        t.updated_at = Some(updated);
        t
    }

    fn board() -> Vec<Ticket> {
        vec![
            ticket(1, "Login mockups", TicketStatus::Done, "designer", 1000),
            ticket(2, "API check", TicketStatus::Review, "backend", 900),
            ticket(3, "Call screen", TicketStatus::InProgress, "frontend", 500),
            ticket(4, "Old work", TicketStatus::Done, "qa", 10),
        ]
    }

    #[test]
    fn status_template() {
        let s = Snapshot::of(&board(), Ask::Status, 0, 1000);
        assert_eq!(template(&s, Ask::Status), "1 in progress, 1 waiting for review, 2 done today.");
        assert_eq!(template(&Snapshot::default(), Ask::Status), "Nothing is in progress right now.");
    }

    #[test]
    fn briefing_only_since_the_last_call() {
        let s = Snapshot::of(&board(), Ask::Briefing, 800, 1000);
        assert_eq!(s.done, vec![Item { title: "Login mockups".into(), who: Some("designer".into()) }]);
        assert_eq!(s.review.len(), 1);
        assert!(s.in_progress.is_empty());
        assert_eq!(template(&s, Ask::Briefing), "Since our last call, 1 finished, 1 waiting for review.");
    }

    #[tokio::test]
    async fn speak_falls_back_to_the_template_when_the_model_fails() {
        let llm = LlmConfig { key: Some("test".into()), base: "http://127.0.0.1:9".into(), ..LlmConfig::off() };
        let s = Snapshot::of(&board(), Ask::Status, 0, 1000);
        assert_eq!(speak(&s, Ask::Status, &llm).await, "1 in progress, 1 waiting for review, 2 done today. Anything else?");
    }
}
