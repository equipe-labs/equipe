//! Local credential storage: `<equipe home>/credentials/<provider>.json`.
//!
//! The JSON shape matches what the TypeScript CLI wrote, so a login made with
//! either build keeps working. Files are 0600 inside a 0700 directory, and are
//! replaced by rename so a concurrent reader never sees a half-written token.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    OauthSubscription,
    ApiKey,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    pub provider: String,
    pub method: AuthMethod,
    /// Short-lived access token (a JWT for OAuth).
    pub access: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<String>,
    /// Unix epoch seconds at which `access` expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<i64>,
    /// For OAuth: the account id claim, sent back as `ChatGPT-Account-ID`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// For OAuth: the workspace's data/compute residency claim. Residency-enforced
    /// workspaces reject requests that omit the matching header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub residency: Option<String>,
}

/// `$EQUIPE_HOME`, or `~/.equipe`.
pub fn equipe_home() -> PathBuf {
    match std::env::var_os("EQUIPE_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".equipe"),
    }
}

pub struct CredentialStore {
    dir: PathBuf,
}

impl CredentialStore {
    /// A store rooted at `home` (credentials live in `home/credentials`).
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            dir: home.into().join("credentials"),
        }
    }

    pub fn default_location() -> Self {
        Self::new(equipe_home())
    }

    pub fn path(&self, provider: &str) -> PathBuf {
        self.dir.join(format!("{provider}.json"))
    }

    pub fn save(&self, cred: &Credentials) -> io::Result<PathBuf> {
        fs::create_dir_all(&self.dir)?;
        restrict(&self.dir, 0o700)?;

        let path = self.path(&cred.provider);
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(cred).map_err(io::Error::other)?;
        write_private(&tmp, &json)?;
        fs::rename(&tmp, &path)?;
        Ok(path)
    }

    /// `Ok(None)` when nobody has signed in; an error when the file exists but is
    /// unreadable or corrupt, so the caller can say so instead of "not signed in".
    pub fn load(&self, provider: &str) -> io::Result<Option<Credentials>> {
        let bytes = match fs::read(self.path(provider)) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // `mode` only applies on creation; enforce it for a leftover temp file too.
    restrict(path, 0o600)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    fs::write(path, bytes)
}

#[cfg(unix)]
fn restrict(path: &std::path::Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn restrict(_path: &std::path::Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "equipe-cred-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample() -> Credentials {
        Credentials {
            provider: "openai".into(),
            method: AuthMethod::OauthSubscription,
            access: "tok".into(),
            refresh: Some("ref".into()),
            expires: Some(1_900_000_000),
            account_id: Some("acct_1".into()),
            residency: Some("eu".into()),
        }
    }

    #[test]
    fn save_then_load_round_trips() {
        let store = CredentialStore::new(temp_home("roundtrip"));
        store.save(&sample()).unwrap();
        assert_eq!(store.load("openai").unwrap(), Some(sample()));
    }

    #[test]
    fn load_without_a_login_is_none() {
        let store = CredentialStore::new(temp_home("missing"));
        assert_eq!(store.load("openai").unwrap(), None);
    }

    #[test]
    fn corrupt_file_is_an_error_not_a_missing_login() {
        let store = CredentialStore::new(temp_home("corrupt"));
        store.save(&sample()).unwrap();
        fs::write(store.path("openai"), b"{not json").unwrap();
        let err = store.load("openai").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn reads_a_credential_written_by_the_typescript_cli() {
        // Exactly the shape src/providers/auth/store.ts wrote.
        let json = r#"{
          "provider": "openai",
          "method": "oauth-subscription",
          "access": "tok",
          "refresh": "ref",
          "expires": 1900000000,
          "accountId": "acct_1"
        }"#;
        let cred: Credentials = serde_json::from_str(json).unwrap();
        assert_eq!(cred.account_id.as_deref(), Some("acct_1"));
        assert_eq!(cred.residency, None);
        assert_eq!(cred.method, AuthMethod::OauthSubscription);
    }

    #[test]
    fn absent_fields_are_omitted_when_written() {
        let mut cred = sample();
        cred.residency = None;
        let json = serde_json::to_string(&cred).unwrap();
        assert!(!json.contains("residency"));
        assert!(json.contains("\"accountId\""));
    }

    #[cfg(unix)]
    #[test]
    fn credential_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let store = CredentialStore::new(temp_home("perms"));
        let path = store.save(&sample()).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
    }
}
