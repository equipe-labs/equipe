//! equipe's secrets in the macOS Keychain, asked about as rarely as macOS allows.
//!
//! Every secret equipe keeps on a Mac — metrics connections' keys
//! (`equipe.metrics`), logins' passwords, seeds and command-line credentials
//! (`equipe.vault`) — lives in **one** Keychain item (service [`SERVICE`],
//! account [`ACCOUNT`]): a JSON object of `"<service>:<account>"` → value.
//!
//! Why one item: macOS keeps a separate access list per item. Each item an
//! older build of equipe made asks the person again ("equipe wants to use
//! your confidential information…") when a build with another signature reads
//! it — with one item per connection and per login, an update meant one
//! dialog per item, all at once. Now it is one dialog at most, and with a
//! stably signed build (Developer ID; see docs/RELEASING.md) "Always Allow"
//! holds across updates.
//!
//! How the asking is kept down:
//! - the item is read once per process and kept in memory ([`Store`]);
//!   the daemon warms it on a blocking thread at start ([`warm`]);
//! - only one read or write that may prompt runs at a time in a process
//!   ([`prompts::begin`]) and across processes (a lock file), and a caller
//!   arriving meanwhile is told to wait instead of opening a second dialog;
//! - after the person says no, background work doesn't ask again for a day
//!   ([`prompts::COOL_OFF`]); what the person does themselves may ask at once;
//! - a background read notes in `prompts.json` that it asked before it reads;
//!   if the process dies with the dialog still up (a daemon restart), the note
//!   stays and background work doesn't open another dialog on top of it for a
//!   day ([`UNANSWERED`]);
//! - items an older equipe kept one per secret are moved into the one item
//!   when first needed, and removed.
//!
//! Nothing that isn't secret is kept here: connection settings, login names,
//! the telemetry id and every other state stay in files under equipe's home.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub use equipe_desktop_control::prompts;
use prompts::{Answer, Kind};

/// The one item's service and account.
pub const SERVICE: &str = "equipe";
pub const ACCOUNT: &str = "secrets";
/// The identity the Keychain's answers are remembered for.
const IDENTITY: &str = "equipe";

/// What the person is told while another process or thread waits on the
/// Keychain's dialog.
pub const WAITING: &str = "the Keychain is asking you about equipe on this Mac: answer that dialog (\"Always Allow\" keeps it from asking again), then try again";
/// What background work is told for a day after the person said no.
pub const REFUSED: &str = "the Keychain was refused to equipe on this Mac: it asks again when you next save or use a key yourself, or tomorrow";
/// What background work is told for a day after an earlier process asked and
/// never saw an answer (its dialog may still be on screen).
pub const UNANSWERED: &str = "the Keychain dialog for equipe was left unanswered: it asks again when you next save or use a key yourself, or tomorrow";

/// How the Keychain answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The person said no, cancelled, or nobody was there to ask.
    Denied,
    NotFound,
    Other(String),
}

/// The Keychain itself, or (in tests) a fake.
pub trait Backend: Send + Sync {
    /// The item's data; `Err(NotFound)` when there is no such item.
    fn read(&self, service: &str, account: &str) -> Result<Vec<u8>, Error>;
    fn write(&self, service: &str, account: &str, data: &[u8]) -> Result<(), Error>;
    /// Gone already is fine.
    fn delete(&self, service: &str, account: &str) -> Result<(), Error>;
}

/// macOS's login Keychain.
#[cfg(target_os = "macos")]
pub struct MacKeychain;

#[cfg(target_os = "macos")]
impl MacKeychain {
    fn error(e: security_framework::base::Error) -> Error {
        match e.code() {
            // errSecItemNotFound
            -25300 => Error::NotFound,
            // errSecAuthFailed, errSecUserCanceled, errSecInteractionNotAllowed
            -25293 | -128 | -25308 => Error::Denied,
            _ => Error::Other(e.to_string()),
        }
    }
}

#[cfg(target_os = "macos")]
impl Backend for MacKeychain {
    fn read(&self, service: &str, account: &str) -> Result<Vec<u8>, Error> {
        security_framework::passwords::get_generic_password(service, account).map_err(Self::error)
    }

