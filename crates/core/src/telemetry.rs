//! Anonymous usage events, only after the person's yes — the contract is
//! docs/TELEMETRY.md, and this file must say no more than it does.
//!
//! The daemon sends them: the desktop app, the terminal and the phone all
//! approve through it. What the person said, the install's random id and the
//! day's counts live in `<equipe home>/telemetry.json`; squads are named
//! there by a random id of their own, and their names never leave it.
//! Nothing is sent while the answer is `unset` or `off`, under
//! `EQUIPE_TELEMETRY=off`, or in CI. A send that fails is tried once more,
//! then dropped; the queue is small and drops when full.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use equipe_protocol::telemetry::{Consent, TelemetryStatus};
use equipe_protocol::{Engine, TicketKind};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

/// PostHog US, project 700951 — the site's, so a visit, a download and a
/// first approval are one funnel. A project key only writes events: it is
/// public by design, as the site's own page carries it.
pub const API_KEY: &str = "phc_ukKw2k6qVu6NPyQowBNVUdVppRZTgn5kZ89nYLs6pCnx";
pub const HOST: &str = "https://us.i.posthog.com";

const FILE: &str = "telemetry.json";
/// Events waiting to go; more are dropped.
const QUEUE: usize = 256;
/// One request carries at most this many.
const BATCH: usize = 50;
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// How often a finished day and a first run are looked for when nothing happens.
const TICK: Duration = Duration::from_secs(600);

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    consent: Consent,
    /// The install's anonymous id: a random UUID, made at the yes and
    /// forgotten at a no.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    install: Option<String>,
    #[serde(default)]
    first_run_sent: bool,
    /// Squad name → its random id. Kept here, never sent.
    #[serde(default)]
    squads: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    day: Option<Day>,
}

/// What the squads did on one UTC day, sent as `daily_active` once it is over.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Day {
    date: String,
    /// By squad id.
    #[serde(default)]
    squads: BTreeMap<String, Active>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Active {
    #[serde(default)]
    person_accepts: u32,
    #[serde(default)]
    engines: BTreeSet<String>,
}

fn load(home: &Path) -> State {
    std::fs::read(home.join(FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn save(home: &Path, state: &State) -> Result<(), String> {
    let _ = std::fs::create_dir_all(home);
    let path = home.join(FILE);
    let tmp = home.join(format!("{FILE}.tmp"));
    let bytes = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, &path)).map_err(|e| format!("{}: {e}", path.display()))
}

/// What in the environment holds events back whatever the person said.
pub fn held_by() -> Option<&'static str> {
    held_by_in(|key| std::env::var(key).ok())
}

fn held_by_in(var: impl Fn(&str) -> Option<String>) -> Option<&'static str> {
    let set = |key: &str, no: &[&str]| var(key).map(|v| v.trim().to_ascii_lowercase()).is_some_and(|v| !v.is_empty() && !no.contains(&v.as_str()));
    if var("EQUIPE_TELEMETRY").is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no")) {
        return Some("EQUIPE_TELEMETRY");
    }
    if set("CI", &["false", "0"]) {
        return Some("CI");
    }
    None
}

pub fn status(home: &Path) -> TelemetryStatus {
    status_with(home, held_by())
}

fn status_with(home: &Path, held: Option<&'static str>) -> TelemetryStatus {
    let consent = load(home).consent;
    TelemetryStatus { consent, sending: consent == Consent::On && held.is_none(), held_by: held.map(str::to_owned) }
}

/// The person's yes or no. A yes makes the install's id; a no forgets it and
/// every squad's, so a later yes starts as a new install.
pub fn set(home: &Path, on: bool) -> Result<TelemetryStatus, String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut state = load(home);
    if on {
        state.consent = Consent::On;
        if state.install.is_none() {
            state.install = Some(uuid_v4());
        }
    } else {
        state = State { consent: Consent::Off, ..State::default() };
    }
    save(home, &state)?;
    Ok(status(home))
}

/// One file, read and written by the daemon's runtime and its timer.
static LOCK: Mutex<()> = Mutex::new(());

/// Who accepted handed-in work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accepter {
    Person,
    Pm,
}

