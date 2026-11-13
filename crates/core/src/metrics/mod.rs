//! A product's numbers (`protocol::metrics`, docs/METRICS.md): the services a
//! squad's projects are connected to, their keys, the daily snapshots of what
//! each reported, and the periods screens and members read from them.
//!
//! - `metrics/connections.json` (the person's only): every connection, its
//!   settings that aren't secret and the names of its keys — no key.
//! - The keys: the daemon machine's Keychain, in equipe's one item
//!   (`crate::keychain`; under `equipe.metrics:<connection id>`, the secret
//!   fields as one JSON object), read once per daemon.
//! - `metrics/<squad>/<connection>/<YYYY-MM-DD>.json`: what a fetch that day
//!   brought ([`Snapshot`]). Read in day order, a later fetch's value of a day
//!   wins, so days a service has forgotten (GitHub's past 14) stay.
//!
//! An adapter's error message has every key cut out before it is kept,
//! answered or logged.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::protocol::metrics::{
    Bucket, ConnectParams, Connection, DisconnectParams, Disconnected, ErrorCode, FetchError, Fetched, MetricDef, MetricValue,
    Point, QueryParams, QueryResult, RefreshParams, SCHEMA, Sample, Shape, Share, Snapshot, SourceKind, Sources, SourcesParams,
    Summary, SummaryParams, Tile,
};
use crate::routines::Clock;

pub mod adapters;
pub mod tool;

/// The Keychain service the keys are kept under.
pub const SERVICE: &str = "equipe.metrics";
/// The hour (local) of the daily fetch.
pub const DAILY_HOUR: u8 = 7;
/// A failed fetch is tried again after this long, unless the service said when.
const RETRY: u64 = 60 * 60;
/// How long one fetch may take.
const FETCH_WAIT: Duration = Duration::from_secs(90);
/// How long an ask of the Keychain may take, its turn included. A build of
/// equipe it doesn't know yet is asked about on the Mac, and that ask waits
/// for the person for as long as it takes.
const KEYCHAIN_WAIT: Duration = Duration::from_secs(30);
/// Days asked for on a connection's first fetch, and on later ones.
const FIRST_DAYS: u32 = 400;
const LATER_DAYS: u32 = 14;
/// Tiles on the phone's card.
const TILES: usize = 4;
/// The North Star (docs/TELEMETRY.md): first on the phone's card.
pub const NORTH_STAR: &str = "north_star";
/// A `rolling` metric's window, and its series: this many values that far apart.
pub const WINDOW: i64 = 7;
pub const ROLLING_POINTS: i64 = 12;

// ---- Days ----

/// Days since 1970-01-01 of a `YYYY-MM-DD`.
pub fn day_number(day: &str) -> Option<i64> {
    let mut parts = day.trim().splitn(3, '-');
    let (y, m, d): (i64, i64, i64) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || day.trim().len() != 10 {
        return None;
    }
    // Howard Hinnant's days-from-civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let n = era * 146_097 + doe - 719_468;
    (day_string(n) == day.trim()).then_some(n)
}

/// `YYYY-MM-DD` of a day counted from 1970-01-01.
pub fn day_string(n: i64) -> String {
    crate::signal_sources::civil_date(n.max(0) as u64)
}

/// The calendar day of a moment, by `clock`.
pub fn day_of(clock: &impl Clock, unix: u64) -> i64 {
    let p = clock.parts(unix);
    day_number(&format!("{:04}-{:02}-{:02}", p.year, p.month, p.day)).unwrap_or(0)
}

/// Monday of the week `n` is in.
fn monday(n: i64) -> i64 {
    // 1970-01-01 was a Thursday.
    n - (n + 3).rem_euclid(7)
}

// ---- Keys ----

/// Where keys go: the Keychain, or (in tests) memory.
pub trait Keys: Send {
    fn set(&mut self, id: &str, keys: &BTreeMap<String, String>) -> Result<(), String>;
    fn get(&self, id: &str) -> Result<BTreeMap<String, String>, String>;
    fn delete(&mut self, id: &str) -> Result<(), String>;
}

/// The daemon machine's Keychain.
pub struct Keychain;

