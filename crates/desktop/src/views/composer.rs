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

use crate::attach::{self, Source, Stage, Strip};
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
    /// The images attached to the message being written.
    pub strip: Strip,
    /// Keeps the inbox folder watched for as long as the window lives.
    inbox_watch: Option<fs_notify::RecommendedWatcher>,
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
            strip: Strip::default(),
            inbox_watch: None,
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
        let typed = self.composer.text(cx);
        if self.composer.strip.busy() {
            self.composer.flash = Some("이미지를 올리는 중입니다 — 끝나면 보내집니다".into());
            cx.notify();
            return;
        }
        let tags = self.composer.strip.tags();
        let text = attach::message(&typed, &tags);
        let carries_images = !tags.is_empty() && !typed.trim().starts_with('/');
        let plan = compose::plan(&self.model, &text, later);
        if plan.commands.is_empty() && plan.flash.is_none() && plan.effect.is_none() && plan.restore.is_none() {
            return;
        }
        if !plan.commands.is_empty() || plan.effect.is_some() {
            self.composer.history.push(&typed);
        }
        // The images went with the message; one that couldn't be sent keeps them.
        if carries_images && plan.restore.is_none() && !plan.commands.is_empty() {
            self.composer.strip.clear();
        }
        for command in plan.commands {
            self.send_command(command);
        }
        self.composer.flash = plan.flash;
        self.composer.choices.clear();
        self.composer.dismissed = None;
        let box_text = plan
            .restore
            .map(|_| typed.trim().to_owned())
            .unwrap_or_default();
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

    /// Attach images: each is made ready and sent to the daemon's machine on a
    /// thread of the background pool; the strip shows it working meanwhile.
    pub fn attach_sources(&mut self, sources: Vec<Source>, cx: &mut Context<Self>) {
        let uploader = self.link.as_ref().map(|link| link.uploader());
        for source in sources {
            let Some(id) = self.composer.strip.begin(&source.name()) else {
                self.composer.flash = Some(format!(
                    "이미지는 한 번에 {}장까지 붙일 수 있습니다",
                    attach::MAX_IMAGES
                ));
                break;
            };
            let uploader = uploader.clone();
            let work = cx.background_spawn(async move {
                let prepared = attach::prepare(source)?;
                let kept = uploader
                    .ok_or("데몬에 연결되어 있지 않습니다")?
                    .upload(&prepared.name, &prepared.bytes)?;
                Ok::<_, String>((prepared, kept.to_string_lossy().into_owned()))
            });
            cx.spawn(async move |this, cx| {
                let done = work.await;
                this.update(cx, |this, cx| {
                    if let Some(why) = this.composer.strip.finish(id, done) {
                        this.composer.flash = Some(why);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// ⌘V in the box. True when the clipboard held images and they were taken;
    /// false lets the box paste its text.
    pub fn paste_item(&mut self, item: &ClipboardItem, cx: &mut Context<Self>) -> bool {
        match attach::from_clipboard(item) {
            Some(sources) => {
                self.attach_sources(sources, cx);
                true
            }
            None => false,
        }
    }

    /// Files dropped on the window: images are attached, anything else is
    /// written into the box as its path.
    pub fn dropped(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen != Screen::Chat {
            return;
        }
        let (images, rest) = attach::from_drop(paths.paths());
        if !rest.is_empty() {
            let mut text = self.composer.text(cx);
            for path in rest {
                if !text.is_empty() && !text.ends_with(' ') {
                    text.push(' ');
                }
                text.push_str(&path.to_string_lossy());
            }
            self.composer.set_text(&text, window, cx);
        }
        self.attach_sources(images, cx);
        self.composer.focus(window, cx);
    }

    /// Images `equipe send-image` drops in this Mac's inbox join the message,
    /// as soon as the folder changes (a file watch, no timer). Only while the
    /// window is on this Mac's daemon: another machine's inbox is its own.
    pub fn watch_inbox(&mut self, cx: &mut Context<Self>) {
        use fs_notify::Watcher as _;
        let dir = attach::inbox();
        let _ = std::fs::create_dir_all(&dir);
        let (tx, mut changed) = tokio::sync::mpsc::unbounded_channel::<()>();
        let ping = tx.clone();
        let watcher = fs_notify::recommended_watcher(move |_| {
            let _ = ping.send(());
        })
        .and_then(|mut w| {
            w.watch(&dir, fs_notify::RecursiveMode::NonRecursive)
                .map(|()| w)
        });
        self.composer.inbox_watch = watcher.ok();
        // What was already waiting.
        let _ = tx.send(());
        cx.spawn(async move |this, cx| {
            while changed.recv().await.is_some() {
                while changed.try_recv().is_ok() {}
                // Wait (quietly) for the daemon: a file taken now would be lost.
                loop {
                    let ready = this
                        .update(cx, |this, _| {
                            !this.endpoint.is_remote()
                                && matches!(this.model.conn, crate::state::Conn::Connected { .. })
                        })
                        .unwrap_or(true);
                    if ready {
                        break;
                    }
                    let remote = this
                        .update(cx, |this, _| this.endpoint.is_remote())
                        .unwrap_or(true);
                    if remote || !attach::inbox_waiting() {
                        break;
                    }
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(2))
                        .await;
                }
                if this
                    .update(cx, |this, _| {
                        this.endpoint.is_remote()
                            || !matches!(this.model.conn, crate::state::Conn::Connected { .. })
                    })
                    .unwrap_or(true)
                {
                    continue;
                }
                let arrived = cx
                    .background_executor()
                    .spawn(async { attach::take_inbox() })
                    .await;
                if arrived.is_empty() {
                    continue;
                }
                if this
                    .update(cx, |this, cx| {
                        this.composer.flash = Some("Mac에서 보낸 이미지를 붙였습니다".into());
                        this.attach_sources(arrived, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
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
        let can = (!text.trim().is_empty() || composer.strip.ready() > 0) && !composer.strip.busy();
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

    let shell_entity = cx.entity().downgrade();
    let strip = strip_row(shell, cx);
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
                .children(strip)
                .child(
                    div()
                        .px_4()
                        .py_3()
                        .text_size(px(crate::theme::BODY))
                        .line_height(px(24.))
                        .child(
                            Textarea::new(&composer.state)
                                .appearance(false)
                                .bordered(false)
                                .on_paste(move |item, _, cx| {
                                    shell_entity
                                        .update(cx, |shell, cx| shell.paste_item(item, cx))
                                        .unwrap_or(false)
                                }),
                        ),
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
                                .child(
                                    div()
                                        .text_color(to_color)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(to_label),
                                )
                                .children(project.map(|name| {
                                    div().text_color(p.muted).child(format!("· {name}"))
                                })),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .text_color(p.muted)
                                .child("⇧⏎ 줄바꿈 · ⌘V 이미지"),
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

/// The attached images, above the text inside the card (T-54 §4): 72-point
/// thumbnails with a number and an always-visible ✕, and what they are.
fn strip_row(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let strip = &shell.composer.strip;
    if strip.is_empty() {
        return None;
    }
    let p = shell.palette;
    let tiles = strip.items().iter().enumerate().map(|(n, item)| {
        let id = item.id;
        let face = match &item.thumb {
            Some(thumb) => img(thumb.clone())
                .size_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(p.muted)
                .child("올리는 중…")
                .into_any_element(),
        };
        div()
            .relative()
            .flex_none()
            .size(px(72.))
            .child(
                div()
                    .size_full()
                    .rounded(px(8.))
                    .overflow_hidden()
                    .bg(p.wash)
                    .border_1()
                    .border_color(p.hair)
                    .child(face),
            )
            .child(
                div()
                    .absolute()
                    .left(px(6.))
                    .bottom(px(4.))
                    .text_size(px(11.))
                    .text_color(p.muted)
                    .child(format!("{}", n + 1)),
            )
            .child(
                div()
                    .id(("unattach", n))
                    .absolute()
                    .top(px(-6.))
                    .right(px(-6.))
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(p.raised)
                    .border_1()
                    .border_color(p.hair)
                    .text_size(px(11.))
                    .text_color(p.text2)
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.wash))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.composer.strip.remove(id);
                        cx.notify();
                    }))
                    .child("✕"),
            )
    });
    let meta = strip
        .items()
        .iter()
        .filter(|i| matches!(i.stage, Stage::Ready(_)))
        .map(|i| {
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(format!("{} · {}", i.name, size_label(i.size)))
        });
    let count = strip.items().len();
    Some(
        h_flex()
            .px_4()
            .pt_4()
            .gap_3()
            .items_start()
            .child(h_flex().gap_3().children(tiles))
            .child(
                v_flex()
                    .min_w_0()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(p.muted)
                    .children(meta.take(2))
                    .child(
                        div()
                            .text_color(p.accent)
                            .child(format!("{count}장 — 메시지와 함께 보냅니다")),
                    ),
            ),
    )
}

fn size_label(bytes: usize) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / f64::from(1 << 20))
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

/// Over the whole window while files are dragged in: a dashed accent frame and
/// "놓으면 첨부" (T-54 §4). Invisible, and in nobody's way, otherwise.
pub fn drop_overlay(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    if shell.screen != Screen::Chat || shell.overlay_open() {
        return None;
    }
    let p = shell.palette;
    Some(
        div()
            .absolute()
            .inset_0()
            .opacity(0.)
            .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(1.))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.dropped(paths, window, cx)
            }))
            .child(
                div()
                    .absolute()
                    .inset(px(8.))
                    .rounded(px(12.))
                    .border_2()
                    .border_dashed()
                    .border_color(p.accent)
                    .bg(p.accent_bg)
                    .flex()
                    .flex_col()
                    .gap_1()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.text)
                            .child("놓으면 이미지가 첨부됩니다"),
                    )
                    .child(
                        div()
                            .text_size(px(SMALL))
                            .text_color(p.text2)
                            .child(format!(
                                "{}장까지 · 한 장에 {} MB까지",
                                attach::MAX_IMAGES,
                                attach::MAX_BYTES >> 20
                            )),
                    ),
            ),
    )
}
