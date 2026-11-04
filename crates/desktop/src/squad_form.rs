//! A squad's four questions, as rules with no UI types: the name it may take,
//! the team it starts with, and where it is made. Both forms ask them — the
//! first run's page (`views/welcome.rs`) and the "새 스쿼드…" sheet over the
//! window (`views/new_squad.rs`) — and both draw the same fields.

use std::path::{Path, PathBuf};

use equipe_core::protocol::Mode;
use equipe_core::roles::{PM, STARTERS};
use equipe_core::squads;

/// Why a name can't be the new squad's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameProblem {
    /// Nothing typed, and nothing to suggest in its place.
    Empty,
    /// Not a slug: lowercase letters, digits and dashes (`squads::valid_name`).
    Rule,
    /// A squad by that name is already on the machine.
    Taken,
}

/// The name the squad takes: what was typed, trimmed — or `fallback` when
/// nothing was — if it is a slug no squad in `taken` has.
pub fn check_name(typed: &str, taken: &[String], fallback: Option<String>) -> Result<String, NameProblem> {
    let typed = typed.trim();
    let name = match (typed.is_empty(), fallback) {
        (false, _) => typed.to_owned(),
        (true, Some(f)) => f,
        (true, None) => return Err(NameProblem::Empty),
    };
    if !squads::valid_name(&name) {
        return Err(NameProblem::Rule);
    }
    if taken.contains(&name) {
        return Err(NameProblem::Taken);
    }
    Ok(name)
}

/// The roles the squad starts with: the PM first, always, then the starters ticked.
pub fn team(ticked: &[bool; STARTERS.len()]) -> Vec<String> {
    std::iter::once(PM).chain(STARTERS.iter().zip(ticked).filter(|(_, on)| **on).map(|((id, _), _)| *id)).map(str::to_owned).collect()
}

/// The starters ticked before anyone touches them.
pub fn default_team() -> [bool; STARTERS.len()] {
    STARTERS.map(|(_, on)| on)
}

/// How much a new squad does alone before anyone says otherwise.
pub const DEFAULT_MODE: Mode = Mode::Auto;

/// Where "새 스쿼드…" makes the squad, for the machine the window is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// This Mac: its own disk, as the first run does — no `equipe` needed.
    Here,
    /// Another machine: `equipe squad new` run there, over the SSH connection
    /// the window already has (`sidecar::Job::Squad`), the console's own path.
    /// Its folder is one on that machine, so this Mac's picker has none to give.
    There,
    /// Another machine the window can't reach now: nothing would answer.
    Unreachable,
}

impl Route {
    pub fn for_window(remote: bool, connected: bool) -> Self {
        match (remote, connected) {
            (false, _) => Self::Here,
            (true, true) => Self::There,
            (true, false) => Self::Unreachable,
        }
    }

    /// The menu item can be pressed.
    pub fn available(self) -> bool {
        self != Self::Unreachable
    }

    /// The folder can be picked with this Mac's own picker.
    pub fn picks_folder(self) -> bool {
        self == Self::Here
    }
}

/// Where Tab goes next among `len` stops from the one at `at` (none: from
/// outside them), round from the last to the first — and back with ⇧Tab.
pub fn next_stop(at: Option<usize>, len: usize, back: bool) -> usize {
    match (at, back) {
        _ if len == 0 => 0,
        (None, false) => 0,
        (None, true) => len - 1,
        (Some(i), false) => (i + 1) % len,
        (Some(i), true) => (i + len - 1) % len,
    }
}

/// The words `equipe squad new` takes on the far machine.
pub fn new_words(name: &str, folder: Option<&Path>) -> Vec<String> {
    let mut words = vec!["new".to_owned(), name.to_owned()];
    words.extend(folder.map(|f| f.display().to_string()));
    words
}

