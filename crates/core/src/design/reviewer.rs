//! Who reviews a designer's work — `equipe design critique` and
//! `second-look`. What makes a review worth having is a reviewer that saw
//! none of the work being made; a second model's eyes help when they are
//! there to be had. So the squad's setting ([`Reviewer`]) decides, and on
//! auto, the default: the other engine when it is installed, signed in and
//! its plan has room, else the member's own engine in a fresh session. A
//! person on one engine always gets that one: nothing fails, and nobody is
//! told to install the other.
//!
//! The daemon decides at each of a member's turns, from what it knows of
//! both engines — signed in, each plan as last read and how fast it burns —
//! and puts the answer in the member's commands' environment
//! ([`REVIEWER_ENV`]), where it stays when a command runs outside the
//! engine's sandbox. From a person's own terminal, or a daemon that predates
//! it, the command decides with what it can see: whether the other engine is
//! installed. Either way, when the other engine is tried and can't run, the
//! member's own takes over ([`fall_back`]). The record says which engine
//! reviewed and why.

use equipe_protocol::Engine;
use equipe_protocol::engines::{ReviewWhy, Reviewed, Reviewer, Window};

use super::critique::caller_engine;

/// Set in a member's commands' environment for each turn: who reviews its
/// design work, and why ([`Reviewed`] as JSON).
pub const REVIEWER_ENV: &str = "EQUIPE_DESIGN_REVIEWER";

/// The other engine takes a review on auto only while its week is used
/// less than this…
pub const WEEKLY_BELOW: u8 = 90;
/// …and its five hours less than this.
pub const FIVE_HOUR_BELOW: u8 = 95;

/// The engine `engine` isn't.
pub fn across(engine: Engine) -> Engine {
    match engine {
        Engine::Claude => Engine::Codex,
        Engine::Codex => Engine::Claude,
    }
}

/// One of the other engine's limits, as last read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limit {
    pub window: Window,
    pub used: u8,
    /// At the burn so far, it runs out before it starts over.
    pub runs_out: bool,
}

/// Where the other engine stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    NotInstalled,
    SignedOut,
    /// Signed in, or taken to be: its limits as last read (none: not read).
    Ready(Vec<Limit>),
}

/// Why the other engine's plan can't take a review now, if it can't.
fn short_of(limits: &[Limit]) -> Option<ReviewWhy> {
    if let Some(l) = limits.iter().filter(|l| l.used >= 100).max_by_key(|l| l.window == Window::Weekly) {
        return Some(ReviewWhy::UsedUp { window: l.window });
    }
    let near = |l: &&Limit| match l.window {
        Window::Weekly => l.used >= WEEKLY_BELOW,
        Window::FiveHour => l.used >= FIVE_HOUR_BELOW,
    };
    if let Some(l) = limits.iter().filter(near).max_by_key(|l| l.window == Window::Weekly) {
        return Some(ReviewWhy::NearLimit { window: l.window, used: l.used });
    }
    // Five hours start over soon enough; a week that runs out doesn't.
    limits.iter().find(|l| l.window == Window::Weekly && l.runs_out).map(|l| ReviewWhy::RunsOut { window: l.window, used: l.used })
}

/// Who reviews the work of a member on `member`, by the squad's `setting`,
/// with the other engine standing as `other`.
pub fn choose(setting: Reviewer, member: Engine, other: &Standing) -> Reviewed {
    let own = |why| Reviewed { engine: member, member: Some(member), why };
    let theirs = |why| Reviewed { engine: across(member), member: Some(member), why };
    match (setting, other) {
        (Reviewer::Same, _) => own(ReviewWhy::Setting),
        (_, Standing::NotInstalled) => own(ReviewWhy::NotInstalled),
        (_, Standing::SignedOut) => own(ReviewWhy::SignedOut),
        (Reviewer::Other, Standing::Ready(_)) => theirs(ReviewWhy::Setting),
        (Reviewer::Auto, Standing::Ready(limits)) => match short_of(limits) {
            Some(why) => own(why),
            None => theirs(ReviewWhy::Room { used: limits.iter().find(|l| l.window == Window::Weekly).map(|l| l.used) }),
        },
    }
}

