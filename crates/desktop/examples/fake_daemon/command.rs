//! `FAKE_COMMAND=working|busy|nothing|asks|stuck|stuck_nothing`: a member's
//! turn ends its steps on a command that runs on — the one a PM once sat on
//! for 28 minutes — and the daemon's look at what runs for it comes every
//! two seconds (`Event::CommandWatch`), as that state: `sqlite3` at next to
//! no CPU, a build at six cores, nothing under the engine, a request waiting
//! on the person, or stuck — with a process, or with nothing. "Stop this
//! command" (`stop_command`) ends it as the daemon would: the step fails,
//! the member says so and its turn ends.

use std::time::Duration;

use equipe_core::protocol::watch::{CommandWatch, WatchState, WatchedProcess};
use equipe_core::protocol::{Engine, Event, SessionId, ToolOutcome, TurnOutcome};
use tokio::sync::broadcast;

const COMMAND: &str = "cd ~/.equipe/squads/atinum-squad; sqlite3 squad.db \"select count(*), datetime(min(at)/1000,'unixepoch','+9 hours'), datetime(max(at)/1000,'unixepoch','+9 hours') from events\"";

pub fn mode() -> Option<String> {
    std::env::var("FAKE_COMMAND").ok().filter(|m| !m.is_empty())
}

/// The look, as the daemon would send it `secs` into the command.
fn look(mode: &str, secs: u64) -> CommandWatch {
    let sqlite = |cpu_percent| Some(WatchedProcess { name: "sqlite3".into(), cpu_percent, secs: 27 * 60 + secs });
    let (process, state) = match mode {
        "busy" => (Some(WatchedProcess { name: "cargo".into(), cpu_percent: 640, secs: 12 * 60 + secs }), WatchState::Busy),
        "nothing" => (None, WatchState::Nothing),
        "asks" => (None, WatchState::AwaitingYou),
        "stuck" => (sqlite(0), WatchState::Stuck { idle_secs: 27 * 60 + secs }),
        "stuck_nothing" => (None, WatchState::Stuck { idle_secs: 26 * 60 + secs }),
        _ => (sqlite(1), WatchState::Working),
    };
    CommandWatch { engine: Engine::Claude, process, quiet_secs: 28 * 60 + 6 + secs, state }
}

/// The command step, then the looks at it, until the turn is stopped.
pub async fn run(session: SessionId, events: &broadcast::Sender<Event>, mode: &str) {
    let label = Some("Count the squad's events and their dates".to_owned());
    let _ = events.send(Event::ToolUse { session, tool: "Bash".into(), target: Some(COMMAND.into()), label, background: false });
    let mut secs = 0;
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        secs += 2;
        let _ = events.send(Event::CommandWatch { session, watch: Some(look(mode, secs)) });
    }
}

/// "Stop this command": the step fails, as the engine hears it, and the turn goes on to its end.
pub async fn stopped(session: SessionId, events: broadcast::Sender<Event>) {
    tokio::time::sleep(Duration::from_millis(600)).await;
    let _ = events.send(Event::CommandWatch { session, watch: None });
    let outcome = ToolOutcome { ok: false, exit: Some(143), ms: Some(1_686_000), output: "Terminated".into(), ..Default::default() };
    let _ = events.send(Event::ToolResult { session, tool: "Bash".into(), target: Some(COMMAND.into()), outcome });
    for chunk in ["The query was stopped. ", "I'll read the count from the journal instead."] {
        let _ = events.send(Event::TextDelta { session, text: chunk.into() });
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let _ = events.send(Event::TurnEnded { session, outcome: TurnOutcome::Completed, usage: None });
}
