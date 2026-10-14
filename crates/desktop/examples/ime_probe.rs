//! Korean IME / glyph-sharpness probe (T-53).
//!
//! A single-line input, a multi-line textarea, and the same Korean sentence at
//! 13/14/16pt in the system font, for side-by-side comparison with TextEdit.
//! Every input event (change / Enter / focus / blur) lands in the log at the
//! bottom, so "does Enter during composition send?" can be read off the screen.
//!
//! `cargo run --release -p equipe-desktop --example ime_probe`
//! `PROBE_FONT=<family>` overrides the font (default: Apple SD Gothic Neo).
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::*;

const SENTENCE: &str = "다람쥐 헌 쳇바퀴에 타고파 — 한글 렌더링 선명도 abc ABC 123 가나다라마바사";

struct Probe {
    single: Entity<InputState>,
    multi: Entity<TextareaState>,
    log: Vec<String>,
    font: SharedString,
}

impl Probe {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let single = cx.new(|cx| InputState::new(window, cx).placeholder("한 줄 입력 (Enter = 전송)"));
        let multi = cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 8).placeholder("여러 줄 입력"));
        cx.subscribe_in(&single, window, |this, state, ev: &InputEvent, _, cx| {
            this.on_event("single", ev, state.read(cx).value().to_string(), cx);
        })
        .detach();
        cx.subscribe_in(&multi, window, |this, state, ev: &InputEvent, _, cx| {
            this.on_event("multi", ev, state.read(cx).value().to_string(), cx);
        })
        .detach();
        let font = std::env::var("PROBE_FONT").unwrap_or_else(|_| "Apple SD Gothic Neo".into());
        Probe { single, multi, log: Vec::new(), font: font.into() }
    }

    fn on_event(&mut self, who: &str, ev: &InputEvent, value: String, cx: &mut Context<Self>) {
        let line = match ev {
            InputEvent::Change => format!("{who} change  {value:?} ({} chars)", value.chars().count()),
            InputEvent::PressEnter { secondary, shift } => {
                format!("{who} ENTER(send?) secondary={secondary} shift={shift} value={value:?}")
            }
            InputEvent::Focus => format!("{who} focus"),
            InputEvent::Blur => format!("{who} blur"),
        };
        self.log.push(line);
        if self.log.len() > 14 {
            self.log.remove(0);
        }
        cx.notify();
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let font = self.font.clone();
        let sample = |pt: f32| {
            div()
                .text_size(px(pt))
                .child(format!("{pt}pt  {SENTENCE}"))
        };
        div()
            .size_full()
            .p_4()
            .gap_3()
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .text_color(rgb(0x000000))
            .font_family(font)
            .child(div().text_size(px(12.)).child("T-53 IME probe — 아래 입력창에 두벌식으로 입력하세요"))
            .child(Input::new(&self.single))
            .child(Textarea::new(&self.multi))
            .child(div().flex().flex_col().gap_1().child(sample(13.)).child(sample(14.)).child(sample(16.)))
            .child(
                div()
                    .mt_2()
                    .p_2()
                    .bg(rgb(0xf3f3f3))
                    .text_size(px(11.))
                    .font_family("Menlo")
                    .flex()
                    .flex_col()
                    .children(self.log.iter().map(|l| div().child(l.clone()))),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, _| {}))
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        let bounds = Bounds::centered(None, size(px(760.), px(720.)), cx);
        let opts = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
        gpui_kit::open_window(opts, cx, |window, cx| cx.new(|cx| Probe::new(window, cx))).unwrap();
        cx.activate(true);
    });
}
