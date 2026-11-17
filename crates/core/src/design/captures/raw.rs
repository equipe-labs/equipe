//! What a squad folder's git keeps of what its tasks hand over: what people
//! read (docs/DESIGN-CAPTURES.md).
//!
//! A hand-in folder (`outputs/T-12 title/`) is for the person and the
//! reviewer: a README that says what was found and names its evidence, and a
//! few small key files. Its pictures, videos and documents already stay out
//! of git ([`OUTPUTS`]); what filled one squad's folder's history past
//! 35,000 files after that was machine output — QA probe and DOM dumps,
//! logs, generated scripts, matrices of every width and theme — one ticket
//! alone handing in 5,000 of them. So:
//!
//! - **Machine output stays on disk only**: anything under a hand-in's
//!   `raw/` folder, and files of a machine kind ([`MACHINE_EXTS`]) anywhere
//!   under `outputs/`. `outputs/.gitignore` carries equipe's block
//!   ([`ignore_block`]); when a task lands, what its worktree holds of it is
//!   carried into the squad's folder ([`carry`]), and kept there
//!   [`RAW_KEPT_DAYS`] days ([`sweep`]).
//! - **Git keeps at most [`FILES_KEPT`] files and [`BYTES_KEPT`] bytes of a
//!   hand-in** — a summary first ([`pick`]). What is past that stays where it
//!   is, listed in the hand-in's own `.gitignore` ([`with_cap`]); it is never
//!   swept away.
//! - Every commit equipe makes in a task's worktree untracks the rest first
//!   ([`prepare`]); **landing guards it** ([`guard_in`]): a branch whose own
//!   commits add machine output anyway has them rewritten without it, and the
//!   member is told ([`told`]). **Upgrading** ([`migrate`]) is one commit on
//!   main that stops tracking what an older folder tracked; every file stays.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::{OUTPUTS, Store, any_case, edit_tree, in_squad_folder, ls_files, now, put_block, rewrite_commits, secs, set_in_index, size_words, tree_blobs, untrack, untrack_on_main};
use crate::worktree::{self, Result};

/// The folder a hand-in keeps its bulk in: never in git.
pub const RAW: &str = "raw";
/// Folders whose files are machine output wherever they are.
const MACHINE_DIRS: [&str; 2] = [RAW, "node_modules"];
/// Machine output, by extension (any case): dumps, logs, scripts and pages
/// tools write, source copied to probe with.
pub const MACHINE_EXTS: [&str; 46] = [
    "json", "jsonl", "ndjson", "log", "out", "err", "stdout", "stderr", "har", "trace", "map", "mjs", "cjs", "js", "jsx", "ts", "tsx", "mts", "cts", "py", "sh", "bash", "zsh", "rb", "go",
    "rs", "java", "kt", "swift", "c", "h", "cpp", "html", "htm", "css", "scss", "less", "xml", "yml", "yaml", "toml", "diff", "patch", "sqlite", "db", "wasm",
];
/// Of one hand-in, git keeps at most this many files…
pub const FILES_KEPT: usize = 20;
/// …this many bytes in all…
pub const BYTES_KEPT: u64 = 1_000_000;
/// …and no file bigger than this: one that big is a dump.
pub const FILE_KEPT: u64 = 256 * 1024;
/// How long machine output stays in the squad's folder.
pub const RAW_KEPT_DAYS: u64 = 30;
const RAW_KEPT: u64 = RAW_KEPT_DAYS * 86_400;
/// A hand-in's summary, by its name without the extension (any case): kept first.
const SUMMARIES: [&str; 7] = ["readme", "summary", "report", "index", "handoff", "results", "findings"];
/// equipe's own files under `outputs/`, never machine output.
const OWN: [&str; 3] = [".gitignore", ".gitkeep", ".captures.json"];
/// What they are called in the log.
pub const WHAT: &str = "hand-ins' machine output";

