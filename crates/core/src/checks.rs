//! A project's own checks, run by equipe on the merged result before handed-in
//! work lands on main.
//!
//! When the PM accepts a code ticket (`accept_work`), its branch merged with
//! main is what main would become. A member saying its tests pass is not
//! evidence of that: the branch may be days behind main, and two changes that
//! each pass alone can break each other. So when the project declares its
//! checks, equipe works the merge out ([`worktree::prepare_merge`]), checks
//! that exact commit out in a temporary worktree, runs every command there,
//! and only then lands that same commit on main ([`worktree::land`]).
//!
//! # Where the commands come from
//!
//! The project declares them in [`FILE`] (`.equipe/checks.json`), committed
//! like a CI workflow:
//!
//! ```json
//! {
//!   "commands": [
//!     "cargo test --workspace --locked",
//!     "cargo clippy --workspace --all-targets --locked -- -D warnings"
//!   ],
//!   "timeoutMinutes": 40
//! }
//! ```
//!
//! It is read from main as it stands — never from the branch being checked,
//! so a branch can't weaken the checks it is held to, and never guessed:
//! equipe doesn't run a heavy suite the project didn't ask for. A project with
//! no such file is merged as before, and the acceptance says no checks ran.
//! A file that can't be read holds the merge back (fail closed).
//!
//! Each command runs through the shell (`sh -c`, `cmd /C` on Windows) at the
//! top of the merged tree, one after another; all must exit 0 within
//! `timeoutMinutes` together ([`DEFAULT_TIMEOUT`] when left out, at most
//! [`MAX_TIMEOUT`]). `EQUIPE_CHECKS=1` is set, and Rust builds go to a build
//! folder of their own that is kept between runs (the runtime sets
//! `CARGO_TARGET_DIR`, as it does for members).
//!
//! When a command fails, it runs once more on main alone: main failing it too
//! means main is broken already, which isn't the branch's fault — the work
//! stays in review and the PM hears so, rather than every member being sent
//! back for main's failure.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::worktree::{self, MergeCommit, Prepared};

/// Where a project declares its checks, from the top of its repository.
pub const FILE: &str = ".equipe/checks.json";

/// How long a project's checks may take together when its file doesn't say.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// The longest a project's file may ask for.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(3 * 60 * 60);

/// The largest checks file read.
const MAX_FILE: u64 = 64 * 1024;

/// How much of a failing command's output is kept: its last lines.
const TAIL_LINES: usize = 40;
const TAIL_BYTES: usize = 4000;

/// Set for the commands, so a project's scripts can tell they run as
/// equipe's checks.
pub const ENV: &str = "EQUIPE_CHECKS";

/// A project's checks, as its file declares them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checks {
    pub commands: Vec<String>,
    pub timeout: Duration,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Declared {
    commands: Vec<String>,
    #[serde(default)]
    timeout_minutes: Option<u64>,
}

/// A checks file's bytes, read; what is wrong with it said.
pub fn parse(bytes: &[u8]) -> Result<Checks, String> {
    let declared: Declared = serde_json::from_slice(bytes).map_err(|e| format!("{FILE} isn't valid: {e}"))?;
    let commands: Vec<String> = declared.commands.iter().map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()).collect();
    if commands.is_empty() {
        return Err(format!("{FILE} lists no commands"));
    }
    let timeout = match declared.timeout_minutes {
        None => DEFAULT_TIMEOUT,
        Some(0) => return Err(format!("{FILE}: timeoutMinutes must be more than 0")),
        Some(minutes) => Duration::from_secs(minutes.saturating_mul(60)).min(MAX_TIMEOUT),
    };
    Ok(Checks { commands, timeout })
}

/// The checks `repo` declares at `rev` (main's tip): `None` when it declares
/// none. Runs git: off the loop.
pub fn declared_at(repo: &Path, rev: &str) -> Result<Option<Checks>, String> {
    match worktree::read_blob(repo, rev, Path::new(FILE), MAX_FILE) {
        Ok(bytes) => parse(&bytes).map(Some),
        Err(worktree::Blob::Missing) => Ok(None),
        Err(worktree::Blob::TooBig(size)) => Err(format!("{FILE} is {size} bytes, more than a checks file needs")),
        Err(worktree::Blob::Failed(err)) => Err(format!("{FILE} couldn't be read: {err}")),
    }
}

