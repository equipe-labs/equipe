//! The first run, before any squad: connect an AI and start the first squad,
//! all from the window (`crate::setup` has the rules). It stands in for the
//! "no squad yet — run `equipe` in a terminal" guide on this Mac.

use std::time::Duration;

use equipe_core::protocol::Engine;
use equipe_core::t;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::engines::mark;
use crate::engines::platform;
use crate::icons::{Icon, icon};
use crate::setup::{self, Claude, Doing};
use crate::theme::{BODY, ICON_SM, MONO, Palette, R_LG, R_MD, SMALL};
use crate::ui::{self, Kind};
use crate::{Shell, motion};

/// How long the squad may take to be picked up before the screen says so.
const JOIN_WAIT: Duration = Duration::from_secs(25);

impl Shell {
    /// Whether the window is on this Mac with no squad to open: the first run.
    pub fn needs_setup(&self) -> bool {
        !self.endpoint.is_remote() && matches!(&self.model.conn, crate::state::Conn::Down { reason: crate::failure::Failure::NoSquads, had_snapshot: false, .. })
    }

    /// Look at where the engines stand, off the UI thread.
    pub fn setup_probe(&mut self, cx: &mut Context<Self>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(setup::probe());
        });
        cx.spawn(async move |this, cx| {
            if let Ok((claude, codex)) = rx.await {
                let _ = this.update(cx, |this, cx| {
                    this.setup.claude = Some(claude);
                    this.setup.codex = Some(codex);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Start one of the steps; when it ends, look again (or join the squad).
    pub fn setup_do(&mut self, doing: Doing, cx: &mut Context<Self>) {
        if self.setup.doing != Doing::Idle {
            return;
        }
        self.setup.doing = doing;
        self.setup.failed = None;
        self.setup.cancelled = false;
        self.setup.confirm_install = false;
        // The squad opens with its team (the PM, then the starters), as one made from the terminal does.
        if doing == Doing::Creating {
            self.pending_team = Some(equipe_core::roles::starting_team());
        }
        let pid = self.setup.pid.clone();
        let squad = setup::suggest_squad();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(setup::job(doing, &squad, &pid));
        });
        cx.spawn(async move |this, cx| {
            let done = rx.await.unwrap_or_else(|_| Err("stopped".into()));
            let _ = this.update(cx, |this, cx| this.setup_done(doing, done, cx));
        })
        .detach();
        cx.notify();
    }

    fn setup_done(&mut self, doing: Doing, done: Result<(), String>, cx: &mut Context<Self>) {
        let cancelled = std::mem::take(&mut self.setup.cancelled);
        self.setup.doing = Doing::Idle;
        match done {
            Ok(()) if doing == Doing::Creating => {
                self.setup.doing = Doing::Joining;
                self.retry(cx);
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(JOIN_WAIT).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.setup.doing == Doing::Joining {
                            this.setup.doing = Doing::Idle;
                            this.setup.failed = Some((Doing::Joining, String::new()));
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            Ok(()) => self.setup_probe(cx),
            // The person ended it themselves: not a failure.
            Err(_) if cancelled => self.setup_probe(cx),
            Err(why) => {
                self.setup.failed = Some((doing, why));
                // A login may still have gone through before it complained.
                self.setup_probe(cx);
            }
        }
        cx.notify();
    }

    pub fn setup_cancel(&mut self, cx: &mut Context<Self>) {
        self.setup.cancel();
        cx.notify();
    }
}

fn card(p: Palette) -> Div {
    v_flex().w_full().gap_2().p_4().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair)
}

fn title(p: Palette, engine: Engine, name: &'static str, badge: &'static str) -> impl IntoElement {
    let platform = platform(engine);
    h_flex()
        .gap_3()
        .items_center()
        .child(mark(platform))
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(name))
        .child(div().h(px(22.)).px_2().flex().items_center().rounded(px(11.)).bg(p.wash).text_size(px(12.)).text_color(p.text2).child(badge))
}

fn desc(p: Palette, text: impl Into<SharedString>) -> Div {
    div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(text.into())
}

fn waiting(p: Palette, text: &str) -> Div {
    h_flex().gap_2().items_center().text_size(px(SMALL)).text_color(p.text2).child(motion::spinner(14., p.text2)).child(text.to_owned())
}

fn connected(p: Palette) -> Div {
    h_flex().gap_2().items_center().text_size(px(SMALL)).text_color(p.text2).child(icon(Icon::Check, ICON_SM, p.accent)).child(t!("setup.connected"))
}

fn cancel(shell: &Shell, id: &'static str, cx: &mut Context<Shell>) -> ui::Button {
    ui::button(shell.palette, id, Kind::Ghost, t!("setup.cancel")).on_click(cx.listener(|this, _, _, cx| this.setup_cancel(cx)))
}

pub fn screen(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let s = &shell.setup;
    let busy = s.doing != Doing::Idle;

    // ChatGPT: the easiest way, answers only.
    let chatgpt_action: AnyElement = match (s.codex, s.doing) {
        (None, _) => waiting(p, t!("setup.checking")).into_any_element(),
        (Some(true), _) => connected(p).into_any_element(),
        (_, Doing::CodexLogin) => h_flex().gap_3().items_center().child(waiting(p, t!("setup.browser_waiting"))).child(cancel(shell, "setup-cancel-codex", cx)).into_any_element(),
        (Some(false), _) => ui::button(p, "setup-chatgpt", Kind::Primary, t!("setup.chatgpt_login")).disabled(busy).on_click(cx.listener(|this, _, _, cx| this.setup_do(Doing::CodexLogin, cx))).into_any_element(),
    };
    let chatgpt = card(p).child(title(p, Engine::Codex, "ChatGPT", t!("setup.chatgpt_badge"))).child(desc(p, t!("setup.chatgpt_desc"))).child(h_flex().pt_1().w_full().min_w_0().child(chatgpt_action));

    // Claude Code: needed for editing files.
    let claude_action: AnyElement = match (s.claude, s.doing) {
        (None, _) => waiting(p, t!("setup.checking")).into_any_element(),
        (Some(Claude::SignedIn), _) => connected(p).into_any_element(),
        (_, Doing::Installing) => waiting(p, t!("setup.installing")).into_any_element(),
        (_, Doing::ClaudeLogin) => h_flex().gap_3().items_center().child(waiting(p, t!("setup.browser_waiting"))).child(cancel(shell, "setup-cancel-claude", cx)).into_any_element(),
        (Some(Claude::Missing), _) if s.confirm_install => v_flex()
            .flex_1()
            .min_w_0()
            .gap_2()
            .child(desc(p, t!("setup.install_ask")))
            .child(div().px_3().py_2().rounded(px(R_MD)).bg(p.code_bg).border_1().border_color(p.hair).font_family(MONO).text_size(px(SMALL)).text_color(p.text2).child(setup::CLAUDE_INSTALL))
            .child(
                h_flex()
                    .gap_2()
                    .child(ui::button(p, "setup-install-go", Kind::Primary, t!("setup.install_go")).disabled(busy).on_click(cx.listener(|this, _, _, cx| this.setup_do(Doing::Installing, cx))))
                    .child(ui::button(p, "setup-install-no", Kind::Ghost, t!("setup.cancel")).on_click(cx.listener(|this, _, _, cx| {
                        this.setup.confirm_install = false;
                        cx.notify();
                    }))),
            )
            .into_any_element(),
        (Some(Claude::Missing), _) => ui::button(p, "setup-install", Kind::Secondary, t!("setup.install")).disabled(busy).on_click(cx.listener(|this, _, _, cx| {
            this.setup.confirm_install = true;
            cx.notify();
        })).into_any_element(),
        (Some(Claude::SignedOut), _) => ui::button(p, "setup-claude", Kind::Secondary, t!("setup.claude_login")).disabled(busy).on_click(cx.listener(|this, _, _, cx| this.setup_do(Doing::ClaudeLogin, cx))).into_any_element(),
    };
    let claude_state = match s.claude {
        Some(Claude::Missing) => t!("setup.claude_missing"),
        _ => t!("setup.claude_desc"),
    };
    let claude = card(p).child(title(p, Engine::Claude, "Claude Code", t!("setup.claude_badge"))).child(desc(p, claude_state)).child(h_flex().pt_1().w_full().min_w_0().child(claude_action));

    // What stopped, in plain words, with the tool's own last words under it.
    let failed = s.failed.as_ref().map(|(doing, why)| {
        let words = match doing {
            Doing::Installing => t!("setup.failed_install"),
            Doing::ClaudeLogin | Doing::CodexLogin => t!("setup.failed_login"),
            Doing::Creating => t!("setup.failed_create"),
            Doing::Joining => t!("setup.failed_join"),
            Doing::Idle => "",
        };
        v_flex()
            .w_full()
            .gap_1()
            .child(h_flex().gap_2().items_start().text_size(px(SMALL)).line_height(px(20.)).text_color(p.danger()).child(div().flex_none().pt(px(3.)).child(icon(Icon::Warning, ICON_SM, p.danger()))).child(div().flex_1().min_w_0().child(words)))
            .when(!why.is_empty(), |c| c.child(div().font_family(MONO).text_size(px(11.5)).line_height(px(17.)).text_color(p.muted).child(why.clone())))
            .when(*doing == Doing::Installing, |c| c.child(h_flex().child(ui::button(p, "setup-guide", Kind::Ghost, t!("setup.open_guide")).icon(Icon::External).on_click(|_, _, cx| cx.open_url(setup::CLAUDE_GUIDE)))))
    });

    // The squad: opens once an AI is connected.
    let name = setup::suggest_squad();
    let squad = card(p)
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("setup.squad_title")))
        .child(desc(p, if s.ready() { t!("setup.squad_text", name = name) } else { t!("setup.squad_locked").to_owned() }))
        .child(h_flex().pt_1().w_full().min_w_0().child(match s.doing {
            Doing::Creating | Doing::Joining => waiting(p, t!("setup.joining")).into_any_element(),
            _ => ui::button(p, "setup-create", Kind::Primary, t!("setup.squad_create")).disabled(busy || !s.ready()).on_click(cx.listener(|this, _, _, cx| this.setup_do(Doing::Creating, cx))).into_any_element(),
        }));

    v_flex()
        .id("setup")
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
                .gap_3()
                .child(div().text_size(px(18.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("setup.title")))
                .child(div().text_size(px(BODY)).line_height(px(24.)).text_color(p.text2).child(t!("setup.text")))
                .child(chatgpt)
                .child(claude)
                .children(failed)
                .child(squad)
                .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(t!("setup.foot"))),
            "setup-in",
        ))
        .into_any_element()
}
