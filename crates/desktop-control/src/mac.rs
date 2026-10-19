//! The Mac behind the desktop tools: ScreenCaptureKit for the screen, CGEvent
//! for input, Accessibility (AXUIElement) for the UI and for aiming — what the
//! T-20 probe proved under a LaunchAgent.

use std::ffi::c_void;
use std::path::Path;
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AnyThread as _;
use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber, NSString};
use objc2_screen_capture_kit::{SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow};
use serde_json::{Value, json};

use crate::controller::{App, Controller, Desktop, Display, Hit, Node, Permissions, Pointer, Window};
use crate::geometry::{Frame, Pt};
use crate::keys::Combo;

type Ref = *const c_void;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CGSize {
    w: f64,
    h: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayCopyDisplayMode(display: u32) -> Ref;
    fn CGDisplayModeGetWidth(mode: Ref) -> usize;
    fn CGDisplayModeGetPixelWidth(mode: Ref) -> usize;
    fn CGDisplayModeRelease(mode: Ref);
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
    fn CGSessionCopyCurrentDictionary() -> Ref;
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> Ref;
    fn CGEventCreateMouseEvent(source: Ref, kind: u32, at: CGPoint, button: u32) -> Ref;
    fn CGEventCreateKeyboardEvent(source: Ref, key: u16, down: bool) -> Ref;
    fn CGEventCreateScrollWheelEvent2(source: Ref, units: u32, count: u32, wheel1: i32, wheel2: i32, wheel3: i32) -> Ref;
    fn CGEventKeyboardSetUnicodeString(event: Ref, len: usize, text: *const u16);
    fn CGEventSetFlags(event: Ref, flags: u64);
    fn CGEventSetIntegerValueField(event: Ref, field: u32, value: i64);
    fn CGEventSetLocation(event: Ref, at: CGPoint);
    fn CGEventPost(tap: u32, event: Ref);
    fn CGImageRetain(image: Ref) -> Ref;
    fn CGImageRelease(image: Ref);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(any: Ref);
    fn CFRetain(any: Ref) -> Ref;
    fn CFEqual(a: Ref, b: Ref) -> bool;
    fn CFDataCreateMutable(allocator: Ref, capacity: isize) -> Ref;
    fn CFDataGetBytePtr(data: Ref) -> *const u8;
    fn CFDataGetLength(data: Ref) -> isize;
    fn CFRunLoopRun();
    fn CFRunLoopGetMain() -> Ref;
    fn CFAbsoluteTimeGetCurrent() -> f64;
    fn CFRunLoopTimerCreate(allocator: Ref, fire: f64, interval: f64, flags: u64, order: isize, callout: extern "C" fn(Ref, *mut c_void), context: Ref) -> Ref;
    fn CFRunLoopAddTimer(run_loop: Ref, timer: Ref, mode: Ref);
    static kCFRunLoopCommonModes: Ref;
    static kCFBooleanTrue: Ref;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: Ref) -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> Ref;
    fn AXUIElementCreateSystemWide() -> Ref;
    fn AXUIElementCopyAttributeValue(element: Ref, attribute: Ref, value: *mut Ref) -> i32;
    fn AXUIElementSetAttributeValue(element: Ref, attribute: Ref, value: Ref) -> i32;
    fn AXUIElementPerformAction(element: Ref, action: Ref) -> i32;
    fn AXUIElementCopyActionNames(element: Ref, names: *mut Ref) -> i32;
    fn AXUIElementCopyElementAtPosition(element: Ref, x: f32, y: f32, hit: *mut Ref) -> i32;
    fn AXUIElementGetPid(element: Ref, pid: *mut i32) -> i32;
    fn AXUIElementSetMessagingTimeout(element: Ref, seconds: f32) -> i32;
    fn AXValueGetValue(value: Ref, kind: u32, out: *mut c_void) -> bool;
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageDestinationCreateWithData(data: Ref, kind: Ref, count: usize, options: Ref) -> Ref;
    fn CGImageDestinationAddImage(destination: Ref, image: Ref, properties: Ref);
    fn CGImageDestinationFinalize(destination: Ref) -> bool;
    static kCGImageDestinationLossyCompressionQuality: Ref;
}

/// A Core Foundation object we hold a reference to.
pub struct Cf(Ref);

