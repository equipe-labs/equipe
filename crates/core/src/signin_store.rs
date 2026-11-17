//! The squad's sign-ins, kept: every sign-in the person finishes for a
//! member — in a sign-in view on their screen (`crate::handoff_view`), in a
//! member's browser in their hands, in their own Chrome window, or through
//! "Sign in to a site" — is kept here, per browser profile and per site, so
//! that equipe never asks for it again until the site itself ends it.
//!
//! - Members' browsers start from it: whatever a profile's Chrome lacks of it
//!   (a session cookie a clean exit dropped, a profile folder made anew) goes
//!   back in when the browser starts and when a lane opens on it — never
//!   over a cookie the browser holds already, which may be fresher (a site
//!   rotates its cookies, and an older copy written over a newer one signs the
//!   browser out).
//! - Before a member asks the person to sign in, the kept sign-in for that
//!   site is put back into its browser once; only if the site still asks is
//!   the person asked — the site ended it, and its entry here goes.
//! - Profiles stay apart: a named profile is another person's accounts
//!   (`browser::split_profile`), and its sign-ins never reach another profile.
//!
//! - One store for every squad on the machine, by default: the person signs
//!   in once anywhere in equipe. A squad can keep a store of its own instead
//!   (`equipe logins share <squad> off`, [`set_keeps_own`]) — one machine may
//!   work for different companies. Squads' own files from before the shared
//!   store are moved into it, sealed to sealed, by [`migrate`].
//!
//! On disk it is one sealed file (`sign-ins.sealed` in equipe's home, or
//! `squads/<squad>/sign-ins.sealed` for a squad that keeps its own; readable
//! by the person alone): ChaCha20-Poly1305 under a key of its own
//! kept in the system's secret store (`crate::vault`: the Keychain on a Mac).
//! It is not under `tmp`, so no cleanup of members' scratch folders reaches
//! it. A cookie's value is never printed (`HandoffCookie`'s `Debug` leaves it
//! out) and never logged.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use equipe_protocol::HandoffCookie;
use serde::{Deserialize, Serialize};

/// A session cookie (no expiry date) is kept this long after its sign-in:
/// the site decides sooner if it ends the session, and nothing stays forever.
pub const SESSION_KEEP: u64 = 30 * 24 * 60 * 60;
/// Two sign-ins to one site this close together are one, told twice.
pub const SAME_SIGN_IN: u64 = 10 * 60;
/// Sites kept per profile, at most: the oldest go first.
pub const MAX_SITES: usize = 200;
/// What a sealed file starts with, and is bound to.
const MAGIC: &[u8] = b"equipe-sign-ins-1";

/// The store every squad shares unless it keeps its own: its name where a
/// store is named (`store` below is this, or a squad's name).
pub const SHARED: &str = "";

/// The secret store's name for a store's key.
pub fn key_account(store: &str) -> String {
    if store == SHARED { "sign-ins".to_owned() } else { format!("sign-ins.{store}") }
}

/// Where a store's sign-ins are kept.
pub fn path(home: &Path, store: &str) -> PathBuf {
    if store == SHARED { home.join("sign-ins.sealed") } else { crate::journal::squad_dir(home, store).join("sign-ins.sealed") }
}

/// The mark of a squad that keeps its sign-ins to itself.
fn own_mark(home: &Path, squad: &str) -> PathBuf {
    crate::journal::squad_dir(home, squad).join("sign-ins.own")
}

/// Whether `squad` keeps a store of its own rather than the shared one.
pub fn keeps_own(home: &Path, squad: &str) -> bool {
    !squad.is_empty() && own_mark(home, squad).is_file()
}

/// Let `squad` keep a store of its own (`own`), or share the machine's again.
/// What it kept on its own goes into the shared store when it shares again ([`migrate`]).
pub fn set_keeps_own(home: &Path, squad: &str, own: bool) -> std::io::Result<()> {
    let mark = own_mark(home, squad);
    if own {
        if let Some(dir) = mark.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(mark, "")
    } else {
        match std::fs::remove_file(mark) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// The store `squad`'s sign-ins are in: the shared one, or its own.
pub fn store_of(home: &Path, squad: &str) -> String {
    if keeps_own(home, squad) { squad.to_owned() } else { SHARED.to_owned() }
}

/// What a store is called in the log.
pub fn store_name(store: &str) -> String {
    if store == SHARED { "every squad".to_owned() } else { store.to_owned() }
}

/// Where a squad's own file goes once moved into the shared store.
fn moved(home: &Path, squad: &str) -> PathBuf {
    path(home, squad).with_extension("sealed.moved")
}

/// One site's sign-in: when the person finished it, and its cookies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub at: u64,
    pub cookies: Vec<HandoffCookie>,
}

/// A squad's sign-ins: by profile (`""` the main one), then by site.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub profiles: BTreeMap<String, BTreeMap<String, Saved>>,
}

