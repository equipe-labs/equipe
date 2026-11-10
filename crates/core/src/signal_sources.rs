//! Signals read straight from a product's numbers (`crate::signals::Source`):
//! a count of PostHog events, App Store downloads. The daemon asks the API
//! itself, outside any member's fence, with keys from a file the person
//! saved on an `ask_secret` card; a key is never put in a command, an
//! argument, a reading or a log, and the host it is sent to comes from that
//! same file — never from what a member proposed.
//!
//! What it reads comes back as a [`Ran`] — the number on stdout, or why it
//! couldn't — and is judged like a command's.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::signals::{Ran, Source, redact};

/// PostHog's US cloud, unless the keys' file names another (`POSTHOG_HOST`).
pub const POSTHOG_HOST: &str = "https://us.posthog.com";
/// App Store Connect's API, unless the keys' file names another (`ASC_API_URL`).
pub const ASC_API: &str = "https://api.appstoreconnect.apple.com";
/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Sales reports' product types that are a first download (not an update or a re-download).
const FIRST_DOWNLOADS: [&str; 7] = ["1", "1F", "1T", "F1", "1E", "1EP", "1EU"];

/// Read `source` now; the keys' file is found under `home`.
pub async fn read(client: &reqwest::Client, source: &Source, home: &Path, now: u64) -> Ran {
    let got = match source {
        Source::Command => Err("not an API signal".to_owned()),
        Source::Posthog { project_id, event, days, keys } => match env(keys, home) {
            Ok(env) => posthog(client, *project_id, event, *days, &env).await,
            Err(why) => Err(why),
        },
        Source::AppStore { app_id, days, keys } => match env(keys, home) {
            Ok(env) => app_store(client, app_id, *days, &env, now).await,
            Err(why) => Err(why),
        },
    };
    match got {
        Ok(n) => Ran { exit: Some(0), stdout: number_text(n), ..Ran::default() },
        Err(why) => Ran { failed: Some(why), ..Ran::default() },
    }
}

fn number_text(n: f64) -> String {
    if n.fract() == 0.0 { format!("{}", n as i64) } else { format!("{n:.2}") }
}

/// The `KEY="value"` lines of the keys' file (`~/…`), as `ask_secret` writes them.
pub fn env(shown: &str, home: &Path) -> Result<HashMap<String, String>, String> {
    let (path, _) = crate::secrets::place(shown, home)?;
    let text = std::fs::read_to_string(&path).map_err(|_| format!("{shown} can't be read: save the keys there with an ask_secret card"))?;
    Ok(parse_env(&text))
}

pub fn parse_env(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        let value = match value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            Some(quoted) => {
                let mut out = String::with_capacity(quoted.len());
                let mut chars = quoted.chars();
                while let Some(c) = chars.next() {
                    out.push(if c == '\\' { chars.next().unwrap_or('\\') } else { c });
                }
                out
            }
            None => value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')).unwrap_or(value).to_owned(),
        };
        out.insert(key.trim().to_owned(), value);
    }
    out
}

fn need<'a>(env: &'a HashMap<String, String>, key: &str) -> Result<&'a str, String> {
    env.get(key).map(String::as_str).filter(|v| !v.trim().is_empty()).ok_or_else(|| format!("{key} is missing from the keys' file"))
}

fn host(env: &HashMap<String, String>, key: &str, default: &str) -> String {
    env.get(key).map(|h| h.trim().trim_end_matches('/').to_owned()).filter(|h| !h.is_empty()).unwrap_or_else(|| default.to_owned())
}

/// What an API said when it said no: its status and its own words, clipped,
/// anything like a key blanked.
fn refusal(api: &str, status: reqwest::StatusCode, body: &[u8]) -> String {
    let said = serde_json::from_slice::<Value>(body).ok().and_then(|v| {
        let detail = v.get("detail").or_else(|| v.pointer("/errors/0/detail")).or_else(|| v.get("error"));
        detail.and_then(Value::as_str).map(str::to_owned)
    });
    let said = said.unwrap_or_default();
    let said = crate::signals::one_line(&said).chars().take(160).collect::<String>();
    if said.is_empty() { format!("{api} said {}", status.as_u16()) } else { format!("{api} said {}: {}", status.as_u16(), redact(&said)) }
}

fn unreachable(api: &str, err: &reqwest::Error) -> String {
    if err.is_timeout() { format!("{api} didn't answer in {}s", TIMEOUT.as_secs()) } else { format!("{api} couldn't be reached") }
}

