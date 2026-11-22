//! A member's word to the person (`tell_person`) shows in the PM's
//! conversation: the person talks to the whole team in one place. The tool
//! call in, the note out as an event and a line of the PM's conversation, the
//! tap on a choice going to the member's own session. A stand-in `claude`
//! plays the engine.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Entry, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-member-note-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn write_script(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A stand-in `claude` that answers "ok" and writes down every turn's
/// arguments (the prompt first, as `-p`), one file a call.
fn recording_claude() -> (PathBuf, PathBuf) {
    let calls = scratch("calls");
    let bin = scratch("bin").join("claude");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\n\
if [ \"$2\" = --input-format ] && IFS= read -r line; then set -- -p \"$(printf '%s' \"$line\" | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"message\"][\"content\"], end=\"\")')\" \"$@\"; fi\n\
n=0\nwhile ! mkdir '{dir}/.n-'$n 2>/dev/null; do n=$((n+1)); done\n\
for a in \"$@\"; do printf '%s\\n<<arg>>\\n' \"$a\"; done > '{dir}/.call-'$$\nmv '{dir}/.call-'$$ '{dir}/call-'$(printf %03d $n)-$$\n\
cat <<EOF\n{{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-test\",\"session_id\":\"sess-1\"}}\n\
{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"usage\":{{\"input_tokens\":3,\"output_tokens\":1}}}}\nEOF\n",
        dir = calls.display()
    );
    write_script(&bin, &script);
    (bin, calls)
}

/// Each recorded turn's arguments, oldest first.
fn calls(dir: &Path) -> Vec<Vec<String>> {
    // Only whole calls: each is written aside and moved into place once complete.
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("call-")).map(|e| e.path()).collect();
    files.sort();
    files.iter().map(|f| std::fs::read_to_string(f).unwrap().split("\n<<arg>>\n").filter(|a| !a.is_empty()).map(str::to_owned).collect()).collect()
}

fn config(claude: &Path, home: &Path, persist: bool) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        // No Codex CLI either: the person's own must never run in a test.
        codex_binary: "/nonexistent/codex".into(),
        persist,
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

/// A home where the squad `acme` works on one project, and that project.
fn squad_home() -> (PathBuf, PathBuf) {
    let project = scratch("site").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    (home, project)
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

fn member(cwd: &Path, role: &str) -> SessionSpec {
    SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: cwd.to_owned(), role: Some(role.into()), squad: Some("acme".into()) }
}

async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, spec: SessionSpec) {
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

#[tokio::test]
async fn a_members_word_to_the_person_shows_in_the_pm_conversation_and_its_choices_go_to_the_member() {
    let (claude, recorded) = recording_claude();
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    add(&handle, &mut events, 1, member(&project, "pm")).await;
    add(&handle, &mut events, 2, member(&project, "designer")).await;

    // Said with two choices: it shows in the PM's conversation, from the designer.
    let said = tool(&handle, "designer", "tell_person", json!({ "text": "Two options for the booking page.", "choices": ["A: airy", "B: dense", "A: airy"] })).await;
    assert!(!said.is_error && said.text.contains("next message"), "{said:?}");
    let seen = until(&mut events, |e| matches!(e, Event::MemberNote { .. })).await;
    let Some(Event::MemberNote { session, from, text, choices, source, at, .. }) = seen.last().cloned() else { panic!("{seen:#?}") };
    assert_eq!((session, from.name.as_str(), from.role.as_str(), source), (1, "designer", "designer", 2));
    assert_eq!((text.as_str(), choices.as_slice()), ("Two options for the booking page.", ["A: airy".to_owned(), "B: dense".to_owned()].as_slice()));
    assert!(at > 0);

    // A screen attaching later reads it as a line of the PM's conversation, not of the designer's.
    let snapshot = handle.snapshot("acme").await;
    let line = |id: u64| snapshot.sessions.iter().find(|s| s.session == id).map(|s| s.recent.clone()).unwrap_or_default();
    assert!(line(1).iter().any(|e| matches!(e, Entry::MemberNote { from, choices, session: 2, .. } if from.name == "designer" && choices.len() == 2)), "{:#?}", line(1));
    assert!(!line(2).iter().any(|e| matches!(e, Entry::MemberNote { .. })));

    // It is a line, not a turn: nothing was said to the PM, and nothing waits on the person.
    assert!(calls(&recorded).is_empty(), "no engine turn ran");
    assert!(snapshot.questions.is_empty() && snapshot.approvals.is_empty());

    // A lone option is no choice; a plain note has none. Leaving out the text is refused.
    tool(&handle, "designer", "tell_person", json!({ "text": "Booking page done.", "choices": ["Only"] })).await;
    let seen = until(&mut events, |e| matches!(e, Event::MemberNote { .. })).await;
    assert!(matches!(seen.last(), Some(Event::MemberNote { choices, text, .. }) if choices.is_empty() && text == "Booking page done."));
    let bad = tool(&handle, "designer", "tell_person", json!({ "choices": ["A", "B"] })).await;
    assert!(bad.is_error, "{bad:?}");

    // The PM talks to the person itself.
    let pm = tool(&handle, "pm", "tell_person", json!({ "text": "hello" })).await;
    assert!(pm.is_error && pm.text.contains("PM"), "{pm:?}");

    // The person taps a choice: it goes to the designer's own session, as the person's message.
    handle.send(serde_json::from_value::<Command>(json!({ "type": "say", "squad": "acme", "session": 2, "text": "A: airy" })).unwrap());
    let seen = until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::UserMessage { session: 2, text, .. } if text == "A: airy")), "{seen:#?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::UserMessage { session: 1, .. })));
    handle.shutdown().await;
}

#[test]
fn only_a_non_pm_member_is_offered_the_tool() {
    let names = |role: &str| equipe_core::board::tools_for(role).iter().filter_map(|t| t["name"].as_str().map(str::to_owned)).collect::<Vec<_>>();
    assert!(names("designer").contains(&"tell_person".to_owned()));
    assert!(names("frontend").contains(&"tell_person".to_owned()));
    assert!(!names("pm").contains(&"tell_person".to_owned()));
}
