//! Risk-based design QA (`design_qa.mode = risk_batch`, docs/DESIGN-QA.md).
//!
//! A designer's turn on every screen-changing hand-in was the most expensive
//! work a squad did, and most of it looked at changes nothing could go wrong
//! with. In `risk_batch` the deterministic precheck runs on every hand-in and
//! [`route`] reads its report with what the ticket says structurally:
//!
//! - a new S1/S2 finding the engineer didn't waive: the hand-in is
//!   **refused** back to the engineer at once (no designer turn);
//! - a high-risk change — new screens or states on the UI map, many screens
//!   at once, a designer's spec attached, findings the engineer waived, a
//!   re-check after a designer's send-back, or what the precheck couldn't
//!   see: a designer looks at it now (**per ticket**), in a light session;
//! - everything else goes on without a designer turn and is collected into
//!   the **daily design review**, one ticket for all of them, with a random
//!   sample audited on the top tier ([`sample`]); misses tighten the routing
//!   ([`should_tighten`]).
//!
//! No runtime types here: the runtime (`crate::runtime::design_qa`) keeps the
//! records and starts the turns.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::BROAD_SCREENS;
use crate::design::precheck::{self, Report};

/// How a squad runs design QA.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Every screen-changing hand-in gets its own design QA ticket, as before.
    PerTicket,
    /// Only high-risk changes get one; the rest go into the daily review.
    #[default]
    RiskBatch,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::PerTicket, Mode::RiskBatch];

    pub fn code(self) -> &'static str {
        match self {
            Mode::PerTicket => "per_ticket",
            Mode::RiskBatch => "risk_batch",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase().replace('-', "_");
        Mode::ALL.into_iter().find(|m| m.code() == text)
    }
}

/// The daily design review's default time, local: the end of a working day.
pub const BATCH_AT: (u8, u8) = (18, 0);
/// So many changes waiting make a review now, whatever the time.
pub const BATCH_SIZE: usize = 12;
/// The share of changes that skipped a designer audited on the top tier, in percent.
pub const SAMPLE: u8 = 15;
/// Audits finding S1/S2 misses in more than this share, in percent, tighten the routing.
pub const AUDIT_MISS: u8 = 10;
/// How many of the latest audited changes the miss rate is read over.
pub const AUDIT_WINDOW: usize = 20;
/// Fewer audited changes than this say nothing yet.
pub const AUDIT_MIN: usize = 5;
/// Tightened routing relaxes after so many audited changes in a row with no miss.
pub const RELAX_CLEAN: usize = 10;
/// Tightened, so many changed screens already make a change high-risk.
pub const TIGHT_BROAD_SCREENS: usize = 2;

/// A squad's design QA settings (`equipe design qa-config`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub mode: Mode,
    /// The daily review's time, local `(hour, minute)`.
    pub batch_at: (u8, u8),
    /// A review is made as soon as this many changes wait.
    pub batch_size: usize,
    /// Audited share, in percent.
    pub sample: u8,
    /// Miss rate that tightens the routing, in percent.
    pub audit_miss: u8,
    /// The routing is tightened now.
    pub tight: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { mode: Mode::default(), batch_at: BATCH_AT, batch_size: BATCH_SIZE, sample: SAMPLE, audit_miss: AUDIT_MISS, tight: false }
    }
}

impl Settings {
    /// The sample rate in force: doubled while tightened, up to half.
    pub fn sample_now(&self) -> u8 {
        if self.tight { self.sample.saturating_mul(2).clamp(self.sample, 50.max(self.sample)) } else { self.sample }
    }
}

/// `18:00` as `(18, 0)`.
pub fn parse_time(text: &str) -> Option<(u8, u8)> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m) = (h.parse::<u8>().ok()?, m.parse::<u8>().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

pub fn time_code((h, m): (u8, u8)) -> String {
    format!("{h:02}:{m:02}")
}

/// Where a hand-in goes after its precheck.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// Back to its engineer: a new S1/S2 finding it didn't waive.
    Refused,
    /// A designer looks now.
    PerTicket,
    /// On without a designer turn, into the daily review.
    Batch,
}

