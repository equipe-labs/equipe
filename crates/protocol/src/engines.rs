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

/// Which engine reviews a designer's work — `equipe design critique` and
/// `second-look`. What matters is a reviewer that saw none of the work being
/// made; the other engine adds a second model's eyes when it can take it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reviewer {
    /// The other engine when it is installed, signed in and its plan has
    /// room; else the member's own engine, in a fresh session.
    #[default]
    Auto,
    /// Always the member's own engine, in a fresh session.
    Same,
    /// Always the other engine — the member's own only when the other can't run.
    Other,
}

impl Reviewer {
    pub const ALL: [Reviewer; 3] = [Reviewer::Auto, Reviewer::Same, Reviewer::Other];

    pub fn code(self) -> &'static str {
        match self {
            Reviewer::Auto => "auto",
            Reviewer::Same => "same",
            Reviewer::Other => "other",
        }
    }

    /// `auto`, `same`, `other` — as typed in a command or kept in a journal.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "auto" | "default" => Some(Reviewer::Auto),
            "same" | "same_engine" | "same_as_member" | "own" => Some(Reviewer::Same),
            "other" | "other_engine" | "cross" => Some(Reviewer::Other),
            _ => None,
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Reviewer::default()
    }
}

/// Who reviewed a designer's work, and why that engine: kept with the
/// critique and on the second look's verdict, so the person can see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviewed {
    pub engine: Engine,
    /// The engine the designer works on, when known: the same one means a
    /// fresh session of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member: Option<Engine>,
    pub why: ReviewWhy,
}

/// Why the reviewer is the engine it is. The reasons about the other engine
/// speak of the one the member isn't on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewWhy {
    /// Named on the command (`--engine`).
    Named,
    /// The squad's setting ([`Reviewer::Same`] or [`Reviewer::Other`]).
    Setting,
    /// The other engine is signed in with room: its week this used, when read.
    Room {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        used: Option<u8>,
    },
    /// The other engine isn't installed.
    NotInstalled,
    /// The other engine isn't signed in.
    SignedOut,
    /// The other engine's limit is used up.
    UsedUp { window: Window },
    /// The other engine's limit is nearly used.
    NearLimit { window: Window, used: u8 },
    /// At its pace, the other engine's limit runs out before it starts over.
    RunsOut { window: Window, used: u8 },
    /// The other engine was tried and couldn't run (signed out, out of usage).
    Failed,
}

/// The engine `engine` isn't.
fn across(engine: Engine) -> Engine {
    match engine {
        Engine::Claude => Engine::Codex,
        Engine::Codex => Engine::Claude,
    }
}

impl Reviewed {
    /// The member's own engine, in a fresh session.
    pub fn fresh(&self) -> bool {
        self.member == Some(self.engine)
    }

    /// The engine the reasons speak of: the one the member isn't on.
    pub fn other(&self) -> Engine {
        across(self.member.unwrap_or_else(|| across(self.engine)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn who_reviewed_and_why_travel_as_plain_words() {
        for r in Reviewer::ALL {
            assert_eq!(Reviewer::parse(r.code()), Some(r));
        }
        assert_eq!(Reviewer::parse("Same as member"), Some(Reviewer::Same));
        assert_eq!(Reviewer::parse("other-engine"), Some(Reviewer::Other));
        assert_eq!(Reviewer::parse("codex"), None);
        assert!(Reviewer::default().is_default() && Reviewer::default() == Reviewer::Auto);
        let near = Reviewed { engine: Engine::Claude, member: Some(Engine::Claude), why: ReviewWhy::NearLimit { window: Window::Weekly, used: 93 } };
        let wire = serde_json::to_value(near).unwrap();
        assert_eq!(wire, json!({ "engine": "claude", "member": "claude", "why": { "kind": "near_limit", "window": "weekly", "used": 93 } }));
        assert_eq!(serde_json::from_value::<Reviewed>(wire).unwrap(), near);
        assert!(near.fresh() && near.other() == Engine::Codex);
        let room = Reviewed { engine: Engine::Codex, member: Some(Engine::Claude), why: ReviewWhy::Room { used: None } };
        assert!(!room.fresh() && room.other() == Engine::Codex);
        assert_eq!(serde_json::to_value(room).unwrap()["why"], json!({ "kind": "room" }));
        let named = Reviewed { engine: Engine::Claude, member: None, why: ReviewWhy::Named };
        assert_eq!(named.other(), Engine::Claude, "nobody's engine known: the reviewer is taken for the other one");
    }

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
