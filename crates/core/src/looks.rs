//! The look each screen shows the app in (`equipe_protocol::look`), as the
//! screens tell it (`rpc::SCREEN_LOOK`): the latest of each machine, when it
//! was heard and when the person last used that screen — kept in the
//! journal of the squad it followed, so it outlives the screen.
//!
//! "The person's look" is that of the screen used last, of any squad: the
//! latest key, pointer or window brought forward wins; of screens no one has
//! used yet, the one heard from last. The UI map's real view is shot in it
//! (`rpc::PERSON_LOOK`), and the map holds its pictures to that screen's
//! version of the app rather than to whatever is installed on the daemon's
//! machine — which is often not where the person sits.

use std::path::Path;

use equipe_protocol::look::{ScreenLook, SeenLook};
use serde::{Deserialize, Serialize};

/// The journal's key for them.
pub const KEY: &str = "screen_looks";
/// Machines kept per squad; past it, the one heard from longest ago goes.
pub const MACHINES_MAX: usize = 16;
/// The longest name or word kept, in characters.
const WORD_MAX: usize = 200;
/// The most folded sections kept.
const FOLDED_MAX: usize = 256;

/// The screens' looks one squad's journal keeps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Looks {
    #[serde(default)]
    pub screens: Vec<SeenLook>,
}

impl Looks {
    /// A screen told its look at `now` (unix seconds): its machine's is
    /// replaced; when the person used it is kept, moved on if they did since.
    pub fn report(&mut self, look: ScreenLook, now: u64) {
        let look = tidy(look);
        let used = look.idle.map(|idle| now.saturating_sub(idle));
        let before = self.screens.iter().position(|s| s.look.machine == look.machine);
        let active = before.and_then(|at| self.screens.remove(at).active).max(used);
        self.screens.push(SeenLook { look, seen: now, active });
        while self.screens.len() > MACHINES_MAX {
            let oldest = self.screens.iter().enumerate().min_by_key(|(_, s)| s.seen).map(|(at, _)| at).unwrap_or(0);
            self.screens.remove(oldest);
        }
    }

    /// The person's: the screen they used last; of screens no one used, the
    /// one heard from last.
    pub fn person(&self) -> Option<&SeenLook> {
        latest(self.screens.iter())
    }
}

/// Of `looks`, the one used last, else heard from last.
fn latest<'a>(looks: impl Iterator<Item = &'a SeenLook>) -> Option<&'a SeenLook> {
    looks.max_by_key(|s| (s.active, s.seen))
}

/// A look as kept: its words no longer than [`WORD_MAX`], its folds no more
/// than [`FOLDED_MAX`].
fn tidy(mut look: ScreenLook) -> ScreenLook {
    let cut = |s: &mut String| {
        if let Some((at, _)) = s.char_indices().nth(WORD_MAX) {
            s.truncate(at);
        }
    };
    for word in [&mut look.machine, &mut look.app, &mut look.version, &mut look.lang] {
        cut(word);
    }
    if let Some(commit) = &mut look.commit {
        cut(commit);
    }
    look.folded.truncate(FOLDED_MAX);
    look.folded.iter_mut().for_each(cut);
    look
}

/// `squad`'s screen told its look at `now`: kept in its journal.
pub fn report(home: &Path, squad: &str, look: ScreenLook, now: u64) -> Result<(), String> {
    // Two screens telling at once: one after the other, so neither is lost.
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    if !crate::squads::valid_name(squad) {
        return Err(format!("no squad {squad}"));
    }
    let journal = crate::journal::Journal::for_squad(home, squad).map_err(|e| e.to_string())?;
    let mut looks = journal.screen_looks();
    looks.report(look, now);
    journal.set_screen_looks(&looks).map_err(|e| e.to_string())?;
    TOLD.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

static TOLD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many looks this process has kept: what was read before is read
/// again once it moves.
pub fn told() -> u64 {
    TOLD.load(std::sync::atomic::Ordering::SeqCst)
}

/// The looks `squad`'s journal keeps; none when it has no journal.
pub fn of(home: &Path, squad: &str) -> Looks {
    let db = crate::journal::squad_dir(home, squad).join("squad.db");
    if !crate::squads::valid_name(squad) || !db.is_file() {
        return Looks::default();
    }
    crate::journal::Journal::open(&db).map(|j| j.screen_looks()).unwrap_or_default()
}

/// The person's look: of every squad's screens, the one used last. `None`
/// when no screen ever told one.
pub fn person(home: &Path) -> Option<SeenLook> {
    let squads = std::fs::read_dir(home.join("squads")).into_iter().flatten().flatten();
    let looks: Vec<Looks> = squads.map(|d| of(home, &d.file_name().to_string_lossy())).collect();
    latest(looks.iter().flat_map(|l| l.screens.iter())).cloned()
}

/// The app the person uses, and where.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PersonApp {
    pub version: String,
    pub machine: String,
}

