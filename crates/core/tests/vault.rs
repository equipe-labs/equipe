//! Logins, driven as a screen and a member drive them: the person saves one
//! (its password to the Keychain — memory here — and never in an event), a
//! member asks to sign in, a card asks the person, and on a no the member
//! hears so. A member never gets a password, and nothing a screen sees has it.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::rpc::ToolCall;
use equipe_core::protocol::secrets::SecretValue;
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

async fn sign_in(handle: &Handle, site: &str) -> equipe_core::protocol::rpc::ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: "frontend".into(), name: "sign_in".into(), arguments: json!({ "site": site, "why": "Add the DNS record" }) }).await
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
    let draft = LoginDraft { id: None, site: "https://dash.cloudflare.com/login".into(), username: "abel@ironact.net".into(), password: Some("s3cr3t-pw".into()), note: None, auto: None };
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
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Deny, words: None, said: None });
    let answer = asking.await.unwrap();
    assert!(!answer.is_error && answer.text.contains("said no"), "{answer:?}");
    let seen = until(&mut events, |e| matches!(e, Event::Logins { logins } if logins[0].uses.iter().any(|u| u.outcome == "declined"))).await;
    // Nothing anyone saw had the password.
    for e in &seen {
        assert!(!format!("{e:?}").contains("s3cr3t-pw"));
    }
    handle.shutdown().await;
}

#[tokio::test]
async fn a_secret_the_person_types_goes_to_its_file_and_never_to_the_member() {
    let project = scratch("nas").join("nas");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let (handle, mut events) = runtime::start(config(&idle_claude(), &home));
    for (id, role) in [(1, "pm"), (2, "backend")] {
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some(role.into()), squad: Some("acme".into()) };
        handle.send(Command::Start { session: id, spec });
        until(&mut events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
    }
    let ask = |args: serde_json::Value| handle.tool(ToolCall { run: None, squad: "acme".into(), member: "backend".into(), name: "ask_secret".into(), arguments: args });

    // Somewhere keys already live: refused, said why.
    let ssh = ask(json!({ "why": "NAS", "save_to": "~/.ssh/nas", "fields": [{ "key": "NAS_PASSWORD", "label": "비밀번호", "secret": true }] })).await;
    assert!(ssh.is_error && ssh.text.contains("keys and logins already live"), "{ssh:?}");

    // Asked: a card for the person; the member carries on meanwhile.
    let asked = ask(json!({ "why": "NAS 매일 자동 수집", "save_to": "~/.config/atinum/nas.env",
        "fields": [{ "key": "NAS_USER", "label": "NAS 아이디" }, { "key": "NAS_PASSWORD", "label": "비밀번호", "secret": true }] })).await;
    assert!(!asked.is_error && asked.text.contains("you never see the values"), "{asked:?}");
    let seen = until(&mut events, |e| matches!(e, Event::SecretAsked { .. })).await;
    let Some(Event::SecretAsked { ask: card, .. }) = seen.last().cloned() else { panic!() };
    assert_eq!((card.member.as_str(), card.save_to.as_str(), card.fields.len()), ("backend", "~/.config/atinum/nas.env", 2));
    assert_eq!(handle.snapshot("acme").await.secret_asks, vec![card.clone()], "a screen attaching sees it");

    // The person types it: saved to the file, readable by them alone; the card closes; the
    // member hears where it is — never the value.
    let values = vec![SecretValue("ai".into()), SecretValue("s3cr3t-nas".into())];
    handle.send(Command::SaveSecret { squad: "acme".into(), id: card.id, values });
    let seen = until(&mut events, |e| matches!(e, Event::SecretClosed { saved: true, .. })).await;
    let file = home.join("person/.config/atinum/nas.env");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "NAS_USER=\"ai\"\nNAS_PASSWORD=\"s3cr3t-nas\"\n");
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let told = until(&mut events, |e| matches!(e, Event::Notice { session: 2, .. })).await;
    let all = format!("{seen:?}{told:?}");
    assert!(!all.contains("s3cr3t-nas"), "no event carries the value");
    assert!(handle.snapshot("acme").await.secret_asks.is_empty());
    handle.shutdown().await;
}

async fn sign_in_as(handle: &Handle, member: &str, site: &str) -> equipe_core::protocol::rpc::ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: member.into(), name: "sign_in".into(), arguments: json!({ "site": site, "why": "Check the billing page" }) }).await
}

