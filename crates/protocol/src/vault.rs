//! Logins the person keeps for members to sign in with (`equipe_core::vault`):
//! a site, a username — and a password that lives only in the daemon
//! machine's secret store (the Keychain on macOS). A member asks to sign in
//! (`sign_in`), the person says yes on a card, and equipe types it into the
//! member's browser itself: the password never reaches a member, a screen, an
//! event or a log. Where the person lets it ([`Login::auto`]), equipe also
//! signs a member's browser back in by itself when the site logs it out. What
//! screens see is [`Login`]: everything but the password.

use serde::{Deserialize, Serialize};

/// A login, as screens see it — no password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Login {
    pub id: u32,
    /// The site it signs in to, by host: `github.com`.
    pub site: String,
    /// Where its sign-in page is, when it isn't the site's home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Where members may use it without asking: a squad's name (every member
    /// and lane of it), or [`EVERY_SQUAD`]. Each from the person's own answer —
    /// a card's "always", or Settings. Entries from before, `squad/member`,
    /// are read as their squad (`equipe_core::vault` moves them on load).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<String>,
    /// The person lets equipe sign in with it by itself when a member's
    /// browser lands on its site's sign-in page (that exact host, or its
    /// sign-in page's): no card, no member asking. Only the person turns it
    /// on, on a card or in Settings — never a member's words or a page.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto: bool,
    /// Why equipe stopped signing in with it by itself: its last try didn't
    /// get past the form (a changed password, most likely). Cleared when the
    /// person saves a new password or turns signing in by itself on again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped: Option<String>,
    /// When it was saved, unix seconds.
    pub created_at: u64,
    /// When its password was last set or replaced, unix seconds (`None`: it
    /// was kept before equipe noted that).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_at: Option<u64>,
    /// Its latest uses, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<Use>,
}

/// [`Login::allowed`]: every squad.
pub const EVERY_SQUAD: &str = "*";

/// How a login stands, for the Logins page's badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// Used, and its last use didn't fail.
    Ok,
    /// The password didn't get past the form last time (or equipe stopped
    /// signing in with it): worth replacing.
    Failed,
    /// Kept, never used yet.
    NeverUsed,
}

impl Login {
    /// When it was last used, unix seconds.
    pub fn last_used(&self) -> Option<u64> {
        self.uses.first().map(|u| u.at)
    }

    /// Its standing: failed when equipe stopped signing in with it, or its
    /// latest use failed and no new password was saved since.
    pub fn health(&self) -> Health {
        let failed_last = self.uses.first().is_some_and(|u| u.outcome == "failed" && self.password_at.is_none_or(|at| u.at >= at));
        if self.stopped.is_some() || failed_last {
            Health::Failed
        } else if self.uses.is_empty() {
            Health::NeverUsed
        } else {
            Health::Ok
        }
    }

    /// Where it is used without asking, as the page names it: `*` is every squad.
    pub fn everywhere(&self) -> bool {
        self.allowed.iter().any(|w| w == EVERY_SQUAD)
    }
}

/// Why and where members sent the person to a site's sign-in, over the last
/// days — what the Logins page shows under "Asked again". Counts, hosts and
/// causes only: never a password, a code or a page's text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginInsights {
    /// How many days it looks back (7 or 30).
    pub days: u32,
    /// When it was read, unix seconds.
    pub at: u64,
    /// Every time the person was asked (handoffs and cards).
    pub asked: u64,
    /// By cause (`session_expired`, `captcha`, …), most first.
    pub by_cause: Vec<CauseCount>,
    /// Per site, most asked first.
    pub sites: Vec<SiteAsks>,
    /// Asks per day, the oldest day first, one entry per day.
    pub trend: Vec<u64>,
    /// Sign-ins finished in the period: in all, and by equipe with no one's hand.
    pub signed_in: u64,
    pub signed_in_by_equipe: u64,
    /// Sites the person said "Never for this site" to: no offer to keep a login.
    pub never: Vec<String>,
    /// Whether this machine has a secret store to keep passwords in.
    pub can_keep: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CauseCount {
    pub cause: String,
    pub count: u64,
}

