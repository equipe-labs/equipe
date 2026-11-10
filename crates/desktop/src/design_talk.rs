//! Talk while looking: the designer's conversation docked beside the design
//! screen, and what the person points at there attached to their words as
//! context (`views/design_talk.rs` draws it). The rules, with no UI types:
//! what a pointed-at thing is called on its chip, the block the member reads
//! before the words (and the pictures that go with it), that block read back
//! for the bubble, and whether the panel is open and how wide.

use std::path::Path;

use equipe_core::t;

use crate::attach::{self, Kind, Meta};

/// Something on the design screen the person pointed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesignContext {
    /// A colour token: its full name (`color.brand.accent`), its value in
    /// light (or its one value) and in dark when it has one.
    Token { name: String, value_light: String, value_dark: Option<String> },
    /// A type style: its full name, and a word on it (`Inter 600 · 32/40`).
    TypeStyle { name: String, sample: String },
    /// A logo or another picture: its name in the system (`assets/logo/mark.svg`)
    /// and where it is on the daemon's machine.
    Asset { name: String, path: String },
    /// A screen of the UI map in one state: the ids (`onboarding`, `login`,
    /// `error`), the title as the person reads it (`Onboarding › Log in › Error`)
    /// and the picture on show, if it has one.
    Screen { flow: String, screen: String, state: String, title: String, picture: Option<String> },
    /// A component of the system: its name (`evidence-card`), its title as
    /// the person reads it, and the picture of it on show, if it has one.
    Component { name: String, title: String, picture: Option<String> },
}

/// The most chips one message carries.
pub const MOST: usize = 12;

/// The block's marks: what the member reads the context between.
const OPEN: &str = "<design-context>";
const CLOSE: &str = "</design-context>";

fn leaf(name: &str) -> &str {
    name.rsplit(['.', '/']).next().unwrap_or(name)
}

/// A logo by where it is and what it is called, as the design folder sorts them.
fn is_logo(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let file = lower.rsplit('/').next().unwrap_or_default();
    lower.split('/').any(|part| part.starts_with("logo") || part == "brand") || file.contains("logo") || file.contains("wordmark") || file.starts_with("mark")
}

impl DesignContext {
    /// What its chip says: `Colour · accent (#1D4ED8 / dark #8FB0FF)`, `Logo · mark.svg`, `Onboarding › Log in › Error`.
    pub fn label(&self) -> String {
        match self {
            Self::Token { name, value_light, value_dark } => {
                let values = match value_dark.as_deref().filter(|d| *d != value_light) {
                    Some(dark) => format!("{value_light} / {} {dark}", t!("design.talk_dark")),
                    None => value_light.clone(),
                };
                format!("{} · {} ({values})", t!("design.talk_kind_color"), leaf(name))
            }
            Self::TypeStyle { name, sample } if sample.trim().is_empty() => format!("{} · {}", t!("design.talk_kind_type"), leaf(name)),
            Self::TypeStyle { name, sample } => format!("{} · {} ({})", t!("design.talk_kind_type"), leaf(name), sample.trim()),
            Self::Asset { name, .. } => {
                let kind = if is_logo(name) { t!("design.talk_kind_logo") } else { t!("design.talk_kind_asset") };
                format!("{kind} · {}", leaf_file(name))
            }
            Self::Screen { title, .. } => title.clone(),
            Self::Component { title, .. } => format!("{} · {title}", t!("design.talk_kind_component")),
        }
    }

    /// The same thing pointed at twice is one chip.
    pub fn key(&self) -> String {
        match self {
            Self::Token { name, .. } | Self::TypeStyle { name, .. } => format!("token:{name}"),
            Self::Asset { path, .. } => format!("asset:{path}"),
            Self::Screen { flow, screen, state, .. } => format!("screen:{flow}/{screen}/{state}"),
            Self::Component { name, .. } => format!("component:{name}"),
        }
    }

    /// The lines under its label in the block: what the member looks it up by.
    fn details(&self) -> Vec<String> {
        match self {
            Self::Token { name, .. } | Self::TypeStyle { name, .. } => vec![format!("token: {name}")],
            Self::Asset { name, path } => vec![format!("asset: {name}"), format!("file: {path}")],
            Self::Screen { flow, screen, state, picture, .. } => {
                let mut out = vec![format!("ui map: {flow} / {screen} / {state}")];
                out.extend(picture.as_ref().map(|p| format!("picture: {p}")));
                out
            }
            Self::Component { name, picture, .. } => {
                let mut out = vec![format!("component: {name}")];
                out.extend(picture.as_ref().map(|p| format!("picture: {p}")));
                out
            }
        }
    }

