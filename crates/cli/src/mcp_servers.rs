//! `equipe mcp add|list|remove|login`: the outside MCP servers the squad
//! connects for its members (`equipe_core::squad_mcp`), and `equipe mcp
//! proxy`, the stdio server a member's engine runs for one of them.
//!
//! Through the daemon when it runs — the same commands, states and cards as
//! the app's Logins › Connections — else kept in the squad's journal for when
//! it does. `login` starts equipe's own sign-in on the daemon and shows its
//! page here (or, for a server connected with `--engine-signin`, runs the
//! engine's own sign-in in this terminal).

use std::path::Path;
use std::time::Duration;

use equipe_core::protocol::cli_signin::{Phase, PastedCode};
use equipe_core::protocol::mcp_servers::McpServerStatus;
use equipe_core::protocol::{Command, Engine, Event};
use equipe_core::squad_mcp::{self as sm, McpAuth, McpServer, McpState};
use equipe_core::t;

/// The engines whose tools are on this machine, and where.
fn engines() -> Vec<(Engine, String)> {
    [(Engine::Claude, "claude"), (Engine::Codex, "codex")].into_iter().filter_map(|(e, name)| on_path(name).map(|p| (e, p))).collect()
}

fn on_path(name: &str) -> Option<String> {
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(name)).find(|p| p.is_file())).map(|p| p.to_string_lossy().into_owned())
}

fn journal(home: &Path, squad: &str) -> Result<equipe_core::journal::Journal, String> {
    equipe_core::journal::Journal::for_squad(home, squad).map_err(|err| format!("Couldn't open the squad's settings: {err}"))
}

/// The running daemon, attached to `squad`. `None`: no daemon.
async fn attach(socket: &Path, squad: &str) -> Option<Result<(equipe_core::gateway::Client, equipe_core::protocol::Snapshot), String>> {
    if !equipe_core::gateway::is_running(socket).await {
        return None;
    }
    Some(equipe_core::gateway::Client::connect(socket, &crate::daemon::build_id(), squad).await.map(|(client, hello)| (client, hello.snapshot)).map_err(|e| e.to_string()))
}