/// One site's asks in the period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteAsks {
    pub site: String,
    pub asked: u64,
    pub by_cause: Vec<CauseCount>,
    /// How long a sign-in usually lasted before this site asked again (the
    /// median, seconds) — `None` when it never asked again after one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reask_after: Option<u64>,
    /// Sign-ins finished for it, and how many of them by equipe itself.
    pub signed_in: u64,
    pub signed_in_by_equipe: u64,
}

impl LoginInsights {
    /// The share of sign-ins equipe finished with no one's hand, 0–100.
    pub fn by_equipe_percent(&self) -> Option<u64> {
        (self.signed_in > 0).then(|| self.signed_in_by_equipe * 100 / self.signed_in)
    }
}

/// One time a member signed in with a login, or was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Use {
    pub at: u64,
    pub squad: String,
    pub member: String,
    /// How it went: `signed_in`, `signed_in_auto` (equipe by itself),
    /// `declined`, `no_form`, `failed`.
    pub outcome: String,
}

/// A login as the person types it in. The password goes once, to be put in
/// the Keychain; left out on a change, the one kept stays.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginDraft {
    /// `None`: a new login.
    #[serde(default)]
    pub id: Option<u32>,
    /// A host or an address: `github.com`, `https://github.com/login`.
    pub site: String,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Let equipe sign in with it by itself ([`Login::auto`]); `None`: as kept
    /// (off for a new login).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto: Option<bool>,
}

/// equipe's offer to keep a login the person just finished by hand in a
/// member's browser: once per site, on a card, never nagging. Saving goes
/// through equipe's own form ([`LoginDraft`]); "never" is remembered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginOffer {
    pub id: u64,
    /// The site signed in to, by host.
    pub site: String,
    /// Its sign-in page, as the browser was on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Whose browser it was, when a member's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub squad: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
    /// When, unix seconds.
    pub at: u64,
}

/// Never the password, whoever prints it.
impl std::fmt::Debug for LoginDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginDraft")
            .field("id", &self.id)
            .field("site", &self.site)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "•••"))
            .field("note", &self.note)
            .field("auto", &self.auto)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(auto: bool) -> Login {
        Login { id: 1, site: "github.com".into(), url: None, username: "abel".into(), note: None, allowed: Vec::new(), auto, stopped: None, created_at: 10, password_at: None, uses: Vec::new() }
    }

    fn used(at: u64, outcome: &str) -> Use {
        Use { at, squad: "acme".into(), member: "qa".into(), outcome: outcome.into() }
    }

    #[test]
    fn a_logins_health_reads_its_uses_and_its_stop() {
        let mut l = login(false);
        assert_eq!(l.health(), Health::NeverUsed);
        l.uses = vec![used(50, "signed_in")];
        assert_eq!(l.health(), Health::Ok);
        assert_eq!(l.last_used(), Some(50));
        l.uses.insert(0, used(60, "failed"));
        assert_eq!(l.health(), Health::Failed);
        // A new password saved after the failure: worth a try again, not a failure.
        l.password_at = Some(70);
        assert_eq!(l.health(), Health::Ok);
        // Declined or no form isn't a bad password.
        l.uses.insert(0, used(80, "declined"));
        assert_eq!(l.health(), Health::Ok);
        l.stopped = Some("form came back".into());
        assert_eq!(l.health(), Health::Failed);
    }

    #[test]
    fn everywhere_is_the_star_scope() {
        let mut l = login(false);
        assert!(!l.everywhere());
        l.allowed = vec!["acme".into()];
        assert!(!l.everywhere());
        l.allowed.push(EVERY_SQUAD.into());
        assert!(l.everywhere());
    }

    #[test]
    fn insights_share_is_none_without_sign_ins() {
        let mut i = LoginInsights::default();
        assert_eq!(i.by_equipe_percent(), None);
        i.signed_in = 4;
        i.signed_in_by_equipe = 3;
        assert_eq!(i.by_equipe_percent(), Some(75));
    }

    #[test]
    fn a_drafts_password_is_never_printed() {
        let draft = LoginDraft { id: None, site: "github.com".into(), username: "abel".into(), password: Some("hunter2".into()), note: None, auto: None };
        let shown = format!("{draft:?}");
        assert!(!shown.contains("hunter2") && shown.contains("•••"), "{shown}");
        let command = crate::Command::SaveLogin { draft };
        assert!(!format!("{command:?}").contains("hunter2"));
    }
}
