//! Why the person is asked to sign in, kept so it can be measured: every
//! time a member's browser goes into the person's hands for a sign-in or a
//! check (`show_my_browser`, a page that waits for a person), every
//! `sign_in` card and its outcome, every sign-in finished — by the person or
//! by equipe itself — goes into the squad's journal (`login_events`) with
//! the site's host, the member, when, and a cause ([`Cause`]). Never a
//! password, a code, a page's text or its address beyond the host.
//!
//! From those rows ([`summarize`]): handoffs per site per week by cause,
//! how long a sign-in lasts before the site is asked for again, the same
//! site asked for by different members within a day, and the share of
//! sign-ins equipe finished with no one's hand. `equipe logins stats` prints
//! them; the PM's weekly retro gets a line of them ([`retro_line`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension as _, params};

use equipe_protocol::HumanCheck;

/// Why a sign-in or a check needed the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    /// The site was signed in to before (or a login is kept for it), and asks again.
    SessionExpired,
    /// No login kept, and never seen signed in: a first sign-in.
    NoLoginKept,
    /// A CAPTCHA, or a whole-page "verify you are human" wall.
    Captcha,
    /// A code from the person's phone, mail or authenticator; a checkpoint after the password.
    TwoFactor,
    /// A passkey, a security key, Touch ID.
    Passkey,
    /// A device-code or authorization-code flow (`aws sso login`, a code to paste back).
    DeviceCode,
    /// A consent or "authorize this app" screen.
    Consent,
    Other,
}

impl Cause {
    pub const ALL: [Cause; 8] = [Cause::SessionExpired, Cause::NoLoginKept, Cause::Captcha, Cause::TwoFactor, Cause::Passkey, Cause::DeviceCode, Cause::Consent, Cause::Other];

    pub fn code(self) -> &'static str {
        match self {
            Cause::SessionExpired => "session_expired",
            Cause::NoLoginKept => "no_login_kept",
            Cause::Captcha => "captcha",
            Cause::TwoFactor => "two_factor",
            Cause::Passkey => "passkey",
            Cause::DeviceCode => "device_code",
            Cause::Consent => "consent",
            Cause::Other => "other",
        }
    }

    pub fn parse(code: &str) -> Option<Cause> {
        Cause::ALL.into_iter().find(|c| c.code() == code)
    }

    /// Whether it asks the person to sign in again, rather than for a step
    /// after the password.
    pub fn is_sign_in(self) -> bool {
        matches!(self, Cause::SessionExpired | Cause::NoLoginKept)
    }

    /// Whether one person's step serves every member asking for it on that
    /// site — a sign-in, or the code or passkey after it — since they all
    /// share one browser: a second member waits on the first hand-off.
    pub fn is_shared_step(self) -> bool {
        self.is_sign_in() || matches!(self, Cause::TwoFactor | Cause::Passkey)
    }
}

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A member's browser went into the person's hands.
    Handoff,
    /// A `sign_in` card asked the person for a yes.
    Card,
    /// A member asked to sign in where no login is kept.
    NoLogin,
    /// A sign-in finished: `by` the person, or equipe.
    SignedIn,
    /// equipe's fill didn't get past the form.
    FillFailed,
}

impl Kind {
    pub fn code(self) -> &'static str {
        match self {
            Kind::Handoff => "handoff",
            Kind::Card => "card",
            Kind::NoLogin => "no_login",
            Kind::SignedIn => "signed_in",
            Kind::FillFailed => "fill_failed",
        }
    }

    pub fn parse(code: &str) -> Option<Kind> {
        [Kind::Handoff, Kind::Card, Kind::NoLogin, Kind::SignedIn, Kind::FillFailed].into_iter().find(|k| k.code() == code)
    }

    /// Whether the person was asked for something.
    pub fn asks(self) -> bool {
        matches!(self, Kind::Handoff | Kind::Card)
    }
}

/// Who finished a sign-in.
pub const BY_PERSON: &str = "person";
pub const BY_EQUIPE: &str = "equipe";

/// One row of the record.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Entry {
    pub at: u64,
    /// The site, by host (`vault::site_of`).
    pub site: String,
    pub member: String,
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<Cause>,
    /// For a sign-in: [`BY_PERSON`] or [`BY_EQUIPE`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// Filled in when rows of several squads are read together.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub squad: String,
    /// For a sign-in equipe finished, or a fill that failed: which autopilot
    /// step did it (`crate::autopilot::Step`: `relogin`, `totp`, `email`,
    /// `sms`, `passkey`, `device_code`, `keepalive`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
}

impl Entry {
    pub fn new(at: u64, site: &str, member: &str, kind: Kind, cause: Option<Cause>, by: Option<&str>) -> Self {
        Entry { at, site: crate::vault::site_of(site), member: member.to_owned(), kind, cause, by: by.map(str::to_owned), squad: String::new(), step: None }
    }

    /// The autopilot step that did it.
    pub fn by_step(mut self, step: crate::autopilot::Step) -> Self {
        self.step = Some(step.code().to_owned());
        self
    }
}

// ── the causes ──

/// The cause of a page that waits for a person: a sign-in counts as expired
/// where the site was `known` (signed in to before, or a login kept).
pub fn cause_of_check(check: HumanCheck, url: &str, known: bool) -> Cause {
    match check {
        HumanCheck::Captcha => Cause::Captcha,
        HumanCheck::TwoFactor => Cause::TwoFactor,
        // A checkpoint after the password is a step of the person's own; a wall over a page, a CAPTCHA's kin.
        HumanCheck::Challenge if ["/checkpoint", "/challenge"].iter().any(|p| url.to_ascii_lowercase().contains(p)) => Cause::TwoFactor,
        HumanCheck::Challenge => Cause::Captcha,
        HumanCheck::SignIn if known => Cause::SessionExpired,
        HumanCheck::SignIn => Cause::NoLoginKept,
    }
}

/// The cause of a member handing its browser to the person: what its page
/// waits for, when it waits; else what the member said it was for and
/// where (the words in any of equipe's languages); else other.
pub fn cause_of_handoff(why: &str, url: Option<&str>, check: Option<HumanCheck>, known: bool) -> Cause {
    if let Some(check) = check {
        return cause_of_check(check, url.unwrap_or_default(), known);
    }
    let said = format!("{} {}", why.to_lowercase(), url.unwrap_or_default().to_lowercase());
    let any = |key: &str, extra: &[&str]| crate::i18n::every(key).iter().copied().chain(extra.iter().copied()).any(|w| !w.is_empty() && said.contains(&w.to_lowercase()));
    if any("match.passkey_words", &["passkey", "webauthn", "security key", "touch id", "face id"]) {
        Cause::Passkey
    } else if any("match.device_code_words", &["device code", "user_code", "/device", "sso login", "aws login"]) {
        Cause::DeviceCode
    } else if any("match.captcha_words", &["captcha", "cloudflare", "turnstile"]) {
        Cause::Captcha
    } else if any("match.code_words", &["2fa", "otp", "mfa", "one-time"]) {
        Cause::TwoFactor
    } else if any("match.consent_words", &["oauth", "/authorize", "consent"]) {
        Cause::Consent
    } else if any("match.sign_in_words", &["logged out", "session expired"]) {
        if known { Cause::SessionExpired } else { Cause::NoLoginKept }
    } else if crate::browser::sign_in_address(url.unwrap_or_default()) {
        // Said nothing of why, but the page it hands over is a sign-in one: it is a sign-in,
        // and seen through as one once it is handed back.
        if known { Cause::SessionExpired } else { Cause::NoLoginKept }
    } else {
        Cause::Other
    }
}

// ── sign-ins seen through ──

/// A member's page seen on a sign-in: where, so that its passing — the
/// same site, past the sign-in — is a sign-in finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigningIn {
    pub lane: String,
    /// The site, by host.
    pub site: String,
    /// The sign-in page's address, as it was.
    pub url: String,
    /// Where it landed, when that is another host of the same organisation
    /// (`mail.google.com` after `accounts.google.com`); empty otherwise.
    pub landed: String,
}

/// Where a sign-in the person started began: its tab, site and address.
struct Origin<'a> {
    page: u64,
    site: &'a str,
    url: &'a str,
}

