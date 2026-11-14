//! Shareable links to a place in a squad: "look at this ticket", "read this
//! conversation", "answer this decision".
//!
//! Two forms say the same thing:
//!
//! - `equipe://open/<squad>/<kind>/<id>?host=<machine>` — what the app opens;
//! - `https://equipe.im/o/#<squad>/<kind>/<id>?host=<machine>` — for chat apps
//!   that don't make a custom scheme clickable. A fragment never reaches a
//!   server; the page at that address only hands it back to the first form.
//!
//! Kinds: `ticket` (`T-2391`), `conversation` (a session id), `member` (a
//! name), `decision` (a question set's id), `needs-you` and `status` (no id).
//! `host` names the machine the squad's daemon runs on — its Tailscale
//! (MagicDNS) name or host name — so the app can tell which of the daemons it
//! knows the link is about. Squads are only named per daemon, so two Macs can
//! each have a squad called "web".
//!
//! Everything here is plain text in, plain text out. Parsing is tolerant:
//! chat apps wrap a link in `<…>`, brackets or quotes, put a full stop or a
//! comma after it, or make it Markdown; [`find`] picks the links out of any
//! text, [`parse`] reads one.

use std::ops::Range;

/// The URL scheme the app registers.
pub const SCHEME: &str = "equipe";

/// Where the https form lives (the page reads `location.hash`).
pub const WEB_BASE: &str = "https://equipe.im/o/";

/// What a link points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Ticket(u32),
    Conversation(u64),
    Member(String),
    /// A question set (`QuestionSet::id`).
    Decision(u64),
    NeedsYou,
    Status,
}

impl Target {
    /// The kind's word in a link.
    pub fn kind(&self) -> &'static str {
        match self {
            Target::Ticket(_) => "ticket",
            Target::Conversation(_) => "conversation",
            Target::Member(_) => "member",
            Target::Decision(_) => "decision",
            Target::NeedsYou => "needs-you",
            Target::Status => "status",
        }
    }
}

/// A link: the squad, a place in it, and the machine its daemon is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepLink {
    pub squad: String,
    pub target: Target,
    /// The machine's name, when the sender knew it.
    pub host: Option<String>,
}

impl DeepLink {
    pub fn new(squad: impl Into<String>, target: Target, host: Option<String>) -> Self {
        Self { squad: squad.into(), target, host: host.filter(|h| !h.trim().is_empty()) }
    }

    /// `squad/kind/id?host=…`: what both forms share.
    fn path(&self) -> String {
        let mut out = escape(&self.squad);
        out.push('/');
        out.push_str(self.target.kind());
        match &self.target {
            Target::Ticket(n) => out.push_str(&format!("/T-{n}")),
            Target::Conversation(id) | Target::Decision(id) => out.push_str(&format!("/{id}")),
            Target::Member(name) => {
                out.push('/');
                out.push_str(&escape(name));
            }
            Target::NeedsYou | Target::Status => {}
        }
        if let Some(host) = &self.host {
            out.push_str("?host=");
            out.push_str(&escape(host));
        }
        out
    }

    /// `equipe://open/<squad>/<kind>/<id>?host=…`
    pub fn uri(&self) -> String {
        format!("{SCHEME}://open/{}", self.path())
    }

    /// `https://equipe.im/o/#<squad>/<kind>/<id>?host=…`
    pub fn web(&self) -> String {
        format!("{WEB_BASE}#{}", self.path())
    }

    /// What copying "as text" puts on the clipboard: the title, then the link
    /// — chat apps show plain text, so the words say what the link is.
    pub fn as_text(&self, title: &str) -> String {
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() { self.uri() } else { format!("{title} · {}", self.uri()) }
    }
}

/// Read one link: either form, as typed or as a chat app left it.
pub fn parse(text: &str) -> Option<DeepLink> {
    find(text).into_iter().next().map(|(_, link)| link)
}

