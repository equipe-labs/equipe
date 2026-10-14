//! Questions `ssh` asks the person — a password, a code from an authenticator,
//! a key's passphrase, "do you trust this host?" — answered in a box in the
//! window instead of a terminal the app doesn't have.
//!
//! `ssh` runs the program named by `SSH_ASKPASS` whenever it has a question
//! (forced by `SSH_ASKPASS_REQUIRE=force`). That program is this app's own
//! binary, started with `EQUIPE_ASKPASS=<socket>` in its environment: [`helper`]
//! hands the question to the running app over a Unix socket (0600 in a 0700
//! folder, and a one-time token on top), waits for the answer and prints it for
//! `ssh`. Nothing is written to disk and nothing outlives the answer: the
//! password goes from the box to `ssh`'s pipe and is dropped.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
use std::os::unix::net::UnixStream as StdStream;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot};

/// Set in the environment of the `ssh` the app starts: the app's socket.
pub const ENV_SOCKET: &str = "EQUIPE_ASKPASS";
/// And the token the helper must show to be answered.
pub const ENV_TOKEN: &str = "EQUIPE_ASKPASS_TOKEN";

/// What kind of question it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// A first meeting with a host: its name, the key's fingerprint, and the
    /// whole question as `ssh` put it.
    HostKey { host: String, fingerprint: String, text: String },
    /// Some other yes/no.
    Confirm { text: String },
    /// A secret to type: password, code, passphrase. `label` is `ssh`'s prompt.
    Secret { label: String },
}

/// A question waiting for the person.
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

/// Tell what `ssh` is asking. `kind` is `$SSH_ASKPASS_PROMPT` (`confirm`,
/// `none`, or empty for a question to type an answer to).
pub fn classify(prompt: &str, kind: &str) -> Ask {
    let lower = prompt.to_lowercase();
    if lower.contains("continue connecting") || lower.contains("authenticity of host") {
        let mut host = String::new();
        if let Some(rest) = prompt.split("authenticity of host '").nth(1) {
            host = rest.split('\'').next().unwrap_or("").to_owned();
        }
        let fingerprint = prompt
            .split_whitespace()
            .find(|w| w.trim_matches(|c: char| c == '.' || c == ',').starts_with("SHA256:") || w.trim_matches('.').starts_with("MD5:"))
            .map(|w| w.trim_end_matches(['.', ',']).to_owned())
            .unwrap_or_default();
        return Ask::HostKey { host, fingerprint, text: prompt.trim().to_owned() };
    }
    if kind == "confirm" || lower.contains("(yes/no") {
        return Ask::Confirm { text: prompt.trim().to_owned() };
    }
    Ask::Secret { label: prompt.trim().to_owned() }
}

// ── the app's side ──

