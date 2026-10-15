//! The equipe desktop app: a window onto the daemon. It attaches to the
//! daemon's socket, follows one squad, shows its members and talks with them;
//! the screens live in `views/` (the board is its own ticket).
//!
//! It draws only when the daemon says something changed or the person acts —
//! see docs/DESKTOP.md. `EQUIPE_LOG_RENDER=1` prints a line per frame, to see that.

mod askpass;
mod attach;
mod board;
mod commands;
mod diffs;
mod compose;
mod activity;
mod convo;
mod decide;
mod daemon;
mod failure;
mod install;
mod files;
mod media;
mod mem;
mod notify;
mod pair;
mod ssh;
#[cfg(feature = "snapshot")]
mod snapshot;
mod state;
mod stream;
mod target;
mod theme;
mod tickets;
mod tips;
mod ui;
mod i18n;
mod icons;
mod motion;
mod relaunch;
mod sent;
mod render;
mod settings;
#[cfg(target_os = "macos")]
mod update;
mod views;
mod wire;
mod yourturn;

use std::time::Duration;

use equipe_core::t;

use equipe_providers::credentials::equipe_home;
use gpui_kit::component::input::{Enter, InputEvent};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use daemon::{Backoff, Link, Spawned, Target};
use target::{Endpoint, Remembered};
use equipe_core::protocol::{Command, Event, SessionId};
use state::{Clock, Conn, Model};
use theme::{BODY, Palette, SANS, SMALL};
use views::board::BoardUi;
use views::browser::{BrowserPanel, PanelEvent};
use views::chat::Chat;
use views::composer::Composer;
use views::connect::{AskUi, ConnectUi};

actions!(
    equipe,
    [ToggleBoard, ToggleBrowser, ChooseTarget, AllProjects, Project1, Project2, Project3, Project4, Project5, Project6, Project7, Project8, Project9, ToggleFolds]
);
// The conversation's: ⌘F, and Escape in its find bar.
actions!(equipe, [FindInConversation, CloseFind]);
// The app menu's.
actions!(equipe, [CheckForUpdates, Hide, HideOthers, ShowAll, Quit]);
// The sidebar's: hide or show it (⌘\), a new message (⌘N), search (⌘K).
actions!(equipe, [ToggleSidebar, NewMessage, SearchAll]);
// Settings (⌘,), from the app menu or anywhere.
actions!(equipe, [OpenSettings]);
// "휴대폰 연결…": a phone paired with the daemon's machine, by QR code.
actions!(equipe, [PairPhone]);

/// How long the daemon gets to answer before the window says it is connecting.
const CONNECTING_AFTER: Duration = Duration::from_millis(400);
/// Updates that arrive within this window are drawn together: a streaming
/// reply draws at most this often (T-54 §3.3), and not at all once it stops.
const COALESCE: Duration = Duration::from_millis(80);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Chat,
    Board,
    /// Settings (⌘,): its own list in the sidebar's place.
    Settings,
}

