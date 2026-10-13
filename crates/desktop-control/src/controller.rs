//! What each desktop tool does, and the checks that keep a member's clicks and
//! keystrokes in the app it means.
//!
//! The Mac itself is behind [`Desktop`], so the rules — refuse a click whose
//! point Accessibility says belongs to another app, refuse to type while the
//! keyboard focus is elsewhere, stop typing the moment focus moves — are
//! tested without a screen. T-20 typed into the person's terminal once,
//! because the target window was behind it; these checks are why it can't
//! happen again.

use std::collections::HashMap;

use serde_json::Value;

use crate::geometry::{Frame, Mapping, Pt};
use crate::keys::{self, Combo};

/// What the person has allowed the helper.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Permissions {
    /// Screen & System Audio Recording.
    pub screen: bool,
    /// Accessibility: the UI tree, raising windows, and input.
    pub accessibility: bool,
    /// Posting input events (granted with Accessibility).
    pub post_events: bool,
}

impl Permissions {
    pub fn all(&self) -> bool {
        self.screen && self.accessibility && self.post_events
    }
}

/// The main display: where it is in points, and device pixels per point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Display {
    pub frame: Frame,
    pub backing: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub pid: i32,
    pub name: String,
    pub bundle_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub id: u32,
    pub pid: i32,
    pub app: String,
    pub title: String,
    pub frame: Frame,
}

/// What Accessibility says is at a point: the app a click there reaches.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub pid: i32,
    pub role: String,
    pub title: String,
}

/// One element of an app's UI.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    pub depth: usize,
    pub role: String,
    pub title: String,
    pub value: Option<String>,
    pub frame: Option<Frame>,
    pub actions: Vec<String>,
    pub focused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pointer {
    Click { at: Pt, right: bool, count: u32 },
    Drag { from: Pt, to: Pt },
    Scroll { at: Pt, down: i32, right: i32 },
}

/// The Mac, as the tools use it.
pub trait Desktop {
    /// An Accessibility element, kept between a member's `elements` call and
    /// the click or keystroke that uses it.
    type Element: Clone;

    fn permissions(&self) -> Permissions;
    fn display(&self) -> Option<Display>;
    /// A JPEG of the display, or of window `window`, `width`×`height` pixels.
    fn capture(&self, window: Option<u32>, width: usize, height: usize) -> Result<Vec<u8>, String>;
    /// Running apps that have a UI.
    fn apps(&self) -> Vec<App>;
    fn open(&self, app: &str) -> Result<(), String>;
    /// Normal windows on screen, front to back.
    fn windows(&self) -> Vec<Window>;
    /// Bring an app — and one of its windows — to the front.
    fn raise(&self, pid: i32, window: Option<&Window>) -> Result<(), String>;
    /// The app with keyboard focus: where typed text goes.
    fn focused_app(&self) -> Option<i32>;
    fn focused_element(&self) -> Option<Self::Element>;
    fn hit(&self, at: Pt) -> Option<Hit>;
    fn elements(&self, pid: i32, window: Option<&Window>, max: usize) -> Vec<(Self::Element, Node)>;
    fn same(&self, a: &Self::Element, b: &Self::Element) -> bool;
    fn frame(&self, element: &Self::Element) -> Option<Frame>;
    fn value(&self, element: &Self::Element) -> Option<String>;
    fn press(&self, element: &Self::Element) -> Result<(), String>;
    fn focus(&self, element: &Self::Element) -> Result<(), String>;
    fn pointer(&self, action: Pointer);
    fn type_char(&self, c: char);
    fn key(&self, combo: Combo);
    fn pause(&self, ms: u64);
}

/// A tool's answer: text, whether it was refused or failed, and a screenshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub text: String,
    pub is_error: bool,
    pub jpeg: Option<Vec<u8>>,
}

impl Reply {
    fn ok(text: impl Into<String>) -> Self {
        Self { text: text.into(), is_error: false, jpeg: None }
    }

    fn refuse(text: impl Into<String>) -> Self {
        Self { text: text.into(), is_error: true, jpeg: None }
    }
}

/// What the person runs to give the helper what it lacks.
pub const SETUP: &str = "equipe desktop setup";

/// What the helper remembers of each member between calls.
struct Member<E> {
    /// Its last screenshot: what its coordinates mean.
    mapping: Option<Mapping>,
    /// The app it last opened, focused or looked into.
    target: Option<i32>,
    /// Its last `elements` call: `e1` is the first.
    elements: Vec<(i32, E, Node)>,
}

impl<E> Default for Member<E> {
    fn default() -> Self {
        Self { mapping: None, target: None, elements: Vec::new() }
    }
}

pub struct Controller<D: Desktop> {
    pub desktop: D,
    members: HashMap<String, Member<D::Element>>,
}

