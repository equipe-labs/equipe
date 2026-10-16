//! What changed in the tokens since the person last looked: the app keeps
//! what each token was when they last looked (`Seen`), and lists what was
//! added, taken away or given another value since.

use std::collections::BTreeMap;

use equipe_protocol::design::Token;
use serde_json::{Value, json};

use super::values::describe;

/// What each token was, by name: its type, value and modes, as JSON.
pub type Seen = BTreeMap<String, Value>;

/// The tokens as they are now, to remember as seen.
pub fn seen(tokens: &[Token]) -> Seen {
    tokens.iter().map(|t| (t.name.clone(), json!({ "type": t.kind, "value": t.value, "modes": t.modes }))).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added { name: String, now: String },
    Removed { name: String, was: String },
    /// `was` and `now` said as a line each (`#1D4ED8`), in the mode that changed.
    Changed { name: String, was: String, now: String },
}

impl Change {
    pub fn name(&self) -> &str {
        match self {
            Change::Added { name, .. } | Change::Removed { name, .. } | Change::Changed { name, .. } => name,
        }
    }
}

/// A seen token, said in a line: its value in `mode` (else its default).
fn said(seen: &Value, mode: Option<&str>) -> String {
    let kind = seen["type"].as_str().unwrap_or_default();
    let value = mode.and_then(|m| seen["modes"].get(m)).unwrap_or(&seen["value"]);
    describe(kind, value)
}

/// What differs between what was seen and `now`, in `now`'s order (what was
/// taken away last). Said in the first mode that changed.
pub fn diff(before: &Seen, now: &[Token]) -> Vec<Change> {
    let after = seen(now);
    let mut out = Vec::new();
    for token in now {
        let new = &after[&token.name];
        match before.get(&token.name) {
            None => out.push(Change::Added { name: token.name.clone(), now: said(new, None) }),
            // The same value written another way (`8px`, `{ value: 8, unit: px }`) is no change.
            Some(old) if old != new => {
                let mut modes: Vec<Option<&str>> = vec![None];
                modes.extend(token.modes.keys().map(|m| Some(m.as_str())));
                modes.extend(old["modes"].as_object().into_iter().flat_map(|m| m.keys().map(|k| Some(k.as_str()))));
                if let Some(mode) = modes.into_iter().find(|m| said(old, *m) != said(new, *m)) {
                    out.push(Change::Changed { name: token.name.clone(), was: said(old, mode), now: said(new, mode) });
                }
            }
            Some(_) => {}
        }
    }
    for (name, old) in before {
        if !after.contains_key(name) {
            out.push(Change::Removed { name: name.clone(), was: said(old, None) });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(name: &str, kind: &str, value: Value) -> Token {
        Token { name: name.into(), kind: kind.into(), value, modes: BTreeMap::new(), description: None, alias: None }
    }

    #[test]
    fn added_removed_and_changed_each_said_with_its_values() {
        let before = seen(&[t("color.accent", "color", json!("#2E5C8A")), t("space.sm", "dimension", json!("8px")), t("color.old", "color", json!("#FF0000")), t("radius.md", "dimension", json!("8px"))]);
        let now = [t("color.accent", "color", json!("#1d4ed8")), t("space.sm", "dimension", json!("8px")), t("radius.md", "dimension", json!({ "value": 8, "unit": "px" })), t("color.dot", "color", json!("#3F6FF5"))];
        let changes = diff(&before, &now);
        assert_eq!(
            changes,
            [
                Change::Changed { name: "color.accent".into(), was: "#2E5C8A".into(), now: "#1D4ED8".into() },
                Change::Added { name: "color.dot".into(), now: "#3F6FF5".into() },
                Change::Removed { name: "color.old".into(), was: "#FF0000".into() },
            ]
        );
        assert!(diff(&seen(&now), &now).is_empty(), "nothing since");
        assert_eq!(diff(&Seen::new(), &now).len(), 4, "never looked: all of it is new");
        assert!(!changes.iter().any(|c| c.name() == "radius.md"), "8px written another way is still 8px");
    }

    #[test]
    fn a_change_in_one_mode_is_said_in_that_mode() {
        let mut bg = t("color.bg", "color", json!("#F6F5F1"));
        bg.modes = [("light".to_owned(), json!("#F6F5F1")), ("dark".to_owned(), json!("#121211"))].into();
        let before = seen(std::slice::from_ref(&bg));
        bg.modes.insert("dark".into(), json!("#0A0A0C"));
        assert_eq!(diff(&before, &[bg]), [Change::Changed { name: "color.bg".into(), was: "#121211".into(), now: "#0A0A0C".into() }]);
    }
}
