//! 설정 › 계정: the logins members sign in with (`protocol::vault`). A row
//! per login — its site and username, who may use it without asking (each
//! taken back with ×), its last use — with change and delete (a second
//! click); a form to add one or change it, the password masked and never
//! shown again; and the latest uses. The password goes once, to the daemon
//! machine's Keychain: no screen gets it back.

use equipe_core::protocol::Command;
use equipe_core::protocol::vault::{Login, LoginDraft};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use super::settings::{Line, Section};
use crate::icons::Icon;
use crate::theme::{CONTROL, Palette, R_MD, SMALL};
use crate::ui::{self, Kind};
use crate::Shell;

/// The form and the second clicks.
pub struct AccountsUi {
    pub site: Entity<InputState>,
    pub username: Entity<InputState>,
    pub password: Entity<InputState>,
    pub note: Entity<InputState>,
    /// The form open: `Some(None)` for a new login, `Some(Some(id))` to change one.
    pub editing: Option<Option<u32>>,
    pub problem: Option<String>,
    pub deleting: Option<u32>,
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
            site: cx.new(|cx| InputState::new(window, cx).placeholder(t!("accounts.site_placeholder"))),
            username: cx.new(|cx| InputState::new(window, cx).placeholder(t!("accounts.username_placeholder"))),
            password: cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(t!("accounts.password_placeholder"))),
            note: cx.new(|cx| InputState::new(window, cx).placeholder(t!("accounts.note_placeholder"))),
            editing: None,
            problem: None,
            deleting: None,
            profile_name: cx.new(|cx| InputState::new(window, cx).placeholder(t!("accounts.profile_name_placeholder"))),
            profile_label: cx.new(|cx| InputState::new(window, cx).placeholder(t!("accounts.profile_label_placeholder"))),
            adding_profile: false,
            profile_problem: None,
            deleting_profile: None,
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn outcome_word(outcome: &str) -> &'static str {
    match outcome {
        "signed_in" => t!("accounts.used_signed_in"),
        "declined" => t!("accounts.used_declined"),
        "no_form" => t!("accounts.used_no_form"),
        _ => t!("accounts.used_failed"),
    }
}

/// `frontend · 2시간 전 · 로그인함`.
fn use_words(login: &Login, now: u64) -> Option<String> {
    let u = login.uses.first()?;
    Some(format!("{} · {} · {}", u.member, crate::activity::ago(now.saturating_sub(u.at)), outcome_word(&u.outcome)))
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

pub(super) fn sections(shell: &Shell, cx: &mut Context<Shell>) -> Vec<Section> {
    let p = shell.palette;
    let ui = &shell.settings.accounts;
    let now = now_secs();
    let machine = shell.endpoint.label();
    let mut lines: Vec<Line> = Vec::new();
    for login in &shell.model.logins {
        let id = login.id;
        let mut desc = login.username.clone();
        if let Some(note) = &login.note {
            desc = format!("{desc} · {note}");
        }
        if let Some(used) = use_words(login, now) {
            desc = format!("{desc} · {}", t!("accounts.last_used", what = used));
        }
        let deleting = ui.deleting == Some(id);
        let controls = h_flex()
            .gap_2()
            .child(ui::button(p, SharedString::from(format!("login-edit-{id}")), Kind::Ghost, t!("accounts.edit")).on_click(cx.listener(move |this, _, window, cx| this.open_login_form(Some(id), window, cx))))
            .child(if deleting {
                ui::button(p, SharedString::from(format!("login-del-{id}")), Kind::Danger, t!("accounts.delete_sure")).on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.accounts.deleting = None;
                    this.send_command(Command::DeleteLogin { id });
                    cx.notify();
                }))
            } else {
                ui::button(p, SharedString::from(format!("login-del-{id}")), Kind::Ghost, t!("accounts.delete")).on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.accounts.deleting = Some(id);
                    cx.notify();
                }))
            });
        // Who may use it without asking: each taken back with ×.
        let allowed = (!login.allowed.is_empty()).then(|| {
            h_flex().gap_1p5().flex_wrap().items_center().child(div().text_size(px(12.)).text_color(p.muted).child(t!("accounts.allowed"))).children(login.allowed.iter().map(|who| {
                let (who_shown, who_taken) = (who.rsplit('/').next().unwrap_or(who).to_owned(), who.clone());
                h_flex()
                    .id(SharedString::from(format!("login-allowed-{id}-{who}")))
                    .h(px(22.))
                    .pl_2()
                    .pr_1()
                    .gap_1()
                    .items_center()
                    .rounded(px(11.))
                    .bg(p.wash)
                    .text_size(px(12.))
                    .text_color(p.text2)
                    .child(who_shown)
                    .child(
                        crate::ui::x_box(SharedString::from(format!("login-forget-{id}-{who}")), p.muted)
                            .cursor_pointer()
                            .hover(move |s| s.bg(p.hair))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.send_command(Command::ForgetLoginAllowance { id, who: who_taken.clone() });
                                cx.notify();
                            })),
                    )
            }))
        });
        let mut line = Line::new(login.site.clone(), desc).words(format!("{} {}", login.username, login.note.clone().unwrap_or_default())).control(controls);
        if let Some(allowed) = allowed {
            line = line.below(allowed);
        }
        lines.push(line);
    }
    // The form, in the list's place for a change, at its end for a new one.
    let form = ui.editing.map(|editing| {
        let field = |label: &'static str, input: &Entity<InputState>| v_flex().w_full().gap_1().child(div().text_size(px(12.)).text_color(p.muted).child(label)).child(boxed(p, input));
        v_flex()
            .w_full()
            .gap_3()
            .pt_1()
            .child(h_flex().w_full().gap_3().child(div().flex_1().child(field(t!("accounts.site"), &ui.site))).child(div().flex_1().child(field(t!("accounts.username"), &ui.username))))
            .child(h_flex().w_full().gap_3().child(div().flex_1().child(field(if editing.is_some() { t!("accounts.password_new") } else { t!("accounts.password") }, &ui.password))).child(div().flex_1().child(field(t!("accounts.note"), &ui.note))))
            .children(ui.problem.clone().map(|why| div().text_size(px(SMALL)).text_color(p.danger()).child(why)))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("accounts.form_note", machine = machine.clone())))
                    .child(ui::button(p, "login-cancel", Kind::Ghost, t!("accounts.cancel")).on_click(cx.listener(|this, _, window, cx| this.close_login_form(window, cx))))
                    .child(ui::button(p, "login-save", Kind::Primary, t!("accounts.save")).on_click(cx.listener(|this, _, window, cx| this.save_login_form(window, cx)))),
            )
    });
    let add = Line::new(t!("accounts.add"), t!("accounts.add_desc")).words(t!("accounts.add_words"));
    let add = match (form, ui.editing.is_some()) {
        (Some(form), true) => add.below(form),
        _ => add.control(ui::button(p, "login-add", Kind::Secondary, t!("accounts.add_button")).icon(Icon::Plus).on_click(cx.listener(|this, _, window, cx| this.open_login_form(None, window, cx)))),
    };
    if lines.is_empty() {
        lines.push(Line::new(t!("accounts.none"), t!("accounts.none_desc")));
    }
    lines.push(add);
    let mut out = vec![Section::new(t!("accounts.section_logins"), lines).foot(t!("accounts.foot", machine = machine))];
    // The latest uses, across every login.
    let mut uses: Vec<(&Login, &equipe_core::protocol::vault::Use)> = shell.model.logins.iter().flat_map(|l| l.uses.iter().map(move |u| (l, u))).collect();
    uses.sort_by_key(|(_, u)| std::cmp::Reverse(u.at));
    if !uses.is_empty() {
        let used: Vec<Line> = uses
            .into_iter()
            .take(12)
            .map(|(l, u)| Line::new(format!("{} · {}", u.member, l.site), format!("{} · {} · {}", l.username, outcome_word(&u.outcome), crate::activity::ago(now.saturating_sub(u.at)))).words(u.squad.clone()))
            .collect();
        out.push(Section::new(t!("accounts.section_uses"), used));
    }
    out
}