type Step<T> = Result<T, Reply>;

/// How many elements `elements` lists unless asked.
const ELEMENTS: usize = 150;

impl<D: Desktop> Controller<D> {
    pub fn new(desktop: D) -> Self {
        Self { desktop, members: HashMap::new() }
    }

    /// Run `tool` for `member`.
    pub fn call(&mut self, member: &str, tool: &str, args: &Value) -> Reply {
        let result = match tool {
            "screenshot" => self.screenshot(member, args),
            "list_windows" => self.list_windows(member, args),
            "open_app" => self.open_app(member, args),
            "focus_window" => self.focus_window(member, args),
            "elements" => self.elements(member, args),
            "click" => self.click(member, args, 1),
            "double_click" => self.click(member, args, 2),
            "drag" => self.drag(member, args),
            "scroll" => self.scroll(member, args),
            "type_text" => self.type_text(member, args),
            "key" => self.key(member, args),
            other => Err(Reply::refuse(format!("There is no desktop tool called {other}."))),
        };
        result.unwrap_or_else(|refused| refused)
    }

    fn state(&mut self, member: &str) -> &mut Member<D::Element> {
        self.members.entry(member.to_owned()).or_default()
    }

    /// Refuse unless the person has allowed what `what` needs.
    fn need(&self, screen: bool, input: bool, what: &str) -> Step<()> {
        let have = self.desktop.permissions();
        let mut missing = Vec::new();
        if screen && !have.screen {
            missing.push("Screen Recording");
        }
        if input && !have.accessibility {
            missing.push("Accessibility");
        }
        if input && have.accessibility && !have.post_events {
            missing.push("sending input");
        }
        if missing.is_empty() {
            return Ok(());
        }
        Err(Reply::refuse(format!(
            "The desktop helper isn't allowed to {what} yet (missing: {}). Ask the person to run `{SETUP}` on the Mac — it says exactly what to switch on.",
            missing.join(", ")
        )))
    }

    fn name_of(&self, pid: Option<i32>) -> String {
        match pid {
            None => "no app".into(),
            Some(pid) => self.desktop.apps().into_iter().find(|a| a.pid == pid).map_or(format!("pid {pid}"), |a| a.name),
        }
    }

    fn find_app(&self, query: &str) -> Option<App> {
        let q = query.trim().to_lowercase();
        let apps = self.desktop.apps();
        let by = |f: &dyn Fn(&App) -> bool| apps.iter().find(|a| f(a)).cloned();
        by(&|a| a.bundle_id.as_deref().is_some_and(|b| b.to_lowercase() == q))
            .or_else(|| by(&|a| a.name.to_lowercase() == q))
            .or_else(|| by(&|a| a.name.to_lowercase().starts_with(&q)))
    }

    /// The app a call means: `app` when given, else the member's last one.
    fn target(&mut self, member: &str, args: &Value) -> Step<App> {
        if let Some(query) = text(args, "app") {
            let app = self
                .find_app(query)
                .ok_or_else(|| Reply::refuse(format!("No app called {query:?} is running. Open it with open_app.")))?;
            self.state(member).target = Some(app.pid);
            return Ok(app);
        }
        let Some(pid) = self.state(member).target else {
            return Err(Reply::refuse("Say which app with `app`, or open_app / focus_window first."));
        };
        self.desktop
            .apps()
            .into_iter()
            .find(|a| a.pid == pid)
            .ok_or_else(|| Reply::refuse("The app you were using has quit. Open it again with open_app."))
    }

    fn mapping(&mut self, member: &str) -> Step<Mapping> {
        if let Some(mapping) = self.state(member).mapping {
            return Ok(mapping);
        }
        let display = self.display()?;
        Ok(Mapping::for_frame(display.frame, display.backing))
    }

    fn display(&self) -> Step<Display> {
        self.desktop.display().ok_or_else(|| {
            Reply::refuse("No display to work on: the Mac has no screen attached, or no one is logged in at it. Ask the person.")
        })
    }

    /// The screen point (`x`, `y`) of the member's last screenshot stands for.
    fn point(&mut self, member: &str, args: &Value, x: &str, y: &str) -> Step<Pt> {
        let (Some(px), Some(py)) = (num(args, x), num(args, y)) else {
            return Err(Reply::refuse(format!("Give `{x}` and `{y}`, or an `element`.")));
        };
        let mapping = self.mapping(member)?;
        mapping.to_screen(px, py).ok_or_else(|| {
            Reply::refuse(format!("({px}, {py}) is outside your last screenshot ({}×{}).", mapping.width, mapping.height))
        })
    }

