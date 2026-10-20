//! Signing in to another machine once: after a password worked, and only if
//! the person left "don't ask again" on, the app makes a key of its own on
//! this Mac (`~/.equipe/ssh/id_ed25519`, made once, readable by the person
//! only) and adds its public half to that machine's `~/.ssh/authorized_keys`
//! over the connection just opened — so no password is asked for it. Every
//! later `ssh` offers that key (`Launcher::options`), so a connection lost to
//! sleep or a new network comes back without asking. No password is kept.

use std::path::PathBuf;

use crate::ssh;
use crate::target::SshHost;

/// The app's own key on this Mac.
pub fn key_path() -> PathBuf {
    equipe_providers::credentials::equipe_home().join("ssh").join("id_ed25519")
}

/// The machines the key went onto, a line each (`SshHost::spec`).
fn keyed_list() -> PathBuf {
    key_path().with_file_name("keyed")
}

/// Whether `host` has the app's key: its runs offer it.
pub fn keyed(host: &SshHost) -> bool {
    key_path().exists() && std::fs::read_to_string(keyed_list()).is_ok_and(|list| list.lines().any(|l| l == host.spec()))
}

/// What `ssh` is told for a machine that has the key: the key, then the
/// usual ones that are there — naming any key on the command line makes
/// `ssh` skip its defaults, which the person may sign in with elsewhere.
pub fn identity_options(host: &SshHost) -> Vec<String> {
    if !keyed(host) {
        return Vec::new();
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let defaults = ["id_ed25519", "id_ecdsa", "id_rsa"].map(|k| home.join(".ssh").join(k));
    std::iter::once(key_path())
        .chain(defaults.into_iter().filter(|p| p.exists()))
        .flat_map(|p| ["-o".to_owned(), format!("IdentityFile={}", p.display())])
        .collect()
}

/// The key, made if there is none yet: its public line.
pub fn ensure_key() -> Result<String, String> {
    let path = key_path();
    let public = path.with_extension("pub");
    if !path.exists() {
        let dir = path.parent().ok_or("no folder for the key")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let host = std::process::Command::new("hostname").arg("-s").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
        let out = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C"])
            .arg(format!("equipe@{}", if host.is_empty() { "mac" } else { &host }))
            .arg("-f")
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
        }
    }
    let line = std::fs::read_to_string(&public).map_err(|e| e.to_string())?;
    let line = line.trim().to_owned();
    if !is_public_key(&line) {
        return Err("the key's public half isn't readable".into());
    }
    Ok(line)
}

/// A public key line as `ssh-keygen` writes it: its type, its base64, a comment.
fn is_public_key(line: &str) -> bool {
    let mut parts = line.split_whitespace();
    let (Some(kind), Some(body)) = (parts.next(), parts.next()) else { return false };
    kind.starts_with("ssh-") && body.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
}

/// What the other machine runs: the key added once, the folder and file kept
/// private as `sshd` wants them; a key there already is left alone.
pub fn install_script(public: &str) -> String {
    let key = ssh::quote(public);
    format!(
        "umask 077; mkdir -p \"$HOME/.ssh\" && chmod 700 \"$HOME/.ssh\" && touch \"$HOME/.ssh/authorized_keys\" && chmod 600 \"$HOME/.ssh/authorized_keys\" && \
{{ grep -qxF {key} \"$HOME/.ssh/authorized_keys\" || printf '%s\\n' {key} >> \"$HOME/.ssh/authorized_keys\"; }}"
    )
}

/// Make the key if need be and add it on `host`, over the shared connection.
/// Blocks: call it on a thread.
pub fn install(host: &SshHost) -> Result<(), String> {
    let public = ensure_key()?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    runtime
        .block_on(ssh::Launcher::system(None).run(host, &install_script(&public)))
        .map_err(|f| f.detail().map_or_else(|| "the other machine didn't answer".to_owned(), str::to_owned))?;
    let mut list = std::fs::read_to_string(keyed_list()).unwrap_or_default();
    if !list.lines().any(|l| l == host.spec()) {
        list.push_str(&host.spec());
        list.push('\n');
        std::fs::write(keyed_list(), list).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_goes_in_once_and_only_a_real_key_does() {
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ0x+/= equipe@mini";
        assert!(is_public_key(key));
        assert!(!is_public_key("not a key"));
        assert!(!is_public_key("ssh-ed25519 AAA;rm -rf ~ x"));
        let script = install_script(key);
        assert!(script.contains("grep -qxF 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ0x+/= equipe@mini'"), "{script}");
        assert!(script.contains(">> \"$HOME/.ssh/authorized_keys\""), "added, never written over: {script}");
        assert!(script.contains("chmod 600"), "kept private: {script}");
    }
}
