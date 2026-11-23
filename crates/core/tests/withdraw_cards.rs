//! Cards taken away when they are no longer needed (`withdraw_card`), driven
//! the way a screen and an engine drive them: the PM withdraws a member's
//! secret request and it leaves every screen — the snapshot, Needs you —
//! with its reason said, the member told it was withdrawn; a member takes
//! back only its own; a ticket that closes or is sent back takes its cards
//! with it; and a PM reply that says to ignore a card still up brings a
//! reminder with its next turn, once.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::protocol::rpc::{RunKey, ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Event, SessionSpec, TicketStatus};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-withdraw-{name}-{}-{n}", std::process::id()));
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
/// first, as `-p`), one file a call; a prompt saying SLOW takes a few
/// seconds, and one saying IGNORECARD is answered "ignore the token card".
fn recording_claude() -> (PathBuf, PathBuf) {
    let calls = scratch("calls");
    let bin = scratch("bin").join("claude");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\n\
if [ \"$2\" = --input-format ] && IFS= read -r line; then set -- -p \"$(printf '%s' \"$line\" | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"message\"][\"content\"], end=\"\")')\" \"$@\"; fi\n\
n=0\nwhile ! mkdir '{dir}/.n-'$n 2>/dev/null; do n=$((n+1)); done\n\
for a in \"$@\"; do printf '%s\\n<<arg>>\\n' \"$a\"; done > '{dir}/.call-'$$\nmv '{dir}/.call-'$$ '{dir}/call-'$(printf %03d $n)-$$\n\
reply=ok\n\
case \"$*\" in *SLOW*) sleep 3; echo made > HERO.md;; *IGNORECARD*) reply='Please ignore the token card from frontend: it is no longer needed.';; *\"You're on T-\"*) echo made > HERO.md;; esac\n\
cat <<EOF\n{{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-test\",\"session_id\":\"sess-1\"}}\n\
{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"$reply\",\"usage\":{{\"input_tokens\":3,\"output_tokens\":1}}}}\nEOF\n",
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

/// The first recorded prompt that `wanted` picks, waited for a while.
async fn prompt_with(dir: &Path, wanted: impl Fn(&str) -> bool) -> String {
    for _ in 0..150 {
        if let Some(p) = prompts(dir).into_iter().find(|p| wanted(p)) {
            return p;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no such prompt; prompts so far: {:#?}", prompts(dir));
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

/// The Vercel token request a frontend member makes the old way.
fn token_request() -> serde_json::Value {
    json!({
        "name": "vercel-deploy-token",
        "service": "vercel",
        "kind": "token",
        "why": "Add your Vercel deploy token for the website",
        "steps": ["Sign in to Vercel", "Open Account Settings > Tokens", "Create a token and copy it"],
        "create_url": "https://vercel.com/account/tokens",
    })
}

fn notices(events: &[Event], to: u64) -> Vec<String> {
    events.iter().filter_map(|e| if let Event::Notice { session, text, .. } = e { (*session == to).then(|| text.clone()) } else { None }).collect()
}

/// A squad with a PM (1) and a frontend member (2), and its daemon.
async fn squad() -> (Handle, UnboundedReceiver<Event>, PathBuf, PathBuf) {
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let (claude, calls) = recording_claude();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    (handle, events, calls, repo)
}

#[tokio::test]
async fn the_pm_withdraws_a_members_secret_request_and_it_leaves_every_screen_with_its_reason() {
    let (handle, mut events, calls, _repo) = squad().await;
    let asked = tool(&handle, "frontend", "request_secret", token_request()).await;
    assert!(!asked.is_error, "{asked:?}");
    let seen = until(&mut events, |e| matches!(e, Event::SecretAsked { .. })).await;
    let Some(Event::SecretAsked { ask: card, .. }) = seen.last().cloned() else { unreachable!() };
    assert_eq!(handle.snapshot("acme").await.secret_asks.len(), 1, "the card is up for the person");

    // Neither which card nor which ticket: the PM is shown what it may withdraw.
    let listed = tool(&handle, "pm", "withdraw_card", json!({})).await;
    assert!(!listed.is_error && listed.text.contains(&format!("secret {} by frontend", card.id)) && listed.text.contains("vercel-deploy-token"), "{listed:?}");
    // Without a reason the person can read, nothing is withdrawn.
    let bare = tool(&handle, "pm", "withdraw_card", json!({ "kind": "secret", "id": card.id })).await;
    assert!(bare.is_error && bare.text.contains("`why`"), "{bare:?}");
    assert_eq!(handle.snapshot("acme").await.secret_asks.len(), 1);
    // Nor one that isn't there.
    let wrong = tool(&handle, "pm", "withdraw_card", json!({ "kind": "decision", "id": card.id, "why": "x" })).await;
    assert!(wrong.is_error && wrong.text.contains(&format!("secret {}", card.id)), "what is open is listed: {wrong:?}");

    let done = tool(&handle, "pm", "withdraw_card", json!({ "kind": "secret", "id": card.id, "why": "deploys go through GitHub now; no token is needed" })).await;
    assert!(!done.is_error && done.text.contains("Withdrawn"), "{done:?}");
    let seen = until(&mut events, |e| matches!(e, Event::SecretClosed { saved: false, .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::SecretClosed { id, saved: false, .. } if *id == card.id)), "{seen:#?}");
    // Gone from what every screen draws its composer and Needs you from.
    let snapshot = handle.snapshot("acme").await;
    assert!(snapshot.secret_asks.is_empty(), "{:?}", snapshot.secret_asks);
    // The person reads why, in the PM's conversation and the member's.
    let seen = [seen, until(&mut events, |e| matches!(e, Event::Notice { session: 2, .. })).await].concat();
    let line = "pm withdrew frontend's card: vercel-deploy-token — Add your Vercel deploy token for the website — deploys go through GitHub now; no token is needed";
    assert!(notices(&seen, 1).iter().any(|n| n == line), "{:#?}", notices(&seen, 1));
    assert!(notices(&seen, 2).iter().any(|n| n == line), "{:#?}", notices(&seen, 2));
    // The member is set going with the news: withdrawn, not "the person said no".
    let woken = prompt_with(&calls, |p| p.contains("pm withdrew your card")).await;
    assert!(woken.contains("deploys go through GitHub now") && woken.contains("don't ask for it again") && !woken.contains("didn't give"), "{woken}");
    // Nothing left to withdraw.
    let again = tool(&handle, "pm", "withdraw_card", json!({ "kind": "secret", "id": card.id, "why": "x" })).await;
    assert!(again.is_error, "{again:?}");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_member_takes_back_only_its_own_cards_and_the_pm_hears_of_it() {
    let (handle, mut events, calls, _repo) = squad().await;
    tool(&handle, "frontend", "request_secret", token_request()).await;
    let seen = until(&mut events, |e| matches!(e, Event::SecretAsked { .. })).await;
    let Some(Event::SecretAsked { ask: card, .. }) = seen.last().cloned() else { unreachable!() };
    let asked = tool(&handle, "pm", "ask_decision", json!({ "questions": [{ "question": "Ship Friday?", "options": [{ "label": "Yes", "recommended": true }, { "label": "No" }] }] })).await;
    assert!(!asked.is_error, "{asked:?}");
    let seen = until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
    let Some(Event::QuestionsAsked { questions: set, .. }) = seen.last().cloned() else { unreachable!() };

    // The PM's decision isn't the member's to take back; its own listing shows only its own.
    let theirs = tool(&handle, "frontend", "withdraw_card", json!({ "kind": "decision", "id": set.id, "why": "x" })).await;
    assert!(theirs.is_error, "{theirs:?}");
    let listed = tool(&handle, "frontend", "withdraw_card", json!({})).await;
    assert!(listed.text.contains(&format!("secret {}", card.id)) && !listed.text.contains("Ship Friday?"), "{listed:?}");

    // Its own, by kind and number: gone, the PM told with its next message.
    let done = tool(&handle, "frontend", "withdraw_card", json!({ "kind": "secret", "id": card.id, "why": "found a way without it" })).await;
    assert!(!done.is_error, "{done:?}");
    let seen = until(&mut events, |e| matches!(e, Event::SecretClosed { saved: false, .. })).await;
    let seen = [seen, until(&mut events, |e| matches!(e, Event::Notice { session: 2, .. })).await].concat();
    assert!(notices(&seen, 2).iter().any(|n| n.starts_with("frontend withdrew its card: vercel-deploy-token") && n.ends_with("— found a way without it")), "{seen:#?}");
    assert!(handle.snapshot("acme").await.secret_asks.is_empty());
    assert_eq!(handle.snapshot("acme").await.questions.len(), 1, "the PM's card stays");
    handle.send(Command::Submit { session: 1, text: "how is it going?".into(), about: None });
    let told = prompt_with(&calls, |p| p.contains("how is it going?")).await;
    assert!(told.contains("frontend withdrew card(s) (found a way without it)"), "{told}");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_ticket_that_closes_or_is_sent_back_takes_its_secret_cards_with_it() {
    let (handle, mut events, _calls, _repo) = squad().await;
    // Its run takes a few seconds: the requests below come from a run still going.
    tool(&handle, "pm", "create_ticket", json!({ "title": "Hero", "body": "SLOW", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let asked = tool_on(&handle, "frontend", 1, "request_secret", token_request()).await;
    assert!(!asked.is_error, "{asked:?}");
    let seen = until(&mut events, |e| matches!(e, Event::SecretAsked { .. })).await;
    let Some(Event::SecretAsked { ask: card, .. }) = seen.last().cloned() else { unreachable!() };
    assert_eq!(card.ticket, Some(1));

    // Handed in, then sent back by the person with a new direction: the card asked under the old one goes.
    let handed = tool_on(&handle, "frontend", 1, "submit_for_review", json!({ "summary": "Hero done.", "changes_screens": false })).await;
    assert!(!handed.is_error, "{handed:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Review)).await;
    assert_eq!(handle.snapshot("acme").await.secret_asks.len(), 1, "still asked while in review");
    handle.send(Command::Reopen { squad: "acme".into(), ticket: 1, note: "Deploy with GitHub Actions instead".into() });
    let seen = until(&mut events, |e| matches!(e, Event::SecretClosed { id, saved: false, .. } if *id == card.id)).await;
    let seen = [seen, until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("withdrew frontend's card on T-1"))).await].concat();
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { text, .. } if text.ends_with("— its ticket was sent back with a new direction"))), "{seen:#?}");
    assert!(handle.snapshot("acme").await.secret_asks.is_empty());

    // Back at work, it asks again; then the ticket is cancelled: that card goes too.
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let again = tool_on(&handle, "frontend", 1, "request_secret", token_request()).await;
    assert!(!again.is_error, "{again:?}");
    let seen = until(&mut events, |e| matches!(e, Event::SecretAsked { .. })).await;
    let Some(Event::SecretAsked { ask: card, .. }) = seen.last().cloned() else { unreachable!() };
    handle.send(Command::Cancel { squad: "acme".into(), ticket: 1 });
    let seen = until(&mut events, |e| matches!(e, Event::SecretClosed { id, saved: false, .. } if *id == card.id)).await;
    let seen = [seen, until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("withdrew frontend's card on T-1") && text.contains("settled"))).await].concat();
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { text, .. } if text.ends_with("— its ticket is settled"))), "{seen:#?}");
    assert!(handle.snapshot("acme").await.secret_asks.is_empty());
    handle.shutdown().await;
}

#[tokio::test]
async fn a_pm_reply_that_says_to_ignore_a_card_still_up_is_reminded_once_at_its_next_turn() {
    let (handle, mut events, calls, _repo) = squad().await;
    tool(&handle, "frontend", "request_secret", token_request()).await;
    let seen = until(&mut events, |e| matches!(e, Event::SecretAsked { .. })).await;
    let Some(Event::SecretAsked { ask: card, .. }) = seen.last().cloned() else { unreachable!() };

    // The PM only says it: "ignore the token card" — the card is still up.
    handle.send(Command::Submit { session: 1, text: "IGNORECARD what about the token?".into(), about: None });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    assert_eq!(handle.snapshot("acme").await.secret_asks.len(), 1, "saying so takes nothing away");
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    handle.send(Command::Submit { session: 1, text: "second message".into(), about: None });
    let next = prompt_with(&calls, |p| p.contains("second message")).await;
    assert!(next.contains("told the person a card is withdrawn") && next.contains(&format!("secret {} by frontend", card.id)) && next.contains("`withdraw_card`"), "{next}");
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;

    // Said again about the same card: not reminded twice.
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    handle.send(Command::Submit { session: 1, text: "IGNORECARD again".into(), about: None });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    handle.send(Command::Submit { session: 1, text: "third message".into(), about: None });
    let third = prompt_with(&calls, |p| p.contains("third message")).await;
    assert!(!third.contains("told the person a card is withdrawn"), "{third}");
    handle.shutdown().await;
}

#[tokio::test]
async fn the_pm_withdraws_an_ask_person_card_and_its_member_hears_it_in_the_answer() {
    let (handle, mut events, _calls, _repo) = squad().await;
    // The member's yes-card: its call waits on the person's answer.
    let asking = {
        let handle = handle.clone();
        tokio::spawn(async move {
            tool(&handle, "frontend", "ask_person", json!({ "action": "Create a Vercel team for the site", "why": "to deploy it", "summary": { "changes": "A new Vercel team", "who_sees": "you", "undo": "delete the team" }, "kind": "outside" })).await
        })
    };
    let seen = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = seen.last().cloned() else { unreachable!() };
    assert_eq!(handle.snapshot("acme").await.approvals.len(), 1);
    let listed = tool(&handle, "pm", "withdraw_card", json!({ "kind": "ask" })).await;
    assert!(listed.text.contains(&format!("ask {} by frontend", approval.id)), "{listed:?}");

    let done = tool(&handle, "pm", "withdraw_card", json!({ "kind": "ask", "id": approval.id, "why": "the site deploys from GitHub; no Vercel team is needed" })).await;
    assert!(!done.is_error, "{done:?}");
    let seen = until(&mut events, |e| matches!(e, Event::ApprovalResolved { approval: id, .. } if *id == approval.id)).await;
    assert!(seen.iter().any(|e| matches!(e, Event::ApprovalResolved { decision: equipe_core::protocol::Decision::Deny, .. })), "{seen:#?}");
    assert!(handle.snapshot("acme").await.approvals.is_empty(), "gone from Needs you");
    let answered = tokio::time::timeout(Duration::from_secs(10), asking).await.expect("its call is answered").unwrap();
    assert!(answered.text.contains("pm withdrew this card: the site deploys from GitHub"), "{answered:?}");
    assert!(answered.text.contains("did not say no") && !answered.text.contains("said no ("), "withdrawn, not the person's no: {answered:?}");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_new_plan_for_a_waiting_ticket_takes_the_cards_asked_under_the_old_one() {
    let (handle, mut events, _calls, _repo) = squad().await;
    tool(&handle, "pm", "create_ticket", json!({ "title": "Deploy", "body": "SLOW", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let asked = tool_on(&handle, "frontend", 1, "request_secret", token_request()).await;
    assert!(!asked.is_error, "{asked:?}");
    let seen = until(&mut events, |e| matches!(e, Event::SecretAsked { .. })).await;
    let Some(Event::SecretAsked { ask: card, .. }) = seen.last().cloned() else { unreachable!() };
    let down = tool_on(&handle, "frontend", 1, "block_on_person", json!({ "needs": ["Create a Vercel token and paste it on the card"], "ask": "Add the Vercel token", "effort_minutes": 2 })).await;
    assert!(!down.is_error, "{down:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Waiting)).await;

    // The PM changes only who it is for: nothing is taken away.
    let same = tool(&handle, "pm", "update_ticket", json!({ "id": 1, "owner": "frontend" })).await;
    assert!(!same.is_error && !same.text.contains("taken away"), "{same:?}");
    assert_eq!(handle.snapshot("acme").await.secret_asks.len(), 1);
    // A new plan: the token asked under the old one goes, and the PM is told so.
    let replanned = tool(&handle, "pm", "update_ticket", json!({ "id": 1, "body": "Deploy with GitHub Actions; no Vercel token." })).await;
    assert!(!replanned.is_error && replanned.text.contains("1 card(s)") && replanned.text.contains("taken away"), "{replanned:?}");
    let seen = until(&mut events, |e| matches!(e, Event::SecretClosed { id, saved: false, .. } if *id == card.id)).await;
    let seen = [seen, until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("withdrew frontend's card on T-1"))).await].concat();
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { text, .. } if text.ends_with("— its ticket's plan changed"))), "{seen:#?}");
    assert!(handle.snapshot("acme").await.secret_asks.is_empty());
    handle.shutdown().await;
}
