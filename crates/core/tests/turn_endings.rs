//! A member's turn on a ticket that ends without ending the work never sits
//! until its lease runs out. Driven the way engines and screens drive it,
//! from the cases seen in a squad's day: an engine that fails within
//! seconds, again and again, for one ticket and for several at once; an
//! engine that reaches its plan's limit mid-turn; a member that ends its
//! turn with only a comment; and a member that hands its ticket back.
//! Stand-in engines play the members; nothing reaches a network.
//!
//! Waits are run as of a later time ([`Handle::turn_ends_due_at`],
//! [`Handle::check_leases_at`]) instead of waiting minutes.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use equipe_core::journal::Journal;
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{
    Access, Command, Engine, Event, ExecutionState, PauseWhy, PlanUsage, PlanWindow, SessionSpec, SessionState, Ticket, TicketStatus, stall_reason,
};
use equipe_core::runtime::{self, Config, Handle, turn_ending};
use equipe_core::squads;
use equipe_providers::{AuthMethod, CredentialStore, Credentials};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-turn-endings-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

/// How the stand-in engine's next turns go.
#[derive(Clone, Copy)]
enum Mode {
    /// It answers in a line and ends — without a board call.
    Answers,
    /// It takes two seconds, then answers in a line.
    Slow,
    /// It fails at once, having done nothing, saying it can't resume its conversation.
    FailsFast,
    /// The conversation it was asked to resume is gone (or another engine's).
    Lost,
    /// It ends on Claude Code's limit message.
    Limited,
    /// It goes on until it is stopped.
    Hangs,
}

/// A stand-in `claude`, signed in. Every turn's prompt is kept in
/// `prompts.log`; how a turn goes is what [`Mode`] it was last set to.
struct Engine0 {
    bin: PathBuf,
}

impl Engine0 {
    fn new(mode: Mode) -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("bin");
        let bin = dir.join("claude");
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = "auth" ]; then echo '{{"loggedIn":true,"authMethod":"claude.ai"}}'; exit 0; fi
if [ "$2" = --input-format ] && IFS= read -r line; then set -- -p "$(printf '%s' "$line" | python3 -c 'import json,sys; print(json.load(sys.stdin)["message"]["content"], end="")')" "$@"; fi
printf '%s\n<<end>>\n' "$*" >> '{log}'
mode=$(cat '{mode}' 2>/dev/null)
echo '{{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}}'
case "$mode" in
  fails) echo '{{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["API Error: 500 Internal server error"]}}'; exit 1;;
  lost) echo '{{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["No conversation found with session ID: sess-0"]}}'; exit 1;;
  limited) echo '{{"type":"result","subtype":"success","is_error":false,"result":"Claude AI usage limit reached|1900000000","usage":{{"input_tokens":3,"output_tokens":1}}}}'; exit 0;;
  hangs) exec sleep 30;;
  slow) sleep 2;;
