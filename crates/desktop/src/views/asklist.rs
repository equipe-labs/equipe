//! A reply that ends asking the person several things in a list of its own
//! words (`crate::asklist`), answered where it was asked: a quiet "이 질문들에
//! 답하기" under the list turns it, in place, into a form — each item with its
//! words, a box that grows, and a tap for the obvious answers (네 · 아니요,
//! 같은 걸로) — and a box for anything to add. Tab goes from box to box, ⌘↵
//! sends every answer to the member in one message ("1) 전화번호 → …"), as
//! the composer would, and the form gives way to that message; Esc puts the
//! list back. A "1." typed into the composer after such a reply offers the
//! form too, above the box.

use std::rc::Rc;

use equipe_core::protocol::SessionId;
use equipe_core::t;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::decide::Took;
use crate::asklist::{self as rules, Asked, Chip};
use crate::convo::Block;
use crate::icons::{Icon, icon};
use crate::motion;
use crate::render;
use crate::state::Presence;
use crate::theme::{MONO, Palette, R_LG, R_SM, SMALL};
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

/// The form, while it is open, and the reply answered from here.
#[derive(Default)]
pub struct AskListUi {
    pub form: Option<Form>,
    /// The conversation and reply answered from here: its button doesn't come
    /// back while the message is on its way.
    sent: Option<(SessionId, u64)>,
}

/// The form open on a reply's list.
pub struct Form {
    session: SessionId,
    /// The reply's words, hashed: the form is that reply's only.
    hash: u64,
    asked: Asked,
    fields: Vec<Entity<TextareaState>>,
    note: Entity<TextareaState>,
    _subscriptions: Vec<Subscription>,
}

/// What the conversation draws for the reply that asks: its row, the list,
/// and the form's boxes when it is open.
pub struct Shown {
    pub ix: usize,
    pub asked: Asked,
    pub open: Option<Open>,
}

/// The form, as it is drawn.
pub struct Open {
    fields: Vec<Entity<TextareaState>>,
    note: Entity<TextareaState>,
    can_send: bool,
    to: String,
}

impl Shell {
    /// The reply in the conversation on screen that asks the person things in
    /// a list — the latest, its turn over, nothing said since: its row, its
    /// words hashed, and the list.
    pub fn asking(&self) -> Option<(usize, u64, Asked)> {
        let session = self.model.viewed()?;
        if self.model.viewed_presence() == Some(Presence::Working) {
            return None;
        }
        let blocks = &self.chat.blocks;
        let ix = blocks.iter().rposition(|b| matches!(b, Block::Reply { .. }))?;
        let Block::Reply { text, live: false, .. } = &blocks[ix] else { return None };
        if blocks[ix + 1..].iter().any(|b| matches!(b, Block::User(_) | Block::Pending { .. } | Block::Thinking(_))) {
            return None;
        }
        let hash = render::hash_of(text);
        if self.asklist.sent == Some((session, hash)) {
            return None;
        }
        Some((ix, hash, rules::find(text.as_str())?))
    }

    /// The form open on the reply that asks, if it is that one's.
    fn asklist_form(&self) -> Option<&Form> {
        let form = self.asklist.form.as_ref()?;
        let (_, hash, _) = self.asking()?;
        (form.hash == hash && Some(form.session) == self.model.viewed()).then_some(form)
    }

