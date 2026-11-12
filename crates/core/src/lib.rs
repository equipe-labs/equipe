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
pub mod autopilot;
pub mod awake;
pub mod bless;
pub mod board;
pub mod briefs;
pub mod by_words;
pub mod call;
pub mod checks;
pub mod command_watch;
pub mod conversation_title;
pub mod controls;
pub mod design_qa;
pub mod taste;
pub mod scratch;
pub mod browser;
pub mod design;
pub mod group;
pub mod guard;
pub mod heartbeat;
pub mod helpers;
pub mod desktop;
pub mod desktop_gate;
pub mod disk;
pub mod leftovers;
pub mod engine_auth;
pub mod engines;
pub mod eta;
pub mod gateway;
pub mod i18n;
pub mod journal;
pub mod lanes;
pub mod logins;
pub mod kept;
pub mod language;
pub mod looks;
pub mod mcp;
pub mod metrics;
pub mod permissions;
pub mod phone;
pub mod plan;
pub mod plan_use;
pub mod quality;
pub mod mock;
pub mod image_repeat;
pub mod models;
pub mod model_providers;
pub mod nemotron;
pub mod origin;
pub mod pairing;
pub mod retro;
pub mod roles;
pub mod routines;
pub mod secrets;
pub mod side_chat;
pub mod signal_sources;
pub mod signals;
pub mod skills;
pub mod runtime;
pub mod squad_home;
pub mod squad_memory;
pub mod squads;
pub mod switching;
pub mod steers;
pub mod store;
pub mod subagent_watch;
pub mod team;
pub mod telemetry;
pub mod tiers;
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
