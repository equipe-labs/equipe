//! The disk, as the window shows it: the banner while it runs short, and the
//! 디스크 panel — free space with a bar, what uses it, the fixes worth making,
//! and a sheet that lists exactly what a clean removes before it does. The
//! daemon looks and cleans, on whichever machine it is (`Command::DiskScan`,
//! `Command::DiskClean`); here are only what is shown and what to ask next.
//! Drawn by `views/disk.rs`.

use std::collections::BTreeSet;
use std::time::Instant;

use equipe_core::disk::{DiskCleaned, DiskGroup, DiskItem, DiskKind, DiskLevel, DiskReport, DiskStatus, DiskWhy};
use equipe_core::protocol::{Command, Event};

/// Where the panel is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Stage {
    /// What uses the disk, and the fixes.
    #[default]
    List,
    /// "These go": the items picked, before the person says yes.
    Confirm,
    /// The daemon is giving them back.
    Cleaning,
    /// What it gave back.
    Done(DiskCleaned),
}

/// One fix worth making: a set of items equipe may remove, or one only the
/// person can clear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    pub kind: FixKind,
    /// What it gives back.
    pub bytes: u64,
    /// The items it removes (equipe's) or the one to clear (the person's).
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixKind {
    /// Finished tickets' worktrees and the build output in them.
    Worktrees,
    /// Members' build folders nobody builds in now.
    Builds,
    /// Logs past their size.
    Logs,
    /// One of the person's caches or build folders: a command, or Finder.
    Yourself,
}

#[derive(Debug, Default)]
pub struct DiskUi {
    /// The disk's room, as the daemon last said (the snapshot, then changes).
    pub status: Option<DiskStatus>,
    pub open: bool,
    pub stage: Stage,
    /// The latest look, and whether one is under way.
    pub report: Option<DiskReport>,
    pub looking: bool,
    /// The items a clean would remove, by id.
    pub picked: BTreeSet<String>,
    /// The banner was closed at this level: it stays away until it gets worse.
    pub dismissed: Option<DiskLevel>,
    /// The command just copied, and when ("복사됨" for a moment).
    pub copied: Option<(String, Instant)>,
}

impl DiskUi {
    /// What the daemon said. `true` when something shown changed.
    pub fn on_event(&mut self, event: &Event) -> bool {
        match event {
            Event::DiskLevel { status } => {
                // Better again: the next time it runs short, the banner shows again.
                if self.dismissed.is_some_and(|d| status.level < d) {
                    self.dismissed = None;
                }
                self.status = Some(status.clone());
                true
            }
            Event::DiskReport { report } => {
                self.looking = false;
                self.status = Some(DiskStatus { container_free: None, ..report.status.clone() });
                // What was picked and is still there stays picked.
                self.picked.retain(|id| report.items.iter().any(|i| i.safe && &i.id == id));
                self.report = Some(report.clone());
                true
            }
            Event::DiskCleaned { cleaned } => {
                self.status = Some(cleaned.status.clone());
                self.picked.clear();
                if self.stage == Stage::Cleaning {
                    self.stage = Stage::Done(cleaned.clone());
                }
                true
            }
            _ => false,
        }
    }

    /// The daemon's status from a fresh snapshot: `None` there means there's room.
    pub fn attached(&mut self, status: Option<DiskStatus>) {
        self.status = status;
    }

    /// The banner's level, while the disk is short and the person hasn't closed it.
    pub fn banner(&self) -> Option<&DiskStatus> {
        let status = self.status.as_ref()?;
        (status.level.is_warning() && self.dismissed.is_none_or(|d| status.level > d)).then_some(status)
    }

    pub fn dismiss(&mut self) {
        self.dismissed = self.status.as_ref().map(|s| s.level);
    }

    /// The panel opens, and the daemon is asked to look.
    pub fn open(&mut self) -> Vec<Command> {
        self.open = true;
        self.stage = Stage::List;
        self.rescan()
    }

    pub fn rescan(&mut self) -> Vec<Command> {
        if self.looking {
            return Vec::new();
        }
        self.looking = true;
        vec![Command::DiskScan]
    }

    pub fn close(&mut self) {
        self.open = false;
        self.stage = Stage::List;
        self.picked.clear();
    }

    /// The fix's items go to the sheet that asks first.
    pub fn ask(&mut self, ids: &[String]) {
        let Some(report) = &self.report else { return };
        self.picked = ids.iter().filter(|id| report.items.iter().any(|i| i.safe && &i.id == *id)).cloned().collect();
        if !self.picked.is_empty() {
            self.stage = Stage::Confirm;
        }
    }

