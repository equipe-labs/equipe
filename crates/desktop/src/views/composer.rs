//! The box the person writes in: several lines, Enter sends and ⇧Enter breaks
//! the line, a Korean syllable still being composed is never sent, ↑↓ walk what
//! was sent, `/…` and `@…` complete, Esc stops the member who is working.
//! Around it: what is attached (any file — pictures, videos, PDFs, text,
//! documents), the "+" menu (⌘U), the memos and steers waiting for a member at
//! work, who the message is for, the member's model, effort and mode, and how
//! full its context is. The text editing is GPUI Kit's `Textarea`; the rules —
//! completion, history, drafts, sending — are `compose.rs` and `attach.rs`,
//! tested without a window.

use equipe_core::t;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use equipe_core::protocol::{Command, Engine, Mode, SessionId};
use gpui_kit::component::input::{InlineToken, InputEvent, Paste, Textarea, TextareaState};
use gpui_kit::component::speech::{SpeechError, SpeechEvent, SpeechState, SystemRecognizer};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::attach::{self, Kind, Source, Stage, Strip};
use crate::commands::Choice;
use crate::compose::{self, Draft, Drafts, Effect, EnterGuard, History, Said};
use crate::state::{Model, Presence};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::target::Endpoint;
use crate::theme::{CONTROL, GAP, ICON, ICON_SM, MONO, Palette, R_LG, R_MD, R_SM, R_XL, ROW, SMALL, look};
use crate::tips::tip;
use crate::{Screen, Shell, files, media};

/// How tall the box may grow, in lines, before it scrolls inside.
const MAX_LINES: usize = 8;
/// Completion lists at most this many rows.
const ROWS: usize = 8;
/// A draft is written to disk this long after the last change.
const SAVE_AFTER: Duration = Duration::from_millis(800);

actions!(composer, [OpenAttach, StopMember, OpenModels, OpenEffort, OpenModes, PasteInline, SendInverted]);

/// The composer's own keys (main.rs binds them with the app's).
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-u", OpenAttach, None),
        KeyBinding::new("cmd-.", StopMember, None),
        KeyBinding::new("cmd-shift-i", OpenModels, None),
        KeyBinding::new("cmd-shift-e", OpenEffort, None),
        KeyBinding::new("cmd-shift-m", OpenModes, None),
        KeyBinding::new("cmd-shift-v", PasteInline, Some("Input")),
        KeyBinding::new("cmd-shift-enter", SendInverted, Some("Input")),
    ]);
}

/// The keys the composer may take before the text box's own bindings.
pub fn takes(key: &str) -> bool {
    matches!(key, "escape" | "up" | "down" | "tab" | "enter" | "left" | "right" | "backspace" | "delete" | "space" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
}

// ── menus ──

/// Which of the bar's menus is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuKind {
    Attach,
    Recent,
    To,
    Model,
    Effort,
    Mode,
    Context,
}

#[derive(Debug, Clone)]
enum Act {
    PickFiles,
    Recent,
    Again(PathBuf),
    Screenshot,
    Commands,
    Select(SessionId),
    Model(String),
    Effort(Option<String>),
    Mode(Mode),
    Refresh(bool),
}

impl MenuKind {
    /// The control a menu opens from: recent files open where the "+" menu was.
    fn trigger(self) -> MenuKind {
        match self {
            MenuKind::Recent => MenuKind::Attach,
            kind => kind,
        }
    }

    /// The edge of its trigger a menu lines up with: the bar's left-hand
    /// controls open menus flush with their left edge, the right-hand ones
    /// (model, effort, context) flush with their right edge.
    fn align(self) -> gpui_kit::base::Align {
        match self {
            MenuKind::Model | MenuKind::Effort | MenuKind::Context => gpui_kit::base::Align::End,
            _ => gpui_kit::base::Align::Start,
        }
    }

    fn width(self) -> f32 {
        match self {
            MenuKind::Recent => 340.,
            MenuKind::Context => 300.,
            MenuKind::Model => 280.,
            _ => 248.,
        }
    }
}

/// What leads a menu row: an icon, a member's mark in their colour, or nothing
/// (the column stays, so every label starts at the same place).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Lead {
    None,
    Icon(Icon),
    Mark(&'static str, usize),
}

#[derive(Debug, Clone)]
struct Row {
    label: String,
    hint: String,
    /// The shortcut that does the same, right-aligned.
    keys: Option<&'static str>,
    lead: Lead,
    /// Opens another menu (a chevron at the end).
    more: bool,
    on: bool,
    act: Option<Act>,
}

impl Row {
    fn new(label: impl Into<String>, hint: impl Into<String>, act: Act) -> Self {
        Self { label: label.into(), hint: hint.into(), keys: None, lead: Lead::None, more: false, on: false, act: Some(act) }
    }

    /// A line of words that does nothing (the context menu's explanation).
    fn note(label: impl Into<String>) -> Self {
        Self { label: label.into(), hint: String::new(), keys: None, lead: Lead::None, more: false, on: false, act: None }
    }

    fn checked(self, on: bool) -> Self {
        Self { on, ..self }
    }

    fn icon(self, icon: Icon) -> Self {
        Self { lead: Lead::Icon(icon), ..self }
    }

    fn keys(self, keys: &'static str) -> Self {
        Self { keys: Some(keys), ..self }
    }

    fn submenu(self) -> Self {
        Self { more: true, ..self }
    }
}

#[derive(Debug, Clone)]
pub struct Menu {
    kind: MenuKind,
    title: Option<String>,
    rows: Vec<Row>,
    pick: usize,
}

impl Menu {
    fn first_pick(rows: &[Row]) -> usize {
        rows.iter().position(|r| r.on && r.act.is_some()).or_else(|| rows.iter().position(|r| r.act.is_some())).unwrap_or(0)
    }

    fn step(&mut self, up: bool) {
        let n = self.rows.len();
        for _ in 0..n {
            self.pick = if up { (self.pick + n - 1) % n } else { (self.pick + 1) % n };
            if self.rows[self.pick].act.is_some() {
                return;
            }
        }
    }
}

/// The efforts each engine takes.
fn efforts(engine: Option<Engine>) -> &'static [&'static str] {
    match engine {
        Some(Engine::Codex) => &["minimal", "low", "medium", "high"],
        _ => &["low", "medium", "high", "xhigh", "max"],
    }
}

/// A model's name as the bar shows it: `claude-opus-5-5` → `opus 5.5`.
fn short_model(model: &str) -> String {
    let m = model.strip_prefix("claude-").unwrap_or(model);
    let mut parts: Vec<&str> = m.split('-').collect();
    // A trailing date (`20250929`) says nothing to the person.
    if parts.last().is_some_and(|p| p.len() == 8 && p.chars().all(|c| c.is_ascii_digit())) {
        parts.pop();
    }
    match parts.as_slice() {
        [name, major, minor] if major.chars().all(|c| c.is_ascii_digit()) && minor.chars().all(|c| c.is_ascii_digit()) => format!("{name} {major}.{minor}"),
        [name, major] if major.chars().all(|c| c.is_ascii_digit()) => format!("{name} {major}"),
        _ => parts.join("-"),
    }
}

// ── the composer ──

pub struct Composer {
    pub state: Entity<TextareaState>,
    history: History,
    drafts: Drafts,
    guard: EnterGuard,
    /// One quiet line over the box: what the last send did.
    pub flash: Option<String>,
    choices: Vec<Choice>,
    pick: usize,
    /// The text completion was dismissed for (Esc), until it changes.
    dismissed: Option<String>,
    /// Whose draft the box holds: its key (squad / member / project).
    holds: Option<String>,
    /// A member the person has asked to stop and who hasn't yet.
    stopping: Option<SessionId>,
    /// The greyed guess at what comes next, shown while the box is empty; → takes it.
    suggestion: Option<String>,
    placeholder: String,
    /// What is attached to the message being written.
    pub strip: Strip,
    /// The attachment ← reached from the start of the box (Backspace removes it).
    chip: Option<usize>,
    /// A menu of the bar, open.
    menu: Option<Menu>,
    /// The menu just closed, fading out (its number keys the fade).
    leaving: Option<(Menu, u64)>,
    menus_closed: u64,
    /// Where each control that opens a menu was last drawn: menus open from there.
    anchors: Rc<std::cell::RefCell<std::collections::HashMap<MenuKind, Bounds<Pixels>>>>,
    /// What a drag over the window would attach, said before the drop.
    drag: Option<(Vec<PathBuf>, attach::Preview)>,
    /// The drop frame shown without a drag (for the snapshot tool).
    pub drag_shown: bool,
    /// ⌘⇧V: the next paste goes into the box however long it is.
    inline_paste: bool,
    /// The effort asked for each member, as the bar shows it.
    efforts: std::collections::HashMap<SessionId, Option<String>>,
    /// The projects' files, for `@`.
    indexes: Vec<files::Index>,
    indexing: Vec<String>,
    /// The attachment shown big over the window.
    pub lightbox: Option<u64>,
    /// A PDF's first page drawn big for it, once drawn.
    page: Option<(u64, std::sync::Arc<Image>)>,
    save_task: Option<Task<()>>,
    /// Writes the draft down as the app quits.
    _quit: Subscription,
    /// Dictation: on-device speech recognition in the app's language.
    pub speech: Entity<SpeechState>,
    /// What is being said, while it is (shown dimmed under the box; Esc drops it).
    dictating: Option<String>,
    _speech: Subscription,
    /// Keeps the inbox folder watched for as long as the window lives.
    inbox_watch: Option<fs_notify::RecommendedWatcher>,
}

