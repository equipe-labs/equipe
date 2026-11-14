//! `cli_sign_in`, driven as a member and a screen drive it, with a fake `aws`
//! on the PATH: the member asks, a card shows the page the tool printed, the
//! person pastes a code (written to the waiting process, in no event), an
//! expired code is followed by "Get a new link", and the second run signs in
//! as the identity the tool's own whoami gives.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::cli_signin::{PastedCode, Phase};
use equipe_core::protocol::rpc::ToolCall;
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-cli-signin-it-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn idle_claude() -> PathBuf {
    let bin = scratch("bin").join("claude");
    executable(&bin, "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; exit 0; fi\nexec sleep 30\n");
    bin
}

/// An `aws` whose `login --remote` prints a page and waits for a code: the
/// first run says the code expired, the next accepts GOODCODE. Its `sts` says who.
const FAKE_AWS: &str = r#"#!/bin/sh
case "$1" in
  login)
    echo "Browser will not be automatically opened."
    echo "Please visit the following URL:"
    echo
    echo "https://us-east-1.signin.aws.amazon.com/v1/authorize?state=abc&code_challenge=xyz"
    echo
    printf "Enter the authorization code displayed in your browser: "
    read code
    if [ -f "$FAKE_AWS_STATE" ]; then
      if [ "$code" = "GOODCODE" ]; then echo; echo "Login completed"; exit 0; fi
      echo; echo "Invalid code"; exit 2
    fi
    touch "$FAKE_AWS_STATE"
    echo; echo "The authorization code has expired"; exit 1;;
  sts) echo "arn:aws:iam::123456789012:user/atinum-admin";;
esac
"#;

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

fn phase(e: &Event, want: Phase) -> bool {
    matches!(e, Event::CliSignInChanged { card, .. } if card.phase == want)
}

async fn ask(handle: &Handle, args: serde_json::Value) -> equipe_core::protocol::rpc::ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: "backend".into(), name: "cli_sign_in".into(), arguments: args }).await
}

#[tokio::test]
async fn a_sign_in_goes_from_a_card_through_an_expired_code_and_a_new_link_to_signed_in() {
    // The fake `aws`, first on the PATH of every process equipe starts.
    let bin = scratch("aws-bin");
    executable(&bin.join("aws"), FAKE_AWS);
    let state = scratch("state").join("ran-once");
    // SAFETY: this file's only test; nothing else reads the environment meanwhile.
    unsafe {
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        std::env::set_var("FAKE_AWS_STATE", &state);
    }
    let project = scratch("proj").join("infra");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let (handle, mut events) = runtime::start(config(&idle_claude(), &home));
    for (id, role) in [(1, "pm"), (2, "backend")] {
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some(role.into()), squad: Some("acme".into()) };
        handle.send(Command::Start { session: id, spec });
        until(&mut events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
    }

    // A tool with no cross-device mode, or a flag that would hand equipe a secret: refused, said why.
    let none = ask(&handle, json!({ "tool": "vercel", "why": "deploy" })).await;
    assert!(none.is_error && none.text.contains("block_on_person"), "{none:?}");
    let token = ask(&handle, json!({ "tool": "gh", "why": "push", "args": ["--with-token"] })).await;
    assert!(token.is_error, "{token:?}");

    // Asked: one card, the page the tool printed; the member carries on meanwhile.
    let asked = ask(&handle, json!({ "tool": "aws", "profile": "atinum", "why": "Deploy the staging stack" })).await;
    assert!(!asked.is_error && asked.text.contains("never see a code"), "{asked:?}");
    let seen = until(&mut events, |e| phase(e, Phase::Waiting)).await;
    let Some(Event::CliSignInChanged { card, .. }) = seen.last().cloned() else { panic!() };
    assert_eq!((card.name.as_str(), card.profile.as_deref(), card.member.as_str(), card.wants_code), ("AWS", Some("atinum"), "backend", true));
    assert_eq!(card.url.as_deref(), Some("https://us-east-1.signin.aws.amazon.com/v1/authorize?state=abc&code_challenge=xyz"));
    assert_eq!(handle.snapshot("acme").await.cli_sign_ins, vec![card.clone()], "a screen attaching sees it");

    // The person pastes a code that has expired: the card says so, and offers a new link.
    handle.send(Command::SubmitCliSignInCode { squad: "acme".into(), id: card.id, code: PastedCode("STALECODE-123".into()) });
    let expired = until(&mut events, |e| phase(e, Phase::Expired)).await;
    handle.send(Command::RestartCliSignIn { squad: "acme".into(), id: card.id });
    let again = until(&mut events, |e| phase(e, Phase::Waiting)).await;

    // The right code this time: signed in, as the tool's own whoami says.
    handle.send(Command::SubmitCliSignInCode { squad: "acme".into(), id: card.id, code: PastedCode("GOODCODE".into()) });
    let done = until(&mut events, |e| phase(e, Phase::SignedIn)).await;
    let Some(Event::CliSignInChanged { card: signed, .. }) = done.last().cloned() else { panic!() };
    assert_eq!(signed.who.as_deref(), Some("arn:aws:iam::123456789012:user/atinum-admin"));

    // No event, no snapshot, no file in the home has either code.
    let all = format!("{expired:?}{again:?}{done:?}{:?}", handle.snapshot("acme").await);
    assert!(!all.contains("STALECODE") && !all.contains("GOODCODE"), "a pasted code is in an event");
    fn files(dir: &Path, out: &mut String) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                files(&path, out);
            } else if let Ok(text) = std::fs::read(&path) {
                out.push_str(&String::from_utf8_lossy(&text));
            }
        }
    }
    let mut kept = String::new();
    files(&home, &mut kept);
    assert!(!kept.contains("STALECODE") && !kept.contains("GOODCODE"), "a pasted code was kept on disk");
    handle.shutdown().await;
}
