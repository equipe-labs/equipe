//! Verification codes a site emails while a member's browser waits on them —
//! GitHub's "Confirm access", Google's "Verify it's you", any "enter the code
//! we emailed you" — filled from the connected mailbox on the person's click.
//!
//! A code exists to show a person is there, so:
//!
//! - no member reads, receives or types it, and none is ever asked for it in
//!   chat: the daemon reads it and types it into the page, and only when the
//!   person presses "Fill code from email" on the card (`Command::
//!   FillVerificationCode`, person-only). Nothing fetches or types a code
//!   without that click ([`Board`] makes raising a card and filling it two
//!   separate steps, and only a click moves one to the other);
//! - only mail that matches the waiting page is looked at ([`pick`]): from the
//!   site's own sender domain, received after the code was asked for and in
//!   the last ten minutes, saying it carries a verification code; the newest
//!   wins, and only a strictly code-shaped token leaves it. The mailbox is
//!   searched for that sender on the server's side, so other mail isn't
//!   fetched at all ([`gmail::look`], [`crate::autopilot::mail::Imap::matching`]);
//! - the code goes from the mailbox to the page and nowhere else: not into a
//!   card, an event, the journal or the log ([`Outcome`] has no place for it,
//!   [`Pick`]'s `Debug` hides it), and while the card lives it is cut out of
//!   anything a member writes, should a page echo it.
//!
//! The mailbox is Gmail over its API with the read-only scope ([`gmail`]), or
//! any mailbox over IMAP read-only (`EXAMINE`, `BODY.PEEK`) with an app
//! password — the one Login autopilot already uses.

pub mod gmail;

use std::time::Duration;

use crate::autopilot::codes;
use crate::autopilot::mail::Message;
use crate::browser::Signals;
use crate::vault::{FillGuard, Filled};

/// Mail older than this isn't the waiting page's.
pub const WINDOW_SECS: u64 = 10 * 60;
/// The mail may land a little before equipe saw the page ask for it (the
/// member pressed "Send code" and the page then showed its field).
pub const EARLY_SECS: u64 = 90;
/// How long a click waits for the mail to come in.
pub const WAIT: Duration = Duration::from_secs(60);
/// How often the mailbox is looked at meanwhile.
pub const TICK: Duration = Duration::from_secs(5);

// ── which page is an emailed-code step ──

/// Words a page uses to say a code was emailed, in English; the locale
/// files' `match.emailed_words` add every other language equipe speaks.
const EMAILED: &[&str] = &[
    "we emailed",
    "we've emailed",
    "we have emailed",
    "emailed you",
    "emailed a code",
    "emailed to",
    "sent to your email",
    "sent to your e-mail",
    "sent an email",
    "sent you an email",
    "code to your email",
    "code was sent to",
    "check your email",
    "check your inbox",
    "via email",
    "by email",
    "email verification code",
    "email code",
    "verification code to",
];

fn emailed_words() -> impl Iterator<Item = &'static str> {
    crate::i18n::every("match.emailed_words").iter().copied().chain(EMAILED.iter().copied())
}

/// Whether the page names a mail address, masked or not (`a•••@example.com`).
fn names_an_address(text: &str) -> bool {
    text.match_indices('@').any(|(at, _)| {
        let after: String = text[at + 1..].chars().take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '-').collect();
        let before = text[..at].chars().next_back().is_some_and(|c| !c.is_whitespace());
        before && after.contains('.') && !after.ends_with('.')
    })
}

/// The site of a member's page that waits on a code the site emailed: an
/// https page with a code field in sight that says the code went by email —
/// GitHub's "Confirm access" (sudo) with the email code chosen, Google's
/// "Verify it's you" with an email code, any "enter the code we emailed".
/// `None` for a code from a phone or an authenticator app, which stay the
/// person's to type.
pub fn email_step(url: &str, title: &str, signals: Option<&Signals>) -> Option<String> {
    let s = signals?;
    let url_l = url.trim().to_ascii_lowercase();
    if !url_l.starts_with("https://") || !(s.otp || s.code) {
        return None;
    }
    let site = crate::vault::site_of(url);
    if site.is_empty() {
        return None;
    }
    let title_l = title.to_lowercase();
    let says = |w: &str| !w.is_empty() && (s.text.contains(&w.to_lowercase()) || title_l.contains(&w.to_lowercase()));
    let mail = says("email") || says("e-mail") || says("inbox") || names_an_address(&s.text);
    if !mail {
        return None;
    }
    // A code to a phone or an authenticator, with mail only mentioned in passing: not ours.
    let phone_only = ["authenticator app", "text message", "sms", "github mobile"].iter().any(|w| says(w)) && !emailed_words().any(says);
    if phone_only {
        return None;
    }
    let github_sudo = (site == "github.com") && (says("confirm access") || url_l.contains("sudo"));
    let google = site == "accounts.google.com" && (says("verify it") || says("verify it's you") || says("verify it’s you") || says("2-step verification"));
    (github_sudo || google || emailed_words().any(says)).then_some(site)
}

