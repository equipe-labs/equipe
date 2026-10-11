//! The Codex OAuth client: authorization URL, token exchange and refresh, JWT
//! claims, and the identity headers every backend call carries.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::{Map, Value};
use url::Url;

use crate::credentials::{AuthMethod, Credentials};
use crate::error::{Error, Result};
use crate::pkce;

/// Public Codex OAuth client id shipped in OpenAI's open-source Codex CLI.
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const CALLBACK_PORT: u16 = 1455;
pub const CALLBACK_PATH: &str = "/auth/callback";
const SCOPE: &str = "openid profile email offline_access";

/// Honest third-party identity — never "codex" / "codex_cli_rs".
pub const ORIGINATOR: &str = "equipe";

pub struct AuthorizationRequest {
    pub url: String,
    pub verifier: String,
    pub state: String,
    pub redirect_uri: String,
}

pub fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}{CALLBACK_PATH}")
}

/// Build the URL to open in the user's browser, with fresh PKCE and state.
pub fn authorization_request(port: u16) -> AuthorizationRequest {
    let pkce = pkce::generate();
    let state = pkce::random_state();
    let redirect_uri = redirect_uri(port);
    let mut url = Url::parse(AUTHORIZE_URL).expect("AUTHORIZE_URL is a valid URL");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("originator", ORIGINATOR);
    AuthorizationRequest {
        url: url.into(),
        verifier: pkce.verifier,
        state,
        redirect_uri,
    }
}

// ── Token endpoint ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

/// Turn a token-endpoint body into credentials.
///
/// `sent_refresh` is the refresh token we just used, kept when the server does
/// not rotate it.
pub fn credentials_from_token_body(
    body: &str,
    sent_refresh: Option<&str>,
    now: i64,
) -> Result<Credentials> {
    let token: TokenResponse = serde_json::from_str(body)
        .map_err(|_| Error::Protocol("token response was not valid JSON".into()))?;
    let access = token
        .access_token
        .filter(|t| !t.is_empty())
        .ok_or_else(|| Error::Protocol("token response missing access_token".into()))?;
    let claims = decode_auth_claims(&access);
    Ok(Credentials {
        provider: "openai".into(),
        method: AuthMethod::OauthSubscription,
        refresh: token
            .refresh_token
            .filter(|t| !t.is_empty())
            .or_else(|| sent_refresh.map(str::to_owned)),
        expires: token.expires_in.map(|secs| now + secs),
        account_id: claims.account_id,
        residency: claims.residency,
        access,
    })
}

async fn post_token_form(client: &reqwest::Client, form: &[(&str, &str)]) -> Result<Credentials> {
    let res = client.post(TOKEN_URL).form(form).send().await?;
    let status = res.status();
    let body = res.text().await?;
    if !status.is_success() {
        return Err(Error::from_status("token request", status.as_u16(), &body));
    }
    let sent_refresh = form
        .iter()
        .find(|(k, _)| *k == "refresh_token")
        .map(|(_, v)| *v);
    credentials_from_token_body(&body, sent_refresh, now_secs())
}

/// Exchange an authorization code for tokens.
pub async fn exchange_code(
    client: &reqwest::Client,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<Credentials> {
    post_token_form(
        client,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ],
    )
    .await
}

/// Get a new access token from a refresh token.
pub async fn refresh(client: &reqwest::Client, refresh_token: &str) -> Result<Credentials> {
    post_token_form(
        client,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", refresh_token),
        ],
    )
    .await
}

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ── JWT claims ──────────────────────────────────────────────────────────────

/// Workspace claims the Codex backend derives from the OAuth JWT.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AuthClaims {
    /// `chatgpt_account_id` — sent as the `ChatGPT-Account-ID` header.
    pub account_id: Option<String>,
    /// `chatgpt_data_residency`, falling back to `chatgpt_compute_residency`.
    /// Without the matching header, residency-enforced workspaces answer 401
    /// "Workspace is not authorized in this region" — which reads like a bad
    /// token but is not one.
    pub residency: Option<String>,
}

