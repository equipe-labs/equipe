//! The first run: a Mac with no squad yet. Name it, pick who is on it, say how
//! much it does alone, and (if you like) give it a folder — the same four
//! questions the terminal asks on first run (`crates/cli/src/onboard.rs`), and
//! the same calls, without opening one. It is the second step of the first run
//! (the AI is connected first: `views/setup.rs`).
//!
//! The four questions are drawn here once (`fields`), for either form: this
//! page, and the "새 스쿼드…" sheet over the window (`views/new_squad.rs`). Their
//! rules are `crate::squad_form`.

use std::path::PathBuf;
use std::time::Duration;

use equipe_core::protocol::Mode;
use equipe_core::roles::{PM, STARTERS};
use equipe_core::{squads, t};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::squad_form::{self, NameProblem, Route};
use crate::theme::{BODY, FIELD, ICON_SM, Palette, R_MD, S3, SMALL, UI};
use crate::ui::{self, Kind};
use crate::Shell;

/// Which of the two forms: the first run's page, or the sheet over the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    First,
    New,
}

pub struct WelcomeUi {
    pub name: Entity<InputState>,
    /// Which of `STARTERS` are on the team; the PM always is.
    pub team: [bool; STARTERS.len()],
    pub mode: Mode,
    pub folder: Option<PathBuf>,
    /// The name isn't allowed.
    pub error: Option<String>,
    /// Making the squad (or joining it) didn't go through.
    pub failed: Option<String>,
    /// The squad is being made, then joined.
    pub busy: bool,
    /// The sheet is over the window (the "새 스쿼드…" form only).
    pub open: bool,
    /// Tab stops, in the order the keys visit them: the team, the modes, then the buttons.
    team_focus: [FocusHandle; STARTERS.len()],
    mode_focus: [FocusHandle; Mode::ALL.len()],
    pick_focus: FocusHandle,
    clear_focus: FocusHandle,
    pub cancel_focus: FocusHandle,
    create_focus: FocusHandle,
}

impl WelcomeUi {
    pub fn new(which: Which, window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t!("welcome.name_placeholder")));
        // ⏎ in the name makes the squad; typing again takes the refusal away.
        cx.subscribe_in(&name, window, move |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { .. } => this.create_squad(which, window, cx),
            InputEvent::Change if this.form(which).error.is_some() => {
                this.form_mut(which).error = None;
                cx.notify();
            }
            _ => {}
        })
        .detach();
        let stop = |cx: &mut Context<Shell>| cx.focus_handle().tab_stop(true);
        Self {
            name,
            team: squad_form::default_team(),
            mode: squad_form::DEFAULT_MODE,
            folder: None,
            error: None,
            failed: None,
            busy: false,
            open: false,
            team_focus: std::array::from_fn(|_| stop(cx)),
            mode_focus: std::array::from_fn(|_| stop(cx)),
            pick_focus: stop(cx),
            clear_focus: stop(cx),
            cancel_focus: stop(cx),
            create_focus: stop(cx),
        }
    }

    /// As it was before anyone answered: an empty name, the default team and mode, no folder.
    pub fn reset(&mut self, window: &mut Window, cx: &mut App) {
        self.name.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        (self.team, self.mode, self.folder) = (squad_form::default_team(), squad_form::DEFAULT_MODE, None);
        (self.error, self.failed, self.busy) = (None, None, false);
    }

    /// Where Tab goes, in order: the name, the team, the modes, the folder's
    /// button (when this Mac's picker gives one), then Cancel (the sheet's) and Create.
    pub fn stops(&self, which: Which, route: Route, cx: &App) -> Vec<FocusHandle> {
        let mut stops = vec![self.name.read(cx).focus_handle(cx)];
        stops.extend(self.team_focus.iter().cloned());
        stops.extend(self.mode_focus.iter().cloned());
        if route.picks_folder() {
            stops.push(if self.folder.is_some() { self.clear_focus.clone() } else { self.pick_focus.clone() });
        }
        if which == Which::New {
            stops.push(self.cancel_focus.clone());
        }
        stops.push(self.create_focus.clone());
        stops
    }

    pub fn create_focus(&self) -> &FocusHandle {
        &self.create_focus
    }
}

