//! Which model and effort a ticket's run lanes get: the rules, without a
//! daemon. A lane picks its tier once, when it starts — never mid-session,
//! where a model change would throw the prompt cache away — and a ticket
//! keeps its tier when its member moves engines: each engine maps the same
//! step to a model and effort of its own ([`target`]).
//!
//! On the squad's default ([`ModelChoice::QualityFirst`]) a ticket is
//! *right-sized* by how hard it looks: the cheapest tier that does it well
//! ([`rule`]), with escalation as the safety net — work that comes back
//! wrong runs one tier up ([`after_send_back`], [`step_up`]). Nothing here
//! reads the plans: when plan room runs short the lanes shrink
//! (`crate::lanes`), and tiers never drop for it — on Max parallel work too.
//! What a member's own model is does not matter, except where the person
//! chose it ([`Member::person`]): that is a floor, and only the person goes
//! below it. A model that an automatic engine move left on a member is not
//! a choice, so it never forces a lane up. What a cheaper tier would have
//! been is recorded ([`TicketModel::shadow`]) where a ticket runs above its
//! rule, and the rule's own reason is kept beside the shown one
//! ([`TicketModel::classified`]), so the effect can be measured.
//!
//! | Ticket | Tier | Claude | Codex |
//! |---|---|---|---|
//! | PM work | the PM's own | — | — |
//! | Risk tag (auth, payments, migration, security, concurrency, release) | deep | Opus · high | Sol · high |
//! | Broad: a refactor, one change in many places | deep | Opus · high | Sol · high |
//! | Design QA (squad policy `economy`, the default; `crate::design_qa::tier_for`) | standard; deep for a redesign, round 2+, or a precheck that needs judgment | Sonnet · medium (Opus · high) | Sol · low (Sol · high) |
//! | QA, security review | standard for a narrow checklist or small diff (QA); else strong; deep with a risk tag | Sonnet · medium / Opus · medium | Sol · low / Sol · medium |
//! | Design work | strong; standard for a small one | Opus · medium | Sol · medium |
//! | Code: a feature, an unclear spec, or several areas | strong | Opus · medium | Sol · medium |
//! | Code: routine — one area, small diff, clear acceptance criteria | standard | Sonnet · medium | Sol · low |
//! | Tests added, chores (deps, config), copy, i18n, docs, changelog | standard | Sonnet · medium | Sol · low |
//! | Read-only research, logs, audits | standard | Sonnet · medium | Sol · low |
//! | Trivial and mechanical (a rename, formatting, a link, a status check) with a check to run | light | Haiku · medium | Luna · medium |
//! | Other tasks | strong | Opus · medium | Sol · medium |
//! | Actions in an app or the browser | strong | Opus · medium | Sol · medium |
//!
//! Max is reached only by escalation or the person. The person sets a
//! ticket's tier whenever they like ([`TierBy::Person`]; `SetTicketTier`),
//! lower than anything included, and pins it against escalation.
//!
//! When work comes back wrong the ticket's next lane starts one tier up,
//! fresh, with a handoff note: a correctness send-back (a failed check or a
//! failed design QA included, as they send the work back), a lane caught
//! looping, or a member that hands its ticket back as something it cannot do
//! or ends its turns twice with no hand-in ([`after_send_back`]). The second
//! correctness send-back goes up once more; the third goes to the PM to
//! decide. Style and environment send-backs raise nothing.

use equipe_protocol::Engine;
use equipe_protocol::TicketKind;
use equipe_protocol::tiers::{ModelChoice, ModelTier, RiskTag, SendBackReason, TicketModel, TierBy, TierReason};

/// The roles whose work is reviewing someone else's.
const REVIEWERS: [&str; 2] = ["qa", crate::roles::SECURITY];

/// Claude Code's own effort when none is given: `xhigh` on its current models.
/// A member set to no effort is taken to run there, so a member whose model
/// the person set never runs on less than that.
pub const CLAUDE_DEFAULT_EFFORT: &str = "xhigh";
/// Codex's effort when none is given and its list doesn't say.
pub const CODEX_DEFAULT_EFFORT: &str = "medium";

/// Identical tool calls in a row that make a lane look caught in a loop.
pub const LOOP_REPEATS: u32 = 8;

