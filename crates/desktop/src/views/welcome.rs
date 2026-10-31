//! The first run: a Mac with no squad yet. Name it, pick who is on it, say how
//! much it does alone, and (if you like) give it a folder — the same four
//! questions the terminal asks on first run (`crates/cli/src/onboard.rs`), and
//! the same calls, without opening one.

use std::path::PathBuf;

use equipe_core::protocol::Mode;
use equipe_core::roles::{PM, STARTERS};
use equipe_core::{squads, t};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{BODY, FIELD, Palette, R_MD, S3, SMALL, UI};
use crate::ui::{self, Kind};
use crate::Shell;

pub struct WelcomeUi {
    pub name: Entity<InputState>,
    /// Which of `STARTERS` are on the team; the PM always is.
    pub team: [bool; STARTERS.len()],
    pub mode: Mode,
    pub folder: Option<PathBuf>,
    pub error: Option<String>,
    /// The squad is being made.
    pub busy: bool,
}

impl WelcomeUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t!("welcome.name_placeholder")));
        let mut team = [false; STARTERS.len()];
        for (on, (_, default)) in team.iter_mut().zip(STARTERS) {
            *on = default;
        }
        Self { name, team, mode: Mode::Auto, folder: None, error: None, busy: false }
    }
}

fn role_words(id: &str) -> (&'static str, &'static str) {
    match id {
        "pm" => (t!("welcome.role_pm"), t!("welcome.role_pm_desc")),
        "designer" => (t!("welcome.role_designer"), t!("welcome.role_designer_desc")),
        "frontend" => (t!("welcome.role_frontend"), t!("welcome.role_frontend_desc")),
        "backend" => (t!("welcome.role_backend"), t!("welcome.role_backend_desc")),
        "qa" => (t!("welcome.role_qa"), t!("welcome.role_qa_desc")),
        _ => (t!("welcome.role_data"), t!("welcome.role_data_desc")),
    }
}

fn label(p: Palette, text: &'static str) -> impl IntoElement {
    div().pt_3().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(text)
}

fn hint(p: Palette, text: impl Into<SharedString>) -> Div {
    div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(text.into())
}

/// A line to pick: a mark, a title, a sentence under it.
fn choice(p: Palette, id: impl Into<ElementId>, on: bool, title: String, desc: &'static str, locked: bool) -> Stateful<Div> {
    h_flex()
        .id(id)
        .gap_3()
        .items_start()
        .px(px(S3))
        .py_2()
        .rounded(px(R_MD))
        .border_1()
        .border_color(if on { p.accent } else { p.hair })
        .when(!locked, |r| r.cursor_pointer().hover(move |s| s.bg(p.wash)))
        .child(div().pt(px(3.)).size(px(18.)).flex_none().when(on, |m| m.child(icon(Icon::Check, 14., p.accent))))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(UI)).line_height(px(22.)).text_color(p.text).child(title))
                .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(desc)),
        )
}

pub fn welcome_screen(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let w = &shell.welcome;
    let lang = crate::i18n::lang();

    let team = std::iter::once(PM).chain(STARTERS.iter().map(|(id, _)| *id)).enumerate().map(|(i, id)| {
        let (title, desc) = role_words(id);
        let on = i == 0 || w.team[i - 1];
        let row = choice(p, ("role", i), on, title.to_owned(), desc, i == 0);
        if i == 0 {
            row.into_any_element()
        } else {
            row.on_click(cx.listener(move |this, _, _, cx| {
                this.welcome.team[i - 1] = !this.welcome.team[i - 1];
                cx.notify();
            }))
            .into_any_element()
        }
    });
    let modes = Mode::ALL.into_iter().map(|m| {
        let recommended = if m == Mode::Auto { format!("{} · ", t!("welcome.recommended")) } else { String::new() };
        let title = format!("{recommended}{}", equipe_core::i18n::label::mode(lang, m));
        choice(p, ("mode", m as usize), w.mode == m, title, equipe_core::i18n::label::mode_short(lang, m), false).on_click(cx.listener(move |this, _, _, cx| {
            this.welcome.mode = m;
            cx.notify();
        }))
    });
    let folder = match &w.folder {
        Some(path) => h_flex()
            .gap_2()
            .items_center()
            .child(icon(Icon::Folder, 16., p.text2))
            .child(div().flex_1().min_w_0().truncate().text_size(px(UI)).text_color(p.text).child(path.display().to_string()))
            .child(ui::button(p, "folder-clear", Kind::Ghost, t!("welcome.folder_clear")).field().on_click(cx.listener(|this, _, _, cx| {
                this.welcome.folder = None;
                cx.notify();
            })))
            .into_any_element(),
        None => h_flex()
            .gap_3()
            .items_center()
            .child(ui::button(p, "folder-pick", Kind::Secondary, t!("welcome.folder_pick")).icon(Icon::Folder).field().on_click(cx.listener(|this, _, _, cx| this.pick_welcome_folder(cx))))
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
                .child(
                    h_flex().pt_4().child(
                        ui::button(p, "create", Kind::Primary, if w.busy { t!("welcome.creating") } else { t!("welcome.create") })
                            .field()
                            .disabled(w.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.create_first_squad(window, cx))),
                    ),
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
                this.welcome.busy = false;
                match done {
                    Ok(folder_error) => {
                        this.pending_team = Some(team);
                        this.composer.flash = folder_error;
                        this.select_squad(&name, window, cx);
                    }
                    Err(why) => this.welcome.error = Some(why),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
