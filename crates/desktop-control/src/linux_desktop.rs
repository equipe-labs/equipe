//! The screenshot, pointer and keyboard tools on Linux, through agent-desktop.
//!
//! On macOS and Windows the helper captures the screen and posts input with
//! the OS's own APIs; on Linux it asks the pinned agent-desktop (X11: XTEST
//! input, X11 capture; no outside tool), one short child process per step, as
//! the accessibility tools do. Every rule of [`crate::controller`] holds: a
//! click goes in only where the app the member means is, typing only while the
//! keyboard focus is in it, and the daemon's lease, person-first wait and send
//! gate come before any of it.
//!
//! What X11 gives that the Mac's Accessibility gave otherwise:
//! - *What is at a point*: the front window there, from agent-desktop's window
//!   list (the focused window first). An overlay that isn't a listed window
//!   isn't seen: a click that would reach it is taken as reaching the window.
//! - *What has the keyboard focus*: the focused window's app, and the element
//!   whose AT-SPI state is `focused` (a snapshot of that window). Typing goes
//!   to that element (`type --headed`, real keys), so the send gate sees the
//!   field it types into, a password field included.
//! - *The whole screen* under a compositing window manager (xfwm4 with its
//!   compositor on, GNOME on X11) can't be read from the root window: it is
//!   put together from each window's own picture, desktop first, the focused
//!   window last. What no window owns (panels drawn by the compositor) is black.

use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;

use crate::controller::{App, Display, Hit, Node, Permissions, Pointer, Window};
use crate::engine::Runner;
use crate::geometry::{Frame, Pt};
use crate::keys::{self, Combo};

/// What a member is told when the engine can't run at all.
pub const NO_ENGINE: &str = "The desktop engine (agent-desktop) isn't installed on this machine yet. Ask the person to run `equipe desktop setup` inside their desktop session.";

/// What a member is told where X11 would need a compositor turned off.
pub const COMPOSITING: &str = "The window manager composites, so the whole screen can't be read (xfwm4: Window Manager Tweaks › Compositor, untick \"Enable display compositing\"; or take one window with `window` / `app`).";

/// The element with the keyboard focus, as a snapshot of the focused window showed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Focused {
    /// agent-desktop's ref, for `type`.
    pub reference: String,
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    pub secure: bool,
}

/// A window from `list-windows`.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub id: String,
    pub pid: i32,
    pub app: String,
    pub title: String,
    pub frame: Frame,
    pub focused: bool,
}

/// An agent-desktop failure: its code (when it said one) and what to tell the member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed {
    pub code: Option<String>,
    pub text: String,
}

impl Failed {
    fn plain(text: impl Into<String>) -> Self {
        Self { code: None, text: text.into() }
    }
}

/// The Linux desktop, as the screenshot tools use it.
pub struct LinuxDesktop {
    runner: Option<Box<dyn Runner>>,
    /// agent-desktop's window ids (34–70 characters, opaque), numbered for
    /// members: `w3` is the third one this helper has seen.
    ids: Mutex<Vec<String>>,
    /// Why the last input didn't go in.
    failure: Mutex<Option<String>>,
}

impl LinuxDesktop {
    pub fn new(runner: Option<Box<dyn Runner>>) -> Self {
        Self { runner, ids: Mutex::new(Vec::new()), failure: Mutex::new(None) }
    }

    /// Run agent-desktop: the envelope's `data`, or why not.
    fn ad(&self, args: &[String], stdin: Option<&str>, secs: u64) -> Result<Value, Failed> {
        let runner = self.runner.as_ref().ok_or_else(|| Failed::plain(NO_ENGINE))?;
        let envelope = runner.run(args, stdin, Duration::from_secs(secs)).map_err(Failed::plain)?;
        if envelope.get("ok").and_then(Value::as_bool) == Some(true) {
            return Ok(envelope.get("data").cloned().unwrap_or(Value::Null));
        }
        let err = envelope.get("error").cloned().unwrap_or(Value::Null);
        let code = err.get("code").and_then(Value::as_str).map(str::to_owned);
        let text = if err.is_null() { "agent-desktop failed without saying why".to_owned() } else { crate::engine::compact::error(&err) };
        Err(Failed { code, text })
    }

