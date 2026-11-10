//! Under a command step still running: what really runs for it on the
//! member's machine, as the daemon looked a moment ago
//! ([`Event::CommandWatch`](equipe_core::protocol::Event::CommandWatch)) —
//! one quiet line (`sqlite3 · 0% CPU · 27m`, or that nothing runs) — and,
//! once it looks stuck, an amber card that says why in a sentence and offers
//! to stop just that command (the member gets an error and carries on, and
//! reads at once what the person sent meanwhile) or the turn.
//!
//! The words are [`crate::activity::watched`]'s; this only draws them.

use equipe_core::protocol::Command;
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::Shell;
use crate::activity::Watched;
use crate::icons::{Icon, icon};
use crate::render::Tones;
use crate::theme::{BODY, BODY_LINE, META, Palette, R_LG, SMALL, SMALL_LINE};
use crate::ui::{self, Kind};

/// A step shows what runs for it once it has run this long: a quick one
/// never does.
pub const AFTER: std::time::Duration = std::time::Duration::from_secs(5);

/// What a running step shows under it, as of the latest look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub watched: Watched,
    /// The person asked to stop the command; it hasn't answered yet.
    pub stopping: bool,
}

/// The line or the card, drawn under the step `key`.
pub fn under_step(shown: &Shown, p: Palette, tones: Tones, key: u64, handle: WeakEntity<Shell>) -> AnyElement {
    match &shown.watched {
        Watched::Line(line) => div()
            .pl(px(20.))
            .pb(px(2.))
            .text_size(px(META))
            .text_color(p.muted)
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(line.clone())
            .into_any_element(),
        Watched::Stuck { says, hint, command } => {
            let stop_command = command.then(|| {
                let handle = handle.clone();
                let label = if shown.stopping { t!("chat.stuck_stopping") } else { t!("chat.stuck_stop_command") };
                ui::button(p, ("watch-stop-command", key), Kind::Primary, label).disabled(shown.stopping).on_click(move |_, _, cx| {
                    handle.update(cx, |shell, cx| shell.stop_command(cx)).ok();
                })
            });
            let stop_turn = ui::button(p, ("watch-stop-turn", key), if *command { Kind::Ghost } else { Kind::Primary }, t!("chat.stuck_stop_turn")).on_click(move |_, _, cx| {
                handle.update(cx, |shell, cx| shell.interrupt(cx)).ok();
            });
            v_flex()
                .w_full()
                .mt_1()
                .mb_1()
                .p(px(14.))
                .gap_2()
                .rounded(px(R_LG))
                .border_1()
                .border_color(Rgba { a: 0.45, ..tones.warning })
                .bg(Rgba { a: 0.04, ..tones.warning })
                .child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(div().h(px(BODY_LINE)).flex().items_center().flex_none().child(icon(Icon::Warning, 15., tones.warning)))
                        .child(div().min_w_0().text_size(px(BODY)).line_height(px(BODY_LINE)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("chat.stuck_title"))),
                )
                .child(
                    v_flex()
                        .pl(px(22.))
                        .gap_2()
                        .child(div().text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.text2).child(says.clone()))
                        .child(div().text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.muted).child(hint.clone()))
                        .child(h_flex().gap_2().flex_wrap().items_center().children(stop_command).child(stop_turn)),
                )
                .into_any_element()
        }
    }
}

impl Shell {
    /// Stop the command the member in view runs now: only it, not the turn.
    pub fn stop_command(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.model.viewed() else { return };
        let Some(convo) = self.model.convo_mut().filter(|c| !c.stopping_command) else { return };
        convo.stopping_command = true;
        self.send_command(Command::StopCommand { session });
        cx.notify();
    }
}
