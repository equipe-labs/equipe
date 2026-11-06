//! Phone tools: members test a squad's mobile app on a simulator or an
//! emulator the way a person would — start the ticket's device with the app,
//! look at the screen, tap, type, swipe and scroll, record the whole flow and
//! check it frame by frame, then report each step with the frame that shows
//! it. Served as the `phone` MCP server (`mcp__phone__…`), offered only to
//! QA and mobile members of a squad with an iOS or Android app
//! ([`detect::platforms`]). docs/mobile-qa.md says what a person installs.
//!
//! Every device is the ticket's and the member's own: made and named for it
//! (`<squad> T<ticket> <member> <model>`, an AVD `<squad>_T<ticket>_<member>`),
//! one booted per member, deleted with its recordings when the ticket is done
//! ([`crate::disk::clear_simulators`], [`crate::disk::clear_recordings`]).
//! The tools never touch another device; the person's own phone is out of
//! reach (pairing one is for later).

pub mod android;
pub mod axe;
pub mod detect;
pub mod exec;
pub mod frames;
pub mod ios;
pub mod report;
pub mod tree;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use equipe_protocol::rpc::{Image, ToolResult};
use image::RgbImage;
use serde_json::{Value, json};

pub use detect::Platforms;
use exec::{Exec, Running};
use frames::Mark;

/// The server's name: members see `mcp__phone__<tool>`.
pub const SERVER: &str = "phone";
pub const START: &str = "start";
pub const LOOK: &str = "look";
pub const ACT: &str = "act";
pub const RECORD: &str = "record";
pub const FRAMES: &str = "frames";
pub const REPORT: &str = "report";
pub const STOP: &str = "stop";

/// The permission card's tool for installing what the phone tools need.
pub const INSTALL: &str = "Install";

/// Said when AXe isn't here and the person said no to it.
pub const NO_AXE: &str = "Touching and reading the iOS Simulator needs AXe, which isn't installed (the person declined it). \
You can still start the app, take screenshots with `look`, record and read frames; say in your report what you couldn't do.";

/// How the server introduces itself.
pub const INSTRUCTIONS: &str = "The ticket's own phone — an iOS Simulator or Android emulator — to test the squad's app \
as a person would: start, look, act, record, frames, report, stop. Coordinates are points of `look`'s screenshot.";

/// The roles that get the phone tools in a squad with a mobile app: QA, and
/// the mobile engineer checking its own work.
pub fn for_role(role: &str) -> bool {
    matches!(role, "qa" | "mobile")
}

/// A platform a device runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Ios,
    Android,
}

impl Platform {
    pub fn word(self) -> &'static str {
        match self {
            Platform::Ios => "ios",
            Platform::Android => "android",
        }
    }
}

/// A hardware key or button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Home,
    Back,
    Enter,
    Delete,
    Tab,
    Escape,
}

impl Key {
    fn of(word: &str) -> Option<Self> {
        Some(match word.trim().to_lowercase().as_str() {
            "home" => Key::Home,
            "back" => Key::Back,
            "enter" | "return" => Key::Enter,
            "delete" | "backspace" => Key::Delete,
            "tab" => Key::Tab,
            "escape" | "esc" => Key::Escape,
            _ => return None,
        })
    }
}

/// One booted simulator or emulator, as the tools drive it.
pub trait Device {
    fn screenshot(&self) -> Result<RgbImage, String>;
    fn tree(&self) -> Result<tree::Screen, String>;
    /// Whether it can be touched and read here (iOS: AXe is installed).
    fn can_touch(&self) -> bool;
    fn tap(&self, x: f64, y: f64) -> Result<(), String>;
    fn type_text(&self, text: &str) -> Result<(), String>;
    fn swipe(&self, from: (f64, f64), to: (f64, f64), seconds: f64) -> Result<(), String>;
    fn key(&self, key: Key) -> Result<(), String>;
    /// Install an app: its bundle id or package, when it can be read.
    fn install(&self, app: &Path) -> Result<Option<String>, String>;
    fn launch(&self, bundle: &str) -> Result<(), String>;
    fn open_url(&self, url: &str) -> Result<(), String>;
    fn record(&self, video: &Path) -> Result<Box<dyn Running>, String>;
    fn finish(&self, running: Box<dyn Running>, video: &Path) -> Result<(), String>;
    /// The screenshot's pixels as the tree's points.
    fn points_per_pixel(&self, image: &RgbImage) -> f64;
}

