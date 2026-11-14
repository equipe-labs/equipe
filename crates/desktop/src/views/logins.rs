//! Logins: one page for everything about the logins members sign in with
//! (`protocol::vault`, `crate::logins`). From the top: the sites members wait
//! on a sign-in for right now (open the browser on one), the saved logins —
//! a search, a row each with its scope, the "signs in by itself" switch, its
//! health and last use, and under a click: change where it is used, edit,
//! replace the password, delete (asked twice, then taken back for a few
//! seconds) — how often members sent the person to sign in and why, with the
//! step that would help the sites asking most, and the sites never offered.
//!
//! A password is typed only into equipe's own masked box, goes once to the
//! daemon machine's secret store and is never shown, copied or read back: this
//! page holds logins without it, and counts, hosts and causes from the record.
//! Every change is the person's own click, sent as a person-only command.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

use equipe_core::protocol::Command;
use equipe_core::protocol::vault::{Health, Login, LoginDraft};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::logins::{self as rules, Choice, Fix, PendingDelete, Problem, Scope};
use crate::theme::{CONTROL, Palette, R_LG, R_MD, SMALL};
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

/// The page's column, at its widest.
const COLUMN: f32 = 820.;
/// The trend's tallest bar.
const TREND_H: f32 = 56.;

/// Which form is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// A new login: site, username, password, the switch.
    Add,
    /// A login's username, sign-in page and note.
    Edit(u32),
    /// A login's password, replaced — a masked box and nothing else.
    Password(u32),
}

/// The page's boxes and what is open.
pub struct LoginsUi {
    pub search: Entity<InputState>,
    pub site: Entity<InputState>,
    pub username: Entity<InputState>,
    pub url: Entity<InputState>,
    pub note: Entity<InputState>,
    pub password: Entity<InputState>,
    /// "Sign in to a site": the site or address box, whether it is open, where the
    /// sign-in stands, and whether the separate Chrome window is asked for.
    pub signin: Entity<InputState>,
    pub signin_open: bool,
    pub signin_view: Option<rules::SignInView>,
    pub signin_window: bool,
    /// When this screen asked for one: its panel opens for the daemon's answer.
    pub signin_asked: Option<Instant>,
    pub form: Option<Form>,
    /// The new login's "signs in by itself" switch.
    pub auto: bool,
    pub problem: Option<Problem>,
    /// The row opened to its actions.
    pub open: Option<u32>,
    /// A delete asked once: the login waiting for the second click.
    pub confirm: Option<u32>,
    /// A delete that can still be taken back.
    pub pending: Option<PendingDelete>,
    /// The look-back of "Asked again", in days.
    pub days: u32,
    pub scroll: ScrollHandle,
    /// Login autopilot's switches and boxes (`views::autopilot`).
    pub autopilot: super::autopilot::AutopilotUi,
    /// "Keys & secrets": what members kept, and a value shown once (`views::keys`).
    pub keys: super::keys::KeysUi,
    /// "Connections": the service whose Disconnect waits for its second click.
    pub connections: super::connections::ConnectionsUi,
    stops: RefCell<HashMap<String, FocusHandle>>,
}

impl LoginsUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        Self {
            search: cx.new(|cx| InputState::new(window, cx).placeholder(t!("loginspage.search_placeholder"))),
            site: cx.new(|cx| InputState::new(window, cx).placeholder(t!("accounts.site_placeholder"))),
            username: cx.new(|cx| InputState::new(window, cx).placeholder(t!("accounts.username_placeholder"))),
            url: cx.new(|cx| InputState::new(window, cx).placeholder(t!("loginspage.url_placeholder"))),
            note: cx.new(|cx| InputState::new(window, cx).placeholder(t!("accounts.note_placeholder"))),
            password: cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(t!("accounts.password_placeholder"))),
            signin: cx.new(|cx| InputState::new(window, cx).placeholder(t!("loginspage.signin_placeholder"))),
            signin_open: false,
            signin_view: None,
            signin_window: false,
            signin_asked: None,
            form: None,
            // A new login signs in by itself unless the person turns it off: saving it is their leave.
            auto: true,
            problem: None,
            open: None,
            confirm: None,
            pending: None,
            days: rules::RANGES[0],
            scroll: ScrollHandle::new(),
            autopilot: super::autopilot::AutopilotUi::new(window, cx),
            keys: super::keys::KeysUi::new(window, cx),
            connections: super::connections::ConnectionsUi::default(),
            stops: RefCell::new(HashMap::new()),
        }
    }

    /// Whether one of the boxes has the keyboard: what is typed is the box's.
    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        [&self.search, &self.site, &self.username, &self.url, &self.note, &self.password, &self.signin].iter().any(|i| i.read(cx).focus_handle(cx).is_focused(window)) || self.keys.typing(window, cx)
    }

    /// A button's place in the Tab order, steady between draws.
    fn stop(&self, key: &str, cx: &mut Context<Shell>) -> FocusHandle {
        self.stops.borrow_mut().entry(key.to_owned()).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
    }
}

/// The page's own subscriptions: ⏎ in a box sends the form, typing in the search filters.
pub fn subscribe(shell: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) {
    let ui = &shell.logins_ui;
    let (search, boxes) = (ui.search.clone(), [ui.site.clone(), ui.username.clone(), ui.url.clone(), ui.note.clone(), ui.password.clone()]);
    shell._subscriptions.push(cx.subscribe_in(&search, window, |_, _, event: &InputEvent, _, cx| {
        if matches!(event, InputEvent::Change) {
            cx.notify();
        }
    }));
    for input in boxes {
        shell._subscriptions.push(cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.logins_submit(window, cx);
            }
        }));
    }
    let signin = shell.logins_ui.signin.clone();
    shell._subscriptions.push(cx.subscribe_in(&signin, window, |this, _, event: &InputEvent, _, cx| {
        if matches!(event, InputEvent::PressEnter { .. }) {
            let typed = this.logins_ui.signin.read(cx).value().to_string();
            this.sign_in_to(&typed, cx);
        }
    }));
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
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

fn field(p: Palette, label: &'static str, input: &Entity<InputState>) -> AnyElement {
    v_flex().w_full().flex_1().min_w_0().gap_1().child(div().text_size(px(12.)).text_color(p.muted).child(label)).child(boxed(p, input)).into_any_element()
}

fn problem_words(problem: Problem) -> &'static str {
    match problem {
        Problem::NeedSite => t!("accounts.need_site"),
        Problem::NeedUsername => t!("accounts.need_username"),
        Problem::NeedPassword => t!("accounts.need_password"),
    }
}

fn cause_words(code: &str) -> &'static str {
    match code {
        "session_expired" => t!("loginspage.cause_session_expired"),
        "no_login_kept" => t!("loginspage.cause_no_login_kept"),
        "captcha" => t!("loginspage.cause_captcha"),
        "two_factor" => t!("loginspage.cause_two_factor"),
        "passkey" => t!("loginspage.cause_passkey"),
        "device_code" => t!("loginspage.cause_device_code"),
        "consent" => t!("loginspage.cause_consent"),
        _ => t!("loginspage.cause_other"),
    }
}

/// A section: a heading (and what it holds), then its card.
fn section(p: Palette, id: &'static str, title: impl Into<SharedString>, aside: Option<String>, body: impl IntoElement) -> AnyElement {
    v_flex()
        .id(id)
        .w_full()
        .gap_2()
        .child(
            h_flex()
                .w_full()
                .px_1()
                .gap_2()
                .items_baseline()
                .child(div().id(SharedString::from(format!("{id}-title"))).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).role(Role::Heading).child(title.into()))
                .children(aside.map(|a| div().text_size(px(12.)).text_color(p.muted).child(a))),
        )
        .child(body)
        .into_any_element()
}

