//! Deciding in the person's own words, drawn: the card over the composer
//! while something waits on them in words — equipe's read-back ("Merge T-12
//! into main, on this Mac"), each thing it does, what answers it ("Your next
//! message answers this…") and its one button (Undo, Cancel, or leave it on
//! its card) — in whichever conversation of the squad is on screen, since the
//! next message answers it. The composer's badge while "All by words" is on,
//! and the log of where words went wrong. The rules are `crate::words`'.

use equipe_core::protocol::by_words::{Stage, Tier};
use equipe_core::protocol::Command;
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{R_LG, R_MD, SMALL};
use crate::ui::{self, Kind};
use crate::words as rules;
use crate::{Screen, Shell};

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Shell {
    /// The card's button: undo it, cancel it, or leave it on its card.
    pub fn words_stop(&mut self, id: u64, cx: &mut Context<Self>) {
        self.send_command(Command::UndoWords { squad: self.model.squad.clone(), id });
        self.model.words.close(id);
        cx.notify();
    }

    /// Settings' "Where words went wrong": the log, asked of the daemon; its
    /// sheet opens when it comes.
    pub fn open_words_log(&mut self, cx: &mut Context<Self>) {
        self.send_command(Command::WordsLog { squad: self.model.squad.clone() });
        cx.notify();
    }

    /// "All by words" for the squad: the daemon keeps it, and takes it only
    /// from the Mac's own screens.
    pub fn set_all_by_words(&mut self, on: bool, cx: &mut Context<Self>) {
        self.send_command(Command::SetWordsAll { squad: self.model.squad.clone(), on });
        self.model.words.all = on;
        self.settings.words_consent = false;
        cx.notify();
    }

    /// While something counts down, the card's seconds move: a redraw each
    /// half second until nothing waits.
    pub fn tick_words(&mut self, cx: &mut Context<Self>) {
        if self.words_tick.is_some() || !self.model.words.any() {
            return;
        }
        self.words_tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(500)).await;
                let going = this.update(cx, |this, cx| {
                    let going = this.model.words.any();
                    if going {
                        cx.notify();
                    } else {
                        this.words_tick = None;
                    }
                    going
                });
                if !matches!(going, Ok(true)) {
                    break;
                }
            }
        }));
    }
}

/// The tier's colour: quiet for low, amber to confirm, red for what can't be taken back.
fn tier_color(shell: &Shell, tier: Tier) -> Rgba {
    let p = shell.palette;
    match tier {
        Tier::Low => p.accent,
        Tier::High => p.amber,
        Tier::Outward => p.red,
    }
}

/// The card over the composer, in any conversation of the squad on screen.
pub fn docked(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat || shell.model.opened.is_some() {
        return None;
    }
    let live = shell.model.words.front()?.clone();
    let p = shell.palette;
    let now = now_ms();
    let color = tier_color(shell, live.tier);
    let handle = cx.entity().downgrade();
    let id = live.id;

    let head = h_flex()
        .gap_2()
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(icon(Icon::Mic, 13., color))
        .child(div().text_color(color).child(rules::heading(&live)))
        .child(div().ml_auto().px(px(6.)).py(px(1.)).rounded(px(R_MD)).border_1().border_color(color).text_color(color).text_size(px(11.)).child(rules::tier(live.tier)));
    let title = div().text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(live.read_back.clone());
    let numbered = live.items.len() > 1 || live.stage == Stage::Which;
    let items: Vec<AnyElement> = live
        .items
        .iter()
        .enumerate()
        .map(|(n, item)| {
            h_flex()
                .gap_2()
                .items_start()
                .text_size(px(13.))
                .line_height(px(19.))
                .text_color(p.text2)
                .child(div().flex_none().w(px(16.)).text_color(p.muted).child(if numbered { format!("{}.", n + 1) } else { "·".into() }))
                .child(div().flex_1().min_w_0().child(item.clone()))
                .into_any_element()
        })
        .collect();
    let list = (!items.is_empty()).then(|| v_flex().w_full().p_2().gap_0p5().rounded(px(R_MD)).bg(p.wash).border_1().border_color(p.hair).children(items));
    // What members wrote: theirs, set apart — never in equipe's voice.
    let quotes: Vec<AnyElement> = live
        .quotes
        .iter()
        .map(|q| {
            v_flex()
                .w_full()
                .pl_3()
                .py_1()
                .gap_0p5()
                .border_l_2()
                .border_color(p.hair)
                .child(div().text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("words.quote_by", member = q.by.clone())))
                .child(div().text_size(px(13.)).line_height(px(19.)).italic().text_color(p.text2).child(format!("“{}”", q.text)))
                .into_any_element()
        })
        .collect();
    let hint = h_flex()
        .gap_1p5()
        .items_start()
        .text_size(px(12.))
        .line_height(px(17.))
        .text_color(p.text2)
        .child(div().pt(px(2.)).child(icon(Icon::Shield, 12., p.muted)))
        .child(div().flex_1().min_w_0().child(rules::hint(&live, now)));
    let mut said = format!("{} · {}", t!("words.log_said", said = live.said.clone()), rules::via(live.via));
    if live.asked_again > 0 {
        said.push_str(&format!(" · {}", t!("words.asked_again", n = live.asked_again.to_string())));
    }
    let stop = {
        let handle = handle.clone();
        let kind = if live.stage == Stage::Countdown { Kind::Danger } else { Kind::Secondary };
        ui::button(p, ("words-stop", id), kind, rules::button(&live)).disabled(!shell.can_act()).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.words_stop(id, cx)).ok();
        })
    };
    let foot = h_flex()
        .w_full()
        .pt_3()
        .gap_2()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .flex_wrap()
        .child(div().flex_1().min_w(px(crate::ui::WORDS_AT_LEAST)).text_size(px(SMALL)).text_color(p.muted).child(said))
        .child(h_flex().flex_none().ml_auto().gap_2().child(stop));
    let inner = v_flex().w_full().gap_2p5().child(head).child(title).children(list).children(quotes).child(hint).child(foot);
    let card = div()
        .id(("words-card", id))
        .w_full()
        .p(px(14.))
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(color)
        .shadow_sm()
        .child(inner);
    Some(motion::rise(div().w_full().mb_3().child(card), ("words-in", id)).into_any_element())
}

/// The composer's badge while "All by words" is on: its name, and Settings ▸
/// Permissions a click away.
pub fn badge(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if !shell.model.words.all {
        return None;
    }
    let p = shell.palette;
    let b = div()
        .id("words-badge")
        .h(px(22.))
        .px_2()
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .rounded(px(R_MD))
        .bg(p.accent_bg)
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.accent)
        .whitespace_nowrap()
        .cursor_pointer()
        .child(icon(Icon::Mic, 11., p.accent))
        .child(t!("words.badge"))
        .on_click(cx.listener(|this, _, window, cx| {
            this.open_settings(Some(crate::settings::Category::Permissions), window, cx);
        }));
    Some(crate::tips::tip(b, p, t!("words.badge_tip"), None).into_any_element())
}

/// How wide the badge is, for the composer's bar to make room.
pub fn badge_width(shell: &Shell) -> Option<f32> {
    shell.model.words.all.then(|| 8. + 11. + 4. + super::composer::text_width(t!("words.badge")) + 8.)
}
