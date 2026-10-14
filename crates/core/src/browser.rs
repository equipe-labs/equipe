//! A browser for the squad: Playwright's MCP server, headless, signed in
//! wherever the person signed in through `equipe browser`. Members use it the
//! way a person would — open a dashboard, create a project, connect a domain,
//! find a key — on the person's own accounts.
//!
//! Each member gets a browser of its own, with a profile kept on disk, that
//! starts from the squad's sign-ins (Playwright storage state). Whatever it
//! signs in to — an account it made with the person's yes — goes back into
//! them, so the next member is signed in too.

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

/// Closing a member's browser closes everything it started: `npx`, the
/// server under it, and Chrome under that — they share a process group.
impl Drop for Running {
    fn drop(&mut self) {
        if let Some(pid) = self.child.id() {
            let _ = std::process::Command::new("kill").args(["-TERM", &format!("-{pid}")]).status();
        }
    }
}

impl Running {
    /// Still running, and showing as asked.
    pub fn is_shown(&self) -> bool {
        self.shown
    }

    pub fn fits(&mut self, shown: bool) -> bool {
        self.shown == shown && matches!(self.child.try_wait(), Ok(None))
    }
}

/// Start a member's browser server on a free local port.
pub fn launch(npx: &Path, home: &Path, profile: &Path, output: &Path, shown: bool) -> std::io::Result<Running> {
    let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    std::fs::create_dir_all(profile)?;
    let mut args: Vec<String> = server(npx, home, profile, output, shown)["args"]
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
        .process_group(0)
        .kill_on_drop(true)
        .spawn()?;
    Ok(Running { child, port, shown, last_used: std::time::Instant::now() })
}

/// How a turn reaches a member's running browser.
pub fn entry(port: u16) -> Value {
    json!({ "type": "http", "url": format!("http://localhost:{port}/mcp") })
}

/// Hold a connection of equipe's own open on a member's browser server. The
/// server closes its browser when its last client leaves, which every turn's
/// does when it ends; held, the browser — its window, its tabs, a sign-in the
/// person is in the middle of — stays between turns.
pub async fn hold(client: &reqwest::Client, port: u16) -> bool {
    if !ready(port, std::time::Duration::from_secs(90)).await {
        return false;
    }
    let url = format!("http://localhost:{port}/mcp");
    let post = |body: &'static str, session: Option<&str>| {
        let mut request = client
            .post(&url)
            .timeout(std::time::Duration::from_secs(60))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(body);
        if let Some(session) = session {
            request = request.header("mcp-session-id", session);
        }
        request.send()
    };
    let initialize = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"equipe","version":"1"}}}"#;
    let Ok(response) = post(initialize, None).await else {
        return false;
    };
    let Some(session) = response.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()).map(str::to_owned) else {
        return false;
    };
    let _ = response.bytes().await;
    let _ = post(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, Some(&session)).await;
    // A first call makes this connection one of the browser's clients.
    let list = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_tabs","arguments":{"action":"list"}}}"#;
    match post(list, Some(&session)).await {
        Ok(response) => response.bytes().await.is_ok(),
        Err(_) => false,
    }
}

/// Call one of a member's browser tools from a connection of equipe's own:
/// the tool's result content, or `None`. The browser is the member's —
/// its tabs and the page it is on.
pub async fn call(client: &reqwest::Client, port: u16, tool: &str, arguments: Value) -> Option<Vec<Value>> {
    let url = format!("http://localhost:{port}/mcp");
    let post = |body: String, session: Option<&str>| {
        let mut request = client
            .post(&url)
            .timeout(std::time::Duration::from_secs(20))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(body);
        if let Some(session) = session {
            request = request.header("mcp-session-id", session);
        }
        request.send()
    };
    let initialize = json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "equipe", "version": "1" } } });
    let response = post(initialize.to_string(), None).await.ok()?;
    let session = response.headers().get("mcp-session-id")?.to_str().ok()?.to_owned();
    let _ = response.bytes().await;
    let _ = post(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(), Some(&session)).await;
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": tool, "arguments": arguments } });
    let raw = post(body.to_string(), Some(&session)).await.ok()?.text().await.ok()?;
    // Streamable HTTP: one JSON message, or server-sent events carrying it.
    let message: Value = raw
        .lines()
        .find_map(|line| line.strip_prefix("data:").and_then(|d| serde_json::from_str(d.trim()).ok()))
        .or_else(|| serde_json::from_str(&raw).ok())?;
    message["result"]["content"].as_array().cloned()
}

