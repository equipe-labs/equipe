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
//! design/flows/<flow>/walkthrough.<mode>.<width>.mp4        (and .json beside it)
//! design/flows/<flow>/history/walkthrough.<YYYYMMDD-HHMMSS>.<mode>.<width>.mp4
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
//!             "url": "http://localhost:3000/login?fixture=wrong-password" } ] } ],
//!     "walk": [
//!       { "go": "http://localhost:3000/login", "screen": "login" },
//!       { "fill": ["#email", "ada@example.com"] }, { "click": "button[type=submit]" },
//!       { "press": "Enter" }, { "wait": 800 }, { "say": "비밀번호가 틀리면", "screen": "login" } ] } ] }
//! ```
//!
//! - Ids are folder names: lowercase letters, digits and `-`; no screen is
//!   called `history`, the folder its flow's earlier walkthroughs are kept in.
//! - A state's `kind` is one of `STATE_KINDS` or another word; `expects`
//!   names the kinds (or ids) the screen should have — each one not among
//!   its states, or among them with no picture, is `missing`.
//! - A state's `url` (else the screen's) is what `equipe design flows shoot`
//!   opens to take its pictures.
//! - An app that isn't a web page (a desktop app, a phone simulator, anything
//!   with a screenshot harness of its own) is shot by a `command` instead: a
//!   state's own, else its screen's, else its flow's; with a `command` and a
//!   `url` both, the `command` is what runs. It is a shell command (`sh -c`)
//!   run at the root of the repository the design folder is in, once for each
//!   picture, that writes one PNG. It is given, as `{name}` in its text and as
//!   `EQUIPE_SHOT_<NAME>` in its environment: `out` (the absolute path to
//!   write), `mode` (`light` or `dark`), `width` (px), `flow`, `screen` and
//!   `state`. In the text each is put in already quoted for the shell, so it
//!   is written bare (`{out}`, never `"{out}"`); other braces are left as
//!   they are. `"command": "scripts/design-shot.sh {mode} {width} {out} board"`.
//! - A state neither a `url` nor a `command` reaches is drawn instead (an HTML
//!   mockup, `equipe design render`) to the same path.
//! - `source` paths are relative to the repository the design folder is in;
//!   one modified after the screen's newest picture makes the screen `stale`.
//! - A flow's `walk` is how to go through it, a step at a time, each with one
//!   action — `go` to a URL, `click` a selector, `fill` a selector with text,
//!   `press` a key, `wait` milliseconds, `say` a caption — and, from the step
//!   a screen comes up on, its id as `screen`. `equipe design flows record`
//!   follows it to record the flow's walkthrough: the video, and beside it
//!   `{"millis": 8200, "marks": [{"screen": "login", "at": 3200}]}`, how long
//!   it runs and where each screen starts. Recorded again, the last one moves
//!   to `history/`, named by when it was recorded.
//! - An app that records itself gives its flow a `record_command` instead of
//!   a walk (it wins over one): run as a `command` is, given `out` (an
//!   `.mp4` to write), `mode`, `width` and `flow` (`EQUIPE_RECORD_<NAME>`).
//!   A `.json` it writes beside `out`, `{"millis": …, "marks": […]}`, is kept
//!   beside the video.
//!
//! What can't be read is said, a line each, and left out; the rest still
//! shows. Fields the file has that aren't these are passed over.
//!
//! The setup (`setup`) is read from the same folder: the brief, the logo, the
//! system, the map and design QA, each done when its files are there.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use equipe_protocol::TicketId;
use equipe_protocol::design::{Flow, FlowMap, Mark, Picture, Screen, ScreenState, Setup, SetupStep, Walkthrough};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::folder;
use crate::i18n::{Catalog, Lang};
use crate::visuals;

/// The map's folder in the design folder, and its file.
pub const DIR: &str = "flows";
pub const FILE: &str = "flows.json";
/// The biggest `flows.json` read.
pub const FILE_MAX: u64 = 1 << 20;
/// A flow's folder of earlier walkthroughs.
pub const HISTORY: &str = "history";
/// The biggest walkthrough shown, and the biggest note beside one read.
pub const VIDEO_MAX: u64 = 2 << 30;
const SIDECAR_MAX: u64 = 64 << 10;
/// How many files in `flows/` are looked at for pictures and walkthroughs.
const FILES_MAX: usize = 5000;
/// How deep design QA's reports are looked for.
const QA_DEPTH: usize = 4;
/// The actions a step of a walk may take, one each.
const ACTIONS: [&str; 6] = ["go", "click", "fill", "press", "wait", "say"];

/// `flows.json` as the designer writes it. Each list is read an entry at a
/// time (`T` a JSON value at first), so a bad one is told and left out while
/// the rest still show.
#[derive(Deserialize, Serialize)]
struct MapFile<T> {
    #[serde(default)]
    flows: Vec<T>,
}

#[derive(Deserialize, Serialize)]
struct FlowFile<T> {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    screens: Vec<T>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    walk: Vec<Value>,
    /// What shoots its states when neither they nor their screens say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    /// What records it, for an app that records itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    record_command: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct ScreenFile<T> {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    source: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ticket: Option<TicketId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    expects: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    states: Vec<T>,
}

#[derive(Deserialize, Serialize)]
struct StateFile {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<String>,
}

/// The note beside a walkthrough.
#[derive(Default, Deserialize)]
struct Sidecar {
    #[serde(default)]
    millis: u64,
    #[serde(default)]
    marks: Vec<Mark>,
}

/// A flow as `flows.json` lists it, every entry read.
struct Listed {
    file: FlowFile<ScreenFile<StateFile>>,
    walk: Vec<WalkStep>,
    /// What is wrong with its walk: among the map's problems too.
    walk_problems: Vec<String>,
}

/// Whether `id` can name a folder: lowercase letters, digits and `-`.
fn is_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The entries of a list in `flows.json`, in order: each read, or the line
/// that says why it can't be — not what it should be, an id that isn't one,
/// an id said before. `parent` is where the list is (`onboarding/login`).
fn entries<T: DeserializeOwned>(values: Vec<Value>, parent: &str, words: &Catalog, id: impl Fn(&T) -> &str) -> Vec<Result<T, String>> {
    let at = |end: &str| if parent.is_empty() { end.to_owned() } else { format!("{parent}/{end}") };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (n, value) in values.into_iter().enumerate() {
        let place = at(&format!("#{}", n + 1));
        let entry = if value.is_object() { serde_json::from_value::<T>(value).ok() } else { None };
        let Some(entry) = entry else {
            out.push(Err(words.fill("flows.problem_entry", &[("at", place)])));
            continue;
        };
        let key = id(&entry).to_owned();
        out.push(if !is_id(&key) {
            Err(words.fill("flows.problem_bad_id", &[("at", place), ("id", key)]))
        } else if !seen.insert(key.clone()) {
            Err(words.fill("flows.problem_duplicate", &[("at", at(&key))]))
        } else {
            Ok(entry)
        });
    }
    out
}

