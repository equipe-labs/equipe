//! What a design QA verdict must stand on. A pass took one look at the
//! engineer's screenshots — ten sections in eight languages passed in a
//! minute, "no screenshots of my own, not compared with the mockup" written in
//! the verdict itself. So a pass now needs the designer's own pictures of the
//! built screens, the spec beside them, the conditions it looked at, and no
//! blocker left; and what makes a screen look unfinished — rhythm, alignment,
//! density, hierarchy, the first impression — is never left out: it is
//! polish, written down even on a pass. No runtime types here.
//!
//! Before any designer turn a deterministic precheck looks at the screens
//! (`crate::design::precheck`); [`gate`] reads its report — nothing changed
//! and clean: passed without a designer; a definite S1/S2 failure: back to
//! the engineer; otherwise the designer, with the report. Each squad says
//! whether that gate acts ([`GateMode`]), and on which model tier the
//! designer checks ([`TierPolicy`], [`tier_for`]).

use equipe_protocol::tiers::ModelTier;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::design::precheck::{Report, Severity};

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
                "at least {OWN_SHOTS} pictures of your own of the built screens in `images` (you have {}; the engineer's don't count, the precheck's do) — start its preview from its worktree and shoot it with `equipe design render http://localhost:<port>/<page> --widths 390,768,1280 --dark`",
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

// ── the precheck's gate ──

/// Whether a squad's design QA precheck runs, and whether its gate acts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateMode {
    /// No precheck: every design QA goes to the designer, as before.
    Off,
    /// The precheck runs and its report goes to the designer; what the gate
    /// would have decided is recorded beside the designer's verdict, never acted on.
    #[default]
    Shadow,
    /// The gate acts: passes, sends back, or hands the designer the report.
    On,
}

impl GateMode {
    pub const ALL: [GateMode; 3] = [GateMode::Off, GateMode::Shadow, GateMode::On];

    pub fn code(self) -> &'static str {
        match self {
            GateMode::Off => "off",
            GateMode::Shadow => "shadow",
            GateMode::On => "on",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        GateMode::ALL.into_iter().find(|m| m.code() == text)
    }
}

/// What the gate makes of a precheck's report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// No screen changed and the checks are clean: passed without a designer.
    AutoPass,
    /// A definite S1/S2 failure this change brought: back to its engineer.
    SendBack,
    /// A designer looks, at what the report flags.
    Designer,
}

impl Decision {
    pub fn code(self) -> &'static str {
        match self {
            Decision::AutoPass => "auto_pass",
            Decision::SendBack => "send_back",
            Decision::Designer => "designer",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Decision::AutoPass, Decision::SendBack, Decision::Designer].into_iter().find(|d| d.code() == text)
    }
}

/// Why a designer's judgment is needed beyond the report's: what takes
/// design QA to the top tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Judgment {
    /// The precheck couldn't run, or rendered nothing.
    PrecheckFailed,
    /// UI files changed that no screen of the map lists.
    Unmapped,
    /// Some states couldn't be rendered, or were left out over the cap.
    NotRendered,
    /// Many screens changed at once.
    Broad,
}

impl Judgment {
    pub fn code(self) -> &'static str {
        match self {
            Judgment::PrecheckFailed => "precheck_failed",
            Judgment::Unmapped => "unmapped",
            Judgment::NotRendered => "not_rendered",
            Judgment::Broad => "broad",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Judgment::PrecheckFailed, Judgment::Unmapped, Judgment::NotRendered, Judgment::Broad].into_iter().find(|j| j.code() == text)
    }
}

/// So many changed states make a change broad: a designer's whole look.
pub const BROAD_STATES: usize = 8;

/// The gate's call on a report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateCall {
    pub decision: Decision,
    /// When the designer is needed for more than the report: why.
    pub judgment: Option<Judgment>,
    /// Why, in a line each, in English.
    pub reasons: Vec<String>,
}

impl GateCall {
    /// What it decided, in a sentence: the report's headline.
    pub fn headline(&self, acted: bool) -> String {
        let verb = match (self.decision, acted) {
            (Decision::AutoPass, true) => "Passed without a designer turn",
            (Decision::AutoPass, false) => "The gate would pass this without a designer turn",
            (Decision::SendBack, true) => "Sent back to the engineer without a designer turn",
            (Decision::SendBack, false) => "The gate would send this back to the engineer",
            (Decision::Designer, _) => "A designer checks what this report flags",
        };
        format!("**{verb}.** {}", self.reasons.join(" "))
    }
}