/// What a member's browser shows now: a PNG of the page, and its address.
pub async fn screenshot(client: &reqwest::Client, port: u16) -> Option<(Vec<u8>, Option<String>)> {
    use base64::Engine as _;
    let content = call(client, port, "browser_take_screenshot", json!({})).await?;
    let image = content.iter().find(|c| c["type"] == "image")?["data"].as_str()?;
    let png = base64::engine::general_purpose::STANDARD.decode(image).ok()?;
    let tabs = call(client, port, "browser_tabs", json!({ "action": "list" })).await.unwrap_or_default();
    let url = tabs.iter().filter_map(|c| c["text"].as_str()).flat_map(str::lines).find(|l| l.contains("(current)")).and_then(current_url);
    Some((png, url))
}

/// The address in a tab line: `- 0: (current) [Title](https://…)`.
fn current_url(line: &str) -> Option<String> {
    let start = line.rfind("](")? + 2;
    Some(line[start..].trim_end_matches(')').to_owned())
}

/// Take a member's browser to `url`.
pub async fn navigate(client: &reqwest::Client, port: u16, url: &str) -> bool {
    call(client, port, "browser_navigate", json!({ "url": url })).await.is_some()
}

/// Wait until the browser server on `port` takes connections — the first
/// launch may download it — up to `wait`.
pub async fn ready(port: u16, wait: std::time::Duration) -> bool {
    let until = std::time::Instant::now() + wait;
    while std::time::Instant::now() < until {
        // The server listens on "localhost", which Node binds to ::1 first:
        // try every address the name has, not only 127.0.0.1.
        if tokio::net::TcpStream::connect(("localhost", port)).await.is_ok() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    false
}

/// Where a member's browser keeps its profile, between turns and restarts.
pub fn profile(home: &Path, squad: &str, member: &str) -> PathBuf {
    home.join("browser").join("profiles").join(squad).join(member)
}

/// One member's browser server: its own profile on disk, starting from the
/// squad's sign-ins and handing new ones back; on screen when `shown`, hidden
/// otherwise. What it saves (snapshots, screenshots) goes to `output`.
pub fn server(npx: &Path, home: &Path, profile: &Path, output: &Path, shown: bool) -> Value {
    // The sign-ins file must exist before the first page reads it.
    ensure_state(home);
    let mut args = vec![json!("-y"), json!(MCP_PACKAGE)];
    if !shown {
        args.push(json!("--headless"));
    }
    args.extend([
        json!("--user-data-dir"),
        json!(profile),
        json!("--output-dir"),
        json!(output),
        json!("--init-page"),
        json!(keeper(home)),
        json!("--idle-timeout"),
        json!(IDLE_MS),
        json!("--config"),
        json!(mcp_config(home)),
    ]);
    json!({ "type": "stdio", "command": npx, "args": args })
}

/// How every member's browser is launched: Google Chrome without the
/// automation switch and with automation hidden from pages, which Google
/// otherwise answers by refusing to sign anyone in; `--test-type` keeps Chrome
/// from warning about that flag in a bar across every window.
const MCP_CONFIG: &str = r#"{"browser":{"browserName":"chromium","launchOptions":{"channel":"chrome","ignoreDefaultArgs":["--enable-automation"],"args":["--disable-blink-features=AutomationControlled","--test-type"]}}}"#;

fn mcp_config(home: &Path) -> PathBuf {
    let path = home.join("browser").join("mcp-config.json");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(MCP_CONFIG) {
        let _ = std::fs::write(&path, MCP_CONFIG);
    }
    path
}

/// The person's sign-in window (`equipe browser`), as a script for Node.js run
/// with Playwright's package: written next to the sign-ins when it changed.
const SIGN_IN: &str = include_str!("../assets/sign-in.cjs");

pub fn sign_in_script(home: &Path) -> PathBuf {
    let path = home.join("browser").join("sign-in.cjs");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(SIGN_IN) {
        let _ = std::fs::write(&path, SIGN_IN);
    }
    path
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
    fn the_current_tabs_address_is_read_from_its_line() {
        assert_eq!(current_url("- 0: (current) [Example Domain](https://example.com/)").as_deref(), Some("https://example.com/"));
        assert_eq!(current_url("- 1: [](about:blank)").as_deref(), Some("about:blank"));
        assert_eq!(current_url("no link"), None);
    }

    #[tokio::test]
    async fn a_server_listening_on_ipv6_localhost_is_seen_ready() {
        // As Node binds "localhost": ::1 only.
        let Ok(listener) = tokio::net::TcpListener::bind("[::1]:0").await else {
            return; // No IPv6 loopback here.
        };
        let port = listener.local_addr().unwrap().port();
        let started = std::time::Instant::now();
        assert!(ready(port, std::time::Duration::from_secs(5)).await);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

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
        let home = std::env::temp_dir().join(format!("equipe-server-{}", std::process::id()));
        let server = server(Path::new("/bin/npx"), &home, Path::new("/p/pm"), Path::new("/out"), false);
        let args: Vec<&str> = server["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
        assert!(args.windows(2).any(|w| w == ["--user-data-dir", "/p/pm"]), "a profile of its own, kept between turns");
        assert!(!args.contains(&"--isolated"));
        assert!(state_file(&home).is_file(), "the sign-ins it starts from");
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn what_a_member_signs_in_to_goes_back_to_the_squad() {
        let home = std::env::temp_dir().join(format!("equipe-keeper-{}", std::process::id()));
        std::fs::create_dir_all(home.join("browser")).unwrap();
        let server = server(Path::new("/bin/npx"), &home, Path::new("/p/pm"), Path::new("/out"), true);
        let args: Vec<&str> = server["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
        let script = args.windows(2).find(|w| w[0] == "--init-page").unwrap()[1];
        let text = std::fs::read_to_string(script).unwrap();
        assert!(text.contains(&*state_file(&home).to_string_lossy()) && !["__STATE__", "__SLOTS__", "__MAX__"].iter().any(|p| text.contains(p)), "{text}");
        assert!(args.windows(2).any(|w| w == ["--idle-timeout", IDLE_MS]), "an idle browser gives its memory back");
        // Google signs nobody in to a browser that says it is automated.
        let config = args.windows(2).find(|w| w[0] == "--config").unwrap()[1];
        let config: Value = serde_json::from_str(&std::fs::read_to_string(config).unwrap()).unwrap();
        assert_eq!(config["browser"]["launchOptions"]["ignoreDefaultArgs"], json!(["--enable-automation"]));
        assert_eq!(config["browser"]["launchOptions"]["channel"], "chrome");
        assert_eq!(config["browser"]["launchOptions"]["args"], json!(["--disable-blink-features=AutomationControlled", "--test-type"]), "no warning bar about it");
        // On the person's screen it shows who drives it, out of reach of clicks and snapshots.
        for part in ["!argv.includes('--headless')", "addInitScript(driving", "attachShadow({ mode: 'closed' })", "aria-hidden", "pointer-events: none"] {
            assert!(text.contains(part), "{part}");
        }
        assert!(!text.contains("innerHTML"), "Trusted Types pages refuse HTML strings");
        std::fs::remove_dir_all(&home).unwrap();
    }
}
