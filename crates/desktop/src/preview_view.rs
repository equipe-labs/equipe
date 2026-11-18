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
    /// included), the viewport's width, and the element there.
    Pin { x: u32, y: u32, width: u32, selector: Option<String>, element: Option<Element> },
    /// The crop of the page around a pin just dropped, as PNG bytes: a
    /// snapshot of the web view's own drawing (never the screen).
    Crop(Vec<u8>),
    /// Esc typed in the page and not taken by it.
    Escape,
    /// ⌘⇧F typed in the page.
    ToggleFull,
    /// A page finished loading, at this address.
    Loaded(String),
}

/// The element under a pin: its words, its box in the page (CSS pixels,
/// scrolling included), and where it and the click sit in the visible part
/// of the page — what the crop is cut from.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub text: Option<String>,
    /// x, y, width, height in the page.
    pub bounds: [u32; 4],
    /// The element's box in the viewport, and the click's spot there (CSS pixels).
    pub in_view: (f64, f64, f64, f64),
    pub click: (f64, f64),
    /// The viewport's size (CSS pixels).
    pub viewport: (f64, f64),
}

/// What the page's script posts, as read back here.
pub fn read_message(body: &str) -> Option<WebEvent> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    match v.get("t")?.as_str()? {
        "pin" => {
            let num = |k: &str| v.get(k).and_then(serde_json::Value::as_f64).filter(|n| n.is_finite());
            let element = (|| {
                let (bx, by, bw, bh) = (num("bx")?, num("by")?, num("bw")?, num("bh")?);
                let px = |n: f64| n.max(0.).round() as u32;
                Some(Element {
                    text: v.get("txt").and_then(|s| s.as_str()).map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect::<String>()).filter(|s| !s.is_empty()),
                    bounds: [px(bx), px(by), px(bw), px(bh)],
                    in_view: (num("vx")?, num("vy")?, bw, bh),
                    click: (num("cx")?, num("cy")?),
                    viewport: (num("vw")?, num("vh")?),
                })
            })();
            Some(WebEvent::Pin {
                x: v.get("x")?.as_f64()?.max(0.).round() as u32,
                y: v.get("y")?.as_f64()?.max(0.).round() as u32,
                width: v.get("w")?.as_f64()?.max(0.).round() as u32,
                selector: v.get("sel").and_then(|s| s.as_str()).map(|s| s.chars().take(300).collect::<String>()).filter(|s| !s.is_empty()),
                element,
            })
        }
        "esc" => Some(WebEvent::Escape),
        "full" => Some(WebEvent::ToggleFull),
        _ => None,
    }
}

