//! What the real-Chrome tools decide from a page alone, with no I/O: which
//! URLs a member may open, which site a window is on, whether a page shows a
//! human check (the person's, never a member's), whether a page is a web
//! messaging app (sending there asks the person), and a page's text.

use serde_json::Value;

/// A URL a member may open in the person's Chrome: `http(s)` only — never
/// `chrome://` (settings, passwords), `file://`, `javascript:` or `data:`.
/// A bare domain gets `https://`.
pub fn web_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    if url.is_empty() {
        return Err("Give the page to open as `url` (https://…).".into());
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("That URL has spaces or control characters in it: encode them (%20) or give the page's real address.".into());
    }
    let lower = url.to_ascii_lowercase();
    // `name:` where what follows isn't a port: a scheme (javascript:, about:, data:…).
    let scheme = lower.split_once(':').is_some_and(|(name, rest)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) && !rest.starts_with(|c: char| c.is_ascii_digit())
    });
    let url = if lower.starts_with("https://") || lower.starts_with("http://") {
        url.to_owned()
    } else if lower.contains("://") || scheme {
        return Err("Only web pages (http:// or https://) open in the person's Chrome: no chrome://, file://, javascript: or data: pages.".into());
    } else {
        format!("https://{url}")
    };
    match host(&url) {
        Some(h) if !h.is_empty() => Ok(url),
        _ => Err(format!("{url:?} names no site.")),
    }
}

/// The host of an `http(s)` URL, lowercase, without user, port or a trailing dot.
pub fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if authority.starts_with('[') {
        authority.split_once(']').map(|(h, _)| format!("{h}]"))?
    } else {
        authority.split(':').next()?.to_owned()
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// The part of a host that names the site: `mail.google.com` → `google.com`,
/// `shop.example.co.uk` → `example.co.uk`. A heuristic (no public-suffix list):
/// good enough to group a site's sign-in pages with the site.
pub fn site(host: &str) -> String {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.starts_with('[') || host.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return host;
    }
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    let n = labels.len();
    if n <= 2 {
        return labels.join(".");
    }
    let second = labels[n - 2];
    let keep = if labels[n - 1].len() == 2 && ["co", "com", "net", "org", "ac", "gov", "edu", "ne", "or", "go"].contains(&second) { 3 } else { 2 };
    labels[n - keep..].join(".")
}

/// Whether two hosts are the same site ([`site`]).
pub fn same_site(a: &str, b: &str) -> bool {
    site(a) == site(b)
}

/// Words that mark a human check: a CAPTCHA, Cloudflare's or hCaptcha's
/// challenge. Matched in an element's name or description, lowercase.
const HUMAN_CHECK: [&str; 13] = [
    "captcha",
    "recaptcha",
    "hcaptcha",
    "turnstile",
    "i'm not a robot",
    "i am not a robot",
    "not a robot",
    "verify you are human",
    "verify you're human",
    "verifying you are human",
    "are you a robot",
    "human verification",
    "security challenge",
];

/// Page titles of a challenge page that stands in front of a site.
const CHALLENGE_TITLES: [&str; 5] = ["just a moment", "attention required", "verifying you are human", "security check", "ddos-guard"];

/// Whether an element named so is part of a human check.
pub fn is_human_check<'a>(names: impl IntoIterator<Item = &'a str>) -> bool {
    names.into_iter().any(|n| {
        let n = n.to_lowercase().replace('\u{2019}', "'");
        HUMAN_CHECK.iter().any(|w| n.contains(w))
    })
}

/// Whether a window title is a challenge page's ("Just a moment...").
pub fn challenge_title(title: &str) -> bool {
    let t = title.to_lowercase();
    CHALLENGE_TITLES.iter().any(|w| t.starts_with(w) || t.contains(&format!("{w} |")) || t.contains(&format!("{w} -")))
}

/// Whether a compact snapshot (the engine's text) shows a human check.
pub fn shows_human_check(snapshot: &str) -> bool {
    let mut lines = snapshot.lines();
    let header = lines.next().unwrap_or_default();
    // The header line carries the window's title in quotes.
    let title = header.split('"').nth(1).unwrap_or_default();
    challenge_title(title) || is_human_check(lines)
}

