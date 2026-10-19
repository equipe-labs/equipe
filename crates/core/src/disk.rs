//! The disk equipe writes to: how much room is left, what takes it, and what
//! equipe may give back (the shapes are [`equipe_protocol::disk`]).
//!
//! * [`probe`] — the free space of the tightest volume among `~/.equipe`, its
//!   worktrees and the squads' projects: one `statvfs` each, every minute or
//!   two, off the core loop. [`next_level`] turns it into a level, with room to
//!   spare before a level is left, so a disk on the edge doesn't flap.
//! * [`scan`] — what uses the space, among what equipe knows of: its members'
//!   worktrees (with their tickets' state and the build output in them), their
//!   build folders, equipe's own data, build output in the squads' project
//!   checkouts, and — to report only — the Mac's usual big caches. Bounded by a
//!   time budget; it never walks the whole disk.
//! * [`clean`] — gives back what a fresh look still says is safe, among the ids
//!   a screen asked for. Only under equipe's own folders, links never followed,
//!   a worktree only with nothing uncommitted (its branch stays).
//!
//! Each of them runs git or walks folders: never on the daemon's core loop
//! (`docs/INCIDENT-2026-10-16-daemon-hang.md`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub use equipe_protocol::disk::*;
use equipe_protocol::{TicketId, TicketStatus};

use crate::worktree::{GIT_TIMEOUT, git_command, output_within};

const GB: u64 = 1_000_000_000;
const MB: u64 = 1_000_000;

/// Under this much free, the disk is low — or under a tenth of it, if smaller.
pub const LOW_BELOW: u64 = 15 * GB;
pub const LOW_FRACTION: u64 = 10;
/// Under this much, builds fail soon: new build work waits.
pub const CRITICAL_BELOW: u64 = 5 * GB;
/// Under this much, the disk is full.
pub const FULL_BELOW: u64 = GB;
/// A level is left only past its threshold and this much more, in percent.
const LEAVE_MARGIN: u64 = 20;

/// Folder names that hold build output, rebuilt when wanted.
pub const BUILD_DIRS: [&str; 7] = ["target", "node_modules", ".next", "dist", "build", "DerivedData", ".turbo"];
/// How deep in a checkout build output is looked for (`packages/web/node_modules`).
const BUILD_DEPTH: usize = 3;
/// A member's build folder nobody built in for this long is stale.
const STALE_AFTER: Duration = Duration::from_secs(3 * 24 * 60 * 60);
/// A log past this size may be trimmed to its last `LOG_KEEP` bytes.
pub const LOG_PAST: u64 = 20 * MB;
const LOG_KEEP: u64 = MB;
/// The Mac's caches are listed from this size.
const CACHE_FROM: u64 = 500 * MB;
/// How long a look may take, all told.
pub const SCAN_BUDGET: Duration = Duration::from_secs(20);

/// How much debug information members' Rust builds carry
/// (`CARGO_PROFILE_DEV_DEBUG`, which the test profile follows). Debug
/// information is most of a debug target's size; line tables keep a
/// backtrace's files and lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BuildDebug {
    /// Line tables only: backtraces still say where. The default.
    #[default]
    Lines,
    /// None at all: the smallest builds.
    Off,
    /// What the project's own profile says.
    Full,
}

impl BuildDebug {
    pub const ALL: [BuildDebug; 3] = [BuildDebug::Lines, BuildDebug::Off, BuildDebug::Full];

    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.word() == word.trim())
    }

    /// As the person types it: `lines`, `off`, `full`.
    pub fn word(self) -> &'static str {
        match self {
            BuildDebug::Lines => "lines",
            BuildDebug::Off => "off",
            BuildDebug::Full => "full",
        }
    }

    /// What `CARGO_PROFILE_DEV_DEBUG` is set to; `None`: left alone.
    pub fn cargo_value(self) -> Option<&'static str> {
        match self {
            BuildDebug::Lines => Some("line-tables-only"),
            BuildDebug::Off => Some("none"),
            BuildDebug::Full => None,
        }
    }
}

/// The level for `free` bytes of `total`, entering a level at its threshold
/// times `percent`/100.
fn level_at(free: u64, total: u64, percent: u64) -> DiskLevel {
    let at = |threshold: u64| threshold / 100 * percent;
    let low = if total == 0 { LOW_BELOW } else { LOW_BELOW.min(total / LOW_FRACTION) };
    if free < at(FULL_BELOW) {
        DiskLevel::Full
    } else if free < at(CRITICAL_BELOW) {
        DiskLevel::Critical
    } else if free < at(low) {
        DiskLevel::Low
    } else {
        DiskLevel::Ok
    }
}

/// The level after `prev`, now that `free` bytes of `total` are free. Worse
/// is at once; better only once past the better level's threshold by a fifth
/// more, so a disk hovering at an edge says so once. A write that just failed
/// for want of room (`no_room`) is full, whatever the count says.
pub fn next_level(prev: DiskLevel, free: u64, total: u64, no_room: bool) -> DiskLevel {
    if no_room {
        return DiskLevel::Full;
    }
    let now = level_at(free, total, 100);
    if now >= prev {
        return now;
    }
    level_at(free, total, 100 + LEAVE_MARGIN).min(prev).max(now)
}

/// The folders to watch: equipe's home (its worktrees and build folders are
/// under it) and every squad's projects.
pub fn watched(home: &Path, projects: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = vec![home.to_owned()];
    paths.extend(projects.iter().cloned());
    paths
}

/// What `statvfs` says of the volume `path` is on: (its id, free, total).
fn volume(path: &Path) -> Option<(u64, u64, u64)> {
    use std::os::unix::ffi::OsStrExt as _;
    // A folder not made yet: the volume it will be on.
    let path = path.ancestors().find(|p| p.exists())?;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs writes only the struct it is given, and `c` is a valid C string.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    use std::os::unix::fs::MetadataExt as _;
    let dev = std::fs::metadata(path).ok()?.dev();
    #[allow(clippy::unnecessary_cast)]
    let (free, total) = (stat.f_bavail as u64 * stat.f_frsize as u64, stat.f_blocks as u64 * stat.f_frsize as u64);
    Some((dev, free, total))
}

/// The free space of the tightest volume among `paths` (its level left
/// `Ok` for [`next_level`] to set). `None` when none could be looked at.
pub fn probe(paths: &[PathBuf]) -> Option<DiskStatus> {
    off_the_loop("disk::probe");
    let mut seen = Vec::new();
    let mut tightest: Option<DiskStatus> = None;
    for path in paths {
        let Some((dev, free, total)) = volume(path) else { continue };
        if seen.contains(&dev) {
            continue;
        }
        seen.push(dev);
        if tightest.as_ref().is_none_or(|t| free < t.free) {
            tightest = Some(DiskStatus { level: DiskLevel::Ok, free, total, path: path.clone(), container_free: None });
        }
    }
    tightest
}

/// The APFS container's free space for the volume `path` is on, as
/// `diskutil` says (macOS; `None` elsewhere or when it doesn't answer).
pub fn container_free(path: &Path) -> Option<u64> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let mut command = std::process::Command::new("/usr/sbin/diskutil");
    command.args(["info", "-plist"]).arg(path);
    let out = output_within(command, Duration::from_secs(10)).ok()?;
    plist_integer(&String::from_utf8_lossy(&out.stdout), "APFSContainerFree")
}