impl Cf {
    /// Take ownership of a +1 reference.
    fn owned(any: Ref) -> Option<Self> {
        (!any.is_null()).then_some(Self(any))
    }

    /// Hold a reference of our own to something borrowed.
    fn retained(any: Ref) -> Option<Self> {
        // SAFETY: a valid CF object, retained before use.
        (!any.is_null()).then(|| Self(unsafe { CFRetain(any) }))
    }
}

impl Clone for Cf {
    fn clone(&self) -> Self {
        // SAFETY: self.0 is a live CF object.
        Self(unsafe { CFRetain(self.0) })
    }
}

impl Drop for Cf {
    fn drop(&mut self) {
        // SAFETY: we hold one reference.
        unsafe { CFRelease(self.0) }
    }
}

fn ns(text: &str) -> Retained<NSString> {
    NSString::from_str(text)
}

fn ptr<T: objc2::Message>(object: &Retained<T>) -> Ref {
    Retained::as_ptr(object) as Ref
}

/// A +1 CF object as an Objective-C one (toll-free bridged).
unsafe fn object(any: Ref) -> Option<Retained<AnyObject>> {
    // SAFETY: the caller passes a +1 reference, or null.
    unsafe { Retained::from_raw(any as *mut AnyObject) }
}

fn attr(element: Ref, name: &str) -> Option<Retained<AnyObject>> {
    let mut value: Ref = std::ptr::null();
    let name = ns(name);
    // SAFETY: element is an AXUIElement; the value comes back +1.
    let err = unsafe { AXUIElementCopyAttributeValue(element, ptr(&name), &mut value) };
    if err != 0 {
        return None;
    }
    // SAFETY: +1 from the copy above.
    unsafe { object(value) }
}

fn string(element: Ref, name: &str) -> Option<String> {
    attr(element, name)?.downcast::<NSString>().ok().map(|s| s.to_string())
}

fn flag(element: Ref, name: &str) -> bool {
    attr(element, name).and_then(|v| v.downcast::<NSNumber>().ok()).is_some_and(|n| n.boolValue())
}

fn set_true(element: Ref, name: &str) -> i32 {
    let name = ns(name);
    // SAFETY: element is an AXUIElement; kCFBooleanTrue is a constant.
    unsafe { AXUIElementSetAttributeValue(element, ptr(&name), kCFBooleanTrue) }
}

fn perform(element: Ref, action: &str) -> i32 {
    let action = ns(action);
    // SAFETY: element is an AXUIElement.
    unsafe { AXUIElementPerformAction(element, ptr(&action)) }
}

fn pid_of(element: Ref) -> Option<i32> {
    let mut pid = 0;
    // SAFETY: element is an AXUIElement.
    (unsafe { AXUIElementGetPid(element, &mut pid) } == 0 && pid != 0).then_some(pid)
}

fn frame_of(element: Ref) -> Option<Frame> {
    let position = attr(element, "AXPosition")?;
    let size = attr(element, "AXSize")?;
    let mut point = CGPoint::default();
    let mut extent = CGSize::default();
    // SAFETY: AXPosition / AXSize are AXValues of these kinds (1: CGPoint, 2: CGSize).
    let ok = unsafe {
        AXValueGetValue(ptr(&position), 1, &mut point as *mut _ as *mut c_void)
            && AXValueGetValue(ptr(&size), 2, &mut extent as *mut _ as *mut c_void)
    };
    ok.then_some(Frame { x: point.x, y: point.y, w: extent.w, h: extent.h })
}

fn children(element: Ref, name: &str) -> Vec<Cf> {
    let Some(list) = attr(element, name) else { return Vec::new() };
    let Ok(list) = list.downcast::<NSArray>() else { return Vec::new() };
    list.iter().filter_map(|child| Cf::retained(ptr(&child))).collect()
}

fn actions(element: Ref) -> Vec<String> {
    let mut names: Ref = std::ptr::null();
    // SAFETY: element is an AXUIElement; the array comes back +1.
    if unsafe { AXUIElementCopyActionNames(element, &mut names) } != 0 {
        return Vec::new();
    }
    // SAFETY: +1 from the copy above.
    let Some(names) = (unsafe { object(names) }) else { return Vec::new() };
    let Ok(names) = names.downcast::<NSArray>() else { return Vec::new() };
    names.iter().filter_map(|n| n.downcast::<NSString>().ok().map(|s| s.to_string())).collect()
}

