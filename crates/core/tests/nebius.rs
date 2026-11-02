//! Members on NVIDIA Nemotron through Nebius Token Factory, driven the way a
//! screen drives the runtime: which engine a member starts on, with and
//! without the key, and what its turns hand Codex.
//!
//! The first tests use the stand-in `codex` (the providers' `tests/fake-codex`).
//! The last, MOCK-VERIFIED and not a real Token Factory run, runs the person's
//! real installed `codex` (skipped without one) against a stand-in Responses
//! API on this machine with a dummy key: a member with nothing signed in
//! writes a file on Nemotron's say-so, and its work lands on a branch. No
//! Nebius call is made.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use equipe_core::protocol::{Access, Command, Engine, Event, SessionSpec, TurnOutcome};
use equipe_core::runtime::{self, Config, Handle};
use equipe_providers::CredentialStore;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::UnboundedReceiver;

const KEY: &str = "test-not-a-key";
const MODEL: &str = "nvidia/nemotron-mock-super";

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-nb-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn write_script(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The stand-in `codex`: what it is sent, and each run's command line.
fn fake_codex(env: &str) -> (PathBuf, PathBuf, PathBuf) {
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("../providers/tests/fake-codex/codex").canonicalize().unwrap();
    let logs = scratch("log");
    let (log, args) = (logs.join("sent.jsonl"), logs.join("args.jsonl"));
    let bin = scratch("bin").join("codex");
    write_script(&bin, &format!("#!/bin/sh\nFAKE_CODEX_LOG='{}' FAKE_CODEX_ARGS='{}' {env} exec '{}' \"$@\"\n", log.display(), args.display(), fake.display()));
    (bin, log, args)
}

/// A `claude` that isn't signed in.
fn no_claude() -> PathBuf {
    let bin = scratch("bin").join("claude");
    write_script(&bin, "#!/bin/sh\necho '{\"loggedIn\":false}'\n");
    bin
}

/// Token Factory as the daemon's environment would give it: the key, `base`, `model`.
fn token_factory(base: &str, model: Option<&str>) -> equipe_core::nemotron::Config {
    equipe_core::nemotron::Config::from_vars(|name| match name {
        "NEBIUS_API_KEY" => Some(KEY.into()),
        "EQUIPE_NEBIUS_BASE_URL" => Some(base.into()),
        "EQUIPE_NEBIUS_MODEL" => model.map(str::to_owned),
        // The stand-in here speaks Responses; chat is covered in the providers' tests.
        "EQUIPE_NEBIUS_WIRE" => Some("responses".into()),
        _ => None,
    })
    .unwrap()
}

fn config(codex: &Path, home: &Path, nemotron: Option<equipe_core::nemotron::Config>) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: no_claude().to_string_lossy().into_owned(),
        codex_binary: codex.to_string_lossy().into_owned(),
        persist: false,
        retention_days: 90,
        equipe_binary: None,
        npx: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        person_home: Some(home.join("person")),
        lang: equipe_core::i18n::Lang::En,
        nemotron,
    }
}

fn lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

/// The command lines `codex app-server` was run with, after `app-server`.
fn app_server_runs(args: &Path) -> Vec<(Vec<String>, bool)> {
    lines(args)
        .into_iter()
        .filter(|run| run["args"][0] == "app-server")
        .map(|run| {
            let argv = run["args"].as_array().unwrap().iter().skip(1).map(|a| a.as_str().unwrap().to_owned()).collect();
            (argv, run["nebius_key"] == true)
        })
        .collect()
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
    if tokio::time::timeout(Duration::from_secs(60), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, spec: SessionSpec) -> Vec<Event> {
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id) || matches!(e, Event::Error { .. })).await
}

async fn turn(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, text: &str) -> Vec<Event> {
    handle.send(Command::Submit { session: id, text: text.into(), about: None });
    until(events, |e| matches!(e, Event::TurnEnded { .. })).await
}

fn plain(cwd: &Path, access: Access) -> SessionSpec {
    SessionSpec { engine: None, model: None, access, cwd: cwd.to_owned(), role: None, squad: None }
}

fn has(argv: &[String], arg: &str) -> bool {
    argv.iter().any(|a| a == arg)
}

