//! The person's requests, driven the way a screen and an engine drive them:
//! what they ask a member for becomes one request, and the tickets the
//! member puts on the board in answer serve it; tickets made while working on it join it; a
//! screen sees where it stands; a milestone is told once in the conversation
//! it was asked in; the member's next turn there starts with where it stands;
//! and asking after it while the member is busy is answered from the board.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::protocol::requests::{Link, Milestone, Phase, Request};
use equipe_core::protocol::rpc::{RunKey, ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Event, SessionSpec, TicketDraft, TicketStatus};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-requests-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn write_script(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A stand-in `claude` that writes down every turn's arguments (the prompt
/// first, as `-p`), one file a call; a prompt saying LONG keeps its turn
/// going a long while, one saying SLOW a few seconds, and a ticket's run
/// leaves a change in its worktree.
fn recording_claude() -> (PathBuf, PathBuf) {
    let calls = scratch("calls");
    let bin = scratch("bin").join("claude");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\n\
if [ \"$2\" = --input-format ] && IFS= read -r line; then set -- -p \"$(printf '%s' \"$line\" | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"message\"][\"content\"], end=\"\")')\" \"$@\"; fi\n\
n=0\nwhile ! mkdir '{dir}/.n-'$n 2>/dev/null; do n=$((n+1)); done\n\
for a in \"$@\"; do printf '%s\\n<<arg>>\\n' \"$a\"; done > '{dir}/.call-'$$\nmv '{dir}/.call-'$$ '{dir}/call-'$(printf %03d $n)-$$\n\
case \"$*\" in *LONG*) exec sleep 30;; *SLOW*) sleep 3; echo made > HERO.md;; *\"You're on T-\"*) echo made > HERO.md;; esac\n\
cat <<EOF\n{{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-test\",\"session_id\":\"sess-1\"}}\n\
{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"usage\":{{\"input_tokens\":3,\"output_tokens\":1}}}}\nEOF\n",
        dir = calls.display()
    );
    write_script(&bin, &script);
    (bin, calls)
}

/// Each recorded turn's prompt, oldest first.
fn prompts(dir: &Path) -> Vec<String> {
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("call-")).map(|e| e.path()).collect();
    files.sort();
    files
        .iter()
        .filter_map(|f| {
            let args: Vec<String> = std::fs::read_to_string(f).unwrap().split("\n<<arg>>\n").map(str::to_owned).collect();
            args.iter().position(|a| a == "-p").and_then(|i| args.get(i + 1).cloned())
        })
        .collect()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
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

/// Every event that came in meanwhile, without waiting.
fn drain(rx: &mut UnboundedReceiver<Event>) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Ok(event) = rx.try_recv() {
        seen.push(event);
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

/// A tool call from the run of `ticket`, as its board server makes it.
async fn tool_on(handle: &Handle, member: &str, ticket: u32, name: &str, arguments: serde_json::Value) -> ToolResult {
    let snapshot = handle.snapshot("acme").await;
    let run = snapshot.sessions.iter().find_map(|s| s.run.filter(|r| r.ticket == ticket).map(|r| RunKey { session: s.session, ticket, generation: r.generation }));
    handle.tool(ToolCall { run, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

async fn requests(handle: &Handle) -> Vec<Request> {
    handle.snapshot("acme").await.requests
}

fn news(events: &[Event]) -> Vec<(Milestone, String)> {
    events.iter().filter_map(|e| if let Event::RequestNews { milestone, line, .. } = e { Some((*milestone, line.clone())) } else { None }).collect()
}

fn notices(events: &[Event], to: u64) -> Vec<String> {
    events.iter().filter_map(|e| if let Event::Notice { session, text, .. } = e { (*session == to).then(|| text.clone()) } else { None }).collect()
}

#[tokio::test]
async fn what_the_person_asks_for_is_one_request_its_tickets_follow_and_its_news_is_told_once() {
    // SAFETY: every test in this file sets it the same, before its daemon starts.
    unsafe { std::env::set_var("EQUIPE_REQUEST_SETTLE", "0") };
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let (claude, calls) = recording_claude();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "designer")).await;
    add(&handle, &mut events, 3, member(&repo, "frontend")).await;

    // The PM's own ticket, with no word of the person's to answer: no request.
    tool(&handle, "pm", "create_ticket", json!({ "title": "Tidy the docs", "body": "-", "owner": "pm", "kind": "task" })).await;
    assert!(requests(&handle).await.is_empty());

    // The person asks the designer; its turn answers them, and files two tickets.
    handle.send(Command::Submit { session: 2, text: "A new input box and a new left menu\n(take your time, LONG)".into(), about: None });
    until(&mut events, |e| matches!(e, Event::UserMessage { session: 2, .. })).await;
    let spec = tool(&handle, "designer", "create_ticket", json!({ "title": "Spec the composer", "body": "-", "owner": "designer", "kind": "task" })).await;
    assert!(!spec.is_error && spec.text.contains("R-1"), "{spec:?}");
    let menu = tool(&handle, "designer", "create_ticket", json!({ "title": "Spec the left menu", "body": "-", "owner": "designer", "kind": "task" })).await;
    assert!(menu.text.contains("R-1"), "the same words, the same request: {menu:?}");
    let listed = requests(&handle).await;
    assert_eq!(listed.len(), 1, "{listed:#?}");
    let r = &listed[0];
    assert_eq!((r.id, r.title.as_str(), r.session, r.member.as_deref()), (1, "A new input box and a new left menu", Some(2), Some("designer")));
    assert_eq!(r.tickets.iter().map(|t| (t.ticket, t.link)).collect::<Vec<_>>(), [(2, Link::Asked), (3, Link::Asked)]);
    assert_eq!(r.status.parts.len(), 2, "each ticket asked for is a part");

    // A request named that doesn't exist is refused before anything is made.
    let odd = tool(&handle, "pm", "create_ticket", json!({ "title": "Build it", "body": "-", "request": 9 })).await;
    assert!(odd.is_error && odd.text.contains("no request R-9"), "{odd:?}");

    // The PM puts the build for the frontend on the board, and puts it with the request.
    tool(&handle, "pm", "create_ticket", json!({ "title": "Build the composer", "body": "-", "owner": "frontend" })).await;
    let linked = tool(&handle, "pm", "update_ticket", json!({ "id": 4, "request": "R-1" })).await;
    assert!(!linked.is_error && linked.text.contains("R-1"), "{linked:?}");
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 4, .. })).await;

    // While building it the frontend files work of its own (saying why it isn't the next item of
    // its ticket's checklist): it joins by its lineage.
    let follow = tool_on(&handle, "frontend", 4, "create_ticket", json!({ "title": "Check the composer on phones", "body": "-", "owner": "qa", "kind": "task", "separate_why": "A check on real phones, nothing on this branch" })).await;
    assert!(follow.text.contains("R-1"), "{follow:?}");
    let r = requests(&handle).await.remove(0);
    assert!(r.tickets.iter().any(|t| t.ticket == 5 && t.parent == Some(4) && t.link == Link::Made), "{:?}", r.tickets);
    assert!(r.status.parts.iter().any(|p| p.ticket == 4 && p.phase == Phase::Working), "{:?}", r.status.parts);

    // Asked after while the designer is busy: equipe answers from the board, at once.
    drain(&mut events);
    handle.send(Command::Submit { session: 2, text: "how's the composer going?".into(), about: None });
    let seen = until(&mut events, |e| matches!(e, Event::Notice { session: 2, text, .. } if text.starts_with("equipe answer"))).await;
    let answer = notices(&seen, 2).into_iter().find(|t| t.starts_with("equipe answer")).unwrap();
    assert!(answer.contains("designer works") && answer.contains("Build the composer — frontend is on it"), "{answer}");
    // …and that question isn't a new request.
    assert_eq!(requests(&handle).await.len(), 1);

    // Handed in, then accepted by the person: a milestone, told once, after it settles.
    let handed = tool_on(&handle, "frontend", 4, "submit_for_review", json!({ "summary": "Built.", "changes_screens": false })).await;
    assert!(!handed.is_error, "{handed:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 4 && ticket.status == TicketStatus::Review)).await;
    drain(&mut events);
    handle.minute_now().await;
    assert!(news(&drain(&mut events)).is_empty(), "handed in is no news yet");
    handle.send(Command::Accept { squad: "acme".into(), ticket: 4 });
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 4 && ticket.status == TicketStatus::Done)).await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    handle.minute_now().await;
    let seen = drain(&mut events);
    let told = news(&seen);
    assert_eq!(told.len(), 1, "{seen:#?}");
    assert!(matches!(told[0].0, Milestone::Done | Milestone::Live), "{told:?}");
    let line = notices(&seen, 2).into_iter().find(|t| t.starts_with("Your request")).expect("told in the conversation it was asked in");
    assert!(line.contains("“A new input box and a new left menu”") && line.contains("Build the composer — "), "{line}");
    handle.minute_now().await;
    assert!(news(&drain(&mut events)).is_empty(), "once each");

    // The designer's next turn in that conversation starts with where it stands.
    handle.send(Command::Interrupt { session: 2 });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    handle.send(Command::Submit { session: 2, text: "and the menu?".into(), about: None });
    // (What was steered into the turn stopped comes first, as a memo; the next turn, either way.)
    let mut last = None;
    for _ in 0..100 {
        last = prompts(&calls).into_iter().rfind(|p| p.contains("what the person asked for in this conversation") && p.contains("(T-2"));
        if last.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let last = last.expect("the turn ran");
    assert!(last.contains("R-1 “A new input box and a new left menu” (T-2, T-3, T-4, T-5)"), "{last}");
    assert!(last.contains("Spec the left menu — "), "{last}");

    handle.shutdown().await;
}

