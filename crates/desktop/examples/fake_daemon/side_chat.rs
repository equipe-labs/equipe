//! Side chats on the stand-in, as the daemon runs them: a side chat opens
//! searching the web, and each message gets a search with the pages it found,
//! a page read, then an answer streamed — all inside [`Event::SideChat`],
//! never in the member's conversation. `FAKE_SIDE_SEARCH=off`: the member's
//! model is on another provider through Codex, with no web search of its
//! own; it reads a file instead.

use std::time::Duration;

use equipe_core::protocol::{Event, SessionId, SideChatId, ToolOutcome, TurnOutcome, Usage};
use tokio::sync::broadcast;

pub fn searches() -> bool {
    std::env::var("FAKE_SIDE_SEARCH").map_or(true, |v| v != "off")
}

pub fn opened(session: SessionId, side: SideChatId, events: &broadcast::Sender<Event>) {
    let _ = events.send(Event::SideChatOpened { squad: Some("demo".into()), session, side, search: searches() });
}

pub fn closed(side: SideChatId, events: &broadcast::Sender<Event>) {
    let _ = events.send(Event::SideChatClosed { squad: Some("demo".into()), side });
}

fn done(output: &str) -> ToolOutcome {
    ToolOutcome { ok: true, exit: None, ms: Some(900), output: output.into(), skipped: 0, diff: None }
}

/// One turn of side chat `side`, about `session`'s conversation.
pub async fn turn(session: SessionId, side: SideChatId, text: String, events: broadcast::Sender<Event>) {
    let say = |event: Event| {
        let _ = events.send(Event::SideChat { squad: Some("demo".into()), session, side, event: Box::new(event) });
    };
    let pause = || tokio::time::sleep(Duration::from_millis(350));
    say(Event::UserMessage { session: side, text, kept: Vec::new() });
    pause().await;
    let answer = if searches() {
        let query = "TextSelection ends gpui floating toolbar".to_owned();
        say(Event::ToolUse { session: side, tool: "WebSearch".into(), target: Some(query.clone()), label: None, background: false });
        pause().await;
        let found = "- [Text selection in GPUI](https://www.gpui.rs/docs/text-selection)\n- [Floating toolbars over selected text](https://developer.apple.com/design/human-interface-guidelines/menus)\n- [Selection API — MDN](https://developer.mozilla.org/en-US/docs/Web/API/Selection)";
        say(Event::ToolResult { session: side, tool: "WebSearch".into(), target: Some(query), outcome: done(found) });
        let page = "https://www.gpui.rs/docs/text-selection".to_owned();
        say(Event::ToolUse { session: side, tool: "WebFetch".into(), target: Some(page.clone()), label: None, background: false });
        pause().await;
        say(Event::ToolResult { session: side, tool: "WebFetch".into(), target: Some(page), outcome: done("") });
        "It goes first because the bar floats over the selection's **start**: until the header knows where its words are drawn, the bar has nowhere to sit.\n\nGPUI's docs say the same — a selection's ends are known only once it is laid out, which is why `TextSelection::ends` is read a frame later."
    } else {
        let file = "/tmp/fake/site/src/header.tsx".to_owned();
        say(Event::ToolUse { session: side, tool: "Read".into(), target: Some(file.clone()), label: None, background: false });
        pause().await;
        say(Event::ToolResult { session: side, tool: "Read".into(), target: Some(file), outcome: done("") });
        "It goes first because the bar floats over the selection's **start**: `header.tsx` lays the words out, and until it has, the bar has nowhere to sit."
    };
    for chunk in answer.split_inclusive(' ') {
        say(Event::TextDelta { session: side, text: chunk.to_owned() });
        tokio::time::sleep(Duration::from_millis(12)).await;
    }
    let usage = Usage { input_tokens: Some(12), output_tokens: Some(140), cache_tokens: Some(18_400), ..Default::default() };
    say(Event::TurnEnded { session: side, outcome: TurnOutcome::Completed, usage: Some(usage) });
}