    /// The picture that goes with it, as the composer attaches one: a tag on
    /// its path on the daemon's machine.
    pub fn picture(&self) -> Option<&str> {
        match self {
            Self::Asset { path, .. } => Some(path),
            Self::Screen { picture, .. } | Self::Component { picture, .. } => picture.as_deref(),
            _ => None,
        }
    }
}

fn leaf_file(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// Point at `item`: added once, at the end, at most [`MOST`]. False: it was there.
pub fn add(list: &mut Vec<DesignContext>, item: DesignContext) -> bool {
    if list.iter().any(|c| c.key() == item.key()) {
        return false;
    }
    if list.len() >= MOST {
        list.remove(0);
    }
    list.push(item);
    true
}

/// The block the member reads before the words: each thing pointed at, its
/// label, then what to look it up by. Empty: nothing pointed at.
pub fn block(items: &[DesignContext]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut out = vec![OPEN.to_owned()];
    for item in items {
        out.push(format!("- {}", item.label().replace('\n', " ")));
        out.extend(item.details().into_iter().map(|d| format!("  {d}")));
    }
    out.push(CLOSE.to_owned());
    out.join("\n")
}

/// The words with what was pointed at before them — after an `@name` that
/// sends them to someone else, so it still does.
pub fn message(items: &[DesignContext], words: &str) -> String {
    let words = words.trim();
    let block = block(items);
    if let Some((mention, rest)) = words.split_once(char::is_whitespace).filter(|(m, _)| m.len() > 1 && m.starts_with('@') && !block.is_empty()) {
        return format!("{mention} {}", message(items, rest));
    }
    match (block.is_empty(), words.is_empty()) {
        (true, _) => words.to_owned(),
        (false, true) => block,
        (false, false) => format!("{block}\n\n{words}"),
    }
}

/// The pictures' tags, each followed by a space (as `Strip::tags`).
pub fn tags(items: &[DesignContext]) -> String {
    items.iter().filter_map(DesignContext::picture).map(|path| format!("{} ", attach::tag(Kind::of(Path::new(path)), path, &Meta::default(), &[], None))).collect()
}

/// A message read back: the labels of what it pointed at, and the rest.
pub fn split(text: &str) -> (Vec<String>, String) {
    let trimmed = text.trim_start();
    let Some(after) = trimmed.strip_prefix(OPEN) else { return (Vec::new(), text.to_owned()) };
    let Some((inside, rest)) = after.split_once(CLOSE) else { return (Vec::new(), text.to_owned()) };
    let labels = inside.lines().filter_map(|l| l.strip_prefix("- ")).map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()).collect();
    (labels, rest.trim_start_matches(['\n', ' ']).to_owned())
}

// ── the panel ──

/// Narrower than this, the room beside the sidebar folds the panel unless
/// the person opened it there.
pub const NARROW: f32 = 1000.;
/// Its width when not dragged, its narrowest, and the room it leaves the
/// design screen at the least.
pub const WIDTH: f32 = 400.;
pub const MIN_WIDTH: f32 = 340.;
const LEAVES: f32 = 560.;

/// Whether the panel is open: in a narrow room only when the person opened
/// it there; else as they left it, open at first.
pub fn shown(kept: Option<bool>, opened_narrow: bool, room: f32) -> bool {
    shown_from(kept, opened_narrow, room, NARROW)
}

/// The designer studio's room is narrow only under this: its own column
/// gives way to the conversation sooner than the design screen's.
pub const STUDIO_NARROW: f32 = 880.;

/// [`shown`], narrow under `narrow`.
pub fn shown_from(kept: Option<bool>, opened_narrow: bool, room: f32, narrow: f32) -> bool {
    if room < narrow { opened_narrow } else { kept != Some(false) }
}

/// Its width: as dragged, within what the room leaves.
pub fn width(dragged: Option<f32>, room: f32) -> f32 {
    let most = (room - LEAVES).max(MIN_WIDTH);
    dragged.unwrap_or(WIDTH).clamp(MIN_WIDTH, most)
}