const DIR: &str = "outputs";
const BLOCK_START: &str = "# >>> equipe: machine output stays on disk, out of git";
const BLOCK_END: &str = "# <<< equipe: machine output";
const CAP_START: &str = "# >>> equipe: past what git keeps of this hand-in";
const CAP_END: &str = "# <<< equipe: past what git keeps";
/// In the store's folder: since when equipe has kept machine output out of
/// git there, Unix seconds — what [`sweep`] counts from for files older than that.
const SINCE: &str = "outputs-raw-since";

/// Whether `rel` — a path from the top of a squad's folder — is machine
/// output: under `outputs/`, in a `raw/` (or `node_modules/`) folder or of a
/// kind in [`MACHINE_EXTS`] (a hidden one too: a tool's `.a.png.audit.json`).
/// equipe's own files ([`OWN`]) never are.
pub fn is_machine(rel: &str) -> bool {
    let Some(rest) = rel.strip_prefix("outputs/") else { return false };
    let parts: Vec<&str> = rest.split('/').collect();
    let name = parts[parts.len() - 1];
    if parts.iter().any(|p| p.is_empty() || *p == "..") || OWN.contains(&name) {
        return false;
    }
    parts[..parts.len() - 1].iter().any(|d| MACHINE_DIRS.contains(d)) || machine_name(name)
}

fn machine_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && MACHINE_EXTS.iter().any(|e| e.eq_ignore_ascii_case(ext)))
}

/// Whether git may keep `rel`, within its hand-in's limit: under `outputs/`,
/// not hidden, not machine output, not one equipe's store keeps.
fn readable(rel: &str) -> bool {
    rel.starts_with("outputs/") && !rel.split('/').any(|p| p.starts_with('.')) && !is_machine(rel) && !OUTPUTS.keeps(rel)
}

/// The hand-in folder `rel` is in, and its path inside it:
/// `outputs/T-3 probe/evidence/a.md` → `("T-3 probe", "evidence/a.md")`.
/// `None` for a file right in `outputs/`, or elsewhere.
fn hand_in(rel: &str) -> Option<(&str, &str)> {
    let (folder, inner) = rel.strip_prefix("outputs/")?.split_once('/')?;
    (!folder.is_empty() && !inner.is_empty()).then_some((folder, inner))
}

/// The block equipe keeps in `outputs/.gitignore`.
pub fn ignore_block() -> String {
    let mut out = format!(
        "{BLOCK_START}\n# Probe and DOM dumps, logs, generated scripts and anything under a hand-in's raw/ stay in\n# the folder ({RAW_KEPT_DAYS} days), never in its history. equipe rewrites this block: edit outside it.\n"
    );
    for dir in MACHINE_DIRS {
        out.push_str(&format!("{dir}/\n"));
    }
    for ext in MACHINE_EXTS {
        out.push_str(&format!("*.{}\n", any_case(ext)));
    }
    out.push_str(BLOCK_END);
    out.push('\n');
    out
}

/// `existing` (`outputs/.gitignore`'s text) with equipe's block as it is now.
pub fn with_block(existing: &str) -> String {
    put_block(existing, BLOCK_START, BLOCK_END, &ignore_block())
}

/// Of a hand-in's readable files — path inside it, size — the ones git
/// keeps, and the ones past its limit: its summary first, then the files
/// nearest its top, Markdown before the rest, smaller before bigger.
pub fn pick(files: &[(String, u64)]) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut order: Vec<&(String, u64)> = files.iter().collect();
    order.sort_by_key(|(path, size)| (!is_summary(path), path.matches('/').count(), !is_markdown(path), *size, path.clone()));
    let (mut kept, mut over) = (BTreeSet::new(), BTreeSet::new());
    let mut bytes = 0;
    for (path, size) in order {
        if kept.len() < FILES_KEPT && *size <= FILE_KEPT && bytes + size <= BYTES_KEPT {
            bytes += size;
            kept.insert(path.clone());
        } else {
            over.insert(path.clone());
        }
    }
    (kept, over)
}

