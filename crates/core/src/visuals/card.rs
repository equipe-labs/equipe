//! HTML cards in a reply (```` ```html card ````), drawn here as PNGs —
//! light and dark — for every screen, the way [`super::diagram`] draws
//! Mermaid (docs/VISUALS.md, *HTML cards*). The card is put in the design
//! system's card (`design/visuals/cards/`: the tokens and `card.css`) and
//! photographed by a headless Chrome that is started for it alone, with a
//! profile made for it and thrown away after — never the person's, never the
//! members' — and that can't run a script or reach anything: the page's CSP
//! allows nothing but its own inline style and `data:` pictures, Chrome is
//! sent through a proxy that isn't there, names don't resolve, and what
//! Chrome does by itself in the background is off. It gets [`Limits::time`]
//! and a window of [`WIDTH`]×[`HEIGHT`]; past them it is killed or cut.
//! A card that breaks the contract ([`check`]) isn't drawn: its source shows.
//!
//! However many replies end at once, the daemon runs at most [`CHROMES`]
//! of them together; a reply draws at most [`PER_REPLY`] new cards in at
//! most [`REPLY_TIME`], waiting included, and what is past that shows its
//! source.
//!
//! Chrome's first start on a machine just booted can take far longer than
//! [`Limits::time`] (it reads its few hundred megabytes cold: 28 s against
//! 0.3 s warm, measured on Ubuntu 24.04). So the daemon draws one throwaway
//! card in the background as it starts ([`warm_up`]), and until a Chrome has
//! drawn once, a drawing gets [`FIRST_START`] instead ([`allowance`]) and a
//! reply that much more time ([`reply_time`]).
//!
//! Off until it has been through a security review (T-340): on with
//! `{"html_cards": true}` in `~/.equipe/visuals.json`, or `EQUIPE_HTML_CARDS=1`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use super::diagram;
use crate::protocol::{Entry, Rendered, RenderedImage};

/// The longest card drawn, in bytes.
pub const SOURCE_MAX: usize = 32 * 1024;
/// The column a card is drawn for, in CSS pixels…
pub const WIDTH: u32 = 640;
/// …and the most of it drawn: what is below is cut off.
pub const HEIGHT: u32 = 1200;
/// Drawn at twice its size, as a diagram is.
const SCALE: u32 = 2;
/// The longest a page kept for a screen to open may be.
pub const PAGE_MAX: u64 = 64 * 1024;

/// How many Chromes draw cards at once, in the whole daemon.
pub const CHROMES: usize = 2;
/// The most cards one reply has drawn (cards drawn before are read back,
/// not counted)…
pub const PER_REPLY: usize = 4;
/// …and the longest all of it may take, waiting for a Chrome included.
pub const REPLY_TIME: Duration = Duration::from_secs(30);
/// Replies waiting for a Chrome at once: one more draws none.
pub const WAITING_MAX: usize = 8;
/// What a drawing may take while no Chrome has drawn since the daemon
/// started: a cold first start, on a machine just booted.
pub const FIRST_START: Duration = Duration::from_secs(120);

/// Whether a Chrome has drawn here once (the warm-up or any card): Chrome's
/// files are read in, and a drawing takes its usual time.
static WARM: AtomicBool = AtomicBool::new(false);

/// Whether a Chrome has drawn once since this process started.
pub fn is_warm() -> bool {
    WARM.load(Ordering::Relaxed)
}

/// What one drawing may take: its `usual` time once a Chrome has drawn
/// here, and at least [`FIRST_START`] before that.
pub fn allowance(warm: bool, usual: Duration) -> Duration {
    if warm { usual } else { usual.max(FIRST_START) }
}

/// What one reply's cards may take: [`REPLY_TIME`], and room for Chrome's
/// first start too while no Chrome has drawn yet.
pub fn reply_time(warm: bool) -> Duration {
    if warm { REPLY_TIME } else { REPLY_TIME + FIRST_START }
}