/// The panel's state this launch.
#[derive(Debug, Default)]
pub struct TalkUi {
    /// Opened while the room was narrow (folded there by default).
    pub opened_narrow: bool,
    /// Its edge being dragged: where it began, the width then.
    pub drag: Option<(f32, f32)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accent() -> DesignContext {
        DesignContext::Token { name: "color.accent".into(), value_light: "#1D4ED8".into(), value_dark: Some("#8FB0FF".into()) }
    }

    fn logo() -> DesignContext {
        DesignContext::Asset { name: "assets/logo/mark.svg".into(), path: "/sq/design/assets/logo/mark.svg".into() }
    }

    fn login_error() -> DesignContext {
        DesignContext::Screen { flow: "onboarding".into(), screen: "login".into(), state: "error".into(), title: "온보딩 › 로그인 › 오류".into(), picture: Some("/sq/design/flows/onboarding/login/error.light.390.png".into()) }
    }

    #[test]
    fn each_thing_pointed_at_is_called_what_the_person_sees() {
        assert_eq!(accent().label(), "색 · accent (#1D4ED8 / 다크 #8FB0FF)");
        let one = DesignContext::Token { name: "color.bg".into(), value_light: "#FFFFFF".into(), value_dark: None };
        assert_eq!(one.label(), "색 · bg (#FFFFFF)");
        let same = DesignContext::Token { name: "color.ink".into(), value_light: "#111".into(), value_dark: Some("#111".into()) };
        assert_eq!(same.label(), "색 · ink (#111)", "one value when both modes are the same");
        assert_eq!(DesignContext::TypeStyle { name: "type.heading.lg".into(), sample: "Inter 600 · 32/40".into() }.label(), "글자 · lg (Inter 600 · 32/40)");
        assert_eq!(DesignContext::TypeStyle { name: "type.body".into(), sample: " ".into() }.label(), "글자 · body");
        assert_eq!(logo().label(), "로고 · mark.svg");
        assert_eq!(DesignContext::Asset { name: "assets/icons/search.png".into(), path: "/x/search.png".into() }.label(), "에셋 · search.png");
        assert_eq!(login_error().label(), "온보딩 › 로그인 › 오류");
        let card = DesignContext::Component { name: "evidence-card".into(), title: "Evidence Card".into(), picture: Some("/sq/design/components/evidence-card.png".into()) };
        assert_eq!(card.label(), "컴포넌트 · Evidence Card");
        assert_eq!(card.key(), "component:evidence-card");
        assert!(block(std::slice::from_ref(&card)).contains("  component: evidence-card\n  picture: /sq/design/components/evidence-card.png"));
        assert_eq!(card.picture(), Some("/sq/design/components/evidence-card.png"));
    }

    #[test]
    fn the_same_thing_pointed_at_again_is_one_chip_and_the_oldest_go_past_the_most() {
        let mut list = Vec::new();
        assert!(add(&mut list, accent()));
        assert!(!add(&mut list, accent()));
        assert!(add(&mut list, logo()));
        assert_eq!(list.len(), 2);
        for n in 0..MOST {
            add(&mut list, DesignContext::Token { name: format!("color.c{n}"), value_light: "#000".into(), value_dark: None });
        }
        assert_eq!(list.len(), MOST);
        assert_eq!(list.last().map(DesignContext::key).as_deref(), Some(format!("token:color.c{}", MOST - 1).as_str()));
        assert!(!list.contains(&accent()), "the oldest went");
    }

    #[test]
    fn the_member_reads_what_was_pointed_at_before_the_words() {
        let text = message(&[accent(), logo(), login_error()], "  이 주황 좀 덜 튀게 ");
        assert_eq!(
            text,
            "<design-context>\n\
             - 색 · accent (#1D4ED8 / 다크 #8FB0FF)\n  token: color.accent\n\
             - 로고 · mark.svg\n  asset: assets/logo/mark.svg\n  file: /sq/design/assets/logo/mark.svg\n\
             - 온보딩 › 로그인 › 오류\n  ui map: onboarding / login / error\n  picture: /sq/design/flows/onboarding/login/error.light.390.png\n\
             </design-context>\n\n이 주황 좀 덜 튀게"
        );
        assert_eq!(message(&[], " 그냥 말 "), "그냥 말");
        assert_eq!(message(&[accent()], "@frontend 이 색으로"), format!("@frontend {}\n\n이 색으로", block(&[accent()])), "still to whoever is named");
        assert_eq!(message(&[accent()], ""), block(&[accent()]));
        assert_eq!(block(&[]), "");
    }

