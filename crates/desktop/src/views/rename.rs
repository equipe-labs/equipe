//! Renaming a conversation with the PM (T-643; the contract is
//! design/specs/rename-conversation/spec.md): one small card hanging under
//! the header's title, wherever it starts — the title itself, the first item
//! of its ⋯, or a sidebar row's pencil.
//!
//! Return saves, Esc puts it away, a click outside acts as Return; while an
//! IME composes, Return and Esc are the composition's. A name is never cut:
//! past [`MAX`] characters it says how many to take out and Save waits. A
//! save shows the new name at once, in the header and the sidebar; refused,
//! or with the Mac out of reach, the old name comes back and the card opens
//! again with what was typed and Try again.

use std::time::{Duration, Instant};

use equipe_core::protocol::{CONVERSATION_TITLE_MAX as MAX, Command, SessionId};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::decide::Took;
use crate::Shell;
use crate::compose::EnterGuard;
use crate::icons::{Icon, icon};
use crate::state::Conn;
use crate::theme::{FIELD, R_MD, R_SM, SMALL};
use crate::ui::{self, Kind};

/// From this many characters the field counts them.
const COUNT_FROM: usize = 30;
/// The card's width, room allowing.
const WIDTH: f32 = 360.;
/// Narrower than this, the buttons drop their key hints.
const HINTS_FROM: f32 = 300.;
/// A press outside that closed the card, and the click on the title it ends in: one gesture.
const SAME_PRESS: Duration = Duration::from_millis(400);

/// Why a name can't be kept as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Nothing but spaces.
    Empty,
    /// This many characters past [`MAX`].
    Long(usize),
    /// The Mac was out of reach: nothing was saved.
    Offline,
    /// The daemon said no.
    Failed,
}

impl Problem {
    fn words(self) -> String {
        match self {
            Problem::Empty => t!("talk.rename_empty").into(),
            Problem::Long(over) => t!("talk.rename_long", max = MAX, over = over),
            Problem::Offline => t!("talk.rename_offline").into(),
            Problem::Failed => t!("talk.rename_failed").into(),
        }
    }
}

/// What a typed name comes to as it would be kept (its line breaks, tabs and
/// runs of spaces one space, trimmed — the daemon's own rule), or why not.
pub fn check(raw: &str) -> Result<String, Problem> {
    let title = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = title.chars().count();
    if title.is_empty() {
        Err(Problem::Empty)
    } else if count > MAX {
        Err(Problem::Long(count - MAX))
    } else {
        Ok(title)
    }
}

/// The card, open.
pub struct Rename {
    pub session: SessionId,
    pub input: Entity<InputState>,
    /// What it says under the field, if anything.
    pub problem: Option<Problem>,
    /// A save failed: its button says Try again, and the count gives the message room.
    pub retry: bool,
    guard: EnterGuard,
    _subscriptions: Vec<Subscription>,
}

/// A name sent and shown already, until the daemon says it is kept (or not).
pub struct Pending {
    pub session: SessionId,
    /// The name before, to put back.
    pub was: Option<String>,
    /// What was typed, to open the card with again.
    pub typed: String,
}

