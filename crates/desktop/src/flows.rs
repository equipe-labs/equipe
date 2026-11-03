//! 화면 지도's state (`views/flows.rs` draws it): the service's every flow,
//! its screens in order and each screen in every state, as the designer
//! keeps them (`equipe_protocol::design::FlowMap`, in `Event::Design`) — what
//! is counted, what is missing or stale, which picture of a state is shown
//! for the mode and width picked, which recording of a flow to watch, and
//! the message that talks about one. No UI types beyond the strip's scroll
//! and the recordings' first frames, so the rules can be tested.

use std::collections::HashMap;
use std::sync::Arc;

use equipe_core::protocol::design::{Flow, FlowMap, Picture, STATE_KINDS, Screen, ScreenState, Walkthrough};
use gpui_kit::{Image, ScrollHandle};

/// The design screen's two tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    /// 화면 지도.
    Map,
    /// 디자인 시스템.
    System,
}

impl Tab {
    pub const ALL: [Tab; 2] = [Tab::Map, Tab::System];

    /// As it is kept (`desktop-target`) and as a script names it.
    pub fn code(self) -> &'static str {
        match self {
            Tab::Map => "map",
            Tab::System => "system",
        }
    }

    pub fn parse(word: &str) -> Option<Tab> {
        match word {
            "map" | "flows" => Some(Tab::Map),
            "system" | "tokens" => Some(Tab::System),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        crate::i18n::catalog().get(match self {
            Tab::Map => "flows.tab_map",
            Tab::System => "flows.tab_system",
        })
    }

    /// The tab on show: the one the person last picked; else the map when
    /// there is one, the design system when there isn't.
    pub fn shown(picked: Option<Tab>, has_map: bool) -> Tab {
        picked.unwrap_or(if has_map { Tab::Map } else { Tab::System })
    }
}

// ── what is counted ──

/// How much a map (or a flow) holds: its screens, their states, the states
/// they should have and have no picture of, the screens whose code moved on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub screens: usize,
    pub states: usize,
    pub missing: usize,
    pub stale: usize,
}

impl Counts {
    fn add(&mut self, screen: &Screen) {
        self.screens += 1;
        self.states += screen.states.len();
        self.missing += missing_of(screen).len();
        self.stale += usize::from(screen.stale);
    }
}

pub fn counts(map: &FlowMap) -> Counts {
    let mut out = Counts::default();
    for screen in map.flows.iter().flat_map(|f| &f.screens) {
        out.add(screen);
    }
    out
}

pub fn flow_counts(flow: &Flow) -> Counts {
    let mut out = Counts::default();
    for screen in &flow.screens {
        out.add(screen);
    }
    out
}

/// What couldn't be read, then each picture of no state the file lists —
/// a line each, for the quiet list over the map.
pub fn problems(map: &FlowMap) -> Vec<String> {
    let words = crate::i18n::catalog();
    map.problems.iter().cloned().chain(map.unlisted.iter().map(|name| words.fill("flows.unlisted", &[("name", name.clone())]))).collect()
}

/// A state's kind as the person reads it (빈 상태, 권한 없음); another word as it is.
pub fn kind_label(kind: &str) -> String {
    if STATE_KINDS.contains(&kind) { crate::i18n::catalog().get(&format!("flows.kind_{kind}")).to_owned() } else { kind.to_owned() }
}

/// A chip of a screen's states: a state it has (with its pictures or
/// without), or one it should have and lacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip<'a> {
    /// The state's id; `+<word>` for one it lacks.
    pub key: String,
    pub label: String,
    pub state: Option<&'a ScreenState>,
    /// No picture of it.
    pub missing: bool,
}

/// What a screen is missing, once each: its `missing`, and its states that
/// have no picture though `missing` doesn't name them.
fn missing_of(screen: &Screen) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in &screen.missing {
        // A state of its, named by id or kind: the state is what lacks a picture.
        let state = screen.states.iter().find(|s| &s.id == word || (&s.kind == word && s.pictures.is_empty()));
        let key = state.map_or_else(|| word.clone(), |s| s.id.clone());
        if !out.contains(&key) {
            out.push(key);
        }
    }
    for state in screen.states.iter().filter(|s| s.pictures.is_empty()) {
        if !out.contains(&state.id) {
            out.push(state.id.clone());
        }
    }
    out
}