/// Send `command` to the running daemon and wait, at most `wait`, for its
/// `McpServers` answer that `done` accepts; the last one seen comes back.
/// `None`: no daemon (the caller keeps it itself).
async fn through_daemon(socket: &Path, squad: &str, command: Command, wait: Duration, done: impl Fn(&[McpServer], &[McpServerStatus]) -> bool) -> Option<Result<(Vec<McpServer>, Vec<McpServerStatus>), String>> {
    let (mut client, snapshot) = match attach(socket, squad).await? {
        Ok(c) => c,
        Err(e) => return Some(Err(e)),
    };
    client.send(command);
    let mut last = (snapshot.mcp_servers, snapshot.mcp_status);
    let answer = async {
        while let Some(event) = client.next_event().await {
            match event {
                Event::McpServers { squad: s, servers, status } if s == squad => {
                    let finished = done(&servers, &status);
                    last = (servers, status);
                    if finished {
                        return Ok(());
                    }
                }
                Event::Error { session: None, message, .. } => return Err(message),
                _ => {}
            }
        }
        Err(t!("mcp.no_answer").to_owned())
    };
    Some(match tokio::time::timeout(wait, answer).await {
        Ok(Ok(())) => Ok(last),
        Ok(Err(why)) => Err(why),
        // Not all looked at in time: what is known so far.
        Err(_) if !last.0.is_empty() => Ok(last),
        Err(_) => Err(t!("mcp.no_answer").to_owned()),
    })
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Where `name` stands in `status`, once it was looked at since `since`.
fn looked_at(status: &[McpServerStatus], name: &str, since: u64) -> Option<McpServerStatus> {
    status.iter().find(|s| s.name == name && !s.engines.is_empty() && s.engines.iter().all(|e| e.state != McpState::Unchecked && e.checked_at >= since)).cloned()
}

/// `equipe mcp add <name> [url] [--roles a,b] [--sse] [--engine-signin]`.
#[allow(clippy::too_many_arguments)]
pub async fn add(home: &Path, socket: &Path, squad: &str, name: &str, url: Option<&str>, roles: &[String], sse: bool, engine_signin: bool) -> Result<Vec<String>, String> {
    let mut server = sm::server(name, url, (!roles.is_empty()).then_some(roles), Some(if sse { "sse" } else { "http" }))?;
    if engine_signin && !matches!(server.auth, McpAuth::Key { .. }) {
        server.auth = McpAuth::Engine;
    }
    let wanted = server.clone();
    let since = now_secs();
    let command = Command::AddMcpServer { squad: squad.to_owned(), server: server.clone(), sign_in: false };
    let mut state = None;
    match through_daemon(socket, squad, command, Duration::from_secs(75), |servers, status| servers.contains(&wanted) && looked_at(status, &wanted.name, since).is_some()).await {
        Some(result) => state = looked_at(&result?.1, &server.name, since),
        None => {
            let journal = journal(home, squad)?;
            let servers = sm::with(&journal.mcp_servers(), server.clone())?;
            journal.set_mcp_servers(&servers).map_err(|err| format!("Couldn't keep it: {err}"))?;
        }
    }
    let roles = if server.roles.is_empty() { t!("mcp.everyone").to_owned() } else { server.roles.join(", ") };
    let mut lines = vec![t!("mcp.added", name = sm::label(&server), url = server.url, roles = roles)];
    if let Some(service) = sm::service_of(&server).and_then(|s| s.mcp) {
        lines.push(t!("mcp.plan_note", url = service.sign_up_url));
    }
    match state {
        // Where it stands, as the app shows it — and what to do next only when something is left.
        Some(status) => {
            lines.push(status_words(&server, &status));
            if status.overall() != McpState::Connected {
                lines.push(next_step(&server));
            }
        }
        None => lines.push(next_step(&server)),
    }
    Ok(lines)
}

/// What the person does next for `server`: sign in (equipe's own sign-in,
/// or the engine's for the fallback), or — a keyed server — save its API key.
fn next_step(server: &McpServer) -> String {
    match (&server.auth, sm::service_of(server).and_then(|s| s.key_url)) {
        (McpAuth::Key { .. }, Some(url)) => t!("mcp.key_next", url = url),
        (McpAuth::Key { .. }, None) => t!("mcp.key_next", url = "-"),
        _ => t!("mcp.login_next", name = server.name),
    }
}

/// One line for where a server stands: one word for all engines when equipe
/// connects it, each engine's for the engine-signed fallback.
fn status_words(server: &McpServer, status: &McpServerStatus) -> String {
    if server.auth.proxied() {
        let first = status.engines.first();
        return state_words(status.overall(), first.and_then(|e| e.detail.as_deref()), &server.name);
    }
    status.engines.iter().map(|e| format!("{}: {}", engine_name(e.engine), state_words(e.state, e.detail.as_deref(), &server.name))).collect::<Vec<_>>().join(" · ")
}

/// `equipe mcp list`: each server and where it stands — the daemon's word
/// when it runs (`check`: looked at again now), the same the app shows.
pub async fn list(home: &Path, socket: &Path, squad: &str, check: bool) -> Result<Vec<String>, String> {
    let since = now_secs();
    let from_daemon = if check {
        through_daemon(socket, squad, Command::CheckMcpServers { squad: squad.to_owned() }, Duration::from_secs(75), |servers, status| servers.iter().filter(|s| s.enabled).all(|s| looked_at(status, &s.name, since).is_some())).await
    } else {
        attach(socket, squad).await.map(|got| got.map(|(_, snapshot)| (snapshot.mcp_servers, snapshot.mcp_status)))
    };
    match from_daemon {
        Some(answer) => {
            let (servers, status) = answer?;
            Ok(lines_of(&servers, |s| status.iter().find(|st| st.name == s.name).filter(|st| !st.engines.is_empty()).map(|st| status_words(s, st))))
        }
        None => list_here(home, squad, check, &engines()).await,
    }
}

fn lines_of(servers: &[McpServer], mut stands: impl FnMut(&McpServer) -> Option<String>) -> Vec<String> {
    if servers.is_empty() {
        return vec![t!("mcp.none").to_owned()];
    }
    let mut lines = Vec::new();
    for server in servers {
        let roles = if server.roles.is_empty() { t!("mcp.everyone").to_owned() } else { server.roles.join(", ") };
        let off = if server.enabled { String::new() } else { format!(" · {}", t!("mcp.off")) };
        lines.push(format!("{}  {}  ({roles}){off}", server.name, server.url));
        if server.enabled
            && let Some(words) = stands(server)
        {
            lines.push(format!("  {words}"));
        }
    }
    lines
}

/// [`list`] with no daemon: the settings, and — for the engine-signed
/// fallback — the engines' tools `engines` asked here.
async fn list_here(home: &Path, squad: &str, check: bool, engines: &[(Engine, String)]) -> Result<Vec<String>, String> {
    let servers = journal(home, squad)?.mcp_servers();
    let dir = sm::signin_dir(home, squad);
    let kept = equipe_core::squad_secrets::read(&journal(home, squad)?);
    let mut words = std::collections::HashMap::new();
    for server in servers.iter().filter(|s| s.enabled && check) {
        let line = match &server.auth {
            McpAuth::Key { secret, .. } if kept.iter().any(|k| k.name == *secret) => t!("mcp.key_saved").to_owned(),
            McpAuth::Key { .. } => next_step(server),
            McpAuth::Oauth => t!("mcp.daemon_off").to_owned(),
            McpAuth::Engine => {
                let mut each = Vec::new();
                for (engine, binary) in engines.iter().filter(|(e, _)| server.transport.on(*e)) {
                    let status = sm::check(*engine, binary, &dir, server, 0).await;
                    each.push(format!("{}: {}", engine_name(*engine), state_words(status.state, status.detail.as_deref(), &server.name)));
                }
                each.join(" · ")
            }
        };
        words.insert(server.name.clone(), line);
    }
    Ok(lines_of(&servers, |s| words.get(&s.name).cloned().filter(|w| !w.is_empty())))
}

fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "Claude Code",
        Engine::Codex => "Codex",
    }
}

