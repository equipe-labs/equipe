//! A squad's outside MCP servers, driven as the person's screen and the PM
//! drive them, against a stand-in server on this machine (`support/fake_mcp.rs`)
//! and a stand-in `claude` that answers turns:
//!
//! - equipe signs in once, as the OAuth client (discovery, registration,
//!   PKCE, a loopback redirect), on one card; every member of the server's
//!   roles then gets equipe's proxy — never a token — and the proxy's
//!   messages reach the server with the token added, renewed on a 401;
//! - over the daemon's socket, the proxy reaches a server on the older
//!   HTTP+SSE transport, and a member without the server is refused;
//! - a keyed server needs its key (never a sign-in), and the key stays in
//!   the daemon;
//! - the PM connects one only on the person's tap;
//! - the engine-signed fallback still signs in with Claude Code's own login.
// One test at a time (`SERIAL`), held across each whole test on purpose.
#![allow(clippy::await_holding_lock)]

#![cfg(unix)]

#[path = "support/fake_mcp.rs"]
mod fake_mcp;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use equipe_core::mcp_gateway::{self, Ask, Tell};
use equipe_core::protocol::cli_signin::Phase;
use equipe_core::protocol::mcp_servers::{McpAuth, McpServer, McpState, McpTransport};
use equipe_core::protocol::rpc::{self, McpProxyMessage, McpProxyOpen, Message, ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Decision, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::{gateway, squad_mcp, squads};
use equipe_providers::CredentialStore;
use fake_mcp::Fake;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// One test at a time: they share the process's in-memory secret store, where
/// the same squad and server names would meet.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn setup() {
    // SAFETY: set before any runtime starts, to the same values by every test here.
    unsafe {
        std::env::set_var("EQUIPE_VAULT_IN_MEMORY", "1");
        std::env::set_var("EQUIPE_MCP_ALLOW_LOOPBACK", "1");
    }
}

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-squad-mcp-it-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A stand-in `claude`: signed in; its `mcp get` says a server added in the
/// folder it runs in needs authentication (and `add` refuses one already
/// there, as Claude Code does); its `mcp login` prints the page and waits;
/// every turn's arguments are written down in `calls/` and answered "ok".
fn fake_claude() -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = scratch("bin");
    let calls = dir.join("calls");
    std::fs::create_dir_all(&calls).unwrap();
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "auth" ]; then echo '{{"loggedIn":true,"authMethod":"claude.ai"}}'; exit 0; fi
if [ "$1" = "mcp" ]; then
  echo "$(pwd) $*" >> '{log}'
  case "$2" in
    get) if [ -f added ]; then printf '%s:\n  Status: ! Needs authentication\n  Type: http\n  URL: %s\n' "$3" "$(cat added)"; else echo "No MCP server named \"$3\"."; exit 1; fi ;;
    add) if [ -f added ]; then echo "MCP server $7 already exists in local config"; exit 1; fi; echo "$8" > added ;;
    remove) rm -f added ;;
    login) echo 'Visit this URL to authorize:'; echo '  https://auth.example.com/oauth/authorize?client_id=x&redirect_uri=http%3A%2F%2Flocalhost%3A5555%2Fcallback '; printf 'Or paste the redirect URL here: '; exec sleep 60 ;;
  esac
  exit 0
fi
n=0
while ! mkdir '{calls}/.n-'$n 2>/dev/null; do n=$((n+1)); done
for a in "$@"; do printf '%s\n<<arg>>\n' "$a"; done > '{calls}/.call-'$$
mv '{calls}/.call-'$$ '{calls}/call-'$(printf %03d $n)-$$
cat <<EOF
{{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}}
{{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{{"input_tokens":3,"output_tokens":1}}}}
EOF
"#,
        log = dir.join("mcp.log").display(),
        calls = calls.display()
    );
    let bin = dir.join("claude");
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, dir)
}

fn calls(dir: &Path) -> Vec<Vec<String>> {
    // Only whole calls: each is written aside and moved into place once complete.
    let mut files: Vec<_> = std::fs::read_dir(dir.join("calls")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("call-")).map(|e| e.path()).collect();
    files.sort();
    files.iter().map(|f| std::fs::read_to_string(f).unwrap().split("\n<<arg>>\n").filter(|a| !a.is_empty()).map(str::to_owned).collect()).collect()
}

