//! Settings › Accounts: a line that opens the Logins page (`views/logins.rs`),
//! where the logins members sign in with are kept and managed, and — under
//! Settings › Browser — the browser profiles: other people's accounts, each
//! in a Chrome of its own (open one to sign in there, add one, drop one with
//! a second click). A login's password goes once to the daemon machine's
//! secret store: no screen gets it back.

use equipe_core::protocol::Command;
use equipe_core::t;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use super::settings::{Line, Section};
use crate::icons::Icon;
use crate::theme::{CONTROL, Palette, R_MD, SMALL};
use crate::ui::{self, Kind};
use crate::Shell;

/// The browser profile form and its second click.
pub struct AccountsUi {
    /// A browser profile being added: its name and whose accounts it holds.
    pub profile_name: Entity<InputState>,
    pub profile_label: Entity<InputState>,
    pub adding_profile: bool,
    pub profile_problem: Option<String>,
    pub deleting_profile: Option<String>,
}

impl AccountsUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        Self {
            profile_name: cx.new(|cx| InputState::new(window, cx).placeholder(t!("accounts.profile_name_placeholder"))),
            profile_label: cx.new(|cx| InputState::new(window, cx).placeholder(t!("accounts.profile_label_placeholder"))),
            adding_profile: false,
            profile_problem: None,
            deleting_profile: None,
        }
    }
}

fn boxed(p: Palette, input: &Entity<InputState>) -> AnyElement {
    div()
        .w_full()
        .h(px(CONTROL + 4.))
        .px_3()
        .flex()
        .items_center()
        .rounded(px(R_MD))
        .border_1()
        .border_color(p.hair)
        .bg(p.code_bg)
        .text_size(px(13.))
        .child(Input::new(input).appearance(false).bordered(false))
        .into_any_element()
}

/// The one line that opens the Logins page, with how many are kept.
pub(super) fn sections(shell: &Shell, cx: &mut Context<Shell>) -> Vec<Section> {
    let p = shell.palette;
    let kept = shell.model.logins.len();
    let auto = shell.model.logins.iter().filter(|l| l.auto).count();
    let desc = if kept == 0 { t!("accounts.logins_none").to_owned() } else { t!("accounts.logins_count", kept = kept, auto = auto) };
    let open = ui::button(p, "settings-open-logins", Kind::Secondary, t!("accounts.logins_open")).icon(Icon::Lock).on_click(cx.listener(|this, _, _, cx| this.open_logins(cx)));
    let line = Line::new(t!("loginspage.title"), desc).words(t!("accounts.add_words")).control(open);
    vec![Section::new(t!("accounts.section_logins"), vec![line]).foot(t!("accounts.foot", machine = shell.endpoint.label()))]
}

