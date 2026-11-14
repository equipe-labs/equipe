//! Sites' one-time codes from the person's mailbox: IMAP over TLS (port 993)
//! with an app password the person made for equipe, kept in the secret
//! store. equipe opens the inbox read-only (`EXAMINE`, `BODY.PEEK` — nothing
//! is marked read), looks only at mail that came since the sign-in started,
//! only at mail whose sender is the site's own domain, and takes only the
//! code out of it ([`super::codes::find_code`]). Nothing else of the mail is
//! kept, logged or handed to anyone.

use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader};

use super::codes;

/// What equipe looks for in the inbox.
#[derive(Debug, Clone)]
pub struct Wanted {
    /// The login's site, by host.
    pub site: String,
    /// The domain its mail comes from, when not its own.
    pub mail_from: Option<String>,
    /// Mail older than this (unix seconds) isn't this sign-in's.
    pub since: u64,
}

/// One message as fetched: who sent it, when, its subject and text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Message {
    pub from: String,
    pub at: u64,
    pub subject: String,
    pub text: String,
}

/// The code for `wanted` in `messages`: from the site's own sender, since
/// the sign-in started, the newest first.
pub fn code_in(messages: &[Message], wanted: &Wanted) -> Option<String> {
    let mut mine: Vec<&Message> = messages
        .iter()
        .filter(|m| m.at + 60 >= wanted.since)
        .filter(|m| codes::sender_domain(&m.from).is_some_and(|d| codes::sender_matches(&d, &wanted.site, wanted.mail_from.as_deref())))
        .collect();
    mine.sort_by_key(|m| std::cmp::Reverse(m.at));
    mine.into_iter().find_map(|m| codes::find_code(&format!("{}\n{}", m.subject, m.text)))
}

/// An IMAP quoted string.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// An IMAP conversation over any stream (TLS in use, a pipe in tests).
pub struct Imap<S> {
    stream: BufReader<S>,
    tag: u32,
}

/// What a command answered: its untagged lines, with each literal (`{n}`)
/// read in after the line that announced it.
type Lines = Vec<(String, Vec<Vec<u8>>)>;

impl<S: AsyncRead + AsyncWrite + Unpin> Imap<S> {
    /// Read the server's greeting.
    pub async fn open(stream: S) -> Result<Self, String> {
        let mut imap = Imap { stream: BufReader::new(stream), tag: 0 };
        let greeting = imap.line().await?;
        if !greeting.starts_with("* OK") && !greeting.starts_with("* PREAUTH") {
            return Err("the mail server didn't greet".into());
        }
        Ok(imap)
    }