/// A screen's chips: its states in its order, then each state it should
/// have and doesn't.
pub fn chips(screen: &Screen) -> Vec<Chip<'_>> {
    let missing = missing_of(screen);
    let mut out: Vec<Chip> = screen.states.iter().map(|s| Chip { key: s.id.clone(), label: s.label.clone(), state: Some(s), missing: missing.contains(&s.id) }).collect();
    for word in missing.iter().filter(|w| !screen.states.iter().any(|s| &&s.id == w)) {
        out.push(Chip { key: format!("+{word}"), label: kind_label(word), state: None, missing: true });
    }
    out
}

/// The state a screen is shown in first: its default one with a picture,
/// else the first with a picture, else its first.
pub fn default_state(screen: &Screen) -> Option<&ScreenState> {
    let drawn = |s: &&ScreenState| !s.pictures.is_empty();
    screen
        .states
        .iter()
        .filter(drawn)
        .find(|s| s.kind == "default" || s.id == "default")
        .or_else(|| screen.states.iter().find(drawn))
        .or_else(|| screen.states.first())
}

/// The widths a screen was shot at, narrowest first.
pub fn widths(screen: &Screen) -> Vec<u32> {
    let mut out: Vec<u32> = screen.states.iter().flat_map(|s| s.pictures.iter().map(|p| p.width)).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The modes a screen was shot in: light, then dark, then any other.
pub fn modes(screen: &Screen) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in screen.states.iter().flat_map(|s| &s.pictures) {
        if !out.contains(&p.mode) {
            out.push(p.mode.clone());
        }
    }
    out.sort_by_key(|m| match m.as_str() {
        "light" => 0,
        "dark" => 1,
        _ => 2,
    });
    out
}

/// The picture of `state` to show: in `mode` at `width` (`None`: the
/// narrowest); short of that, in the mode at the width nearest (the wider
/// on a tie), then in another mode the same way. `None`: it has none.
pub fn pick<'a>(state: &'a ScreenState, mode: &str, width: Option<u32>) -> Option<&'a Picture> {
    let nearest = |pictures: Vec<&'a Picture>| match width {
        Some(w) => pictures.into_iter().min_by_key(|p| (p.width.abs_diff(w), std::cmp::Reverse(p.width))),
        None => pictures.into_iter().min_by_key(|p| p.width),
    };
    let same: Vec<&Picture> = state.pictures.iter().filter(|p| p.mode == mode).collect();
    if same.is_empty() { nearest(state.pictures.iter().collect()) } else { nearest(same) }
}

// ── watching a flow ──

/// The walkthrough to watch: the newest that shows the flow as it is, else the newest.
pub fn video(flow: &Flow) -> Option<&Walkthrough> {
    flow.videos.iter().find(|v| v.current).or_else(|| flow.videos.first())
}

/// The ones before it, newest first.
pub fn older_videos(flow: &Flow) -> Vec<&Walkthrough> {
    let shown = video(flow).map(|v| v.path.as_str());
    flow.videos.iter().filter(|v| Some(v.path.as_str()) != shown).collect()
}

/// The screens a walkthrough passes, by their titles, each with when it comes (`0:12`).
pub fn marks(flow: &Flow, video: &Walkthrough) -> Vec<(String, String)> {
    video
        .marks
        .iter()
        .map(|m| {
            let title = flow.screens.iter().find(|s| s.id == m.screen).map_or_else(|| m.screen.clone(), |s| s.title.clone());
            (title, crate::media::clock(m.at as f64 / 1000.))
        })
        .collect()
}

/// How long it plays: `1:42`.
pub fn length(video: &Walkthrough) -> String {
    crate::media::clock(video.millis as f64 / 1000.)
}

// ── talking about a screen ──

/// Where a state is: `온보딩 › 로그인 › 오류`.
pub fn context(flow: &Flow, screen: &Screen, state: &str) -> String {
    format!("{} › {} › {}", flow.title, screen.title, state)
}