/// What one drawing may take.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Chrome is killed past it, whatever it is doing.
    pub time: Duration,
    /// The window, in CSS pixels.
    pub width: u32,
    pub height: u32,
    /// Where every request Chrome makes is sent: a port nothing listens on.
    pub proxy: String,
}

impl Default for Limits {
    fn default() -> Self {
        Self { time: Duration::from_secs(10), width: WIDTH, height: HEIGHT, proxy: "127.0.0.1:9".into() }
    }
}

/// The Chromes running for cards, and the replies waiting for one.
struct Pool {
    running: usize,
    waiting: usize,
    /// The most that ever ran together: what the tests look at.
    most: usize,
}

static POOL: Mutex<Pool> = Mutex::new(Pool { running: 0, waiting: 0, most: 0 });
static FREED: Condvar = Condvar::new();

/// One of the [`CHROMES`]: given back when dropped.
struct Slot;

impl Slot {
    /// A Chrome to draw with, waiting no later than `until`; none past it,
    /// or when [`WAITING_MAX`] replies are waiting already.
    fn take(until: Instant) -> Option<Self> {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        if pool.running >= CHROMES && pool.waiting >= WAITING_MAX {
            return None;
        }
        pool.waiting += 1;
        while pool.running >= CHROMES {
            let Some(left) = until.checked_duration_since(Instant::now()).filter(|l| !l.is_zero()) else {
                pool.waiting -= 1;
                return None;
            };
            pool = FREED.wait_timeout(pool, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        pool.waiting -= 1;
        pool.running += 1;
        pool.most = pool.most.max(pool.running);
        Some(Self)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        POOL.lock().unwrap_or_else(|e| e.into_inner()).running -= 1;
        FREED.notify_one();
    }
}

/// The most Chromes that have drawn cards at once since the daemon started.
pub fn most_at_once() -> usize {
    POOL.lock().unwrap_or_else(|e| e.into_inner()).most
}

/// What one reply's cards may take, all together.
#[derive(Debug, Clone)]
pub struct Budget {
    /// Nothing is started past it, and a Chrome running then is killed.
    pub until: Instant,
    /// New cards still to be drawn.
    pub cards: usize,
    /// For each drawing; its time no later than `until`.
    pub limits: Limits,
}

impl Budget {
    /// A reply's: [`PER_REPLY`] cards in [`reply_time`], from now.
    pub fn reply() -> Self {
        Self { until: Instant::now() + reply_time(is_warm()), cards: PER_REPLY, limits: Limits::default() }
    }
}

/// Whether cards are drawn at all: off unless `visuals.json` in equipe's
/// home says `"html_cards": true`, or `EQUIPE_HTML_CARDS` is `1`.
pub fn enabled(home: &Path) -> bool {
    if std::env::var("EQUIPE_HTML_CARDS").is_ok_and(|v| v == "1") {
        return true;
    }
    let Ok(text) = std::fs::read_to_string(home.join("visuals.json")) else { return false };
    serde_json::from_str::<serde_json::Value>(&text).is_ok_and(|v| v["html_cards"] == true)
}

/// The source of each card of `text`: a fenced block whose info string is
/// the words `html card`, any case. Plain `html` is code being shown.
pub fn blocks(text: &str) -> Vec<String> {
    diagram::fenced(text, |info| {
        let words: Vec<String> = info.split_whitespace().map(str::to_ascii_lowercase).collect();
        words == ["html", "card"]
    })
}

/// Whether a reply has a card in it.
pub fn has_blocks(text: &str) -> bool {
    text.contains("card") && !blocks(text).is_empty()
}

/// Elements a card may not have: they run, load, embed, submit, or would
/// make the fragment a page of its own.
const TAGS: [&str; 24] = [
    "script", "iframe", "frame", "frameset", "object", "embed", "applet", "form", "input", "button", "link", "meta", "base", "html",
    "head", "body", "portal", "audio", "video", "source", "track", "foreignobject", "animate", "set",
];
/// Attributes that load something: only `data:` or `#` in them.
const LOADING: [&str; 13] =
    ["src", "srcset", "href", "xlink:href", "action", "formaction", "poster", "background", "data", "ping", "lowsrc", "dynsrc", "codebase"];
/// Elements a click or a key follows — HTML's and SVG's `<a>`, `<area>` —
/// whose `href` may only be a place in the page (`#…`): a card's page,
/// opened in the browser panel, goes nowhere (T-357).
const LINKS: [&str; 2] = ["a", "area"];
/// Attributes that send what a link points at somewhere else: a new window,
/// a download.
const LEAVING: [&str; 2] = ["target", "download"];

/// Whether `fragment` keeps the card contract (docs/VISUALS.md): small, one
/// fragment, nothing that runs or loads, no link out of the page. The first
/// fence only — the page's CSP and Chrome's blocked network hold whatever
/// gets past it.
pub fn check(fragment: &str) -> Result<(), String> {
    if fragment.len() > SOURCE_MAX {
        return Err(format!("{} bytes, more than {SOURCE_MAX}", fragment.len()));
    }
    let lower = fragment.to_ascii_lowercase();
    for (name, attrs) in tags(&lower) {
        if TAGS.contains(&name) {
            return Err(format!("<{name}>"));
        }
        for (attr, value) in attrs {
            if attr.starts_with("on") {
                return Err(format!("{attr}= on <{name}>"));
            }
            if LEAVING.contains(&attr) {
                return Err(format!("{attr}= on <{name}>"));
            }
            let link = LINKS.contains(&name) && (attr == "href" || attr == "xlink:href");
            if link && !url(value).starts_with('#') {
                return Err(format!("<{name} {attr}> goes to {value}"));
            }
            if LOADING.contains(&attr) && !inline(value) {
                return Err(format!("{attr}= loads {value}"));
            }
        }
    }
    if lower.contains("@import") || lower.contains("image-set(") {
        return Err("a stylesheet that loads".into());
    }
    let mut rest = lower.as_str();
    while let Some(at) = rest.find("url(") {
        rest = &rest[at + 4..];
        let arg = rest.trim_start().trim_start_matches(['"', '\'']);
        if !inline(arg) {
            return Err(format!("url({})", arg.chars().take(40).collect::<String>()));
        }
    }
    Ok(())
}

/// A URL as a browser reads it: without the spaces and controls around it.
/// Characters past ASCII stay — a browser keeps them too, so `\u{3000}#x`
/// isn't `#x`.
fn url(value: &str) -> &str {
    value.trim_matches(|c: char| c <= ' ')
}

/// A value that loads nothing: a `data:` picture, or a place in the page.
fn inline(value: &str) -> bool {
    let value = url(value);
    value.starts_with("data:image/") || value.starts_with('#')
}

/// Each start tag of `html` (lowercased): its name and its attributes.
/// Every `<` and a letter is read as the start of a tag, even inside an
/// attribute's value, a comment or a `<style>`: what a browser takes for a
/// tag — a `<style>` that ends early, a comment closed by `--!>` — is among
/// them, however this reads the rest.
fn tags(html: &str) -> Vec<(&str, Vec<(&str, &str)>)> {
    let bytes = html.as_bytes();
    let mut found = Vec::new();
    for open in 0..bytes.len() {
        if bytes[open] == b'<' && bytes.get(open + 1).is_some_and(u8::is_ascii_alphabetic) {
            found.push(tag(html, open + 1));
        }
    }
    found
}

/// The tag whose name starts at `i`, read as HTML's tokenizer does.
fn tag(html: &str, mut i: usize) -> (&str, Vec<(&str, &str)>) {
    let bytes = html.as_bytes();
    let start = i;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'>' | b'/') {
        i += 1;
    }
    let name = &html[start..i];
    let mut attrs = Vec::new();
    loop {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'>' {
            break;
        }
        // A name may start with `=` (HTML keeps it in the name).
        let at = i;
        i += 1;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'>' | b'=' | b'/') {
            i += 1;
        }
        let attr = &html[at..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = "";
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let quote = bytes.get(i).copied().filter(|q| *q == b'"' || *q == b'\'');
            let from = if quote.is_some() { i + 1 } else { i };
            i = match quote {
                // std's search, quick even in a debug build: an unclosed quote runs to the end.
                Some(q) => html[from..].find(q as char).map_or(bytes.len(), |n| from + n),
                None => from + html[from..].find(|c: char| c.is_ascii_whitespace() || c == '>').unwrap_or(html.len() - from),
            };
            value = &html[from..i];
            if quote.is_some() && i < bytes.len() {
                i += 1;
            }
        }
        attrs.push((attr, value));
    }
    (name, attrs)
}