/// The tools, as MCP tool definitions. How to use each is said here, not in charters.
pub fn tools() -> Vec<Value> {
    let target = json!({
        "description": "What to aim at: an element's label as `look` shows it (\"Pay\"), its id (\"id:pay\"), or a point {\"x\": 201, \"y\": 725} in points.",
        "anyOf": [{ "type": "string" }, { "type": "object", "properties": { "x": { "type": "number" }, "y": { "type": "number" } }, "required": ["x", "y"] }],
    });
    vec![
        json!({
            "name": START,
            "description": "Start your phone for this ticket: an iOS Simulator or Android emulator made for the ticket and you \
(equipe names it, and deletes it with its recordings when the ticket is done), booted without a window. Give `app` — the `.app` \
your simulator build made, or an `.apk` — to install and launch it, or `bundle` to launch one already on it. It answers with the \
device's id to build for (`xcodebuild … -destination 'id=<udid>'`, `adb -s <serial>`). One device at a time: starting one for \
another ticket shuts your last one down. Never another simulator, emulator or a real phone.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "platform": { "type": "string", "enum": ["ios", "android"], "description": "Needed only when the squad's app runs on both." },
                    "app": { "type": "string", "description": "The built app to install: a simulator `.app` folder or an `.apk`, by path (relative to where you work, or absolute)." },
                    "bundle": { "type": "string", "description": "The app's bundle id (iOS) or package (Android), to launch it — read from `app` when it can be." },
                    "device": { "type": "string", "description": "iOS: the simulator model (\"iPhone 17 Pro\", the default when this Xcode has it). Android uses a phone-sized emulator." },
                    "url": { "type": "string", "description": "A link to open once started (a deep link, a web page)." },
                },
            },
        }),
        json!({
            "name": LOOK,
            "description": "See the phone's screen as a person does: a screenshot, and the screen's elements one a line — kind, \
label, id, value and frame in points (`Button \"Pay\" id=pay @ 20,700 362×50`), the same points as the picture. Aim `act` with the \
labels. `screenshot: false` gives only the elements: cheaper, when words are enough.",
            "inputSchema": {
                "type": "object",
                "properties": { "screenshot": { "type": "boolean", "description": "Include the picture (default true)." } },
            },
        }),
        json!({
            "name": ACT,
            "description": "Do one thing on the phone as a finger would, then get the screen's elements back. `tap` a `target`; \
`type` `text` into the field that has focus (tap it first); `scroll` `up`/`down`/`left`/`right` to see what is further that way; \
`swipe` a `target` (or the middle of the screen) in a `direction`; `key`: `enter`, `delete`, `tab`, `escape`, `back`, `home`; \
`wait` until `target` is on the screen (`gone: true`: until it isn't), up to `timeout` seconds; `launch` the app again; \
`open_url`. A target that matches nothing or several things is refused with what is there: look, then aim again. While \
recording, each action is marked in the video with `step`, so `frames` finds the moments before and after it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "do": { "type": "string", "enum": ["tap", "type", "scroll", "swipe", "key", "wait", "launch", "open_url"] },
                    "target": target,
                    "text": { "type": "string", "description": "`type`: the text. `key`: the key." },
                    "direction": { "type": "string", "enum": ["up", "down", "left", "right"] },
                    "gone": { "type": "boolean", "description": "`wait`: until `target` is no longer on the screen." },
                    "timeout": { "type": "number", "description": "`wait`: seconds, default 10, at most 60." },
                    "step": { "type": "string", "description": "The flow's step this is, in a few words (\"tap Pay\"): its mark in the recording." },
                    "bundle": { "type": "string", "description": "`launch`: the app, when not the one `start` launched." },
                    "url": { "type": "string", "description": "`open_url`: the link." },
                    "look": { "type": "boolean", "description": "Add a screenshot of the screen after it." },
                },
                "required": ["do"],
            },
        }),
        json!({
            "name": RECORD,
            "description": "Record the phone's screen as a video, to check a whole flow afterwards frame by frame and to show the \
person. `start` with a short `name` for the flow (\"checkout\") before its first step, act through it, `stop` after the last: \
you get the video's path, its length and the actions marked in it. One recording at a time; Android records at most three \
minutes at once. Recordings are kept with the ticket; one a ticket you file names is kept until that ticket is done too.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["start", "stop"] },
                    "name": { "type": "string", "description": "`start`: the flow's name, a few words." },
                },
                "required": ["action"],
            },
        }),
        json!({
            "name": FRAMES,
            "description": "Frames of a recording, to check each step against what you expected — never only the last screen. \
Without `at`: the moment before and the screen after each action marked while recording, and every screen the video settled \
on. With `at`: those times, in seconds. Saved as pictures beside the recording; their paths come back numbered, with one sheet \
of them all. Read a frame's file to see it full size; a report's frames are these paths.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "recording": { "type": "string", "description": "Its name or path, as `record` gave it (default: your latest)." },
                    "at": { "type": "array", "items": { "type": "number" }, "description": "Times in seconds." },
                    "max": { "type": "integer", "description": "At most this many frames (default 12, up to 24)." },
                },
            },
        }),
        json!({
            "name": REPORT,
            "description": "Give the person what you found in a recorded flow: each step with what you expected, what you saw and \
its verdict — `pass`, `fail`, or `blocked` when you couldn't get to check it — and, for a pass or a fail, the frame that shows it \
(a path `frames` gave). equipe puts it on the ticket's record and in the conversation, with the recording to play and the key \
frames, so the person sees it without asking. A verdict without its frame is refused. Then file each failure as a bug with \
`create_ticket`, naming the recording and the frame. Write `step`, `expected` and `seen` in the person's language.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "flow": { "type": "string", "description": "What was tested, in a few words." },
                    "recording": { "type": "string", "description": "The recording's name or path, as `record` gave it." },
                    "steps": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "step": { "type": "string" },
                                "expected": { "type": "string" },
                                "seen": { "type": "string" },
                                "verdict": { "type": "string", "enum": ["pass", "fail", "blocked"] },
                                "frame": { "type": "string" },
                            },
                            "required": ["step", "expected", "seen", "verdict"],
                        },
                    },
                },
                "required": ["flow", "recording", "steps"],
            },
        }),
        json!({
            "name": STOP,
            "description": "Put the phone away: a recording still running is saved first, then the device is shut down (it stays \
the ticket's until the ticket is done). Do it when you're through, before you hand in.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
    ]
}

