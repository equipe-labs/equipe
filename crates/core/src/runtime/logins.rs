//! Logging in once, on the daemon (`crate::vault`, `crate::logins`):
//!
//! - every sign-in handoff, `sign_in` card and finished sign-in is recorded
//!   with its cause (hosts and names only) for `equipe logins stats` and the
//!   weekly retro;
//! - a sign-in the person finishes by hand in a member's browser (a sign-in
//!   page that turns into the site, on the same host) is offered, once, to
//!   be kept — typed into equipe's own form, never through a member;
//! - where the person lets a login sign in by itself, a member's browser
//!   that lands on that site's sign-in page is signed back in by equipe —
//!   only on the login's exact host, rate-limited, stopped (and the person
//!   told once) after a try that didn't get past the form; a code, a
//!   CAPTCHA or a passkey after it goes to the person as before;
//! - several members asking for the same site wait on one card, one
//!   browser in the person's hands, and all carry on when it is done.

use crate::logins::{self, BY_EQUIPE, BY_PERSON, Entry, Kind};
use crate::protocol::vault::{Login, LoginOffer};

use super::*;

/// How many times equipe tries signing in with one login by itself in
/// [`RELOGIN_WINDOW`], and how long it leaves between two tries.
const RELOGIN_TRIES: usize = 3;
const RELOGIN_WINDOW: Duration = Duration::from_secs(60 * 60);
const RELOGIN_GAP: Duration = Duration::from_secs(60);

/// equipe signing a member's browser in by itself, now.
pub(super) struct Relogin {
    login: u32,
    page: u64,
    site: String,
    squad: String,
    member: String,
    /// Times its page was looked at again while still loading.
    looks: u8,
    /// What it is doing: typing the password, or a code or passkey step
    /// (`crate::autopilot`).
    step: crate::autopilot::Step,
    /// On an identity provider's page for another site ("Sign in with …",
    /// [`vault::idp_login_for`]): recorded as `step=idp`, and done only once
    /// the provider sends the page back to the site that asked.
    idp: bool,
}

impl Relogin {
    pub(super) fn new(login: u32, page: u64, site: String, squad: &str, member: &str, step: crate::autopilot::Step) -> Self {
        Relogin { login, page, site, squad: squad.to_owned(), member: member.to_owned(), looks: 0, step, idp: false }
    }

    pub(super) fn via_idp(self, idp: bool) -> Self {
        Relogin { idp, ..self }
    }

    /// The step the sign-in record keeps for it.
    fn recorded(&self) -> crate::autopilot::Step {
        if self.idp { crate::autopilot::Step::Idp } else { self.step }
    }
}

/// equipe's fills waiting for the browser host's answer, by token: shared
/// with the task that waits, which registers before it asks.
pub(super) type FillWaits = std::sync::Arc<std::sync::Mutex<HashMap<u64, oneshot::Sender<Option<String>>>>>;

/// How long a fill waits for the browser host's answer (it holds the tab a
/// few seconds after submitting, `browser-host.cjs`).
const FILL_WAIT: Duration = Duration::from_secs(20);

/// What the person's or equipe's sign-in means for members waiting on it.
const WAITER_PROMPT: &str = "[equipe: the sign-in to {site} you were waiting for is done (in another member's browser — every member shares the \
browser's sign-ins). Reload your page there and carry on; ask the person again only if it still asks you to sign in.]";

impl Runtime {
    /// Whether `site` counts as signed in to before: a login kept for it,
    /// or a sign-in on the squad's record.
    fn site_known(&mut self, squad: &str, site: &str) -> bool {
        self.logins.iter().any(|l| vault::serves(&l.site, site)) || self.journal(squad).is_some_and(|j| j.signed_in_before(site))
    }

    /// Keep a row of the sign-in record (`crate::logins`).
    pub(super) fn login_record(&mut self, squad: &str, entry: Entry) {
        if entry.site.is_empty() {
            return;
        }
        if let Some(journal) = self.journal(squad) {
            let _ = journal.record_login(&entry);
        }
    }

