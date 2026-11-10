//! The helper's socket: where the daemon brings members' desktop calls.
//!
//! The helper is the only process the person grants Screen Recording and
//! Accessibility to; it runs as its own LaunchAgent in their GUI session and
//! listens at `~/.equipe/desktop/helper.sock`. One request per connection, one
//! JSON line each way. A request carries a token kept beside the socket, which
//! members' sandboxed commands can't read, so only the daemon gets through.
//! Requests are served one at a time: two members never move the pointer at once.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::io::BufRead as _;
#[cfg(any(unix, windows))]
use std::time::Duration;

#[cfg(any(unix, windows))]
use base64::Engine as _;
#[cfg(any(unix, windows))]
use equipe_protocol::rpc::Image;
use equipe_protocol::rpc::ToolResult;
#[cfg(any(unix, windows))]
use serde_json::json;
use serde_json::Value;
#[cfg(any(unix, windows))]
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

use crate::controller::{Controller, Desktop};
#[cfg(any(unix, windows))]
use crate::controller::{Reply, SETUP};

/// The helper's folder in equipe's home: socket, token, signing key, bundles.
pub fn dir(home: &Path) -> PathBuf {
    home.join("desktop")
}

pub fn socket_path(home: &Path) -> PathBuf {
    dir(home).join("helper.sock")
}

/// A stable, per-home pipe name. The token still authenticates every request;
/// rejecting remote clients keeps the pipe local to this machine.
#[cfg(windows)]
fn pipe_name(home: &Path) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in home.as_os_str().to_string_lossy().to_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!(r"\\.\pipe\equipe-desktop-{hash:016x}")
}

fn token_path(home: &Path) -> PathBuf {
    dir(home).join("token")
}

pub fn log_path(home: &Path) -> PathBuf {
    dir(home).join("helper.log")
}

/// The folder, only the person's to open.
pub fn ensure_dir(home: &Path) -> io::Result<PathBuf> {
    let dir = dir(home);
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

/// Random hex from the system's generator.
pub fn random_hex(bytes: usize) -> io::Result<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(io::Error::other)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// The token requests must carry, made on first use.
pub fn ensure_token(home: &Path) -> io::Result<String> {
    if let Some(token) = read_token(home) {
        return Ok(token);
    }
    ensure_dir(home)?;
    let token = random_hex(32)?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(token_path(home))?;
    file.write_all(token.as_bytes())?;
    Ok(token)
}

fn read_token(home: &Path) -> Option<String> {
    std::fs::read_to_string(token_path(home)).ok().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
}

/// What a member gets when the helper isn't there to answer.
#[cfg(unix)]
pub const NOT_RUNNING: &str =
    "The desktop helper isn't running on this Mac. Ask the person to run `equipe desktop setup` on the Mac (while logged in at its screen).";
#[cfg(windows)]
pub const NOT_RUNNING: &str =
    "The desktop helper isn't running on this Windows PC. Ask the person to run `equipe desktop setup` while logged in at its screen.";

/// Internal requests `equipe desktop setup` makes: the helper's own state,
/// and asking macOS for the permissions (which puts the helper in both lists).
pub const STATUS: &str = "_status";
pub const REQUEST: &str = "_request";

/// Requests the daemon makes before a member's input, for the send gate: what
/// the input would act on (`arguments`: `{ tool, arguments }`), and the screen.
pub const SCENE: &str = "_scene";
pub const CAPTURE: &str = "_capture";

/// Serve on the socket until the process ends, one request at a time.
#[cfg(unix)]
pub fn serve<D: Desktop>(home: &Path, controller: &mut Controller<D>, status: impl Fn(&D) -> Value, request: impl Fn(&D)) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    ensure_dir(home)?;
    let socket = socket_path(home);
    let _ = std::fs::remove_file(&socket);
    let listener = std::os::unix::net::UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    eprintln!("equipe desktop helper: listening at {}", socket.display());
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let mut line = String::new();
        if io::BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        let result = answer(home, controller, &line, &status, &request);
        let mut out = &stream;
        let _ = out.write_all(format!("{}\n", serde_json::to_string(&result).unwrap_or_default()).as_bytes());
    }
    Ok(())
}

#[cfg(windows)]
pub fn serve<D: Desktop>(home: &Path, controller: &mut Controller<D>, status: impl Fn(&D) -> Value, request: impl Fn(&D)) -> io::Result<()> {
    ensure_dir(home)?;
    ensure_token(home)?;
    let name = pipe_name(home);
    let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?;
    runtime.block_on(async move {
        use tokio::net::windows::named_pipe::ServerOptions;

        let mut first = true;
        loop {
            let mut options = ServerOptions::new();
            options.first_pipe_instance(first).reject_remote_clients(true);
            let server = options.create(&name)?;
            first = false;
            server.connect().await?;
            let mut line = String::new();
            let mut stream = tokio::io::BufReader::new(server);
            if tokio::time::timeout(Duration::from_secs(10), stream.read_line(&mut line)).await.is_err() {
                continue;
            }
            let result = answer(home, controller, &line, &status, &request);
            let output = format!("{}\n", serde_json::to_string(&result).unwrap_or_default());
            let mut server = stream.into_inner();
            let _ = server.write_all(output.as_bytes()).await;
            let _ = server.flush().await;
        }
    })
}

