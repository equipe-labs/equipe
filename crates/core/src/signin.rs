//! Signing in to a site in equipe's own browser, started by the person: the
//! one way in behind `equipe browser <url>` and the app's "Sign in to a
//! site". This module holds what both share and the tests can see without a
//! browser: reading what the person typed ([`resolve`]), the sites to pick
//! from ([`COMMON_SITES`]), and the sign-in being followed ([`Flow`]) so its
//! end is seen in any window — the browser panel or the separate Chrome
//! window.
//!
//! The runtime opens the page (`Runtime::start_sign_in`) and feeds
//! [`Flow::claim`] and `logins::SignInWatch::see_person` with the person's
//! own tabs; the offer to keep the login follows from there.

use std::collections::HashSet;

use crate::browser::HostPage;
use crate::vault;

/// Sites squads often need, to pick from: name and sign-in page. `equipe
/// browser` without an address lists them on its start page; the app offers
/// them as quick picks.
pub const COMMON_SITES: [(&str, &str); 7] = [
    ("GitHub", "https://github.com/login"),
    ("Vercel", "https://vercel.com/login"),
    ("PostHog", "https://app.posthog.com/login"),
    ("Supabase", "https://supabase.com/dashboard/sign-in"),
    ("Cloudflare", "https://dash.cloudflare.com/login"),
    ("Netlify", "https://app.netlify.com/login"),
    ("Google Search Console", "https://search.google.com/search-console"),
];

/// How long a sign-in is followed for, in seconds: a person may need a
/// phone, a code, a coffee.
pub const FLOW_FOR: u64 = 30 * 60;

/// An address to open from what the person typed or picked: a host
/// (`github.com`), an address (`https://github.com/login`), or the start
/// page equipe made itself (`data:text/html,…`). `None` for anything that
/// isn't a web address — a bare word, spaces, another scheme.
pub fn resolve(input: &str) -> Option<String> {
    let text = input.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) && !text.starts_with("data:") {
        return None;
    }
    if text.starts_with("data:text/html,") {
        return Some(text.to_owned());
    }
    let (scheme, rest) = match text.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        // `host:3000` is a port; `javascript:…` or `mailto:…` is another scheme.
        None if text.split(['/', '?', '#']).next().unwrap_or_default().rsplit_once(':').is_some_and(|(_, port)| port.is_empty() || !port.chars().all(|c| c.is_ascii_digit())) => return None,
        None => (String::new(), text),
    };
    if !scheme.is_empty() && scheme != "http" && scheme != "https" {
        return None;
    }
    let host = vault::site_of(rest);
    let local = host == "localhost" || host.starts_with("127.") || host == "[::1]";
    let labels_ok = host.split('.').all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_'));
    if host.is_empty() || !(local || (host.contains('.') && labels_ok)) {
        return None;
    }
    let scheme = if scheme.is_empty() { if local { "http" } else { "https" } } else { scheme.as_str() };
    Some(format!("{scheme}://{rest}"))
}

/// A sign-in the person started, followed until it is done.
#[derive(Debug, Clone)]
pub struct Flow {
    /// The site, by host; empty for the start page.
    pub site: String,
    /// Where it started.
    pub url: String,
    /// In the separate Chrome window, not a panel.
    pub window: bool,
    /// The person's tabs that were open before it, so its own is told apart.
    known: HashSet<u64>,
    /// The tab it opened, once seen.
    page: Option<u64>,
    started: u64,
}

impl Flow {
    pub fn new(url: &str, window: bool, open_tabs: impl IntoIterator<Item = u64>, now: u64) -> Self {
        let site = if url.starts_with("data:") { String::new() } else { vault::site_of(url) };
        Flow { site, url: url.to_owned(), window, known: open_tabs.into_iter().collect(), page: None, started: now }
    }

    pub fn expired(&self, now: u64) -> bool {
        now.saturating_sub(self.started) > FLOW_FOR
    }

    /// The tab it opened: the first of the person's tabs that wasn't open
    /// before. Once found, it stays that tab.
    pub fn claim(&mut self, pages: &[HostPage]) -> Option<u64> {
        if self.page.is_none() {
            self.page = pages.iter().find(|p| !self.known.contains(&p.id)).map(|p| p.id);
        }
        self.page
    }

    /// Its tab, once claimed.
    pub fn page(&self) -> Option<u64> {
        self.page
    }

    /// Whether its tab was claimed and is gone from `pages`.
    pub fn closed(&self, pages: &[HostPage]) -> bool {
        self.page.is_some_and(|id| !pages.iter().any(|p| p.id == id))
    }
}

/// What the person's own tabs showed of a sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// A tab went from a sign-in to the site: done. `flow`, when it was the
    /// tab of the sign-in the person started.
    SignedIn { was: crate::logins::SigningIn, flow: Option<(String, bool)> },
    /// The tab of the sign-in the person started was closed before one was
    /// seen: its site and whether it was the Chrome window.
    Closed { site: String, window: bool },
}

/// The person's own tabs, followed for sign-ins — in the browser panel or the
/// separate Chrome window alike — and the sign-in the person started, if one
/// is under way. Pure: fed with the tabs as the host tells them.
#[derive(Debug, Default)]
pub struct Tracker {
    watch: crate::logins::SignInWatch,
    flow: Option<Flow>,
}

impl Tracker {
    /// A sign-in the person started: followed from now, instead of any before it.
    pub fn start(&mut self, flow: Flow) {
        self.flow = Some(flow);
    }

    pub fn pending(&self) -> Option<&Flow> {
        self.flow.as_ref()
    }

