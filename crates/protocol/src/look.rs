//! What a screen shows the app in — its language, light or dark, the
//! window's and the sidebar's size, what is folded, the zoom — and which app
//! it is: told to the daemon by each attached screen (`rpc::SCREEN_LOOK`),
//! so the UI map's real view (`equipe-desktop --features snapshot`) is shot
//! as the person sees it on their own Mac, which is often not the daemon's.

use serde::{Deserialize, Serialize};

/// Light or dark, as the screen draws it now (the Mac's, resolved).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

/// The look one screen shows, and the app showing it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenLook {
    /// The machine the screen runs on, by the name its owner gave it
    /// (`Abel's MacBook Pro`).
    pub machine: String,
    /// The app: `equipe-desktop`.
    pub app: String,
    /// Its version (`0.103.25`) and, when it knows, its commit.
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The language its words are in (`en`, `zh-Hant`).
    pub lang: String,
    pub theme: Theme,
    /// The window, in points.
    pub width: u32,
    pub height: u32,
    /// The sidebar's width as dragged, in points; none: its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_width: Option<u32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub sidebar_hidden: bool,
    /// The sidebar's projects folded to their header, by name (the squad's
    /// shared folder: "").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folded: Vec<String>,
    /// The team shown row by row.
    #[serde(default, skip_serializing_if = "is_false")]
    pub team_open: bool,
    /// The plans' meters folded to their heading.
    #[serde(default, skip_serializing_if = "is_false")]
    pub usage_folded: bool,
    /// The designer's conversation beside the design screen, open or
    /// folded; none: as the screen decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design_talk: Option<bool>,
    /// How large it draws, in percent.
    pub zoom: u16,
    /// Seconds since the person last used this screen — a key, the
    /// pointer, the window brought forward; none: not since it started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle: Option<u64>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A screen's look as the daemon keeps it: when it last heard of it, and
/// when the person last used that screen (unix seconds, the daemon's clock).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenLook {
    pub look: ScreenLook,
    pub seen: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<u64>,
}

/// The daemon's answer to `rpc::PERSON_LOOK`: the look of the screen the
/// person used last; none when no screen ever told one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonLook {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<SeenLook>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::{self, Message};

    #[test]
    fn a_look_travels_and_an_older_one_reads() {
        let look = ScreenLook {
            machine: "MacBook".into(),
            app: "equipe-desktop".into(),
            version: "0.103.25".into(),
            commit: Some("5a1288f".into()),
            lang: "en".into(),
            theme: Theme::Dark,
            width: 1440,
            height: 900,
            sidebar_width: Some(220),
            sidebar_hidden: false,
            folded: vec![String::new(), "web".into()],
            team_open: true,
            usage_folded: false,
            design_talk: Some(false),
            zoom: 110,
            idle: Some(3),
        };
        let line = Message::notification(rpc::SCREEN_LOOK, &look).to_line();
        let back = Message::from_line(&line).unwrap();
        assert_eq!((back.method.as_deref(), back.id), (Some(rpc::SCREEN_LOOK), None), "told, not asked");
        assert_eq!(back.params_as::<ScreenLook>(), Some(look.clone()));
        let wire = serde_json::to_value(&look).unwrap();
        assert_eq!((wire["theme"].as_str(), wire["lang"].as_str(), wire["sidebar_width"].as_u64()), (Some("dark"), Some("en"), Some(220)));
        // What is as it starts takes no room; a screen that says less still reads.
        let plain = ScreenLook { machine: "mini".into(), app: "equipe-desktop".into(), version: "1".into(), lang: "ko".into(), width: 1200, height: 760, zoom: 100, ..ScreenLook::default() };
        let text = serde_json::to_string(&plain).unwrap();
        for absent in ["commit", "sidebar_width", "sidebar_hidden", "folded", "team_open", "usage_folded", "design_talk", "idle"] {
            assert!(!text.contains(absent), "{absent}: {text}");
        }
        assert_eq!(serde_json::from_str::<ScreenLook>(&text).unwrap(), plain);
        // The daemon's answer.
        let seen = SeenLook { look, seen: 1_762_000_000, active: Some(1_761_999_990) };
        let answer = Message::from_line(&Message::response(7, PersonLook { look: Some(seen.clone()) }).to_line()).unwrap();
        assert_eq!(answer.result_as::<PersonLook>().unwrap().look, Some(seen));
        // No screen ever said: still an answer (a bare null would read as none at all).
        let none = Message::from_line(&Message::response(7, PersonLook::default()).to_line()).unwrap();
        assert_eq!((none.result.is_some(), none.result_as::<PersonLook>()), (true, Some(PersonLook::default())));
    }
}
