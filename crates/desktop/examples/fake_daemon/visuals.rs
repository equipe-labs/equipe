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

pub const ASKED: &str = "지금 어디까지 왔어? 그림으로 보여 줘";

pub fn reply() -> String {
    format!(
        "지금 상황을 그림으로 정리했어요.\n\n```mermaid\nflowchart LR\n  A(\"✓ T-335 계약 문서<br/>pm\"):::ok --> B(\"● T-338 데스크톱 표시<br/>frontend\"):::work\n  B --> C(\"◆ 디자인 QA<br/>designer\"):::wait\n  B --> D(\"○ T-339 폰 표시<br/>mobile\"):::next\n```\n\n| 항목 | 상태 | 담당 |\n|---|---|---|\n| 도표 테마 | 진행 중 | frontend |\n| HTML 카드 | 대기 | frontend |\n| 데몬 렌더 | 완료 | backend |\n\n보드 현황이에요.\n\n```html card\n{}\n```\n\n두 안을 비교하면 이래요.\n\n```html card\n{}\n```\n\n끝나면 다시 알려 드릴게요.",
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

/// `read_image` for a card's picture: the squad's `visuals/` only.
pub fn read(ask: &equipe_core::protocol::rpc::ReadImage) -> Result<equipe_core::protocol::rpc::ImageRead, String> {
    let root = equipe_core::visuals::diagram::dir(&equipe_providers::credentials::equipe_home(), "demo");
    let path = std::path::PathBuf::from(&ask.path);
    if path.parent() != Some(root.as_path()) {
        return Err(format!("{} isn't a picture demo shows", ask.path));
    }
    equipe_core::visuals::read(&path, &[root], ask.max)
}
