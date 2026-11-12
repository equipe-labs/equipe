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
    /// Its latest uses, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<Use>,
}

/// [`Login::allowed`]: every squad.
pub const EVERY_SQUAD: &str = "*";

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

    #[test]
    fn a_drafts_password_is_never_printed() {
        let draft = LoginDraft { id: None, site: "github.com".into(), username: "abel".into(), password: Some("hunter2".into()), note: None, auto: None };
        let shown = format!("{draft:?}");
        assert!(!shown.contains("hunter2") && shown.contains("•••"), "{shown}");
        let command = crate::Command::SaveLogin { draft };
        assert!(!format!("{command:?}").contains("hunter2"));
    }
}
