//! 비교하며 검토: a draft of the design system in review, held against the
//! accepted one before it is accepted — the whole UI map flipped through as
//! 지금 / 바뀐 뒤, the screens whose pictures differ marked 바뀜 and stepped
//! through on their own — and 버전 기록, the accepted versions to go back to.
//! Pictures are paired by flow, screen, state, mode and width; one differs
//! by what it holds (the hash the daemon works out while a draft is in
//! review), else by when it was taken. No UI types here, so the rules are tested.

use std::collections::BTreeSet;

use equipe_core::protocol::design::{DesignPart, DesignVersion, FlowMap, Picture, Screen, ScreenState};

/// Which side of the comparison is on show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// 지금: the accepted system.
    Now,
    /// 바뀐 뒤: the draft.
    After,
}

impl Side {
    pub fn parse(word: &str) -> Option<Side> {
        match word {
            "now" | "before" => Some(Side::Now),
            "after" | "draft" => Some(Side::After),
            _ => None,
        }
    }
}

/// Where a picture is in a map: its flow, screen and state, its mode and width.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    pub flow: String,
    pub screen: String,
    pub state: String,
    pub mode: String,
    pub width: u32,
}

/// A picture as the map is now and as the draft has it, matched by [`Key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair<'a> {
    pub key: Key,
    pub now: Option<&'a Picture>,
    pub after: Option<&'a Picture>,
}

impl Pair<'_> {
    /// Changed: drawn again differently, added, or gone.
    pub fn changed(&self) -> bool {
        match (self.now, self.after) {
            (Some(a), Some(b)) => differs(a, b),
            _ => true,
        }
    }
}

/// Whether two pictures of the same place show something different: by what
/// they hold when both say, else by when they were taken.
pub fn differs(now: &Picture, after: &Picture) -> bool {
    match (&now.hash, &after.hash) {
        (Some(a), Some(b)) => a != b,
        _ => now.taken != after.taken,
    }
}

fn keyed(map: &FlowMap) -> Vec<(Key, &Picture)> {
    let mut out = Vec::new();
    for flow in &map.flows {
        for screen in &flow.screens {
            for state in &screen.states {
                for p in &state.pictures {
                    out.push((Key { flow: flow.id.clone(), screen: screen.id.clone(), state: state.id.clone(), mode: p.mode.clone(), width: p.width }, p));
                }
            }
        }
    }
    out
}

/// Every picture of either map, paired: the draft's in its order, then those
/// only the accepted one has.
pub fn pairs<'a>(now: &'a FlowMap, after: &'a FlowMap) -> Vec<Pair<'a>> {
    let before = keyed(now);
    let mut out: Vec<Pair> = keyed(after).into_iter().map(|(key, p)| Pair { now: before.iter().find(|(k, _)| *k == key).map(|(_, q)| *q), after: Some(p), key }).collect();
    for (key, p) in before {
        if !out.iter().any(|pair| pair.key == key) {
            out.push(Pair { key, now: Some(p), after: None });
        }
    }
    out
}

/// What the draft changed in the UI map, by screen and by state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    screens: BTreeSet<(String, String)>,
    states: BTreeSet<(String, String, String)>,
}

/// The screens and states whose pictures differ, and the screens one map has
/// and the other hasn't.
pub fn changes(now: &FlowMap, after: &FlowMap) -> Changes {
    let mut out = Changes::default();
    for pair in pairs(now, after).into_iter().filter(Pair::changed) {
        out.screens.insert((pair.key.flow.clone(), pair.key.screen.clone()));
        out.states.insert((pair.key.flow, pair.key.screen, pair.key.state));
    }
    let screens = |map: &FlowMap| map.flows.iter().flat_map(|f| f.screens.iter().map(|s| (f.id.clone(), s.id.clone()))).collect::<BTreeSet<_>>();
    let (a, b) = (screens(now), screens(after));
    out.screens.extend(a.symmetric_difference(&b).cloned());
    out
}

impl Changes {
    pub fn screen(&self, flow: &str, screen: &str) -> bool {
        self.screens.contains(&(flow.to_owned(), screen.to_owned()))
    }

    pub fn state(&self, flow: &str, screen: &str, state: &str) -> bool {
        self.states.contains(&(flow.to_owned(), screen.to_owned(), state.to_owned()))
    }

    /// How many screens changed, in the map or in one flow.
    pub fn count(&self) -> usize {
        self.screens.len()
    }

    pub fn in_flow(&self, flow: &str) -> usize {
        self.screens.iter().filter(|(f, _)| f == flow).count()
    }

    /// The changed screens `map` has, in its order.
    pub fn order(&self, map: &FlowMap) -> Vec<(String, String)> {
        map.flows.iter().flat_map(|f| f.screens.iter().map(|s| (f.id.clone(), s.id.clone()))).filter(|k| self.screens.contains(k)).collect()
    }