/// The design system's tokens and card styles (T-336), the card's frame.
const TOKENS_CSS: &str = include_str!("../../../../design/build/css/tokens.css");
const CARD_CSS: &str = include_str!("../../../../design/visuals/cards/card.css");
/// Nothing but the card is drawn: no page around it, a transparent ground,
/// and the five colours a card may use by name.
const FRAME_CSS: &str = "html,body{margin:0!important;padding:0!important;background:transparent!important}\
.vc-card{--fg:var(--eq-color-text);--bg:var(--eq-card-bg);--muted:var(--eq-color-muted);--accent:var(--eq-color-accent);--line:var(--eq-color-line)}";
/// Nothing loads, nothing runs, nothing is sent: the page's own inline style
/// and `data:` pictures only.
pub const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; base-uri 'none'; form-action 'none'";

/// The page a card is drawn in: `fragment` in the design system's card, in
/// a tone (`Some(dark)`), or following the screen's (`None`, the page kept).
pub fn page(fragment: &str, dark: Option<bool>) -> String {
    let theme = match dark {
        Some(true) => " data-theme=\"dark\"",
        Some(false) => " data-theme=\"light\"",
        None => "",
    };
    let lang = lang_of(fragment);
    format!(
        "<!doctype html><html lang=\"{lang}\"{theme}><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"{CSP}\">\
<meta name=\"viewport\" content=\"width={WIDTH}\"><style>{TOKENS_CSS}\n{CARD_CSS}\n{FRAME_CSS}</style></head>\
<body><main class=\"vc\"><article class=\"vc-card\">{fragment}</article></main></body></html>"
    )
}