impl Route {
    pub fn code(self) -> &'static str {
        match self {
            Route::Refused => "refused",
            Route::PerTicket => "per_ticket",
            Route::Batch => "batch",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Route::Refused, Route::PerTicket, Route::Batch].into_iter().find(|r| r.code() == text)
    }
}

/// Why a change is high-risk: read from the report and the ticket's
/// structure, never from its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// The engineer waived new S1/S2 findings: a designer judges the waiver.
    Waived,
    /// States the map had no picture of at the base: new screens, flows or states.
    NewScreens,
    /// Many screens changed at once.
    Broad,
    /// A designer's spec or mockup is attached: the build is held to it.
    Spec,
    /// A designer sent its last round back: the same designer checks the fix.
    Recheck,
    /// The routing is tightened and the change has new findings.
    Tight,
    /// The precheck couldn't see it: no report, or nothing (or too little) rendered.
    Unverified,
    /// Many changed UI files no screen of the map lists.
    UnmappedLarge,
}

impl Risk {
    pub fn code(self) -> &'static str {
        match self {
            Risk::Waived => "waived",
            Risk::NewScreens => "new_screens",
            Risk::Broad => "broad",
            Risk::Spec => "spec",
            Risk::Recheck => "recheck",
            Risk::Tight => "tight",
            Risk::Unverified => "unverified",
            Risk::UnmappedLarge => "unmapped_large",
        }
    }

    /// In a few English words, for the designer's brief and the record.
    pub fn said(self) -> &'static str {
        match self {
            Risk::Waived => "the engineer waived new S1/S2 findings",
            Risk::NewScreens => "it adds screens or states the UI map had no picture of",
            Risk::Broad => "many screens changed at once",
            Risk::Spec => "a designer's spec or mockup is attached",
            Risk::Recheck => "a designer sent its last round back",
            Risk::Tight => "routing is tightened after audit misses and it has new findings",
            Risk::Unverified => "the precheck couldn't see it",
            Risk::UnmappedLarge => "many changed UI files no screen of the map lists",
        }
    }
}

/// What [`route`] reads.
#[derive(Debug, Clone, Copy, Default)]
pub struct RouteAsk<'a> {
    /// The precheck's report; `None`: it gave none.
    pub report: Option<&'a Report>,
    /// The engineer waived the precheck's findings, with a reason.
    pub waived: bool,
    /// A designer's spec or mockup is attached ([`spec_attached`]).
    pub spec: bool,
    /// The last design QA of the same ticket was a designer's send-back.
    pub recheck: bool,
    /// The routing is tightened.
    pub tight: bool,
}

/// Where a hand-in goes, why, and whether a designer can judge it from the
/// report alone (a light session with no repository).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteCall {
    pub route: Route,
    pub risks: Vec<Risk>,
    /// The report shows enough for a light session; else the designer gets
    /// the full one, with the worktree.
    pub lite: bool,
}

impl RouteCall {
    pub fn risk_codes(&self) -> String {
        self.risks.iter().map(|r| r.code()).collect::<Vec<_>>().join(",")
    }
}

/// Where a hand-in with changed screens goes in `risk_batch`.
pub fn route(ask: &RouteAsk<'_>) -> RouteCall {
    let call = |route, risks: Vec<Risk>, lite| RouteCall { route, risks, lite };
    let Some(r) = ask.report else { return call(Route::PerTicket, vec![Risk::Unverified], false) };
    let blockers = r.new_blockers();
    if !blockers.is_empty() && !ask.waived {
        return call(Route::Refused, Vec::new(), false);
    }
    let taken = r.taken();
    if taken == 0 {
        // Nothing on the map to render and no UI file outside it: nothing users see.
        if r.touched.is_empty() && r.unmapped.is_empty() && r.errors.is_empty() {
            return call(if ask.spec || ask.recheck { Route::PerTicket } else { Route::Batch }, [ask.spec.then_some(Risk::Spec), ask.recheck.then_some(Risk::Recheck)].into_iter().flatten().collect(), false);
        }
        return call(Route::PerTicket, vec![Risk::Unverified], false);
    }
    // Half or more of the pictures failed: too little seen to let it through.
    if r.failed() * 2 >= r.shots.len() {
        return call(Route::PerTicket, vec![Risk::Unverified], false);
    }
    let mut risks = Vec::new();
    if !blockers.is_empty() {
        risks.push(Risk::Waived);
    }
    if !r.new_states().is_empty() {
        risks.push(Risk::NewScreens);
    }
    let broad = if ask.tight { TIGHT_BROAD_SCREENS } else { BROAD_SCREENS };
    if r.changed_screens().len() >= broad {
        risks.push(Risk::Broad);
    }
    if ask.spec {
        risks.push(Risk::Spec);
    }
    if ask.recheck {
        risks.push(Risk::Recheck);
    }
    if ask.tight && r.findings.iter().any(|f| f.new) {
        risks.push(Risk::Tight);
    }
    let large = r.unmapped.len() >= r.unmapped_large_at();
    if large {
        risks.push(Risk::UnmappedLarge);
    }
    let route = if risks.is_empty() { Route::Batch } else { Route::PerTicket };
    call(route, risks, !large)
}

