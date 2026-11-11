//! Which model and effort a ticket's run lanes get: the rules, without a
//! daemon. A lane picks its tier once, when it starts — never mid-session,
//! where a model change would throw the prompt cache away — and a ticket
//! keeps its tier when its member moves engines: each engine maps the same
//! step to a model and effort of its own ([`target`]).
//!
//! On the squad's default, quality first ([`ModelChoice::QualityFirst`]),
//! equipe only raises: a ticket runs at the stronger of what its rules ask
//! ([`rule`]) and what its member already runs on ([`floor`]), component by
//! component — never a weaker model, never less thinking than the member's
//! own. What a cheaper tier would have been is recorded
//! ([`TicketModel::shadow`]) and never acted on. Nothing here reads the
//! plans: when plan room runs short the lanes shrink (`crate::lanes`), and
//! tiers never drop for it — on Max parallel work too.
//!
//! | Ticket | Tier | Claude | Codex | Cheaper tier recorded |
//! |---|---|---|---|---|
//! | PM work | the PM's own | — | — | never |
//! | Design QA review (squad policy `economy`, the default; `crate::design_qa::tier_for`) | standard; deep for a redesign, round 2+, or a precheck that needs judgment | Sonnet · medium (Opus · high) | Sol · low (Sol · high) | below the member's own unless the person set its model |
//! | QA, security review | strong (deep with a risk tag) | Opus · medium | Sol · medium | never |
//! | Risk tag (auth, payments, migration, security, concurrency, release) | deep | Opus · high | Sol · high | never |
//! | Design work | deep | Opus · high | Sol · high | never |
//! | Code: one change in many places, a large refactor | deep | Opus · high | Sol · high | never |
//! | Code: a feature or a fix | strong | Opus · medium | Sol · medium | yes |
//! | Code: a small change in one place, with a check to run | standard | Sonnet · medium | Sol · low | yes |
//! | Docs, changelog | standard | Sonnet · medium | Sol · low | with a check |
//! | Read-only research, logs, a status check | light with a check, else standard | Haiku · medium | Luna · medium | with a check |
//! | Other tasks, actions | strong | Opus · medium | Sol · medium | tasks: yes; actions: never |
//!
//! The member's own model and effort are the floor: a frontend member on
//! Sonnet runs a feature on Opus, a backend member on Opus at its own effort
//! keeps it. A ticket with no check to run never goes on a light tier and
//! never records a cheaper one. Only the person goes below the floor, by
//! setting a ticket's tier themselves ([`TierBy::Person`]) — and design QA's
//! policy ([`QaTier`]), where nobody but equipe set the designer's model: a
//! model the person set for the member is its floor there too.
//!
//! When work comes back wrong ([`after_send_back`]): the first correctness
//! send-back carries on in the same conversation, warm; the second (failed
//! checks count) — or a lane caught looping — starts the ticket's next lane
//! one tier up, fresh, with a handoff note; the third goes to the PM to
//! decide. Style and environment send-backs raise nothing.

use equipe_protocol::Engine;
use equipe_protocol::TicketKind;
use equipe_protocol::tiers::{ModelChoice, ModelTier, RiskTag, SendBackReason, TicketModel, TierBy, TierReason};

/// The roles whose work is reviewing someone else's.
const REVIEWERS: [&str; 2] = ["qa", crate::roles::SECURITY];

/// Claude Code's own effort when none is given: `xhigh` on its current models.
/// A member set to no effort is taken to run there, so equipe never passes
/// less than that.
pub const CLAUDE_DEFAULT_EFFORT: &str = "xhigh";
/// Codex's effort when none is given and its list doesn't say.
pub const CODEX_DEFAULT_EFFORT: &str = "medium";

/// Identical tool calls in a row that make a lane look caught in a loop.
pub const LOOP_REPEATS: u32 = 8;

/// What the rules read from a ticket and who works it.
#[derive(Debug, Clone, Copy)]
pub struct Work<'a> {
    pub role: &'a str,
    pub kind: TicketKind,
    pub title: &'a str,
    pub body: &'a str,
    /// It is a design QA ticket: a designer checks an engineer's screens.
    pub design_qa: bool,
    /// A design QA ticket's own tier, as the squad's policy reads it
    /// (`crate::design_qa::tier_for`); `None`: design QA's fixed cell.
    pub qa_tier: Option<QaTier>,
    /// The member changes files on its tickets.
    pub edits: bool,
}

/// The tier design QA's policy picked for a design QA ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QaTier {
    pub tier: ModelTier,
    /// It may run below its member's own model and effort: nobody but equipe
    /// set the member's model. `false`: the person set it, so the member's
    /// own is the floor and the policy's tier is only recorded beside it.
    pub below_floor: bool,
}

/// What the rules ask for a ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub tier: ModelTier,
    pub reason: TierReason,
    pub risk: Vec<RiskTag>,
    /// A cell no tier is ever lowered in: a cheaper one is never recorded.
    pub never_lower: bool,
    /// It runs at exactly its tier, below its member's own model included:
    /// design QA's policy, where nobody but equipe set the member's model.
    pub below_floor: bool,
}

