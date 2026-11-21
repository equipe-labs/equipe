//! The squad's autonomy policy: what its members just do, what they decide
//! and tell the person, and what they ask first — set once by the person, so
//! the squad keeps moving without them for everything that doesn't need them.
//!
//! Three levels per kind of thing ([`Level`]): ask, decide-and-tell (the
//! member goes ahead after a short window and the person gets a one-line
//! note they can take back), and just-do. Some kinds always ask, whatever the
//! policy says: sign-ins and credentials, money, permanent deletion, sending
//! or publishing outside, security settings and legal matters — the hard
//! floor ([`Kind::is_floor`]). Nothing here can lower it.
//!
//! Beyond the levels, the person can grant a **standing allowance** once,
//! from a request's card: "test records labeled `[test]` in the ERP may be
//! created, and deleted by the member that made them within 7 days". It has a
//! scope spelled out in plain words, an expiry and a log of every use; a
//! member's later request inside it is allowed at once with a one-line note,
//! and the person can revoke it any time. Only the person's own action on
//! their screen grants, revokes or sets any of this — never a member's or the
//! PM's words.
//!
//! Plain types and pure functions, shared by the daemon and every screen.

use serde::{Deserialize, Serialize};

use crate::decide::{Answer, QuestionSet};
use crate::{SessionId, TicketId};

/// A day, in seconds.
pub const DAY: u64 = 24 * 60 * 60;

/// What a request or a question is about, for the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A choice between options a member made (a layout, a direction).
    Design,
    /// Merging reviewed work into main (the PM's acceptance).
    Merge,
    /// The squad's own process: how to file a ticket, what to name a
    /// follow-up, whether to close a duplicate — invisible in the product.
    Internal,
    /// Writing to a live system's data: records, migrations, live tests.
    ProductionData,
    /// Running a project's declared production deploy once its work landed
    /// on main (`.equipe/deploy.json`).
    Deploy,
    /// Starting the small, low-risk items of a proposed sprint without the
    /// person's tap (docs/SPRINTS.md): never marketing, never over budget.
    Sprint,
    /// Signing in, accounts and credentials: keys, tokens, passwords, sign-ups.
    SignIn,
    /// Money: payments, plans, billing, budgets.
    Money,
    /// Deleting something for good.
    Deletion,
    /// Sending messages or publishing outside the squad: email, chat, issues, posts.
    Outside,
    /// Security settings: permissions, roles, access, protection.
    Security,
    /// Legal matters: terms, licences, contracts, consent.
    Legal,
    /// Anything else: asked, and open to a standing allowance.
    Other,
}

impl Kind {
    /// The kinds the person sets a level for.
    pub const SET: [Kind; 6] = [Kind::Design, Kind::Merge, Kind::Internal, Kind::ProductionData, Kind::Deploy, Kind::Sprint];
    /// The kinds that always ask, in every squad, whatever its policy.
    pub const FLOOR: [Kind; 6] = [Kind::SignIn, Kind::Money, Kind::Deletion, Kind::Outside, Kind::Security, Kind::Legal];

    /// Whether it always asks the person: the hard floor.
    pub fn is_floor(self) -> bool {
        Self::FLOOR.contains(&self)
    }

    /// Whether a standing allowance may cover it: never the floor, and not
    /// the kinds the levels already decide.
    pub fn allowable(self) -> bool {
        matches!(self, Kind::ProductionData | Kind::Other)
    }

