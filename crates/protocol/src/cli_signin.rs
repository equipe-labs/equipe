//! A command-line sign-in the person finishes on a card instead of in a
//! terminal (`cli_sign_in`): equipe runs the CLI's cross-device mode itself
//! and shows its page, its code to confirm, and a box when the flow wants a
//! code pasted back. What the person pastes goes straight to the waiting
//! process; nothing here keeps it, and its `Debug` never shows it.

use serde::{Deserialize, Serialize};

use crate::{SessionId, TicketId};

/// Where a sign-in stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The CLI was started; its page isn't known yet.
    Starting,
    /// Waiting for the person to sign in on the page.
    Waiting,
    /// A code was handed over; the CLI is checking it.
    Checking,
    /// Done: `who` says as whom.
    SignedIn,
    /// The link or the code ran out: "Get a new link" starts it again.
    Expired,
    /// The CLI ended without signing in.
    Failed,
}

impl Phase {
    /// Still needs the person (a card to answer), as opposed to finished.
    pub fn open(self) -> bool {
        !matches!(self, Self::SignedIn)
    }
}

/// One sign-in a member asked the person to finish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliSignIn {
    pub id: u64,
    pub session: SessionId,
    pub member: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    /// The tool's word (`aws`, `aws_sso`, `gh`, `gcloud`, `az`).
    pub tool: String,
    /// How the card names it: "AWS", "GitHub".
    pub name: String,
    /// The profile it signs in (AWS), when one was named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// What it is for, in the person's language.
    pub why: String,
    /// The page to open, once the CLI printed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The code the page asks them to confirm (a device code), if the flow shows one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_code: Option<String>,
    /// The flow has the person paste a code from the page back.
    #[serde(default)]
    pub wants_code: bool,
    pub phase: Phase,
    /// As whom it signed in (the CLI's own whoami, no secrets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Option<String>,
    /// Why it stopped, when the tool said (its last line, short): no addresses, no codes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// When it was asked, unix seconds.
    pub at: u64,
}

/// A code the person pasted: on the wire as a plain string, never in a log.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PastedCode(pub String);

impl std::fmt::Debug for PastedCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PastedCode(•••)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pasted_code_never_shows_in_debug() {
        let code = PastedCode("4/0AbCdEf-secretvalue".into());
        assert!(!format!("{code:?}").contains("secretvalue"));
        let cmd = crate::Command::SubmitCliSignInCode { squad: "a".into(), id: 1, code };
        assert!(!format!("{cmd:?}").contains("secretvalue"));
    }

    #[test]
    fn a_finished_card_is_not_open() {
        assert!(Phase::Waiting.open() && Phase::Expired.open());
        assert!(!Phase::SignedIn.open());
    }
}
