//! Isolated workspaces for agents that edit code.
//!
//! An agent never works in the user's checkout. It gets a fresh git worktree on a
//! new branch, cut from `HEAD`, under equipe's home rather than inside the repo
//! (so `git status` in the user's checkout stays clean). When it finishes, its
//! changes are committed to that branch and the worktree is removed: what remains
//! is one reviewable branch.
//!
//! Phase 2's orchestrator will own this per ticket; `equipe try --edit` uses it
//! for a single turn.

use std::path::{Path, PathBuf};
use std::process::Command;

pub type Result<T> = std::result::Result<T, String>;

/// Run git in `dir` and return trimmed stdout, or stderr as the error.
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|err| format!("couldn't run git: {err}"))?;
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
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let path = home.join("worktrees").join(&name).join(id);
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

#[cfg(test)]
mod tests {
    use super::*;

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
        git(&dir, &["init", "-q"]).unwrap();
        git(&dir, &["config", "user.name", "Test"]).unwrap();
        git(&dir, &["config", "user.email", "test@example.com"]).unwrap();
        std::fs::write(dir.join("README.md"), "hello\n").unwrap();
        git(&dir, &["add", "-A"]).unwrap();
        git(&dir, &["commit", "-q", "-m", "init"]).unwrap();
        dir
    }

    #[test]
    fn finds_the_repository_root_from_a_subdirectory() {
        let repo = repo();
        std::fs::create_dir_all(repo.join("a/b")).unwrap();
        let root = repo_root(&repo.join("a/b")).unwrap();
        assert_eq!(root.canonicalize().unwrap(), repo.canonicalize().unwrap());
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
}
