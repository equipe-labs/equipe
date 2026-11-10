//! Ending a process and what it started: its process group, signalled
//! directly. Never by running `kill`: its parsing differs by system — on
//! Ubuntu 24.04 (procps-ng 4.0.4) `kill -TERM -<pid>` of a pid that leads no
//! group sent SIGTERM to every process of the user, and CI's runner died of
//! it mid-test.

/// Send `signal` to `pid`'s group when `pid` leads one (started with
/// `process_group(0)`), else to `pid` alone. Nothing for pids 0 and 1, or a
/// pid gone. True when something was signalled.
#[derive(Debug, Clone, Copy)]
pub enum Signal {
    Terminate,
    Kill,
}

#[cfg(unix)]
pub fn end(pid: u32, signal: Signal) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else { return false };
    if pid <= 1 {
        return false;
    }
    // SAFETY: plain system calls on a number; a stale pid fails with ESRCH.
    unsafe {
        let signal = match signal {
            Signal::Terminate => libc::SIGTERM,
            Signal::Kill => libc::SIGKILL,
        };
        if libc::getpgid(pid) == pid {
            libc::killpg(pid, signal) == 0
        } else {
            libc::kill(pid, signal) == 0
        }
    }
}

#[cfg(windows)]
pub fn end(pid: u32, _signal: Signal) -> bool {
    if pid <= 1 {
        return false;
    }
    std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt as _;

    #[test]
    fn a_group_ends_whole_and_a_lone_process_ends_alone() {
        // A group leader with a child of its own: both go.
        let mut leader = std::process::Command::new("sh").arg("-c").arg("sleep 30 & wait").process_group(0).spawn().unwrap();
        // A process in this test's own group: only it goes, not the group it is in.
        let mut lone = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        // A bystander in this group: untouched.
        let mut bystander = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(end(leader.id(), Signal::Terminate));
        assert!(end(lone.id(), Signal::Terminate));
        assert!(!leader.wait().unwrap().success());
        assert!(!lone.wait().unwrap().success());
        assert!(bystander.try_wait().unwrap().is_none(), "nothing else was signalled");
        let _ = bystander.kill();
        let _ = bystander.wait();
        assert!(!end(0, Signal::Terminate) && !end(1, Signal::Terminate) && !end(u32::MAX, Signal::Terminate));
    }
}
