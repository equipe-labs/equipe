//! Login autopilot: the person signs in to a site once, and equipe keeps the
//! account working without asking them again — as far as each site lets it,
//! and only as far as the person said. Every capability is off until the
//! person turns it on for that login (`protocol::vault::Autopilot`), and only
//! a person-only command turns it on; nothing a member writes or a page shows
//! does.
//!
//! - [`totp`]: codes from an authenticator seed kept in the secret store;
//! - [`mail`] and [`sms`]: a site's one-time code read from the person's
//!   mailbox (IMAP) or from Messages on this Mac, only from the site's own
//!   sender, only the code ([`codes`]);
//! - [`fill`]: what equipe types into the site's own page — the code, the
//!   "remember me" box, the passkey button, a device-code approval;
//! - [`keepalive`]: looks at a site now and then to catch an expired sign-in
//!   before a member does;
//! - [`cli_logins`]: command-line credentials and device-code sign-ins;
//! - [`qr`]: an authenticator QR code read off the site's own setup page,
//!   on this machine.
//!
//! Never: solving or getting around a CAPTCHA or a bot check, spoofing a
//! browser, or getting around the operating system's guards. A CAPTCHA goes
//! to the person in one tap, and the sign-in carries on by itself after it.

pub mod cli_logins;
pub mod codes;
pub mod fill;
pub mod keepalive;
pub mod mail;
pub mod qr;
pub mod sms;
pub mod totp;

/// What finished a sign-in with no one's hand, as the sign-in record keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// The kept password, typed by equipe (`Login::auto`).
    Relogin,
    Totp,
    Email,
    Sms,
    Passkey,
    DeviceCode,
    /// A keep-alive look found the site signed out and signed it back in.
    Keepalive,
}

impl Step {
    pub const ALL: [Step; 7] = [Step::Relogin, Step::Totp, Step::Email, Step::Sms, Step::Passkey, Step::DeviceCode, Step::Keepalive];

    pub fn code(self) -> &'static str {
        match self {
            Step::Relogin => "relogin",
            Step::Totp => "totp",
            Step::Email => "email",
            Step::Sms => "sms",
            Step::Passkey => "passkey",
            Step::DeviceCode => "device_code",
            Step::Keepalive => "keepalive",
        }
    }

    pub fn parse(code: &str) -> Option<Step> {
        Step::ALL.into_iter().find(|s| s.code() == code)
    }

    /// Roughly how many minutes of the person's it saves, each time: what a
    /// handoff of that kind costs them — being interrupted, finding the
    /// browser, typing, finding the phone or the mail, coming back. Rough on
    /// purpose; the record keeps the count, this only weighs it.
    pub fn minutes_saved(self) -> u64 {
        match self {
            Step::Relogin | Step::Keepalive => 2,
            Step::Totp | Step::Passkey => 3,
            Step::Email | Step::Sms => 4,
            Step::DeviceCode => 5,
        }
    }
}

/// How long equipe watches the mailbox or Messages for a site's code after
/// its code page came up, before handing the page to the person.
pub const CODE_WAIT_SECS: u64 = 180;

/// Login autopilot's own settings (`autopilot.json` in equipe's home): the
/// mailbox (its app password in the secret store), Messages, keep-alive.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mailbox: Option<crate::protocol::vault::Mailbox>,
    pub sms: bool,
    pub keepalive_hours: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { mailbox: None, sms: false, keepalive_hours: keepalive::DEFAULT_HOURS }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_step_has_a_code_that_reads_back_and_a_weight() {
        for s in Step::ALL {
            assert_eq!(Step::parse(s.code()), Some(s));
            assert!(s.minutes_saved() > 0);
        }
        assert_eq!(Step::parse("captcha"), None, "a CAPTCHA is never equipe's");
    }
}
