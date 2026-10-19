//! Isolated workspaces for agents that edit code.
//!
//! An agent never works in the user's checkout. It gets a fresh git worktree on a
//! new branch, cut from `HEAD`, under equipe's home rather than inside the repo
//! (so `git status` in the user's checkout stays clean). When it finishes, its
//! changes are committed to that branch and the worktree is removed: what remains
//! is one reviewable branch.
//!
//! A member gets one per ticket, kept while the work is in review so it can be
//! reopened, and checked out again from its branch if it comes back later;
//! `equipe try --edit` uses one for a single turn.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

pub type Result<T> = std::result::Result<T, String>;

/// The longest one of equipe's local git commands may take before it is
/// stopped: each takes milliseconds on a healthy machine, and the daemon must
/// never wait on one for good (a full disk, a lock, a prompt nobody sees).
pub const GIT_TIMEOUT: Duration = Duration::from_secs(60);
/// A push talks to a server, which may be slow.
const PUSH_TIMEOUT: Duration = Duration::from_secs(180);

static GIT: OnceLock<PathBuf> = OnceLock::new();

/// Run this git instead of the one on `PATH` — for tests that need a slow or
/// a failing one. Set once per process.
#[doc(hidden)]
pub fn use_git(program: PathBuf) {
    let _ = GIT.set(program);
}

/// A git command for `dir`.
pub(crate) fn git_command(dir: &Path) -> Command {
    let mut command = Command::new(GIT.get().map_or(Path::new("git"), PathBuf::as_path));
    command.arg("-C").arg(dir);
    // Nobody is at the daemon's terminal: a prompt for credentials fails rather than waits.
    command.env("GIT_TERMINAL_PROMPT", "0");
    command
}

/// What `command` printed, once it ends — or an error once `timeout` passes,
/// the command and whatever it started killed. Its standard input is empty.
pub(crate) fn output_within(mut command: Command, timeout: Duration) -> Result<Output> {
    use std::os::unix::process::CommandExt as _;
    let program = command.get_program().to_string_lossy().into_owned();
    // A group of its own, so a timeout stops what it started too (a push's ssh).
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    let child = command.spawn().map_err(|err| format!("couldn't run {program}: {err}"))?;
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(timeout) {
        Ok(out) => out.map_err(|err| format!("couldn't run {program}: {err}")),
        Err(_) => {
            crate::group::end(pid, libc::SIGKILL);
            let args: Vec<String> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
            Err(format!("{program} {} took more than {}s and was stopped", args.join(" "), timeout.as_secs()))
        }
    }
}

/// For what may take long — a merge worked out per ticket, a push — and so
/// must run off the daemon's core loop (`spawn_blocking`): there it would hold
/// up every screen (docs/INCIDENT-2026-10-16-daemon-hang.md). A debug build
/// stops at once when it is called there.
fn off_the_loop(what: &str) {
    debug_assert!(!crate::runtime::on_core_loop(), "{what} runs git and must not run on the core loop: spawn_blocking it");
}

/// Run git in `dir` and return trimmed stdout, or stderr as the error.
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    git_within(dir, args, GIT_TIMEOUT)
}

fn git_within(dir: &Path, args: &[&str], timeout: Duration) -> Result<String> {
    let mut command = git_command(dir);
    command.args(args);
    let out = output_within(command, timeout)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(format!("git {} failed: {stderr}", args.join(" ")))
    }
}

/// The top of the git repository containing `dir`.
pub fn repo_root(dir: &Path) -> Result<PathBuf> {
    git(dir, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .map_err(|_| "not inside a git repository — `--edit` works on a branch of one".to_owned())
}

/// The folder under `worktrees/` a repository's worktrees go in: its own name —
/// or, for a squad home (every one of them is called `home`), `<squad>.home`.
fn folder_name(repo: &Path) -> String {
    let name = repo.file_name().map_or_else(|| "repo".into(), |n| n.to_string_lossy().into_owned());
    let squads = repo.parent().and_then(Path::parent).and_then(Path::file_name);
    match repo.parent().and_then(Path::file_name) {
        Some(squad) if name == "home" && squads.is_some_and(|s| s == "squads") => format!("{}.home", squad.to_string_lossy()),
        _ => name,
    }
}

/// Where `repo`'s worktrees are under `home`: one folder each, by its id.
pub fn worktrees_of(repo: &Path, home: &Path) -> PathBuf {
    home.join("worktrees").join(folder_name(repo))
}

/// Why a file couldn't be had from git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blob {
    /// `rev` has no such file (or there is no `rev`).
    Missing,
    /// It is bigger than was asked for at most.
    TooBig(u64),
    Failed(String),
}