/// Every link in `text` with where it is (byte range, wrapping and trailing
/// punctuation left outside), in order.
pub fn find(text: &str) -> Vec<(Range<usize>, DeepLink)> {
    let lower = text.to_ascii_lowercase();
    let mut at = 0;
    let mut out = Vec::new();
    while at < text.len() {
        let Some((start, body)) = next_start(&lower, at) else { break };
        let end = token_end(text, body);
        let token = trim_tail(&text[start..end]);
        let end = start + token.len();
        let body_in_token = (body - start).min(token.len());
        match read(&token[body_in_token..]) {
            Some(link) => {
                out.push((start..end, link));
                at = end.max(start + 1);
            }
            None => at = (start + 1).min(text.len()),
        }
    }
    out
}

/// The earliest link start at or after `from`: where it begins and where its
/// `squad/…` part begins.
fn next_start(lower: &str, from: usize) -> Option<(usize, usize)> {
    // `equipe://open/`, `equipe:/open/` and `equipe:open/`, as people retype it.
    let scheme = lower[from..].match_indices("equipe:").find_map(|(i, _)| {
        let start = from + i;
        let rest = &lower[start + "equipe:".len()..];
        let skipped = rest.len() - rest.trim_start_matches('/').len();
        rest[skipped..].starts_with("open/").then_some((start, start + "equipe:".len() + skipped + "open/".len()))
    });
    let web = ["https://equipe.im/o", "http://equipe.im/o", "https://www.equipe.im/o", "equipe.im/o"]
        .iter()
        .filter_map(|prefix| {
            lower[from..].match_indices(prefix).find_map(|(i, _)| {
                let start = from + i;
                // Not the tail of another word (`notequipe.im/o`).
                if start > 0 && lower.as_bytes()[start - 1].is_ascii_alphanumeric() {
                    return None;
                }
                let rest = &lower[start + prefix.len()..];
                let rest_trim = rest.trim_start_matches('/');
                // The part after the page's own path: `#squad/…`.
                let hash = rest_trim.find('#')?;
                // Only the page's own path may lie between: `/o/#`, `/o#`.
                if hash > 0 {
                    return None;
                }
                Some((start, start + prefix.len() + (rest.len() - rest_trim.len()) + 1))
            })
        })
        .min_by_key(|(start, _)| *start);
    [scheme, web].into_iter().flatten().min_by_key(|(start, _)| *start)
}

