//! The weekly design review: once a week the designer comes to the person on
//! its own, in one message — what is out of line in the UI map, one proposal
//! to refine put as a visual question (`ask_decision`), and references worth
//! a look, the other engine's critique beside the proposal. A routine like any
//! (`crate::routines`): added on, once, when a designer joins, the person's to
//! move or turn off; handed to the designer rather than the PM; and held back
//! while the designer's setup is unfinished — then the designer is nudged on
//! with the step it is at instead. Plain functions; the runtime keeps and sends.

use std::path::{Path, PathBuf};

use equipe_protocol::design::{Setup, SetupStep};
use equipe_protocol::routines::{Routine, Schedule};

use crate::i18n::{Lang, core};

/// The role the review is the work of.
pub const ROLE: &str = "designer";

/// Mondays at 10:00, the daemon machine's local time: the week's start, after
/// the PM's check-in (09:00) and before marketing's (11:00).
pub const WHEN: Schedule = Schedule::Weekly { weekday: 1, hour: 10, minute: 0 };

/// What the review brings: what is out of line, one proposal, references.
pub const PARTS: usize = 3;

/// How long after the review is handed over a decision the designer puts is
/// taken for the review's proposal.
pub const ASK_WITHIN: u64 = 6 * 60 * 60;

/// The routine, on, in the person's language — theirs to change or turn off.
pub fn routine(lang: Lang, id: u32) -> Routine {
    Routine {
        id,
        title: core(lang).get("routine.design_review.title").to_owned(),
        prompt: core(lang).get("routine.design_review.prompt").to_owned(),
        when: WHEN,
        on: true,
        last_run: None,
        next_run: None,
    }
}

/// One design folder and how far its setup has come: a project's, by its
/// name, or the squad's own (`None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub project: Option<String>,
    pub dir: PathBuf,
    pub steps: Vec<SetupStep>,
}

/// The design folders the review looks at: each project's that has one, or —
/// none has — the squad's own, if it has one. A folder the setup never
/// reached isn't there.
pub fn places(squad_home: &Path, projects: &[PathBuf]) -> Vec<Place> {
    let at = |project: Option<String>, dir: PathBuf| dir.is_dir().then(|| Place { steps: super::flows::setup(&dir), project, dir });
    let mut out: Vec<Place> = projects.iter().filter_map(|p| at(Some(crate::board::project_name(p)), p.join(super::DIR))).collect();
    if out.is_empty() {
        out.extend(at(None, squad_home.join(super::DIR)));
    }
    out
}

/// Whether the review runs, or the setup comes first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    Review,
    /// The first folder with a step left, and that step; no folder at all:
    /// the brief, of the first project (the squad's, with none).
    Setup { project: Option<String>, dir: Option<PathBuf>, step: Setup },
}

/// The review runs only once every folder it looks at has its five steps done.
pub fn gate(places: &[Place], projects: &[PathBuf]) -> Gate {
    if places.is_empty() {
        return Gate::Setup { project: projects.first().map(|p| crate::board::project_name(p)), dir: None, step: Setup::Brief };
    }
    for place in places {
        if let Some(left) = place.steps.iter().find(|s| !s.done) {
            return Gate::Setup { project: place.project.clone(), dir: Some(place.dir.clone()), step: left.step };
        }
    }
    Gate::Review
}

/// What the designer is handed for the review: the routine as any is, then
/// where the design folders are.
pub fn message(routine: &Routine, places: &[Place]) -> String {
    let folders: Vec<String> = places.iter().map(|p| p.dir.to_string_lossy().into_owned()).collect();
    format!("{}\n\n(Design folders: {}.)", crate::routines::message(routine), folders.join(", "))
}

/// Whether a decision the designer puts at `now` is the review's proposal:
/// the review was handed to it at `handed`, at most [`ASK_WITHIN`] before.
pub fn is_its_proposal(handed: Option<u64>, now: u64) -> bool {
    handed.is_some_and(|at| now.saturating_sub(at) <= ASK_WITHIN)
}

