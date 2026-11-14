//! `equipe secrets list | show <name> | delete <name>`: the keys, tokens and
//! passwords members obtained and gave equipe to keep (`secret_save`), seen
//! from the person's own terminal. The facts — name, kind, service, the
//! projects, who saved it and when, its last use — come from the squads'
//! journals; a value from this machine's secret store, only for `show`, only
//! after the person says yes at a terminal, and only to that terminal.
//! Refused to a member's command and to anything that isn't a terminal: a
//! member uses a key through `secret_use` and `secret_write`, never by
//! reading it.

use std::io::{BufRead, IsTerminal as _, Write};

use clap::{Args, Subcommand};
use equipe_core::journal::Journal;
use equipe_core::protocol::secrets::{NO_PROJECT, SquadSecret};
use equipe_core::squad_secrets;
use equipe_core::t;
use equipe_core::vault::Vault;

#[derive(Subcommand)]
pub enum SecretsAction {
    /// The kept keys: name, kind, service, projects, who saved it, last use — never a value
    List(ListArgs),
    /// One key's facts, then — if you say yes here, at a terminal — its value
    Show(NameArgs),
    /// Delete a key: its value leaves the secret store
    Delete(DeleteArgs),
}

#[derive(Args)]
pub struct ListArgs {
    /// One squad (default: every squad)
    #[arg(long)]
    pub squad: Option<String>,
    /// The facts as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct NameArgs {
    /// The key's name
    pub name: String,
    /// Its squad, when several keep one of that name
    #[arg(long)]
    pub squad: Option<String>,
}

#[derive(Args)]
pub struct DeleteArgs {
    /// The key's name
    pub name: String,
    /// Its squad, when several keep one of that name
    #[arg(long)]
    pub squad: Option<String>,
    /// Don't ask first
    #[arg(long)]
    pub yes: bool,
}

pub fn command(action: SecretsAction) -> Result<(), String> {
    let home = equipe_providers::credentials::equipe_home();
    // A show asks the Keychain for a value the person wants now: it may wait for its dialog.
    equipe_core::keychain::patient();
    let mut vault = Vault::open(&home, equipe_core::vault::system_store());
    let member = equipe_core::guard::run_by_member(std::env::var_os(equipe_core::guard::MEMBER_ENV));
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let (stdin, mut stdout) = (std::io::stdin(), std::io::stdout());
    let mut input = stdin.lock();
    let mut here = Here { home: &home, vault: &mut vault, member, terminal, input: &mut input, out: &mut stdout };
    match action {
        SecretsAction::List(a) => here.list(a.squad.as_deref(), a.json),
        SecretsAction::Show(a) => here.show(&a.name, a.squad.as_deref()),
        SecretsAction::Delete(a) => here.delete(&a.name, a.squad.as_deref(), a.yes),
    }
}

/// Where a command runs, as plain values: tests give it a scratch home, a
/// vault in memory and a typed answer.
struct Here<'a> {
    home: &'a std::path::Path,
    vault: &'a mut Vault,
    /// Run by a member's command.
    member: bool,
    /// Standard input and output are the person's terminal.
    terminal: bool,
    input: &'a mut dyn BufRead,
    out: &'a mut dyn Write,
}