/// Decode the workspace claims from an access-token JWT (best effort).
///
/// A malformed token yields empty claims rather than an error, so the request
/// still goes out and surfaces as a 401 instead of failing at header assembly.
pub fn decode_auth_claims(access_token: &str) -> AuthClaims {
    let Some(payload) = access_token.split('.').nth(1).filter(|p| !p.is_empty()) else {
        return AuthClaims::default();
    };
    let Ok(bytes) = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')) else {
        return AuthClaims::default();
    };
    let Ok(Value::Object(top)) = serde_json::from_slice::<Value>(&bytes) else {
        return AuthClaims::default();
    };
    // Claims appear either at the top level or namespaced under the auth claim.
    let nested = top
        .get("https://api.openai.com/auth")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let pick = |key: &str| non_empty(&top, key).or_else(|| non_empty(&nested, key));
    AuthClaims {
        account_id: pick("chatgpt_account_id"),
        residency: pick("chatgpt_data_residency").or_else(|| pick("chatgpt_compute_residency")),
    }
}

fn non_empty(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

// ── Request identity ────────────────────────────────────────────────────────

/// Identity headers for calls to the ChatGPT backend — honest, non-impersonating.
pub fn identity_headers(cred: &Credentials, version: &str) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        ("Authorization", format!("Bearer {}", cred.access)),
        ("originator", ORIGINATOR.to_owned()),
        ("User-Agent", format!("equipe/{version}")),
    ];
    if let Some(account) = &cred.account_id {
        headers.push(("ChatGPT-Account-ID", account.clone()));
    }
    // Credentials stored before residency was tracked still carry it in the JWT.
    let residency = cred
        .residency
        .clone()
        .or_else(|| decode_auth_claims(&cred.access).residency);
    if let Some(residency) = residency {
        headers.push(("x-openai-internal-codex-residency", residency));
    }
    headers
}