/// One card per login however many members ask at once; "always" is the
/// whole squad's; and every handoff, card and sign-in is kept with its
/// cause — hosts and names only.
#[tokio::test]
async fn members_asking_at_once_share_one_card_and_always_is_the_whole_squads() {
    // SAFETY: set once, before the runtime starts, and only by this test binary.
    unsafe { std::env::set_var("EQUIPE_VAULT_IN_MEMORY", "1") };
    let project = scratch("shared").join("shared");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let (handle, mut events) = runtime::start(config(&idle_claude(), &home));
    for (id, role) in [(1, "pm"), (2, "frontend"), (3, "backend")] {
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some(role.into()), squad: Some("acme".into()) };
        handle.send(Command::Start { session: id, spec });
        until(&mut events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
    }
    // No login yet: kept on the record as such.
    let none = sign_in_as(&handle, "backend", "https://github.com/login?return_to=%2Fsecret").await;
    assert!(none.is_error, "{none:?}");
    let draft = LoginDraft { id: None, site: "github.com".into(), username: "abel".into(), password: Some("pw-github-1".into()), note: None, auto: None };
    handle.send(Command::SaveLogin { draft });
    until(&mut events, |e| matches!(e, Event::Logins { logins } if !logins.is_empty())).await;

    // Two members at once: one card, both wait on it.
    let first = {
        let handle = handle.clone();
        tokio::spawn(async move { sign_in_as(&handle, "frontend", "github.com").await })
    };
    let seen = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = seen.last().cloned() else { panic!() };
    let second = {
        let handle = handle.clone();
        tokio::spawn(async move { sign_in_as(&handle, "backend", "https://github.com/settings/billing").await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    // The person says no once: both hear it.
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Deny, words: None, said: None });
    let (first, second) = (first.await.unwrap(), second.await.unwrap());
    assert!(first.text.contains("said no") && second.text.contains("said no"), "{first:?} {second:?}");
    let seen = until(&mut events, |e| matches!(e, Event::Logins { logins } if logins[0].uses.iter().any(|u| u.outcome == "declined"))).await;
    assert!(!seen.iter().any(|e| matches!(e, Event::ApprovalRequested { .. })), "no second card: {seen:#?}");

    // "Always" from one member's card: the squad's.
    let asking = {
        let handle = handle.clone();
        tokio::spawn(async move { sign_in_as(&handle, "frontend", "github.com").await })
    };
    let seen = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = seen.last().cloned() else { panic!() };
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Always, words: None, said: None });
    let _ = asking.await.unwrap();
    until(&mut events, |e| matches!(e, Event::Logins { logins } if logins[0].allowed == ["acme"])).await;
    // Another member of the squad: no card at all (no browser here, so it stops there).
    let other = sign_in_as(&handle, "backend", "github.com").await;
    assert!(other.is_error && other.text.contains("browser couldn't be started"), "straight to the form, no card: {other:?}");

    // The person's own changes: every squad, signing in by itself.
    handle.send(Command::AllowLogin { id: 1, who: "*".into() });
    until(&mut events, |e| matches!(e, Event::Logins { logins } if logins[0].allowed.contains(&"*".to_owned()))).await;
    handle.send(Command::SetLoginAuto { id: 1, auto: true });
    let seen = until(&mut events, |e| matches!(e, Event::Logins { logins } if logins[0].auto)).await;
    handle.send(Command::ForgetLoginAllowance { id: 1, who: "acme/frontend".into() });
    until(&mut events, |e| matches!(e, Event::Logins { logins } if logins[0].allowed == ["*"])).await;
    for e in &seen {
        assert!(!format!("{e:?}").contains("pw-github-1"));
    }
    // An offer nobody made: answering it does nothing.
    handle.send(Command::DismissLoginOffer { id: 99, never: true });
    assert!(handle.snapshot("acme").await.login_offers.is_empty());

    // Login autopilot: the person's seed goes to the secret store; screens hear only that one is kept.
    let seed = "otpauth://totp/GitHub:abel?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=GitHub";
    handle.send(Command::SetLoginTotp { id: 1, secret: Some(SecretValue(seed.into())) });
    let seen = until(&mut events, |e| matches!(e, Event::Logins { logins } if logins[0].autopilot.has_totp)).await;
    let Some(Event::Logins { logins }) = seen.last() else { panic!() };
    assert!(logins[0].autopilot.totp, "keeping a seed is turning its codes on");
    // A bad one is refused, saying why.
    handle.send(Command::SetLoginTotp { id: 1, secret: Some(SecretValue("not a seed!".into())) });
    until(&mut events, |e| matches!(e, Event::Error { message, .. } if message.contains("authenticator"))).await;
    // The switches are the person's; a screen can't claim a seed or passkey.
    let mut asked = logins[0].autopilot.clone();
    (asked.keepalive, asked.has_passkey) = (true, true);
    handle.send(Command::SetLoginAutopilot { id: 1, autopilot: asked });
    let seen = until(&mut events, |e| matches!(e, Event::Logins { logins } if logins[0].autopilot.keepalive)).await;
    let Some(Event::Logins { logins }) = seen.last() else { panic!() };
    assert!(!logins[0].autopilot.has_passkey && logins[0].autopilot.has_totp);
    handle.send(Command::Autopilot);
    let seen = until(&mut events, |e| matches!(e, Event::Autopilot { .. })).await;
    for e in &seen {
        assert!(!format!("{e:?}").contains("GEZDGNBV"), "{e:?}");
    }
    for file in ["vault.json", "autopilot.json"] {
        assert!(!std::fs::read_to_string(home.join(file)).unwrap_or_default().contains("GEZDGNBV"), "{file}");
    }
    handle.shutdown().await;

    // On the record: the no-login, one card for the two who asked at once and
    // one for the "always", the signs-in tried — github.com only, never the path.
    let rows = equipe_core::logins::read(&equipe_core::journal::squad_dir(&home, "acme").join("squad.db"), 0).unwrap();
    let kinds: Vec<&str> = rows.iter().map(|r| r.kind.code()).collect();
    assert_eq!(kinds.iter().filter(|k| **k == "no_login").count(), 1, "{rows:#?}");
    assert_eq!(kinds.iter().filter(|k| **k == "card").count(), 2, "one card for two members, one for the always: {rows:#?}");
    assert!(rows.iter().all(|r| r.site == "github.com"), "{rows:#?}");
    let file = std::fs::read(equipe_core::journal::squad_dir(&home, "acme").join("squad.db")).unwrap();
    assert!(!String::from_utf8_lossy(&file).contains("pw-github-1"));
}

