//! A side chat on the person's real Claude Code: a short conversation, then
//! a side chat about it that searches the web once — proving the fork knows
//! the conversation, searches, and leaves the original session's transcript
//! byte for byte as it was. Spends real tokens, so it runs only when asked:
//!
//! ```text
//! EQUIPE_LIVE_SIDE_CHAT=1 EQUIPE_LIVE_BIN=target/debug/equipe \
//!   cargo test -p equipe-core --test side_chat_live -- --ignored --nocapture
//! ```
//!
//! `EQUIPE_LIVE_BIN`: the `equipe` binary whose guard Claude Code asks
//! (`equipe hook --side-chat`); `EQUIPE_LIVE_MODEL`: the model (default
//! `haiku`, the cheapest). Equipe reads no credential: Claude Code signs
//! itself in, as always.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::{Access, Command, Event, SessionSpec, Usage};
use equipe_core::runtime::{self, Config};
use equipe_providers::CredentialStore;
use tokio::sync::mpsc::UnboundedReceiver;

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
    tokio::time::timeout(Duration::from_secs(300), wait).await.expect("the engine answers within five minutes");
    seen
}

/// Claude Code's transcript of session `id`, wherever it keeps it.
fn transcript(id: &str) -> Option<PathBuf> {
    let projects = PathBuf::from(std::env::var_os("HOME")?).join(".claude/projects");
    std::fs::read_dir(projects).ok()?.flatten().map(|d| d.path().join(format!("{id}.jsonl"))).find(|p| p.is_file())
}

fn bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

fn total(usage: &Option<Usage>) -> String {
    usage.as_ref().map_or_else(|| "?".into(), |u| format!("{} (input {:?}, cached {:?}, output {:?})", u.total(), u.input_tokens, u.cache_tokens, u.output_tokens))
}

