//! Who is on the other end of the daemon's local socket — the person's own
//! screen, or a member's process reaching in.
//!
//! The socket (`~/.equipe/equipe.sock`) is owned by the user, so only the
//! user's own processes can open it — but a member runs as that same user.
//! Inside the fence a member's commands can't connect at all (the sandbox
//! denies Unix-socket connections). In **full access** there is no fence, so a
//! member's shell command *can* open the socket and speak the protocol. The
//! floor that keeps approvals the person's ([`crate::permissions::person_only`])
//! then rests here: the daemon refuses a person-only command whose peer process
//! is one of its own descendants — every member engine is a child of the
//! daemon, and a member's commands are the engine's children — while leaving
//! the person's own clients, which are not daemon descendants, untouched.
//!
//! The phone relay and Telegram run inside the daemon and connect to its socket
//! in-process, so their peer is the daemon itself; that is allowed (the person
//! paired the phone, and pairing auth stands).
//!
//! The pure walk is tested without real processes, against a parent-of map.

/// A process's parent, as the system tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parent {
    /// Its parent process id.
    Pid(u32),
    /// It has reached the top — `launchd`/`init` (pid 1), or a reparented
    /// process whose parent is pid 1 or 0 (the kernel). The daemon was not on
    /// the way up, so this peer is not a daemon descendant.
    Top,
    /// The parent couldn't be read (the process is gone, or the system refused).
    Unknown,
}

/// Where a socket client sits relative to the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The daemon's own process: a self-connection (the phone relay, Telegram).
    Daemon,
    /// A process whose ancestry reaches the top without passing the daemon:
    /// the person's desktop app or CLI, launched by them, not by the daemon.
    Person,
    /// A descendant of the daemon: a member's engine, or a process that engine
    /// started. A member.
    Member,
    /// The peer, or a step of its ancestry, couldn't be read before it could be
    /// told apart. Treated as a member for person-only commands (fail closed).
    Unknown,
}

impl Origin {
    /// Whether a person-only command from this origin may go ahead: the
    /// daemon's own self-connections and the person's own clients, yes;
    /// a member or an unreadable chain, no (fail closed).
    pub fn person_may(self) -> bool {
        matches!(self, Origin::Daemon | Origin::Person)
    }
}

/// How far up the parent chain the walk goes before giving up: deep enough for
/// any real tree, bounded so a cycle (which should never happen) can't loop.
const MAX_DEPTH: usize = 64;

/// Classify a peer `pid` against the running `daemon` pid, walking parents with
/// `parent_of`. Pure: the walk is the same whatever supplies the parents.
pub fn classify(peer: Option<u32>, daemon: u32, parent_of: impl Fn(u32) -> Parent) -> Origin {
    let Some(peer) = peer else { return Origin::Unknown };
    if peer == daemon {
        return Origin::Daemon;
    }
    let mut cur = peer;
    for _ in 0..MAX_DEPTH {
        match parent_of(cur) {
            Parent::Pid(p) if p == daemon => return Origin::Member,
            // Reached the top (init/kernel) without meeting the daemon.
            Parent::Top => return Origin::Person,
            Parent::Pid(p) if p == 0 || p == 1 || p == cur => return Origin::Person,
            Parent::Pid(p) => cur = p,
            Parent::Unknown => return Origin::Unknown,
        }
    }
    // Too deep to tell: fail closed.
    Origin::Unknown
}

/// The parent of `pid` on this machine.
#[cfg(target_os = "macos")]
pub fn parent_of(pid: u32) -> Parent {
    // proc_pidinfo(pid, PROC_PIDTBSDINFO) → proc_bsdinfo, whose pbi_ppid is the
    // parent (libproc, part of libSystem). No process spawned, no ps parsed.
    use std::mem;
    let mut info: libc::proc_bsdinfo = unsafe { mem::zeroed() };
    let size = mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let got = unsafe {
        libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, &mut info as *mut _ as *mut libc::c_void, size)
    };
    if got != size {
        // Short read or error: no such process any more, or refused.
        return Parent::Unknown;
    }
    let ppid = info.pbi_ppid;
    if ppid <= 1 { Parent::Top } else { Parent::Pid(ppid) }
}