/// Who calls, on what, and where its files go: what the runtime knows.
#[derive(Debug, Clone)]
pub struct Call {
    pub squad: String,
    pub member: String,
    pub ticket: u32,
    pub platforms: Platforms,
    /// The ticket's recordings: `<squad home>/recordings/T-<ticket>/`.
    pub recordings: PathBuf,
    /// equipe's home, where AXe is kept.
    pub equipe_home: PathBuf,
    /// Where the member works, for a relative app path.
    pub cwd: PathBuf,
}

/// What became of a call.
#[derive(Debug)]
pub enum Outcome {
    Done(ToolResult),
    /// iOS needs AXe and it isn't here: ask the person, then call again.
    NeedsAxe,
    /// A report to put on the ticket and in the conversation; then answer with the result.
    Report(Box<report::Report>, ToolResult),
}

struct Recording {
    name: String,
    path: PathBuf,
    running: Box<dyn Running>,
    started: Instant,
    marks: Vec<Mark>,
}

struct Seat {
    ticket: u32,
    platform: Platform,
    /// The simulator's udid, or the emulator's serial.
    id: String,
    name: String,
    bundle: Option<String>,
    recording: Option<Recording>,
    /// The latest recording's name.
    last: Option<String>,
}

/// The devices members have up, and the recordings running: one per member.
#[derive(Default)]
pub struct Phones {
    seats: Mutex<HashMap<(String, String), Seat>>,
    /// The person said no to installing AXe (until the daemon restarts).
    axe_declined: Mutex<bool>,
}

/// The daemon's.
pub static PHONES: LazyLock<Phones> = LazyLock::new(Phones::default);

/// A recording runs at most this long before the next call stops it.
const RECORDING_MAX: Duration = Duration::from_secs(10 * 60);

impl Phones {
    pub fn decline_axe(&self) {
        *self.axe_declined.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = true;
    }

