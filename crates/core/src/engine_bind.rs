//! Which member a running engine process works for, readable without the
//! daemon's loop.
//!
//! A connection from a member's process (equipe's board tools, an outside
//! MCP server's proxy) is bound to the member whose turn runs the engine it
//! is under ([`crate::origin::engine`]). The turn's own task notes the engine
//! the moment the engine says its process ([`bind`]) and drops the note when
//! the turn ends (the [`Bound`] guard), so the gateway can tell who a
//! connection is for even while the loop is busy with something slow (a
//! merge, a worktree): before, the answer went through the loop, and a
//! connection that came during a long step was refused as "not from a turn
//! equipe runs" — a member's fresh conversation lost its outside servers for
//! the whole turn.
//!
//! Process ids are reused by the system only after the process is gone, and
//! the note goes with the turn, so an entry never outlives its engine.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use equipe_protocol::MemberRef;

static ENGINES: LazyLock<RwLock<HashMap<u32, MemberRef>>> = LazyLock::new(Default::default);

/// The note that engine `pid` works for a member; removed when dropped.
#[derive(Debug)]
pub struct Bound {
    pid: u32,
}

impl Drop for Bound {
    fn drop(&mut self) {
        if let Ok(mut all) = ENGINES.write() {
            all.remove(&self.pid);
        }
    }
}

/// Note that engine process `pid` runs a turn of `member`, until the returned guard drops.
pub fn bind(pid: u32, member: MemberRef) -> Bound {
    if let Ok(mut all) = ENGINES.write() {
        all.insert(pid, member);
    }
    Bound { pid }
}

/// The member whose turn runs as engine process `pid`, if one does.
pub fn member_of(pid: u32) -> Option<MemberRef> {
    ENGINES.read().ok()?.get(&pid).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dana() -> MemberRef {
        MemberRef { squad: "acme".into(), member: "dana".into() }
    }

    #[test]
    fn an_engine_is_bound_while_its_turn_runs_and_not_after() {
        // Process ids far above any real one: tests in this process never meet.
        let pid = 4_000_000_001;
        assert_eq!(member_of(pid), None);
        let bound = bind(pid, dana());
        assert_eq!(member_of(pid), Some(dana()));
        drop(bound);
        assert_eq!(member_of(pid), None, "the turn ended: nothing is bound to it any more");
    }

    #[test]
    fn a_fresh_conversation_binds_its_new_engine_at_once() {
        // The old conversation's engine ends; the new one's starts under another process.
        let (old, new) = (4_000_000_002, 4_000_000_003);
        let first = bind(old, dana());
        drop(first);
        let _second = bind(new, dana());
        assert_eq!(member_of(new), Some(dana()));
        assert_eq!(member_of(old), None);
    }
}
