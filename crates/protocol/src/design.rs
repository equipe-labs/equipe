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
}
