//! A member's ask for a secret (`ask_secret`): a card with a box for each
//! value — hidden where it is a password, a key or a token — where it will be
//! saved, and Save / Not now. What is typed goes to the daemon's machine, into
//! that file, and nowhere else: never into a conversation, never to a member.
//! Over the composer in the conversation — folded to one line until the
//! person opens it, so the work on screen isn't pushed aside — and in 나에게.

use std::collections::HashMap;

use equipe_core::protocol::Command;
use equipe_core::protocol::secrets::{SecretAsk, SecretValue};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{CONTROL, MONO, Palette, R_LG, R_MD, SMALL, look};
use crate::tips::tip;
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

/// The cards' boxes, by ask and field.
#[derive(Default)]
pub struct SecretsUi {
    /// The asks the person opened over the composer; the rest stay one line.
    pub open: std::collections::HashSet<u64>,
    inputs: HashMap<(u64, usize), Entity<InputState>>,
    subscriptions: HashMap<(u64, usize), Subscription>,
}

impl SecretsUi {
    /// Whether one of the boxes has the keyboard: what is typed is the box's.
    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        self.inputs.values().any(|f| f.read(cx).focus_handle(cx).is_focused(window))
    }
}

impl Shell {
    /// A box for each value of each ask; those of asks gone, dropped.
    pub fn secrets_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let asks = self.model.secret_asks.clone();
        self.secrets.inputs.retain(|(id, _), _| asks.iter().any(|a| a.id == *id));
        self.secrets.open.retain(|id| asks.iter().any(|a| a.id == *id));
        self.secrets.subscriptions.retain(|(id, _), _| asks.iter().any(|a| a.id == *id));
        for ask in asks {
            for (n, field) in ask.fields.iter().enumerate() {
                if self.secrets.inputs.contains_key(&(ask.id, n)) {
                    continue;
                }
                let input = cx.new(|cx| InputState::new(window, cx).masked(field.secret).placeholder(field.label.clone()));
                let id = ask.id;
                // ⏎ in the last box saves, once every box has something.
                let subscription = cx.subscribe_in(&input, window, move |shell, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        shell.save_secret_card(id, window, cx);
                    }
                });
                self.secrets.inputs.insert((ask.id, n), input);
                self.secrets.subscriptions.insert((ask.id, n), subscription);
            }
        }
    }

    /// Save: every box filled, the values to the daemon — and out of the boxes at once.
    pub fn save_secret_card(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ask) = self.model.secret_asks.iter().find(|a| a.id == id).cloned() else { return };
        let inputs: Vec<Entity<InputState>> = (0..ask.fields.len()).filter_map(|n| self.secrets.inputs.get(&(id, n)).cloned()).collect();
        let values: Vec<String> = inputs.iter().map(|i| i.read(cx).value().to_string()).collect();
        if values.len() != ask.fields.len() || values.iter().any(|v| v.trim().is_empty()) {
            // The first empty box gets the keyboard.
            if let Some(empty) = inputs.iter().find(|i| i.read(cx).value().trim().is_empty()) {
                empty.update(cx, |s, cx| s.focus(window, cx));
            }
            return;
        }
        for input in &inputs {
            input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        }
        let squad = self.model.squad.clone();
        self.send_command(Command::SaveSecret { squad, id, values: values.into_iter().map(SecretValue).collect() });
        self.model.secret_asks.retain(|a| a.id != id);
        self.model.revision += 1;
        cx.notify();
    }

    /// "입력": the folded ask opens over the composer, its first box ready.
    pub fn open_secret_card(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.secrets.open.insert(id);
        if let Some(first) = self.secrets.inputs.get(&(id, 0)) {
            first.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    pub fn dismiss_secret_card(&mut self, id: u64, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        self.send_command(Command::DismissSecret { squad, id });
        self.model.secret_asks.retain(|a| a.id != id);
        self.model.revision += 1;
        cx.notify();
    }
}

/// The lock that marks a secret's ask in 나에게.
pub fn mark(p: Palette) -> AnyElement {
    tip(div().id("secret-mark").size(px(14.)).flex_none().flex().items_center().justify_center().child(icon(Icon::Lock, 13., p.accent)), p, t!("secrets.title"), None).into_any_element()
}

/// 나에게's row for an ask: what it is for, who asks.
pub fn row_text(shell: &Shell, id: u64) -> Option<(String, String, Rgba)> {
    let ask = shell.model.secret_asks.iter().find(|a| a.id == id)?;
    let (_, color) = who(shell, ask);
    Some((ask.why.clone(), ask.member.clone(), color))
}

fn who(shell: &Shell, ask: &SecretAsk) -> (String, Rgba) {
    let role = shell.model.members.iter().find(|m| m.session == ask.session).map_or("", |m| m.role.as_str());
    let (mark, hue) = look(role, &ask.member);
    (format!("{mark} {}", ask.member), shell.palette.member(hue))
}

/// The card for ask `id`.
pub fn card(shell: &Shell, id: u64, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let ask = shell.model.secret_asks.iter().find(|a| a.id == id)?.clone();
    let (name, color) = who(shell, &ask);
    let boxes = ask.fields.iter().enumerate().filter_map(|(n, field)| {
        let input = shell.secrets.inputs.get(&(id, n))?.clone();
        Some(
            v_flex()
                .gap_1()
                .child(h_flex().gap_1p5().items_center().text_size(px(12.)).text_color(p.muted).child(field.label.clone()).child(div().font_family(MONO).text_size(px(11.)).opacity(0.7).child(field.key.clone())))
                .child(
                    div()
                        .w_full()
                        .h(px(CONTROL + 4.))
                        .px_3()
                        .flex()
                        .items_center()
                        .rounded(px(R_MD))
                        .border_1()
                        .border_color(p.hair)
                        .bg(p.code_bg)
                        .text_size(px(13.))
                        .child(Input::new(&input).appearance(false).bordered(false)),
                ),
        )
    });
    let save = ui::button(p, ("secret-save", id), Kind::Primary, t!("secrets.save")).key("⏎").icon(Icon::Lock).on_click(cx.listener(move |this, _, window, cx| this.save_secret_card(id, window, cx)));
    let later = ui::button(p, ("secret-later", id), Kind::Ghost, t!("secrets.not_now")).on_click(cx.listener(move |this, _, _, cx| this.dismiss_secret_card(id, cx)));
    Some(
        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(div().text_color(p.accent).child(t!("secrets.mark")))
                    .child(div().text_color(color).child(name))
                    .children(ask.ticket.map(|t| div().text_color(p.muted).child(format!("T-{t}")))),
            )
            .child(div().text_size(px(15.)).text_color(p.text).child(ask.why.clone()))
            .child(v_flex().gap_2().children(boxes))
            .child(div().text_size(px(SMALL)).line_height(px(18.)).text_color(p.muted).child(t!("secrets.where", path = ask.save_to)))
            .child(h_flex().gap_2().items_center().child(save).child(later))
            .into_any_element(),
    )
}