/// Who reviews, for a command run now: the engine named on it; else what
/// the daemon decided for this turn ([`REVIEWER_ENV`]); else — a person's
/// own terminal, a daemon that predates it — auto with what can be seen
/// here: the other engine when it is `installed`. Nobody's engine known,
/// the work is taken for Claude's (the designer's, unless moved).
pub fn resolve(named: Option<Engine>, env: impl Fn(&str) -> Option<String>, installed: impl Fn(Engine) -> bool) -> Reviewed {
    let caller = caller_engine(&env);
    if let Some(engine) = named {
        return Reviewed { engine, member: caller, why: ReviewWhy::Named };
    }
    if let Some(decided) = env(REVIEWER_ENV).and_then(|v| serde_json::from_str::<Reviewed>(&v).ok()) {
        // The member's engine is the daemon's word; the reviewer stays its decision.
        return decided;
    }
    let member = caller.unwrap_or(Engine::Claude);
    let other = if installed(across(member)) { Standing::Ready(Vec::new()) } else { Standing::NotInstalled };
    choose(Reviewer::Auto, member, &other)
}

/// When the reviewer couldn't run — `missing`: its program isn't there —
/// the member's own engine, fresh, takes over: unless it was named, or was
/// the member's own already.
pub fn fall_back(r: &Reviewed, missing: bool) -> Option<Reviewed> {
    let member = r.member?;
    (r.why != ReviewWhy::Named && r.engine != member).then_some(Reviewed { engine: member, member: Some(member), why: if missing { ReviewWhy::NotInstalled } else { ReviewWhy::Failed } })
}

fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "Claude",
        Engine::Codex => "Codex",
    }
}

fn window_name(window: Window) -> &'static str {
    match window {
        Window::Weekly => "week",
        Window::FiveHour => "five-hour window",
    }
}

/// Who reviewed and why, in a line — what the command prints and the member
/// reads ("Codex reviewed: Codex has room — its week is 40% used").
pub fn said(r: &Reviewed) -> String {
    let who = if r.fresh() { format!("{} reviewed in a fresh session", engine_name(r.engine)) } else { format!("{} reviewed", engine_name(r.engine)) };
    let other = engine_name(r.other());
    let why = match r.why {
        ReviewWhy::Named => "named on the command".to_owned(),
        ReviewWhy::Setting => "the squad's setting".to_owned(),
        ReviewWhy::Room { used: Some(used) } => format!("{other} has room — its week is {used}% used"),
        ReviewWhy::Room { used: None } => format!("{other} is available"),
        ReviewWhy::NotInstalled => format!("{other} isn't installed"),
        ReviewWhy::SignedOut => format!("{other} isn't signed in"),
        ReviewWhy::UsedUp { window } => format!("{other}'s {} is used up", window_name(window)),
        ReviewWhy::NearLimit { window, used } => format!("{other}'s {} is {used}% used", window_name(window)),
        ReviewWhy::RunsOut { window, used } => format!("{other}'s {} is {used}% used and would run out before it starts over", window_name(window)),
        ReviewWhy::Failed => format!("{other} couldn't run"),
    };
    format!("{who}: {why}")
}

/// The line a critique is kept with, first in its file: who reviewed and why.
const MARK: &str = "<!-- equipe-reviewer: ";

/// [`MARK`] with `r`, as a line.
pub fn mark(r: &Reviewed) -> String {
    format!("{MARK}{} -->", serde_json::to_string(r).unwrap_or_default())
}

