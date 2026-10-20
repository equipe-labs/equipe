//! Logins, driven as a screen and a member drive them: the person saves one
//! (its password to the Keychain — memory here — and never in an event), a
//! member asks to sign in, a card asks the person, and on a no the member
//! hears so. A member never gets a password, and nothing a screen sees has it.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::rpc::ToolCall;
use equipe_core::protocol::vault::LoginDraft;
use equipe_core::protocol::{Access, Command, Decision, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-vault-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn idle_claude() -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = scratch("bin").join("claude");
    std::fs::write(&bin, "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; exit 0; fi\nexec sleep 30\n").unwrap();
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
        // A browser is said to be there; none starts, as nobody says yes.
        npx: Some("/nonexistent/npx".into()),
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        lang: equipe_core::i18n::Lang::En,
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

async fn sign_in(handle: &Handle, site: &str) -> equipe_core::protocol::rpc::ToolResult {
    handle.tool(ToolCall { squad: "acme".into(), member: "frontend".into(), name: "sign_in".into(), arguments: json!({ "site": site, "why": "Add the DNS record" }) }).await
}

#[tokio::test]
async fn a_member_signs_in_only_with_the_persons_yes_and_never_sees_the_password() {
    // SAFETY: set once, before the runtime starts, and only by this test binary.
    unsafe { std::env::set_var("EQUIPE_VAULT_IN_MEMORY", "1") };
    let project = scratch("site").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let (handle, mut events) = runtime::start(config(&idle_claude(), &home));
    for (id, role) in [(1, "pm"), (2, "frontend")] {
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some(role.into()), squad: Some("acme".into()) };
        handle.send(Command::Start { session: id, spec });
        until(&mut events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
    }

    // No login kept: told where the person adds one.
    let none = sign_in(&handle, "dash.cloudflare.com").await;
    assert!(none.is_error && none.text.contains("Settings › Accounts"), "{none:?}");

    // The person saves one: every screen hears it, without the password.
    let draft = LoginDraft { id: None, site: "https://dash.cloudflare.com/login".into(), username: "abel@ironact.net".into(), password: Some("s3cr3t-pw".into()), note: None };
    handle.send(Command::SaveLogin { draft });
    let seen = until(&mut events, |e| matches!(e, Event::Logins { .. })).await;
    let Some(Event::Logins { logins }) = seen.last() else { panic!() };
    assert_eq!((logins[0].site.as_str(), logins[0].username.as_str()), ("dash.cloudflare.com", "abel@ironact.net"));
    let file = std::fs::read_to_string(home.join("vault.json")).unwrap();
    assert!(!file.contains("s3cr3t-pw"));

    // The member asks: a card for the person, naming the site and the account.
    let asking = {
        let handle = handle.clone();
        tokio::spawn(async move { sign_in(&handle, "https://dash.cloudflare.com/x").await })
    };
    let seen = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = seen.last() else { panic!() };
    assert_eq!((approval.tool.as_str(), approval.action.as_str()), ("Sign in", "dash.cloudflare.com · abel@ironact.net"));
    assert_eq!(approval.reason.as_deref(), Some("Add the DNS record"));

    // The person says no: the member hears it, and the use is kept as declined.
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Deny });
    let answer = asking.await.unwrap();
    assert!(!answer.is_error && answer.text.contains("said no"), "{answer:?}");
    let seen = until(&mut events, |e| matches!(e, Event::Logins { logins } if logins[0].uses.iter().any(|u| u.outcome == "declined"))).await;
    // Nothing anyone saw had the password.
    for e in &seen {
        assert!(!format!("{e:?}").contains("s3cr3t-pw"));
    }
    handle.shutdown().await;
}
