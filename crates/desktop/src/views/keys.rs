//! "Keys & secrets" on the Logins page: the keys, tokens and passwords
//! members obtained and gave equipe to keep (`secret_save`,
//! `protocol::secrets::SquadSecret`) — for each, its name, the service, the
//! projects it may be used on, who saved it, and when it was last used. Under
//! a click: show it once (after a confirmation; it hides again by itself),
//! copy it, change the projects it may be used on (or any), delete it (asked
//! twice).
//!
//! The list carries facts only. A value comes from the daemon only when the
//! person asks to see it ([`Command::RevealSquadSecret`]), is held here for
//! half a minute at most, and is dropped when hidden, when the row closes
//! or when the page is left.

use std::time::{Duration, Instant};

use equipe_core::protocol::Command;
use equipe_core::protocol::secrets::{NO_PROJECT, SquadSecret};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::theme::{Palette, SMALL};
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

/// How long a value stays on screen once shown.
const SHOWN_FOR: Duration = Duration::from_secs(30);

/// One key, as a row and a command name it.
pub type Which = (String, String);

/// What the person is asked to confirm before it happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirm {
    Reveal(Which),
    Delete(Which),
}

/// A value on screen, for a little while.
pub struct Revealed {
    pub which: Which,
    pub value: String,
    pub at: Instant,
}

pub struct KeysUi {
    /// The row opened to its actions.
    pub open: Option<Which>,
    pub confirm: Option<Confirm>,
    pub revealed: Option<Revealed>,
    /// The reveal this screen asked for and waits on: its number and key.
    pub asked: Option<(u64, Which)>,
    next_ask: u64,
    /// The projects a key may be used on, comma between: the box of the row's edit.
    pub projects: Entity<InputState>,
    pub editing: bool,
    pub any: bool,
    /// What the copy button last did, for the row: `true` once copied.
    pub copied: bool,
}

impl KeysUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        Self {
            open: None,
            confirm: None,
            revealed: None,
            asked: None,
            next_ask: 1,
            projects: cx.new(|cx| InputState::new(window, cx).placeholder(t!("keys.projects_placeholder"))),
            editing: false,
            any: false,
            copied: false,
        }
    }

    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        self.projects.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Nothing of a value stays: the page was left, a row closed.
    pub fn forget(&mut self) {
        self.revealed = None;
        self.asked = None;
        self.confirm = None;
        self.copied = false;
    }

    fn shown(&self, which: &Which) -> Option<&str> {
        self.revealed.as_ref().filter(|r| r.which == *which && r.at.elapsed() < SHOWN_FOR).map(|r| r.value.as_str())
    }
}

fn which(secret: &SquadSecret) -> Which {
    (secret.squad.clone(), secret.name.clone())
}

fn element(prefix: &str, which: &Which) -> SharedString {
    SharedString::from(format!("{prefix}-{}-{}", which.0, which.1))
}

fn projects_text(secret: &SquadSecret) -> String {
    if secret.any_project {
        return t!("keys.any_project").to_owned();
    }
    if secret.projects.is_empty() {
        return t!("keys.no_projects").to_owned();
    }
    secret.projects.iter().map(|p| if p == NO_PROJECT { t!("keys.no_project").to_owned() } else { p.clone() }).collect::<Vec<_>>().join(", ")
}

fn kind_words(secret: &SquadSecret) -> &'static str {
    use equipe_core::protocol::secrets::SecretKind as K;
    match secret.kind {
        K::ApiKey => t!("keys.kind_api_key"),
        K::Token => t!("keys.kind_token"),
        K::Password => t!("keys.kind_password"),
        K::ConnectionString => t!("keys.kind_connection_string"),
        K::Other => t!("keys.kind_other"),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The section: a heading, then a card with a row for each key.
pub fn section(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let keys = &shell.model.squad_secrets;
    let mut body = v_flex().w_full().rounded(px(crate::theme::R_LG)).border_1().border_color(p.hair).bg(p.raised).overflow_hidden();
    if keys.is_empty() {
        body = body.child(
            v_flex()
                .w_full()
                .py_6()
                .px_4()
                .gap_1()
                .items_center()
                .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(t!("keys.empty_title")))
                .child(div().max_w(px(460.)).text_center().text_size(px(13.)).line_height(px(19.)).text_color(p.muted).child(t!("keys.empty_body"))),
        );
    }
    for (ix, secret) in keys.iter().enumerate() {
        body = body.child(row(shell, secret, ix > 0, cx));
    }
    let aside = (!keys.is_empty()).then(|| t!("keys.count", n = keys.len()));
    let mut head = h_flex().w_full().px_1().gap_2().items_baseline().child(div().id("keys-title").text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).role(Role::Heading).child(t!("keys.title")));
    if let Some(aside) = aside {
        head = head.child(div().text_size(px(12.)).text_color(p.muted).child(aside));
    }
    v_flex()
        .id("logins-keys")
        .w_full()
        .gap_2()
        .child(head)
        .child(div().w_full().px_1().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("keys.subtitle", machine = shell.endpoint.label())))
        .child(body)
        .into_any_element()
}

