//! A squad's design system, as screens show it (`docs/DESIGN-SYSTEM.md`):
//! a `design/` folder the designer keeps — `tokens.json` (W3C Design Tokens),
//! `guidelines.md`, `assets/`, `components/` — in the squad's folder, or in a
//! project of its own. The daemon reads it and sends what it holds, read and
//! resolved ([`Event::Design`](crate::Event::Design)), so a screen on another
//! Mac or a phone never reads the daemon's disk; its pictures come through
//! `read_image`, by the paths named here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::TicketId;

/// One design system: the squad's, a project's, or a ticket's draft of either.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignSystem {
    /// The project it belongs to, by name; `None`: the squad's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Its folder, on the daemon's machine.
    pub dir: String,
    /// A draft a ticket handed in and the person hasn't accepted: not the
    /// design system yet. `None`: the accepted one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<DraftOf>,
    /// Its modes, the first the default: `["light", "dark"]`. Empty: one look.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<String>,
    /// Every token, in the file's order, aliases resolved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tokens: Vec<Token>,
    /// What couldn't be read, in a line each: the file isn't JSON, an alias
    /// points nowhere, a colour that isn't one. The rest still shows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
    /// `guidelines.md`, as it is (cut short past a limit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidelines: Option<String>,
    /// Its pictures: `assets/` and `components/`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<Asset>,
    /// The service's every flow and screen, each in every state
    /// (`flows/flows.json` and the pictures beside it). `None`: no map yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flows: Option<FlowMap>,
    /// How far the designer's first setup has come, step by step, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<SetupStep>,
    /// Changes whenever anything in the folder does.
    #[serde(default)]
    pub stamp: u64,
}

impl DesignSystem {
    /// Which one it is, for a screen to remember: `squad`, `project:<name>`,
    /// with `#T-<n>` for a draft.
    pub fn key(&self) -> String {
        let base = self.project.as_ref().map_or_else(|| "squad".to_owned(), |p| format!("project:{p}"));
        match &self.draft {
            Some(d) => format!("{base}#T-{}", d.ticket),
            None => base,
        }
    }

    /// Whether there is anything in it to show.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty() && self.guidelines.is_none() && self.assets.is_empty()
    }
}

/// The ticket a draft comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftOf {
    pub ticket: TicketId,
    pub title: String,
}

/// A design token: its name (the path of groups, dotted: `color.bg`), its
/// type (`$type`: `color`, `dimension`, `fontFamily`, `fontWeight`,
/// `duration`, `cubicBezier`, `number`, `typography`, `shadow`,
/// `transition`, …), and its value as DTCG writes it, aliases resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// In the default mode, or its only value.
    pub value: Value,
    /// By mode, when it has a value per mode — every mode it has.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub modes: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The token it refers to, as written (`{color.blue.600}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

impl Token {
    /// Its value in `mode`, else its default.
    pub fn value_in(&self, mode: Option<&str>) -> &Value {
        mode.and_then(|m| self.modes.get(m)).unwrap_or(&self.value)
    }

    /// The group it is in: its name without the last part (`color.brand`).
    pub fn group(&self) -> &str {
        self.name.rsplit_once('.').map_or("", |(group, _)| group)
    }

    /// Its own name, the last part (`primary`).
    pub fn leaf(&self) -> &str {
        self.name.rsplit_once('.').map_or(self.name.as_str(), |(_, leaf)| leaf)
    }
}

/// A picture of the design system's: a logo, an icon, an illustration, a
/// component's preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    /// Where it is on the daemon's machine, as `read_image` asks for it.
    pub path: String,
    /// Where it is in the folder: `assets/logo/mark.svg`.
    pub name: String,
    pub kind: AssetKind,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Logo,
    Icon,
    Illustration,
    Component,
    Other,
}

/// The service's UI map, as the designer keeps it in `design/flows/`:
/// `flows.json` names the flows, their screens in order and each screen's
/// states; the pictures sit beside it, one per state, mode and width, at
/// `flows/<flow>/<screen>/<state>.<mode>.<width>.png` (`login/error.dark.390.png`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowMap {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flows: Vec<Flow>,
    /// What couldn't be read, a line each; the rest still shows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
    /// The pictures in `flows/` of no state the file lists, by where they are
    /// in it (`onboarding/login/eror.dark.390.png`): misnamed, or left from a
    /// state that went.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unlisted: Vec<String>,
}

/// A flow a person goes through: 온보딩, 결제 — its screens in the order met.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    /// Its folder's name, and how a screen names it: `onboarding`.
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screens: Vec<Screen>,
    /// The designer's recordings of going through it, newest first: the
    /// current one per mode and width, then the earlier ones kept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub videos: Vec<Walkthrough>,
}

/// A recording of a flow gone through from start to end, to watch again
/// any time: `flows/<flow>/walkthrough.<mode>.<width>.mp4`, the earlier ones
/// in `flows/<flow>/history/`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Walkthrough {
    /// Where it is on the daemon's machine.
    pub path: String,
    /// `light` or `dark`.
    pub mode: String,
    /// The width it was recorded at, in CSS px.
    pub width: u32,
    /// When it was recorded (its file's time), in Unix seconds.
    pub taken: u64,
    /// How long it runs, in milliseconds.
    pub millis: u64,
    /// Where in it each screen starts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<Mark>,
    /// The latest recording, not one kept from before.
    #[serde(default)]
    pub current: bool,
}

/// The moment a screen comes up in a walkthrough.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    /// The screen's id in the flow: `login`.
    pub screen: String,
    /// From the start, in milliseconds.
    pub at: u64,
}

