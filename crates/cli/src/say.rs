//! `equipe say`: the person's message to a squad's PM from a terminal or a
//! script, with files (`--attach`, repeated) and, for the platform's own
//! words, `--system` (recorded as the platform's, not the person's).

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use equipe_core::protocol::{Attachment, Command, Event};

use crate::t;

/// What `equipe say` sends.
pub struct Say {
    pub squad: String,
    pub text: String,
    pub attach: Vec<PathBuf>,
    pub system: bool,
}

/// The media type a file's extension says; none known: a plain file.
fn mime_of(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("txt" | "md") => "text/plain",
        Some("mp4" | "mov") => "video/mp4",
        _ => "application/octet-stream",
    }
}

/// The files as the wire carries them, or why not (the same caps the daemon holds).
pub fn attachments(paths: &[PathBuf]) -> Result<Vec<Attachment>, String> {
    if paths.len() > equipe_core::protocol::ATTACHMENTS_MAX {
        return Err(t!("say.too_many", max = equipe_core::protocol::ATTACHMENTS_MAX));
    }
    paths
        .iter()
        .map(|path| {
            let bytes = std::fs::read(path).map_err(|e| t!("say.unreadable", file = path.display(), why = e))?;
            if bytes.len() > equipe_core::protocol::ATTACHMENT_MAX_BYTES {
                return Err(t!("say.too_big", file = path.display(), mb = equipe_core::protocol::ATTACHMENT_MAX_BYTES >> 20));
            }
            Ok(Attachment {
                name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                mime: mime_of(path).to_owned(),
                data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            })
        })
        .collect()
}

pub async fn command(say: Say) -> Result<(), String> {
    if say.text.trim().is_empty() && say.attach.is_empty() {
        return Err(t!("say.needs_text").to_owned());
    }
    let attachments = attachments(&say.attach)?;
    let (mut client, hello) = crate::daemon::attach(&say.squad).await.map_err(|e| e.to_string())?;
    let pm = hello.snapshot.sessions.iter().find(|s| s.member.as_ref().is_some_and(|m| m.role == "pm")).map(|s| s.session).ok_or_else(|| t!("say.no_pm", squad = say.squad))?;
    client.send(Command::Say {
        squad: say.squad.clone(),
        session: pm,
        text: say.text,
        via: Default::default(),
        all: false,
        later: false,
        pasted: false,
        about: None,
        device: None,
        nonce: Some(rand_nonce()),
        attachments,
        system: say.system,
    });
    // Until the daemon has taken it (or refused it), so the line isn't lost on exit.
    let wait = async {
        while let Some(event) = client.next_event().await {
            match event {
                Event::UserMessage { .. } | Event::SystemMessage { .. } | Event::MessageQueued { .. } | Event::SteerQueued { .. } => return Ok(()),
                Event::Error { message, .. } => return Err(message),
                _ => {}
            }
        }
        Ok(())
    };
    match tokio::time::timeout(Duration::from_secs(15), wait).await {
        Ok(result) => result?,
        Err(_) => return Err(t!("say.no_answer").to_owned()),
    }
    println!("{}", t!("say.sent", squad = say.squad));
    Ok(())
}

fn rand_nonce() -> u64 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    (nanos ^ (u64::from(std::process::id()) << 32)) & ((1 << 52) - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_go_as_attachments_within_the_caps() {
        let dir = std::env::temp_dir().join(format!("equipe-say-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("shot.png");
        std::fs::write(&png, b"png").unwrap();
        let got = attachments(std::slice::from_ref(&png)).unwrap();
        assert_eq!((got[0].name.as_str(), got[0].mime.as_str()), ("shot.png", "image/png"));
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&got[0].data_base64).unwrap(), b"png");
        assert!(attachments(&vec![png; 6]).is_err(), "at most five");
        assert!(attachments(&[dir.join("gone.png")]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