/// What the gate makes of `report` (`None`: the precheck didn't give one).
/// Definite failures this change brought send it back whatever else is
/// unknown; a pass needs everything rendered, nothing unmapped and no
/// picture changed.
pub fn gate(report: Option<&Report>) -> GateCall {
    let designer = |judgment: Option<Judgment>, reason: String| GateCall { decision: Decision::Designer, judgment, reasons: vec![reason] };
    let Some(r) = report else {
        return designer(Some(Judgment::PrecheckFailed), "The precheck gave no report.".into());
    };
    let blockers = r.new_blockers();
    if !blockers.is_empty() {
        let mut rules: Vec<&str> = blockers.iter().map(|f| f.rule.as_str()).collect();
        rules.sort_unstable();
        rules.dedup();
        return GateCall { decision: Decision::SendBack, judgment: None, reasons: vec![format!("{} S1/S2 finding(s) this change brought: {}.", blockers.len(), rules.join(", "))] };
    }
    let taken = r.shots.len() - r.failed();
    if taken == 0 {
        if r.touched.is_empty() && r.unmapped.is_empty() && r.errors.is_empty() {
            return designer(None, "No screen of the UI map lists the files this change touched.".into());
        }
        let why = r.errors.first().cloned().unwrap_or_else(|| "nothing could be rendered".into());
        return designer(Some(Judgment::PrecheckFailed), format!("The precheck rendered nothing: {why}."));
    }
    if !r.unmapped.is_empty() {
        return designer(Some(Judgment::Unmapped), format!("{} changed UI file(s) no screen of the map lists.", r.unmapped.len()));
    }
    if r.failed() > 0 || r.left_out > 0 || !r.errors.is_empty() {
        return designer(Some(Judgment::NotRendered), format!("{} picture(s) failed and {} state(s) were left out.", r.failed(), r.left_out));
    }
    let changed = r.changed();
    if changed.is_empty() {
        let before = r.findings.iter().filter(|f| f.severity == Severity::Blocker).count();
        let also = if before > 0 { format!(" ({before} finding(s) were there before and are unchanged.)") } else { String::new() };
        return GateCall { decision: Decision::AutoPass, judgment: None, reasons: vec![format!("No picture of the {} touched screen(s) changed against the base, and nothing new was found.{also}", r.touched.len())] };
    }
    if changed.len() >= BROAD_STATES {
        return designer(Some(Judgment::Broad), format!("{} states changed.", changed.len()));
    }
    designer(None, format!("{} state(s) changed: {}.", changed.len(), changed.join(", ")))
}

// ── the designer's tier ──

/// Which model tier design QA runs on: `cheap` for the usual check, `top`
/// for a redesign, a second round or more, or a report that needs judgment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TierPolicy {
    pub cheap: ModelTier,
    pub top: ModelTier,
}

impl TierPolicy {
    /// Sonnet-class at medium effort, Opus-class at high when it takes judgment.
    pub const ECONOMY: TierPolicy = TierPolicy { cheap: ModelTier::Standard, top: ModelTier::Deep };
    /// Always the top tier: design QA as it ran before tiering.
    pub const QUALITY: TierPolicy = TierPolicy { cheap: ModelTier::Deep, top: ModelTier::Deep };

    /// `economy`, `quality`, or `<cheap>/<top>` (`standard/deep`).
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "economy" => Some(TierPolicy::ECONOMY),
            "quality" => Some(TierPolicy::QUALITY),
            other => {
                let (cheap, top) = other.split_once('/')?;
                let (cheap, top) = (ModelTier::parse(cheap)?, ModelTier::parse(top)?);
                (cheap <= top).then_some(TierPolicy { cheap, top })
            }
        }
    }

    pub fn code(&self) -> String {
        match *self {
            TierPolicy::ECONOMY => "economy".into(),
            TierPolicy::QUALITY => "quality".into(),
            TierPolicy { cheap, top } => format!("{}/{}", cheap.code(), top.code()),
        }
    }
}

impl Default for TierPolicy {
    fn default() -> Self {
        TierPolicy::ECONOMY
    }
}

/// What a design QA ticket's tier is read from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TierAsk {
    /// Its round: 1 for the first design QA of the ticket it checks.
    pub round: u32,
    /// The ticket it checks redesigns a screen.
    pub redesign: bool,
    /// The precheck left something only a designer's judgment settles.
    pub judgment: bool,
}

/// The tier `ask` runs at under `policy`, and why it is the top one, if it is.
pub fn tier_for(policy: TierPolicy, ask: TierAsk) -> (ModelTier, Option<&'static str>) {
    let why = if ask.redesign {
        Some("redesign")
    } else if ask.round >= 2 {
        Some("round_2_plus")
    } else if ask.judgment {
        Some("needs_judgment")
    } else {
        None
    };
    match why {
        Some(why) if policy.top > policy.cheap => (policy.top, Some(why)),
        _ => (policy.cheap, None),
    }
}

