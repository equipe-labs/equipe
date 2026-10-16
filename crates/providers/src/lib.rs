//! How equipe authenticates and talks to model backends.
//!
//! Wire formats and protocol rules live in pure functions so they are testable
//! without a network or a login; the async functions only add I/O around them.

pub mod claude_code;
pub mod codex_app;
pub mod credentials;
pub mod error;
pub mod openai;
pub mod pkce;
pub mod turn;

pub use credentials::{AuthMethod, CredentialStore, Credentials};
pub use error::{Error, Result};
