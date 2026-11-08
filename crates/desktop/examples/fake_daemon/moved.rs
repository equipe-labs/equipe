//! A member that moved to the other engine in the middle of a ticket, as the
//! daemon leaves it (`runtime/engines.rs`): its old session closed, a new one
//! under the same name carrying on from equipe's hand-over packet.
//!
//! `FAKE_MOVED=1`: backend moved from Claude to Codex 25 minutes ago, mid
//! T-70, Claude's plan out — its Claude conversation (session 30) kept, the
//! Codex one (3, as the squad has it) carrying on. `FAKE_MOVED=twice`: before
//! that, the person had moved it from Claude to Codex between tickets — and it
//! came back: Claude (29), Codex (30), Claude (3). `FAKE_MOVED=live`: backend
//! is on Claude when a screen attaches and moves to Codex `FAKE_MOVED_AFTER` ms
//! later (default 4000), mid T-70, as the daemon makes it: closed, started
//! again under its name, the move told.
//!
//! `hand_over` answers with the packet each new session opened with and the
//! conversation before it.

use std::time::Duration;

use equipe_core::protocol::engines::{HandOver, Why};
use equipe_core::protocol::{Command, Engine, Entry, Event, Member, SessionId, SessionInfo, ToolOutcome};
use tokio::sync::broadcast;

const SQUAD: &str = "demo";
/// Where the live move goes.
const LIVE: SessionId = 31;

pub fn mode() -> Option<String> {
    std::env::var("FAKE_MOVED").ok().filter(|v| !v.is_empty() && v != "0")
}

fn twice() -> bool {
    mode().as_deref() == Some("twice")
}

fn live() -> bool {
    mode().as_deref() == Some("live")
}

fn now() -> i64 {
    super::unix() as i64
}

fn ran(command: &str, ok: bool, output: &str) -> Entry {
    Entry::Tool {
        tool: "Bash".into(),
        target: Some(command.into()),
        outcome: Some(Box::new(ToolOutcome { ok, exit: (!ok).then_some(101), ms: Some(8_400), output: output.into(), skipped: 0, diff: None })),
        label: None,
        background: false,
    }
}

fn said(text: &str) -> Entry {
    Entry::User { text: text.into(), kept: Vec::new() }
}

fn reply(text: &str) -> Entry {
    Entry::Reply { text: text.into() }
}

fn chapter() -> Entry {
    Entry::Note { undo: None, text: "── T-70 · Orders API pagination — on branch equipe/t-70".into() }
}

/// Backend's conversation on Claude, on T-70, up to the move.
fn on_claude() -> Vec<Entry> {
    vec![
        chapter(),
        said("Please start T-70."),
        reply("Sure. I'll read how `GET /orders` builds its query first, then add cursor pagination behind the same route."),
        Entry::Tool { tool: "Read".into(), target: Some(format!("{}/src/orders.rs", super::repo("api").display())), outcome: None, label: None, background: false },
        said("Keep the old `offset` parameter working — the mobile app still sends it."),
        reply("Understood: `offset` stays, and a cursor wins when both come. I'll write the test for that first."),
        ran("cargo test -p api orders::", false, "test orders::offset_still_pages ... FAILED\n\nthread 'orders::offset_still_pages' panicked at src/orders.rs:88:9:\nassertion `left == right` failed\n  left: 20\n right: 40"),
        reply("The offset test fails as expected: the cursor path ignores `offset`. Next: read `offset` when no cursor is given."),
    ]
}

/// Backend's talk on Claude between tickets, before the first move (`twice`).
fn before_codex() -> Vec<Entry> {
    vec![
        said("Look over the orders schema before you start T-70."),
        reply("Done: `orders.created_at` has an index, `id` is a ULID — both sort, so a cursor can be `(created_at, id)`. Nothing to migrate."),
        said("Good. Do the next ones on Codex."),
        reply("Fine by me — I'll carry on there."),
    ]
}

/// Backend's conversation on Codex after the last move: what it read in the packet, and on.
fn after() -> Vec<Entry> {
    vec![
        chapter(),
        said("Please start T-70."),
        reply("I've read the hand-over. The cursor pagination is committed on `equipe/t-70` (`3f9c2ab`); what's left is the `offset` fallback the failing test pins down. Checking the worktree first."),
        ran("git status --short && git log --oneline -3", true, "3f9c2ab equipe: handoff wip — T-70 Orders API pagination\n8e41d07 orders: cursor type\nb2a9e10 main: bump"),
        ran("cargo test -p api orders::", true, "test orders::cursor_pages ... ok\ntest orders::offset_still_pages ... ok\n\ntest result: ok. 2 passed"),
        reply("`offset` pages again when no cursor is given; both tests pass. Next: the `next_cursor` field in the response, then I'll hand T-70 in."),
    ]
}