    /// Input: on failure, kept for [`crate::controller::Desktop::input_failure`].
    fn input(&self, args: &[String], stdin: Option<&str>) {
        if let Err(failed) = self.ad(args, stdin, 15) {
            *self.failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(failed.text);
        }
    }

    fn number(&self, id: &str) -> u32 {
        let mut ids = self.ids.lock().unwrap_or_else(|e| e.into_inner());
        let index = ids.iter().position(|known| known == id).unwrap_or_else(|| {
            ids.push(id.to_owned());
            ids.len() - 1
        });
        index as u32 + 1
    }

    fn id_of(&self, number: u32) -> Option<String> {
        let ids = self.ids.lock().unwrap_or_else(|e| e.into_inner());
        ids.get((number as usize).checked_sub(1)?).cloned()
    }

    fn listed(&self) -> Vec<Listed> {
        self.ad(&args(&["list-windows"]), None, 10).map(|data| parse_windows(&data)).unwrap_or_default()
    }

    fn focused_window(&self) -> Option<Listed> {
        self.listed().into_iter().find(|w| w.focused)
    }

    /// A PNG from `screenshot` with `selector`.
    #[cfg(target_os = "linux")]
    fn shot(&self, selector: &[String]) -> Result<Vec<u8>, Failed> {
        let mut cli = args(&["screenshot"]);
        cli.extend_from_slice(selector);
        let data = self.ad(&cli, None, 20)?;
        let encoded = data.get("data").and_then(Value::as_str).ok_or_else(|| Failed::plain("agent-desktop's screenshot had no image"))?;
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|e| Failed::plain(e.to_string()))
    }

    /// The whole screen from each window's own picture (see the module's notes).
    #[cfg(target_os = "linux")]
    fn composed(&self, display: Frame) -> Result<image::RgbImage, String> {
        let windows = self.listed();
        let mut canvas = image::RgbImage::new(display.w.round().max(1.0) as u32, display.h.round().max(1.0) as u32);
        let mut painted = 0;
        for window in paint_order(&windows, display) {
            let Ok(png) = self.shot(&[kv("--window-id", &window.id)]) else { continue };
            let Ok(picture) = image::load_from_memory_with_format(&png, image::ImageFormat::Png) else { continue };
            let (w, h) = (window.frame.w.round().max(1.0) as u32, window.frame.h.round().max(1.0) as u32);
            let picture = if picture.width() == w && picture.height() == h { picture } else { picture.resize_exact(w, h, image::imageops::FilterType::Triangle) };
            image::imageops::overlay(&mut canvas, &picture.to_rgb8(), (window.frame.x - display.x).round() as i64, (window.frame.y - display.y).round() as i64);
            painted += 1;
        }
        if painted == 0 {
            return Err(format!("{COMPOSITING} No window could be captured either."));
        }
        Ok(canvas)
    }
}

