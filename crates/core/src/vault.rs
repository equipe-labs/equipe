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

use crate::protocol::vault::{CliKind, CliLogin, CliLoginDraft, EVERY_SQUAD, Login, LoginDraft, Mailbox, MailboxDraft, Use};

/// The service the passwords are kept under, in every secret store.
pub const SERVICE: &str = "equipe.vault";
/// Uses kept per login, the latest.
const USES_KEPT: usize = 20;

/// Where passwords go: the system's secret store, or (in tests) memory. One
/// interface for every platform; a platform without a store refuses — a
/// password is never written to a file instead.
pub trait Secrets: Send {
    /// Keep `value` under `account` (a login's password under its id; its
    /// authenticator seed, passkey, the mailbox's and command-line
    /// credentials under names of their own, [`account`]).
    fn set_named(&mut self, account: &str, value: &str) -> Result<(), String>;
    fn get_named(&self, account: &str) -> Result<String, String>;
    /// Gone already is fine.
    fn delete_named(&mut self, account: &str) -> Result<(), String>;
    /// Whether a password can be kept here at all: offering to keep a login
    /// where it can't be would only end in an error.
    fn available(&self) -> bool {
        true
    }
    /// A secret under a service of its own (`equipe.secrets`, squads' keys
    /// members obtained). On the Keychain it is `equipe.secrets:<account>` in
    /// the same one item; elsewhere the service leads the account's name.
    fn set_in(&mut self, service: &str, account: &str, value: &str) -> Result<(), String> {
        self.set_named(&format!("{service}:{account}"), value)
    }
    /// `interactive`: the person asked for it now, so a Keychain that was
    /// refused a while ago may ask again.
    fn get_in(&self, service: &str, account: &str, _interactive: bool) -> Result<String, String> {
        self.get_named(&format!("{service}:{account}"))
    }
    fn delete_in(&mut self, service: &str, account: &str) -> Result<(), String> {
        self.delete_named(&format!("{service}:{account}"))
    }
    fn set(&mut self, id: u32, password: &str) -> Result<(), String> {
        self.set_named(&id.to_string(), password)
    }
    fn get(&self, id: u32) -> Result<String, String> {
        self.get_named(&id.to_string())
    }
    fn delete(&mut self, id: u32) -> Result<(), String> {
        self.delete_named(&id.to_string())
    }
}

/// The names secrets other than a login's password are kept under, in the
/// same secret store and service.
pub mod account {
    /// A login's authenticator (TOTP) seed, as an `otpauth://` address.
    pub fn totp(id: u32) -> String {
        format!("{id}.totp")
    }
    /// equipe's own passkey for a login's site: the credential as Chrome's
    /// virtual authenticator gave it (its private key in it).
    pub fn passkey(id: u32) -> String {
        format!("{id}.passkey")
    }
    /// The app password of the mailbox sites' codes come to.
    pub const MAILBOX: &str = "mailbox";
    /// A command-line credential's token or key.
    pub fn cli(id: u32) -> String {
        format!("cli.{id}")
    }
}

/// The daemon machine's own secret store.
pub struct SecretStore;

/// The secret store of the machine the daemon runs on.
pub fn system_store() -> Box<dyn Secrets> {
    Box::new(SecretStore)
}

/// macOS: equipe's one Keychain item ([`crate::keychain`]), read once per
/// process — never one dialog per login.
#[cfg(target_os = "macos")]
impl Secrets for SecretStore {
    fn set_named(&mut self, account: &str, password: &str) -> Result<(), String> {
        crate::keychain::global().set(SERVICE, account, password)
    }

    fn get_named(&self, account: &str) -> Result<String, String> {
        crate::keychain::global().get(SERVICE, account, false)
    }

    fn delete_named(&mut self, account: &str) -> Result<(), String> {
        crate::keychain::global().delete(SERVICE, account)
    }

    fn set_in(&mut self, service: &str, account: &str, value: &str) -> Result<(), String> {
        crate::keychain::global().set(service, account, value)
    }

    fn get_in(&self, service: &str, account: &str, interactive: bool) -> Result<String, String> {
        crate::keychain::global().get(service, account, interactive)
    }

    fn delete_in(&mut self, service: &str, account: &str) -> Result<(), String> {
        crate::keychain::global().delete(service, account)
    }
}

/// Windows: Credential Manager, a generic credential per login
/// (`equipe.vault/<id>`, and `equipe.vault/<name>` for the rest), kept for
/// this user on this machine.
#[cfg(windows)]
mod credential_manager {
    use windows_sys::Win32::Security::Credentials::{CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW};

    /// "Element not found": no credential of that name.
    const NOT_FOUND: i32 = 1168;