esac
echo '{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"ok"}}}}}}'
echo '{{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{{"input_tokens":3,"output_tokens":1}}}}'
"#,
            log = dir.join("prompts.log").display(),
            mode = dir.join("mode").display(),
        );
        let draft = dir.join("claude.draft");
        std::fs::write(&draft, script).unwrap();
        assert!(Process::new("cp").arg(&draft).arg(&bin).status().unwrap().success());
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let engine = Engine0 { bin };
        engine.set(mode);
        engine
    }

    fn set(&self, mode: Mode) {
        let word = match mode {
            Mode::Answers => "answers",
            Mode::Slow => "slow",
            Mode::FailsFast => "fails",
            Mode::Lost => "lost",
            Mode::Limited => "limited",
            Mode::Hangs => "hangs",
        };
        std::fs::write(self.bin.with_file_name("mode"), word).unwrap();
    }

    fn prompts(&self) -> Vec<String> {
        // Lossy: engines appending big prompts at once can cut a multi-byte character in two.
        let log = String::from_utf8_lossy(&std::fs::read(self.bin.with_file_name("prompts.log")).unwrap_or_default()).into_owned();
        log.split("<<end>>").map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned).collect()
    }

    /// The prompts that said `wanted`.
    fn said(&self, wanted: &str) -> Vec<String> {
        self.prompts().into_iter().filter(|p| p.contains(wanted)).collect()
    }

    async fn prompt_with(&self, wanted: &str) -> String {
        for _ in 0..400 {
            if let Some(p) = self.prompts().into_iter().rev().find(|p| p.contains(wanted)) {
                return p;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("no prompt said {wanted:?}: {:#?}", self.prompts());
    }

    /// Wait until `n` prompts have said `wanted`.
    async fn said_times(&self, wanted: &str, n: usize) {
        for _ in 0..400 {
            if self.said(wanted).len() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("{wanted:?} said {} times, not {n}: {:#?}", self.said(wanted).len(), self.prompts());
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A home with the squad `acme` on a fresh repository, and a Codex login
/// made up for the test.
fn squad() -> (PathBuf, PathBuf) {
    let repo = scratch("repo");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let login = Credentials {
        provider: "openai".into(),
        method: AuthMethod::OauthSubscription,
        access: "test-only".into(),
        refresh: None,
        expires: Some(4_000_000_000),
        account_id: None,
        residency: None,
    };
    CredentialStore::new(&home).save(&login).unwrap();
    (home, repo)
}

/// The daemon's setup; `tools`: members reach the board (an equipe binary
/// to serve it — never run here).
fn config(claude: &Path, home: &Path, tools: bool) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        codex_binary: "/nonexistent/codex".into(),
        persist: true,
        retention_days: 90,
        equipe_binary: tools.then(|| "/opt/equipe".into()),
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

async fn within(rx: &mut UnboundedReceiver<Event>, wait: Duration) -> Vec<Event> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(wait, async {
        while let Some(event) = rx.recv().await {
            seen.push(event);
        }
    })
    .await;
    seen
}

fn member(cwd: &Path, role: &str) -> SessionSpec {
    SessionSpec { engine: Some(Engine::Claude), model: None, access: Access::EditFiles, cwd: cwd.to_owned(), role: Some(role.into()), squad: Some("acme".into()) }
}

async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, spec: SessionSpec) {
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

async fn ticket(handle: &Handle, id: u32) -> Ticket {
    handle.snapshot("acme").await.tickets.into_iter().find(|t| t.id == id).expect("on the board")
}

async fn create(handle: &Handle, title: &str, owner: &str) {
    let made = tool(handle, "pm", "create_ticket", json!({ "title": title, "body": "Do it.", "owner": owner })).await;
    assert!(!made.is_error, "{made:?}");
}

fn execution(t: &Ticket) -> equipe_core::protocol::TicketExecution {
    *t.execution.clone().expect("the daemon keeps the ticket's run")
}

/// Wait until `id`'s run shows paused for `why`.
async fn paused(handle: &Handle, id: u32, why: PauseWhy) -> Ticket {
    for _ in 0..200 {
        let t = ticket(handle, id).await;
        if t.execution.as_ref().and_then(|e| e.pause.as_ref()).is_some_and(|p| p.why == why) {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("T-{id} never paused for {why:?}: {:#?}", ticket(handle, id).await);
}

/// Wait until `id`'s run is no longer paused.
async fn going(handle: &Handle, id: u32) {
    for _ in 0..200 {
        if ticket(handle, id).await.execution.as_ref().is_some_and(|e| e.pause.is_none()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("T-{id} stayed paused: {:#?}", ticket(handle, id).await);
}

fn counted(home: &Path, kind: &str) -> u64 {
    let journal = Journal::for_squad(home, "acme").unwrap();
    journal.turn_endings_since(0).unwrap().into_iter().find(|(k, _)| k == kind).map_or(0, |(_, n)| n)
}

/// A turn its engine fails within seconds, having done nothing, waits and
/// goes again — 30 seconds, then 2 minutes — never at once and never in a
/// loop; its ticket stays in progress, never stalled, its card saying why
/// and when; and it goes on when the engine works again.
#[tokio::test]
async fn an_engine_that_fails_fast_is_retried_after_a_growing_wait_never_in_a_loop() {
    let engine = Engine0::new(Mode::FailsFast);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home, false));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    let start = now();
    create(&handle, "Hero", "frontend").await;
    engine.prompt_with("You're on T-1").await;

    let failed = paused(&handle, 1, PauseWhy::Retrying).await;
    let pause = execution(&failed).pause.unwrap();
    assert_eq!((failed.status, execution(&failed).state), (TicketStatus::InProgress, ExecutionState::Active), "the same run, in progress");
    assert_eq!(pause.attempt, 1);
    assert!(pause.cause.as_deref().is_some_and(|c| c.contains("API Error: 500")), "what the engine said: {pause:?}");
    let first = pause.resumes_at.unwrap();
    assert!((start + 30..=now() + 30).contains(&first), "30 seconds on: {first} from {start}");
    // No hot loop: nothing runs it meanwhile.
    within(&mut events, Duration::from_millis(800)).await;
    assert_eq!(engine.said("You're on T-1").len(), 1, "one try so far");
    // However late the lease check, a run equipe paused is not a stall.
    handle.check_leases_at(now() + 1_000).await;
    assert_eq!(execution(&ticket(&handle, 1).await).state, ExecutionState::Active);
    // Before its time, nothing.
    handle.turn_ends_due_at(first - 1).await;
    within(&mut events, Duration::from_millis(300)).await;
    assert_eq!(engine.said("You're on T-1").len(), 1);

    // Its time: asked again the same, with why — and it fails again: 2 minutes now.
    handle.turn_ends_due_at(first).await;
    engine.said_times("You're on T-1", 2).await;
    let second = loop {
        let t = paused(&handle, 1, PauseWhy::Retrying).await;
        let p = execution(&t).pause.unwrap();
        if p.attempt == 2 {
            break p;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(second.resumes_at.unwrap() >= now() + 100, "two minutes: {second:?}");
    assert_eq!(counted(&home, turn_ending::FAILED_FAST), 2);

    // The engine works again: the next try goes, and the ticket goes on.
    engine.set(Mode::Answers);
    handle.turn_ends_due_at(second.resumes_at.unwrap()).await;
    engine.said_times("You're on T-1", 3).await;
    going(&handle, 1).await;
    let t = ticket(&handle, 1).await;
    assert_eq!((t.status, execution(&t).state), (TicketStatus::InProgress, ExecutionState::Active));
    handle.shutdown().await;
}

/// Conversations that can't be resumed (gone, or another engine's after a
/// move) start fresh at once, with the ticket's brief — equipe's own
/// bookkeeping, so however many at once, the squad's breaker stays shut.
#[tokio::test]
async fn conversations_that_cant_be_resumed_start_fresh_at_once_and_never_pause_the_squad() {
    let engine = Engine0::new(Mode::Lost);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home, false));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    add(&handle, &mut events, 3, member(&repo, "backend")).await;
    add(&handle, &mut events, 4, member(&repo, "qa")).await;
    create(&handle, "Hero", "frontend").await;
    create(&handle, "API", "backend").await;
    create(&handle, "Checks", "qa").await;
    for id in 1..=3 {
        engine.prompt_with(&format!("You're on T-{id}")).await;
    }
    // Each goes again with a fresh conversation, due now — no backoff.
    let t = paused(&handle, 1, PauseWhy::Retrying).await;
    assert!(execution(&t).pause.unwrap().resumes_at.unwrap() <= now() + 1, "at once: {t:?}");
    engine.set(Mode::Answers);
    handle.turn_ends_due_at(now()).await;
    let again = engine.prompt_with("couldn't be resumed, so this is a fresh one").await;
    assert!(!again.contains("--resume"), "{again}");
    assert_eq!(counted(&home, turn_ending::SQUAD_PAUSED), 0, "never the breaker");
    assert_eq!(counted(&home, turn_ending::FAILED_FAST), 0, "not an engine failing");
    handle.shutdown().await;
}

/// Several tickets failing fast at once — one cause for all —
/// open the squad's breaker: nothing new starts and nothing retries, the
/// person is told once with what the engine said; a test turn now and then
/// looks again, and when it works the squad goes on by itself.
#[tokio::test]
async fn several_fast_failures_at_once_pause_the_squad_until_a_test_turn_works() {
    let engine = Engine0::new(Mode::FailsFast);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home, false));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    add(&handle, &mut events, 3, member(&repo, "backend")).await;
    add(&handle, &mut events, 4, member(&repo, "qa")).await;
    create(&handle, "Hero", "frontend").await;
    create(&handle, "API", "backend").await;
    create(&handle, "Checks", "qa").await;
    let told = |e: &Event| matches!(e, Event::Notice { text, .. } if text.contains("failed at once") && text.contains("API Error: 500"));
    let seen = until(&mut events, told).await;
    for id in 1..=3 {
        paused(&handle, id, PauseWhy::SquadPaused).await;
    }
    assert_eq!(counted(&home, turn_ending::SQUAD_PAUSED), 1);

    // Paused: new work waits, and no retry goes, not even when it is due.
    create(&handle, "Footer", "frontend").await;
    handle.turn_ends_due_at(now() + 60).await;
    let later = within(&mut events, Duration::from_millis(800)).await;
    assert!(!later.iter().any(|e| matches!(e, Event::WorkStarted { ticket: 4, .. })), "nothing new starts");
    assert_eq!(ticket(&handle, 4).await.status, TicketStatus::Todo);
    let tries = engine.said("You're on T-").len();
    assert_eq!(tries, 3, "one try each, no retry: {:#?}", engine.prompts());
    // Told once.
    assert_eq!(seen.iter().chain(&later).filter(|e| told(e)).count(), 1);

    // The cause is gone: the test turn works, and everything goes on.
    engine.set(Mode::Answers);
    handle.turn_ends_due_at(now() + 3 * 60).await;
    until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("The engines work again"))).await;
    handle.turn_ends_due_at(now() + 10 * 60).await;
    for id in 1..=3 {
        going(&handle, id).await;
    }
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 4, .. })).await;
    handle.shutdown().await;
}

