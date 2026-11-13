//! equipe's offer to keep a login (`Event::LoginOffered`): the person just
//! signed in to a site by hand in a member's browser, and no login is kept
//! for it. A card asks once — "Save this login?" — with equipe's own boxes
//! for the username and the password (masked), a switch to let equipe sign
//! in there by itself when the site logs a member out, and Save / Not now /
//! Never for this site. What is typed goes to the daemon's machine, into its
//! secret store, and nowhere else: never to a member, never into a
//! conversation. Over the composer in the conversation (folded to one line
//! until opened) and in the Inbox.

use std::collections::{HashMap, HashSet};

use equipe_core::protocol::Command;
use equipe_core::protocol::vault::{LoginDraft, LoginOffer};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{CONTROL, Palette, R_LG, R_MD, SMALL};
use crate::tips::tip;
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

/// The cards' boxes, by offer.
#[derive(Default)]
pub struct LoginOffersUi {
    /// The offers the person opened over the composer; the rest stay one line.
    pub open: HashSet<u64>,
    inputs: HashMap<u64, (Entity<InputState>, Entity<InputState>)>,
    subscriptions: HashMap<u64, Subscription>,
    /// Offers whose "sign in by itself" switch the person turned off (on by
    /// default: saving the login is their leave).
    manual: HashSet<u64>,
    /// Offers for an identity provider's own login whose "Use for Sign in
    /// with …" switch the person turned off (on by default).
    no_idp: HashSet<u64>,
}

impl LoginOffersUi {
    /// Whether one of the boxes has the keyboard: what is typed is the box's.
    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        self.inputs.values().any(|(u, p)| u.read(cx).focus_handle(cx).is_focused(window) || p.read(cx).focus_handle(cx).is_focused(window))
    }
}

impl Shell {
    /// Boxes for each offer; those of offers gone, dropped.
    pub fn login_offers_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let offers = self.model.login_offers.clone();
        let ui = &mut self.login_offers;
        ui.inputs.retain(|id, _| offers.iter().any(|o| o.id == *id));
        ui.subscriptions.retain(|id, _| offers.iter().any(|o| o.id == *id));
        ui.open.retain(|id| offers.iter().any(|o| o.id == *id));
        ui.manual.retain(|id| offers.iter().any(|o| o.id == *id));
        ui.no_idp.retain(|id| offers.iter().any(|o| o.id == *id));
        for offer in offers {
            if self.login_offers.inputs.contains_key(&offer.id) {
                continue;
            }
            let username = cx.new(|cx| InputState::new(window, cx).placeholder(t!("accounts.username_placeholder")));
            let password = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(t!("accounts.password_placeholder")));
            let id = offer.id;
            // ⏎ in the password box saves.
            let subscription = cx.subscribe_in(&password, window, move |shell, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    shell.save_login_offer(id, window, cx);
                }
            });
            self.login_offers.inputs.insert(id, (username, password));
            self.login_offers.subscriptions.insert(id, subscription);
        }
    }

    /// Save: both boxes filled, the login to the daemon — and the boxes
    /// emptied at once. The daemon closes the offer when it hears of it.
    pub fn save_login_offer(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(offer) = self.model.login_offers.iter().find(|o| o.id == id).cloned() else { return };
        let Some((u, p)) = self.login_offers.inputs.get(&id).cloned() else { return };
        let username = u.read(cx).value().trim().to_owned();
        let password = p.read(cx).value().to_string();
        if username.is_empty() || password.is_empty() {
            let empty = if username.is_empty() { u } else { p };
            empty.update(cx, |s, cx| s.focus(window, cx));
            return;
        }
        for input in [&u, &p] {
            input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        }
        let auto = !self.login_offers.manual.contains(&id);
        // "Sign in with …": on by default for an identity provider's own login — saving it is the person's leave.
        let idp = equipe_core::vault::idp_provider(&offer.site).map(|_| !self.login_offers.no_idp.contains(&id));
        let draft = LoginDraft { id: None, site: offer.site.clone(), username, password: Some(password), note: None, auto: Some(auto), idp };
        self.send_command(Command::SaveLogin { draft });
        self.model.login_offers.retain(|o| o.id != id);
        self.model.revision += 1;
        cx.notify();
    }

    /// "Not now", or "never" for that site.
    pub fn dismiss_login_offer(&mut self, id: u64, never: bool, cx: &mut Context<Self>) {
        self.send_command(Command::DismissLoginOffer { id, never });
        self.model.login_offers.retain(|o| o.id != id);
        self.model.revision += 1;
        cx.notify();
    }

    /// "Enter": the folded offer opens over the composer, its first box ready.
    pub fn open_login_offer(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.login_offers.open.insert(id);
        if let Some((u, _)) = self.login_offers.inputs.get(&id) {
            u.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }
}

/// The key that marks an offer in the Inbox.
pub fn mark(p: Palette) -> AnyElement {
    tip(div().id("login-offer-mark").size(px(14.)).flex_none().flex().items_center().justify_center().child(icon(Icon::Lock, 13., p.accent)), p, t!("logins.offer_title"), None).into_any_element()
}

/// The Inbox's row for an offer: which site, whose browser.
pub fn row_text(shell: &Shell, id: u64) -> Option<(String, String)> {
    let offer = shell.model.login_offers.iter().find(|o| o.id == id)?;
    Some((t!("logins.offer_row", site = offer.site), offer.member.clone().unwrap_or_default()))
}

