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

pub mod board;
pub mod browser;
pub mod gateway;
pub mod journal;
pub mod language;
pub mod mcp;
pub mod permissions;
pub mod mock;
pub mod roles;
pub mod runtime;
pub mod squad_home;
pub mod squad_memory;
pub mod squads;
pub mod store;
pub mod worktree;

pub use equipe_protocol as protocol;

pub use roles::{ROLES, Role};
pub use store::{LogKind, LogLine, Store, Ticket, TicketStatus};
