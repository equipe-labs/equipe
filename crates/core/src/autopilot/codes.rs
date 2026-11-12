//! One-time codes read from the person's mail or Messages: which message is
//! a site's, and the code in it. Only a code-shaped token (4 to 8 digits,
//! or a short letters-and-digits code next to a code word) ever leaves a
//! message — and only for the login whose exact site is waiting on a code;
//! nothing else of the person's mail or messages is kept or passed on.

/// A site's registrable domain, roughly: its last two labels — three where
/// the second-to-last is a common second level under a country code
/// (`example.co.uk`, `example.com.au`, `example.co.kr`).
pub fn registrable(host: &str) -> String {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    if labels.len() <= 2 {
        return labels.join(".");
    }
    let n = labels.len();
    let second = labels[n - 2];
    let take = if labels[n - 1].len() == 2 && ["co", "com", "net", "org", "ac", "gov", "edu", "ne", "or", "go"].contains(&second) { 3 } else { 2 };
    labels[n - take.min(n)..].join(".")
}

/// The domain of an address in a `From:` line: `GitHub <noreply@github.com>`
/// → `github.com`.
pub fn sender_domain(from: &str) -> Option<String> {
    let inner = from.rsplit_once('<').map_or(from, |(_, rest)| rest.split('>').next().unwrap_or(rest));
    let (_, domain) = inner.trim().rsplit_once('@')?;
    let domain = domain.trim().trim_matches(|c: char| !c.is_ascii_alphanumeric()).to_ascii_lowercase();
    (!domain.is_empty() && domain.contains('.')).then_some(domain)
}

/// Whether mail from `domain` is the site's own: its registrable domain,
/// or a subdomain of it (`github.com` sends from `github.com`, AWS from
/// `signin.aws`) — or of the domain the person named for it (`mail_from`).
pub fn sender_matches(domain: &str, site: &str, mail_from: Option<&str>) -> bool {
    let domain = domain.to_ascii_lowercase();
    let under = |base: &str| !base.is_empty() && (domain == base || domain.ends_with(&format!(".{base}")));
    under(&registrable(site)) || mail_from.map(|m| m.trim().trim_start_matches('@').to_ascii_lowercase()).is_some_and(|m| under(&m))
}

/// A WebOTP origin-bound code (`@github.com #123456` on a message's last
/// line): the host it is bound to, and the code.
pub fn origin_bound(text: &str) -> Option<(String, String)> {
    let last = text.lines().rev().map(str::trim).find(|l| !l.is_empty())?;
    let mut words = last.split_whitespace();
    let host = words.next()?.strip_prefix('@')?.to_ascii_lowercase();
    let code = words.next()?.strip_prefix('#')?;
    (!host.is_empty() && code_shaped(code)).then(|| (host, code.to_owned()))
}

