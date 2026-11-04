//! The equipe desktop app: a window onto the daemon. It attaches to the
//! daemon's socket, follows one squad, shows its members and talks with them;
//! the screens live in `views/` (the board is its own ticket).
//!
//! It draws only when the daemon says something changed or the person acts —
//! see docs/DESKTOP.md. `EQUIPE_LOG_RENDER=1` prints a line per frame, to see that.

mod asklist;
mod askpass;
mod attach;
mod beat;
mod board;
mod chapters;
mod checks;
mod browse;
mod commands;
mod diffs;
mod engines;
mod switching;
mod export;
mod compose;
mod accept;
mod activity;
mod convo;
mod decide;
mod deck;
mod daemon;
mod disk;
mod doc;
mod compare;
mod design;
mod design_talk;
mod dictation;
mod flows;
mod failure;
mod help;
mod install;
mod files;
mod media;
mod mem;
mod notify;
mod pair;
mod pane;
mod queue;
mod quote;
mod readonly;
mod screen_look;
mod setup;
mod sheet;
mod side_split;
mod sidecar;
mod squad_form;
mod ssh;
mod sshkey;
#[cfg(feature = "snapshot")]
mod snapshot;
mod state;
mod stream;
mod target;
mod theme;
mod routine;
mod retro;
mod team;
mod term;
mod tailnet;
mod tickets;
mod tips;
mod transcribe;
mod ui;
mod i18n;
mod icons;
mod motion;
mod relaunch;
use relaunch::DaemonUpdate;
mod metrics;
mod roadmap;
mod sent;
mod releases;
mod render;
mod settings;
#[cfg(target_os = "macos")]
mod update;
mod usage;
mod views;
mod visuals;
mod voiceover;
mod windows;
mod wire;
mod yourturn;
mod zoom;

use std::time::Duration;

use equipe_core::t;

use equipe_providers::credentials::equipe_home;
use gpui_kit::component::input::{Enter, InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use daemon::{Backoff, Link, Spawned, Target};
use target::{Endpoint, Remembered};
use equipe_core::protocol::{Command, Event, SessionId};
use state::{Clock, Conn, Model};
use theme::{BODY, Palette, SMALL, Sans};
use views::board::BoardUi;
use views::browser::{BrowserPanel, PanelEvent};
use views::chat::Chat;
use views::composer::Composer;
use views::connect::{AskUi, ConnectUi};

actions!(
    equipe,
    [ToggleBoard, ToggleBrowser, ToggleTerminal, ChooseTarget, AllProjects, Project1, Project2, Project3, Project4, Project5, Project6, Project7, Project8, Project9, ToggleFolds]
);
// The conversation's: ⌘F, and Escape in its find bar.
actions!(equipe, [FindInConversation, CloseFind]);
// The app menu's.
actions!(equipe, [CheckForUpdates, Hide, HideOthers, ShowAll, Quit]);
// The sidebar's: hide or show it (⌘\), a new message (⌘N), search (⌘K).
actions!(equipe, [ToggleSidebar, NewMessage, SearchAll]);
// Settings (⌘,), from the app menu or anywhere.
actions!(equipe, [OpenSettings]);
// "Connect a phone…": a phone paired with the daemon's machine, by QR code.
actions!(equipe, [PairPhone]);
// A press outside a box over the window: what Esc does to it.
actions!(equipe, [DismissOverlay, NewSquad]);
// The View menu's: the window's size, as a browser zooms (⌘+, ⌘-, ⌘0).
actions!(equipe, [ZoomIn, ZoomOut, ActualSize]);
// The Window menu's: one more window (⌘⇧N), and the Mac's own two.
actions!(equipe, [NewWindow, MinimizeWindow, ZoomWindow]);

/// The menu bar and ⌘K: each item is a command line (T-62 §2.7 — every menu
/// item is a command, and every command is in the palette). A line that ends
/// in a space wants arguments: it goes in the box with its completions open.
macro_rules! menu_commands {
    ($($Action:ident => $line:expr),* $(,)?) => {
        actions!(equipe, [$($Action),*]);

        /// Wire every menu action to its command line.
        fn menu_listeners(mut root: Div, cx: &mut Context<Shell>) -> Div {
            $(root = root.on_action(cx.listener(|this, _: &$Action, window, cx| this.menu_line($line, window, cx)));)*
            root
        }
    };
}

menu_commands!(
    CommandPalette => "/",
    ShowUsage => "/usage",
    ShowHelp => "/help",
    OpenSquadFolder => "/home",
    HomeChanges => "/home changes",
    HomeUndo => "/home undo",
    OpenSquadMemory => "/memory",
    MemoryChanges => "/memory changes",
    ExportConversation => "/export",
    TidyBoard => "/tidy",
    SwitchSquad => "/squad ",
    AddFolder => "/squad add ",
    RemoveFolder => "/squad remove ",
    AddMembers => "/add ",
    SetModel => "/model ",
    SetMode => "/mode ",
    CompactMember => "/compact",
    ClearMember => "/clear",
    CloseMember => "/close",
    BrowserSignin => "/browser signin",
    BrowserShow => "/browser show",
    BrowserHide => "/browser hide",
    BrowserMax => "/browser max ",
    SendApps => "/send-apps",
    ScreenBack => "/desktop stop",
    ScreenOpen => "/desktop resume",
    SuggestOn => "/suggest on",
    SuggestOff => "/suggest off",
);

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
    /// Design system: the squad's design system (`views/design.rs`).
    Design,
    /// Roadmap: the work as one picture, what waits on what (`views/roadmap.rs`).
    Roadmap,
    /// Metrics: the product's numbers, from the services it reports to (`views/metrics.rs`).
    Metrics,
    /// A project's page: its conversations and its work (`views/project.rs`).
    Project,
    /// Progress: what needs a look, what is under way, what finished (`views/progress.rs`).
    Progress,
}

