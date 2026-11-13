//! Putting the helper on the Mac: the process that holds the person's
//! Accessibility (and Screen Recording) permission, run as a LaunchAgent in
//! their GUI session.
//!
//! **One permission entry.** When equipe.app is installed, the helper *is*
//! equipe.app: its own executable, run by the LaunchAgent as
//! `equipe-desktop desktop serve` ([`Host::App`]). macOS attributes a
//! permission to the code that runs, so the one "equipe" row (with equipe's
//! icon) in System Settings covers the app, the helper, and agent-desktop
//! started by it. Nothing is copied or signed on the Mac, and the app's own
//! Developer ID signature keeps the grant across updates; Sparkle replaces the
//! app, the LaunchAgent's path stays the same.
//!
//! **Without the app** (the command line alone), the helper is a small app
//! bundle in `~/Applications` holding a copy of `equipe` ([`Host::Bundle`]),
//! named "equipe" with equipe's icon. macOS keeps a permission for the code
//! requirement it recorded; equipe's release binaries are signed with a
//! Developer ID, but a copy is re-signed here under the bundle's own
//! identifier, with the same certificate every time, so every build satisfies
//! the same requirement (T-20 proved it). Until a Developer ID is chosen
//! ([`Signer::Identity`]), the certificate is a self-signed one made on this
//! Mac, in a keychain of its own. The bundle is rebuilt only when the helper's
//! code changed ([`REVISION`]) — not on every update — since macOS's App
//! Management guards app bundles: a new build is made beside the old one and
//! swapped in by renaming, and an old bundle is moved aside, never written
//! into or deleted from.
//!
//! **Moving to the app.** A Mac that had the bundle and then gets equipe.app:
//! the LaunchAgent runs the app; while the app isn't allowed yet and the old
//! bundle is, the app hands over to the old bundle (`crate::serve_as_app`), so
//! nothing stops working. Once the app is allowed, the old bundle is moved out
//! of `~/Applications` ([`retire_bundle`]) and the person is told once that its
//! row can be removed.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::helper;

pub const BUNDLE_ID: &str = "im.equipe.desktop-helper";
pub const LABEL: &str = BUNDLE_ID;
/// The bundle's folder name, kept from before so a permission already given
/// to it stays where it is.
pub const APP_NAME: &str = "equipe Desktop Helper";
/// What the bundle is called on screen.
pub const DISPLAY_NAME: &str = "equipe";
pub const EXECUTABLE: &str = "equipe-desktop-helper";
/// The key in equipe.app's Info.plist saying its executable serves the helper
/// (`desktop serve`): an older app doesn't, and isn't used as the host.
pub const APP_SERVES_KEY: &str = "EquipeDesktopServe";
/// The key in the bundle's Info.plist with the helper code's [`REVISION`].
pub const REVISION_KEY: &str = "EquipeHelperRevision";
/// This build's helper code (a hash of this crate's sources, `build.rs`).
pub const REVISION: &str = env!("EQUIPE_HELPER_REVISION");
/// equipe's icon, for the bundle.
const ICON: &[u8] = include_bytes!("../../desktop/macos/AppIcon.icns");
/// What a signing failure's message starts with, for a screen to recognise.
pub const SIGNING_FAILED: &str = "signing failed:";
#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
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
<key>CFBundleDevelopmentRegion</key><string>en</string>
<key>LSHasLocalizedDisplayName</key><true/>
<key>CFBundleName</key><string>{DISPLAY_NAME}</string>
<key>CFBundleDisplayName</key><string>{DISPLAY_NAME}</string>
<key>CFBundleExecutable</key><string>{EXECUTABLE}</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>{REVISION_KEY}</key><string>{REVISION}</string>
<key>LSUIElement</key><true/>
<key>NSScreenCaptureUsageDescription</key><string>equipe's members see the screen to work desktop apps for you.</string>
<key>NSAppleEventsUsageDescription</key><string>equipe's members work desktop apps for you.</string>
</dict></plist>
"#
    )
}

/// A string value of `key` in an Info.plist's text.
pub fn plist_value(text: &str, key: &str) -> Option<String> {
    let after = text.split(&format!("<key>{key}</key>")).nth(1)?;
    let after = after.trim_start();
    if after.starts_with("<true/>") {
        return Some("true".into());
    }
    let value = after.strip_prefix("<string>")?.split("</string>").next()?;
    Some(value.trim().to_owned())
}

/// Where the helper runs from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Host {
    /// equipe.app itself: one permission entry, equipe's icon.
    App(PathBuf),
    /// The bundle in `~/Applications`: the command line alone.
    Bundle,
}