/// What the rules ask for `w`. `None` for the PM, whose work keeps its own model.
pub fn rule(w: &Work<'_>) -> Option<Rule> {
    if w.role == crate::roles::PM {
        return None;
    }
    let text = format!("{}\n{}", w.title, w.body);
    let risk = risk_tags(&text);
    let checked = checkable(w.body);
    let at = |tier, reason, never_lower: bool| Rule { tier, reason, risk: risk.clone(), never_lower: never_lower || !risk.is_empty() || !checked, below_floor: false };
    Some(if let (true, Some(q)) = (w.design_qa, w.qa_tier) {
        // Kept at the member's own where the person set it, the policy's tier recorded beside it.
        Rule { tier: q.tier, reason: TierReason::DesignQa, risk: risk.clone(), never_lower: q.below_floor, below_floor: q.below_floor }
    } else if w.design_qa {
        at(ModelTier::Deep, TierReason::DesignQa, true)
    } else if REVIEWERS.contains(&w.role) {
        at(if risk.is_empty() { ModelTier::Strong } else { ModelTier::Deep }, TierReason::Review, true)
    } else if !risk.is_empty() {
        at(ModelTier::Deep, TierReason::Risk, true)
    } else if w.role == "designer" {
        at(ModelTier::Deep, TierReason::Design, true)
    } else if broad(&text) {
        at(ModelTier::Deep, TierReason::Broad, true)
    } else if docs(w.title) {
        at(ModelTier::Standard, TierReason::Docs, false)
    } else if read_only(w) {
        at(if checked { ModelTier::Light } else { ModelTier::Standard }, TierReason::ReadOnly, false)
    } else {
        match w.kind {
            TicketKind::Code if checked && small(w.title) => at(ModelTier::Standard, TierReason::Small, false),
            TicketKind::Code => at(ModelTier::Strong, TierReason::Code, false),
            TicketKind::Task => at(ModelTier::Strong, TierReason::Task, false),
            TicketKind::Action => at(ModelTier::Strong, TierReason::Action, true),
        }
    })
}

/// Whether any of `words` is in `text` as a word of its own (its start at a
/// word's start): `auth` in "auth flow" and "authentication", not in "author"
/// unless listed so.
fn has_word(text: &str, words: &[&str]) -> bool {
    let lower = text.to_lowercase();
    words.iter().any(|w| {
        lower.match_indices(w).any(|(at, _)| at == 0 || !lower[..at].chars().next_back().is_some_and(char::is_alphanumeric))
    })
}

/// What in `text` — a ticket's title and body, paths included — is costly to
/// get wrong. Read in English words and paths only; a ticket it misses runs at
/// its member's floor or its kind's tier, never lower.
pub fn risk_tags(text: &str) -> Vec<RiskTag> {
    const RULES: [(RiskTag, &[&str]); 6] = [
        (
            RiskTag::Auth,
            &["auth/", "authentication", "authorization", "authoriz", "oauth", "login", "log in", "sign-in", "sign in", "signin", "password", "permission", "session token", "access token", "jwt", "sso", "rbac"],
        ),
        (RiskTag::Payments, &["payment", "billing", "invoice", "checkout", "stripe", "subscription", "refund", "pricing plan"]),
        (RiskTag::Migration, &["migration", "migrate", "schema change", "alter table", "drop table", "backfill", "delete data", "data deletion", "purge"]),
        (RiskTag::Security, &["security", "vulnerab", "secret", "encrypt", "decrypt", "credential", "xss", "csrf", "injection", "sandbox escape", "cve-"]),
        (RiskTag::Concurrency, &["concurren", "race condition", "deadlock", "mutex", "lock-free", "thread safety", "thread-safe", "atomic"]),
        (RiskTag::Release, &["release", "deploy", "rollout", "roll out", "production", ".github/workflows", "publish to", "version bump"]),
    ];
    let mut tags: Vec<RiskTag> = RULES.iter().filter(|(_, words)| has_word(text, words)).map(|(tag, _)| *tag).collect();
    // Plain `auth` names the area too: `auth.rs`, "the auth flow".
    if !tags.contains(&RiskTag::Auth) && has_word(text, &["auth"]) && !has_word(text, &["author"]) {
        tags.insert(0, RiskTag::Auth);
    }
    tags
}

/// Whether a ticket says how its work is checked: a test, a lint, a build or
/// a command to run, or test-plan cases (`TP-1`).
pub fn checkable(body: &str) -> bool {
    has_word(
        body,
        &["test", "lint", "clippy", "typecheck", "type-check", "tsc", "cargo ", "npm run", "npm test", "pnpm ", "yarn ", "pytest", "go test", "make ", "xcodebuild", "gradle", "tp-", "verify with", "check with", "ci "],
    )
}

/// One change made the same way in several places, or a large refactor.
fn broad(text: &str) -> bool {
    has_word(text, &["everywhere", "across the", "across all", "all call sites", "every call site", "each call site", "rename", "refactor", "rewrite", "codebase-wide", "repo-wide", "throughout"])
}

/// Docs or a changelog, as its title says.
fn docs(title: &str) -> bool {
    has_word(title, &["docs", "doc ", "documentation", "readme", "changelog", "release notes", "typo"])
}

/// A small change in one place, as its title says.
fn small(title: &str) -> bool {
    has_word(title, &["lint", "typo", "copy", "wording", "label", "config value", "bump", "constant", "rename a", "fix a warning", "warning"])
}

/// Reading only: a member that doesn't edit, or a task that investigates.
fn read_only(w: &Work<'_>) -> bool {
    w.kind != TicketKind::Code
        && (!w.edits || has_word(w.title, &["investigate", "research", "analyze", "analyse", "look into", "status check", "check the status", "read the logs", "log analysis", "survey", "audit"]))
}

