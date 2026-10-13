//! T-20 spike — not product code. Run under a throwaway LaunchAgent to learn
//! whether a launchd login service can see the screen, click, type and open
//! apps, and what the TCC permissions attach to.
//!
//! desktop-probe status        session + permissions, prompts nothing
//! desktop-probe request       asks macOS for the permissions (adds us to the lists)
//! desktop-probe child         re-runs `status` as a child process (inheritance)
//! desktop-probe run <dir>     capture → open TextEdit → click → type → read back (AX) → capture
//! desktop-probe capture <dir> one capture, ScreenCaptureKit then CGDisplayCreateImage

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, ClassType};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber, NSString};
use objc2_screen_capture_kit::{SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow};

type Ref = *const c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Point {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Size {
    w: f64,
    h: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Rect {
    origin: Point,
    size: Size,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> Rect;
    fn CGDisplayPixelsWide(display: u32) -> usize;
    fn CGDisplayPixelsHigh(display: u32) -> usize;
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayIsAsleep(display: u32) -> u32;
    fn CGDisplayCreateImage(display: u32) -> Ref;
    fn CGImageGetWidth(image: Ref) -> usize;
    fn CGImageGetHeight(image: Ref) -> usize;
    fn CGImageGetBytesPerRow(image: Ref) -> usize;
    fn CGImageGetBitmapInfo(image: Ref) -> u32;
    fn CGImageGetDataProvider(image: Ref) -> Ref;
    fn CGDataProviderCopyData(provider: Ref) -> Ref;
    fn CGImageRetain(image: Ref) -> Ref;
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
    fn CGSessionCopyCurrentDictionary() -> Ref;
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> Ref;
    fn CGEventCreate(source: Ref) -> Ref;
    fn CGEventGetLocation(event: Ref) -> Point;
    fn CGEventCreateMouseEvent(source: Ref, kind: u32, at: Point, button: u32) -> Ref;
    fn CGEventCreateKeyboardEvent(source: Ref, key: u16, down: bool) -> Ref;
    fn CGEventKeyboardSetUnicodeString(event: Ref, len: usize, text: *const u16);
    fn CGEventPost(tap: u32, event: Ref);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: Ref) -> *const u8;
    fn CFDataGetLength(data: Ref) -> isize;
    fn CFRelease(any: Ref);
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: Ref) -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> Ref;
    fn AXUIElementCopyAttributeValue(element: Ref, attribute: Ref, value: *mut Ref) -> i32;
}

/// Longest edge of the image a model would get.
const MODEL_EDGE: usize = 1568;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("status");
    let dir = args.get(2).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("desktop-probe"));
    println!("==== desktop-probe {mode} pid={} ppid={} at {:?}", std::process::id(), parent(), now());
    match mode {
        "status" => status(),
        "request" => request(),
        "child" => {
            status();
            let out = Command::new(std::env::current_exe().unwrap()).arg("status").output().unwrap();
            println!("---- child says:\n{}", String::from_utf8_lossy(&out.stdout));
        }
        "capture" => {
            status();
            std::fs::create_dir_all(&dir).unwrap();
            capture_both(&dir, "capture");
        }
        "run" => {
            status();
            std::fs::create_dir_all(&dir).unwrap();
            run(&dir);
        }
        other => println!("unknown mode {other}"),
    }
    println!("==== done");
}

fn parent() -> i32 {
    // SAFETY: getppid has no preconditions.
    unsafe { libc_getppid() }
}
unsafe extern "C" {
    #[link_name = "getppid"]
    fn libc_getppid() -> i32;
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn sh(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned() + &String::from_utf8_lossy(&o.stderr).trim().to_owned())
        .unwrap_or_else(|e| format!("({e})"))
}

fn ns(text: &str) -> Retained<NSString> {
    NSString::from_str(text)
}

/// A +1 CF object as an Objective-C one (toll-free bridged).
unsafe fn owned(any: Ref) -> Option<Retained<AnyObject>> {
    unsafe { Retained::from_raw(any as *mut AnyObject) }
}

