//! OpenAI: ChatGPT (Codex) subscription OAuth and the Codex backend.
//!
//! Mechanism is the one OpenAI's own open-source Codex CLI uses: the public Codex
//! OAuth client + PKCE, then the ChatGPT backend with the resulting token. equipe
//! identifies itself honestly as a third-party harness; it never impersonates the
//! official CLI.

pub mod codex;
pub mod login;
pub mod oauth;
pub mod responses;
pub mod sse;

/// Base URL of the ChatGPT (Codex) backend.
pub const CODEX_BACKEND_URL: &str = "https://chatgpt.com/backend-api/codex";