/// Members' pages on a sign-in, followed from tab to tab (by page id).
#[derive(Debug, Default)]
pub struct SignInWatch {
    pages: std::collections::HashMap<u64, SigningIn>,
}

impl SignInWatch {
    /// A lane's tabs as the browser host tells them: the pages that were on
    /// a sign-in and now are past it, on the same site (or a subdomain of
    /// it), each with where it signed in. A page that went to another site,
    /// or closed, is forgotten.
    pub fn see(&mut self, lane: &str, pages: &[crate::browser::HostPage]) -> Vec<(u64, SigningIn)> {
        self.see_from(lane, pages, None)
    }

    /// The person's own tabs (lane `""`, in the browser panel or the Chrome
    /// window alike — they are the person's hands whichever shows them).
    /// `flow`, a sign-in the person started: its tab counts as signing in to
    /// the flow's site even while it is on another host (an identity
    /// provider's page, "Sign in with Google"), so coming back to the site
    /// past the sign-in finishes it.
    pub fn see_person(&mut self, pages: &[crate::browser::HostPage], flow: Option<&crate::signin::Flow>) -> Vec<(u64, SigningIn)> {
        let origin = flow.filter(|f| !f.site.is_empty()).and_then(|f| f.page().map(|page| Origin { page, site: &f.site, url: &f.url }));
        self.see_from("", pages, origin)
    }

    fn see_from(&mut self, lane: &str, pages: &[crate::browser::HostPage], origin: Option<Origin<'_>>) -> Vec<(u64, SigningIn)> {
        let mut finished = Vec::new();
        for page in pages {
            let signals = page.signals.as_ref();
            let started = origin.as_ref().filter(|o| o.page == page.id);
            if crate::browser::signing_in(&page.url, &page.title, signals) {
                let was = self.pages.entry(page.id).or_insert_with(|| match started {
                    Some(o) => SigningIn { lane: lane.to_owned(), site: o.site.to_owned(), url: if crate::vault::serves(o.site, &page.url) { page.url.clone() } else { o.url.to_owned() }, landed: String::new() },
                    None => SigningIn { lane: lane.to_owned(), site: crate::vault::site_of(&page.url), url: page.url.clone(), landed: String::new() },
                });
                // Begun on another page of the flow's site (its home, a provider's page): the site's own sign-in page is the one to keep.
                if let Some(o) = started
                    && was.url == o.url
                    && page.url != o.url
                    && crate::vault::serves(o.site, &page.url)
                {
                    was.url = page.url.clone();
                }
                continue;
            }
            if !self.pages.contains_key(&page.id) || !crate::browser::past_sign_in(&page.url, &page.title, signals, page.loading) {
                continue;
            }
            if let Some(mut was) = self.pages.remove(&page.id)
                && crate::vault::same_org(&was.site, &page.url)
            {
                if !crate::vault::serves(&was.site, &page.url) {
                    was.landed = crate::vault::site_of(&page.url);
                }
                finished.push((page.id, was));
            }
        }
        let open: std::collections::HashSet<u64> = pages.iter().map(|p| p.id).collect();
        self.pages.retain(|id, s| s.lane != lane || open.contains(id));
        finished
    }

    /// The browser host went: its pages with it.
    pub fn clear(&mut self) {
        self.pages.clear();
    }
}

/// What a member's page says, once back from the person's hands, of the
/// sign-in to `site` it was handed over for ([`back_verdict`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Back {
    /// Past the sign-in, on `site` — or, handed over on an identity
    /// provider's page, on the site the provider sent it on to (`on`).
    SignedIn { on: String },
    /// Still on a sign-in, or the code after it: the person didn't get through.
    StillAsks,
    /// Loading, or somewhere that says nothing yet.
    Wait,
}

/// Whether a page back from the person's hands shows the sign-in to `site` went through.
pub fn back_verdict(site: &str, page: &crate::browser::HostPage) -> Back {
    if page.loading {
        return Back::Wait;
    }
    if crate::browser::signing_in(&page.url, &page.title, page.signals.as_ref()) {
        return Back::StillAsks;
    }
    let on = crate::vault::site_of(&page.url);
    let went_on = crate::vault::idp_provider(site).is_some() && crate::vault::idp_provider(&on).is_none();
    if (crate::vault::same_org(site, &page.url) || went_on) && crate::browser::past_sign_in(&page.url, &page.title, page.signals.as_ref(), page.loading) {
        Back::SignedIn { on }
    } else {
        Back::Wait
    }
}

/// What equipe's own sign-in came to, by where the page went after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relogin {
    /// Past the sign-in: done.
    SignedIn,
    /// Back on the form after the password: the kept password doesn't work.
    FormCameBack,
    /// Anything else — nothing typed, a code or a CAPTCHA next, a page still
    /// loading, the page gone: the person's, as any page that waits on them.
    Person,
}

/// What equipe's sign-in at `site` came to: what the fill did, and the page
/// as it is now.
pub fn relogin_outcome(filled: &crate::vault::Filled, page: Option<&equipe_protocol::BrowserPage>, site: &str) -> Relogin {
    let Some(page) = page else { return Relogin::Person };
    if filled.refused.is_some() || filled.filled.is_empty() || page.loading {
        return Relogin::Person;
    }
    let same_site = crate::vault::site_of(&page.url) == site;
    match page.check {
        Some(HumanCheck::SignIn) if same_site && filled.submitted && filled.step.as_deref() == Some("password") => Relogin::FormCameBack,
        None => Relogin::SignedIn,
        _ => Relogin::Person,
    }
}

/// Whether to offer keeping a login for `site` the person just signed in
/// to by hand: not where one is kept, nor where they said never, nor where
/// it was offered already, nor where nothing can keep a password.
pub fn offer_wanted(site: &str, logins: &[crate::protocol::vault::Login], never: &[String], offered: &std::collections::HashSet<String>, can_keep: bool) -> bool {
    let site = crate::vault::site_of(site);
    !site.is_empty() && can_keep && !logins.iter().any(|l| crate::vault::serves(&l.site, &site)) && !never.iter().any(|s| crate::vault::serves(s, &site)) && !offered.contains(&site)
}

// ── the record, in a squad's journal ──

/// The table, made where it isn't yet.
pub(crate) fn create_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS login_events (
           at INTEGER NOT NULL,
           site TEXT NOT NULL,
           member TEXT NOT NULL,
           kind TEXT NOT NULL,
           cause TEXT,
           by TEXT
         );
         CREATE INDEX IF NOT EXISTS login_events_at ON login_events(at);",
    )?;
    // Which autopilot step finished a sign-in: a column journals from before lack.
    let has_step = conn.prepare("SELECT step FROM login_events LIMIT 0").is_ok();
    if !has_step {
        conn.execute_batch("ALTER TABLE login_events ADD COLUMN step TEXT")?;
    }
    Ok(())
}

/// A page on this machine (a member's own preview, a dev server): handing it
/// to the person is showing work, not a sign-in, so it isn't counted as one.
pub(crate) fn is_local_page(site: &str) -> bool {
    let host = site.trim_start_matches('[').split(']').next().unwrap_or(site);
    let host = host.rsplit_once(':').filter(|(h, port)| !h.is_empty() && !h.contains(':') && port.chars().all(|c| c.is_ascii_digit())).map_or(host, |(h, _)| h);
    host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") || host == "::1" || host.starts_with("127.") || host == "0.0.0.0"
}

pub(crate) fn insert(conn: &Connection, e: &Entry, keep_from: i64) -> rusqlite::Result<()> {
    if is_local_page(&e.site) {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO login_events (at, site, member, kind, cause, by, step) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![e.at as i64, e.site, e.member, e.kind.code(), e.cause.map(Cause::code), e.by, e.step],
    )?;
    conn.execute("DELETE FROM login_events WHERE at < ?1", [keep_from])?;
    Ok(())
}