    /// A member hands its browser to the person (`show_my_browser`): kept
    /// with its cause. `None`: go ahead and hand it over. `Some(text)`:
    /// another member's browser is in the person's hands for a sign-in to
    /// the same site already — this one waits on that, told so in `text`.
    pub(super) fn handoff_asked(&mut self, call: &ToolCall, url: Option<&str>, why: &str) -> Option<String> {
        let id = self.caller_id(call)?;
        let (squad, key) = self.browser_key(id)?;
        // Where it is: the address it names, else the tab its member works in.
        let page = self.lane_page(&format!("{squad}/{key}"));
        // The browser the person is handed: the member's own, as `ask_in_panel` hands it.
        let lane = self.member_id(&squad, &call.member).and_then(|m| self.browser_key(m)).map_or_else(|| format!("{squad}/{key}"), |(s, k)| format!("{s}/{k}"));
        let at = url.map(str::to_owned).or_else(|| page.as_ref().map(|p| p.url.clone())).unwrap_or_default();
        let site = vault::site_of(&at);
        let check = page.as_ref().filter(|p| site.is_empty() || vault::serves(&site, &p.url)).and_then(|p| p.check);
        let known = !site.is_empty() && self.site_known(&squad, &site);
        let cause = logins::cause_of_handoff(why, Some(&at), check, known);
        if cause.is_sign_in()
            && !site.is_empty()
            && let Some(other) = self.lane_signing_in_to(&site, &lane)
        {
            self.login_waiters.entry(other.clone()).or_default().push((id, site.clone()));
            let whose = browser::member_of(other.split_once('/').map_or(other.as_str(), |(_, k)| k)).to_owned();
            return Some(format!(
                "The person is signing in to {site} in {whose}'s browser right now, and every member shares the browser's sign-ins. \
Don't hand over yours: end your turn saying you wait for that sign-in — you get a message when it is done, and carry on then."
            ));
        }
        self.login_record(&squad, Entry::new(now_secs(), &site, &call.member, Kind::Handoff, Some(cause), None));
        if cause.is_sign_in() && !site.is_empty() {
            self.asked_site.insert(lane, site);
        }
        None
    }

    /// The tab a lane's member works in, as last told.
    fn lane_page(&self, lane: &str) -> Option<equipe_protocol::BrowserPage> {
        match self.tabs.get(lane) {
            Some(Event::BrowserTabs { pages, agent_page, .. }) => agent_page.and_then(|a| pages.iter().find(|p| p.id == a)).or(pages.last()).cloned(),
            _ => None,
        }
    }

    /// Another lane than `except` in the person's hands for a sign-in to
    /// `site` now: a page of it waiting on a sign-in there, or its member
    /// having asked them in to sign in there.
    fn lane_signing_in_to(&self, site: &str, except: &str) -> Option<String> {
        use equipe_protocol::HumanCheck;
        let checking = self.checking.iter().find(|(lane, (_, url, check))| lane.as_str() != except && matches!(check, HumanCheck::SignIn | HumanCheck::TwoFactor) && vault::serves(site, url)).map(|(lane, _)| lane.clone());
        checking.or_else(|| self.asked_site.iter().find(|(lane, asked)| lane.as_str() != except && self.held.contains_key(lane.as_str()) && vault::serves(site, asked)).map(|(lane, _)| lane.clone()))
    }

    /// A member's page starts waiting on the person: kept with its cause —
    /// unless the person is already on a sign-in to the same site in another
    /// member's browser, which this one waits on (`true`): one question, not two.
    pub(super) fn check_paused(&mut self, lane: &str, url: &str, check: equipe_protocol::HumanCheck) -> bool {
        let Some((squad, key)) = lane.split_once('/') else { return false };
        let site = vault::site_of(url);
        let joined = matches!(check, equipe_protocol::HumanCheck::SignIn) && self.lane_signing_in_to(&site, lane).is_some();
        if joined {
            return true;
        }
        let (squad, member) = (squad.to_owned(), browser::member_of(key).to_owned());
        let known = self.site_known(&squad, &site);
        let cause = logins::cause_of_check(check, url, known);
        self.login_record(&squad, Entry::new(now_secs(), &site, &member, Kind::Handoff, Some(cause), None));
        false
    }

    /// The host's word on a lane's tabs, before screens get them: a page that
    /// was on a sign-in and now is past it, on the same site, is a sign-in
    /// finished — by the person when the browser was in their hands.
    pub(super) async fn follow_sign_ins(&mut self, lane: &str, pages: &[browser::HostPage], touched: bool) {
        if lane.is_empty() {
            return;
        }
        for (page, was) in self.signing_in.see(lane, pages) {
            self.sign_in_finished(lane, page, was, touched).await;
        }
    }