/// What counts as a mockup a designer attached: a picture or document of
/// one of these kinds anywhere under the repository's `design/` folder but
/// the UI map's, design QA's and the checks' own folders.
const MOCKUP_KINDS: [&str; 8] = [".png", ".jpg", ".jpeg", ".webp", ".svg", ".pdf", ".html", ".fig"];
/// …and a written spec: text in a folder kept for specs of screens. The
/// system's own files (`design/guidelines.md`, `design/tokens.json`) are
/// named by every ticket and say nothing of what this one changes.
const SPEC_KINDS: [&str; 2] = [".md", ".json"];
const SPEC_FOLDERS: [&str; 5] = ["specs/", "spec/", "mockups/", "screens/", "redesign/"];
/// Folders under `design/` that hold no spec: the map, design QA's reports, the checks.
const NOT_SPECS: [&str; 3] = ["flows/", "qa/", "checks/"];

/// The `design/…` files `text` names, by path, that are a designer's spec or
/// mockup for this work: not the map, the reports or the system's own files.
pub fn spec_paths(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || matches!(c, '`' | '(' | ')' | '[' | ']' | '"' | '\'' | '<' | '>' | ',' | ';')) {
        let token = token.trim_end_matches(['.', ':', '!', '?']);
        let Some(at) = token.find("design/") else { continue };
        // `design/` itself, or a folder ending in it (`app/design/`), not `redesign/`.
        if at > 0 && !token[..at].ends_with('/') {
            continue;
        }
        let rel = &token[at..];
        let lower = rel.to_ascii_lowercase();
        let inner = &lower["design/".len()..];
        if NOT_SPECS.iter().any(|d| inner.starts_with(d)) {
            continue;
        }
        let mockup = MOCKUP_KINDS.iter().any(|k| lower.ends_with(k));
        let spec = SPEC_KINDS.iter().any(|k| lower.ends_with(k)) && SPEC_FOLDERS.iter().any(|d| inner.starts_with(d) || inner.contains(&format!("/{d}")));
        if !mockup && !spec {
            continue;
        }
        if !out.iter().any(|o| o == rel) {
            out.push(rel.to_owned());
        }
    }
    out
}

/// Whether a designer's spec is attached to a ticket: its plan is a design
/// file, or its body names a spec or mockup under `design/`. Who wrote the
/// ticket says nothing by itself: a designer files small fixes too.
pub fn spec_attached(body: &str, plan: Option<&str>) -> bool {
    plan.is_some_and(|p| p.starts_with("design/") || p.contains("/design/")) || !spec_paths(body).is_empty()
}

/// A mix of the bits of `x`: the same everywhere, for a sample anyone can
/// check again (splitmix64).
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// `rate` percent of `items` (rounded up, at least one when the rate isn't
/// zero), picked at random but reproducibly by `salt` (the review's ticket).
pub fn sample(items: &[u64], rate: u8, salt: u64) -> Vec<u64> {
    if rate == 0 || items.is_empty() {
        return Vec::new();
    }
    let k = (items.len() * usize::from(rate.min(100))).div_ceil(100).clamp(1, items.len());
    let mut keyed: Vec<(u64, u64)> = items.iter().map(|&i| (mix(i ^ mix(salt)), i)).collect();
    keyed.sort_unstable();
    let mut picked: Vec<u64> = keyed.into_iter().take(k).map(|(_, i)| i).collect();
    picked.sort_unstable();
    picked
}

