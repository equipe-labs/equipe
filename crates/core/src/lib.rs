//! equipe core: the squad's domain, shared by every front end (CLI, desktop).
//!
//! - `roles`: the built-in squad roles.
//! - `store`: the board and agent logs, persisted in SQLite. Logs never live in
//!   UI memory. Writers append in batched transactions; a UI asks only for the
//!   rows it is about to draw, addressed by a per-agent `seq`.

pub mod mock;
pub mod roles;
pub mod store;

pub use roles::{ROLES, Role};
pub use store::{LogKind, LogLine, Store, Ticket, TicketStatus};
