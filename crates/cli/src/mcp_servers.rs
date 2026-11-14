//! `equipe mcp add|list|remove|login`: the outside MCP servers the squad
//! connects for its members (`equipe_core::squad_mcp`). Through the daemon
//! when it runs — its screens and the next turns see the change at once —
//! else kept in the squad's journal for when it does. `login` runs the
//! engine's own sign-in right here, in this terminal: equipe never sees the
//! token.

use std::path::Path;
use std::time::Duration;

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

/// Send `command` to the running daemon and wait for its `McpServers` answer
/// that `done` accepts. `None`: no daemon (the caller keeps it itself).
async fn through_daemon(socket: &Path, squad: &str, command: Command, done: impl Fn(&[McpServer]) -> bool) -> Option<Result<(), String>> {
    if !equipe_core::gateway::is_running(socket).await {
        return None;
    }
    let (mut client, _) = match equipe_core::gateway::Client::connect(socket, &crate::daemon::build_id(), squad).await {
        Ok(c) => c,
        Err(e) => return Some(Err(e.to_string())),
    };
    client.send(command);
    let answer = async {
        while let Some(event) = client.next_event().await {
            match event {
                Event::McpServers { squad: s, servers, .. } if s == squad && done(&servers) => return Ok(()),
                Event::Error { session: None, message, .. } => return Err(message),
                _ => {}
            }
        }
        Err(t!("mcp.no_answer").to_owned())
    };
    Some(tokio::time::timeout(Duration::from_secs(15), answer).await.unwrap_or_else(|_| Err(t!("mcp.no_answer").to_owned())))
}