/// Whether audits say the routing lets too much through: of the latest
/// audited changes (newest first, `true` for an S1/S2 miss), more than
/// `threshold` percent missed.
pub fn should_tighten(latest: &[bool], threshold: u8) -> bool {
    let window = &latest[..latest.len().min(AUDIT_WINDOW)];
    let misses = window.iter().filter(|m| **m).count();
    window.len() >= AUDIT_MIN && misses * 100 > usize::from(threshold) * window.len()
}

/// Whether tightened routing may relax: the latest [`RELAX_CLEAN`] audited
/// changes since it tightened (newest first) had no miss.
pub fn may_relax(since_tight: &[bool]) -> bool {
    since_tight.len() >= RELAX_CLEAN && since_tight[..RELAX_CLEAN].iter().all(|m| !m)
}

/// The tier a light per-ticket session runs at: the cheap one, the top one
/// from a ticket's third round on (a disagreement that keeps coming back).
pub fn lite_tier(policy: super::TierPolicy, round: u32) -> equipe_protocol::tiers::ModelTier {
    if round >= 3 { policy.top } else { policy.cheap }
}

/// A designer's word on one change of a review or audit (`design_review`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewItem {
    pub ticket: u64,
    /// S1/S2: each becomes a fix ticket for its engineer.
    pub blockers: Vec<String>,
    /// S3/S4: gathered into the polish ticket.
    pub polish: Vec<String>,
}

/// A daily review's or audit's verdict.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Review {
    pub findings: String,
    pub items: Vec<ReviewItem>,
}

