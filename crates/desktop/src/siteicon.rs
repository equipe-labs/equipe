//! A service's own icon, for the Logins › Connections list where Simple
//! Icons has no logo (Mobbin, 21st.dev, AWS) and for any server added by
//! address: fetched at run time from the service's own site, by this app on
//! this Mac (never the daemon), and kept on disk. Nothing of it is shipped
//! in the repository: the icon is shown only to identify the service
//! (`THIRD_PARTY_NOTICES.md`), and the row's monogram stands in until it
//! arrives — and for good when none does.
//!
//! What is fetched is narrow: https only (a redirect to anything else
//! stops), one icon at most 256 KB, only PNG or ICO (read by the file's own
//! first bytes, whatever the server calls it; SVG is skipped, as drawing
//! a stranger's SVG is not safe), no cookies or credentials, a few seconds
//! at most. An accepted icon is decoded, shrunk to at most 64 points a side
//! and kept as a PNG under `<home>/cache/site-icons/` for 30 days (a day for
//! "no icon here", so a site is not asked again at every launch); a stale
//! one is still shown when the site cannot be reached.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use futures::AsyncReadExt as _;
use gpui_kit::http_client::{AsyncBody, HttpClient, HttpRequestExt as _, Method, RedirectPolicy, Url};
use gpui_kit::*;

use crate::Shell;

/// The most an icon, as sent, may be.
pub const MAX_BYTES: usize = 256 * 1024;
/// The most a homepage is read for its `<link rel=icon>`.
const MAX_PAGE: usize = 512 * 1024;
/// How long a fetched icon is kept, and how long "no icon here" is.
pub const TTL: Duration = Duration::from_secs(30 * 86_400);
pub const MISS_TTL: Duration = Duration::from_secs(86_400);
/// One request, whole (connecting through the last byte).
const TIMEOUT: Duration = Duration::from_secs(8);
/// Redirects followed for one address (each one https).
pub const MAX_HOPS: usize = 3;
/// The icon is kept at most this many pixels a side; smaller than `MIN_SIDE` is no icon.
const KEEP_SIDE: u32 = 64;
const MIN_SIDE: u32 = 16;
/// The largest a picture may claim to be before it is decoded.
const MAX_SIDE: u32 = 1024;

/// What to show for one service: where its icon may be (tried in order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    /// Names its cache file: a registrable domain, or `catalog-<id>`.
    pub key: String,
    /// Addresses of the icon itself, then (`homepage`) the page whose `<link rel=icon>` may name one.
    pub urls: Vec<String>,
    pub homepage: Option<String>,
    /// Tried last when the page named none (a custom server's `/favicon.ico`).
    pub last: Vec<String>,
}

/// The official icon of a catalog service Simple Icons has no logo for, on
/// its own domain (checked 2026-11-15 to answer with an image).
pub fn for_catalog(id: &str) -> Option<Wanted> {
    let urls: &[&str] = match id {
        // No apple-touch-icon there (404): the site's own favicon.
        "mobbin" => &["https://mobbin.com/favicon.ico"],
        "21st" => &["https://21st.dev/apple-touch-icon.png", "https://21st.dev/favicon.ico"],
        // The touch icon redirects (https, within amazon's own hosts) to a 144px PNG; the favicon is 16px.
        "aws" => &["https://aws.amazon.com/apple-touch-icon.png", "https://aws.amazon.com/favicon.ico"],
        _ => return None,
    };
    Some(Wanted { key: format!("catalog-{id}"), urls: urls.iter().map(|u| (*u).to_owned()).collect(), homepage: None, last: Vec::new() })
}

/// A server added by address: its site's icon, from the address's registrable domain.
pub fn for_server_url(server_url: &str) -> Option<Wanted> {
    let domain = site_of(server_url)?;
    Some(Wanted { urls: vec![format!("https://{domain}/apple-touch-icon.png")], homepage: Some(format!("https://{domain}/")), last: vec![format!("https://{domain}/favicon.ico")], key: domain })
}

/// What a kept key calls its service, when that reads as a host (`deploy.acme.example`).
pub fn for_service_text(service: &str) -> Option<Wanted> {
    let s = service.trim();
    if s.contains(char::is_whitespace) || s.contains('/') || !s.contains('.') {
        return None;
    }
    for_server_url(&format!("https://{s}"))
}

/// The two-label suffixes under which a name is registered (`example.co.uk`): the common ones, as no list of
/// public suffixes is shipped. A name under one this misses is asked for at its own host, which is harmless.
const SECOND_LEVEL: [&str; 9] = ["co", "com", "org", "net", "ac", "gov", "edu", "ne", "or"];