impl Composer {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let placeholder = t!("composer.placeholder").to_owned();
        let locale = speech_locale(equipe_core::i18n::Lang::detect());
        let speech = cx.new(|cx| SpeechState::new(cx).recognizer(SystemRecognizer::new().locale(locale)));
        let heard = cx.subscribe_in(&speech, window, |shell: &mut Shell, _, event: &SpeechEvent, window, cx| shell.on_speech(event, window, cx));
        let state = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, MAX_LINES).submit_on_enter(true).placeholder(placeholder.clone()));
        Self {
            state,
            history: History::default(),
            drafts: Drafts::load(Drafts::file(&equipe_providers::credentials::equipe_home())),
            guard: EnterGuard::default(),
            flash: None,
            choices: Vec::new(),
            pick: 0,
            dismissed: None,
            holds: None,
            stopping: None,
            suggestion: None,
            placeholder,
            strip: Strip::default(),
            chip: None,
            menu: None,
            leaving: None,
            menus_closed: 0,
            anchors: Rc::default(),
            drag: None,
            drag_shown: false,
            inline_paste: false,
            efforts: Default::default(),
            indexes: Vec::new(),
            indexing: Vec::new(),
            lightbox: None,
            page: None,
            save_task: None,
            _quit: cx.on_app_quit(|shell: &mut Shell, cx| {
                shell.composer.stash(cx);
                shell.composer.drafts.save();
                async {}
            }),
            speech,
            dictating: None,
            _speech: heard,
            inbox_watch: None,
        }
    }

    pub fn text(&self, cx: &App) -> String {
        self.state.read(cx).value().to_string()
    }

    pub(crate) fn set_text(&self, text: &str, window: &mut Window, cx: &mut App) {
        self.set_text_at(text, text.len(), window, cx);
    }

    fn set_text_at(&self, text: &str, caret: usize, window: &mut Window, cx: &mut App) {
        let caret = (0..=caret.min(text.len())).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
        self.state.update(cx, |s, cx| {
            s.set_value(text.to_owned(), window, cx);
            s.set_selected_range(caret..caret, cx);
        });
    }

    fn caret(&self, cx: &App) -> usize {
        self.state.read(cx).cursor()
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
        self.choices = if self.dismissed.as_deref() == Some(text.as_str()) { Vec::new() } else { compose::choices_with(model, &text, &self.indexes) };
    }

    /// What the box holds now, as a draft.
    fn draft(&self, cx: &App) -> Draft {
        Draft { text: self.text(cx), caret: self.caret(cx), attachments: self.strip.saved() }
    }

    /// Put what the box holds away under the key it belongs to.
    fn stash(&mut self, cx: &App) {
        if let Some(key) = self.holds.clone() {
            let draft = self.draft(cx);
            self.drafts.put(key, draft);
        }
    }

    /// The key of the draft for what is on screen.
    fn key_of(model: &Model) -> Option<String> {
        let member = model.selected_member()?;
        Some(Drafts::key(&model.squad, &member.name, model.focus.as_deref()))
    }

    /// Keep the box holding the draft of whoever is on screen: when the member
    /// or the project in focus changes, the old draft is put away and the
    /// new one's comes out — words, caret and attachments.
    pub fn follow(&mut self, model: &Model, window: &mut Window, cx: &mut App) {
        let guess = model.convo().and_then(|c| c.suggestion.clone()).filter(|_| model.viewed_presence() == Some(Presence::Ready));
        // A stop asked for is over once nothing in view is at work.
        if self.stopping.is_some() && model.viewed_presence() != Some(Presence::Working) {
            self.stopping = None;
        }
        self.suggestion = guess;
        let hint = match &self.suggestion {
            Some(g) => format!("{g}  →"),
            None => placeholder(model),
        };
        if hint != self.placeholder {
            self.state.update(cx, |s, cx| s.set_placeholder(hint.clone(), window, cx));
            self.placeholder = hint;
        }
        let key = Self::key_of(model);
        if key == self.holds {
            return;
        }
        self.stash(cx);
        let draft = key.as_ref().map(|k| self.drafts.get(k)).unwrap_or_default();
        self.set_text_at(&draft.text, draft.caret, window, cx);
        self.strip.restore(&draft.attachments);
        self.holds = key;
        self.history.stop();
        self.choices.clear();
        self.menu = None;
        self.chip = None;
        self.dismissed = None;
        self.flash = None;
    }
}

/// The language dictation listens for: the app's.
fn speech_locale(lang: equipe_core::i18n::Lang) -> &'static str {
    use equipe_core::i18n::Lang as L;
    match lang {
        L::Ko => "ko-KR",
        L::En => "en-US",
        L::Ja => "ja-JP",
        L::ZhHans => "zh-CN",
        L::ZhHant => "zh-TW",
        L::Es => "es-ES",
        L::Fr => "fr-FR",
        L::De => "de-DE",
    }
}

/// What the empty box says: who it is for.
fn placeholder(model: &Model) -> String {
    let Some(member) = model.selected_member() else {
        return t!("composer.placeholder").to_owned();
    };
    match model.opened {
        Some(ticket) => {
            let to = if member.ticket == Some(ticket) { member.name.as_str() } else { model.members.iter().find(|m| Model::is_pm(m)).map_or(member.name.as_str(), |m| m.name.as_str()) };
            t!("composer.placeholder_ticket", id = ticket, name = to)
        }
        None => t!("composer.placeholder_to", name = member.name),
    }
}

impl Shell {
    // ── typing ──

    /// The box changed or was acted on: the text, the guard, completion.
    pub fn on_composer_event(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Change => {
                self.composer.history.stop();
                self.composer.flash = None;
                self.composer.chip = None;
                if self.composer.text(cx).contains('@') {
                    self.index_projects(cx);
                }
                self.composer.refresh_choices(&self.model, cx);
                self.save_drafts_soon(cx);
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

    /// Read the projects' files for `@`, once each, when `@` is first typed —
    /// on a daemon on this Mac only: another machine's disk isn't ours to walk.
    fn index_projects(&mut self, cx: &mut Context<Self>) {
        if self.endpoint.is_remote() {
            return;
        }
        for root in self.model.projects.clone() {
            let name = equipe_core::board::project_name(&root);
            if self.composer.indexing.contains(&name) {
                continue;
            }
            self.composer.indexing.push(name.clone());
            let walk = cx.background_spawn(async move { files::index(&name, &root) });
            cx.spawn(async move |this, cx| {
                let index = walk.await;
                this.update(cx, |this, cx| {
                    this.composer.indexes.retain(|i| i.project != index.project);
                    this.composer.indexes.push(index);
                    this.composer.refresh_choices(&this.model, cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// Write the drafts down a moment after the last change, off the UI thread.
    fn save_drafts_soon(&mut self, cx: &mut Context<Self>) {
        if self.composer.save_task.is_some() {
            return;
        }
        self.composer.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_AFTER).await;
            let snapshot = this
                .update(cx, |this, cx| {
                    this.composer.save_task = None;
                    this.composer.stash(cx);
                    this.composer.drafts.snapshot()
                })
                .ok()
                .flatten();
            if let Some((path, text)) = snapshot {
                cx.background_executor().spawn(async move { compose::write_private(&path, &text) }).await;
            }
        }));
    }

    // ── sending ──

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
        let typed = attach::clean(&self.composer.text(cx));
        if self.composer.strip.busy() {
            self.composer.flash = Some(t!("composer.uploading_wait").into());
            cx.notify();
            return;
        }
        let tags = self.composer.strip.tags();
        let text = attach::message(&typed, &tags);
        let carries = !tags.is_empty() && !typed.trim().starts_with('/');
        let plan = compose::plan(&self.model, &text, later);
        if plan.commands.is_empty() && plan.flash.is_none() && plan.effect.is_none() && plan.restore.is_none() {
            return;
        }
        if !plan.commands.is_empty() || plan.effect.is_some() {
            let attachments = if carries { self.composer.strip.saved() } else { Vec::new() };
            self.composer.history.push(Said { text: typed.trim().to_owned(), attachments });
        }
        // The attachments went with the message; one that couldn't be sent keeps them.
        if carries && plan.restore.is_none() && !plan.commands.is_empty() {
            self.composer.strip.clear();
        }
        for command in plan.commands {
            self.send_command(command);
        }
        self.composer.flash = plan.flash;
        self.composer.choices.clear();
        self.close_menu(cx);
        self.composer.dismissed = None;
        let box_text = plan.restore.map(|_| typed.trim().to_owned()).unwrap_or_default();
        self.composer.set_text(&box_text, window, cx);
        match plan.effect {
            Some(Effect::Select(session)) => self.select_member(session, window, cx),
            Some(Effect::ShowBoard) => self.show(Screen::Board, cx),
            Some(Effect::Find(words)) => self.board_find(&words, cx),
            Some(Effect::Focus(project)) => self.set_focus(project, window, cx),
            None => {}
        }
        self.save_drafts_soon(cx);
        // What was just said belongs in view.
        self.chat.follow_tail(cx);
        cx.notify();
    }

    /// ⌘⇧⏎: to a member at work, the other of steer and memo, this once.
    fn send_inverted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let steerable = self.model.selected_member().is_some_and(|m| m.engine == Some(Engine::Claude));
        let working = self.model.viewed_presence() == Some(Presence::Working);
        self.send_as(working && steerable, window, cx);
    }

    /// Stop the turn in progress (Esc on an empty box, ⌘., or the Stop button).
    pub fn interrupt(&mut self, cx: &mut Context<Self>) {
        // Only the conversation in view stops; a member's others go on.
        let Some(session) = self.model.viewed() else { return };
        if self.model.viewed_presence() != Some(Presence::Working) || self.composer.stopping == Some(session) {
            return;
        }
        self.composer.stopping = Some(session);
        self.send_command(Command::Interrupt { session });
        cx.notify();
    }

    // ── dictation ──

    /// The mic button: start listening, or stop and keep what was said.
    fn toggle_dictation(&mut self, cx: &mut Context<Self>) {
        self.composer.speech.update(cx, |s, cx| s.toggle(cx));
        cx.notify();
    }

    fn on_speech(&mut self, event: &SpeechEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            SpeechEvent::Started => self.composer.dictating = Some(String::new()),
            SpeechEvent::Partial(text) => self.composer.dictating = Some(text.to_string()),
            SpeechEvent::Final(text) => {
                self.composer.dictating = None;
                let said = text.trim();
                if !said.is_empty() {
                    let mut now = self.composer.text(cx);
                    if !now.is_empty() && !now.ends_with([' ', '\n']) {
                        now.push(' ');
                    }
                    now.push_str(said);
                    self.composer.set_text(&now, window, cx);
                    self.composer.focus(window, cx);
                }
            }
            SpeechEvent::Cancelled => self.composer.dictating = None,
            SpeechEvent::Error(error) => {
                self.composer.dictating = None;
                self.composer.flash = Some(match error {
                    SpeechError::PermissionDenied => t!("composer.dictation_denied").to_owned(),
                    SpeechError::NoInputDevice => t!("composer.dictation_no_mic").to_owned(),
                    SpeechError::Unsupported => t!("composer.dictation_unsupported").to_owned(),
                    other => t!("composer.dictation_failed", why = other),
                });
            }
        }
        cx.notify();
    }

    // ── attaching ──

    /// Where attachments are kept: the daemon on this Mac's folder, or the far
    /// machine (an `upload` line, or SSH for big files).
    fn store(&self) -> Option<attach::Store> {
        match &self.endpoint {
            Endpoint::Local { socket } => Some(attach::Store::Local { home: socket.parent()?.to_owned() }),
            Endpoint::Ssh(host) => Some(attach::Store::Remote { uploader: self.link.as_ref()?.uploader(), ssh: Some((crate::ssh::Launcher::system(None), host.clone())) }),
        }
    }

    /// Attach: each is checked, read on this Mac and sent to the daemon's
    /// machine on a thread of the background pool; its chip shows how far it got.
    pub fn attach_sources(&mut self, sources: Vec<Source>, cx: &mut Context<Self>) {
        let store = self.store();
        let mut over = 0;
        for source in sources {
            if let Some(path) = source.path() {
                if let Err(why) = attach::check(path) {
                    self.composer.strip.refuse(&source.name(), source.kind(), why);
                    continue;
                }
                self.composer.drafts.used(path);
            }
            let Some(id) = self.composer.strip.begin(&source) else {
                over += 1;
                continue;
            };
            let store = store.clone();
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Step>();
            cx.background_spawn(async move {
                let prepared = match attach::prepare(source) {
                    Ok(prepared) => prepared,
                    Err(why) => return drop(tx.send(Step::Done(Err(why)))),
                };
                let _ = tx.send(Step::Shown(Box::new(prepared_head(&prepared))));
                let Some(store) = store else {
                    return drop(tx.send(Step::Done(Err(t!("composer.not_connected").into()))));
                };
                let progress = |share: f32| drop(tx.send(Step::Share(share)));
                let done = attach::deliver(&prepared, &store, &progress);
                let _ = tx.send(Step::Done(done));
            })
            .detach();
            cx.spawn(async move |this, cx| {
                while let Some(step) = rx.recv().await {
                    let finished = matches!(step, Step::Done(_));
                    let alive = this.update(cx, |this, cx| {
                        let strip = &mut this.composer.strip;
                        let redraw = match step {
                            Step::Shown(prepared) => {
                                strip.prepared(id, &prepared);
                                true
                            }
                            Step::Share(share) => strip.progress(id, share),
                            Step::Done(done) => {
                                strip.finish(id, done);
                                this.save_drafts_soon(cx);
                                true
                            }
                        };
                        if redraw {
                            cx.notify();
                        }
                    });
                    if alive.is_err() || finished {
                        break;
                    }
                }
            })
            .detach();
        }
        if over > 0 {
            self.composer.flash = Some(t!("composer.too_many", n = attach::MAX_ATTACHMENTS, over = over));
        }
        cx.notify();
    }

    /// ⌘V in the box. True when the clipboard held files or a picture and they
    /// were attached, or held a long text that became a file; false lets the
    /// box paste its text.
    pub fn paste_item(&mut self, item: &ClipboardItem, cx: &mut Context<Self>) -> bool {
        let inline = std::mem::take(&mut self.composer.inline_paste);
        if let Some(sources) = attach::from_clipboard(item) {
            self.attach_sources(sources, cx);
            return true;
        }
        match item.text() {
            Some(text) if !inline && attach::is_long_paste(&text) => {
                let name = format!("{}.txt", t!("composer.pasted_name"));
                self.attach_sources(vec![Source::Text { name, text }], cx);
                self.composer.flash = Some(t!("composer.pasted_as_file").into());
                true
            }
            _ => false,
        }
    }

    /// Files dropped on the window: attached; folders are written into the box as their paths.
    pub fn dropped(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.drag = None;
        if self.screen != Screen::Chat {
            return;
        }
        let (files, folders) = attach::from_drop(paths.paths());
        if !folders.is_empty() {
            let mut text = self.composer.text(cx);
            for path in folders {
                if !text.is_empty() && !text.ends_with(' ') {
                    text.push(' ');
                }
                text.push_str(&path.to_string_lossy());
            }
            self.composer.set_text(&text, window, cx);
        }
        self.attach_sources(files, cx);
        self.composer.focus(window, cx);
    }

    /// "사진·파일…": the system's file picker.
    fn pick_files(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some(t!("composer.pick_prompt").into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                this.update(cx, |this, cx| this.attach_sources(paths.into_iter().map(Source::File).collect(), cx)).ok();
            }
        })
        .detach();
    }