fn card(p: Palette) -> Div {
    v_flex().w_full().rounded(px(R_LG)).border_1().border_color(p.hair).bg(p.raised).overflow_hidden()
}

/// What a card says when it has nothing yet: what it is for, in a sentence.
fn quiet(p: Palette, title: &'static str, body: String) -> AnyElement {
    v_flex().w_full().py_6().px_4().gap_1().items_center().child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(title)).child(div().max_w(px(460.)).text_center().text_size(px(13.)).line_height(px(19.)).text_color(p.muted).child(body)).into_any_element()
}

fn chip(p: Palette, text: String, tone: Rgba) -> Div {
    div().flex_none().h(px(22.)).px_2().flex().items_center().rounded_full().bg(p.wash).text_size(px(12.)).text_color(tone).whitespace_nowrap().child(text)
}

/// The site's tile: its first letter, in a colour that stays the site's.
fn tile(p: Palette, site: &str) -> AnyElement {
    let color = p.member(rules::tint(site));
    div()
        .flex_none()
        .size(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .bg(Rgba { a: 0.16, ..color })
        .text_size(px(14.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(rules::letter(site))
        .into_any_element()
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui;
    let header = ui::title_bar(h_flex())
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pr_3()
        .pl(px(shell.header_inset(16., cx)))
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(icon(Icon::Lock, 15., p.text2))
        .child(div().min_w_0().truncate().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("loginspage.title")))
        .child(div().flex_1())
        .child(
            ui::button(p, "logins-sign-in", Kind::Primary, t!("loginspage.signin"))
                .icon(Icon::Globe)
                .focus(&ui.stop("logins-sign-in", cx))
                .on_click(cx.listener(|this, _, window, cx| this.open_sign_in(None, window, cx))),
        )
        .child(
            ui::button(p, "logins-add", Kind::Secondary, t!("loginspage.add"))
                .icon(Icon::Plus)
                .focus(&ui.stop("logins-add", cx))
                .on_click(cx.listener(|this, _, window, cx| this.logins_open_form(Form::Add, None, window, cx))),
        );

    let mut column = v_flex().w_full().max_w(px(COLUMN)).gap_5().child(
        div().w_full().px_1().text_size(px(13.)).line_height(px(19.)).text_color(p.muted).child(t!("loginspage.subtitle")),
    );
    if shell.model.login_insights.as_ref().is_some_and(|i| !i.can_keep) {
        column = column.child(
            h_flex()
                .id("logins-no-store")
                .role(Role::Alert)
                .w_full()
                .gap_2()
                .p_3()
                .items_start()
                .rounded(px(R_MD))
                .bg(p.danger_bg())
                .text_size(px(13.))
                .line_height(px(19.))
                .text_color(p.text)
                .child(div().pt(px(2.)).flex_none().child(icon(Icon::Warning, 14., p.danger())))
                .child(div().flex_1().min_w_0().child(t!("loginspage.no_store", machine = shell.endpoint.label()))),
        );
    }
    if shell.logins_ui.signin_open || shell.logins_ui.signin_view.is_some() {
        column = column.child(sign_in_section(shell, cx));
    }
    for offer in offers_section(shell, cx) {
        column = column.child(offer);
    }
    column = column.child(waiting_section(shell, cx)).child(saved_section(shell, cx)).child(super::connections::section(shell, cx)).child(super::keys::section(shell, cx)).child(autopilot_section(shell, cx)).child(asked_section(shell, cx)).child(never_section(shell, cx));
    column = column.child(div().w_full().px_1().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("loginspage.foot", machine = shell.endpoint.label())));

    let body = div().id("logins-body").flex_1().min_h_0().overflow_y_scroll().track_scroll(&ui.scroll).child(div().w_full().px_5().py_5().flex().justify_center().child(column));
    v_flex().flex_1().min_w_0().h_full().child(header).child(body).children(undo_strip(shell, cx)).into_any_element()
}

// ── sign in to a site ──

/// "Sign in to a site": the box, the quick picks, where the browser opens, and
/// where the sign-in stands.
fn sign_in_section(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui;
    let machine = shell.endpoint.label();
    let remote = shell.endpoint.is_remote();
    let mut body = card(p);
    if ui.signin_open {
        let waits = rules::waiting(&shell.model.browser_waits, &shell.model.logins);
        let (recent, common) = rules::sign_in_picks(&waits, shell.model.login_insights.as_ref(), &shell.model.logins, 4);
        let picks = |heading: &'static str, id: &'static str, list: Vec<rules::Pick>, cx: &mut Context<Shell>| -> Option<AnyElement> {
            if list.is_empty() {
                return None;
            }
            let buttons = list.into_iter().enumerate().map(|(ix, pick)| {
                let url = pick.url.clone();
                ui::button(p, SharedString::from(format!("{id}-{ix}")), Kind::Secondary, pick.label).on_click(cx.listener(move |this, _, _, cx| this.sign_in_to(&url, cx))).into_any_element()
            });
            Some(v_flex().w_full().gap_1().child(div().text_size(px(12.)).text_color(p.muted).child(heading)).child(h_flex().w_full().flex_wrap().gap_1p5().children(buttons)).into_any_element())
        };
        let window_on = ui.signin_window;
        let handle = cx.entity().downgrade();
        let toggle = ui::switch(p, "logins-sign-in-window", window_on).name(t!("loginspage.signin_window")).on_flip(move |on, _, cx| {
            let _ = handle.update(cx, |shell, cx| {
                shell.logins_ui.signin_window = on;
                cx.notify();
            });
        });
        let mut form = v_flex()
            .w_full()
            .p_4()
            .gap_3()
            .child(div().text_size(px(13.)).line_height(px(19.)).text_color(p.text2).child(t!("loginspage.signin_body")))
            .child(h_flex().w_full().gap_2().items_center().child(div().flex_1().min_w_0().child(boxed(p, &ui.signin))).child(ui::button(p, "logins-sign-in-go", Kind::Primary, t!("loginspage.signin_go")).icon(Icon::Globe).on_click(cx.listener(|this, _, _, cx| {
                let typed = this.logins_ui.signin.read(cx).value().to_string();
                this.sign_in_to(&typed, cx);
            }))).child(ui::button(p, "logins-sign-in-close", Kind::Ghost, t!("loginspage.signin_close")).on_click(cx.listener(|this, _, _, cx| {
                this.logins_ui.signin_open = false;
                cx.notify();
            }))))
            .children(picks(t!("loginspage.signin_recent"), "logins-pick-recent", recent, cx))
            .children(picks(t!("loginspage.signin_common"), "logins-pick-common", common, cx))
            .child(h_flex().w_full().gap_2().items_center().child(toggle).child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("loginspage.signin_window"))));
        // Where it will open, when that isn't the screen in front of the person.
        if remote {
            form = form.child(div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(if window_on { t!("loginspage.signin_remote_window", machine = machine) } else { t!("loginspage.signin_remote_panel", machine = machine) }));
        }
        body = body.child(form);
    }
    if let Some(view) = &ui.signin_view {
        let site = view.site().to_owned();
        let note = match rules::note_of(view, remote) {
            rules::Note::Opening => t!("loginspage.signin_opening", site = site),
            rules::Note::Panel => t!("loginspage.signin_panel", site = site),
            rules::Note::Window => t!("loginspage.signin_window_open", site = site),
            rules::Note::RemoteWindow => t!("loginspage.signin_window_remote", site = site, machine = machine),
            rules::Note::DoneOffered => t!("loginspage.signin_done_offered", site = site),
            rules::Note::Done => t!("loginspage.signin_done", site = site),
            rules::Note::Failed => t!("loginspage.signin_failed", machine = machine),
            rules::Note::Closed => t!("loginspage.signin_closed", site = site),
            rules::Note::BadAddress => t!("loginspage.signin_bad").to_owned(),
        };
        let bad = matches!(view, rules::SignInView::BadAddress | rules::SignInView::Failed);
        let tone = if bad { p.danger() } else { p.text };
        body = body.child(
            h_flex()
                .id("logins-sign-in-note")
                .role(if bad { Role::Alert } else { Role::Status })
                .w_full()
                .px_4()
                .py_3()
                .gap_2()
                .items_start()
                .when(ui.signin_open, |r| r.border_t_1().border_color(p.hair))
                .child(div().pt(px(2.)).flex_none().child(icon(if bad { Icon::Warning } else { Icon::Globe }, 14., if bad { p.danger() } else { p.accent })))
                .child(div().flex_1().min_w_0().text_size(px(13.)).line_height(px(19.)).text_color(tone).child(note))
                .child(ui::button(p, "logins-sign-in-dismiss", Kind::Ghost, t!("loginspage.signin_dismiss")).on_click(cx.listener(|this, _, _, cx| {
                    this.logins_ui.signin_view = None;
                    cx.notify();
                }))),
        );
    }
    section(p, "logins-sign-in", t!("loginspage.signin_title"), None, body)
}

