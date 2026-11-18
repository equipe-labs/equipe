//! A design session as the screen keeps it (`views/design_session.rs` draws
//! it): the designer's conversation on one page, the page live beside it, the
//! pins the person drops on it, quick reactions, the model rounds run on, and
//! the tokens the session used. The page comes from what the designer shows
//! (`equipe design session show`, read from its conversation —
//! `equipe_core::design::session::markers`), and is drawn through the
//! daemon's shared browser: a page on the daemon's machine (`localhost`, a
//! file in the worktree) shows the same on this Mac or another. No UI types
//! here, so the rules are tested without a window.

use equipe_core::design::session::{Marker, Pin, markers};
use equipe_core::protocol::design::{PickReason, QuickReaction};
use equipe_core::protocol::design_session::SessionFrom;
use equipe_core::protocol::stream::{Fit, FrameHeader};
use equipe_core::protocol::{BrowserPage, Engine, Entry, SessionId};

/// The two widths the page is drawn at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Viewport {
    #[default]
    Desktop,
    Phone,
}

impl Viewport {
    /// The page's box in CSS pixels, and its device scale.
    pub fn fit(self) -> Fit {
        match self {
            Viewport::Desktop => Fit { w: 1440, h: 900, dpr: 1. },
            Viewport::Phone => Fit { w: 390, h: 844, dpr: 2. },
        }
    }

    pub fn width(self) -> u32 {
        self.fit().w
    }
}

/// What is live beside the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Live {
    /// A page the shared browser opens.
    Page(String),
    /// A static render, read as any picture is.
    Render(String),
}

/// What the designer's conversation says since the session started: the
/// latest thing shown, how many times something was shown (each one a
/// change: the page reloads), and the pins it marked done.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shown {
    pub live: Option<Live>,
    pub count: usize,
    pub done: Vec<(u32, String)>,
    /// The prototype's variants shown, by letter (`A`…), each at the page it
    /// was last shown at, in letter order: put side by side.
    pub variants: Vec<(String, String)>,
}

/// [`Shown`], from `entries` past `since`.
pub fn shown(entries: &[Entry], since: usize) -> Shown {
    let mut out = Shown::default();
    for entry in entries.iter().skip(since) {
        let Entry::Tool { outcome: Some(outcome), .. } = entry else { continue };
        if !outcome.ok {
            continue;
        }
        for marker in markers(&outcome.output) {
            match marker {
                Marker::Show(url) => {
                    out.live = Some(Live::Page(url));
                    out.count += 1;
                }
                Marker::Render(path) => {
                    out.live = Some(Live::Render(path));
                    out.count += 1;
                }
                Marker::Variant { label, url } => {
                    out.variants.retain(|(l, _)| *l != label);
                    out.variants.push((label, url.clone()));
                    out.variants.sort();
                    out.live = Some(Live::Page(url));
                    out.count += 1;
                }
                Marker::Done { n, what } => {
                    out.done.retain(|(m, _)| *m != n);
                    out.done.push((n, what));
                }
            }
        }
    }
    out
}

/// The variant whose page is live, when it is one of them.
pub fn variant_in_front(variants: &[(String, String)], live: Option<&Live>) -> Option<String> {
    let Some(Live::Page(url)) = live else { return None };
    variants.iter().find(|(_, u)| same_page(u, url)).map(|(l, _)| l.clone())
}

/// What "Use this" picks: the variant picked (else the one live) and its
/// page, or — no variants — the page live. `None`: nothing live to pick, or
/// variants side by side with none picked yet.
pub fn picked(shown: &Shown, variant: Option<&str>, side_by_side: bool) -> Option<(Option<String>, String)> {
    let Some(Live::Page(live)) = &shown.live else { return None };
    if let Some(v) = variant
        && let Some((l, url)) = shown.variants.iter().find(|(l, _)| l == v)
    {
        return Some((Some(l.clone()), url.clone()));
    }
    if side_by_side {
        return None;
    }
    match variant_in_front(&shown.variants, shown.live.as_ref()) {
        Some(l) => Some((Some(l), live.clone())),
        None => Some((None, live.clone())),
    }
}

