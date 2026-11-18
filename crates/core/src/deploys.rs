//! What went live ([`equipe_protocol::deploys`]): the rules, with no runtime
//! in them, so each is tested alone.
//!
//! - **Where a project's main stands, and how it goes live** ([`look`]):
//!   off the daemon's loop, a few git commands and, for a GitHub project with
//!   new commits, a few `gh api` calls. A project that reports production
//!   deployments (Vercel does, for every push) is live once one of them
//!   succeeded for a commit on main; one with a deploy workflow, once that
//!   workflow's run on main succeeded; one with neither (or no GitHub), once
//!   its commits are on main — as far as equipe can see.
//! - **What it brought** ([`parse_log`], [`note`]): the tickets equipe merged
//!   in that range of commits (its merge commits say `equipe: merge T-12 …`),
//!   one plain line each from the ticket's title, grouped by the person's
//!   request when equipe knows it; checks, docs, tests and tidying are
//!   counted, not listed ([`internal`]).
//! - **Told once, together** ([`fold`], [`due`], [`line`]): deploys of a
//!   project within [`WINDOW`] of each other are one note, told when nothing
//!   more went live for [`QUIET`], or when the window closes.
//!
//! No model writes any of it: the notes are the tickets' own titles, which
//! the PM writes in the person's language.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use equipe_protocol::deploys::{Deploy, DeploySource, NoteItem};
use equipe_protocol::title::TicketLabel;
use equipe_protocol::{SessionId, Ticket, TicketId, TicketKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::i18n::Catalog;
use crate::worktree;

/// Deploys of one project this close to the first are told as one note.
pub const WINDOW: u64 = 10 * 60;
/// A note is told once nothing more went live for this long (or its window closed).
pub const QUIET: u64 = 3 * 60;
/// Deploys kept per squad, the newest.
pub const KEPT: usize = 60;
/// Lines a note names at most; the rest are "+N more".
pub const SHOWN: usize = 5;
/// GitHub is asked about a project with commits not yet live at most this often.
pub const LOOK_EVERY: u64 = 2 * 60;
/// How long what a project goes live with is trusted before it is looked at again.
const CHANNEL_FRESH: u64 = 6 * 60 * 60;
/// ...or, when GitHub couldn't say, this long.
const CHANNEL_RETRY: u64 = 30 * 60;
/// A production deployment older than this doesn't make a project one that
/// deploys that way any more.
const CHANNEL_WITHIN: u64 = 30 * 24 * 60 * 60;
/// The longest one `gh api` call may take.
const GH_TIMEOUT: Duration = Duration::from_secs(20);
/// Deployments whose statuses are asked for in one look, at most.
const STATUSES_ASKED: usize = 5;

/// How a project goes live, as last looked at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum Channel {
    /// Production deployments reported to GitHub (Vercel, Netlify, an
    /// Actions job with an environment).
    Deployments,
    /// These deploy workflows (GitHub Actions), by id.
    Workflows { ids: Vec<u64> },
    /// No deploy signal: main is as far as equipe sees.
    Main,
}

/// What equipe keeps of each project it watches (`meta` key `deploy_watch`, by path).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watch {
    /// The newest commit told live — or, at first, where watching started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<Channel>,
    /// When the channel was looked at, and whether GitHub couldn't say then.
    #[serde(default)]
    pub channel_at: u64,
    #[serde(default)]
    pub channel_failed: bool,
    /// The site's address, as the repository names it (its homepage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// When GitHub was last asked about commits not yet live.
    #[serde(default)]
    pub looked_at: u64,
}

/// A commit known to be live, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Live {
    pub sha: String,
    pub at: u64,
    pub url: Option<String>,
    pub environment: Option<String>,
    pub source: DeploySource,
}

/// One commit on main in a deploy's range, as `git log` gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merge {
    pub sha: String,
    pub subject: String,
    /// The tickets it merged, from its subject.
    pub tickets: Vec<TicketId>,
    /// The files it changed against main before it.
    pub files: Vec<String>,
}

/// What one look at a project found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Looked {
    /// Nothing new live, or nothing to look at.
    Nothing,
    /// Watching starts at this commit: what came before isn't news.
    Start(String),
    /// Commits after `from` up to `live.sha` are live.
    Live { from: String, live: Live, merges: Vec<Merge> },
}

/// A request of the person's a ticket belongs to (`requests`, when the
/// squad's journal keeps them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestOf {
    pub id: u64,
    pub title: String,
    pub session: Option<SessionId>,
}

// ── GitHub ──

/// What reads GitHub's API: `gh` as the person signed it in, or a stand-in.
pub trait GitHub {
    /// The JSON at `path` (`repos/o/r/deployments?…`).
    fn get(&self, path: &str) -> Result<Value, String>;
}

/// `gh api`, as the person (or the machine) signed it in.
pub struct Gh;

impl GitHub for Gh {
    fn get(&self, path: &str) -> Result<Value, String> {
        let mut command = std::process::Command::new("gh");
        command.args(["api", "-H", "Accept: application/vnd.github+json", path]);
        // Nobody is at the daemon's terminal: no prompt, no update notice.
        command.env("GH_PROMPT_DISABLED", "1").env("GH_NO_UPDATE_NOTIFIER", "1").env("NO_COLOR", "1");
        let out = worktree::output_within(command, GH_TIMEOUT)?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("gh api {path}: {}", err.lines().next().unwrap_or("failed").trim()));
        }
        serde_json::from_slice(&out.stdout).map_err(|e| format!("gh api {path}: {e}"))
    }
}

/// `owner/repo` of a GitHub remote, in any of the shapes git keeps it.
pub fn github_repo(url: &str) -> Option<String> {
    let url = url.trim();
    let path = match url.strip_prefix("git@github.com:") {
        Some(rest) => rest,
        None => {
            let (_, rest) = url.split_once("://")?;
            let (authority, path) = rest.split_once('/')?;
            // Past any user and token, and before any port.
            let host = authority.rsplit('@').next().unwrap_or(authority);
            let host = host.split(':').next().unwrap_or(host);
            if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com") {
                return None;
            }
            path
        }
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    (ok(owner) && ok(repo)).then(|| format!("{owner}/{repo}"))
}