/// The "Save this login?" cards asked, here too: a sign-in finished in a
/// window or panel that isn't this page leaves its card where the person is.
fn offers_section(shell: &Shell, cx: &mut Context<Shell>) -> Vec<AnyElement> {
    let p = shell.palette;
    let ids: Vec<u64> = shell.model.login_offers.iter().map(|o| o.id).collect();
    ids.into_iter()
        .filter_map(|id| super::login_offers::card(shell, id, cx))
        .map(|card| div().id("logins-offer").w_full().p_3().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).shadow_sm().child(card).into_any_element())
        .collect()
}

// ── waiting now ──

fn waiting_section(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui;
    let waits = rules::waiting(&shell.model.browser_waits, &shell.model.logins);
    let mut body = card(p);
    if waits.is_empty() {
        body = body.child(div().w_full().px_4().py_3().text_size(px(13.)).text_color(p.muted).child(t!("loginspage.waiting_none")));
    }
    for (ix, w) in waits.iter().enumerate() {
        let (page, site) = (w.page, w.site.clone());
        let kept = w.kept;
        let save_site = w.site.clone();
        let open_site = w.site.clone();
        let open = ui::button(p, SharedString::from(format!("logins-wait-open-{ix}")), Kind::Primary, t!("loginspage.waiting_open"))
            .icon(Icon::Globe)
            .focus(&ui.stop(&format!("logins-wait-open-{ix}"), cx))
            .on_click(cx.listener(move |this, _, window, cx| this.open_waiting_login(page, &open_site, window, cx)));
        let save = (!kept).then(|| {
            ui::button(p, SharedString::from(format!("logins-wait-save-{ix}")), Kind::Ghost, t!("loginspage.fix_save"))
                .focus(&ui.stop(&format!("logins-wait-save-{ix}"), cx))
                .on_click(cx.listener(move |this, _, window, cx| this.logins_open_form(Form::Add, Some(save_site.clone()), window, cx)))
        });
        body = body.child(
            h_flex()
                .id(SharedString::from(format!("logins-wait-{ix}")))
                .w_full()
                .px_4()
                .py_3()
                .gap_3()
                .items_center()
                .when(ix > 0, |r| r.border_t_1().border_color(p.hair))
                .child(icon(Icon::Hand, 15., p.amber))
                .child(v_flex().flex_1().min_w_0().gap_0p5().child(div().truncate().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(site)).child(div().truncate().text_size(px(12.)).text_color(p.muted).child(t!("loginspage.waiting_members", members = w.members.join(", ")))))
                .child(h_flex().flex_none().gap_1().children(save).child(open)),
        );
    }
    section(p, "logins-waiting", t!("loginspage.waiting_title"), None, body)
}

// ── saved logins ──

fn saved_section(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui;
    let hidden: Vec<u32> = ui.pending.iter().map(|d| d.id).collect();
    let query = ui.search.read(cx).value().trim().to_owned();
    let all: Vec<&Login> = shell.model.logins.iter().filter(|l| !hidden.contains(&l.id)).collect();
    let shown = rules::visible(&shell.model.logins, &query, &hidden);
    let now = now_secs();

    let search = h_flex()
        .w_full()
        .h(px(CONTROL + 2.))
        .px_3()
        .gap_2()
        .items_center()
        .rounded(px(R_MD))
        .bg(p.wash)
        .child(icon(Icon::Search, 13., p.muted))
        .child(div().flex_1().min_w_0().text_size(px(13.)).child(Input::new(&ui.search).appearance(false).bordered(false)))
        .children((!query.is_empty()).then(|| {
            ui::x_box("logins-search-clear", p.muted, t!("loginspage.search_clear")).cursor_pointer().on_click(cx.listener(|this, _, window, cx| {
                this.logins_ui.search.update(cx, |s, cx| s.set_value(String::new(), window, cx));
                cx.notify();
            }))
        }));

    let mut body = card(p);
    if !all.is_empty() {
        body = body.child(div().w_full().p_3().border_b_1().border_color(p.hair).child(search));
    }
    if ui.form == Some(Form::Add) {
        body = body.child(add_form(shell, cx));
    }
    if all.is_empty() && ui.form != Some(Form::Add) {
        let add = ui::button(p, "logins-empty-add", Kind::Secondary, t!("loginspage.add")).icon(Icon::Plus).focus(&ui.stop("logins-empty-add", cx)).on_click(cx.listener(|this, _, window, cx| this.logins_open_form(Form::Add, None, window, cx)));
        body = body.child(quiet(p, t!("loginspage.empty_title"), t!("loginspage.empty_body").to_owned())).child(div().w_full().pb_5().flex().justify_center().child(add));
    } else if shown.is_empty() && !all.is_empty() {
        body = body.child(div().w_full().p_4().text_size(px(13.)).text_color(p.muted).child(t!("loginspage.no_match", query = query)));
    }
    for (ix, login) in shown.iter().enumerate() {
        body = body.child(login_row(shell, login, ix > 0, now, cx));
    }
    let count = (!all.is_empty()).then(|| t!("loginspage.saved_count", n = all.len()));
    section(p, "logins-saved", t!("loginspage.saved_title"), count, body)
}

fn health_badge(p: Palette, login: &Login) -> AnyElement {
    let (words, color) = match login.health() {
        Health::Ok => (t!("loginspage.health_ok"), p.member(3)),
        Health::Failed => (t!("loginspage.health_failed"), p.danger()),
        Health::NeverUsed => (t!("loginspage.health_never"), p.muted),
    };
    h_flex().flex_none().max_w(px(200.)).gap_1p5().items_center().text_size(px(12.)).text_color(color).child(div().flex_none().size(px(7.)).rounded_full().bg(color)).child(words).into_any_element()
}

fn scope_chip(p: Palette, login: &Login, squad: &str) -> AnyElement {
    let (text, tone) = match rules::scope_of(login, squad) {
        Scope::Every => (t!("loginspage.scope_every").to_owned(), p.text2),
        Scope::ThisSquad => (t!("loginspage.scope_this").to_owned(), p.text2),
        Scope::Squad(name) => (name, p.text2),
        Scope::Squads(n) => (t!("loginspage.scope_squads", n = n), p.text2),
        Scope::AskFirst => (t!("loginspage.scope_ask").to_owned(), p.muted),
    };
    chip(p, text, tone).into_any_element()
}