/// The keys live in equipe's one Keychain item ([`crate::keychain`]): read
/// once per daemon, never one dialog per connection.
#[cfg(target_os = "macos")]
impl Keys for Keychain {
    fn set(&mut self, id: &str, keys: &BTreeMap<String, String>) -> Result<(), String> {
        let text = serde_json::to_string(keys).map_err(|e| e.to_string())?;
        crate::keychain::global().set(SERVICE, id, &text)
    }

    fn get(&self, id: &str) -> Result<BTreeMap<String, String>, String> {
        let text = crate::keychain::global().get(SERVICE, id, false)?;
        serde_json::from_str(&text).map_err(|_| "the kept keys aren't readable".into())
    }

    fn delete(&mut self, id: &str) -> Result<(), String> {
        crate::keychain::global().delete(SERVICE, id)
    }
}

#[cfg(not(target_os = "macos"))]
impl Keys for Keychain {
    fn set(&mut self, _: &str, _: &BTreeMap<String, String>) -> Result<(), String> {
        Err("metrics keys need macOS's Keychain, which this machine doesn't have".into())
    }

    fn get(&self, _: &str) -> Result<BTreeMap<String, String>, String> {
        Err("metrics keys need macOS's Keychain, which this machine doesn't have".into())
    }

    fn delete(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Keys in memory, for tests.
#[derive(Default)]
pub struct InMemory(pub BTreeMap<String, BTreeMap<String, String>>);

impl Keys for InMemory {
    fn set(&mut self, id: &str, keys: &BTreeMap<String, String>) -> Result<(), String> {
        self.0.insert(id.to_owned(), keys.clone());
        Ok(())
    }

    fn get(&self, id: &str) -> Result<BTreeMap<String, String>, String> {
        self.0.get(id).cloned().ok_or_else(|| "no keys kept".into())
    }

    fn delete(&mut self, id: &str) -> Result<(), String> {
        self.0.remove(id);
        Ok(())
    }
}

/// `text` with every key cut out.
pub fn redact(text: &str, keys: &BTreeMap<String, String>) -> String {
    let mut text = text.to_owned();
    for key in keys.values().map(|k| k.trim()).filter(|k| k.len() >= 4) {
        text = text.replace(key, "•••");
    }
    text
}

// ---- Errors ----

/// A request that can't be answered: a JSON-RPC error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub code: i64,
    pub message: String,
}

/// docs/METRICS.md, "Errors".
pub const BAD_PARAMS: i64 = -32602;
pub const UNKNOWN_KIND: i64 = -32010;
pub const BAD_SETTINGS: i64 = -32011;
pub const NOT_FOUND: i64 = -32012;
pub const KEYCHAIN: i64 = -32013;
pub const STORAGE: i64 = -32014;

fn refuse(code: i64, message: impl Into<String>) -> Refused {
    Refused { code, message: message.into() }
}

// ---- On disk ----

fn dir(home: &Path) -> PathBuf {
    home.join("metrics")
}

fn connections_file(home: &Path) -> PathBuf {
    dir(home).join("connections.json")
}

fn snapshots_dir(home: &Path, squad: &str, id: &str) -> PathBuf {
    dir(home).join(squad).join(id)
}

/// A squad's name as a folder may be named: no separators, no `..`.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// Every connection on the machine.
pub fn connections(home: &Path) -> Vec<Connection> {
    std::fs::read_to_string(connections_file(home)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn private_write(path: &Path, text: &str) -> Result<(), String> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    let _ = std::fs::set_permissions(dir_root(path), std::fs::Permissions::from_mode(0o700));
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, text).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    let _ = std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o600));
    std::fs::rename(&partial, path).map_err(|e| e.to_string())
}

/// The `metrics` folder a path is in.
#[cfg(unix)]
fn dir_root(path: &Path) -> &Path {
    path.ancestors().find(|a| a.file_name().is_some_and(|n| n == "metrics")).unwrap_or(path)
}

fn keep_connections(home: &Path, all: &[Connection]) -> Result<(), String> {
    let text = serde_json::to_string_pretty(all).map_err(|e| e.to_string())?;
    private_write(&connections_file(home), &text)
}

