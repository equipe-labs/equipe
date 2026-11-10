//! App Store Connect: an app's first downloads a day, from the daily sales
//! reports (one report a day, gzipped TSV). Apple reports a day late, so
//! today is never asked, and a day it has no report for yet is left out —
//! only a day it says sold nothing is 0. It keeps daily reports about a year.

use std::sync::Arc;

use tokio::sync::Semaphore;

use super::{Adapter, Answer, Fetch, Fetching, error, refused, send, unix_now};
use crate::metrics::{day_number, day_string};
use crate::protocol::metrics::{ErrorCode, FetchError, Field, MetricDef, Point, Shape, SourceKind, SourceSpec};
use crate::signal_sources::{asc_token, first_downloads};

pub struct AppStore;

const SERVICE: &str = "App Store Connect";
/// Apple's API; a test points `api_url` at its stand-in.
const API: &str = "https://api.appstoreconnect.apple.com";
/// Days of daily reports Apple keeps.
const KEEPS: u32 = 365;
/// Requests in flight at once.
const AT_ONCE: usize = 8;
/// Apple's 404 for a day it has a report for, with nothing sold in it. Any
/// other 404 ("Report is not available yet…", a day before the vendor
/// existed) means the day isn't known.
const NO_SALES: &str = "There were no sales for the date specified";

impl Adapter for AppStore {
    fn spec(&self) -> SourceSpec {
        let field = |key: &str, label: &str, secret, hint: &str| Field {
            key: key.into(),
            label: label.into(),
            secret,
            required: true,
            hint: (!hint.is_empty()).then(|| hint.into()),
        };
        SourceSpec {
            kind: SourceKind::AppStore,
            label: "App Store Connect".into(),
            fields: vec![
                field("issuer_id", "Issuer ID", false, "App Store Connect → Users and Access → Integrations → App Store Connect API"),
                field("key_id", "Key ID", false, ""),
                field("private_key", "Private key (.p8)", true, "The text of the AuthKey_….p8 file; the key needs the Sales or Finance role, or Admin"),
                field("vendor_number", "Vendor number", false, "App Store Connect → Payments and Financial Reports, top left"),
                field("app_id", "Apple ID of the app", false, "App Store Connect → the app → App Information → Apple ID"),
            ],
            metrics: vec![MetricDef { name: "downloads".into(), label: "Downloads".into(), unit: "downloads".into(), shape: Shape::Count }],
            keeps_days: Some(KEEPS),
        }
    }

