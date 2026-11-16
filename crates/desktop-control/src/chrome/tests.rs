//! The real-Chrome tools against a stand-in agent-desktop and a stand-in
//! Mac: envelopes in agent-desktop 1.2.0's documented shape and a Chrome
//! profile list, all hand-written. Nothing here opens, reads or touches the
//! person's Chrome or screen.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::engine::Runner;

type Calls = Arc<Mutex<Vec<(Vec<String>, Option<String>)>>>;
/// Chrome's windows as the stand-ins see them: id, title.
type Windows = Arc<Mutex<Vec<(String, String)>>>;

/// agent-desktop, answering from `windows`.
struct FakeEngine {
    calls: Calls,
    windows: Windows,
}

/// A page with a field, a password field, a human check, a button and a link.
fn page(window: &str, title: &str) -> Value {
    json!({ "ok": true, "command": "snapshot", "data": {
        "app": "Google Chrome",
        "window": { "id": window, "title": title },
        "snapshot_id": format!("s{}", window.trim_start_matches("w-")),
        "ref_count": 5,
        "complete": true,
        "tree": { "role": "window", "name": title, "children": [ { "role": "webarea", "name": title, "children": [
            { "role": "heading", "name": "Sign in to Example" },
            { "ref_id": format!("@s{}:e1", window.trim_start_matches("w-")), "role": "textfield", "name": "Email", "states": ["enabled"] },
            { "ref_id": format!("@s{}:e2", window.trim_start_matches("w-")), "role": "textfield", "name": "Password", "value": "hunter2", "states": ["enabled", "secure"] },
            { "ref_id": format!("@s{}:e3", window.trim_start_matches("w-")), "role": "checkbox", "name": "I'm not a robot", "states": ["enabled"] },
            { "ref_id": format!("@s{}:e4", window.trim_start_matches("w-")), "role": "button", "name": "Send", "states": ["enabled"] },
            { "ref_id": format!("@s{}:e5", window.trim_start_matches("w-")), "role": "link", "name": "Forgot password?", "states": ["enabled"] }
        ] } ] }
    }})
}

impl Runner for FakeEngine {
    fn run(&self, args: &[String], stdin: Option<&str>, _: Duration) -> Result<Value, String> {
        self.calls.lock().unwrap().push((args.to_vec(), stdin.map(str::to_owned)));
        let command = args.iter().find(|a| !a.starts_with("--")).cloned().unwrap_or_default();
        let windows = self.windows.lock().unwrap().clone();
        Ok(match command.as_str() {
            "list-windows" => json!({ "ok": true, "command": "list-windows", "data": windows.iter().map(|(id, title)| json!({ "id": id, "title": title, "app_name": "Google Chrome" })).collect::<Vec<_>>() }),
            "list-apps" => json!({ "ok": true, "command": "list-apps", "data": { "apps": [ { "name": "Google Chrome", "bundle_id": "com.google.Chrome" } ] } }),
            "snapshot" => {
                let asked = args.iter().find_map(|a| a.strip_prefix("--window-id=")).map(str::to_owned);
                let (id, title) = match asked {
                    Some(id) => windows.iter().find(|(w, _)| *w == id).cloned().unwrap_or((id, String::new())),
                    None => windows.last().cloned().unwrap_or_default(),
                };
                page(&id, &title)
            }
            other => json!({ "ok": true, "command": other, "data": { "action": other, "disposition": { "delivery": "delivered_verified", "retry": "unsafe" } } }),
        })
    }
}

/// The Mac: a profile list, and Chrome windows that open and close.
struct FakeMac {
    windows: Windows,
    opened: Arc<Mutex<Vec<(String, String)>>>,
    closed: Arc<Mutex<Vec<u32>>>,
    next: AtomicU32,
}

impl ChromeOs for FakeMac {
    fn local_state(&self) -> Result<String, String> {
        Ok(profiles::tests::LOCAL_STATE.to_owned())
    }
    fn open(&self, dir: &str, url: &str) -> Result<(), String> {
        self.opened.lock().unwrap().push((dir.to_owned(), url.to_owned()));
        let n = self.next.fetch_add(1, Ordering::SeqCst);
        self.windows.lock().unwrap().push((format!("w-{n}"), "Sign in · Example".to_owned()));
        Ok(())
    }
    fn close(&self, window: u32) -> Result<(), String> {
        self.closed.lock().unwrap().push(window);
        self.windows.lock().unwrap().retain(|(id, _)| *id != format!("w-{window}"));
        Ok(())
    }
    fn pause(&self, _: u64) {}
}

