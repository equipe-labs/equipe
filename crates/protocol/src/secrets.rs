//! Secrets a member needs for a command or a config — a NAS login, an API
//! key, a token — that the person types into a card rather than a
//! conversation (`ask_secret`). What a screen sends goes straight to a file
//! on the daemon's machine (`equipe_core::secrets`); the member hears only
//! where it is and under which keys, never the values. Nothing here carries a
//! value in its `Debug`, so no log does either.

use serde::{Deserialize, Serialize};

use crate::{SessionId, TicketId};

/// One value asked for: the key it is saved under, how the card names it,
/// and whether it is typed hidden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretField {
    /// `NAS_USER`: uppercase letters, digits and `_`.
    pub key: String,
    /// `NAS 아이디`, in the person's language.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_value_never_shows_in_a_log() {
        let value = SecretValue("hunter2".into());
        assert_eq!(format!("{value:?}"), "SecretValue(•••)");
        assert_eq!(serde_json::to_string(&value).unwrap(), "\"hunter2\"", "on the wire as typed");
    }
}
