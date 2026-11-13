//! Links in the words a member puts to the person — a question, a "waiting on
//! you" ask.
//!
//! A link doesn't belong in a sentence: a sign-in address is four hundred
//! characters of query string that bury the ask, can't be clicked and is hard
//! to copy. Screens draw each as a row of its own under the words. A member
//! gives its links cleanly (`links` on a question); for words that still carry
//! one, [`extract`] finds links in them (plain addresses and Markdown links),
//! takes the long ones out of the sentence and keeps what is left readable.

use serde::{Deserialize, Serialize};

/// An address at most this long may stay in the sentence it is in.
pub const SHORT: usize = 40;

/// A link: where it goes, and what the member calls it (empty: screens
/// name it by its address).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    pub url: String,
}

/// What screens know of a link's address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Looks {
    /// The host, lowercase, without a leading `www.`.
    pub host: String,
    /// The service's own name where the host is a well-known one ("AWS").
    pub service: Option<&'static str>,
    /// The page is a sign-in (or a device-code or authorize page).
    pub sign_in: bool,
}

/// A link's host and path as a person would read them: `None` for anything
/// that isn't a web address.
pub fn looks(url: &str) -> Option<Looks> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let (authority, path) = match rest.find(['/', '?', '#']) {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    };
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h).split(':').next()?.trim_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    let service = SERVICES.iter().find(|(suffix, _)| host == *suffix || host.ends_with(&format!(".{suffix}"))).map(|(_, name)| *name);
    let first = host.split('.').next().unwrap_or_default();
    let path = path.split(['?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    let sign_in = matches!(first, "signin" | "login" | "accounts" | "auth" | "sso" | "id" | "idp")
        || ["/signin", "/sign-in", "/login", "/log-in", "/authorize", "/oauth", "/device", "/sso"].iter().any(|p| path.contains(p));
    Some(Looks { host, service, sign_in })
}

/// Hosts with a name of their own, by the domain they are under.
const SERVICES: &[(&str, &str)] = &[
    ("amazonaws.com", "AWS"),
    ("aws.amazon.com", "AWS"),
    ("github.com", "GitHub"),
    ("gitlab.com", "GitLab"),
    ("google.com", "Google"),
    ("googleapis.com", "Google"),
    ("microsoftonline.com", "Microsoft"),
    ("microsoft.com", "Microsoft"),
    ("live.com", "Microsoft"),
    ("azure.com", "Azure"),
    ("openai.com", "OpenAI"),
    ("chatgpt.com", "ChatGPT"),
    ("anthropic.com", "Anthropic"),
    ("claude.ai", "Claude"),
    ("vercel.com", "Vercel"),
    ("cloudflare.com", "Cloudflare"),
    ("apple.com", "Apple"),
    ("slack.com", "Slack"),
    ("notion.so", "Notion"),
    ("atlassian.com", "Atlassian"),
    ("figma.com", "Figma"),
    ("stripe.com", "Stripe"),
];

/// What `text` is once its links are found: the words left, and every link.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extracted {
    /// The words with each long link taken out (a short address stays where
    /// it was said; a Markdown link with a short address is its label).
    pub text: String,
    /// Every link found, once each, in the order they were said.
    pub links: Vec<Link>,
}

/// Find the links in `text` — `http(s)://` addresses and `[label](address)`
/// — and take the long ones out of the sentence. Trailing punctuation isn't
/// part of an address, nor are the brackets around one; a query string with
/// `%` escapes is kept whole.
pub fn extract(text: &str) -> Extracted {
    let mut out = String::new();
    let mut links: Vec<Link> = Vec::new();
    let mut taken = false;
    let mut inline = false;
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        // `[label](address)`
        if rest.starts_with('[')
            && let Some((label, url, len)) = markdown_link(rest)
        {
            let long = url.len() > SHORT;
            push(&mut links, label, url);
            if long {
                taken = true;
                close_gap(&mut out, &text[i + len..]);
            } else {
                inline = true;
                out.push_str(if label.trim().is_empty() { url } else { label });
            }
            i += len;
            continue;
        }
        // A bare address, at the start of a word.
        let at_word = text[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        if at_word && (rest.starts_with("http://") || rest.starts_with("https://")) {
            let len = bare_len(rest);
            let url = &rest[..len];
            if looks(url).is_some() {
                // Angle brackets round it (`<https://…>`) go with it.
                let wrapped = out.ends_with('<') && rest[len..].starts_with('>');
                let long = url.len() > SHORT;
                push(&mut links, "", url);
                if long {
                    taken = true;
                    close_gap(&mut out, &text[i + len..]);
                    if wrapped {
                        out.pop();
                        i += 1;
                    }
                } else {
                    out.push_str(url);
                }
                i += len;
                continue;
            }
        }
        let c = rest.chars().next().unwrap_or_default();
        out.push(c);
        i += c.len_utf8();
    }
    let text = if taken || inline { tidy(&out) } else { text.trim().to_owned() };
    Extracted { text, links }
}