impl Host {
    /// What System Settings lists it as.
    pub fn name(&self) -> &'static str {
        match self {
            Host::App(_) => DISPLAY_NAME,
            Host::Bundle => {
                // A bundle made before it was named "equipe" is still listed as it was.
                let text = std::fs::read_to_string(bundle_path().join("Contents/Info.plist")).unwrap_or_default();
                if !text.is_empty() && plist_value(&text, REVISION_KEY).is_none() { APP_NAME } else { DISPLAY_NAME }
            }
        }
    }
}

/// The executable of `app` when it is an equipe.app that serves the helper.
pub fn app_executable(app: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(app.join("Contents/Info.plist")).ok()?;
    if plist_value(&text, APP_SERVES_KEY).as_deref() != Some("true") {
        return None;
    }
    let exe = app.join("Contents/MacOS").join(plist_value(&text, "CFBundleExecutable")?);
    exe.is_file().then_some(exe)
}

/// Whether `app` stays where it is: not on a disk image, not in the copy
/// macOS runs a downloaded app from before it is moved (App Translocation).
fn settled_app(app: &Path) -> bool {
    let text = app.to_string_lossy();
    !text.starts_with("/Volumes/") && !text.contains("/AppTranslocation/")
}

/// The equipe.app that hosts the helper on this Mac, if any:
/// `EQUIPE_DESKTOP_APP`, else the app this process runs from, else
/// `/Applications/equipe.app`, else `~/Applications/equipe.app`.
/// `EQUIPE_DESKTOP_APP=none` keeps the bundle.
pub fn app_host() -> Option<PathBuf> {
    if let Some(given) = std::env::var_os("EQUIPE_DESKTOP_APP") {
        if given == "none" {
            return None;
        }
        let app = PathBuf::from(given);
        return app_executable(&app).map(|_| app);
    }
    let own = std::env::current_exe().ok().and_then(|exe| exe.ancestors().find(|a| a.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf));
    own.into_iter()
        .chain([PathBuf::from("/Applications/equipe.app"), dirs_home().join("Applications/equipe.app")])
        .filter(|app| settled_app(app) && app != &bundle_path())
        .find(|app| app_executable(app).is_some())
}

/// Where the helper would run from now.
pub fn host() -> Host {
    app_host().map_or(Host::Bundle, Host::App)
}

/// The old bundle's executable, when it is there: what the app hands over to
/// while it isn't allowed yet.
pub fn bundle_executable() -> Option<PathBuf> {
    let exe = bundle_path().join("Contents/MacOS").join(EXECUTABLE);
    exe.is_file().then_some(exe)
}

/// What kind of process `exe` is, for the helper's status: `app`, `bundle`, or `cli`.
pub fn role_of(exe: &Path) -> &'static str {
    if exe.ends_with(Path::new("Contents/MacOS").join(EXECUTABLE)) {
        "bundle"
    } else if exe.ancestors().any(|a| a.extension().is_some_and(|e| e == "app") && app_executable(a).is_some_and(|x| x == exe)) {
        "app"
    } else {
        "cli"
    }
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
    /// Something changed: a new bundle, or the LaunchAgent pointed elsewhere.
    pub changed: bool,
    pub requirement: String,
    pub host: Host,
}

/// Write `text` to `path` only when it differs: rewriting a LaunchAgent's
/// plist is what makes macOS say again that an app "can run in the background".
fn write_if_changed(path: &Path, text: &str) -> Result<bool, String> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(text) {
        return Ok(false);
    }
    std::fs::create_dir_all(path.parent().expect("a plist has a folder")).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Point the LaunchAgent at `exe` and have it loaded: rewritten and reloaded
/// only when it ran something else.
fn agent(home: &Path, exe: &Path) -> Result<bool, String> {
    let plist = plist_path();
    let changed = write_if_changed(&plist, &agent_plist(exe, home, &helper::log_path(home)))?;
    if changed {
        let _ = run("launchctl", &["bootout", &format!("{}/{LABEL}", gui_domain())]);
        run("launchctl", &["bootstrap", &gui_domain(), &plist.to_string_lossy()])?;
    } else if !loaded() {
        run("launchctl", &["bootstrap", &gui_domain(), &plist.to_string_lossy()])?;
    }
    Ok(changed)
}

