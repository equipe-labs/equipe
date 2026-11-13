//! `equipe context-config`: the three ways equipe keeps what members read
//! short — fresh sessions for conversations whose context grew long, caps on
//! very long tool answers, and a look before work starts on tickets that may
//! be obsolete. All on unless turned off here, per squad; the daemon reads the
//! setting live, so a change holds from the next turn on.

use clap::Args;
use equipe_core::runtime::{NATIVE_DESK_TOKENS, NATIVE_TICKET_TOKENS, ROTATE_DESK_TOKENS, ROTATE_TICKET_TOKENS};
use equipe_core::t;
use equipe_core::tool_cap::MAX_TOOL_TOKENS;

#[derive(Args)]
pub struct ContextConfigArgs {
    /// The squad (default: the one this folder is in)
    #[arg(long)]
    pub squad: Option<String>,
    /// Long conversations go on in a fresh session, opened with a summary packet: on or off
    #[arg(long)]
    pub rotation: Option<String>,
    /// Very long tool answers are cut, with what to call to see the rest: on or off
    #[arg(long)]
    pub tool_caps: Option<String>,
    /// A ticket whose blocker was cancelled, or that repeats an open one, waits for the PM before it starts: on or off
    #[arg(long)]
    pub obsolete_check: Option<String>,
    /// Ask the engines' own auto-compaction to start at the budgets, as a backstop to rotation: on or off
    #[arg(long)]
    pub native_compact: Option<String>,
    /// The context window members run on: standard (the engine compacts early) or large (1M, where the account has it); a ticket can ask for large with a `context: large` line in its body
    #[arg(long)]
    pub window: Option<String>,
}

/// `on` or `off`.
fn switch(word: &Option<String>) -> Result<Option<bool>, String> {
    match word.as_deref().map(str::trim) {
        None => Ok(None),
        Some(w) if w.eq_ignore_ascii_case("on") => Ok(Some(true)),
        Some(w) if w.eq_ignore_ascii_case("off") => Ok(Some(false)),
        Some(_) => Err(t!("context.config.bad_switch").to_owned()),
    }
}

fn word(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

pub fn command(a: ContextConfigArgs) -> Result<(), String> {
    let home = equipe_providers::credentials::equipe_home();
    let cwd = std::env::current_dir().unwrap_or_default();
    let squad = crate::home::squad_for(&home, &cwd, a.squad.clone()).ok_or_else(|| t!("design.qa.no_squad").to_owned())?;
    let journal = equipe_core::journal::Journal::for_squad(&home, &squad).map_err(|e| e.to_string())?;
    let (rotation, caps, obsolete, native) = (switch(&a.rotation)?, switch(&a.tool_caps)?, switch(&a.obsolete_check)?, switch(&a.native_compact)?);
    let window = match a.window.as_deref().map(str::trim) {
        None => None,
        Some(w) if w.eq_ignore_ascii_case("standard") => Some(false),
        Some(w) if w.eq_ignore_ascii_case("large") => Some(true),
        Some(_) => return Err(t!("context.config.bad_window").to_owned()),
    };
    fn e(err: impl std::fmt::Display) -> String {
        err.to_string()
    }
    if let Some(on) = rotation {
        journal.set_context_rotation(on).map_err(e)?;
    }
    if let Some(on) = caps {
        journal.set_context_tool_caps(on).map_err(e)?;
    }
    if let Some(on) = obsolete {
        journal.set_context_obsolete_check(on).map_err(e)?;
    }
    if let Some(on) = native {
        journal.set_context_native_compact(on).map_err(e)?;
    }
    if let Some(large) = window {
        journal.set_context_window_large(large).map_err(e)?;
    }
    let changed = rotation.is_some() || caps.is_some() || obsolete.is_some() || native.is_some() || window.is_some();
    let now = (word(journal.context_rotation()), word(journal.context_tool_caps()), word(journal.context_obsolete_check()));
    if changed {
        println!("{}", t!("context.config.set", squad = squad, rotation = now.0, caps = now.1, obsolete = now.2));
    } else {
        println!("{}", t!("context.config.now", squad = squad, rotation = now.0, caps = now.1, obsolete = now.2));
    }
    let size = if journal.context_window_large() { "large" } else { "standard" };
    println!(
        "{}",
        t!("context.config.engine", native = word(journal.context_native_compact()), window = size, desk = NATIVE_DESK_TOKENS / 1000, ticket = NATIVE_TICKET_TOKENS / 1000)
    );
    println!("{}", t!("context.config.limits", desk = ROTATE_DESK_TOKENS / 1000, ticket = ROTATE_TICKET_TOKENS / 1000, answer = MAX_TOOL_TOKENS / 1000));
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()).saturating_sub(7 * 24 * 3600);
    let rotations = journal.rotations_since(since).unwrap_or_default();
    let count = |kind: &str| rotations.iter().find(|(k, _, _)| k == kind).map_or(0, |(_, n, _)| *n);
    let (desk, ticket) = (count("desk"), count("ticket"));
    if desk + ticket == 0 {
        println!("{}", t!("context.config.none"));
    } else {
        let tokens: u64 = rotations.iter().map(|(_, _, t)| *t).sum();
        println!("{}", t!("context.config.rotations", total = desk + ticket, desk = desk, ticket = ticket, tokens = tokens / 1000));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_is_on_or_off_and_nothing_else() {
        assert_eq!(switch(&None), Ok(None));
        assert_eq!(switch(&Some("off".into())), Ok(Some(false)));
        assert_eq!(switch(&Some(" ON ".into())), Ok(Some(true)));
        assert!(switch(&Some("maybe".into())).is_err());
    }

    #[test]
    fn the_command_takes_each_switch() {
        use clap::Parser as _;
        #[derive(clap::Parser)]
        struct Cli {
            #[command(flatten)]
            args: ContextConfigArgs,
        }
        let cli = Cli::try_parse_from(["x", "--rotation", "off", "--tool-caps", "on", "--obsolete-check", "off", "--squad", "acme"]).unwrap();
        let a = cli.args;
        assert_eq!((a.rotation.as_deref(), a.tool_caps.as_deref(), a.obsolete_check.as_deref(), a.squad.as_deref()), (Some("off"), Some("on"), Some("off"), Some("acme")));
    }
}
