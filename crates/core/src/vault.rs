//! Logins for members to sign in with (`protocol::vault`). What a screen may
//! see — site, username, note, where it may be used without asking, whether
//! equipe signs in with it by itself, its latest uses — is kept in
//! `vault.json` in equipe's home (readable by the person only); the password
//! only in the daemon machine's secret store ([`SecretStore`]: the Keychain
//! on macOS, Credential Manager on Windows, the Secret Service on Linux —
//! and none at all, never a file, where there is no such store). A member
//! never gets it: [`fill_script`] is run in the member's browser by equipe,
//! which types it in and submits the form, and the member hears only how it
//! went.
//!
//! Signing in by itself ([`auto_login_for`]) is narrower than a member's
//! `sign_in`: only a login the person turned it on for, only on its site's
//! exact host (or its sign-in page's), only over https, and the fill script
//! checks the page's host again at the moment it types.

use std::path::{Path, PathBuf};

use crate::protocol::vault::{EVERY_SQUAD, Login, LoginDraft, Use};

/// The service the passwords are kept under, in every secret store.
pub const SERVICE: &str = "equipe.vault";
/// Uses kept per login, the latest.
const USES_KEPT: usize = 20;

/// Where passwords go: the system's secret store, or (in tests) memory. One
/// interface for every platform; a platform without a store refuses — a
/// password is never written to a file instead.
pub trait Secrets: Send {
    fn set(&mut self, id: u32, password: &str) -> Result<(), String>;
    fn get(&self, id: u32) -> Result<String, String>;
    fn delete(&mut self, id: u32) -> Result<(), String>;
    /// Whether a password can be kept here at all: offering to keep a login
    /// where it can't be would only end in an error.
    fn available(&self) -> bool {
        true
    }
}

/// The daemon machine's own secret store.
pub struct SecretStore;

/// The secret store of the machine the daemon runs on.
pub fn system_store() -> Box<dyn Secrets> {
    Box::new(SecretStore)
}

#[cfg(target_os = "macos")]
impl Secrets for SecretStore {
    fn set(&mut self, id: u32, password: &str) -> Result<(), String> {
        security_framework::passwords::set_generic_password(SERVICE, &id.to_string(), password.as_bytes()).map_err(|e| e.to_string())
    }

    fn get(&self, id: u32) -> Result<String, String> {
        let bytes = security_framework::passwords::get_generic_password(SERVICE, &id.to_string()).map_err(|e| e.to_string())?;
        String::from_utf8(bytes).map_err(|_| "the kept password isn't text".into())
    }

