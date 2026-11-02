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

/// What a member's browser on the person's screen shows of who drives it:
/// written into both browser scripts.
const DRIVING: &str = include_str!("../assets/driving.js");

/// Whether a member's tool call may go ahead while its browser is in the
/// person's hands: written into the browser host.
const GATE: &str = include_str!("../assets/gate.js");

/// The script that hands a member's sign-ins back, with the state file's path
/// written in. Written next to the state when it changed.
const KEEP_SIGN_INS: &str = include_str!("../assets/keep-sign-ins.ts");

fn keeper(home: &Path) -> PathBuf {
    let path = home.join("browser").join("keep-sign-ins.ts");
    let quoted = |p: PathBuf| serde_json::to_string(&p.to_string_lossy()).unwrap_or_default();
    let script = KEEP_SIGN_INS
        .replace("__STATE__", &quoted(state_file(home)))
        .replace("__SLOTS__", &quoted(home.join("browser").join("open")))
        .replace("__MAX__", &at_once(home).to_string())
        .replace("__DRIVING__", DRIVING);
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
    kind: Kind,
    pub port: u16,
    shown: bool,
    pub last_used: std::time::Instant,
}

enum Kind {
    /// A Chrome of its own, with a profile of its own.
    Own(tokio::process::Child),
    /// Its lane in the browser host's Chrome.
    Lane { id: String, host: SharedHost },
}

/// Closing a member's browser closes everything it started: `npx`, the
/// server under it, and Chrome under that — they share a process group. A
/// lane's windows close; the host's Chrome stays for everyone else.
impl Drop for Running {
    fn drop(&mut self) {
        match &self.kind {
            Kind::Own(child) => {
                if let Some(pid) = child.id() {
                    crate::group::end(pid, libc::SIGTERM);
                }
            }
            Kind::Lane { id, host } => {
                if let Ok(mut host) = host.lock() {
                    host.send(&json!({ "op": "close", "id": id }));
                }
            }
        }
    }
}

impl Running {
    /// Still running, and showing as asked.
    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// A lane of the browser host, not a Chrome of its own.
    pub fn is_lane(&self) -> bool {
        matches!(self.kind, Kind::Lane { .. })
    }

    /// Whether it can be used showing as asked. A lane goes on screen or out of
    /// sight as it stands; a browser of its own must start again to change.
    pub fn fits(&mut self, shown: bool) -> bool {
        match &mut self.kind {
            Kind::Own(child) => self.shown == shown && matches!(child.try_wait(), Ok(None)),
            Kind::Lane { id, host } => {
                let Ok(mut host) = host.lock() else {
                    return false;
                };
                if !host.alive() {
                    return false;
                }
                if self.shown != shown {
                    host.send(&json!({ "op": "show", "id": id, "shown": shown }));
                    self.shown = shown;
                }
                true
            }
        }
    }
}

/// The browser host, shared by every lane in it.
pub type SharedHost = std::sync::Arc<std::sync::Mutex<Host>>;

/// equipe's one Chrome for every member: one profile the person signs in to
/// once, Google included, and a lane per member — an MCP endpoint that sees
/// only that member's tabs. See `assets/browser-host.cjs`.
pub struct Host {
    child: std::process::Child,
    /// Its requests, one line each, written by a thread of their own: a host
    /// that stops reading must never hold up whoever asks — the daemon's loop
    /// asks with the host's lock held, and a full pipe blocks a write for good.
    requests: std::sync::mpsc::SyncSender<String>,
    /// Requests dropped since the host last took one, for the log.
    dropped: u64,
    /// Each lane's port, kept for the host's life: a lane closed and opened
    /// again is the same endpoint.
    lanes: std::collections::HashMap<String, u16>,
}

const HOST: &str = include_str!("../assets/browser-host.cjs");

/// The profile the browser host's Chrome keeps: the person's sign-ins, once.
pub fn host_profile(home: &Path) -> PathBuf {
    home.join("browser").join("chrome")
}

/// What the browser host says, besides pictures (`assets/browser-host.cjs`).
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostEvent {
    Ready,
    /// A lane's tabs (`lane` `""`: the person's own).
    Tabs {
        lane: String,
        #[serde(default)]
        member: Option<String>,
        #[serde(default)]
        color: Option<String>,
        #[serde(default)]
        shown: bool,
        pages: Vec<HostPage>,
        /// The tab its member's tools last worked in.
        #[serde(default)]
        agent: Option<u64>,
    },
    /// Who has a lane in hand: `"person"`, or the member again. `refused`:
    /// its member's tools were turned away while the person had it.
    Control {
        lane: String,
        by: Option<String>,
        #[serde(default)]
        refused: bool,
    },
    Dialog {
        page: u64,
        lane: String,
        kind: String,
        message: String,
        #[serde(default)]
        default: Option<String>,
    },
    DialogClosed { page: u64 },
    /// A member's tool call was refused on a tab the person is on, for
    /// something the person can allow at a click (`close`).
    Wants { lane: String, page: u64, action: String },
    /// The host is gone (its Chrome closed, or it failed).
    #[serde(skip)]
    Exited,
}

/// A tab as the host tells it: what a screen is told ([`HostPage::page`]),
/// and what the host read off the page to tell whether it waits for a person.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct HostPage {
    pub id: u64,
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub loading: bool,
    #[serde(default)]
    pub favicon: Option<String>,
    #[serde(default)]
    pub signals: Option<Signals>,
    #[serde(default)]
    pub crashed: bool,
}

/// What the host reads off a member's page (`PROBE` in `browser-host.cjs`).
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct Signals {
    /// The address of each frame in it: Turnstile, reCAPTCHA and hCaptcha
    /// live in frames of their own.
    pub frames: Vec<String>,
    /// A CAPTCHA box in sight — bigger than the badge an invisible one shows.
    pub widget: bool,
    /// A CAPTCHA on the page already answered (its response field filled).
    pub solved: bool,
    /// A password field in sight.
    pub password: bool,
    /// A one-time-code field in sight (`autocomplete="one-time-code"`, an `otp` field).
    pub otp: bool,
    /// A short code-sized field in sight (4–8 characters, or numeric).
    pub code: bool,
    /// The start of what the page says, lowercased.
    pub text: String,
}

impl HostPage {
    /// The tab as screens are told it: whether it waits for a person decided here.
    pub fn page(self) -> equipe_protocol::BrowserPage {
        let check = human_check(&self.url, &self.title, self.signals.as_ref());
        let favicon = self.favicon.filter(|f| f.starts_with("https://") || f.starts_with("http://"));
        equipe_protocol::BrowserPage { id: self.id, url: self.url, title: self.title, loading: self.loading, favicon, check, crashed: self.crashed }
    }
}