    /// "스크린샷": the system's selection, then attached.
    fn screenshot(&mut self, cx: &mut Context<Self>) {
        let path = crate::ssh::scratch_dir().join(format!("screenshot-{}.png", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())));
        let shot = path.clone();
        let taken = cx.background_spawn(async move { std::process::Command::new("/usr/sbin/screencapture").arg("-i").arg("-x").arg(&shot).status().is_ok_and(|s| s.success()) && shot.is_file() });
        cx.spawn(async move |this, cx| {
            if taken.await {
                this.update(cx, |this, cx| this.attach_sources(vec![Source::File(path)], cx)).ok();
            }
        })
        .detach();
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
        .and_then(|mut w| w.watch(&dir, fs_notify::RecursiveMode::NonRecursive).map(|()| w));
        self.composer.inbox_watch = watcher.ok();
        // What was already waiting.
        let _ = tx.send(());
        cx.spawn(async move |this, cx| {
            while changed.recv().await.is_some() {
                while changed.try_recv().is_ok() {}
                // Wait (quietly) for the daemon: a file taken now would be lost.
                loop {
                    let ready = this.update(cx, |this, _| !this.endpoint.is_remote() && matches!(this.model.conn, crate::state::Conn::Connected { .. })).unwrap_or(true);
                    if ready {
                        break;
                    }
                    let remote = this.update(cx, |this, _| this.endpoint.is_remote()).unwrap_or(true);
                    if remote || !attach::inbox_waiting() {
                        break;
                    }
                    cx.background_executor().timer(Duration::from_secs(2)).await;
                }
                if this.update(cx, |this, _| this.endpoint.is_remote() || !matches!(this.model.conn, crate::state::Conn::Connected { .. })).unwrap_or(true) {
                    continue;
                }
                let arrived = cx.background_executor().spawn(async { attach::take_inbox() }).await;
                if arrived.is_empty() {
                    continue;
                }
                if this
                    .update(cx, |this, cx| {
                        this.composer.flash = Some(t!("composer.inbox_attached").into());
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

    /// The person removed an attachment (✕, Backspace on its chip).
    fn unattach(&mut self, id: u64, cx: &mut Context<Self>) {
        self.composer.strip.remove(id);
        if self.composer.lightbox == Some(id) {
            self.composer.lightbox = None;
        }
        self.composer.chip = self.composer.chip.filter(|_| !self.composer.strip.is_empty()).map(|i| i.min(self.composer.strip.len().saturating_sub(1)));
        self.save_drafts_soon(cx);
        cx.notify();
    }

    // ── the memos and steers waiting ──

    /// A memo waiting for the member's turn to end, taken back.
    fn unqueue(&mut self, text: &str) {
        if let Some(session) = self.model.viewed() {
            self.send_command(Command::Unqueue { session, text: text.to_owned() });
        }
    }

    /// 편집: the memo comes back into the box (and leaves the queue).
    fn edit_memo(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.unqueue(&text);
        let mut now = self.composer.text(cx);
        if !now.trim().is_empty() {
            now.push('\n');
        }
        now.push_str(&text);
        self.composer.set_text(&now, window, cx);
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// 지금 지시: the memo is read at the member's next step instead.
    fn steer_now(&mut self, text: String, cx: &mut Context<Self>) {
        self.unqueue(&text);
        if let Some(session) = self.model.viewed() {
            self.send_command(Command::Submit { session, text });
        }
        cx.notify();
    }

    // ── menus ──

    /// Close the bar's menu; it fades out where it was.
    pub(crate) fn close_menu(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = self.composer.menu.take() else { return };
        self.composer.menus_closed += 1;
        let n = self.composer.menus_closed;
        self.composer.leaving = Some((menu, n));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(motion::FAST + Duration::from_millis(40)).await;
            this.update(cx, |this, cx| {
                if this.composer.leaving.as_ref().is_some_and(|(_, at)| *at == n) {
                    this.composer.leaving = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn open_menu(&mut self, kind: MenuKind, window: &mut Window, cx: &mut Context<Self>) {
        crate::tips::hide(cx);
        if self.composer.menu.as_ref().is_some_and(|m| m.kind == kind) {
            self.close_menu(cx);
            return;
        }
        let member = self.model.selected_member();
        let mut title = None;
        let rows: Vec<Row> = match kind {
            MenuKind::Attach => {
                let recent = self.composer.drafts.recent.iter().any(|p| p.exists());
                let mut rows = vec![Row::new(t!("composer.menu_files"), "", Act::PickFiles).icon(Icon::Paperclip).keys("⌘U")];
                if recent {
                    rows.push(Row::new(t!("composer.menu_recent"), "", Act::Recent).icon(Icon::Clock).submenu());
                }
                rows.push(Row::new(t!("composer.menu_screenshot"), "", Act::Screenshot).icon(Icon::Camera));
                rows.push(Row::new(t!("composer.menu_commands"), "", Act::Commands).icon(Icon::Slash).keys("/"));
                rows
            }
            MenuKind::Recent => {
                title = Some(t!("composer.menu_recent").to_owned());
                self.composer.drafts.recent.iter().filter(|p| p.exists()).map(|p| Row::new(p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()), p.parent().map_or_else(String::new, short_dir), Act::Again(p.clone())).icon(Icon::File)).collect()
            }
            MenuKind::To => {
                title = Some(t!("composer.menu_to").to_owned());
                let selected = self.model.selected;
                self.model.ordered().iter().filter(|m| m.presence != Presence::Closed).map(|m| {
                        let (mark, hue) = look(&m.role, &m.name);
                        Row { lead: Lead::Mark(mark, hue), ..Row::new(m.name.clone(), m.role.clone(), Act::Select(m.session)).checked(Some(m.session) == selected) }
                    })
                    .collect()
            }
            MenuKind::Model => {
                let Some(member) = member else { return };
                title = Some(t!("composer.menu_model", name = member.name));
                let mut models = compose::engine_models(member.engine, member.model.as_deref());
                if models.is_empty() {
                    models.extend(member.model.clone());
                }
                let now = member.model.clone().unwrap_or_default();
                let exact = models.contains(&now);
                models
                    .into_iter()
                    .map(|m| {
                        let short = short_model(&m);
                        let hint = if short == m { String::new() } else { m.clone() };
                        Row::new(short, hint, Act::Model(m.clone())).checked(now == m || (!exact && now.contains(&m)))
                    })
                    .collect()
            }
            MenuKind::Effort => {
                let Some(member) = member else { return };
                title = Some(t!("composer.menu_effort", name = member.name));
                let now = self.composer.efforts.get(&member.session).cloned().flatten();
                let mut rows = vec![Row::new(t!("composer.effort_default"), "", Act::Effort(None)).checked(now.is_none())];
                rows.extend(efforts(member.engine).iter().map(|e| Row::new(*e, "", Act::Effort(Some((*e).to_owned()))).checked(now.as_deref() == Some(*e))));
                rows
            }
            MenuKind::Mode => {
                title = Some(t!("composer.menu_mode").to_owned());
                Mode::ALL.iter().map(|m| Row::new(m.label(), m.short(), Act::Mode(*m)).checked(*m == self.model.mode)).collect()
            }
            MenuKind::Context => {
                let Some(member) = member else { return };
                let used = member.context.unwrap_or(0);
                title = Some(t!("composer.context_title", name = member.name, used = used));
                vec![
                    Row::note(t!("composer.context_explain")),
                    Row::new(t!("composer.context_compact"), "/compact", Act::Refresh(true)).icon(Icon::Compact),
                    Row::new(t!("composer.context_clear"), "/clear", Act::Refresh(false)).icon(Icon::Clear),
                ]
            }
        };
        if rows.is_empty() {
            return;
        }
        let pick = Menu::first_pick(&rows);
        // A menu opening takes the place of one still fading out.
        self.composer.leaving = None;
        self.composer.menu = Some(Menu { kind, title, rows, pick });
        self.composer.choices.clear();
        self.composer.focus(window, cx);
        cx.notify();
    }

    fn run_row(&mut self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(act) = self.composer.menu.as_ref().and_then(|m| m.rows.get(at)).and_then(|r| r.act.clone()) else { return };
        if matches!(act, Act::Recent) {
            // The next menu opens in its place, with no fade between.
            self.composer.menu = None;
        } else {
            self.close_menu(cx);
        }
        let member = self.model.selected_member().map(|m| (m.session, m.name.clone()));
        match act {
            Act::PickFiles => self.pick_files(cx),
            Act::Recent => return self.open_menu(MenuKind::Recent, window, cx),
            Act::Again(path) => self.attach_sources(vec![Source::File(path)], cx),
            Act::Screenshot => self.screenshot(cx),
            Act::Commands => {
                if self.composer.text(cx).is_empty() {
                    self.composer.set_text("/", window, cx);
                    self.composer.refresh_choices(&self.model, cx);
                }
            }
            Act::Select(session) => self.select_member(session, window, cx),
            Act::Model(model) => {
                if let Some((session, name)) = member {
                    self.send_command(Command::SetModel { session, model: model.clone() });
                    self.composer.flash = Some(t!("compose.model_set", name = name, wanted = model));
                }
            }
            Act::Effort(effort) => {
                if let Some((session, name)) = member {
                    self.send_command(Command::SetEffort { session, effort: effort.clone() });
                    self.composer.flash = Some(match &effort {
                        Some(e) => t!("composer.effort_set", name = name, effort = e),
                        None => t!("composer.effort_reset", name = name),
                    });
                    self.composer.efforts.insert(session, effort);
                }
            }
            Act::Mode(mode) => self.send_command(Command::SetMode { squad: self.model.squad.clone(), member: None, mode: Some(mode) }),
            Act::Refresh(summary) => {
                if let Some((session, _)) = member {
                    self.send_command(Command::Refresh { session, summary });
                }
            }
        }
        self.composer.focus(window, cx);
        cx.notify();
    }

    // ── keys ──

    /// Esc, ↑↓←→, Tab, Enter, Backspace and 1–9, before the box's own
    /// bindings see them. True when the key was taken.
    pub fn on_composer_key(&mut self, key: &str, modifiers: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.screen != Screen::Chat || !self.composer.is_focused(window, cx) {
            return false;
        }
        let plain = !modifiers.control && !modifiers.alt && !modifiers.platform && !modifiers.shift;
        // The picture shown big has the arrows and Esc.
        if self.composer.lightbox.is_some() && plain && matches!(key, "left" | "right" | "escape" | "space") {
            match key {
                "left" | "right" => self.step_lightbox(key == "right", cx),
                _ => self.composer.lightbox = None,
            }
            cx.notify();
            return true;
        }
        // Dictating: Esc drops what is being said, the box as it was.
        if key == "escape" && self.composer.dictating.is_some() {
            self.composer.speech.update(cx, |s, cx| s.cancel(cx));
            self.composer.dictating = None;
            cx.notify();
            return true;
        }
        // A menu of the bar is open: it has the keys.
        if let Some(menu) = self.composer.menu.as_mut() {
            match key {
                "escape" => self.close_menu(cx),
                "up" | "down" => menu.step(key == "up"),
                "enter" | "tab" => {
                    let at = menu.pick;
                    self.run_row(at, window, cx);
                }
                digit if plain && digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit()) => {
                    let n: usize = digit.parse().unwrap_or(0);
                    let at = menu.rows.iter().enumerate().filter(|(_, r)| r.act.is_some()).nth(n.saturating_sub(1)).map(|(i, _)| i);
                    if let Some(at) = at {
                        self.run_row(at, window, cx);
                    }
                }
                _ => return false,
            }
            cx.notify();
            return true;
        }
        // An attachment reached with ←: the arrows walk the chips, Backspace removes one.
        if let Some(at) = self.composer.chip {
            let n = self.composer.strip.len();
            match key {
                "left" if plain => self.composer.chip = Some(at.saturating_sub(1)),
                "right" if plain => self.composer.chip = (at + 1 < n).then_some(at + 1),
                "backspace" | "delete" => {
                    if let Some(id) = self.composer.strip.items().get(at).map(|i| i.id) {
                        self.unattach(id, cx);
                    }
                }
                "space" | "enter" if plain => {
                    let id = self.composer.strip.items().get(at).map(|i| i.id);
                    self.show_big(id, cx);
                }
                "escape" => self.composer.chip = None,
                _ => {
                    self.composer.chip = None;
                    return false;
                }
            }
            cx.notify();
            return true;
        }
        let text = self.composer.text(cx);
        match key {
            "escape" if plain => {
                if self.composer.open() {
                    self.composer.dismissed = Some(text);
                    self.composer.choices.clear();
                } else if !self.composer.guard.composing() && text.is_empty() {
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
                // ↑ on an empty box takes back the last memo still waiting.
                if up && text.is_empty() && !self.composer.history.walking() && self.composer.strip.is_empty() {
                    let last = self.model.convo().and_then(|c| c.queued.last().cloned());
                    if let Some(memo) = last.filter(|_| self.model.viewed_presence() == Some(Presence::Working)) {
                        self.edit_memo(memo, window, cx);
                        return true;
                    }
                }
                let said = if up { self.composer.history.older(&text).cloned() } else { self.composer.history.newer() };
                match said {
                    Some(said) => {
                        self.composer.set_text(&said.text, window, cx);
                        self.composer.strip.restore(&said.attachments);
                        cx.notify();
                        true
                    }
                    None => false,
                }
            }
            // → on an empty box takes the greyed suggestion.
            "right" if plain && text.is_empty() && self.composer.suggestion.is_some() => {
                if let Some(guess) = self.composer.suggestion.clone() {
                    self.composer.set_text(&guess, window, cx);
                    cx.notify();
                }
                true
            }
            // ← at the very start reaches the attachments.
            "left" if plain && !self.composer.strip.is_empty() && self.composer.caret(cx) == 0 && self.composer.state.read(cx).selected_range().is_empty() => {
                self.composer.chip = Some(self.composer.strip.len() - 1);
                cx.notify();
                true
            }
            // Backspace at the start of an empty box removes the last attachment.
            "backspace" if plain && text.is_empty() && !self.composer.strip.is_empty() => {
                if let Some(item) = self.composer.strip.items().last().map(|i| i.id) {
                    self.unattach(item, cx);
                }
                true
            }
            // Tab with something written to a member at work: a memo for after.
            "tab" if plain && !self.composer.open() && self.model.viewed_presence() == Some(Presence::Working) && !text.trim().is_empty() && !text.starts_with('/') => {
                self.send_as(true, window, cx);
                true
            }
            "tab" if plain && self.composer.open() => {
                self.take_choice(self.composer.pick, window, cx);
                true
            }
            // Otherwise Tab leaves the box for the buttons, as everywhere else.
            "tab" if !modifiers.control && !modifiers.alt && !modifiers.platform => {
                if modifiers.shift {
                    window.focus_prev(cx);
                } else {
                    window.focus_next(cx);
                }
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
        // A command's name becomes one token: it goes whole, or not at all.
        if let Some(range) = choice.token.clone().filter(|r| choice.text.get(r.clone()).is_some()) {
            let token = InlineToken::new(format!("cmd:{}", &choice.text[range.clone()]), choice.text[range.clone()].to_owned());
            self.composer.state.update(cx, |s, cx| {
                let _ = s.replace_range_with_token(range, token, window, cx);
                let end = choice.text.len();
                s.set_selected_range(end..end, cx);
            });
        }
        self.composer.refresh_choices(&self.model, cx);
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// Show an attachment big (`None`: close). A PDF's first page is drawn
    /// for it off the UI thread, from the file on this Mac.
    pub(crate) fn show_big(&mut self, id: Option<u64>, cx: &mut Context<Self>) {
        self.composer.lightbox = id;
        cx.notify();
        let Some(item) = id.and_then(|id| self.composer.strip.items().iter().find(|i| i.id == id)) else { return };
        if item.kind != Kind::Pdf || self.composer.page.as_ref().is_some_and(|(at, _)| *at == item.id) {
            return;
        }
        let local = !self.endpoint.is_remote();
        let Some(path) = item.origin.clone().or_else(|| match &item.stage {
            Stage::Ready(ready) if local => Some(PathBuf::from(&ready.path)),
            _ => None,
        }) else {
            return;
        };
        let id = item.id;
        let drawn = cx.background_spawn(async move { media::pdf(&path, 1600).ok().and_then(|(_, page)| page).and_then(|page| media::png(&page).ok()) });
        cx.spawn(async move |this, cx| {
            if let Some(png) = drawn.await {
                this.update(cx, |this, cx| {
                    this.composer.page = Some((id, std::sync::Arc::new(Image::from_bytes(ImageFormat::Png, png))));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn step_lightbox(&mut self, forward: bool, cx: &mut Context<Self>) {
        let items = self.composer.strip.items();
        let Some(at) = self.composer.lightbox.and_then(|id| items.iter().position(|i| i.id == id)) else { return };
        let n = items.len();
        let next = if forward { (at + 1) % n } else { (at + n - 1) % n };
        let id = items[next].id;
        self.show_big(Some(id), cx);
    }

    /// The window is being dragged over: say what dropping would do.
    pub(crate) fn drag_moved(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        let paths = paths.paths().to_vec();
        if self.composer.drag.as_ref().is_some_and(|(p, _)| *p == paths) {
            return;
        }
        let room = attach::MAX_ATTACHMENTS.saturating_sub(self.composer.strip.items().iter().filter(|i| i.failed().is_none()).count());
        let preview = attach::preview(&paths, room);
        self.composer.drag = Some((paths, preview));
        cx.notify();
    }
}

/// One step of an attachment on its way, as the UI hears it.
enum Step {
    Shown(Box<attach::Prepared>),
    Share(f32),
    Done(Result<attach::Ready, String>),
}

/// What a `Prepared`'s chip needs, without its bytes.
fn prepared_head(p: &attach::Prepared) -> attach::Prepared {
    attach::Prepared {
        kind: p.kind,
        name: p.name.clone(),
        size: p.size,
        thumb: p.thumb.clone(),
        meta: p.meta.clone(),
        main: attach::Payload::Bytes { name: String::new(), bytes: Vec::new() },
        frames: Vec::new(),
        sheet: None,
    }
}

fn short_dir(dir: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    match dir.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => dir.display().to_string(),
    }
}

// ── drawing ──

/// A button the mouse and the keyboard both reach: Tab focuses it, Enter or
/// Space runs it, and it says what it does to VoiceOver.
fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, run: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static, cx: &mut Context<Shell>) -> Stateful<Div> {
    let run = Rc::new(run);
    let key = run.clone();
    div()
        .id(id)
        .aria_label(label)
        .tab_index(0)
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, window, cx| run(this, window, cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                key(this, window, cx);
                cx.stop_propagation();
            }
        }))
}

/// The composer: what waits, then the card (attachments, the box, the bar),
/// and — only while a member works — what Enter and Tab do now.
pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let composer = &shell.composer;
    v_flex()
        .w_full()
        .max_w(px(720.))
        .flex_none()
        .pb_3()
        // Lined up with the text in the card: its padding and border.
        .children(composer.flash.clone().map(|flash| motion::fade(div().px(px(INSET + 1.)).pb_2().text_size(px(SMALL)).text_color(p.text2).child(flash.clone()), ElementId::Name(format!("flash-{}", crate::render::hash_of(&flash)).into()), motion::BASE)))
        .children(rail(shell, cx))
        .child(card(shell, cx))
        .children(hint_line(shell, cx))
}

/// The card's inner padding: the text and the bar's controls start here.
const INSET: f32 = 12.;

fn card(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let composer = &shell.composer;
    let shell_entity = cx.entity().downgrade();
    let token_color = p.accent;
    let token_bg = p.accent_bg;
    v_flex()
        .id("composer")
        .key_context("Composer")
        .relative()
        .rounded(px(R_XL))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .shadow_sm()
        .on_action(cx.listener(|this, _: &OpenAttach, window, cx| this.open_menu(MenuKind::Attach, window, cx)))
        .on_action(cx.listener(|this, _: &StopMember, _, cx| this.interrupt(cx)))
        .on_action(cx.listener(|this, _: &OpenModels, window, cx| this.open_menu(MenuKind::Model, window, cx)))
        .on_action(cx.listener(|this, _: &OpenEffort, window, cx| this.open_menu(MenuKind::Effort, window, cx)))
        .on_action(cx.listener(|this, _: &OpenModes, window, cx| this.open_menu(MenuKind::Mode, window, cx)))
        .on_action(cx.listener(|this, _: &PasteInline, window, cx| {
            this.composer.inline_paste = true;
            window.dispatch_action(Box::new(Paste), cx);
        }))
        .on_action(cx.listener(|this, _: &SendInverted, window, cx| this.send_inverted(window, cx)))
        .children(completion(shell, cx))
        .children(menu_layer(shell, cx))
        .children(strip(shell, cx))
        .child(
            motion::SmoothHeight::new(
                "composer-text",
                // The text box pads its text itself (GPUI Kit's medium input
                // padding): take that off, so the text starts at INSET too.
                div()
                    .px(px(INSET) - gpui_kit::component::Size::Medium.input_px())
                    .pt(px(INSET) - gpui_kit::component::Size::Medium.input_py())
                    .child(
                        Textarea::new(&composer.state)
                            .appearance(false)
                            .bordered(false)
                            .text_size(px(crate::theme::BODY))
                            .line_height(px(24.))
                            .aria_label(composer.placeholder.clone())
                            .token(move |token, _, _| {
                                div()
                                    .px_1()
                                    .rounded(px(5.))
                                    .bg(token_bg)
                                    .text_color(token_color)
                                    .font_family(MONO)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(token.token().label().clone())
                            })
                            .on_paste(move |item, _, cx| shell_entity.update(cx, |shell, cx| shell.paste_item(item, cx)).unwrap_or(false)),
                    )
                    .into_any_element(),
            ),
        )
        .children(composer.dictating.clone().map(|heard| {
            h_flex()
                .px(px(INSET))
                .pb_1()
                .gap_2()
                .items_center()
                .text_size(px(crate::theme::BODY))
                .line_height(px(24.))
                .child(div().size(px(8.)).flex_none().rounded_full().bg(p.danger()).with_animation("listening", Animation::new(Duration::from_millis(1200)).repeat().with_easing(pulsating_between(0.35, 1.)), |d, t| d.opacity(t)))
                .child(div().min_w_0().text_color(p.muted).italic().child(if heard.is_empty() { t!("composer.dictation_listening").to_owned() } else { heard }))
        }))
        .child(bar(shell, cx))
}

/// The list `/` and `@` open above the card: rows grouped, letters that
/// matched in bold, a word on what each does, and where it comes from.
fn completion(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let p = shell.palette;
    let composer = &shell.composer;
    if !composer.open() || composer.menu.is_some() {
        return None;
    }
    let mut rows: Vec<AnyElement> = Vec::new();
    let mut group = None;
    for (ix, choice) in composer.choices.iter().take(ROWS * 2).enumerate() {
        if choice.group.is_some() && choice.group != group {
            group = choice.group;
            rows.push(div().h(px(28.)).px_2().flex().items_end().pb_1().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(choice.group.unwrap_or_default()).into_any_element());
        }
        let on = ix == composer.pick;
        let row = h_flex()
            .id(("choice", ix))
            .h(px(ROW))
            .px_2()
            .gap_3()
            .items_center()
            .rounded(px(R_SM))
            .cursor_pointer()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.composer.pick != ix {
                    this.composer.pick = ix;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.take_choice(ix, window, cx)))
            .child(highlighted(p, &choice.label, &choice.hits).flex_none().max_w(px(320.)).overflow_hidden())
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(SMALL)).text_color(p.muted).child(choice.about.clone()))
            .children(choice.badge.clone().map(|badge| div().flex_none().px_2().h(px(18.)).flex().items_center().rounded(px(9.)).bg(p.wash).text_size(px(10.5)).text_color(p.muted).child(badge)));
        rows.push(motion::hover_fill(row, ("choice-fill", ix), p.wash, on).into_any_element());
    }
    // The list sits over the card's top edge; the surface inside it moves.
    let surface = v_flex()
        .id("completion")
        .p_1()
        .max_h(px(ROWS as f32 * ROW + 48.))
        .overflow_y_scroll()
        .rounded(px(R_LG))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .children(rows);
    Some(div().absolute().left_0().right_0().bottom(relative(1.)).mb(px(GAP)).child(motion::pop(surface, "completion-in", true)))
}

/// A label with the matched letters bright and bold, the rest quieter.
fn highlighted(p: Palette, label: &str, hits: &[usize]) -> Div {
    let mut row = h_flex().font_family(MONO).text_size(px(13.)).whitespace_nowrap();
    let mut run = String::new();
    let mut run_hit = false;
    let flush = |row: Div, run: &mut String, hit: bool| {
        if run.is_empty() {
            return row;
        }
        let text = std::mem::take(run);
        row.child(div().text_color(if hit { p.accent } else { p.text2 }).when(hit, |d| d.font_weight(FontWeight::BOLD)).child(text))
    };
    for (i, c) in label.chars().enumerate() {
        let hit = hits.contains(&i);
        if hit != run_hit {
            row = flush(row, &mut run, run_hit);
            run_hit = hit;
        }
        run.push(c);
    }
    flush(row, &mut run, run_hit)
}

/// The bar's menu, open over its control — and the one just closed, fading.
fn menu_layer(shell: &Shell, cx: &mut Context<Shell>) -> Vec<AnyElement> {
    let mut layers = Vec::new();
    if let Some((menu, n)) = &shell.composer.leaving {
        layers.extend(menu_popup(shell, menu, Some(*n), cx));
    }
    if let Some(menu) = &shell.composer.menu {
        layers.extend(menu_popup(shell, menu, None, cx));
    }
    layers
}

/// A menu of the bar, anchored to the control that opened it: directly above
/// it (below when there's no room), flush with its edge, `GAP` away. A leading
/// column for icons, the label, the shortcut or detail on the right in a
/// quieter tone, a chevron for a menu that opens another. `leaving`: the
/// menu has closed and fades out, taking no clicks.
fn menu_popup(shell: &Shell, menu: &Menu, leaving: Option<u64>, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let anchor = *shell.composer.anchors.borrow().get(&menu.kind.trigger())?;
    let live = leaving.is_none();
    let rows: Vec<AnyElement> = menu
        .rows
        .iter()
        .enumerate()
        .map(|(ix, row)| {
            let base = h_flex().min_h(px(ROW)).px_2().gap_2().items_center().rounded(px(R_SM));
            match &row.act {
                None => base.py_1().child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(row.label.clone())).into_any_element(),
                Some(_) => {
                    let picked = ix == menu.pick;
                    let lead = match row.lead {
                        Lead::Icon(name) => icon(name, ICON, p.text2).into_any_element(),
                        Lead::Mark(mark, hue) => div().text_size(px(12.)).text_color(p.member(hue)).child(mark).into_any_element(),
                        Lead::None if row.on => icon(Icon::Check, ICON, p.accent).into_any_element(),
                        Lead::None => div().into_any_element(),
                    };
                    let trail: Option<AnyElement> = match (row.keys, row.on && row.lead != Lead::None, row.more) {
                        (_, _, true) => Some(icon(Icon::ChevronRight, ICON_SM, p.muted).into_any_element()),
                        (_, true, _) => Some(icon(Icon::Check, ICON_SM, p.accent).into_any_element()),
                        (Some(keys), _, _) => Some(div().text_size(px(12.)).text_color(p.muted).child(keys).into_any_element()),
                        _ => None,
                    };
                    let row_el = base
                        .id(("menu-row", ix))
                        .when(live, |r| {
                            r.cursor_pointer()
                                // The pointer moves the highlight, as the arrows do.
                                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                    if let Some(menu) = this.composer.menu.as_mut()
                                        && *hovered
                                        && menu.pick != ix
                                    {
                                        menu.pick = ix;
                                        cx.notify();
                                    }
                                }))
                                .on_click(cx.listener(move |this, _, window, cx| this.run_row(ix, window, cx)))
                        })
                        .child(div().w(px(ICON)).flex_none().flex().justify_center().child(lead))
                        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(13.)).text_color(p.text).child(row.label.clone()))
                        .when(!row.hint.is_empty(), |r| r.child(div().flex_none().max_w(px(160.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.)).text_color(p.muted).child(row.hint.clone())))
                        .children(trail.map(|t| div().flex_none().flex().items_center().child(t)));
                    motion::hover_fill(row_el, ("menu-row-fill", ix), p.wash, picked).into_any_element()
                }
            }
        })
        .collect();
    let kind = menu.kind;
    let surface = v_flex()
        .id(if live { "bar-menu" } else { "bar-menu-leaving" })
        .w(px(kind.width()))
        .p_1()
        .rounded(px(R_MD))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .text_color(p.text)
        .children(menu.title.clone().map(|title| div().h(px(28.)).px_2().flex().items_center().text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(title)))
        .children(rows)
        .when(live, |s| {
            // A click anywhere else closes it — but not on its own control,
            // whose click does that already.
            s.on_mouse_down_out(cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                if !anchor.contains(&e.position) {
                    this.close_menu(cx);
                }
            }))
        });
    let surface: AnyElement = match leaving {
        None => motion::pop(surface, ("menu-in", kind as u64), true).into_any_element(),
        Some(n) => surface.shadow(motion::float_shadow(0.)).with_animation(("menu-out", n), Animation::new(motion::FAST).with_easing(motion::ease_out), |s, t| s.opacity(1. - t)).into_any_element(),
    };
    let positioner = gpui_kit::base::Positioner::side(anchor).placement(gpui_kit::base::Placement::Top).align(kind.align()).offset(px(GAP)).margin(px(8.));
    let positioner = if live { positioner.occlude() } else { positioner };
    Some(deferred(positioner.child(surface)).with_priority(2).into_any_element())
}

/// Remember where a control that opens a menu is, for the menu to open there.
fn anchored<E: ParentElement>(el: E, anchors: &Rc<std::cell::RefCell<std::collections::HashMap<MenuKind, Bounds<Pixels>>>>, kind: MenuKind) -> E {
    use gpui_kit::base::ElementExt as _;
    let anchors = anchors.clone();
    el.on_prepaint(move |bounds, _, _| {
        anchors.borrow_mut().insert(kind, bounds);
    })
}

/// A square button with an icon, a fill easing in under the pointer, and a
/// tooltip: its name and its shortcut.
fn icon_button(shell: &Shell, id: &'static str, glyph: Icon, label: &'static str, keys: Option<&'static str>, run: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static, cx: &mut Context<Shell>) -> Stateful<Div> {
    let p = shell.palette;
    tip(
        button(id, label, run, cx).size(px(CONTROL)).flex_none().flex().items_center().justify_center().rounded(px(R_MD)).child(icon(glyph, ICON, p.text2)),
        p,
        label,
        keys,
    )
}

/// The bar under the box. Left: "+", dictation, who it is for, the mode.
/// Right: how full the context is, the model, the effort, and Send — or Stop
/// while the member works. Every control is `CONTROL` tall, `GAP`-ish apart.
fn bar(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let composer = &shell.composer;
    let model = &shell.model;
    let member = model.selected_member();
    let working = model.viewed_presence() == Some(Presence::Working);
    let stopping = working && model.viewed().is_some_and(|s| composer.stopping == Some(s));
    let text = composer.text(cx);
    let has_words = !text.trim().is_empty();
    let has_any = has_words || composer.strip.ready() > 0;
    let steerable = member.is_some_and(|m| m.engine == Some(Engine::Claude));
    let (to_label, to_color) = recipient(p, model, &text);
    let project = model.focus.clone().filter(|_| member.is_some_and(Model::is_pm));
    let anchors = composer.anchors.clone();
    let open = |kind: MenuKind| composer.menu.as_ref().is_some_and(|m| m.kind.trigger() == kind);

    // "+" wears a hairline, as Claude's does: its edge is where the text above starts.
    let plus_open = open(MenuKind::Attach);
    let plus = anchored(icon_button(shell, "attach", Icon::Plus, t!("composer.tip_attach"), Some("⌘U"), |this, window, cx| this.open_menu(MenuKind::Attach, window, cx), cx), &anchors, MenuKind::Attach)
        .border_1()
        .border_color(p.hair);
    let plus = motion::hover_fill(plus, "attach-fill", p.wash, plus_open);

    let listening = composer.dictating.is_some();
    let mic = composer.speech.read(cx).is_available(cx).then(|| {
        let label = if listening { t!("composer.tip_mic_stop") } else { t!("composer.tip_mic") };
        let b = tip(
            button("mic", label, |this, _, cx| this.toggle_dictation(cx), cx)
                .size(px(CONTROL))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(R_MD))
                .when(listening, |b| b.bg(p.danger_bg()))
                .child(icon(Icon::Mic, ICON, if listening { p.danger() } else { p.text2 })),
            p,
            label,
            None,
        );
        motion::hover_fill(b, "mic-fill", p.wash, false)
    });

