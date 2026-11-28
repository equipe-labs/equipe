//! `show_references`, driven as a member and a screen drive it: the PM shows
//! references — a picture it saved is kept in the squad's picture store and
//! the gallery comes as one note in its conversation, a picture that can't be
//! had is said to the member — a screen gets the picture by `read_image`, the
//! person's heart reaches their taste, and a card kept lands in the project's
//! references with its link and never its picture. A stand-in `claude` plays
//! the engine.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::design::{QuickReaction, TasteSignal};
use equipe_core::protocol::references::RefCard;
use equipe_core::protocol::rpc::{ReadImage, ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-refcards-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A stand-in `claude` that answers "ok" to every turn.
fn claude() -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = scratch("bin").join("claude");
    let script = "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; exit 0; fi\n\
cat <<EOF\n{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-test\",\"session_id\":\"sess-1\"}\n\
{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}\nEOF\n";
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

fn member(cwd: &Path, role: &str) -> SessionSpec {
    SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: cwd.to_owned(), role: Some(role.into()), squad: Some("acme".into()) }
}

async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, spec: SessionSpec) {
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
}

async fn tool(handle: &Handle, member: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: member.into(), name: equipe_core::references::TOOL.into(), arguments }).await
}

fn png() -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(32, 24, image::Rgba([40, 90, 200, 255])).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[tokio::test]
async fn the_pm_shows_references_as_one_note_and_the_persons_reactions_reach_taste_and_references() {
    let claude = claude();
    let project = scratch("site").join("site");
    std::fs::create_dir_all(project.join("design")).unwrap();
    std::fs::write(project.join("design/tokens.json"), "{}").unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let folder = equipe_core::squad_home::path(&home, "acme");
    std::fs::write(folder.join("pricing.png"), png()).unwrap();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&project, "pm")).await;
    add(&handle, &mut events, 2, member(&project, "qa")).await;

    // Only the PM, a designer or frontend shows references; a wrong call is said at once.
    let refused = tool(&handle, "qa", json!({ "items": [{ "picture": "pricing.png", "title": "A", "link": "https://a.example.com" }] })).await;
    assert!(refused.is_error && refused.text.contains("Only the PM"), "{}", refused.text);
    let wrong = tool(&handle, "pm", json!({ "items": [{ "picture": "https://user:pw@cdn.example.com/a.png", "title": "A", "link": "https://a.example.com" }] })).await;
    assert!(wrong.is_error && wrong.text.contains("picture"), "{}", wrong.text);

    // A picture it saved, and one whose site doesn't exist.
    let args = json!({ "items": [
        { "picture": folder.join("pricing.png").to_string_lossy(), "title": "Three-tier pricing", "site": "21st.dev", "link": "https://21st.dev/r/pricing", "kind": "component", "why": "Same three plans", "take": "Raised middle plan", "leave": "Glow", "pattern": "pricing-table" },
        { "picture": "https://nothing.invalid/x.png", "title": "Gone", "link": "https://nothing.invalid/page" },
    ] });
    let answered = tool(&handle, "pm", args).await;
    assert!(!answered.is_error, "{}", answered.text);
    assert!(answered.text.contains("1 reference(s): Three-tier pricing") && answered.text.contains("Gone ("), "{}", answered.text);
    let seen = until(&mut events, |e| matches!(e, Event::Notice { session: 1, text, .. } if text.contains("is showing you"))).await;
    let note = seen.iter().rev().find_map(|e| if let Event::Notice { session: 1, text, .. } = e { Some(text.clone()) } else { None }).unwrap();
    assert!(note.starts_with("◫ pm is showing you 1 reference\n"), "{note}");
    let cards = equipe_core::references::parse_body(&note);
    assert_eq!(cards.len(), 1);
    let card = cards[0].clone();
    let store = equipe_core::references::store_dir(&home, "acme");
    assert!(Path::new(&card.picture).starts_with(&store), "kept in the squad's store: {}", card.picture);
    assert!(!Path::new(&card.picture).starts_with(&folder) && !Path::new(&card.picture).starts_with(&project), "never in a repository");
    assert_eq!((card.title.as_str(), card.site.as_str(), card.take.as_str(), card.pattern.as_str()), ("Three-tier pricing", "21st.dev", "Raised middle plan", "pricing-table"));

    // A screen, here or attached from elsewhere, gets it by its path.
    let read = equipe_core::visuals::serve(&home, &ReadImage { squad: "acme".into(), path: card.picture.clone(), max: Some(48) }, None).unwrap();
    assert!(read.mime.starts_with("image/"), "{}", read.mime);

    // A heart: the squad's taste keeps the picture liked.
    let signal = TasteSignal::React { picture: Some(card.picture.clone()), label: "Three-tier pricing (21st.dev)".into(), reaction: QuickReaction::Up, reasons: vec![] };
    handle.send(Command::TasteSignals { squad: "acme".into(), project: None, signals: vec![signal] });
    let signals = folder.join("design/tuner/signals.jsonl");
    let waited = tokio::time::timeout(Duration::from_secs(10), async {
        while !std::fs::read_to_string(&signals).is_ok_and(|t| t.contains(&card.picture)) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "the heart reached the taste signals");
    let line = std::fs::read_to_string(&signals).unwrap();
    assert!(line.contains(r#""kind":"like""#) && line.contains(r#""liked":true"#), "{line}");

    // Kept among the project's references: its link and lines, never its picture.
    handle.send(Command::KeepReference { squad: "acme".into(), session: Some(1), project: Some("site".into()), card: RefCard { ..card.clone() } });
    let seen = until(&mut events, |e| matches!(e, Event::Notice { session: 1, text, .. } if text.contains("in the references"))).await;
    let notes = std::fs::read_to_string(project.join("design/references/pricing-table/notes.md")).unwrap();
    assert!(notes.contains("## 21st.dev — Three-tier pricing\n- Source: https://21st.dev/r/pricing (21st.dev)"), "{notes}");
    assert!(!notes.contains(".png") && !project.join("design/references/pricing-table").read_dir().unwrap().any(|e| e.unwrap().file_name() != "notes.md"), "no picture: {notes}");
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { text, .. } if text.contains("Kept “Three-tier pricing”"))));
    // Kept twice: said, not added again.
    handle.send(Command::KeepReference { squad: "acme".into(), session: Some(1), project: Some("site".into()), card });
    until(&mut events, |e| matches!(e, Event::Notice { session: 1, text, .. } if text.contains("already in the references"))).await;
    assert_eq!(std::fs::read_to_string(project.join("design/references/pricing-table/notes.md")).unwrap(), notes);
}

#[tokio::test]
async fn nothing_shows_when_no_picture_can_be_had() {
    let claude = claude();
    let project = scratch("site").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&project, "designer")).await;
    let answered = tool(&handle, "designer", json!({ "items": [{ "picture": "missing.png", "title": "Nothing", "link": "https://a.example.com/x" }] })).await;
    assert!(answered.is_error && answered.text.contains("nothing was shown"), "{}", answered.text);
}
