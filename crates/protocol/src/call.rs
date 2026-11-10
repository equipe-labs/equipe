//! A call — the voice radio (`docs/remote-protocol.md`, "A call"): the
//! person talks, the daemon splits each utterance by who it is for and sends
//! each part on as an ordinary submit. Nothing said in a call decides anything.

use serde::{Deserialize, Serialize};

use crate::SessionId;

/// How an utterance reached the daemon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallVia {
    #[default]
    Voice,
    Typed,
}

/// What kind of line the phone says aloud.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SayKind {
    /// After work was sent.
    Reply,
    /// Asked once when it couldn't tell.
    Ask,
    /// As the call ends.
    Bye,
}

/// Who read the utterance for its routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteBy {
    /// The fast model.
    Llm,
    /// The model timed out or failed; words matched.
    Rules,
    /// Nothing matched: the PM.
    Fallback,
}

/// Why a call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallEnded {
    /// They said they were done.
    Person,
    /// Silence.
    Idle,
    /// `call_end`.
    Hangup,
}

/// Who a route is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Member,
    /// Everyone (`about` null).
    Squad,
    Pm,
}

/// A route's recipient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallTarget {
    pub kind: TargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// One part of an utterance and who it went to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallRoute {
    pub to: CallTarget,
    pub text: String,
}

/// Something sent during a call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dispatched {
    pub to: CallTarget,
    pub text: String,
    /// Unix ms.
    pub at: i64,
}