    // A picker: what it is set to, and a chevron.
    let picker = |id: &'static str, aria: String, label: String, color: Rgba, kind: MenuKind, tip_label: Option<(&'static str, &'static str)>, cx: &mut Context<Shell>| {
        let is_open = open(kind);
        let b = button(id, aria, move |this, window, cx| this.open_menu(kind, window, cx), cx)
            .h(px(CONTROL))
            .pl_2()
            .pr(px(6.))
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .rounded(px(R_MD))
            .text_size(px(SMALL))
            .text_color(color)
            .whitespace_nowrap()
            .child(label)
            .child(icon(Icon::ChevronDown, ICON_SM, p.muted));
        let b = anchored(b, &anchors, kind);
        let b = match tip_label {
            Some((label, keys)) => tip(b, p, label, Some(keys)),
            None => b,
        };
        motion::hover_fill(b, (id, 1usize), p.wash, is_open)
    };

    let to = picker(
        "to",
        t!("composer.aria_to", name = to_label.clone()),
        match &project {
            Some(project) => format!("{to_label} · {project}"),
            None => to_label.clone(),
        },
        to_color,
        MenuKind::To,
        None,
        cx,
    );
    let mode_pill = member.map(|_| picker("mode", t!("composer.aria_mode", mode = model.mode.label()), model.mode.label().to_owned(), p.muted, MenuKind::Mode, Some((t!("composer.tip_mode"), "⌘⇧M")), cx));
    let model_pill = member.and_then(|m| m.model.clone()).map(|m| picker("model", t!("composer.aria_model", model = m.clone()), short_model(&m), p.text2, MenuKind::Model, Some((t!("composer.tip_model"), "⌘⇧I")), cx));
    let effort_pill = member.map(|m| {
        let now = composer.efforts.get(&m.session).cloned().flatten();
        let label = now.clone().unwrap_or_else(|| t!("composer.effort_short").to_owned());
        picker("effort", t!("composer.aria_effort", effort = label.clone()), label, if now.is_some() { p.text2 } else { p.muted }, MenuKind::Effort, Some((t!("composer.tip_effort"), "⌘⇧E")), cx)
    });
    let ring = member.and_then(|m| m.context).map(|used| {
        let is_open = open(MenuKind::Context);
        let b = button("context", t!("composer.aria_context", used = used), |this, window, cx| this.open_menu(MenuKind::Context, window, cx), cx)
            .h(px(CONTROL))
            .px_2()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(R_MD))
            .child(context_ring(p, used))
            .child(div().text_size(px(12.)).text_color(if used >= 80 { p.danger() } else { p.muted }).child(format!("{used}%")));
        let b = tip(anchored(b, &anchors, MenuKind::Context), p, t!("composer.tip_context"), None);
        motion::hover_fill(b, "context-fill", p.wash, is_open)
    });

    // Send: a filled square with an arrow, quiet until there is something to send.
    let send_button = |id: &'static str, glyph: Icon, label: &'static str, keys: Option<&'static str>, enabled: bool, run: fn(&mut Shell, &mut Window, &mut Context<Shell>), cx: &mut Context<Shell>| {
        let b = button(id, label, move |this, window, cx| if enabled { run(this, window, cx) }, cx)
            .size(px(CONTROL))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(R_MD))
            .bg(if enabled { p.primary } else { p.wash })
            .when(!enabled, |b| b.cursor_default())
            .child(icon(glyph, ICON, if enabled { p.on_primary } else { p.muted }));
        tip(b, p, label, keys)
    };
    let quiet = |id: &'static str, label: String, aria: String, tip_label: &'static str, keys: &'static str, run: fn(&mut Shell, &mut Window, &mut Context<Shell>), cx: &mut Context<Shell>| {
        let b = button(id, aria, run, cx)
            .h(px(CONTROL))
            .px_3()
            .flex_none()
            .flex()
            .items_center()
            .rounded(px(R_MD))
            .text_size(px(SMALL))
            .border_1()
            .border_color(p.hair)
            .text_color(p.text)
            .child(label);
        motion::hover_fill(tip(b, p, tip_label, Some(keys)), (id, 1usize), p.wash, false)
    };
    let stop = |cx: &mut Context<Shell>| {
        let label = if stopping { t!("composer.stopping") } else { t!("composer.tip_stop") };
        let b = button("stop", t!("composer.aria_stop"), |this, _, cx| this.interrupt(cx), cx)
            .size(px(CONTROL))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(R_MD))
            .border_1()
            .border_color(p.hair)
            .child(div().size(px(10.)).rounded(px(2.)).bg(if stopping { p.muted } else { p.text }));
        motion::hover_fill(tip(b, p, label, Some("⌘.")), "stop-fill", p.wash, false)
    };

    let mut right: Vec<AnyElement> = Vec::new();
    right.extend(ring.map(IntoElement::into_any_element));
    right.extend(model_pill.map(IntoElement::into_any_element));
    right.extend(effort_pill.map(IntoElement::into_any_element));
    if working && has_words && steerable {
        right.push(quiet("memo", t!("composer.memo").into(), t!("composer.aria_memo").into(), t!("composer.tip_memo"), "⇥", |this, window, cx| this.send_as(true, window, cx), cx).into_any_element());
    }
    if working && has_any {
        // Words for a member at work: Send steers (or leaves a memo); Stop stays in reach.
        right.push(stop(cx).into_any_element());
        let label = if steerable { t!("composer.tip_steer") } else { t!("composer.tip_memo") };
        right.push(send_button("send", Icon::ArrowUp, label, Some("⏎"), !composer.strip.busy(), |this, window, cx| this.send(window, cx), cx).into_any_element());
    } else if working {
        right.push(stop(cx).into_any_element());
    } else {
        let can = has_any && !composer.strip.busy();
        right.push(send_button("send", Icon::ArrowUp, t!("composer.tip_send"), Some("⏎"), can, |this, window, cx| this.send(window, cx), cx).into_any_element());
    }

    h_flex()
        .px(px(INSET))
        .pt_1()
        .pb(px(INSET - 2.))
        .gap_1()
        .items_center()
        .child(plus)
        .children(mic)
        .child(h_flex().flex_1().min_w_0().overflow_hidden().gap_1().items_center().child(to).children(mode_pill))
        .child(h_flex().flex_none().gap_1().items_center().children(right))
}

