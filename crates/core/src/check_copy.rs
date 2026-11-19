//! A disposable copy of a project for a member that checks work it may not
//! change — QA, a reviewer, any member on a ticket it doesn't edit — on Codex.
//!
//! Such a member's turn is read-only. On Claude Code that refuses the edit
//! tools and leaves its commands alone, so it can still run the project's
//! tests. Codex's `read-only` sandbox refuses every write the operating
//! system sees, so a test runner can't even make its cache
//! (`node_modules/.vite`, `.pytest_cache`) and nothing runs. And Codex's
//! `workspace-write` always makes the turn's working directory writable — on
//! a member that doesn't edit, the person's own checkout.
//!
//! So on Codex such a turn runs in a copy of the checkout, made here under
//! the lane's temporary folder: a `git clone --shared` (the person's objects
//! read in place, nothing registered in their repository) with every file of
//! the working tree cloned copy-on-write over it — uncommitted changes,
//! `node_modules`, `.env` and all — so it costs next to no room. The turn's
//! sandbox writes the copy and nothing of the person's: tests, builds,
//! caches and dev servers run; the source they check can't be changed where
//! it lives. A change to the copy's own source (tracked files, and new files
//! git doesn't ignore) is undone ([`undo_changes`]) as the turn goes and once
//! more at its end, and the member is told: what it reports is about the code
//! as it is. Caches and build output — what git ignores — stay.
//!
//! The copy is kept for the next turn while the checkout stands as it was,
//! and made again once it moves; it goes with the lane's temporary folder. It
//! is made only where the system clones copy-on-write (APFS, a reflink
//! filesystem): never a full copy. Elsewhere, or for a folder that isn't a
//! git checkout, the turn stays read-only, as before.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::worktree::{self, Result};

/// The copy's folder under a lane's temporary folder.
const FOLDER: &str = "checkout";
/// What the copy was made from and its source as made, in its `.git`.
const MARK: &str = "equipe-copy";
/// The scratch index holding the copy's source as made.
const BASELINE_INDEX: &str = "equipe-baseline-index";
/// The scratch index the source as it is now is read into.
const NOW_INDEX: &str = "equipe-now-index";
/// Paths handed to one `git checkout-index`.
const BATCH: usize = 200;

/// A copy ready for a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckCopy {
    /// The top of the copy (the repository's top, copied).
    pub root: PathBuf,
    /// The turn's working directory: the project's place inside the copy.
    pub cwd: PathBuf,
}

/// Where the copy for a lane whose temporary folder is `lane_tmp` goes.
pub fn dir_in(lane_tmp: &Path) -> PathBuf {
    lane_tmp.join(FOLDER)
}

/// Whether a copy can be made in `dir`: this system, on its filesystem there,
/// clones a file copy-on-write. Tried with a file of its own, removed after.
pub fn can_copy_in(dir: &Path) -> bool {
    if !worktree::CAN_CLONE || std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let (from, to) = (dir.join(".equipe-clone-probe"), dir.join(".equipe-clone-probe-copy"));
    let _ = std::fs::remove_file(&to);
    let cloned = std::fs::write(&from, b"probe").is_ok() && worktree::clone_tree(&from, &to).is_ok();
    let _ = std::fs::remove_file(&from);
    let _ = std::fs::remove_file(&to);
    cloned
}

/// The copy of `project`'s checkout at `at`: the one there when the
/// checkout still stands as it was when that was made, else a new one.
/// An error — not a git checkout, no copy-on-write here — means there is
/// none and the turn stays read-only. It runs git and clones: off the loop.
pub fn prepare(project: &Path, at: &Path) -> Result<CheckCopy> {
    if !worktree::CAN_CLONE {
        return Err("this system can't clone a folder copy-on-write".to_owned());
    }
    let repo = PathBuf::from(worktree::git(project, &["rev-parse", "--show-toplevel"]).map_err(|_| format!("{} isn't a git checkout", project.display()))?);
    let head = worktree::git(&repo, &["rev-parse", "--verify", "HEAD"]).map_err(|_| format!("{} has no commit yet", repo.display()))?;
    let print = fingerprint(&repo, &head)?;
    let cwd = inside(at, &repo, project);
    if read_mark(at).is_some_and(|(made_from, _)| made_from == print) {
        return Ok(CheckCopy { root: at.to_owned(), cwd });
    }
    match std::fs::remove_dir_all(at) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => return Err(format!("couldn't clear {}: {err}", at.display())),
        _ => {}
    }
    let made = make(&repo, &head, at).and_then(|()| baseline(at)).and_then(|tree| write_mark(at, &print, &tree));
    if let Err(err) = made {
        let _ = std::fs::remove_dir_all(at);
        return Err(err);
    }
    Ok(CheckCopy { root: at.to_owned(), cwd })
}

