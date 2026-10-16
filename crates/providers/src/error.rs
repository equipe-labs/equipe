use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// The backend refused the credential. Re-authenticating is the fix.
    #[error("backend rejected the token (HTTP {status}). Re-run `equipe login`. {detail}")]
    Unauthorized { status: u16, detail: String },

    #[error("rate limited or out of quota (HTTP 429). {0}")]
    RateLimited(String),

    #[error("{what} failed (HTTP {status}): {detail}")]
    Status {
        what: &'static str,
        status: u16,
        detail: String,
    },

    /// The backend answered, but not in a shape we understand.
    #[error("{0}")]
    Protocol(String),

    /// The backend reported an error inside an otherwise healthy response.
    #[error("{message}")]
    Backend {
        message: String,
        code: Option<String>,
    },

    /// The person's own `codex` CLI has no sign-in. equipe never signs it in
    /// itself: `codex login` does, in the person's hands.
    #[error("the codex CLI isn't signed in. Run `equipe login codex` — it runs `codex login`.")]
    CodexSignedOut,

    #[error("login timed out. Retry `equipe login`.")]
    LoginTimeout,

    #[error("{0}")]
    Login(String),
}

impl Error {
    /// Map a non-success HTTP response to the error a user can act on.
    pub(crate) fn from_status(what: &'static str, status: u16, body: &str) -> Self {
        let detail: String = body.chars().take(300).collect();
        match status {
            401 | 403 => Error::Unauthorized { status, detail },
            429 => Error::RateLimited(detail),
            _ => Error::Status {
                what,
                status,
                detail: if detail.is_empty() {
                    "no response body".into()
                } else {
                    detail
                },
            },
        }
    }
}