    fn target(id: &str) -> Vec<u16> {
        format!("{}/{id}", super::SERVICE).encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn last_error() -> String {
        std::io::Error::last_os_error().to_string()
    }

    pub fn set(id: &str, password: &str) -> Result<(), String> {
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

    pub fn get(id: &str) -> Result<String, String> {
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

    pub fn delete(id: &str) -> Result<(), String> {
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
    fn set_named(&mut self, account: &str, password: &str) -> Result<(), String> {
        credential_manager::set(account, password)
    }

    fn get_named(&self, account: &str) -> Result<String, String> {
        credential_manager::get(account)
    }

    fn delete_named(&mut self, account: &str) -> Result<(), String> {
        credential_manager::delete(account)
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

    fn attributes(id: &str) -> [String; 4] {
        ["service".into(), super::SERVICE.into(), "account".into(), id.to_owned()]
    }

    pub fn set(id: &str, password: &str) -> Result<(), String> {
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

    pub fn get(id: &str) -> Result<String, String> {
        if !available() {
            return Err(missing());
        }
        let out = Command::new(TOOL).arg("lookup").args(attributes(id)).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
        if !out.status.success() || out.stdout.is_empty() {
            return Err("no password kept".into());
        }
        String::from_utf8(out.stdout).map_err(|_| "the kept password isn't text".into())
    }

    pub fn delete(id: &str) -> Result<(), String> {
        if !available() {
            return Ok(());
        }
        // Gone already is fine too: `clear` of nothing succeeds.
        Command::new(TOOL).arg("clear").args(attributes(id)).stdin(Stdio::null()).output().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
impl Secrets for SecretStore {
    fn set_named(&mut self, account: &str, password: &str) -> Result<(), String> {
        secret_service::set(account, password)
    }

    fn get_named(&self, account: &str) -> Result<String, String> {
        secret_service::get(account)
    }

    fn delete_named(&mut self, account: &str) -> Result<(), String> {
        secret_service::delete(account)
    }

    fn available(&self) -> bool {
        secret_service::available()
    }
}

#[cfg(not(any(unix, windows)))]
impl Secrets for SecretStore {
    fn set_named(&mut self, _: &str, _: &str) -> Result<(), String> {
        Err("logins need the system's secret store, which this machine doesn't have".into())
    }

    fn get_named(&self, _: &str) -> Result<String, String> {
        Err("logins need the system's secret store, which this machine doesn't have".into())
    }

    fn delete_named(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }

    fn available(&self) -> bool {
        false
    }
}

/// Passwords in memory, for tests.
#[derive(Default)]
pub struct InMemory(pub std::collections::HashMap<String, String>);

impl Secrets for InMemory {
    fn set_named(&mut self, account: &str, password: &str) -> Result<(), String> {
        self.0.insert(account.to_owned(), password.to_owned());
        Ok(())
    }

    fn get_named(&self, account: &str) -> Result<String, String> {
        self.0.get(account).cloned().ok_or_else(|| "no password kept".into())
    }

    fn delete_named(&mut self, account: &str) -> Result<(), String> {
        self.0.remove(account);
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
    /// Login autopilot's own settings (`autopilot.json`).
    pub settings: crate::autopilot::Settings,
    settings_file: PathBuf,
    /// Command-line credentials, without their secrets (`cli-logins.json`).
    pub cli: Vec<CliLogin>,
    cli_file: PathBuf,
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

/// Identity providers whose own sign-in page equipe may fill when another
/// site sends the browser there ("Sign in with Google" …), by host, and the
/// name they go by. Only these exact hosts (and an Okta tenant's own
/// `<tenant>.okta.com`): a login saved for `google.com` isn't one.
const IDP_HOSTS: [(&str, &str); 5] = [
    ("accounts.google.com", "Google"),
    ("login.microsoftonline.com", "Microsoft"),
    ("login.live.com", "Microsoft"),
    ("appleid.apple.com", "Apple"),
    ("github.com", "GitHub"),
];

/// The identity provider whose own sign-in host `site` is, by name: `None`
/// for any other site.
pub fn idp_provider(site: &str) -> Option<&'static str> {
    let host = site_of(site);
    if let Some((_, name)) = IDP_HOSTS.iter().find(|(h, _)| *h == host) {
        return Some(name);
    }
    // An Okta tenant: one label before okta.com, nothing deeper.
    host.strip_suffix(".okta.com").filter(|t| !t.is_empty() && !t.contains('.')).map(|_| "Okta")
}

/// Whether an identity provider's page grants another site something — a
/// consent or permission screen, an approval — rather than asking who you
/// are: never equipe's, even with the person's leave for "Sign in with …".
pub fn idp_grant_page(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    let rest = lower.split_once("://").map_or(lower.as_str(), |(_, r)| r);
    let path = rest.split(['?', '#']).next().unwrap_or_default();
    let path = path.find('/').map_or("", |at| &path[at..]);
    ["consent", "approv", "grant", "permission"].iter().any(|w| path.contains(w))
}

/// The identity-provider login equipe may use at `url` when that page signs
/// in on another site's behalf ("Sign in with Google" on a company
/// dashboard) — a narrow exception to [`hands_off_elsewhere`]. Only when all
/// hold: the login's site is the provider's own host ([`idp_provider`]),
/// the person turned "Use this account for 'Sign in with …'" on for it
/// (`Autopilot::idp`) and lets it sign in by itself, it isn't stopped, the
/// page is on exactly that host over https, the page isn't a grant step
/// ([`idp_grant_page`]) — and exactly one login matches. What equipe may do
/// there is narrower still ([`FillGuard::idp`]).
pub fn idp_login_for<'a>(logins: &'a [Login], url: &str) -> Option<&'a Login> {
    let host = site_of(url);
    if host.is_empty() || !url.trim().to_ascii_lowercase().starts_with("https://") || idp_grant_page(url) {
        return None;
    }
    let mut found = logins.iter().filter(|l| l.auto && l.stopped.is_none() && l.autopilot.idp && idp_provider(&l.site).is_some() && site_of(&l.site) == host);
    let one = found.next()?;
    if found.next().is_some() { None } else { Some(one) }
}

impl Vault {
    pub fn open(home: &Path, secrets: Box<dyn Secrets>) -> Self {
        let file = home.join("vault.json");
        let never_file = home.join("vault-never-offer.json");
        let logins = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let never_offer = std::fs::read_to_string(&never_file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let settings_file = home.join("autopilot.json");
        let cli_file = home.join("cli-logins.json");
        let settings = std::fs::read_to_string(&settings_file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let cli = std::fs::read_to_string(&cli_file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let mut vault = Self { file, logins, secrets, never_file, never_offer, settings, settings_file, cli, cli_file };
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
                if let Some(idp) = draft.idp {
                    login.autopilot.idp = idp && idp_provider(&login.site).is_some();
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
                // Saving a login is the person's leave for equipe to sign in
                // with it by itself: on unless they turned it off on the form.
                let auto = draft.auto.unwrap_or(true);
                // An identity provider's own account ("Sign in with Google" …)
                // is saved to be used on other sites' sign-in steps too: on by
                // default, off when the form says so. Consent screens stay the
                // person's whatever this says (`idp_grant_page`, `FillGuard::idp`).
                let autopilot = crate::protocol::vault::Autopilot { idp: draft.idp.unwrap_or(true) && idp_provider(&site).is_some(), ..Default::default() };
                self.logins.push(Login { id, site, url, username, note, allowed: Vec::new(), auto, stopped: None, created_at: now, password_at: None, uses: Vec::new(), autopilot });
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
        if new_password && let Some(l) = self.logins.iter_mut().find(|l| l.id == id) {
            l.password_at = Some(now);
        }
        self.keep()?;
        Ok(id)
    }

    pub fn delete(&mut self, id: u32) -> Result<(), String> {
        self.secrets.delete(id)?;
        // Its seed and passkey go with it.
        let _ = self.secrets.delete_named(&account::totp(id));
        let _ = self.secrets.delete_named(&account::passkey(id));
        self.logins.retain(|l| l.id != id);
        self.keep()
    }

    /// The person's autopilot switches for a login ([`Autopilot::switches_from`]).
    pub fn set_autopilot(&mut self, id: u32, asked: &crate::protocol::vault::Autopilot) -> Result<(), String> {
        let login = self.logins.iter_mut().find(|l| l.id == id).ok_or("that login is gone")?;
        login.autopilot = login.autopilot.switches_from(asked);
        self.keep()
    }

    /// Keep a login's authenticator seed (already checked: an `otpauth://`
    /// address), or take it away. Keeping one turns its code autopilot on:
    /// the person gave it for nothing else.
    pub fn set_totp(&mut self, id: u32, seed: Option<&str>) -> Result<(), String> {
        let login = self.logins.iter().position(|l| l.id == id).ok_or("that login is gone")?;
        match seed {
            Some(seed) => self.secrets.set_named(&account::totp(id), seed)?,
            None => self.secrets.delete_named(&account::totp(id))?,
        }
        let a = &mut self.logins[login].autopilot;
        a.has_totp = seed.is_some();
        a.totp = seed.is_some();
        self.keep()
    }

    /// The authenticator seed, to make a code from — never to be handed on.
    pub fn totp(&self, id: u32) -> Result<String, String> {
        self.secrets.get_named(&account::totp(id))
    }

    /// Keep equipe's own passkey for a login's site, or take it away.
    pub fn set_passkey(&mut self, id: u32, credential: Option<&str>) -> Result<(), String> {
        let login = self.logins.iter().position(|l| l.id == id).ok_or("that login is gone")?;
        match credential {
            Some(c) => self.secrets.set_named(&account::passkey(id), c)?,
            None => self.secrets.delete_named(&account::passkey(id))?,
        }
        let a = &mut self.logins[login].autopilot;
        a.has_passkey = credential.is_some();
        a.passkey = credential.is_some();
        self.keep()
    }

    /// equipe's passkey for a login's site — to give to the browser, never on.
    pub fn passkey(&self, id: u32) -> Result<String, String> {
        self.secrets.get_named(&account::passkey(id))
    }

    /// The latest keep-alive look at a login's site.
    pub fn checked(&mut self, id: u32, look: Use) {
        if let Some(l) = self.logins.iter_mut().find(|l| l.id == id) {
            l.autopilot.checked = Some(look);
        }
        let _ = self.keep();
    }

    fn keep_settings(&self) -> Result<(), String> {
        write_private(&self.settings_file, &serde_json::to_string_pretty(&self.settings).map_err(|e| e.to_string())?)
    }

    /// The mailbox sites' codes come to: its app password to the secret
    /// store, the rest to `autopilot.json`; `None` forgets both.
    pub fn save_mailbox(&mut self, draft: Option<MailboxDraft>) -> Result<(), String> {
        let Some(draft) = draft else {
            self.secrets.delete_named(account::MAILBOX)?;
            self.settings.mailbox = None;
            return self.keep_settings();
        };
        let host = draft.host.trim().trim_start_matches("imaps://").trim_end_matches('/').to_ascii_lowercase();
        let username = draft.username.trim().to_owned();
        if host.is_empty() || username.is_empty() || host.contains(['/', ' ']) {
            return Err("a mailbox needs its IMAP server and username".into());
        }
        let new_password = draft.password.as_deref().is_some_and(|p| !p.is_empty());
        if !new_password && self.settings.mailbox.is_none() {
            return Err("a mailbox needs its app password".into());
        }
        if let Some(password) = draft.password.filter(|p| !p.is_empty()) {
            self.secrets.set_named(account::MAILBOX, &password)?;
        }
        self.settings.mailbox = Some(Mailbox { host, port: if draft.port == 0 { 993 } else { draft.port }, username });
        self.keep_settings()
    }

    pub fn set_sms(&mut self, on: bool) -> Result<(), String> {
        self.settings.sms = on;
        self.keep_settings()
    }

    pub fn set_keepalive_hours(&mut self, hours: u32) -> Result<(), String> {
        self.settings.keepalive_hours = hours.min(24 * 7);
        self.keep_settings()
    }

    fn keep_cli(&self) -> Result<(), String> {
        write_private(&self.cli_file, &serde_json::to_string_pretty(&self.cli).map_err(|e| e.to_string())?)
    }

    /// A command-line credential made or changed from the person's draft:
    /// its id. Its secret goes to the secret store alone.
    pub fn save_cli(&mut self, draft: CliLoginDraft, now: u64) -> Result<u32, String> {
        let name = draft.name.trim().to_owned();
        let host = draft.host.trim().to_ascii_lowercase();
        let public = draft.public.trim().to_owned();
        match draft.kind {
            CliKind::Env if !crate::autopilot::cli_logins::env_name(&name) => return Err("an environment variable's name is uppercase letters, digits and _ (NPM_TOKEN)".into()),
            CliKind::AwsKeys if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) || public.is_empty() => {
                return Err("an AWS key needs its profile's name and its access key id".into());
            }
            CliKind::GhApp if !public.contains('/') => return Err("a GitHub App needs <app id>/<installation id>".into()),
            CliKind::DeviceLogin if !["gh", "aws", "vercel"].contains(&name.as_str()) || host.is_empty() => {
                return Err("a device sign-in names its CLI (gh, aws or vercel) and the hosts of its approval page".into());
            }
            _ => {}
        }
        let host = if host.is_empty() && matches!(draft.kind, CliKind::GhToken | CliKind::GhApp) { "github.com".to_owned() } else { host };
        let squad = draft.squad.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        let secret = draft.secret.filter(|s| !s.trim().is_empty());
        let needs_secret = draft.kind != CliKind::DeviceLogin;
        let id = match draft.id {
            Some(id) => {
                let c = self.cli.iter_mut().find(|c| c.id == id).ok_or("that credential is gone")?;
                (c.name, c.host, c.public, c.squad, c.kind) = (name, host, public, squad, draft.kind);
                id
            }
            None => {
                if needs_secret && secret.is_none() {
                    return Err("give its token or key".into());
                }
                let id = self.cli.iter().map(|c| c.id).max().unwrap_or(0) + 1;
                self.cli.push(CliLogin { id, kind: draft.kind, name, host, public, squad, created_at: now, last: None });
                id
            }
        };
        if let Some(secret) = secret
            && needs_secret
            && let Err(e) = self.secrets.set_named(&account::cli(id), secret.trim())
        {
            if draft.id.is_none() {
                self.cli.retain(|c| c.id != id);
            }
            return Err(e);
        }
        self.keep_cli()?;
        Ok(id)
    }

    pub fn delete_cli(&mut self, id: u32) -> Result<(), String> {
        let _ = self.secrets.delete_named(&account::cli(id));
        self.cli.retain(|c| c.id != id);
        self.keep_cli()
    }

    /// How a command-line credential's latest use went.
    pub fn cli_used(&mut self, id: u32, last: &str) {
        if let Some(c) = self.cli.iter_mut().find(|c| c.id == id) {
            c.last = Some(last.to_owned());
        }
        let _ = self.keep_cli();
    }

    /// Any other secret: the mailbox's, a command-line credential's.
    pub fn secret(&self, account: &str) -> Result<String, String> {
        self.secrets.get_named(account)
    }

    pub fn set_secret(&mut self, account: &str, value: Option<&str>) -> Result<(), String> {
        match value {
            Some(v) => self.secrets.set_named(account, v),
            None => self.secrets.delete_named(account),
        }
    }

    /// A key a squad's member obtained, kept under `equipe.secrets:<squad>/<name>`
    /// ([`crate::squad_secrets`]); its facts are in the squad's journal.
    pub fn set_squad_secret(&mut self, squad: &str, name: &str, value: &str) -> Result<(), String> {
        self.secrets.set_in(crate::squad_secrets::SERVICE, &crate::squad_secrets::account(squad, name), value)
    }

    pub fn squad_secret(&self, squad: &str, name: &str, interactive: bool) -> Result<String, String> {
        self.secrets.get_in(crate::squad_secrets::SERVICE, &crate::squad_secrets::account(squad, name), interactive)
    }

    pub fn delete_squad_secret(&mut self, squad: &str, name: &str) -> Result<(), String> {
        self.secrets.delete_in(crate::squad_secrets::SERVICE, &crate::squad_secrets::account(squad, name))
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

    /// Where `login` may be used without asking: exactly `squads` (names, or
    /// [`EVERY_SQUAD`]; empty: asks every time). One change, so the page's
    /// "this squad / all squads / ask first" never passes through a state
    /// between.
    pub fn set_scope(&mut self, id: u32, squads: &[String]) -> Result<(), String> {
        let login = self.logins.iter_mut().find(|l| l.id == id).ok_or("that login is gone")?;
        let mut scope: Vec<String> = Vec::new();
        for who in squads {
            let who = scope_of(who);
            if !who.is_empty() && !scope.contains(&who) {
                scope.push(who);
            }
        }
        // Every squad covers any one squad.
        if scope.iter().any(|w| w == EVERY_SQUAD) {
            scope = vec![EVERY_SQUAD.to_owned()];
        }
        login.allowed = scope;
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

    /// Offer keeping a login for `site` again: the person took the "never" back.
    pub fn forget_never(&mut self, site: &str) -> Result<(), String> {
        let site = site_of(site);
        self.never_offer.retain(|s| *s != site);
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
    /// Tick the form's "Remember me" / "Keep me signed in" / "Trust this
    /// device" box (the login's [`Autopilot::remember`]).
    pub remember: bool,
    /// A "Sign in with …" fill on an identity provider's own page
    /// ([`FillGuard::idp`]): the account (lowercased) the page must be
    /// about. The scripts then refuse consent and permission screens and
    /// steps they don't recognize, pick the account in a chooser only by
    /// its exact address, and type only into the provider's identity fields.
    pub account: Option<String>,
}

impl FillGuard {
    /// A member's `sign_in`: the login's site and its subdomains, as
    /// [`serves`] has it.
    pub fn serving(login: &Login) -> Self {
        Self { hosts: auto_hosts(login), subdomains: true, http: true, remember: login.autopilot.remember, account: None }
    }

    /// equipe by itself: those exact hosts, https unless the login's own
    /// sign-in page is http or the host is this machine.
    pub fn exact(login: &Login) -> Self {
        let hosts = auto_hosts(login);
        let http = login.url.as_deref().is_some_and(|u| u.trim().to_ascii_lowercase().starts_with("http://")) || hosts.iter().any(|h| local(h));
        Self { hosts, subdomains: false, http, remember: login.autopilot.remember, account: None }
    }

    /// equipe on an identity provider's page for another site
    /// ([`idp_login_for`]): the provider's host alone, https only, no
    /// "remember" box, and identity only — for this login's account.
    pub fn idp(login: &Login) -> Self {
        Self { hosts: vec![site_of(&login.site)], subdomains: false, http: false, remember: false, account: Some(login.username.trim().to_lowercase()) }
    }
}

/// The mark the fill script's answer starts with: put together when the
/// script runs, so the script's own text (which a browser tool may echo
/// back beside the answer) never carries it.
pub(crate) const ANSWER_MARK: &str = "EQUIPE_FILL:";

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
  const HOSTS = {hosts}, SUB = {sub}, HTTP = {http}, REMEMBER = {remember};
  const host = location.hostname.toLowerCase().replace(/^www\./, '');
  const here = HOSTS.some((h) => host === h || (SUB && (host.endsWith('.' + h) || h.endsWith('.' + host))));
  if (!here) return say({{ refused: 'host', filled: [], submitted: false }});
  if (location.protocol !== 'https:' && !(HTTP && location.protocol === 'http:')) return say({{ refused: 'insecure', filled: [], submitted: false }});
  const U = {u}, P = {p};
  const shown = (el) => {{ const r = el.getBoundingClientRect(); const cs = getComputedStyle(el); return r.width > 0 && r.height > 0 && cs.visibility !== 'hidden' && cs.display !== 'none' && !el.disabled && !el.readOnly; }};
{idp}  const set = (el, v) => {{
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
  // On a provider's page: only its own identity field, never a guess.
  if (IDP) user = inputs.find((i) => textual(i) && (i.type === 'email' || /username|email/i.test(i.autocomplete || '') || /^(identifier|loginfmt|email|username|login)$/i.test(i.name || i.id || '')));
  if (pass && !user && !IDP) {{ const before = inputs.slice(0, inputs.indexOf(pass)).filter(textual); user = before[before.length - 1]; }}
  if (!pass && !user) return say(IDP ? pick() : {{ filled: [], submitted: false }});
  if (IDP && user && user.value && user.value.trim().toLowerCase() !== ACCOUNT) return say({{ refused: 'account', filled: [], submitted: false }});
  if (IDP && pass && !user && !SHOWS_ACCOUNT) return say({{ refused: 'account', filled: [], submitted: false }});
  const filled = [];
  if (user && !user.value) {{ user.focus(); set(user, U); filled.push('username'); }} else if (user) {{ filled.push('username (already there)'); }}
  if (pass) {{ pass.focus(); set(pass, P); filled.push('password'); }}
  const remembered = REMEMBER ? ({remember_js})((pass || user).form || document) : 0;
  const form = (pass || user).form;
  const button = (form || document).querySelector('button[type=submit], input[type=submit], button:not([type])');
  let submitted = false;
  if (form && form.requestSubmit) {{ try {{ form.requestSubmit(button || undefined); submitted = true; }} catch (e) {{}} }}
  if (!submitted && button) {{ button.click(); submitted = true; }}
  return say({{ filled, submitted, step: pass ? 'password' : 'username', remembered }});
}}"#,
        sub = guard.subdomains,
        http = guard.http,
        remember = guard.remember,
        remember_js = crate::autopilot::fill::remember_js(),
        idp = crate::autopilot::fill::idp_js(guard),
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
    /// `password`, `username`, `code` or `passkey`: which step of a sign-in it was.
    pub step: Option<String>,
    /// "Remember me" / "Trust this device" boxes it ticked.
    pub remembered: u32,
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
        LoginDraft { id: None, site: site.into(), username: user.into(), password: password.map(Into::into), note: None, auto: None, idp: None }
    }

    fn login(site: &str, url: Option<&str>, auto: bool) -> Login {
        Login { id: 1, site: site.into(), url: url.map(Into::into), username: "abel".into(), note: None, allowed: Vec::new(), auto, stopped: None, created_at: 0, password_at: None, uses: Vec::new(), autopilot: Default::default() }
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
        vault.save(LoginDraft { id: Some(id), site: "github.com".into(), username: "abel2".into(), password: None, note: Some(" work ".into()), auto: None, idp: None }, 2).unwrap();
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
    fn a_saved_provider_account_signs_in_for_other_sites_unless_turned_off() {
        let home = home();
        let mut vault = Vault::open(&home, Box::new(InMemory::default()));
        let google = vault.save(draft("accounts.google.com", "abel@example.com", Some("pw")), 1).unwrap();
        let shop = vault.save(draft("shop.example", "abel", Some("pw")), 1).unwrap();
        let of = |id: u32| vault.logins.iter().find(|l| l.id == id).unwrap().autopilot.idp;
        assert!(of(google), "a provider's own account is for 'Sign in with Google' too");
        assert!(!of(shop), "an ordinary site's login has no such use");
        assert!(idp_login_for(&vault.logins, "https://accounts.google.com/v3/signin/identifier?continue=x").is_some());
        assert!(idp_login_for(&vault.logins, "https://accounts.google.com/signin/oauth/consent?client_id=x").is_none(), "never a consent page");
    }

    #[test]
    fn signing_in_by_itself_is_the_persons_to_turn_on_and_stops_after_a_failed_try() {
        let home = home();
        let mut vault = Vault::open(&home, Box::new(InMemory::default()));
        let id = vault.save(draft("github.com", "abel", Some("pw")), 1).unwrap();
        assert!(vault.logins[0].auto, "saving it is the person's leave: on for a new login");
        let off = vault.save(LoginDraft { auto: Some(false), ..draft("gitlab.com", "abel", Some("pw")) }, 1).unwrap();
        assert!(!vault.logins.iter().find(|l| l.id == off).unwrap().auto, "unless they turned it off on the form");
        vault.set_auto(id, false).unwrap();
        assert!(auto_login_for(&vault.logins, "https://github.com/login").is_none());
        vault.set_auto(id, true).unwrap();
        assert!(auto_login_for(&vault.logins, "https://github.com/login").is_some());
        vault.stop_auto(id, "the form came back").unwrap();
        assert!(auto_login_for(&vault.logins, "https://github.com/login").is_none(), "stopped");
        // A new password is worth a try again.
        vault.save(LoginDraft { id: Some(id), site: "github.com".into(), username: "abel".into(), password: Some("pw2".into()), note: None, auto: None, idp: None }, 2).unwrap();
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
    fn identity_providers_are_their_own_exact_hosts() {
        assert_eq!(idp_provider("https://accounts.google.com/v3/signin"), Some("Google"));
        assert_eq!(idp_provider("login.microsoftonline.com"), Some("Microsoft"));
        assert_eq!(idp_provider("acme.okta.com"), Some("Okta"));
        for not in ["google.com", "mail.google.com", "accounts.google.com.evil.io", "evil.acme.okta.com", "okta.com", "example.com"] {
            assert_eq!(idp_provider(not), None, "{not}");
        }
        assert!(idp_grant_page("https://accounts.google.com/signin/oauth/consent?authuser=0"));
        assert!(idp_grant_page("https://accounts.google.com/o/oauth2/approval/v2"));
        assert!(idp_grant_page("https://login.microsoftonline.com/common/Consent/Set"));
        assert!(!idp_grant_page("https://accounts.google.com/v3/signin/challenge/pwd?redirect_uri=https%3A%2F%2Fapp.example.com%2Fconsent"), "only the path counts");
    }

    #[test]
    fn sign_in_with_a_provider_needs_its_own_login_the_persons_switch_and_its_exact_host() {
        let google = |idp: bool| Login { site: "accounts.google.com".into(), username: "abel@example.com".into(), autopilot: crate::protocol::vault::Autopilot { idp, ..Default::default() }, ..login("accounts.google.com", None, true) };
        // Google asking its password again for a company dashboard's "Sign in with Google".
        let handoff = "https://accounts.google.com/v3/signin/challenge/pwd?client_id=123.apps.googleusercontent.com&redirect_uri=https%3A%2F%2Fdash.example.com%2Fauth&response_type=code";
        assert!(hands_off_elsewhere(handoff));
        assert!(auto_login_for(&[google(true)], handoff).is_none(), "the general rule still hands it off");
        assert_eq!(idp_login_for(&[google(true)], handoff).map(|l| l.id), Some(1));
        assert!(idp_login_for(&[google(false)], handoff).is_none(), "off by default: the person's switch");
        assert!(idp_login_for(&[Login { auto: false, ..google(true) }], handoff).is_none(), "and it signs in by itself");
        assert!(idp_login_for(&[Login { stopped: Some("x".into()), ..google(true) }], handoff).is_none(), "not stopped");
        assert!(idp_login_for(&[google(true), Login { id: 2, username: "other@example.com".into(), ..google(true) }], handoff).is_none(), "two accounts: the person's choice");
        // A login for any other site never qualifies, switch or not.
        let dash = Login { autopilot: crate::protocol::vault::Autopilot { idp: true, ..Default::default() }, ..login("google.com", None, true) };
        assert!(idp_login_for(&[dash], handoff).is_none());
        for elsewhere in [
            "http://accounts.google.com/v3/signin/challenge/pwd?client_id=x",
            "https://accounts.google.com.evil.io/v3/signin/challenge/pwd?client_id=x",
            "https://myaccount.google.com/signin?client_id=x",
            "https://evil.io/accounts.google.com?client_id=x",
            "https://accounts.google.com/signin/oauth/consent?client_id=x",
        ] {
            assert!(idp_login_for(&[google(true)], elsewhere).is_none(), "{elsewhere}");
        }
        // What it may do there: the provider's host alone, https, no remember box, this account.
        let guard = FillGuard::idp(&google(true));
        assert_eq!(guard, FillGuard { hosts: vec!["accounts.google.com".into()], subdomains: false, http: false, remember: false, account: Some("abel@example.com".into()) });
        let script = fill_script("abel@example.com", "pw", &guard);
        assert!(script.contains("const IDP = true") && script.contains("refused: 'grant'") && script.contains("refused: 'account'"));
        assert!(fill_script("a", "p", &FillGuard::exact(&login("github.com", None, true))).contains("const IDP = false"), "a site's own sign-in is as before");
    }

    #[test]
    fn autopilot_secrets_live_in_the_secret_store_and_the_files_say_only_whether() {
        let home = home();
        let mut vault = Vault::open(&home, Box::new(InMemory::default()));
        let id = vault.save(draft("github.com", "abel", Some("pw")), 1).unwrap();
        let seed = "otpauth://totp/equipe?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&algorithm=SHA1&digits=6&period=30";
        vault.set_totp(id, Some(seed)).unwrap();
        vault.set_passkey(id, Some(r#"{"privateKey":"PRIVATE"}"#)).unwrap();
        vault.save_mailbox(Some(MailboxDraft { host: "imap.example.com".into(), port: 993, username: "me".into(), password: Some("app-pass".into()) })).unwrap();
        let cli = vault.save_cli(CliLoginDraft { id: None, kind: CliKind::Env, name: "NPM_TOKEN".into(), host: String::new(), public: String::new(), squad: None, secret: Some("npm_x".into()) }, 1).unwrap();
        for file in ["vault.json", "autopilot.json", "cli-logins.json"] {
            let text = std::fs::read_to_string(home.join(file)).unwrap();
            for secret in ["GEZDGNBV", "PRIVATE", "app-pass", "npm_x"] {
                assert!(!text.contains(secret), "{file}: {text}");
            }
        }
        let a = &vault.logins[0].autopilot;
        assert!(a.has_totp && a.totp && a.has_passkey && a.passkey, "giving a seed or a passkey is turning it on");
        assert_eq!(vault.totp(id).unwrap(), seed);
        assert_eq!(vault.secret(&account::cli(cli)).unwrap(), "npm_x");
        assert_eq!(vault.secret(account::MAILBOX).unwrap(), "app-pass");
        // A screen's command can't say a seed is kept.
        vault.set_autopilot(id, &crate::protocol::vault::Autopilot { totp: false, has_totp: false, keepalive: true, ..Default::default() }).unwrap();
        let a = &vault.logins[0].autopilot;
        assert!(a.has_totp && !a.totp && a.keepalive);
        // Kept across a restart; deleting the login takes its seed and passkey.
        let again = Vault::open(&home, Box::new(InMemory::default()));
        assert_eq!((again.settings.mailbox.as_ref().map(|m| m.host.as_str()), again.cli.len()), (Some("imap.example.com"), 1));
        vault.delete(id).unwrap();
        assert!(vault.totp(id).is_err() && vault.passkey(id).is_err());
        assert!(vault.save_cli(CliLoginDraft { id: None, kind: CliKind::Env, name: "bad name".into(), host: String::new(), public: String::new(), squad: None, secret: Some("x".into()) }, 1).is_err());
        vault.save_mailbox(None).unwrap();
        assert!(vault.secret(account::MAILBOX).is_err() && vault.settings.mailbox.is_none());
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
    fn a_never_is_taken_back_and_stays_taken_back() {
        let home = home();
        let mut vault = Vault::open(&home, Box::new(InMemory::default()));
        vault.never_offer("github.com").unwrap();
        vault.never_offer("gitlab.com").unwrap();
        vault.forget_never("https://www.github.com/login").unwrap();
        assert!(!vault.refused_offer("github.com") && vault.refused_offer("gitlab.com"));
        let again = Vault::open(&home, Box::new(InMemory::default()));
        assert_eq!(again.never_offer, ["gitlab.com"]);
        // A site never said no to: nothing to take back, nothing breaks.
        vault.forget_never("example.com").unwrap();
    }

    #[test]
    fn scope_is_set_in_one_change_and_every_squad_covers_the_rest() {
        let home = home();
        let mut vault = Vault::open(&home, Box::new(InMemory::default()));
        let id = vault.save(draft("github.com", "abel", Some("pw")), 100).unwrap();
        vault.set_scope(id, &["acme".into(), "acme/qa".into(), "shop".into()]).unwrap();
        assert_eq!(vault.logins[0].allowed, ["acme", "shop"], "an old squad/member entry is its squad, once");
        vault.set_scope(id, &["acme".into(), EVERY_SQUAD.into()]).unwrap();
        assert_eq!(vault.logins[0].allowed, ["*"]);
        vault.set_scope(id, &[]).unwrap();
        assert!(vault.logins[0].allowed.is_empty(), "empty: asks every time");
        assert!(vault.set_scope(99, &[]).is_err());
        assert!(Vault::open(&home, Box::new(InMemory::default())).logins[0].allowed.is_empty(), "kept");
    }

    #[test]
    fn a_new_password_is_dated_and_changing_the_rest_leaves_the_date() {
        let home = home();
        let mut vault = Vault::open(&home, Box::new(InMemory::default()));
        let id = vault.save(draft("github.com", "abel", Some("pw")), 100).unwrap();
        assert_eq!(vault.logins[0].password_at, Some(100));
        let mut change = draft("github.com", "abel2", None);
        change.id = Some(id);
        vault.save(change, 200).unwrap();
        assert_eq!((vault.logins[0].username.as_str(), vault.logins[0].password_at), ("abel2", Some(100)));
        let mut replace = draft("github.com", "abel2", Some("new"));
        replace.id = Some(id);
        vault.save(replace, 300).unwrap();
        assert_eq!(vault.logins[0].password_at, Some(300));
        assert_eq!(vault.password(id).unwrap(), "new");
    }

    #[test]
    fn the_fill_script_carries_the_values_as_json_checks_the_host_and_answers_without_them() {
        let guard = FillGuard::exact(&login("github.com", None, true));
        let script = fill_script("a\"b", "p'w\\</script>", &guard);
        assert!(script.contains(r#"const U = "a\"b", P = "p'w\\</script>";"#), "{script}");
        assert!(script.contains("requestSubmit"));
        assert!(script.contains(r#"const HOSTS = ["github.com"], SUB = false, HTTP = false, REMEMBER = true;"#), "{script}");
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