/// The snapshots of a connection, oldest first.
pub fn snapshots(home: &Path, squad: &str, id: &str) -> Vec<Snapshot> {
    let Ok(entries) = std::fs::read_dir(snapshots_dir(home, squad, id)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> =
        entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    files
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .filter_map(|t| serde_json::from_str::<Snapshot>(&t).ok())
        .filter(|s| s.schema <= SCHEMA)
        .collect()
}

/// Each (metric, day)'s point, a later snapshot's winning.
fn merged(snapshots: &[Snapshot]) -> BTreeMap<(String, i64), Point> {
    let mut all = BTreeMap::new();
    for snapshot in snapshots {
        for point in &snapshot.points {
            if let Some(n) = day_number(&point.day) {
                all.insert((point.metric.clone(), n), point.clone());
            }
        }
    }
    all
}

// ---- The service ----

/// The daemon's metrics: connections, keys and fetches. Shared by every
/// screen's connection and the daily fetch.
pub struct Metrics {
    home: PathBuf,
    /// Asked on the blocking pool, one at a time ([`Metrics::ask_keychain`]):
    /// the Keychain's calls block, for good while it waits on the person.
    keys: Arc<Mutex<Box<dyn Keys>>>,
    /// The turn to ask the Keychain, held until the ask comes back.
    keychain_turn: Arc<tokio::sync::Semaphore>,
    keychain_wait: Duration,
    client: reqwest::Client,
    /// connections.json is read, changed and written by one at a time.
    file: Mutex<()>,
    /// Connections being fetched now.
    fetching: Mutex<HashSet<String>>,
    /// Offer the fake source.
    pub with_fake: bool,
}

impl Metrics {
    pub fn new(home: &Path, keys: Box<dyn Keys>) -> Self {
        Self {
            home: home.to_owned(),
            keys: Arc::new(Mutex::new(keys)),
            keychain_turn: Arc::new(tokio::sync::Semaphore::new(1)),
            keychain_wait: KEYCHAIN_WAIT,
            client: reqwest::Client::new(),
            file: Mutex::new(()),
            fetching: Mutex::new(HashSet::new()),
            with_fake: std::env::var_os("EQUIPE_METRICS_FAKE").is_some(),
        }
    }

    /// The daemon's: keys in the Keychain.
    pub fn open(home: &Path) -> Self {
        Self::new(home, Box::new(Keychain))
    }

    fn change<T>(&self, f: impl FnOnce(&mut Vec<Connection>) -> Result<T, Refused>) -> Result<T, Refused> {
        let _held = self.file.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut all = connections(&self.home);
        let out = f(&mut all)?;
        keep_connections(&self.home, &all).map_err(|e| refuse(STORAGE, format!("the connections couldn't be saved: {e}")))?;
        Ok(out)
    }

    fn of(&self, squad: &str, project: Option<&str>) -> Vec<Connection> {
        connections(&self.home).into_iter().filter(|c| c.squad == squad && project.is_none_or(|p| c.project.as_deref() == Some(p))).collect()
    }

    /// A JSON-RPC request: its method and params → its result.
    pub async fn call(self: &Arc<Self>, method: &str, params: serde_json::Value, now: u64) -> Result<serde_json::Value, Refused> {
        use crate::protocol::metrics as m;
        fn parse<T: serde::de::DeserializeOwned>(method: &str, params: serde_json::Value) -> Result<T, Refused> {
            serde_json::from_value(params).map_err(|e| refuse(BAD_PARAMS, format!("{method}: {e}")))
        }
        fn answer(value: impl serde::Serialize) -> Result<serde_json::Value, Refused> {
            serde_json::to_value(value).map_err(|e| refuse(STORAGE, e.to_string()))
        }
        let today = day_of(&crate::routines::Local, now);
        match method {
            m::SOURCES => answer(self.sources(&parse(method, params)?)?),
            m::CONNECT => {
                let p = parse(method, params)?;
                answer(self.keyed(move |me| me.connect(p, now)).await?)
            }
            m::DISCONNECT => {
                let p = parse(method, params)?;
                answer(self.keyed(move |me| me.disconnect(&p)).await?)
            }
            m::REFRESH => answer(self.refresh(&parse(method, params)?, now, today).await?),
            m::QUERY => answer(query(&self.home, &parse(method, params)?, today)?),
            m::SUMMARY => answer(summary(&self.home, &parse(method, params)?, today)?),
            _ => Err(refuse(-32601, "unknown method")),
        }
    }

    pub fn sources(&self, p: &SourcesParams) -> Result<Sources, Refused> {
        if !valid_name(&p.squad) {
            return Err(refuse(BAD_PARAMS, "name a squad"));
        }
        Ok(Sources { kinds: adapters::offered(self.with_fake), connections: self.of(&p.squad, p.project.as_deref()) })
    }

    /// A new connection, or new settings or keys for one. Answers at once; the
    /// first fetch is the caller's to start ([`Metrics::refresh_one`]).
    pub fn connect(&self, p: ConnectParams, now: u64) -> Result<Connection, Refused> {
        if !valid_name(&p.squad) {
            return Err(refuse(BAD_PARAMS, "name a squad"));
        }
        let kept = match &p.id {
            Some(id) => Some(self.of(&p.squad, None).into_iter().find(|c| &c.id == id).ok_or_else(|| refuse(NOT_FOUND, format!("no connection {id}")))?),
            None => None,
        };
        let kind = kept.as_ref().map(|c| c.kind).or(p.kind).ok_or_else(|| refuse(BAD_PARAMS, "say which `kind` to connect"))?;
        if p.kind.is_some_and(|k| k != kind) {
            return Err(refuse(BAD_PARAMS, "a connection's kind can't change: disconnect it and connect the other"));
        }
        let adapter = adapters::adapter(kind).ok_or_else(|| refuse(UNKNOWN_KIND, format!("{} can't be connected yet", kind.as_str())))?;
        let spec = adapter.spec();
        if let Some(unknown) = p.settings.keys().find(|k| !spec.fields.iter().any(|f| &f.key == *k)) {
            return Err(refuse(BAD_SETTINGS, format!("{} has no field `{unknown}`", spec.label)));
        }
        let given = |key: &str| p.settings.get(key).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let mut settings = BTreeMap::new();
        let mut keys = BTreeMap::new();
        let old_keys = match &kept {
            Some(c) if !c.keys.is_empty() => self.lock_keys().get(&c.id).unwrap_or_default(),
            _ => BTreeMap::new(),
        };
        for field in &spec.fields {
            // An empty value for a setting of a kept connection clears it.
            let cleared = kept.is_some() && !field.secret && !field.required && p.settings.get(&field.key).is_some_and(|v| v.trim().is_empty());
            let value = given(&field.key).or_else(|| {
                if cleared {
                    None
                } else if field.secret {
                    old_keys.get(&field.key).cloned()
                } else {
                    kept.as_ref().and_then(|c| c.settings.get(&field.key).cloned())
                }
            });
            match value {
                Some(v) if field.secret => {
                    keys.insert(field.key.clone(), v);
                }
                Some(v) => {
                    settings.insert(field.key.clone(), v);
                }
                None if field.required => return Err(refuse(BAD_SETTINGS, format!("{} needs `{}`", spec.label, field.key))),
                None => {}
            }
        }
        let project = p.project.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        let squad = p.squad.clone();
        let connection = self.change(|all| {
            let id = match &kept {
                Some(c) => c.id.clone(),
                None => {
                    let next = all.iter().filter_map(|c| c.id.strip_prefix('m')?.parse::<u32>().ok()).max().unwrap_or(0) + 1;
                    format!("m{next}")
                }
            };
            self.lock_keys().set(&id, &keys).map_err(|e| refuse(KEYCHAIN, format!("the keys couldn't be kept in the Keychain: {}", redact(&e, &keys))))?;
            let connection = Connection {
                id: id.clone(),
                squad,
                project: project.or_else(|| kept.as_ref().and_then(|c| c.project.clone())),
                kind,
                settings,
                keys: keys.keys().cloned().collect(),
                connected_at: kept.as_ref().map_or(now, |c| c.connected_at),
                // New keys: the last fetch's error is no longer what holds.
                last: None,
                last_ok: kept.as_ref().and_then(|c| c.last_ok),
            };
            match all.iter_mut().find(|c| c.id == id) {
                Some(c) => *c = connection.clone(),
                None => all.push(connection.clone()),
            }
            Ok(connection)
        })?;
        crate::daemon_log!("equipe: metrics {}: {} connected for {}", connection.id, kind.as_str(), connection.squad);
        Ok(connection)
    }

    pub fn disconnect(&self, p: &DisconnectParams) -> Result<Disconnected, Refused> {
        let gone = self.change(|all| {
            let at = all.iter().position(|c| c.id == p.id && c.squad == p.squad).ok_or_else(|| refuse(NOT_FOUND, format!("no connection {}", p.id)))?;
            self.lock_keys().delete(&p.id).map_err(|e| refuse(KEYCHAIN, format!("the keys couldn't be taken out of the Keychain: {e}")))?;
            Ok(all.remove(at))
        })?;
        if p.forget {
            let _ = std::fs::remove_dir_all(snapshots_dir(&self.home, &gone.squad, &gone.id));
        }
        crate::daemon_log!("equipe: metrics {}: disconnected{}", gone.id, if p.forget { ", its numbers forgotten" } else { "" });
        Ok(Disconnected { id: gone.id })
    }

    /// `ask`, which touches the Keychain, on the blocking pool in its turn —
    /// never on a worker of the runtime. Asked on one, a Keychain that waited
    /// on the person held up every screen, and the timers and sockets with
    /// them, for good (the daemon of 2026-11-04). `Err` when it doesn't come
    /// back within [`KEYCHAIN_WAIT`]: it is left to come back on its own, its
    /// turn held till then, so no other ask piles up behind it.
    async fn ask_keychain<T: Send + 'static>(&self, ask: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
        let deadline = tokio::time::Instant::now() + self.keychain_wait;
        let silent = || format!("the Keychain didn't answer in {}s: it may be waiting for you to allow equipe on this Mac", self.keychain_wait.as_secs());
        let turn = match tokio::time::timeout_at(deadline, self.keychain_turn.clone().acquire_owned()).await {
            Ok(Ok(turn)) => turn,
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => return Err(silent()),
        };
        let asked = tokio::task::spawn_blocking(move || {
            let _turn = turn;
            ask()
        });
        match tokio::time::timeout_at(deadline, asked).await {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err(silent()),
        }
    }

    /// A change that keeps or drops keys, asked as [`Metrics::ask_keychain`].
    async fn keyed<T: Send + 'static>(self: &Arc<Self>, change: impl FnOnce(&Self) -> Result<T, Refused> + Send + 'static) -> Result<T, Refused> {
        let me = self.clone();
        self.ask_keychain(move || change(&me)).await.unwrap_or_else(|e| Err(refuse(KEYCHAIN, e)))
    }

    fn lock_keys(&self) -> std::sync::MutexGuard<'_, Box<dyn Keys>> {
        self.keys.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Fetch the squad's connections (or one) now; answers once all are done.
    pub async fn refresh(&self, p: &RefreshParams, now: u64, today: i64) -> Result<Sources, Refused> {
        let mut chosen = self.of(&p.squad, p.project.as_deref());
        if let Some(id) = &p.id {
            chosen.retain(|c| &c.id == id);
            if chosen.is_empty() {
                return Err(refuse(NOT_FOUND, format!("no connection {id}")));
            }
        }
        for c in &chosen {
            self.refresh_one(&c.id, now, today).await;
        }
        self.sources(&SourcesParams { squad: p.squad.clone(), project: p.project.clone() })
    }

    /// Fetch one connection and keep what it brought, or why it couldn't.
    /// One fetch per connection at a time: a second ask while one runs waits
    /// for that one's numbers rather than fetching again.
    pub async fn refresh_one(&self, id: &str, now: u64, today: i64) {
        let fetching = |id: &str| self.fetching.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains(id);
        if !self.fetching.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(id.to_owned()) {
            let waited = std::time::Instant::now();
            while fetching(id) && waited.elapsed() < FETCH_WAIT {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            return;
        }
        self.fetch(id, now, today).await;
        self.fetching.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(id);
    }

    async fn fetch(&self, id: &str, now: u64, today: i64) {
        let Some(connection) = connections(&self.home).into_iter().find(|c| c.id == id) else { return };
        let fail = |code, message: &str| FetchError { code, message: message.to_owned(), retry_at: None };
        let keys = {
            let (keys, id) = (self.keys.clone(), id.to_owned());
            self.ask_keychain(move || keys.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&id)).await.and_then(|got| got)
        };
        let got = match (adapters::adapter(connection.kind), keys) {
            (None, _) => Err(fail(ErrorCode::Other, "this kind of source can't be read yet")),
            (_, Err(e)) => Err(fail(ErrorCode::Keychain, &format!("the key couldn't be read from the Keychain ({e})"))),
            (Some(adapter), Ok(keys)) => {
                let spec = adapter.spec();
                let today_s = day_string(today);
                let days = if connection.last_ok.is_some() { LATER_DAYS } else { FIRST_DAYS };
                let fetch = adapters::Fetch { settings: &connection.settings, keys: &keys, today: &today_s, days, client: &self.client };
                let got = match tokio::time::timeout(FETCH_WAIT, adapter.fetch(fetch)).await {
                    Ok(got) => got,
                    Err(_) => Err(fail(ErrorCode::Network, "the service didn't answer in time")),
                };
                // Only metrics it says it reports, of days that are days, none after today.
                got.map(|points| {
                    points
                        .into_iter()
                        .filter(|p| spec.metrics.iter().any(|m| m.name == p.metric) && day_number(&p.day).is_some_and(|n| n <= today) && p.value.is_finite())
                        .collect::<Vec<_>>()
                })
                .map_err(|e| FetchError { message: redact(&e.message, &keys), ..e })
            }
        };
        let squad = connection.squad.clone();
        let outcome = self.change(|all| {
            // Disconnected meanwhile: nothing to keep.
            let Some(c) = all.iter_mut().find(|c| c.id == id) else { return Ok(None) };
            match &got {
                Ok(points) => {
                    let snapshot = Snapshot { schema: SCHEMA, connection: id.to_owned(), kind: c.kind, fetched_at: now, points: points.clone() };
                    let text = serde_json::to_string(&snapshot).map_err(|e| refuse(STORAGE, e.to_string()))?;
                    let file = snapshots_dir(&self.home, &squad, id).join(format!("{}.json", day_string(today)));
                    private_write(&file, &text).map_err(|e| refuse(STORAGE, e))?;
                    c.last = Some(Fetched { at: now, error: None });
                    c.last_ok = Some(now);
                }
                Err(e) => c.last = Some(Fetched { at: now, error: Some(e.clone()) }),
            }
            Ok(Some(()))
        });
        match (&got, outcome) {
            (Ok(points), Ok(Some(()))) => crate::daemon_log!("equipe: metrics {id}: {} points kept", points.len()),
            (Err(e), Ok(Some(()))) => crate::daemon_log!("equipe: metrics {id}: the fetch failed ({:?})", e.code),
            (_, Err(e)) => crate::daemon_log!("equipe: metrics {id}: couldn't be kept: {}", e.message),
            (_, Ok(None)) => {}
        }
    }

    /// The connections due a fetch now.
    pub fn due(&self, now: u64, clock: &impl Clock) -> Vec<String> {
        connections(&self.home).into_iter().filter(|c| due(c, now, clock)).map(|c| c.id).collect()
    }

    /// Each connection once a day from [`DAILY_HOUR`], and after a failure
    /// again when the service said or an hour later. Runs for good.
    pub async fn daily(self: Arc<Self>) {
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let today = day_of(&crate::routines::Local, now);
            for id in self.due(now, &crate::routines::Local) {
                self.refresh_one(&id, now, today).await;
            }
            tokio::time::sleep(Duration::from_secs(10 * 60)).await;
        }
    }
}

