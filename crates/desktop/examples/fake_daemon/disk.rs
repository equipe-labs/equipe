//! `FAKE_DISK=low|critical|full`: the disk runs short, as on the day it
//! filled up — finished tickets' worktrees holding Rust `target/`s, members'
//! build folders, an agent's worktree in a project, Xcode's DerivedData. The
//! snapshot carries the level, `disk_scan` answers with the look, and
//! `disk_clean` gives back what was picked (only what is safe) and looks again.
//! `FAKE_DISK=critical` also leaves a member's build failing for want of room
//! last in frontend's conversation, with its news line.

use std::path::PathBuf;
use std::time::Duration;

use equipe_core::disk::{DiskCleaned, DiskGroup, DiskItem, DiskKind, DiskLevel, DiskReport, DiskStatus, DiskWhy};
use equipe_core::protocol::{Command, Event};
use tokio::sync::broadcast;

const MB: u64 = 1_000_000;
const TOTAL: u64 = 494_384_795_648;

pub fn mode() -> Option<DiskLevel> {
    match std::env::var("FAKE_DISK").ok()?.as_str() {
        "low" => Some(DiskLevel::Low),
        "critical" => Some(DiskLevel::Critical),
        "full" => Some(DiskLevel::Full),
        _ => None,
    }
}

/// What the disk has free now: less as it's set worse, more as it's cleaned.
fn free() -> &'static std::sync::Mutex<u64> {
    static FREE: std::sync::OnceLock<std::sync::Mutex<u64>> = std::sync::OnceLock::new();
    FREE.get_or_init(|| {
        std::sync::Mutex::new(match mode() {
            Some(DiskLevel::Low) => 11_400 * MB,
            Some(DiskLevel::Full) => 612 * MB,
            _ => 4_200 * MB,
        })
    })
}

/// The ids cleaned so far: gone from the next look.
fn gone() -> &'static std::sync::Mutex<Vec<String>> {
    static GONE: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
    GONE.get_or_init(Default::default)
}

fn level(free: u64) -> DiskLevel {
    equipe_core::disk::next_level(DiskLevel::Ok, free, TOTAL, false)
}

pub fn status() -> Option<DiskStatus> {
    mode()?;
    let free = *free().lock().unwrap();
    let status = DiskStatus { level: level(free), free, total: TOTAL, path: home().join(".equipe"), container_free: None };
    status.level.is_warning().then_some(status)
}

fn home() -> PathBuf {
    PathBuf::from("/Users/abel")
}

#[allow(clippy::too_many_arguments)]
fn item(group: DiskGroup, kind: DiskKind, label: &str, path: &str, bytes: u64, safe: bool, why: DiskWhy, ticket: Option<u32>, command: Option<&str>) -> DiskItem {
    let path = PathBuf::from(path.replace('~', "/Users/abel"));
    let kind_word = serde_json::to_value(kind).unwrap().as_str().unwrap().to_owned();
    DiskItem {
        id: format!("{kind_word}:{}", path.display()),
        group,
        kind,
        label: label.into(),
        path,
        bytes,
        partial: false,
        safe,
        why,
        squad: Some("demo".into()),
        ticket,
        command: command.map(str::to_owned),
    }
}