/// `equipe mcp add <name> [url] [--roles a,b] [--sse]`.
pub async fn add(home: &Path, socket: &Path, squad: &str, name: &str, url: Option<&str>, roles: &[String], sse: bool) -> Result<Vec<String>, String> {
    let server = sm::server(name, url, (!roles.is_empty()).then_some(roles), Some(if sse { "sse" } else { "http" }))?;
    let wanted = server.clone();
    let command = Command::AddMcpServer { squad: squad.to_owned(), server: server.clone(), sign_in: false };
    match through_daemon(socket, squad, command, move |servers| servers.contains(&wanted)).await {
        Some(result) => result?,
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
    lines.push(next_step(&server));
    Ok(lines)
}

/// What the person does next for `server`: sign in with the engine's own
/// CLI, or (a keyed server) save its API key on the app's key card.
fn next_step(server: &McpServer) -> String {
    match (&server.auth, sm::service_of(server).and_then(|s| s.key_url)) {
        (McpAuth::Key { .. }, Some(url)) => t!("mcp.key_next", url = url),
        (McpAuth::Key { .. }, None) => t!("mcp.key_next", url = "-"),
        _ => t!("mcp.login_next", name = server.name),
    }
}

/// `equipe mcp list`: each server and where it stands on each engine here
/// (`check`: asked of the engines' own tools now).
pub async fn list(home: &Path, squad: &str, check: bool) -> Result<Vec<String>, String> {
    list_with(home, squad, check, &engines()).await
}

/// [`list`], asking the engines' tools `engines`.
async fn list_with(home: &Path, squad: &str, check: bool, engines: &[(Engine, String)]) -> Result<Vec<String>, String> {
    let servers = journal(home, squad)?.mcp_servers();
    if servers.is_empty() {
        return Ok(vec![t!("mcp.none").to_owned()]);
    }
    let dir = sm::signin_dir(home, squad);
    let mut lines = Vec::new();
    for server in &servers {
        let roles = if server.roles.is_empty() { t!("mcp.everyone").to_owned() } else { server.roles.join(", ") };
        let off = if server.enabled { String::new() } else { format!(" · {}", t!("mcp.off")) };
        lines.push(format!("{}  {}  ({roles}){off}", server.name, server.url));
        if !check || !server.enabled {
            continue;
        }
        // A keyed server stands on its key being kept (its facts; the value is never read here).
        if let McpAuth::Key { secret, .. } = &server.auth {
            let kept = equipe_core::squad_secrets::read(&journal(home, squad)?).iter().any(|k| k.name == *secret);
            lines.push(format!("  {}", if kept { t!("mcp.key_saved").to_owned() } else { next_step(server) }));
            continue;
        }
        for (engine, binary) in engines {
            if !server.transport.on(*engine) {
                continue;
            }
            let status = sm::check(*engine, binary, &dir, server, 0).await;
            lines.push(format!("  {}: {}", engine_name(*engine), state_words(status.state, status.detail.as_deref(), &server.name)));
        }
    }
    Ok(lines)
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
    match through_daemon(socket, squad, command, move |servers| !servers.iter().any(|s| s.name == left)).await {
        Some(result) => result?,
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

/// `equipe mcp login <name> [--engine claude|codex]`: the engine's own
/// sign-in, interactive, in this terminal; its browser page opens here.
pub async fn login(home: &Path, socket: &Path, squad: &str, name: &str, engine: Option<&str>) -> Result<String, String> {
    let name = sm::check_name(name)?;
    let server = journal(home, squad)?.mcp_servers().into_iter().find(|s| s.name == name).ok_or_else(|| t!("mcp.unknown", name = name))?;
    if !server.auth.is_engine() {
        return Err(next_step(&server));
    }
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
            sm::prepare_claude(&binary, &dir, &server).await?;
            (sm::claude_login_args(&server.name, false), Some(dir))
        }
        Engine::Codex => (sm::codex_login_args(&server), None),
    };
    let mut cmd = tokio::process::Command::new(&binary);
    cmd.args(&args);
    if let Some(dir) = &cwd {
        cmd.current_dir(dir);
    }
    let status = cmd.status().await.map_err(|e| format!("{binary}: {e}"))?;
    if !status.success() {
        return Err(t!("mcp.login_failed", name = sm::label(&server)));
    }
    // The daemon looks again, so its screens and the next turns know.
    let _ = through_daemon(socket, squad, Command::CheckMcpServers { squad: squad.to_owned() }, |_| true).await;
    Ok(t!("mcp.signed_in", name = sm::label(&server), engine = engine_name(wanted)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_reads_in_the_persons_words() {
        assert!(state_words(McpState::NeedsSignIn, None, "mobbin").contains("equipe mcp login mobbin"));
        assert!(state_words(McpState::Failing, Some("keychain locked"), "m").contains("keychain locked"));
        assert_ne!(state_words(McpState::Connected, None, "m"), state_words(McpState::Unchecked, None, "m"));
    }

    #[tokio::test]
    async fn without_a_daemon_add_list_and_remove_keep_the_squads_servers() {
        let home = std::env::temp_dir().join(format!("equipe-cli-mcp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(equipe_core::journal::squad_dir(&home, "acme")).unwrap();
        let socket = home.join("no-daemon.sock");
        let said = add(&home, &socket, "acme", "mobbin", None, &[], false).await.unwrap();
        assert!(said[0].contains("Mobbin") && said[0].contains("designer, frontend"), "{said:?}");
        assert!(said.iter().any(|l| l.contains("https://mobbin.com/pricing")) && said.iter().any(|l| l.contains("equipe mcp login mobbin")), "{said:?}");
        add(&home, &socket, "acme", "docs", Some("https://docs.example.com/mcp"), &["qa".to_owned()], false).await.unwrap();
        assert!(add(&home, &socket, "acme", "docs", Some("http://docs.example.com/mcp"), &[], false).await.is_err(), "https only");
        let listed = list(&home, "acme", false).await.unwrap();
        assert_eq!(listed, ["mobbin  https://api.mobbin.com/mcp  (designer, frontend)", "docs  https://docs.example.com/mcp  (qa)"]);
        // A keyed server: its key is saved on the app's key card, never typed here.
        let keyed = add(&home, &socket, "acme", "21st", None, &[], false).await.unwrap();
        assert!(keyed.iter().any(|l| l.contains("https://21st.dev/mcp")) && !keyed.iter().any(|l| l.contains("mcp login")), "{keyed:?}");
        assert!(list_with(&home, "acme", true, &[]).await.unwrap().iter().any(|l| l.contains("https://21st.dev/mcp")));
        assert!(login(&home, &socket, "acme", "21st", None).await.unwrap_err().contains("https://21st.dev/mcp"));
        remove_with(&home, &socket, "acme", "21st", &[]).await.unwrap();
        // No engine's tool touched here: the machine's own sign-ins are left alone.
        remove_with(&home, &socket, "acme", "docs", &[]).await.unwrap();
        assert_eq!(journal(&home, "acme").unwrap().mcp_servers().len(), 1);
        assert!(remove_with(&home, &socket, "acme", "docs", &[]).await.is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