/// The language a card is written in, as its page says (`lang`) so its type
/// is set for that script ([`TOKENS_CSS`]'s `:lang()` faces): what its words
/// are in, else English.
fn lang_of(fragment: &str) -> &'static str {
    let mut words = String::new();
    let mut in_tag = false;
    for c in fragment.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                words.push(' ');
            }
            _ if !in_tag => words.push(c),
            _ => {}
        }
    }
    match crate::language::detect(&words) {
        Some("Chinese") => "zh",
        Some(name) => crate::i18n::Lang::ALL.iter().find(|l| l.english_name() == name).map_or("en", |l| l.code()),
        None => "en",
    }
}

/// The Chrome that draws: `EQUIPE_CHROME`, or Google Chrome, Chrome for
/// Testing or Chromium where they are usually installed.
pub fn chrome() -> Option<PathBuf> {
    if let Some(given) = std::env::var_os("EQUIPE_CHROME") {
        return Some(PathBuf::from(given)).filter(|p| p.is_file());
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    [
        PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        home.join("Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        PathBuf::from("/Applications/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"),
        PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        PathBuf::from("/usr/bin/google-chrome"),
        PathBuf::from("/usr/bin/chromium"),
        PathBuf::from("/usr/bin/chromium-browser"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// How Chrome is started to draw: headless, a fresh profile (given apart),
/// no sign-in, no keychain, nothing of its own in the background, and every
/// request it might make — the page's or its own — sent to a proxy on a port
/// nothing listens on, loopback included, with no name resolving.
const CHROME_ARGS: [&str; 21] = [
    "--headless",
    "--no-first-run",
    "--no-default-browser-check",
    "--use-mock-keychain",
    "--password-store=basic",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-sync",
    "--disable-extensions",
    "--disable-default-apps",
    "--disable-client-side-phishing-detection",
    "--disable-domain-reliability",
    "--disable-features=Translate,OptimizationHints,MediaRouter,AutofillServerCommunication,InterestFeedContentSuggestions",
    "--no-pings",
    "--mute-audio",
    "--disable-gpu",
    "--hide-scrollbars",
    "--proxy-bypass-list=<-loopback>",
    "--host-resolver-rules=MAP * ~NOTFOUND",
    "--default-background-color=00000000",
    "--force-device-scale-factor=2",
];

/// A folder of one drawing's own — the page, the profile, the picture —
/// removed when it is done, however it ended.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, String> {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!("equipe-card-{}-{n}-{nanos}", std::process::id()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        builder.create(&dir).map_err(|e| format!("no scratch folder: {e}"))?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `html` photographed by `chrome` in a tone: a PNG at twice its size, as
/// tall as what it shows (no more than the window), and its size in pixels.
pub fn render(chrome: &Path, html: &str, dark: bool, limits: &Limits) -> Result<(Vec<u8>, u32, u32), String> {
    let scratch = Scratch::new()?;
    let page = scratch.0.join("card.html");
    let shot = scratch.0.join("card.png");
    std::fs::write(&page, html).map_err(|e| e.to_string())?;
    let mut command = Command::new(chrome);
    command
        .args(CHROME_ARGS)
        .arg(format!("--user-data-dir={}", scratch.0.join("profile").display()))
        .arg(format!("--proxy-server=http://{}", limits.proxy))
        .arg(format!("--window-size={},{}", limits.width, limits.height))
        .arg(format!("--screenshot={}", shot.display()));
    if dark {
        // A card's own `prefers-color-scheme: dark` too.
        command.arg("--force-dark-mode");
    }
    command.arg(format!("file://{}", page.display().to_string().replace('%', "%25").replace(' ', "%20")));
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|e| format!("Chrome didn't start: {e}"))?;
    let group = child.id();
    let deadline = Instant::now() + limits.time;
    let ended = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => break Err(format!("not drawn in {:?}", limits.time)),
            Err(e) => break Err(e.to_string()),
        }
    };
    // Chrome and its helpers, whatever is left of them: the group it leads.
    // SAFETY: a signal to a process group of ours; ESRCH when it is gone.
    crate::group::end(group, crate::group::Signal::Kill);
    let _ = child.wait();
    ended?;
    let bytes = std::fs::read(&shot).map_err(|_| "Chrome drew nothing".to_owned())?;
    // It started and drew: whatever it read from disk is read now.
    WARM.store(true, Ordering::Relaxed);
    if bytes.len() > diagram::PNG_MAX {
        return Err(format!("{} bytes, bigger than {}", bytes.len(), diagram::PNG_MAX));
    }
    fitted(&bytes, limits)
}

/// The picture cut to what it shows: the rows below the card, transparent,
/// left off; never past the window, at [`SCALE`].
fn fitted(png: &[u8], limits: &Limits) -> Result<(Vec<u8>, u32, u32), String> {
    use resvg::tiny_skia::{IntRect, Pixmap};
    let pixmap = Pixmap::decode_png(png).map_err(|e| e.to_string())?;
    let (width, most) = (pixmap.width(), pixmap.height().min(limits.height * SCALE));
    let rows = pixmap.pixels().chunks(width as usize).take(most as usize).rposition(|row| row.iter().any(|p| p.alpha() > 0)).ok_or("an empty card")? as u32 + 1;
    let cut = pixmap.clone_rect(IntRect::from_xywh(0, 0, width, rows).ok_or("an empty card")?).ok_or("an empty card")?;
    let png = cut.encode_png().map_err(|e| e.to_string())?;
    Ok((png, width, rows))
}

/// The page of a card kept for a screen to open: `<key>.html` in the squad's `visuals/`.
pub fn page_file(home: &Path, squad: &str, key: &str) -> PathBuf {
    diagram::dir(home, squad).join(format!("{key}.html"))
}

/// Whether `path` is one of the squad's kept card pages, by its name alone.
pub fn is_page(home: &Path, squad: &str, path: &Path) -> bool {
    let named = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".html"));
    named.is_some_and(|k| k.len() == 16 && k.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))) && path.parent() == Some(diagram::dir(home, squad).as_path())
}