/// Whether a deployment's environment is production: GitHub's flag, or its
/// name (`Production`, `Production – site`, `prod`).
pub fn production(environment: &str, flagged: bool) -> bool {
    let name = environment.trim().to_lowercase();
    flagged || name.starts_with("production") || name == "prod" || name == "live"
}

/// A production deployment on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deployment {
    pub id: u64,
    pub sha: String,
    pub environment: String,
    pub created: u64,
}

/// The production deployments in a `deployments` answer, newest first as GitHub lists them.
pub fn deployments(json: &Value) -> Vec<Deployment> {
    json.as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let environment = d.get("environment")?.as_str()?.to_owned();
            let flagged = d.get("production_environment").and_then(Value::as_bool).unwrap_or(false);
            production(&environment, flagged).then_some(())?;
            Some(Deployment {
                id: d.get("id")?.as_u64()?,
                sha: d.get("sha")?.as_str()?.to_owned(),
                created: d.get("created_at").and_then(Value::as_str).and_then(crate::quality::live::unix_of).unwrap_or(0),
                environment,
            })
        })
        .collect()
}

/// When a deployment succeeded, and where it is — from its statuses (newest
/// first). A deployment replaced since reads `inactive` now, but its
/// `success` is still among them.
pub fn success(statuses: &Value) -> Option<(u64, Option<String>)> {
    let s = statuses.as_array()?.iter().find(|s| s.get("state").and_then(Value::as_str) == Some("success"))?;
    let at = s.get("created_at").and_then(Value::as_str).and_then(crate::quality::live::unix_of).unwrap_or(0);
    let url = ["environment_url", "target_url"].into_iter().find_map(|k| s.get(k).and_then(Value::as_str).filter(|u| u.starts_with("http")).map(str::to_owned));
    Some((at, url))
}

