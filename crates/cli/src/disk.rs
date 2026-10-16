//! `equipe disk`: how much room the disk has, what uses it, and what to do
//! about it; `equipe disk clean` gives back what equipe may remove, after
//! the person's yes; `equipe disk debug` sets how lean members' Rust builds
//! are. The daemon looks and cleans when it runs (one hand on the worktrees);
//! with none running, this process does, from the squads' journals.

use std::path::{Path, PathBuf};

use equipe_core::disk::{self, DiskCleaned, DiskGroup, DiskItem, DiskReport};
use equipe_core::i18n::label;
use equipe_core::t;

use crate::ui;

/// The groups in the order they're shown.
const GROUPS: [DiskGroup; 5] = [DiskGroup::Worktrees, DiskGroup::Builds, DiskGroup::Projects, DiskGroup::Equipe, DiskGroup::Caches];
/// Rows shown per group before the rest is summed up.
const ROWS: usize = 8;

pub async fn command(words: &[String], squad: Option<String>, yes: bool) -> Result<(), String> {
    let home = equipe_providers::credentials::equipe_home();
    match words.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => show(&home).await,
        ["clean"] => clean(&home, yes).await,
        ["debug", rest @ ..] => debug(&home, squad, rest.first().copied()),
        _ => Err(t!("disk.usage").into()),
    }
}

/// The daemon's look when it runs; this process's own otherwise.
async fn look(home: &Path) -> Result<DiskReport, String> {
    let socket = equipe_core::gateway::socket_path(home);
    if equipe_core::gateway::is_running(&socket).await {
        return equipe_core::gateway::disk_report(&socket).await.map_err(|e| e.to_string())?.ok_or_else(|| t!("disk.no_answer").into());
    }
    let home = home.to_owned();
    tokio::task::spawn_blocking(move || {
        let known = disk::known_from_journals(&home);
        let projects: Vec<PathBuf> = known.projects.iter().map(|(_, p)| p.clone()).collect();
        let mut status = disk::probe(&disk::watched(&home, &projects)).unwrap_or_default();
        status.level = disk::next_level(disk::DiskLevel::Ok, status.free, status.total, false);
        disk::report(&known, status, disk::SCAN_BUDGET)
    })
    .await
    .map_err(|e| e.to_string())
}

async fn give_back(home: &Path, ids: Vec<String>) -> Result<DiskCleaned, String> {
    let socket = equipe_core::gateway::socket_path(home);
    if equipe_core::gateway::is_running(&socket).await {
        return equipe_core::gateway::disk_clean(&socket, ids).await.map_err(|e| e.to_string())?.ok_or_else(|| t!("disk.no_answer").into());
    }
    let home = home.to_owned();
    tokio::task::spawn_blocking(move || {
        let known = disk::known_from_journals(&home);
        let mut cleaned = disk::clean(&known, &ids);
        let projects: Vec<PathBuf> = known.projects.iter().map(|(_, p)| p.clone()).collect();
        if let Some(mut status) = disk::probe(&disk::watched(&home, &projects)) {
            status.level = disk::next_level(disk::DiskLevel::Ok, status.free, status.total, false);
            cleaned.status = status;
        }
        cleaned
    })
    .await
    .map_err(|e| e.to_string())
}