/// `<key>name</key><integer>n</integer>` in a plist.
fn plist_integer(plist: &str, name: &str) -> Option<u64> {
    let after = &plist[plist.find(&format!("<key>{name}</key>"))?..];
    let start = after.find("<integer>")? + "<integer>".len();
    let end = after[start..].find("</integer>")?;
    after[start..start + end].trim().parse().ok()
}

fn off_the_loop(what: &str) {
    debug_assert!(!crate::runtime::on_core_loop(), "{what} walks folders and must not run on the core loop: spawn_blocking it");
}

// ── what equipe knows ──

/// A ticket, as far as its worktree goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownTicket {
    pub squad: String,
    pub id: TicketId,
    pub status: TicketStatus,
    pub merged: Option<bool>,
    /// The worktree it keeps, while it keeps one.
    pub worktree: Option<PathBuf>,
    pub project: Option<PathBuf>,
}

/// What a look at the disk starts from: gathered by the daemon on its loop
/// (cheap: what it holds) and walked off it.
#[derive(Debug, Clone, Default)]
pub struct Known {
    /// equipe's home: everything it may remove is under it.
    pub home: PathBuf,
    pub tickets: Vec<KnownTicket>,
    /// Worktrees members work in now.
    pub in_use: Vec<PathBuf>,
    /// Members' build folders a turn under way may be building in.
    pub building: Vec<PathBuf>,
    /// Each squad's projects: (squad, checkout).
    pub projects: Vec<(String, PathBuf)>,
    /// The person's home folder, for the Mac's caches; `None`: not looked at.
    pub user_home: Option<PathBuf>,
}

/// What equipe knows, read from the squads' journals — for `equipe disk`
/// with no daemon running (nothing is in use then).
pub fn known_from_journals(home: &Path) -> Known {
    let mut known = Known { home: home.to_owned(), user_home: std::env::var_os("HOME").map(PathBuf::from), ..Known::default() };
    for squad in crate::squads::list(home) {
        let Ok(journal) = crate::journal::Journal::for_squad(home, &squad.name) else { continue };
        for t in journal.tickets().unwrap_or_default() {
            let worktree = t.worktree.clone().or_else(|| journal.ticket_work(t.id).ok().and_then(|w| w.worktree));
            known.tickets.push(KnownTicket { squad: squad.name.clone(), id: t.id, status: t.status, merged: t.merged, worktree, project: t.project.clone() });
        }
        known.projects.extend(squad.projects.into_iter().map(|p| (squad.name.clone(), p)));
    }
    known
}

// ── measuring ──

/// A deadline every walk shares.
struct Budget {
    until: Instant,
    ran_out: bool,
}

impl Budget {
    fn new(budget: Duration) -> Self {
        Self { until: Instant::now() + budget, ran_out: false }
    }

    fn left(&mut self) -> bool {
        if Instant::now() >= self.until {
            self.ran_out = true;
        }
        !self.ran_out
    }
}

/// The bytes under `path` on disk, links not followed; `false` when the
/// budget ran out before the end.
fn measure(path: &Path, budget: &mut Budget) -> (u64, bool) {
    use std::os::unix::fs::MetadataExt as _;
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return (0, true);
    };
    let mut total = meta.blocks() * 512;
    if !meta.is_dir() {
        return (total, true);
    }
    let mut stack = vec![path.to_owned()];
    let mut seen = 0u32;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen.is_multiple_of(256) && !budget.left() {
                return (total, false);
            }
            let Ok(meta) = entry.metadata() else { continue };
            total += meta.blocks() * 512;
            if meta.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    (total, budget.left() || stack.is_empty())
}

/// `path` with its folders' links resolved, `path` itself as it is: what a
/// removal would act on.
fn real(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?.canonicalize().ok()?;
    Some(parent.join(path.file_name()?))
}

/// Whether removing `path` stays under one of `roots`: its folders resolved,
/// and it neither a link nor a root itself.
pub fn removable(path: &Path, roots: &[PathBuf]) -> bool {
    let Some(at) = real(path) else { return false };
    let Ok(meta) = std::fs::symlink_metadata(&at) else { return false };
    if meta.file_type().is_symlink() {
        return false;
    }
    roots.iter().filter_map(|r| r.canonicalize().ok()).any(|root| at != root && at.starts_with(&root))
}

/// equipe's folders a clean may act under.
fn roots(home: &Path) -> Vec<PathBuf> {
    vec![home.join("worktrees"), home.join("cache").join("build"), home.to_owned()]
}

fn git_in(dir: &Path, args: &[&str]) -> Option<String> {
    let mut command = git_command(dir);
    command.args(args);
    let out = output_within(command, GIT_TIMEOUT).ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether a worktree has changes not committed (or git can't say).
fn uncommitted(worktree: &Path) -> bool {
    git_in(worktree, &["status", "--porcelain"]).is_none_or(|s| !s.trim().is_empty())
}

/// Build output folders in `checkout`, up to a few levels down — not inside
/// `.git`, another build folder, or a link — that git ignores there.
fn build_dirs(checkout: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut level = vec![checkout.to_owned()];
    for _ in 0..BUILD_DEPTH {
        let mut next = Vec::new();
        for dir in level {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else { continue };
                if !kind.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if BUILD_DIRS.contains(&name.as_str()) {
                    found.push(entry.path());
                } else if !name.starts_with('.') {
                    next.push(entry.path());
                }
            }
        }
        level = next;
    }
    if found.is_empty() {
        return found;
    }
    // Only what git ignores is build output; a `build/` of sources stays.
    let relative: Vec<String> = found.iter().filter_map(|p| p.strip_prefix(checkout).ok()).map(|p| format!("{}/", p.to_string_lossy())).collect();
    let mut command = git_command(checkout);
    command.arg("check-ignore").args(&relative);
    let ignored: Vec<String> = output_within(command, GIT_TIMEOUT)
        .map(|out| String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim_end_matches('/').to_owned()).collect())
        .unwrap_or_default();
    found.into_iter().filter(|p| p.strip_prefix(checkout).is_ok_and(|r| ignored.iter().any(|i| Path::new(i) == r))).collect()
}

/// `t24-6ad1cb4a` → 24.
fn ticket_of_folder(name: &str) -> Option<TicketId> {
    let rest = name.strip_prefix('t')?;
    rest.split('-').next()?.parse().ok()
}

fn id(kind: DiskKind, path: &Path) -> String {
    let kind = serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
    format!("{kind}:{}", path.display())
}

fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-~".contains(c)) { s.into_owned() } else { format!("'{}'", s.replace('\'', "'\\''")) }
}

struct Scan<'a> {
    known: &'a Known,
    budget: Budget,
    items: Vec<DiskItem>,
}

