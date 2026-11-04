//! "새 스쿼드…": the squad switcher's way to one more squad. A sheet over the
//! window with the first run's four questions (`views/welcome.rs`'s `fields`,
//! its rules `crate::squad_form`) — the name, the team, the mode, a folder —
//! then Create: the squad is made on the machine the window is on, becomes the
//! current one, and its PM's conversation opens to talk to. Esc or Cancel
//! closes it.
//!
//! This Mac makes it on its own disk, as the first run does. Another machine
//! makes it with `equipe squad new` run there over the window's SSH connection
//! (`sidecar::Job::Squad`, the console's `/squad new`); its mode is the
//! daemon's to set once the window is attached to it (`Command::SetMode`), and
//! its folder is added there afterwards — this Mac's picker has none of that
//! machine's to give. Not reachable, the item says so and does nothing.

use equipe_core::protocol::{Command, Mode};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::welcome::{self, Which};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::squad_form::{self, Route};
use crate::state::Conn;
use crate::theme::{Palette, R_MD, SMALL};
use crate::ui::{self, Kind};
use crate::Shell;

/// The sheet's width.
const WIDTH: f32 = 600.;
/// The questions' height before they scroll: the sheet stays inside the box
/// every sheet has (`connect::modal_w`, 640 tall), its title and buttons in sight.
const BODY_MAX: f32 = 472.;

fn badge(p: Palette) -> AnyElement {
    div().size(px(32.)).flex_none().rounded(px(R_MD)).bg(p.wash).border_1().border_color(p.hair).flex().items_center().justify_center().child(icon(Icon::Users, 17., p.text)).into_any_element()
}

/// The sheet, while it's open.
pub fn sheet(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let w = &shell.new_squad;
    if !w.open {
        return None;
    }
    let p = shell.palette;
    let route = shell.new_squad_route();
    let host = shell.endpoint.label();
    let close = ui::icon_button(p, "new-squad-close", Icon::X, t!("new_squad.close"), Some("Esc")).on_click(cx.listener(|this, _, window, cx| this.close_new_squad(window, cx)));
    let header = h_flex()
        .gap_3()
        .items_start()
        .child(div().mt(px(2.)).child(badge(p)))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_size(px(17.)).line_height(px(24.)).font_weight(FontWeight::SEMIBOLD).child(t!("new_squad.title")))
                .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(match route {
                    Route::There => t!("new_squad.subtitle_there", host = host),
                    _ => t!("new_squad.subtitle_here").to_owned(),
                })),
        )
        .child(close);
    let body = v_flex().id("new-squad-body").max_h(px(BODY_MAX)).overflow_y_scroll().gap_2().children(welcome::fields(shell, Which::New, route, cx));
    let foot = h_flex()
        .gap_2()
        .items_center()
        .justify_end()
        .border_t_1()
        .border_color(p.hair)
        .pt_3()
        .children(w.failed.clone().map(|why| welcome::failed(p, why).flex_1().min_w_0().max_h(px(60.)).overflow_hidden().mr_2()))
        .when(w.busy, |r| r.child(motion::spinner(14., p.text2)))
        .child(ui::button(p, "new-squad-cancel", Kind::Secondary, t!("sheet.cancel")).field().focus(&w.cancel_focus).disabled(w.busy).on_click(cx.listener(|this, _, window, cx| this.close_new_squad(window, cx))))
        .child(welcome::create_button(shell, Which::New, cx));
    Some(super::connect::modal_w(p, "new-squad", WIDTH, v_flex().gap_4().child(header).child(body).child(foot)))
}

/// The switcher's item: what it says under the pointer when it can't be pressed.
pub fn unavailable_tip(route: Route, host: &str) -> Option<String> {
    (!route.available()).then(|| t!("new_squad.unreachable", host = host))
}

impl Shell {
    /// Where the window would make a squad now.
    pub fn new_squad_route(&self) -> Route {
        Route::for_window(self.endpoint.is_remote(), matches!(self.model.conn, Conn::Connected { .. }))
    }