#[tokio::test]
async fn with_nothing_signed_in_a_token_factory_key_starts_members_on_nemotron() {
    let (codex, _, args) = fake_codex("FAKE_CODEX_SIGNED_OUT=1");
    let nemotron = token_factory("http://127.0.0.1:9/v1/", Some(MODEL));
    let (handle, mut events) = runtime::start(config(&codex, &scratch("home"), Some(nemotron)));
    let started = add(&handle, &mut events, 1, plain(&scratch("cwd"), Access::ReadOnly)).await;
    let Some(Event::SessionStarted { engine: Engine::Codex, model, .. }) = started.last() else { panic!("{started:#?}") };
    assert_eq!(model.as_deref(), Some(MODEL));
    let notice = until(&mut events, |e| matches!(e, Event::Notice { .. })).await;
    let Some(Event::Notice { text, .. }) = notice.last() else { panic!() };
    assert!(text.contains("NVIDIA Nemotron") && text.contains(MODEL) && text.contains("Nebius Token Factory"), "{text}");

    let ran = turn(&handle, &mut events, 1, "hello").await;
    assert!(matches!(ran.last(), Some(Event::TurnEnded { outcome: TurnOutcome::Completed, .. })), "{ran:#?}");
    // Billed per token: no plan's usage shown for it.
    assert!(!ran.iter().any(|e| matches!(e, Event::PlanUsage { .. })), "{ran:#?}");
    let runs = app_server_runs(&args);
    let (argv, key_in_env) = runs.last().expect("an app-server ran the turn");
    assert!(has(argv, "model_provider=\"nebius\""), "{argv:?}");
    // Its requests go through equipe's relay on the loopback, on to Token Factory.
    assert!(argv.iter().any(|a| a.starts_with("model_providers.nebius.base_url=\"http://127.0.0.1:") && !a.contains(":9\"")), "{argv:?}");
    assert!(has(argv, "model_providers.nebius.env_key=\"NEBIUS_API_KEY\"") && has(argv, "model_providers.nebius.wire_api=\"responses\""), "{argv:?}");
    assert!(*key_in_env, "the key reaches the app-server by its environment");
    assert!(!argv.iter().any(|a| a.contains(KEY)), "and never by its command line: {argv:?}");
}

#[tokio::test]
async fn with_nothing_signed_in_a_member_named_for_an_engine_starts_on_nemotron_too() {
    let (codex, _, _) = fake_codex("FAKE_CODEX_SIGNED_OUT=1");
    let nemotron = token_factory("http://127.0.0.1:9/v1/", Some(MODEL));
    let (handle, mut events) = runtime::start(config(&codex, &scratch("home"), Some(nemotron)));
    for (id, engine) in [(1, Engine::Claude), (2, Engine::Codex)] {
        let spec = SessionSpec { engine: Some(engine), ..plain(&scratch("cwd"), Access::ReadOnly) };
        let started = add(&handle, &mut events, id, spec).await;
        let Some(Event::SessionStarted { engine: Engine::Codex, model, .. }) = started.last() else { panic!("{started:#?}") };
        assert_eq!(model.as_deref(), Some(MODEL));
    }
}

#[tokio::test]
async fn with_another_engine_signed_in_a_signed_out_codex_still_says_how_to_sign_in() {
    let (codex, _, _) = fake_codex("FAKE_CODEX_SIGNED_OUT=1");
    let mut config = config(&codex, &scratch("home"), Some(token_factory("http://127.0.0.1:9/v1/", Some(MODEL))));
    let claude = scratch("bin").join("claude");
    write_script(&claude, "#!/bin/sh\necho '{\"loggedIn\":true,\"authMethod\":\"claude.ai\",\"subscriptionType\":\"max\"}'\n");
    config.claude_binary = claude.to_string_lossy().into_owned();
    let (handle, mut events) = runtime::start(config);
    let spec = SessionSpec { engine: Some(Engine::Codex), ..plain(&scratch("cwd"), Access::ReadOnly) };
    let failed = add(&handle, &mut events, 1, spec).await;
    let Some(Event::Error { message, .. }) = failed.last() else { panic!("{failed:#?}") };
    assert!(message.contains("Codex CLI isn't signed in"), "the person picks per member with /model: {message}");
    // Picked by model: on Nemotron, Claude signed in or not.
    let spec = SessionSpec { engine: Some(Engine::Codex), model: Some(MODEL.into()), ..plain(&scratch("cwd"), Access::ReadOnly) };
    let started = add(&handle, &mut events, 2, spec).await;
    assert!(matches!(started.last(), Some(Event::SessionStarted { engine: Engine::Codex, .. })), "{started:#?}");
}

