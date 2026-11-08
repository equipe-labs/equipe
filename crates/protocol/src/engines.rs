//! Two plans, one squad: equipe watches what is left of each engine's plan
//! ([`crate::EnginePlan`]) and moves members between engines — between
//! tickets, or at the end of a turn mid-ticket with the ticket handed over
//! ([`Move::now`]); never mid-turn. How it does is the squad's choice
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
    /// At the end of the member's current turn, even mid-ticket: its ticket
    /// is handed over (same branch and worktree, a brief of what was done and
    /// what is next) and carried on on the other engine. Otherwise a member on
    /// a ticket finishes it where it is and moves before its next one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub now: bool,
}

impl Move {
    /// Only its model or effort changes; its engine stays.
    pub fn in_place(&self) -> bool {
        self.from == self.to
    }
}

/// How much of the conversation before a move a screen is sent with its
/// hand-over ([`crate::Event::HandOver`]): its latest lines, at most this many.
pub const EARLIER_LINES: usize = 600;
/// A hand-over packet is kept to this many bytes at most.
pub const PACKET_MOST: usize = 64 * 1024;

/// A member moved to the other engine: the session it was closed, and one
/// under the same name carries on, opening with the hand-over packet equipe
/// wrote. Its memory, worktree and tickets stay; the conversation before is
/// the journal's, read back with [`crate::Command::HandOver`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandOver {
    pub member: String,
    /// The session that closed.
    pub from_session: crate::SessionId,
    /// The one that carries on.
    pub to_session: crate::SessionId,
    pub from: Engine,
    /// Where it carries on: the other engine — or the same one, when the
    /// other couldn't start it.
    pub to: Engine,
    pub why: Why,
    /// When, in unix seconds.
    pub at: i64,
    /// The ticket it was in the middle of, handed over with it; `None`: it
    /// moved between tickets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<crate::TicketId>,
}

impl HandOver {
    /// The moves session `session` carries on from, newest first: the one
    /// into it, then the one into that, and so on.
    pub fn chain(all: &[HandOver], session: crate::SessionId) -> Vec<&HandOver> {
        let mut out: Vec<&HandOver> = Vec::new();
        let mut at = session;
        while let Some(h) = all.iter().rev().find(|h| h.to_session == at && !out.iter().any(|o| o.to_session == h.to_session)) {
            out.push(h);
            at = h.from_session;
        }
        out
    }
}

/// Why a plan's moves are worth it, as the Usage screen's forecast says:
/// when the plan runs out at the pace of late, against when it starts over,
/// and how much of its week the members moving used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forecast {
    /// When it runs out at this pace (unix seconds); `None`: it lasts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_out_at: Option<i64>,
    /// When the window starts over (unix seconds).
    pub resets_at: i64,
    /// Of the engine's load this week, what the members moving used, in
    /// basis points (10 000 = 100 %); `None`: not known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_bp: Option<u32>,
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
    /// The forecast behind it, when the plan's pace is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forecast: Option<Forecast>,
}

impl Proposal {
    /// The engine members move to — the first move's.
    pub fn to(&self) -> Option<Engine> {
        self.moves.iter().find(|m| !m.in_place()).map(|m| m.to)
    }

    /// Whether any of its moves still waits for its member.
    pub fn waiting(&self) -> bool {
        self.moves.iter().any(|m| !m.done)
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
            moves: vec![Move { member: "backend".into(), role: "backend".into(), from: Engine::Claude, to: Engine::Codex, model: None, effort: None, was_model: Some("opus".into()), done: false, now: false }],
            applied: false,
            proposed_at: 1_760_000_000,
            forecast: None,
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
        // A move now and its forecast, as a newer daemon sends them; an older screen reads neither.
        let mut now = shift();
        now.moves[0].now = true;
        now.forecast = Some(Forecast { runs_out_at: Some(1_760_100_000), resets_at: 1_760_300_000, load_bp: Some(6_400) });
        let wire = serde_json::to_value(&now).unwrap();
        assert_eq!(wire["moves"][0]["now"], json!(true));
        assert_eq!(wire["forecast"], json!({ "runs_out_at": 1_760_100_000i64, "resets_at": 1_760_300_000i64, "load_bp": 6_400 }));
        assert_eq!(serde_json::from_value::<Proposal>(wire).unwrap(), now);
        assert!(now.waiting());
    }

    fn moved(from: u64, to: u64, engine: (Engine, Engine)) -> HandOver {
        HandOver { member: "backend".into(), from_session: from, to_session: to, from: engine.0, to: engine.1, why: Why::Person, at: 1_760_000_000 + to as i64, ticket: None }
    }

    #[test]
    fn a_member_carries_on_from_every_session_before_it_newest_first_and_says_so_on_the_wire() {
        let all = vec![moved(1, 2, (Engine::Claude, Engine::Codex)), moved(7, 8, (Engine::Claude, Engine::Codex)), moved(2, 3, (Engine::Codex, Engine::Claude))];
        let chain: Vec<(u64, u64)> = HandOver::chain(&all, 3).iter().map(|h| (h.from_session, h.to_session)).collect();
        assert_eq!(chain, [(2, 3), (1, 2)], "Claude to Codex and back: both moves, the latest first");
        assert!(HandOver::chain(&all, 1).is_empty(), "where it began: nothing before");
        assert_eq!(HandOver::chain(&all, 8).len(), 1, "another member's move is its own");
        // A record that loops is read once, not forever.
        let looped = vec![moved(1, 2, (Engine::Claude, Engine::Codex)), moved(2, 1, (Engine::Codex, Engine::Claude))];
        assert_eq!(HandOver::chain(&looped, 2).len(), 2);
        let mid = HandOver { ticket: Some(42), why: Why::Exhausted, ..moved(1, 2, (Engine::Claude, Engine::Codex)) };
        let wire = serde_json::to_value(&mid).unwrap();
        assert_eq!(wire, json!({ "member": "backend", "from_session": 1, "to_session": 2, "from": "claude", "to": "codex", "why": "exhausted", "at": 1_760_000_002i64, "ticket": 42 }));
        assert_eq!(serde_json::from_value::<HandOver>(wire).unwrap(), mid);
        assert_eq!(serde_json::to_value(moved(1, 2, (Engine::Claude, Engine::Codex))).unwrap().get("ticket"), None, "between tickets: no ticket on the wire");
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
