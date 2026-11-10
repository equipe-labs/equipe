//! What a design QA verdict must stand on. A pass took one look at the
//! engineer's screenshots — ten sections in eight languages passed in a
//! minute, "no screenshots of my own, not compared with the mockup" written in
//! the verdict itself. So a pass now needs the designer's own pictures of the
//! built screens, the spec beside them, the conditions it looked at, and no
//! blocker left; and what makes a screen look unfinished — rhythm, alignment,
//! density, hierarchy, the first impression — is never left out: it is
//! polish, written down even on a pass. No runtime types here.

use serde_json::Value;

/// The designer's own pictures of the built screens a pass needs at least.
pub const OWN_SHOTS: usize = 2;
/// The conditions it must have looked at: widths, light and dark, states.
pub const CONDITIONS: usize = 3;

/// What a `design_qa` call says it stands on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Proof {
    /// Its own pictures: those it gave, not the engineer's.
    pub own_shots: usize,
    /// The built screens put beside the spec or mockup they were built from.
    pub compared: bool,
    /// What it looked at: "390", "1280", "dark", "hover", "empty state"…
    pub checked: Vec<String>,
    /// Differences from the spec, the tokens or accessibility: back to the engineer.
    pub blockers: Vec<String>,
    /// What keeps it from looking finished: noted, never blocking.
    pub polish: Vec<String>,
}

impl Proof {
    /// Read from `design_qa`'s arguments; `own_shots` is the runtime's count.
    pub fn read(args: &Value, own_shots: usize) -> Proof {
        let list = |key: &str| args.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()).unwrap_or_default();
        Proof { own_shots, compared: args.get("compared").and_then(Value::as_bool).unwrap_or(false), checked: list("checked"), blockers: list("blockers"), polish: list("polish") }
    }

    /// Why a pass can't stand on this, if it can't — said to the designer.
    pub fn refuses_pass(&self) -> Option<String> {
        let mut missing = Vec::new();
        if self.own_shots < OWN_SHOTS {
            missing.push(format!(
                "at least {OWN_SHOTS} pictures of your own of the built screens in `images` (you have {}; the engineer's don't count) — start its preview from its worktree and shoot it with `equipe design render http://localhost:<port>/<page> --widths 390,768,1280 --dark`",
                self.own_shots
            ));
        }
        if !self.compared {
            missing.push("the built screens put beside the spec or mockup they were built from (`compared: true` once you have)".to_owned());
        }
        if self.checked.len() < CONDITIONS {
            missing.push(format!("at least {CONDITIONS} conditions you looked at in `checked` — widths, light and dark, states (empty, loading, error, long text, hover, focus)"));
        }
        if !self.blockers.is_empty() {
            missing.push("no blocker left — a blocker sends it back: verdict changes".to_owned());
        }
        (!missing.is_empty()).then(|| format!("A pass needs {}. Look at it yourself, then finish again.", missing.join("; ")))
    }

    /// Why a send-back can't stand on this, if it can't.
    pub fn refuses_changes(&self) -> Option<String> {
        self.blockers.is_empty().then(|| "Changes needs what to change: each in `blockers` — where, the spec's value, what was built. What only keeps it from looking finished is `polish`, which doesn't send it back.".to_owned())
    }

    /// The verdict as the engineer and the person read it: the summary, then
    /// each blocker and each polish point numbered, then what was looked at.
    pub fn report(&self, summary: &str) -> String {
        let mut out = vec![summary.trim().to_owned()];
        let numbered = |head: &str, items: &[String]| format!("{head}\n{}", items.iter().enumerate().map(|(i, s)| format!("{}. {s}", i + 1)).collect::<Vec<_>>().join("\n"));
        if !self.blockers.is_empty() {
            out.push(numbered("Blockers:", &self.blockers));
        }
        if !self.polish.is_empty() {
            out.push(numbered("Polish:", &self.polish));
        }
        if !self.checked.is_empty() {
            out.push(format!("Checked: {}.", self.checked.join(", ")));
        }
        out.retain(|s| !s.is_empty());
        out.join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn proof(args: Value, own: usize) -> Proof {
        Proof::read(&args, own)
    }

    #[test]
    fn a_pass_stands_only_on_the_designers_own_look_beside_the_spec() {
        let full = json!({ "compared": true, "checked": ["390", "1280", "dark", "empty state"], "polish": ["Hero padding uneven: 40 left, 48 right"] });
        assert_eq!(proof(full.clone(), 2).refuses_pass(), None);
        let said = proof(full.clone(), 0).refuses_pass().unwrap();
        assert!(said.contains("pictures of your own") && said.contains("equipe design render http://localhost"), "{said}");
        assert!(proof(json!({ "checked": ["390", "1280", "dark"] }), 3).refuses_pass().unwrap().contains("beside the spec"), "not compared");
        assert!(proof(json!({ "compared": true, "checked": ["1280"] }), 3).refuses_pass().unwrap().contains("conditions"), "one width alone");
        let blocked = json!({ "compared": true, "checked": ["390", "1280", "dark"], "blockers": ["Title 20px, spec 24px"] });
        assert!(proof(blocked, 3).refuses_pass().unwrap().contains("verdict changes"));
    }

    #[test]
    fn a_send_back_names_its_blockers_and_polish_never_sends_back_but_is_always_said() {
        assert!(proof(json!({ "polish": ["a bit cramped"] }), 0).refuses_changes().is_some(), "polish alone isn't a send-back");
        assert_eq!(proof(json!({ "blockers": ["Title 20px, spec 24px"] }), 0).refuses_changes(), None);
        let p = proof(json!({ "compared": true, "checked": ["390", "1280", "dark"], "blockers": ["Title 20px, spec 24px"], "polish": ["Cards feel cramped at 390: 12px gaps, the system's step is 16"] }), 2);
        let report = p.report("Built close to the spec.");
        assert!(report.starts_with("Built close to the spec."));
        assert!(report.contains("Blockers:\n1. Title 20px, spec 24px") && report.contains("Polish:\n1. Cards feel cramped"), "{report}");
        assert!(report.ends_with("Checked: 390, 1280, dark."), "{report}");
    }
}