fn items() -> Vec<DiskItem> {
    use DiskGroup as G;
    use DiskKind as K;
    use DiskWhy as W;
    let wt = "~/.equipe/worktrees";
    let all = vec![
        item(G::Worktrees, K::WorktreeBuild, "T-24 · equipe · target/", &format!("{wt}/equipe/t24-6ad1cb4a/target"), 9_840 * MB, true, W::Merged, Some(24), None),
        item(G::Worktrees, K::WorktreeBuild, "T-56 · equipe · target/", &format!("{wt}/equipe/t56-6acee978/target"), 6_120 * MB, true, W::Cancelled, Some(56), None),
        item(G::Worktrees, K::WorktreeBuild, "T-13 · site · node_modules/", &format!("{wt}/site/t13-6ace0004/node_modules"), 1_310 * MB, true, W::Merged, Some(13), None),
        item(G::Worktrees, K::Worktree, "T-24 · equipe", &format!("{wt}/equipe/t24-6ad1cb4a"), 182 * MB, true, W::Merged, Some(24), None),
        item(G::Worktrees, K::Worktree, "T-56 · equipe", &format!("{wt}/equipe/t56-6acee978"), 176 * MB, true, W::Cancelled, Some(56), None),
        item(G::Worktrees, K::Worktree, "T-13 · site", &format!("{wt}/site/t13-6ace0004"), 64 * MB, true, W::Merged, Some(13), None),
        item(G::Worktrees, K::WorktreeBuild, "T-107 · equipe · target/", &format!("{wt}/equipe/t107-6acf6d96/target"), 4_480 * MB, false, W::OpenTicket, Some(107), None),
        item(G::Worktrees, K::Worktree, "T-112 · equipe", &format!("{wt}/equipe/t112-6ad1cec2"), 240 * MB, false, W::Uncommitted, Some(112), None),
        item(G::Builds, K::BuildCache, "backend · demo", "~/.equipe/cache/build/demo/backend", 7_350 * MB, true, W::Stale, None, None),
        item(G::Builds, K::BuildCache, "frontend · demo", "~/.equipe/cache/build/demo/frontend", 5_210 * MB, false, W::InUse, None, None),
        item(G::Builds, K::BuildCache, "pm · demo", "~/.equipe/cache/build/demo/pm", 1_940 * MB, true, W::Rebuildable, None, None),
        item(G::Equipe, K::Log, "daemon.log", "~/.equipe/daemon.log", 86 * MB, true, W::OldLog, None, None),
        item(G::Equipe, K::Browser, "browser/", "~/.equipe/browser", 1_120 * MB, false, W::Needed, None, None),
        item(G::Equipe, K::Uploads, "uploads/", "~/.equipe/uploads", 214 * MB, false, W::Needed, None, None),
        item(G::Equipe, K::Journals, "squads/*/squad.db", "~/.equipe/squads", 38 * MB, false, W::Needed, None, None),
        item(G::Projects, K::ProjectBuild, "equipe · .claude/worktrees/agent-a24f/target/", "~/code/equipe/.claude/worktrees/agent-a24f/target", 14_700 * MB, false, W::NotEquipes, None, Some("rm -rf ~/code/equipe/.claude/worktrees/agent-a24f/target")),
        item(G::Projects, K::ProjectBuild, "equipe · target/", "~/code/equipe/target", 11_900 * MB, false, W::NotEquipes, None, Some("rm -rf ~/code/equipe/target")),
        item(G::Projects, K::ProjectBuild, "site · node_modules/", "~/code/site/node_modules", 980 * MB, false, W::NotEquipes, None, Some("rm -rf ~/code/site/node_modules")),
        item(G::Caches, K::DerivedData, "~/Library/Developer/Xcode/DerivedData", "~/Library/Developer/Xcode/DerivedData", 18_300 * MB, false, W::NotEquipes, None, Some("rm -rf ~/Library/Developer/Xcode/DerivedData")),
        item(G::Caches, K::Cargo, "~/.cargo/registry", "~/.cargo/registry", 3_400 * MB, false, W::NotEquipes, None, Some("rm -rf ~/.cargo/registry/cache ~/.cargo/registry/src")),
        item(G::Caches, K::Npm, "~/.npm/_cacache", "~/.npm/_cacache", 2_150 * MB, false, W::NotEquipes, None, Some("npm cache clean --force")),
        item(G::Caches, K::LibraryCache, "~/Library/Caches/com.spotify.client", "~/Library/Caches/com.spotify.client", 1_620 * MB, false, W::NotEquipes, None, None),
    ];
    let gone = gone().lock().unwrap();
    all.into_iter().filter(|i| !gone.contains(&i.id)).collect()
}

fn report() -> DiskReport {
    let free = *free().lock().unwrap();
    let mut items = items();
    items.sort_by(|a, b| b.safe.cmp(&a.safe).then(b.bytes.cmp(&a.bytes)));
    DiskReport {
        status: DiskStatus { level: level(free), free, total: TOTAL, path: home().join(".equipe"), container_free: Some(free - 120 * MB) },
        items,
        took_ms: 1_840,
        partial: false,
        at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
    }
}

/// A look, and a clean: as the daemon answers them. `true` when it was one.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    match command {
        Command::DiskScan => {
            let events = events.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(900)).await;
                let _ = events.send(Event::DiskReport { report: report() });
            });
            true
        }
        Command::DiskClean { ids } => {
            let ids = ids.clone();
            let events = events.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(1400)).await;
                let safe: Vec<DiskItem> = items().into_iter().filter(|i| i.safe && ids.contains(&i.id)).collect();
                let freed: u64 = safe.iter().map(|i| i.bytes).sum();
                let before = level(*free().lock().unwrap());
                *free().lock().unwrap() += freed;
                gone().lock().unwrap().extend(safe.iter().map(|i| i.id.clone()));
                let now = *free().lock().unwrap();
                let status = DiskStatus { level: level(now), free: now, total: TOTAL, path: home().join(".equipe"), container_free: None };
                let cleaned = DiskCleaned { freed, removed: safe.iter().map(|i| i.id.clone()).collect(), skipped: Vec::new(), status: status.clone() };
                let _ = events.send(Event::DiskCleaned { cleaned });
                if status.level != before {
                    let _ = events.send(Event::DiskLevel { status });
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
                let _ = events.send(Event::DiskReport { report: report() });
            });
            true
        }
        _ => false,
    }
}

/// `FAKE_DISK=critical|full`: frontend's build failed for want of room, as its conversation ends.
pub fn failed_build() -> Vec<equipe_core::protocol::Entry> {
    use equipe_core::protocol::{Entry, ToolOutcome};
    if !matches!(mode(), Some(DiskLevel::Critical | DiskLevel::Full)) {
        return Vec::new();
    }
    let output = "   Compiling gpui v0.2.0\nerror: failed to write /Users/abel/.equipe/cache/build/demo/frontend/debug/deps/libgpui.rlib: No space left on device (os error 28)\n\nerror: could not compile `gpui` (lib) due to 1 previous error";
    let outcome = ToolOutcome { ok: false, exit: Some(101), ms: Some(48_200), output: output.into(), skipped: 0, diff: None };
    vec![
        Entry::Tool { tool: "Bash".into(), target: Some("cargo build -p equipe-desktop".into()), outcome: Some(Box::new(outcome)) },
        Entry::Note { text: equipe_core::i18n::core(equipe_core::i18n::Lang::Ko).fill("news.disk_build_failed", &[("name", "frontend".to_owned())]) },
    ]
}