/// The deploy workflows of a `actions/workflows` answer: active, and named
/// for deploying in their file or name.
pub fn deploy_workflows(json: &Value) -> Vec<u64> {
    json.get("workflows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|w| w.get("state").and_then(Value::as_str) == Some("active"))
        .filter(|w| {
            let path = w.get("path").and_then(Value::as_str).unwrap_or("");
            let file = path.rsplit('/').next().unwrap_or(path).to_lowercase();
            let name = w.get("name").and_then(Value::as_str).unwrap_or("").to_lowercase();
            file.starts_with("deploy") || file.contains("-deploy") || file.contains("_deploy") || name.starts_with("deploy")
        })
        .filter_map(|w| w.get("id").and_then(Value::as_u64))
        .collect()
}

/// The successful runs on `main` in a workflow's `runs` answer, as commits live.
pub fn runs(json: &Value, main: &str) -> Vec<Live> {
    json.get("workflow_runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| r.get("conclusion").and_then(Value::as_str) == Some("success"))
        .filter(|r| r.get("head_branch").and_then(Value::as_str).is_none_or(|b| b == main))
        .filter_map(|r| {
            Some(Live {
                sha: r.get("head_sha")?.as_str()?.to_owned(),
                at: r.get("updated_at").and_then(Value::as_str).and_then(crate::quality::live::unix_of).unwrap_or(0),
                url: None,
                environment: r.get("name").and_then(Value::as_str).map(str::to_owned),
                source: DeploySource::Workflow,
            })
        })
        .collect()
}

/// How the project at GitHub's `repo` goes live, and its site's address:
/// production deployments lately, else a deploy workflow, else main. `Err`
/// when GitHub couldn't say.
pub fn channel(gh: &dyn GitHub, repo: &str, now: u64) -> Result<(Channel, Option<String>), String> {
    let site = gh.get(&format!("repos/{repo}"))?.get("homepage").and_then(Value::as_str).map(str::trim).filter(|h| !h.is_empty()).map(|h| {
        if h.contains("://") { h.to_owned() } else { format!("https://{h}") }
    });
    let recent = deployments(&gh.get(&format!("repos/{repo}/deployments?per_page=30"))?);
    if recent.iter().any(|d| now.saturating_sub(d.created) < CHANNEL_WITHIN) {
        return Ok((Channel::Deployments, site));
    }
    let ids = deploy_workflows(&gh.get(&format!("repos/{repo}/actions/workflows?per_page=100"))?);
    Ok((if ids.is_empty() { Channel::Main } else { Channel::Workflows { ids } }, site))
}

// ── git ──

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    worktree::git(repo, args).ok()
}

fn is_ancestor(repo: &Path, commit: &str, of: &str) -> bool {
    git(repo, &["merge-base", "--is-ancestor", commit, of]).is_some()
}

/// Where main stands as far as its users can have it: origin's main as equipe
/// last pushed or fetched it, else the local main (a project with no remote).
/// Its name, and the commit.
pub fn head(repo: &Path) -> Option<(String, String)> {
    let main = worktree::main_branch(repo).ok()?;
    let sha = git(repo, &["rev-parse", "-q", "--verify", &format!("refs/remotes/origin/{main}^{{commit}}")])
        .or_else(|| git(repo, &["rev-parse", "-q", "--verify", &format!("refs/heads/{main}^{{commit}}")]))?;
    Some((main, sha))
}

/// Whether `sha` came after `from` and is in `head`.
fn in_range(repo: &Path, from: &str, sha: &str, head: &str) -> bool {
    sha != from && is_ancestor(repo, from, sha) && is_ancestor(repo, sha, head)
}

/// What `git log --first-parent --diff-merges=first-parent --name-only
/// --format=%x1e%H%x1f%s` printed: each commit with its subject, its tickets and its files.
pub fn parse_log(text: &str) -> Vec<Merge> {
    text.split('\u{1e}')
        .filter_map(|record| {
            let mut lines = record.lines();
            let (sha, subject) = lines.next()?.split_once('\u{1f}')?;
            let files = lines.map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
            Some(Merge { sha: sha.trim().to_owned(), tickets: tickets_in(subject), subject: subject.trim().to_owned(), files })
        })
        .collect()
}

/// The tickets a commit's subject says it merged: equipe's own merges
/// (`equipe: merge T-12 …`), or a person's merge of a member's branch
/// (`Merge branch 'equipe/frontend-t12-…'`).
pub fn tickets_in(subject: &str) -> Vec<TicketId> {
    let digits = |s: &str| -> Option<TicketId> {
        let d: String = s.chars().take_while(char::is_ascii_digit).collect();
        d.parse().ok()
    };
    if let Some(rest) = subject.strip_prefix("equipe: merge T-") {
        return digits(rest).into_iter().collect();
    }
    if let Some((_, branch)) = subject.split_once("equipe/") {
        let name = branch.split(['\'', '"', ' ']).next().unwrap_or(branch);
        return name.match_indices("-t").find_map(|(at, _)| digits(&name[at + 2..])).into_iter().collect();
    }
    Vec::new()
}

/// The commits after `from` up to `to` on main, newest first.
pub fn merges(repo: &Path, from: &str, to: &str) -> Vec<Merge> {
    let range = format!("{from}..{to}");
    let text = git(repo, &["log", "--first-parent", "--diff-merges=first-parent", "--name-only", "--format=%x1e%H%x1f%s", &range]).unwrap_or_default();
    parse_log(&text)
}

/// One look at the project at `repo`, from what equipe kept of it: whether
/// commits came live since, and what is kept now. Blocks (git, and GitHub
/// when there is something new): call it off the loop.
pub fn look(repo: &Path, mut watch: Watch, gh: &dyn GitHub, now: u64) -> (Watch, Looked) {
    let Some((main, head)) = head(repo) else { return (watch, Looked::Nothing) };
    let Some(from) = watch.live.clone() else {
        watch.live = Some(head.clone());
        return (watch, Looked::Start(head));
    };
    if from == head {
        return (watch, Looked::Nothing);
    }
    // History rewritten under it: start again from here, telling nothing.
    if !is_ancestor(repo, &from, &head) {
        watch.live = Some(head.clone());
        return (watch, Looked::Start(head));
    }
    let repo_name = git(repo, &["remote", "get-url", "origin"]).as_deref().and_then(github_repo);
    let fresh = if watch.channel_failed { CHANNEL_RETRY } else { CHANNEL_FRESH };
    if let Some(name) = &repo_name
        && (watch.channel.is_none() || now.saturating_sub(watch.channel_at) >= fresh)
    {
        match channel(gh, name, now) {
            Ok((channel, site)) => {
                watch.channel = Some(channel);
                watch.site = site;
                watch.channel_failed = false;
            }
            // GitHub couldn't say (gh missing, signed out): main it is, for now.
            Err(err) => {
                crate::daemon_log!("equipe: deploys: couldn't ask GitHub how {name} goes live: {err}");
                watch.channel = Some(Channel::Main);
                watch.channel_failed = true;
            }
        }
        watch.channel_at = now;
    }
    let channel = if repo_name.is_some() { watch.channel.clone().unwrap_or(Channel::Main) } else { Channel::Main };
    let live = match (&channel, &repo_name) {
        (Channel::Main, _) | (_, None) => Some(Live { sha: head.clone(), at: now, url: watch.site.clone(), environment: None, source: DeploySource::Main }),
        (_, Some(_)) if now.saturating_sub(watch.looked_at) < LOOK_EVERY => return (watch, Looked::Nothing),
        (Channel::Deployments, Some(name)) => {
            watch.looked_at = now;
            newest_deployment(repo, gh, name, &from, &head).map(|l| Live { url: watch.site.clone().or(l.url), ..l })
        }
        (Channel::Workflows { ids }, Some(name)) => {
            watch.looked_at = now;
            newest_run(repo, gh, name, ids, &main, &from, &head).map(|l| Live { url: watch.site.clone(), ..l })
        }
    };
    let Some(live) = live else { return (watch, Looked::Nothing) };
    let merges = merges(repo, &from, &live.sha);
    watch.live = Some(live.sha.clone());
    (watch, Looked::Live { from, live, merges })
}

/// The newest production deployment that succeeded for a commit after
/// `from` on main.
fn newest_deployment(repo: &Path, gh: &dyn GitHub, name: &str, from: &str, head: &str) -> Option<Live> {
    let listed = deployments(&gh.get(&format!("repos/{name}/deployments?per_page=20")).ok()?);
    let mut asked = 0;
    for d in listed {
        if !in_range(repo, from, &d.sha, head) {
            // Older than what was told live already: nothing further down is new.
            if d.sha == from || is_ancestor(repo, &d.sha, from) {
                break;
            }
            continue;
        }
        if asked == STATUSES_ASKED {
            break;
        }
        asked += 1;
        let Ok(statuses) = gh.get(&format!("repos/{name}/deployments/{}/statuses?per_page=20", d.id)) else { continue };
        if let Some((at, url)) = success(&statuses) {
            return Some(Live { sha: d.sha, at, url, environment: Some(d.environment), source: DeploySource::Deployment });
        }
    }
    None
}

/// The newest successful deploy run on main for a commit after `from`.
fn newest_run(repo: &Path, gh: &dyn GitHub, name: &str, ids: &[u64], main: &str, from: &str, head: &str) -> Option<Live> {
    ids.iter()
        .filter_map(|id| gh.get(&format!("repos/{name}/actions/workflows/{id}/runs?branch={main}&status=success&per_page=10")).ok())
        .flat_map(|json| runs(&json, main))
        .filter(|l| in_range(repo, from, &l.sha, head))
        .max_by_key(|l| l.at)
}

// ── the note ──

/// A path nobody using the product sees changed: tests, docs, the CI, the
/// design specs, tooling's settings, lockfiles.
pub fn internal_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    let parts: Vec<&str> = lower.split('/').collect();
    let file = parts.last().copied().unwrap_or("");
    let dirs = &parts[..parts.len().saturating_sub(1)];
    const TEST_DIRS: [&str; 11] = ["test", "tests", "__tests__", "spec", "specs", "e2e", "__snapshots__", "__mocks__", "fixtures", "cypress", "playwright"];
    const OWN_DIRS: [&str; 7] = ["docs", "doc", "design", ".github", ".husky", ".vscode", ".storybook"];
    const LOCKS: [&str; 7] = ["package-lock.json", "yarn.lock", "pnpm-lock.yaml", "bun.lockb", "cargo.lock", "poetry.lock", "uv.lock"];
    if dirs.iter().any(|d| TEST_DIRS.contains(d)) || dirs.first().is_some_and(|d| OWN_DIRS.contains(d)) {
        return true;
    }
    let stem_has = |mark: &str| file.split('.').skip(1).any(|part| part == mark);
    if stem_has("test") || stem_has("spec") || stem_has("stories") || file.contains("_test.") || file.starts_with("test_") {
        return true;
    }
    let at_root = dirs.is_empty();
    let doc = [".md", ".mdx", ".txt", ".rst"].iter().any(|e| file.ends_with(e));
    let readme = ["readme", "changelog", "contributing", "license", "agents.md", "claude.md"].iter().any(|n| file.starts_with(n));
    let tooling = ["tsconfig", "eslint", ".eslintrc", ".prettierrc", "prettier.config", "vitest.config", "jest.config", "playwright.config", ".editorconfig", ".gitignore", ".gitattributes"]
        .iter()
        .any(|n| file.starts_with(n));
    LOCKS.contains(&file) || readme || tooling || (doc && at_root)
}

