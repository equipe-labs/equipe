mod mem;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use equipe_core::mock::MockAgent;
use equipe_core::{LogKind, Store, Ticket, TicketStatus};
use gpui_kit::component::*;
use gpui_kit::*;

/// How often the UI looks for new log rows. Agents may emit thousands of lines a
/// second; the screen only ever repaints at this rate, and only when something
/// actually changed.
const UI_TICK: Duration = Duration::from_millis(33);
const MEM_TICK: Duration = Duration::from_secs(1);
const ROW_HEIGHT: f32 = 20.;

struct LogPane {
    agent: SharedString,
    shown: u64,
    scroll: UniformListScrollHandle,
}

struct Workspace {
    store: Store,
    tickets: Vec<Ticket>,
    agents: Vec<MockAgent>,
    panes: Vec<LogPane>,
    footprint_mb: f64,
    peak_mb: f64,
    lines_per_sec: u32,
    _ticker: Task<()>,
}

impl Workspace {
    fn new(db: PathBuf, agent_count: usize, lines_per_sec: u32, cx: &mut Context<Self>) -> Self {
        let mut store = Store::open(&db).expect("open store");
        store.seed_tickets_if_empty().expect("seed tickets");
        let tickets = store.tickets().expect("load tickets");

        const NAMES: [&str; 6] = ["frontend", "backend", "qa", "devops", "designer", "po"];
        let agents: Vec<MockAgent> = (0..agent_count)
            .map(|i| {
                let name = NAMES[i % NAMES.len()];
                MockAgent::spawn(db.clone(), name, lines_per_sec, Duration::from_millis(50))
            })
            .collect();
        let panes = agents
            .iter()
            .map(|a| LogPane {
                agent: a.name.clone().into(),
                shown: 0,
                scroll: UniformListScrollHandle::new(),
            })
            .collect();

        let tick = Duration::from_millis(env_or("EQUIPE_TICK_MS", UI_TICK.as_millis() as u64));
        let ticker = cx.spawn(async move |this, cx| {
            let mut last_mem = Instant::now() - MEM_TICK;
            loop {
                cx.background_executor().timer(tick).await;
                let alive = this.update(cx, |this, cx| {
                    let mut changed = false;
                    for (pane, agent) in this.panes.iter_mut().zip(&this.agents) {
                        let committed = agent.committed.load(Ordering::Acquire);
                        if committed != pane.shown {
                            // Follow the tail only if the user hasn't scrolled away.
                            let follow = pane.scroll.is_scrolled_to_end().unwrap_or(true);
                            pane.shown = committed;
                            if follow {
                                pane.scroll.scroll_to_bottom();
                            }
                            changed = true;
                        }
                    }
                    // The memory readout rides along with frames that happen anyway.
                    // Repainting just to show it would keep the window's GPU working
                    // set (~100 MB on macOS) resident; it is released ~1s after the
                    // last frame, so an idle equipe must draw nothing at all.
                    if changed && last_mem.elapsed() >= MEM_TICK {
                        last_mem = Instant::now();
                        if let Some(b) = mem::phys_footprint_bytes() {
                            this.footprint_mb = b as f64 / 1_048_576.;
                            this.peak_mb = this.peak_mb.max(this.footprint_mb);
                        }
                    }
                    if changed {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        Self {
            store,
            tickets,
            agents,
            panes,
            footprint_mb: 0.,
            peak_mb: 0.,
            lines_per_sec,
            _ticker: ticker,
        }
    }

    fn render_board(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let columns = [
            TicketStatus::Todo,
            TicketStatus::InProgress,
            TicketStatus::Review,
            TicketStatus::Done,
        ];
        v_flex()
            .w(px(280.))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .p_3()
            .gap_3()
            .child(div().text_sm().font_semibold().child("Board"))
            .children(columns.into_iter().map(|status| {
                let tickets = self.tickets.iter().filter(|t| t.status == status);
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(status.label().to_uppercase()),
                    )
                    .children(tickets.map(|t| {
                        v_flex()
                            .px_2()
                            .py_1p5()
                            .rounded_md()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.background)
                            .child(div().text_sm().child(t.title.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(format!("#{} · {}", t.id, t.role)),
                            )
                    }))
            }))
    }

    fn render_pane(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pane = &self.panes[ix];
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .border_r_1()
            .border_color(border)
            .child(
                h_flex()
                    .px_3()
                    .py_1p5()
                    .justify_between()
                    .border_b_1()
                    .border_color(border)
                    .child(div().text_sm().font_semibold().child(pane.agent.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("{} lines", pane.shown)),
                    ),
            )
            .child(
                uniform_list(
                    ("log", ix),
                    pane.shown as usize,
                    cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                        // Only the visible rows are ever read from SQLite.
                        let agent = this.panes[ix].agent.clone();
                        let rows = this
                            .store
                            .lines(&agent, range.start as u64..range.end as u64)
                            .unwrap_or_default();
                        let theme = cx.theme();
                        rows.into_iter()
                            .map(|line| {
                                let color = match line.kind {
                                    LogKind::Text => theme.foreground,
                                    LogKind::Tool => theme.info,
                                    LogKind::DiffAdd => theme.success,
                                    LogKind::DiffDel => theme.danger,
                                };
                                div()
                                    .id(line.seq as usize)
                                    .h(px(ROW_HEIGHT))
                                    .px_3()
                                    .text_xs()
                                    .font_family(theme.mono_font_family.clone())
                                    .text_color(color)
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(line.text)
                            })
                            .collect()
                    }),
                )
                .track_scroll(&pane.scroll)
                .flex_1(),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let total: u64 = self.panes.iter().map(|p| p.shown).sum();
        let panes: Vec<_> = (0..self.panes.len()).map(|ix| self.render_pane(ix, cx)).collect();
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_board(cx))
                    .child(h_flex().flex_1().min_w_0().h_full().children(panes)),
            )
            .child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_4()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "memory {:.1} MB (peak {:.1} MB)",
                        self.footprint_mb, self.peak_mb
                    ))
                    .child(format!(
                        "{} agents × {} lines/s",
                        self.agents.len(),
                        self.lines_per_sec
                    ))
                    .child(format!("{total} lines stored")),
            )
    }
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn main() {
    let db = PathBuf::from(
        std::env::var("EQUIPE_DB").unwrap_or_else(|_| ".equipe/desktop-spike.sqlite".into()),
    );
    if let Some(dir) = db.parent() {
        std::fs::create_dir_all(dir).expect("create db dir");
    }
    // Start every spike run from an empty log so measurements are comparable.
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    let agents: usize = env_or("EQUIPE_AGENTS", 3);
    let rate: u32 = env_or("EQUIPE_RATE", 200);

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let (w, h): (f32, f32) = (env_or("EQUIPE_W", 1200.), env_or("EQUIPE_H", 760.));
        let bounds = Bounds::centered(None, size(px(w), px(h)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|cx| Workspace::new(db.clone(), agents, rate, cx))
        })
        .expect("failed to open window");
    });
}
