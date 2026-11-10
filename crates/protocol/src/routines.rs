//! Routines: the squad's regular operations work — a weekly report, a look
//! at the dependencies — that the person sets up once and equipe hands the
//! PM on its schedule, as if the person had sent it then. Only the person
//! makes, changes or runs one ([`crate::Command::SaveRoutine`],
//! [`crate::Command::RunRoutine`]); a member never does. The PM may only
//! propose one ([`RoutineProposal`]), which waits for the person's click
//! ([`crate::Command::ApproveRoutine`]).

use serde::{Deserialize, Serialize};

/// When a routine runs, in the daemon machine's local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "every", rename_all = "snake_case")]
pub enum Schedule {
    Daily { hour: u8, minute: u8 },
    /// `weekday`: 1 Monday … 7 Sunday.
    Weekly { weekday: u8, hour: u8, minute: u8 },
}

impl Schedule {
    pub fn hour_minute(self) -> (u8, u8) {
        match self {
            Schedule::Daily { hour, minute } | Schedule::Weekly { hour, minute, .. } => (hour, minute),
        }
    }

    /// Within its ranges: an hour 0–23, a minute 0–59, a weekday 1–7.
    pub fn is_valid(self) -> bool {
        let (hour, minute) = self.hour_minute();
        let day = match self {
            Schedule::Weekly { weekday, .. } => (1..=7).contains(&weekday),
            Schedule::Daily { .. } => true,
        };
        hour < 24 && minute < 60 && day
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routine {
    /// `0` in a [`crate::Command::SaveRoutine`]: a new one, numbered by the daemon.
    pub id: u32,
    pub title: String,
    /// What the PM is asked, each time.
    pub prompt: String,
    pub when: Schedule,
    /// Off: kept, not run.
    #[serde(default = "on")]
    pub on: bool,
    /// When it last ran, unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<u64>,
    /// When it runs next, unix seconds: the daemon's to work out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run: Option<u64>,
}

fn on() -> bool {
    true
}

/// A routine the PM proposed (`propose_routine`), waiting for the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutineProposal {
    pub id: u64,
    /// What would be saved: `id` 0, on.
    pub routine: Routine,
    /// Why, in the person's language.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// Who proposed it, and the conversation it was proposed in.
    pub by: String,
    pub session: crate::SessionId,
    /// When, in unix seconds.
    #[serde(default)]
    pub proposed_at: u64,
}

/// How long an empty board waits before the PM is woken, unless the person says otherwise.
pub const IDLE_WAKE_HOURS: u32 = 24;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_routine_reads_back_as_it_was_written() {
        let routine = Routine { id: 3, title: "Weekly report".into(), prompt: "Sum up the week".into(), when: Schedule::Weekly { weekday: 1, hour: 9, minute: 0 }, on: true, last_run: None, next_run: Some(1_700_000_000) };
        let json = serde_json::to_value(&routine).unwrap();
        assert_eq!(json["when"], serde_json::json!({ "every": "weekly", "weekday": 1, "hour": 9, "minute": 0 }));
        assert_eq!(serde_json::from_value::<Routine>(json).unwrap(), routine);
        let bare: Routine = serde_json::from_str(r#"{"id":0,"title":"t","prompt":"p","when":{"every":"daily","hour":8,"minute":30}}"#).unwrap();
        assert!(bare.on, "on unless said otherwise");
        assert!(Schedule::Daily { hour: 23, minute: 59 }.is_valid());
        assert!(!Schedule::Weekly { weekday: 0, hour: 9, minute: 0 }.is_valid());
        assert!(!Schedule::Daily { hour: 24, minute: 0 }.is_valid());
    }
}