/// Make the squad on this Mac's disk: the squad, how much it does alone, and
/// its folder. A folder that won't take doesn't undo the squad: why comes back
/// for the window to say, beside the squad that was made.
pub fn make_here(home: &Path, name: &str, mode: Mode, folder: Option<PathBuf>, user_home: Option<&Path>) -> Result<Option<String>, String> {
    squads::create_without_project(home, name)?;
    equipe_core::journal::Journal::for_squad(home, name).and_then(|j| j.set_mode(mode)).map_err(|e| e.to_string())?;
    Ok(folder.and_then(|f| squads::add_folder(home, name, &f.display().to_string(), user_home).err()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_name_is_a_free_slug() {
        let taken = names(&["assistant", "equipe-squad"]);
        assert_eq!(check_name("  shop  ", &taken, None), Ok("shop".to_owned()));
        assert_eq!(check_name("photo-studio-2", &taken, None), Ok("photo-studio-2".to_owned()));
        assert_eq!(check_name("Shop", &taken, None), Err(NameProblem::Rule));
        assert_eq!(check_name("my shop", &taken, None), Err(NameProblem::Rule));
        assert_eq!(check_name("-shop", &taken, None), Err(NameProblem::Rule));
        assert_eq!(check_name("가게", &taken, None), Err(NameProblem::Rule));
        assert_eq!(check_name(&"a".repeat(41), &taken, None), Err(NameProblem::Rule));
        assert_eq!(check_name("equipe-squad", &taken, None), Err(NameProblem::Taken));
    }

    #[test]
    fn nothing_typed_takes_the_suggestion_or_asks_for_a_name() {
        let taken = names(&["assistant"]);
        assert_eq!(check_name("   ", &taken, None), Err(NameProblem::Empty));
        assert_eq!(check_name("", &taken, Some("abel".into())), Ok("abel".to_owned()));
        // The suggestion is held to the same rules.
        assert_eq!(check_name("", &taken, Some("assistant".into())), Err(NameProblem::Taken));
    }

    #[test]
    fn the_pm_is_always_on_the_team_and_first() {
        let mut ticked = [false; STARTERS.len()];
        assert_eq!(team(&ticked), names(&["pm"]));
        ticked[STARTERS.iter().position(|(id, _)| *id == "qa").unwrap()] = true;
        assert_eq!(team(&ticked), names(&["pm", "qa"]));
        // Untouched, it is the team every screen starts a squad with.
        assert_eq!(team(&default_team()), equipe_core::roles::starting_team());
    }

    #[test]
    fn the_item_makes_the_squad_on_the_machine_the_window_is_on() {
        assert_eq!(Route::for_window(false, true), Route::Here);
        // This Mac's disk is there even while its daemon is starting.
        assert_eq!(Route::for_window(false, false), Route::Here);
        assert_eq!(Route::for_window(true, true), Route::There);
        assert_eq!(Route::for_window(true, false), Route::Unreachable);
        assert!(Route::Here.available() && Route::There.available() && !Route::Unreachable.available());
        // A folder from this Mac's picker means nothing to another machine.
        assert!(Route::Here.picks_folder());
        assert!(!Route::There.picks_folder());
    }

    #[test]
    fn tab_goes_round_the_sheet_and_never_out_of_it() {
        assert_eq!(next_stop(None, 5, false), 0);
        assert_eq!(next_stop(None, 5, true), 4);
        assert_eq!(next_stop(Some(1), 5, false), 2);
        assert_eq!(next_stop(Some(4), 5, false), 0);
        assert_eq!(next_stop(Some(0), 5, true), 4);
        assert_eq!(next_stop(Some(3), 5, true), 2);
    }

    #[test]
    fn far_away_it_is_the_consoles_own_command() {
        assert_eq!(new_words("shop", None), names(&["new", "shop"]));
        assert_eq!(new_words("shop", Some(Path::new("/srv/shop"))), names(&["new", "shop", "/srv/shop"]));
        let job = crate::sidecar::Job::Squad { squad: String::new(), words: new_words("shop", None), dry_run: false };
        assert_eq!(job.args(), names(&["squad", "new", "shop"]));
    }

    #[test]
    fn made_here_it_has_its_mode_and_a_name_taken_is_refused() {
        let home = std::env::temp_dir().join(format!("equipe-squad-form-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert_eq!(make_here(&home, "shop", Mode::Ask, None, None), Ok(None));
        assert!(squads::exists(&home, "shop"));
        assert_eq!(equipe_core::journal::Journal::for_squad(&home, "shop").unwrap().mode(), Mode::Ask);
        assert!(make_here(&home, "shop", Mode::Auto, None, None).is_err());
        // A folder that won't take is said, and the squad stays.
        let said = make_here(&home, "cafe", Mode::Auto, Some(PathBuf::from("relative/path")), None).unwrap();
        assert!(said.is_some() && squads::exists(&home, "cafe"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
