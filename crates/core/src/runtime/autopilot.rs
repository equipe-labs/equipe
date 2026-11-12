//! Login autopilot on the daemon (`crate::autopilot`): what equipe does by
//! itself so the person signs in once and isn't asked again —
//!
//! - a member's page waits on a code after the password: equipe types it,
//!   from the login's authenticator seed, the person's mailbox or Messages
//!   (whichever the person turned on for that login), or presses the
//!   passkey button with its own passkey for the site;
//! - a member's page shows its sign-in ran out (the site answered "not
//!   signed in", or says so) without a form: equipe takes it to the sign-in,
//!   signs back in, and brings it back where it was;
//! - keep-alive: now and then, a look at each site that has it on, in a tab
//!   of equipe's own, signing back in before anyone needs it;
//! - command-line credentials: tokens given to `gh`, an AWS profile's
//!   `credential_process`, values members' commands get in their
//!   environment, a device-code sign-in equipe runs and approves itself.
//!
//! Every one is the person's to turn on (`protocol::vault::Autopilot`, the
//! Settings card); a CAPTCHA is never equipe's — the person gets it in one tap.

use std::collections::HashSet;

use crate::autopilot::{self as ap, Step};
use crate::logins::{BY_EQUIPE, Entry, Kind};
use crate::protocol::vault::{AutopilotStatus, CliKind, Login};

use super::*;

/// equipe's own lanes in the browser host: no member, no tools.
const VISIT_LANE: &str = "equipe:visit";
const DEVICE_LANE: &str = "equipe:device";
/// Code steps equipe tries for one login in an hour.
const CODE_TRIES: usize = 3;
/// Rounds of fills on one page equipe opened for itself.
const VISIT_ROUNDS: u8 = 6;
/// How long a device-code sign-in equipe started may take.
const DEVICE_WAIT: Duration = Duration::from_secs(5 * 60);
/// A GitHub App's token is minted again this long before it runs out.
const APP_TOKEN_EARLY: u64 = 10 * 60;

/// What the host read off a member's page that autopilot uses: whether it
/// offers a passkey, whether its sign-in ran out.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Hints {
    passkey: bool,
    ran_out: bool,
}

/// A page equipe opened for itself, and what for.
pub(super) struct Visit {
    lane: &'static str,
    page: Option<u64>,
    rounds: u8,
    /// The steps that got it signed back in, so far.
    steps: Vec<Step>,
    purpose: Purpose,
}

pub(super) enum Purpose {
    KeepAlive { login: u32 },
    Device { cli: u32, code: Option<String>, hosts: Vec<String> },
}

/// Login autopilot's state on the loop.
#[derive(Default)]
pub(super) struct State {
    /// Values members' commands get in their environment, by credential id
    /// (read from the secret store off the loop).
    cli_values: HashMap<u32, String>,
    loaded: bool,
    keepalive_last: u64,
    visits: HashMap<u64, Visit>,
    next_token: u64,
    /// QR scans and passkey registrations under way: token → login.
    scans: HashMap<u64, u32>,
    registering: HashMap<u64, u32>,
    /// Passkey sign-ins under way: fill token → login.
    passkey_uses: HashMap<u64, u32>,
    /// Pages equipe reloaded because their sign-in ran out: where they were, and when.
    returns: HashMap<u64, (String, std::time::Instant)>,
    /// Pages whose code equipe typed and the site didn't take: never tried again.
    code_failed: HashSet<u64>,
    expiry_tried: HashMap<u64, std::time::Instant>,
    code_tries: HashMap<u32, Vec<std::time::Instant>>,
    hints: HashMap<u64, Hints>,
    /// Device-code sign-ins running, by credential id: the member to tell.
    device_running: HashMap<u32, Option<SessionId>>,
    /// When each GitHub App's token is minted again.
    app_next: HashMap<u32, u64>,
    sms_access: Option<bool>,
}

/// What comes back to the loop from autopilot's work off it.
pub(super) enum Note {
    CliValues(HashMap<u32, String>),
    /// A fill on a page equipe opened for itself ended: look at it again.
    Look { token: u64 },
    /// A change to the vault went through (or didn't): the card again.
    Changed(Result<(), String>),
    Scanned(Result<(), String>),
    /// A command-line credential was given to its CLI (or wasn't).
    Applied { id: u32, result: Result<String, String>, next: Option<u64> },
    DevicePrompt { cli: u32, prompt: ap::cli_logins::DevicePrompt },
    DeviceDone { cli: u32, command: String, result: Result<(), String> },
}

impl Runtime {
    fn ap_token(&mut self) -> u64 {
        // Apart from the fills' own tokens: high, counting down.
        self.autopilot.next_token += 1;
        u64::MAX / 2 - self.autopilot.next_token
    }

    // ── the person's commands ──

    pub(super) fn autopilot_command(&mut self, command: Command) {
        match command {
            Command::SetLoginAutopilot { id, autopilot } => self.with_vault(move |v| v.set_autopilot(id, &autopilot)),
            Command::SetLoginTotp { id, secret } => {
                let seed = match secret.map(|s| ap::totp::Totp::parse(&s.0)) {
                    None => None,
                    Some(Ok(t)) => Some(t.to_uri()),
                    Some(Err(why)) => return self.error(None, say!(self, "error.autopilot_totp", why = why), None),
                };
                self.with_vault(move |v| v.set_totp(id, seed.as_deref()));
            }
            Command::ScanLoginTotp { id } => self.scan_totp(id),
            Command::RegisterLoginPasskey { id } => self.register_passkey(id),
            Command::ForgetLoginPasskey { id } => self.with_vault(move |v| v.set_passkey(id, None)),
            Command::SaveMailbox { mailbox } => self.vault_then_status(move |v| v.save_mailbox(mailbox)),
            Command::SetSmsCodes { on } => {
                self.autopilot.sms_access = self.person_home().and_then(|h| ap::sms::access(&h));
                self.vault_then_status(move |v| v.set_sms(on));
            }
            Command::SetKeepAlive { hours } => self.vault_then_status(move |v| v.set_keepalive_hours(hours)),
            Command::SaveCliLogin { draft } => self.save_cli_login(draft),
            Command::DeleteCliLogin { id } => {
                self.autopilot.cli_values.remove(&id);
                self.autopilot.app_next.remove(&id);
                self.vault_then_status(move |v| v.delete_cli(id));
            }
            Command::Autopilot => self.publish_autopilot(),
            _ => {}
        }
    }

