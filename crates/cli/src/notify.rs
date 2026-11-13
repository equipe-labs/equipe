//! `equipe notify`: how a squad escalates what stays a bottleneck
//! (`equipe_core::escalation`) — show it, or change one
//! setting: `equipe notify set quiet_from 22:00`, `equipe notify set only urgent`.
//! A setting is the squad's own; `--default` sets the one every squad without
//! its own follows. The daemon reads it from the squad's journal each minute,
//! so a change needs no restart.

use clap::{Args, Subcommand};
use equipe_core::escalation;
use equipe_core::protocol::importance::NotifySettings;
use equipe_core::t;

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct NotifyArgs {
    #[command(subcommand)]
    pub action: Option<NotifyAction>,
    /// The squad (default: the one this folder belongs to, else your first)
    #[arg(long)]
    pub squad: Option<String>,
}

#[derive(Subcommand)]
pub enum NotifyAction {
    /// Show how notifications escalate (the default)
    Show {
        /// The squad (default: the one this folder belongs to, else your first)
        #[arg(long)]
        squad: Option<String>,
    },
    /// Change one setting: on, only (blocking | urgent), quiet (on | off), quiet_from and
    /// quiet_to (HH:MM, your time), levels (three waits: 2h,8h,24h), max_per_hour,
    /// phone (on | off), remind_deploy (2h | off)
    Set {
        /// The setting
        key: String,
        /// Its new value
        value: String,
        /// The squad (default: the one this folder belongs to, else your first)
        #[arg(long)]
        squad: Option<String>,
        /// Set the default every squad without its own settings follows
        #[arg(long)]
        default: bool,
    },
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub fn command(args: NotifyArgs) -> Result<(), String> {
    match args.action {
        Some(NotifyAction::Set { key, value, squad, default }) => set(squad, &key, &value, default),
        Some(NotifyAction::Show { squad }) => show(squad),
        None => show(args.squad),
    }
}

fn squad_of(named: Option<String>) -> Result<(std::path::PathBuf, String), String> {
    let home = equipe_providers::credentials::equipe_home();
    let cwd = std::env::current_dir().unwrap_or_default();
    let squad = crate::home::squad_for(&home, &cwd, named).ok_or_else(|| t!("design.qa.no_squad").to_owned())?;
    Ok((home, squad))
}

fn words() -> &'static equipe_core::i18n::Catalog {
    equipe_core::i18n::core(crate::i18n::lang())
}

fn show(squad: Option<String>) -> Result<(), String> {
    let (home, squad) = squad_of(squad)?;
    let journal = equipe_core::journal::Journal::for_squad(&home, &squad).map_err(|e| e.to_string())?;
    let own = journal.notify_settings();
    let source = if own.is_some() { t!("notify.source_squad", squad = squad).to_string() } else { t!("notify.source_default", squad = squad).to_string() };
    println!("{}", escalation::describe(&escalation::effective(&home, own), words()));
    println!("{source}");
    Ok(())
}

fn set(squad: Option<String>, key: &str, value: &str, default: bool) -> Result<(), String> {
    let (home, squad) = squad_of(squad)?;
    let journal = equipe_core::journal::Journal::for_squad(&home, &squad).map_err(|e| e.to_string())?;
    let mut settings: NotifySettings = if default { escalation::person_default(&home) } else { escalation::effective(&home, journal.notify_settings()) };
    escalation::apply(&mut settings, key, value, words())?;
    // Quiet hours are in the person's own time: this machine's, where they typed it.
    settings.utc_offset_secs = Some(escalation::local_utc_offset(now()));
    if default {
        escalation::save_person_default(&home, &settings).map_err(|e| e.to_string())?;
    } else {
        journal.set_notify_settings(&settings).map_err(|e| e.to_string())?;
    }
    println!("{}", escalation::describe(&settings, words()));
    println!("{}", if default { t!("notify.saved_default").to_string() } else { t!("notify.saved_squad", squad = squad).to_string() });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::escalation::KEYS;

    /// A home with one squad, and the pieces `show` and `set` read and write.
    fn home() -> (std::path::PathBuf, equipe_core::journal::Journal) {
        let dir = std::env::temp_dir().join(format!("equipe-notify-cli-{}-{}", std::process::id(), now()));
        let journal = equipe_core::journal::Journal::for_squad(&dir, "acme").unwrap();
        (dir, journal)
    }

    #[test]
    fn a_squads_own_setting_is_kept_in_its_journal_and_the_default_in_the_home() {
        let (dir, journal) = home();
        assert_eq!(journal.notify_settings(), None);
        let mut s = escalation::effective(&dir, journal.notify_settings());
        escalation::apply(&mut s, "quiet_from", "22:00", words()).unwrap();
        journal.set_notify_settings(&s).unwrap();
        assert_eq!(journal.notify_settings().unwrap().quiet_from, 22 * 60);
        // The squad without its own follows the person's default.
        let mut d = escalation::person_default(&dir);
        escalation::apply(&mut d, "only", "urgent", words()).unwrap();
        escalation::save_person_default(&dir, &d).unwrap();
        let other = equipe_core::journal::Journal::for_squad(&dir, "other").unwrap();
        assert_eq!(escalation::effective(&dir, other.notify_settings()).only, equipe_core::protocol::importance::Only::Urgent);
        assert_eq!(escalation::effective(&dir, journal.notify_settings()).quiet_from, 22 * 60, "its own wins");
        assert!(KEYS.contains(&"quiet_from"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