/// Whether a text message is the site's: bound to its very host (WebOTP),
/// or naming it — the brand of its domain (`github` for `github.com`) as a
/// word in the message.
pub fn sms_matches(text: &str, site: &str) -> bool {
    let site = site.trim_start_matches("www.").to_ascii_lowercase();
    if let Some((host, _)) = origin_bound(text) {
        return host.trim_start_matches("www.") == site;
    }
    let brand = registrable(&site).split('.').next().unwrap_or_default().to_owned();
    if brand.chars().count() < 3 {
        return false;
    }
    let lower = text.to_lowercase();
    lower.match_indices(&brand).any(|(at, _)| {
        let before = lower[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = lower[at + brand.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        before && after
    })
}

/// A one-time code's shape: 4 to 8 digits, or 5 to 10 letters and digits
/// with at least two digits and no lowercase (Google's `G-123456` without
/// its `G-`).
fn code_shaped(token: &str) -> bool {
    let n = token.chars().count();
    (token.chars().all(|c| c.is_ascii_digit()) && (4..=8).contains(&n))
        || ((5..=10).contains(&n) && token.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) && token.chars().filter(char::is_ascii_digit).count() >= 2)
}

/// Words that sit next to a code in a site's message, in any of equipe's
/// languages, plus the usual English ones.
fn code_words() -> Vec<String> {
    let mut words: Vec<String> = crate::i18n::every("match.code_words").iter().map(|w| w.to_lowercase()).collect();
    words.extend(["code", "otp", "pin", "verification", "verify", "one-time", "passcode", "security", "2fa", "login", "sign-in", "sign in"].map(str::to_owned));
    words
}

/// The one-time code in a message's subject and text: the WebOTP-bound one
/// if there is one; else a code-shaped token, digits first, nearest a code
/// word — never a year, a date, a price or a phone number's run of digits.
pub fn find_code(text: &str) -> Option<String> {
    if let Some((_, code)) = origin_bound(text) {
        return Some(code);
    }
    let words = code_words();
    let lower = text.to_lowercase();
    // Tokens with where they start; "123 456" and "123-456" read as one.
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut tokens: Vec<(usize, String)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (at, c) = chars[i];
        if !c.is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        let mut token = String::new();
        let mut j = i;
        while j < chars.len() {
            let c = chars[j].1;
            if c.is_ascii_alphanumeric() {
                token.push(c);
                j += 1;
            } else if (c == ' ' || c == '-') && j + 1 < chars.len() && chars[j + 1].1.is_ascii_digit() && token.chars().all(|t| t.is_ascii_digit()) && (3..=4).contains(&token.len()) {
                // A split code: "123 456", "1234-5678": digits on both sides, halves alike.
                let rest: String = chars[j + 1..].iter().map(|(_, c)| *c).take_while(char::is_ascii_digit).collect();
                if rest.len() == token.len() && (3..=4).contains(&rest.len()) && chars.get(j + 1 + rest.len()).is_none_or(|(_, c)| !c.is_ascii_alphanumeric()) {
                    token.push_str(&rest);
                    j += 1 + rest.len();
                }
                break;
            } else {
                break;
            }
        }
        // Glued to a currency sign, a slash or a colon (a price, a date, a time): not a code.
        let before = if i > 0 { Some(chars[i - 1].1) } else { None };
        let after = chars.get(j).map(|(_, c)| *c);
        let money_or_date = before.is_some_and(|b| "$€£¥₩/:.".contains(b)) || after.is_some_and(|a| "/:%".contains(a)) || (after == Some('.') && chars.get(j + 1).is_some_and(|(_, c)| c.is_ascii_digit()));
        if !money_or_date {
            tokens.push((at, token));
        }
        i = j.max(i + 1);
    }
    let year = |t: &str| t.len() == 4 && t.parse::<u32>().is_ok_and(|y| (1990..=2099).contains(&y));
    let distance = |at: usize| words.iter().filter_map(|w| lower.match_indices(w.as_str()).map(|(p, _)| p.abs_diff(at)).min()).min().unwrap_or(usize::MAX);
    let mut found: Vec<(bool, usize, String)> = tokens
        .into_iter()
        .filter(|(_, t)| code_shaped(t) && !year(t))
        .map(|(at, t)| (!t.chars().all(|c| c.is_ascii_digit()), distance(at), t))
        .filter(|(letters, d, _)| !*letters || *d < 40)
        .collect();
    found.sort();
    found.into_iter().next().map(|(_, _, t)| t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_from_the_sites_own_domain_is_its_and_nothing_else_is() {
        assert_eq!(sender_domain("GitHub <noreply@github.com>").as_deref(), Some("github.com"));
        assert_eq!(sender_domain("no-reply@signin.aws").as_deref(), Some("signin.aws"));
        assert_eq!(sender_domain("nobody"), None);
        assert!(sender_matches("github.com", "github.com", None));
        assert!(sender_matches("mail.github.com", "github.com", None));
        assert!(sender_matches("github.com", "gist.github.com", None), "a subdomain's mail comes from the site's domain");
        assert!(!sender_matches("github.com.evil.io", "github.com", None));
        assert!(!sender_matches("evilgithub.com", "github.com", None));
        assert!(sender_matches("signin.aws", "console.aws.amazon.com", Some("signin.aws")), "the domain the person named");
        assert!(sender_matches("x.example.co.uk", "login.example.co.uk", None));
        assert!(!sender_matches("other.co.uk", "login.example.co.uk", None));
        assert_eq!(registrable("a.b.example.com"), "example.com");
        assert_eq!(registrable("erp.example.co.kr"), "example.co.kr");
    }

    #[test]
    fn the_code_is_found_and_dates_prices_and_years_are_not() {
        assert_eq!(find_code("Your GitHub launch code\n\nHere is your GitHub launch code: 48291037").as_deref(), Some("48291037"));
        assert_eq!(find_code("Subject: Sign-in attempt on 2026-11-13\nYour verification code is 123 456. It expires in 10 minutes.").as_deref(), Some("123456"));
        assert_eq!(find_code("G-482910 is your Google verification code.").as_deref(), Some("482910"));
        assert_eq!(find_code("Order total $1234 on 11/13. Thanks!"), None);
        assert_eq!(find_code("Welcome back! Happy 2026."), None);
        assert_eq!(find_code("Your code: K7M2Q9").as_deref(), Some("K7M2Q9"));
        assert_eq!(find_code("Use 551234 to sign in.\n\n@github.com #551234").as_deref(), Some("551234"));
    }

    #[test]
    fn a_text_message_is_the_sites_when_bound_to_its_host_or_naming_it() {
        assert!(sms_matches("Your GitHub authentication code is 123456.", "github.com"));
        assert!(sms_matches("123456 is your code\n\n@github.com #123456", "github.com"));
        assert!(!sms_matches("123456 is your code\n\n@github.com.evil.io #123456", "github.com"), "bound to another host");
        assert!(!sms_matches("Your bank code is 123456", "github.com"));
        assert!(!sms_matches("Your notgithub code is 123456", "github.com"), "a word of its own");
        assert_eq!(origin_bound("x\n@example.com #12ab"), None, "not code-shaped");
    }
}