/// The bytes of `rel` (from the top of `repo`) as `rev` has it — a ticket's
/// branch, the main branch — when no bigger than `max`. Runs git: off the loop.
pub fn read_blob(repo: &Path, rev: &str, rel: &Path, max: u64) -> std::result::Result<Vec<u8>, Blob> {
    off_the_loop("read_blob");
    let rel = rel.to_string_lossy();
    if rev.is_empty() || rev.starts_with('-') || rev.contains(':') || rel.starts_with('/') || rel.split('/').any(|p| p == "..") {
        return Err(Blob::Missing);
    }
    let object = format!("{rev}:{rel}");
    let size = git(repo, &["cat-file", "-s", &object]).map_err(|_| Blob::Missing)?;
    let size: u64 = size.parse().map_err(|_| Blob::Failed(format!("git gave {size} as its size")))?;
    if size > max {
        return Err(Blob::TooBig(size));
    }
    // Only a file: a folder of that name is no picture.
    if git(repo, &["cat-file", "-t", &object]).map_err(|_| Blob::Missing)? != "blob" {
        return Err(Blob::Missing);
    }
    let mut command = git_command(repo);
    command.args(["cat-file", "blob", &object]);
    let out = output_within(command, GIT_TIMEOUT).map_err(Blob::Failed)?;
    if !out.status.success() {
        return Err(Blob::Failed(String::from_utf8_lossy(&out.stderr).trim().to_owned()));
    }
    Ok(out.stdout)
}

#[derive(Debug, Clone)]
pub struct Sandbox {
    pub repo: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    /// The commit the branch was cut from.
    pub base: String,
}

/// Cut a new branch from `HEAD` — `equipe/<label>-<id>` — and check it out in a
/// worktree under `home`.
pub fn create(repo: &Path, home: &Path, id: &str, label: &str) -> Result<Sandbox> {
    let base = git(repo, &["rev-parse", "HEAD"])
        .map_err(|_| "the repository has no commits yet — commit something first".to_owned())?;
    let path = home.join("worktrees").join(folder_name(repo)).join(id);
    let branch = format!("equipe/{label}-{id}");
    std::fs::create_dir_all(path.parent().expect("worktree path has a parent"))
        .map_err(|err| format!("couldn't create {}: {err}", path.display()))?;
    git(
        repo,
        &["worktree", "add", "-q", "-b", &branch, &path.to_string_lossy(), &base],
    )?;
    Ok(Sandbox {
        repo: repo.to_owned(),
        path,
        branch,
        base,
    })
}

/// Check out an existing branch again — a reopened ticket whose worktree was
/// removed — in a fresh worktree under `home`.
pub fn resume(repo: &Path, home: &Path, id: &str, branch: &str, base: &str) -> Result<Sandbox> {
    let path = home.join("worktrees").join(folder_name(repo)).join(id);
    std::fs::create_dir_all(path.parent().expect("worktree path has a parent"))
        .map_err(|err| format!("couldn't create {}: {err}", path.display()))?;
    git(repo, &["worktree", "add", "-q", &path.to_string_lossy(), branch])?;
    Ok(Sandbox { repo: repo.to_owned(), path, branch: branch.to_owned(), base: base.to_owned() })
}

/// Cut a ticket's branch from `HEAD` with the work it builds on already in it:
/// each of `after` — `(what, branch)`, the finished tickets it depends on — is
/// merged in unless `HEAD` has it. Their work may be accepted but not merged
/// yet; the ticket must not start without it. When two of them conflict, nothing
/// is left behind and the error names them.
pub fn create_after(repo: &Path, home: &Path, id: &str, label: &str, after: &[(String, String)]) -> Result<Sandbox> {
    let mut sandbox = create(repo, home, id, label)?;
    for (what, branch) in after {
        if git(repo, &["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")]).is_err()
            || is_ancestor(&sandbox.path, branch, "HEAD")
        {
            continue;
        }
        let message = format!("equipe: start from {what} ({branch})");
        if let Err(err) = with_identity(&sandbox.path, &["merge", "-q", "--no-ff", "--no-edit", "-m", &message, branch]) {
            let files = git(&sandbox.path, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
            let _ = git(&sandbox.path, &["merge", "--abort"]);
            let _ = sandbox.discard();
            return Err(if files.is_empty() {
                format!("couldn't bring in {what} ({branch}): {err}")
            } else {
                format!("{what} ({branch}) conflicts with the work before it in {}", files.lines().collect::<Vec<_>>().join(", "))
            });
        }
    }
    sandbox.base = git(&sandbox.path, &["rev-parse", "HEAD"])?;
    Ok(sandbox)
}

/// Whether `commit` is already contained in `into`.
fn is_ancestor(dir: &Path, commit: &str, into: &str) -> bool {
    git(dir, &["merge-base", "--is-ancestor", commit, into]).is_ok()
}

/// The branch accepted work lands on: the remote's default branch, else `main`,
/// else `master`.
pub fn main_branch(repo: &Path) -> Result<String> {
    if let Ok(head) = git(repo, &["symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD"])
        && let Some(name) = head.strip_prefix("origin/")
        && git(repo, &["rev-parse", "--verify", "-q", &format!("refs/heads/{name}")]).is_ok()
    {
        return Ok(name.to_owned());
    }
    ["main", "master"]
        .into_iter()
        .find(|name| git(repo, &["rev-parse", "--verify", "-q", &format!("refs/heads/{name}")]).is_ok())
        .map(str::to_owned)
        .ok_or_else(|| "the repository has neither a main nor a master branch".to_owned())
}

/// Whether `branch` has nothing left to bring to the main branch: it is in
/// main already, or merging it would change nothing there (`+0 −0` — its
/// changes landed another way, or it made none).
pub fn is_merged(repo: &Path, branch: &str) -> Result<bool> {
    off_the_loop("is_merged");
    let main = main_branch(repo)?;
    Ok(is_ancestor(repo, branch, &main) || adds_nothing(repo, branch, &main))
}