struct World {
    chrome: Chrome,
    home: PathBuf,
    calls: Calls,
    windows: Windows,
    opened: Arc<Mutex<Vec<(String, String)>>>,
    closed: Arc<Mutex<Vec<u32>>>,
}

fn world(name: &str, on: bool) -> World {
    let home = std::env::temp_dir().join(format!("eq-chrome-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    if on {
        set_enabled(&home, "acme", true).unwrap();
    }
    let calls: Calls = Default::default();
    let windows: Windows = Arc::new(Mutex::new(vec![("w-1".into(), "The person's own tab".into())]));
    let opened: Arc<Mutex<Vec<(String, String)>>> = Default::default();
    let closed: Arc<Mutex<Vec<u32>>> = Default::default();
    let engine = Arc::new(Engine::new(Box::new(FakeEngine { calls: calls.clone(), windows: windows.clone() })));
    let mac = FakeMac { windows: windows.clone(), opened: opened.clone(), closed: closed.clone(), next: AtomicU32::new(900) };
    let chrome = Chrome::new(&home, engine, Box::new(mac));
    World { chrome, home, calls, windows, opened, closed }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

const A: &str = "acme/frontend";
const B: &str = "acme/backend";

fn ok(answer: Answer) -> String {
    assert!(!answer.is_error, "{}", answer.text);
    answer.text
}

fn refused(answer: Answer) -> String {
    assert!(answer.is_error, "expected a refusal, got: {}", answer.text);
    answer.text
}

#[test]
fn it_is_off_until_the_person_turns_it_on_for_the_squad() {
    let w = world("off", false);
    let text = refused(w.chrome.call(A, "chrome_open", &json!({ "url": "https://example.com", "profile": "Work" })));
    assert!(text.contains("equipe desktop chrome on"), "{text}");
    assert!(w.opened.lock().unwrap().is_empty(), "nothing opened");
    assert!(!enabled(&w.home, "acme"));
    set_enabled(&w.home, "acme", true).unwrap();
    set_enabled(&w.home, "other", true).unwrap();
    assert!(enabled(&w.home, "acme") && enabled(&w.home, "other"));
    set_enabled(&w.home, "other", false).unwrap();
    assert!(enabled(&w.home, "acme") && !enabled(&w.home, "other"), "per squad");
    assert!(w.chrome.guard_generic(A, "snapshot", &json!({ "app": "Google Chrome" })).is_some());
}

#[test]
fn profiles_show_masked_accounts_and_the_squads_choices() {
    let w = world("profiles", true);
    let text = ok(w.chrome.call(A, "chrome_profiles", &json!({})));
    assert!(text.contains("Profile 2 · \"Work\" · j***@acme.example"), "{text}");
    assert!(!text.contains("jordan.lee@") && !text.contains("jordan@example"), "never a full address: {text}");
    assert!(text.contains("No profile chosen"), "{text}");
    profiles::remember(&w.home, "acme", "github.com", "Profile 2").unwrap();
    let text = ok(w.chrome.call(A, "chrome_profiles", &json!({})));
    assert!(text.contains("github.com → Profile 2 (\"Work\")"), "{text}");
}

#[test]
fn opening_asks_which_profile_then_keeps_the_choice_and_the_window_is_the_members() {
    let w = world("open", true);
    let ask = refused(w.chrome.call(A, "chrome_open", &json!({ "url": "github.com/login" })));
    assert!(ask.contains("Which of the person's Chrome profiles is for github.com") && ask.contains("ask_decision"), "{ask}");
    assert!(w.opened.lock().unwrap().is_empty());

    let text = ok(w.chrome.call(A, "chrome_open", &json!({ "url": "github.com/login", "profile": "Work", "remember": true })));
    assert!(text.starts_with("opened w-900 in the person's Chrome — profile \"Work\" (Profile 2, j***@acme.example) at github.com"), "{text}");
    assert!(text.contains("This squad now uses \"Work\" for github.com"), "{text}");
    assert_eq!(*w.opened.lock().unwrap(), [("Profile 2".to_owned(), "https://github.com/login".to_owned())]);

    // The squad's choice: no profile needed next time, for any page of the site.
    let text = ok(w.chrome.call(B, "chrome_open", &json!({ "url": "https://gist.github.com/x" })));
    assert!(text.starts_with("opened w-901") && text.contains("(Profile 2"), "{text}");

    for bad in ["chrome://settings/passwords", "file:///etc/hosts", "javascript:alert(1)"] {
        let text = refused(w.chrome.call(A, "chrome_open", &json!({ "url": bad, "profile": "Work" })));
        assert!(text.contains("Only web pages"), "{text}");
    }
    assert_eq!(w.opened.lock().unwrap().len(), 2);
}

#[test]
fn a_member_reads_and_acts_only_in_its_own_windows() {
    let w = world("own", true);
    ok(w.chrome.call(A, "chrome_open", &json!({ "url": "https://example.com", "profile": "Default" })));
    let snap = ok(w.chrome.call(A, "chrome_snapshot", &json!({})));
    let args = w.calls.lock().unwrap().last().unwrap().0.clone();
    assert!(args.contains(&"--app=Google Chrome".to_owned()) && args.contains(&"--window-id=w-900".to_owned()), "{args:?}");
    assert!(args.contains(&"--timeout-ms=8000".to_owned()), "Chrome's web tree gets time to switch on: {args:?}");
    assert!(snap.contains("e4 button \"Send\""), "{snap}");
    assert!(snap.contains("human check") && snap.contains("show_my_browser"), "a human check on the page is called out: {snap}");

    // Not the person's window, not another member's.
    assert!(refused(w.chrome.call(A, "chrome_snapshot", &json!({ "window": "w-1" }))).contains("isn't one of your Chrome windows"));
    assert!(refused(w.chrome.call(B, "chrome_snapshot", &json!({ "window": "w-900" }))).contains("isn't one of your Chrome windows"));
    assert!(refused(w.chrome.call(B, "chrome_close", &json!({ "window": "w900" }))).contains("isn't one of your Chrome windows"));
    assert!(refused(w.chrome.call(B, "chrome_snapshot", &json!({}))).contains("chrome_open"));
    assert!(w.chrome.capture_target(B, &json!({ "window": "w-900" })).is_err());
    assert_eq!(w.chrome.capture_target(A, &json!({})), Ok(900));

    // Acting: by ref, headless.
    ok(w.chrome.call(A, "chrome_act", &json!({ "ref": "e1", "action": "type", "text": "me@example.com" })));
    let (args, stdin) = w.calls.lock().unwrap().last().unwrap().clone();
    assert!(args.contains(&"type".to_owned()) && args.contains(&"--stdin".to_owned()) && args.contains(&"@s900:e1".to_owned()), "{args:?}");
    assert!(!args.iter().any(|a| a.contains("me@example.com")), "typed text never in arguments");
    assert_eq!(stdin.as_deref(), Some("me@example.com"));
    ok(w.chrome.call(A, "chrome_act", &json!({ "ref": "e5" })));
    assert!(w.calls.lock().unwrap().last().unwrap().0.contains(&"click".to_owned()));
    let text = refused(w.chrome.call(A, "chrome_act", &json!({ "ref": "e4", "action": "double_click" })));
    assert!(text.contains("No action \"double_click\""), "{text}");
    let headed = w.chrome.call(A, "chrome_act", &json!({ "ref": "e5", "headed": true }));
    assert!(!headed.is_error && !w.calls.lock().unwrap().last().unwrap().0.contains(&"--headed".to_owned()), "never physical input here");
    // Another member never has this member's refs.
    assert!(refused(w.chrome.call(B, "chrome_act", &json!({ "ref": "s900:e5" }))).contains("isn't in any snapshot"));
}

#[test]
fn a_human_check_is_never_a_members_to_act_on() {
    let w = world("captcha", true);
    ok(w.chrome.call(A, "chrome_open", &json!({ "url": "https://example.com", "profile": "Default" })));
    ok(w.chrome.call(A, "chrome_snapshot", &json!({})));
    let before = w.calls.lock().unwrap().len();
    let text = refused(w.chrome.call(A, "chrome_act", &json!({ "ref": "e3", "action": "check" })));
    assert!(text.contains("It is the person's"), "{text}");
    assert!(w.chrome.scene(A, &json!({ "ref": "e3" })).is_err(), "nothing for the gate to ask either");
    assert_eq!(w.calls.lock().unwrap().len(), before, "agent-desktop never ran");

    // A challenge page in front of the site: nothing on it takes input.
    *w.windows.lock().unwrap() = vec![("w-900".into(), "Just a moment...".into())];
    ok(w.chrome.call(A, "chrome_snapshot", &json!({})));
    assert!(refused(w.chrome.call(A, "chrome_act", &json!({ "ref": "e5" }))).contains("It is the person's"));
}

#[test]
fn the_send_gate_sees_a_web_messaging_page_as_its_app() {
    let w = world("scene", true);
    ok(w.chrome.call(A, "chrome_open", &json!({ "url": "https://mail.google.com", "profile": "Default" })));
    *w.windows.lock().unwrap() = vec![("w-900".into(), "Inbox (2) - someone@example.com - Gmail".into())];
    ok(w.chrome.call(A, "chrome_snapshot", &json!({})));
    let scene = w.chrome.scene(A, &json!({ "ref": "e4" })).unwrap();
    assert_eq!(scene["app"], json!({ "name": "Mail" }), "{scene}");
    assert_eq!(scene["target"]["role"], "AXButton");
    assert_eq!(scene["target"]["title"], "Send");

    // The input goes ahead with the scene the gate judged; the engine re-checks with its own app name.
    let mut args = json!({ "ref": "e4" });
    args[EXPECT] = scene.clone();
    ok(w.chrome.call(A, "chrome_act", &args));
    // Another page by then: nothing done.
    let mut stale = scene;
    stale["app"] = json!({ "name": "Slack" });
    let mut args = json!({ "ref": "e4" });
    args[EXPECT] = stale;
    assert!(refused(w.chrome.call(A, "chrome_act", &args)).contains("changed"));

    // A page that isn't messaging stays Chrome.
    *w.windows.lock().unwrap() = vec![("w-900".into(), "Docs · Example".into())];
    ok(w.chrome.call(A, "chrome_snapshot", &json!({})));
    let scene = w.chrome.scene(A, &json!({ "ref": "e4" })).unwrap();
    assert_eq!(scene["app"]["name"], "Google Chrome");
    assert_eq!(scene["app"]["bundle_id"], "com.google.Chrome");
}

#[test]
fn reading_gives_the_pages_text_without_passwords() {
    let w = world("read", true);
    ok(w.chrome.call(A, "chrome_open", &json!({ "url": "https://example.com", "profile": "Default" })));
    let text = ok(w.chrome.call(A, "chrome_read", &json!({})));
    assert!(text.starts_with("\"Sign in · Example\" · w-900\n"), "{text}");
    assert!(text.contains("Sign in to Example") && text.contains("Forgot password?"), "{text}");
    assert!(!text.contains("hunter2"), "a password field's value never shows");
    let found = ok(w.chrome.call(A, "chrome_read", &json!({ "find": "forgot" })));
    assert_eq!(found, "\"Sign in · Example\" · w-900\nForgot password?");
}

#[test]
fn closing_frees_the_window_and_a_closed_window_is_forgotten() {
    let w = world("close", true);
    ok(w.chrome.call(A, "chrome_open", &json!({ "url": "https://example.com", "profile": "Default" })));
    ok(w.chrome.call(A, "chrome_open", &json!({ "url": "https://example.org", "profile": "Default" })));
    assert_eq!(ok(w.chrome.call(A, "chrome_close", &json!({ "window": "w-900" }))), "closed w-900");
    assert_eq!(*w.closed.lock().unwrap(), [900]);
    assert!(refused(w.chrome.call(A, "chrome_snapshot", &json!({ "window": "w-900" }))).contains("isn't one of your"));
    // The person closed the other one themselves.
    w.windows.lock().unwrap().retain(|(id, _)| id != "w-901");
    assert!(refused(w.chrome.call(A, "chrome_snapshot", &json!({}))).contains("was closed"));
    assert!(refused(w.chrome.call(A, "chrome_snapshot", &json!({}))).contains("chrome_open"), "and forgotten");
}

#[test]
fn a_sign_in_hand_off_finds_the_members_window_for_the_site() {
    let w = world("handoff", true);
    assert!(w.chrome.window_for_site(A, Some("https://github.com")).is_err());
    ok(w.chrome.call(A, "chrome_open", &json!({ "url": "https://github.com/login", "profile": "Work" })));
    ok(w.chrome.call(A, "chrome_open", &json!({ "url": "https://example.com", "profile": "Default" })));
    let found = w.chrome.window_for_site(A, Some("https://github.com/session")).unwrap();
    assert_eq!(found["id"], "w-900");
    assert_eq!(found["profile"], "Work");
    assert_eq!(found["site"], "github.com");
    let latest = w.chrome.window_for_site(A, Some("https://accounts.google.com")).unwrap();
    assert_eq!(latest["id"], "w-901", "another site (a sign-in redirect): the latest window");
    assert!(w.chrome.window_for_site(B, None).is_err(), "another member has none");
}

#[test]
fn the_desktop_tools_stay_out_of_the_persons_chrome() {
    let w = world("generic", true);
    assert!(w.chrome.guard_generic(A, "snapshot", &json!({ "app": "Google Chrome" })).is_some());
    assert!(w.chrome.guard_generic(A, "find", &json!({ "app": "com.google.Chrome", "text": "x" })).is_some());
    assert!(w.chrome.guard_generic(A, "snapshot", &json!({ "app": "TextEdit" })).is_none());
    ok(w.chrome.call(A, "chrome_open", &json!({ "url": "https://example.com", "profile": "Default" })));
    ok(w.chrome.call(A, "chrome_snapshot", &json!({})));
    assert!(w.chrome.guard_generic(A, "act", &json!({ "ref": "e5" })).is_some(), "a Chrome ref goes through chrome_act");
    let read = Answer { text: "Google Chrome · \"Bank\" · w-1 · snapshot s1 · 3 refs\ne1 button \"Pay\"".into(), is_error: false };
    let guarded = w.chrome.guard_answer(A, "snapshot", read);
    assert!(guarded.is_error && !guarded.text.contains("Bank"), "{}", guarded.text);
    let other = Answer { text: "TextEdit · \"Untitled\" · w-2".into(), is_error: false };
    assert!(!w.chrome.guard_answer(A, "snapshot", other).is_error);
}

#[test]
fn the_action_log_says_what_was_done_never_what_was_typed() {
    let w = world("log", true);
    ok(w.chrome.call(A, "chrome_open", &json!({ "url": "https://example.com/reset?token=abc123", "profile": "Default" })));
    ok(w.chrome.call(A, "chrome_snapshot", &json!({})));
    ok(w.chrome.call(A, "chrome_act", &json!({ "ref": "e1", "action": "set_value", "text": "secret words" })));
    refused(w.chrome.call(A, "chrome_act", &json!({ "ref": "e3", "action": "check" })));
    let log = std::fs::read_to_string(log_path(&w.home)).unwrap();
    let lines: Vec<Value> = log.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 4, "{log}");
    assert_eq!(lines[0]["site"], "example.com");
    assert_eq!(lines[2]["action"], "set_value");
    assert_eq!(lines[3]["ok"], false);
    assert!(!log.contains("secret words") && !log.contains("abc123") && !log.contains("/reset"), "{log}");
}

#[test]
fn window_ids_read_in_every_form() {
    assert_eq!(window_id("w-4521").as_deref(), Some("w-4521"));
    assert_eq!(window_id("w4521").as_deref(), Some("w-4521"));
    assert_eq!(window_id(" 4521 ").as_deref(), Some("w-4521"));
    assert_eq!(window_id("main"), None);
    assert_eq!(window_number("w-7"), Some(7));
    assert_eq!(squad_of("acme/frontend"), "acme");
    assert!(is_chrome("google chrome") && is_chrome("com.google.Chrome") && !is_chrome("Chromium"));
    assert!(on_engine("chrome_act") && on_engine(WINDOW) && !on_engine(CAPTURE) && !on_engine("snapshot"));
}