/// The order to paint windows in to make the whole screen: a window covering
/// the display (the desktop) first, the focused window last, the rest in
/// between, the list's last first (front windows come first in it).
pub fn paint_order(windows: &[Listed], display: Frame) -> Vec<&Listed> {
    let covers = |w: &Listed| w.frame.w >= display.w && w.frame.h >= display.h;
    let mut order: Vec<&Listed> = windows.iter().filter(|w| covers(w) && !w.focused).collect();
    order.extend(windows.iter().rev().filter(|w| !covers(w) && !w.focused));
    order.extend(windows.iter().filter(|w| w.focused));
    order
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// `--flag=value`: a value starting with `-` (a negative coordinate) isn't read as a flag.
fn kv(flag: &str, value: &str) -> String {
    format!("{flag}={value}")
}

fn xy(at: Pt) -> String {
    format!("{},{}", at.x.round() as i64, at.y.round() as i64)
}

fn frame_of(bounds: &Value) -> Option<Frame> {
    let n = |k: &str| bounds.get(k).and_then(Value::as_f64);
    Some(Frame { x: n("x")?, y: n("y")?, w: n("width")?, h: n("height")? })
}

/// `list-displays` data: the primary display (the first when none says so).
pub fn parse_display(data: &Value) -> Option<Display> {
    let list = data.as_array().or_else(|| data.get("displays").and_then(Value::as_array))?;
    let display = list.iter().find(|d| d.get("is_primary").and_then(Value::as_bool) == Some(true)).or_else(|| list.first())?;
    let frame = frame_of(display.get("bounds")?)?;
    let backing = display.get("scale").and_then(Value::as_f64).filter(|s| *s > 0.0).unwrap_or(1.0);
    Some(Display { frame, backing })
}

/// `list-windows` data: the visible, unminimized windows, the focused one first.
pub fn parse_windows(data: &Value) -> Vec<Listed> {
    let list = data.as_array().or_else(|| data.get("windows").and_then(Value::as_array));
    let mut windows: Vec<Listed> = list
        .into_iter()
        .flatten()
        .filter(|w| w.get("visible").and_then(Value::as_bool) != Some(false) && w.get("minimized").and_then(Value::as_bool) != Some(true))
        .filter_map(|w| {
            Some(Listed {
                id: w.get("id")?.as_str()?.to_owned(),
                pid: i32::try_from(w.get("pid")?.as_i64()?).ok()?,
                app: w.get("app_name").and_then(Value::as_str).unwrap_or_default().to_owned(),
                title: w.get("title").and_then(Value::as_str).unwrap_or_default().to_owned(),
                frame: frame_of(w.get("bounds")?)?,
                focused: w.get("is_focused").and_then(Value::as_bool) == Some(true),
            })
        })
        .collect();
    windows.sort_by_key(|w| !w.focused);
    windows
}

/// `list-apps` data, apps with a window on screen first: a member's
/// headless browser has the same name as the person's, and no window.
pub fn parse_apps(data: &Value, windows: &[Listed]) -> Vec<App> {
    let list = data.get("apps").or(Some(data)).and_then(Value::as_array);
    let mut apps: Vec<App> = list
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some(App {
                pid: i32::try_from(a.get("pid")?.as_i64()?).ok()?,
                name: a.get("name")?.as_str()?.to_owned(),
                bundle_id: a.get("bundle_id").and_then(Value::as_str).filter(|b| !b.is_empty()).map(str::to_owned),
            })
        })
        .collect();
    apps.sort_by_key(|a| !windows.iter().any(|w| w.pid == a.pid));
    apps
}

/// The focused element in a snapshot's tree.
pub fn focused_in(tree: &Value) -> Option<Focused> {
    let states = tree.get("states").and_then(Value::as_array);
    let has = |s: &str| states.is_some_and(|list| list.iter().any(|v| v == s));
    if has("focused")
        && let Some(reference) = tree.get("ref_id").and_then(Value::as_str)
    {
        let text = |k: &str| tree.get(k).and_then(Value::as_str).map(str::to_owned);
        return Some(Focused {
            reference: reference.to_owned(),
            role: text("role").unwrap_or_default(),
            name: text("name").unwrap_or_default(),
            value: text("value"),
            secure: has("secure"),
        });
    }
    tree.get("children").and_then(Value::as_array)?.iter().find_map(focused_in)
}

/// What is at `at`: the focused window when it covers the point (it is in
/// front), else the first listed window that does.
pub fn hit_at(windows: &[Listed], at: Pt) -> Option<Hit> {
    let window = windows.iter().find(|w| w.focused && w.frame.contains(at)).or_else(|| windows.iter().find(|w| w.frame.contains(at)))?;
    Some(Hit { pid: window.pid, role: "AXWindow".into(), title: window.title.clone() })
}

