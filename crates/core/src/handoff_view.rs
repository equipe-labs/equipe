//! The sign-in view: the person signs in on the screen they are looking at —
//! in a plain web view of the desktop app's browser panel, not driven by
//! anything, so a site's "verify you are human" check sees a person in a
//! genuine browser — and the session they get is handed to the member's
//! browser on the daemon's Mac:
//!
//! 1. the daemon asks the screen for a view ([`protocol::Event::BrowserWindow`]
//!    with a `view`), at the page the member asked for;
//! 2. at Done the screen sends the cookies of the sites it visited in the view
//!    ([`protocol::Command::BrowserCookies`]) over its existing connection —
//!    encrypted already, for a screen on another Mac — and clears the view;
//! 3. the daemon keeps only the cookies of that site and the hosts the sign-in
//!    went through ([`scope_cookies`]), adds them to the member's browser
//!    profile (the host's `cookies` request: Playwright `addCookies`), loads the
//!    page again, and hands the browser back.
//!
//! Only the cookies the sign-in made are carried, never anything else of the
//! screen's browsing, and a cookie's value is never printed. Nothing here
//! disguises the member's automated browser, and nothing answers a check. A
//! site may tie its clearance cookie to the browser or address it was given
//! to: what carries over is the sign-in session, and a member's browser that
//! still meets a check afterwards says so (`challenged`) instead of trying
//! again by itself.
//!
//! This module is the part with no I/O: scoping, and the state of each view.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use equipe_protocol::HandoffCookie;

/// Most cookies one sign-in carries over, and the biggest value of one.
pub const MAX_COOKIES: usize = 300;
pub const MAX_VALUE: usize = 8192;
/// Hosts a view may report.
const MAX_HOSTS: usize = 32;

/// After the cookies went in: how long to give the member's browser to load
/// the page again before handing it back anyway.
pub const BACK_WAIT: Duration = Duration::from_secs(12);
/// …and at least this long, for the page to start loading with its cookies.
pub const BACK_AT_LEAST: Duration = Duration::from_millis(1500);
/// A view whose screen never answers (it left, or it hung) is given up after this.
pub const VIEW_WAIT: Duration = Duration::from_secs(45 * 60);

/// A host name as a web address has it: lowercase letters, digits, `-`, `.`,
/// without a port.
fn host_of(text: &str) -> Option<String> {
    let text = text.trim().to_ascii_lowercase();
    let text = text.split_once("://").map_or(text.as_str(), |(_, rest)| rest);
    let host = text.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next().unwrap_or_default().trim_matches('.');
    let ok = !host.is_empty() && host.len() <= 253 && host.split('.').all(|l| !l.is_empty() && !l.starts_with('-') && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    ok.then(|| host.to_owned())
}

/// Whether a cookie set for `domain` (`.example.com` or `example.com`) is one
/// the sign-in made: its domain is a host the view visited, or a parent of one
/// with a dot of its own (never `com`).
fn domain_is_ours(domain: &str, hosts: &[String]) -> bool {
    let d = domain.trim().trim_start_matches('.').to_ascii_lowercase();
    if !d.contains('.') || !d.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') {
        return false;
    }
    hosts.iter().any(|h| *h == d || h.ends_with(&format!(".{d}")))
}

/// What a sign-in's cookies come to once scoped.
#[derive(Debug, PartialEq)]
pub struct Scoped {
    pub cookies: Vec<HandoffCookie>,
    /// Cookies of other sites, or too many or too big: left out.
    pub dropped: usize,
}

/// Keep the cookies of `site` (the page the member asked for) and of the hosts
/// the view visited meanwhile — the sign-in's own redirects (`auth.openai.com`
/// for `chatgpt.com`; `accounts.google.com` only if the view went there) —
/// and nothing else.
pub fn scope_cookies(cookies: Vec<HandoffCookie>, site: &str, visited: &[String]) -> Scoped {
    let mut hosts: Vec<String> = visited.iter().filter_map(|h| host_of(h)).take(MAX_HOSTS).collect();
    hosts.extend(host_of(site));
    let total = cookies.len();
    let kept: Vec<HandoffCookie> = cookies
        .into_iter()
        .filter(|c| !c.name.is_empty() && c.value.len() <= MAX_VALUE && domain_is_ours(&c.domain, &hosts))
        .take(MAX_COOKIES)
        .map(|mut c| {
            if c.path.is_empty() {
                c.path = "/".to_owned();
            }
            c.domain = c.domain.trim().to_ascii_lowercase();
            c
        })
        .collect();
    Scoped { dropped: total - kept.len(), cookies: kept }
}