/// What the form says under the name when it can't be the squad's.
pub fn name_problem(problem: NameProblem, typed: &str) -> String {
    match problem {
        NameProblem::Empty => t!("welcome.name_empty").to_owned(),
        NameProblem::Rule => t!("welcome.name_rule").to_owned(),
        NameProblem::Taken => t!("welcome.name_taken", name = typed.trim()),
    }
}

fn role_words(id: &str) -> (&'static str, &'static str) {
    match id {
        "pm" => (t!("welcome.role_pm"), t!("welcome.role_pm_desc")),
        "designer" => (t!("welcome.role_designer"), t!("welcome.role_designer_desc")),
        "frontend" => (t!("welcome.role_frontend"), t!("welcome.role_frontend_desc")),
        "backend" => (t!("welcome.role_backend"), t!("welcome.role_backend_desc")),
        "qa" => (t!("welcome.role_qa"), t!("welcome.role_qa_desc")),
        "security" => (t!("welcome.role_security"), t!("welcome.role_security_desc")),
        "marketing" => (t!("welcome.role_marketing"), t!("welcome.role_marketing_desc")),
        _ => (t!("welcome.role_data"), t!("welcome.role_data_desc")),
    }
}

fn label(p: Palette, text: &'static str) -> Div {
    div().pt_3().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(text)
}

fn hint(p: Palette, text: impl Into<SharedString>) -> Div {
    div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(text.into())
}

/// What a line to pick shows before its words: a square for one of several,
/// a circle for the one of a few — filled when chosen, drawn empty when not.
fn mark(p: Palette, round: bool, on: bool) -> Div {
    let b = div().size(px(18.)).flex_none().flex().items_center().justify_center().border_1().border_color(if on { p.accent } else { p.muted });
    if round {
        b.rounded_full().when(on, |m| m.child(div().size(px(8.)).rounded_full().bg(p.accent)))
    } else {
        b.rounded(px(4.)).when(on, |m| m.bg(p.accent).child(icon(Icon::Check, 12., p.on_primary)))
    }
}

/// A line to pick: a mark, a title, a sentence under it. Reached by Tab
/// (`focus`) and pressed with Space or Return, as a button is.
fn choice(p: Palette, id: impl Into<ElementId>, round: bool, on: bool, title: String, desc: &'static str, focus: Option<&FocusHandle>) -> Stateful<Div> {
    let ring = Rgba { a: 0.55, ..p.accent };
    h_flex()
        .id(id)
        .gap_3()
        .items_start()
        .px(px(S3))
        .py_2()
        .rounded(px(R_MD))
        .border_1()
        .border_color(if on { p.accent } else { p.hair })
        .when(focus.is_some(), |r| r.cursor_pointer().hover(move |s| s.bg(p.wash)))
        .when_some(focus, |r, f| r.track_focus(f).focus_visible(move |s| s.shadow(vec![BoxShadow { color: ring.into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(2.), inset: false }])))
        .child(div().pt(px(2.)).child(mark(p, round, on)))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(UI)).line_height(px(22.)).text_color(p.text).child(title))
                .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(desc)),
        )
}

/// A role as a chip — the same mark, the title alone, its sentence as the
/// tooltip — for the sheet, where a line each would run past the window.
fn chip(p: Palette, id: impl Into<ElementId>, on: bool, title: &'static str, desc: &'static str, focus: Option<&FocusHandle>) -> Stateful<Div> {
    use crate::tips::TipExt as _;
    let ring = Rgba { a: 0.55, ..p.accent };
    h_flex()
        .id(id)
        .h(px(32.))
        .px(px(10.))
        .gap_2()
        .items_center()
        .rounded(px(R_MD))
        .border_1()
        .border_color(if on { p.accent } else { p.hair })
        .when(focus.is_some(), |r| r.cursor_pointer().hover(move |s| s.bg(p.wash)))
        .when_some(focus, |r, f| r.track_focus(f).focus_visible(move |s| s.shadow(vec![BoxShadow { color: ring.into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(2.), inset: false }])))
        .child(mark(p, false, on))
        .child(div().text_size(px(SMALL)).text_color(p.text).whitespace_nowrap().child(title))
        .tip(p, desc, None)
}

