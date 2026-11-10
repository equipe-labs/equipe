//! Side chats: a conversation on the side about a passage of a member's
//! conversation, which knows that conversation so far but never adds to it
//! ([`crate::Command::StartSideChat`], [`crate::Event::SideChat`]).

/// A side chat, as the screen that opened it numbers it.
pub type SideChatId = u64;

/// A side chat nobody wrote in for this long is let go.
pub const IDLE_SECS: u64 = 30 * 60;

/// A fresh side chat id: random, below 2^53, as a session's.
pub fn new_id() -> SideChatId {
    crate::new_session_id()
}

#[cfg(test)]
mod tests {
    use crate::{Command, Event};

    #[test]
    fn a_side_chats_words_travel_inside_it_and_belong_to_no_conversation() {
        let inner = Event::TextDelta { session: 9, text: "hi".into() };
        let event = Event::SideChat { squad: Some("demo".into()), session: 1, side: 9, event: Box::new(inner) };
        let back: Event = serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        assert_eq!(back, event);
        assert_eq!(event.session(), None, "never folded into the conversation it is about");
        assert_eq!(event.squad(), Some("demo"), "only the squad's screens get it");
        let start = Command::StartSideChat { session: 1, side: 9, passage: "a".into() };
        let back: Command = serde_json::from_str(&serde_json::to_string(&start).unwrap()).unwrap();
        assert_eq!(back, start);
    }
}
