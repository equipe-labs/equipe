//! A squad's outside MCP servers, driven as the person's screen and the PM
//! drive them, with a stand-in `claude` that answers turns and its own `mcp`
//! subcommands: a server is connected (by the person, or by the PM only on
//! the person's tap), reaches only the members of its roles — name and
//! address, never a token, `--strict-mcp-config` kept — and a member who
//! needs its sign-in puts exactly one sign-in card in front of the person,
//! running the engine's own `claude mcp login` in the squad's sign-in folder.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::cli_signin::Phase;
use equipe_core::protocol::mcp_servers::McpState;
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Decision, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::{squad_mcp, squads};
use equipe_providers::CredentialStore;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-squad-mcp-it-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A stand-in `claude`: signed in; its `mcp get` says a server added in the
/// folder it runs in needs authentication; its `mcp login` prints the page
/// and waits for the redirect; every turn's arguments are written down in
/// `calls/` and answered "ok".
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
    add) echo "$8" > added ;;
    remove) rm -f added ;;
    login) echo 'Visit this URL to authorize:'; echo '  https://auth.example.com/oauth/authorize?client_id=x&redirect_uri=http%3A%2F%2Flocalhost%3A5555%2Fcallback '; printf 'Or paste the redirect URL here: '; exec sleep 60 ;;
  esac
  exit 0
fi
n=$(ls '{calls}' | wc -l | tr -d ' ')
for a in "$@"; do printf '%s\n<<arg>>\n' "$a"; done > '{calls}/call-'$(printf %03d $n)-$$
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
    let mut files: Vec<_> = std::fs::read_dir(dir.join("calls")).unwrap().flatten().map(|e| e.path()).collect();
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

async fn tool(handle: &Handle, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
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

fn servers(call: &[String]) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(arg(call, "--mcp-config").expect("an MCP config")).unwrap()["mcpServers"].clone()
}

