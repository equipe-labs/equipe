//! A browser for the squad: Playwright's MCP server, headless, signed in
//! wherever the person signed in through `equipe browser`. Members use it the
//! way a person would — open a dashboard, create a project, connect a domain,
//! find a key — on the person's own accounts.
//!
//! Each member gets its own in-memory browser, started from the squad's
//! sign-ins (Playwright storage state). Whatever it signs in to — an account it
//! made with the person's yes — goes back into them, so the next turn, and the
//! next member, are signed in too.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The MCP server's name: its tools are `mcp__browser__…`.
pub const SERVER: &str = "browser";
/// Pinned, so nothing upstream changes what members run without a release.
pub const MCP_PACKAGE: &str = "@playwright/mcp@0.0.83";
/// What `equipe browser` opens the person's sign-in window with.
pub const PLAYWRIGHT_PACKAGE: &str = "playwright@1.64.0";

/// The person's sign-ins, as Playwright storage state (cookies and local
/// storage), shared by every squad: one person, one set of accounts.
pub fn state_file(home: &Path) -> PathBuf {
    home.join("browser").join("state.json")
}

/// Whether the person has signed in anywhere yet.
pub fn signed_in(home: &Path) -> bool {
    state_file(home).is_file()
}

/// The sign-ins file, made empty if there is none yet: a member's browser
/// starts from it whether or not the person has signed in anywhere, and keeps
/// there what it signs in to. Only the person can read it.
fn ensure_state(home: &Path) -> PathBuf {
    let dir = home.join("browser");
    let state = state_file(home);
    if !state.is_file() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
        let _ = std::fs::write(&state, r#"{"cookies":[],"origins":[]}"#);
        let _ = std::fs::set_permissions(&state, std::os::unix::fs::PermissionsExt::from_mode(0o600));
    }
    state
}

/// The script that hands a member's sign-ins back, with the state file's path
/// written in. Written next to the state when it changed.
const KEEP_SIGN_INS: &str = include_str!("../assets/keep-sign-ins.ts");

fn keeper(home: &Path) -> PathBuf {
    let path = home.join("browser").join("keep-sign-ins.ts");
    let quoted = |p: PathBuf| serde_json::to_string(&p.to_string_lossy()).unwrap_or_default();
    let script = KEEP_SIGN_INS
        .replace("__STATE__", &quoted(state_file(home)))
        .replace("__SLOTS__", &quoted(home.join("browser").join("open")))
        .replace("__MAX__", &at_once(home).to_string());
    if std::fs::read_to_string(&path).ok().as_deref() != Some(script.as_str()) {
        let _ = std::fs::write(&path, script);
    }
    path
}

/// How many members' browsers may be open at once: what the person set with
/// `/browser max`, or one for every 4 GB of memory, 2 to 8.
pub fn at_once(home: &Path) -> usize {
    std::fs::read_to_string(max_file(home))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(|| (memory_gb() / 4).clamp(2, 8))
}

/// The most browsers the person allows open at once, kept for every squad.
pub fn set_at_once(home: &Path, max: usize) -> std::io::Result<()> {
    std::fs::create_dir_all(home.join("browser"))?;
    std::fs::write(max_file(home), max.to_string())
}

fn max_file(home: &Path) -> PathBuf {
    home.join("browser").join("max")
}

fn memory_gb() -> usize {
    let bytes = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .or_else(|| {
            let info = std::fs::read_to_string("/proc/meminfo").ok()?;
            let kb = info.lines().find(|l| l.starts_with("MemTotal:"))?.split_whitespace().nth(1)?.parse::<u64>().ok()?;
            Some(kb * 1024)
        });
    bytes.map_or(16, |b| (b / (1 << 30)) as usize)
}

/// A browser left unused this long closes, giving its memory back; the next
/// step opens a fresh one, still signed in. Long enough for the person to sign
/// in at a window a member opened for them.
const IDLE_MS: &str = "900000";

/// One member's browser, kept between its turns: Playwright's MCP server on a
/// local port, which every turn connects to — so a page, a half-done sign-in,
/// a tab the person is looking at, all outlive the turn that opened them.
pub struct Running {
    child: tokio::process::Child,
    pub port: u16,
    shown: bool,
    pub last_used: std::time::Instant,
}

impl Running {
    /// Still running, and showing as asked.
    pub fn fits(&mut self, shown: bool) -> bool {
        self.shown == shown && matches!(self.child.try_wait(), Ok(None))
    }
}