/// Put the helper in place and have its LaunchAgent loaded: equipe.app
/// itself when it is installed ([`app_host`]), else a bundle holding
/// `source` (this equipe), signed by `signer`.
pub fn install(home: &Path, source: &Path, version: &str, signer: &Signer) -> Result<Installed, String> {
    helper::ensure_dir(home).map_err(|e| e.to_string())?;
    helper::ensure_token(home).map_err(|e| e.to_string())?;
    if let Some(app) = app_host() {
        let exe = app_executable(&app).ok_or("equipe.app has no executable")?;
        let changed = agent(home, &exe)?;
        let requirement = requirement(&app);
        return Ok(Installed { app: app.clone(), changed, requirement, host: Host::App(app) });
    }
    let app = bundle_path();
    let exe = app.join("Contents/MacOS").join(EXECUTABLE);
    let revision = std::fs::read_to_string(app.join("Contents/Info.plist")).ok().and_then(|t| plist_value(&t, REVISION_KEY));
    let current = exe.is_file() && revision.as_deref() == Some(REVISION) && run("codesign", &["--verify", &app.to_string_lossy()]).is_ok();
    let mut changed = !current;
    if changed {
        // Made beside the old one, then swapped in by renaming.
        let staged = app.with_file_name(format!(".{APP_NAME}.new.app"));
        if staged.exists() {
            set_aside(home, &staged);
        }
        let staged_exe = staged.join("Contents/MacOS").join(EXECUTABLE);
        std::fs::create_dir_all(staged_exe.parent().expect("MacOS has a parent")).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(staged.join("Contents/Resources")).map_err(|e| e.to_string())?;
        std::fs::copy(source, &staged_exe).map_err(|e| format!("Couldn't copy equipe into the helper: {e}"))?;
        std::fs::write(staged.join("Contents/Info.plist"), info_plist(version)).map_err(|e| e.to_string())?;
        std::fs::write(staged.join("Contents/Resources/AppIcon.icns"), ICON).map_err(|e| e.to_string())?;
        // Listed as "equipe", whatever its folder is called.
        let en = staged.join("Contents/Resources/en.lproj");
        std::fs::create_dir_all(&en).map_err(|e| e.to_string())?;
        std::fs::write(en.join("InfoPlist.strings"), format!("\"CFBundleDisplayName\" = \"{DISPLAY_NAME}\";\n\"CFBundleName\" = \"{DISPLAY_NAME}\";\n")).map_err(|e| e.to_string())?;
        sign(home, &staged, signer)?;
        let _ = run("launchctl", &["bootout", &format!("{}/{LABEL}", gui_domain())]);
        if app.exists() && !set_aside(home, &app) {
            return Err(format!("Couldn't move the old helper out of the way ({}). Remove it, then run equipe desktop setup again.", app.display()));
        }
        std::fs::rename(&staged, &app).map_err(|e| format!("Couldn't put the helper in {}: {e}", app.display()))?;
        // Known to Launch Services, so System Settings lists it.
        let _ = run(LSREGISTER, &["-f", &app.to_string_lossy()]);
    }
    changed |= agent(home, &exe)?;
    let requirement = requirement(&app);
    Ok(Installed { app, changed, requirement, host: Host::Bundle })
}

/// Where old helper bundles are moved: equipe's own folder, out of
/// `~/Applications` and out of System Settings' sight.
fn retired_dir(home: &Path) -> PathBuf {
    helper::dir(home).join("retired")
}

/// Move `bundle` out of the way by renaming it — never by writing into it or
/// deleting it, which App Management guards. `true` once it is gone from
/// where it was.
fn set_aside(home: &Path, bundle: &Path) -> bool {
    let dir = retired_dir(home);
    let _ = std::fs::create_dir_all(&dir);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let name = bundle.file_name().map(|n| n.to_string_lossy().trim_start_matches('.').trim_end_matches(".app").to_owned()).unwrap_or_default();
    std::fs::rename(bundle, dir.join(format!("{name}-{stamp}.app"))).is_ok()
}

/// The old helper bundle, no longer used once equipe.app is allowed: forgotten
/// by Launch Services and moved out of `~/Applications`. `true` when it was there.
pub fn retire_bundle(home: &Path) -> bool {
    let app = bundle_path();
    if !app.exists() {
        return false;
    }
    let _ = run(LSREGISTER, &["-u", &app.to_string_lossy()]);
    set_aside(home, &app)
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
    // Asked for by the person: the bundle and the ones set aside go.
    let _ = std::fs::remove_dir_all(bundle_path());
    let _ = std::fs::remove_dir_all(retired_dir(home));
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
            let attempt = || -> Result<(), String> {
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
            };
            attempt().or_else(|first| {
                // errSecInternalComponent and its kin: the private keychain
                // locked again, or its key not open to codesign from this
                // session. Opened again with its own password, kept from
                // locking, the key opened to codesign — and once more.
                reopen(home, &keychain)?;
                attempt().map_err(|second| {
                    let first = if first == second { String::new() } else { format!("; first try: {first}") };
                    format!(
                        "{SIGNING_FAILED} macOS wouldn't let codesign use the helper's private signing key ({second}{first}). This happens when that keychain can't be opened from where equipe runs — for instance over SSH with nobody logged in at the Mac. Run equipe desktop setup at the Mac's own screen, or install equipe.app, which needs no signing."
                    )
                })
            })
        }
    }
}