    /// A sign-in finished in `lane`'s page: by equipe's own fill (its check
    /// says so), or by the person — whose hands it was in: handed to them by
    /// its member, or held for them and `touched` (a page its member just
    /// left isn't a sign-in) — then kept on the record, other members' pages
    /// waiting on the same site reloaded, those waiting on it told, and
    /// keeping the login offered.
    async fn sign_in_finished(&mut self, lane: &str, page: u64, was: logins::SigningIn, touched: bool) {
        if self.relogins.get(lane).is_some_and(|r| r.page == page) {
            return;
        }
        let by_person = self.asked_in.contains_key(lane) || (touched && (self.held.contains_key(lane) || self.checking.contains_key(lane)));
        if !by_person {
            return;
        }
        let Some((squad, key)) = lane.split_once('/') else { return };
        let (squad, member, profile) = (squad.to_owned(), browser::member_of(key).to_owned(), browser::split_profile(key).1.is_some());
        self.login_record(&squad, Entry::new(now_secs(), &was.site, &member, Kind::SignedIn, None, Some(BY_PERSON)));
        self.reload_waiting(&was.site, lane);
        self.release_login_waiters(lane).await;
        // Another person's browser keeps their accounts: not the person's to keep in the vault.
        if !profile {
            self.offer_login(&squad, &member, &was.site, &was.url);
        }
    }

