//! The box the person writes in: several lines, Enter sends and ⇧Enter breaks
//! the line, a Korean syllable still being composed is never sent, ↑↓ walk what
//! was sent, `/…` and `@…` complete, and Esc stops the member who is working.
//! The text editing is GPUI Kit's `Textarea`; what happens around it —
//! completion, history, drafts, sending — is `compose.rs`, tested without a window.

use std::time::Instant;

use equipe_core::protocol::{Command, Engine, SessionId};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::commands::Choice;
use crate::compose::{self, Drafts, Effect, EnterGuard, History};
use crate::state::{Model, Presence};
use crate::theme::{MONO, Palette, SMALL, look};
use crate::{Screen, Shell};

/// How tall the box may grow, in lines, before it scrolls inside.
const MAX_LINES: usize = 8;
/// Completion lists at most this many rows (32 tall).
const ROWS: usize = 8;

pub struct Composer {
    pub state: Entity<TextareaState>,
    history: History,
    drafts: Drafts,
    guard: EnterGuard,
    /// One quiet line under the conversation: what the last send did.
    pub flash: Option<String>,
    choices: Vec<Choice>,
    pick: usize,
    /// The text completion was dismissed for (Esc), until it changes.
    dismissed: Option<String>,
    /// Whose draft the box holds: the member and the project in focus.
    holds: Option<(SessionId, Option<String>)>,
    /// A member the person has asked to stop and who hasn't yet.
    stopping: Option<SessionId>,
    /// The greyed guess at what comes next, shown while the box is empty;
    /// → takes it.
    suggestion: Option<String>,
}

const PLACEHOLDER: &str = "메시지 쓰기…";

impl Composer {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let state = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, MAX_LINES).submit_on_enter(true).placeholder(PLACEHOLDER));
        Self {
            state,
            history: History::default(),
            drafts: Drafts::default(),
            guard: EnterGuard::default(),
            flash: None,
            choices: Vec::new(),
            pick: 0,
            dismissed: None,
            holds: None,
            stopping: None,
            suggestion: None,
        }
    }

    pub fn text(&self, cx: &App) -> String {
        self.state.read(cx).value().to_string()
    }

    pub(crate) fn set_text(&self, text: &str, window: &mut Window, cx: &mut App) {
        self.state.update(cx, |s, cx| {
            s.set_value(text.to_owned(), window, cx);
            s.set_selected_range(text.len()..text.len(), cx);
        });
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.state.update(cx, |s, cx| s.focus(window, cx));
    }

    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.state.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Whether completion is open.
    fn open(&self) -> bool {
        !self.choices.is_empty()
    }

    fn refresh_choices(&mut self, model: &Model, cx: &App) {
        let text = self.text(cx);
        self.pick = 0;
        self.choices = if self.dismissed.as_deref() == Some(text.as_str()) { Vec::new() } else { compose::choices(model, &text) };
    }

    /// Keep the box holding the draft of whoever is on screen: when the member
    /// or the project in focus changes, the old draft is put away and the
    /// new one's comes out.
    pub fn follow(&mut self, model: &Model, window: &mut Window, cx: &mut App) {
        let guess = model.convo().and_then(|c| c.suggestion.clone()).filter(|_| model.selected_member().is_some_and(|m| m.presence == Presence::Ready));
        if guess != self.suggestion {
            let hint = guess.as_ref().map_or_else(|| PLACEHOLDER.to_owned(), |g| format!("{g}  →"));
            self.state.update(cx, |s, cx| s.set_placeholder(hint, window, cx));
            self.suggestion = guess;
        }
        let key = model.selected.map(|id| (id, model.focus.clone()));
        if key == self.holds {
            return;
        }
        if let Some((member, project)) = self.holds.take() {
            let text = self.text(cx);
            self.drafts.put(member, project, text);
        }
        let draft = key.as_ref().map(|(m, p)| self.drafts.get(*m, p)).unwrap_or_default();
        self.set_text(&draft, window, cx);
        self.holds = key;
        self.history.stop();
        self.choices.clear();
        self.dismissed = None;
        self.flash = None;
    }
}

impl Shell {
    /// The box changed or was acted on: the text, the guard, completion.
    pub fn on_composer_event(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Change => {
                self.composer.history.stop();
                self.composer.flash = None;
                self.composer.refresh_choices(&self.model, cx);
                cx.notify();
            }
            InputEvent::PressEnter { shift: false, .. } => self.send(window, cx),
            InputEvent::PressEnter { .. } | InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    /// The box was touched in any way: is a syllable still being composed?
    pub fn on_composer_touched(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.composer.state.clone();
        let composing = state.update(cx, |s, cx| gpui_kit::EntityInputHandler::marked_text_range(s, window, cx).is_some());
        self.composer.guard.observe(composing, Instant::now());
    }

    /// Send what is in the box: to the member on screen, to `@name`, or as a command.
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.send_as(false, window, cx);
    }