/// The registrable domain of an https address: `mcp.figma.com` as `figma.com`. `None`: not https, no host, an
/// address (IP, `localhost`, a name with no dot, `.local`/`.internal`/`.lan`) that is not a public site's.
pub fn site_of(address: &str) -> Option<String> {
    let url = Url::parse(address).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    let host = url.host_str()?.trim_end_matches('.').to_lowercase();
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
        return None;
    }
    let labels: Vec<&str> = host.split('.').collect();
    let valid = |l: &&str| !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && !l.starts_with('-') && !l.ends_with('-');
    if labels.len() < 2 || !labels.iter().all(valid) {
        return None;
    }
    if matches!(labels[labels.len() - 1], "local" | "internal" | "lan" | "localhost" | "invalid" | "home" | "corp") {
        return None;
    }
    let n = labels.len();
    let keep = if n >= 3 && labels[n - 1].len() == 2 && SECOND_LEVEL.contains(&labels[n - 2]) { 3 } else { 2 };
    Some(labels[n - keep..].join("."))
}

/// Whether `url` may be fetched at all: https, a host, no credentials in it.
pub fn https_ok(url: &Url) -> bool {
    url.scheme() == "https" && url.host_str().is_some() && url.username().is_empty() && url.password().is_none()
}

/// What an icon file is, by its first bytes (not by what the server called it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Png,
    Ico,
}

/// PNG or ICO by magic; anything else (SVG, HTML, JPEG, a lie) is `None`.
pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(Kind::Png)
    } else if bytes.starts_with(&[0, 0, 1, 0]) && bytes.len() > 6 && u16::from_le_bytes([bytes[4], bytes[5]]) > 0 {
        Some(Kind::Ico)
    } else {
        None
    }
}

/// Whether `bytes`, as received, are an icon this app keeps: small enough, PNG or ICO.
pub fn acceptable(bytes: &[u8]) -> Option<Kind> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return None;
    }
    sniff(bytes)
}

/// An icon, shrunk and re-encoded as a PNG; `None` when it is not a picture, claims to be huge or is tiny.
pub fn normalize(bytes: &[u8]) -> Option<Vec<u8>> {
    let kind = acceptable(bytes)?;
    let format = match kind {
        Kind::Png => image::ImageFormat::Png,
        Kind::Ico => image::ImageFormat::Ico,
    };
    let (w, h) = image::ImageReader::with_format(Cursor::new(bytes), format).into_dimensions().ok()?;
    if w.max(h) > MAX_SIDE || w.min(h) < MIN_SIDE {
        return None;
    }
    let picture = image::load_from_memory_with_format(bytes, format).ok()?;
    let picture = if picture.width().max(picture.height()) > KEEP_SIDE { picture.resize(KEEP_SIDE, KEEP_SIDE, image::imageops::FilterType::Lanczos3) } else { picture };
    let mut out = Cursor::new(Vec::new());
    picture.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// The icon addresses a page names: `<link rel="apple-touch-icon" …>` first, then `rel="icon"` /
/// `"shortcut icon"`, resolved against the page's address, https only, SVG left out.
pub fn icon_links(html: &str, base: &Url) -> Vec<Url> {
    let lower = html.to_ascii_lowercase();
    let mut touch = Vec::new();
    let mut plain = Vec::new();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<link") {
        let start = at + i;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        let tag = &html[start..end];
        at = end.max(start + 5);
        let rel = attribute(tag, "rel").map(|r| r.to_ascii_lowercase());
        let (Some(rel), Some(href)) = (rel, attribute(tag, "href")) else { continue };
        let words: Vec<&str> = rel.split_whitespace().collect();
        let is_touch = words.iter().any(|w| w.starts_with("apple-touch-icon"));
        let is_icon = words.contains(&"icon");
        if !is_touch && !is_icon {
            continue;
        }
        let Ok(url) = base.join(href.trim()) else { continue };
        if !https_ok(&url) || url.path().to_ascii_lowercase().ends_with(".svg") {
            continue;
        }
        let list = if is_touch { &mut touch } else { &mut plain };
        if !list.contains(&url) {
            list.push(url);
        }
    }
    touch.extend(plain);
    touch
}

