//! Memory probe: a near-empty window that optionally re-renders once a second.
//! `PROBE_KIT=1` mounts it through gpui-kit; otherwise plain GPUI.
use std::time::Duration;

use gpui_kit::*;

struct Probe {
    n: u64,
    _tick: Option<Task<()>>,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0xffffff))
            .text_color(rgb(0x000000))
            .child(format!("frame {}", self.n))
    }
}

fn main() {
    let redraw = std::env::var("PROBE_REDRAW").is_ok_and(|v| v == "1");
    let kit = std::env::var("PROBE_KIT").is_ok_and(|v| v == "1");
    gpui_kit::application().run(move |cx| {
        let build = move |cx: &mut App| {
            cx.new(|cx: &mut Context<Probe>| {
                let tick = redraw.then(|| {
                    cx.spawn(async move |this, cx| loop {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        if this.update(cx, |p, cx| { p.n += 1; cx.notify(); }).is_err() { break; }
                    })
                });
                Probe { n: 0, _tick: tick }
            })
        };
        let bounds = Bounds::centered(None, size(px(600.), px(400.)), cx);
        let mk = move || WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
        let opts = mk();
        let windows: usize = std::env::var("PROBE_WINDOWS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        for _ in 1..windows {
            cx.open_window(mk(), move |_, cx| build(cx)).unwrap();
        }
        if kit {
            gpui_kit::init(cx);
            gpui_kit::open_window(opts, cx, move |_, cx| build(cx)).unwrap();
        } else {
            cx.open_window(opts, move |_, cx| build(cx)).unwrap();
        }
    });
}