/// Start a member's browser server on a free local port.
pub fn launch(npx: &Path, home: &Path, output: &Path, shown: bool) -> std::io::Result<Running> {
    let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let mut args: Vec<String> = server(npx, home, output, shown)["args"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a.as_str().map(str::to_owned))
        .collect();
    // Every turn's connection shares the one browser — its tabs and sign-ins.
    args.extend(["--port".into(), port.to_string(), "--host".into(), "localhost".into(), "--shared-browser-context".into()]);
    let child = tokio::process::Command::new(npx)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    Ok(Running { child, port, shown, last_used: std::time::Instant::now() })
}

/// How a turn reaches a member's running browser.
pub fn entry(port: u16) -> Value {
    json!({ "type": "http", "url": format!("http://localhost:{port}/mcp") })
}

/// Wait until the browser server on `port` takes connections — the first
/// launch may download it — up to `wait`.
pub async fn ready(port: u16, wait: std::time::Duration) -> bool {
    let until = std::time::Instant::now() + wait;
    while std::time::Instant::now() < until {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    false
}

/// One member's browser server: in memory, starting from the squad's
/// sign-ins and handing new ones back; on screen when `shown`, hidden
/// otherwise. What it saves (snapshots, screenshots) goes to `output`.
pub fn server(npx: &Path, home: &Path, output: &Path, shown: bool) -> Value {
    let mut args = vec![json!("-y"), json!(MCP_PACKAGE)];
    if !shown {
        args.push(json!("--headless"));
    }
    args.extend([
        json!("--isolated"),
        json!("--storage-state"),
        json!(ensure_state(home)),
        json!("--output-dir"),
        json!(output),
        json!("--init-page"),
        json!(keeper(home)),
        json!("--idle-timeout"),
        json!(IDLE_MS),
    ]);
    json!({ "type": "stdio", "command": npx, "args": args })
}

/// `npx` on PATH: the browser needs Node.js.
pub fn find_npx() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join("npx")).find(|p| p.is_file())
}

/// The sites in a storage state the person is signed in to, by domain.
pub fn sites(state: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(state) else {
        return Vec::new();
    };
    let mut sites: Vec<String> = value["cookies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["domain"].as_str())
        .map(|d| {
            let d = d.trim_start_matches('.');
            // The registrable part is enough to recognise: vercel.com, not api.vercel.com.
            let parts: Vec<&str> = d.split('.').collect();
            parts[parts.len().saturating_sub(2)..].join(".")
        })
        .collect();
    sites.sort();
    sites.dedup();
    sites
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_person_sets_how_many_browsers_open_at_once() {
        let home = std::env::temp_dir().join(format!("equipe-max-{}", std::process::id()));
        assert!((2..=8).contains(&at_once(&home)), "by memory until set");
        set_at_once(&home, 10).unwrap();
        assert_eq!(at_once(&home), 10);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn sites_are_named_by_their_domain() {
        let state = r#"{"cookies":[{"domain":".vercel.com"},{"domain":"api.vercel.com"},{"domain":"us.posthog.com"}],"origins":[]}"#;
        assert_eq!(sites(state), ["posthog.com", "vercel.com"]);
        assert!(sites("not json").is_empty());
    }

    #[test]
    fn a_member_starts_from_the_persons_sign_ins_in_a_browser_of_its_own() {
        let server = server(Path::new("/bin/npx"), Path::new("/h"), Path::new("/out"), false);
        let args: Vec<&str> = server["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
        assert!(args.windows(2).any(|w| w == ["--storage-state", "/h/browser/state.json"]));
        assert!(args.contains(&"--isolated"));
    }

    #[test]
    fn what_a_member_signs_in_to_goes_back_to_the_squad() {
        let home = std::env::temp_dir().join(format!("equipe-keeper-{}", std::process::id()));
        std::fs::create_dir_all(home.join("browser")).unwrap();
        let server = server(Path::new("/bin/npx"), &home, Path::new("/out"), true);
        let args: Vec<&str> = server["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
        let script = args.windows(2).find(|w| w[0] == "--init-page").unwrap()[1];
        let text = std::fs::read_to_string(script).unwrap();
        assert!(text.contains(&*state_file(&home).to_string_lossy()) && !["__STATE__", "__SLOTS__", "__MAX__"].iter().any(|p| text.contains(p)), "{text}");
        assert!(args.windows(2).any(|w| w == ["--idle-timeout", IDLE_MS]), "an idle browser gives its memory back");
        std::fs::remove_dir_all(&home).unwrap();
    }
}
