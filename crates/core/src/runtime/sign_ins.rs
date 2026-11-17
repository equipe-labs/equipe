//! Signing in once, on the daemon (`crate::signin_store`): every sign-in the
//! person finishes is kept, sealed, per browser profile and site — in the
//! store every squad on the machine shares, or a squad's own where it keeps
//! one — and members' browsers never ask for it again while the site takes it.
//!
//! - kept: a sign-in view's cookies at Done (`window.rs`); a sign-in the
//!   person finished in a member's browser in their hands, in their own
//!   Chrome window, or through "Sign in to a site" — its site's cookies,
//!   asked of the profile's Chrome (`collect`);
//! - put back: whatever a profile's Chrome lacks of it, when the browser
//!   host starts it and when a lane opens on it (`restore`, never over a
//!   cookie it holds — that one may be fresher);
//! - tried first: a member about to hand its browser to the person for a
//!   sign-in gets the login autopilot's sign-in where the person lets it,
//!   else the kept sign-in put back into its browser, once; only if the site
//!   still asks is the person asked — the site ended it, and it goes.
//!
//! A cookie's value is never printed, logged, or shown to a member.

use std::collections::HashSet;

use crate::handoff_view;
use crate::signin_store::{self, Record};
use equipe_protocol::{HandoffCookie, HumanCheck};

use super::*;

/// What the runtime keeps of the squads' kept sign-ins.
#[derive(Default)]
pub(super) struct Kept {
    /// Whether sign-ins are kept at all: their key needs the secret store.
    enabled: bool,
    /// Each store's (`signin_store::store_of`: the shared one, `""`, or a
    /// squad's own), once read.
    read: HashMap<String, Record>,
    /// Stores being read now.
    reading: HashSet<String>,
    /// `(store, profile)` whose kept sign-ins went into the profile's Chrome
    /// since it last started.
    restored: HashSet<(String, String)>,
    /// `(lane, site)` whose kept sign-in was put back before asking the
    /// person: when that sign-in was made.
    tried: HashMap<(String, String), u64>,
    /// Cookies asked of a profile's Chrome to keep, by token: for which
    /// squads, profile and site.
    collecting: HashMap<u64, (Vec<String>, String, String)>,
    next: u64,
    /// Reads and changes off the loop, numbered as begun: what one brings back
    /// is taken only if nothing begun after it was taken already. By store.
    turns: u64,
    taken: HashMap<String, u64>,
}

impl Kept {
    pub(super) fn new(enabled: bool) -> Self {
        Kept { enabled, ..Default::default() }
    }

    fn turn(&mut self) -> u64 {
        self.turns += 1;
        self.turns
    }
}

/// A cookie as the browser host's `cookies` and `restore` take it (Playwright's names).
pub(super) fn host_cookie(c: &HandoffCookie) -> Value {
    json!({ "name": c.name, "value": c.value, "domain": c.domain, "path": c.path, "expires": c.expires, "secure": c.secure, "httpOnly": c.http_only, "sameSite": c.same_site })
}

/// The profile a lane's key is in: `""` the main one.
fn profile_of_key(key: &str) -> String {
    browser::split_profile(key).1.unwrap_or_default().to_owned()
}

impl Runtime {
    /// The store `squad`'s sign-ins are kept in: the shared one, or its own.
    fn sign_in_store(&self, squad: &str) -> String {
        signin_store::store_of(&self.config.home, squad)
    }

    /// Read the kept sign-ins of `squad`'s store off the loop (the secret
    /// store may ask the person first), unless they are read or being read.
    /// The shared store takes in squads' own files from before it first.
    pub(super) fn want_sign_ins(&mut self, squad: &str) {
        let store = self.sign_in_store(squad);
        if !self.kept_sign_ins.enabled || self.kept_sign_ins.read.contains_key(&store) || !self.kept_sign_ins.reading.insert(store.clone()) {
            return;
        }
        let turn = self.kept_sign_ins.turn();
        let (vault, internal, home) = (self.vault.clone(), self.internal.clone(), self.config.home.clone());
        tokio::task::spawn_blocking(move || {
            let record = match vault.lock() {
                Ok(mut v) => {
                    if store == signin_store::SHARED {
                        let squads: Vec<String> = crate::squads::list(&home).into_iter().map(|s| s.name).collect();
                        match signin_store::migrate(&home, &squads, &mut *v, now_secs()) {
                            Ok(moved) if !moved.is_empty() => crate::daemon_log!("equipe: the kept sign-ins of {} now go for every squad", moved.join(", ")),
                            Ok(_) => {}
                            Err(why) => crate::daemon_log!("equipe: squads' kept sign-ins weren't moved into the shared ones: {why}"),
                        }
                    }
                    let name = signin_store::store_name(&store);
                    signin_store::load(&home, &store, &*v).map_err(|why| crate::daemon_log!("equipe: the sign-ins kept for {name} couldn't be read: {why}")).ok()
                }
                Err(_) => None,
            };
            let _ = internal.send(Internal::SignIns { squad: store, record: record.map(Box::new), turn });
        });
    }