/// The part of the visible page a pin's crop shows, in CSS pixels of the
/// viewport: the element with some room around it — or, for one too big to
/// say much (a whole section), a box around the click — kept inside the
/// viewport. `None` when nothing of it is visible.
pub fn crop_rect(element: &Element) -> Option<(f64, f64, f64, f64)> {
    const PAD: f64 = 16.;
    const MOST: (f64, f64) = (640., 400.);
    const AROUND: (f64, f64) = (480., 300.);
    let (vw, vh) = element.viewport;
    let (x, y, w, h) = element.in_view;
    let (x, y, w, h) = if w + 2. * PAD <= MOST.0 && h + 2. * PAD <= MOST.1 {
        (x - PAD, y - PAD, w + 2. * PAD, h + 2. * PAD)
    } else {
        (element.click.0 - AROUND.0 / 2., element.click.1 - AROUND.1 / 2., AROUND.0, AROUND.1)
    };
    let (left, top) = (x.max(0.), y.max(0.));
    let (right, bottom) = ((x + w).min(vw), (y + h).min(vh));
    (right - left >= 4. && bottom - top >= 4.).then_some((left, top, right - left, bottom - top))
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
    const el = e.target;
    const r = el.getBoundingClientRect ? el.getBoundingClientRect() : { left: e.clientX, top: e.clientY, width: 0, height: 0 };
    const words = (el.innerText || el.getAttribute && (el.getAttribute('aria-label') || el.getAttribute('alt') || el.getAttribute('placeholder')) || '').slice(0, 400);
    post({ t: 'pin', x: e.clientX + window.scrollX, y: e.clientY + window.scrollY, w: window.innerWidth, sel: selector(el),
      txt: words, bx: r.left + window.scrollX, by: r.top + window.scrollY, bw: r.width, bh: r.height, vx: r.left, vy: r.top,
      cx: e.clientX, cy: e.clientY, vw: window.innerWidth, vh: window.innerHeight });
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
    pub fn snapshot(&self, _: (f64, f64, f64, f64), _: tokio::sync::mpsc::UnboundedSender<WebEvent>) {}
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

        /// A snapshot of `rect` (CSS pixels of the visible page) as the web
        /// view draws it — WebKit's own rendering of this view, offscreen,
        /// never a capture of the screen — sent as [`WebEvent::Crop`].
        pub fn snapshot(&self, rect: (f64, f64, f64, f64), to: UnboundedSender<WebEvent>) {
            use objc2::runtime::{AnyClass, AnyObject};
            use objc2::{Encode, Encoding, msg_send};
            use wry::WebViewExtMacOS as _;

            #[repr(C)]
            #[derive(Clone, Copy)]
            struct Rect {
                x: f64,
                y: f64,
                w: f64,
                h: f64,
            }
            // SAFETY: laid out as CGRect (two CGFloat pairs), which is what the encoding says.
            unsafe impl Encode for Rect {
                const ENCODING: Encoding = Encoding::Struct("CGRect", &[Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]), Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING])]);
            }

            let Some(config_class) = AnyClass::get(c"WKSnapshotConfiguration") else { return };
            let Some(rep_class) = AnyClass::get(c"NSBitmapImageRep") else { return };
            let Some(dict_class) = AnyClass::get(c"NSDictionary") else { return };
            let zoom = self.zoom;
            let at = Rect { x: rect.0 * zoom, y: rect.1 * zoom, w: rect.2 * zoom, h: rect.3 * zoom };
            let view = self.webview.webview();
            let view: *mut AnyObject = objc2::rc::Retained::as_ptr(&view) as *mut AnyObject;
            let rep_class: &'static AnyClass = rep_class;
            let dict_class: &'static AnyClass = dict_class;
            let done = block2::RcBlock::new(move |image: *mut AnyObject, _error: *mut AnyObject| {
                if image.is_null() {
                    return;
                }
                // SAFETY: AppKit calls on the NSImage WebKit handed over, on
                // the main thread: its TIFF data, a bitmap of it, that as PNG.
                let png = unsafe {
                    let tiff: *mut AnyObject = msg_send![image, TIFFRepresentation];
                    if tiff.is_null() {
                        return;
                    }
                    let rep: *mut AnyObject = msg_send![rep_class, imageRepWithData: tiff];
                    if rep.is_null() {
                        return;
                    }
                    let props: *mut AnyObject = msg_send![dict_class, dictionary];
                    // NSBitmapImageFileTypePNG = 4.
                    let data: *mut AnyObject = msg_send![rep, representationUsingType: 4usize, properties: props];
                    if data.is_null() {
                        return;
                    }
                    let len: usize = msg_send![data, length];
                    let bytes: *const u8 = msg_send![data, bytes];
                    if bytes.is_null() || len == 0 {
                        return;
                    }
                    std::slice::from_raw_parts(bytes, len).to_vec()
                };
                let _ = to.send(WebEvent::Crop(png));
            });
            // SAFETY: `+new` on WebKit's snapshot configuration, its rect set,
            // and `-takeSnapshotWithConfiguration:completionHandler:` on the
            // web view this preview owns, on the main thread; the block lives
            // as long as WebKit holds it.
            unsafe {
                let config: Option<objc2::rc::Retained<AnyObject>> = msg_send![config_class, new];
                let Some(config) = config else { return };
                let _: () = msg_send![&*config, setRect: at];
                let _: () = msg_send![view, takeSnapshotWithConfiguration: &*config, completionHandler: &*done];
            }
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
            Some(WebEvent::Pin { x: 120, y: 2211, width: 1440, selector: Some("main > section:nth-of-type(2) > h2".into()), element: None })
        );
        let full = r#"{"t":"pin","x":130,"y":1210,"w":1440,"sel":"main > button","txt":"  Start\n  free trial ","bx":100,"by":1190,"bw":160,"bh":40,"vx":100,"vy":190,"cx":130,"cy":210,"vw":1440,"vh":900}"#;
        let Some(WebEvent::Pin { element: Some(el), .. }) = read_message(full) else { panic!() };
        assert_eq!(el.text.as_deref(), Some("Start free trial"));
        assert_eq!(el.bounds, [100, 1190, 160, 40]);
        assert_eq!((el.in_view, el.click, el.viewport), ((100., 190., 160., 40.), (130., 210.), (1440., 900.)));
        assert_eq!(read_message(r#"{"t":"pin","x":-3,"y":5,"w":390,"sel":""}"#), Some(WebEvent::Pin { x: 0, y: 5, width: 390, selector: None, element: None }));
        assert_eq!(read_message(r#"{"t":"esc"}"#), Some(WebEvent::Escape));
        assert_eq!(read_message(r#"{"t":"full"}"#), Some(WebEvent::ToggleFull));
        assert_eq!(read_message(r#"{"t":"eval","js":"x"}"#), None);
        assert_eq!(read_message("not json"), None);
    }

    #[test]
    fn a_pins_crop_is_its_element_with_room_or_a_box_around_the_click_inside_the_view() {
        let el = |in_view, click| Element { text: None, bounds: [0, 0, 0, 0], in_view, click, viewport: (1440., 900.) };
        assert_eq!(crop_rect(&el((100., 190., 160., 40.), (130., 210.))), Some((84., 174., 192., 72.)), "a button, with room around");
        assert_eq!(crop_rect(&el((0., 0., 1440., 2000.), (700., 450.))), Some((460., 300., 480., 300.)), "a whole section: around the click");
        assert_eq!(crop_rect(&el((1420., 880., 40., 40.), (1430., 890.))), Some((1404., 864., 36., 36.)), "kept inside the view");
        assert_eq!(crop_rect(&el((2000., 2000., 10., 10.), (5., 5.))), None, "nothing of it visible");
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