fn system() -> Cf {
    // SAFETY: the system-wide element, +1.
    Cf(unsafe { AXUIElementCreateSystemWide() })
}

fn application(pid: i32) -> Cf {
    // SAFETY: an element for pid's app, +1.
    Cf(unsafe { AXUIElementCreateApplication(pid) })
}

/// What an element is called: its title, else its description or placeholder.
fn title_of(e: Ref) -> String {
    string(e, "AXTitle")
        .filter(|t| !t.is_empty())
        .or_else(|| string(e, "AXDescription").filter(|t| !t.is_empty()))
        .or_else(|| string(e, "AXPlaceholderValue"))
        .unwrap_or_default()
}

/// Roles that only hold others: listed only when they say something.
const CONTAINERS: &[&str] = &["AXGroup", "AXUnknown", "AXSplitGroup", "AXLayoutArea", "AXLayoutItem", "AXScrollArea"];

pub struct Mac;

impl Mac {
    pub fn new() -> Self {
        // An app that doesn't answer shouldn't hold a member for six seconds a call.
        // SAFETY: the system-wide element.
        unsafe { AXUIElementSetMessagingTimeout(system().0, 3.0) };
        Mac
    }

    fn walk(&self, element: &Cf, depth: usize, max: usize, out: &mut Vec<(Cf, Node)>) {
        if out.len() >= max || depth > 40 {
            return;
        }
        let e = element.0;
        let role = string(e, "AXRole").unwrap_or_default();
        let title = title_of(e);
        let value = string(e, "AXValue");
        let actions: Vec<String> = actions(e).into_iter().filter(|a| a != "AXShowMenu" && a != "AXScrollToVisible").collect();
        let quiet = CONTAINERS.contains(&role.as_str()) && title.is_empty() && value.as_deref().is_none_or(str::is_empty) && actions.is_empty();
        let next = if quiet {
            depth
        } else {
            let node = Node { depth, role, title, value, frame: frame_of(e), actions, focused: flag(e, "AXFocused") };
            out.push((element.clone(), node));
            depth + 1
        };
        for child in children(e, "AXChildren") {
            self.walk(&child, next, max, out);
        }
    }

    /// The AX window of `pid` that is `window` on screen, else its front one.
    fn ax_window(&self, pid: i32, window: Option<&Window>) -> Option<Cf> {
        let app = application(pid);
        let windows = children(app.0, "AXWindows");
        match window {
            Some(w) => windows.into_iter().find(|ax| {
                frame_of(ax.0).is_some_and(|f| (f.x - w.frame.x).abs() < 4.0 && (f.y - w.frame.y).abs() < 4.0 && (f.w - w.frame.w).abs() < 4.0)
            }),
            None => attr(app.0, "AXFocusedWindow")
                .and_then(|w| Cf::retained(ptr(&w)))
                .or_else(|| attr(app.0, "AXMainWindow").and_then(|w| Cf::retained(ptr(&w))))
                .or_else(|| windows.into_iter().next()),
        }
    }

    pub fn status(&self) -> Value {
        let permissions = self.permissions();
        let display = self.display().map(|d| json!({ "width": d.frame.w, "height": d.frame.h, "scale": d.backing }));
        // SAFETY: the session dictionary, +1, or null without a GUI session.
        let session = unsafe { object(CGSessionCopyCurrentDictionary()) }.map(|dict| {
            // SAFETY: a CFDictionary, toll-free bridged.
            let dict: Retained<NSDictionary<NSString, AnyObject>> = unsafe { Retained::cast_unchecked(dict) };
            let yes = |key: &str| dict.objectForKey(&ns(key)).and_then(|v| v.downcast::<NSNumber>().ok()).is_some_and(|n| n.boolValue());
            json!({ "on_console": yes("kCGSSessionOnConsoleKey"), "locked": yes("CGSSessionScreenIsLocked") })
        });
        json!({
            "screen": permissions.screen,
            "accessibility": permissions.accessibility,
            "post_events": permissions.post_events,
            "display": display,
            "session": session,
            "exe": std::env::current_exe().ok(),
            "pid": std::process::id(),
        })
    }

