//! Token freshness and the model listing that proves a token reaches the backend.

use serde_json::Value;

use super::CODEX_BACKEND_URL;
use super::oauth::{self, with_identity};
use crate::credentials::{CredentialStore, Credentials};
use crate::error::{Error, Result};

/// Refresh with this much headroom before the token actually expires.
const REFRESH_SKEW_SECONDS: i64 = 60;

/// The Codex client level the model catalog is requested for.
///
/// The backend hides every model whose `minimal_client_version` is above the
/// `client_version` it is sent, and answers HTTP 200 with an empty list — so
/// sending equipe's own 0.0.1 looks exactly like "this account has no models".
/// This is a capability level, not an identity: equipe still names itself in
/// `originator` and `User-Agent`. Its turns use plain Responses features only,
/// and models that need more are marked hidden and filtered out below.
pub const CATALOG_CLIENT_VERSION: &str = "1.0.0";

/// Whether `cred` should be refreshed before use at time `now`.
pub fn needs_refresh(cred: &Credentials, now: i64) -> bool {
    cred.refresh.is_some()
        && cred
            .expires
            .is_some_and(|expires| expires - now <= REFRESH_SKEW_SECONDS)
}

/// Return credentials with a usable access token, refreshing and persisting if stale.
pub async fn ensure_fresh(
    client: &reqwest::Client,
    store: &CredentialStore,
    cred: Credentials,
) -> Result<Credentials> {
    if !needs_refresh(&cred, oauth::now_secs()) {
        return Ok(cred);
    }
    let refresh_token = cred.refresh.as_deref().unwrap_or_default();
    let refreshed = oauth::refresh(client, refresh_token).await?;
    store.save(&refreshed)?;
    Ok(refreshed)
}

/// Usable model ids from a listing, best first.
///
/// The Codex catalog (`{"models": [{slug, visibility, priority, …}]}`) is ranked
/// by `priority` with hidden entries dropped; other shapes keep their order.
pub fn extract_model_ids(json: &Value) -> Vec<String> {
    let list = json
        .as_array()
        .or_else(|| json.get("data").and_then(Value::as_array))
        .or_else(|| json.get("models").and_then(Value::as_array));
    let Some(list) = list else {
        return Vec::new();
    };
    let mut ranked: Vec<(i64, usize, String)> = list
        .iter()
        .enumerate()
        .filter_map(|(order, item)| match item {
            Value::String(id) => Some((i64::MAX, order, id.clone())),
            Value::Object(rec) => {
                let hidden = rec
                    .get("visibility")
                    .and_then(Value::as_str)
                    .is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "hide" | "hidden"));
                if hidden {
                    return None;
                }
                let id = ["slug", "id", "name"]
                    .iter()
                    .find_map(|key| rec.get(*key).and_then(Value::as_str))
                    .filter(|id| !id.trim().is_empty())?;
                let priority = rec.get("priority").and_then(Value::as_i64).unwrap_or(i64::MAX);
                Some((priority, order, id.trim().to_owned()))
            }
            _ => None,
        })
        .collect();
    ranked.sort();
    let mut seen = std::collections::HashSet::new();
    ranked
        .into_iter()
        .map(|(_, _, id)| id)
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

/// Ask the backend which models this account has. Proves the token works.
pub async fn list_models(
    client: &reqwest::Client,
    cred: &Credentials,
    client_version: &str,
) -> Result<Vec<String>> {
    let builder = client
        .get(format!("{CODEX_BACKEND_URL}/models"))
        .query(&[("client_version", CATALOG_CLIENT_VERSION)]);
    let res = with_identity(builder, cred, client_version).send().await?;
    let status = res.status();
    let body = res.text().await?;
    if !status.is_success() {
        return Err(Error::from_status("models request", status.as_u16(), &body));
    }
    let json: Value = serde_json::from_str(&body)
        .map_err(|_| Error::Protocol("models response was not valid JSON".into()))?;
    Ok(extract_model_ids(&json))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::AuthMethod;
    use serde_json::json;

    fn cred(refresh: Option<&str>, expires: Option<i64>) -> Credentials {
        Credentials {
            provider: "openai".into(),
            method: AuthMethod::OauthSubscription,
            access: "a".into(),
            refresh: refresh.map(str::to_owned),
            expires,
            account_id: None,
            residency: None,
        }
    }

    #[test]
    fn refreshes_inside_the_skew_window() {
        assert!(needs_refresh(&cred(Some("r"), Some(1_030)), 1_000));
        assert!(needs_refresh(&cred(Some("r"), Some(900)), 1_000));
    }

    #[test]
    fn leaves_a_fresh_token_alone() {
        assert!(!needs_refresh(&cred(Some("r"), Some(5_000)), 1_000));
    }

    #[test]
    fn cannot_refresh_without_a_refresh_token_or_known_expiry() {
        assert!(!needs_refresh(&cred(None, Some(0)), 1_000));
        assert!(!needs_refresh(&cred(Some("r"), None), 1_000));
    }

    #[test]
    fn extracts_ids_from_every_listing_shape() {
        assert_eq!(extract_model_ids(&json!(["a", "b"])), ["a", "b"]);
        assert_eq!(extract_model_ids(&json!({ "data": [{ "id": "a" }] })), ["a"]);
        assert_eq!(
            extract_model_ids(&json!({ "models": [{ "slug": "a" }, { "name": "b" }] })),
            ["a", "b"]
        );
    }

    #[test]
    fn ranks_the_codex_catalog_by_priority_and_drops_hidden_models() {
        // The shape and values the backend returned for a real Pro account.
        let catalog = json!({ "models": [
            { "slug": "gpt-6-luna", "visibility": "list", "priority": 4 },
            { "slug": "gpt-reserve", "visibility": "hide", "priority": 4 },
            { "slug": "gpt-5.6-luna", "visibility": "list", "priority": 9 },
            { "slug": "gpt-5.6-terra", "visibility": "list", "priority": 8 },
            { "slug": "codex-auto-review", "visibility": "hide", "priority": 43 },
        ]});
        assert_eq!(extract_model_ids(&catalog), ["gpt-6-luna", "gpt-5.6-terra", "gpt-5.6-luna"]);
    }

    #[test]
    fn unranked_entries_follow_ranked_ones_in_listing_order() {
        let catalog = json!({ "models": [
            { "slug": "b" }, { "slug": "a" }, { "slug": "top", "priority": 1 }, { "slug": "b" },
        ]});
        assert_eq!(extract_model_ids(&catalog), ["top", "b", "a"]);
    }

    #[test]
    fn requests_the_catalog_for_a_real_codex_client_level() {
        // 0.0.1 makes the backend hide every model behind an HTTP 200 — see the constant.
        let (major, minor) = CATALOG_CLIENT_VERSION.split_once('.').unwrap();
        assert!(major.parse::<u32>().unwrap() >= 1 || minor.parse::<u32>().unwrap() >= 155);
    }

    #[test]
    fn skips_entries_without_an_id_and_tolerates_garbage() {
        assert_eq!(
            extract_model_ids(&json!({ "models": [{ "x": 1 }, 42, null, { "id": "ok" }] })),
            ["ok"]
        );
        assert!(extract_model_ids(&json!({ "unexpected": true })).is_empty());
        assert!(extract_model_ids(&json!("nope")).is_empty());
    }
}