    /// A vault change off the loop (the secret store may ask the person first), then the card again.
    fn vault_then_status(&mut self, change: impl FnOnce(&mut vault::Vault) -> Result<(), String> + Send + 'static) {
        let (vault, internal) = (self.vault.clone(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let result = vault.lock().map_err(|_| "the logins are unreadable".to_owned()).and_then(|mut v| change(&mut v));
            let _ = internal.send(Internal::Autopilot(Note::Changed(result)));
        });
    }

    /// Login autopilot as a whole, to every screen.
    pub(super) fn publish_autopilot(&mut self) {
        let status = self.autopilot_status();
        self.publish(Event::Autopilot { status });
    }

    pub(super) fn autopilot_status(&mut self) -> AutopilotStatus {
        let (settings, cli) = match self.vault.lock() {
            Ok(v) => (v.settings.clone(), v.cli.clone()),
            Err(_) => Default::default(),
        };
        let since = now_secs().saturating_sub(7 * 24 * 3600);
        let rows: Vec<Entry> = self.journals.values().filter_map(|j| j.logins_since(since).ok()).flatten().collect();
        let stats = crate::logins::summarize(&rows);
        if self.autopilot.sms_access.is_none() {
            self.autopilot.sms_access = self.person_home().and_then(|h| ap::sms::access(&h));
        }
        AutopilotStatus {
            mailbox: settings.mailbox,
            sms: settings.sms,
            sms_access: self.autopilot.sms_access,
            keepalive_hours: settings.keepalive_hours,
            cli,
            resolved: stats.resolved.into_iter().collect(),
            asked: stats.asked,
            minutes_saved: stats.minutes_saved,
        }
    }

    pub(super) async fn autopilot_note(&mut self, note: Note) {
        match note {
            Note::CliValues(values) => self.autopilot.cli_values = values,
            Note::Look { token } => self.look_again(token),
            Note::Changed(Ok(())) => {
                self.reload_cli_values();
                self.publish_autopilot();
            }
            Note::Changed(Err(why)) | Note::Scanned(Err(why)) => {
                crate::daemon_log!("equipe: login autopilot: {why}");
                self.error(None, say!(self, "error.autopilot", why = why), None);
            }
            Note::Scanned(Ok(())) => {}
            Note::Applied { id, result, next } => {
                if let Some(next) = next {
                    self.autopilot.app_next.insert(id, next);
                }
                let line = match &result {
                    Ok(line) => line.clone(),
                    Err(why) => format!("failed: {why}"),
                };
                if let Err(why) = &result {
                    crate::daemon_log!("equipe: a command-line credential wasn't applied: {why}");
                    self.error(None, say!(self, "error.autopilot", why = why), None);
                }
                self.vault_then_status(move |v| {
                    v.cli_used(id, &line);
                    Ok(())
                });
            }
            Note::DevicePrompt { cli, prompt } => self.approve_device(cli, prompt),
            Note::DeviceDone { cli, command, result } => self.device_done(cli, command, result).await,
        }
    }

    // ── the minute tick ──

    pub(super) fn autopilot_minute(&mut self) {
        if !self.autopilot.loaded {
            self.autopilot.loaded = true;
            self.reload_cli_values();
            // GitHub Apps' tokens: minted at start, then before each runs out.
            let apps: Vec<u32> = self.vault.lock().map(|v| v.cli.iter().filter(|c| c.kind == CliKind::GhApp).map(|c| c.id).collect()).unwrap_or_default();
            for id in apps {
                self.autopilot.app_next.insert(id, 0);
            }
        }
        let now = now_secs();
        let due: Vec<u32> = self.autopilot.app_next.iter().filter(|(_, at)| **at <= now).map(|(id, _)| *id).collect();
        for id in due {
            self.autopilot.app_next.insert(id, now + 15 * 60);
            self.apply_cli(id);
        }
        self.keepalive_tick(now);
    }

    fn reload_cli_values(&mut self) {
        let (vault, internal) = (self.vault.clone(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let Ok(v) = vault.lock() else { return };
            let values: HashMap<u32, String> = v.cli.iter().filter(|c| c.kind == CliKind::Env).filter_map(|c| v.secret(&vault::account::cli(c.id)).ok().map(|s| (c.id, s))).collect();
            let _ = internal.send(Internal::Autopilot(Note::CliValues(values)));
        });
    }

    /// Values just kept for members' commands: in their next runs at once.
    pub(super) fn keep_run_values(&mut self, values: Vec<(u32, String)>) {
        self.autopilot.cli_values.extend(values);
    }

    /// The environment a member's run in `squad` gets from the person's kept values.
    pub(super) fn member_run_env(&self, squad: &str) -> Vec<(String, String)> {
        let cli = self.vault.lock().map(|v| v.cli.clone()).unwrap_or_default();
        ap::cli_logins::run_env(&cli, &self.autopilot.cli_values, squad)
    }

    // ── a member's page waiting on a person ──

    /// The host's word on a lane's pages, before they are judged: what
    /// autopilot reads off them, and a page whose sign-in ran out on a site
    /// equipe signs in to by itself is taken to its sign-in now.
    pub(super) fn see_pages(&mut self, lane: &str, pages: &[browser::HostPage]) {
        let words = |text: &str| ["passkey", "security key", "webauthn"].iter().chain(crate::i18n::every("match.passkey_words").iter()).any(|w| !w.is_empty() && text.contains(&w.to_lowercase()));
        // What is kept per page, kept small: pages come and go by the thousand.
        if self.autopilot.hints.len() > 4096 {
            self.autopilot.hints.clear();
            self.autopilot.expiry_tried.clear();
            self.autopilot.code_failed.clear();
        }
        for page in pages {
            let ran_out = browser::session_ran_out(&page.title, page.signals.as_ref());
            let passkey = page.signals.as_ref().is_some_and(|s| words(&s.text));
            self.autopilot.hints.insert(page.id, Hints { passkey, ran_out });
        }
        if lane.is_empty() || lane.starts_with("equipe:") || self.relogins.contains_key(lane) {
            return;
        }
        let Some((squad, key)) = lane.split_once('/') else { return };
        if browser::split_profile(key).1.is_some() || !self.browsers.get(&(squad.to_owned(), key.to_owned())).is_some_and(|b| b.is_lane()) {
            return;
        }
        for page in pages {
            let ran_out = self.autopilot.hints.get(&page.id).is_some_and(|h| h.ran_out);
            if !ran_out || page.loading || browser::human_check(&page.url, &page.title, page.signals.as_ref()).is_some() {
                continue;
            }
            let Some(login) = vault::auto_login_for(&self.logins, &page.url).cloned() else { continue };
            let now = std::time::Instant::now();
            if self.autopilot.expiry_tried.get(&page.id).is_some_and(|t| now.duration_since(*t) < Duration::from_secs(120)) {
                continue;
            }
            self.autopilot.expiry_tried.insert(page.id, now);
            // Loaded again: a site whose sign-in ran out shows its sign-in
            // (which equipe then fills, as any), and one that only seemed to
            // shows the page as it was.
            crate::daemon_log!("equipe: {}'s sign-in looks to have run out in a member's page; looking again", login.site);
            self.autopilot.returns.insert(page.id, (page.url.clone(), now));
            self.hub.op(&json!({ "op": "nav", "page": page.id, "action": "reload" }));
        }
    }