    /// Ask macOS for every permission: puts the helper in both lists, with a
    /// prompt where macOS shows one.
    pub fn request(&self) {
        // SAFETY: these show the system's prompts and return the current state.
        unsafe {
            CGRequestScreenCaptureAccess();
            CGRequestPostEventAccess();
            let options = NSDictionary::from_slices(&[&*ns("AXTrustedCheckOptionPrompt")], &[&*NSNumber::new_bool(true)]);
            AXIsProcessTrustedWithOptions(ptr(&options));
        }
        // On macOS 26 asking isn't enough to be listed under Screen Recording:
        // an attempt to capture is what puts the helper there.
        if !self.permissions().screen
            && let Ok(image) = capture(None, 64, 36)
        {
            // SAFETY: retained by capture().
            unsafe { CGImageRelease(image) };
        }
    }
}

impl Default for Mac {
    fn default() -> Self {
        Self::new()
    }
}

fn post(event: Ref) {
    if event.is_null() {
        return;
    }
    // SAFETY: a synthesised event we own, posted at the HID tap.
    unsafe {
        CGEventPost(0, event);
        CFRelease(event);
    }
}

fn mouse(kind: u32, at: CGPoint, button: u32, clicks: i64) {
    // SAFETY: a new mouse event.
    let event = unsafe { CGEventCreateMouseEvent(std::ptr::null(), kind, at, button) };
    if clicks > 0 && !event.is_null() {
        // kCGMouseEventClickState
        unsafe { CGEventSetIntegerValueField(event, 1, clicks) };
    }
    post(event);
}

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

impl Desktop for Mac {
    type Element = Cf;

    fn permissions(&self) -> Permissions {
        // SAFETY: plain queries.
        unsafe {
            Permissions {
                // The preflight can say no while macOS allows the capture (seen on
                // macOS 26: tccd "Allowed (System Set)", preflight false), so
                // ScreenCaptureKit itself has the last word.
                screen: CGPreflightScreenCaptureAccess() || shareable().is_ok(),
                accessibility: AXIsProcessTrusted(),
                post_events: CGPreflightPostEventAccess(),
            }
        }
    }

    fn display(&self) -> Option<Display> {
        // SAFETY: display queries on the main display.
        unsafe {
            let id = CGMainDisplayID();
            let bounds = CGDisplayBounds(id);
            if bounds.size.w < 1.0 {
                return None;
            }
            let mode = CGDisplayCopyDisplayMode(id);
            let backing = if mode.is_null() {
                1.0
            } else {
                let (points, pixels) = (CGDisplayModeGetWidth(mode), CGDisplayModeGetPixelWidth(mode));
                CGDisplayModeRelease(mode);
                if points == 0 { 1.0 } else { pixels as f64 / points as f64 }
            };
            Some(Display {
                frame: Frame { x: bounds.origin.x, y: bounds.origin.y, w: bounds.size.w, h: bounds.size.h },
                backing,
            })
        }
    }

    fn capture(&self, window: Option<u32>, width: usize, height: usize) -> Result<Vec<u8>, String> {
        let image = capture(window, width, height)?;
        let jpeg = jpeg(image);
        // SAFETY: retained by capture().
        unsafe { CGImageRelease(image) };
        jpeg
    }

    fn apps(&self) -> Vec<App> {
        let workspace = NSWorkspace::sharedWorkspace();
        workspace
            .runningApplications()
            .iter()
            .filter(|app| app.activationPolicy() == NSApplicationActivationPolicy::Regular)
            .map(|app| App {
                pid: app.processIdentifier(),
                name: app.localizedName().map(|n| n.to_string()).unwrap_or_default(),
                bundle_id: app.bundleIdentifier().map(|b| b.to_string()),
            })
            .collect()
    }

