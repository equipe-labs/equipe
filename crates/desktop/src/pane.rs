//! The window's big parts, each a cached view drawn from the shell's state:
//! the sidebar, the screen beside it, and the conversation in that.
//!
//! GPUI draws a view again with every view around it, up to the window; any
//! frame drew the shell, so all of it — a spinner's turn in the sidebar laid
//! out a day's conversation again. Now a frame draws the views that asked for
//! it and those around them, and lays the other parts where they were: the
//! sidebar's spinners draw the sidebar, the composer's cursor the screen but
//! not the conversation in it. A part is drawn again when the shell is
//! notified, or anything it read while drawn is (an input's text, a list's
//! scroll: `Window::notified_entities` in `vendor/gpui-pre`).

use std::cell::Cell;

use gpui_kit::*;

use crate::{Shell, views};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Sidebar,
    /// The screen beside the sidebar: a conversation, the board, a guide.
    Body,
    /// The conversation's list, in the chat screen.
    Conversation,
}

pub struct Pane {
    shell: WeakEntity<Shell>,
    part: Part,
}

pub struct Panes {
    pub sidebar: Entity<Pane>,
    pub body: Entity<Pane>,
    pub conversation: Entity<Pane>,
    /// The pane being drawn now: where what turns in it asks the clock to
    /// draw again (`Shell::turning`). `None`: the shell itself.
    pub drawing: Cell<Option<EntityId>>,
}

impl Panes {
    pub fn new(cx: &mut Context<Shell>) -> Self {
        let shell = cx.weak_entity();
        let mut pane = |part| {
            let shell = shell.clone();
            cx.new(|_| Pane { shell, part })
        };
        Self { sidebar: pane(Part::Sidebar), body: pane(Part::Body), conversation: pane(Part::Conversation), drawing: Cell::new(None) }
    }
}

impl Render for Pane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (me, part) = (cx.entity_id(), self.part);
        self.shell
            .update(cx, |shell, cx| {
                if shell.log_renders {
                    eprintln!("render {part:?} {me:?}");
                }
                let was = shell.panes.drawing.replace(Some(me));
                let element = match part {
                    Part::Sidebar => views::sidebar::render(shell, cx).into_any_element(),
                    Part::Body => div().size_full().flex().flex_col().child(shell.body(cx)).into_any_element(),
                    Part::Conversation => views::chat::conversation_pane(shell, cx),
                };
                shell.panes.drawing.set(was);
                element
            })
            .unwrap_or_else(|_| Empty.into_any_element())
    }
}