    fn element(&mut self, member: &str, args: &Value) -> Step<Option<(i32, D::Element, Node)>> {
        let Some(id) = text(args, "element") else {
            return Ok(None);
        };
        let index = id.trim().trim_start_matches('e').parse::<usize>().ok().filter(|n| *n > 0);
        let found = index.and_then(|n| self.state(member).elements.get(n - 1).cloned());
        found.map(Some).ok_or_else(|| Reply::refuse(format!("No element {id}: call `elements` again (ids last until your next call).")))
    }

    /// Bring `app` forward if it isn't, then make sure a click at `at` reaches
    /// it — Accessibility's hit test, not the window list, which an overlay
    /// (BetterDisplay's, say) makes lie.
    fn aim(&mut self, member: &str, app: &App, at: Pt) -> Step<Hit> {
        if self.desktop.focused_app() != Some(app.pid) {
            let _ = self.desktop.raise(app.pid, None);
            self.desktop.pause(300);
        }
        let (x, y) = self.mapping(member)?.to_image(at);
        match self.desktop.hit(at) {
            None => Err(Reply::refuse(format!("Accessibility can't tell what is at ({x}, {y}): nothing done."))),
            Some(hit) if hit.pid != app.pid => Err(Reply::refuse(format!(
                "At ({x}, {y}) is {} ({}), not {}: nothing done. Take a screenshot to see where things are, or bring {} forward with focus_window.",
                self.name_of(Some(hit.pid)),
                describe_hit(&hit),
                app.name,
                app.name
            ))),
            Some(hit) => Ok(hit),
        }
    }

    /// Refuse to type unless the keyboard focus is in `app`.
    fn keyboard(&self, app: &App, what: &str) -> Step<()> {
        let focused = self.desktop.focused_app();
        if focused == Some(app.pid) {
            return Ok(());
        }
        Err(Reply::refuse(format!(
            "Keyboard focus is in {}, not {}: nothing {what}. Bring {} forward with focus_window (or click into its field), then try again.",
            self.name_of(focused),
            app.name,
            app.name
        )))
    }

    fn screenshot(&mut self, member: &str, args: &Value) -> Step<Reply> {
        self.need(true, false, "see the screen")?;
        let display = self.display()?;
        let window = match (text(args, "window"), text(args, "app")) {
            (Some(id), _) => {
                let window = self.window(id)?;
                self.state(member).target = Some(window.pid);
                Some(window)
            }
            (None, Some(_)) => {
                let app = self.target(member, args)?;
                let window = self.desktop.windows().into_iter().find(|w| w.pid == app.pid);
                Some(window.ok_or_else(|| Reply::refuse(format!("{} has no window on screen.", app.name)))?)
            }
            (None, None) => None,
        };
        let frame = window.as_ref().map_or(display.frame, |w| w.frame);
        let mapping = Mapping::for_frame(frame, display.backing);
        let jpeg = self
            .desktop
            .capture(window.as_ref().map(|w| w.id), mapping.width, mapping.height)
            .map_err(|why| Reply::refuse(format!("Couldn't capture the screen: {why}")))?;
        self.state(member).mapping = Some(mapping);
        let what = match &window {
            Some(w) => format!("Window w{} — {}: {}", w.id, w.app, w.title),
            None => "The screen".into(),
        };
        let focus = self.name_of(self.desktop.focused_app());
        Ok(Reply {
            text: format!(
                "{what}, {}×{} pt, as a {}×{} image (1 px = {:.2} pt). Keyboard focus: {focus}. Coordinates are this image's pixels until your next screenshot.",
                frame.w.round(),
                frame.h.round(),
                mapping.width,
                mapping.height,
                mapping.scale()
            ),
            is_error: false,
            jpeg: Some(jpeg),
        })
    }

    fn window(&self, id: &str) -> Step<Window> {
        let number = id.trim().trim_start_matches('w').parse::<u32>().ok();
        number
            .and_then(|n| self.desktop.windows().into_iter().find(|w| w.id == n))
            .ok_or_else(|| Reply::refuse(format!("No window {id} on screen: list_windows shows what there is.")))
    }

    fn list_windows(&mut self, member: &str, args: &Value) -> Step<Reply> {
        self.need(true, false, "list windows")?;
        let mapping = self.mapping(member)?;
        let only = text(args, "app").map(|q| self.find_app(q).map(|a| a.pid).unwrap_or(-1));
        let focused = self.desktop.focused_app();
        let lines: Vec<String> = self
            .desktop
            .windows()
            .into_iter()
            .filter(|w| only.is_none_or(|pid| pid == w.pid))
            .map(|w| {
                let focus = if Some(w.pid) == focused { "  (keyboard focus)" } else { "" };
                format!("w{}  {} — {}  {}{focus}", w.id, w.app, w.title, mapping.describe(w.frame))
            })
            .collect();
        if lines.is_empty() {
            return Ok(Reply::ok("No windows on screen."));
        }
        Ok(Reply::ok(format!("{}\n(where: pixels of your last screenshot)", lines.join("\n"))))
    }

