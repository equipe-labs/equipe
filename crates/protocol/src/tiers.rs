//! Which model and effort a ticket's run lanes get: its *tier*. A lane picks
//! it once, when it starts (a model change mid-session would throw the
//! prompt cache away), from abstract steps that every engine maps to a model
//! and an effort of its own (`equipe_core::tiers`), so a ticket keeps its
//! step when its member moves engines.
//!
//! The squad's [`ModelChoice`] says whether equipe picks at all. On
//! [`ModelChoice::QualityFirst`] — the default — it only ever raises a
//! ticket above its member's own model and effort, never lowers it; what a
//! cheaper step would have been is recorded beside it ([`TicketModel::shadow`])
//! and never acted on. The person may raise, lower or pin any ticket's tier.
//!
//! Everything here is optional on the wire: a ticket without [`TicketModel`]
//! runs on its member's own model, as before.

use serde::{Deserialize, Serialize};

use crate::Engine;

/// A step of model strength and thinking, weakest first. A step this build
/// doesn't know reads as [`ModelTier::Strong`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    /// The cheapest model: reading, checking, bulk work whose results are checked.
    Light,
    /// An everyday model: a small change in one place that a command checks.
    Standard,
    /// A strong model at moderate thinking: features and fixes.
    #[default]
    Strong,
    /// A strong model thinking hard: risky, broad or design work, reviews.
    Deep,
    /// A strong model thinking hardest: reached by escalation or the person.
    Max,
}

impl<'de> Deserialize<'de> for ModelTier {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(ModelTier::parse(&text).unwrap_or_default())
    }
}

impl ModelTier {
    pub const ALL: [ModelTier; 5] = [ModelTier::Light, ModelTier::Standard, ModelTier::Strong, ModelTier::Deep, ModelTier::Max];

    pub fn code(self) -> &'static str {
        match self {
            ModelTier::Light => "light",
            ModelTier::Standard => "standard",
            ModelTier::Strong => "strong",
            ModelTier::Deep => "deep",
            ModelTier::Max => "max",
        }
    }

    /// A tier as typed in a command.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        ModelTier::ALL.into_iter().find(|t| t.code() == text)
    }

    /// One step up; the top stays.
    pub fn up(self) -> Self {
        ModelTier::ALL.get(self as usize + 1).copied().unwrap_or(self)
    }

    /// One step down; the bottom stays.
    pub fn down(self) -> Self {
        (self as usize).checked_sub(1).map_or(self, |at| ModelTier::ALL[at])
    }
}

/// The squad's say on per-ticket model choice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelChoice {
    /// Members' own models only, as before tiers: nothing raised.
    Off,
    /// Raise a ticket's model and effort where it matters, never lower them
    /// below its member's own. Also what a choice this build doesn't know
    /// reads as.
    #[default]
    #[serde(other)]
    QualityFirst,
}

impl ModelChoice {
    pub const ALL: [ModelChoice; 2] = [ModelChoice::QualityFirst, ModelChoice::Off];

    pub fn code(self) -> &'static str {
        match self {
            ModelChoice::QualityFirst => "quality",
            ModelChoice::Off => "off",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "quality" | "quality_first" | "quality-first" | "on" | "auto" => Some(ModelChoice::QualityFirst),
            "off" | "own" | "members" => Some(ModelChoice::Off),
            _ => None,
        }
    }

    pub fn is_default(&self) -> bool {
        *self == ModelChoice::default()
    }
}

/// Why a ticket runs at its tier: the main reason, for the person to read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TierReason {
    /// Code work: features and fixes start strong.
    #[default]
    Code,
    /// It touches something costly to get wrong ([`TicketModel::risk`]).
    Risk,
    /// One change made the same way in several places, or a large refactor.
    Broad,
    /// Design work.
    Design,
    /// A design QA review: a review is never weaker than the work.
    DesignQa,
    /// A review of someone else's work (QA, security).
    Review,
    /// The PM's own work keeps the PM's model.
    Pm,
    /// Writing, research or a decision in the squad's folder.
    Task,
    /// Doing something in an app or the browser.
    Action,
    /// Docs or a changelog.
    Docs,
    /// A small change in one place, checked by a command.
    Small,
    /// Reading only: research, logs, a status check.
    ReadOnly,
    /// The member's own model and effort were already stronger.
    MemberFloor,
    /// The member's own model, as set: per-ticket choice is off, or equipe
    /// can't tell how strong its model is.
    MemberOwn,
    /// Raised after work was sent back for being wrong, or checks failed.
    Escalated,
    /// The person chose it.
    Person,
    /// A reason this build doesn't know.
    #[serde(other)]
    Other,
}

/// What makes a ticket costly to get wrong: never on a cheaper tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTag {
    /// Sign-in, permissions, accounts, tokens.
    Auth,
    /// Money: payments, billing, invoices.
    Payments,
    /// Data migrated or deleted, schema changes.
    Migration,
    /// Security: secrets, encryption, vulnerabilities.
    Security,
    /// Concurrency: locks, races, threads.
    Concurrency,
    /// Releases and deploys.
    Release,
    /// A tag this build doesn't know.
    #[serde(other)]
    Other,
}

impl RiskTag {
    pub const ALL: [RiskTag; 6] = [RiskTag::Auth, RiskTag::Payments, RiskTag::Migration, RiskTag::Security, RiskTag::Concurrency, RiskTag::Release];