pub(crate) fn with_identity(
    mut builder: reqwest::RequestBuilder,
    cred: &Credentials,
    version: &str,
) -> reqwest::RequestBuilder {
    for (name, value) in identity_headers(cred, version) {
        builder = builder.header(name, value);
    }
    builder
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// An access token whose payload carries `claims`. The signature is irrelevant here.
    fn jwt(claims: Value) -> String {
        format!("header.{}.signature", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    fn credential(access: String) -> Credentials {
        Credentials {
            provider: "openai".into(),
            method: AuthMethod::OauthSubscription,
            access,
            refresh: None,
            expires: None,
            account_id: None,
            residency: None,
        }
    }

    fn header<'a>(headers: &'a [(&'static str, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }

    // ── Authorization request ──

    #[test]
    fn authorization_request_carries_pkce_state_and_an_honest_originator() {
        let req = authorization_request(CALLBACK_PORT);
        let url = Url::parse(&req.url).unwrap();
        let q = |k: &str| {
            url.query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.into_owned())
        };
        assert_eq!(
            format!("{}{}", url.origin().ascii_serialization(), url.path()),
            AUTHORIZE_URL
        );
        assert_eq!(q("response_type").as_deref(), Some("code"));
        assert_eq!(q("client_id").as_deref(), Some(CLIENT_ID));
        assert_eq!(q("code_challenge_method").as_deref(), Some("S256"));
        assert_eq!(q("code_challenge"), Some(pkce::challenge_for(&req.verifier)));
        assert_eq!(q("state"), Some(req.state.clone()));
        assert_eq!(q("redirect_uri"), Some(req.redirect_uri.clone()));
        // The verifier must never leave the process — only its hash goes on the wire.
        assert!(!req.url.contains(&req.verifier));
        assert_eq!(q("originator").as_deref(), Some(ORIGINATOR));
    }

    #[test]
    fn redirect_uri_honours_a_custom_port() {
        assert_eq!(redirect_uri(3000), "http://localhost:3000/auth/callback");
    }

    // ── Token body ──

    #[test]
    fn token_body_becomes_credentials_with_claims_and_expiry() {
        let access = jwt(json!({ "chatgpt_account_id": "acct_1", "chatgpt_data_residency": "eu" }));
        let body = json!({ "access_token": access, "refresh_token": "r2", "expires_in": 3600 }).to_string();
        let cred = credentials_from_token_body(&body, Some("r1"), 1_000).unwrap();
        assert_eq!(cred.access, access);
        assert_eq!(cred.refresh.as_deref(), Some("r2"));
        assert_eq!(cred.expires, Some(4_600));
        assert_eq!(cred.account_id.as_deref(), Some("acct_1"));
        assert_eq!(cred.residency.as_deref(), Some("eu"));
    }

    #[test]
    fn token_body_keeps_the_sent_refresh_token_when_not_rotated() {
        let body = json!({ "access_token": "a" }).to_string();
        let cred = credentials_from_token_body(&body, Some("r1"), 0).unwrap();
        assert_eq!(cred.refresh.as_deref(), Some("r1"));
        assert_eq!(cred.expires, None);
    }

    #[test]
    fn token_body_without_access_token_is_an_error() {
        let err = credentials_from_token_body(r#"{"refresh_token":"r"}"#, None, 0).unwrap_err();
        assert!(err.to_string().contains("missing access_token"));
        let err = credentials_from_token_body("not json", None, 0).unwrap_err();
        assert!(err.to_string().contains("not valid JSON"));
    }

    // ── JWT claims ──

    #[test]
    fn reads_top_level_claims() {
        let claims = decode_auth_claims(&jwt(
            json!({ "chatgpt_account_id": "acct_123", "chatgpt_data_residency": "eu" }),
        ));
        assert_eq!(claims.account_id.as_deref(), Some("acct_123"));
        assert_eq!(claims.residency.as_deref(), Some("eu"));
    }

    #[test]
    fn reads_namespaced_claims() {
        let claims = decode_auth_claims(&jwt(json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct_456",
                "chatgpt_data_residency": "us"
            }
        })));
        assert_eq!(claims.account_id.as_deref(), Some("acct_456"));
        assert_eq!(claims.residency.as_deref(), Some("us"));
    }

    #[test]
    fn falls_back_to_compute_residency() {
        let claims = decode_auth_claims(&jwt(json!({ "chatgpt_compute_residency": "eu" })));
        assert_eq!(claims.residency.as_deref(), Some("eu"));
    }

    #[test]
    fn prefers_data_residency_over_compute_residency() {
        let claims = decode_auth_claims(&jwt(
            json!({ "chatgpt_data_residency": "eu", "chatgpt_compute_residency": "us" }),
        ));
        assert_eq!(claims.residency.as_deref(), Some("eu"));
    }

    #[test]
    fn malformed_tokens_yield_empty_claims() {
        // A panic here would break header assembly; a 401 from the backend is the
        // intended failure mode instead.
        for token in ["", "not-a-jwt", "a.!!!not-base64!!!.c", "a..c", &jwt(json!({}))] {
            assert_eq!(decode_auth_claims(token), AuthClaims::default(), "token: {token}");
        }
    }

    #[test]
    fn ignores_empty_and_non_string_claims() {
        let claims = decode_auth_claims(&jwt(
            json!({ "chatgpt_account_id": "", "chatgpt_data_residency": 42 }),
        ));
        assert_eq!(claims, AuthClaims::default());
    }

    #[test]
    fn tolerates_padded_payloads() {
        let padded = format!(
            "h.{}.s",
            base64::engine::general_purpose::URL_SAFE
                .encode(json!({ "chatgpt_account_id": "acct_9" }).to_string())
        );
        assert_eq!(decode_auth_claims(&padded).account_id.as_deref(), Some("acct_9"));
    }

    // ── Request identity ──

    #[test]
    fn identifies_equipe_honestly() {
        let headers = identity_headers(&credential("tok".into()), "1.2.3");
        assert_eq!(header(&headers, "Authorization"), Some("Bearer tok"));
        assert_eq!(header(&headers, "originator"), Some("equipe"));
        assert_eq!(header(&headers, "User-Agent"), Some("equipe/1.2.3"));
    }

    #[test]
    fn omits_account_and_residency_when_unknown() {
        let headers = identity_headers(&credential(jwt(json!({}))), "1.2.3");
        assert_eq!(header(&headers, "ChatGPT-Account-ID"), None);
        assert_eq!(header(&headers, "x-openai-internal-codex-residency"), None);
    }

    #[test]
    fn sends_residency_stored_on_the_credential() {
        let mut cred = credential("tok".into());
        cred.account_id = Some("acct_1".into());
        cred.residency = Some("eu".into());
        let headers = identity_headers(&cred, "1.2.3");
        assert_eq!(header(&headers, "ChatGPT-Account-ID"), Some("acct_1"));
        assert_eq!(header(&headers, "x-openai-internal-codex-residency"), Some("eu"));
    }

    #[test]
    fn recovers_residency_from_credentials_saved_before_it_was_tracked() {
        // Without this fallback, an existing ~/.equipe credential keeps 401-ing on a
        // residency-enforced workspace until the user happens to re-login.
        let headers = identity_headers(
            &credential(jwt(json!({ "chatgpt_data_residency": "eu" }))),
            "1.2.3",
        );
        assert_eq!(header(&headers, "x-openai-internal-codex-residency"), Some("eu"));
    }
}