/// A step of a walk, as written: one action, and maybe the screen it brings up.
fn step_of(value: &Value) -> Option<WalkStep> {
    let object = value.as_object()?;
    let screen = match object.get("screen") {
        None => None,
        Some(Value::String(screen)) => Some(screen.clone()),
        Some(_) => return None,
    };
    let mut actions = object.iter().filter(|(key, _)| ACTIONS.contains(&key.as_str()));
    let (action, arg) = actions.next()?;
    if actions.next().is_some() {
        return None;
    }
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let step = match action.as_str() {
        "go" => Step::Go(text(arg)?),
        "click" => Step::Click(text(arg)?),
        "fill" => match arg.as_array().map(Vec::as_slice) {
            Some([selector, value]) => Step::Fill(text(selector)?, text(value)?),
            _ => return None,
        },
        "press" => Step::Press(text(arg)?),
        "wait" => Step::Wait(arg.as_u64()?),
        _ => Step::Say(text(arg)?),
    };
    Some(WalkStep { screen, step })
}

/// Flow `flow`'s walk, each bad step told in `problems` and left out.
fn steps(values: &[Value], flow: &str, screens: &[ScreenFile<StateFile>], words: &Catalog, problems: &mut Vec<String>) -> Vec<WalkStep> {
    let mut out = Vec::new();
    for (n, value) in values.iter().enumerate() {
        let at = format!("{flow}/walk/#{}", n + 1);
        match step_of(value) {
            None => problems.push(words.fill("flows.problem_step", &[("at", at)])),
            Some(WalkStep { screen: Some(screen), .. }) if !screens.iter().any(|s| s.id == screen) => {
                problems.push(words.fill("flows.problem_step_screen", &[("at", at), ("screen", screen)]));
            }
            Some(step) => out.push(step),
        }
    }
    out
}

/// `flows/flows.json` in `design`, read: what can't be is told in `problems`
/// and left out. `None` when there is none, or it is a link out of `design`.
fn parse(design: &Path, words: &Catalog, problems: &mut Vec<String>) -> Option<Vec<Listed>> {
    let name = format!("{DIR}/{FILE}");
    let bytes = folder::inside(design, &name, FILE_MAX)?;
    if bytes.len() as u64 > FILE_MAX {
        problems.push(words.fill("design.problem_too_big", &[("file", name), ("limit", format!("{} MB", FILE_MAX >> 20))]));
        return Some(Vec::new());
    }
    let value = match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) => value,
        Err(e) => {
            problems.push(words.fill("flows.problem_not_json", &[("line", e.line().to_string()), ("column", e.column().to_string())]));
            return Some(Vec::new());
        }
    };
    let file = if value.is_object() { serde_json::from_value::<MapFile<Value>>(value).ok() } else { None };
    let Some(file) = file else {
        problems.push(words.get("flows.problem_not_map").to_owned());
        return Some(Vec::new());
    };
    let mut out = Vec::new();
    for flow in entries::<FlowFile<Value>>(file.flows, "", words, |f| &f.id) {
        let flow = match flow {
            Ok(flow) => flow,
            Err(problem) => {
                problems.push(problem);
                continue;
            }
        };
        let mut screens = Vec::new();
        for screen in entries::<ScreenFile<Value>>(flow.screens, &flow.id, words, |s| &s.id) {
            let screen = match screen {
                Ok(screen) if screen.id == HISTORY => {
                    problems.push(words.fill("flows.problem_reserved", &[("at", format!("{}/{}", flow.id, screen.id))]));
                    continue;
                }
                Ok(screen) => screen,
                Err(problem) => {
                    problems.push(problem);
                    continue;
                }
            };
            let at = format!("{}/{}", flow.id, screen.id);
            let mut states = Vec::new();
            for state in entries::<StateFile>(screen.states, &at, words, |s| &s.id) {
                match state {
                    Ok(state) => states.push(state),
                    Err(problem) => problems.push(problem),
                }
            }
            screens.push(ScreenFile { id: screen.id, title: screen.title, note: screen.note, url: screen.url, command: screen.command, source: screen.source, ticket: screen.ticket, expects: screen.expects, states });
        }
        let mut walk_problems = Vec::new();
        let walk = steps(&flow.walk, &flow.id, &screens, words, &mut walk_problems);
        problems.extend(walk_problems.iter().cloned());
        out.push(Listed { file: FlowFile { id: flow.id, title: flow.title, summary: flow.summary, screens, walk: Vec::new(), command: flow.command, record_command: flow.record_command }, walk, walk_problems });
    }
    Some(out)
}

/// The files under `dir` (not a link), each by the names of its folders and
/// its own: no deeper than `depth` names, no hidden ones, links listed and
/// not followed; sorted.
fn files_in(dir: &Path, depth: usize) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    if !std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) {
        return out;
    }
    let mut stack: Vec<(PathBuf, Vec<String>)> = vec![(dir.to_path_buf(), Vec::new())];
    while let Some((at, names)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || out.len() >= FILES_MAX {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            let mut parts = names.clone();
            parts.push(name);
            if !meta.is_dir() {
                out.push(parts);
            } else if parts.len() < depth {
                stack.push((entry.path(), parts));
            }
        }
    }
    out.sort();
    out
}

/// A file's time in Unix seconds.
fn secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// What is in `root`'s `flows/` (`root` a design folder, its real path).
#[derive(Default)]
struct Found {
    /// Each listed state's pictures, by flow, screen and state, with their times.
    pictures: HashMap<[String; 3], Vec<(Picture, SystemTime)>>,
    /// Each listed flow's walkthroughs, with their times.
    videos: HashMap<String, Vec<(Walkthrough, SystemTime)>>,
    /// Pictures of no listed state.
    unlisted: Vec<String>,
}

/// The pictures and walkthroughs in `root`'s `flows/`, of the flows `listed`:
/// only what is what its name says and stays inside `root`.
fn find(root: &Path, listed: &[Listed]) -> Found {
    let states: HashSet<[&str; 3]> = listed
        .iter()
        .flat_map(|f| f.file.screens.iter().flat_map(move |s| s.states.iter().map(move |st| [f.file.id.as_str(), s.id.as_str(), st.id.as_str()])))
        .collect();
    let flows: HashSet<&str> = listed.iter().map(|f| f.file.id.as_str()).collect();
    let roots = [root.to_path_buf()];
    let base = root.join(DIR);
    let mut found = Found::default();
    for parts in files_in(&base, 3) {
        let path = parts.iter().fold(base.clone(), |path, part| path.join(part));
        let name = parts.last().map(String::as_str).unwrap_or_default();
        if let Some((stamp, mode, width)) = parse_video_name(name) {
            let current = match parts.as_slice() {
                [flow, _] if stamp.is_none() && flows.contains(flow.as_str()) => true,
                [flow, kept, _] if stamp.is_some() && kept == HISTORY && flows.contains(flow.as_str()) => false,
                _ => continue,
            };
            let Ok(real) = std::fs::canonicalize(&path) else { continue };
            let Ok(meta) = std::fs::metadata(&real) else { continue };
            if !real.starts_with(root) || !meta.is_file() || meta.len() > VIDEO_MAX {
                continue;
            }
            let Ok(modified) = meta.modified() else { continue };
            let note = format!("{DIR}/{}.json", parts.join("/").trim_end_matches(".mp4"));
            let sidecar = folder::inside(root, &note, SIDECAR_MAX).filter(|b| b.len() as u64 <= SIDECAR_MAX).and_then(|b| serde_json::from_slice::<Sidecar>(&b).ok()).unwrap_or_default();
            let video = Walkthrough { path: path.to_string_lossy().into_owned(), mode, width, taken: secs(modified), millis: sidecar.millis, marks: sidecar.marks, current };
            found.videos.entry(parts[0].clone()).or_default().push((video, modified));
            continue;
        }
        if !visuals::is_image_name(name) {
            continue;
        }
        let picture = match parts.as_slice() {
            [flow, screen, file] => parse_picture_name(file).filter(|(state, ..)| states.contains(&[flow.as_str(), screen.as_str(), state.as_str()])),
            _ => None,
        };
        let Some((state, mode, width)) = picture else {
            found.unlisted.push(parts.join("/"));
            continue;
        };
        let Ok(real) = visuals::check(&path, &roots) else { continue };
        let Ok(modified) = std::fs::metadata(&real).and_then(|m| m.modified()) else { continue };
        let picture = Picture { path: path.to_string_lossy().into_owned(), mode, width, taken: secs(modified), hash: None };
        found.pictures.entry([parts[0].clone(), parts[1].clone(), state]).or_default().push((picture, modified));
    }
    found
}