    /// A store's kept sign-ins, read or just kept: the profiles with a lane
    /// open, of a squad using that store, that haven't had them since their
    /// Chrome started get them now.
    pub(super) fn sign_ins_read(&mut self, store: &str, record: Option<Record>, turn: u64) {
        self.kept_sign_ins.reading.remove(store);
        let Some(record) = record else { return };
        // Older than what memory holds already (a read begun before a change).
        if self.kept_sign_ins.taken.get(store).is_some_and(|t| *t > turn) {
            return;
        }
        self.kept_sign_ins.taken.insert(store.to_owned(), turn);
        self.kept_sign_ins.read.insert(store.to_owned(), record);
        let lanes: Vec<(String, String)> = self.browsers.iter().filter(|(_, b)| b.is_lane()).map(|(k, _)| k.clone()).collect();
        for (squad, key) in lanes {
            if self.sign_in_store(&squad) == store {
                self.restore_sign_ins(&squad, &key);
            }
        }
    }

    /// The kept sign-ins of the squad's store into the Chrome of `key`'s
    /// profile, once per start of it: only the cookies it lacks.
    pub(super) fn restore_sign_ins(&mut self, squad: &str, key: &str) {
        let profile = profile_of_key(key);
        let store = self.sign_in_store(squad);
        let Some(record) = self.kept_sign_ins.read.get(&store) else {
            self.want_sign_ins(squad);
            return;
        };
        let at = (store, profile.clone());
        if self.kept_sign_ins.restored.contains(&at) {
            return;
        }
        let cookies = record.live(&profile, now_secs());
        if cookies.is_empty() {
            self.kept_sign_ins.restored.insert(at);
            return;
        }
        let mut request = json!({ "op": "restore", "cookies": cookies.iter().map(host_cookie).collect::<Vec<_>>() });
        if !profile.is_empty() {
            browser::with_profile(&mut request, &self.config.home, &profile);
        }
        if self.hub.op(&request) {
            self.kept_sign_ins.restored.insert(at);
        }
    }

    /// A profile's Chrome starts again (the browser host started anew, or
    /// the profile came back from the person's own window): its kept
    /// sign-ins go in again when a lane opens on it. `None`: every profile.
    pub(super) fn sign_ins_restart(&mut self, profile: Option<&str>) {
        match profile {
            Some(p) => self.kept_sign_ins.restored.retain(|(_, q)| q != p),
            None => self.kept_sign_ins.restored.clear(),
        }
    }

    /// Keep a sign-in the person finished to `site` in `profile` for each of
    /// `squads` — in each one's store, once per store: in memory at once (for
    /// the next member's ask), sealed on disk off the loop. Its site may be
    /// tried again before asking.
    pub(super) fn keep_sign_in(&mut self, squads: Vec<String>, profile: &str, site: &str, cookies: Vec<HandoffCookie>) {
        let site = vault::site_of(site);
        if !self.kept_sign_ins.enabled || cookies.is_empty() || site.is_empty() {
            return;
        }
        let now = now_secs();
        self.kept_sign_ins.tried.retain(|(_, s), _| !vault::serves(s, &site));
        let mut stores: Vec<String> = squads.iter().map(|q| self.sign_in_store(q)).collect();
        stores.sort();
        stores.dedup();
        for squad in stores {
            let turn = self.kept_sign_ins.turn();
            if let Some(record) = self.kept_sign_ins.read.get_mut(&squad) {
                record.remember(profile, &site, cookies.clone(), now);
                self.kept_sign_ins.taken.insert(squad.clone(), turn);
            }
            let (vault, internal, home) = (self.vault.clone(), self.internal.clone(), self.config.home.clone());
            let (profile, site, cookies) = (profile.to_owned(), site.clone(), cookies.clone());
            tokio::task::spawn_blocking(move || {
                let kept = match vault.lock() {
                    Ok(mut v) => signin_store::update(&home, &squad, &mut *v, now, |r| r.remember(&profile, &site, cookies, now)),
                    Err(_) => Err("the logins are unusable".into()),
                };
                let name = signin_store::store_name(&squad);
                match kept {
                    Ok(record) => {
                        crate::daemon_log!("equipe: the sign-in to {site} is kept for {name}");
                        let _ = internal.send(Internal::SignIns { squad, record: Some(Box::new(record)), turn });
                    }
                    Err(why) => crate::daemon_log!("equipe: the sign-in to {site} wasn't kept for {name}: {why}"),
                }
            });
        }
    }

