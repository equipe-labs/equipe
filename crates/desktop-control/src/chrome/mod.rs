//! Members working the person's own Google Chrome — their real profiles,
//! every sign-in already there — through the accessibility tree
//! (docs/REAL-CHROME.md).
//!
//! A member opens a page in a new window of a profile the person chose
//! (`chrome_open`); that window is the member's alone: its refs, its
//! snapshots, its actions, and nobody else's — not another member's, not the
//! person's own windows. Reading and acting go through the accessibility
//! engine ([`crate::engine`], agent-desktop), which turns on Chrome's web
//! accessibility tree by itself and acts on refs headless: no pointer, no
//! keyboard, no focus taken, so several members work their own windows at
//! once, beside the person.
//!
//! The rules are code: a human check (a CAPTCHA, "verify you are human") is
//! the person's and takes no member input; only `http(s)` pages open; the
//! send gate judges each ref action (payment, password and security pages
//! take none; a web messaging page counts as its desktop app, so sending
//! there asks the person). Each call is written to an action log without
//! what was typed.
//!
//! Off unless the person turns it on for the squad (`equipe desktop chrome on`).

pub mod os;
pub mod page;
pub mod profiles;

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::controller::EXPECT;
use crate::engine::{Answer, Engine, RefView};
use os::ChromeOs;

/// The member tools (`mcp__desktop__chrome_…`).
pub const TOOLS: [&str; 7] = ["chrome_profiles", "chrome_open", "chrome_snapshot", "chrome_act", "chrome_read", "chrome_capture", "chrome_close"];
/// The picture of a member's window: served by the screenshot tools'
/// controller (ScreenCaptureKit), once this module says which window.
pub const CAPTURE: &str = "chrome_capture";
/// The daemon's internal ask: the real window a member has for a site (for
/// pointing a sign-in hand-off at it).
pub const WINDOW: &str = "_chrome_window";
/// Chrome's name as macOS shows it.
pub const APP: &str = "Google Chrome";

/// What `chrome_act` does, by `action`: the headless ref actions only.
pub const ACTIONS: [&str; 13] = ["click", "type", "set_value", "clear", "focus", "select", "check", "uncheck", "toggle", "expand", "collapse", "scroll", "scroll_to"];

/// How often, and how long, `chrome_open` looks for the window it opened.
const OPEN_POLL_MS: u64 = 250;
const OPEN_POLLS: u32 = 40;
/// Chrome's web tree may take its renderer's two-second switch to answer.
const SNAPSHOT_TIMEOUT_MS: u64 = 8000;
/// The action log is started again past this size (the old one kept as `.1`).
const LOG_LIMIT: u64 = 1 << 20;

const OFF: &str = "Working in the person's own Chrome is off for this squad: only the person turns it on (`equipe desktop chrome on`). Use your own browser tools meanwhile.";
const NOT_YOURS: &str = "That isn't one of your Chrome windows: you act only in windows you opened with chrome_open (the person's own windows, and other members', aren't yours to touch).";
const CHANGED: &str = "The page changed since equipe checked this input (another element, title or page): nothing done. chrome_snapshot again and call again.";
const GENERIC: &str = "Google Chrome is the person's own browser here: work in a window of your own with chrome_open, chrome_snapshot and chrome_act — the desktop tools don't read or act in Chrome.";

/// Whether `tool` is one of the real-Chrome tools.
pub fn handles(tool: &str) -> bool {
    TOOLS.contains(&tool)
}

/// Whether the helper serves `tool` on the engine's threads: all but the
/// picture, which needs the screenshot controller.
pub fn on_engine(tool: &str) -> bool {
    (handles(tool) && tool != CAPTURE) || tool == WINDOW
}

/// Whether `tool` needs the accessibility engine installed (the profile list doesn't).
pub fn needs_engine(tool: &str) -> bool {
    handles(tool) && tool != "chrome_profiles"
}

/// The squad of a helper member key (`squad/member`).
pub fn squad_of(member: &str) -> &str {
    member.split_once('/').map_or(member, |(squad, _)| squad)
}

