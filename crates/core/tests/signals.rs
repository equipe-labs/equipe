//! A project's signals, driven the way a screen and an engine drive them: the
//! PM proposes each on a card, only the person's yes turns it on, it runs at
//! once inside its fence, and what fails wakes the PM — once, until it passes
//! again. The commands stand in for the squad's own (2026-10-31): the site
//! without its analytics tag, the traffic workflow failing, the stars passing.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::journal::Journal;
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Decision, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-signals-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A stand-in `claude`: signed in, and a turn takes a long while.
fn busy_claude() -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = scratch("bin").join("claude");
    let script = "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; exit 0; fi\nexec sleep 30\n";
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
        npx: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        person_home: Some(home.join("person")),
        lang: equipe_core::i18n::Lang::Ko,
        nemotron: None,
    }
}

/// A home where the squad `acme` works on the project `site`, and that project.
fn squad_home() -> (PathBuf, PathBuf) {
    let project = scratch("p").join("site");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("index.html"), "<html><body>no tag here</body></html>\n").unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    (home, project)
}

async fn until(rx: &mut UnboundedReceiver<Event>, seen: &mut Vec<Event>, done: impl Fn(&[Event]) -> bool) {
    let wait = async {
        while !done(seen) {
            match rx.recv().await {
                Some(event) => seen.push(event),
                None => return,
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(20), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
}

async fn start(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, role: &str, project: &Path) {
    let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.to_owned(), role: Some(role.into()), squad: Some("acme".into()) };
    handle.send(Command::Start { session: id, spec });
    let mut seen = Vec::new();
    until(events, &mut seen, |s| s.iter().any(|e| matches!(e, Event::SessionStarted { session, .. } if *session == id))).await;
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

/// What the PM was told of failing signals: woken with it, or left it for after its turn.
fn signal_wakes(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::UserMessage { session: 1, text } | Event::MessageQueued { session: 1, text, .. } | Event::SteerQueued { session: 1, text, .. } => Some(text.clone()),
            _ => None,
        })
        .filter(|t| t.starts_with("[Signal · "))
        .collect()
}

/// The PM proposes a signal; the person answers its card. What the PM is told.
async fn propose(handle: &Handle, events: &mut UnboundedReceiver<Event>, args: serde_json::Value, decision: Decision) -> (String, String) {
    let h = handle.clone();
    let waiting = tokio::spawn(async move { tool(&h, "pm", "propose_signal", args).await });
    let mut seen = Vec::new();
    until(events, &mut seen, |s| s.iter().any(|e| matches!(e, Event::ApprovalRequested { .. }))).await;
    let approval = seen.iter().find_map(|e| if let Event::ApprovalRequested { approval, .. } = e { Some(approval.clone()) } else { None }).unwrap();
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision, words: None });
    let said = waiting.await.unwrap();
    assert!(!said.is_error, "{said:?}");
    (approval.action, said.text)
}

