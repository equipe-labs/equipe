//! The disk equipe writes to: how much room is left, what takes it, and what
//! can be given back.
//!
//! The daemon looks at the free space every minute or two, off its core loop,
//! and tells the screens only when the [`DiskLevel`] changes
//! ([`crate::Event::DiskLevel`]). What uses the space is measured only when a
//! screen asks ([`crate::Command::DiskScan`] → [`crate::Event::DiskReport`]):
//! a bounded look at what equipe knows of — its members' worktrees and build
//! folders, its own data, the squad's project checkouts — and, to report only,
//! the Mac's usual big caches. Cleaning ([`crate::Command::DiskClean`]) acts
//! only on items of a fresh look of the daemon's own that are [`DiskItem::safe`]:
//! a screen names them by id, never by path.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{SquadName, TicketId};

/// How much room the disk has, from fine to none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskLevel {
    #[default]
    Ok,
    /// Under 15 GB free, or 10% of the disk, whichever is smaller.
    Low,
    /// Under 5 GB free: builds may fail. New build work waits.
    Critical,
    /// Under 1 GB free, or a write just failed for want of room.
    Full,
}

impl DiskLevel {
    /// Whether the person should hear about it: anything but fine.
    pub fn is_warning(self) -> bool {
        self != DiskLevel::Ok
    }

    /// Whether equipe holds back work that builds.
    pub fn holds_builds(self) -> bool {
        self >= DiskLevel::Critical
    }
}

/// The free space on the volume equipe writes to that has the least of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskStatus {
    pub level: DiskLevel,
    /// Bytes free for an ordinary user.
    pub free: u64,
    /// The volume's size, bytes.
    pub total: u64,
    /// A folder on that volume, as looked at (`~/.equipe`, a project).
    pub path: PathBuf,
    /// The APFS container's free space, when the Mac says (it can differ from
    /// `free` by what the system keeps back). Only with a report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_free: Option<u64>,
}

/// Where an item is, as the panel groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskGroup {
    /// Members' worktrees under `~/.equipe/worktrees`.
    Worktrees,
    /// Members' build folders under `~/.equipe/cache/build`.
    Builds,
    /// Build output in the squad's project checkouts.
    Projects,
    /// equipe's own data: logs, the browser, journals, uploads.
    Equipe,
    /// The Mac's usual big caches: reported, never touched.
    Caches,
}

/// What an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskKind {
    /// Build output in a member's worktree (`target/`, `node_modules/`, `.next/`…).
    WorktreeBuild,
    /// A member's worktree as a whole (its branch stays).
    Worktree,
    /// A member's build folder (`CARGO_TARGET_DIR`).
    BuildCache,
    /// Build output in a project checkout, or in another tool's worktree there.
    ProjectBuild,
    /// A log equipe writes.
    Log,
    /// The members' browser: its profile and cache.
    Browser,
    /// The squads' journals.
    Journals,
    /// Files the person attached.
    Uploads,
    /// Xcode's DerivedData.
    DerivedData,
    /// The Mac's simulators.
    Simulators,
    /// `~/Library/Caches`, one app's.
    LibraryCache,
    /// Cargo's registry and git checkouts.
    Cargo,
    /// npm's cache.
    Npm,
    /// pnpm's store.
    Pnpm,
    /// Yarn's cache.
    Yarn,
    /// Docker's disk image.
    Docker,
}

/// Why an item may, or may not, be given back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskWhy {
    /// Its ticket is done and in main.
    Merged,
    /// Its ticket was cancelled.
    Cancelled,
    /// No ticket keeps it any more.
    NoTicket,
    /// Its ticket is still open (in progress, waiting, in review): left alone.
    OpenTicket,
    /// Its ticket is done but not in main yet: left alone until it is.
    NotMerged,
    /// It has changes not committed: left alone, whatever its ticket.
    Uncommitted,
    /// A member works in it now.
    InUse,
    /// Rebuilt when wanted; nobody has built there for days.
    Stale,
    /// Rebuilt when wanted; its member's next build starts over.
    Rebuildable,
    /// A log past its size: its last part is kept.
    OldLog,
    /// equipe needs it: shown so the person knows.
    Needed,
    /// Not equipe's: the person clears it if they want, with `command`.
    NotEquipes,
}