/// Whether `app` (a name or bundle id a member wrote) is Google Chrome.
pub fn is_chrome(app: &str) -> bool {
    let app = app.trim();
    ["google chrome", "chrome", os::BUNDLE].iter().any(|c| app.eq_ignore_ascii_case(c))
}

// ── the person's switch, per squad ──

const SETTING: &str = "real_chrome";

fn settings_path(home: &Path) -> PathBuf {
    crate::helper::dir(home).join("settings.json")
}

fn settings(home: &Path) -> Value {
    std::fs::read_to_string(settings_path(home)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}))
}

/// Whether the person turned real Chrome on for `squad`. Off unless they did.
pub fn enabled(home: &Path, squad: &str) -> bool {
    match &settings(home)[SETTING] {
        Value::Array(list) => list.iter().any(|s| s.as_str() == Some(squad)),
        _ => false,
    }
}

/// Turn real Chrome on or off for `squad`: the person's alone.
pub fn set_enabled(home: &Path, squad: &str, on: bool) -> std::io::Result<()> {
    let mut all = settings(home);
    let mut list: Vec<String> = all[SETTING].as_array().map(|l| l.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    list.retain(|s| s != squad);
    if on {
        list.push(squad.to_owned());
    }
    all[SETTING] = json!(list);
    crate::helper::ensure_dir(home)?;
    std::fs::write(settings_path(home), all.to_string())
}

// ── the tools ──

/// A window a member opened.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Owned {
    /// agent-desktop's id, `w-` and the window number.
    id: String,
    /// The profile's folder and name.
    dir: String,
    name: String,
    /// The host it was opened at.
    host: String,
}

/// The real-Chrome tools, shared by the helper's threads.
pub struct Chrome {
    home: PathBuf,
    engine: Arc<Engine>,
    os: Box<dyn ChromeOs>,
    /// Member → the windows it opened, oldest first. Forgotten when the
    /// helper restarts (members open a window again).
    windows: Mutex<HashMap<String, Vec<Owned>>>,
    /// One `chrome_open` at a time, so each sees its own new window.
    opening: Mutex<()>,
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn flag(args: &Value, key: &str) -> bool {
    args.get(key).and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true"))).unwrap_or(false)
}

fn number(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.0) as u64)).or_else(|| v.as_str()?.trim().parse().ok()))
}

/// `w-123` from what a member wrote (`w-123`, `w123`, `123`).
pub fn window_id(raw: &str) -> Option<String> {
    window_number(raw).map(|n| format!("w-{n}"))
}

/// The window number in a window id.
pub fn window_number(raw: &str) -> Option<u32> {
    raw.trim().trim_start_matches('w').trim_start_matches('-').parse().ok()
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max { text.to_owned() } else { format!("{}…", text.chars().take(max).collect::<String>()) }
}

impl Chrome {
    pub fn new(home: &Path, engine: Arc<Engine>, os: Box<dyn ChromeOs>) -> Self {
        Self { home: home.to_owned(), engine, os, windows: Mutex::new(HashMap::new()), opening: Mutex::new(()) }
    }

    fn windows(&self) -> std::sync::MutexGuard<'_, HashMap<String, Vec<Owned>>> {
        self.windows.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run a real-Chrome tool for `member` (`squad/member`).
    pub fn call(&self, member: &str, tool: &str, args: &Value) -> Answer {
        let result = if !enabled(&self.home, squad_of(member)) {
            Err(OFF.to_owned())
        } else {
            match tool {
                "chrome_profiles" => self.profiles(member),
                "chrome_open" => self.open(member, args),
                "chrome_snapshot" => self.snapshot(member, args),
                "chrome_act" => self.act(member, args),
                "chrome_read" => self.read(member, args),
                "chrome_close" => self.close(member, args),
                other => Err(format!("There is no Chrome tool called {other}.")),
            }
        };
        self.log(member, tool, args, result.is_ok());
        match result {
            Ok(text) => Answer { text, is_error: false },
            Err(text) => Answer { text, is_error: true },
        }
    }