/// A combo as agent-desktop's `press` takes it. The shortcut key members
/// write as `cmd` is Ctrl on Linux, as on Windows; `ctrl` stays Ctrl.
pub fn press_name(combo: Combo) -> String {
    let mut parts = Vec::new();
    if combo.flags & (keys::CONTROL | keys::COMMAND) != 0 {
        parts.push("ctrl".to_owned());
    }
    if combo.flags & keys::OPTION != 0 {
        parts.push("alt".to_owned());
    }
    if combo.flags & keys::SHIFT != 0 {
        parts.push("shift".to_owned());
    }
    let key = keys::key_name(combo.code).unwrap_or_else(|| format!("key {}", combo.code));
    parts.push(match key.as_str() {
        "," => "comma".to_owned(),
        "." => "period".to_owned(),
        "/" => "slash".to_owned(),
        ";" => "semicolon".to_owned(),
        "'" => "apostrophe".to_owned(),
        "[" => "leftbracket".to_owned(),
        "]" => "rightbracket".to_owned(),
        "\\" => "backslash".to_owned(),
        "-" => "minus".to_owned(),
        "=" => "equal".to_owned(),
        "`" => "grave".to_owned(),
        _ => key,
    });
    parts.join("+")
}

/// A character as a key `press` can send, where it is one (printable ASCII).
pub fn char_key(c: char) -> Option<String> {
    match c {
        ' ' => Some("space".into()),
        c if c.is_ascii_graphic() => Some(c.to_string()),
        _ => None,
    }
}

impl crate::controller::Desktop for LinuxDesktop {
    type Element = Focused;