    /// Other members' pages waiting on a sign-in to `site`: reloaded, as the
    /// sign-in just made is theirs too — passed, they go back to their
    /// members by themselves.
    pub(super) fn reload_waiting(&mut self, site: &str, except: &str) {
        let waiting: Vec<u64> = self
            .tabs
            .iter()
            .filter(|(lane, _)| lane.as_str() != except && !lane.is_empty() && browser::split_profile(lane.as_str()).1.is_none())
            .flat_map(|(_, tabs)| match tabs {
                Event::BrowserTabs { pages, .. } => pages.iter().filter(|p| p.check == Some(equipe_protocol::HumanCheck::SignIn) && vault::serves(site, &p.url)).map(|p| p.id).collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect();
        for page in waiting {
            self.hub.op(&json!({ "op": "nav", "page": page, "action": "reload" }));
        }
    }

    /// Members that waited on `lane`'s sign-in carry on.
    pub(super) async fn release_login_waiters(&mut self, lane: &str) {
        self.asked_site.remove(lane);
        for (session, site) in self.login_waiters.remove(lane).unwrap_or_default() {
            let text = say!(self, "notice.sign_in_shared", site = site);
            self.carry_on(session, text, WAITER_PROMPT.replace("{site}", &site)).await;
        }
    }

    /// Offer, once, to keep a login for `site` the person just signed in to
    /// by hand: not where one is kept, nor where they said never, nor where
    /// this machine has no secret store to keep it in.
    fn offer_login(&mut self, squad: &str, member: &str, site: &str, url: &str) {
        let site = vault::site_of(site);
        if !logins::offer_wanted(&site, &self.logins, &self.never_offer, &self.login_offered, self.can_keep_logins) {
            return;
        }
        self.login_offered.insert(site.clone());
        let offer = LoginOffer { id: self.next_login_offer, site, url: Some(url.to_owned()).filter(|u| u.starts_with("https://") || u.starts_with("http://")), squad: Some(squad.to_owned()), member: Some(member.to_owned()), at: now_secs() };
        self.next_login_offer += 1;
        self.login_offers.push(offer.clone());
        self.publish(Event::LoginOffered { offer });
    }

    /// Offers for `site` close: a login for it was saved.
    pub(super) fn close_login_offers(&mut self, site: &str) {
        let closed: Vec<u64> = self.login_offers.iter().filter(|o| vault::serves(&o.site, site)).map(|o| o.id).collect();
        self.login_offers.retain(|o| !closed.contains(&o.id));
        for id in closed {
            self.publish(Event::LoginOfferClosed { id });
        }
    }

    /// The person's "not now", or "never" for that site.
    pub(super) fn dismiss_login_offer(&mut self, id: u64, never: bool) {
        let Some(at) = self.login_offers.iter().position(|o| o.id == id) else { return };
        let offer = self.login_offers.remove(at);
        self.publish(Event::LoginOfferClosed { id });
        if never {
            self.never_offer.push(offer.site.clone());
            let site = offer.site;
            self.with_vault(move |v| v.never_offer(&site));
        }
    }

    // ── signing in by itself ──

    /// A member's page waits on a sign-in at `url`: equipe signs in by itself
    /// when the person lets it for a login of that exact host, it is the
    /// person's main browser (a lane of the shared Chrome), and it hasn't
    /// tried too often. `true`: under way — the person isn't asked. On an
    /// identity provider's page for another site, only an opted-in provider
    /// login, and only its identity steps ([`vault::idp_login_for`]).
    pub(super) fn try_relogin(&mut self, lane: &str, page: u64, url: &str) -> bool {
        let Some((squad, key)) = lane.split_once('/') else { return false };
        // Another person's browser: not equipe's to type in.
        if browser::split_profile(key).1.is_some() || self.relogins.contains_key(lane) {
            return false;
        }
        let (login, idp) = match vault::auto_login_for(&self.logins, url) {
            Some(login) => (login.clone(), false),
            None => match vault::idp_login_for(&self.logins, url) {
                Some(login) => (login.clone(), true),
                None => return false,
            },
        };
        if !self.browsers.get(&(squad.to_owned(), key.to_owned())).is_some_and(|b| b.is_lane()) {
            return false;
        }
        let Some(host) = self.host.clone() else { return false };
        let now = std::time::Instant::now();
        let tries = self.relogin_tries.entry(login.id).or_default();
        tries.retain(|t| now.duration_since(*t) < RELOGIN_WINDOW);
        if tries.len() >= RELOGIN_TRIES || tries.last().is_some_and(|t| now.duration_since(*t) < RELOGIN_GAP) {
            return false;
        }
        tries.push(now);
        let site = vault::site_of(url);
        if idp {
            crate::daemon_log!("equipe: signing in on {site} for another site by itself (\"Sign in with …\")");
        }
        self.relogins.insert(lane.to_owned(), Relogin::new(login.id, page, site, squad, browser::member_of(key), crate::autopilot::Step::Relogin).via_idp(idp));
        let (vault, internal, waits, lane) = (self.vault.clone(), self.internal.clone(), self.fill_waits.clone(), lane.to_owned());
        let first_token = self.next_fill_token();
        tokio::spawn(async move {
            let outcome = relogin_fill(&host, &waits, &vault, &lane, page, &login, first_token, idp).await;
            let _ = internal.send(Internal::Relogin { lane, outcome });
        });
        true
    }

    /// Tokens for fills: three each (a sign-in may ask on up to three
    /// pages: a provider's account chooser, the email, the password).
    pub(super) fn next_fill_token(&mut self) -> u64 {
        self.next_fill += 3;
        self.next_fill
    }

    /// The browser host's answer to a fill: to the task waiting on it.
    pub(super) fn filled(&mut self, token: u64, answer: Option<String>) {
        if let Some(wait) = self.fill_waits.lock().ok().and_then(|mut w| w.remove(&token)) {
            let _ = wait.send(answer);
        }
    }

    /// equipe's own sign-in ended: where did the page go? Past the sign-in —
    /// done, kept on the record, the member's conversation told. Back on the
    /// form after a password — the login no longer works: equipe stops
    /// signing in with it by itself and tells the person, once. Anything
    /// else that waits on a person (a code, a CAPTCHA) — theirs, as before.
    pub(super) async fn relogin_done(&mut self, lane: String, outcome: Result<vault::Filled, String>) {
        let Some(r) = self.relogins.remove(&lane) else { return };
        let (session, pages, agent) = match self.tabs.get(&lane) {
            Some(Event::BrowserTabs { session, pages, agent_page, .. }) => (*session, pages.clone(), *agent_page),
            _ => (None, Vec::new(), None),
        };
        let page = pages.iter().find(|p| p.id == r.page);
        // Still loading after the submit: looked at again in a moment, a few times.
        if page.is_some_and(|p| p.loading) && r.looks < 3 {
            let internal = self.internal.clone();
            let again = lane.clone();
            self.relogins.insert(lane, Relogin { looks: r.looks + 1, ..r });
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let _ = internal.send(Internal::Relogin { lane: again, outcome });
            });
            return;
        }
        let username = self.logins.iter().find(|l| l.id == r.login).map(|l| l.username.clone()).unwrap_or_default();
        let filled = match outcome {
            Ok(filled) => filled,
            Err(why) => {
                crate::daemon_log!("equipe: signing in to {} by itself didn't start: {why}", r.site);
                vault::Filled::default()
            }
        };
        let step = r.step;
        let mut outcome = logins::relogin_outcome(&filled, page, &r.site);
        // "Sign in with …": done only once the provider sends the page back
        // to the site that asked. Still on the provider (a consent screen, a
        // step equipe doesn't take) is the member's or the person's.
        if r.idp && outcome == logins::Relogin::SignedIn && page.is_some_and(|p| vault::site_of(&p.url) == r.site) {
            outcome = logins::Relogin::Person;
        }
        match outcome {
            logins::Relogin::FormCameBack if step == crate::autopilot::Step::Relogin => {
                crate::daemon_log!("equipe: signing in to {} by itself didn't get past the form; stopped until the person changes the login", r.site);
                let why = "the sign-in form came back after equipe typed the password in";
                if let Some(login) = self.logins.iter_mut().find(|l| l.id == r.login) {
                    login.stopped = Some(why.to_owned());
                }
                let (id, squad, member) = (r.login, r.squad.clone(), r.member.clone());
                self.with_vault(move |v| {
                    v.used(id, now_secs(), &squad, &member, "failed");
                    v.stop_auto(id, why)
                });
                self.login_record(&r.squad, Entry::new(now_secs(), &r.site, &r.member, Kind::FillFailed, None, Some(BY_EQUIPE)).by_step(r.recorded()));
                let message = say!(self, "error.login_auto_stopped", site = r.site, username = username);
                self.publish(Event::Error { session: None, message, next: None });
            }
            logins::Relogin::SignedIn => {
                let (id, squad, member) = (r.login, r.squad.clone(), r.member.clone());
                self.with_vault(move |v| {
                    v.used(id, now_secs(), &squad, &member, "signed_in_auto");
                    Ok(())
                });
                self.login_record(&r.squad, Entry::new(now_secs(), &r.site, &r.member, Kind::SignedIn, None, Some(BY_EQUIPE)).by_step(r.recorded()));
                if let Some(session) = session {
                    let text = say!(self, "notice.signed_back_in", site = r.site, username = username);
                    self.publish(Event::Notice { undo: None, session, text });
                }
                // A page equipe reloaded because its sign-in ran out: back where it was.
                self.signed_back(r.page, page.map(|p| p.url.as_str()));
                self.reload_waiting(&r.site, &lane);
                self.release_login_waiters(&lane).await;
            }
            // A code that didn't take, nothing typed (another page now, no
            // form), or a step after the password: the person's, as any page
            // that waits on them — or the next autopilot step's.
            logins::Relogin::FormCameBack | logins::Relogin::Person => {
                if step != crate::autopilot::Step::Relogin && filled.submitted && page.is_some_and(|p| p.check.is_some() && vault::site_of(&p.url) == r.site) {
                    self.login_record(&r.squad, Entry::new(now_secs(), &r.site, &r.member, Kind::FillFailed, None, Some(BY_EQUIPE)).by_step(r.recorded()));
                    self.code_failed(r.page);
                }
            }
        }
        // The lane's pages as they are now: one still waiting goes to the person.
        if !pages.is_empty() {
            self.watch_checks(&lane, session, &pages, agent);
        }
    }