    /// Its name in the policy, the command line and the tools.
    pub fn code(self) -> &'static str {
        match self {
            Kind::Design => "design",
            Kind::Merge => "merge",
            Kind::Internal => "internal",
            Kind::ProductionData => "production_data",
            Kind::Deploy => "deploy",
            Kind::Sprint => "sprint",
            Kind::SignIn => "sign_in",
            Kind::Money => "money",
            Kind::Deletion => "deletion",
            Kind::Outside => "outside",
            Kind::Security => "security",
            Kind::Legal => "legal",
            Kind::Other => "other",
        }
    }

    /// The key of its name among the words people read (`autonomy.kind.…`).
    pub fn words_key(self) -> String {
        format!("autonomy.kind.{}", if self == Kind::Other { "misc" } else { self.code() })
    }

    /// Read from its code, or a word people use for it.
    pub fn parse(text: &str) -> Option<Kind> {
        let text = text.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        Some(match text.as_str() {
            "design" | "design_choice" | "design_choices" => Kind::Design,
            "merge" | "merges" | "accept" => Kind::Merge,
            "internal" | "process" => Kind::Internal,
            "production_data" | "production" | "prod" | "data" | "live_data" => Kind::ProductionData,
            "deploy" | "deploys" | "go_live" | "production_deploy" => Kind::Deploy,
            "sprint" | "sprints" | "sprint_items" => Kind::Sprint,
            "sign_in" | "signin" | "login" | "credentials" | "account" | "accounts" => Kind::SignIn,
            "money" | "payment" | "payments" | "billing" => Kind::Money,
            "deletion" | "delete" | "permanent_deletion" => Kind::Deletion,
            "outside" | "send" | "publish" | "messages" => Kind::Outside,
            "security" => Kind::Security,
            "legal" => Kind::Legal,
            "other" => Kind::Other,
            _ => return None,
        })
    }

    /// The floor kind `text`'s words name, if any — in every language equipe
    /// speaks (`match.floor.*`). A request that names one asks, whatever it
    /// says it is. `except` is left out of the search: a deletion an
    /// allowance covers still names deleting.
    pub fn floor_named(text: &str, except: Option<Kind>) -> Option<Kind> {
        let lower = text.to_lowercase();
        Self::FLOOR.into_iter().filter(|k| Some(*k) != except).find(|k| crate::words::every(&format!("match.floor.{}", k.code())).iter().any(|w| names(&lower, w)))
    }
}

/// Whether `lower` holds `word`: a word of letters without spaces in it
/// (Han, kana) anywhere; any other where it starts and ends a word, so
/// "pay" isn't found in "payload".
fn names(lower: &str, word: &str) -> bool {
    let word = word.to_lowercase();
    let (Some(first), Some(last)) = (word.chars().next(), word.chars().last()) else { return false };
    let wordy = |c: char| c.is_ascii_alphanumeric() || crate::words::spaced_letter(c);
    lower.match_indices(word.as_str()).any(|(at, _)| {
        let before = lower[..at].chars().next_back();
        let after = lower[at + word.len()..].chars().next();
        let starts = !wordy(first) || before.is_none_or(|c| !wordy(c));
        // A word in a script with spaces takes endings after it; an ASCII
        // one ends where ASCII letters do, whatever particle follows it.
        let ends = !last.is_ascii_alphanumeric() || after.is_none_or(|c| !c.is_ascii_alphanumeric());
        starts && ends
    })
}

/// How far members go on their own with a kind of thing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// The person decides first.
    #[default]
    Ask,
    /// The member decides after a short window and tells the person, who can
    /// take it back.
    #[serde(rename = "decide_and_tell", alias = "tell")]
    Tell,
    /// The member decides; it is logged.
    #[serde(rename = "just_do", alias = "do")]
    JustDo,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Ask, Level::Tell, Level::JustDo];

    pub fn code(self) -> &'static str {
        match self {
            Level::Ask => "ask",
            Level::Tell => "decide_and_tell",
            Level::JustDo => "just_do",
        }
    }

    pub fn parse(text: &str) -> Option<Level> {
        match text.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "ask" => Some(Level::Ask),
            "decide_and_tell" | "tell" => Some(Level::Tell),
            "just_do" | "do" => Some(Level::JustDo),
            _ => None,
        }
    }

    /// How much it lets members do without the person.
    pub fn reach(self) -> u8 {
        self as u8
    }
}