/// The page that puts the prototype's variants side by side in the live
/// preview, as a `data:` address: each variant in a frame at its address on
/// this screen (`here`: `None` when it can't be reached from here, and it is
/// left out), laid out `width` CSS pixels wide and scaled to its column, its
/// letter over it. `None` with fewer than two to show.
pub fn side_by_side_page(variants: &[(String, String)], here: impl Fn(&str) -> Option<String>, width: u32) -> Option<String> {
    let shown: Vec<(String, String)> = variants.iter().filter_map(|(l, u)| here(u).map(|h| (l.clone(), h))).collect();
    if shown.len() < 2 {
        return None;
    }
    let escape = |t: &str| t.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;");
    let columns: String = shown
        .iter()
        .map(|(label, url)| format!("<section><header>{}</header><div class=\"f\"><iframe src=\"{}\" width=\"{width}\"></iframe></div></section>", escape(label), escape(url)))
        .collect();
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><style>html,body{{margin:0;height:100%;background:#f4f4f5;font:600 12px -apple-system,system-ui,sans-serif}}\
@media (prefers-color-scheme:dark){{html,body{{background:#18181b;color:#e4e4e7}}}}\
main{{display:flex;gap:8px;height:100%;padding:8px;box-sizing:border-box}}section{{flex:1;min-width:0;display:flex;flex-direction:column}}\
header{{padding:2px 4px 6px}}.f{{flex:1;position:relative;overflow:hidden;border-radius:6px;background:#fff}}\
iframe{{position:absolute;left:0;top:0;border:0;transform-origin:0 0}}</style><main>{columns}</main>\
<script>function fit(){{for(const f of document.querySelectorAll('.f')){{const i=f.firstChild,s=f.clientWidth/{width};\
i.style.transform='scale('+s+')';i.style.height=(f.clientHeight/s)+'px';}}}}addEventListener('resize',fit);fit();</script>"
    );
    Some(format!("data:text/html;charset=utf-8,{}", percent(&html)))
}

/// `text` as it may stand in a `data:` address.
fn percent(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~ !*'(),;:@/?=+$".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out.replace(' ', "%20")
}

/// How the live page reaches this screen. Always through the daemon: a
/// page by the shared browser's stream (on its machine, where `localhost`
/// and the worktree's files are), a render as a picture `read_image` sends.
/// Never opened by this Mac's own browser when the daemon is elsewhere —
/// its `localhost` isn't the designer's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Stream(String),
    Picture(String),
}

pub fn route(live: &Live) -> Route {
    match live {
        Live::Page(url) => Route::Stream(url.clone()),
        Live::Render(path) => Route::Picture(path.clone()),
    }
}

/// The address "Open in browser" may open on this Mac: any page when the
/// daemon is here; from another machine only a public web page — never its
/// `localhost` or its files, which are that machine's.
pub fn open_here(url: &str, remote: bool) -> Option<String> {
    if !remote {
        return Some(url.to_owned());
    }
    let lower = url.to_ascii_lowercase();
    let host = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?.split(['/', ':', '?', '#']).next().unwrap_or_default().to_owned();
    let local = host.is_empty() || host == "localhost" || host.ends_with(".local") || host.starts_with("127.") || host.starts_with("10.") || host.starts_with("192.168.") || host == "0.0.0.0" || host == "[::1]";
    (!local).then(|| url.to_owned())
}

/// An address as tabs are matched by it: no `#…`, no trailing slash.
fn same_page(a: &str, b: &str) -> bool {
    let norm = |u: &str| u.split('#').next().unwrap_or_default().trim_end_matches('/').to_ascii_lowercase();
    norm(a) == norm(b)
}

/// What the screen asks of the shared browser for `url` to be live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tab {
    /// Watch this tab of the person's own: it is at the page.
    Watch(u64),
    /// Their session tab is elsewhere: send it to the page.
    Go(u64),
    /// No tab yet: open one at the page.
    Open,
}

/// [`Tab`]: the session's own tab (`ours`) when it is still open, else a tab
/// of the person's at the page.
pub fn tab_for(url: &str, ours: Option<u64>, mine: &[BrowserPage]) -> Tab {
    if let Some(page) = ours.and_then(|id| mine.iter().find(|p| p.id == id)) {
        return if same_page(&page.url, url) { Tab::Watch(page.id) } else { Tab::Go(page.id) };
    }
    match mine.iter().rev().find(|p| same_page(&p.url, url)) {
        Some(page) => Tab::Watch(page.id),
        None => Tab::Open,
    }
}

/// The tab just opened for the session: the newest of `mine` that wasn't
/// there before (`before`), at the page if one is.
pub fn opened(url: &str, before: &[u64], mine: &[BrowserPage]) -> Option<u64> {
    let new: Vec<&BrowserPage> = mine.iter().filter(|p| !before.contains(&p.id)).collect();
    new.iter().find(|p| same_page(&p.url, url)).or(new.last()).map(|p| p.id)
}

// ── pins ──

/// A pin on the screen: the core's pin, whether it went with a message yet,
/// and what the designer said changed once it marked it done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinUi {
    pub pin: Pin,
    pub sent: bool,
    pub done: Option<String>,
}

/// Where a click on the drawn page lands in the page: the picture drawn in a
/// `drawn` box (points, its top left at 0,0) of a frame of `header`; CSS
/// pixels from the page's top left, scrolling included. `None`: off the page.
pub fn place(click: (f32, f32), drawn: (f32, f32), header: &FrameHeader) -> Option<(u32, u32)> {
    let (x, y) = click;
    let (w, h) = drawn;
    if w <= 0. || h <= 0. || x < 0. || y < 0. || x > w || y > h {
        return None;
    }
    let css_x = f64::from(x / w) * header.css_w + header.sx;
    let css_y = f64::from(y / h) * header.css_h + header.sy - header.top;
    (css_y >= 0.).then(|| (css_x.round() as u32, css_y.round() as u32))
}

/// Where a pin shows over the page now (points in the `drawn` box), or
/// `None` when it is scrolled out of sight or of another width.
pub fn spot(pin: &Pin, drawn: (f32, f32), header: &FrameHeader) -> Option<(f32, f32)> {
    if (f64::from(pin.width) - header.css_w).abs() > 1. || header.css_w <= 0. || header.css_h <= 0. {
        return None;
    }
    let vx = (f64::from(pin.x) - header.sx) / header.css_w;
    let vy = (f64::from(pin.y) - header.sy + header.top) / header.css_h;
    ((0.0..=1.0).contains(&vx) && (0.0..=1.0).contains(&vy)).then(|| ((vx as f32) * drawn.0, (vy as f32) * drawn.1))
}

/// Where a click on a render lands, in its own pixels (`dims`), drawn in `drawn`.
pub fn place_on_render(click: (f32, f32), drawn: (f32, f32), dims: (u32, u32)) -> Option<(u32, u32)> {
    let (x, y) = click;
    if drawn.0 <= 0. || drawn.1 <= 0. || x < 0. || y < 0. || x > drawn.0 || y > drawn.1 {
        return None;
    }
    Some(((x / drawn.0 * dims.0 as f32).round() as u32, (y / drawn.1 * dims.1 as f32).round() as u32))
}

/// The crop that goes with a pin: a box `side` device pixels square around
/// the click, kept inside a `w`×`h` picture — (x, y, w, h).
pub fn crop_box(at: (u32, u32), (w, h): (u32, u32), side: u32) -> Option<(u32, u32, u32, u32)> {
    if w == 0 || h == 0 {
        return None;
    }
    let (cw, ch) = (side.min(w), side.min(h));
    let x = at.0.saturating_sub(cw / 2).min(w - cw);
    let y = at.1.saturating_sub(ch / 2).min(h - ch);
    Some((x, y, cw, ch))
}

/// The next pin's number.
pub fn next_number(pins: &[PinUi]) -> u32 {
    pins.iter().map(|p| p.pin.n).max().unwrap_or(0) + 1
}

/// The pins marked done since the session started, applied to `pins`.
pub fn mark_done(pins: &mut [PinUi], done: &[(u32, String)]) -> bool {
    let mut changed = false;
    for pin in pins.iter_mut() {
        if let Some((_, what)) = done.iter().find(|(n, _)| *n == pin.pin.n)
            && pin.done.as_deref() != Some(what.as_str())
        {
            pin.done = Some(what.clone());
            changed = true;
        }
    }
    changed
}

// ── the model and the tokens ──

/// The fast model a session's rounds run on, for a member on `engine` set to
/// `model` — `None` when it is on one already (or its engine names its own:
/// Codex's list is its own, so its rounds keep the member's model).
pub fn fast_model(engine: Option<Engine>, model: Option<&str>) -> Option<&'static str> {
    if engine.is_some_and(|e| e != Engine::Claude) {
        return None;
    }
    let family = model.and_then(equipe_core::models::family).map(|f| f.alias).or(model);
    match family {
        Some("sonnet" | "haiku") => None,
        _ => Some("sonnet"),
    }
}

/// The tokens used since the session started.
pub fn tokens_used(now: u64, at_start: u64) -> u64 {
    now.saturating_sub(at_start)
}

/// A token count as the header says it: `850`, `12.4k`, `1.2M`.
pub fn tokens_label(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1_000.),
        _ => format!("{:.1}M", n as f64 / 1_000_000.),
    }
}

// ── the session ──

/// A design session on screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub squad: String,
    pub designer: SessionId,
    pub project: Option<String>,
    pub page: String,
    pub from: SessionFrom,
    /// The designer's conversation length when it started: what came after is the session's.
    pub since: usize,
    pub tokens_at_start: u64,
    pub viewport: Viewport,
    pub pins: Vec<PinUi>,
    /// A click on the page waiting for its note.
    pub pending: Option<Pending>,
    /// The polish step: the stronger model.
    pub polish: bool,
    /// The model the designer was on before, to go back to.
    pub model_before: Option<String>,
    /// The shared browser's tab the page is in, and the tabs there were
    /// when one was asked for.
    pub tab: Option<u64>,
    pub asked_tab: Option<Vec<u64>>,
    /// How many times something was shown when the page last reloaded.
    pub reloaded_at: usize,
    /// The reactions given this session, to the round they were for.
    pub reactions: Vec<(usize, QuickReaction)>,
    /// A thumbs-down waiting for its reasons.
    pub down: Option<Vec<PickReason>>,
}