/// The checks the main branch of `repo` declares now. Runs git: off the loop.
pub fn declared_on_main(repo: &Path) -> Result<Option<Checks>, String> {
    let main = worktree::main_branch(repo)?;
    declared_at(repo, &format!("refs/heads/{main}"))
}

/// How a run of a branch's checks ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The branch is in main already, or would change nothing there: nothing
    /// to check, nothing to land.
    Nothing { main: String },
    /// Every command passed on the merge, and main is still where the merge
    /// was worked out: [`worktree::land`] it.
    Passed { merge: MergeCommit, ran: Vec<String> },
    /// The project declares no checks (on main as it stands): the merge to
    /// land, unchecked.
    Undeclared { merge: MergeCommit },
    /// A command failed on the merge; main alone passes it, or couldn't be told.
    Failed { command: String, code: Option<i32>, tail: String },
    /// A command was still running when the time for all of them ran out.
    TimedOut { command: String, after: Duration, tail: String },
    /// A command failed on the merge and on main alone too: main is broken
    /// already, not by this branch.
    MainFails { command: String, tail: String },
    /// Main or the branch moved on while the checks ran: what passed isn't
    /// what would land.
    Moved,
    /// The checks couldn't be run, or the file couldn't be read: nothing lands.
    Error(String),
}

/// What runs a branch's checks.
pub struct Run<'a> {
    pub repo: &'a Path,
    pub branch: &'a str,
    /// The merge commit's message.
    pub message: &'a str,
    /// Where the temporary worktrees go; each is removed when done.
    pub scratch: &'a Path,
    /// A name for this run's folders (`t12`).
    pub label: &'a str,
    /// Set for every command (`CARGO_TARGET_DIR`…).
    pub env: &'a [(String, String)],
}

impl Run<'_> {
    /// Work the merge out, read the checks main declares, run them on the
    /// merge in a temporary worktree, and say how it went. Nothing moves:
    /// landing is the caller's. Minutes: off the loop.
    pub fn run(&self) -> Outcome {
        debug_assert!(!crate::runtime::on_core_loop(), "checks run off the core loop");
        let merge = match worktree::prepare_merge(self.repo, self.branch, self.message) {
            Ok(Prepared::Nothing { main }) => return Outcome::Nothing { main },
            Ok(Prepared::Merge(merge)) => merge,
            Err(err) => return Outcome::Error(err),
        };
        // Read at the very commit the merge is worked out on: never the branch's.
        let checks = match declared_at(self.repo, &merge.tip) {
            Ok(Some(checks)) => checks,
            Ok(None) => return Outcome::Undeclared { merge },
            Err(err) => return Outcome::Error(err),
        };
        let deadline = Instant::now() + checks.timeout;
        let ended = match self.on(&merge.commit, "merge", |dir| run_all(dir, &checks.commands, self.env, deadline, checks.timeout)) {
            Ok(ended) => ended,
            Err(err) => return Outcome::Error(err),
        };
        match ended {
            Ended::Passed => {}
            Ended::TimedOut { command, tail } => return Outcome::TimedOut { command, after: checks.timeout, tail },
            Ended::Failed { command, code, tail } => {
                // Main alone, the same command, as long again at most.
                let alone = Instant::now() + checks.timeout;
                let main_fails = self.on(&merge.tip, "main", |dir| run_all(dir, std::slice::from_ref(&command), self.env, alone, checks.timeout));
                return match main_fails {
                    Ok(Ended::Failed { tail: main_tail, .. }) => Outcome::MainFails { command, tail: main_tail },
                    _ => Outcome::Failed { command, code, tail },
                };
            }
        }
        // What passed must be what lands: neither main nor the branch moved meanwhile.
        if !worktree::main_is_at(self.repo, &merge) || !worktree::branch_is_at(self.repo, self.branch, &merge) {
            return Outcome::Moved;
        }
        Outcome::Passed { merge, ran: checks.commands }
    }

    /// `work` in a temporary worktree at `commit`, removed afterwards whatever happens.
    fn on(&self, commit: &str, what: &str, work: impl FnOnce(&Path) -> Ended) -> Result<Ended, String> {
        let short: String = commit.chars().take(10).collect();
        let dir = self.scratch.join(format!("{}-{what}-{short}", self.label));
        std::fs::create_dir_all(self.scratch).map_err(|e| format!("{}: {e}", self.scratch.display()))?;
        if dir.exists() {
            let _ = worktree::remove_at(&dir);
        }
        worktree::add_detached(self.repo, &dir, commit)?;
        // The project's installed dependencies, cloned copy-on-write where they match.
        for note in worktree::seed_dependencies(&dir) {
            crate::daemon_log!("equipe: checks: {note}");
        }
        let ended = work(&dir);
        if let Err(err) = worktree::remove_at(&dir) {
            crate::daemon_log!("equipe: checks: couldn't remove {}: {err}", dir.display());
        }
        Ok(ended)
    }
}

