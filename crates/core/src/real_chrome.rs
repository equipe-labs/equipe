//! A member working in the person's own Chrome (docs/REAL-CHROME.md) that
//! needs the person — a sign-in, a human check — points them at its own
//! window there, not at an automated browser.
//!
//! The window is already the person's real browser: their profile, their
//! sign-ins, nothing automated in it. So there is nothing to hand over or
//! copy back: the member puts its ticket down for the person
//! (`block_on_person`), naming the window, and the person does the check in
//! it themselves. Only when the member has such a window open and the person
//! turned real Chrome on for the squad; otherwise `show_my_browser` works as
//! it always did.

use std::path::Path;

use equipe_desktop_control::{chrome, helper};
use equipe_protocol::rpc::{ToolCall, ToolResult};
use serde_json::{Value, json};

/// The answer to a member's `show_my_browser` when it has a window of the
/// person's own Chrome for that page: where the person goes instead. `None`
/// when it doesn't (or real Chrome is off, or the helper isn't running):
/// the usual hand-off goes on.
pub async fn redirect(home: &Path, call: &ToolCall) -> Option<ToolResult> {
    if call.name != crate::board::SHOW_BROWSER || !chrome::enabled(home, &call.squad) {
        return None;
    }
    let member = format!("{}/{}", call.squad, call.member);
    let found = helper::call(home, &member, chrome::WINDOW, &json!({ "url": call.arguments.get("url") })).await;
    if found.is_error {
        return None;
    }
    let window: Value = serde_json::from_str(&found.text).ok()?;
    Some(ToolResult::ok(pointer(&window)?))
}

/// What the member is told: the person does it in its window.
pub fn pointer(window: &Value) -> Option<String> {
    let id = window["id"].as_str()?;
    let profile = window["profile"].as_str().unwrap_or("?");
    let title = window["title"].as_str().filter(|t| !t.trim().is_empty());
    let named = match title {
        Some(title) => format!("the Chrome window {title:?} ({id}, profile {profile:?})"),
        None => format!("the Chrome window {id} (profile {profile:?})"),
    };
    Some(format!(
        "That page is in your own window of the person's Chrome — {named} — their real browser, with nothing automated in it. \
The person signs in or passes the check there themselves; nothing is copied anywhere. Call block_on_person now with one line \
of what they should do in that window (name its title and profile). When they unblock you, chrome_snapshot the window and carry on. \
Never click or type into a human check yourself."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_person_is_pointed_at_the_members_own_window() {
        let window = json!({ "id": "w-900", "profile": "Work", "dir": "Profile 2", "site": "github.com", "title": "Sign in to GitHub · GitHub" });
        let text = pointer(&window).unwrap();
        assert!(text.contains("\"Sign in to GitHub · GitHub\" (w-900, profile \"Work\")"), "{text}");
        assert!(text.contains("block_on_person") && text.contains("chrome_snapshot"), "{text}");
        let untitled = pointer(&json!({ "id": "w-7", "profile": "Personal", "title": "" })).unwrap();
        assert!(untitled.contains("the Chrome window w-7 (profile \"Personal\")"), "{untitled}");
        assert_eq!(pointer(&json!({})), None);
    }

    #[tokio::test]
    async fn without_real_chrome_the_usual_hand_off_goes_on() {
        let home = std::env::temp_dir().join(format!("eq-real-chrome-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let call = ToolCall { squad: "acme".into(), member: "frontend".into(), name: crate::board::SHOW_BROWSER.into(), arguments: json!({ "url": "https://example.com" }), run: None };
        assert!(redirect(&home, &call).await.is_none(), "off for the squad");
        chrome::set_enabled(&home, "acme", true).unwrap();
        assert!(redirect(&home, &call).await.is_none(), "on, but no helper (so no window) to point at");
        let other = ToolCall { name: crate::board::SIGN_IN.into(), ..call };
        assert!(redirect(&home, &other).await.is_none(), "only show_my_browser");
        let _ = std::fs::remove_dir_all(&home);
    }
}