    /// The site no longer takes the sign-in kept for it: it goes from `squad`'s store.
    pub(super) fn forget_sign_in(&mut self, squad: &str, profile: &str, site: &str, made: u64) {
        let store = self.sign_in_store(squad);
        let squad = store.as_str();
        let turn = self.kept_sign_ins.turn();
        if let Some(record) = self.kept_sign_ins.read.get_mut(squad) {
            record.forget(profile, site, made);
            self.kept_sign_ins.taken.insert(squad.to_owned(), turn);
        }
        let (vault, home, squad, profile, site) = (self.vault.clone(), self.config.home.clone(), squad.to_owned(), profile.to_owned(), site.to_owned());
        tokio::task::spawn_blocking(move || {
            if let Ok(mut v) = vault.lock()
                && let Err(why) = signin_store::update(&home, &squad, &mut *v, now_secs(), |r| {
                    r.forget(&profile, &site, made);
                })
            {
                crate::daemon_log!("equipe: the ended sign-in to {site} wasn't dropped for {}: {why}", signin_store::store_name(&squad));
            }
        });
    }

    /// The person finished signing in to `site` in `profile`'s Chrome: its
    /// cookies of that site are asked of the browser host, to keep for `squads`.
    pub(super) fn collect_sign_in(&mut self, squads: Vec<String>, profile: &str, site: &str) {
        let site = vault::site_of(site);
        if !self.kept_sign_ins.enabled || site.is_empty() || squads.is_empty() {
            return;
        }
        self.kept_sign_ins.next += 1;
        let token = self.kept_sign_ins.next;
        let mut request = json!({ "op": "collect", "hosts": [site], "token": token });
        if !profile.is_empty() {
            browser::with_profile(&mut request, &self.config.home, profile);
        }
        // A host that never answers leaves a few stale tokens: only the latest are kept.
        if self.kept_sign_ins.collecting.len() > 64
            && let Some(oldest) = self.kept_sign_ins.collecting.keys().min().copied()
        {
            self.kept_sign_ins.collecting.remove(&oldest);
        }
        if self.hub.op(&request) {
            self.kept_sign_ins.collecting.insert(token, (squads, profile.to_owned(), site));
        }
    }

    /// The browser host's answer to `collect`.
    pub(super) fn on_collected(&mut self, token: u64, cookies: Vec<HandoffCookie>) {
        let Some((squads, profile, site)) = self.kept_sign_ins.collecting.remove(&token) else { return };
        let cookies = handoff_view::scope_cookies(cookies, &site, &[]).cookies;
        self.keep_sign_in(squads, &profile, &site, cookies);
    }

    /// Sign-in cookies went into a profile's Chrome, as the host tells it:
    /// for the log — how many, never which values.
    pub(super) fn cookies_went_in(&mut self, lane: Option<&str>, profile: &str, added: usize, failed: usize, restore: bool) {
        let whose = match lane {
            Some(lane) => format!("{lane}'s browser"),
            None if profile.is_empty() => "the members' browser".to_owned(),
            None => format!("the {profile} profile's browser"),
        };
        match (restore, failed) {
            (true, _) if added == 0 && failed == 0 => {}
            (true, 0) => crate::daemon_log!("equipe: {added} kept sign-in cookies went back into {whose}"),
            (true, _) => crate::daemon_log!("equipe: {added} kept sign-in cookies went back into {whose}; Chrome refused {failed}"),
            (false, 0) => crate::daemon_log!("equipe: {added} sign-in cookies went into {whose}"),
            (false, _) => crate::daemon_log!("equipe: {added} of {} sign-in cookies went into {whose}; Chrome refused {failed}", added + failed),
        }
    }

