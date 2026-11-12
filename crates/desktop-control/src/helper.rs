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
use crate::engine::Engine;
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
/// Asks macOS for Accessibility only (its system prompt, once): what the
/// daemon does by itself when a member needs the desktop. `REQUEST` asks for
/// everything, Screen Recording included, and is the person's "Allow…".
pub const REQUEST_ACCESSIBILITY: &str = "_request_accessibility";

/// What a permission request asks macOS for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    Everything,
    Accessibility,
}

/// Requests the daemon makes before a member's input, for the send gate: what
/// the input would act on (`arguments`: `{ tool, arguments }`), and the screen.
pub const SCENE: &str = "_scene";
pub const CAPTURE: &str = "_capture";

/// Serve on the socket until the process ends, one request at a time.
#[cfg(unix)]
pub fn serve<D: Desktop>(home: &Path, controller: &mut Controller<D>, status: impl Fn(&D) -> Value, request: impl Fn(&D, Ask)) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    ensure_dir(home)?;
    let socket = socket_path(home);
    let _ = std::fs::remove_file(&socket);
    let listener = std::os::unix::net::UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    eprintln!("equipe desktop helper: listening at {}", socket.display());
    let engine = engine(home);
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let mut line = String::new();
        if io::BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        // The accessibility tools run on a thread of their own: they take
        // no screen, so one member's slow snapshot holds nobody else up.
        if let Some(engine) = engine.as_ref().filter(|_| for_engine(home, &line)) {
            let engine = std::sync::Arc::clone(engine);
            std::thread::spawn(move || {
                let result = engine_answer(&engine, &line);
                let mut out = &stream;
                let _ = out.write_all(format!("{}\n", serde_json::to_string(&result).unwrap_or_default()).as_bytes());
            });
            continue;
        }
        let result = answer(home, controller, engine.as_deref(), &line, &status, &request);
        let mut out = &stream;
        let _ = out.write_all(format!("{}\n", serde_json::to_string(&result).unwrap_or_default()).as_bytes());
    }
    Ok(())
}

/// The accessibility engine, when this machine has a release pinned.
#[cfg(any(unix, windows))]
fn engine(home: &Path) -> Option<std::sync::Arc<Engine>> {
    crate::engine::Cli::installed(home).map(|cli| std::sync::Arc::new(Engine::new(Box::new(cli))))
}

/// The tool a request names, and for `_scene` the tool it is about.
#[cfg(any(unix, windows))]
fn engine_tool(message: &Value) -> Option<&str> {
    let tool = message["tool"].as_str()?;
    let tool = if tool == SCENE { message["arguments"]["tool"].as_str()? } else { tool };
    crate::engine::handles(tool).then_some(tool)
}

/// Whether `line` is an authenticated request for the engine.
#[cfg(unix)]
fn for_engine(home: &Path, line: &str) -> bool {
    let Ok(message) = serde_json::from_str::<Value>(line) else { return false };
    let token = message["token"].as_str().unwrap_or_default();
    read_token(home).is_some_and(|t| same(&t, token)) && engine_tool(&message).is_some()
}

/// An engine request's answer (its token already checked).
#[cfg(any(unix, windows))]
fn engine_answer(engine: &Engine, line: &str) -> ToolResult {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return ToolResult::error("unreadable request");
    };
    let member = message["member"].as_str().unwrap_or("someone");
    let tool = message["tool"].as_str().unwrap_or_default();
    if tool == SCENE {
        let arguments = &message["arguments"];
        return match engine.scene(member, arguments["tool"].as_str().unwrap_or_default(), &arguments["arguments"]) {
            Ok(scene) => ToolResult::ok(scene.to_string()),
            Err(refused) => ToolResult::error(refused),
        };
    }
    let answer = engine.call(member, tool, &message["arguments"]);
    // The tool's name only: what it read or typed stays out of the log.
    eprintln!("equipe desktop helper: {member} {tool}{}", if answer.is_error { " (refused)" } else { "" });
    ToolResult { text: answer.text, is_error: answer.is_error, image: None }
}

#[cfg(windows)]
pub fn serve<D: Desktop>(home: &Path, controller: &mut Controller<D>, status: impl Fn(&D) -> Value, request: impl Fn(&D, Ask)) -> io::Result<()> {
    ensure_dir(home)?;
    ensure_token(home)?;
    let name = pipe_name(home);
    let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?;
    runtime.block_on(async move {
        use tokio::net::windows::named_pipe::ServerOptions;

        let engine = engine(home);
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
            let result = answer(home, controller, engine.as_deref(), &line, &status, &request);
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
    engine: Option<&Engine>,
    line: &str,
    status: &impl Fn(&D) -> Value,
    request: &impl Fn(&D, Ask),
) -> ToolResult {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return ToolResult::error("unreadable request");
    };
    let token = message["token"].as_str().unwrap_or_default();
    if read_token(home).is_none_or(|t| !same(&t, token)) {
        return ToolResult::error("wrong token");
    }
    if engine_tool(&message).is_some() {
        return match engine {
            Some(engine) => engine_answer(engine, line),
            None => ToolResult::error("The accessibility tools aren't available on this platform yet: use screenshot, elements and click."),
        };
    }
    let member = message["member"].as_str().unwrap_or("someone");
    let tool = message["tool"].as_str().unwrap_or_default();
    // The helper's own state, and the engine's beside it.
    let with_engine = |mut status: Value| {
        if let (Some(engine), Some(object)) = (engine, status.as_object_mut()) {
            object.insert("engine".into(), engine.status());
        }
        status
    };
    match tool {
        STATUS => ToolResult::ok(with_engine(status(&controller.desktop)).to_string()),
        REQUEST | REQUEST_ACCESSIBILITY => {
            request(&controller.desktop, if tool == REQUEST { Ask::Everything } else { Ask::Accessibility });
            ToolResult::ok(with_engine(status(&controller.desktop)).to_string())
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
