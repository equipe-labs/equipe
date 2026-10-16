//! A change to the team the PM proposes (`propose_team_change`): members of a
//! role to add, or one to let go. An agent never makes the change, and never
//! takes its own reading of the person's words for a yes: the proposal waits
//! for the person, who approves it with a command of their own
//! ([`crate::Command::ApproveTeamChange`] — the card's button, a key, the
//! phone) or declines it. Approved, it runs what `/add` and `/close` run.

use serde::{Deserialize, Serialize};

use crate::{Engine, SessionId, TicketId};

/// What the change does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Add,
    Remove,
}

/// How many members one proposal may add.
pub const MOST_ADDED: usize = 3;

/// A change waiting for the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamChange {
    pub id: u64,
    pub change: Change,
    /// The role added (a built-in one, or a custom one by its name), or the
    /// role of the member who leaves.
    pub role: String,
    /// How many join; 1 for a removal.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub count: usize,
    /// A removal's member, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
    /// What the new members run on; left out, what `/add` would pick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<Engine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Why, in the person's language.
    pub reason: String,
    /// The tickets that wait for the hands it brings: the new members take
    /// them first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unblocks: Vec<TicketId>,
    /// Who proposed it, and the conversation it was proposed in.
    pub by: String,
    pub session: SessionId,
    /// When, in unix seconds.
    #[serde(default)]
    pub proposed_at: u64,
    /// A removal of a member at work: the ticket it is on, which goes back
    /// to the board when it leaves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_on: Option<TicketId>,
}

fn one() -> usize {
    1
}

fn is_one(n: &usize) -> bool {
    *n == 1
}

/// The names `count` new members of `role` join as, given the names taken:
/// `role`, then `role-2`, `role-3`… — whichever are free, in order, as the
/// daemon names each one that joins.
pub fn new_names(role: &str, count: usize, taken: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in 1.. {
        if out.len() >= count {
            break;
        }
        let name = if n == 1 { role.to_owned() } else { format!("{role}-{n}") };
        if !taken.contains(&name.as_str()) {
            out.push(name);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn add() -> TeamChange {
        TeamChange {
            id: 4,
            change: Change::Add,
            role: "mobile".into(),
            count: 1,
            member: None,
            engine: None,
            model: None,
            effort: None,
            reason: "T-117 has waited 4 hours".into(),
            unblocks: vec![117],
            by: "pm".into(),
            session: 9,
            proposed_at: 1_760_000_000,
            busy_on: None,
        }
    }

    #[test]
    fn a_proposal_says_what_it_has_and_nothing_more_on_the_wire() {
        let wire = serde_json::to_value(add()).unwrap();
        assert_eq!(
            wire,
            json!({ "id": 4, "change": "add", "role": "mobile", "reason": "T-117 has waited 4 hours", "unblocks": [117], "by": "pm", "session": 9, "proposed_at": 1_760_000_000u64 })
        );
        assert_eq!(serde_json::from_value::<TeamChange>(wire).unwrap(), add());
        let bare: TeamChange = serde_json::from_value(json!({ "id": 1, "change": "remove", "role": "qa", "member": "qa-2", "reason": "idle", "by": "pm", "session": 2 })).unwrap();
        assert_eq!((bare.count, bare.member.as_deref(), bare.unblocks.len(), bare.proposed_at), (1, Some("qa-2"), 0, 0));
        let many = TeamChange { count: 2, engine: Some(Engine::Claude), model: Some("opus".into()), effort: Some("high".into()), ..add() };
        let wire = serde_json::to_value(&many).unwrap();
        assert_eq!((wire["count"].clone(), wire["engine"].clone(), wire["effort"].clone()), (json!(2), json!("claude"), json!("high")));
        assert_eq!(serde_json::from_value::<TeamChange>(wire).unwrap(), many);
    }

    #[test]
    fn new_members_are_named_as_the_daemon_names_them() {
        assert_eq!(new_names("mobile", 1, &["pm", "mobile"]), ["mobile-2"]);
        assert_eq!(new_names("mobile", 2, &["mobile", "mobile-3"]), ["mobile-2", "mobile-4"]);
        assert_eq!(new_names("qa", 1, &["pm"]), ["qa"]);
        assert!(new_names("qa", 0, &[]).is_empty());
    }
}
