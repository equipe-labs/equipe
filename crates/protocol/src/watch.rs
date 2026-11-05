//! What a member's command is really doing while its step reads "Running".
//!
//! The daemon runs on the member's machine and knows the engine process it
//! started for the turn (Claude Code, or Codex's app server). While a command
//! of the member's runs, it looks at the processes under that engine every
//! few seconds — off its core loop, one `ps` for every member at once — and
//! tells the screens what runs there for the command, how much CPU it used
//! since the last look, and how long the engine has been silent
//! ([`crate::Event::CommandWatch`]). A command that uses the CPU is at work,
//! however long it takes; one that has used next to none for minutes while
//! its engine says nothing, or a step with nothing running under the engine
//! at all, looks stuck — and the person can stop just that command
//! ([`crate::Command::StopCommand`]): the engine hears it failed and goes on
//! with its turn.

use serde::{Deserialize, Serialize};

use crate::Engine;

/// A look at a member's command under way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandWatch {
    /// The engine the command runs under.
    pub engine: Engine,
    /// The process shown for the command: its busiest, else its deepest one
    /// that isn't a shell around it. `None`: nothing runs for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<WatchedProcess>,
    /// Seconds since the engine last said anything — text, a step, a thought.
    pub quiet_secs: u64,
    pub state: WatchState,
}

/// One process of the command's, as a person reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchedProcess {
    /// The program's name: `sqlite3`, `cargo`.
    pub name: String,
    /// The CPU the whole command used since the last look, in percent of one
    /// core (a build on eight cores reads 800).
    pub cpu_percent: u32,
    /// How long this process has run, in seconds.
    pub secs: u64,
}

/// How the command looks, from what runs and what the engine says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WatchState {
    /// Running and using the CPU — or not long quiet: at work.
    Working,
    /// Using a good deal of CPU: busy, however long it takes.
    Busy,
    /// Nothing runs under the engine for the command (it hasn't started, or
    /// it ended and the engine hasn't gone on), not long enough to say more.
    Nothing,
    /// The engine waits for the person's answer to a request of the member's.
    AwaitingYou,
    /// Looks stuck: the command used next to no CPU for `idle_secs` — or
    /// nothing has run for it that long — and the engine said nothing.
    Stuck { idle_secs: u64 },
}

impl WatchState {
    pub fn is_stuck(self) -> bool {
        matches!(self, WatchState::Stuck { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_travels_as_json_and_back() {
        let watch = CommandWatch {
            engine: Engine::Claude,
            process: Some(WatchedProcess { name: "sqlite3".into(), cpu_percent: 0, secs: 1620 }),
            quiet_secs: 1686,
            state: WatchState::Stuck { idle_secs: 1600 },
        };
        let json = serde_json::to_string(&watch).unwrap();
        assert!(json.contains(r#""state":{"kind":"stuck","idle_secs":1600}"#), "{json}");
        assert_eq!(serde_json::from_str::<CommandWatch>(&json).unwrap(), watch);
        let nothing = CommandWatch { engine: Engine::Codex, process: None, quiet_secs: 4, state: WatchState::Nothing };
        assert!(!serde_json::to_string(&nothing).unwrap().contains("process"), "nothing running says no process");
    }
}