impl Scan<'_> {
    #[allow(clippy::too_many_arguments)]
    fn push(&mut self, group: DiskGroup, kind: DiskKind, label: String, path: &Path, bytes: u64, whole: bool, safe: bool, why: DiskWhy) -> usize {
        self.items.push(DiskItem {
            id: id(kind, path),
            group,
            kind,
            label,
            path: path.to_owned(),
            bytes,
            partial: !whole,
            safe,
            why,
            squad: None,
            ticket: None,
            command: None,
        });
        self.items.len() - 1
    }

    /// Members' worktrees: each with its ticket's state, and the build
    /// output in it apart.
    fn worktrees(&mut self) {
        let root = self.known.home.join("worktrees");
        let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
        let in_use: Vec<PathBuf> = self.known.in_use.iter().map(|p| canon(p)).collect();
        let Ok(repos) = std::fs::read_dir(&root) else { return };
        let mut dirs: Vec<PathBuf> = repos.flatten().flat_map(|r| std::fs::read_dir(r.path()).into_iter().flatten().flatten()).map(|d| d.path()).collect();
        dirs.sort();
        for dir in dirs {
            // Only a worktree, and never through a link.
            let is_link = std::fs::symlink_metadata(&dir).map(|m| m.file_type().is_symlink()).unwrap_or(true);
            if is_link || !dir.join(".git").is_file() || !self.budget.left() {
                continue;
            }
            let path = canon(&dir);
            let folder = dir.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let number = dir.file_name().and_then(|n| ticket_of_folder(&n.to_string_lossy()));
            let keeper = self.known.tickets.iter().find(|t| t.worktree.as_deref().is_some_and(|w| canon(w) == path));
            // A worktree no ticket keeps: the closed ticket of its number, on its repository.
            let closed = keeper.is_none().then(|| {
                self.known.tickets.iter().find(|t| {
                    Some(t.id) == number && t.status.is_closed() && {
                        let project = t.project.as_deref().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
                        project.as_deref() == Some(folder.as_str()) || folder == format!("{}.home", t.squad)
                    }
                })
            });
            let ticket = keeper.or(closed.flatten());
            let why = if in_use.iter().any(|u| u.starts_with(&path) || path.starts_with(u)) {
                DiskWhy::InUse
            } else if keeper.is_some_and(|t| !t.status.is_closed()) {
                DiskWhy::OpenTicket
            } else {
                match ticket {
                    Some(t) if t.status == TicketStatus::Cancelled => DiskWhy::Cancelled,
                    Some(t) if t.merged == Some(true) => DiskWhy::Merged,
                    Some(_) => DiskWhy::NotMerged,
                    None => DiskWhy::NoTicket,
                }
            };
            let finished = matches!(why, DiskWhy::Merged | DiskWhy::Cancelled | DiskWhy::NoTicket);
            // Asked of git only when it could matter.
            let why = if finished && uncommitted(&path) { DiskWhy::Uncommitted } else { why };
            let safe = matches!(why, DiskWhy::Merged | DiskWhy::Cancelled | DiskWhy::NoTicket);
            let name = number.map_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), |n| format!("T-{n}"));
            let squad = ticket.map(|t| t.squad.clone());
            let mut builds_bytes = 0;
            for build in build_dirs(&path) {
                let (bytes, whole) = measure(&build, &mut self.budget);
                builds_bytes += bytes;
                if bytes < MB {
                    continue;
                }
                let inside = build.strip_prefix(&path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                let at = self.push(DiskGroup::Worktrees, DiskKind::WorktreeBuild, format!("{name} · {folder} · {inside}/"), &build, bytes, whole, safe, why);
                self.items[at].squad = squad.clone();
                self.items[at].ticket = number;
            }
            let (bytes, whole) = measure(&path, &mut self.budget);
            let at = self.push(DiskGroup::Worktrees, DiskKind::Worktree, format!("{name} · {folder}"), &path, bytes.saturating_sub(builds_bytes), whole, safe, why);
            self.items[at].squad = squad;
            self.items[at].ticket = number;
        }
    }

    /// Members' build folders (`CARGO_TARGET_DIR`).
    fn builds(&mut self) {
        let root = self.known.home.join("cache").join("build");
        let Ok(squads) = std::fs::read_dir(&root) else { return };
        let building: Vec<PathBuf> = self.known.building.iter().map(|p| p.canonicalize().unwrap_or_else(|_| p.clone())).collect();
        for squad in squads.flatten() {
            let squad_name = squad.file_name().to_string_lossy().into_owned();
            for member in std::fs::read_dir(squad.path()).into_iter().flatten().flatten() {
                let path = member.path();
                let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
                if !meta.is_dir() || !self.budget.left() {
                    continue;
                }
                let (bytes, whole) = measure(&path, &mut self.budget);
                if bytes < MB {
                    continue;
                }
                let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
                let stale = newest(&path).is_some_and(|at| at.elapsed().unwrap_or_default() > STALE_AFTER);
                let why = if building.contains(&canon) {
                    DiskWhy::InUse
                } else if stale {
                    DiskWhy::Stale
                } else {
                    DiskWhy::Rebuildable
                };
                let label = format!("{} · {squad_name}", member.file_name().to_string_lossy());
                // A working member's folder stays, but the incremental caches in
                // it may go between its builds: listed apart, not counted twice.
                let mut rest = bytes;
                if why == DiskWhy::InUse {
                    for cache in incremental_dirs(&path) {
                        let (size, whole) = measure(&cache, &mut self.budget);
                        if size < INCREMENTAL_FROM || !idle(cache.parent().unwrap_or(&cache)) {
                            continue;
                        }
                        rest = rest.saturating_sub(size);
                        let inside = cache.strip_prefix(&path).unwrap_or(&cache).display().to_string();
                        let at = self.push(DiskGroup::Builds, DiskKind::BuildCache, format!("{label} · {inside}/"), &cache, size, whole, true, DiskWhy::Rebuildable);
                        self.items[at].squad = Some(squad_name.clone());
                    }
                }
                let at = self.push(DiskGroup::Builds, DiskKind::BuildCache, label, &path, rest, whole, why != DiskWhy::InUse, why);
                self.items[at].squad = Some(squad_name.clone());
            }
        }
    }

    /// Build output in the squads' checkouts, and in other tools' worktrees
    /// there (`.claude/worktrees/*`): the person's, so only reported.
    fn projects(&mut self) {
        let mut seen: Vec<PathBuf> = Vec::new();
        for (squad, project) in &self.known.projects {
            let Ok(project) = project.canonicalize() else { continue };
            if seen.contains(&project) {
                continue;
            }
            seen.push(project.clone());
            let mut checkouts = vec![project.clone()];
            for tools in [".claude/worktrees", ".codex/worktrees", ".worktrees"] {
                let mut more: Vec<PathBuf> = std::fs::read_dir(project.join(tools)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.join(".git").exists()).collect();
                more.sort();
                checkouts.extend(more);
            }
            let name = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            for checkout in checkouts {
                if !self.budget.left() {
                    return;
                }
                for build in build_dirs(&checkout) {
                    let (bytes, whole) = measure(&build, &mut self.budget);
                    if bytes < 50 * MB {
                        continue;
                    }
                    let inside = build.strip_prefix(&project).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                    let at = self.push(DiskGroup::Projects, DiskKind::ProjectBuild, format!("{name} · {inside}/"), &build, bytes, whole, false, DiskWhy::NotEquipes);
                    self.items[at].squad = Some(squad.clone());
                    self.items[at].command = Some(format!("rm -rf {}", shell_quote(&build)));
                }
            }
        }
    }

    /// equipe's own: logs, the members' browser, journals, uploads.
    fn equipe(&mut self) {
        let home = self.known.home.clone();
        let mut logs: Vec<PathBuf> = Vec::new();
        for dir in [home.clone(), home.join("browser"), home.join("desktop")] {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                let is_log = path.extension().is_some_and(|e| e == "log") || path.file_name().is_some_and(|n| n.to_string_lossy().contains(".log."));
                if is_log && entry.file_type().is_ok_and(|t| t.is_file()) {
                    logs.push(path);
                }
            }
        }
        logs.sort();
        for log in logs {
            let bytes = std::fs::symlink_metadata(&log).map(|m| m.len()).unwrap_or(0);
            if bytes < MB {
                continue;
            }
            let label = log.strip_prefix(&home).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let old = bytes > LOG_PAST;
            self.push(DiskGroup::Equipe, DiskKind::Log, label, &log, if old { bytes - LOG_KEEP } else { bytes }, true, old, if old { DiskWhy::OldLog } else { DiskWhy::Needed });
        }
        let journals: Vec<PathBuf> = std::fs::read_dir(home.join("squads")).into_iter().flatten().flatten().map(|d| d.path()).collect();
        let mut journal_bytes = 0;
        for squad in &journals {
            for entry in std::fs::read_dir(squad).into_iter().flatten().flatten() {
                if entry.file_name().to_string_lossy().ends_with(".db") || entry.file_name().to_string_lossy().contains(".db-") {
                    journal_bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
        if journal_bytes >= MB {
            self.push(DiskGroup::Equipe, DiskKind::Journals, "squads/*/squad.db".into(), &home.join("squads"), journal_bytes, true, false, DiskWhy::Needed);
        }
        for (kind, folder) in [(DiskKind::Browser, "browser"), (DiskKind::Uploads, "uploads")] {
            let path = home.join(folder);
            if !path.is_dir() || !self.budget.left() {
                continue;
            }
            let (bytes, whole) = measure(&path, &mut self.budget);
            if bytes >= MB {
                self.push(DiskGroup::Equipe, kind, format!("{folder}/"), &path, bytes, whole, false, DiskWhy::Needed);
            }
        }
    }

    /// The Mac's usual big caches, to report: the person clears them, with
    /// the command shown.
    fn caches(&mut self) {
        let Some(user) = self.known.user_home.clone() else { return };
        let library = user.join("Library");
        let listed: [(DiskKind, PathBuf, Option<&str>); 9] = [
            (DiskKind::DerivedData, library.join("Developer/Xcode/DerivedData"), Some("rm -rf ~/Library/Developer/Xcode/DerivedData")),
            (DiskKind::Simulators, library.join("Developer/CoreSimulator/Devices"), Some("xcrun simctl delete unavailable")),
            (DiskKind::Cargo, user.join(".cargo/registry"), Some("rm -rf ~/.cargo/registry/cache ~/.cargo/registry/src")),
            (DiskKind::Cargo, user.join(".cargo/git"), Some("rm -rf ~/.cargo/git/checkouts")),
            (DiskKind::Npm, user.join(".npm/_cacache"), Some("npm cache clean --force")),
            (DiskKind::Pnpm, library.join("pnpm/store"), Some("pnpm store prune")),
            (DiskKind::Pnpm, user.join(".local/share/pnpm/store"), Some("pnpm store prune")),
            (DiskKind::Yarn, library.join("Caches/Yarn"), Some("yarn cache clean")),
            (DiskKind::Docker, library.join("Containers/com.docker.docker/Data"), Some("docker system prune")),
        ];
        for (kind, path, command) in listed {
            if !path.is_dir() || !self.budget.left() {
                continue;
            }
            let (bytes, whole) = measure(&path, &mut self.budget);
            if bytes < CACHE_FROM {
                continue;
            }
            let label = path.strip_prefix(&user).map(|p| format!("~/{}", p.display())).unwrap_or_default();
            let at = self.push(DiskGroup::Caches, kind, label, &path, bytes, whole, false, DiskWhy::NotEquipes);
            self.items[at].command = command.map(str::to_owned);
        }
        // The biggest of the rest of `~/Library/Caches`: shown in Finder.
        let caches = library.join("Caches");
        let mut apps: Vec<(u64, bool, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&caches).into_iter().flatten().flatten() {
            if entry.file_name() == "Yarn" || !entry.file_type().is_ok_and(|t| t.is_dir()) || !self.budget.left() {
                continue;
            }
            let (bytes, whole) = measure(&entry.path(), &mut self.budget);
            apps.push((bytes, whole, entry.path()));
        }
        apps.sort_by_key(|(bytes, ..)| std::cmp::Reverse(*bytes));
        for (bytes, whole, path) in apps.into_iter().take(5).filter(|(bytes, ..)| *bytes >= CACHE_FROM) {
            let label = format!("~/Library/Caches/{}", path.file_name().unwrap_or_default().to_string_lossy());
            self.push(DiskGroup::Caches, DiskKind::LibraryCache, label, &path, bytes, whole, false, DiskWhy::NotEquipes);
        }
    }
}

/// When anything directly in `dir`, or `dir` itself, last changed.
fn newest(dir: &Path) -> Option<std::time::SystemTime> {
    let own = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
    std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| e.metadata().and_then(|m| m.modified()).ok()).chain(own).max()
}