#[tokio::test]
async fn the_logins_page_changes_scope_in_one_step_reads_the_record_and_takes_a_never_back() {
    use equipe_core::logins::{BY_EQUIPE, Cause, Entry, Kind};
    unsafe { std::env::set_var("EQUIPE_VAULT_IN_MEMORY", "1") };
    let project = scratch("page").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let other = scratch("page-shop").join("shop");
    std::fs::create_dir_all(&other).unwrap();
    squads::create(&home, "shop", &other).unwrap();
    // A record from before, in two squads' journals, and a site said never to.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    for (squad, row) in [
        ("acme", Entry::new(now - 3_600, "github.com", "qa", Kind::Handoff, Some(Cause::SessionExpired), None)),
        ("shop", Entry::new(now - 1_800, "github.com", "pm", Kind::Card, Some(Cause::SessionExpired), None)),
        ("shop", Entry::new(now - 600, "github.com", "pm", Kind::SignedIn, None, Some(BY_EQUIPE))),
    ] {
        equipe_core::journal::Journal::for_squad(&home, squad).unwrap().record_login(&row).unwrap();
    }
    std::fs::write(home.join("vault-never-offer.json"), r#"["slack.com","erp.example.com"]"#).unwrap();
    let (handle, mut events) = runtime::start(config(&idle_claude(), &home));

    let draft = LoginDraft { id: None, site: "github.com".into(), username: "abel".into(), password: Some("pw-page-1".into()), note: Some("work".into()), auto: None };
    handle.send(Command::SaveLogin { draft });
    until(&mut events, |e| matches!(e, Event::Logins { logins } if !logins.is_empty())).await;

    // Scope, in one command each: every squad, then one squad, then ask each time.
    for (squads, expect) in [(vec!["*".to_owned()], vec!["*"]), (vec!["shop".to_owned()], vec!["shop"]), (vec![], vec![])] {
        handle.send(Command::SetLoginScope { id: 1, squads });
        until(&mut events, |e| matches!(e, Event::Logins { logins } if logins[0].allowed == expect)).await;
    }
    // A login that isn't there: said, not made up.
    handle.send(Command::SetLoginScope { id: 42, squads: vec!["*".into()] });
    until(&mut events, |e| matches!(e, Event::Error { .. })).await;

    // The record of every squad, and the vault's own facts — read-only.
    handle.send(Command::LoginInsights { days: 7 });
    let seen = until(&mut events, |e| matches!(e, Event::LoginInsights { .. })).await;
    let Some(Event::LoginInsights { insights }) = seen.last() else { panic!() };
    assert_eq!((insights.days, insights.asked, insights.signed_in, insights.signed_in_by_equipe), (7, 2, 1, 1), "{insights:#?}");
    assert_eq!(insights.sites[0].site, "github.com");
    assert_eq!(insights.never, ["erp.example.com", "slack.com"]);
    assert!(insights.can_keep);
    assert_eq!(insights.trend.len(), 7);

    // A never taken back: the list read again with it gone, and kept.
    handle.send(Command::ForgetNeverOffer { site: "https://www.slack.com/signin".into() });
    let seen = until(&mut events, |e| matches!(e, Event::LoginInsights { .. })).await;
    let Some(Event::LoginInsights { insights }) = seen.last() else { panic!() };
    assert_eq!(insights.never, ["erp.example.com"]);
    assert_eq!(insights.days, 7, "read again over the look-back the page last asked for");
    handle.send(Command::LoginInsights { days: 30 });
    let seen = until(&mut events, |e| matches!(e, Event::LoginInsights { .. })).await;
    let Some(Event::LoginInsights { insights }) = seen.last() else { panic!() };
    assert_eq!((insights.days, insights.trend.len()), (30, 30));
    for e in &seen {
        assert!(!format!("{e:?}").contains("pw-page-1"));
    }
    handle.shutdown().await;
    assert!(!std::fs::read_to_string(home.join("vault-never-offer.json")).unwrap().contains("slack.com"));
}