/// A tag's attribute value: `href="x"`, `href='x'` or `href=x`.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=').filter(|_| before_ok) else { continue };
        let rest = rest.trim_start();
        return match rest.chars().next()? {
            q @ ('"' | '\'') => rest[1..].split(q).next(),
            _ => rest.split(|c: char| c.is_whitespace() || c == '>').next(),
        };
    }
    None
}

// ───────────────────────── the cache ─────────────────────────

/// What the disk holds for a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cached {
    /// A kept icon within its time: use it.
    Fresh,
    /// "No icon here", asked lately: leave the monogram, do not ask again yet.
    KnownMissing,
    /// Nothing, or too old: ask the site (an old icon, if there is one, stays the fallback).
    Fetch,
}

/// From the ages of the kept icon and of the "no icon" note (`None`: no such file).
pub fn cache_state(icon_age: Option<Duration>, miss_age: Option<Duration>) -> Cached {
    match (icon_age, miss_age) {
        (Some(a), _) if a < TTL => Cached::Fresh,
        (None, Some(m)) if m < MISS_TTL => Cached::KnownMissing,
        _ => Cached::Fetch,
    }
}

/// Where icons are kept.
pub fn cache_dir() -> PathBuf {
    equipe_providers::credentials::equipe_home().join("cache").join("site-icons")
}

fn file_name(key: &str) -> String {
    key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect()
}

fn icon_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{}.png", file_name(key)))
}

fn miss_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{}.none", file_name(key)))
}

fn age(path: &Path) -> Option<Duration> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(SystemTime::now().duration_since(modified).unwrap_or_default())
}

fn kept(dir: &Path, key: &str) -> (Cached, Option<PathBuf>) {
    let icon = icon_path(dir, key);
    let state = cache_state(age(&icon), age(&miss_path(dir, key)));
    (state, icon.is_file().then_some(icon))
}

fn keep(dir: &Path, key: &str, png: &[u8]) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let path = icon_path(dir, key);
    let tmp = dir.join(format!("{}.tmp{}", file_name(key), std::process::id()));
    std::fs::write(&tmp, png).ok()?;
    std::fs::rename(&tmp, &path).ok()?;
    let _ = std::fs::remove_file(miss_path(dir, key));
    Some(path)
}

fn note_missing(dir: &Path, key: &str) {
    if std::fs::create_dir_all(dir).is_ok() {
        let _ = std::fs::write(miss_path(dir, key), b"");
    }
}

// ───────────────────────── the fetch ─────────────────────────

/// One address's body, redirects (https only, a few) followed by hand: at most `limit` bytes, or an error.
async fn get(client: &Arc<dyn HttpClient>, address: &str, limit: usize) -> anyhow::Result<Vec<u8>> {
    let mut url = Url::parse(address)?;
    for _ in 0..=MAX_HOPS {
        anyhow::ensure!(https_ok(&url), "not an https address");
        // No cookies, no credentials: a bare GET.
        let request = gpui_kit::http_client::Request::builder()
            .method(Method::GET)
            .uri(url.as_str())
            .header("Accept", "image/png,image/x-icon,image/vnd.microsoft.icon,text/html;q=0.5")
            .follow_redirects(RedirectPolicy::NoFollow)
            .timeout(TIMEOUT)
            .body(AsyncBody::empty())?;
        let mut response = client.send(request).await?;
        let status = response.status();
        if status.is_redirection() {
            let to = response.headers().get("location").and_then(|v| v.to_str().ok()).ok_or_else(|| anyhow::anyhow!("a redirect to nowhere"))?;
            url = url.join(to)?;
            continue;
        }
        anyhow::ensure!(status.is_success(), "answered {status}");
        if let Some(len) = response.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<usize>().ok()) {
            anyhow::ensure!(len <= limit, "too large");
        }
        let mut body = Vec::new();
        response.body_mut().take(limit as u64 + 1).read_to_end(&mut body).await?;
        anyhow::ensure!(body.len() <= limit, "too large");
        return Ok(body);
    }
    anyhow::bail!("too many redirects")
}

/// Where the bytes come from: the network, or (a snapshot build) a folder of fixtures.
#[derive(Clone)]
enum Transport {
    Net(Arc<dyn HttpClient>),
    #[cfg(feature = "snapshot")]
    Fixtures(PathBuf),
    #[cfg(feature = "snapshot")]
    Off,
}