    fn open(&self, app: &str) -> Result<(), String> {
        let bundle_id = app.contains('.') && !app.contains(' ') && !app.ends_with(".app");
        let flag = if bundle_id { "-b" } else { "-a" };
        let out = std::process::Command::new("open").args(["-F", flag, app]).output().map_err(|e| e.to_string())?;
        if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()) }
    }

    fn windows(&self) -> Vec<Window> {
        let apps = self.apps();
        // On screen only, no desktop elements: front to back.
        // SAFETY: the window list, +1; dictionaries are toll-free bridged.
        let Some(list) = (unsafe { object(CGWindowListCopyWindowInfo(1 | 16, 0)) }) else { return Vec::new() };
        let list: Retained<NSArray<NSDictionary<NSString, AnyObject>>> = unsafe { Retained::cast_unchecked(list) };
        let mut windows = Vec::new();
        for info in list.iter() {
            let number = |key: &str| info.objectForKey(&ns(key)).and_then(|v| v.downcast::<NSNumber>().ok()).map(|n| n.doubleValue());
            let text = |key: &str| info.objectForKey(&ns(key)).and_then(|v| v.downcast::<NSString>().ok()).map(|s| s.to_string());
            if number("kCGWindowLayer") != Some(0.0) || number("kCGWindowAlpha") == Some(0.0) {
                continue;
            }
            let pid = number("kCGWindowOwnerPID").unwrap_or(0.0) as i32;
            // Only apps' windows: an overlay app (BetterDisplay's) isn't one.
            let Some(app) = apps.iter().find(|a| a.pid == pid) else { continue };
            let Some(bounds) = info.objectForKey(&ns("kCGWindowBounds")) else { continue };
            // SAFETY: kCGWindowBounds is a dictionary of numbers.
            let bounds: Retained<NSDictionary<NSString, NSNumber>> = unsafe { Retained::cast_unchecked(bounds) };
            let get = |k: &str| bounds.objectForKey(&ns(k)).map_or(0.0, |n| n.doubleValue());
            let frame = Frame { x: get("X"), y: get("Y"), w: get("Width"), h: get("Height") };
            if frame.w < 40.0 || frame.h < 40.0 {
                continue;
            }
            windows.push(Window {
                id: number("kCGWindowNumber").unwrap_or(0.0) as u32,
                pid,
                app: app.name.clone(),
                title: text("kCGWindowName").unwrap_or_default(),
                frame,
            });
        }
        windows
    }

    fn raise(&self, pid: i32, window: Option<&Window>) -> Result<(), String> {
        // A background process can't activate an app (cooperative activation);
        // Accessibility can.
        let app = application(pid);
        let front = set_true(app.0, "AXFrontmost");
        let raised = self.ax_window(pid, window).map_or(-1, |w| {
            let _ = set_true(w.0, "AXMain");
            perform(w.0, "AXRaise")
        });
        if front == 0 || raised == 0 { Ok(()) } else { Err(format!("Accessibility said no (AXFrontmost {front}, AXRaise {raised})")) }
    }

    fn focused_app(&self) -> Option<i32> {
        let app = attr(system().0, "AXFocusedApplication")?;
        pid_of(ptr(&app))
    }

    fn focused_element(&self) -> Option<Cf> {
        let element = attr(system().0, "AXFocusedUIElement")?;
        Cf::retained(ptr(&element))
    }

    fn hit(&self, at: Pt) -> Option<Hit> {
        let mut hit: Ref = std::ptr::null();
        // SAFETY: a system-wide hit test; the element comes back +1.
        if unsafe { AXUIElementCopyElementAtPosition(system().0, at.x as f32, at.y as f32, &mut hit) } != 0 {
            return None;
        }
        let hit = Cf::owned(hit)?;
        // What the click acts on: the icon inside a send button is the button.
        let mut acting = hit.clone();
        let mut at = hit.clone();
        for _ in 0..4 {
            if actions(at.0).iter().any(|a| a == "AXPress") {
                acting = at;
                break;
            }
            let Some(parent) = attr(at.0, "AXParent").and_then(|p| Cf::retained(ptr(&p))) else { break };
            at = parent;
        }
        Some(Hit { pid: pid_of(hit.0)?, role: string(acting.0, "AXRole").unwrap_or_default(), title: title_of(acting.0) })
    }

    fn elements(&self, pid: i32, window: Option<&Window>, max: usize) -> Vec<(Cf, Node)> {
        // Chromium and Electron apps (Teams) build their tree only for an assistive client.
        let _ = set_true(application(pid).0, "AXManualAccessibility");
        let mut out = Vec::new();
        if let Some(root) = self.ax_window(pid, window) {
            self.walk(&root, 0, max, &mut out);
        }
        out
    }

    fn same(&self, a: &Cf, b: &Cf) -> bool {
        // SAFETY: two live CF objects.
        unsafe { CFEqual(a.0, b.0) }
    }

    fn frame(&self, element: &Cf) -> Option<Frame> {
        frame_of(element.0)
    }

    fn node(&self, element: &Cf) -> Option<Node> {
        let e = element.0;
        let role = string(e, "AXRole")?;
        Some(Node { role, title: title_of(e), value: string(e, "AXValue"), focused: flag(e, "AXFocused"), ..Node::default() })
    }

    fn value(&self, element: &Cf) -> Option<String> {
        string(element.0, "AXValue")
    }

    fn press(&self, element: &Cf) -> Result<(), String> {
        match perform(element.0, "AXPress") {
            0 => Ok(()),
            err => Err(format!("AXPress failed ({err})")),
        }
    }

    fn focus(&self, element: &Cf) -> Result<(), String> {
        match set_true(element.0, "AXFocused") {
            0 => Ok(()),
            err => Err(format!("AXFocused failed ({err})")),
        }
    }

    fn pointer(&self, action: Pointer) {
        let point = |p: Pt| CGPoint { x: p.x, y: p.y };
        match action {
            Pointer::Click { at, right, count } => {
                let (down, up, button) = if right { (3, 4, 1) } else { (1, 2, 0) };
                mouse(5, point(at), 0, 0);
                sleep(40);
                for n in 1..=count.max(1) {
                    mouse(down, point(at), button, n as i64);
                    sleep(30);
                    mouse(up, point(at), button, n as i64);
                    sleep(40);
                }
            }
            Pointer::Drag { from, to } => {
                mouse(5, point(from), 0, 0);
                sleep(40);
                mouse(1, point(from), 0, 1);
                for step in 1..=12 {
                    let t = step as f64 / 12.0;
                    mouse(6, CGPoint { x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t }, 0, 0);
                    sleep(15);
                }
                mouse(2, point(to), 0, 1);
            }
            Pointer::Scroll { at, down, right } => {
                mouse(5, point(at), 0, 0);
                sleep(40);
                // Lines; positive wheel values scroll up and left.
                // SAFETY: a new scroll event, placed at the point.
                let event = unsafe { CGEventCreateScrollWheelEvent2(std::ptr::null(), 1, 2, -down, -right, 0) };
                if !event.is_null() {
                    unsafe { CGEventSetLocation(event, point(at)) };
                }
                post(event);
            }
        }
    }

    fn type_char(&self, c: char) {
        let mut units = [0u16; 2];
        let units = c.encode_utf16(&mut units);
        for down in [true, false] {
            // SAFETY: a new key event carrying the character as Unicode, so it
            // types the same whatever the input source (Korean included).
            unsafe {
                let event = CGEventCreateKeyboardEvent(std::ptr::null(), 0, down);
                if event.is_null() {
                    continue;
                }
                CGEventSetFlags(event, 0);
                CGEventKeyboardSetUnicodeString(event, units.len(), units.as_ptr());
                post(event);
            }
        }
        sleep(8);
    }

    fn key(&self, combo: Combo) {
        for down in [true, false] {
            // SAFETY: a new key event with its modifiers.
            unsafe {
                let event = CGEventCreateKeyboardEvent(std::ptr::null(), combo.code, down);
                if event.is_null() {
                    continue;
                }
                CGEventSetFlags(event, combo.flags);
                post(event);
            }
            sleep(20);
        }
    }

    fn pause(&self, ms: u64) {
        sleep(ms);
    }

    fn process_name(&self, pid: i32) -> Option<String> {
        if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            && let Some(name) = app.localizedName()
        {
            return Some(name.to_string());
        }
        // A system process: its executable's name (UserNotificationCenter, for a macOS dialog).
        let out = std::process::Command::new("ps").args(["-o", "comm=", "-p", &pid.to_string()]).output().ok()?;
        let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        path.rsplit('/').next().filter(|n| !n.is_empty()).map(str::to_owned)
    }
}

