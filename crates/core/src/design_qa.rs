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
//! designer checks ([`TierPolicy`], [`tier_for`]). In `risk_batch` mode
//! ([`Mode`]) only high-risk changes get a designer turn of their own; the
//! rest go into a daily review ([`route`]).

use equipe_protocol::tiers::ModelTier;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::design::precheck::{Report, Severity};

pub mod follow_up;
pub mod reference;
mod risk;
pub use risk::*;

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
    /// Built from the person's own design: a "same feel?" verdict per screen, side by side with it.
    pub side_by_side: Vec<reference::Feel>,
    /// The reference's signature items seen in the build (their ids).
    pub signature: Vec<String>,
    /// File names of its own pictures (in `images`, not the engineer's).
    pub own_pictures: Vec<String>,
    /// "Not confirmed", "didn't render"… in its own words.
    pub hedged: Vec<String>,
}

impl Proof {
    /// Read from `design_qa`'s arguments; `own_shots` is the runtime's count.
    /// Lists written as strings count as lists (`checked` as `"390, 1280,
    /// dark"` too — they are labels), and `compared` as the text `"true"`.
    pub fn read(args: &Value, own_shots: usize) -> Proof {
        use crate::board::{arg_bool, arg_labels, arg_strs};
        Proof {
            own_shots,
            compared: arg_bool(args, "compared").unwrap_or(false),
            checked: arg_labels(args, "checked"),
            blockers: arg_strs(args, "blockers"),
            polish: arg_strs(args, "polish"),
            side_by_side: reference::read_feel(args),
            signature: arg_labels(args, "signature"),
            own_pictures: Vec::new(),
            hedged: reference::hedges(args),
        }
    }

    /// With the paths of its own pictures, which the runtime resolved.
    pub fn with_own_pictures(mut self, pictures: impl IntoIterator<Item = String>) -> Proof {
        self.own_pictures = pictures.into_iter().collect();
        self
    }

    /// Why a pass can't stand on this, if it can't — said to the designer.
    pub fn refuses_pass(&self) -> Option<String> {
        self.refuses_pass_against(None)
    }