/// When `source`, in `repo`, was last changed: `Err` when it is outside
/// `repo` (or a link out of it), `Ok(None)` when it isn't there.
fn changed(repo: &Path, source: &str) -> Result<Option<SystemTime>, ()> {
    let rel = Path::new(source);
    if rel.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(());
    }
    let (Ok(root), Ok(real)) = (std::fs::canonicalize(repo), std::fs::canonicalize(repo.join(rel))) else { return Ok(None) };
    if !real.starts_with(&root) {
        return Err(());
    }
    Ok(std::fs::metadata(&real).and_then(|m| m.modified()).ok())
}

/// The map in `design` (a design folder), its pictures found and its
/// missing and stale screens worked out; `repo`, the repository it is in
/// (`None` for the squad's own folder: nothing is stale). `None` when there
/// is no `flows/flows.json`.
pub fn read(design: &Path, repo: Option<&Path>, lang: Lang) -> Option<FlowMap> {
    let words = crate::i18n::core(lang);
    let mut map = FlowMap::default();
    let listed = parse(design, words, &mut map.problems)?;
    let root = std::fs::canonicalize(design).ok()?;
    let mut found = find(&root, &listed);
    for Listed { file: flow, .. } in listed {
        let mut screens = Vec::new();
        for screen in flow.screens {
            let mut newest: Option<SystemTime> = None;
            let states: Vec<ScreenState> = screen
                .states
                .into_iter()
                .map(|state| {
                    let mut pictures = found.pictures.remove(&[flow.id.clone(), screen.id.clone(), state.id.clone()]).unwrap_or_default();
                    pictures.sort_by_key(|(p, _)| (p.mode != "light", p.width));
                    newest = newest.max(pictures.iter().map(|(_, at)| *at).max());
                    ScreenState {
                        label: state.label.unwrap_or_else(|| state.id.clone()),
                        kind: state.kind.unwrap_or_else(|| state.id.clone()),
                        note: state.note,
                        pictures: pictures.into_iter().map(|(p, _)| p).collect(),
                        id: state.id,
                    }
                })
                .collect();
            // Expected and not among the states, then each state with no picture.
            let mut missing: Vec<String> = Vec::new();
            for want in &screen.expects {
                if !states.iter().any(|s| &s.id == want || &s.kind == want) && !missing.contains(want) {
                    missing.push(want.clone());
                }
            }
            for state in states.iter().filter(|s| s.pictures.is_empty()) {
                if !missing.contains(&state.id) {
                    missing.push(state.id.clone());
                }
            }
            let mut stale = false;
            if let Some(repo) = repo {
                for source in &screen.source {
                    match changed(repo, source) {
                        Err(()) => map.problems.push(words.fill("flows.problem_source_outside", &[("at", format!("{}/{}", flow.id, screen.id)), ("path", source.clone())])),
                        Ok(Some(at)) => stale |= newest.is_some_and(|newest| at > newest),
                        Ok(None) => {}
                    }
                }
            }
            let title = screen.title.unwrap_or_else(|| screen.id.clone());
            screens.push(Screen { id: screen.id, title, note: screen.note, url: screen.url, source: screen.source, ticket: screen.ticket, states, missing, stale });
        }
        let mut videos = found.videos.remove(&flow.id).unwrap_or_default();
        videos.sort_by_key(|(v, at)| (!v.current, std::cmp::Reverse(*at), v.mode != "light", v.width));
        let title = flow.title.unwrap_or_else(|| flow.id.clone());
        map.flows.push(Flow { id: flow.id, title, summary: flow.summary, screens, videos: videos.into_iter().map(|(v, _)| v).collect() });
    }
    map.unlisted = found.unlisted;
    Some(map)
}

/// How far the designer's first setup has come in `design`: every step of
/// `Setup`, in order, each done when its files are there — the brief, a
/// logo, the tokens and the guidelines, a map with a flow, a QA report.
pub fn setup(design: &Path) -> Vec<SetupStep> {
    let has = |name: &str| folder::within(design, name).is_some();
    let logo = folder::assets(design).iter().any(|a| a.name.starts_with("assets/logo/"));
    let flows = parse(design, crate::i18n::core(Lang::En), &mut Vec::new()).is_some_and(|flows| !flows.is_empty());
    let qa = !files_in(&design.join(folder::QA), QA_DEPTH).is_empty();
    [(Setup::Brief, has(folder::BRIEF)), (Setup::Logo, logo), (Setup::System, has(folder::TOKENS) && has(folder::GUIDELINES)), (Setup::Flows, flows), (Setup::Qa, qa)]
        .into_iter()
        .map(|(step, done)| SetupStep { step, done })
        .collect()
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

/// A flow's walkthrough's file name: `walkthrough.light.390.mp4`. The note
/// beside it is named the same, with `.json`.
pub fn video_name(mode: &str, width: u32) -> String {
    format!("walkthrough.{mode}.{width}.mp4")
}

/// An earlier walkthrough's name in `history/`, by when it was recorded
/// (`YYYYMMDD-HHMMSS`): `walkthrough.20261104-051200.light.390.mp4`.
pub fn history_name(stamp: &str, mode: &str, width: u32) -> String {
    format!("walkthrough.{stamp}.{mode}.{width}.mp4")
}

/// A walkthrough's stamp (an earlier one's), mode and width, from its file
/// name; `None` when it isn't one.
pub fn parse_video_name(name: &str) -> Option<(Option<String>, String, u32)> {
    let rest = name.strip_prefix("walkthrough.")?.strip_suffix(".mp4")?;
    let mut parts = rest.rsplitn(3, '.');
    let width = parts.next()?.parse().ok()?;
    let mode = parts.next()?;
    let stamp = parts.next();
    let is_stamp = |s: &str| s.len() == 15 && s.bytes().enumerate().all(|(i, b)| if i == 8 { b == b'-' } else { b.is_ascii_digit() });
    (matches!(mode, "light" | "dark") && stamp.is_none_or(is_stamp)).then(|| (stamp.map(str::to_owned), mode.to_owned(), width))
}

/// What `equipe design flows check` finds wrong with a map: where (a flow,
/// maybe a screen, maybe a state — or, for a picture of none, the names on
/// disk) and what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub flow: String,
    pub screen: Option<String>,
    /// The state, or the kind or id expected; for a picture of no listed
    /// state, the state its name says (else its file name).
    pub state: Option<String>,
    pub kind: FindingKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// A state the screen should have and has no picture of.
    MissingState,
    /// The screen's code changed after its newest picture.
    Stale,
    /// A light picture with no dark one at its width.
    NoDark,
    /// A picture of no state `flows.json` lists.
    Unlisted,
    /// A screen with no picture at all.
    NoPictures,
}