/// What uses the space among what equipe knows of, biggest first within the
/// order the panel shows: what equipe may give back, then what it keeps, then
/// what only the person can clear. Within `budget`; `partial` when it ran out.
pub fn scan(known: &Known, budget: Duration) -> (Vec<DiskItem>, bool) {
    off_the_loop("disk::scan");
    let mut scan = Scan { known, budget: Budget::new(budget), items: Vec::new() };
    scan.worktrees();
    scan.builds();
    scan.equipe();
    scan.projects();
    scan.caches();
    let ran_out = scan.budget.ran_out;
    let mut items = scan.items;
    items.retain(|i| i.bytes > 0 || i.kind == DiskKind::Worktree);
    items.sort_by(|a, b| b.safe.cmp(&a.safe).then(b.bytes.cmp(&a.bytes)));
    (items, ran_out)
}

/// A full report: the free space, with the container's, and what uses it.
pub fn report(known: &Known, mut status: DiskStatus, budget: Duration) -> DiskReport {
    let started = Instant::now();
    status.container_free = container_free(&status.path);
    let (items, partial) = scan(known, budget);
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    DiskReport { status, items, took_ms: started.elapsed().as_millis() as u64, partial, at }
}

// ── giving it back ──

/// Give back the items among `ids` that a fresh look still says equipe may
/// remove. Never anything outside equipe's own folders, never through a link,
/// a worktree only with nothing uncommitted, its branch kept. The status is
/// left for the caller to fill.
pub fn clean(known: &Known, ids: &[String]) -> DiskCleaned {
    off_the_loop("disk::clean");
    // Only equipe's own: what the person's caches hold is never looked at here.
    let fresh = Known { user_home: None, projects: Vec::new(), ..known.clone() };
    let (items, _) = scan(&fresh, Duration::from_secs(120));
    let roots = roots(&known.home);
    let mut done = DiskCleaned::default();
    // Build output before the worktree it is in, so each is counted once.
    let mut asked: Vec<&DiskItem> = ids.iter().filter_map(|id| items.iter().find(|i| &i.id == id)).collect();
    asked.sort_by_key(|i| i.kind == DiskKind::Worktree);
    for item in asked {
        let id = &item.id;
        if !item.safe {
            done.skipped.push((id.clone(), item.why));
            continue;
        }
        let gone = match item.kind {
            // Never under a build: cargo is kept out while it goes, or it stays.
            DiskKind::BuildCache if removable(&item.path, &roots) => match unbuilt(&item.path, || std::fs::remove_dir_all(&item.path).is_ok()) {
                Some(gone) => gone,
                None => {
                    done.skipped.push((id.clone(), DiskWhy::InUse));
                    continue;
                }
            },
            DiskKind::WorktreeBuild if removable(&item.path, &roots) => std::fs::remove_dir_all(&item.path).is_ok(),
            DiskKind::Worktree if removable(&item.path, &roots) => {
                if uncommitted(&item.path) {
                    done.skipped.push((id.clone(), DiskWhy::Uncommitted));
                    continue;
                }
                remove_worktree(&item.path)
            }
            DiskKind::Log if removable(&item.path, &roots) => trim_log(&item.path).is_ok(),
            _ => false,
        };
        if gone {
            // A worktree's own bytes: its build output was counted apart.
            let builds: u64 = if item.kind == DiskKind::Worktree {
                items.iter().filter(|i| i.kind == DiskKind::WorktreeBuild && i.path.starts_with(&item.path) && !ids.contains(&i.id)).map(|i| i.bytes).sum()
            } else {
                0
            };
            done.freed += item.bytes + builds;
            done.removed.push(id.clone());
        }
    }
    done
}