/// The squad's MCP sign-in card once it shows its page (its events may have gone by during a turn).
async fn waiting_card(handle: &Handle) -> equipe_core::protocol::cli_signin::CliSignIn {
    for _ in 0..200 {
        if let Some(card) = handle.snapshot("acme").await.cli_sign_ins.into_iter().find(|c| c.tool == "claude_mcp" && c.phase == Phase::Waiting) {
            return card;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no MCP sign-in card showed its page");
}

fn mcp_cards(events: &[Event]) -> Vec<u64> {
    let mut ids: Vec<u64> = events.iter().filter_map(|e| match e { Event::CliSignInChanged { card, .. } if card.tool == "claude_mcp" => Some(card.id), _ => None }).collect();
    ids.dedup();
    ids
}

#[tokio::test]
async fn a_connected_server_reaches_only_its_roles_and_one_card_asks_for_its_sign_in() {
    let (claude, bin) = fake_claude();
    let project = scratch("proj").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    for (id, role) in [(1, "pm"), (2, "designer"), (3, "qa")] {
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some(role.into()), squad: Some("acme".into()) };
        handle.send(Command::Start { session: id, spec });
        until(&mut events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
    }

    // No server yet: nobody is told of one, nothing outside reaches anyone.
    let before = turn(&handle, &mut events, &bin, 2).await;
    assert!(!before.join("\n").contains("Outside MCP servers"));
    assert_eq!(servers(&before).as_object().unwrap().keys().filter(|k| k.as_str() == "mobbin").count(), 0);

    // The person connects Mobbin: kept, shown, and checked through Claude Code's own `mcp get`.
    let mobbin = squad_mcp::server("mobbin", None, None, None).unwrap();
    handle.send(Command::AddMcpServer { squad: "acme".into(), server: mobbin.clone(), sign_in: false });
    let seen = until(&mut events, |e| matches!(e, Event::McpServers { status, .. } if status.first().is_some_and(|s| s.overall() == McpState::NeedsSignIn))).await;
    assert!(seen.iter().any(|e| matches!(e, Event::McpServers { servers, .. } if servers == &vec![mobbin.clone()])));
    assert!(mcp_cards(&seen).is_empty(), "not asked for: no card");
    let log = std::fs::read_to_string(bin.join("mcp.log")).unwrap();
    let signin = squad_mcp::signin_dir(&home, "acme");
    assert!(log.contains(&format!("{} mcp add --scope local --transport http mobbin https://api.mobbin.com/mcp", signin.canonicalize().unwrap().display())), "{log}");
    let snapshot = handle.snapshot("acme").await;
    assert_eq!((snapshot.mcp_servers.len(), snapshot.mcp_status[0].overall()), (1, McpState::NeedsSignIn));

    // The designer's next turn: Mobbin by name and address, its tools named, nothing else of it.
    let designer = turn(&handle, &mut events, &bin, 2).await;
    assert_eq!(servers(&designer)["mobbin"], serde_json::json!({ "type": "http", "url": "https://api.mobbin.com/mcp" }));
    assert!(designer.iter().any(|a| a == "--strict-mcp-config"), "the person's own servers stay out");
    let config = arg(&designer, "--mcp-config").unwrap().to_ascii_lowercase();
    for word in ["token", "authorization", "bearer", "headers"] {
        assert!(!config.contains(word), "{word} in the member's MCP config: {config}");
    }
    assert!(designer.join("\n").contains("`mobbin` (Mobbin: search real product screens and flows for design references; tools `mcp__mobbin__*`)"), "the designer is told of it");
    // It needs a sign-in: one card, the engine's own login, its page printed.
    let card = waiting_card(&handle).await;
    assert_eq!((card.name.as_str(), card.member.as_str(), card.wants_code), ("Mobbin (Claude Code)", "designer", true));
    assert!(card.url.as_deref().is_some_and(|u| u.starts_with("https://auth.example.com/oauth/authorize")), "{:?}", card.url);
    let log = std::fs::read_to_string(bin.join("mcp.log")).unwrap();
    assert!(log.lines().any(|l| l.ends_with("mcp login mobbin --no-browser") && l.starts_with(&signin.canonicalize().unwrap().display().to_string())), "{log}");

    // QA isn't one of its roles: nothing of it.
    let qa = turn(&handle, &mut events, &bin, 3).await;
    assert!(servers(&qa).get("mobbin").is_none() && !qa.join("\n").contains("Outside MCP servers"));

    // Again: no second card.
    let again = turn(&handle, &mut events, &bin, 2).await;
    assert!(servers(&again).get("mobbin").is_some());
    assert_eq!(handle.snapshot("acme").await.cli_sign_ins.iter().filter(|c| c.tool == "claude_mcp").count(), 1);

    // Put off: closed, and not asked again on the designer's turns until the person asks.
    handle.send(Command::CancelCliSignIn { squad: "acme".into(), id: card.id });
    until(&mut events, |e| matches!(e, Event::CliSignInClosed { id, .. } if *id == card.id)).await;
    turn(&handle, &mut events, &bin, 2).await;
    assert!(handle.snapshot("acme").await.cli_sign_ins.is_empty(), "put off: not asked again");
    handle.send(Command::SignInMcpServer { squad: "acme".into(), name: "mobbin".into() });
    let theirs = waiting_card(&handle).await;
    assert_eq!(theirs.member, "", "the person's own ask");

    // Removed: its card goes, members stop getting it, and Claude Code's entry (and sign-in) is cleared.
    handle.send(Command::RemoveMcpServer { squad: "acme".into(), name: "mobbin".into() });
    until(&mut events, |e| matches!(e, Event::McpServers { servers, .. } if servers.is_empty())).await;
    let after = turn(&handle, &mut events, &bin, 2).await;
    assert!(servers(&after).get("mobbin").is_none());
    assert!(handle.snapshot("acme").await.cli_sign_ins.is_empty());
    handle.shutdown().await;
}

#[tokio::test]
async fn the_pm_connects_a_server_only_on_the_persons_tap_and_takes_one_off_at_once() {
    let (claude, _bin) = fake_claude();
    let project = scratch("proj").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    for (id, role) in [(1, "pm"), (2, "designer")] {
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some(role.into()), squad: Some("acme".into()) };
        handle.send(Command::Start { session: id, spec });
        until(&mut events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
    }
    // A member never connects one.
    let refused = tool(&handle, "designer", "change_setting", serde_json::json!({ "setting": "mcp_server", "value": "add", "name": "mobbin" })).await;
    assert!(refused.is_error && refused.text.contains("Only the PM"), "{}", refused.text);

    let h = handle.clone();
    let asking = tokio::spawn(async move { tool(&h, "pm", "change_setting", serde_json::json!({ "setting": "mcp_server", "value": "add", "name": "mobbin", "why": "they bought Mobbin Pro" })).await });
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = asked.last() else { unreachable!() };
    assert!(approval.action.contains("Mobbin") && approval.action.contains("https://api.mobbin.com/mcp") && approval.action.contains("designer, frontend"), "{}", approval.action);
    assert!(!asked.iter().any(|e| matches!(e, Event::McpServers { .. })), "nothing before the tap");
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Allow, words: None, said: None });
    until(&mut events, |e| matches!(e, Event::McpServers { servers, .. } if servers.len() == 1)).await;
    assert!(asking.await.unwrap().text.starts_with("The person said yes"));
    // Asked with the sign-in: its card follows the check.
    until(&mut events, |e| matches!(e, Event::CliSignInChanged { card, .. } if card.tool == "claude_mcp")).await;
    let read = tool(&handle, "pm", "settings", serde_json::json!({})).await;
    assert!(read.text.contains("outside MCP server mobbin (https://api.mobbin.com/mcp) for designer, frontend: on"), "{}", read.text);

    // Taking it off only narrows: at once.
    let off = tool(&handle, "pm", "change_setting", serde_json::json!({ "setting": "mcp_server", "value": "remove", "name": "mobbin" })).await;
    assert!(!off.is_error && off.text.starts_with("Done:"), "{}", off.text);
    until(&mut events, |e| matches!(e, Event::McpServers { servers, .. } if servers.is_empty())).await;
    handle.shutdown().await;
}

