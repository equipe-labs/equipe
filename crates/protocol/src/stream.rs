//! The browser stream: a second connection a screen opens for the live
//! browser panel, beside the JSON-RPC one that carries commands and events.
//!
//! It starts as an ordinary JSON-RPC line — a `browser_stream` request
//! ([`crate::rpc::BROWSER_STREAM`], params [`StreamHello`]) answered with
//! `true` — and then both directions switch to binary messages:
//!
//! `u32be length | u8 kind | payload` (the length counts the kind byte)
//!
//! - kind [`JSON`]: a JSON value — screen → daemon a [`StreamIn`], daemon →
//!   screen a [`StreamOut`];
//! - kind [`FRAME`] (daemon → screen): a picture of a watched page,
//!   `u16be header length | header JSON` ([`FrameHeader`]) `| JPEG`.
//!
//! Frames are paced by credit: the daemon sends one only while the screen has
//! credit, and the screen gives one back ([`StreamIn::Credit`]) once it has
//! drawn a frame. A slow screen (a far machine) gets fewer frames, always the
//! newest — never a backlog. Pictures never travel on the event connection.

use serde::{Deserialize, Serialize};

/// A JSON message.
pub const JSON: u8 = 1;
/// A frame: `u16be header length | header JSON | JPEG`.
pub const FRAME: u8 = 2;
/// The largest message either side accepts.
pub const MAX_MESSAGE: usize = 32 << 20;
/// The credit a screen starts with: frames under way at once.
pub const INITIAL_CREDIT: u32 = 2;

/// The `browser_stream` request's params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamHello {
    /// JPEG quality the screen asks for, 1–100: lower over a network.
    #[serde(default)]
    pub quality: Option<u8>,
}

/// What a frame is of: the page, its place in the page's frames, the
/// picture's size, and the page's viewport in CSS pixels — what a click on the
/// picture is mapped to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameHeader {
    pub page: u64,
    pub seq: u64,
    /// The JPEG's size, in pixels.
    pub w: u32,
    pub h: u32,
    /// The page's viewport, in CSS pixels.
    #[serde(rename = "cssW")]
    pub css_w: f64,
    #[serde(rename = "cssH")]
    pub css_h: f64,
    /// Where the page is scrolled to, in CSS pixels.
    #[serde(default)]
    pub sx: f64,
    #[serde(default)]
    pub sy: f64,
    /// Top of the page in the viewport (an emulated device's chrome), CSS pixels.
    #[serde(default)]
    pub top: f64,
}

/// Screen → daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamIn {
    /// Show `page`, about `fps` times a second at most, no bigger than
    /// `max_w`×`max_h` pixels. Replaces whatever was watched before. `fit`:
    /// the picture's box, for a page the person drives (their own tab, or a
    /// member's in their hands) to be laid out at — so it fills the box.
    Watch {
        page: u64,
        fps: u32,
        max_w: u32,
        max_h: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fit: Option<Fit>,
    },
    /// Nothing to show (the panel closed or out of sight).
    Unwatch,
    /// `n` frames drawn: that many more may come.
    Credit { n: u32 },
    /// The person's hand on `page`.
    Input { page: u64, ev: InputEvent },
}

/// The box a page is laid out at: CSS pixels, and device pixels per CSS
/// pixel (the screen's scale, for crisp text).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Fit {
    pub w: u32,
    pub h: u32,
    pub dpr: f32,
}

/// Daemon → screen, besides frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamOut {
    /// There is no browser to watch (members use a Chrome each, or none).
    NoBrowser,
}

/// Modifier bits, as Chrome's DevTools protocol counts them.
pub mod modifiers {
    pub const ALT: u32 = 1;
    pub const CTRL: u32 = 2;
    pub const META: u32 = 4;
    pub const SHIFT: u32 = 8;
}