fn is_summary(path: &str) -> bool {
    !path.contains('/') && SUMMARIES.iter().any(|s| path.rsplit_once('.').map_or(path, |(stem, _)| stem).eq_ignore_ascii_case(s))
}

fn is_markdown(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}

/// `existing` (a hand-in's own `.gitignore`) listing `over` — paths inside
/// the hand-in past what git keeps of it — in equipe's block; none: no block.
pub fn with_cap(existing: &str, over: &BTreeSet<String>) -> String {
    let block = if over.is_empty() {
        String::new()
    } else {
        let mut out = format!(
            "{CAP_START}\n# Git keeps {FILES_KEPT} files and {} of a hand-in, none over {}: these stay in the\n# folder, out of its history. equipe rewrites this block: edit outside it.\n",
            size_words(BYTES_KEPT),
            size_words(FILE_KEPT)
        );
        for path in over {
            out.push_str(&pattern(path));
            out.push('\n');
        }
        out.push_str(CAP_END);
        out.push('\n');
        out
    };
    put_block(existing, CAP_START, CAP_END, &block)
}

/// The paths a hand-in's own `.gitignore` lists in equipe's block ([`with_cap`]).
fn listed(text: &str) -> BTreeSet<String> {
    let Some(at) = text.find(CAP_START) else { return BTreeSet::new() };
    let block = &text[at..];
    let block = &block[..block.find(CAP_END).unwrap_or(block.len())];
    block.lines().skip(1).filter_map(|l| l.strip_prefix('/')).map(unpattern).collect()
}

/// The path a [`pattern`] matches.
fn unpattern(pattern: &str) -> String {
    let mut out = String::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// An ignore pattern matching exactly `path`, from the ignore file's folder.
fn pattern(path: &str) -> String {
    let mut out = String::from("/");
    for c in path.chars() {
        if matches!(c, '\\' | '*' | '?' | '[') {
            out.push('\\');
        }
        out.push(c);
    }
    // Trailing spaces count only when escaped.
    let kept = out.trim_end_matches(' ').len();
    let spaces = out.len() - kept;
    out.truncate(kept);
    out.push_str(&"\\ ".repeat(spaces));
    out
}

/// Each hand-in's readable files on disk in `checkout`: folder → (path
/// inside it, size). Hidden files, links and machine folders are passed over.
fn readable_on_disk(checkout: &Path) -> BTreeMap<String, Vec<(String, u64)>> {
    let mut out: BTreeMap<String, Vec<(String, u64)>> = BTreeMap::new();
    let Ok(folders) = std::fs::read_dir(checkout.join(DIR)) else { return out };
    for folder in folders.flatten() {
        let name = folder.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !folder.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let mut files = Vec::new();
        let mut stack = vec![(folder.path(), String::new(), 0usize)];
        while let Some((at, rel, depth)) = stack.pop() {
            for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
                let entry_name = entry.file_name().to_string_lossy().into_owned();
                let Ok(kind) = entry.file_type() else { continue };
                if entry_name.starts_with('.') {
                    continue;
                }
                let path = if rel.is_empty() { entry_name.clone() } else { format!("{rel}/{entry_name}") };
                if kind.is_dir() && depth < 8 && !MACHINE_DIRS.contains(&entry_name.as_str()) {
                    stack.push((entry.path(), path, depth + 1));
                } else if kind.is_file() && readable(&format!("{DIR}/{name}/{path}")) {
                    files.push((path, entry.metadata().map(|m| m.len()).unwrap_or_default()));
                }
            }
        }
        if !files.is_empty() {
            out.insert(name, files);
        }
    }
    out
}

/// Write `path` as `change` makes its text: left alone when that changes
/// nothing, removed when it leaves nothing.
fn rewrite_file(path: &Path, change: impl FnOnce(&str) -> String) -> Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let wanted = change(&existing);
    if wanted == existing {
        return Ok(());
    }
    if wanted.trim().is_empty() {
        return std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, wanted).map_err(|e| format!("{}: {e}", path.display()))
}

