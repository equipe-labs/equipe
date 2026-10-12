//! The contract between equipe's brain (`equipe-core`) and its screens.
//!
//! Screens — the command line, the terminal UI, the desktop app — send
//! [`Command`]s and render [`Event`]s. They never call an engine and hold no
//! state beyond what they show; every decision happens in the core.
//!
//! Inside one process these travel as Rust values over a channel. Every message
//! also serializes to JSON, so the same contract can cross a socket later
//! (`equipe serve`) as JSON-RPC — the shape ACP, MCP and LSP already use.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One conversation with one agent. Chosen by the screen that opens it.
pub type SessionId = u64;

/// The engines equipe can run a turn on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    /// The user's ChatGPT plan, signed in by equipe.
    Codex,
    /// The user's installed Claude Code, signed in by itself.
    Claude,
}

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Engine::Codex => "codex",
            Engine::Claude => "claude code",
        }
    }
}

/// What a session may do to files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// Answer and read; never write.
    ReadOnly,
    /// Edit files — on a new branch, in a worktree of its own.
    EditFiles,
}

/// How to open a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSpec {
    /// `None`: the first engine that is ready.
    pub engine: Option<Engine>,
    /// `None`: the engine's own default.
    pub model: Option<String>,
    pub access: Access,
    /// Where the agent works. With [`Access::EditFiles`], the repository the
    /// session's branch is cut from.
    pub cwd: PathBuf,
}

/// Screen → core: "do this".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Open a session. Answered by [`Event::SessionStarted`] or [`Event::Error`].
    Start { session: SessionId, spec: SessionSpec },
    /// Send the user's message. One turn runs at a time per session; a message
    /// sent while a turn runs is refused with an error.
    Submit { session: SessionId, text: String },
    /// Stop the turn in progress (Esc). Answered by a `TurnEnded` with
    /// [`TurnOutcome::Interrupted`].
    Interrupt { session: SessionId },
    /// End the session. With [`Access::EditFiles`] this lands the branch first.
    /// Answered by [`Event::SessionClosed`].
    Close { session: SessionId },
}

/// Token counts for one turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    /// The engine stopped early but left usable output.
    Incomplete,
    /// The user stopped it.
    Interrupted,
    /// It failed; an [`Event::Error`] says why.
    Failed,
}

/// The isolated place an editing session works in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub branch: String,
    /// The commit the branch was cut from.
    pub base: String,
    pub path: PathBuf,
}

/// Core → screen: "this happened".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    SessionStarted {
        session: SessionId,
        engine: Engine,
        /// The model, once known. Codex resolves it at start; Claude Code
        /// reports it with the first turn.
        model: Option<String>,
        /// Set for [`Access::EditFiles`] sessions.
        workspace: Option<Workspace>,
    },
    /// A fragment of the agent's answer.
    TextDelta { session: SessionId, text: String },
    /// The agent is using a tool, e.g. `Edit` on a file.
    ToolUse {
        session: SessionId,
        tool: String,
        target: Option<String>,
    },
    /// The engine named its model (Claude Code does on its first turn).
    ModelKnown { session: SessionId, model: String },
    TurnEnded {
        session: SessionId,
        outcome: TurnOutcome,
        usage: Option<Usage>,
    },
    /// An editing session closed with changes, committed to its branch.
    BranchLanded {
        session: SessionId,
        branch: String,
        base: String,
        /// `git diff --stat` of the branch.
        stat: String,
    },
    /// An editing session closed without changing anything; no branch was kept.
    NothingChanged { session: SessionId },
    SessionClosed { session: SessionId },
    /// Something went wrong. `next` is the command that fixes it, when there is one.
    Error {
        session: Option<SessionId>,
        message: String,
        next: Option<String>,
    },
}

impl Event {
    /// The session this event belongs to, if any.
    pub fn session(&self) -> Option<SessionId> {
        match self {
            Event::SessionStarted { session, .. }
            | Event::TextDelta { session, .. }
            | Event::ToolUse { session, .. }
            | Event::ModelKnown { session, .. }
            | Event::TurnEnded { session, .. }
            | Event::BranchLanded { session, .. }
            | Event::NothingChanged { session }
            | Event::SessionClosed { session } => Some(*session),
            Event::Error { session, .. } => *session,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn commands_serialize_with_a_type_tag() {
        let cmd = Command::Submit { session: 1, text: "hi".into() };
        assert_eq!(
            serde_json::to_value(&cmd).unwrap(),
            json!({ "type": "submit", "session": 1, "text": "hi" })
        );
    }

    #[test]
    fn events_round_trip_through_json() {
        // The contract must survive a socket, not just a channel.
        let events = [
            Event::SessionStarted {
                session: 1,
                engine: Engine::Claude,
                model: None,
                workspace: Some(Workspace { branch: "equipe/try-1".into(), base: "abc".into(), path: "/w".into() }),
            },
            Event::TextDelta { session: 1, text: "안녕".into() },
            Event::ToolUse { session: 1, tool: "Edit".into(), target: Some("a.rs".into()) },
            Event::TurnEnded {
                session: 1,
                outcome: TurnOutcome::Interrupted,
                usage: Some(Usage { input_tokens: Some(3), output_tokens: None }),
            },
            Event::Error { session: None, message: "no engine".into(), next: Some("equipe login".into()) },
        ];
        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event, "{json}");
        }
    }

    #[test]
    fn a_session_spec_round_trips() {
        let spec = SessionSpec { engine: None, model: Some("haiku".into()), access: Access::EditFiles, cwd: "/repo".into() };
        let cmd = Command::Start { session: 7, spec };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"edit_files\""), "{json}");
        assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), cmd);
    }

    #[test]
    fn every_session_event_names_its_session() {
        assert_eq!(Event::NothingChanged { session: 4 }.session(), Some(4));
        assert_eq!(Event::Error { session: None, message: String::new(), next: None }.session(), None);
    }
}