pub struct Shell {
    pub palette: Palette,
    pub model: Model,
    pub squads: Vec<String>,
    pub screen: Screen,
    pub chat: Chat,
    pub composer: Composer,
    pub board: BoardUi,
    /// The answer boxes of the cards tickets waiting for the person ask on.
    pub turn: views::yourturn::TurnUi,
    /// Members' questions: where the person is in each set, and its boxes.
    pub decide: views::decide::DecideUi,
    pub settings: views::settings::SettingsUi,
    /// The shared Chrome, live, beside the conversation (⌘⇧B).
    pub browser: Entity<BrowserPanel>,
    /// The ticket a `T-n` link asked the board to open.
    pub board_ticket: Option<u32>,
    /// Where a click on a notification says to go: the notifier's threads send, we follow.
    clicks: tokio::sync::mpsc::UnboundedSender<notify::Go>,
    /// An update downloaded and waiting for a restart: its version.
    pub update_ready: Option<String>,
    /// Whose daemon this window is a window onto, and what it remembers of that.
    pub endpoint: Endpoint,
    pub remembered: Remembered,
    pub connect: ConnectUi,
    pub ask: AskUi,
    /// "휴대폰 연결": the sheet's state and the paired phones (`pair.rs`).
    pub pairing: pair::Pairing,
    /// The paired phone whose "연결 끊기" was pressed once, asking to be sure.
    pub forgetting: Option<String>,
    _pair_tick: Option<Task<()>>,
    /// Something said on a thread of ours (a failed fetch) for the flash line.
    notices: tokio::sync::mpsc::UnboundedSender<String>,
    connecting_shown: bool,
    log_renders: bool,
    renders: u64,
    started: std::time::Instant,
    // Dropping these ends the link's thread and the tasks that feed from it.
    link: Option<Link>,
    _pump: Option<Task<()>>,
    _connecting: Option<Task<()>>,
    _clicks: Option<Task<()>>,
    _updates: Option<Task<()>>,
    _notices: Option<Task<()>>,
    _fetched: Option<Task<()>>,
    _prompts: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Sparkle skips a first launch and then waits its whole hour: look now,
        // and again whenever the window comes back after a while.
        relaunch::check_quietly(cx, std::time::Duration::ZERO);
        cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                relaunch::check_quietly(cx, relaunch::LOOK_AGAIN_AFTER);
            }
        })
        .detach();
        cx.observe_window_appearance(window, |this, window, cx| {
            this.palette = Palette::new(this.remembered.appearance.resolve(window.appearance()));
            this.sync_browser(window, cx);
            cx.notify();
        })
        .detach();
        let (clicks, mut clicked) = tokio::sync::mpsc::unbounded_channel();
        let (notices, mut noticed) = tokio::sync::mpsc::unbounded_channel::<String>();
        let remembered = Remembered::load(&equipe_home());
        let (fetched_tx, mut fetched) = tokio::sync::mpsc::unbounded_channel::<diffs::Fetched>();
        let endpoint = std::env::var("EQUIPE_TARGET")
            .ok()
            .and_then(|t| Endpoint::parse(&t).ok())
            .or_else(|| remembered.last.clone())
            .unwrap_or_else(Endpoint::this_mac);
        let connect = ConnectUi::new(window, cx, target::ssh_dir().map(|d| target::config_hosts(&d)).unwrap_or_default());
        let ask = AskUi::new(window, cx);
        // Light or dark as the person picked, else as the Mac is.
        remembered.appearance.apply();
        let palette = Palette::new(remembered.appearance.resolve(window.appearance()));
        let browser_width = remembered.browser_width;
        let browser = cx.new(|cx| BrowserPanel::new(browser_width, palette, window, cx));
        let mut shell = Self {
            palette,
            model: Model::new(""),
            squads: Vec::new(),
            screen: Screen::Chat,
            chat: Chat::new(cx),
            composer: Composer::new(window, cx),
            board: BoardUi::new(window, cx, fetched_tx),
            turn: Default::default(),
            decide: Default::default(),
            settings: views::settings::SettingsUi::new(window, cx),
            browser,
            board_ticket: None,
            clicks,
            update_ready: relaunch::watch().borrow().clone(),
            endpoint: endpoint.clone(),
            remembered,
            connect,
            ask,
            pairing: pair::Pairing::default(),
            forgetting: None,
            _pair_tick: None,
            notices,
            connecting_shown: false,
            log_renders: std::env::var_os("EQUIPE_LOG_RENDER").is_some(),
            renders: 0,
            started: std::time::Instant::now(),
            link: None,
            _pump: None,
            _connecting: None,
            _clicks: None,
            _updates: None,
            _notices: None,
            _fetched: None,
            _prompts: None,
            _subscriptions: Vec::new(),
        };
        if let Some(width) = shell.remembered.sidebar_width {
            shell.board.sidebar_width = width.clamp(views::sidebar::MIN_WIDTH, views::sidebar::MAX_WIDTH);
        }
        shell.chat.follow = !shell.remembered.whole_conversation;
        shell.board.view.layout = if shell.remembered.tickets_board { tickets::Layout::Board } else { tickets::Layout::List };
        shell._clicks = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(go) = clicked.recv().await {
                if this.update_in(cx, |this, window, cx| this.go(go, window, cx)).is_err() {
                    break;
                }
            }
        }));
        // An update downloaded in the background: the sidebar offers the restart.
        let mut updates = relaunch::watch();
        shell._updates = Some(cx.spawn_in(window, async move |this, cx| {
            while updates.changed().await.is_ok() {
                let version = updates.borrow_and_update().clone();
                if this.update_in(cx, |this, _, cx| {
                    this.update_ready = version;
                    cx.notify();
                })
                .is_err()
                {
                    break;
                }
            }
        }));
        shell._notices = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(text) = noticed.recv().await {
                if this.update_in(cx, |this, _, cx| {
                    this.composer.flash = Some(text);
                    cx.notify();
                })
                .is_err()
                {
                    break;
                }
            }
        }));
        shell._fetched = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(done) = fetched.recv().await {
                if this.update_in(cx, |this, _, cx| this.board_fetched(done, cx)).is_err() {
                    break;
                }
            }
        }));
        let board_subscriptions = shell.board_subscriptions(window, cx);
        shell._subscriptions.extend(board_subscriptions);
        let (connect_input, ask_input) = (shell.connect.input.clone(), shell.ask.input.clone());
        shell._subscriptions.push(cx.subscribe_in(&connect_input, window, |this, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let text = input.read(cx).value().to_string();
                this.connect_typed(&text, window, cx);
            }
        }));
        shell._subscriptions.push(cx.subscribe_in(&ask_input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_secret(window, cx);
            }
        }));
        let browser = shell.browser.clone();
        shell._subscriptions.push(cx.subscribe_in(&browser, window, |this, _, event: &PanelEvent, window, cx| match event {
            PanelEvent::Command(command) => this.send_command(command.clone()),
            PanelEvent::Resized(width) => {
                this.remembered.browser_width = Some(*width);
                this.remembered.save(&equipe_home());
            }
            PanelEvent::Opened => {
                this.show(Screen::Chat, cx);
                this.sync_browser(window, cx);
            }
        }));
        // Settings' search filters as it is typed.
        let settings_search = shell.settings.search.clone();
        shell._subscriptions.push(cx.subscribe_in(&settings_search, window, |_, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        }));
        let state = shell.composer.state.clone();
        shell._subscriptions.push(cx.subscribe_in(&state, window, |this, _, event: &InputEvent, window, cx| this.on_composer_event(event, window, cx)));
        shell._subscriptions.push(cx.observe_in(&state, window, |this, _, window, cx| this.on_composer_touched(window, cx)));
        // Esc, ↑↓, Tab and Enter get to the composer's completion and history
        // before the text box's own bindings take them.
        let this = cx.entity().downgrade();
        shell._subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            let key = event.keystroke.key.as_str();
            let modifiers = event.keystroke.modifiers;
            // Typing into the browser panel's page: its keys are the page's.
            if this.update(cx, |shell, cx| shell.browser.read(cx).focus.is_focused(window)).unwrap_or(false) {
                return;
            }
            // A box over the window has the keys: typing a password must never
            // answer a request for permission underneath.
            if this.update(cx, |shell, _| shell.overlay_open()).unwrap_or(false) {
                if key == "escape" {
                    this.update(cx, |shell, cx| shell.escape_overlay(window, cx)).ok();
                    cx.stop_propagation();
                }
                return;
            }
            // Settings has the page's keys: Esc, ↑↓, ⌘F.
            if this.update(cx, |shell, cx| shell.on_settings_key(key, &modifiers, window, cx)).unwrap_or(false) {
                cx.stop_propagation();
                return;
            }
            // A small menu open (the project slice): Esc puts it away.
            if key == "escape" && this.update(cx, |shell, cx| shell.close_popover(cx)).unwrap_or(false) {
                cx.stop_propagation();
                return;
            }
            // A member's questions above an empty composer (or in the peek):
            // 1–9, ↑↓, Space, ⏎, ⌘⏎, Tab, Esc — and its own box's typing.
            match this.update(cx, |shell, cx| shell.on_decide_key(key, &modifiers, window, cx)).unwrap_or(views::decide::Took::Pass) {
                views::decide::Took::Taken => {
                    cx.stop_propagation();
                    return;
                }
                views::decide::Took::Typing => return,
                views::decide::Took::Pass => {}
            }
            // The work screens' keys: A R N…, moving, the peek, search.
            if this.update(cx, |shell, cx| shell.on_board_key(key, &modifiers, window, cx)).unwrap_or(false) {
                cx.stop_propagation();
                return;
            }
            // y / a / ⇧a / n / o answer a request for permission while nothing is typed.
            if matches!(key, "y" | "a" | "n" | "o") && !(modifiers.control || modifiers.alt || modifiers.platform || modifiers.function) {
                if this.update(cx, |shell, cx| shell.on_approval_key(key, modifiers.shift, cx)).unwrap_or(false) {
                    cx.stop_propagation();
                }
                return;
            }
            if !views::composer::takes(key) {
                return;
            }
            let taken = this.update(cx, |shell, cx| shell.on_composer_key(key, &modifiers, window, cx)).unwrap_or(false);
            if taken {
                cx.stop_propagation();
            }
        }));
        shell.attach(Target::new(endpoint, env!("CARGO_PKG_VERSION")), window, cx);
        shell.composer.focus(window, cx);
        shell.watch_inbox(cx);
        shell.open_start(window, cx);
        shell
    }

    /// Bring what depends on the model up to date: the draft in the box and the
    /// conversation list.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.follow(&self.model, window, cx);
        self.turn_sync(window, cx);
        self.decide_sync(window, cx);
        self.chat_sync(cx);
        self.board_sync(cx);
        self.sync_browser(window, cx);
    }

    /// Tell the browser panel what it follows: this daemon and squad, the
    /// conversation on screen and who each conversation is.
    fn sync_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut who: std::collections::HashMap<SessionId, views::browser::Who> = self
            .model
            .members
            .iter()
            .map(|m| (m.session, views::browser::Who { name: m.name.clone(), hue: theme::look(&m.role, &m.name).1 }))
            .collect();
        for lane in &self.model.lanes {
            if let Some(member) = who.get(&lane.of).cloned() {
                who.insert(lane.session, member);
            }
        }
        let (endpoint, squad, viewed, palette, chat) = (self.endpoint.clone(), self.model.squad.clone(), self.model.viewed(), self.palette, self.screen == Screen::Chat);
        self.browser.update(cx, |b, cx| b.sync(&endpoint, &squad, viewed, who, palette, chat, window, cx));
    }

    /// ⌘⇧B, or the header's button: the browser panel opens or closes.
    pub fn toggle_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen != Screen::Chat {
            self.show(Screen::Chat, cx);
            if self.browser.read(cx).open {
                self.sync_browser(window, cx);
                return;
            }
        }
        self.browser.update(cx, |b, cx| b.toggle(window, cx));
        self.sync_browser(window, cx);
        if !self.browser.read(cx).open {
            self.composer.focus(window, cx);
        }
        cx.notify();
    }

    pub fn send_command(&self, command: Command) {
        if let Some(link) = &self.link {
            link.send(command);
        }
    }

    /// Talk to this member.
    pub fn select_member(&mut self, session: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        self.model.select(session);
        self.screen = Screen::Chat;
        self.refresh(window, cx);
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// A project chip (or ⌘0–⌘9): the squad's focus, the PM's thread, the board's filter.
    pub fn focus_project(&mut self, project: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.focus == project {
            return;
        }
        self.send_command(Command::Focus { squad: self.model.squad.clone(), project: project.clone() });
        self.set_focus(project, window, cx);
    }

    /// The same, when the daemon has been told already (`/project`).
    pub fn set_focus(&mut self, project: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.model.set_focus(project);
        self.refresh(window, cx);
        cx.notify();
    }

    fn project_number(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = self.model.project_names().get(n - 1).cloned() {
            self.focus_project(Some(name), window, cx);
        }
    }

    /// Follow `target`'s squad from scratch, letting go of any earlier link.
    fn attach(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        self.model = Model::new(target.prefer.as_deref().unwrap_or(""));
        self.connecting_shown = false;
        // Phones are the daemon's machine's: another machine has its own. A
        // code open closes with the old connection.
        let _ = self.pairing.close();
        self.pairing.devices = None;
        self.pairing.devices_asked = None;
        // Another machine: its files aren't the ones known.
        self.chat.sent.borrow_mut().clear();
        self.chat.gallery = None;
        // Questions still open were for the connection being left.
        self.ask.queue.clear();
        let backoff = if target.endpoint.is_remote() { Backoff::remote() } else { Backoff::default() };
        let Spawned { link, mut updates, mut prompts } = daemon::spawn(target, backoff);
        self.link = Some(link);
        self._prompts = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(prompt) = prompts.recv().await {
                if this.update_in(cx, |this, window, cx| this.ask_user(prompt, window, cx)).is_err() {
                    break;
                }
            }
        }));

        // The daemon's updates, drawn together when they land close to each
        // other, and only if something on screen differs.
        self._pump = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(first) = updates.recv().await {
                let mut batch = vec![first];
                cx.background_executor().timer(COALESCE).await;
                while let Ok(more) = updates.try_recv() {
                    batch.push(more);
                }
                let alive = this.update_in(cx, |this, window, cx| {
                    let now = Clock::now();
                    this.model.unix = state::now_unix();
                    let mut changed = false;
                    for update in batch {
                        // The shared Chrome's tabs and hands: the browser panel's.
                        if let daemon::Update::Event(
                            event @ (Event::BrowserTabs { .. } | Event::BrowserControl { .. } | Event::BrowserDialog { .. } | Event::BrowserDialogClosed { .. }),
                        ) = &update
                        {
                            // Told the squad first: these come right after the snapshot.
                            this.sync_browser(window, cx);
                            this.browser.update(cx, |b, cx| b.on_event(event, window, cx));
                            continue;
                        }
                        // Pairing a phone: the sheet's and settings', never the model's.
                        if let daemon::Update::Event(
                            event @ (Event::PairOffer { .. } | Event::PairAsked { .. } | Event::Paired { .. } | Event::PairEnded { .. } | Event::Devices { .. }),
                        ) = &update
                        {
                            changed |= this.pairing.on_event(event, std::time::Instant::now());
                            match event {
                                // The list in settings shows it at once.
                                Event::Paired { .. } => this.send_command(Command::Devices),
                                // A phone waits for the person's yes: bring the window to them.
                                Event::PairAsked { .. } if this.pairing.open => window.activate_window(),
                                _ => {}
                            }
                            continue;
                        }
                        if let daemon::Update::Down(_) = &update {
                            this.pairing.disconnected();
                        }
                        match &update {
                            // The daemon's own list, asked on every attach: a squad
                            // made since shows up, on this Mac or the far one.
                            daemon::Update::Squads(list) => {
                                let names: Vec<String> = list.squads.iter().map(|s| s.name.clone()).collect();
                                if names != this.squads {
                                    this.squads = names;
                                    changed = true;
                                }
                            }
                            // Over SSH the bridge had to start a daemon nobody installed
                            // as a login service: it won't see the screen.
                            daemon::Update::Bridge(bridge) if bridge.started => {
                                this.composer.flash = Some(t!("main.remote_daemon_started").into());
                                changed = true;
                            }
                            // Back on the line: what a message attached that couldn't be had is asked again.
                            daemon::Update::Attached(_) => this.chat.sent.borrow_mut().retry_gone(),
                            _ => {}
                        }
                        let attached = matches!(update, daemon::Update::Attached(_));
                        changed |= this.model.apply(update, now);
                        // Settings open on its list: the daemon (maybe another one) is asked again.
                        if attached && this.screen == Screen::Settings {
                            this.pairing.devices_asked = None;
                            this.ask_devices(cx);
                        }
                    }
                    // Told while the window is out of sight; in sight, the cards say it.
                    // Only the kinds the person wants (settings → 알림).
                    let pings = this.remembered.pings;
                    let alerts: Vec<notify::Alert> = this.model.take_alerts().into_iter().filter(|a| pings.wants(a)).collect();
                    if !alerts.is_empty() && !window.is_window_active() {
                        for toast in notify::toasts(&alerts, &this.model.squad, true) {
                            notify::send(toast, this.clicks.clone(), pings.sound);
                        }
                    }
                    if let Some(flash) = this.model.take_flash() {
                        this.composer.flash = Some(flash);
                        changed = true;
                    }
                    if changed {
                        if this.log_renders {
                            eprintln!("daemon update -> redraw");
                        }
                        this.refresh(window, cx);
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
        // Quick connections show nothing; a slow one says so, once.
        self._connecting = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CONNECTING_AFTER).await;
            let _ = this.update(cx, |this, cx| {
                if this.model.conn == Conn::Connecting {
                    this.connecting_shown = true;
                    cx.notify();
                }
            });
        }));
    }

    pub fn select_squad(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if name == self.model.squad {
            return;
        }
        let mut target = Target::new(self.endpoint.clone(), env!("CARGO_PKG_VERSION"));
        target.prefer = Some(name.to_owned());
        self.attach(target, window, cx);
        cx.notify();
    }

    // ── which machine ──

    /// Show the chooser: this Mac, the hosts `~/.ssh/config` names, or one typed.
    pub fn open_chooser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dir) = target::ssh_dir() {
            self.connect.hosts = target::config_hosts(&dir);
        }
        self.connect.open = true;
        self.connect.error = None;
        let input = self.connect.input.clone();
        input.update(cx, |s, cx| {
            s.set_value(String::new(), window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    pub fn close_chooser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.connect.open = false;
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// Connect to `endpoint` and remember it for the next launch.
    pub fn connect_to(&mut self, endpoint: Endpoint, window: &mut Window, cx: &mut Context<Self>) {
        self.connect.open = false;
        self.remembered.use_endpoint(&endpoint);
        self.remembered.save(&equipe_home());
        self.endpoint = endpoint.clone();
        self.squads.clear();
        self.composer.flash = None;
        self.attach(Target::new(endpoint, env!("CARGO_PKG_VERSION")), window, cx);
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// What was typed in the chooser (or picked from its list): `user@host[:port]`.
    pub fn connect_typed(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        match Endpoint::parse(text) {
            Ok(endpoint) => self.connect_to(endpoint, window, cx),
            Err(why) => {
                self.connect.error = Some(why);
                cx.notify();
            }
        }
    }

    /// "Try again", after the person fixed what was wrong.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        if let Some(link) = &self.link {
            link.retry();
        }
        cx.notify();
    }

    // ── questions from ssh ──

    pub fn overlay_open(&self) -> bool {
        self.connect.open || !self.ask.queue.is_empty() || self.pairing.open
    }

    fn escape_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ask.queue.is_empty() {
            self.answer_prompt(askpass::Answer::Cancel, window, cx);
        } else if self.pairing.open {
            self.close_pairing(window, cx);
        } else {
            self.close_chooser(window, cx);
        }
    }

    /// `ssh` wants something from the person: a box over the window.
    fn ask_user(&mut self, prompt: askpass::Prompt, window: &mut Window, cx: &mut Context<Self>) {
        self.ask.queue.push_back(prompt);
        if self.ask.queue.len() == 1 {
            self.prepare_question(window, cx);
        }
        // It may be a reconnect in the background: bring the window to the person.
        window.activate_window();
        cx.notify();
    }

    fn prepare_question(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let secret = matches!(self.ask.queue.front().map(|p| &p.ask), Some(askpass::Ask::Secret { .. }));
        if secret {
            let input = self.ask.input.clone();
            input.update(cx, |s, cx| {
                s.set_value(String::new(), window, cx);
                s.focus(window, cx);
            });
        }
    }

    pub fn answer_prompt(&mut self, answer: askpass::Answer, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(prompt) = self.ask.queue.pop_front() {
            let _ = prompt.reply.send(answer);
        }
        if self.ask.queue.is_empty() {
            self.composer.focus(window, cx);
        } else {
            self.prepare_question(window, cx);
        }
        cx.notify();
    }

    pub fn submit_secret(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.ask.input.read(cx).value().to_string();
        self.ask.input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        self.answer_prompt(askpass::Answer::Text(text), window, cx);
    }

    /// Show a file the daemon's machine named (an approval's screen). On this
    /// Mac it opens where it is; from another machine it is fetched first, over
    /// the same SSH connection, into a private temp file.
    pub fn open_file(&self, path: &str) {
        match &self.endpoint {
            Endpoint::Local { .. } => {
                let _ = std::process::Command::new("open").arg(path).spawn();
            }
            Endpoint::Ssh(host) => {
                let (host, path, notices) = (host.clone(), path.to_owned(), self.notices.clone());
                std::thread::spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime");
                    let fetched = runtime.block_on(ssh::Launcher::system(None).run(&host, &format!("cat -- {}", ssh::quote(&path))));
                    match fetched {
                        Ok(bytes) => {
                            let local = ssh::scratch_dir().join(format!("{}-{}", std::process::id(), ssh::safe_name(std::path::Path::new(&path))));
                            match std::fs::write(&local, bytes) {
                                Ok(()) => drop(std::process::Command::new("open").arg(&local).spawn()),
                                Err(e) => drop(notices.send(t!("main.preview_save_failed", e = e))),
                            }
                        }
                        Err(failure) => drop(notices.send(t!("main.preview_fetch_failed", host = host.spec(), why = failure.guide(&Endpoint::Ssh(host.clone())).short))),
                    }
                });
            }
        }
    }

    /// A notification was clicked: bring the window forward, on what it was about.
    /// "다시 시작해 업데이트": the downloaded update goes in, and the app starts again.
    pub fn relaunch_to_update(&mut self, cx: &mut Context<Self>) {
        if !relaunch::now() {
            self.composer.flash = Some(t!("main.update_on_quit").into());
            cx.notify();
        }
    }

    fn go(&mut self, go: notify::Go, window: &mut Window, cx: &mut Context<Self>) {
        match go {
            notify::Go::Member(session) => self.select_member(session, window, cx),
            notify::Go::Ticket(ticket) => self.open_ticket(ticket, cx),
            notify::Go::Board => self.show(Screen::Board, cx),
        }
        window.activate_window();
        cx.activate(true);
    }

    /// Put away a small menu open over the screen. True when one was.
    pub fn close_popover(&mut self, cx: &mut Context<Self>) -> bool {
        if self.board.popover.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    pub fn show(&mut self, screen: Screen, cx: &mut Context<Self>) {
        if self.screen != screen {
            self.screen = screen;
            self.board_sync(cx);
            cx.notify();
        }
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let nothing_attached = matches!(self.model.conn, Conn::Down { had_snapshot: false, .. });
        // A Mac without equipe is told how to install; a far machine's missing
        // equipe is that machine's guide, below.
        if nothing_attached && !self.endpoint.is_remote() && !install::cli_installed() {
            return install_screen(p);
        }
        match &self.model.conn {
            Conn::Down { reason, had_snapshot: false, .. } => v_flex().flex_1().child(self.drag_strip()).child(views::connect::guide_screen(self, &reason.guide(&self.endpoint), cx)).into_any_element(),
            Conn::Connecting => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(BODY))
                .text_color(p.muted)
                .when(self.connecting_shown, |d| {
                    d.child(match &self.endpoint {
                        Endpoint::Local { .. } => t!("main.connecting_local").to_owned(),
                        remote => t!("main.connecting_ssh", name = remote.label()),
                    })
                })
                .into_any_element(),
            conn => {
                let screen = match self.screen {
                    Screen::Chat => views::chat::screen(self, cx).into_any_element(),
                    // 나에게, 작업 중, 모든 일, a saved view: the menu item chosen.
                    Screen::Board => views::board::render(self, cx).into_any_element(),
                    Screen::Settings => views::settings::render(self, cx),
                };
                v_flex().flex_1().min_w_0().h_full().children(self.banner(conn, cx)).child(screen).into_any_element()
            }
        }
    }

    /// A strip to drag the window by, over a screen with no header of its
    /// own: the title bar's height and hairline (its buttons float above).
    fn drag_strip(&self) -> impl IntoElement + use<> {
        div().h(px(views::sidebar::STRIP)).flex_none().w_full().border_b_1().border_color(self.palette.hair).window_control_area(WindowControlArea::Drag)
    }

    /// The sidebar, sliding in or out for a moment after ⌘\ — its width read
    /// from the one clock the headers beside it read too, so they go in step.
    fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.sidebar_sliding(cx) {
            window.request_animation_frame();
        }
        let shown = self.sidebar_shown(cx);
        if shown <= 0. {
            return None;
        }
        let side = views::sidebar::render(self, cx).into_any_element();
        if shown >= 1. {
            return Some(side);
        }
        Some(div().h_full().flex_none().overflow_hidden().w(px(self.board.sidebar_width * shown)).child(side).into_any_element())
    }

    /// Shown over the last state when the daemon goes away after we had it.
    fn banner(&self, conn: &Conn, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let p = self.palette;
        let Conn::Down { reason, since, had_snapshot: true } = conn else { return None };
        let guide = reason.guide(&self.endpoint);
        let waits = reason.waits_for_person();
        Some(
            h_flex()
                .h(px(36.))
                .flex_none()
                .px_4()
                .gap_3()
                .items_center()
                .bg(p.accent_bg)
                .border_b_1()
                .border_color(p.hair)
                .text_size(px(SMALL))
                .child(div().text_color(p.text).font_weight(FontWeight::SEMIBOLD).child("✗"))
                .child(div().text_color(p.text).child(format!("{} · {since}", guide.short)))
                .child(div().flex_1().min_w_0().text_color(p.muted).child(if waits { t!("main.banner_needs_you") } else { t!("main.banner_reconnecting") }))
                .child(ui::button(p, "banner-retry", ui::Kind::Secondary, t!("main.retry")).icon(icons::Icon::Retry).on_click(cx.listener(|this, _, _, cx| this.retry(cx)))),
        )
    }
}