/// Settings › Browser › Browser profiles: other people's accounts, each in a Chrome of its own —
/// open one to sign in there, add one, drop one (a second click).
pub(super) fn profiles_section(shell: &Shell, cx: &mut Context<Shell>) -> Section {
    let p = shell.palette;
    let ui = &shell.settings.accounts;
    let mut lines: Vec<Line> = Vec::new();
    for profile in &shell.model.browser_profiles {
        let name = profile.name.clone();
        let deleting = ui.deleting_profile.as_deref() == Some(name.as_str());
        let (open, drop) = (name.clone(), name.clone());
        let controls = h_flex()
            .gap_2()
            .child(ui::button(p, SharedString::from(format!("profile-open-{name}")), Kind::Secondary, t!("accounts.profile_open")).icon(Icon::Globe).on_click(cx.listener(move |this, _, window, cx| this.open_profile_tab(&open, window, cx))))
            .child(if deleting {
                ui::button(p, SharedString::from(format!("profile-del-{name}")), Kind::Danger, t!("accounts.delete_sure")).on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.accounts.deleting_profile = None;
                    this.send_command(Command::DeleteBrowserProfile { name: drop.clone() });
                    cx.notify();
                }))
            } else {
                ui::button(p, SharedString::from(format!("profile-del-{name}")), Kind::Ghost, t!("accounts.delete")).on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.accounts.deleting_profile = Some(drop.clone());
                    cx.notify();
                }))
            });
        let desc = if profile.label.is_empty() { t!("accounts.profile_tools", server = equipe_core::browser::server_name(Some(&name))) } else { format!("{} · {}", profile.label, t!("accounts.profile_tools", server = equipe_core::browser::server_name(Some(&name)))) };
        lines.push(Line::new(name.clone(), desc).words(format!("{} {} {}", name, profile.label, t!("accounts.profile_words"))).control(controls));
    }
    let add = Line::new(t!("accounts.profile_add"), t!("accounts.profile_add_desc")).words(t!("accounts.profile_add_words"));
    let add = if ui.adding_profile {
        let field = |label: &'static str, input: &Entity<InputState>| v_flex().w_full().gap_1().child(div().text_size(px(12.)).text_color(p.muted).child(label)).child(boxed(p, input));
        add.below(
            v_flex()
                .w_full()
                .gap_3()
                .pt_1()
                .child(h_flex().w_full().gap_3().child(div().flex_1().child(field(t!("accounts.profile_name"), &ui.profile_name))).child(div().flex_1().child(field(t!("accounts.profile_label"), &ui.profile_label))))
                .children(ui.profile_problem.clone().map(|why| div().text_size(px(SMALL)).text_color(p.danger()).child(why)))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(div().flex_1())
                        .child(ui::button(p, "profile-cancel", Kind::Ghost, t!("accounts.cancel")).on_click(cx.listener(|this, _, _, cx| {
                            this.settings.accounts.adding_profile = false;
                            this.settings.accounts.profile_problem = None;
                            cx.notify();
                        })))
                        .child(ui::button(p, "profile-save", Kind::Primary, t!("accounts.save")).on_click(cx.listener(|this, _, window, cx| this.save_profile_form(window, cx)))),
                ),
        )
    } else {
        add.control(ui::button(p, "profile-add", Kind::Secondary, t!("accounts.add_button")).icon(Icon::Plus).on_click(cx.listener(|this, _, window, cx| this.open_profile_form(window, cx))))
    };
    lines.push(add);
    Section::new(t!("accounts.section_profiles"), lines).foot(t!("accounts.profiles_foot", machine = shell.endpoint.label()))
}

/// A profile's name as the daemon takes it: 1–24 lowercase letters, digits or `-`, not `main`.
pub fn profile_name_ok(name: &str) -> bool {
    (1..=24).contains(&name.len()) && name != "main" && !name.starts_with('-') && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl Shell {
    fn open_profile_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ui = &self.settings.accounts;
        let (n, l) = (ui.profile_name.clone(), ui.profile_label.clone());
        n.update(cx, |i, cx| {
            i.set_value(String::new(), window, cx);
            i.focus(window, cx);
        });
        l.update(cx, |i, cx| i.set_value(String::new(), window, cx));
        self.settings.accounts.adding_profile = true;
        self.settings.accounts.profile_problem = None;
        cx.notify();
    }

    fn save_profile_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ui = &self.settings.accounts;
        let name = ui.profile_name.read(cx).value().trim().to_lowercase();
        let label = ui.profile_label.read(cx).value().trim().to_owned();
        if !profile_name_ok(&name) {
            self.settings.accounts.profile_problem = Some(t!("accounts.profile_name_bad").to_owned());
            return cx.notify();
        }
        self.send_command(Command::SaveBrowserProfile { profile: equipe_core::protocol::BrowserProfile { name: name.clone(), label } });
        self.settings.accounts.adding_profile = false;
        self.settings.accounts.profile_problem = None;
        // Straight to signing in there.
        self.open_profile_tab(&name, window, cx);
    }

    /// Profile `name`'s Chrome, in the browser panel, at the sign-in page: the
    /// person signs in there as that person.
    pub fn open_profile_tab(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen != crate::Screen::Chat || !self.browser.read(cx).open {
            self.toggle_browser(window, cx);
        }
        let profile = name.to_owned();
        self.browser.update(cx, |b, cx| b.new_tab_in(Some("https://accounts.google.com/".into()), Some(profile), cx));
        cx.notify();
    }
}