    /// equipe signed a member's page back in: one it took to the sign-in
    /// goes back where it was.
    pub(super) fn signed_back(&mut self, page: u64, now_at: Option<&str>) {
        if let Some((url, at)) = self.autopilot.returns.remove(&page)
            && at.elapsed() < Duration::from_secs(5 * 60)
            && now_at != Some(url.as_str())
        {
            self.hub.op(&json!({ "op": "nav", "page": page, "action": "go", "url": url }));
        }
    }

    /// equipe typed a code on `page` and the site didn't take it: that page
    /// goes to the person, and equipe doesn't try a code there again.
    pub(super) fn code_failed(&mut self, page: u64) {
        self.autopilot.code_failed.insert(page);
    }

    /// A member's page waits on a person: equipe finishes it by itself where
    /// the person let it — `true`: under way, nobody is asked.
    pub(super) fn try_autopilot(&mut self, lane: &str, page: u64, url: &str, check: equipe_protocol::HumanCheck) -> bool {
        use equipe_protocol::HumanCheck;
        match check {
            HumanCheck::SignIn => self.try_relogin(lane, page, url) || self.try_code(lane, page, url, true),
            HumanCheck::TwoFactor => self.try_code(lane, page, url, false),
            // A CAPTCHA, a "verify you are human" wall: the person's, always.
            HumanCheck::Captcha | HumanCheck::Challenge => false,
        }
    }

    /// The step equipe can finish a code (or passkey) page of `login` with,
    /// as the person allowed it: the authenticator seed, then equipe's
    /// passkey (where the page offers one), then the mailbox or Messages.
    fn code_step(&self, login: &Login, passkey_offered: bool, sign_in_page: bool) -> Option<Step> {
        let a = &login.autopilot;
        let settings = self.vault.lock().map(|v| v.settings.clone()).unwrap_or_default();
        if sign_in_page {
            return (a.passkey && a.has_passkey && passkey_offered).then_some(Step::Passkey);
        }
        if a.totp && a.has_totp {
            Some(Step::Totp)
        } else if a.passkey && a.has_passkey && passkey_offered {
            Some(Step::Passkey)
        } else if a.email && settings.mailbox.is_some() {
            Some(Step::Email)
        } else if a.sms && settings.sms && cfg!(target_os = "macos") {
            Some(Step::Sms)
        } else {
            None
        }
    }

    /// A code page (or, `sign_in_page`, a sign-in offering a passkey) in a
    /// member's browser, for a login of that exact host: finished by equipe
    /// where the person turned the step on.
    fn try_code(&mut self, lane: &str, page: u64, url: &str, sign_in_page: bool) -> bool {
        let Some((squad, key)) = lane.split_once('/') else { return false };
        if browser::split_profile(key).1.is_some() || self.relogins.contains_key(lane) || !self.browsers.get(&(squad.to_owned(), key.to_owned())).is_some_and(|b| b.is_lane()) {
            return false;
        }
        if self.autopilot.code_failed.contains(&page) {
            return false;
        }
        let Some(login) = autopilot_login_for(&self.logins, url).cloned() else { return false };
        let offered = self.autopilot.hints.get(&page).is_some_and(|h| h.passkey);
        let Some(step) = self.code_step(&login, offered, sign_in_page) else { return false };
        if !self.code_try_allowed(login.id) {
            return false;
        }
        let Some(host) = self.host.clone() else { return false };
        let site = vault::site_of(url);
        self.relogins.insert(lane.to_owned(), logins::Relogin::new(login.id, page, site.clone(), squad, browser::member_of(key), step));
        let job = self.code_job(&login, step);
        let (internal, waits, lane) = (self.internal.clone(), self.fill_waits.clone(), lane.to_owned());
        let token = self.next_fill_token();
        if step == Step::Passkey {
            self.autopilot.passkey_uses.insert(token, login.id);
        }
        crate::daemon_log!("equipe: finishing {site}'s sign-in by itself ({})", step.code());
        tokio::spawn(async move {
            let outcome = code_fill(&host, &waits, &lane, page, job, token).await;
            let _ = internal.send(Internal::Relogin { lane, outcome });
        });
        true
    }

    fn code_try_allowed(&mut self, login: u32) -> bool {
        let now = std::time::Instant::now();
        let tries = self.autopilot.code_tries.entry(login).or_default();
        tries.retain(|t| now.duration_since(*t) < Duration::from_secs(3600));
        if tries.len() >= CODE_TRIES {
            return false;
        }
        tries.push(now);
        true
    }

    /// What a code step needs, gathered on the loop; secrets are read off it.
    fn code_job(&self, login: &Login, step: Step) -> CodeJob {
        let settings = self.vault.lock().map(|v| v.settings.clone()).unwrap_or_default();
        CodeJob {
            vault: self.vault.clone(),
            login: login.clone(),
            step,
            mailbox: settings.mailbox,
            home: self.person_home(),
            since: now_secs().saturating_sub(120),
        }
    }

    // ── keep-alive ──

    fn keepalive_tick(&mut self, now: u64) {
        let hours = self.vault.lock().map(|v| v.settings.keepalive_hours).unwrap_or(0);
        if self.host.is_none() || self.autopilot.visits.values().any(|v| v.lane == VISIT_LANE) {
            return;
        }
        let Some(login) = ap::keepalive::due(&self.logins, hours, now, self.autopilot.keepalive_last).cloned() else { return };
        self.autopilot.keepalive_last = now;
        let token = self.ap_token();
        self.autopilot.visits.insert(token, Visit { lane: VISIT_LANE, page: None, rounds: 0, steps: Vec::new(), purpose: Purpose::KeepAlive { login: login.id } });
        self.hub.op(&json!({ "op": "visit", "lane": VISIT_LANE, "url": ap::keepalive::check_url(&login), "token": token }));
    }

    fn look_again(&mut self, token: u64) {
        let Some(page) = self.autopilot.visits.get(&token).and_then(|v| v.page) else { return };
        self.hub.op(&json!({ "op": "look", "page": page, "token": token }));
    }

