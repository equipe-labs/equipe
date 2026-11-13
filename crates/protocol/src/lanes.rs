//! How many tickets a squad works at once: the squad's choice
//! ([`Parallel`]). On auto, equipe sizes its run lanes by what is left of
//! each engine's plan (`equipe_core::lanes`); on max, by the person's fixed
//! caps, whatever the plans' usage — for those who would rather burn the
//! plan, or add accounts later. Either way an engine out of its plan takes
//! nothing new, a low disk runs one ticket at a time, and approvals and the
//! sandbox stay as they are. Engine switching doesn't change with it.

use serde::{Deserialize, Serialize};

/// Max's runs per member when the person hasn't said.
pub const MAX_PER_MEMBER: u32 = 6;
/// Max's runs per engine across the squad when the person hasn't said.
pub const MAX_PER_ENGINE: u32 = 16;
/// The most runs per member the person can set.
pub const MOST_PER_MEMBER: u32 = 12;
/// The most runs per engine the person can set.
pub const MOST_PER_ENGINE: u32 = 32;

/// Who sizes the lanes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelMode {
    /// By the person's caps, whatever the plans' usage.
    Max,
    /// By the plans' usage: fewer at once as a plan runs low. Also what a
    /// mode this build doesn't know reads as (serde's `other` goes last).
    #[default]
    #[serde(other)]
    Auto,
}

impl ParallelMode {
    pub const ALL: [ParallelMode; 2] = [ParallelMode::Auto, ParallelMode::Max];

    pub fn code(self) -> &'static str {
        match self {
            ParallelMode::Auto => "auto",
            ParallelMode::Max => "max",
        }
    }

    /// `auto` or `max`, as typed in a command.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "auto" | "plan" => Some(ParallelMode::Auto),
            "max" | "booster" | "boost" => Some(ParallelMode::Max),
            _ => None,
        }
    }
}

/// The squad's parallel work: its mode, and max's caps — kept on auto too,
/// so max comes back as the person left it. A snapshot or command without it
/// is auto, as from a daemon or screen that predates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Parallel {
    #[serde(default)]
    pub mode: ParallelMode,
    /// Max's runs per member, 1 through [`MOST_PER_MEMBER`].
    #[serde(default = "per_member")]
    pub per_member: u32,
    /// Max's runs per engine across the squad, 1 through [`MOST_PER_ENGINE`].
    #[serde(default = "per_engine")]
    pub per_engine: u32,
}

fn per_member() -> u32 {
    MAX_PER_MEMBER
}

fn per_engine() -> u32 {
    MAX_PER_ENGINE
}

impl Default for Parallel {
    fn default() -> Self {
        Parallel { mode: ParallelMode::Auto, per_member: MAX_PER_MEMBER, per_engine: MAX_PER_ENGINE }
    }
}

impl Parallel {
    /// Max at `per_member` and `per_engine`, within bounds.
    pub fn max(per_member: u32, per_engine: u32) -> Self {
        Parallel { mode: ParallelMode::Max, per_member, per_engine }.clamped()
    }

    pub fn is_max(&self) -> bool {
        self.mode == ParallelMode::Max
    }

    /// What a snapshot leaves out: auto at max's defaults.
    pub fn is_default(&self) -> bool {
        *self == Parallel::default()
    }

    /// Its caps within bounds: at least one, at most [`MOST_PER_MEMBER`] and
    /// [`MOST_PER_ENGINE`].
    pub fn clamped(self) -> Self {
        Parallel { per_member: self.per_member.clamp(1, MOST_PER_MEMBER), per_engine: self.per_engine.clamp(1, MOST_PER_ENGINE), ..self }
    }
}

/// An engine whose new runs are paced to its plan's budget (on auto): the
/// squad starts `cap` runs at once on it instead of `of`, so this week's plan
/// lasts — never a lighter model, never a run stopped mid-turn. Max, the
/// person's override, ends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pacing {
    pub engine: crate::Engine,
    /// Runs at once allowed now.
    pub cap: u32,
    /// Runs at once the plan's usage alone would allow.
    pub of: u32,
    /// Until when the plan is meant to last (unix seconds).
    pub until: i64,
    /// `until` is the other engine's return, not this plan's reset.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub other_engine: bool,
    /// When pacing last changed (unix seconds).
    #[serde(default)]
    pub since: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn auto_is_the_default_and_whatever_is_left_out_reads_as_it() {
        assert_eq!(Parallel::default(), Parallel { mode: ParallelMode::Auto, per_member: 6, per_engine: 16 });
        assert_eq!(serde_json::from_value::<Parallel>(json!({})).unwrap(), Parallel::default());
        assert_eq!(serde_json::from_value::<Parallel>(json!({ "mode": "max" })).unwrap(), Parallel::max(6, 16));
        assert_eq!(serde_json::from_value::<Parallel>(json!({ "mode": "warp", "per_member": 3 })).unwrap().mode, ParallelMode::Auto, "a newer mode: auto");
        let max = Parallel::max(8, 20);
        assert_eq!(serde_json::to_value(max).unwrap(), json!({ "mode": "max", "per_member": 8, "per_engine": 20 }));
        assert_eq!(serde_json::from_value::<Parallel>(serde_json::to_value(max).unwrap()).unwrap(), max);
    }

    #[test]
    fn pacing_is_told_as_json_and_a_snapshot_without_it_reads_as_none() {
        let pacing = Pacing { engine: crate::Engine::Claude, cap: 6, of: 14, until: 99, other_engine: false, since: 5 };
        let wire = serde_json::to_value(pacing).unwrap();
        assert_eq!(wire, json!({ "engine": "claude", "cap": 6, "of": 14, "until": 99, "since": 5 }));
        assert_eq!(serde_json::from_value::<Pacing>(wire).unwrap(), pacing);
        let event = crate::Event::PacingChanged { squad: "acme".into(), pacing: vec![pacing] };
        assert_eq!((event.squad(), event.session()), (Some("acme"), None));
        assert_eq!(serde_json::from_str::<crate::Event>(&serde_json::to_string(&event).unwrap()).unwrap(), event);
        let old: crate::Snapshot = serde_json::from_str(r#"{"squad":"acme","sessions":[]}"#).unwrap();
        assert!(old.pacing.is_empty());
        assert!(serde_json::to_value(&old).unwrap().get("pacing").is_none());
    }

    #[test]
    fn the_caps_stay_within_bounds() {
        assert_eq!(Parallel::max(0, 0), Parallel::max(1, 1));
        assert_eq!(Parallel::max(99, 99), Parallel::max(MOST_PER_MEMBER, MOST_PER_ENGINE));
        assert_eq!((MOST_PER_MEMBER, MOST_PER_ENGINE), (12, 32));
        assert!(Parallel::default().is_default() && !Parallel::max(6, 16).is_default());
        assert!(!Parallel { per_member: 4, ..Parallel::default() }.is_default(), "auto, the caps the person left: still said");
        assert!(ParallelMode::ALL.iter().all(|m| ParallelMode::parse(m.code()) == Some(*m)));
        assert_eq!(ParallelMode::parse(" Max "), Some(ParallelMode::Max));
        assert_eq!(ParallelMode::parse("fast"), None);
    }
}
