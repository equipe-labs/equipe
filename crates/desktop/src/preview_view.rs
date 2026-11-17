//! The live preview's web view: a plain WebKit view (WKWebView, through
//! `wry`, as the sign-in view) over the preview pane, showing a member's page
//! from the daemon's machine at this screen's own resolution. It is the
//! person's own browser — no automation, nothing streamed: they click, type
//! and scroll in it as in any browser. Its store is private and goes with it.
//!
//! What the page tells the pane comes back as [`WebEvent`]s: a pin dropped
//! (a design session's pin mode), Esc and ⌘⇧F typed while the page has the
//! keys, a page loaded. And [`answers`] says whether a page answers at a
//! port of this Mac (directly, or through a forward to the daemon's).

use std::io::{Read as _, Write as _};
use std::net::{TcpStream, ToSocketAddrs as _};
use std::time::Duration;

/// What the page tells the pane.
#[derive(Debug, Clone, PartialEq)]
pub enum WebEvent {
    /// A pin dropped on the page: CSS pixels from its top left (scrolling
    /// included), the viewport's width, and the element's selector.
    Pin { x: u32, y: u32, width: u32, selector: Option<String> },
    /// Esc typed in the page and not taken by it.
    Escape,
    /// ⌘⇧F typed in the page.
    ToggleFull,
    /// A page finished loading, at this address.
    Loaded(String),
}

/// What the page's script posts, as read back here.
pub fn read_message(body: &str) -> Option<WebEvent> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    match v.get("t")?.as_str()? {
        "pin" => Some(WebEvent::Pin {
            x: v.get("x")?.as_f64()?.max(0.).round() as u32,
            y: v.get("y")?.as_f64()?.max(0.).round() as u32,
            width: v.get("w")?.as_f64()?.max(0.).round() as u32,
            selector: v.get("sel").and_then(|s| s.as_str()).map(|s| s.chars().take(300).collect::<String>()).filter(|s| !s.is_empty()),
        }),
        "esc" => Some(WebEvent::Escape),
        "full" => Some(WebEvent::ToggleFull),
        _ => None,
    }
}

/// Whether the web view may go to an address: web pages (and the inline
/// documents pages make) only — never a local file, nor another scheme a
/// page could open an app with.
pub fn allowed(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    ["http://", "https://", "about:", "data:", "blob:"].iter().any(|scheme| lower.starts_with(scheme))
}

/// The script every page of the preview runs: pins (a click, in pin mode,
/// posts where it landed instead of reaching the page; numbered markers drawn
/// over the page) and the pane's keys while the page has them.
pub const SCRIPT: &str = r#"(() => {
  if (window.__equipe) return;
  const post = (m) => { try { window.ipc.postMessage(JSON.stringify(m)); } catch (_) {} };
  const selector = (el) => {
    const parts = [];
    while (el && el.nodeType === 1 && parts.length < 5) {
      if (el.id) { parts.unshift('#' + (window.CSS && CSS.escape ? CSS.escape(el.id) : el.id)); break; }
      let part = el.tagName.toLowerCase();
      const parent = el.parentElement;
      if (parent) {
        const same = Array.prototype.filter.call(parent.children, (c) => c.tagName === el.tagName);
        if (same.length > 1) part += ':nth-of-type(' + (same.indexOf(el) + 1) + ')';
      }
      parts.unshift(part);
      el = parent;
    }
    return parts.join(' > ');
  };
  let pinning = false;
  const layer = () => {
    let l = document.getElementById('__equipe_pins');
    if (!l) {
      l = document.createElement('div');
      l.id = '__equipe_pins';
      l.style.cssText = 'position:absolute;left:0;top:0;width:0;height:0;z-index:2147483647;pointer-events:none';
      (document.body || document.documentElement).appendChild(l);
    }
    return l;
  };
  window.__equipe = {
    pinning(on) {
      pinning = !!on;
      document.documentElement.style.cursor = pinning ? 'crosshair' : '';
    },
    pins(list) {
      const l = layer();
      l.textContent = '';
      for (const p of list) {
        const d = document.createElement('div');
        d.textContent = String(p.n);
        d.style.cssText = 'position:absolute;left:' + (p.x - 11) + 'px;top:' + (p.y - 11) + 'px;width:22px;height:22px;border-radius:11px;'
          + 'background:' + (p.done ? '#2f9e6b' : '#5b5bd6') + ';color:#fff;font:600 12px/22px -apple-system,system-ui,sans-serif;'
          + 'text-align:center;box-shadow:0 1px 4px rgba(0,0,0,.35)';
        l.appendChild(d);
      }
    },
  };
  document.addEventListener('click', (e) => {
    if (!pinning) return;
    e.preventDefault();
    e.stopPropagation();
    window.__equipe.pinning(false);
    post({ t: 'pin', x: e.clientX + window.scrollX, y: e.clientY + window.scrollY, w: window.innerWidth, sel: selector(e.target) });
  }, true);
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !e.defaultPrevented) {
      if (pinning) { window.__equipe.pinning(false); return; }
      post({ t: 'esc' });
    } else if ((e.key === 'f' || e.key === 'F') && e.metaKey && e.shiftKey) {
      e.preventDefault();
      post({ t: 'full' });
    }
  });
})();"#;