/// Who put a ticket on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Maker {
    Person,
    Pm,
    Member,
}

/// The squad as an event sees it: how many members, on which engines.
#[derive(Debug, Clone, Default)]
pub struct Shape {
    pub team_size: usize,
    pub engines: Vec<Engine>,
}

/// What happened in a squad. Only these few facts go out — no title, no
/// text, no name, no path.
#[derive(Debug, Clone, Copy)]
pub enum Happened {
    TicketCreated { kind: TicketKind, by: Maker },
    WorkHandedIn { kind: TicketKind },
    WorkAccepted { kind: TicketKind, by: Accepter },
}

/// The daemon's sender. [`Telemetry::off`] sends nothing, ever (tests, the
/// terminal's own runtime).
#[derive(Clone)]
pub struct Telemetry {
    inner: Option<Arc<Inner>>,
}

struct Inner {
    home: PathBuf,
    version: String,
    held: Option<&'static str>,
    queue: mpsc::Sender<Value>,
}

impl Telemetry {
    pub fn off() -> Self {
        Self { inner: None }
    }

    /// The daemon's: to PostHog, held back as the environment says.
    pub fn start(home: &Path, version: &str) -> Self {
        Self::start_at(home, version, HOST, held_by())
    }

    /// To `host`, held back by `held` (tests: a local server).
    pub fn start_at(home: &Path, version: &str, host: &str, held: Option<&'static str>) -> Self {
        let (queue, rx) = mpsc::channel(QUEUE);
        let inner = Arc::new(Inner { home: home.to_owned(), version: version.to_owned(), held, queue });
        let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(20)).build().unwrap_or_default();
        tokio::spawn(deliver(client, format!("{}/batch/", host.trim_end_matches('/')), rx));
        let ticker = Arc::downgrade(&inner);
        tokio::spawn(async move {
            loop {
                let Some(inner) = ticker.upgrade() else { break };
                inner.send(|state, now| due(state, &inner.version, now));
                drop(inner);
                tokio::time::sleep(TICK).await;
            }
        });
        Self { inner: Some(inner) }
    }

    /// Something happened in `squad`: sent when the person said yes.
    pub fn record(&self, squad: &str, shape: &Shape, happened: Happened) {
        if let Some(inner) = &self.inner {
            inner.send(|state, now| record(state, &inner.version, now, squad, shape, happened));
        }
    }
}

impl Inner {
    fn send(&self, events: impl FnOnce(&mut State, u64) -> Vec<Value>) {
        if self.held.is_some() {
            return;
        }
        let out = {
            let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut state = load(&self.home);
            if state.consent != Consent::On || state.install.is_none() {
                return;
            }
            let out = events(&mut state, now_secs());
            if out.is_empty() || save(&self.home, &state).is_err() {
                return;
            }
            out
        };
        for event in out {
            // Full: dropped, as any send that fails.
            let _ = self.queue.try_send(event);
        }
    }
}

async fn deliver(client: reqwest::Client, url: String, mut rx: mpsc::Receiver<Value>) {
    while let Some(first) = rx.recv().await {
        let mut batch = vec![first];
        while batch.len() < BATCH {
            match rx.try_recv() {
                Ok(event) => batch.push(event),
                Err(_) => break,
            }
        }
        let body = json!({ "api_key": API_KEY, "batch": batch }).to_string();
        for attempt in 0..2 {
            let sent = client.post(&url).header("content-type", "application/json").body(body.clone()).send().await;
            if sent.is_ok_and(|r| r.status().is_success()) {
                break;
            }
            if attempt == 0 {
                tokio::time::sleep(RETRY_AFTER).await;
            }
        }
    }
}

/// What is due with nothing happening: the first run, and a day that is over.
fn due(state: &mut State, version: &str, now: u64) -> Vec<Value> {
    let mut out = Vec::new();
    let install = state.install.clone().unwrap_or_default();
    if !state.first_run_sent {
        state.first_run_sent = true;
        out.push(event(&install, "first_run", Map::new(), version, now));
    }
    let today = date(now);
    if state.day.as_ref().is_some_and(|d| d.date != today) {
        let day = state.day.take().unwrap_or_default();
        for (squad, active) in day.squads {
            let mut props = Map::new();
            props.insert("squad".into(), json!(squad));
            props.insert("day".into(), json!(day.date));
            props.insert("person_accepts".into(), json!(bucket(active.person_accepts)));
            props.insert("engines".into(), json!(active.engines.into_iter().collect::<Vec<_>>().join("+")));
            out.push(event(&install, "daily_active", props, version, end_of(&day.date).unwrap_or(now)));
        }
    }
    out
}