    /// [`Proof::refuses_pass`], and for a product built from the person's
    /// own design, the side-by-side comparison with it and its signature items.
    pub fn refuses_pass_against(&self, reference: Option<&reference::Reference>) -> Option<String> {
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
        if let Some(r) = reference {
            missing.extend(reference::refuses(r, &self.side_by_side, &self.signature, &self.own_pictures, &self.hedged));
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
        if !self.side_by_side.is_empty() {
            let rows = self.side_by_side.iter().map(|f| format!("- {}: {}{}", f.screen, if f.same { "same feel" } else { "not the same feel" }, if f.why.is_empty() { String::new() } else { format!(" ({})", f.why) })).collect::<Vec<_>>().join("\n");
            out.push(format!("Side by side with the person's design:\n{rows}"));
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

/// Why a designer's judgment is needed beyond the report's. Some reasons
/// take design QA to the top tier ([`Judgment::needs_top`]); the rest leave
/// the designer on the cheap tier, with the report, for what wasn't seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Judgment {
    /// The precheck couldn't run, or its setup is broken.
    PrecheckFailed,
    /// Nothing, or most of what the change touches, could be rendered.
    NotRendered,
    /// A few pictures failed; the rest were rendered.
    Partial,
    /// Touched states were left out over the cap or the time budget; the rest were rendered.
    Capped,
    /// A few changed UI files no screen of the map lists; the rest rendered.
    Unmapped,
    /// Many changed UI files no screen of the map lists.
    UnmappedLarge,
    /// Many screens changed at once.
    Broad,
    /// Built from the person's own design: only a side-by-side look with it can pass it.
    Reference,
}

impl Judgment {
    pub const ALL: [Judgment; 8] = [Judgment::PrecheckFailed, Judgment::NotRendered, Judgment::Partial, Judgment::Capped, Judgment::Unmapped, Judgment::UnmappedLarge, Judgment::Broad, Judgment::Reference];

    pub fn code(self) -> &'static str {
        match self {
            Judgment::PrecheckFailed => "precheck_failed",
            Judgment::NotRendered => "not_rendered",
            Judgment::Partial => "partial",
            Judgment::Capped => "capped",
            Judgment::Unmapped => "unmapped",
            Judgment::UnmappedLarge => "unmapped_large",
            Judgment::Broad => "broad",
            Judgment::Reference => "reference",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Judgment::ALL.into_iter().find(|j| j.code() == text)
    }

    /// Whether this takes the designer to the top tier: the report leaves
    /// too little seen (or too much changed) for a cheaper look.
    pub fn needs_top(self) -> bool {
        matches!(self, Judgment::PrecheckFailed | Judgment::NotRendered | Judgment::UnmappedLarge | Judgment::Broad | Judgment::Reference)
    }

    /// Whether a journal's `judgment` text (none, or one this version
    /// doesn't know) takes the top tier: an unknown one does.
    pub fn text_needs_top(text: &str) -> bool {
        Judgment::parse(text).is_none_or(Judgment::needs_top)
    }
}

/// So many changed screens make a change broad: a designer's whole look.
/// Screens, not states: with the higher state cap one screen with many
/// states would otherwise look broad by itself.
pub const BROAD_SCREENS: usize = 6;

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
/// picture changed — states left out, over the cap or the time budget,
/// never pass, but they only need the designer on the cheap tier.
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
    let mut notes: Vec<(Judgment, String)> = Vec::new();
    if taken == 0 {
        if !r.errors.is_empty() {
            let why = r.errors.iter().take(3).cloned().collect::<Vec<_>>().join("; ");
            return designer(Some(Judgment::PrecheckFailed), format!("The precheck rendered nothing: {why}."));
        }
        if !r.touched.is_empty() {
            return designer(Some(Judgment::NotRendered), "The precheck rendered nothing: no picture of the touched screens could be taken.".into());
        }
        if r.unmapped.is_empty() {
            return designer(None, "No screen of the UI map lists the files this change touched.".into());
        }
    }
    if !r.unmapped.is_empty() {
        let large = r.unmapped.len() >= r.unmapped_large_at();
        let kind = if large { Judgment::UnmappedLarge } else { Judgment::Unmapped };
        notes.push((kind, format!("{} changed UI file(s) no screen of the map lists.", r.unmapped.len())));
    }
    if r.failed() > 0 {
        // Most pictures failed: little was seen; a few: the designer renders those.
        let kind = if r.failed() * 2 >= r.shots.len() { Judgment::NotRendered } else { Judgment::Partial };
        let why = r.failure_causes(1).into_iter().next().map(|(why, _)| format!(" Most common cause: {why}")).unwrap_or_default();
        notes.push((kind, format!("{} picture(s) failed.{why}", r.failed())));
    }
    if r.left_out > 0 {
        notes.push((Judgment::Capped, format!("{} touched state(s) were left out over the cap and are not covered by the report.", r.left_out)));
    }
    if !r.errors.is_empty() && taken > 0 {
        let why = r.errors.iter().take(3).cloned().collect::<Vec<_>>().join("; ");
        notes.push((Judgment::Partial, format!("The precheck reported problems: {why}.")));
    }
    let changed = r.changed();
    let screens = r.changed_screens();
    if screens.len() >= BROAD_SCREENS {
        notes.push((Judgment::Broad, format!("{} screens changed ({} states).", screens.len(), changed.len())));
    }
    if !notes.is_empty() {
        let judgment = notes.iter().map(|(j, _)| *j).find(|j| j.needs_top()).or_else(|| notes.first().map(|(j, _)| *j));
        return GateCall { decision: Decision::Designer, judgment, reasons: notes.into_iter().map(|(_, why)| why).collect() };
    }
    if changed.is_empty() {
        let before = r.findings.iter().filter(|f| f.severity == Severity::Blocker).count();
        let also = if before > 0 { format!(" ({before} finding(s) were there before and are unchanged.)") } else { String::new() };
        return GateCall { decision: Decision::AutoPass, judgment: None, reasons: vec![format!("No picture of the {} touched screen(s) changed against the base, and nothing new was found.{also}", r.touched.len())] };
    }
    designer(None, format!("{} state(s) changed: {}.", changed.len(), changed.join(", ")))
}

/// What the gate makes of the precheck of a ticket built from a live
/// prototype the person picked with "Use this"
/// (`crate::design::session::APPROVED_PROTOTYPE`): they saw, touched and
/// chose the design already, so a designer doesn't look again — the
/// precheck alone passes it when the built screens rendered at the widths it
/// checks and brought no S1/S2 finding. Pictures differing from the base are
/// expected here (the screen is new). Only a precheck that failed, saw too
/// little, or found S1/S2 problems hands it to the designer, with the report.
pub fn gate_prototype(report: Option<&Report>) -> GateCall {
    gate_prototype_against(report, None)
}

/// [`gate_prototype`] for a build that has the person's own design to match
/// ([`reference::Reference`]): render checks never pass it alone. A clean
/// precheck still hands it to the designer, who puts it side by side with
/// that design (a definite S1/S2 still goes back to its engineer first).
pub fn gate_prototype_against(report: Option<&Report>, reference: Option<&reference::Reference>) -> GateCall {
    let call = gate_prototype_only(report);
    against(call, reference)
}

/// Any gate's call for a build with the person's own design to match: a
/// pass becomes the designer's side-by-side look.
pub fn against(call: GateCall, reference: Option<&reference::Reference>) -> GateCall {
    match (call.decision, reference) {
        (Decision::AutoPass, Some(r)) => GateCall {
            decision: Decision::Designer,
            judgment: Some(Judgment::Reference),
            reasons: vec![format!(
                "Built from the person's own design ({}): the precheck rendered it cleanly, but only a side-by-side look with that design can pass it.",
                if r.source.is_empty() { "their prototype" } else { r.source.as_str() }
            )],
        },
        _ => call,
    }
}

fn gate_prototype_only(report: Option<&Report>) -> GateCall {
    let designer = |judgment: Option<Judgment>, reason: String| GateCall { decision: Decision::Designer, judgment, reasons: vec![reason] };
    let Some(r) = report else {
        return designer(Some(Judgment::PrecheckFailed), "The precheck gave no report.".into());
    };
    let blockers = r.new_blockers();
    if !blockers.is_empty() {
        let mut rules: Vec<&str> = blockers.iter().map(|f| f.rule.as_str()).collect();
        rules.sort_unstable();
        rules.dedup();
        return designer(None, format!("{} S1/S2 finding(s) on the screens built from the picked prototype: {}.", blockers.len(), rules.join(", ")));
    }
    let taken = r.shots.len() - r.failed();
    if taken == 0 {
        return designer(Some(if r.errors.is_empty() { Judgment::NotRendered } else { Judgment::PrecheckFailed }), "The precheck rendered none of the built screens.".into());
    }
    if r.failed() * 2 >= r.shots.len() {
        return designer(Some(Judgment::NotRendered), format!("{} of {} picture(s) of the built screens failed.", r.failed(), r.shots.len()));
    }
    let conditions = if r.conditions.is_empty() { String::new() } else { format!(" at {}", r.conditions.join(", ")) };
    GateCall {
        decision: Decision::AutoPass,
        judgment: None,
        reasons: vec![format!("Built from the prototype the person picked: {taken} picture(s) of the built screens rendered{conditions}, and nothing new at S1/S2.")],
    }
}

// ── the designer's tier ──

/// Which model tier design QA runs on: `cheap` for the usual check, `top`
/// for a redesign, a third round or more, or a report that needs judgment.
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
    } else if ask.round >= 3 {
        Some("round_3_plus")
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
    fn a_build_from_the_persons_own_design_never_passes_on_render_checks_alone() {
        let built = report(vec![shot("auth/login/default", true, false)], vec![]);
        let r = reference::Reference { source: "prototype/index.html".into(), ..Default::default() };
        assert_eq!(gate_prototype(Some(&built)).decision, Decision::AutoPass);
        let call = gate_prototype_against(Some(&built), Some(&r));
        assert_eq!((call.decision, call.judgment), (Decision::Designer, Some(Judgment::Reference)));
        assert!(call.reasons[0].contains("side-by-side") && Judgment::Reference.needs_top());
        // A definite S1/S2 still goes back to its engineer first; the plain gate is held the same way.
        let new = Finding::bare("reflow", Severity::Blocker, "auth/login/default", "390.light.normal", true);
        assert_eq!(against(gate(Some(&report(vec![shot("auth/login/default", true, false)], vec![new]))), Some(&r)).decision, Decision::SendBack);
        assert_eq!(against(gate(Some(&report(vec![shot("auth/login/default", false, false)], vec![]))), Some(&r)).decision, Decision::Designer);
        // The designer's pass needs the side-by-side verdicts and the signature.
        let r = reference::Reference { source: "prototype/index.html".into(), screens: vec!["lock".into()], items: vec![reference::Item { id: "orb".into(), name: "Orb".into() }] };
        let base = json!({ "compared": true, "checked": ["390", "1280", "dark"] });
        assert!(Proof::read(&base, 2).refuses_pass_against(Some(&r)).unwrap().contains("side by side"));
        let full = json!({ "compared": true, "checked": ["390", "1280", "dark"], "side_by_side": [{"screen": "lock", "same_feel": true, "pictures": ["handin-shots/lock-side.png"]}], "signature": ["orb"] });
        assert!(Proof::read(&full, 2).refuses_pass_against(Some(&r)).unwrap().contains("picture of your own"));
        let own = || Proof::read(&full, 2).with_own_pictures(["/w/handin-shots/lock-side.png".to_owned(), "/w/b.png".to_owned()]);
        assert_eq!(own().refuses_pass_against(Some(&r)), None);
        assert!(own().report("ok").contains("lock: same feel"));
    }

    #[test]
    fn a_screen_built_from_the_picked_prototype_passes_on_its_precheck_and_only_failures_reach_a_designer() {
        // New screens, every picture changed, nothing new at S1/S2: passed, no designer turn.
        let built = report(vec![shot("pricing/page/default", true, false), shot("pricing/page/annual", true, false)], vec![]);
        let call = gate_prototype(Some(&built));
        assert_eq!((call.decision, call.judgment), (Decision::AutoPass, None));
        assert!(call.reasons[0].contains("prototype the person picked") && call.reasons[0].contains("390.light.normal"), "{:?}", call.reasons);
        assert_eq!(gate(Some(&built)).decision, Decision::Designer, "the usual gate would have called the designer");
        // A new S1/S2 finding: the designer, with the report — not straight back to the engineer.
        let new = Finding::bare("reflow", Severity::Blocker, "pricing/page/default", "390.light.normal", true);
        let call = gate_prototype(Some(&report(vec![shot("pricing/page/default", true, false)], vec![new])));
        assert_eq!(call.decision, Decision::Designer);
        assert!(call.reasons[0].contains("reflow"));
        // The precheck failed or saw too little: the designer.
        assert_eq!(gate_prototype(None).decision, Decision::Designer);
        let half = report(vec![shot("pricing/page/default", true, false), shot("pricing/page/annual", false, true)], vec![]);
        assert_eq!((gate_prototype(Some(&half)).decision, gate_prototype(Some(&half)).judgment), (Decision::Designer, Some(Judgment::NotRendered)));
    }

    #[test]
    fn what_the_gate_cannot_tell_goes_to_a_designer_and_some_of_it_needs_judgment() {
        assert_eq!(gate(None).judgment, Some(Judgment::PrecheckFailed));
        let failed = report(vec![shot("auth/login/default", false, false), shot("auth/login/error", false, true)], vec![]);
        assert_eq!((gate(Some(&failed)).decision, gate(Some(&failed)).judgment), (Decision::Designer, Some(Judgment::NotRendered)), "no pass on half the pictures");
        let unmapped = Report { unmapped: vec!["src/components/Nav.tsx".into()], ..report(vec![shot("auth/login/default", false, false)], vec![]) };
        assert_eq!(gate(Some(&unmapped)).judgment, Some(Judgment::Unmapped));
        let broad = report((0..BROAD_SCREENS).map(|n| shot(&format!("auth/s{n}/default"), true, false)).collect(), vec![]);
        assert_eq!(gate(Some(&broad)).judgment, Some(Judgment::Broad));
        let nothing = Report { touched: vec![], ..report(vec![], vec![]) };
        assert_eq!((gate(Some(&nothing)).decision, gate(Some(&nothing)).judgment), (Decision::Designer, None), "nothing on the map to render: a plain look");
        let broken = Report { errors: vec!["the preview didn't start".into()], ..report(vec![], vec![]) };
        assert_eq!(gate(Some(&broken)).judgment, Some(Judgment::PrecheckFailed));
        assert!(GateMode::ALL.iter().all(|m| GateMode::parse(m.code()) == Some(*m)));
        assert_eq!(GateMode::default(), GateMode::Shadow, "measured before it acts");
    }

    fn judged(r: &Report) -> (Decision, Option<Judgment>) {
        let call = gate(Some(r));
        (call.decision, call.judgment)
    }

    #[test]
    fn states_left_out_over_the_cap_never_pass_and_only_need_the_cheap_tier() {
        let seen = vec![shot("auth/login/default", false, false)];
        let capped = Report { left_out: 44, left_out_states: vec!["auth/login/error".into()], ..report(seen.clone(), vec![]) };
        let call = gate(Some(&capped));
        assert_eq!((call.decision, call.judgment), (Decision::Designer, Some(Judgment::Capped)), "rendered and unchanged, yet 44 states unseen: no pass");
        assert!(call.reasons[0].contains("44") && !Judgment::Capped.needs_top());
        // The same with changes: still capped, cheap.
        let changed = Report { left_out: 2, ..report(vec![shot("auth/login/default", true, false)], vec![]) };
        assert_eq!(judged(&changed), (Decision::Designer, Some(Judgment::Capped)));
        // Nothing left out and nothing changed: the pass is back.
        assert_eq!(judged(&report(seen, vec![])).0, Decision::AutoPass);
        // A new blocker still goes back whatever was left out.
        let new = Finding::bare("reflow", Severity::Blocker, "auth/login/default", "390.light.normal", true);
        assert_eq!(gate(Some(&Report { left_out: 9, ..report(vec![shot("auth/login/default", true, false)], vec![new]) })).decision, Decision::SendBack);
    }

    #[test]
    fn not_rendered_means_nothing_or_most_pictures_failed_and_a_few_failures_are_partial() {
        let none = report(vec![shot("auth/login/default", false, true), shot("auth/login/error", false, true)], vec![]);
        assert_eq!(judged(&none), (Decision::Designer, Some(Judgment::NotRendered)));
        let few = report((0..6).map(|n| shot(&format!("auth/login/s{n}"), false, n == 0)).collect(), vec![]);
        assert_eq!(judged(&few), (Decision::Designer, Some(Judgment::Partial)));
        assert!(!Judgment::Partial.needs_top() && Judgment::NotRendered.needs_top());
        assert!(gate(Some(&none)).reasons[0].contains("timed out") || gate(Some(&few)).reasons[0].contains("timed out"), "the cause of the failures is said");
        assert!(gate(Some(&few)).reasons[0].contains("Most common cause: timed out"), "{:?}", gate(Some(&few)).reasons);
        assert!(few.summary("x").contains("Why pictures failed: timed out (x1)"));
        let half = report(vec![shot("auth/login/default", false, false), shot("auth/login/error", false, true)], vec![]);
        assert_eq!(judged(&half).1, Some(Judgment::NotRendered), "half of what was asked unseen is too little");
    }

    #[test]
    fn unmapped_ui_files_need_the_top_tier_only_when_many_and_the_threshold_is_the_maps() {
        let unmapped = |n: usize| Report { unmapped: (0..n).map(|i| format!("src/components/C{i}.tsx")).collect(), ..report(vec![shot("auth/login/default", false, false)], vec![]) };
        assert_eq!(judged(&unmapped(2)), (Decision::Designer, Some(Judgment::Unmapped)));
        assert_eq!(judged(&unmapped(crate::design::precheck::UNMAPPED_LARGE)).1, Some(Judgment::UnmappedLarge));
        let strict = Report { unmapped_large: 2, ..unmapped(2) };
        assert_eq!(judged(&strict).1, Some(Judgment::UnmappedLarge));
        assert!(!Judgment::Unmapped.needs_top() && Judgment::UnmappedLarge.needs_top());
        // Changed code users never see is quiet: it neither blocks the pass nor asks for judgment.
        let quiet = Report { unmapped_quiet: vec!["src/lib/api.ts".into()], ..report(vec![shot("auth/login/default", false, false)], vec![]) };
        assert_eq!(judged(&quiet).0, Decision::AutoPass);
        // Nothing rendered, only unmapped UI files: unmapped, not a failure.
        let only = Report { touched: vec![], unmapped: vec!["src/components/A.tsx".into()], ..report(vec![], vec![]) };
        assert_eq!(judged(&only).1, Some(Judgment::Unmapped));
    }

    #[test]
    fn the_strongest_reason_wins_and_all_are_said() {
        let r = Report { left_out: 3, unmapped: (0..12).map(|i| format!("src/components/C{i}.tsx")).collect(), ..report(vec![shot("auth/login/default", false, false)], vec![]) };
        let call = gate(Some(&r));
        assert_eq!(call.judgment, Some(Judgment::UnmappedLarge));
        assert_eq!(call.reasons.len(), 2, "{:?}", call.reasons);
        let broad_and_capped = Report { left_out: 1, ..report((0..BROAD_SCREENS).map(|n| shot(&format!("auth/s{n}/default"), true, false)).collect(), vec![]) };
        assert_eq!(judged(&broad_and_capped).1, Some(Judgment::Broad));
    }

    #[test]
    fn a_failed_precheck_says_why_and_judgments_read_back_from_the_journal() {
        let broken = Report { errors: vec!["the preview didn't start: `npm run dev` stopped (exit 1) — Error: EADDRINUSE".into(), "second".into()], ..report(vec![], vec![]) };
        let call = gate(Some(&broken));
        assert_eq!(call.judgment, Some(Judgment::PrecheckFailed));
        assert!(call.reasons[0].contains("EADDRINUSE") && call.reasons[0].contains("second"), "{:?}", call.reasons);
        for j in Judgment::ALL {
            assert_eq!(Judgment::parse(j.code()), Some(j));
            assert_eq!(Judgment::text_needs_top(j.code()), j.needs_top());
        }
        assert!(Judgment::text_needs_top("something_from_a_newer_version"), "unknown: the safe top tier");
    }

    #[test]
    fn design_qa_runs_cheap_unless_it_takes_judgment() {
        let first = TierAsk { round: 1, ..TierAsk::default() };
        assert_eq!(tier_for(TierPolicy::ECONOMY, first), (ModelTier::Standard, None));
        assert_eq!(tier_for(TierPolicy::ECONOMY, TierAsk { round: 2, ..first }), (ModelTier::Standard, None), "a re-check of a fix stays routine");
        assert_eq!(tier_for(TierPolicy::ECONOMY, TierAsk { round: 3, ..first }), (ModelTier::Deep, Some("round_3_plus")));
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
    fn a_verdict_written_with_lists_as_text_reads_as_the_lists_it_means() {
        // Seen from cloud designers: `checked` as one line, `compared` as text, a lone blocker as a string.
        let as_text = json!({ "compared": "true", "checked": "390, 1280, dark", "polish": "[\"Hero padding uneven\"]" });
        assert_eq!(proof(as_text, 2).refuses_pass(), None);
        let p = proof(json!({ "compared": true, "checked": ["390", "1280", "dark"], "blockers": "Title 20px, spec 24px" }), 2);
        assert_eq!(p.blockers, ["Title 20px, spec 24px"], "a blocker is a sentence: never split at its commas");
        assert!(p.refuses_pass().unwrap().contains("verdict changes"), "a blocker written as text still holds a pass back");
        assert_eq!(p.refuses_changes(), None);
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