fn login_row(shell: &Shell, login: &Login, divider: bool, now: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui;
    let id = login.id;
    let open = ui.open == Some(id);
    let mut sub = login.username.clone();
    if let Some(note) = &login.note {
        sub = format!("{sub} · {note}");
    }
    let used = login.last_used().map(|at| t!("loginspage.last_used", when = crate::activity::ago(now.saturating_sub(at))));
    let handle = cx.entity().downgrade();
    let auto = ui::switch(p, SharedString::from(format!("login-auto-{id}")), login.auto).name(t!("accounts.auto", site = login.site)).on_flip(move |on, _, cx| {
        let _ = handle.update(cx, |this, cx| {
            this.send_command(Command::SetLoginAuto { id, auto: on });
            cx.notify();
        });
    });
    let auto = crate::tips::tip(div().id(SharedString::from(format!("login-auto-tip-{id}"))).flex_none().child(auto), p, t!("accounts.auto_tip"), None);
    let toggle = ui::icon_button(p, SharedString::from(format!("login-more-{id}")), if open { Icon::ChevronUp } else { Icon::ChevronDown }, t!("loginspage.details", site = login.site), None)
        .expanded(open)
        .focus(&ui.stop(&format!("login-more-{id}"), cx))
        .on_click(cx.listener(move |this, _, _, cx| this.logins_toggle_row(id, cx)));
    let line = h_flex()
        .id(SharedString::from(format!("login-row-{id}")))
        .w_full()
        .px_4()
        .py_3()
        .gap_3()
        .items_center()
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .aria_label(format!("{}, {}", login.site, login.username))
        .on_click(cx.listener(move |this, _, _, cx| this.logins_toggle_row(id, cx)))
        .child(tile(p, &login.site))
        .child(v_flex().flex_1().min_w_0().gap_0p5().child(div().truncate().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(login.site.clone())).child(div().truncate().text_size(px(12.)).text_color(p.muted).child(sub)))
        .child(
            h_flex()
                .flex_none()
                .gap_3()
                .items_center()
                .child(scope_chip(p, login, &shell.model.squad))
                .child(v_flex().w(px(200.)).gap_0p5().child(health_badge(p, login)).children(used.map(|u| div().truncate().text_size(px(11.5)).text_color(p.muted).child(u))))
                .child(auto)
                .child(super::autopilot::toggle(shell, login, cx))
                .child(toggle),
        );
    // The login's autopilot switches, when its "Autopilot" is pressed.
    let autopilot = super::autopilot::below(shell, login, cx).map(|switches| div().w_full().px_4().pb_4().pl(px(16. + 32. + 12.)).child(switches));
    v_flex()
        .id(SharedString::from(format!("login-block-{id}")))
        .w_full()
        .when(divider, |b| b.border_t_1().border_color(p.hair))
        .child(line)
        .children(autopilot)
        .children(open.then(|| login_panel(shell, login, now, cx)))
        .into_any_element()
}

/// What a row opened shows: its form, or its details and actions.
fn login_panel(shell: &Shell, login: &Login, now: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui;
    let id = login.id;
    let panel = v_flex().w_full().gap_3().px_4().pb_4().pt_1().pl(px(16. + 32. + 12.));
    let mut form_buttons = |save_label: &'static str| {
        h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(18.)).text_color(p.danger()).children(ui.problem.map(problem_words)))
            .child(ui::button(p, "login-form-cancel", Kind::Ghost, t!("accounts.cancel")).focus(&ui.stop("login-form-cancel", cx)).on_click(cx.listener(|this, _, window, cx| this.logins_close_form(window, cx))))
            .child(ui::button(p, "login-form-save", Kind::Primary, save_label).focus(&ui.stop("login-form-save", cx)).on_click(cx.listener(|this, _, window, cx| this.logins_submit(window, cx))))
    };
    match ui.form {
        Some(Form::Edit(open)) if open == id => panel
            .child(h_flex().w_full().gap_3().child(field(p, t!("accounts.username"), &ui.username)).child(field(p, t!("loginspage.url_label"), &ui.url)))
            .child(field(p, t!("accounts.note"), &ui.note))
            .child(form_buttons(t!("accounts.save")))
            .into_any_element(),
        Some(Form::Password(open)) if open == id => panel
            .child(field(p, t!("loginspage.password_new"), &ui.password))
            .child(div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("loginspage.password_note", machine = shell.endpoint.label())))
            .child(form_buttons(t!("loginspage.password_save")))
            .into_any_element(),
        _ => {
            let squad = shell.model.squad.clone();
            let mut details: Vec<String> = Vec::new();
            if let Some(url) = &login.url {
                details.push(t!("loginspage.signin_page", url = url));
            }
            if let Some(at) = login.password_at {
                details.push(t!("loginspage.password_age", when = crate::activity::ago(now.saturating_sub(at))));
            }
            if login.stopped.is_some() && login.auto {
                details.push(t!("accounts.auto_stopped").to_owned());
            }
            let scope = h_flex().gap_2().items_center().flex_wrap().child(div().text_size(px(12.)).text_color(p.muted).child(t!("loginspage.scope_label"))).children(Choice::ALL.into_iter().map(|choice| {
                let (key, label) = match choice {
                    Choice::AskFirst => ("ask", t!("loginspage.scope_ask")),
                    Choice::ThisSquad => ("this", t!("loginspage.scope_this")),
                    Choice::Every => ("every", t!("loginspage.scope_every")),
                };
                let squads = choice.squads(&squad);
                let mut b = ui::button(p, SharedString::from(format!("login-scope-{key}-{id}")), Kind::Secondary, label).selected(choice.is(login, &squad)).focus(&ui.stop(&format!("login-scope-{key}-{id}"), cx));
                b = b.on_click(cx.listener(move |this, _, _, cx| {
                    this.send_command(Command::SetLoginScope { id, squads: squads.clone() });
                    cx.notify();
                }));
                b
            }));
            let confirming = ui.confirm == Some(id);
            let actions = h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .flex_wrap()
                .child(ui::button(p, SharedString::from(format!("login-edit-{id}")), Kind::Secondary, t!("loginspage.edit")).icon(Icon::Pencil).focus(&ui.stop(&format!("login-edit-{id}"), cx)).on_click(cx.listener(move |this, _, window, cx| this.logins_open_form(Form::Edit(id), None, window, cx))))
                .child(ui::button(p, SharedString::from(format!("login-pw-{id}")), Kind::Secondary, t!("loginspage.password_replace")).icon(Icon::Lock).focus(&ui.stop(&format!("login-pw-{id}"), cx)).on_click(cx.listener(move |this, _, window, cx| this.logins_open_form(Form::Password(id), None, window, cx))))
                .child(div().flex_1())
                .children(if confirming {
                    vec![
                        div().text_size(px(12.5)).text_color(p.text2).child(t!("loginspage.delete_ask", site = login.site)).into_any_element(),
                        ui::button(p, SharedString::from(format!("login-del-no-{id}")), Kind::Ghost, t!("accounts.cancel")).focus(&ui.stop(&format!("login-del-no-{id}"), cx)).on_click(cx.listener(|this, _, _, cx| {
                            this.logins_ui.confirm = None;
                            cx.notify();
                        })).into_any_element(),
                        ui::button(p, SharedString::from(format!("login-del-yes-{id}")), Kind::Danger, t!("accounts.delete")).focus(&ui.stop(&format!("login-del-yes-{id}"), cx)).on_click(cx.listener(move |this, _, _, cx| this.logins_delete(id, cx))).into_any_element(),
                    ]
                } else {
                    vec![ui::button(p, SharedString::from(format!("login-del-{id}")), Kind::Ghost, t!("accounts.delete")).icon(Icon::Trash).focus(&ui.stop(&format!("login-del-{id}"), cx)).on_click(cx.listener(move |this, _, _, cx| {
                        this.logins_ui.confirm = Some(id);
                        cx.notify();
                    })).into_any_element()]
                });
            panel
                .child(scope)
                .children((!details.is_empty()).then(|| v_flex().gap_0p5().children(details.into_iter().map(|d| div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(d)))))
                .child(actions)
                .into_any_element()
        }
    }
}