    /// "이 질문들에 답하기": the list becomes the form, the first box takes the
    /// keys — with what was typed after a "1." in the composer, if anything.
    pub fn asklist_open(&mut self, first: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((ix, hash, asked)), Some(session)) = (self.asking(), self.model.viewed()) else { return };
        let mut subscriptions = Vec::new();
        let mut field = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            let state = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 4).submit_on_enter(false).placeholder(placeholder));
            subscriptions.push(cx.subscribe_in(&state, window, |this: &mut Shell, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { secondary: true, .. } => this.asklist_send(window, cx),
                InputEvent::Change => {
                    // A box may have grown a line, and Send may wake.
                    if let Some((ix, _, _)) = this.asking() {
                        this.chat.remeasure_at(ix, cx);
                    }
                    cx.notify();
                }
                _ => {}
            }));
            state
        };
        let fields: Vec<Entity<TextareaState>> = asked.items.iter().map(|_| field(t!("asklist.answer_placeholder"), window, cx)).collect();
        let note = field(t!("asklist.note_placeholder"), window, cx);
        if let Some(first_box) = fields.first() {
            first_box.update(cx, |s, cx| {
                if let Some(words) = first.filter(|w| !w.is_empty()) {
                    s.set_value(words, window, cx);
                }
                s.focus(window, cx);
            });
        }
        self.asklist.form = Some(Form { session, hash, asked, fields, note, _subscriptions: subscriptions });
        self.chat.remeasure_at(ix, cx);
        cx.notify();
    }

    /// Esc, or Cancel: the list as it was, the keys back in the composer.
    pub fn asklist_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.asklist.form.take().is_none() {
            return;
        }
        if let Some((ix, _, _)) = self.asking() {
            self.chat.remeasure_at(ix, cx);
        }
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// ⌘↵, or Send: every answer to the member in one message, sent as the
    /// composer sends one — to the project in view — and the form gives way
    /// to it.
    pub fn asklist_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.asklist_form() else { return };
        let answers: Vec<String> = form.fields.iter().map(|f| f.read(cx).value().to_string()).collect();
        let note = form.note.read(cx).value().to_string();
        if !rules::can_send(&answers, &note) || !self.can_act() {
            return;
        }
        let text = rules::message(&form.asked.items, &answers, &note, t!("asklist.skipped"));
        let answered = (form.session, form.hash);
        let plan = crate::compose::plan(&self.model, &text, false);
        if plan.commands.is_empty() {
            self.composer.flash = plan.flash;
            cx.notify();
            return;
        }
        for command in plan.commands {
            self.send_command(command);
        }
        self.asklist.sent = Some(answered);
        self.asklist.form = None;
        self.composer.focus(window, cx);
        self.chat.follow_tail(cx);
        cx.notify();
    }

    /// A quick answer tapped: into item `n`'s box, and on to the next box.
    pub fn asklist_say(&mut self, n: usize, words: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.asklist_form() else { return };
        let (Some(field), next) = (form.fields.get(n).cloned(), form.fields.get(n + 1).cloned().unwrap_or_else(|| form.note.clone())) else { return };
        field.update(cx, |s, cx| s.set_value(words.to_owned(), window, cx));
        next.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    /// A keystroke, before anyone else sees it: in the form, Tab and ⇧Tab go
    /// from box to box and Esc puts the list back (the rest is the boxes'
    /// typing); in the composer, ⏎ on a lone line begun "1." opens the form
    /// with what follows in the first box.
    pub fn on_asklist_key(&mut self, key: &str, m: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> Took {
        if self.screen != Screen::Chat || m.control || m.alt || m.function {
            return Took::Pass;
        }
        let boxes: Vec<Entity<TextareaState>> = self.asklist.form.iter().flat_map(|form| form.fields.iter().chain(std::iter::once(&form.note))).cloned().collect();
        if let Some(at) = boxes.iter().position(|f| f.read(cx).focus_handle(cx).is_focused(window)) {
            return match key {
                "tab" if !m.platform => {
                    let next = if m.shift { (at + boxes.len() - 1) % boxes.len() } else { (at + 1) % boxes.len() };
                    boxes[next].update(cx, |s, cx| s.focus(window, cx));
                    Took::Taken
                }
                "escape" => {
                    self.asklist_cancel(window, cx);
                    Took::Taken
                }
                _ => Took::Typing,
            };
        }
        if key != "enter" || m.shift || m.platform || !self.composer.is_focused(window, cx) {
            return Took::Pass;
        }
        // The Enter that ends a Korean syllable is only that.
        let state = self.composer.state.clone();
        if state.update(cx, |s, cx| gpui_kit::EntityInputHandler::marked_text_range(s, window, cx).is_some()) {
            return Took::Pass;
        }
        let typed = self.composer.text(cx);
        let Some(first) = rules::started(&typed).filter(|_| !typed.contains('\n')).map(str::to_owned) else { return Took::Pass };
        if self.asking().is_none() {
            return Took::Pass;
        }
        self.composer.set_text("", window, cx);
        self.asklist_open(Some(first), window, cx);
        Took::Taken
    }

    /// The hint's click: the form, with what was typed after "1.".
    fn asklist_from_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.composer.text(cx);
        let first = rules::started(&typed).map(str::to_owned);
        self.composer.set_text("", window, cx);
        self.asklist_open(first, window, cx);
    }

    /// Dev tool (`snapshot.rs`): words into item `n`'s box (`n` past the
    /// items: the box for anything to add).
    #[cfg(feature = "snapshot")]
    pub fn asklist_type(&mut self, n: usize, words: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.asklist_form() else { return };
        let Some(field) = form.fields.get(n).cloned().or_else(|| (n == form.fields.len()).then(|| form.note.clone())) else { return };
        field.update(cx, |s, cx| {
            s.set_value(words.to_owned(), window, cx);
            s.focus(window, cx);
        });
        if let Some((ix, _, _)) = self.asking() {
            self.chat.remeasure_at(ix, cx);
        }
        cx.notify();
    }
}