/// Work nobody using the product sees: a check or chore by its label, work
/// that doesn't ship (a task, an action), a conventional `refactor:`/`test:`/
/// `docs:`… title, or only internal paths changed ([`internal_path`]).
pub fn internal(t: &Ticket, files: &[String]) -> bool {
    if matches!(t.label, Some(TicketLabel::Check | TicketLabel::Chore)) || t.kind != TicketKind::Code {
        return true;
    }
    let title = t.title.trim().to_ascii_lowercase();
    let conventional = ["refactor", "test", "tests", "docs", "doc", "chore", "ci", "build", "style"]
        .iter()
        .any(|p| title.strip_prefix(p).is_some_and(|rest| rest.starts_with(':') || rest.starts_with('(')));
    conventional || (!files.is_empty() && files.iter().all(|f| internal_path(f)))
}

/// What a deploy brought, as the person reads it: one line per ticket they
/// can see (its title), oldest first and those of one request together, the
/// person's earliest request first; how many changes nobody sees came along
/// (internal tickets, and commits that aren't a ticket's); and every ticket.
pub fn note(merges: &[Merge], tickets: &[Ticket], requests: &HashMap<TicketId, RequestOf>) -> (Vec<NoteItem>, usize, Vec<TicketId>) {
    let mut items: Vec<NoteItem> = Vec::new();
    let mut internal_count = 0;
    let mut all = Vec::new();
    // Oldest first: git log gives the newest first.
    for m in merges.iter().rev() {
        if m.tickets.is_empty() {
            internal_count += 1;
            continue;
        }
        for &id in &m.tickets {
            if all.contains(&id) {
                continue;
            }
            all.push(id);
            let Some(t) = tickets.iter().find(|t| t.id == id) else {
                // A ticket gone from the board: its merge's subject names it.
                let title = m.subject.split_once(&format!("T-{id}")).map_or("", |(_, rest)| rest).trim().to_owned();
                if title.is_empty() {
                    internal_count += 1;
                } else {
                    push(&mut items, NoteItem { title, tickets: vec![id], request: None, request_title: None, session: None });
                }
                continue;
            };
            if internal(t, &m.files) {
                internal_count += 1;
                continue;
            }
            let request = requests.get(&id);
            push(
                &mut items,
                NoteItem {
                    title: t.title.trim().to_owned(),
                    tickets: vec![id],
                    request: request.map(|r| r.id),
                    request_title: request.map(|r| r.title.clone()),
                    session: request.and_then(|r| r.session),
                },
            );
        }
    }
    // One request's lines together, in the order its first line came.
    let order: Vec<Option<u64>> = items.iter().map(|i| i.request).fold(Vec::new(), |mut seen, r| {
        if r.is_none() || !seen.contains(&r) {
            seen.push(r);
        }
        seen
    });
    let mut grouped: Vec<NoteItem> = Vec::with_capacity(items.len());
    let mut loose = items.iter().filter(|i| i.request.is_none());
    for r in order {
        match r {
            Some(_) => grouped.extend(items.iter().filter(|i| i.request == r).cloned()),
            None => grouped.extend(loose.next().cloned()),
        }
    }
    (grouped, internal_count, all)
}

/// `item` added — into the line already saying the same, if one does.
fn push(items: &mut Vec<NoteItem>, item: NoteItem) {
    match items.iter_mut().find(|i| i.title == item.title) {
        Some(same) => same.tickets.extend(item.tickets),
        None => items.push(item),
    }
}

// ── told once, together ──

/// `new` among `deploys` (the newest first): folded into the same project's
/// deploy not told yet when it came within [`WINDOW`] of that one, else
/// kept as the newest. The deploys kept stay at [`KEPT`]. Its id.
pub fn fold(deploys: &mut Vec<Deploy>, mut new: Deploy) -> u64 {
    if let Some(open) = deploys.iter_mut().find(|d| d.path == new.path && !d.told && d.source == new.source && new.at.saturating_sub(d.at) < WINDOW) {
        open.to = new.to;
        open.last_at = new.at.max(open.last_at);
        open.url = new.url.or(open.url.take());
        open.environment = new.environment.or(open.environment.take());
        for item in new.items {
            push(&mut open.items, item);
        }
        open.internal += new.internal;
        for t in new.tickets {
            if !open.tickets.contains(&t) {
                open.tickets.push(t);
            }
        }
        return open.id;
    }
    new.id = deploys.iter().map(|d| d.id).max().unwrap_or(0) + 1;
    new.last_at = new.last_at.max(new.at);
    let id = new.id;
    deploys.insert(0, new);
    deploys.truncate(KEPT);
    id
}

/// Whether a deploy's news is due: not told yet, and nothing more went live
/// for [`QUIET`] — or its [`WINDOW`] closed.
pub fn due(d: &Deploy, now: u64) -> bool {
    !d.told && now >= (d.at + WINDOW).min(d.last_at.max(d.at) + QUIET)
}