fn add_form(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui;
    let handle = cx.entity().downgrade();
    v_flex()
        .id("logins-add-form")
        .w_full()
        .gap_3()
        .p_4()
        .border_b_1()
        .border_color(p.hair)
        .bg(p.wash)
        .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("loginspage.add_title")))
        .child(h_flex().w_full().gap_3().child(field(p, t!("accounts.site"), &ui.site)).child(field(p, t!("accounts.username"), &ui.username)))
        .child(field(p, t!("accounts.password"), &ui.password))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(ui::switch(p, "login-form-auto", ui.auto).name(t!("accounts.auto_form")).on_flip(move |on, _, cx| {
                    let _ = handle.update(cx, |this, cx| {
                        this.logins_ui.auto = on;
                        cx.notify();
                    });
                }))
                .child(v_flex().flex_1().min_w_0().gap_0p5().child(div().text_size(px(SMALL)).text_color(p.text2).child(t!("accounts.auto_form"))).child(div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("accounts.auto_form_note")))),
        )
        .child(div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("loginspage.password_note", machine = shell.endpoint.label())))
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(p.danger()).children(ui.problem.map(problem_words)))
                .child(ui::button(p, "login-form-cancel", Kind::Ghost, t!("accounts.cancel")).focus(&ui.stop("login-form-cancel", cx)).on_click(cx.listener(|this, _, window, cx| this.logins_close_form(window, cx))))
                .child(ui::button(p, "login-form-save", Kind::Primary, t!("accounts.save")).icon(Icon::Lock).focus(&ui.stop("login-form-save", cx)).on_click(cx.listener(|this, _, window, cx| this.logins_submit(window, cx)))),
        )
        .into_any_element()
}

// ── login autopilot ──

/// What equipe may finish by itself beyond the password: this week, the
/// mailbox, Messages, keep-alive, command-line sign-ins.
fn autopilot_section(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let card = super::settings::draw_section(shell.palette, super::autopilot::section(shell, cx));
    section(shell.palette, "logins-autopilot", t!("autopilot.title"), None, card)
}

// ── asked again ──

fn asked_section(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui;
    let seg = |days: u32| {
        let on = ui.days == days;
        let label = if days == 7 { t!("loginspage.range_7") } else { t!("loginspage.range_30") };
        div()
            .id(SharedString::from(format!("logins-range-{days}")))
            .px_2p5()
            .h(px(26.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(12.5))
            .role(Role::Tab)
            .aria_selected(on)
            .when(on, |d| d.bg(p.popover()).text_color(p.text).font_weight(FontWeight::SEMIBOLD))
            .when(!on, |d| d.text_color(p.text2).hover(move |s| s.text_color(p.text)))
            .on_click(cx.listener(move |this, _, _, cx| this.logins_set_days(days, cx)))
            .child(label)
    };
    let range = h_flex().flex_none().p(px(2.)).gap(px(2.)).rounded(px(8.)).bg(p.wash).children(rules::RANGES.into_iter().map(seg));
    let head = h_flex().w_full().items_center().child(div().flex_1()).child(range);

    let mut body = card(p);
    let Some(insights) = shell.model.login_insights.as_ref().filter(|i| i.days == ui.days) else {
        body = body.child(div().w_full().p_4().text_size(px(13.)).text_color(p.muted).child(t!("plan_use.adding_up")));
        return section(p, "logins-asked", t!("loginspage.asked_title"), None, v_flex().w_full().gap_2().child(head).child(body));
    };
    if insights.asked == 0 && insights.signed_in == 0 {
        body = body.child(quiet(p, t!("loginspage.asked_empty_title"), t!("loginspage.asked_empty_body", days = insights.days)));
        return section(p, "logins-asked", t!("loginspage.asked_title"), None, v_flex().w_full().gap_2().child(head).child(body));
    }
    let stat = |value: String, label: &'static str, sub: Option<String>| {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_0p5()
            .child(div().text_size(px(22.)).line_height(px(26.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(value))
            .child(div().text_size(px(12.)).text_color(p.text2).child(label))
            .children(sub.map(|s| div().text_size(px(11.5)).text_color(p.muted).child(s)))
    };
    let share = match insights.by_equipe_percent() {
        Some(percent) => stat(format!("{percent}%"), t!("loginspage.stat_auto"), Some(t!("loginspage.stat_auto_sub", equipe = insights.signed_in_by_equipe, total = insights.signed_in))),
        None => stat("–".to_owned(), t!("loginspage.stat_auto"), Some(t!("loginspage.stat_auto_none").to_owned())),
    };
    let reask = match rules::typical_reask(insights) {
        Some(secs) => stat(equipe_core::logins::span(secs), t!("loginspage.stat_reask"), Some(t!("loginspage.stat_reask_sub").to_owned())),
        None => stat("–".to_owned(), t!("loginspage.stat_reask"), Some(t!("loginspage.stat_reask_none").to_owned())),
    };
    body = body.child(h_flex().w_full().gap_4().p_4().child(stat(insights.asked.to_string(), t!("loginspage.stat_asked"), Some(t!("loginspage.stat_asked_sub", days = insights.days)))).child(share).child(reask));

    // The trend: asks per day, today's bar the strongest.
    let bars = rules::bars(&insights.trend, 0.04);
    let last = bars.len().saturating_sub(1);
    let trend = h_flex()
        .id("logins-trend")
        .role(Role::Image)
        .aria_label(t!("loginspage.trend_aria", n = insights.asked, days = insights.days))
        .w_full()
        .h(px(TREND_H))
        .gap(px(if insights.days > 7 { 2. } else { 6. }))
        .items_end()
        .children(bars.iter().enumerate().map(|(i, f)| div().flex_1().min_w_0().h(px((TREND_H * f).max(2.))).rounded(px(2.)).bg(Rgba { a: if i == last { 1. } else { 0.5 }, ..p.accent })));
    body = body.child(v_flex().w_full().gap_1p5().px_4().pb_4().child(div().text_size(px(12.)).text_color(p.muted).child(t!("loginspage.trend_title"))).child(trend));

    // Why, by cause.
    let top = insights.by_cause.first().map_or(1, |c| c.count.max(1));
    if !insights.by_cause.is_empty() {
        body = body.child(
            v_flex().w_full().gap_1().px_4().py_3().border_t_1().border_color(p.hair).child(div().pb_1().text_size(px(12.)).text_color(p.muted).child(t!("loginspage.causes_title"))).children(insights.by_cause.iter().map(|c| {
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(div().w(px(190.)).flex_none().truncate().text_size(px(SMALL)).text_color(p.text).child(cause_words(&c.cause)))
                    .child(div().flex_1().h(px(4.)).rounded_full().bg(p.hair).child(div().h_full().w(relative(c.count as f32 / top as f32)).rounded_full().bg(p.accent)))
                    .child(div().w(px(36.)).flex_none().text_right().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(c.count.to_string()))
            })),
        );
    }

    // Top sites to fix, with the one step that helps.
    let fixes = rules::suggestions(insights, &shell.model.logins, 5);
    if !fixes.is_empty() {
        body = body.child(v_flex().w_full().gap_1().px_4().py_3().border_t_1().border_color(p.hair).child(div().pb_1().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("loginspage.fix_title"))).children(fixes.into_iter().enumerate().map(|(ix, s)| fix_row(shell, s, ix, cx))));
    }

    // By site.
    if !insights.sites.is_empty() {
        body = body.child(
            v_flex().w_full().gap_1().px_4().py_3().border_t_1().border_color(p.hair).child(div().pb_1().text_size(px(12.)).text_color(p.muted).child(t!("loginspage.sites_title"))).children(insights.sites.iter().take(8).map(|s| {
                let mut facts: Vec<String> = Vec::new();
                if let Some(secs) = s.reask_after {
                    facts.push(t!("loginspage.site_reask", span = equipe_core::logins::span(secs)));
                }
                if s.signed_in > 0 {
                    facts.push(t!("loginspage.site_auto", equipe = s.signed_in_by_equipe, total = s.signed_in));
                }
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_baseline()
                    .child(div().w(px(190.)).flex_none().truncate().text_size(px(SMALL)).text_color(p.text).child(s.site.clone()))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(p.muted).child(facts.join(" · ")))
                    .child(div().flex_none().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(t!("loginspage.site_asks", n = s.asked)))
            })),
        );
    }
    section(p, "logins-asked", t!("loginspage.asked_title"), None, v_flex().w_full().gap_2().child(head).child(body))
}

