//! equipe core: the board and agent logs, persisted in SQLite.
//!
//! Logs never live in UI memory. Writers append in batched transactions; the UI
//! asks only for the rows it is about to draw, addressed by a per-agent `seq`.

pub mod mock;
pub mod store;

pub use store::{LogKind, LogLine, Store, Ticket, TicketStatus};