/// What the conversation draws for the reply that asks, gathered from the
/// shell before the rows are drawn.
pub fn shown(shell: &Shell, cx: &App) -> Option<Rc<Shown>> {
    let (ix, _, asked) = shell.asking()?;
    let open = shell.asklist_form().map(|form| {
        let answers: Vec<String> = form.fields.iter().map(|f| f.read(cx).value().to_string()).collect();
        let note = form.note.read(cx).value().to_string();
        let to = shell.model.selected_member().map(|m| m.name.clone()).unwrap_or_default();
        Open { fields: form.fields.clone(), note: form.note.clone(), can_send: rules::can_send(&answers, &note) && shell.can_act(), to }
    });
    Some(Rc::new(Shown { ix, asked, open }))
}

/// Under the list: a quiet way into the form.
pub fn button(p: Palette, key: u64, handle: &WeakEntity<Shell>) -> AnyElement {
    let handle = handle.clone();
    let pill = h_flex()
        .id(("asklist-open", key))
        .flex_none()
        .h(px(28.))
        .px_2p5()
        .gap(px(6.))
        .items_center()
        .rounded_full()
        .border_1()
        .border_color(p.hair)
        .text_size(px(12.5))
        .text_color(p.text2)
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash).text_color(p.text).border_color(p.muted))
        .child(icon(Icon::Pencil, 12., p.accent))
        .child(t!("asklist.answer_these"))
        .on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.asklist_open(None, window, cx)).ok();
        });
    // Its own width, not the column's.
    h_flex().mt(px(6.)).mb(px(10.)).child(pill).into_any_element()
}

/// The list as a form: each item, its box and its quick answers; a box for
/// anything to add; the keys, Cancel and Send.
pub fn form(p: Palette, tones: render::Tones, shown: &Shown, open: &Open, key: u64, handle: &WeakEntity<Shell>, cx: &mut App) -> AnyElement {
    let items: Vec<AnyElement> = shown.asked.items.iter().enumerate().map(|(n, item)| row(p, tones, n, &item.text, &item.chips, open.fields.get(n), key, handle)).collect();
    let note = v_flex()
        .gap_1p5()
        .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child(t!("asklist.note")))
        .child(answer_box(p, &open.note));
    let cancel = {
        let handle = handle.clone();
        ui::button(p, ("asklist-cancel", key), Kind::Ghost, t!("asklist.cancel")).key("Esc").on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.asklist_cancel(window, cx)).ok();
        })
    };
    let send = {
        let handle = handle.clone();
        ui::button(p, ("asklist-send", key), Kind::Primary, t!("asklist.send")).key("⌘↵").disabled(!open.can_send).on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.asklist_send(window, cx)).ok();
        })
    };
    let foot = h_flex()
        .w_full()
        .pt_3()
        .gap_2()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .text_size(px(12.))
                .line_height(px(17.))
                .text_color(p.muted)
                .when(!open.to.is_empty(), |c| c.child(t!("asklist.to", name = open.to.clone())))
                .child(div().opacity(0.8).overflow_hidden().whitespace_nowrap().text_ellipsis().child(t!("asklist.keys"))),
        )
        .child(cancel)
        .child(send);
    let _ = cx;
    let card = v_flex()
        .id(("asklist-form", key))
        .w_full()
        .mt_1()
        .mb_2()
        .p(px(14.))
        .gap_3()
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(Rgba { a: (p.accent.a * 0.45).max(0.3), ..p.accent })
        .shadow_sm()
        .child(v_flex().gap(px(14.)).children(items))
        .child(note)
        .child(foot);
    motion::rise(card, ("asklist-in", key)).into_any_element()
}

