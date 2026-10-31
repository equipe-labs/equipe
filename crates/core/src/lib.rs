//! equipe core: the squad's domain, shared by every front end (CLI, desktop).
//!
//! - `runtime`: the brain. Runs sessions on engines, driven by protocol
//!   `Command`s, reporting `Event`s. Screens talk to nothing else.
//! - `squads`: which projects each squad works on.
//! - `board`: equipe's rules for tickets, and the tools members use on them.
//! - `roles`: the built-in squad roles.
//! - `worktree`: isolated branches for agents that edit code.
//! - `store`: the board and agent logs, persisted in SQLite. Logs never live in
//!   UI memory. Writers append in batched transactions; a UI asks only for the
//!   rows it is about to draw, addressed by a per-agent `seq`.

pub mod accept;
pub mod awake;
pub mod board;
pub mod browser;
pub mod design;
pub mod group;
pub mod heartbeat;
pub mod desktop;
pub mod desktop_gate;
pub mod disk;
pub mod engines;
pub mod eta;
pub mod gateway;
pub mod i18n;
pub mod journal;
pub mod language;
pub mod mcp;
pub mod permissions;
pub mod mock;
pub mod pairing;
pub mod roles;
pub mod routines;
pub mod secrets;
pub mod runtime;
pub mod squad_home;
pub mod squad_memory;
pub mod squads;
pub mod switching;
pub mod steers;
pub mod store;
pub mod team;
pub mod vault;
pub mod visuals;
pub mod watchdog;
pub mod worktree;

/// A line for the daemon's log — its standard error, `~/.equipe/daemon.log`.
/// Unlike `eprintln!`, never a panic when it can't be written: a full disk is
/// exactly when it can't, and a panic there stops the daemon.
#[macro_export]
macro_rules! daemon_log {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

pub use equipe_protocol as protocol;

pub use roles::{ROLES, Role};
pub use store::{LogKind, LogLine, Store, Ticket, TicketStatus};