/// A ticket body longer than this many characters is not a small, clearly
/// specified change.
const ROUTINE_BODY_CHARS: usize = 800;

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
    let at = |tier, reason, never_lower: bool| Rule { tier, reason, risk: risk.clone(), never_lower: never_lower || !risk.is_empty(), below_floor: false };
    Some(if let (true, Some(q)) = (w.design_qa, w.qa_tier) {
        // Kept at the member's own where the person set it, the policy's tier recorded beside it.
        Rule { tier: q.tier, reason: TierReason::DesignQa, risk: risk.clone(), never_lower: q.below_floor, below_floor: q.below_floor }
    } else if w.design_qa {
        // A checklist over what changed on screen; a redesign or a second round goes up by the policy or by escalation.
        at(ModelTier::Standard, TierReason::DesignQa, false)
    } else if REVIEWERS.contains(&w.role) {
        let routine = w.role != crate::roles::SECURITY && narrow(&text) && (checked || checklist(&text));
        let tier = if !risk.is_empty() {
            ModelTier::Deep
        } else if routine {
            ModelTier::Standard
        } else {
            ModelTier::Strong
        };
        at(tier, TierReason::Review, w.role == crate::roles::SECURITY)
    } else if trivial(w.title) && risk.is_empty() && !wide_scope(&text) && !read_only(w) {
        // Mechanical, and a checker catches what goes wrong: the cheapest model when there is one.
        at(if checked { ModelTier::Light } else { ModelTier::Standard }, TierReason::Small, false)
    } else if docs(w.title) {
        // Words and docs carry no risk of their own, whatever they are about.
        at(ModelTier::Standard, TierReason::Docs, false)
    } else if !risk.is_empty() {
        at(ModelTier::Deep, TierReason::Risk, true)
    } else if w.role == "designer" {
        at(if small(w.title) { ModelTier::Standard } else { ModelTier::Strong }, TierReason::Design, false)
    } else if wide_scope(&text) {
        at(ModelTier::Deep, TierReason::Broad, true)
    } else if tests_added(w.title) {
        at(ModelTier::Standard, TierReason::Small, false)
    } else if read_only(w) {
        at(if trivial(w.title) && checked { ModelTier::Light } else { ModelTier::Standard }, TierReason::ReadOnly, false)
    } else {
        match w.kind {
            TicketKind::Code if routine_code(w, &text, checked) => at(ModelTier::Standard, TierReason::Small, false),
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
/// get wrong. Read in English words and paths only; a ticket it misses is
/// sized by what else it says, and escalation catches the rest.
pub fn risk_tags(text: &str) -> Vec<RiskTag> {
    const RULES: [(RiskTag, &[&str]); 6] = [
        (
            RiskTag::Auth,
            &["auth/", "authentication", "authorization", "authoriz", "oauth", "login", "log in", "sign-in", "sign in", "signin", "password", "permission", "session token", "access token", "jwt", "sso", "rbac"],
        ),
        (RiskTag::Payments, &["payment", "billing", "invoice", "checkout", "stripe", "subscription", "refund", "pricing plan"]),
        (RiskTag::Migration, &["migration", "migrate", "schema change", "alter table", "drop table", "backfill", "delete data", "data deletion"]),
        (RiskTag::Security, &["security", "vulnerab", "secret", "encrypt", "decrypt", "credential", "xss", "csrf", "injection", "sandbox escape", "cve-"]),
        (RiskTag::Concurrency, &["concurren", "race condition", "deadlock", "mutex", "lock-free", "thread safety", "thread-safe"]),
        (RiskTag::Release, &["release", "deploy", "rollout", "roll out", ".github/workflows", "publish to", "version bump"]),
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

/// Whether a body states what done looks like: a check to run, or its
/// acceptance criteria in words.
fn criteria(body: &str) -> bool {
    checkable(body) || has_word(body, &["acceptance", "criteria", "done when", "expected", "steps to reproduce", "repro"])
}

/// A checklist to follow: what a QA run is asked for when it is routine.
fn checklist(text: &str) -> bool {
    has_word(text, &["checklist", "smoke test", "verify", "regression", "walk through", "walkthrough", "test plan", "tp-", "acceptance"])
}

/// A refactor, or one change made the same way in several places.
fn wide_scope(text: &str) -> bool {
    has_word(
        text,
        &["everywhere", "across the", "across all", "all call sites", "every call site", "each call site", "rename all", "rename every", "renaming", "refactor", "rewrite", "codebase-wide", "repo-wide", "throughout"],
    )
}

/// Docs, copy or translations, as the title says.
fn docs(title: &str) -> bool {
    has_word(
        title,
        &["docs", "doc ", "documentation", "readme", "changelog", "release notes", "typo", "copy", "wording", "microcopy", "i18n", "l10n", "translation", "translations", "locale", "locales"],
    )
}

/// Tests added, as the title says.
fn tests_added(title: &str) -> bool {
    has_word(
        title,
        &["add test", "add tests", "add a test", "write test", "write tests", "write a test", "tests for", "test coverage", "unit test", "unit tests", "regression test", "integration test", "e2e test"],
    )
}

/// Mechanical work a checker catches: a rename, formatting, a link, a status check.
fn trivial(title: &str) -> bool {
    has_word(
        title,
        &["rename a", "rename the", "rename this", "reformat", "formatting", "whitespace", "fix link", "fix links", "fix the link", "fix broken link", "broken link", "broken links", "dead link", "update link", "status check", "check the status"],
    )
}

/// A small change in one place, or a chore, as its title says.
fn small(title: &str) -> bool {
    has_word(
        title,
        &[
            "lint", "typo", "copy", "wording", "label", "config", "bump", "constant", "fix a warning", "warning", "tweak", "chore", "dependency", "dependencies", "deps", "minor", "small", "tiny", "one-line", "one line", "flag",
        ],
    )
}

/// A small change, a chore or words, as a title says: what design QA of it
/// needs no top tier for, even without a precheck report.
pub fn small_change(title: &str) -> bool {
    small(title) || docs(title)
}

/// New behaviour as the title says: a feature, whatever its size.
fn feature(title: &str) -> bool {
    has_word(title, &["add", "implement", "build", "create", "introduce", "support", "new ", "redesign", "integrate", "port ", "overhaul", "rework", "enable"])
}

/// What areas of the repository `text` names by path: the first two parts
/// (`crates/core`, `src/auth`) of every file path in it, once each.
fn areas(text: &str) -> usize {
    let mut seen: Vec<String> = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | '(' | ')' | ',' | ';' | '[' | ']')) {
        let token = token.trim_end_matches(['.', ':']);
        if token.contains("://") || !token.contains('/') {
            continue;
        }
        let parts: Vec<&str> = token.split('/').filter(|p| !p.is_empty()).collect();
        let file = parts.last().is_some_and(|last| last.contains('.')) || token.ends_with('/');
        if parts.len() < 2 || !file {
            continue;
        }
        let key = if parts.len() == 2 { parts[0].to_owned() } else { format!("{}/{}", parts[0], parts[1]) };
        if !seen.contains(&key) {
            seen.push(key);
        }
    }
    seen.len()
}

/// One area and no word of several: a footprint that stays in one place.
fn narrow(text: &str) -> bool {
    areas(text) <= 1
        && !has_word(text, &["across", "several", "multiple", "various", "end-to-end", "end to end", "full-stack", "full stack", "cross-cutting", "all platforms", "all screens", "frontend and backend", "backend and frontend"])
}

/// A routine code change: one area, a small diff expected, and its acceptance
/// criteria clear — a small change its title names with a check to run, or a
/// short ticket that is not new behaviour and says what done looks like.
fn routine_code(w: &Work<'_>, text: &str, checked: bool) -> bool {
    if !narrow(text) {
        return false;
    }
    if small(w.title) {
        return checked;
    }
    !feature(w.title) && criteria(w.body) && w.body.chars().count() <= ROUTINE_BODY_CHARS
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
    /// The person chose this model (`/model`, or a model for the member's
    /// role): it is the floor of every lane. `false`: it is where equipe
    /// left the member — a role's default, an automatic move to the other
    /// engine — and says nothing about how hard a ticket is.
    pub person: bool,
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
        // No model set: the tier's own. One equipe can't place: left as it is.
        _ => m.model.is_some(),
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
    // Only the person's own model is a floor. A model a provider serves is the person's by nature.
    let provider = m.model.is_some_and(|model| model.trim_start().starts_with('@'));
    let own_tier = floor(m);
    let floor = if m.person { own_tier } else if provider { None } else { Some(ModelTier::Light) };
    let own = (m.model.map(str::to_owned), m.effort.map(str::to_owned));
    let base = TicketModel {
        pinned: prev.pinned,
        escalations: prev.escalations,
        risk: rule.map(|r| r.risk.clone()).unwrap_or_default(),
        engine: Some(m.engine),
        classified: rule.map(|r| r.reason),
        ..TicketModel::default()
    };
    let (ticket, (model, effort)) = match (prev.by, prev.tier) {
        // The person's own pick: theirs, even below the member's floor.
        (TierBy::Person, Some(tier)) => (TicketModel { tier: Some(tier), by: TierBy::Person, reason: TierReason::Person, ..base }, resolve(tier, m, false)),
        _ if choice == ModelChoice::Off => (TicketModel { tier: own_tier, reason: TierReason::MemberOwn, ..base }, own),
        _ => match (rule, floor) {
            (None, _) => (TicketModel { tier: own_tier, reason: TierReason::Pm, ..base }, own),
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
                // Kept at the member's own only where the person chose it.
                (TicketModel { tier: Some(tier), by, reason, shadow, ..base }, resolve(tier, m, m.person && !rule.below_floor))
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
    /// The first and second correctness send-backs: its next lane starts one
    /// tier up, fresh, with a handoff note.
    StepUp,
    /// The third and after: the PM decides — split it, rewrite what it asks,
    /// or ask the person.
    PmDecides,
}

/// What a send-back for `reason` does, `correctness` being how many
/// correctness send-backs the ticket has had, this one included. Tickets
/// start on the cheapest tier that fits, so the first one that comes back
/// wrong already goes up.
pub fn after_send_back(reason: SendBackReason, correctness: u32) -> Next {
    match (reason, correctness) {
        (SendBackReason::Style | SendBackReason::Environment, _) => Next::Nothing,
        (SendBackReason::Correctness, 0..=2) => Next::StepUp,
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

    /// A member on Sonnet that equipe left there (a role's default): nobody chose it.
    const CLAUDE: Member<'static> = Member { engine: Engine::Claude, model: Some("sonnet"), effort: None, catalog: &[], person: false };

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

    const TESTED: &str = "Run `cargo test -p web` to check.";

    #[test]
    fn routine_code_with_a_narrow_footprint_and_clear_criteria_is_standard() {
        let repro = "Steps to reproduce: export with no rows. Expected: the CSV still has its header row. Run `cargo test -p export`.";
        assert_eq!(tier("backend", TicketKind::Code, "Fix the empty export header", repro), (ModelTier::Standard, TierReason::Small, false));
        assert_eq!(tier("frontend", TicketKind::Code, "Fix a lint warning in the header", TESTED), (ModelTier::Standard, TierReason::Small, false));
        let one_area = "The retry delay in crates/core/src/retry.rs is off by one. Done when `cargo test -p equipe-core retry` passes.";
        assert_eq!(tier("backend", TicketKind::Code, "Fix the retry delay", one_area), (ModelTier::Standard, TierReason::Small, false));
        // Chores: a dependency bump, a config tweak.
        assert_eq!(tier("backend", TicketKind::Code, "Bump the serde dependency", TESTED), (ModelTier::Standard, TierReason::Small, false));
        assert_eq!(tier("backend", TicketKind::Code, "Tweak the config value for retries", TESTED), (ModelTier::Standard, TierReason::Small, false));
    }

    #[test]
    fn features_unclear_specs_and_several_areas_stay_strong() {
        assert_eq!(tier("frontend", TicketKind::Code, "Add a settings page", TESTED), (ModelTier::Strong, TierReason::Code, false), "a feature");
        assert_eq!(tier("mobile", TicketKind::Code, "Fix the pull to refresh", ""), (ModelTier::Strong, TierReason::Code, false), "nothing says what done is");
        assert_eq!(tier("frontend", TicketKind::Code, "Fix a lint warning in the header", "Just fix it."), (ModelTier::Strong, TierReason::Code, false), "small, but nothing checks it");
        let two = "The delay in crates/core/src/retry.rs and the label in crates/desktop/src/views/jobs.rs disagree. Run `cargo test`.";
        assert_eq!(tier("backend", TicketKind::Code, "Fix the retry delay", two).0, ModelTier::Strong, "two areas");
        let spread = "Make the export work across several screens. Run `npm test`.";
        assert_eq!(tier("frontend", TicketKind::Code, "Fix the export", spread).0, ModelTier::Strong, "several areas in words");
        let long = format!("Expected: it works. Run `cargo test`. {}", "Details. ".repeat(120));
        assert_eq!(tier("backend", TicketKind::Code, "Fix the importer", &long).0, ModelTier::Strong, "a long ticket is not a small diff");
        assert_eq!(tier("marketing", TicketKind::Task, "Draft the launch post", ""), (ModelTier::Strong, TierReason::Task, false));
        assert_eq!(tier("marketing", TicketKind::Action, "Reply on the forum", TESTED), (ModelTier::Strong, TierReason::Action, true), "acting in an app is never lowered");
    }

    #[test]
    fn copy_docs_i18n_and_added_tests_are_standard() {
        assert_eq!(tier("backend", TicketKind::Code, "Update the README", ""), (ModelTier::Standard, TierReason::Docs, false));
        assert_eq!(tier("marketing", TicketKind::Task, "Changelog for 1.4", "Check with `npm run lint:md`."), (ModelTier::Standard, TierReason::Docs, false));
        assert_eq!(tier("frontend", TicketKind::Code, "Translate the empty states (i18n)", ""), (ModelTier::Standard, TierReason::Docs, false));
        assert_eq!(tier("frontend", TicketKind::Code, "Fix the wording of the save button", ""), (ModelTier::Standard, TierReason::Docs, false));
        assert_eq!(tier("backend", TicketKind::Code, "Add tests for the export endpoint", TESTED), (ModelTier::Standard, TierReason::Small, false));
        assert_eq!(tier("backend", TicketKind::Code, "Add test coverage for the importer", ""), (ModelTier::Standard, TierReason::Small, false));
    }

    #[test]
    fn read_only_work_is_standard_and_only_trivial_mechanical_work_is_light() {
        assert_eq!(tier("data", TicketKind::Task, "Weekly signups", ""), (ModelTier::Standard, TierReason::ReadOnly, false));
        assert_eq!(tier("data", TicketKind::Task, "Weekly signups", "Verify with the dashboard query test."), (ModelTier::Standard, TierReason::ReadOnly, false), "a query is not trivial");
        assert_eq!(tier("backend", TicketKind::Task, "Investigate the slow export", ""), (ModelTier::Standard, TierReason::ReadOnly, false));
        // Light: a status check, a rename, formatting, links — with a check to run.
        assert_eq!(tier("data", TicketKind::Task, "Status check of the nightly job", "Verify with `make status`."), (ModelTier::Light, TierReason::ReadOnly, false));
        assert_eq!(tier("backend", TicketKind::Code, "Rename the helper", TESTED), (ModelTier::Light, TierReason::Small, false));
        assert_eq!(tier("frontend", TicketKind::Code, "Reformat the stylesheet", "Check with `npm run lint`."), (ModelTier::Light, TierReason::Small, false));
        assert_eq!(tier("backend", TicketKind::Code, "Fix broken links in the guide", "Check with `make links`."), (ModelTier::Light, TierReason::Small, false));
        // Without anything to check it, trivial work is standard, never light.
        assert_eq!(tier("backend", TicketKind::Code, "Rename the helper", ""), (ModelTier::Standard, TierReason::Small, false));
        // A rename everywhere is broad, not trivial.
        assert_eq!(tier("backend", TicketKind::Code, "Rename the job everywhere", TESTED).0, ModelTier::Deep);
    }

    #[test]
    fn deep_is_for_real_risk_and_broad_refactors() {
        assert_eq!(tier("backend", TicketKind::Code, "Retry failed payments", TESTED), (ModelTier::Deep, TierReason::Risk, true));
        assert_eq!(tier("backend", TicketKind::Code, "Add a migration for the orders table", TESTED), (ModelTier::Deep, TierReason::Risk, true));
        assert_eq!(tier("backend", TicketKind::Code, "Fix the race condition in the uploader", TESTED), (ModelTier::Deep, TierReason::Risk, true));
        assert_eq!(tier("backend", TicketKind::Task, "Deploy 1.4", ""), (ModelTier::Deep, TierReason::Risk, true));
        assert_eq!(tier("backend", TicketKind::Code, "Refactor the job queue", TESTED), (ModelTier::Deep, TierReason::Broad, true));
        assert_eq!(tier("frontend", TicketKind::Code, "Use the new token everywhere", TESTED), (ModelTier::Deep, TierReason::Broad, true));
        // Words and docs about risky things are still words.
        let r = rule(&work("frontend", TicketKind::Code, "Fix a typo on the login page", "Run `npm test`.")).unwrap();
        assert_eq!((r.tier, r.reason, r.risk.as_slice()), (ModelTier::Standard, TierReason::Docs, &[RiskTag::Auth][..]), "the risk is still read and shown");
        // Max is never a rule's: only escalation or the person reaches it.
        for w in [work("backend", TicketKind::Code, "Payment migration across the codebase", TESTED), work("security", TicketKind::Task, "Review the secrets storage", "")] {
            assert!(rule(&w).unwrap().tier < ModelTier::Max, "{w:?}");
        }
    }

    #[test]
    fn reviews_design_qa_design_and_the_pm() {
        // QA following a checklist or a check: standard; unclear scope: strong; risk: deep.
        assert_eq!(tier("qa", TicketKind::Task, "Review T-12", TESTED), (ModelTier::Standard, TierReason::Review, false));
        assert_eq!(tier("qa", TicketKind::Task, "Verify T-12 against its checklist", ""), (ModelTier::Standard, TierReason::Review, false));
        assert_eq!(tier("qa", TicketKind::Task, "Review T-9", ""), (ModelTier::Strong, TierReason::Review, false));
        assert_eq!(tier("qa", TicketKind::Task, "Review T-9 across several screens", TESTED).0, ModelTier::Strong, "a wide review");
        assert_eq!(tier("qa", TicketKind::Task, "Review the session token storage", TESTED), (ModelTier::Deep, TierReason::Review, true));
        // A security review is never routine.
        assert_eq!(tier("security", TicketKind::Task, "Review T-5", TESTED), (ModelTier::Strong, TierReason::Review, true));
        assert_eq!(tier("security", TicketKind::Task, "Review the session token storage", ""), (ModelTier::Deep, TierReason::Review, true));
        // Design work: strong, standard when small.
        assert_eq!(tier("designer", TicketKind::Code, "New empty states", TESTED), (ModelTier::Strong, TierReason::Design, false));
        assert_eq!(tier("designer", TicketKind::Code, "Tweak the icon label", TESTED), (ModelTier::Standard, TierReason::Design, false));
        // Design QA: a checklist over small UI changes, unless the squad's policy says higher.
        let qa = rule(&Work { design_qa: true, ..work("designer", TicketKind::Task, "Design QA of T-4", "") }).unwrap();
        assert_eq!((qa.tier, qa.reason, qa.never_lower), (ModelTier::Standard, TierReason::DesignQa, false));
        let deep = rule(&Work { design_qa: true, qa_tier: Some(QaTier { tier: ModelTier::Deep, below_floor: true }), ..work("designer", TicketKind::Task, "Design QA of T-4", "") }).unwrap();
        assert_eq!(deep.tier, ModelTier::Deep);
        assert!(rule(&work("pm", TicketKind::Task, "Plan the quarter", "")).is_none(), "the PM keeps its own model");
    }

    #[test]
    fn a_footprint_is_the_areas_its_paths_name() {
        assert_eq!(areas("Fix crates/core/src/a.rs and crates/core/tests/b.rs"), 1);
        assert_eq!(areas("Fix `crates/core/src/a.rs` and (crates/desktop/src/b.rs)."), 2);
        assert_eq!(areas("The input/output of the job and https://example.com/a/b.html"), 0, "not paths");
        assert_eq!(areas("docs/GUIDE.md"), 1);
        assert!(narrow("A fix in src/auth/session.rs"));
        assert!(!narrow("A fix in src/a/x.rs and lib/b/y.rs"));
        assert!(!narrow("Change it across the app"));
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
        assert!(risk_tags("Use an atomic design system and purge the cache in production mode").is_empty(), "words that name nothing risky");
        let r = rule(&work("frontend", TicketKind::Code, "Fix the login redirect", "Run `npm test`.")).unwrap();
        assert_eq!((r.tier, r.reason, r.never_lower, r.risk.as_slice()), (ModelTier::Deep, TierReason::Risk, true, &[RiskTag::Auth][..]));
    }

    #[test]
    fn tiers_map_to_each_engines_models() {
        assert_eq!(target(ModelTier::Strong, Engine::Claude), ("opus", "medium"));
        assert_eq!(target(ModelTier::Standard, Engine::Claude), ("sonnet", "medium"));
        assert_eq!(target(ModelTier::Light, Engine::Claude), ("haiku", "medium"));
        assert_eq!(target(ModelTier::Standard, Engine::Codex), ("sol", "low"));
        assert_eq!(model_rank(Engine::Claude, "claude-opus-5-5"), Some(2));
        assert_eq!(model_rank(Engine::Claude, "sonnet"), Some(1));
        assert_eq!(model_rank(Engine::Codex, "gpt-6.1-sol"), Some(1));
        assert_eq!(model_rank(Engine::Codex, "gpt-6-astra"), Some(2));
        assert_eq!(model_rank(Engine::Codex, "gpt-5.6-terra"), None);
        assert_eq!(model_rank(Engine::Claude, "@openrouter/claude-opus"), None, "a provider's model: unknown");
    }

    #[test]
    fn the_members_own_model_and_effort_cover_the_tiers_they_reach() {
        // Sonnet at Claude Code's own effort: standard; Opus there: the top.
        assert_eq!(floor(&CLAUDE), Some(ModelTier::Standard));
        assert_eq!(floor(&Member { model: Some("opus"), ..CLAUDE }), Some(ModelTier::Max));
        assert_eq!(floor(&Member { model: Some("opus"), effort: Some("medium"), ..CLAUDE }), Some(ModelTier::Strong));
        assert_eq!(floor(&Member { model: Some("haiku"), effort: Some("low"), ..CLAUDE }), Some(ModelTier::Light));
        assert_eq!(floor(&Member { model: None, ..CLAUDE }), None);
        let c = codex_catalog();
        let catalog = offers(&c);
        let sol = Member { engine: Engine::Codex, model: Some("gpt-6.1-sol"), effort: None, catalog: &catalog, person: true };
        assert_eq!(floor(&sol), Some(ModelTier::Strong), "Sol at its default, medium");
    }

    #[test]
    fn a_ticket_runs_on_its_own_tier_whatever_model_its_member_came_up_on() {
        let routine = rule(&work("backend", TicketKind::Code, "Bump the serde dependency", TESTED)).unwrap();
        // A member that came up on Opus after an automatic move (nobody chose it): Sonnet, medium.
        let opus = Member { model: Some("claude-opus-5-5"), ..CLAUDE };
        let c = choose(None, Some(&routine), &opus, ModelChoice::QualityFirst);
        assert_eq!((c.ticket.tier, c.ticket.reason, c.ticket.shadow, c.ticket.by), (Some(ModelTier::Standard), TierReason::Small, None, TierBy::Rule));
        assert_eq!((c.model.as_deref(), c.effort.as_deref()), (Some("sonnet"), Some("medium")));
        assert_eq!(c.ticket.classified, Some(TierReason::Small));
        // A member with no model at all gets the tier's, not the engine's default.
        let none = Member { model: None, ..CLAUDE };
        let c = choose(None, Some(&routine), &none, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.effort.as_deref()), (Some("sonnet"), Some("medium")));
        // A feature on a Sonnet member: Opus at medium, not the engine's own xhigh.
        let feature = rule(&work("frontend", TicketKind::Code, "Add a settings page", TESTED)).unwrap();
        let c = choose(None, Some(&feature), &CLAUDE, ModelChoice::QualityFirst);
        assert_eq!((c.ticket.tier, c.model.as_deref(), c.effort.as_deref()), (Some(ModelTier::Strong), Some("opus"), Some("medium")));
        // Risk: Opus at high; nothing cheaper recorded.
        let risky = rule(&work("backend", TicketKind::Code, "Payment retries", TESTED)).unwrap();
        let c = choose(None, Some(&risky), &opus, ModelChoice::QualityFirst);
        assert_eq!((c.ticket.tier, c.model.as_deref(), c.effort.as_deref(), c.ticket.reason), (Some(ModelTier::Deep), Some("claude-opus-5-5"), Some("high"), TierReason::Risk));
        assert_eq!((c.ticket.risk.as_slice(), c.ticket.shadow), (&[RiskTag::Payments][..], None));
        // The PM keeps its own model.
        let c = choose(None, None, &opus, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.ticket.reason), (Some("claude-opus-5-5"), TierReason::Pm));
        // A provider's model is the person's: left alone.
        let provider = Member { engine: Engine::Codex, model: Some("@nebius/nemotron-ultra"), effort: None, catalog: &[], person: false };
        let c = choose(None, Some(&routine), &provider, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.ticket.tier, c.ticket.reason), (Some("@nebius/nemotron-ultra"), None, TierReason::MemberOwn));
    }

    #[test]
    fn only_a_model_the_person_chose_is_a_floor() {
        let routine = rule(&work("backend", TicketKind::Code, "Bump the serde dependency", TESTED)).unwrap();
        let opus = Member { model: Some("claude-opus-5-5"), effort: Some("high"), person: true, ..CLAUDE };
        let c = choose(None, Some(&routine), &opus, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.effort.as_deref()), (Some("claude-opus-5-5"), Some("high")), "never a weaker model or less thinking than the person chose");
        assert_eq!((c.ticket.tier, c.ticket.reason, c.ticket.shadow), (Some(ModelTier::Deep), TierReason::MemberFloor, Some(ModelTier::Standard)), "the rule's tier recorded beside it");
        assert_eq!(c.ticket.classified, Some(TierReason::Small), "what the rules made of it is kept");
        // A person's Opus at Claude Code's own effort: the top.
        let c = choose(None, Some(&routine), &Member { effort: None, ..opus }, ModelChoice::QualityFirst);
        assert_eq!((c.ticket.tier, c.effort), (Some(ModelTier::Max), None));
        // A person's Sonnet: a feature still goes to Opus.
        let feature = rule(&work("frontend", TicketKind::Code, "Add a settings page", TESTED)).unwrap();
        let c = choose(None, Some(&feature), &Member { person: true, ..CLAUDE }, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.effort.as_deref()), (Some("opus"), None), "raised, at the member's own effort");
    }

    #[test]
    fn off_is_the_members_own_model_and_unknown_models_are_left_alone() {
        let code = rule(&work("frontend", TicketKind::Code, "Add a page", "Run tests.")).unwrap();
        let c = choose(None, Some(&code), &CLAUDE, ModelChoice::Off);
        assert_eq!((c.model.as_deref(), c.effort, c.ticket.reason), (Some("sonnet"), None, TierReason::MemberOwn));
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
        let person = Member { person: true, ..opus };
        let c = choose(None, Some(&qa(ModelTier::Standard, false)), &person, ModelChoice::QualityFirst);
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
        let sol = Member { engine: Engine::Codex, model: Some("gpt-6.1-sol"), effort: Some("high"), catalog: &offers, person: false };
        let c = choose(None, Some(&qa(ModelTier::Standard, true)), &sol, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.effort.as_deref()), (Some("gpt-6.1-sol"), Some("low")));
    }

    #[test]
    fn a_ticket_keeps_its_tier_when_its_member_moves_engines() {
        let risky = rule(&work("backend", TicketKind::Code, "Migrate the users table", TESTED)).unwrap();
        let first = choose(None, Some(&risky), &CLAUDE, ModelChoice::QualityFirst);
        assert_eq!((first.ticket.tier, first.model.as_deref(), first.effort.as_deref()), (Some(ModelTier::Deep), Some("opus"), Some("high")));
        // Moved to Codex on Sol at its default: the same tier, on Codex's model.
        let c = codex_catalog();
        let catalog = offers(&c);
        let codex = Member { engine: Engine::Codex, model: Some("gpt-6.1-sol"), effort: None, catalog: &catalog, person: false };
        let moved = choose(Some(&first.ticket), Some(&risky), &codex, ModelChoice::QualityFirst);
        assert_eq!((moved.ticket.tier, moved.ticket.engine, moved.model.as_deref(), moved.effort.as_deref()), (Some(ModelTier::Deep), Some(Engine::Codex), Some("gpt-6.1-sol"), Some("high")));
        // An escalated ticket keeps its raised tier across the move too.
        let raised = step_up(&first.ticket).unwrap();
        let moved = choose(Some(&raised), Some(&risky), &codex, ModelChoice::QualityFirst);
        assert_eq!((moved.ticket.tier, moved.ticket.reason, moved.effort.as_deref()), (Some(ModelTier::Max), TierReason::Escalated, Some("xhigh")));
        // A Codex model without xhigh takes its highest below.
        let astra = Member { model: Some("gpt-6-astra"), ..codex };
        assert_eq!(resolve(ModelTier::Max, &astra, true), (Some("gpt-6-astra".into()), Some("high".into())));
        // A person's member on Luna is raised to the account's latest Sol.
        let luna = Member { model: Some("gpt-6-luna"), person: true, ..codex };
        assert_eq!(resolve(ModelTier::Strong, &luna, true).0.as_deref(), Some("gpt-6.1-sol"));
        // A member on Opus after a move to Claude is Sonnet for routine work, not Opus.
        let routine = rule(&work("backend", TicketKind::Code, "Bump the serde dependency", TESTED)).unwrap();
        let back = Member { model: Some("opus"), effort: None, ..CLAUDE };
        assert_eq!(choose(None, Some(&routine), &back, ModelChoice::QualityFirst).model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn a_correctness_send_back_climbs_a_tier_and_style_never_does() {
        assert_eq!(after_send_back(SendBackReason::Correctness, 1), Next::StepUp, "first: one tier up, fresh");
        assert_eq!(after_send_back(SendBackReason::Correctness, 2), Next::StepUp, "second: one more");
        assert_eq!(after_send_back(SendBackReason::Correctness, 3), Next::PmDecides, "third: the PM decides");
        assert_eq!(after_send_back(SendBackReason::Correctness, 6), Next::PmDecides);
        for n in 1..5 {
            assert_eq!(after_send_back(SendBackReason::Style, n), Next::Nothing);
            assert_eq!(after_send_back(SendBackReason::Environment, n), Next::Nothing);
        }
    }

    #[test]
    fn escalation_is_the_safety_net_of_a_cheap_start() {
        let routine = rule(&work("backend", TicketKind::Code, "Bump the serde dependency", TESTED)).unwrap();
        let first = choose(None, Some(&routine), &CLAUDE, ModelChoice::QualityFirst);
        assert_eq!((first.ticket.tier, first.model.as_deref(), first.effort.as_deref()), (Some(ModelTier::Standard), Some("sonnet"), Some("medium")));
        // Back wrong once: the next lane is Opus at medium, fresh, counted.
        let up = step_up(&first.ticket).unwrap();
        assert_eq!((up.tier, up.by, up.reason, up.escalations, up.fresh), (Some(ModelTier::Strong), TierBy::Escalation, TierReason::Escalated, 1, true));
        let next = choose(Some(&up), Some(&routine), &CLAUDE, ModelChoice::QualityFirst);
        assert_eq!((next.ticket.tier, next.ticket.reason, next.fresh, next.model.as_deref(), next.effort.as_deref()), (Some(ModelTier::Strong), TierReason::Escalated, true, Some("opus"), Some("medium")));
        assert_eq!(next.ticket.classified, Some(TierReason::Small), "what the rules made of it is still kept");
        assert_eq!(next.ticket.shadow, Some(ModelTier::Standard), "and what they asked");
        // Again: Deep, never back down on a later lane.
        let again = step_up(&next.ticket).unwrap();
        let third = choose(Some(&again), Some(&routine), &CLAUDE, ModelChoice::QualityFirst);
        assert_eq!((third.ticket.tier, third.effort.as_deref()), (Some(ModelTier::Deep), Some("high")));
        let later = choose(Some(&third.ticket), Some(&routine), &CLAUDE, ModelChoice::QualityFirst);
        assert_eq!(later.ticket.tier, Some(ModelTier::Deep), "a raised ticket never comes back down mid-way");
        // Trivial work escalates to Standard first.
        let light = rule(&work("backend", TicketKind::Code, "Rename the helper", TESTED)).unwrap();
        let c = choose(None, Some(&light), &CLAUDE, ModelChoice::QualityFirst);
        assert_eq!((c.ticket.tier, c.model.as_deref()), (Some(ModelTier::Light), Some("haiku")));
        assert_eq!(step_up(&c.ticket).unwrap().tier, Some(ModelTier::Standard));
        // Pinned by the person: escalation leaves it be.
        assert_eq!(step_up(&TicketModel { pinned: true, ..up }), None);
        // On a member the person set to Opus, a lane never goes under it — escalation or not.
        let person = Member { model: Some("opus"), effort: Some("medium"), person: true, ..CLAUDE };
        let c = choose(None, Some(&routine), &person, ModelChoice::QualityFirst);
        assert_eq!((c.ticket.tier, c.model.as_deref()), (Some(ModelTier::Strong), Some("opus")));
    }

    #[test]
    fn the_person_raises_lowers_and_pins() {
        let code = rule(&work("backend", TicketKind::Code, "Add an endpoint", "Run tests.")).unwrap();
        let first = choose(None, Some(&code), &CLAUDE, ModelChoice::QualityFirst);
        // Lower: their call, below what the rules ask.
        let lowered = set_by_person(Some(&first.ticket), Some(ModelTier::Light), false);
        let c = choose(Some(&lowered), Some(&code), &CLAUDE, ModelChoice::QualityFirst);
        assert_eq!((c.model.as_deref(), c.ticket.reason, c.fresh), (Some("haiku"), TierReason::Person, true));
        // Raise and pin: kept even when choice is off, and escalation leaves it.
        let pinned = set_by_person(Some(&c.ticket), Some(ModelTier::Max), true);
        let c = choose(Some(&pinned), Some(&code), &CLAUDE, ModelChoice::Off);
        assert_eq!((c.model.as_deref(), c.effort.as_deref(), c.ticket.pinned), (Some("opus"), Some("xhigh"), true));
        assert_eq!(step_up(&c.ticket), None);
        // Given back: the rules again.
        let auto = set_by_person(Some(&c.ticket), None, false);
        assert_eq!((auto.by, auto.pinned, auto.tier), (TierBy::Rule, false, None));
        let c = choose(Some(&auto), Some(&code), &CLAUDE, ModelChoice::QualityFirst);
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