fn arg<'a>(call: &'a [String], flag: &str) -> Option<&'a str> {
    call.iter().position(|a| a == flag).and_then(|i| call.get(i + 1)).map(String::as_str)
}

fn config(claude: &Path, home: &Path) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        codex_binary: "/nonexistent/codex".into(),
        persist: true,
        retention_days: 90,
        equipe_binary: Some("/opt/equipe".into()),
        npx: None,
        design_path: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        person_home: Some(home.join("person")),
        lang: equipe_core::i18n::Lang::En,
        nemotron: None,
        watch_disk: false,
    }
}

async fn until(rx: &mut UnboundedReceiver<Event>, done: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    let wait = async {
        while let Some(event) = rx.recv().await {
            let stop = done(&event);
            seen.push(event);
            if stop {
                return;
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(30), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: Value) -> ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

/// One turn of `session`, and its arguments.
async fn turn(handle: &Handle, events: &mut UnboundedReceiver<Event>, recorded: &Path, session: u64) -> Vec<String> {
    let before = calls(recorded).len();
    handle.send(Command::Submit { session, text: "go".into(), about: None });
    until(events, |e| matches!(e, Event::TurnEnded { session: s, .. } if *s == session)).await;
    let all = calls(recorded);
    assert_eq!(all.len(), before + 1);
    all.last().unwrap().clone()
}

fn servers(call: &[String]) -> Value {
    serde_json::from_str::<Value>(arg(call, "--mcp-config").expect("an MCP config")).unwrap()["mcpServers"].clone()
}

/// The squad's MCP sign-in card of `tool` once it shows its page.
async fn waiting_card(handle: &Handle, tool: &str) -> equipe_core::protocol::cli_signin::CliSignIn {
    for _ in 0..300 {
        if let Some(card) = handle.snapshot("acme").await.cli_sign_ins.into_iter().find(|c| c.tool == tool && c.phase == Phase::Waiting && c.url.is_some()) {
            return card;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no MCP sign-in card showed its page");
}

async fn overall(handle: &Handle, name: &str, want: McpState) {
    for _ in 0..300 {
        if handle.snapshot("acme").await.mcp_status.iter().any(|s| s.name == name && s.overall() == want) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{name} never stood {want:?}: {:?}", handle.snapshot("acme").await.mcp_status);
}

async fn squad_with(roles: &[(u64, &str)]) -> (Handle, UnboundedReceiver<Event>, PathBuf, PathBuf, PathBuf) {
    setup();
    let (claude, bin) = fake_claude();
    let project = scratch("proj").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    for (id, role) in roles {
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some((*role).into()), squad: Some("acme".into()) };
        handle.send(Command::Start { session: *id, spec });
        until(&mut events, |e| matches!(e, Event::SessionStarted { session, .. } if session == id)).await;
    }
    (handle, events, bin, home, project)
}

/// A proxy as the daemon's gateway runs one, on `handle`'s credentials: what goes in, what comes back.
fn proxy_on(handle: &Handle, member: &str, server: &str) -> (tokio::sync::mpsc::UnboundedSender<Value>, UnboundedReceiver<Value>) {
    let open = McpProxyOpen { squad: "acme".into(), server: server.into(), member: member.into() };
    let ask: Ask = {
        let (handle, open) = (handle.clone(), open.clone());
        Arc::new(move |rejected| {
            let (handle, open) = (handle.clone(), open.clone());
            Box::pin(async move { handle.mcp_upstream(&open, rejected).await })
        })
    };
    let tell: Tell = {
        let (handle, open) = (handle.clone(), open.clone());
        Arc::new(move |seen| handle.mcp_seen(&open, seen))
    };
    let (to_server, from_client) = unbounded_channel();
    let (to_client, from_server) = unbounded_channel();
    tokio::spawn(mcp_gateway::serve(mcp_gateway::client(), ask, tell, from_client, to_client, Arc::new(|_: &str| {})));
    (to_server, from_server)
}

async fn reply_to(rx: &mut UnboundedReceiver<Value>, id: u64) -> Value {
    let wait = async {
        while let Some(message) = rx.recv().await {
            if message["id"] == id {
                return message;
            }
        }
        panic!("the proxy went away");
    };
    tokio::time::timeout(Duration::from_secs(20), wait).await.expect("an answer in time")
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

#[tokio::test]
async fn equipe_signs_in_once_and_every_member_of_its_roles_gets_the_server_through_its_proxy() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Fake::start().await;
    let (handle, mut events, bin, home, _) = squad_with(&[(1, "pm"), (2, "designer"), (3, "qa")]).await;

    // Connected by its address: equipe's own connection looks, and finds it wants a sign-in.
    let docs = squad_mcp::server("docs", Some(&fake.url("/mcp")), Some(&["designer".to_owned()]), None).unwrap();
    assert_eq!(docs.auth, McpAuth::Oauth);
    handle.send(Command::AddMcpServer { squad: "acme".into(), server: docs.clone(), sign_in: false });
    overall(&handle, "docs", McpState::NeedsSignIn).await;
    assert!(handle.snapshot("acme").await.cli_sign_ins.is_empty(), "not asked for: no card");

    // The designer's turn: not given while it needs a sign-in, and one card asks for it.
    let first = turn(&handle, &mut events, &bin, 2).await;
    assert!(servers(&first).get("docs").is_none());
    let card = waiting_card(&handle, "equipe_mcp").await;
    assert_eq!((card.member.as_str(), card.server.as_deref(), card.wants_code), ("designer", Some("docs"), true));
    let page = card.url.clone().unwrap();
    assert!(page.starts_with(&fake.url("/auth/authorize?response_type=code&client_id=client-1&redirect_uri=http%3A%2F%2Flocalhost%3A")), "{page}");
    assert!(page.contains("code_challenge_method=S256") && page.contains("scope=mcp"), "{page}");
    assert!(equipe_core::cli_signin::loopback_port(&page).is_some(), "a remote screen can tunnel its redirect");
    // Again: still one card.
    turn(&handle, &mut events, &bin, 2).await;
    assert_eq!(handle.snapshot("acme").await.cli_sign_ins.iter().filter(|c| c.tool == "equipe_mcp").count(), 1);

    // The person opens the page; it comes back to equipe on this Mac, and it is connected — nothing restarted.
    let landed = fake_mcp::browse(&page).await;
    assert!(landed.contains("Signed in"), "{landed}");
    until(&mut events, |e| matches!(e, Event::CliSignInChanged { card: c, .. } if c.id == card.id && c.phase == Phase::SignedIn)).await;
    overall(&handle, "docs", McpState::Connected).await;
    let token = fake.state.lock().unwrap().valid.iter().next().cloned().unwrap();
    assert_eq!(fake.state.lock().unwrap().registrations, 1);
    assert!(!format!("{:?}", handle.snapshot("acme").await).contains(&token), "no token reaches a screen");

    // The designer's next turn has it — equipe's proxy, no token, no address.
    let call = turn(&handle, &mut events, &bin, 2).await;
    assert_eq!(servers(&call)["docs"], json!({ "type": "stdio", "command": "/opt/equipe", "args": ["mcp", "proxy", "--squad", "acme", "--server", "docs", "--member", "designer", "--home", home.to_string_lossy()] }));
    let config = arg(&call, "--mcp-config").unwrap().to_ascii_lowercase();
    for word in [token.as_str(), "authorization", "bearer", "headers", "127.0.0.1"] {
        assert!(!config.contains(word), "{word} in the member's MCP config: {config}");
    }
    assert!(call.iter().any(|a| a == "--strict-mcp-config"));
    assert!(call.join("\n").contains("`docs` (tools `mcp__docs__*`)"), "the designer is told of it");
    // QA isn't one of its roles: nothing of it, and its proxy would be refused.
    let qa = turn(&handle, &mut events, &bin, 3).await;
    assert!(servers(&qa).get("docs").is_none());
    let refused = handle.mcp_upstream(&McpProxyOpen { squad: "acme".into(), server: "docs".into(), member: "qa".into() }, None).await;
    assert!(refused.unwrap_err().contains("isn't connected for qa"));

    // Through the proxy: initialize, the tools listed (as an event stream), one called — the token added by the daemon.
    let (to, mut from) = proxy_on(&handle, "designer", "docs");
    to.send(request(1, "initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "claude-code", "version": "x" } }))).unwrap();
    assert_eq!(reply_to(&mut from, 1).await["result"]["serverInfo"]["name"], "fake");
    to.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).unwrap();
    to.send(request(2, "tools/list", json!({}))).unwrap();
    assert_eq!(reply_to(&mut from, 2).await["result"]["tools"][0]["name"], "search_screens");
    to.send(request(3, "tools/call", json!({ "name": "search_screens", "arguments": { "query": "checkout" } }))).unwrap();
    assert_eq!(reply_to(&mut from, 3).await["result"]["content"][0]["text"], "3 screens for checkout");
    assert_eq!(fake.state.lock().unwrap().calls.last().cloned(), Some(("search_screens".to_owned(), token.clone())));

    // The server stops taking the token: renewed once with the refresh token, and the call goes through.
    fake.revoke_all();
    to.send(request(4, "tools/call", json!({ "name": "search_screens", "arguments": { "query": "onboarding" } }))).unwrap();
    assert_eq!(reply_to(&mut from, 4).await["result"]["content"][0]["text"], "3 screens for onboarding");
    let (refreshes, used) = { let s = fake.state.lock().unwrap(); (s.refreshes, s.calls.last().unwrap().1.clone()) };
    assert_eq!(refreshes, 1);
    assert_ne!(used, token, "the fresh token");
    overall(&handle, "docs", McpState::Connected).await;

    // Disconnected: its sign-in goes with it; connected again, it asks for one again.
    handle.send(Command::RemoveMcpServer { squad: "acme".into(), name: "docs".into() });
    until(&mut events, |e| matches!(e, Event::McpServers { servers, .. } if servers.is_empty())).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    handle.send(Command::AddMcpServer { squad: "acme".into(), server: docs, sign_in: false });
    overall(&handle, "docs", McpState::NeedsSignIn).await;
    handle.shutdown().await;
}

#[tokio::test]
async fn over_the_daemons_socket_a_proxy_reaches_an_sse_server_and_a_member_without_it_is_refused() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    setup();
    let fake = Fake::start().await;
    let (claude, _bin) = fake_claude();
    let project = scratch("proj").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let socket = gateway::socket_path(&home);
    let serving = tokio::spawn({
        let (config, socket) = (config(&claude, &home), socket.clone());
        async move { gateway::serve(config, &socket, "test").await }
    });
    for _ in 0..200 {
        if gateway::is_running(&socket).await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (mut client, _) = gateway::Client::connect(&socket, "test", "acme").await.unwrap();
    for (id, role) in [(1, "pm"), (2, "designer"), (3, "qa")] {
        client.send(Command::Start { session: id, spec: SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some(role.into()), squad: Some("acme".into()) } });
    }
    async fn next(client: &mut gateway::Client) -> Event {
        tokio::time::timeout(Duration::from_secs(30), client.next_event()).await.expect("an event in time").expect("the daemon is there")
    }
    // Connected with Sign in, as the app's Connect does: the card comes up for the person.
    let mut docs = squad_mcp::server("docs", Some(&fake.url("/sse")), Some(&["designer".to_owned()]), Some("sse")).unwrap();
    docs.transport = McpTransport::Sse;
    client.send(Command::AddMcpServer { squad: "acme".into(), server: docs, sign_in: true });
    let page = loop {
        if let Event::CliSignInChanged { card, .. } = next(&mut client).await
            && let Some(url) = card.url.filter(|_| card.server.as_deref() == Some("docs"))
        {
            assert_eq!(card.member, "", "the person's own ask");
            break url;
        }
    };
    assert!(fake_mcp::browse(&page).await.contains("Signed in"));
    loop {
        if let Event::McpServers { status, .. } = next(&mut client).await
            && status.iter().any(|s| s.name == "docs" && s.overall() == McpState::Connected)
        {
            break;
        }
    }

    // The proxy's own connection: the designer's.
    async fn open(socket: &Path, member: &str) -> (tokio::io::Lines<tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>>, tokio::net::unix::OwnedWriteHalf, Result<(), String>) {
        use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
        let stream = tokio::net::UnixStream::connect(socket).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let open = McpProxyOpen { squad: "acme".into(), server: "docs".into(), member: member.into() };
        writer.write_all(Message::request(1, rpc::MCP_PROXY, open).to_line().as_bytes()).await.unwrap();
        let mut lines = tokio::io::BufReader::new(reader).lines();
        loop {
            let line = lines.next_line().await.unwrap().unwrap();
            let message = Message::from_line(&line).unwrap();
            if message.id == Some(1) {
                let answer = match message.error {
                    Some(e) => Err(e.message),
                    None => Ok(()),
                };
                return (lines, writer, answer);
            }
        }
    }
    let (_, _, refused) = open(&socket, "qa").await;
    assert!(refused.unwrap_err().contains("isn't connected for qa"), "qa doesn't get it");
    let (mut lines, mut writer, opened) = open(&socket, "designer").await;
    opened.unwrap();
    let send = |message: Value| Message::notification(rpc::MCP_PROXY_SEND, McpProxyMessage { message }).to_line();
    async fn recv(lines: &mut tokio::io::Lines<tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>>, id: u64) -> Value {
        let wait = async {
            loop {
                let line = lines.next_line().await.unwrap().unwrap();
                let message = Message::from_line(&line).unwrap();
                if message.method.as_deref() == Some(rpc::MCP_PROXY_RECV)
                    && let Some(McpProxyMessage { message }) = message.params_as::<McpProxyMessage>()
                    && message["id"] == id
                {
                    return message;
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(20), wait).await.expect("an answer in time")
    }
    use tokio::io::AsyncWriteExt as _;
    writer.write_all(send(request(1, "initialize", json!({}))).as_bytes()).await.unwrap();
    assert_eq!(recv(&mut lines, 1).await["result"]["protocolVersion"], "2025-06-18");
    writer.write_all(send(request(2, "tools/call", json!({ "name": "search_screens", "arguments": { "query": "pricing" } }))).as_bytes()).await.unwrap();
    assert_eq!(recv(&mut lines, 2).await["result"]["content"][0]["text"], "3 screens for pricing");
    assert!(fake.state.lock().unwrap().calls.last().is_some_and(|(_, c)| c.starts_with("at-")));
    drop(client);
    serving.abort();
}

#[tokio::test]
async fn a_keyed_server_needs_its_key_never_a_sign_in_and_the_key_stays_in_the_daemon() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    setup();
    let fake = Fake::start().await;
    let (claude, bin) = fake_claude();
    let project = scratch("proj").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    // 21st.dev's catalog entry, at the stand-in's address.
    let ui = McpServer { url: fake.url("/keyed"), ..squad_mcp::server("21st", None, None, None).unwrap() };
    assert!(matches!(ui.auth, McpAuth::Key { .. }));
    equipe_core::journal::Journal::for_squad(&home, "acme").unwrap().set_mcp_servers(std::slice::from_ref(&ui)).unwrap();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    for (id, role) in [(1, "pm"), (2, "frontend")] {
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some(role.into()), squad: Some("acme".into()) };
        handle.send(Command::Start { session: id, spec });
        until(&mut events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
    }
    handle.send(Command::CheckMcpServers { squad: "acme".into() });
    overall(&handle, "21st", McpState::NeedsKey).await;

    // Add key: straight to the key card — the steps and the page to create it.
    handle.send(Command::SignInMcpServer { squad: "acme".into(), name: "21st".into() });
    let seen = until(&mut events, |e| matches!(e, Event::SecretAsked { .. })).await;
    let Some(Event::SecretAsked { ask, .. }) = seen.last().cloned() else { panic!() };
    let store = ask.store.clone().unwrap();
    assert_eq!((store.name.as_str(), store.create_url.as_deref()), ("21st-api-key", Some("https://21st.dev/mcp")));
    assert!(handle.snapshot("acme").await.cli_sign_ins.is_empty(), "never a sign-in card");
    // Asked again: still one card.
    handle.send(Command::SignInMcpServer { squad: "acme".into(), name: "21st".into() });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(handle.snapshot("acme").await.secret_asks.len(), 1);
    let without = turn(&handle, &mut events, &bin, 2).await;
    assert!(servers(&without).get("21st").is_none(), "no key: not given");

    // Saved: connected at once (equipe's own look, with the key), no member turn needed.
    handle.send(Command::SaveSecret { squad: "acme".into(), id: ask.id, values: vec![equipe_core::protocol::secrets::SecretValue(fake_mcp::KEY.into())] });
    overall(&handle, "21st", McpState::Connected).await;
    let statuses = handle.snapshot("acme").await.mcp_status;
    assert!(statuses.iter().flat_map(|s| &s.engines).all(|e| e.state != McpState::NeedsSignIn), "a keyed server never needs a sign-in");

    // The member's turn: the proxy, not the key.
    let call = turn(&handle, &mut events, &bin, 2).await;
    assert!(!call.join("\n").contains(fake_mcp::KEY), "the key is never in a member's config or arguments");
    assert_eq!(servers(&call)["21st"]["args"][1], "proxy");
    assert!(!format!("{:?}", handle.snapshot("acme").await).contains(fake_mcp::KEY));
    // Its proxy's call carries the key, added by the daemon.
    let (to, mut from) = proxy_on(&handle, "frontend", "21st");
    to.send(request(7, "tools/call", json!({ "name": "search_screens", "arguments": { "query": "hero" } }))).unwrap();
    assert_eq!(reply_to(&mut from, 7).await["result"]["content"][0]["text"], "3 screens for hero");
    assert_eq!(fake.state.lock().unwrap().calls.last().map(|(_, c)| c.clone()), Some(format!("key {}", fake_mcp::KEY)));
    handle.shutdown().await;
}

#[tokio::test]
async fn a_sign_in_card_the_person_let_go_is_not_put_up_again_by_members_after_a_restart() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Fake::start().await;
    let (handle, mut events, bin, home, _) = squad_with(&[(2, "designer")]).await;
    let docs = squad_mcp::server("docs", Some(&fake.url("/mcp")), Some(&["designer".to_owned()]), None).unwrap();
    handle.send(Command::AddMcpServer { squad: "acme".into(), server: docs, sign_in: false });
    overall(&handle, "docs", McpState::NeedsSignIn).await;
    // A member's turn puts the card up once; the person lets it go.
    turn(&handle, &mut events, &bin, 2).await;
    let card = waiting_card(&handle, "equipe_mcp").await;
    handle.send(Command::CancelCliSignIn { squad: "acme".into(), id: card.id });
    for _ in 0..100 {
        if handle.snapshot("acme").await.cli_sign_ins.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    handle.shutdown().await;

    // equipe starts again: the member's turns don't put it up again; Connections still says it needs a sign-in.
    // (The squad comes back from its journal, the designer with it.)
    let (claude, bin) = fake_claude();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    turn(&handle, &mut events, &bin, 2).await;
    overall(&handle, "docs", McpState::NeedsSignIn).await;
    turn(&handle, &mut events, &bin, 2).await;
    assert!(handle.snapshot("acme").await.cli_sign_ins.iter().all(|c| c.tool != "equipe_mcp"), "not asked again by a member");
    // The person asks for it in Connections: the card, at once.
    handle.send(Command::SignInMcpServer { squad: "acme".into(), name: "docs".into() });
    waiting_card(&handle, "equipe_mcp").await;
    handle.shutdown().await;
}

#[tokio::test]
async fn the_pm_connects_a_server_only_on_the_persons_tap_and_takes_one_off_at_once() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Fake::start().await;
    let (handle, mut events, _bin, _home, _) = squad_with(&[(1, "pm"), (2, "designer")]).await;
    let url = fake.url("/mcp");
    // A member never connects one.
    let refused = tool(&handle, "designer", "change_setting", json!({ "setting": "mcp_server", "value": "add", "name": "docs", "url": url })).await;
    assert!(refused.is_error && refused.text.contains("Only the PM"), "{}", refused.text);

    let h = handle.clone();
    let asked_url = url.clone();
    let asking = tokio::spawn(async move { tool(&h, "pm", "change_setting", json!({ "setting": "mcp_server", "value": "add", "name": "docs", "url": asked_url, "roles": ["designer"], "why": "design references" })).await });
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = asked.last() else { unreachable!() };
    assert!(approval.action.contains("docs") && approval.action.contains(&url) && approval.action.contains("designer"), "{}", approval.action);
    assert!(!asked.iter().any(|e| matches!(e, Event::McpServers { .. })), "nothing before the tap");
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Allow, words: None, said: None });
    until(&mut events, |e| matches!(e, Event::McpServers { servers, .. } if servers.len() == 1)).await;
    assert!(asking.await.unwrap().text.starts_with("The person said yes"));
    // Asked with the sign-in: equipe's own card follows the look.
    waiting_card(&handle, "equipe_mcp").await;

    // Taking it off only narrows: at once.
    let off = tool(&handle, "pm", "change_setting", json!({ "setting": "mcp_server", "value": "remove", "name": "docs" })).await;
    assert!(!off.is_error && off.text.starts_with("Done:"), "{}", off.text);
    until(&mut events, |e| matches!(e, Event::McpServers { servers, .. } if servers.is_empty())).await;
    assert!(handle.snapshot("acme").await.cli_sign_ins.is_empty(), "its card goes with it");
    handle.shutdown().await;
}

#[tokio::test]
async fn the_engine_signed_fallback_signs_in_with_claude_codes_own_login_and_adds_idempotently() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (handle, mut events, bin, home, _) = squad_with(&[(1, "pm"), (2, "designer")]).await;
    let mobbin = McpServer { auth: McpAuth::Engine, ..squad_mcp::server("mobbin", None, None, None).unwrap() };
    // An entry an earlier equipe left in Claude Code's local config: reused, never "already exists".
    let signin = squad_mcp::signin_dir(&home, "acme");
    std::fs::create_dir_all(&signin).unwrap();
    std::fs::write(signin.join("added"), "https://api.mobbin.com/mcp\n").unwrap();
    handle.send(Command::AddMcpServer { squad: "acme".into(), server: mobbin.clone(), sign_in: false });
    overall(&handle, "mobbin", McpState::NeedsSignIn).await;
    let log = std::fs::read_to_string(bin.join("mcp.log")).unwrap();
    assert!(!log.contains("mcp add"), "already there: reused: {log}");

    // The designer's turn: by address, as the engine signed in for it, and one card running Claude Code's own login.
    let designer = turn(&handle, &mut events, &bin, 2).await;
    assert_eq!(servers(&designer)["mobbin"], json!({ "type": "http", "url": "https://api.mobbin.com/mcp" }));
    let card = waiting_card(&handle, "claude_mcp").await;
    assert_eq!((card.name.as_str(), card.member.as_str()), ("Mobbin (Claude Code)", "designer"));
    let log = std::fs::read_to_string(bin.join("mcp.log")).unwrap();
    assert!(log.lines().any(|l| l.ends_with("mcp login mobbin --no-browser") && l.starts_with(&signin.canonicalize().unwrap().display().to_string())), "{log}");

    // Removed: its card goes, and Claude Code's entry is cleared, so adding it again starts clean.
    handle.send(Command::RemoveMcpServer { squad: "acme".into(), name: "mobbin".into() });
    until(&mut events, |e| matches!(e, Event::McpServers { servers, .. } if servers.is_empty())).await;
    for _ in 0..100 {
        if !signin.join("added").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!signin.join("added").exists(), "the engine's local entry is gone");
    assert!(handle.snapshot("acme").await.cli_sign_ins.is_empty());
    handle.send(Command::AddMcpServer { squad: "acme".into(), server: mobbin, sign_in: false });
    overall(&handle, "mobbin", McpState::NeedsSignIn).await;
    assert!(!handle.snapshot("acme").await.mcp_status.iter().any(|s| s.detail().is_some_and(|d| d.contains("already exists"))));
    handle.shutdown().await;
}