    fn end_visit(&mut self, token: u64) -> Option<Visit> {
        let visit = self.autopilot.visits.remove(&token)?;
        if let Some(page) = visit.page {
            self.hub.op(&json!({ "op": "closetab", "page": page }));
            self.autopilot.hints.remove(&page);
        }
        Some(visit)
    }

    /// A page equipe opened for itself, as it is now.
    pub(super) fn on_visited(&mut self, token: u64, page: Option<browser::HostPage>) {
        let Some(visit) = self.autopilot.visits.get_mut(&token) else {
            // Nobody waits for it any more: closed.
            if let Some(p) = page {
                self.hub.op(&json!({ "op": "closetab", "page": p.id }));
            }
            return;
        };
        let Some(page) = page else {
            self.end_visit(token);
            return;
        };
        visit.page = Some(page.id);
        let (lane, rounds) = (visit.lane, visit.rounds);
        self.see_pages(lane, std::slice::from_ref(&page));
        let check = browser::human_check(&page.url, &page.title, page.signals.as_ref());
        let past = browser::past_sign_in(&page.url, &page.title, page.signals.as_ref(), page.loading) && !browser::session_ran_out(&page.title, page.signals.as_ref());
        match &self.autopilot.visits[&token].purpose {
            Purpose::KeepAlive { login } => {
                let login = *login;
                self.keepalive_seen(token, login, &page, check, past, rounds);
            }
            Purpose::Device { cli, code, hosts } => {
                let (cli, code, hosts) = (*cli, code.clone(), hosts.clone());
                self.device_seen(token, cli, code, &hosts, &page, check, rounds);
            }
        }
    }

