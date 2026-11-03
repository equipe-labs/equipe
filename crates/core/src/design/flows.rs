//! The service's UI map (`equipe_protocol::design::FlowMap`): every flow a
//! person goes through, its screens in order, and each screen in every state
//! — so the person can see all of it, cases they'd never meet by using it
//! included, and talk about any one screen with the designer or the PM.
//!
//! The designer keeps it in the design folder, under `flows/`:
//!
//! ```text
//! design/flows/flows.json
//! design/flows/<flow>/<screen>/<state>.<mode>.<width>.png   (login/error.dark.390.png)
//! ```
//!
//! `flows.json`:
//!
//! ```json
//! { "flows": [
//!   { "id": "onboarding", "title": "온보딩", "summary": "첫 실행부터 첫 프로젝트까지",
//!     "screens": [
//!       { "id": "login", "title": "로그인", "note": "틀리면 필드 아래 한 줄로 알려요",
//!         "url": "http://localhost:3000/login", "source": ["src/app/login/page.tsx"], "ticket": 142,
//!         "expects": ["empty", "error", "loading"],
//!         "states": [
//!           { "id": "default", "label": "기본", "kind": "default" },
//!           { "id": "error", "label": "오류", "kind": "error", "note": "비밀번호가 틀렸을 때",
//!             "url": "http://localhost:3000/login?fixture=wrong-password" } ] } ] } ] }
//! ```
//!
//! - Ids are folder names: lowercase letters, digits and `-`.
//! - A state's `kind` is one of `STATE_KINDS` or another word; `expects`
//!   names the kinds (or ids) the screen should have — each one not among
//!   its states, or among them with no picture, is `missing`.
//! - A state's `url` (else the screen's) is what `equipe design flows shoot`
//!   opens to take its pictures; a state that can't be reached by a URL is
//!   drawn instead (an HTML mockup, `equipe design render`) to the same path.
//! - `source` paths are relative to the repository the design folder is in;
//!   one modified after the screen's newest picture makes the screen `stale`.
//!
//! The setup (`setup`) is read from the same folder: the brief, the logo, the
//! system, the map and design QA, each done when its files are there.

use std::path::Path;

use equipe_protocol::design::{FlowMap, SetupStep};

use crate::i18n::Lang;

/// The map's folder in the design folder, and its file.
pub const DIR: &str = "flows";
pub const FILE: &str = "flows.json";
/// The biggest `flows.json` read.
pub const FILE_MAX: u64 = 1 << 20;

/// The map in `design` (a design folder), its pictures found and its
/// missing and stale screens worked out; `repo`, the repository it is in
/// (`None` for the squad's own folder: nothing is stale). `None` when there
/// is no `flows/flows.json`.
pub fn read(design: &Path, repo: Option<&Path>, lang: Lang) -> Option<FlowMap> {
    let _ = (design, repo, lang);
    None
}

/// How far the designer's first setup has come in `design`: every step of
/// `Setup`, in order, each done when its files are there.
pub fn setup(design: &Path) -> Vec<SetupStep> {
    let _ = design;
    Vec::new()
}

/// A picture's file name: `error.dark.390.png`.
pub fn picture_name(state: &str, mode: &str, width: u32) -> String {
    format!("{state}.{mode}.{width}.png")
}

/// A picture's state, mode and width, from its file name; `None` when it
/// isn't one.
pub fn parse_picture_name(name: &str) -> Option<(String, String, u32)> {
    let stem = name.strip_suffix(".png")?;
    let mut parts = stem.rsplitn(3, '.');
    let width = parts.next()?.parse().ok()?;
    let mode = parts.next()?;
    let state = parts.next()?;
    (matches!(mode, "light" | "dark") && !state.is_empty()).then(|| (state.to_owned(), mode.to_owned(), width))
}

// --- STUBS for the CLI to build against, until the core's land: the merge
// --- takes the real `Finding`, `FindingKind`, `check`, `Shot`, `shots` and
// --- `starter` (same names and shapes) and drops these.

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Finding {
    pub flow: String,
    pub screen: Option<String>,
    pub state: Option<String>,
    pub kind: FindingKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    MissingState,
    Stale,
    NoDark,
    Unlisted,
    NoPictures,
}

pub fn check(map: &FlowMap) -> Vec<Finding> {
    let _ = map;
    Vec::new()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    pub flow: String,
    pub screen: String,
    pub state: String,
    pub url: Option<String>,
    pub dir: std::path::PathBuf,
}

pub fn shots(design: &Path) -> Vec<Shot> {
    let _ = design;
    Vec::new()
}

pub fn starter(lang: Lang) -> String {
    let _ = lang;
    "{ \"flows\": [] }\n".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Go(String),
    Click(String),
    Fill(String, String),
    Press(String),
    Wait(u64),
    Say(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkStep {
    pub screen: Option<String>,
    pub step: Step,
}

pub fn walk(design: &Path, flow: &str, lang: Lang) -> Result<Vec<WalkStep>, String> {
    let _ = (design, lang);
    Err(format!("{flow}: no walk"))
}

pub const HISTORY: &str = "history";

pub fn video_name(mode: &str, width: u32) -> String {
    format!("walkthrough.{mode}.{width}.mp4")
}

pub fn history_name(stamp: &str, mode: &str, width: u32) -> String {
    format!("walkthrough.{stamp}.{mode}.{width}.mp4")
}

pub fn parse_video_name(name: &str) -> Option<(Option<String>, String, u32)> {
    let rest = name.strip_prefix("walkthrough.")?.strip_suffix(".mp4")?;
    let mut parts = rest.rsplitn(3, '.');
    let width = parts.next()?.parse().ok()?;
    let mode = parts.next()?;
    let stamp = parts.next().map(str::to_owned);
    matches!(mode, "light" | "dark").then(|| (stamp, mode.to_owned(), width))
}

// --- END STUBS

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_named_by_its_state_mode_and_width() {
        assert_eq!(picture_name("error", "dark", 390), "error.dark.390.png");
        assert_eq!(parse_picture_name("error.dark.390.png"), Some(("error".into(), "dark".into(), 390)));
        assert_eq!(parse_picture_name("long-name.light.1280.png"), Some(("long-name".into(), "light".into(), 1280)));
        assert_eq!(parse_picture_name("error.sepia.390.png"), None);
        assert_eq!(parse_picture_name("error.png"), None);
    }
}