#[tokio::test]
async fn without_a_key_nothing_signed_in_says_so_and_how() {
    let (codex, _, _) = fake_codex("FAKE_CODEX_SIGNED_OUT=1");
    let (handle, mut events) = runtime::start(config(&codex, &scratch("home"), None));
    let failed = add(&handle, &mut events, 1, plain(&scratch("cwd"), Access::ReadOnly)).await;
    let Some(Event::Error { message, .. }) = failed.last() else { panic!("{failed:#?}") };
    assert!(message.contains("No AI is ready") && message.contains("Nebius key") && message.contains("Settings › AI platforms"), "{message}");
    // On Codex by name, signed out, still: how to sign in.
    let spec = SessionSpec { engine: Some(Engine::Codex), ..plain(&scratch("cwd"), Access::ReadOnly) };
    let failed = add(&handle, &mut events, 2, spec).await;
    let Some(Event::Error { message, .. }) = failed.last() else { panic!("{failed:#?}") };
    assert!(message.contains("Codex CLI isn't signed in"), "{message}");
}

#[tokio::test]
async fn a_signed_in_codex_runs_as_before_with_a_key_present() {
    let (codex, _, args) = fake_codex("");
    let nemotron = token_factory("http://127.0.0.1:9/v1/", Some(MODEL));
    let (handle, mut events) = runtime::start(config(&codex, &scratch("home"), Some(nemotron)));
    let started = add(&handle, &mut events, 1, plain(&scratch("cwd"), Access::ReadOnly)).await;
    let Some(Event::SessionStarted { engine: Engine::Codex, model, .. }) = started.last() else { panic!("{started:#?}") };
    assert_eq!(model.as_deref(), Some("gpt-6.1-sol"), "the ChatGPT account's own default");
    let ran = turn(&handle, &mut events, 1, "hello").await;
    assert!(ran.iter().any(|e| matches!(e, Event::PlanUsage { engine: Engine::Codex, .. })), "its plan's usage, as before");
    let runs = app_server_runs(&args);
    assert!(!runs.is_empty() && runs.iter().all(|(argv, _)| argv.is_empty()), "the person's own Codex, as they set it up: {runs:?}");
}

#[tokio::test]
async fn a_member_moved_to_an_nvidia_model_runs_its_next_turn_on_token_factory() {
    let (codex, log, args) = fake_codex("");
    let nemotron = token_factory("http://127.0.0.1:9/v1/", None);
    let (handle, mut events) = runtime::start(config(&codex, &scratch("home"), Some(nemotron)));
    add(&handle, &mut events, 1, plain(&scratch("cwd"), Access::ReadOnly)).await;
    // `/model nvidia/…`, and an effort Token Factory doesn't take.
    handle.send(Command::SetModel { session: 1, model: MODEL.into() });
    handle.send(Command::SetEffort { session: 1, effort: Some("ultra".into()) });
    // (Reading the ChatGPT plan's usage runs an app-server of its own now and then, without a provider.)
    let before = app_server_runs(&args).len();
    turn(&handle, &mut events, 1, "hello").await;
    let runs = app_server_runs(&args);
    assert!(runs[before..].iter().any(|(argv, _)| has(argv, "model_provider=\"nebius\"")), "{runs:?}");
    let start = lines(&log).into_iter().rfind(|m| m["method"] == "turn/start").unwrap();
    assert_eq!(start["params"]["model"], MODEL);
    assert!(start["params"].get("effort").is_none(), "{start}");
    // Back to OpenAI's: back to the person's own Codex.
    handle.send(Command::SetModel { session: 1, model: "gpt-6.1-sol".into() });
    let before = app_server_runs(&args).len();
    turn(&handle, &mut events, 1, "hello").await;
    let runs = app_server_runs(&args);
    assert!(runs.len() > before && runs[before..].iter().all(|(argv, _)| argv.is_empty()), "{runs:?}");
}

// ── the real codex, on a stand-in Token Factory ──