fn boxed(p: Palette, input: &Entity<InputState>) -> Div {
    div().w_full().h(px(CONTROL + 4.)).px_3().flex().items_center().rounded(px(R_MD)).border_1().border_color(p.hair).bg(p.code_bg).text_size(px(13.)).child(Input::new(input).appearance(false).bordered(false))
}

/// The card for offer `id`.
pub fn card(shell: &Shell, id: u64, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let offer: LoginOffer = shell.model.login_offers.iter().find(|o| o.id == id)?.clone();
    let (u, pw) = shell.login_offers.inputs.get(&id)?.clone();
    let auto = !shell.login_offers.manual.contains(&id);
    let field = |label: &'static str, input: &Entity<InputState>| v_flex().flex_1().gap_1().child(div().text_size(px(12.)).text_color(p.muted).child(label)).child(boxed(p, input));
    let handle = cx.entity().downgrade();
    let switch = ui::switch(p, SharedString::from(format!("login-offer-auto-{id}")), auto).name(t!("logins.offer_auto", site = offer.site)).on_flip(move |on, _, cx| {
        let _ = handle.update(cx, |shell, cx| {
            if on {
                shell.login_offers.manual.remove(&id);
            } else {
                shell.login_offers.manual.insert(id);
            }
            cx.notify();
        });
    });
    let provider = equipe_core::vault::idp_provider(&offer.site);
    let idp_on = !shell.login_offers.no_idp.contains(&id);
    let idp_handle = cx.entity().downgrade();
    let idp_switch = provider.map(|provider| {
        ui::switch(p, SharedString::from(format!("login-offer-idp-{id}")), idp_on).name(t!("logins.offer_idp", provider = provider)).on_flip(move |on, _, cx| {
            let _ = idp_handle.update(cx, |shell, cx| {
                if on {
                    shell.login_offers.no_idp.remove(&id);
                } else {
                    shell.login_offers.no_idp.insert(id);
                }
                cx.notify();
            });
        })
    });
    let save = ui::button(p, ("login-offer-save", id), Kind::Primary, t!("accounts.save")).key("⏎").icon(Icon::Lock).on_click(cx.listener(move |this, _, window, cx| this.save_login_offer(id, window, cx)));
    let later = ui::button(p, ("login-offer-later", id), Kind::Ghost, t!("secrets.not_now")).on_click(cx.listener(move |this, _, _, cx| this.dismiss_login_offer(id, false, cx)));
    let never = ui::button(p, ("login-offer-never", id), Kind::Ghost, t!("logins.offer_never")).on_click(cx.listener(move |this, _, _, cx| this.dismiss_login_offer(id, true, cx)));
    let manage = ui::button(p, ("login-offer-manage", id), Kind::Ghost, t!("logins.offer_manage")).on_click(cx.listener(|this, _, _, cx| this.open_logins(cx)));
    let whose = offer.member.as_deref().map(|m| t!("logins.offer_whose", name = m));
    Some(
        v_flex()
            .w_full()
            .gap_3()
            .child(h_flex().gap_2().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(div().text_color(p.accent).child(t!("logins.offer_title"))).children(whose.map(|w| div().text_color(p.muted).child(w))))
            .child(div().text_size(px(15.)).text_color(p.text).child(t!("logins.offer_question", site = offer.site)))
            .child(h_flex().w_full().gap_3().child(field(t!("accounts.username"), &u)).child(field(t!("accounts.password"), &pw)))
            .child(h_flex().gap_2().items_center().child(switch).child(div().text_size(px(SMALL)).text_color(p.text2).child(t!("logins.offer_auto", site = offer.site))))
            .children(provider.zip(idp_switch).map(|(provider, switch)| h_flex().gap_2().items_center().child(switch).child(div().text_size(px(SMALL)).text_color(p.text2).child(t!("logins.offer_idp", provider = provider)))))
            .child(div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("logins.offer_where", machine = shell.endpoint.label())))
            .child(h_flex().gap_2().items_center().child(save).child(later).child(never).child(div().flex_1()).child(manage))
            .into_any_element(),
    )
}

/// The first offer, over the composer, in the conversation.
pub fn docked(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
    }
    let offer = shell.model.login_offers.first()?.clone();
    let id = offer.id;
    let p = shell.palette;
    if !shell.login_offers.open.contains(&id) {
        let open = ui::button(p, ("login-offer-open", id), Kind::Secondary, t!("logins.offer_open")).icon(Icon::Lock).on_click(cx.listener(move |this, _, window, cx| this.open_login_offer(id, window, cx)));
        let later = ui::button(p, ("login-offer-later-folded", id), Kind::Ghost, t!("secrets.not_now")).on_click(cx.listener(move |this, _, _, cx| this.dismiss_login_offer(id, false, cx)));
        let line = h_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .items_center()
            .text_size(px(13.))
            .child(icon(Icon::Lock, 13., p.accent))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(p.text2).child(t!("logins.offer_question", site = offer.site)))
            .child(h_flex().flex_none().gap_1().child(later).child(open));
        return Some(motion::rise(div().w_full().mb_2().px_3().py_2().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).child(line), ("login-offer-folded-in", id)).into_any_element());
    }
    let body = card(shell, id, cx)?;
    Some(motion::rise(div().w_full().mb_3().p_3().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).shadow_sm().child(body), ("login-offer-in", id)).into_any_element())
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
        .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("logins.offer_title")))
        .children(card(shell, id, cx))
        .into_any_element()
}