    pub fn code(self) -> &'static str {
        match self {
            RiskTag::Auth => "auth",
            RiskTag::Payments => "payments",
            RiskTag::Migration => "migration",
            RiskTag::Security => "security",
            RiskTag::Concurrency => "concurrency",
            RiskTag::Release => "release",
            RiskTag::Other => "other",
        }
    }
}

/// Who set a ticket's tier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TierBy {
    /// equipe, one step up after the work came back wrong.
    Escalation,
    /// The person.
    Person,
    /// equipe's rules, at its lane's start. Also who a word this build
    /// doesn't know reads as.
    #[default]
    #[serde(other)]
    Rule,
}

/// A ticket's tier, and what its latest lane ran on. Absent: the ticket
/// runs on its member's own model.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketModel {
    /// The step its lanes run at; `None`: its member's own model, whose
    /// strength equipe can't tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<ModelTier>,
    #[serde(default)]
    pub by: TierBy,
    #[serde(default)]
    pub reason: TierReason,
    /// What makes it costly to get wrong, as read from it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risk: Vec<RiskTag>,
    /// The engine, model and effort its latest lane started on: what the
    /// card shows. `None` model or effort: the engine's own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<Engine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The person fixed it: escalation leaves it alone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    /// How many times equipe raised it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub escalations: u32,
    /// The cheaper tier the rules would have picked, had they been allowed
    /// to lower it: recorded, never acted on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ModelTier>,
    /// Its next lane starts a fresh conversation with a handoff note,
    /// because its tier changed since the last one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fresh: bool,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// Why work was sent back: what decides whether a stronger model would help.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendBackReason {
    /// Taste, wording, polish: a matter of preference, not of strength.
    Style,
    /// The environment: a device, a simulator, a conflict with main, a flaky
    /// service — a stronger model wouldn't help.
    Environment,
    /// It doesn't do what was asked, or breaks something: a stronger model may
    /// help. What a send-back without a reason, or with one this build
    /// doesn't know, counts as.
    #[default]
    #[serde(other)]
    Correctness,
}

impl SendBackReason {
    pub const ALL: [SendBackReason; 3] = [SendBackReason::Correctness, SendBackReason::Style, SendBackReason::Environment];

    pub fn code(self) -> &'static str {
        match self {
            SendBackReason::Correctness => "correctness",
            SendBackReason::Style => "style",
            SendBackReason::Environment => "environment",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        SendBackReason::ALL.into_iter().find(|r| r.code() == text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tiers_step_up_and_down_within_the_ladder() {
        assert_eq!(ModelTier::Light.up(), ModelTier::Standard);
        assert_eq!(ModelTier::Deep.up(), ModelTier::Max);
        assert_eq!(ModelTier::Max.up(), ModelTier::Max, "the top stays");
        assert_eq!(ModelTier::Light.down(), ModelTier::Light, "the bottom stays");
        assert_eq!(ModelTier::Strong.down(), ModelTier::Standard);
        assert!(ModelTier::Light < ModelTier::Max);
        assert!(ModelTier::ALL.iter().all(|t| ModelTier::parse(t.code()) == Some(*t)));
        assert_eq!(ModelTier::parse(" Deep "), Some(ModelTier::Deep));
        assert_eq!(ModelTier::parse("huge"), None);
    }

    #[test]
    fn whatever_is_missing_or_unknown_reads_as_the_old_behaviour() {
        // A ticket from before tiers has no model at all; one with an empty one runs on its member's model.
        let bare: TicketModel = serde_json::from_value(json!({})).unwrap();
        assert_eq!(bare, TicketModel::default());
        assert_eq!(bare.tier, None);
        assert_eq!(serde_json::to_value(&bare).unwrap(), json!({ "by": "rule", "reason": "code" }));
        // Newer words from a newer daemon don't break an older screen.
        let newer: TicketModel = serde_json::from_value(json!({ "tier": "ultra", "by": "oracle", "reason": "vibes", "risk": ["auth", "lasers"] })).unwrap();
        assert_eq!((newer.tier, newer.by, newer.reason), (Some(ModelTier::Strong), TierBy::Rule, TierReason::Other));
        assert_eq!(newer.risk, [RiskTag::Auth, RiskTag::Other]);
        assert_eq!(serde_json::from_value::<ModelChoice>(json!("turbo")).unwrap(), ModelChoice::QualityFirst);
        assert_eq!(ModelChoice::default(), ModelChoice::QualityFirst);
        assert_eq!(serde_json::from_value::<SendBackReason>(json!("vibes")).unwrap(), SendBackReason::Correctness);
        let full = TicketModel { tier: Some(ModelTier::Deep), by: TierBy::Escalation, reason: TierReason::Escalated, escalations: 1, fresh: true, ..TicketModel::default() };
        assert_eq!(serde_json::from_value::<TicketModel>(serde_json::to_value(&full).unwrap()).unwrap(), full);
    }

    #[test]
    fn choices_and_reasons_parse_as_typed() {
        assert!(ModelChoice::ALL.iter().all(|c| ModelChoice::parse(c.code()) == Some(*c)));
        assert_eq!(ModelChoice::parse("Quality first"), None);
        assert_eq!(ModelChoice::parse("quality-first"), Some(ModelChoice::QualityFirst));
        assert!(SendBackReason::ALL.iter().all(|r| SendBackReason::parse(r.code()) == Some(*r)));
        assert_eq!(SendBackReason::parse("taste"), None);
    }
}