/// The private signing keychain unlocked with its stored password, set never
/// to lock by itself, and its key open to codesign without asking anyone.
#[cfg(target_os = "macos")]
fn reopen(home: &Path, keychain: &str) -> Result<(), String> {
    let password = std::fs::read_to_string(signing_dir(home).join("password")).map_err(|e| format!("The signing keychain's password is missing: {e}"))?;
    let password = password.trim();
    run("security", &["unlock-keychain", "-p", password, keychain])?;
    // No timeout and no lock on sleep: a keychain that locks itself after five
    // minutes is what made a later signing fail.
    run("security", &["set-keychain-settings", keychain])?;
    run("security", &["set-key-partition-list", "-S", "apple-tool:,apple:", "-s", "-k", password, keychain]).map(|_| ())
}

#[cfg(not(target_os = "macos"))]
fn reopen(_home: &Path, _keychain: &str) -> Result<(), String> {
    Ok(())
}

/// The self-signed certificate's keychain and SHA-1, made on first use. The
/// key never leaves the keychain; nothing is marked trusted.
#[cfg(target_os = "macos")]
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
    // Never locking by itself (an older equipe made it with the default five-minute lock).
    let _ = run("security", &["set-keychain-settings", &keychain_s]);
    let found = run_out("security", &["find-certificate", "-c", CERT_NAME, "-Z", &keychain_s])?;
    let hash = found
        .lines()
        .find(|l| l.contains("SHA-1"))
        .and_then(|l| l.split_whitespace().last())
        .ok_or("The signing certificate has no SHA-1 hash")?
        .to_owned();
    Ok((keychain_s, hash))
}

#[cfg(not(target_os = "macos"))]
fn self_signed(_home: &Path) -> Result<(String, String), String> {
    Err("desktop control signing unsupported on Windows yet".into())
}

#[cfg(all(test, target_os = "macos"))]
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
    fn the_bundle_is_equipe_with_its_icon_and_says_its_revision() {
        let info = info_plist("1.0.0");
        assert_eq!(plist_value(&info, "CFBundleName").as_deref(), Some("equipe"));
        assert_eq!(plist_value(&info, "CFBundleDisplayName").as_deref(), Some("equipe"));
        assert_eq!(plist_value(&info, "CFBundleIconFile").as_deref(), Some("AppIcon"));
        assert_eq!(plist_value(&info, REVISION_KEY).as_deref(), Some(REVISION));
        assert_eq!(plist_value(&info, "LSUIElement").as_deref(), Some("true"));
        assert!(ICON.starts_with(b"icns"), "equipe's own icon");
        assert_eq!(REVISION.len(), 16);
    }

    #[test]
    fn only_an_app_that_says_it_serves_hosts_the_helper() {
        let dir = std::env::temp_dir().join(format!("eq-host-{}", std::process::id()));
        let app = dir.join("equipe.app");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/MacOS/equipe-desktop"), "").unwrap();
        let plist = |serves: bool| {
            format!(
                "<plist><dict><key>CFBundleExecutable</key><string>equipe-desktop</string>{}</dict></plist>",
                if serves { "<key>EquipeDesktopServe</key><true/>" } else { "" }
            )
        };
        std::fs::write(app.join("Contents/Info.plist"), plist(false)).unwrap();
        assert_eq!(app_executable(&app), None, "an older app doesn't serve");
        std::fs::write(app.join("Contents/Info.plist"), plist(true)).unwrap();
        assert_eq!(app_executable(&app), Some(app.join("Contents/MacOS/equipe-desktop")));
        assert_eq!(role_of(&app.join("Contents/MacOS/equipe-desktop")), "app");
        assert_eq!(role_of(Path::new("/h/Applications/equipe Desktop Helper.app/Contents/MacOS/equipe-desktop-helper")), "bundle");
        assert_eq!(role_of(Path::new("/h/.local/bin/equipe")), "cli");
        assert!(!settled_app(Path::new("/Volumes/equipe/equipe.app")) && settled_app(&app));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_plist_is_written_only_when_it_changes() {
        let dir = std::env::temp_dir().join(format!("eq-plist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("a.plist");
        assert!(write_if_changed(&path, "x").unwrap());
        assert!(!write_if_changed(&path, "x").unwrap(), "the same plist isn't written again");
        assert!(write_if_changed(&path, "y").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_helper_lives_where_system_settings_lists_it() {
        let app = bundle_path();
        assert!(app.ends_with("Applications/equipe Desktop Helper.app"), "{}", app.display());
    }
}