    fn open_app(&mut self, member: &str, args: &Value) -> Step<Reply> {
        let Some(query) = text(args, "app") else {
            return Err(Reply::refuse("Say which app to open."));
        };
        self.desktop.open(query).map_err(|why| Reply::refuse(format!("Couldn't open {query}: {why}")))?;
        let mut app = None;
        for _ in 0..40 {
            app = self.find_app(query);
            if app.is_some() {
                break;
            }
            self.desktop.pause(250);
        }
        let app = app.ok_or_else(|| Reply::refuse(format!("Asked macOS to open {query}, but it isn't running after 10 s.")))?;
        self.state(member).target = Some(app.pid);
        // Opened from the background, an app starts behind the one in front.
        if !self.desktop.permissions().accessibility {
            return Ok(Reply::ok(format!(
                "{} is open, but behind other windows: without Accessibility the helper can't bring it forward. Ask the person to run `{SETUP}`.",
                app.name
            )));
        }
        for _ in 0..8 {
            if self.desktop.focused_app() == Some(app.pid) {
                break;
            }
            let _ = self.desktop.raise(app.pid, None);
            self.desktop.pause(250);
        }
        let windows = self.desktop.windows().into_iter().filter(|w| w.pid == app.pid).count();
        if self.desktop.focused_app() == Some(app.pid) {
            Ok(Reply::ok(format!("{} is open and in front, with {windows} window(s) on screen.", app.name)))
        } else {
            Ok(Reply::ok(format!(
                "{} is open, but keyboard focus is still in {}. Use focus_window before typing.",
                app.name,
                self.name_of(self.desktop.focused_app())
            )))
        }
    }

    fn focus_window(&mut self, member: &str, args: &Value) -> Step<Reply> {
        self.need(false, true, "bring windows forward")?;
        let (app, window) = match text(args, "window") {
            Some(id) => {
                let window = self.window(id)?;
                let app = self
                    .desktop
                    .apps()
                    .into_iter()
                    .find(|a| a.pid == window.pid)
                    .ok_or_else(|| Reply::refuse(format!("{}'s app isn't running any more.", window.title)))?;
                self.state(member).target = Some(app.pid);
                (app, Some(window))
            }
            None => (self.target(member, args)?, None),
        };
        self.desktop.raise(app.pid, window.as_ref()).map_err(|why| Reply::refuse(format!("Couldn't bring {} forward: {why}", app.name)))?;
        self.desktop.pause(400);
        let focused = self.desktop.focused_app();
        if focused == Some(app.pid) {
            Ok(Reply::ok(format!("{} is in front, with the keyboard focus.", app.name)))
        } else {
            Err(Reply::refuse(format!(
                "Asked macOS to bring {} forward, but keyboard focus is still in {}.",
                app.name,
                self.name_of(focused)
            )))
        }
    }

    fn elements(&mut self, member: &str, args: &Value) -> Step<Reply> {
        self.need(false, true, "read apps' UI")?;
        let app = self.target(member, args)?;
        let window = text(args, "window").map(|id| self.window(id)).transpose()?;
        let max = args.get("max").and_then(Value::as_u64).map_or(ELEMENTS, |n| n.clamp(1, 1000) as usize);
        let found = self.desktop.elements(app.pid, window.as_ref(), max);
        let mapping = self.mapping(member)?;
        let mut lines = Vec::new();
        for (n, (_, node)) in found.iter().enumerate() {
            let mut line = format!("{}e{} {}", "  ".repeat(node.depth.min(12)), n + 1, label(node));
            if let Some(value) = node.value.as_deref().filter(|v| !v.is_empty()) {
                line.push_str(&format!(" = {:?}", clip(value, 80)));
            }
            if let Some(frame) = node.frame {
                line.push_str(&format!(" {}", mapping.describe(frame)));
            }
            if node.actions.iter().any(|a| a == "AXPress") {
                line.push_str(" [press]");
            }
            if node.focused {
                line.push_str(" (focused)");
            }
            lines.push(line);
        }
        let count = found.len();
        self.state(member).elements = found.into_iter().map(|(e, node)| (app.pid, e, node)).collect();
        let more = if count == max { format!(" (the first {max}; ask for more with `max`)") } else { String::new() };
        Ok(Reply::ok(format!("{}: {count} elements{more}. Where: pixels of your last screenshot.\n{}", app.name, lines.join("\n"))))
    }

