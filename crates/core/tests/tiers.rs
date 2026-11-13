//! Each ticket's model and effort, driven the way a screen and an engine
//! drive them: a lane starts on the tier its ticket's rules ask — whatever
//! model its member was left on, unless the person chose it — and the turns
//! record what they ran on; work sent back for correctness starts its next
//! lane one tier up with a handoff note, twice, and goes to the PM the third
//! time; a member handing its ticket back as beyond it raises it too; style
//! and environment send-backs raise nothing; the person's own tier wins; and
//! with the squad's model choice off, members' own models run as before.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::journal::Journal;
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::tiers::{ModelChoice, ModelTier, TierBy, TierReason};
use equipe_core::protocol::{Access, Command, Event, SessionSpec, Ticket, TicketStatus};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-tiers-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A stand-in `claude`, signed in: a ticket's turn takes a moment and writes
/// `HERO.md` where it runs, its prompt kept beside it ([`prompts`]); every
/// turn answers in a line.
fn claude() -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = scratch("bin").join("claude");
    let log = prompts(&bin);
    let answer = r#"cat <<EOF
{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"ok"}}}
{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{"input_tokens":3,"output_tokens":1}}
EOF"#;
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\nif [ \"$2\" = --input-format ] && IFS= read -r line; then set -- -p \"$(printf '%s' \"$line\" | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"message\"][\"content\"], end=\"\")')\" \"$@\"; fi\ncase \"$*\" in *\"You're on T-\"*) printf '%s\\n<<end>>\\n' \"$*\" >> '{}'; sleep 1; echo \"made by the squad\" > HERO.md;; esac\n{answer}\n",
        log.display()
    );
    let draft = bin.with_extension("draft");
    std::fs::write(&draft, script).unwrap();
    assert!(Process::new("cp").arg(&draft).arg(&bin).status().unwrap().success());
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
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

fn config(claude: &Path, home: &Path, persist: bool) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        codex_binary: scratch("missing").join("codex-does-not-exist").to_string_lossy().into_owned(),
        persist,
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
    if tokio::time::timeout(Duration::from_secs(20), wait).await.is_err() {
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

/// Where the stand-in `claude` at `bin` keeps the ticket prompts it was given.
fn prompts(bin: &Path) -> PathBuf {
    bin.with_file_name("prompts.log")
}

/// T-1 as the board has it now.
async fn ticket(handle: &Handle) -> Ticket {
    handle.snapshot("acme").await.tickets.into_iter().find(|t| t.id == 1).expect("on the board")
}

/// The `n`th ticket prompt the stand-in was given (from 1), once it was:
/// the arguments it ran with, the prompt among them.
async fn nth_prompt(bin: &Path, n: usize) -> String {
    for _ in 0..300 {
        // Lossy: engines appending big prompts at once can cut a multi-byte character in two.
        let log = String::from_utf8_lossy(&std::fs::read(prompts(bin)).unwrap_or_default()).into_owned();
        let all: Vec<&str> = log.split("<<end>>").filter(|p| p.contains("You're on T-")).collect();
        if let Some(found) = all.get(n - 1) {
            return (*found).to_owned();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no ticket prompt {n}: {}", std::fs::read_to_string(prompts(bin)).unwrap_or_default());
}

fn arg_after<'a>(prompt: &'a str, flag: &str) -> Option<&'a str> {
    prompt.split_whitespace().skip_while(|w| *w != flag).nth(1)
}

/// A squad of a PM and a frontend (Sonnet, its role's model) on one
/// repository, kept for its journal, T-1 under way on its first lane.
async fn started(title: &str, body: &str) -> (Handle, UnboundedReceiver<Event>, PathBuf, PathBuf) {
    started_as("frontend", title, body).await
}

/// The same, T-1 owned by a member of `role` (named after it).
async fn started_as(role: &str, title: &str, body: &str) -> (Handle, UnboundedReceiver<Event>, PathBuf, PathBuf) {
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let bin = claude();
    let (handle, mut events) = runtime::start(config(&bin, &home, true));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, role)).await;
    tool(&handle, "pm", "create_ticket", json!({ "title": title, "body": body, "owner": role })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    (handle, events, home, bin)
}