/// A pin dropped, its note still being written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub x: u32,
    pub y: u32,
    /// Where it shows, in the drawn box, while the note is written.
    pub at: (u32, u32),
    pub reasons: Vec<PickReason>,
    /// The crop, as PNG bytes, once made.
    pub crop: Option<Vec<u8>>,
    /// Dropped on the live preview (`views/preview.rs`): the page's width
    /// there, and the element the click was on.
    pub width: Option<u32>,
    pub selector: Option<String>,
    /// The element's words and its box in the page, when the page said.
    pub text: Option<String>,
    pub bounds: Option<[u32; 4]>,
}

impl Session {
    pub fn new(squad: &str, designer: SessionId, project: Option<String>, page: &str, from: SessionFrom, since: usize, tokens: u64) -> Self {
        Self {
            squad: squad.to_owned(),
            designer,
            project,
            page: page.to_owned(),
            from,
            since,
            tokens_at_start: tokens,
            viewport: Viewport::Desktop,
            pins: Vec::new(),
            pending: None,
            polish: false,
            model_before: None,
            tab: None,
            asked_tab: None,
            reloaded_at: 0,
            reactions: Vec::new(),
            down: None,
        }
    }

    /// The pins that go with the next message.
    pub fn unsent(&self) -> Vec<Pin> {
        self.pins.iter().filter(|p| !p.sent).map(|p| p.pin.clone()).collect()
    }