// ── tiers on an engine ──

/// The model family and effort a tier is on `engine`.
pub fn target(tier: ModelTier, engine: Engine) -> (&'static str, &'static str) {
    match (engine, tier) {
        (Engine::Claude, ModelTier::Light) => ("haiku", "medium"),
        (Engine::Claude, ModelTier::Standard) => ("sonnet", "medium"),
        (Engine::Claude, ModelTier::Strong) => ("opus", "medium"),
        (Engine::Claude, ModelTier::Deep) => ("opus", "high"),
        (Engine::Claude, ModelTier::Max) => ("opus", "xhigh"),
        (Engine::Codex, ModelTier::Light) => ("luna", "medium"),
        (Engine::Codex, ModelTier::Standard) => ("sol", "low"),
        (Engine::Codex, ModelTier::Strong) => ("sol", "medium"),
        (Engine::Codex, ModelTier::Deep) => ("sol", "high"),
        (Engine::Codex, ModelTier::Max) => ("sol", "xhigh"),
    }
}

/// How strong a model family is on its engine; `None` for one equipe can't
/// place: a provider's, an unknown family.
fn family_rank(engine: Engine, family: &str) -> Option<u8> {
    match engine {
        Engine::Claude => ["haiku", "sonnet", "opus", "fable"].iter().position(|f| *f == family),
        Engine::Codex => ["luna", "sol", "astra"].iter().position(|f| *f == family),
    }
    .map(|r| r as u8)
}

/// How strong `model` is on `engine`.
pub fn model_rank(engine: Engine, model: &str) -> Option<u8> {
    if model.trim_start().starts_with('@') {
        return None;
    }
    match engine {
        Engine::Claude => family_rank(engine, crate::models::family(model)?.alias),
        Engine::Codex => model.split(['-', '_', '.']).find_map(|part| family_rank(engine, part)),
    }
}

/// How hard an effort thinks, lowest first; `None` for a word equipe doesn't know.
pub fn effort_rank(effort: &str) -> Option<u8> {
    ["minimal", "low", "medium", "high", "xhigh", "max"].iter().position(|e| *e == effort.trim()).map(|r| r as u8)
}

/// A model the account's Codex lists.
#[derive(Debug, Clone, Copy)]
pub struct Offer<'a> {
    pub id: &'a str,
    pub efforts: &'a [String],
    pub default_effort: Option<&'a str>,
}

/// A member as its ticket's lane would start: its engine, the model and the
/// effort it is set to (`None`: the engine's own), and what its Codex lists.
#[derive(Debug, Clone, Copy)]
pub struct Member<'a> {
    pub engine: Engine,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub catalog: &'a [Offer<'a>],
}

impl Member<'_> {
    fn offer(&self, model: &str) -> Option<&Offer<'_>> {
        self.catalog.iter().find(|o| o.id == model)
    }

    /// How hard it thinks: its effort, else its engine's default for its model.
    fn effort_rank(&self) -> Option<u8> {
        match self.effort {
            Some(effort) => effort_rank(effort),
            None => match self.engine {
                Engine::Claude => effort_rank(CLAUDE_DEFAULT_EFFORT),
                Engine::Codex => effort_rank(self.model.and_then(|m| self.offer(m)).and_then(|o| o.default_effort).unwrap_or(CODEX_DEFAULT_EFFORT)),
            },
        }
    }

    /// The efforts `model` takes on its engine, lowest first.
    fn efforts(&self, model: &str) -> Vec<String> {
        match self.engine {
            Engine::Codex => match self.offer(model) {
                Some(offer) if !offer.efforts.is_empty() => offer.efforts.to_vec(),
                _ => crate::models::CODEX_EFFORTS.iter().map(|e| (*e).to_owned()).collect(),
            },
            Engine::Claude => crate::models::efforts(Engine::Claude, Some(model)).iter().map(|e| (*e).to_owned()).collect(),
        }
    }

    /// The model of `family` to run: Claude Code's alias; on Codex the
    /// latest the account lists, else the next stronger family it lists.
    fn model_of(&self, family: &str) -> Option<String> {
        match self.engine {
            Engine::Claude => Some(family.to_owned()),
            Engine::Codex => {
                let ids: Vec<String> = self.catalog.iter().map(|o| o.id.to_owned()).collect();
                let stronger: &[&str] = match family {
                    "luna" => &["luna", "sol", "astra"],
                    "sol" => &["sol", "astra"],
                    _ => &["astra"],
                };
                crate::roles::codex_family(&ids, stronger)
            }
        }
    }
}

/// The strongest tier `m`'s own model and effort already cover — the floor
/// a ticket never goes below without the person. `None` when equipe can't
/// tell how strong its model is.
pub fn floor(m: &Member<'_>) -> Option<ModelTier> {
    let model = model_rank(m.engine, m.model?)?;
    let effort = m.effort_rank()?;
    let covered = |t: &ModelTier| {
        let (family, effort_at) = target(*t, m.engine);
        family_rank(m.engine, family).is_some_and(|r| r <= model) && effort_rank(effort_at).is_some_and(|e| e <= effort)
    };
    Some(ModelTier::ALL.into_iter().rev().find(covered).unwrap_or(ModelTier::Light))
}

