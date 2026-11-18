//! Processes a finished ticket left running: dev servers, test daemons, file
//! servers a member started from the ticket's worktree or from a scratch
//! folder in the system's temporary folder, then never stopped. They hold
//! memory and ports for as long as the machine is up, and keep the folder
//! they sit in from ever looking idle.
//!
//! A process belongs to a ticket only by where it stands: its working folder
//! or its program is inside one of the folders given. Nothing else is ever
//! signalled — not equipe's own process or the ones above it, not the engines
//! of live sessions (`keep`), not another user's, not a process whose place
//! can't be read.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A process and where it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    pub pid: u32,
    /// Its working folder, when it can be read.
    pub cwd: Option<PathBuf>,
    /// Its program's file, when it can be read.
    pub exe: Option<PathBuf>,
}

/// Every process of this user that can be read, where it stands.
#[cfg(target_os = "macos")]
pub fn running() -> Vec<Standing> {
    use std::mem;
    // SAFETY: libproc calls with buffers of the sizes passed; a process gone
    // between the list and its look simply fails to be read.
    unsafe {
        let want = libc::proc_listallpids(std::ptr::null_mut(), 0);
        if want <= 0 {
            return Vec::new();
        }
        let mut pids = vec![0 as libc::pid_t; want as usize + 64];
        let got = libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * mem::size_of::<libc::pid_t>()) as libc::c_int);
        if got <= 0 {
            return Vec::new();
        }
        let me = libc::getuid();
        let mut out = Vec::new();
        for &pid in pids.iter().take(got as usize).filter(|p| **p > 1) {
            // Owned by this user: the full record is refused for any other's.
            let mut bsd: libc::proc_bsdinfo = mem::zeroed();
            let size = mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
            if libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&mut bsd as *mut libc::proc_bsdinfo).cast(), size) != size || bsd.pbi_uid != me {
                continue;
            }
            let mut paths: libc::proc_vnodepathinfo = mem::zeroed();
            let size = mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
            let cwd = (libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, (&mut paths as *mut libc::proc_vnodepathinfo).cast(), size) == size)
                .then(|| c_path(paths.pvi_cdir.vip_path.as_ptr().cast::<u8>(), mem::size_of_val(&paths.pvi_cdir.vip_path)))
                .flatten();
            let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
            let n = libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32);
            let exe = (n > 0).then(|| PathBuf::from(String::from_utf8_lossy(&buf[..n as usize]).into_owned()));
            out.push(Standing { pid: pid as u32, cwd, exe });
        }
        out
    }
}

/// The NUL-ended path in `len` bytes at `at`.
#[cfg(target_os = "macos")]
unsafe fn c_path(at: *const u8, len: usize) -> Option<PathBuf> {
    // SAFETY: the caller gives `len` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(at, len) };
    let end = bytes.iter().position(|b| *b == 0)?;
    (end > 0).then(|| PathBuf::from(String::from_utf8_lossy(&bytes[..end]).into_owned()))
}

/// Every process of this user that can be read, where it stands.
#[cfg(target_os = "linux")]
pub fn running() -> Vec<Standing> {
    use std::os::unix::fs::MetadataExt as _;
    // SAFETY: a plain system call.
    let me = unsafe { libc::getuid() };
    let mut out = Vec::new();
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()).filter(|p| *p > 1) else { continue };
        let dir = entry.path();
        if std::fs::metadata(&dir).map(|m| m.uid()).ok() != Some(me) {
            continue;
        }
        out.push(Standing { pid, cwd: std::fs::read_link(dir.join("cwd")).ok(), exe: std::fs::read_link(dir.join("exe")).ok() });
    }
    out
}

/// Where processes stand isn't read on this system: none is ever stopped.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn running() -> Vec<Standing> {
    Vec::new()
}

