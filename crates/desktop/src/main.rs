//! The equipe desktop app: a window onto the daemon. It attaches to the
//! daemon's socket, follows one squad, and shows its members; screens for the
//! conversation and the board live in `views/`.
//!
//! It draws only when the daemon says something changed or the person acts —
//! see docs/DESKTOP.md. `EQUIPE_LOG_RENDER=1` prints a line per frame, to see that.

mod daemon;
mod mem;
mod state;
mod theme;
mod views;

use std::time::Duration;

use equipe_core::gateway;
use equipe_providers::credentials::equipe_home;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use daemon::{Backoff, Link, Target};
use state::{Clock, Conn, Model, squad_names};
use theme::{BODY, Palette, SANS, SMALL};

/// How long the daemon gets to answer before the window says it is connecting.
const CONNECTING_AFTER: Duration = Duration::from_millis(400);
/// Updates that arrive within this window are drawn together.
const COALESCE: Duration = Duration::from_millis(100);

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
            problem: None,
            connecting_shown: false,
            log_renders: std::env::var_os("EQUIPE_LOG_RENDER").is_some(),
            renders: 0,
            started: std::time::Instant::now(),
            link: None,
            _pump: None,
            _connecting: None,
        };
        match Target::from_env(env!("CARGO_PKG_VERSION")) {
            Ok(target) => shell.attach(target, cx),
            Err(problem) => shell.problem = Some(problem),
        }
        shell
    }

    /// Follow `target`'s squad from scratch, letting go of any earlier link.
    fn attach(&mut self, target: Target, cx: &mut Context<Self>) {
        self.model = Model::new(&target.squad);
        self.connecting_shown = false;
        let (link, mut updates) = daemon::spawn(target, Backoff::default());
        self.link = Some(link);

        // The daemon's updates, drawn together when they land close to each
        // other, and only if something on screen differs.
        self._pump = Some(cx.spawn(async move |this, cx| {
            while let Some(first) = updates.recv().await {
                let mut batch = vec![first];
                cx.background_executor().timer(COALESCE).await;
                while let Ok(more) = updates.try_recv() {
                    batch.push(more);
                }
                let alive = this.update(cx, |this, cx| {
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

    pub fn select_squad(&mut self, name: &str, cx: &mut Context<Self>) {
        if name == self.model.squad {
            return;
        }
        let home = equipe_home();
        self.attach(
            Target { socket: gateway::socket_path(&home), squad: name.to_owned(), version: env!("CARGO_PKG_VERSION").to_owned() },
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

    fn body(&self) -> AnyElement {
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
                    Screen::Chat => views::chat::render(self).into_any_element(),
                    Screen::Board => views::board::render(self).into_any_element(),
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
        v_flex()
            .size_full()
            .bg(p.bg)
            .font_family(SANS)
            .text_color(p.text)
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
                    .child(div().flex_1().min_w_0().h_full().flex().flex_col().child(self.body())),
            )
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
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
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(window, cx))).expect("failed to open window");
        cx.activate(true);
    });
}