/// The body on a Mac without equipe: what the app needs, and the line that installs it.
fn install_screen(p: Palette) -> AnyElement {
    let button = |id: &'static str, label: &'static str| ui::button(p, id, ui::Kind::Secondary, label);

    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .child(div().text_size(px(18.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("main.install_title")))
        .child(
            div()
                .max_w(px(520.))
                .text_size(px(BODY))
                .line_height(px(24.))
                .text_color(p.text2)
                .text_center()
                .child(t!("main.install_text")),
        )
        .child(
            div()
                .id("install-command")
                .px_3()
                .py_1()
                .rounded(px(8.))
                .bg(p.code_bg)
                .border_1()
                .border_color(p.hair)
                .font_family("Menlo")
                .text_size(px(SMALL))
                .text_color(p.text2)
                .child(install::INSTALL_COMMAND),
        )
        .child(
            h_flex()
                .gap_2()
                .child(button("copy-install", t!("main.copy_command")).icon(icons::Icon::Copy).on_click(|_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(install::INSTALL_COMMAND.to_owned()))))
                .child(button("open-site", t!("main.install_guide")).icon(icons::Icon::External).on_click(|_, _, cx| cx.open_url(install::SITE))),
        )
        .child(div().text_size(px(SMALL)).text_color(p.muted).child(t!("main.homebrew_hint")))
        .into_any_element()
}