#[cfg(any(unix, windows))]
fn answer<D: Desktop>(
    home: &Path,
    controller: &mut Controller<D>,
    line: &str,
    status: &impl Fn(&D) -> Value,
    request: &impl Fn(&D),
) -> ToolResult {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return ToolResult::error("unreadable request");
    };
    let token = message["token"].as_str().unwrap_or_default();
    if read_token(home).is_none_or(|t| !same(&t, token)) {
        return ToolResult::error("wrong token");
    }
    let member = message["member"].as_str().unwrap_or("someone");
    let tool = message["tool"].as_str().unwrap_or_default();
    match tool {
        STATUS => ToolResult::ok(status(&controller.desktop).to_string()),
        REQUEST => {
            request(&controller.desktop);
            ToolResult::ok(status(&controller.desktop).to_string())
        }
        SCENE => {
            let arguments = &message["arguments"];
            let inner = arguments["tool"].as_str().unwrap_or_default();
            match controller.scene(member, inner, &arguments["arguments"]) {
                Ok(scene) => ToolResult::ok(scene.to_string()),
                Err(refused) => ToolResult::error(refused.text),
            }
        }
        CAPTURE => reply(controller.capture()),
        _ => {
            let reply = controller.call(member, tool, &message["arguments"]);
            eprintln!("equipe desktop helper: {member} {tool}{}", if reply.is_error { " (refused)" } else { "" });
            self::reply(reply)
        }
    }
}

#[cfg(any(unix, windows))]
fn reply(reply: Reply) -> ToolResult {
    ToolResult {
        text: reply.text,
        is_error: reply.is_error,
        image: reply.jpeg.map(|bytes| Image { mime: "image/jpeg".into(), data: base64::engine::general_purpose::STANDARD.encode(bytes) }),
    }
}

/// Compare without stopping at the first difference.
#[cfg(any(unix, windows))]
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Send one call to the helper and wait for its answer — what the daemon does
/// for each desktop call a member makes.
#[cfg(unix)]
pub async fn call(home: &Path, member: &str, tool: &str, arguments: &Value) -> ToolResult {
    let Some(token) = read_token(home) else {
        return ToolResult::error(NOT_RUNNING);
    };
    let request = json!({ "token": token, "member": member, "tool": tool, "arguments": arguments });
    let exchange = async {
        let mut stream = tokio::net::UnixStream::connect(socket_path(home)).await?;
        stream.write_all(format!("{request}\n").as_bytes()).await?;
        let mut line = String::new();
        tokio::io::BufReader::new(stream).read_line(&mut line).await?;
        serde_json::from_str::<ToolResult>(&line).map_err(io::Error::other)
    };
    match tokio::time::timeout(Duration::from_secs(90), exchange).await {
        Ok(Ok(result)) => result,
        Ok(Err(err)) if matches!(err.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => ToolResult::error(NOT_RUNNING),
        Ok(Err(err)) => ToolResult::error(format!("The desktop helper didn't answer ({err}). If it keeps happening, ask the person to run `{SETUP}`.")),
        Err(_) => ToolResult::error("The desktop helper took longer than 90 s to answer; nothing more was done."),
    }
}

#[cfg(windows)]
pub async fn call(home: &Path, member: &str, tool: &str, arguments: &Value) -> ToolResult {
    use tokio::net::windows::named_pipe::ClientOptions;

    let Some(token) = read_token(home) else {
        return ToolResult::error(NOT_RUNNING);
    };
    let request = json!({ "token": token, "member": member, "tool": tool, "arguments": arguments });
    let exchange = async {
        let mut stream = ClientOptions::new().open(pipe_name(home))?;
        stream.write_all(format!("{request}\n").as_bytes()).await?;
        let mut line = String::new();
        tokio::io::BufReader::new(stream).read_line(&mut line).await?;
        serde_json::from_str::<ToolResult>(&line).map_err(io::Error::other)
    };
    match tokio::time::timeout(Duration::from_secs(90), exchange).await {
        Ok(Ok(result)) => result,
        Ok(Err(err)) if matches!(err.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => ToolResult::error(NOT_RUNNING),
        Ok(Err(err)) => ToolResult::error(format!("The desktop helper didn't answer ({err}). If it keeps happening, ask the person to run `{SETUP}`.")),
        Err(_) => ToolResult::error("The desktop helper took longer than 90 s to answer; nothing more was done."),
    }
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;

    #[test]
    fn tokens_compare_whole() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd") && !same("abc", "ab"));
    }

    #[tokio::test]
    async fn without_a_helper_a_member_is_told_how_to_set_one_up() {
        let home = std::env::temp_dir().join(format!("eq-dh-{}", std::process::id()));
        let result = call(&home, "frontend", "screenshot", &json!({})).await;
        assert!(result.is_error && result.text.contains(SETUP), "{}", result.text);
        ensure_token(&home).unwrap();
        let result = call(&home, "frontend", "screenshot", &json!({})).await;
        assert_eq!(result.text, NOT_RUNNING, "a token but no socket");
        std::fs::remove_dir_all(&home).unwrap();
    }
}