    /// Every item equipe may remove, to the sheet.
    pub fn ask_all(&mut self) {
        let ids: Vec<String> = self.report.iter().flat_map(|r| r.items.iter().filter(|i| i.safe).map(|i| i.id.clone())).collect();
        self.ask(&ids);
    }

    /// Back from the sheet, or from what was given back.
    pub fn back(&mut self) {
        self.stage = Stage::List;
        self.picked.clear();
    }

    /// The person said yes: the daemon gives them back.
    pub fn confirm(&mut self) -> Vec<Command> {
        if self.stage != Stage::Confirm || self.picked.is_empty() {
            return Vec::new();
        }
        self.stage = Stage::Cleaning;
        vec![Command::DiskClean { ids: self.picked.iter().cloned().collect() }]
    }

    /// The items the sheet lists, biggest first, and their total.
    pub fn plan(&self) -> (Vec<&DiskItem>, u64) {
        let Some(report) = &self.report else { return (Vec::new(), 0) };
        let mut items: Vec<&DiskItem> = report.items.iter().filter(|i| self.picked.contains(&i.id)).collect();
        items.sort_by_key(|i| std::cmp::Reverse(i.bytes));
        let total = items.iter().map(|i| i.bytes).sum();
        (items, total)
    }

    /// The window lost the daemon: what was under way won't answer.
    pub fn disconnected(&mut self) {
        self.looking = false;
        if self.stage == Stage::Cleaning {
            self.stage = Stage::List;
        }
    }
}

/// The fixes worth making, most room first: what equipe may give back,
/// then the person's own biggest caches, each with its command.
pub fn fixes(report: &DiskReport) -> Vec<Fix> {
    let mut fixes = Vec::new();
    let mut add = |kind: FixKind, items: Vec<&DiskItem>| {
        if !items.is_empty() {
            fixes.push(Fix { kind, bytes: items.iter().map(|i| i.bytes).sum(), ids: items.iter().map(|i| i.id.clone()).collect() });
        }
    };
    let safe = |group: DiskGroup| report.items.iter().filter(move |i| i.safe && i.group == group);
    add(FixKind::Worktrees, safe(DiskGroup::Worktrees).collect());
    add(FixKind::Builds, safe(DiskGroup::Builds).collect());
    add(FixKind::Logs, safe(DiskGroup::Equipe).filter(|i| i.kind == DiskKind::Log).collect());
    fixes.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    // The person's own, biggest few: after equipe's, which are one click and safe.
    let mut theirs: Vec<&DiskItem> = report.items.iter().filter(|i| i.why == DiskWhy::NotEquipes && i.bytes >= 1_000_000_000).collect();
    theirs.sort_by_key(|i| std::cmp::Reverse(i.bytes));
    fixes.extend(theirs.into_iter().take(3).map(|i| Fix { kind: FixKind::Yourself, bytes: i.bytes, ids: vec![i.id.clone()] }));
    fixes
}

/// The groups as the panel lists them, each with its items, biggest first,
/// and its total.
pub fn groups(report: &DiskReport) -> Vec<(DiskGroup, Vec<&DiskItem>, u64)> {
    const ORDER: [DiskGroup; 5] = [DiskGroup::Worktrees, DiskGroup::Builds, DiskGroup::Projects, DiskGroup::Equipe, DiskGroup::Caches];
    let mut out: Vec<(DiskGroup, Vec<&DiskItem>, u64)> = ORDER
        .into_iter()
        .filter_map(|group| {
            let mut items: Vec<&DiskItem> = report.items.iter().filter(|i| i.group == group && i.bytes > 0).collect();
            if items.is_empty() {
                return None;
            }
            items.sort_by_key(|i| std::cmp::Reverse(i.bytes));
            let total = items.iter().map(|i| i.bytes).sum();
            Some((group, items, total))
        })
        .collect();
    out.sort_by_key(|(_, _, total)| std::cmp::Reverse(*total));
    out
}

