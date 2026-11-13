//! The prompt budget: every path that can make macOS ask the person something
//! (a Keychain password, Accessibility, Screen Recording, Local Network,
//! notifications, App Management) goes through here, so that
//!
//! - at most one prompt of each kind is outstanding in a process
//!   ([`begin`]): a second caller is told "waiting for the person" instead
//!   of stacking a second dialog on the first;
//! - the answer is remembered across processes and restarts
//!   (`<home>/prompts.json`, [`record`]): after a yes, equipe never asks
//!   again for the same identity unless the OS revoked it ([`may_ask`],
//!   [`observe`]); after a no or an unanswered prompt, background work waits
//!   [`COOL_OFF`] before asking again — something the person starts
//!   (`interactive`) may ask at once.
//!
//! It lives in this crate because it is the lowest one that talks to macOS's
//! permission system; `equipe-core` re-exports it for the Keychain.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

/// How long background work waits before asking again after a no, or a
/// prompt nobody answered (seconds).
pub const COOL_OFF: u64 = 24 * 60 * 60;

/// What macOS may ask the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Keychain,
    Accessibility,
    ScreenRecording,
    LocalNetwork,
    Notifications,
    AppManagement,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Keychain, Kind::Accessibility, Kind::ScreenRecording, Kind::LocalNetwork, Kind::Notifications, Kind::AppManagement];

    pub fn key(self) -> &'static str {
        match self {
            Kind::Keychain => "keychain",
            Kind::Accessibility => "accessibility",
            Kind::ScreenRecording => "screen_recording",
            Kind::LocalNetwork => "local_network",
            Kind::Notifications => "notifications",
            Kind::AppManagement => "app_management",
        }
    }

    fn flag(self) -> &'static AtomicBool {
        static FLAGS: [AtomicBool; 6] = [const { AtomicBool::new(false) }; 6];
        &FLAGS[self as usize]
    }
}

/// A prompt of one kind outstanding in this process, until dropped.
#[derive(Debug)]
pub struct Outstanding(Kind);

impl Drop for Outstanding {
    fn drop(&mut self) {
        self.0.flag().store(false, Ordering::SeqCst);
    }
}

/// The right to show a prompt of `kind` now, or `None` while one of that kind
/// is already up in this process.
pub fn begin(kind: Kind) -> Option<Outstanding> {
    kind.flag().compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).ok().map(|_| Outstanding(kind))
}

/// How the person answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Asked; no answer seen yet.
    Asked,
    Granted,
    Denied,
}

impl Answer {
    fn key(self) -> &'static str {
        match self {
            Answer::Asked => "asked",
            Answer::Granted => "granted",
            Answer::Denied => "denied",
        }
    }

    fn of(text: &str) -> Option<Self> {
        match text {
            "asked" => Some(Answer::Asked),
            "granted" => Some(Answer::Granted),
            "denied" => Some(Answer::Denied),
            _ => None,
        }
    }
}

/// What is remembered of one kind: for which identity (a code requirement, a
/// bundle, an item), the answer, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub identity: String,
    pub answer: Answer,
    pub at: u64,
}

fn file(home: &Path) -> PathBuf {
    home.join("prompts.json")
}

fn load(home: &Path) -> BTreeMap<String, Record> {
    let Ok(text) = std::fs::read_to_string(file(home)) else { return BTreeMap::new() };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) else { return BTreeMap::new() };
    map.into_iter()
        .filter_map(|(kind, v)| {
            let answer = Answer::of(v["answer"].as_str()?)?;
            Some((kind, Record { identity: v["identity"].as_str()?.to_owned(), answer, at: v["at"].as_u64().unwrap_or(0) }))
        })
        .collect()
}

fn save(home: &Path, all: &BTreeMap<String, Record>) {
    let map: serde_json::Map<String, Value> =
        all.iter().map(|(k, r)| (k.clone(), json!({ "identity": r.identity, "answer": r.answer.key(), "at": r.at }))).collect();
    if std::fs::create_dir_all(home).is_ok() {
        let tmp = file(home).with_extension("json.tmp");
        if std::fs::write(&tmp, Value::Object(map).to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, file(home));
        }
    }
}