/// One piece of the person's input, as Chrome's `Input` domain takes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum InputEvent {
    /// `kind`: `mousePressed`, `mouseReleased` or `mouseMoved`; `x`, `y` in the
    /// page's CSS pixels; `button`: `left`, `middle`, `right` or `none`.
    Mouse {
        #[serde(rename = "type")]
        kind: String,
        x: f64,
        y: f64,
        button: String,
        buttons: u32,
        #[serde(rename = "clickCount")]
        click_count: u32,
        modifiers: u32,
    },
    Wheel { x: f64, y: f64, dx: f64, dy: f64, modifiers: u32 },
    /// `kind`: `keyDown` (with `text`), `rawKeyDown` (without), `keyUp`.
    /// `commands`: the editing commands a Mac binds to the key (`selectAll`).
    Key {
        #[serde(rename = "type")]
        kind: String,
        key: String,
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        vk: u32,
        modifiers: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        commands: Vec<String>,
        #[serde(default)]
        repeat: bool,
    },
    /// Text being composed (Korean, Japanese…), not yet committed.
    Ime { text: String },
    /// Text committed: typed, composed, or pasted.
    Insert { text: String },
}

/// One binary message: `u32be length | kind | payload`.
pub fn encode(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 5);
    out.extend_from_slice(&(payload.len() as u32 + 1).to_be_bytes());
    out.push(kind);
    out.extend_from_slice(payload);
    out
}

/// A JSON message.
pub fn encode_json(value: &impl Serialize) -> Vec<u8> {
    encode(JSON, &serde_json::to_vec(value).expect("stream messages serialize"))
}

/// A frame message's payload: its header, then the JPEG.
pub fn frame_payload(header: &FrameHeader, jpeg: &[u8]) -> Vec<u8> {
    let head = serde_json::to_vec(header).expect("frame headers serialize");
    let mut out = Vec::with_capacity(2 + head.len() + jpeg.len());
    out.extend_from_slice(&(head.len() as u16).to_be_bytes());
    out.extend_from_slice(&head);
    out.extend_from_slice(jpeg);
    out
}

/// A frame message's header and JPEG.
pub fn parse_frame(payload: &[u8]) -> Option<(FrameHeader, &[u8])> {
    let len = u16::from_be_bytes(payload.get(..2)?.try_into().ok()?) as usize;
    let header = serde_json::from_slice(payload.get(2..2 + len)?).ok()?;
    Some((header, &payload[2 + len..]))
}

/// Binary messages cut out of a byte stream as it arrives.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

/// A message too big to be one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TooBig(pub usize);

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole message — its kind and payload — if one has arrived.
    pub fn message(&mut self) -> Result<Option<(u8, Vec<u8>)>, TooBig> {
        let Some(head) = self.buf.get(..4) else {
            return Ok(None);
        };
        let len = u32::from_be_bytes(head.try_into().expect("four bytes")) as usize;
        if len == 0 || len > MAX_MESSAGE {
            return Err(TooBig(len));
        }
        if self.buf.len() < 4 + len {
            return Ok(None);
        }
        let kind = self.buf[4];
        let payload = self.buf[5..4 + len].to_vec();
        self.buf.drain(..4 + len);
        Ok(Some((kind, payload)))
    }
}

/// Where a point of the picture on screen falls on the page: `at` and the
/// picture's rectangle (`origin`, `size`) in the same units, the page's
/// viewport in CSS pixels. `None` outside the picture.
pub fn to_page(at: (f64, f64), origin: (f64, f64), size: (f64, f64), header: &FrameHeader) -> Option<(f64, f64)> {
    if size.0 <= 0. || size.1 <= 0. {
        return None;
    }
    let (fx, fy) = ((at.0 - origin.0) / size.0, (at.1 - origin.1) / size.1);
    if !(0.0..=1.0).contains(&fx) || !(0.0..=1.0).contains(&fy) {
        return None;
    }
    Some((fx * header.css_w, fy * header.css_h + header.top))
}

/// The rectangle a `w`×`h` picture takes in a `bounds` box, its aspect kept
/// and centred: `(origin, size)`.
pub fn fit(bounds_origin: (f64, f64), bounds_size: (f64, f64), w: f64, h: f64) -> ((f64, f64), (f64, f64)) {
    if w <= 0. || h <= 0. || bounds_size.0 <= 0. || bounds_size.1 <= 0. {
        return (bounds_origin, (0., 0.));
    }
    let scale = (bounds_size.0 / w).min(bounds_size.1 / h);
    let size = (w * scale, h * scale);
    let origin = (bounds_origin.0 + (bounds_size.0 - size.0) / 2., bounds_origin.1 + (bounds_size.1 - size.1) / 2.);
    (origin, size)
}

