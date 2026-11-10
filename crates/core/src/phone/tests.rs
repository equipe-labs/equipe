//! The phone tools' rules, against a fake simulator.

use super::exec::fake::Fake;
use super::*;

const LIST: &str = r#"{"devices":{"rt.26":[
    {"name":"iPhone 17 Pro","udid":"PERSONS","state":"Booted"},
    {"name":"T12 iPhone 17 Pro","udid":"BYHAND","state":"Shutdown"}]}}"#;
const RUNTIMES: &str = r#"{"runtimes":[{"platform":"iOS","version":"26.5","identifier":"rt.26","isAvailable":true,"supportedDeviceTypes":[{"name":"iPhone 17 Pro"}]}]}"#;
const SCREEN: &str = r#"[{"type":"Application","frame":{"x":0,"y":0,"width":402,"height":874},"children":[
    {"type":"Button","AXLabel":"Pay","AXUniqueId":"pay","frame":{"x":20,"y":700,"width":362,"height":50},"children":[]},
    {"type":"StaticText","AXLabel":"Cart (1)","frame":{"x":20,"y":100,"width":200,"height":30},"children":[]}]}]"#;

struct Rig {
    fake: Fake,
    phones: Phones,
    home: PathBuf,
}

impl Rig {
    fn new(with_axe: bool) -> Self {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let home = std::env::temp_dir().join(format!("equipe-phone-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        if with_axe {
            std::fs::create_dir_all(axe::dir(&home)).unwrap();
            std::fs::write(axe::dir(&home).join("axe"), "").unwrap();
        }
        let fake = Fake::default();
        fake.on("simctl list devices -j", LIST).on("simctl list runtimes -j", RUNTIMES).on("simctl create", "OURS\n").on("describe-ui", SCREEN);
        Rig { fake, phones: Phones::default(), home }
    }

    fn call(&self, member: &str, ticket: u32, name: &str, args: Value) -> Outcome {
        let c = Call {
            squad: "acme".into(),
            member: member.into(),
            ticket,
            platforms: Platforms { ios: true, android: false },
            recordings: recordings_dir(&self.home.join("home"), ticket),
            equipe_home: self.home.clone(),
            cwd: self.home.clone(),
        };
        call(&self.phones, &self.fake, &c, name, &args)
    }

    fn text(&self, member: &str, ticket: u32, name: &str, args: Value) -> (bool, String) {
        match self.call(member, ticket, name, args) {
            Outcome::Done(r) | Outcome::Report(_, r) => (!r.is_error, r.text),
            Outcome::NeedsAxe => (false, "needs axe".into()),
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

#[test]
fn a_member_gets_a_device_of_its_own_for_the_ticket_and_never_anyone_else_s() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let rig = Rig::new(true);
    let (ok, text) = rig.text("qa", 12, START, json!({}));
    assert!(ok, "{text}");
    assert!(text.contains("acme T12 qa iPhone 17 Pro is up") && text.contains("id=OURS"), "{text}");
    assert!(rig.fake.called("xcrun simctl create acme T12 qa iPhone 17 Pro iPhone 17 Pro rt.26"));
    assert!(rig.fake.called("xcrun simctl boot OURS"));
    let touched_others = rig.fake.calls().iter().any(|c| c.contains("PERSONS") || c.contains("BYHAND"));
    assert!(!touched_others, "the person's simulator and one made by hand are left alone: {:?}", rig.fake.calls());
}

#[test]
fn every_tool_refuses_without_a_device_or_on_another_ticket_s() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let rig = Rig::new(true);
    let (ok, text) = rig.text("qa", 12, ACT, json!({"do": "tap", "target": "Pay"}));
    assert!(!ok && text.contains("isn't up"), "{text}");
    rig.text("qa", 12, START, json!({}));
    let (ok, text) = rig.text("qa", 13, ACT, json!({"do": "tap", "target": "Pay"}));
    assert!(!ok && text.contains("T-12's"), "{text}");
    let (ok, text) = rig.text("frontend", 12, LOOK, json!({"screenshot": false}));
    assert!(!ok && text.contains("isn't up"), "another member's device isn't theirs: {text}");
}

#[test]
fn starting_on_another_ticket_puts_the_last_device_away_first() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let rig = Rig::new(true);
    rig.text("qa", 12, START, json!({}));
    rig.fake.on("simctl create", "OURS13\n");
    let (ok, text) = rig.text("qa", 13, START, json!({}));
    assert!(ok && text.starts_with("Shut down your last device, acme T12 qa iPhone 17 Pro."), "{text}");
    assert!(rig.fake.called("xcrun simctl shutdown OURS"));
    let seats = rig.phones.seats.lock().unwrap();
    assert_eq!(seats.len(), 1, "one device a member");
}

#[test]
fn a_tap_aims_by_label_and_answers_with_the_screen() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let rig = Rig::new(true);
    rig.text("qa", 12, START, json!({}));
    let (ok, text) = rig.text("qa", 12, ACT, json!({"do": "tap", "target": "pay"}));
    assert!(ok, "{text}");
    assert!(text.starts_with("Done: tapped \"pay\". The screen now:\nButton \"Pay\""), "{text}");
    let axe = axe::dir(&rig.home).join("axe").to_string_lossy().into_owned();
    assert!(rig.fake.calls().contains(&format!("{axe} tap -x 201.0 -y 725.0 --udid OURS")), "{:?}", rig.fake.calls());
    let (ok, text) = rig.text("qa", 12, ACT, json!({"do": "tap", "target": "Checkout"}));
    assert!(!ok && text.contains("Nothing on the screen is \"Checkout\""), "{text}");
    let (ok, text) = rig.text("qa", 12, ACT, json!({"do": "key", "text": "home"}));
    assert!(ok, "{text}");
    let (ok, text) = rig.text("qa", 12, ACT, json!({"do": "fly"}));
    assert!(!ok && text.contains("isn't something"), "{text}");
}