/// `text` for a line that only names the ask (a list row, a summary): its
/// long links taken out; as written when nothing else is left.
pub fn words(text: &str) -> String {
    let found = extract(text);
    if found.text.is_empty() { text.trim().to_owned() } else { found.text }
}

/// A space left before the punctuation that follows a cut-out address goes.
fn close_gap(out: &mut String, after: &str) {
    if after.starts_with(['.', ',', ';', '!', '?', ')', ']']) {
        out.truncate(out.trim_end_matches(' ').len());
    }
}

fn push(links: &mut Vec<Link>, label: &str, url: &str) {
    match links.iter_mut().find(|l| l.url == url) {
        Some(known) if known.label.is_empty() => known.label = label.trim().to_owned(),
        Some(_) => {}
        None => links.push(Link { label: label.trim().to_owned(), url: url.to_owned() }),
    }
}

/// `[label](https://…)` at the start of `s`: the label, the address, and how
/// many bytes it takes.
fn markdown_link(s: &str) -> Option<(&str, &str, usize)> {
    let close = s.find(']')?;
    let label = &s[1..close];
    if label.contains(['[', '\n']) {
        return None;
    }
    let after = s[close + 1..].strip_prefix('(')?;
    let mut depth = 1usize;
    let mut end = None;
    for (at, c) in after.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(at);
                    break;
                }
            }
            c if c.is_whitespace() => return None,
            _ => {}
        }
    }
    let url = after[..end?].trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) || looks(url).is_none() {
        return None;
    }
    Some((label, url, close + 2 + end? + 1))
}