    /// The window number `chrome_capture` may picture for `member`: one of its own.
    pub fn capture_target(&self, member: &str, args: &Value) -> Result<u32, String> {
        let result = if enabled(&self.home, squad_of(member)) { self.window_for(member, args).map(|w| w.id) } else { Err(OFF.to_owned()) };
        self.log(member, CAPTURE, args, result.is_ok());
        result.and_then(|id| window_number(&id).ok_or_else(|| NOT_YOURS.to_owned()))
    }

    /// Chrome's windows now: id and title.
    fn chrome_windows(&self, member: &str) -> Vec<(String, String)> {
        let Ok(data) = self.engine.raw(member, vec!["list-windows".into(), format!("--app={APP}")], Duration::from_secs(15)) else {
            return Vec::new(); // Chrome isn't running
        };
        let list = data.get("windows").or(Some(&data)).and_then(Value::as_array).cloned().unwrap_or_default();
        list.iter()
            .filter_map(|w| Some((w.get("id")?.as_str()?.to_owned(), w.get("title").and_then(Value::as_str).unwrap_or_default().to_owned())))
            .collect()
    }

    fn taken(&self) -> HashSet<String> {
        self.windows().values().flatten().map(|w| w.id.clone()).collect()
    }

    /// The member's window a call means: `window`, else the one it opened last.
    fn window_for(&self, member: &str, args: &Value) -> Result<Owned, String> {
        let windows = self.windows();
        let mine = windows.get(member).map(Vec::as_slice).unwrap_or_default();
        match text(args, "window") {
            Some(raw) => {
                let id = window_id(raw).ok_or_else(|| format!("{raw:?} isn't a window id: use the one chrome_open gave you, like w-4521."))?;
                mine.iter().find(|w| w.id == id).cloned().ok_or_else(|| NOT_YOURS.to_owned())
            }
            None => mine.last().cloned().ok_or_else(|| "Open a window first with chrome_open.".to_owned()),
        }
    }

    /// Whether `window` is one of `member`'s.
    fn mine(&self, member: &str, window: Option<&str>) -> Result<(), String> {
        let windows = self.windows();
        let ok = window.is_some_and(|id| windows.get(member).is_some_and(|mine| mine.iter().any(|w| w.id == id)));
        if ok { Ok(()) } else { Err(NOT_YOURS.to_owned()) }
    }

    fn forget(&self, member: &str, id: &str) {
        if let Some(mine) = self.windows().get_mut(member) {
            mine.retain(|w| w.id != id);
        }
    }

    /// The window, if Chrome still shows it; forgotten when it doesn't.
    fn still_open(&self, member: &str, window: &Owned) -> Result<String, String> {
        match self.chrome_windows(member).into_iter().find(|(id, _)| *id == window.id) {
            Some((_, title)) => Ok(title),
            None => {
                self.forget(member, &window.id);
                Err(format!("Your window {} was closed (by the person, or Chrome). Open another with chrome_open.", window.id))
            }
        }
    }

    fn profiles(&self, member: &str) -> Result<String, String> {
        let list = profiles::parse(&self.os.local_state()?)?;
        let mut out = format!("The person's Chrome profiles (folder · name · Google account):\n{}", profiles::lines(&list));
        let chosen = profiles::chosen(&self.home, squad_of(member));
        if chosen.is_empty() {
            out.push_str("\nNo profile chosen for any site yet. Unsure which one a site wants? Ask the person (ask_decision), then chrome_open with `profile` and `remember: true`.");
        } else {
            out.push_str("\nThis squad uses:");
            for (site, dir) in chosen {
                let name = list.iter().find(|p| p.dir == dir).map(|p| format!(" ({:?})", p.name)).unwrap_or_default();
                out.push_str(&format!("\n{site} → {dir}{name}"));
            }
        }
        let windows = self.windows();
        let mine = windows.get(member).map(Vec::as_slice).unwrap_or_default();
        if !mine.is_empty() {
            out.push_str("\nYour windows:");
            for w in mine {
                out.push_str(&format!("\n{} · {:?} · {}", w.id, w.name, page::site(&w.host)));
            }
        }
        Ok(out)
    }