    fn write(&self, service: &str, account: &str, data: &[u8]) -> Result<(), Error> {
        security_framework::passwords::set_generic_password(service, account, data).map_err(Self::error)
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), Error> {
        match security_framework::passwords::delete_generic_password(service, account).map_err(Self::error) {
            Ok(()) | Err(Error::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

fn key(service: &str, account: &str) -> String {
    format!("{service}:{account}")
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// How long a caller waits for another's turn before it is told [`WAITING`]
/// (milliseconds): a moment in the daemon, whose threads (its loop among them)
/// mustn't hang on a dialog that waits for the person; a command run for the
/// person may wait longer ([`patient`]).
const WAIT_MS: u64 = 250;

/// This process may wait up to a minute for another one's Keychain dialog:
/// a command the person (or the AWS CLI) runs, not the daemon.
pub fn patient() {
    #[cfg(target_os = "macos")]
    global().wait_ms.store(60_000, Ordering::SeqCst);
}

/// The secrets of one Keychain item, read once and kept in memory.
pub struct Store {
    backend: Box<dyn Backend>,
    home: PathBuf,
    /// The item's contents once read; `None` until then.
    cache: Mutex<Option<BTreeMap<String, String>>>,
    /// The process's one Keychain turn goes through the prompt budget
    /// ([`prompts::begin`]); a store of its own (tests) has its own.
    budget: bool,
    own_turn: AtomicBool,
    wait_ms: AtomicU64,
}

/// A store's turn, released when dropped.
enum Turn<'a> {
    Budget(#[allow(dead_code)] prompts::Outstanding),
    Own(&'a AtomicBool),
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        if let Turn::Own(flag) = self {
            flag.store(false, Ordering::SeqCst);
        }
    }
}

impl Store {
    pub fn new(home: &Path, backend: Box<dyn Backend>) -> Self {
        Self { backend, home: home.to_owned(), cache: Mutex::new(None), budget: false, own_turn: AtomicBool::new(false), wait_ms: AtomicU64::new(WAIT_MS) }
    }

    fn try_turn(&self) -> Option<Turn<'_>> {
        if self.budget {
            prompts::begin(Kind::Keychain).map(Turn::Budget)
        } else {
            self.own_turn.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).ok().map(|_| Turn::Own(&self.own_turn))
        }
    }

    fn cached(&self) -> std::sync::MutexGuard<'_, Option<BTreeMap<String, String>>> {
        self.cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Hold the Keychain for a call that may show a dialog: this process's
    /// turn and the lock other equipe processes take. `Err(WAITING)` while
    /// another holds it longer than this process waits.
    fn turn(&self) -> Result<(Turn<'_>, Option<std::fs::File>), String> {
        let deadline = Instant::now() + Duration::from_millis(self.wait_ms.load(Ordering::SeqCst));
        let outstanding = loop {
            if let Some(turn) = self.try_turn() {
                break turn;
            }
            if Instant::now() >= deadline {
                return Err(WAITING.into());
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let lock = {
            use fs2::FileExt as _;
            let _ = std::fs::create_dir_all(&self.home);
            let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(self.home.join("keychain.lock")).ok();
            if let Some(file) = &file {
                loop {
                    if file.try_lock_exclusive().is_ok() {
                        break;
                    }
                    if Instant::now() >= deadline {
                        return Err(WAITING.into());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            file
        };
        Ok((outstanding, lock))
    }

    /// Why background work holds back, after the person's recent no or a
    /// dialog an earlier process left unanswered. Never for the person's own
    /// calls, nor after a grant.
    fn holding_back(&self, interactive: bool) -> Option<&'static str> {
        if interactive {
            return None;
        }
        let r = prompts::recorded(&self.home, Kind::Keychain)?;
        if r.identity != IDENTITY || unix_now().saturating_sub(r.at) >= prompts::COOL_OFF {
            return None;
        }
        match r.answer {
            Answer::Denied => Some(REFUSED),
            Answer::Asked => Some(UNANSWERED),
            Answer::Granted => None,
        }
    }

    /// Read an item that may show a dialog. Background reads note that they
    /// asked first, so a process that dies with the dialog up leaves a trace;
    /// whatever comes back (but a no, which [`Self::denied`] records) means no
    /// dialog is waiting any more.
    fn read(&self, service: &str, account: &str, interactive: bool) -> Result<Vec<u8>, Error> {
        if !interactive {
            prompts::record(&self.home, Kind::Keychain, IDENTITY, Answer::Asked, unix_now());
        }
        let result = self.backend.read(service, account);
        if !matches!(result, Err(Error::Denied)) {
            prompts::record(&self.home, Kind::Keychain, IDENTITY, Answer::Granted, unix_now());
        }
        result
    }

    fn denied(&self) -> String {
        prompts::record(&self.home, Kind::Keychain, IDENTITY, Answer::Denied, unix_now());
        REFUSED.into()
    }

    /// The item's contents, read now if they haven't been in this process.
    fn load(&self, interactive: bool) -> Result<BTreeMap<String, String>, String> {
        if let Some(all) = self.cached().clone() {
            return Ok(all);
        }
        if let Some(why) = self.holding_back(interactive) {
            return Err(why.into());
        }
        let _turn = self.turn()?;
        // Read by whoever held the turn before.
        if let Some(all) = self.cached().clone() {
            return Ok(all);
        }
        let all = match self.read(SERVICE, ACCOUNT, interactive) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "equipe's Keychain item isn't readable".to_owned())?,
            Err(Error::NotFound) => BTreeMap::new(),
            Err(Error::Denied) => return Err(self.denied()),
            Err(Error::Other(e)) => return Err(e),
        };
        *self.cached() = Some(all.clone());
        Ok(all)
    }

    /// Write `all` as the item, and keep it as what is cached.
    fn keep(&self, all: BTreeMap<String, String>) -> Result<(), String> {
        let text = serde_json::to_vec(&all).map_err(|e| e.to_string())?;
        let _turn = self.turn()?;
        match self.backend.write(SERVICE, ACCOUNT, &text) {
            Ok(()) => {
                *self.cached() = Some(all);
                Ok(())
            }
            Err(Error::Denied) => Err(self.denied()),
            Err(Error::NotFound) => Err("equipe's Keychain item couldn't be written".into()),
            Err(Error::Other(e)) => Err(e),
        }
    }

    /// The secret kept under `service`/`account`. `interactive`: something
    /// the person does now, which may ask even after a recent no.
    pub fn get(&self, service: &str, account: &str, interactive: bool) -> Result<String, String> {
        let mut all = self.load(interactive)?;
        if let Some(value) = all.get(&key(service, account)) {
            return Ok(value.clone());
        }
        // Kept by an older equipe as an item of its own: moved into the one.
        if let Some(why) = self.holding_back(interactive) {
            return Err(why.into());
        }
        let legacy = {
            let _turn = self.turn()?;
            self.read(service, account, interactive)
        };
        match legacy {
            Ok(bytes) => {
                let value = String::from_utf8(bytes).map_err(|_| "the kept secret isn't text".to_owned())?;
                all.insert(key(service, account), value.clone());
                if self.keep(all).is_ok() {
                    let _turn = self.turn();
                    let _ = self.backend.delete(service, account);
                }
                Ok(value)
            }
            Err(Error::NotFound) => Err("nothing kept".into()),
            Err(Error::Denied) => Err(self.denied()),
            Err(Error::Other(e)) => Err(e),
        }
    }

    /// Keep `value` under `service`/`account`: what the person does.
    pub fn set(&self, service: &str, account: &str, value: &str) -> Result<(), String> {
        let mut all = self.load(true)?;
        all.insert(key(service, account), value.to_owned());
        self.keep(all)?;
        // An older equipe's own item for it, if any, would only go stale.
        let _turn = self.turn();
        let _ = self.backend.delete(service, account);
        Ok(())
    }

    /// Forget the secret under `service`/`account` (gone already is fine).
    pub fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        let mut all = self.load(true)?;
        if all.remove(&key(service, account)).is_some() {
            self.keep(all)?;
        }
        let _turn = self.turn()?;
        match self.backend.delete(service, account) {
            Ok(()) | Err(Error::NotFound) => Ok(()),
            Err(Error::Denied) => Err(self.denied()),
            Err(Error::Other(e)) => Err(e),
        }
    }
}

/// This Mac's Keychain, shared by everything in the process.
#[cfg(target_os = "macos")]
pub fn global() -> &'static Store {
    static STORE: std::sync::OnceLock<Store> = std::sync::OnceLock::new();
    STORE.get_or_init(|| Store { budget: true, ..Store::new(&equipe_providers::credentials::equipe_home(), Box::new(MacKeychain)) })
}

/// Read the item now, on a blocking thread: the daemon at start, so a
/// dialog (an update with a new signature) is shown once, early, and never
/// holds up a member's command or a screen.
pub fn warm() {
    #[cfg(target_os = "macos")]
    if std::env::var_os("EQUIPE_VAULT_IN_MEMORY").is_none() {
        let _ = std::thread::Builder::new().name("keychain-warm".into()).spawn(|| {
            let _ = global().load(false);
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    use super::*;

    #[derive(Default)]
    struct Fake {
        items: Mutex<BTreeMap<(String, String), Vec<u8>>>,
        reads: AtomicUsize,
        deny: std::sync::atomic::AtomicBool,
    }

    impl Backend for Arc<Fake> {
        fn read(&self, service: &str, account: &str) -> Result<Vec<u8>, Error> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.deny.load(Ordering::SeqCst) {
                return Err(Error::Denied);
            }
            self.items.lock().unwrap().get(&(service.into(), account.into())).cloned().ok_or(Error::NotFound)
        }
        fn write(&self, service: &str, account: &str, data: &[u8]) -> Result<(), Error> {
            self.items.lock().unwrap().insert((service.into(), account.into()), data.to_vec());
            Ok(())
        }
        fn delete(&self, service: &str, account: &str) -> Result<(), Error> {
            self.items.lock().unwrap().remove(&(service.into(), account.into()));
            Ok(())
        }
    }

    fn home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eq-keychain-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn every_secret_is_one_item_read_once_per_process() {
        let fake = Arc::new(Fake::default());
        let store = Store::new(&home("one"), Box::new(fake.clone()));
        store.set("equipe.metrics", "c1", "{\"token\":\"a\"}").unwrap();
        store.set("equipe.vault", "7", "hunter2").unwrap();
        assert_eq!(fake.items.lock().unwrap().len(), 1, "one Keychain item, so one dialog at most");
        let reads = fake.reads.load(Ordering::SeqCst);
        for _ in 0..5 {
            assert_eq!(store.get("equipe.vault", "7", false).unwrap(), "hunter2");
            assert_eq!(store.get("equipe.metrics", "c1", false).unwrap(), "{\"token\":\"a\"}");
        }
        assert_eq!(fake.reads.load(Ordering::SeqCst), reads, "kept in memory: the Keychain isn't asked again");
        store.delete("equipe.vault", "7").unwrap();
        assert!(store.get("equipe.vault", "7", false).is_err());
    }

    #[test]
    fn an_older_equipes_items_move_into_the_one() {
        let fake = Arc::new(Fake::default());
        fake.items.lock().unwrap().insert(("equipe.metrics".into(), "c1".into()), b"{\"k\":\"v\"}".to_vec());
        let store = Store::new(&home("legacy"), Box::new(fake.clone()));
        assert_eq!(store.get("equipe.metrics", "c1", false).unwrap(), "{\"k\":\"v\"}");
        let items = fake.items.lock().unwrap();
        assert!(!items.contains_key(&("equipe.metrics".into(), "c1".into())), "the old item is gone");
        let one: BTreeMap<String, String> = serde_json::from_slice(&items[&(SERVICE.into(), ACCOUNT.into())]).unwrap();
        assert_eq!(one["equipe.metrics:c1"], "{\"k\":\"v\"}");
    }

    #[test]
    fn after_a_no_background_work_waits_and_the_person_may_ask() {
        let fake = Arc::new(Fake::default());
        let h = home("no");
        let store = Store::new(&h, Box::new(fake.clone()));
        fake.deny.store(true, Ordering::SeqCst);
        assert_eq!(store.get("equipe.metrics", "c1", false).unwrap_err(), REFUSED);
        let reads = fake.reads.load(Ordering::SeqCst);
        assert_eq!(store.get("equipe.metrics", "c1", false).unwrap_err(), REFUSED);
        assert_eq!(fake.reads.load(Ordering::SeqCst), reads, "no second dialog for background work");
        fake.deny.store(false, Ordering::SeqCst);
        store.set("equipe.metrics", "c1", "x").unwrap();
        assert_eq!(store.get("equipe.metrics", "c1", false).unwrap(), "x");
        let _ = std::fs::remove_dir_all(&h);
    }

    fn asked(h: &Path) {
        prompts::record(h, Kind::Keychain, IDENTITY, Answer::Asked, unix_now());
    }

    #[test]
    fn a_dialog_an_earlier_process_left_up_is_not_doubled() {
        let fake = Arc::new(Fake::default());
        let h = home("orphan");
        asked(&h); // an earlier process died mid-dialog
        let store = Store::new(&h, Box::new(fake.clone()));
        assert_eq!(store.get("equipe.vault", "1", false).unwrap_err(), UNANSWERED);
        assert_eq!(store.load(false).unwrap_err(), UNANSWERED);
        assert_eq!(fake.reads.load(Ordering::SeqCst), 0, "no second dialog for background work");
        assert_eq!(store.get("equipe.vault", "1", true).unwrap_err(), "nothing kept", "the person's own call still asks");
        assert!(fake.reads.load(Ordering::SeqCst) > 0);
        assert_eq!(prompts::recorded(&h, Kind::Keychain).unwrap().answer, Answer::Granted);
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn a_background_read_leaves_a_grant_that_stops_nothing() {
        let fake = Arc::new(Fake::default());
        let h = home("granted");
        let store = Store::new(&h, Box::new(fake.clone()));
        assert_eq!(store.get("equipe.vault", "1", false).unwrap_err(), "nothing kept");
        assert_eq!(prompts::recorded(&h, Kind::Keychain).unwrap().answer, Answer::Granted);
        assert_eq!(store.get("equipe.vault", "2", false).unwrap_err(), "nothing kept", "later background reads still run");
        let fresh = Store::new(&h, Box::new(fake.clone()));
        let reads = fake.reads.load(Ordering::SeqCst);
        assert_eq!(fresh.get("equipe.vault", "1", false).unwrap_err(), "nothing kept");
        assert!(fake.reads.load(Ordering::SeqCst) > reads, "a new process reads again");
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn the_older_item_read_respects_the_hold_back() {
        let fake = Arc::new(Fake::default());
        let h = home("legacy-hold");
        fake.items.lock().unwrap().insert(("equipe.metrics".into(), "m1".into()), b"v".to_vec());
        let store = Store::new(&h, Box::new(fake.clone()));
        store.load(false).unwrap();
        let reads = fake.reads.load(Ordering::SeqCst);
        asked(&h); // another process is mid-dialog
        assert_eq!(store.get("equipe.metrics", "m1", false).unwrap_err(), UNANSWERED);
        assert_eq!(fake.reads.load(Ordering::SeqCst), reads, "the per-secret item isn't read behind a dialog");
        assert_eq!(store.get("equipe.metrics", "m1", true).unwrap(), "v");
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn a_dialog_already_up_is_never_doubled() {
        let fake = Arc::new(Fake::default());
        let h = home("busy");
        let store = Store::new(&h, Box::new(fake));
        store.wait_ms.store(100, Ordering::SeqCst);
        let held = store.try_turn().expect("free");
        assert_eq!(store.get("equipe.vault", "1", false).unwrap_err(), WAITING);
        drop(held);
        assert_eq!(store.get("equipe.vault", "1", false).unwrap_err(), "nothing kept", "free again: asked, and nothing there");
        let _ = std::fs::remove_dir_all(&h);
    }
}