/// 설정 › 브라우저 › 브라우저 프로필: other people's accounts, each in a Chrome of its own —
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

    /// The form, empty for a new login or filled for a change (the password
    /// left empty: it stays as kept unless one is typed).
    pub fn open_login_form(&mut self, id: Option<u32>, window: &mut Window, cx: &mut Context<Self>) {
        let login = id.and_then(|id| self.model.logins.iter().find(|l| l.id == id).cloned());
        let ui = &self.settings.accounts;
        let site = login.as_ref().map(|l| l.url.clone().unwrap_or_else(|| l.site.clone())).unwrap_or_default();
        let (username, note) = login.as_ref().map(|l| (l.username.clone(), l.note.clone().unwrap_or_default())).unwrap_or_default();
        let (s, u, pw, n) = (ui.site.clone(), ui.username.clone(), ui.password.clone(), ui.note.clone());
        s.update(cx, |i, cx| i.set_value(site, window, cx));
        u.update(cx, |i, cx| i.set_value(username, window, cx));
        pw.update(cx, |i, cx| i.set_value(String::new(), window, cx));
        n.update(cx, |i, cx| i.set_value(note, window, cx));
        self.settings.accounts.editing = Some(id);
        self.settings.accounts.problem = None;
        if id.is_some() { pw.update(cx, |i, cx| i.focus(window, cx)) } else { s.update(cx, |i, cx| i.focus(window, cx)) }
        cx.notify();
    }

    pub fn close_login_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pw = self.settings.accounts.password.clone();
        pw.update(cx, |i, cx| i.set_value(String::new(), window, cx));
        self.settings.accounts.editing = None;
        self.settings.accounts.problem = None;
        cx.notify();
    }

    /// The form's login to the daemon, once it reads right; the password box
    /// is emptied at once.
    fn save_login_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = self.settings.accounts.editing else { return };
        let ui = &self.settings.accounts;
        let site = ui.site.read(cx).value().trim().to_owned();
        let username = ui.username.read(cx).value().trim().to_owned();
        let password = ui.password.read(cx).value().to_string();
        let note = ui.note.read(cx).value().trim().to_owned();
        let problem = if site.is_empty() || equipe_core::vault::site_of(&site).is_empty() {
            Some(t!("accounts.need_site"))
        } else if username.is_empty() {
            Some(t!("accounts.need_username"))
        } else if editing.is_none() && password.is_empty() {
            Some(t!("accounts.need_password"))
        } else {
            None
        };
        if let Some(why) = problem {
            self.settings.accounts.problem = Some(why.to_owned());
            return cx.notify();
        }
        let draft = LoginDraft { id: editing, site, username, password: (!password.is_empty()).then_some(password), note: (!note.is_empty()).then_some(note) };
        self.send_command(Command::SaveLogin { draft });
        self.close_login_form(window, cx);
    }
}