/// What a member is told when a human check is in the way.
pub const HUMAN_CHECK_NOTE: &str = "A human check (CAPTCHA, \"verify you are human\") is on this page. It is the person's: never click, type into or wait it out on their behalf. Call show_my_browser with the page's URL — it points the person at this same window, where they pass it themselves — then carry on when they're done.";

/// A web messaging app, by its window title, as the send gate knows the
/// desktop app of the same name (one of `desktop_gate::DEFAULT_APPS`): an
/// input that may send there asks the person, as in the app.
pub fn web_messaging(title: &str) -> Option<&'static str> {
    const BY_LAST_PART: [(&str, &str); 7] = [
        ("gmail", "Mail"),
        ("outlook", "Microsoft Outlook"),
        ("slack", "Slack"),
        ("microsoft teams", "Microsoft Teams"),
        ("discord", "Discord"),
        ("telegram", "Telegram"),
        ("telegram web", "Telegram"),
    ];
    const ENDS_WITH: [(&str, &str); 2] = [("whatsapp", "WhatsApp"), ("messages for web", "Messages")];
    let t = title.trim().to_lowercase();
    // The part after the last " - ", " | " or dash: where a web app names itself.
    let cut = [" - ", " | ", " \u{2013} ", " \u{2014} "].iter().filter_map(|sep| t.rfind(sep).map(|i| i + sep.len())).max().unwrap_or(0);
    let last = t[cut..].trim();
    if let Some((_, app)) = BY_LAST_PART.iter().find(|(name, _)| *name == last) {
        return Some(app);
    }
    ENDS_WITH.iter().find(|(name, _)| t.ends_with(name)).map(|(_, app)| *app)
}

/// Roles whose text is the page's content (not controls' labels alone).
const TEXT_ROLES: [&str; 12] = ["statictext", "text", "heading", "paragraph", "link", "cell", "listitem", "label", "code", "blockquote", "caption", "term"];

/// A page's text in reading order from a snapshot tree (agent-desktop's
/// JSON, all nodes): each text node's value or name once, password fields
/// never. At most `max` characters, with how much was cut.
pub fn text_of(tree: &Value, max: usize) -> String {
    fit(&lines_of(tree), max)
}

/// [`text_of`]'s lines, uncut.
pub fn lines_of(tree: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    walk(tree, &mut lines);
    lines
}

/// Lines joined, at most `max` characters, with how much was cut.
pub fn fit(lines: &[String], max: usize) -> String {
    let mut out = String::new();
    let (mut used, mut cut) = (0usize, 0usize);
    for line in lines {
        let len = line.chars().count() + usize::from(used > 0);
        if cut > 0 || used + len > max {
            cut += len;
            continue;
        }
        if used > 0 {
            out.push('\n');
        }
        out.push_str(line);
        used += len;
    }
    if out.is_empty() && cut == 0 {
        out.push_str("(no text on the page yet — it may still be loading: `wait` with app \"Google Chrome\" and `text`, then read again)");
    }
    if cut > 0 {
        out.push_str(&format!("\n… {cut} more characters cut: narrow it with `find`, or raise `max_chars`."));
    }
    out
}

