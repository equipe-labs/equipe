//! A member's ask for a secret (`ask_secret`): a card with a box for each
//! value — hidden where it is a password, a key or a token — where it will be
//! saved, and Save / Not now. What is typed goes to the daemon's machine, into
//! that file, and nowhere else: never into a conversation, never to a member.
//! Over the composer in the conversation — folded to one line until the
//! person opens it, so the work on screen isn't pushed aside — and in the Inbox.

use std::collections::HashMap;
use std::time::{Duration, Instant};

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
    /// Keys just saved from a card, for a few seconds: the card's "Saved (name)".
    pub saved: Vec<(String, Instant)>,
}

/// How long "Saved" stays where the card was.
const SAVED_FOR: Duration = Duration::from_secs(6);

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
                let placeholder = if ask.store.is_some() { t!("secrets.store.placeholder").to_owned() } else { field.label.clone() };
                let input = cx.new(|cx| InputState::new(window, cx).masked(field.secret).placeholder(placeholder));
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
        // A key kept in the squad's store: the card turns into "Saved (name)" for a moment.
        if let Some(store) = &ask.store {
            self.secrets.saved.push((store.name.clone(), Instant::now()));
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SAVED_FOR).await;
                let _ = this.update(cx, |shell, cx| {
                    shell.secrets.saved.retain(|(_, at)| at.elapsed() < SAVED_FOR);
                    cx.notify();
                });
            })
            .detach();
        }
        self.model.revision += 1;
        cx.notify();
    }

    /// "Enter": the folded ask opens over the composer, its first box ready.
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

/// The lock that marks a secret's ask in the Inbox.
pub fn mark(p: Palette) -> AnyElement {
    tip(div().id("secret-mark").size(px(14.)).flex_none().flex().items_center().justify_center().child(icon(Icon::Lock, 13., p.accent)), p, t!("secrets.title"), None).into_any_element()
}

/// The Inbox's row for an ask: what it is for, who asks.
pub fn row_text(shell: &Shell, id: u64) -> Option<(String, String, Rgba)> {
    let ask = shell.model.secret_asks.iter().find(|a| a.id == id)?;
    let (_, color) = who(shell, ask);
    let why = if ask.why.is_empty() { ask.store.as_ref().map(|st| equipe_core::connections::title_of(&st.name, &st.service)).unwrap_or_default() } else { ask.why.clone() };
    Some((why, ask.member.clone(), color))
}

fn who(shell: &Shell, ask: &SecretAsk) -> (String, Rgba) {
    let role = shell.model.members.iter().find(|m| m.session == ask.session).map_or("", |m| m.role.as_str());
    let (_, hue) = look(role, &ask.member);
    (ask.member.clone(), shell.palette.member(hue))
}

/// The card for ask `id`.
pub fn card(shell: &Shell, id: u64, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let ask = shell.model.secret_asks.iter().find(|a| a.id == id)?.clone();
    if let Some(store) = ask.store.clone() {
        return store_card(shell, &ask, &store, cx);
    }
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
            .child(div().text_size(px(SMALL)).line_height(px(18.)).text_color(p.muted).child(if ask.save_to.is_empty() { t!("secrets.vault", machine = shell.endpoint.label()) } else { t!("secrets.where", path = ask.save_to) }))
            .child(h_flex().gap_2().items_center().child(save).child(later))
            .into_any_element(),
    )
}

/// The first ask, over the composer, in the conversation.
pub fn docked(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
    }
    let saved = saved_line(shell);
    let Some(ask) = shell.model.secret_asks.first().cloned() else { return saved };
    if let Some(line) = saved {
        return Some(v_flex().w_full().child(line).children(docked_ask(shell, ask, cx)).into_any_element());
    }
    docked_ask(shell, ask, cx)
}

/// "Saved (name)", where the card was, for a few seconds.
pub fn saved_line(shell: &Shell) -> Option<AnyElement> {
    let p = shell.palette;
    let (name, at) = shell.secrets.saved.last()?;
    if at.elapsed() >= SAVED_FOR {
        return None;
    }
    Some(
        motion::rise(
            h_flex()
                .id("secret-saved")
                .role(Role::Status)
                .w_full()
                .mb_2()
                .px_3()
                .py_2()
                .gap_2()
                .items_center()
                .rounded(px(R_LG))
                .bg(p.raised)
                .border_1()
                .border_color(p.hair)
                .text_size(px(13.))
                .child(icon(Icon::Check, 14., p.accent))
                .child(div().text_color(p.text).child(t!("secrets.store.saved", name = name))),
            SharedString::from(format!("secret-saved-in-{name}")),
        )
        .into_any_element(),
    )
}

