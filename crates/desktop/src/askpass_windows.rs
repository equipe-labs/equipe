//! Questions `ssh.exe` asks the person, carried back to the running app over
//! a Windows named pipe. The pipe is owned by this logon session's default
//! DACL and every request must also present the launch's one-time token.

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
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

/// Scratch files still need a folder; unlike Unix, the askpass channel itself
/// never lives here and leaves no socket file to sweep.
pub fn private_dir() -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("equipe-desktop");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn token() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{now:032x}{:08x}{next:016x}", std::process::id())
}

fn pipe_name(token: &str) -> PathBuf {
    PathBuf::from(format!(
        r"\\.\pipe\equipe-desktop-askpass-{}-{}",
        std::process::id(),
        &token[..16]
    ))
}

fn listener(name: &Path, first: bool) -> std::io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options.first_pipe_instance(first).create(name)
}

pub struct Server {
    pub socket: PathBuf,
    pub token: String,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Server {
    pub fn start(_dir: &Path) -> std::io::Result<(Self, mpsc::UnboundedReceiver<Prompt>)> {
        let token = token();
        let socket = pipe_name(&token);
        let first = listener(&socket, true)?;
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = mpsc::unbounded_channel();
        let (name, expected, flag) = (socket.clone(), token.clone(), cancelled.clone());
        tokio::spawn(async move {
            let mut next = Some(first);
            while let Some(server) = next.take() {
                if server.connect().await.is_err() {
                    break;
                }
                next = listener(&name, false).ok();
                let (tx, expected, flag) = (tx.clone(), expected.clone(), flag.clone());
                tokio::spawn(async move {
                    let _ = serve(server, tx, &expected, &flag).await;
                });
            }
        });
        Ok((
            Self {
                socket,
                token,
                cancelled,
            },
            rx,
        ))
    }

    pub fn take_cancelled(&self) -> bool {
        self.cancelled
            .swap(false, std::sync::atomic::Ordering::SeqCst)
    }

    pub fn forget_cancelled(&self) {
        self.cancelled
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

async fn serve(
    stream: NamedPipeServer,
    prompts: mpsc::UnboundedSender<Prompt>,
    expected: &str,
    cancelled: &std::sync::atomic::AtomicBool,
) -> std::io::Result<()> {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = tokio::io::BufReader::new(reader).lines();
    let (Some(token), Some(kind), Some(prompt)) = (
        lines.next_line().await?,
        lines.next_line().await?,
        lines.next_line().await?,
    ) else {
        return Ok(());
    };
    if token != expected {
        return Ok(());
    }
    let prompt = prompt.replace("\\n", "\n");
    let ask = classify(&prompt, &kind);
    let (reply, answer) = oneshot::channel();
    let declined = cancelled.load(std::sync::atomic::Ordering::SeqCst);
    let answer = if kind == "none" || declined || prompts.send(Prompt { ask, reply }).is_err() {
        Answer::Cancel
    } else {
        answer.await.unwrap_or(Answer::Cancel)
    };
    let line = match answer {
        Answer::Yes => "yes\n".to_owned(),
        Answer::Text(text) => format!("ok\n{text}\n"),
        Answer::Cancel => {
            if kind != "none" {
                cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            "cancel\n".to_owned()
        }
    };
    writer.write_all(line.as_bytes()).await?;
    writer.flush().await
}

pub fn helper() -> Option<i32> {
    let socket = std::env::var_os(ENV_SOCKET)?;
    let token = std::env::var(ENV_TOKEN).unwrap_or_default();
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let kind = std::env::var("SSH_ASKPASS_PROMPT").unwrap_or_default();
    Some(ask_app(
        Path::new(&socket),
        &token,
        &kind,
        &prompt,
        &mut std::io::stdout(),
    ))
}

fn ask_app(
    socket: &Path,
    token: &str,
    kind: &str,
    prompt: &str,
    out: &mut dyn std::io::Write,
) -> i32 {
    // A helper can arrive between one connected instance and the next one
    // being created. Retry the short busy/not-found window.
    let mut stream = None;
    for _ in 0..100 {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(socket)
        {
            Ok(file) => {
                stream = Some(file);
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    let Some(mut stream) = stream else { return 1 };
    let prompt = prompt.replace('\n', "\\n");
    if writeln!(stream, "{token}\n{kind}\n{prompt}").is_err() {
        return 1;
    }
    let mut reader = BufReader::new(stream);
    let mut verdict = String::new();
    if reader.read_line(&mut verdict).is_err() {
        return 1;
    }
    match verdict.trim_end() {
        "yes" => {
            let _ = writeln!(out, "yes");
            0
        }
        "ok" => {
            let mut secret = String::new();
            let _ = reader.read_line(&mut secret);
            let _ = writeln!(out, "{}", secret.trim_end_matches(['\r', '\n']));
            0
        }
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_named_pipes_and_unique() {
        let a = token();
        let b = token();
        assert_ne!(a, b);
        assert!(
            pipe_name(&a)
                .to_string_lossy()
                .starts_with(r"\\.\pipe\equipe-desktop-askpass-")
        );
    }

    #[test]
    fn tells_a_first_meeting_from_a_password() {
        let first = "The authenticity of host 'mini (10.0.0.2)' can't be established.\nED25519 key fingerprint is SHA256:abc123+/XYZ.\nAre you sure you want to continue connecting (yes/no/[fingerprint])? ";
        assert!(
            matches!(classify(first, ""), Ask::HostKey { host, fingerprint, .. } if host == "mini (10.0.0.2)" && fingerprint == "SHA256:abc123+/XYZ")
        );
        assert!(matches!(
            classify("me@mini's password: ", ""),
            Ask::Secret { .. }
        ));
    }

    #[tokio::test]
    async fn a_secret_round_trips_over_the_named_pipe() {
        let dir = private_dir().unwrap();
        let (server, mut prompts) = Server::start(&dir).unwrap();
        let socket = server.socket.clone();
        let token = server.token.clone();
        let helper = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            let code = ask_app(&socket, &token, "", "me@mini's password: ", &mut out);
            (code, String::from_utf8(out).unwrap())
        });
        let prompt = prompts.recv().await.unwrap();
        assert!(matches!(prompt.ask, Ask::Secret { .. }));
        prompt
            .reply
            .send(Answer::Text("pipe-secret".into()))
            .unwrap();
        let (code, out) = helper.await.unwrap();
        assert_eq!(code, 0);
        assert_eq!(out, "pipe-secret\n");
        assert!(!server.take_cancelled());
    }
}
