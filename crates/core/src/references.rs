//! `show_references`: a member shows the person real components, screens and
//! patterns right in the conversation — found in the design libraries the
//! squad connected (21st.dev, Mobbin, the shadcn registry) or saved in its
//! worktree — as a gallery card they open big, react to and keep.
//!
//! - **The tool** ([`tool`], [`parse_args`]): 1 to [`MOST`] items, each a
//!   picture (an https address a library's result gave, or a path where the
//!   member works), a title, where it is from (site and link), its kind, and
//!   optional one-liners — why it fits, what to take, what to leave — and the
//!   pattern folder it belongs in.
//! - **The pictures** ([`fetch`], [`store`]): the daemon fetches each itself —
//!   https only, no credentials in the address, only public addresses (every
//!   hop of a redirect checked again, the address it resolved to pinned), at
//!   most [`FETCH_MAX`] bytes, a PNG, JPEG, WebP or GIF by its bytes — and
//!   keeps it in the squad's picture store, `<squad>/visuals/refs/`, by the
//!   hash of its bytes: outside every repository, given to any screen
//!   following the squad by `read_image` ([`is_stored`]), and brought back
//!   within [`FILES_MAX`] files and [`BYTES_MAX`] bytes ([`prune`]).
//! - **The card** ([`note_body`], [`parse_body`]): one note of the
//!   conversation, in Markdown a screen that knows nothing of cards still
//!   reads (docs/VISUALS.md, *Reference cards*).
//! - **Keeping one** ([`kept_entry`]): an entry in the project's
//!   `design/references/<pattern>/notes.md` — the link and the lines, never
//!   the picture, which is another product's.

use std::collections::HashSet;
use std::io::Read as _;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use crate::protocol::references::{RefCard, RefKind};

/// The tool's name.
pub const TOOL: &str = "show_references";
/// The most items one call shows.
pub const MOST: usize = 12;
/// The longest title, site and pattern, and the longest one-liner.
pub const TITLE_CHARS: usize = 90;
pub const SITE_CHARS: usize = 40;
pub const LINE_CHARS: usize = 200;
pub const PATTERN_CHARS: usize = 40;
/// The longest address taken.
pub const URL_CHARS: usize = 2000;
/// The biggest picture fetched or copied.
pub const FETCH_MAX: u64 = 8 << 20;
/// The most pixels on a side a picture may have.
pub const SIDE_MAX: u32 = 16_384;
/// Redirects followed, at most.
pub const REDIRECTS: usize = 3;
/// How long one picture may take, and all of a call's.
pub const ONE_WAIT: Duration = Duration::from_secs(15);
pub const ALL_WAIT: Duration = Duration::from_secs(40);
/// What the store keeps, at most: past either, the pictures used longest ago go.
pub const FILES_MAX: usize = 1000;
pub const BYTES_MAX: u64 = 256 << 20;

/// Whether a member of `role` is given the tool: the PM (for the person
/// talking at its desk), the designer and frontend.
pub fn offered(role: &str) -> bool {
    matches!(crate::roles::canonical(role), crate::roles::PM | "designer" | crate::roles::ENGINEER)
}

/// The tool's definition, as `tools/list` gives it.
pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "Show the person reference pictures right in this conversation, as a gallery card they open big, react to \
and keep: real components, screens or patterns. When the person asks to see references, components, screens or examples, search \
the design libraries you have (21st.dev, Mobbin, the shadcn registry) and call this in the same turn with the best 4 to 8 — never \
describe them in words instead, and never ask them to open a folder or a site. Give each its picture (the https address the \
library's result gave, or a path in your worktree), its title, where it is from (`site`, `link`) and its `kind`; add a line on \
why it fits and what to take and leave. equipe fetches and keeps the pictures itself, outside every repository. Mobbin's \
pictures are private study material: shown here, never shipped, committed or copied into a project. Then say in a line or two \
what to look at; the card shows the rest. Whatever the person should look at or try goes in front of them — this card (its \
Try it), `equipe design session show <url|file>`, or `show_my_browser` — never only opened in your own browser, which they \
can't see; and say what to click by the names they see on screen, never by internal numbers.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MOST,
                    "items": {
                        "type": "object",
                        "properties": {
                            "picture": { "type": "string", "description": "The picture: an https address from the library's result (no sign-in or key in it), or a PNG, JPG, WebP or GIF you saved, by its path in your worktree." },
                            "title": { "type": "string", "description": "What it is, in a few words: `Command menu`, `Linear — Issues`." },
                            "site": { "type": "string", "description": "Where it is from, as a person names it: `21st.dev`, `Mobbin`, `shadcn/ui`. Default: the link's site." },
                            "link": { "type": "string", "description": "Its page there: an https address the person can open." },
                            "kind": { "type": "string", "enum": RefKind::ALL.map(RefKind::key), "description": "component (one part), screen (a whole screen) or pattern (across screens). Default: component." },
                            "why": { "type": "string", "description": "One line: why it fits what the person is making." },
                            "take": { "type": "string", "description": "One line: what to take from it." },
                            "leave": { "type": "string", "description": "One line: what to leave." },
                            "pattern": { "type": "string", "description": "The pattern folder it belongs in among the project's references (`command-menu`, `pricing-table`). Default: made from its title." },
                        },
                        "required": ["picture", "title", "link"],
                    },
                },
            },
            "required": ["items"],
        },
    })
}