/// What is wrong with `map`, screen by screen in its order, then the
/// pictures of no listed state. A screen never pictured is said once, not
/// once for each of its states.
pub fn check(map: &FlowMap) -> Vec<Finding> {
    let mut out = Vec::new();
    for flow in &map.flows {
        for screen in &flow.screens {
            let finding = |state: Option<&str>, kind| Finding { flow: flow.id.clone(), screen: Some(screen.id.clone()), state: state.map(str::to_owned), kind };
            let unpictured = screen.states.iter().all(|s| s.pictures.is_empty());
            if unpictured {
                out.push(finding(None, FindingKind::NoPictures));
            }
            for missing in &screen.missing {
                if !(unpictured && screen.states.iter().any(|s| &s.id == missing)) {
                    out.push(finding(Some(missing), FindingKind::MissingState));
                }
            }
            if screen.stale {
                out.push(finding(None, FindingKind::Stale));
            }
            for state in &screen.states {
                let dark = |width: u32| state.pictures.iter().any(|p| p.mode == "dark" && p.width == width);
                if state.pictures.iter().any(|p| p.mode == "light" && !dark(p.width)) {
                    out.push(finding(Some(&state.id), FindingKind::NoDark));
                }
            }
        }
    }
    for rel in &map.unlisted {
        let parts: Vec<&str> = rel.split('/').collect();
        let (folders, file) = parts.split_at(parts.len().saturating_sub(1));
        let file = file.first().copied().unwrap_or_default();
        let finding = Finding {
            flow: folders.first().map_or_else(String::new, |f| (*f).to_owned()),
            screen: folders.get(1).map(|s| (*s).to_owned()),
            state: Some(parse_picture_name(file).map_or_else(|| file.to_owned(), |(state, ..)| state)),
            kind: FindingKind::Unlisted,
        };
        if !out.contains(&finding) {
            out.push(finding);
        }
    }
    out
}

/// A `flows.json` to start from, in `lang`: one flow, one screen in two
/// states with a third expected, and a walk through it.
pub fn starter(lang: Lang) -> String {
    let words = crate::i18n::core(lang);
    let word = |key: &str| Some(words.get(key).to_owned());
    let url = "http://localhost:3000/login";
    let state = |id: &str, label: &str, note: Option<&str>, url: Option<String>| StateFile { id: id.into(), label: word(label), kind: Some(id.into()), note: note.and_then(word), url, command: None };
    let screen = ScreenFile {
        id: "login".into(),
        title: word("flows.starter_screen"),
        note: word("flows.starter_screen_note"),
        url: Some(url.into()),
        command: None,
        source: vec!["src/app/login/page.tsx".into()],
        ticket: None,
        expects: vec!["error".into(), "loading".into()],
        states: vec![
            state("default", "flows.starter_state_default", None, None),
            state("error", "flows.starter_state_error", Some("flows.starter_state_error_note"), Some(format!("{url}?fixture=wrong-password"))),
        ],
    };
    let walk = vec![
        serde_json::json!({ "go": url, "screen": "login" }),
        serde_json::json!({ "say": words.get("flows.starter_say") }),
        serde_json::json!({ "wait": 1500 }),
    ];
    let flow = FlowFile { id: "onboarding".into(), title: word("flows.starter_flow"), summary: word("flows.starter_flow_summary"), screens: vec![screen], walk, command: None, record_command: None };
    let mut text = serde_json::to_string_pretty(&MapFile { flows: vec![flow] }).unwrap_or_default();
    text.push('\n');
    text
}

/// A state to shoot: where it runs, and the folder its pictures go in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    pub flow: String,
    pub screen: String,
    pub state: String,
    /// The state's own `url`, else its screen's.
    pub url: Option<String>,
    /// The state's own `command`, else its screen's, else its flow's: what
    /// runs instead of opening `url` when there is one.
    pub command: Option<String>,
    /// `<design>/flows/<flow>/<screen>`: each picture is `picture_name` in it.
    pub dir: PathBuf,
}

/// Every state `design`'s map lists, in its order, to shoot.
pub fn shots(design: &Path) -> Vec<Shot> {
    let Some(listed) = parse(design, crate::i18n::core(Lang::En), &mut Vec::new()) else { return Vec::new() };
    let mut out = Vec::new();
    for Listed { file: flow, .. } in listed {
        let flow_command = given(flow.command);
        for screen in flow.screens {
            let dir = design.join(DIR).join(&flow.id).join(&screen.id);
            let screen_command = given(screen.command).or_else(|| flow_command.clone());
            for state in screen.states {
                let url = state.url.or_else(|| screen.url.clone());
                let command = given(state.command).or_else(|| screen_command.clone());
                out.push(Shot { flow: flow.id.clone(), screen: screen.id.clone(), state: state.id, url, command, dir: dir.clone() });
            }
        }
    }
    out
}

/// A command as written, `None` when blank.
fn given(command: Option<String>) -> Option<String> {
    command.filter(|c| !c.trim().is_empty())
}

/// What a shot's command is given: each placeholder by name, the PNG to
/// write first. In its environment each is `EQUIPE_SHOT_<NAME>`.
pub fn shot_values(out: &Path, mode: &str, width: u32, shot: &Shot) -> Vec<(&'static str, String)> {
    vec![("out", out.to_string_lossy().into_owned()), ("mode", mode.to_owned()), ("width", width.to_string()), ("flow", shot.flow.clone()), ("screen", shot.screen.clone()), ("state", shot.state.clone())]
}

/// What a flow's `record_command` is given (`EQUIPE_RECORD_<NAME>`): the
/// video to write first.
pub fn record_values(out: &Path, mode: &str, width: u32, flow: &str) -> Vec<(&'static str, String)> {
    vec![("out", out.to_string_lossy().into_owned()), ("mode", mode.to_owned()), ("width", width.to_string()), ("flow", flow.to_owned())]
}

/// `values` as environment variables, each `<prefix><NAME>`
/// (`EQUIPE_SHOT_OUT`).
pub fn env_of(prefix: &str, values: &[(&str, String)]) -> Vec<(String, String)> {
    values.iter().map(|(name, value)| (format!("{prefix}{}", name.to_ascii_uppercase()), value.clone())).collect()
}