/// Where a server stands, in the person's words.
pub fn state_words(state: McpState, detail: Option<&str>, name: &str) -> String {
    match (state, detail) {
        (McpState::Connected, _) => t!("mcp.connected").to_owned(),
        (McpState::NeedsSignIn, _) => t!("mcp.needs_sign_in", name = name),
        (McpState::NeedsKey, Some(why)) => t!("mcp.needs_key_why", why = why),
        (McpState::NeedsKey, None) => t!("mcp.needs_key").to_owned(),
        (McpState::Failing, Some(why)) => t!("mcp.failing_why", why = why),
        (McpState::Failing, None) => t!("mcp.failing").to_owned(),
        (McpState::Unchecked, _) => t!("mcp.unchecked").to_owned(),
    }
}

/// `equipe mcp remove <name>`.
pub async fn remove(home: &Path, socket: &Path, squad: &str, name: &str) -> Result<String, String> {
    remove_with(home, socket, squad, name, &engines()).await
}

/// [`remove`], clearing the sign-in kept by the engines' tools `engines`.
async fn remove_with(home: &Path, socket: &Path, squad: &str, name: &str, engines: &[(Engine, String)]) -> Result<String, String> {
    let name = sm::check_name(name)?;
    let servers = journal(home, squad)?.mcp_servers();
    let Some(gone) = servers.iter().find(|s| s.name == name).cloned() else {
        return Err(t!("mcp.unknown", name = name));
    };
    let command = Command::RemoveMcpServer { squad: squad.to_owned(), name: name.clone() };
    let left = name.clone();
    match through_daemon(socket, squad, command, Duration::from_secs(15), move |servers, _| !servers.iter().any(|s| s.name == left)).await {
        Some(result) => {
            result?;
        }
        None => {
            let kept: Vec<McpServer> = servers.into_iter().filter(|s| s.name != name).collect();
            journal(home, squad)?.set_mcp_servers(&kept).map_err(|err| format!("Couldn't keep it: {err}"))?;
            let shared = equipe_core::squads::list(home).iter().filter(|s| s.name != squad).any(|s| journal(home, &s.name).is_ok_and(|j| j.mcp_servers().iter().any(|o| o.name == gone.name && o.url == gone.url)));
            if !shared {
                for (engine, binary) in engines {
                    sm::forget(*engine, binary, &sm::signin_dir(home, squad), &gone).await;
                }
            }
        }
    }
    Ok(t!("mcp.removed", name = sm::label(&gone)))
}