pub(crate) fn select(conn: &Connection, since: u64) -> rusqlite::Result<Vec<Entry>> {
    // A journal read without writing (`read`) may be older than the step column.
    let has_step = conn.prepare("SELECT step FROM login_events LIMIT 0").is_ok();
    let sql = if has_step { "SELECT at, site, member, kind, cause, by, step FROM login_events WHERE at >= ?1 ORDER BY at, rowid" } else { "SELECT at, site, member, kind, cause, by, NULL FROM login_events WHERE at >= ?1 ORDER BY at, rowid" };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([since as i64], |r| {
        let kind: String = r.get(3)?;
        let cause: Option<String> = r.get(4)?;
        Ok((r.get::<_, i64>(0)?.max(0) as u64, r.get::<_, String>(1)?, r.get::<_, String>(2)?, kind, cause, r.get::<_, Option<String>>(5)?, r.get::<_, Option<String>>(6)?))
    })?;
    Ok(rows
        .filter_map(Result::ok)
        .filter_map(|(at, site, member, kind, cause, by, step)| Some(Entry { at, site, member, kind: Kind::parse(&kind)?, cause: cause.as_deref().and_then(Cause::parse), by, squad: String::new(), step }))
        .collect())
}

/// Whether `site` was signed in to before, in this record.
pub(crate) fn signed_in_before(conn: &Connection, site: &str) -> bool {
    conn.query_row("SELECT 1 FROM login_events WHERE site = ?1 AND kind = 'signed_in' LIMIT 1", [crate::vault::site_of(site)], |_| Ok(()))
        .optional()
        .ok()
        .flatten()
        .is_some()
}

/// A squad's rows since `since`, read from its journal file without
/// writing to it (for `equipe logins stats`); none from a journal older
/// than the record.
pub fn read(db: &Path, since: u64) -> Result<Vec<Entry>, String> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| format!("{}: {e}", db.display()))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    let has = conn.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'login_events'", [], |_| Ok(())).optional().ok().flatten().is_some();
    if !has {
        return Ok(Vec::new());
    }
    select(&conn, since).map_err(|e| e.to_string())
}

// ── what the rows say ──

const DAY: u64 = 24 * 60 * 60;

/// The Monday (UTC) a moment's week starts on.
pub fn week_of(at: u64) -> u64 {
    // The first of January 1970 was a Thursday.
    let days = at / DAY;
    (days - (days + 3) % 7) * DAY
}

/// One site's handoffs in one week, by cause.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SiteWeek {
    pub site: String,
    /// The week's Monday, unix seconds (UTC).
    pub week: u64,
    pub total: u64,
    pub causes: BTreeMap<Cause, u64>,
}

/// The measures, over the rows given.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Stats {
    /// Every time the person was asked (handoffs and cards).
    pub asked: u64,
    pub by_cause: BTreeMap<Cause, u64>,
    /// Per site per week, most asked first.
    pub site_weeks: Vec<SiteWeek>,
    /// Per site: how long after a sign-in it was first asked for again,
    /// the median, in seconds — and how many such spans there were.
    pub lasts: Vec<(String, u64, u64)>,
    /// Per site: handoffs that came within a day of another member's for
    /// the same site — the same question asked twice.
    pub repeats: Vec<(String, u64)>,
    pub repeats_total: u64,
    /// Sign-ins finished: in all, and by equipe with no one's hand.
    pub signed_in: u64,
    pub signed_in_by_equipe: u64,
    /// Sign-ins equipe finished, by the autopilot step that did it.
    pub resolved: BTreeMap<String, u64>,
    /// Roughly how many minutes of the person's those saved.
    pub minutes_saved: u64,
}

impl Stats {
    /// The share of sign-ins equipe finished by itself, 0–100.
    pub fn by_equipe_percent(&self) -> Option<u64> {
        (self.signed_in > 0).then(|| self.signed_in_by_equipe * 100 / self.signed_in)
    }
}

fn median(mut values: Vec<u64>) -> u64 {
    values.sort_unstable();
    values.get(values.len() / 2).copied().unwrap_or(0)
}

/// What the rows say. Rows of several squads may be mixed: a site is a
/// site, whoever's browser it was in — they share one.
pub fn summarize(entries: &[Entry]) -> Stats {
    let mut rows: Vec<&Entry> = entries.iter().collect();
    rows.sort_by_key(|e| e.at);
    let mut stats = Stats::default();
    let mut weeks: BTreeMap<(String, u64), SiteWeek> = BTreeMap::new();
    for e in rows.iter().filter(|e| e.kind.asks()) {
        let cause = e.cause.unwrap_or(Cause::Other);
        stats.asked += 1;
        *stats.by_cause.entry(cause).or_default() += 1;
        let week = week_of(e.at);
        let w = weeks.entry((e.site.clone(), week)).or_insert_with(|| SiteWeek { site: e.site.clone(), week, total: 0, causes: BTreeMap::new() });
        w.total += 1;
        *w.causes.entry(cause).or_default() += 1;
    }
    stats.site_weeks = weeks.into_values().collect();
    stats.site_weeks.sort_by(|a, b| b.week.cmp(&a.week).then(b.total.cmp(&a.total)).then(a.site.cmp(&b.site)));
    // How long a sign-in lasted: from each one to the next time its site was asked for a sign-in.
    let sites: BTreeSet<&str> = rows.iter().map(|e| e.site.as_str()).collect();
    for site in sites {
        let mut spans = Vec::new();
        let mut since: Option<u64> = None;
        for e in rows.iter().filter(|e| e.site == site) {
            match e.kind {
                Kind::SignedIn => since = Some(e.at),
                k if k.asks() && e.cause.is_some_and(Cause::is_sign_in) => {
                    if let Some(from) = since.take() {
                        spans.push(e.at.saturating_sub(from));
                    }
                }
                _ => {}
            }
        }
        if !spans.is_empty() {
            let n = spans.len() as u64;
            stats.lasts.push((site.to_owned(), median(spans), n));
        }
        // The same site, asked again within a day by another member.
        let asked: Vec<&&Entry> = rows.iter().filter(|e| e.site == site && e.kind.asks()).collect();
        let repeats = asked.iter().enumerate().filter(|(i, e)| asked[..*i].iter().any(|before| before.member != e.member && e.at.saturating_sub(before.at) <= DAY)).count() as u64;
        if repeats > 0 {
            stats.repeats.push((site.to_owned(), repeats));
            stats.repeats_total += repeats;
        }
    }
    stats.repeats.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for e in rows.iter().filter(|e| e.kind == Kind::SignedIn) {
        stats.signed_in += 1;
        if e.by.as_deref() == Some(BY_EQUIPE) {
            stats.signed_in_by_equipe += 1;
            // A sign-in from before steps were kept: the kept password.
            let step = e.step.as_deref().and_then(crate::autopilot::Step::parse).unwrap_or(crate::autopilot::Step::Relogin);
            *stats.resolved.entry(step.code().to_owned()).or_default() += 1;
            stats.minutes_saved += step.minutes_saved();
        }
    }
    stats
}

/// The Logins page's read model over the rows of the last `days` days
/// (`now` is unix seconds): totals by cause, per site, asks per day, and the
/// share of sign-ins equipe finished by itself. `never` and `can_keep` are
/// the vault's to fill in. Counts, hosts and causes only.
pub fn insights(entries: &[Entry], now: u64, days: u32) -> equipe_protocol::vault::LoginInsights {
    use equipe_protocol::vault::{CauseCount, LoginInsights, SiteAsks};
    let days = days.max(1);
    let from = now.saturating_sub(u64::from(days) * DAY);
    let rows: Vec<Entry> = entries.iter().filter(|e| e.at >= from && e.at <= now).cloned().collect();
    let stats = summarize(&rows);
    let counted = |by: &BTreeMap<Cause, u64>| {
        let mut v: Vec<CauseCount> = by.iter().map(|(c, n)| CauseCount { cause: c.code().to_owned(), count: *n }).collect();
        v.sort_by(|a, b| b.count.cmp(&a.count).then(a.cause.cmp(&b.cause)));
        v
    };
    let mut per_site: BTreeMap<&str, (BTreeMap<Cause, u64>, u64, u64)> = BTreeMap::new();
    for e in &rows {
        let site = per_site.entry(e.site.as_str()).or_default();
        if e.kind.asks() {
            *site.0.entry(e.cause.unwrap_or(Cause::Other)).or_default() += 1;
        } else if e.kind == Kind::SignedIn {
            site.1 += 1;
            if e.by.as_deref() == Some(BY_EQUIPE) {
                site.2 += 1;
            }
        }
    }
    let mut sites: Vec<SiteAsks> = per_site
        .into_iter()
        .filter(|(_, (causes, signed, _))| !causes.is_empty() || *signed > 0)
        .map(|(site, (causes, signed_in, signed_in_by_equipe))| SiteAsks {
            site: site.to_owned(),
            asked: causes.values().sum(),
            by_cause: counted(&causes),
            reask_after: stats.lasts.iter().find(|(s, _, _)| s == site).map(|(_, median, _)| *median),
            signed_in,
            signed_in_by_equipe,
        })
        .collect();
    sites.sort_by(|a, b| b.asked.cmp(&a.asked).then(a.site.cmp(&b.site)));
    // Asks per day, the oldest day first; the last bar is the day that holds `now`.
    let mut trend = vec![0u64; days as usize];
    for e in rows.iter().filter(|e| e.kind.asks()) {
        let last = trend.len() - 1;
        let ago = (((now - e.at) / DAY) as usize).min(last);
        trend[last - ago] += 1;
    }
    LoginInsights {
        days,
        at: now,
        asked: stats.asked,
        by_cause: counted(&stats.by_cause),
        sites,
        trend,
        signed_in: stats.signed_in,
        signed_in_by_equipe: stats.signed_in_by_equipe,
        never: Vec::new(),
        can_keep: false,
    }
}