/// The card in a kept page ([`page`]), if it still keeps the contract: a
/// page kept under an older one (links out, before T-357) is not given out.
pub fn kept_fragment(html: &str) -> Result<&str, String> {
    let open = "<article class=\"vc-card\">";
    let from = html.find(open).ok_or("not a card's page")? + open.len();
    let fragment = html[from..].strip_suffix("</article></main></body></html>").ok_or("not a card's page")?;
    check(fragment)?;
    Ok(fragment)
}

/// Whole or not at all: a screen never reads half a file.
fn keep(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
    let partial = path.with_extension("part");
    std::fs::write(&partial, bytes).and_then(|()| std::fs::rename(&partial, path)).map_err(|e| e.to_string())
}

/// The card drawn in one tone and kept (once: a key drawn before is read
/// back), with one of the daemon's [`CHROMES`], within `budget`'s time.
fn draw_one(home: &Path, squad: &str, key: &str, source: &str, chrome: &Path, dark: bool, budget: &Budget) -> Result<RenderedImage, String> {
    let path = diagram::file(home, squad, key, dark);
    if let Some(kept) = diagram::read_size(&path) {
        super::cache::touch(&path);
        return Ok(kept);
    }
    let _slot = Slot::take(budget.until).ok_or("no Chrome free in the reply's time")?;
    let left = budget.until.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err("out of the reply's time".into());
    }
    // Before any Chrome has drawn here (the warm-up still going), Chrome's first start's allowance.
    let limits = Limits { time: allowance(is_warm(), budget.limits.time).min(left), ..budget.limits.clone() };
    let (png, width, height) = render(chrome, &page(source, Some(dark)), dark, &limits)?;
    keep(&path, &png)?;
    Ok(RenderedImage { path: path.to_string_lossy().into_owned(), width, height })
}