    /// Drop a pin with its note; `None` when the note is empty.
    pub fn drop_pin(&mut self, x: u32, y: u32, note: &str, reasons: Vec<PickReason>, selector: Option<String>) -> Option<u32> {
        let width = self.viewport.width();
        self.drop_pin_wide(x, y, width, note, reasons, selector)
    }

    /// [`Self::drop_pin`] on a page laid out `width` CSS pixels wide (the live preview's).
    pub fn drop_pin_wide(&mut self, x: u32, y: u32, width: u32, note: &str, reasons: Vec<PickReason>, selector: Option<String>) -> Option<u32> {
        self.drop_pin_on(x, y, width, note, reasons, selector, None, None)
    }

    /// [`Self::drop_pin_wide`] with the element's words and box, as the live preview's page says them.
    #[allow(clippy::too_many_arguments)]
    pub fn drop_pin_on(&mut self, x: u32, y: u32, width: u32, note: &str, reasons: Vec<PickReason>, selector: Option<String>, text: Option<String>, bounds: Option<[u32; 4]>) -> Option<u32> {
        let note = Pin::tidy(note);
        if note.is_empty() && reasons.is_empty() {
            return None;
        }
        let n = next_number(&self.pins);
        let pin = Pin { n, x, y, width, note, selector, reasons, crop: None, text, bounds };
        self.pins.push(PinUi { pin, sent: false, done: None });
        Some(n)
    }

