//! Per-deploy preview addresses: hosts that change with every deploy
//! (`app-1a2b3c4d5-team.vercel.app`, `deploy-preview-42--site.netlify.app`,
//! `1a2b3c4d.app.pages.dev`). A sign-in there lives in a cookie for that one
//! host, so it never carries to the next deploy and the person would be asked
//! again each time. equipe refuses to put the person in front of one — a site
//! sign-in, a browser handed over, a sign-in yes, a ticket put down on them —
//! and tells the member to verify on localhost with the project's test sign-in,
//! or on the stable production address once the change has landed.
//!
//! More patterns go in `<equipe home>/per-deploy-hosts`, one per line, `*`
//! matching any run of characters (`*.preview.example.com`); `#` starts a comment.

use std::path::Path;

use serde_json::Value;

/// The file of extra patterns, in equipe's home.
pub const EXTRA_FILE: &str = "per-deploy-hosts";

/// `len` characters of lowercase letters and digits, at least one a digit.
fn hashlike(part: &str, len: usize) -> bool {
    part.len() == len && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()) && part.bytes().any(|b| b.is_ascii_digit())
}

fn hex(part: &str, len: usize) -> bool {
    part.len() == len && part.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `*` matches any run of characters; everything else itself, ignoring case.
fn glob(pattern: &str, host: &str) -> bool {
    let (p, h) = (pattern.to_ascii_lowercase(), host.to_ascii_lowercase());
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == h;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !h.starts_with(first) || !h[first.len()..].ends_with(last) || h.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &h[first.len()..h.len() - last.len()];
    for middle in &parts[1..parts.len() - 1] {
        match rest.find(middle) {
            Some(at) => rest = &rest[at + middle.len()..],
            None => return false,
        }
    }
    true
}

/// Whether `host` changes with every deploy: a built-in pattern, or one of `extra`.
pub fn per_deploy(host: &str, extra: &[String]) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if extra.iter().any(|p| glob(p, &host)) {
        return true;
    }
    // Vercel: `<project>-<9-character hash>-<team>.vercel.app`; a branch's
    // (`-git-<branch>-`) and the project's own address stay.
    if let Some(label) = host.strip_suffix(".vercel.app")
        && !label.contains('.')
        && !label.contains("-git-")
    {
        let parts: Vec<&str> = label.split('-').collect();
        return parts.len() >= 2 && parts[1..].iter().any(|p| hashlike(p, 9));
    }
    // Netlify: a pull request's preview, or one deploy's permalink.
    if let Some(label) = host.strip_suffix(".netlify.app")
        && !label.contains('.')
    {
        return label.starts_with("deploy-preview-") || label.split_once("--").is_some_and(|(id, _)| hex(id, 24));
    }
    // Cloudflare Pages: `<8 hex>.<project>.pages.dev`.
    if let Some(rest) = host.strip_suffix(".pages.dev") {
        let labels: Vec<&str> = rest.split('.').collect();
        return labels.len() == 2 && hex(labels[0], 8);
    }
    // Firebase Hosting preview channels: `<site>--<channel>-<8 characters>.web.app`.
    if let Some(label) = host.strip_suffix(".web.app").or_else(|| host.strip_suffix(".firebaseapp.com"))
        && !label.contains('.')
        && let Some((_, channel)) = label.split_once("--")
    {
        return channel.rsplit_once('-').is_some_and(|(_, tail)| tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_alphanumeric()));
    }
    false
}

/// The hosts written in `text`: in addresses (`https://host/…`) and bare (`host.vercel.app`).
pub fn hosts_in(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || "()[]<>\"'`,;|".contains(c))
        .filter_map(|word| {
            let word = word.split_once("://").map_or(word, |(_, rest)| rest);
            let host = word.split(['/', '?', '#']).next()?;
            let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
            let host = host.split(':').next()?.trim_end_matches(['.', '!']);
            (host.contains('.') && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')).then(|| host.to_ascii_lowercase())
        })
        .collect()
}

/// The first per-deploy host in `texts`.
pub fn first_per_deploy<'a>(texts: impl IntoIterator<Item = &'a str>, extra: &[String]) -> Option<String> {
    texts.into_iter().flat_map(hosts_in).find(|h| per_deploy(h, extra))
}

/// The extra patterns kept in `home` (equipe's), none when there is no file.
pub fn read_extra(home: &Path) -> Vec<String> {
    std::fs::read_to_string(home.join(EXTRA_FILE))
        .unwrap_or_default()
        .lines()
        .map(|l| l.split('#').next().unwrap_or_default().trim())
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

fn strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|v| strings(v, out)),
        Value::Object(map) => map.values().for_each(|v| strings(v, out)),
        _ => {}
    }
}

