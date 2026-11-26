//! The board tools' refusals, replayed from what cloud members really sent
//! (the gateway's tool_errors, docs/tool-usage.md in equipe-cloud): each call
//! shape that failed either goes through now, or is refused with the call to
//! make instead — and that call, made as the refusal writes it, goes through.
//! One retry, never a guess.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-accept-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Windows canonicalization adds the verbatim `\\?\` prefix, which Git for
    // Windows does not accept as a `worktree add` destination.
    #[cfg(windows)]
    return dir;
    #[cfg(not(windows))]
    dir.canonicalize().unwrap()
}

/// Write an executable script without holding it open for writing (see tests/runtime.rs).
#[cfg(unix)]
fn write_script(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    let draft = path.with_extension("draft");
    std::fs::write(&draft, script).unwrap();
    assert!(Process::new("cp").arg(&draft).arg(path).status().unwrap().success());
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A stand-in `claude`, signed in: a ticket's turn takes a moment and writes
/// `HERO.md` where it runs; every turn answers in a line.
fn claude() -> PathBuf {
    #[cfg(unix)]
    let bin = scratch("bin").join("claude");
    #[cfg(windows)]
    let bin = scratch("bin").join("claude.exe");
    #[cfg(unix)]
    {
    let answer = r#"cat <<EOF
{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"ok"}}}
{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{"input_tokens":3,"output_tokens":1}}
EOF"#;
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\nif [ \"$2\" = --input-format ] && IFS= read -r line; then set -- -p \"$(printf '%s' \"$line\" | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"message\"][\"content\"], end=\"\")')\" \"$@\"; fi\ncase \"$*\" in *\"You're on T-\"*) sleep 1; echo \"made by the squad\" > HERO.md;; esac\n{answer}\n"
    );
    write_script(&bin, &script);
    }
    #[cfg(windows)]
    {
        // A native fake keeps TP-5 independent of cmd.exe quoting. The real
        // npm `.cmd` launchers are exercised separately by the Windows job.
        let source = bin.with_extension("rs");
        std::fs::write(
            &source,
            r##"fn main() {
    if std::env::args().nth(1).as_deref() == Some("auth") {
        println!(r#"{{"loggedIn":true,"authMethod":"claude.ai"}}"#);
        return;
    }
    std::thread::sleep(std::time::Duration::from_secs(1));
    if std::fs::metadata(".git").is_ok_and(|m| m.is_file()) {
        std::fs::write("HERO.md", "made by the squad\n").unwrap();
    }
    println!(r#"{{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}}"#);
    println!(r#"{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"ok"}}}}}}"#);
    println!(r#"{{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{{"input_tokens":3,"output_tokens":1}}}}"#);
}
"##,
        )
        .unwrap();
        let built = Process::new("rustc").args(["--edition=2024", "-o"]).arg(&bin).arg(&source).status().unwrap();
        assert!(built.success(), "build the Windows fake engine");
    }
    bin
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

fn config(claude: &Path, home: &Path) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        // No Codex CLI either: the person's own must never run in a test.
        codex_binary: scratch("missing").join("codex-does-not-exist").to_string_lossy().into_owned(),
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

/// The first `tool({...})` call a refusal writes, its arguments parsed —
/// placeholders (`<…>`) filled the way a member fills them.
fn written_call(text: &str, tool: &str) -> Value {
    let at = text.find(&format!("{tool}({{")).unwrap_or_else(|| panic!("no {tool}(…) call in: {text}")) + tool.len() + 1;
    let mut depth = 0;
    let mut in_str = false;
    let mut escaped = false;
    let bytes = text.as_bytes();
    let mut end = at;
    for (i, &b) in bytes[at..].iter().enumerate() {
        if in_str {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = at + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    fill(serde_json::from_str(&text[at..end]).unwrap_or_else(|e| panic!("{e}: {}", &text[at..end])))
}

fn fill(value: Value) -> Value {
    match value {
        Value::String(s) if s.starts_with('<') && s.ends_with('>') => Value::String("Filled in by the member".into()),
        Value::Array(items) => Value::Array(items.into_iter().map(fill).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, fill(v))).collect()),
        other => other,
    }
}

/// A squad of a PM and a frontend on one repository, with T-1 the frontend's
/// at work: a checklist of two items.
async fn at_work() -> (Handle, UnboundedReceiver<Event>, PathBuf) {
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let (handle, mut events) = runtime::start(config(&claude(), &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    handle.send(Command::SetSelfLand { squad: "acme".into(), on: false });
    let made = tool(&handle, "pm", "create_ticket", json!({ "title": "Hero", "body": "-", "owner": "frontend", "steps": ["Build the hero", "Test the hero"] })).await;
    assert!(!made.is_error, "{made:?}");
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    (handle, events, home)
}

async fn worktree(handle: &Handle) -> PathBuf {
    let snapshot = handle.snapshot("acme").await;
    PathBuf::from(snapshot.tickets.iter().find(|t| t.id == 1).and_then(|t| t.worktree.clone()).expect("T-1 has a worktree"))
}

fn png(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // 1×1 PNG.
    let bytes: [u8; 67] = [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06,
        0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D,
        0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    std::fs::write(path, bytes).unwrap();
}

#[tokio::test]
async fn check_steps_takes_items_however_they_are_written_and_an_empty_call_is_shown_the_one_to_make() {
    let (handle, _events, _home) = at_work().await;
    // Seen from members: a lone number, a number as text, the list as a string, an item's own words.
    for done in [json!(1), json!("1"), json!("[1]"), json!(["Build the hero"]), json!(1.0)] {
        let said = tool(&handle, "frontend", "check_steps", json!({ "done": done })).await;
        assert!(!said.is_error, "{done}: {said:?}");
        assert!(said.text.contains("1. [x] Build the hero"), "{done}: {}", said.text);
    }
    // A call with nothing it reads (a wrong name): the checklist, and calls that work.
    let said = tool(&handle, "frontend", "check_steps", json!({ "items": [2] })).await;
    assert!(said.is_error);
    assert!(said.text.contains("`items`") && said.text.contains("2. [ ] Test the hero"), "{}", said.text);
    let again = tool(&handle, "frontend", "check_steps", written_call(&said.text, "check_steps")).await;
    assert!(!again.is_error, "{again:?}");
    assert!(again.text.contains("2. [x] Test the hero"), "{}", again.text);
    // Words that are no item: refused with the checklist and the numbers to give.
    let said = tool(&handle, "frontend", "check_steps", json!({ "done": ["Ship it"] })).await;
    assert!(said.is_error && said.text.contains("1. [x] Build the hero"), "{}", said.text);
    let again = tool(&handle, "frontend", "check_steps", written_call(&said.text, "check_steps")).await;
    assert!(!again.is_error, "{again:?}");
    // One line to add, given as a string.
    let said = tool(&handle, "frontend", "check_steps", json!({ "add": "Check it at 390px" })).await;
    assert!(!said.is_error && said.text.contains("Check it at 390px"), "{said:?}");
}

#[tokio::test]
async fn a_hand_in_without_changes_screens_shows_the_call_that_goes_through() {
    let (handle, _events, _home) = at_work().await;
    let said = tool(&handle, "frontend", "submit_for_review", json!({ "summary": "Hero done." })).await;
    assert!(said.is_error);
    assert!(said.text.contains("no screen file"), "{}", said.text);
    let again = tool(&handle, "frontend", "submit_for_review", written_call(&said.text, "submit_for_review")).await;
    assert!(!again.is_error, "{again:?}");
}

#[tokio::test]
async fn a_screenshot_where_the_browser_saved_it_is_taken_and_one_elsewhere_is_shown_the_copy_and_the_call() {
    let (handle, _events, _home) = at_work().await;
    // The member's browser saves its screenshots in equipe's folder for it, not the worktree.
    let shot = std::env::temp_dir().join("equipe-browser").join("acme").join(format!("frontend-{}", std::process::id())).join("hero-390.png");
    png(&shot);
    let said = tool(&handle, "frontend", "submit_for_review", json!({ "summary": "Hero done.", "changes_screens": false, "images": [shot.to_string_lossy()] })).await;
    assert!(!said.is_error, "{said:?}");
    let _ = std::fs::remove_dir_all(shot.parent().unwrap());
}

#[tokio::test]
async fn a_screenshot_saved_outside_is_refused_with_the_copy_to_make_and_the_call_after_it() {
    let (handle, _events, _home) = at_work().await;
    let elsewhere = scratch("elsewhere").join("hero-1280.png");
    png(&elsewhere);
    let said = tool(&handle, "frontend", "submit_for_review", json!({ "summary": "Hero done.", "changes_screens": false, "images": [elsewhere.to_string_lossy()] })).await;
    assert!(said.is_error);
    assert!(said.text.contains("mkdir -p handin-shots && cp"), "{}", said.text);
    // The member runs the copy where it works, then the call as written.
    let work = worktree(&handle).await;
    let copy = said.text.split('`').find(|s| s.starts_with("mkdir -p handin-shots")).unwrap();
    assert!(Process::new("sh").arg("-c").arg(copy).current_dir(&work).status().unwrap().success());
    let again = tool(&handle, "frontend", "submit_for_review", written_call(&said.text, "submit_for_review")).await;
    assert!(!again.is_error, "{again:?}");
}

#[tokio::test]
async fn the_pm_changing_work_in_progress_is_shown_the_call_that_does_it() {
    let (handle, _events, _home) = at_work().await;
    let said = tool(&handle, "pm", "update_ticket", json!({ "id": 1, "body": "Also the empty state." })).await;
    assert!(said.is_error);
    assert!(said.text.contains("in progress") && said.text.contains("nothing was changed"), "{}", said.text);
    let again = tool(&handle, "pm", "update_ticket", written_call(&said.text, "update_ticket")).await;
    assert!(!again.is_error, "{again:?}");
}

#[tokio::test]
async fn a_title_with_an_id_or_an_arrow_is_shown_the_call_with_it_fixed() {
    let (handle, _events, _home) = at_work().await;
    for title in ["T-1: Hero on phones", "[QA] Pricing on phones", "Open the page → footer fits on phones"] {
        let said = tool(&handle, "pm", "create_ticket", json!({ "title": title, "body": "Check the hero.", "owner": "frontend" })).await;
        assert!(said.is_error, "{title}");
        let call = written_call(&said.text, "create_ticket");
        assert!(!call["title"].as_str().unwrap().contains("Filled in"), "{title}: a title equipe could fix is fixed: {}", said.text);
        let again = tool(&handle, "pm", "create_ticket", call).await;
        assert!(!again.is_error, "{title}: {again:?}");
    }
}