/// How full the context is, as a ring: 14 points, the used part bright.
fn context_ring(p: Palette, used: u8) -> impl IntoElement {
    let color = if used >= 80 { p.danger() } else { p.text2 };
    let track = p.hair;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let r = bounds.size.width.min(bounds.size.height) / 2. - px(1.5);
            let center = bounds.center();
            let arc = |from: f32, to: f32| {
                let mut path = PathBuilder::stroke(px(2.));
                let steps = 48;
                for i in 0..=steps {
                    let a = from + (to - from) * i as f32 / steps as f32;
                    let angle = (a * 360. - 90.).to_radians();
                    let at = point(center.x + r * angle.cos(), center.y + r * angle.sin());
                    if i == 0 { path.move_to(at) } else { path.line_to(at) }
                }
                path.build().ok()
            };
            if let Some(path) = arc(0., 1.) {
                window.paint_path(path, track);
            }
            let share = f32::from(used.min(100)) / 100.;
            if share > 0. && let Some(path) = arc(0., share) {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(ICON_SM))
    .flex_none()
}

/// Under the card while a member works: what Enter and Tab do now, lined up
/// with the text inside the card. Nothing when nobody works — the controls'
/// tooltips say their keys.
fn hint_line(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let p = shell.palette;
    let model = &shell.model;
    let text = shell.composer.text(cx);
    let working = model.viewed_presence() == Some(Presence::Working);
    if !working {
        return None;
    }
    let steerable = model.selected_member().is_some_and(|m| m.engine == Some(Engine::Claude));
    let says = !text.trim().is_empty() && !text.starts_with('/');
    let hint = match (says, steerable) {
        (true, true) => t!("composer.hint_steer"),
        (true, false) => t!("composer.hint_memo"),
        (false, _) => t!("composer.hint_stop"),
    };
    Some(motion::fade(h_flex().h(px(24.)).px(px(INSET + 1.)).items_center().text_size(px(11.5)).text_color(p.muted).child(hint), "hint-in", motion::BASE))
}