    fn delete(&mut self, id: u32) -> Result<(), String> {
        match security_framework::passwords::delete_generic_password(SERVICE, &id.to_string()) {
            Ok(()) => Ok(()),
            // Gone already: nothing to take away.
            Err(e) if e.code() == -25300 => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Windows: Credential Manager, a generic credential per login
/// (`equipe.vault/<id>`), kept for this user on this machine.
#[cfg(windows)]
mod credential_manager {
    use windows_sys::Win32::Security::Credentials::{CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW};

    /// "Element not found": no credential of that name.
    const NOT_FOUND: i32 = 1168;

    fn target(id: u32) -> Vec<u16> {
        format!("{}/{id}", super::SERVICE).encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn last_error() -> String {
        std::io::Error::last_os_error().to_string()
    }

    pub fn set(id: u32, password: &str) -> Result<(), String> {
        let mut name = target(id);
        let mut blob = password.as_bytes().to_vec();
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: name.as_mut_ptr(),
            CredentialBlobSize: u32::try_from(blob.len()).map_err(|_| "the password is too long".to_owned())?,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        // SAFETY: every pointer in `credential` points into a buffer alive for the call.
        let ok = unsafe { CredWriteW(&credential, 0) } != 0;
        blob.fill(0);
        if ok { Ok(()) } else { Err(last_error()) }
    }

    pub fn get(id: u32) -> Result<String, String> {
        let name = target(id);
        let mut found: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `name` is a NUL-terminated wide string; on success `found` is
        // Windows' own buffer, read once and handed back with `CredFree`.
        unsafe {
            if CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut found) == 0 || found.is_null() {
                return Err(last_error());
            }
            let c = &*found;
            let bytes = if c.CredentialBlob.is_null() { Vec::new() } else { std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec() };
            CredFree(found as *const core::ffi::c_void);
            String::from_utf8(bytes).map_err(|_| "the kept password isn't text".into())
        }
    }

    pub fn delete(id: u32) -> Result<(), String> {
        let name = target(id);
        // SAFETY: `name` is a NUL-terminated wide string alive for the call.
        if unsafe { CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        // Gone already: nothing to take away.
        if error.raw_os_error() == Some(NOT_FOUND) { Ok(()) } else { Err(error.to_string()) }
    }
}

#[cfg(windows)]
impl Secrets for SecretStore {
    fn set(&mut self, id: u32, password: &str) -> Result<(), String> {
        credential_manager::set(id, password)
    }

    fn get(&self, id: u32) -> Result<String, String> {
        credential_manager::get(id)
    }

    fn delete(&mut self, id: u32) -> Result<(), String> {
        credential_manager::delete(id)
    }
}

/// Linux (and other Unixes with a desktop session): the Secret Service —
/// GNOME Keyring, KWallet — through libsecret's `secret-tool`. The password
/// goes over its standard input and comes back over its standard output,
/// never on a command line. No `secret-tool`: no logins, said so.
#[cfg(all(unix, not(target_os = "macos")))]
mod secret_service {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    const TOOL: &str = "secret-tool";

    pub fn available() -> bool {
        std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(TOOL).is_file()))
    }

    fn missing() -> String {
        "logins need the system's secret store (the Secret Service, through libsecret's `secret-tool`), which this machine doesn't have".into()
    }

    fn attributes(id: u32) -> [String; 4] {
        ["service".into(), super::SERVICE.into(), "account".into(), id.to_string()]
    }

    pub fn set(id: u32, password: &str) -> Result<(), String> {
        if !available() {
            return Err(missing());
        }
        let mut child = Command::new(TOOL)
            .arg("store")
            .arg(format!("--label=equipe login {id}"))
            .args(attributes(id))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(password.as_bytes()).map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() { Ok(()) } else { Err(format!("the secret store refused it: {}", String::from_utf8_lossy(&out.stderr).trim())) }
    }

    pub fn get(id: u32) -> Result<String, String> {
        if !available() {
            return Err(missing());
        }
        let out = Command::new(TOOL).arg("lookup").args(attributes(id)).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
        if !out.status.success() || out.stdout.is_empty() {
            return Err("no password kept".into());
        }
        String::from_utf8(out.stdout).map_err(|_| "the kept password isn't text".into())
    }

    pub fn delete(id: u32) -> Result<(), String> {
        if !available() {
            return Ok(());
        }
        // Gone already is fine too: `clear` of nothing succeeds.
        Command::new(TOOL).arg("clear").args(attributes(id)).stdin(Stdio::null()).output().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
impl Secrets for SecretStore {
    fn set(&mut self, id: u32, password: &str) -> Result<(), String> {
        secret_service::set(id, password)
    }

    fn get(&self, id: u32) -> Result<String, String> {
        secret_service::get(id)
    }

    fn delete(&mut self, id: u32) -> Result<(), String> {
        secret_service::delete(id)
    }

    fn available(&self) -> bool {
        secret_service::available()
    }
}

#[cfg(not(any(unix, windows)))]
impl Secrets for SecretStore {
    fn set(&mut self, _: u32, _: &str) -> Result<(), String> {
        Err("logins need the system's secret store, which this machine doesn't have".into())
    }

    fn get(&self, _: u32) -> Result<String, String> {
        Err("logins need the system's secret store, which this machine doesn't have".into())
    }

    fn delete(&mut self, _: u32) -> Result<(), String> {
        Ok(())
    }

    fn available(&self) -> bool {
        false
    }
}

/// Passwords in memory, for tests.
#[derive(Default)]
pub struct InMemory(pub std::collections::HashMap<u32, String>);

impl Secrets for InMemory {
    fn set(&mut self, id: u32, password: &str) -> Result<(), String> {
        self.0.insert(id, password.to_owned());
        Ok(())
    }

    fn get(&self, id: u32) -> Result<String, String> {
        self.0.get(&id).cloned().ok_or_else(|| "no password kept".into())
    }

    fn delete(&mut self, id: u32) -> Result<(), String> {
        self.0.remove(&id);
        Ok(())
    }
}

/// The logins, as kept on disk (no passwords), and the sites the person
/// said never to offer keeping one for.
pub struct Vault {
    file: PathBuf,
    pub logins: Vec<Login>,
    secrets: Box<dyn Secrets>,
    never_file: PathBuf,
    /// Sites equipe never offers to keep a login for: the person said so.
    pub never_offer: Vec<String>,
}

/// A site as logins name it: its host, lower-case, without `www.`.
pub fn site_of(text: &str) -> String {
    let text = text.trim();
    let rest = text.split_once("://").map_or(text, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next().unwrap_or_default().to_ascii_lowercase();
    host.strip_prefix("www.").unwrap_or(&host).to_owned()
}

/// Whether a login for `kept` serves `asked` — the same site, or one of its
/// subdomains either way (`accounts.google.com` for `google.com`).
pub fn serves(kept: &str, asked: &str) -> bool {
    let (kept, asked) = (site_of(kept), site_of(asked));
    !kept.is_empty() && (kept == asked || asked.ends_with(&format!(".{kept}")) || kept.ends_with(&format!(".{asked}")))
}

/// Where a login may be used without asking, as kept: a squad's name, or
/// [`EVERY_SQUAD`]. An entry from before squad-wide approvals,
/// `squad/member`, is its squad.
pub fn scope_of(who: &str) -> String {
    let who = who.trim();
    if who == EVERY_SQUAD {
        return who.to_owned();
    }
    who.split_once('/').map_or(who, |(squad, _)| squad).to_owned()
}

/// Whether members of `squad` may use `login` without asking.
pub fn allows(login: &Login, squad: &str) -> bool {
    login.allowed.iter().any(|w| w == EVERY_SQUAD || scope_of(w) == squad)
}

/// The hosts equipe signs in with `login` on by itself: its site, and its
/// sign-in page's host — exactly those, never a subdomain or a lookalike.
pub fn auto_hosts(login: &Login) -> Vec<String> {
    let mut hosts = vec![site_of(&login.site)];
    if let Some(url) = &login.url {
        let host = site_of(url);
        if !host.is_empty() && !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts.retain(|h| !h.is_empty());
    hosts
}

/// Whether `host` is this machine: a sign-in over plain http is let through
/// only there (or where the login's own sign-in page is http).
fn local(host: &str) -> bool {
    host == "localhost" || host == "127.0.0.1" || host.ends_with(".localhost")
}

/// Why a sign-in page is one equipe doesn't fill by itself even with the
/// person's leave: it signs in on another site's behalf — an OAuth or SSO
/// hand-off, or a page that sends the browser to another host afterwards —
/// where what comes next (granting an app access) is the person's to see.
pub fn hands_off_elsewhere(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let (path, query) = lower.split_once('?').unwrap_or((&lower, ""));
    if ["oauth", "authorize", "saml", "openid"].iter().any(|w| path.contains(w)) {
        return true;
    }
    let host = site_of(url);
    query.split('&').any(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decoded(value);
        if ["client_id", "redirect_uri", "response_type", "samlrequest", "relaystate"].contains(&key) {
            return true;
        }
        if ["oauth", "authorize", "saml"].iter().any(|w| value.contains(w)) {
            return true;
        }
        // A return address on another host: the sign-in serves that site.
        (value.starts_with("http://") || value.starts_with("https://") || value.starts_with("//")) && {
            let target = site_of(value.trim_start_matches('/'));
            !target.is_empty() && target != host && !target.ends_with(&format!(".{host}"))
        }
    })
}

/// `%2F` and `+` as written in an address's query, read back.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_ascii_lowercase()
}

/// The login equipe may sign a member's browser in with by itself at `url`
/// — a sign-in page there — when there is exactly one: turned on by the
/// person, not stopped after a failed try, its site (or sign-in page) on
/// that very host, over https (http only on this machine, or where the
/// login's own sign-in page is http), and not an OAuth or SSO hand-off.
pub fn auto_login_for<'a>(logins: &'a [Login], url: &str) -> Option<&'a Login> {
    let host = site_of(url);
    if host.is_empty() || hands_off_elsewhere(url) {
        return None;
    }
    let scheme = url.trim().split_once("://").map(|(s, _)| s.to_ascii_lowercase()).unwrap_or_default();
    let mut found = logins.iter().filter(|l| l.auto && l.stopped.is_none() && auto_hosts(l).contains(&host)).filter(|l| {
        scheme == "https" || (scheme == "http" && (local(&host) || l.url.as_deref().is_some_and(|u| u.trim().to_ascii_lowercase().starts_with("http://"))))
    });
    let one = found.next()?;
    // Two accounts for one site: which is the person's to say.
    if found.next().is_some() { None } else { Some(one) }
}

impl Vault {
    pub fn open(home: &Path, secrets: Box<dyn Secrets>) -> Self {
        let file = home.join("vault.json");
        let never_file = home.join("vault-never-offer.json");
        let logins = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let never_offer = std::fs::read_to_string(&never_file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let mut vault = Self { file, logins, secrets, never_file, never_offer };
        if vault.squad_wide() {
            let _ = vault.keep();
        }
        vault
    }

    /// "Always" from before it was squad-wide — `squad/member`, one member's
    /// — becomes its squad's: the person let that login be used in that
    /// squad, and every lane and member of it shares one browser and its
    /// sign-ins already. True when anything changed.
    fn squad_wide(&mut self) -> bool {
        let mut changed = false;
        for login in &mut self.logins {
            let mut moved: Vec<String> = Vec::new();
            for who in &login.allowed {
                let scope = scope_of(who);
                if scope.is_empty() {
                    continue;
                }
                if !moved.contains(&scope) {
                    moved.push(scope);
                }
            }
            if moved != login.allowed {
                login.allowed = moved;
                changed = true;
            }
        }
        changed
    }

    /// Whether a password can be kept on this machine.
    pub fn can_keep(&self) -> bool {
        self.secrets.available()
    }

    fn keep(&self) -> Result<(), String> {
        write_private(&self.file, &serde_json::to_string_pretty(&self.logins).map_err(|e| e.to_string())?)
    }

    /// A login made or changed from the person's draft: its id.
    pub fn save(&mut self, draft: LoginDraft, now: u64) -> Result<u32, String> {
        let site = site_of(&draft.site);
        let username = draft.username.trim().to_owned();
        if site.is_empty() || username.is_empty() {
            return Err("a login needs a site and a username".into());
        }
        // An address beyond the site's home is where its sign-in page is.
        let url = draft.site.trim().contains('/').then(|| {
            let t = draft.site.trim();
            if t.contains("://") { t.to_owned() } else { format!("https://{t}") }
        });
        let note = draft.note.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
        let new_password = draft.password.as_deref().is_some_and(|p| !p.is_empty());
        let id = match draft.id {
            Some(id) => {
                let login = self.logins.iter_mut().find(|l| l.id == id).ok_or("that login is gone")?;
                login.site = site;
                login.url = url.or(login.url.take());
                login.username = username;
                login.note = note;
                if let Some(auto) = draft.auto {
                    login.auto = auto;
                }
                // A new password, or signing in by itself turned on again: worth a try again.
                if new_password || draft.auto == Some(true) {
                    login.stopped = None;
                }
                id
            }
            None => {
                if !new_password {
                    return Err("a new login needs its password".into());
                }
                let id = self.logins.iter().map(|l| l.id).max().unwrap_or(0) + 1;
                let auto = draft.auto.unwrap_or(false);
                self.logins.push(Login { id, site, url, username, note, allowed: Vec::new(), auto, stopped: None, created_at: now, uses: Vec::new() });
                id
            }
        };
        if let Some(password) = draft.password.filter(|p| !p.is_empty())
            && let Err(e) = self.secrets.set(id, &password)
        {
            if draft.id.is_none() {
                self.logins.retain(|l| l.id != id);
            }
            return Err(e);
        }
        self.keep()?;
        Ok(id)
    }

    pub fn delete(&mut self, id: u32) -> Result<(), String> {
        self.secrets.delete(id)?;
        self.logins.retain(|l| l.id != id);
        self.keep()
    }

    /// The logins for `site`.
    pub fn for_site(&self, site: &str) -> Vec<&Login> {
        self.logins.iter().filter(|l| serves(&l.site, site)).collect()
    }

    /// Let `login` be used without asking in `who`: a squad, or every squad.
    pub fn allow(&mut self, id: u32, who: &str) -> Result<(), String> {
        let scope = scope_of(who);
        if scope.is_empty() {
            return Err("say which squad".into());
        }
        if let Some(l) = self.logins.iter_mut().find(|l| l.id == id)
            && !l.allowed.contains(&scope)
        {
            l.allowed.push(scope);
        }
        self.keep()
    }

    pub fn forget(&mut self, id: u32, who: &str) -> Result<(), String> {
        let scope = scope_of(who);
        if let Some(l) = self.logins.iter_mut().find(|l| l.id == id) {
            l.allowed.retain(|w| scope_of(w) != scope);
        }
        self.keep()
    }

    /// Turn signing in by itself on or off for a login — the person's alone.
    /// On again, a stop after a failed try is forgotten.
    pub fn set_auto(&mut self, id: u32, on: bool) -> Result<(), String> {
        let login = self.logins.iter_mut().find(|l| l.id == id).ok_or("that login is gone")?;
        login.auto = on;
        if on {
            login.stopped = None;
        }
        self.keep()
    }

    /// equipe stops signing in with a login by itself, and why — until the
    /// person saves a new password or turns it on again.
    pub fn stop_auto(&mut self, id: u32, why: &str) -> Result<(), String> {
        if let Some(l) = self.logins.iter_mut().find(|l| l.id == id) {
            l.stopped = Some(why.to_owned());
        }
        self.keep()
    }

    pub fn used(&mut self, id: u32, at: u64, squad: &str, member: &str, outcome: &str) {
        if let Some(l) = self.logins.iter_mut().find(|l| l.id == id) {
            l.uses.insert(0, Use { at, squad: squad.to_owned(), member: member.to_owned(), outcome: outcome.to_owned() });
            l.uses.truncate(USES_KEPT);
        }
        let _ = self.keep();
    }

    /// Never offer to keep a login for `site` again.
    pub fn never_offer(&mut self, site: &str) -> Result<(), String> {
        let site = site_of(site);
        if !site.is_empty() && !self.never_offer.contains(&site) {
            self.never_offer.push(site);
        }
        write_private(&self.never_file, &serde_json::to_string_pretty(&self.never_offer).map_err(|e| e.to_string())?)
    }

    /// Whether the person said never to offer keeping a login for `site`.
    pub fn refused_offer(&self, site: &str) -> bool {
        self.never_offer.iter().any(|s| serves(s, site))
    }

    /// The password, to be typed in by equipe — never to be handed on.
    pub fn password(&self, id: u32) -> Result<String, String> {
        self.secrets.get(id)
    }
}

/// `text` to `file`, readable by the person only.
fn write_private(file: &Path, text: &str) -> Result<(), String> {
    std::fs::write(file, text).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// What marks a field as the username one, in any language: the locale
/// files' words, as a regular expression's alternatives.
fn login_fields() -> String {
    let escape = |w: &str| w.chars().fold(String::new(), |mut out, c| {
        if "\\^$.*+?()[]{}|/-".contains(c) {
            out.push('\\');
        }
        out.push(c);
        out
    });
    crate::i18n::every("match.login_fields").iter().map(|w| escape(w)).collect::<Vec<_>>().join("|")
}

/// Where the fill script may type: the hosts it checks the page against
/// when it runs, whether their subdomains count, and whether plain http does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillGuard {
    pub hosts: Vec<String>,
    pub subdomains: bool,
    pub http: bool,
}

impl FillGuard {
    /// A member's `sign_in`: the login's site and its subdomains, as
    /// [`serves`] has it.
    pub fn serving(login: &Login) -> Self {
        Self { hosts: auto_hosts(login), subdomains: true, http: true }
    }

    /// equipe by itself: those exact hosts, https unless the login's own
    /// sign-in page is http or the host is this machine.
    pub fn exact(login: &Login) -> Self {
        let hosts = auto_hosts(login);
        let http = login.url.as_deref().is_some_and(|u| u.trim().to_ascii_lowercase().starts_with("http://")) || hosts.iter().any(|h| local(h));
        Self { hosts, subdomains: false, http }
    }
}

/// The mark the fill script's answer starts with: put together when the
/// script runs, so the script's own text (which a browser tool may echo
/// back beside the answer) never carries it.
const ANSWER_MARK: &str = "EQUIPE_FILL:";

/// What equipe runs in a member's page (Playwright's `browser_evaluate`) to
/// sign in: first checks the page is where `guard` lets it type (its host,
/// and https), then fills the username and password fields it finds — the
/// password one, and the username one before it, or one marked as a
/// username or email — as a person's typing would (the native setter, then
/// `input` and `change`), and submits their form. A page asking for the
/// username alone (the first step of a two-step sign-in) gets that, and
/// submits. It answers what it did as JSON after [`ANSWER_MARK`] — never
/// the values. Only the top page: never a frame of another site.
pub fn fill_script(username: &str, password: &str, guard: &FillGuard) -> String {
    let u = serde_json::to_string(username).unwrap_or_default();
    let p = serde_json::to_string(password).unwrap_or_default();
    let field = serde_json::to_string(&login_fields()).unwrap_or_default();
    let hosts = serde_json::to_string(&guard.hosts).unwrap_or_default();
    let (mark_a, mark_b) = ANSWER_MARK.split_at(6);
    format!(
        r#"() => {{
  const MARK = {mark_a:?} + {mark_b:?};
  const say = (what) => MARK + JSON.stringify(what);
  const HOSTS = {hosts}, SUB = {sub}, HTTP = {http};
  const host = location.hostname.toLowerCase().replace(/^www\./, '');
  const here = HOSTS.some((h) => host === h || (SUB && (host.endsWith('.' + h) || h.endsWith('.' + host))));
  if (!here) return say({{ refused: 'host', filled: [], submitted: false }});
  if (location.protocol !== 'https:' && !(HTTP && location.protocol === 'http:')) return say({{ refused: 'insecure', filled: [], submitted: false }});
  const U = {u}, P = {p};
  const shown = (el) => {{ const r = el.getBoundingClientRect(); const cs = getComputedStyle(el); return r.width > 0 && r.height > 0 && cs.visibility !== 'hidden' && cs.display !== 'none' && !el.disabled && !el.readOnly; }};
  const set = (el, v) => {{
    const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, v);
    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  }};
  const inputs = Array.from(document.querySelectorAll('input')).filter(shown);
  const pass = inputs.find((i) => i.type === 'password');
  const textual = (i) => ['text', 'email', 'tel', ''].includes(i.type) || i.type === undefined;
  const named = inputs.find((i) => textual(i) && new RegExp({field}, 'i').test([i.autocomplete, i.name, i.id, i.placeholder, i.getAttribute('aria-label')].join(' ')));
  let user = named;
  if (pass && !user) {{ const before = inputs.slice(0, inputs.indexOf(pass)).filter(textual); user = before[before.length - 1]; }}
  if (!pass && !user) return say({{ filled: [], submitted: false }});
  const filled = [];
  if (user && !user.value) {{ user.focus(); set(user, U); filled.push('username'); }} else if (user) {{ filled.push('username (already there)'); }}
  if (pass) {{ pass.focus(); set(pass, P); filled.push('password'); }}
  const form = (pass || user).form;
  const button = (form || document).querySelector('button[type=submit], input[type=submit], button:not([type])');
  let submitted = false;
  if (form && form.requestSubmit) {{ try {{ form.requestSubmit(button || undefined); submitted = true; }} catch (e) {{}} }}
  if (!submitted && button) {{ button.click(); submitted = true; }}
  return say({{ filled, submitted, step: pass ? 'password' : 'username' }});
}}"#,
        sub = guard.subdomains,
        http = guard.http,
    )
}