/// Where an item's picture is to come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picture {
    /// An https address: fetched by the daemon.
    Remote(String),
    /// A path where the member works (or the squad's picture store): copied into the store.
    Local(String),
}

/// One item as the member asked for it, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub picture: Picture,
    /// The card without its picture yet.
    pub card: RefCard,
}

/// `text` on one line: whitespace collapsed, at most `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        return line;
    }
    let cut: String = line.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// A title, site or pattern: one line, without what would break the card's
/// Markdown (`*`, brackets, back-quotes, angle brackets, bars).
fn plain(text: &str, max: usize) -> String {
    let cleaned: String = text.chars().map(|c| if matches!(c, '*' | '[' | ']' | '`' | '<' | '>' | '|' | '\\' | '\u{b7}') { ' ' } else { c }).collect();
    one_line(&cleaned, max)
}

/// A pattern folder's name: lowercase words joined by `-`.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(PATTERN_CHARS).collect();
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() { "references".to_owned() } else { out }
}

/// `url` when it is an address the daemon may fetch or a person may open:
/// https, no credentials, port or IP, a site's name (as
/// [`crate::connections::clean_url`] has it, but long enough for a CDN's
/// signed picture address). `(` and `)` are escaped so it sits in Markdown whole.
pub fn check_link(url: &str) -> Result<String, String> {
    let url = url.trim();
    let refuse = |why: &str| Err(format!("{url:?}: {why}"));
    if url.len() > URL_CHARS {
        return refuse(&format!("an address of at most {URL_CHARS} characters"));
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '<' | '>' | '\\' | '`')) {
        return refuse("not an address");
    }
    let Some(rest) = url.get(..8).filter(|scheme| scheme.eq_ignore_ascii_case("https://")).map(|_| &url[8..]) else {
        return refuse("only https addresses");
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    if authority.contains(['@', ':']) {
        return refuse("no sign-in, key or port in the address");
    }
    let labels: Vec<&str> = authority.split('.').collect();
    let host_ok = labels.len() >= 2 && labels.iter().all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    let last = labels.last().copied().unwrap_or_default();
    if !host_ok || last.chars().all(|c| c.is_ascii_digit()) || matches!(last, "localhost" | "local" | "internal" | "lan" | "home" | "arpa") {
        return refuse("not a public site's address");
    }
    Ok(format!("https://{authority}{}", &rest[rest.find(['/', '?', '#']).unwrap_or(rest.len())..]).replace('(', "%28").replace(')', "%29"))
}

/// The host of a checked address.
fn host_of(url: &str) -> String {
    url.strip_prefix("https://").unwrap_or(url).split(['/', '?', '#']).next().unwrap_or_default().to_ascii_lowercase()
}

/// A site's name from its address: the host without `www.`.
fn site_of(url: &str) -> String {
    let host = host_of(url);
    host.strip_prefix("www.").map(str::to_owned).unwrap_or(host)
}

/// The items of a `show_references` call, checked: each with its picture,
/// title and link; what is wrong said, item by item, so the member can call again.
pub fn parse_args(args: &Value) -> Result<Vec<Asked>, String> {
    let items = match args.get("items") {
        Some(Value::Array(items)) => items.as_slice(),
        Some(one @ Value::Object(_)) => std::slice::from_ref(one),
        _ => return Err("Give `items`: 1 to 12 references, each with its `picture`, `title` and `link`.".into()),
    };
    if items.is_empty() {
        return Err("Give at least one reference in `items`.".into());
    }
    if items.len() > MOST {
        return Err(format!("At most {MOST} references a call, not {}: show the best ones.", items.len()));
    }
    let mut out: Vec<Asked> = Vec::new();
    for (n, item) in items.iter().enumerate() {
        let n = n + 1;
        let field = |key: &str| item.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default();
        let title = plain(field("title"), TITLE_CHARS);
        if title.is_empty() {
            return Err(format!("Item {n}: give its `title`."));
        }
        let link = match field("link") {
            "" => return Err(format!("Item {n} ({title}): give its `link`, the page it is from.")),
            link => check_link(link).map_err(|why| format!("Item {n} ({title}): its `link`: {why}."))?,
        };
        let picture = match field("picture") {
            "" => return Err(format!("Item {n} ({title}): give its `picture`.")),
            p if p.to_ascii_lowercase().starts_with("http://") => return Err(format!("Item {n} ({title}): its `picture` must be an https address.")),
            p if p.to_ascii_lowercase().starts_with("https://") => Picture::Remote(check_link(p).map_err(|why| format!("Item {n} ({title}): its `picture`: {why}."))?),
            p if p.contains("://") || p.starts_with("data:") => return Err(format!("Item {n} ({title}): its `picture` is an https address or a path where you work.")),
            p => Picture::Local(p.to_owned()),
        };
        let kind = match field("kind") {
            "" => RefKind::Component,
            k => RefKind::parse(k).ok_or_else(|| format!("Item {n} ({title}): `kind` is component, screen or pattern, not {k:?}."))?,
        };
        let site = match plain(field("site"), SITE_CHARS) {
            s if s.is_empty() => site_of(&link),
            s => s,
        };
        let pattern = match field("pattern") {
            "" => slug(&title),
            p => slug(p),
        };
        let card = RefCard { picture: String::new(), title, site, link, kind, why: one_line(field("why"), LINE_CHARS), take: one_line(field("take"), LINE_CHARS), leave: one_line(field("leave"), LINE_CHARS), pattern };
        if out.iter().any(|a| a.picture == picture) {
            continue;
        }
        out.push(Asked { picture, card });
    }
    Ok(out)
}