// ── which mail is the page's ──

/// What a card waits for: mail from `site`'s own sender (or the domain the
/// person named for it), sent after `requested_at` (unix seconds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub site: String,
    pub mail_from: Option<String>,
    pub requested_at: u64,
}

impl Wanted {
    /// The sender domains to search the mailbox for, on the server's side.
    pub fn domains(&self) -> Vec<String> {
        let mut out = vec![codes::registrable(&self.site)];
        if let Some(m) = self.mail_from.as_deref().map(|m| m.trim().trim_start_matches('@').to_ascii_lowercase()).filter(|m| !m.is_empty()) {
            out.push(m);
        }
        out.dedup();
        out
    }

    /// The earliest a message may have arrived to be this page's.
    pub fn earliest(&self, now: u64) -> u64 {
        self.requested_at.saturating_sub(EARLY_SECS).max(now.saturating_sub(WINDOW_SECS))
    }
}

/// The code picked out of a message, and when the message came. Its `Debug`
/// never shows the code.
#[derive(Clone, PartialEq, Eq)]
pub struct Pick {
    pub code: String,
    pub at: u64,
}

impl std::fmt::Debug for Pick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pick").field("code", &"•••").field("at", &self.at).finish()
    }
}

/// Words that say a message carries a verification code.
fn verification_words() -> impl Iterator<Item = String> {
    crate::i18n::every("match.code_words").iter().map(|w| w.to_lowercase()).chain(["verification", "verify", "confirm", "sudo", "one-time", "passcode", "security code", "code"].map(str::to_owned))
}

/// A code as strict as a site's: 6 to 8 digits, or a short run of capitals
/// and digits next to a code word ([`codes::find_code`] finds it, never a
/// year, a date, a price or a phone number's run).
fn strict_code(text: &str) -> Option<String> {
    let code = codes::find_code(text)?;
    let digits = code.chars().all(|c| c.is_ascii_digit());
    ((digits && (6..=8).contains(&code.len())) || (!digits && (6..=10).contains(&code.len()))).then_some(code)
}

/// The code for `wanted` among `messages` at `now`: from the site's own
/// sender domain, arrived after the code was asked for and within the last
/// ten minutes, saying it is a verification code — the newest such message
/// that carries a strict code. Every other message is passed over unread.
pub fn pick(messages: &[Message], wanted: &Wanted, now: u64) -> Option<Pick> {
    let earliest = wanted.earliest(now);
    let mut mine: Vec<&Message> = messages
        .iter()
        .filter(|m| m.at >= earliest && m.at <= now + EARLY_SECS)
        .filter(|m| codes::sender_domain(&m.from).is_some_and(|d| codes::sender_matches(&d, &wanted.site, wanted.mail_from.as_deref())))
        .collect();
    mine.sort_by_key(|m| std::cmp::Reverse(m.at));
    mine.into_iter().find_map(|m| {
        let text = format!("{}\n{}", m.subject, m.text);
        let lower = text.to_lowercase();
        if !verification_words().any(|w| lower.contains(&w)) {
            return None;
        }
        strict_code(&text).map(|code| Pick { code, at: m.at })
    })
}

// ── the click ──

/// Where the mail comes from: Gmail's API or IMAP in use, a fake in tests.
pub trait Mailbox {
    /// Messages from `wanted`'s sender domains that arrived since
    /// `since` (unix seconds) — only those: the mailbox is searched for the
    /// sender on its own side.
    fn look(&self, wanted: &Wanted, since: u64) -> impl std::future::Future<Output = Result<Vec<Message>, String>> + Send;
}

/// The member's page the code goes into: the browser host's `fill` in use
/// (the member's tools held off the tab while it types), a fake in tests.
pub trait Page {
    fn run(&self, script: String) -> impl std::future::Future<Output = Result<Filled, String>> + Send;
}

/// How a click ended — never with the code in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Typed into the page; `submitted` when the page's own button was
    /// pressed as part of the same click (the page's flow wants it).
    Filled { submitted: bool },
    /// No matching mail came in time.
    NoMail,
    /// The page wasn't one the code may go into (another host, http), or
    /// had no code field: nothing was typed.
    Refused(String),
    /// The mailbox couldn't be read.
    Failed(String),
}