/// T-1's current lane hands in; the PM sends it back with `text` for `reason`;
/// its next lane starts.
async fn round(handle: &Handle, events: &mut UnboundedReceiver<Event>, text: &str, reason: &str) -> Vec<Event> {
    round_by(handle, events, "frontend", text, reason).await
}

async fn round_by(handle: &Handle, events: &mut UnboundedReceiver<Event>, member: &str, text: &str, reason: &str) -> Vec<Event> {
    let handed = tool(handle, member, "submit_for_review", json!({ "summary": "Done.", "changes_screens": false })).await;
    assert!(!handed.is_error, "{handed:?}");
    until(events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Review)).await;
    let back = tool(handle, "pm", "comment", json!({ "id": 1, "text": text, "review": "changes", "reason": reason })).await;
    assert!(!back.is_error, "{back:?}");
    until(events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await
}

fn raised(seen: &[Event]) -> bool {
    seen.iter().any(|e| matches!(e, Event::Notice { text, .. } if text.contains("moves to a stronger model")))
}

#[tokio::test]
async fn a_feature_starts_on_opus_at_medium_and_correctness_send_backs_climb_two_steps_then_go_to_the_pm() {
    let (handle, mut events, home, bin) = started("Add a settings page", "Run `npm test` to check.").await;
    // A feature is strong whatever the frontend member (Sonnet, its role's default) was left on: Opus at medium.
    let first = nth_prompt(&bin, 1).await;
    assert_eq!((arg_after(&first, "--model"), arg_after(&first, "--effort")), (Some("opus"), Some("medium")), "{first}");
    let model = ticket(&handle).await.model.expect("a model chosen");
    assert_eq!((model.tier, model.by, model.reason, model.model.as_deref(), model.classified), (Some(ModelTier::Strong), TierBy::Rule, TierReason::Code, Some("opus"), Some(TierReason::Code)));

    // First correctness send-back: its next lane starts one tier up, fresh, with a handoff note; the person is told.
    let seen = round(&handle, &mut events, "The form loses its values on save.", "correctness").await;
    assert!(raised(&seen), "{seen:#?}");
    let second = nth_prompt(&bin, 2).await;
    assert!(second.contains("Handoff: T-1 moves to a stronger model (deep)") && second.contains("The form loses its values on save."), "{second}");
    assert!(!second.contains("--resume"), "a fresh conversation: {second}");
    assert_eq!((arg_after(&second, "--model"), arg_after(&second, "--effort")), (Some("opus"), Some("high")));
    let model = ticket(&handle).await.model.unwrap();
    assert_eq!((model.tier, model.by, model.reason, model.escalations, model.fresh), (Some(ModelTier::Deep), TierBy::Escalation, TierReason::Escalated, 1, false));
    assert_eq!(model.classified, Some(TierReason::Code), "what the rules made of it is kept beside why it runs where it does");
    let record = Journal::for_squad(&home, "acme").unwrap().ticket_history(1).unwrap();
    assert!(record.iter().any(|(by, kind, _)| by == "equipe" && kind == "model raised"), "{record:?}");
    assert!(record.iter().any(|(_, kind, text)| kind == "handoff" && text.contains("branch")), "{record:?}");

    // Second: one more step.
    let seen = round(&handle, &mut events, "Saving still drops the email field.", "correctness").await;
    assert!(raised(&seen), "{seen:#?}");
    let third = nth_prompt(&bin, 3).await;
    assert!(third.contains("Handoff: T-1 moves to a stronger model (max)") && !third.contains("--resume"), "{third}");
    assert_eq!((arg_after(&third, "--model"), arg_after(&third, "--effort")), (Some("opus"), Some("xhigh")));
    assert_eq!(ticket(&handle).await.model.unwrap().escalations, 2);

    // Third: no higher; the PM decides.
    let seen = round(&handle, &mut events, "The email field is gone again.", "correctness").await;
    assert!(!raised(&seen));
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { session: 1, text, .. } if text.contains("the PM decides"))), "{seen:#?}");
    assert_eq!(ticket(&handle).await.model.unwrap().escalations, 2);
    let fourth = nth_prompt(&bin, 4).await;
    assert!(fourth.contains("--resume") && !fourth.contains("Handoff:"), "the raised lane carries on: {fourth}");

    // Every turn kept what it ran on; every choice and the raises are logged, with what the rules made of the ticket.
    let journal = Journal::for_squad(&home, "acme").unwrap();
    let turns = journal.ticket_turns(1).unwrap();
    assert!(!turns.is_empty() && turns.iter().all(|t| t.engine.as_deref() == Some("claude") && t.model.is_some()), "{turns:?}");
    let choices = journal.model_choices(1).unwrap();
    assert!(choices.iter().any(|c| c.what == "escalated" && c.tier.as_deref() == Some("deep") && c.by == "escalation"), "{choices:?}");
    assert_eq!(choices.iter().filter(|c| c.what == "lane").count(), 4, "{choices:?}");
    assert!(choices.iter().filter(|c| c.what == "lane").all(|c| c.classified.as_deref() == Some("code")), "{choices:?}");
    handle.shutdown().await;
}