// ───────────────────────── the pictures ─────────────────────────

/// Whether `ip` is an address on the public internet: not this machine, not
/// a private, shared, link-local, multicast or reserved one.
pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_documentation()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0)
                || (a == 198 && (18..20).contains(&b))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 || (first == 0x2001 && v6.segments()[1] == 0x0db8))
        }
    }
}

/// What the bytes of a fetched or copied picture are: its type, the
/// extension it is kept under, and its size — a PNG, JPEG, WebP or GIF by
/// its first bytes, decodable, no bigger than [`SIDE_MAX`] on a side. An SVG
/// never is: it could reach past itself when drawn.
pub fn accept(bytes: &[u8]) -> Result<(&'static str, &'static str, (u32, u32)), String> {
    if bytes.len() as u64 > FETCH_MAX {
        return Err(format!("bigger than {} MB", FETCH_MAX >> 20));
    }
    let (mime, ext) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ("image/png", "png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ("image/jpeg", "jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        ("image/gif", "gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ("image/webp", "webp")
    } else {
        return Err("not a PNG, JPEG, WebP or GIF picture".into());
    };
    let dims = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok().and_then(|r| r.into_dimensions().ok()).ok_or("a picture that can't be read")?;
    if dims.0 == 0 || dims.1 == 0 || dims.0 > SIDE_MAX || dims.1 > SIDE_MAX {
        return Err(format!("{}×{} pixels: at most {SIDE_MAX} on a side", dims.0, dims.1));
    }
    Ok((mime, ext, dims))
}

/// What one request came back with.
#[derive(Debug, PartialEq, Eq)]
pub enum Got {
    /// The picture's bytes.
    Body(Vec<u8>),
    /// Go there instead (as the response said it, relative or not).
    Redirect(String),
}

/// One GET of `url` with `client`: its body, at most [`FETCH_MAX`] bytes
/// (refused at once when it says it is bigger), or where it redirects.
pub async fn get(client: &reqwest::Client, url: &str) -> Result<Got, String> {
    let mut response = client.get(url).header("accept", "image/png,image/jpeg,image/webp,image/gif;q=0.9,*/*;q=0.1").send().await.map_err(|e| if e.is_timeout() { "it took too long".to_owned() } else { "it couldn't be reached".to_owned() })?;
    let status = response.status().as_u16();
    if (300..400).contains(&status) {
        let to = response.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_owned).ok_or("a redirect to nowhere")?;
        return Ok(Got::Redirect(to));
    }
    if !(200..300).contains(&status) {
        return Err(format!("its site answered {status}"));
    }
    if response.content_length().is_some_and(|n| n > FETCH_MAX) {
        return Err(format!("bigger than {} MB", FETCH_MAX >> 20));
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "the download broke off".to_owned())? {
        if (body.len() + chunk.len()) as u64 > FETCH_MAX {
            return Err(format!("bigger than {} MB", FETCH_MAX >> 20));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Got::Body(body))
}

/// Where a redirect from `from` to `to` goes, as an absolute address.
fn joined(from: &str, to: &str) -> String {
    let to = to.trim();
    if to.contains("://") {
        return to.to_owned();
    }
    let origin = format!("https://{}", host_of(from));
    if to.starts_with("//") {
        return format!("https:{to}");
    }
    if to.starts_with('/') {
        return format!("{origin}{to}");
    }
    let base = from.split(['?', '#']).next().unwrap_or(from);
    match base.rfind('/') {
        Some(at) if at >= "https://".len() => format!("{}/{to}", &base[..at]),
        _ => format!("{origin}/{to}"),
    }
}

/// A public address `host` resolves to, on port 443; refused when it has none.
async fn public_addr(host: &str) -> Result<SocketAddr, String> {
    let found = tokio::time::timeout(Duration::from_secs(8), tokio::net::lookup_host((host, 443))).await.map_err(|_| format!("{host} didn't resolve in time"))?.map_err(|_| format!("{host} doesn't resolve"))?;
    let all: Vec<SocketAddr> = found.collect();
    if all.iter().any(|a| !public_ip(a.ip())) {
        return Err(format!("{host} points inside a private network"));
    }
    all.into_iter().next().ok_or_else(|| format!("{host} doesn't resolve"))
}

/// Fetch the picture at `url` (checked by [`check_link`]): each hop's host
/// resolved and refused unless public, that address pinned for the request,
/// redirects followed by hand at most [`REDIRECTS`] times, each checked again;
/// the bytes then checked by [`accept`].
pub async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let bytes = fetch_public(url).await?;
    accept(&bytes)?;
    Ok(bytes)
}

