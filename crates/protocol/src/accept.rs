//! Handed-in work the person said in words they accept, merge or send back
//! ("전부 수락", "T-1이랑 T-3 수락해줘", "머지까지 해줘") — put in front of
//! them as a card to confirm. The PM proposes it (`propose_accept`), or a
//! screen does from the person's own unambiguous words
//! ([`crate::Command::ProposeAccept`]); either way it only waits. Their words
//! are never the acceptance, an agent's even less: only their own command
//! ([`crate::Command::ConfirmAccept`] — the card's button, a key, the phone)
//! runs what the board's buttons run — [`crate::Command::Accept`],
//! [`crate::Command::Merge`], [`crate::Command::Reopen`] — or they say no
//! ([`crate::Command::DeclineAccept`]).

use serde::{Deserialize, Serialize};

use crate::{SessionId, TicketId};

/// What the person said to do with the work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Accept it, landing code the project's way as the board's accept does.
    Accept,
    /// Accept it and merge it into main — what is accepted already, merge.
    Merge,
    /// Send it back with what to change (`note`).
    Changes,
}

/// Who put it in front of the person: the PM's tool, or a screen reading the
/// person's own words.
pub const FROM_WORDS: &str = "you";

/// A proposal waiting for the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptProposal {
    pub id: u64,
    pub verdict: Verdict,
    /// The tickets, each in review (or, to merge, accepted and not on main yet).
    pub tickets: Vec<TicketId>,
    /// To send back: what to change, in the person's words.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// One line on what it is, in the person's language.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    /// The member that proposed it (the PM), or [`FROM_WORDS`].
    pub by: String,
    /// The conversation it shows in.
    pub session: SessionId,
    /// When, in unix seconds.
    #[serde(default)]
    pub proposed_at: u64,
}

impl AcceptProposal {
    /// Raised from the person's own words by a screen, not by an agent.
    pub fn from_words(&self) -> bool {
        self.by == FROM_WORDS
    }
}

/// How a ticket confirmed lands, once accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Land {
    /// Merged into the project's main branch.
    Merge,
    /// Merged, and main pushed to `origin`.
    Push,
}

/// One ticket the person confirmed, and — the screen knowing the project's
/// way — whether it merges after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confirmed {
    pub ticket: TicketId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<Land>,
}

impl Confirmed {
    pub fn only(ticket: TicketId) -> Self {
        Self { ticket, merge: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn proposal() -> AcceptProposal {
        AcceptProposal { id: 4, verdict: Verdict::Accept, tickets: vec![1, 3], note: String::new(), summary: "로그인 화면과 결제".into(), by: "pm".into(), session: 9, proposed_at: 1_760_000_000 }
    }

    #[test]
    fn a_proposal_says_what_it_has_and_nothing_more_on_the_wire() {
        let wire = serde_json::to_value(proposal()).unwrap();
        assert_eq!(wire, json!({ "id": 4, "verdict": "accept", "tickets": [1, 3], "summary": "로그인 화면과 결제", "by": "pm", "session": 9, "proposed_at": 1_760_000_000u64 }));
        assert_eq!(serde_json::from_value::<AcceptProposal>(wire).unwrap(), proposal());
        let back = AcceptProposal { verdict: Verdict::Changes, note: "버튼을 더 크게".into(), summary: String::new(), by: FROM_WORDS.into(), ..proposal() };
        let wire = serde_json::to_value(&back).unwrap();
        assert_eq!((wire["verdict"].clone(), wire["note"].clone(), wire.get("summary")), (json!("changes"), json!("버튼을 더 크게"), None));
        assert!(serde_json::from_value::<AcceptProposal>(wire).unwrap().from_words());
        let bare: AcceptProposal = serde_json::from_value(json!({ "id": 1, "verdict": "merge", "tickets": [2], "by": "pm", "session": 2 })).unwrap();
        assert_eq!((bare.verdict, bare.note.as_str(), bare.proposed_at, bare.from_words()), (Verdict::Merge, "", 0, false));
    }

    #[test]
    fn a_confirmed_ticket_merges_only_when_it_says_so() {
        assert_eq!(serde_json::to_value(Confirmed::only(3)).unwrap(), json!({ "ticket": 3 }));
        let pushed = Confirmed { ticket: 1, merge: Some(Land::Push) };
        assert_eq!(serde_json::to_value(pushed).unwrap(), json!({ "ticket": 1, "merge": "push" }));
        assert_eq!(serde_json::from_value::<Confirmed>(json!({ "ticket": 1, "merge": "push" })).unwrap(), pushed);
    }
}
