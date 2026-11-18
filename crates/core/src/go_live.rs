//! Going live after a landing: the rules, with no runtime in them, so each is
//! tested alone (the runtime's side is `runtime/go_live.rs`).
//!
//! Pushing main is not going live. Many projects deploy from a git push (a
//! Vercel or Netlify integration, a deploy workflow) and `crate::deploys`
//! watches those; others deploy only when someone runs a command. Such a
//! project declares that command in [`FILE`] (`.equipe/deploy.json`),
//! committed like its checks:
//!
//! ```json
//! { "command": "npm run deploy:prod", "from": "main" }
//! ```
//!
//! - `command` (required): run through the shell (`sh -c`, `cmd /C` on
//!   Windows) in the project's main checkout — the folder where equipe's
//!   landings merge — with `EQUIPE_DEPLOY=1` set and no input.
//! - `from`: the branch it deploys from; equipe's landings go to the main
//!   branch, so any other branch is refused. Left out: the main branch.
//! - `timeoutMinutes`: how long it may run ([`DEFAULT_TIMEOUT`], at most
//!   [`MAX_TIMEOUT`]).
//! - `quietSeconds`: how long after a landing equipe waits for more before it
//!   deploys ([`DEFAULT_QUIET`], at most [`MAX_QUIET`]).
//!
//! It is read from main as it stands, never from a branch being landed. A
//! project without the file can have the PM confirm a command instead
//! (`set_deploy`); equipe suggests one when the project has an obvious one
//! ([`suggest`]: a `deploy:prod` or `deploy` script in `package.json`, a
//! `deploy` target in its `Makefile`) but never runs a command nobody
//! declared or confirmed.
//!
//! After each landing that pushed main, the project's [`Lane`] notes it
//! ([`Lane::landed`]); once landings stop coming for the quiet time — or
//! [`MAX_WAIT`] after the first of them — the command runs once for all of
//! them ([`Lane::due_at`]), one run at a time per project. It runs only on a
//! clean checkout of main that is where origin's main is ([`checkout`]);
//! otherwise nothing runs and the reason is said.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use equipe_protocol::TicketId;
use serde::{Deserialize, Serialize};

use crate::worktree;

/// Where a project declares how it goes live, from the top of its repository.
pub const FILE: &str = ".equipe/deploy.json";
/// How long after a landing equipe waits for more before it deploys, when the file doesn't say.
pub const DEFAULT_QUIET: u64 = 2 * 60;
/// The longest quiet time a file may ask for.
pub const MAX_QUIET: u64 = 15 * 60;
/// Landings that keep coming still go live this long after the first of them.
pub const MAX_WAIT: u64 = 10 * 60;
/// How long the command may run when the file doesn't say.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// The longest a file may ask for.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// Set for the command, so a project's scripts can tell equipe runs them.
pub const ENV: &str = "EQUIPE_DEPLOY";
/// The largest deploy file read.
const MAX_FILE: u64 = 16 * 1024;
/// How much of a failing command's output is kept: its last lines.
const TAIL_LINES: usize = 20;
const TAIL_BYTES: usize = 2000;
/// Uncommitted files named when the checkout isn't clean, at most.
const DIRTY_NAMED: usize = 5;

/// How a project goes live, as its file declares it or the PM confirmed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoLive {
    pub command: String,
    /// The branch it deploys from, when the file names one.
    pub from: Option<String>,
    pub timeout: Duration,
    /// Seconds of quiet after a landing before it runs.
    pub quiet: u64,
}