fn fix_row(shell: &Shell, s: rules::Suggestion, ix: usize, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui;
    let (why, label) = match s.fix {
        Fix::SaveLogin => (t!("loginspage.fix_save_why", n = s.asked), t!("loginspage.fix_save")),
        Fix::TurnOnAuto { .. } => (t!("loginspage.fix_auto_why", n = s.asked), t!("loginspage.fix_auto")),
        Fix::ReplacePassword { .. } => (t!("loginspage.fix_replace_why", n = s.asked), t!("loginspage.password_replace")),
    };
    let (site, fix) = (s.site.clone(), s.fix.clone());
    let key = format!("logins-fix-{ix}");
    h_flex()
        .id(SharedString::from(format!("logins-fixrow-{ix}")))
        .w_full()
        .gap_3()
        .items_center()
        .child(v_flex().flex_1().min_w_0().child(div().truncate().text_size(px(SMALL)).text_color(p.text).child(s.site)).child(div().truncate().text_size(px(12.)).text_color(p.muted).child(why)))
        .child(ui::button(p, SharedString::from(key.clone()), Kind::Secondary, label).focus(&ui.stop(&key, cx)).on_click(cx.listener(move |this, _, window, cx| this.logins_fix(&site, fix.clone(), window, cx))))
        .into_any_element()
}

// ── never save ──

fn never_section(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui;
    let never: Vec<String> = shell.model.login_insights.as_ref().map(|i| i.never.clone()).unwrap_or_default();
    let mut body = card(p);
    if never.is_empty() {
        body = body.child(quiet(p, t!("loginspage.never_empty_title"), t!("loginspage.never_empty_body").to_owned()));
    }
    for (ix, site) in never.iter().enumerate() {
        let drop = site.clone();
        let key = format!("logins-never-{ix}");
        body = body.child(
            h_flex()
                .id(SharedString::from(format!("logins-neverrow-{ix}")))
                .w_full()
                .px_4()
                .py_2p5()
                .gap_3()
                .items_center()
                .when(ix > 0, |r| r.border_t_1().border_color(p.hair))
                .child(tile(p, site))
                .child(div().flex_1().min_w_0().truncate().text_size(px(SMALL)).text_color(p.text).child(site.clone()))
                .child(ui::button(p, SharedString::from(key.clone()), Kind::Ghost, t!("loginspage.never_remove")).focus(&ui.stop(&key, cx)).on_click(cx.listener(move |this, _, _, cx| this.logins_forget_never(&drop, cx)))),
        );
    }
    section(p, "logins-never", t!("loginspage.never_title"), Some(t!("loginspage.never_aside").to_owned()), body)
}

// ── the strip that takes a delete back ──

fn undo_strip(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let pending = shell.logins_ui.pending.as_ref()?;
    Some(
        h_flex()
            .id("logins-undo")
            .role(Role::Status)
            .w_full()
            .flex_none()
            .px_5()
            .py_2p5()
            .gap_3()
            .items_center()
            .border_t_1()
            .border_color(p.hair)
            .bg(p.raised)
            .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(p.text).child(t!("loginspage.deleted", site = pending.site)))
            .child(ui::button(p, "logins-undo-button", Kind::Secondary, t!("loginspage.undo")).icon(Icon::Withdraw).focus(&shell.logins_ui.stop("logins-undo-button", cx)).on_click(cx.listener(|this, _, _, cx| this.logins_undo(cx))))
            .into_any_element(),
    )
}

impl Shell {
    /// The Logins page, with a fresh look at what it shows.
    pub fn open_logins(&mut self, cx: &mut Context<Self>) {
        self.send_command(Command::Logins);
        self.send_command(Command::LoginInsights { days: self.logins_ui.days });
        self.send_command(Command::Autopilot);
        self.keys_open();
        self.show(Screen::Logins, cx);
        cx.notify();
    }

    /// "Sign in to a site": the Logins page with its box open and ready —
    /// for a site or address already known (`/sign-in github.com`), it opens
    /// at once.
    pub fn open_sign_in(&mut self, site: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.open_logins(cx);
        match site.filter(|s| !s.trim().is_empty()) {
            Some(site) => self.sign_in_to(&site, cx),
            None => {
                let ui = &mut self.logins_ui;
                ui.signin_open = true;
                ui.signin.update(cx, |s, cx| s.focus(window, cx));
                cx.notify();
            }
        }
    }

    /// Open equipe's own browser at `typed` (a site or an address) — what
    /// `equipe browser <url>` does, by the daemon, wherever it runs: in the
    /// panel here, or the separate Chrome window on the daemon's Mac.
    pub fn sign_in_to(&mut self, typed: &str, cx: &mut Context<Self>) {
        let Some(url) = rules::sign_in_target(typed) else {
            self.logins_ui.signin_view = Some(rules::SignInView::BadAddress);
            cx.notify();
            return;
        };
        let ui = &mut self.logins_ui;
        let window = ui.signin_window;
        ui.signin_view = Some(rules::SignInView::Opening { site: equipe_core::vault::site_of(&url) });
        ui.signin_asked = Some(Instant::now());
        self.send_command(Command::BrowserSignIn { url, window });
        cx.notify();
    }

    /// "Open browser to log in" on a waiting site: the member's own page in
    /// the panel, in the person's hands, as before; where that page is gone,
    /// the same sign-in as "Sign in to a site".
    pub fn open_waiting_login(&mut self, page: u64, site: &str, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        if self.browser.read(cx).knows_page(page) {
            self.reveal_browser(page, cx);
        } else {
            self.sign_in_to(site, cx);
        }
    }