/// What ScreenCaptureKit may capture: refused without Screen Recording.
fn shareable() -> Result<Retained<SCShareableContent>, String> {
    let (tx, rx) = mpsc::channel::<Result<usize, String>>();
    let tx = Mutex::new(tx);
    let handler = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
        let result = if content.is_null() {
            // SAFETY: the error is valid when content is null.
            Err(unsafe { error.as_ref() }.map_or("no content".into(), |e| e.localizedDescription().to_string()))
        } else {
            // SAFETY: retained here, released by the receiver.
            Ok(unsafe { Retained::retain(content) }.map_or(0, |c| Retained::into_raw(c) as usize))
        };
        let _ = tx.lock().map(|tx| tx.send(result));
    });
    // SAFETY: the handler lives until it has been called.
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };
    let content = rx.recv_timeout(Duration::from_secs(10)).map_err(|_| "ScreenCaptureKit didn't answer".to_owned())??;
    // SAFETY: the +1 reference taken in the handler.
    unsafe { Retained::from_raw(content as *mut SCShareableContent) }.ok_or_else(|| "no content".to_owned())
}

/// One capture with ScreenCaptureKit, sized by the capture itself. The image
/// comes back retained.
fn capture(window: Option<u32>, width: usize, height: usize) -> Result<Ref, String> {
    let content = shareable()?;
    let filter = match window {
        Some(id) => {
            let windows = unsafe { content.windows() };
            let found: Option<Retained<SCWindow>> = windows.iter().find(|w| unsafe { w.windowID() } == id);
            let found = found.ok_or_else(|| format!("window {id} isn't on screen any more"))?;
            unsafe { SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &found) }
        }
        None => {
            let main = unsafe { CGMainDisplayID() };
            let displays = unsafe { content.displays() };
            let display = displays.iter().find(|d| unsafe { d.displayID() } == main).or_else(|| displays.firstObject()).ok_or("no display")?;
            unsafe { SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &display, &NSArray::<SCWindow>::new()) }
        }
    };
    let config = unsafe { SCStreamConfiguration::new() };
    unsafe {
        config.setWidth(width);
        config.setHeight(height);
        config.setShowsCursor(true);
        config.setScalesToFit(true);
    }
    let (tx, rx) = mpsc::channel::<Result<usize, String>>();
    let tx = Mutex::new(tx);
    let handler = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
        let result = if image.is_null() {
            // SAFETY: the error is valid when the image is null.
            Err(unsafe { error.as_ref() }.map_or("no image".into(), |e| e.localizedDescription().to_string()))
        } else {
            // SAFETY: kept past the handler; released after encoding.
            Ok(unsafe { CGImageRetain(image as Ref) } as usize)
        };
        let _ = tx.lock().map(|tx| tx.send(result));
    });
    // SAFETY: filter and config outlive the call; the handler lives until called.
    unsafe { SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(&filter, &config, Some(&handler)) };
    rx.recv_timeout(Duration::from_secs(10)).map_err(|_| "the capture timed out".to_owned())?.map(|p| p as Ref)
}