/// One screen of a flow, in every state it has.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screen {
    /// Its folder's name in the flow's: `login`.
    pub id: String,
    pub title: String,
    /// What it is for and how it behaves, in a line or two.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Where it runs, to shoot it again: `http://localhost:3000/login`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The code it is built from, relative to the repository: when one of
    /// these is newer than its pictures, they are stale.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source: Vec<String>,
    /// The ticket that last changed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<ScreenState>,
    /// The states it should have and has no picture of: its `expects` that
    /// aren't drawn, and its states without a picture — by kind or id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    /// Its code changed after its newest picture was taken.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stale: bool,
}

/// A state of a screen: 기본, 빈 상태, 로딩, 오류 — with its pictures.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenState {
    /// Its pictures' name: `error`.
    pub id: String,
    /// As the person reads it: 오류.
    pub label: String,
    /// What kind of state: one of [`STATE_KINDS`], or another word.
    pub kind: String,
    /// How it comes about or what is different in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<Picture>,
}

/// The kinds of state a screen is checked for, in the order shown.
pub const STATE_KINDS: [&str; 9] = ["default", "empty", "loading", "error", "long", "success", "permission", "offline", "first"];

/// One picture of a state: in a mode, at a width.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picture {
    /// Where it is on the daemon's machine, as `read_image` asks for it.
    pub path: String,
    /// `light` or `dark`.
    pub mode: String,
    /// The width it was shot at, in CSS px.
    pub width: u32,
    /// When it was taken (its file's time), in Unix seconds.
    #[serde(default)]
    pub taken: u64,
    /// What it holds, as a short hex digest of its bytes — worked out while
    /// a draft of its system is in review, so a screen can tell the pictures
    /// the draft changed from the copies it only kept. `None`: not worked out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// An accepted version of a design folder: a commit on its repository's
/// main branch that changed the folder (answered to
/// [`Command::DesignVersions`](crate::Command::DesignVersions)).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignVersion {
    /// The commit, whole.
    pub commit: String,
    /// When it landed (the commit's time), in Unix seconds.
    pub at: u64,
    /// Its subject line, without the ticket's mark when it names one.
    pub subject: String,
    /// The ticket its subject names (`T-144`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    /// What it changed, in [`DesignPart`]'s order, each once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<DesignPart>,
}

/// A part of a design folder a version changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignPart {
    /// `tokens.json`.
    Tokens,
    /// The logo (`assets/logo/`, or a picture named a logo, a wordmark or a mark).
    Logo,
    /// The UI map (`flows/`).
    Flows,
    /// `guidelines.md`.
    Guidelines,
    /// Anything else: other pictures, components, the brief, design QA.
    Other,
}

/// A step of the designer's first setup, done or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupStep {
    pub step: Setup,
    pub done: bool,
}

/// The designer's first setup, in order: the brief (`brand.md`), the logo
/// (`assets/logo/`), the system (`tokens.json` and `guidelines.md`), the UI
/// map (`flows/flows.json` with a flow), design QA (`qa/` with a report).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Setup {
    Brief,
    Logo,
    System,
    Flows,
    Qa,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_design_system_travels_without_what_it_lacks() {
        let empty = DesignSystem { dir: "/h/squads/acme/home/design".into(), ..Default::default() };
        let wire = serde_json::to_string(&empty).unwrap();
        assert_eq!(wire, r#"{"dir":"/h/squads/acme/home/design","stamp":0}"#);
        assert!(empty.is_empty());
        assert_eq!(empty.key(), "squad");
        let token = Token { name: "color.bg".into(), kind: "color".into(), value: "#F6F5F1".into(), modes: BTreeMap::from([("dark".into(), "#0A0A0C".into())]), description: None, alias: None };
        let draft = DesignSystem { project: Some("web".into()), draft: Some(DraftOf { ticket: 144, title: "브랜드".into() }), tokens: vec![token.clone()], ..empty };
        assert_eq!(draft.key(), "project:web#T-144");
        let back: DesignSystem = serde_json::from_str(&serde_json::to_string(&draft).unwrap()).unwrap();
        assert_eq!(back, draft);
        assert_eq!(serde_json::to_value(&token).unwrap()["type"], "color");
        assert_eq!((token.group(), token.leaf()), ("color", "bg"));
        assert_eq!(token.value_in(Some("dark")), "#0A0A0C");
        assert_eq!(token.value_in(Some("high-contrast")), "#F6F5F1");
    }

    #[test]
    fn a_version_says_when_what_and_which_ticket_and_a_picture_its_hash_only_when_known() {
        let v = DesignVersion { commit: "a".repeat(40), at: 1_793_000_000, subject: "브랜드 리프레시".into(), ticket: Some(170), parts: vec![DesignPart::Tokens, DesignPart::Flows] };
        let wire = serde_json::to_value(&v).unwrap();
        assert_eq!(wire["parts"], serde_json::json!(["tokens", "flows"]));
        assert_eq!(serde_json::from_value::<DesignVersion>(wire).unwrap(), v);
        let bare: DesignVersion = serde_json::from_str(r#"{"commit":"b","at":1,"subject":"x"}"#).unwrap();
        assert!(bare.ticket.is_none() && bare.parts.is_empty());
        let picture = Picture { path: "/d/flows/a/b/default.light.390.png".into(), mode: "light".into(), width: 390, taken: 1, hash: None };
        assert!(!serde_json::to_string(&picture).unwrap().contains("hash"), "an older screen reads it as before");
        let hashed = Picture { hash: Some("9f2c".into()), ..picture };
        assert_eq!(serde_json::from_str::<Picture>(&serde_json::to_string(&hashed).unwrap()).unwrap(), hashed);
    }
}