/// What of a call to `tool` puts the person in front of a page: the texts to
/// look for a per-deploy host in. Nothing for a call that doesn't.
pub fn texts_of(tool: &str, args: &Value) -> Vec<String> {
    let keys: &[&str] = match tool {
        crate::board::SIGN_IN => &["site"],
        crate::board::SHOW_BROWSER => &["url", "why"],
        // Only a sign-in's yes: a preview may be named in other asks.
        crate::board::ASK_PERSON if args["kind"].as_str() == Some("sign_in") => &["action", "why", "summary"],
        crate::board::BLOCK_ON_PERSON => &["ask", "needs", "why"],
        _ => &[],
    };
    let mut out = Vec::new();
    for key in keys {
        strings(&args[*key], &mut out);
    }
    out
}

/// What the member is told instead.
pub fn refusal(host: &str) -> String {
    format!(
        "`{host}` is a per-deploy preview address: it changes with every deploy, so a sign-in there never carries to the \
next one and the person would be asked again each time. Nothing was put to the person. Verify on localhost with the \
project's own test sign-in (a seeded test user, the dev login or the test auth bypass), or on the stable production \
address once your change has landed — never ask the person to sign in on a preview."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn per_deploy_previews_are_told_from_stable_addresses() {
        let none: &[String] = &[];
        for host in [
            "shop-web-k3j9x2m1q-acme-team.vercel.app",
            "shop-a1b2c3d4e-acme.vercel.app",
            "shop-a1b2c3d4e.vercel.app",
            "deploy-preview-42--shop.netlify.app",
            "5f1e2d3c4b5a69788796a5b4--shop.netlify.app",
            "1a2b3c4d.shop.pages.dev",
            "shop--pr42-abcd1234.web.app",
            "SHOP-K3J9X2M1Q-ACME.vercel.app",
        ] {
            assert!(per_deploy(host, none), "{host}");
        }
        for host in [
            "shop.vercel.app",
            "shop-acme.vercel.app",
            "shop-git-feature-login-acme.vercel.app",
            "my-dashboard-acme.vercel.app",
            "shop.netlify.app",
            "main--shop.netlify.app",
            "shop.pages.dev",
            "staging.shop.pages.dev",
            "shop.web.app",
            "shop.example.com",
            "localhost",
            "vercel.com",
        ] {
            assert!(!per_deploy(host, none), "{host}");
        }
    }

    #[test]
    fn more_patterns_come_from_a_file() {
        let extra = vec!["*.preview.example.com".to_owned(), "pr-*.staging.example.com".to_owned()];
        assert!(per_deploy("abc.preview.example.com", &extra));
        assert!(per_deploy("pr-12.staging.example.com", &extra));
        assert!(!per_deploy("staging.example.com", &extra));
        assert!(!per_deploy("preview.example.com", &extra));
        let home = std::env::temp_dir().join(format!("equipe-per-deploy-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(EXTRA_FILE), "# previews\n*.preview.example.com  # ours\n\n").unwrap();
        assert_eq!(read_extra(&home), ["*.preview.example.com"]);
        let _ = std::fs::remove_dir_all(&home);
        assert!(read_extra(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn hosts_are_found_in_links_and_words() {
        let text = "Sign in at [the preview](https://shop-k3j9x2m1q-acme.vercel.app/login?next=/) or shop.example.com:443, then `localhost:3000`.";
        assert_eq!(hosts_in(text), ["shop-k3j9x2m1q-acme.vercel.app", "shop.example.com"]);
        assert_eq!(first_per_deploy([text], &[]).as_deref(), Some("shop-k3j9x2m1q-acme.vercel.app"));
    }

    #[test]
    fn the_calls_that_face_the_person_are_looked_at() {
        let url = "https://shop-k3j9x2m1q-acme.vercel.app";
        let hit = |tool: &str, args: Value| first_per_deploy(texts_of(tool, &args).iter().map(String::as_str), &[]);
        assert!(hit("sign_in", json!({ "site": url, "why": "x" })).is_some());
        assert!(hit("show_my_browser", json!({ "url": url, "why": "Sign in with the company account" })).is_some());
        assert!(hit("block_on_person", json!({ "ask": "Sign in", "needs": [format!("Open [the preview]({url}) and sign in")] })).is_some());
        assert!(hit("ask_person", json!({ "kind": "sign_in", "action": format!("Sign in on {url}") })).is_some());
        // Another ask naming a preview, a stable address, another tool: left alone.
        assert!(hit("ask_person", json!({ "kind": "outside", "action": format!("Share {url}") })).is_none());
        assert!(hit("sign_in", json!({ "site": "https://shop.example.com/login" })).is_none());
        assert!(hit("comment", json!({ "text": url })).is_none());
        assert!(refusal("x.vercel.app").contains("localhost") && refusal("x.vercel.app").contains("production"));
    }
}