/// One click on "Fill code from email": look in the mailbox for the matching
/// mail every `tick` for up to `wait`, take the code out of the newest, and
/// type it into `page` on its own host only, over https ([`guard_for`]). While it
/// lives the code is cut out of anything a member writes (`mask_key`).
pub async fn fill_from_mail<M: Mailbox, P: Page>(mailbox: &M, page: &P, wanted: &Wanted, mask_key: &str, wait: Duration, tick: Duration, now: impl Fn() -> u64) -> Outcome {
    let until = tokio::time::Instant::now() + wait;
    let found = loop {
        let at = now();
        // A mailbox that refuses (a revoked grant, a wrong app password) won't
        // do better in five seconds: said at once, and the person may press again.
        let messages = match mailbox.look(wanted, wanted.earliest(at)).await {
            Ok(messages) => messages,
            Err(why) => return Outcome::Failed(why),
        };
        if let Some(found) = pick(&messages, wanted, at) {
            break found;
        }
        if tokio::time::Instant::now() + tick > until {
            return Outcome::NoMail;
        }
        tokio::time::sleep(tick).await;
    };
    crate::squad_secrets::watch(mask_key, &found.code);
    let script = crate::autopilot::fill::code_script(&found.code, &guard_for(&wanted.site));
    drop(found);
    match page.run(script).await {
        Ok(filled) if filled.refused.is_some() => Outcome::Refused(filled.refused.unwrap_or_default()),
        Ok(filled) if filled.filled.iter().any(|f| f == "code") => Outcome::Filled { submitted: filled.submitted },
        Ok(_) => Outcome::Refused("no_field".into()),
        Err(why) => Outcome::Failed(why),
    }
}

/// The guard a code goes into a page with: that page's exact host, https,
/// no "remember me" box ticked on the person's behalf.
pub fn guard_for(site: &str) -> FillGuard {
    FillGuard { hosts: vec![crate::vault::site_of(site)], subdomains: false, http: false, remember: false, account: None }
}

// ── "Open email" ──

/// A URL's query value, escaped.
pub(crate) fn pct(text: &str) -> String {
    text.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
        out
    })
}

/// "Open email": the mailbox's own web page, searched for the site's mail of
/// the last hour — for Gmail (by API, or over IMAP at `imap.gmail.com`), and
/// the few webmails whose search takes an address. `None` elsewhere.
pub fn open_url(kind: &str, address: &str, imap_host: Option<&str>, site: &str) -> Option<String> {
    let domain = codes::registrable(site);
    let gmail = kind == "gmail" || imap_host.is_some_and(|h| h.eq_ignore_ascii_case("imap.gmail.com") || h.eq_ignore_ascii_case("imap.googlemail.com"));
    if gmail {
        return Some(format!("https://mail.google.com/mail/?authuser={}#search/{}", pct(address), pct(&format!("from:{domain} newer_than:1h"))));
    }
    match imap_host.map(str::to_ascii_lowercase).as_deref() {
        Some("imap.fastmail.com") => Some(format!("https://app.fastmail.com/mail/search:from%3A{}", pct(&domain))),
        Some("outlook.office365.com" | "imap-mail.outlook.com") => Some("https://outlook.live.com/mail/0/".to_owned()),
        Some("imap.mail.me.com") => Some("https://www.icloud.com/mail/".to_owned()),
        _ => None,
    }
}

// ── cards and clicks, as a state machine ──

/// What the daemon does next for a card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Nothing: the card waits for the person.
    Wait,
    /// The person clicked: look in the mailbox and fill the page.
    Fetch,
}

/// The cards' states, apart from screens and the browser: raising a card
/// never fetches or types anything; only [`Board::click`] does, once per
/// click, and not while a fetch is under way.
#[derive(Debug, Default)]
pub struct Board {
    cards: Vec<crate::protocol::verify::Verification>,
}

impl Board {
    pub fn cards(&self) -> &[crate::protocol::verify::Verification] {
        &self.cards
    }

    pub fn get(&self, id: u64) -> Option<&crate::protocol::verify::Verification> {
        self.cards.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut crate::protocol::verify::Verification> {
        self.cards.iter_mut().find(|c| c.id == id)
    }

    /// The open card for `lane` and `site`, if one is up.
    pub fn of(&self, lane: &str, site: &str) -> Option<&crate::protocol::verify::Verification> {
        self.cards.iter().find(|c| c.lane == lane && c.site == site)
    }

    /// Put a card up — one per lane and site: the same ask again keeps the
    /// card that is up. `true` when it is new. Never fetches anything.
    pub fn raise(&mut self, card: crate::protocol::verify::Verification) -> (bool, Act) {
        if self.of(&card.lane, &card.site).is_some() {
            return (false, Act::Wait);
        }
        self.cards.push(card);
        (true, Act::Wait)
    }

    /// The person pressed "Fill code from email": the one way to [`Act::Fetch`].
    pub fn click(&mut self, id: u64) -> Act {
        let Some(card) = self.get_mut(id) else { return Act::Wait };
        if !card.phase.open() || card.mailbox.is_none() {
            return Act::Wait;
        }
        card.phase = crate::protocol::verify::Phase::Fetching;
        card.detail = None;
        Act::Fetch
    }

    pub fn close(&mut self, id: u64) -> Option<crate::protocol::verify::Verification> {
        let at = self.cards.iter().position(|c| c.id == id)?;
        Some(self.cards.remove(at))
    }

    /// The cards of a lane (its browser went back to its member, its tab closed).
    pub fn of_lane(&self, lane: &str) -> Vec<u64> {
        self.cards.iter().filter(|c| c.lane == lane).map(|c| c.id).collect()
    }
}

#[cfg(test)]
mod tests;