#[tokio::test]
#[ignore = "spends real Claude Code tokens: EQUIPE_LIVE_SIDE_CHAT=1"]
async fn a_live_side_chat_forks_searches_and_leaves_the_session_as_it_was() {
    if std::env::var("EQUIPE_LIVE_SIDE_CHAT").as_deref() != Ok("1") {
        eprintln!("EQUIPE_LIVE_SIDE_CHAT=1 to run");
        return;
    }
    let scratch = std::env::temp_dir().join(format!("equipe-side-live-{}", std::process::id()));
    let (home, cwd) = (scratch.join("home"), scratch.join("project"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.canonicalize().unwrap();
    std::fs::write(cwd.join("NOTES.md"), "The release codename is tangerine.\n").unwrap();
    let model = std::env::var("EQUIPE_LIVE_MODEL").unwrap_or_else(|_| "haiku".into());
    let config = Config {
        store: CredentialStore::new(&home),
        home: home.clone(),
        client_version: "live".into(),
        claude_binary: "claude".into(),
        codex_binary: "/nonexistent/codex".into(),
        persist: false,
        retention_days: 90,
        equipe_binary: std::env::var_os("EQUIPE_LIVE_BIN").map(|b| PathBuf::from(b).canonicalize().unwrap()),
        npx: None,
        design_path: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        person_home: Some(home.join("person")),
        lang: equipe_core::i18n::Lang::En,
        nemotron: None,
        watch_disk: false,
    };
    let (handle, mut events) = runtime::start(config);
    let spec = SessionSpec { engine: Some(equipe_core::protocol::Engine::Claude), model: Some(model), access: Access::ReadOnly, cwd: cwd.clone(), role: None, squad: None };
    handle.send(Command::Start { session: 1, spec });
    until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
    handle.send(Command::Submit { session: 1, text: "We'll ship the next release on Rust's newest stable version. Reply with one short sentence.".into(), about: None });
    let main = until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    let main_usage = main.iter().find_map(|e| if let Event::TurnEnded { usage, .. } = e { Some(usage.clone()) } else { None }).flatten();
    // The main conversation's transcript: the only one in this project's folder
    // of Claude Code's (named after the folder, each other character a `-`).
    let folder = |dir: &Path| dir.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>();
    let kept = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".claude/projects").join(folder(&cwd));
    let listed = |dir: &Path| -> Vec<PathBuf> { std::fs::read_dir(dir).map(|d| d.flatten().map(|f| f.path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect()).unwrap_or_default() };
    let all = listed(&kept);
    assert_eq!(all.len(), 1, "one conversation in {}: {all:?}", kept.display());
    let original = all[0].clone();
    let id = original.file_stem().unwrap().to_string_lossy().into_owned();
    assert_eq!(transcript(&id).as_deref(), Some(original.as_path()));
    let before = bytes(&original);

    handle.send(Command::StartSideChat { session: 1, side: 9, passage: "Rust's newest stable version".into() });
    let opened = until(&mut events, |e| matches!(e, Event::SideChatOpened { .. })).await;
    assert!(matches!(opened.last(), Some(Event::SideChatOpened { search: true, .. })));
    handle.send(Command::SideChatSubmit {
        session: 1,
        side: 9,
        passage: "Rust's newest stable version".into(),
        text: "Search the web once: what is the newest stable Rust version right now? One line, with the source. Then say what we said we'd ship it on, in three words.".into(),
    });
    let side = until(&mut events, |e| matches!(e, Event::SideChat { event, .. } if matches!(**event, Event::TurnEnded { .. }))).await;
    let inner: Vec<Event> = side.iter().filter_map(|e| if let Event::SideChat { event, .. } = e { Some((**event).clone()) } else { None }).collect();
    let tools: Vec<(String, Option<String>)> = inner.iter().filter_map(|e| if let Event::ToolUse { tool, target, .. } = e { Some((tool.clone(), target.clone())) } else { None }).collect();
    let sources: Vec<String> = inner.iter().filter_map(|e| if let Event::ToolResult { tool, outcome, .. } = e { (tool == "WebSearch").then(|| outcome.output.clone()) } else { None }).collect();
    let answer: String = inner.iter().filter_map(|e| if let Event::TextDelta { text, .. } = e { Some(text.as_str()) } else { None }).collect();
    let side_usage = inner.iter().find_map(|e| if let Event::TurnEnded { usage, .. } = e { Some(usage.clone()) } else { None }).flatten();
    eprintln!("tools: {tools:?}\nsources: {sources:#?}\nanswer: {answer}\nmain turn tokens: {}\nside chat tokens: {}", total(&main_usage), total(&side_usage));
    let errors: Vec<&Event> = inner.iter().filter(|e| matches!(e, Event::Error { .. })).collect();
    assert!(errors.is_empty(), "{errors:?}");
    assert!(tools.iter().any(|(t, _)| t == "WebSearch"), "it searched the web: {tools:?}");
    assert!(!tools.iter().any(|(t, _)| !equipe_core::side_chat::CLAUDE_TOOLS.contains(&t.as_str())), "nothing but reading and the web: {tools:?}");
    assert!(!answer.trim().is_empty());
    assert!(side.iter().all(|e| e.session() != Some(1)), "nothing of it in the main conversation");
    let after = bytes(&original);
    assert!(after == before, "the original session's transcript is byte for byte as it was ({} → {} bytes)", before.len(), after.len());
    eprintln!("original transcript {} unchanged: {} bytes", original.display(), after.len());
    // The fork is a conversation of its own beside it.
    let forks: Vec<PathBuf> = listed(&kept).into_iter().filter(|p| *p != original).collect();
    eprintln!("forks: {forks:?}");
    assert_eq!(forks.len(), 1, "the side chat went on in a copy");
    handle.send(Command::CloseSideChat { side: 9 });
    until(&mut events, |e| matches!(e, Event::SideChatClosed { .. })).await;
}