/// Titles of a whole-page "checking your browser" wall — Cloudflare's, in the
/// languages it speaks, and the like: the locale files' words, and the walls'
/// own names.
fn wall_titles() -> impl Iterator<Item = &'static str> {
    crate::i18n::every("match.wall_titles").iter().copied().chain(["attention required! | cloudflare", "ddos-guard"])
}

/// Words of a page asking whether a person is there.
fn human_words() -> &'static [&'static str] {
    crate::i18n::every("match.human_words")
}

/// Where a CAPTCHA's frame comes from (not an invisible one's badge).
const CAPTCHA_FRAMES: &[&str] = &["challenges.cloudflare.com", "hcaptcha.com", "/recaptcha/api2/", "/recaptcha/enterprise/", "recaptcha.net/recaptcha"];

/// Words of a code from the person's phone.
fn code_words() -> impl Iterator<Item = &'static str> {
    crate::i18n::every("match.code_words").iter().copied().chain(["2fa"])
}

/// Words of a sign-in page, in its address or title: the locale files'
/// words, and addresses' own marks.
fn sign_in_words() -> impl Iterator<Item = &'static str> {
    crate::i18n::every("match.sign_in_words").iter().copied().chain(["/auth", "/sso", "sso.", "accounts."])
}

/// Whether a page waits for a person, and for what: from its address, its
/// title and what the host read off it. Only what a person must do counts —
/// an invisible CAPTCHA's badge, or one already answered, is none.
pub fn human_check(url: &str, title: &str, signals: Option<&Signals>) -> Option<equipe_protocol::HumanCheck> {
    use equipe_protocol::HumanCheck;
    let none = Signals::default();
    let s = signals.unwrap_or(&none);
    let url_l = url.to_lowercase();
    let title_l = title.trim().trim_end_matches(['.', '…', ' ']).to_lowercase();
    let says = |w: &str| s.text.contains(w) || title_l.contains(w);
    // A wall over the whole page: its title, or Cloudflare's challenge in the address.
    let wall_title = title_l.chars().count() <= 40 && wall_titles().any(|w| title_l.starts_with(w));
    if wall_title || url_l.contains("__cf_chl") || url_l.contains("cf_chl_") || url_l.contains("/cdn-cgi/challenge-platform") {
        return Some(HumanCheck::Challenge);
    }
    // A CAPTCHA in sight, not yet answered.
    let captcha_frame = s.frames.iter().any(|f| {
        let f = f.to_lowercase();
        CAPTCHA_FRAMES.iter().any(|c| f.contains(c)) && !f.contains("size=invisible")
    });
    if !s.solved && (s.widget || (captcha_frame && human_words().iter().any(|w| says(w)))) {
        return Some(HumanCheck::Captcha);
    }
    if s.otp || (s.code && !s.password && code_words().any(says)) {
        return Some(HumanCheck::TwoFactor);
    }
    if s.password && sign_in_words().any(|w| url_l.contains(w) || title_l.contains(w)) {
        return Some(HumanCheck::SignIn);
    }
    None
}

/// What equipe does with a member's browser when its tabs change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckStep {
    /// A page waits for a person: into their hands it goes, its member waits.
    Pause { page: u64, url: String, check: equipe_protocol::HumanCheck },
    /// Nothing waits any more: back to its member — unless the person put a
    /// hand on it, then it stays theirs until they give it back.
    Release,
    Nothing,
}

/// The next step for a member's browser: `pausing`, equipe put it in the
/// person's hands for a check already; `held`, it is in their hands for
/// another reason (asked in, or taken); `let_go(page, url)`, the person handed
/// that page back still waiting — leave it to the member.
pub fn check_step(pages: &[equipe_protocol::BrowserPage], agent: Option<u64>, pausing: bool, held: bool, let_go: impl Fn(u64, &str) -> bool) -> CheckStep {
    // The tab its member works in first.
    let waiting = pages.iter().filter(|p| p.check.is_some() && !let_go(p.id, &p.url)).min_by_key(|p| Some(p.id) != agent);
    match (waiting, pausing) {
        (Some(page), false) if !held => CheckStep::Pause { page: page.id, url: page.url.clone(), check: page.check.unwrap_or(equipe_protocol::HumanCheck::Challenge) },
        (None, true) => CheckStep::Release,
        _ => CheckStep::Nothing,
    }
}

/// The site an address is on, as a person names it: `dash.cloudflare.com`.
pub fn site_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    host.strip_prefix("www.").unwrap_or(host)
}

/// The first page id a browser host started now gives: ids go on from the
/// clock, so a host started again (or a daemon) never names a tab with an
/// id a screen still holds for another — a screen showing one tab's address
/// over another's picture. Room for 2²⁰ tabs a second, and below 2⁵³, where
/// JavaScript's numbers stop being whole.
pub fn first_page_id(now: std::time::SystemTime) -> u64 {
    let secs = now.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    ((secs % (1 << 32)) << 20) + 1
}

impl Host {
    /// Start the host: its Chrome comes up with the shared profile, taking up
    /// the sign-ins members' own browsers kept. What it says comes on
    /// `events`; its pictures go to `hub`.
    pub fn start(npx: &Path, home: &Path, hub: &std::sync::Arc<Hub>, events: tokio::sync::mpsc::UnboundedSender<HostEvent>) -> std::io::Result<SharedHost> {
        use std::os::unix::process::CommandExt as _;
        let state = ensure_state(home);
        let script = home.join("browser").join("browser-host.cjs");
        let text = HOST.replace("__DRIVING__", DRIVING).replace("__GATE__", GATE);
        if std::fs::read_to_string(&script).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&script, text)?;
        }
        let profile = host_profile(home);
        std::fs::create_dir_all(&profile)?;
        let log = std::fs::OpenOptions::new().create(true).append(true).open(home.join("browser").join("host.log"))?;
        let mut child = std::process::Command::new(npx)
            .args(["-y", "-p", PLAYWRIGHT_PACKAGE, "node"])
            .arg(&script)
            .arg(&profile)
            .arg(&state)
            .env("EQUIPE_FIRST_PAGE", first_page_id(std::time::SystemTime::now()).to_string())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(log)
            .process_group(0)
            .spawn()?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let hub = hub.clone();
        std::thread::Builder::new().name("equipe-browser-host".into()).spawn(move || {
            read_host(stdout, &hub, &events);
            let _ = events.send(HostEvent::Exited);
        })?;
        let (requests, lines) = std::sync::mpsc::sync_channel::<String>(HOST_QUEUE);
        std::thread::Builder::new().name("equipe-browser-host-in".into()).spawn(move || write_host(stdin, lines))?;
        Ok(std::sync::Arc::new(std::sync::Mutex::new(Host { child, requests, dropped: 0, lanes: Default::default() })))
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Never waits: a request the host has no room for is dropped (it isn't
    /// reading — it is stuck, or going), and the log says so once.
    fn send(&mut self, request: &Value) {
        match self.requests.try_send(request.to_string()) {
            Ok(()) => {
                if self.dropped > 0 {
                    crate::daemon_log!("equipe: the browser host reads again ({} requests were dropped)", self.dropped);
                    self.dropped = 0;
                }
            }
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                if self.dropped == 0 {
                    crate::daemon_log!("equipe: the browser host stopped reading its requests; dropping them until it does");
                }
                self.dropped += 1;
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {}
        }
    }

    /// Open a tab in the host's Chrome for the person, at `url`, on screen.
    pub fn person(&mut self, url: &str) {
        self.send(&json!({ "op": "person", "url": url }));
    }

    /// Any other request (`assets/browser-host.cjs` lists them).
    pub fn op(&mut self, request: &Value) {
        self.send(request);
    }
}