/// Frames for screens, latest first: what the daemon keeps per watched page
/// and per screen — the newest frame, which one the screen has, and its
/// credit. A screen with no credit is sent nothing; given credit, it gets the
/// newest frame it hasn't seen, never the ones it missed.
#[derive(Debug, Clone, Default)]
pub struct Pacer {
    credit: u32,
    /// The seq of the last frame sent, of the page watched.
    sent: Option<(u64, u64)>,
    /// When the last frame went, and how far apart they may go.
    last: Option<std::time::Instant>,
    gap: std::time::Duration,
}

impl Pacer {
    pub fn new(fps: u32) -> Self {
        Self { credit: INITIAL_CREDIT, sent: None, last: None, gap: gap(fps) }
    }

    /// A new page or rate: frames of the page start over, credit stays.
    pub fn watch(&mut self, fps: u32) {
        self.gap = gap(fps);
        self.sent = None;
    }

    pub fn credit(&mut self, n: u32) {
        self.credit = (self.credit + n).min(INITIAL_CREDIT * 4);
    }

    /// Whether frame `seq` of `page` should go now; if so, it counts as sent.
    /// `Err(wait)`: not yet, but it may go after `wait`.
    pub fn take(&mut self, page: u64, seq: u64, now: std::time::Instant) -> Result<bool, std::time::Duration> {
        if self.credit == 0 || self.sent.is_some_and(|(p, s)| p == page && s >= seq) {
            return Ok(false);
        }
        if let Some(last) = self.last {
            let since = now.saturating_duration_since(last);
            if since < self.gap {
                return Err(self.gap - since);
            }
        }
        self.credit -= 1;
        self.sent = Some((page, seq));
        self.last = Some(now);
        Ok(true)
    }
}