    /// As `send`; `later` leaves a member at work a memo instead of steering it.
    pub fn send_as(&mut self, later: bool, window: &mut Window, cx: &mut Context<Self>) {
        // The Enter that ends a Korean composition is only that.
        if !self.composer.guard.allows_send(Instant::now()) {
            return;
        }
        let text = self.composer.text(cx);
        let plan = compose::plan(&self.model, &text, later);
        if plan.commands.is_empty() && plan.flash.is_none() && plan.effect.is_none() && plan.restore.is_none() {
            return;
        }
        if !plan.commands.is_empty() || plan.effect.is_some() {
            self.composer.history.push(&text);
        }
        for command in plan.commands {
            self.send_command(command);
        }
        self.composer.flash = plan.flash;
        self.composer.choices.clear();
        self.composer.dismissed = None;
        let box_text = plan.restore.unwrap_or_default();
        self.composer.set_text(&box_text, window, cx);
        match plan.effect {
            Some(Effect::Select(session)) => self.select_member(session, window, cx),
            Some(Effect::ShowBoard) => self.show(Screen::Board, cx),
            Some(Effect::Find(words)) => self.board_find(&words, cx),
            Some(Effect::Focus(project)) => self.set_focus(project, window, cx),
            None => {}
        }
        // What was just said belongs in view.
        self.chat.follow_tail(cx);
        cx.notify();
    }

    /// Stop the turn in progress (Esc, or the Stop button).
    pub fn interrupt(&mut self, cx: &mut Context<Self>) {
        let Some(member) = self.model.selected_member() else { return };
        if member.presence != Presence::Working || self.composer.stopping == Some(member.session) {
            return;
        }
        let session = member.session;
        self.composer.stopping = Some(session);
        self.send_command(Command::Interrupt { session });
        cx.notify();
    }

    /// Esc, ↑, ↓, Tab and Enter, before the box's own bindings see them. True
    /// when the key was taken.
    pub fn on_composer_key(&mut self, key: &str, modifiers: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.screen != Screen::Chat || !self.composer.is_focused(window, cx) {
            return false;
        }
        let plain = !modifiers.control && !modifiers.alt && !modifiers.platform && !modifiers.shift;
        match key {
            "escape" if plain => {
                if self.composer.open() {
                    self.composer.dismissed = Some(self.composer.text(cx));
                    self.composer.choices.clear();
                } else if !self.composer.guard.composing() {
                    self.interrupt(cx);
                }
                cx.notify();
                true
            }
            "up" | "down" if plain => {
                let up = key == "up";
                if self.composer.open() {
                    let n = self.composer.choices.len().min(ROWS);
                    self.composer.pick = if up { (self.composer.pick + n - 1) % n } else { (self.composer.pick + 1) % n };
                    cx.notify();
                    return true;
                }
                let text = self.composer.text(cx);
                let line = if up { self.composer.history.older(&text).map(str::to_owned) } else { self.composer.history.newer().map(str::to_owned) };
                match line {
                    Some(line) => {
                        self.composer.set_text(&line, window, cx);
                        cx.notify();
                        true
                    }
                    None => false,
                }
            }
            // → on an empty box takes the greyed suggestion.
            "right" if plain && self.composer.text(cx).is_empty() && self.composer.suggestion.is_some() => {
                if let Some(guess) = self.composer.suggestion.clone() {
                    self.composer.set_text(&guess, window, cx);
                    cx.notify();
                }
                true
            }
            // Tab with something written to a member at work: a memo for after.
            "tab" if plain && !self.composer.open() && self.model.selected_member().is_some_and(|m| m.presence == Presence::Working) && {
                let text = self.composer.text(cx);
                !text.trim().is_empty() && !text.starts_with('/')
            } =>
            {
                self.send_as(true, window, cx);
                true
            }
            "tab" if plain && self.composer.open() => {
                self.take_choice(self.composer.pick, window, cx);
                true
            }
            "enter" if plain && self.composer.open() => {
                // A choice that is whole runs; one that wants more is taken.
                let whole = self.composer.choices.get(self.composer.pick).is_some_and(|c| !c.text.ends_with(' '));
                self.take_choice(self.composer.pick, window, cx);
                if whole {
                    self.send(window, cx);
                }
                true
            }
            _ => false,
        }
    }

    pub fn take_choice(&mut self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = self.composer.choices.get(at).cloned() else { return };
        self.composer.set_text(&choice.text, window, cx);
        self.composer.refresh_choices(&self.model, cx);
        self.composer.focus(window, cx);
        cx.notify();
    }
}