#[tokio::test]
async fn a_ticket_the_person_puts_on_the_board_is_a_request_told_in_the_pms_conversation() {
    // SAFETY: as above.
    unsafe { std::env::set_var("EQUIPE_REQUEST_SETTLE", "0") };
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let (claude, _calls) = recording_claude();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    let draft = TicketDraft { title: "Fix the footer links".into(), body: "-".into(), ..Default::default() };
    handle.send(Command::CreateTicket { squad: "acme".into(), draft });
    until(&mut events, |e| matches!(e, Event::RequestChanged { .. })).await;
    let listed = requests(&handle).await;
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].title.as_str(), listed[0].session), ("Fix the footer links", Some(1)));
    // Dropped: all of it is finished, and that is told once.
    handle.send(Command::Cancel { squad: "acme".into(), ticket: 1 });
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Cancelled)).await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    drain(&mut events);
    handle.minute_now().await;
    let seen = drain(&mut events);
    assert_eq!(news(&seen).iter().map(|(m, _)| *m).collect::<Vec<_>>(), [Milestone::AllDone], "{seen:#?}");
    assert!(notices(&seen, 1).iter().any(|t| t.contains("Fix the footer links — dropped") && t.contains("all done")), "{seen:#?}");
    assert!(requests(&handle).await[0].closed_at.is_some());
    handle.shutdown().await;
}

