//! A place opened in a window of its own: ⌘-click (or ⇧-click, ⌥⌘-click,
//! as browsers open a link elsewhere) — to see two places side by side, the
//! PM's conversation beside the board, two members at once. The sidebar is
//! already how one window goes from place to place, so there are no tabs of
//! equipe's own; the windows share the Mac's tab group, so "Merge All
//! Windows" (or the Mac's "prefer tabs") makes tabs of them for whoever
//! wants them. The new window is another screen on the same daemon: it
//! connects, then goes to the place. Only the first window tells the
//! person things (notifications) and takes what other apps share, so nothing
//! comes twice.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::*;

use super::sidebar::Place;
use crate::Shell;

/// Set while a window of the app opens after the first: its shell
/// leaves the once-per-app duties to the first.
static OPENING_ANOTHER: AtomicBool = AtomicBool::new(false);

/// Whether the shell being made now is one more window, not the app's first.
pub fn opening_another() -> bool {
    OPENING_ANOTHER.swap(false, Ordering::SeqCst)
}

/// Whether a click's keys ask for a window of its own: ⌘ (as a link opens
/// elsewhere on the Mac), ⇧ (Chrome) or ⌥⌘ (Safari).
pub fn wants_window(m: &Modifiers) -> bool {
    !m.control && !m.function && (m.platform || m.shift)
}

/// What to show there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Goal {
    Place(Place),
    /// A project's slice, its conversation.
    Project(String),
    /// A ticket, on the board.
    Ticket(u32),
}

/// Open `goal` in a window of its own — once the click that asked is done
/// with (a window can't be made while the one clicked is busy with it).
pub fn open(goal: Goal, cx: &mut App) {
    cx.defer(move |cx| {
        OPENING_ANOTHER.store(true, Ordering::SeqCst);
        let (_, shell) = crate::open_app_window(cx);
        OPENING_ANOTHER.store(false, Ordering::SeqCst);
        shell.update(cx, |shell, cx| {
            shell.goal = Some(goal);
            cx.notify();
        });
    });
}

impl Shell {
    /// A click with ⌘ (⇧, ⌥⌘) opens `goal` in a window of its own: true
    /// when it did, and the click is done.
    pub fn open_elsewhere(&mut self, e: &ClickEvent, goal: Goal, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !wants_window(&e.modifiers()) {
            return false;
        }
        open(goal, cx);
        true
    }

    /// Go where this window was opened to go, once the daemon has told what
    /// there is (the member, the ticket, the project).
    pub fn reach_goal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(goal) = self.goal.clone() else { return };
        if !self.model.attached() {
            return;
        }
        self.goal = None;
        match goal {
            Goal::Place(place) => self.go_to(place, window, cx),
            Goal::Project(name) => {
                self.focus_project(Some(name), window, cx);
                self.open_side(crate::state::Side::Pm, window, cx);
            }
            Goal::Ticket(id) => self.open_ticket(id, cx),
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::wants_window;
    use gpui_kit::Modifiers;

    #[test]
    fn a_click_with_command_opens_a_window_as_a_mac_link_opens_elsewhere() {
        let m = |platform, shift, alt| Modifiers { platform, shift, alt, ..Default::default() };
        assert!(wants_window(&m(true, false, false)), "⌘-click");
        assert!(wants_window(&m(false, true, false)), "⇧-click, as Chrome");
        assert!(wants_window(&m(true, false, true)), "⌥⌘-click, as Safari");
        assert!(!wants_window(&m(false, false, false)), "a plain click: here");
        assert!(!wants_window(&Modifiers { control: true, platform: true, ..Default::default() }));
    }
}