/// Before equipe commits a task's worktree of a squad's folder: equipe's
/// block in `outputs/.gitignore`, each hand-in's own `.gitignore` listing
/// what is past its limit, and whatever git tracks of either untracked (the
/// files stay). Nothing outside a squad's folder.
pub fn prepare(worktree: &Path) -> Result<()> {
    if !in_squad_folder(worktree) || !worktree.join(DIR).is_dir() {
        return Ok(());
    }
    rewrite_file(&worktree.join(OUTPUTS.ignore_path), with_block)?;
    let mut over_all: BTreeSet<String> = BTreeSet::new();
    for (folder, files) in readable_on_disk(worktree) {
        let (_, over) = pick(&files);
        rewrite_file(&worktree.join(DIR).join(&folder).join(".gitignore"), |text| with_cap(text, &over))?;
        over_all.extend(over.iter().map(|p| format!("{DIR}/{folder}/{p}")));
    }
    // Only what this checkout holds: a sparse one leaves other tickets' folders as main has them.
    let tracked: Vec<String> = ls_files(worktree, None)?.into_iter().filter(|p| (is_machine(p) || over_all.contains(p)) && worktree.join(p).is_file()).collect();
    if !tracked.is_empty() {
        untrack(worktree, &tracked, None)?;
    }
    Ok(())
}

/// What a tree holds under `outputs/` that git shouldn't — machine output,
/// and what is past each hand-in's limit — and the ignore files it should
/// have for that: `(path, text)`, an empty text for one that should go.
/// Only the hand-ins named in `folders`, when given. A tree holds none of
/// what was past a limit before (it is untracked, still on disk): what a
/// hand-in's list names stays on it, unless the tree tracks it again within
/// the limit.
fn plan(repo: &Path, rev: &str, blobs: &[(String, String, u64)], folders: Option<&BTreeSet<String>>) -> (BTreeSet<String>, Vec<(String, String)>) {
    let show = |rel: &str| worktree::git_bytes(repo, &["show", &format!("{rev}:{rel}")], None, None).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
    let wanted = |folder: &str| folders.is_none_or(|f| f.contains(folder));
    let mut untracked: BTreeSet<String> = blobs.iter().map(|(p, ..)| p).filter(|p| is_machine(p)).cloned().collect();
    let mut readable_files: BTreeMap<&str, Vec<(String, u64)>> = BTreeMap::new();
    let mut capped: BTreeSet<&str> = BTreeSet::new();
    for (path, _, size) in blobs {
        let Some((folder, inner)) = hand_in(path) else { continue };
        if !wanted(folder) {
            continue;
        }
        if inner == ".gitignore" {
            capped.insert(folder);
        } else if readable(path) {
            readable_files.entry(folder).or_default().push((inner.to_owned(), *size));
        }
    }
    let mut ignores = Vec::new();
    let root = show(OUTPUTS.ignore_path).unwrap_or_default();
    if with_block(&root) != root {
        ignores.push((OUTPUTS.ignore_path.to_owned(), with_block(&root)));
    }
    for (folder, files) in &readable_files {
        let (kept, over) = pick(files);
        untracked.extend(over.iter().map(|p| format!("{DIR}/{folder}/{p}")));
        let rel = format!("{DIR}/{folder}/.gitignore");
        let now = if capped.contains(folder) { show(&rel).unwrap_or_default() } else { String::new() };
        let over: BTreeSet<String> = over.into_iter().chain(listed(&now).into_iter().filter(|p| !kept.contains(p))).collect();
        let text = with_cap(&now, &over);
        if text != now {
            ignores.push((rel, if text.trim().is_empty() { String::new() } else { text }));
        }
    }
    (untracked, ignores)
}

/// `ignores` (from [`plan`]) into the scratch `index`.
fn set_ignores(repo: &Path, index: &Path, ignores: &[(String, String)]) -> Result<()> {
    for (rel, text) in ignores {
        if text.is_empty() {
            untrack(repo, std::slice::from_ref(rel), Some(index))?;
        } else {
            set_in_index(repo, index, rel, text.as_bytes())?;
        }
    }
    Ok(())
}

