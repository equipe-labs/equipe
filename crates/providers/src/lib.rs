//! How equipe authenticates and talks to model backends.
//!
//! Wire formats and protocol rules live in pure functions so they are testable
//! without a network or a login; the async functions only add I/O around them.

pub mod chat_wire;
pub mod claude_code;
pub mod codex_app;
pub mod credentials;
#[cfg(windows)]
pub mod windows_credentials;
pub mod error;
pub mod openai;
pub mod platform;
pub mod pkce;
pub mod relay;
pub mod tools_path;
pub mod turn;
mod child_job;

pub use credentials::{AuthMethod, CredentialStore, Credentials};
pub use error::{Error, Result};