/// Every squad's rows since `since` (they share one browser), each marked
/// with its squad, read without writing to a journal. A squad whose journal
/// can't be read is skipped: the page shows what it can.
pub fn read_all(home: &Path, since: u64) -> Vec<Entry> {
    let mut rows = Vec::new();
    for squad in crate::squads::list(home) {
        let db = crate::journal::squad_dir(home, &squad.name).join("squad.db");
        if let Ok(found) = read(&db, since) {
            rows.extend(found.into_iter().map(|mut row| {
                row.squad = squad.name.clone();
                row
            }));
        }
    }
    rows
}

/// A span of seconds as a person reads it: `40m`, `5h`, `3d`.
pub fn span(secs: u64) -> String {
    match secs {
        0..3_600 => format!("{}m", secs / 60),
        3_600..172_800 => format!("{}h", secs / 3_600),
        _ => format!("{}d", secs / DAY),
    }
}

/// The week's sign-in handoffs, for the PM's retro, in a line: `None` when
/// the person was asked for none.
pub fn retro_line(stats: &Stats) -> Option<String> {
    if stats.asked == 0 && stats.signed_in == 0 {
        return None;
    }
    type Tally = (u64, BTreeMap<Cause, u64>);
    let mut by_site: BTreeMap<&str, Tally> = BTreeMap::new();
    for w in &stats.site_weeks {
        let s = by_site.entry(w.site.as_str()).or_default();
        s.0 += w.total;
        for (c, n) in &w.causes {
            *s.1.entry(*c).or_default() += n;
        }
    }
    let mut sites: Vec<(&str, Tally)> = by_site.into_iter().collect();
    sites.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
    let listed: Vec<String> = sites
        .iter()
        .take(5)
        .map(|(site, (n, causes))| {
            let why: Vec<String> = causes.iter().map(|(c, k)| format!("{} {k}", c.code().replace('_', " "))).collect();
            format!("{site} {n} ({})", why.join(", "))
        })
        .collect();
    let mut line = format!("Sign-ins and checks the person was asked to do in members' browsers this week (counted by equipe): {}", stats.asked);
    if !listed.is_empty() {
        line.push_str(&format!(" — {}", listed.join("; ")));
    }
    line.push('.');
    if stats.repeats_total > 0 {
        line.push_str(&format!(" {} came within a day of another member's for the same site.", stats.repeats_total));
    }
    if let Some(percent) = stats.by_equipe_percent() {
        line.push_str(&format!(" Sign-ins finished: {}, {percent}% by equipe itself with a kept login.", stats.signed_in));
    }
    if !stats.resolved.is_empty() {
        let steps: Vec<String> = stats.resolved.iter().map(|(s, n)| format!("{} {n}", s.replace('_', " "))).collect();
        line.push_str(&format!(" equipe's own steps: {} — about {} minutes of the person's saved.", steps.join(", "), stats.minutes_saved));
    }
    line.push_str(
        " Sites asked for again and again are worth a kept login the person lets equipe sign in with by itself, and its login autopilot \
(codes, keep-alive) in Settings › Accounts; CAPTCHAs stay the person's.",
    );
    Some(line)
}

// ── one ask, not several ──

/// How long a sign-in the person was asked for stays the one ask for that
/// member and site: asked again meanwhile, the member waits on it rather
/// than the person getting another window, panel or notice.
pub const ASK_COALESCE: u64 = 3 * 60;

/// The sign-ins the person was asked for and hasn't answered yet, by squad,
/// member and site, with when. A session that expired mid-step made a member
/// ask for the same site three times in ninety seconds; the person now gets
/// one ask, and the member is told it is still open ([`RecentAsks::open`]).
#[derive(Debug, Default)]
pub struct RecentAsks {
    asks: std::collections::HashMap<(String, String, String), u64>,
}

impl RecentAsks {
    /// The person was just asked to sign `member` of `squad` in to `site`.
    pub fn asked(&mut self, squad: &str, member: &str, site: &str, now: u64) {
        if site.is_empty() {
            return;
        }
        self.asks.retain(|_, at| now.saturating_sub(*at) < ASK_COALESCE);
        self.asks.insert((squad.to_owned(), member.to_owned(), site.to_owned()), now);
    }

    /// How long ago the person was asked to sign `member` in to `site` — a
    /// site that one serves, or one that serves it — when that ask is still
    /// open and recent: `None` once it was answered or is older than
    /// [`ASK_COALESCE`].
    pub fn open(&self, squad: &str, member: &str, site: &str, now: u64) -> Option<u64> {
        if site.is_empty() {
            return None;
        }
        self.asks
            .iter()
            .filter(|((s, m, asked), _)| s == squad && m == member && crate::vault::same_org(asked, site))
            .map(|(_, at)| now.saturating_sub(*at))
            .filter(|ago| *ago < ASK_COALESCE)
            .min()
    }

    /// The person answered `member`'s asks — handed its browser back, or the
    /// sign-in finished: the next ask is a new one.
    pub fn answered(&mut self, squad: &str, member: &str) {
        self.asks.retain(|(s, m, _), _| !(s == squad && m == member));
    }

    /// A sign-in to `site` finished in the browser every member shares: no
    /// ask for it is open any more, whoever asked.
    pub fn signed_in(&mut self, site: &str) {
        if site.is_empty() {
            return;
        }
        self.asks.retain(|(_, _, asked), _| !crate::vault::same_org(asked, site));
    }
}

/// How long a site's sign-in waiting on the person, or refused to equipe's
/// own fill, is told to other members instead of asking the person again.
pub const SITE_WAIT: u64 = 3 * 60 * 60;

/// What stands in the way of signing in to a site for the members other than
/// the one who found out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// A member's browser went to the person for this sign-in, not yet handed back.
    Pending,
    /// equipe's own fill, or a CAPTCHA, didn't get past: the site blocks automation.
    Blocked,
}

/// One site's sign-in standing, by who met it and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteWait {
    pub site: String,
    pub member: String,
    /// The lane that asked, for a pending ask: whose hand-back releases the others.
    pub lane: String,
    pub wait: Wait,
    pub at: u64,
}

/// The sign-ins that wait on the person, or that automation was blocked at,
/// in the last [`SITE_WAIT`]: told to the next member to ask for the same
/// site, whoever it is, before it asks (one question, no second card).
#[derive(Debug, Default)]
pub struct SiteWaits {
    waits: Vec<SiteWait>,
}

impl SiteWaits {
    fn forget_old(&mut self, now: u64) {
        self.waits.retain(|w| now.saturating_sub(w.at) < SITE_WAIT);
    }