/// Where the token starting at `body` ends: at a space, a quote or bracket
/// chat apps put around a link, or CJK / full-width punctuation.
fn token_end(text: &str, body: usize) -> usize {
    let stop = |c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`' | '[' | ']' | '(' | ')' | '|' | '{' | '}' | '\u{201c}' | '\u{201d}' | '\u{2018}' | '\u{2019}') || ('\u{3000}'..='\u{303f}').contains(&c) || ('\u{ff00}'..='\u{ffef}').contains(&c);
    text[body..].find(stop).map_or(text.len(), |i| body + i)
}

/// Without the punctuation a sentence puts after a link (and Markdown's
/// emphasis marks).
fn trim_tail(token: &str) -> &str {
    token.trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '_', '~', '/', '\u{2026}'])
}

/// `squad/kind/id?host=…` — the part after `open/` or `#`.
fn read(body: &str) -> Option<DeepLink> {
    let body = body.trim_start_matches('/');
    let (path, query) = match body.split_once('?') {
        Some((p, q)) => (p, q),
        None => (body, ""),
    };
    let path = path.trim_end_matches('/');
    let mut parts = path.split('/');
    let squad = unescape(parts.next()?)?;
    if squad.trim().is_empty() {
        return None;
    }
    let kind = parts.next()?.to_ascii_lowercase();
    let id = parts.next().filter(|s| !s.is_empty());
    let number = |s: Option<&str>| s.and_then(|s| s.trim_start_matches(['T', 't']).trim_start_matches('-').parse::<u64>().ok());
    let target = match kind.as_str() {
        "ticket" => Target::Ticket(u32::try_from(number(id)?).ok()?),
        "conversation" => Target::Conversation(number(id)?),
        "decision" => Target::Decision(number(id)?),
        "member" => Target::Member(unescape(id?)?).filter_named()?,
        "needs-you" | "needsyou" | "needs_you" => Target::NeedsYou,
        "status" => Target::Status,
        _ => return None,
    };
    let host = query.split('&').find_map(|pair| pair.strip_prefix("host=")).and_then(unescape).map(|h| h.trim().to_owned()).filter(|h| !h.is_empty());
    Some(DeepLink { squad, target, host })
}

impl Target {
    /// A member link needs a name.
    fn filter_named(self) -> Option<Self> {
        match &self {
            Target::Member(name) if name.trim().is_empty() => None,
            _ => Some(self),
        }
    }
}

/// Percent-encode everything but the unreserved characters, so a link is one
/// word to every chat app (a squad or member may have spaces or any script).
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Percent-decode; `+` stays a plus. `None` for bytes that aren't text.
fn unescape(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

// ── which daemon a link is about ──

/// A host name as a comparison key: lower case, without a user, port or
/// trailing dot — and, for a name (not an address), only its first label, so
/// `mini.tail3383cf.ts.net`, `Mini` and `mini.local` are one machine.
pub fn host_key(host: &str) -> String {
    let host = host.trim();
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(v6),
        None if host.matches(':').count() == 1 => host.split(':').next().unwrap_or(host),
        None => host,
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host;
    }
    host.split('.').next().unwrap_or_default().to_owned()
}

/// A daemon the app knows or is attached to, as far as routing is concerned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Daemon {
    /// Every name it answers to (host names, Tailscale names, an address).
    pub names: Vec<String>,
    /// The squads it is known to have (empty until it has said).
    pub squads: Vec<String>,
    /// A window is connected to it now.
    pub attached: bool,
    /// The squad that window shows (empty when none or unknown yet).
    pub current_squad: String,
}

impl Daemon {
    fn is(&self, host: &str) -> bool {
        let key = host_key(host);
        !key.is_empty() && self.names.iter().any(|n| host_key(n) == key)
    }

    fn has_squad(&self, squad: &str) -> bool {
        self.current_squad.eq_ignore_ascii_case(squad) || self.squads.iter().any(|s| s.eq_ignore_ascii_case(squad))
    }
}

/// What to do with a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// A window is on that daemon and squad: go there.
    Here(usize),
    /// The daemon is attached, on another squad of it: open that squad.
    Squad(usize),
    /// The daemon is known (not attached right now): offer to switch to it.
    Switch(usize),
    /// Nothing known has that host or squad: say which Mac and how to connect.
    Unknown,
}