/// Whether `c` is due a fetch at `now`.
pub fn due(c: &Connection, now: u64, clock: &impl Clock) -> bool {
    let today = day_of(clock, now);
    if c.last_ok.is_some_and(|ok| day_of(clock, ok) == today) {
        return false;
    }
    // Never fetched: now, whatever the hour.
    if c.last.is_none() && c.last_ok.is_none() {
        return true;
    }
    if clock.parts(now).hour < DAILY_HOUR {
        return false;
    }
    match c.last.as_ref().and_then(|l| l.error.as_ref().map(|e| (l.at, e))) {
        Some((at, e)) => now >= e.retry_at.unwrap_or(at + RETRY),
        None => true,
    }
}

// ---- Reading ----

/// The period asked for: (from, to) and the one before, as day numbers.
fn period(days: Option<u32>, to: Option<&str>, today: i64) -> Result<(i64, i64, u32), Refused> {
    let days = days.unwrap_or(7);
    if !(1..=366).contains(&days) {
        return Err(refuse(BAD_PARAMS, "`days` is 1 to 366"));
    }
    let to = match to {
        Some(t) => day_number(t).ok_or_else(|| refuse(BAD_PARAMS, "`to` is a day, YYYY-MM-DD"))?,
        None => today,
    };
    Ok((to - i64::from(days) + 1, to, days))
}