/// The call that puts pins over the page: `(n, x, y, done)` each.
pub fn pins_call(pins: &[(u32, u32, u32, bool)]) -> String {
    let list: Vec<serde_json::Value> = pins.iter().map(|(n, x, y, done)| serde_json::json!({ "n": n, "x": x, "y": y, "done": done })).collect();
    format!("window.__equipe && window.__equipe.pins({})", serde_json::Value::Array(list))
}

/// The call that turns pin mode on or off.
pub fn pinning_call(on: bool) -> String {
    format!("window.__equipe && window.__equipe.pinning({on})")
}

/// Whether a page answers at `port` of this Mac: something listens there
/// and doesn't hang up at once. Through a forward to another Mac a closed
/// port there is a connection here that ends with nothing: not answering.
/// Blocking: run it off the window's thread.
pub fn answers(port: u16, secure: bool) -> bool {
    let Ok(addresses) = ("localhost", port).to_socket_addrs() else { return false };
    for address in addresses {
        let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(800)) else { continue };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(2500)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(800)));
        // A plain page answers a request; an https one waits for its handshake.
        if !secure && stream.write_all(format!("HEAD / HTTP/1.0\r\nHost: localhost:{port}\r\n\r\n").as_bytes()).is_err() {
            return false;
        }
        let mut byte = [0u8; 1];
        return match stream.read(&mut byte) {
            Ok(0) => false,
            Ok(_) => true,
            // Still connected and quiet: something is there.
            Err(e) => matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut),
        };
    }
    false
}

#[cfg(target_os = "macos")]
pub use mac::PreviewWebView;

/// Where there is no web view to embed: none opens, and the pane says so.
#[cfg(not(target_os = "macos"))]
pub struct PreviewWebView;

#[cfg(not(target_os = "macos"))]
impl PreviewWebView {
    pub fn open(_: &gpui_kit::Window, _: &str, _: (f32, f32, f32, f32), _: tokio::sync::mpsc::UnboundedSender<WebEvent>) -> Result<Self, String> {
        Err("no web view on this system".into())
    }
    pub fn set_bounds(&self, _: (f32, f32, f32, f32)) {}
    pub fn set_visible(&mut self, _: bool) {}
    pub fn load(&self, _: &str) {}
    pub fn reload(&self) {}
    pub fn back(&self) {}
    pub fn zoom(&mut self, _: f64) {}
    pub fn set_dark(&mut self, _: Option<bool>) {}
    pub fn run(&self, _: &str) {}
    pub fn focus(&self) {}
}

#[cfg(target_os = "macos")]
mod mac {
    use tokio::sync::mpsc::UnboundedSender;
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::{NewWindowResponse, PageLoadEvent, Rect, WebView, WebViewBuilder};