/// Each card of `text` that drew, in both tones, with its page kept beside
/// them. Nothing while cards are off ([`enabled`]) or there is no Chrome; a
/// card too long, past [`diagram::PER_REPLY`], breaking the contract, failing
/// to draw or past the reply's [`Budget`] is left out: its source is shown.
pub fn draw(home: &Path, squad: &str, text: &str) -> Vec<Rendered> {
    if !enabled(home) {
        return Vec::new();
    }
    let Some(chrome) = chrome() else {
        crate::daemon_log!("equipe: no Chrome to draw the cards of {squad}; their source shows");
        return Vec::new();
    };
    draw_within(home, squad, text, &chrome, Budget::reply())
}

/// Chrome started once, in the background, as the daemon starts — while cards
/// are on and there is a Chrome: one throwaway card drawn with one of the
/// [`CHROMES`] and [`FIRST_START`] to do it in, the way every card is drawn
/// (its own throwaway profile, no network, its process group killed after).
/// It never holds up the caller; a card drawn meanwhile gets the first
/// start's allowance too. `None` when there is nothing to warm.
pub fn warm_up(home: &Path) -> Option<std::thread::JoinHandle<()>> {
    if !enabled(home) || is_warm() {
        return None;
    }
    let chrome = chrome()?;
    let spawned = std::thread::Builder::new().name("card-warm-up".into()).spawn(move || {
        let started = Instant::now();
        let Some(_slot) = Slot::take(started + FIRST_START) else { return };
        let limits = Limits { time: FIRST_START, ..Limits::default() };
        match render(&chrome, &page("<p>warm</p>", Some(false)), false, &limits) {
            Ok(_) => crate::daemon_log!("equipe: Chrome started for cards in {:.1}s", started.elapsed().as_secs_f32()),
            Err(why) => crate::daemon_log!("equipe: Chrome's first start for cards failed after {:.1}s ({why})", started.elapsed().as_secs_f32()),
        }
        // Tried once, however it went: a Chrome that won't draw in the first
        // start's time won't for a card either, and the cards after get their
        // usual time rather than tying a Chrome up for minutes each.
        WARM.store(true, Ordering::Relaxed);
    });
    spawned.ok()
}