/// The body at `url`, under [`fetch`]'s rules (https, public addresses only,
/// pinned, redirects checked by hand, at most [`FETCH_MAX`] bytes), whatever
/// it is: the caller checks what it holds.
pub async fn fetch_public(url: &str) -> Result<Vec<u8>, String> {
    let mut url = check_link(url)?;
    for _ in 0..=REDIRECTS {
        let host = host_of(&url);
        let addr = public_addr(&host).await?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .resolve(&host, addr)
            .connect_timeout(Duration::from_secs(8))
            .timeout(ONE_WAIT)
            .user_agent(concat!("equipe/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        match get(&client, &url).await? {
            Got::Body(bytes) => return Ok(bytes),
            Got::Redirect(to) => url = check_link(&joined(&url, &to)).map_err(|why| format!("it redirects elsewhere: {why}"))?,
        }
    }
    Err(format!("more than {REDIRECTS} redirects"))
}

/// Read a picture a member saved (`real`, already checked to be where it
/// works): at most [`FETCH_MAX`] bytes, then [`accept`]ed.
pub fn read_local(real: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(real).map_err(|_| "no such file".to_owned())?;
    let mut bytes = Vec::new();
    file.take(FETCH_MAX + 1).read_to_end(&mut bytes).map_err(|_| "couldn't be read".to_owned())?;
    accept(&bytes)?;
    Ok(bytes)
}

/// The squad's picture store: in its folder of the daemon's, beside the
/// diagrams drawn for it — never in a repository.
pub fn store_dir(home: &Path, squad: &str) -> PathBuf {
    crate::visuals::diagram::dir(home, squad).join("refs")
}

/// A stored picture's name: 16 lowercase hex characters and a raster extension.
fn stored_name(name: &str) -> bool {
    let Some((stem, ext)) = name.split_once('.') else { return false };
    stem.len() == 16 && stem.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) && matches!(ext, "png" | "jpg" | "gif" | "webp")
}

/// Whether `path` is one of the squad's stored reference pictures, by its
/// name alone: right in its store, named as the store names them.
pub fn is_stored(home: &Path, squad: &str, path: &Path) -> bool {
    crate::squads::valid_name(squad) && path.file_name().and_then(|n| n.to_str()).is_some_and(stored_name) && path.parent() == Some(store_dir(home, squad).as_path())
}

/// Keep `bytes` (already [`accept`]ed) in the squad's store, by their hash:
/// the path a screen asks for it by. The same picture twice is one file.
pub fn store(home: &Path, squad: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let (_, ext, _) = accept(bytes)?;
    let dir = store_dir(home, squad);
    std::fs::create_dir_all(&dir).map_err(|e| format!("the picture store couldn't be made: {e}"))?;
    let path = dir.join(format!("{}.{ext}", &hex(bytes)[..16]));
    if path.is_file() {
        crate::visuals::cache::touch(&path);
        return Ok(path);
    }
    let part = path.with_extension("part");
    std::fs::write(&part, bytes).map_err(|e| format!("the picture couldn't be kept: {e}"))?;
    std::fs::rename(&part, &path).map_err(|e| format!("the picture couldn't be kept: {e}"))?;
    Ok(path)
}

/// The bytes' SHA-256, in hex: what a stored picture's name is made of.
fn hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Bring the store within `files` files and `bytes` bytes: the pictures used
/// longest ago go first, never one of `keep`. A `.part` older than a minute
/// goes too; files not named as the store names them are left alone.
pub fn prune(dir: &Path, keep: &HashSet<String>, files: usize, bytes: u64) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut kept: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
        let at = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if name.ends_with(".part") {
            if at.elapsed().is_ok_and(|age| age > Duration::from_secs(60)) {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        if meta.is_file() && stored_name(&name) {
            kept.push((at, meta.len(), path));
        }
    }
    kept.sort();
    let (mut count, mut total) = (kept.len(), kept.iter().map(|k| k.1).sum::<u64>());
    for (_, size, path) in kept {
        if count <= files && total <= bytes {
            break;
        }
        if keep.contains(path.to_string_lossy().as_ref()) {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            count -= 1;
            total = total.saturating_sub(size);
        }
    }
}