/// How a squad's members are let to work, over and above its [`Policy`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Autonomy {
    /// For people who are not developers (what self-serve and cloud teams
    /// start on): internal matters are the team's own — staffing, files in
    /// the workspace, builds, tests, dev servers, project dependencies,
    /// screenshots, git inside the workspace all proceed without asking.
    /// Only what reaches outward or cannot be undone still asks the person:
    /// publishing, messages to real people, spending, deleting, credentials,
    /// connecting accounts. Approvals stay the person's alone either way.
    Full,
    /// Members ask for what their mode says they ask for. The default.
    #[default]
    #[serde(other)]
    Standard,
}

impl Autonomy {
    pub fn code(self) -> &'static str {
        match self {
            Autonomy::Standard => "standard",
            Autonomy::Full => "full",
        }
    }

    pub fn parse(text: &str) -> Option<Autonomy> {
        match text.trim().to_ascii_lowercase().as_str() {
            "standard" | "default" | "off" => Some(Autonomy::Standard),
            "full" | "non_developer" | "non-developer" | "nondev" => Some(Autonomy::Full),
            _ => None,
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Autonomy::Standard
    }
}

/// How long a decide-and-tell choice waits for the person by default.
pub const WINDOW_MINUTES: u32 = 30;
/// The windows screens offer, in minutes; 0: at once.
pub const WINDOWS: [u32; 4] = [0, 30, 120, 480];

fn tell() -> Level {
    Level::Tell
}
fn just_do() -> Level {
    Level::JustDo
}
fn window() -> u32 {
    WINDOW_MINUTES
}

/// A squad's autonomy policy. Every squad starts on [`Policy::default`]:
/// design choices decided and told, merges of reviewed work, internal
/// questions and a project's declared deploy after a landing just done,
/// production data asked (see docs/AUTONOMY.md for the data behind these).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    #[serde(default = "tell")]
    pub design: Level,
    #[serde(default = "just_do")]
    pub merge: Level,
    #[serde(default = "just_do")]
    pub internal: Level,
    #[serde(default)]
    pub production_data: Level,
    /// A project's declared go-live command after its work landed: just done
    /// unless the person keeps deploys for themselves.
    #[serde(default = "just_do")]
    pub deploy: Level,
    /// A proposed sprint's small, low-risk items: asked unless the person
    /// lets them start on their own.
    #[serde(default)]
    pub sprint: Level,
    /// Minutes a decide-and-tell choice waits for the person before the
    /// member takes its recommendation; a low-impact one doesn't wait.
    #[serde(default = "window")]
    pub window_minutes: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self { design: Level::Tell, merge: Level::JustDo, internal: Level::JustDo, production_data: Level::Ask, deploy: Level::JustDo, sprint: Level::Ask, window_minutes: WINDOW_MINUTES }
    }
}

/// Why a level couldn't be set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The kind is on the hard floor (or is "other"): it always asks.
    Floor(Kind),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Floor(kind) => write!(f, "{} always asks the person; no policy changes that", kind.code()),
        }
    }
}

impl Policy {
    /// What `kind` gets: the floor and "other" always ask.
    pub fn level(&self, kind: Kind) -> Level {
        match kind {
            Kind::Design => self.design,
            Kind::Merge => self.merge,
            Kind::Internal => self.internal,
            Kind::ProductionData => self.production_data,
            Kind::Deploy => self.deploy,
            Kind::Sprint => self.sprint,
            _ => Level::Ask,
        }
    }

    /// Set `kind`'s level. The floor only ever asks: asking is accepted (it
    /// changes nothing), anything else refused.
    pub fn set(&mut self, kind: Kind, level: Level) -> Result<(), Refused> {
        let slot = match kind {
            Kind::Design => &mut self.design,
            Kind::Merge => &mut self.merge,
            Kind::Internal => &mut self.internal,
            Kind::ProductionData => &mut self.production_data,
            Kind::Deploy => &mut self.deploy,
            Kind::Sprint => &mut self.sprint,
            _ if level == Level::Ask => return Ok(()),
            _ => return Err(Refused::Floor(kind)),
        };
        *slot = level;
        Ok(())
    }