/// `██████████████████░░░░` for `used` of `total`.
pub fn bar(used: u64, total: u64, width: usize) -> String {
    let filled = if total == 0 { 0 } else { ((used as f64 / total as f64) * width as f64).round() as usize }.min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

/// The status in two lines: free of total with a bar, and the level.
fn status_lines(status: &disk::DiskStatus) -> Vec<String> {
    let p = ui::out();
    let lang = crate::i18n::lang();
    let used = status.total.saturating_sub(status.free);
    let level = label::disk_level(lang, status.level);
    let level = if status.level.is_warning() { p.bold(level) } else { p.dim(level) };
    let mut lines = vec![format!(
        "  {}  {}  {}",
        p.bold(&t!("disk.free_of", free = disk::size(status.free), total = disk::size(status.total))),
        p.dim(&bar(used, status.total, 24)),
        level
    )];
    if let Some(line) = label::disk_line(lang, status) {
        lines.push(p.note(2, &line));
    }
    if let Some(container) = status.container_free.filter(|c| c.abs_diff(status.free) > 1_000_000_000) {
        lines.push(p.note(2, &t!("disk.container_free", free = disk::size(container))));
    }
    lines
}

/// One item as a row: its size, its words, why — and for what only the
/// person clears, the command.
fn item_lines(item: &DiskItem) -> Vec<String> {
    let p = ui::out();
    let lang = crate::i18n::lang();
    let size = format!("{}{}", if item.partial { "≥" } else { "" }, disk::size(item.bytes));
    let why = label::disk_why(lang, item.why);
    let mark = if item.safe { p.accent("✓") } else { " ".into() };
    let mut lines = vec![format!("  {mark} {}  {}  {}", p.bold(&ui::pad(&size, 9)), item.label, p.dim(why))];
    if let Some(command) = &item.command {
        lines.push(format!("      {}", p.dim(&format!("$ {command}"))));
    }
    lines
}

async fn show(home: &Path) -> Result<(), String> {
    let p = ui::out();
    eprintln!("{}", p.dim(t!("disk.looking")));
    let report = look(home).await?;
    println!();
    println!("{}", p.section(t!("disk.title"), &report.status.path.display().to_string()));
    for line in status_lines(&report.status) {
        println!("{line}");
    }
    let lang = crate::i18n::lang();
    for group in GROUPS {
        let items: Vec<&DiskItem> = report.items.iter().filter(|i| i.group == group).collect();
        if items.is_empty() {
            continue;
        }
        let total: u64 = items.iter().map(|i| i.bytes).sum();
        println!();
        println!("{}", p.section(label::disk_group(lang, group), &disk::size(total)));
        for item in items.iter().take(ROWS) {
            for line in item_lines(item) {
                println!("{line}");
            }
        }
        if items.len() > ROWS {
            let rest: u64 = items[ROWS..].iter().map(|i| i.bytes).sum();
            println!("  {}", p.dim(&t!("disk.more", count = items.len() - ROWS, size = disk::size(rest))));
        }
    }
    println!();
    if report.partial {
        println!("{}", p.note(2, t!("disk.partial")));
    }
    let reclaim = report.reclaimable();
    if reclaim > 0 {
        println!("  {}  {}", p.hint("equipe disk clean"), p.dim(&t!("disk.can_free", size = disk::size(reclaim))));
    } else {
        println!("{}", p.note(2, t!("disk.nothing_safe")));
    }
    if report.items.iter().any(|i| i.command.is_some()) {
        println!("{}", p.note(2, t!("disk.yourself")));
    }
    Ok(())
}

async fn clean(home: &Path, yes: bool) -> Result<(), String> {
    let p = ui::out();
    eprintln!("{}", p.dim(t!("disk.looking")));
    let report = look(home).await?;
    let safe: Vec<&DiskItem> = report.items.iter().filter(|i| i.safe).collect();
    if safe.is_empty() {
        println!("{}", p.ok_line(t!("disk.nothing_safe")));
        return Ok(());
    }
    let total: u64 = safe.iter().map(|i| i.bytes).sum();
    println!();
    println!("{}", p.para(0, &t!("disk.will_remove", count = safe.len(), size = disk::size(total))));
    for item in &safe {
        for line in item_lines(item) {
            println!("{line}");
        }
        println!("      {}", p.dim(&item.path.display().to_string()));
    }
    println!("{}", p.note(2, t!("disk.kept")));
    println!();
    if !yes {
        if !ui::interactive() {
            return Err(t!("disk.ask_yes").into());
        }
        let go = dialoguer::Confirm::new().with_prompt(t!("disk.confirm", size = disk::size(total))).default(false).interact().map_err(|e| e.to_string())?;
        if !go {
            println!("{}", p.note(0, t!("disk.nothing_removed")));
            return Ok(());
        }
    }
    let ids: Vec<String> = safe.iter().map(|i| i.id.clone()).collect();
    let cleaned = give_back(home, ids).await?;
    println!("{}", p.ok_line(&t!("disk.freed", size = disk::size(cleaned.freed), count = cleaned.removed.len())));
    let lang = crate::i18n::lang();
    for (id, why) in &cleaned.skipped {
        let name = report.items.iter().find(|i| &i.id == id).map_or(id.as_str(), |i| i.label.as_str());
        println!("{}", p.note(2, &t!("disk.skipped", name = name, why = label::disk_why(lang, *why))));
    }
    for line in status_lines(&cleaned.status) {
        println!("{line}");
    }
    Ok(())
}

/// Show or set how lean a squad's Rust builds are.
fn debug(home: &Path, squad: Option<String>, word: Option<&str>) -> Result<(), String> {
    let p = ui::out();
    let cwd = std::env::current_dir().unwrap_or_default();
    let squad = crate::home::squad_for(home, &cwd, squad).ok_or_else(|| t!("disk.no_squad").to_owned())?;
    let journal = equipe_core::journal::Journal::for_squad(home, &squad).map_err(|e| e.to_string())?;
    let describe = |d: disk::BuildDebug| match d {
        disk::BuildDebug::Lines => t!("disk.debug_lines"),
        disk::BuildDebug::Off => t!("disk.debug_off"),
        disk::BuildDebug::Full => t!("disk.debug_full"),
    };
    let Some(word) = word else {
        let now = journal.build_debug();
        println!("{}", p.row(&squad, &format!("{} — {}", now.word(), describe(now))));
        println!("{}", p.note(2, t!("disk.debug_how")));
        return Ok(());
    };
    let debug = disk::BuildDebug::parse(word).ok_or_else(|| t!("disk.debug_how").to_owned())?;
    journal.set_build_debug(debug).map_err(|e| e.to_string())?;
    println!("{}", p.ok_line(&t!("disk.debug_set", squad = squad, word = debug.word(), what = describe(debug))));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_fills_with_what_is_used() {
        assert_eq!(bar(0, 100, 10), "░░░░░░░░░░");
        assert_eq!(bar(96, 100, 10), "██████████");
        assert_eq!(bar(50, 100, 10), "█████░░░░░");
        assert_eq!(bar(5, 0, 4), "░░░░");
    }
}