/// How full the disk is, 0–1.
pub fn used_fraction(status: &DiskStatus) -> f32 {
    if status.total == 0 { 0. } else { (status.total.saturating_sub(status.free) as f64 / status.total as f64) as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn item(id: &str, group: DiskGroup, kind: DiskKind, bytes: u64, safe: bool, why: DiskWhy) -> DiskItem {
        DiskItem { id: id.into(), group, kind, label: id.into(), path: PathBuf::from(format!("/x/{id}")), bytes, partial: false, safe, why, squad: None, ticket: None, command: None }
    }

    fn report() -> DiskReport {
        DiskReport {
            status: DiskStatus { level: DiskLevel::Critical, free: 4_200_000_000, total: 494_000_000_000, path: "/u/.equipe".into(), container_free: None },
            items: vec![
                item("wt-build", DiskGroup::Worktrees, DiskKind::WorktreeBuild, 9_000_000_000, true, DiskWhy::Merged),
                item("wt", DiskGroup::Worktrees, DiskKind::Worktree, 300_000_000, true, DiskWhy::Merged),
                item("wt-open", DiskGroup::Worktrees, DiskKind::Worktree, 2_000_000_000, false, DiskWhy::OpenTicket),
                item("build", DiskGroup::Builds, DiskKind::BuildCache, 12_000_000_000, true, DiskWhy::Stale),
                item("log", DiskGroup::Equipe, DiskKind::Log, 40_000_000, true, DiskWhy::OldLog),
                item("derived", DiskGroup::Caches, DiskKind::DerivedData, 20_000_000_000, false, DiskWhy::NotEquipes),
                item("small", DiskGroup::Caches, DiskKind::Npm, 600_000_000, false, DiskWhy::NotEquipes),
            ],
            took_ms: 1200,
            partial: false,
            at: 0,
        }
    }

    #[test]
    fn fixes_come_most_room_first_equipes_before_the_persons() {
        let fixes = fixes(&report());
        let kinds: Vec<FixKind> = fixes.iter().map(|f| f.kind).collect();
        assert_eq!(kinds, vec![FixKind::Builds, FixKind::Worktrees, FixKind::Logs, FixKind::Yourself]);
        assert_eq!(fixes[1].bytes, 9_300_000_000);
        assert_eq!(fixes[1].ids, vec!["wt-build".to_owned(), "wt".to_owned()]);
        // Under a gigabyte, the person's own isn't worth a fix.
        assert_eq!(fixes[3].ids, vec!["derived".to_owned()]);
    }

    #[test]
    fn the_sheet_lists_only_what_equipe_may_remove_and_sends_exactly_that() {
        let mut ui = DiskUi::default();
        assert_eq!(ui.open(), vec![Command::DiskScan]);
        assert!(ui.rescan().is_empty(), "one look at a time");
        ui.on_event(&Event::DiskReport { report: report() });
        ui.ask(&["wt-build".into(), "wt-open".into(), "derived".into()]);
        assert_eq!(ui.stage, Stage::Confirm);
        let (items, total) = ui.plan();
        assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["wt-build"]);
        assert_eq!(total, 9_000_000_000);
        assert_eq!(ui.confirm(), vec![Command::DiskClean { ids: vec!["wt-build".into()] }]);
        assert_eq!(ui.stage, Stage::Cleaning);
        let cleaned = DiskCleaned { freed: 9_000_000_000, removed: vec!["wt-build".into()], skipped: vec![], status: DiskStatus { level: DiskLevel::Low, free: 13_200_000_000, ..Default::default() } };
        ui.on_event(&Event::DiskCleaned { cleaned: cleaned.clone() });
        assert_eq!(ui.stage, Stage::Done(cleaned));
        assert_eq!(ui.status.as_ref().map(|s| s.level), Some(DiskLevel::Low));
    }

    #[test]
    fn nothing_safe_asks_nothing() {
        let mut ui = DiskUi::default();
        ui.on_event(&Event::DiskReport { report: report() });
        ui.ask(&["derived".into()]);
        assert_eq!(ui.stage, Stage::List);
        assert!(ui.confirm().is_empty());
    }

    #[test]
    fn a_closed_banner_comes_back_only_when_it_gets_worse() {
        let mut ui = DiskUi::default();
        let at = |level| Event::DiskLevel { status: DiskStatus { level, free: 1, total: 2, ..Default::default() } };
        ui.on_event(&at(DiskLevel::Low));
        assert!(ui.banner().is_some());
        ui.dismiss();
        assert!(ui.banner().is_none());
        ui.on_event(&at(DiskLevel::Low));
        assert!(ui.banner().is_none());
        ui.on_event(&at(DiskLevel::Critical));
        assert!(ui.banner().is_some());
        ui.dismiss();
        // Room again, then short again: shown.
        ui.on_event(&at(DiskLevel::Ok));
        assert!(ui.banner().is_none());
        ui.on_event(&at(DiskLevel::Low));
        assert!(ui.banner().is_some());
    }

    #[test]
    fn groups_go_biggest_first() {
        let r = report();
        let groups = groups(&r);
        assert_eq!(groups.iter().map(|g| g.0).collect::<Vec<_>>(), vec![DiskGroup::Caches, DiskGroup::Builds, DiskGroup::Worktrees, DiskGroup::Equipe]);
        assert_eq!(groups[2].1[0].id, "wt-build");
    }
}