/// The person saying anything to a member of a paused squad tries a test turn at once.
#[tokio::test]
async fn a_paused_squad_tries_again_at_once_when_the_person_says_something() {
    let engine = Engine0::new(Mode::FailsFast);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home, false));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    for (id, role) in [(2, "frontend"), (3, "backend"), (4, "qa")] {
        add(&handle, &mut events, id, member(&repo, role)).await;
        create(&handle, role, role).await;
    }
    until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("failed at once"))).await;
    engine.set(Mode::Answers);
    let tries = engine.said("You're on T-").len();
    handle.send(Command::Submit { session: 1, text: "I fixed it.".into(), about: None });
    engine.said_times("You're on T-", tries + 1).await;
    until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("The engines work again"))).await;
    handle.shutdown().await;
}

/// The engine's limit reached mid-turn: the turn is kept as limited, not
/// completed; the run is paused for that engine — never stalled, never
/// requeued — and goes on by itself, in the same conversation, once the
/// engine has room again.
#[tokio::test]
async fn a_turn_cut_short_by_the_engines_limit_pauses_and_goes_on_when_it_has_room() {
    let engine = Engine0::new(Mode::Limited);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home, true));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    create(&handle, "Hero", "frontend").await;
    engine.prompt_with("You're on T-1").await;
    let limited = paused(&handle, 1, PauseWhy::EngineLimit).await;
    let pause = execution(&limited).pause.unwrap();
    assert_eq!(pause.engine, Some(Engine::Claude));
    assert!(pause.resumes_at.is_some_and(|at| at > now()), "it looks again later: {pause:?}");
    until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("reached its usage limit during T-1"))).await;
    let turns = Journal::for_squad(&home, "acme").unwrap().turns_since(0).unwrap();
    assert!(turns.iter().any(|t| t.ticket == Some(1) && t.outcome == turn_ending::LIMITED), "kept as limited: {turns:#?}");
    assert!(!turns.iter().any(|t| t.ticket == Some(1) && t.outcome == "completed"), "not as completed: {turns:#?}");
    // Not a turn without an end: it isn't reminded or handed to the PM.
    assert_eq!(engine.said("ended without ending the work").len(), 0);
    handle.check_leases_at(now() + 10_000).await;
    let still = ticket(&handle, 1).await;
    assert_eq!((still.status, execution(&still).state), (TicketStatus::InProgress, ExecutionState::Active), "not stalled");

    // Nothing said it has room: it waits.
    handle.turn_ends_due_at(now() + 60).await;
    within(&mut events, Duration::from_millis(300)).await;
    assert_eq!(engine.said("You're on T-1").len(), 1);
    // Read with room (after it paused): it goes on, the same run.
    engine.set(Mode::Hangs);
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let resets = now() as i64 + 3 * 3600;
    handle.usage_read(Engine::Claude, PlanUsage { five_hour: Some(PlanWindow { used_percent: 10, resets_at: resets }), seven_day: Some(PlanWindow { used_percent: 20, resets_at: resets + 86_400 }) });
    let again = engine.prompt_with("has room again").await;
    assert!(again.contains("T-1") && again.contains("--resume sess-1"), "the same conversation, carried on: {again}");
    going(&handle, 1).await;
    assert_eq!(execution(&ticket(&handle, 1).await).generation, execution(&still).generation, "the same run");
    assert_eq!(counted(&home, turn_ending::LIMITED), 1);
    handle.shutdown().await;
}