/// [`draw`] with this Chrome, within `budget`.
pub fn draw_within(home: &Path, squad: &str, text: &str, chrome: &Path, mut budget: Budget) -> Vec<Rendered> {
    let mut drawn: Vec<Rendered> = Vec::new();
    let mut over = 0;
    for source in blocks(text).into_iter().take(diagram::PER_REPLY) {
        let key = diagram::key(&source);
        if source.trim().is_empty() || drawn.iter().any(|r| r.key == key) {
            continue;
        }
        let new = [false, true].iter().any(|&dark| !diagram::file(home, squad, &key, dark).is_file());
        if new && (budget.cards == 0 || Instant::now() >= budget.until) {
            over += 1;
            continue;
        }
        let made = check(&source).and_then(|()| {
            if new {
                budget.cards -= 1;
            }
            let light = draw_one(home, squad, &key, &source, chrome, false, &budget)?;
            let dark = draw_one(home, squad, &key, &source, chrome, true, &budget)?;
            let page_path = page_file(home, squad, &key);
            keep(&page_path, page(&source, None).as_bytes())?;
            Ok((light, dark, page_path))
        });
        match made {
            Ok((light, dark, page_path)) => drawn.push(Rendered { kind: "card".into(), key, light, dark, page: Some(page_path.to_string_lossy().into_owned()) }),
            Err(why) => crate::daemon_log!("equipe: a card of {squad} wasn't drawn ({why}); its source shows"),
        }
    }
    if over > 0 {
        crate::daemon_log!("equipe: {over} card(s) of a reply of {squad} past what a reply may draw; their source shows");
    }
    drawn
}

/// The cards of these lines' replies drawn before, read back without
/// drawing: for a screen that attaches later. Only those that keep the
/// contract as it is now.
pub fn kept(home: &Path, squad: &str, lines: &[Entry]) -> Vec<Rendered> {
    let mut found: Vec<Rendered> = Vec::new();
    for line in lines {
        let Entry::Reply { text } = line else { continue };
        for source in blocks(text).into_iter().take(diagram::PER_REPLY) {
            let key = diagram::key(&source);
            let page_path = page_file(home, squad, &key);
            // A page kept under an older, looser contract isn't offered (T-357).
            if found.iter().any(|r| r.key == key) || !page_path.is_file() || check(&source).is_err() {
                continue;
            }
            if let (Some(light), Some(dark)) = (diagram::read_size(&diagram::file(home, squad, &key, false)), diagram::read_size(&diagram::file(home, squad, &key, true))) {
                found.push(Rendered { kind: "card".into(), key, light, dark, page: Some(page_path.to_string_lossy().into_owned()) });
            }
        }
    }
    found
}

#[cfg(all(test, unix))]
mod tests;
