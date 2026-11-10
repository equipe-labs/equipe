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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
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