/// An engine out of its plan, read so, with the other engine's room: the
/// switching rules move the member, and its paused ticket goes with it —
/// handed over, not stalled.
#[tokio::test]
async fn a_ticket_paused_for_an_engines_limit_goes_with_its_member_to_the_other_engine() {
    let engine = Engine0::new(Mode::Limited);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home, true));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "qa")).await;
    create(&handle, "Checks", "qa").await;
    paused(&handle, 1, PauseWhy::EngineLimit).await;
    let resets = now() as i64 + 4 * 86_400;
    handle.usage_read(Engine::Codex, PlanUsage { five_hour: Some(PlanWindow { used_percent: 5, resets_at: resets - 3 * 86_400 }), seven_day: Some(PlanWindow { used_percent: 5, resets_at: resets }) });
    handle.usage_read(Engine::Claude, PlanUsage { five_hour: Some(PlanWindow { used_percent: 100, resets_at: resets - 3 * 86_400 }), seven_day: Some(PlanWindow { used_percent: 100, resets_at: resets }) });
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.note.as_deref().is_some_and(|n| n.starts_with("Handed over from Claude to Codex")))).await;
    assert!(!seen.iter().any(|e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.execution.as_ref().is_some_and(|x| x.state == ExecutionState::Stalled))), "never stalled");
    handle.shutdown().await;
}