    fn unavailable(&self, tool: &str) -> Option<&'static str> {
        (tool == "elements").then_some(
            "On Linux, read an app with `snapshot` / `find` (refs to act on; `bounds: true` for where things are), then `act`, or click by point after a screenshot.",
        )
    }
    /// No permission to grant on Linux: what the session lacks, the daemon
    /// says before a call gets here (`missing`).
    fn permissions(&self) -> Permissions {
        Permissions { screen: true, accessibility: true, post_events: true }
    }
    fn display(&self) -> Option<Display> {
        self.ad(&args(&["list-displays"]), None, 10).ok().as_ref().and_then(parse_display)
    }
    fn capture(&self, window: Option<u32>, width: usize, height: usize) -> Result<Vec<u8>, String> {
        let selector = match window {
            Some(n) => vec![kv("--window-id", &self.id_of(n).ok_or_else(|| format!("window w{n} is no longer on screen"))?)],
            None => vec![kv("--screen", "0")],
        };
        #[cfg(target_os = "linux")]
        {
            use image::codecs::jpeg::JpegEncoder;
            let picture = match self.shot(&selector) {
                Ok(png) => image::load_from_memory_with_format(&png, image::ImageFormat::Png).map_err(|e| e.to_string())?,
                Err(failed) if window.is_none() && failed.code.as_deref() == Some("PLATFORM_NOT_SUPPORTED") => {
                    let display = self.display().ok_or("no display")?;
                    image::DynamicImage::ImageRgb8(self.composed(display.frame)?)
                }
                Err(failed) if failed.code.as_deref() == Some("PLATFORM_NOT_SUPPORTED") => return Err(format!("{} {COMPOSITING}", failed.text)),
                Err(failed) => return Err(failed.text),
            };
            let (w, h) = (width.max(1) as u32, height.max(1) as u32);
            let picture = if picture.width() == w && picture.height() == h { picture } else { picture.resize_exact(w, h, image::imageops::FilterType::Triangle) };
            let mut jpeg = Vec::new();
            JpegEncoder::new_with_quality(&mut jpeg, 82).encode_image(&picture.to_rgb8()).map_err(|e| e.to_string())?;
            Ok(jpeg)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (selector, width, height);
            Err("screen capture through agent-desktop is built for Linux only".into())
        }
    }
    fn apps(&self) -> Vec<App> {
        let windows = self.listed();
        self.ad(&args(&["list-apps"]), None, 10).map(|data| parse_apps(&data, &windows)).unwrap_or_default()
    }
    fn open(&self, app: &str) -> Result<(), String> {
        self.ad(&args(&["launch", app]), None, 35).map(drop).map_err(|f| f.text)
    }
    fn windows(&self) -> Vec<Window> {
        self.listed()
            .into_iter()
            .map(|w| Window { id: self.number(&w.id), pid: w.pid, app: w.app, title: w.title, frame: w.frame })
            .collect()
    }
    fn raise(&self, pid: i32, window: Option<&Window>) -> Result<(), String> {
        let id = match window {
            Some(w) => self.id_of(w.id),
            None => self.listed().into_iter().find(|w| w.pid == pid).map(|w| w.id),
        }
        .ok_or("it has no window on screen")?;
        self.ad(&["--headed".into(), "focus-window".into(), kv("--window-id", &id)], None, 10).map(drop).map_err(|f| f.text)
    }
    fn focused_app(&self) -> Option<i32> {
        self.focused_window().map(|w| w.pid)
    }
    fn focused_element(&self) -> Option<Focused> {
        let window = self.focused_window()?;
        let data = self.ad(&["snapshot".into(), kv("--window-id", &window.id), "-i".into()], None, 10).ok()?;
        focused_in(data.get("tree")?)
    }
    fn hit(&self, at: Pt) -> Option<Hit> {
        hit_at(&self.listed(), at)
    }
    fn elements(&self, _pid: i32, _window: Option<&Window>, _max: usize) -> Vec<(Focused, Node)> {
        Vec::new()
    }
    fn same(&self, a: &Focused, b: &Focused) -> bool {
        a.role == b.role && a.name == b.name
    }
    fn frame(&self, _element: &Focused) -> Option<Frame> {
        None
    }
    fn value(&self, element: &Focused) -> Option<String> {
        if element.secure { None } else { element.value.clone() }
    }
    fn node(&self, element: &Focused) -> Option<Node> {
        Some(Node {
            role: crate::engine::gate_role(&element.role, element.secure),
            title: element.name.clone(),
            value: self.value(element),
            focused: true,
            ..Node::default()
        })
    }
    fn press(&self, _element: &Focused) -> Result<(), String> {
        Err("use `act` on a ref on Linux".into())
    }
    fn focus(&self, _element: &Focused) -> Result<(), String> {
        Err("use `act` (action focus) on a ref on Linux".into())
    }
    fn pointer(&self, action: Pointer) {
        let cli = match action {
            Pointer::Click { at, right, count } => vec![
                "--headed".into(),
                "mouse-click".into(),
                kv("--xy", &xy(at)),
                kv("--button", if right { "right" } else { "left" }),
                kv("--count", &count.max(1).to_string()),
            ],
            Pointer::Drag { from, to } => vec!["--headed".into(), "drag".into(), kv("--from-xy", &xy(from)), kv("--to-xy", &xy(to))],
            Pointer::Scroll { at, down, right } => {
                if down == 0 && right == 0 {
                    return;
                }
                // agent-desktop: positive dy scrolls up.
                vec![
                    "--headed".into(),
                    "mouse-wheel".into(),
                    kv("--x", &(at.x.round() as i64).to_string()),
                    kv("--y", &(at.y.round() as i64).to_string()),
                    kv("--dy", &(-down).to_string()),
                    kv("--dx", &right.to_string()),
                ]
            }
        };
        self.input(&cli, None);
    }
    fn type_char(&self, c: char) {
        match char_key(c) {
            Some(key) => self.input(&["--headed".into(), "press".into(), key], None),
            None => {
                *self.failure.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(format!("{c:?} isn't a key on this keyboard layout, and no focused text field takes it: type it with `act` (type) on the field's ref."));
            }
        }
    }
    fn key(&self, combo: Combo) {
        self.input(&["--headed".into(), "press".into(), press_name(combo)], None);
    }
    /// Real keys into the focused text field (`type --headed`, the text on
    /// stdin); where nothing focused takes text, key by key when every
    /// character is a key.
    fn type_run(&self, text: &str) -> Option<Result<(), String>> {
        let typed = match self.focused_element() {
            Some(field) => match self.ad(&["--headed".into(), "type".into(), field.reference, "--stdin".into()], Some(text), 30) {
                Ok(_) => Ok(()),
                // Not a text field (a terminal's keys, a canvas): key by key below.
                Err(Failed { code: Some(code), .. }) if code == "POLICY_DENIED" || code == "ACTION_NOT_SUPPORTED" => Err(None),
                Err(failed) => Err(Some(failed.text)),
            },
            None => Err(None),
        };
        Some(match typed {
            Ok(()) => Ok(()),
            Err(Some(why)) => Err(why),
            Err(None) => {
                let keys: Option<Vec<String>> = text.chars().map(char_key).collect();
                let Some(keys) = keys else {
                    return Some(Err("Nothing focused takes text, and some characters aren't keys: click into the field first, or use `act` (type) on its ref.".into()));
                };
                for key in keys {
                    self.input(&["--headed".into(), "press".into(), key], None);
                    if let Some(why) = self.input_failure() {
                        return Some(Err(why));
                    }
                }
                Ok(())
            }
        })
    }
    fn input_failure(&self) -> Option<String> {
        self.failure.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
    fn pause(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::{Controller, Desktop};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    type Calls = Arc<Mutex<Vec<(Vec<String>, Option<String>)>>>;

    /// agent-desktop as a script: each call's arguments recorded, answers by command.
    #[derive(Clone, Default)]
    struct Fake {
        calls: Calls,
        focused: Arc<Mutex<&'static str>>,
        refuse_type: Arc<Mutex<Option<&'static str>>>,
    }

    impl Runner for Fake {
        fn run(&self, args: &[String], stdin: Option<&str>, _timeout: Duration) -> Result<Value, String> {
            self.calls.lock().unwrap().push((args.to_vec(), stdin.map(str::to_owned)));
            let command = args.iter().find(|a| !a.starts_with('-')).cloned().unwrap_or_default();
            let focused = *self.focused.lock().unwrap();
            let window = |id: &str, pid: i64, app: &str, title: &str, x: f64, f: bool| {
                json!({ "id": id, "pid": pid, "app_name": app, "title": title, "bounds": { "x": x, "y": 0.0, "width": 400.0, "height": 300.0 }, "is_focused": f, "visible": true, "minimized": false })
            };
            let data = match command.as_str() {
                "list-displays" => json!([{ "bounds": { "x": 0.0, "y": 0.0, "width": 1280.0, "height": 800.0 }, "is_primary": true, "scale": 1.0 }]),
                "list-windows" => json!([
                    window("w-1.0-org-xfce-mousepad-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 100, "mousepad", "Untitled 1 - Mousepad", 0.0, focused == "mousepad"),
                    window("w-1.1-org-chromium-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", 200, "chrome", "Inbox", 500.0, focused == "chrome"),
                ]),
                "list-apps" => json!({ "apps": [{ "name": "chrome", "pid": 300 }, { "name": "chrome", "pid": 200 }, { "name": "mousepad", "pid": 100 }] }),
                "snapshot" => json!({ "tree": { "role": "window", "children": [{ "role": "group", "children": [
                    { "ref_id": "@s1:e1", "role": "textfield", "name": "Document", "value": "hello", "states": ["focused", "enabled"] }
                ] }] } }),
                "type" => {
                    if let Some(code) = *self.refuse_type.lock().unwrap() {
                        return Ok(json!({ "ok": false, "error": { "code": code, "message": "not a text target", "disposition": { "delivery": "not_delivered", "retry": "safe" } } }));
                    }
                    json!({ "action": "type" })
                }
                "press" if args.iter().any(|a| a == "alt+f4") => {
                    return Ok(json!({ "ok": false, "error": { "code": "POLICY_DENIED", "message": "blocked combo", "disposition": { "delivery": "not_delivered", "retry": "safe" } } }));
                }
                _ => json!({}),
            };
            Ok(json!({ "ok": true, "data": data }))
        }
    }

    fn desktop(fake: &Fake) -> LinuxDesktop {
        *fake.focused.lock().unwrap() = "mousepad";
        LinuxDesktop::new(Some(Box::new(fake.clone())))
    }

    fn ran(fake: &Fake, command: &str) -> Vec<(Vec<String>, Option<String>)> {
        fake.calls.lock().unwrap().iter().filter(|(a, _)| a.iter().any(|x| x == command)).cloned().collect()
    }

    #[test]
    fn agent_desktops_lists_read_as_the_controller_needs() {
        let fake = Fake::default();
        let d = desktop(&fake);
        let display = d.display().unwrap();
        assert_eq!((display.frame.w, display.frame.h, display.backing), (1280.0, 800.0, 1.0));
        let windows = d.windows();
        assert_eq!(windows.iter().map(|w| (w.id, w.pid)).collect::<Vec<_>>(), [(1, 100), (2, 200)], "focused first, numbered for members");
        assert_eq!(d.windows()[1].id, 2, "the same window keeps its number");
        // The person's browser before a member's headless one of the same name.
        assert_eq!(d.apps().iter().map(|a| a.pid).collect::<Vec<_>>(), [200, 100, 300]);
        assert_eq!(d.focused_app(), Some(100));
        let field = d.focused_element().unwrap();
        assert_eq!(field.reference, "@s1:e1");
        let node = d.node(&field).unwrap();
        assert_eq!((node.role.as_str(), node.title.as_str(), node.value.as_deref()), ("AXTextField", "Document", Some("hello")));
        assert!(ran(&fake, "snapshot")[0].0.iter().any(|a| a.starts_with("--window-id=w-1.0-org-xfce-mousepad")));
    }

    #[test]
    fn a_point_belongs_to_the_focused_window_first() {
        let fake = Fake::default();
        let d = desktop(&fake);
        assert_eq!(d.hit(Pt { x: 10.0, y: 10.0 }).map(|h| h.pid), Some(100));
        assert_eq!(d.hit(Pt { x: 600.0, y: 10.0 }).map(|h| h.pid), Some(200));
        assert_eq!(d.hit(Pt { x: 1000.0, y: 700.0 }), None, "no window there: nothing is known to be there");
    }

    #[test]
    fn pointer_and_keys_go_through_agent_desktop_headed() {
        let fake = Fake::default();
        let d = desktop(&fake);
        d.pointer(Pointer::Click { at: Pt { x: 10.4, y: -3.6 }, right: true, count: 2 });
        d.pointer(Pointer::Drag { from: Pt { x: 1.0, y: 2.0 }, to: Pt { x: 30.0, y: 40.0 } });
        d.pointer(Pointer::Scroll { at: Pt { x: 5.0, y: 6.0 }, down: 3, right: 0 });
        d.key(keys::parse("cmd+shift+s").unwrap());
        assert!(d.input_failure().is_none());
        let calls: Vec<Vec<String>> = fake.calls.lock().unwrap().iter().map(|(a, _)| a.clone()).collect();
        assert_eq!(calls[0], ["--headed", "mouse-click", "--xy=10,-4", "--button=right", "--count=2"]);
        assert_eq!(calls[1], ["--headed", "drag", "--from-xy=1,2", "--to-xy=30,40"]);
        assert_eq!(calls[2], ["--headed", "mouse-wheel", "--x=5", "--y=6", "--dy=-3", "--dx=0"], "down is agent-desktop's negative dy");
        assert_eq!(calls[3], ["--headed", "press", "ctrl+shift+s"], "cmd is Ctrl on Linux");
        d.key(keys::parse("alt+f4").unwrap());
        assert!(d.input_failure().unwrap().contains("POLICY_DENIED"), "a refusal is told, once");
        assert!(d.input_failure().is_none());
    }

    #[test]
    fn combos_and_characters_are_named_as_press_takes_them() {
        assert_eq!(press_name(keys::parse("ctrl+,").unwrap()), "ctrl+comma");
        assert_eq!(press_name(keys::parse("return").unwrap()), "return");
        assert_eq!(press_name(keys::parse("alt+left").unwrap()), "alt+left");
        assert_eq!(char_key(' ').as_deref(), Some("space"));
        assert_eq!(char_key('A').as_deref(), Some("A"));
        assert_eq!(char_key('é'), None);
    }

    #[test]
    fn typing_goes_to_the_focused_field_on_stdin_and_key_by_key_where_it_takes_none() {
        let fake = Fake::default();
        let d = desktop(&fake);
        assert_eq!(d.type_run("안녕 hello"), Some(Ok(())));
        let typed = ran(&fake, "type");
        assert_eq!(typed[0].0, ["--headed", "type", "@s1:e1", "--stdin"]);
        assert_eq!(typed[0].1.as_deref(), Some("안녕 hello"), "the text on stdin, never in the arguments");
        // A terminal's text node takes keys, not EditableText.
        *fake.refuse_type.lock().unwrap() = Some("POLICY_DENIED");
        assert_eq!(d.type_run("ls -a"), Some(Ok(())));
        let pressed: Vec<String> = ran(&fake, "press").iter().map(|(a, _)| a[2].clone()).collect();
        assert_eq!(pressed, ["l", "s", "space", "-", "a"]);
        assert!(d.type_run("é").unwrap().unwrap_err().contains("act"), "a character no key makes isn't guessed at");
    }

    #[test]
    fn the_screen_is_painted_desktop_first_and_the_focused_window_last() {
        let display = Frame { x: 0.0, y: 0.0, w: 1280.0, h: 800.0 };
        let w = |id: &str, w: f64, focused: bool| Listed { id: id.into(), pid: 1, app: "a".into(), title: id.into(), frame: Frame { x: 0.0, y: 0.0, w, h: if w > 1000.0 { 800.0 } else { 100.0 } }, focused };
        let windows = [w("front", 100.0, true), w("second", 100.0, false), w("third", 100.0, false), w("desktop", 1280.0, false)];
        let order: Vec<&str> = paint_order(&windows, display).iter().map(|w| w.id.as_str()).collect();
        assert_eq!(order, ["desktop", "third", "second", "front"]);
    }

    #[test]
    fn the_controller_clicks_types_and_presses_through_it_with_its_checks() {
        let fake = Fake::default();
        let mut c = Controller::new(desktop(&fake));
        // A point in the other app's window is refused before any input.
        let refused = c.call("m", "click", &json!({ "app": "mousepad", "x": 600, "y": 10 }));
        assert!(refused.is_error && refused.text.contains("not mousepad"), "{}", refused.text);
        assert!(ran(&fake, "mouse-click").is_empty());
        let clicked = c.call("m", "click", &json!({ "app": "mousepad", "x": 10, "y": 10 }));
        assert!(!clicked.is_error, "{}", clicked.text);
        let typed = c.call("m", "type_text", &json!({ "app": "mousepad", "text": "one\ntwo" }));
        assert!(!typed.is_error, "{}", typed.text);
        let stdin: Vec<String> = ran(&fake, "type").iter().filter_map(|(_, s)| s.clone()).collect();
        assert_eq!(stdin, ["one", "two"], "runs typed whole, the newline a return key between them");
        assert!(ran(&fake, "press").iter().any(|(a, _)| a[2] == "return"));
        // Focus in the other app: nothing typed.
        *fake.focused.lock().unwrap() = "chrome";
        let elsewhere = c.call("m", "type_text", &json!({ "app": "mousepad", "text": "x" }));
        assert!(elsewhere.is_error && elsewhere.text.contains("Keyboard focus is in chrome"), "{}", elsewhere.text);
        // The scene a send gate judges names the focused field.
        *fake.focused.lock().unwrap() = "mousepad";
        let scene = c.scene("m", "type_text", &json!({ "app": "mousepad", "text": "x" })).unwrap();
        assert_eq!(scene["focused"]["role"], "AXTextField");
        let refused = c.call("m", "elements", &json!({ "app": "mousepad" }));
        assert!(refused.is_error && refused.text.contains("snapshot"));
    }
}