    fn click(&mut self, member: &str, args: &Value, count: u32) -> Step<Reply> {
        self.need(false, true, "click")?;
        let right = text(args, "button") == Some("right");
        let (app, at, what) = match self.element(member, args)? {
            Some((pid, element, node)) => {
                let app = self.app_of(member, pid)?;
                // Accessibility's own press: no coordinates, nothing else in the way.
                if count == 1 && !right && node.actions.iter().any(|a| a == "AXPress") && self.desktop.press(&element).is_ok() {
                    return Ok(Reply::ok(format!("Pressed {} in {}.", label(&node), app.name)));
                }
                let frame = self.desktop.frame(&element).or(node.frame);
                let at = frame.map(|f| f.center()).ok_or_else(|| Reply::refuse(format!("{} has no place on screen.", label(&node))))?;
                (app, at, Some(label(&node)))
            }
            None => {
                let app = self.target(member, args)?;
                let at = self.point(member, args, "x", "y")?;
                (app, at, None)
            }
        };
        let hit = self.aim(member, &app, at)?;
        self.desktop.pointer(Pointer::Click { at, right, count });
        self.desktop.pause(150);
        let (x, y) = self.mapping(member)?.to_image(at);
        let verb = match (count, right) {
            (2, _) => "Double-clicked",
            (_, true) => "Right-clicked",
            _ => "Clicked",
        };
        Ok(Reply::ok(format!("{verb} {} in {} at ({x}, {y}).", what.unwrap_or_else(|| describe_hit(&hit)), app.name)))
    }

    fn app_of(&mut self, member: &str, pid: i32) -> Step<App> {
        self.state(member).target = Some(pid);
        self.desktop
            .apps()
            .into_iter()
            .find(|a| a.pid == pid)
            .ok_or_else(|| Reply::refuse("That element's app has quit. Open it again with open_app."))
    }

    fn drag(&mut self, member: &str, args: &Value) -> Step<Reply> {
        self.need(false, true, "drag")?;
        let app = self.target(member, args)?;
        let from = self.point(member, args, "from_x", "from_y")?;
        let to = self.point(member, args, "to_x", "to_y")?;
        self.aim(member, &app, from)?;
        self.desktop.pointer(Pointer::Drag { from, to });
        Ok(Reply::ok(format!("Dragged in {}.", app.name)))
    }

    fn scroll(&mut self, member: &str, args: &Value) -> Step<Reply> {
        self.need(false, true, "scroll")?;
        let (app, at) = match self.element(member, args)? {
            Some((pid, element, node)) => {
                let app = self.app_of(member, pid)?;
                let frame = self.desktop.frame(&element).or(node.frame);
                (app, frame.map(|f| f.center()).ok_or_else(|| Reply::refuse("That element has no place on screen."))?)
            }
            None => {
                let app = self.target(member, args)?;
                let at = match (num(args, "x"), num(args, "y")) {
                    (None, None) => {
                        let window = self.desktop.windows().into_iter().find(|w| w.pid == app.pid);
                        window.map(|w| w.frame.center()).ok_or_else(|| Reply::refuse(format!("{} has no window on screen.", app.name)))?
                    }
                    _ => self.point(member, args, "x", "y")?,
                };
                (app, at)
            }
        };
        self.aim(member, &app, at)?;
        let down = num(args, "down").unwrap_or(if num(args, "right").is_some() { 0.0 } else { 5.0 }) as i32;
        let right = num(args, "right").unwrap_or(0.0) as i32;
        self.desktop.pointer(Pointer::Scroll { at, down, right });
        Ok(Reply::ok(format!("Scrolled {} in {}.", if down < 0 { "up" } else { "down" }, app.name)))
    }

    fn type_text(&mut self, member: &str, args: &Value) -> Step<Reply> {
        self.need(false, true, "type")?;
        let Some(text) = text(args, "text").filter(|t| !t.is_empty()) else {
            return Err(Reply::refuse("Say what to type in `text`."));
        };
        let element = self.element(member, args)?;
        let app = match &element {
            Some((pid, ..)) => self.app_of(member, *pid)?,
            None => self.target(member, args)?,
        };
        if let Some((_, element, node)) = &element {
            if self.desktop.focused_app() != Some(app.pid) {
                let _ = self.desktop.raise(app.pid, None);
                self.desktop.pause(300);
            }
            self.desktop.focus(element).map_err(|why| Reply::refuse(format!("Couldn't put the focus on {}: {why}", label(node))))?;
            self.desktop.pause(150);
        }
        self.keyboard(&app, "typed")?;
        if let Some((_, element, node)) = &element
            && !self.desktop.focused_element().is_some_and(|f| self.desktop.same(&f, element))
        {
            return Err(Reply::refuse(format!("Focus is in {} but not on {}: nothing typed.", app.name, label(node))));
        }
        let total = text.chars().count();
        for (typed, c) in text.chars().enumerate() {
            // The person may click elsewhere mid-way: every character checks again.
            let focused = self.desktop.focused_app();
            if focused != Some(app.pid) {
                return Err(Reply::refuse(format!(
                    "Typed {typed} of {total} characters, then keyboard focus moved to {}: stopped there.",
                    self.name_of(focused)
                )));
            }
            match c {
                '\n' => self.desktop.key(Combo { code: keys::RETURN, flags: 0 }),
                '\t' => self.desktop.key(Combo { code: keys::TAB, flags: 0 }),
                c => self.desktop.type_char(c),
            }
        }
        self.desktop.pause(200);
        // Read it back as the app shows it: autocorrect and capitals change text.
        let now = self.desktop.focused_element().and_then(|f| self.desktop.value(&f));
        Ok(Reply::ok(match now {
            Some(value) => format!("Typed {total} characters into {}. The field now reads: {:?}", app.name, tail(&value, 300)),
            None => format!("Typed {total} characters into {}.", app.name),
        }))
    }