fn row(shell: &Shell, secret: &SquadSecret, divider: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui.keys;
    let which = which(secret);
    let open = ui.open.as_ref() == Some(&which);
    let now = now_secs();
    let last = match &secret.last_used {
        Some(u) => t!("keys.last_used", member = u.member, when = crate::activity::ago(now.saturating_sub(u.at))),
        None => t!("keys.never_used").to_owned(),
    };
    let made = t!("keys.created", member = secret.created_by, when = crate::activity::ago(now.saturating_sub(secret.created_at)));
    let toggle_which = which.clone();
    let toggle = ui::icon_button(p, element("key-more", &which), if open { Icon::ChevronUp } else { Icon::ChevronDown }, t!("keys.details", name = secret.name), None).expanded(open).on_click(cx.listener({
        let which = toggle_which.clone();
        move |this, _, _, cx| this.keys_toggle(which.clone(), cx)
    }));
    let line = h_flex()
        .id(element("key-row", &which))
        .w_full()
        .px_4()
        .py_3()
        .gap_3()
        .items_center()
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .aria_label(format!("{}, {}", secret.name, secret.service))
        .on_click(cx.listener({
            let which = which.clone();
            move |this, _, _, cx| this.keys_toggle(which.clone(), cx)
        }))
        .child(div().flex_none().size(px(32.)).flex().items_center().justify_center().rounded(px(8.)).bg(p.wash).child(icon(Icon::Lock, 14., p.text2)))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().truncate().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(secret.name.clone()))
                .child(div().truncate().text_size(px(12.)).text_color(p.muted).child(format!("{} · {} · {}", secret.service, kind_words(secret), made))),
        )
        .child(
            h_flex().flex_none().gap_3().items_center().child(chip(p, projects_text(secret))).child(div().w(px(190.)).truncate().text_size(px(11.5)).text_color(p.muted).child(last)).child(toggle),
        );
    v_flex().id(element("key-block", &which)).w_full().when(divider, |b| b.border_t_1().border_color(p.hair)).child(line).children(open.then(|| panel(shell, secret, &which, cx))).into_any_element()
}

fn chip(p: Palette, text: String) -> Div {
    div().flex_none().max_w(px(220.)).h(px(22.)).px_2().flex().items_center().rounded_full().bg(p.wash).text_size(px(12.)).text_color(p.text2).whitespace_nowrap().overflow_hidden().child(text)
}

/// What a row opened shows: the value (when asked for), where it may be used, and the actions.
fn panel(shell: &Shell, secret: &SquadSecret, which: &Which, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui.keys;
    let panel = v_flex().w_full().gap_3().px_4().pb_4().pt_1().pl(px(16. + 32. + 12.));
    let mut details: Vec<String> = Vec::new();
    if !secret.note.is_empty() {
        details.push(secret.note.clone());
    }
    if let Some(ticket) = secret.created_ticket {
        details.push(t!("keys.on_ticket", id = ticket));
    }
    if let Some(u) = &secret.last_used {
        let project = if u.project.is_empty() { t!("keys.no_project").to_owned() } else { u.project.clone() };
        details.push(t!("keys.last_use_detail", member = u.member, project = project));
    }
    let details = v_flex().gap_0p5().children(details.into_iter().map(|d| div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(d)));

    // A value on screen, hidden again by itself.
    if let Some(value) = ui.shown(which) {
        let copy_which = which.clone();
        let hide = which.clone();
        let _ = hide;
        let shown = v_flex()
            .w_full()
            .gap_2()
            .p_3()
            .rounded(px(crate::theme::R_MD))
            .bg(p.wash)
            .child(div().id(element("key-value", which)).w_full().text_size(px(13.)).line_height(px(19.)).text_color(p.text).font_family(crate::theme::MONO).child(value.to_owned()))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().text_size(px(12.)).text_color(p.muted).child(t!("keys.shown_note")))
                    .child(ui::button(p, element("key-copy", which), Kind::Secondary, if ui.copied { t!("keys.copied") } else { t!("keys.copy") }).icon(if ui.copied { Icon::Check } else { Icon::Copy }).on_click(cx.listener(move |this, _, _, cx| this.keys_copy(&copy_which, cx))))
                    .child(ui::button(p, element("key-hide", which), Kind::Ghost, t!("keys.hide")).on_click(cx.listener(|this, _, _, cx| {
                        this.logins_ui.keys.forget();
                        cx.notify();
                    }))),
            );
        return panel.child(details).child(shown).child(actions(shell, secret, which, cx)).into_any_element();
    }
    panel.child(details).child(scope_editor(shell, secret, which, cx)).child(actions(shell, secret, which, cx)).into_any_element()
}

