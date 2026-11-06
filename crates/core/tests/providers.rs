//! Model providers the person connects from a screen — added, listed,
//! removed — and members set to one of their models, driven the way a
//! screen drives the runtime. Against a stand-in OpenAI-compatible server on
//! this machine and the stand-in `codex` (the providers' `tests/fake-codex`):
//! no provider is reached, and no key is real.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use equipe_core::protocol::{Access, Command, Engine, Event, ProviderKey, ProviderStatus, ProviderTest, SessionSpec, TurnOutcome, secrets::SecretValue};
use equipe_core::runtime::{self, Config, Handle};
use equipe_providers::CredentialStore;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::UnboundedReceiver;

const KEY: &str = "sk-test-not-a-real-key-42";

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-pv-{name}-{}-{n}", std::process::id()));
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

fn no_claude() -> PathBuf {
    let bin = scratch("bin").join("claude");
    write_script(&bin, "#!/bin/sh\necho '{\"loggedIn\":false}'\n");
    bin
}

fn config(codex: &Path, home: &Path) -> Config {
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

/// A stand-in OpenAI-compatible server answering `GET /models` with
/// `status` and `body`: its base URL, and the requests' heads.
async fn server(status: u16, body: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let kept = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let kept = kept.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16_384];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                kept.lock().unwrap().push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let head = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(body.as_bytes()).await;
            });
        }
    });
    (base, seen)
}

const MODELS: &str = r#"{"object":"list","data":[{"id":"qwen/qwen3-coder"},{"id":"text-embedding-3-small"},{"id":"moonshotai/kimi-k2"}]}"#;

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

async fn providers(handle: &Handle, events: &mut UnboundedReceiver<Event>) -> Vec<ProviderStatus> {
    handle.send(Command::Providers);
    let seen = until(events, |e| matches!(e, Event::Providers { .. })).await;
    let Some(Event::Providers { providers, .. }) = seen.last() else { unreachable!() };
    providers.clone()
}

async fn tested(events: &mut UnboundedReceiver<Event>, id: &str) -> ProviderTest {
    let seen = until(events, |e| matches!(e, Event::ProviderTested { id: i, test } if i == id && *test != ProviderTest::Testing)).await;
    let Some(Event::ProviderTested { test, .. }) = seen.last() else { unreachable!() };
    test.clone()
}

/// What a screen sees, as text: the key is never in it.
fn shown(events: &[Event]) -> String {
    events.iter().map(|e| serde_json::to_string(e).unwrap()).collect::<Vec<_>>().join("\n")
}