/// Undo what changed in the copy at `root`'s source since it was made: each
/// tracked file put back as it was, each new file git doesn't ignore
/// removed. What git ignores — caches, build output — stays. The paths
/// undone, relative to `root`; none when nothing changed. Off the loop.
pub fn undo_changes(root: &Path) -> Result<Vec<String>> {
    let (_, baseline) = read_mark(root).ok_or_else(|| format!("{} isn't a copy equipe made", root.display()))?;
    let mut undone: Vec<String> = Vec::new();
    // Files changed or removed are put back first, and the source read again
    // before a new file goes: a `.gitignore` taken away makes everything it
    // ignored (`node_modules`) look new, and only what is new with it back is.
    for _ in 0..3 {
        let now = source_tree(root, NOW_INDEX, true)?;
        if now == baseline {
            break;
        }
        let diff = worktree::git_bytes(root, &["diff-tree", "-r", "-z", "--no-renames", "--name-status", &baseline, &now], None, None)?;
        let (mut restore, mut added) = (Vec::new(), Vec::new());
        let mut fields = diff.split(|b| *b == 0).filter(|f| !f.is_empty());
        while let (Some(status), Some(path)) = (fields.next(), fields.next()) {
            let path = String::from_utf8_lossy(path).into_owned();
            if status.first() == Some(&b'A') { added.push(path) } else { restore.push(path) }
        }
        if restore.is_empty() {
            for path in &added {
                remove(&root.join(path));
            }
        }
        let index = git_dir(root).join(BASELINE_INDEX);
        for batch in restore.chunks(BATCH) {
            let mut args = vec!["checkout-index", "-f", "-q", "--"];
            args.extend(batch.iter().map(String::as_str));
            worktree::git_bytes(root, &args, Some(&index), None)?;
        }
        let done = if restore.is_empty() { added } else { restore };
        for path in done {
            if !undone.contains(&path) {
                undone.push(path);
            }
        }
    }
    Ok(undone)
}

/// What equipe tells a member whose change to the copy it undid, in the
/// turn: the files, and that it checks rather than changes.
pub fn undone_note(paths: &[String]) -> String {
    format!(
        "equipe undid your change to {}: on this ticket you check the work, you don't change it — those files are back as they \
were. Run the checks on the code as it is, and report what needs changing instead of changing it.",
        listed(paths)
    )
}

/// What equipe tells a member at the start of a turn in a copy.
pub fn framing(copy: &CheckCopy, project: &Path) -> String {
    format!(
        "Your commands run in a disposable copy of the project at `{}`; the project itself, `{}`, is read-only to you. Run its \
tests, builds, linters and dev servers in the copy — caches and build output may be written there. You check this ticket's \
work without changing it: any change to the copy's source is undone, and you are told.",
        copy.cwd.display(),
        project.display()
    )
}