/// The model and effort `tier` runs at for `m`. Kept at the floor
/// (`floor_kept`), each is the member's own wherever that is already as
/// strong; otherwise — the person's choice — exactly the tier's.
pub fn resolve(tier: ModelTier, m: &Member<'_>, floor_kept: bool) -> (Option<String>, Option<String>) {
    let (family, effort) = target(tier, m.engine);
    let own = m.model.and_then(|model| model_rank(m.engine, model));
    let wanted = family_rank(m.engine, family);
    // The member's own model where it is as strong (or, for the person's
    // pick, the same family — its exact version kept).
    let keep_model = match (own, wanted) {
        (Some(own), Some(wanted)) if floor_kept => own >= wanted,
        (Some(own), Some(wanted)) => own == wanted,
        _ => true,
    };
    let model = if keep_model { m.model.map(str::to_owned) } else { m.model_of(family).or_else(|| m.model.map(str::to_owned)) };
    // The tier's effort, as near as this model takes it without going over.
    let offered = model.as_deref().map(|id| m.efforts(id)).unwrap_or_default();
    let want = effort_rank(effort).unwrap_or(2);
    let capped = offered.iter().filter(|e| effort_rank(e).is_some_and(|r| r <= want)).max_by_key(|e| effort_rank(e)).cloned();
    let effort = match capped {
        // A model with no efforts at all, or none this low: the engine's own.
        None => m.effort.map(str::to_owned).filter(|_| floor_kept),
        Some(capped) if floor_kept && m.effort_rank().is_some_and(|own| own >= effort_rank(&capped).unwrap_or(0)) => m.effort.map(str::to_owned),
        Some(capped) => Some(capped),
    };
    (model, effort)
}

/// What a ticket's next lane starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// What the ticket keeps and shows.
    pub ticket: TicketModel,
    /// The model and effort the lane starts with (`None`: the engine's own).
    pub model: Option<String>,
    pub effort: Option<String>,
    /// It starts a fresh conversation with a handoff note: its tier changed
    /// since its last lane, by escalation or the person.
    pub fresh: bool,
}

/// What the next lane of a ticket starts on: `prev` as the ticket keeps it,
/// `rule` as [`rule`] read it, `m` its member as it stands, under the
/// squad's `choice`.
pub fn choose(prev: Option<&TicketModel>, rule: Option<&Rule>, m: &Member<'_>, choice: ModelChoice) -> Choice {
    let prev = prev.cloned().unwrap_or_default();
    let floor = floor(m);
    let own = (m.model.map(str::to_owned), m.effort.map(str::to_owned));
    let base = TicketModel { pinned: prev.pinned, escalations: prev.escalations, risk: rule.map(|r| r.risk.clone()).unwrap_or_default(), engine: Some(m.engine), ..TicketModel::default() };
    let (ticket, (model, effort)) = match (prev.by, prev.tier) {
        // The person's own pick: theirs, even below the member's floor.
        (TierBy::Person, Some(tier)) => (TicketModel { tier: Some(tier), by: TierBy::Person, reason: TierReason::Person, ..base }, resolve(tier, m, false)),
        _ if choice == ModelChoice::Off => (TicketModel { tier: floor, reason: TierReason::MemberOwn, ..base }, own),
        _ => match (rule, floor) {
            (None, _) => (TicketModel { tier: floor, reason: TierReason::Pm, ..base }, own),
            (Some(_), None) => (TicketModel { tier: None, reason: TierReason::MemberOwn, ..base }, own),
            (Some(rule), Some(floor)) => {
                // Once raised, a ticket never comes back down mid-way.
                let raised = prev.tier.filter(|_| prev.by == TierBy::Escalation);
                // A rule that may go below the member's own has no floor.
                let floor = if rule.below_floor { ModelTier::Light } else { floor };
                let tier = rule.tier.max(floor).max(raised.unwrap_or(ModelTier::Light));
                let escalated = raised.is_some_and(|r| r >= rule.tier.max(floor));
                let reason = if escalated {
                    TierReason::Escalated
                } else if floor > rule.tier && !rule.never_lower {
                    TierReason::MemberFloor
                } else {
                    rule.reason
                };
                let shadow = (!rule.never_lower && rule.tier < tier).then_some(rule.tier);
                let by = if escalated { TierBy::Escalation } else { TierBy::Rule };
                (TicketModel { tier: Some(tier), by, reason, shadow, ..base }, resolve(tier, m, !rule.below_floor))
            }
        },
    };
    let fresh = prev.fresh;
    Choice { ticket: TicketModel { model: model.clone(), effort: effort.clone(), ..ticket }, model, effort, fresh }
}

/// What a send-back does to its ticket's model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Nothing: style and environment aren't a model's strength.
    Nothing,
    /// The first correctness send-back: the same conversation carries on, warm.
    SameLane,
    /// The second: its next lane starts one tier up, fresh, with a handoff note.
    StepUp,
    /// The third and after: the PM decides — split it, rewrite what it asks,
    /// or ask the person.
    PmDecides,
}

/// What a send-back for `reason` does, `correctness` being how many
/// correctness send-backs the ticket has had, this one included.
pub fn after_send_back(reason: SendBackReason, correctness: u32) -> Next {
    match (reason, correctness) {
        (SendBackReason::Style | SendBackReason::Environment, _) => Next::Nothing,
        (SendBackReason::Correctness, 0 | 1) => Next::SameLane,
        (SendBackReason::Correctness, 2) => Next::StepUp,
        (SendBackReason::Correctness, _) => Next::PmDecides,
    }
}