#[tokio::test]
async fn a_provider_added_from_a_screen_is_kept_listed_and_removed_and_its_key_never_comes_back() {
    let (codex, _, _) = fake_codex("");
    let home = scratch("home");
    let (base, seen) = server(200, MODELS).await;
    let (handle, mut events) = runtime::start(config(&codex, &home));
    assert!(providers(&handle, &mut events).await.is_empty());

    handle.send(Command::AddProvider { preset: "other".into(), base_url: Some(base.clone()), key: Some(SecretValue(KEY.into())) });
    let added = until(&mut events, |e| matches!(e, Event::Providers { providers, .. } if !providers.is_empty())).await;
    let Some(Event::Providers { providers: list, codex: true }) = added.last() else { panic!("{added:#?}") };
    let id = list[0].id.clone();
    assert!(id.starts_with("other-127-0-0-1-"), "{id}");
    assert_eq!((list[0].key, list[0].removable, list[0].base_url.as_str()), (ProviderKey::Saved, true, base.as_str()));
    // Listed at once: its models a member can work with, by the ids a member is set to.
    assert_eq!(tested(&mut events, &id).await, ProviderTest::Ok { count: 2 });
    let after = until(&mut events, |e| matches!(e, Event::Providers { .. })).await;
    let Some(Event::Providers { providers: list, .. }) = after.last() else { unreachable!() };
    let models: Vec<&str> = list[0].models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(models, [format!("@{id}/qwen/qwen3-coder"), format!("@{id}/moonshotai/kimi-k2")]);
    let request = seen.lock().unwrap()[0].clone();
    assert!(request.starts_with("GET /v1/models ") && request.to_ascii_lowercase().contains(&format!("authorization: bearer {}", KEY.to_ascii_lowercase())), "{request}");
    // The key: in a file only the person can read, never in what screens are told.
    use std::os::unix::fs::PermissionsExt as _;
    let key_file = home.join("keys").join(&id);
    assert_eq!(std::fs::read_to_string(&key_file).unwrap(), KEY);
    assert_eq!(std::fs::metadata(&key_file).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(!std::fs::read_to_string(home.join("providers.json")).unwrap().contains(KEY));
    assert!(!shown(&added).contains(KEY) && !shown(&after).contains(KEY));

    handle.send(Command::RemoveProvider { id: id.clone() });
    let gone = until(&mut events, |e| matches!(e, Event::Providers { providers, .. } if providers.is_empty())).await;
    assert!(!shown(&gone).contains(KEY));
    assert!(!key_file.exists());
}

#[tokio::test]
async fn a_provider_that_refuses_the_key_or_isnt_there_says_so_in_plain_words() {
    let (codex, _, _) = fake_codex("");
    let home = scratch("home");
    let (refusing, _) = server(401, r#"{"error":{"message":"invalid api key"}}"#).await;
    let (handle, mut events) = runtime::start(config(&codex, &home));
    handle.send(Command::AddProvider { preset: "other".into(), base_url: Some(refusing.clone()), key: Some(SecretValue(KEY.into())) });
    let id = until(&mut events, |e| matches!(e, Event::Providers { providers, .. } if !providers.is_empty())).await;
    let Some(Event::Providers { providers, .. }) = id.last() else { unreachable!() };
    let id = providers[0].id.clone();
    let ProviderTest::Failed { why } = tested(&mut events, &id).await else { panic!() };
    assert!(why.contains("didn't accept the key") && !why.contains(KEY), "{why}");
    // Nothing listening there: it can't be reached, and says where it looked.
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/v1", listener.local_addr().unwrap())
    };
    handle.send(Command::AddProvider { preset: "lmstudio".into(), base_url: Some(closed.clone()), key: None });
    let ProviderTest::Failed { why } = tested(&mut events, "lmstudio").await else { panic!() };
    assert!(why.contains("Couldn't reach LM Studio") && why.contains(&closed), "{why}");
    // What can't be added says why, before anything is kept.
    handle.send(Command::AddProvider { preset: "groq".into(), base_url: None, key: None });
    let ProviderTest::Failed { why } = tested(&mut events, "groq").await else { panic!() };
    assert!(why.contains("Groq needs an API key"), "{why}");
    assert!(!home.join("keys").join("groq").exists());
}

#[tokio::test]
async fn a_nebius_key_saved_before_is_the_nebius_entry() {
    let (codex, _, _) = fake_codex("");
    let home = scratch("home");
    equipe_core::nemotron::save_key(&home, "sk-nebius-from-before-1").unwrap();
    let (handle, mut events) = runtime::start(config(&codex, &home));
    handle.send(Command::Providers);
    let seen = until(&mut events, |e| matches!(e, Event::Providers { .. })).await;
    let Some(Event::Providers { providers, .. }) = seen.last() else { unreachable!() };
    let nebius = providers.iter().find(|p| p.id == "nebius").expect("its entry");
    assert_eq!((nebius.name.as_str(), nebius.key, nebius.base_url.as_str()), ("Nebius Token Factory", ProviderKey::Saved, equipe_core::nemotron::DEFAULT_BASE_URL));
    // Forgotten from the old card: off the list too.
    handle.send(Command::SetNebiusKey { key: None });
    until(&mut events, |e| matches!(e, Event::Providers { providers, .. } if providers.iter().all(|p| p.id != "nebius"))).await;
}

fn lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[tokio::test]
async fn a_member_set_to_a_providers_model_runs_on_it_through_codexs_agent_with_its_key_by_the_environment_only() {
    let (codex, log, args) = fake_codex("FAKE_CODEX_SIGNED_OUT=1");
    let home = scratch("home");
    let (base, _) = server(200, MODELS).await;
    let id = equipe_core::model_providers::add(&home, "other", Some(&base), Some(KEY)).unwrap();
    let model = equipe_core::model_providers::model_ref(&id, "qwen/qwen3-coder");
    let (handle, mut events) = runtime::start(config(&codex, &home));
    let spec = SessionSpec { engine: None, model: Some(model.clone()), access: Access::ReadOnly, cwd: scratch("cwd"), role: None, squad: None };
    handle.send(Command::Start { session: 1, spec });
    let started = until(&mut events, |e| matches!(e, Event::SessionStarted { .. } | Event::Error { .. })).await;
    let Some(Event::SessionStarted { engine: Engine::Codex, model: Some(on), .. }) = started.last() else { panic!("{started:#?}") };
    assert_eq!(on, &model);
    // Said plainly: on what, and what it can't do there yet.
    let notices = until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("subagents"))).await;
    let texts: Vec<&str> = notices.iter().filter_map(|e| if let Event::Notice { text, .. } = e { Some(text.as_str()) } else { None }).collect();
    assert!(texts.iter().any(|t| t.contains("qwen/qwen3-coder") && t.contains("127.0.0.1")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("no subagents or web search") && t.contains("can't look at images")), "{texts:?}");

    handle.send(Command::Submit { session: 1, text: "hello".into(), about: None });
    let ran = until(&mut events, |e| matches!(e, Event::TurnEnded { .. })).await;
    assert!(matches!(ran.last(), Some(Event::TurnEnded { outcome: TurnOutcome::Completed, .. })), "{ran:#?}");
    assert!(!ran.iter().any(|e| matches!(e, Event::PlanUsage { .. })), "billed per token: no plan");
    let run = lines(&args).into_iter().rfind(|r| r["args"][0] == "app-server").expect("an app-server ran the turn");
    let argv: Vec<String> = run["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_owned()).collect();
    let codex_id = equipe_core::model_providers::codex_id(&id);
    let var = format!("EQUIPE_{}_API_KEY", id.to_ascii_uppercase().replace('-', "_"));
    assert!(argv.iter().any(|a| a == &format!("model_provider=\"{codex_id}\"")), "{argv:?}");
    assert!(argv.iter().any(|a| a == &format!("model_providers.{codex_id}.env_key=\"{var}\"")), "{argv:?}");
    assert!(argv.iter().any(|a| a.starts_with(&format!("model_providers.{codex_id}.base_url=\"http://127.0.0.1:"))), "through equipe's relay: {argv:?}");
    assert!(argv.iter().any(|a| a == "features.multi_agent=false"), "plain: no subagents");
    assert!(run["keys"].as_array().unwrap().iter().any(|k| k == var.as_str()), "the key reaches it by its environment: {run}");
    assert!(!argv.iter().any(|a| a.contains(KEY)), "never by its command line");
    // The provider is asked for the model by its own id.
    let start = lines(&log).into_iter().rfind(|m| m["method"] == "thread/start" || m["method"] == "turn/start").unwrap();
    assert_eq!(start["params"]["model"], "qwen/qwen3-coder", "{start}");
}

#[tokio::test]
async fn a_member_set_to_a_provider_that_isnt_connected_says_how_to_connect_it() {
    let (codex, _, _) = fake_codex("FAKE_CODEX_SIGNED_OUT=1");
    let (handle, mut events) = runtime::start(config(&codex, &scratch("home")));
    let spec = SessionSpec { engine: None, model: Some("@groq/llama-3.3-70b-versatile".into()), access: Access::ReadOnly, cwd: scratch("cwd"), role: None, squad: None };
    handle.send(Command::Start { session: 1, spec });
    let failed = until(&mut events, |e| matches!(e, Event::SessionStarted { .. } | Event::Error { .. })).await;
    let Some(Event::Error { message, .. }) = failed.last() else { panic!("{failed:#?}") };
    assert!(message.contains("Groq isn't connected") && message.contains("Settings"), "{message}");
}