impl Review {
    /// Read from `design_review`'s arguments, holding it to cover every one
    /// of `expected` (the changes it was given).
    pub fn read(args: &Value, expected: &[u64]) -> Result<Review, String> {
        let findings = args.get("findings").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or("Say in `findings` what you looked at and how the changes stand.")?;
        let list = |v: &Value, key: &str| -> Vec<String> { v.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()).unwrap_or_default() };
        let mut items: Vec<ReviewItem> = Vec::new();
        for v in args.get("items").and_then(Value::as_array).into_iter().flatten() {
            let Some(ticket) = v.get("ticket").and_then(Value::as_u64) else { continue };
            if !expected.contains(&ticket) {
                return Err(format!("T-{ticket} isn't one of the changes you were given: {}.", expected.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ")));
            }
            match items.iter_mut().find(|i| i.ticket == ticket) {
                Some(item) => {
                    item.blockers.extend(list(v, "blockers"));
                    item.polish.extend(list(v, "polish"));
                }
                None => items.push(ReviewItem { ticket, blockers: list(v, "blockers"), polish: list(v, "polish") }),
            }
        }
        let missing: Vec<String> = expected.iter().filter(|t| !items.iter().any(|i| i.ticket == **t)).map(|t| format!("T-{t}")).collect();
        if !missing.is_empty() {
            return Err(format!("Give every change its entry in `items`, even when it is fine (no blockers, no polish); missing: {}.", missing.join(", ")));
        }
        Ok(Review { findings: findings.to_owned(), items })
    }

    pub fn blockers(&self) -> usize {
        self.items.iter().map(|i| i.blockers.len()).sum()
    }

    pub fn polish(&self) -> usize {
        self.items.iter().map(|i| i.polish.len()).sum()
    }
}

/// Pictures `qa_pictures` gives at once.
pub const PICTURES_AT_ONCE: usize = 6;

/// The pictures in the reports under `dirs` of `states` at `conditions`
/// (that exist), at most [`PICTURES_AT_ONCE`]; and the states none had.
pub fn find_pictures(dirs: &[PathBuf], states: &[String], conditions: &[String]) -> (Vec<String>, Vec<String>) {
    let mut found: Vec<String> = Vec::new();
    let mut seen: Vec<&String> = Vec::new();
    for dir in dirs {
        let Some(report) = precheck::read(dir) else { continue };
        for s in &report.shots {
            if !states.contains(&s.key) || !conditions.contains(&s.condition) {
                continue;
            }
            if let Some(p) = s.picture.as_ref().filter(|p| Path::new(p).is_file())
                && !found.contains(p)
                && found.len() < PICTURES_AT_ONCE
            {
                found.push(p.clone());
                if let Some(k) = states.iter().find(|k| **k == s.key) {
                    seen.push(k);
                }
            }
        }
    }
    let missing = states.iter().filter(|k| !seen.contains(k)).cloned().collect();
    (found, missing)
}

/// What `qa_pictures` answers a designer.
pub fn pictures_said(found: &[String], missing: &[String]) -> String {
    let mut out = Vec::new();
    if !found.is_empty() {
        out.push(format!("Pictures (Read each once):\n- {}", found.join("\n- ")));
    }
    if !missing.is_empty() {
        out.push(format!(
            "Not rendered: {}. Judge without them, or say in your findings what you couldn't see.",
            missing.join(", ")
        ));
    }
    if out.is_empty() { "Nothing matched.".into() } else { out.join("\n\n") }
}

/// What a designer in a light design QA session is told instead of its
/// charter: who it is, what it has, its budget, the severity line, and how
/// to finish. In English; findings in `language`.
pub fn lite_charter(name: &str, language: Option<&str>) -> String {
    let language = language.map(|l| format!(" Write your findings in {l}.")).unwrap_or_default();
    format!(
        "You are {name}, the squad's designer, in a short design QA session run by equipe. You have no repository here: only \
equipe's precheck report of the change (report.md and report.json in this folder), its pictures and crops, and the spec excerpts in \
your brief. Judge from them; don't look for the code or start a preview. Your budget is about ten tool calls: read the brief, look at \
the pictures it names (Read each once — a picture already looked at stays in your context), decide. To see a state or condition you \
weren't given, call `qa_pictures` with its key (`flow/screen/state`) and conditions (`390.dark.normal`, `1280.light.text-200`) — \
never browse the folders. What the precheck measured is settled: don't measure it again; a finding it marks as there before isn't \
this change's. Blockers (S1/S2) are function or data broken, layout broken at a major width, a WCAG 2.2 A/AA failure such as content \
or a control lost at 200% text or 320px reflow, or a clear difference from the spec. Polish (S3/S4) is what keeps it from looking \
finished — rhythm, alignment, density, hierarchy — always written down, never a reason to send work back. Finish with the tool your \
brief names, then end your turn with one line.{language}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::precheck::{Diff, Finding, Severity, Shot};
    use serde_json::json;

    fn shot(key: &str, changed: bool, base: bool) -> Shot {
        Shot {
            key: key.into(),
            condition: "390.light.normal".into(),
            picture: Some(format!("/tmp/{key}.png")),
            baseline: base.then(|| format!("/tmp/base/{key}.png")),
            diff: base.then(|| Diff { changed, share: if changed { 0.2 } else { 0. }, ..Diff::default() }),
            ..Shot::default()
        }
    }

    fn report(shots: Vec<Shot>, findings: Vec<Finding>) -> Report {
        Report { touched: vec!["settings/main".into()], conditions: vec!["390.light.normal".into()], shots, findings, ..Report::default() }
    }

    fn routed(r: &Report, f: impl FnOnce(&mut RouteAsk<'_>)) -> RouteCall {
        let mut ask = RouteAsk { report: Some(r), ..RouteAsk::default() };
        f(&mut ask);
        route(&ask)
    }

    #[test]
    fn a_low_risk_change_goes_into_the_daily_review_and_a_definite_failure_is_refused_unless_waived() {
        let small = report(vec![shot("settings/main/default", true, true)], vec![]);
        assert_eq!(routed(&small, |_| {}), RouteCall { route: Route::Batch, risks: vec![], lite: true });
        let blocker = Finding::bare("text-clipped", Severity::Blocker, "settings/main/default", "390.light.text-200", true);
        let broken = report(vec![shot("settings/main/default", true, true)], vec![blocker]);
        assert_eq!(routed(&broken, |_| {}).route, Route::Refused, "back to the engineer, no designer turn");
        let waived = routed(&broken, |a| a.waived = true);
        assert_eq!((waived.route, waived.risks.as_slice(), waived.lite), (Route::PerTicket, &[Risk::Waived][..], true), "a designer judges the waiver");
        // A finding that was there before never refuses.
        let old = Finding::bare("text-clipped", Severity::Blocker, "settings/main/default", "390.light.text-200", false);
        assert_eq!(routed(&report(vec![shot("settings/main/default", true, true)], vec![old]), |_| {}).route, Route::Batch);
    }

    #[test]
    fn high_risk_is_read_from_the_map_and_the_ticket_not_from_words() {
        // A state the map had no picture of: a new screen.
        let new = report(vec![shot("settings/main/default", true, true), shot("billing/plans/default", true, false)], vec![]);
        assert_eq!(new.new_states(), ["billing/plans/default"]);
        assert_eq!(routed(&new, |_| {}).risks, [Risk::NewScreens]);
        let broad = report((0..BROAD_SCREENS).map(|n| shot(&format!("app/s{n}/default"), true, true)).collect(), vec![]);
        assert_eq!(routed(&broad, |_| {}).risks, [Risk::Broad]);
        let small = report(vec![shot("settings/main/default", true, true)], vec![]);
        assert_eq!(routed(&small, |a| a.spec = true).risks, [Risk::Spec]);
        assert_eq!(routed(&small, |a| a.recheck = true).risks, [Risk::Recheck]);
        let large = Report { unmapped: (0..12).map(|i| format!("src/components/C{i}.tsx")).collect(), ..small.clone() };
        let call = routed(&large, |_| {});
        assert_eq!((call.risks.as_slice(), call.lite), (&[Risk::UnmappedLarge][..], false), "too much outside the map for a light look");
        assert!(spec_attached("Build it per `design/specs/billing.md`.", None));
        assert!(spec_attached("-", Some("design/plans/billing.md")));
        assert!(!spec_attached("Fix the redesign/ folder and design/flows/flows.json", None), "the map isn't a spec");
        assert!(!spec_attached("Follow design/guidelines.md and design/tokens.json; see design/qa/T-12.md.", None), "the system's files and QA reports aren't one");
        assert_eq!(spec_paths("see design/mock.png, and app/design/screens/billing.md."), ["design/mock.png", "design/screens/billing.md"]);
    }

    #[test]
    fn what_the_precheck_could_not_see_goes_to_a_designer_with_the_worktree() {
        assert_eq!(route(&RouteAsk::default()), RouteCall { route: Route::PerTicket, risks: vec![Risk::Unverified], lite: false });
        let failed = Shot { error: Some("timed out".into()), ..shot("settings/main/error", false, true) };
        let half = report(vec![shot("settings/main/default", false, true), failed], vec![]);
        assert_eq!(routed(&half, |_| {}).risks, [Risk::Unverified]);
        let nothing = Report { touched: vec![], ..report(vec![], vec![]) };
        assert_eq!(routed(&nothing, |_| {}).route, Route::Batch, "nothing users see");
        let broken = Report { errors: vec!["the preview didn't start".into()], ..report(vec![], vec![]) };
        assert_eq!(routed(&broken, |_| {}).risks, [Risk::Unverified]);
    }

    #[test]
    fn tightened_routing_sends_more_to_a_designer_and_audits_more() {
        let two = report(vec![shot("a/one/default", true, true), shot("a/two/default", true, true)], vec![]);
        assert_eq!(routed(&two, |_| {}).route, Route::Batch);
        assert_eq!(routed(&two, |a| a.tight = true).risks, [Risk::Broad]);
        let polish = Finding::bare("overlap", Severity::Polish, "a/one/default", "390.light.normal", true);
        let one = report(vec![shot("a/one/default", true, true)], vec![polish]);
        assert_eq!(routed(&one, |_| {}).route, Route::Batch);
        assert_eq!(routed(&one, |a| a.tight = true).risks, [Risk::Tight]);
        let s = Settings::default();
        assert_eq!((s.sample_now(), Settings { tight: true, ..s }.sample_now()), (15, 30));
        assert_eq!(Settings { tight: true, sample: 40, ..s }.sample_now(), 50);
    }

    #[test]
    fn the_audit_sample_is_its_share_rounded_up_and_reproducible() {
        let items: Vec<u64> = (100..120).collect();
        let picked = sample(&items, 15, 7);
        assert_eq!(picked.len(), 3);
        assert_eq!(picked, sample(&items, 15, 7), "the same review picks the same");
        assert_ne!(picked, sample(&items, 15, 8), "another review, another pick");
        assert_eq!(sample(&items[..2], 10, 1).len(), 1, "at least one");
        assert!(sample(&items, 0, 1).is_empty() && sample(&[], 20, 1).is_empty());
    }

    #[test]
    fn misses_over_the_threshold_tighten_and_a_clean_run_relaxes() {
        assert!(!should_tighten(&[true, true, true], 10), "too few audits to say");
        assert!(should_tighten(&[true, false, false, false, false], 10));
        let mostly_clean: Vec<bool> = (0..20).map(|i| i == 3).collect();
        assert!(!should_tighten(&mostly_clean, 10), "1 of 20 is 5%");
        assert!(may_relax(&[false; 10]) && !may_relax(&[false; 9]));
        let mut once = vec![false; 10];
        once[4] = true;
        assert!(!may_relax(&once));
        assert_eq!(lite_tier(super::super::TierPolicy::ECONOMY, 1), equipe_protocol::tiers::ModelTier::Standard);
        assert_eq!(lite_tier(super::super::TierPolicy::ECONOMY, 3), equipe_protocol::tiers::ModelTier::Deep);
    }

    #[test]
    fn a_review_covers_every_change_it_was_given() {
        let args = json!({ "findings": "Two fine, one cut off.", "items": [
            { "ticket": 4, "blockers": ["Save cut off at 390 dark"], "polish": ["Gap 12 not 16"] },
            { "ticket": 5 },
        ]});
        let missing = Review::read(&args, &[4, 5, 6]).unwrap_err();
        assert!(missing.contains("T-6"), "{missing}");
        let review = Review::read(&args, &[4, 5]).unwrap();
        assert_eq!((review.blockers(), review.polish()), (1, 1));
        assert!(Review::read(&json!({ "findings": "x", "items": [{ "ticket": 9 }] }), &[4]).unwrap_err().contains("isn't one of"));
        assert!(Review::read(&json!({ "items": [] }), &[]).is_err(), "findings needed");
        for m in Mode::ALL {
            assert_eq!(Mode::parse(m.code()), Some(m));
        }
        assert_eq!(Mode::parse("risk-batch"), Some(Mode::RiskBatch));
        assert_eq!(Mode::default(), Mode::RiskBatch);
        assert_eq!((parse_time("18:00"), parse_time("7:05"), parse_time("24:00")), (Some((18, 0)), Some((7, 5)), None));
        assert!(lite_charter("designer", Some("Korean")).contains("Write your findings in Korean."));
    }

    #[test]
    fn qa_pictures_finds_the_states_asked_for_and_says_what_it_did_not_take() {
        let dir = std::env::temp_dir().join(format!("equipe-qa-pictures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("shots")).unwrap();
        let pic = dir.join("shots").join("a.png");
        std::fs::write(&pic, b"png").unwrap();
        let report = Report {
            shots: vec![
                precheck::Shot { key: "a/b/default".into(), condition: "390.dark.normal".into(), picture: Some(pic.to_string_lossy().into_owned()), ..Default::default() },
                precheck::Shot { key: "a/b/default".into(), condition: "1280.light.normal".into(), picture: Some("/nowhere.png".into()), ..Default::default() },
            ],
            ..Default::default()
        };
        precheck::write(&dir, &report, "# x").unwrap();
        let (found, missing) = find_pictures(std::slice::from_ref(&dir), &["a/b/default".into(), "a/c/error".into()], &["390.dark.normal".into(), "1280.light.normal".into()]);
        assert_eq!(found, [pic.to_string_lossy().into_owned()], "a picture that isn't there isn't given");
        assert_eq!(missing, ["a/c/error"]);
        let said = pictures_said(&found, &missing);
        assert!(said.contains("Read each once") && said.contains("Not rendered: a/c/error"), "{said}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