    /// The daemon's word on a sign-in ([`Event::BrowserSignIn`]): the line the
    /// page keeps, and — for one this screen asked for, or on the window in
    /// front — the browser panel opened on the page.
    pub fn sign_in_event(&mut self, site: &str, state: equipe_core::protocol::BrowserSignInState, window_asked: bool, window: &mut Window, cx: &mut Context<Self>) {
        use equipe_core::protocol::BrowserSignInState as State;
        let mine = self.logins_ui.signin_asked.is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(30 * 60));
        let view = rules::view_of(site, state, window_asked);
        // Another screen's sign-in: this page shows a line only for what it asked for.
        if mine || self.logins_ui.signin_view.as_ref().is_some_and(|v| v.site() == site) {
            self.logins_ui.signin_view = Some(view);
        }
        if matches!(state, State::Opened) && !window_asked && (mine || window.is_window_active()) {
            // Shown where the person is: the panel beside the conversation, its new tab picked.
            self.show_sign_in_panel(window, cx);
        }
        // Done, failed or closed: nothing more is waited for.
        if !matches!(state, State::Opened) {
            self.logins_ui.signin_asked = None;
        }
        cx.notify();
    }

    fn show_sign_in_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen != Screen::Chat {
            self.show(Screen::Chat, cx);
        }
        self.doc = None;
        if let Some(ui) = self.terminal.as_mut().filter(|_| !self.remembered.term_below) {
            ui.open = false;
        }
        if !self.browser.read(cx).open {
            self.browser.update(cx, |b, cx| b.toggle(window, cx));
            self.sync_browser(window, cx);
        }
        self.browser.update(cx, |b, cx| b.expect_new_tab(cx));
        cx.notify();
    }

    fn logins_set_days(&mut self, days: u32, cx: &mut Context<Self>) {
        if self.logins_ui.days != days {
            self.logins_ui.days = days;
            self.send_command(Command::LoginInsights { days });
            cx.notify();
        }
    }

    fn logins_toggle_row(&mut self, id: u32, cx: &mut Context<Self>) {
        let ui = &mut self.logins_ui;
        ui.open = if ui.open == Some(id) { None } else { Some(id) };
        ui.confirm = None;
        // Another row opened: its form isn't this one's.
        if ui.open != Some(id) || matches!(ui.form, Some(Form::Edit(f) | Form::Password(f)) if f != id) {
            ui.form = None;
            ui.problem = None;
        }
        cx.notify();
    }

    /// Open a form with its boxes emptied (an edit's filled from the login);
    /// the page scrolls to the top for a new login, where its form is.
    pub fn logins_open_form(&mut self, form: Form, site: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.show(Screen::Logins, cx);
        let ui = &self.logins_ui;
        let login = match form {
            Form::Edit(id) | Form::Password(id) => self.model.logins.iter().find(|l| l.id == id).cloned(),
            Form::Add => None,
        };
        let (s, u, url, n, pw) = (ui.site.clone(), ui.username.clone(), ui.url.clone(), ui.note.clone(), ui.password.clone());
        s.update(cx, |i, cx| i.set_value(site.clone().unwrap_or_default(), window, cx));
        u.update(cx, |i, cx| i.set_value(login.as_ref().map(|l| l.username.clone()).unwrap_or_default(), window, cx));
        url.update(cx, |i, cx| i.set_value(login.as_ref().and_then(|l| l.url.clone()).unwrap_or_default(), window, cx));
        n.update(cx, |i, cx| i.set_value(login.as_ref().and_then(|l| l.note.clone()).unwrap_or_default(), window, cx));
        pw.update(cx, |i, cx| i.set_value(String::new(), window, cx));
        let ui = &mut self.logins_ui;
        ui.form = Some(form);
        ui.problem = None;
        // A new login signs in by itself unless the person turns it off.
        ui.auto = true;
        ui.confirm = None;
        match form {
            Form::Add => {
                ui.scroll.set_offset(point(px(0.), px(0.)));
                if site.is_some() { u.update(cx, |i, cx| i.focus(window, cx)) } else { s.update(cx, |i, cx| i.focus(window, cx)) }
            }
            Form::Edit(id) => {
                ui.open = Some(id);
                u.update(cx, |i, cx| i.focus(window, cx));
            }
            Form::Password(id) => {
                ui.open = Some(id);
                pw.update(cx, |i, cx| i.focus(window, cx));
            }
        }
        cx.notify();
    }

    /// Close the form; the password box is emptied at once.
    pub fn logins_close_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pw = self.logins_ui.password.clone();
        pw.update(cx, |i, cx| i.set_value(String::new(), window, cx));
        self.logins_ui.form = None;
        self.logins_ui.problem = None;
        cx.notify();
    }

    /// The open form's login to the daemon, once it reads right. The password
    /// goes once, and its box is emptied.
    pub fn logins_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.logins_ui.form else { return };
        let ui = &self.logins_ui;
        let text = |i: &Entity<InputState>| i.read(cx).value().trim().to_owned();
        let (site, username, url, note) = (text(&ui.site), text(&ui.username), text(&ui.url), text(&ui.note));
        let password = ui.password.read(cx).value().to_string();
        let draft = match form {
            Form::Add => {
                if let Some(problem) = rules::check_new(&site, &username, &password) {
                    self.logins_ui.problem = Some(problem);
                    return cx.notify();
                }
                LoginDraft { id: None, site: rules::draft_site(&site, ""), username, password: Some(password), note: (!note.is_empty()).then_some(note), auto: Some(ui.auto), idp: None }
            }
            Form::Edit(id) => {
                let Some(login) = self.model.logins.iter().find(|l| l.id == id) else { return self.logins_close_form(window, cx) };
                if username.is_empty() {
                    self.logins_ui.problem = Some(Problem::NeedUsername);
                    return cx.notify();
                }
                LoginDraft { id: Some(id), site: rules::draft_site(&login.site, &url), username, password: None, note: (!note.is_empty()).then_some(note), auto: None, idp: None }
            }
            Form::Password(id) => {
                let Some(login) = self.model.logins.iter().find(|l| l.id == id) else { return self.logins_close_form(window, cx) };
                if password.is_empty() {
                    self.logins_ui.problem = Some(Problem::NeedPassword);
                    return cx.notify();
                }
                LoginDraft { id: Some(id), site: login.url.clone().unwrap_or_else(|| login.site.clone()), username: login.username.clone(), password: Some(password), note: login.note.clone(), auto: None, idp: None }
            }
        };
        self.send_command(Command::SaveLogin { draft });
        self.logins_close_form(window, cx);
    }

    /// The second click: the row goes away at once, and the daemon is told
    /// only when the time to take it back is up.
    fn logins_delete(&mut self, id: u32, cx: &mut Context<Self>) {
        let Some(login) = self.model.logins.iter().find(|l| l.id == id).cloned() else { return };
        // A delete still waiting is sent first: one can be taken back at a time.
        self.logins_commit(true);
        let now = Instant::now();
        self.logins_ui.pending = Some(PendingDelete::new(&login, now));
        self.logins_ui.confirm = None;
        self.logins_ui.open = None;
        self.logins_ui.form = None;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(crate::logins::UNDO_FOR + std::time::Duration::from_millis(150)).await;
            let _ = this.update(cx, |shell, cx| {
                shell.logins_commit(false);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Send the delete waiting, if it is due (or `now`).
    fn logins_commit(&mut self, now: bool) {
        let due = self.logins_ui.pending.as_ref().is_some_and(|d| now || d.due(Instant::now()));
        if due && let Some(pending) = self.logins_ui.pending.take() {
            self.send_command(Command::DeleteLogin { id: pending.id });
        }
    }

    fn logins_undo(&mut self, cx: &mut Context<Self>) {
        self.logins_ui.pending = None;
        cx.notify();
    }

    fn logins_forget_never(&mut self, site: &str, cx: &mut Context<Self>) {
        self.send_command(Command::ForgetNeverOffer { site: site.to_owned() });
        if let Some(insights) = self.model.login_insights.as_mut() {
            insights.never.retain(|s| s != site);
        }
        cx.notify();
    }

    /// A suggestion's one step: a new login for the site, signing in by itself
    /// turned on, or the password replaced.
    fn logins_fix(&mut self, site: &str, fix: Fix, window: &mut Window, cx: &mut Context<Self>) {
        match fix {
            Fix::SaveLogin => self.logins_open_form(Form::Add, Some(site.to_owned()), window, cx),
            Fix::TurnOnAuto { id } => {
                self.send_command(Command::SetLoginAuto { id, auto: true });
                cx.notify();
            }
            Fix::ReplacePassword { id } => {
                self.logins_open_form(Form::Password(id), None, window, cx);
                self.logins_ui.scroll.set_offset(point(px(0.), px(0.)));
            }
        }
    }

    /// The snapshot tool's `logins[:<what>]`: the page, with a row open or a form.
    #[cfg(feature = "snapshot")]
    pub fn snapshot_logins(&mut self, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.show(Screen::Logins, cx);
        match what.split_once(':').unwrap_or((what, "")) {
            ("add", _) => self.logins_open_form(Form::Add, None, window, cx),
            ("open", n) => {
                if let Some(id) = n.parse().ok().or_else(|| self.model.logins.first().map(|l| l.id)) {
                    self.logins_ui.open = Some(id);
                }
            }
            ("password", n) => {
                if let Some(id) = n.parse().ok().or_else(|| self.model.logins.first().map(|l| l.id)) {
                    self.logins_open_form(Form::Password(id), None, window, cx);
                }
            }
            ("month", _) => {
                self.logins_ui.days = 30;
                self.snapshot_logins_seed(cx);
            }
            // The Connections section on a sample: keys kept, a sign-in or two, and — `connections:card` — a Connect card up.
            ("connections", card) => {
                self.snapshot_logins_seed(cx);
                self.snapshot_connections_seed(card == "card");
                self.secrets_sync(window, cx);
                self.logins_ui.scroll.set_offset(point(px(0.), px(0.)));
            }
            _ => {}
        }
        cx.notify();
    }

    /// Sample kept keys for Connections, with no daemon asked: an Anthropic admin key used lately,
    /// a GitHub token never used, and (`card`) a Vercel token's card waiting on the person.
    #[cfg(feature = "snapshot")]
    pub fn snapshot_connections_seed(&mut self, card: bool) {
        use equipe_core::protocol::secrets::{SecretAsk, SecretField, SecretKind, SecretStore, SecretUse, SquadSecret};
        let now = now_secs();
        let squad = self.model.squad.clone();
        let key = |name: &str, service: &str, kind: SecretKind, used: Option<u64>| SquadSecret {
            squad: squad.clone(),
            name: name.into(),
            kind,
            service: service.into(),
            projects: Vec::new(),
            any_project: true,
            created_by: "backend".into(),
            created_ticket: None,
            created_at: now - 86_400 * 3,
            note: String::new(),
            last_used: used.map(|ago| SecretUse { at: now - ago, member: "backend".into(), ticket: None, project: String::new(), how: "env".into() }),
        };
        self.model.squad_secrets = vec![key("anthropic-admin-key", "anthropic", SecretKind::ApiKey, Some(2 * 3_600)), key("github-token", "github", SecretKind::Token, None), key("acme-deploy-token", "deploy.acme.example", SecretKind::Token, Some(86_400))];
        if card {
            self.model.secret_asks = vec![SecretAsk {
                id: 77,
                session: 0,
                member: String::new(),
                ticket: None,
                why: String::new(),
                fields: vec![SecretField { key: "vercel-token".into(), label: "Vercel token".into(), secret: true }],
                save_to: String::new(),
                at: now,
                store: Some(SecretStore {
                    name: "vercel-token".into(),
                    service: "vercel".into(),
                    kind: SecretKind::Token,
                    projects: Vec::new(),
                    create_url: Some("https://vercel.com/account/tokens".into()),
                    scopes_hint: None,
                }),
            }];
        }
    }

    /// Sample logins, a record, waits and nevers for the page, with no daemon asked.
    #[cfg(feature = "snapshot")]
    pub fn snapshot_logins_seed(&mut self, cx: &mut Context<Self>) {
        use equipe_core::protocol::HumanCheck;
        use equipe_core::protocol::vault::{CauseCount, LoginInsights, SiteAsks, Use};
        let now = now_secs();
        let used = |ago: u64, outcome: &str| Use { at: now - ago, squad: "demo".into(), member: "backend".into(), outcome: outcome.into() };
        let login = |id: u32, site: &str, user: &str| Login { id, site: site.into(), url: None, username: user.into(), note: None, allowed: Vec::new(), auto: false, stopped: None, created_at: now - 86_400 * 30, password_at: Some(now - 86_400 * 12), uses: Vec::new(), autopilot: Default::default() };
        let mut cloudflare = login(1, "dash.cloudflare.com", "abel@example.com");
        cloudflare.note = Some("DNS".into());
        cloudflare.url = Some("https://dash.cloudflare.com/login".into());
        cloudflare.allowed = vec!["demo".into()];
        cloudflare.auto = true;
        cloudflare.uses = vec![used(3_600, "signed_in_auto")];
        let mut github = login(2, "github.com", "abel-dev");
        github.allowed = vec!["*".into()];
        github.uses = vec![used(86_400, "signed_in")];
        let mut erp = login(3, "erp.example.com", "pm@example.com");
        erp.allowed = vec!["demo".into()];
        erp.auto = true;
        erp.stopped = Some("the sign-in form came back after equipe typed the password in".into());
        erp.uses = vec![used(7_200, "failed")];
        let slack = login(4, "slack.com", "team@example.com");
        self.model.logins = vec![cloudflare, github, erp, slack];
        let causes = |pairs: &[(&str, u64)]| pairs.iter().map(|(c, n)| CauseCount { cause: (*c).into(), count: *n }).collect::<Vec<_>>();
        let site = |name: &str, pairs: &[(&str, u64)], reask: Option<u64>, signed: u64, auto: u64| SiteAsks { site: name.into(), asked: pairs.iter().map(|p| p.1).sum(), by_cause: causes(pairs), reask_after: reask, signed_in: signed, signed_in_by_equipe: auto };
        self.model.login_insights = Some(LoginInsights {
            days: self.logins_ui.days,
            at: now,
            asked: 14,
            by_cause: causes(&[("session_expired", 8), ("no_login_kept", 3), ("two_factor", 2), ("captcha", 1)]),
            sites: vec![
                site("github.com", &[("session_expired", 5), ("two_factor", 2)], Some(5 * 3_600), 4, 3),
                site("erp.example.com", &[("session_expired", 3), ("no_login_kept", 1)], Some(26 * 3_600), 2, 0),
                site("notion.so", &[("no_login_kept", 2)], None, 0, 0),
                site("bank.example.com", &[("captcha", 1)], None, 0, 0),
            ],
            trend: (0..self.logins_ui.days).map(|i| [0, 2, 1, 0, 4, 3, 4][(i % 7) as usize]).collect(),
            signed_in: 6,
            signed_in_by_equipe: 3,
            never: vec!["bank.example.com".into(), "mail.example.org".into()],
            can_keep: true,
        });
        let wait = |page: u64, member: &str, site: &str| crate::state::BrowserWait { page, lane: format!("demo/{member}"), session: None, member: member.into(), site: site.into(), check: Some(HumanCheck::SignIn), why: None };
        self.model.browser_waits = vec![wait(7, "frontend", "notion.so"), wait(8, "qa", "notion.so")];
        self.show(Screen::Logins, cx);
        cx.notify();
    }

    /// The words in the page's boxes follow the language.
    pub fn logins_relocalize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ui = &self.logins_ui;
        ui.search.update(cx, |s, cx| s.set_placeholder(t!("loginspage.search_placeholder"), window, cx));
        ui.url.update(cx, |s, cx| s.set_placeholder(t!("loginspage.url_placeholder"), window, cx));
        ui.site.update(cx, |s, cx| s.set_placeholder(t!("accounts.site_placeholder"), window, cx));
        ui.username.update(cx, |s, cx| s.set_placeholder(t!("accounts.username_placeholder"), window, cx));
        ui.note.update(cx, |s, cx| s.set_placeholder(t!("accounts.note_placeholder"), window, cx));
        ui.password.update(cx, |s, cx| s.set_placeholder(t!("accounts.password_placeholder"), window, cx));
        ui.signin.update(cx, |s, cx| s.set_placeholder(t!("loginspage.signin_placeholder"), window, cx));
    }
}