impl GoLive {
    /// A command the PM confirmed (`set_deploy`), with the defaults.
    pub fn confirmed(command: &str) -> Self {
        GoLive { command: command.trim().to_owned(), from: None, timeout: DEFAULT_TIMEOUT, quiet: DEFAULT_QUIET }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Declared {
    command: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    timeout_minutes: Option<u64>,
    #[serde(default)]
    quiet_seconds: Option<u64>,
}

/// A deploy file's bytes, read; what is wrong with it said.
pub fn parse(bytes: &[u8]) -> Result<GoLive, String> {
    let declared: Declared = serde_json::from_slice(bytes).map_err(|e| format!("{FILE} isn't valid: {e}"))?;
    let command = declared.command.trim().to_owned();
    if command.is_empty() {
        return Err(format!("{FILE} names no command"));
    }
    let timeout = match declared.timeout_minutes {
        None => DEFAULT_TIMEOUT,
        Some(0) => return Err(format!("{FILE}: timeoutMinutes must be more than 0")),
        Some(minutes) => Duration::from_secs(minutes.saturating_mul(60)).min(MAX_TIMEOUT),
    };
    let from = declared.from.map(|f| f.trim().to_owned()).filter(|f| !f.is_empty());
    let quiet = declared.quiet_seconds.unwrap_or(DEFAULT_QUIET).min(MAX_QUIET);
    Ok(GoLive { command, from, timeout, quiet })
}

/// How `repo` goes live as `rev` (main's tip) declares it: `None` when it
/// declares nothing. Runs git: off the loop.
pub fn declared_at(repo: &Path, rev: &str) -> Result<Option<GoLive>, String> {
    match worktree::read_blob(repo, rev, Path::new(FILE), MAX_FILE) {
        Ok(bytes) => parse(&bytes).map(Some),
        Err(worktree::Blob::Missing) => Ok(None),
        Err(worktree::Blob::TooBig(size)) => Err(format!("{FILE} is {size} bytes, more than a deploy file needs")),
        Err(worktree::Blob::Failed(err)) => Err(format!("{FILE} couldn't be read: {err}")),
    }
}

/// How `repo`'s main branch declares it goes live now. Runs git: off the loop.
pub fn declared_on_main(repo: &Path) -> Result<Option<GoLive>, String> {
    let main = worktree::main_branch(repo)?;
    declared_at(repo, &format!("refs/heads/{main}"))
}

/// Where a project's go-live command comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// Its file declares it, or the PM confirmed one (the file wins).
    Command(GoLive),
    /// Its file can't be read: nothing runs until it is fixed.
    Invalid(String),
    /// Neither: what equipe would suggest the PM confirm, if anything.
    Undeclared { suggestion: Option<String> },
}

/// How `repo` goes live: its file on main, else the command the PM
/// `confirmed`, else a suggestion from the project's own files. Off the loop.
pub fn find(repo: &Path, confirmed: Option<&str>) -> Found {
    match declared_on_main(repo) {
        Ok(Some(go)) => Found::Command(go),
        Err(err) => Found::Invalid(err),
        Ok(None) => match confirmed.map(str::trim).filter(|c| !c.is_empty()) {
            Some(command) => Found::Command(GoLive::confirmed(command)),
            None => Found::Undeclared { suggestion: worktree::main_branch(repo).ok().and_then(|main| suggest(repo, &format!("refs/heads/{main}"))) },
        },
    }
}

// ── discovery ──

/// The deploy script a `package.json` names, if any: `deploy:prod` first,
/// then `deploy:production`, then `deploy`.
pub fn package_script(package_json: &[u8]) -> Option<&'static str> {
    let json: serde_json::Value = serde_json::from_slice(package_json).ok()?;
    let scripts = json.get("scripts")?.as_object()?;
    ["deploy:prod", "deploy:production", "deploy"].into_iter().find(|name| scripts.get(*name).and_then(|s| s.as_str()).is_some_and(|s| !s.trim().is_empty()))
}

/// Whether a `Makefile` has a `deploy` target.
pub fn makefile_deploys(makefile: &str) -> bool {
    makefile.lines().any(|line| line.strip_prefix("deploy").is_some_and(|rest| rest.trim_start().starts_with(':') && !rest.trim_start().starts_with(":=")))
}

