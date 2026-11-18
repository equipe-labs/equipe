//! `equipe squad autonomy`: how far a squad's members go without you
//! (`equipe_core::protocol::autonomy`) — show it, set a kind's level, the
//! decide-and-tell window, list and revoke standing allowances.
//!
//! With the daemon running, changes go through it as your own commands (so
//! every screen hears of them, and a member's shell can't send them); without
//! it, they are written to the squad's journal, which the daemon reads afresh.

use std::path::Path;

use equipe_core::autonomy::Store;
use equipe_core::protocol::autonomy::{Kind, Level};
use equipe_core::protocol::{Command, Event};
use equipe_core::t;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn core_word(key: &str) -> String {
    equipe_core::i18n::core(crate::i18n::lang()).get(key).to_owned()
}

fn kind_word(kind: Kind) -> String {
    core_word(&kind.words_key())
}

fn level_word(level: Level) -> String {
    core_word(&format!("autonomy.level.{}", level.code()))
}

/// What a change is, read from the words after `autonomy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Show,
    Set(Kind, Level),
    Window(u32),
    Revoke(u64),
}

/// Read `show`, `set <kind> <level>`, `window <minutes>`, `revoke <id>`.
pub fn parse(words: &[&str]) -> Result<Action, String> {
    let usage = || t!("autonomy.usage").to_owned();
    Ok(match words {
        [] | ["show"] | ["allowances"] => Action::Show,
        ["set", kind, level] => {
            let kind = Kind::parse(kind).ok_or_else(usage)?;
            let level = Level::parse(level).ok_or_else(usage)?;
            if !Kind::SET.contains(&kind) && level != Level::Ask {
                return Err(t!("autonomy.floor", kind = kind_word(kind)));
            }
            Action::Set(kind, level)
        }
        ["window", minutes] => Action::Window(minutes.trim_end_matches("min").parse::<u32>().map_err(|_| usage())?.min(24 * 60)),
        ["revoke", id] => Action::Revoke(id.trim_start_matches('#').parse().map_err(|_| usage())?),
        _ => return Err(usage()),
    })
}

/// The lines `show` prints for `store`.
pub fn lines(squad: &str, store: &Store, pm_accepts: bool, now: u64) -> Vec<String> {
    let state = store.state(pm_accepts);
    let mut out = vec![t!("autonomy.title", squad = squad)];
    for kind in Kind::SET {
        out.push(format!("  {:<16} {}  ({})", kind.code(), level_word(state.policy.level(kind)), kind_word(kind)));
    }
    out.push(t!("autonomy.window", minutes = state.policy.window_minutes));
    out.push(t!("autonomy.always_asks", kinds = Kind::FLOOR.iter().map(|k| kind_word(*k)).collect::<Vec<_>>().join(", ")));
    out.push(String::new());
    out.push(t!("autonomy.allowances").to_owned());
    if state.allowances.is_empty() {
        out.push(format!("  {}", t!("autonomy.none")));
    }
    for a in &state.allowances {
        let stands = match a.revoked_at {
            Some(_) => t!("autonomy.revoked").to_owned(),
            None if a.expires_at <= now => t!("autonomy.expired").to_owned(),
            None => t!("autonomy.days_left", days = a.expires_at.saturating_sub(now).div_ceil(86_400)),
        };
        out.push(format!("  #{} {} — {stands} · {}", a.id, a.scope.words, t!("autonomy.used", uses = a.uses)));
    }
    if !state.uses.is_empty() {
        out.push(String::new());
        out.push(t!("autonomy.latest_uses").to_owned());
        for u in state.uses.iter().take(10) {
            out.push(format!("  #{} {}: {}", u.allowance, u.member, u.what));
        }
    }
    if !state.picks.is_empty() {
        out.push(String::new());
        out.push(t!("autonomy.picks").to_owned());
        for p in state.picks.iter().take(10) {
            out.push(format!("  {}: {} → {}", p.member, p.question, p.picked));
        }
    }
    out
}

