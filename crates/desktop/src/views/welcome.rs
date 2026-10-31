//! The first run on this Mac, drawn: no squad yet, so the window starts one
//! (`welcome.rs`) — git, a project folder, a name — instead of sending the
//! person to a terminal.

use equipe_core::t;
use std::path::PathBuf;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{BODY, CONTROL, R_MD, R_XL, S3, SMALL};
use crate::ui::{self, Kind};
use crate::welcome::{GIT_POLL, Welcome};
use crate::Shell;

/// The first-run page's state: the model and the name's field.
pub struct WelcomeUi {
    pub state: Welcome,
    pub name: Entity<InputState>,
    /// The name last put in the field: once the person types another, a
    /// picked folder no longer renames it.
    pub suggested: String,
    pub _watch: Option<Task<()>>,
}

impl WelcomeUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t!("welcome.name_placeholder")));
        Self { state: Welcome::default(), name, suggested: String::new(), _watch: None }
    }
}

/// One step's row: its number or ✓, its title and words, and what it does.
fn step(shell: &Shell, n: usize, done: bool, title: String, text: String, control: Option<AnyElement>, below: Option<AnyElement>) -> impl IntoElement {
    let p = shell.palette;
    let mark = div()
        .size(px(24.))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .when(done, |d| d.bg(p.accent_bg).child(icon(Icon::Check, 14., p.accent)))
        .when(!done, |d| d.border_1().border_color(p.hair).text_color(p.muted).child(n.to_string()));
    h_flex()
        .w_full()
        .gap_3()
        .items_start()
        .py(px(S3))
        .child(mark)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_size(px(BODY)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title))
                .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(text))
                .children(below),
        )
        .children(control.map(|c| div().flex_none().child(c)))
}

pub fn welcome_screen(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let w = &shell.welcome.state;
    let git = w.git == Some(true);
    let name = shell.welcome.name.read(cx).value().to_string();

    // 1. git, with Apple's developer tools.
    let git_control: Option<AnyElement> = match w.git {
        Some(true) => None,
        None => Some(motion::spinner(14., p.text2)),
        Some(false) if w.git_asked => Some(h_flex().gap_2().items_center().child(motion::spinner(14., p.text2)).child(div().text_size(px(SMALL)).text_color(p.text2).child(t!("welcome.git_installing"))).into_any_element()),
        Some(false) => Some(ui::button(p, "welcome-git", Kind::Primary, t!("welcome.git_install")).icon(Icon::ArrowDown).on_click(cx.listener(|this, _, _, cx| this.welcome_install_git(cx))).into_any_element()),
    };
    let git_text = match (w.git, w.git_asked) {
        (Some(true), _) => t!("welcome.git_ready"),
        (Some(false), true) => t!("welcome.git_asked"),
        _ => t!("welcome.git_text"),
    };

    // 2. The project: a folder of the person's, or none.
    let folder_text = match &w.folder {
        Some(folder) => folder.display().to_string(),
        None => t!("welcome.folder_text").to_owned(),
    };
    let folder_control = h_flex()
        .gap_2()
        .when(w.folder.is_some(), |r| {
            r.child(ui::button(p, "welcome-folder-clear", Kind::Ghost, t!("welcome.folder_clear")).on_click(cx.listener(|this, _, window, cx| this.welcome_folder(None, window, cx))))
        })
        .child(ui::button(p, "welcome-folder", Kind::Secondary, t!("welcome.folder_pick")).icon(Icon::Folder).on_click(cx.listener(|this, _, _, cx| this.welcome_pick_folder(cx))))
        .into_any_element();

    // 3. Its name, then 시작.
    let field = div()
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
        .child(Input::new(&shell.welcome.name).appearance(false).bordered(false));
    let start = ui::button(p, "welcome-start", Kind::Primary, if w.creating { t!("welcome.starting") } else { t!("welcome.start") })
        .field()
        .disabled(!w.can_start(&name))
        .on_click(cx.listener(|this, _, window, cx| this.welcome_start(window, cx)));

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
                .child(
                    v_flex()
                        .mt_3()
                        .px(px(S3 + 4.))
                        .py_1()
                        .rounded(px(R_XL))
                        .border_1()
                        .border_color(p.hair)
                        .bg(p.raised)
                        .child(step(shell, 1, git, t!("welcome.git_title").to_owned(), git_text.to_owned(), git_control, None))
                        .child(div().h(px(1.)).bg(p.hair))
                        .child(step(shell, 2, w.folder.is_some(), t!("welcome.folder_title").to_owned(), folder_text, Some(folder_control), None))
                        .child(div().h(px(1.)).bg(p.hair))
                        .child(step(shell, 3, equipe_core::squads::valid_name(name.trim()), t!("welcome.name_title").to_owned(), t!("welcome.name_text").to_owned(), None, Some(field.into_any_element()))),
                )
                .children(w.problem.clone().map(|why| {
                    h_flex()
                        .gap_2()
                        .items_start()
                        .p(px(S3))
                        .rounded(px(R_MD))
                        .bg(p.wash)
                        .text_size(px(SMALL))
                        .line_height(px(20.))
                        .text_color(p.danger())
                        .child(div().pt(px(2.)).flex_none().child(icon(Icon::Alert, 14., p.danger())))
                        .child(div().flex_1().min_w_0().child(why))
                }))
                .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(t!("welcome.next")))
                .child(
                    h_flex()
                        .gap_2()
                        .pt_2()
                        .child(start)
                        .child(ui::button(p, "switch", Kind::Ghost, t!("connect.switch_target")).on_click(cx.listener(|this, _, window, cx| this.open_chooser(window, cx)))),
                ),
            "welcome-in",
        ))
        .into_any_element()
}