/// Where a key may be used: a box of project names and a switch for any.
fn scope_editor(shell: &Shell, secret: &SquadSecret, which: &Which, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui.keys;
    if !ui.editing {
        return h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(div().text_size(px(12.)).text_color(p.muted).child(t!("keys.used_on")))
            .child(div().flex_1().min_w_0().truncate().text_size(px(12.5)).text_color(p.text2).child(projects_text(secret)))
            .child(ui::button(p, element("key-edit", which), Kind::Secondary, t!("keys.edit_projects")).icon(Icon::Pencil).on_click(cx.listener({
                let which = which.clone();
                move |this, _, window, cx| this.keys_edit(&which, window, cx)
            })))
            .into_any_element();
    }
    let handle = cx.entity().downgrade();
    let any = ui::switch(p, element("key-any", which), ui.any).name(t!("keys.any_project")).on_flip(move |on, _, cx| {
        let _ = handle.update(cx, |this, cx| {
            this.logins_ui.keys.any = on;
            cx.notify();
        });
    });
    v_flex()
        .w_full()
        .gap_2()
        .child(div().text_size(px(12.)).text_color(p.muted).child(t!("keys.projects_label")))
        .child(div().w_full().text_size(px(13.)).child(Input::new(&ui.projects)))
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(any)
                .child(div().flex_1().text_size(px(12.5)).text_color(p.text2).child(t!("keys.any_project_desc")))
                .child(ui::button(p, element("key-edit-cancel", which), Kind::Ghost, t!("accounts.cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.logins_ui.keys.editing = false;
                    cx.notify();
                })))
                .child(ui::button(p, element("key-edit-save", which), Kind::Primary, t!("accounts.save")).on_click(cx.listener({
                    let which = which.clone();
                    move |this, _, _, cx| this.keys_save_scope(&which, cx)
                }))),
        )
        .into_any_element()
}

/// Show, delete — each asked first.
fn actions(shell: &Shell, secret: &SquadSecret, which: &Which, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui.keys;
    let mut bar = h_flex().w_full().gap_2().items_center().flex_wrap();
    match &ui.confirm {
        Some(Confirm::Reveal(w)) if w == which => {
            bar = bar
                .child(div().flex_1().min_w_0().text_size(px(12.5)).line_height(px(18.)).text_color(p.text2).child(t!("keys.reveal_ask", name = secret.name)))
                .child(ui::button(p, element("key-reveal-no", which), Kind::Ghost, t!("accounts.cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.logins_ui.keys.confirm = None;
                    cx.notify();
                })))
                .child(ui::button(p, element("key-reveal-yes", which), Kind::Primary, t!("keys.reveal_yes")).on_click(cx.listener({
                    let which = which.clone();
                    move |this, _, _, cx| this.keys_reveal(&which, cx)
                })));
        }
        Some(Confirm::Delete(w)) if w == which => {
            bar = bar
                .child(div().flex_1().min_w_0().text_size(px(12.5)).line_height(px(18.)).text_color(p.text2).child(t!("keys.delete_ask", name = secret.name)))
                .child(ui::button(p, element("key-del-no", which), Kind::Ghost, t!("accounts.cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.logins_ui.keys.confirm = None;
                    cx.notify();
                })))
                .child(ui::button(p, element("key-del-yes", which), Kind::Danger, t!("accounts.delete")).on_click(cx.listener({
                    let which = which.clone();
                    move |this, _, _, cx| this.keys_delete(&which, cx)
                })));
        }
        _ => {
            bar = bar
                .child(ui::button(p, element("key-reveal", which), Kind::Secondary, t!("keys.reveal")).icon(Icon::Lock).on_click(cx.listener({
                    let which = which.clone();
                    move |this, _, _, cx| {
                        this.logins_ui.keys.confirm = Some(Confirm::Reveal(which.clone()));
                        cx.notify();
                    }
                })))
                .child(div().flex_1())
                .child(ui::button(p, element("key-del", which), Kind::Ghost, t!("accounts.delete")).icon(Icon::Trash).on_click(cx.listener({
                    let which = which.clone();
                    move |this, _, _, cx| {
                        this.logins_ui.keys.confirm = Some(Confirm::Delete(which.clone()));
                        cx.notify();
                    }
                })));
        }
    }
    bar.into_any_element()
}