/// What the fill script said it did, read out of a browser tool's answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct Filled {
    /// `host` or `insecure`: the page wasn't one it may type on; nothing typed.
    pub refused: Option<String>,
    pub filled: Vec<String>,
    pub submitted: bool,
    /// `password` or `username`: which step of a sign-in it was.
    pub step: Option<String>,
}

/// The fill script's answer in a browser tool's text — which may quote the
/// script too, and gives the answer as a JSON string: `None` when there is
/// none (the call failed).
pub fn fill_answer(text: &str) -> Option<Filled> {
    for line in text.lines().map(str::trim) {
        let unquoted = if line.starts_with('"') { serde_json::from_str::<String>(line).ok() } else { Some(line.to_owned()) };
        let Some(line) = unquoted else { continue };
        if let Some(rest) = line.find(ANSWER_MARK).map(|at| &line[at + ANSWER_MARK.len()..])
            && let Ok(filled) = serde_json::from_str::<Filled>(rest.trim())
        {
            return Some(filled);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("equipe-vault-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn draft(site: &str, user: &str, password: Option<&str>) -> LoginDraft {
        LoginDraft { id: None, site: site.into(), username: user.into(), password: password.map(Into::into), note: None, auto: None }
    }

    fn login(site: &str, url: Option<&str>, auto: bool) -> Login {
        Login { id: 1, site: site.into(), url: url.map(Into::into), username: "abel".into(), note: None, allowed: Vec::new(), auto, stopped: None, created_at: 0, uses: Vec::new() }
    }

    #[test]
    fn a_site_is_its_host_and_serves_its_subdomains() {
        assert_eq!(site_of("https://www.GitHub.com/login?x=1"), "github.com");
        assert_eq!(site_of("accounts.google.com"), "accounts.google.com");
        assert_eq!(site_of("http://user@example.com:8080/a"), "example.com");
        assert!(serves("google.com", "https://accounts.google.com/signin"));
        assert!(serves("github.com", "github.com"));
        assert!(!serves("github.com", "notgithub.com"));
        assert!(!serves("", "github.com"));
    }

    #[test]
    fn the_password_goes_to_the_secret_store_alone_and_the_file_holds_the_rest() {
        let home = home();
        let mut vault = Vault::open(&home, Box::new(InMemory::default()));
        assert!(vault.save(draft("github.com", "abel", None), 1).is_err(), "a new login needs its password");
        let id = vault.save(draft("https://github.com/login", "abel", Some("hunter2")), 1).unwrap();
        let file = std::fs::read_to_string(home.join("vault.json")).unwrap();
        assert!(!file.contains("hunter2"), "{file}");
        assert!(file.contains("github.com") && file.contains("https://github.com/login"));
        assert_eq!(vault.password(id).unwrap(), "hunter2");
        // A change without a password keeps the one kept.
        vault.save(LoginDraft { id: Some(id), site: "github.com".into(), username: "abel2".into(), password: None, note: Some(" work ".into()), auto: None }, 2).unwrap();
        assert_eq!(vault.password(id).unwrap(), "hunter2");
        let l = &vault.logins[0];
        assert_eq!((l.username.as_str(), l.note.as_deref(), l.url.as_deref()), ("abel2", Some("work"), Some("https://github.com/login")));
        // Kept across a restart.
        let again = Vault::open(&home, Box::new(InMemory::default()));
        assert_eq!(again.logins.len(), 1);
        // Where it may be used without asking: a squad, every squad.
        vault.allow(id, "acme").unwrap();
        vault.allow(id, "acme").unwrap();
        assert_eq!(vault.logins[0].allowed, ["acme"]);
        assert!(allows(&vault.logins[0], "acme") && !allows(&vault.logins[0], "other"));
        vault.allow(id, EVERY_SQUAD).unwrap();
        assert!(allows(&vault.logins[0], "other"));
        vault.forget(id, EVERY_SQUAD).unwrap();
        vault.used(id, 5, "acme", "frontend", "signed_in");
        assert_eq!(vault.logins[0].uses[0].outcome, "signed_in");
        vault.forget(id, "acme/frontend").unwrap();
        assert!(vault.logins[0].allowed.is_empty(), "an old member entry takes back its squad");
        assert_eq!(vault.for_site("https://github.com/x").len(), 1);
        vault.delete(id).unwrap();
        assert!(vault.logins.is_empty() && vault.password(id).is_err());
    }

    #[test]
    fn a_members_always_from_before_becomes_its_squads_on_load() {
        let home = home();
        let old = r#"[{"id":1,"site":"github.com","username":"abel","allowed":["acme/frontend","acme/backend-3","beta/qa"],"created_at":1}]"#;
        std::fs::write(home.join("vault.json"), old).unwrap();
        let vault = Vault::open(&home, Box::new(InMemory::default()));
        assert_eq!(vault.logins[0].allowed, ["acme", "beta"]);
        assert!(!vault.logins[0].auto, "never on by itself");
        let file = std::fs::read_to_string(home.join("vault.json")).unwrap();
        assert!(file.contains("\"acme\"") && !file.contains("acme/frontend"), "{file}");
    }

    #[test]
    fn signing_in_by_itself_is_the_persons_to_turn_on_and_stops_after_a_failed_try() {
        let home = home();
        let mut vault = Vault::open(&home, Box::new(InMemory::default()));
        let id = vault.save(draft("github.com", "abel", Some("pw")), 1).unwrap();
        assert!(!vault.logins[0].auto, "off for a new login");
        vault.set_auto(id, true).unwrap();
        assert!(auto_login_for(&vault.logins, "https://github.com/login").is_some());
        vault.stop_auto(id, "the form came back").unwrap();
        assert!(auto_login_for(&vault.logins, "https://github.com/login").is_none(), "stopped");
        // A new password is worth a try again.
        vault.save(LoginDraft { id: Some(id), site: "github.com".into(), username: "abel".into(), password: Some("pw2".into()), note: None, auto: None }, 2).unwrap();
        assert!(vault.logins[0].stopped.is_none() && vault.logins[0].auto);
        vault.set_auto(id, false).unwrap();
        assert!(auto_login_for(&vault.logins, "https://github.com/login").is_none());
    }

    #[test]
    fn equipe_signs_in_by_itself_only_on_the_logins_exact_host_over_https() {
        let logins = [login("github.com", None, true)];
        assert!(auto_login_for(&logins, "https://github.com/login?return_to=%2Fsettings").is_some());
        assert!(auto_login_for(&logins, "https://www.github.com/session").is_some());
        for elsewhere in [
            "https://github.com.evil.io/login",
            "https://evilgithub.com/login",
            "https://gist.github.com/login",
            "http://github.com/login",
            "https://github.com/login/oauth/authorize?client_id=x",
            "https://github.com/login?return_to=https%3A%2F%2Fevil.io%2F",
            "https://github.com/login?client_id=abc&return_to=%2Flogin%2Foauth%2Fauthorize",
        ] {
            assert!(auto_login_for(&logins, elsewhere).is_none(), "{elsewhere}");
        }
        // Its sign-in page's host counts too, exactly.
        let sso = [login("app.example.com", Some("https://login.example.com/signin"), true)];
        assert!(auto_login_for(&sso, "https://login.example.com/signin").is_some());
        assert!(auto_login_for(&sso, "https://example.com/signin").is_none());
        // Off, or two accounts for the site: not equipe's to pick.
        assert!(auto_login_for(&[login("github.com", None, false)], "https://github.com/login").is_none());
        let two = [login("github.com", None, true), Login { id: 2, username: "other".into(), ..login("github.com", None, true) }];
        assert!(auto_login_for(&two, "https://github.com/login").is_none());
        // Plain http only on this machine, or where the login's own page is.
        assert!(auto_login_for(&[login("localhost", None, true)], "http://localhost/login").is_some());
        assert!(auto_login_for(&[login("intranet.lan", Some("http://intranet.lan/login"), true)], "http://intranet.lan/login").is_some());
    }

    #[test]
    fn never_offering_a_site_is_kept() {
        let home = home();
        let mut vault = Vault::open(&home, Box::new(InMemory::default()));
        assert!(!vault.refused_offer("github.com"));
        vault.never_offer("https://github.com/login").unwrap();
        let again = Vault::open(&home, Box::new(InMemory::default()));
        assert!(again.refused_offer("github.com") && again.refused_offer("https://www.github.com/x"));
        assert!(!again.refused_offer("gitlab.com"));
    }

    #[test]
    fn the_fill_script_carries_the_values_as_json_checks_the_host_and_answers_without_them() {
        let guard = FillGuard::exact(&login("github.com", None, true));
        let script = fill_script("a\"b", "p'w\\</script>", &guard);
        assert!(script.contains(r#"const U = "a\"b", P = "p'w\\</script>";"#), "{script}");
        assert!(script.contains("requestSubmit"));
        assert!(script.contains(r#"const HOSTS = ["github.com"], SUB = false, HTTP = false;"#), "{script}");
        assert!(!script.contains(ANSWER_MARK), "the script never carries the mark it answers with");
        // The host and scheme are checked before the values are even set.
        assert!(script.find("refused: 'host'").unwrap() < script.find("const U =").unwrap());
        let serving = FillGuard::serving(&login("github.com", Some("https://github.com/login"), false));
        assert!(serving.subdomains && serving.http);
    }

    #[test]
    fn the_fill_answer_is_read_out_of_a_tool_answer_that_quotes_the_script_too() {
        let text = "### Result\n\"EQUIPE_FILL:{\\\"filled\\\":[\\\"username\\\",\\\"password\\\"],\\\"submitted\\\":true,\\\"step\\\":\\\"password\\\"}\"\n### Ran Playwright code\nawait page.evaluate('() => { const MARK = \"EQUIPE\" + \"_FILL:\"; filled.push(\\'username\\') }');";
        let filled = fill_answer(text).unwrap();
        assert_eq!((filled.submitted, filled.step.as_deref(), filled.filled.len()), (true, Some("password"), 2));
        let refused = fill_answer("\"EQUIPE_FILL:{\\\"refused\\\":\\\"host\\\",\\\"filled\\\":[],\\\"submitted\\\":false}\"").unwrap();
        assert_eq!(refused.refused.as_deref(), Some("host"));
        assert_eq!(fill_answer("### Error\nTarget closed"), None);
    }
}