    fn put(&mut self, site: &str, member: &str, lane: &str, wait: Wait, now: u64) {
        let site = crate::vault::site_of(site);
        if site.is_empty() {
            return;
        }
        self.forget_old(now);
        // A block outlasts a pending ask of the same site; one entry per site, member and kind.
        self.waits.retain(|w| !(w.wait == wait && w.member == member && crate::vault::same_org(&w.site, &site)));
        self.waits.push(SiteWait { site, member: member.to_owned(), lane: lane.to_owned(), wait, at: now });
    }

    /// `member`'s browser went to the person for a sign-in to `site`.
    pub fn pending(&mut self, site: &str, member: &str, lane: &str, now: u64) {
        self.put(site, member, lane, Wait::Pending, now);
    }

    /// A fill or a CAPTCHA at `site` didn't get through for `member`.
    pub fn blocked(&mut self, site: &str, member: &str, now: u64) {
        self.put(site, member, "", Wait::Blocked, now);
    }

    /// The person handed `member`'s browser back: what waited on it is over (a block stays).
    pub fn answered(&mut self, member: &str) {
        self.waits.retain(|w| !(w.wait == Wait::Pending && w.member == member));
    }

    /// A sign-in to `site` finished: nothing waits or blocks there any more.
    pub fn signed_in(&mut self, site: &str) {
        self.waits.retain(|w| !crate::vault::same_org(&w.site, site));
    }

    /// What another member than `member` has met at `site` lately, a pending ask first.
    pub fn other(&mut self, site: &str, member: &str, now: u64) -> Option<SiteWait> {
        self.forget_old(now);
        let found = self.waits.iter().filter(|w| w.member != member && crate::vault::same_org(&w.site, site));
        found.clone().find(|w| w.wait == Wait::Pending).or_else(|| found.clone().next()).cloned()
    }
}

/// What a member is told, before it asks, when another member's sign-in to
/// `site` waits on the person or was blocked for automation.
pub fn site_wait_told(wait: &SiteWait, now: u64) -> String {
    let ago = span(now.saturating_sub(wait.at));
    let site = &wait.site;
    let who = &wait.member;
    match wait.wait {
        Wait::Pending => format!(
            "The sign-in to {site} is waiting on the person: {who}'s browser was handed to them {ago} ago, and every member shares the browser's sign-ins. \
Don't ask again and don't put up another card: end your turn saying you wait for that sign-in — you get a message when it is done, and carry on then."
        ),
        Wait::Blocked => format!(
            "The sign-in to {site} is blocked for automation: equipe's own fill (or a CAPTCHA) didn't get through for {who} {ago} ago, so another card would fail the same way. \
No card was shown. Hand your browser to the person with `show_my_browser` only if the page really needs a sign-in — they sign in by hand — otherwise carry on without the site."
        ),
    }
}

/// What a member asking again for a sign-in still open is told, `ago`
/// seconds after the person was asked.
pub fn still_asked(site: &str, ago: u64) -> String {
    format!(
        "The person was asked to sign in to {site} for you {ago}s ago, and that ask is still open with them: don't ask again. \
End your turn saying you wait for that sign-in — you get a message when they hand your browser back, and carry on then."
    )
}