/// What is remembered of `kind`.
pub fn recorded(home: &Path, kind: Kind) -> Option<Record> {
    load(home).remove(kind.key())
}

/// Remember `answer` for `kind` and `identity` at `now`.
pub fn record(home: &Path, kind: Kind, identity: &str, answer: Answer, now: u64) {
    let mut all = load(home);
    let next = Record { identity: identity.to_owned(), answer, at: now };
    if all.get(kind.key()) != Some(&next) {
        all.insert(kind.key().to_owned(), next);
        save(home, &all);
    }
}

/// Whether equipe may ask for `kind` for `identity` now: never again after a
/// yes for the same identity, and — for background work — not within
/// [`COOL_OFF`] of an unanswered prompt or a no.
pub fn may_ask(home: &Path, kind: Kind, identity: &str, interactive: bool, now: u64) -> bool {
    match recorded(home, kind) {
        None => true,
        Some(r) if r.identity != identity => true,
        Some(r) => match r.answer {
            Answer::Granted => false,
            Answer::Asked | Answer::Denied => interactive || now.saturating_sub(r.at) >= COOL_OFF,
        },
    }
}

/// What the OS says now about `kind` for `identity`: a grant is remembered; a
/// grant the OS no longer reports (revoked, or the identity's requirement no
/// longer matching) is forgotten, so one prompt may be shown again.
pub fn observe(home: &Path, kind: Kind, identity: &str, granted: bool, now: u64) {
    match (recorded(home, kind), granted) {
        (Some(r), true) if r.identity == identity && r.answer == Answer::Granted => {}
        (_, true) => record(home, kind, identity, Answer::Granted, now),
        (Some(r), false) if r.identity == identity && r.answer == Answer::Granted => {
            let mut all = load(home);
            all.remove(kind.key());
            save(home, &all);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eq-prompts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn one_prompt_of_a_kind_at_a_time() {
        let first = begin(Kind::Notifications).expect("nothing outstanding yet");
        assert!(begin(Kind::Notifications).is_none(), "a second dialog never stacks on the first");
        assert!(begin(Kind::LocalNetwork).is_some(), "another kind is its own");
        drop(first);
        assert!(begin(Kind::Notifications).is_some(), "free again once answered");
    }

    #[test]
    fn a_yes_is_never_asked_again_unless_the_os_takes_it_back() {
        let h = home("yes");
        assert!(may_ask(&h, Kind::Accessibility, "app", false, 100));
        record(&h, Kind::Accessibility, "app", Answer::Asked, 100);
        assert!(!may_ask(&h, Kind::Accessibility, "app", false, 200), "an unanswered prompt isn't shown again at once");
        assert!(may_ask(&h, Kind::Accessibility, "app", true, 200), "the person asking for it may");
        assert!(may_ask(&h, Kind::Accessibility, "app", false, 100 + COOL_OFF), "after the cool-off");
        observe(&h, Kind::Accessibility, "app", true, 300);
        assert!(!may_ask(&h, Kind::Accessibility, "app", true, 10 * COOL_OFF), "granted: never again");
        assert!(may_ask(&h, Kind::Accessibility, "bundle", false, 300), "another identity is another question");
        observe(&h, Kind::Accessibility, "app", false, 400);
        assert!(may_ask(&h, Kind::Accessibility, "app", false, 400), "revoked by the OS: one prompt again");
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn a_no_waits_for_background_work() {
        let h = home("no");
        record(&h, Kind::Keychain, "equipe", Answer::Denied, 1000);
        assert!(!may_ask(&h, Kind::Keychain, "equipe", false, 1000 + COOL_OFF - 1));
        assert!(may_ask(&h, Kind::Keychain, "equipe", true, 1001));
        assert_eq!(recorded(&h, Kind::Keychain).map(|r| r.answer), Some(Answer::Denied));
        let _ = std::fs::remove_dir_all(&h);
    }
}