/// One item: its number, its words, its box and its quick answers.
#[allow(clippy::too_many_arguments)]
fn row(p: Palette, tones: render::Tones, n: usize, text: &str, chips: &[Chip], field: Option<&Entity<TextareaState>>, key: u64, handle: &WeakEntity<Shell>) -> AnyElement {
    let number = div()
        .flex_none()
        .size(px(20.))
        .mt(px(1.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(p.hair)
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.text2)
        .child((n + 1).to_string());
    let words = super::chat::markdown(p, ("asklist-item", render::hash_of((key, n))), text, crate::theme::UI, crate::theme::UI_PROSE_LINE, p.text, handle);
    let taps: Vec<AnyElement> = chips
        .iter()
        .map(|chip| {
            let (label, glyph, color) = match chip {
                Chip::Yes => (t!("asklist.yes"), Icon::Check, tones.added),
                Chip::No => (t!("asklist.no"), Icon::X, p.muted),
                Chip::Same => (t!("asklist.same"), Icon::Retry, p.accent),
            };
            let handle = handle.clone();
            div()
                .id(("asklist-chip", render::hash_of((key, n, label))))
                .flex_none()
                .h(px(28.))
                .px_2p5()
                .flex()
                .items_center()
                .gap_1()
                .rounded_full()
                .border_1()
                .border_color(p.hair)
                .text_size(px(12.5))
                .text_color(p.text2)
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash).text_color(p.text))
                .child(icon(glyph, 12., color))
                .child(label)
                .on_click(move |_, window, cx| {
                    handle.update(cx, |shell, cx| shell.asklist_say(n, label, window, cx)).ok();
                })
                .into_any_element()
        })
        .collect();
    let answer = field.map(|field| h_flex().w_full().gap_2().items_start().child(answer_box(p, field)).children(taps));
    h_flex()
        .w_full()
        .gap_2p5()
        .items_start()
        .child(number)
        .child(v_flex().flex_1().min_w_0().gap_2().child(words).children(answer))
        .into_any_element()
}

/// A box that grows with what is typed in it.
fn answer_box(p: Palette, field: &Entity<TextareaState>) -> impl IntoElement {
    div().flex_1().min_w_0().px_3().py(px(2.)).rounded(px(R_SM + 2.)).bg(p.bg).border_1().border_color(p.hair).text_size(px(14.)).child(Textarea::new(field).appearance(false).bordered(false))
}

/// Over the composer, when the person starts a message "1." after a reply
/// that asks in a list: the form, offered.
pub fn hint(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat || shell.asklist_form().is_some() {
        return None;
    }
    let typed = shell.composer.text(cx);
    rules::started(&typed)?;
    let (_, hash, _) = shell.asking()?;
    let p = shell.palette;
    // ⏎ opens it while the message is one line; after that, a click does.
    let enter = !typed.contains('\n');
    let pill = h_flex()
        .id(("asklist-hint", hash))
        .h(px(30.))
        .pl_3()
        .pr(px(if enter { 6. } else { 12. }))
        .gap(px(6.))
        .items_center()
        .rounded_full()
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .shadow(motion::float_shadow(0.5))
        .text_size(px(12.5))
        .text_color(p.text)
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .child(icon(Icon::Pencil, 12., p.accent))
        .child(t!("asklist.hint"))
        .when(enter, |c| {
            c.child(
                div()
                    .flex_none()
                    .min_w(px(20.))
                    .h(px(20.))
                    .px(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .border_1()
                    .border_color(p.hair)
                    .font_family(MONO)
                    .font_features(crate::theme::key_face())
                    .text_size(px(SMALL - 1.))
                    .text_color(p.muted)
                    .child("⏎"),
            )
        })
        .on_click(cx.listener(|shell, _, window, cx| shell.asklist_from_composer(window, cx)));
    Some(h_flex().w_full().pb_2().child(motion::rise(pill, ("asklist-hint-in", hash))).into_any_element())
}