/// A turn that ends with no end to the work — only words — is resumed once,
/// in the same conversation, with the ways to finish; ended so again, its
/// ticket goes to the PM with the member's last word on it as the reason.
#[tokio::test]
async fn a_turn_without_an_end_is_reminded_once_then_handed_to_the_pm() {
    let engine = Engine0::new(Mode::Slow);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home, true));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    create(&handle, "Hero", "frontend").await;
    engine.prompt_with("You're on T-1").await;
    let said = tool(&handle, "frontend", "comment", json!({ "id": 1, "text": "Can't start yet: T-9 isn't in production." })).await;
    assert!(!said.is_error, "{said:?}");

    let nudge = engine.prompt_with("ended without ending the work").await;
    for way in ["submit_for_review", "block_on_person", "hand_back", "waiting_on", "duplicate", "not_my_role", "cannot"] {
        assert!(nudge.contains(way), "{way}: {nudge}");
    }
    until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("equipe reminded it how to finish"))).await;
    // Ended the same way again: to the PM, with its last word.
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Proposed)).await;
    let back = ticket(&handle, 1).await;
    assert!(back.handed_back(), "{back:#?}");
    let note = back.note.clone().unwrap();
    assert!(note.contains("its turns kept ending without a hand-in") && note.contains("Can't start yet: T-9 isn't in production."), "{note}");
    assert!(back.worktree.is_some() && back.branch.is_some(), "its work kept");
    assert_eq!(execution(&back).state, ExecutionState::Stopped);
    assert!(!seen.iter().any(|e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.execution.as_ref().is_some_and(|x| x.state == ExecutionState::Stalled))), "never stalled");
    // The PM is woken to settle it.
    let woken = engine.prompt_with("Settle it now").await;
    assert!(woken.contains("T-1"), "{woken}");
    assert_eq!(engine.said("ended without ending the work").len(), 1, "reminded once");
    assert_eq!((counted(&home, turn_ending::NO_END), counted(&home, turn_ending::NUDGED), counted(&home, turn_ending::TO_PM)), (2, 1, 1));
    // The PM may settle it: here, it cancels it.
    let cancelled = tool(&handle, "pm", "update_ticket", json!({ "id": 1, "status": "cancelled" })).await;
    assert!(!cancelled.is_error, "{cancelled:?}");
    handle.shutdown().await;
}