    /// The person's tabs as told now, at unix time `now`.
    pub fn see(&mut self, pages: &[HostPage], now: u64) -> Vec<Seen> {
        if self.flow.as_ref().is_some_and(|f| f.expired(now)) {
            self.flow = None;
        }
        if let Some(flow) = self.flow.as_mut() {
            flow.claim(pages);
        }
        let mut out = Vec::new();
        for (page, was) in self.watch.see_person(pages, self.flow.as_ref()) {
            let own = self.flow.as_ref().is_some_and(|f| f.page() == Some(page));
            let flow = if own { self.flow.take().map(|f| (f.site, f.window)) } else { None };
            out.push(Seen::SignedIn { was, flow });
        }
        if self.flow.as_ref().is_some_and(|f| f.closed(pages))
            && let Some(f) = self.flow.take()
        {
            out.push(Seen::Closed { site: f.site, window: f.window });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_or_an_address_is_opened_and_anything_else_is_not() {
        assert_eq!(resolve("github.com").as_deref(), Some("https://github.com"));
        assert_eq!(resolve("  https://github.com/login ").as_deref(), Some("https://github.com/login"));
        assert_eq!(resolve("app.posthog.com/login?x=1").as_deref(), Some("https://app.posthog.com/login?x=1"));
        assert_eq!(resolve("http://localhost:3000/login").as_deref(), Some("http://localhost:3000/login"));
        assert_eq!(resolve("localhost:3000").as_deref(), Some("http://localhost:3000"));
        for bad in ["", "   ", "github", "my bank", "javascript:alert(1)", "file:///etc/passwd", "ftp://x.com", "a..b", "-.com/"] {
            assert_eq!(resolve(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_start_page_equipe_made_goes_through_unchanged() {
        let page = "data:text/html,%3Ch1%3Eequipe%3C%2Fh1%3E";
        assert_eq!(resolve(page).as_deref(), Some(page));
        assert_eq!(Flow::new(page, true, [], 0).site, "");
    }

    #[test]
    fn every_common_site_is_an_address_with_a_host() {
        for (name, url) in COMMON_SITES {
            assert!(!name.is_empty() && resolve(url).as_deref() == Some(url), "{name}");
        }
    }

    fn page(id: u64) -> HostPage {
        HostPage { id, url: "about:blank".into(), title: String::new(), loading: false, signals: None, ..Default::default() }
    }

    fn at(id: u64, url: &str, password: bool) -> HostPage {
        HostPage { id, url: url.into(), signals: Some(crate::browser::Signals { password, ..Default::default() }), ..Default::default() }
    }

    #[test]
    fn a_sign_in_in_the_chrome_window_is_seen_as_it_is_in_the_panel() {
        // The two differ only in how the page is shown: both are the person's own tab.
        for window in [false, true] {
            let mut t = Tracker::default();
            t.start(Flow::new("https://github.com/login", window, [1], 1_000));
            assert!(t.see(&[at(1, "https://old.example/", false), at(5, "https://github.com/login", true)], 1_001).is_empty());
            assert!(t.see(&[at(5, "https://github.com/login/otp", false)], 1_030).is_empty());
            let seen = t.see(&[at(5, "https://github.com/", false)], 1_060);
            assert_eq!(seen.len(), 1);
            let Seen::SignedIn { was, flow } = &seen[0] else { panic!("{seen:?}") };
            assert_eq!((was.site.as_str(), flow.clone()), ("github.com", Some(("github.com".to_owned(), window))));
            assert!(t.pending().is_none(), "done: nothing more is followed");
        }
    }

    #[test]
    fn a_sign_in_the_person_does_in_a_tab_nobody_asked_for_is_still_seen_but_leaves_the_flow() {
        let mut t = Tracker::default();
        t.start(Flow::new("https://github.com/login", false, [], 0));
        t.see(&[at(5, "https://github.com/login", true)], 1);
        // Another tab signs in to another site while the flow waits.
        t.see(&[at(5, "https://github.com/login", true), at(6, "https://vercel.com/login", true)], 2);
        let seen = t.see(&[at(5, "https://github.com/login", true), at(6, "https://vercel.com/", false)], 3);
        let Seen::SignedIn { was, flow } = &seen[0] else { panic!() };
        assert_eq!((was.site.as_str(), flow.is_none()), ("vercel.com", true));
        assert!(t.pending().is_some(), "its own tab hasn't finished");
    }

    #[test]
    fn a_flow_closed_or_left_too_long_ends_quietly() {
        let mut t = Tracker::default();
        t.start(Flow::new("https://github.com/login", true, [], 0));
        t.see(&[at(5, "https://github.com/login", true)], 1);
        assert_eq!(t.see(&[], 2), [Seen::Closed { site: "github.com".into(), window: true }]);
        assert!(t.see(&[], 3).is_empty());
        t.start(Flow::new("https://github.com/login", true, [], 10));
        assert!(t.see(&[at(5, "https://github.com/login", true)], 10 + FLOW_FOR + 1).is_empty() && t.pending().is_none());
    }

    #[test]
    fn a_flow_follows_the_tab_it_opened_not_the_ones_there_before() {
        let mut flow = Flow::new("https://github.com/login", false, [1, 2], 100);
        assert_eq!(flow.site, "github.com");
        assert_eq!(flow.claim(&[page(1), page(2)]), None);
        assert_eq!(flow.claim(&[page(1), page(2), page(7)]), Some(7));
        assert_eq!(flow.claim(&[page(1), page(2), page(7), page(8)]), Some(7), "once claimed, it stays");
        assert!(!flow.closed(&[page(7)]) && flow.closed(&[page(1)]));
        assert!(!flow.expired(100 + FLOW_FOR) && flow.expired(101 + FLOW_FOR));
    }
}