fn record(state: &mut State, version: &str, now: u64, squad: &str, shape: &Shape, happened: Happened) -> Vec<Value> {
    let mut out = due(state, version, now);
    let install = state.install.clone().unwrap_or_default();
    let id = match state.squads.get(squad) {
        Some(id) => id.clone(),
        None => {
            let id = uuid_v4();
            state.squads.insert(squad.to_owned(), id.clone());
            let mut props = Map::new();
            props.insert("squad".into(), json!(id));
            props.insert("team_size".into(), json!(shape.team_size));
            out.push(event(&install, "squad_created", props, version, now));
            id
        }
    };
    let day = state.day.get_or_insert_with(|| Day { date: date(now), ..Day::default() });
    let active = day.squads.entry(id.clone()).or_default();
    active.engines.extend(shape.engines.iter().map(|e| engine(*e).to_owned()));
    let mut props = Map::new();
    props.insert("squad".into(), json!(id));
    let name = match happened {
        Happened::TicketCreated { kind, by } => {
            props.insert("kind".into(), json!(kind.label()));
            props.insert("by".into(), json!(match by {
                Maker::Person => "person",
                Maker::Pm => "pm",
                Maker::Member => "member",
            }));
            "ticket_created"
        }
        Happened::WorkHandedIn { kind } => {
            props.insert("kind".into(), json!(kind.label()));
            "work_handed_in"
        }
        Happened::WorkAccepted { kind, by } => {
            if by == Accepter::Person {
                active.person_accepts += 1;
            }
            props.insert("kind".into(), json!(kind.label()));
            props.insert("by".into(), json!(if by == Accepter::Person { "person" } else { "pm" }));
            "work_accepted"
        }
    };
    out.push(event(&install, name, props, version, now));
    out
}

/// One PostHog event: anonymous — no person profile, no place from the IP.
fn event(install: &str, name: &str, mut props: Map<String, Value>, version: &str, at: u64) -> Value {
    props.insert("app_version".into(), json!(version));
    props.insert("os".into(), json!(std::env::consts::OS));
    props.insert("$lib".into(), json!("equipe-daemon"));
    props.insert("$process_person_profile".into(), json!(false));
    props.insert("$geoip_disable".into(), json!(true));
    json!({ "event": name, "distinct_id": install, "timestamp": rfc3339(at), "properties": props })
}

fn engine(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "claude",
        Engine::Codex => "codex",
    }
}

/// The person's approvals in a day, as a range rather than a count.
fn bucket(n: u32) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        2..=4 => "2-4",
        5..=9 => "5-9",
        _ => "10+",
    }
}