/// Whether a tool the member used could have written a file: a command, an
/// edit, a write. What only reads, searches or talks to a server couldn't.
pub fn may_write(tool: &str) -> bool {
    matches!(tool, "Bash" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit")
}

/// `paths` for a sentence: the first few in backticks, then how many more.
pub fn listed(paths: &[String]) -> String {
    const SHOWN: usize = 5;
    let named: Vec<String> = paths.iter().take(SHOWN).map(|p| format!("`{p}`")).collect();
    match paths.len().saturating_sub(SHOWN) {
        0 => named.join(", "),
        more => format!("{} and {more} more", named.join(", ")),
    }
}

/// Make the copy of `repo` at `head` at `at`: a shared clone, its HEAD at
/// `head` and its index at HEAD, then every top-level entry of the working
/// tree but `.git` cloned in copy-on-write.
fn make(repo: &Path, head: &str, at: &Path) -> Result<()> {
    let parent = at.parent().ok_or_else(|| format!("{} has no parent", at.display()))?;
    std::fs::create_dir_all(parent).map_err(|err| format!("couldn't create {}: {err}", parent.display()))?;
    let (repo_s, at_s) = (repo.to_string_lossy(), at.to_string_lossy());
    // `-c core.hooksPath`: none of the person's hooks run in equipe's own git.
    worktree::git(parent, &["-c", "core.hooksPath=/dev/null", "clone", "--quiet", "--shared", "--no-checkout", "--", &repo_s, &at_s])?;
    worktree::git(at, &["update-ref", "--no-deref", "HEAD", head])?;
    worktree::git(at, &["reset", "--quiet"])?;
    // A Rust project's own `target` stays out: a member builds in its own
    // place (`CARGO_TARGET_DIR`), and it can be hundreds of thousands of files.
    let rust = repo.join("Cargo.toml").is_file();
    let entries = std::fs::read_dir(repo).map_err(|err| format!("couldn't read {}: {err}", repo.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == ".git" || (rust && name == "target") {
            continue;
        }
        worktree::clone_tree(&entry.path(), &at.join(&name)).map_err(|err| format!("couldn't clone {}: {err}", entry.path().display()))?;
    }
    Ok(())
}

/// Read the copy's source as made into the baseline index: its tree.
fn baseline(at: &Path) -> Result<String> {
    source_tree(at, BASELINE_INDEX, false)
}

/// The tree of the copy's source as it stands: HEAD's files and every file
/// git doesn't ignore, read into the scratch index `name`. `from_baseline`:
/// starting from the baseline index, so files unchanged since aren't read again.
fn source_tree(root: &Path, name: &str, from_baseline: bool) -> Result<String> {
    let dir = git_dir(root);
    let index = dir.join(name);
    let _ = std::fs::remove_file(&index);
    if from_baseline {
        std::fs::copy(dir.join(BASELINE_INDEX), &index).map_err(|err| format!("couldn't copy the baseline index: {err}"))?;
    } else {
        // From HEAD: a tracked file git would ignore is still read.
        worktree::git_bytes(root, &["read-tree", "HEAD"], Some(&index), None)?;
    }
    worktree::git_bytes(root, &["add", "--all"], Some(&index), None)?;
    let tree = worktree::git_bytes(root, &["write-tree"], Some(&index), None)?;
    Ok(String::from_utf8_lossy(&tree).trim().to_owned())
}

fn git_dir(root: &Path) -> PathBuf {
    root.join(".git")
}

fn read_mark(root: &Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(git_dir(root).join(MARK)).ok()?;
    let mut lines = text.lines();
    Some((lines.next()?.to_owned(), lines.next()?.to_owned()))
}

fn write_mark(root: &Path, print: &str, tree: &str) -> Result<()> {
    std::fs::write(git_dir(root).join(MARK), format!("{print}\n{tree}\n")).map_err(|err| format!("couldn't mark the copy: {err}"))
}

/// How `repo`'s checkout stands: its HEAD, what git says is changed in it,
/// and each changed file's size and time. Read without taking git's lock:
/// the person's checkout is only looked at.
fn fingerprint(repo: &Path, head: &str) -> Result<String> {
    let status = worktree::git_bytes(repo, &["--no-optional-locks", "status", "--porcelain=v1", "-z", "--untracked-files=all"], None, None)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    head.hash(&mut hasher);
    status.hash(&mut hasher);
    for entry in status.split(|b| *b == 0).filter(|e| e.len() > 3) {
        let path = repo.join(String::from_utf8_lossy(&entry[3..]).as_ref());
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            meta.len().hash(&mut hasher);
            meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).hash(&mut hasher);
        }
    }
    Ok(format!("{head}:{:016x}", hasher.finish()))
}

/// `project`'s place in the copy at `at` of the repository whose top is `repo`.
fn inside(at: &Path, repo: &Path, project: &Path) -> PathBuf {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
    match canon(project).strip_prefix(canon(repo)) {
        Ok(rel) if !rel.as_os_str().is_empty() => at.join(rel),
        _ => at.to_owned(),
    }
}

