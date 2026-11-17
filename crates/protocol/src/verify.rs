//! A verification code a site emailed while a member's browser waits on it
//! (GitHub's "Confirm access", Google's "Verify it's you", any "enter the
//! code we emailed you"): ONE card for the person — "GitHub sent a
//! verification code to <mailbox>" — with "Fill code from email". Only on
//! that click does the daemon read the matching mail from the connected
//! mailbox and type the code into the page. The card never carries the
//! code, and neither does any event: a code exists to show a person is
//! there, so no member reads, receives or types it.

use serde::{Deserialize, Serialize};

use crate::{SessionId, TicketId};

/// How the site sent the code. Email is the one equipe can fetch; the
/// others stay the person's to type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Email,
}

impl Channel {
    /// Its name on the wire, as a member passes it to `block_on_person`.
    pub fn key(self) -> &'static str {
        match self {
            Channel::Email => "email",
        }
    }

    pub fn parse(key: &str) -> Option<Channel> {
        match key.trim().to_ascii_lowercase().as_str() {
            "email" | "e-mail" | "mail" => Some(Channel::Email),
            _ => None,
        }
    }
}

/// Where a card stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Waiting for the person's click.
    Waiting,
    /// The person clicked: equipe is looking in the mailbox (up to a minute)
    /// and typing the code into the page.
    Fetching,
    /// Typed into the page and sent: the member is told the person confirmed.
    Filled,
    /// No matching mail came in time: the card says how to get a new one.
    NoMail,
    /// It couldn't be done (the mailbox refused, the page moved on):
    /// `detail` says why, never with a code.
    Failed,
}

impl Phase {
    /// Still needs the person (a button to press), as opposed to finished or under way.
    pub fn open(self) -> bool {
        matches!(self, Phase::Waiting | Phase::NoMail | Phase::Failed)
    }
}

/// Which mailbox a code is fetched from, as the card names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxRef {
    /// The address the code went to: the connected mailbox's.
    pub address: String,
    /// `gmail` or `imap`.
    pub kind: String,
}

/// One card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    pub id: u64,
    /// The member's conversation whose browser waits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionId>,
    pub member: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    /// The page's site, by host (`github.com`).
    pub site: String,
    /// How the card names it ("GitHub").
    pub name: String,
    pub channel: Channel,
    /// The mailbox equipe reads it from; `None` when none is connected — the
    /// card then offers to connect one, and "I'll type it".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailbox: Option<MailboxRef>,
    /// "Open email": the mailbox's own web page, searched for the site's
    /// recent mail — opened in the person's own browser, never a member's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_url: Option<String>,
    /// The member's browser lane and its tab waiting on the code.
    pub lane: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u64>,
    pub phase: Phase,
    /// Why it stopped, short; never a code or a word of anyone's mail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// When it was raised, unix seconds.
    pub at: u64,
}

/// A host as people call it: "GitHub" for `github.com`, "Google" for
/// `accounts.google.com`; else the host without `www.`.
pub fn site_name(host: &str) -> String {
    let host = host.trim().trim_start_matches("www.").to_ascii_lowercase();
    let known = [
        ("github.com", "GitHub"),
        ("google.com", "Google"),
        ("microsoft.com", "Microsoft"),
        ("live.com", "Microsoft"),
        ("apple.com", "Apple"),
        ("amazon.com", "Amazon"),
        ("vercel.com", "Vercel"),
        ("gitlab.com", "GitLab"),
        ("slack.com", "Slack"),
        ("notion.so", "Notion"),
        ("atlassian.com", "Atlassian"),
        ("stripe.com", "Stripe"),
        ("cloudflare.com", "Cloudflare"),
    ];
    known.iter().find(|(h, _)| host == *h || host.ends_with(&format!(".{h}"))).map_or(host.clone(), |(_, n)| (*n).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> Verification {
        Verification {
            id: 7,
            session: Some(3),
            member: "backend".into(),
            ticket: Some(12),
            site: "github.com".into(),
            name: "GitHub".into(),
            channel: Channel::Email,
            mailbox: Some(MailboxRef { address: "squad@example.com".into(), kind: "gmail".into() }),
            open_url: Some("https://mail.google.com/mail/?authuser=squad%40example.com#search/from%3Agithub.com".into()),
            lane: "acme/backend".into(),
            page: Some(41),
            phase: Phase::Waiting,
            detail: None,
            at: 1_794_528_000,
        }
    }

    #[test]
    fn a_card_goes_over_the_wire_and_back_whole_and_carries_no_code_field() {
        let c = card();
        let json = serde_json::to_string(&crate::Event::VerificationChanged { squad: "acme".into(), card: c.clone() }).unwrap();
        assert!(!json.contains("\"code\""), "a card has no place for a code: {json}");
        let back: crate::Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back, crate::Event::VerificationChanged { squad: "acme".into(), card: c });
        let click = crate::Command::FillVerificationCode { squad: "acme".into(), id: 7 };
        let back: crate::Command = serde_json::from_str(&serde_json::to_string(&click).unwrap()).unwrap();
        assert_eq!(back, click);
        let mut bare = card();
        (bare.mailbox, bare.open_url, bare.page, bare.ticket, bare.session) = (None, None, None, None, None);
        let json = serde_json::to_string(&bare).unwrap();
        assert!(!json.contains("mailbox") && !json.contains("open_url"), "{json}");
        assert_eq!(serde_json::from_str::<Verification>(&json).unwrap(), bare);
    }

    #[test]
    fn channels_and_names_read_as_members_and_people_write_them() {
        assert_eq!(Channel::parse("Email"), Some(Channel::Email));
        assert_eq!(Channel::parse("sms"), None, "a code to a phone is the person's to type");
        assert_eq!(site_name("github.com"), "GitHub");
        assert_eq!(site_name("accounts.google.com"), "Google");
        assert_eq!(site_name("www.example.org"), "example.org");
        assert!(Phase::Waiting.open() && Phase::NoMail.open() && !Phase::Fetching.open() && !Phase::Filled.open());
    }
}