/// What a deploy's news says, in one line: where it is live and what the
/// person can see now (at most [`SHOWN`] lines), and how many changes
/// nobody sees came with it. `session`: only the lines asked for in that
/// conversation ("What you asked for is live…").
pub fn line(d: &Deploy, words: &Catalog, session: Option<SessionId>) -> String {
    let items: Vec<&NoteItem> = d.items.iter().filter(|i| session.is_none() || i.session == session).collect();
    let head = match (session.is_some(), d.source.is_live(), d.host()) {
        (true, true, Some(host)) => words.fill("deploy.yours_live_on", &[("host", host.to_owned())]),
        (true, true, None) => words.fill("deploy.yours_live", &[("project", d.project.clone())]),
        (true, false, _) => words.fill("deploy.yours_on_main", &[("project", d.project.clone())]),
        (false, true, Some(host)) => words.fill("deploy.live_on", &[("host", host.to_owned())]),
        (false, true, None) => words.fill("deploy.live", &[("project", d.project.clone())]),
        (false, false, _) => words.fill("deploy.on_main", &[("project", d.project.clone())]),
    };
    if items.is_empty() {
        return format!("{head} {}", words.fill("deploy.only_internal", &[("count", d.internal.to_string())]));
    }
    let mut parts: Vec<String> = items.iter().take(SHOWN).map(|i| format!("• {}", i.title)).collect();
    if items.len() > SHOWN {
        parts.push(words.fill("deploy.more", &[("count", (items.len() - SHOWN).to_string())]));
    }
    if session.is_none() && d.internal > 0 {
        parts.push(words.fill("deploy.internal", &[("count", d.internal.to_string())]));
    }
    format!("{head} {}", parts.join(" "))
}