    pub fn axe_declined(&self) -> bool {
        *self.axe_declined.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Everything of `ticket` let go of: recordings saved, devices shut down —
    /// before the disk's clean-up deletes them. Its devices' names, for the log.
    pub fn release_ticket(&self, exec: &dyn Exec, squad: &str, ticket: u32) -> Vec<String> {
        let mut seats = self.seats.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let keys: Vec<(String, String)> = seats.iter().filter(|((s, _), seat)| s == squad && seat.ticket == ticket).map(|(k, _)| k.clone()).collect();
        let mut gone = Vec::new();
        for key in keys {
            if let Some(mut seat) = seats.remove(&key) {
                put_away(exec, &mut seat);
                gone.push(seat.name);
            }
        }
        gone
    }
}

fn adb_or_err() -> Result<String, String> {
    android::adb_path().ok_or_else(|| android::NO_SDK.to_owned())
}

/// The device a seat is, driven.
fn device<'a>(exec: &'a dyn Exec, seat: &Seat, axe: Option<String>) -> Result<Box<dyn Device + 'a>, String> {
    Ok(match seat.platform {
        Platform::Ios => Box::new(ios::Ios { exec, udid: seat.id.clone(), axe }),
        Platform::Android => Box::new(android::Android { exec, adb: adb_or_err()?, serial: seat.id.clone() }),
    })
}

/// A recording stopped and saved, a device shut down.
fn put_away(exec: &dyn Exec, seat: &mut Seat) {
    if let Some(rec) = seat.recording.take()
        && let Ok(d) = device(exec, seat, None)
    {
        let _ = d.finish(rec.running, &rec.path);
        save_marks(&rec.path, &rec.marks);
    }
    match seat.platform {
        Platform::Ios => ios::shutdown(exec, &seat.id),
        Platform::Android => {
            if let Some(adb) = android::adb_path() {
                android::shutdown(exec, &adb, &seat.id);
            }
        }
    }
}

fn marks_path(video: &Path) -> PathBuf {
    video.with_extension("marks.json")
}

fn save_marks(video: &Path, marks: &[Mark]) {
    if let Ok(text) = serde_json::to_string_pretty(marks) {
        let _ = std::fs::write(marks_path(video), text);
    }
}