pub struct Shell {
    pub palette: Palette,
    pub model: Model,
    pub squads: Vec<String>,
    pub screen: Screen,
    pub chat: Chat,
    pub composer: Composer,
    pub board: BoardUi,
    /// The window's big parts, each drawn on its own (`pane.rs`).
    pub panes: pane::Panes,
    /// The answer boxes of the cards tickets waiting for the person ask on.
    pub turn: views::yourturn::TurnUi,
    /// Where this window was opened to go (⌘-click elsewhere, a window
    /// opened again at launch), once connected.
    pub goal: Option<views::elsewhere::Goal>,
    /// Which of the app's windows this is (`windows.rs`): its own machine and squad.
    pub window_id: u64,
    /// The machine, squad and place last told to the app's list of windows,
    /// and the title last set.
    seat_told: Option<(windows::Seat, windows::Spot)>,
    title_shown: String,
    /// The boxes of members' asks for a secret.
    pub secrets: views::secrets::SecretsUi,
    /// The boxes the person writes to a member in, on its request for permission.
    pub approval_words: views::approvals::Words,
    /// Members' questions: where the person is in each set, and its boxes.
    pub decide: views::decide::DecideUi,
    /// Members' pictures — decisions' options, hand-ins — and the one shown big.
    pub shots: std::rc::Rc<std::cell::RefCell<visuals::Visuals>>,
    pub viewer: Option<visuals::Viewer>,
    /// How far the big view is zoomed in, and where.
    pub viewer_zoom: visuals::Zoom,
    /// A reply's list of questions, answered in place.
    pub asklist: views::asklist::AskListUi,
    /// Words selected in the conversation, asked about (`views/quote.rs`).
    pub quote: views::quote::QuoteUi,
    pub settings: views::settings::SettingsUi,
    /// Design system's state: what the daemon said, what is on screen.
    pub design: design::DesignUi,
    /// Compare and review's box for what to change, a draft sent back with it.
    pub design_redo: Entity<InputState>,
    /// The routines' list on Progress: which is being edited or deleted.
    pub routines_ui: views::routines::RoutinesUi,
    /// Each project's newest tag, as last found, and when it was asked.
    pub releases: Vec<(std::path::PathBuf, releases::Release)>,
    /// The squad's briefs and plans, as the daemon last sent them.
    pub docs: Vec<equipe_core::protocol::briefs::ProjectDoc>,
    pub releases_asked: Option<std::time::Instant>,
    /// Metrics' state: what the daemon said, the form being filled.
    pub metrics: metrics::Metrics,
    pub metrics_ui: views::metrics::Inputs,
    /// Taste & decisions' box for a note.
    pub design_taste: views::design_taste::Inputs,
    /// The shared Chrome, live, beside the conversation (⌘⇧B).
    pub browser: Entity<BrowserPanel>,
    /// The terminal at the right, while one runs.
    pub(crate) terminal: Option<views::terminal::TermUi>,
    /// The reply whose command was last typed into the terminal unasked.
    term_offered: Option<u64>,
    /// The sign-in card's box for the code a sign-in page shows, once asked for.
    pub(crate) signin_code: Option<Entity<gpui_kit::component::input::InputState>>,
    pub(crate) signin_sub: Option<Subscription>,
    /// The document open in the panel at the right, if one is.
    pub(crate) doc: Option<doc::Doc>,
    doc_reads: u64,
    /// The side panel's edge being dragged: where it began, and the width then.
    pub(crate) side_drag: Option<(f32, f32)>,
    /// The terminal's top edge being dragged (under the conversation): where it began, the height then.
    pub(crate) term_drag: Option<(f32, f32)>,
    /// The daemon's machine being brought up to this app's release.
    pub(crate) daemon_update: Option<DaemonUpdate>,
    /// The person closed the banner: not again this launch.
    /// The daemon's update was asked for with the app's: the app restarts once it is in.
    relaunch_after_daemon: bool,
    /// The status page's releases, as last found.
    /// The banner's "restart now" asked once: the next click restarts.
    pub(crate) daemon_restart_armed: bool,
    /// The ticket a `T-n` link asked the board to open.
    pub board_ticket: Option<u32>,
    /// Notifications: what macOS allows, the sheet asking, the reply awaited, the Dock's number.
    pub bell: notify::Bell,
    /// Where an update stands: being downloaded, waiting for a restart, failed.
    pub update: relaunch::Update,
    /// Whose daemon this window is a window onto, and what it remembers of that.
    pub endpoint: Endpoint,
    pub remembered: Remembered,
    pub connect: ConnectUi,
    /// The first run's form (`views/welcome.rs`).
    pub welcome: views::welcome::WelcomeUi,
    /// "New squad…"'s sheet: the same form over the window (`views/new_squad.rs`).
    pub new_squad: views::welcome::WelcomeUi,
    /// Anonymous usage data: the daemon's answer and the question (`views/telemetry.rs`).
    pub telemetry: views::telemetry::TelemetryUi,
    pub ask: AskUi,
    /// A password just typed with "don't ask again" on: the key goes there once connected.
    ssh_key_wanted: bool,
    /// "Connect a phone": the sheet's state and the paired phones (`pair.rs`).
    pub pairing: pair::Pairing,
    /// The AI platforms, as settings shows them.
    pub engines: engines::Engines,
    /// The first run's git, engine and squad steps, while no squad exists.
    pub setup: setup::Setup,
    /// The paired phone whose "Disconnect" was pressed once, asking to be sure.
    pub forgetting: Option<String>,
    /// "Disk": the banner while it runs short, and the panel (`disk.rs`).
    pub disk: disk::DiskUi,
    /// `/usage`, over the window (`views/usage.rs`).
    pub usage_open: bool,
    /// The window's size, shown for a moment after it changes (`zoom.rs`).
    pub zoom_hud: Option<zoom::Hud>,
    _pair_tick: Option<Task<()>>,
    /// Something said on a thread of ours (a failed fetch) for the flash line.
    notices: tokio::sync::mpsc::UnboundedSender<String>,
    /// A side command on the daemon's machine finished: what to do with its answer.
    jobs: tokio::sync::mpsc::UnboundedSender<(sheet::After, Result<Vec<String>, String>)>,
    /// A sheet over the window (usage, help, changes, memory) and a question asked first.
    pub sheet: Option<sheet::Sheet>,
    pub confirm: Option<sheet::Ask>,
    /// The roles a squad just started joins with, once it is open and empty.
    pending_team: Option<Vec<String>>,
    /// A squad just made on another machine, and how much it does alone: set
    /// on its daemon once the window is attached to it.
    pending_mode: Option<(String, equipe_core::protocol::Mode)>,
    connecting_shown: bool,
    /// How far the connection being made has got, for its screen.
    pub(crate) connecting: views::connect::Connecting,
    pub(crate) log_renders: bool,
    /// The tickets on cards to confirm whose size was asked for, as they looked then.
    pub(crate) accept_asked: std::collections::HashMap<equipe_core::protocol::TicketId, (Option<String>, String, Option<bool>)>,
    renders: u64,
    started: std::time::Instant,
    /// When this screen tells the daemon its look (`screen_look.rs`).
    look_teller: screen_look::Teller,
    // Dropping these ends the link's thread and the tasks that feed from it.
    link: Option<Link>,
    _pump: Option<Task<()>>,
    _connecting: Option<Task<()>>,
    _updates: Option<Task<()>>,
    _notices: Option<Task<()>>,
    _jobs: Option<Task<()>>,
    _fetched: Option<Task<()>>,
    _prompts: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    fn new(opening: windows::Opening, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Started again on the way to the newest release: keep at it.
        relaunch::begin_chain();
        // Sparkle skips a first launch and then waits its whole hour: look now,
        // and again whenever the window comes back after a while.
        relaunch::check_quietly(cx, std::time::Duration::ZERO);
        cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                // Brought forward: the person is at this screen.
                screen_look::used();
                relaunch::check_quietly(cx, relaunch::LOOK_AGAIN_AFTER);
            }
        })
        .detach();
        // Its size, kept as the person leaves it: it opens at it again, and
        // the UI map's real view is shot at it.
        cx.observe_window_bounds(window, |this, window, cx| this.keep_window_size(window, cx)).detach();
        cx.observe_window_appearance(window, |this, window, cx| {
            let appearance = this.remembered.appearance.resolve(window.appearance());
            this.palette = Palette::new(appearance);
            this.palette.apply_to_kit(appearance, cx);
            this.sync_browser(window, cx);
            cx.notify();
        })
        .detach();
        let (notices, mut noticed) = tokio::sync::mpsc::unbounded_channel::<String>();
        let (jobs, mut finished) = tokio::sync::mpsc::unbounded_channel();
        let mut remembered = Remembered::load(&equipe_home());
        // The real view: the person's own screen's look over it (never saved).
        if let Some(look) = readonly::look() {
            look.lay_over(&mut remembered);
        }
        let (fetched_tx, mut fetched) = tokio::sync::mpsc::unbounded_channel::<diffs::Fetched>();
        // Its own machine — the one it was opened on — else as the app was left.
        let endpoint = opening
            .endpoint
            .clone()
            .or_else(|| std::env::var("EQUIPE_TARGET").ok().and_then(|t| Endpoint::parse(&t).ok()))
            .or_else(|| remembered.last.clone())
            .unwrap_or_else(Endpoint::this_mac);
        let connect = ConnectUi::new(window, cx, target::ssh_dir().map(|d| target::config_hosts(&d)).unwrap_or_default());
        let ask = AskUi::new(window, cx);
        let welcome = views::welcome::WelcomeUi::new(views::welcome::Which::First, window, cx);
        let new_squad = views::welcome::WelcomeUi::new(views::welcome::Which::New, window, cx);
        let telemetry = views::telemetry::TelemetryUi::new(cx);
        // Light or dark as the person picked, else as the Mac is.
        remembered.appearance.apply();
        let appearance = remembered.appearance.resolve(window.appearance());
        let palette = Palette::new(appearance);
        palette.apply_to_kit(appearance, cx);
        let browser_width = remembered.browser_width;
        let browser = cx.new(|cx| BrowserPanel::new(browser_width, palette, window, cx));
        let mut shell = Self {
            palette,
            model: Model::new(""),
            squads: Vec::new(),
            screen: Screen::Chat,
            chat: Chat::new(cx),
            composer: Composer::new(
                window,
                cx,
                dictation::choose(remembered.dictation.as_deref().and_then(equipe_core::i18n::Lang::parse), crate::i18n::lang(), &equipe_core::i18n::preferred(), |l| remembered.dictation_servers || dictation::heard_here(l)),
                remembered.dictation_servers,
            ),
            board: BoardUi::new(window, cx, fetched_tx),
            panes: pane::Panes::new(cx),
            turn: Default::default(),
            goal: opening.goal.clone(),
            window_id: window.window_handle().window_id().as_u64(),
            seat_told: None,
            title_shown: String::new(),
            secrets: Default::default(),
            approval_words: Default::default(),
            decide: Default::default(),
            shots: Default::default(),
            viewer: None,
            viewer_zoom: Default::default(),
            asklist: Default::default(),
            quote: Default::default(),
            settings: views::settings::SettingsUi::new(window, cx),
            design: Default::default(),
            design_redo: cx.new(|cx| InputState::new(window, cx).placeholder(t!("design.compare_redo_placeholder"))),
            routines_ui: Default::default(),
            releases: Vec::new(),
            docs: Vec::new(),
            releases_asked: None,
            metrics: metrics::Metrics::new(),
            metrics_ui: Default::default(),
            design_taste: Default::default(),
            browser,
            terminal: None,
            term_offered: None,
            signin_code: None,
            signin_sub: None,
            doc: None,
            doc_reads: 0,
            side_drag: None,
            term_drag: None,
            daemon_update: None,
            relaunch_after_daemon: false,
            daemon_restart_armed: false,
            board_ticket: None,
            bell: Default::default(),
            update: relaunch::current(),
            endpoint: endpoint.clone(),
            remembered,
            connect,
            welcome,
            new_squad,
            telemetry,
            ask,
            ssh_key_wanted: false,
            pairing: pair::Pairing::default(),
            engines: engines::Engines::default(),
            setup: setup::Setup::default(),
            forgetting: None,
            disk: Default::default(),
            usage_open: false,
            zoom_hud: None,
            _pair_tick: None,
            notices,
            jobs,
            sheet: None,
            confirm: None,
            pending_team: None,
            pending_mode: None,
            connecting_shown: false,
            connecting: views::connect::Connecting::new(std::time::Instant::now()),
            log_renders: std::env::var_os("EQUIPE_LOG_RENDER").is_some(),
            accept_asked: std::collections::HashMap::new(),
            renders: 0,
            started: std::time::Instant::now(),
            look_teller: Default::default(),
            link: None,
            _pump: None,
            _connecting: None,
            _updates: None,
            _notices: None,
            _jobs: None,
            _fetched: None,
            _prompts: None,
            _subscriptions: Vec::new(),
        };
        // The size the window was left at.
        zoom::apply(shell.remembered.zoom.0, window);
        if let Some(width) = shell.remembered.sidebar_width {
            shell.board.sidebar_width = width.clamp(views::sidebar::MIN_WIDTH, views::sidebar::MAX_WIDTH);
        }
        shell.chat.follow = !shell.remembered.whole_conversation;
        shell.board.view.layout = if shell.remembered.tickets_board { tickets::Layout::Board } else { tickets::Layout::List };
        // What macOS allows, and whether this window is in front of the person.
        shell.start_bell(window, cx);
        // An update downloaded in the background: the sidebar offers the restart.
        let mut updates = relaunch::watch();
        shell._updates = Some(cx.spawn_in(window, async move |this, cx| {
            while updates.changed().await.is_ok() {
                let state = updates.borrow_and_update().clone();
                if this.update_in(cx, |this, window, cx| {
                    // On the way to the newest: the person already said restart — it goes in at once.
                    if state.ready().is_some() && relaunch::chaining() {
                        relaunch::log("the newest is downloaded: putting it in now, as the restart asked");
                        if relaunch::now() {
                            return;
                        }
                    }
                    // Found and downloaded: each told once (and remembered across launches).
                    if let Some(alert) = notify::update_alert(&state, &mut this.remembered.update_told) {
                        this.remembered.save(&equipe_home());
                        this.announce(vec![alert], window, cx);
                    }
                    this.update = state;
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
        shell._jobs = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some((after, answer)) = finished.recv().await {
                if this.update_in(cx, |this, window, cx| this.finish_job(after, answer, window, cx)).is_err() {
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
        let name_input = shell.connect.name_input.clone();
        shell._subscriptions.push(cx.subscribe_in(&name_input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.finish_naming(cx);
            }
        }));
        shell._subscriptions.push(cx.subscribe_in(&connect_input, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { .. } => this.choose_first(window, cx),
            // The list narrows as it is typed; a refusal goes with the text that earned it.
            InputEvent::Change => {
                this.connect.error = None;
                cx.notify();
            }
            _ => {}
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
        // Compare and review's words: ⏎ sends the draft back with them.
        let design_redo = shell.design_redo.clone();
        shell._subscriptions.push(cx.subscribe_in(&design_redo, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { .. } => this.design_redo_send(window, cx),
            InputEvent::Change => cx.notify(),
            _ => {}
        }));
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
            screen_look::used();
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
                } else if key == "tab" && !modifiers.platform && !modifiers.control && !modifiers.alt && this.update(cx, |shell, _| shell.new_squad.open && shell.ask.queue.is_empty()).unwrap_or(false) {
                    this.update(cx, |shell, cx| shell.new_squad_tab(modifiers.shift, window, cx)).ok();
                    cx.stop_propagation();
                } else if key == "enter" && this.update(cx, |shell, _| shell.confirm.as_ref().is_some_and(|a| !a.destructive()) && shell.ask.queue.is_empty()).unwrap_or(false) {
                    this.update(cx, |shell, cx| shell.answer_question(true, window, cx)).ok();
                    cx.stop_propagation();
                }
                return;
            }
            // Typing in a "your turn" card's box or one of the browser panel's: T, N,
            // y, 1… are letters there, not shortcuts. Esc still leaves; Tab and ⌘⏎ are the card's own.
            if key != "escape" && this.update(cx, |shell, cx| shell.turn.typing(window, cx) || shell.secrets.typing(window, cx) || shell.browser.read(cx).typing(window, cx) || shell.design_taste.typing(window, cx)).unwrap_or(false) {
                return;
            }
            // Writing to a member on its request: Esc closes the box; every other
            // key is the box's (⏎ and ⌘⏎ answer from it).
            match this.update(cx, |shell, cx| shell.on_words_key(key, window, cx)).ok().flatten() {
                Some(true) => {
                    cx.stop_propagation();
                    return;
                }
                Some(false) => return,
                None => {}
            }
            // A deck on show: ←/→, Space, Esc — and nothing typed underneath.
            if this.update(cx, |shell, cx| shell.on_deck_key(key, &modifiers, cx)).unwrap_or(false) {
                cx.stop_propagation();
                return;
            }
            // A member's picture shown big: ←/→, a pick, Esc — and nothing typed underneath.
            if this.update(cx, |shell, cx| shell.on_viewer_key(key, &modifiers, window, cx)).unwrap_or(false) {
                cx.stop_propagation();
                return;
            }
            // Words selected in the conversation: ⏎ or ⌘⇧R asks about them; in
            // the box that opens, ⏎ sends and Esc closes.
            match this.update(cx, |shell, cx| shell.on_quote_key(key, &modifiers, window, cx)).unwrap_or(views::decide::Took::Pass) {
                views::decide::Took::Taken => {
                    cx.stop_propagation();
                    return;
                }
                views::decide::Took::Typing => return,
                views::decide::Took::Pass => {}
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
            // A reply's list of questions as a form: Tab, Esc — and its boxes' typing;
            // ⏎ on a "1." in the composer opens it.
            match this.update(cx, |shell, cx| shell.on_asklist_key(key, &modifiers, window, cx)).unwrap_or(views::decide::Took::Pass) {
                views::decide::Took::Taken => {
                    cx.stop_propagation();
                    return;
                }
                views::decide::Took::Typing => return,
                views::decide::Took::Pass => {}
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
            // A change to the team the PM proposed: ⌘⏎ approves it, above an empty composer or in the peek.
            // Work to confirm: ⌘⏎ confirms what is ticked, above an empty composer or in the peek.
            if this.update(cx, |shell, cx| shell.on_accept_key(key, &modifiers, cx)).unwrap_or(false) {
                cx.stop_propagation();
                return;
            }
            if this.update(cx, |shell, cx| shell.on_team_key(key, &modifiers, cx)).unwrap_or(false) {
                cx.stop_propagation();
                return;
            }
            // A routine the PM proposed: ⌘⏎ turns it on, above an empty composer or in the peek.
            if this.update(cx, |shell, cx| shell.on_routine_key(key, &modifiers, cx)).unwrap_or(false) {
                cx.stop_propagation();
                return;
            }
            // Design system's keys: sections, the lightbox.
            if this.update(cx, |shell, cx| shell.on_design_key(key, &modifiers, window, cx)).unwrap_or(false) {
                cx.stop_propagation();
                return;
            }
            // The work screens' keys: A R N…, moving, the peek, search.
            if this.update(cx, |shell, cx| shell.on_board_key(key, &modifiers, window, cx)).unwrap_or(false) {
                cx.stop_propagation();
                return;
            }
            // y / a / ⇧a / n / o answer a request for permission while nothing is typed.
            if matches!(key, "y" | "a" | "n" | "o" | "r") && !(modifiers.control || modifiers.alt || modifiers.platform || modifiers.function) {
                if this.update(cx, |shell, cx| shell.on_approval_key(key, modifiers.shift, window, cx)).unwrap_or(false) {
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
        // Several windows: what is drawn inside text views reaches the shell of
        // the window in front — where the click is.
        shell._subscriptions.push(cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                cx.set_global(render::ShellRef(cx.entity().downgrade()));
            }
        }));
        shell.load_peers(cx);
        shell.tell_looks(window, cx);
        // One of the app's windows, on its own machine and squad.
        let first = windows::none_open();
        let squad = opening.squad.clone().unwrap_or_default();
        windows::opened(shell.window_id, window.window_handle(), cx.weak_entity(), windows::Seat { endpoint: endpoint.clone(), squad });
        let mut first_link = Target::new(endpoint, env!("CARGO_PKG_VERSION"));
        if let Some(squad) = opening.squad {
            first_link.prefer = Some(squad);
        }
        shell.attach(first_link, window, cx);
        shell.composer.focus(window, cx);
        // The app's first window takes what other apps share and greets the
        // person (on the screen they chose to start on); one more only goes
        // where it was opened to go.
        if first {
            shell.watch_inbox(cx);
            if shell.remembered.start != crate::target::Start::Pm {
                shell.goal = None;
                shell.open_start(window, cx);
            }
        }
        // "Connect to another machine…" in a new window: the chooser, over it.
        if opening.chooser {
            shell.open_chooser(window, cx);
        }
        shell
    }

    /// The app's first window closed and this one is the first now: what
    /// only the first does (what other apps share) is its.
    pub fn take_first_duties(&mut self, cx: &mut Context<Self>) {
        if self.composer.inbox_watch.is_none() {
            self.watch_inbox(cx);
        }
    }

    /// The window's machine and squad as the app's list knows them.
    pub fn seat(&self) -> windows::Seat {
        windows::Seat { endpoint: self.endpoint.clone(), squad: self.model.squad.clone() }
    }

    /// Tell the app's list where this window is, and the Mac what it is
    /// called ("equipe-squad · Mac mini"): each draw, only what changed.
    fn sync_window(&mut self, window: &mut Window) {
        let seat = self.seat();
        let spot = windows::Spot::of(self.place(), self.board.project_page.as_deref());
        if self.seat_told.as_ref() != Some(&(seat.clone(), spot.clone())) {
            let route = seat.route();
            if let Some((old, _)) = &self.seat_told
                && old.route() != route
                && window.is_window_active()
            {
                notify::set_in_sight(old.route(), false);
                notify::set_in_sight(route, true);
            }
            windows::moved(self.window_id, &seat, &spot);
            self.seat_told = Some((seat, spot));
        }
        let title = windows::title(&self.model.squad, &self.place_name());
        if title != self.title_shown {
            window.set_window_title(&title);
            self.title_shown = title;
        }
    }

    /// Bring what depends on the model up to date: the draft in the box and the
    /// conversation list.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.follow(&self.model, window, cx);
        self.turn_sync(window, cx);
        self.decide_sync(window, cx);
        self.secrets_sync(window, cx);
        self.chat_sync(cx);
        self.board_sync(cx);
        self.sync_browser(window, cx);
        self.reach_goal(window, cx);
    }

    /// Tell the browser panel what it follows: this daemon and squad, the
    /// conversation on screen and who each conversation is.
    fn sync_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut who: std::collections::HashMap<SessionId, views::browser::Who> = self
            .model
            .members
            .iter()
            .map(|m| (m.session, views::browser::Who { name: m.name.clone(), role: m.role.clone(), hue: theme::look(&m.role, &m.name).1, working: m.presence == state::Presence::Working }))
            .collect();
        for lane in &self.model.lanes {
            if let Some(member) = who.get(&lane.of).cloned() {
                let working = lane.presence == state::Presence::Working;
                who.insert(lane.session, views::browser::Who { working, ..member });
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
        self.await_reply(&command);
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

    /// A project chip (or ⌃0–⌃9): the squad's focus, the PM's thread, the board's filter.
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
            self.open_project_page(name, window, cx);
        }
    }

    /// Follow `target`'s squad from scratch, letting go of any earlier link.
    fn attach(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        self.model = Model::new(target.prefer.as_deref().unwrap_or(""));
        self.model.remote = target.endpoint.is_remote();
        self.design.reset();
        self.sheet = None;
        self.confirm = None;
        self.connecting_shown = false;
        // Another machine: its files aren't the ones open, nor its shell.
        self.doc = None;
        self.terminal = None;
        self.connecting = views::connect::Connecting::new(std::time::Instant::now());
        // Phones are the daemon's machine's: another machine has its own. A
        // code open closes with the old connection.
        let _ = self.pairing.close();
        self.pairing.devices = None;
        self.pairing.devices_asked = None;
        // Another machine: its files aren't the ones known.
        self.chat.sent.borrow_mut().clear();
        self.chat.gallery = None;
        self.shots.borrow_mut().clear();
        self.viewer = None;
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
                        // The squad's design system: the design screen's, never the model's.
                        let update = match update {
                            daemon::Update::Event(Event::Design { squad, systems, folder, taste }) => {
                                changed |= this.design_arrived(squad, systems, folder, taste);
                                continue;
                            }
                            daemon::Update::Event(Event::DesignVersions { squad, project, versions }) => {
                                changed |= this.design_versions_arrived(squad, project, versions);
                                continue;
                            }
                            // Briefs and plans: the project pages'.
                            daemon::Update::Event(Event::ProjectDocs { squad, docs }) => {
                                changed |= this.docs_arrived(squad, docs);
                                continue;
                            }
                            other => other,
                        };
                        // The shared Chrome's tabs and hands: the browser panel's.
                        if let daemon::Update::Event(
                            event @ (Event::BrowserTabs { .. } | Event::BrowserControl { .. } | Event::BrowserDialog { .. } | Event::BrowserDialogClosed { .. } | Event::BrowserWants { .. }),
                        ) = &update
                        {
                            // Told the squad first: these come right after the snapshot.
                            this.sync_browser(window, cx);
                            this.browser.update(cx, |b, cx| b.on_event(event, window, cx));
                            // A page waiting for the person: Needs you, and a notification.
                            changed |= this.model.on_browser(event);
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
                        // The disk: the banner's and the panel's, never the model's.
                        if let daemon::Update::Event(event @ (Event::DiskLevel { .. } | Event::DiskReport { .. } | Event::DiskCleaned { .. })) = &update {
                            changed |= this.disk.on_event(event);
                            // Given back on equipe's own: said in a line, once.
                            if let Some(freed) = this.disk.take_auto() {
                                this.composer.flash = Some(t!("disk.auto_cleaned", size = equipe_core::disk::size(freed)));
                            }
                            continue;
                        }
                        // The engines: settings' AI platforms, never the model's.
                        if let daemon::Update::Event(event @ (Event::Engines { .. } | Event::Nebius { .. } | Event::SignIn { .. } | Event::EngineAuth { .. } | Event::EngineInstall { .. })) = &update {
                            let effect = this.engines.on_event(event);
                            changed |= effect.changed;
                            // The first engine ready: a squad that had none starts its team now.
                            if this.engines.became_ready() && this.model.attached() {
                                // What said no AI was ready is said no longer.
                                this.composer.problem = None;
                                this.start_team();
                            }
                            // The code's page, in this Mac's browser, for the screen that asked.
                            if let Some(url) = effect.open
                                && !cfg!(feature = "snapshot")
                            {
                                cx.open_url(&url);
                            }
                            // Installed as the person asked: straight on to its sign-in.
                            if let Some(engine) = effect.sign_in
                                && !cfg!(feature = "snapshot")
                            {
                                match engines::platform(engine).signs_in {
                                    engines::SignsIn::FromApp => this.engine_sign_in(engine, cx),
                                    engines::SignsIn::InTerminal { .. } if !this.endpoint.is_remote() => this.engine_terminal(engine, cx),
                                    engines::SignsIn::InTerminal { .. } => {}
                                }
                            }
                            continue;
                        }
                        // How far a connection got: its screen's, never the model's.
                        if let daemon::Update::Leg(_) | daemon::Update::Door(_) = &update {
                            changed |= this.connecting.hear(&update, std::time::Instant::now());
                            continue;
                        }
                        if let daemon::Update::Down(_) = &update {
                            changed |= this.connecting.hear(&update, std::time::Instant::now());
                            this.pairing.disconnected();
                            this.disk.disconnected();
                            this.engines.disconnected();
                        }
                        if let daemon::Update::Attached(hello) = &update {
                            this.disk.attached(hello.snapshot.disk.clone());
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
                            daemon::Update::Attached(_) => {
                                this.chat.sent.borrow_mut().retry_gone();
                                this.shots.borrow_mut().retry_gone();
                                // The daemon tells its browser's tabs afresh: none held from before.
                                this.browser.update(cx, |b, cx| b.reattached(window, cx));
                            }
                            _ => {}
                        }
                        let attached = matches!(update, daemon::Update::Attached(_));
                        let ended = match &update {
                            daemon::Update::Event(Event::TurnEnded { session, .. }) => Some(*session),
                            _ => None,
                        };
                        let back = match &update {
                            daemon::Update::Event(Event::ConversationBack { session, .. }) => Some(*session),
                            _ => None,
                        };
                        changed |= this.model.apply(update, now);
                        // A conversation taken up again from its project's page: opened.
                        if let Some(session) = back.filter(|s| this.board.reopening == Some(*s)) {
                            this.board.reopening = None;
                            this.open_side(state::Side::Conversation(session), window, cx);
                        }
                        // A card to confirm shows each ticket's size: asked of git once per look.
                        this.accept_stats();
                        // A reply asking the person to run something as the machine's
                        // administrator: the terminal opens with it typed, not run.
                        if let Some(session) = ended {
                            this.offer_terminal(session, window, cx);
                        }
                        // A conversation ending on a lost sign-in: why is asked, if not told yet.
                        this.signin_watch();
                        // Settings open on its list: the daemon (maybe another one) is asked again.
                        if attached {
                            this.engines.attached();
                            this.telemetry_look(cx);
                            this.install_ssh_key(cx);
                            // The daemon back on this app's release: the update's line has said its piece.
                            if this.daemon_update.is_some() && notify::stale_daemon(&this.model.daemon_version, env!("CARGO_PKG_VERSION")).is_none() {
                                this.daemon_update = None;
                                this.daemon_restart_armed = false;
                            }
                            // The projects in the order the person left them on this Mac.
                            let squad = this.model.squad.clone();
                            this.model.project_order = this.remembered.project_orders.iter().find(|(s, _)| *s == squad).map(|(_, n)| n.clone()).unwrap_or_default();
                            this.model.order_projects();
                        }
                        if attached && this.screen == Screen::Settings {
                            this.pairing.devices_asked = None;
                            this.ask_devices(cx);
                            if this.settings.category == settings::Category::Engines {
                                this.ask_engines(cx);
                            }
                        }
                        if attached {
                            this.start_team();
                            this.apply_pending_mode();
                            // A new connection: the design system is asked for again, if on screen.
                            this.design.reset();
                        }
                    }
                    // Told while the window is out of sight; in sight, the cards say it.
                    // Only the kinds the person wants (settings → Notifications).
                    let alerts = this.model.take_alerts();
                    this.announce(alerts, window, cx);
                    this.speak(cx);
                    this.sync_badge();
                    if let Some(flash) = this.model.take_flash() {
                        this.composer.flash = Some(flash);
                        changed = true;
                    }
                    if let Some(problem) = this.model.take_problem() {
                        this.composer.problem = Some(problem);
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
        // Quick connections show nothing; a slow one says so, with its seconds.
        self._connecting = Some(self.tick_connecting(CONNECTING_AFTER, cx));
    }

    /// This window to squad `name`, on its machine: the other windows stay where they are.
    pub fn select_squad(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if name == self.model.squad {
            return;
        }
        let mut target = Target::new(self.endpoint.clone(), env!("CARGO_PKG_VERSION"));
        target.prefer = Some(name.to_owned());
        self.attach(target, window, cx);
        cx.notify();
    }

    /// How a window opened from this one starts: on its machine and squad.
    pub fn opening_beside(&self, goal: Option<views::elsewhere::Goal>) -> windows::Opening {
        windows::Opening { endpoint: Some(self.endpoint.clone()), squad: Some(self.model.squad.clone()).filter(|s| !s.is_empty()), goal, chooser: false }
    }

    /// New Window (⌘⇧N): another window on this machine and squad, to point
    /// elsewhere without touching this one.
    pub fn new_window(&mut self, squad: Option<String>, cx: &mut Context<Self>) {
        let mut opening = self.opening_beside(None);
        if squad.is_some() {
            opening.squad = squad;
        }
        views::elsewhere::open(opening, cx);
    }

    /// "Connect to another machine…" in a new window: one opens beside this,
    /// its chooser open — this window stays on its machine.
    pub fn new_window_elsewhere(&mut self, cx: &mut Context<Self>) {
        let opening = windows::Opening { chooser: true, ..self.opening_beside(None) };
        views::elsewhere::open(opening, cx);
    }

    // ── which machine ──

    /// Show the chooser: this Mac, the hosts `~/.ssh/config` names, or one typed.
    pub fn open_chooser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dir) = target::ssh_dir() {
            self.connect.hosts = target::config_hosts(&dir);
        }
        self.connect.open = true;
        self.connect.error = None;
        self.connect.show_hidden = false;
        self.probe_hosts(cx);
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
        // From a guide: the try is shown leg by leg, and the guide comes back if it fails.
        if self.endpoint.is_remote() && matches!(self.model.conn, Conn::Down { had_snapshot: false, .. }) {
            self.connecting = views::connect::Connecting { retrying: true, door: self.connecting.door.take(), ..views::connect::Connecting::new(std::time::Instant::now()) };
            self.connecting_shown = true;
            self._connecting = Some(self.tick_connecting(Duration::ZERO, cx));
        }
        cx.notify();
    }

    /// The connecting screen, from `after` on and each second while it shows:
    /// its seconds count and its hints come in on time.
    fn tick_connecting(&self, after: Duration, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            loop {
                let showing = this.update(cx, |this, cx| {
                    let showing = this.model.conn == Conn::Connecting || (this.connecting.retrying && matches!(this.model.conn, Conn::Down { had_snapshot: false, .. }));
                    if showing {
                        this.connecting_shown = true;
                        cx.notify();
                    }
                    showing
                });
                if !matches!(showing, Ok(true)) {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
        })
    }

    // ── questions from ssh ──

    pub fn overlay_open(&self) -> bool {
        self.connect.open || !self.ask.queue.is_empty() || self.new_squad.open || self.pairing.open || self.disk.open || self.usage_open || self.confirm.is_some() || self.sheet.is_some() || self.bell.sheet
    }

    fn escape_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ask.queue.is_empty() {
            self.answer_prompt(askpass::Answer::Cancel, window, cx);
        } else if self.new_squad.open {
            self.close_new_squad(window, cx);
        } else if self.pairing.open {
            self.close_pairing(window, cx);
        } else if self.disk.open {
            self.close_disk(window, cx);
        } else if self.usage_open {
            self.close_usage(window, cx);
        } else if self.bell.sheet {
            self.notify_later(window, cx);
        } else if self.connect.open {
            self.close_chooser(window, cx);
        } else if self.confirm.is_some() {
            self.answer_question(false, window, cx);
        } else {
            self.close_info(window, cx);
        }
    }

    /// `ssh` wants something from the person: a box over the window.
    fn ask_user(&mut self, prompt: askpass::Prompt, window: &mut Window, cx: &mut Context<Self>) {
        self.ask.queue.push_back(prompt);
        // ssh asks once the machine answered: signing in.
        self.connecting.reached(daemon::Leg::SigningIn, std::time::Instant::now());
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
        // A password typed with "don't ask again" on: once it works, the key goes there.
        let password = self.ask.queue.front().is_some_and(|q| matches!(&q.ask, askpass::Ask::Secret { label } if label.to_lowercase().contains("password")));
        if password && self.ask.remember && self.endpoint.is_remote() {
            self.ssh_key_wanted = true;
        }
        self.ask.input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        self.answer_prompt(askpass::Answer::Text(text), window, cx);
    }

    /// The viewed conversation's last reply, if it asks the person to run a
    /// `sudo` command: typed into the terminal for them, once per reply.
    fn offer_terminal(&mut self, session: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen != Screen::Chat || self.model.viewed() != Some(session) {
            return;
        }
        let Some(text) = self.model.convo().and_then(|c| {
            c.blocks().into_iter().rev().find_map(|b| match b {
                convo::Block::Reply { text, .. } => Some(text),
                _ => None,
            })
        }) else {
            return;
        };
        let Some(block) = term::shell_blocks(&text).into_iter().find(|b| term::wants_person(b)) else { return };
        let key = render::hash_of(&text);
        if self.term_offered == Some(key) {
            return;
        }
        self.term_offered = Some(key);
        self.open_terminal(Some(term::command_of(&block)), window, cx);
        self.composer.flash = Some(t!("term.offered").into());
    }

    // ── the document panel ──

    /// Open `path` (on the daemon's machine) in the panel at the right, read
    /// off the main thread; the same one again is read again.
    /// Each ticket, for [`doc::places`]: its number, whether it is the viewed
    /// member's, and where its work is.
    pub fn ticket_places(&self) -> Vec<(u64, bool, doc::Place)> {
        let mine = self.model.viewed().and_then(|s| self.model.members.iter().find(|m| m.session == s)).map(|m| m.name.clone());
        self.model
            .tickets
            .iter()
            .map(|t| (u64::from(t.id), mine.is_some() && (t.owner == mine || t.assignee == mine), (t.worktree.clone(), t.branch.clone(), t.project.clone())))
            .collect()
    }

    pub fn open_doc(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        self.doc_reads += 1;
        let read = self.doc_reads;
        match &mut self.doc {
            // Read again in place: where it was scrolled to stays.
            Some(doc) if doc.path == path => (doc.read, doc.folded, doc.fresh) = (read, false, false),
            _ => self.doc = Some(doc::Doc::new(path.clone(), read)),
        }
        let endpoint = self.endpoint.clone();
        // Written in a ticket's worktree, or on its branch only: looked for there too.
        let base = render::base_dir(self);
        let also = doc::elsewhere(&path, &base, &doc::places(&path, &self.ticket_places()));
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || drop(tx.send(doc::read(&endpoint, &path, &also))));
        cx.spawn(async move |this, cx| {
            let Ok(got) = rx.await else { return };
            let _ = this.update(cx, |this, cx| {
                let Some(doc) = this.doc.as_mut().filter(|d| d.read == read) else { return };
                match got {
                    Ok((text, cut)) => (doc.text, doc.cut) = (Some(Ok(text)), cut),
                    Err(why) => doc.text = Some(Err(why)),
                }
                // A deck opened afresh is shown as it was made to be: slide by slide.
                if std::mem::take(&mut doc.fresh) && doc.slides().is_some_and(|s| !s.is_empty()) {
                    doc.presenting = Some(0);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// A web link from a reply: in the browser the person chose (settings),
    /// or with ⌘ (`other`) in the other — equipe's panel or the Mac's.
    pub fn open_link(&mut self, url: &str, other: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.remembered.links_in_app == other {
            cx.open_url(url);
            return;
        }
        if self.screen != Screen::Chat {
            self.show(Screen::Chat, cx);
        }
        // The right side is the browser's now.
        self.doc = None;
        if let Some(ui) = self.terminal.as_mut().filter(|_| !self.remembered.term_below) {
            ui.open = false;
        }
        if !self.browser.read(cx).open {
            self.browser.update(cx, |b, cx| b.toggle(window, cx));
            self.sync_browser(window, cx);
        }
        self.browser.update(cx, |b, cx| b.open_tab(url, cx));
        cx.notify();
    }

    pub fn set_links_in_app(&mut self, on: bool) {
        self.remembered.links_in_app = on;
        self.remembered.save(&equipe_providers::credentials::equipe_home());
    }

    /// Signed in with a password and asked not to be again: the app's key
    /// goes onto that machine over the connection now open (`sshkey.rs`).
    fn install_ssh_key(&mut self, cx: &mut Context<Self>) {
        let Endpoint::Ssh(host) = self.endpoint.clone() else { return };
        if !std::mem::take(&mut self.ssh_key_wanted) || sshkey::keyed(&host) {
            return;
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        let target = host.clone();
        std::thread::spawn(move || {
            let _ = tx.send(sshkey::install(&target));
        });
        cx.spawn(async move |this, cx| {
            let done = rx.await.unwrap_or_else(|_| Err("stopped".into()));
            let _ = this.update(cx, |this, cx| {
                this.composer.flash = Some(match done {
                    Ok(()) => t!("connect.remembered", who = host.spec()),
                    Err(why) => t!("connect.remember_failed", why = why),
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// The person makes `path` the squad's HQ, or none: the daemon keeps it
    /// and tells the PM; every screen hears it back (`ProjectsChanged`).
    pub fn set_hq(&mut self, path: Option<std::path::PathBuf>, cx: &mut Context<Self>) {
        self.send_command(equipe_core::protocol::Command::SetHq { squad: self.model.squad.clone(), path });
        cx.notify();
    }

    /// Project `name` dragged before `before`: kept for this squad on this Mac.
    pub fn reorder_project(&mut self, name: &str, before: Option<&str>, cx: &mut Context<Self>) {
        if before == Some(name) {
            return;
        }
        let names = self.model.move_project(name, before);
        let squad = self.model.squad.clone();
        self.remembered.project_orders.retain(|(s, _)| *s != squad);
        self.remembered.project_orders.push((squad, names));
        self.remembered.save(&equipe_providers::credentials::equipe_home());
        self.board_sync(cx);
        cx.notify();
    }

    /// The side panel's width now: as the person left it, within the window.
    pub fn side_width(&self, window: &Window) -> f32 {
        let sidebar = if self.board.sidebar_hidden { 0. } else { self.board.sidebar_width };
        doc::kept_width(self.remembered.side_width, window.viewport_size().width.into(), sidebar)
    }

    fn side_released(&mut self, cx: &mut Context<Self>) {
        if self.side_drag.take().is_some() {
            self.remembered.save(&equipe_providers::credentials::equipe_home());
            cx.notify();
        }
    }

    /// Folded to a rail at the edge, or open again.
    pub fn fold_doc(&mut self, folded: bool, cx: &mut Context<Self>) {
        if let Some(doc) = self.doc.as_mut() {
            doc.folded = folded;
            cx.notify();
        }
    }

    pub fn reload_doc(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.doc.as_ref().map(|d| d.path.clone()) {
            self.open_doc(path, cx);
        }
    }

    pub fn close_doc(&mut self, cx: &mut Context<Self>) {
        self.doc = None;
        cx.notify();
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
    /// "Restart to Update": the downloaded update goes in, and the app starts again.
    pub fn relaunch_to_update(&mut self, cx: &mut Context<Self>) {
        // A newer release than the downloaded one: on to it after this restart.
        relaunch::chain_if_newer();
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
            notify::Go::Browser(page) => self.reveal_browser(page, cx),
            // Downloaded: the notice's click restarts into it; before, the window shows the pill.
            notify::Go::Update => {
                if self.update.ready().is_some() {
                    self.relaunch_to_update(cx);
                }
            }
            notify::Go::Relaunch => self.relaunch_to_update(cx),
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

    /// The screen beside the sidebar, drawn in its pane (`pane.rs`).
    pub fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let nothing_attached = matches!(self.model.conn, Conn::Down { had_snapshot: false, .. });
        // A Mac without equipe is told how to install; a far machine's missing
        // equipe is that machine's guide, below.
        if nothing_attached && !self.endpoint.is_remote() && !install::cli_installed() {
            return install_screen(p);
        }
        let remote = self.endpoint.is_remote();
        match &self.model.conn {
            Conn::Down { had_snapshot: false, .. } | Conn::Connecting if remote && self.connecting_shown && (self.connecting.retrying || self.model.conn == Conn::Connecting) => {
                v_flex().flex_1().child(self.drag_strip()).child(views::connect::connecting_screen(self, cx)).into_any_element()
            }
            Conn::Down { reason: failure::Failure::NoSquads, had_snapshot: false, .. } if !remote => v_flex().flex_1().min_h_0().child(self.drag_strip()).child(views::setup::screen(self, cx)).into_any_element(),
            Conn::Down { reason, had_snapshot: false, .. } => v_flex().flex_1().child(self.drag_strip()).child(views::connect::guide_screen(self, &reason.guide_at(&self.endpoint, self.connecting.door.as_deref()), cx)).into_any_element(),
            Conn::Connecting => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(BODY))
                .text_color(p.muted)
                .when(self.connecting_shown, |d| d.child(t!("main.connecting_local")))
                .into_any_element(),
            // Asked once, before the squad's screens: in the first run's last step, and the first time a squad already there opens after the update.
            conn if self.telemetry_asking() => v_flex().flex_1().min_w_0().h_full().children(self.banner(conn, cx)).child(views::telemetry::screen(self, cx)).into_any_element(),
            conn => {
                let screen = match self.screen {
                    Screen::Chat => views::chat::screen(self, cx).into_any_element(),
                    // Needs you, In progress, All work, a saved view: the menu item chosen.
                    Screen::Board => views::board::render(self, cx).into_any_element(),
                    Screen::Settings => views::settings::render(self, cx),
                    Screen::Design => views::design::render(self, cx),
                    Screen::Roadmap => views::roadmap::render(self, cx),
                    Screen::Metrics => views::metrics::render(self, cx),
                    Screen::Project => views::project::render(self, cx),
                    Screen::Progress => views::progress::render(self, cx),
                };
                v_flex().flex_1().min_w_0().h_full().children(self.banner(conn, cx)).children(views::disk::banner(self, cx)).child(screen).into_any_element()
            }
        }
    }

    /// A strip to drag the window by, over a screen with no header of its
    /// own: the title bar's height and hairline (its buttons float above).
    fn drag_strip(&self) -> impl IntoElement + use<> {
        ui::title_bar(div().h(px(views::sidebar::STRIP)).flex_none().w_full().border_b_1().border_color(self.palette.hair))
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
        let width = self.board.sidebar_width;
        let side = self.panes.sidebar.clone().cached(StyleRefinement::default().w(px(width)).h_full().flex_none()).into_any_element();
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
                .child(icons::icon(icons::Icon::XCircle, theme::ICON_SM, p.text))
                .child(div().text_color(p.text).child(format!("{} · {since}", guide.short)))
                .child(div().flex_1().min_w_0().text_color(p.muted).child(if waits { t!("main.banner_needs_you") } else { t!("main.banner_reconnecting") }))
                .child(ui::button(p, "banner-retry", ui::Kind::Secondary, t!("main.retry")).icon(icons::Icon::Retry).on_click(cx.listener(|this, _, _, cx| this.retry(cx)))),
        )
    }
}

/// What runs on the daemon's machine: the latest equipe over the one there
/// (`equipe update`, else the install script), then a restart once its
/// members are idle, left to run on its own. An `equipe` that is the app's
/// own (a link into an .app) is new with the app already: only the restart.
const UPDATE_SCRIPT: &str = r#"exe=$(command -v equipe) || { curl -fsSL https://github.com/equipe-labs/equipe/releases/latest/download/install.sh | sh || exit 1; exe=$HOME/.local/bin/equipe; }
case "$(readlink "$exe" 2>/dev/null)" in
  *.app/*) ;;
  *) equipe update >/dev/null 2>&1 || curl -fsSL https://github.com/equipe-labs/equipe/releases/latest/download/install.sh | sh >/dev/null 2>&1 || { echo "the update didn't install" >&2; exit 1; } ;;
esac
nohup equipe daemon restart --when-idle >/dev/null 2>&1 &
"#;

/// The restart now, over a member's turn: the restart left waiting for the
/// members to be idle gives way to one at once (`[i]` so the pattern doesn't
/// match this script's own shell).
const RESTART_NOW_SCRIPT: &str = r#"pkill -f 'daemon restart --when-[i]dle' 2>/dev/null
command -v equipe >/dev/null || PATH=$HOME/.local/bin:$PATH
nohup equipe daemon restart >/dev/null 2>&1 &
"#;

impl Shell {
    /// The daemon as the update pill needs it: where it runs — a Tailscale
    /// device by its name (`tinums-mac-mini`), not its address — its release,
    /// and where its own update stands.
    pub(crate) fn daemon_view(&self) -> relaunch::Daemon {
        let place = match &self.endpoint {
            Endpoint::Ssh(host) => tailnet::peer_for(&self.connect.peers, &host.host).map(|peer| peer.short().to_owned()),
            Endpoint::Local { .. } => None,
        }
        .unwrap_or_else(|| self.endpoint.label());
        let connected = matches!(self.model.conn, Conn::Connected { .. });
        relaunch::Daemon {
            place,
            theirs: self.model.daemon_version.split('+').next().unwrap_or_default().to_owned(),
            older: connected && notify::stale_daemon(&self.model.daemon_version, env!("CARGO_PKG_VERSION")).is_some(),
            remote: self.endpoint.is_remote(),
            update: self.daemon_update.clone(),
            armed: self.daemon_restart_armed,
        }
    }

    /// The daemon's machine first, then the app into its waiting version.
    pub(crate) fn update_both(&mut self, cx: &mut Context<Self>) {
        self.relaunch_after_daemon = true;
        self.update_daemon(cx);
    }

    /// Update the daemon's machine, off the main thread.
    pub(crate) fn update_daemon(&mut self, cx: &mut Context<Self>) {
        self.daemon_update = Some(DaemonUpdate::Running);
        self.daemon_restart_armed = false;
        self.on_daemon_machine(UPDATE_SCRIPT, DaemonUpdate::Done, cx);
    }

    /// The person's second click: the daemon starts again on the new
    /// release now, members mid-turn or not; the app reconnects on its own.
    /// What this screen shows the app in, looked at every
    /// [`screen_look::LOOK_EVERY`] and told to the daemon a moment after it
    /// changes, or as the person uses it. Never from the real view.
    fn tell_looks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if readonly::on() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            loop {
                if this.update_in(cx, |this, window, cx| this.tell_look(window, cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(screen_look::LOOK_EVERY).await;
            }
        })
        .detach();
    }

    fn tell_look(&mut self, window: &Window, cx: &App) {
        let look = self.screen_look(window, cx);
        screen_look::keep(look.clone());
        if let Some(told) = self.look_teller.next(look, screen_look::used_at(), std::time::Instant::now())
            && !self.link.as_ref().is_some_and(|link| link.tell_look(&told))
        {
            self.look_teller.forget();
        }
    }

    /// The look this window shows now.
    fn screen_look(&self, window: &Window, cx: &App) -> equipe_core::protocol::look::ScreenLook {
        use equipe_core::protocol::look::{ScreenLook, Theme};
        let size = window.bounds().size;
        ScreenLook {
            machine: screen_look::machine().to_owned(),
            app: equipe_core::design::flows::DESKTOP_APP.to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            commit: option_env!("EQUIPE_COMMIT").map(str::to_owned),
            lang: crate::i18n::lang().code().to_owned(),
            theme: if gpui_kit::component::Theme::global(cx).mode.is_dark() { Theme::Dark } else { Theme::Light },
            width: f32::from(size.width).round() as u32,
            height: f32::from(size.height).round() as u32,
            sidebar_width: self.remembered.sidebar_width.map(|w| w.round() as u32),
            sidebar_hidden: self.board.sidebar_hidden,
            folded: self.remembered.folded.iter().cloned().collect(),
            team_open: self.remembered.team_open,
            usage_folded: self.remembered.usage_folded,
            design_talk: self.remembered.design_talk,
            zoom: self.remembered.zoom.0,
            idle: None,
        }
    }

    /// The window's size, kept a moment after it stops changing (a drag
    /// sends many) — not while it fills the screen.
    fn keep_window_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static TURN: AtomicU64 = AtomicU64::new(0);
        let WindowBounds::Windowed(bounds) = window.window_bounds() else { return };
        let size = (f32::from(bounds.size.width).round(), f32::from(bounds.size.height).round());
        if self.remembered.window == Some(size) {
            return;
        }
        self.remembered.window = Some(size);
        let turn = TURN.fetch_add(1, Ordering::SeqCst) + 1;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(600)).await;
            if TURN.load(Ordering::SeqCst) == turn {
                let _ = this.update(cx, |this, _| this.remembered.save(&equipe_home()));
            }
        })
        .detach();
    }

    pub(crate) fn restart_daemon_now(&mut self, cx: &mut Context<Self>) {
        self.daemon_update = Some(DaemonUpdate::Restarting);
        self.daemon_restart_armed = false;
        self.on_daemon_machine(RESTART_NOW_SCRIPT, DaemonUpdate::Restarting, cx);
    }

    /// A script run where the daemon runs (this Mac, or over SSH), off the
    /// main thread; `then` is the banner's state once it succeeded.
    fn on_daemon_machine(&mut self, script: &'static str, then: DaemonUpdate, cx: &mut Context<Self>) {
        // Never the person's daemon updated or restarted from the real view.
        if readonly::blocked("the daemon, running") {
            return;
        }
        let endpoint = self.endpoint.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let done = match &endpoint {
                Endpoint::Local { .. } => {
                    let path = format!("{}/.local/bin:/opt/homebrew/bin:/usr/local/bin:{}", std::env::var("HOME").unwrap_or_default(), std::env::var("PATH").unwrap_or_default());
                    match std::process::Command::new("sh").arg("-c").arg(script).env("PATH", path).stdin(std::process::Stdio::null()).output() {
                        Ok(out) if out.status.success() => Ok(()),
                        Ok(out) => Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()),
                        Err(e) => Err(e.to_string()),
                    }
                }
                Endpoint::Ssh(host) => tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| e.to_string())
                    .and_then(|rt| rt.block_on(ssh::Launcher::system(None).run(host, script)).map(|_| ()).map_err(|f| f.detail().map_or_else(|| f.guide(&endpoint).short, str::to_owned))),
            };
            let _ = tx.send(done);
        });
        cx.spawn(async move |this, cx| {
            let done = rx.await.unwrap_or_else(|_| Err("stopped".into()));
            let _ = this.update(cx, |this, cx| {
                let installed = done.is_ok() && then == DaemonUpdate::Done;
                this.daemon_update = Some(match done {
                    Ok(()) => then,
                    Err(why) => DaemonUpdate::Failed(if why.is_empty() { t!("main.update_daemon_unknown").into() } else { why }),
                });
                // Both asked for: the daemon is on the new release, now the app.
                if std::mem::take(&mut this.relaunch_after_daemon) && installed {
                    this.relaunch_to_update(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
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
                .font_family(theme::MONO)
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
        self.design_redo.update(cx, |s, cx| s.set_placeholder(t!("design.compare_redo_placeholder"), window, cx));
        self.browser.update(cx, |b, cx| b.relocalize(window, cx));
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The metrics cards pick their layout by the width the screen has.
        self.metrics.room = f32::from(window.viewport_size().width) - self.board.sidebar_width;
        // A message held for its uploads goes as soon as they're up.
        if let Some(later) = self.composer.send_when_ready.filter(|_| !self.composer.strip.busy()) {
            self.composer.send_when_ready = None;
            cx.defer_in(window, move |this, window, cx| this.send_as(later, window, cx));
        }
        if !self.setup.probed && self.needs_setup() {
            self.setup.probed = true;
            cx.defer_in(window, |this, _, cx| this.setup_probe(cx));
        }
        if self.board.words != crate::i18n::lang() {
            self.relocalize_inputs(window, cx);
        }
        self.board.relocalize(window, cx);
        self.board_prepare(window, cx);
        self.design_prepare(window, cx);
        // What the person did here (accepted, answered, a project in focus) changes Needs you too.
        self.sync_badge();
        self.sync_window(window);
        self.renders += 1;
        self.decide.room = f32::from(window.viewport_size().height);
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
            .sans()
            .text_color(p.text)
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &DismissOverlay, window, cx| {
                if this.overlay_open() {
                    this.escape_overlay(window, cx);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &NewMessage, window, cx| this.new_message(window, cx)))
            .on_action(cx.listener(|this, _: &SearchAll, window, cx| this.search_everything(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.toggle_settings(window, cx)))
            .on_action(cx.listener(|this, _: &PairPhone, window, cx| this.open_pairing(window, cx)))
            // The Squad menu's "New Squad…" is the switcher's: the same sheet.
            .on_action(cx.listener(|this, _: &NewSquad, window, cx| this.open_new_squad(window, cx)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
                screen_look::used();
                // A selection dragged out in the terminal follows the pointer anywhere.
                views::terminal::dragged(this, e, window, cx);
                // A hairline between the sidebar's areas, held.
                if this.board.side_split_drag.is_some() {
                    if e.pressed_button == Some(MouseButton::Left) {
                        this.side_split_dragged(f32::from(e.position.y), cx);
                    } else {
                        this.side_split_released(cx);
                    }
                }
                if this.board.sidebar_drag.is_some() {
                    if e.pressed_button == Some(MouseButton::Left) {
                        this.sidebar_dragged(f32::from(e.position.x), cx);
                    } else {
                        this.sidebar_released(cx);
                    }
                }
                // The terminal below grows as its top edge goes up.
                if let Some((y0, h0)) = this.term_drag {
                    if e.pressed_button == Some(MouseButton::Left) {
                        this.remembered.term_height = Some(h0 + y0 - f32::from(e.position.y));
                        cx.notify();
                    } else if this.term_drag.take().is_some() {
                        this.remembered.save(&equipe_providers::credentials::equipe_home());
                    }
                }
                // The board's ticket panel too.
                if let Some((x0, w0)) = this.board.peek_drag {
                    if e.pressed_button == Some(MouseButton::Left) {
                        this.remembered.peek_width = Some((w0 + x0 - f32::from(e.position.x)).max(views::board::PEEK_LEAST));
                        cx.notify();
                    } else if this.board.peek_drag.take().is_some() {
                        this.remembered.save(&equipe_providers::credentials::equipe_home());
                    }
                }
                // The design screen's conversation too.
                if this.design.talk.drag.is_some() {
                    this.design_talk_dragged(f32::from(e.position.x), e.pressed_button == Some(MouseButton::Left), cx);
                }
                // The side panel grows as its left edge goes left.
                if let Some((x0, w0)) = this.side_drag {
                    if e.pressed_button == Some(MouseButton::Left) {
                        this.remembered.side_width = Some(w0 + x0 - f32::from(e.position.x));
                        cx.notify();
                    } else {
                        this.side_released(cx);
                    }
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, window, cx| {
                views::terminal::released(this, e.position, window, cx);
                this.sidebar_released(cx);
                this.side_split_released(cx);
                this.side_released(cx);
                if this.design.talk.drag.is_some() {
                    this.design_talk_dragged(0., false, cx);
                }
                if this.board.peek_drag.take().is_some() {
                    this.remembered.save(&equipe_providers::credentials::equipe_home());
                    cx.notify();
                }
                if this.term_drag.take().is_some() {
                    this.remembered.save(&equipe_providers::credentials::equipe_home());
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleBoard, _, cx| {
                let next = if this.screen == Screen::Chat { Screen::Board } else { Screen::Chat };
                this.show(next, cx);
            }))
            .on_action(cx.listener(|this, _: &ChooseTarget, window, cx| this.open_chooser(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleBrowser, window, cx| this.toggle_browser(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleTerminal, window, cx| this.toggle_terminal(window, cx)))
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
            }))
            // A ticket's card on the pointer takes Esc, and its keys while the pointer is on it.
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if views::ticket_card::key(this, e, window, cx) {
                    cx.stop_propagation();
                }
            }));
        let root = menu_listeners(root, cx);
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
                .children(views::new_squad::sheet(self, cx))
                .children(views::pair::sheet(self, cx))
                .children(views::disk::sheet(self, cx))
                .children(views::usage::panel(self, cx))
                .children(views::bell::sheet(self, cx))
                .children(views::connect::question(self, cx))
                .children(self.zoom_chip(window, cx))
                .children(tips::layer(cx));
        }
        // Before the first connection there is nothing to list: no sidebar
        // (it showed only its ⌘N), the connecting or setup screen alone.
        let away = matches!(self.model.conn, Conn::Down { had_snapshot: false, .. } | Conn::Connecting);
        let sidebar = if away { None } else { self.sidebar(window, cx) };
        // A document open takes the right side; closed, the browser is back.
        // A project's page opens its briefs and plans there too.
        let doc = matches!(self.screen, Screen::Chat | Screen::Project).then(|| views::doc::panel(self, window, cx)).flatten();
        // Then the terminal, sized to the panel as the window is now.
        views::terminal::fit(self, window, cx);
        let terminal = (self.screen == Screen::Chat).then(|| views::terminal::panel(self, window, cx)).flatten();
        // Under the conversation, or at its right where the document isn't.
        let (below, terminal) = if self.remembered.term_below { (terminal, None) } else { (None, terminal) };
        let doc = doc.or(terminal);
        let browser = (doc.is_none() && self.screen == Screen::Chat && self.browser.read(cx).open && !matches!(self.model.conn, Conn::Down { had_snapshot: false, .. } | Conn::Connecting)).then(|| self.browser.clone());
        // The design screen: the designer's conversation at its right (`views/design_talk.rs`).
        let talk = if away { None } else { views::design_talk::panel(self, window, cx) };
        let body = self.panes.body.clone().cached(StyleRefinement::default().flex_1().min_w_0().min_h_0());
        // What doesn't fit the room beside the panel is cut at its edge, not drawn under it.
        let middle = v_flex().flex_1().min_w_0().h_full().overflow_hidden().child(body).children(below);
        root.relative().child(h_flex().flex_1().min_h_0().children(sidebar).child(middle).children(doc).children(talk).children(browser))
            // The title bar's fixed buttons, by the traffic lights, over the
            // sidebar or the header alike: the same place whatever slides.
            .child(views::sidebar::title_buttons(self, cx))
            // Over everything: the chooser, then a question from ssh above it.
            .children(views::quote::layer(self, window, cx))
            .children(views::quote::peek(self))
            .children(views::composer::drop_overlay(self, cx))
            .children(views::composer::lightbox(self, cx))
            .children(views::design::lightbox(self, cx))
            .children(views::attached::gallery(self, cx))
            .children(views::visuals::viewer(self, window, cx))
            .children(views::deck::presentation(self, window, cx))
            .children(views::sheet::sheet(self, cx))
            .children(views::sheet::question(self, cx))
            .children(self.connect.open.then(|| views::connect::chooser(self, cx)))
            .children(views::new_squad::sheet(self, cx))
            .children(views::pair::sheet(self, cx))
            .children(views::disk::sheet(self, cx))
                .children(views::usage::panel(self, cx))
            .children(views::bell::sheet(self, cx))
            .children(views::connect::question(self, cx))
            .children(self.zoom_chip(window, cx))
            .child(views::ticket_card::layer(cx))
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
    cx.on_action(|_: &Quit, cx| {
        // Every window open now opens again next time.
        windows::quitting();
        cx.quit();
    });
    // New Window: beside the window in front, on its machine and squad; with
    // none open, as the app was left.
    cx.on_action(|_: &NewWindow, cx| {
        let front = cx.active_window().and_then(|active| windows::shells().into_iter().find(|(h, _)| h.window_id() == active.window_id())).and_then(|(_, s)| s.upgrade());
        match front {
            Some(shell) => shell.update(cx, |shell, cx| shell.new_window(None, cx)),
            None => views::elsewhere::open(windows::Opening::default(), cx),
        }
    });
    cx.on_action(|_: &MinimizeWindow, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.minimize_window());
        }
    });
    cx.on_action(|_: &ZoomWindow, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.zoom_window());
        }
    });
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
    ]), edit_menu(), squad_menu(), member_menu(), view_menu(), window_menu(), help_menu()]);
    // The Mac lists the windows under it, by their titles ("equipe-squad · Mac mini").
    windows::windows_menu_at(WINDOW_MENU_AT);
}

/// Where the Window menu is in the menu bar: after equipe, Edit, Squad, Member and View.
const WINDOW_MENU_AT: isize = 5;

/// The Window menu: one more window, and the Mac's own items; macOS lists
/// the open windows under them.
fn window_menu() -> Menu {
    Menu::new(t!("menu.window")).items([
        MenuItem::action(t!("menu.new_window"), NewWindow),
        MenuItem::separator(),
        MenuItem::action(t!("menu.minimize"), MinimizeWindow),
        MenuItem::action(t!("menu.zoom_window"), ZoomWindow),
    ])
}

/// ⌘+ ⌘- ⌘0 and the View menu: the app's to answer, whatever has the focus
/// — or nothing at all (a screen whose text box left the window).
fn answer_zoom(handle: AnyWindowHandle, shell: Entity<Shell>, cx: &mut App) {
    let zoom = move |step: zoom::Step| {
        let shell = shell.clone();
        // Later: the window is in use while it dispatches the key.
        move |cx: &mut App| {
            let shell = shell.clone();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| shell.update(cx, |shell, cx| shell.zoom(step, window, cx)));
            });
        }
    };
    let (zoom_in, zoom_out, actual) = (zoom(zoom::Step::In), zoom(zoom::Step::Out), zoom(zoom::Step::Actual));
    cx.on_action(move |_: &ZoomIn, cx| zoom_in(cx));
    cx.on_action(move |_: &ZoomOut, cx| zoom_out(cx));
    cx.on_action(move |_: &ActualSize, cx| actual(cx));
}

/// The View menu: the window's size, as a browser zooms.
fn view_menu() -> Menu {
    Menu::new(t!("main.menu_view")).items([
        MenuItem::action(t!("main.menu_zoom_in"), ZoomIn),
        MenuItem::action(t!("main.menu_zoom_out"), ZoomOut),
        MenuItem::action(t!("main.menu_actual_size"), ActualSize),
        MenuItem::separator(),
        MenuItem::action(t!("menu.palette"), CommandPalette),
        MenuItem::action(t!("menu.toggle_board"), ToggleBoard),
        MenuItem::action(t!("menu.all_projects"), AllProjects),
        MenuItem::action(t!("menu.toggle_folds"), ToggleFolds),
    ])
}

/// The window's keys. ⌘0 is actual size, as everywhere on the Mac, so the
/// project slices are ⌃0–⌃9. ⌘+ comes first: the menu shows the first.
fn window_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-b", ToggleBoard, None),
        KeyBinding::new("cmd-shift-b", ToggleBrowser, None),
        KeyBinding::new("ctrl-`", ToggleTerminal, None),
        KeyBinding::new("cmd-\\", ToggleSidebar, None),
        KeyBinding::new("cmd-n", NewMessage, None),
        KeyBinding::new("cmd-k", SearchAll, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-+", ZoomIn, None),
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd-shift-=", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("cmd-0", ActualSize, None),
        KeyBinding::new("ctrl-0", AllProjects, None),
        KeyBinding::new("ctrl-1", Project1, None),
        KeyBinding::new("ctrl-2", Project2, None),
        KeyBinding::new("ctrl-3", Project3, None),
        KeyBinding::new("ctrl-4", Project4, None),
        KeyBinding::new("ctrl-5", Project5, None),
        KeyBinding::new("ctrl-6", Project6, None),
        KeyBinding::new("ctrl-7", Project7, None),
        KeyBinding::new("ctrl-8", Project8, None),
        KeyBinding::new("ctrl-9", Project9, None),
        KeyBinding::new("cmd-shift-o", ToggleFolds, None),
        KeyBinding::new("cmd-f", FindInConversation, None),
        KeyBinding::new("escape", CloseFind, Some("FindBar")),
        KeyBinding::new("cmd-shift-k", ChooseTarget, None),
        KeyBinding::new("cmd-shift-p", CommandPalette, None),
        KeyBinding::new("cmd-u", ShowUsage, None),
        KeyBinding::new("cmd-/", ShowHelp, None),
        KeyBinding::new("cmd-shift-h", OpenSquadFolder, None),
        KeyBinding::new("cmd-shift-e", ExportConversation, None),
        KeyBinding::new("cmd-shift-n", NewWindow, None),
        KeyBinding::new("alt-cmd-n", AddMembers, None),
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        // ⌥⏎ and ⌃J break the line like ⇧⏎ (T-54 §8).
        KeyBinding::new("alt-enter", Enter { secondary: false, shift: true }, Some("Input")),
        KeyBinding::new("ctrl-j", Enter { secondary: false, shift: true }, Some("Input")),
    ]
}

/// The Edit menu: the text box's own actions, under the selectors macOS knows
/// — which is also where it adds "Start Dictation…" and "Emoji & Symbols".
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

/// The Squad menu: its items are command lines.
fn squad_menu() -> Menu {
    Menu::new(t!("menu.squad")).items([
        MenuItem::action(t!("menu.switch_squad"), SwitchSquad),
        MenuItem::action(t!("menu.new_squad"), NewSquad),
        MenuItem::action(t!("menu.add_folder"), AddFolder),
        MenuItem::action(t!("menu.remove_folder"), RemoveFolder),
        MenuItem::separator(),
        MenuItem::action(t!("menu.open_squad_folder"), OpenSquadFolder),
        MenuItem::action(t!("menu.home_changes"), HomeChanges),
        MenuItem::action(t!("menu.home_undo"), HomeUndo),
        MenuItem::separator(),
        MenuItem::action(t!("menu.open_memory"), OpenSquadMemory),
        MenuItem::action(t!("menu.memory_changes"), MemoryChanges),
        MenuItem::separator(),
        MenuItem::action(t!("menu.export"), ExportConversation),
        MenuItem::action(t!("menu.tidy"), TidyBoard),
    ])
}

/// The Member menu: its items are command lines.
fn member_menu() -> Menu {
    Menu::new(t!("menu.member")).items([
        MenuItem::action(t!("menu.add_members"), AddMembers),
        MenuItem::action(t!("menu.model"), SetModel),
        MenuItem::action(t!("menu.mode"), SetMode),
        MenuItem::separator(),
        MenuItem::action(t!("menu.compact"), CompactMember),
        MenuItem::action(t!("menu.clear"), ClearMember),
        MenuItem::action(t!("menu.close_member"), CloseMember),
        MenuItem::separator(),
        MenuItem::action(t!("menu.usage"), ShowUsage),
        MenuItem::separator(),
        MenuItem::action(t!("menu.browser_signin"), BrowserSignin),
        MenuItem::action(t!("menu.browser_show"), BrowserShow),
        MenuItem::action(t!("menu.browser_hide"), BrowserHide),
        MenuItem::action(t!("menu.browser_max"), BrowserMax),
        MenuItem::action(t!("menu.send_apps"), SendApps),
        MenuItem::separator(),
        MenuItem::action(t!("menu.screen_back"), ScreenBack),
        MenuItem::action(t!("menu.screen_open"), ScreenOpen),
        MenuItem::separator(),
        MenuItem::action(t!("menu.suggest_on"), SuggestOn),
        MenuItem::action(t!("menu.suggest_off"), SuggestOff),
    ])
}

/// The Help menu: its items are command lines.
fn help_menu() -> Menu {
    Menu::new(t!("menu.help")).items([MenuItem::action(t!("menu.shortcuts"), ShowHelp)])
}


fn main() {
    // `ssh` starts this same binary to ask the person a question: answer it
    // through the running app and leave, without opening a window.
    if let Some(code) = askpass::helper() {
        std::process::exit(code);
    }
    // The real view (a snapshot build on the person's own daemon): their
    // language and look, read from their own state, before any word is shown.
    if let Some(look) = readonly::begin() {
        i18n::set(look.lang);
        // Asked by `scripts/design-shot-live.sh` before it picks a build:
        // the version the person's own screen runs, and where.
        if std::env::args().any(|a| a == "--person-look") {
            if let Some(from) = &look.from {
                println!("{}\t{}", from.version, from.machine);
            }
            return;
        }
    }
    // The language picked for every screen — this app's own pick from before
    // becomes that, once — else the Mac's, before any word is shown.
    i18n::migrate(&equipe_home());
    // From the bundle: notices without a window or a daemon, to see macOS show them (docs/DESKTOP.md).
    if std::env::args().any(|a| a == "--notify-test") {
        views::bell::notify_test();
        return;
    }
    // Images a reply points at on the web come through this client.
    let http = std::sync::Arc::new(reqwest_client::ReqwestClient::user_agent("equipe-desktop").unwrap_or_else(|_| reqwest_client::ReqwestClient::new()));
    // The app's icons, then GPUI Kit's own (the sidebar's, the components').
    let app = gpui_kit::application().with_http_client(http).with_assets(icons::AppAssets);
    // The real view: no Dock icon, no menu bar — nothing of it on the screen.
    let app = if readonly::on() { app.with_windowing(WindowingRequest::Headless) } else { app };
    // The window closed (⌘W, the red button) leaves the app in the Dock, as a
    // Mac app does: a click on its icon then opens the window again. Without
    // this the click did nothing and the app looked dead.
    app.on_reopen(|cx| {
        match cx.windows().first() {
            Some(window) => {
                let _ = window.update(cx, |_, window, _| window.activate_window());
            }
            None => {
                open_kept_windows(cx);
            }
        }
        cx.activate(true);
    });
    app.run(|cx| {
        gpui_kit::init(cx);
        // The notification centre's delegate, before a click can come back;
        // a click goes to the window showing the squad it came from.
        notify::start();
        follow_notice_clicks(cx);
        // A window closed: the others take on what it did; quitting keeps them all.
        cx.on_window_closed(|cx, id| windows::closed(id.as_u64(), cx)).detach();
        cx.on_app_quit(|_| {
            windows::quitting();
            async {}
        })
        .detach();
        tips::init(cx);
        render::code::warm();
        cx.bind_keys(window_bindings());
        app_menu(cx);
        views::composer::bind_keys(cx);
        #[allow(unused_variables)]
        let (handle, shell) = open_kept_windows(cx);
        // The real view draws to its pictures alone: it never comes forward.
        if !readonly::on() {
            cx.activate(true);
        }
        #[cfg(feature = "snapshot")]
        snapshot::run(handle, shell, cx);
    });
}

/// The smallest the window goes, in points.
const MIN_WINDOW: (f32, f32) = (760., 520.);

/// Where a click on a notice goes: the window showing the squad it came
/// from (else the first), which comes to the front there.
fn follow_notice_clicks(cx: &mut App) {
    let (clicks, mut clicked) = tokio::sync::mpsc::unbounded_channel::<notify::Click>();
    notify::follow_clicks(clicks);
    cx.spawn(async move |cx| {
        while let Some(click) = clicked.recv().await {
            cx.update(|cx| {
                let Some((handle, shell)) = windows::clicked(click.route) else { return };
                let Some(shell) = shell.upgrade() else { return };
                let _ = handle.update(cx, |_, window, cx| shell.update(cx, |shell, cx| shell.go(click.go, window, cx)));
            });
        }
    })
    .detach();
}

/// The windows the app was left with, each on its machine and squad and
/// where it was — the first in front; one window as before when none were
/// kept (or a machine is named for this launch: `EQUIPE_TARGET`). A machine
/// out of reach shows its window reconnecting, as any dropped line does.
fn open_kept_windows(cx: &mut App) -> (AnyWindowHandle, Entity<Shell>) {
    let kept = if readonly::on() || std::env::var_os("EQUIPE_TARGET").is_some() { Vec::new() } else { windows::load(&equipe_home()) };
    let mut openings = kept.iter().map(windows::Kept::opening);
    let first = open_app_window(openings.next().unwrap_or_default(), cx);
    let more = openings.len();
    for opening in openings {
        open_app_window(opening, cx);
    }
    if more > 0 {
        let _ = first.0.update(cx, |_, window, _| window.activate_window());
    }
    first
}

/// A window of the app, with the conversation in it: at launch, from the
/// Dock after it was closed, and for one more (New Window, a place opened
/// elsewhere, `views::elsewhere`) — a little down and right of the others,
/// so it is seen to open. All in one tab group, so the Mac can make tabs of
/// them.
pub fn open_app_window(opening: windows::Opening, cx: &mut App) -> (AnyWindowHandle, Entity<Shell>) {
    {
        let others = cx.windows().len() as f32;
        // As large as the person left it; the real view, as their app is.
        let (width, height) = match readonly::look() {
            Some(look) => look.window,
            None => Remembered::load(&equipe_home()).window.unwrap_or(readonly::WINDOW),
        };
        let mut bounds = Bounds::centered(None, size(px(width.max(MIN_WINDOW.0)), px(height.max(MIN_WINDOW.1))), cx);
        if others > 0. {
            bounds.origin.x += px(28. * others);
            bounds.origin.y += px(28. * others);
        }
        let hidden = readonly::on();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(MIN_WINDOW.0), px(MIN_WINDOW.1))),
            show: !hidden,
            focus: !hidden,
            titlebar: Some(TitlebarOptions {
                title: Some("equipe".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(views::sidebar::LIGHTS_AT.0), px(views::sidebar::LIGHTS_AT.1))),
            }),
            tabbing_identifier: Some("equipe".to_owned()),
            ..Default::default()
        };
        let (handle, shell) = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(opening, window, cx))).expect("failed to open window");
        answer_zoom(handle, shell.clone(), cx);
        (handle, shell)
    }
}

#[cfg(test)]
mod tests {
    use super::window_bindings;
    use gpui_kit::Keystroke;

    /// What a keystroke is bound to in the window's table, by action name.
    fn bound(keys: &str) -> Vec<&'static str> {
        let typed = [Keystroke::parse(keys).unwrap()];
        window_bindings().iter().filter(|b| b.match_keystrokes(&typed) == Some(false)).map(|b| b.action().name()).collect()
    }

    #[test]
    fn zoom_has_the_macs_keys_and_the_slices_moved_to_control() {
        assert_eq!(bound("cmd-0"), ["equipe::ActualSize"], "⌘0 is actual size, as everywhere on the Mac");
        assert_eq!(bound("cmd-="), ["equipe::ZoomIn"]);
        assert_eq!(bound("cmd--"), ["equipe::ZoomOut"]);
        // ⌘+ on a US keyboard is ⌘⇧= typing "+": either binding takes it.
        assert!(bound("cmd-shift-=->+").iter().all(|a| *a == "equipe::ZoomIn") && !bound("cmd-shift-=->+").is_empty());
        assert_eq!(bound("ctrl-0"), ["equipe::AllProjects"]);
        for n in 1..=9 {
            assert_eq!(bound(&format!("ctrl-{n}")), [format!("equipe::Project{n}").as_str()]);
            assert!(bound(&format!("cmd-{n}")).is_empty(), "⌘{n} is free again");
        }
    }

    #[test]
    fn a_new_window_has_the_macs_keys() {
        assert_eq!(bound("cmd-shift-n"), ["equipe::NewWindow"], "⌘⇧N: one more window, as browsers and editors open one");
        assert_eq!(bound("alt-cmd-n"), ["equipe::AddMembers"], "adding members moved over for it");
        assert_eq!(bound("cmd-m"), ["equipe::MinimizeWindow"]);
        assert_eq!(bound("cmd-n"), ["equipe::NewMessage"], "⌘N stays a new message");
    }

    #[test]
    fn no_key_is_bound_twice() {
        let bindings = window_bindings();
        let mut seen = std::collections::HashSet::new();
        for b in &bindings {
            let keys = b.keystrokes().iter().map(|k| k.unparse()).collect::<Vec<_>>().join(" ");
            let context = b.predicate().map(|p| p.to_string()).unwrap_or_default();
            assert!(seen.insert((keys.clone(), context)), "{keys} is bound twice");
        }
        // The menu shows the first binding of each: ⌘+, not ⌘=.
        let first_zoom_in = bindings.iter().find(|b| b.action().name() == "equipe::ZoomIn").unwrap();
        assert_eq!(first_zoom_in.keystrokes()[0].unparse(), "cmd-+");
    }
}
