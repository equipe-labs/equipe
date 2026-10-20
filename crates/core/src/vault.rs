//! Logins for members to sign in with (`protocol::vault`). What a screen may
//! see — site, username, note, who may use it, its latest uses — is kept in
//! `vault.json` in equipe's home (readable by the person only); the password
//! only in the daemon machine's Keychain (`equipe.vault`, account = its id).
//! A member never gets it: [`fill_script`] is run in the member's browser by
//! equipe, which types it in and submits the form, and the member hears
//! only how it went.

use std::path::{Path, PathBuf};

use crate::protocol::vault::{Login, LoginDraft, Use};

/// The Keychain service the passwords are kept under.
pub const SERVICE: &str = "equipe.vault";
/// Uses kept per login, the latest.
const USES_KEPT: usize = 20;

/// Where passwords go: the Keychain, or (in tests) memory.
pub trait Secrets: Send {
    fn set(&mut self, id: u32, password: &str) -> Result<(), String>;
    fn get(&self, id: u32) -> Result<String, String>;
    fn delete(&mut self, id: u32) -> Result<(), String>;
}

/// The daemon machine's Keychain.
pub struct Keychain;

#[cfg(target_os = "macos")]
impl Secrets for Keychain {
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

#[cfg(not(target_os = "macos"))]
impl Secrets for Keychain {
    fn set(&mut self, _: u32, _: &str) -> Result<(), String> {
        Err("logins need macOS's Keychain, which this machine doesn't have".into())
    }

    fn get(&self, _: u32) -> Result<String, String> {
        Err("logins need macOS's Keychain, which this machine doesn't have".into())
    }

