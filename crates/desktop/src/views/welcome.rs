//! The first run: a Mac with no squad yet. Name it, pick who is on it, say how
//! much it does alone, and (if you like) give it a folder — the same four
//! questions the terminal asks on first run (`crates/cli/src/onboard.rs`), and
//! the same calls, without opening one. It is the second step of the first run
//! (the AI is connected first: `views/setup.rs`).

use std::path::PathBuf;
use std::time::Duration;

use equipe_core::protocol::Mode;
use equipe_core::roles::{PM, STARTERS};
use equipe_core::{squads, t};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{BODY, FIELD, ICON_SM, Palette, R_MD, S3, SMALL, UI};
use crate::ui::{self, Kind};
use crate::Shell;

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
    /// Tab stops, in the order the keys visit them: the team, the modes, then the buttons.
    team_focus: [FocusHandle; STARTERS.len()],
    mode_focus: [FocusHandle; Mode::ALL.len()],
    pick_focus: FocusHandle,
    clear_focus: FocusHandle,
    create_focus: FocusHandle,
}

impl WelcomeUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t!("welcome.name_placeholder")));
        let mut team = [false; STARTERS.len()];
        for (on, (_, default)) in team.iter_mut().zip(STARTERS) {
            *on = default;
        }
        let stop = |cx: &mut Context<Shell>| cx.focus_handle().tab_stop(true);
        Self {
            name,
            team,
            mode: Mode::Auto,
            folder: None,
            error: None,
            failed: None,
            busy: false,
            team_focus: std::array::from_fn(|_| stop(cx)),
            mode_focus: std::array::from_fn(|_| stop(cx)),
            pick_focus: stop(cx),
            clear_focus: stop(cx),
            create_focus: stop(cx),
        }
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
        _ => (t!("welcome.role_data"), t!("welcome.role_data_desc")),
    }
}