impl Shell {
    /// The language changed: the words kept in the inputs outside the board
    /// follow, as the board's do ([`views::board::BoardUi::relocalize`]).
    fn relocalize_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = &self.chat.find.input {
            input.update(cx, |s, cx| s.set_placeholder(t!("chat.find_placeholder"), window, cx));
        }
        self.connect.input.update(cx, |s, cx| s.set_placeholder(t!("connect.host_placeholder"), window, cx));
        self.ask.input.update(cx, |s, cx| s.set_placeholder(t!("connect.secret_placeholder"), window, cx));
        self.browser.update(cx, |b, cx| b.relocalize(window, cx));
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.board.words != crate::i18n::lang() {
            self.relocalize_inputs(window, cx);
        }
        self.board.relocalize(window, cx);
        self.board_prepare(window, cx);
        self.renders += 1;
        if self.log_renders {
            let mb = mem::phys_footprint_bytes().map_or(0., |b| b as f64 / 1_048_576.);
            eprintln!("render #{} {:.1}s ({mb:.0} MB)", self.renders, self.started.elapsed().as_secs_f32());
        }
        let p = self.palette;
        macro_rules! project_actions {
            ($root:expr, $cx:expr, $($action:ident => $n:expr),*) => {{
                let mut root = $root;
                $(root = root.on_action($cx.listener(|this, _: &$action, window, cx| this.project_number($n, window, cx)));)*
                root
            }};
        }
        let root = v_flex()
            .size_full()
            .bg(p.bg)
            .font_family(SANS)
            .text_color(p.text)
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &NewMessage, window, cx| this.new_message(window, cx)))
            .on_action(cx.listener(|this, _: &SearchAll, window, cx| this.search_everything(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.toggle_settings(window, cx)))
            .on_action(cx.listener(|this, _: &PairPhone, window, cx| this.open_pairing(window, cx)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if this.board.sidebar_drag.is_some() {
                    if e.pressed_button == Some(MouseButton::Left) {
                        this.sidebar_dragged(f32::from(e.position.x), cx);
                    } else {
                        this.sidebar_released(cx);
                    }
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.sidebar_released(cx)))
            .on_action(cx.listener(|this, _: &ToggleBoard, _, cx| {
                let next = if this.screen == Screen::Chat { Screen::Board } else { Screen::Chat };
                this.show(next, cx);
            }))
            .on_action(cx.listener(|this, _: &ChooseTarget, window, cx| this.open_chooser(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleBrowser, window, cx| this.toggle_browser(window, cx)))
            // The browser panel's divider follows the pointer wherever it goes.
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
                let pressed = e.pressed_button == Some(MouseButton::Left);
                this.browser.update(cx, |b, cx| b.drag_to(f32::from(e.position.x), pressed, window, cx));
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, e: &MouseUpEvent, window, cx| {
                    this.browser.update(cx, |b, cx| b.drag_to(f32::from(e.position.x), false, window, cx));
                }),
            )
            .on_action(cx.listener(|this, _: &AllProjects, window, cx| this.focus_project(None, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleFolds, _, cx| this.toggle_folds(cx)))
            .on_action(cx.listener(|this, _: &FindInConversation, window, cx| this.find_open(window, cx)))
            .on_action(cx.listener(|this, _: &CloseFind, window, cx| {
                this.chat_escape(window, cx);
            }));
        let root = project_actions!(
            root, cx, Project1 => 1, Project2 => 2, Project3 => 3, Project4 => 4, Project5 => 5, Project6 => 6, Project7 => 7, Project8 => 8, Project9 => 9
        );
        // The sidebar runs to the window's top, the traffic lights over its
        // strip; each screen's header is the title bar on the right.
        if self.screen == Screen::Settings {
            // Settings: its own list in the sidebar's place, the page beside it.
            return root
                .relative()
                .child(h_flex().flex_1().min_h_0().child(views::settings::render(self, cx)))
                .children(self.connect.open.then(|| views::connect::chooser(self, cx)))
                .children(views::pair::sheet(self, cx))
                .children(views::connect::question(self, cx))
                .children(tips::layer(cx));
        }
        let sidebar = self.sidebar(window, cx);
        let browser = (self.screen == Screen::Chat && self.browser.read(cx).open && !matches!(self.model.conn, Conn::Down { had_snapshot: false, .. } | Conn::Connecting)).then(|| self.browser.clone());
        root.relative().child(h_flex().flex_1().min_h_0().children(sidebar).child(div().flex_1().min_w_0().h_full().flex().flex_col().child(self.body(cx))).children(browser))
            // The title bar's fixed buttons, by the traffic lights, over the
            // sidebar or the header alike: the same place whatever slides.
            .child(views::sidebar::title_buttons(self, cx))
            // Over everything: the chooser, then a question from ssh above it.
            .children(views::composer::drop_overlay(self, cx))
            .children(views::composer::lightbox(self, cx))
            .children(views::attached::gallery(self, cx))
            .children(self.connect.open.then(|| views::connect::chooser(self, cx)))
            .children(views::pair::sheet(self, cx))
            .children(views::connect::question(self, cx))
            .children(tips::layer(cx))
    }
}

