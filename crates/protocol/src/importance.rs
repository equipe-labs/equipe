//! How much a thing that waits on the person matters: what it holds up.
//!
//! `equipe_core::importance` works it out (a pure function with named
//! weights); this is what travels — with each item of the Status screen's
//! list of what waits, and what the desktop orders Needs you by. Facts and a
//! short reason, never a sentence in the person's language: screens put the
//! facts in words themselves.

use serde::{Deserialize, Serialize};

use crate::{Priority, TicketId, TicketStatus};

/// What kind of thing waits on the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitKind {
    /// A member asks for permission to do something.
    Permission,
    /// Questions with options, a change to the team, a routine, work to confirm.
    Decision,
    /// A sign-in, a code, a human check, or a secret only the person can give.
    SignIn,
    /// A ticket a member put down until the person does their part.
    Ticket,
    /// A run that stopped without a hand-in.
    Stalled,
    /// Work handed in, to accept.
    Review,
    /// Suggested work to take on or drop.
    Proposal,
    /// Accepted work not in the main branch yet.
    NotOnMain,
    /// Anything else (an offer to keep a login, say).
    Other,
}

/// What one waiting thing holds up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Impact {
    /// The sum of the named weights: higher matters more. 0: it holds nothing up.
    pub score: u32,
    /// Open tickets that wait on it, directly or through `depends_on`.
    pub blocks: u32,
    /// The most urgent of what it blocks and of its own ticket; `None`: nothing
    /// open is attached to it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Deploy or release work waits on it, or the PM said to hold deploys for it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub holds_deploy: bool,
    /// Members parked or idle because of it.
    pub idle_members: u32,
    /// The facts in one short English line, for the PM and logs.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

impl Impact {
    /// Whether it holds up urgent or high-priority work.
    pub fn is_pressing(&self) -> bool {
        matches!(self.priority, Some(Priority::Urgent | Priority::High))
    }

    /// Whether it belongs in a list's "most blocking" part: it blocks another
    /// ticket, holds a deploy, or holds urgent or high work.
    pub fn is_blocking(&self) -> bool {
        self.blocks > 0 || self.holds_deploy || self.is_pressing()
    }
}

/// One item of what waits on the person, as the Status screen lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waiting {
    pub kind: WaitKind,
    /// The ticket it is about, when it is about one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    /// The ticket's status, when `ticket` is set and known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TicketStatus>,
    /// The ticket's title, or what is asked, in a line.
    pub title: String,
    /// Since when it has waited, unix seconds; `None`: not known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    pub impact: Impact,
}

/// Which waiting things may escalate at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Only {
    /// Anything that holds work up.
    #[default]
    Blocking,
    /// Only what holds up urgent or high work, or a deploy.
    Urgent,
}

/// How a squad escalates what stays a bottleneck, and when it may speak: per
/// squad, over the person's default (`equipe_core::escalation`). Quiet hours
/// are minutes after local midnight; the person's own offset from UTC says
/// what "local" is (the daemon's machine may be elsewhere).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifySettings {
    /// Escalate at all. Off: things only wait in Needs you.
    pub on: bool,
    pub only: Only,
    /// Minutes an item that holds urgent work or a deploy waits before levels 2, 3 and 4.
    pub level_mins: [u32; 3],
    /// Never notify between `quiet_from` and `quiet_to`; what waited goes out as one summary after.
    pub quiet_on: bool,
    pub quiet_from: u16,
    pub quiet_to: u16,
    /// The person's offset from UTC, seconds; `None`: the daemon machine's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utc_offset_secs: Option<i32>,
    /// At most this many notifications an hour, all items together.
    pub max_per_hour: u32,
    /// Also tell paired phones and Telegram.
    pub phone: bool,
    /// Tell again every this many minutes about what holds a deploy up; `None`: not again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remind_deploy_mins: Option<u32>,
}

impl Default for NotifySettings {
    fn default() -> Self {
        NotifySettings {
            on: true,
            only: Only::Blocking,
            level_mins: [120, 480, 1440],
            quiet_on: true,
            quiet_from: 23 * 60,
            quiet_to: 8 * 60,
            utc_offset_secs: None,
            max_per_hour: 3,
            phone: true,
            remind_deploy_mins: None,
        }
    }
}

/// Why a notice reads as it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    /// One item reached a new level.
    Escalation,
    /// What waited through quiet hours, or came due together, as one.
    Batch,
    /// The daily summary of what has blocked work for a day or more.
    Daily,
    /// The person has missed this several times.
    Missed,
}

/// One item of a notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeItem {
    pub ticket: Option<TicketId>,
    pub title: String,
    pub level: u8,
    pub blocks: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub holds_deploy: bool,
    /// How long it has waited, seconds.
    pub waited_secs: u64,
    /// How many notices about it went unanswered (a `Missed` notice's count).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub missed: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// What the person is told about things that stay a bottleneck (`Event::Escalated`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub kind: NoticeKind,
    pub items: Vec<NoticeItem>,
    /// Phones and Telegram too ([`NotifySettings::phone`]); the desktop always.
    pub phone: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_impact_survives_the_wire_and_an_older_daemons_missing_parts_read_as_nothing_held() {
        let waiting = Waiting {
            kind: WaitKind::Ticket,
            ticket: Some(7),
            status: Some(TicketStatus::Waiting),
            title: "Sign in to AWS".into(),
            since: Some(1_760_000_000),
            impact: Impact { score: 55, blocks: 5, priority: Some(Priority::Urgent), holds_deploy: true, idle_members: 1, reason: "blocks 5".into() },
        };
        let back: Waiting = serde_json::from_str(&serde_json::to_string(&waiting).unwrap()).unwrap();
        assert_eq!(back, waiting);
        let old: Impact = serde_json::from_value(json!({})).unwrap();
        assert_eq!(old, Impact::default());
        assert!(!old.is_blocking());
        assert_eq!(serde_json::to_value(WaitKind::NotOnMain).unwrap(), json!("not_on_main"));
        let bare = serde_json::to_value(Impact::default()).unwrap();
        assert!(bare.get("priority").is_none() && bare.get("holds_deploy").is_none() && bare.get("reason").is_none(), "{bare}");
    }

    #[test]
    fn blocking_is_a_ticket_held_a_deploy_held_or_urgent_work_held() {
        let none = Impact::default();
        assert!(!none.is_blocking());
        assert!(Impact { blocks: 1, ..none.clone() }.is_blocking());
        assert!(Impact { holds_deploy: true, ..none.clone() }.is_blocking());
        assert!(Impact { priority: Some(Priority::High), ..none.clone() }.is_blocking());
        assert!(!Impact { priority: Some(Priority::Normal), idle_members: 2, ..none }.is_blocking(), "a parked member alone is not blocking work");
    }
}
