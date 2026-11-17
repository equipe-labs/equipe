//! Connecting outside services (`equipe_core::connections`): a member's
//! `request_secret` card and the person's paste into the squad's secret
//! store, the same card started from Logins › Connections, the card's trip
//! over a remote connection, a service's status, and Disconnect. The value
//! is in no event, no file of the squad's home and no log. The secret store
//! is a fake in memory (`EQUIPE_VAULT_IN_MEMORY`): nothing here touches the Keychain.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::connections;
use equipe_core::protocol::rpc::ToolCall;
use equipe_core::protocol::secrets::{SecretKind, SecretValue};
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A value no other test in this binary (or any other) has.
const VALUE: &str = "sk-ant-admin01-Qm4vT8xK2pLw9RzN6bY3dHs7cJ1fGa5e";

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-squad-secrets-it-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A `claude` that says it is signed in and, for a turn, writes its environment where the test reads it.
fn recording_claude(env_file: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = scratch("bin").join("claude");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\nenv > '{}'\nexec sleep 30\n",
        env_file.display()
    );
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
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
        npx: Some("/nonexistent/npx".into()),
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
    if tokio::time::timeout(Duration::from_secs(15), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).env_remove("GIT_DIR").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A git repository with `.env.local` ignored and `tracked.env` tracked.
fn repo(name: &str) -> PathBuf {
    let dir = scratch(name).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    std::fs::write(dir.join(".gitignore"), ".env.local\n").unwrap();
    std::fs::write(dir.join("tracked.env"), "A=1\n").unwrap();
    git(&dir, &["add", "tracked.env", ".gitignore"]);
    dir.canonicalize().unwrap()
}

struct World {
    handle: Handle,
    events: UnboundedReceiver<Event>,
    home: PathBuf,
    /// Where members work at their desk: the squad's home, a git repository.
    desk: PathBuf,
    env_file: PathBuf,
}

async fn world() -> World {
    // SAFETY: set once, before the runtime starts, and only by this test binary.
    unsafe { std::env::set_var("EQUIPE_VAULT_IN_MEMORY", "1") };
    let project = repo("shop");
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let env_file = scratch("env").join("turn.env");
    let (handle, mut events) = runtime::start(config(&recording_claude(&env_file), &home));
    for (id, role) in [(1, "pm"), (2, "backend")] {
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some(role.into()), squad: Some("acme".into()) };
        handle.send(Command::Start { session: id, spec });
        until(&mut events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
    }
    // A member's desk is the squad's home: a git repository, here with one ignored and one tracked file.
    let desk = equipe_core::squad_home::path(&home, "acme");
    if !desk.join(".git").exists() {
        git(&desk, &["init", "-q"]);
    }
    let ignore = std::fs::read_to_string(desk.join(".gitignore")).unwrap_or_default();
    std::fs::write(desk.join(".gitignore"), format!("{ignore}\n.env.local\n")).unwrap();
    std::fs::write(desk.join("tracked.env"), "A=1\n").unwrap();
    git(&desk, &["add", "-f", "tracked.env"]);
    World { handle, events, home, desk: desk.canonicalize().unwrap(), env_file }
}


impl World {
    async fn call(&self, member: &str, name: &str, arguments: serde_json::Value) -> equipe_core::protocol::rpc::ToolResult {
        self.handle.tool(ToolCall { run: None, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
    }

    async fn request(&self) -> equipe_core::protocol::rpc::ToolResult {
        self.call(
            "backend",
            "request_secret",
            json!({ "name": "anthropic-admin-key", "service": "anthropic", "kind": "api_key", "why": "read usage and cost reports", "scopes_hint": "read-only / usage scope", "steps": ["Sign in to the Console", "Open Settings > Admin keys", "Click Create key and copy it"] }),
        )
        .await
    }

    /// Every file under the squad's home as text, but for what the person's own `.env.local` writes.
    fn everything_on_disk(&self) -> String {
        fn walk(dir: &Path, out: &mut String) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.file_name().is_some_and(|n| n != ".env.local") {
                    out.push_str(&String::from_utf8_lossy(&std::fs::read(&path).unwrap_or_default()));
                }
            }
        }
        let mut out = String::new();
        walk(&self.home, &mut out);
        out
    }
}