/// Whether a ticket redesigns screens, as its title and body say in English.
pub fn redesign(title: &str, body: &str) -> bool {
    let text = format!("{title}\n{body}").to_lowercase();
    ["redesign", "re-design", "restyle", "visual overhaul", "new look", "visual refresh"].iter().any(|w| text.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::design::precheck::{Diff, Finding, Shot};

    fn shot(key: &str, changed: bool, error: bool) -> Shot {
        Shot {
            key: key.into(),
            condition: "390.light.normal".into(),
            picture: Some(format!("/tmp/{key}.png")),
            diff: Some(Diff { changed, share: if changed { 0.2 } else { 0. }, ..Diff::default() }),
            error: error.then(|| "timed out".into()),
            ..Shot::default()
        }
    }

    fn report(shots: Vec<Shot>, findings: Vec<Finding>) -> Report {
        Report { touched: vec!["auth/login".into()], conditions: vec!["390.light.normal".into()], shots, findings, ..Report::default() }
    }

    #[test]
    fn the_gate_passes_nothing_changed_and_clean_and_sends_back_definite_failures() {
        // Nothing changed, nothing new: passed without a designer, even with old trouble on it.
        let old = Finding::bare("text-clipped", Severity::Blocker, "auth/login/default", "390.light.normal", false);
        let call = gate(Some(&report(vec![shot("auth/login/default", false, false)], vec![old])));
        assert_eq!((call.decision, call.judgment), (Decision::AutoPass, None));
        assert!(call.headline(false).contains("would pass") && call.headline(true).contains("Passed without"));
        // A new S1/S2: back to the engineer — even where other pictures failed.
        let new = Finding::bare("reflow", Severity::Blocker, "auth/login/default", "390.light.normal", true);
        let call = gate(Some(&report(vec![shot("auth/login/default", true, false), shot("auth/login/error", false, true)], vec![new])));
        assert_eq!(call.decision, Decision::SendBack);
        assert!(call.reasons[0].contains("reflow"));
        // New polish only: the designer, on the cheap tier.
        let polish = Finding::bare("overlap", Severity::Polish, "auth/login/default", "390.light.normal", true);
        let call = gate(Some(&report(vec![shot("auth/login/default", true, false)], vec![polish])));
        assert_eq!((call.decision, call.judgment), (Decision::Designer, None));
    }

    #[test]
    fn what_the_gate_cannot_tell_goes_to_a_designer_and_some_of_it_needs_judgment() {
        assert_eq!(gate(None).judgment, Some(Judgment::PrecheckFailed));
        let failed = report(vec![shot("auth/login/default", false, false), shot("auth/login/error", false, true)], vec![]);
        assert_eq!((gate(Some(&failed)).decision, gate(Some(&failed)).judgment), (Decision::Designer, Some(Judgment::NotRendered)), "no pass on half the pictures");
        let unmapped = Report { unmapped: vec!["src/components/Nav.tsx".into()], ..report(vec![shot("auth/login/default", false, false)], vec![]) };
        assert_eq!(gate(Some(&unmapped)).judgment, Some(Judgment::Unmapped));
        let broad = report((0..BROAD_STATES).map(|n| shot(&format!("auth/login/s{n}"), true, false)).collect(), vec![]);
        assert_eq!(gate(Some(&broad)).judgment, Some(Judgment::Broad));
        let nothing = Report { touched: vec![], ..report(vec![], vec![]) };
        assert_eq!((gate(Some(&nothing)).decision, gate(Some(&nothing)).judgment), (Decision::Designer, None), "nothing on the map to render: a plain look");
        let broken = Report { errors: vec!["the preview didn't start".into()], ..report(vec![], vec![]) };
        assert_eq!(gate(Some(&broken)).judgment, Some(Judgment::PrecheckFailed));
        assert!(GateMode::ALL.iter().all(|m| GateMode::parse(m.code()) == Some(*m)));
        assert_eq!(GateMode::default(), GateMode::Shadow, "measured before it acts");
    }

    #[test]
    fn design_qa_runs_cheap_unless_it_takes_judgment() {
        let first = TierAsk { round: 1, ..TierAsk::default() };
        assert_eq!(tier_for(TierPolicy::ECONOMY, first), (ModelTier::Standard, None));
        assert_eq!(tier_for(TierPolicy::ECONOMY, TierAsk { round: 2, ..first }), (ModelTier::Deep, Some("round_2_plus")));
        assert_eq!(tier_for(TierPolicy::ECONOMY, TierAsk { redesign: true, ..first }), (ModelTier::Deep, Some("redesign")));
        assert_eq!(tier_for(TierPolicy::ECONOMY, TierAsk { judgment: true, ..first }), (ModelTier::Deep, Some("needs_judgment")));
        assert_eq!(tier_for(TierPolicy::QUALITY, first), (ModelTier::Deep, None), "quality: the top tier always");
        assert_eq!(TierPolicy::parse("strong/max"), Some(TierPolicy { cheap: ModelTier::Strong, top: ModelTier::Max }));
        assert_eq!(TierPolicy::parse("max/light"), None, "the top is never under the cheap one");
        for p in [TierPolicy::ECONOMY, TierPolicy::QUALITY, TierPolicy { cheap: ModelTier::Light, top: ModelTier::Strong }] {
            assert_eq!(TierPolicy::parse(&p.code()), Some(p));
        }
        assert!(redesign("Redesign the pricing page", "") && !redesign("Fix a typo", "in the footer"));
    }

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