    fn keepalive_seen(&mut self, token: u64, id: u32, page: &browser::HostPage, check: Option<equipe_protocol::HumanCheck>, past: bool, rounds: u8) {
        use equipe_protocol::HumanCheck;
        let Some(login) = self.logins.iter().find(|l| l.id == id).cloned() else {
            self.end_visit(token);
            return;
        };
        let outcome = match check {
            None if past => {
                let steps = self.end_visit(token).map(|v| v.steps).unwrap_or_default();
                if !steps.is_empty() {
                    // Signed back in before anyone needed it: kept as keep-alive's.
                    let squad = self.squad_of_login(&login);
                    self.login_record(&squad, Entry::new(now_secs(), &login.site, "equipe", Kind::SignedIn, None, Some(BY_EQUIPE)).by_step(Step::Keepalive));
                    crate::daemon_log!("equipe: keep-alive found {} signed out and signed back in", login.site);
                }
                Some(if steps.is_empty() { "signed_in" } else { "signed_back_in" })
            }
            // The form back after equipe typed the password: the kept one no longer works.
            Some(HumanCheck::SignIn) if self.autopilot.visits.get(&token).is_some_and(|v| v.steps.contains(&Step::Relogin)) => {
                self.password_failed(&login, "equipe");
                Some("form_came_back")
            }
            Some(HumanCheck::SignIn) if rounds < VISIT_ROUNDS && login.auto && login.stopped.is_none() && vault::auto_hosts(&login).contains(&vault::site_of(&page.url)) => {
                self.visit_fill(token, &login, page.id, None);
                None
            }
            // A code typed and asked for again: the site didn't take it.
            Some(HumanCheck::TwoFactor) if self.autopilot.visits.get(&token).is_some_and(|v| v.steps.iter().any(|s| *s != Step::Relogin)) => Some("code_not_taken"),
            Some(HumanCheck::SignIn | HumanCheck::TwoFactor) if rounds < VISIT_ROUNDS && autopilot_login_for(std::slice::from_ref(&login), &page.url).is_some() => {
                let offered = self.autopilot.hints.get(&page.id).is_some_and(|h| h.passkey);
                match self.code_step(&login, offered, check == Some(HumanCheck::SignIn)) {
                    Some(step) if self.code_try_allowed(login.id) => {
                        self.visit_fill(token, &login, page.id, Some(step));
                        None
                    }
                    _ => Some("needs_person"),
                }
            }
            // A CAPTCHA, a code equipe can't get, another site: left for when a member needs it.
            Some(HumanCheck::Captcha | HumanCheck::Challenge) => Some("captcha"),
            _ if page.loading && rounds < VISIT_ROUNDS => {
                if let Some(v) = self.autopilot.visits.get_mut(&token) {
                    v.rounds += 1;
                }
                let internal = self.internal.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    let _ = internal.send(Internal::Autopilot(Note::Look { token }));
                });
                None
            }
            _ => Some("needs_person"),
        };
        if let Some(outcome) = outcome {
            self.end_visit(token);
            let look = crate::protocol::vault::Use { at: now_secs(), squad: String::new(), member: "equipe".into(), outcome: outcome.into() };
            if let Some(l) = self.logins.iter_mut().find(|l| l.id == id) {
                l.autopilot.checked = Some(look.clone());
            }
            self.with_vault(move |v| {
                v.checked(id, look);
                Ok(())
            });
        }
    }

    /// The kept password didn't get past the form on a page equipe opened
    /// itself: equipe stops signing in with it and tells the person once, as
    /// for a member's page (`relogin_done`).
    fn password_failed(&mut self, login: &Login, member: &str) {
        let why = "the sign-in form came back after equipe typed the password in";
        if let Some(l) = self.logins.iter_mut().find(|l| l.id == login.id) {
            l.stopped = Some(why.to_owned());
        }
        let id = login.id;
        let (squad, who) = (self.squad_of_login(login), member.to_owned());
        let squad_for_use = squad.clone();
        self.with_vault(move |v| {
            v.used(id, now_secs(), &squad_for_use, &who, "failed");
            v.stop_auto(id, why)
        });
        self.login_record(&squad, Entry::new(now_secs(), &login.site, member, Kind::FillFailed, None, Some(BY_EQUIPE)).by_step(Step::Keepalive));
        let message = say!(self, "error.login_auto_stopped", site = login.site, username = login.username);
        self.publish(Event::Error { session: None, message, next: None });
    }

    /// The squad a login's sign-ins are kept in when no member asked: its latest use's.
    fn squad_of_login(&self, login: &Login) -> String {
        login.uses.first().map(|u| u.squad.clone()).or_else(|| self.journals.keys().next().cloned()).unwrap_or_default()
    }

    /// A fill on a page equipe opened for itself: the password (`step`
    /// `None`), or a code step; then it is looked at again.
    fn visit_fill(&mut self, token: u64, login: &Login, page: u64, step: Option<Step>) {
        let Some(host) = self.host.clone() else { return };
        let Some(visit) = self.autopilot.visits.get_mut(&token) else { return };
        visit.rounds += 1;
        visit.steps.push(step.unwrap_or(Step::Relogin));
        let lane = visit.lane;
        let (internal, waits, vault, login) = (self.internal.clone(), self.fill_waits.clone(), self.vault.clone(), login.clone());
        let fill_token = self.next_fill_token();
        let job = step.map(|s| self.code_job(&login, s));
        if step == Some(Step::Passkey) {
            self.autopilot.passkey_uses.insert(fill_token, login.id);
        }
        tokio::spawn(async move {
            let _ = match job {
                Some(job) => code_fill(&host, &waits, lane, page, job, fill_token).await,
                None => logins::relogin_fill(&host, &waits, &vault, lane, page, &login, fill_token).await,
            };
            let _ = internal.send(Internal::Autopilot(Note::Look { token }));
        });
    }

    // ── device-code sign-ins ──

    /// A member's reply asked the person to run a sign-in command (in a
    /// ```bash block): equipe runs it itself where the person lets it approve
    /// that CLI's device sign-ins.
    pub(super) fn device_login_from_reply(&mut self, session: SessionId, reply: &str) {
        let Some(command) = shell_commands(reply).into_iter().find(|c| ap::cli_logins::device_command(c).is_some()) else { return };
        let Some(cli_name) = ap::cli_logins::cli_of(&command) else { return };
        let cli = self.vault.lock().map(|v| v.cli.clone()).unwrap_or_default();
        let Some(approval) = ap::cli_logins::approval_for(&cli, &self.logins, &cli_name) else { return };
        if self.autopilot.device_running.contains_key(&approval.id) {
            return;
        }
        let Some((program, args)) = ap::cli_logins::device_command(&command) else { return };
        self.autopilot.device_running.insert(approval.id, Some(session));
        let text = say!(self, "notice.autopilot_device", cli = cli_name);
        self.publish(Event::Notice { undo: None, session, text });
        let internal = self.internal.clone();
        let id = approval.id;
        tokio::spawn(async move {
            let notes = internal.clone();
            let result = run_device_login(&program, &args, move |prompt| {
                let _ = notes.send(Internal::Autopilot(Note::DevicePrompt { cli: id, prompt }));
            })
            .await;
            let _ = internal.send(Internal::Autopilot(Note::DeviceDone { cli: id, command, result }));
        });
    }

    /// The CLI printed where to approve it: opened in equipe's own tab of the
    /// shared, signed-in browser — only on the hosts the person named.
    fn approve_device(&mut self, cli: u32, prompt: ap::cli_logins::DevicePrompt) {
        let Some(url) = prompt.url.clone() else { return };
        let Some(login) = self.approval(cli) else { return };
        let hosts = ap::cli_logins::approval_hosts(&login);
        if !ap::cli_logins::on_hosts(&url, &hosts) || url.starts_with("http://") {
            crate::daemon_log!("equipe: {}'s sign-in page isn't on the hosts named for it; left to the person", login.name);
            return;
        }
        if self.autopilot.visits.values().any(|v| matches!(v.purpose, Purpose::Device { cli: c, .. } if c == cli)) {
            return;
        }
        let token = self.ap_token();
        self.autopilot.visits.insert(token, Visit { lane: DEVICE_LANE, page: None, rounds: 0, steps: Vec::new(), purpose: Purpose::Device { cli, code: prompt.code, hosts } });
        self.hub.op(&json!({ "op": "visit", "lane": DEVICE_LANE, "url": url, "token": token }));
    }

    #[allow(clippy::too_many_arguments)]
    fn device_seen(&mut self, token: u64, cli: u32, code: Option<String>, hosts: &[String], page: &browser::HostPage, check: Option<equipe_protocol::HumanCheck>, rounds: u8) {
        use equipe_protocol::HumanCheck;
        if !self.autopilot.device_running.contains_key(&cli) || rounds >= VISIT_ROUNDS {
            self.end_visit(token);
            return;
        }
        // The browser signed out of the approving site: signed back in first,
        // where the person lets it — once: the form back after it is the person's.
        if check == Some(HumanCheck::SignIn)
            && let Some(login) = vault::auto_login_for(&self.logins, &page.url).cloned()
            && !self.autopilot.visits.get(&token).is_some_and(|v| v.steps.contains(&Step::Relogin))
        {
            self.visit_fill(token, &login, page.id, None);
            return;
        }
        let url = page.url.to_ascii_lowercase();
        if check.is_some() || !ap::cli_logins::on_hosts(&url, hosts) || url.starts_with("http://") {
            // Done (the CLI's own callback), or a step that is the person's: nothing more to press.
            self.end_visit(token);
            return;
        }
        let Some(host) = self.host.clone() else { return };
        let guard = vault::FillGuard { hosts: vec![vault::site_of(&page.url)], subdomains: false, http: false, remember: false };
        let script = ap::fill::device_script(code.as_deref(), &guard);
        if let Some(v) = self.autopilot.visits.get_mut(&token) {
            v.rounds += 1;
        }
        let (internal, waits, fill_token) = (self.internal.clone(), self.fill_waits.clone(), self.next_fill_token());
        let page = page.id;
        tokio::spawn(async move {
            let _ = fill_once(&host, &waits, DEVICE_LANE, page, script, fill_token).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            let _ = internal.send(Internal::Autopilot(Note::Look { token }));
        });
    }

    async fn device_done(&mut self, cli: u32, command: String, result: Result<(), String>) {
        let session = self.autopilot.device_running.remove(&cli).flatten();
        let tokens: Vec<u64> = self.autopilot.visits.iter().filter(|(_, v)| matches!(v.purpose, Purpose::Device { cli: c, .. } if c == cli)).map(|(t, _)| *t).collect();
        for token in tokens {
            self.end_visit(token);
        }
        let approval = self.approval(cli);
        let name = approval.as_ref().map(|c| c.name.clone()).unwrap_or_default();
        let line = match &result {
            Ok(()) => "signed in by itself".to_owned(),
            Err(why) => format!("failed: {why}"),
        };
        self.vault_then_status(move |v| {
            v.cli_used(cli, &line);
            Ok(())
        });
        let squad = session.and_then(|s| self.sessions.get(&s)).and_then(|s| s.squad.clone()).unwrap_or_default();
        let member = session.and_then(|s| self.sessions.get(&s)).and_then(|s| s.member.as_ref().map(|m| m.name.to_string())).unwrap_or_default();
        let site = approval.as_ref().and_then(|c| ap::cli_logins::approval_hosts(c).first().cloned()).unwrap_or_default();
        match result {
            Ok(()) => {
                self.login_record(&squad, Entry::new(now_secs(), site.trim_start_matches("*."), &member, Kind::SignedIn, None, Some(BY_EQUIPE)).by_step(Step::DeviceCode));
                if let Some(session) = session {
                    let text = say!(self, "notice.autopilot_device_done", cli = name);
                    let prompt = format!("[equipe: `{command}` is done — equipe ran it itself and approved it in the shared browser, so {name} is signed in again. Don't ask the person to run it; carry on.]");
                    self.carry_on(session, text, prompt).await;
                }
            }
            Err(why) => {
                crate::daemon_log!("equipe: {name}'s device sign-in by itself didn't finish: {why}");
                self.login_record(&squad, Entry::new(now_secs(), site.trim_start_matches("*."), &member, Kind::Handoff, Some(crate::logins::Cause::DeviceCode), None));
                let message = say!(self, "error.autopilot_device", cli = name);
                self.error(session, message, Some(&command));
            }
        }
    }

    /// A device sign-in approval by its number: kept, or a login's own switch's.
    fn approval(&self, id: u32) -> Option<crate::protocol::vault::CliLogin> {
        let cli = self.vault.lock().map(|v| v.cli.clone()).unwrap_or_default();
        ap::cli_logins::approval_by_id(&cli, &self.logins, id)
    }

    // ── command-line credentials ──

    fn save_cli_login(&mut self, draft: crate::protocol::vault::CliLoginDraft) {
        let (vault, internal) = (self.vault.clone(), self.internal.clone());
        let home = self.person_home();
        tokio::task::spawn_blocking(move || {
            let saved = vault.lock().map_err(|_| "the logins are unreadable".to_owned()).and_then(|mut v| v.save_cli(draft, now_secs()).map(|id| (id, v.cli.iter().find(|c| c.id == id).cloned())));
            match saved {
                Ok((id, Some(login))) => {
                    // An AWS profile is set up now; tokens are given to their CLI on the loop's next word.
                    if login.kind == CliKind::AwsKeys {
                        let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "equipe".into());
                        let result = home.ok_or_else(|| "no home folder".to_owned()).and_then(|h| ap::cli_logins::write_aws_profile(&h, &login.name, &exe)).map(|()| format!("profile {} set up", login.name));
                        let _ = internal.send(Internal::Autopilot(Note::Applied { id, result, next: None }));
                    }
                    let _ = internal.send(Internal::Autopilot(Note::Changed(Ok(()))));
                    if matches!(login.kind, CliKind::GhToken | CliKind::GhApp) {
                        let _ = internal.send(Internal::Autopilot(Note::Applied { id, result: Ok("saved".into()), next: Some(0) }));
                    }
                }
                Ok((_, None)) => {}
                Err(why) => {
                    let _ = internal.send(Internal::Autopilot(Note::Changed(Err(why))));
                }
            }
        });
    }

    /// Give a GitHub token (or a GitHub App's fresh installation token) to `gh`.
    fn apply_cli(&mut self, id: u32) {
        let Some(login) = self.vault.lock().ok().and_then(|v| v.cli.iter().find(|c| c.id == id).cloned()) else {
            self.autopilot.app_next.remove(&id);
            return;
        };
        if !matches!(login.kind, CliKind::GhToken | CliKind::GhApp) {
            self.autopilot.app_next.remove(&id);
            return;
        }
        let (vault, internal) = (self.vault.clone(), self.internal.clone());
        tokio::spawn(async move {
            let vault2 = vault.clone();
            let secret = tokio::task::spawn_blocking(move || vault2.lock().map_err(|_| "the logins are unreadable".to_owned()).and_then(|v| v.secret(&vault::account::cli(id)))).await.unwrap_or_else(|_| Err("stopped".into()));
            let (result, next) = match (login.kind, secret) {
                (_, Err(why)) => (Err(format!("its secret couldn't be read ({why})")), None),
                (CliKind::GhToken, Ok(token)) => (ap::cli_logins::gh_token(&login.host, &token).await.map(|()| "given to gh".to_owned()), None),
                (_, Ok(pem)) => match ap::cli_logins::installation_token(&login.public, &pem, now_secs()).await {
                    Ok((token, until)) => (ap::cli_logins::gh_token(&login.host, &token).await.map(|()| "fresh installation token given to gh".to_owned()), Some(until.saturating_sub(APP_TOKEN_EARLY))),
                    Err(why) => (Err(why), Some(now_secs() + 15 * 60)),
                },
            };
            let _ = internal.send(Internal::Autopilot(Note::Applied { id, result, next }));
        });
    }

    // ── authenticator QR codes and passkeys ──

    fn scan_totp(&mut self, id: u32) {
        let Some(hosts) = self.logins.iter().find(|l| l.id == id).map(vault::auto_hosts) else { return };
        let token = self.ap_token();
        self.autopilot.scans.insert(token, id);
        self.hub.op(&json!({ "op": "snap", "hosts": hosts, "token": token }));
    }

    pub(super) fn on_snap(&mut self, token: u64, png: Option<browser::Hidden>) {
        let Some(id) = self.autopilot.scans.remove(&token) else { return };
        let Some(png) = png.and_then(|p| p.0.as_str().map(str::to_owned)) else {
            return self.error(None, say!(self, "error.autopilot_no_qr"), None);
        };
        let (vault, internal) = (self.vault.clone(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            use base64::Engine as _;
            let picture = base64::engine::general_purpose::STANDARD.decode(png).unwrap_or_default();
            let found = ap::qr::otpauth_in(&picture);
            drop(picture);
            let result = match found.map(|uri| ap::totp::Totp::parse(&uri)) {
                Some(Ok(totp)) => vault.lock().map_err(|_| "the logins are unreadable".to_owned()).and_then(|mut v| v.set_totp(id, Some(&totp.to_uri())).map(|()| v.logins.clone())),
                Some(Err(why)) => Err(why),
                None => Err("no authenticator QR code was found on the site's page in equipe's browser".to_owned()),
            };
            match result {
                Ok(logins) => {
                    let _ = internal.send(Internal::Logins(Ok(logins)));
                    let _ = internal.send(Internal::Autopilot(Note::Scanned(Ok(()))));
                }
                Err(why) => {
                    let _ = internal.send(Internal::Autopilot(Note::Scanned(Err(why))));
                }
            }
        });
    }

    fn register_passkey(&mut self, id: u32) {
        let Some(login) = self.logins.iter().find(|l| l.id == id).cloned() else { return };
        let token = self.ap_token();
        self.autopilot.registering.insert(token, id);
        let hosts = vault::auto_hosts(&login);
        self.hub.op(&json!({ "op": "passkey_add", "hosts": hosts, "token": token, "minutes": 10 }));
        // The person's own tab on the site, to add the passkey in.
        let url = login.autopilot.check_url.clone().unwrap_or_else(|| format!("https://{}/", login.site));
        self.hub.op(&json!({ "op": "newtab", "url": url }));
    }

    /// equipe's passkey from the browser host: a new one the person added on
    /// the site (kept, if it is for the login's own site), or one just used
    /// (kept with its counter moved on).
    pub(super) fn on_passkey(&mut self, token: u64, credential: Option<browser::Hidden>) {
        let (id, new) = match (self.autopilot.registering.remove(&token), self.autopilot.passkey_uses.remove(&token)) {
            (Some(id), _) => (id, true),
            (None, Some(id)) => (id, false),
            _ => return,
        };
        let Some(credential) = credential.map(|c| c.0) else {
            if new {
                self.error(None, say!(self, "error.autopilot_no_passkey"), None);
            }
            return;
        };
        let Some(login) = self.logins.iter().find(|l| l.id == id).cloned() else { return };
        let rp = credential.get("rpId").and_then(|r| r.as_str()).unwrap_or_default().to_ascii_lowercase();
        let ours = vault::auto_hosts(&login).iter().any(|h| !rp.is_empty() && rp.contains('.') && (h == &rp || h.ends_with(&format!(".{rp}"))));
        if !ours {
            crate::daemon_log!("equipe: a passkey for another site than {}'s was refused", login.site);
            return;
        }
        let text = credential.to_string();
        drop(credential);
        if new {
            self.with_vault(move |v| v.set_passkey(id, Some(&text)));
        } else {
            self.vault_then_status(move |v| {
                // Only the same credential, its counter moved on.
                let kept: serde_json::Value = serde_json::from_str(&v.passkey(id)?).map_err(|e| e.to_string())?;
                let now: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                if kept.get("credentialId") == now.get("credentialId") { v.set_passkey(id, Some(&text)) } else { Ok(()) }
            });
        }
    }
}