    /// Whether setting `kind` to `level` lets members do more without the
    /// person than now: then only the person's tap may do it.
    pub fn widens(&self, kind: Kind, level: Level) -> bool {
        level.reach() > self.level(kind).reach()
    }

    /// The window as set, at most a day.
    pub fn window_secs(&self) -> u64 {
        u64::from(self.window_minutes.min(24 * 60)) * 60
    }
}

/// What a request for permission leads its card with: three short lines in
/// the person's own language — what will change, who sees it, how it is
/// undone. The member's long technical text folds away under them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plain {
    pub changes: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub who_sees: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub undo: String,
}

impl Plain {
    pub fn is_empty(&self) -> bool {
        self.changes.trim().is_empty()
    }
}

/// What a write to a live system does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Create,
    Update,
    Delete,
}

impl Op {
    pub fn parse(text: &str) -> Option<Op> {
        match text.trim().to_ascii_lowercase().as_str() {
            "create" | "add" | "insert" | "new" => Some(Op::Create),
            "update" | "change" | "edit" | "modify" => Some(Op::Update),
            "delete" | "remove" => Some(Op::Delete),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Op::Create => "create",
            Op::Update => "update",
            Op::Delete => "delete",
        }
    }
}

/// What a standing allowance covers. Either an engine rule (`Bash(npm
/// test:*)`, as "always allow" keeps them) or a labeled write: requests on
/// `system` whose words carry `marker`, doing one of `ops` — a delete only of
/// what the same member made under it within `undo_days`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    /// The scope in plain words, as the person saw it when they granted it.
    pub words: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ops: Vec<Op>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub undo_days: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Scope {
    /// Whether it can stand: a rule, or a labeled write with a system, a
    /// marker of a few characters and at least one op — never "anything".
    pub fn is_sound(&self) -> bool {
        if self.rule.as_deref().is_some_and(|r| !r.trim().is_empty()) {
            return true;
        }
        let marker = self.marker.as_deref().map(str::trim).unwrap_or_default();
        self.system.as_deref().is_some_and(|s| !s.trim().is_empty()) && marker.chars().count() >= 3 && !self.ops.is_empty()
    }
}

/// What the daemon adds to a request for permission for the policy: its
/// kind, its plain words, and what "Allow for this kind" would grant (none:
/// the button isn't offered — the floor, or nothing sound to generalise to).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ask {
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plain: Option<Plain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing: Option<Scope>,
    /// The member didn't wait: it works on other things, and the answer
    /// reaches it as a message.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub later: bool,
    /// What a write to a live system does, as the member said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<Op>,
}

/// The expiries the card offers for a standing allowance, in days.
pub const EXPIRY_DAYS: [u32; 3] = [1, 7, 30];
/// The one it starts on.
pub const DEFAULT_DAYS: u32 = 7;
/// The longest a standing allowance lasts.
pub const MOST_DAYS: u32 = 90;

/// A standing allowance the person granted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allowance {
    pub id: u64,
    pub kind: Kind,
    pub scope: Scope,
    /// The member whose request it was granted on.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from: String,
    pub granted_at: u64,
    pub expires_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<u64>,
    /// How many requests it has allowed.
    #[serde(default)]
    pub uses: u32,
}

impl Allowance {
    /// Whether it still allows: not revoked, not expired, and never on the floor.
    pub fn live(&self, now: u64) -> bool {
        self.revoked_at.is_none() && now < self.expires_at && self.kind.allowable() && self.scope.is_sound()
    }
}

/// One request a standing allowance allowed, for its audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Use {
    pub allowance: u64,
    pub member: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<Op>,
    pub what: String,
    pub at: u64,
}

/// How many uses the log keeps.
pub const USES_KEPT: usize = 500;