/// Decide where `link` goes among `daemons`. With no host in the link, the
/// squad's name alone decides, among the daemons attached.
pub fn route(link: &DeepLink, daemons: &[Daemon]) -> Route {
    let same_host = |d: &Daemon| link.host.as_deref().is_none_or(|h| d.is(h));
    let mine: Vec<usize> = daemons.iter().enumerate().filter(|(_, d)| same_host(d) && (link.host.is_some() || d.attached)).map(|(i, _)| i).collect();
    // Attached and on the squad now.
    if let Some(i) = mine.iter().copied().find(|i| daemons[*i].attached && daemons[*i].current_squad.eq_ignore_ascii_case(&link.squad)) {
        return Route::Here(i);
    }
    // Attached, and the squad is one of its other squads.
    if let Some(i) = mine.iter().copied().find(|i| daemons[*i].attached && daemons[*i].has_squad(&link.squad)) {
        return Route::Squad(i);
    }
    // Known by host but not attached: the squad can't be checked until connected.
    if link.host.is_some()
        && let Some(i) = mine.iter().copied().find(|i| !daemons[*i].attached)
    {
        return Route::Switch(i);
    }
    Route::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket() -> DeepLink {
        DeepLink::new("web", Target::Ticket(2391), Some("mini".into()))
    }

    #[test]
    fn both_forms_round_trip_every_kind() {
        let links = [
            ticket(),
            DeepLink::new("web", Target::Conversation(1782345678901234), None),
            DeepLink::new("my squad", Target::Member("Jean-Luc Zoë".into()), Some("Mac mini.local".into())),
            DeepLink::new("웹", Target::Decision(77), Some("mini.tail3383cf.ts.net".into())),
            DeepLink::new("web", Target::NeedsYou, Some("mini".into())),
            DeepLink::new("web", Target::Status, None),
        ];
        for link in links {
            assert_eq!(parse(&link.uri()).as_ref(), Some(&link), "{}", link.uri());
            assert_eq!(parse(&link.web()).as_ref(), Some(&link), "{}", link.web());
            assert!(link.uri().is_ascii() && !link.uri().contains(' '), "one word for every chat app: {}", link.uri());
        }
    }

    #[test]
    fn the_formats_are_what_the_docs_say() {
        assert_eq!(ticket().uri(), "equipe://open/web/ticket/T-2391?host=mini");
        assert_eq!(ticket().web(), "https://equipe.im/o/#web/ticket/T-2391?host=mini");
        assert_eq!(DeepLink::new("web", Target::Status, None).uri(), "equipe://open/web/status");
        assert_eq!(DeepLink::new("web", Target::NeedsYou, None).web(), "https://equipe.im/o/#web/needs-you");
    }

    #[test]
    fn a_chat_app_wrapping_or_punctuating_a_link_changes_nothing() {
        let link = ticket();
        let uri = link.uri();
        for text in [
            format!("{uri}."),
            format!("look: {uri}, please"),
            format!("<{uri}>"),
            format!("<{uri}|T-2391>"),
            format!("({uri})"),
            format!("[T-2391]({uri})"),
            format!("\"{uri}\""),
            format!("`{uri}`"),
            format!("**{uri}**"),
            format!("{uri}!?"),
            format!("See {uri}。"),
            format!("T-2391 구글시트 4개 공유하기 · {uri}"),
            format!("{uri}\nnext line"),
            "Equipe://Open/web/ticket/t2391?host=mini".to_owned(),
        ] {
            assert_eq!(parse(&text).as_ref(), Some(&link), "{text}");
        }
        let web = link.web();
        for text in [format!("{web}."), format!("<{web}>"), format!("({web})"), format!("[here]({web})"), format!("{web}/"), web.replace("https://", "")] {
            assert_eq!(parse(&text).as_ref(), Some(&link), "{text}");
        }
    }

    #[test]
    fn a_link_found_in_text_says_where_it_is() {
        let uri = ticket().uri();
        let text = format!("a {uri}, b equipe://open/web/status.");
        let found = find(&text);
        assert_eq!(found.len(), 2);
        assert_eq!(&text[found[0].0.clone()], uri);
        assert_eq!(&text[found[1].0.clone()], "equipe://open/web/status");
        assert_eq!(found[1].1.target, Target::Status);
    }

    #[test]
    fn an_earlier_word_that_is_not_a_link_does_not_hide_a_later_link() {
        let link = parse("equipe: see equipe:foo and https://equipe.im/oops then equipe://open/web/status.");
        assert_eq!(link, Some(DeepLink::new("web", Target::Status, None)));
    }

    #[test]
    fn what_is_not_a_link_is_left_alone() {
        for text in [
            "",
            "equipe",
            "equipe://",
            "equipe://open/",
            "equipe://open/web",
            "equipe://open/web/ticket",
            "equipe://open/web/ticket/abc",
            "equipe://open/web/ticket/T-99999999999",
            "equipe://open/web/planet/3",
            "equipe://open/web/member/",
            "equipe://open//ticket/3",
            "https://equipe.im/o/",
            "https://equipe.im/o/#",
            "https://example.com/o/#web/ticket/3",
            "notequipe.im/o/#web/ticket/3",
        ] {
            assert_eq!(parse(text), None, "{text}");
        }
    }

    #[test]
    fn extra_query_parts_and_a_missing_host_are_fine() {
        let link = parse("equipe://open/web/ticket/3?utm=x&host=Mini&more=1").unwrap();
        assert_eq!(link.host.as_deref(), Some("Mini"));
        assert_eq!(parse("equipe://open/web/ticket/3?host=").unwrap().host, None);
        assert_eq!(parse("equipe://open/web/ticket/3").unwrap().host, None);
        assert_eq!(parse("equipe://open/web/ticket/3/extra").unwrap().target, Target::Ticket(3));
    }

    #[test]
    fn copying_as_text_puts_the_title_before_the_link() {
        assert_eq!(ticket().as_text("T-2391 구글시트 4개 공유하기"), "T-2391 구글시트 4개 공유하기 · equipe://open/web/ticket/T-2391?host=mini");
        assert_eq!(ticket().as_text("  two\nlines  "), "two lines · equipe://open/web/ticket/T-2391?host=mini");
        assert_eq!(ticket().as_text("  "), ticket().uri());
        // What was copied reads back as the same link.
        assert_eq!(parse(&ticket().as_text("T-2391 구글시트 4개 공유하기")), Some(ticket()));
    }

    #[test]
    fn host_names_compare_by_machine() {
        assert_eq!(host_key("Mini"), "mini");
        assert_eq!(host_key("abel@mini.tail3383cf.ts.net."), "mini");
        assert_eq!(host_key("mini.local:22"), "mini");
        assert_eq!(host_key("100.66.18.3"), "100.66.18.3");
        assert_eq!(host_key("[fd7a::1]"), "fd7a::1");
    }

    fn daemon(names: &[&str], squads: &[&str], attached: bool, current: &str) -> Daemon {
        Daemon { names: names.iter().map(|s| (*s).into()).collect(), squads: squads.iter().map(|s| (*s).into()).collect(), attached, current_squad: current.into() }
    }

    #[test]
    fn a_link_to_the_daemon_a_window_is_on_goes_there() {
        let daemons = [daemon(&["this-mac", "macbook"], &["web"], true, "web"), daemon(&["mini.tail3383cf.ts.net"], &["web", "docs"], true, "docs")];
        let on_mini = DeepLink::new("web", Target::Ticket(1), Some("MINI".into()));
        // Squad "web" exists on both Macs: the host decides, the squad then picks the sub-place.
        assert_eq!(route(&on_mini, &daemons), Route::Squad(1), "attached to the mini, on another squad of it");
        assert_eq!(route(&DeepLink::new("docs", Target::Status, Some("mini".into())), &daemons), Route::Here(1));
        assert_eq!(route(&DeepLink::new("web", Target::Status, Some("macbook".into())), &daemons), Route::Here(0));
    }

    #[test]
    fn without_a_host_the_squad_decides_among_attached_daemons() {
        let daemons = [daemon(&["a"], &["web"], false, ""), daemon(&["b"], &["web"], true, "web")];
        assert_eq!(route(&DeepLink::new("web", Target::NeedsYou, None), &daemons), Route::Here(1));
        assert_eq!(route(&DeepLink::new("other", Target::NeedsYou, None), &daemons), Route::Unknown);
    }

    #[test]
    fn a_known_daemon_not_attached_is_offered() {
        let daemons = [daemon(&["macbook"], &["web"], true, "web"), daemon(&["mini.tail3383cf.ts.net"], &[], false, "")];
        assert_eq!(route(&DeepLink::new("web", Target::Ticket(5), Some("mini".into())), &daemons), Route::Switch(1));
    }

    #[test]
    fn an_unknown_host_or_squad_is_not_guessed() {
        let daemons = [daemon(&["macbook"], &["web"], true, "web")];
        assert_eq!(route(&DeepLink::new("web", Target::Ticket(5), Some("studio".into())), &daemons), Route::Unknown);
        assert_eq!(route(&DeepLink::new("nope", Target::Ticket(5), Some("macbook".into())), &daemons), Route::Unknown);
        assert_eq!(route(&DeepLink::new("web", Target::Ticket(5), None), &[]), Route::Unknown);
    }
}