/// The login whose code (or passkey) step equipe may finish at `url`: that
/// exact host, https (http only on this machine), not an OAuth or SSO
/// hand-off, exactly one login, with a code step the person turned on.
pub(super) fn autopilot_login_for<'a>(logins: &'a [Login], url: &str) -> Option<&'a Login> {
    let host = vault::site_of(url);
    if host.is_empty() || vault::hands_off_elsewhere(url) {
        return None;
    }
    let lower = url.trim().to_ascii_lowercase();
    let local = host == "localhost" || host == "127.0.0.1";
    if !lower.starts_with("https://") && !(local && lower.starts_with("http://")) {
        return None;
    }
    let a = |l: &Login| (l.autopilot.totp && l.autopilot.has_totp) || l.autopilot.email || l.autopilot.sms || (l.autopilot.passkey && l.autopilot.has_passkey);
    let mut found = logins.iter().filter(|l| a(l) && vault::auto_hosts(l).contains(&host));
    let one = found.next()?;
    if found.next().is_some() { None } else { Some(one) }
}

/// The shell commands in a reply's ```bash / ```sh blocks, a line each.
fn shell_commands(reply: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in reply.lines() {
        let t = line.trim();
        if let Some(lang) = t.strip_prefix("```") {
            inside = !inside && ["bash", "sh", "shell", "zsh", "console"].contains(&lang.trim());
            continue;
        }
        if inside && !t.is_empty() && !t.starts_with('#') {
            out.push(t.trim_start_matches("$ ").to_owned());
        }
    }
    out
}