/// How many `event` events (all of them when empty) PostHog counted in the
/// last `days` days, by a HogQL query with the personal API key.
async fn posthog(client: &reqwest::Client, project_id: u64, event: &str, days: u32, env: &HashMap<String, String>) -> Result<f64, String> {
    let key = need(env, "POSTHOG_PERSONAL_API_KEY")?;
    let base = host(env, "POSTHOG_HOST", POSTHOG_HOST);
    let filter = if event.is_empty() { String::new() } else { format!("event = '{}' AND ", event.replace('\\', "\\\\").replace('\'', "\\'")) };
    let query = format!("SELECT count() FROM events WHERE {filter}timestamp >= now() - INTERVAL {days} DAY");
    let body = json!({ "query": { "kind": "HogQLQuery", "query": query }, "name": "equipe signal" });
    let url = format!("{base}/api/projects/{project_id}/query/");
    let resp = client
        .post(&url)
        .bearer_auth(key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| unreachable("PostHog", &e))?;
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| unreachable("PostHog", &e))?;
    if !status.is_success() {
        return Err(refusal("PostHog", status, &bytes));
    }
    let answer: Value = serde_json::from_slice(&bytes).map_err(|_| "PostHog's answer wasn't JSON".to_owned())?;
    answer.pointer("/results/0/0").and_then(Value::as_f64).ok_or_else(|| "PostHog's answer had no count".to_owned())
}

/// First downloads of `app_id` over the last `days` whole days, from App
/// Store Connect's daily sales reports. Those come a day or so late, so the
/// days end the day before yesterday (UTC); a day with no sales is 0.
async fn app_store(client: &reqwest::Client, app_id: &str, days: u32, env: &HashMap<String, String>, now: u64) -> Result<f64, String> {
    let (key_id, issuer, vendor) = (need(env, "ASC_KEY_ID")?, need(env, "ASC_ISSUER_ID")?, need(env, "ASC_VENDOR_NUMBER")?);
    let pem = match env.get("ASC_PRIVATE_KEY").filter(|k| !k.trim().is_empty()) {
        Some(pem) => pem.replace("\\n", "\n"),
        None => {
            let file = need(env, "ASC_KEY_FILE")?;
            let path = match file.strip_prefix("~/") {
                Some(rest) => std::env::var_os("HOME").map(|h| Path::new(&h).join(rest)).unwrap_or_else(|| file.into()),
                None => file.into(),
            };
            std::fs::read_to_string(&path).map_err(|_| "the App Store key (ASC_KEY_FILE) can't be read".to_owned())?
        }
    };
    let token = asc_token(key_id, issuer, &pem, now)?;
    let base = host(env, "ASC_API_URL", ASC_API);
    let today = now / 86_400;
    let mut total = 0.0;
    for back in 2..2 + u64::from(days) {
        let date = civil_date(today - back);
        let url = format!(
            "{base}/v1/salesReports?filter%5Bfrequency%5D=DAILY&filter%5BreportType%5D=SALES&filter%5BreportSubType%5D=SUMMARY&filter%5Bversion%5D=1_0&filter%5BvendorNumber%5D={vendor}&filter%5BreportDate%5D={date}"
        );
        let resp = client
            .get(&url)
            .bearer_auth(&token)
            .header(reqwest::header::ACCEPT, "application/a-gzip, application/json")
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(|e| unreachable("App Store Connect", &e))?;
        let status = resp.status();
        let bytes = resp.bytes().await.map_err(|e| unreachable("App Store Connect", &e))?;
        // No sales that day: Apple has no report for it.
        if status == reqwest::StatusCode::NOT_FOUND {
            continue;
        }
        if !status.is_success() {
            return Err(refusal("App Store Connect", status, &bytes));
        }
        total += first_downloads(&gunzip(&bytes)?, app_id);
    }
    Ok(total)
}

fn gunzip(bytes: &[u8]) -> Result<String, String> {
    use std::io::Read as _;
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(String::from_utf8_lossy(bytes).into_owned());
    }
    let mut text = String::new();
    flate2::read::GzDecoder::new(bytes).read_to_string(&mut text).map_err(|_| "App Store Connect's report couldn't be unpacked".to_owned())?;
    Ok(text)
}

