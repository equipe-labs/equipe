//! Secrets a member needs for a command or a config — a NAS login, an API
//! key, a token — that the person types into a card rather than a
//! conversation (`ask_secret`). What a screen sends goes straight to a file
//! on the daemon's machine (`equipe_core::secrets`); the member hears only
//! where it is and under which keys, never the values. Nothing here carries a
//! value in its `Debug`, so no log does either.

use serde::{Deserialize, Serialize};

use crate::{SessionId, SquadName, TicketId};

/// One value asked for: the key it is saved under, how the card names it,
/// and whether it is typed hidden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretField {
    /// `NAS_USER`: uppercase letters, digits and `_`.
    pub key: String,
    /// `NAS username`, in the person's language.
    pub label: String,
    /// Typed hidden (a password, a key); `false` for a username.
    #[serde(default)]
    pub secret: bool,
}

/// A member's request, waiting for the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretAsk {
    pub id: u64,
    pub session: SessionId,
    pub member: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    /// What it is for, in the person's language.
    pub why: String,
    pub fields: Vec<SecretField>,
    /// Where it is saved, as the person reads it: `~/.config/atinum/nas.env`.
    pub save_to: String,
    /// When it was asked, unix seconds.
    pub at: u64,
    /// Set for a `request_secret` card: the value goes into the squad's
    /// secret store under this name instead of an environment variable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<SecretStore>,
    /// What the person does to get it, in order, in their language (1 to 8
    /// short lines). Empty only for a card kept before guides existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<String>,
    /// The page the secret comes from (https): one button opens it on the
    /// screen the person is looking at. `None`: it has no web page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The service's sign-in page (https), a second link when it differs from `url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_in_url: Option<String>,
}

/// Where a `request_secret` (or Connections) card's value is kept: the squad
/// secret store, under `name`, with these facts. Nothing here is a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretStore {
    /// The name it is kept under (`anthropic-admin-key`).
    pub name: String,
    /// The service it is for: a catalog id (`anthropic`) or a site.
    pub service: String,
    pub kind: SecretKind,
    /// The projects whose work may use it, by name (checked by the daemon).
    #[serde(default)]
    pub projects: Vec<String>,
    /// The page on the service where the person creates the key (https).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_url: Option<String>,
    /// What to create there ("read-only / usage scope"), as the member or the
    /// catalog put it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes_hint: Option<String>,
}

/// A value the person typed: on the wire as a plain string, never in a log.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretValue(pub String);

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue(•••)")
    }
}

/// What a key a member obtained is: how a screen labels it, and what a
/// member says when it saves one (`secret_save`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    ApiKey,
    Token,
    Password,
    ConnectionString,
    #[default]
    Other,
}

impl SecretKind {
    pub const ALL: [SecretKind; 5] = [Self::ApiKey, Self::Token, Self::Password, Self::ConnectionString, Self::Other];

    /// The word members and the CLI use for it.
    pub fn code(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::Token => "token",
            Self::Password => "password",
            Self::ConnectionString => "connection_string",
            Self::Other => "other",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        let word = word.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        Self::ALL.into_iter().find(|k| k.code() == word)
    }
}

/// One use of a squad secret, as the journal keeps it: who, on which
/// ticket and project, when. Never the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretUse {
    pub at: u64,
    pub member: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    /// The project it was used on; empty: no project (the squad's own folder).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
    /// `env` (a turn's environment) or `file` (written into the worktree).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub how: String,
}

/// A key, token or password a member of a squad obtained and gave to equipe
/// to keep (`secret_save`): what a screen may see about it. The value lives
/// only in the machine's secret store; nothing here carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SquadSecret {
    pub squad: SquadName,
    pub name: String,
    pub kind: SecretKind,
    /// The service or site it is for: `posthog`, `vercel`, `app.posthog.com`.
    #[serde(default)]
    pub service: String,
    /// The projects whose work may use it.
    #[serde(default)]
    pub projects: Vec<String>,
    /// The person marked it usable on any project: no first-use question.
    #[serde(default)]
    pub any_project: bool,
    /// The member that saved it, and the ticket it was on.
    pub created_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_ticket: Option<TicketId>,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<SecretUse>,
}

/// What stands for "no project" in a secret's list of projects: work in the
/// squad's own folder, on no repository.
pub const NO_PROJECT: &str = "";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_is_said_in_one_word_and_read_back_loosely() {
        for kind in SecretKind::ALL {
            assert_eq!(SecretKind::parse(kind.code()), Some(kind));
        }
        assert_eq!(SecretKind::parse("API-Key"), Some(SecretKind::ApiKey));
        assert_eq!(SecretKind::parse("connection string"), Some(SecretKind::ConnectionString));
        assert_eq!(SecretKind::parse("pgp"), None);
    }

    #[test]
    fn a_guide_rides_the_card_over_the_wire_and_an_old_card_still_reads() {
        let ask = SecretAsk {
            id: 1,
            session: 2,
            member: "backend".into(),
            ticket: None,
            why: "usage reports".into(),
            fields: vec![SecretField { key: "K".into(), label: "k".into(), secret: true }],
            save_to: String::new(),
            at: 0,
            store: None,
            steps: vec!["Open the page".into(), "Click Create key".into()],
            url: Some("https://platform.claude.com/settings/admin-keys".into()),
            sign_in_url: Some("https://platform.claude.com/".into()),
        };
        let json = serde_json::to_string(&ask).unwrap();
        assert_eq!(serde_json::from_str::<SecretAsk>(&json).unwrap(), ask);
        let old = r#"{"id":1,"session":2,"member":"m","why":"w","fields":[],"save_to":"","at":0}"#;
        let read: SecretAsk = serde_json::from_str(old).unwrap();
        assert!(read.steps.is_empty() && read.url.is_none() && read.sign_in_url.is_none());
    }

    #[test]
    fn a_typed_value_never_shows_in_a_log() {
        let value = SecretValue("hunter2".into());
        assert_eq!(format!("{value:?}"), "SecretValue(•••)");
        assert_eq!(serde_json::to_string(&value).unwrap(), "\"hunter2\"", "on the wire as typed");
    }
}