/// Whether merging `branch` into `into` leaves `into`'s files as they are.
fn adds_nothing(repo: &Path, branch: &str, into: &str) -> bool {
    let Ok(tree) = git(repo, &["rev-parse", &format!("refs/heads/{into}^{{tree}}")]) else {
        return false;
    };
    // A conflict exits 1 and a failure otherwise: neither is "nothing".
    git(repo, &["merge-tree", "--write-tree", "--no-messages", &format!("refs/heads/{into}"), branch])
        .is_ok_and(|out| out.lines().next().is_some_and(|merged| merged.trim() == tree))
}

/// The tip of each of a repository's branches, by name (`main`, `equipe/x-1`).
fn branch_tips(repo: &Path) -> Result<HashMap<String, String>> {
    let out = git(repo, &["for-each-ref", "--format=%(objectname) %(refname:short)", "refs/heads/"])?;
    Ok(out.lines().filter_map(|l| l.split_once(' ')).map(|(sha, name)| (name.to_owned(), sha.to_owned())).collect())
}

/// [`is_merged`] for many branches, each answer remembered for as long as
/// neither the branch nor main moves: asked every minute, it runs a few git
/// commands per repository, and works a merge out only for a branch that moved.
#[derive(Default)]
pub struct MergedCache {
    /// By repository and branch: the branch's tip and main's when it was
    /// told, and whether it was merged then.
    told: HashMap<(PathBuf, String), (String, String, bool)>,
}

impl MergedCache {
    /// For each of `branches` of the repository `project` is in: whether it
    /// has nothing left to bring to main; `None` when that can't be told.
    pub fn merged(&mut self, project: &Path, branches: &[String]) -> Vec<Option<bool>> {
        off_the_loop("MergedCache::merged");
        let known = repo_root(project).and_then(|repo| Ok((main_branch(&repo)?, branch_tips(&repo)?, repo)));
        let Ok((main, tips, repo)) = known else {
            return vec![None; branches.len()];
        };
        let Some(main_tip) = tips.get(&main).cloned() else {
            return vec![None; branches.len()];
        };
        self.told.retain(|(r, b), _| *r != repo || tips.contains_key(b));
        branches
            .iter()
            .map(|branch| {
                let tip = tips.get(branch)?;
                let key = (repo.clone(), branch.clone());
                if let Some((t, m, merged)) = self.told.get(&key)
                    && t == tip
                    && *m == main_tip
                {
                    return Some(*merged);
                }
                let merged = is_ancestor(&repo, branch, &main) || adds_nothing(&repo, branch, &main);
                self.told.insert(key, (tip.clone(), main_tip.clone(), merged));
                Some(merged)
            })
            .collect()
    }

    /// How many answers it keeps.
    pub fn len(&self) -> usize {
        self.told.len()
    }

    pub fn is_empty(&self) -> bool {
        self.told.is_empty()
    }
}

/// Merge `branch` into the main branch with a merge commit, never touching a
/// conflict: the merge is worked out apart from any checkout, and only a clean
/// one moves the branch. A checkout that has main checked out follows along
/// (fast-forward), keeping its uncommitted changes — or refuses, and then
/// nothing changes. Returns the main branch's name; `Ok` with nothing done when
/// `branch` is in it already, or would change nothing in it.
pub fn merge_into_main(repo: &Path, branch: &str, message: &str) -> Result<String> {
    let main = main_branch(repo)?;
    if is_ancestor(repo, branch, &main) || adds_nothing(repo, branch, &main) {
        return Ok(main);
    }
    let tip = git(repo, &["rev-parse", &format!("refs/heads/{main}")])?;
    let mut command = git_command(repo);
    command.args(["merge-tree", "--write-tree", "--name-only", "--no-messages", &tip, branch]);
    let out = output_within(command, GIT_TIMEOUT)?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines = stdout.lines();
    let tree = lines.next().unwrap_or("").trim().to_owned();
    match out.status.code() {
        Some(0) => {}
        Some(1) => {
            let files: Vec<&str> = lines.take_while(|l| !l.is_empty()).collect();
            return Err(format!("{branch} conflicts with {main} in {} — nothing was merged", files.join(", ")));
        }
        _ => return Err(format!("git merge-tree failed: {}", String::from_utf8_lossy(&out.stderr).trim())),
    }
    let commit = with_identity(repo, &["commit-tree", &tree, "-p", &tip, "-p", branch, "-m", message])?;
    let checked_out = git(repo, &["worktree", "list", "--porcelain"])?
        .split("\n\n")
        .find(|entry| entry.lines().any(|l| l == format!("branch refs/heads/{main}")))
        .and_then(|entry| entry.lines().find_map(|l| l.strip_prefix("worktree ")).map(PathBuf::from));
    match checked_out {
        Some(path) => git(&path, &["merge", "-q", "--ff-only", &commit]).map_err(|err| {
            format!("{main} is checked out in {} and couldn't move without touching its changes — nothing was merged ({err})", path.display())
        })?,
        None => git(repo, &["update-ref", &format!("refs/heads/{main}"), &commit, &tip])?,
    };
    Ok(main)
}