impl Shell {
    /// The card, open on conversation `session` (one that is open, begun and
    /// the person's: not a draft, not the PM's own). `typed`: words to start
    /// with instead of its name — a save that failed — left unselected.
    pub fn open_rename(&mut self, session: SessionId, typed: Option<(String, Problem)>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(lane) = self.model.lanes.iter().find(|l| l.session == session && l.kept) else { return };
        if self.model.draft_talk.is_some() {
            return;
        }
        self.board.popover = None;
        let retry = typed.is_some();
        let (words, problem) = match typed {
            Some((words, problem)) => (words, Some(problem)),
            None => (lane.title.clone().unwrap_or_default(), None),
        };
        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |s, cx| {
            s.set_value(words.clone(), window, cx);
            // Opened afresh: the whole name, to type over. Again after a failure: the cursor at its end.
            if retry {
                s.set_selected_range(words.len()..words.len(), cx);
            } else {
                s.select_all(window, cx);
            }
            s.focus(window, cx);
        });
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |shell: &mut Shell, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    // A name being fixed: an emptiness said before is said again only on Return.
                    if let Some(r) = shell.rename.as_mut()
                        && r.problem == Some(Problem::Empty)
                    {
                        r.problem = None;
                    }
                    cx.notify();
                }
            }),
            // The Return that ends a composition is only that.
            cx.observe_in(&input, window, |shell: &mut Shell, input, window, cx| {
                let composing = input.update(cx, |s, cx| gpui_kit::EntityInputHandler::marked_text_range(s, window, cx).is_some());
                if let Some(r) = shell.rename.as_mut() {
                    r.guard.observe(composing, Instant::now());
                }
            }),
        ];
        self.rename = Some(Rename { session, input, problem, retry, guard: EnterGuard::default(), _subscriptions: subscriptions });
        cx.notify();
    }

    /// Return, Save or a click outside: the name kept — shown at once and
    /// sent — or the card stays open saying why not.
    pub fn rename_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.rename.as_mut() else { return };
        if !r.guard.allows_send(Instant::now()) || r.guard.composing() {
            return;
        }
        if self.rename_commit(cx) {
            self.rename_close(window, cx);
        }
    }

    /// The card's name kept: sent, and shown in the header and the sidebar
    /// at once. True when the card is done with (sent, or the name it had);
    /// false when it says why not.
    fn rename_commit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(r) = self.rename.as_mut() else { return true };
        let typed = r.input.read(cx).value().to_string();
        let session = r.session;
        let was = self.model.lanes.iter().find(|l| l.session == session).and_then(|l| l.title.clone());
        let title = match check(&typed) {
            Ok(title) => title,
            Err(problem) => {
                r.problem = Some(problem);
                cx.notify();
                return false;
            }
        };
        if was.as_deref() == Some(title.as_str()) {
            return true;
        }
        if !matches!(self.model.conn, Conn::Connected { .. }) || self.link.is_none() {
            r.problem = Some(Problem::Offline);
            r.retry = true;
            cx.notify();
            return false;
        }
        self.send_command(Command::RenameConversation { squad: self.model.squad.clone(), session, title: title.clone() });
        if let Some(lane) = self.model.lanes.iter_mut().find(|l| l.session == session) {
            lane.title = Some(title);
        }
        self.rename_pending = Some(Pending { session, was, typed });
        true
    }

    /// Esc or Cancel: the card goes, the name as it was, the keys back on the title.
    pub fn rename_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.take().is_some() {
            self.title_focus.focus(window, cx);
            cx.notify();
        }
    }

    /// A press outside the card: as Return. A name that can't be kept keeps it open.
    pub fn rename_outside(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rename_save(window, cx);
        if self.rename.is_none() {
            self.rename_closed_at = Some(Instant::now());
        }
    }

    /// The title clicked (or Return on it): the card, unless this click ends
    /// the press that just closed it.
    pub fn rename_from_title(&mut self, session: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename_closed_at.take().is_some_and(|at| at.elapsed() < SAME_PRESS) || self.rename.is_some() {
            return;
        }
        self.open_rename(session, None, window, cx);
    }

    /// Another conversation (or screen) took the header: a name that can be
    /// kept is, one that can't is let go — the header is someone else's now.
    pub fn rename_left(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self.rename.as_ref() else { return };
        if self.screen == crate::Screen::Chat && self.model.talk == Some(r.session) && self.model.draft_talk.is_none() {
            return;
        }
        self.rename_commit(cx);
        self.rename = None;
        cx.notify();
    }

    /// What the daemon said about a name sent (`kept`), or the line to it
    /// gone while one was on its way: refused, the old name back and the card
    /// open again with what was typed.
    pub fn rename_heard(&mut self, heard: Option<(SessionId, bool)>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.rename_pending.as_ref() else { return };
        let failed = match heard {
            Some((session, kept)) if session == pending.session => (!kept).then_some(Problem::Failed),
            _ if !matches!(self.model.conn, Conn::Connected { .. }) => Some(Problem::Offline),
            _ => return,
        };
        let Some(pending) = self.rename_pending.take() else { return };
        let Some(problem) = failed else {
            // Kept: said for a moment where the screen says such things (and to a screen reader).
            if let Some(title) = self.model.lanes.iter().find(|l| l.session == pending.session).and_then(|l| l.title.clone()) {
                self.composer.flash = Some(t!("talk.renamed", title = title));
                cx.notify();
            }
            return;
        };
        if let Some(lane) = self.model.lanes.iter_mut().find(|l| l.session == pending.session) {
            lane.title = pending.was;
        }
        if self.screen == crate::Screen::Chat && self.model.talk == Some(pending.session) {
            self.open_rename(pending.session, Some((pending.typed, problem)), window, cx);
        }
        cx.notify();
    }

    /// A key while the card is open, from the window's root: Return saves and
    /// Esc closes (an IME's composition first); the rest is the field's.
    pub fn on_rename_key(&mut self, key: &str, m: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> Took {
        let Some(r) = self.rename.as_ref() else { return Took::Pass };
        let input = r.input.clone();
        if !input.read(cx).focus_handle(cx).is_focused(window) {
            return Took::Pass;
        }
        let composing = input.update(cx, |s, cx| gpui_kit::EntityInputHandler::marked_text_range(s, window, cx).is_some());
        match key {
            _ if composing => Took::Typing,
            "escape" => {
                self.rename_close(window, cx);
                Took::Taken
            }
            "enter" if !m.shift && !m.alt && !m.control => {
                self.rename_save(window, cx);
                Took::Taken
            }
            _ => Took::Typing,
        }
    }
}