/// The app menu: updates, hiding, quitting.
fn app_menu(cx: &mut App) {
    #[cfg(target_os = "macos")]
    cx.set_global(update::Updates(update::Updater::start().map(std::rc::Rc::new)));
    let can_update = relaunch::can_update(cx);
    cx.on_action(|_: &CheckForUpdates, cx| relaunch::check_now(cx));
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    CAN_UPDATE.store(can_update, std::sync::atomic::Ordering::Relaxed);
    set_menus(cx);
}

/// Whether the app updates itself, as the app menu found at start.
static CAN_UPDATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The menu bar, in the language in use: at start, and again when it changes.
pub fn set_menus(cx: &mut App) {
    let can_update = CAN_UPDATE.load(std::sync::atomic::Ordering::Relaxed);
    cx.set_menus(vec![Menu::new("equipe").items([
        MenuItem::action(t!("main.menu_choose_target"), ChooseTarget),
        MenuItem::action(t!("pair.menu_item"), PairPhone),
        MenuItem::action(t!("settings.menu_item"), OpenSettings),
        MenuItem::action(t!("main.menu_check_updates"), CheckForUpdates).disabled(!can_update),
        MenuItem::separator(),
        MenuItem::action(t!("main.menu_hide"), Hide),
        MenuItem::action(t!("main.menu_hide_others"), HideOthers),
        MenuItem::action(t!("main.menu_show_all"), ShowAll),
        MenuItem::separator(),
        MenuItem::action(t!("main.menu_quit"), Quit),
    ]), edit_menu()]);
}