impl Here<'_> {
    fn squads(&self, only: Option<&str>) -> Vec<String> {
        match only {
            Some(squad) => vec![squad.to_owned()],
            None => equipe_core::squads::list(self.home).into_iter().map(|s| s.name).collect(),
        }
    }

    /// Every kept key of `only` (or every squad), by squad then name.
    fn kept(&self, only: Option<&str>) -> Vec<SquadSecret> {
        let mut all = Vec::new();
        for squad in self.squads(only) {
            if !equipe_core::squads::exists(self.home, &squad) {
                continue;
            }
            if let Ok(journal) = Journal::for_squad(self.home, &squad) {
                all.extend(squad_secrets::read(&journal).into_iter().map(|s| SquadSecret { squad: squad.clone(), ..s }));
            }
        }
        all
    }

    fn find(&self, name: &str, squad: Option<&str>) -> Result<SquadSecret, String> {
        let mut found: Vec<SquadSecret> = self.kept(squad).into_iter().filter(|s| s.name == name).collect();
        match found.len() {
            0 => Err(t!("secrets.not_found", name = name)),
            1 => Ok(found.remove(0)),
            _ => Err(t!("secrets.ambiguous", name = name, squads = found.iter().map(|s| s.squad.as_str()).collect::<Vec<_>>().join(", "))),
        }
    }

    fn say(&mut self, line: &str) -> Result<(), String> {
        writeln!(self.out, "{line}").map_err(|e| e.to_string())
    }

    fn list(&mut self, squad: Option<&str>, json: bool) -> Result<(), String> {
        let all = self.kept(squad);
        if json {
            return self.say(&serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?);
        }
        if all.is_empty() {
            return self.say(t!("secrets.none"));
        }
        for s in &all {
            self.say(&row(s))?;
        }
        Ok(())
    }

    fn show(&mut self, name: &str, squad: Option<&str>) -> Result<(), String> {
        if self.member {
            return Err(t!("secrets.member_refused").to_owned());
        }
        let secret = self.find(name, squad)?;
        for line in facts(&secret) {
            self.say(&line)?;
        }
        if !self.terminal {
            return Err(t!("secrets.need_terminal").to_owned());
        }
        if !self.confirm(&t!("secrets.confirm_show", name = secret.name))? {
            return self.say(t!("secrets.cancelled"));
        }
        let value = self.vault.squad_secret(&secret.squad, &secret.name, true)?;
        self.say(&value)
    }

    fn delete(&mut self, name: &str, squad: Option<&str>, yes: bool) -> Result<(), String> {
        if self.member {
            return Err(t!("secrets.member_refused").to_owned());
        }
        let secret = self.find(name, squad)?;
        if !yes {
            if !self.terminal {
                return Err(t!("secrets.need_terminal").to_owned());
            }
            if !self.confirm(&t!("secrets.confirm_delete", name = secret.name))? {
                return self.say(t!("secrets.cancelled"));
            }
        }
        self.vault.delete_squad_secret(&secret.squad, &secret.name)?;
        let journal = Journal::for_squad(self.home, &secret.squad).map_err(|e| e.to_string())?;
        squad_secrets::remove(&journal, &secret.name)?;
        self.say(&t!("secrets.deleted", name = secret.name))
    }

    /// `question [y/N]`: yes only for an answer that says it.
    fn confirm(&mut self, question: &str) -> Result<bool, String> {
        write!(self.out, "{question} [y/N] ").map_err(|e| e.to_string())?;
        self.out.flush().map_err(|e| e.to_string())?;
        let mut answer = String::new();
        self.input.read_line(&mut answer).map_err(|e| e.to_string())?;
        Ok(matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
    }
}

fn projects(s: &SquadSecret) -> String {
    if s.any_project {
        t!("secrets.any_project").to_owned()
    } else if s.projects.is_empty() {
        t!("secrets.no_projects").to_owned()
    } else {
        s.projects.iter().map(|p| if p == NO_PROJECT { t!("secrets.no_project").to_owned() } else { p.clone() }).collect::<Vec<_>>().join(", ")
    }
}

fn last(s: &SquadSecret) -> String {
    match &s.last_used {
        Some(u) => t!("secrets.last_used", when = equipe_core::squad_memory::date(u.at), member = u.member),
        None => t!("secrets.never_used").to_owned(),
    }
}

/// One line of the list.
fn row(s: &SquadSecret) -> String {
    t!("secrets.row", squad = s.squad, name = s.name, kind = s.kind.code(), service = s.service, projects = projects(s), when = equipe_core::squad_memory::date(s.created_at), member = s.created_by, last = last(s))
}