/// Who the line goes to, as the bar shows it: always someone, never a guess.
fn recipient(p: Palette, model: &Model, text: &str) -> (String, Rgba) {
    let name_of = |m: &crate::state::Member| {
        let (mark, hue) = look(&m.role, &m.name);
        (format!("{mark} {}", m.name), p.member(hue))
    };
    if text.starts_with('/') {
        return (t!("composer.command").into(), p.accent);
    }
    if let Some(rest) = text.strip_prefix('@') {
        let name = rest.split(char::is_whitespace).next().unwrap_or("");
        if let Some(m) = model.open_member(name) {
            return name_of(m);
        }
    }
    // A ticket open whose member isn't the one in view: it goes to the PM.
    if let (Some(ticket), Some(selected)) = (model.opened, model.selected_member())
        && selected.ticket != Some(ticket)
        && let Some(pm) = model.members.iter().find(|m| Model::is_pm(m) && m.presence != Presence::Closed)
    {
        return name_of(pm);
    }
    model.selected_member().map_or(("—".into(), p.muted), name_of)
}

// ── the strip of attachments ──

/// What is attached, above the text inside the card: pictures as 72-point
/// thumbnails with their number, other files as chips with their kind, name
/// and what they hold — each with a ✕ the keyboard reaches too, how far it
/// got, and (in red, until dismissed) why one couldn't be attached.
fn strip(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let strip = &shell.composer.strip;
    if strip.is_empty() {
        return None;
    }
    let p = shell.palette;
    // A chip pops into place when it arrives.
    let chips: Vec<AnyElement> = strip.items().iter().enumerate().map(|(n, item)| motion::rise(div().flex_none().child(chip(shell, item, n, cx)), ("chip-in", item.id)).into_any_element()).collect();
    let failed = strip.failed();
    let count = strip.len() - failed;
    let summary = h_flex()
        .px(px(INSET))
        .pt_2()
        .gap_2()
        .items_center()
        .text_size(px(11.5))
        .text_color(p.muted)
        .when(count > 0, |r| r.child(t!("composer.strip_count", count = count, max = attach::MAX_ATTACHMENTS)))
        .when(failed >= 2, |r| {
            r.child(div().text_color(p.danger()).child(t!("composer.failed_summary", n = failed)))
                .child(button("dismiss-failed", t!("composer.dismiss_failed"), |this, _, cx| {
                    this.composer.strip.dismiss_failed();
                    cx.notify();
                }, cx)
                .text_color(p.text2)
                .underline()
                .child(t!("composer.dismiss_failed")))
        });
    Some(
        v_flex()
            .child(h_flex().px(px(INSET)).pt(px(INSET)).gap_2().flex_wrap().items_start().children(chips))
            .child(summary),
    )
}