/// `equipe mcp login <name> [--engine claude|codex]`: equipe's own sign-in,
/// started on the daemon — the same card the app shows — its page opened
/// here; or, for the engine-signed fallback, the engine's own, in this terminal.
pub async fn login(home: &Path, socket: &Path, squad: &str, name: &str, engine: Option<&str>) -> Result<String, String> {
    let name = sm::check_name(name)?;
    let server = journal(home, squad)?.mcp_servers().into_iter().find(|s| s.name == name).ok_or_else(|| t!("mcp.unknown", name = name))?;
    match server.auth {
        McpAuth::Key { .. } => Err(next_step(&server)),
        McpAuth::Oauth => login_through_daemon(socket, squad, &server).await,
        McpAuth::Engine => login_with_engine(home, socket, squad, &server, engine).await,
    }
}

async fn login_through_daemon(socket: &Path, squad: &str, server: &McpServer) -> Result<String, String> {
    use tokio::io::AsyncBufReadExt as _;
    let (mut client, _) = attach(socket, squad).await.ok_or_else(|| t!("mcp.daemon_off").to_owned())??;
    client.send(Command::SignInMcpServer { squad: squad.to_owned(), name: server.name.clone() });
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut card_id = None;
    let mut shown = false;
    let deadline = tokio::time::Instant::now() + equipe_core::mcp_oauth::SIGN_IN_WAIT;
    loop {
        tokio::select! {
            event = client.next_event() => match event {
                Some(Event::CliSignInChanged { squad: s, card }) if s == squad && card.server.as_deref() == Some(server.name.as_str()) => {
                    card_id = Some(card.id);
                    match card.phase {
                        Phase::Waiting | Phase::Checking => {
                            if let (Some(url), false) = (card.url.as_deref(), shown) {
                                shown = true;
                                println!("  {}", t!("mcp.login_page", name = sm::label(server)));
                                println!("  {url}");
                                println!("  {}", t!("mcp.login_paste"));
                                let _ = std::process::Command::new("open").arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
                            }
                        }
                        Phase::SignedIn => return Ok(t!("mcp.signed_in_equipe", name = sm::label(server))),
                        Phase::Expired => return Err(t!("mcp.login_failed", name = sm::label(server))),
                        Phase::Failed => return Err(format!("{} ({})", t!("mcp.login_failed", name = sm::label(server)), card.detail.unwrap_or_default())),
                        Phase::Starting => {}
                    }
                }
                Some(Event::CliSignInClosed { squad: s, id }) if s == squad && card_id == Some(id) => return Err(t!("mcp.login_failed", name = sm::label(server))),
                // Connected with no sign-in at all: nothing more to do.
                Some(Event::McpServers { squad: s, status, .. }) if s == squad && card_id.is_none() && status.iter().any(|st| st.name == server.name && st.overall() == McpState::Connected) => {
                    return Ok(t!("mcp.signed_in_equipe", name = sm::label(server)));
                }
                Some(Event::Error { session: None, message, .. }) => return Err(message),
                Some(_) => {}
                None => return Err(t!("mcp.no_answer").to_owned()),
            },
            line = stdin.next_line(), if card_id.is_some() => {
                if let (Ok(Some(line)), Some(id)) = (line, card_id)
                    && !line.trim().is_empty()
                {
                    client.send(Command::SubmitCliSignInCode { squad: squad.to_owned(), id, code: PastedCode(line.trim().to_owned()) });
                }
            }
            _ = tokio::time::sleep_until(deadline) => return Err(t!("mcp.login_failed", name = sm::label(server))),
        }
    }
}