/// Space or Return does `act`; Tab moves on, Shift-Tab back.
fn keys(act: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static, cx: &mut Context<Shell>) -> impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static {
    let shell = cx.entity().downgrade();
    move |event, window, cx| {
        let k = &event.keystroke;
        match k.key.as_str() {
            "space" | "enter" if !k.modifiers.modified() => {
                shell.update(cx, |this, cx| act(this, window, cx)).ok();
            }
            "tab" if k.modifiers.shift => window.focus_prev(cx),
            "tab" => window.focus_next(cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

/// Choices side by side, two to a row, each as tall as the taller — for the
/// sheet, where one to a line would run past the window.
fn pairs(items: Vec<AnyElement>) -> Div {
    let mut items = items.into_iter();
    let mut rows = Vec::new();
    while let Some(first) = items.next() {
        let second = items.next().unwrap_or_else(|| div().into_any_element());
        rows.push(div().flex().flex_row().gap_2().child(div().flex().flex_1().min_w_0().child(first)).child(div().flex().flex_1().min_w_0().child(second)));
    }
    v_flex().gap_2().children(rows)
}

/// The four questions — the name, the team, the mode, the folder — and why the
/// last try didn't go through, for either form. `route` says whether this
/// Mac's folder picker can give the folder (another machine's can't).
pub fn fields(shell: &Shell, which: Which, route: Route, cx: &mut Context<Shell>) -> Vec<AnyElement> {
    let p = shell.palette;
    let w = shell.form(which);
    let lang = crate::i18n::lang();
    let side_by_side = which == Which::New;
    let fill = |c: Stateful<Div>| if side_by_side { c.flex_1().min_w_0() } else { c };
    // The sheet is shorter than the page: less room above each question.
    let label = |text: &'static str| if side_by_side { label(p, text).pt_1() } else { label(p, text) };

    let team = std::iter::once(PM).chain(STARTERS.iter().map(|(id, _)| *id)).enumerate().map(|(i, id)| {
        let (title, desc) = role_words(id);
        let on = i == 0 || w.team[i - 1];
        // The sheet: a chip a role, its sentence under the pointer.
        let line = |focus: Option<&FocusHandle>| if side_by_side { chip(p, ("role", i), on, title, desc, focus) } else { choice(p, ("role", i), false, on, title.to_owned(), desc, focus) };
        if i == 0 {
            return line(None).into_any_element();
        }
        let flip = move |this: &mut Shell, _: &mut Window, cx: &mut Context<Shell>| {
            let form = this.form_mut(which);
            form.team[i - 1] = !form.team[i - 1];
            cx.notify();
        };
        let focus = w.team_focus[i - 1].clone();
        line(Some(&focus))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.form(which).team_focus[i - 1].clone().focus(window, cx);
                flip(this, window, cx);
            }))
            .on_key_down(keys(flip, cx))
            .into_any_element()
    }).collect::<Vec<_>>();
    let modes = Mode::ALL.into_iter().map(|m| {
        let recommended = if m == squad_form::DEFAULT_MODE { format!("{} · ", t!("welcome.recommended")) } else { String::new() };
        let title = format!("{recommended}{}", equipe_core::i18n::label::mode(lang, m));
        let pick = move |this: &mut Shell, _: &mut Window, cx: &mut Context<Shell>| {
            this.form_mut(which).mode = m;
            cx.notify();
        };
        let ix = Mode::ALL.iter().position(|x| *x == m).unwrap_or(0);
        let focus = w.mode_focus[ix].clone();
        fill(choice(p, ("mode", ix), true, w.mode == m, title, equipe_core::i18n::label::mode_short(lang, m), Some(&focus)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.form(which).mode_focus[ix].clone().focus(window, cx);
                pick(this, window, cx);
            }))
            .on_key_down(keys(pick, cx))
            .into_any_element()
    }).collect::<Vec<_>>();
    let folder = match (&w.folder, route.picks_folder()) {
        // Another machine's folder isn't this Mac's to pick: it is added there, once the squad is open.
        (_, false) => h_flex()
            .gap_2()
            .items_start()
            .child(div().flex_none().pt(px(3.)).child(icon(Icon::Folder, ICON_SM, p.muted)))
            .child(div().flex_1().min_w_0().child(hint(p, t!("new_squad.folder_there", host = shell.endpoint.label()))))
            .into_any_element(),
        (Some(path), true) => h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(icon(Icon::Folder, 16., p.text2))
            .child(div().flex_1().min_w_0().truncate().text_size(px(UI)).text_color(p.text).child(path.display().to_string()))
            .child(ui::button(p, "folder-clear", Kind::Ghost, t!("welcome.folder_clear")).field().focus(&w.clear_focus).on_click(cx.listener(move |this, _, window, cx| {
                this.form_mut(which).folder = None;
                this.form(which).pick_focus.clone().focus(window, cx);
                cx.notify();
            })))
            .into_any_element(),
        (None, true) => h_flex()
            .gap_3()
            .items_center()
            .child(ui::button(p, "folder-pick", Kind::Secondary, t!("welcome.folder_pick")).icon(Icon::Folder).field().focus(&w.pick_focus).on_click(cx.listener(move |this, _, _, cx| this.pick_squad_folder(which, cx))))
            // The sheet: the question on the button's line, which says "optional" already.
            .when(!side_by_side, |r| r.child(div().flex_1().min_w_0().child(hint(p, t!("welcome.folder_none")))))
            .into_any_element(),
    };
    let folder = if side_by_side && route.picks_folder() {
        h_flex().pt_1().gap_3().items_center().child(label(t!("welcome.folder_label")).pt_0().flex_none()).child(div().flex_1().min_w_0().flex().justify_end().child(folder)).into_any_element()
    } else {
        v_flex().gap_2().child(label(t!("welcome.folder_label"))).child(folder).into_any_element()
    };
    let team = if side_by_side { h_flex().flex_wrap().gap_2().children(team) } else { v_flex().gap_2().children(team) };
    let modes = if side_by_side { pairs(modes) } else { v_flex().gap_2().children(modes) };

    vec![
        label(t!("welcome.name_label")).into_any_element(),
        h_flex()
            .w_full()
            .h(px(FIELD))
            .pl(px(S3) - gpui_kit::component::Size::Medium.input_px())
            .items_center()
            .rounded(px(R_MD))
            .bg(p.code_bg)
            .border_1()
            .border_color(if w.error.is_some() { p.danger() } else { p.hair })
            .text_size(px(UI))
            .text_color(p.text)
            .child(Input::new(&w.name).appearance(false).bordered(false))
            .into_any_element(),
        hint(p, w.error.clone().unwrap_or_else(|| t!("welcome.name_hint").to_owned())).when(w.error.is_some(), |h| h.text_color(p.danger())).into_any_element(),
        label(t!("welcome.team_label")).into_any_element(),
        team.into_any_element(),
        // The sheet: the hint beside the question, not under it.
        if side_by_side {
            h_flex().gap_2().items_end().child(label(t!("welcome.mode_label"))).child(hint(p, t!("welcome.mode_hint")).text_size(px(12.))).into_any_element()
        } else {
            v_flex().gap_2().child(label(t!("welcome.mode_label"))).child(hint(p, t!("welcome.mode_hint"))).into_any_element()
        },
        modes.into_any_element(),
        folder,
    ]
    .into_iter()
    // The sheet says it beside its buttons, where it is in sight (`failed`).
    .chain(w.failed.clone().filter(|_| !side_by_side).map(|why| failed(p, why).pt_3().into_any_element()))
    .collect()
}