/// The step's number, 1 to 5, and its name in the person's language.
fn step_words(lang: Lang, step: Setup) -> (usize, &'static str) {
    let (n, key) = match step {
        Setup::Brief => (1, "routine.design_review.step_brief"),
        Setup::Logo => (2, "routine.design_review.step_logo"),
        Setup::System => (3, "routine.design_review.step_system"),
        Setup::Flows => (4, "routine.design_review.step_flows"),
        Setup::Qa => (5, "routine.design_review.step_qa"),
    };
    (n, core(lang).get(key))
}

/// What the designer is handed instead while the setup is unfinished: the
/// review waits, and the step it is at goes on.
pub fn nudge(lang: Lang, routine: &Routine, project: Option<&str>, dir: Option<&Path>, step: Setup) -> String {
    let words = core(lang);
    let dir = dir.map(|d| d.to_string_lossy().into_owned()).unwrap_or_else(|| super::DIR.to_owned() + "/");
    let design = match project {
        Some(name) => words.fill("routine.design_review.of_project", &[("name", name.to_owned()), ("dir", dir)]),
        None => words.fill("routine.design_review.of_squad", &[("dir", dir)]),
    };
    let (n, name) = step_words(lang, step);
    let text = words.fill("routine.design_review.setup", &[("design", design), ("n", n.to_string()), ("step", name.to_owned())]);
    format!("[Routine · {}]\n\n{text}", routine.title.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routines::{Clock, Parts, Utc, next_run};

    /// Monday 2026-10-19 08:00 UTC.
    const MON_8: u64 = 1_792_396_800;
    const HOUR: u64 = 3600;

    fn steps(done: usize) -> Vec<SetupStep> {
        [Setup::Brief, Setup::Logo, Setup::System, Setup::Flows, Setup::Qa].into_iter().enumerate().map(|(n, step)| SetupStep { step, done: n < done }).collect()
    }

    fn place(project: Option<&str>, done: usize) -> Place {
        Place { project: project.map(str::to_owned), dir: PathBuf::from(format!("/p/{}/design", project.unwrap_or("squad"))), steps: steps(done) }
    }

    #[test]
    fn the_review_comes_on_monday_mornings_at_ten_after_the_check_in() {
        let r = routine(Lang::Ko, 4);
        assert_eq!((r.id, r.on, r.when), (4, true, Schedule::Weekly { weekday: 1, hour: 10, minute: 0 }));
        assert_eq!(r.title, "디자인 리뷰");
        // Monday 08:00: this morning at 10. Monday 10:00 sharp: next week's.
        assert_eq!(next_run(r.when, MON_8, &Utc), MON_8 + 2 * HOUR);
        assert_eq!(next_run(r.when, MON_8 + 2 * HOUR, &Utc), MON_8 + 2 * HOUR + 7 * 24 * HOUR);
        // From a Saturday night: the Monday after, across the month's end.
        let sat = Utc.unix(Parts { year: 2026, month: 10, day: 31, weekday: 6, hour: 22, minute: 0 });
        let next = Utc.parts(next_run(r.when, sat, &Utc));
        assert_eq!((next.month, next.day, next.weekday, next.hour, next.minute), (11, 2, 1, 10, 0));
        // After the PM's weekly check-in, before marketing's.
        let at = |r: &Routine| r.when.hour_minute();
        assert!(at(&crate::heartbeat::weekly_check_in(Lang::En)) < at(&r) && at(&r) < at(&crate::heartbeat::weekly_marketing_check(Lang::En, 2)));
    }

    #[test]
    fn the_review_waits_for_the_setup_and_names_the_step_it_is_at() {
        let projects = [PathBuf::from("/p/web"), PathBuf::from("/p/app")];
        // No design folder anywhere: the brief, of the first project.
        assert_eq!(gate(&[], &projects), Gate::Setup { project: Some("web".into()), dir: None, step: Setup::Brief });
        assert_eq!(gate(&[], &[]), Gate::Setup { project: None, dir: None, step: Setup::Brief });
        // The first folder with a step left, at that step.
        let at_map = gate(&[place(Some("web"), 5), place(Some("app"), 3)], &projects);
        assert_eq!(at_map, Gate::Setup { project: Some("app".into()), dir: Some("/p/app/design".into()), step: Setup::Flows });
        // A step done after one left still leaves that one.
        let mut gap = place(None, 5);
        gap.steps[1].done = false;
        assert!(matches!(gate(&[gap], &[]), Gate::Setup { step: Setup::Logo, .. }));
        // All five done everywhere: the review.
        assert_eq!(gate(&[place(Some("web"), 5), place(Some("app"), 5)], &projects), Gate::Review);
        assert_eq!(gate(&[place(None, 5)], &[]), Gate::Review);
    }

    #[test]
    fn the_folders_are_the_projects_own_and_the_squads_only_when_none_has_one() {
        let root = std::env::temp_dir().join(format!("equipe-review-{}", std::process::id()));
        let (home, web, api) = (root.join("home"), root.join("web"), root.join("api"));
        for dir in [home.join("design"), web.join("design"), api.clone()] {
            std::fs::create_dir_all(&dir).unwrap();
        }
        std::fs::write(web.join("design/brand.md"), "# brand").unwrap();
        let found = places(&home, &[web.clone(), api.clone()]);
        assert_eq!(found.iter().map(|p| p.project.clone()).collect::<Vec<_>>(), [Some("web".to_owned())], "the API has no design folder; the squad's then doesn't count");
        assert_eq!(found[0].steps.iter().filter(|s| s.done).map(|s| s.step).collect::<Vec<_>>(), [Setup::Brief]);
        assert_eq!(places(&home, std::slice::from_ref(&api)).iter().map(|p| p.project.clone()).collect::<Vec<_>>(), [None]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_prompt_asks_for_the_three_parts_the_critique_and_the_taste_and_changes_nothing() {
        for lang in Lang::ALL {
            let prompt = routine(lang, 1).prompt;
            for must in ["equipe design flows check", "equipe design critique", "ask_decision", "design taste notes", "design/taste.md", "design/decisions/", "design/references/", "tokens.json", "guidelines.md"] {
                assert!(prompt.contains(must), "{lang:?} lacks {must}: {prompt}");
            }
            assert!(prompt.matches("\n1. ").count() == 1 && prompt.contains("\n2. ") && prompt.contains("\n3. "), "{lang:?}: three parts, numbered");
            if lang != Lang::Ko {
                assert!(!prompt.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c)), "{lang:?}");
            }
        }
        let en = routine(Lang::En, 1).prompt;
        for must in ["at most three", "one click", "Two or three references", "its link", "one message", "Change nothing", "Weekly design review"] {
            assert!(en.contains(must), "{must}: {en}");
        }
        let r = routine(Lang::En, 1);
        let handed = message(&r, &[place(Some("web"), 5), place(Some("app"), 5)]);
        assert!(handed.starts_with("[Routine · Design review]\n\n"), "{handed}");
        assert!(handed.ends_with("(Design folders: /p/web/design, /p/app/design.)"), "{handed}");
    }

    #[test]
    fn the_decision_put_soon_after_the_review_is_its_proposal() {
        assert!(is_its_proposal(Some(MON_8), MON_8 + 40 * 60), "asked within the review's turn");
        assert!(is_its_proposal(Some(MON_8), MON_8 + ASK_WITHIN));
        assert!(!is_its_proposal(Some(MON_8), MON_8 + ASK_WITHIN + 1), "a question days on is other work");
        assert!(!is_its_proposal(None, MON_8), "no review handed over");
    }

    #[test]
    fn the_nudge_says_the_review_waits_and_what_step_to_go_on_with() {
        let r = routine(Lang::En, 1);
        let nudge_en = nudge(Lang::En, &r, Some("web"), Some(Path::new("/p/web/design")), Setup::Flows);
        assert!(nudge_en.starts_with("[Routine · Design review]\n\n"), "{nudge_en}");
        for must in ["web", "/p/web/design", "4", "UI map"] {
            assert!(nudge_en.contains(must), "{must}: {nudge_en}");
        }
        let ko = nudge(Lang::Ko, &routine(Lang::Ko, 1), None, None, Setup::Brief);
        assert!(ko.contains("브리프") && ko.contains("design/"), "{ko}");
        for lang in Lang::ALL {
            let said = nudge(lang, &routine(lang, 1), Some("web"), None, Setup::Qa);
            assert!(!said.contains('{'), "{lang:?}: every name filled: {said}");
        }
    }
}