/// What [`migrate`] did: one commit on main.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migrated {
    pub commit: String,
    /// Files no longer tracked, and their size.
    pub untracked: usize,
    pub bytes: u64,
}

/// A squad's folder stops tracking its hand-ins' machine output, and what is
/// past each one's limit: one commit, made apart from any checkout, that
/// untracks them and writes the ignore files; the checkout main is checked
/// out in follows and keeps every file on disk. History stays as it was.
/// `Ok(None)` when there is nothing to do, or `repo` isn't a squad's folder.
pub fn migrate(repo: &Path) -> Result<Option<Migrated>> {
    if !in_squad_folder(repo) {
        return Ok(None);
    }
    let main = worktree::main_branch(repo)?;
    let tip = worktree::git(repo, &["rev-parse", &format!("refs/heads/{main}")])?;
    let blobs = tree_blobs(repo, &tip, DIR)?;
    if blobs.is_empty() {
        return Ok(None);
    }
    let (untracked, ignores) = plan(repo, &tip, &blobs, None);
    if untracked.is_empty() && ignores.is_empty() {
        return Ok(None);
    }
    let paths: Vec<String> = untracked.iter().cloned().collect();
    let tree = worktree::git(repo, &["rev-parse", &format!("{tip}^{{tree}}")])?;
    let new_tree = edit_tree(repo, &tree, |index| {
        if !paths.is_empty() {
            untrack(repo, &paths, Some(index))?;
        }
        set_ignores(repo, index, &ignores)
    })?;
    let bytes: u64 = blobs.iter().filter(|(p, ..)| untracked.contains(p)).map(|(_, _, s)| s).sum();
    let message = format!(
        "equipe: keep {WHAT} out of git\n\n{} files ({}) under {DIR}/ stop being tracked: probe and DOM dumps, logs,\ngenerated scripts, and what is past each hand-in's limit ({FILES_KEPT} files, {}).\nThey stay in the folder; history is unchanged.",
        paths.len(),
        size_words(bytes),
        size_words(BYTES_KEPT)
    );
    let commit = untrack_on_main(repo, &main, &tip, &new_tree, &message, &paths)?;
    if let Some(store) = Store::of(repo) {
        since(&store);
    }
    Ok(Some(Migrated { commit, untracked: paths.len(), bytes }))
}

/// Before `branch` merges into `main` in a squad's folder: its own commits
/// that add machine output under `outputs/`, or more of a hand-in than git
/// keeps, are rewritten without those files (left in its worktree, carried
/// into the folder when it lands), its last commit given the ignore files
/// that keep them out. How many it took out, and their size.
pub(super) fn guard_in(repo: &Path, branch: &str, main: &str) -> Result<(usize, u64)> {
    let local = format!("refs/heads/{branch}");
    let tip = worktree::git(repo, &["rev-parse", &format!("refs/heads/{main}")])?;
    let head = worktree::git(repo, &["rev-parse", &local])?;
    let names = |args: &[&str]| -> Result<Vec<String>> {
        let out = worktree::git_bytes(repo, args, None, None)?;
        Ok(out.split(|b| *b == 0 || *b == b'\n').filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).filter(|p| p.starts_with("outputs/")).collect())
    };
    let mut added: BTreeSet<String> = names(&["-c", "core.quotePath=false", "log", "--no-merges", "--format=", "--name-only", "-z", "--diff-filter=AMT", &format!("{tip}..{head}")])?.into_iter().collect();
    added.extend(names(&["-c", "core.quotePath=false", "diff", "--name-only", "-z", "--diff-filter=AMT", &format!("{tip}...{head}")])?);
    if added.is_empty() {
        return Ok((0, 0));
    }
    let folders: BTreeSet<String> = added.iter().filter_map(|p| hand_in(p)).map(|(f, _)| f.to_owned()).collect();
    let blobs = tree_blobs(repo, &head, DIR)?;
    let (untracked, ignores) = plan(repo, &head, &blobs, Some(&folders));
    // Only what the branch added: a file main tracks stays as it is — taken
    // out here, landing would delete it from the folder ([`migrate`] untracks it).
    let on_main: BTreeSet<String> = tree_blobs(repo, &tip, DIR)?.into_iter().map(|(p, ..)| p).collect();
    let strip: BTreeSet<String> = added.into_iter().filter(|p| (untracked.contains(p) || is_machine(p)) && !on_main.contains(p)).collect();
    if strip.is_empty() {
        return Ok((0, 0));
    }
    let new_head = rewrite_commits(repo, &tip, &head, &strip, &|index, _| set_ignores(repo, index, &ignores))?;
    let checked_out = worktree::checked_out_at(repo, branch);
    worktree::git(repo, &["update-ref", &local, &new_head, &head])?;
    if let Some(worktree) = checked_out {
        // Its index follows the branch; the files stay as they are, and the
        // ignore files equipe wrote match what is committed.
        let _ = worktree::git(&worktree, &["reset", "-q"]);
        for (rel, text) in &ignores {
            let path = worktree.join(rel);
            let _ = if text.is_empty() { std::fs::remove_file(&path) } else { std::fs::write(&path, text) };
        }
    }
    let stripped: Vec<u64> = blobs.iter().filter(|(p, ..)| strip.contains(p)).map(|(_, _, s)| *s).collect();
    Ok((stripped.len(), stripped.iter().sum()))
}