async fn login_with_engine(home: &Path, socket: &Path, squad: &str, server: &McpServer, engine: Option<&str>) -> Result<String, String> {
    let here = engines();
    let wanted = match engine.map(|e| e.trim().to_ascii_lowercase()) {
        None => here.first().map(|(e, _)| *e).ok_or_else(|| t!("mcp.no_engine").to_owned())?,
        Some(e) if e.starts_with("claude") => Engine::Claude,
        Some(e) if e == "codex" => Engine::Codex,
        Some(e) => return Err(t!("mcp.bad_engine", engine = e)),
    };
    let binary = here.iter().find(|(e, _)| *e == wanted).map(|(_, b)| b.clone()).ok_or_else(|| t!("mcp.no_engine").to_owned())?;
    let dir = sm::signin_dir(home, squad);
    let (args, cwd) = match wanted {
        Engine::Claude => {
            sm::prepare_claude(&binary, &dir, server).await?;
            (sm::claude_login_args(&server.name, false), Some(dir))
        }
        Engine::Codex => (sm::codex_login_args(server), None),
    };
    let mut cmd = tokio::process::Command::new(&binary);
    cmd.args(&args);
    if let Some(dir) = &cwd {
        cmd.current_dir(dir);
    }
    let status = cmd.status().await.map_err(|e| format!("{binary}: {e}"))?;
    if !status.success() {
        return Err(t!("mcp.login_failed", name = sm::label(server)));
    }
    // The daemon looks again, so its screens and the next turns know.
    let _ = through_daemon(socket, squad, Command::CheckMcpServers { squad: squad.to_owned() }, Duration::from_secs(15), |_, _| true).await;
    Ok(t!("mcp.signed_in", name = sm::label(server), engine = engine_name(wanted)))
}

// ───────────────────────── the proxy ─────────────────────────