fn uuid_v4() -> String {
    use std::io::Read as _;
    let mut b = [0u8; 16];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    if read.is_err() {
        // No /dev/urandom: the clock's nanoseconds and the process, mixed — never the machine's.
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default() ^ (u128::from(std::process::id()) << 64);
        let mut x = seed | 1;
        for byte in &mut b {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *byte = (x >> 32) as u8;
        }
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `2026-11-01`, in UTC.
fn date(secs: u64) -> String {
    let (y, m, d) = civil((secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

fn rfc3339(secs: u64) -> String {
    let rest = secs % 86_400;
    format!("{}T{:02}:{:02}:{:02}Z", date(secs), rest / 3600, rest % 3600 / 60, rest % 60)
}

/// The day's last second, for its `daily_active`.
fn end_of(day: &str) -> Option<u64> {
    let mut parts = day.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    // Days from civil, the inverse of `civil`.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + 86_399).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    fn home() -> PathBuf {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("equipe-telemetry-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn shape() -> Shape {
        Shape { team_size: 5, engines: vec![Engine::Claude, Engine::Codex] }
    }

    /// A local "PostHog": the bodies of what reaches it, and how many connections did.
    async fn server() -> (String, Arc<Mutex<Vec<Value>>>, Arc<std::sync::atomic::AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (seen, count) = (bodies.clone(), connections.clone());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let seen = seen.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        let Ok(n) = socket.read(&mut chunk).await else { return };
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        let text = String::from_utf8_lossy(&buf).to_string();
                        if let Some(end) = text.find("\r\n\r\n") {
                            let length = text[..end].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                            if buf.len() >= end + 4 + length {
                                let body: Value = serde_json::from_slice(&buf[end + 4..end + 4 + length]).unwrap();
                                seen.lock().unwrap().push(body);
                                let _ = socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}").await;
                                return;
                            }
                        }
                    }
                });
            }
        });
        (host, bodies, connections)
    }

    fn happen(t: &Telemetry) {
        t.record("acme-secret-name", &shape(), Happened::TicketCreated { kind: TicketKind::Code, by: Maker::Person });
        t.record("acme-secret-name", &shape(), Happened::WorkHandedIn { kind: TicketKind::Task });
        t.record("acme-secret-name", &shape(), Happened::WorkAccepted { kind: TicketKind::Task, by: Accepter::Person });
    }

    #[tokio::test]
    async fn nothing_reaches_the_network_while_unset_off_or_held() {
        let (host, bodies, connections) = server().await;
        // Unset: never asked.
        let unset = home();
        happen(&Telemetry::start_at(&unset, "0.0.0", &host, None));
        // Off: said no.
        let off = home();
        set(&off, false).unwrap();
        happen(&Telemetry::start_at(&off, "0.0.0", &host, None));
        // On, but `EQUIPE_TELEMETRY=off` or CI.
        let held = home();
        set(&held, true).unwrap();
        happen(&Telemetry::start_at(&held, "0.0.0", &host, Some("CI")));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(connections.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(bodies.lock().unwrap().is_empty());
        assert!(!unset.join(FILE).exists(), "nothing is even kept while unset");
        assert_eq!(status_with(&held, Some("CI")), TelemetryStatus { consent: Consent::On, sending: false, held_by: Some("CI".into()) });
    }

    #[tokio::test]
    async fn on_sends_anonymous_events_in_posthogs_batch_form() {
        let (host, bodies, _) = server().await;
        let dir = home();
        set(&dir, true).unwrap();
        let t = Telemetry::start_at(&dir, "0.86.4", &host, None);
        happen(&t);
        let mut events: Vec<Value> = Vec::new();
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            events = bodies.lock().unwrap().iter().flat_map(|b| b["batch"].as_array().cloned().unwrap_or_default()).collect();
            if events.len() >= 5 {
                break;
            }
        }
        let names: Vec<&str> = events.iter().map(|e| e["event"].as_str().unwrap()).collect();
        assert_eq!(names, ["first_run", "squad_created", "ticket_created", "work_handed_in", "work_accepted"]);
        let body = &bodies.lock().unwrap()[0];
        assert_eq!(body["api_key"], API_KEY);
        let install = load(&dir).install.unwrap();
        assert_eq!(install.len(), 36);
        let squad = &events[1]["properties"]["squad"];
        for e in &events {
            assert_eq!(e["distinct_id"], install.as_str());
            assert_eq!(e["properties"]["$process_person_profile"], false);
            assert_eq!(e["properties"]["app_version"], "0.86.4");
            assert!(e["timestamp"].as_str().unwrap().ends_with('Z'));
            if e["event"] != "first_run" {
                assert_eq!(&e["properties"]["squad"], squad);
            }
        }
        assert_eq!(events[1]["properties"]["team_size"], 5);
        assert_eq!((events[2]["properties"]["kind"].clone(), events[2]["properties"]["by"].clone()), (json!("code"), json!("person")));
        assert_eq!(events[4]["properties"]["by"], "person");
    }

    /// The contract's allow-list: every property any event carries.
    const ALLOWED: &[&str] = &["squad", "team_size", "kind", "by", "day", "person_accepts", "engines", "app_version", "os", "$lib", "$process_person_profile", "$geoip_disable"];

    #[test]
    fn no_event_carries_a_name_a_path_or_anything_off_the_list() {
        let mut state = State { consent: Consent::On, install: Some(uuid_v4()), ..State::default() };
        let day = 1_793_534_400; // 2026-11-01T12:00:00Z
        let mut events = Vec::new();
        events.extend(record(&mut state, "1.0.0", day, "acme-secret-name", &shape(), Happened::TicketCreated { kind: TicketKind::Action, by: Maker::Member }));
        events.extend(record(&mut state, "1.0.0", day, "acme-secret-name", &shape(), Happened::WorkAccepted { kind: TicketKind::Code, by: Accepter::Pm }));
        for _ in 0..3 {
            events.extend(record(&mut state, "1.0.0", day, "acme-secret-name", &shape(), Happened::WorkAccepted { kind: TicketKind::Code, by: Accepter::Person }));
        }
        // The next day: yesterday's `daily_active` goes first.
        events.extend(record(&mut state, "1.0.0", day + 86_400, "acme-secret-name", &shape(), Happened::WorkHandedIn { kind: TicketKind::Code }));
        let daily = events.iter().find(|e| e["event"] == "daily_active").expect("yesterday's daily_active");
        assert_eq!(daily["properties"]["person_accepts"], "2-4");
        assert_eq!(daily["properties"]["engines"], "claude+codex");
        assert_eq!(daily["properties"]["day"], "2026-11-01");
        assert_eq!(daily["timestamp"], "2026-11-01T23:59:59Z");
        for e in &events {
            let keys: BTreeSet<&str> = e.as_object().unwrap().keys().map(String::as_str).collect();
            assert_eq!(keys, BTreeSet::from(["event", "distinct_id", "timestamp", "properties"]));
            for key in e["properties"].as_object().unwrap().keys() {
                assert!(ALLOWED.contains(&key.as_str()), "{key} isn't in docs/TELEMETRY.md");
            }
            let text = e.to_string();
            for banned in ["acme-secret-name", "/Users", "/home", "@", "title", "path", "email", "project"] {
                assert!(!text.contains(banned), "{banned} in {text}");
            }
        }
    }

    #[test]
    fn a_no_forgets_the_id_and_a_later_yes_is_a_new_install() {
        let dir = home();
        let first = { set(&dir, true).unwrap(); load(&dir).install.unwrap() };
        set(&dir, false).unwrap();
        let state = load(&dir);
        assert_eq!((state.consent, state.install.is_none(), state.squads.is_empty()), (Consent::Off, true, true));
        set(&dir, true).unwrap();
        assert_ne!(load(&dir).install.unwrap(), first);
    }

    /// The real PostHog, by hand: `cargo test -p equipe-core --lib live_posthog -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_posthog() {
        let dir = home();
        set(&dir, true).unwrap();
        let t = Telemetry::start_at(&dir, env!("CARGO_PKG_VERSION"), HOST, None);
        happen(&t);
        tokio::time::sleep(Duration::from_secs(5)).await;
        println!("distinct_id {}", load(&dir).install.unwrap());
    }

    #[test]
    fn the_environment_holds_events_back() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| v.to_string());
        assert_eq!(held_by_in(env(&[])), None);
        assert_eq!(held_by_in(env(&[("EQUIPE_TELEMETRY", "off")])), Some("EQUIPE_TELEMETRY"));
        assert_eq!(held_by_in(env(&[("EQUIPE_TELEMETRY", "on")])), None);
        assert_eq!(held_by_in(env(&[("CI", "true")])), Some("CI"));
        assert_eq!(held_by_in(env(&[("CI", "false")])), None);
    }

    #[test]
    fn dates_are_utc_days() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(rfc3339(1_793_534_400), "2026-11-01T12:00:00Z");
        assert_eq!(end_of("2026-11-01"), Some(1_793_577_599));
        assert_eq!(date(end_of("2024-02-29").unwrap()), "2024-02-29");
        assert_eq!(bucket(0), "0");
        assert_eq!(bucket(12), "10+");
        let id = uuid_v4();
        assert_eq!((id.len(), &id[14..15]), (36, "4"));
    }
}