    // ── the person's own changes ──

    pub(super) fn allow_login(&mut self, id: u32, who: String) {
        self.with_vault(move |v| v.allow(id, &who));
    }

    pub(super) fn set_login_auto(&mut self, id: u32, on: bool) {
        if on {
            self.relogin_tries.remove(&id);
        }
        self.with_vault(move |v| v.set_auto(id, on));
    }

    /// The person takes back a "Never for this site": offers there may come
    /// again. The page's list is read again once the vault has it.
    pub(super) fn forget_never_offer(&mut self, site: String) {
        let site = vault::site_of(&site);
        self.never_offer.retain(|s| *s != site);
        // Offered again at the next sign-in by hand, not only after a restart.
        self.login_offered.remove(&site);
        let (vault, internal, home) = (self.vault.clone(), self.internal.clone(), self.config.home.clone());
        let now = now_secs();
        let days = self.login_insights_days;
        tokio::task::spawn_blocking(move || {
            if let Ok(mut v) = vault.lock()
                && let Err(why) = v.forget_never(&site)
            {
                crate::daemon_log!("equipe: a never-offer wasn't taken back: {why}");
            }
            let _ = internal.send(Internal::LoginInsights(Box::new(login_insights(&home, &vault, now, days))));
        });
    }

    /// The page's read model over the last `days` days (clamped to 1–90),
    /// read off the loop: the journals are opened read-only.
    pub(super) fn read_login_insights(&mut self, days: u32) {
        let days = days.clamp(1, 90);
        self.login_insights_days = days;
        let (vault, internal, home) = (self.vault.clone(), self.internal.clone(), self.config.home.clone());
        let now = now_secs();
        tokio::task::spawn_blocking(move || {
            let _ = internal.send(Internal::LoginInsights(Box::new(login_insights(&home, &vault, now, days))));
        });
    }