#[tokio::test]
async fn routine_work_runs_on_sonnet_even_for_a_member_whose_role_defaults_to_opus_and_a_send_back_raises_it() {
    // A backend member's role default is Opus — equipe's, not the person's: no floor.
    let (handle, mut events, _home, bin) = started_as("backend", "Bump the serde dependency", "Run `cargo test`.").await;
    let first = nth_prompt(&bin, 1).await;
    assert_eq!((arg_after(&first, "--model"), arg_after(&first, "--effort")), (Some("sonnet"), Some("medium")), "{first}");
    let model = ticket(&handle).await.model.unwrap();
    assert_eq!((model.tier, model.reason, model.shadow), (Some(ModelTier::Standard), TierReason::Small, None));
    // Back wrong once: Opus at medium, fresh.
    let seen = round_by(&handle, &mut events, "backend", "The lockfile still has the old version.", "correctness").await;
    assert!(raised(&seen), "{seen:#?}");
    let second = nth_prompt(&bin, 2).await;
    assert!(second.contains("Handoff: T-1 moves to a stronger model (strong)") && !second.contains("--resume"), "{second}");
    assert_eq!((arg_after(&second, "--model"), arg_after(&second, "--effort")), (Some("opus"), Some("medium")));
    handle.shutdown().await;
}