    fn fetch<'a>(&'a self, fetch: Fetch<'a>) -> Fetching<'a> {
        Box::pin(async move {
            let setting = |key: &str, what: &str| {
                fetch.settings.get(key).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()).ok_or_else(|| error(ErrorCode::Other, format!("no {what}")))
            };
            let (issuer, key_id, app_id) = (setting("issuer_id", "issuer ID")?, setting("key_id", "key ID")?, setting("app_id", "app's Apple ID")?);
            let vendor = setting("vendor_number", "vendor number")?;
            if !vendor.bytes().all(|b| b.is_ascii_digit()) {
                return Err(error(ErrorCode::Other, "the vendor number is digits only"));
            }
            let pem = fetch.keys.get("private_key").filter(|k| !k.trim().is_empty()).ok_or_else(|| error(ErrorCode::Auth, "no private key"))?;
            let token = asc_token(&key_id, &issuer, &pem.replace("\\n", "\n"), unix_now()).map_err(|_| error(ErrorCode::Auth, "the private key isn't a .p8 key"))?;
            let today = day_number(fetch.today).ok_or_else(|| error(ErrorCode::Other, format!("{} isn't a day", fetch.today)))?;
            let base = fetch.settings.get("api_url").map(|u| u.trim().trim_end_matches('/').to_owned()).filter(|u| !u.is_empty()).unwrap_or_else(|| API.into());

            let gate = Arc::new(Semaphore::new(AT_ONCE));
            let (token, app_id) = (Arc::new(token), Arc::new(app_id));
            let mut asking = tokio::task::JoinSet::new();
            for back in 1..=i64::from(fetch.days.min(KEEPS)) {
                let day = day_string(today - back);
                let url = format!(
                    "{base}/v1/salesReports?filter%5Bfrequency%5D=DAILY&filter%5BreportType%5D=SALES&filter%5BreportSubType%5D=SUMMARY&filter%5Bversion%5D=1_0&filter%5BvendorNumber%5D={vendor}&filter%5BreportDate%5D={day}"
                );
                let (client, gate, token, app_id) = (fetch.client.clone(), gate.clone(), token.clone(), app_id.clone());
                asking.spawn(async move {
                    let _turn = gate.acquire_owned().await.map_err(|_| error(ErrorCode::Other, "stopped"))?;
                    let request = client.get(url).bearer_auth(token.as_str()).header(reqwest::header::ACCEPT, "application/a-gzip, application/json");
                    let answer = send(SERVICE, request).await?;
                    read_day(&answer, &day, &app_id)
                });
            }

            // A refused key or a "not now" ends the fetch (dropping the set
            // stops the rest); another failed day is left out, unless no day
            // came through at all.
            let (mut points, mut failed) = (Vec::new(), None);
            let mut answered = false;
            while let Some(done) = asking.join_next().await {
                match done.map_err(|_| error(ErrorCode::Other, "a request stopped"))? {
                    Ok(point) => {
                        answered = true;
                        points.extend(point);
                    }
                    Err(e) if matches!(e.code, ErrorCode::Auth | ErrorCode::RateLimited) => return Err(e),
                    Err(e) => {
                        failed.get_or_insert(e);
                    }
                }
            }
            if let (false, Some(e)) = (answered, failed) {
                return Err(e);
            }
            points.sort_by(|a: &Point, b| a.day.cmp(&b.day));
            Ok(points)
        })
    }
}

/// One day's answer: its point, none when Apple doesn't know the day yet, or
/// why it couldn't be read.
fn read_day(answer: &Answer, day: &str, app_id: &str) -> Result<Option<Point>, FetchError> {
    let point = |value| Some(Point { metric: "downloads".into(), day: day.into(), value, breakdown: None });
    if (200..300).contains(&answer.status) {
        let tsv = gunzip(&answer.body)?;
        if !tsv.lines().next().is_some_and(|h| h.split('\t').any(|c| c.trim() == "Product Type Identifier")) {
            return Err(error(ErrorCode::BadResponse, format!("{SERVICE}'s report for {day} wasn't a sales report")));
        }
        return Ok(point(first_downloads(&tsv, app_id)));
    }
    let said = said(&answer.body);
    if answer.status == 404 {
        return Ok(if said.as_deref().is_some_and(|s| s.contains(NO_SALES)) { point(0.0) } else { None });
    }
    let mut refusal = refused(SERVICE, answer, said, unix_now());
    match answer.status {
        401 => refusal.message.push_str(" (the key may be revoked, expired or not this issuer's)"),
        403 => refusal.message.push_str(" (the key needs the Sales or Finance role, or Admin)"),
        _ => {}
    }
    Err(refusal)
}

/// What Apple said: its first error's `detail`.
fn said(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    value.pointer("/errors/0/detail").and_then(|d| d.as_str()).map(str::to_owned)
}