/// Push the main branch to `origin`.
pub fn push_main(repo: &Path) -> Result<String> {
    off_the_loop("push_main");
    if !has_origin(repo) {
        return Err(NO_ORIGIN.into());
    }
    let main = main_branch(repo)?;
    git_within(repo, &["push", "-q", "origin", &main], PUSH_TIMEOUT)?;
    Ok(main)
}

/// What `push_main` says when the repository has nowhere to push to: a
/// local-only project, merged and kept here, not a failure.
pub const NO_ORIGIN: &str = "no remote named origin";

/// Whether `repo` has a remote named `origin` to push main to.
pub fn has_origin(repo: &Path) -> bool {
    git(repo, &["remote", "get-url", "origin"]).is_ok_and(|url| !url.trim().is_empty())
}

/// Run a git command that makes a commit, as the user — or as equipe when no
/// identity is configured on this machine.
fn with_identity(dir: &Path, args: &[&str]) -> Result<String> {
    git(dir, args).or_else(|_| {
        // A failed merge must not be retried over its own leftovers. Only ever
        // in a worktree of equipe's: `dir` is one whenever `args` merge.
        if args.first() == Some(&"merge") {
            let _ = git(dir, &["merge", "--abort"]);
        }
        let mut with: Vec<&str> = FALLBACK_IDENTITY.to_vec();
        with.extend(args);
        git(dir, &with)
    })
}

/// Who a commit is attributed to when the user's git identity isn't configured.
const FALLBACK_IDENTITY: [&str; 4] = ["-c", "user.name=equipe", "-c", "user.email=equipe@users.noreply.local"];

impl Sandbox {
    /// Commit everything the agent changed. `Ok(None)` when it changed nothing.
    pub fn commit_all(&self, message: &str) -> Result<Option<String>> {
        git(&self.path, &["add", "-A"])?;
        if git(&self.path, &["diff", "--cached", "--quiet"]).is_ok() {
            return Ok(None);
        }
        let commit = ["commit", "-q", "-m", message];
        if git(&self.path, &commit).is_err() {
            // Most often: no user.name/user.email on this machine. Hooks that
            // reject the commit fail again here and surface their message.
            let mut with_identity: Vec<&str> = FALLBACK_IDENTITY.to_vec();
            with_identity.extend(commit);
            git(&self.path, &with_identity)?;
        }
        git(&self.path, &["rev-parse", "HEAD"]).map(Some)
    }

    /// `git diff --stat` of the branch against where it started.
    pub fn diff_stat(&self) -> Result<String> {
        git(&self.repo, &["diff", "--stat", &format!("{}..{}", self.base, self.branch)])
    }

    /// The files the branch changed since where it started, by path.
    pub fn changed_files(&self) -> Result<Vec<String>> {
        let out = git(&self.repo, &["-c", "core.quotePath=false", "diff", "--name-only", &format!("{}..{}", self.base, self.branch)])?;
        Ok(out.lines().filter(|l| !l.is_empty() && !l.ends_with(".gitkeep")).map(str::to_owned).collect())
    }

    /// Whether the branch has any commit beyond where it started.
    pub fn has_commits(&self) -> bool {
        git(&self.repo, &["rev-list", "--count", &format!("{}..{}", self.base, self.branch)])
            .is_ok_and(|n| n != "0")
    }

    /// Remove the worktree directory; the branch stays.
    pub fn remove_worktree(&self) -> Result<()> {
        git(&self.repo, &["worktree", "remove", "--force", &self.path.to_string_lossy()]).map(|_| ())
    }

    /// Remove the branch as well, for a run that produced nothing.
    pub fn discard(&self) -> Result<()> {
        self.remove_worktree()?;
        git(&self.repo, &["branch", "-q", "-D", &self.branch]).map(|_| ())
    }
}

/// Give back the space tickets' worktrees hold. In a worktree no one works in
/// now, build output (`target/`, rebuilt when wanted) goes; a worktree no
/// ticket keeps either — its work landed, or it was dropped — goes too, once
/// nothing in it is uncommitted (its branch stays). `in_use` and `kept` are
/// worktree paths. Returns what it did, one line each.
pub fn tidy(home: &Path, in_use: &[PathBuf], kept: &[PathBuf]) -> Vec<String> {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
    let in_use: Vec<PathBuf> = in_use.iter().map(|p| canon(p)).collect();
    let kept: Vec<PathBuf> = kept.iter().map(|p| canon(p)).collect();
    let mut done = Vec::new();
    let Ok(repos) = std::fs::read_dir(home.join("worktrees")) else {
        return done;
    };
    for dir in repos.flatten().flat_map(|r| std::fs::read_dir(r.path()).into_iter().flatten().flatten()) {
        let path = canon(&dir.path());
        // Only a worktree: its `.git` is a file pointing back at its repository.
        if !path.join(".git").is_file() || in_use.iter().any(|u| u.starts_with(&path) || path.starts_with(u)) {
            continue;
        }
        let target = path.join("target");
        if target.is_dir() && std::fs::remove_dir_all(&target).is_ok() {
            done.push(format!("cleared {}", target.display()));
        }
        if kept.contains(&path) || !git(&path, &["status", "--porcelain"]).is_ok_and(|s| s.is_empty()) {
            continue;
        }
        let Ok(common) = git(&path, &["rev-parse", "--path-format=absolute", "--git-common-dir"]) else {
            continue;
        };
        let repo = Path::new(&common).parent().map(Path::to_owned).unwrap_or_default();
        if git(&repo, &["worktree", "remove", "--force", &path.to_string_lossy()]).is_ok() {
            done.push(format!("removed {}", path.display()));
        }
    }
    done
}