/// What a code step needs off the loop.
pub(super) struct CodeJob {
    vault: std::sync::Arc<std::sync::Mutex<vault::Vault>>,
    login: Login,
    step: Step,
    mailbox: Option<crate::protocol::vault::Mailbox>,
    home: Option<PathBuf>,
    since: u64,
}

/// Run one script in a page through the browser host's `fill`, and read its answer.
async fn fill_once(host: &browser::SharedHost, waits: &logins::FillWaits, lane: &str, page: u64, script: String, token: u64) -> Result<vault::Filled, String> {
    let (tx, rx) = oneshot::channel();
    waits.lock().map_err(|_| "stopped".to_owned())?.insert(token, tx);
    host.lock().map_err(|_| "the browser host is gone".to_owned())?.op(&json!({ "op": "fill", "lane": lane, "page": page, "script": script, "token": token }));
    let answer = tokio::time::timeout(Duration::from_secs(25), rx).await.ok().and_then(Result::ok).flatten();
    if let Ok(mut w) = waits.lock() {
        w.remove(&token);
    }
    Ok(answer.as_deref().and_then(vault::fill_answer).unwrap_or_else(|| vault::Filled { filled: vec!["unknown".into()], submitted: true, ..Default::default() }))
}

/// Finish a code (or passkey) step in `lane`'s page: the code made from the
/// seed, or waited for in the mailbox or Messages (up to
/// [`ap::CODE_WAIT_SECS`]), typed in by the browser host; the passkey given
/// to the page for the moment. Secrets are read off the loop and go nowhere
/// but the page.
async fn code_fill(host: &browser::SharedHost, waits: &logins::FillWaits, lane: &str, page: u64, job: CodeJob, token: u64) -> Result<vault::Filled, String> {
    let guard = vault::FillGuard::exact(&job.login);
    let id = job.login.id;
    let read = |what: fn(&vault::Vault, u32) -> Result<String, String>| {
        let vault = job.vault.clone();
        async move { tokio::task::spawn_blocking(move || vault.lock().map_err(|_| "the logins are unreadable".to_owned()).and_then(|v| what(&v, id))).await.map_err(|_| "stopped".to_owned())? }
    };
    let code = match job.step {
        Step::Passkey => {
            let credential: serde_json::Value = serde_json::from_str(&read(vault::Vault::passkey).await?).map_err(|_| "the kept passkey is unreadable".to_owned())?;
            let (tx, rx) = oneshot::channel();
            waits.lock().map_err(|_| "stopped".to_owned())?.insert(token, tx);
            let request = json!({ "op": "passkey_use", "lane": lane, "page": page, "credential": credential, "script": ap::fill::passkey_script(&guard), "token": token });
            host.lock().map_err(|_| "the browser host is gone".to_owned())?.op(&request);
            drop(request);
            let answer = tokio::time::timeout(Duration::from_secs(40), rx).await.ok().and_then(Result::ok).flatten();
            if let Ok(mut w) = waits.lock() {
                w.remove(&token);
            }
            return Ok(answer.as_deref().and_then(vault::fill_answer).unwrap_or_default());
        }
        Step::Totp => {
            let seed = read(vault::Vault::totp).await?;
            ap::totp::Totp::parse(&seed)?.code_to_type(now_secs())
        }
        Step::Email | Step::Sms => {
            let until = std::time::Instant::now() + Duration::from_secs(ap::CODE_WAIT_SECS);
            let mut found = None;
            while found.is_none() && std::time::Instant::now() < until {
                found = match job.step {
                    Step::Email => {
                        let Some(m) = job.mailbox.clone() else { return Err("no mailbox is connected".into()) };
                        let vault = job.vault.clone();
                        let password = tokio::task::spawn_blocking(move || vault.lock().map_err(|_| "stopped".to_owned()).and_then(|v| v.secret(vault::account::MAILBOX))).await.map_err(|_| "stopped".to_owned())??;
                        let wanted = ap::mail::Wanted { site: job.login.site.clone(), mail_from: job.login.autopilot.mail_from.clone(), since: job.since };
                        ap::mail::look_once(&m.host, m.port, &m.username, &password, &wanted).await?
                    }
                    _ => {
                        let db = ap::sms::database(job.home.as_deref().ok_or("no home folder")?);
                        let (site, since) = (job.login.site.clone(), job.since);
                        tokio::task::spawn_blocking(move || ap::sms::code_since(&db, &site, since)).await.map_err(|_| "stopped".to_owned())?.map_err(|r| r.to_string())?
                    }
                };
                if found.is_none() {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
            found.ok_or("no code came in time")?
        }
        Step::Relogin | Step::Keepalive | Step::DeviceCode => return Err("not a code step".into()),
    };
    fill_once(host, waits, lane, page, ap::fill::code_script(&code, &guard), token).await
}

/// Run a device-code sign-in command equipe started itself: what it prints
/// (`on_prompt` gets its page and code once both — or the page alone — are
/// known), Enter for its "press Enter" prompts, and its browser only told
/// the address (`BROWSER=echo`: the shared browser opens it instead).
async fn run_device_login(program: &str, args: &[String], on_prompt: impl Fn(ap::cli_logins::DevicePrompt) + Send + 'static) -> Result<(), String> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .env("BROWSER", "echo")
        .env("GH_BROWSER", "echo")
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    for mut out in [child.stdout.take().map(|o| Box::new(o) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), child.stderr.take().map(|e| Box::new(e) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)].into_iter().flatten() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            while let Ok(n) = out.read(&mut buf).await {
                if n == 0 || tx.send(String::from_utf8_lossy(&buf[..n]).into_owned()).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);
    let mut stdin = child.stdin.take();
    let mut seen = ap::cli_logins::DevicePrompt::default();
    let mut told = false;
    let mut text = String::new();
    let deadline = tokio::time::Instant::now() + DEVICE_WAIT;
    loop {
        tokio::select! {
            chunk = rx.recv() => {
                let Some(chunk) = chunk else { break };
                text.push_str(&chunk);
                for line in chunk.lines() {
                    ap::cli_logins::read_prompt(&mut seen, line);
                }
                // "Press Enter to open … in your browser", "(Y/n)": Enter.
                let lower = chunk.to_ascii_lowercase();
                if (lower.contains("press enter") || lower.contains("(y/n)") || lower.trim_end().ends_with('?'))
                    && let Some(stdin) = stdin.as_mut()
                {
                    let _ = stdin.write_all(b"\n").await;
                }
                if !told && seen.url.is_some() {
                    told = true;
                    on_prompt(seen.clone());
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                let _ = child.kill().await;
                return Err("it took longer than five minutes".into());
            }
        }
    }
    drop(stdin.take());
    let status = tokio::time::timeout_at(deadline, child.wait()).await.map_err(|_| "it didn't finish".to_owned())?.map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("{program} ended with {status}")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::vault::Autopilot;

    fn login(id: u32, site: &str, autopilot: Autopilot) -> Login {
        Login { id, site: site.into(), url: None, username: "a".into(), note: None, allowed: Vec::new(), auto: true, stopped: None, created_at: 0, password_at: None, uses: Vec::new(), autopilot }
    }

    #[test]
    fn a_code_step_is_only_for_the_logins_exact_host_over_https_with_a_step_turned_on() {
        let on = Autopilot { totp: true, has_totp: true, ..Autopilot::default() };
        let logins = [login(1, "github.com", on.clone())];
        assert!(autopilot_login_for(&logins, "https://github.com/sessions/two-factor/app").is_some());
        for elsewhere in ["https://github.com.evil.io/sessions/two-factor", "https://gist.github.com/sessions/two-factor", "http://github.com/sessions/two-factor", "https://github.com/login/oauth/authorize?client_id=x"] {
            assert!(autopilot_login_for(&logins, elsewhere).is_none(), "{elsewhere}");
        }
        assert!(autopilot_login_for(&[login(1, "github.com", Autopilot::default())], "https://github.com/sessions/two-factor").is_none(), "off by default");
        let seed_off = Autopilot { totp: true, has_totp: false, ..Autopilot::default() };
        assert!(autopilot_login_for(&[login(1, "github.com", seed_off)], "https://github.com/sessions/two-factor").is_none(), "no seed kept");
        assert!(autopilot_login_for(&[login(1, "github.com", on.clone()), login(2, "github.com", on)], "https://github.com/sessions/two-factor").is_none(), "two accounts: the person's choice");
    }

    #[test]
    fn a_replys_sign_in_command_is_read_out_of_its_shell_block() {
        let reply = "gh isn't signed in. Run this:\n\n```bash\ngh auth login\n```\n\nThen tell me.";
        assert_eq!(shell_commands(reply), ["gh auth login"]);
        assert!(shell_commands("```rust\ngh auth login\n```").is_empty(), "not a shell block");
        assert_eq!(shell_commands("```sh\n# sign in\n$ aws sso login --profile acme\n```"), ["aws sso login --profile acme"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_device_sign_in_is_run_with_enter_for_its_prompts_and_its_page_told_once() {
        let dir = std::env::temp_dir().join(format!("equipe-device-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let script = dir.join("fake-gh");
        std::fs::write(&script, "#!/bin/sh\necho '! First copy your one-time code: 3A1F-9C2B' >&2\nprintf 'Press Enter to open https://github.com/login/device in your browser... ' >&2\nread x\n\"$BROWSER\" https://github.com/login/device\necho 'Logged in as abel'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let told = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = told.clone();
        let result = run_device_login(script.to_str().unwrap(), &[], move |p| seen.lock().unwrap().push(p)).await;
        assert_eq!(result, Ok(()));
        let told = told.lock().unwrap();
        assert_eq!(told.len(), 1);
        assert_eq!((told[0].url.as_deref(), told[0].code.as_deref()), (Some("https://github.com/login/device"), Some("3A1F-9C2B")));
        let _ = std::fs::remove_dir_all(dir);
    }
}
