//! equipe core: the squad's domain, shared by every front end (CLI, desktop).
//!
//! - `runtime`: the brain. Runs sessions on engines, driven by protocol
//!   `Command`s, reporting `Event`s. Screens talk to nothing else.
//! - `roles`: the built-in squad roles.
//! - `worktree`: isolated branches for agents that edit code.
//! - `store`: the board and agent logs, persisted in SQLite. Logs never live in
//!   UI memory. Writers append in batched transactions; a UI asks only for the
//!   rows it is about to draw, addressed by a per-agent `seq`.

pub mod gateway;
pub mod journal;
pub mod mock;
pub mod roles;
pub mod runtime;
pub mod store;
pub mod worktree;

pub use equipe_protocol as protocol;

pub use roles::{ROLES, Role};
pub use store::{LogKind, LogLine, Store, Ticket, TicketStatus};
