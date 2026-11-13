//! What members working the desktop need on this machine, and what it has —
//! for the person's settings: the desktop helper and its permissions, the
//! accessibility engine, the session. A guide on request: macOS's prompts for
//! the helper, and System Settings open where the switch is.

use std::path::Path;

use equipe_desktop_control::engine::pin;
use equipe_desktop_control::helper;
use equipe_protocol::DesktopReadiness;
use serde_json::Value;

/// System Settings, at the Accessibility list.
pub const ACCESSIBILITY_PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
/// System Settings, at the Screen Recording list.
pub const SCREEN_PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

/// Look at the desktop of the machine equipe's `home` is on. `guide`: also
/// ask the helper to request its permissions, and open System Settings at
/// what is still missing.
pub async fn readiness(home: &Path, guide: bool) -> DesktopReadiness {
    let mut ready = DesktopReadiness {
        supported: cfg!(any(target_os = "macos", windows, target_os = "linux")),
        engine_version: pin::VERSION.to_owned(),
        work_locked: super::work_locked(home),
        ..Default::default()
    };
    if !ready.supported {
        return ready;
    }
    ready.helper_name = helper_name(None);
    let home_owned = home.to_owned();
    ready.engine = tokio::task::spawn_blocking(move || matches!(pin::check(&home_owned, pin::this_machine()), pin::Installed::Ready(_))).await.unwrap_or(false);
    let answer = helper::call(home, "settings", if guide { helper::REQUEST } else { helper::STATUS }, &Value::Null).await;
    if !answer.is_error
        && let Ok(status) = serde_json::from_str::<Value>(&answer.text)
    {
        apply(&mut ready, &status);
    }
    // No helper to say it: what the session lacks, as far as can be told from here.
    #[cfg(target_os = "linux")]
    if !ready.helper {
        ready.missing = tokio::task::spawn_blocking(|| equipe_desktop_control::linux::check().missing()).await.unwrap_or_default();
    }
    if guide && cfg!(target_os = "macos") && ready.helper {
        if ready.accessibility != Some(true) {
            let _ = equipe_providers::platform::open_status(ACCESSIBILITY_PANE);
        } else if ready.screen != Some(true) {
            let _ = equipe_providers::platform::open_status(SCREEN_PANE);
        }
    }
    ready
}

/// What the helper's `_status` says, into `ready`.
pub(super) fn apply(ready: &mut DesktopReadiness, status: &Value) {
    ready.helper = true;
    ready.accessibility = status["accessibility"].as_bool().map(|ax| ax && status["post_events"].as_bool().unwrap_or(true));
    ready.screen = status["screen"].as_bool();
    ready.engine_grant = status["engine"]["accessibility"].as_str().map(|state| state == "granted");
    ready.locked = status["session"]["locked"].as_bool() == Some(true);
    ready.helper_name = helper_name(Some(status));
    ready.missing = status["missing"].as_array().map(|lines| lines.iter().filter_map(|l| l.as_str().map(str::to_owned)).collect()).unwrap_or_default();
}

/// What System Settings lists the helper as: "equipe" when equipe.app (or the
/// bundle named after it) holds the permission, else the older bundle's name.
pub fn helper_name(status: Option<&Value>) -> String {
    use equipe_desktop_control::install::{self, Host};
    match status.and_then(|s| s["role"].as_str()) {
        Some("app") => install::DISPLAY_NAME.to_owned(),
        Some("bundle") => Host::Bundle.name().to_owned(),
        _ if cfg!(target_os = "macos") => install::host().name().to_owned(),
        _ => install::DISPLAY_NAME.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_helpers_status_reads_as_what_the_person_sees() {
        let mut ready = DesktopReadiness { supported: true, engine: true, ..Default::default() };
        apply(
            &mut ready,
            &json!({ "screen": false, "accessibility": true, "post_events": true, "session": { "locked": false }, "engine": { "accessibility": "granted" } }),
        );
        assert_eq!((ready.helper, ready.accessibility, ready.screen, ready.engine_grant), (true, Some(true), Some(false), Some(true)));
        apply(&mut ready, &json!({ "accessibility": true, "post_events": true, "role": "app", "engine": { "accessibility": "granted" } }));
        assert_eq!(ready.helper_name, "equipe", "equipe.app holds it: one entry, called equipe");
        assert!(ready.ready(), "Screen Recording is only for screenshots");
        apply(&mut ready, &json!({ "accessibility": true, "post_events": false, "engine": { "accessibility": "denied" } }));
        assert_eq!(ready.accessibility, Some(false));
        assert!(!ready.ready());
        assert!(ready.missing.is_empty());
    }

    #[test]
    fn a_linux_helpers_status_says_what_the_session_lacks() {
        let mut ready = DesktopReadiness { supported: true, engine: true, ..Default::default() };
        let missing = vec!["AT-SPI (the desktop's accessibility bus) isn't answering".to_owned()];
        apply(&mut ready, &json!({ "role": "linux", "screen": null, "accessibility": false, "post_events": false, "missing": missing }));
        assert_eq!((ready.helper, ready.accessibility, ready.screen), (true, Some(false), None));
        assert_eq!(ready.missing, missing);
        assert_eq!(ready.helper_name, "equipe");
        assert!(!ready.ready());
        apply(&mut ready, &json!({ "role": "linux", "screen": null, "accessibility": true, "post_events": true, "missing": [], "engine": { "accessibility": "granted" } }));
        assert!(ready.missing.is_empty() && ready.ready(), "AT-SPI answering is all Linux asks; no picture needed");
    }

    #[tokio::test]
    async fn without_a_helper_nothing_is_ready_and_nothing_is_asked() {
        let home = std::env::temp_dir().join(format!("eq-ready-{}", std::process::id()));
        let ready = readiness(&home, false).await;
        assert!(!ready.helper && !ready.engine && !ready.ready());
        assert_eq!(ready.engine_version, pin::VERSION);
        let _ = std::fs::remove_dir_all(&home);
    }
}