    /// A member is about to hand its browser to the person to sign in
    /// (`show_my_browser`): first what equipe can do by itself. `Some(text)`:
    /// the member's answer — equipe is signing it in with a login the person
    /// lets sign in by itself, or put the sign-in the person made before back
    /// into its browser. `None`: ask the person — nothing kept for that site,
    /// not a sign-in (a code, a CAPTCHA, a passkey are theirs), or the kept
    /// sign-in was put back already and the site still asks: it ended it.
    pub(super) fn try_kept_sign_in(&mut self, call: &ToolCall, url: Option<&str>, why: &str) -> Option<String> {
        let member = self.member_id(&call.squad, &call.member)?;
        let (squad, key) = self.browser_key(member)?;
        let lane = format!("{squad}/{key}");
        let page = self.lane_page(&lane);
        let at = url.map(str::to_owned).or_else(|| page.as_ref().map(|p| p.url.clone())).filter(|u| u.starts_with("http"))?;
        let site = vault::site_of(&at);
        if site.is_empty() {
            return None;
        }
        let on_site = page.as_ref().filter(|p| vault::serves(&site, &p.url));
        let check = on_site.and_then(|p| p.check);
        // A page waiting on a code, a CAPTCHA or a check now is the person's; so
        // are a passkey and a device code, whatever was kept. A sign-in — said
        // to need a code after it, even — is tried first.
        let theirs = matches!(check, Some(c) if c != HumanCheck::SignIn)
            || matches!(crate::logins::cause_of_handoff(why, Some(&at), check, true), crate::logins::Cause::Passkey | crate::logins::Cause::DeviceCode);
        if theirs {
            return None;
        }
        // The login autopilot, where the person lets a login sign in by itself
        // (a page of its browser on that site's sign-in).
        if let Some(p) = on_site.filter(|p| p.check == Some(HumanCheck::SignIn)).cloned()
            && self.try_relogin(&lane, p.id, &p.url)
        {
            return Some(format!(
                "equipe is signing your browser in to {site} by itself, with the login the person keeps for it — don't hand your browser \
to the person. End your turn now saying you wait for that sign-in: you get a message when it is done, and carry on then."
            ));
        }
        // A copy of a sign-in that holds only in the browser that made it (Google's) is
        // refused — worse, taken for a stolen one: what the profile holds is all there is.
        if handoff_view::binds_to_browser(&site) {
            return None;
        }
        let profile = profile_of_key(&key);
        let Some(record) = self.kept_sign_ins.read.get(&self.sign_in_store(&squad)) else {
            self.want_sign_ins(&squad);
            return None;
        };
        let (made, cookies) = record.for_site(&profile, &site, now_secs())?;
        // The browser it would hand over: a lane of the shared Chrome — opened
        // now if it isn't.
        if !self.browsers.contains_key(&(squad.clone(), key.clone())) {
            self.open_browser(&squad, &key, false);
        }
        if !self.browsers.get(&(squad.clone(), key.clone())).is_some_and(|b| b.is_lane()) {
            return None;
        }
        let tried = (lane.clone(), site.clone());
        if self.kept_sign_ins.tried.get(&tried) == Some(&made) {
            // Put back once already, and the site asks again: it ended that sign-in.
            self.kept_sign_ins.tried.remove(&tried);
            self.forget_sign_in(&squad, &profile, &site, made);
            crate::daemon_log!("equipe: {site} no longer takes the sign-in kept for {squad}; {} asks the person", call.member);
            return None;
        }
        self.kept_sign_ins.tried.insert(tried, made);
        // Only what its Chrome lacks: a cookie it holds may be fresher than the kept copy.
        let request = json!({ "op": "cookies", "lane": lane, "cookies": cookies.iter().map(host_cookie).collect::<Vec<_>>(), "url": at, "missing": true });
        if !self.hub.op(&request) {
            return None;
        }
        crate::daemon_log!("equipe: the sign-in to {site} kept for {squad} goes back into {lane}'s browser before the person is asked");
        Some(format!(
            "The person signed in to {site} before, and equipe keeps that sign-in: it is back in your browser now, and {at} loads again — \
don't ask the person. Look at the page (a snapshot): signed in, carry on. Only if it still asks you to sign in, call `show_my_browser` \
again, and say in `why` that the saved sign-in no longer works — then the person is asked."
        ))
    }
}