    fn open(&self, member: &str, args: &Value) -> Result<String, String> {
        let url = page::web_url(text(args, "url").unwrap_or_default())?;
        let host = page::host(&url).unwrap_or_default();
        let site = page::site(&host);
        let squad = squad_of(member);
        let list = profiles::parse(&self.os.local_state()?)?;
        let remembered = profiles::remembered(&self.home, squad, &host);
        let profile = match text(args, "profile") {
            Some(want) => profiles::resolve(&list, want)?.clone(),
            None => match remembered.as_deref().and_then(|dir| list.iter().find(|p| p.dir == dir)) {
                Some(p) => p.clone(),
                None if list.len() == 1 => list[0].clone(),
                None => {
                    return Err(format!(
                        "Which of the person's Chrome profiles is for {site}? Pick by the account; if it isn't plain, ask the person (ask_decision). Then call again with `profile` and `remember: true`, so the squad keeps the choice:\n{}",
                        profiles::lines(&list)
                    ));
                }
            },
        };
        let (id, title) = {
            let _turn = self.opening.lock().unwrap_or_else(|e| e.into_inner());
            let before: HashSet<String> = self.chrome_windows(member).into_iter().map(|(id, _)| id).collect();
            self.os.open(&profile.dir, &url)?;
            let mut found = None;
            for _ in 0..OPEN_POLLS {
                self.os.pause(OPEN_POLL_MS);
                let taken = self.taken();
                if let Some(new) = self.chrome_windows(member).into_iter().rfind(|(id, _)| !before.contains(id) && !taken.contains(id)) {
                    found = Some(new);
                    break;
                }
            }
            let (id, title) = found.ok_or(
                "Chrome didn't show a new window within 10 s. It may be showing its profile picker or a dialog, or not be installed: ask the person to look at Chrome.",
            )?;
            self.windows().entry(member.to_owned()).or_default().push(Owned { id: id.clone(), dir: profile.dir.clone(), name: profile.name.clone(), host: host.clone() });
            (id, title)
        };
        let keep = flag(args, "remember") && remembered.as_deref() != Some(profile.dir.as_str());
        if keep {
            profiles::remember(&self.home, squad, &host, &profile.dir).map_err(|e| format!("Opened {id}, but the choice couldn't be kept: {e}"))?;
        }
        let account = profile.masked().map(|m| format!(", {m}")).unwrap_or_default();
        let mut out = format!("opened {id} in the person's Chrome — profile {:?} ({}{account}) at {site}", profile.name, profile.dir);
        if !title.is_empty() {
            out.push_str(&format!(" · {:?}", clip(&title, 80)));
        }
        if keep {
            out.push_str(&format!("\nThis squad now uses {:?} for {site}.", profile.name));
        }
        out.push_str("\nIt is yours alone until chrome_close. Read it with chrome_snapshot (still loading: `wait` with app \"Google Chrome\" and `text`, then snapshot).");
        Ok(out)
    }

    fn snapshot(&self, member: &str, args: &Value) -> Result<String, String> {
        let mut engine_args = json!({ "app": APP, "timeout_ms": number(args, "timeout_ms").unwrap_or(SNAPSHOT_TIMEOUT_MS) });
        if let Some(root) = text(args, "root") {
            let view = self.engine.describe(member, root)?;
            self.mine(member, view.window_id.as_deref())?;
            engine_args["root"] = json!(root);
        } else {
            let window = self.window_for(member, args)?;
            self.still_open(member, &window)?;
            engine_args["window"] = json!(window.id);
        }
        for key in ["skeleton", "all"] {
            if flag(args, key) {
                engine_args[key] = json!(true);
            }
        }
        if let Some(depth) = number(args, "depth") {
            engine_args["depth"] = json!(depth);
        }
        let answer = self.engine.call(member, "snapshot", &engine_args);
        if answer.is_error {
            return Err(answer.text);
        }
        let mut out = answer.text;
        if page::shows_human_check(&out) {
            out.push_str("\n\n");
            out.push_str(page::HUMAN_CHECK_NOTE);
        }
        Ok(out)
    }