/// The recommended answers to a set, one per question: its recommended
/// option. `None` when any question has none to take — no options, none
/// recommended, a taste board — so the person's answer is needed.
pub fn recommended_answers(set: &QuestionSet) -> Option<Vec<Answer>> {
    set.questions
        .iter()
        .map(|q| {
            if q.board || q.options.is_empty() {
                return None;
            }
            let picked: Vec<usize> = q.options.iter().enumerate().filter(|(_, c)| c.shown().1).map(|(n, _)| n).collect();
            match picked.as_slice() {
                [] => None,
                [one, ..] if !q.multi_select => Some(Answer::pick(*one)),
                _ => Some(Answer { picked, ..Answer::default() }),
            }
        })
        .collect()
}

/// A choice a member made itself under decide-and-tell, kept for a week so
/// the person can change it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoPick {
    pub set: QuestionSet,
    pub answers: Vec<Answer>,
    pub kind: Kind,
    pub at: u64,
    /// The person opened it again to choose themselves.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reopened: bool,
}

/// How long an automatic pick can be changed.
pub const PICK_KEPT: u64 = 7 * DAY;

/// An automatic pick as screens show it: one line, and "change".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickView {
    pub set: u64,
    pub session: SessionId,
    pub member: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    pub question: String,
    pub picked: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    pub at: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reopened: bool,
}

impl AutoPick {
    /// Its one line: the first question, the option taken and why (the option's own line).
    pub fn view(&self) -> PickView {
        let q = self.set.questions.first();
        let picked = self.answers.first().and_then(|a| a.picked.first()).and_then(|&n| q.and_then(|q| q.options.get(n)));
        PickView {
            set: self.set.id,
            session: self.set.session,
            member: self.set.member.clone(),
            ticket: self.set.ticket,
            question: q.map(|q| q.question.clone()).unwrap_or_default(),
            picked: picked.map(|c| c.shown().0).unwrap_or_default(),
            why: picked.map(|c| c.description.clone()).unwrap_or_default(),
            at: self.at,
            reopened: self.reopened,
        }
    }
}

/// A squad's autonomy as screens show it: the policy, the standing
/// allowances (live and past), the latest uses and automatic picks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub policy: Policy,
    #[serde(default, skip_serializing_if = "Autonomy::is_default")]
    pub mode: Autonomy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowances: Vec<Allowance>,
    /// Latest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<Use>,
    /// Latest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub picks: Vec<PickView>,
    /// Choices waiting to be taken on their member's recommendation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub due: Vec<DueView>,
}

/// A question set taken on its member's recommendation at `at` unless the
/// person answers first: its card says so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DueView {
    pub set: u64,
    pub at: u64,
}