/// How a run of commands ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ended {
    Passed,
    Failed { command: String, code: Option<i32>, tail: String },
    TimedOut { command: String, tail: String },
}

/// Each of `commands` in `dir`, one after another, until one fails or
/// `deadline` passes.
fn run_all(dir: &Path, commands: &[String], env: &[(String, String)], deadline: Instant, timeout: Duration) -> Ended {
    let log = dir.with_extension("log");
    let ended = (|| {
        for command in commands {
            match run_one(dir, command, env, deadline, &log, timeout) {
                One::Passed => continue,
                One::Failed(code) => return Ended::Failed { command: command.clone(), code, tail: tail(&log) },
                One::TimedOut => return Ended::TimedOut { command: command.clone(), tail: tail(&log) },
            }
        }
        Ended::Passed
    })();
    let _ = std::fs::remove_file(&log);
    ended
}

enum One {
    Passed,
    Failed(Option<i32>),
    TimedOut,
}

/// `command` through the shell in `dir`, its output (both streams) to `log`;
/// stopped, with everything it started, once `deadline` passes.
fn run_one(dir: &Path, command: &str, env: &[(String, String)], deadline: Instant, log: &PathBuf, timeout: Duration) -> One {
    let Ok(out) = std::fs::File::create(log) else {
        return One::Failed(None);
    };
    let Ok(err) = out.try_clone() else {
        return One::Failed(None);
    };
    #[cfg(windows)]
    let mut shell = {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    };
    #[cfg(not(windows))]
    let mut shell = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    shell.current_dir(dir).stdin(Stdio::null()).stdout(Stdio::from(out)).stderr(Stdio::from(err));
    shell.env(ENV, "1").env("GIT_TERMINAL_PROMPT", "0");
    for (key, value) in env {
        shell.env(key, value);
    }
    // A group of its own, so a timeout stops what it started too (cargo's rustc, a test binary).
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        shell.process_group(0);
    }
    let mut child = match shell.spawn() {
        Ok(child) => child,
        Err(e) => {
            let _ = std::fs::write(log, format!("couldn't run the shell: {e}\n"));
            return One::Failed(None);
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return One::Passed,
            Ok(Some(status)) => return One::Failed(status.code()),
            Ok(None) => {}
            Err(_) => return One::Failed(None),
        }
        if Instant::now() >= deadline {
            crate::group::end(child.id(), crate::group::Signal::Kill);
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::OpenOptions::new()
                .append(true)
                .open(log)
                .and_then(|mut f| std::io::Write::write_all(&mut f, format!("\n[stopped: the checks took more than {} minutes]\n", timeout.as_secs() / 60).as_bytes()));
            return One::TimedOut;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The last part of what a command printed: its last [`TAIL_LINES`] lines,
/// at most [`TAIL_BYTES`].
fn tail(log: &Path) -> String {
    let bytes = std::fs::read(log).unwrap_or_default();
    let from = bytes.len().saturating_sub(TAIL_BYTES * 4);
    let text = String::from_utf8_lossy(&bytes[from..]);
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n");
    if kept.len() > TAIL_BYTES {
        let mut cut = kept.len() - TAIL_BYTES;
        while !kept.is_char_boundary(cut) {
            cut += 1;
        }
        kept = kept[cut..].to_owned();
    }
    kept.trim().to_owned()
}

/// Folders a run left behind under `scratch` — the daemon stopped mid-run —
/// removed: nothing runs checks before the daemon has started. What went.
pub fn sweep(scratch: &Path) -> Vec<String> {
    let mut done = Vec::new();
    for entry in std::fs::read_dir(scratch).into_iter().flatten().flatten() {
        let path = entry.path();
        let gone = if entry.file_type().is_ok_and(|t| t.is_dir()) { worktree::remove_at(&path).is_ok() } else { std::fs::remove_file(&path).is_ok() };
        if gone {
            done.push(format!("removed {}, left by checks that didn't finish", path.display()));
        }
    }
    done
}

/// What the member is told when its work failed the checks: the command,
/// how it ended, and its output's last part.
pub fn failed_reason(outcome: &Outcome) -> Option<String> {
    let (what, tail) = match outcome {
        Outcome::Failed { command, code, tail } => {
            let code = code.map(|c| format!(" (exit code {c})")).unwrap_or_default();
            (format!("`{command}` failed{code}"), tail)
        }
        Outcome::TimedOut { command, after, tail } => (format!("`{command}` was still running after {} minutes and was stopped", after.as_secs() / 60), tail),
        _ => return None,
    };
    let tail = if tail.is_empty() { String::new() } else { format!(" Its output ended:\n```\n{tail}\n```\n") };
    Some(format!(
        "equipe ran the project's checks ({FILE}) on your branch merged with main, before merging it, and {what}.{tail} Main \
alone passes it, so the change is in your branch or in how it meets main: merge main into your branch in your worktree \
(git merge main; no rebase needed), fix it, run the same command yourself until it passes, commit, and hand it in again."
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-checks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// A repository on `main` with `checks` (if any) committed, and a branch
    /// `work` that adds `work.txt`, behind a later commit on main.
    fn repo(name: &str, checks: Option<&str>) -> PathBuf {
        let dir = scratch(name).join("repo");
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.name", "Test"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        std::fs::write(dir.join("README.md"), "hello\n").unwrap();
        if let Some(checks) = checks {
            std::fs::create_dir_all(dir.join(".equipe")).unwrap();
            std::fs::write(dir.join(FILE), checks).unwrap();
        }
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "init"]);
        git(&dir, &["checkout", "-q", "-b", "work"]);
        std::fs::write(dir.join("work.txt"), "work\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "work"]);
        git(&dir, &["checkout", "-q", "main"]);
        std::fs::write(dir.join("later.txt"), "later\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "later on main"]);
        dir
    }

    fn run(repo: &Path) -> Outcome {
        let scratch = repo.parent().unwrap().join("checks");
        let outcome = Run { repo, branch: "work", message: "merge work", scratch: &scratch, label: "t1", env: &[] }.run();
        // Nothing is left behind, whatever happened.
        assert_eq!(std::fs::read_dir(&scratch).map(|d| d.count()).unwrap_or(0), 0, "scratch emptied");
        assert!(!git(repo, &["worktree", "list"]).contains(scratch.to_string_lossy().as_ref()), "worktrees removed");
        outcome
    }

    #[test]
    fn the_file_is_read_strictly_and_the_timeout_bounded() {
        let checks = parse(br#"{ "commands": ["cargo test", "  ", "cargo clippy"], "timeoutMinutes": 45 }"#).unwrap();
        assert_eq!(checks, Checks { commands: vec!["cargo test".into(), "cargo clippy".into()], timeout: Duration::from_secs(45 * 60) });
        assert_eq!(parse(br#"{ "commands": ["make check"] }"#).unwrap().timeout, DEFAULT_TIMEOUT);
        assert_eq!(parse(br#"{ "commands": ["x"], "timeoutMinutes": 100000 }"#).unwrap().timeout, MAX_TIMEOUT);
        assert!(parse(br#"{ "commands": [] }"#).unwrap_err().contains("lists no commands"));
        assert!(parse(br#"{ "commands": ["x"], "timeoutMinutes": 0 }"#).is_err());
        // A misspelt key isn't silently ignored.
        assert!(parse(br#"{ "command": ["x"] }"#).is_err());
        assert!(parse(b"not json").unwrap_err().contains("isn't valid"));
    }

    #[test]
    fn checks_that_pass_on_the_merged_tree_give_the_merge_to_land_and_main_is_untouched_until_then() {
        // The command sees the merged tree: the branch's file and main's later one together.
        let repo = repo("pass", Some(r#"{ "commands": ["test -f work.txt && test -f later.txt", "test \"$EQUIPE_CHECKS\" = 1"] }"#));
        let before = git(&repo, &["rev-parse", "main"]);
        let Outcome::Passed { merge, ran } = run(&repo) else { panic!("expected a pass") };
        assert_eq!(ran.len(), 2);
        assert_eq!((merge.tip.as_str(), merge.main.as_str()), (before.as_str(), "main"));
        assert_eq!(git(&repo, &["rev-parse", "main"]), before, "nothing landed yet");
        // The very commit checked is the one that lands.
        worktree::land(&repo, &merge).unwrap();
        assert_eq!(git(&repo, &["rev-parse", "main"]), merge.commit);
        assert_eq!(git(&repo, &["show", "main:work.txt"]), "work");
    }

    #[test]
    fn a_failing_check_says_which_command_and_its_output_and_main_alone_passing_puts_it_on_the_branch() {
        let repo = repo("fail", Some(r#"{ "commands": ["true", "echo broken by the branch; test ! -f work.txt"] }"#));
        let before = git(&repo, &["rev-parse", "main"]);
        let outcome = run(&repo);
        let Outcome::Failed { command, code, tail } = &outcome else { panic!("expected a failure: {outcome:?}") };
        assert_eq!((command.as_str(), *code), ("echo broken by the branch; test ! -f work.txt", Some(1)));
        assert!(tail.contains("broken by the branch"), "{tail}");
        assert_eq!(git(&repo, &["rev-parse", "main"]), before, "main untouched");
        let reason = failed_reason(&outcome).unwrap();
        assert!(reason.contains("exit code 1") && reason.contains("broken by the branch") && reason.contains("git merge main"), "{reason}");
    }

    #[test]
    fn main_failing_the_same_command_alone_is_main_broken_not_the_branch() {
        let repo = repo("broken-main", Some(r#"{ "commands": ["echo main is red; false"] }"#));
        let outcome = run(&repo);
        assert!(matches!(&outcome, Outcome::MainFails { command, tail } if command == "echo main is red; false" && tail.contains("main is red")), "{outcome:?}");
        assert_eq!(failed_reason(&outcome), None, "nobody is sent back for it");
    }

    #[test]
    fn a_check_past_its_time_is_stopped_with_what_it_started() {
        let repo = repo("slow", Some(r#"{ "commands": ["sleep 30 & echo started; wait"], "timeoutMinutes": 1 }"#));
        let scratch = repo.parent().unwrap().join("checks");
        let merge = match worktree::prepare_merge(&repo, "work", "m").unwrap() {
            Prepared::Merge(m) => m,
            other => panic!("{other:?}"),
        };
        // A deadline already near, rather than a minute's wait.
        let dir = scratch.join("slow");
        std::fs::create_dir_all(&scratch).unwrap();
        worktree::add_detached(&repo, &dir, &merge.commit).unwrap();
        let begun = Instant::now();
        let ended = run_all(&dir, &["sleep 30 & echo started; wait".to_owned()], &[], Instant::now() + Duration::from_millis(500), Duration::from_secs(60));
        assert!(begun.elapsed() < Duration::from_secs(10), "stopped, not waited for");
        assert!(matches!(&ended, Ended::TimedOut { command, tail } if command.starts_with("sleep") && tail.contains("started") && tail.contains("stopped")), "{ended:?}");
        worktree::remove_at(&dir).unwrap();
        let reason = failed_reason(&Outcome::TimedOut { command: "sleep 30".into(), after: Duration::from_secs(60), tail: "started".into() }).unwrap();
        assert!(reason.contains("still running after 1 minutes"), "{reason}");
    }

    #[test]
    fn no_declared_checks_gives_the_merge_unchecked_and_a_bad_file_holds_it_back() {
        let repo = repo("none", None);
        assert!(matches!(run(&repo), Outcome::Undeclared { .. }));
        let bad = repo_with_bad_file();
        assert!(matches!(run(&bad), Outcome::Error(err) if err.contains("isn't valid")));
    }

    fn repo_with_bad_file() -> PathBuf {
        repo("bad", Some("{ oops"))
    }

    #[test]
    fn the_checks_are_read_from_main_never_from_the_branch() {
        // The branch tries to switch its checks off; main's still run.
        let repo = repo("weakened", Some(r#"{ "commands": ["test ! -f work.txt"] }"#));
        git(&repo, &["checkout", "-q", "work"]);
        std::fs::write(repo.join(FILE), r#"{ "commands": ["true"] }"#).unwrap();
        git(&repo, &["commit", "-q", "-am", "weaken the checks"]);
        git(&repo, &["checkout", "-q", "main"]);
        assert!(matches!(run(&repo), Outcome::Failed { command, .. } if command == "test ! -f work.txt"));
    }

    #[test]
    fn a_branch_already_in_main_has_nothing_to_check() {
        let repo = repo("merged", Some(r#"{ "commands": ["false"] }"#));
        git(&repo, &["merge", "-q", "--no-edit", "work"]);
        assert!(matches!(run(&repo), Outcome::Nothing { main } if main == "main"));
    }

    #[test]
    fn main_moving_while_the_checks_ran_lands_nothing() {
        let repo = repo("moved", Some(r#"{ "commands": ["true"] }"#));
        let merge = match worktree::prepare_merge(&repo, "work", "m").unwrap() {
            Prepared::Merge(m) => m,
            other => panic!("{other:?}"),
        };
        std::fs::write(repo.join("meanwhile.txt"), "x\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "meanwhile"]);
        let now = git(&repo, &["rev-parse", "main"]);
        assert!(worktree::land(&repo, &merge).unwrap_err().starts_with(worktree::MAIN_MOVED));
        assert_eq!(git(&repo, &["rev-parse", "main"]), now);
    }

    #[test]
    fn the_branch_moving_while_the_checks_ran_is_not_what_passed() {
        // A member commits on the branch mid-run: what passed isn't what would land.
        let repo = repo("branch-moved", Some(r#"{ "commands": ["git update-ref refs/heads/work HEAD"] }"#));
        let before = git(&repo, &["rev-parse", "main"]);
        assert_eq!(run(&repo), Outcome::Moved);
        assert_eq!(git(&repo, &["rev-parse", "main"]), before);
    }

    #[test]
    fn leftovers_of_a_run_that_never_finished_are_swept() {
        let repo = repo("sweep", None);
        let scratch = repo.parent().unwrap().join("checks");
        std::fs::create_dir_all(&scratch).unwrap();
        let head = git(&repo, &["rev-parse", "main"]);
        worktree::add_detached(&repo, &scratch.join("t9-merge-abc"), &head).unwrap();
        std::fs::write(scratch.join("t9-merge-abc.log"), "x").unwrap();
        assert_eq!(sweep(&scratch).len(), 2);
        assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0);
        assert!(!git(&repo, &["worktree", "list"]).contains("t9-merge"));
    }
}