/// The message about a screen: what it is about as a quote (as asking about
/// words of the conversation sends it, `quote::message`), the person's words,
/// then the picture shown, attached as the composer attaches one (a tag on
/// the daemon's machine's path). Nothing: no words.
pub fn talk_message(about: &str, words: &str, picture: Option<&str>) -> Option<String> {
    let words = words.trim();
    if words.is_empty() {
        return None;
    }
    let text = crate::quote::message(about, words);
    Some(match picture {
        Some(path) => crate::attach::message(&text, &format!("[image: {path}] ")),
        None => text,
    })
}

// ── the tab's state ──

/// Where the person is in the map: the flow, the screen open below the
/// strip and the state on show, the mode and width picked, who to talk to.
#[derive(Default)]
pub struct MapUi {
    /// The flow on show, by id; `None`: the first.
    pub flow: Option<String>,
    /// The screen open below the strip, by id; `None`: none open.
    pub screen: Option<String>,
    /// Its chip on show, by key; `None`: its default state.
    pub chip: Option<String>,
    /// Light or dark as picked; `None`: as the window is.
    pub mode: Option<String>,
    /// The width picked; `None`: the narrowest.
    pub width: Option<u32>,
    /// Talking to the PM rather than the designer.
    pub to_pm: bool,
    /// The state's picture shown big.
    pub zoomed: bool,
    pub problems_open: bool,
    /// The older walkthroughs listed.
    pub older_open: bool,
    /// A walkthrough's first frame, by its path, read on this Mac once
    /// (`None`: there is none to read here).
    pub posters: HashMap<String, Option<Arc<Image>>>,
    pub strip: ScrollHandle,
    /// The tab's column, scrolled to the screen opened.
    pub body: ScrollHandle,
}