/// The version of `app` the person uses, and on which machine: their
/// screen's, when the screen they used last is that app.
pub fn person_app(home: &Path, app: &str) -> Option<PersonApp> {
    person(home).filter(|s| s.look.app == app && !s.look.version.trim().is_empty()).map(|s| PersonApp { version: s.look.version, machine: s.look.machine })
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::look::Theme;

    fn look(machine: &str, version: &str, lang: &str, idle: Option<u64>) -> ScreenLook {
        ScreenLook { machine: machine.into(), app: "equipe-desktop".into(), version: version.into(), lang: lang.into(), width: 1200, height: 760, zoom: 100, idle, ..ScreenLook::default() }
    }

    #[test]
    fn the_screen_used_last_is_the_persons_whoever_spoke_last() {
        let mut looks = Looks::default();
        assert_eq!(looks.person(), None, "no screen ever said");
        // The MacBook, used a moment ago.
        looks.report(ScreenLook { sidebar_width: Some(200), theme: Theme::Dark, ..look("MacBook", "0.103.25", "en", Some(2)) }, 1000);
        // The Mac mini's app, attached after it — nobody at it.
        looks.report(look("Mac mini", "0.103.25", "ko", None), 1010);
        let person = looks.person().unwrap();
        assert_eq!((person.look.machine.as_str(), person.look.lang.as_str(), person.active, person.seen), ("MacBook", "en", Some(998), 1000));
        // Someone touches the mini: it is theirs now.
        looks.report(look("Mac mini", "0.103.25", "ko", Some(0)), 1020);
        assert_eq!(looks.person().unwrap().look.machine, "Mac mini");
        // Back at the MacBook, its look changed meanwhile: one entry a machine, the latest.
        looks.report(ScreenLook { sidebar_width: Some(180), ..look("MacBook", "0.103.26", "en", Some(1)) }, 1030);
        assert_eq!(looks.screens.len(), 2);
        let person = looks.person().unwrap();
        assert_eq!((person.look.machine.as_str(), person.look.version.as_str(), person.look.sidebar_width, person.active), ("MacBook", "0.103.26", Some(180), Some(1029)));
        // Told again without being used: it keeps when it was.
        looks.report(look("MacBook", "0.103.26", "en", None), 1040);
        assert_eq!((looks.person().unwrap().active, looks.person().unwrap().seen), (Some(1029), 1040));
        // Of screens no one used, the one heard from last.
        let mut idle = Looks::default();
        idle.report(look("a", "1", "en", None), 5);
        idle.report(look("b", "1", "en", None), 6);
        assert_eq!(idle.person().unwrap().look.machine, "b");
    }

    #[test]
    fn what_a_screen_says_is_kept_small() {
        let mut looks = Looks::default();
        for n in 0..(MACHINES_MAX as u64 + 3) {
            looks.report(look(&format!("m{n}"), "1", "en", None), n);
        }
        assert_eq!(looks.screens.len(), MACHINES_MAX);
        assert!(!looks.screens.iter().any(|s| s.look.machine == "m0"), "the one heard from longest ago went");
        looks.report(ScreenLook { folded: vec!["x".repeat(WORD_MAX + 50); FOLDED_MAX + 9], ..look(&"é".repeat(WORD_MAX * 2), "1", "en", None) }, 99);
        let kept = looks.screens.last().unwrap();
        assert_eq!((kept.look.machine.chars().count(), kept.look.folded.len(), kept.look.folded[0].len()), (WORD_MAX, FOLDED_MAX, WORD_MAX));
    }

    #[test]
    fn kept_in_the_squads_journal_and_read_across_squads() {
        let home = std::env::temp_dir().join(format!("equipe-looks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(person(&home), None, "nothing kept anywhere");
        for squad in ["acme", "zeta"] {
            crate::journal::Journal::for_squad(&home, squad).unwrap();
        }
        assert_eq!(person(&home), None, "squads, but no screen ever said");
        report(&home, "acme", look("Mac mini", "0.86.3", "ko", Some(0)), 100).unwrap();
        report(&home, "zeta", look("MacBook", "0.103.25", "en", Some(0)), 200).unwrap();
        assert_eq!(of(&home, "acme").screens.len(), 1, "each squad keeps its own");
        assert_eq!(person(&home).unwrap().look.machine, "MacBook", "the one used last, whichever squad it followed");
        assert_eq!(person_app(&home, "equipe-desktop"), Some(PersonApp { version: "0.103.25".into(), machine: "MacBook".into() }));
        assert_eq!(person_app(&home, "someone-elses-app"), None);
        assert!(report(&home, "../x", look("a", "1", "en", None), 1).is_err());
        assert_eq!(of(&home, "nobody"), Looks::default());
        std::fs::remove_dir_all(home).unwrap();
    }
}