    fn key(&mut self, member: &str, args: &Value) -> Step<Reply> {
        self.need(false, true, "press keys")?;
        let combos = keys::parse_all(text(args, "keys").unwrap_or_default()).map_err(Reply::refuse)?;
        let app = self.target(member, args)?;
        for (n, combo) in combos.iter().enumerate() {
            let what = if n == 0 { "pressed".to_owned() } else { format!("more pressed after {n} of {}", combos.len()) };
            self.keyboard(&app, &what)?;
            self.desktop.key(*combo);
            self.desktop.pause(60);
        }
        Ok(Reply::ok(format!("Pressed {} in {}.", text(args, "keys").unwrap_or_default().trim(), app.name)))
    }
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty())
}

fn num(args: &Value, key: &str) -> Option<f64> {
    args.get(key).and_then(|v| v.as_f64().or_else(|| v.as_str()?.trim().parse().ok()))
}

fn label(node: &Node) -> String {
    if node.title.is_empty() { node.role.clone() } else { format!("{} {:?}", node.role, clip(&node.title, 60)) }
}

fn describe_hit(hit: &Hit) -> String {
    if hit.title.is_empty() { hit.role.clone() } else { format!("{} {:?}", hit.role, clip(&hit.title, 60)) }
}

fn clip(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("");
    if line.chars().count() > max || line.len() < text.len() {
        format!("{}…", line.chars().take(max).collect::<String>())
    } else {
        line.to_owned()
    }
}

fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max { text.to_owned() } else { format!("…{}", text.chars().skip(count - max).collect::<String>()) }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use serde_json::json;

    use super::*;

    const TEXTEDIT: i32 = 100;
    const GHOSTTY: i32 = 200;

    /// A Mac with TextEdit and the person's terminal on it.
    struct Fake {
        permissions: Permissions,
        focused: RefCell<i32>,
        /// What a click anywhere lands on.
        hit: RefCell<i32>,
        typed: RefCell<String>,
        pointer: RefCell<Vec<Pointer>>,
        raised: RefCell<Vec<i32>>,
        /// After this many characters, focus jumps to the terminal.
        steal_after: Option<usize>,
        /// Raising doesn't move focus (cooperative activation said no).
        stubborn: bool,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                permissions: Permissions { screen: true, accessibility: true, post_events: true },
                focused: RefCell::new(TEXTEDIT),
                hit: RefCell::new(TEXTEDIT),
                typed: RefCell::default(),
                pointer: RefCell::default(),
                raised: RefCell::default(),
                steal_after: None,
                stubborn: false,
            }
        }
    }

    impl Desktop for Fake {
        type Element = usize;

        fn permissions(&self) -> Permissions {
            self.permissions
        }
        fn display(&self) -> Option<Display> {
            Some(Display { frame: Frame { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 }, backing: 1.0 })
        }
        fn capture(&self, _: Option<u32>, w: usize, h: usize) -> Result<Vec<u8>, String> {
            Ok(vec![0xff, 0xd8, w as u8, h as u8])
        }
        fn apps(&self) -> Vec<App> {
            vec![
                App { pid: TEXTEDIT, name: "TextEdit".into(), bundle_id: Some("com.apple.TextEdit".into()) },
                App { pid: GHOSTTY, name: "Ghostty".into(), bundle_id: Some("com.mitchellh.ghostty".into()) },
            ]
        }
        fn open(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn windows(&self) -> Vec<Window> {
            vec![
                Window { id: 7, pid: GHOSTTY, app: "Ghostty".into(), title: "zsh".into(), frame: Frame { x: 0.0, y: 0.0, w: 800.0, h: 600.0 } },
                Window { id: 9, pid: TEXTEDIT, app: "TextEdit".into(), title: "Untitled".into(), frame: Frame { x: 100.0, y: 100.0, w: 600.0, h: 400.0 } },
            ]
        }
        fn raise(&self, pid: i32, _: Option<&Window>) -> Result<(), String> {
            self.raised.borrow_mut().push(pid);
            if !self.stubborn {
                *self.focused.borrow_mut() = pid;
                *self.hit.borrow_mut() = pid;
            }
            Ok(())
        }
        fn focused_app(&self) -> Option<i32> {
            Some(*self.focused.borrow())
        }
        fn focused_element(&self) -> Option<usize> {
            Some(1)
        }
        fn hit(&self, _: Pt) -> Option<Hit> {
            Some(Hit { pid: *self.hit.borrow(), role: "AXTextArea".into(), title: String::new() })
        }
        fn elements(&self, _: i32, _: Option<&Window>, _: usize) -> Vec<(usize, Node)> {
            vec![
                (0, Node { role: "AXButton".into(), title: "Save".into(), actions: vec!["AXPress".into()], ..Node::default() }),
                (1, Node { depth: 1, role: "AXTextArea".into(), frame: Some(Frame { x: 100.0, y: 100.0, w: 600.0, h: 400.0 }), ..Node::default() }),
            ]
        }
        fn same(&self, a: &usize, b: &usize) -> bool {
            a == b
        }
        fn frame(&self, _: &usize) -> Option<Frame> {
            None
        }
        fn value(&self, _: &usize) -> Option<String> {
            Some(self.typed.borrow().clone())
        }
        fn press(&self, _: &usize) -> Result<(), String> {
            Ok(())
        }
        fn focus(&self, _: &usize) -> Result<(), String> {
            Ok(())
        }
        fn pointer(&self, action: Pointer) {
            self.pointer.borrow_mut().push(action);
        }
        fn type_char(&self, c: char) {
            self.typed.borrow_mut().push(c);
            if self.steal_after == Some(self.typed.borrow().chars().count()) {
                *self.focused.borrow_mut() = GHOSTTY;
            }
        }
        fn key(&self, _: Combo) {
            self.typed.borrow_mut().push('⌨');
        }
        fn pause(&self, _: u64) {}
    }

    fn controller(fake: Fake) -> Controller<Fake> {
        Controller::new(fake)
    }

    #[test]
    fn typing_is_refused_while_the_focus_is_in_another_app() {
        let mut c = controller(Fake::new());
        *c.desktop.focused.borrow_mut() = GHOSTTY;
        let reply = c.call("frontend", "type_text", &json!({ "text": "안녕하세요", "app": "TextEdit" }));
        assert!(reply.is_error);
        assert!(reply.text.contains("Ghostty") && reply.text.contains("nothing typed"), "{}", reply.text);
        assert_eq!(*c.desktop.typed.borrow(), "", "not a single character reached the terminal");
        assert!(c.desktop.raised.borrow().is_empty(), "typing never pulls another app forward on its own");

        let keys = c.call("frontend", "key", &json!({ "keys": "return" }));
        assert!(keys.is_error && keys.text.contains("Ghostty"), "{}", keys.text);
        assert_eq!(*c.desktop.typed.borrow(), "");
    }

    #[test]
    fn typing_in_the_app_with_focus_goes_through_and_reads_back() {
        let mut c = controller(Fake::new());
        let reply = c.call("frontend", "type_text", &json!({ "text": "hello 안녕하세요\n", "app": "textedit" }));
        assert!(!reply.is_error, "{}", reply.text);
        assert_eq!(*c.desktop.typed.borrow(), "hello 안녕하세요⌨", "the newline is the return key");
        assert!(reply.text.contains("hello 안녕하세요"), "{}", reply.text);
        // The member's app is remembered for the next call.
        assert!(!c.call("frontend", "key", &json!({ "keys": "cmd+s" })).is_error);
    }

    #[test]
    fn typing_stops_the_moment_focus_moves_away() {
        let mut c = controller(Fake { steal_after: Some(3), ..Fake::new() });
        let reply = c.call("frontend", "type_text", &json!({ "text": "abcdef", "app": "TextEdit" }));
        assert!(reply.is_error && reply.text.contains("Typed 3 of 6") && reply.text.contains("Ghostty"), "{}", reply.text);
        assert_eq!(*c.desktop.typed.borrow(), "abc");
    }

    #[test]
    fn a_click_that_would_land_in_another_app_is_refused() {
        // TextEdit won't come forward; the terminal stays on top of the point.
        let mut c = controller(Fake { stubborn: true, ..Fake::new() });
        *c.desktop.focused.borrow_mut() = GHOSTTY;
        *c.desktop.hit.borrow_mut() = GHOSTTY;
        let reply = c.call("frontend", "click", &json!({ "x": 400, "y": 300, "app": "TextEdit" }));
        assert!(reply.is_error && reply.text.contains("Ghostty") && reply.text.contains("nothing done"), "{}", reply.text);
        assert!(c.desktop.pointer.borrow().is_empty(), "no click posted");
        assert_eq!(*c.desktop.raised.borrow(), [TEXTEDIT], "it tried bringing TextEdit forward first");
    }

    #[test]
    fn a_click_is_posted_at_the_screen_point_the_image_pixel_stands_for() {
        let mut c = controller(Fake::new());
        // 1440×900 pt at 1×: the image is 1357×848, so pixels are a little larger than points.
        let shot = c.call("frontend", "screenshot", &json!({}));
        assert!(!shot.is_error && shot.jpeg.is_some(), "{}", shot.text);
        let reply = c.call("frontend", "click", &json!({ "x": 678.5, "y": 424, "app": "TextEdit" }));
        assert!(!reply.is_error, "{}", reply.text);
        let Pointer::Click { at, .. } = c.desktop.pointer.borrow()[0] else { panic!() };
        assert!((at.x - 720.0).abs() < 1.0 && (at.y - 450.0).abs() < 1.0, "{at:?}");

        let window = c.call("frontend", "screenshot", &json!({ "window": "w9" }));
        assert!(window.text.contains("Untitled"), "{}", window.text);
        c.call("frontend", "click", &json!({ "x": 0, "y": 0 }));
        let Pointer::Click { at, .. } = c.desktop.pointer.borrow()[1] else { panic!() };
        assert_eq!(at, Pt { x: 100.0, y: 100.0 }, "a window's pixels start at the window");
    }

    #[test]
    fn elements_are_pressed_through_accessibility_or_clicked_where_they_are() {
        let mut c = controller(Fake::new());
        let tree = c.call("frontend", "elements", &json!({ "app": "TextEdit" }));
        assert!(tree.text.contains("e1 AXButton \"Save\"") && tree.text.contains("[press]"), "{}", tree.text);
        let pressed = c.call("frontend", "click", &json!({ "element": "e1" }));
        assert!(pressed.text.starts_with("Pressed"), "{}", pressed.text);
        assert!(c.desktop.pointer.borrow().is_empty(), "no coordinates involved");
        let clicked = c.call("frontend", "click", &json!({ "element": "e2" }));
        assert!(!clicked.is_error, "{}", clicked.text);
        let Pointer::Click { at, .. } = c.desktop.pointer.borrow()[0] else { panic!() };
        assert_eq!(at, Pt { x: 400.0, y: 300.0 }, "the element's centre");
        assert!(c.call("frontend", "click", &json!({ "element": "e9" })).text.contains("call `elements` again"));
        assert!(c.call("pm", "click", &json!({ "element": "e1" })).is_error, "ids are each member's own");
    }

    #[test]
    fn without_the_permissions_every_tool_says_how_to_set_them_up() {
        let mut c = controller(Fake { permissions: Permissions::default(), ..Fake::new() });
        for (tool, args) in [
            ("screenshot", json!({})),
            ("click", json!({ "x": 1, "y": 1, "app": "TextEdit" })),
            ("type_text", json!({ "text": "a", "app": "TextEdit" })),
            ("elements", json!({ "app": "TextEdit" })),
        ] {
            let reply = c.call("frontend", tool, &args);
            assert!(reply.is_error && reply.text.contains(SETUP), "{tool}: {}", reply.text);
        }
        assert!(c.call("frontend", "screenshot", &json!({})).text.contains("Screen Recording"));
        assert_eq!(*c.desktop.typed.borrow(), "");
    }

    #[test]
    fn a_member_says_which_app_it_means() {
        let mut c = controller(Fake::new());
        let reply = c.call("frontend", "type_text", &json!({ "text": "x" }));
        assert!(reply.is_error && reply.text.contains("Say which app"), "{}", reply.text);
        assert!(c.call("frontend", "click", &json!({ "x": 1, "y": 1, "app": "Teams" })).text.contains("No app called"));
        let opened = c.call("frontend", "open_app", &json!({ "app": "TextEdit" }));
        assert!(opened.text.contains("in front"), "{}", opened.text);
        assert!(!c.call("frontend", "type_text", &json!({ "text": "x" })).is_error, "open_app sets the app");
    }

    #[test]
    fn focus_window_reports_when_macos_wont_bring_the_app_forward() {
        let mut c = controller(Fake { stubborn: true, ..Fake::new() });
        *c.desktop.focused.borrow_mut() = GHOSTTY;
        let reply = c.call("frontend", "focus_window", &json!({ "window": "w9" }));
        assert!(reply.is_error && reply.text.contains("still in Ghostty"), "{}", reply.text);
        let listed = c.call("frontend", "list_windows", &json!({}));
        assert!(listed.text.contains("w9  TextEdit — Untitled") && listed.text.contains("w7  Ghostty — zsh  (0, 0)"), "{}", listed.text);
    }
}