/// Whether a cookie still counts at `now`: its expiry not passed, or — one
/// for the session — its sign-in not older than [`SESSION_KEEP`].
fn alive(c: &HandoffCookie, at: u64, now: u64) -> bool {
    match c.expires {
        Some(t) => t > 0 && u64::try_from(t).is_ok_and(|t| t > now),
        None => now.saturating_sub(at) < SESSION_KEEP,
    }
}

fn cookie_key(c: &HandoffCookie) -> (String, String, String) {
    (c.name.clone(), c.domain.clone(), c.path.clone())
}

/// Whether the sign-in kept under `kept` is one for `site`: kept for it (or
/// a part of it), or holding a cookie of it — a sign-in begun at an identity
/// provider (`accounts.google.com`) that went on to the app carries the app's.
fn covers(kept: &str, saved: &Saved, site: &str) -> bool {
    crate::vault::serves(kept, site) || saved.cookies.iter().any(|c| crate::vault::serves(c.domain.trim_start_matches('.'), site))
}

impl Record {
    pub fn is_empty(&self) -> bool {
        self.profiles.values().all(BTreeMap::is_empty)
    }

    /// Keep the sign-in to `site` in `profile` the person just finished:
    /// what was kept for that site before is replaced.
    pub fn remember(&mut self, profile: &str, site: &str, cookies: Vec<HandoffCookie>, now: u64) {
        let site = crate::vault::site_of(site);
        let cookies: Vec<HandoffCookie> = cookies.into_iter().map(crate::handoff_view::normalize).filter(|c| !c.name.is_empty() && alive(c, now, now)).collect();
        if site.is_empty() || cookies.is_empty() {
            return;
        }
        let sites = self.profiles.entry(profile.to_owned()).or_default();
        // The same sign-in told twice (a view's cookies, then the site's own
        // read back from the browser they went into): one, the newer cookie
        // of each kept.
        let cookies = match sites.get(&site) {
            Some(before) if now.saturating_sub(before.at) < SAME_SIGN_IN => {
                let mut merged: BTreeMap<(String, String, String), HandoffCookie> = before.cookies.iter().map(|c| (cookie_key(c), c.clone())).collect();
                merged.extend(cookies.into_iter().map(|c| (cookie_key(&c), c)));
                merged.into_values().collect()
            }
            _ => cookies,
        };
        sites.insert(site, Saved { at: now, cookies });
        while sites.len() > MAX_SITES {
            let Some(oldest) = sites.iter().min_by_key(|(_, s)| s.at).map(|(k, _)| k.clone()) else { break };
            sites.remove(&oldest);
        }
    }

    /// The site ended the sign-in kept for `site` in `profile` (the one made
    /// at `made`, [`Record::for_site`]): it goes, and whatever else was kept
    /// under that site. Another site's sign-in that went through it (an app
    /// signed in to with Google, for Google) stays. Whether any went.
    pub fn forget(&mut self, profile: &str, site: &str, made: u64) -> bool {
        let Some(sites) = self.profiles.get_mut(profile) else { return false };
        let before = sites.len();
        sites.retain(|kept, saved| !(crate::vault::serves(kept, site) || (saved.at == made && covers(kept, saved, site))));
        before != sites.len()
    }

    /// Everything kept for `profile` that still counts at `now`, newest
    /// sign-in first where two hold the same cookie (Google's, for two sites
    /// signed in to through it).
    pub fn live(&self, profile: &str, now: u64) -> Vec<HandoffCookie> {
        let Some(sites) = self.profiles.get(profile) else { return Vec::new() };
        let mut by_age: Vec<&Saved> = sites.values().collect();
        by_age.sort_by_key(|s| s.at);
        let mut merged: BTreeMap<(String, String, String), HandoffCookie> = BTreeMap::new();
        for saved in by_age {
            for c in saved.cookies.iter().filter(|c| alive(c, saved.at, now)) {
                merged.insert(cookie_key(c), c.clone());
            }
        }
        merged.into_values().collect()
    }