fn status() {
    println!("exe            {:?}", std::env::current_exe().ok());
    println!("launchd session {}", sh("launchctl", &["managername"]));
    println!("codesign       {}", sh("codesign", &["-dv", &std::env::current_exe().unwrap().to_string_lossy()]).lines().filter(|l| l.starts_with("Identifier") || l.starts_with("CodeDirectory") || l.starts_with("TeamIdentifier")).collect::<Vec<_>>().join(" | "));
    // SAFETY: plain CoreGraphics / HIServices queries without arguments.
    unsafe {
        println!("screen capture {}", CGPreflightScreenCaptureAccess());
        println!("post events    {}", CGPreflightPostEventAccess());
        println!("accessibility  {}", AXIsProcessTrusted());
        let session = CGSessionCopyCurrentDictionary();
        match owned(session) {
            Some(dict) => {
                let dict: Retained<NSDictionary<NSString, AnyObject>> = Retained::cast_unchecked(dict);
                for key in ["kCGSSessionOnConsoleKey", "CGSSessionScreenIsLocked", "kCGSessionLoginDoneKey", "kCGSSessionUserNameKey"] {
                    println!("session {key:<26} {:?}", dict.objectForKey(&ns(key)).map(|v| describe(&v)));
                }
            }
            None => println!("session        (none — no window server session)"),
        }
        let mut ids = [0u32; 8];
        let mut count = 0;
        CGGetActiveDisplayList(8, ids.as_mut_ptr(), &mut count);
        for &id in &ids[..count as usize] {
            let b = CGDisplayBounds(id);
            println!(
                "display {id}: {}x{} pt, {}x{} px, asleep={} main={}",
                b.size.w,
                b.size.h,
                CGDisplayPixelsWide(id),
                CGDisplayPixelsHigh(id),
                CGDisplayIsAsleep(id),
                id == CGMainDisplayID()
            );
        }
        if count == 0 {
            println!("display        none active");
        }
        let (named, total) = window_names();
        println!("windows        {total} on screen, {named} with a title readable (titles need screen recording)");
    }
}

fn describe(value: &AnyObject) -> String {
    if let Some(s) = value.downcast_ref::<NSString>() {
        return s.to_string();
    }
    if let Some(n) = value.downcast_ref::<NSNumber>() {
        return n.stringValue().to_string();
    }
    format!("{value:?}")
}

/// On-screen windows of other apps, and how many expose their title.
unsafe fn window_names() -> (usize, usize) {
    unsafe {
        let Some(list) = owned(CGWindowListCopyWindowInfo(1, 0)) else { return (0, 0) };
        let list: Retained<NSArray<NSDictionary<NSString, AnyObject>>> = Retained::cast_unchecked(list);
        let me = std::process::id() as i32;
        let mut named = 0;
        let mut total = 0;
        for window in list.iter() {
            let pid = window.objectForKey(&ns("kCGWindowOwnerPID")).and_then(|v| v.downcast::<NSNumber>().ok()).map_or(0, |n| n.intValue());
            if pid == me {
                continue;
            }
            total += 1;
            if window.objectForKey(&ns("kCGWindowName")).is_some() {
                named += 1;
            }
        }
        (named, total)
    }
}

fn request() {
    // SAFETY: these show the system's permission prompts (or add the entry to
    // System Settings' list) and return the current state.
    unsafe {
        println!("request screen capture -> {}", CGRequestScreenCaptureAccess());
        println!("request post events    -> {}", CGRequestPostEventAccess());
        let options = NSDictionary::from_slices(&[&*ns("AXTrustedCheckOptionPrompt")], &[&*NSNumber::new_bool(true)]);
        println!("request accessibility  -> {}", AXIsProcessTrustedWithOptions(Retained::as_ptr(&options) as Ref));
    }
}

/// Screen size in points, and the capture size for the model.
fn target_size() -> (Rect, usize, usize) {
    // SAFETY: plain display queries.
    let bounds = unsafe { CGDisplayBounds(CGMainDisplayID()) };
    let (pw, ph) = unsafe { (CGDisplayPixelsWide(CGMainDisplayID()), CGDisplayPixelsHigh(CGMainDisplayID())) };
    let scale = (MODEL_EDGE as f64 / pw.max(ph) as f64).min(1.0);
    (bounds, ((pw as f64) * scale) as usize, ((ph as f64) * scale) as usize)
}