    /// The ref a `chrome_act` means, if it may be acted on: in one of the
    /// member's windows, and not part of a human check.
    fn check_ref(&self, member: &str, args: &Value) -> Result<RefView, String> {
        let raw = text(args, "ref").ok_or("Say which element with `ref` (from your chrome_snapshot, like e12).")?;
        let view = self.engine.describe(member, raw)?;
        if !view.app.as_deref().is_some_and(is_chrome) {
            return Err("That ref isn't in Chrome: chrome_act takes refs from your chrome_snapshot.".into());
        }
        self.mine(member, view.window_id.as_deref())?;
        let names = [view.name.as_deref(), view.description.as_deref()].into_iter().flatten();
        if page::is_human_check(names) || view.window.as_deref().is_some_and(page::challenge_title) {
            return Err(page::HUMAN_CHECK_NOTE.to_owned());
        }
        Ok(view)
    }

    /// What `chrome_act` would act on, for the send gate (the shape of
    /// `equipe_core::desktop_gate::Scene`): the engine's, with a web
    /// messaging page counted as its desktop app.
    pub fn scene(&self, member: &str, args: &Value) -> Result<Value, String> {
        if !enabled(&self.home, squad_of(member)) {
            return Err(OFF.to_owned());
        }
        let view = self.check_ref(member, args)?;
        let mut scene = self.engine.scene(member, "act", &without(args, &[EXPECT, "headed"]))?;
        if let Some(app) = view.window.as_deref().and_then(page::web_messaging) {
            scene["app"] = json!({ "name": app });
        }
        Ok(scene)
    }

    fn act(&self, member: &str, args: &Value) -> Result<String, String> {
        let action = text(args, "action").unwrap_or("click");
        if !ACTIONS.contains(&action) {
            return Err(format!(
                "No action {action:?} in Chrome: one of {}. Physical clicks and keys aren't offered here — the page must take an accessibility action.",
                ACTIONS.join(", ")
            ));
        }
        self.check_ref(member, args)?;
        let mut engine_args = without(args, &[EXPECT, "headed"]);
        engine_args["action"] = json!(action);
        if let Some(expected) = args.get(EXPECT) {
            // The gate judged this module's scene (a web messaging page as its
            // app); the engine checks again against its own, so it gets that
            // with the app as the engine names it.
            if self.scene(member, args)?.get("app") != expected.get("app") {
                return Err(CHANGED.into());
            }
            let mut for_engine = expected.clone();
            for_engine["app"] = self.engine.scene(member, "act", &engine_args)?["app"].clone();
            engine_args[EXPECT] = for_engine;
        }
        let answer = self.engine.call(member, "act", &engine_args);
        if answer.is_error { Err(answer.text) } else { Ok(answer.text) }
    }

    fn read(&self, member: &str, args: &Value) -> Result<String, String> {
        let window = self.window_for(member, args)?;
        let title = self.still_open(member, &window)?;
        let cli = vec![
            "snapshot".to_owned(),
            format!("--app={APP}"),
            format!("--window-id={}", window.id),
            "--compact".into(),
            "--max-depth=40".into(),
            format!("--timeout-ms={SNAPSHOT_TIMEOUT_MS}"),
        ];
        let data = self.engine.raw(member, cli, Duration::from_millis(SNAPSHOT_TIMEOUT_MS + 20_000))?;
        let mut lines = page::lines_of(data.get("tree").unwrap_or(&Value::Null));
        if let Some(find) = text(args, "find") {
            let find = find.to_lowercase();
            lines.retain(|l| l.to_lowercase().contains(&find));
            if lines.is_empty() {
                lines.push(format!("(no line on the page contains {find:?})"));
            }
        }
        let max = number(args, "max_chars").unwrap_or(12_000).clamp(500, 60_000) as usize;
        let human = page::challenge_title(&title) || page::is_human_check(lines.iter().map(String::as_str));
        let mut out = format!("{:?} · {}\n{}", clip(&title, 120), window.id, page::fit(&lines, max));
        if human {
            out.push_str("\n\n");
            out.push_str(page::HUMAN_CHECK_NOTE);
        }
        Ok(out)
    }