/// When a decide-and-tell set is taken on the member's recommendation:
/// after the policy's window, or at once when it is of low impact.
pub fn due_at(asked_at: u64, policy: &Policy, low_impact: bool) -> u64 {
    if low_impact { asked_at } else { asked_at + policy.window_secs() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decide::{Choice, Question};

    #[test]
    fn the_defaults_decide_and_tell_design_just_do_merges_and_process_and_ask_for_live_data() {
        let p = Policy::default();
        assert_eq!(p.level(Kind::Design), Level::Tell);
        assert_eq!(p.level(Kind::Merge), Level::JustDo);
        assert_eq!(p.level(Kind::Internal), Level::JustDo);
        assert_eq!(p.level(Kind::ProductionData), Level::Ask);
        assert_eq!(p.level(Kind::Deploy), Level::JustDo, "a declared deploy after a landing ships at once");
        assert_eq!(p.window_minutes, 30);
        // An older squad's file with nothing in it reads as the defaults.
        assert_eq!(serde_json::from_str::<Policy>("{}").unwrap(), p);
    }

    #[test]
    fn the_floor_always_asks_and_no_policy_lowers_it() {
        let mut p = Policy::default();
        for kind in Kind::FLOOR.into_iter().chain([Kind::Other]) {
            assert_eq!(p.level(kind), Level::Ask, "{kind:?}");
            for level in [Level::Tell, Level::JustDo] {
                assert_eq!(p.set(kind, level), Err(Refused::Floor(kind)), "{kind:?} {level:?}");
            }
            assert_eq!(p.set(kind, Level::Ask), Ok(()));
            assert_eq!(kind.allowable(), kind == Kind::Other, "{kind:?}: only \"other\" may be allowed standing");
            assert_eq!(p.level(kind), Level::Ask);
        }
        assert!(Kind::FLOOR.iter().all(|k| k.is_floor()));
        assert!(Kind::SET.iter().all(|k| !k.is_floor()));
        // A level for a kind the person sets is kept, and widening is told apart.
        assert!(p.widens(Kind::ProductionData, Level::JustDo));
        p.set(Kind::ProductionData, Level::JustDo).unwrap();
        assert_eq!(p.level(Kind::ProductionData), Level::JustDo);
        assert!(!p.widens(Kind::ProductionData, Level::Ask));
        assert!(!p.widens(Kind::Money, Level::JustDo) || p.set(Kind::Money, Level::JustDo).is_err());
    }

    #[test]
    fn levels_and_kinds_read_back_from_their_codes_and_peoples_words() {
        for level in Level::ALL {
            assert_eq!(Level::parse(level.code()), Some(level));
            assert_eq!(serde_json::from_value::<Level>(serde_json::json!(level.code())).unwrap(), level);
        }
        assert_eq!(Level::parse("decide-and-tell"), Some(Level::Tell));
        assert_eq!(Level::parse("just do"), Some(Level::JustDo));
        for kind in Kind::SET.into_iter().chain(Kind::FLOOR).chain([Kind::Other]) {
            assert_eq!(Kind::parse(kind.code()), Some(kind));
        }
        assert_eq!(Kind::parse("production"), Some(Kind::ProductionData));
        assert_eq!(Kind::parse("sign-in"), Some(Kind::SignIn));
        assert_eq!(Kind::parse("go live"), Some(Kind::Deploy));
        assert_eq!(Kind::parse("whatever"), None);
    }

    #[test]
    fn floor_words_are_found_in_every_language_and_only_as_words() {
        assert_eq!(Kind::floor_named("Create an account and accept the terms", None), Some(Kind::SignIn));
        assert_eq!(Kind::floor_named("Open the billing page and upgrade the plan", None), Some(Kind::Money));
        assert_eq!(Kind::floor_named("Post a public issue on GitHub", None), Some(Kind::Outside));
        assert_eq!(Kind::floor_named("Delete the empty Vercel project", None), Some(Kind::Deletion));
        assert_eq!(Kind::floor_named("Delete the empty Vercel project", Some(Kind::Deletion)), None, "covered by an allowance's delete");
        assert_eq!(Kind::floor_named("슬랙 DM으로 시험 메시지를 보낼게요", None), Some(Kind::Outside));
        assert_eq!(Kind::floor_named("시험 문서 두 개를 삭제합니다", None), Some(Kind::Deletion));
        assert_eq!(Kind::floor_named("Stripeで支払いを行います", None), Some(Kind::Money));
        // Inside another word, it isn't named.
        assert_eq!(Kind::floor_named("Store the payload in the draft record [test]", None), None);
        assert_eq!(Kind::floor_named("Create one unsubmitted test record titled [test] seal register", None), None);
    }

    fn scope() -> Scope {
        Scope { words: "Test records in ERP".into(), system: Some("ERP".into()), marker: Some("[test]".into()), ops: vec![Op::Create, Op::Delete], undo_days: 7, ..Scope::default() }
    }

    #[test]
    fn an_allowance_stands_only_while_live_sound_and_off_the_floor() {
        let a = Allowance { id: 1, kind: Kind::ProductionData, scope: scope(), from: "qa".into(), granted_at: 0, expires_at: 7 * DAY, revoked_at: None, uses: 0 };
        assert!(a.live(0) && a.live(7 * DAY - 1));
        assert!(!a.live(7 * DAY), "expired");
        assert!(!Allowance { revoked_at: Some(5), ..a.clone() }.live(10), "revoked");
        assert!(!Allowance { kind: Kind::Money, ..a.clone() }.live(10), "the floor is never allowed standing");
        // Never "anything": a marker too short, no system or no op isn't a scope.
        assert!(!Allowance { scope: Scope { marker: Some("t".into()), ..scope() }, ..a.clone() }.live(10));
        assert!(!Allowance { scope: Scope { system: None, ..scope() }, ..a.clone() }.live(10));
        assert!(!Allowance { scope: Scope { ops: vec![], ..scope() }, ..a.clone() }.live(10));
        assert!(Allowance { scope: Scope { words: "npm test".into(), rule: Some("Bash(npm test:*)".into()), ..Scope::default() }, kind: Kind::Other, ..a }.live(10));
    }

    fn option(label: &str, recommended: bool) -> Choice {
        Choice { label: label.into(), description: format!("why {label}"), recommended, ..Choice::default() }
    }

    fn set(questions: Vec<Question>) -> QuestionSet {
        QuestionSet { id: 7, session: 3, member: "designer".into(), ticket: Some(12), questions, asked_at: 100, review: false, retro: None }
    }

    #[test]
    fn the_recommendation_is_taken_only_where_every_question_has_one() {
        let q = |options: Vec<Choice>| Question { question: "Which layout?".into(), options, ..Question::default() };
        let s = set(vec![q(vec![option("A", false), option("B", true)])]);
        assert_eq!(recommended_answers(&s), Some(vec![Answer::pick(1)]));
        // An old label that says it counts too.
        assert_eq!(recommended_answers(&set(vec![q(vec![option("A (Recommended)", false), option("B", false)])])), Some(vec![Answer::pick(0)]));
        assert_eq!(recommended_answers(&set(vec![q(vec![option("A", false), option("B", false)])])), None, "nothing recommended");
        assert_eq!(recommended_answers(&set(vec![q(vec![])])), None, "a plain question needs the person's words");
        let board = Question { board: true, ..q(vec![option("A", true), option("B", false)]) };
        assert_eq!(recommended_answers(&set(vec![board])), None, "a taste board is the person's");
        assert_eq!(recommended_answers(&set(vec![q(vec![option("A", true), option("B", false)]), q(vec![option("C", false), option("D", false)])])), None, "one without: the person's");
        // The pick's one line: the option and its own why.
        let pick = AutoPick { set: s.clone(), answers: vec![Answer::pick(1)], kind: Kind::Design, at: 200, reopened: false };
        let view = pick.view();
        assert_eq!((view.picked.as_str(), view.why.as_str(), view.question.as_str(), view.set), ("B", "why B", "Which layout?", 7));
    }

    #[test]
    fn a_decide_and_tell_choice_is_due_after_the_window_or_at_once_when_low_impact() {
        let p = Policy::default();
        assert_eq!(due_at(1_000, &p, false), 1_000 + 30 * 60);
        assert_eq!(due_at(1_000, &p, true), 1_000);
        assert_eq!(due_at(1_000, &Policy { window_minutes: 0, ..p.clone() }, false), 1_000);
        assert_eq!(Policy { window_minutes: 100_000, ..p }.window_secs(), DAY, "at most a day");
    }

    #[test]
    fn the_state_travels_as_json_and_an_empty_one_is_the_defaults() {
        let state = State::default();
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json, serde_json::json!({ "policy": { "design": "decide_and_tell", "merge": "just_do", "internal": "just_do", "production_data": "ask", "deploy": "just_do", "sprint": "ask", "window_minutes": 30 } }));
        assert_eq!(serde_json::from_value::<State>(json).unwrap(), state);
    }
}
