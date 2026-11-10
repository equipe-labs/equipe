//! `FAKE_HELPERS=1`: backend on Codex over Nebius Token Factory (no
//! subagents of its own), with equipe's helpers at work for it — shown as
//! its subagents, as the daemon shows them. Two read and search, a step now
//! and then; the third has been quiet for minutes and looks stuck, with its
//! card to stop it. Stopping one hands back what it had, as a helper does.
//! `FAKE_HELPERS=codex`: the same quiet one is a subagent of Codex's own on
//! OpenAI, backend on GPT.

use std::time::Duration;

use equipe_core::protocol::{Engine, Entry, Event, SessionId, SessionInfo, SubagentInfo, ToolOutcome};
use tokio::sync::broadcast;

/// The member whose helpers these are.
pub const MEMBER: SessionId = 3;

pub fn mode() -> Option<String> {
    std::env::var("FAKE_HELPERS").ok().filter(|v| !v.is_empty() && v != "0")
}

fn tool(name: &str, target: &str, output: &str) -> Entry {
    Entry::Tool {
        tool: name.into(),
        target: Some(target.into()),
        outcome: Some(Box::new(ToolOutcome { ok: true, exit: None, ms: Some(180), output: output.into(), skipped: 0, diff: None })),
        label: None,
        background: false,
    }
}

/// Backend as a snapshot shows it: on its engine, its helpers at work.
pub fn dress(backend: &mut SessionInfo, codex_own: bool) {
    backend.engine = Engine::Codex;
    backend.model = Some(if codex_own { "gpt-6.1-sol".into() } else { "nvidia/nemotron-3-super-120b-a12b".into() });
    let brief = |what: &str| format!("{what}. Read only: list each place as path:line, and say what you couldn't check.");
    let quiet = SubagentInfo {
        id: if codex_own { "thr-019a".into() } else { "h3".into() },
        what: "Check the pagination tests against the spec".into(),
        step: Some("Bash cargo test -p api pagination -- --nocapture".into()),
        secs: 9 * 60 + 12,
        brief: brief("Check the pagination tests against docs/api/pagination.md"),
        entries: vec![tool("Read", "docs/api/pagination.md", "## Cursors\nA cursor is opaque base64url…")],
        stuck_secs: Some(6 * 60 + 5),
    };
    backend.subagents = if codex_own {
        vec![quiet]
    } else {
        vec![
            SubagentInfo {
                id: "h1".into(),
                what: "Find every caller of encode_cursor".into(),
                step: Some("Grep encode_cursor".into()),
                secs: 74,
                brief: brief("Find every caller of encode_cursor in api/ and what each passes it"),
                entries: vec![tool("Grep", "encode_cursor", "api/cursor.rs:18\napi/orders.rs:142\napi/refunds.rs:77")],
                stuck_secs: None,
            },
            SubagentInfo {
                id: "h2".into(),
                what: "Read the order list's query plan".into(),
                step: Some("Read api/orders.rs".into()),
                secs: 41,
                brief: brief("Read how the order list is queried and where it sorts"),
                entries: Vec::new(),
                stuck_secs: None,
            },
            quiet,
        ]
    };
    backend.recent.push(Entry::Reply { text: "I'll hand the reading to helpers and keep the change myself.".into() });
}

/// The ones at work take a step now and then.
pub async fn play(events: broadcast::Sender<Event>) {
    let steps = [("h1", "Read api/orders.rs"), ("h2", "Grep ORDER BY"), ("h1", "Read api/refunds.rs"), ("h2", "Read api/orders.rs")];
    for (n, (id, step)) in steps.iter().cycle().enumerate() {
        tokio::time::sleep(Duration::from_millis(1800)).await;
        let (name, target) = step.split_once(' ').unwrap_or((step, ""));
        let _ = events.send(Event::SubagentEntry { session: MEMBER, id: (*id).into(), entry: tool(name, target, "…") });
        let _ = events.send(Event::SubagentStep { session: MEMBER, id: (*id).into(), step: (*step).into() });
        if n > 400 {
            break;
        }
    }
}

/// The person stopped one: it hands back what it had.
pub async fn stopped(session: SessionId, id: String, events: broadcast::Sender<Event>) {
    tokio::time::sleep(Duration::from_millis(700)).await;
    let result = "Helper h3 (Check the pagination tests against the spec) was stopped by the person after 9m 20s. What it had by then:\n\n`docs/api/pagination.md:12` says a cursor is opaque base64url; `api/tests/pagination.rs` hasn't been run yet.\n\nWhat it didn't get to is unchecked: check it yourself, or hand it to a new helper with a narrower brief.";
    // One of Codex's own hands back nothing equipe sees: its member hears it stopped.
    let result = (!id.starts_with("thr")).then(|| result.to_owned());
    let _ = events.send(Event::SubagentEnded { session, id, result, failed: false });
}