fn walk(node: &Value, lines: &mut Vec<String>) {
    let role = node.get("role").and_then(Value::as_str).unwrap_or_default().to_ascii_lowercase();
    let secure = node.get("states").and_then(Value::as_array).is_some_and(|s| s.iter().any(|v| v.as_str() == Some("secure")));
    let children = node.get("children").and_then(Value::as_array);
    let leaf = children.is_none_or(|c| c.is_empty());
    let says = |key: &str| node.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let text = if secure {
        None
    } else if TEXT_ROLES.contains(&role.as_str()) || leaf && role != "window" {
        says("value").or_else(|| says("name"))
    } else {
        None
    };
    if let Some(text) = text {
        let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        // A link or heading's text often repeats as its own static text child.
        if lines.last() != Some(&line) {
            lines.push(line);
        }
    }
    for child in children.into_iter().flatten() {
        walk(child, lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_web_pages_open() {
        assert_eq!(web_url("https://github.com/login").unwrap(), "https://github.com/login");
        assert_eq!(web_url("  example.com/a?b=c ").unwrap(), "https://example.com/a?b=c");
        assert_eq!(web_url("HTTP://Example.com").unwrap(), "HTTP://Example.com");
        assert_eq!(web_url("localhost:3000/x").unwrap(), "https://localhost:3000/x");
        for bad in ["chrome://settings/passwords", "file:///etc/hosts", "javascript:alert(1)", "data:text/html,hi", "about:blank", "", "https://", "https://a b.com"] {
            assert!(web_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn hosts_and_sites() {
        assert_eq!(host("https://user:pw@Mail.Google.com:443/x?y#z").as_deref(), Some("mail.google.com"));
        assert_eq!(host("https://[::1]:8080/").as_deref(), Some("[::1]"));
        assert_eq!(site("mail.google.com"), "google.com");
        assert_eq!(site("accounts.google.com"), "google.com");
        assert_eq!(site("shop.example.co.uk"), "example.co.uk");
        assert_eq!(site("www.naver.com"), "naver.com");
        assert_eq!(site("127.0.0.1"), "127.0.0.1");
        assert!(same_site("accounts.google.com", "docs.google.com"));
        assert!(!same_site("github.com", "gitlab.com"));
    }

    #[test]
    fn human_checks_are_seen_by_name_and_by_title() {
        assert!(is_human_check(["reCAPTCHA"]));
        assert!(is_human_check(["Widget containing a Cloudflare security challenge"]));
        assert!(is_human_check(["I\u{2019}m not a robot"]));
        assert!(is_human_check(["Verify you are human"]));
        assert!(!is_human_check(["Sign in", "Robotics course"]));
        assert!(challenge_title("Just a moment..."));
        assert!(challenge_title("Attention Required! | Cloudflare"));
        assert!(!challenge_title("Pull requests · acme/app"));
        let page = "Google Chrome · \"Just a moment...\" · w-7 · snapshot s1 · 1 refs\ne1 button \"Retry\"";
        assert!(shows_human_check(page));
        let page = "Google Chrome · \"Log in\" · w-7 · snapshot s1 · 2 refs\ne1 checkbox \"I'm not a robot\"\ne2 button \"Log in\"";
        assert!(shows_human_check(page));
        assert!(!shows_human_check("Google Chrome · \"Log in\" · w-7 · snapshot s1 · 1 refs\ne1 button \"Log in\""));
    }

    #[test]
    fn web_messaging_apps_count_as_their_desktop_app() {
        assert_eq!(web_messaging("Inbox (3) - someone@example.com - Gmail"), Some("Mail"));
        assert_eq!(web_messaging("Mail - Jordan Lee - Outlook"), Some("Microsoft Outlook"));
        assert_eq!(web_messaging("launch-plan (Channel) - Acme - Slack"), Some("Slack"));
        assert_eq!(web_messaging("Chat | Microsoft Teams"), Some("Microsoft Teams"));
        assert_eq!(web_messaging("#general | Friends - Discord"), Some("Discord"));
        assert_eq!(web_messaging("(2) WhatsApp"), Some("WhatsApp"));
        assert_eq!(web_messaging("Google Messages for web"), Some("Messages"));
        assert_eq!(web_messaging("Pull requests · acme/app"), None);
        assert_eq!(web_messaging("How Slack works - Wikipedia"), None);
    }

    #[test]
    fn page_text_reads_in_order_without_passwords_and_says_what_was_cut() {
        let tree = json!({ "role": "window", "name": "Docs", "children": [
            { "role": "heading", "name": "Pricing", "children": [ { "role": "statictext", "name": "Pricing" } ] },
            { "role": "statictext", "name": "  Pro   plan: $10 " },
            { "role": "textfield", "name": "Password", "value": "hunter2", "states": ["secure"] },
            { "role": "link", "name": "Contact sales" },
            { "role": "group", "name": "Layout box", "children": [ { "role": "button", "name": "Buy" } ] },
        ]});
        assert_eq!(text_of(&tree, 1000), "Pricing\nPro plan: $10\nContact sales\nBuy");
        let cut = text_of(&tree, 21);
        assert!(cut.starts_with("Pricing\nPro plan: $10\n…"), "{cut}");
        assert!(cut.contains("more characters cut"), "{cut}");
        assert!(text_of(&json!({ "role": "window", "name": "Blank" }), 100).contains("still be loading"));
    }
}