    #[test]
    fn the_pictures_go_as_attachments_do() {
        assert_eq!(tags(&[accent()]), "");
        assert_eq!(tags(&[logo(), login_error()]), "[file: /sq/design/assets/logo/mark.svg] [image: /sq/design/flows/onboarding/login/error.light.390.png] ");
        let none = DesignContext::Screen { flow: "f".into(), screen: "s".into(), state: "empty".into(), title: "F › S › 빈 상태".into(), picture: None };
        assert_eq!(tags(&[none]), "");
    }

    #[test]
    fn a_message_read_back_shows_its_chips_and_its_words() {
        let text = message(&[accent(), login_error()], "> 인용\n\n이 주황 좀 덜 튀게");
        let (labels, rest) = split(&text);
        assert_eq!(labels, ["색 · accent (#1D4ED8 / 다크 #8FB0FF)", "온보딩 › 로그인 › 오류"]);
        assert_eq!(rest, "> 인용\n\n이 주황 좀 덜 튀게");
        assert_eq!(split("그냥 말"), (Vec::new(), "그냥 말".to_owned()));
        assert_eq!(split("<design-context>\n- 끝이 없음"), (Vec::new(), "<design-context>\n- 끝이 없음".to_owned()), "not a block without its end");
        assert_eq!(split(&message(&[accent()], "")).1, "");
    }

    #[test]
    fn the_panel_folds_in_a_narrow_room_unless_opened_there_and_keeps_room_for_the_screen() {
        assert!(shown(None, false, 1400.), "open at first");
        assert!(!shown(Some(false), false, 1400.), "folded as the person left it");
        assert!(shown(Some(true), false, 1400.));
        assert!(!shown(Some(true), false, 800.), "a narrow room folds it");
        assert!(shown(Some(false), true, 800.), "unless opened there");
        assert!(shown_from(None, false, 920., STUDIO_NARROW), "the studio keeps it open in a room the design screen folds it in");
        assert!(!shown(None, false, 920.));
        assert_eq!(width(None, 1400.), WIDTH);
        assert_eq!(width(Some(700.), 1100.), 540.);
        assert_eq!(width(Some(100.), 1400.), MIN_WIDTH);
        assert_eq!(width(Some(900.), 700.), MIN_WIDTH, "never narrower than its least");
    }

    /// The bodies of the functions in `source` whose names start with `prefix`.
    fn bodies<'a>(source: &'a str, prefix: &str) -> Vec<(&'a str, &'a str)> {
        let mut found = Vec::new();
        let mut rest = source;
        while let Some(at) = rest.find(&format!("pub fn {prefix}")) {
            let from = &rest[at + 7..];
            let name = &from[..from.find('(').unwrap()];
            let end = from.find("
    }
").unwrap();
            found.push((name, &from[..end]));
            rest = &from[end..];
        }
        found
    }

    #[test]
    fn every_ask_from_the_design_screen_goes_beside_and_never_through_the_box_or_the_chat_screen() {
        let sources = [include_str!("views/design.rs"), include_str!("views/design_setup.rs"), include_str!("views/flows.rs"), include_str!("views/design_talk.rs")];
        let mut asks = Vec::new();
        for source in sources {
            asks.extend(bodies(source, "design_ask_"));
            asks.extend(bodies(source, "flows_ask_"));
        }
        let names: Vec<&str> = asks.iter().map(|(n, _)| *n).collect();
        for expected in ["design_ask_setup", "design_ask_designer", "design_ask_with", "design_ask_fix", "design_ask_beside", "flows_ask_draw", "flows_ask_map", "flows_ask_video"] {
            assert!(names.contains(&expected), "{expected} in {names:?}");
        }
        for (name, body) in asks {
            // Choosing someone switches to the chat screen; the box is the
            // person's, its words for whatever conversation they belong to.
            for never in ["select_member", "composer.set_text", "self.send(", "send_as(", "Screen::Chat"] {
                assert!(!body.contains(never), "{name} uses {never}");
            }
            assert!(name == "design_ask_beside" || ["design_ask_beside(", "design_ask_with("].iter().any(|via| body.contains(via)), "{name} goes some other way");
        }
    }
}