/// A metric over a period: its value, series, breakdown, latest day known and
/// how many days are known.
type Over = (Option<f64>, Vec<Sample>, BTreeMap<String, f64>, Option<i64>, u32);

/// A metric's value over days `from..=to`, and its series cut by `bucket`.
fn over(def: &MetricDef, points: &BTreeMap<(String, i64), Point>, from: i64, to: i64, bucket: Bucket) -> Over {
    let day = |n: i64| points.get(&(def.name.clone(), n));
    let add = |values: &mut dyn Iterator<Item = (i64, f64)>| -> Option<f64> {
        let mut out: Option<(i64, f64)> = None;
        for (n, v) in values {
            out = Some(match (def.shape, out) {
                (Shape::Total, _) | (_, None) => (n, v),
                (_, Some((_, sum))) => (n, sum + v),
            });
        }
        out.map(|(_, v)| v)
    };
    let value = add(&mut (from..=to).filter_map(|n| day(n).map(|p| (n, p.value))));
    let mut series = Vec::new();
    let mut start = from;
    while start <= to {
        let end = match bucket {
            Bucket::Day => start,
            Bucket::Week => (monday(start) + 6).min(to),
        };
        series.push(Sample { day: day_string(start), value: add(&mut (start..=end).filter_map(|n| day(n).map(|p| (n, p.value)))) });
        start = end + 1;
    }
    let mut breakdown = BTreeMap::new();
    match def.shape {
        Shape::Breakdown => {
            for n in from..=to {
                for (label, v) in day(n).and_then(|p| p.breakdown.as_ref()).into_iter().flatten() {
                    *breakdown.entry(label.clone()).or_insert(0.0) += v;
                }
            }
        }
        // A total split by label (GitHub's referrers of the last 14 days): its last day's split.
        Shape::Total => {
            if let Some(split) = (from..=to).rev().find_map(day).and_then(|p| p.breakdown.clone()) {
                breakdown = split;
            }
        }
        Shape::Count | Shape::Rolling => {}
    }
    let known: Vec<i64> = (from..=to).filter(|n| day(*n).is_some()).collect();
    (value, series, breakdown, known.last().copied(), known.len() as u32)
}