    fn delete(&mut self, _: u32) -> Result<(), String> {
        Ok(())
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

/// The logins, as kept on disk (no passwords).
pub struct Vault {
    file: PathBuf,
    pub logins: Vec<Login>,
    secrets: Box<dyn Secrets>,
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

impl Vault {
    pub fn open(home: &Path, secrets: Box<dyn Secrets>) -> Self {
        let file = home.join("vault.json");
        let logins = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { file, logins, secrets }
    }

    fn keep(&self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.logins).map_err(|e| e.to_string())?;
        std::fs::write(&self.file, text).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&self.file, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
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
        let id = match draft.id {
            Some(id) => {
                let login = self.logins.iter_mut().find(|l| l.id == id).ok_or("that login is gone")?;
                login.site = site;
                login.url = url.or(login.url.take());
                login.username = username;
                login.note = note;
                id
            }
            None => {
                if draft.password.as_deref().is_none_or(str::is_empty) {
                    return Err("a new login needs its password".into());
                }
                let id = self.logins.iter().map(|l| l.id).max().unwrap_or(0) + 1;
                self.logins.push(Login { id, site, url, username, note, allowed: Vec::new(), created_at: now, uses: Vec::new() });
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

    pub fn allow(&mut self, id: u32, who: &str) -> Result<(), String> {
        if let Some(l) = self.logins.iter_mut().find(|l| l.id == id)
            && !l.allowed.iter().any(|w| w == who)
        {
            l.allowed.push(who.to_owned());
        }
        self.keep()
    }

    pub fn forget(&mut self, id: u32, who: &str) -> Result<(), String> {
        if let Some(l) = self.logins.iter_mut().find(|l| l.id == id) {
            l.allowed.retain(|w| w != who);
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

    /// The password, to be typed in by equipe — never to be handed on.
    pub fn password(&self, id: u32) -> Result<String, String> {
        self.secrets.get(id)
    }
}

/// What equipe runs in a member's page (Playwright's `browser_evaluate`) to
/// sign in: fills the username and password fields it finds — the password
/// one, and the username one before it, or one marked as a username or email
/// — as a person's typing would (the native setter, then `input` and
/// `change`), and submits their form. A page asking for the username alone
/// (the first step of a two-step sign-in) gets that, and submits. It answers
/// what it did as JSON — never the values.
pub fn fill_script(username: &str, password: &str) -> String {
    let u = serde_json::to_string(username).unwrap_or_default();
    let p = serde_json::to_string(password).unwrap_or_default();
    format!(
        r#"() => {{
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
  const named = inputs.find((i) => textual(i) && /user|login|email|account|id|아이디|이메일/i.test([i.autocomplete, i.name, i.id, i.placeholder, i.getAttribute('aria-label')].join(' ')));
  let user = named;
  if (pass && !user) {{ const before = inputs.slice(0, inputs.indexOf(pass)).filter(textual); user = before[before.length - 1]; }}
  if (!pass && !user) return JSON.stringify({{ filled: [], submitted: false }});
  const filled = [];
  if (user && !user.value) {{ user.focus(); set(user, U); filled.push('username'); }} else if (user) {{ filled.push('username (already there)'); }}
  if (pass) {{ pass.focus(); set(pass, P); filled.push('password'); }}
  const form = (pass || user).form;
  const button = (form || document).querySelector('button[type=submit], input[type=submit], button:not([type])');
  let submitted = false;
  if (form && form.requestSubmit) {{ try {{ form.requestSubmit(button || undefined); submitted = true; }} catch (e) {{}} }}
  if (!submitted && button) {{ button.click(); submitted = true; }}
  return JSON.stringify({{ filled, submitted, step: pass ? 'password' : 'username' }});
}}"#
    )
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
        LoginDraft { id: None, site: site.into(), username: user.into(), password: password.map(Into::into), note: None }
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
    fn the_password_goes_to_the_keychain_alone_and_the_file_holds_the_rest() {
        let home = home();
        let mut vault = Vault::open(&home, Box::new(InMemory::default()));
        assert!(vault.save(draft("github.com", "abel", None), 1).is_err(), "a new login needs its password");
        let id = vault.save(draft("https://github.com/login", "abel", Some("hunter2")), 1).unwrap();
        let file = std::fs::read_to_string(home.join("vault.json")).unwrap();
        assert!(!file.contains("hunter2"), "{file}");
        assert!(file.contains("github.com") && file.contains("https://github.com/login"));
        assert_eq!(vault.password(id).unwrap(), "hunter2");
        // A change without a password keeps the one kept.
        vault.save(LoginDraft { id: Some(id), site: "github.com".into(), username: "abel2".into(), password: None, note: Some(" work ".into()) }, 2).unwrap();
        assert_eq!(vault.password(id).unwrap(), "hunter2");
        let l = &vault.logins[0];
        assert_eq!((l.username.as_str(), l.note.as_deref(), l.url.as_deref()), ("abel2", Some("work"), Some("https://github.com/login")));
        // Kept across a restart.
        let again = Vault::open(&home, Box::new(InMemory::default()));
        assert_eq!(again.logins.len(), 1);
        // Who may, and what happened.
        vault.allow(id, "acme/frontend").unwrap();
        vault.allow(id, "acme/frontend").unwrap();
        assert_eq!(vault.logins[0].allowed, ["acme/frontend"]);
        vault.used(id, 5, "acme", "frontend", "signed_in");
        assert_eq!(vault.logins[0].uses[0].outcome, "signed_in");
        vault.forget(id, "acme/frontend").unwrap();
        assert!(vault.logins[0].allowed.is_empty());
        assert_eq!(vault.for_site("https://github.com/x").len(), 1);
        vault.delete(id).unwrap();
        assert!(vault.logins.is_empty() && vault.password(id).is_err());
    }

    #[test]
    fn the_fill_script_carries_the_values_as_json_and_answers_without_them() {
        let script = fill_script("a\"b", "p'w\\</script>");
        assert!(script.contains(r#"const U = "a\"b", P = "p'w\\</script>";"#), "{script}");
        assert!(script.contains("requestSubmit"));
        assert!(script.contains("JSON.stringify({ filled, submitted"), "only what it did comes back");
    }
}