/// The units of first downloads in a sales report (tab-separated, a header
/// row first), of `app_id` alone when it is given.
pub fn first_downloads(tsv: &str, app_id: &str) -> f64 {
    let mut lines = tsv.lines();
    let Some(header) = lines.next() else { return 0.0 };
    let cols: Vec<&str> = header.split('\t').map(str::trim).collect();
    let at = |name: &str| cols.iter().position(|c| *c == name);
    let (Some(units), Some(kind)) = (at("Units"), at("Product Type Identifier")) else { return 0.0 };
    let apple_id = at("Apple Identifier");
    lines
        .map(|l| l.split('\t').map(str::trim).collect::<Vec<_>>())
        .filter(|row| row.get(kind).is_some_and(|k| FIRST_DOWNLOADS.contains(k)))
        .filter(|row| app_id.is_empty() || apple_id.and_then(|i| row.get(i)).is_some_and(|id| *id == app_id))
        .filter_map(|row| row.get(units).and_then(|u| u.parse::<f64>().ok()))
        .sum()
}

/// An App Store Connect API token: an ES256 JWT good for 20 minutes.
pub fn asc_token(key_id: &str, issuer: &str, pem: &str, now: u64) -> Result<String, String> {
    use base64::Engine as _;
    use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair};
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).map(str::trim).collect();
    let der = base64::engine::general_purpose::STANDARD.decode(body).map_err(|_| "the App Store key isn't a .p8 key".to_owned())?;
    let rng = ring::rand::SystemRandom::new();
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, &rng).map_err(|_| "the App Store key isn't a .p8 key".to_owned())?;
    let header = json!({ "alg": "ES256", "kid": key_id, "typ": "JWT" });
    let claims = json!({ "iss": issuer, "iat": now, "exp": now + 20 * 60, "aud": "appstoreconnect-v1" });
    let signing = format!("{}.{}", b64.encode(header.to_string()), b64.encode(claims.to_string()));
    let sig = pair.sign(&rng, signing.as_bytes()).map_err(|_| "the App Store token couldn't be signed".to_owned())?;
    Ok(format!("{signing}.{}", b64.encode(sig.as_ref())))
}

/// `YYYY-MM-DD` of a day counted from 1970-01-01.
pub fn civil_date(days: u64) -> String {
    // Howard Hinnant's days-to-civil.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_file_reads_as_ask_secret_writes_it() {
        let env = parse_env("# keys\nPOSTHOG_PERSONAL_API_KEY=\"phx_a\\\"b\\$c\"\nexport POSTHOG_HOST=http://127.0.0.1:9\nPLAIN='x'\n");
        assert_eq!(env["POSTHOG_PERSONAL_API_KEY"], "phx_a\"b$c");
        assert_eq!(env["POSTHOG_HOST"], "http://127.0.0.1:9");
        assert_eq!(env["PLAIN"], "x");
        assert!(!env.contains_key("# keys"));
    }

    #[test]
    fn a_sales_report_counts_first_downloads_of_the_app_only() {
        let tsv = "Provider\tSKU\tUnits\tProduct Type Identifier\tApple Identifier\n\
APPLE\tapp\t5\t1\t111\nAPPLE\tapp\t3\t7\t111\nAPPLE\tapp\t2\t1F\t111\nAPPLE\tother\t9\t1\t222\nAPPLE\tapp\t4\t3\t111\n";
        assert_eq!(first_downloads(tsv, "111"), 7.0, "updates (7) and re-downloads (3) don't count");
        assert_eq!(first_downloads(tsv, ""), 16.0);
        assert_eq!(first_downloads("", "111"), 0.0);
    }

    #[test]
    fn days_are_civil_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(20_757), "2026-10-31");
        assert_eq!(civil_date(19_782), "2024-02-29");
    }

    #[test]
    fn an_app_store_token_is_an_es256_jwt_that_verifies() {
        use base64::Engine as _;
        use ring::signature::{ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _, UnparsedPublicKey};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pem = format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n", base64::engine::general_purpose::STANDARD.encode(pkcs8.as_ref()));
        let token = asc_token("KEY123", "issuer-1", &pem, 1000).unwrap();
        let parts: Vec<&str> = token.split('.').collect();
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let claims: Value = serde_json::from_slice(&b64.decode(parts[1]).unwrap()).unwrap();
        assert_eq!((claims["iss"].as_str(), claims["aud"].as_str(), claims["exp"].as_u64()), (Some("issuer-1"), Some("appstoreconnect-v1"), Some(2200)));
        let public = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap().public_key().as_ref().to_vec();
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, public).verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &b64.decode(parts[2]).unwrap()).unwrap();
        assert!(asc_token("k", "i", "not a key", 0).is_err());
    }
}