/// The conversations a deploy's lines were asked in, each once, in order.
pub fn sessions(d: &Deploy) -> Vec<SessionId> {
    let mut out = Vec::new();
    for s in d.items.iter().filter_map(|i| i.session) {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::process::Command;

    fn ticket(id: TicketId, title: &str, kind: TicketKind, label: Option<TicketLabel>) -> Ticket {
        let mut t: Ticket = serde_json::from_value(serde_json::json!({
            "id": id, "title": title, "body": "", "status": "done", "kind": "code", "owner": null, "assignee": null,
            "project": "/c/site", "depends_on": [], "branch": null, "merged": true, "created_by": "pm", "note": null
        }))
        .unwrap();
        t.kind = kind;
        t.label = label;
        t
    }

    fn merge(id: TicketId, files: &[&str]) -> Merge {
        Merge { sha: format!("s{id}"), subject: format!("equipe: merge T-{id} x"), tickets: vec![id], files: files.iter().map(|f| f.to_string()).collect() }
    }

    fn deploy(path: &str, at: u64, items: &[&str]) -> Deploy {
        Deploy {
            id: 0,
            project: "site".into(),
            path: PathBuf::from(path),
            source: DeploySource::Deployment,
            environment: Some("Production".into()),
            url: Some("https://example.com".into()),
            from: Some("a".into()),
            to: format!("t{at}"),
            at,
            last_at: at,
            items: items.iter().map(|t| NoteItem { title: t.to_string(), tickets: vec![], request: None, request_title: None, session: None }).collect(),
            internal: 0,
            tickets: vec![],
            told: false,
        }
    }

    #[test]
    fn a_github_remote_names_its_repository_in_every_shape() {
        for url in [
            "https://github.com/acme/site.git",
            "https://github.com/acme/site",
            "https://x-access-token:abc@github.com/acme/site.git",
            "git@github.com:acme/site.git",
            "ssh://git@github.com/acme/site.git",
            "https://github.com/acme/site/",
        ] {
            assert_eq!(github_repo(url).as_deref(), Some("acme/site"), "{url}");
        }
        assert_eq!(github_repo("https://gitlab.com/acme/site.git"), None);
        assert_eq!(github_repo("/srv/git/site.git"), None);
        assert_eq!(github_repo("https://notgithub.com/acme/site"), None);
    }

    #[test]
    fn a_production_environment_is_flagged_or_named_so() {
        assert!(production("Production", false));
        assert!(production("Production – atinum-website", false));
        assert!(production("staging", true));
        assert!(!production("Preview – site", false));
        assert!(!production("staging", false));
    }

    #[test]
    fn vercels_deployments_and_statuses_read_as_live_commits() {
        let listed = serde_json::json!([
            {"id": 3, "sha": "ccc", "environment": "Preview – site", "production_environment": false, "created_at": "2026-11-18T00:03:00Z"},
            {"id": 2, "sha": "bbb", "environment": "Production – site", "production_environment": false, "created_at": "2026-11-18T00:02:00Z"},
            {"id": 1, "sha": "aaa", "environment": "Production", "created_at": "2026-11-18T00:01:00Z"}
        ]);
        let found = deployments(&listed);
        assert_eq!(found.iter().map(|d| d.id).collect::<Vec<_>>(), [2, 1], "previews aren't production");
        // Replaced since: inactive now, its success still listed.
        let statuses = serde_json::json!([
            {"state": "inactive", "created_at": "2026-11-18T01:00:00Z", "environment_url": ""},
            {"state": "success", "created_at": "2026-11-18T00:04:00Z", "environment_url": "https://site-abc.vercel.app", "target_url": "https://vercel.com/x"},
            {"state": "in_progress", "created_at": "2026-11-18T00:02:30Z"}
        ]);
        let (at, url) = success(&statuses).unwrap();
        assert_eq!(at, crate::quality::live::unix_of("2026-11-18T00:04:00Z").unwrap());
        assert_eq!(url.as_deref(), Some("https://site-abc.vercel.app"));
        assert_eq!(success(&serde_json::json!([{"state": "failure"}, {"state": "pending"}])), None, "never succeeded: not live");
    }

    #[test]
    fn a_deploy_workflow_is_found_by_its_file_and_its_runs_say_what_went_live() {
        let workflows = serde_json::json!({"workflows": [
            {"id": 1, "name": "CI", "path": ".github/workflows/ci.yml", "state": "active"},
            {"id": 2, "name": "Deploy", "path": ".github/workflows/deploy.yml", "state": "active"},
            {"id": 3, "name": "Runtime", "path": ".github/workflows/deploy-runtime.yml", "state": "active"},
            {"id": 4, "name": "Old deploy", "path": ".github/workflows/deploy-old.yml", "state": "disabled_manually"},
            {"id": 5, "name": "Deal pipeline", "path": ".github/workflows/deal-pipeline.yml", "state": "active"}
        ]});
        assert_eq!(deploy_workflows(&workflows), [2, 3]);
        let listed = serde_json::json!({"workflow_runs": [
            {"head_sha": "bbb", "head_branch": "main", "conclusion": "failure", "updated_at": "2026-11-18T00:05:00Z"},
            {"head_sha": "aaa", "head_branch": "main", "conclusion": "success", "updated_at": "2026-11-18T00:03:00Z", "name": "Deploy"},
            {"head_sha": "zzz", "head_branch": "other", "conclusion": "success", "updated_at": "2026-11-18T00:04:00Z"}
        ]});
        let live = runs(&listed, "main");
        assert_eq!(live.iter().map(|l| l.sha.as_str()).collect::<Vec<_>>(), ["aaa"]);
        assert_eq!(live[0].source, DeploySource::Workflow);
    }

    #[test]
    fn a_merge_commit_names_its_ticket_and_the_log_its_files() {
        assert_eq!(tickets_in("equipe: merge T-3663 Model chip mark"), [3663]);
        assert_eq!(tickets_in("Merge branch 'equipe/frontend-t3663-model-chip' into main"), [3663]);
        assert_eq!(tickets_in("Merge branch 'equipe/frontend-2-t12-x'"), [12]);
        assert!(tickets_in("Fix typo").is_empty());
        let log = "\u{1e}aaa\u{1f}equipe: merge T-2 New input box\n\ncomponents/Box.tsx\ntests/box.test.ts\n\u{1e}bbb\u{1f}Bump deps\n\npackage-lock.json\n";
        let merges = parse_log(log);
        assert_eq!(merges.len(), 2);
        assert_eq!((merges[0].sha.as_str(), merges[0].tickets.as_slice(), merges[0].files.len()), ("aaa", &[2][..], 2));
        assert!(merges[1].tickets.is_empty());
    }

    #[test]
    fn checks_docs_tests_and_tidying_are_internal() {
        let code = |title: &str| ticket(1, title, TicketKind::Code, None);
        let files = |f: &[&str]| f.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!internal(&code("New input box"), &files(&["components/Box.tsx", "tests/box.test.ts"])), "a screen changed");
        assert!(internal(&code("Faster tests"), &files(&["tests/a.test.ts", "src/b.spec.ts", "__snapshots__/c.snap"])), "tests only");
        assert!(internal(&code("Spec update"), &files(&["design/components/composer.md", "docs/a.md", "README.md"])), "docs and specs only");
        assert!(internal(&code("Pin tools"), &files(&[".github/workflows/ci.yml", "package-lock.json", "tsconfig.json"])), "the CI and tooling");
        assert!(!internal(&code("Blog post"), &files(&["content/blog/hello.mdx"])), "a page of the site is seen");
        assert!(internal(&code("refactor: split the store"), &files(&["src/store.ts"])), "said to be a refactor");
        assert!(internal(&code("docs(readme): install"), &[]));
        assert!(!internal(&code("Refactored checkout is faster"), &files(&["src/checkout.ts"])), "only the conventional prefix counts");
        assert!(internal(&ticket(1, "Run the QA", TicketKind::Code, Some(TicketLabel::Check)), &files(&["src/a.ts"])));
        assert!(internal(&ticket(1, "Tidy", TicketKind::Code, Some(TicketLabel::Chore)), &files(&["src/a.ts"])));
        assert!(internal(&ticket(1, "Design QA: T-3 box", TicketKind::Task, None), &[]), "a task ships nothing");
        assert!(!internal(&ticket(1, "Left menu", TicketKind::Code, Some(TicketLabel::Design)), &files(&["src/menu.tsx"])));
    }

    #[test]
    fn a_note_lists_what_the_person_sees_by_request_and_counts_the_rest() {
        let tickets = vec![
            ticket(1, "New Tinumi input box", TicketKind::Code, Some(TicketLabel::Feature)),
            ticket(2, "Left menu collapses to icons", TicketKind::Code, Some(TicketLabel::Design)),
            ticket(3, "Fix flaky test", TicketKind::Code, Some(TicketLabel::Chore)),
            ticket(4, "Claude mark in the model picker", TicketKind::Code, None),
            ticket(5, "Input box keeps focus", TicketKind::Code, Some(TicketLabel::Fix)),
        ];
        // Newest first, as git log gives them.
        let merges = vec![
            merge(5, &["components/Box.tsx"]),
            Merge { sha: "x".into(), subject: "Hotfix by hand".into(), tickets: vec![], files: vec!["src/a.ts".into()] },
            merge(4, &["components/Picker.tsx"]),
            merge(3, &["tests/a.test.ts"]),
            merge(2, &["components/Menu.tsx"]),
            merge(1, &["components/Box.tsx"]),
            merge(9, &["src/gone.ts"]),
        ];
        let mut requests = HashMap::new();
        requests.insert(1, RequestOf { id: 7, title: "Jahed-style input".into(), session: Some(11) });
        requests.insert(5, RequestOf { id: 7, title: "Jahed-style input".into(), session: Some(11) });
        requests.insert(2, RequestOf { id: 8, title: "Menu".into(), session: Some(12) });
        let (items, internal, all) = note(&merges, &tickets, &requests);
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["x", "New Tinumi input box", "Input box keeps focus", "Left menu collapses to icons", "Claude mark in the model picker"], "a gone ticket by its merge's words, then one request's lines together");
        assert_eq!(internal, 2, "the chore and the commit that isn't a ticket's");
        assert_eq!(all, [9, 1, 2, 3, 4, 5]);
        assert_eq!(items[1].session, Some(11));
        assert_eq!(items[1].request_title.as_deref(), Some("Jahed-style input"));
        assert_eq!(items[4].request, None);
    }

    #[test]
    fn deploys_close_together_are_one_note_told_once_quiet() {
        let mut kept = Vec::new();
        let first = fold(&mut kept, deploy("/c/site", 1_000, &["A"]));
        let second = fold(&mut kept, deploy("/c/site", 1_000 + 120, &["B", "A"]));
        assert_eq!(first, second, "two minutes later: the same note");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["A", "B"]);
        assert_eq!((kept[0].to.as_str(), kept[0].last_at), ("t1120", 1_120));
        let other = fold(&mut kept, deploy("/c/api", 1_130, &["C"]));
        assert_ne!(other, first, "another project: its own note");
        // Told once nothing more came for a while.
        assert!(!due(&kept[1], 1_120 + QUIET - 1));
        assert!(due(&kept[1], 1_120 + QUIET));
        // Deploys that keep coming: told when the window closes all the same.
        let mut busy = deploy("/c/site", 5_000, &["A"]);
        busy.last_at = 5_000 + WINDOW - 30;
        assert!(due(&busy, 5_000 + WINDOW));
        kept[1].told = true;
        let later = fold(&mut kept, deploy("/c/site", 1_200, &["D"]));
        assert_ne!(later, first, "told already: a new note");
        let far = fold(&mut kept, deploy("/c/api", 1_130 + WINDOW, &["E"]));
        assert_ne!(far, other, "past the window: a new note");
        for i in 0..KEPT + 5 {
            let mut d = deploy("/c/x", 10_000 + i as u64 * WINDOW, &["Z"]);
            d.told = true;
            fold(&mut kept, d);
        }
        assert_eq!(kept.len(), KEPT, "only the newest are kept");
    }

    #[test]
    fn the_line_says_where_it_is_live_and_hides_internal_work() {
        let words = crate::i18n::core(crate::i18n::Lang::En);
        let mut d = deploy("/c/site", 1, &["New input box", "Left menu", "Claude mark", "Four", "Five", "Six", "Seven"]);
        d.url = Some("https://atinuminvest.vc/".into());
        d.internal = 3;
        assert_eq!(line(&d, words, None), "Live now on atinuminvest.vc: • New input box • Left menu • Claude mark • Four • Five +2 more · +3 internal changes");
        d.items.truncate(1);
        d.internal = 1;
        d.items[0].session = Some(4);
        assert_eq!(line(&d, words, Some(4)), "What you asked for is live on atinuminvest.vc: • New input box");
        d.source = DeploySource::Main;
        assert_eq!(line(&d, words, None), "On main in site (no deploy seen): • New input box · +1 internal change");
        d.items.clear();
        assert_eq!(line(&d, words, None), "On main in site (no deploy seen): only internal changes (1).");
        assert_eq!(sessions(&deploy("/c/site", 1, &[])), Vec::<SessionId>::new());
    }

    // ── looking at a project ──

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_CONFIG_GLOBAL", "/dev/null").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// A repository on main with one commit, its origin `remote` when given.
    fn repo(name: &str, remote: Option<&str>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-deploys-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git_in(&dir, &["init", "-q", "-b", "main"]);
        git_in(&dir, &["config", "user.email", "t@example.com"]);
        git_in(&dir, &["config", "user.name", "t"]);
        git_in(&dir, &["config", "commit.gpgsign", "false"]);
        commit(&dir, "README.md", "start");
        if let Some(remote) = remote {
            git_in(&dir, &["remote", "add", "origin", remote]);
        }
        dir
    }

    fn commit(dir: &Path, file: &str, subject: &str) -> String {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, subject).unwrap();
        git_in(dir, &["add", "-A"]);
        git_in(dir, &["commit", "-q", "-m", subject]);
        git_in(dir, &["rev-parse", "HEAD"])
    }

    /// GitHub as a few fixed answers, by the start of the path; what was asked, kept.
    struct Fake {
        answers: Vec<(String, Result<Value, String>)>,
        asked: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(answers: Vec<(&str, Result<Value, String>)>) -> Self {
            Fake { answers: answers.into_iter().map(|(p, a)| (p.to_owned(), a)).collect(), asked: RefCell::new(Vec::new()) }
        }
    }

    impl GitHub for Fake {
        fn get(&self, path: &str) -> Result<Value, String> {
            self.asked.borrow_mut().push(path.to_owned());
            self.answers.iter().find(|(p, _)| path.starts_with(p.as_str())).map(|(_, a)| a.clone()).unwrap_or_else(|| Err(format!("no answer for {path}")))
        }
    }

    #[test]
    fn a_project_without_github_is_live_once_its_commits_are_on_main() {
        let dir = repo("local", None);
        let gh = Fake::new(vec![]);
        let (watch, looked) = look(&dir, Watch::default(), &gh, 100);
        let start = match looked {
            Looked::Start(sha) => sha,
            other => panic!("first look starts watching: {other:?}"),
        };
        assert_eq!(look(&dir, watch.clone(), &gh, 110).1, Looked::Nothing, "nothing new");
        let sha = commit(&dir, "src/box.tsx", "equipe: merge T-4 New input box");
        let (watch, looked) = look(&dir, watch, &gh, 120);
        let Looked::Live { from, live, merges } = looked else { panic!("on main: live as far as equipe sees") };
        assert_eq!((from, live.sha.as_str(), live.source, live.at), (start, sha.as_str(), DeploySource::Main, 120));
        assert_eq!(merges.len(), 1);
        assert_eq!((merges[0].tickets.as_slice(), merges[0].files.as_slice()), (&[4][..], &["src/box.tsx".to_owned()][..]));
        assert_eq!(watch.live.as_deref(), Some(sha.as_str()));
        assert!(gh.asked.borrow().is_empty(), "no GitHub, nothing asked");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_vercel_project_is_live_once_a_production_deployment_succeeded() {
        let dir = repo("vercel", Some("https://github.com/acme/site.git"));
        let gh_start = Fake::new(vec![]);
        let (watch, _) = look(&dir, Watch::default(), &gh_start, 0);
        let a = commit(&dir, "src/a.tsx", "equipe: merge T-1 A");
        let b = commit(&dir, "src/b.tsx", "equipe: merge T-2 B");
        let now = crate::quality::live::unix_of("2026-11-18T00:10:00Z").unwrap();
        // b still building, a live: a's range is live, b waits.
        let gh = Fake::new(vec![
            ("repos/acme/site/deployments/20/", Ok(serde_json::json!([{"state": "in_progress"}]))),
            ("repos/acme/site/deployments/10/", Ok(serde_json::json!([{"state": "success", "created_at": "2026-11-18T00:05:00Z", "environment_url": "https://site-a.vercel.app"}]))),
            (
                "repos/acme/site/deployments",
                Ok(serde_json::json!([
                    {"id": 21, "sha": b, "environment": "Preview – site", "created_at": "2026-11-18T00:06:00Z"},
                    {"id": 20, "sha": b, "environment": "Production – site", "created_at": "2026-11-18T00:06:00Z"},
                    {"id": 10, "sha": a, "environment": "Production – site", "created_at": "2026-11-18T00:04:00Z"}
                ])),
            ),
            ("repos/acme/site", Ok(serde_json::json!({"homepage": "atinuminvest.vc"}))),
        ]);
        let (watch, looked) = look(&dir, watch, &gh, now);
        let Looked::Live { live, merges, .. } = looked else { panic!("a's deployment succeeded") };
        assert_eq!((live.sha.as_str(), live.source, live.environment.as_deref()), (a.as_str(), DeploySource::Deployment, Some("Production – site")));
        assert_eq!(live.url.as_deref(), Some("https://atinuminvest.vc"), "the site's own address over the deploy's");
        assert_eq!(merges.iter().flat_map(|m| m.tickets.clone()).collect::<Vec<_>>(), [1]);
        assert_eq!(watch.channel, Some(Channel::Deployments));
        // Asked again within the minute: GitHub is left alone.
        let asked = gh.asked.borrow().len();
        assert_eq!(look(&dir, watch.clone(), &gh, now + 30).1, Looked::Nothing);
        assert_eq!(gh.asked.borrow().len(), asked, "not asked again so soon");
        // Later, b still not live: nothing told.
        assert_eq!(look(&dir, watch, &gh, now + LOOK_EVERY).1, Looked::Nothing);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_project_with_a_deploy_workflow_is_live_once_its_run_succeeded() {
        let dir = repo("workflow", Some("git@github.com:acme/erp.git"));
        let (watch, _) = look(&dir, Watch::default(), &Fake::new(vec![]), 0);
        let a = commit(&dir, "src/a.rs", "equipe: merge T-7 Faster KPIs");
        let now = crate::quality::live::unix_of("2026-11-18T00:10:00Z").unwrap();
        let gh = Fake::new(vec![
            ("repos/acme/erp/deployments", Ok(serde_json::json!([]))),
            ("repos/acme/erp/actions/workflows/2/runs", Ok(serde_json::json!({"workflow_runs": [{"head_sha": a, "head_branch": "main", "conclusion": "success", "updated_at": "2026-11-18T00:08:00Z"}]}))),
            ("repos/acme/erp/actions/workflows", Ok(serde_json::json!({"workflows": [{"id": 2, "name": "Deploy", "path": ".github/workflows/deploy.yml", "state": "active"}]}))),
            ("repos/acme/erp", Ok(serde_json::json!({"homepage": null}))),
        ]);
        let (watch, looked) = look(&dir, watch, &gh, now);
        let Looked::Live { live, .. } = looked else { panic!("the deploy run succeeded") };
        assert_eq!((live.sha.as_str(), live.source), (a.as_str(), DeploySource::Workflow));
        assert_eq!(watch.channel, Some(Channel::Workflows { ids: vec![2] }));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn github_saying_nothing_of_deploys_or_failing_falls_back_to_main() {
        // A GitHub project with no deployments and no deploy workflow.
        let dir = repo("plain", Some("https://github.com/acme/lib"));
        let (watch, _) = look(&dir, Watch::default(), &Fake::new(vec![]), 0);
        let a = commit(&dir, "src/lib.rs", "equipe: merge T-3 Parser");
        let gh = Fake::new(vec![
            ("repos/acme/lib/deployments", Ok(serde_json::json!([]))),
            ("repos/acme/lib/actions/workflows", Ok(serde_json::json!({"workflows": [{"id": 1, "name": "CI", "path": ".github/workflows/ci.yml", "state": "active"}]}))),
            ("repos/acme/lib", Ok(serde_json::json!({}))),
        ]);
        let (kept, looked) = look(&dir, watch.clone(), &gh, 1_000);
        assert!(matches!(&looked, Looked::Live { live, .. } if live.sha == a && live.source == DeploySource::Main), "{looked:?}");
        assert_eq!(kept.channel, Some(Channel::Main));
        // gh missing or signed out: main as well, asked again sooner.
        let broken = Fake::new(vec![]);
        let (kept, looked) = look(&dir, watch, &broken, 1_000);
        assert!(matches!(looked, Looked::Live { live, .. } if live.source == DeploySource::Main));
        assert!(kept.channel_failed);
        commit(&dir, "src/b.rs", "equipe: merge T-5 More");
        let before = broken.asked.borrow().len();
        let (_, looked) = look(&dir, kept.clone(), &broken, 1_000 + CHANNEL_RETRY - 1);
        assert!(matches!(looked, Looked::Live { .. }));
        assert_eq!(broken.asked.borrow().len(), before, "not asked again before the retry is due");
        let mut stale = kept;
        stale.live = Some(a);
        let _ = look(&dir, stale, &broken, 1_000 + CHANNEL_RETRY);
        assert!(broken.asked.borrow().len() > before, "asked again once the retry is due");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rewritten_history_starts_watching_again_without_news() {
        let dir = repo("rewrite", None);
        let (watch, _) = look(&dir, Watch::default(), &Fake::new(vec![]), 0);
        commit(&dir, "a.txt", "one");
        git_in(&dir, &["reset", "-q", "--hard", "HEAD~1"]);
        commit(&dir, "b.txt", "two");
        let mut gone = watch.clone();
        gone.live = Some("0000000000000000000000000000000000000000".into());
        assert!(matches!(look(&dir, gone, &Fake::new(vec![]), 5).1, Looked::Start(_)));
        let _ = std::fs::remove_dir_all(dir);
    }
}