fn docked_ask(shell: &Shell, ask: SecretAsk, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let id = ask.id;
    let p = shell.palette;
    // Folded: who asks, for how many values, and why — in one line, with Enter and Later.
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
            .child(div().flex_none().text_color(p.text2).child(match &ask.store {
                Some(store) => t!("secrets.store.folded", what = equipe_core::connections::title_of(&store.name, &store.service)),
                None => t!("secrets.folded", count = ask.fields.len()),
            }))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(p.muted).child(ask.why.clone()))
            .child(h_flex().flex_none().gap_1().child(later).child(open));
        return Some(motion::rise(div().w_full().mb_2().px_3().py_2().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).child(line), ("secret-folded-in", id)).into_any_element());
    }
    let body = card(shell, id, cx)?;
    Some(motion::rise(div().w_full().mb_3().p_3().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).shadow_sm().child(body), ("secret-in", id)).into_any_element())
}

/// The card in the Inbox's peek.
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

/// A catalog service's words (`connections.svc.<id>.<part>`), if it has any.
pub fn service_words(id: &str, part: &str) -> Option<String> {
    let key = format!("connections.svc.{id}.{part}");
    crate::i18n::catalog().own(&key).map(str::to_owned)
}

/// The card for a key asked to be kept in the squad's secret store
/// (`request_secret`, or Connect on the Logins page): what it is for, the
/// button that opens the key's page, what to create there, a masked box.
fn store_card(shell: &Shell, ask: &SecretAsk, store: &equipe_core::protocol::secrets::SecretStore, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let id = ask.id;
    let (name, color) = who(shell, ask);
    let input = shell.secrets.inputs.get(&(id, 0))?.clone();
    let what = equipe_core::connections::title_of(&store.name, &store.service);
    let catalog = equipe_core::connections::Service::of(&store.service);
    let why = if ask.why.is_empty() { catalog.and_then(|s| service_words(s.id, "use")) } else { Some(ask.why.clone()) };
    let hint = store.scopes_hint.clone().or_else(|| catalog.and_then(|s| service_words(s.id, "hint")));
    // Opened here, on the screen the person is looking at: only an address that checks out.
    let page = store.create_url.clone().and_then(|u| equipe_core::connections::clean_url(&u).ok());
    let page_row = match page {
        Some(url) => {
            let host = equipe_core::connections::host_of(&url);
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(ui::button(p, ("secret-open-page", id), Kind::Secondary, t!("secrets.store.open_page")).icon(Icon::External).on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))))
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(p.muted).child(t!("secrets.store.opens", host = host)))
                .into_any_element()
        }
        None => div().w_full().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("secrets.store.no_page")).into_any_element(),
    };
    let field = div().w_full().h(px(CONTROL + 4.)).px_3().flex().items_center().rounded(px(R_MD)).border_1().border_color(p.hair).bg(p.code_bg).text_size(px(13.)).child(Input::new(&input).appearance(false).bordered(false));
    let save = ui::button(p, ("secret-save", id), Kind::Primary, t!("secrets.store.save")).key("⏎").icon(Icon::Lock).on_click(cx.listener(move |this, _, window, cx| this.save_secret_card(id, window, cx)));
    let later = ui::button(p, ("secret-later", id), Kind::Ghost, t!("secrets.not_now")).on_click(cx.listener(move |this, _, _, cx| this.dismiss_secret_card(id, cx)));
    let mut head = h_flex().gap_2().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(div().text_color(p.accent).child(t!("secrets.mark")));
    if !ask.member.is_empty() {
        head = head.child(div().text_color(color).child(name));
    }
    head = head.children(ask.ticket.map(|t| div().text_color(p.muted).child(format!("T-{t}"))));
    Some(
        v_flex()
            .id(("secret-store-card", id))
            .w_full()
            .gap_3()
            .child(head)
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("secrets.store.title", what = what)))
            .children(why.map(|w| div().text_size(px(13.)).line_height(px(19.)).text_color(p.text2).child(t!("secrets.store.for", why = w))))
            .child(page_row)
            .children(hint.map(|h| h_flex().w_full().gap_2().items_start().child(div().pt(px(2.)).flex_none().child(icon(Icon::Info, 13., p.muted))).child(div().flex_1().min_w_0().text_size(px(12.5)).line_height(px(18.)).text_color(p.muted).child(h))))
            .child(field)
            .child(div().text_size(px(SMALL)).line_height(px(18.)).text_color(p.muted).child(t!("secrets.store.where", machine = shell.endpoint.label())))
            .child(h_flex().gap_2().items_center().child(save).child(later))
            .into_any_element(),
    )
}