#[test]
fn without_axe_ios_asks_for_it_and_after_a_no_says_what_still_works() {
    if !cfg!(target_os = "macos") || axe::find(Path::new("/nonexistent")).is_some() {
        return;
    }
    let rig = Rig::new(false);
    let (ok, text) = rig.text("qa", 12, START, json!({}));
    assert!(ok, "starting needs no AXe: {text}");
    assert!(matches!(rig.call("qa", 12, ACT, json!({"do": "tap", "target": "Pay"})), Outcome::NeedsAxe));
    rig.phones.decline_axe();
    let (ok, text) = rig.text("qa", 12, ACT, json!({"do": "tap", "target": "Pay"}));
    assert!(!ok && text.contains("declined"), "{text}");
    let (ok, text) = rig.text("qa", 12, LOOK, json!({"screenshot": false}));
    assert!(ok && text.contains("declined"), "{text}");
}

#[test]
fn a_recording_marks_each_action_and_is_kept_with_the_ticket() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let rig = Rig::new(true);
    rig.text("qa", 12, START, json!({}));
    let (ok, text) = rig.text("qa", 12, RECORD, json!({"action": "start", "name": "Check out!"}));
    assert!(ok && text.contains("Recording check-out to"), "{text}");
    let video = recordings_dir(&rig.home.join("home"), 12).join("check-out.mp4");
    assert!(rig.fake.called(&format!("spawn xcrun simctl io OURS recordVideo --codec=h264 --force {}", video.display())));
    let (ok, text) = rig.text("qa", 12, RECORD, json!({"action": "start"}));
    assert!(!ok && text.contains("already"), "{text}");
    rig.text("qa", 12, ACT, json!({"do": "tap", "target": "Pay", "step": "tap Pay"}));
    let (ok, text) = rig.text("qa", 12, RECORD, json!({"action": "stop"}));
    assert!(ok && text.contains("Marked: 0:00.0 tap Pay"), "{text}");
    assert!(rig.fake.called("SIGINT xcrun simctl io OURS recordVideo"), "stopped as Ctrl-C would, so the file is finished");
    let marks = load_marks(&video);
    assert_eq!(marks.len(), 1);
    assert!(marks[0].what == "tap Pay" && marks[0].at < 1., "{marks:?}");
    let (ok, text) = rig.text("qa", 12, RECORD, json!({"action": "start", "name": "check out"}));
    assert!(ok, "{text}");
}

#[test]
fn stopping_saves_the_recording_and_shuts_the_device_down() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let rig = Rig::new(true);
    rig.text("qa", 12, START, json!({}));
    rig.text("qa", 12, RECORD, json!({"action": "start", "name": "flow"}));
    let (ok, text) = rig.text("qa", 12, STOP, json!({}));
    assert!(ok && text.starts_with("Saved the recording"), "{text}");
    assert!(rig.fake.called("SIGINT"));
    assert!(rig.fake.called("xcrun simctl shutdown OURS"));
    let (ok, text) = rig.text("qa", 12, STOP, json!({}));
    assert!(!ok && text.contains("nothing to stop"), "{text}");
}

#[test]
fn a_ticket_done_with_lets_go_of_its_devices_only() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let rig = Rig::new(true);
    rig.text("qa", 12, START, json!({}));
    rig.fake.on("simctl create", "MOBILE14\n");
    rig.text("mobile", 14, START, json!({}));
    let gone = rig.phones.release_ticket(&rig.fake, "acme", 12);
    assert_eq!(gone, vec!["acme T12 qa iPhone 17 Pro".to_owned()]);
    assert!(rig.fake.called("xcrun simctl shutdown OURS") && !rig.fake.called("xcrun simctl shutdown MOBILE14"));
}

#[test]
fn the_platform_is_the_squad_s_or_asked_for() {
    let rig = Rig::new(true);
    let c = Call {
        squad: "acme".into(),
        member: "qa".into(),
        ticket: 12,
        platforms: Platforms { ios: true, android: true },
        recordings: rig.home.join("r"),
        equipe_home: rig.home.clone(),
        cwd: rig.home.clone(),
    };
    let Outcome::Done(r) = call(&rig.phones, &rig.fake, &c, START, &json!({})) else { panic!() };
    assert!(r.is_error && r.text.contains("say which"), "{}", r.text);
    let Outcome::Done(r) = call(&rig.phones, &rig.fake, &Call { platforms: Platforms { ios: true, android: false }, ..c.clone() }, START, &json!({"platform": "android"})) else { panic!() };
    assert!(r.is_error && r.text.contains("doesn't run on android"), "{}", r.text);
}

#[test]
fn the_tools_say_how_to_use_them_and_every_name_is_unique() {
    let tools = tools();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, [START, LOOK, ACT, RECORD, FRAMES, REPORT, STOP]);
    for t in &tools {
        assert!(t["description"].as_str().unwrap().len() > 150, "{}", t["name"]);
        assert_eq!(t["inputSchema"]["type"], "object");
    }
    let report = &tools[5]["description"];
    assert!(report.as_str().unwrap().contains("without its frame is refused"));
    assert_eq!(slug("  Check out!  "), "check-out");
    assert_eq!(slug("결제 흐름"), "결제-흐름", "a name in any script stays");
    assert_eq!(slug("!!"), "flow");
}
