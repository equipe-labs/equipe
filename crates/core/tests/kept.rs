//! What the person sends stays: a sent message's attachments are never swept,
//! and the words of its text files (a voice memo written down, a long paste)
//! are kept with the message — in the journal, in what members read back, in
//! what a screen searches — after the files are gone.

#![cfg(unix)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use equipe_core::gateway;
use equipe_core::journal::Journal;
use equipe_core::kept;
use equipe_core::protocol::rpc::UploadKind;
use equipe_core::protocol::tags::Kept;
use equipe_core::protocol::{Access, Command, Entry, Event, SessionSpec};
use equipe_core::runtime::{self, Config};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-kept-it-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A stand-in `claude` that answers "ok" and writes down each turn's prompt.
fn claude(calls: &Path) -> PathBuf {
    let bin = scratch("bin").join("claude");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "auth" ]; then echo '{{"loggedIn":true,"authMethod":"claude.ai"}}'; exit 0; fi
if [ "$2" = --input-format ] && IFS= read -r line; then printf '%s' "$line" | python3 -c 'import json,sys; print(json.load(sys.stdin)["message"]["content"], end="")' > "{dir}/prompt-$$"; fi
cat <<EOF
{{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}}
{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"ok"}}}}}}
{{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{{"input_tokens":3,"output_tokens":1}}}}
EOF
"#,
        dir = calls.display()
    );
    std::fs::write(&bin, script).unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn prompts(calls: &Path) -> Vec<String> {
    std::fs::read_dir(calls).unwrap().flatten().map(|e| std::fs::read_to_string(e.path()).unwrap()).collect()
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
    let wait = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(event) = rx.recv().await {
            let stop = done(&event);
            seen.push(event);
            if stop {
                return;
            }
        }
    });
    assert!(wait.await.is_ok(), "timed out; saw {seen:#?}");
    seen
}

fn upload(home: &Path, name: &str, kind: UploadKind, bytes: &[u8]) -> PathBuf {
    let (mut file, path) = gateway::create_upload(home, name, Some(kind)).unwrap();
    file.write_all(bytes).unwrap();
    path
}

/// A home where squad `acme` works on a project.
fn home() -> (PathBuf, PathBuf) {
    let home = scratch("home");
    let project = scratch("project");
    squads::create(&home, "acme", &project).unwrap();
    (home, project)
}

const WORDS: &str = "Move the launch to Thursday and ship the pricing page first.\n";

#[tokio::test]
async fn a_voice_memo_sent_keeps_its_words_after_the_week_and_after_its_file_is_gone() {
    let (home, project) = home();
    let calls = scratch("calls");
    let claude = claude(&calls);
    // A voice memo and its words, as the Mac attaches them; a picture
    // attached to a draft never sent.
    let memo = upload(&home, "Samseong-dong.m4a", UploadKind::File, b"\0\0\0 ftypM4A audio");
    let transcript = upload(&home, "Samseong-dong-transcript.txt", UploadKind::File, WORDS.as_bytes());
    let draft = upload(&home, "draft.png", UploadKind::Image, b"png");
    let text = format!("sort this out [audio: {} · 0:31] [file: {} · 1 line]", memo.display(), transcript.display());
    {
        let (handle, mut events) = runtime::start(config(&claude, &home));
        handle.send(Command::Start { session: 1, spec: SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some("pm".into()), squad: Some("acme".into()) } });
        until(&mut events, |e| matches!(e, Event::SessionStarted { .. })).await;
        handle.send(Command::Submit { session: 1, text: text.clone(), about: None });
        let turn = until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
        // Every screen hears the words with the message…
        let said = turn.iter().find_map(|e| match e {
            Event::UserMessage { text: t, kept, .. } if *t == text => Some(kept.clone()),
            _ => None,
        });
        assert_eq!(said, Some(vec![Kept { path: transcript.display().to_string(), text: WORDS.into() }]));
        // …and the member's prompt is the message as sent: the tags, nothing added.
        let prompt = prompts(&calls).into_iter().find(|p| p.contains("sort this out")).expect("the PM's turn");
        assert!(prompt.ends_with(&text), "{prompt}");
        assert!(!prompt.contains("pricing page"), "the member reads the file itself: {prompt}");
        handle.shutdown().await;
    }

    // Eight days on, the sweep runs: the draft goes, what was sent stays.
    let later = SystemTime::now() + Duration::from_secs(8 * 24 * 3600);
    assert_eq!(gateway::sweep_uploads(&home, later), 1);
    assert!(!draft.exists(), "an unsent draft is swept");
    assert!(memo.exists() && transcript.exists(), "a sent message's files stay");

    // Even once its files are gone (the person removed them), the words stay.
    std::fs::remove_file(&memo).unwrap();
    std::fs::remove_file(&transcript).unwrap();
    let journal = Journal::for_squad(&home, "acme").unwrap();
    let sessions = journal.open_sessions().unwrap();
    let pm = sessions.iter().find(|s| s.member.as_ref().is_some_and(|m| m.role == "pm")).unwrap();
    let lines = journal.conversation(pm.id).unwrap();
    let kept_line = lines.iter().find_map(|e| match e {
        Entry::User { text: t, kept } if *t == text => Some(kept.clone()),
        _ => None,
    });
    assert_eq!(kept_line.as_deref().map(|k| k[0].text.as_str()), Some(WORDS), "in the journal");
    // What a member reads of the conversation (a side chat, a fresh start, the other engine).
    assert!(equipe_core::side_chat::summary(&lines).contains("ship the pricing page"), "{}", equipe_core::side_chat::summary(&lines));
    // A search over the conversation as a screen gets it back after a restart.
    let (handle, _events) = runtime::start(config(&claude, &home));
    let snapshot = handle.snapshot("acme").await;
    let recent = &snapshot.sessions.iter().find(|s| s.session == pm.id).unwrap().recent;
    let found = recent.iter().any(|e| matches!(e, Entry::User { kept, .. } if kept.iter().any(|k| k.text.to_lowercase().contains("pricing page"))));
    assert!(found, "{recent:#?}");
    handle.shutdown().await;
}