#[tokio::test]
async fn the_squads_signals_go_on_by_card_and_a_failure_wakes_the_pm_once() {
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&busy_claude(), &home));
    start(&handle, &mut events, 1, "pm", &project).await;
    start(&handle, &mut events, 2, "qa", &project).await;

    // Only the PM proposes, and a bad one is said why before any card.
    let analytics = json!({ "name": "site-analytics", "command": "grep -c analytics-config index.html", "expect": "at_least", "value": 1, "reason": "PostHog 태그" });
    let refused = tool(&handle, "qa", "propose_signal", analytics.clone()).await;
    assert!(refused.is_error && refused.text.contains("Only the PM"), "{refused:?}");
    let odd = tool(&handle, "pm", "propose_signal", json!({ "name": "x", "command": "true", "every": "often" })).await;
    assert!(odd.is_error && odd.text.contains("every"), "{odd:?}");

    // Declined: nothing kept.
    let (_, said) = propose(&handle, &mut events, json!({ "name": "nope", "command": "true" }), Decision::Deny).await;
    assert!(said.contains("said no"), "{said}");
    assert!(Journal::for_squad(&home, "acme").unwrap().signals().is_empty());

    // The three: the card shows the exact command; each runs at once on the yes.
    let (card, said) = propose(&handle, &mut events, analytics, Decision::Allow).await;
    assert!(card.contains("grep -c analytics-config index.html") && card.contains("≥ 1") && card.contains("site"), "{card}");
    assert!(said.contains("turned on the signal \"site-analytics\""), "{said}");
    let traffic = json!({ "name": "traffic-workflow", "command": "echo failure; echo 'GH_TRAFFIC_TOKEN is not set' >&2", "expect": "equals", "value": "success" });
    propose(&handle, &mut events, traffic, Decision::Allow).await;
    let stars = json!({ "name": "stars", "command": "echo 25", "expect": "at_least", "value": 0, "change_pct": 20, "every": "1d" });
    propose(&handle, &mut events, stars, Decision::Allow).await;

    // Both broken ones reach the PM, told what to do about them; the stars don't.
    let mut seen = Vec::new();
    until(&mut events, &mut seen, |s| signal_wakes(s).len() >= 2).await;
    let wakes = signal_wakes(&seen);
    let analytics_wake = wakes.iter().find(|t| t.starts_with("[Signal · site-analytics · failing]")).expect("analytics");
    assert!(analytics_wake.contains("0 < 1") && analytics_wake.contains("`create_ticket`") && analytics_wake.contains("`block_on_person`"), "{analytics_wake}");
    let traffic_wake = wakes.iter().find(|t| t.starts_with("[Signal · traffic-workflow · failing]")).expect("traffic");
    assert!(traffic_wake.contains("\"failure\""), "{traffic_wake}");

    // Kept: the readings, and the PM reads them out.
    let journal = Journal::for_squad(&home, "acme").unwrap();
    let ran_once = |j: &Journal| j.signal_states().values().filter(|s| !s.readings.is_empty()).count() == 3;
    for _ in 0..100 {
        if ran_once(&journal) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let states = journal.signal_states();
    assert!(states["site/stars"].readings[0].ok && states["site/stars"].readings[0].value.as_deref() == Some("25"), "{states:?}");
    assert!(states["site/site-analytics"].alerted && !states["site/stars"].alerted);
    let report = tool(&handle, "pm", "signals", json!({})).await;
    assert!(report.text.contains("site · site-analytics — FAILING since") && report.text.contains("site · stars — passing"), "{}", report.text);
    let refused = tool(&handle, "qa", "signals", json!({})).await;
    assert!(refused.is_error, "{refused:?}");

    // Run again: still failing, but the PM was told already.
    let ran = tool(&handle, "pm", "signals", json!({ "check": true })).await;
    assert!(ran.text.contains("Running"), "{}", ran.text);
    for _ in 0..200 {
        if journal.signal_states().values().all(|s| s.readings.len() >= 2) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(journal.signal_states().values().all(|s| s.readings.len() == 2), "{:?}", journal.signal_states());
    let mut more = Vec::new();
    while let Ok(event) = events.try_recv() {
        more.push(event);
    }
    assert!(signal_wakes(&more).is_empty(), "told twice: {:?}", signal_wakes(&more));
    handle.shutdown().await;
}

#[tokio::test]
async fn a_signals_command_writes_nowhere_but_its_scratch() {
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&busy_claude(), &home));
    start(&handle, &mut events, 1, "pm", &project).await;
    // It reads the project, writes to its own temp folder, and can't touch the project.
    let fenced = json!({ "name": "fence", "command": "cat index.html >/dev/null && touch \"$TMPDIR/ok\" && ! touch written 2>/dev/null && echo fenced", "expect": "equals", "value": "fenced" });
    propose(&handle, &mut events, fenced, Decision::Allow).await;
    let journal = Journal::for_squad(&home, "acme").unwrap();
    for _ in 0..200 {
        if journal.signal_states().get("site/fence").is_some_and(|s| !s.readings.is_empty()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let reading = journal.signal_states()["site/fence"].readings[0].clone();
    assert!(reading.ok, "{reading:?}");
    assert!(!project.join("written").exists());
    handle.shutdown().await;
}
