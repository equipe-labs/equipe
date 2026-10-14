//! Putting the helper on the Mac: an app bundle with a fixed bundle id and a
//! fixed signature, run as a LaunchAgent in the person's GUI session.
//!
//! macOS keeps a permission for the code requirement it recorded. equipe ships
//! ad-hoc signed, whose requirement is the binary's hash — every build would
//! lose the grants. Signed with the same certificate under the same identifier,
//! every build of the helper satisfies the same requirement and keeps them
//! (T-20 proved it). Until equipe has a Developer ID, the certificate is a
//! self-signed one made on this Mac and kept in a keychain of its own;
//! [`Signer::Identity`] signs with any identity instead.
//!
//! The bundle's executable is a copy of `equipe` itself, run as `equipe desktop
//! serve`. It lives in `~/Applications`: System Settings lists only apps
//! Launch Services knows, which leaves out bundles in hidden folders like
//! `~/.equipe`. A new build is made beside the old bundle and swapped in, not
//! written over it: macOS's App Management blocks rewriting an app bundle.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::helper;

pub const BUNDLE_ID: &str = "im.equipe.desktop-helper";
pub const LABEL: &str = BUNDLE_ID;
pub const APP_NAME: &str = "equipe Desktop Helper";
pub const EXECUTABLE: &str = "equipe-desktop-helper";
const CERT_NAME: &str = "equipe Desktop Helper (self-signed)";

/// How the helper is signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signer {
    /// A certificate made on this Mac, in a keychain under `~/.equipe/desktop/signing`.
    SelfSigned,
    /// A signing identity from the person's keychains, by name or hash
    /// ("Developer ID Application: …").
    Identity(String),
}

impl Signer {
    /// The signer the person chose last (`equipe desktop setup --sign`).
    pub fn chosen(home: &Path) -> Self {
        match std::fs::read_to_string(signer_file(home)).map(|s| s.trim().to_owned()) {
            Ok(name) if !name.is_empty() && name != "self" => Self::Identity(name),
            _ => Self::SelfSigned,
        }
    }

    pub fn remember(&self, home: &Path) -> std::io::Result<()> {
        helper::ensure_dir(home)?;
        let name = match self {
            Self::SelfSigned => "self",
            Self::Identity(name) => name,
        };
        std::fs::write(signer_file(home), name)
    }

    pub fn describe(&self) -> String {
        match self {
            Self::SelfSigned => "a self-signed certificate made on this Mac".into(),
            Self::Identity(name) => name.clone(),
        }
    }
}

fn signer_file(home: &Path) -> PathBuf {
    helper::dir(home).join("signer")
}

fn signing_dir(home: &Path) -> PathBuf {
    helper::dir(home).join("signing")
}

pub fn plist_path() -> PathBuf {
    dirs_home().join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn uid() -> String {
    run_out("id", &["-u"]).unwrap_or_else(|_| "501".into())
}

pub fn gui_domain() -> String {
    format!("gui/{}", uid())
}

/// Where the helper's bundle lives.
pub fn bundle_path() -> PathBuf {
    dirs_home().join("Applications").join(format!("{APP_NAME}.app"))
}

const LSREGISTER: &str =
    "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

pub fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
<key>CFBundleName</key><string>{APP_NAME}</string>
<key>CFBundleExecutable</key><string>{EXECUTABLE}</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>LSUIElement</key><true/>
<key>NSScreenCaptureUsageDescription</key><string>equipe's members see the screen to work desktop apps for you.</string>
<key>NSAppleEventsUsageDescription</key><string>equipe's members work desktop apps for you.</string>
</dict></plist>
"#
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The LaunchAgent: only in the GUI session (`Aqua`), the only one that sees
/// the window server, and back up if it ever stops.
pub fn agent_plist(exe: &Path, home: &Path, log: &Path) -> String {
    let (exe, home, log) = (escape(&exe.to_string_lossy()), escape(&home.to_string_lossy()), escape(&log.to_string_lossy()));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{exe}</string><string>desktop</string><string>serve</string><string>--home</string><string>{home}</string></array>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
  <key>ProcessType</key><string>Interactive</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#
    )
}

