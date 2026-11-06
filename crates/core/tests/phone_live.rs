//! The phone tools on a real iOS Simulator, end to end: AXe installed from its
//! pinned release, a simulator made for the ticket, the app installed and
//! launched, a flow recorded and checked frame by frame, reported, and the
//! simulator deleted after. Needs Xcode with an iOS runtime and a network, so
//! it runs only when asked:
//!
//! ```sh
//! EQUIPE_PHONE_LIVE_APP=/path/to/Shop.app cargo test -p equipe-core --test phone_live -- --ignored --nocapture
//! ```
//!
//! The app is one with an "Add item" button, a "Checkout" link and a total
//! with the id `total` (docs/mobile-qa.md has it). `EQUIPE_PHONE_LIVE_OUT`, if
//! set, gets a copy of the recording, its frames and its report.

use equipe_core::phone::{self, Call, Outcome, Platforms, exec::Real};
use serde_json::json;

#[test]
#[ignore = "drives a real iOS Simulator"]
fn a_flow_is_recorded_checked_and_reported_on_a_real_simulator() {
    let Ok(app) = std::env::var("EQUIPE_PHONE_LIVE_APP") else {
        eprintln!("EQUIPE_PHONE_LIVE_APP isn't set: nothing to test");
        return;
    };
    let home = std::env::temp_dir().join(format!("eqlive-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let axe = phone::axe::install(&Real, &home).expect("AXe installs from its pinned release");
    assert!(axe.is_file());
    let c = Call {
        squad: "eqlive".into(),
        member: "qa".into(),
        ticket: 1,
        platforms: Platforms { ios: true, android: false },
        recordings: phone::recordings_dir(&home.join("squad"), 1),
        equipe_home: home.clone(),
        cwd: home.clone(),
    };
    let phones = phone::Phones::default();
    let run = |name: &str, args: serde_json::Value| -> (bool, String) {
        let started = std::time::Instant::now();
        let (ok, text) = match phone::call(&phones, &Real, &c, name, &args) {
            Outcome::Done(r) | Outcome::Report(_, r) => (!r.is_error, r.text),
            Outcome::NeedsAxe => (false, "needs AXe".into()),
        };
        eprintln!("── {name} {args} ({:.1}s) ok={ok}\n{text}\n", started.elapsed().as_secs_f64());
        (ok, text)
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (ok, text) = run(phone::START, json!({ "app": app }));
        assert!(ok && text.contains("launched com.example.shop"), "{text}");
        let (ok, text) = run(phone::ACT, json!({ "do": "wait", "target": "Add item", "timeout": 20 }));
        assert!(ok, "{text}");
        let (ok, text) = run(phone::LOOK, json!({ "screenshot": false }));
        assert!(ok && text.contains("Button \"Add item\""), "{text}");
        assert!(run(phone::RECORD, json!({ "action": "start", "name": "checkout" })).0);
        assert!(run(phone::ACT, json!({ "do": "tap", "target": "Add item", "step": "add the first item" })).0);
        let (ok, text) = run(phone::ACT, json!({ "do": "tap", "target": "Add item", "step": "add the second item" }));
        assert!(ok && text.contains("Items: 2"), "{text}");
        assert!(run(phone::ACT, json!({ "do": "tap", "target": "Checkout", "step": "open checkout" })).0);
        let (ok, text) = run(phone::ACT, json!({ "do": "wait", "target": "id:total" }));
        assert!(ok, "{text}");
        let (ok, text) = run(phone::LOOK, json!({ "screenshot": false }));
        assert!(ok && text.contains("Total: $5"), "the planted bug shows: {text}");
        let (ok, text) = run(phone::RECORD, json!({ "action": "stop" }));
        assert!(ok && text.contains("open checkout"), "{text}");
        let (ok, text) = run(phone::FRAMES, json!({}));
        assert!(ok, "{text}");
        let frames: Vec<String> = text.lines().filter_map(|l| l.rsplit(" — ").next()).filter(|p| p.ends_with(".jpg")).map(str::to_owned).collect();
        assert!(frames.len() >= 3, "frames before and after each step: {text}");
        let last = frames.last().unwrap().clone();
        let (ok, text) = run(
            phone::REPORT,
            json!({ "flow": "Checkout of two items", "recording": "checkout", "steps": [
                { "step": "Add two items", "expected": "Items: 2", "seen": "Items: 2", "verdict": "pass", "frame": frames[frames.len() / 2] },
                { "step": "Open checkout", "expected": "Total: $10", "seen": "Total: $5", "verdict": "fail", "frame": last },
            ]}),
        );
        assert!(ok && text.contains("1 of 2 steps passed"), "{text}");
        if let Ok(out) = std::env::var("EQUIPE_PHONE_LIVE_OUT") {
            let _ = std::fs::create_dir_all(&out);
            let _ = std::process::Command::new("cp").args(["-R"]).arg(&c.recordings).arg(&out).status();
        }
        assert!(run(phone::STOP, json!({})).0);
    }));
    // Whatever happened, the simulator made for it goes.
    for line in equipe_core::disk::clear_simulators(1, "eqlive", true) {
        eprintln!("{line}");
    }
    let _ = std::fs::remove_dir_all(&home);
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