/// The package runner a repository uses, by its lockfile at `rev`.
fn runner(repo: &Path, rev: &str) -> &'static str {
    let has = |file: &str| matches!(worktree::read_blob(repo, rev, Path::new(file), 1), Ok(_) | Err(worktree::Blob::TooBig(_)));
    if has("pnpm-lock.yaml") {
        "pnpm run"
    } else if has("yarn.lock") {
        "yarn run"
    } else if has("bun.lockb") || has("bun.lock") {
        "bun run"
    } else {
        "npm run"
    }
}

/// The command a project at `rev` obviously deploys with, to suggest the PM
/// confirm: its `package.json`'s deploy script, else its `Makefile`'s
/// `deploy` target. Never run without that confirmation. Off the loop.
pub fn suggest(repo: &Path, rev: &str) -> Option<String> {
    if let Ok(bytes) = worktree::read_blob(repo, rev, Path::new("package.json"), 512 * 1024)
        && let Some(script) = package_script(&bytes)
    {
        return Some(format!("{} {script}", runner(repo, rev)));
    }
    match worktree::read_blob(repo, rev, Path::new("Makefile"), 512 * 1024) {
        Ok(bytes) if makefile_deploys(&String::from_utf8_lossy(&bytes)) => Some("make deploy".into()),
        _ => None,
    }
}

// ── the checkout it runs in ──

/// Why the command didn't run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocked {
    /// The repository has no main branch equipe can name.
    NoMain(String),
    /// The file deploys from another branch than the one landings go to.
    From { from: String, main: String },
    /// No checkout has the main branch checked out.
    NotCheckedOut { main: String },
    /// The checkout has uncommitted or untracked files (the first few named).
    Dirty { dir: PathBuf, files: Vec<String> },
    /// The checkout isn't where origin's main is (or origin has no main).
    NotAtOrigin { dir: PathBuf, main: String, head: String, origin: Option<String> },
}

impl Blocked {
    /// Why, in a line for the PM.
    pub fn why(&self) -> String {
        match self {
            Blocked::NoMain(err) => format!("equipe couldn't tell the main branch: {err}"),
            Blocked::From { from, main } => format!("{FILE} deploys from {from}, but equipe lands work on {main}"),
            Blocked::NotCheckedOut { main } => format!("{main} isn't checked out in any folder of the project to deploy from"),
            Blocked::Dirty { dir, files } => format!("its main checkout ({}) has uncommitted files: {}", dir.display(), files.join(", ")),
            Blocked::NotAtOrigin { dir, main, head, origin: Some(origin) } => {
                format!("its main checkout ({}) is at {}, not at origin/{main} ({})", dir.display(), short(head), short(origin))
            }
            Blocked::NotAtOrigin { dir, main, .. } => format!("origin has no {main} to compare its main checkout ({}) with", dir.display()),
        }
    }
}

/// A commit's short name.
pub fn short(sha: &str) -> &str {
    &sha[..sha.len().min(9)]
}

/// Where the command runs for `repo` and the commit it deploys: main's
/// checkout, clean, at origin's main. Runs git: off the loop.
pub fn checkout(repo: &Path, go: &GoLive) -> Result<(PathBuf, String), Blocked> {
    let main = worktree::main_branch(repo).map_err(Blocked::NoMain)?;
    if let Some(from) = go.from.as_ref().filter(|f| **f != main) {
        return Err(Blocked::From { from: from.clone(), main });
    }
    let dir = worktree::checked_out_at(repo, &main).ok_or_else(|| Blocked::NotCheckedOut { main: main.clone() })?;
    let status = worktree::git(&dir, &["status", "--porcelain", "--untracked-files=all"]).map_err(|err| Blocked::NoMain(err.to_string()))?;
    let files: Vec<String> =
        status.lines().filter_map(|l| l.trim_start().split_once(' ').map(|(_, path)| path.trim().to_owned())).filter(|p| !p.is_empty()).collect();
    if !files.is_empty() {
        let mut named: Vec<String> = files.iter().take(DIRTY_NAMED).cloned().collect();
        if files.len() > DIRTY_NAMED {
            named.push(format!("+{} more", files.len() - DIRTY_NAMED));
        }
        return Err(Blocked::Dirty { dir, files: named });
    }
    let head = worktree::git(&dir, &["rev-parse", "HEAD"]).map_err(|err| Blocked::NoMain(err.to_string()))?;
    let origin = worktree::git(repo, &["rev-parse", "-q", "--verify", &format!("refs/remotes/origin/{main}^{{commit}}")]).ok();
    if origin.as_deref() != Some(head.as_str()) {
        return Err(Blocked::NotAtOrigin { dir, main, head, origin });
    }
    Ok((dir, head))
}

