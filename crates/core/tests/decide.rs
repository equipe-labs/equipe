//! Decisions put to the person (`ask_decision`), driven the way a screen and
//! an engine drive them: the tool call in, the questions out as events, the
//! person's answers in as a command, and the member's next turn reading them.
//! A stand-in `claude` plays the engine.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::decide::{self, Answer};
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-decide-{name}-{}-{n}", std::process::id()));
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
n=$(ls '{dir}' | wc -l | tr -d ' ')\nfor a in \"$@\"; do printf '%s\\n<<arg>>\\n' \"$a\"; done > '{dir}/call-'$(printf %03d $n)-$$\n\
cat <<EOF\n{{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-test\",\"session_id\":\"sess-1\"}}\n\
{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"usage\":{{\"input_tokens\":3,\"output_tokens\":1}}}}\nEOF\n",
        dir = calls.display()
    );
    write_script(&bin, &script);
    (bin, calls)
}

/// Each recorded turn's arguments, oldest first.
fn calls(dir: &Path) -> Vec<Vec<String>> {
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    files.sort();
    files.iter().map(|f| std::fs::read_to_string(f).unwrap().split("\n<<arg>>\n").filter(|a| !a.is_empty()).map(str::to_owned).collect()).collect()
}

fn arg<'a>(call: &'a [String], flag: &str) -> Option<&'a str> {
    call.iter().position(|a| a == flag).and_then(|i| call.get(i + 1)).map(String::as_str)
}