#[tokio::test]
async fn a_keyed_server_asks_for_its_key_once_and_its_header_reaches_the_member_only_from_a_private_file() {
    // SAFETY: set before the runtime starts; the other tests here never read it.
    unsafe { std::env::set_var("EQUIPE_VAULT_IN_MEMORY", "1") };
    const KEY: &str = "21st_sk_test_0123456789abcdef";
    let (claude, bin) = fake_claude();
    let project = scratch("proj").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    for (id, role) in [(1, "pm"), (2, "frontend")] {
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some(role.into()), squad: Some("acme".into()) };
        handle.send(Command::Start { session: id, spec });
        until(&mut events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
    }

    // Connected with its sign-in: no key yet, so the key card — the catalog's page and steps.
    let ui = squad_mcp::server("21st", None, None, None).unwrap();
    handle.send(Command::AddMcpServer { squad: "acme".into(), server: ui.clone(), sign_in: true });
    let seen = until(&mut events, |e| matches!(e, Event::SecretAsked { .. })).await;
    let Some(Event::SecretAsked { ask, .. }) = seen.last().cloned() else { panic!() };
    let store = ask.store.clone().unwrap();
    assert_eq!((store.name.as_str(), store.create_url.as_deref()), ("21st-api-key", Some("https://21st.dev/mcp")));
    // Without it the member doesn't get the server, and no second card comes up.
    let before = turn(&handle, &mut events, &bin, 2).await;
    assert!(servers(&before).get("21st").is_none());
    assert_eq!(handle.snapshot("acme").await.secret_asks.len(), 1);

    // Saved: the next turn has it, its header from a file only the person can read — not on the command line.
    handle.send(Command::SaveSecret { squad: "acme".into(), id: ask.id, values: vec![equipe_core::protocol::secrets::SecretValue(KEY.into())] });
    until(&mut events, |e| matches!(e, Event::SquadSecrets { secrets } if !secrets.is_empty())).await;
    let call = turn(&handle, &mut events, &bin, 2).await;
    assert!(!call.join("\n").contains(KEY), "the key is never an argument");
    let path = PathBuf::from(arg(&call, "--mcp-config").unwrap());
    assert!(path.starts_with(home.join("credentials")), "{}", path.display());
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(config["mcpServers"]["21st"], serde_json::json!({ "type": "http", "url": "https://21st.dev/api/mcp", "headers": { "x-api-key": KEY } }));
    assert!(config["mcpServers"]["equipe"].is_object(), "equipe's own servers with it");
    assert!(call.iter().any(|a| a == "--strict-mcp-config"));
    assert!(call.join("\n").contains("`21st` (21st.dev: proven React, Tailwind and shadcn/ui components to adapt"), "the member is told what it is for");
    // Nothing a screen sees carries it.
    let snapshot = handle.snapshot("acme").await;
    assert!(!format!("{snapshot:?}").contains(KEY));
    assert_eq!(snapshot.mcp_status[0].overall(), McpState::Connected, "looked at again once its key was kept");

    // Removed: the private file goes with the next turn.
    handle.send(Command::RemoveMcpServer { squad: "acme".into(), name: "21st".into() });
    until(&mut events, |e| matches!(e, Event::McpServers { servers, .. } if servers.is_empty())).await;
    let after = turn(&handle, &mut events, &bin, 2).await;
    assert!(servers(&after).get("21st").is_none() && !path.exists());
    handle.shutdown().await;
}