/// One thing that takes space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskItem {
    /// How a screen names it to clean it: stable within one look.
    pub id: String,
    pub group: DiskGroup,
    pub kind: DiskKind,
    /// Short words for it: `T-24 · target/`, `frontend`, `daemon.log`.
    pub label: String,
    pub path: PathBuf,
    pub bytes: u64,
    /// The measuring stopped at its time budget: at least `bytes`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
    /// equipe may remove it, on the person's click.
    pub safe: bool,
    pub why: DiskWhy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub squad: Option<SquadName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    /// For what equipe doesn't remove: the command that clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// What uses the space, biggest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskReport {
    pub status: DiskStatus,
    pub items: Vec<DiskItem>,
    /// How long the look took, milliseconds.
    pub took_ms: u64,
    /// The look ran out of time: some sizes are at least what they say.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
    /// When it was made, Unix seconds.
    pub at: u64,
}

impl DiskReport {
    /// What equipe may give back, in all.
    pub fn reclaimable(&self) -> u64 {
        self.items.iter().filter(|i| i.safe).map(|i| i.bytes).sum()
    }
}

/// What a clean did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskCleaned {
    /// Bytes given back.
    pub freed: u64,
    /// What went, by item id.
    pub removed: Vec<String>,
    /// What was asked for and left, and why (it changed since the look: a
    /// change not committed, a member at work).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<(String, DiskWhy)>,
    /// The disk after it.
    pub status: DiskStatus,
}

/// `bytes` as people read it: `4.2 GB`, `880 MB`, `12 KB` (decimal, as macOS says).
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000. && unit < UNITS.len() - 1 {
        value /= 1000.;
        unit += 1;
    }
    match unit {
        0 => format!("{bytes} B"),
        _ if value < 10. => format!("{value:.1} {}", UNITS[unit]),
        _ => format!("{value:.0} {}", UNITS[unit]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_as_macos_says_them() {
        assert_eq!(size(512), "512 B");
        assert_eq!(size(4_200_000_000), "4.2 GB");
        assert_eq!(size(880_000_000), "880 MB");
        assert_eq!(size(15_000_000_000), "15 GB");
    }

    #[test]
    fn levels_order_from_fine_to_none() {
        assert!(DiskLevel::Ok < DiskLevel::Low && DiskLevel::Low < DiskLevel::Critical && DiskLevel::Critical < DiskLevel::Full);
        assert!(!DiskLevel::Low.holds_builds() && DiskLevel::Critical.holds_builds());
    }

    #[test]
    fn a_report_round_trips_and_leaves_out_what_is_empty() {
        let report = DiskReport {
            status: DiskStatus { level: DiskLevel::Critical, free: 4_200_000_000, total: 494_000_000_000, path: "/Users/abel/.equipe".into(), container_free: Some(4_100_000_000) },
            items: vec![DiskItem {
                id: "worktree-build:3".into(),
                group: DiskGroup::Worktrees,
                kind: DiskKind::WorktreeBuild,
                label: "T-24 · target/".into(),
                path: "/Users/abel/.equipe/worktrees/site/t24-6ad1cb4a/target".into(),
                bytes: 9_800_000_000,
                partial: false,
                safe: true,
                why: DiskWhy::Merged,
                squad: Some("acme".into()),
                ticket: Some(24),
                command: None,
            }],
            took_ms: 840,
            partial: false,
            at: 1_760_000_000,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["status"]["level"], "critical");
        assert_eq!(json["items"][0]["why"], "merged");
        assert!(json["items"][0].get("partial").is_none() && json["items"][0].get("command").is_none());
        assert_eq!(serde_json::from_value::<DiskReport>(json).unwrap(), report);
        assert_eq!(report.reclaimable(), 9_800_000_000);
    }
}
