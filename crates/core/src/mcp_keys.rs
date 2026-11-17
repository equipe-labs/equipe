//! The credentials equipe's outside MCP connections run on — each server's
//! OAuth tokens ([`crate::mcp_oauth`]) and a keyed server's API key
//! ([`crate::squad_mcp`]'s `McpAuth::Key`) — kept where the daemon can read
//! them with nobody at the screen.
//!
//! Why not the Keychain: the daemon reads these unattended, at any hour, on
//! a Mac nobody may sit at (a Mac mini running the squad). The Keychain asks
//! the person with a dialog whenever the reading program's identity isn't
//! one it was told to trust — after an update that shipped ad hoc, for one —
//! and an unanswered dialog left every outside server failing ("the Keychain
//! dialog for equipe was left unanswered") until someone came to that Mac.
//! These credentials are only ever used by the daemon, so they are kept in a
//! file of equipe's own instead:
//!
//! - `credentials/mcp-keys.sealed` in equipe's home: one JSON map of
//!   `"<squad>/<name>"` → value, sealed with ChaCha20-Poly1305;
//! - `credentials/mcp-keys.key`: its random 256-bit key.
//!
//! Both are readable by the person's account only (0600, in a 0700 folder),
//! and `credentials/` is one of the paths no member's fenced command may read
//! (the runtime's `deny_read`). The threat model, plainly: this protects
//! against other accounts on the Mac, against the file being copied or
//! backed up without its key, and against members' commands; it does not
//! protect against a program running as the person outside equipe's fences —
//! the same as Claude Code's and Codex's own sign-in files. What the person
//! keeps for members (`crate::squad_secrets`) and their logins stay in the
//! Keychain.
//!
//! Values that an earlier equipe kept in the Keychain move here the first
//! time the daemon reads them with the Keychain open — the person saving or
//! signing in again, or answering its dialog once — and are read from here
//! after that, whatever the Keychain says.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What a sealed file starts with, and is bound to.
const MAGIC: &[u8] = b"equipe-mcp-keys-1";

/// One read-change-write at a time in this process.
static LOCK: Mutex<()> = Mutex::new(());

fn dir(home: &Path) -> PathBuf {
    home.join("credentials")
}

/// Where the sealed values are kept.
pub fn path(home: &Path) -> PathBuf {
    dir(home).join("mcp-keys.sealed")
}

fn key_path(home: &Path) -> PathBuf {
    dir(home).join("mcp-keys.key")
}

fn account(squad: &str, name: &str) -> String {
    format!("{squad}/{name}")
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    // Written aside and moved into place: a crash never leaves half a file.
    let aside = path.with_extension("new");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&aside).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(&aside, path).map_err(|e| e.to_string())
}

