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
    /// What equipe may do by itself around this login's sign-in beyond
    /// typing the password — codes, a passkey of its own, keeping the
    /// session alive — each the person's to turn on ([`Autopilot`]).
    #[serde(default, skip_serializing_if = "Autopilot::is_default")]
    pub autopilot: Autopilot,
}

/// Login autopilot: what equipe may do by itself so the person signs in to a
/// site once and isn't asked again. Each switch is the person's alone (a
/// Settings control, a person-only command); nothing a member says or a
/// page shows turns one on. Every code, seed and key stays in the daemon
/// machine's secret store: screens see only whether one is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Autopilot {
    /// Type the code from the authenticator seed kept for this login (RFC 6238)
    /// on the site's own code page.
    pub totp: bool,
    /// Read the site's one-time code from the person's connected mailbox
    /// (only code-shaped text, only from the site's own sender domain).
    pub email: bool,
    /// Read the site's one-time code from the Messages the person's phone
    /// syncs to this Mac (needs their Full Disk Access grant).
    pub sms: bool,
    /// Sign in with equipe's own passkey for this site (a separate device the
    /// person can revoke on the site), registered once with them.
    pub passkey: bool,
    /// Tick "Remember me", "Keep me signed in", "Trust this device" on the
    /// forms equipe fills itself, so the site asks less often. On unless the
    /// person turns it off: it only ever touches the site's own form.
    pub remember: bool,
    /// Look at the site now and then to catch an expired sign-in early and
    /// sign back in before a member needs it.
    pub keepalive: bool,
    /// Approve device-code sign-ins (`gh auth login`, `aws sso login`, …) for
    /// this account in the shared browser, when equipe started them itself.
    pub device_code: bool,
    /// The domain the site's code mail comes from, when not its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mail_from: Option<String>,
    /// A page of the site that shows whether it is signed in (keep-alive looks at it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_url: Option<String>,
    /// An authenticator seed is kept for it (never the seed itself). Set by equipe.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub has_totp: bool,
    /// equipe's own passkey is kept for it. Set by equipe.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub has_passkey: bool,
    /// The latest keep-alive look: when, and what it found. Set by equipe.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<Use>,
}

impl Default for Autopilot {
    fn default() -> Self {
        Self { totp: false, email: false, sms: false, passkey: false, remember: true, keepalive: false, device_code: false, mail_from: None, check_url: None, has_totp: false, has_passkey: false, checked: None }
    }
}

impl Autopilot {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The person's switches from `asked`, keeping what equipe itself keeps
    /// (whether a seed or passkey is kept, the latest look): a screen's
    /// command can't claim a seed that isn't there.
    pub fn switches_from(&self, asked: &Autopilot) -> Autopilot {
        let clean = |t: &Option<String>| t.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
        Autopilot { mail_from: clean(&asked.mail_from), check_url: clean(&asked.check_url), has_totp: self.has_totp, has_passkey: self.has_passkey, checked: self.checked.clone(), ..asked.clone() }
    }
}

/// The mailbox equipe reads sites' one-time codes from (IMAP over TLS, an
/// app password the person made for it), as screens see it — no password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailbox {
    pub host: String,
    #[serde(default = "imap_port")]
    pub port: u16,
    pub username: String,
}

fn imap_port() -> u16 {
    993
}

/// A mailbox as the person types it in; the app password goes once, to the
/// secret store.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxDraft {
    pub host: String,
    #[serde(default = "imap_port")]
    pub port: u16,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

impl std::fmt::Debug for MailboxDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailboxDraft").field("host", &self.host).field("port", &self.port).field("username", &self.username).field("password", &self.password.as_ref().map(|_| "•••")).finish()
    }
}

/// A credential equipe keeps for a command-line tool, so members' commands
/// work without anyone signing in at a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliKind {
    /// A GitHub token (fine-grained) given to `gh` (`gh auth login --with-token`) and git.
    GhToken,
    /// A GitHub App: equipe mints its installation token (an hour long) and
    /// gives it to `gh`, again before it runs out.
    GhApp,
    /// An AWS access key behind an AWS CLI profile's `credential_process`.
    AwsKeys,
    /// A value members' commands get as an environment variable
    /// (`NPM_TOKEN`, `VERCEL_TOKEN`, …), at run time only.
    Env,
    /// A device-code sign-in equipe runs and approves itself in the shared
    /// browser (`gh auth login --web`, `aws sso login`, `vercel login`).
    DeviceLogin,
}