/// The Edit menu: the text box's own actions, under the selectors macOS knows
/// — which is also where it adds "받아쓰기 시작…" and "이모지 및 기호".
fn edit_menu() -> Menu {
    use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
    Menu::new(t!("main.menu_edit")).items([
        MenuItem::os_action(t!("main.menu_undo"), Undo, OsAction::Undo),
        MenuItem::os_action(t!("main.menu_redo"), Redo, OsAction::Redo),
        MenuItem::separator(),
        MenuItem::os_action(t!("main.menu_cut"), Cut, OsAction::Cut),
        MenuItem::os_action(t!("main.menu_copy"), Copy, OsAction::Copy),
        MenuItem::os_action(t!("main.menu_paste"), Paste, OsAction::Paste),
        MenuItem::os_action(t!("main.menu_select_all"), SelectAll, OsAction::SelectAll),
    ])
}

fn main() {
    // `ssh` starts this same binary to ask the person a question: answer it
    // through the running app and leave, without opening a window.
    if let Some(code) = askpass::helper() {
        std::process::exit(code);
    }
    // The language picked for every screen — this app's own pick from before
    // becomes that, once — else the Mac's, before any word is shown.
    i18n::migrate(&equipe_home());
    // Images a reply points at on the web come through this client.
    let http = std::sync::Arc::new(reqwest_client::ReqwestClient::user_agent("equipe-desktop").unwrap_or_else(|_| reqwest_client::ReqwestClient::new()));
    // The app's icons, then GPUI Kit's own (the sidebar's, the components').
    gpui_kit::application().with_http_client(http).with_assets(icons::AppAssets).run(|cx| {
        gpui_kit::init(cx);
        tips::init(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-b", ToggleBoard, None),
            KeyBinding::new("cmd-shift-b", ToggleBrowser, None),
            KeyBinding::new("cmd-\\", ToggleSidebar, None),
            KeyBinding::new("cmd-n", NewMessage, None),
            KeyBinding::new("cmd-k", SearchAll, None),
            KeyBinding::new("cmd-,", OpenSettings, None),
            KeyBinding::new("cmd-0", AllProjects, None),
            KeyBinding::new("cmd-1", Project1, None),
            KeyBinding::new("cmd-2", Project2, None),
            KeyBinding::new("cmd-3", Project3, None),
            KeyBinding::new("cmd-4", Project4, None),
            KeyBinding::new("cmd-5", Project5, None),
            KeyBinding::new("cmd-6", Project6, None),
            KeyBinding::new("cmd-7", Project7, None),
            KeyBinding::new("cmd-8", Project8, None),
            KeyBinding::new("cmd-9", Project9, None),
            KeyBinding::new("cmd-shift-o", ToggleFolds, None),
            KeyBinding::new("cmd-f", FindInConversation, None),
            KeyBinding::new("escape", CloseFind, Some("FindBar")),
            KeyBinding::new("cmd-shift-k", ChooseTarget, None),
            // ⌥⏎ and ⌃J break the line like ⇧⏎ (T-54 §8).
            KeyBinding::new("alt-enter", Enter { secondary: false, shift: true }, Some("Input")),
            KeyBinding::new("ctrl-j", Enter { secondary: false, shift: true }, Some("Input")),
        ]);
        app_menu(cx);
        views::composer::bind_keys(cx);
        let bounds = Bounds::centered(None, size(px(1200.), px(760.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(760.), px(520.))),
            titlebar: Some(TitlebarOptions {
                title: Some("equipe".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(views::sidebar::LIGHTS_AT.0), px(views::sidebar::LIGHTS_AT.1))),
            }),
            ..Default::default()
        };
        let (handle, shell) = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(window, cx))).expect("failed to open window");
        cx.activate(true);
        #[cfg(feature = "snapshot")]
        snapshot::run(handle, shell, cx);
        #[cfg(not(feature = "snapshot"))]
        let _ = (handle, shell);
    });
}