/// `command` with each `{name}` of `values` put in, quoted for the shell, in
/// one pass: what is put in is never read again, and braces naming nothing
/// (`${HOME}`, `{}`) are left as they are.
pub fn fill(command: &str, values: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(command.len());
    let mut rest = command;
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let named = after.find('}').and_then(|end| values.iter().find(|(name, _)| *name == &after[..end]).map(|(_, value)| (end, value)));
        match named {
            Some((end, value)) => {
                out.push_str(&shell_quote(value));
                rest = &after[end + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `text` as one word to `sh`: bare when it is only letters, digits and
/// `-_./:=@%+,`, else in single quotes, each quote in it closed, escaped and
/// opened again.
pub fn shell_quote(text: &str) -> String {
    let plain = !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./:=@%+,".contains(&b));
    if plain { text.to_owned() } else { format!("'{}'", text.replace('\'', "'\\''")) }
}

/// One thing a walk does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Open a URL.
    Go(String),
    /// Click what a selector finds.
    Click(String),
    /// Type text into what a selector finds.
    Fill(String, String),
    /// Press a key: `Enter`.
    Press(String),
    /// Wait, in milliseconds.
    Wait(u64),
    /// Show a caption.
    Say(String),
}

/// A step of a flow's walk, and the screen that shows from it on, if it
/// brings one up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkStep {
    pub screen: Option<String>,
    pub step: Step,
}

/// Flow `flow`'s walk in `design`, to record it. `Err`, in `lang`, a line
/// for each reason: no map, no such flow, no walk, a bad step.
pub fn walk(design: &Path, flow: &str, lang: Lang) -> Result<Vec<WalkStep>, String> {
    let words = crate::i18n::core(lang);
    let found = listed_flow(design, flow, words)?;
    if !found.walk_problems.is_empty() {
        return Err(found.walk_problems.join("\n"));
    }
    if found.walk.is_empty() {
        return Err(words.fill("flows.walk_empty", &[("flow", flow.to_owned())]));
    }
    Ok(found.walk)
}

/// Flow `flow` as `design`'s map lists it; `Err` when there is no map or no
/// such flow, with what else couldn't be read.
fn listed_flow(design: &Path, flow: &str, words: &Catalog) -> Result<Listed, String> {
    let mut problems = Vec::new();
    let Some(listed) = parse(design, words, &mut problems) else {
        return Err(words.fill("flows.walk_no_map", &[("path", design.join(DIR).join(FILE).to_string_lossy().into_owned())]));
    };
    let Some(found) = listed.into_iter().find(|l| l.file.id == flow) else {
        problems.push(words.fill("flows.walk_no_flow", &[("flow", flow.to_owned())]));
        return Err(problems.join("\n"));
    };
    Ok(found)
}

/// How a flow is recorded: by its own `record_command`, or its walk
/// followed in Chrome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recording {
    Command(String),
    Walk(Vec<WalkStep>),
}

/// How flow `flow` in `design` is recorded: its `record_command` when it
/// has one, else its walk. `Err`, in `lang`, as `walk` says it — and, when
/// it has neither, that an app that isn't a web page records itself by a
/// `record_command`.
pub fn recording(design: &Path, flow: &str, lang: Lang) -> Result<Recording, String> {
    let words = crate::i18n::core(lang);
    let found = listed_flow(design, flow, words)?;
    if let Some(command) = given(found.file.record_command) {
        return Ok(Recording::Command(command));
    }
    if !found.walk_problems.is_empty() {
        return Err(found.walk_problems.join("\n"));
    }
    if found.walk.is_empty() {
        return Err(words.fill("flows.record_none", &[("flow", flow.to_owned())]));
    }
    Ok(Recording::Walk(found.walk))
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::design::{Flow, Mark, Picture, Screen, ScreenState, Setup};
    use std::os::unix::fs::symlink;
    use std::time::{Duration, SystemTime};

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-flows-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn png(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([20, 90, 250, 255])).save(path).unwrap();
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn touch(path: &Path, at: SystemTime) {
        std::fs::File::options().write(true).open(path).unwrap().set_modified(at).unwrap();
    }

    fn shape(state: &ScreenState) -> Vec<(&str, u32)> {
        state.pictures.iter().map(|p| (p.mode.as_str(), p.width)).collect()
    }

    const MAP: &str = r#"{ "flows": [
      { "id": "onboarding", "title": "Onboarding", "summary": "First launch to first project", "colour": "ignored",
        "screens": [
          { "id": "login", "title": "Sign in", "note": "One line under the field", "url": "http://localhost:3000/login",
            "source": ["src/login.tsx"], "ticket": 142, "expects": ["empty", "error", "loading"],
            "states": [
              { "id": "default", "label": "Default", "kind": "default" },
              { "id": "error", "label": "Error", "kind": "error", "url": "http://localhost:3000/login?fixture=wrong" },
              { "id": "loading-slow", "label": "Loading", "kind": "loading" },
              { "id": "Error", "label": "Shouting" },
              { "id": "error", "label": "Again" } ] },
          7 ] },
      { "id": "Bad_Id", "title": "Bad" },
      { "id": "onboarding", "title": "Twice" },
      { "title": "No id" } ] }"#;

    #[test]
    fn the_map_is_read_with_its_pictures_what_is_missing_and_a_line_for_each_bad_entry() {
        let root = dir("read");
        let design = root.join("design");
        assert_eq!(read(&design, None, Lang::En), None, "no folder");
        std::fs::create_dir_all(&design).unwrap();
        assert_eq!(read(&design, None, Lang::En), None, "no flows.json");
        write(&design.join("flows/flows.json"), MAP);
        let login = design.join("flows/onboarding/login");
        png(&login.join("default.dark.390.png"));
        png(&login.join("default.light.1280.png"));
        png(&login.join("default.light.390.png"));
        png(&login.join("error.light.390.png"));
        write(&login.join("error.dark.390.png"), "not a picture");
        png(&root.join("outside.png"));
        symlink(root.join("outside.png"), login.join("default.dark.1280.png")).unwrap();
        png(&login.join("eror.light.390.png"));
        write(&login.join("notes.txt"), "not a picture either");
        png(&design.join("flows/gone/home/default.light.390.png"));
        let map = read(&design, None, Lang::En).unwrap();
        assert_eq!(map.flows.len(), 1, "{map:?}");
        let flow = &map.flows[0];
        assert_eq!((flow.id.as_str(), flow.title.as_str(), flow.summary.as_deref()), ("onboarding", "Onboarding", Some("First launch to first project")));
        assert_eq!(flow.screens.len(), 1);
        let screen = &flow.screens[0];
        assert_eq!((screen.id.as_str(), screen.url.as_deref(), screen.ticket), ("login", Some("http://localhost:3000/login"), Some(142)));
        assert_eq!(screen.source, ["src/login.tsx"]);
        let ids: Vec<&str> = screen.states.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["default", "error", "loading-slow"]);
        assert_eq!(shape(&screen.states[0]), [("light", 390), ("light", 1280), ("dark", 390)], "light first, then by width; a link out of the folder isn't one");
        assert_eq!(shape(&screen.states[1]), [("light", 390), ("dark", 390)], "named a picture, it is listed, as an asset is");
        assert!(visuals::read(Path::new(&screen.states[1].pictures[1].path), std::slice::from_ref(&design), None).is_err(), "read, its bytes are refused");
        assert!(screen.states[2].pictures.is_empty());
        let first = &screen.states[0].pictures[0];
        assert_eq!(first.path, login.join("default.light.390.png").to_string_lossy());
        assert!(first.taken > 1_600_000_000);
        assert_eq!(screen.missing, ["empty", "loading-slow"], "an expected kind with no state, then each state with no picture");
        assert!(!screen.stale, "no repository: never stale");
        assert_eq!(map.unlisted, ["gone/home/default.light.390.png", "onboarding/login/eror.light.390.png"]);
        assert_eq!(map.problems.len(), 6, "{:#?}", map.problems);
        for (problem, says) in map.problems.iter().zip(["onboarding/login/#4", "onboarding/login/error", "onboarding/#2", "Bad_Id", "onboarding", "#4"]) {
            assert!(problem.contains(says), "{problem} should name {says}");
        }
        let ko = read(&design, None, Lang::Ko).unwrap();
        assert!(ko.problems.iter().all(|p| p.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c))), "{:?}", ko.problems);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_file_that_cant_be_read_says_why_and_one_that_is_a_link_out_isnt_read() {
        let root = dir("broken");
        let design = root.join("design");
        write(&design.join("flows/flows.json"), "{ \"flows\": [ oops");
        let map = read(&design, None, Lang::En).unwrap();
        assert!(map.flows.is_empty());
        assert_eq!(map.problems.len(), 1);
        assert!(map.problems[0].contains("JSON") && map.problems[0].contains('1'), "{:?}", map.problems);
        write(&design.join("flows/flows.json"), "[1, 2]");
        assert_eq!(read(&design, None, Lang::En).unwrap().problems.len(), 1);
        write(&design.join("flows/flows.json"), &" ".repeat(FILE_MAX as usize + 1));
        let big = read(&design, None, Lang::En).unwrap();
        assert!(big.problems[0].contains("flows.json") && big.problems[0].contains("1 MB"), "{:?}", big.problems);
        std::fs::remove_file(design.join("flows/flows.json")).unwrap();
        write(&root.join("elsewhere.json"), r#"{ "flows": [ { "id": "a", "title": "A" } ] }"#);
        symlink(root.join("elsewhere.json"), design.join("flows/flows.json")).unwrap();
        assert_eq!(read(&design, None, Lang::En), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_screen_is_stale_when_its_code_changed_after_its_newest_picture_and_only_code_in_the_repository_counts() {
        let repo = dir("stale");
        let design = repo.join("design");
        write(
            &design.join("flows/flows.json"),
            r#"{ "flows": [ { "id": "app", "title": "App", "screens": [
                 { "id": "home", "title": "Home", "source": ["src/home.tsx", "src/gone.tsx"], "states": [ { "id": "default", "label": "Default", "kind": "default" } ] },
                 { "id": "out", "title": "Out", "source": ["../secret.tsx", "/etc/hosts", "src/link.tsx"], "states": [ { "id": "default", "label": "Default", "kind": "default" } ] } ] } ] }"#,
        );
        let now = SystemTime::now();
        let picture = design.join("flows/app/home/default.light.390.png");
        png(&picture);
        png(&design.join("flows/app/out/default.light.390.png"));
        touch(&picture, now);
        touch(&design.join("flows/app/out/default.light.390.png"), now - Duration::from_secs(3600));
        write(&repo.join("src/home.tsx"), "old");
        touch(&repo.join("src/home.tsx"), now - Duration::from_secs(60));
        let secret = repo.parent().unwrap().join(format!("equipe-flows-secret-{}.tsx", std::process::id()));
        write(&secret, "outside");
        symlink(&secret, repo.join("src/link.tsx")).unwrap();
        let screens = |map: &FlowMap| map.flows[0].screens.iter().map(|s| s.stale).collect::<Vec<_>>();
        let map = read(&design, Some(&repo), Lang::En).unwrap();
        assert_eq!(screens(&map), [false, false], "older code; code outside never counts, though newer");
        assert_eq!(map.problems.len(), 3, "each source outside the repository says so: {:#?}", map.problems);
        assert!(map.problems.iter().any(|p| p.contains("../secret.tsx")));
        touch(&repo.join("src/home.tsx"), now + Duration::from_secs(60));
        assert_eq!(screens(&read(&design, Some(&repo), Lang::En).unwrap()), [true, false]);
        let squad = read(&design, None, Lang::En).unwrap();
        assert_eq!(screens(&squad), [false, false], "no repository, nothing stale");
        assert!(squad.problems.is_empty(), "and nothing to say of sources: {:?}", squad.problems);
        std::fs::remove_file(secret).unwrap();
        std::fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn the_setup_has_every_step_in_order_each_done_when_its_files_are_there() {
        let design = dir("setup");
        let done = |design: &Path| setup(design).iter().map(|s| (s.step, s.done)).collect::<Vec<_>>();
        assert_eq!(done(&design), [(Setup::Brief, false), (Setup::Logo, false), (Setup::System, false), (Setup::Flows, false), (Setup::Qa, false)]);
        write(&design.join("brand.md"), "# Brief");
        write(&design.join("assets/logo/readme.txt"), "not a logo");
        write(&design.join("tokens.json"), "{}");
        write(&design.join("flows/flows.json"), r#"{ "flows": [] }"#);
        std::fs::create_dir_all(design.join("qa")).unwrap();
        write(&design.join("qa/.DS_Store"), "");
        assert_eq!(done(&design), [(Setup::Brief, true), (Setup::Logo, false), (Setup::System, false), (Setup::Flows, false), (Setup::Qa, false)]);
        png(&design.join("assets/logo/mark.png"));
        write(&design.join("guidelines.md"), "# Guidelines");
        write(&design.join("flows/flows.json"), r#"{ "flows": [ { "id": "app", "title": "App" } ] }"#);
        write(&design.join("qa/2026-11-04/report.md"), "# QA");
        assert!(done(&design).iter().all(|(_, done)| *done), "{:?}", done(&design));
        std::fs::remove_dir_all(design).unwrap();
    }

    fn picture(mode: &str, width: u32) -> Picture {
        Picture { path: format!("/d/{mode}.{width}.png"), mode: mode.into(), width, taken: 1, hash: None }
    }

    fn state(id: &str, pictures: Vec<Picture>) -> ScreenState {
        ScreenState { id: id.into(), label: id.into(), kind: id.into(), note: None, pictures }
    }

    #[test]
    fn the_check_finds_missing_states_stale_screens_lone_light_pictures_strays_and_screens_never_pictured() {
        let finding = |flow: &str, screen: Option<&str>, state: Option<&str>, kind| Finding { flow: flow.into(), screen: screen.map(Into::into), state: state.map(Into::into), kind };
        let map = FlowMap {
            flows: vec![Flow {
                id: "app".into(),
                title: "App".into(),
                summary: None,
                screens: vec![
                    Screen {
                        id: "home".into(),
                        title: "Home".into(),
                        states: vec![state("default", vec![picture("light", 390), picture("dark", 390), picture("light", 1280)]), state("error", vec![]), state("long", vec![picture("dark", 390)])],
                        missing: vec!["empty".into(), "error".into()],
                        stale: true,
                        ..Screen::default()
                    },
                    Screen { id: "blank".into(), title: "Blank".into(), states: vec![state("default", vec![])], missing: vec!["loading".into(), "default".into()], ..Screen::default() },
                ],
                ..Flow::default()
            }],
            problems: vec![],
            unlisted: vec!["app/home/eror.light.390.png".into(), "app/home/eror.dark.390.png".into(), "old/home/x.png".into(), "app/loose.png".into()],
        };
        assert_eq!(
            check(&map),
            [
                finding("app", Some("home"), Some("empty"), FindingKind::MissingState),
                finding("app", Some("home"), Some("error"), FindingKind::MissingState),
                finding("app", Some("home"), None, FindingKind::Stale),
                finding("app", Some("home"), Some("default"), FindingKind::NoDark),
                finding("app", Some("blank"), None, FindingKind::NoPictures),
                finding("app", Some("blank"), Some("loading"), FindingKind::MissingState),
                finding("app", Some("home"), Some("eror"), FindingKind::Unlisted),
                finding("old", Some("home"), Some("x.png"), FindingKind::Unlisted),
                finding("app", None, Some("loose.png"), FindingKind::Unlisted),
            ]
        );
        assert_eq!(check(&FlowMap::default()), []);
    }

    #[test]
    fn the_starter_is_a_map_with_one_flow_in_the_persons_language() {
        let design = dir("starter");
        for lang in Lang::ALL {
            let text = starter(lang);
            write(&design.join("flows/flows.json"), &text);
            let map = read(&design, None, lang).unwrap();
            assert!(map.problems.is_empty(), "{lang:?}: {:?}", map.problems);
            assert_eq!(map.flows.len(), 1, "{lang:?}");
            let screen = &map.flows[0].screens[0];
            assert!(screen.states.len() >= 2 && !screen.missing.is_empty(), "{lang:?}: it shows what a state and a missing one are");
            assert!(!walk(&design, &map.flows[0].id, lang).unwrap().is_empty(), "{lang:?}: with a walk to record");
            let hangul = text.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c));
            assert_eq!(hangul, lang == Lang::Ko, "{lang:?}: {text}");
            assert!(text.find("\"id\"").unwrap() < text.find("\"screens\"").unwrap(), "written in the order the spec shows");
        }
        assert_ne!(starter(Lang::En), starter(Lang::Fr));
        std::fs::remove_dir_all(design).unwrap();
    }

    #[test]
    fn each_state_is_shot_at_its_own_url_else_its_screens_into_its_screens_folder() {
        let design = dir("shots");
        assert!(shots(&design).is_empty());
        write(&design.join("flows/flows.json"), MAP);
        let found = shots(&design);
        let urls: Vec<(&str, Option<&str>)> = found.iter().map(|s| (s.state.as_str(), s.url.as_deref())).collect();
        assert_eq!(urls, [("default", Some("http://localhost:3000/login")), ("error", Some("http://localhost:3000/login?fixture=wrong")), ("loading-slow", Some("http://localhost:3000/login"))]);
        assert!(found.iter().all(|s| s.flow == "onboarding" && s.screen == "login" && s.dir == design.join("flows/onboarding/login")));
        std::fs::remove_dir_all(design).unwrap();
    }

    const COMMANDS: &str = r#"{ "flows": [
      { "id": "desktop", "title": "Desktop", "command": "scripts/shot.sh {mode} {width} {out} flow", "record_command": "scripts/record.sh {out}",
        "screens": [
          { "id": "board", "title": "Board", "command": "scripts/shot.sh {mode} {width} {out} board", "url": "http://localhost:3000/board",
            "states": [
              { "id": "default", "label": "Default" },
              { "id": "picked", "label": "Picked", "command": "scripts/shot.sh {mode} {width} {out} 'board;pick:2'", "window": "ignored" },
              { "id": "blank", "label": "Blank", "command": "   " } ] },
          { "id": "chat", "title": "Chat", "url": "http://localhost:3000/chat",
            "states": [ { "id": "default", "label": "Default" } ] } ] },
      { "id": "web", "title": "Web",
        "screens": [ { "id": "home", "title": "Home", "url": "http://localhost:3000/",
            "states": [ { "id": "default", "label": "Default" } ] },
          { "id": "sketch", "title": "Sketch", "states": [ { "id": "default", "label": "Default" } ] } ] },
      { "id": "bad", "title": "Bad", "screens": [ { "id": "x", "title": "X", "command": 7, "states": [ { "id": "default" } ] } ] } ] }"#;

    #[test]
    fn a_states_command_beats_its_screens_which_beats_its_flows_and_a_command_is_carried_beside_the_url() {
        let design = dir("commands");
        write(&design.join("flows/flows.json"), COMMANDS);
        let found = shots(&design);
        let got: Vec<(&str, &str, Option<&str>, Option<&str>)> = found.iter().map(|s| (s.screen.as_str(), s.state.as_str(), s.command.as_deref(), s.url.as_deref())).collect();
        assert_eq!(
            got,
            [
                ("board", "default", Some("scripts/shot.sh {mode} {width} {out} board"), Some("http://localhost:3000/board")),
                ("board", "picked", Some("scripts/shot.sh {mode} {width} {out} 'board;pick:2'"), Some("http://localhost:3000/board")),
                ("board", "blank", Some("scripts/shot.sh {mode} {width} {out} board"), Some("http://localhost:3000/board")),
                ("chat", "default", Some("scripts/shot.sh {mode} {width} {out} flow"), Some("http://localhost:3000/chat")),
                ("home", "default", None, Some("http://localhost:3000/")),
                ("sketch", "default", None, None),
            ],
            "the state's, else the screen's, else the flow's; a blank one is none; a screen whose command isn't text is left out"
        );
        let map = read(&design, None, Lang::En).unwrap();
        assert_eq!(map.flows.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["desktop", "web", "bad"]);
        assert!(map.flows[2].screens.is_empty());
        assert_eq!(map.problems.len(), 1, "an unknown field is passed over, a command that isn't text isn't: {:?}", map.problems);
        assert!(map.problems[0].contains("bad/#1"), "{:?}", map.problems);
        std::fs::remove_dir_all(design).unwrap();
    }

    #[test]
    fn placeholders_are_put_in_quoted_for_the_shell_in_one_pass() {
        let shot = Shot { flow: "desktop".into(), screen: "board".into(), state: "it's-{mode}".into(), url: None, command: None, dir: PathBuf::from("/d") };
        let values = shot_values(Path::new("/tmp/My Design/board/default.dark.1280.png"), "dark", 1280, &shot);
        assert_eq!(
            fill("scripts/shot.sh {mode} {width} {out} --state={state} {flow}/{screen}", &values),
            "scripts/shot.sh dark 1280 '/tmp/My Design/board/default.dark.1280.png' --state='it'\\''s-{mode}' desktop/board",
            "a value with a space or quote is quoted; what is put in is never filled again"
        );
        assert_eq!(fill("echo ${HOME} {} {nope} {out", &values), "echo ${HOME} {} {nope} {out", "braces naming nothing are left as they are");
        assert_eq!(fill("{{mode}}", &values), "{dark}");
        assert_eq!(fill("", &values), "");
        assert_eq!((shell_quote("light"), shell_quote(""), shell_quote("a b"), shell_quote("$(rm -rf /)"), shell_quote("x;y")), ("light".into(), "''".into(), "'a b'".into(), "'$(rm -rf /)'".into(), "'x;y'".into()));
        assert_eq!(
            env_of("EQUIPE_SHOT_", &values).iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>(),
            ["EQUIPE_SHOT_OUT=/tmp/My Design/board/default.dark.1280.png", "EQUIPE_SHOT_MODE=dark", "EQUIPE_SHOT_WIDTH=1280", "EQUIPE_SHOT_FLOW=desktop", "EQUIPE_SHOT_SCREEN=board", "EQUIPE_SHOT_STATE=it's-{mode}"],
            "in the environment as they are"
        );
        let record = record_values(Path::new("/w/walkthrough.mp4"), "light", 390, "desktop");
        assert_eq!(fill("rec {out} {mode} {width} {flow} {state}", &record), "rec /w/walkthrough.mp4 light 390 desktop {state}");
        assert_eq!(env_of("EQUIPE_RECORD_", &record)[0], ("EQUIPE_RECORD_OUT".to_owned(), "/w/walkthrough.mp4".to_owned()));
    }

    #[test]
    fn a_flow_is_recorded_by_its_record_command_else_its_walk_and_with_neither_says_so() {
        let design = dir("recording");
        write(&design.join("flows/flows.json"), COMMANDS);
        assert_eq!(recording(&design, "desktop", Lang::En), Ok(Recording::Command("scripts/record.sh {out}".into())));
        let why = recording(&design, "web", Lang::En).unwrap_err();
        assert!(why.contains("web") && why.contains("record_command") && why.contains("walk"), "{why}");
        assert!(recording(&design, "web", Lang::Ko).unwrap_err().chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c)));
        assert!(recording(&design, "gone", Lang::En).unwrap_err().contains("gone"));
        write(&design.join("flows/flows.json"), WALKED);
        assert!(matches!(recording(&design, "onboarding", Lang::En), Ok(Recording::Walk(steps)) if steps.len() == 6));
        std::fs::remove_dir_all(design).unwrap();
    }

    const WALKED: &str = r##"{ "flows": [ { "id": "onboarding", "title": "Onboarding",
        "walk": [
          { "go": "http://localhost:3000/login", "screen": "login" },
          { "fill": ["#email", "ada@example.com"] },
          { "click": "button[type=submit]" },
          { "press": "Enter" },
          { "wait": 500 },
          { "say": "A wrong password", "screen": "home" } ],
        "screens": [ { "id": "login", "title": "Sign in" }, { "id": "home", "title": "Home" } ] } ] }"##;

    #[test]
    fn a_flows_walk_is_its_steps_each_one_action_and_maybe_the_screen_it_brings_up() {
        let design = dir("walk");
        assert!(walk(&design, "onboarding", Lang::En).is_err(), "no map");
        write(&design.join("flows/flows.json"), WALKED);
        let steps = walk(&design, "onboarding", Lang::En).unwrap();
        let step = |screen: Option<&str>, step| WalkStep { screen: screen.map(Into::into), step };
        assert_eq!(
            steps,
            [
                step(Some("login"), Step::Go("http://localhost:3000/login".into())),
                step(None, Step::Fill("#email".into(), "ada@example.com".into())),
                step(None, Step::Click("button[type=submit]".into())),
                step(None, Step::Press("Enter".into())),
                step(None, Step::Wait(500)),
                step(Some("home"), Step::Say("A wrong password".into())),
            ]
        );
        assert!(read(&design, None, Lang::En).unwrap().problems.is_empty());
        assert!(walk(&design, "checkout", Lang::En).unwrap_err().contains("checkout"), "no such flow");
        write(&design.join("flows/flows.json"), r#"{ "flows": [ { "id": "onboarding", "title": "Onboarding" } ] }"#);
        assert!(walk(&design, "onboarding", Lang::En).unwrap_err().contains("onboarding"), "no walk");
        write(
            &design.join("flows/flows.json"),
            r##"{ "flows": [ { "id": "onboarding", "title": "Onboarding", "screens": [ { "id": "login", "title": "Sign in" } ],
                 "walk": [ { "go": "http://localhost:3000", "click": "a" }, { "wait": 10, "screen": "nowhere" }, { "fill": ["#only"] }, { "dance": true }, "go", { "wait": 10 } ] } ] }"##,
        );
        let why = walk(&design, "onboarding", Lang::En).unwrap_err();
        assert_eq!(why.lines().count(), 5, "a line for each bad step: {why}");
        assert!(why.contains("onboarding/walk/#1") && why.contains("nowhere") && why.contains("#5"), "{why}");
        let map = read(&design, None, Lang::En).unwrap();
        assert_eq!(map.problems.len(), 5, "the map says so too: {:?}", map.problems);
        assert_eq!(map.flows[0].screens.len(), 1, "and still shows");
        std::fs::remove_dir_all(design).unwrap();
    }

    #[test]
    fn a_flows_walkthroughs_are_read_newest_first_with_where_each_screen_starts() {
        let root = dir("videos");
        let design = root.join("design");
        write(&design.join("flows/flows.json"), WALKED);
        let flow = design.join("flows/onboarding");
        let now = SystemTime::now();
        let video = |name: &str, at: SystemTime| {
            write(&flow.join(name), "not really an mp4");
            touch(&flow.join(name), at);
        };
        video("walkthrough.light.390.mp4", now);
        write(&flow.join("walkthrough.light.390.json"), r#"{ "millis": 8200, "marks": [ { "screen": "login", "at": 0 }, { "screen": "home", "at": 3200 } ] }"#);
        video("walkthrough.dark.390.mp4", now);
        write(&flow.join("walkthrough.dark.390.json"), "{ broken");
        video("history/walkthrough.20261101-090000.light.390.mp4", now - Duration::from_secs(86_400));
        write(&flow.join("history/walkthrough.20261101-090000.light.390.json"), r#"{ "millis": 7000 }"#);
        video("history/walkthrough.20261102-090000.light.390.mp4", now - Duration::from_secs(3600));
        video("intro.mp4", now);
        video("walkthrough.light.390.webm", now);
        write(&root.join("elsewhere.mp4"), "outside");
        symlink(root.join("elsewhere.mp4"), flow.join("walkthrough.light.1280.mp4")).unwrap();
        let map = read(&design, None, Lang::En).unwrap();
        let videos = &map.flows[0].videos;
        let shape: Vec<(&str, u32, bool, u64)> = videos.iter().map(|v| (v.mode.as_str(), v.width, v.current, v.millis)).collect();
        assert_eq!(shape, [("light", 390, true, 8200), ("dark", 390, true, 0), ("light", 390, false, 0), ("light", 390, false, 7000)], "current ones, then the kept ones newest first; a link out isn't one");
        assert_eq!(videos[0].marks, [Mark { screen: "login".into(), at: 0 }, Mark { screen: "home".into(), at: 3200 }]);
        assert_eq!(videos[0].path, flow.join("walkthrough.light.390.mp4").to_string_lossy());
        assert!(videos[2].path.ends_with("history/walkthrough.20261102-090000.light.390.mp4"));
        assert!(videos[0].taken > 1_600_000_000);
        assert!(map.unlisted.is_empty() && map.problems.is_empty(), "{map:?}");
        write(&design.join("flows/flows.json"), r#"{ "flows": [ { "id": "onboarding", "title": "Onboarding", "screens": [ { "id": "history", "title": "History" } ] } ] }"#);
        let map = read(&design, None, Lang::En).unwrap();
        assert!(map.flows[0].screens.is_empty() && map.problems.len() == 1 && map.problems[0].contains("history"), "{map:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_video_is_named_by_its_mode_and_width_and_a_kept_one_by_when_too() {
        assert_eq!(video_name("light", 390), "walkthrough.light.390.mp4");
        assert_eq!(history_name("20261104-051200", "dark", 1280), "walkthrough.20261104-051200.dark.1280.mp4");
        assert_eq!(parse_video_name("walkthrough.light.390.mp4"), Some((None, "light".into(), 390)));
        assert_eq!(parse_video_name("walkthrough.20261104-051200.dark.1280.mp4"), Some((Some("20261104-051200".into()), "dark".into(), 1280)));
        assert_eq!(parse_video_name("walkthrough.yesterday.dark.390.mp4"), None);
        assert_eq!(parse_video_name("walkthrough.2026110-0512000.dark.390.mp4"), None);
        assert_eq!(parse_video_name("walkthrough.sepia.390.mp4"), None);
        assert_eq!(parse_video_name("intro.light.390.mp4"), None);
        assert_eq!(parse_video_name("walkthrough.light.390.webm"), None);
        assert_eq!(parse_video_name("walkthrough.light.wide.mp4"), None);
    }

    #[test]
    fn a_picture_is_named_by_its_state_mode_and_width() {
        assert_eq!(picture_name("error", "dark", 390), "error.dark.390.png");
        assert_eq!(parse_picture_name("error.dark.390.png"), Some(("error".into(), "dark".into(), 390)));
        assert_eq!(parse_picture_name("long-name.light.1280.png"), Some(("long-name".into(), "light".into(), 1280)));
        assert_eq!(parse_picture_name("error.sepia.390.png"), None);
        assert_eq!(parse_picture_name("error.png"), None);
    }
}