fn gap(fps: u32) -> std::time::Duration {
    std::time::Duration::from_millis(1000 / u64::from(fps.max(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn header(page: u64, seq: u64) -> FrameHeader {
        FrameHeader { page, seq, w: 640, h: 400, css_w: 1280., css_h: 800., sx: 0., sy: 0., top: 0. }
    }

    #[test]
    fn messages_survive_any_cut_of_the_stream() {
        let mut wire = encode_json(&StreamIn::Credit { n: 1 });
        wire.extend(encode(FRAME, &frame_payload(&header(3, 9), &[0xff, 0xd8, 1, 2, 3])));
        for cut in 0..wire.len() {
            let mut d = Decoder::default();
            d.push(&wire[..cut]);
            let mut got = Vec::new();
            while let Some(m) = d.message().unwrap() {
                got.push(m);
            }
            d.push(&wire[cut..]);
            while let Some(m) = d.message().unwrap() {
                got.push(m);
            }
            assert_eq!(got.len(), 2, "cut at {cut}");
            assert_eq!(serde_json::from_slice::<StreamIn>(&got[0].1).unwrap(), StreamIn::Credit { n: 1 });
            assert_eq!(got[1].0, FRAME);
            let (h, jpeg) = parse_frame(&got[1].1).unwrap();
            assert_eq!(h, header(3, 9));
            assert_eq!(jpeg, &[0xff, 0xd8, 1, 2, 3]);
        }
    }

    #[test]
    fn a_runaway_length_is_refused() {
        let mut d = Decoder::default();
        d.push(&(MAX_MESSAGE as u32 + 1).to_be_bytes());
        assert!(d.message().is_err());
        let mut d = Decoder::default();
        d.push(&[0, 0, 0, 0]);
        assert!(d.message().is_err());
    }

    #[test]
    fn the_hosts_frame_header_reads() {
        let raw = br#"{"page":4,"seq":12,"w":751,"h":800,"cssW":1200,"cssH":1279,"sx":0,"sy":340.5,"top":0,"ts":1792055661.08}"#;
        let h: FrameHeader = serde_json::from_slice(raw).unwrap();
        assert_eq!((h.page, h.seq, h.w, h.css_w, h.sy), (4, 12, 751, 1200., 340.5));
    }

    #[test]
    fn input_goes_as_chromes_input_domain_takes_it() {
        let click = InputEvent::Mouse { kind: "mousePressed".into(), x: 1., y: 2., button: "left".into(), buttons: 1, click_count: 2, modifiers: 0 };
        let v = serde_json::to_value(&click).unwrap();
        assert_eq!(v["t"], "mouse");
        assert_eq!(v["type"], "mousePressed");
        assert_eq!(v["clickCount"], 2);
        let key = InputEvent::Key {
            kind: "rawKeyDown".into(),
            key: "a".into(),
            code: "KeyA".into(),
            text: None,
            vk: 65,
            modifiers: modifiers::META,
            commands: vec!["selectAll".into()],
            repeat: false,
        };
        let v = serde_json::to_value(&key).unwrap();
        assert_eq!((v["t"].as_str(), v["type"].as_str(), v["vk"].as_u64()), (Some("key"), Some("rawKeyDown"), Some(65)));
        assert!(v.get("text").is_none());
        assert_eq!(v["commands"][0], "selectAll");
        let watch = serde_json::to_value(StreamIn::Watch { page: 3, fps: 30, max_w: 1200, max_h: 900, fit: None }).unwrap();
        assert_eq!(watch["type"], "watch");
        assert!(watch.get("fit").is_none(), "an older daemon reads it as before");
        let fitted = StreamIn::Watch { page: 3, fps: 30, max_w: 1200, max_h: 900, fit: Some(Fit { w: 600, h: 450, dpr: 2. }) };
        let back: StreamIn = serde_json::from_value(serde_json::to_value(&fitted).unwrap()).unwrap();
        assert_eq!(back, fitted);
    }

    #[test]
    fn a_click_maps_to_the_page_through_the_picture() {
        let h = header(1, 1);
        // A 1280×800 page drawn at 640×400 from (100, 50).
        assert_eq!(to_page((100., 50.), (100., 50.), (640., 400.), &h), Some((0., 0.)));
        assert_eq!(to_page((420., 250.), (100., 50.), (640., 400.), &h), Some((640., 400.)));
        assert_eq!(to_page((740., 450.), (100., 50.), (640., 400.), &h), Some((1280., 800.)));
        assert_eq!(to_page((99., 50.), (100., 50.), (640., 400.), &h), None, "outside the picture");
        let mobile = FrameHeader { top: 56., ..h };
        assert_eq!(to_page((100., 50.), (100., 50.), (640., 400.), &mobile), Some((0., 56.)));
    }

    #[test]
    fn a_picture_fits_its_box_keeping_its_shape() {
        // A wide box: the picture is as tall as the box, centred across.
        assert_eq!(fit((0., 0.), (1000., 400.), 1280., 800.), ((180., 0.), (640., 400.)));
        // A tall box: as wide, centred down.
        assert_eq!(fit((10., 20.), (640., 1000.), 1280., 800.), ((10., 320.), (640., 400.)));
        assert_eq!(fit((0., 0.), (100., 100.), 0., 0.), ((0., 0.), (0., 0.)));
    }

    #[test]
    fn frames_go_by_credit_newest_first_and_no_faster_than_asked() {
        let t0 = Instant::now();
        let mut p = Pacer::new(10);
        assert_eq!(p.take(1, 1, t0), Ok(true));
        assert_eq!(p.take(1, 1, t0 + Duration::from_millis(500)), Ok(false), "the same frame twice");
        assert!(matches!(p.take(1, 2, t0 + Duration::from_millis(40)), Err(wait) if wait == Duration::from_millis(60)), "too soon at 10 fps");
        assert_eq!(p.take(1, 5, t0 + Duration::from_millis(100)), Ok(true), "the newest, skipping 2–4");
        assert_eq!(p.take(1, 6, t0 + Duration::from_millis(300)), Ok(false), "out of credit");
        p.credit(1);
        assert_eq!(p.take(1, 6, t0 + Duration::from_millis(300)), Ok(true));
        assert_eq!(p.take(1, 4, t0 + Duration::from_millis(900)), Ok(false), "older than what it has");
        p.credit(1);
        p.watch(30);
        assert_eq!(p.take(2, 1, t0 + Duration::from_millis(900)), Ok(true), "another page starts over");
    }
}