/// One view's progress.
#[derive(Debug)]
struct View {
    url: String,
    site: String,
    started: Instant,
    /// Cookies were sent for the member's browser, at this time.
    applied: Option<Instant>,
    /// The person pressed Done, at this time.
    done: Option<Instant>,
}

/// The views open now, by lane.
#[derive(Debug, Default)]
pub struct Views {
    by_lane: BTreeMap<String, View>,
}

/// What a view's cookies came to, for the daemon to carry out.
#[derive(Debug, PartialEq)]
pub enum Taken {
    /// Not a view in progress, or it already took its cookies: nothing.
    Ignored,
    /// The scoped cookies, and the page to load again.
    Apply { url: String, scoped: Scoped },
    /// The screen couldn't open its view.
    Failed,
}

impl Views {
    pub fn is_empty(&self) -> bool {
        self.by_lane.is_empty()
    }

    pub fn lanes(&self) -> Vec<String> {
        self.by_lane.keys().cloned().collect()
    }

    pub fn holds(&self, lane: &str) -> bool {
        self.by_lane.contains_key(lane)
    }

    /// A member hands `lane`'s browser over for `url` (on `site`).
    pub fn begin(&mut self, now: Instant, lane: &str, url: &str, site: &str) {
        self.by_lane.insert(lane.to_owned(), View { url: url.to_owned(), site: site.to_owned(), started: now, applied: None, done: None });
    }

    /// The screen sent its cookies (`failed`: it couldn't open the view).
    /// Only a view in progress that hasn't taken cookies yet takes them.
    pub fn cookies(&mut self, now: Instant, lane: &str, visited: &[String], cookies: Vec<HandoffCookie>, failed: bool) -> Taken {
        let Some(view) = self.by_lane.get_mut(lane) else { return Taken::Ignored };
        if failed {
            self.by_lane.remove(lane);
            return Taken::Failed;
        }
        if view.applied.is_some() {
            return Taken::Ignored;
        }
        view.applied = Some(now);
        Taken::Apply { url: view.url.clone(), scoped: scope_cookies(cookies, &view.site, visited) }
    }

    /// The person pressed Done (or closed the view). Whether this lane was a view.
    pub fn done(&mut self, now: Instant, lane: &str) -> bool {
        let Some(view) = self.by_lane.get_mut(lane) else { return false };
        view.done.get_or_insert(now);
        true
    }

    /// The views to hand back now: Done pressed, their cookies in a moment
    /// ago and the page loaded again (`loaded`), or the wait over. A view the
    /// screen never answered, for [`VIEW_WAIT`], goes too (`true` for stale).
    pub fn ready(&mut self, now: Instant, loaded: impl Fn(&str) -> bool) -> Vec<(String, bool)> {
        let ready: Vec<(String, bool)> = self
            .by_lane
            .iter()
            .filter_map(|(lane, v)| {
                if now.saturating_duration_since(v.started) >= VIEW_WAIT && v.done.is_none() {
                    return Some((lane.clone(), true));
                }
                let done = v.done?;
                // Done with no cookies sent: nothing to wait for.
                let since = v.applied.map_or(done, |a| a.max(done));
                let waited = now.saturating_duration_since(since);
                let page = v.applied.is_none() || (waited >= BACK_AT_LEAST && loaded(lane));
                (page || waited >= BACK_WAIT).then(|| (lane.clone(), false))
            })
            .collect();
        for (lane, _) in &ready {
            self.by_lane.remove(lane);
        }
        ready
    }

    /// What the screen is asked: the page, and the site.
    pub fn ask_of(&self, lane: &str) -> Option<(String, String)> {
        self.by_lane.get(lane).map(|v| (v.url.clone(), v.site.clone()))
    }

    /// This view ends: its page.
    pub fn take(&mut self, lane: &str) -> Option<String> {
        self.by_lane.remove(lane).map(|v| v.url)
    }

    /// Every view ends (the screen they were on is gone): each lane, and its page.
    pub fn end_all(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.by_lane).into_iter().map(|(lane, v)| (lane, v.url)).collect()
    }
}

#[cfg(test)]
mod tests;