/// A `rolling` metric over days `from..=to`: its last known day's value, the
/// value 7 days before that day, and 12 values 7 days apart ending there — the
/// same whatever the period's length or bucket. Nothing when no day of it was
/// ever fetched (a source without the definition: the screen says so).
fn rolling(c: &Connection, def: &MetricDef, points: &BTreeMap<(String, i64), Point>, from: i64, to: i64) -> Option<MetricValue> {
    let day = |n: i64| points.get(&(def.name.clone(), n)).map(|p| p.value);
    if !points.keys().any(|(m, _)| m == &def.name) {
        return None;
    }
    let known: Vec<i64> = (from..=to).filter(|n| day(*n).is_some()).collect();
    let end = known.last().copied().unwrap_or(to);
    let value = day(end);
    let previous = day(end - WINDOW);
    let series = (0..ROLLING_POINTS).rev().map(|k| end - k * WINDOW).map(|n| Sample { day: day_string(n), value: day(n) }).collect();
    Some(MetricValue {
        connection: c.id.clone(),
        kind: c.kind,
        metric: def.name.clone(),
        label: def.label.clone(),
        unit: def.unit.clone(),
        shape: def.shape,
        value,
        previous,
        change: value.zip(previous).map(|(v, w)| round1(v - w)),
        change_pct: value.zip(previous).filter(|(_, w)| *w != 0.0).map(|(v, w)| round1((v - w) / w * 100.0)),
        as_of: known.last().copied().map(day_string),
        days_known: known.len() as u32,
        series,
        previous_series: Vec::new(),
        breakdown: Vec::new(),
    })
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// `metrics.query`, from the snapshots on disk alone: no key is read.
pub fn query(home: &Path, p: &QueryParams, today: i64) -> Result<QueryResult, Refused> {
    if !valid_name(&p.squad) {
        return Err(refuse(BAD_PARAMS, "name a squad"));
    }
    let (from, to, days) = period(p.days, p.to.as_deref(), today)?;
    let (prev_from, prev_to) = (from - i64::from(days), from - 1);
    let chosen: Vec<Connection> = connections(home)
        .into_iter()
        .filter(|c| c.squad == p.squad && p.project.as_deref().is_none_or(|x| c.project.as_deref() == Some(x)))
        .filter(|c| p.connections.is_empty() || p.connections.contains(&c.id))
        .collect();
    let mut values = Vec::new();
    for c in &chosen {
        let Some(adapter) = adapters::adapter(c.kind) else { continue };
        let points = merged(&snapshots(home, &c.squad, &c.id));
        for def in adapter.spec().metrics.iter().filter(|d| p.metrics.is_empty() || p.metrics.contains(&d.name)) {
            if def.shape == Shape::Rolling {
                values.extend(rolling(c, def, &points, from, to));
                continue;
            }
            let (value, series, now_split, as_of, days_known) = over(def, &points, from, to, p.bucket);
            let (previous, previous_series, then_split, _, _) = over(def, &points, prev_from, prev_to, p.bucket);
            let change = value.zip(previous).map(|(v, w)| round1(v - w));
            let change_pct = value.zip(previous).filter(|(_, w)| *w != 0.0).map(|(v, w)| round1((v - w) / w * 100.0));
            let mut breakdown: Vec<Share> =
                now_split.into_iter().map(|(label, value)| Share { previous: then_split.get(&label).copied(), label, value }).collect();
            breakdown.sort_by(|a, b| b.value.total_cmp(&a.value).then_with(|| a.label.cmp(&b.label)));
            values.push(MetricValue {
                connection: c.id.clone(),
                kind: c.kind,
                metric: def.name.clone(),
                label: def.label.clone(),
                unit: def.unit.clone(),
                shape: def.shape,
                value,
                previous,
                change,
                change_pct,
                as_of: as_of.map(day_string),
                days_known,
                series,
                previous_series,
                breakdown,
            });
        }
    }
    Ok(QueryResult {
        from: day_string(from),
        to: day_string(to),
        previous_from: day_string(prev_from),
        previous_to: day_string(prev_to),
        days,
        bucket: p.bucket,
        values,
        connections: chosen,
    })
}

/// `metrics.summary`: the last 7 days beside the 7 before, the first
/// [`TILES`] numbers that aren't breakdowns — the North Star first.
pub fn summary(home: &Path, p: &SummaryParams, today: i64) -> Result<Summary, Refused> {
    let q = QueryParams { squad: p.squad.clone(), project: p.project.clone(), days: Some(7), ..QueryParams::default() };
    let result = query(home, &q, today)?;
    let error_of = |id: &str| result.connections.iter().find(|c| c.id == id).and_then(|c| c.last.as_ref()?.error.clone());
    let mut numbers: Vec<&MetricValue> = result.values.iter().filter(|v| v.shape != Shape::Breakdown).collect();
    numbers.sort_by_key(|v| v.metric != NORTH_STAR);
    let tiles = numbers
        .into_iter()
        .take(TILES)
        .map(|v| Tile {
            connection: v.connection.clone(),
            kind: v.kind,
            metric: v.metric.clone(),
            label: v.label.clone(),
            unit: v.unit.clone(),
            value: v.value,
            previous: v.previous,
            change_pct: v.change_pct,
            as_of: v.as_of.clone(),
            error: error_of(&v.connection),
        })
        .collect();
    Ok(Summary {
        squad: p.squad.clone(),
        project: p.project.clone(),
        from: result.from,
        to: result.to,
        tiles,
        updated_at: result.connections.iter().filter_map(|c| c.last_ok).max(),
    })
}

/// How a kind of source is named.
pub fn kind_label(kind: SourceKind) -> String {
    adapters::adapter(kind).map_or_else(|| kind.as_str().to_owned(), |a| a.spec().label)
}

#[cfg(test)]
mod tests;