#[tokio::test]
async fn a_model_the_person_set_for_a_member_is_the_floor_of_its_lanes() {
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let bin = claude();
    let (handle, mut events) = runtime::start(config(&bin, &home, true));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    // The person's own `/model`: opus, at Claude Code's own effort.
    handle.send(Command::SetModel { session: 2, model: "opus".into() });
    until(&mut events, |e| matches!(e, Event::ModelKnown { session: 2, .. })).await;
    tool(&handle, "pm", "create_ticket", json!({ "title": "Bump the serde dependency", "body": "Run `cargo test`.", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let first = nth_prompt(&bin, 1).await;
    assert_eq!(arg_after(&first, "--model"), Some("opus"), "{first}");
    let model = ticket(&handle).await.model.unwrap();
    assert_eq!((model.tier, model.reason, model.shadow, model.classified), (Some(ModelTier::Max), TierReason::MemberFloor, Some(ModelTier::Standard), Some(TierReason::Small)), "what the rules asked is recorded");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_member_handing_its_ticket_back_as_beyond_it_raises_its_next_lane() {
    let (handle, mut events, home, _bin) = started("Add a settings page", "Run `npm test` to check.").await;
    let handed = tool(&handle, "frontend", "hand_back", json!({ "reason": "cannot", "note": "It needs a design decision I can't make." })).await;
    assert!(!handed.is_error, "{handed:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Proposed)).await;
    let record = loop {
        let record = Journal::for_squad(&home, "acme").unwrap().ticket_history(1).unwrap();
        if record.iter().any(|(_, kind, _)| kind == "model raised") {
            break record;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(record.iter().any(|(_, kind, text)| kind == "model raised" && text.contains("beyond it")), "{record:?}");
    let model = ticket(&handle).await.model.unwrap();
    assert_eq!((model.tier, model.by, model.escalations), (Some(ModelTier::Deep), TierBy::Escalation, 1));
    handle.shutdown().await;
}

#[tokio::test]
async fn a_ticket_at_the_top_starts_fresh_and_says_so_rather_than_claiming_a_raise() {
    // A migration is risk: deep. The first send-back raises it to max; the second has nowhere to go.
    let (handle, mut events, home, bin) = started_as("backend", "Migrate the users table", "Run `cargo test`.").await;
    let model = ticket(&handle).await.model.unwrap();
    assert_eq!((model.tier, model.reason, model.shadow), (Some(ModelTier::Deep), TierReason::Risk, None));
    let seen = round_by(&handle, &mut events, "backend", "The index is missing.", "correctness").await;
    assert!(raised(&seen), "{seen:#?}");
    let seen = round_by(&handle, &mut events, "backend", "Still no index.", "correctness").await;
    assert!(!raised(&seen));
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { text, .. } if text.contains("strongest model already"))), "{seen:#?}");
    let third = nth_prompt(&bin, 3).await;
    assert!(third.contains("Handoff:") && !third.contains("--resume"), "{third}");
    assert_eq!((arg_after(&third, "--model"), arg_after(&third, "--effort")), (Some("opus"), Some("xhigh")));
    let record = Journal::for_squad(&home, "acme").unwrap().ticket_history(1).unwrap();
    assert!(record.iter().any(|(_, kind, text)| kind == "model raised" && text.contains("already at max")), "{record:?}");
    handle.shutdown().await;
}

#[tokio::test]
async fn style_and_environment_send_backs_never_raise_the_model() {
    let (handle, mut events, home, bin) = started("Add a settings page", "Run `npm test` to check.").await;
    for (n, (text, reason)) in [("Use sentence case.", "style"), ("The simulator was down.", "environment"), ("Tighter spacing.", "style")].into_iter().enumerate() {
        let seen = round(&handle, &mut events, text, reason).await;
        assert!(!raised(&seen), "{reason}: {seen:#?}");
        assert!(nth_prompt(&bin, n + 2).await.contains("--resume"));
    }
    let model = ticket(&handle).await.model.unwrap();
    assert_eq!((model.tier, model.escalations), (Some(ModelTier::Strong), 0));
    assert!(!Journal::for_squad(&home, "acme").unwrap().ticket_history(1).unwrap().iter().any(|(_, kind, _)| kind == "model raised"));
    handle.shutdown().await;
}

#[tokio::test]
async fn the_persons_tier_wins_lower_included_and_a_pin_keeps_escalation_off() {
    let (handle, mut events, home, bin) = started("Add a settings page", "Run `npm test` to check.").await;
    // Lowered by the person to standard and pinned: Sonnet at medium, from its next lane, fresh.
    handle.send(Command::SetTicketTier { squad: "acme".into(), ticket: 1, tier: Some(ModelTier::Standard), pinned: true });
    let told = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.model.as_ref().is_some_and(|m| m.by == TierBy::Person))).await;
    let Some(Event::TicketChanged { ticket: shown, .. }) = told.last() else { unreachable!() };
    let model = shown.model.clone().unwrap();
    assert_eq!((model.tier, model.pinned, model.model.as_deref(), model.effort.as_deref()), (Some(ModelTier::Standard), true, Some("sonnet"), Some("medium")), "shown at once");
    round(&handle, &mut events, "The form loses its values on save.", "correctness").await;
    let second = nth_prompt(&bin, 2).await;
    assert_eq!((arg_after(&second, "--model"), arg_after(&second, "--effort")), (Some("sonnet"), Some("medium")), "{second}");
    assert!(!second.contains("--resume"), "its tier changed: a fresh conversation");
    // Two correctness send-backs: pinned, so not raised.
    let seen = round(&handle, &mut events, "Still loses them.", "correctness").await;
    assert!(!raised(&seen));
    assert_eq!(ticket(&handle).await.model.unwrap().tier, Some(ModelTier::Standard));
    let record = Journal::for_squad(&home, "acme").unwrap().ticket_history(1).unwrap();
    assert!(record.iter().any(|(by, kind, text)| by == "you" && kind == "model" && text.contains("standard")), "{record:?}");
    // A member can't set a tier: the person's alone.
    assert!(equipe_core::permissions::person_only(&Command::SetTicketTier { squad: "acme".into(), ticket: 1, tier: None, pinned: false }));
    handle.shutdown().await;
}

#[tokio::test]
async fn with_model_choice_off_members_own_models_run_and_nothing_climbs() {
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let bin = claude();
    let (handle, mut events) = runtime::start(config(&bin, &home, true));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    assert_eq!(handle.snapshot("acme").await.model_choice, ModelChoice::QualityFirst, "the default");
    handle.send(Command::SetModelChoice { squad: "acme".into(), choice: ModelChoice::Off });
    until(&mut events, |e| matches!(e, Event::ModelChoiceChanged { choice: ModelChoice::Off, .. })).await;
    assert_eq!(handle.snapshot("acme").await.model_choice, ModelChoice::Off);
    tool(&handle, "pm", "create_ticket", json!({ "title": "Add a settings page", "body": "Run `npm test` to check.", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    assert_eq!(arg_after(&nth_prompt(&bin, 1).await, "--model"), Some("sonnet"), "the member's own");
    assert_eq!(ticket(&handle).await.model.unwrap().reason, TierReason::MemberOwn);
    for text in ["Wrong.", "Still wrong."] {
        assert!(!raised(&round(&handle, &mut events, text, "correctness").await));
    }
    assert_eq!(ticket(&handle).await.model.unwrap().escalations, 0);
    assert_eq!(Journal::for_squad(&home, "acme").unwrap().send_backs(1, equipe_core::protocol::tiers::SendBackReason::Correctness).unwrap(), 2, "still measured");
    handle.shutdown().await;
}

#[tokio::test]
async fn the_pm_reads_and_sets_model_choice_with_a_card_only_to_turn_it_on() {
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let bin = claude();
    let (handle, mut events) = runtime::start(config(&bin, &home, true));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    let read = tool(&handle, "pm", "settings", json!({})).await;
    assert!(read.text.contains("Model choice (model_choice): quality"), "{}", read.text);
    // Off only narrows: at once.
    let off = tool(&handle, "pm", "change_setting", json!({ "setting": "model_choice", "value": "off" })).await;
    assert!(!off.is_error && off.text.starts_with("Done:"), "{off:?}");
    until(&mut events, |e| matches!(e, Event::ModelChoiceChanged { choice: ModelChoice::Off, .. })).await;
    // Back on spends the plan faster: the person's tap.
    let handle2 = handle.clone();
    let asking = tokio::spawn(async move { tool(&handle2, "pm", "change_setting", json!({ "setting": "model_choice", "value": "quality", "why": "best models" })).await });
    let asked = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = asked.last() else { unreachable!() };
    assert_eq!(Journal::for_squad(&home, "acme").unwrap().model_choice(), ModelChoice::Off, "not before the tap");
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: equipe_core::protocol::Decision::Allow, words: None, said: None });
    until(&mut events, |e| matches!(e, Event::ModelChoiceChanged { choice: ModelChoice::QualityFirst, .. })).await;
    assert!(asking.await.unwrap().text.starts_with("The person said yes"));
    handle.shutdown().await;
}