/// ScreenCaptureKit, sized for the model by the capture itself.
fn capture_sck(width: usize, height: usize) -> Result<Ref, String> {
    let (tx, rx) = mpsc::channel::<Result<usize, String>>();
    let tx = Mutex::new(tx);
    let handler = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
        let result = if content.is_null() {
            // SAFETY: the error is valid when content is null.
            Err(unsafe { error.as_ref() }.map_or("no content, no error".into(), |e| e.localizedDescription().to_string()))
        } else {
            // SAFETY: retained by us before the block returns.
            let _ = unsafe { Retained::retain(content) }.map(Retained::into_raw);
            Ok(content as usize)
        };
        let _ = tx.lock().unwrap().send(result);
    });
    // SAFETY: the completion handler lives until it has been called.
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };
    let content = rx.recv_timeout(Duration::from_secs(10)).map_err(|_| "getShareableContent: timed out".to_owned())??;
    // SAFETY: retained above; never released (probe).
    let content: &SCShareableContent = unsafe { &*(content as *const SCShareableContent) };
    let displays = unsafe { content.displays() };
    println!("sck            {} display(s), {} window(s)", displays.count(), unsafe { content.windows() }.count());
    let main = unsafe { CGMainDisplayID() };
    let display = displays.iter().find(|d| unsafe { d.displayID() } == main).or_else(|| displays.firstObject()).ok_or("sck: no display")?;
    let filter = unsafe { SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &display, &NSArray::<SCWindow>::new()) };
    let config = unsafe { SCStreamConfiguration::new() };
    unsafe {
        config.setWidth(width);
        config.setHeight(height);
        config.setShowsCursor(true);
    }
    let (tx, rx) = mpsc::channel::<Result<usize, String>>();
    let tx = Mutex::new(tx);
    let handler = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
        let result = if image.is_null() {
            // SAFETY: the error is valid when the image is null.
            Err(unsafe { error.as_ref() }.map_or("no image, no error".into(), |e| e.localizedDescription().to_string()))
        } else {
            // SAFETY: kept past the handler; released by nobody (probe).
            Ok(unsafe { CGImageRetain(image as Ref) } as usize)
        };
        let _ = tx.lock().unwrap().send(result);
    });
    unsafe { SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(&filter, &config, Some(&handler)) };
    let _ = SCScreenshotManager::class();
    rx.recv_timeout(Duration::from_secs(10)).map_err(|_| "captureImage: timed out".to_owned())?.map(|p| p as Ref)
}

/// Writes the image as PNG (BGRA → RGBA) and returns a crude "is there
/// anything but one colour" figure.
fn save(image: Ref, path: &Path) -> String {
    // SAFETY: image is a valid CGImage.
    unsafe {
        let (w, h, row) = (CGImageGetWidth(image), CGImageGetHeight(image), CGImageGetBytesPerRow(image));
        let data = CGDataProviderCopyData(CGImageGetDataProvider(image));
        let bytes = std::slice::from_raw_parts(CFDataGetBytePtr(data), CFDataGetLength(data) as usize);
        let mut rgba = Vec::with_capacity(w * h * 4);
        let mut colours = std::collections::HashSet::new();
        for y in 0..h {
            for x in 0..w {
                let p = &bytes[y * row + x * 4..y * row + x * 4 + 4];
                rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
                if x % 7 == 0 && y % 7 == 0 {
                    colours.insert([p[0] >> 4, p[1] >> 4, p[2] >> 4]);
                }
            }
        }
        CFRelease(data);
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&rgba).unwrap();
        format!("{w}x{h} bitmapinfo={:#x} colours~{} -> {}", CGImageGetBitmapInfo(image), colours.len(), path.display())
    }
}

fn capture_both(dir: &Path, tag: &str) {
    let (bounds, w, h) = target_size();
    let started = Instant::now();
    match capture_sck(w, h) {
        Ok(image) => println!("capture sck    ok in {:?}: {}", started.elapsed(), save(image, &dir.join(format!("{tag}-sck.png")))),
        Err(e) => println!("capture sck    FAILED: {e}"),
    }
    let started = Instant::now();
    // SAFETY: plain capture of the main display.
    let image = unsafe { CGDisplayCreateImage(CGMainDisplayID()) };
    if image.is_null() {
        println!("capture cg     FAILED (null)");
    } else {
        println!("capture cg     ok in {:?}: {}", started.elapsed(), save(image, &dir.join(format!("{tag}-cg.png"))));
    }
    println!("coords         image {w}x{h} px ↔ screen {}x{} pt: x_pt = x_img * {:.4}", bounds.size.w, bounds.size.h, bounds.size.w / w as f64);
}

fn mouse() -> Point {
    // SAFETY: an empty event carries the current pointer location.
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        let at = CGEventGetLocation(event);
        CFRelease(event);
        at
    }
}

fn click(at: Point) {
    // SAFETY: synthesised events posted at the HID tap.
    unsafe {
        for kind in [5u32, 1, 2] {
            let event = CGEventCreateMouseEvent(std::ptr::null(), kind, at, 0);
            CGEventPost(0, event);
            CFRelease(event);
            std::thread::sleep(Duration::from_millis(60));
        }
    }
}