/// The first ask, over the composer, in the conversation.
pub fn docked(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
    }
    let ask = shell.model.secret_asks.first()?.clone();
    let id = ask.id;
    let p = shell.palette;
    // Folded: who asks, for how many values, and why — in one line, with 입력 and 나중에.
    if !shell.secrets.open.contains(&id) {
        let (name, color) = who(shell, &ask);
        let open = ui::button(p, ("secret-open", id), Kind::Secondary, t!("secrets.open")).icon(Icon::Lock).on_click(cx.listener(move |this, _, window, cx| this.open_secret_card(id, window, cx)));
        let later = ui::button(p, ("secret-later-folded", id), Kind::Ghost, t!("secrets.not_now")).on_click(cx.listener(move |this, _, _, cx| this.dismiss_secret_card(id, cx)));
        let line = h_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .items_center()
            .text_size(px(13.))
            .child(icon(Icon::Lock, 13., p.accent))
            .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).text_color(color).child(name))
            .children(ask.ticket.map(|t| div().flex_none().text_color(p.muted).child(format!("T-{t}"))))
            .child(div().flex_none().text_color(p.text2).child(t!("secrets.folded", count = ask.fields.len())))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(p.muted).child(ask.why.clone()))
            .child(h_flex().flex_none().gap_1().child(later).child(open));
        return Some(motion::rise(div().w_full().mb_2().px_3().py_2().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).child(line), ("secret-folded-in", id)).into_any_element());
    }
    let body = card(shell, id, cx)?;
    Some(motion::rise(div().w_full().mb_3().p_3().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).shadow_sm().child(body), ("secret-in", id)).into_any_element())
}

/// The card in 나에게's peek.
pub fn peek(shell: &Shell, id: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    v_flex()
        .id("peek")
        .size_full()
        .overflow_y_scroll()
        .p_4()
        .gap_3()
        .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("secrets.title")))
        .children(card(shell, id, cx))
        .into_any_element()
}