    /// The sign-in kept for `site` in `profile`, if any of it still counts:
    /// when it was made, and its cookies that do.
    pub fn for_site(&self, profile: &str, site: &str, now: u64) -> Option<(u64, Vec<HandoffCookie>)> {
        let sites = self.profiles.get(profile)?;
        let saved = sites.iter().filter(|(kept, saved)| covers(kept, saved, site)).map(|(_, s)| s).max_by_key(|s| s.at)?;
        let cookies: Vec<HandoffCookie> = saved.cookies.iter().filter(|c| alive(c, saved.at, now)).cloned().collect();
        (!cookies.is_empty()).then_some((saved.at, cookies))
    }

    /// Another store's sign-ins taken in: per profile and site, the newer
    /// sign-in of the two stays.
    pub fn absorb(&mut self, other: Record) {
        for (profile, theirs) in other.profiles {
            let sites = self.profiles.entry(profile).or_default();
            for (site, saved) in theirs {
                if sites.get(&site).is_none_or(|mine| mine.at < saved.at) {
                    sites.insert(site, saved);
                }
            }
            while sites.len() > MAX_SITES {
                let Some(oldest) = sites.iter().min_by_key(|(_, s)| s.at).map(|(k, _)| k.clone()) else { break };
                sites.remove(&oldest);
            }
        }
    }

    /// The sites kept for `profile`, for the log.
    pub fn sites(&self, profile: &str) -> Vec<String> {
        self.profiles.get(profile).map(|s| s.keys().cloned().collect()).unwrap_or_default()
    }

    /// Drop what no longer counts at `now`: dead cookies, then empty sites and profiles.
    pub fn prune(&mut self, now: u64) {
        for sites in self.profiles.values_mut() {
            for saved in sites.values_mut() {
                let at = saved.at;
                saved.cookies.retain(|c| alive(c, at, now));
            }
            sites.retain(|_, s| !s.cookies.is_empty());
        }
        self.profiles.retain(|_, s| !s.is_empty());
    }
}

/// A new key for a squad's file.
pub fn new_key() -> Result<[u8; 32], String> {
    use ring::rand::SecureRandom as _;
    let mut key = [0u8; 32];
    ring::rand::SystemRandom::new().fill(&mut key).map_err(|_| "no randomness for a key".to_owned())?;
    Ok(key)
}

fn sealing_key(key: &[u8; 32]) -> Result<ring::aead::LessSafeKey, String> {
    let unbound = ring::aead::UnboundKey::new(&ring::aead::CHACHA20_POLY1305, key).map_err(|_| "a bad key".to_owned())?;
    Ok(ring::aead::LessSafeKey::new(unbound))
}

/// The record, sealed under `key`: the mark, a fresh nonce, the sealed text.
pub fn seal(key: &[u8; 32], record: &Record) -> Result<Vec<u8>, String> {
    use ring::rand::SecureRandom as _;
    let mut nonce = [0u8; ring::aead::NONCE_LEN];
    ring::rand::SystemRandom::new().fill(&mut nonce).map_err(|_| "no randomness for a nonce".to_owned())?;
    let mut text = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    sealing_key(key)?
        .seal_in_place_append_tag(ring::aead::Nonce::assume_unique_for_key(nonce), ring::aead::Aad::from(MAGIC), &mut text)
        .map_err(|_| "the sign-ins couldn't be sealed".to_owned())?;
    let mut out = Vec::with_capacity(MAGIC.len() + nonce.len() + text.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&text);
    Ok(out)
}

/// A sealed record, opened with `key`. Another key, or a changed byte: refused.
pub fn open(key: &[u8; 32], sealed: &[u8]) -> Result<Record, String> {
    let rest = sealed.strip_prefix(MAGIC).ok_or("not a sealed sign-ins file")?;
    if rest.len() < ring::aead::NONCE_LEN {
        return Err("a sealed sign-ins file cut short".into());
    }
    let (nonce, text) = rest.split_at(ring::aead::NONCE_LEN);
    let nonce = ring::aead::Nonce::try_assume_unique_for_key(nonce).map_err(|_| "a bad nonce".to_owned())?;
    let mut text = text.to_vec();
    let plain = sealing_key(key)?.open_in_place(nonce, ring::aead::Aad::from(MAGIC), &mut text).map_err(|_| "the sign-ins couldn't be opened with their key".to_owned())?;
    serde_json::from_slice(plain).map_err(|e| e.to_string())
}