fn load_marks(video: &Path) -> Vec<Mark> {
    std::fs::read_to_string(marks_path(video)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// A flow's name as a file name: `checkout-flow`.
fn slug(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if c.is_alphanumeric() { c.to_lowercase().next().unwrap_or(c) } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    let s: String = s.chars().take(40).collect();
    if s.is_empty() { "flow".into() } else { s }
}

fn image_result(text: String, image: &RgbImage) -> ToolResult {
    let data = base64::engine::general_purpose::STANDARD.encode(frames::jpeg(image));
    ToolResult { text, is_error: false, image: Some(Image { mime: "image/jpeg".into(), data }) }
}

/// Run one phone tool call. Blocking: the daemon calls it off its loop.
pub fn call(phones: &Phones, exec: &dyn Exec, c: &Call, name: &str, args: &Value) -> Outcome {
    let axe = axe::find(&c.equipe_home).map(|p| p.to_string_lossy().into_owned());
    let mut seats = phones.seats.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let key = (c.squad.clone(), c.member.clone());
    // A recording forgotten: stopped and saved before anything else.
    let mut said = String::new();
    if let Some(seat) = seats.get_mut(&key)
        && seat.recording.as_ref().is_some_and(|r| r.started.elapsed() > RECORDING_MAX)
    {
        let rec = seat.recording.take().expect("checked");
        if let Ok(d) = device(exec, seat, axe.clone()) {
            let _ = d.finish(rec.running, &rec.path);
        }
        save_marks(&rec.path, &rec.marks);
        said = format!("(Your recording {} ran past {} minutes and was stopped and saved.)\n", rec.name, RECORDING_MAX.as_secs() / 60);
    }
    if name == START {
        return start(exec, c, args, &mut seats, axe).map_or_else(|e| Outcome::Done(ToolResult::error(e)), |t| Outcome::Done(ToolResult::ok(format!("{said}{t}"))));
    }
    if name == REPORT {
        return match report::from_args(args, &c.recordings, c.ticket, &c.member) {
            Ok(r) => {
                let file = r.recording.with_extension("report.json");
                let _ = std::fs::write(&file, serde_json::to_string_pretty(&r).unwrap_or_default());
                let failed: Vec<String> = r.steps.iter().enumerate().filter(|(_, s)| s.verdict == report::Verdict::Fail).map(|(i, s)| format!("{}. {}", i + 1, s.step)).collect();
                let next = if failed.is_empty() {
                    String::new()
                } else {
                    format!(" File each failure as a bug with `create_ticket`, naming {} and its frame: {}.", r.recording.display(), failed.join("; "))
                };
                let text = format!("{said}Reported on T-{}: {} of {} steps passed. The person sees it in the conversation, with the recording and the frames.{next}", c.ticket, r.passed(), r.steps.len());
                Outcome::Report(Box::new(r), ToolResult::ok(text))
            }
            Err(e) => Outcome::Done(ToolResult::error(e)),
        };
    }
    if name == FRAMES {
        let last = seats.get(&key).filter(|s| s.ticket == c.ticket).and_then(|s| s.last.clone());
        drop(seats);
        return Outcome::Done(frames_of(c, args, last.as_deref()).map_or_else(ToolResult::error, |r| {
            ToolResult { text: format!("{said}{}", r.text), ..r }
        }));
    }
    let Some(seat) = seats.get_mut(&key) else {
        return Outcome::Done(ToolResult::error(if name == STOP { "Your phone isn't up: nothing to stop.".to_owned() } else { "Your phone isn't up. Start it with `start` first.".to_owned() }));
    };
    if seat.ticket != c.ticket {
        return Outcome::Done(ToolResult::error(format!("Your phone is T-{}'s. Start one for T-{} with `start`.", seat.ticket, c.ticket)));
    }
    let needs_touch = name == ACT || (name == LOOK && seat.platform == Platform::Ios);
    if seat.platform == Platform::Ios && axe.is_none() && needs_touch && !phones.axe_declined() {
        return Outcome::NeedsAxe;
    }
    let result = match name {
        LOOK => look(exec, seat, axe, args),
        ACT => act(exec, seat, axe, args),
        RECORD => record(exec, seat, c, args),
        STOP => {
            let mut seat = seats.remove(&key).expect("present");
            let recorded = seat.recording.as_ref().map(|r| r.path.display().to_string());
            put_away(exec, &mut seat);
            Ok(ToolResult::ok(match recorded {
                Some(path) => format!("Saved the recording {path} and shut {} down.", seat.name),
                None => format!("Shut {} down. It stays the ticket's until the ticket is done.", seat.name),
            }))
        }
        other => Err(format!("The phone has no tool {other}.")),
    };
    Outcome::Done(match result {
        Ok(r) => ToolResult { text: format!("{said}{}", r.text), ..r },
        Err(e) => ToolResult::error(format!("{said}{e}")),
    })
}

fn start(exec: &dyn Exec, c: &Call, args: &Value, seats: &mut HashMap<(String, String), Seat>, axe: Option<String>) -> Result<String, String> {
    let platform = match args["platform"].as_str().map(str::trim) {
        Some("ios") if c.platforms.ios => Platform::Ios,
        Some("android") if c.platforms.android => Platform::Android,
        Some(p @ ("ios" | "android")) => return Err(format!("The squad's app doesn't run on {p} here.")),
        Some(other) if !other.is_empty() => return Err(format!("{other} isn't a platform: ios or android.")),
        _ => match (c.platforms.ios, c.platforms.android) {
            (true, false) => Platform::Ios,
            (false, true) => Platform::Android,
            (true, true) => return Err("The squad's app runs on iOS and Android: say which with `platform`.".into()),
            (false, false) => return Err("The squad has no mobile app to run here.".into()),
        },
    };
    let key = (c.squad.clone(), c.member.clone());
    // One device a member: the last one, if another, goes down first.
    let mut moved = String::new();
    if let Some(seat) = seats.get(&key)
        && (seat.ticket != c.ticket || seat.platform != platform)
    {
        let mut old = seats.remove(&key).expect("present");
        put_away(exec, &mut old);
        moved = format!("Shut down your last device, {}. ", old.name);
    }
    let prefix = format!("{} T{} {}", c.squad, c.ticket, c.member);
    if !seats.contains_key(&key) {
        let seat = match platform {
            Platform::Ios => {
                if !cfg!(target_os = "macos") {
                    return Err("The iOS Simulator runs only on a Mac.".into());
                }
                let (sim, _) = ios::ensure(exec, &prefix, args["device"].as_str())?;
                if !sim.booted {
                    ios::boot(exec, &sim.udid)?;
                }
                Seat { ticket: c.ticket, platform, id: sim.udid, name: sim.name, bundle: None, recording: None, last: None }
            }
            Platform::Android => {
                let adb = adb_or_err()?;
                let emulator = android::emulator_path().ok_or("The Android emulator isn't installed (Android Studio › SDK Manager › SDK Tools › Android Emulator).")?;
                let avd = android::avd_name(&prefix);
                android::ensure_avd(&avd)?;
                let serial = android::boot(exec, &adb, &emulator, &avd)?;
                Seat { ticket: c.ticket, platform, id: serial, name: avd, bundle: None, recording: None, last: None }
            }
        };
        seats.insert(key.clone(), seat);
    }
    let seat = seats.get_mut(&key).expect("just put");
    let d = device(exec, seat, axe)?;
    let mut done = vec![format!("{moved}{} is up", seat.name)];
    if let Some(app) = args["app"].as_str().map(str::trim).filter(|a| !a.is_empty()) {
        let path = if Path::new(app).is_absolute() { PathBuf::from(app) } else { c.cwd.join(app) };
        if !path.exists() {
            return Err(format!("There's no app at {}. Build it for the simulator or emulator first.", path.display()));
        }
        let bundle = d.install(&path)?;
        done.push(format!("installed {}", path.display()));
        seat.bundle = bundle.or(seat.bundle.take());
    }
    if let Some(bundle) = args["bundle"].as_str().map(str::trim).filter(|b| !b.is_empty()) {
        seat.bundle = Some(bundle.to_owned());
    }
    if let Some(bundle) = seat.bundle.clone().filter(|_| args["app"].is_string() || args["bundle"].is_string()) {
        d.launch(&bundle)?;
        done.push(format!("launched {bundle}"));
    }
    if let Some(url) = args["url"].as_str().map(str::trim).filter(|u| !u.is_empty()) {
        d.open_url(url)?;
        done.push(format!("opened {url}"));
    }
    let build = match seat.platform {
        Platform::Ios => format!("Build for it with `xcodebuild … -destination 'id={}'`.", seat.id),
        Platform::Android => format!("Reach it with `adb -s {}`.", seat.id),
    };
    Ok(format!("{}. {build} Then `look`.", done.join(", ")))
}

fn look(exec: &dyn Exec, seat: &Seat, axe: Option<String>, args: &Value) -> Result<ToolResult, String> {
    let d = device(exec, seat, axe)?;
    let tree = if d.can_touch() { Some(d.tree()) } else { None };
    let elements = match &tree {
        Some(Ok(screen)) => tree::compact(screen),
        Some(Err(e)) => format!("The elements couldn't be read: {e}"),
        None => NO_AXE.to_owned(),
    };
    if args["screenshot"].as_bool() == Some(false) {
        return Ok(ToolResult::ok(elements));
    }
    let shot = d.screenshot()?;
    let ppp = match &tree {
        Some(Ok(screen)) if screen.size.w > 0. => screen.size.w / f64::from(shot.width().max(1)),
        _ => d.points_per_pixel(&shot),
    };
    let (w, h) = ((f64::from(shot.width()) * ppp).round().max(1.) as u32, (f64::from(shot.height()) * ppp).round().max(1.) as u32);
    let picture = image::imageops::resize(&shot, w, h, image::imageops::FilterType::Triangle);
    Ok(image_result(format!("{} · {w}×{h} points (the picture's own pixels).\n{elements}", seat.name), &picture))
}

/// How long a screen is given to settle after an action before it is read.
const SETTLE: Duration = Duration::from_millis(700);

fn act(exec: &dyn Exec, seat: &mut Seat, axe: Option<String>, args: &Value) -> Result<ToolResult, String> {
    let d = device(exec, seat, axe)?;
    let what = args["do"].as_str().unwrap_or_default().trim().to_owned();
    let target = tree::Target::of(args);
    let needs_touch = !matches!(what.as_str(), "launch" | "open_url");
    if needs_touch && !d.can_touch() {
        return Err(NO_AXE.into());
    }
    let screen = || d.tree();
    let point = |t: &tree::Target| -> Result<(f64, f64), String> {
        match t {
            tree::Target::Point(x, y) => Ok((*x, *y)),
            _ => tree::find(&screen()?, t),
        }
    };
    let mark_of = |described: String| args["step"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or(described);
    let at = seat.recording.as_ref().map(|r| r.started.elapsed().as_secs_f64());
    let done: String = match what.as_str() {
        "tap" => {
            let t = target.ok_or("Say what to tap: `target`, a label, an id or a point.")?;
            let (x, y) = point(&t)?;
            d.tap(x, y)?;
            format!("tapped {}", t.describe())
        }
        "type" => {
            let text = args["text"].as_str().filter(|t| !t.is_empty()).ok_or("Say what to type: `text`.")?;
            d.type_text(text)?;
            format!("typed {} characters", text.chars().count())
        }
        "scroll" | "swipe" => {
            let size = screen().map(|s| s.size).unwrap_or(tree::Frame { x: 0., y: 0., w: 402., h: 874. });
            let direction = args["direction"].as_str().unwrap_or(if what == "scroll" { "down" } else { "" });
            let from = match &target {
                Some(t) if what == "swipe" => point(t)?,
                _ => (size.x + size.w / 2., size.y + size.h / 2.),
            };
            // Scrolling down shows what is below: the finger moves up.
            let (dx, dy) = match (what.as_str(), direction) {
                ("scroll", "down") | ("swipe", "up") => (0., -size.h * 0.35),
                ("scroll", "up") | ("swipe", "down") => (0., size.h * 0.35),
                ("scroll", "right") | ("swipe", "left") => (-size.w * 0.35, 0.),
                ("scroll", "left") | ("swipe", "right") => (size.w * 0.35, 0.),
                _ => return Err("Say which way: `direction` up, down, left or right.".into()),
            };
            d.swipe(from, (from.0 + dx, from.1 + dy), 0.3)?;
            format!("{what} {direction}")
        }
        "key" => {
            let word = args["text"].as_str().or(args["key"].as_str()).unwrap_or_default();
            let key = Key::of(word).ok_or("Say which key in `text`: enter, delete, tab, escape, back or home.")?;
            d.key(key)?;
            format!("pressed {}", word.trim().to_lowercase())
        }
        "wait" => {
            let t = target.ok_or("Say what to wait for: `target`.")?;
            let gone = args["gone"].as_bool().unwrap_or(false);
            let timeout = args["timeout"].as_f64().unwrap_or(10.).clamp(0.5, 60.);
            let until = Instant::now() + Duration::from_secs_f64(timeout);
            loop {
                let here = screen().is_ok_and(|s| tree::has(&s, &t));
                if here != gone {
                    break format!("{} {} ", t.describe(), if gone { "is gone" } else { "is on the screen" });
                }
                if Instant::now() >= until {
                    return Err(format!(
                        "After {timeout}s {} {}. The screen now:\n{}",
                        t.describe(),
                        if gone { "is still there" } else { "still isn't on the screen" },
                        screen().map(|s| tree::compact(&s)).unwrap_or_default()
                    ));
                }
                exec.sleep(Duration::from_millis(400));
            }
        }
        "launch" => {
            let bundle = args["bundle"].as_str().map(str::to_owned).or(seat.bundle.clone()).ok_or("Which app? Give `bundle`.")?;
            d.launch(&bundle)?;
            seat.bundle = Some(bundle.clone());
            format!("launched {bundle}")
        }
        "open_url" => {
            let url = args["url"].as_str().filter(|u| !u.trim().is_empty()).ok_or("Which link? Give `url`.")?;
            d.open_url(url)?;
            format!("opened {url}")
        }
        "" => return Err("Say what to do: `do`.".into()),
        other => return Err(format!("{other} isn't something `act` does: tap, type, scroll, swipe, key, wait, launch or open_url.")),
    };
    if let (Some(at), Some(rec)) = (at, seat.recording.as_mut())
        && what != "wait"
    {
        rec.marks.push(Mark { at, what: mark_of(done.clone()) });
    }
    if what != "wait" {
        exec.sleep(SETTLE);
    }
    let after = d.tree().map(|s| tree::compact(&s)).unwrap_or_else(|e| format!("(the screen couldn't be read: {e})"));
    let text = format!("Done: {done}. The screen now:\n{after}");
    if args["look"].as_bool() == Some(true) {
        let shot = d.screenshot()?;
        let ppp = d.points_per_pixel(&shot);
        let (w, h) = ((f64::from(shot.width()) * ppp).round().max(1.) as u32, (f64::from(shot.height()) * ppp).round().max(1.) as u32);
        return Ok(image_result(text, &image::imageops::resize(&shot, w, h, image::imageops::FilterType::Triangle)));
    }
    Ok(ToolResult::ok(text))
}

fn record(exec: &dyn Exec, seat: &mut Seat, c: &Call, args: &Value) -> Result<ToolResult, String> {
    let d = device(exec, seat, None)?;
    match args["action"].as_str().unwrap_or_default() {
        "start" => {
            if let Some(rec) = &seat.recording {
                return Err(format!("You're recording {} already: `stop` it first.", rec.name));
            }
            std::fs::create_dir_all(&c.recordings).map_err(|e| format!("{} couldn't be made: {e}", c.recordings.display()))?;
            let base = slug(args["name"].as_str().unwrap_or("flow"));
            let name = (1..)
                .map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
                .find(|n| !c.recordings.join(format!("{n}.mp4")).exists())
                .unwrap_or(base);
            let path = c.recordings.join(format!("{name}.mp4"));
            let running = d.record(&path)?;
            // The recorder takes a moment to start: the flow's first step waits for it.
            exec.sleep(Duration::from_millis(1200));
            seat.recording = Some(Recording { name: name.clone(), path: path.clone(), running, started: Instant::now(), marks: Vec::new() });
            Ok(ToolResult::ok(format!("Recording {name} to {}. Act through the flow (give each action its `step`), then `record` `stop`.", path.display())))
        }
        "stop" => {
            let rec = seat.recording.take().ok_or("You aren't recording.")?;
            d.finish(rec.running, &rec.path)?;
            save_marks(&rec.path, &rec.marks);
            seat.last = Some(rec.name.clone());
            let length = frames::duration(&rec.path).map(frames::clock).unwrap_or_else(|| "?".into());
            let marks: Vec<String> = rec.marks.iter().map(|m| format!("{} {}", frames::clock(m.at), m.what)).collect();
            Ok(ToolResult::ok(format!(
                "Saved {} ({length}). Marked: {}. Next: `frames` to check each step.",
                rec.path.display(),
                if marks.is_empty() { "nothing".to_owned() } else { marks.join("; ") }
            )))
        }
        _ => Err("Say `action`: start or stop.".into()),
    }
}

/// The frames `args` ask for, saved, listed and shown on one sheet.
fn frames_of(c: &Call, args: &Value, last: Option<&str>) -> Result<ToolResult, String> {
    let asked = args["recording"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).or(last.map(str::to_owned)).or_else(|| newest_video(&c.recordings));
    let asked = asked.ok_or("There's no recording for this ticket yet: `record` a flow first.")?;
    let video = report::resolve(&c.recordings, &asked, "mp4").ok_or_else(|| format!("There's no recording {asked} for T-{}.", c.ticket))?;
    let duration = frames::duration(&video).ok_or("The recording couldn't be read (it may still be being written).")?;
    let max = args["max"].as_u64().unwrap_or(12).clamp(1, 24) as usize;
    let at: Vec<f64> = args["at"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
    let (marks, settled) = if at.is_empty() {
        let times = frames::sample_times(duration);
        let small = frames::grab(&video, &times, 160)?;
        let samples: Vec<(f64, image::GrayImage)> = times.iter().zip(small).filter_map(|(t, f)| Some((*t, frames::probe(&f?)))).collect();
        (load_marks(&video), frames::settled(&samples))
    } else {
        (Vec::new(), Vec::new())
    };
    let chosen = frames::choose(duration, &settled, &marks, &at, max);
    let times: Vec<f64> = chosen.iter().map(|(t, _)| *t).collect();
    let pictures = frames::grab(&video, &times, frames::SAVED_SIDE)?;
    let stem = video.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "flow".into());
    let dir = video.with_file_name(&stem);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut lines = Vec::new();
    let mut kept = Vec::new();
    for ((t, why), picture) in chosen.iter().zip(pictures) {
        let Some(picture) = picture else { continue };
        let n = kept.len() + 1;
        let path = dir.join(format!("{n:02}-{t:.1}s.jpg"));
        frames::save_jpeg(&picture, &path)?;
        let why = match why {
            frames::Why::Before(w) => format!("just before {w}"),
            frames::Why::After(w) => format!("after {w}"),
            frames::Why::Settled => "the screen settled".into(),
            frames::Why::Asked => "asked".into(),
        };
        lines.push(format!("{n}. {} — {why} — {}", frames::clock(*t), path.display()));
        kept.push(picture);
    }
    if kept.is_empty() {
        return Err("No frame could be read from the recording.".into());
    }
    let sheet = frames::sheet(&kept).map(|s| frames::fit(s, 1568)).ok_or("No sheet")?;
    Ok(image_result(format!("{} ({}), {} frames, numbered as on the sheet:\n{}", video.display(), frames::clock(duration), kept.len(), lines.join("\n")), &sheet))
}

fn newest_video(dir: &Path) -> Option<String> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "mp4"))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path().to_string_lossy().into_owned())
}

/// Where a ticket's recordings go: in the squad's folder, which every member
/// reads and the person can open, under `recordings/` — kept out of its git.
pub fn recordings_dir(squad_home: &Path, ticket: u32) -> PathBuf {
    squad_home.join(RECORDINGS).join(format!("T-{ticket}"))
}

/// The squad folder's recordings folder.
pub const RECORDINGS: &str = "recordings";

#[cfg(test)]
mod tests;