/// How much of `s` (which starts with `http`) is the address: up to a space or
/// a quote, less the punctuation that ends a sentence (and a closing bracket
/// the address never opened).
fn bare_len(s: &str) -> usize {
    let mut end = s.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`' | '\u{201d}' | '\u{300d}')).unwrap_or(s.len());
    loop {
        let url = &s[..end];
        let Some(last) = url.chars().next_back() else { return end };
        let drop = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '*' | '\u{3002}' | '\u{ff0c}' | '\u{3001}' | '\u{ff01}' | '\u{ff1f}' | '\u{2026}' | '\u{ff09}' => true,
            ')' => url.matches(')').count() > url.matches('(').count(),
            ']' => url.matches(']').count() > url.matches('[').count(),
            '}' => url.matches('}').count() > url.matches('{').count(),
            _ => false,
        };
        if !drop || end <= "https://".len() {
            return end;
        }
        end -= last.len_utf8();
    }
}

/// The words once an address was cut out of them: runs of spaces closed, an
/// empty pair of brackets gone, and a colon or dash that led to the address
/// dropped from the end of its line.
fn tidy(s: &str) -> String {
    let mut s = s.to_owned();
    for pair in ["()", "[]", "<>", "\u{ff08}\u{ff09}", "\u{300c}\u{300d}", "\"\"", "''"] {
        s = s.replace(pair, "");
    }
    let lines: Vec<String> = s
        .lines()
        .map(|line| {
            let mut words = line.split_whitespace().collect::<Vec<_>>().join(" ");
            for (gap, close) in [(" ,", ","), (" .", "."), (" )", ")"), (" ]", "]")] {
                words = words.replace(gap, close);
            }
            while let Some(last) = words.chars().next_back() {
                if matches!(last, ':' | '-' | '\u{2013}' | '\u{2014}' | ',' | ';' | '(' | '\u{ff1a}') {
                    words.pop();
                    words = words.trim_end().to_owned();
                } else {
                    break;
                }
            }
            words
        })
        .collect();
    // Blank lines (the cut's, or the paragraphs') close up to one between paragraphs.
    let mut out: Vec<&str> = Vec::new();
    for line in &lines {
        if line.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(line);
    }
    out.join("\n").trim().to_owned()
}

/// `links`, then every link `text` has that they don't: the question's own
/// links first, as the member gave them.
pub fn merged(given: &[Link], found: Vec<Link>) -> Vec<Link> {
    let mut out: Vec<Link> = given.iter().filter(|l| looks(&l.url).is_some()).cloned().collect();
    for link in found {
        match out.iter_mut().find(|l| l.url == link.url) {
            Some(known) if known.label.is_empty() => known.label = link.label,
            Some(_) => {}
            None => out.push(link),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const AWS: &str = "https://ap-northeast-2.signin.aws.amazon.com/v1/authorize?response_type=code&client_id=arn%3Aaws%3Asignin%3A%3A%3Adevtools%2Fcross-device&state=abc123&code_challenge=xyz-_~.&code_challenge_method=SHA-256";

    #[test]
    fn a_long_address_leaves_the_sentence_and_the_words_stay_whole() {
        let got = extract(&format!("Sign in to AWS at this address from any device and type the code shown here: {AWS}"));
        assert_eq!(got.text, "Sign in to AWS at this address from any device and type the code shown here");
        assert_eq!(got.links, vec![Link { label: String::new(), url: AWS.to_owned() }], "the encoded query string is kept whole");
    }

    #[test]
    fn trailing_punctuation_and_brackets_are_not_the_address() {
        let long = "https://example.com/a/very/long/path/that/goes/on/and/on/for/a/while";
        for (text, want) in [
            (format!("Go to {long}."), "Go to."),
            (format!("Go to {long}, then wait"), "Go to, then wait"),
            (format!("(see {long})"), "(see)"),
            (format!("Is it {long}?"), "Is it?"),
            (format!("<{long}>"), ""),
        ] {
            let got = extract(&text);
            assert_eq!(got.links.len(), 1, "{text}");
            assert_eq!(got.links[0].url, long, "{text}");
            assert_eq!(got.text, want, "{text}");
        }
        let wiki = "https://en.wikipedia.org/wiki/Rust_(programming_language)_and_much_more_text";
        assert_eq!(extract(&format!("See {wiki}.")).links[0].url, wiki, "a bracket the address opened stays");
    }

    #[test]
    fn a_short_address_stays_where_it_was_said_and_is_still_a_link() {
        let got = extract("Add it at https://example.com/new then say done.");
        assert_eq!(got.text, "Add it at https://example.com/new then say done.");
        assert_eq!(got.links.len(), 1);
    }

    #[test]
    fn markdown_links_give_their_label_and_a_long_one_leaves_the_words() {
        let got = extract(&format!("Open [AWS sign-in]({AWS}) and type the code."));
        assert_eq!(got.text, "Open and type the code.");
        assert_eq!(got.links, vec![Link { label: "AWS sign-in".into(), url: AWS.to_owned() }]);
        let short = extract("Read [the guide](https://a.io/g) first.");
        assert_eq!(short.text, "Read the guide first.");
        assert_eq!(short.links[0].label, "the guide");
    }

    #[test]
    fn the_same_address_twice_is_one_link_and_a_label_fills_a_bare_one() {
        let long = "https://example.com/a/very/long/path/that/goes/on/and/on/for/a/while";
        let got = extract(&format!("{long} or [there]({long})"));
        assert_eq!(got.links, vec![Link { label: "there".into(), url: long.to_owned() }]);
    }

    #[test]
    fn words_without_a_link_come_back_as_they_were() {
        let got = extract("What is the AWS account id? (12 digits)");
        assert_eq!(got.text, "What is the AWS account id? (12 digits)");
        assert!(got.links.is_empty());
        assert!(extract("ftp://x/y and javascript:alert(1) and https:// nothing").links.is_empty());
        assert!(extract("nothttps://example.com/x").links.is_empty(), "inside a word is not an address");
    }

    #[test]
    fn lines_the_cut_emptied_go_and_paragraphs_stay() {
        let got = extract(&format!("Sign in:\n{AWS}\n\nThen type the code."));
        assert_eq!(got.text, "Sign in\n\nThen type the code.");
    }

    #[test]
    fn links_look_like_what_they_are() {
        let l = looks(AWS).unwrap();
        assert_eq!((l.host.as_str(), l.service, l.sign_in), ("ap-northeast-2.signin.aws.amazon.com", Some("AWS"), true));
        let g = looks("https://www.github.com/org/repo").unwrap();
        assert_eq!((g.host.as_str(), g.service, g.sign_in), ("github.com", Some("GitHub"), false));
        let d = looks("https://example.org/device?code=1").unwrap();
        assert_eq!((d.service, d.sign_in), (None, true));
        assert!(looks("file:///etc/passwd").is_none() && looks("https:///x").is_none());
    }

    #[test]
    fn merging_keeps_the_members_own_links_first_and_adds_the_found() {
        let given = vec![Link { label: "Sign in".into(), url: "https://a.io/x".into() }, Link { label: "bad".into(), url: "javascript:1".into() }];
        let got = merged(&given, vec![Link { label: String::new(), url: "https://a.io/x".into() }, Link { label: String::new(), url: "https://b.io/y".into() }]);
        assert_eq!(got.iter().map(|l| l.url.as_str()).collect::<Vec<_>>(), ["https://a.io/x", "https://b.io/y"]);
        assert_eq!(got[0].label, "Sign in");
    }
}