// ── the run ──

/// How a run of the command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    pub ok: bool,
    pub code: Option<i32>,
    pub timed_out: bool,
    /// The last of what it printed.
    pub tail: String,
    pub took: Duration,
}

/// Run `command` in `dir`, given `timeout`; what it printed goes to `log`.
/// Blocking: off the loop.
pub fn run(dir: &Path, command: &str, timeout: Duration, log: &Path) -> Ran {
    let began = Instant::now();
    let failed = |tail: String| Ran { ok: false, code: None, timed_out: false, tail, took: began.elapsed() };
    if let Some(parent) = log.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(out) = std::fs::File::create(log) else { return failed(format!("couldn't write {}", log.display())) };
    let Ok(err) = out.try_clone() else { return failed(format!("couldn't write {}", log.display())) };
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
    // A group of its own, so a timeout stops what it started too.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        shell.process_group(0);
    }
    let mut child = match shell.spawn() {
        Ok(child) => child,
        Err(e) => return failed(format!("couldn't run the shell: {e}")),
    };
    let deadline = began + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ran { ok: status.success(), code: status.code(), timed_out: false, tail: tail(log), took: began.elapsed() },
            Ok(None) => {}
            Err(e) => return failed(format!("couldn't wait for the command: {e}")),
        }
        if Instant::now() >= deadline {
            crate::group::end(child.id(), crate::group::Signal::Kill);
            let _ = child.kill();
            let _ = child.wait();
            return Ran { ok: false, code: None, timed_out: true, tail: tail(log), took: began.elapsed() };
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
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
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

// ── one go ──

/// How one go at deploying a project ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The command succeeded: `sha` is live, and the commits after `from` came with it.
    Went { command: String, sha: String, took: Duration, from: String, merges: Vec<crate::deploys::Merge> },
    /// Nothing ran, and why.
    Blocked { command: String, blocked: Blocked },
    /// It ran and failed (or ran out of time) on `sha`.
    Failed { command: String, sha: String, ran: Ran, timeout: Duration },
    /// The project's file can't be read.
    Invalid(String),
    /// The project declares no command any more, and none is confirmed.
    Gone,
}

/// One go at deploying `project`: its command found ([`find`]), run on a
/// clean main checkout at origin's main ([`checkout`]), what it printed in
/// `log` — one go at a time per repository. `from`: the last commit known
/// live, for what this one brought. Blocking: off the loop.
pub fn go(project: &Path, confirmed: Option<&str>, from: Option<&str>, log: &Path) -> Outcome {
    let repo = match worktree::repo_root(project) {
        Ok(repo) => repo,
        Err(err) => return Outcome::Blocked { command: confirmed.unwrap_or_default().to_owned(), blocked: Blocked::NoMain(err) },
    };
    // Its own lock, not the landings': a deploy that takes minutes never holds a landing up.
    crate::squad_home::one_at_a_time(&repo.join(".git").join("equipe-go-live"), || {
        let go = match find(&repo, confirmed) {
            Found::Command(go) => go,
            Found::Invalid(err) => return Outcome::Invalid(err),
            Found::Undeclared { .. } => return Outcome::Gone,
        };
        let command = go.command.clone();
        let (dir, sha) = match checkout(&repo, &go) {
            Ok(found) => found,
            Err(blocked) => return Outcome::Blocked { command, blocked },
        };
        crate::daemon_log!("equipe: deploying {} at {}: `{command}` in {}", project.display(), short(&sha), dir.display());
        let ran = run(&dir, &command, go.timeout, log);
        if !ran.ok {
            return Outcome::Failed { command, sha, ran, timeout: go.timeout };
        }
        let known = |c: &str| worktree::git(&repo, &["merge-base", "--is-ancestor", c, &sha]).is_ok();
        let from = from
            .filter(|f| *f != sha && known(f))
            .map(str::to_owned)
            .or_else(|| worktree::git(&repo, &["rev-parse", "-q", "--verify", &format!("{sha}^")]).ok())
            .unwrap_or_else(|| sha.clone());
        let merges = if from == sha { Vec::new() } else { crate::deploys::merges(&repo, &from, &sha) };
        Outcome::Went { command, sha, took: ran.took, from, merges }
    })
}