/// `t` one tier up, after its work came back wrong: fresh at its next lane,
/// with its escalations counted. A ticket the person pinned stays as it is;
/// `None` then.
pub fn step_up(t: &TicketModel) -> Option<TicketModel> {
    if t.pinned {
        return None;
    }
    let tier = t.tier.map(ModelTier::up);
    Some(TicketModel { tier, by: if tier.is_some() { TierBy::Escalation } else { t.by }, reason: TierReason::Escalated, escalations: t.escalations + 1, fresh: true, ..t.clone() })
}

/// `t` as the person sets it: `tier` for its next lanes, `pinned` against
/// escalation; no tier and not pinned gives it back to the rules.
pub fn set_by_person(t: Option<&TicketModel>, tier: Option<ModelTier>, pinned: bool) -> TicketModel {
    let t = t.cloned().unwrap_or_default();
    match tier {
        Some(tier) => TicketModel { tier: Some(tier), by: TierBy::Person, reason: TierReason::Person, pinned, fresh: t.fresh || t.tier != Some(tier), ..t },
        None if pinned => TicketModel { pinned: true, ..t },
        None => TicketModel { by: TierBy::Rule, pinned: false, tier: if t.by == TierBy::Person { None } else { t.tier }, fresh: t.fresh || t.by == TierBy::Person, ..t },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work<'a>(role: &'a str, kind: TicketKind, title: &'a str, body: &'a str) -> Work<'a> {
        Work { role, kind, title, body, design_qa: false, qa_tier: None, edits: !matches!(role, "qa" | "security" | "data" | "pm") }
    }

    fn tier(role: &str, kind: TicketKind, title: &str, body: &str) -> (ModelTier, TierReason, bool) {
        let r = rule(&work(role, kind, title, body)).unwrap();
        (r.tier, r.reason, r.never_lower)
    }

    const CLAUDE: Member<'static> = Member { engine: Engine::Claude, model: Some("sonnet"), effort: None, catalog: &[] };

    fn codex_catalog() -> Vec<(String, Vec<String>)> {
        let e = |l: &[&str]| l.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        vec![
            ("gpt-6.1-sol".into(), e(&["low", "medium", "high", "xhigh"])),
            ("gpt-6-astra".into(), e(&["low", "medium", "high"])),
            ("gpt-6-luna".into(), e(&["minimal", "low", "medium"])),
        ]
    }

    fn offers(c: &[(String, Vec<String>)]) -> Vec<Offer<'_>> {
        c.iter().map(|(id, efforts)| Offer { id, efforts, default_effort: Some("medium") }).collect()
    }

    #[test]
    fn each_role_and_kind_starts_at_its_tier() {
        let tested = "Run `cargo test -p web` to check.";
        assert_eq!(tier("frontend", TicketKind::Code, "Add a settings page", tested), (ModelTier::Strong, TierReason::Code, false));
        assert_eq!(tier("backend", TicketKind::Code, "Fix the export endpoint", tested), (ModelTier::Strong, TierReason::Code, false));
        assert_eq!(tier("mobile", TicketKind::Code, "Add pull to refresh", ""), (ModelTier::Strong, TierReason::Code, true), "no check to run: never lowered");
        assert_eq!(tier("backend", TicketKind::Code, "Refactor the job queue", tested), (ModelTier::Deep, TierReason::Broad, true));
        assert_eq!(tier("frontend", TicketKind::Code, "Use the new token everywhere", tested), (ModelTier::Deep, TierReason::Broad, true));
        assert_eq!(tier("frontend", TicketKind::Code, "Fix a lint warning in the header", tested), (ModelTier::Standard, TierReason::Small, false));
        assert_eq!(tier("frontend", TicketKind::Code, "Fix a lint warning in the header", "Just fix it."), (ModelTier::Strong, TierReason::Code, true), "small without a check: as any code");
        assert_eq!(tier("backend", TicketKind::Code, "Update the README", ""), (ModelTier::Standard, TierReason::Docs, true));
        assert_eq!(tier("marketing", TicketKind::Task, "Changelog for 1.4", "Check with `npm run lint:md`."), (ModelTier::Standard, TierReason::Docs, false));
        assert_eq!(tier("designer", TicketKind::Code, "New empty states", tested), (ModelTier::Deep, TierReason::Design, true));
        assert_eq!(tier("qa", TicketKind::Task, "Review T-12", tested), (ModelTier::Strong, TierReason::Review, true));
        assert_eq!(tier("security", TicketKind::Task, "Review the session token storage", ""), (ModelTier::Deep, TierReason::Review, true), "a risky review thinks hard");
        assert_eq!(tier("data", TicketKind::Task, "Weekly signups", "Verify with the dashboard query test."), (ModelTier::Light, TierReason::ReadOnly, false));
        assert_eq!(tier("data", TicketKind::Task, "Weekly signups", ""), (ModelTier::Standard, TierReason::ReadOnly, true), "never light without a check");
        assert_eq!(tier("marketing", TicketKind::Task, "Draft the launch post", ""), (ModelTier::Strong, TierReason::Task, true));
        assert_eq!(tier("marketing", TicketKind::Action, "Reply on the forum", tested), (ModelTier::Strong, TierReason::Action, true));
        assert!(rule(&work("pm", TicketKind::Task, "Plan the quarter", "")).is_none(), "the PM keeps its own model");
        let qa = rule(&Work { design_qa: true, ..work("designer", TicketKind::Task, "Design QA of T-4", "") }).unwrap();
        assert_eq!((qa.tier, qa.reason, qa.never_lower), (ModelTier::Deep, TierReason::DesignQa, true));
    }

    #[test]
    fn risk_is_read_from_words_and_paths_and_never_lowered() {
        assert_eq!(risk_tags("Fix the login redirect in src/auth/session.rs"), [RiskTag::Auth]);
        assert_eq!(risk_tags("Stripe webhook retries"), [RiskTag::Payments]);
        assert_eq!(risk_tags("Add a migration for the orders table"), [RiskTag::Migration]);
        assert_eq!(risk_tags("Rotate the leaked secret"), [RiskTag::Security]);
        assert_eq!(risk_tags("Race condition in the uploader"), [RiskTag::Concurrency]);
        assert_eq!(risk_tags("Deploy 1.4 to production"), [RiskTag::Release]);
        assert_eq!(risk_tags("Update auth.rs"), [RiskTag::Auth]);
        assert!(risk_tags("Show the author's name").is_empty(), "author is not auth");
        assert!(risk_tags("Make the button blue").is_empty());
        let r = rule(&work("frontend", TicketKind::Code, "Fix a typo on the login page", "Run `npm test`.")).unwrap();
        assert_eq!((r.tier, r.reason, r.never_lower, r.risk.as_slice()), (ModelTier::Deep, TierReason::Risk, true, &[RiskTag::Auth][..]));
    }

    #[test]
    fn tiers_map_to_each_engines_models() {
        assert_eq!(target(ModelTier::Strong, Engine::Claude), ("opus", "medium"));
        assert_eq!(target(ModelTier::Standard, Engine::Codex), ("sol", "low"));
        assert_eq!(model_rank(Engine::Claude, "claude-opus-5-5"), Some(2));
        assert_eq!(model_rank(Engine::Claude, "sonnet"), Some(1));
        assert_eq!(model_rank(Engine::Codex, "gpt-6.1-sol"), Some(1));
        assert_eq!(model_rank(Engine::Codex, "gpt-6-astra"), Some(2));
        assert_eq!(model_rank(Engine::Codex, "gpt-5.6-terra"), None);
        assert_eq!(model_rank(Engine::Claude, "@openrouter/claude-opus"), None, "a provider's model: unknown");
    }

    #[test]
    fn the_members_own_model_and_effort_are_the_floor() {
        // Sonnet at Claude Code's own effort: standard; Opus there: the top.
        assert_eq!(floor(&CLAUDE), Some(ModelTier::Standard));
        assert_eq!(floor(&Member { model: Some("opus"), ..CLAUDE }), Some(ModelTier::Max));
        assert_eq!(floor(&Member { model: Some("opus"), effort: Some("medium"), ..CLAUDE }), Some(ModelTier::Strong));
        assert_eq!(floor(&Member { model: Some("haiku"), effort: Some("low"), ..CLAUDE }), Some(ModelTier::Light));
        assert_eq!(floor(&Member { model: None, ..CLAUDE }), None);
        let c = codex_catalog();
        let catalog = offers(&c);
        let sol = Member { engine: Engine::Codex, model: Some("gpt-6.1-sol"), effort: None, catalog: &catalog };
        assert_eq!(floor(&sol), Some(ModelTier::Strong), "Sol at its default, medium");
    }

    #[test]
    fn quality_first_raises_and_never_lowers() {
        let code = rule(&work("frontend", TicketKind::Code, "Add a settings page", "Run `npm test`.")).unwrap();
        // A frontend member on Sonnet: the feature goes to Opus, at Claude Code's own effort.
        let c = choose(None, Some(&code), &CLAUDE, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.effort.as_deref()), (Some("opus"), None));
        assert_eq!((c.ticket.tier, c.ticket.reason, c.ticket.shadow), (Some(ModelTier::Strong), TierReason::Code, None));
        // A backend member on Opus at its own effort keeps it; the cheaper tier is only recorded.
        let opus = Member { model: Some("claude-opus-5-5"), ..CLAUDE };
        let c = choose(None, Some(&code), &opus, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.effort.as_deref()), (Some("claude-opus-5-5"), None), "never a weaker model or less thinking");
        assert_eq!((c.ticket.tier, c.ticket.reason, c.ticket.shadow), (Some(ModelTier::Max), TierReason::MemberFloor, Some(ModelTier::Strong)));
        // A member set to think little thinks as its tier asks.
        let low = Member { model: Some("opus"), effort: Some("low"), ..CLAUDE };
        let risky = rule(&work("backend", TicketKind::Code, "Payment retries", "Run `cargo test`.")).unwrap();
        let c = choose(None, Some(&risky), &low, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.effort.as_deref(), c.ticket.reason), (Some("opus"), Some("high"), TierReason::Risk));
        assert_eq!(c.ticket.risk, [RiskTag::Payments]);
        assert_eq!(c.ticket.shadow, None, "risk: nothing cheaper recorded");
    }

    #[test]
    fn a_cheaper_tier_is_recorded_but_never_applied() {
        let small = rule(&work("backend", TicketKind::Code, "Fix a lint warning", "Run `cargo clippy`.")).unwrap();
        let opus = Member { model: Some("opus"), effort: Some("high"), ..CLAUDE };
        let c = choose(None, Some(&small), &opus, ModelChoice::QualityFirst);
        assert_eq!(c.ticket.shadow, Some(ModelTier::Standard), "Sonnet would have done, by the rules");
        assert_eq!((c.model.as_deref(), c.effort.as_deref()), (Some("opus"), Some("high")), "but the member's own runs");
        // Never-lower cells record nothing cheaper.
        for w in [work("qa", TicketKind::Task, "Review T-3", "Run tests."), work("mobile", TicketKind::Code, "Add a tab", "")] {
            let r = rule(&w).unwrap();
            assert_eq!(choose(None, Some(&r), &opus, ModelChoice::QualityFirst).ticket.shadow, None, "{w:?}");
        }
    }

    #[test]
    fn reviews_design_qa_and_the_pm_are_never_lowered() {
        let opus = Member { model: Some("opus"), effort: Some("medium"), ..CLAUDE };
        let qa = rule(&Work { design_qa: true, ..work("designer", TicketKind::Task, "Design QA of T-4", "") }).unwrap();
        let c = choose(None, Some(&qa), &opus, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.effort.as_deref(), c.ticket.reason), (Some("opus"), Some("high"), TierReason::DesignQa));
        let review = rule(&work("qa", TicketKind::Task, "Review T-9", "")).unwrap();
        let c = choose(None, Some(&review), &CLAUDE, ModelChoice::QualityFirst);
        assert_eq!(c.model.as_deref(), Some("opus"), "a review is never on a cheap model");
        // The PM: its own model and effort, whatever it is.
        let c = choose(None, None, &CLAUDE, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.effort.as_deref(), c.ticket.reason), (Some("sonnet"), None, TierReason::Pm));
    }

    #[test]
    fn design_qa_runs_at_its_policys_tier_below_the_member_unless_the_person_set_its_model() {
        let opus = Member { model: Some("opus"), effort: Some("high"), ..CLAUDE };
        let qa = |tier, below_floor| rule(&Work { design_qa: true, qa_tier: Some(QaTier { tier, below_floor }), ..work("designer", TicketKind::Task, "Design QA of T-4", "") }).unwrap();
        // Nobody but equipe set the designer's Opus: Sonnet at medium.
        let c = choose(None, Some(&qa(ModelTier::Standard, true)), &opus, ModelChoice::QualityFirst);
        assert_eq!((c.ticket.tier, c.model.as_deref(), c.effort.as_deref(), c.ticket.reason), (Some(ModelTier::Standard), Some("sonnet"), Some("medium"), TierReason::DesignQa));
        // Escalated by the policy: Opus at high.
        let c = choose(None, Some(&qa(ModelTier::Deep, true)), &opus, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.effort.as_deref()), (Some("opus"), Some("high")));
        // The person set the member's model: kept, the cheaper tier only recorded.
        let c = choose(None, Some(&qa(ModelTier::Standard, false)), &opus, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.effort.as_deref(), c.ticket.shadow), (Some("opus"), Some("high"), Some(ModelTier::Standard)));
        // The person's tier on the ticket, and off, win over the policy.
        let pinned = set_by_person(None, Some(ModelTier::Max), false);
        let c = choose(Some(&pinned), Some(&qa(ModelTier::Standard, true)), &opus, ModelChoice::QualityFirst);
        assert_eq!((c.ticket.tier, c.ticket.reason), (Some(ModelTier::Max), TierReason::Person));
        let c = choose(None, Some(&qa(ModelTier::Standard, true)), &opus, ModelChoice::Off);
        assert_eq!((c.model.as_deref(), c.ticket.reason), (Some("opus"), TierReason::MemberOwn));
        // On Codex the same step is its own engine's.
        let catalog = codex_catalog();
        let offers = offers(&catalog);
        let sol = Member { engine: Engine::Codex, model: Some("gpt-6.1-sol"), effort: Some("high"), catalog: &offers };
        let c = choose(None, Some(&qa(ModelTier::Standard, true)), &sol, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.effort.as_deref()), (Some("gpt-6.1-sol"), Some("low")));
    }

    #[test]
    fn off_is_the_members_own_model_and_unknown_models_are_left_alone() {
        let code = rule(&work("frontend", TicketKind::Code, "Add a page", "Run tests.")).unwrap();
        let c = choose(None, Some(&code), &CLAUDE, ModelChoice::Off);
        assert_eq!((c.model.as_deref(), c.effort, c.ticket.reason), (Some("sonnet"), None, TierReason::MemberOwn));
        let provider = Member { engine: Engine::Codex, model: Some("@nebius/nemotron-ultra"), effort: None, catalog: &[] };
        let c = choose(None, Some(&code), &provider, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.ticket.tier, c.ticket.reason), (Some("@nebius/nemotron-ultra"), None, TierReason::MemberOwn));
    }

    #[test]
    fn a_ticket_keeps_its_tier_when_its_member_moves_engines() {
        let risky = rule(&work("backend", TicketKind::Code, "Migrate the users table", "Run `cargo test`.")).unwrap();
        let claude = Member { model: Some("sonnet"), effort: Some("medium"), ..CLAUDE };
        let first = choose(None, Some(&risky), &claude, ModelChoice::QualityFirst);
        assert_eq!((first.ticket.tier, first.model.as_deref(), first.effort.as_deref()), (Some(ModelTier::Deep), Some("opus"), Some("high")));
        // Moved to Codex on Sol at its default: the same tier, on Codex's model.
        let c = codex_catalog();
        let catalog = offers(&c);
        let codex = Member { engine: Engine::Codex, model: Some("gpt-6.1-sol"), effort: None, catalog: &catalog };
        let moved = choose(Some(&first.ticket), Some(&risky), &codex, ModelChoice::QualityFirst);
        assert_eq!((moved.ticket.tier, moved.ticket.engine, moved.model.as_deref(), moved.effort.as_deref()), (Some(ModelTier::Deep), Some(Engine::Codex), Some("gpt-6.1-sol"), Some("high")));
        // An escalated ticket keeps its raised tier across the move too.
        let raised = step_up(&first.ticket).unwrap();
        let moved = choose(Some(&raised), Some(&risky), &codex, ModelChoice::QualityFirst);
        assert_eq!((moved.ticket.tier, moved.ticket.reason, moved.effort.as_deref()), (Some(ModelTier::Max), TierReason::Escalated, Some("xhigh")));
        // A Codex model without xhigh takes its highest below.
        let astra = Member { model: Some("gpt-6-astra"), ..codex };
        assert_eq!(resolve(ModelTier::Max, &astra, true), (Some("gpt-6-astra".into()), Some("high".into())));
        // A member on Luna is raised to the account's latest Sol.
        let luna = Member { model: Some("gpt-6-luna"), ..codex };
        assert_eq!(resolve(ModelTier::Strong, &luna, true).0.as_deref(), Some("gpt-6.1-sol"));
    }

    #[test]
    fn correctness_send_backs_climb_the_ladder_and_style_never_does() {
        assert_eq!(after_send_back(SendBackReason::Correctness, 1), Next::SameLane, "first: the same lane, warm");
        assert_eq!(after_send_back(SendBackReason::Correctness, 2), Next::StepUp, "second: one tier up, fresh");
        assert_eq!(after_send_back(SendBackReason::Correctness, 3), Next::PmDecides, "third: the PM decides");
        assert_eq!(after_send_back(SendBackReason::Correctness, 6), Next::PmDecides);
        for n in 1..5 {
            assert_eq!(after_send_back(SendBackReason::Style, n), Next::Nothing);
            assert_eq!(after_send_back(SendBackReason::Environment, n), Next::Nothing);
        }
        let code = rule(&work("frontend", TicketKind::Code, "Add a page", "Run tests.")).unwrap();
        let first = choose(None, Some(&code), &CLAUDE, ModelChoice::QualityFirst);
        // The first send-back carries on: the same model, no fresh start.
        let again = choose(Some(&first.ticket), Some(&code), &CLAUDE, ModelChoice::QualityFirst);
        assert_eq!((again.model.clone(), again.fresh), (first.model.clone(), false));
        // The second: one tier up, fresh, counted.
        let up = step_up(&again.ticket).unwrap();
        assert_eq!((up.tier, up.by, up.escalations, up.fresh), (Some(ModelTier::Deep), TierBy::Escalation, 1, true));
        let next = choose(Some(&up), Some(&code), &Member { effort: Some("medium"), ..CLAUDE }, ModelChoice::QualityFirst);
        assert_eq!((next.ticket.tier, next.ticket.reason, next.fresh, next.effort.as_deref()), (Some(ModelTier::Deep), TierReason::Escalated, true, Some("high")));
        // Pinned by the person: escalation leaves it be.
        assert_eq!(step_up(&TicketModel { pinned: true, ..up }), None);
    }

    #[test]
    fn the_person_raises_lowers_and_pins() {
        let code = rule(&work("backend", TicketKind::Code, "Add an endpoint", "Run tests.")).unwrap();
        let opus = Member { model: Some("opus"), effort: Some("high"), ..CLAUDE };
        let first = choose(None, Some(&code), &opus, ModelChoice::QualityFirst);
        // Lower: their call, below the member's own.
        let lowered = set_by_person(Some(&first.ticket), Some(ModelTier::Standard), false);
        let c = choose(Some(&lowered), Some(&code), &opus, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.effort.as_deref(), c.ticket.reason, c.fresh), (Some("sonnet"), Some("medium"), TierReason::Person, true));
        // Raise and pin: kept even when choice is off, and escalation leaves it.
        let pinned = set_by_person(Some(&c.ticket), Some(ModelTier::Max), true);
        let c = choose(Some(&pinned), Some(&code), &CLAUDE, ModelChoice::Off);
        assert_eq!((c.model.as_deref(), c.effort.as_deref(), c.ticket.pinned), (Some("opus"), Some("xhigh"), true));
        assert_eq!(step_up(&c.ticket), None);
        // Given back: the rules again.
        let auto = set_by_person(Some(&c.ticket), None, false);
        assert_eq!((auto.by, auto.pinned, auto.tier), (TierBy::Rule, false, None));
        let c = choose(Some(&auto), Some(&code), &opus, ModelChoice::QualityFirst);
        assert_eq!((c.ticket.by, c.model.as_deref()), (TierBy::Rule, Some("opus")));
    }

    #[test]
    fn nothing_here_depends_on_the_plans_or_parallel_work() {
        // `choose` takes no plan and no parallel mode: Max or a plan running
        // low can only shrink the lanes, never lower a tier.
        let code = rule(&work("frontend", TicketKind::Code, "Add a page", "Run tests.")).unwrap();
        let a = choose(None, Some(&code), &CLAUDE, ModelChoice::QualityFirst);
        let b = choose(None, Some(&code), &CLAUDE, ModelChoice::QualityFirst);
        assert_eq!(a, b);
    }
}