#[tokio::test]
async fn what_was_sent_before_is_rescued_once_on_the_first_start() {
    let (home, _project) = home();
    let calls = scratch("calls");
    let claude = claude(&calls);
    let shot = upload(&home, "screen.png", UploadKind::Image, b"png");
    let paste = upload(&home, "pasted.txt", UploadKind::File, "a long paste\nof two lines\n".as_bytes());
    let gone = gateway::uploads_dir(&home).join("1700000000000-gone.png");
    let draft = upload(&home, "draft.png", UploadKind::Image, b"png");
    let said = format!("look [image: {}] [file: {} · 2 lines] [image: {}]", shot.display(), paste.display(), gone.display());
    {
        // A journal from before: the message carries its tags only.
        let journal = Journal::for_squad(&home, "acme").unwrap();
        journal.record_conversation(40, None, &home).unwrap();
        journal.append(40, &Entry::User { text: said.clone(), kept: Vec::new() }).unwrap();
        journal.append(40, &Entry::Reply { text: "seen".into() }).unwrap();
    }
    let (handle, _events) = runtime::start(config(&claude, &home));
    handle.snapshot("acme").await;
    handle.shutdown().await;
    let names = kept::names(&home);
    for path in [&shot, &paste] {
        assert!(names.contains(path.file_name().unwrap().to_str().unwrap()), "{} kept", path.display());
    }
    assert!(!names.contains(draft.file_name().unwrap().to_str().unwrap()), "a draft isn't");
    let journal = Journal::for_squad(&home, "acme").unwrap();
    let lines = journal.conversation(40).unwrap();
    assert_eq!(lines[0], Entry::User { text: said.clone(), kept: vec![Kept { path: paste.display().to_string(), text: "a long paste\nof two lines\n".into() }] });
    // The sweep a week on leaves them.
    gateway::sweep_uploads(&home, SystemTime::now() + Duration::from_secs(8 * 24 * 3600));
    assert!(shot.exists() && paste.exists() && !draft.exists());

    // Once: a second start finds it done; doing it again would change nothing.
    assert_eq!(kept::rescue(&home, &journal).unwrap(), None);
    let index = std::fs::read_to_string(home.join(kept::INDEX)).unwrap();
    let (handle, _events) = runtime::start(config(&claude, &home));
    handle.snapshot("acme").await;
    handle.shutdown().await;
    assert_eq!(std::fs::read_to_string(home.join(kept::INDEX)).unwrap(), index);
    assert_eq!(Journal::for_squad(&home, "acme").unwrap().conversation(40).unwrap(), lines);
}
