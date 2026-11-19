//! The machine's own credentials come first: with an AWS profile that signs in
//! by itself (`credential_process`) in the person's config, a member's
//! `cli_sign_in` for AWS is refused — naming that profile, no card — and goes
//! through once the member names the permission it lacks (`why_not_machine`).
//! And a sign-in on a per-deploy preview address is never put to the person.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::cli_signin::Phase;
use equipe_core::protocol::rpc::ToolCall;
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("equipe-machine-creds-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// An `aws` whose `login --remote` prints a page and waits.
const FAKE_AWS: &str = r#"#!/bin/sh
case "$1" in
  login)
    echo "Please visit the following URL:"
    echo
    echo "https://us-east-1.signin.aws.amazon.com/v1/authorize?state=abc"
    echo
    printf "Enter the authorization code displayed in your browser: "
    read code
    exit 1;;
esac
"#;

/// The person's AWS config: two profiles that sign in by themselves, one console sign-in. Fake values only.
const AWS_CONFIG: &str = "\
[profile example-read]
credential_process = /usr/local/bin/fake-helper --role read

[profile example-deploy]
credential_process = /usr/local/bin/fake-helper --role deploy

[profile example-console]
login_session = arn:aws:iam::123456789012:user/example
";

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
    if tokio::time::timeout(Duration::from_secs(30), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

async fn call(handle: &Handle, name: &str, args: serde_json::Value) -> equipe_core::protocol::rpc::ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: "backend".into(), name: name.into(), arguments: args }).await
}

#[tokio::test]
async fn a_machine_credential_comes_before_a_sign_in_card() {
    let bin = scratch("bin");
    executable(&bin.join("aws"), FAKE_AWS);
    let claude = bin.join("claude");
    executable(&claude, "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; exit 0; fi\nexec sleep 30\n");
    // SAFETY: this file's only test; nothing else reads the environment meanwhile.
    unsafe {
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
    }
    let home = scratch("home");
    std::fs::create_dir_all(home.join("person/.aws")).unwrap();
    std::fs::write(home.join("person/.aws/config"), AWS_CONFIG).unwrap();
    let project = scratch("proj").join("infra");
    std::fs::create_dir_all(&project).unwrap();
    squads::create(&home, "acme", &project).unwrap();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    for (id, role) in [(1, "pm"), (2, "backend")] {
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some(role.into()), squad: Some("acme".into()) };
        handle.send(Command::Start { session: id, spec });
        until(&mut events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
    }

    // A read-only check, with a console profile that ran out: refused, the working profiles named.
    let refused = call(&handle, "cli_sign_in", json!({ "tool": "aws", "profile": "example-console", "why": "Read the migration inventory" })).await;
    assert!(refused.is_error, "{refused:?}");
    assert!(refused.text.contains("`example-read` (read)") && refused.text.contains("`example-deploy` (deploy)"), "{refused:?}");
    assert!(refused.text.contains("AWS_PROFILE=example-read") && refused.text.contains("why_not_machine"), "{refused:?}");
    assert!(!refused.text.contains("fake-helper"), "a profile's command is never repeated: {refused:?}");
    assert!(handle.snapshot("acme").await.cli_sign_ins.is_empty(), "no card");
    // A reason too vague to name a permission is no reason.
    let vague = call(&handle, "cli_sign_in", json!({ "tool": "aws_sso", "why": "x", "why_not_machine": "need it" })).await;
    assert!(vague.is_error && handle.snapshot("acme").await.cli_sign_ins.is_empty(), "{vague:?}");

    // The permission they lack, named: the card goes up.
    let asked = call(
        &handle,
        "cli_sign_in",
        json!({ "tool": "aws", "profile": "example-console", "why": "Create the deploy role", "why_not_machine": "iam:CreateRole AccessDenied with example-read and example-deploy" }),
    )
    .await;
    assert!(!asked.is_error && asked.text.contains("never see a code"), "{asked:?}");
    let seen = until(&mut events, |e| matches!(e, Event::CliSignInChanged { card, .. } if card.phase == Phase::Waiting)).await;
    let Some(Event::CliSignInChanged { card, .. }) = seen.last().cloned() else { panic!() };
    assert_eq!((card.name.as_str(), card.profile.as_deref()), ("AWS", Some("example-console")));
    handle.send(Command::CancelCliSignIn { squad: "acme".into(), id: card.id });
    until(&mut events, |e| matches!(e, Event::CliSignInClosed { .. })).await;

    // A sign-in on a per-deploy preview: never put to the person.
    let preview = call(&handle, "sign_in", json!({ "site": "https://shop-k3j9x2m1q-example.vercel.app/login", "why": "Check the dashboard" })).await;
    assert!(preview.is_error && preview.text.contains("per-deploy") && preview.text.contains("localhost"), "{preview:?}");
    let handed = call(&handle, "show_my_browser", json!({ "url": "https://deploy-preview-7--shop.netlify.app", "why": "Sign in with the company account" })).await;
    assert!(handed.is_error && handed.text.contains("per-deploy"), "{handed:?}");
    handle.shutdown().await;
}