    async fn line(&mut self) -> Result<String, String> {
        let mut line = Vec::new();
        let n = tokio::time::timeout(std::time::Duration::from_secs(20), self.stream.read_until(b'\n', &mut line)).await.map_err(|_| "the mail server stopped answering".to_owned())?.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("the mail server hung up".into());
        }
        Ok(String::from_utf8_lossy(&line).trim_end().to_owned())
    }

    /// Send `command`; its untagged answer, or why it failed. The command
    /// itself (which may carry the password) is never part of an error.
    async fn run(&mut self, command: &str) -> Result<Lines, String> {
        self.tag += 1;
        let tag = format!("e{}", self.tag);
        self.stream.get_mut().write_all(format!("{tag} {command}\r\n").as_bytes()).await.map_err(|e| e.to_string())?;
        self.stream.get_mut().flush().await.map_err(|e| e.to_string())?;
        let mut lines: Lines = Vec::new();
        loop {
            let line = self.line().await?;
            if let Some(rest) = line.strip_prefix(&format!("{tag} ")) {
                return if rest.starts_with("OK") { Ok(lines) } else { Err(format!("the mail server said: {}", rest.chars().take(120).collect::<String>())) };
            }
            let mut literals = Vec::new();
            let mut current = line.clone();
            // `… {123}`: that many bytes follow, then the line goes on.
            while let Some(n) = current.strip_suffix('}').and_then(|l| l.rsplit_once('{')).and_then(|(_, n)| n.parse::<usize>().ok()) {
                if n > 4 * 1024 * 1024 {
                    return Err("a message too big to read".into());
                }
                let mut buf = vec![0; n];
                self.stream.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
                literals.push(buf);
                current = self.line().await?;
            }
            lines.push((line, literals));
        }
    }

    pub async fn login(&mut self, user: &str, password: &str) -> Result<(), String> {
        self.run(&format!("LOGIN {} {}", quoted(user), quoted(password))).await.map(|_| ()).map_err(|_| "the mailbox refused the username or app password".to_owned())
    }

    /// The inbox's mail since `since` (unix seconds; IMAP searches by day),
    /// the latest `most`, read without marking anything read.
    pub async fn recent(&mut self, since: u64, most: usize) -> Result<Vec<Message>, String> {
        self.run("EXAMINE INBOX").await?;
        let day = imap_date(since.saturating_sub(24 * 3600));
        let found = self.run(&format!("UID SEARCH SINCE {day}")).await?;
        let mut uids: Vec<u64> = found.iter().filter_map(|(l, _)| l.strip_prefix("* SEARCH")).flat_map(|rest| rest.split_whitespace().filter_map(|u| u.parse().ok()).collect::<Vec<_>>()).collect();
        uids.sort_unstable();
        let latest: Vec<String> = uids.iter().rev().take(most).map(u64::to_string).collect();
        if latest.is_empty() {
            return Ok(Vec::new());
        }
        let fetched = self.run(&format!("UID FETCH {} (INTERNALDATE BODY.PEEK[HEADER.FIELDS (FROM SUBJECT CONTENT-TYPE CONTENT-TRANSFER-ENCODING)] BODY.PEEK[TEXT]<0.65536>)", latest.join(","))).await?;
        Ok(fetched
            .into_iter()
            .filter(|(l, lits)| l.starts_with("* ") && l.contains("FETCH") && lits.len() >= 2)
            .map(|(line, lits)| {
                let at = line.split_once("INTERNALDATE \"").and_then(|(_, r)| r.split_once('"')).and_then(|(d, _)| parse_internal_date(d)).unwrap_or(0);
                let headers = String::from_utf8_lossy(&lits[0]).into_owned();
                let header = |name: &str| header_value(&headers, name);
                let text = readable(&header("content-type"), &header("content-transfer-encoding"), &lits[1]);
                Message { from: header("from"), at, subject: decode_words(&header("subject")), text }
            })
            .collect())
    }

    /// Mail from `domains` (or their subdomains) since `since`, read without
    /// marking anything read — and nothing else: the server searches for the
    /// sender, only the matches' sender and date are fetched, and the text
    /// only of the newest `most` whose sender is truly one of `domains`.
    pub async fn matching(&mut self, since: u64, domains: &[String], most: usize) -> Result<Vec<Message>, String> {
        self.run("EXAMINE INBOX").await?;
        let day = imap_date(since.saturating_sub(24 * 3600));
        let domains: Vec<&String> = domains.iter().filter(|d| !d.is_empty() && !d.contains(['"', '\\', '\r', '\n'])).collect();
        let Some((last, rest)) = domains.split_last() else { return Ok(Vec::new()) };
        // `OR FROM a OR FROM b FROM c`: IMAP's OR takes two keys.
        let from = rest.iter().rev().fold(format!("FROM {}", quoted(last)), |keys, d| format!("OR FROM {} {keys}", quoted(d)));
        let found = self.run(&format!("UID SEARCH SINCE {day} {from}")).await?;
        let mut uids: Vec<u64> = found.iter().filter_map(|(l, _)| l.strip_prefix("* SEARCH")).flat_map(|rest| rest.split_whitespace().filter_map(|u| u.parse().ok()).collect::<Vec<_>>()).collect();
        uids.sort_unstable();
        let latest: Vec<String> = uids.iter().rev().take(20).map(u64::to_string).collect();
        if latest.is_empty() {
            return Ok(Vec::new());
        }
        let heads = self.run(&format!("UID FETCH {} (UID INTERNALDATE BODY.PEEK[HEADER.FIELDS (FROM SUBJECT)])", latest.join(","))).await?;
        let mut mine: Vec<(u64, Message)> = heads
            .into_iter()
            .filter(|(l, lits)| l.starts_with("* ") && l.contains("FETCH") && !lits.is_empty())
            .filter_map(|(line, lits)| {
                let uid = line.split_once("UID ").and_then(|(_, r)| r.split(|c: char| !c.is_ascii_digit()).next()).and_then(|u| u.parse::<u64>().ok())?;
                let at = line.split_once("INTERNALDATE \"").and_then(|(_, r)| r.split_once('"')).and_then(|(d, _)| parse_internal_date(d)).unwrap_or(0);
                let headers = String::from_utf8_lossy(&lits[0]).into_owned();
                let from = header_value(&headers, "from");
                let ours = codes::sender_domain(&from).is_some_and(|d| domains.iter().any(|base| d == **base || d.ends_with(&format!(".{base}"))));
                (ours && at + 60 >= since).then(|| (uid, Message { from, at, subject: decode_words(&header_value(&headers, "subject")), text: String::new() }))
            })
            .collect();
        mine.sort_by_key(|(_, m)| std::cmp::Reverse(m.at));
        mine.truncate(most);
        for (uid, message) in &mut mine {
            let body = self.run(&format!("UID FETCH {uid} (BODY.PEEK[HEADER.FIELDS (CONTENT-TYPE CONTENT-TRANSFER-ENCODING)] BODY.PEEK[TEXT]<0.65536>)")).await?;
            if let Some((_, lits)) = body.into_iter().find(|(l, lits)| l.starts_with("* ") && l.contains("FETCH") && lits.len() >= 2) {
                let headers = String::from_utf8_lossy(&lits[0]).into_owned();
                message.text = readable(&header_value(&headers, "content-type"), &header_value(&headers, "content-transfer-encoding"), &lits[1]);
            }
        }
        Ok(mine.into_iter().map(|(_, m)| m).collect())
    }

    pub async fn logout(&mut self) {
        let _ = self.run("LOGOUT").await;
    }
}