/// Whether `path` is `root` or inside it.
fn inside(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

/// The folders to compare against, links resolved (`/tmp` is `/private/tmp`
/// on a Mac, and that is how a process's place is told). A folder that is
/// gone stays as given.
fn resolved(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots.iter().map(|r| r.canonicalize().unwrap_or_else(|_| r.clone())).filter(|r| r.components().count() > 2).collect()
}

/// Which of `all` stand in one of `roots`: by working folder or program. Never
/// `keep`, nor this process, nor its parents.
pub fn standing_in(all: &[Standing], roots: &[PathBuf], keep: &[u32]) -> Vec<u32> {
    let roots = resolved(roots);
    if roots.is_empty() {
        return Vec::new();
    }
    let above = ancestors(std::process::id());
    let mut pids: Vec<u32> = all
        .iter()
        .filter(|s| !keep.contains(&s.pid) && s.pid != std::process::id() && !above.contains(&s.pid))
        .filter(|s| [&s.cwd, &s.exe].into_iter().flatten().any(|p| roots.iter().any(|root| inside(p, root))))
        .map(|s| s.pid)
        .collect();
    pids.sort_unstable();
    pids
}

/// The processes standing in `roots` now.
pub fn holders(roots: &[PathBuf], keep: &[u32]) -> Vec<u32> {
    standing_in(&running(), roots, keep)
}

/// `pid`'s parents, up to the first.
fn ancestors(pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut at = pid;
    // Bounded: a chain cannot be this long, and a loop must not hang a clean.
    for _ in 0..64 {
        match crate::origin::parent_of(at) {
            crate::origin::Parent::Pid(parent) if !out.contains(&parent) => {
                out.push(parent);
                at = parent;
            }
            _ => break,
        }
    }
    out
}

/// Stop what stands in `roots`: asked politely, then, those still there after
/// a few seconds (and still standing there: a pid may be given to another),
/// for good. Each process alone, never its group — a group may hold what
/// stands elsewhere. What was stopped, for the log.
pub fn stop_in(roots: &[PathBuf], keep: &[u32]) -> Vec<String> {
    let found = standing_in(&running(), roots, keep);
    if found.is_empty() {
        return Vec::new();
    }
    for pid in &found {
        signal(*pid, false);
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut left = found.clone();
    while !left.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        left = standing_in(&running(), roots, keep).into_iter().filter(|p| found.contains(p)).collect();
    }
    for pid in &left {
        signal(*pid, true);
    }
    found.iter().map(|pid| format!("stopped process {pid} left running in {}", describe(roots))).collect()
}

fn describe(roots: &[PathBuf]) -> String {
    roots.first().map(|r| r.display().to_string()).unwrap_or_default()
}

#[cfg(unix)]
fn signal(pid: u32, kill: bool) {
    let Ok(pid) = libc::pid_t::try_from(pid) else { return };
    if pid <= 1 {
        return;
    }
    // SAFETY: a plain system call on a number; a stale pid fails with ESRCH.
    unsafe {
        libc::kill(pid, if kill { libc::SIGKILL } else { libc::SIGTERM });
    }
}

#[cfg(not(unix))]
fn signal(_pid: u32, _kill: bool) {}

// ── orphans: what members started, left behind by the shell that started it ──

/// In every member command's environment: whose run started it —
/// `<squad>:<session>:<ticket or ->` ([`run_tag`]). A process keeps it when
/// the shell that started it is gone and it is handed to the system
/// (parent 1), so what a lane left running is still known as the lane's.
pub const RUN_ENV: &str = "EQUIPE_RUN";

/// [`RUN_ENV`]'s value for a run.
pub fn run_tag(squad: &str, session: u64, ticket: Option<u64>) -> String {
    format!("{squad}:{session}:{}", ticket.map_or_else(|| "-".to_owned(), |t| t.to_string()))
}

/// A run's tag read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub squad: String,
    pub session: u64,
    pub ticket: Option<u64>,
}

impl Tag {
    pub fn parse(value: &str) -> Option<Tag> {
        let (rest, ticket) = value.rsplit_once(':')?;
        let (squad, session) = rest.rsplit_once(':')?;
        (!squad.is_empty()).then_some(())?;
        Some(Tag { squad: squad.to_owned(), session: session.parse().ok()?, ticket: if ticket == "-" { None } else { Some(ticket.parse().ok()?) } })
    }
}

/// A process of this user handed to the system (its parent gone) that a
/// member's command started: it has [`crate::guard::MEMBER_ENV`] — or, where
/// its environment can't be read, it may stand in a ticket's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    pub pid: u32,
    pub cwd: Option<PathBuf>,
    /// Its run, when it was started by an equipe that tags them.
    pub tag: Option<Tag>,
    /// Its command line, shortened, for the log.
    pub command: String,
}

/// What the daemon knows to judge an orphan by.
pub struct Judge<'a> {
    /// Where ticket `ticket` of squad `squad` stands; `None`: not known.
    pub status: &'a dyn Fn(&str, u64) -> Option<equipe_protocol::TicketStatus>,
    /// The squad and ticket a folder belongs to: a ticket's worktree, or its
    /// temporary folder.
    pub place: &'a dyn Fn(&Path) -> Option<(String, u64)>,
}