/// The packet the session after `h` opened with, as the daemon writes it.
fn packet(h: &HandOver) -> String {
    let (from, to) = (name(h.from), name(h.to));
    let because = match h.why {
        Why::Exhausted => format!("{from}'s plan ran out until its reset"),
        _ => "the person moved you".to_owned(),
    };
    let mut out = format!(
        "[equipe] You are backend, carrying on on {to} what you were doing on {from} — {because}. This conversation is new, but nothing was lost: this is the hand-over packet. Read all of it before you do anything, and the repository's AGENTS.md.\n\n"
    );
    if h.ticket.is_some() {
        out.push_str(
            "## The ticket: T-70 Orders API pagination\nPage `GET /orders` with a cursor; keep `offset` for the mobile app.\n\nAcceptance criteria:\n- `?cursor=` pages by `(created_at, id)`\n- `?offset=` still works when no cursor is given\n- tests for both\n\n\
## The work so far\nBranch equipe/t-70, in the worktree ~/.equipe/worktrees/api-t70 — the same one, yours to carry on in.\nEverything it held was committed as it stood (\"equipe: handoff wip\", 3f9c2ab).\nChanged since the branch began (`git diff --stat b2a9e10..equipe/t-70`):\n```\n src/orders.rs       | 64 +++++++++++++++++++++++++++++++++------\n tests/orders.rs     | 41 +++++++++++++++++++++++++\n 2 files changed, 96 insertions(+), 9 deletions(-)\n```\n\n",
        );
    }
    out.push_str("## Where things stand\n- Done: what the commit and diff above hold — check them against the ticket.\n- Next: where your last reply left it — \"The offset test fails as expected: the cursor path ignores `offset`. Next: read `offset` when no cursor is given.\"\n- Open decisions: none waiting on the person.\n\n");
    out.push_str("## What the person said on this ticket, verbatim\n> Keep the old `offset` parameter working — the mobile app still sends it.\n\n");
    out.push_str("## The last commands you ran, and what they said\n- `Bash cargo test -p api orders::` → failed, exit 101\n```\ntest orders::offset_still_pages ... FAILED\nassertion `left == right` failed\n  left: 20\n right: 40\n```\n\n");
    out.push_str("## The end of the conversation, verbatim\nPerson: Please start T-70.\n\nYou: Sure. I'll read how `GET /orders` builds its query first, then add cursor pagination behind the same route.\n\nPerson: Keep the old `offset` parameter working — the mobile app still sends it.\n\nYou: Understood: `offset` stays, and a cursor wins when both come. I'll write the test for that first.\n\n");
    out.push_str("## Before you change anything\n1. Read this packet through.\n2. Check the worktree yourself: `git status`, `git log --oneline -5` and the branch's diff; run the ticket's tests and see what passes.\n3. Say in one line what you will do next — then do it.\nCarry on from that state: don't redo what is done, don't undo it, don't start a new branch or worktree.");
    out
}

fn name(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "Claude",
        Engine::Codex => "Codex",
    }
}

/// The moves on record, oldest first.
pub fn hand_overs() -> Vec<HandOver> {
    let now = now();
    let mid = |from, to, engines: (Engine, Engine)| HandOver { member: "backend".into(), from_session: from, to_session: to, from: engines.0, to: engines.1, why: Why::Exhausted, at: now - 25 * 60, ticket: Some(70) };
    match mode().as_deref() {
        None | Some("live") => Vec::new(),
        Some("twice") => vec![
            HandOver { member: "backend".into(), from_session: 29, to_session: 30, from: Engine::Claude, to: Engine::Codex, why: Why::Person, at: now - 3 * 3600, ticket: None },
            mid(30, 3, (Engine::Codex, Engine::Claude)),
        ],
        Some(_) => vec![mid(30, 3, (Engine::Claude, Engine::Codex))],
    }
}

/// Backend as the snapshot has it: the session that carries on, on the engine it moved to.
pub fn dress(sessions: &mut [SessionInfo]) {
    if mode().is_none() {
        return;
    }
    let Some(backend) = sessions.iter_mut().find(|s| s.member.as_ref().is_some_and(|m| m.name == "backend")) else { return };
    if live() {
        backend.recent = on_claude();
        return;
    }
    let to = hand_overs().last().map_or(Engine::Codex, |h| h.to);
    backend.engine = to;
    backend.model = Some(if to == Engine::Codex { "gpt-5.5-codex".into() } else { "claude-opus-5-5".into() });
    backend.state = equipe_core::protocol::SessionState::Ready;
    backend.recent = after();
}

/// The conversation before session `to`, by the move into it.
fn before(to: SessionId) -> Vec<Entry> {
    match to {
        30 if twice() => before_codex(),
        30 | 3 | LIVE => on_claude(),
        _ => Vec::new(),
    }
}

/// `hand_over`: the packet and the conversation before.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    let Command::HandOver { session, .. } = command else { return false };
    let all = if live() { live_moves() } else { hand_overs() };
    let Some(h) = all.iter().find(|h| h.to_session == *session) else { return true };
    let lines = before(*session);
    let latest = u64::try_from(h.at).unwrap_or(0);
    let lines_at = super::said_at(&lines, latest);
    let _ = events.send(Event::HandOver { squad: SQUAD.into(), session: *session, packet: packet(h), lines, lines_at });
    true
}

fn live_moves() -> Vec<HandOver> {
    static AT: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    let at = *AT.get_or_init(now);
    vec![HandOver { member: "backend".into(), from_session: 3, to_session: LIVE, from: Engine::Claude, to: Engine::Codex, why: Why::Exhausted, at, ticket: Some(70) }]
}

/// `live`: the move, as the daemon makes it, a while after a screen attaches.
pub async fn play(events: broadcast::Sender<Event>) {
    let after = std::env::var("FAKE_MOVED_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(4000);
    tokio::time::sleep(Duration::from_millis(after)).await;
    let h = live_moves().remove(0);
    let _ = events.send(Event::SessionClosed { session: 3 });
    let member = Some(Member { role: "backend".into(), name: "backend".into() });
    let _ = events.send(Event::SessionStarted { session: LIVE, project: "/tmp/fake/site".into(), engine: Engine::Codex, model: Some("gpt-5.5-codex".into()), workspace: None, member, squad: Some(SQUAD.into()) });
    let _ = events.send(Event::MemberMoved { squad: SQUAD.into(), hand_over: h });
    let _ = events.send(Event::Notice { undo: None, session: LIVE, text: "Moved from Claude to Codex.".into() });
}

/// Whether the live move is still to come.
pub fn plays() -> bool {
    live()
}