/// `equipe squad autonomy …` for `squad`.
pub async fn command(home: &Path, socket: &Path, squad: &str, words: &[&str]) -> Result<Vec<String>, String> {
    let action = parse(words)?;
    let journal = equipe_core::journal::Journal::for_squad(home, squad).map_err(|e| e.to_string())?;
    if action == Action::Show {
        return Ok(lines(squad, &journal.autonomy(), journal.pm_accepts(), now()));
    }
    let command = match action {
        Action::Set(kind, level) => Command::SetAutonomy { squad: squad.to_owned(), kind, level },
        Action::Window(minutes) => Command::SetAutonomyWindow { squad: squad.to_owned(), minutes },
        Action::Revoke(id) => {
            if !journal.autonomy().allowances.iter().any(|a| a.id == id && a.live(now())) {
                return Err(t!("autonomy.no_allowance", id = id));
            }
            Command::RevokeAllowance { squad: squad.to_owned(), id }
        }
        Action::Show => unreachable!("shown above"),
    };
    drop(journal);
    if equipe_core::gateway::is_running(socket).await {
        through_daemon(socket, squad, command).await?;
    } else {
        apply(home, squad, &command)?;
    }
    let journal = equipe_core::journal::Journal::for_squad(home, squad).map_err(|e| e.to_string())?;
    let mut out = vec![t!("autonomy.done").to_owned()];
    out.extend(lines(squad, &journal.autonomy(), journal.pm_accepts(), now()));
    Ok(out)
}

/// The change written to the squad's journal, as the daemon would make it.
fn apply(home: &Path, squad: &str, command: &Command) -> Result<(), String> {
    let journal = equipe_core::journal::Journal::for_squad(home, squad).map_err(|e| e.to_string())?;
    let mut store = journal.autonomy();
    match command {
        Command::SetAutonomy { kind: Kind::Merge, level, .. } => {
            journal.set_pm_accepts(*level != Level::Ask).map_err(|e| e.to_string())?;
            store.policy.merge = *level;
        }
        Command::SetAutonomy { kind, level, .. } => store.policy.set(*kind, *level).map_err(|_| t!("autonomy.floor", kind = kind_word(*kind)))?,
        Command::SetAutonomyWindow { minutes, .. } => store.policy.window_minutes = *minutes,
        Command::RevokeAllowance { id, .. } => {
            store.revoke(*id, now());
        }
        _ => {}
    }
    journal.set_autonomy(&store).map_err(|e| e.to_string())
}

/// The change sent to the running daemon as your own, answered by the squad's autonomy as it is now.
async fn through_daemon(socket: &Path, squad: &str, command: Command) -> Result<(), String> {
    let (mut client, _) = equipe_core::gateway::Client::connect(socket, &crate::daemon::build_id(), squad).await.map_err(|e| e.to_string())?;
    client.send(command);
    let answer = async {
        while let Some(event) = client.next_event().await {
            match event {
                Event::AutonomyChanged { squad: s, .. } if s == squad => return Ok(()),
                Event::Error { message, .. } => return Err(message),
                _ => {}
            }
        }
        Err(t!("autonomy.no_answer").to_owned())
    };
    tokio::time::timeout(std::time::Duration::from_secs(15), answer).await.map_err(|_| t!("autonomy.no_answer").to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_after_autonomy_are_read_and_the_floor_refused() {
        assert_eq!(parse(&[]), Ok(Action::Show));
        assert_eq!(parse(&["set", "design", "just_do"]), Ok(Action::Set(Kind::Design, Level::JustDo)));
        assert_eq!(parse(&["set", "production", "decide-and-tell"]), Ok(Action::Set(Kind::ProductionData, Level::Tell)));
        assert_eq!(parse(&["window", "45"]), Ok(Action::Window(45)));
        assert_eq!(parse(&["revoke", "#3"]), Ok(Action::Revoke(3)));
        assert!(parse(&["set", "money", "just_do"]).is_err(), "the floor always asks");
        assert_eq!(parse(&["set", "money", "ask"]), Ok(Action::Set(Kind::Money, Level::Ask)));
        assert!(parse(&["set", "design"]).is_err());
        assert!(parse(&["window", "soon"]).is_err());
    }

    #[test]
    fn show_lists_the_levels_the_floor_and_the_allowances() {
        let mut store = Store::default();
        let scope = equipe_core::protocol::autonomy::Scope { words: "Test records marked [test] in the ERP".into(), system: Some("ERP".into()), marker: Some("[test]".into()), ops: vec![equipe_core::protocol::autonomy::Op::Create], ..Default::default() };
        store.grant(Kind::ProductionData, scope, "qa", 7, 0).unwrap();
        let shown = lines("acme", &store, true, 86_400).join("\n");
        assert!(shown.contains("design") && shown.contains("production_data"), "{shown}");
        assert!(shown.contains("#1 Test records marked [test] in the ERP"), "{shown}");
        assert!(shown.contains("6"), "days left: {shown}");
    }
}