/// `13-Nov-2026` for a unix time.
fn imap_date(at: u64) -> String {
    let (y, m, d) = civil(at / 86_400);
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{d}-{}-{y}", MONTHS[(m - 1) as usize])
}

/// Days since 1970 as a civil date (Howard Hinnant's algorithm).
fn civil(days: u64) -> (i64, u32, u32) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// IMAP's `13-Nov-2026 09:12:44 +0900` as unix seconds.
fn parse_internal_date(text: &str) -> Option<u64> {
    let mut parts = text.split_whitespace();
    let date = parts.next()?;
    let time = parts.next()?;
    let zone = parts.next().unwrap_or("+0000");
    let mut dmy = date.split('-');
    let d: u32 = dmy.next()?.trim().parse().ok()?;
    let m = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"].iter().position(|n| n.eq_ignore_ascii_case(dmy.clone().next().unwrap_or_default()))? as u32 + 1;
    dmy.next();
    let y: i64 = dmy.next()?.parse().ok()?;
    let hms: Vec<i64> = time.split(':').filter_map(|p| p.parse().ok()).collect();
    if hms.len() != 3 {
        return None;
    }
    let sign = if zone.starts_with('-') { -1 } else { 1 };
    let zone = zone.trim_start_matches(['+', '-']);
    let offset = sign * (zone.get(..2)?.parse::<i64>().ok()? * 3600 + zone.get(2..4)?.parse::<i64>().ok()? * 60);
    let secs = days_from_civil(y, m, d) * 86_400 + hms[0] * 3600 + hms[1] * 60 + hms[2] - offset;
    u64::try_from(secs).ok()
}

/// A header's value (folded lines joined), lower-case name.
fn header_value(headers: &str, name: &str) -> String {
    let mut value: Option<String> = None;
    for line in headers.lines() {
        if let Some(v) = value.as_mut() {
            if line.starts_with([' ', '\t']) {
                v.push(' ');
                v.push_str(line.trim());
                continue;
            }
            break;
        }
        if let Some((key, rest)) = line.split_once(':')
            && key.trim().eq_ignore_ascii_case(name)
        {
            value = Some(rest.trim().to_owned());
        }
    }
    value.unwrap_or_default()
}

/// `=?utf-8?B?…?=` and `=?utf-8?Q?…?=` words in a subject, read.
fn decode_words(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("=?") {
        out.push_str(&rest[..start]);
        let word = &rest[start + 2..];
        let mut parts = word.splitn(3, '?');
        let (Some(_charset), Some(enc), Some(tail)) = (parts.next(), parts.next(), parts.next()) else {
            out.push_str(&rest[start..]);
            return out;
        };
        let Some(end) = tail.find("?=") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let body = &tail[..end];
        let bytes = if enc.eq_ignore_ascii_case("b") { base64_decode(body) } else { quoted_printable(&body.replace('_', " ")) };
        out.push_str(&String::from_utf8_lossy(&bytes));
        rest = &tail[end + 2..];
    }
    out.push_str(rest);
    out
}