/// What a task's member is told when landing took machine output out of its
/// commits, in English.
pub fn told(ticket: u64, count: usize, bytes: u64) -> String {
    format!(
        "When T-{ticket} landed, equipe took {count} files ({}) under outputs/ out of its commits: machine output (probe and DOM dumps, logs, generated scripts, per-width or per-theme matrices) or more than git keeps of a hand-in ({FILES_KEPT} files, {}). They stay in the squad's folder — machine output for {RAW_KEPT_DAYS} days — and never go into its history. A hand-in folder holds what people read: a README.md that says what you found and names its evidence, plus a few small key files; put bulk output under its raw/ folder, and never `git add -f` it.",
        size_words(bytes),
        size_words(BYTES_KEPT)
    )
}

/// The files a checkout holds under `outputs/` that git ignores and equipe's
/// store doesn't keep: machine output and what is past a hand-in's limit,
/// by path from its top.
fn kept_on_disk(checkout: &Path) -> Vec<String> {
    let Ok(out) = worktree::git_bytes(checkout, &["-c", "core.quotePath=false", "ls-files", "--others", "--ignored", "--exclude-standard", "-z", "--", DIR], None, None) else {
        return Vec::new();
    };
    out.split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .filter(|p| {
            // The folder's own leftovers (its top `.gitignore`'s) aren't hand-ins.
            let name = p.rsplit('/').next().unwrap_or_default();
            name != ".DS_Store" && !name.ends_with(".tmp") && !OUTPUTS.keeps(p)
        })
        .collect()
}

/// Once a task's work landed on main in a squad's folder (main now at a
/// merge of `head`): what the worktree at `head` holds on disk only under
/// `outputs/` is carried into the checkout main is checked out in, at the
/// same paths — before the worktree goes with it. How many files, and their size.
pub fn carry(repo: &Path, head: &str) -> (usize, u64) {
    if !in_squad_folder(repo) {
        return (0, 0);
    }
    let Ok(main) = worktree::main_branch(repo) else { return (0, 0) };
    let checkout = worktree::checked_out_at(repo, &main).unwrap_or_else(|| repo.to_owned());
    let listed = worktree::git(repo, &["worktree", "list", "--porcelain"]).unwrap_or_default();
    let mut done = (0, 0);
    for entry in listed.split("\n\n") {
        let path = entry.lines().find_map(|l| l.strip_prefix("worktree ")).map(PathBuf::from);
        let at_head = entry.lines().any(|l| l.strip_prefix("HEAD ") == Some(head));
        if let Some(path) = path.filter(|p| at_head && *p != checkout) {
            let (files, bytes) = carry_from(&path, &checkout);
            done = (done.0 + files, done.1 + bytes);
        }
    }
    done
}