fn type_text(text: &str) {
    // SAFETY: synthesised key events carrying Unicode, one character each.
    unsafe {
        for unit in text.encode_utf16() {
            for down in [true, false] {
                let event = CGEventCreateKeyboardEvent(std::ptr::null(), 0, down);
                CGEventKeyboardSetUnicodeString(event, 1, &unit);
                CGEventPost(0, event);
                CFRelease(event);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }
}

/// TextEdit's frontmost document window, in screen points.
fn window_of(pid: i32) -> Option<Rect> {
    // SAFETY: window list query; dictionaries are toll-free bridged.
    unsafe {
        let list = owned(CGWindowListCopyWindowInfo(1, 0))?;
        let list: Retained<NSArray<NSDictionary<NSString, AnyObject>>> = Retained::cast_unchecked(list);
        for window in list.iter() {
            let number = |key: &str| window.objectForKey(&ns(key)).and_then(|v| v.downcast::<NSNumber>().ok()).map(|n| n.doubleValue());
            if number("kCGWindowOwnerPID") != Some(pid as f64) || number("kCGWindowLayer") != Some(0.0) {
                continue;
            }
            let bounds: Retained<NSDictionary<NSString, NSNumber>> = Retained::cast_unchecked(window.objectForKey(&ns("kCGWindowBounds"))?);
            let get = |k: &str| bounds.objectForKey(&ns(k)).map_or(0.0, |n| n.doubleValue());
            return Some(Rect { origin: Point { x: get("X"), y: get("Y") }, size: Size { w: get("Width"), h: get("Height") } });
        }
        None
    }
}

fn attr(element: Ref, name: &str) -> Option<Retained<AnyObject>> {
    let mut value: Ref = std::ptr::null();
    let name = ns(name);
    // SAFETY: element is an AXUIElement; the value comes back +1.
    let err = unsafe { AXUIElementCopyAttributeValue(element, Retained::as_ptr(&name) as Ref, &mut value) };
    if err != 0 {
        return None;
    }
    unsafe { owned(value) }
}

fn text(element: Ref, name: &str) -> Option<String> {
    attr(element, name)?.downcast::<NSString>().ok().map(|s| s.to_string())
}

/// Walks the accessibility tree: prints roles and returns the first text area's value.
fn walk(element: Ref, depth: usize, printed: &mut usize, found: &mut Option<String>) {
    let role = text(element, "AXRole").unwrap_or_default();
    let title = text(element, "AXTitle").or_else(|| text(element, "AXDescription")).unwrap_or_default();
    if *printed < 45 {
        println!("ax {}{role} {title:?}", "  ".repeat(depth));
        *printed += 1;
    }
    if role == "AXTextArea" && found.is_none() {
        *found = text(element, "AXValue");
    }
    if depth > 8 {
        return;
    }
    if let Some(children) = attr(element, "AXChildren") {
        // SAFETY: AXChildren is a CFArray of AXUIElements.
        let children: Retained<NSArray<AnyObject>> = unsafe { Retained::cast_unchecked(children) };
        for child in children.iter() {
            walk(Retained::as_ptr(&child) as Ref, depth + 1, printed, found);
        }
    }
}

fn run(dir: &Path) {
    capture_both(dir, "before");

    let doc = dir.join("probe.txt");
    std::fs::write(&doc, "").unwrap();
    let was_running = !sh("pgrep", &["-x", "TextEdit"]).is_empty();
    println!("textedit       running before: {was_running}");
    println!("open -a        {:?}", sh("open", &["-a", "TextEdit", &doc.to_string_lossy()]));
    let mut pid = 0;
    let mut window = None;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(200));
        pid = sh("pgrep", &["-nx", "TextEdit"]).parse().unwrap_or(0);
        if pid != 0 {
            window = window_of(pid);
            if window.is_some() {
                break;
            }
        }
    }
    println!("textedit       pid={pid} window={window:?}");
    let Some(window) = window else {
        println!("textedit       no window seen — stop");
        return;
    };
    std::thread::sleep(Duration::from_millis(800));

    let at = Point { x: window.origin.x + window.size.w / 2.0, y: window.origin.y + window.size.h / 2.0 };
    let before = mouse();
    click(at);
    let after = mouse();
    println!("click          at {at:?}; pointer {before:?} -> {after:?}");
    std::thread::sleep(Duration::from_millis(300));

    let typed = "hello from equipe probe 안녕하세요";
    type_text(typed);
    println!("type           {typed:?}");
    std::thread::sleep(Duration::from_millis(500));

    // SAFETY: the app element for TextEdit's pid.
    let app = unsafe { AXUIElementCreateApplication(pid) };
    let mut printed = 0;
    let mut found = None;
    walk(app, 0, &mut printed, &mut found);
    println!("ax text area   {found:?}");
    println!("RESULT typed text landed: {}", found.as_deref().is_some_and(|t| t.contains(typed)));

    capture_both(dir, "after");

    if !was_running && pid != 0 {
        println!("textedit       quit (we started it): {}", sh("kill", &[&pid.to_string()]));
    }
}