impl Transport {
    fn of(cx: &App) -> Self {
        #[cfg(feature = "snapshot")]
        {
            // A snapshot never reaches the network by accident: fixtures from a folder, or `EQUIPE_SITE_ICONS_LIVE=1`.
            if let Some(dir) = std::env::var_os("EQUIPE_SITE_ICON_DIR") {
                return Self::Fixtures(PathBuf::from(dir));
            }
            if std::env::var_os("EQUIPE_SITE_ICONS_LIVE").is_none() {
                return Self::Off;
            }
        }
        Self::Net(cx.http_client())
    }

    async fn fetch(&self, key: &str, address: &str, limit: usize) -> anyhow::Result<Vec<u8>> {
        match self {
            Self::Net(client) => get(client, address, limit).await,
            // The fixture's name is the cache key; the address is only asked for its shape (https).
            #[cfg(feature = "snapshot")]
            Self::Fixtures(dir) => {
                anyhow::ensure!(Url::parse(address).is_ok_and(|u| https_ok(&u)), "not an https address");
                if address.ends_with('/') {
                    anyhow::bail!("no page in fixtures");
                }
                ["png", "ico"].iter().find_map(|ext| std::fs::read(dir.join(format!("{}.{ext}", file_name(key)))).ok()).ok_or_else(|| anyhow::anyhow!("no fixture"))
            }
            #[cfg(feature = "snapshot")]
            Self::Off => anyhow::bail!("snapshot: no network"),
        }
    }
}

/// The icon for `wanted`, as PNG bytes: the first address that answers with an acceptable picture.
async fn fetch_icon(transport: &Transport, wanted: &Wanted) -> Option<Vec<u8>> {
    let mut tried: Vec<String> = Vec::new();
    let mut try_one = |address: String| {
        let fresh = !tried.contains(&address);
        if fresh {
            tried.push(address.clone());
        }
        (fresh, address)
    };
    for address in &wanted.urls {
        let (fresh, address) = try_one(address.clone());
        if fresh && let Some(png) = transport.fetch(&wanted.key, &address, MAX_BYTES).await.ok().and_then(|b| normalize(&b)) {
            return Some(png);
        }
    }
    if let Some(home) = &wanted.homepage
        && let Ok(base) = Url::parse(home)
        && let Ok(page) = transport.fetch(&wanted.key, home, MAX_PAGE).await
    {
        let html = String::from_utf8_lossy(&page).into_owned();
        // Only on the page's own site: a page naming an icon elsewhere is followed (a CDN is common), still https.
        for link in icon_links(&html, &base).into_iter().take(4) {
            let (fresh, address) = try_one(link.to_string());
            if fresh && let Some(png) = transport.fetch(&wanted.key, &address, MAX_BYTES).await.ok().and_then(|b| normalize(&b)) {
                return Some(png);
            }
        }
    }
    for address in &wanted.last {
        let (fresh, address) = try_one(address.clone());
        if fresh && let Some(png) = transport.fetch(&wanted.key, &address, MAX_BYTES).await.ok().and_then(|b| normalize(&b)) {
            return Some(png);
        }
    }
    None
}

// ───────────────────────── what the window asks ─────────────────────────

enum Slot {
    Loading,
    Ready(PathBuf),
    /// Asked and none came: the monogram stays this session.
    Missing,
}

fn slots() -> &'static Mutex<HashMap<String, Slot>> {
    static SLOTS: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    SLOTS.get_or_init(Mutex::default)
}