/// The image as JPEG, in memory only.
fn jpeg(image: Ref) -> Result<Vec<u8>, String> {
    // SAFETY: ImageIO writing into a CFMutableData we own.
    unsafe {
        let data = Cf::owned(CFDataCreateMutable(std::ptr::null(), 0)).ok_or("out of memory")?;
        let kind = ns("public.jpeg");
        let destination = Cf::owned(CGImageDestinationCreateWithData(data.0, ptr(&kind), 1, std::ptr::null())).ok_or("no JPEG encoder")?;
        let key: &NSString = &*(kCGImageDestinationLossyCompressionQuality as *const NSString);
        let options = NSDictionary::from_slices(&[key], &[&*NSNumber::new_f64(0.75)]);
        CGImageDestinationAddImage(destination.0, image, ptr(&options));
        if !CGImageDestinationFinalize(destination.0) {
            return Err("couldn't encode the image".into());
        }
        let bytes = std::slice::from_raw_parts(CFDataGetBytePtr(data.0), CFDataGetLength(data.0) as usize);
        Ok(bytes.to_vec())
    }
}

extern "C" fn tick(_: Ref, _: *mut c_void) {}

/// Run the helper: requests served on a thread of their own, while the main
/// thread runs the run loop that keeps AppKit's list of running apps current.
pub fn serve(home: &Path) -> std::io::Result<()> {
    let home = home.to_owned();
    std::thread::spawn(move || {
        let mut controller = Controller::new(Mac::new());
        if let Err(err) = crate::helper::serve(&home, &mut controller, Mac::status, Mac::request) {
            eprintln!("equipe desktop helper: {err}");
            std::process::exit(1);
        }
    });
    // SAFETY: a timer on the main run loop, so it has a source and keeps running.
    unsafe {
        let timer = CFRunLoopTimerCreate(std::ptr::null(), CFAbsoluteTimeGetCurrent() + 3600.0, 3600.0, 0, 0, tick, std::ptr::null());
        CFRunLoopAddTimer(CFRunLoopGetMain(), timer, kCFRunLoopCommonModes);
        CFRunLoopRun();
    }
    Ok(())
}