/// The executable the installed agent runs, read back from its plist.
pub fn installed_exe() -> Option<PathBuf> {
    let plist = std::fs::read_to_string(plist_path()).ok()?;
    let args = plist.split("<array>").nth(1)?.split("</array>").next()?;
    let first = args.split("<string>").nth(1)?.split("</string>").next()?;
    Some(PathBuf::from(first.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")))
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{program} {}: {}", args.first().copied().unwrap_or(""), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn run_out(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(format!("{program}: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// What installing did.
pub struct Installed {
    pub app: PathBuf,
    /// A new bundle was made (the helper is a new build).
    pub changed: bool,
    pub requirement: String,
}

/// Put `source` (this equipe) in place as the helper, signed by `signer`, and
/// (re)load its LaunchAgent. A bundle already holding this build is kept.
pub fn install(home: &Path, source: &Path, version: &str, signer: &Signer) -> Result<Installed, String> {
    helper::ensure_dir(home).map_err(|e| e.to_string())?;
    helper::ensure_token(home).map_err(|e| e.to_string())?;
    let app = bundle_path();
    let exe = app.join("Contents/MacOS").join(EXECUTABLE);
    let same_build = std::fs::read(&exe).ok().zip(std::fs::read(source).ok()).is_some_and(|(a, b)| a == b);
    let current = same_build && run("codesign", &["--verify", &app.to_string_lossy()]).is_ok();
    let changed = !current;
    if changed {
        // Made beside the old one, then swapped in.
        let staged = app.with_file_name(format!(".{APP_NAME}.new.app"));
        if staged.exists() {
            std::fs::remove_dir_all(&staged).map_err(|e| format!("Couldn't clear {}: {e}", staged.display()))?;
        }
        let staged_exe = staged.join("Contents/MacOS").join(EXECUTABLE);
        std::fs::create_dir_all(staged_exe.parent().expect("MacOS has a parent")).map_err(|e| e.to_string())?;
        std::fs::copy(source, &staged_exe).map_err(|e| format!("Couldn't copy equipe into the helper: {e}"))?;
        std::fs::write(staged.join("Contents/Info.plist"), info_plist(version)).map_err(|e| e.to_string())?;
        sign(home, &staged, signer)?;
        let _ = run("launchctl", &["bootout", &format!("{}/{LABEL}", gui_domain())]);
        if app.exists() {
            let old = app.with_file_name(format!(".{APP_NAME}.old.app"));
            let _ = std::fs::remove_dir_all(&old);
            std::fs::rename(&app, &old).map_err(|e| {
                format!("Couldn't replace {} ({e}). If macOS says it blocked modifying apps, allow App Management for your terminal, or delete the old helper and run setup again.", app.display())
            })?;
            let _ = std::fs::remove_dir_all(&old);
        }
        std::fs::rename(&staged, &app).map_err(|e| format!("Couldn't put the helper in {}: {e}", app.display()))?;
        // Known to Launch Services, so System Settings lists it.
        let _ = run(LSREGISTER, &["-f", &app.to_string_lossy()]);
    }
    let requirement = requirement(&app);
    if changed || installed_exe().is_none_or(|e| e != exe) {
        let plist = plist_path();
        std::fs::create_dir_all(plist.parent().expect("LaunchAgents has a parent")).map_err(|e| e.to_string())?;
        std::fs::write(&plist, agent_plist(&exe, home, &helper::log_path(home))).map_err(|e| e.to_string())?;
        let _ = run("launchctl", &["bootout", &format!("{}/{LABEL}", gui_domain())]);
        run("launchctl", &["bootstrap", &gui_domain(), &plist.to_string_lossy()])?;
    } else if !loaded() {
        run("launchctl", &["bootstrap", &gui_domain(), &plist_path().to_string_lossy()])?;
    }
    Ok(Installed { app, changed, requirement })
}

/// The requirement macOS will record for the helper's permissions.
fn requirement(app: &Path) -> String {
    let Ok(out) = Command::new("codesign").args(["-d", "-r-"]).arg(app).output() else {
        return String::new();
    };
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    text.lines().find(|l| l.contains("designated")).unwrap_or("").trim().to_owned()
}

/// Whether the helper's agent is loaded in the GUI session.
pub fn loaded() -> bool {
    run("launchctl", &["print", &format!("{}/{LABEL}", gui_domain())]).is_ok()
}

/// Restart the helper: a permission given while it ran is seen by the next process.
pub fn restart() -> Result<(), String> {
    run("launchctl", &["kickstart", "-k", &format!("{}/{LABEL}", gui_domain())])
}

/// Stop the helper and remove it: its agent, its bundle, its socket.
pub fn uninstall(home: &Path) -> bool {
    let had = plist_path().is_file();
    let _ = run("launchctl", &["bootout", &format!("{}/{LABEL}", gui_domain())]);
    let _ = std::fs::remove_file(plist_path());
    let _ = run(LSREGISTER, &["-u", &bundle_path().to_string_lossy()]);
    let _ = std::fs::remove_dir_all(bundle_path());
    let _ = std::fs::remove_file(helper::socket_path(home));
    had
}

/// Whether someone is logged in at the Mac's screen: the GUI session exists.
pub fn gui_session() -> bool {
    run("launchctl", &["print", &gui_domain()]).is_ok()
}

/// Whether the squad's daemon is a login service in the GUI session.
pub fn daemon_in_aqua(label: &str) -> bool {
    run("launchctl", &["print", &format!("{}/{label}", gui_domain())]).is_ok()
}

fn sign(home: &Path, app: &Path, signer: &Signer) -> Result<(), String> {
    let app = app.to_string_lossy();
    match signer {
        Signer::Identity(identity) => run("codesign", &["--force", "--options", "runtime", "-s", identity, "-i", BUNDLE_ID, &app]),
        Signer::SelfSigned => {
            let (keychain, hash) = self_signed(home)?;
            let requirement = format!("designated => identifier \"{BUNDLE_ID}\" and certificate leaf = H\"{hash}\"");
            // codesign finds identities only through the search list: ours is
            // added for this one call, then the list goes back as it was.
            let before: Vec<String> = run_out("security", &["list-keychains", "-d", "user"])?
                .lines()
                .map(|l| l.trim().trim_matches('"').to_owned())
                .filter(|l| !l.is_empty())
                .collect();
            let mut with: Vec<&str> = vec!["list-keychains", "-d", "user", "-s"];
            with.extend(before.iter().map(String::as_str));
            with.push(&keychain);
            run("security", &with)?;
            let signed = run("codesign", &["--force", "--keychain", &keychain, "-s", &hash, "-i", BUNDLE_ID, &format!("-r={requirement}"), &app]);
            let mut back: Vec<&str> = vec!["list-keychains", "-d", "user", "-s"];
            back.extend(before.iter().map(String::as_str));
            let restored = run("security", &back);
            signed.and(restored)
        }
    }
}

/// The self-signed certificate's keychain and SHA-1, made on first use. The
/// key never leaves the keychain; nothing is marked trusted.
fn self_signed(home: &Path) -> Result<(String, String), String> {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = signing_dir(home);
    let keychain = dir.join("signing.keychain-db");
    let password_file = dir.join("password");
    let keychain_s = keychain.to_string_lossy().into_owned();
    if !keychain.is_file() {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        let password = helper::random_hex(16).map_err(|e| e.to_string())?;
        std::fs::write(&password_file, &password).map_err(|e| e.to_string())?;
        let _ = std::fs::set_permissions(&password_file, std::fs::Permissions::from_mode(0o600));
        let cnf = dir.join("cert.cnf");
        std::fs::write(
            &cnf,
            format!(
                "[req]\ndistinguished_name = dn\nx509_extensions = ext\nprompt = no\n[dn]\nCN = {CERT_NAME}\n[ext]\nbasicConstraints = critical,CA:false\nkeyUsage = critical,digitalSignature\nextendedKeyUsage = critical,codeSigning\n"
            ),
        )
        .map_err(|e| e.to_string())?;
        let (key, cert, p12) = (dir.join("key.pem"), dir.join("cert.pem"), dir.join("cert.p12"));
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let made = (|| {
            run(
                "/usr/bin/openssl",
                &["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "3650", "-config", &s(&cnf), "-keyout", &s(&key), "-out", &s(&cert)],
            )?;
            let pass = format!("pass:{password}");
            run("/usr/bin/openssl", &["pkcs12", "-export", "-inkey", &s(&key), "-in", &s(&cert), "-passout", &pass, "-out", &s(&p12)])?;
            run("security", &["create-keychain", "-p", &password, &keychain_s])?;
            run("security", &["set-keychain-settings", &keychain_s])?;
            run("security", &["unlock-keychain", "-p", &password, &keychain_s])?;
            run("security", &["import", &s(&p12), "-k", &keychain_s, "-P", &password, "-T", "/usr/bin/codesign"])?;
            run("security", &["set-key-partition-list", "-S", "apple-tool:,apple:", "-s", "-k", &password, &keychain_s])
        })();
        let _ = std::fs::remove_file(&key);
        let _ = std::fs::remove_file(&p12);
        if let Err(why) = made {
            let _ = std::fs::remove_file(&keychain);
            return Err(format!("Couldn't make the signing certificate: {why}"));
        }
    }
    let password = std::fs::read_to_string(&password_file).map_err(|e| format!("The signing keychain's password is missing: {e}"))?;
    run("security", &["unlock-keychain", "-p", password.trim(), &keychain_s])?;
    let found = run_out("security", &["find-certificate", "-c", CERT_NAME, "-Z", &keychain_s])?;
    let hash = found
        .lines()
        .find(|l| l.contains("SHA-1"))
        .and_then(|l| l.split_whitespace().last())
        .ok_or("The signing certificate has no SHA-1 hash")?
        .to_owned();
    Ok((keychain_s, hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_runs_only_in_the_gui_session() {
        let plist = agent_plist(Path::new("/h/Applications/equipe Desktop Helper.app/Contents/MacOS/x"), Path::new("/h/.equipe"), Path::new("/h/log"));
        assert!(plist.contains("<key>LimitLoadToSessionType</key><string>Aqua</string>"));
        assert!(plist.contains("<string>desktop</string><string>serve</string><string>--home</string><string>/h/.equipe</string>"));
        let info = info_plist("0.11.0");
        assert!(info.contains(BUNDLE_ID) && info.contains("<key>LSUIElement</key><true/>") && info.contains("NSScreenCaptureUsageDescription"));
    }

    #[test]
    fn the_helper_lives_where_system_settings_lists_it() {
        let app = bundle_path();
        assert!(app.ends_with("Applications/equipe Desktop Helper.app"), "{}", app.display());
    }
}