/// The ✕ on a chip.
fn remove_button(p: Palette, id: u64, name: &str, cx: &mut Context<Shell>) -> Stateful<Div> {
    let b = button(("unattach", id), t!("composer.aria_remove", name = name), move |this, _, cx| this.unattach(id, cx), cx)
        .size(px(20.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .hover(move |s| s.bg(p.wash))
        .child(icon(Icon::X, 12., p.text2));
    tip(b, p, t!("composer.tip_remove"), None)
}

/// What a chip says under its name.
fn chip_detail(item: &attach::Item) -> String {
    match &item.stage {
        Stage::Preparing => match item.kind {
            Kind::Video => t!("composer.reading_video").into(),
            _ => t!("composer.preparing").into(),
        },
        Stage::Sending(share) => t!("composer.sending", percent = (share * 100.).round() as u32),
        // The chip names the file already.
        Stage::Failed(why) => why.strip_prefix(&format!("{} — ", item.name)).unwrap_or(why).to_owned(),
        Stage::Ready(_) => {
            let size = attach::size_label(item.size);
            let m = &item.meta;
            match item.kind {
                Kind::Pdf => m.pages.map_or(size.clone(), |n| t!("composer.pdf_pages", size = size, pages = n)),
                Kind::Text => m.lines.map_or(size.clone(), |n| t!("composer.text_lines", size = size, lines = n)),
                Kind::Video => match m.duration {
                    Some(d) if m.frames > 0 => t!("composer.video_frames", duration = media::clock(d), frames = m.frames),
                    Some(d) => format!("{} · {size}", media::clock(d)),
                    None => size,
                },
                Kind::Audio => m.duration.map_or(size.clone(), |d| format!("{} · {size}", media::clock(d))),
                _ => size,
            }
        }
    }
}

fn kind_color(p: Palette, kind: Kind) -> Rgba {
    match kind {
        Kind::Pdf => p.member(6),
        Kind::Text => p.member(3),
        Kind::Doc => p.member(2),
        Kind::Video => p.member(4),
        Kind::Audio => p.member(1),
        _ => p.muted,
    }
}

fn chip(shell: &Shell, item: &attach::Item, n: usize, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = item.id;
    let focused = shell.composer.chip == Some(n);
    let failed = item.failed().is_some();
    let share = match item.stage {
        Stage::Sending(s) => Some(s),
        Stage::Preparing => Some(0.),
        _ => None,
    };
    let progress = share.map(|s| {
        div()
            .absolute()
            .left_0()
            .bottom_0()
            .h(px(2.))
            .w(relative(s.max(0.04)))
            .bg(p.accent)
    });
    // A picture: its thumbnail, 72 points, with its number.
    if item.kind == Kind::Image && !failed {
        let face = match &item.thumb {
            Some(thumb) => img(thumb.clone()).size_full().object_fit(ObjectFit::Cover).into_any_element(),
            None => div().size_full().flex().items_center().justify_center().text_size(px(11.)).text_color(p.muted).child(t!("composer.preparing")).into_any_element(),
        };
        let number = shell.composer.strip.picture_number(id).unwrap_or(n + 1);
        // Beside file chips a picture is their height; among pictures only, 72 points.
        let side = if shell.composer.strip.items().iter().all(|i| i.kind == Kind::Image && i.failed().is_none()) { 72. } else { 52. };
        return div()
            .id(("chip", id))
            .relative()
            .flex_none()
            .size(px(side))
            .child(
                div()
                    .id(("thumb", id))
                    .size_full()
                    .rounded(px(R_LG))
                    .overflow_hidden()
                    .bg(p.wash)
                    .border_1()
                    .border_color(if focused { p.accent } else { p.hair })
                    .when(focused, |d| d.border_2())
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.show_big(Some(id), cx)))
                    .child(face)
                    .children(progress),
            )
            .child(div().absolute().left(px(5.)).bottom(px(4.)).px_1().rounded(px(4.)).bg(rgba(0x0000008C)).text_size(px(10.)).text_color(rgb(0xFFFFFF)).child(if side > 60. { t!("composer.picture_number", n = number) } else { number.to_string() }))
            .child(remove_button(p, id, &item.name, cx).absolute().top(px(-6.)).right(px(-6.)))
            .into_any_element();
    }
    // Any other file: a chip with its kind, name and what it holds.
    let icon = match (&item.thumb, failed) {
        (Some(thumb), false) => div()
            .relative()
            .size(px(36.))
            .flex_none()
            .rounded(px(R_SM))
            .overflow_hidden()
            .bg(p.wash)
            .child(img(thumb.clone()).size_full().object_fit(ObjectFit::Cover))
            .when(item.kind == Kind::Video, |d| d.child(div().absolute().inset_0().flex().items_center().justify_center().text_size(px(12.)).text_color(rgb(0xFFFFFF)).child("▶")))
            .into_any_element(),
        _ => div()
            .size(px(36.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(R_SM))
            .bg(if failed { p.danger_bg() } else { p.wash })
            .text_size(px(9.5))
            .font_weight(FontWeight::BOLD)
            .text_color(if failed { p.danger() } else { kind_color(p, item.kind) })
            .child(if failed { "!".to_owned() } else { attach::badge(&item.name, item.kind) })
            .into_any_element(),
    };
    let detail = chip_detail(item);
    let opens = matches!(item.stage, Stage::Ready(_));
    div()
        .id(("chip", id))
        .relative()
        .flex_none()
        .w(px(224.))
        .h(px(52.))
        .child(
            h_flex()
                .id(("chip-face", id))
                .size_full()
                .pl(px(8.))
                .pr(px(28.))
                .gap_2()
                .items_center()
                .rounded(px(R_LG))
                .overflow_hidden()
                .bg(if failed { p.danger_bg() } else { p.bg })
                .border_1()
                .border_color(if failed { p.danger() } else if focused { p.accent } else { p.hair })
                .when(focused, |d| d.border_2())
                .when(opens, |d| {
                    d.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.show_big(Some(id), cx)))
                })
                .child(icon)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.5)).line_height(px(17.)).text_color(p.text).child(if item.kind == Kind::Text && item.origin.is_none() { t!("composer.pasted_label").to_owned() } else { item.name.clone() }))
                        .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(11.)).line_height(px(15.)).text_color(if failed { p.danger() } else { p.muted }).child(detail)),
                )
                .children(progress),
        )
        .child(remove_button(p, id, &item.name, cx).absolute().top(px(16.)).right(px(5.)))
        .into_any_element()
}

// ── what waits for a member at work ──

/// What a button does.
type Run = dyn Fn(&mut Shell, &mut Window, &mut Context<Shell>);

/// Above the card while a member works: one card per memo it will get when
/// its turn ends (편집 · 삭제 · 지금 지시) and per steer it hasn't read yet
/// (grey, until it does).
fn rail(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let p = shell.palette;
    let model = &shell.model;
    let convo = model.convo()?;
    if convo.queued.is_empty() && convo.steering.is_empty() {
        return None;
    }
    let steerable = model.selected_member().is_some_and(|m| m.engine == Some(Engine::Claude));
    let working = model.viewed_presence() == Some(Presence::Working);
    let link = |id: (&'static str, usize), label: &'static str, run: Rc<Run>, cx: &mut Context<Shell>| {
        let b = button(id, label, move |this, window, cx| run(this, window, cx), cx).px_2().h(px(24.)).flex().items_center().rounded(px(R_SM)).text_size(px(12.)).text_color(p.text2).child(label);
        motion::hover_fill(b, (id.0, id.1 + 1000), p.wash, false)
    };
    let badge = |label: &'static str, fg: Rgba, bg: Rgba| {
        div().flex_none().h(px(20.)).px_1p5().flex().items_center().rounded(px(R_SM)).bg(bg).text_size(px(11.)).text_color(fg).child(label)
    };
    let mut cards: Vec<AnyElement> = Vec::new();
    for (i, memo) in convo.queued.iter().enumerate().take(4) {
        let (edit, drop_it, now) = (memo.clone(), memo.clone(), memo.clone());
        let card = h_flex()
            .h(px(36.))
            .pl(px(INSET))
            .pr_1()
            .gap_2()
            .items_center()
            .rounded(px(R_LG))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .child(badge(t!("composer.rail_memo"), p.text2, p.wash))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(SMALL)).text_color(p.text2).child(memo.lines().next().unwrap_or_default().to_owned()))
            .child(link(("memo-edit", i), t!("composer.rail_edit"), Rc::new(move |this, window, cx| this.edit_memo(edit.clone(), window, cx)), cx))
            .child(link(("memo-drop", i), t!("composer.rail_delete"), Rc::new(move |this, _, cx| {
                this.unqueue(&drop_it);
                cx.notify();
            }), cx))
            .when(steerable && working, |c| c.child(link(("memo-now", i), t!("composer.rail_now"), Rc::new(move |this, _, cx| this.steer_now(now.clone(), cx)), cx)));
        cards.push(motion::rise(card, ("memo-in", crate::render::hash_of(memo))).into_any_element());
    }
    for (i, steer) in convo.steering.iter().enumerate().take(3) {
        let card = h_flex()
            .id(("steer", i))
            .h(px(36.))
            .px(px(INSET))
            .gap_2()
            .items_center()
            .rounded(px(R_LG))
            .border_1()
            .border_dashed()
            .border_color(p.hair)
            .child(badge(t!("composer.rail_steer"), p.accent, p.accent_bg))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(SMALL)).text_color(p.muted).child(steer.lines().next().unwrap_or_default().to_owned()))
            .child(div().flex_none().text_size(px(12.)).text_color(p.muted).child(t!("composer.rail_unread")));
        cards.push(motion::rise(card, ("steer-in", crate::render::hash_of(steer))).into_any_element());
    }
    let more = convo.queued.len().saturating_sub(4) + convo.steering.len().saturating_sub(3);
    Some(
        v_flex()
            .gap_1()
            .pb_2()
            .children(cards)
            .when(more > 0, |r| r.child(div().px(px(INSET + 1.)).text_size(px(12.)).text_color(p.muted).child(t!("composer.rail_more", n = more)))),

    )
}