    /// ←/→ among the changed screens only, across the flows: the one after
    /// (or before) `at`, the first (or last) when none is open. `None`: none further.
    pub fn step(&self, map: &FlowMap, at: Option<(&str, &str)>, forward: bool) -> Option<(String, String)> {
        let all: Vec<(String, String)> = map.flows.iter().flat_map(|f| f.screens.iter().map(|s| (f.id.clone(), s.id.clone()))).collect();
        let here = at.and_then(|(f, s)| all.iter().position(|(af, as_)| af == f && as_ == s));
        let changed = |i: &usize| self.screens.contains(&all[*i]);
        let to = match (here, forward) {
            (None, true) => (0..all.len()).find(changed),
            (None, false) => (0..all.len()).rev().find(changed),
            (Some(n), true) => (n + 1..all.len()).find(changed),
            (Some(n), false) => (0..n).rev().find(changed),
        };
        to.map(|i| all[i].clone())
    }
}

/// The same state in the other map: `flow`'s `screen`'s state `state`.
pub fn counterpart<'a>(map: &'a FlowMap, flow: &str, screen: &str, state: &str) -> Option<&'a ScreenState> {
    find_screen(map, flow, screen)?.states.iter().find(|s| s.id == state)
}

pub fn find_screen<'a>(map: &'a FlowMap, flow: &str, screen: &str) -> Option<&'a Screen> {
    map.flows.iter().find(|f| f.id == flow)?.screens.iter().find(|s| s.id == screen)
}

/// Side by side when the window is this wide, else one side at a time.
pub const SIDE_BY_SIDE_AT: f32 = 1180.;

pub fn side_by_side(window_width: f32) -> bool {
    window_width >= SIDE_BY_SIDE_AT
}

// ── 버전 기록 ──

/// A part's word in the locale files.
pub fn part_key(part: DesignPart) -> &'static str {
    match part {
        DesignPart::Tokens => "design.versions_part_tokens",
        DesignPart::Logo => "design.versions_part_logo",
        DesignPart::Flows => "design.versions_part_flows",
        DesignPart::Guidelines => "design.versions_part_guidelines",
        DesignPart::Other => "design.versions_part_other",
    }
}

/// `2026-11-02 14:30`, in the Mac's time zone.
pub fn local_date(unix: u64) -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let secs = unix as libc::time_t;
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!("{:04}-{:02}-{:02} {:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min)
}

/// The short name of a version: its commit's first seven.
pub fn short(version: &DesignVersion) -> String {
    version.commit.chars().take(7).collect()
}