fn carry_from(from: &Path, to: &Path) -> (usize, u64) {
    let mut done = (0, 0);
    for rel in kept_on_disk(from) {
        let (src, dst) = (from.join(&rel), to.join(&rel));
        let Ok(meta) = std::fs::symlink_metadata(&src) else { continue };
        if !meta.is_file() {
            continue;
        }
        if let Ok(there) = std::fs::symlink_metadata(&dst) {
            if !there.is_file() || (there.len() == meta.len() && there.modified().ok() == meta.modified().ok()) {
                continue;
            }
            let _ = std::fs::remove_file(&dst);
        }
        if let Some(parent) = dst.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            continue;
        }
        let placed = std::fs::hard_link(&src, &dst).is_ok()
            || std::fs::copy(&src, &dst).is_ok_and(|_| {
                if let (Ok(file), Ok(at)) = (std::fs::File::options().write(true).open(&dst), meta.modified()) {
                    let _ = file.set_modified(at);
                }
                true
            });
        if placed {
            done = (done.0 + 1, done.1 + meta.len());
        } else {
            crate::daemon_log!("equipe: couldn't carry {rel} into {}", to.display());
        }
    }
    done
}

/// Since when the squad's folder of `store` has kept machine output out of
/// git: written now the first time it is asked.
fn since(store: &Store) -> u64 {
    let file = store.root().join(SINCE);
    if let Some(at) = std::fs::read_to_string(&file).ok().and_then(|t| t.trim().parse().ok()) {
        return at;
    }
    let at = now();
    let _ = std::fs::create_dir_all(store.root());
    let _ = std::fs::write(&file, at.to_string());
    at
}

/// Machine output in a squad's folder older than [`RAW_KEPT_DAYS`] — by its
/// time, or by when equipe started keeping it out of git there if that is
/// later — goes, with the folders it leaves empty. Only machine output in a
/// hand-in, never one git tracks, never one equipe's store keeps, never
/// what is past a hand-in's limit. What went, a line.
pub fn sweep(repo: &Path) -> Vec<String> {
    if !in_squad_folder(repo) {
        return Vec::new();
    }
    let Some(store) = Store::of(repo) else { return Vec::new() };
    let cutoff = now().saturating_sub(RAW_KEPT);
    if since(&store) > cutoff {
        return Vec::new();
    }
    let checkout = worktree::main_branch(repo).ok().and_then(|m| worktree::checked_out_at(repo, &m)).unwrap_or_else(|| repo.to_owned());
    let mut gone = (0usize, 0u64);
    let mut emptied: BTreeSet<PathBuf> = BTreeSet::new();
    for rel in kept_on_disk(&checkout) {
        if hand_in(&rel).is_none() || !is_machine(&rel) {
            continue;
        }
        let path = checkout.join(&rel);
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.is_file() && meta.modified().map(secs).unwrap_or(u64::MAX) < cutoff && std::fs::remove_file(&path).is_ok() {
            gone = (gone.0 + 1, gone.1 + meta.len());
            if let Some(parent) = path.parent() {
                emptied.insert(parent.to_owned());
            }
        }
    }
    let outputs = checkout.join(DIR);
    // Deepest first, each up to (not including) `outputs/`.
    let mut dirs: Vec<PathBuf> = emptied.into_iter().flat_map(|d| d.ancestors().take_while(|a| a.starts_with(&outputs) && *a != outputs).map(Path::to_owned).collect::<Vec<_>>()).collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    dirs.dedup();
    for dir in dirs {
        let _ = std::fs::remove_dir(&dir);
    }
    if gone.0 == 0 { Vec::new() } else { vec![format!("{} files of machine output older than {RAW_KEPT_DAYS} days ({}) left {}", gone.0, size_words(gone.1), outputs.display())] }
}

#[cfg(test)]
mod tests;
