//! The engine against a stand-in agent-desktop: recorded envelopes in the
//! shape agent-desktop 1.2.0 prints (hand-written from its documented output,
//! not captured from anyone's screen). Nothing here touches the real desktop.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::*;

const CHAT: &str = include_str!("fixtures/slack-chat.json");
const SKELETON: &str = include_str!("fixtures/slack-skeleton.json");

type Calls = Arc<Mutex<Vec<(Vec<String>, Option<String>)>>>;
/// What the stand-in answers, by command; the latest of a command wins.
type Answers = Arc<Mutex<Vec<(&'static str, Value)>>>;

/// Answers by command name; keeps every call it got.
struct Fake {
    calls: Calls,
    answers: Answers,
}

impl Runner for Fake {
    fn run(&self, args: &[String], stdin: Option<&str>, _: Duration) -> Result<Value, String> {
        self.calls.lock().unwrap().push((args.to_vec(), stdin.map(str::to_owned)));
        // The command: the first argument that isn't a global flag or its value.
        let mut command = None;
        for a in args {
            if a.starts_with("--") {
                continue;
            }
            command = Some(a.as_str());
            break;
        }
        let command = command.unwrap_or_default();
        let answers = self.answers.lock().unwrap();
        let found = answers.iter().rev().find(|(c, _)| *c == command).map(|(_, v)| v.clone());
        Ok(found.unwrap_or_else(|| json!({ "ok": true, "command": command, "data": { "action": command, "disposition": { "delivery": "delivered_verified", "retry": "unsafe" } } })))
    }
}

fn engine() -> (Engine, Calls, Answers) {
    let calls: Calls = Default::default();
    let answers: Answers = Default::default();
    answers.lock().unwrap().push(("snapshot", serde_json::from_str(CHAT).unwrap()));
    answers.lock().unwrap().push((
        "list-apps",
        json!({ "ok": true, "command": "list-apps", "data": { "apps": [ { "name": "Slack", "pid": 811, "bundle_id": "com.tinyspeck.slackmacgap", "presentation": "foreground" } ] } }),
    ));
    (Engine::new(Box::new(Fake { calls: calls.clone(), answers: answers.clone() })), calls, answers)
}

const M: &str = "acme/frontend";

fn last_args(calls: &Calls) -> Vec<String> {
    calls.lock().unwrap().last().unwrap().0.clone()
}

#[test]
fn a_snapshot_runs_in_the_members_own_session_and_reads_compact() {
    let (engine, calls, _) = engine();
    let answer = engine.call(M, "snapshot", &json!({ "app": "Slack" }));
    assert!(!answer.is_error, "{}", answer.text);
    let args = last_args(&calls);
    assert_eq!(args[0], format!("--session={}", session_of(M)));
    assert!(session_of(M).starts_with("eq-") && !session_of(M).contains("frontend"), "no names on disk");
    assert_eq!(&args[1..], ["snapshot", "--app=Slack", "-i", "--compact"]);
    assert!(answer.text.starts_with("Slack · \"launch-plan (Channel) - Acme - Slack\" · w-4521 · snapshot s4k9m2qa · 75 refs"), "{}", answer.text);
    assert!(answer.text.contains("e74 button \"Send now\""), "{}", answer.text);
    assert!(answer.text.contains("e70 textfield \"Message #launch-plan\" =\"Moving the review to 3pm tomorrow, does that work?\" focused"));
    assert_ne!(session_of("acme/backend"), session_of(M));
}

#[test]
fn skeleton_and_drill_down_pass_through_and_a_drill_keeps_its_snapshots_app() {
    let (engine, calls, answers) = engine();
    answers.lock().unwrap().push(("snapshot", serde_json::from_str(SKELETON).unwrap()));
    let map = engine.call(M, "snapshot", &json!({ "app": "Slack", "skeleton": true }));
    assert!(map.text.contains("e1 group \"Sidebar\" +23"), "{}", map.text);
    assert!(last_args(&calls).contains(&"--skeleton".to_owned()));
    // The drill answers with the composer's refs, under the same snapshot id.
    answers.lock().unwrap().push(("snapshot", serde_json::from_str(CHAT).unwrap()));
    engine.call(M, "snapshot", &json!({ "root": "e4" }));
    assert_eq!(&last_args(&calls)[1..], ["snapshot", "--root=@s4k9m2qa:e4", "-i", "--compact"]);
    let scene = engine.scene(M, "act", &json!({ "ref": "e74", "action": "click" })).unwrap();
    assert_eq!(scene["app"]["name"], "Slack");
}

#[test]
fn a_ref_action_is_headless_and_names_its_qualified_ref() {
    let (engine, calls, _) = engine();
    engine.call(M, "snapshot", &json!({ "app": "Slack" }));
    let answer = engine.call(M, "act", &json!({ "ref": "e12", "action": "click" }));
    assert_eq!(answer.text, "click e12: delivered verified");
    assert_eq!(&last_args(&calls)[1..], ["click", "--", "@s4k9m2qa:e12"], "no --headed: no pointer, no focus");
    engine.call(M, "act", &json!({ "ref": "s4k9m2qa:e12", "action": "double_click" }));
    assert_eq!(&last_args(&calls)[1..], ["--headed", "double-click", "--", "@s4k9m2qa:e12"], "a double click is physical");
    engine.call(M, "act", &json!({ "ref": "e3", "action": "scroll", "direction": "up", "amount": 5 }));
    assert_eq!(&last_args(&calls)[1..], ["scroll", "--direction=up", "--amount=5", "--", "@s4k9m2qa:e3"]);
}

#[test]
fn voiceover_is_neither_launched_nor_closed_and_other_apps_still_are() {
    let (engine, calls, _) = engine();
    for action in ["launch", "close"] {
        for app in ["VoiceOver", "VoiceOver Utility"] {
            let answer = engine.call(M, "apps", &json!({ "action": action, "app": app }));
            assert!(answer.is_error && answer.text.contains("accessibility tree"), "{action} {app}: {}", answer.text);
        }
    }
    assert!(calls.lock().unwrap().is_empty(), "nothing reached the engine");
    assert!(!engine.call(M, "apps", &json!({ "action": "launch", "app": "Simulator" })).is_error);
}

#[test]
fn nothing_is_acted_on_in_system_settings_but_it_can_be_read() {
    let (engine, calls, answers) = engine();
    let settings = CHAT.replace("\"Slack\"", "\"System Settings\"");
    answers.lock().unwrap().push(("snapshot", serde_json::from_str(&settings).unwrap()));
    engine.call(M, "snapshot", &json!({ "app": "System Settings" }));
    let before = calls.lock().unwrap().len();
    let answer = engine.call(M, "act", &json!({ "ref": "e12", "action": "click" }));
    assert!(answer.is_error && answer.text.starts_with("Blocked:"), "{}", answer.text);
    assert_eq!(calls.lock().unwrap().len(), before, "no click was sent");
}

#[test]
fn typed_text_goes_in_on_stdin_never_in_the_arguments() {
    let (engine, calls, _) = engine();
    engine.call(M, "snapshot", &json!({ "app": "Slack" }));
    let answer = engine.call(M, "act", &json!({ "ref": "e70", "action": "type", "text": "s3cret pass phrase" }));
    assert!(!answer.is_error, "{}", answer.text);
    let (args, stdin) = calls.lock().unwrap().last().unwrap().clone();
    assert_eq!(&args[1..], ["type", "--stdin", "--", "@s4k9m2qa:e70"]);
    assert_eq!(stdin.as_deref(), Some("s3cret pass phrase"));
    assert!(args.iter().all(|a| !a.contains("s3cret")));
    engine.call(M, "clipboard", &json!({ "action": "set", "text": "token-123" }));
    let (args, stdin) = calls.lock().unwrap().last().unwrap().clone();
    assert!(args.iter().all(|a| !a.contains("token-123")) && stdin.as_deref() == Some("token-123"));
}

#[test]
fn nothing_a_member_writes_reads_as_a_flag() {
    let (engine, calls, _) = engine();
    engine.call(M, "snapshot", &json!({ "app": "--headed" }));
    assert_eq!(&last_args(&calls)[1..], ["snapshot", "--app=--headed", "-i", "--compact"]);
    engine.call(M, "snapshot", &json!({ "app": "Slack" }));
    engine.call(M, "act", &json!({ "ref": "e40", "action": "select", "value": "--headed" }));
    assert_eq!(&last_args(&calls)[1..], ["select", "--", "@s4k9m2qa:e40", "--headed"]);
    engine.call(M, "apps", &json!({ "action": "close", "app": "--force" }));
    assert_eq!(&last_args(&calls)[1..], ["close-app", "--", "--force"]);
    let refused = engine.call(M, "act", &json!({ "ref": "e3", "action": "scroll", "direction": "--headed" }));
    assert!(refused.is_error, "{}", refused.text);
}

#[test]
fn a_ref_no_snapshot_showed_is_refused_before_anything_runs() {
    let (engine, calls, _) = engine();
    let refused = engine.call(M, "act", &json!({ "ref": "e3" }));
    assert!(refused.is_error && refused.text.contains("Take a snapshot first"), "{}", refused.text);
    engine.call(M, "snapshot", &json!({ "app": "Slack" }));
    let before = calls.lock().unwrap().len();
    let refused = engine.call(M, "act", &json!({ "ref": "zz99:e3" }));
    assert!(refused.is_error && refused.text.contains("isn't in any snapshot"), "{}", refused.text);
    let refused = engine.call(M, "act", &json!({ "ref": "rm -rf" }));
    assert!(refused.is_error && refused.text.contains("isn't a ref"), "{}", refused.text);
    // Another member's refs are its own.
    let refused = engine.call("acme/backend", "act", &json!({ "ref": "s4k9m2qa:e12" }));
    assert!(refused.is_error, "{}", refused.text);
    assert_eq!(calls.lock().unwrap().len(), before, "none of these reached agent-desktop");
}

#[test]
fn the_scene_of_a_send_button_names_the_app_the_button_and_the_draft() {
    let (engine, _, _) = engine();
    engine.call(M, "snapshot", &json!({ "app": "Slack" }));
    let scene = engine.scene(M, "act", &json!({ "ref": "e74", "action": "click" })).unwrap();
    assert_eq!(scene["app"], json!({ "name": "Slack", "bundle_id": "com.tinyspeck.slackmacgap" }));
    assert_eq!(scene["window"], "launch-plan (Channel) - Acme - Slack");
    assert_eq!(scene["target"], json!({ "role": "AXButton", "title": "Send now" }));
    assert_eq!(scene["focused"]["value"], "Moving the review to 3pm tomorrow, does that work?");
    assert_eq!(scene["draft_ref"], "@s4k9m2qa:e70");
    // Typing goes into the field itself.
    let typing = engine.scene(M, "act", &json!({ "ref": "e70", "action": "type", "text": "hi" })).unwrap();
    assert_eq!(typing["focused"]["role"], "AXTextField");
    assert_eq!(typing["target"], typing["focused"]);
}

#[test]
fn a_password_field_reads_as_one_to_the_gate() {
    let (engine, _, answers) = engine();
    answers.lock().unwrap().push((
        "snapshot",
        json!({ "ok": true, "command": "snapshot", "data": { "app": "Safari", "snapshot_id": "s1", "ref_count": 1, "complete": true, "tree": { "role": "window", "children": [
            { "ref_id": "@s1:e1", "role": "textfield", "name": "Password", "value": "•••", "states": ["secure", "focused"] }
        ] } } }),
    ));
    engine.call(M, "snapshot", &json!({ "app": "Safari" }));
    let scene = engine.scene(M, "act", &json!({ "ref": "e1", "action": "type", "text": "x" })).unwrap();
    assert_eq!(scene["focused"], json!({ "role": "AXSecureTextField", "title": "Password" }), "and never its value");
}

#[test]
fn a_checked_input_goes_ahead_only_while_its_scene_holds() {
    let (engine, calls, answers) = engine();
    engine.call(M, "snapshot", &json!({ "app": "Slack" }));
    let send = json!({ "ref": "e74", "action": "click", "why": "reply to Kim" });
    let mut scene = engine.scene(M, "act", &send).unwrap();
    let mut args = send.clone();
    args[crate::controller::EXPECT] = scene.clone();
    assert!(!engine.call(M, "act", &args).is_error);
    // Checked against another element: refused, nothing run.
    scene["target"]["title"] = json!("Huddle");
    args[crate::controller::EXPECT] = scene.clone();
    let before = calls.lock().unwrap().len();
    assert!(engine.call(M, "act", &args).text.starts_with("The screen changed"));
    assert_eq!(calls.lock().unwrap().len(), before);
    // After the person's yes, the conversation and the draft are read again live.
    let scene = engine.scene(M, "act", &send).unwrap();
    let mut asked = send.clone();
    asked[crate::controller::EXPECT] = scene;
    asked[crate::controller::ASKED] = json!(true);
    answers.lock().unwrap().push(("list-windows", json!({ "ok": true, "command": "list-windows", "data": [ { "id": "w-4521", "title": "random (Channel) - Acme - Slack", "app_name": "Slack" } ] })));
    let refused = engine.call(M, "act", &asked);
    assert!(refused.is_error && refused.text.starts_with("The screen changed"), "another conversation: {}", refused.text);
    answers.lock().unwrap().push(("list-windows", json!({ "ok": true, "command": "list-windows", "data": [ { "id": "w-4521", "title": "launch-plan (Channel) - Acme - Slack", "app_name": "Slack" } ] })));
    answers.lock().unwrap().push(("get", json!({ "ok": true, "command": "get", "data": { "value": "Moving the review to 4pm tomorrow, does that work?" } })));
    assert!(engine.call(M, "act", &asked).is_error, "another draft than the one the person read");
    answers.lock().unwrap().push(("get", json!({ "ok": true, "command": "get", "data": { "value": "Moving the review to 3pm tomorrow, does that work?" } })));
    let sent = engine.call(M, "act", &asked);
    assert!(!sent.is_error, "{}", sent.text);
    assert_eq!(&last_args(&calls)[1..], ["click", "--", "@s4k9m2qa:e74"]);
}

#[test]
fn a_notification_is_acted_on_only_as_listed_and_by_its_fingerprint() {
    let (engine, calls, answers) = engine();
    let refused = engine.call(M, "notifications", &json!({ "action": "act", "index": 1, "name": "Reply" }));
    assert!(refused.is_error, "{}", refused.text);
    answers.lock().unwrap().push((
        "list-notifications",
        json!({ "ok": true, "command": "list-notifications", "data": { "notifications": [ { "index": 1, "app_name": "Slack", "title": "Kim Minji", "body": "lunch?", "actions": ["Reply"] } ] } }),
    ));
    let listed = engine.call(M, "notifications", &json!({}));
    assert_eq!(listed.text, "1 Slack \"Kim Minji\": lunch? [Reply]");
    let scene = engine.scene(M, "notifications", &json!({ "action": "act", "index": 1, "name": "Reply" })).unwrap();
    assert_eq!(scene["target"]["title"], "Reply");
    assert_eq!(scene["app"]["bundle_id"], "com.tinyspeck.slackmacgap");
    engine.call(M, "notifications", &json!({ "action": "act", "index": 1, "name": "Reply" }));
    assert_eq!(&last_args(&calls)[1..], ["--headed", "notification-action", "--expected-app=Slack", "--", "1", "Reply"]);
}

#[test]
fn errors_come_back_compact_with_whether_to_retry() {
    let (engine, _, answers) = engine();
    engine.call(M, "snapshot", &json!({ "app": "Slack" }));
    answers.lock().unwrap().push((
        "click",
        json!({ "ok": false, "command": "click", "error": { "code": "STALE_REF", "message": "@s4k9m2qa:e12 not found in current RefMap", "disposition": { "delivery": "not_delivered", "retry": "safe" } } }),
    ));
    let answer = engine.call(M, "act", &json!({ "ref": "e12" }));
    assert!(answer.is_error);
    assert_eq!(answer.text, "STALE_REF: @s4k9m2qa:e12 not found in current RefMap (not delivered: safe to retry) Take a fresh snapshot and use its refs.");
}

#[test]
fn the_engine_reports_what_macos_lets_it_do() {
    let (engine, _, answers) = engine();
    answers.lock().unwrap().push((
        "permissions",
        json!({ "ok": true, "command": "permissions", "data": { "accessibility": { "state": "granted" }, "screen_recording": { "state": "denied" } } }),
    ));
    let status = engine.status();
    assert_eq!(status["installed"], true);
    assert_eq!(status["accessibility"], "granted");
    assert_eq!(status["version"], pin::VERSION);
}

/// The measure behind docs/DESKTOP-CONTROL.md: what a member reads to work
/// one chat window, as a tree and as a screenshot. Run with
/// `cargo test -p equipe-desktop-control snapshot_costs -- --nocapture`.
#[test]
fn snapshot_costs_a_fraction_of_a_screenshot() {
    let chat: Value = serde_json::from_str(CHAT).unwrap();
    let skeleton: Value = serde_json::from_str(SKELETON).unwrap();
    let raw = CHAT.len();
    let full = compact::snapshot(&chat["data"]);
    let map = compact::snapshot(&skeleton["data"]);
    // Text: about four bytes a token (English, JSON-ish). Images: Claude
    // counts width × height / 750 tokens, at the size the screenshot tool
    // sends (a 1470×956 pt display at 2×, sized for the model).
    let tokens = |bytes: usize| bytes.div_ceil(4);
    let shot = crate::geometry::Mapping::for_frame(crate::geometry::Frame { x: 0.0, y: 0.0, w: 1470.0, h: 956.0 }, 2.0);
    let image_tokens = (shot.width * shot.height).div_ceil(750);
    println!("raw agent-desktop JSON (-i --compact): {raw} bytes ≈ {} tokens", tokens(raw));
    println!("equipe's compact tree, full window:    {} bytes ≈ {} tokens ({} lines)", full.len(), tokens(full.len()), full.lines().count());
    println!("equipe's compact tree, skeleton:       {} bytes ≈ {} tokens", map.len(), tokens(map.len()));
    println!("screenshot {}×{}: ≈ {image_tokens} tokens (plus the base64 JPEG kept in the engine's transcript)", shot.width, shot.height);
    assert!(full.len() * 2 < raw, "the compact tree is less than half the raw JSON");
    assert!(tokens(full.len()) < image_tokens, "the full tree costs less than one screenshot");
    assert!(tokens(map.len()) * 10 < image_tokens, "a skeleton costs under a tenth of one");
}

#[test]
fn a_headed_click_that_times_out_unsent_goes_in_with_the_pointer_at_the_elements_centre() {
    let (mut engine, calls, answers) = engine();
    engine.pointer_fallback = true;
    assert!(!engine.call(M, "snapshot", &json!({ "app": "Slack" })).is_error);
    let timeout = |delivery: &str| json!({ "ok": false, "command": "click", "error": { "code": "TIMEOUT", "message": "did not become actionable", "disposition": { "delivery": delivery, "retry": "safe" } } });
    answers.lock().unwrap().push(("click", timeout("not_delivered")));
    answers.lock().unwrap().push(("get", json!({ "ok": true, "command": "get", "data": { "property": "bounds", "value": { "x": 100.0, "y": 40.0, "width": 80.0, "height": 20.0 } } })));
    let clicked = engine.call(M, "act", &json!({ "ref": "e3", "headed": true }));
    assert!(!clicked.is_error && clicked.text.contains("pointer at its centre (140, 50)"), "{}", clicked.text);
    assert_eq!(&last_args(&calls)[1..], ["--headed", "mouse-click", "--xy=140,50"]);
    // Something may have landed: told, never clicked again.
    answers.lock().unwrap().push(("click", timeout("delivery_uncertain")));
    let before = calls.lock().unwrap().len();
    let uncertain = engine.call(M, "act", &json!({ "ref": "e3", "headed": true }));
    assert!(uncertain.is_error && uncertain.text.contains("TIMEOUT"));
    assert_eq!(calls.lock().unwrap().len(), before + 1);
    // Headless: no pointer.
    answers.lock().unwrap().push(("click", timeout("not_delivered")));
    assert!(engine.call(M, "act", &json!({ "ref": "e3" })).is_error);
}