/// A stand-in Responses API: the turn's first request gets a function call
/// writing `made.txt`, the one carrying its output the final message.
async fn stand_in() -> (String, Arc<Mutex<Vec<(Vec<(String, String)>, Value)>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}/v1/", listener.local_addr().unwrap().port());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                loop {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 16 * 1024];
                    let end = loop {
                        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break at;
                        }
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
                    let headers: Vec<(String, String)> = head.split("\r\n").skip(1).filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned())).collect();
                    let length: usize = headers.iter().find(|(n, _)| n == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
                    let mut body = buf[end + 4..].to_vec();
                    while body.len() < length {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => body.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    let n = {
                        let mut log = log.lock().unwrap();
                        log.push((headers, body.clone()));
                        log.len()
                    };
                    let answered = body["input"].as_array().into_iter().flatten().any(|i| i["type"] == "function_call_output");
                    let has_tool = body["tools"].as_array().into_iter().flatten().any(|t| t["name"] == "exec_command");
                    let item = if has_tool && !answered {
                        json!({ "type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "exec_command", "arguments": json!({ "cmd": "printf 'fn main() {}\\n' > made.rs" }).to_string(), "status": "completed" })
                    } else {
                        json!({ "type": "message", "id": format!("msg_{n}"), "role": "assistant", "status": "completed", "content": [{ "type": "output_text", "text": "Wrote made.rs.", "annotations": [] }] })
                    };
                    let id = format!("resp_{n}");
                    let usage = json!({ "input_tokens": 100, "input_tokens_details": { "cached_tokens": 0 }, "output_tokens": 10, "output_tokens_details": { "reasoning_tokens": 0 }, "total_tokens": 110 });
                    let events = [
                        json!({ "type": "response.created", "response": { "id": id, "object": "response", "status": "in_progress", "output": [] } }),
                        json!({ "type": "response.output_item.done", "output_index": 0, "item": item }),
                        json!({ "type": "response.completed", "response": { "id": id, "object": "response", "status": "completed", "output": [item], "usage": usage } }),
                    ];
                    let sse: String = events.iter().map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap())).collect();
                    let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{sse}", sse.len());
                    if stream.write_all(reply.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (base, seen)
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn repo() -> PathBuf {
    let dir = scratch("repo");
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

#[tokio::test]
async fn mock_verified_a_member_with_nothing_signed_in_writes_code_on_nemotron_through_the_real_codex() {
    let binary = std::env::var("EQUIPE_CODEX").ok().filter(|b| !b.is_empty()).unwrap_or_else(|| "codex".to_owned());
    let Some(real) = equipe_providers::codex_app::find(&binary) else {
        eprintln!("skipped: no `codex` installed (or $EQUIPE_CODEX)");
        return;
    };
    // The real codex, in a home of its own: no ChatGPT sign-in, none of the person's config.
    let codex_home = scratch("codex-home");
    let codex = scratch("bin").join("codex");
    write_script(&codex, &format!("#!/bin/sh\nCODEX_HOME='{}' exec '{}' \"$@\"\n", codex_home.display(), real.display()));
    let (base, seen) = stand_in().await;
    let (handle, mut events) = runtime::start(config(&codex, &scratch("home"), Some(token_factory(&base, Some(MODEL)))));
    let repo = repo();
    let started = add(&handle, &mut events, 1, plain(&repo, Access::EditFiles)).await;
    let Some(Event::SessionStarted { engine: Engine::Codex, model, workspace: Some(workspace), .. }) = started.last() else { panic!("{started:#?}") };
    assert_eq!(model.as_deref(), Some(MODEL));

    let ran = turn(&handle, &mut events, 1, "Add made.rs.").await;
    let requests = seen.lock().unwrap().clone();
    let shapes: Vec<Value> = requests.iter().map(|(h, b)| json!({ "authorization_is_dummy": h.iter().any(|(n, v)| n == "authorization" && v == &format!("Bearer {KEY}")), "model": b["model"], "tools": b["tools"].as_array().map(|t| t.iter().map(|t| t["name"].clone()).collect::<Vec<_>>()), "input": b["input"].as_array().map(|i| i.iter().map(|i| i["type"].clone()).collect::<Vec<_>>()) })).collect();
    let context = format!("\nrequests: {shapes:#?}\nevents: {ran:#?}");
    eprintln!("MOCK-VERIFIED (not a real Token Factory run): {shapes:#?}");
    assert!(matches!(ran.last(), Some(Event::TurnEnded { outcome: TurnOutcome::Completed, .. })), "{context}");
    assert!(ran.iter().any(|e| matches!(e, Event::ToolResult { tool, outcome, .. } if tool == "Bash" && outcome.ok)), "{context}");
    assert_eq!(std::fs::read_to_string(workspace.path.join("made.rs")).ok().as_deref(), Some("fn main() {}\n"), "{context}");
    assert!(requests.len() >= 2 && requests.iter().all(|(h, b)| h.iter().any(|(n, v)| n == "authorization" && v == &format!("Bearer {KEY}")) && b["model"] == MODEL), "{context}");

    // Its work lands on a branch, as any member's does.
    handle.send(Command::Close { session: 1 });
    let closed = until(&mut events, |e| matches!(e, Event::SessionClosed { .. })).await;
    let branch = closed.iter().find_map(|e| if let Event::BranchLanded { branch, .. } = e { Some(branch.clone()) } else { None }).expect("a branch");
    assert_eq!(git(&repo, &["show", &format!("{branch}:made.rs")]), "fn main() {}");
    let _ = std::fs::remove_dir_all(&codex_home);
}