#[tokio::test]
async fn a_stopped_kind_of_work_is_cancelled_and_turned_away_until_the_person_lifts_it() {
    // SAFETY: as above.
    unsafe { std::env::set_var("EQUIPE_REQUEST_SETTLE", "0") };
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let (claude, calls) = recording_claude();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "designer")).await;
    add(&handle, &mut events, 3, member(&repo, "frontend")).await;

    handle.send(Command::Submit { session: 2, text: "Tidy up the component gallery\n(LONG)".into(), about: None });
    until(&mut events, |e| matches!(e, Event::UserMessage { session: 2, .. })).await;
    for title in ["Polish the component gallery spacing", "Component gallery dark mode"] {
        let made = tool(&handle, "designer", "create_ticket", json!({ "title": title, "body": "-", "owner": "designer", "kind": "task" })).await;
        assert!(made.text.contains("R-1"), "{made:?}");
    }

    // "Stop this" on the request: its open tickets go, and the stop stays in force.
    handle.send(Command::StopWork { squad: "acme".into(), request: Some(1), ticket: None, words: String::new() });
    let seen = until(&mut events, |e| matches!(e, Event::StopsChanged { stops, .. } if !stops.is_empty())).await;
    let Some(Event::StopsChanged { stops, .. }) = seen.last() else { unreachable!() };
    assert_eq!((stops[0].id, stops[0].words.as_str(), stops[0].terms.as_slice()), (1, "Tidy up the component gallery", &["component gallery".to_owned()][..]));
    assert_eq!(stops[0].cancelled, [1, 2]);
    let board = handle.snapshot("acme").await;
    assert!(board.tickets.iter().filter(|t| t.id <= 2).all(|t| t.status == TicketStatus::Cancelled), "{:?}", board.tickets);

    // New tickets of it are turned away, whoever files them, with why; others go on.
    let again = tool(&handle, "designer", "create_ticket", json!({ "title": "Tighten the component galleries grid", "body": "-", "kind": "task" })).await;
    assert!(again.is_error && again.text.contains("the person stopped this kind of work") && again.text.contains("ask the person first"), "{again:?}");
    let pm = tool(&handle, "pm", "create_ticket", json!({ "title": "Component gallery: fix card radius", "body": "-", "kind": "task" })).await;
    assert!(pm.is_error, "{pm:?}");
    let other = tool(&handle, "pm", "create_ticket", json!({ "title": "Build the checkout", "body": "-", "owner": "frontend" })).await;
    assert!(!other.is_error, "{other:?}");
    assert_eq!(handle.snapshot("acme").await.stops[0].refused, 2);

    // Only the person stops work: a member they didn't tell can't.
    let unasked = tool(&handle, "frontend", "stop_work", json!({ "words": "no more checkout", "terms": ["checkout"] })).await;
    assert!(unasked.is_error && unasked.text.contains("Only the person"), "{unasked:?}");
    // Said in chat: the member they told stops it.
    handle.send(Command::Submit { session: 1, text: "no more onboarding emails, please".into(), about: None });
    until(&mut events, |e| matches!(e, Event::UserMessage { session: 1, .. })).await;
    let said = tool(&handle, "pm", "stop_work", json!({ "words": "no more onboarding emails", "terms": ["onboarding email"] })).await;
    assert!(!said.is_error && said.text.contains("S-2"), "{said:?}");
    let email = tool(&handle, "designer", "create_ticket", json!({ "title": "Write the onboarding emails", "body": "-", "kind": "task" })).await;
    assert!(email.is_error && email.text.contains("S-2"), "{email:?}");

    // Every member's next turn starts with what is stopped.
    handle.send(Command::Interrupt { session: 2 });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    // The person asks for it again in their own words: that ticket goes on the board.
    handle.send(Command::Submit { session: 2, text: "ok, now do the component gallery spacing after all".into(), about: None });
    let mut told = false;
    for _ in 0..100 {
        told = prompts(&calls).iter().any(|p| p.contains("the person stopped these kinds of work") && p.contains("S-1 “Tidy up the component gallery”"));
        if told {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(told, "{:?}", prompts(&calls));
    let asked = tool(&handle, "designer", "create_ticket", json!({ "title": "Component gallery spacing", "body": "-", "kind": "task" })).await;
    assert!(!asked.is_error, "{asked:?}");

    // Lifted: that work may be filed again.
    handle.send(Command::LiftStop { squad: "acme".into(), stop: 1 });
    until(&mut events, |e| matches!(e, Event::StopsChanged { stops, .. } if stops.iter().all(|s| s.id != 1))).await;
    let lifted = tool(&handle, "pm", "create_ticket", json!({ "title": "Component gallery fonts", "body": "-", "kind": "task" })).await;
    assert!(!lifted.is_error, "{lifted:?}");
    handle.shutdown().await;
}

#[tokio::test]
async fn questions_about_a_settled_ticket_are_withdrawn_and_swept_from_before() {
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let (claude, _calls) = recording_claude();
    let set = {
        let (handle, mut events) = runtime::start(config(&claude, &home));
        add(&handle, &mut events, 1, member(&repo, "pm")).await;
        add(&handle, &mut events, 2, member(&repo, "frontend")).await;
        // Its run takes a few seconds: the question and the hand-in below come from a run still going
        // (one that ended at once raced them, and lost on a slow machine).
        tool(&handle, "pm", "create_ticket", json!({ "title": "Hero", "body": "SLOW", "owner": "frontend" })).await;
        until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
        let asked = tool_on(&handle, "frontend", 1, "ask_decision", json!({ "questions": [{ "question": "Which hero photo goes on the landing page?", "options": ["The team photo", "The product shot"] }] })).await;
        assert!(!asked.is_error, "{asked:?}");
        let seen = until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
        let Some(Event::QuestionsAsked { questions, .. }) = seen.last() else { unreachable!() };
        let set = questions.clone();
        assert_eq!(set.ticket, Some(1));
        // Handed in and accepted: the card about it goes, and the member hears why.
        let handed = tool_on(&handle, "frontend", 1, "submit_for_review", json!({ "summary": "Hero done.", "changes_screens": false })).await;
        assert!(!handed.is_error, "{handed:?}");
        until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Review)).await;
        assert_eq!(handle.snapshot("acme").await.questions.len(), 1, "still asked while in review");
        handle.send(Command::Accept { squad: "acme".into(), ticket: 1 });
        let seen = until(&mut events, |e| matches!(e, Event::QuestionsClosed { id, answers: None, .. } if *id == set.id)).await;
        let seen = [seen, until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("withdrawn"))).await].concat();
        assert!(seen.iter().any(|e| matches!(e, Event::Notice { text, .. } if text == "1 question about T-1 withdrawn: it is settled.")), "{seen:#?}");
        assert!(handle.snapshot("acme").await.questions.is_empty());
        handle.shutdown().await;
        set
    };
    // A card kept from before, about the ticket now done: swept when read.
    let journal = equipe_core::journal::Journal::for_squad(&home, "acme").unwrap();
    let stale = equipe_core::protocol::decide::QuestionSet { id: set.id + 1, ..set.clone() };
    let other = equipe_core::protocol::decide::QuestionSet { id: set.id + 2, ticket: None, ..set };
    journal.set_questions(&[stale, other.clone()]).unwrap();
    drop(journal);
    let (handle, _events) = runtime::start(config(&claude, &home));
    assert_eq!(handle.snapshot("acme").await.questions, [other], "only the one about no settled ticket");
    handle.shutdown().await;
}