/// Why `orphan` goes, or `None` to leave it: its ticket — by its tag, or by
/// the ticket folder it stands in — is closed, or out of work and not in
/// review (put back, waiting on the person: a dev server left from its run).
/// One in progress or in review stays (QA or the person may be looking at
/// what it serves); one whose ticket isn't known, too.
pub fn verdict(orphan: &Orphan, judge: &Judge) -> Option<String> {
    use equipe_protocol::TicketStatus;
    let (squad, ticket) = match &orphan.tag {
        Some(tag) => (tag.squad.clone(), tag.ticket?),
        None => (judge.place)(orphan.cwd.as_deref()?)?,
    };
    match (judge.status)(&squad, ticket)? {
        status if status.is_closed() => Some(format!("T-{ticket} of {squad} is closed")),
        TicketStatus::InProgress | TicketStatus::Review => None,
        status => {
            let word = format!("{status:?}").to_lowercase();
            Some(format!("T-{ticket} of {squad} is no longer being worked on ({word})"))
        }
    }
}

/// The most dev servers members leave running for one project at once:
/// past it, the oldest go.
pub const DEV_SERVERS_PER_PROJECT: usize = 3;

/// Whether `command` runs a dev server or a watcher.
fn dev_server(command: &str) -> bool {
    ["next-server", "next dev", "vite", "webpack serve", "webpack-dev-server", "astro dev", "nuxt dev", "react-scripts start", "storybook dev", "start-storybook", "tsc --watch", "tsc -w", "nodemon"]
        .iter()
        .any(|m| command.contains(m))
}

/// Of `orphans` (those staying), the dev servers past
/// [`DEV_SERVERS_PER_PROJECT`] for one project — a worktree's project under
/// `<home>/worktrees/` — the oldest first; why each goes.
pub fn over_the_cap(orphans: &[Orphan], home: &Path) -> Vec<(Orphan, String)> {
    let root = home.join("worktrees");
    let root = root.canonicalize().unwrap_or(root);
    let mut by_project: std::collections::BTreeMap<String, Vec<&Orphan>> = std::collections::BTreeMap::new();
    for o in orphans.iter().filter(|o| dev_server(&o.command)) {
        let Some(project) = o.cwd.as_deref().and_then(|c| c.strip_prefix(&root).ok()).and_then(|r| r.components().next()).map(|c| c.as_os_str().to_string_lossy().into_owned()) else { continue };
        by_project.entry(project).or_default().push(o);
    }
    let mut out = Vec::new();
    for (project, mut servers) in by_project {
        // The newest stay: a later process has a higher number, near enough.
        servers.sort_by_key(|o| std::cmp::Reverse(o.pid));
        out.extend(servers.into_iter().skip(DEV_SERVERS_PER_PROJECT).map(|o| (o.clone(), format!("more than {DEV_SERVERS_PER_PROJECT} dev servers left running for {project}"))));
    }
    out
}

/// This user's processes handed to the system that members' commands started.
pub fn orphans() -> Vec<Orphan> {
    let me = std::process::id();
    running()
        .into_iter()
        .filter(|s| s.pid != me && parent(s.pid) == Some(1))
        .filter_map(|s| {
            let (env, command) = environment(s.pid).unwrap_or_default();
            let var = |name: &str| env.iter().find_map(|e| e.strip_prefix(name).and_then(|v| v.strip_prefix('=')).map(str::to_owned));
            // Where its environment can be read, only a member's. Recent macOS
            // shows another process's environment rarely: then where it stands
            // decides alone (`verdict`: a ticket's folder of equipe's own).
            if !env.is_empty() && var(crate::guard::MEMBER_ENV).is_none_or(|m| m.is_empty()) {
                return None;
            }
            Some(Orphan { pid: s.pid, cwd: s.cwd, tag: var(RUN_ENV).and_then(|t| Tag::parse(&t)), command })
        })
        .collect()
}

/// Stop `pids`, politely then for good after a few seconds. What was stopped.
pub fn reap(orphans: &[(Orphan, String)]) -> Vec<String> {
    for (o, _) in orphans {
        signal(o.pid, false);
    }
    std::thread::sleep(Duration::from_secs(4));
    let still: Vec<u32> = running().iter().map(|s| s.pid).collect();
    for (o, _) in orphans {
        // Still the same process (handed to the system, a member's): for good.
        if still.contains(&o.pid) && parent(o.pid) == Some(1) {
            signal(o.pid, true);
        }
    }
    orphans.iter().map(|(o, why)| format!("stopped process {} left running by a member ({why}): {}", o.pid, o.command)).collect()
}