/// The icon's file when it is ready; else `None` — and the first time, it is looked for on disk and, if need
/// be, fetched, the window redrawn when it lands.
pub fn icon(wanted: &Wanted, cx: &mut Context<Shell>) -> Option<PathBuf> {
    {
        let mut map = slots().lock().ok()?;
        match map.get(&wanted.key) {
            Some(Slot::Ready(path)) => return Some(path.clone()),
            Some(Slot::Loading | Slot::Missing) => return None,
            None => {
                map.insert(wanted.key.clone(), Slot::Loading);
            }
        }
    }
    let wanted = wanted.clone();
    let key = wanted.key.clone();
    let transport = Transport::of(cx);
    let dir = cache_dir();
    cx.spawn(async move |this, cx| {
        let task = cx.background_spawn(async move {
            let (state, old) = kept(&dir, &wanted.key);
            match state {
                Cached::Fresh => return old,
                Cached::KnownMissing => return None,
                Cached::Fetch => {}
            }
            match fetch_icon(&transport, &wanted).await {
                Some(png) => keep(&dir, &wanted.key, &png).or(old),
                None => {
                    // An old icon is better than none when the site cannot be reached; else remember the miss.
                    if old.is_none() {
                        note_missing(&dir, &wanted.key);
                    }
                    old
                }
            }
        });
        let found = task.await;
        if let Ok(mut map) = slots().lock() {
            map.insert(key, found.map_or(Slot::Missing, Slot::Ready));
        }
        let _ = this.update(cx, |_, cx| cx.notify());
    })
    .detach();
    None
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui_kit's glob brings its own `test` macro over `#[test]`.
    use super::{Cached, Kind, KEEP_SIDE, MAX_BYTES, MAX_SIDE, MISS_TTL, TTL, acceptable, cache_state, file_name, for_catalog, for_server_url, for_service_text, https_ok, icon_links, icon_path, keep, kept, normalize, note_missing, site_of};
    use gpui_kit::http_client::Url;
    use std::io::Cursor;
    use std::time::Duration;

    fn png(side: u32) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(side, side).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn ico(side: u8) -> Vec<u8> {
        // One 32-bit entry holding a PNG, as modern .ico files do.
        let data = png(u32::from(side));
        let mut out = vec![0, 0, 1, 0, 1, 0, side, side, 0, 0, 1, 0, 32, 0];
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(22u32.to_le_bytes());
        out.extend(data);
        out
    }

    #[test]
    fn a_catalog_service_without_a_simple_icons_logo_has_its_own_sites_icon() {
        for id in ["mobbin", "21st", "aws"] {
            let w = for_catalog(id).expect(id);
            assert!(w.urls.iter().all(|u| u.starts_with("https://")), "{id}");
            assert!(w.key.starts_with("catalog-"));
        }
        assert_eq!(for_catalog("mobbin").unwrap().urls, ["https://mobbin.com/favicon.ico"]);
        assert_eq!(for_catalog("aws").unwrap().urls[0], "https://aws.amazon.com/apple-touch-icon.png");
        assert!(for_catalog("github").is_none(), "Simple Icons carries it: nothing is fetched");
        assert!(for_catalog("anthropic").is_none());
    }

    #[test]
    fn a_server_is_asked_at_its_registrable_domain() {
        assert_eq!(site_of("https://mcp.figma.com/mcp").as_deref(), Some("figma.com"));
        assert_eq!(site_of("https://api.mobbin.com/mcp").as_deref(), Some("mobbin.com"));
        assert_eq!(site_of("https://docs.acme.co.uk/mcp").as_deref(), Some("acme.co.uk"));
        assert_eq!(site_of("https://Docs.Acme.Example/mcp").as_deref(), Some("acme.example"));
        assert_eq!(site_of("https://acme.io./x").as_deref(), Some("acme.io"));
        // Not a public site's: no icon is asked for.
        for bad in ["http://acme.io/mcp", "https://localhost/mcp", "https://127.0.0.1/mcp", "https://[::1]/mcp", "https://printer.local/", "https://intranet/", "ftp://acme.io/", "not a url", "https://a.internal/", "https://-bad.io/"] {
            assert_eq!(site_of(bad), None, "{bad}");
        }
        let w = for_server_url("https://docs.acme.example/mcp").unwrap();
        assert_eq!(w.key, "acme.example");
        assert_eq!(w.urls, ["https://acme.example/apple-touch-icon.png"]);
        assert_eq!(w.homepage.as_deref(), Some("https://acme.example/"));
        assert_eq!(w.last, ["https://acme.example/favicon.ico"]);
        assert!(for_server_url("http://docs.acme.example/mcp").is_none());
        // A key's service text that reads as a host.
        assert!(for_service_text("deploy.acme.example").is_some());
        assert!(for_service_text("my token").is_none() && for_service_text("openai").is_none() && for_service_text("a.b/c").is_none());
    }

    #[test]
    fn only_https_without_credentials_may_be_fetched() {
        let ok = |s: &str| https_ok(&Url::parse(s).unwrap());
        assert!(ok("https://acme.io/x.png"));
        assert!(!ok("http://acme.io/x.png") && !ok("ftp://acme.io/x.png") && !ok("file:///etc/passwd") && !ok("data:image/png;base64,AAAA"));
        assert!(!ok("https://user:pw@acme.io/x.png") && !ok("https://user@acme.io/x.png"));
        // A redirect is resolved against where it came from, and judged again.
        let from = Url::parse("https://acme.io/a").unwrap();
        assert!(!https_ok(&from.join("http://acme.io/b").unwrap()));
        assert!(https_ok(&from.join("/b.png").unwrap()));
        assert!(!https_ok(&from.join("//evil.example@acme.io/").unwrap_or(Url::parse("http://x").unwrap())));
    }

    #[test]
    fn only_png_and_ico_within_the_size_are_icons_whatever_the_server_says() {
        assert_eq!(acceptable(&png(32)), Some(Kind::Png));
        assert_eq!(acceptable(&ico(32)), Some(Kind::Ico));
        assert_eq!(acceptable(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), None, "SVG is skipped");
        assert_eq!(acceptable(b"<!doctype html><title>404</title>"), None);
        assert_eq!(acceptable(b"\xFF\xD8\xFF\xE0jpeg"), None);
        assert_eq!(acceptable(b""), None);
        let mut big = png(32);
        big.resize(MAX_BYTES + 1, 0);
        assert_eq!(acceptable(&big), None, "over the limit");
        big.truncate(MAX_BYTES);
        assert_eq!(acceptable(&big), Some(Kind::Png), "at the limit");
    }

    #[test]
    fn an_icon_is_kept_as_a_small_png_or_not_at_all() {
        let small = normalize(&png(32)).expect("a 32px png");
        assert_eq!(image::load_from_memory(&small).unwrap().width(), 32);
        let shrunk = normalize(&png(256)).expect("a 256px png");
        assert_eq!(image::load_from_memory(&shrunk).unwrap().width(), KEEP_SIDE);
        let from_ico = normalize(&ico(48)).expect("an ico");
        assert_eq!(image::load_from_memory(&from_ico).unwrap().width(), 48);
        assert!(normalize(&png(8)).is_none(), "a speck is no icon");
        assert!(normalize(&png(MAX_SIDE + 1)).is_none(), "a huge picture is not decoded");
        // Bytes that start like a PNG and are not one.
        let mut lie = png(32);
        lie.truncate(40);
        assert!(normalize(&lie).is_none());
    }

    #[test]
    fn a_page_names_its_icon_apple_touch_first_https_only_never_svg() {
        let base = Url::parse("https://acme.io/docs/").unwrap();
        let html = r#"<html><head>
            <link rel="stylesheet" href="/a.css">
            <LINK REL="shortcut icon" HREF="/favicon-32.png">
            <link rel='apple-touch-icon' sizes="180x180" href='/touch.png'>
            <link rel="icon" type="image/svg+xml" href="/logo.svg">
            <link rel="icon" href="http://acme.io/insecure.png">
            <link rel="icon" href="https://cdn.acme.io/i.ico">
            <link href=/unquoted.png rel=icon>
            <link rel="icon">
            </head></html>"#;
        let links: Vec<String> = icon_links(html, &base).iter().map(Url::to_string).collect();
        assert_eq!(links, ["https://acme.io/touch.png", "https://acme.io/favicon-32.png", "https://cdn.acme.io/i.ico", "https://acme.io/unquoted.png"]);
        assert!(icon_links("<p>no head</p>", &base).is_empty());
        assert!(icon_links("<link rel=icon href=", &base).is_empty());
    }

    #[test]
    fn the_cache_keeps_an_icon_thirty_days_and_a_miss_one() {
        let day = Duration::from_secs(86_400);
        assert_eq!(cache_state(Some(day), None), Cached::Fresh);
        assert_eq!(cache_state(Some(TTL - Duration::from_secs(1)), Some(day)), Cached::Fresh);
        assert_eq!(cache_state(Some(TTL), None), Cached::Fetch, "stale: ask again");
        assert_eq!(cache_state(Some(TTL + day), Some(Duration::ZERO)), Cached::Fetch, "a stale icon is asked for again, whatever a miss said");
        assert_eq!(cache_state(None, Some(day / 2)), Cached::KnownMissing);
        assert_eq!(cache_state(None, Some(MISS_TTL)), Cached::Fetch);
        assert_eq!(cache_state(None, None), Cached::Fetch);
    }

    #[test]
    fn a_kept_icon_is_found_again_and_a_miss_is_remembered() {
        let dir = std::env::temp_dir().join(format!("equipe-siteicon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(kept(&dir, "acme.io"), (Cached::Fetch, None));
        note_missing(&dir, "acme.io");
        assert_eq!(kept(&dir, "acme.io"), (Cached::KnownMissing, None));
        let path = keep(&dir, "acme.io", &png(32)).expect("kept");
        assert!(path.is_file());
        assert_eq!(kept(&dir, "acme.io"), (Cached::Fresh, Some(path)), "the miss note went with it");
        // A key can never leave the folder.
        assert_eq!(file_name("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(icon_path(&dir, "../x").parent(), Some(dir.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
