//! The squad from the equipe app: phones paired with this machine reach the
//! daemon through the equipe relay, every frame sealed between the two ends.
//! See `docs/remote-protocol.md`.
//!
//! - `seal`: the frames' encryption.
//! - `core`: what the daemon does with frames, events and pairing — plain state.
//! - `link`: in the daemon, the relay connection and the local socket `equipe
//!   pair` and `equipe devices` talk to.
//! - `cli`: `equipe pair`, `equipe devices`, `equipe demo`.
//! - `upload`: files a phone attaches, received in pieces.
//! - `demo`: the scripted squad `equipe demo serve` shows store reviewers.

pub mod cli;
pub mod core;
pub mod demo;
pub mod link;
pub mod seal;
pub mod upload;

use std::path::{Path, PathBuf};

use equipe_core::t;
use serde::{Deserialize, Serialize};

/// Where the relay is, unless `EQUIPE_RELAY` says otherwise.
pub const DEFAULT_RELAY: &str = "https://relay.equipe.im";

/// This machine with the relay: its id and token there, its key pair, and the
/// devices paired with it. Kept where only the person can read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub relay: String,
    pub id: String,
    pub token: String,
    pub private: String,
    pub public: String,
    #[serde(default)]
    pub devices: Vec<Device>,
}

impl Config {
    pub fn keys(&self) -> seal::Keys {
        seal::Keys { private: self.private.clone(), public: self.public.clone() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    /// Its public key, base64url.
    pub key: String,
    /// When it was paired, Unix seconds.
    pub added: u64,
    /// When it last connected or left, Unix seconds; 0 when never seen.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub seen: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

pub fn config_path(home: &Path) -> PathBuf {
    home.join("remote.json")
}

/// Where the daemon answers `equipe pair` and `equipe devices`.
pub fn local_socket(home: &Path) -> PathBuf {
    // The gateway carries the screens' pairing here too (`equipe_core::pairing`).
    equipe_core::pairing::socket_path(home)
}

pub fn load(home: &Path) -> Option<Config> {
    serde_json::from_slice(&std::fs::read(config_path(home)).ok()?).ok()
}

pub fn save(home: &Path, config: &Config) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::create_dir_all(home)?;
    let path = config_path(home);
    let part = path.with_extension("json.part");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&part)?;
    file.write_all(&serde_json::to_vec_pretty(config).expect("config serializes"))?;
    std::fs::rename(part, path)
}

pub fn relay_url(config: Option<&Config>) -> String {
    std::env::var("EQUIPE_RELAY")
        .ok()
        .filter(|r| !r.is_empty())
        .or_else(|| config.map(|c| c.relay.clone()))
        .unwrap_or_else(|| DEFAULT_RELAY.to_owned())
        .trim_end_matches('/')
        .to_owned()
}

pub fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn unb64(text: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).map_err(|_| "not base64url".to_owned())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex(&sha2::Sha256::digest(bytes))
}

/// `N` bytes from the system's randomness.
pub fn random<const N: usize>() -> [u8; N] {
    use std::io::Read as _;
    let mut bytes = [0u8; N];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).expect("the system's randomness");
    bytes
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

pub fn hostname() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| t!("remote.this_machine").into())
}

/// Register this machine with the relay: its id and token there, and a key pair.
pub async fn register(home: &Path) -> Result<Config, String> {
    let relay = relay_url(None);
    let answer = reqwest::Client::new()
        .post(format!("{relay}/v1/daemons"))
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|err| t!("remote.couldnt_reach_relay_at", relay = relay, err = err.without_url()))?;
    if !answer.status().is_success() {
        return Err(t!("remote.relay_said", relay = relay, status = answer.status()));
    }
    let body: serde_json::Value = answer.json().await.map_err(|err| err.without_url().to_string())?;
    let (Some(id), Some(token)) = (body["id"].as_str(), body["token"].as_str()) else {
        return Err(t!("remote.relay_no_id").into());
    };
    let keys = seal::generate();
    let config = Config { relay, id: id.to_owned(), token: token.to_owned(), private: keys.private, public: keys.public, devices: Vec::new() };
    save(home, &config).map_err(|err| t!("remote.couldnt_keep", err = err))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_is_kept_where_only_the_person_reads_it() {
        use std::os::unix::fs::PermissionsExt as _;
        let home = std::env::temp_dir().join(format!("equipe-remote-{}", std::process::id()));
        let keys = seal::generate();
        let config = Config { relay: "https://r".into(), id: "ab".into(), token: "t".into(), private: keys.private, public: keys.public, devices: vec![] };
        save(&home, &config).unwrap();
        assert_eq!(load(&home), Some(config));
        assert_eq!(std::fs::metadata(config_path(&home)).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(&home).unwrap();
        assert_eq!(unb64(&b64(b"\x00\xffhi")).unwrap(), b"\x00\xffhi");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_ne!(random::<16>(), random::<16>());
    }
}