// ───────────────────────── the card ─────────────────────────

/// The marks of a card's lines after its title: why it fits, take, leave.
const WHY: &str = "> ";
const TAKE: &str = "- \u{2713} ";
const LEAVE: &str = "- \u{2717} ";

/// A picture's destination in Markdown: in `<…>` when it holds a space.
fn destination(path: &str) -> String {
    if path.contains(char::is_whitespace) { format!("<{path}>") } else { path.to_owned() }
}

/// The cards as the note's body (its first line, the head, is the
/// runtime's, in the person's language): per card its picture, its title
/// line — title, kind, source linked, pattern — and its one-liners.
pub fn note_body(cards: &[RefCard]) -> String {
    let mut out: Vec<String> = Vec::new();
    for card in cards {
        let mut lines = vec![format!("![{}]({})", card.title, destination(&card.picture))];
        let mut head = format!("**{}** \u{b7} {} \u{b7} [{}]({})", card.title, card.kind.key(), card.site, card.link);
        if !card.pattern.is_empty() {
            head.push_str(&format!(" \u{b7} `{}`", card.pattern));
        }
        lines.push(head);
        for (mark, text) in [(WHY, &card.why), (TAKE, &card.take), (LEAVE, &card.leave)] {
            if !text.is_empty() {
                lines.push(format!("{mark}{text}"));
            }
        }
        out.push(lines.join("\n"));
    }
    out.join("\n\n")
}

/// `![alt](dest)` on one line: the alt and the destination.
fn image_line(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix("![")?;
    let (alt, rest) = rest.split_once("](")?;
    let dest = rest.strip_suffix(')')?;
    let dest = dest.trim().trim_start_matches('<').trim_end_matches('>').trim();
    (!dest.is_empty()).then(|| (alt.trim().to_owned(), dest.to_owned()))
}

/// A card's title line: `**title** · kind · [site](link) · \`pattern\``.
fn title_line(line: &str, card: &mut RefCard) -> bool {
    let Some(rest) = line.trim().strip_prefix("**") else { return false };
    let Some((title, rest)) = rest.split_once("**") else { return false };
    card.title = title.trim().to_owned();
    for part in rest.split('\u{b7}').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(inner) = part.strip_prefix('[')
            && let Some((site, rest)) = inner.split_once("](")
        {
            card.site = site.trim().to_owned();
            card.link = rest.trim_end_matches(')').trim().to_owned();
        } else if part.len() > 2 && part.starts_with('`') && part.ends_with('`') {
            card.pattern = part.trim_matches('`').trim().to_owned();
        } else if let Some(kind) = RefKind::parse(part) {
            card.kind = kind;
        }
    }
    true
}

/// The cards of a note's body ([`note_body`]): a card per picture line, its
/// title line and one-liners after it; a picture without a title line is
/// called by its alt. Lines it doesn't know are passed over.
pub fn parse_body(text: &str) -> Vec<RefCard> {
    let mut cards: Vec<RefCard> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_end();
        if let Some((alt, dest)) = image_line(trimmed) {
            cards.push(RefCard { picture: dest, title: alt, ..RefCard::default() });
            continue;
        }
        let Some(card) = cards.last_mut() else { continue };
        if trimmed.trim().is_empty() {
            continue;
        }
        if title_line(trimmed, card) {
            continue;
        }
        if let Some(why) = trimmed.strip_prefix(WHY) {
            card.why = why.trim().to_owned();
        } else if let Some(take) = trimmed.strip_prefix(TAKE) {
            card.take = take.trim().to_owned();
        } else if let Some(leave) = trimmed.strip_prefix(LEAVE) {
            card.leave = leave.trim().to_owned();
        }
    }
    cards.retain(|c| !c.title.is_empty() && !c.picture.is_empty());
    cards
}

/// A gallery's key: [`crate::visuals::diagram::key`] of its note, as a screen
/// makes it of the note it shows — what the person's reactions are kept by.
pub fn key(note: &str) -> String {
    crate::visuals::diagram::key(note)
}

// ───────────────────────── keeping one ─────────────────────────

/// The notes file a kept card goes in, in design folder `dir`.
pub fn notes_path(dir: &Path, card: &RefCard) -> PathBuf {
    let pattern = if card.pattern.trim().is_empty() { slug(&card.title) } else { slug(&card.pattern) };
    dir.join(crate::design::refs::DIR).join(pattern).join(crate::design::refgallery::NOTES)
}