/// A folder only this user can use, short enough for socket paths (macOS: ≤ 104
/// bytes): `/tmp/equipe-desktop-<uid>`. Refuses a folder someone else made.
pub fn private_dir() -> std::io::Result<PathBuf> {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let dir = PathBuf::from(format!("/tmp/equipe-desktop-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = std::fs::symlink_metadata(&dir)?;
    if !meta.is_dir() || meta.uid() != uid {
        return Err(std::io::Error::other(format!("{} isn't ours", dir.display())));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

/// Remove sockets left by instances of the app that are gone.
fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid = name.to_str().and_then(|n| n.strip_prefix("askpass-")).and_then(|n| n.strip_suffix(".sock")).and_then(|n| n.parse::<i32>().ok());
        // SAFETY: signal 0 only checks that the process exists.
        if let Some(pid) = pid
            && pid != std::process::id() as i32
            && unsafe { libc::kill(pid, 0) } != 0
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn token() -> String {
    use std::io::Read as _;
    let mut bytes = [0u8; 16];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).is_err() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        bytes = (nanos ^ u128::from(std::process::id())).to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The listening end, inside the app. Dropping it removes the socket.
pub struct Server {
    pub socket: PathBuf,
    pub token: String,
    /// Set when the person closed a box without answering, since the last [`Server::take_cancelled`].
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Server {
    /// Listen at `dir/askpass-<pid>.sock`; questions arrive on the receiver.
    pub fn start(dir: &Path) -> std::io::Result<(Self, mpsc::UnboundedReceiver<Prompt>)> {
        sweep(dir);
        let socket = dir.join(format!("askpass-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        let token = token();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = mpsc::unbounded_channel();
        let (expected, flag) = (token.clone(), cancelled.clone());
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (tx, expected, flag) = (tx.clone(), expected.clone(), flag.clone());
                tokio::spawn(async move {
                    let _ = serve(stream, tx, &expected, &flag).await;
                });
            }
        });
        Ok((Self { socket, token, cancelled }, rx))
    }

    /// Whether a box was closed unanswered since the last call.
    pub fn take_cancelled(&self) -> bool {
        self.cancelled.swap(false, std::sync::atomic::Ordering::SeqCst)
    }

    pub fn forget_cancelled(&self) {
        self.cancelled.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}

async fn serve(
    stream: tokio::net::UnixStream,
    prompts: mpsc::UnboundedSender<Prompt>,
    expected: &str,
    cancelled: &std::sync::atomic::AtomicBool,
) -> std::io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = tokio::io::BufReader::new(reader).lines();
    let (Some(token), Some(kind), Some(prompt)) = (lines.next_line().await?, lines.next_line().await?, lines.next_line().await?) else {
        return Ok(());
    };
    if token != expected {
        return Ok(());
    }
    // The prompt travelled with its newlines turned into `\n` escapes.
    let prompt = prompt.replace("\\n", "\n");
    let ask = classify(&prompt, &kind);
    let (reply, answer) = oneshot::channel();
    // Once the person has closed a box, `ssh` asking again in the same try
    // (it retries a password) is not asked again.
    let declined = cancelled.load(std::sync::atomic::Ordering::SeqCst);
    let answer = if kind == "none" || declined || prompts.send(Prompt { ask, reply }).is_err() { Answer::Cancel } else { answer.await.unwrap_or(Answer::Cancel) };
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

// ── the helper's side (what `ssh` runs) ──

/// `Some(code)` when this process was started by `ssh` to ask a question: it has
/// asked the app and printed the answer; the caller exits with `code`.
pub fn helper() -> Option<i32> {
    let socket = std::env::var_os(ENV_SOCKET)?;
    let token = std::env::var(ENV_TOKEN).unwrap_or_default();
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let kind = std::env::var("SSH_ASKPASS_PROMPT").unwrap_or_default();
    Some(ask_app(Path::new(&socket), &token, &kind, &prompt, &mut std::io::stdout()))
}

fn ask_app(socket: &Path, token: &str, kind: &str, prompt: &str, out: &mut dyn std::io::Write) -> i32 {
    let Ok(mut stream) = StdStream::connect(socket) else { return 1 };
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
            let _ = writeln!(out, "{}", secret.trim_end_matches('\n'));
            0
        }
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_a_first_meeting_from_a_password() {
        let first = "The authenticity of host 'mini (10.0.0.2)' can't be established.\nED25519 key fingerprint is SHA256:abc123+/XYZ.\nThis key is not known by any other names.\nAre you sure you want to continue connecting (yes/no/[fingerprint])? ";
        match classify(first, "") {
            Ask::HostKey { host, fingerprint, .. } => {
                assert_eq!(host, "mini (10.0.0.2)");
                assert_eq!(fingerprint, "SHA256:abc123+/XYZ");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(classify("me@mini's password: ", ""), Ask::Secret { label } if label == "me@mini's password:"));
        assert!(matches!(classify("Enter passphrase for key '/Users/me/.ssh/id_ed25519': ", ""), Ask::Secret { .. }));
        assert!(matches!(classify("(me@mini) Verification code: ", ""), Ask::Secret { .. }));
        assert!(matches!(classify("Allow use of key id_ed25519?\nKey fingerprint SHA256:x.", "confirm"), Ask::Confirm { .. }));
    }

    fn socket() -> PathBuf {
        PathBuf::from(format!("/tmp/eqap-{}-{}.sock", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()))
    }

    /// The helper's whole round trip against the real server, answered like the window would.
    async fn round_trip(answer: Answer, token_ok: bool, kind: &'static str) -> (i32, String, bool) {
        let dir = PathBuf::from("/tmp");
        let (server, mut prompts) = {
            // Server::start names the socket after the pid; tests share a pid, so use a fresh folder.
            let dir = dir.join(format!("eqapd-{}", socket().file_name().unwrap().to_string_lossy()));
            std::fs::create_dir_all(&dir).unwrap();
            Server::start(&dir).unwrap()
        };
        let path = server.socket.clone();
        let token = if token_ok { server.token.clone() } else { "wrong".into() };
        let helper = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            let code = ask_app(&path, &token, kind, "me@mini's password: ", &mut out);
            (code, String::from_utf8(out).unwrap())
        });
        if token_ok {
            let prompt = prompts.recv().await.expect("a question reaches the window");
            let _ = prompt.reply.send(answer);
        }
        let (code, out) = helper.await.unwrap();
        let cancelled = server.take_cancelled();
        let _ = std::fs::remove_dir_all(server.socket.parent().unwrap());
        (code, out, cancelled)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_answer_comes_back_to_ssh_on_stdout() {
        assert_eq!(round_trip(Answer::Text("s3cret".into()), true, "").await, (0, "s3cret\n".into(), false));
        assert_eq!(round_trip(Answer::Yes, true, "confirm").await, (0, "yes\n".into(), false));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancelling_fails_the_askpass_and_is_remembered() {
        assert_eq!(round_trip(Answer::Cancel, true, "").await, (1, String::new(), true));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_caller_without_the_token_is_not_asked_about() {
        let (code, out, cancelled) = round_trip(Answer::Yes, false, "").await;
        assert_eq!((code, out.as_str(), cancelled), (1, "", false));
    }
}