impl Shell {
    /// The page opened: what the daemon keeps, asked for; nothing of a value kept from before.
    pub fn keys_open(&mut self) {
        self.logins_ui.keys.forget();
        self.send_command(Command::SquadSecrets);
    }

    fn keys_toggle(&mut self, which: Which, cx: &mut Context<Self>) {
        let ui = &mut self.logins_ui.keys;
        let was = ui.open.as_ref() == Some(&which);
        ui.forget();
        ui.editing = false;
        ui.open = if was { None } else { Some(which) };
        cx.notify();
    }

    fn keys_reveal(&mut self, which: &Which, cx: &mut Context<Self>) {
        let ui = &mut self.logins_ui.keys;
        let ask = ui.next_ask;
        ui.next_ask += 1;
        ui.asked = Some((ask, which.clone()));
        ui.confirm = None;
        self.send_command(Command::RevealSquadSecret { squad: which.0.clone(), name: which.1.clone(), ask });
        cx.notify();
    }

    /// The daemon's answer to this screen's own ask; another screen's is none of ours.
    pub fn keys_revealed(&mut self, squad: &str, name: &str, ask: u64, value: String, cx: &mut Context<Self>) {
        let ui = &mut self.logins_ui.keys;
        if ui.asked.as_ref().is_some_and(|(n, w)| *n == ask && w.0 == squad && w.1 == name) && self.screen == Screen::Logins {
            ui.asked = None;
            ui.copied = false;
            ui.revealed = Some(Revealed { which: (squad.to_owned(), name.to_owned()), value, at: Instant::now() });
            cx.notify();
        }
    }

    fn keys_copy(&mut self, which: &Which, cx: &mut Context<Self>) {
        let ui = &mut self.logins_ui.keys;
        if let Some(value) = ui.shown(which).map(str::to_owned) {
            cx.write_to_clipboard(ClipboardItem::new_string(value));
            ui.copied = true;
            cx.notify();
        }
    }

    fn keys_edit(&mut self, which: &Which, window: &mut Window, cx: &mut Context<Self>) {
        let Some(secret) = self.model.squad_secrets.iter().find(|s| (s.squad.as_str(), s.name.as_str()) == (which.0.as_str(), which.1.as_str())).cloned() else { return };
        let text = secret.projects.iter().filter(|p| p.as_str() != NO_PROJECT).cloned().collect::<Vec<_>>().join(", ");
        let ui = &mut self.logins_ui.keys;
        ui.projects.update(cx, |s, cx| s.set_value(text, window, cx));
        ui.any = secret.any_project;
        ui.editing = true;
        cx.notify();
    }

    fn keys_save_scope(&mut self, which: &Which, cx: &mut Context<Self>) {
        let typed = self.logins_ui.keys.projects.read(cx).value().to_string();
        let projects: Vec<String> = typed.split(',').map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned).collect();
        let any_project = self.logins_ui.keys.any;
        self.logins_ui.keys.editing = false;
        self.send_command(Command::SetSquadSecretProjects { squad: which.0.clone(), name: which.1.clone(), projects, any_project });
        cx.notify();
    }

    fn keys_delete(&mut self, which: &Which, cx: &mut Context<Self>) {
        let ui = &mut self.logins_ui.keys;
        ui.forget();
        ui.open = None;
        self.send_command(Command::DeleteSquadSecret { squad: which.0.clone(), name: which.1.clone() });
        cx.notify();
    }
}
