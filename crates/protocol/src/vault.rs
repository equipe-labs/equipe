//! Logins the person keeps for members to sign in with (`equipe_core::vault`):
//! a site, a username — and a password that lives only in the daemon
//! machine's Keychain. A member asks to sign in (`sign_in`), the person says
//! yes on a card, and equipe types it into the member's browser itself: the
//! password never reaches a member, a screen, an event or a log. What screens
//! see is [`Login`]: everything but the password.

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
    /// Members who may use it without asking (`squad/member`), each from the
    /// person's own "always" on a card.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<String>,
    /// When it was saved, unix seconds.
    pub created_at: u64,
    /// Its latest uses, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<Use>,
}

/// One time a member signed in with a login, or was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Use {
    pub at: u64,
    pub squad: String,
    pub member: String,
    /// How it went: `signed_in`, `declined`, `no_form`, `failed`.
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
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drafts_password_is_never_printed() {
        let draft = LoginDraft { id: None, site: "github.com".into(), username: "abel".into(), password: Some("hunter2".into()), note: None };
        let shown = format!("{draft:?}");
        assert!(!shown.contains("hunter2") && shown.contains("•••"), "{shown}");
        let command = crate::Command::SaveLogin { draft };
        assert!(!format!("{command:?}").contains("hunter2"));
    }
}