/// `equipe mcp proxy`: a stdio MCP server for one outside server, run by a
/// member's engine. It holds no credential: each line from the engine goes
/// to the daemon, which forwards it with the credential added
/// (`equipe_core::mcp_gateway`); each message back is a line to the engine.
/// When the daemon can't be reached, or won't serve it, every request is
/// answered with why, so the engine says it.
pub async fn proxy(home: &Path, squad: &str, server: &str, member: &str) -> Result<(), String> {
    use equipe_core::protocol::rpc::{self, McpProxyMessage, McpProxyOpen, Message};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
    let socket = equipe_core::gateway::socket_path(home);
    let mut engine_lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut engine_out = tokio::io::stdout();
    let opened: Result<(tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf), String> = async {
        let stream = tokio::net::UnixStream::connect(&socket).await.map_err(|_| "equipe's daemon isn't running on this Mac".to_owned())?;
        let (reader, mut writer) = stream.into_split();
        let open = McpProxyOpen { squad: squad.to_owned(), server: server.to_owned(), member: member.to_owned() };
        writer.write_all(Message::request(1, rpc::MCP_PROXY, open).to_line().as_bytes()).await.map_err(|e| e.to_string())?;
        Ok((reader, writer))
    }
    .await;
    let (reader, mut writer) = match opened {
        Ok(halves) => halves,
        Err(why) => return answer_with(&why, &mut engine_lines, &mut engine_out).await,
    };
    let mut daemon_lines = tokio::io::BufReader::new(reader).lines();
    // The daemon's yes, or why not.
    loop {
        let Ok(Some(line)) = daemon_lines.next_line().await else {
            return answer_with("equipe's daemon closed the connection", &mut engine_lines, &mut engine_out).await;
        };
        let Ok(message) = Message::from_line(&line) else { continue };
        if message.id != Some(1) {
            continue;
        }
        if let Some(error) = message.error {
            return answer_with(&error.message, &mut engine_lines, &mut engine_out).await;
        }
        break;
    }
    loop {
        tokio::select! {
            line = engine_lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
                let sent = Message::notification(rpc::MCP_PROXY_SEND, McpProxyMessage { message }).to_line();
                if writer.write_all(sent.as_bytes()).await.is_err() {
                    break;
                }
            }
            line = daemon_lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                let Ok(message) = Message::from_line(&line) else { continue };
                if message.method.as_deref() != Some(rpc::MCP_PROXY_RECV) {
                    continue;
                }
                let Some(McpProxyMessage { message }) = message.params_as::<McpProxyMessage>() else { continue };
                let mut text = message.to_string();
                text.push('\n');
                if engine_out.write_all(text.as_bytes()).await.is_err() || engine_out.flush().await.is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Answer every request from the engine with `why` until it goes.
async fn answer_with<R, W>(why: &str, lines: &mut tokio::io::Lines<R>, out: &mut W) -> Result<(), String>
where
    R: tokio::io::AsyncBufRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt as _;
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if let Some(reply) = equipe_core::mcp_gateway::error_reply(&message, why) {
            let mut text = reply.to_string();
            text.push('\n');
            if out.write_all(text.as_bytes()).await.is_err() || out.flush().await.is_err() {
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_reads_in_the_persons_words() {
        assert!(state_words(McpState::NeedsSignIn, None, "mobbin").contains("equipe mcp login mobbin"));
        assert!(state_words(McpState::Failing, Some("keychain locked"), "m").contains("keychain locked"));
        assert_ne!(state_words(McpState::Connected, None, "m"), state_words(McpState::Unchecked, None, "m"));
        let key = state_words(McpState::NeedsKey, None, "21st");
        assert!(!key.contains("login") && !key.contains("sign"), "a keyed server never needs a sign-in: {key}");
    }

    #[tokio::test]
    async fn without_a_daemon_add_list_and_remove_keep_the_squads_servers() {
        let home = std::env::temp_dir().join(format!("equipe-cli-mcp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(equipe_core::journal::squad_dir(&home, "acme")).unwrap();
        let socket = home.join("no-daemon.sock");
        let said = add(&home, &socket, "acme", "mobbin", None, &[], false, false).await.unwrap();
        assert!(said[0].contains("Mobbin") && said[0].contains("designer, frontend"), "{said:?}");
        assert!(said.iter().any(|l| l.contains("https://mobbin.com/pricing")) && said.iter().any(|l| l.contains("equipe mcp login mobbin")), "{said:?}");
        add(&home, &socket, "acme", "docs", Some("https://docs.example.com/mcp"), &["qa".to_owned()], false, false).await.unwrap();
        assert!(add(&home, &socket, "acme", "docs", Some("http://docs.example.com/mcp"), &[], false, false).await.is_err(), "https only");
        let listed = list(&home, &socket, "acme", false).await.unwrap();
        assert_eq!(listed, ["mobbin  https://api.mobbin.com/mcp  (designer, frontend)", "docs  https://docs.example.com/mcp  (qa)"]);
        // Added again: the same one, kept once.
        add(&home, &socket, "acme", "mobbin", None, &[], false, false).await.unwrap();
        assert_eq!(journal(&home, "acme").unwrap().mcp_servers().len(), 2, "idempotent");
        // The engine-signed fallback, asked for.
        add(&home, &socket, "acme", "legacy", Some("https://legacy.example.com/mcp"), &[], false, true).await.unwrap();
        assert!(journal(&home, "acme").unwrap().mcp_servers().iter().any(|s| s.name == "legacy" && s.auth.is_engine()));
        remove_with(&home, &socket, "acme", "legacy", &[]).await.unwrap();
        // A keyed server: its key is saved on the app's key card, never typed here.
        let keyed = add(&home, &socket, "acme", "21st", None, &[], false, false).await.unwrap();
        assert!(keyed.iter().any(|l| l.contains("https://21st.dev/mcp")) && !keyed.iter().any(|l| l.contains("mcp login")), "{keyed:?}");
        assert!(list_here(&home, "acme", true, &[]).await.unwrap().iter().any(|l| l.contains("https://21st.dev/mcp")));
        assert!(login(&home, &socket, "acme", "21st", None).await.unwrap_err().contains("https://21st.dev/mcp"));
        remove_with(&home, &socket, "acme", "21st", &[]).await.unwrap();
        // No engine's tool touched here: the machine's own sign-ins are left alone.
        remove_with(&home, &socket, "acme", "docs", &[]).await.unwrap();
        assert_eq!(journal(&home, "acme").unwrap().mcp_servers().len(), 1);
        assert!(remove_with(&home, &socket, "acme", "docs", &[]).await.is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
