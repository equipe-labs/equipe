//! `FAKE_VISUALS=1`: the PM's last reply holds a Mermaid flow, a table and
//! two HTML cards, and the daemon has drawn the cards as it does (Chrome, in
//! `visuals/`, with their pages) — for looking at how the conversation shows
//! them. `late`: the pictures come 6 s after a screen attaches
//! (`visuals_rendered`, live); `none`: they never come, so the cards wait and
//! then show their source. Needs Google Chrome (or `EQUIPE_CHROME`).

use std::time::Duration;

use equipe_core::protocol::{Event, Rendered};
use tokio::sync::broadcast;

fn mode() -> Option<String> {
    std::env::var("FAKE_VISUALS").ok().filter(|v| !v.is_empty())
}

pub fn on() -> bool {
    mode().is_some()
}

const BOARD: &str = include_str!("../../../core/src/visuals/card/examples/board.html");
const COMPARE: &str = include_str!("../../../core/src/visuals/card/examples/compare.html");

pub const ASKED: &str = "Where are we? Show me in a picture";

pub fn reply() -> String {
    format!(
        "Here is where things stand, as a picture.\n\n```mermaid\nflowchart LR\n  A(\"✓ T-335 Contract<br/>pm\"):::ok --> B(\"● T-338 Desktop pictures<br/>frontend\"):::work\n  B --> C(\"◆ Design QA<br/>designer\"):::wait\n  B --> D(\"○ T-339 Phone pictures<br/>mobile\"):::next\n```\n\n| Item | Status | Who |\n|---|---|---|\n| Diagram theme | In progress | frontend |\n| HTML cards | Waiting | frontend |\n| Daemon drawing | Done | backend |\n\nThe board as it is.\n\n```html card\n{}\n```\n\nThe two options side by side.\n\n```html card\n{}\n```\n\nI'll tell you when it's done.",
        BOARD.trim(),
        COMPARE.trim()
    )
}

/// The cards of the reply, drawn the way the daemon draws them.
fn drawn() -> Vec<Rendered> {
    // SAFETY: set before any thread reads it, at attach.
    unsafe { std::env::set_var("EQUIPE_HTML_CARDS", "1") };
    equipe_core::visuals::card::draw(&equipe_providers::credentials::equipe_home(), "demo", &reply())
}

/// What a screen attaching finds in the PM's session.
pub fn visuals() -> Vec<Rendered> {
    match mode().as_deref() {
        Some("late" | "none") => Vec::new(),
        _ => drawn(),
    }
}

pub fn on_hello(events: &broadcast::Sender<Event>) {
    if mode().as_deref() != Some("late") {
        return;
    }
    let events = events.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(6)).await;
        let visuals = tokio::task::spawn_blocking(drawn).await.unwrap_or_default();
        let _ = events.send(Event::VisualsRendered { session: 1, visuals });
    });
}

/// `read_image` for a card's picture or its kept page (the browser panel
/// opens that as a `data:` URL): the squad's `visuals/` only, answered as the
/// daemon answers it.
pub fn read(ask: &equipe_core::protocol::rpc::ReadImage) -> Result<equipe_core::protocol::rpc::ImageRead, String> {
    let home = equipe_providers::credentials::equipe_home();
    let root = equipe_core::visuals::diagram::dir(&home, "demo");
    let path = std::path::PathBuf::from(&ask.path);
    if path.parent() != Some(root.as_path()) {
        return Err(format!("{} isn't a picture demo shows", ask.path));
    }
    equipe_core::visuals::serve(&home, ask, None)
}