fn base64_decode(text: &str) -> Vec<u8> {
    use base64::Engine as _;
    let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD.decode(clean.trim_end_matches('=')).or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(clean.trim_end_matches('='))).unwrap_or_default()
}

fn quoted_printable(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            if bytes.get(i + 1) == Some(&b'\r') && bytes.get(i + 2) == Some(&b'\n') {
                i += 3;
                continue;
            }
            if bytes.get(i + 1) == Some(&b'\n') {
                i += 2;
                continue;
            }
            if let Some(b) = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// HTML as the text a person would read: tags and styles out, entities in.
pub fn html_as_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0;
    while i < html.len() {
        if lower[i..].starts_with("<style") || lower[i..].starts_with("<script") {
            let close = if lower[i..].starts_with("<style") { "</style>" } else { "</script>" };
            i = lower[i..].find(close).map_or(html.len(), |e| i + e + close.len());
            continue;
        }
        let c = html[i..].chars().next().unwrap_or(' ');
        if c == '<' {
            i = html[i..].find('>').map_or(html.len(), |e| i + e + 1);
            out.push(' ');
            continue;
        }
        out.push(c);
        i += c.len_utf8();
    }
    out.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'")
}

/// A body as text: each MIME part decoded by its own encoding, HTML read
/// as text; a multipart message's parts in order.
fn readable(content_type: &str, encoding: &str, body: &[u8]) -> String {
    let lower = content_type.to_ascii_lowercase();
    if lower.starts_with("multipart/") {
        let boundary = content_type.split(';').filter_map(|p| p.trim().split_once('=')).find(|(k, _)| k.trim().eq_ignore_ascii_case("boundary")).map(|(_, v)| v.trim().trim_matches('"').to_owned());
        let Some(boundary) = boundary else { return String::from_utf8_lossy(body).into_owned() };
        let text = String::from_utf8_lossy(body);
        let marker = format!("--{boundary}");
        return text
            .split(marker.as_str())
            .skip(1)
            .filter(|part| !part.starts_with("--"))
            .filter_map(|part| {
                let part = part.trim_start_matches(['\r', '\n']);
                let (head, rest) = part.split_once("\r\n\r\n").or_else(|| part.split_once("\n\n"))?;
                Some(readable(&header_value(head, "content-type"), &header_value(head, "content-transfer-encoding"), rest.as_bytes()))
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    let bytes = match encoding.to_ascii_lowercase().trim() {
        "base64" => base64_decode(&String::from_utf8_lossy(body)),
        "quoted-printable" => quoted_printable(&String::from_utf8_lossy(body)),
        _ => body.to_vec(),
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if lower.contains("html") { html_as_text(&text) } else { text }
}

/// Connect to the mailbox over TLS: the system's trusted certificates, the
/// mailbox's own host name checked.
pub async fn connect(host: &str, port: u16) -> Result<Imap<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>, String> {
    use rustls_platform_verifier::BuilderVerifierExt as _;
    let provider = std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_platform_verifier()
        .map_err(|e| e.to_string())?
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_owned()).map_err(|_| format!("{host} isn't a host name"))?;
    let tcp = tokio::time::timeout(std::time::Duration::from_secs(15), tokio::net::TcpStream::connect((host, port))).await.map_err(|_| format!("{host} didn't answer"))?.map_err(|e| e.to_string())?;
    let tls = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config)).connect(name, tcp).await.map_err(|e| format!("TLS with {host}: {e}"))?;
    Imap::open(tls).await
}

/// Look once in the mailbox for `wanted`'s code.
pub async fn look_once(host: &str, port: u16, user: &str, password: &str, wanted: &Wanted) -> Result<Option<String>, String> {
    let mut imap = connect(host, port).await?;
    imap.login(user, password).await?;
    // Only the site's own mail is fetched, not the inbox's latest.
    let mut domains = vec![codes::registrable(&wanted.site)];
    domains.extend(wanted.mail_from.as_deref().map(|m| m.trim().trim_start_matches('@').to_ascii_lowercase()).filter(|m| !m.is_empty()));
    let messages = imap.matching(wanted.since, &domains, 3).await;
    imap.logout().await;
    Ok(code_in(&messages?, wanted))
}

/// Look once in the mailbox for mail from `domains` since `since`: only
/// those messages ([`Imap::matching`]).
pub async fn matching_once(host: &str, port: u16, user: &str, password: &str, since: u64, domains: &[String]) -> Result<Vec<Message>, String> {
    let mut imap = connect(host, port).await?;
    imap.login(user, password).await?;
    let messages = imap.matching(since, domains, crate::verify::gmail::READ_AT_MOST).await;
    imap.logout().await;
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_and_write_as_imap_has_them() {
        assert_eq!(imap_date(1_794_528_000 + 3_600), "13-Nov-2026");
        assert_eq!(parse_internal_date("13-Nov-2026 09:00:00 +0900"), Some(1_794_528_000));
        assert_eq!(parse_internal_date(" 1-Jan-1970 00:00:10 +0000"), Some(10));
    }

    #[test]
    fn a_multipart_mail_is_read_as_text_by_each_parts_encoding() {
        let body = "--b1\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\nWW91ciBjb2RlIGlzIDQ4MjkxMA==\r\n--b1\r\nContent-Type: text/html\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n<style>.a{}</style><p>Code=3A <b>482910</b></p>\r\n--b1--\r\n";
        let text = readable("multipart/alternative; boundary=\"b1\"", "", body.as_bytes());
        assert!(text.contains("Your code is 482910") && text.contains("Code: ") && !text.contains("<b>") && !text.contains(".a{}"), "{text}");
        assert_eq!(decode_words("=?UTF-8?B?7J247KadIOy9lOuTnA==?= 123"), "인증 코드 123");
    }

    #[test]
    fn only_the_sites_own_mail_since_the_sign_in_gives_a_code() {
        let wanted = Wanted { site: "github.com".into(), mail_from: None, since: 1_000 };
        let m = |from: &str, at: u64, text: &str| Message { from: from.into(), at, subject: String::new(), text: text.into() };
        assert_eq!(code_in(&[m("GitHub <noreply@github.com>", 1_010, "Your code: 111222")], &wanted).as_deref(), Some("111222"));
        assert_eq!(code_in(&[m("evil <x@github.com.evil.io>", 1_010, "Your code: 111222")], &wanted), None, "another sender");
        assert_eq!(code_in(&[m("noreply@github.com", 100, "Your code: 111222")], &wanted), None, "from before the sign-in");
        let newest = [m("noreply@github.com", 1_010, "Your code: 111222"), m("noreply@github.com", 1_020, "Your code: 333444")];
        assert_eq!(code_in(&newest, &wanted).as_deref(), Some("333444"), "the newest");
    }

    #[tokio::test]
    async fn only_the_senders_mail_is_searched_for_and_only_its_text_is_fetched() {
        let (client, server) = tokio::io::duplex(1 << 16);
        let script = tokio::spawn(async move {
            let mut s = BufReader::new(server);
            let mut said = Vec::new();
            s.get_mut().write_all(b"* OK IMAP ready\r\n").await.unwrap();
            let gh = "From: GitHub <noreply@github.com>\r\nSubject: [GitHub] Sudo email verification code\r\n\r\n";
            let evil = "From: x@github.com.evil.io\r\nSubject: code\r\n\r\n";
            let heads = format!(
                "* 1 FETCH (UID 9 INTERNALDATE \"13-Nov-2026 09:00:00 +0000\" BODY[HEADER.FIELDS (FROM SUBJECT)] {{{}}}\r\n{gh})\r\n* 2 FETCH (UID 11 INTERNALDATE \"13-Nov-2026 09:00:30 +0000\" BODY[HEADER.FIELDS (FROM SUBJECT)] {{{}}}\r\n{evil})\r\nTAG OK done\r\n",
                gh.len(),
                evil.len()
            );
            let (h, t) = ("Content-Type: text/plain\r\n\r\n", "Verification code: 48213907\r\n");
            let body = format!("* 1 FETCH (UID 9 BODY[HEADER.FIELDS (CONTENT-TYPE CONTENT-TRANSFER-ENCODING)] {{{}}}\r\n{h} BODY[TEXT]<0> {{{}}}\r\n{t})\r\nTAG OK done\r\n", h.len(), t.len());
            let replies = ["TAG OK logged in\r\n".to_owned(), "TAG OK [READ-ONLY] done\r\n".to_owned(), "* SEARCH 9 11\r\nTAG OK done\r\n".to_owned(), heads, body, "TAG OK bye\r\n".to_owned()];
            for reply in replies {
                let mut line = String::new();
                s.read_line(&mut line).await.unwrap();
                let tag = line.split_whitespace().next().unwrap().to_owned();
                said.push(line.trim_end().to_owned());
                s.get_mut().write_all(reply.replace("TAG", &tag).as_bytes()).await.unwrap();
            }
            said
        });
        let mut imap = Imap::open(client).await.unwrap();
        imap.login("me@example.com", "pw").await.unwrap();
        let since = parse_internal_date("13-Nov-2026 08:59:00 +0000").unwrap();
        let found = imap.matching(since, &["github.com".to_owned()], 3).await.unwrap();
        imap.logout().await;
        assert_eq!(found.len(), 1, "the lookalike sender is passed over: {found:?}");
        assert!(found[0].text.contains("48213907") && found[0].from.contains("noreply@github.com"));
        let said = script.await.unwrap();
        assert!(said[1].ends_with("EXAMINE INBOX"), "read-only");
        assert!(said[2].contains("UID SEARCH SINCE") && said[2].ends_with(r#"FROM "github.com""#), "searched for the sender on the server: {said:?}");
        assert!(said[3].contains("UID FETCH 11,9") && said[3].contains("BODY.PEEK[HEADER.FIELDS (FROM SUBJECT)]") && !said[3].contains("TEXT"), "heads only first: {said:?}");
        assert!(said[4].contains("UID FETCH 9 ") && said[4].contains("BODY.PEEK[TEXT]"), "the text of the sender's own mail alone: {said:?}");
    }

    #[tokio::test]
    async fn the_inbox_is_read_without_marking_anything_and_the_password_goes_quoted() {
        let (client, server) = tokio::io::duplex(1 << 16);
        let script = tokio::spawn(async move {
            let mut s = BufReader::new(server);
            let mut said = Vec::new();
            s.get_mut().write_all(b"* OK IMAP ready\r\n").await.unwrap();
            let header = "From: GitHub <noreply@github.com>\r\nSubject: [GitHub] Your code\r\n\r\n";
            let text = "Here is your code: 551234\r\n";
            let fetch = format!("* 1 FETCH (UID 9 INTERNALDATE \"13-Nov-2026 09:00:00 +0000\" BODY[HEADER.FIELDS (FROM SUBJECT)] {{{}}}\r\n{header} BODY[TEXT]<0> {{{}}}\r\n{text})\r\nTAG OK done\r\n", header.len(), text.len());
            let replies = ["TAG OK logged in\r\n".to_owned(), "* 3 EXISTS\r\nTAG OK [READ-ONLY] done\r\n".to_owned(), "* SEARCH 7 9\r\nTAG OK done\r\n".to_owned(), fetch, "* BYE\r\nTAG OK bye\r\n".to_owned()];
            for reply in replies {
                let mut line = String::new();
                s.read_line(&mut line).await.unwrap();
                let tag = line.split_whitespace().next().unwrap().to_owned();
                said.push(line.trim_end().to_owned());
                s.get_mut().write_all(reply.replace("TAG", &tag).as_bytes()).await.unwrap();
            }
            said
        });
        let mut imap = Imap::open(client).await.unwrap();
        imap.login("me@example.com", "app \"pass\"").await.unwrap();
        let since = parse_internal_date("13-Nov-2026 08:59:00 +0000").unwrap();
        let messages = imap.recent(since, 15).await.unwrap();
        imap.logout().await;
        assert_eq!(code_in(&messages, &Wanted { site: "github.com".into(), mail_from: None, since }).as_deref(), Some("551234"));
        let said = script.await.unwrap();
        assert!(said[0].ends_with(r#"LOGIN "me@example.com" "app \"pass\"""#), "{said:?}");
        assert!(said[1].ends_with("EXAMINE INBOX"), "read-only");
        assert!(said[3].contains("UID FETCH 9,7") && said[3].contains("BODY.PEEK[TEXT]"), "nothing marked read: {said:?}");
    }
}
