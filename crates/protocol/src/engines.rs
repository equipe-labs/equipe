//! Two plans, one squad: equipe watches what is left of each engine's plan
//! ([`crate::EnginePlan`]) and moves members between engines at their ticket
//! boundaries — never mid-turn. How it does is the squad's choice
//! ([`Switching`]): only showing, proposing on a card the person answers, or
//! moving on its own and saying so, with one click to undo. A move is never
//! paid for: equipe doesn't spend credits or overage on its own.
//!
//! The same card proposes the squad's setup once, for a person signed in to
//! both engines whose team runs on one: who writes, who reviews, on which
//! engine and model.

use serde::{Deserialize, Serialize};

use crate::Engine;

/// How a squad moves its members between engines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Switching {
    /// Show the plans' usage; never propose, never move.
    Off,
    /// Propose moves on a card; only the person's click makes them.
    #[default]
    Suggest,
    /// Move at ticket boundaries on its own, and tell the person, who may undo.
    Auto,
}

impl Switching {
    pub const ALL: [Switching; 3] = [Switching::Off, Switching::Suggest, Switching::Auto];

    pub fn code(self) -> &'static str {
        match self {
            Switching::Off => "off",
            Switching::Suggest => "suggest",
            Switching::Auto => "auto",
        }
    }

    /// `off`, `suggest`, `auto` — as typed in a command.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "off" | "none" => Some(Switching::Off),
            "suggest" | "ask" | "propose" => Some(Switching::Suggest),
            "auto" | "on" => Some(Switching::Auto),
            _ => None,
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Switching::default()
    }
}

/// Which of a plan's limits a proposal is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Window {
    FiveHour,
    Weekly,
}

/// Why members move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// The engine's plan runs low: the implementation roles move first.
    Shift,
    /// It is nearly gone: every role that can move does, the PM last.
    Drain,
    /// It is gone until a reset too far off to wait for: the work is handed over.
    Exhausted,
    /// Both plans run low: a lighter model or less thinking on the same engine.
    StepDown,
    /// The original engine recovered: members go back.
    Return,
    /// The squad's recommended setup for both engines.
    Setup,
    /// The person moved members themselves (`/engine`, the member's menu).
    Person,
}

/// One member's move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Move {
    pub member: String,
    pub role: String,
    pub from: Engine,
    pub to: Engine,
    /// The model it runs on after; `None`: what the engine recommends for its role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// What it runs on now, for the card's "now → after".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub was_model: Option<String>,
    /// Made: the member runs as it says now.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
}

impl Move {
    /// Only its model or effort changes; its engine stays.
    pub fn in_place(&self) -> bool {
        self.from == self.to
    }
}

/// Moves equipe proposes — or, in [`Switching::Auto`], made and tells of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: u64,
    pub why: Why,
    /// The engine whose plan it is about; `None` for the setup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<Engine>,
    /// The limit that set it off, how much of it is used, and when it starts over (unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Window>,
    #[serde(default)]
    pub used_percent: u8,
    #[serde(default)]
    pub resets_at: i64,
    pub moves: Vec<Move>,
    /// Made on its own ([`Switching::Auto`]): the card tells, and offers to undo.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub applied: bool,
    /// When, in unix seconds.
    #[serde(default)]
    pub proposed_at: u64,
}

impl Proposal {
    /// The engine members move to — the first move's.
    pub fn to(&self) -> Option<Engine> {
        self.moves.iter().find(|m| !m.in_place()).map(|m| m.to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shift() -> Proposal {
        Proposal {
            id: 9,
            why: Why::Shift,
            engine: Some(Engine::Claude),
            window: Some(Window::FiveHour),
            used_percent: 86,
            resets_at: 1_760_007_200,
            moves: vec![Move { member: "backend".into(), role: "backend".into(), from: Engine::Claude, to: Engine::Codex, model: None, effort: None, was_model: Some("opus".into()), done: false }],
            applied: false,
            proposed_at: 1_760_000_000,
        }
    }

    #[test]
    fn a_proposal_says_what_it_has_and_nothing_more_on_the_wire() {
        let wire = serde_json::to_value(shift()).unwrap();
        assert_eq!(
            wire,
            json!({ "id": 9, "why": "shift", "engine": "claude", "window": "five_hour", "used_percent": 86, "resets_at": 1_760_007_200i64,
                    "moves": [{ "member": "backend", "role": "backend", "from": "claude", "to": "codex", "was_model": "opus" }], "proposed_at": 1_760_000_000u64 })
        );
        assert_eq!(serde_json::from_value::<Proposal>(wire).unwrap(), shift());
        let setup: Proposal = serde_json::from_value(json!({ "id": 1, "why": "setup", "moves": [] })).unwrap();
        assert_eq!((setup.engine, setup.window, setup.applied, setup.to()), (None, None, false, None));
        assert_eq!(shift().to(), Some(Engine::Codex));
    }

    #[test]
    fn the_squads_choice_reads_as_people_type_it_and_suggests_by_default() {
        assert_eq!(Switching::default(), Switching::Suggest);
        assert_eq!(Switching::parse(" Auto "), Some(Switching::Auto));
        assert_eq!(Switching::parse("off"), Some(Switching::Off));
        assert_eq!(Switching::parse("sometimes"), None);
        assert!(Switching::ALL.iter().all(|s| Switching::parse(s.code()) == Some(*s)));
        assert_eq!(serde_json::to_value(Switching::Auto).unwrap(), json!("auto"));
    }
}