/// Requests waiting for the browser host to read them, at most.
const HOST_QUEUE: usize = 4096;

/// The host's standard input: each request as a line, until the host or
/// the [`Host`] goes.
fn write_host(mut to: impl std::io::Write, lines: std::sync::mpsc::Receiver<String>) {
    for line in lines {
        if writeln!(to, "{line}").and_then(|()| to.flush()).is_err() {
            return;
        }
    }
}

/// The host's standard output, until it ends: messages and frames.
fn read_host(mut out: impl std::io::Read, hub: &Hub, events: &tokio::sync::mpsc::UnboundedSender<HostEvent>) {
    use equipe_protocol::stream;
    let mut decoder = stream::Decoder::default();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = match out.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        decoder.push(&buf[..n]);
        loop {
            match decoder.message() {
                Ok(Some((stream::JSON, payload))) => {
                    if let Ok(event) = serde_json::from_slice::<HostEvent>(&payload)
                        && events.send(event).is_err()
                    {
                        return;
                    }
                }
                Ok(Some((stream::FRAME, payload))) => {
                    if let Some((header, jpeg)) = stream::parse_frame(&payload) {
                        hub.put(header, jpeg.to_vec());
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                // Out of step with the host: nothing after this can be read.
                Err(_) => return,
            }
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        crate::group::end(self.child.id(), libc::SIGTERM);
    }
}

/// A picture of a watched page, as the host sent it.
#[derive(Debug)]
pub struct Frame {
    pub header: equipe_protocol::stream::FrameHeader,
    pub jpeg: Vec<u8>,
}

/// What a watcher wants of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Want {
    pub page: u64,
    pub fps: u32,
    pub max_w: u32,
    pub max_h: u32,
    pub quality: u8,
}

/// The pictures of the shared Chrome's pages, for whoever watches them: the
/// browser panels' streams and the terminal's watch. The host draws a page
/// only while someone watches it — at the most any watcher asks — and each
/// page keeps only its newest frame.
pub struct Hub {
    inner: std::sync::Mutex<HubInner>,
    /// Bumped with every frame: watchers wake on it and look at their page.
    tick: tokio::sync::watch::Sender<u64>,
}

#[derive(Default)]
struct HubInner {
    host: Option<SharedHost>,
    frames: std::collections::HashMap<u64, std::sync::Arc<Frame>>,
    watchers: std::collections::HashMap<u64, Want>,
    /// What the host was asked to draw, by page.
    asked: std::collections::HashMap<u64, Want>,
    next: u64,
    /// Browser panels connected now.
    panels: usize,
    /// Screens attached now that have a browser panel (the desktop app, on
    /// this Mac or another), whether or not the panel is open yet.
    screens: usize,
    /// The box each panel shows its page in, by watcher: the page is laid
    /// out at the largest — and the host was told this, by page.
    fits: std::collections::HashMap<u64, (u64, Fit)>,
    fitted: std::collections::HashMap<u64, Fit>,
}

/// A browser panel's picture box: CSS pixels and the screen's scale. A page
/// a panel watches is laid out at it — whoever drives — so it reads at the
/// panel's size rather than as the member's wide window shrunk into it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub w: u32,
    pub h: u32,
    pub dpr: f32,
}

/// The size each page should be laid out at: for each page panels watch,
/// the largest of their boxes (several screens at once: the biggest panel).
fn fitted(fits: impl IntoIterator<Item = (u64, Fit)>) -> std::collections::HashMap<u64, Fit> {
    let mut out: std::collections::HashMap<u64, Fit> = std::collections::HashMap::new();
    for (page, fit) in fits {
        let area = |f: &Fit| u64::from(f.w) * u64::from(f.h);
        out.entry(page).and_modify(|a| if (area(&fit), fit.dpr) > (area(a), a.dpr) { *a = fit }).or_insert(fit);
    }
    out
}

/// What the host should be drawing: for each watched page, the most any
/// watcher of it wants.
fn wanted(watchers: impl IntoIterator<Item = Want>) -> std::collections::HashMap<u64, Want> {
    let mut out: std::collections::HashMap<u64, Want> = std::collections::HashMap::new();
    for w in watchers {
        out.entry(w.page)
            .and_modify(|a| {
                a.fps = a.fps.max(w.fps);
                a.max_w = a.max_w.max(w.max_w);
                a.max_h = a.max_h.max(w.max_h);
                a.quality = a.quality.max(w.quality);
            })
            .or_insert(w);
    }
    out
}