/// `pid`'s parent.
#[cfg(target_os = "macos")]
fn parent(pid: u32) -> Option<u32> {
    // SAFETY: a libproc call with a buffer of the size passed.
    unsafe {
        let mut bsd: libc::proc_bsdinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        (libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, (&mut bsd as *mut libc::proc_bsdinfo).cast(), size) == size).then_some(bsd.pbi_ppid)
    }
}

#[cfg(target_os = "linux")]
fn parent(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn parent(_pid: u32) -> Option<u32> {
    None
}

/// `pid`'s environment, and its command line shortened.
#[cfg(target_os = "macos")]
fn environment(pid: u32) -> Option<(Vec<String>, String)> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    let mut size: libc::size_t = 0;
    // SAFETY: sysctl asked for its size first, then into a buffer of that size.
    let buf = unsafe {
        if libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) != 0 || size < 4 {
            return None;
        }
        let mut buf = vec![0u8; size];
        if libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
        buf.truncate(size);
        buf
    };
    procargs(&buf)
}

/// `KERN_PROCARGS2`'s layout: the argument count, the program's path, NULs,
/// the arguments, then the environment, each NUL-ended.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn procargs(buf: &[u8]) -> Option<(Vec<String>, String)> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?).max(0) as usize;
    let mut parts = buf[4..].split(|b| *b == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned());
    let _program = parts.next()?;
    let args: Vec<String> = parts.by_ref().take(argc).collect();
    Some((parts.collect(), shortened(&args.join(" "))))
}