// ── over the window ──

/// Over the whole window while files are dragged in: a dashed frame that
/// says what dropping will attach — and what it won't, and why — before the drop.
pub fn drop_overlay(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    if shell.screen != Screen::Chat || shell.overlay_open() {
        return None;
    }
    let p = shell.palette;
    let preview = shell.composer.drag.as_ref().map(|(_, preview)| preview.clone()).unwrap_or_default();
    let kinds: Vec<String> = preview.taken.iter().map(|(kind, n)| kind_word(*kind, *n)).collect();
    let title = if preview.taken.is_empty() { t!("composer.drop_nothing").to_owned() } else { t!("composer.drop_title_kinds", kinds = kinds.join(" · ")) };
    let refused = preview.refused.iter().take(3).map(|(name, why)| {
        h_flex()
            .gap_2()
            .max_w(px(520.))
            .text_size(px(SMALL))
            .child(div().flex_none().text_color(p.danger()).child("✕"))
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(p.text2).child(if why.contains(name.as_str()) { why.clone() } else { format!("{name} — {why}") }))
    });
    Some(
        div()
            .id("drop-overlay")
            .absolute()
            .inset_0()
            .opacity(if shell.composer.drag_shown { 1. } else { 0. })
            .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(1.))
            .on_drag_move::<ExternalPaths>(cx.listener(|this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                let paths = event.drag(cx).clone();
                this.drag_moved(&paths, cx);
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| this.dropped(paths, window, cx)))
            .child(
                div()
                    .absolute()
                    .inset(px(8.))
                    .rounded(px(R_XL))
                    .border_2()
                    .border_dashed()
                    .border_color(p.accent)
                    .bg(Rgba { a: 0.9, ..p.bg })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        v_flex()
                            .max_w(px(560.))
                            .px_8()
                            .py_6()
                            .gap_2()
                            .items_center()
                            .rounded(px(16.))
                            .bg(p.raised)
                            .border_1()
                            .border_color(p.hair)
                            .shadow_lg()
                            .child(div().text_size(px(26.)).text_color(p.accent).child("＋"))
                            .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).text_center().child(title))
                            .children(refused)
                            .when(preview.refused.len() > 3, |d| d.child(div().text_size(px(SMALL)).text_color(p.muted).child(t!("composer.drop_more_refused", n = preview.refused.len() - 3))))
                            .when(preview.folders > 0, |d| d.child(div().text_size(px(SMALL)).text_color(p.text2).child(t!("composer.drop_folders", n = preview.folders))))
                            .when(preview.over > 0, |d| d.child(div().text_size(px(SMALL)).text_color(p.danger()).child(t!("composer.drop_over", n = preview.over, max = attach::MAX_ATTACHMENTS))))
                            .child(div().pt_1().text_size(px(12.)).text_color(p.muted).text_center().child(t!("composer.drop_limits", n = attach::MAX_ATTACHMENTS, mb = attach::MAX_BYTES >> 20))),
                    ),
            ),
    )
}

/// `사진 2`, `PDF 1`, …
fn kind_word(kind: Kind, n: usize) -> String {
    let word = match kind {
        Kind::Image => t!("composer.kind_image"),
        Kind::Video => t!("composer.kind_video"),
        Kind::Audio => t!("composer.kind_audio"),
        Kind::Pdf => "PDF",
        Kind::Text => t!("composer.kind_text"),
        Kind::Doc => t!("composer.kind_doc"),
        Kind::File => t!("composer.kind_file"),
    };
    format!("{word} {n}")
}

/// An attachment shown big over the window (a click on its chip): a picture
/// whole, a video's frames with what it is and "QuickTime에서 열기", a PDF's
/// first page. ←/→ go through the others, Esc closes.
pub fn lightbox(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let id = shell.composer.lightbox?;
    let items = shell.composer.strip.items();
    let at = items.iter().position(|i| i.id == id)?;
    let item = &items[at];
    let local = !shell.endpoint.is_remote();
    let ready = match &item.stage {
        Stage::Ready(ready) => Some(ready.clone()),
        _ => None,
    };
    // What to show: the file itself on this Mac (a picture, a video's sheet), else the thumbnail.
    let sheet = ready.as_ref().and_then(|r| r.tag.split(" · ").find_map(|f| f.strip_prefix("sheet: ")).map(|s| PathBuf::from(s.trim_end_matches(']'))));
    let shown: Option<ImageSource> = match (item.kind, local, &ready) {
        (Kind::Image, true, Some(r)) => Some(PathBuf::from(&r.path).into()),
        (Kind::Video, true, Some(_)) => sheet.map(Into::into),
        (Kind::Pdf, _, _) => shell.composer.page.as_ref().filter(|(at, _)| *at == id).map(|(_, page)| page.clone().into()),
        _ => None,
    }
    .or_else(|| item.thumb.clone().map(Into::into));
    let detail = chip_detail(item);
    let open_in = item.origin.clone().or_else(|| ready.as_ref().filter(|_| local).map(|r| PathBuf::from(&r.path)));
    let open_label = match item.kind {
        Kind::Video => t!("composer.open_quicktime"),
        _ => t!("composer.open_system"),
    };
    let n = items.len();
    let round = |b: Stateful<Div>, glyph: Icon, label: &'static str, keys: &'static str| {
        let p = shell.palette;
        tip(b.size(px(CONTROL)).flex().items_center().justify_center().rounded_full().hover(|s| s.bg(rgba(0xFFFFFF1F))).child(icon(glyph, ICON, rgb(0xFFFFFF))), p, label, Some(keys))
    };
    let backdrop = div()
            .id("lightbox")
            .absolute()
            .inset_0()
            .bg(rgba(0x000000F0))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .p_8()
            .on_click(cx.listener(|this, _, _, cx| {
                this.composer.lightbox = None;
                cx.notify();
            }))
            .child(motion::rise(
                div()
                    .id("lightbox-picture")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(match shown {
                        Some(source) => img(source).max_w_full().max_h_full().object_fit(ObjectFit::Contain).rounded(px(8.)).into_any_element(),
                        None => div().size(px(160.)).rounded(px(R_XL)).bg(rgba(0xFFFFFF14)).flex().items_center().justify_center().text_size(px(28.)).font_weight(FontWeight::BOLD).text_color(rgb(0xFFFFFF)).child(attach::badge(&item.name, item.kind)).into_any_element(),
                    }),
                ("lightbox-shown", id),
            ))
            .child(
                h_flex()
                    .id("lightbox-bar")
                    .flex_none()
                    .h(px(44.))
                    .px_2()
                    .gap_3()
                    .items_center()
                    .rounded(px(22.))
                    .bg(rgba(0x2A2A2EF2))
                    .border_1()
                    .border_color(rgba(0xFFFFFF1F))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .when(n > 1, |b| {
                        b.child(round(button("lightbox-prev", t!("composer.aria_previous"), |this, _, cx| {
                            this.step_lightbox(false, cx);
                            cx.notify();
                        }, cx), Icon::ArrowLeft, t!("composer.aria_previous"), "←"))
                    })
                    .child(div().text_size(px(13.)).text_color(rgb(0xFFFFFF)).child(item.name.clone()))
                    .child(div().text_size(px(12.)).text_color(rgba(0xFFFFFFB3)).child(format!("{detail} · {}/{n}", at + 1)))
                    .children(open_in.map(|path| {
                        button("lightbox-open", open_label, move |_, _, cx| cx.open_with_system(&path), cx).px_3().h(px(CONTROL)).flex().items_center().gap_1p5().rounded_full().border_1().border_color(rgba(0xFFFFFF4D)).hover(|s| s.bg(rgba(0xFFFFFF1F))).text_size(px(12.)).text_color(rgb(0xFFFFFF)).child(icon(Icon::External, ICON_SM, rgb(0xFFFFFF))).child(open_label)
                    }))
                    .when(n > 1, |b| {
                        b.child(round(button("lightbox-next", t!("composer.aria_next"), |this, _, cx| {
                            this.step_lightbox(true, cx);
                            cx.notify();
                        }, cx), Icon::ArrowRight, t!("composer.aria_next"), "→"))
                    })
                    .child(round(button("lightbox-close", t!("composer.aria_close"), |this, _, cx| {
                        this.composer.lightbox = None;
                        cx.notify();
                    }, cx), Icon::X, t!("composer.aria_close"), "Esc")),
            );
    Some(motion::fade(backdrop, "lightbox-in", motion::BASE))
}


#[cfg(test)]
mod tests {
    use super::{Act, Engine, Menu, MenuKind, Row, efforts, short_model, takes};

    #[test]
    fn model_names_read_short() {
        assert_eq!(short_model("claude-opus-5-5"), "opus 5.5");
        assert_eq!(short_model("claude-sonnet-4-5-20250929"), "sonnet 4.5");
        assert_eq!(short_model("opus"), "opus");
        assert_eq!(short_model("gpt-5.6"), "gpt-5.6");
    }

    #[test]
    fn the_composer_takes_the_keys_it_handles() {
        for key in ["escape", "up", "backspace", "left", "1", "9"] {
            assert!(takes(key), "{key}");
        }
        assert!(!takes("a") && !takes("0"));
    }

    #[test]
    fn effort_choices_follow_the_engine() {
        assert_eq!(efforts(Some(Engine::Claude)), ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(efforts(Some(Engine::Codex))[0], "minimal");
    }

    #[test]
    fn a_menu_walks_only_rows_that_do_something() {
        let rows = vec![Row { label: "title".into(), hint: String::new(), on: false, act: None }, Row::new("a", "", Act::Commands), Row::new("b", "", Act::Screenshot).checked(true)];
        let mut menu = Menu { kind: MenuKind::Attach, title: None, pick: Menu::first_pick(&rows), rows };
        assert_eq!(menu.pick, 2, "the checked one first");
        menu.step(false);
        assert_eq!(menu.pick, 1, "past the title, round to the first");
        menu.step(true);
        assert_eq!(menu.pick, 2);
    }
}