#[tokio::test]
async fn small_handed_in_work_is_the_pms_to_decide_and_weighty_work_goes_up_with_why() {
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let (claude, _calls) = recording_claude();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    // The PM's gate is what this looks at: members don't land their own here (that never reaches a card at all).
    handle.send(Command::SetSelfLand { squad: "acme".into(), on: false });
    tool(&handle, "pm", "create_ticket", json!({ "title": "Tighten the hero spacing", "body": "-", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let handed = tool_on(&handle, "frontend", 1, "submit_for_review", json!({ "summary": "The hero sits closer to the menu, so the page reads at a glance. Tests pass.", "changes_screens": false })).await;
    assert!(!handed.is_error, "{handed:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Review)).await;

    // Small: no card — the PM is told to decide it, each ticket named by title and what changes.
    let small = tool(&handle, "pm", "propose_accept", json!({ "verdict": "accept", "tickets": [1] })).await;
    assert!(!small.is_error && small.text.starts_with("No card"), "{small:?}");
    assert!(small.text.contains("T-1 “Tighten the hero spacing” — The hero sits closer to the menu, so the page reads at a glance."), "{small:?}");
    assert!(handle.snapshot("acme").await.accepts.is_empty());

    // Weighty, by the PM's word: the card goes up, saying why.
    let weighty = tool(&handle, "pm", "propose_accept", json!({ "verdict": "accept", "tickets": [1], "why": "data", "summary": "Hero" })).await;
    assert!(!weighty.is_error && weighty.text.starts_with("Proposed"), "{weighty:?}");
    let card = handle.snapshot("acme").await.accepts.remove(0);
    assert_eq!(card.summary, "Hero · Needs your yes: “Tighten the hero spacing” (production data)");
    let odd = tool(&handle, "pm", "propose_accept", json!({ "verdict": "accept", "tickets": [1], "why": "vibes" })).await;
    assert!(odd.is_error || odd.text.starts_with("Already"), "{odd:?}");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_requests_ticket_isnt_chained_behind_other_work_and_a_member_unblocks_its_own() {
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let (claude, _calls) = recording_claude();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "designer")).await;
    // Internal work on the app (T-1), and work on it for qa, who isn't on the squad (T-2, never starts).
    tool(&handle, "pm", "create_ticket", json!({ "title": "Sidebar collapse code", "body": "Collapse the rail in src/components/Sidebar.tsx", "owner": "qa" })).await;

    // The person asks the designer for a prototype: chained behind T-1 without a reason, refused.
    handle.send(Command::Submit { session: 2, text: "A Linear-style live prototype, please\n(LONG)".into(), about: None });
    until(&mut events, |e| matches!(e, Event::UserMessage { session: 2, .. })).await;
    let proto = json!({ "title": "Live prototype of the new layout", "body": "In design/directions/linear-claude/", "owner": "qa", "kind": "task", "depends_on": [1] });
    let chained = tool(&handle, "designer", "create_ticket", proto.clone()).await;
    assert!(chained.is_error && chained.text.contains("work outside it") && chained.text.contains("depends_why"), "{chained:?}");
    // With a reason it may — and the places that don't overlap are pointed out all the same? Only without one.
    let mut why = proto.clone();
    why["depends_why"] = json!("It reuses the collapsed rail's motion");
    let made = tool(&handle, "designer", "create_ticket", why).await;
    assert!(!made.is_error && made.text.contains("R-1"), "{made:?}");

    // The PM can't chain it behind internal work either, without saying why.
    tool(&handle, "pm", "create_ticket", json!({ "title": "Icon swap in the app", "body": "src/icons/", "owner": "qa" })).await;
    let pm = tool(&handle, "pm", "update_ticket", json!({ "id": 2, "depends_on": [1, 3] })).await;
    assert!(pm.is_error && pm.text.contains("T-3"), "{pm:?}");

    // Outside a request, a dependency whose places overlap nothing is only pointed out.
    let loose = tool(&handle, "pm", "create_ticket", json!({ "title": "Docs for the rail", "body": "docs/rail.md", "owner": "qa", "depends_on": [1] })).await;
    assert!(!loose.is_error && loose.text.contains("nothing overlaps"), "{loose:?}");

    // qa isn't here; the designer's own ticket (T-2, owner qa) isn't its own: asking goes to the PM, now.
    let asked = tool(&handle, "designer", "unblock", json!({ "id": 2, "why": "The prototype is only in design/directions/" })).await;
    assert!(!asked.is_error && asked.text.contains("Asked the PM"), "{asked:?}");
    until(&mut events, |e| matches!(e, Event::SystemMessage { session: 1, text, .. } | Event::MessageQueued { session: 1, text, .. } if text.starts_with("[Unblock · designer]"))).await;

    // Its own ticket, not started: it drops the dependency itself, logged, and the PM is told.
    tool(&handle, "pm", "create_ticket", json!({ "title": "Prototype motion notes", "body": "design/directions/notes.md", "owner": "designer", "kind": "task", "depends_on": [1], "depends_why": "test" })).await;
    let own = tool(&handle, "designer", "unblock", json!({ "id": 5, "on": [1], "why": "Notes only, in design/directions/" })).await;
    assert!(!own.is_error && own.text.contains("no longer waits on T-1"), "{own:?}");
    let t5 = handle.snapshot("acme").await.tickets.into_iter().find(|t| t.id == 5).unwrap();
    assert!(t5.depends_on.is_empty(), "{t5:?}");
    let refused = tool(&handle, "designer", "unblock", json!({ "id": 5, "on": [1], "why": "again" })).await;
    assert!(refused.is_error, "{refused:?}");
    handle.shutdown().await;
}