fn encode_key(key: &[u8; 32]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(key)
}

fn decode_key(text: &str) -> Option<[u8; 32]> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(text.trim()).ok()?.try_into().ok()
}

/// Where a squad's key lives: the system's secret store (`crate::vault`), or
/// memory in tests.
pub trait Keys {
    fn get(&self, account: &str) -> Result<String, String>;
    fn set(&mut self, account: &str, value: &str) -> Result<(), String>;
}

impl Keys for crate::vault::Vault {
    fn get(&self, account: &str) -> Result<String, String> {
        self.secret(account)
    }

    fn set(&mut self, account: &str, value: &str) -> Result<(), String> {
        self.set_secret(account, Some(value))
    }
}

/// One change to a squad's file at a time, in this process.
static FILES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A squad's sign-ins as kept. No file yet: none. A file whose key can't be
/// had (the secret store refused): an error, and the file is left alone.
pub fn load(home: &Path, squad: &str, keys: &dyn Keys) -> Result<Record, String> {
    let _one = FILES.lock().map_err(|_| "the sign-ins are unusable".to_owned())?;
    read(home, squad, keys)
}

fn read(home: &Path, squad: &str, keys: &dyn Keys) -> Result<Record, String> {
    let sealed = match std::fs::read(path(home, squad)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Record::default()),
        Err(e) => return Err(e.to_string()),
    };
    let key = keys.get(&key_account(squad)).ok().and_then(|k| decode_key(&k)).ok_or("the sign-ins' key isn't in the secret store")?;
    open(&key, &sealed)
}

/// Change a squad's sign-ins and keep them: read, `change`, pruned, sealed,
/// written whole (readable by the person alone). The first sign-in kept
/// makes the squad's key. The record as kept now.
pub fn update(home: &Path, squad: &str, keys: &mut dyn Keys, now: u64, change: impl FnOnce(&mut Record)) -> Result<Record, String> {
    let _one = FILES.lock().map_err(|_| "the sign-ins are unusable".to_owned())?;
    write(home, squad, keys, now, change)
}

/// Squads' own files, of squads that share the store now, moved into the
/// shared one: each opened with its key and taken in ([`Record::absorb`]) —
/// in memory, nothing of it printed — then set aside (`sign-ins.sealed.moved`),
/// so it is moved once. A file whose key can't be had stays where it is.
/// The squads moved.
pub fn migrate(home: &Path, squads: &[String], keys: &mut dyn Keys, now: u64) -> Result<Vec<String>, String> {
    let _one = FILES.lock().map_err(|_| "the sign-ins are unusable".to_owned())?;
    let mut taken = Record::default();
    let mut from = Vec::new();
    for squad in squads.iter().filter(|q| !q.is_empty() && !keeps_own(home, q) && path(home, q).is_file()) {
        match read(home, squad, &*keys) {
            Ok(record) => {
                taken.absorb(record);
                from.push(squad.clone());
            }
            Err(why) => crate::daemon_log!("equipe: {squad}'s kept sign-ins stay its own for now: {why}"),
        }
    }
    if from.is_empty() {
        return Ok(from);
    }
    write(home, SHARED, keys, now, |r| r.absorb(taken))?;
    for squad in &from {
        let _ = std::fs::rename(path(home, squad), moved(home, squad));
    }
    Ok(from)
}

fn write(home: &Path, squad: &str, keys: &mut dyn Keys, now: u64, change: impl FnOnce(&mut Record)) -> Result<Record, String> {
    let file = path(home, squad);
    let kept_key = keys.get(&key_account(squad)).ok().and_then(|k| decode_key(&k));
    let mut record = match (read(home, squad, &*keys), file.exists()) {
        (Ok(record), _) => record,
        // Its key is gone or refused: the file stays as it is, nothing is kept.
        (Err(e), true) if kept_key.is_none() => return Err(e),
        // Damaged: kept afresh under the same key.
        (Err(_), _) => Record::default(),
    };
    change(&mut record);
    record.prune(now);
    let key = match kept_key {
        Some(key) => key,
        None => {
            let key = new_key()?;
            keys.set(&key_account(squad), &encode_key(&key))?;
            key
        }
    };
    let sealed = seal(&key, &record)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension(format!("sealed.{}", std::process::id()));
    write_private(&tmp, &sealed).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    Ok(record)
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests;