/// A command-line credential, as screens see it — no secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliLogin {
    pub id: u32,
    pub kind: CliKind,
    /// What it is for: the env variable's name, the AWS profile, the CLI
    /// (`gh`, `aws`, `vercel`) of a device-code sign-in.
    pub name: String,
    /// The host it serves: `github.com`; for a device-code sign-in, the site
    /// whose page approves it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub host: String,
    /// GitHub App: the app's id and the installation's. AWS: the access key id.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub public: String,
    /// The squad whose members get it; `None`: every squad.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub squad: Option<String>,
    pub created_at: u64,
    /// How its latest use went, in a line (never a value).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
}

/// A command-line credential as the person types it in.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliLoginDraft {
    #[serde(default)]
    pub id: Option<u32>,
    pub kind: CliKind,
    pub name: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub public: String,
    #[serde(default)]
    pub squad: Option<String>,
    /// The token, key or private key: once, to the secret store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

impl std::fmt::Debug for CliLoginDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliLoginDraft")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("host", &self.host)
            .field("squad", &self.squad)
            .field("secret", &self.secret.as_ref().map(|_| "•••"))
            .finish()
    }
}

/// Login autopilot as a whole, for the Settings card: what is connected,
/// and what it saved the person this week.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutopilotStatus {
    pub mailbox: Option<Mailbox>,
    /// The person lets equipe read codes from Messages.
    pub sms: bool,
    /// Whether equipe can read Messages now (Full Disk Access): `None` where
    /// this machine has no Messages to read.
    pub sms_access: Option<bool>,
    /// Hours between keep-alive looks at a site (jittered); 0: never.
    pub keepalive_hours: u32,
    pub cli: Vec<CliLogin>,
    /// This week: what equipe did by itself, by step (`totp`, `email`, `sms`,
    /// `passkey`, `device_code`, `keepalive`, `relogin`), …
    pub resolved: Vec<(String, u64)>,
    /// … what the person was still asked for, …
    pub asked: u64,
    /// … and roughly how many minutes of theirs that saved.
    pub minutes_saved: u64,
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
        Login { id: 1, site: "github.com".into(), url: None, username: "abel".into(), note: None, allowed: Vec::new(), auto, stopped: None, created_at: 10, password_at: None, uses: Vec::new(), autopilot: Default::default() }
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
        let mailbox = MailboxDraft { host: "imap.example.com".into(), port: 993, username: "me".into(), password: Some("app-pass".into()) };
        assert!(!format!("{mailbox:?}").contains("app-pass"));
        let cli = CliLoginDraft { id: None, kind: CliKind::GhToken, name: "gh".into(), host: "github.com".into(), public: String::new(), squad: None, secret: Some("ghp_x".into()) };
        assert!(!format!("{cli:?}").contains("ghp_x"));
        let totp = crate::Command::SetLoginTotp { id: 1, secret: Some(crate::secrets::SecretValue("JBSWY3DPEHPK3PXP".into())) };
        assert!(!format!("{totp:?}").contains("JBSWY3DP"));
    }

    #[test]
    fn a_login_from_before_has_every_autopilot_switch_off_but_remember() {
        let old: Login = serde_json::from_str(r#"{"id":1,"site":"github.com","username":"a","created_at":1}"#).unwrap();
        let a = &old.autopilot;
        assert!(!a.totp && !a.email && !a.sms && !a.passkey && !a.keepalive && !a.device_code && a.remember);
        assert!(!serde_json::to_string(&old).unwrap().contains("autopilot"), "a default autopilot isn't written");
        // A screen can't claim a seed is kept.
        let kept = Autopilot { has_totp: true, ..Autopilot::default() };
        let asked = Autopilot { totp: true, has_totp: false, has_passkey: true, mail_from: Some("  ".into()), ..Autopilot::default() };
        let now = kept.switches_from(&asked);
        assert!(now.totp && now.has_totp && !now.has_passkey && now.mail_from.is_none());
    }
}
