//! The phone tools on a real Android emulator: an AVD written for the ticket,
//! booted without a window, its home screen read and touched, a short flow
//! recorded and framed, then the emulator stopped and its AVD deleted. Needs
//! the Android SDK with the emulator and a system image, so it runs only when
//! asked:
//!
//! ```sh
//! EQUIPE_PHONE_LIVE_ANDROID=1 cargo test -p equipe-core --test phone_live_android -- --ignored --nocapture
//! ```

use equipe_core::phone::{self, Call, Outcome, Platforms, exec::Real};
use serde_json::json;

#[test]
#[ignore = "drives a real Android emulator"]
fn a_flow_is_recorded_and_framed_on_a_real_emulator() {
    if std::env::var_os("EQUIPE_PHONE_LIVE_ANDROID").is_none() {
        eprintln!("EQUIPE_PHONE_LIVE_ANDROID isn't set: nothing to test");
        return;
    }
    let home = std::env::temp_dir().join(format!("eqlive-android-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let c = Call {
        squad: "eqlive".into(),
        member: "qa".into(),
        ticket: 2,
        platforms: Platforms { ios: false, android: true },
        recordings: phone::recordings_dir(&home.join("squad"), 2),
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
        eprintln!("── {name} {args} ({:.1}s) ok={ok}\n{}\n", started.elapsed().as_secs_f64(), text.chars().take(1500).collect::<String>());
        (ok, text)
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (ok, text) = run(phone::START, json!({}));
        assert!(ok && text.contains("eqlive_T2_qa is up"), "{text}");
        let (ok, text) = run(phone::LOOK, json!({}));
        assert!(ok && text.contains(" @ "), "the home screen's elements: {text}");
        assert!(run(phone::RECORD, json!({ "action": "start", "name": "home" })).0);
        assert!(run(phone::ACT, json!({ "do": "scroll", "direction": "down", "step": "open the app drawer" })).0);
        assert!(run(phone::ACT, json!({ "do": "key", "text": "home", "step": "go home" })).0);
        let (ok, text) = run(phone::RECORD, json!({ "action": "stop" }));
        assert!(ok && text.contains("go home"), "{text}");
        let (ok, text) = run(phone::FRAMES, json!({}));
        assert!(ok && text.contains(".jpg"), "{text}");
        if let Ok(out) = std::env::var("EQUIPE_PHONE_LIVE_OUT") {
            let _ = std::fs::create_dir_all(&out);
            let _ = std::process::Command::new("cp").args(["-R"]).arg(&c.recordings).arg(&out).status();
        }
        assert!(run(phone::STOP, json!({})).0);
    }));
    for line in equipe_core::disk::clear_avds(2, "eqlive", true) {
        eprintln!("{line}");
    }
    let _ = std::fs::remove_dir_all(&home);
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