/// From this size an incremental cache is worth listing apart.
const INCREMENTAL_FROM: u64 = if cfg!(test) { MB } else { 100 * MB };

/// The incremental caches in a build folder: `debug/incremental`, and per
/// target (`aarch64-apple-darwin/debug/incremental`).
fn incremental_dirs(build: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut look = |dir: &Path| {
        for profile in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let cache = profile.path().join("incremental");
            if std::fs::symlink_metadata(&cache).is_ok_and(|m| m.is_dir()) {
                found.push(cache);
            }
        }
    };
    look(build);
    for target in std::fs::read_dir(build).into_iter().flatten().flatten() {
        if target.file_name().to_string_lossy().contains('-') && target.path().is_dir() {
            look(&target.path());
        }
    }
    found
}

/// Cargo's locks in a build folder or one of its profiles (`debug/.cargo-lock`,
/// `.cargo-build-lock`, …): held while it builds there.
fn cargo_locks(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, locks: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            if meta.is_file() && name.starts_with(".cargo") && name.ends_with("lock") {
                locks.push(entry.path());
            } else if meta.is_dir() && depth > 0 && !matches!(name.as_str(), "deps" | "incremental" | "build" | ".fingerprint" | "examples") {
                walk(&entry.path(), depth - 1, locks);
            }
        }
    }
    let mut locks = Vec::new();
    walk(dir, 2, &mut locks);
    locks
}

/// Whether cargo is building nowhere in `dir` right now.
fn idle(dir: &Path) -> bool {
    unbuilt(dir, || ()).is_some()
}

/// `f`, with cargo kept out of the build folder `path` is (or is in) while it
/// runs: each of its locks taken, none waited for. `None` when cargo builds
/// there now. A cache inside a profile (`debug/incremental`) takes that
/// profile's locks.
fn unbuilt<T>(path: &Path, f: impl FnOnce() -> T) -> Option<T> {
    let dir = if path.file_name().is_some_and(|n| n == "incremental") { path.parent().unwrap_or(path) } else { path };
    let mut held = Vec::new();
    for lock in cargo_locks(dir) {
        let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&lock) else { continue };
        if file.try_lock().is_err() {
            return None;
        }
        held.push(file);
    }
    let out = f();
    drop(held);
    Some(out)
}

/// When the disk runs short: the caches no build needs between builds go,
/// without asking — incremental caches in members' build folders, and at
/// `deeper` (nearly full) whole build folders nobody built in for days. Only
/// where cargo isn't building, kept out while they go. What went, for the log.
pub fn trim_builds(home: &Path, deeper: bool) -> (u64, Vec<String>) {
    off_the_loop("disk::trim_builds");
    let roots = roots(home);
    let mut freed = 0;
    let mut done = Vec::new();
    let mut give = |path: &Path, done: &mut Vec<String>| {
        let mut budget = Budget::new(Duration::from_secs(10));
        let (bytes, _) = measure(path, &mut budget);
        if removable(path, &roots) && unbuilt(path, || std::fs::remove_dir_all(path).is_ok()) == Some(true) {
            freed += bytes;
            done.push(format!("{} ({})", path.display(), size(bytes)));
        }
    };
    for squad in std::fs::read_dir(home.join("cache").join("build")).into_iter().flatten().flatten() {
        for member in std::fs::read_dir(squad.path()).into_iter().flatten().flatten() {
            let path = member.path();
            if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            if deeper && newest(&path).is_some_and(|at| at.elapsed().unwrap_or_default() > STALE_AFTER) {
                give(&path, &mut done);
                continue;
            }
            for cache in incremental_dirs(&path) {
                give(&cache, &mut done);
            }
        }
    }
    (freed, done)
}

/// `git worktree remove` from its repository: the branch stays.
fn remove_worktree(path: &Path) -> bool {
    let Some(common) = git_in(path, &["rev-parse", "--path-format=absolute", "--git-common-dir"]) else {
        return false;
    };
    let repo = Path::new(common.trim()).parent().map(Path::to_owned).unwrap_or_default();
    git_in(&repo, &["worktree", "remove", "--force", &path.to_string_lossy()]).is_some()
}

/// Keep a log's last part: what is written next still goes at its end.
fn trim_log(path: &Path) -> std::io::Result<()> {
    use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
    let mut file = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
    let len = file.metadata()?.len();
    let mut tail = Vec::new();
    file.seek(SeekFrom::Start(len.saturating_sub(LOG_KEEP)))?;
    file.read_to_end(&mut tail)?;
    // From the start of a line.
    let from = tail.iter().position(|b| *b == b'\n').map_or(0, |n| n + 1);
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&tail[from..])
}

/// Build output in a worktree whose ticket is done with — merged or
/// cancelled — given back at once; the worktree and branch stay. Returns what
/// went, one line each, for the log.
pub fn clear_builds(home: &Path, worktree: &Path) -> Vec<String> {
    off_the_loop("disk::clear_builds");
    let roots = [home.join("worktrees")];
    let mut done = Vec::new();
    if !worktree.join(".git").is_file() {
        return done;
    }
    for build in build_dirs(worktree) {
        if removable(&build, &roots) && std::fs::remove_dir_all(&build).is_ok() {
            done.push(format!("cleared {}", build.display()));
        }
    }
    done
}

/// Whether a simulator named `name` is ticket `id`'s: `T126 iPhone 17 Pro`,
/// `T-126 …`, `acme T126 …` (`squad` given) — not `T1260 …`.
fn ticket_simulator(name: &str, id: TicketId, squad: Option<&str>) -> bool {
    let name = squad.and_then(|s| name.strip_prefix(s)).map_or(name, str::trim_start);
    [format!("T{id}"), format!("T-{id}")].iter().any(|tag| name == tag || name.strip_prefix(tag.as_str()).is_some_and(|rest| rest.starts_with([' ', '-', '_', '·'])))
}