/// The card, under the header's title (its parent places it). `room`: the
/// header's width, to keep the card inside it.
pub fn card(shell: &Shell, room: f32, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let r = shell.rename.as_ref()?;
    if shell.model.talk != Some(r.session) {
        return None;
    }
    let p = shell.palette;
    let typed = r.input.read(cx).value().to_string();
    let count = typed.split_whitespace().collect::<Vec<_>>().join(" ").chars().count();
    let over = count > MAX;
    // Too long is said as it is typed; empty only on Return (the field may be on its way to a new name).
    let problem = r.problem.or(over.then(|| Problem::Long(count - MAX)));
    let width = if room > 0. { (room - 16.).clamp(240., WIDTH) } else { WIDTH };
    let hints = width >= HINTS_FROM;
    // The field has the keys while the card is open: its edge is the focus colour, or red for a name that can't be kept.
    let edge = if problem.is_some_and(|p| p != Problem::Offline && p != Problem::Failed) { p.danger() } else { p.accent };
    let saves = !over && problem != Some(Problem::Empty);
    // The count in the field from COUNT_FROM up; past MAX on the message's line, in red; never after a failure.
    let inside = (!r.retry && !over && count >= COUNT_FROM).then(|| div().flex_none().text_size(px(SMALL - 1.)).text_color(p.muted).child(t!("talk.rename_count", n = count, max = MAX)));
    let message = problem.map(|problem| {
        h_flex()
            .id("rename-message")
            .gap(px(6.))
            .items_start()
            .role(Role::Alert)
            .aria_label(problem.words())
            .text_size(px(SMALL - 1.))
            .text_color(p.danger())
            .child(div().flex_none().pt(px(2.)).child(icon(Icon::Alert, 13., p.danger())))
            .child(div().flex_1().min_w_0().child(problem.words()))
            .when(over && !r.retry, |d| d.child(div().flex_none().child(t!("talk.rename_count", n = count, max = MAX))))
    });
    let save_label = if r.retry { t!("talk.rename_retry") } else { t!("board.save") };
    let save = ui::button(p, "rename-save", Kind::Primary, save_label)
        .when(hints, |b| b.key("↩"))
        .disabled(!saves)
        .on_click(cx.listener(|this, _, window, cx| this.rename_save(window, cx)));
    let cancel = ui::button(p, "rename-cancel", Kind::Secondary, t!("peek.cancel"))
        .when(hints, |b| b.key("Esc"))
        .on_click(cx.listener(|this, _, window, cx| this.rename_close(window, cx)));
    let card = v_flex()
        .id("rename-card")
        .w(px(width))
        .p_3()
        .gap_2()
        .rounded(px(R_MD))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .shadow_lg()
        .occlude()
        .role(Role::Dialog)
        .aria_label(t!("talk.rename_dialog"))
        .on_mouse_down_out(cx.listener(|this, _, window, cx| this.rename_outside(window, cx)))
        .child(div().text_size(px(SMALL - 1.)).text_color(p.muted).child(t!("talk.rename_label")))
        .child(
            h_flex()
                .id("rename-field")
                .h(px(FIELD))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(R_SM))
                .bg(p.code_bg)
                .border_1()
                .border_color(edge)
                .aria_label(t!("talk.rename_label"))
                .text_size(px(SMALL))
                .child(div().flex_1().min_w_0().child(Input::new(&r.input).appearance(false).bordered(false)))
                .children(inside),
        )
        .children(message)
        // Primary first, laid out right to left: side by side Save is on the right; stacked, on top.
        .child(h_flex().flex_row_reverse().flex_wrap().gap_2().child(save).child(cancel));
    Some(deferred(anchored().snap_to_window_with_margin(px(8.)).child(card)).with_priority(2).into_any_element())
}

#[cfg(test)]
mod tests {
    use super::{MAX, Problem, check};

    #[test]
    fn a_name_is_kept_as_one_line_and_never_cut() {
        assert_eq!(check("  Checkout\n\tpage   copy "), Ok("Checkout page copy".into()));
        assert_eq!(check("   \n "), Err(Problem::Empty));
        assert_eq!(check(&"a".repeat(MAX)), Ok("a".repeat(MAX)));
        assert_eq!(check(&"a".repeat(MAX + 8)), Err(Problem::Long(8)), "eight to take out, none taken");
        // Korean is counted a syllable at a time, as the daemon counts.
        assert_eq!(check(&"결".repeat(MAX)).map(|t| t.chars().count()), Ok(MAX));
        assert_eq!(check(&"결".repeat(MAX + 1)), Err(Problem::Long(1)));
    }

    #[test]
    fn the_desktop_and_the_daemon_agree_on_what_a_name_comes_to() {
        for raw in ["  a  b ", "", " ", &"x".repeat(MAX + 1), "회의\n요약"] {
            assert_eq!(check(raw).ok(), equipe_core::protocol::conversation_title(raw), "{raw:?}");
        }
    }
}
