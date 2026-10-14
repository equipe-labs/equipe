//! The equipe desktop app: a window onto the daemon. It attaches to the
//! daemon's socket, follows one squad, shows its members and talks with them;
//! the screens live in `views/` (the board is its own ticket).
//!
//! It draws only when the daemon says something changed or the person acts —
//! see docs/DESKTOP.md. `EQUIPE_LOG_RENDER=1` prints a line per frame, to see that.

mod commands;
mod compose;
mod convo;
mod daemon;
mod mem;
#[cfg(feature = "snapshot")]
mod snapshot;
mod state;
mod theme;
mod views;

use std::time::Duration;

use equipe_core::gateway;
use equipe_providers::credentials::equipe_home;
use gpui_kit::component::input::{Enter, InputEvent};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use daemon::{Backoff, Link, Target};
use equipe_core::protocol::{Command, SessionId};
use state::{Clock, Conn, Model, squad_names};
use theme::{BODY, Palette, SANS, SMALL};
use views::chat::Chat;
use views::composer::Composer;

actions!(
    equipe,
    [ToggleBoard, AllProjects, Project1, Project2, Project3, Project4, Project5, Project6, Project7, Project8, Project9, ToggleFolds]
);

/// How long the daemon gets to answer before the window says it is connecting.
const CONNECTING_AFTER: Duration = Duration::from_millis(400);
/// Updates that arrive within this window are drawn together: a streaming
/// reply draws at most this often (T-54 §3.3), and not at all once it stops.
const COALESCE: Duration = Duration::from_millis(80);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Chat,
    Board,
}

pub struct Shell {
    pub palette: Palette,
    pub model: Model,
    pub squads: Vec<String>,
    pub screen: Screen,
    pub chat: Chat,
    pub composer: Composer,
    /// The ticket a `T-n` link asked the board to open.
    pub board_ticket: Option<u32>,
    /// Why there is nothing to attach to (no squad yet), when so.
    problem: Option<String>,
    connecting_shown: bool,
    log_renders: bool,
    renders: u64,
    started: std::time::Instant,
    // Dropping these ends the link's thread and the tasks that feed from it.
    link: Option<Link>,
    _pump: Option<Task<()>>,
    _connecting: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_appearance(window, |this, window, cx| {
            this.palette = Palette::new(window.appearance());
            cx.notify();
        })
        .detach();
        let mut shell = Self {
            palette: Palette::new(window.appearance()),
            model: Model::new(""),
            squads: squad_names(&equipe_home()),
            screen: Screen::Chat,
            chat: Chat::new(cx),
            composer: Composer::new(window, cx),
            board_ticket: None,
            problem: None,
            connecting_shown: false,
            log_renders: std::env::var_os("EQUIPE_LOG_RENDER").is_some(),
            renders: 0,
            started: std::time::Instant::now(),
            link: None,
            _pump: None,
            _connecting: None,
            _subscriptions: Vec::new(),
        };
        let state = shell.composer.state.clone();
        shell._subscriptions.push(cx.subscribe_in(&state, window, |this, _, event: &InputEvent, window, cx| this.on_composer_event(event, window, cx)));
        shell._subscriptions.push(cx.observe_in(&state, window, |this, _, window, cx| this.on_composer_touched(window, cx)));
        // Esc, ↑↓, Tab and Enter get to the composer's completion and history
        // before the text box's own bindings take them.
        let this = cx.entity().downgrade();
        shell._subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            let key = event.keystroke.key.as_str();
            if !matches!(key, "escape" | "up" | "down" | "tab" | "enter") {
                return;
            }
            let modifiers = event.keystroke.modifiers;
            let taken = this.update(cx, |shell, cx| shell.on_composer_key(key, &modifiers, window, cx)).unwrap_or(false);
            if taken {
                cx.stop_propagation();
            }
        }));
        match Target::from_env(env!("CARGO_PKG_VERSION")) {
            Ok(target) => shell.attach(target, window, cx),
            Err(problem) => shell.problem = Some(problem),
        }
        shell.composer.focus(window, cx);
        shell
    }

    /// Bring what depends on the model up to date: the draft in the box and the
    /// conversation list.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.follow(&self.model, window, cx);
        self.chat.sync(&self.model, cx);
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
        self.model = Model::new(&target.squad);
        self.connecting_shown = false;
        let (link, mut updates) = daemon::spawn(target, Backoff::default());
        self.link = Some(link);

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
                    let mut changed = false;
                    for update in batch {
                        // A squad may have been made since: look again on attach.
                        if matches!(update, daemon::Update::Attached(_)) {
                            let squads = squad_names(&equipe_home());
                            if squads != this.squads {
                                this.squads = squads;
                                changed = true;
                            }
                        }
                        changed |= this.model.apply(update, now);
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
        let home = equipe_home();
        self.attach(
            Target { socket: gateway::socket_path(&home), squad: name.to_owned(), version: env!("CARGO_PKG_VERSION").to_owned() },
            window,
            cx,
        );
        cx.notify();
    }

    pub fn show(&mut self, screen: Screen, cx: &mut Context<Self>) {
        if self.screen != screen {
            self.screen = screen;
            cx.notify();
        }
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        if let Some(problem) = &self.problem {
            return empty_state(p, "아직 스쿼드가 없습니다", problem, None);
        }
        match &self.model.conn {
            Conn::Down { since, had_snapshot: false, .. } => empty_state(
                p,
                "equipe 데몬이 실행 중이 아닙니다",
                "터미널에서 `equipe`를 실행하면 데몬이 시작됩니다. 켜지는 즉시 이 창이 알아서 연결합니다.",
                Some(format!("~/.equipe/equipe.sock을 {since}에 찾아봤습니다")),
            ),
            Conn::Connecting => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(BODY))
                .text_color(p.muted)
                .when(self.connecting_shown, |d| d.child("데몬에 연결하는 중…"))
                .into_any_element(),
            conn => {
                let screen = match self.screen {
                    Screen::Chat => views::chat::screen(self, cx).into_any_element(),
                    Screen::Board => v_flex().flex_1().min_h_0().child(views::header::render(self, cx)).child(views::board::render(self)).into_any_element(),
                };
                v_flex().flex_1().min_w_0().h_full().children(banner(p, conn)).child(screen).into_any_element()
            }
        }
    }
}