fn label(p: Palette, text: &'static str) -> impl IntoElement {
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

/// How long the squad may take to be picked up before the form says so.
const JOIN_WAIT: Duration = Duration::from_secs(25);

pub fn welcome_screen(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let w = &shell.welcome;
    let lang = crate::i18n::lang();

    let team = std::iter::once(PM).chain(STARTERS.iter().map(|(id, _)| *id)).enumerate().map(|(i, id)| {
        let (title, desc) = role_words(id);
        let on = i == 0 || w.team[i - 1];
        if i == 0 {
            return choice(p, ("role", i), false, on, title.to_owned(), desc, None).into_any_element();
        }
        let flip = move |this: &mut Shell, _: &mut Window, cx: &mut Context<Shell>| {
            this.welcome.team[i - 1] = !this.welcome.team[i - 1];
            cx.notify();
        };
        let focus = w.team_focus[i - 1].clone();
        choice(p, ("role", i), false, on, title.to_owned(), desc, Some(&focus))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.welcome.team_focus[i - 1].clone().focus(window, cx);
                flip(this, window, cx);
            }))
            .on_key_down(keys(flip, cx))
            .into_any_element()
    }).collect::<Vec<_>>();
    let modes = Mode::ALL.into_iter().map(|m| {
        let recommended = if m == Mode::Auto { format!("{} · ", t!("welcome.recommended")) } else { String::new() };
        let title = format!("{recommended}{}", equipe_core::i18n::label::mode(lang, m));
        let pick = move |this: &mut Shell, _: &mut Window, cx: &mut Context<Shell>| {
            this.welcome.mode = m;
            cx.notify();
        };
        let ix = Mode::ALL.iter().position(|x| *x == m).unwrap_or(0);
        let focus = w.mode_focus[ix].clone();
        choice(p, ("mode", ix), true, w.mode == m, title, equipe_core::i18n::label::mode_short(lang, m), Some(&focus))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.welcome.mode_focus[ix].clone().focus(window, cx);
                pick(this, window, cx);
            }))
            .on_key_down(keys(pick, cx))
    }).collect::<Vec<_>>();
    let folder = match &w.folder {
        Some(path) => h_flex()
            .gap_2()
            .items_center()
            .child(icon(Icon::Folder, 16., p.text2))
            .child(div().flex_1().min_w_0().truncate().text_size(px(UI)).text_color(p.text).child(path.display().to_string()))
            .child(ui::button(p, "folder-clear", Kind::Ghost, t!("welcome.folder_clear")).field().focus(&w.clear_focus).on_click(cx.listener(|this, _, _, cx| {
                this.welcome.folder = None;
                cx.notify();
            })))
            .into_any_element(),
        None => h_flex()
            .gap_3()
            .items_center()
            .child(ui::button(p, "folder-pick", Kind::Secondary, t!("welcome.folder_pick")).icon(Icon::Folder).field().focus(&w.pick_focus).on_click(cx.listener(|this, _, _, cx| this.pick_welcome_folder(cx))))
            .child(div().flex_1().min_w_0().child(hint(p, t!("welcome.folder_none"))))
            .into_any_element(),
    };

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
                .child(label(p, t!("welcome.name_label")))
                .child(
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
                        .child(Input::new(&w.name).appearance(false).bordered(false)),
                )
                .child(hint(p, w.error.clone().unwrap_or_else(|| t!("welcome.name_hint").to_owned())).when(w.error.is_some(), |h| h.text_color(p.danger())))
                .child(label(p, t!("welcome.team_label")))
                .child(v_flex().gap_2().children(team))
                .child(label(p, t!("welcome.mode_label")))
                .child(hint(p, t!("welcome.mode_hint")))
                .child(v_flex().gap_2().children(modes))
                .child(label(p, t!("welcome.folder_label")))
                .child(folder)
                .children(w.failed.clone().map(|why| {
                    h_flex().pt_3().gap_2().items_start().text_size(px(SMALL)).line_height(px(20.)).text_color(p.danger()).child(div().flex_none().pt(px(3.)).child(icon(Icon::Warning, ICON_SM, p.danger()))).child(div().flex_1().min_w_0().child(why))
                }))
                .child(
                    h_flex().pt_4().gap_3().items_center().child(
                        ui::button(p, "create", Kind::Primary, if w.busy { t!("welcome.creating") } else { t!("welcome.create") })
                            .field()
                            .focus(&w.create_focus)
                            .disabled(w.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.create_first_squad(window, cx))),
                    ).when(w.busy, |r| r.child(motion::spinner(14., p.text2))),
                ),
            "welcome-in",
        ))
        .into_any_element()
}

impl Shell {
    /// The system's folder picker; the folder is added when the squad is made.
    fn pick_welcome_folder(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(t!("welcome.folder_prompt").into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| {
                    this.welcome.folder = Some(path);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// The first squad, made where the terminal's first run makes it, then
    /// followed: its team joins as soon as the window is attached.
    fn create_first_squad(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.welcome.busy {
            return;
        }
        let home = equipe_providers::credentials::equipe_home();
        let typed = self.welcome.name.read(cx).value().trim().to_owned();
        let name = if typed.is_empty() { squads::suggest_name(&home, std::path::Path::new(&std::env::var("USER").unwrap_or_default())) } else { typed };
        if !squads::valid_name(&name) {
            self.welcome.error = Some(t!("welcome.name_rule").to_owned());
            cx.notify();
            return;
        }
        let mut team: Vec<String> = vec![PM.to_owned()];
        team.extend(STARTERS.iter().zip(self.welcome.team).filter(|(_, on)| *on).map(|((id, _), _)| (*id).to_owned()));
        let (mode, folder) = (self.welcome.mode, self.welcome.folder.clone());
        self.welcome.error = None;
        self.welcome.failed = None;
        self.welcome.busy = true;
        cx.notify();
        let made = name.clone();
        let task = cx.background_executor().spawn(async move {
            squads::create_without_project(&home, &made)?;
            equipe_core::journal::Journal::for_squad(&home, &made).and_then(|j| j.set_mode(mode)).map_err(|e| e.to_string())?;
            // A folder that won't take doesn't undo the squad: it opens, and says so.
            let folder_error = folder.and_then(|f| squads::add_folder(&home, &made, &f.display().to_string(), std::env::var_os("HOME").map(PathBuf::from).as_deref()).err());
            Ok::<_, String>(folder_error)
        });
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