/// What `show` says before it asks.
fn facts(s: &SquadSecret) -> Vec<String> {
    let mut lines = vec![
        t!("secrets.fact_name", squad = s.squad, name = s.name),
        t!("secrets.fact_kind", kind = s.kind.code(), service = s.service),
        t!("secrets.fact_projects", projects = projects(s)),
        t!("secrets.fact_saved", when = equipe_core::squad_memory::date(s.created_at), member = s.created_by),
        last(s),
    ];
    if !s.note.is_empty() {
        lines.push(s.note.clone());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::secrets::{SecretKind, SecretUse};
    use equipe_core::vault::InMemory;

    fn scratch(name: &str) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("equipe-cli-secrets-{name}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A home with a squad `acme` keeping `ph` (value `phc_cli_test_value_1234`), in a vault in memory.
    fn setup() -> (std::path::PathBuf, Vault) {
        let home = scratch("home");
        let project = scratch("project");
        equipe_core::squads::create(&home, "acme", &project).unwrap();
        let mut vault = Vault::open(&home, Box::new(InMemory::default()));
        vault.set_squad_secret("acme", "ph", "phc_cli_test_value_1234").unwrap();
        let journal = Journal::for_squad(&home, "acme").unwrap();
        let secret = SquadSecret {
            squad: "acme".into(),
            name: "ph".into(),
            kind: SecretKind::ApiKey,
            service: "posthog".into(),
            projects: vec!["web".into()],
            any_project: false,
            created_by: "backend".into(),
            created_ticket: Some(2),
            created_at: 1_791_849_600,
            note: String::new(),
            last_used: Some(SecretUse { at: 1_791_936_000, member: "frontend".into(), ticket: Some(5), project: "web".into(), how: "file".into() }),
        };
        squad_secrets::put(&journal, secret).unwrap();
        (home, vault)
    }

    fn run<T>(home: &std::path::Path, vault: &mut Vault, member: bool, terminal: bool, typed: &str, f: impl FnOnce(&mut Here<'_>) -> T) -> (T, String) {
        let mut input = typed.as_bytes();
        let mut out: Vec<u8> = Vec::new();
        let result = f(&mut Here { home, vault, member, terminal, input: &mut input, out: &mut out });
        (result, String::from_utf8(out).unwrap())
    }

    #[test]
    fn list_gives_facts_and_never_a_value() {
        let (home, mut vault) = setup();
        let (result, out) = run(&home, &mut vault, false, false, "", |h| h.list(None, false));
        assert!(result.is_ok());
        for part in ["acme/ph", "api_key", "posthog", "web", "backend", "frontend"] {
            assert!(out.contains(part), "{part}: {out}");
        }
        assert!(!out.contains("phc_cli_test_value_1234"));
        let (_, json) = run(&home, &mut vault, false, false, "", |h| h.list(Some("acme"), true));
        assert!(json.contains("\"name\": \"ph\"") && !json.contains("phc_cli_test_value_1234"), "{json}");
        let (_, none) = run(&home, &mut vault, false, false, "", |h| h.list(Some("nobody"), false));
        assert!(none.contains("No keys"), "{none}");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn show_prints_the_value_only_after_a_yes_at_a_terminal() {
        let (home, mut vault) = setup();
        let (result, out) = run(&home, &mut vault, false, true, "y\n", |h| h.show("ph", None));
        assert!(result.is_ok());
        assert!(out.contains("Print the value of ph") && out.trim_end().ends_with("phc_cli_test_value_1234"), "{out}");
        // A no, or nothing, prints no value.
        for typed in ["n\n", "\n", ""] {
            let (result, out) = run(&home, &mut vault, false, true, typed, |h| h.show("ph", None));
            assert!(result.is_ok());
            assert!(!out.contains("phc_cli_test_value_1234") && out.contains("Cancelled"), "{out}");
        }
        // Not a terminal: facts, then refused — and a yes in the input is no leave.
        let (result, out) = run(&home, &mut vault, false, false, "y\n", |h| h.show("ph", None));
        assert!(result.unwrap_err().contains("terminal"));
        assert!(!out.contains("phc_cli_test_value_1234"), "{out}");
        // A member's command: refused outright.
        let (result, out) = run(&home, &mut vault, true, true, "y\n", |h| h.show("ph", None));
        assert!(result.unwrap_err().contains("secret_use") && out.is_empty());
        let (result, _) = run(&home, &mut vault, false, true, "y\n", |h| h.show("nope", None));
        assert!(result.unwrap_err().contains("nope"));
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn delete_asks_then_removes_the_facts_and_the_value() {
        let (home, mut vault) = setup();
        let (result, out) = run(&home, &mut vault, false, true, "n\n", |h| h.delete("ph", None, false));
        assert!(result.is_ok() && out.contains("Cancelled"));
        assert!(vault.squad_secret("acme", "ph", true).is_ok(), "still kept");
        let (result, _) = run(&home, &mut vault, true, true, "y\n", |h| h.delete("ph", None, true));
        assert!(result.is_err(), "a member's command can't delete");
        let (result, out) = run(&home, &mut vault, false, true, "yes\n", |h| h.delete("ph", None, false));
        assert!(result.is_ok() && out.contains("Deleted ph"), "{out}");
        assert!(vault.squad_secret("acme", "ph", true).is_err());
        let (_, listed) = run(&home, &mut vault, false, false, "", |h| h.list(None, false));
        assert!(listed.contains("No keys"), "{listed}");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_name_kept_in_two_squads_needs_the_squad() {
        let (home, mut vault) = setup();
        let project = scratch("project2");
        equipe_core::squads::create(&home, "beta", &project).unwrap();
        let journal = Journal::for_squad(&home, "beta").unwrap();
        let other = squad_secrets::read(&Journal::for_squad(&home, "acme").unwrap()).remove(0);
        squad_secrets::put(&journal, SquadSecret { squad: "beta".into(), ..other }).unwrap();
        let (result, _) = run(&home, &mut vault, false, true, "y\n", |h| h.show("ph", None));
        assert!(result.unwrap_err().contains("acme, beta"));
        let (result, _) = run(&home, &mut vault, false, true, "n\n", |h| h.show("ph", Some("beta")));
        assert!(result.is_ok());
        std::fs::remove_dir_all(home).unwrap();
    }
}