/// A turn the person stopped is theirs: not reminded, not handed on — and
/// the lease is still the last net.
#[tokio::test]
async fn a_turn_the_person_stopped_is_left_alone_and_the_lease_is_still_the_net() {
    let engine = Engine0::new(Mode::Hangs);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home, true));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    create(&handle, "Hero", "frontend").await;
    engine.prompt_with("You're on T-1").await;
    let run = handle.snapshot("acme").await.sessions.iter().find(|s| s.run.is_some_and(|r| r.of == 2)).map(|s| s.session).unwrap();
    engine.set(Mode::Answers);
    handle.send(Command::Interrupt { session: run });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session, .. } if *session == run)).await;
    within(&mut events, Duration::from_millis(600)).await;
    assert_eq!(engine.said("ended without ending the work").len(), 0, "not reminded");
    let idle = handle.snapshot("acme").await;
    assert!(idle.sessions.iter().any(|s| s.session == run && s.state == SessionState::Ready && s.work.is_some()), "it sits on its ticket");
    handle.check_leases_at(now() + 1_000).await;
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.execution.as_ref().is_some_and(|x| x.state == ExecutionState::Stalled))).await;
    assert_eq!(execution(&ticket(&handle, 1).await).reason.as_deref(), Some(stall_reason::LEASE_EXPIRED));
    handle.shutdown().await;
}