/// The simulators made for ticket `id` (named for it, as the mobile role is
/// told to), shut down and deleted once the ticket is done with: each holds a
/// booted iPhone's memory and gigabytes of disk. `squad`'s own names always;
/// a bare `T<id>` only when `shared` is false (no other squad has that
/// number open). What went, for the log.
pub fn clear_simulators(id: TicketId, squad: &str, shared: bool) -> Vec<String> {
    off_the_loop("disk::clear_simulators");
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let simctl = |args: &[&str]| {
        let mut command = std::process::Command::new("xcrun");
        command.arg("simctl").args(args);
        output_within(command, Duration::from_secs(60)).ok().filter(|o| o.status.success())
    };
    let Some(list) = simctl(&["list", "devices", "-j"]) else { return Vec::new() };
    let Ok(list) = serde_json::from_slice::<serde_json::Value>(&list.stdout) else { return Vec::new() };
    let mut done = Vec::new();
    for device in list["devices"].as_object().into_iter().flat_map(|runtimes| runtimes.values()).filter_map(|d| d.as_array()).flatten() {
        let (Some(name), Some(udid)) = (device["name"].as_str(), device["udid"].as_str()) else { continue };
        let ours = ticket_simulator(name, id, Some(squad)) && (name.starts_with(squad) || !shared);
        if !ours {
            continue;
        }
        if device["state"].as_str() == Some("Booted") {
            simctl(&["shutdown", udid]);
        }
        if simctl(&["delete", udid]).is_some() {
            done.push(format!("deleted simulator {name} ({udid})"));
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_simulator_is_a_tickets_by_its_name() {
        assert!(ticket_simulator("T126 iPhone 17 Pro", 126, None));
        assert!(ticket_simulator("T-126 Media", 126, None));
        assert!(ticket_simulator("T126", 126, None));
        assert!(ticket_simulator("acme T126 iPhone", 126, Some("acme")));
        assert!(!ticket_simulator("T1260 iPhone", 126, None));
        assert!(!ticket_simulator("iPhone 17 Pro", 126, None));
        assert!(!ticket_simulator("Test T126", 126, None));
    }

    const G: u64 = GB;

    #[test]
    fn levels_enter_at_their_threshold() {
        let total = 500 * G;
        assert_eq!(next_level(DiskLevel::Ok, 40 * G, total, false), DiskLevel::Ok);
        assert_eq!(next_level(DiskLevel::Ok, 14 * G, total, false), DiskLevel::Low);
        assert_eq!(next_level(DiskLevel::Ok, 4 * G, total, false), DiskLevel::Critical);
        assert_eq!(next_level(DiskLevel::Ok, G / 2, total, false), DiskLevel::Full);
        // A small disk is low at a tenth of it.
        assert_eq!(next_level(DiskLevel::Ok, 9 * G, 100 * G, false), DiskLevel::Low);
        assert_eq!(next_level(DiskLevel::Ok, 11 * G, 100 * G, false), DiskLevel::Ok);
    }

    #[test]
    fn a_level_is_left_only_with_room_to_spare() {
        let total = 500 * G;
        // Just past 5 GB: still critical; past 6 GB, low.
        assert_eq!(next_level(DiskLevel::Critical, 5 * G + G / 2, total, false), DiskLevel::Critical);
        assert_eq!(next_level(DiskLevel::Critical, 6 * G + G / 2, total, false), DiskLevel::Low);
        // Low until 18 GB.
        assert_eq!(next_level(DiskLevel::Low, 16 * G, total, false), DiskLevel::Low);
        assert_eq!(next_level(DiskLevel::Low, 19 * G, total, false), DiskLevel::Ok);
        // A big clean goes all the way at once.
        assert_eq!(next_level(DiskLevel::Full, 40 * G, total, false), DiskLevel::Ok);
        // Full holds until 1.2 GB.
        assert_eq!(next_level(DiskLevel::Full, G + G / 10, total, false), DiskLevel::Full);
        assert_eq!(next_level(DiskLevel::Full, 2 * G, total, false), DiskLevel::Critical);
        // Worse is at once, whatever the margin.
        assert_eq!(next_level(DiskLevel::Low, 4 * G, total, false), DiskLevel::Critical);
    }

    #[test]
    fn a_write_with_no_room_is_full_whatever_the_count() {
        assert_eq!(next_level(DiskLevel::Ok, 40 * G, 500 * G, true), DiskLevel::Full);
    }

    #[test]
    fn hovering_at_an_edge_changes_the_level_once() {
        let total = 500 * G;
        let mut level = DiskLevel::Ok;
        let mut changes = 0;
        for free in [15 * G + 1, 15 * G - 1, 15 * G + 1, 15 * G - 1, 16 * G, 14 * G, 17 * G] {
            let next = next_level(level, free, total, false);
            changes += usize::from(next != level);
            level = next;
        }
        assert_eq!((level, changes), (DiskLevel::Low, 1));
    }

    #[test]
    fn the_plist_says_the_containers_free_space() {
        let plist = "<dict>\n\t<key>APFSContainerFree</key>\n\t<integer>12861775872</integer>\n\t<key>APFSContainerSize</key>\n\t<integer>494384795648</integer>";
        assert_eq!(plist_integer(plist, "APFSContainerFree"), Some(12_861_775_872));
        assert_eq!(plist_integer(plist, "Nope"), None);
    }

    #[test]
    fn the_tightest_volume_is_the_one_told() {
        let dir = scratch("probe");
        let status = probe(&[dir.clone(), dir.join("not-yet")]).unwrap();
        assert!(status.total > 0 && status.free <= status.total);
        assert_eq!(status.path, dir);
    }

    // ── a temporary tree ──

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn scratch(name: &str) -> PathBuf {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("equipe-disk-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn fill(dir: &Path, bytes: usize) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("blob"), vec![7u8; bytes]).unwrap();
    }

    /// equipe's home with a repository `site` and a worktree per ticket.
    struct Tree {
        home: PathBuf,
        repo: PathBuf,
        outside: PathBuf,
    }

    impl Tree {
        fn new() -> Self {
            let root = scratch("tree");
            let (home, repo, outside) = (root.join("equipe"), root.join("site"), root.join("persons"));
            std::fs::create_dir_all(&home).unwrap();
            std::fs::create_dir_all(&repo).unwrap();
            git(&repo, &["init", "-q", "-b", "main"]);
            git(&repo, &["config", "user.name", "T"]);
            git(&repo, &["config", "user.email", "t@example.com"]);
            std::fs::write(repo.join(".gitignore"), "target\nnode_modules\ndist/\n").unwrap();
            std::fs::create_dir_all(repo.join("build")).unwrap();
            std::fs::write(repo.join("build/release.sh"), "echo ok\n").unwrap();
            git(&repo, &["add", "-A"]);
            git(&repo, &["commit", "-q", "-m", "init"]);
            fill(&outside, 4096);
            Tree { home, repo, outside }
        }

        /// A worktree for ticket `id`, with a `target/` of `bytes`.
        fn worktree(&self, id: TicketId, bytes: usize) -> PathBuf {
            let path = self.home.join("worktrees").join("site").join(format!("t{id}-6ad1cb4a"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            git(&self.repo, &["worktree", "add", "-q", "-b", &format!("equipe/t{id}"), &path.to_string_lossy()]);
            fill(&path.join("target").join("debug"), bytes);
            path.canonicalize().unwrap()
        }

        fn ticket(&self, id: TicketId, status: TicketStatus, merged: Option<bool>, worktree: Option<&Path>) -> KnownTicket {
            KnownTicket { squad: "acme".into(), id, status, merged, worktree: worktree.map(Path::to_owned), project: Some(self.repo.clone()) }
        }
    }

    fn find<'a>(items: &'a [DiskItem], kind: DiskKind, path: &Path) -> &'a DiskItem {
        items.iter().find(|i| i.kind == kind && i.path == path).unwrap_or_else(|| panic!("no {kind:?} at {}: {items:#?}", path.display()))
    }

    #[test]
    fn worktrees_are_sized_and_told_safe_only_when_their_ticket_is_done_with() {
        let tree = Tree::new();
        let merged = tree.worktree(1, 3 * MB as usize);
        let cancelled = tree.worktree(2, 2 * MB as usize);
        let open = tree.worktree(3, 2 * MB as usize);
        let dirty = tree.worktree(4, 2 * MB as usize);
        std::fs::write(dirty.join("README.md"), "half done\n").unwrap();
        let orphan = tree.worktree(5, 2 * MB as usize);
        let busy = tree.worktree(6, 2 * MB as usize);
        let not_merged = tree.worktree(7, 2 * MB as usize);
        let known = Known {
            home: tree.home.clone(),
            tickets: vec![
                tree.ticket(1, TicketStatus::Done, Some(true), None),
                tree.ticket(2, TicketStatus::Cancelled, None, None),
                tree.ticket(3, TicketStatus::Review, None, Some(&open)),
                tree.ticket(4, TicketStatus::Done, Some(true), None),
                tree.ticket(6, TicketStatus::InProgress, None, Some(&busy)),
                tree.ticket(7, TicketStatus::Done, Some(false), None),
            ],
            in_use: vec![busy.clone()],
            ..Known::default()
        };
        let (items, partial) = scan(&known, SCAN_BUDGET);
        assert!(!partial);
        let build = find(&items, DiskKind::WorktreeBuild, &merged.join("target"));
        assert!(build.safe && build.why == DiskWhy::Merged && build.bytes >= 3 * MB && build.ticket == Some(1), "{build:?}");
        assert_eq!(build.label, "T-1 · site · target/");
        assert!(find(&items, DiskKind::Worktree, &merged).safe);
        assert_eq!(find(&items, DiskKind::Worktree, &cancelled).why, DiskWhy::Cancelled);
        assert_eq!(find(&items, DiskKind::Worktree, &orphan).why, DiskWhy::NoTicket);
        for (path, why) in [(&open, DiskWhy::OpenTicket), (&dirty, DiskWhy::Uncommitted), (&busy, DiskWhy::InUse), (&not_merged, DiskWhy::NotMerged)] {
            let item = find(&items, DiskKind::Worktree, path);
            assert!(!item.safe && item.why == why, "{item:?}");
            assert!(!find(&items, DiskKind::WorktreeBuild, &path.join("target")).safe);
        }
        // A worktree's own bytes leave out its build output.
        assert!(find(&items, DiskKind::Worktree, &merged).bytes < MB);
        // What equipe may give back comes first, biggest first.
        assert!(items[0].safe && items[0].path == merged.join("target"));
    }

    #[test]
    fn a_build_folder_of_sources_is_not_build_output() {
        let tree = Tree::new();
        let path = tree.worktree(1, MB as usize * 2);
        fill(&path.join("dist"), MB as usize * 2);
        fill(&path.join("packages").join("web").join("node_modules"), MB as usize * 2);
        let dirs = build_dirs(&path);
        assert!(dirs.contains(&path.join("target")) && dirs.contains(&path.join("dist")) && dirs.contains(&path.join("packages/web/node_modules")), "{dirs:?}");
        // `build/` holds a script git tracks: left alone.
        assert!(!dirs.contains(&path.join("build")));
    }

    #[test]
    fn cleaning_gives_back_what_is_safe_and_nothing_else() {
        let tree = Tree::new();
        let merged = tree.worktree(1, 3 * MB as usize);
        let open = tree.worktree(3, 2 * MB as usize);
        let dirty = tree.worktree(4, 2 * MB as usize);
        std::fs::write(dirty.join("README.md"), "half done\n").unwrap();
        let known = Known {
            home: tree.home.clone(),
            tickets: vec![tree.ticket(1, TicketStatus::Done, Some(true), None), tree.ticket(3, TicketStatus::Review, None, Some(&open)), tree.ticket(4, TicketStatus::Cancelled, None, None)],
            ..Known::default()
        };
        let (items, _) = scan(&known, SCAN_BUDGET);
        // Everything asked for, the unsafe too: only the safe goes.
        let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let done = clean(&known, &ids);
        assert!(!merged.exists(), "the finished worktree went");
        assert!(open.join("target").exists() && dirty.join("target").exists() && dirty.join("README.md").exists());
        assert!(done.freed >= 3 * MB, "{done:?}");
        assert!(done.skipped.iter().any(|(id, why)| id.contains("t4-") && *why == DiskWhy::Uncommitted), "{done:?}");
        // Its branch stays.
        git(&tree.repo, &["rev-parse", "--verify", "equipe/t1"]);
        // A made-up id does nothing.
        let none = clean(&known, &[format!("worktree:{}", tree.outside.display()), format!("build_cache:{}", tree.repo.display())]);
        assert_eq!(none.removed.len(), 0);
        assert!(tree.outside.join("blob").exists() && tree.repo.join(".gitignore").exists());
    }

    #[test]
    fn links_out_of_equipes_folders_are_never_followed() {
        let tree = Tree::new();
        let merged = tree.worktree(1, MB as usize * 2);
        // A link in the build output to the person's files, and a build folder that is a link.
        std::os::unix::fs::symlink(&tree.outside, merged.join("target").join("persons")).unwrap();
        std::os::unix::fs::symlink(&tree.outside, merged.join("node_modules")).unwrap();
        // A member's build folder that is a link out.
        let builds = tree.home.join("cache").join("build").join("acme");
        std::fs::create_dir_all(&builds).unwrap();
        std::os::unix::fs::symlink(&tree.outside, builds.join("frontend")).unwrap();
        let known = Known { home: tree.home.clone(), tickets: vec![tree.ticket(1, TicketStatus::Done, Some(true), None)], ..Known::default() };
        let (items, _) = scan(&known, SCAN_BUDGET);
        assert!(!items.iter().any(|i| i.path.starts_with(&tree.outside) || i.path == merged.join("node_modules") || i.path == builds.join("frontend")), "{items:#?}");
        let build = find(&items, DiskKind::WorktreeBuild, &merged.join("target"));
        assert!(build.bytes < 3 * MB, "the link's target isn't counted");
        let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        clean(&known, &ids);
        assert!(!merged.exists());
        assert!(tree.outside.join("blob").exists(), "the person's files stay");
    }

    #[test]
    fn nothing_outside_equipes_folders_is_removable() {
        let tree = Tree::new();
        let roots = roots(&tree.home);
        let inside = tree.home.join("cache").join("build").join("acme").join("pm");
        fill(&inside, 10);
        assert!(removable(&inside, &roots));
        assert!(!removable(&tree.outside, &roots));
        assert!(!removable(&tree.home, &roots), "never a root itself");
        assert!(!removable(&tree.home.join("..").join("persons"), &roots));
        // Through a linked folder that leads out.
        std::os::unix::fs::symlink(&tree.outside, tree.home.join("away")).unwrap();
        assert!(!removable(&tree.home.join("away").join("blob"), &roots));
    }

    #[test]
    fn members_build_folders_are_safe_unless_one_builds_now() {
        let tree = Tree::new();
        let builds = tree.home.join("cache").join("build").join("acme");
        fill(&builds.join("frontend").join("debug"), 2 * MB as usize);
        fill(&builds.join("backend").join("debug"), 2 * MB as usize);
        let known = Known { home: tree.home.clone(), building: vec![builds.join("backend")], ..Known::default() };
        let (items, _) = scan(&known, SCAN_BUDGET);
        let front = find(&items, DiskKind::BuildCache, &builds.join("frontend"));
        assert!(front.safe && front.why == DiskWhy::Rebuildable && front.label == "frontend · acme", "{front:?}");
        let back = find(&items, DiskKind::BuildCache, &builds.join("backend"));
        assert!(!back.safe && back.why == DiskWhy::InUse);
        let done = clean(&known, &[front.id.clone(), back.id.clone()]);
        assert!(!builds.join("frontend").exists() && builds.join("backend").exists());
        assert_eq!(done.removed, vec![front.id.clone()]);
    }

    /// Cargo building in `dir` (a profile): its lock held until dropped.
    fn building_in(dir: &Path) -> std::fs::File {
        std::fs::create_dir_all(dir).unwrap();
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(dir.join(".cargo-build-lock")).unwrap();
        lock.lock().unwrap();
        lock
    }

    #[test]
    fn a_working_members_incremental_cache_goes_between_its_builds() {
        let tree = Tree::new();
        let back = tree.home.join("cache").join("build").join("acme").join("backend");
        fill(&back.join("debug").join("deps"), 2 * MB as usize);
        fill(&back.join("debug").join("incremental"), 2 * MB as usize);
        let known = Known { home: tree.home.clone(), building: vec![back.clone()], ..Known::default() };
        let (items, _) = scan(&known, SCAN_BUDGET);
        let cache = find(&items, DiskKind::BuildCache, &back.join("debug").join("incremental"));
        assert!(cache.safe && cache.label == "backend · acme · debug/incremental/", "{cache:?}");
        let whole = find(&items, DiskKind::BuildCache, &back);
        assert!(!whole.safe && whole.bytes < 3 * MB, "the cache isn't counted twice: {whole:?}");

        // While cargo builds there it isn't offered, and a click from before takes nothing.
        let held = building_in(&back.join("debug"));
        assert!(!scan(&known, SCAN_BUDGET).0.iter().any(|i| i.path == cache.path));
        assert!(clean(&known, std::slice::from_ref(&cache.id)).removed.is_empty());
        assert!(back.join("debug").join("incremental").exists());
        drop(held);

        // Let go: free again — after a moment, if a fork elsewhere in this process held it too.
        let removed = (0..100).map(|_| clean(&known, std::slice::from_ref(&cache.id)).removed).find(|r| !r.is_empty() || { std::thread::sleep(Duration::from_millis(10)); false });
        assert_eq!(removed, Some(vec![cache.id.clone()]));
        assert!(!back.join("debug").join("incremental").exists() && back.join("debug").join("deps").exists());
    }

    #[test]
    fn a_build_folder_being_built_in_is_never_removed() {
        let tree = Tree::new();
        let front = tree.home.join("cache").join("build").join("acme").join("frontend");
        fill(&front.join("debug").join("deps"), 2 * MB as usize);
        let known = Known { home: tree.home.clone(), ..Known::default() };
        let item = find(&scan(&known, SCAN_BUDGET).0, DiskKind::BuildCache, &front).clone();
        // A build began after the look: the folder stays, and says why.
        let held = building_in(&front.join("debug"));
        let done = clean(&known, std::slice::from_ref(&item.id));
        assert!(done.removed.is_empty() && done.skipped == vec![(item.id.clone(), DiskWhy::InUse)] && front.exists());
        drop(held);
    }

    #[test]
    fn a_short_disk_trims_only_caches_no_build_needs() {
        let tree = Tree::new();
        let builds = tree.home.join("cache").join("build").join("acme");
        fill(&builds.join("pm").join("debug").join("incremental"), MB as usize);
        fill(&builds.join("pm").join("debug").join("deps"), MB as usize);
        fill(&builds.join("qa").join("aarch64-apple-darwin").join("debug").join("incremental"), MB as usize);
        fill(&builds.join("mobile").join("debug").join("incremental"), MB as usize);
        let held = building_in(&builds.join("mobile").join("debug"));
        let (freed, done) = trim_builds(&tree.home, false);
        assert!(!builds.join("pm").join("debug").join("incremental").exists() && builds.join("pm").join("debug").join("deps").exists());
        assert!(!builds.join("qa").join("aarch64-apple-darwin").join("debug").join("incremental").exists(), "per target too");
        assert!(builds.join("mobile").join("debug").join("incremental").exists(), "cargo builds there now");
        assert!(done.len() == 2 && freed >= 2 * MB, "{done:?} {freed}");
        drop(held);

        // Nearly full: a whole folder nobody built in for days goes too.
        let old = std::time::SystemTime::now() - STALE_AFTER - Duration::from_secs(60);
        for dir in [builds.join("pm"), builds.join("pm").join("debug")] {
            std::fs::File::open(&dir).unwrap().set_modified(old).unwrap();
        }
        trim_builds(&tree.home, false);
        assert!(builds.join("pm").exists(), "only when nearly full");
        trim_builds(&tree.home, true);
        assert!(!builds.join("pm").exists() && builds.join("mobile").join("debug").exists());
    }

    #[test]
    fn an_old_log_keeps_its_last_part() {
        let tree = Tree::new();
        let log = tree.home.join("daemon.log");
        let line = "equipe: something happened\n";
        std::fs::write(&log, line.repeat((LOG_PAST as usize / line.len()) + 1000)).unwrap();
        let known = Known { home: tree.home.clone(), ..Known::default() };
        let (items, _) = scan(&known, SCAN_BUDGET);
        let item = find(&items, DiskKind::Log, &log);
        assert!(item.safe && item.why == DiskWhy::OldLog);
        clean(&known, std::slice::from_ref(&item.id));
        let kept = std::fs::read_to_string(&log).unwrap();
        assert!(kept.len() as u64 <= LOG_KEEP && kept.starts_with("equipe:") && kept.ends_with('\n'));
    }

    #[test]
    fn project_build_output_is_reported_with_its_command_never_cleaned() {
        let tree = Tree::new();
        fill(&tree.repo.join("target").join("debug"), 60 * MB as usize);
        let agent = tree.repo.join(".claude").join("worktrees").join("agent-1");
        git(&tree.repo, &["worktree", "add", "-q", "-b", "agent-1", &agent.to_string_lossy()]);
        fill(&agent.join("target").join("debug"), 60 * MB as usize);
        let known = Known { home: tree.home.clone(), projects: vec![("acme".into(), tree.repo.clone())], ..Known::default() };
        let (items, _) = scan(&known, SCAN_BUDGET);
        let own = find(&items, DiskKind::ProjectBuild, &tree.repo.join("target"));
        assert!(!own.safe && own.why == DiskWhy::NotEquipes && own.command.as_deref() == Some(format!("rm -rf {}", tree.repo.join("target").display()).as_str()));
        let theirs = find(&items, DiskKind::ProjectBuild, &agent.join("target"));
        assert_eq!(theirs.label, "site · .claude/worktrees/agent-1/target/");
        clean(&known, &[own.id.clone(), theirs.id.clone()]);
        assert!(tree.repo.join("target").exists() && agent.join("target").exists());
    }

    #[test]
    fn a_look_stops_at_its_budget_and_says_so() {
        let tree = Tree::new();
        let builds = tree.home.join("cache").join("build").join("acme").join("pm");
        for n in 0..2000 {
            std::fs::create_dir_all(builds.join(format!("d{n}"))).unwrap();
        }
        fill(&builds, 2 * MB as usize);
        let known = Known { home: tree.home.clone(), ..Known::default() };
        let (_, partial) = scan(&known, Duration::ZERO);
        assert!(partial);
    }

    #[test]
    fn a_finished_tickets_build_output_goes_and_its_worktree_stays() {
        let tree = Tree::new();
        let path = tree.worktree(1, 2 * MB as usize);
        std::fs::write(path.join("notes.md"), "kept\n").unwrap();
        let done = clear_builds(&tree.home, &path);
        assert_eq!(done.len(), 1);
        assert!(!path.join("target").exists() && path.join("notes.md").exists());
        // Never outside equipe's worktrees.
        fill(&tree.repo.join("target"), 10);
        assert!(clear_builds(&tree.home, &tree.repo).is_empty());
        assert!(tree.repo.join("target").exists());
    }
}