fn config(claude: &Path, home: &Path, persist: bool) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        persist,
        retention_days: 90,
        equipe_binary: Some("/opt/equipe".into()),
        npx: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        lang: equipe_core::i18n::Lang::En,
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
    handle.tool(ToolCall { squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

/// The reference's two questions: one pick, then several.
fn two_questions() -> serde_json::Value {
    json!({ "questions": [
        { "question": "Which database should the API use?", "header": "Database", "options": [
            { "label": "Postgres (Recommended)", "description": "Relational, hosted on Supabase" },
            { "label": "SQLite", "description": "One file, no server" },
        ] },
        { "question": "Which features ship first?", "header": "Scope", "multi_select": true, "options": [
            { "label": "Auth" }, { "label": "Payments" }, { "label": "Search" },
        ] },
    ] })
}

fn asked(events: &[Event]) -> decide::QuestionSet {
    events.iter().find_map(|e| if let Event::QuestionsAsked { questions, .. } = e { Some(questions.clone()) } else { None }).expect("asked")
}

#[tokio::test]
async fn a_member_asks_the_person_decides_and_the_answers_start_its_next_turn() {
    let (claude, recorded) = recording_claude();
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    add(&handle, &mut events, 1, member(&project, "pm")).await;
    add(&handle, &mut events, 2, member(&project, "backend")).await;

    // Checked first: a question needs two options at least.
    let bad = tool(&handle, "backend", "ask_decision", json!({ "questions": [{ "question": "Which?", "options": ["only"] }] })).await;
    assert!(bad.is_error && bad.text.contains("2 to 4 options"), "{bad:?}");

    // Asked: it doesn't wait — the answer comes as the person's next message.
    let said = tool(&handle, "backend", "ask_decision", two_questions()).await;
    assert!(!said.is_error && said.text.contains("next message") && said.text.contains("end it now"), "{said:?}");
    let seen = until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
    let set = asked(&seen);
    assert_eq!((set.member.as_str(), set.session, set.ticket, set.questions.len()), ("backend", 2, None, 2));
    assert!(set.questions[1].multi_select && set.questions[0].options[0].description == "Relational, hosted on Supabase");
    assert!(set.asked_at > 0);
    // The PM hears of it in its conversation.
    let news = until(&mut events, |e| matches!(e, Event::Notice { session: 1, .. })).await;
    assert!(matches!(news.last(), Some(Event::Notice { text, .. }) if text == "backend asks you to decide 2 things · Which database should the API use?"), "{news:#?}");
    // A screen attaching sees it waiting.
    let snapshot = handle.snapshot("acme").await;
    assert_eq!(snapshot.questions, std::slice::from_ref(&set));

    // The person answers: Postgres; Auth and Payments, with a note.
    let answers = vec![Answer { picked: vec![0], other: None }, Answer { picked: vec![1, 0], other: Some("SSO later".into()) }];
    handle.send(Command::AnswerQuestions { squad: "acme".into(), id: set.id, answers, about: None });
    let seen = until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    let closed = seen.iter().find_map(|e| if let Event::QuestionsClosed { id, answers, .. } = e { Some((*id, answers.clone())) } else { None });
    assert_eq!(closed, Some((set.id, Some(vec![Answer { picked: vec![0], other: None }, Answer { picked: vec![0, 1], other: Some("SSO later".into()) }]))));
    let expected = "Answers to your questions:\n1) Which database should the API use? → Postgres (Recommended)\n2) Which features ship first? → Auth, Payments (+ note: SSO later)";
    assert!(seen.iter().any(|e| matches!(e, Event::UserMessage { session: 2, text } if text == expected)), "{seen:#?}");
    let prompt = calls(&recorded).last().map(|c| arg(c, "-p").unwrap_or_default().to_owned()).unwrap_or_default();
    assert!(prompt.ends_with(expected), "the member reads them as its next message: {prompt}");
    assert!(handle.snapshot("acme").await.questions.is_empty(), "no longer waiting");

    // Answered again from another screen: nothing more happens.
    handle.send(Command::AnswerQuestions { squad: "acme".into(), id: set.id, answers: vec![], about: None });
    handle.send(Command::Submit { session: 1, text: "status?".into(), about: None });
    let after = until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    assert!(!after.iter().any(|e| matches!(e, Event::QuestionsClosed { .. } | Event::UserMessage { session: 2, .. } | Event::Error { .. })), "{after:#?}");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_visual_choice_shows_its_pictures_found_where_the_member_works_and_a_blind_one_is_nudged() {
    let (claude, _recorded) = recording_claude();
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    add(&handle, &mut events, 1, member(&project, "pm")).await;
    add(&handle, &mut events, 2, member(&project, "designer")).await;
    // The designer sits in the squad's folder: its renders are there.
    let folder = equipe_core::squad_home::path(&home, "acme");
    std::fs::create_dir_all(folder.join("brand")).unwrap();
    for name in ["a.png", "b.png"] {
        image::RgbaImage::new(8, 8).save(folder.join("brand").join(name)).unwrap();
    }
    std::fs::write(project.join("elsewhere.png"), b"x").unwrap();
    let logo = |a: &str| json!({ "questions": [{ "question": "Which logo?", "options": [
        { "label": "Blue dot (Recommended)", "images": [a] },
        { "label": "Grey dot", "images": "brand/b.png" },
    ] }] });

    // A picture that isn't where it works: said, to fix — nothing is asked.
    let outside = tool(&handle, "designer", "ask_decision", logo(&project.join("elsewhere.png").to_string_lossy())).await;
    assert!(outside.is_error && outside.text.contains("option 1") && outside.text.contains("outside"), "{outside:?}");
    let climbing = tool(&handle, "designer", "ask_decision", logo("../../../../etc/hosts.png")).await;
    assert!(climbing.is_error && climbing.text.contains("`..`"), "{climbing:?}");

    let said = tool(&handle, "designer", "ask_decision", logo("brand/a.png")).await;
    assert!(!said.is_error && !said.text.contains("choosing blind"), "{said:?}");
    let set = asked(&until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await);
    let real = |name: &str| folder.join("brand").join(name).canonicalize().unwrap().to_string_lossy().into_owned();
    assert_eq!(set.questions[0].options[0].images, [real("a.png")]);
    assert_eq!(set.questions[0].options[1].images, [real("b.png")]);

    // Asked in words alone: it goes, and the member is told to show them next time.
    let blind = tool(&handle, "designer", "ask_decision", json!({ "questions": [{ "question": "로고 마스터는 어느 안으로 할까요?", "options": ["e. + 파란 원 점 (추천)", "e. 회색 점 유지"] }] })).await;
    assert!(!blind.is_error && blind.text.contains("Asked.") && blind.text.contains("choosing blind") && blind.text.contains("`images`"), "{blind:?}");
    handle.shutdown().await;
}

#[tokio::test]
async fn skipped_questions_say_so_and_a_closed_set_tells_the_member_to_go_on() {
    let (claude, _recorded) = recording_claude();
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    add(&handle, &mut events, 1, member(&project, "pm")).await;

    // The PM asks; the person skips both.
    tool(&handle, "pm", "ask_decision", two_questions()).await;
    let set = asked(&until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await);
    handle.send(Command::AnswerQuestions { squad: "acme".into(), id: set.id, answers: vec![Answer::skipped()], about: None });
    let seen = until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    let text = seen.iter().find_map(|e| if let Event::UserMessage { text, .. } = e { Some(text.clone()) } else { None }).unwrap();
    assert_eq!(text, "Answers to your questions:\n1) Which database should the API use? → skipped — your call\n2) Which features ship first? → skipped — your call");
    assert!(!seen.iter().any(|e| matches!(e, Event::Notice { session: 1, text } if text.contains("asks you to decide"))), "the PM isn't told of its own");

    // Asked again, and closed unanswered: the PM goes on without it.
    tool(&handle, "pm", "ask_decision", json!({ "questions": [{ "question": "Ship today?", "options": ["Yes (Recommended)", "No"] }] })).await;
    let set = asked(&until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await);
    handle.send(Command::DismissQuestions { squad: "acme".into(), id: set.id });
    let seen = until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::QuestionsClosed { id, answers: None, .. } if *id == set.id)), "{seen:#?}");
    let told = seen.iter().find_map(|e| if let Event::UserMessage { text, .. } = e { Some(text.clone()) } else { None }).unwrap();
    assert!(decide::is_dismissed(&told) && told.contains("(Ship today?)") && told.contains("best judgement"), "{told}");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_member_that_leaves_takes_its_questions_and_waiting_ones_survive_a_restart() {
    let (claude, _recorded) = recording_claude();
    let (home, project) = squad_home();
    {
        let (handle, mut events) = runtime::start(config(&claude, &home, true));
        add(&handle, &mut events, 1, member(&project, "pm")).await;
        add(&handle, &mut events, 2, member(&project, "backend")).await;
        add(&handle, &mut events, 3, member(&project, "qa")).await;
        tool(&handle, "backend", "ask_decision", two_questions()).await;
        let kept = asked(&until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await);
        tool(&handle, "qa", "ask_decision", json!({ "questions": [{ "question": "Test on Safari too?", "options": ["Yes", "No"] }] })).await;
        let gone = asked(&until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await);
        // qa leaves: its question no longer waits, and nobody is told anything.
        handle.send(Command::Close { session: 3 });
        let seen = until(&mut events, |e| matches!(e, Event::QuestionsClosed { .. })).await;
        assert!(matches!(seen.last(), Some(Event::QuestionsClosed { id, answers: None, .. }) if *id == gone.id), "{seen:#?}");
        assert_eq!(handle.snapshot("acme").await.questions, [kept]);
        handle.shutdown().await;
    }
    let (handle, _events) = runtime::start(config(&claude, &home, true));
    let snapshot = handle.snapshot("acme").await;
    assert_eq!(snapshot.questions.len(), 1, "still waiting after a restart");
    assert_eq!(snapshot.questions[0].member, "backend");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_member_with_equipes_tools_has_no_engine_question_tool() {
    let (claude, recorded) = recording_claude();
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&claude, &home, false));
    add(&handle, &mut events, 1, member(&project, "pm")).await;
    handle.send(Command::Submit { session: 1, text: "hi".into(), about: None });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    let call = calls(&recorded).pop().unwrap();
    let at = call.iter().position(|a| a == "--disallowedTools").expect("disallowed tools");
    assert!(call[at..].iter().any(|a| a == "AskUserQuestion"), "{call:?}");
    handle.shutdown().await;
}