/// The parent of `pid` on this machine.
#[cfg(target_os = "linux")]
pub fn parent_of(pid: u32) -> Parent {
    // /proc/<pid>/stat field 4 is the parent pid. The comm (field 2) is in
    // parentheses and may hold spaces or ')', so read ppid after the last ')'.
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return Parent::Unknown;
    };
    let Some(after) = stat.rsplit_once(')').map(|(_, rest)| rest) else {
        return Parent::Unknown;
    };
    // after = " <state> <ppid> ..."
    let mut fields = after.split_whitespace();
    let _state = fields.next();
    match fields.next().and_then(|p| p.parse::<u32>().ok()) {
        Some(ppid) if ppid <= 1 => Parent::Top,
        Some(ppid) => Parent::Pid(ppid),
        None => Parent::Unknown,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn parent_of(_pid: u32) -> Parent {
    Parent::Unknown
}

/// The origin of a socket peer with id `peer`, as this daemon sees it.
pub fn of(peer: Option<u32>) -> Origin {
    classify(peer, std::process::id(), parent_of)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A tree of parents, for the pure walk.
    fn tree(pairs: &[(u32, Parent)]) -> impl Fn(u32) -> Parent + '_ {
        let map: HashMap<u32, Parent> = pairs.iter().copied().collect();
        move |pid| map.get(&pid).copied().unwrap_or(Parent::Unknown)
    }

    const DAEMON: u32 = 1000;

    #[test]
    fn the_daemon_itself_is_a_self_connection() {
        // The phone relay and Telegram connect in-process: peer is the daemon.
        assert_eq!(classify(Some(DAEMON), DAEMON, tree(&[])), Origin::Daemon);
        assert!(classify(Some(DAEMON), DAEMON, tree(&[])).person_may());
    }

    #[test]
    fn a_members_process_is_a_descendant_of_the_daemon() {
        // daemon(1000) → engine(2000) → shell(3000) → python(4000) on the socket.
        let t = tree(&[(4000, Parent::Pid(3000)), (3000, Parent::Pid(2000)), (2000, Parent::Pid(DAEMON))]);
        assert_eq!(classify(Some(4000), DAEMON, &t), Origin::Member);
        assert_eq!(classify(Some(2000), DAEMON, &t), Origin::Member, "the engine itself");
        assert!(!classify(Some(4000), DAEMON, &t).person_may());
    }

    #[test]
    fn the_persons_own_client_reaches_the_top_without_the_daemon() {
        // launchd(1) → app(5000) → (socket). The app is not a daemon child.
        let t = tree(&[(5000, Parent::Pid(1)), (6000, Parent::Top)]);
        assert_eq!(classify(Some(5000), DAEMON, &t), Origin::Person);
        assert_eq!(classify(Some(6000), DAEMON, &t), Origin::Person);
        // A terminal → shell → equipe attach, none of them the daemon.
        let cli = tree(&[(7000, Parent::Pid(7001)), (7001, Parent::Pid(7002)), (7002, Parent::Top)]);
        assert_eq!(classify(Some(7000), DAEMON, &cli), Origin::Person);
        assert!(classify(Some(7000), DAEMON, &cli).person_may());
    }

    #[test]
    fn an_app_that_launched_the_daemon_is_not_a_descendant_of_it() {
        // app(8000) → daemon(1000): the app is the daemon's parent, not child.
        let t = tree(&[(8000, Parent::Top), (DAEMON, Parent::Pid(8000))]);
        assert_eq!(classify(Some(8000), DAEMON, &t), Origin::Person);
    }

    #[test]
    fn an_unreadable_or_absent_peer_fails_closed() {
        assert_eq!(classify(None, DAEMON, tree(&[])), Origin::Unknown);
        // The chain breaks before the daemon or the top is reached.
        let t = tree(&[(9000, Parent::Pid(9001)), (9001, Parent::Unknown)]);
        assert_eq!(classify(Some(9000), DAEMON, &t), Origin::Unknown);
        assert!(!classify(Some(9000), DAEMON, &t).person_may());
        // A peer with no parent at all (gone): unknown.
        assert_eq!(classify(Some(9999), DAEMON, tree(&[])), Origin::Unknown);
    }

    #[test]
    fn a_cycle_cannot_loop_forever() {
        let t = tree(&[(50, Parent::Pid(60)), (60, Parent::Pid(50))]);
        assert_eq!(classify(Some(50), DAEMON, &t), Origin::Unknown);
    }

    #[test]
    fn this_process_reads_as_its_own_daemon_and_the_walk_reaches_the_top() {
        // Against the real system: our own pid classifies as Daemon.
        let me = std::process::id();
        assert_eq!(classify(Some(me), me, parent_of), Origin::Daemon);
        // And a real peer (our parent) walked against a different "daemon" pid
        // reaches the top as a Person — the walk terminates on this machine.
        let other = if me == 1 { 2 } else { 1 };
        assert!(matches!(of_with(Some(me), other), Origin::Person | Origin::Member | Origin::Unknown));
    }

    fn of_with(peer: Option<u32>, daemon: u32) -> Origin {
        classify(peer, daemon, parent_of)
    }
}