    /// "새 스쿼드…": the sheet opens, answered as nobody has yet, the name to type.
    pub fn open_new_squad(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::tips::hide(cx);
        self.board.popup = None;
        // From the Squad menu, where nothing is greyed: said instead.
        if let Some(why) = unavailable_tip(self.new_squad_route(), &self.endpoint.label()) {
            self.composer.flash = Some(why);
            cx.notify();
            return;
        }
        if self.new_squad.open {
            return;
        }
        self.new_squad.reset(window, cx);
        self.new_squad.open = true;
        self.new_squad.name.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    /// Esc, Cancel, ×: the sheet goes, nothing made. While the squad is being made it stays.
    pub fn close_new_squad(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_squad.busy {
            return;
        }
        self.new_squad.open = false;
        if self.screen != crate::Screen::Settings {
            self.composer.focus(window, cx);
        }
        cx.notify();
    }

    /// Tab and ⇧Tab inside the sheet: round its stops, never out to the window under it.
    pub fn new_squad_tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let stops = self.new_squad.stops(Which::New, self.new_squad_route(), cx);
        let at = stops.iter().position(|f| f.contains_focused(window, cx));
        stops[squad_form::next_stop(at, stops.len(), back)].focus(window, cx);
        cx.notify();
    }

    /// Create: checked, made where the window is, then switched to.
    pub(super) fn create_new_squad(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_squad.busy {
            return;
        }
        let typed = self.new_squad.name.read(cx).value().to_string();
        let name = match squad_form::check_name(&typed, &self.squads, None) {
            Ok(name) => name,
            Err(problem) => {
                self.new_squad.error = Some(welcome::name_problem(problem, &typed));
                self.new_squad.name.update(cx, |s, cx| s.focus(window, cx));
                cx.notify();
                return;
            }
        };
        let route = self.new_squad_route();
        let team = squad_form::team(&self.new_squad.team);
        let (mode, folder) = (self.new_squad.mode, self.new_squad.folder.clone().filter(|_| route.picks_folder()));
        (self.new_squad.error, self.new_squad.failed, self.new_squad.busy) = (None, None, true);
        cx.notify();
        let made = name.clone();
        let task = match route {
            Route::Here => {
                let home = equipe_providers::credentials::equipe_home();
                let user_home = std::env::var_os("HOME").map(std::path::PathBuf::from);
                cx.background_executor().spawn(async move { squad_form::make_here(&home, &made, mode, folder, user_home.as_deref()).map(|folder_error| (folder_error, None)) })
            }
            Route::There => {
                let endpoint = self.endpoint.clone();
                // Blocks on ssh, off the window's thread, as every side command does.
                cx.background_executor().spawn(async move {
                    let job = crate::sidecar::Job::Squad { squad: String::new(), words: squad_form::new_words(&made, None), dry_run: false };
                    crate::sidecar::run(&endpoint, &job).map(|_| (None, Some(mode)))
                })
            }
            Route::Unreachable => {
                self.new_squad.busy = false;
                self.new_squad.failed = unavailable_tip(route, &self.endpoint.label());
                return;
            }
        };
        cx.spawn_in(window, async move |this, cx| {
            let done = task.await;
            this.update_in(cx, |this, window, cx| {
                match done {
                    Ok((folder_error, mode_later)) => this.made_new_squad(&name, team, folder_error, mode_later, window, cx),
                    Err(why) => {
                        this.new_squad.busy = false;
                        this.new_squad.failed = Some(format!("{}\n{why}", t!("welcome.failed_create")));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Made: the sheet goes, the squad is the window's, its team joins and the
    /// PM's conversation is the one open, the box ready to type in.
    fn made_new_squad(&mut self, name: &str, team: Vec<String>, folder_error: Option<String>, mode_later: Option<Mode>, window: &mut Window, cx: &mut Context<Self>) {
        self.new_squad.busy = false;
        self.new_squad.open = false;
        if !self.squads.iter().any(|s| s == name) {
            self.squads.push(name.to_owned());
        }
        self.pending_team = Some(team);
        self.pending_mode = mode_later.map(|m| (name.to_owned(), m));
        self.composer.flash = Some(folder_error.unwrap_or_else(|| t!("new_squad.made", name = name)));
        self.show(crate::Screen::Chat, cx);
        self.select_squad(name, window, cx);
        self.composer.focus(window, cx);
    }

    /// Attached to the squad just made on another machine: how much it does
    /// alone, as the sheet said — there it is the daemon's to set.
    pub fn apply_pending_mode(&mut self) {
        if let Some((squad, mode)) = self.pending_mode.take_if(|(squad, _)| *squad == self.model.squad) {
            self.send_command(Command::SetMode { squad, member: None, mode: Some(mode) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::unavailable_tip;
    use crate::squad_form::Route;

    #[test]
    fn the_item_says_why_only_when_the_far_machine_cant_be_reached() {
        assert_eq!(unavailable_tip(Route::Here, "abel@studio"), None);
        assert_eq!(unavailable_tip(Route::There, "abel@studio"), None);
        let why = unavailable_tip(Route::Unreachable, "abel@studio").expect("a reason");
        assert!(why.contains("abel@studio"), "{why}");
    }
}