/// The bytes under `path`, links not followed.
fn size_of(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path).into_iter().flatten().flatten().map(|e| size_of(&e.path())).sum()
}

/// Members' build caches (`~/.equipe/cache/build/<squad>/<member>`) kept
/// under `cap` bytes each: past it, a member not building now (`busy` are
/// those that may be) loses its incremental state first — what grows without
/// end — and, still past it, the whole cache, which its next build makes anew.
pub fn trim_builds(home: &Path, busy: &[PathBuf], cap: u64) -> Vec<String> {
    let mut done = Vec::new();
    let Ok(squads) = std::fs::read_dir(home.join("cache").join("build")) else {
        return done;
    };
    for dir in squads.flatten().flat_map(|s| std::fs::read_dir(s.path()).into_iter().flatten().flatten()) {
        let path = dir.path();
        if busy.contains(&path) || size_of(&path) <= cap {
            continue;
        }
        // `debug/incremental`, `release/incremental`, `<target>/debug/incremental`…
        let profiles = std::fs::read_dir(&path).into_iter().flatten().flatten().flat_map(|p| {
            let nested: Vec<PathBuf> = std::fs::read_dir(p.path()).into_iter().flatten().flatten().map(|q| q.path()).collect();
            std::iter::once(p.path()).chain(nested)
        });
        for incremental in profiles.map(|p| p.join("incremental")).filter(|p| p.is_dir()).collect::<Vec<_>>() {
            if std::fs::remove_dir_all(&incremental).is_ok() {
                done.push(format!("cleared {}", incremental.display()));
            }
        }
        if size_of(&path) > cap && std::fs::remove_dir_all(&path).is_ok() {
            done.push(format!("cleared {}", path.display()));
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_members_build_cache_past_its_cap_loses_incremental_state_then_all_unless_it_builds() {
        let home = scratch("builds");
        let member = |name: &str| home.join("cache").join("build").join("acme").join(name);
        let fill = |dir: &Path, bytes: usize| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("blob"), vec![0u8; bytes]).unwrap();
        };
        // Mostly incremental state: trimming it is enough.
        fill(&member("frontend").join("debug").join("incremental"), 900);
        fill(&member("frontend").join("debug").join("deps"), 100);
        // Past the cap without it: all of it goes.
        fill(&member("backend").join("debug").join("deps"), 900);
        // Building now: untouched.
        fill(&member("pm").join("debug").join("incremental"), 900);
        let done = trim_builds(&home, &[member("pm")], 500);
        assert!(!member("frontend").join("debug").join("incremental").exists() && member("frontend").join("debug").join("deps").exists(), "{done:?}");
        assert!(!member("backend").exists(), "{done:?}");
        assert!(member("pm").join("debug").join("incremental").exists());
    }

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// A fresh directory; a counter, not the clock, keeps parallel tests apart.
    fn scratch(name: &str) -> PathBuf {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("equipe-wt-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A repository with one commit and a configured identity.
    fn repo() -> PathBuf {
        let dir = scratch("repo");
        git(&dir, &["init", "-q", "-b", "main"]).unwrap();
        git(&dir, &["config", "user.name", "Test"]).unwrap();
        git(&dir, &["config", "user.email", "test@example.com"]).unwrap();
        std::fs::write(dir.join("README.md"), "hello\n").unwrap();
        git(&dir, &["add", "-A"]).unwrap();
        git(&dir, &["commit", "-q", "-m", "init"]).unwrap();
        dir
    }

    #[test]
    fn idle_worktrees_give_back_their_builds_and_finished_ones_go() {
        let repo = repo();
        let home = scratch("tidy-home");
        let working = create(&repo, &home, "t1-a", "x").unwrap();
        let waiting = create(&repo, &home, "t2-b", "x").unwrap();
        let landed = create(&repo, &home, "t3-c", "x").unwrap();
        let unsaved = create(&repo, &home, "t4-d", "x").unwrap();
        for s in [&working, &waiting, &landed, &unsaved] {
            std::fs::create_dir_all(s.path.join("target/debug")).unwrap();
            std::fs::write(s.path.join("target/debug/app"), "binary").unwrap();
        }
        // An untracked target/ isn't a change worth keeping, but this is.
        std::fs::write(unsaved.path.join(".gitignore"), "target\n").unwrap();
        for s in [&working, &waiting, &landed] {
            std::fs::write(s.path.join(".gitignore"), "target\n").unwrap();
            git(&s.path, &["add", "-A"]).unwrap();
            git(&s.path, &["commit", "-q", "-m", "ignore builds"]).unwrap();
        }

        let done = tidy(&home, std::slice::from_ref(&working.path), std::slice::from_ref(&waiting.path));
        assert!(working.path.join("target/debug/app").exists(), "the one at work keeps everything");
        assert!(waiting.path.exists() && !waiting.path.join("target").exists(), "kept, its build given back");
        assert!(!landed.path.exists(), "no ticket keeps it: gone");
        assert!(git(&repo, &["rev-parse", "--verify", "-q", &landed.branch]).is_ok(), "its branch stays");
        assert!(unsaved.path.exists() && !unsaved.path.join("target").exists(), "uncommitted work stays");
        assert_eq!(done.len(), 4, "{done:?}");
        assert!(tidy(&home, std::slice::from_ref(&working.path), std::slice::from_ref(&waiting.path)).is_empty(), "nothing more");
    }

    #[test]
    fn a_squad_homes_worktrees_are_named_by_its_squad() {
        assert_eq!(folder_name(Path::new("/h/.equipe/squads/acme/home")), "acme.home");
        assert_eq!(folder_name(Path::new("/code/site")), "site");
        assert_eq!(folder_name(Path::new("/code/home")), "home");
    }

    #[test]
    fn finds_the_repository_root_from_a_subdirectory() {
        let repo = repo();
        std::fs::create_dir_all(repo.join("a/b")).unwrap();
        let root = repo_root(&repo.join("a/b")).unwrap();
        assert_eq!(root.canonicalize().unwrap(), repo.canonicalize().unwrap());
    }

    #[test]
    fn a_branch_whose_worktree_was_removed_can_be_checked_out_again() {
        let repo = repo();
        let home = scratch("home");
        let sandbox = create(&repo, &home, "t5", "frontend").unwrap();
        std::fs::write(sandbox.path.join("A.md"), "a").unwrap();
        sandbox.commit_all("a").unwrap();
        assert!(sandbox.has_commits());
        sandbox.remove_worktree().unwrap();
        let again = resume(&repo, &home, "t5-again", &sandbox.branch, &sandbox.base).unwrap();
        assert_eq!(std::fs::read_to_string(again.path.join("A.md")).unwrap(), "a");
        assert_eq!(again.branch, "equipe/frontend-t5");
    }

    #[test]
    fn a_command_that_never_ends_is_stopped_at_its_time_with_what_it_started() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & sleep 30"]);
        let started = std::time::Instant::now();
        let err = output_within(command, Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("took more than"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        let mut quick = Command::new("echo");
        quick.arg("hi");
        assert_eq!(String::from_utf8_lossy(&output_within(quick, Duration::from_secs(10)).unwrap().stdout), "hi\n");
    }

    #[test]
    fn the_merged_sweep_answers_anew_when_main_or_a_branch_moves() {
        let repo = repo();
        let home = scratch("sweep-home");
        let made = |id: &str, file: &str| {
            let s = create(&repo, &home, id, "dev").unwrap();
            std::fs::write(s.path.join(file), "x\n").unwrap();
            s.commit_all(id).unwrap();
            s.remove_worktree().unwrap();
            s.branch
        };
        let (one, two) = (made("t1", "A.md"), made("t2", "B.md"));
        let mut cache = MergedCache::default();
        let both = [one.clone(), two.clone(), "equipe/gone".to_owned()];
        assert_eq!(cache.merged(&repo, &both), [Some(false), Some(false), None]);
        assert_eq!(cache.len(), 2);
        // Main takes one: both are asked again (main moved), and the answers change.
        merge_into_main(&repo, &one, "merge t1").unwrap();
        assert_eq!(cache.merged(&repo, &both), [Some(true), Some(false), None]);
        // Nothing moved: the remembered answers, the same.
        assert_eq!(cache.merged(&repo, &both), [Some(true), Some(false), None]);
        // Outside a repository: nothing can be told.
        assert_eq!(cache.merged(&scratch("not-a-repo"), &both[..1]), [None]);
    }

    #[test]
    fn a_branch_with_nothing_left_to_bring_counts_as_merged() {
        let repo = repo();
        let home = scratch("nothing-home");
        let branch = |name: &str, file: &str, text: &str| {
            let s = create(&repo, &home, name, "frontend").unwrap();
            std::fs::write(s.path.join(file), text).unwrap();
            s.commit_all(name).unwrap();
            s.remove_worktree().unwrap();
            s.branch
        };
        // Work of its own: not merged until it is.
        let real = branch("t1", "A.md", "a\n");
        assert_eq!(is_merged(&repo, &real), Ok(false));
        // In main already: an ancestor.
        merge_into_main(&repo, &real, "merge t1").unwrap();
        assert_eq!(is_merged(&repo, &real), Ok(true));
        // A branch that changed something and changed it back: +0 −0, not an ancestor.
        let s = create(&repo, &home, "t2", "frontend").unwrap();
        std::fs::write(s.path.join("B.md"), "b\n").unwrap();
        s.commit_all("add b").unwrap();
        std::fs::remove_file(s.path.join("B.md")).unwrap();
        s.commit_all("drop b").unwrap();
        s.remove_worktree().unwrap();
        assert!(git(&repo, &["merge-base", "--is-ancestor", &s.branch, "main"]).is_err());
        assert_eq!(is_merged(&repo, &s.branch), Ok(true), "merging it would change nothing");
        // Its change landed on main another way (cherry-picked, squashed).
        let picked = branch("t3", "C.md", "c\n");
        git(&repo, &["cherry-pick", &picked]).unwrap();
        assert_eq!(is_merged(&repo, &picked), Ok(true));
        // …and merging one makes no commit.
        let tip = git(&repo, &["rev-parse", "main"]).unwrap();
        merge_into_main(&repo, &s.branch, "merge t2").unwrap();
        assert_eq!(git(&repo, &["rev-parse", "main"]).unwrap(), tip);
        // A conflict is not "nothing".
        let theirs = branch("t4", "README.md", "theirs\n");
        std::fs::write(repo.join("README.md"), "ours\n").unwrap();
        git(&repo, &["commit", "-qam", "ours"]).unwrap();
        assert_eq!(is_merged(&repo, &theirs), Ok(false));
    }

    #[test]
    fn refuses_to_work_outside_a_repository() {
        assert!(repo_root(&scratch("plain")).unwrap_err().contains("not inside a git repository"));
    }

    #[test]
    fn changes_land_on_a_branch_and_leave_the_checkout_alone() {
        let repo = repo();
        let sandbox = create(&repo, &scratch("home"), "t1", "try").unwrap();
        assert!(sandbox.path.join("README.md").exists());

        std::fs::write(sandbox.path.join("README.md"), "hello\nworld\n").unwrap();
        let sha = sandbox.commit_all("equipe try: say world").unwrap();
        assert!(sha.is_some());
        assert!(sandbox.diff_stat().unwrap().contains("README.md"));

        sandbox.remove_worktree().unwrap();
        assert!(!sandbox.path.exists());
        // The branch carries the change; the user's checkout does not.
        assert_eq!(git(&repo, &["show", "equipe/try-t1:README.md"]).unwrap(), "hello\nworld");
        assert_eq!(std::fs::read_to_string(repo.join("README.md")).unwrap(), "hello\n");
        assert_eq!(git(&repo, &["status", "--porcelain"]).unwrap(), "");
    }

    #[test]
    fn new_files_are_committed_too() {
        let repo = repo();
        let sandbox = create(&repo, &scratch("home"), "t2", "try").unwrap();
        std::fs::write(sandbox.path.join("NEW.md"), "new\n").unwrap();
        assert!(sandbox.commit_all("add").unwrap().is_some());
        sandbox.remove_worktree().unwrap();
        assert_eq!(git(&repo, &["show", "equipe/try-t2:NEW.md"]).unwrap(), "new");
    }

    #[test]
    fn a_run_that_changed_nothing_leaves_no_branch_behind() {
        let repo = repo();
        let sandbox = create(&repo, &scratch("home"), "t3", "try").unwrap();
        assert_eq!(sandbox.commit_all("nothing").unwrap(), None);
        sandbox.discard().unwrap();
        assert!(git(&repo, &["rev-parse", "--verify", "-q", "equipe/try-t3"]).is_err());
    }

    #[test]
    fn commits_even_without_a_configured_identity() {
        let repo = repo();
        let sandbox = create(&repo, &scratch("home"), "t4", "try").unwrap();
        git(&repo, &["config", "--unset", "user.email"]).unwrap();
        git(&repo, &["config", "--unset", "user.name"]).unwrap();
        std::fs::write(sandbox.path.join("README.md"), "changed\n").unwrap();
        let result = sandbox.commit_all("no identity");
        // Either the global identity or the fallback must have made the commit.
        assert!(matches!(result, Ok(Some(_))), "{result:?}");
    }

    /// A finished ticket's branch, with `file` written on it.
    fn finished(repo: &Path, home: &Path, id: &str, file: &str, text: &str) -> String {
        let sandbox = create(repo, home, id, "dev").unwrap();
        std::fs::write(sandbox.path.join(file), text).unwrap();
        sandbox.commit_all(id).unwrap();
        sandbox.remove_worktree().unwrap();
        sandbox.branch
    }

    #[test]
    fn a_ticket_starts_with_the_unmerged_work_it_depends_on() {
        let repo = repo();
        let home = scratch("home");
        let one = finished(&repo, &home, "t1", "ONE.md", "one\n");
        let two = finished(&repo, &home, "t2", "TWO.md", "two\n");
        let after = [("T-1".to_owned(), one), ("T-2".to_owned(), two)];
        let sandbox = create_after(&repo, &home, "t3", "dev", &after).unwrap();
        assert_eq!(std::fs::read_to_string(sandbox.path.join("ONE.md")).unwrap(), "one\n");
        assert_eq!(std::fs::read_to_string(sandbox.path.join("TWO.md")).unwrap(), "two\n");
        // Its own change is what it shows, not the work it started from.
        assert!(!sandbox.has_commits());
        std::fs::write(sandbox.path.join("THREE.md"), "three\n").unwrap();
        sandbox.commit_all("three").unwrap();
        assert_eq!(sandbox.diff_stat().unwrap().matches('|').count(), 1, "{}", sandbox.diff_stat().unwrap());
        // main is untouched.
        assert!(!repo.join("ONE.md").exists());
    }

    #[test]
    fn work_already_in_head_or_gone_is_skipped() {
        let repo = repo();
        let home = scratch("home");
        let one = finished(&repo, &home, "t1", "ONE.md", "one\n");
        merge_into_main(&repo, &one, "merge").unwrap();
        let after = [("T-1".to_owned(), one), ("T-9".to_owned(), "equipe/gone-t9".to_owned())];
        let sandbox = create_after(&repo, &home, "t2", "dev", &after).unwrap();
        assert_eq!(sandbox.base, git(&repo, &["rev-parse", "main"]).unwrap());
    }

    #[test]
    fn dependencies_that_conflict_stop_the_start_and_leave_nothing() {
        let repo = repo();
        let home = scratch("home");
        let one = finished(&repo, &home, "t1", "README.md", "one\n");
        let two = finished(&repo, &home, "t2", "README.md", "two\n");
        let after = [("T-1".to_owned(), one), ("T-2".to_owned(), two)];
        let err = create_after(&repo, &home, "t3", "dev", &after).unwrap_err();
        assert!(err.contains("T-2") && err.contains("README.md"), "{err}");
        assert!(git(&repo, &["rev-parse", "--verify", "-q", "equipe/dev-t3"]).is_err());
        assert!(!home.join("worktrees").join(repo.file_name().unwrap()).join("t3").exists());
    }

    #[test]
    fn accepted_work_merges_into_the_checked_out_main() {
        let repo = repo();
        let home = scratch("home");
        let one = finished(&repo, &home, "t1", "ONE.md", "one\n");
        // The person's own uncommitted change stays where it is.
        std::fs::write(repo.join("NOTES.md"), "mine\n").unwrap();
        assert!(!is_merged(&repo, &one).unwrap());
        assert_eq!(merge_into_main(&repo, &one, "equipe: merge T-1").unwrap(), "main");
        assert!(is_merged(&repo, &one).unwrap());
        assert_eq!(std::fs::read_to_string(repo.join("ONE.md")).unwrap(), "one\n");
        assert_eq!(std::fs::read_to_string(repo.join("NOTES.md")).unwrap(), "mine\n");
        assert_eq!(git(&repo, &["status", "--porcelain"]).unwrap(), "?? NOTES.md");
        assert_eq!(git(&repo, &["log", "-1", "--format=%s%n%P"]).unwrap().lines().nth(1).unwrap().split(' ').count(), 2);
        // Merging again does nothing.
        let head = git(&repo, &["rev-parse", "main"]).unwrap();
        merge_into_main(&repo, &one, "again").unwrap();
        assert_eq!(git(&repo, &["rev-parse", "main"]).unwrap(), head);
    }

    #[test]
    fn main_moves_even_when_nothing_has_it_checked_out() {
        let repo = repo();
        let home = scratch("home");
        let one = finished(&repo, &home, "t1", "ONE.md", "one\n");
        git(&repo, &["checkout", "-q", "-b", "elsewhere"]).unwrap();
        merge_into_main(&repo, &one, "merge").unwrap();
        assert_eq!(git(&repo, &["show", "main:ONE.md"]).unwrap(), "one");
        assert!(!repo.join("ONE.md").exists(), "the checkout on another branch stays as it was");
    }

    #[test]
    fn a_conflicting_merge_changes_nothing() {
        let repo = repo();
        let home = scratch("home");
        let one = finished(&repo, &home, "t1", "README.md", "one\n");
        std::fs::write(repo.join("README.md"), "main moved\n").unwrap();
        git(&repo, &["commit", "-q", "-am", "main moved"]).unwrap();
        let head = git(&repo, &["rev-parse", "main"]).unwrap();
        let err = merge_into_main(&repo, &one, "merge").unwrap_err();
        assert!(err.contains("README.md") && err.contains("nothing was merged"), "{err}");
        assert_eq!(git(&repo, &["rev-parse", "main"]).unwrap(), head);
        assert_eq!(git(&repo, &["status", "--porcelain"]).unwrap(), "");
    }

    #[test]
    fn a_checkout_whose_changes_would_be_overwritten_refuses_the_merge() {
        let repo = repo();
        let home = scratch("home");
        let one = finished(&repo, &home, "t1", "README.md", "one\n");
        std::fs::write(repo.join("README.md"), "mine\n").unwrap();
        let head = git(&repo, &["rev-parse", "main"]).unwrap();
        assert!(merge_into_main(&repo, &one, "merge").unwrap_err().contains("nothing was merged"));
        assert_eq!(git(&repo, &["rev-parse", "main"]).unwrap(), head);
        assert_eq!(std::fs::read_to_string(repo.join("README.md")).unwrap(), "mine\n");
    }

    #[test]
    fn a_project_with_no_remote_has_nothing_to_push_to() {
        let repo = repo();
        assert!(!has_origin(&repo));
        assert_eq!(push_main(&repo).unwrap_err(), NO_ORIGIN);
    }

    #[test]
    fn main_is_pushed_to_origin() {
        let repo = repo();
        let remote = scratch("remote");
        git(&remote, &["init", "-q", "--bare"]).unwrap();
        git(&repo, &["remote", "add", "origin", &remote.to_string_lossy()]).unwrap();
        let one = finished(&repo, &scratch("home"), "t1", "ONE.md", "one\n");
        merge_into_main(&repo, &one, "merge").unwrap();
        push_main(&repo).unwrap();
        assert_eq!(git(&remote, &["rev-parse", "main"]).unwrap(), git(&repo, &["rev-parse", "main"]).unwrap());
    }
}