/// The run of member `of`, once open.
async fn run_of(handle: &Handle, of: u64) -> u64 {
    for _ in 0..200 {
        if let Some(run) = handle.snapshot("acme").await.sessions.iter().find(|s| s.run.is_some_and(|r| r.of == of)).map(|s| s.session) {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("member {of} has no run");
}

/// `hand_back` waiting on another ticket: it waits on the board, and comes
/// back to its member, in the same worktree, once that one is done.
#[tokio::test]
async fn a_ticket_handed_back_to_wait_on_another_comes_back_when_that_one_is_done() {
    let engine = Engine0::new(Mode::Hangs);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home, true));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    // T-1 is nobody's yet: it stays open.
    create(&handle, "Deploy the API", "data").await;
    create(&handle, "Hero", "frontend").await;
    engine.prompt_with("You're on T-2").await;
    let run = run_of(&handle, 2).await;

    for (args, says) in [
        (json!({ "reason": "waiting_on", "note": "-" }), "Name the tickets"),
        (json!({ "reason": "waiting_on", "tickets": [2], "note": "-" }), "the ticket you are on"),
        (json!({ "reason": "waiting_on", "tickets": [99], "note": "-" }), "There is no T-99"),
        (json!({ "reason": "later", "note": "-" }), "Say `reason`"),
        (json!({ "reason": "cannot" }), "Say why in `note`"),
    ] {
        let refused = tool(&handle, "frontend", "hand_back", args).await;
        assert!(refused.is_error && refused.text.contains(says), "{says}: {refused:?}");
    }
    let handed = tool(&handle, "frontend", "hand_back", json!({ "reason": "waiting_on", "tickets": [1], "note": "Can't start yet: T-1 isn't in production." })).await;
    assert!(!handed.is_error && handed.text.contains("waiting on T-1"), "{handed:?}");
    engine.set(Mode::Hangs);
    handle.send(Command::Interrupt { session: run });
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 2 && ticket.status == TicketStatus::Todo && !ticket.depends_on.is_empty())).await;
    let waiting = ticket(&handle, 2).await;
    assert_eq!((waiting.depends_on.clone(), waiting.owner.as_deref(), waiting.assignee.as_deref()), (vec![1], Some("frontend"), None));
    assert!(waiting.worktree.is_some(), "its worktree kept");
    assert_eq!(execution(&waiting).state, ExecutionState::Stopped);
    assert!(!handle.snapshot("acme").await.sessions.iter().any(|s| s.run.is_some_and(|r| r.of == 2)), "its run is over");
    // Not handed to the PM: it waits on the board for T-1.
    assert_eq!(engine.said("Settle it now").len(), 0);
    let later = within(&mut events, Duration::from_millis(500)).await;
    assert!(!later.iter().any(|e| matches!(e, Event::WorkStarted { ticket: 2, .. })), "it waits: {later:#?}");

    // T-1 done (here: dropped): T-2 comes back to frontend, where it left off.
    let dropped = tool(&handle, "pm", "update_ticket", json!({ "id": 1, "status": "cancelled" })).await;
    assert!(!dropped.is_error, "{dropped:?}");
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 2, of: Some(2), .. })).await;
    let back = engine.prompt_with("You handed this ticket back to wait on T-1").await;
    assert!(back.contains("Can't start yet: T-1 isn't in production."), "{back}");
    assert_eq!(ticket(&handle, 2).await.worktree, waiting.worktree, "the same worktree");
    assert_eq!(counted(&home, turn_ending::HANDED_BACK), 1);
    handle.shutdown().await;
}

/// `hand_back` as a duplicate, another role's work, or something that can't
/// be done: the ticket goes back among the proposals with the reason, its
/// work kept, and the PM is woken to settle it.
#[tokio::test]
async fn a_ticket_handed_back_for_the_pm_says_why_and_wakes_the_pm() {
    for (reason, extra, says) in [
        ("duplicate", json!({ "tickets": [1] }), "a duplicate"),
        ("not_my_role", json!({ "suggest": "backend" }), "Suggests: backend."),
        ("cannot", json!({}), "can't be done as asked"),
    ] {
        let engine = Engine0::new(Mode::Hangs);
        let (home, repo) = squad();
        let (handle, mut events) = runtime::start(config(&engine.bin, &home, true));
        add(&handle, &mut events, 1, member(&repo, "pm")).await;
        add(&handle, &mut events, 2, member(&repo, "frontend")).await;
        create(&handle, "Search, merged already", "data").await;
        create(&handle, "Search", "frontend").await;
        engine.prompt_with("You're on T-2").await;
        let run = run_of(&handle, 2).await;
        let mut args = json!({ "reason": reason, "note": "Found while starting." });
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let handed = tool(&handle, "frontend", "hand_back", args).await;
        assert!(!handed.is_error && handed.text.contains("goes to the PM"), "{reason}: {handed:?}");
        handle.send(Command::Interrupt { session: run });
        until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 2 && ticket.status == TicketStatus::Proposed)).await;
        let back = ticket(&handle, 2).await;
        let note = back.note.clone().unwrap_or_default();
        assert!(back.handed_back() && note.contains(says) && note.contains("Found while starting."), "{reason}: {note}");
        assert!(back.branch.is_some(), "{reason}: its work kept");
        let woken = engine.prompt_with("Settle it now").await;
        assert!(woken.contains("frontend handed T-2 back"), "{reason}: {woken}");
        assert_eq!(counted(&home, turn_ending::HANDED_BACK), 1, "{reason}");
        handle.shutdown().await;
    }
}