    fn close(&self, member: &str, args: &Value) -> Result<String, String> {
        let window = self.window_for(member, args)?;
        let number = window_number(&window.id).ok_or_else(|| NOT_YOURS.to_owned())?;
        match self.os.close(number) {
            Ok(()) => {
                self.forget(member, &window.id);
                Ok(format!("closed {}", window.id))
            }
            Err(why) => match self.still_open(member, &window) {
                Ok(_) => Err(why),
                Err(_) => Ok(format!("{} was already closed", window.id)),
            },
        }
    }

    /// The real window `member` has for the site of `url` (else its latest):
    /// JSON with its id, profile and title, for pointing a sign-in hand-off
    /// at it. An error when it has none.
    pub fn window_for_site(&self, member: &str, url: Option<&str>) -> Result<Value, String> {
        if !enabled(&self.home, squad_of(member)) {
            return Err(OFF.to_owned());
        }
        let host = url.and_then(|u| page::web_url(u).ok()).and_then(|u| page::host(&u));
        let window = {
            let windows = self.windows();
            let mine = windows.get(member).map(Vec::as_slice).unwrap_or_default();
            let same = host.as_deref().and_then(|h| mine.iter().rev().find(|w| page::same_site(&w.host, h)));
            same.or(mine.last()).cloned().ok_or("No Chrome window of yours is open.")?
        };
        let title = self.still_open(member, &window)?;
        Ok(json!({ "id": window.id, "profile": window.name, "dir": window.dir, "site": page::site(&window.host), "title": title }))
    }

    /// The desktop tools stay out of Chrome while real Chrome is on for the
    /// squad: the person's other windows are theirs. Before a call: a
    /// refusal for a snapshot or find of Chrome, or an act on a Chrome ref.
    pub fn guard_generic(&self, member: &str, tool: &str, args: &Value) -> Option<String> {
        if !enabled(&self.home, squad_of(member)) {
            return None;
        }
        let ref_in_chrome = |key: &str| text(args, key).and_then(|r| self.engine.describe(member, r).ok()).and_then(|v| v.app).is_some_and(|a| is_chrome(&a));
        let chrome = match tool {
            "snapshot" | "find" if text(args, "root").is_some() => ref_in_chrome("root"),
            "snapshot" | "find" => text(args, "app").is_some_and(is_chrome),
            "act" => ref_in_chrome("ref"),
            _ => false,
        };
        chrome.then(|| GENERIC.to_owned())
    }

    /// After a desktop snapshot or find without an app (the focused one): a
    /// refusal in place of what it read, when that was Chrome.
    pub fn guard_answer(&self, member: &str, tool: &str, answer: Answer) -> Answer {
        let header = answer.text.lines().next().unwrap_or_default();
        let app = header.split(" · ").next().unwrap_or_default();
        if matches!(tool, "snapshot" | "find") && !answer.is_error && is_chrome(app) && enabled(&self.home, squad_of(member)) {
            return Answer { text: GENERIC.to_owned(), is_error: true };
        }
        answer
    }

    /// One line in the action log: who, which tool, which action and window,
    /// whether it went ahead. Never what was typed, read or shown.
    fn log(&self, member: &str, tool: &str, args: &Value, ok: bool) {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut line = json!({ "at": at, "member": member, "tool": tool, "ok": ok });
        if let Some(action) = text(args, "action") {
            line["action"] = json!(action);
        }
        if let Some(window) = text(args, "window").and_then(window_id) {
            line["window"] = json!(window);
        }
        if let Some(site) = text(args, "url").and_then(|u| page::web_url(u).ok()).and_then(|u| page::host(&u)) {
            line["site"] = json!(page::site(&site));
        }
        let path = log_path(&self.home);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_LIMIT) {
            let _ = std::fs::rename(&path, path.with_extension("log.1"));
        }
        if crate::helper::ensure_dir(&self.home).is_ok()
            && let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// The real-Chrome action log.
pub fn log_path(home: &Path) -> PathBuf {
    crate::helper::dir(home).join("chrome-actions.log")
}

fn without(args: &Value, keys: &[&str]) -> Value {
    let mut out = args.clone();
    if let Some(object) = out.as_object_mut() {
        for key in keys {
            object.remove(*key);
        }
    }
    out
}

#[cfg(test)]
mod tests;