/// Shown over the last state when the daemon goes away after we had it.
fn banner(p: Palette, conn: &Conn) -> Option<impl IntoElement + use<>> {
    let Conn::Down { since, had_snapshot: true, .. } = conn else { return None };
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
            .child(div().text_color(p.text).child(format!("데몬과의 연결이 {since}에 끊겼습니다.")))
            .child(div().text_color(p.muted).child("다시 연결하는 중 — 데몬이 켜지면 자동으로 이어집니다")),
    )
}

/// The body when there is nothing to show: a heading, a sentence, how to fix it.
fn empty_state(p: Palette, title: &str, text: &str, note: Option<String>) -> AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .child(div().text_size(px(18.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title.to_owned()))
        .child(div().max_w(px(480.)).text_size(px(BODY)).line_height(px(24.)).text_color(p.text2).text_center().child(text.to_owned()))
        .child(
            div()
                .px_3()
                .py_1()
                .rounded(px(8.))
                .bg(p.code_bg)
                .border_1()
                .border_color(p.hair)
                .font_family("Menlo")
                .text_size(px(SMALL))
                .text_color(p.text2)
                .child("equipe"),
        )
        .children(note.map(|n| div().text_size(px(SMALL)).text_color(p.muted).child(n)))
        .into_any_element()
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .on_action(cx.listener(|this, _: &ToggleBoard, _, cx| {
                let next = if this.screen == Screen::Chat { Screen::Board } else { Screen::Chat };
                this.show(next, cx);
            }))
            .on_action(cx.listener(|this, _: &AllProjects, window, cx| this.focus_project(None, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleFolds, _, cx| this.toggle_folds(cx)));
        let root = project_actions!(
            root, cx, Project1 => 1, Project2 => 2, Project3 => 3, Project4 => 4, Project5 => 5, Project6 => 6, Project7 => 7, Project8 => 8, Project9 => 9
        );
        root
            .child(
                // The title bar: drags the window; the traffic lights sit at its left.
                div()
                    .h(px(40.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .window_control_area(WindowControlArea::Drag)
                    .text_size(px(SMALL))
                    .text_color(p.muted)
                    .child(self.model.squad.clone()),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(p.hair)
                    .child(views::sidebar::render(self, cx))
                    .child(div().flex_1().min_w_0().h_full().flex().flex_col().child(self.body(cx))),
            )
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-b", ToggleBoard, None),
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
            // ⌥⏎ and ⌃J break the line like ⇧⏎ (T-54 §8).
            KeyBinding::new("alt-enter", Enter { secondary: false, shift: true }, Some("Input")),
            KeyBinding::new("ctrl-j", Enter { secondary: false, shift: true }, Some("Input")),
        ]);
        let bounds = Bounds::centered(None, size(px(1200.), px(760.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(760.), px(520.))),
            titlebar: Some(TitlebarOptions {
                title: Some("equipe".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(14.), px(13.))),
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