/// The composer card with its completion list above and its bar below.
pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let composer = &shell.composer;
    let model = &shell.model;
    let member = model.selected_member();
    let working = member.is_some_and(|m| m.presence == Presence::Working);
    let stopping = working && member.is_some_and(|m| composer.stopping == Some(m.session));
    let text = composer.text(cx);
    let (to_label, to_color) = recipient(p, model, &text);
    let project = model.focus.clone().filter(|_| member.is_some_and(Model::is_pm));

    let list = composer.open().then(|| {
        v_flex()
            .absolute()
            .left_0()
            .right_0()
            .bottom(relative(1.))
            .mb_2()
            .py_1()
            .rounded(px(12.))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .children(composer.choices.iter().take(ROWS).enumerate().map(|(ix, choice)| {
                let on = ix == composer.pick;
                h_flex()
                    .id(("choice", ix))
                    .h(px(32.))
                    .px_4()
                    .gap_3()
                    .items_center()
                    .cursor_pointer()
                    .when(on, |r| r.bg(p.wash))
                    .hover(move |s| s.bg(p.wash))
                    .on_click(cx.listener(move |this, _, window, cx| this.take_choice(ix, window, cx)))
                    .child(div().flex_none().font_family(MONO).text_size(px(13.)).text_color(p.text).child(choice.label.clone()))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(SMALL)).text_color(p.muted).child(choice.about.clone()))
            }))
    });

    let button = if working {
        div()
            .id("stop")
            .h(px(28.))
            .px_4()
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(SMALL))
            .font_weight(FontWeight::SEMIBOLD)
            .border_1()
            .border_color(p.hair)
            .text_color(if stopping { p.muted } else { p.text })
            .when(!stopping, |b| b.cursor_pointer().hover(move |s| s.bg(p.wash)).on_click(cx.listener(|this, _, _, cx| this.interrupt(cx))))
            .child(if stopping { "중단하는 중…" } else { "■ 중단 ⎋" })
    } else {
        let can = !text.trim().is_empty();
        div()
            .id("send")
            .h(px(28.))
            .px_4()
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(SMALL))
            .font_weight(FontWeight::SEMIBOLD)
            .bg(if can { p.primary } else { p.wash })
            .text_color(if can { p.on_primary } else { p.muted })
            .when(can, |b| b.cursor_pointer().on_click(cx.listener(|this, _, window, cx| this.send(window, cx))))
            .child("보내기 ⏎")
    };

    // What Enter does with a line to a member at work: steer it (Claude) or
    // leave a memo (Codex, which cannot be steered).
    let steerable = member.is_some_and(|m| m.engine == Some(Engine::Claude));
    let says = working && !text.trim().is_empty() && !text.starts_with('/');
    let hint = match (says, steerable) {
        (true, true) => "⏎ 조종 · ⇥ 메모 · ⇧⏎ 줄바꿈",
        (true, false) => "⏎ 메모 · ⇧⏎ 줄바꿈",
        _ => "⇧⏎ 줄바꿈",
    };
    let memo_button = (says && steerable).then(|| {
        div()
            .id("memo")
            .h(px(28.))
            .px_4()
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(SMALL))
            .font_weight(FontWeight::SEMIBOLD)
            .border_1()
            .border_color(p.hair)
            .text_color(p.text)
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .on_click(cx.listener(|this, _, window, cx| this.send_as(true, window, cx)))
            .child("메모 ⇥")
    });

    v_flex()
        .w_full()
        .max_w(px(720.))
        .flex_none()
        .pb_4()
        .children(composer.flash.clone().map(|flash| div().px_1().pb_2().text_size(px(SMALL)).text_color(p.text2).child(flash)))
        .child(
            v_flex()
                .relative()
                .rounded(px(12.))
                .bg(p.raised)
                .border_1()
                .border_color(p.hair)
                .children(list)
                .child(
                    div()
                        .px_4()
                        .py_3()
                        .text_size(px(crate::theme::BODY))
                        .line_height(px(24.))
                        .child(Textarea::new(&composer.state).appearance(false).bordered(false)),
                )
                .child(
                    h_flex()
                        .h(px(36.))
                        .px_4()
                        .gap_3()
                        .items_center()
                        .child(
                            h_flex()
                                .gap_2()
                                .min_w_0()
                                .text_size(px(SMALL))
                                .child(div().text_color(p.muted).child("받는 사람"))
                                .child(div().text_color(to_color).font_weight(FontWeight::SEMIBOLD).child(to_label))
                                .children(project.map(|name| div().text_color(p.muted).child(format!("· {name}")))),
                        )
                        .child(div().flex_1().text_size(px(12.)).text_color(p.muted).child(hint))
                        .children(memo_button)
                        .child(button),
                ),
        )
}

/// Who the line goes to, as the bar shows it: always someone, never a guess.
fn recipient(p: Palette, model: &Model, text: &str) -> (String, Rgba) {
    let name_of = |m: &crate::state::Member| {
        let (mark, hue) = look(&m.role, &m.name);
        (format!("{mark} {}", m.name), p.member(hue))
    };
    if text.starts_with('/') {
        return ("명령".into(), p.accent);
    }
    if let Some(rest) = text.strip_prefix('@') {
        let name = rest.split(char::is_whitespace).next().unwrap_or("");
        if let Some(m) = model.open_member(name) {
            return name_of(m);
        }
    }
    model.selected_member().map_or(("—".into(), p.muted), name_of)
}