    use super::{SCRIPT, WebEvent, allowed, read_message};

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { position: LogicalPosition::new(f64::from(x), f64::from(y)).into(), size: LogicalSize::new(f64::from(w.max(1.)), f64::from(h.max(1.))).into() }
    }

    /// An open preview web view.
    pub struct PreviewWebView {
        webview: WebView,
        shown: bool,
        zoom: f64,
        dark: Option<bool>,
    }

    impl PreviewWebView {
        /// Open a view at `url`, over `(x, y, w, h)` of the window's content
        /// in points; what its pages tell goes to `events`.
        pub fn open(window: &gpui_kit::Window, url: &str, at: (f32, f32, f32, f32), events: UnboundedSender<WebEvent>) -> Result<Self, String> {
            let (posted, loaded) = (events.clone(), events);
            let webview = WebViewBuilder::new()
                .with_url(url)
                // A private store: nothing of the preview stays on this Mac once it closes.
                .with_incognito(true)
                .with_bounds(rect(at.0, at.1, at.2, at.3))
                .with_accept_first_mouse(true)
                .with_back_forward_navigation_gestures(true)
                .with_initialization_script(SCRIPT)
                .with_navigation_handler(|next| allowed(&next))
                // A page that opens a window: a web page in the person's browser, never another view.
                .with_new_window_req_handler(|next, _| {
                    if next.starts_with("http://") || next.starts_with("https://") {
                        let _ = equipe_providers::platform::open(&next);
                    }
                    NewWindowResponse::Deny
                })
                .with_ipc_handler(move |request| {
                    if let Some(event) = read_message(request.body()) {
                        let _ = posted.send(event);
                    }
                })
                .with_on_page_load_handler(move |event, url| {
                    if matches!(event, PageLoadEvent::Finished) {
                        let _ = loaded.send(WebEvent::Loaded(url));
                    }
                })
                .build_as_child(window)
                .map_err(|err| err.to_string())?;
            Ok(Self { webview, shown: true, zoom: 1., dark: None })
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

        pub fn load(&self, url: &str) {
            if allowed(url) {
                let _ = self.webview.load_url(url);
            }
        }

        pub fn reload(&self) {
            let _ = self.webview.reload();
        }

        pub fn back(&self) {
            let _ = self.webview.evaluate_script("history.back()");
        }

        /// The page drawn at `zoom` (a wide page fitted into a narrower pane).
        pub fn zoom(&mut self, zoom: f64) {
            if (self.zoom - zoom).abs() > 0.001 {
                self.zoom = zoom;
                let _ = self.webview.zoom(zoom);
            }
        }

        /// The page's own dark or light look (`prefers-color-scheme`), through
        /// the view's appearance; `None`: the Mac's.
        pub fn set_dark(&mut self, dark: Option<bool>) {
            if self.dark == dark {
                return;
            }
            self.dark = dark;
            use objc2::msg_send;
            use objc2::runtime::{AnyClass, AnyObject};
            use objc2_foundation::NSString;
            use wry::WebViewExtMacOS as _;
            let Some(appearance_class) = AnyClass::get(c"NSAppearance") else { return };
            let view = self.webview.webview();
            let view: *mut AnyObject = objc2::rc::Retained::as_ptr(&view) as *mut AnyObject;
            // SAFETY: AppKit's `+[NSAppearance appearanceNamed:]` and
            // `-[NSView setAppearance:]` (nil follows the window), on the main
            // thread, on the web view this preview owns.
            unsafe {
                let look: *mut AnyObject = match dark {
                    None => std::ptr::null_mut(),
                    Some(false) => msg_send![appearance_class, appearanceNamed: &*NSString::from_str("NSAppearanceNameAqua")],
                    Some(true) => msg_send![appearance_class, appearanceNamed: &*NSString::from_str("NSAppearanceNameDarkAqua")],
                };
                let _: () = msg_send![view, setAppearance: look];
            }
        }

        /// Run a call of the pane's in the page ([`super::pins_call`]).
        pub fn run(&self, js: &str) {
            let _ = self.webview.evaluate_script(js);
        }

        pub fn focus(&self) {
            let _ = self.webview.focus();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_posts_pins_and_keys_and_nothing_else() {
        assert_eq!(
            read_message(r##"{"t":"pin","x":120.4,"y":2210.6,"w":1440,"sel":"main > section:nth-of-type(2) > h2"}"##),
            Some(WebEvent::Pin { x: 120, y: 2211, width: 1440, selector: Some("main > section:nth-of-type(2) > h2".into()) })
        );
        assert_eq!(read_message(r#"{"t":"pin","x":-3,"y":5,"w":390,"sel":""}"#), Some(WebEvent::Pin { x: 0, y: 5, width: 390, selector: None }));
        assert_eq!(read_message(r#"{"t":"esc"}"#), Some(WebEvent::Escape));
        assert_eq!(read_message(r#"{"t":"full"}"#), Some(WebEvent::ToggleFull));
        assert_eq!(read_message(r#"{"t":"eval","js":"x"}"#), None);
        assert_eq!(read_message("not json"), None);
    }

    #[test]
    fn the_view_goes_to_web_pages_and_nothing_else() {
        assert!(allowed("http://localhost:4899/") && allowed("https://example.com/") && allowed("about:blank") && allowed("data:text/html,hi") && allowed("blob:http://localhost/x"));
        assert!(!allowed("file:///Users/me/secret") && !allowed("slack://open") && !allowed("javascript:alert(1)") && !allowed("x-apple-systempreferences:"));
    }

    #[test]
    fn pins_and_pin_mode_are_calls_into_the_page() {
        let call = pins_call(&[(1, 10, 20, false), (2, 5, 900, true)]);
        assert!(call.starts_with("window.__equipe && window.__equipe.pins([{"), "{call}");
        assert!(call.contains(r#""n":2"#) && call.contains(r#""y":900"#) && call.contains(r#""done":true"#), "{call}");
        assert_eq!(pinning_call(true), "window.__equipe && window.__equipe.pinning(true)");
    }

    #[test]
    fn a_page_answers_or_it_doesnt() {
        use std::net::TcpListener;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let serve = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 256];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.0 200 OK\r\n\r\n");
        });
        assert!(answers(port, false), "a server there");
        serve.join().unwrap();
        // Nothing listens any more.
        assert!(!answers(port, false), "nothing there");
    }
}