impl MapUi {
    pub fn flow<'a>(&self, map: &'a FlowMap) -> Option<&'a Flow> {
        self.flow.as_ref().and_then(|id| map.flows.iter().find(|f| &f.id == id)).or_else(|| map.flows.first())
    }

    /// The screen open, in the flow on show.
    pub fn screen<'a>(&self, map: &'a FlowMap) -> Option<&'a Screen> {
        let flow = self.flow(map)?;
        self.screen.as_ref().and_then(|id| flow.screens.iter().find(|s| &s.id == id))
    }

    /// The chip on show of `screen`: the one picked, else its default state's.
    pub fn chip<'a>(&self, screen: &'a Screen) -> Option<Chip<'a>> {
        let chips = chips(screen);
        let picked = self.chip.as_ref().and_then(|k| chips.iter().position(|c| &c.key == k));
        let default = default_state(screen).and_then(|d| chips.iter().position(|c| c.key == d.id));
        let at = picked.or(default).or(if chips.is_empty() { None } else { Some(0) })?;
        chips.into_iter().nth(at)
    }

    /// The mode to show in: the one picked, else the window's.
    pub fn mode(&self, dark: bool) -> String {
        self.mode.clone().unwrap_or_else(|| if dark { "dark" } else { "light" }.to_owned())
    }

    pub fn pick_flow(&mut self, id: &str) {
        if self.flow.as_deref() != Some(id) {
            self.flow = Some(id.to_owned());
            self.screen = None;
            self.chip = None;
            self.zoomed = false;
            self.older_open = false;
        }
    }

    /// A card clicked: its screen open below the strip, or closed when it was.
    pub fn toggle_screen(&mut self, id: &str) {
        if self.screen.as_deref() == Some(id) {
            self.screen = None;
        } else {
            self.screen = Some(id.to_owned());
        }
        self.chip = None;
        self.zoomed = false;
    }

    /// ←/→: the screen before or after the open one (the first or the last
    /// when none is). Its place in the flow, to scroll to.
    pub fn step_screen(&mut self, map: &FlowMap, forward: bool) -> Option<usize> {
        let screens = &self.flow(map)?.screens;
        if screens.is_empty() {
            return None;
        }
        let at = self.screen.as_ref().and_then(|id| screens.iter().position(|s| &s.id == id));
        let to = match at {
            None if forward => 0,
            None => screens.len() - 1,
            Some(n) if forward => (n + 1).min(screens.len() - 1),
            Some(n) => n.saturating_sub(1),
        };
        if at != Some(to) {
            self.screen = Some(screens[to].id.clone());
            self.chip = None;
        }
        Some(to)
    }

    /// ↑/↓: the flow above or below.
    pub fn step_flow(&mut self, map: &FlowMap, forward: bool) {
        let Some(flow) = self.flow(map) else { return };
        let at = map.flows.iter().position(|f| f.id == flow.id).unwrap_or(0);
        let to = if forward { (at + 1).min(map.flows.len() - 1) } else { at.saturating_sub(1) };
        let id = map.flows[to].id.clone();
        self.pick_flow(&id);
    }

    /// The zoomed picture's ←/→: the state before or after that has a picture.
    pub fn step_state(&mut self, screen: &Screen, forward: bool) {
        let chips: Vec<Chip> = chips(screen).into_iter().filter(|c| !c.missing).collect();
        if chips.is_empty() {
            return;
        }
        let now = self.chip(screen).map(|c| c.key);
        let at = chips.iter().position(|c| Some(&c.key) == now.as_ref()).unwrap_or(0);
        let n = chips.len();
        self.chip = Some(chips[if forward { (at + 1) % n } else { (at + n - 1) % n }].key.clone());
    }

    /// The daemon's word: the place kept where it still is.
    pub fn arrived(&mut self, map: Option<&FlowMap>) {
        let Some(map) = map else {
            *self = Self { mode: self.mode.take(), width: self.width, to_pm: self.to_pm, posters: std::mem::take(&mut self.posters), ..Self::default() };
            return;
        };
        if self.flow.as_ref().is_some_and(|id| !map.flows.iter().any(|f| &f.id == id)) {
            self.flow = None;
            self.screen = None;
        }
        match self.screen(map) {
            None => {
                self.screen = None;
                self.chip = None;
                self.zoomed = false;
            }
            Some(screen) => {
                if self.chip.as_ref().is_some_and(|k| !chips(screen).iter().any(|c| &c.key == k)) {
                    self.chip = None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(mode: &str, width: u32) -> Picture {
        Picture { path: format!("/d/flows/x/{mode}.{width}.png"), mode: mode.into(), width, taken: 1 }
    }

    fn state(id: &str, kind: &str, pictures: Vec<Picture>) -> ScreenState {
        ScreenState { id: id.into(), label: id.to_uppercase(), kind: kind.into(), note: None, pictures }
    }

    fn screen(id: &str, states: Vec<ScreenState>, missing: &[&str], stale: bool) -> Screen {
        Screen { id: id.into(), title: id.into(), states, missing: missing.iter().map(|m| (*m).to_owned()).collect(), stale, ..Default::default() }
    }

    fn map() -> FlowMap {
        let login = screen(
            "login",
            vec![state("default", "default", vec![picture("light", 390), picture("dark", 390), picture("light", 1280)]), state("error", "error", vec![picture("dark", 390)]), state("loading", "loading", vec![])],
            &["empty", "loading"],
            true,
        );
        let home = screen("home", vec![state("first", "first", vec![picture("light", 390)]), state("default", "default", vec![picture("light", 390)])], &[], false);
        let pay = screen("pay", vec![state("default", "default", vec![picture("light", 390)])], &["offline"], false);
        FlowMap {
            flows: vec![
                Flow { id: "onboarding".into(), title: "온보딩".into(), screens: vec![login, home], ..Default::default() },
                Flow { id: "checkout".into(), title: "결제".into(), screens: vec![pay], ..Default::default() },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn the_map_counts_its_screens_states_whats_missing_and_whats_stale() {
        let map = map();
        assert_eq!(counts(&map), Counts { screens: 3, states: 6, missing: 3, stale: 1 });
        assert_eq!(flow_counts(&map.flows[0]), Counts { screens: 2, states: 5, missing: 2, stale: 1 });
        assert_eq!(flow_counts(&map.flows[1]).missing, 1);
        assert_eq!(counts(&FlowMap::default()), Counts::default());
        let read = FlowMap { problems: vec!["flows.json: 7행".into()], unlisted: vec!["onboarding/login/eror.dark.390.png".into()], ..map };
        let lines = problems(&read);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("onboarding/login/eror.dark.390.png"), "a stray picture is named: {lines:?}");
    }

    #[test]
    fn a_screens_chips_are_its_states_then_what_it_lacks_once_each() {
        let map = map();
        let login = &map.flows[0].screens[0];
        let chips = chips(login);
        let keys: Vec<(&str, bool)> = chips.iter().map(|c| (c.key.as_str(), c.missing)).collect();
        assert_eq!(keys, [("default", false), ("error", false), ("loading", true), ("+empty", true)], "the loading state lacks its picture: named once");
        assert_eq!(chips[3].label, kind_label("empty"));
        assert_eq!(kind_label("onboarding-tour"), "onboarding-tour", "a word of the designer's own reads as it is");
        // A state with no picture is missing though `missing` doesn't say so.
        let bare = screen("s", vec![state("default", "default", vec![picture("light", 390)]), state("long", "long", vec![])], &[], false);
        assert_eq!(flow_counts(&Flow { screens: vec![bare.clone()], ..Default::default() }).missing, 1);
        assert!(super::chips(&bare)[1].missing);
    }

    #[test]
    fn a_screen_shows_its_default_state_first_else_the_first_drawn() {
        let map = map();
        assert_eq!(default_state(&map.flows[0].screens[1]).unwrap().id, "default", "by kind, wherever it is");
        let undrawn_default = screen("s", vec![state("default", "default", vec![]), state("error", "error", vec![picture("dark", 390)])], &[], false);
        assert_eq!(default_state(&undrawn_default).unwrap().id, "error");
        let none = screen("s", vec![state("a", "default", vec![])], &[], false);
        assert_eq!(default_state(&none).unwrap().id, "a");
        assert!(default_state(&screen("s", vec![], &[], false)).is_none());
    }

    #[test]
    fn the_picture_shown_is_the_mode_and_width_picked_else_the_nearest() {
        let map = map();
        let login = &map.flows[0].screens[0];
        let default = &login.states[0];
        let shot = |mode: &str, width: Option<u32>| pick(default, mode, width).map(|p| (p.mode.as_str(), p.width));
        assert_eq!(shot("light", Some(1280)), Some(("light", 1280)));
        assert_eq!(shot("light", None), Some(("light", 390)), "no width picked: the narrowest");
        assert_eq!(shot("dark", Some(1280)), Some(("dark", 390)), "dark has only 390: the mode first");
        assert_eq!(shot("light", Some(800)), Some(("light", 390)), "the nearest");
        assert_eq!(shot("light", Some(835)), Some(("light", 1280)), "a tie goes to the wider");
        let error = &login.states[1];
        assert_eq!(pick(error, "light", Some(390)).map(|p| p.mode.as_str()), Some("dark"), "no light one: the other mode");
        assert!(pick(&login.states[2], "light", None).is_none());
        assert_eq!(widths(login), [390, 1280]);
        assert_eq!(modes(login), ["light", "dark"]);
    }

    #[test]
    fn the_message_quotes_where_the_screen_is_then_the_words_and_the_picture() {
        let about = context(&map().flows[0], &map().flows[0].screens[0], "오류");
        assert_eq!(about, "온보딩 › login › 오류");
        assert_eq!(
            talk_message(&about, "  버튼이 너무 작아요 \n", Some("/h/design/flows/onboarding/login/error.dark.390.png")).unwrap(),
            "> 온보딩 › login › 오류\n\n버튼이 너무 작아요 [image: /h/design/flows/onboarding/login/error.dark.390.png]"
        );
        assert_eq!(talk_message(&about, "그려 주세요", None).unwrap(), "> 온보딩 › login › 오류\n\n그려 주세요");
        assert_eq!(talk_message(&about, "   ", Some("/p.png")), None, "nothing said: nothing sent");
        let tags = equipe_core::protocol::tags::find(&talk_message(&about, "x", Some("/p.png")).unwrap());
        assert_eq!(tags.len(), 1, "the picture reads as an attachment");
    }

    #[test]
    fn the_walkthrough_watched_is_the_newest_current_one_and_its_marks_read_as_screens() {
        use equipe_core::protocol::design::Mark;
        let mut flow = map().flows[0].clone();
        assert!(video(&flow).is_none() && older_videos(&flow).is_empty());
        let take = |path: &str, taken: u64, current: bool| Walkthrough { path: path.into(), mode: "light".into(), width: 390, taken, millis: 102_400, marks: vec![], current };
        flow.videos = vec![take("/v/3.mp4", 30, false), take("/v/2.mp4", 20, true), take("/v/1.mp4", 10, false)];
        assert_eq!(video(&flow).unwrap().path, "/v/2.mp4", "the newest is out of date: the newest that isn't");
        assert_eq!(older_videos(&flow).iter().map(|v| v.path.as_str()).collect::<Vec<_>>(), ["/v/3.mp4", "/v/1.mp4"]);
        flow.videos[1].current = false;
        assert_eq!(video(&flow).unwrap().path, "/v/3.mp4", "none current: the newest");
        let mut v = take("/v/4.mp4", 40, true);
        v.marks = vec![Mark { screen: "login".into(), at: 0 }, Mark { screen: "home".into(), at: 12_400 }, Mark { screen: "gone".into(), at: 61_000 }];
        assert_eq!(marks(&flow, &v), [("login".to_owned(), "0:00".to_owned()), ("home".into(), "0:12".into()), ("gone".into(), "1:01".into())]);
        assert_eq!(length(&v), "1:42");
    }

    #[test]
    fn the_tab_is_the_one_picked_else_the_map_when_there_is_one() {
        assert_eq!(Tab::shown(None, true), Tab::Map);
        assert_eq!(Tab::shown(None, false), Tab::System);
        assert_eq!(Tab::shown(Some(Tab::System), true), Tab::System);
        assert_eq!(Tab::shown(Some(Tab::Map), false), Tab::Map, "picked: kept, the empty map says how to get one");
        for tab in Tab::ALL {
            assert_eq!(Tab::parse(tab.code()), Some(tab));
        }
    }

    #[test]
    fn the_place_is_kept_while_it_is_there_and_keys_step_through_it() {
        let map = map();
        let mut ui = MapUi::default();
        assert_eq!(ui.flow(&map).unwrap().id, "onboarding", "the first");
        assert!(ui.screen(&map).is_none(), "none open until one is clicked");
        assert_eq!(ui.step_screen(&map, true), Some(0));
        assert_eq!(ui.screen(&map).unwrap().id, "login");
        assert_eq!(ui.chip(ui.screen(&map).unwrap()).unwrap().key, "default");
        ui.step_screen(&map, true);
        assert_eq!(ui.step_screen(&map, true), Some(1), "the last stays the last");
        ui.toggle_screen("home");
        assert!(ui.screen.is_none(), "its card again: closed");
        ui.toggle_screen("login");
        ui.chip = Some("error".into());
        ui.step_state(ui.screen(&map).unwrap(), true);
        assert_eq!(ui.chip.as_deref(), Some("default"), "past the drawn ones, round to the first");
        ui.chip = Some("+empty".into());
        ui.arrived(Some(&map));
        assert_eq!(ui.chip.as_deref(), Some("+empty"), "still there: kept");
        ui.step_flow(&map, true);
        assert_eq!((ui.flow.as_deref(), ui.screen.as_deref()), (Some("checkout"), None));
        ui.step_flow(&map, true);
        assert_eq!(ui.flow.as_deref(), Some("checkout"));
        // The flow goes: back to the first; the map goes: nothing open, the mode kept.
        ui.mode = Some("dark".into());
        let fewer = FlowMap { flows: vec![map.flows[0].clone()], ..Default::default() };
        ui.arrived(Some(&fewer));
        assert_eq!(ui.flow(&fewer).unwrap().id, "onboarding");
        ui.toggle_screen("login");
        ui.arrived(None);
        assert_eq!((ui.screen.as_deref(), ui.mode.as_deref()), (None, Some("dark")));
        assert_eq!(MapUi::default().mode(true), "dark");
    }
}