async fn wait_for(file: &Path) {
    for _ in 0..100 {
        if file.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{} never appeared: the member's turn didn't start", file.display());
}

fn no_value_in(events: &[Event]) {
    for e in events {
        assert!(!format!("{e:?}").contains(VALUE), "{e:?}");
        assert!(!serde_json::to_string(e).unwrap().contains(VALUE), "no event carries the value on the wire: {e:?}");
    }
}

#[tokio::test]
async fn a_requested_key_goes_from_the_card_into_the_store_and_the_member_is_told_only_its_name() {
    let mut w = world().await;
    let asked = w.request().await;
    assert!(!asked.is_error && asked.text.contains("saved as anthropic-admin-key"), "{asked:?}");

    // One card, for every screen: what it is for, the catalog's key page, the member's hint — and no value.
    let seen = until(&mut w.events, |e| matches!(e, Event::SecretAsked { .. })).await;
    let Some(Event::SecretAsked { ask: card, .. }) = seen.last().cloned() else { panic!() };
    let store = card.store.clone().expect("a key kept in the squad's store");
    assert_eq!((store.name.as_str(), store.service.as_str(), store.kind), ("anthropic-admin-key", "anthropic", SecretKind::ApiKey));
    assert_eq!(store.create_url.as_deref(), Some("https://platform.claude.com/settings/admin-keys"));
    assert_eq!(store.scopes_hint.as_deref(), Some("read-only / usage scope"));
    // The guide rides the card: the member's numbered steps and the page, the catalog's sign-in as a second link.
    assert_eq!(card.steps, ["Sign in to the Console", "Open Settings > Admin keys", "Click Create key and copy it"]);
    assert_eq!((card.url.as_deref(), card.sign_in_url.as_deref()), (Some("https://platform.claude.com/settings/admin-keys"), Some("https://platform.claude.com/")));
    assert_eq!((card.member.as_str(), card.why.as_str(), card.fields.len(), card.fields[0].secret), ("backend", "read usage and cost reports", 1, true));
    assert_eq!(w.handle.snapshot("acme").await.secret_asks, vec![card.clone()], "a screen attaching sees it");

    // The person pastes it: it reaches the store, the card closes, the facts are published.
    w.handle.send(Command::SaveSecret { squad: "acme".into(), id: card.id, values: vec![SecretValue(VALUE.into())] });
    let _ = std::fs::remove_file(&w.env_file);
    let seen = until(&mut w.events, |e| matches!(e, Event::SecretClosed { saved: true, .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::SecretClosed { id, saved: true, .. } if *id == card.id)), "{seen:#?}");
    let Some(Event::SquadSecrets { secrets }) = seen.iter().rev().find(|e| matches!(e, Event::SquadSecrets { .. })) else { panic!("{seen:#?}") };
    assert_eq!((secrets[0].name.as_str(), secrets[0].service.as_str(), secrets[0].created_by.as_str()), ("anthropic-admin-key", "anthropic", "backend"));
    assert_eq!(secrets[0].kind, SecretKind::ApiKey);
    assert!(w.handle.snapshot("acme").await.secret_asks.is_empty());

    // The member's turn is set going again (its engine starts); what it is told is the name only.
    wait_for(&w.env_file).await;
    no_value_in(&seen);

    // The value is in no file of the squad's home (journal, logs, transcripts) …
    assert!(!w.everything_on_disk().contains(VALUE), "the value is only in the secret store");
    // … and it is in the store: a git-ignored file gets it from there.
    w.handle.send(Command::SetSquadSecretProjects { squad: "acme".into(), name: "anthropic-admin-key".into(), projects: Vec::new(), any_project: true });
    until(&mut w.events, |e| matches!(e, Event::SquadSecrets { secrets } if secrets.iter().any(|s| s.any_project))).await;
    let wrote = w.call("backend", "secret_write", json!({ "name": "anthropic-admin-key", "path": ".env.local", "key": "ANTHROPIC_ADMIN_KEY" })).await;
    assert!(!wrote.is_error && wrote.text.contains("Wrote anthropic-admin-key") && !wrote.text.contains(VALUE), "{wrote:?}");
    let file = std::fs::read_to_string(w.desk.join(".env.local")).unwrap();
    assert!(file.contains(&format!("ANTHROPIC_ADMIN_KEY=\"{VALUE}\"")), "{file}");
    w.handle.shutdown().await;
}

#[tokio::test]
async fn a_card_that_asks_for_nothing_valid_is_refused_and_a_page_must_be_https() {
    let w = world().await;
    for bad in ["http://acme.example.com/keys", "javascript:alert(1)", "https://user:pw@acme.example.com/", "file:///etc/passwd"] {
        let r = w.call("backend", "request_secret", json!({ "name": "k", "service": "acme", "kind": "token", "why": "x", "steps": ["Open it"], "create_url": bad })).await;
        assert!(r.is_error, "{bad}: {r:?}");
    }
    let r = w.call("backend", "request_secret", json!({ "name": "k", "service": "acme", "kind": "token", "steps": ["Open it"], "create_url": "https://acme.example.com/k" })).await;
    assert!(r.is_error && r.text.contains("why"), "{r:?}");
    // The card never appears bare: no steps, or a website's key without its page, is refused with what to add.
    let r = w.call("backend", "request_secret", json!({ "name": "k", "service": "acme", "kind": "token", "why": "x", "create_url": "https://acme.example.com/k" })).await;
    assert!(r.is_error && r.text.contains("`steps`"), "{r:?}");
    let r = w.call("backend", "request_secret", json!({ "name": "k", "service": "acme", "kind": "token", "why": "x", "steps": ["Open it"] })).await;
    assert!(r.is_error && r.text.contains("`create_url`"), "{r:?}");
    assert!(w.handle.snapshot("acme").await.secret_asks.is_empty(), "nothing was put to the person");
    // An unknown service works with the member's own page.
    let ok = w.call("backend", "request_secret", json!({ "name": "acme-token", "service": "acme", "kind": "token", "why": "deploy status", "steps": ["Open Settings > Tokens", "Click New token"], "create_url": "https://acme.example.com/settings/tokens" })).await;
    assert!(!ok.is_error, "{ok:?}");
    let asks = w.handle.snapshot("acme").await.secret_asks;
    assert_eq!(asks[0].store.as_ref().and_then(|s| s.create_url.as_deref()), Some("https://acme.example.com/settings/tokens"));
    assert_eq!((asks[0].url.as_deref(), asks[0].steps.len()), (Some("https://acme.example.com/settings/tokens"), 2));
    w.handle.shutdown().await;
}

#[tokio::test]
async fn the_card_works_the_same_over_a_remote_connection_and_the_value_is_never_logged() {
    let mut w = world().await;
    w.request().await;
    let seen = until(&mut w.events, |e| matches!(e, Event::SecretAsked { .. })).await;
    let Some(Event::SecretAsked { ask: card, .. }) = seen.last().cloned() else { panic!() };

    // What crosses the connection: the ask as the daemon sends it, the paste as the screen sends it.
    let wire_event = serde_json::to_string(seen.last().unwrap()).unwrap();
    let Event::SecretAsked { ask: remote, .. } = serde_json::from_str(&wire_event).unwrap() else { panic!() };
    assert_eq!(remote, card, "the card survives the wire whole");
    let paste = Command::SaveSecret { squad: "acme".into(), id: remote.id, values: vec![SecretValue(VALUE.into())] };
    assert!(!format!("{paste:?}").contains(VALUE), "a log of the command shows no value: {paste:?}");
    let wire_command = serde_json::to_string(&paste).unwrap();
    assert!(wire_command.contains(VALUE), "it travels as typed, over the authenticated connection");
    let received: Command = serde_json::from_str(&wire_command).unwrap();
    assert!(!format!("{received:?}").contains(VALUE));
    w.handle.send(received);
    let after = until(&mut w.events, |e| matches!(e, Event::SquadSecrets { secrets } if !secrets.is_empty())).await;
    no_value_in(&after);
    assert!(!w.everything_on_disk().contains(VALUE));
    w.handle.shutdown().await;
}

#[tokio::test]
async fn connect_from_the_logins_page_puts_the_same_card_up_with_no_member_waiting_and_disconnect_deletes_the_key() {
    let mut w = world().await;
    w.handle.send(Command::RequestServiceKey { squad: "acme".into(), service: "anthropic".into() });
    let seen = until(&mut w.events, |e| matches!(e, Event::SecretAsked { .. })).await;
    let Some(Event::SecretAsked { ask: card, .. }) = seen.last().cloned() else { panic!() };
    assert!(card.member.is_empty(), "the person started it");
    let store = card.store.clone().unwrap();
    assert_eq!((store.name.as_str(), store.service.as_str(), store.create_url.as_deref()), ("anthropic-admin-key", "anthropic", Some("https://platform.claude.com/settings/admin-keys")));

    // Not now: closes, nobody is told, nothing kept.
    w.handle.send(Command::DismissSecret { squad: "acme".into(), id: card.id });
    until(&mut w.events, |e| matches!(e, Event::SecretClosed { saved: false, .. })).await;
    assert!(w.handle.snapshot("acme").await.secret_asks.is_empty());

    // Again, and save.
    w.handle.send(Command::RequestServiceKey { squad: "acme".into(), service: "anthropic".into() });
    let seen = until(&mut w.events, |e| matches!(e, Event::SecretAsked { .. })).await;
    let Some(Event::SecretAsked { ask: card, .. }) = seen.last().cloned() else { panic!() };
    w.handle.send(Command::SaveSecret { squad: "acme".into(), id: card.id, values: vec![SecretValue(format!("  {VALUE}\n").trim().into())] });
    let seen = until(&mut w.events, |e| matches!(e, Event::SquadSecrets { secrets } if !secrets.is_empty())).await;
    no_value_in(&seen);
    let Some(Event::SquadSecrets { secrets }) = seen.last().cloned() else { panic!() };
    assert_eq!(secrets[0].created_by, "you");

    // Connections' status for the squad: a key saved, never used yet.
    let anthropic = connections::Service::by_id("anthropic").unwrap();
    let status = connections::status(anthropic, "acme", &secrets, &[]);
    assert_eq!((status.keys.as_slice(), status.last_used.is_none()), (["anthropic-admin-key".to_owned()].as_slice(), true));

    // The member uses it (the person let any project): the status shows the last use.
    w.handle.send(Command::SetSquadSecretProjects { squad: "acme".into(), name: "anthropic-admin-key".into(), projects: Vec::new(), any_project: true });
    let used = w.call("backend", "secret_use", json!({ "name": "anthropic-admin-key", "as_env": "ANTHROPIC_ADMIN_KEY" })).await;
    assert!(!used.is_error, "{used:?}");
    let seen = until(&mut w.events, |e| matches!(e, Event::SquadSecrets { secrets } if secrets.iter().any(|s| s.last_used.is_some()))).await;
    let Some(Event::SquadSecrets { secrets }) = seen.last().cloned() else { panic!() };
    assert_eq!(connections::status(anthropic, "acme", &secrets, &[]).last_used.map(|u| u.member), Some("backend".to_owned()));

    // Disconnect: the key is deleted from the store and the facts; a member can't use it any more.
    for key in connections::keys_of(anthropic, "acme", &secrets) {
        w.handle.send(Command::DeleteSquadSecret { squad: "acme".into(), name: key.name.clone() });
    }
    let seen = until(&mut w.events, |e| matches!(e, Event::SquadSecrets { secrets } if secrets.is_empty())).await;
    no_value_in(&seen);
    let gone = w.call("backend", "secret_use", json!({ "name": "anthropic-admin-key", "as_env": "ANTHROPIC_ADMIN_KEY" })).await;
    assert!(gone.is_error && gone.text.contains("No key called"), "{gone:?}");
    assert_eq!(connections::status(anthropic, "acme", &[], &[]).keys.len(), 0);

    // A service with no key to add (AWS: console sign-in only), or none at all, puts no card up.
    w.handle.send(Command::RequestServiceKey { squad: "acme".into(), service: "aws".into() });
    w.handle.send(Command::RequestServiceKey { squad: "acme".into(), service: "nowhere".into() });
    until(&mut w.events, |e| matches!(e, Event::Error { .. })).await;
    assert!(w.handle.snapshot("acme").await.secret_asks.is_empty());
    w.handle.shutdown().await;
}