/// The report's text: gunzipped when it is gzip.
fn gunzip(bytes: &[u8]) -> Result<String, FetchError> {
    use std::io::Read as _;
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(String::from_utf8_lossy(bytes).into_owned());
    }
    let mut text = String::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_string(&mut text)
        .map_err(|_| error(ErrorCode::BadResponse, format!("{SERVICE}'s report couldn't be unpacked")))?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use base64::Engine as _;
    use ring::signature::{ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _, UnparsedPublicKey};

    use super::*;
    use crate::metrics::adapters::stand_in::{Reply, Seen, serve};

    const REPORT: &[u8] = include_bytes!("fixtures/app_store/sales_2026-11-01.tsv.gz");
    const NO_SALES_BODY: &[u8] = include_bytes!("fixtures/app_store/no_sales.json");
    const NOT_YET_BODY: &[u8] = include_bytes!("fixtures/app_store/not_yet.json");
    const APP: &str = "1234567890";

    /// A fresh P-256 key as a .p8's text, and its public half.
    fn key() -> (String, Vec<u8>) {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let public = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap().public_key().as_ref().to_vec();
        let pem = format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n", base64::engine::general_purpose::STANDARD.encode(pkcs8.as_ref()));
        (pem, public)
    }

    fn settings(url: &str) -> BTreeMap<String, String> {
        [("issuer_id", "69a6de7f-0000-47e3-e053-5b8c7c11a4d1"), ("key_id", "2X9R4HXF34"), ("vendor_number", "85012345"), ("app_id", APP), ("api_url", url)]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    fn report_day(seen: &Seen) -> String {
        let at = seen.path.find("filter%5BreportDate%5D=").unwrap() + "filter%5BreportDate%5D=".len();
        seen.path[at..at + 10].to_owned()
    }

    async fn run(settings: &BTreeMap<String, String>, pem: &str, days: u32) -> Result<Vec<Point>, FetchError> {
        let keys: BTreeMap<String, String> = [("private_key".to_owned(), pem.to_owned())].into();
        let client = reqwest::Client::new();
        AppStore.fetch(Fetch { settings, keys: &keys, today: "2026-11-03", days, client: &client }).await
    }

    #[tokio::test]
    async fn days_are_first_downloads_of_the_app_and_unknown_days_are_left_out() {
        let (url, seen) = serve(|s| match report_day(s).as_str() {
            "2026-11-02" => Reply::status(404, NOT_YET_BODY),
            "2026-11-01" | "2026-10-30" => Reply::ok(REPORT),
            _ => Reply::status(404, NO_SALES_BODY),
        })
        .await;
        let (pem, public) = key();
        let points = run(&settings(&url), &pem, 5).await.unwrap();
        let got: Vec<(&str, &str, f64)> = points.iter().map(|p| (p.metric.as_str(), p.day.as_str(), p.value)).collect();
        // 9 + 3 + 2 + 3 first downloads; not the updates (7), the re-download (3) or the other app.
        assert_eq!(got, [("downloads", "2026-10-29", 0.0), ("downloads", "2026-10-30", 17.0), ("downloads", "2026-10-31", 0.0), ("downloads", "2026-11-01", 17.0)]);

        let seen = seen.lock().unwrap();
        let mut days: Vec<String> = seen.iter().map(report_day).collect();
        days.sort();
        assert_eq!(days, ["2026-10-29", "2026-10-30", "2026-10-31", "2026-11-01", "2026-11-02"], "today is never asked");
        let first = &seen[0];
        assert!(first.path.starts_with("/v1/salesReports?filter%5Bfrequency%5D=DAILY&filter%5BreportType%5D=SALES&filter%5BreportSubType%5D=SUMMARY&filter%5Bversion%5D=1_0&filter%5BvendorNumber%5D=85012345&"));
        assert_eq!(first.headers["accept"], "application/a-gzip, application/json");
        let token = first.headers["authorization"].strip_prefix("Bearer ").unwrap();
        assert!(seen.iter().all(|s| s.headers["authorization"] == first.headers["authorization"]), "one token per fetch");
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header: serde_json::Value = serde_json::from_slice(&b64.decode(parts[0]).unwrap()).unwrap();
        assert_eq!((header["alg"].as_str(), header["kid"].as_str()), (Some("ES256"), Some("2X9R4HXF34")));
        let claims: serde_json::Value = serde_json::from_slice(&b64.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["iss"].as_str(), Some("69a6de7f-0000-47e3-e053-5b8c7c11a4d1"));
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, public).verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &b64.decode(parts[2]).unwrap()).unwrap();
    }

    #[tokio::test]
    async fn a_first_fetch_asks_a_year_at_most() {
        let (url, seen) = serve(|_| Reply::status(404, NO_SALES_BODY)).await;
        let points = run(&settings(&url), &key().0, 400).await.unwrap();
        assert_eq!(seen.lock().unwrap().len(), 365);
        assert_eq!(points.len(), 365);
        assert_eq!((points[0].day.as_str(), points[364].day.as_str()), ("2025-11-03", "2026-11-02"));
    }

    #[tokio::test]
    async fn a_refused_key_is_auth_without_the_token() {
        let body = r#"{"errors":[{"status":"401","code":"NOT_AUTHORIZED","title":"Authentication credentials are missing or invalid.","detail":"Provide a properly configured and signed bearer token, and make sure that it has not expired."}]}"#;
        let (url, seen) = serve(move |_| Reply::status(401, body)).await;
        let (pem, _) = key();
        let e = run(&settings(&url), &pem, 3).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Auth);
        assert!(e.message.starts_with("App Store Connect said 401: Provide a properly configured"), "{}", e.message);
        let token = seen.lock().unwrap()[0].headers["authorization"].clone();
        assert!(!e.message.contains(token.trim_start_matches("Bearer ")) && !e.message.contains("PRIVATE KEY"));

        let (url, _) = serve(|_| Reply::status(403, r#"{"errors":[{"status":"403","code":"FORBIDDEN_ERROR","detail":"This request is forbidden for security reasons"}]}"#)).await;
        let e = run(&settings(&url), &pem, 3).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Auth);
        assert!(e.message.contains("Sales or Finance role"), "{}", e.message);
    }

    #[tokio::test]
    async fn too_many_requests_is_rate_limited_until_retry_after() {
        let (url, _) = serve(|_| Reply::status(429, r#"{"errors":[{"status":"429","code":"RATE_LIMIT_EXCEEDED","detail":"The request rate limit has been reached."}]}"#).header("Retry-After", "120")).await;
        let before = unix_now();
        let e = run(&settings(&url), &key().0, 3).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::RateLimited);
        let at = e.retry_at.unwrap();
        assert!((before + 120..=unix_now() + 120).contains(&at));
    }

    #[tokio::test]
    async fn an_answer_that_isnt_a_report_is_a_bad_response() {
        let (url, _) = serve(|_| Reply::ok(&b"<html>maintenance</html>"[..])).await;
        assert_eq!(run(&settings(&url), &key().0, 2).await.unwrap_err().code, ErrorCode::BadResponse);
        let (url, _) = serve(|_| Reply::ok(&REPORT[..20])).await;
        assert_eq!(run(&settings(&url), &key().0, 2).await.unwrap_err().code, ErrorCode::BadResponse);

        // One bad day among good ones is left out.
        let (url, _) = serve(|s| if report_day(s) == "2026-11-02" { Reply::ok(&b"garbage"[..]) } else { Reply::ok(REPORT) }).await;
        let points = run(&settings(&url), &key().0, 2).await.unwrap();
        assert_eq!(points.iter().map(|p| p.day.as_str()).collect::<Vec<_>>(), ["2026-11-01"]);
    }

    #[tokio::test]
    async fn bad_settings_send_nothing() {
        let (url, seen) = serve(|_| Reply::ok(REPORT)).await;
        let mut bad = settings(&url);
        bad.insert("vendor_number".into(), "85012345&filter%5Bx%5D=1".into());
        assert_eq!(run(&bad, &key().0, 3).await.unwrap_err().code, ErrorCode::Other);
        let e = run(&settings(&url), "-----BEGIN PRIVATE KEY-----\nnot a key\n-----END PRIVATE KEY-----", 3).await.unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (ErrorCode::Auth, "the private key isn't a .p8 key"));
        assert!(seen.lock().unwrap().is_empty());
    }
}