/// What a member asking again right after the person handed its browser
/// back from a sign-in to `site` is told: equipe is still seeing whether
/// it went through.
pub fn just_handed_back(site: &str) -> String {
    format!(
        "The person just handed your browser back from signing in to {site}, and equipe is still checking that it went through: \
reload your page there and take a snapshot before asking again. Every member shares the browser's sign-ins, so you are most \
likely signed in. Only if the page still asks you to sign in, call `show_my_browser` again."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEEK: u64 = 7 * DAY;

    fn e(at: u64, site: &str, member: &str, kind: Kind, cause: Option<Cause>, by: Option<&str>) -> Entry {
        Entry::new(at, site, member, kind, cause, by)
    }

    #[test]
    fn the_logins_page_reads_causes_sites_a_trend_and_equipes_share() {
        let now = 1_794_000_000;
        let rows = [
            // Outside the 7 days: only the 30-day look has it.
            e(now - 20 * DAY, "slack.com", "pm", Kind::Handoff, Some(Cause::NoLoginKept), None),
            e(now - 6 * DAY - 10, "github.com", "qa", Kind::SignedIn, None, Some(BY_PERSON)),
            e(now - 2 * DAY, "github.com", "qa", Kind::Handoff, Some(Cause::SessionExpired), None),
            e(now - 2 * DAY + 5, "github.com", "backend", Kind::Card, Some(Cause::SessionExpired), None),
            e(now - 100, "github.com", "backend", Kind::Handoff, Some(Cause::TwoFactor), None),
            e(now - 90, "github.com", "backend", Kind::SignedIn, None, Some(BY_EQUIPE)),
            e(now - 50, "erp.example.com", "pm", Kind::Handoff, None, None),
            // A `sign_in` without a login asks the person nothing yet.
            e(now - 40, "erp.example.com", "pm", Kind::NoLogin, Some(Cause::NoLoginKept), None),
        ];
        let week = insights(&rows, now, 7);
        assert_eq!((week.days, week.asked), (7, 4));
        assert_eq!(week.by_cause[0].cause, "session_expired");
        assert_eq!(week.by_cause[0].count, 2);
        assert_eq!(week.sites.iter().map(|s| (s.site.as_str(), s.asked)).collect::<Vec<_>>(), [("github.com", 3), ("erp.example.com", 1)]);
        let github = &week.sites[0];
        assert_eq!((github.signed_in, github.signed_in_by_equipe), (2, 1));
        assert_eq!(github.reask_after, Some(6 * DAY + 10 - 2 * DAY), "from the sign-in to the first ask again");
        assert_eq!(week.sites[1].by_cause[0].cause, "other", "no cause on record reads as other");
        assert_eq!(week.sites[1].reask_after, None);
        assert_eq!(week.trend.len(), 7);
        assert_eq!(week.trend.iter().sum::<u64>(), 4);
        assert_eq!((week.trend[6], week.trend[5], week.trend[4]), (2, 1, 1), "today's two, then the day-and-a-bit-ago one and the two-days-ago one");
        assert_eq!(week.by_equipe_percent(), Some(50));
        let month = insights(&rows, now, 30);
        assert_eq!((month.trend.len(), month.asked), (30, 5));
        assert!(month.sites.iter().any(|s| s.site == "slack.com"));
        let empty = insights(&[], now, 0);
        assert_eq!((empty.days, empty.trend.len(), empty.asked, empty.by_equipe_percent()), (1, 1, 0, None));
    }

    #[test]
    fn rows_of_every_squad_are_read_without_writing() {
        let home = std::env::temp_dir().join(format!("equipe-logins-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for (squad, site) in [("acme", "github.com"), ("shop", "slack.com")] {
            let journal = crate::journal::Journal::for_squad(&home, squad).unwrap();
            journal.record_login(&e(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()), site, "qa", Kind::Handoff, Some(Cause::Captcha), None)).unwrap();
        }
        let rows = read_all(&home, 0);
        let mut found: Vec<(String, String)> = rows.iter().map(|r| (r.squad.clone(), r.site.clone())).collect();
        found.sort();
        assert_eq!(found, [("acme".to_owned(), "github.com".to_owned()), ("shop".to_owned(), "slack.com".to_owned())]);
        assert!(read_all(&home.join("nowhere"), 0).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_check_and_a_members_words_have_a_cause() {
        assert_eq!(cause_of_check(HumanCheck::SignIn, "https://github.com/login", true), Cause::SessionExpired);
        assert_eq!(cause_of_check(HumanCheck::SignIn, "https://github.com/login", false), Cause::NoLoginKept);
        assert_eq!(cause_of_check(HumanCheck::Challenge, "https://www.linkedin.com/checkpoint/challenge/x", false), Cause::TwoFactor);
        assert_eq!(cause_of_check(HumanCheck::Challenge, "https://example.com/?__cf_chl_tk=1", false), Cause::Captcha);
        assert_eq!(cause_of_check(HumanCheck::TwoFactor, "https://github.com/sessions/two-factor", true), Cause::TwoFactor);
        // What the page waits for comes first.
        assert_eq!(cause_of_handoff("Please sign in", None, Some(HumanCheck::Captcha), true), Cause::Captcha);
        assert_eq!(cause_of_handoff("Approve with your passkey", None, None, true), Cause::Passkey);
        assert_eq!(cause_of_handoff("Paste the device code back", Some("https://device.sso.us-east-1.amazonaws.com/"), None, true), Cause::DeviceCode);
        assert_eq!(cause_of_handoff("Enter the verification code from your phone", None, None, true), Cause::TwoFactor);
        assert_eq!(cause_of_handoff("Click Authorize to give the app access", Some("https://github.com/login/oauth/authorize"), None, true), Cause::Consent);
        assert_eq!(cause_of_handoff("Please log in", Some("https://github.com/login"), None, true), Cause::SessionExpired);
        assert_eq!(cause_of_handoff("Please log in", Some("https://github.com/login"), None, false), Cause::NoLoginKept);
        assert_eq!(cause_of_handoff("Look at this chart", Some("https://example.com/chart"), None, true), Cause::Other);
        // In the person's language too.
        assert_eq!(cause_of_handoff("로그인해 주세요", None, None, false), Cause::NoLoginKept);
    }

    #[test]
    fn handoffs_are_counted_per_site_per_week_and_by_cause() {
        let monday = week_of(1_794_000_000);
        assert_eq!(week_of(monday + 6 * DAY + 3_600), monday, "a week runs Monday to Sunday");
        let rows = [
            e(monday + 10, "github.com", "qa-2", Kind::Handoff, Some(Cause::SessionExpired), None),
            e(monday + 20, "github.com", "backend", Kind::Handoff, Some(Cause::TwoFactor), None),
            e(monday + 30, "github.com", "backend", Kind::Card, Some(Cause::SessionExpired), None),
            e(monday + 40, "erp.example.com", "pm", Kind::Handoff, Some(Cause::Captcha), None),
            e(monday + 50, "github.com", "backend", Kind::NoLogin, Some(Cause::NoLoginKept), None),
            e(monday + WEEK + 5, "github.com", "frontend", Kind::Handoff, Some(Cause::SessionExpired), None),
        ];
        let stats = summarize(&rows);
        assert_eq!(stats.asked, 5, "a member's own `sign_in` without a login asks the person nothing");
        assert_eq!(stats.by_cause[&Cause::SessionExpired], 3);
        let first = stats.site_weeks.iter().find(|w| w.week == monday && w.site == "github.com").unwrap();
        assert_eq!((first.total, first.causes[&Cause::TwoFactor]), (3, 1));
        assert_eq!(stats.site_weeks[0].week, monday + WEEK, "the latest week first");
    }

    #[test]
    fn how_long_a_sign_in_lasts_repeats_by_other_members_and_equipes_share() {
        let t = 1_794_000_000;
        let rows = [
            e(t, "github.com", "qa-2", Kind::SignedIn, None, Some(BY_PERSON)),
            // A step after the password isn't asking again.
            e(t + 60, "github.com", "qa-2", Kind::Handoff, Some(Cause::TwoFactor), None),
            e(t + 5 * 3_600, "github.com", "backend", Kind::Handoff, Some(Cause::SessionExpired), None),
            e(t + 5 * 3_600 + 100, "github.com", "frontend", Kind::Handoff, Some(Cause::SessionExpired), None),
            e(t + 6 * 3_600, "github.com", "backend", Kind::SignedIn, None, Some(BY_EQUIPE)),
            e(t + 9 * 3_600, "github.com", "frontend", Kind::Card, Some(Cause::SessionExpired), None),
            e(t + 9 * 3_600 + 10, "github.com", "frontend", Kind::SignedIn, None, Some(BY_PERSON)),
            // Days apart: not the same question twice.
            e(t + 3 * DAY, "slack.com", "pm", Kind::Handoff, Some(Cause::NoLoginKept), None),
            e(t + 5 * DAY, "slack.com", "qa", Kind::Handoff, Some(Cause::NoLoginKept), None),
        ];
        let stats = summarize(&rows);
        let (site, median, n) = &stats.lasts[0];
        assert_eq!((site.as_str(), *n), ("github.com", 2));
        assert_eq!(*median, 5 * 3_600, "the median of 5h and 3h, the later one");
        assert_eq!(stats.repeats, [("github.com".to_owned(), 3)], "backend after qa-2, frontend after both, and frontend's card");
        assert_eq!(stats.repeats_total, 3);
        assert_eq!((stats.signed_in, stats.signed_in_by_equipe, stats.by_equipe_percent()), (3, 1, Some(33)));
        let line = retro_line(&stats).unwrap();
        assert!(line.contains("github.com 4 (session expired 3, two factor 1); slack.com 2 (no login kept 2)") && line.contains("3 came within a day"), "{line}");
        assert!(line.contains("33% by equipe"), "{line}");
        assert_eq!(retro_line(&Stats::default()), None);
        assert_eq!((span(40 * 60), span(5 * 3_600), span(3 * DAY)), ("40m".to_owned(), "5h".to_owned(), "3d".to_owned()));
    }

    fn host_page(id: u64, url: &str, password: bool, otp: bool) -> crate::browser::HostPage {
        crate::browser::HostPage {
            id,
            url: url.into(),
            title: String::new(),
            loading: false,
            favicon: None,
            signals: Some(crate::browser::Signals { password, otp, ..Default::default() }),
            crashed: false,
        }
    }

    #[test]
    fn a_page_back_from_the_persons_hands_tells_whether_their_sign_in_went_through() {
        // Their own window: the member's page loads again on the site, signed in.
        assert_eq!(back_verdict("github.com", &host_page(3, "https://github.com/acme", false, false)), Back::SignedIn { on: "github.com".into() });
        // Handed over on Google's sign-in, back on the console it was for.
        assert_eq!(
            back_verdict("accounts.google.com", &host_page(3, "https://console.cloud.google.com/home", false, false)),
            Back::SignedIn { on: "console.cloud.google.com".into() }
        );
        // Still on the form, or the code after it: not through.
        assert_eq!(back_verdict("github.com", &host_page(3, "https://github.com/login", true, false)), Back::StillAsks);
        assert_eq!(back_verdict("github.com", &host_page(3, "https://github.com/sessions/two-factor", false, true)), Back::StillAsks);
        // Still loading, still on the provider, or elsewhere: nothing to say yet.
        let mut loading = host_page(3, "https://github.com/acme", false, false);
        loading.loading = true;
        assert_eq!(back_verdict("github.com", &loading), Back::Wait);
        assert_eq!(back_verdict("accounts.google.com", &host_page(3, "https://accounts.google.com/v3/signin/identifier", false, false)), Back::Wait);
        assert_eq!(back_verdict("slack.com", &host_page(3, "https://evil.example/", false, false)), Back::Wait, "another site than the one asked");
    }

    #[test]
    fn a_sign_in_that_lands_on_a_sibling_host_is_a_sign_in_finished() {
        // The sign-in host and the site it leads to are two hosts of one organisation.
        let mut watch = SignInWatch::default();
        for (from, to) in [
            ("https://accounts.google.com/v3/signin/identifier", "https://myaccount.google.com/"),
            ("https://signin.aws.amazon.com/signin", "https://console.aws.amazon.com/console/home"),
            ("https://ap-northeast-2.signin.aws.amazon.com/oauth", "https://ap-northeast-2.console.aws.amazon.com/console/home"),
        ] {
            watch.see("acme/qa-2", &[host_page(5, from, true, false)]);
            let done = watch.see("acme/qa-2", &[host_page(5, to, false, false)]);
            assert_eq!(done.len(), 1, "{from} -> {to}");
            assert_eq!(done[0].1.site, crate::vault::site_of(from));
            assert_eq!(done[0].1.landed, crate::vault::site_of(to), "kept for the host it landed on too");
        }
        // The same host: nothing landed elsewhere. Another organisation, or a shared hosting suffix: no finish.
        watch.see("acme/qa-2", &[host_page(6, "https://github.com/login", true, false)]);
        assert_eq!(watch.see("acme/qa-2", &[host_page(6, "https://github.com/acme", false, false)])[0].1.landed, "");
        watch.see("acme/qa-2", &[host_page(7, "https://login.alice.github.io/login", true, false)]);
        assert!(watch.see("acme/qa-2", &[host_page(7, "https://bob.github.io/", false, false)]).is_empty());
        watch.see("acme/qa-2", &[host_page(8, "https://accounts.google.com/signin", true, false)]);
        assert!(watch.see("acme/qa-2", &[host_page(8, "https://evil.example/", false, false)]).is_empty());
        // Handed back from AWS's sign-in, on its console: through.
        assert_eq!(
            back_verdict("ap-northeast-2.signin.aws.amazon.com", &host_page(3, "https://ap-northeast-2.console.aws.amazon.com/console/home", false, false)),
            Back::SignedIn { on: "ap-northeast-2.console.aws.amazon.com".into() }
        );
        assert_eq!(back_verdict("accounts.google.com", &host_page(3, "https://mail.google.com/mail/u/0/", false, false)), Back::SignedIn { on: "mail.google.com".into() });
    }

    #[test]
    fn a_hand_over_with_no_reason_on_a_sign_in_page_is_a_sign_in() {
        // Said "other" before: never watched for its finish, so never on the record.
        for url in ["https://accounts.google.com/v3/signin/identifier", "https://github.com/login", "https://app.example.com/sso/start"] {
            assert_eq!(cause_of_handoff("Please take over", Some(url), None, false), Cause::NoLoginKept, "{url}");
            assert_eq!(cause_of_handoff("Please take over", Some(url), None, true), Cause::SessionExpired, "{url}");
        }
        assert_eq!(cause_of_handoff("Please take over", Some("https://app.example.com/board"), None, true), Cause::Other);
    }

    #[test]
    fn a_site_waiting_on_the_person_or_blocked_is_told_to_the_other_members() {
        let mut waits = SiteWaits::default();
        let t = 1_000_000;
        assert!(waits.other("accounts.google.com", "backend", t).is_none());
        waits.pending("accounts.google.com", "qa-2", "acme/qa-2", t);
        let told = waits.other("mail.google.com", "backend", t + 600).expect("a sibling host of the same sign-in");
        assert_eq!((told.wait, told.member.as_str(), told.lane.as_str()), (Wait::Pending, "qa-2", "acme/qa-2"));
        let text = site_wait_told(&told, t + 600);
        assert!(text.contains("waiting on the person") && text.contains("qa-2") && text.contains("10m") && text.contains("don't put up another card"), "{text}");
        // Not for the member who asked, nor for another site.
        assert!(waits.other("accounts.google.com", "qa-2", t + 600).is_none() && waits.other("github.com", "backend", t + 600).is_none());
        // Handed back: no longer pending. A block outlasts it, and says so.
        waits.answered("qa-2");
        assert!(waits.other("accounts.google.com", "backend", t + 700).is_none());
        waits.blocked("accounts.google.com", "qa-2", t + 800);
        let blocked = waits.other("accounts.google.com", "backend", t + 900).unwrap();
        assert!(blocked.wait == Wait::Blocked && site_wait_told(&blocked, t + 900).contains("blocked for automation"));
        waits.answered("qa-2");
        assert!(waits.other("accounts.google.com", "backend", t + 900).is_some(), "a hand-back doesn't lift a block");
        // A few hours on, or once signed in: gone.
        assert!(waits.other("accounts.google.com", "backend", t + 800 + SITE_WAIT).is_none());
        waits.blocked("github.com", "qa-2", t);
        waits.signed_in("github.com");
        assert!(waits.other("github.com", "backend", t + 1).is_none());
    }

    #[test]
    fn one_persons_step_on_a_site_serves_every_member_asking_for_it() {
        for cause in [Cause::SessionExpired, Cause::NoLoginKept, Cause::TwoFactor, Cause::Passkey] {
            assert!(cause.is_shared_step(), "{cause:?}");
        }
        for cause in [Cause::Captcha, Cause::Consent, Cause::DeviceCode, Cause::Other] {
            assert!(!cause.is_shared_step(), "{cause:?}");
        }
    }

    #[test]
    fn a_sign_in_page_that_turns_into_its_site_is_a_sign_in_finished() {
        let mut watch = SignInWatch::default();
        assert!(watch.see("acme/qa", &[host_page(7, "https://github.com/login?return_to=%2Facme", true, false)]).is_empty());
        // The code after the password: still signing in.
        assert!(watch.see("acme/qa", &[host_page(7, "https://github.com/sessions/two-factor", false, true)]).is_empty());
        let done = watch.see("acme/qa", &[host_page(7, "https://github.com/acme", false, false)]);
        assert_eq!(done.len(), 1);
        assert_eq!((done[0].0, done[0].1.site.as_str(), done[0].1.url.as_str()), (7, "github.com", "https://github.com/login?return_to=%2Facme"));
        // Once.
        assert!(watch.see("acme/qa", &[host_page(7, "https://github.com/acme/x", false, false)]).is_empty());
        // Gone elsewhere: nothing finished that can be told.
        watch.see("acme/qa", &[host_page(8, "https://github.com/login", true, false)]);
        assert!(watch.see("acme/qa", &[host_page(8, "https://evil.example/", false, false)]).is_empty());
        assert!(watch.see("acme/qa", &[host_page(8, "https://github.com/", false, false)]).is_empty(), "forgotten once it left");
        // A tab closed is forgotten; another lane's pages are its own.
        watch.see("acme/qa", &[host_page(9, "https://github.com/login", true, false)]);
        watch.see("acme/pm", &[]);
        assert_eq!(watch.see("acme/qa", &[host_page(9, "https://github.com/", false, false)]).len(), 1);
    }

    #[test]
    fn a_sign_in_in_the_persons_own_tab_is_seen_in_any_window() {
        let mut watch = SignInWatch::default();
        // No flow: the person's tabs still count, as their own hands.
        assert!(watch.see_person(&[host_page(3, "https://github.com/login", true, false)], None).is_empty());
        let done = watch.see_person(&[host_page(3, "https://github.com/", false, false)], None);
        assert_eq!((done.len(), done[0].1.lane.as_str(), done[0].1.site.as_str()), (1, "", "github.com"));
    }

    #[test]
    fn a_sign_in_started_by_the_person_survives_a_trip_through_an_identity_provider() {
        use crate::signin::Flow;
        let mut watch = SignInWatch::default();
        let mut flow = Flow::new("https://app.example.com/", false, [1], 0);
        let tab = |url: &str, password: bool| vec![host_page(1, "https://old.example/", false, false), host_page(9, url, password, false)];
        // Its tab: the app's home, then its "Sign in" page, then the provider, then back.
        flow.claim(&tab("https://app.example.com/", false));
        assert!(watch.see_person(&tab("https://app.example.com/", false), Some(&flow)).is_empty());
        assert!(watch.see_person(&tab("https://app.example.com/login", true), Some(&flow)).is_empty());
        assert!(watch.see_person(&tab("https://accounts.google.com/o/oauth2/auth?client_id=x", true), Some(&flow)).is_empty());
        let done = watch.see_person(&tab("https://app.example.com/dashboard", false), Some(&flow));
        assert_eq!(done.len(), 1);
        assert_eq!((done[0].0, done[0].1.site.as_str(), done[0].1.url.as_str()), (9, "app.example.com", "https://app.example.com/login"));
    }

    #[test]
    fn a_flow_that_goes_straight_to_the_provider_is_still_the_starting_sites_sign_in() {
        use crate::signin::Flow;
        let mut watch = SignInWatch::default();
        let mut flow = Flow::new("https://app.example.com/start", true, [], 0);
        let tab = |url: &str, password: bool| vec![host_page(4, url, password, false)];
        flow.claim(&tab("https://accounts.google.com/signin", true));
        assert!(watch.see_person(&tab("https://accounts.google.com/signin", true), Some(&flow)).is_empty());
        let done = watch.see_person(&tab("https://app.example.com/home", false), Some(&flow));
        assert_eq!((done.len(), done[0].1.site.as_str(), done[0].1.url.as_str()), (1, "app.example.com", "https://app.example.com/start"));
        // Without the flow it is a sign-in to Google that never came back to Google.
        let mut plain = SignInWatch::default();
        plain.see_person(&tab("https://accounts.google.com/signin", true), None);
        assert!(plain.see_person(&tab("https://app.example.com/home", false), None).is_empty());
    }

    #[test]
    fn equipes_own_sign_in_is_judged_by_where_the_page_went() {
        use crate::vault::Filled;
        let filled = Filled { refused: None, filled: vec!["username".into(), "password".into()], submitted: true, step: Some("password".into()), remembered: 0 };
        let page = |url: &str, check: Option<HumanCheck>, loading: bool| equipe_protocol::BrowserPage { id: 7, url: url.into(), title: String::new(), loading, favicon: None, check, crashed: false };
        assert_eq!(relogin_outcome(&filled, Some(&page("https://github.com/acme", None, false)), "github.com"), Relogin::SignedIn);
        assert_eq!(relogin_outcome(&filled, Some(&page("https://github.com/session", Some(HumanCheck::SignIn), false)), "github.com"), Relogin::FormCameBack);
        assert_eq!(relogin_outcome(&filled, Some(&page("https://github.com/session", Some(HumanCheck::SignIn), true)), "github.com"), Relogin::Person, "still loading: not judged");
        assert_eq!(relogin_outcome(&filled, Some(&page("https://github.com/sessions/two-factor", Some(HumanCheck::TwoFactor), false)), "github.com"), Relogin::Person, "a code next is the person's");
        assert_eq!(relogin_outcome(&filled, None, "github.com"), Relogin::Person);
        let username_only = Filled { step: Some("username".into()), filled: vec!["username".into()], ..filled.clone() };
        assert_eq!(relogin_outcome(&username_only, Some(&page("https://github.com/login", Some(HumanCheck::SignIn), false)), "github.com"), Relogin::Person, "no password typed: nothing says it is wrong");
        let refused = Filled { refused: Some("host".into()), filled: Vec::new(), submitted: false, step: None, remembered: 0 };
        assert_eq!(relogin_outcome(&refused, Some(&page("https://github.com/acme", None, false)), "github.com"), Relogin::Person);
    }

    #[test]
    fn keeping_a_login_is_offered_once_and_never_where_the_person_said_no() {
        let mut offered = std::collections::HashSet::new();
        assert!(offer_wanted("github.com", &[], &[], &offered, true));
        assert!(!offer_wanted("github.com", &[], &[], &offered, false), "nothing to keep it in");
        assert!(!offer_wanted("github.com", &[], &["github.com".into()], &offered, true), "never");
        let kept = crate::protocol::vault::Login { id: 1, site: "github.com".into(), url: None, username: "a".into(), note: None, allowed: Vec::new(), auto: false, stopped: None, created_at: 0, password_at: None, uses: Vec::new(), autopilot: Default::default() };
        assert!(!offer_wanted("https://www.github.com/x", &[kept], &[], &offered, true), "kept already");
        offered.insert("github.com".to_owned());
        assert!(!offer_wanted("github.com", &[], &[], &offered, true), "once");
    }

    #[test]
    fn what_equipe_finished_is_counted_by_step_with_the_minutes_it_saved() {
        use crate::autopilot::Step;
        let t = 1_794_000_000;
        let rows = [
            e(t, "github.com", "qa", Kind::SignedIn, None, Some(BY_EQUIPE)),
            e(t + 10, "github.com", "qa", Kind::SignedIn, None, Some(BY_EQUIPE)).by_step(Step::Totp),
            e(t + 20, "aws.amazon.com", "be", Kind::SignedIn, None, Some(BY_EQUIPE)).by_step(Step::Email),
            e(t + 30, "github.com", "pm", Kind::SignedIn, None, Some(BY_PERSON)),
            e(t + 40, "github.com", "pm", Kind::Handoff, Some(Cause::Captcha), None),
        ];
        let stats = summarize(&rows);
        assert_eq!(stats.resolved.get("relogin"), Some(&1), "from before steps: the kept password");
        assert_eq!((stats.resolved.get("totp"), stats.resolved.get("email")), (Some(&1), Some(&1)));
        assert_eq!(stats.minutes_saved, Step::Relogin.minutes_saved() + Step::Totp.minutes_saved() + Step::Email.minutes_saved());
        let line = retro_line(&stats).unwrap();
        assert!(line.contains("email 1, relogin 1, totp 1") && line.contains("minutes"), "{line}");
        // Kept and read back, and an old journal without the column still reads.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE login_events (at INTEGER NOT NULL, site TEXT NOT NULL, member TEXT NOT NULL, kind TEXT NOT NULL, cause TEXT, by TEXT); INSERT INTO login_events VALUES (1, 'github.com', 'qa', 'signed_in', NULL, 'equipe');").unwrap();
        assert_eq!(select(&conn, 0).unwrap()[0].step, None);
        create_table(&conn).unwrap();
        create_table(&conn).unwrap();
        insert(&conn, &rows[1], 0).unwrap();
        assert_eq!(select(&conn, 0).unwrap()[1].step.as_deref(), Some("totp"));
    }

    #[test]
    fn the_record_keeps_no_more_than_hosts_and_reads_back() {
        let conn = Connection::open_in_memory().unwrap();
        create_table(&conn).unwrap();
        insert(&conn, &e(10, "https://github.com/login?return_to=/secret-path", "qa", Kind::Handoff, Some(Cause::SessionExpired), None), 0).unwrap();
        insert(&conn, &e(20, "github.com", "qa", Kind::SignedIn, None, Some(BY_PERSON)), 0).unwrap();
        let rows = select(&conn, 0).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].site, "github.com", "the host alone: never the path or query");
        assert!(signed_in_before(&conn, "https://www.github.com/x") && !signed_in_before(&conn, "gitlab.com"));
        // Old rows go.
        insert(&conn, &e(30, "gitlab.com", "qa", Kind::Handoff, Some(Cause::Other), None), 25).unwrap();
        assert_eq!(select(&conn, 0).unwrap().len(), 1);
    }

    #[test]
    fn a_page_on_this_machine_is_not_a_sign_in() {
        for site in ["localhost", "localhost:4899", "127.0.0.1", "127.0.0.1:3000", "app.localhost", "::1", "[::1]:8080"] {
            assert!(is_local_page(site), "{site}");
        }
        for site in ["github.com", "accounts.google.com", "localhost.example.com", "128.0.0.1"] {
            assert!(!is_local_page(site), "{site}");
        }
    }

    #[test]
    fn a_member_asking_again_for_the_same_sign_in_waits_on_the_one_ask() {
        let mut asks = RecentAsks::default();
        let t = 1_000_000;
        assert_eq!(asks.open("acme", "qa", "app.example.com", t), None, "nothing asked yet");
        asks.asked("acme", "qa", "app.example.com", t);
        // The session expired mid-step: asked again 40s and 85s later, the same ask.
        assert_eq!(asks.open("acme", "qa", "app.example.com", t + 40), Some(40));
        assert_eq!(asks.open("acme", "qa", "login.app.example.com", t + 85), Some(85), "its sign-in page's host is the same site");
        // Another member, another site, another squad: their own asks.
        assert_eq!(asks.open("acme", "frontend", "app.example.com", t + 40), None);
        assert_eq!(asks.open("acme", "qa", "github.com", t + 40), None);
        assert_eq!(asks.open("other", "qa", "app.example.com", t + 40), None);
        // Past the window, a new ask.
        assert_eq!(asks.open("acme", "qa", "app.example.com", t + ASK_COALESCE), None);
        // Handed back, or signed in: the next ask is a new one.
        asks.asked("acme", "qa", "app.example.com", t + 200);
        asks.answered("acme", "qa");
        assert_eq!(asks.open("acme", "qa", "app.example.com", t + 210), None);
        asks.asked("acme", "qa", "app.example.com", t + 300);
        asks.asked("acme", "frontend", "app.example.com", t + 300);
        asks.signed_in("app.example.com");
        assert_eq!((asks.open("acme", "qa", "app.example.com", t + 310), asks.open("acme", "frontend", "app.example.com", t + 310)), (None, None));
        // An address without a site is never coalesced.
        asks.asked("acme", "qa", "", t);
        assert_eq!(asks.open("acme", "qa", "", t + 1), None);
        assert!(still_asked("app.example.com", 40).contains("don't ask again") && just_handed_back("app.example.com").contains("reload your page"));
    }
}