impl Hub {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { inner: Default::default(), tick: tokio::sync::watch::Sender::new(0) })
    }

    /// The host frames come from now (`None`: gone). What was watched is asked
    /// of the new one.
    pub fn set_host(&self, host: Option<SharedHost>) {
        let mut inner = self.inner.lock().expect("hub");
        inner.host = host;
        inner.frames.clear();
        inner.asked.clear();
        inner.fitted.clear();
        Self::apply(&mut inner);
    }

    /// A request for the host; `false` when there is none.
    pub fn op(&self, request: &Value) -> bool {
        let host = self.inner.lock().expect("hub").host.clone();
        match host {
            Some(host) => host.lock().map(|mut h| h.op(request)).is_ok(),
            None => false,
        }
    }

    /// A new watcher's id.
    pub fn watcher(&self) -> u64 {
        let mut inner = self.inner.lock().expect("hub");
        inner.next += 1;
        inner.next
    }

    /// `watcher` watches `want.page` from now (instead of anything before).
    pub fn watch(&self, watcher: u64, want: Want) {
        let mut inner = self.inner.lock().expect("hub");
        inner.watchers.insert(watcher, want);
        Self::apply(&mut inner);
    }

    pub fn unwatch(&self, watcher: u64) {
        let mut inner = self.inner.lock().expect("hub");
        let fitted = inner.fits.remove(&watcher).is_some();
        if inner.watchers.remove(&watcher).is_some() || fitted {
            Self::apply(&mut inner);
        }
    }

    /// The box `watcher`'s panel shows `page` in (`None`: it shows none): the
    /// page is laid out at the largest box any panel shows it in, and at its
    /// member's own size again once none does. The panel asks only once its
    /// box holds still; the host waits for a member's step under way.
    pub fn fit(&self, watcher: u64, fit: Option<(u64, Fit)>) {
        let mut inner = self.inner.lock().expect("hub");
        let before = inner.fits.get(&watcher).copied();
        if before == fit {
            return;
        }
        match fit {
            Some(fit) => inner.fits.insert(watcher, fit),
            None => inner.fits.remove(&watcher),
        };
        Self::apply(&mut inner);
    }

    /// Tell the host what changed in what it should draw.
    fn apply(inner: &mut HubInner) {
        let now = wanted(inner.watchers.values().copied());
        let mut requests = Vec::new();
        for (page, want) in &now {
            if inner.asked.get(page) != Some(want) {
                requests.push(json!({ "op": "watch", "page": page, "maxW": want.max_w, "maxH": want.max_h, "quality": want.quality, "fps": want.fps }));
            }
        }
        for page in inner.asked.keys().filter(|p| !now.contains_key(p)) {
            requests.push(json!({ "op": "unwatch", "page": page }));
        }
        inner.frames.retain(|page, _| now.contains_key(page));
        inner.asked = now;
        let sizes = fitted(inner.fits.values().copied());
        for (page, fit) in &sizes {
            if inner.fitted.get(page) != Some(fit) {
                requests.push(json!({ "op": "fit", "page": page, "w": fit.w, "h": fit.h, "dpr": fit.dpr }));
            }
        }
        for page in inner.fitted.keys().filter(|p| !sizes.contains_key(p)) {
            requests.push(json!({ "op": "unfit", "page": page }));
        }
        inner.fitted = sizes;
        if let Some(host) = &inner.host
            && let Ok(mut host) = host.lock()
        {
            for request in &requests {
                host.op(request);
            }
        }
    }

    fn put(&self, header: equipe_protocol::stream::FrameHeader, jpeg: Vec<u8>) {
        let page = header.page;
        {
            let mut inner = self.inner.lock().expect("hub");
            // Only pages still watched keep a picture.
            if !inner.asked.contains_key(&page) {
                return;
            }
            inner.frames.insert(page, std::sync::Arc::new(Frame { header, jpeg }));
        }
        self.tick.send_modify(|n| *n += 1);
    }

    /// The newest frame of `page`.
    pub fn latest(&self, page: u64) -> Option<std::sync::Arc<Frame>> {
        self.inner.lock().expect("hub").frames.get(&page).cloned()
    }

    /// Wakes with every frame.
    pub fn frames(&self) -> tokio::sync::watch::Receiver<u64> {
        self.tick.subscribe()
    }

    /// A browser panel connected (`true`) or left.
    pub fn panel(&self, connected: bool) {
        let mut inner = self.inner.lock().expect("hub");
        inner.panels = if connected { inner.panels + 1 } else { inner.panels.saturating_sub(1) };
    }

    /// A screen with a browser panel attached (`true`) or left — its panel
    /// open or not: it opens the panel when a member hands the person its
    /// browser.
    pub fn screen(&self, attached: bool) {
        let mut inner = self.inner.lock().expect("hub");
        inner.screens = if attached { inner.screens + 1 } else { inner.screens.saturating_sub(1) };
    }

    /// Whether a screen with a browser panel is attached: then a member's
    /// browser the person is handed shows there, in their hands — never as a
    /// window on the daemon's Mac, which may be a machine nobody is looking at
    /// (the app attached over SSH). On the daemon's own Mac too: the panel is
    /// where the person already is, and it says what to do and hands back.
    /// The raw window only comes up with no such screen attached, or when the
    /// person asks for it (the panel's "open in Chrome").
    pub fn has_panel(&self) -> bool {
        let inner = self.inner.lock().expect("hub");
        inner.panels > 0 || inner.screens > 0
    }
}

// ── profiles: other people's accounts, each in a Chrome of its own ──

pub use equipe_protocol::BrowserProfile;

fn profiles_file(home: &Path) -> PathBuf {
    home.join("browser").join("profiles.json")
}