// ── the PM's tool ──

/// `set_deploy`: the PM says how a project goes live, or deploys it now.
pub const SET_DEPLOY: &str = "set_deploy";

/// `set_deploy`, as an MCP tool definition.
pub fn tool() -> serde_json::Value {
    serde_json::json!({
        "name": SET_DEPLOY,
        "description": "Say how a project goes live when it doesn't declare it in `.equipe/deploy.json`: the one shell \
command equipe then runs in the project's main checkout after work lands and main is pushed (once for several landings \
close together) — `npm run deploy:prod`, `make deploy`. Only the command the project itself deploys to production with, \
as its files, its docs or the person say; never one you made up. The project's file wins over it; `command: \"\"` \
stops it. `deploy_now: true` runs it at once: to retry after a fix, or on the person's yes where deploys wait for them.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": { "type": "string", "description": "The project, by name; left out when the squad has one." },
                "command": { "type": "string", "description": "The command, run in the project's main checkout; \"\" stops it." },
                "deploy_now": { "type": "boolean", "description": "Deploy now, rather than after the next landing." },
            },
        },
    })
}

// ── one project's lane ──

/// Landings pushed and not yet run for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    /// When the first and the last of them came (unix seconds).
    pub first: u64,
    pub last: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tickets: Vec<TicketId>,
}

/// What equipe keeps of one project's going live (`meta` key `go_live`, by
/// the project's path): landings waiting, the last commit deployed, the last
/// failure while it stands, a command the PM confirmed, and the suggestion
/// it was asked about.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lane {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Pending>,
    /// Seconds of quiet before a run, as its command last said.
    #[serde(default = "default_quiet")]
    pub quiet: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployed: Option<String>,
    #[serde(default)]
    pub deployed_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked: Option<String>,
    /// Its main declares a deploy file (readable or not), as last read.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub declared: bool,
    /// While its work can't go live, why, as a code a request's status says
    /// (`deploy_failed`, `deploy_blocked`, `deploy_invalid`, `deploy_waits`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stuck: Option<String>,
    /// Its command runs now (not kept: a restart ends every run).
    #[serde(skip)]
    pub running: bool,
}

fn default_quiet() -> u64 {
    DEFAULT_QUIET
}

impl Lane {
    /// Whether its work is live only once a deploy says so: it declares or
    /// has a confirmed command, or the PM was asked to confirm one.
    pub fn by_deploy(&self) -> bool {
        self.declared || self.confirmed.is_some() || self.asked.is_some()
    }

    /// A landing of `ticket` (none: the PM asked for a deploy now) was pushed at `now`.
    pub fn landed(&mut self, ticket: Option<TicketId>, now: u64) {
        let pending = self.pending.get_or_insert(Pending { first: now, last: now, tickets: Vec::new() });
        pending.last = pending.last.max(now);
        if let Some(t) = ticket.filter(|t| !pending.tickets.contains(t)) {
            pending.tickets.push(t);
        }
    }