    /// The offers waiting, for a screen that attaches.
    pub(super) fn login_offers_now(&self) -> Vec<LoginOffer> {
        self.login_offers.clone()
    }
}

/// The record's insights with the vault's own facts — the sites never
/// offered, whether a secret store is here — added.
fn login_insights(home: &std::path::Path, vault: &std::sync::Arc<std::sync::Mutex<vault::Vault>>, now: u64, days: u32) -> crate::protocol::vault::LoginInsights {
    let days = days.clamp(1, 90);
    let rows = logins::read_all(home, now.saturating_sub(u64::from(days) * 24 * 60 * 60));
    let mut insights = logins::insights(&rows, now, days);
    if let Ok(v) = vault.lock() {
        insights.never = v.never_offer.clone();
        insights.never.sort();
        insights.can_keep = v.can_keep();
    }
    insights
}

/// equipe types a login into the member's page by itself, through the
/// browser host — not the member's own browser tools, so nothing of it comes
/// back through them, and the host keeps the member's tools off that tab
/// meanwhile. No navigation: the page is on the sign-in already. Only where
/// [`vault::FillGuard::exact`] lets it, checked again in the page as it
/// types. Twice for a sign-in that asks the username and the password on
/// pages of their own. The password is fetched off the loop and goes nowhere
/// but the page.
///
/// `idp`: on an identity provider's page for another site
/// ([`vault::FillGuard::idp`]) — up to three pages (the account chooser,
/// the email, the password), identity only.
#[allow(clippy::too_many_arguments)]
pub(super) async fn relogin_fill(host: &browser::SharedHost, waits: &FillWaits, vault: &std::sync::Arc<std::sync::Mutex<vault::Vault>>, lane: &str, page: u64, login: &Login, first_token: u64, idp: bool) -> Result<vault::Filled, String> {
    let id = login.id;
    let vault = vault.clone();
    let password = tokio::task::spawn_blocking(move || vault.lock().map_err(|_| "the logins are unreadable".to_owned()).and_then(|v| v.password(id)))
        .await
        .map_err(|_| "stopped".to_owned())?
        .map_err(|why| format!("the password couldn't be read from the secret store ({why})"))?;
    let guard = if idp { vault::FillGuard::idp(login) } else { vault::FillGuard::exact(login) };
    let mut last = vault::Filled::default();
    for round in 0..if idp { 3u64 } else { 2 } {
        let token = first_token + round;
        let (tx, rx) = oneshot::channel();
        waits.lock().map_err(|_| "stopped".to_owned())?.insert(token, tx);
        let request = json!({ "op": "fill", "lane": lane, "page": page, "script": vault::fill_script(&login.username, &password, &guard), "token": token });
        host.lock().map_err(|_| "the browser host is gone".to_owned())?.op(&request);
        drop(request);
        let answer = tokio::time::timeout(FILL_WAIT, rx).await.ok().and_then(Result::ok).flatten();
        if let Ok(mut w) = waits.lock() {
            w.remove(&token);
        }
        // No answer read back (the submit's page change can cut the script's
        // own answer off): typed and submitted as far as anyone can tell.
        let filled = answer.as_deref().and_then(vault::fill_answer).unwrap_or_else(|| vault::Filled { filled: vec!["unknown".into()], submitted: true, ..Default::default() });
        let username_only = filled.submitted && (filled.step.as_deref() == Some("username") || (idp && filled.step.as_deref() == Some("account")));
        last = filled;
        if !username_only {
            break;
        }
        // The username alone (or the account picked): its next page asks the password.
        tokio::time::sleep(Duration::from_millis(2500)).await;
    }
    Ok(last)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_waiters_prompt_names_the_site() {
        let text = super::WAITER_PROMPT.replace("{site}", "github.com");
        assert!(text.contains("sign-in to github.com") && text.contains("Reload your page"), "{text}");
    }
}
