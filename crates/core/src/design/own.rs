//! What equipe writes into a project's main checkout itself, outside any
//! ticket (docs/DESIGN-CAPTURES.md, "Files equipe writes on main").
//!
//! Most design work is a ticket's, made and committed in its worktree. A
//! few files are written where the project's design folder is — its main
//! checkout — when the person acts: the style profile and taste notes in
//! `design/taste.md`, a design decision and its picture in
//! `design/decisions/`, a reference kept from the conversation, a critique
//! run there. Left uncommitted, they block the next landing: main is
//! checked out there, and its fast-forward refuses to touch them.
//!
//! So each is committed on main as soon as it is written ([`land`]) — one
//! commit of equipe's, made apart from the checkout, which then follows by
//! a fast-forward ([`crate::worktree::commit_checkout_files_on_main`]) —
//! and before every landing whatever such files are still uncommitted
//! there are committed the same way ([`commit_pending`]). The taste tuner's
//! signals are the person's own on this machine and stay out of the working
//! tree altogether (`taste::tuner::signals_file`); one an older equipe left
//! in a checkout is moved there.
//!
//! In a ticket's worktree nothing is done: its member commits its work. A
//! squad's folder is saved its own way (`squad_home::save_in`).

use std::path::{Path, PathBuf};

use crate::worktree::{self, Result};

/// The message of the commit [`commit_pending`] makes.
pub const PENDING_MESSAGE: &str = "equipe: commit the design notes equipe wrote\n\nTaste notes, decisions and critiques equipe wrote in this checkout\noutside any ticket, committed so main can move.";

/// Whether `path` (from the top of a repository) is a file equipe writes in
/// a design folder outside any ticket: `taste.md`, what is under
/// `decisions/`, a critique's text, a kept reference's `notes.md`, the
/// tuner's signals.
pub fn is_own(path: &str) -> bool {
    let dir = super::DIR;
    let Some(inner) = path.strip_prefix(&format!("{dir}/")).or_else(|| path.rsplit_once(&format!("/{dir}/")).map(|(_, rest)| rest)) else {
        return false;
    };
    inner == crate::taste::store::NOTES_FILE
        || inner.starts_with(&format!("{}/", crate::taste::store::DECISIONS_DIR))
        || (inner.starts_with("critiques/") && inner.ends_with(".md"))
        || (inner.starts_with("references/") && inner.ends_with("/notes.md"))
        || inner == crate::taste::tuner::SIGNALS_FILE
}

/// What [`land`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Landed {
    /// Committed on main: these files, from the top of the repository.
    Committed(Vec<String>),
    /// Nothing to commit: main holds them already.
    Nothing,
    /// Left as they are, and why: a ticket's worktree (its member commits
    /// them), a squad's folder, no repository, main not checked out there.
    Left(&'static str),
}

/// The checkout `dir` is in, when it is a project's main checkout with the
/// main branch checked out: the repository's top and main's name.
fn main_checkout(dir: &Path) -> std::result::Result<PathBuf, &'static str> {
    let top = worktree::git(dir, &["rev-parse", "--show-toplevel"]).map(PathBuf::from).map_err(|_| "not in a git repository")?;
    if worktree::admin_dir(&top).is_some() {
        return Err("a ticket's worktree: its member commits it");
    }
    if worktree::is_squad_home(&top) {
        return Err("a squad's folder, saved its own way");
    }
    let main = worktree::main_branch(&top).map_err(|_| "no main branch")?;
    match worktree::checked_out_at(&top, &main) {
        Some(at) if same(&at, &top) => Ok(top),
        _ => Err("main isn't checked out there"),
    }
}

fn same(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// `files` — just written by equipe in the design folder `dir` — committed
/// on main with `message` when `dir` is in the project's main checkout with
/// main checked out. Runs git: off the daemon's loop ([`land_soon`] there).
pub fn land(dir: &Path, files: &[PathBuf], message: &str) -> Result<Landed> {
    let top = match main_checkout(dir) {
        Ok(top) => top,
        Err(why) => return Ok(Landed::Left(why)),
    };
    let canon_top = std::fs::canonicalize(&top).unwrap_or_else(|_| top.clone());
    let rels: Vec<String> = files
        .iter()
        .filter_map(|f| {
            let f = std::fs::canonicalize(f).ok()?;
            Some(f.strip_prefix(&canon_top).ok()?.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    if rels.is_empty() {
        return Ok(Landed::Nothing);
    }
    let committed = worktree::commit_checkout_files_on_main(&top, &top, &rels, message)?;
    Ok(if committed.is_empty() { Landed::Nothing } else { Landed::Committed(committed) })
}

/// [`land`], from wherever equipe is: on a blocking thread of the daemon's
/// runtime when there is one, else at once. What went wrong is logged.
pub fn land_soon(dir: &Path, files: Vec<PathBuf>, message: String) {
    let dir = dir.to_owned();
    let run = move || {
        if let Err(err) = land(&dir, &files, &message) {
            crate::daemon_log!("equipe: what equipe wrote in {} stays uncommitted for now (it is committed before the next merge): {err}", dir.display());
        }
    };
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn_blocking(run);
        }
        Err(_) => run(),
    }
}

/// Before main moves in `repo`: the files equipe wrote in the checkout main
/// is checked out in ([`is_own`]) that are still uncommitted — changed, or
/// new — committed on main, and the tuner's signals an older equipe left
/// there moved out of the working tree. Files the person staged, deleted or
/// left conflicted are left alone, as is everything else. The files
/// committed; none when there is nothing to do.
pub fn commit_pending(repo: &Path) -> Result<Vec<String>> {
    let Ok(main) = worktree::main_branch(repo) else { return Ok(Vec::new()) };
    let Some(checkout) = worktree::checked_out_at(repo, &main) else { return Ok(Vec::new()) };
    if worktree::is_squad_home(&checkout) || worktree::admin_dir(&checkout).is_some() {
        return Ok(Vec::new());
    }
    let pattern = format!(":(glob)**/{}/**", super::DIR);
    let status = worktree::git_bytes(&checkout, &["-c", "core.quotePath=false", "status", "--porcelain=v1", "-z", "--untracked-files=all", "--", &pattern], None, None)?;
    let mut pending = Vec::new();
    for entry in status.split(|b| *b == 0).filter(|e| e.len() > 3) {
        let (code, path) = (&entry[..2], String::from_utf8_lossy(&entry[3..]).into_owned());
        let untracked = code == b"??";
        // Changed on disk only: nothing staged, nothing deleted or conflicted.
        if !(untracked || code == b" M") || !is_own(&path) {
            continue;
        }
        if untracked && path.ends_with(&format!("/{}", crate::taste::tuner::SIGNALS_FILE)) {
            let dir = checkout.join(path.trim_end_matches(crate::taste::tuner::SIGNALS_FILE)).join("");
            if let Err(err) = crate::taste::tuner::move_out_of_tree(&dir) {
                crate::daemon_log!("equipe: the taste signals in {} stay where they are: {err}", dir.display());
            }
            continue;
        }
        pending.push(path);
    }
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    worktree::commit_checkout_files_on_main(repo, &checkout, &pending, PENDING_MESSAGE)
}

#[cfg(test)]
mod tests;