    /// A deploy asked for now: due at once.
    pub fn now(&mut self, now: u64) {
        let back = now.saturating_sub(self.quiet.max(MAX_WAIT));
        self.pending = Some(Pending { first: back, last: back, tickets: self.pending.take().map(|p| p.tickets).unwrap_or_default() });
    }

    /// When the landings waiting go live: once none came for the quiet time,
    /// or [`MAX_WAIT`] after the first (never sooner than the quiet time).
    pub fn due_at(&self) -> Option<u64> {
        let p = self.pending.as_ref()?;
        Some((p.last + self.quiet).min(p.first + MAX_WAIT.max(self.quiet)))
    }

    /// Whether its command should start now.
    pub fn is_due(&self, now: u64) -> bool {
        !self.running && self.due_at().is_some_and(|at| at <= now)
    }

    /// Its command starts: the landings it carries.
    pub fn start(&mut self) -> Vec<TicketId> {
        self.running = true;
        self.pending.take().map(|p| p.tickets).unwrap_or_default()
    }

    /// Its command ended: `Ok(commit)` deployed, or why not. Whether this is
    /// news — a failure, or the first success after one.
    pub fn finished(&mut self, result: Result<&str, &str>, now: u64) -> bool {
        self.running = false;
        match result {
            Ok(sha) => {
                self.deployed = Some(sha.to_owned());
                self.deployed_at = now;
                self.stuck = None;
                self.failed.take().is_some()
            }
            Err(why) => {
                self.failed = Some(why.to_owned());
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deploy_file_names_its_command_and_may_tune_it() {
        let go = parse(br#"{ "command": " npm run deploy:prod ", "from": "main" }"#).unwrap();
        assert_eq!((go.command.as_str(), go.from.as_deref(), go.timeout, go.quiet), ("npm run deploy:prod", Some("main"), DEFAULT_TIMEOUT, DEFAULT_QUIET));
        let tuned = parse(br#"{ "command": "make deploy", "timeoutMinutes": 600, "quietSeconds": 5 }"#).unwrap();
        assert_eq!((tuned.timeout, tuned.quiet, tuned.from), (MAX_TIMEOUT, 5, None), "capped, and a short quiet kept");
        assert_eq!(parse(br#"{ "command": "x", "quietSeconds": 99999 }"#).unwrap().quiet, MAX_QUIET);
        assert!(parse(br#"{ "command": "  " }"#).unwrap_err().contains("names no command"));
        assert!(parse(br#"{ "command": "x", "timeoutMinutes": 0 }"#).is_err());
        assert!(parse(br#"{ "command": "x", "run": "y" }"#).unwrap_err().contains("isn't valid"), "an unknown field is a typo, not ignored");
        assert!(parse(b"not json").is_err());
        assert_eq!(GoLive::confirmed(" vercel deploy --prod ").command, "vercel deploy --prod");
    }

    #[test]
    fn an_obvious_deploy_command_is_found_to_suggest_never_to_run() {
        assert_eq!(package_script(br#"{"scripts":{"build":"next build","deploy":"x","deploy:prod":"sh scripts/deploy-prod.sh"}}"#), Some("deploy:prod"));
        assert_eq!(package_script(br#"{"scripts":{"deploy":"vercel --prod"}}"#), Some("deploy"));
        assert_eq!(package_script(br#"{"scripts":{"deploy":"  "}}"#), None);
        assert_eq!(package_script(br#"{"scripts":{"build":"x"}}"#), None);
        assert_eq!(package_script(b"{"), None);
        assert!(makefile_deploys("build:\n\tcargo build\n\ndeploy: build\n\t./ship\n"));
        assert!(makefile_deploys("deploy :\n\t./ship\n"));
        assert!(!makefile_deploys("deploy_dir := out\nredeploy:\n\tx\n"));
    }

    #[test]
    fn landings_close_together_go_live_once_after_the_quiet_time() {
        let mut lane = Lane { quiet: 120, ..Lane::default() };
        assert_eq!(lane.due_at(), None, "nothing landed");
        lane.landed(Some(1), 1_000);
        assert_eq!(lane.due_at(), Some(1_120));
        // Another a minute later: the wait starts again from it.
        lane.landed(Some(2), 1_060);
        lane.landed(Some(2), 1_061);
        assert_eq!(lane.due_at(), Some(1_181));
        assert!(!lane.is_due(1_180) && lane.is_due(1_181));
        // Landings that keep coming still go live MAX_WAIT after the first.
        let mut busy = Lane { quiet: 120, ..Lane::default() };
        for at in (0..20).map(|i| 5_000 + i * 60) {
            busy.landed(Some(at as u32), at);
        }
        assert_eq!(busy.due_at(), Some(5_000 + MAX_WAIT));
        // One run carries them all; a landing while it runs waits for the next.
        assert_eq!(lane.start(), [1, 2]);
        assert!(lane.running && lane.pending.is_none());
        lane.landed(Some(3), 1_300);
        assert!(!lane.is_due(9_999), "never two runs at once");
        assert!(lane.finished(Err("exit 1"), 1_400), "a failure is news");
        assert_eq!(lane.failed.as_deref(), Some("exit 1"));
        assert!(lane.is_due(1_420), "the landing that came meanwhile tries again");
        assert_eq!(lane.start(), [3]);
        assert!(lane.finished(Ok("abc"), 1_500), "live again after a failure is news");
        assert_eq!((lane.deployed.as_deref(), lane.failed.as_deref(), lane.deployed_at), (Some("abc"), None, 1_500));
        lane.landed(None, 2_000);
        lane.start();
        assert!(!lane.finished(Ok("def"), 2_200), "a success after a success is the deploy's own news");
    }

    #[test]
    fn a_deploy_asked_for_now_is_due_at_once_and_a_lane_reads_back_as_kept() {
        let mut lane = Lane { quiet: 120, confirmed: Some("make deploy".into()), ..Lane::default() };
        lane.landed(Some(4), 10_000);
        lane.now(10_001);
        assert!(lane.is_due(10_001));
        assert_eq!(lane.pending.as_ref().map(|p| p.tickets.clone()), Some(vec![4]));
        lane.running = true;
        let kept: Lane = serde_json::from_str(&serde_json::to_string(&lane).unwrap()).unwrap();
        assert!(!kept.running, "a restart ends every run");
        assert_eq!((kept.confirmed.as_deref(), kept.quiet), (Some("make deploy"), 120));
        let old: Lane = serde_json::from_str("{}").unwrap();
        assert_eq!(old.quiet, DEFAULT_QUIET);
    }

    // ── on a repository ──

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_CONFIG_GLOBAL", "/dev/null").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-go-live-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    /// A repository on main with `files` committed, pushed to a bare origin.
    fn repo(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = scratch(name);
        let origin = scratch(&format!("{name}-origin"));
        git_in(&origin, &["init", "-q", "--bare", "-b", "main"]);
        git_in(&dir, &["init", "-q", "-b", "main"]);
        git_in(&dir, &["config", "user.email", "t@example.com"]);
        git_in(&dir, &["config", "user.name", "t"]);
        git_in(&dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("README.md"), "hi").unwrap();
        for (path, text) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        git_in(&dir, &["add", "-A"]);
        git_in(&dir, &["commit", "-q", "-m", "start"]);
        git_in(&dir, &["remote", "add", "origin", origin.to_str().unwrap()]);
        git_in(&dir, &["push", "-q", "origin", "main"]);
        dir
    }

    #[test]
    fn a_project_declares_its_command_or_gets_one_suggested() {
        let declared = repo("declared", &[(FILE, r#"{ "command": "make ship" }"#)]);
        assert_eq!(find(&declared, Some("ignored")), Found::Command(GoLive { command: "make ship".into(), ..GoLive::confirmed("x") }), "the file wins over the PM's");
        let pnpm = repo("pnpm", &[("package.json", r#"{"scripts":{"deploy:prod":"sh x.sh"}}"#), ("pnpm-lock.yaml", "x")]);
        assert_eq!(find(&pnpm, None), Found::Undeclared { suggestion: Some("pnpm run deploy:prod".into()) });
        assert_eq!(find(&pnpm, Some("pnpm run deploy:prod")), Found::Command(GoLive::confirmed("pnpm run deploy:prod")));
        let make = repo("make", &[("Makefile", "deploy:\n\t./ship\n")]);
        assert_eq!(find(&make, None), Found::Undeclared { suggestion: Some("make deploy".into()) });
        let plain = repo("plain", &[]);
        assert_eq!(find(&plain, None), Found::Undeclared { suggestion: None });
        let broken = repo("broken", &[(FILE, "{")]);
        assert!(matches!(find(&broken, Some("x")), Found::Invalid(_)), "a broken file runs nothing, not the PM's either");
        for dir in [declared, pnpm, make, plain, broken] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn it_runs_only_on_a_clean_main_checkout_at_origin_and_says_why_not() {
        let dir = repo("checkout", &[]);
        let go = GoLive::confirmed("true");
        let (at, sha) = checkout(&dir, &go).unwrap();
        assert_eq!((at, sha.clone()), (dir.clone(), git_in(&dir, &["rev-parse", "HEAD"])));
        std::fs::write(dir.join("stray.txt"), "x").unwrap();
        let dirty = checkout(&dir, &go).unwrap_err();
        assert!(matches!(&dirty, Blocked::Dirty { files, .. } if files == &["stray.txt"]), "{dirty:?}");
        assert!(dirty.why().contains("uncommitted files: stray.txt"));
        std::fs::remove_file(dir.join("stray.txt")).unwrap();
        // A commit on main not pushed yet: not what origin has.
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        git_in(&dir, &["add", "-A"]);
        git_in(&dir, &["commit", "-q", "-m", "local"]);
        let behind = checkout(&dir, &go).unwrap_err();
        assert!(matches!(behind, Blocked::NotAtOrigin { origin: Some(ref o), .. } if *o == sha), "{behind:?}");
        assert!(behind.why().contains("not at origin/main"));
        git_in(&dir, &["push", "-q", "origin", "main"]);
        assert!(checkout(&dir, &go).is_ok());
        let other = GoLive { from: Some("production".into()), ..go.clone() };
        assert_eq!(checkout(&dir, &other).unwrap_err(), Blocked::From { from: "production".into(), main: "main".into() });
        // Main checked out nowhere: the squad's folder is on another branch.
        git_in(&dir, &["checkout", "-q", "-b", "side"]);
        assert_eq!(checkout(&dir, &go).unwrap_err(), Blocked::NotCheckedOut { main: "main".into() });
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_run_says_how_it_ended_with_the_last_of_its_output() {
        let dir = scratch("run");
        let log = dir.join("logs/deploy.log");
        let ok = run(&dir, "echo \"$EQUIPE_DEPLOY\" > ran.txt; echo deployed", Duration::from_secs(30), &log);
        assert!(ok.ok && ok.code == Some(0) && ok.tail == "deployed", "{ok:?}");
        assert_eq!(std::fs::read_to_string(dir.join("ran.txt")).unwrap().trim(), "1");
        let failed = run(&dir, "for i in $(seq 1 50); do echo line $i; done; echo 'Error: no token' >&2; exit 3", Duration::from_secs(30), &log);
        assert!(!failed.ok && failed.code == Some(3) && !failed.timed_out, "{failed:?}");
        assert!(failed.tail.ends_with("Error: no token") && failed.tail.lines().count() == TAIL_LINES, "{}", failed.tail);
        let slow = run(&dir, "echo started; sleep 30", Duration::from_millis(300), &log);
        assert!(!slow.ok && slow.timed_out && slow.tail == "started", "{slow:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