/// The key, made the first time it is needed (`make`).
fn key(home: &Path, make: bool) -> Result<Option<[u8; 32]>, String> {
    use base64::Engine as _;
    match std::fs::read_to_string(key_path(home)) {
        Ok(text) => {
            let bytes = base64::engine::general_purpose::STANDARD.decode(text.trim()).map_err(|_| "equipe's MCP key file is damaged".to_owned())?;
            bytes.try_into().map(Some).map_err(|_| "equipe's MCP key file is damaged".to_owned())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && make => {
            let key = crate::signin_store::new_key()?;
            write_private(&key_path(home), base64::engine::general_purpose::STANDARD.encode(key).as_bytes())?;
            Ok(Some(key))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn sealing_key(key: &[u8; 32]) -> Result<ring::aead::LessSafeKey, String> {
    let unbound = ring::aead::UnboundKey::new(&ring::aead::CHACHA20_POLY1305, key).map_err(|_| "a bad key".to_owned())?;
    Ok(ring::aead::LessSafeKey::new(unbound))
}

fn seal(key: &[u8; 32], all: &BTreeMap<String, String>) -> Result<Vec<u8>, String> {
    use ring::rand::SecureRandom as _;
    let mut nonce = [0u8; ring::aead::NONCE_LEN];
    ring::rand::SystemRandom::new().fill(&mut nonce).map_err(|_| "no randomness for a nonce".to_owned())?;
    let mut text = serde_json::to_vec(all).map_err(|e| e.to_string())?;
    sealing_key(key)?
        .seal_in_place_append_tag(ring::aead::Nonce::assume_unique_for_key(nonce), ring::aead::Aad::from(MAGIC), &mut text)
        .map_err(|_| "the MCP keys couldn't be sealed".to_owned())?;
    Ok([MAGIC, &nonce[..], &text].concat())
}

fn open(key: &[u8; 32], sealed: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let rest = sealed.strip_prefix(MAGIC).ok_or("not equipe's MCP keys file")?;
    if rest.len() < ring::aead::NONCE_LEN {
        return Err("equipe's MCP keys file is cut short".into());
    }
    let (nonce, text) = rest.split_at(ring::aead::NONCE_LEN);
    let nonce = ring::aead::Nonce::try_assume_unique_for_key(nonce).map_err(|_| "a bad nonce".to_owned())?;
    let mut text = text.to_vec();
    let plain = sealing_key(key)?.open_in_place(nonce, ring::aead::Aad::from(MAGIC), &mut text).map_err(|_| "equipe's MCP keys couldn't be opened with their key".to_owned())?;
    serde_json::from_slice(plain).map_err(|e| e.to_string())
}

fn read_all(home: &Path) -> Result<BTreeMap<String, String>, String> {
    let sealed = match std::fs::read(path(home)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e.to_string()),
    };
    let Some(key) = key(home, false)? else { return Err("equipe's MCP key file is missing".into()) };
    open(&key, &sealed)
}

fn write_all(home: &Path, all: &BTreeMap<String, String>) -> Result<(), String> {
    if all.is_empty() {
        let _ = std::fs::remove_file(path(home));
        return Ok(());
    }
    let key = key(home, true)?.ok_or("no key for equipe's MCP keys")?;
    write_private(&path(home), &seal(&key, all)?)
}

/// The value kept under `squad`'s `name`; `None` when there is none.
pub fn get(home: &Path, squad: &str, name: &str) -> Result<Option<String>, String> {
    let _held = LOCK.lock().map_err(|_| "busy".to_owned())?;
    Ok(read_all(home)?.remove(&account(squad, name)))
}

/// Keep `value` under `squad`'s `name`.
pub fn set(home: &Path, squad: &str, name: &str, value: &str) -> Result<(), String> {
    let _held = LOCK.lock().map_err(|_| "busy".to_owned())?;
    // A damaged file is started over: what it held was unreadable anyway.
    let mut all = read_all(home).unwrap_or_default();
    all.insert(account(squad, name), value.to_owned());
    write_all(home, &all)
}

/// Forget `squad`'s `name` (gone already is fine).
pub fn delete(home: &Path, squad: &str, name: &str) -> Result<(), String> {
    let _held = LOCK.lock().map_err(|_| "busy".to_owned())?;
    let mut all = read_all(home)?;
    if all.remove(&account(squad, name)).is_some() {
        write_all(home, &all)?;
    }
    Ok(())
}

/// Whether a secret store's refusal is the Keychain's dialog — unanswered,
/// waited on, or refused — rather than "nothing kept": what a person at
/// that Mac clears once.
pub fn keychain_blocked(why: &str) -> bool {
    why == crate::keychain::UNANSWERED || why == crate::keychain::WAITING || why == crate::keychain::REFUSED
}

/// What the person is told when a credential kept by an earlier equipe is
/// behind the Keychain's dialog on `mac`: the one thing to do, either way.
pub fn blocked_detail(mac: &str, keyed: bool) -> String {
    let again = if keyed { "enter the key again in Connections" } else { "sign in again from its card" };
    format!("its saved credential is behind a Keychain dialog on {mac}: {again} (equipe then keeps it where no dialog is needed), or approve the dialog on {mac}'s screen once")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-mcp-keys-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn values_survive_a_restart_with_no_secret_store_at_all() {
        let home = home("restart");
        assert_eq!(get(&home, "acme", "TWENTY_FIRST_KEY").unwrap(), None);
        set(&home, "acme", "TWENTY_FIRST_KEY", "k-123").unwrap();
        set(&home, "acme", "mcp-oauth-mobbin", "{\"access_token\":\"at\"}").unwrap();
        set(&home, "other", "TWENTY_FIRST_KEY", "k-456").unwrap();
        // A new process reads the same files: no Keychain, no dialog.
        assert_eq!(get(&home, "acme", "TWENTY_FIRST_KEY").unwrap().as_deref(), Some("k-123"));
        assert_eq!(get(&home, "other", "TWENTY_FIRST_KEY").unwrap().as_deref(), Some("k-456"));
        delete(&home, "acme", "TWENTY_FIRST_KEY").unwrap();
        assert_eq!(get(&home, "acme", "TWENTY_FIRST_KEY").unwrap(), None);
        assert!(get(&home, "acme", "mcp-oauth-mobbin").unwrap().is_some());
    }

    #[test]
    fn the_file_holds_no_value_in_the_clear_and_only_the_person_reads_it() {
        let home = home("sealed");
        set(&home, "acme", "TWENTY_FIRST_KEY", "very-secret-value").unwrap();
        let bytes = std::fs::read(path(&home)).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("very-secret-value"));
        assert!(!String::from_utf8_lossy(&bytes).contains("TWENTY_FIRST_KEY"), "not even the names");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for file in [path(&home), key_path(&home)] {
                assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600, "{file:?}");
            }
            assert_eq!(std::fs::metadata(dir(&home)).unwrap().permissions().mode() & 0o777, 0o700);
        }
        // Another key can't open it.
        let other = crate::signin_store::new_key().unwrap();
        assert!(open(&other, &bytes).is_err());
    }

    #[test]
    fn the_keychains_dialog_is_told_apart_from_nothing_kept() {
        assert!(keychain_blocked(crate::keychain::UNANSWERED));
        assert!(keychain_blocked(crate::keychain::REFUSED));
        assert!(!keychain_blocked("nothing kept"));
        assert!(blocked_detail("mac-mini", true).contains("enter the key again"));
        assert!(blocked_detail("mac-mini", false).contains("sign in again"));
    }
}