#[cfg(target_os = "linux")]
fn environment(pid: u32) -> Option<(Vec<String>, String)> {
    let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    let split = |b: &[u8]| b.split(|c| *c == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).collect::<Vec<_>>();
    Some((split(&env), shortened(&split(&cmd).join(" "))))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn environment(_pid: u32) -> Option<(Vec<String>, String)> {
    None
}

fn shortened(command: &str) -> String {
    let mut s: String = command.chars().take(120).collect();
    if command.chars().count() > 120 {
        s.push('…');
    }
    s
}

/// The squad and ticket an equipe folder belongs to, by its shape:
/// `<home>/tmp/<squad>/t<ticket>/…`, or a worktree
/// `<home>/worktrees/<project>/t<ticket>-…/…` of a project only `squads`
/// (each with its projects' folder names) has.
pub fn ticket_place(path: &Path, home: &Path, squads: &[(String, Vec<String>)]) -> Option<(String, u64)> {
    let ticket_of = |part: &str| -> Option<u64> {
        let digits: String = part.strip_prefix('t')?.chars().take_while(char::is_ascii_digit).collect();
        let rest = &part[1 + digits.len()..];
        (rest.is_empty() || rest.starts_with('-')).then_some(())?;
        digits.parse().ok()
    };
    let parts = |root: PathBuf| -> Option<Vec<String>> {
        let root = root.canonicalize().unwrap_or(root);
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        Some(path.strip_prefix(&root).ok()?.components().take(2).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect())
    };
    if let Some([squad, folder]) = parts(home.join("tmp")).as_deref() {
        return Some((squad.clone(), ticket_of(folder)?));
    }
    let [project, folder] = parts(home.join("worktrees"))?.try_into().ok()?;
    let mut owners = squads.iter().filter(|(_, projects)| projects.contains(&project));
    let (squad, _) = owners.next()?;
    // A project two squads share: whose ticket it is can't be told.
    owners.next().is_none().then_some(())?;
    Some((squad.clone(), ticket_of(&folder)?))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_runs_tag_is_read_back() {
        assert_eq!(Tag::parse(&run_tag("acme", 42, Some(3311))), Some(Tag { squad: "acme".into(), session: 42, ticket: Some(3311) }));
        assert_eq!(Tag::parse(&run_tag("my:squad", 7, None)), Some(Tag { squad: "my:squad".into(), session: 7, ticket: None }));
        assert_eq!(Tag::parse("nonsense"), None);
    }

    #[test]
    fn an_orphan_goes_only_when_its_ticket_is_closed() {
        let home = Path::new("/h/.equipe");
        let squads = vec![("atinum".to_owned(), vec!["atinum-website".to_owned()]), ("other".to_owned(), vec!["shared".to_owned()]), ("third".to_owned(), vec!["shared".to_owned()])];
        let place = |p: &Path| ticket_place(p, home, &squads);
        assert_eq!(place(Path::new("/h/.equipe/worktrees/atinum-website/t3311-6af0c1-1/design/directions/x")), Some(("atinum".into(), 3311)));
        assert_eq!(place(Path::new("/h/.equipe/tmp/atinum/t3443")), Some(("atinum".into(), 3443)));
        assert_eq!(place(Path::new("/h/.equipe/worktrees/shared/t9-a-1")), None, "two squads share it: unknown");
        assert_eq!(place(Path::new("/h/.equipe/worktrees/atinum-website/tools")), None);
        assert_eq!(place(Path::new("/Users/me/Projects/site")), None);
        use equipe_protocol::TicketStatus;
        let status = |squad: &str, t: u64| match (squad, t) {
            ("atinum", 3311) => Some(TicketStatus::Done),
            ("atinum", 3312) => Some(TicketStatus::Todo),
            ("atinum", 3313) => Some(TicketStatus::Review),
            ("atinum", 3800) => Some(TicketStatus::InProgress),
            _ => None,
        };
        let judge = Judge { status: &status, place: &place };
        let legacy = Orphan { pid: 13700, cwd: Some("/h/.equipe/worktrees/atinum-website/t3311-6af0c1-1/design".into()), tag: None, command: "node -e …chromium.launch…".into() };
        assert_eq!(verdict(&legacy, &judge).as_deref(), Some("T-3311 of atinum is closed"));
        let open = Orphan { cwd: Some("/h/.equipe/worktrees/atinum-website/t3800-6af0c1-1".into()), ..legacy.clone() };
        assert_eq!(verdict(&open, &judge), None, "its ticket in progress: a server the run uses");
        let at = |t: u64| Orphan { cwd: Some(format!("/h/.equipe/worktrees/atinum-website/t{t}-6af0c1-1").into()), ..legacy.clone() };
        assert_eq!(verdict(&at(3312), &judge).as_deref(), Some("T-3312 of atinum is no longer being worked on (todo)"), "put back: its server goes");
        assert_eq!(verdict(&at(3313), &judge), None, "in review: QA or the person may be looking");
        assert_eq!(verdict(&at(4000), &judge), None, "a ticket not known: never");
        let tagged = Orphan { cwd: Some("/anywhere".into()), tag: Some(Tag { squad: "atinum".into(), session: 9, ticket: Some(3311) }), ..legacy.clone() };
        assert!(verdict(&tagged, &judge).is_some(), "by its tag, wherever it stands");
        let desk = Orphan { tag: Some(Tag { squad: "atinum".into(), session: 9, ticket: None }), ..tagged.clone() };
        assert_eq!(verdict(&desk, &judge), None, "no ticket: never");
        assert_eq!(verdict(&Orphan { cwd: None, ..legacy }, &judge), None, "nowhere known: never");
    }

    #[test]
    fn dev_servers_past_the_cap_for_one_project_go_the_oldest_first() {
        let home = Path::new("/h/.equipe");
        let server = |pid: u32, project: &str, command: &str| Orphan { pid, cwd: Some(format!("/h/.equipe/worktrees/{project}/t{pid}-a-1").into()), tag: None, command: command.into() };
        let orphans = vec![
            server(10, "site", "node .../next/dist/server/next-server"),
            server(20, "site", "next-server (v15)"),
            server(30, "site", "node node_modules/.bin/vite"),
            server(40, "site", "next-server (v15)"),
            server(50, "site", "node scripts/mock-anthropic.mjs"),
            server(60, "erp", "next-server (v15)"),
        ];
        let going = over_the_cap(&orphans, home);
        assert_eq!(going.iter().map(|(o, _)| o.pid).collect::<Vec<_>>(), [10], "site's three newest stay; a mock isn't a dev server; erp has one");
        assert!(going[0].1.contains("more than 3 dev servers left running for site"));
    }

    #[test]
    fn procargs_give_the_environment_after_the_arguments() {
        let mut buf = 2i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/usr/bin/node\0\0\0node\0-e\0EQUIPE_MEMBER=designer\0EQUIPE_RUN=acme:4:12\0");
        let (env, command) = procargs(&buf).unwrap();
        assert_eq!(env, ["EQUIPE_MEMBER=designer", "EQUIPE_RUN=acme:4:12"]);
        assert_eq!(command, "node -e");
    }

    #[test]
    fn a_member_process_left_behind_is_found_with_its_tag() {
        // A child that outlives its shell: started through `sh -c '… &'`, so the system takes it over.
        let out = Command::new("sh")
            .args(["-c", "sleep 30 >/dev/null 2>&1 & echo $!"])
            .env(crate::guard::MEMBER_ENV, "designer")
            .env(RUN_ENV, run_tag("acme", 5, Some(77)))
            .stdout(Stdio::piped())
            .output()
            .unwrap();
        let pid: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        // Taken over by the system itself — not by a reaper of a container's own.
        let adopted = parent(pid) == Some(1);
        let found = orphans().into_iter().find(|o| o.pid == pid);
        signal(pid, true);
        if adopted {
            let found = found.expect("found as an orphan");
            // Its tag where the system shows another process's environment (Linux; not recent macOS).
            if let Some(tag) = found.tag {
                assert_eq!(tag, Tag { squad: "acme".into(), session: 5, ticket: Some(77) });
            }
            assert!(found.command.starts_with("sleep 30"), "{}", found.command);
        }
    }
    use std::process::{Command, Stdio};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-leftovers-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn at(pid: u32, cwd: &str) -> Standing {
        Standing { pid, cwd: Some(PathBuf::from(cwd)), exe: None }
    }

    #[test]
    fn a_process_belongs_to_a_folder_by_its_working_folder_or_its_program() {
        let all = vec![
            at(100, "/tmp/t757"),
            at(101, "/tmp/t757/app/.next"),
            at(102, "/tmp/t7570"),
            at(103, "/tmp"),
            at(104, "/Users/p/code"),
            Standing { pid: 105, cwd: Some(PathBuf::from("/")), exe: Some(PathBuf::from("/tmp/t757/target/debug/fake_daemon")) },
            Standing { pid: 106, cwd: None, exe: None },
        ];
        assert_eq!(standing_in(&all, &[PathBuf::from("/tmp/t757")], &[]), [100, 101, 105]);
        assert_eq!(standing_in(&all, &[PathBuf::from("/tmp/t757")], &[101]), [100, 105], "an engine of a live session is kept");
    }

    #[test]
    fn a_whole_temporary_folder_or_a_home_is_never_a_ticket_s_to_clear() {
        let all = vec![at(100, "/tmp"), at(101, "/tmp/x"), at(102, "/Users/p")];
        for root in ["/", "/tmp", "/Users"] {
            assert!(standing_in(&all, &[PathBuf::from(root)], &[]).is_empty(), "{root}");
        }
    }

    #[test]
    fn this_process_and_its_parents_are_never_found() {
        let cwd = std::env::current_dir().unwrap();
        let me = Standing { pid: std::process::id(), cwd: Some(cwd.clone()), exe: None };
        let parent = ancestors(std::process::id()).first().map(|p| Standing { pid: *p, cwd: Some(cwd.clone()), exe: None });
        let all: Vec<Standing> = std::iter::once(me).chain(parent).collect();
        assert!(standing_in(&all, &[cwd.join("deeper").parent().unwrap().to_owned()], &[]).is_empty());
    }

    #[test]
    fn what_a_ticket_left_running_in_its_folders_is_stopped_and_no_other_process_is() {
        let dir = scratch("stop");
        let mine = dir.join("t757-build");
        let other = dir.join("t758-build");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let spawn = |cwd: &Path| Command::new("sleep").arg("60").current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::null()).spawn().unwrap();
        let mut left = spawn(&mine);
        let mut deeper = {
            std::fs::create_dir_all(mine.join("a/b")).unwrap();
            spawn(&mine.join("a/b"))
        };
        let mut bystander = spawn(&other);
        let mut outside = spawn(&std::env::temp_dir());
        std::thread::sleep(Duration::from_millis(300));
        let lines = stop_in(std::slice::from_ref(&mine), &[]);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(left.wait().is_ok() && deeper.wait().is_ok(), "both ended");
        assert!(bystander.try_wait().unwrap().is_none() && outside.try_wait().unwrap().is_none(), "the others run on");
        let _ = (bystander.kill(), outside.kill(), bystander.wait(), outside.wait());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_process_that_ignores_the_polite_request_is_ended_for_good() {
        let dir = scratch("stubborn");
        let mut stubborn = Command::new("sh").args(["-c", "trap '' TERM; while :; do sleep 1; done"]).current_dir(&dir).stdin(Stdio::null()).stdout(Stdio::null()).spawn().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let lines = stop_in(std::slice::from_ref(&dir), &[]);
        assert!(!lines.is_empty(), "{lines:?}");
        assert!(stubborn.wait().is_ok_and(|s| !s.success()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