/// The profiles besides the main one, by name.
pub fn profiles(home: &Path) -> Vec<BrowserProfile> {
    let mut all: Vec<BrowserProfile> = std::fs::read_to_string(profiles_file(home)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    all.retain(|p| valid_profile(&p.name));
    all.sort_by(|a, b| a.name.cmp(&b.name));
    all.dedup_by(|a, b| a.name == b.name);
    all
}

/// A profile's name: 1–24 of `a-z 0-9 -`, and not the main one's.
fn valid_profile(name: &str) -> bool {
    (1..=24).contains(&name.len()) && name != "main" && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !name.starts_with('-')
}

fn write_profiles(home: &Path, all: &[BrowserProfile]) -> std::io::Result<()> {
    std::fs::create_dir_all(home.join("browser"))?;
    std::fs::write(profiles_file(home), serde_json::to_string_pretty(all).unwrap_or_default())
}

/// Add profile `name` (lowercased), or name it `label` again.
pub fn save_profile(home: &Path, name: &str, label: &str) -> Result<BrowserProfile, String> {
    let name = name.trim().to_lowercase();
    if !valid_profile(&name) {
        return Err(format!("{name:?}: a profile's name is 1–24 lowercase letters, digits or `-`, and not `main`"));
    }
    let profile = BrowserProfile { name: name.clone(), label: label.trim().to_owned() };
    let mut all = profiles(home);
    all.retain(|p| p.name != name);
    all.push(profile.clone());
    all.sort_by(|a, b| a.name.cmp(&b.name));
    write_profiles(home, &all).map_err(|e| e.to_string())?;
    Ok(profile)
}

/// Forget profile `name`. Its Chrome's folder stays, its sign-ins in it, until
/// the person deletes it: adding the name again finds them.
pub fn delete_profile(home: &Path, name: &str) -> Result<(), String> {
    let mut all = profiles(home);
    all.retain(|p| p.name != name);
    write_profiles(home, &all).map_err(|e| e.to_string())
}

/// Where profile `name`'s Chrome keeps itself.
pub fn profile_dir(home: &Path, name: &str) -> PathBuf {
    home.join("browser").join("chromes").join(name)
}

/// Profile `name`'s sign-ins file: its own, so one person's never reach another's.
pub fn profile_state(home: &Path, name: &str) -> PathBuf {
    profile_dir(home, name).join("state.json")
}

/// A conversation's browser `key` in profile `profile` (`None`: the main one).
pub fn in_profile(key: &str, profile: Option<&str>) -> String {
    match profile {
        Some(p) => format!("{key}#{p}"),
        None => key.to_owned(),
    }
}

/// A browser key's conversation key and profile.
pub fn split_profile(key: &str) -> (&str, Option<&str>) {
    match key.split_once('#') {
        Some((key, profile)) => (key, Some(profile)),
        None => (key, None),
    }
}

/// The MCP server a member reaches a profile's browser by: `browser`, `browser_jhjung`.
pub fn server_name(profile: Option<&str>) -> String {
    match profile {
        Some(p) => format!("{SERVER}_{}", p.replace('-', "_")),
        None => SERVER.to_owned(),
    }
}

/// The browser of a member's conversation about `project` beside its own
/// (`None`: the whole squad's): `pm@site`, `pm@squad`.
pub fn lane_key(member: &str, project: Option<&str>) -> String {
    format!("{member}@{}", project.unwrap_or("squad"))
}

/// Whose browser `key` is: `pm` for `pm@site`.
pub fn member_of(key: &str) -> &str {
    let key = split_profile(key).0;
    key.split_once('@').map_or(key, |(member, _)| member)
}

/// A conversation's lane in the host — its endpoint, on screen when `shown`.
/// `key` is a member's name, or a conversation of it beside its own
/// ([`lane_key`]): each its own windows, in the member's colour, labelled
/// with what it is about.
pub fn lane(host: &SharedHost, home: &Path, squad: &str, key: &str, output: &Path, shown: bool) -> std::io::Result<Running> {
    let id = format!("{squad}/{key}");
    let member = member_of(key);
    let (bare, profile) = split_profile(key);
    let label = match bare.split_once('@') {
        Some((member, "squad")) => format!("{member} (the whole squad)"),
        Some((member, about)) => format!("{member} ({about})"),
        None => member.to_owned(),
    };
    let label = match profile {
        Some(p) => format!("{label} · {p}"),
        None => label,
    };
    let mut locked = host.lock().map_err(|_| std::io::Error::other("the browser host is unusable"))?;
    let port = match locked.lanes.get(&id) {
        Some(port) => *port,
        None => std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port(),
    };
    locked.lanes.insert(id.clone(), port);
    let mut request = json!({
        "op": "lane", "id": id, "member": label, "color": member_color(member),
        "port": port, "output": output, "shown": shown,
    });
    if let Some(p) = profile {
        with_profile(&mut request, home, p);
    }
    locked.send(&request);
    drop(locked);
    Ok(Running { kind: Kind::Lane { id, host: host.clone() }, port, shown, last_used: std::time::Instant::now() })
}

/// A request to the host made for profile `name`'s Chrome: started from its
/// folder, with its own sign-ins file, if it isn't running yet.
pub fn with_profile(request: &mut Value, home: &Path, name: &str) {
    let dir = profile_dir(home, name);
    let _ = std::fs::create_dir_all(&dir);
    request["profile"] = json!(name);
    request["profileDir"] = json!(dir);
    request["state"] = json!(profile_state(home, name));
}

/// A member's colour, as the console shows it: by role (`frontend-2` is a
/// frontend), anyone else's by name. The same as `keep-sign-ins.ts`.
pub fn member_color(name: &str) -> &'static str {
    const PALETTE: [&str; 8] =
        ["232, 190, 120", "236, 150, 190", "120, 190, 240", "140, 208, 150", "182, 162, 242", "110, 212, 200", "240, 160, 110", "205, 212, 120"];
    let role = match name.rsplit_once('-') {
        Some((role, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => role,
        _ => name,
    };
    let hue = match role {
        "pm" => 0,
        "designer" => 1,
        "frontend" => 2,
        "backend" => 3,
        "qa" => 4,
        "data" => 5,
        "mobile" => 6,
        "security" => 7,
        "marketing" => 1,
        _ => 6 + role.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)) as usize % 2,
    };
    PALETTE[hue]
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
    Ok(Running { kind: Kind::Own(child), port, shown, last_used: std::time::Instant::now() })
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
    fn a_host_that_stops_reading_never_holds_up_whoever_asks() {
        // A host that never reads its standard input: a full pipe in no time.
        let mut child = std::process::Command::new("sleep").arg("30").stdin(std::process::Stdio::piped()).spawn().unwrap();
        let stdin = child.stdin.take().unwrap();
        let (requests, lines) = std::sync::mpsc::sync_channel::<String>(HOST_QUEUE);
        std::thread::spawn(move || write_host(stdin, lines));
        let mut host = Host { child, requests, dropped: 0, lanes: Default::default() };
        let started = std::time::Instant::now();
        let big = json!({ "op": "input", "page": 1, "ev": "x".repeat(16 * 1024) });
        for _ in 0..2 * HOST_QUEUE {
            host.op(&big);
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
        assert!(host.dropped > 0, "what it had no room for was dropped");
        let _ = host.child.kill();
    }

    #[test]
    fn a_conversation_beside_a_members_own_has_a_browser_of_its_own_in_its_colour() {
        assert_eq!(lane_key("pm", Some("site")), "pm@site");
        assert_eq!(lane_key("pm", None), "pm@squad");
        assert_eq!(member_of("pm@site"), "pm");
        assert_eq!(member_of("frontend-2"), "frontend-2");
        assert_eq!(member_color(member_of("frontend-2@app")), member_color("frontend"));
    }

    #[test]
    fn another_persons_accounts_live_in_a_browser_profile_of_their_own() {
        let home = std::env::temp_dir().join(format!("equipe-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert!(profiles(&home).is_empty(), "none until the person adds one");
        save_profile(&home, "JHJung", "jhjung@atinuminvest.co.kr").unwrap();
        save_profile(&home, "qa-2", "").unwrap();
        save_profile(&home, "jhjung", "정진혁 · jhjung@atinuminvest.co.kr").unwrap();
        let all = profiles(&home);
        assert_eq!(all.iter().map(|p| (p.name.as_str(), p.label.as_str())).collect::<Vec<_>>(), [("jhjung", "정진혁 · jhjung@atinuminvest.co.kr"), ("qa-2", "")], "named once, the label as last saved");
        for bad in ["", "two words", "../up", "main", "a#b", "x".repeat(25).as_str()] {
            assert!(save_profile(&home, bad, "").is_err(), "{bad:?}");
        }
        assert_eq!(profile_dir(&home, "jhjung"), home.join("browser").join("chromes").join("jhjung"));
        delete_profile(&home, "qa-2").unwrap();
        assert_eq!(profiles(&home).len(), 1);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_members_browser_in_another_profile_is_a_lane_and_a_server_of_its_own() {
        assert_eq!(in_profile("qa", None), "qa");
        assert_eq!(in_profile("qa", Some("jhjung")), "qa#jhjung");
        assert_eq!(in_profile("pm@site", Some("jhjung")), "pm@site#jhjung");
        assert_eq!(split_profile("pm@site#jhjung"), ("pm@site", Some("jhjung")));
        assert_eq!(split_profile("qa"), ("qa", None));
        assert_eq!(member_of("pm@site#jhjung"), "pm");
        assert_eq!(member_of("qa#jhjung"), "qa");
        assert_eq!(server_name(None), "browser");
        assert_eq!(server_name(Some("jhjung")), "browser_jhjung");
        assert_eq!(server_name(Some("qa-2")), "browser_qa_2");
    }

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

    fn header(page: u64, seq: u64) -> equipe_protocol::stream::FrameHeader {
        equipe_protocol::stream::FrameHeader { page, seq, w: 2, h: 1, css_w: 2., css_h: 1., sx: 0., sy: 0., top: 0. }
    }

    #[test]
    fn a_page_is_drawn_at_the_most_any_watcher_wants() {
        let want = |page, fps, max_w, quality| Want { page, fps, max_w, max_h: max_w, quality };
        let all = wanted([want(1, 5, 800, 50), want(1, 30, 600, 70), want(2, 2, 1280, 60)]);
        assert_eq!(all[&1], Want { page: 1, fps: 30, max_w: 800, max_h: 800, quality: 70 });
        assert_eq!(all[&2], want(2, 2, 1280, 60));
        assert!(wanted([]).is_empty());
    }

    #[test]
    fn each_watched_page_keeps_only_its_newest_frame_and_unwatched_pages_none() {
        let hub = Hub::new();
        let woke = hub.frames();
        hub.put(header(1, 1), vec![1]);
        assert!(hub.latest(1).is_none(), "nobody watches page 1 yet");
        let a = hub.watcher();
        let b = hub.watcher();
        hub.watch(a, Want { page: 1, fps: 30, max_w: 800, max_h: 800, quality: 70 });
        hub.watch(b, Want { page: 1, fps: 5, max_w: 800, max_h: 800, quality: 50 });
        hub.put(header(1, 2), vec![2]);
        hub.put(header(1, 3), vec![3]);
        assert!(woke.has_changed().unwrap(), "watchers wake on a frame");
        assert_eq!(hub.latest(1).map(|f| (f.header.seq, f.jpeg.clone())), Some((3, vec![3])));
        hub.unwatch(a);
        assert!(hub.latest(1).is_some(), "still watched by the other");
        hub.unwatch(b);
        assert!(hub.latest(1).is_none(), "nobody watches it: its frame goes");
        assert!(!hub.has_panel());
        hub.panel(true);
        assert!(hub.has_panel());
        hub.panel(false);
        hub.panel(false);
        assert!(!hub.has_panel());
    }

    /// The host's requests, as a hub sends them: a fake host that writes them down.
    fn recording_hub() -> (std::sync::Arc<Hub>, PathBuf) {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let hub = Hub::new();
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let log = std::env::temp_dir().join(format!("equipe-hub-{}-{n}", std::process::id()));
        use std::os::unix::process::CommandExt as _;
        // In a group of its own, as the real host is: dropped, the group is ended.
        let mut child = std::process::Command::new("sh").arg("-c").arg(format!("exec cat > '{}'", log.display())).stdin(std::process::Stdio::piped()).process_group(0).spawn().unwrap();
        let stdin = child.stdin.take().unwrap();
        let (requests, lines) = std::sync::mpsc::sync_channel::<String>(HOST_QUEUE);
        std::thread::spawn(move || write_host(stdin, lines));
        hub.set_host(Some(std::sync::Arc::new(std::sync::Mutex::new(Host { child, requests, dropped: 0, lanes: Default::default() }))));
        (hub, log)
    }

    /// The sizes the host was asked for, once `wait_for` of them came.
    fn sizes_sent(log: &Path, wait_for: usize) -> Vec<Value> {
        let read = || -> Vec<Value> {
            let text = std::fs::read_to_string(log).unwrap_or_default();
            text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|r| r["op"] == "fit" || r["op"] == "unfit").collect()
        };
        for _ in 0..100 {
            if read().len() >= wait_for {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        read()
    }

    #[test]
    fn a_watched_members_page_is_laid_out_at_the_largest_panel_and_its_own_size_once_unwatched() {
        let small = Fit { w: 520, h: 640, dpr: 2. };
        let large = Fit { w: 900, h: 1000, dpr: 1. };
        assert_eq!(fitted([(1, small), (1, large), (2, small)]), std::collections::HashMap::from([(1, large), (2, small)]), "the biggest box wins");
        assert!(fitted([]).is_empty());

        let (hub, log) = recording_hub();
        let (a, b) = (hub.watcher(), hub.watcher());
        hub.fit(a, Some((7, small)));
        hub.fit(a, Some((7, small)));
        hub.fit(b, Some((7, large)));
        hub.fit(b, Some((7, small)));
        hub.unwatch(b);
        hub.fit(a, None);
        let ops = sizes_sent(&log, 4);
        hub.set_host(None);
        let _ = std::fs::remove_file(&log);
        let shown: Vec<String> = ops.iter().map(|o| format!("{}:{}x{}", o["op"].as_str().unwrap(), o["w"], o["h"])).collect();
        assert_eq!(
            shown,
            ["fit:520x640", "fit:900x1000", "fit:520x640", "unfit:nullxnull"],
            "once each change: the first panel; a bigger one; back to the first; nobody watching — the member's own size"
        );
        assert!(ops.iter().all(|o| o["page"] == 7));
    }

    /// The gate a member's browser tools pass while the person has its
    /// browser (`assets/gate.js`), run in Node as the browser host runs it.
    #[test]
    fn a_check_nobody_touched_never_stops_its_member_and_a_persons_hand_keeps_only_its_own_tab() {
        let Some(node) = std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join("node")).find(|p| p.is_file())) else {
            return; // No Node here: the smoke test covers it where there is.
        };
        let since = 1_792_000_000_000u64;
        let check = json!({ "control": "person", "reason": "check", "touched": false, "page": 7, "since": since });
        let hand = json!({ "control": "person", "reason": "hand", "touched": true, "page": 7, "since": since });
        let touched_check = json!({ "control": "person", "reason": "check", "touched": true, "page": 7, "since": since });
        let call = |tool: &str, args: Value, targets: &[u64]| json!({ "tool": tool, "args": args, "targets": targets });
        let close = |targets: &[u64]| call("browser_tabs", json!({ "action": "close", "index": 0 }), targets);
        // (lane, call, what should come of it: "go", "no", or "ask" — refused, the person asked to let it close)
        let cases = [
            (json!({ "control": null }), call("browser_click", json!({}), &[7]), "go"),
            // A hold from detection alone: leaving the page, and other tabs, are the member's.
            (check.clone(), close(&[7]), "go"),
            (check.clone(), call("browser_close", json!({}), &[7, 8]), "go"),
            (check.clone(), call("browser_navigate", json!({ "url": "https://example.com" }), &[7]), "go"),
            (check.clone(), call("browser_navigate_back", json!({}), &[7]), "go"),
            (check.clone(), call("browser_tabs", json!({ "action": "new" }), &[]), "go"),
            (check.clone(), call("browser_click", json!({}), &[8]), "go"),
            (check.clone(), call("browser_click", json!({}), &[7]), "no"),
            // The person's hand: that tab is theirs, closing it is theirs to allow; the rest stays the member's.
            (hand.clone(), call("browser_snapshot", json!({}), &[7]), "no"),
            (hand.clone(), call("browser_navigate", json!({ "url": "https://example.com" }), &[7]), "no"),
            (hand.clone(), close(&[7]), "ask"),
            (hand.clone(), call("browser_close", json!({}), &[7, 8]), "ask"),
            (hand.clone(), close(&[8]), "go"),
            (hand.clone(), call("browser_navigate", json!({ "url": "https://example.com" }), &[8]), "go"),
            (hand.clone(), call("browser_tabs", json!({ "action": "new" }), &[]), "go"),
            (touched_check.clone(), close(&[7]), "ask"),
            (touched_check, call("browser_navigate", json!({ "url": "https://example.com" }), &[7]), "no"),
        ];
        let input: Vec<Value> = cases.iter().map(|(lane, call, _)| json!({ "lane": lane, "call": call })).collect();
        let script = format!("{GATE}\nconst cases = {};\nprocess.stdout.write(JSON.stringify(cases.map((c) => gate(c.lane, c.call))));", Value::Array(input));
        let out = std::process::Command::new(node).arg("-e").arg(script).output().expect("node runs");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let said: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();
        for ((lane, call, want), got) in cases.iter().zip(&said) {
            let came = match (got.is_null(), got["wants"].as_str()) {
                (true, _) => "go",
                (false, Some("close")) => "ask",
                (false, _) => "no",
            };
            assert_eq!(came, *want, "{lane} {call}: {got}");
            if came != "go" {
                assert_eq!(got["page"], 7, "{got}");
            }
        }
        // What the member is told says why, since when, and what it can do.
        let refused = said[8]["text"].as_str().unwrap();
        assert!(regex_lite_like(refused), "{refused}");
        assert!(said[7]["text"].as_str().unwrap().contains("close this tab or go elsewhere"), "{}", said[7]);
        assert!(said[10]["text"].as_str().unwrap().contains("asked whether to let you close it"), "{}", said[10]);
    }

    /// `The person is using this tab since HH:MM — ask them in words, or wait.`
    fn regex_lite_like(text: &str) -> bool {
        let Some(rest) = text.strip_prefix("The person is using this tab since ") else { return false };
        let b = rest.as_bytes();
        b.len() > 5 && b[0].is_ascii_digit() && b[1].is_ascii_digit() && b[2] == b':' && b[3].is_ascii_digit() && b[4].is_ascii_digit() && rest[5..].starts_with(" — ask them in words, or wait.")
    }

    #[test]
    fn a_screen_with_a_panel_attached_counts_before_its_panel_opens() {
        let hub = Hub::new();
        assert!(!hub.has_panel(), "nobody attached: the window on this Mac");
        // The app on another Mac, its panel closed: the hand-over goes to it.
        hub.screen(true);
        assert!(hub.has_panel());
        hub.panel(true);
        hub.screen(false);
        assert!(hub.has_panel(), "its panel's line still up");
        hub.panel(false);
        hub.screen(false);
        assert!(!hub.has_panel());
    }

    #[test]
    fn a_watcher_only_ever_gets_its_own_pages_frames() {
        let hub = Hub::new();
        let (a, b) = (hub.watcher(), hub.watcher());
        let want = |page| Want { page, fps: 30, max_w: 800, max_h: 800, quality: 70 };
        hub.watch(a, want(10));
        hub.watch(b, want(20));
        hub.put(header(20, 1), vec![20]);
        assert!(hub.latest(10).is_none(), "page 20's picture is never page 10's");
        hub.put(header(10, 1), vec![10]);
        assert_eq!(hub.latest(10).unwrap().jpeg, vec![10]);
        assert_eq!(hub.latest(20).unwrap().jpeg, vec![20]);
        // A watcher moving to another page: the old page's picture goes with it.
        hub.watch(a, want(30));
        assert!(hub.latest(10).is_none());
        hub.put(header(10, 2), vec![10]);
        assert!(hub.latest(10).is_none() && hub.latest(30).is_none(), "a late frame of a page nobody watches is dropped");
    }

    #[test]
    fn a_browser_started_again_never_reuses_a_page_id() {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_792_000_000);
        let first = first_page_id(t);
        let later = first_page_id(t + std::time::Duration::from_secs(1));
        assert!(later >= first + (1 << 20), "a second later starts past a million tabs of the first");
        assert!(later < 1 << 53, "whole in JavaScript");
        assert!(first_page_id(std::time::UNIX_EPOCH + std::time::Duration::from_secs(u32::MAX as u64 * 3)) < 1 << 53);
    }

    fn signals(f: impl FnOnce(&mut Signals)) -> Signals {
        let mut s = Signals::default();
        f(&mut s);
        s
    }

    #[test]
    fn a_whole_page_check_is_known_by_its_title_or_its_address() {
        use equipe_protocol::HumanCheck::Challenge;
        for title in ["Just a moment...", "Just a moment…", "잠시만 기다리십시오…", "しばらくお待ちください...", "Attention Required! | Cloudflare", "请稍候…"] {
            assert_eq!(human_check("https://dash.cloudflare.com/", title, None), Some(Challenge), "{title}");
        }
        assert_eq!(human_check("https://example.com/?__cf_chl_rt_tk=abc", "", None), Some(Challenge));
        assert_eq!(human_check("https://example.com/cdn-cgi/challenge-platform/h/b/orchestrate", "example.com", None), Some(Challenge));
        // A title that only starts the same is something else.
        assert_eq!(human_check("https://blog.example.com/p/1", "Just a moment of silence: notes from a quiet week in the mountains", None), None);
        assert_eq!(human_check("https://appstoreconnect.apple.com/apps", "App Store Connect", None), None);
    }

    #[test]
    fn a_captcha_counts_while_it_is_in_sight_and_unanswered() {
        use equipe_protocol::HumanCheck::Captcha;
        let turnstile = signals(|s| {
            s.frames = vec!["https://challenges.cloudflare.com/cdn-cgi/challenge-platform/h/g/turnstile/if/ov2/av0/rcv/abc".into()];
            s.widget = true;
        });
        assert_eq!(human_check("https://dash.cloudflare.com/login", "Cloudflare", Some(&turnstile)), Some(Captcha));
        let answered = Signals { solved: true, ..turnstile.clone() };
        assert_eq!(human_check("https://dash.cloudflare.com/login", "Cloudflare", Some(&answered)), None, "answered: nothing to do");
        // A frame alone counts when the page asks whether a person is there.
        let asked = signals(|s| {
            s.frames = vec!["https://www.google.com/recaptcha/api2/anchor?k=x&size=normal".into()];
            s.text = "please verify you are human to continue".into();
        });
        assert_eq!(human_check("https://shop.example.com/", "Shop", Some(&asked)), Some(Captcha));
        // An invisible reCAPTCHA's badge on an ordinary page is none.
        let badge = signals(|s| s.frames = vec!["https://www.google.com/recaptcha/api2/anchor?k=x&size=invisible".into()]);
        assert_eq!(human_check("https://shop.example.com/", "Shop", Some(&badge)), None);
        let quiet = signals(|s| s.frames = vec!["https://newassets.hcaptcha.com/captcha/v1/abc/static/hcaptcha.html#frame=checkbox".into()]);
        assert_eq!(human_check("https://shop.example.com/", "Shop", Some(&quiet)), None, "a frame in a page that asks nothing");
    }

    #[test]
    fn a_sign_in_wall_and_a_code_from_the_phone_are_the_persons() {
        use equipe_protocol::HumanCheck::{SignIn, TwoFactor};
        let password = signals(|s| s.password = true);
        assert_eq!(human_check("https://vercel.com/login", "Login – Vercel", Some(&password)), Some(SignIn));
        assert_eq!(human_check("https://accounts.google.com/v3/signin/identifier", "Google 계정", Some(&password)), Some(SignIn));
        assert_eq!(human_check("https://app.example.com/settings/security", "Change password", Some(&password)), None, "a password field elsewhere");
        let otp = signals(|s| s.otp = true);
        assert_eq!(human_check("https://github.com/sessions/two-factor/app", "Two-factor authentication", Some(&otp)), Some(TwoFactor));
        let code = signals(|s| {
            s.code = true;
            s.text = "6자리 인증 코드를 입력하세요".into();
        });
        assert_eq!(human_check("https://nid.naver.com/", "네이버", Some(&code)), Some(TwoFactor));
        assert_eq!(human_check("https://example.com/search", "Search", Some(&signals(|s| s.code = true))), None, "a short field alone");
    }

    #[test]
    fn a_tab_is_told_with_what_it_waits_for_and_a_web_icon_only() {
        let page = HostPage {
            id: 7,
            url: "https://dash.cloudflare.com/".into(),
            title: "Just a moment...".into(),
            loading: false,
            favicon: Some("data:image/png;base64,AAAA".into()),
            signals: None,
            crashed: false,
        }
        .page();
        assert_eq!(page.check, Some(equipe_protocol::HumanCheck::Challenge));
        assert_eq!(page.favicon, None, "the screen fetches icons from the web only");
        let raw = r#"{"type":"tabs","lane":"acme/backend","pages":[{"id":3,"url":"https://a.com/","title":"A","favicon":"https://a.com/favicon.ico","signals":{"password":true}}]}"#;
        let HostEvent::Tabs { pages, .. } = serde_json::from_str::<HostEvent>(raw).unwrap() else { panic!() };
        assert_eq!(pages[0].clone().page().favicon.as_deref(), Some("https://a.com/favicon.ico"));
        assert!(pages[0].signals.as_ref().unwrap().password);
    }

    #[test]
    fn a_page_waiting_for_a_person_pauses_its_member_until_it_passes() {
        use equipe_protocol::{BrowserPage, HumanCheck};
        let page = |id: u64, check: Option<HumanCheck>| BrowserPage { id, url: format!("https://site{id}.com/"), check, ..Default::default() };
        let never = |_: u64, _: &str| false;
        let quiet = [page(1, None), page(2, None)];
        assert_eq!(check_step(&quiet, Some(1), false, false, never), CheckStep::Nothing);
        // The member's own tab first, whatever else waits.
        let both = [page(1, Some(HumanCheck::SignIn)), page(2, Some(HumanCheck::Challenge))];
        assert_eq!(check_step(&both, Some(2), false, false, never), CheckStep::Pause { page: 2, url: "https://site2.com/".into(), check: HumanCheck::Challenge });
        assert_eq!(check_step(&both, Some(2), true, true, never), CheckStep::Nothing, "paused already");
        assert_eq!(check_step(&both, Some(2), false, true, never), CheckStep::Nothing, "in the person's hands for something else");
        // Passed: back it goes.
        assert_eq!(check_step(&quiet, Some(2), true, true, never), CheckStep::Release);
        // Handed back still waiting: left to the member.
        let handed = |id: u64, _: &str| id == 2;
        assert_eq!(check_step(&[page(2, Some(HumanCheck::Challenge))], Some(2), false, false, handed), CheckStep::Nothing);
    }

    #[test]
    fn a_site_is_named_by_its_host() {
        assert_eq!(site_of("https://www.dash.cloudflare.com/login?x=1"), "dash.cloudflare.com");
        assert_eq!(site_of("https://user@appstoreconnect.apple.com/apps"), "appstoreconnect.apple.com");
        assert_eq!(site_of("about:blank"), "about:blank");
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