    /// The unsent pins went with a message: each crop's place among the
    /// message's pictures given (`crops`: pin and its 1-based picture).
    pub fn sent(&mut self, crops: &[(u32, usize)]) -> Vec<Pin> {
        let mut out = Vec::new();
        for p in self.pins.iter_mut().filter(|p| !p.sent) {
            p.pin.crop = crops.iter().find(|(n, _)| *n == p.pin.n).map(|(_, c)| *c);
            p.sent = true;
            out.push(p.pin.clone());
        }
        out
    }

    /// The page should load again: something new was shown since it last did.
    pub fn should_reload(&mut self, shown: &Shown) -> bool {
        if shown.count > self.reloaded_at {
            self.reloaded_at = shown.count;
            return true;
        }
        false
    }
}

/// How "Design session" starts from where the person is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// On a designer's ticket: its own page, at once.
    Ticket(equipe_core::protocol::TicketId),
    /// In the designer's conversation: which page is asked.
    Ask,
}

/// [`Start`] for the member in view (`selected`: its role and ticket) and the
/// ticket opened (its id and its owner's role); `None`: nothing of the
/// designer's in view, no button.
pub fn start_for(selected: Option<(&str, Option<equipe_core::protocol::TicketId>)>, opened: Option<(equipe_core::protocol::TicketId, Option<&str>)>) -> Option<Start> {
    if let Some((id, role)) = opened {
        return (role == Some("designer")).then_some(Start::Ticket(id));
    }
    match selected {
        Some(("designer", Some(ticket))) => Some(Start::Ticket(ticket)),
        Some(("designer", None)) => Some(Start::Ask),
        _ => None,
    }
}

/// The name a crop goes by among the message's attachments.
pub fn crop_name(n: u32) -> String {
    format!("pin-{n}.png")
}