/// Why the last try didn't go through, in the danger colour after a warning mark.
pub fn failed(p: Palette, why: String) -> Div {
    h_flex().gap_2().items_start().text_size(px(SMALL)).line_height(px(20.)).text_color(p.danger()).child(div().flex_none().pt(px(3.)).child(icon(Icon::Warning, ICON_SM, p.danger()))).child(div().flex_1().min_w_0().child(why))
}

/// The form's main button: 스쿼드 만들기, or 만드는 중… while it is.
pub fn create_button(shell: &Shell, which: Which, cx: &mut Context<Shell>) -> ui::Button {
    let w = shell.form(which);
    ui::button(shell.palette, "create", Kind::Primary, if w.busy { t!("welcome.creating") } else { t!("welcome.create") })
        .field()
        .focus(&w.create_focus)
        .disabled(w.busy)
        .on_click(cx.listener(move |this, _, window, cx| this.create_squad(which, window, cx)))
}

/// How long the squad may take to be picked up before the form says so.
const JOIN_WAIT: Duration = Duration::from_secs(25);

pub fn welcome_screen(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let w = &shell.welcome;

    v_flex()
        .id("welcome")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .items_center()
        .py_8()
        .child(motion::rise(
            v_flex()
                .w_full()
                .max_w(px(560.))
                .px_4()
                .gap_2()
                .child(h_flex().child(ui::button(p, "welcome-back", Kind::Ghost, t!("welcome.back")).icon(Icon::ArrowLeft).disabled(w.busy).on_click(cx.listener(|this, _, _, cx| {
                    this.setup.squad_step = false;
                    cx.notify();
                }))))
                .child(div().text_size(px(22.)).line_height(px(30.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("welcome.title")))
                .child(div().text_size(px(BODY)).line_height(px(24.)).text_color(p.text2).child(t!("welcome.text")))
                .children(fields(shell, Which::First, Route::Here, cx))
                .child(h_flex().pt_4().gap_3().items_center().child(create_button(shell, Which::First, cx)).when(w.busy, |r| r.child(motion::spinner(14., p.text2)))),
            "welcome-in",
        ))
        .into_any_element()
}

impl Shell {
    pub fn form(&self, which: Which) -> &WelcomeUi {
        match which {
            Which::First => &self.welcome,
            Which::New => &self.new_squad,
        }
    }

    pub fn form_mut(&mut self, which: Which) -> &mut WelcomeUi {
        match which {
            Which::First => &mut self.welcome,
            Which::New => &mut self.new_squad,
        }
    }

    /// The system's folder picker; the folder is added when the squad is made.
    fn pick_squad_folder(&mut self, which: Which, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(t!("welcome.folder_prompt").into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| {
                    this.form_mut(which).folder = Some(path);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// The form's Create: the first squad, or one more (`views/new_squad.rs`).
    pub fn create_squad(&mut self, which: Which, window: &mut Window, cx: &mut Context<Self>) {
        match which {
            Which::First => self.create_first_squad(window, cx),
            Which::New => self.create_new_squad(window, cx),
        }
    }

    /// The first squad, made where the terminal's first run makes it, then
    /// followed: its team joins as soon as the window is attached.
    fn create_first_squad(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.welcome.busy || crate::readonly::blocked("a new squad") {
            return;
        }
        let home = equipe_providers::credentials::equipe_home();
        let typed = self.welcome.name.read(cx).value().to_string();
        let taken: Vec<String> = squads::list(&home).into_iter().map(|s| s.name).collect();
        let suggested = squads::suggest_name(&home, std::path::Path::new(&std::env::var("USER").unwrap_or_default()));
        let name = match squad_form::check_name(&typed, &taken, Some(suggested)) {
            Ok(name) => name,
            Err(problem) => {
                self.welcome.error = Some(name_problem(problem, &typed));
                cx.notify();
                return;
            }
        };
        let team = squad_form::team(&self.welcome.team);
        let (mode, folder) = (self.welcome.mode, self.welcome.folder.clone());
        self.welcome.error = None;
        self.welcome.failed = None;
        self.welcome.busy = true;
        cx.notify();
        let made = name.clone();
        let user_home = std::env::var_os("HOME").map(PathBuf::from);
        let task = cx.background_executor().spawn(async move { squad_form::make_here(&home, &made, mode, folder, user_home.as_deref()) });
        cx.spawn_in(window, async move |this, cx| {
            let done = task.await;
            this.update_in(cx, |this, window, cx| {
                match done {
                    // Stays "busy" until the board takes the form's place.
                    Ok(folder_error) => {
                        this.pending_team = Some(team);
                        this.composer.flash = folder_error;
                        this.select_squad(&name, window, cx);
                        cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(JOIN_WAIT).await;
                            this.update(cx, |this, cx| {
                                if this.welcome.busy {
                                    this.welcome.busy = false;
                                    this.welcome.failed = Some(t!("welcome.failed_join").to_owned());
                                    cx.notify();
                                }
                            })
                            .ok();
                        })
                        .detach();
                    }
                    Err(why) => {
                        this.welcome.busy = false;
                        this.welcome.failed = Some(format!("{}\n{why}", t!("welcome.failed_create")));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
