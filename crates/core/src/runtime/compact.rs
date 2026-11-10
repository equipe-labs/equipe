//! A member's conversation being compacted — summarized to make room — told
//! to every screen as it begins and ends ([`Event::Compacting`],
//! [`Event::Compacted`], [`Event::CompactFailed`]), and kept on the session
//! for a screen that attaches meanwhile.
//!
//! Three kinds: Claude Code's own (mid-turn, its context nearly full: its
//! `status` `compacting` lines), Codex's (its `contextCompaction` item), and
//! equipe's — a desk conversation started afresh with a summary of the last
//! one's end, because it grew full or cold (`auto`) or the person asked
//! (`/compact`, `manual`).
//!
//! How long one may take is a guess, not progress: the longest of the last
//! few of the same engine and model, kept in `compactions.json`, or a
//! cautious default before there are any.

use super::*;
use equipe_protocol::{CompactTrigger, CompactingInfo};

/// How many past compactions of an engine and model the guess looks at.
const KEPT: usize = 5;
/// The guess before any compaction of that engine and model: a long
/// conversation's summary takes Claude Code a minute or two.
const ENGINE_DEFAULT_MS: u64 = 120_000;
/// equipe's own — the last messages handed on — takes no model at all.
const EQUIPE_DEFAULT_MS: u64 = 2_000;
/// What [`ENGINE_DEFAULT_MS`]'s key is for equipe's own.
const EQUIPE_KEY: &str = "equipe";

/// Past compactions' times, by engine and model (`claude/claude-opus-5-5`), newest last.
#[derive(Default)]
pub(super) struct CompactTimes {
    loaded: bool,
    times: HashMap<String, Vec<u64>>,
}

/// Who compacts: the engine, mid-turn, or equipe, at a desk conversation's start.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum By {
    Engine,
    Equipe,
}

fn file(home: &Path) -> PathBuf {
    home.join("compactions.json")
}

impl Runtime {
    fn compact_key(&self, id: SessionId, by: By) -> String {
        match (by, self.sessions.get(&id)) {
            (By::Engine, Some(s)) => format!("{}/{}", engine_name(s.engine), s.model.as_deref().unwrap_or("default")),
            _ => EQUIPE_KEY.to_owned(),
        }
    }

    fn compact_times(&mut self) -> &mut HashMap<String, Vec<u64>> {
        if !self.compactions.loaded {
            self.compactions.loaded = true;
            if self.config.persist
                && let Ok(text) = std::fs::read_to_string(file(&self.config.home))
            {
                self.compactions.times = serde_json::from_str(&text).unwrap_or_default();
            }
        }
        &mut self.compactions.times
    }

    /// How long a compaction keyed `key` may take: the longest of the last few.
    fn compact_estimate(&mut self, key: &str) -> u64 {
        let default = if key == EQUIPE_KEY { EQUIPE_DEFAULT_MS } else { ENGINE_DEFAULT_MS };
        self.compact_times().get(key).and_then(|t| t.iter().max().copied()).unwrap_or(default)
    }

    fn remember_compaction(&mut self, key: String, took_ms: u64) {
        let times = self.compact_times().entry(key).or_default();
        times.push(took_ms);
        let extra = times.len().saturating_sub(KEPT);
        times.drain(..extra);
        if self.config.persist
            && let Ok(text) = serde_json::to_string_pretty(&self.compactions.times)
        {
            let _ = std::fs::write(file(&self.config.home), text);
        }
    }

    /// The member it is, and the project its conversation is about now.
    fn compact_who(&self, id: SessionId) -> (Option<String>, Option<String>) {
        let Some(s) = self.sessions.get(&id) else { return (None, None) };
        let project = match (&s.work, &s.lane) {
            (Some(work), _) => Some(board::project_name(&work.project)),
            (None, Some((_, project))) => project.clone(),
            (None, None) => s.thread.clone(),
        };
        (s.name().map(str::to_owned), project)
    }

    /// A compaction began. One already under way is not begun again.
    pub(super) fn compaction_began(&mut self, id: SessionId, by: By, trigger: CompactTrigger) {
        if self.sessions.get(&id).is_none_or(|s| s.compacting.is_some()) {
            return;
        }
        let key = self.compact_key(id, by);
        let estimate_ms = self.compact_estimate(&key);
        let started_at = now_ms();
        let info = CompactingInfo { trigger, started_at, estimate_ms };
        if let Some(s) = self.sessions.get_mut(&id) {
            s.compacting = Some((info, key, std::time::Instant::now()));
        }
        let (member, project) = self.compact_who(id);
        self.publish(Event::Compacting { session: id, member, project, trigger, started_at, estimate_ms });
    }

    /// The compaction under way ended: done, or failed (`Some(why)`).
    pub(super) fn compaction_ended(&mut self, id: SessionId, failed: Option<String>) {
        let Some((info, key, since)) = self.sessions.get_mut(&id).and_then(|s| s.compacting.take()) else {
            return;
        };
        let took_ms = u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX);
        let (member, project) = self.compact_who(id);
        let trigger = info.trigger;
        match failed {
            None => {
                self.remember_compaction(key, took_ms);
                self.publish(Event::Compacted { session: id, member, project, trigger, took_ms });
            }
            Some(message) => self.publish(Event::CompactFailed { session: id, member, project, trigger, message }),
        }
    }

    /// What an engine said of its compaction, as [`activity_event`] made it.
    pub(super) fn on_engine_compaction(&mut self, event: Event) {
        match event {
            Event::Compacting { session, trigger, .. } => self.compaction_began(session, By::Engine, trigger),
            Event::Compacted { session, .. } => self.compaction_ended(session, None),
            Event::CompactFailed { session, message, .. } => self.compaction_ended(session, Some(message)),
            _ => {}
        }
    }
}

/// Why a compaction a turn's end cut short failed.
pub(super) const CUT_SHORT: &str = "The turn ended before the compaction finished.";

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
}

fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "claude",
        Engine::Codex => "codex",
    }
}
