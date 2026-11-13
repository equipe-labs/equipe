//! What a member never does to the person's Mac while testing: switch a
//! system-wide feature on or off.
//!
//! A QA member once turned on macOS VoiceOver on the person's own Mac to
//! "confirm table navigation". VoiceOver, Zoom, display and keyboard settings
//! belong to the person, not to the work. Members test screen readers in a
//! browser's accessibility tree, in a simulator, or ask the person
//! ([`ALTERNATIVES`]). Shell commands are vetted by `equipe_core::guard`
//! (same wording); this is the desktop tools' side: key chords, launching or
//! quitting the accessibility apps, and acting inside System Settings.
//!
//! Everything here is pure: it answers with the refusal, or `None`.

use crate::keys::{self, COMMAND, CONTROL, Combo, OPTION};

/// What to do instead, in every refusal.
pub const ALTERNATIVES: &str = "Test screen readers without touching this Mac's settings: read the browser's accessibility tree \
(Playwright's ARIA snapshot, axe, Chrome's accessibility snapshot), use VoiceOver or the Accessibility Inspector inside the iOS Simulator, \
or ask the person.";

fn refused(what: &str) -> String {
    format!("Blocked: {what} changes a system-wide accessibility or display setting on the person's Mac, which is theirs, not yours to change while working. {ALTERNATIVES}")
}

const F5: u16 = 96;
const KEY_8: u16 = 28;
const KEY_EQUALS: u16 = 24;
const KEY_MINUS: u16 = 27;
const KEY_COMMA: u16 = 43;
const KEY_PERIOD: u16 = 47;

/// Why one shortcut is refused: ⌘F5 and fn⌘F5 (VoiceOver), ⌥⌘F5 (the
/// Accessibility Shortcuts panel, as a triple-press of Touch ID is), ⌥⌘8 and
/// ⌥⌘= / ⌥⌘- (Zoom), ⌃⌥⌘8 (invert colours), ⌃⌥⌘, and ⌃⌥⌘. (contrast).
pub fn chord(combo: &Combo) -> Option<String> {
    let has = |mask: u64| combo.flags & mask == mask;
    let what = if has(COMMAND) && combo.code == F5 {
        if has(OPTION) { "the Accessibility Shortcuts panel (⌥⌘F5)" } else { "VoiceOver (⌘F5)" }
    } else if has(COMMAND | OPTION) && matches!(combo.code, KEY_8 | KEY_EQUALS | KEY_MINUS) {
        "Zoom (⌥⌘8, ⌥⌘=, ⌥⌘-)"
    } else if has(COMMAND | OPTION | CONTROL) && matches!(combo.code, KEY_COMMA | KEY_PERIOD) {
        "the display contrast shortcut"
    } else {
        return None;
    };
    Some(refused(&format!("that shortcut ({what})")))
}

/// Why any of `keys` (as the `key` tool takes them) is refused. A shortcut
/// the tool can't read isn't this guard's to judge: it fails on its own.
pub fn chords(keys: &str) -> Option<String> {
    keys::parse_all(keys).ok()?.iter().find_map(chord)
}

/// Why opening or closing `query` (a name, a bundle id or a URL) is refused:
/// VoiceOver and its utility, and System Settings' Accessibility panes.
pub fn app(query: &str) -> Option<String> {
    let q = query.to_lowercase();
    if q.contains("voiceover") || q.contains("voice over") || q.contains("com.apple.scrod") {
        return Some(refused("starting or stopping VoiceOver"));
    }
    if q.contains("universalaccess") || (q.contains("x-apple.systempreferences") && q.contains("accessibility")) || q.contains("settings.extension.accessibility") {
        return Some(refused("opening System Settings at Accessibility"));
    }
    None
}

/// Whether `name` / `bundle_id` is System Settings (or its older name) or
/// VoiceOver Utility: where a member only reads, never clicks or types.
pub fn is_system_settings(name: &str, bundle_id: Option<&str>) -> bool {
    let n = name.to_lowercase();
    let b = bundle_id.unwrap_or_default().to_lowercase();
    b == "com.apple.systempreferences" || b.starts_with("com.apple.voiceover") || n == "system settings" || n == "system preferences" || n.contains("voiceover")
}

/// Why acting (a click, typing, a ref action) in `name` is refused: System
/// Settings is the person's. Reading it (a snapshot, a screenshot) is fine.
pub fn acting_on(name: &str, bundle_id: Option<&str>) -> Option<String> {
    is_system_settings(name, bundle_id).then(|| refused(&format!("acting in {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(keys: &str) -> Option<String> {
        chords(keys)
    }

    #[test]
    fn the_chords_that_toggle_system_features_are_refused_with_the_alternatives() {
        for keys in ["cmd+f5", "fn+cmd+f5", "Cmd+F5", "option+cmd+f5", "alt+cmd+8", "option+cmd+=", "option+cmd+-", "ctrl+option+cmd+8", "ctrl+option+cmd+,", "cmd+s cmd+f5"] {
            let why = refusal(keys).unwrap_or_else(|| panic!("{keys} should be refused"));
            assert!(why.starts_with("Blocked:") && why.contains("accessibility tree") && why.contains("Simulator") && why.contains("ask the person"), "{keys}: {why}");
        }
    }

    #[test]
    fn ordinary_shortcuts_still_go_through() {
        for keys in ["cmd+s", "return", "cmd+shift+t", "cmd+8", "option+8", "f5", "cmd+a delete", "ctrl+option+left", "cmd+=", "banana+x"] {
            assert_eq!(refusal(keys), None, "{keys}");
        }
    }

    #[test]
    fn the_accessibility_apps_and_panes_are_not_opened_by_a_member() {
        for query in ["VoiceOver", "voiceover.app", "VoiceOver Utility", "com.apple.VoiceOverUtility", "x-apple.systempreferences:com.apple.preference.universalaccess", "x-apple.systempreferences:com.apple.Accessibility-Settings.extension"] {
            assert!(app(query).is_some(), "{query}");
        }
        for query in ["Safari", "TextEdit", "Accessibility Inspector", "com.apple.AccessibilityInspector", "System Settings", "Simulator"] {
            assert_eq!(app(query), None, "{query}");
        }
    }

    #[test]
    fn system_settings_is_read_never_acted_in() {
        assert!(acting_on("System Settings", Some("com.apple.systempreferences")).is_some());
        assert!(acting_on("VoiceOver Utility", None).is_some());
        assert_eq!(acting_on("Safari", Some("com.apple.Safari")), None);
        assert_eq!(acting_on("Accessibility Inspector", Some("com.apple.AccessibilityInspector")), None);
    }
}
