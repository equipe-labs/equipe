//! Windows placeholder until the named-pipe askpass work lands.

use std::path::{Path, PathBuf};
use tokio::sync::{mpsc, oneshot};

pub const ENV_SOCKET: &str = "EQUIPE_ASKPASS";
pub const ENV_TOKEN: &str = "EQUIPE_ASKPASS_TOKEN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    HostKey { host: String, fingerprint: String, text: String },
    Confirm { text: String },
    Secret { label: String },
}

pub struct Prompt {
    pub ask: Ask,
    pub reply: oneshot::Sender<Answer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Yes,
    Text(String),
    Cancel,
}

pub fn classify(prompt: &str, kind: &str) -> Ask {
    let lower = prompt.to_lowercase();
    if lower.contains("continue connecting") || lower.contains("authenticity of host") {
        let host = prompt
            .split("authenticity of host '")
            .nth(1)
            .and_then(|rest| rest.split('\'').next())
            .unwrap_or_default()
            .to_owned();
        let fingerprint = prompt
            .split_whitespace()
            .find(|word| word.starts_with("SHA256:") || word.starts_with("MD5:"))
            .unwrap_or_default()
            .trim_end_matches(['.', ','])
            .to_owned();
        Ask::HostKey { host, fingerprint, text: prompt.trim().to_owned() }
    } else if kind == "confirm" || lower.contains("(yes/no") {
        Ask::Confirm { text: prompt.trim().to_owned() }
    } else {
        Ask::Secret { label: prompt.trim().to_owned() }
    }
}

pub fn private_dir() -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("equipe-desktop");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub struct Server {
    pub socket: PathBuf,
    pub token: String,
}

impl Server {
    pub fn start(_dir: &Path) -> std::io::Result<(Self, mpsc::UnboundedReceiver<Prompt>)> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "SSH askpass unsupported on Windows yet",
        ))
    }

    pub fn take_cancelled(&self) -> bool {
        false
    }

    pub fn forget_cancelled(&self) {}
}

pub fn helper() -> Option<i32> {
    None
}