/// The entry a kept card adds to its pattern's `notes.md`, in the form the
/// references gallery reads (`## <product> — <screen>`, then Source / Take /
/// Leave / Fit): its link, never its picture.
pub fn kept_entry(card: &RefCard, today: &str) -> String {
    let mut lines = vec![format!("## {} \u{2014} {}", plain(&card.site, SITE_CHARS), plain(&card.title, TITLE_CHARS)), format!("- Source: {} ({}), {today}", card.link, plain(&card.site, SITE_CHARS))];
    for (label, text) in [("Take", &card.take), ("Leave", &card.leave), ("Fit", &card.why)] {
        let text = one_line(text, LINE_CHARS);
        if !text.is_empty() {
            lines.push(format!("- {label}: {text}"));
        }
    }
    lines.push(if card.study_only() {
        "- Picture: not kept — the source's own, private study material: never shipped or committed".to_owned()
    } else {
        "- Picture: not kept — another product's: see the source".to_owned()
    });
    lines.join("\n")
}

/// `previous` (the notes file as it is, or none) with `card`'s entry added;
/// `None` when it already holds the card's link.
pub fn with_kept(previous: Option<&str>, card: &RefCard, today: &str) -> Option<String> {
    let entry = kept_entry(card, today);
    match previous.map(str::trim_end).filter(|p| !p.trim().is_empty()) {
        Some(p) if p.contains(&card.link) => None,
        Some(p) => Some(format!("{p}\n\n{entry}\n")),
        None => Some(format!("# {}\n\n{entry}\n", if card.pattern.is_empty() { slug(&card.title) } else { slug(&card.pattern) })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255])).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn a_call_gives_one_to_twelve_items_each_with_a_picture_a_title_and_a_link() {
        let ok = parse_args(&json!({ "items": [
            { "picture": "https://cdn.21st.dev/a.png", "title": "  Command   menu ", "link": "https://21st.dev/r/cmdk", "kind": "Component", "why": "fast\nkeyboard", "take": "grouped rows", "leave": "neon" },
            { "picture": "shots/pricing.png", "title": "Pricing [table]", "link": "https://www.mobbin.com/screens/1", "kind": "screen", "site": "Mobbin" },
            { "picture": "https://cdn.21st.dev/a.png", "title": "the same picture again", "link": "https://21st.dev/r/x" },
        ] }))
        .unwrap();
        assert_eq!(ok.len(), 2, "a picture shown once");
        assert_eq!(ok[0].picture, Picture::Remote("https://cdn.21st.dev/a.png".into()));
        assert_eq!((ok[0].card.title.as_str(), ok[0].card.site.as_str(), ok[0].card.why.as_str(), ok[0].card.pattern.as_str()), ("Command menu", "21st.dev", "fast keyboard", "command-menu"));
        assert_eq!((ok[1].picture.clone(), ok[1].card.kind, ok[1].card.title.as_str()), (Picture::Local("shots/pricing.png".into()), RefKind::Screen, "Pricing table"));
        let bad = |item: Value| parse_args(&json!({ "items": [item] })).unwrap_err();
        assert!(bad(json!({ "picture": "https://a.example.com/x.png", "link": "https://a.example.com" })).contains("title"));
        assert!(bad(json!({ "picture": "https://a.example.com/x.png", "title": "A" })).contains("link"));
        assert!(bad(json!({ "picture": "http://a.example.com/x.png", "title": "A", "link": "https://a.example.com" })).contains("https"));
        assert!(bad(json!({ "picture": "https://user:pw@a.example.com/x.png", "title": "A", "link": "https://a.example.com" })).contains("picture"));
        assert!(bad(json!({ "picture": "https://127.0.0.1/x.png", "title": "A", "link": "https://a.example.com" })).contains("picture"));
        assert!(bad(json!({ "picture": "https://a.example.com:8443/x.png", "title": "A", "link": "https://a.example.com" })).contains("picture"));
        assert!(bad(json!({ "picture": "data:image/png;base64,AAAA", "title": "A", "link": "https://a.example.com" })).contains("picture"));
        assert!(bad(json!({ "picture": "x.png", "title": "A", "link": "https://a.example.com", "kind": "widget" })).contains("kind"));
        assert!(parse_args(&json!({ "items": [] })).is_err());
        assert!(parse_args(&json!({})).is_err());
        let many: Vec<Value> = (0..13).map(|i| json!({ "picture": format!("p{i}.png"), "title": "A", "link": "https://a.example.com" })).collect();
        assert!(parse_args(&json!({ "items": many })).unwrap_err().contains("At most 12"));
        let long = "x".repeat(400);
        let cut = parse_args(&json!({ "items": [{ "picture": "a.png", "title": long, "link": "https://a.example.com", "why": long }] })).unwrap();
        assert!(cut[0].card.title.chars().count() <= TITLE_CHARS && cut[0].card.why.chars().count() <= LINE_CHARS);
    }

    #[test]
    fn the_tool_is_the_pm_designer_and_frontend_s_and_says_to_show_in_the_same_turn() {
        assert!(offered("pm") && offered("designer") && offered("frontend"));
        assert!(offered("engineer") && offered("backend"), "the software engineer builds screens too");
        assert!(!offered("qa") && !offered("security"));
        let tool = tool();
        let doc = tool["description"].as_str().unwrap();
        for part in ["same turn", "Mobbin's pictures are private study material", "never shipped, committed", "never only opened in your own browser", "never by internal numbers"] {
            assert!(doc.contains(part), "{part}");
        }
        assert_eq!(tool["inputSchema"]["properties"]["items"]["maxItems"], MOST);
        assert_eq!(tool["inputSchema"]["properties"]["items"]["items"]["required"], json!(["picture", "title", "link"]));
    }

    #[test]
    fn only_public_addresses_are_fetched_from() {
        for private in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fc00::1", "fe80::1", "::ffff:10.0.0.1", "240.0.0.1"] {
            assert!(!public_ip(private.parse().unwrap()), "{private}");
        }
        for public in ["1.1.1.1", "151.101.1.69", "2606:4700::1111"] {
            assert!(public_ip(public.parse().unwrap()), "{public}");
        }
        assert_eq!(joined("https://a.example.com/x/y.png?s=1", "/z.png"), "https://a.example.com/z.png");
        assert_eq!(joined("https://a.example.com/x/y.png", "z.png"), "https://a.example.com/x/z.png");
        assert_eq!(joined("https://a.example.com/x", "//cdn.example.com/q.png"), "https://cdn.example.com/q.png");
        assert!(check_link(&joined("https://a.example.com/x", "http://cdn.example.com/q.png")).is_err(), "a redirect to plain http is refused");
        assert_eq!(check_link("https://a.example.com/a_(b).png").unwrap(), "https://a.example.com/a_%28b%29.png");
        // A CDN's signed address is long; its host is lowercased, its path kept.
        let signed = format!("https://CDN.Example.com/i.png?sig={}", "a".repeat(1500));
        assert_eq!(check_link(&signed).unwrap(), signed.replace("CDN.Example.com", "cdn.example.com"));
        for bad in ["https://printer.local/x.png", "https://router.home/x", "https://10.0.0.1/x", "https://a.example.com:443/x", "ftp://a.example.com/x", "https://a.example.com/x y"] {
            assert!(check_link(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_picture_is_taken_by_its_bytes_and_its_size() {
        let (mime, ext, dims) = accept(&png(40, 30)).unwrap();
        assert_eq!((mime, ext, dims), ("image/png", "png", (40, 30)));
        assert!(accept(b"<svg xmlns='http://www.w3.org/2000/svg'></svg>").unwrap_err().contains("not a PNG"));
        assert!(accept(b"\x89PNG\r\n\x1a\nnot really").is_err(), "a PNG's first bytes alone aren't a picture");
        assert!(accept(&vec![0u8; FETCH_MAX as usize + 1]).unwrap_err().contains("bigger than"));
    }

    /// A one-shot HTTP server on loopback answering every request with `reply`.
    async fn serve(reply: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let reply = reply.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = socket.read(&mut buf).await;
                    let _ = socket.write_all(&reply).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        format!("http://{addr}/p.png")
    }

    fn response(head: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!("HTTP/1.1 {head}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", body.len()).into_bytes();
        out.extend_from_slice(body);
        out
    }

    #[tokio::test]
    async fn a_download_is_capped_and_a_redirect_is_handed_back_to_be_checked() {
        let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        let picture = png(8, 8);
        let url = serve(response("200 OK", &picture)).await;
        assert_eq!(get(&client, &url).await.unwrap(), Got::Body(picture));
        let url = serve(b"HTTP/1.1 302 Found\r\nlocation: http://10.0.0.1/x.png\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec()).await;
        assert_eq!(get(&client, &url).await.unwrap(), Got::Redirect("http://10.0.0.1/x.png".into()), "not followed: checked first");
        let url = serve(format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", FETCH_MAX + 1).into_bytes()).await;
        assert!(get(&client, &url).await.unwrap_err().contains("bigger than"), "refused by what it says it is");
        let url = serve(response("404 Not Found", b"")).await;
        assert!(get(&client, &url).await.unwrap_err().contains("404"));
        // The daemon never fetches from this machine, whatever the address says.
        assert!(fetch("https://localhost/x.png").await.is_err());
    }

    #[test]
    fn pictures_are_kept_by_their_hash_outside_any_repository_and_served_by_name() {
        let home = std::env::temp_dir().join(format!("equipe-refcards-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let path = store(&home, "acme", &png(4, 4)).unwrap();
        assert_eq!(path.parent().unwrap(), store_dir(&home, "acme"));
        assert!(path.starts_with(crate::visuals::diagram::dir(&home, "acme")), "beside the squad's drawn diagrams");
        assert!(!path.starts_with(crate::squad_home::path(&home, "acme")), "never in the squad's folder, a git repository");
        assert_eq!(store(&home, "acme", &png(4, 4)).unwrap(), path, "the same picture is one file");
        assert!(is_stored(&home, "acme", &path));
        assert!(!is_stored(&home, "other", &path), "another squad's isn't");
        assert!(!is_stored(&home, "acme", &store_dir(&home, "acme").join("notes.png")));
        assert!(!is_stored(&home, "acme", &store_dir(&home, "acme").join("sub").join(path.file_name().unwrap())));
        assert!(store(&home, "acme", b"<svg/>").is_err());
        // Over quota: the oldest go, never one kept.
        let b = store(&home, "acme", &png(5, 5)).unwrap();
        let c = store(&home, "acme", &png(6, 6)).unwrap();
        let old = SystemTime::now() - Duration::from_secs(3600);
        for p in [&path, &b] {
            std::fs::File::open(p).unwrap().set_modified(old).unwrap();
        }
        std::fs::write(store_dir(&home, "acme").join("mine.txt"), "not ours").unwrap();
        let keep: HashSet<String> = [path.to_string_lossy().into_owned()].into();
        prune(&store_dir(&home, "acme"), &keep, 2, u64::MAX);
        assert!(path.exists() && !b.exists() && c.exists());
        assert!(store_dir(&home, "acme").join("mine.txt").exists(), "what isn't ours is left alone");
        // A screen gets it through `read_image`, by its path alone.
        let read = crate::visuals::serve(&home, &crate::protocol::rpc::ReadImage { squad: "acme".into(), path: path.to_string_lossy().into_owned(), max: None }, None).unwrap();
        assert_eq!(read.mime, "image/png");
        let _ = std::fs::remove_dir_all(&home);
    }

    fn card(n: usize) -> RefCard {
        RefCard {
            picture: format!("/Users/a b/.equipe/squads/acme/visuals/refs/{n:016x}.png"),
            title: format!("Card {n}"),
            site: "21st.dev".into(),
            link: format!("https://21st.dev/r/{n}"),
            kind: if n.is_multiple_of(2) { RefKind::Component } else { RefKind::Screen },
            why: "fits the dense list".into(),
            take: "36px rows".into(),
            leave: if n == 1 { String::new() } else { "the neon".into() },
            pattern: "command-menu".into(),
        }
    }

    #[test]
    fn a_gallery_reads_back_from_its_note_as_it_was_written() {
        let cards: Vec<RefCard> = (0..3).map(card).collect();
        let body = note_body(&cards);
        assert!(body.contains("![Card 0](</Users/a b/.equipe/squads/acme/visuals/refs/0000000000000000.png>)"), "{body}");
        assert!(body.contains("**Card 1** \u{b7} screen \u{b7} [21st.dev](https://21st.dev/r/1) \u{b7} `command-menu`"), "{body}");
        assert_eq!(parse_body(&body), cards);
        let note = format!("Designer is showing you 3 references\n{body}");
        assert_eq!(parse_body(&note), cards, "the head is passed over");
        assert_eq!(key(&note), key(&format!("{note}\n")), "the key is the trimmed note's");
        // A picture with no title line is called by its alt; words around are passed over.
        let loose = parse_body("intro\n![Alt only](/x/y.png)\nsomething else\n\n![](/x/z.png)");
        assert_eq!(loose.len(), 1);
        assert_eq!((loose[0].title.as_str(), loose[0].kind), ("Alt only", RefKind::Component));
        // What the member wrote can't break the card.
        let tricky = parse_args(&json!({ "items": [{ "picture": "a.png", "title": "**Bold** [x](y) `z`", "link": "https://a.example.com/q", "why": "line\n![evil](/etc/passwd)" }] })).unwrap();
        let mut c = tricky[0].card.clone();
        c.picture = "/s/visuals/refs/0123456789abcdef.png".into();
        let parsed = parse_body(&note_body(std::slice::from_ref(&c)));
        assert_eq!(parsed.len(), 1, "{parsed:?}");
        assert_eq!(parsed[0].picture, c.picture);
    }

    #[test]
    fn a_kept_card_is_its_link_and_lines_in_its_patterns_notes_never_its_picture() {
        let mut c = card(2);
        let path = notes_path(Path::new("/p/design"), &c);
        assert_eq!(path, Path::new("/p/design/references/command-menu/notes.md"));
        let first = with_kept(None, &c, "2026-11-17").unwrap();
        assert!(first.starts_with("# command-menu\n\n## 21st.dev \u{2014} Card 2\n- Source: https://21st.dev/r/2 (21st.dev), 2026-11-17\n- Take: 36px rows\n- Leave: the neon\n- Fit: fits the dense list\n"), "{first}");
        assert!(!first.contains(".png"), "no picture: {first}");
        assert_eq!(with_kept(Some(&first), &c, "2026-11-18"), None, "kept once");
        c.link = "https://mobbin.com/screens/9".into();
        let second = with_kept(Some(&first), &c, "2026-11-18").unwrap();
        assert!(second.starts_with(first.trim_end()) && second.contains("private study material"), "{second}");
        let (notes, _) = crate::design::refgallery::parse_notes(&second);
        assert_eq!(notes.len(), 2, "the references gallery reads both");
    }
}