// ── what the page does ──

impl Shell {
    /// The page shows: git looked at (and watched while macOS installs it),
    /// and the name field given its first name.
    pub fn welcome_shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.welcome.state.git.is_none() {
            self.welcome.state.git = Some(crate::install::git_ready(std::env::var_os("PATH").as_deref()));
            self.welcome_rename(window, cx);
        }
    }

    /// The name field follows the folder until the person types one.
    fn welcome_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.welcome.name.read(cx).value().to_string();
        if !typed.is_empty() && typed != self.welcome.suggested {
            return;
        }
        let user = std::env::var("USER").ok();
        let name = self.welcome.state.suggestion(&equipe_providers::credentials::equipe_home(), user.as_deref());
        self.welcome.suggested = name.clone();
        self.welcome.name.update(cx, |s, cx| s.set_value(name, window, cx));
    }

    pub fn welcome_install_git(&mut self, cx: &mut Context<Self>) {
        if !crate::install::install_git() {
            self.welcome.state.problem = Some(t!("welcome.git_failed").to_owned());
            return cx.notify();
        }
        self.welcome.state.git_asked = true;
        self.welcome._watch = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(GIT_POLL).await;
                let ready = cx.background_executor().spawn(async { crate::install::git_ready(std::env::var_os("PATH").as_deref()) }).await;
                let Ok(()) = this.update(cx, |this, cx| {
                    if ready {
                        this.welcome.state.git = Some(true);
                        this.welcome.state.git_asked = false;
                        cx.notify();
                    }
                }) else {
                    return;
                };
                if ready {
                    return;
                }
            }
        }));
        cx.notify();
    }

    pub fn welcome_pick_folder(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(t!("welcome.folder_prompt").into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.into_iter().next()
            {
                cx.update(|cx| {
                    if let Some(window) = cx.active_window() {
                        let _ = window.update(cx, |_, window, cx| {
                            let _ = this.update(cx, |this, cx| this.welcome_folder(Some(path), window, cx));
                        });
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    pub fn welcome_folder(&mut self, folder: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        self.welcome.state.folder = folder;
        self.welcome_rename(window, cx);
        cx.notify();
    }

    /// 시작: `equipe squad new`, then the new squad opens with its team.
    pub fn welcome_start(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.welcome.name.read(cx).value().to_string();
        let say = t!("welcome.made", name = typed.trim()).to_owned();
        if let Some((job, after)) = self.welcome.state.start(&typed, say) {
            self.run_job(job, after, cx);
        }
        cx.notify();
    }
}
