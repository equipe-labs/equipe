//! The sign-in view: where the person signs in when a member hands them its
//! browser — a plain web view (WKWebView, through `wry`) inside the browser
//! panel of the screen they are looking at, on this Mac or another one from
//! the daemon's.
//!
//! It is nothing but a browser: no automation, no debugging port, the web
//! view's own user agent, a private (non-persistent) cookie store. A site's
//! "verify you are human" check sees a person in WebKit, and passes them as it
//! passes anyone. At Done the cookies the sign-in made are read out of its
//! store — only those of the hosts the view visited — and sent to the daemon
//! (`Command::BrowserCookies`) over the connection the screen already has.
//! Dropping the view discards the store: nothing of it stays on this Mac.
//!
//! See `equipe_core::handoff_view` for what the daemon does with them.

#[cfg(target_os = "macos")]
pub use mac::SignInView;

/// Where there is no web view to embed: a view that never opens, so a hand-off
/// goes to the daemon's own Chrome window as it did.
#[cfg(not(target_os = "macos"))]
pub struct SignInView;

#[cfg(not(target_os = "macos"))]
impl SignInView {
    pub fn open(_: &gpui_kit::Window, _: &str, _: (f32, f32, f32, f32)) -> Result<Self, String> {
        Err("no web view on this system".into())
    }

    pub fn set_bounds(&self, _: (f32, f32, f32, f32)) {}

    pub fn set_visible(&mut self, _: bool) {}

    pub fn finish(self) -> (Vec<String>, Vec<equipe_core::protocol::HandoffCookie>) {
        (Vec::new(), Vec::new())
    }
}

/// Hosts only: the address a navigation goes to, as a lowercase host name.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h).split(':').next()?.trim_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// Whether the view may go to an address: web pages only — nothing local,
/// nothing of another scheme a page could open an app with.
pub fn allowed(url: &str) -> bool {
    url == "about:blank" || host_of(url).is_some()
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::collections::BTreeSet;
    use std::rc::Rc;

    use equipe_core::handoff_view::scope_cookies;
    use equipe_core::protocol::HandoffCookie;
    use gpui_kit::Window;
    use wry::cookie::{Cookie, SameSite};
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::{Rect, WebView, WebViewBuilder};

    use super::{allowed, host_of};

    /// One cookie as the daemon takes it.
    pub(super) fn convert(c: &Cookie<'_>) -> HandoffCookie {
        HandoffCookie {
            name: c.name().to_owned(),
            value: c.value().to_owned(),
            // The cookie crate drops the leading dot WebKit's domain cookies carry, so a
            // host-only cookie can't be told apart: all are taken as the site's own (a
            // dot, as a sign-in's cookies are), still only of hosts the view visited.
            domain: match c.domain().unwrap_or_default() {
                d if d.is_empty() || d.parse::<std::net::IpAddr>().is_ok() => d.to_owned(),
                d => format!(".{d}"),
            },
            path: c.path().unwrap_or("/").to_owned(),
            expires: c.expires_datetime().map(|t| t.unix_timestamp()),
            secure: c.secure().unwrap_or(false),
            http_only: c.http_only().unwrap_or(false),
            same_site: c.same_site().map(|s| match s {
                SameSite::Strict => "Strict",
                SameSite::Lax => "Lax",
                SameSite::None => "None",
            }
            .to_owned()),
        }
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { position: LogicalPosition::new(f64::from(x), f64::from(y)).into(), size: LogicalSize::new(f64::from(w.max(1.)), f64::from(h.max(1.))).into() }
    }

    /// An open sign-in view.
    pub struct SignInView {
        webview: WebView,
        visited: Rc<RefCell<BTreeSet<String>>>,
        shown: bool,
    }

    impl SignInView {
        /// Open a view at `url`, over `(x, y, w, h)` of the window's content in points.
        pub fn open(window: &Window, url: &str, at: (f32, f32, f32, f32)) -> Result<Self, String> {
            let visited = Rc::new(RefCell::new(BTreeSet::new()));
            visited.borrow_mut().extend(host_of(url));
            let seen = visited.clone();
            let webview = WebViewBuilder::new()
                .with_url(url)
                // A private store: nothing of the sign-in stays on this Mac once the view is dropped.
                .with_incognito(true)
                .with_bounds(rect(at.0, at.1, at.2, at.3))
                .with_navigation_handler(move |next| {
                    if !allowed(&next) {
                        return false;
                    }
                    seen.borrow_mut().extend(host_of(&next));
                    true
                })
                .build_as_child(window)
                .map_err(|err| err.to_string())?;
            Ok(Self { webview, visited, shown: true })
        }

        pub fn set_bounds(&self, at: (f32, f32, f32, f32)) {
            let _ = self.webview.set_bounds(rect(at.0, at.1, at.2, at.3));
        }

        pub fn set_visible(&mut self, shown: bool) {
            if self.shown != shown {
                self.shown = shown;
                let _ = self.webview.set_visible(shown);
            }
        }

        /// What the sign-in made, at Done: the hosts the view visited and the
        /// cookies of those hosts alone. The view is dropped after.
        pub fn finish(self) -> (Vec<String>, Vec<HandoffCookie>) {
            let hosts: Vec<String> = self.visited.borrow().iter().cloned().collect();
            let all: Vec<HandoffCookie> = self.webview.cookies().map(|c| c.iter().map(convert).collect()).unwrap_or_default();
            (hosts.clone(), scope_cookies(all, "", &hosts).cookies)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_webkit_cookie_becomes_the_cookie_the_daemon_takes() {
            let mut c = Cookie::new("session", "v");
            c.set_domain(".chatgpt.com");
            c.set_path("/");
            c.set_secure(true);
            c.set_http_only(true);
            c.set_same_site(SameSite::Lax);
            let got = convert(&c);
            assert_eq!((got.name.as_str(), got.domain.as_str(), got.path.as_str()), ("session", ".chatgpt.com", "/"));
            assert!(got.secure && got.http_only && got.expires.is_none());
            assert_eq!(got.same_site.as_deref(), Some("Lax"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_read_from_web_addresses_only() {
        assert_eq!(host_of("https://Auth.OpenAI.com/log-in?x=1").as_deref(), Some("auth.openai.com"));
        assert_eq!(host_of("http://user:pw@example.com:8080/a").as_deref(), Some("example.com"));
        assert_eq!(host_of("file:///etc/passwd"), None);
        assert_eq!(host_of("javascript:alert(1)"), None);
        assert_eq!(host_of("https:///nothing"), None);
    }

    #[test]
    fn the_view_goes_to_web_pages_and_nothing_else() {
        assert!(allowed("https://chatgpt.com/") && allowed("http://localhost:3000/") && allowed("about:blank"));
        assert!(!allowed("file:///Users/me/secret") && !allowed("slack://open") && !allowed("data:text/html,hi") && !allowed("javascript:1"));
    }
}