/// What a reaction's example is called in the style profile.
pub fn round_label(page: &str, round: usize) -> String {
    format!("{page} · round {round}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::ToolOutcome;

    fn tool(output: &str, ok: bool) -> Entry {
        Entry::Tool { tool: "Bash".into(), target: Some("equipe design session show".into()), outcome: Some(Box::new(ToolOutcome { ok, output: output.into(), ..ToolOutcome::default() })), label: None, background: false }
    }

    fn header(sx: f64, sy: f64) -> FrameHeader {
        FrameHeader { page: 1, seq: 1, w: 1440, h: 900, css_w: 1440., css_h: 900., sx, sy, top: 0. }
    }

    #[test]
    fn what_is_live_and_what_is_done_is_read_from_the_designers_commands_since_the_session_began() {
        let entries = vec![
            tool("design-session: show http://localhost:3000/old", true),
            Entry::User { text: "kickoff".into(), kept: vec![] },
            tool("Live.\ndesign-session: show http://localhost:5173/pricing", true),
            tool("design-session: show http://localhost:5173/broken", false),
            tool("design-session: done 1 Quieter accent", true),
            tool("design-session: render /w/design/sessions/r2.png", true),
            tool("design-session: done 1 Quieter accent, smaller", true),
        ];
        let got = shown(&entries, 1);
        assert_eq!(got.live, Some(Live::Render("/w/design/sessions/r2.png".into())));
        assert_eq!(got.count, 2, "what failed shows nothing");
        assert_eq!(got.done, [(1, "Quieter accent, smaller".to_owned())], "the latest word on a pin");
        assert_eq!(shown(&entries, 99), Shown::default());
        let mut session = Session::new("acme", 2, None, "Pricing", SessionFrom::DesignScreen, 1, 0);
        assert!(session.should_reload(&got));
        assert!(!session.should_reload(&got), "once per change");
    }

    #[test]
    fn the_prototypes_variants_are_kept_by_letter_for_side_by_side() {
        let entries = vec![
            tool("design-session: variant B http://localhost:5173/proto/b", true),
            tool("design-session: variant A http://localhost:5173/proto/a", true),
            tool("design-session: variant B http://localhost:5173/proto/b2", true),
        ];
        let got = shown(&entries, 0);
        assert_eq!(got.variants, [("A".to_owned(), "http://localhost:5173/proto/a".to_owned()), ("B".to_owned(), "http://localhost:5173/proto/b2".to_owned())]);
        assert_eq!(got.live, Some(Live::Page("http://localhost:5173/proto/b2".into())), "the latest shown is the one in front");
        assert_eq!(got.count, 3, "each one shown reloads");
    }

    #[test]
    fn use_this_picks_the_variant_in_front_or_the_page_shown() {
        let variants = vec![("A".to_owned(), "http://localhost:5173/a".to_owned()), ("B".to_owned(), "http://localhost:5173/b".to_owned())];
        let shown = Shown { live: Some(Live::Page("http://localhost:5173/b/".into())), count: 2, done: vec![], variants: variants.clone() };
        assert_eq!(variant_in_front(&shown.variants, shown.live.as_ref()), Some("B".into()));
        assert_eq!(picked(&shown, None, false), Some((Some("B".into()), "http://localhost:5173/b/".into())), "the one live");
        assert_eq!(picked(&shown, Some("A"), false), Some((Some("A".into()), "http://localhost:5173/a".into())), "the one picked");
        assert_eq!(picked(&shown, None, true), None, "side by side: pick one first");
        let one = Shown { live: Some(Live::Page("http://localhost:3000/".into())), count: 1, ..Shown::default() };
        assert_eq!(picked(&one, None, false), Some((None, "http://localhost:3000/".into())));
        assert_eq!(picked(&Shown { live: Some(Live::Render("/w/r.png".into())), ..Shown::default() }, None, false), None, "a picture is no prototype");
    }

    #[test]
    fn the_variants_go_side_by_side_in_one_page_at_their_addresses_here() {
        let variants = vec![("A".to_owned(), "http://localhost:5173/a".to_owned()), ("B".to_owned(), "http://localhost:5173/b?x=1&y=2".to_owned()), ("C".to_owned(), "http://localhost:9999/c".to_owned())];
        let here = |u: &str| u.strip_prefix("http://localhost:5173").map(|rest| format!("http://localhost:61000{rest}"));
        let page = side_by_side_page(&variants, here, 1440).unwrap();
        assert!(page.starts_with("data:text/html;charset=utf-8,"));
        assert!(!page.contains(' ') && !page.contains('"') && !page.contains('#') && !page.contains('<'), "{page}");
        let html = page.replace("%22", "\"").replace("%3C", "<").replace("%3E", ">").replace("%26", "&").replace("%20", " ");
        assert!(html.contains("<iframe src=\"http://localhost:61000/a\" width=\"1440\">"), "{html}");
        assert!(html.contains("src=\"http://localhost:61000/b?x=1&amp;y=2\""), "{html}");
        assert!(!html.contains("9999"), "one not reachable from here is left out");
        assert_eq!(side_by_side_page(&variants[..1], here, 1440), None);
    }

    #[test]
    fn the_live_page_always_comes_through_the_daemon_and_opens_here_only_when_it_can() {
        assert_eq!(route(&Live::Page("http://localhost:5173/".into())), Route::Stream("http://localhost:5173/".into()));
        assert_eq!(route(&Live::Render("/w/r.png".into())), Route::Picture("/w/r.png".into()));
        assert_eq!(open_here("http://localhost:5173/", false).as_deref(), Some("http://localhost:5173/"), "the daemon is on this Mac");
        assert_eq!(open_here("http://localhost:5173/", true), None, "another machine's localhost isn't ours");
        assert_eq!(open_here("file:///w/design/sessions/a.html", true), None);
        assert_eq!(open_here("http://192.168.1.4:3000/", true), None);
        assert_eq!(open_here("https://preview.example.com/p", true).as_deref(), Some("https://preview.example.com/p"));
    }

    #[test]
    fn the_session_watches_its_own_tab_or_opens_one() {
        let page = |id: u64, url: &str| BrowserPage { id, url: url.into(), ..BrowserPage::default() };
        let mine = [page(4, "https://example.com/"), page(7, "http://localhost:5173/pricing/")];
        assert_eq!(tab_for("http://localhost:5173/pricing", None, &mine), Tab::Watch(7));
        assert_eq!(tab_for("http://localhost:5173/home", Some(7), &mine), Tab::Go(7), "its tab goes where the designer shows");
        assert_eq!(tab_for("http://localhost:5173/home", None, &mine), Tab::Open);
        assert_eq!(tab_for("http://localhost:5173/pricing#top", Some(9), &mine), Tab::Watch(7), "its tab was closed: one at the page");
        let after = [page(4, "https://example.com/"), page(12, "about:blank")];
        assert_eq!(opened("file:///w/a.html", &[4], &after), Some(12), "the new one, still loading");
        assert_eq!(opened("file:///w/a.html", &[4, 12], &after), None);
    }

    #[test]
    fn a_click_on_the_page_becomes_a_pin_where_it_is_in_the_page_and_shows_there_as_it_scrolls() {
        // Drawn at 720×450: half size; the page scrolled 600 down.
        let h = header(0., 600.);
        assert_eq!(place((360., 100.), (720., 450.), &h), Some((720, 800)));
        assert_eq!(place((800., 100.), (720., 450.), &h), None, "off the page");
        let mut session = Session::new("acme", 2, None, "Pricing", SessionFrom::Member, 0, 0);
        assert_eq!(session.drop_pin(720, 800, "  too   loud ", vec![PickReason::Colors], None), Some(1));
        assert_eq!(session.drop_pin(1, 1, "  ", vec![], None), None, "nothing said, no pin");
        session.viewport = Viewport::Phone;
        assert_eq!(session.drop_pin(100, 40, "logo", vec![], None), Some(2));
        let pin = &session.pins[0].pin;
        assert_eq!((pin.note.as_str(), pin.width), ("too loud", 1440));
        assert_eq!(spot(pin, (720., 450.), &h), Some((360., 100.)));
        assert_eq!(spot(pin, (720., 450.), &header(0., 1000.)), None, "scrolled out of sight");
        assert_eq!(spot(&session.pins[1].pin, (720., 450.), &h), None, "drawn at another width");
        assert_eq!(place_on_render((50., 25.), (100., 50.), (2880, 1800)), Some((1440, 900)));
        assert_eq!(crop_box((10, 10), (1440, 900), 400), Some((0, 0, 400, 400)));
        assert_eq!(crop_box((1430, 890), (1440, 900), 400), Some((1040, 500, 400, 400)));
        assert_eq!(crop_box((5, 5), (100, 50), 400), Some((0, 0, 100, 50)));
    }

    #[test]
    fn pins_go_with_the_next_message_with_their_crops_and_are_ticked_when_done() {
        let mut session = Session::new("acme", 2, None, "Pricing", SessionFrom::Member, 0, 0);
        session.drop_pin(10, 20, "a", vec![], None);
        session.drop_pin(30, 40, "b", vec![], None);
        assert_eq!(session.unsent().len(), 2);
        let sent = session.sent(&[(2, 1)]);
        assert_eq!(sent.iter().map(|p| p.crop).collect::<Vec<_>>(), [None, Some(1)]);
        assert!(session.unsent().is_empty());
        let message = equipe_core::design::session::with_pins("http://localhost:5173/pricing", &sent, "these two");
        assert!(message.starts_with("<design-pins>\non: http://localhost:5173/pricing at 1440 wide\n- pin 1 at 10,20: a\n- pin 2 at 30,40: b\n  crop: attached picture 1"), "{message}");
        assert!(mark_done(&mut session.pins, &[(2, "Moved it".into())]));
        assert!(!mark_done(&mut session.pins, &[(2, "Moved it".into())]), "once");
        assert_eq!(session.pins[1].done.as_deref(), Some("Moved it"));
        assert_eq!(next_number(&session.pins), 3);
        assert_eq!(crop_name(3), "pin-3.png");
    }

    #[test]
    fn a_session_starts_from_a_designers_ticket_or_its_conversation_and_nowhere_else() {
        assert_eq!(start_for(None, Some((12, Some("designer")))), Some(Start::Ticket(12)));
        assert_eq!(start_for(Some(("designer", None)), Some((12, Some("frontend")))), None, "a frontend ticket open: none");
        assert_eq!(start_for(Some(("designer", Some(7))), None), Some(Start::Ticket(7)), "at work on a ticket: that one");
        assert_eq!(start_for(Some(("designer", None)), None), Some(Start::Ask));
        assert_eq!(start_for(Some(("pm", None)), None), None);
        assert_eq!(start_for(None, None), None);
    }

    #[test]
    fn rounds_run_on_a_fast_model_and_the_header_counts_the_sessions_tokens() {
        assert_eq!(fast_model(Some(Engine::Claude), Some("opus")), Some("sonnet"));
        assert_eq!(fast_model(Some(Engine::Claude), Some("claude-opus-5-5")), Some("sonnet"));
        assert_eq!(fast_model(Some(Engine::Claude), Some("claude-sonnet-5-5")), None, "on a fast one already");
        assert_eq!(fast_model(None, None), Some("sonnet"));
        assert_eq!(fast_model(Some(Engine::Codex), Some("gpt-5")), None, "Codex keeps its own");
        assert_eq!(tokens_used(15_000, 2_600), 12_400);
        assert_eq!(tokens_used(5, 10), 0);
        assert_eq!(tokens_label(850), "850");
        assert_eq!(tokens_label(12_400), "12.4k");
        assert_eq!(tokens_label(1_250_000), "1.2M");
    }
}