/// A kept critique's reviewer, and the critique without its mark.
pub fn read_mark(text: &str) -> (Option<Reviewed>, &str) {
    let Some(rest) = text.strip_prefix(MARK) else { return (None, text) };
    let (line, after) = rest.split_once('\n').unwrap_or((rest, ""));
    let json = line.trim_end().strip_suffix("-->").map(str::trim);
    match json.and_then(|j| serde_json::from_str(j).ok()) {
        Some(r) => (Some(r), after),
        None => (None, text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).to_owned())
    }

    fn week(used: u8) -> Limit {
        Limit { window: Window::Weekly, used, runs_out: false }
    }

    fn five(used: u8) -> Limit {
        Limit { window: Window::FiveHour, used, runs_out: false }
    }

    #[test]
    fn on_auto_the_other_engine_reviews_when_it_is_there_with_room_else_a_fresh_session_of_the_members_own() {
        let ready = |limits: Vec<Limit>| choose(Reviewer::Auto, Engine::Claude, &Standing::Ready(limits));
        let room = ready(vec![week(40), five(20)]);
        assert_eq!(room, Reviewed { engine: Engine::Codex, member: Some(Engine::Claude), why: ReviewWhy::Room { used: Some(40) } });
        assert!(!room.fresh());
        assert_eq!(ready(Vec::new()).engine, Engine::Codex, "signed in, its plan not read yet: it takes it");
        assert_eq!(ready(vec![week(89)]).engine, Engine::Codex, "just under the mark");
        // The other engine missing, signed out, or with no room: the member's own, fresh.
        for (standing, why) in [
            (Standing::NotInstalled, ReviewWhy::NotInstalled),
            (Standing::SignedOut, ReviewWhy::SignedOut),
            (Standing::Ready(vec![week(100), five(30)]), ReviewWhy::UsedUp { window: Window::Weekly }),
            (Standing::Ready(vec![week(50), five(100)]), ReviewWhy::UsedUp { window: Window::FiveHour }),
            (Standing::Ready(vec![week(90)]), ReviewWhy::NearLimit { window: Window::Weekly, used: 90 }),
            (Standing::Ready(vec![week(20), five(97)]), ReviewWhy::NearLimit { window: Window::FiveHour, used: 97 }),
            (Standing::Ready(vec![Limit { runs_out: true, ..week(70) }]), ReviewWhy::RunsOut { window: Window::Weekly, used: 70 }),
        ] {
            let r = choose(Reviewer::Auto, Engine::Claude, &standing);
            assert_eq!(r, Reviewed { engine: Engine::Claude, member: Some(Engine::Claude), why }, "{standing:?}");
            assert!(r.fresh());
        }
        // Five hours that run out at this pace start over soon: they don't hold it.
        assert_eq!(ready(vec![week(30), Limit { runs_out: true, ..five(80) }]).engine, Engine::Codex);
        // From a Codex member, the other is Claude.
        assert_eq!(choose(Reviewer::Auto, Engine::Codex, &Standing::Ready(vec![week(10)])).engine, Engine::Claude);
        assert_eq!(choose(Reviewer::Auto, Engine::Codex, &Standing::NotInstalled).engine, Engine::Codex, "only Codex here: Codex");
    }

    #[test]
    fn a_squad_that_set_it_gets_what_it_set_and_never_an_engine_that_isnt_there() {
        let low = Standing::Ready(vec![week(95)]);
        let same = choose(Reviewer::Same, Engine::Claude, &Standing::Ready(vec![week(5)]));
        assert_eq!((same.engine, same.why), (Engine::Claude, ReviewWhy::Setting));
        let other = choose(Reviewer::Other, Engine::Claude, &low);
        assert_eq!((other.engine, other.why), (Engine::Codex, ReviewWhy::Setting), "the person asked for it, low or not");
        let missing = choose(Reviewer::Other, Engine::Claude, &Standing::NotInstalled);
        assert_eq!((missing.engine, missing.why), (Engine::Claude, ReviewWhy::NotInstalled), "one engine here: that one");
        assert_eq!(choose(Reviewer::Other, Engine::Codex, &Standing::SignedOut).engine, Engine::Codex);
    }

    #[test]
    fn a_command_follows_the_daemons_word_inside_the_sandbox_and_out_and_decides_alone_without_it() {
        let decided = Reviewed { engine: Engine::Claude, member: Some(Engine::Claude), why: ReviewWhy::NearLimit { window: Window::Weekly, used: 93 } };
        let json: &'static str = Box::leak(serde_json::to_string(&decided).unwrap().into_boxed_str());
        let pairs: &'static [(&'static str, &'static str)] = Box::leak(Box::new([(REVIEWER_ENV, json), ("CLAUDECODE", "1")]));
        let everything = |_: Engine| true;
        // Inside Claude Code's sandbox: its own variables and equipe's.
        assert_eq!(resolve(None, env(pairs), everything), decided);
        // Outside it, only equipe's remain — the same answer.
        let outside: &'static [(&'static str, &'static str)] = Box::leak(Box::new([(REVIEWER_ENV, json), (crate::design::critique::MEMBER_ENGINE_ENV, "claude")]));
        assert_eq!(resolve(None, env(outside), everything), decided);
        // Named on the command: that one, whatever the daemon said.
        let named = resolve(Some(Engine::Codex), env(pairs), everything);
        assert_eq!((named.engine, named.member, named.why), (Engine::Codex, Some(Engine::Claude), ReviewWhy::Named));
        // No word from the daemon (a person's terminal, an older daemon): the other engine when installed.
        let codex_member = env(&[("CODEX_THREAD_ID", "t-1")]);
        assert_eq!(resolve(None, &codex_member, everything), Reviewed { engine: Engine::Claude, member: Some(Engine::Codex), why: ReviewWhy::Room { used: None } });
        assert_eq!(resolve(None, &codex_member, |e| e == Engine::Codex), Reviewed { engine: Engine::Codex, member: Some(Engine::Codex), why: ReviewWhy::NotInstalled });
        let terminal = resolve(None, env(&[]), |e| e == Engine::Claude);
        assert_eq!((terminal.engine, terminal.why), (Engine::Claude, ReviewWhy::NotInstalled), "only Claude here: Claude, never a failure");
        assert_eq!(resolve(None, env(&[]), everything).engine, Engine::Codex, "the designer is on Claude unless moved");
        // A word that doesn't read is no word.
        assert_eq!(resolve(None, env(&[(REVIEWER_ENV, "codex please"), ("CLAUDECODE", "1")]), everything).engine, Engine::Codex);
    }

    #[test]
    fn when_the_other_engine_cant_run_the_members_own_takes_over_unless_one_was_named() {
        let room = Reviewed { engine: Engine::Codex, member: Some(Engine::Claude), why: ReviewWhy::Room { used: None } };
        assert_eq!(fall_back(&room, true), Some(Reviewed { engine: Engine::Claude, member: Some(Engine::Claude), why: ReviewWhy::NotInstalled }));
        assert_eq!(fall_back(&room, false).map(|r| r.why), Some(ReviewWhy::Failed));
        let set = Reviewed { why: ReviewWhy::Setting, ..room };
        assert_eq!(fall_back(&set, false).map(|r| r.engine), Some(Engine::Claude), "set to the other engine, it still never fails for it");
        assert_eq!(fall_back(&Reviewed { why: ReviewWhy::Named, ..room }, true), None, "named: said as it failed");
        assert_eq!(fall_back(&Reviewed { engine: Engine::Claude, ..room }, false), None, "the member's own already");
        assert_eq!(fall_back(&Reviewed { member: None, ..room }, false), None);
    }

    #[test]
    fn the_record_says_which_engine_reviewed_and_why() {
        let r = |engine, why| Reviewed { engine, member: Some(Engine::Claude), why };
        assert_eq!(said(&r(Engine::Codex, ReviewWhy::Room { used: Some(40) })), "Codex reviewed: Codex has room — its week is 40% used");
        assert_eq!(said(&r(Engine::Codex, ReviewWhy::Room { used: None })), "Codex reviewed: Codex is available");
        assert_eq!(said(&r(Engine::Claude, ReviewWhy::NearLimit { window: Window::Weekly, used: 93 })), "Claude reviewed in a fresh session: Codex's week is 93% used");
        assert_eq!(said(&r(Engine::Claude, ReviewWhy::UsedUp { window: Window::FiveHour })), "Claude reviewed in a fresh session: Codex's five-hour window is used up");
        assert_eq!(said(&r(Engine::Claude, ReviewWhy::NotInstalled)), "Claude reviewed in a fresh session: Codex isn't installed");
        assert_eq!(said(&r(Engine::Claude, ReviewWhy::SignedOut)), "Claude reviewed in a fresh session: Codex isn't signed in");
        assert_eq!(said(&r(Engine::Claude, ReviewWhy::Failed)), "Claude reviewed in a fresh session: Codex couldn't run");
        assert!(said(&r(Engine::Claude, ReviewWhy::RunsOut { window: Window::Weekly, used: 72 })).ends_with("Codex's week is 72% used and would run out before it starts over"));
        assert_eq!(said(&r(Engine::Claude, ReviewWhy::Setting)), "Claude reviewed in a fresh session: the squad's setting");
        assert_eq!(said(&Reviewed { engine: Engine::Claude, member: Some(Engine::Codex), why: ReviewWhy::Room { used: Some(12) } }), "Claude reviewed: Claude has room — its week is 12% used");
        // Kept first in a critique's file, read back off it.
        let room = r(Engine::Codex, ReviewWhy::Room { used: Some(40) });
        let kept = format!("{}\n## Whole\nCalm.", mark(&room));
        assert_eq!(read_mark(&kept), (Some(room), "## Whole\nCalm."));
        assert_eq!(read_mark("## Whole\nCalm."), (None, "## Whole\nCalm."), "a critique kept before it");
        assert_eq!(read_mark("<!-- equipe-reviewer: nonsense -->\nx").0, None);
    }
}