/// Remove what is at `path`, a link as itself.
fn remove(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            let _ = std::fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(path);
        }
        Err(_) => {}
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn scratch(name: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("equipe-check-copy-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    /// A web project: tracked source, an ignored `node_modules`, and an
    /// uncommitted change of the person's.
    fn project() -> PathBuf {
        let repo = scratch("repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo.join(".gitignore"), "node_modules/\n").unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/app.js"), "export const answer = 42;\n").unwrap();
        std::fs::create_dir_all(repo.join("node_modules/vitest")).unwrap();
        std::fs::write(repo.join("node_modules/vitest/index.js"), "// vitest\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        std::fs::write(repo.join("src/app.js"), "export const answer = 43;\n").unwrap();
        repo
    }

    /// Only where this filesystem clones copy-on-write (CI's may not).
    fn copy_of(repo: &Path) -> Option<CheckCopy> {
        match prepare(repo, &dir_in(&scratch("lane"))) {
            Ok(copy) => Some(copy),
            Err(why) if why.starts_with("couldn't clone") => {
                eprintln!("skipped: {why}");
                None
            }
            Err(why) => panic!("{why}"),
        }
    }

    #[test]
    fn the_copy_holds_the_checkout_as_it_stands_and_its_writes_never_reach_it() {
        let repo = project();
        let Some(copy) = copy_of(&repo) else { return };
        assert_eq!(copy.cwd, copy.root);
        // The person's uncommitted change, the ignored dependencies, git's view of both.
        assert_eq!(std::fs::read_to_string(copy.root.join("src/app.js")).unwrap(), "export const answer = 43;\n");
        assert!(copy.root.join("node_modules/vitest/index.js").is_file());
        assert_eq!(git(&copy.root, &["rev-parse", "HEAD"]), git(&repo, &["rev-parse", "HEAD"]));
        assert_eq!(git(&copy.root, &["status", "--porcelain"]), "M src/app.js");
        // A cache written in the copy, and a change to its source: the person's checkout has neither.
        std::fs::create_dir_all(copy.root.join("node_modules/.vite/vitest")).unwrap();
        std::fs::write(copy.root.join("node_modules/.vite/vitest/results.json"), "{}").unwrap();
        std::fs::write(copy.root.join("src/app.js"), "export const answer = 0;\n").unwrap();
        assert!(!repo.join("node_modules/.vite").exists());
        assert_eq!(std::fs::read_to_string(repo.join("src/app.js")).unwrap(), "export const answer = 43;\n");
        // Nothing of the copy is registered in the person's repository.
        assert_eq!(worktree::listed(&repo).len(), 1);
    }

    #[test]
    fn a_change_to_the_copys_source_is_undone_and_its_caches_stay() {
        let repo = project();
        let Some(copy) = copy_of(&repo) else { return };
        assert_eq!(undo_changes(&copy.root).unwrap(), Vec::<String>::new(), "nothing changed yet");
        std::fs::write(copy.root.join("src/app.js"), "export const answer = 0;\n").unwrap();
        std::fs::write(copy.root.join("src/new.js"), "new\n").unwrap();
        std::fs::remove_file(copy.root.join(".gitignore")).unwrap();
        std::fs::create_dir_all(copy.root.join("node_modules/.vite")).unwrap();
        std::fs::write(copy.root.join("node_modules/.vite/cache"), "warm").unwrap();
        let mut undone = undo_changes(&copy.root).unwrap();
        undone.sort();
        assert_eq!(undone, [".gitignore", "src/app.js", "src/new.js"], "the ignore file's removal exposed nothing else for good");
        // Back as the copy was made — the person's uncommitted change included.
        assert_eq!(std::fs::read_to_string(copy.root.join("src/app.js")).unwrap(), "export const answer = 43;\n");
        assert!(!copy.root.join("src/new.js").exists());
        assert_eq!(std::fs::read_to_string(copy.root.join(".gitignore")).unwrap(), "node_modules/\n");
        assert_eq!(std::fs::read_to_string(copy.root.join("node_modules/.vite/cache")).unwrap(), "warm");
        assert_eq!(undo_changes(&copy.root).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn the_copy_is_kept_while_the_checkout_stands_and_made_again_once_it_moves() {
        let repo = project();
        let lane = scratch("lane");
        let Ok(first) = prepare(&repo, &dir_in(&lane)) else { return };
        std::fs::write(first.root.join("node_modules/.warm"), "yes").unwrap();
        let again = prepare(&repo, &dir_in(&lane)).unwrap();
        assert!(again.root.join("node_modules/.warm").exists(), "kept: nothing moved");
        std::fs::write(repo.join("src/app.js"), "export const answer = 44;\n").unwrap();
        let moved = prepare(&repo, &dir_in(&lane)).unwrap();
        assert!(!moved.root.join("node_modules/.warm").exists(), "made again");
        assert_eq!(std::fs::read_to_string(moved.root.join("src/app.js")).unwrap(), "export const answer = 44;\n");
    }

    #[test]
    fn a_project_in_a_subfolder_runs_in_its_place_in_the_copy_and_a_plain_folder_gets_none() {
        let repo = project();
        let Some(copy) = copy_of(&repo.join("src")) else { return };
        assert_eq!(copy.cwd, copy.root.join("src"));
        let plain = scratch("plain");
        assert!(prepare(&plain, &dir_in(&scratch("lane"))).unwrap_err().contains("isn't a git checkout"));
    }

    #[test]
    fn what_the_member_is_told_names_the_files() {
        let paths: Vec<String> = (1..=7).map(|n| format!("src/{n}.js")).collect();
        assert_eq!(listed(&paths[..1]), "`src/1.js`");
        assert!(listed(&paths).ends_with("`src/5.js` and 2 more"));
        assert!(undone_note(&paths[..2]).starts_with("equipe undid your change to `src/1.js`, `src/2.js`"));
        assert!(may_write("Bash") && may_write("Edit") && may_write("Write"));
        assert!(!may_write("Read") && !may_write("Grep") && !may_write("mcp__equipe__board"));
    }
}