/// Whether a version can be asked back: not the newest — that is the folder now.
pub fn can_restore(versions: &[DesignVersion], at: usize) -> bool {
    at > 0 && at < versions.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::design::Flow;

    fn picture(screen: &str, state: &str, mode: &str, width: u32, hash: Option<&str>, taken: u64) -> Picture {
        Picture { path: format!("/d/{screen}/{state}.{mode}.{width}.png"), mode: mode.into(), width, taken, hash: hash.map(str::to_owned), ..Default::default() }
    }

    fn screen(id: &str, states: Vec<(&str, Vec<Picture>)>) -> Screen {
        Screen { id: id.into(), title: id.into(), states: states.into_iter().map(|(s, pictures)| ScreenState { id: s.into(), label: s.into(), kind: s.into(), note: None, pictures }).collect(), ..Default::default() }
    }

    fn map(flows: Vec<(&str, Vec<Screen>)>) -> FlowMap {
        FlowMap { flows: flows.into_iter().map(|(id, screens)| Flow { id: id.into(), title: id.into(), screens, ..Default::default() }).collect(), ..Default::default() }
    }

    /// 온보딩: login (re-shot in dark), signup (the same); 결제: plans (an
    /// error state added), pay (gone in the draft), receipt (new).
    fn now_and_after() -> (FlowMap, FlowMap) {
        let now = map(vec![
            ("onboarding", vec![
                screen("login", vec![("default", vec![picture("login", "default", "light", 390, Some("a1"), 10), picture("login", "default", "dark", 390, Some("a2"), 10)])]),
                screen("signup", vec![("default", vec![picture("signup", "default", "light", 390, Some("b1"), 10)])]),
            ]),
            ("checkout", vec![
                screen("plans", vec![("default", vec![picture("plans", "default", "light", 390, Some("c1"), 10)])]),
                screen("pay", vec![("default", vec![picture("pay", "default", "light", 390, Some("d1"), 10)])]),
            ]),
        ]);
        let after = map(vec![
            ("onboarding", vec![
                // A worktree's copy: its time differs, what it holds doesn't.
                screen("login", vec![("default", vec![picture("login", "default", "light", 390, Some("a1"), 99), picture("login", "default", "dark", 390, Some("A2"), 99)])]),
                screen("signup", vec![("default", vec![picture("signup", "default", "light", 390, Some("b1"), 99)])]),
            ]),
            ("checkout", vec![
                screen("plans", vec![("default", vec![picture("plans", "default", "light", 390, Some("c1"), 99)]), ("error", vec![picture("plans", "error", "light", 390, Some("c2"), 99)])]),
                screen("receipt", vec![("success", vec![picture("receipt", "success", "light", 390, Some("e1"), 99)])]),
            ]),
        ]);
        (now, after)
    }

    #[test]
    fn pictures_pair_by_flow_screen_state_mode_and_width() {
        let (now, after) = now_and_after();
        let pairs = pairs(&now, &after);
        let said: Vec<(String, bool, bool, bool)> = pairs.iter().map(|p| (format!("{}/{}/{}.{}.{}", p.key.flow, p.key.screen, p.key.state, p.key.mode, p.key.width), p.now.is_some(), p.after.is_some(), p.changed())).collect();
        assert_eq!(
            said,
            [
                ("onboarding/login/default.light.390".to_owned(), true, true, false),
                ("onboarding/login/default.dark.390".into(), true, true, true),
                ("onboarding/signup/default.light.390".into(), true, true, false),
                ("checkout/plans/default.light.390".into(), true, true, false),
                ("checkout/plans/error.light.390".into(), false, true, true),
                ("checkout/receipt/success.light.390".into(), false, true, true),
                ("checkout/pay/default.light.390".into(), true, false, true),
            ],
            "the draft's order, then what only the accepted one has"
        );
    }

    #[test]
    fn a_picture_differs_by_what_it_holds_else_by_when_it_was_taken() {
        let a = picture("s", "default", "light", 390, Some("x"), 10);
        assert!(!differs(&a, &Picture { taken: 99, ..a.clone() }), "copied into a worktree: the same");
        assert!(differs(&a, &Picture { hash: Some("y".into()), ..a.clone() }));
        let bare = Picture { hash: None, ..a.clone() };
        assert!(!differs(&bare, &bare.clone()), "an older daemon: the same time, the same picture");
        assert!(differs(&bare, &Picture { taken: 11, ..bare.clone() }));
        assert!(differs(&a, &Picture { taken: 11, ..bare }), "one side unhashed: by time");
    }

    #[test]
    fn the_changed_screens_are_marked_counted_and_stepped_through_alone() {
        let (now, after) = now_and_after();
        let changes = changes(&now, &after);
        assert!(changes.screen("onboarding", "login") && !changes.screen("onboarding", "signup"));
        assert!(changes.state("onboarding", "login", "default") && changes.state("checkout", "plans", "error") && !changes.state("checkout", "plans", "default"));
        assert!(changes.screen("checkout", "pay") && changes.screen("checkout", "receipt"), "gone and new count too");
        assert_eq!((changes.count(), changes.in_flow("onboarding"), changes.in_flow("checkout")), (4, 1, 3));
        let pair = |f: &str, s: &str| (f.to_owned(), s.to_owned());
        assert_eq!(changes.order(&after), [pair("onboarding", "login"), pair("checkout", "plans"), pair("checkout", "receipt")], "what the draft shows, in its order");
        assert_eq!(changes.order(&now), [pair("onboarding", "login"), pair("checkout", "plans"), pair("checkout", "pay")]);
        // → from nothing open: the first; past signup (unchanged) to plans, across flows.
        assert_eq!(changes.step(&after, None, true), Some(pair("onboarding", "login")));
        assert_eq!(changes.step(&after, Some(("onboarding", "login")), true), Some(pair("checkout", "plans")));
        assert_eq!(changes.step(&after, Some(("onboarding", "signup")), true), Some(pair("checkout", "plans")), "from one that didn't change: the next that did");
        assert_eq!(changes.step(&after, Some(("checkout", "receipt")), true), None, "the last: nowhere further");
        assert_eq!(changes.step(&after, Some(("checkout", "plans")), false), Some(pair("onboarding", "login")));
        assert_eq!(changes.step(&after, None, false), Some(pair("checkout", "receipt")));
        assert_eq!(super::changes(&now, &now), Changes::default(), "the same map: nothing");
    }

    #[test]
    fn the_same_state_is_found_in_the_other_map() {
        let (now, after) = now_and_after();
        assert_eq!(counterpart(&now, "onboarding", "login", "default").map(|s| s.pictures.len()), Some(2));
        assert!(counterpart(&now, "checkout", "plans", "error").is_none(), "a state the draft added");
        assert!(counterpart(&after, "checkout", "pay", "default").is_none(), "a screen the draft took out");
        assert!(side_by_side(1440.) && !side_by_side(1024.));
    }

    #[test]
    fn a_version_reads_short_dated_and_only_an_older_one_goes_back() {
        let v = |c: &str| DesignVersion { commit: c.repeat(40), at: 1_793_000_000, subject: "x".into(), ticket: None, parts: vec![] };
        let versions = [v("a"), v("b")];
        assert_eq!(short(&versions[0]), "aaaaaaa");
        assert!(!can_restore(&versions, 0), "the newest is the folder now");
        assert!(can_restore(&versions, 1) && !can_restore(&versions, 2));
        let date = local_date(1_793_000_000);
        assert_eq!(date.len(), 16, "{date}");
        assert!(date.starts_with("2026-"), "{date}");
        for part in [DesignPart::Tokens, DesignPart::Logo, DesignPart::Flows, DesignPart::Guidelines, DesignPart::Other] {
            assert_ne!(crate::i18n::catalog().get(part_key(part)), part_key(part), "a word for it");
        }
    }
}
