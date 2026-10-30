//! The picked thing, at the right of the list (Space) or as a page (⏎): what
//! it is and what the person is asked. A review shows the hand-in, what to
//! check, the files changed, what it unblocks, the last words; a wait, what
//! to do (1–9 ticks lines off); a proposal, why; a request for permission,
//! its card. The answers sit at the foot, the words box (R) with them.

use equipe_core::t;

use equipe_core::protocol::{OffMain, Ticket, TicketId, TicketStatus};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::{Flow, btn, dot, hue_of, kbd, live_activity, member_color};
use super::icons;
use crate::board::{self as rules, MergeWay, Tab};
use super::yourturn::Place;
use crate::convo::Block;
use crate::state::{Need, local_hm, short_span};
use crate::theme::{MONO, Palette, SMALL, look};
use crate::Shell;

/// The peek: a scrolling body and the answers at its foot.
pub fn panel(shell: &Shell, need: Need, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    match need {
        Need::Permission(id) => v_flex()
            .id("peek")
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .gap_3()
            .child(kicker(p, t!("state.section_permission")))
            .children(super::approvals::card(shell, id, false, cx))
            .into_any_element(),
        Need::Questions(id) => super::decide::peek(shell, id, cx),
        Need::Team(id) => super::team::peek(shell, id, cx),
        Need::Accept(id) => super::accept::peek(shell, id, cx),
        Need::Browser(page) => super::browser::need_peek(shell, page, cx),
        Need::Secret(id) => super::secrets::peek(shell, id, cx),
        Need::Ticket(id) => {
            let Some(t) = shell.model.tickets.iter().find(|t| t.id == id).cloned() else { return div().into_any_element() };
            let width = shell.board.peek_width;
            let pad = pad(width);
            // The properties' box spans the peek, its rows inside its padding and edge.
            let inside = width - 2. * pad - 2. * BOX_PAD - 2.;
            v_flex()
                .size_full()
                .child(
                    v_flex()
                        .id("peek")
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .overflow_y_scroll()
                        .overflow_x_hidden()
                        .track_scroll(&shell.board.peek_scroll)
                        .p(px(pad))
                        .gap_4()
                        .child(head(shell, &t, false, cx))
                        .child(div().w_full().px(px(BOX_PAD)).py(px(6.)).rounded(px(8.)).border_1().border_color(p.hair).child(super::tickets::properties(shell, &t, true, inside, cx)))
                        .children(body(shell, &t, false, Place::Peek, cx)),
                )
                .child(foot(shell, &t, width, cx))
                .into_any_element()
        }
    }
}

/// A ticket beside the conversation (a `T-n` in a row was clicked): what the
/// peek shows of it, to read; answering it is the board's (its ⤢).
pub fn side(shell: &Shell, id: TicketId, cx: &mut Context<Shell>) -> AnyElement {
    let Some(t) = shell.model.tickets.iter().find(|t| t.id == id).cloned() else { return div().into_any_element() };
    v_flex().id(("side-peek", id as u64)).size_full().overflow_y_scroll().p_4().gap_4().child(head(shell, &t, false, cx)).children(body(shell, &t, false, Place::Side, cx)).into_any_element()
}

/// The ticket as a page: its head, the tabs, and the tab's content.
pub fn page(shell: &Shell, t: &Ticket, tab: Tab, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let tab_button = |id: &'static str, label: &'static str, which: Tab, cx: &mut Context<Shell>| {
        let on = tab == which;
        div()
            .id(id)
            .h(px(32.))
            .px_3()
            .flex()
            .items_center()
            .text_size(px(SMALL))
            .cursor_pointer()
            .border_b_2()
            .border_color(if on { p.text } else { transparent_black().into() })
            .when(!on, |d| d.text_color(p.text2).hover(move |s| s.text_color(p.text)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.board.view.page = Some(which);
                cx.notify();
            }))
            .child(crate::ui::steady(label, FontWeight::SEMIBOLD, on, false))
    };
    let content: Vec<AnyElement> = match tab {
        Tab::Overview => std::iter::once(super::tickets::description(shell, t, cx)).chain(body(shell, t, true, Place::Peek, cx)).collect(),
        Tab::Changes => changes(shell, t, true, cx).into_iter().collect(),
        Tab::Activity => vec![super::tickets::timeline(shell, t, cx), activity(shell, t.id, 40)],
    };
    let news = shell.model.histories.get(&t.id).map_or(0, |h| h.iter().filter(|e| e.kind == "comment").count());
    v_flex()
        .flex_1()
        .min_h_0()
        .child(
            h_flex()
                .flex_none()
                .h(px(44.))
                .px_4()
                .gap_2()
                .items_center()
                .border_b_1()
                .border_color(p.hair)
                .child(btn(p, "page-back", "←", Some("Esc"), false, true).on_click(cx.listener(|this, _, _, cx| {
                    this.board.view.page = None;
                    cx.notify();
                })))
                .child(div().text_size(px(12.)).text_color(p.muted).child(super::board::menu_title(shell.board.view.menu, &shell.saved_views())))
                .child(div().text_size(px(12.)).text_color(p.muted).child("›"))
                .child(div().font_family(MONO).text_size(px(12.)).text_color(p.text2).child(format!("T-{}", t.id)))
                .child(div().flex_1()),
        )
        .child(
            h_flex()
                .flex_1()
                .min_h_0()
                .items_start()
                .child(
                    v_flex()
                        .id("page")
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_y_scroll()
                        .child(
                            v_flex()
                                .w_full()
                                .max_w(px(780.))
                                .mx_auto()
                                .px_8()
                                .pt_6()
                                .pb_8()
                                .gap_4()
                                .child(head(shell, t, true, cx))
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .border_b_1()
                                        .border_color(p.hair)
                                        .child(tab_button("tab-overview", t!("peek.tab_overview"), Tab::Overview, cx))
                                        .child(tab_button("tab-changes", t!("peek.tab_changes"), Tab::Changes, cx))
                                        .child(tab_button("tab-activity", t!("peek.tab_activity"), Tab::Activity, cx).when(news > 0, |b| b.child(div().ml_1().text_size(px(11.)).text_color(p.muted).child(news.to_string())))),
                                )
                                .children(content),
                        ),
                )
                .child(
                    v_flex()
                        .id("page-props")
                        .w(px(PAGE_PROPS))
                        .flex_none()
                        .h_full()
                        .overflow_y_scroll()
                        .px_4()
                        .py_5()
                        .border_l_1()
                        .border_color(p.hair)
                        .child(super::tickets::properties(shell, t, false, PAGE_PROPS - 32. - 1., cx)),
                ),
        )
        .child(foot(shell, t, shell.board.list_width + 16., cx))
}


/// The page's column of properties.
const PAGE_PROPS: f32 = 320.;
/// The properties' box: its padding across.
const BOX_PAD: f32 = 12.;
/// A label beside its value takes this much, at most.
const LABEL: f32 = 110.;
/// What a value needs beside its label, at least.
const VALUE_MIN: f32 = 170.;
/// From this width the answers sit in one row; under it, the first on a row
/// of its own and the rest two to a row.
const FOOT_ONE_ROW: f32 = 480.;

/// How a ticket's property list lays out in `width`, the room inside its box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Props {
    /// Each label at the left of its value, `label` wide.
    Columns { label: f32 },
    /// Each label over its value.
    Stacked,
}

pub fn props_layout(width: f32) -> Props {
    let label = (width * 0.36).clamp(84., LABEL);
    if width - label - 8. >= VALUE_MIN { Props::Columns { label } } else { Props::Stacked }
}

/// The peek's padding: a little less when it is narrow.
pub fn pad(width: f32) -> f32 {
    if width < 360. { 12. } else { 16. }
}

/// About how wide `words` are at the properties' 12.5 points: for whether
/// one is cut, and gets a tooltip.
pub fn words_width(words: &str) -> f32 {
    words.chars().map(|c| if c.is_ascii() { 7. } else { 12.5 }).sum()
}

/// How the answers at the foot lay out in `width`: `None`, in a row (it
/// wraps if it must); `Some(n)`, the first alone when it is the main one, the
/// rest `n` to a row, each as wide as the others.
pub fn foot_columns(width: f32) -> Option<usize> {
    (width < FOOT_ONE_ROW).then_some(2)
}

fn kicker(p: Palette, text: impl Into<SharedString>) -> Div {
    div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(text.into())
}

/// Nemotron's review of the hand-in (Nebius Token Factory), as plain text:
/// who read it, the verdict, the summary, risks by file and line, the tests
/// missing. Advice for the person; it decides nothing.
fn code_review(p: Palette, review: &equipe_core::protocol::review::CodeReview) -> AnyElement {
    use equipe_core::protocol::review::ReviewVerdict;
    let (word, tone) = match review.verdict {
        ReviewVerdict::LooksGood => (t!("peek.review_looks_good"), p.accent),
        ReviewVerdict::NeedsAttention => (t!("peek.review_needs_attention"), p.amber),
        ReviewVerdict::Risky => (t!("peek.review_risky"), p.red),
    };
    let line = |text: String| h_flex().gap_2().items_start().text_size(px(13.)).line_height(px(20.)).text_color(p.text2).child(div().flex_none().text_color(p.muted).child("·")).child(div().flex_1().min_w_0().child(text));
    let chip = div().flex_none().px_2().rounded(px(999.)).border_1().border_color(tone).text_color(tone).text_size(px(11.)).line_height(px(18.)).font_weight(FontWeight::SEMIBOLD).child(word);
    let mut block = v_flex()
        .gap_1()
        .child(kicker(p, t!("peek.review_kicker", model = review.model)))
        .child(h_flex().gap_2().items_start().child(chip).child(div().flex_1().min_w_0().text_size(px(13.)).line_height(px(20.)).text_color(p.text).child(review.summary.clone())));
    if !review.risks.is_empty() {
        block = block.child(div().pt_1().text_size(px(11.)).text_color(p.muted).child(t!("peek.review_risks")));
        block = block.children(review.risks.iter().map(|r| {
            let place = match (r.file.as_str(), r.line) {
                ("", _) => String::new(),
                (file, Some(n)) => format!("{file}:{n} — "),
                (file, None) => format!("{file} — "),
            };
            line(format!("{place}{}", r.why))
        }));
    }
    if !review.missing_tests.is_empty() {
        block = block.child(div().pt_1().text_size(px(11.)).text_color(p.muted).child(t!("peek.review_missing_tests")));
        block = block.children(review.missing_tests.iter().map(|m| line(m.clone())));
    }
    if review.truncated {
        block = block.child(div().text_size(px(11.)).text_color(p.muted).child(t!("peek.review_truncated")));
    }
    block.into_any_element()
}

/// Icon, number, status; the title; where, who, the branch, how big, how long.
fn head(shell: &Shell, t: &Ticket, wide: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let _ = cx;
    let who = rules::who(t).map(str::to_owned);
    let mut facts: Vec<AnyElement> = Vec::new();
    if let Some(project) = model.ticket_project_name(t) {
        facts.push(h_flex().gap(px(6.)).items_center().child(dot(hue_of(p, model, t).unwrap_or(p.muted), 6.)).child(project).into_any_element());
    }
    if let Some(w) = &who {
        let member = model.members.iter().find(|m| m.name == *w || m.role == *w);
        let (mark, _) = member.map_or_else(|| look(w, w), |m| look(&m.role, &m.name));
        facts.push(h_flex().gap_1().text_color(member_color(p, model, w)).child(mark).child(w.clone()).into_any_element());
    }
    if let Some(branch) = t.branch.clone().filter(|_| !t.kind.in_home()) {
        facts.push(div().font_family(MONO).text_size(px(12.)).child(format!("{branch} → main")).into_any_element());
    }
    if let Some(stat) = shell.board.stat(t) {
        facts.push(div().child(stat.short()).into_any_element());
    }
    if let Some(since) = model.in_state_for(t) {
        facts.push(div().child(t!("peek.since", span = short_span(since))).into_any_element());
    }
    if t.kind.in_home() {
        facts.push(div().child(t!("board.kind_label", kind = equipe_core::i18n::label::kind(crate::i18n::lang(), t.kind))).into_any_element());
    }
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(icons::status(p, t.status, 14.))
                .child(div().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{}", t.id)))
                .child(div().text_size(px(12.)).text_color(icons::color(p, t.status)).child(crate::state::state_word(t))),
        )
        .child(super::tickets::title(shell, t, if wide { 22. } else { 16. }, cx))
        .child(h_flex().flex_wrap().gap_x_3().gap_y_1().text_size(px(12.)).text_color(p.muted).children(facts))
        .into_any_element()
}

/// The body, by what the ticket asks. A wait's asks are the card they are
/// answered on (beside the conversation, to read: its card is in it).
fn body(shell: &Shell, t: &Ticket, wide: bool, place: Place, cx: &mut Context<Shell>) -> Vec<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    let handle = cx.entity().downgrade();
    let mut out: Vec<AnyElement> = Vec::new();
    let md = |key: &'static str, text: &str| super::chat::markdown(p, (key, t.id as u64), text, crate::theme::UI, crate::theme::UI_PROSE_LINE, p.text2, &handle).into_any_element();
    // What the person left open when they last answered it: a quiet line.
    out.extend(super::yourturn::remaining(p, t));
    // A decision its member waits on about it, answered right here: first —
    // after a hand-in's pictures and note, which come first in a review.
    let review = matches!(t.status, TicketStatus::Review | TicketStatus::Done);
    if !review {
        out.extend(super::decide::for_ticket(shell, t.id, cx));
    }
    match t.status {
        TicketStatus::Waiting => {
            match super::yourturn::Card::of(shell, t.id, cx).filter(|c| !c.asks.is_empty()) {
                Some(card) => {
                    let draw = super::yourturn::Draw { p, tones: crate::render::tones(&p), handle: handle.clone(), key: crate::render::hash_of(("peek-turn", t.id, place == Place::Side)), open: false, place, window: None };
                    out.push(v_flex().gap_2().child(kicker(p, t!("peek.your_part"))).child(super::yourturn::card(&card, draw, cx)).into_any_element());
                }
                None => {
                    if let Some(note) = t.note.as_deref().filter(|n| !n.trim().is_empty()) {
                        out.push(md("peek-note", note));
                    }
                }
            }
            if !t.body.trim().is_empty() && !wide {
                out.push(v_flex().gap_1().child(kicker(p, t!("peek.the_work"))).child(md("peek-body", &t.body)).into_any_element());
            }
        }
        TicketStatus::Review | TicketStatus::Done => {
            // Accepted, not in main: why, first.
            if let Some(why) = rules::off_main_why(t, shell.merge_way_of(&model.ticket_project_name(t).unwrap_or_default())) {
                let tone = if matches!(t.off_main, Some(OffMain::Conflict { .. } | OffMain::Failed { .. })) { p.amber } else { p.accent };
                out.push(
                    v_flex()
                        .gap_1()
                        .pl_3()
                        .py_1()
                        .border_l_2()
                        .border_color(tone)
                        .text_size(px(13.))
                        .line_height(px(20.))
                        .child(div().text_color(p.text).child(t!("state.not_on_main_hint")))
                        .child(div().text_color(p.text2).child(why))
                        .into_any_element(),
                );
            }
            // What the work looks like, over what was said of it.
            let key = crate::render::hash_of(("hand-in-gallery", t.id, wide, place == Place::Side));
            let guess = if wide { 700. } else { shell.board.peek_width - 2. * pad(shell.board.peek_width) };
            out.extend(super::visuals::gallery(shell, key, t.id, &t.images, guess, cx));
            if let Some(note) = t.note.as_deref().filter(|n| !n.trim().is_empty()) {
                out.push(v_flex().gap_1().child(kicker(p, t!("peek.hand_in"))).child(md("peek-note", note)).into_any_element());
            }
            if let Some(review) = model.reviews.get(&t.id) {
                out.push(code_review(p, review));
            }
            out.extend(super::decide::for_ticket(shell, t.id, cx));
            // On the page the description above carries its checklist already.
            let checks = if wide { Vec::new() } else { rules::checklist(&t.body) };
            if !checks.is_empty() {
                out.push(
                    v_flex()
                        .gap_1()
                        .child(kicker(p, t!("peek.to_check")))
                        .children(checks.into_iter().map(|c| h_flex().gap_2().items_start().text_size(px(13.)).line_height(px(20.)).text_color(p.text2).child(div().flex_none().text_color(p.muted).child("□")).child(div().flex_1().min_w_0().child(c))))
                        .into_any_element(),
                );
            } else if t.note.is_none() && !t.body.trim().is_empty() && !wide {
                out.push(md("peek-body", &t.body));
            }
            out.extend(changes(shell, t, wide, cx));
        }
        TicketStatus::Proposed => {
            if !t.body.trim().is_empty() && !wide {
                out.push(v_flex().gap_1().child(kicker(p, t!("peek.why"))).child(md("peek-body", &t.body)).into_any_element());
            }
            let facts = [
                (t!("peek.owner"), t.owner.clone().unwrap_or_else(|| "—".into())),
                (t!("peek.project"), model.ticket_project_name(t).unwrap_or_else(|| t!("board.squad_folder").into())),
                (t!("peek.proposed_by"), t.created_by.clone()),
            ];
            out.push(
                v_flex()
                    .gap_1()
                    .children(facts.into_iter().map(|(k, v)| h_flex().gap_3().text_size(px(13.)).child(div().w(px(64.)).flex_none().text_color(p.muted).child(k)).child(div().text_color(p.text2).child(v))))
                    .into_any_element(),
            );
        }
        _ => {
            if t.status == TicketStatus::InProgress {
                let mut live: Vec<String> = Vec::new();
                if let Some(a) = live_activity(model, t) {
                    live.push(a);
                }
                if let Some(eta) = model.eta(t) {
                    live.push(if eta.at <= model.unix { t!("board.eta_passed", span = short_span(model.unix - eta.at)) } else { t!("peek.eta", at = if eta.guessed { format!("~{}", local_hm(eta.at)) } else { local_hm(eta.at) }) });
                }
                if let Some(q) = model.quiet_for(t) {
                    live.push(format!("{} {}", t!("board.flag_quiet"), short_span(q)));
                }
                if !live.is_empty() {
                    out.push(h_flex().gap_2().items_center().text_size(px(13.)).text_color(p.text2).when(model.is_live(t), |d| d.child(icons::live_dot(p, shell.pulse(cx)))).child(live.join(" · ")).into_any_element());
                }
            }
            if let Some(note) = t.note.as_deref().filter(|n| !n.trim().is_empty() && t.status.is_closed()) {
                out.push(md("peek-note", note));
            }
            if !t.body.trim().is_empty() && !wide {
                out.push(md("peek-body", &t.body));
            }
            if t.branch.is_some() {
                out.extend(changes(shell, t, wide, cx));
            }
        }
    }
    // The page lists what it waits for in its properties.
    if !wide {
        out.extend(links(shell, t, cx));
    }
    if !wide {
        out.push(activity(shell, t.id, 3));
    }
    out
}

/// The files a branch changed, each with its lines; D for the whole diff.
fn changes(shell: &Shell, t: &Ticket, wide: bool, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    t.branch.as_ref()?;
    let shown = if wide { 200 } else { 10 };
    let body: AnyElement = match shell.board.stat(t) {
        None => div().text_size(px(12.)).text_color(p.muted).child("…").into_any_element(),
        Some(stat) if stat.files.is_empty() => div().text_size(px(12.)).text_color(p.muted).child(t!("diff.none")).into_any_element(),
        Some(stat) => v_flex()
            .children(stat.files.iter().take(shown).map(|f| {
                h_flex()
                    .h(px(24.))
                    .gap_3()
                    .items_center()
                    .text_size(px(12.))
                    .child(div().flex_1().min_w_0().truncate().font_family(MONO).text_color(p.text2).child(f.path.clone()))
                    .child(div().flex_none().text_color(p.accent).child(format!("+{}", f.added)))
                    .child(div().w(px(36.)).flex_none().text_align(TextAlign::Right).text_color(p.muted).child(format!("−{}", f.removed)))
            }))
            .when(stat.files.len() > shown, |c| c.child(div().text_size(px(12.)).text_color(p.muted).child(t!("peek.more_files", n = stat.files.len() - shown))))
            .into_any_element(),
    };
    let n = shell.board.stat(t).map_or(0, |s| s.files.len());
    Some(
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(kicker(p, t!("peek.files_changed", n = n)))
                    .child(div().flex_1())
                    .child(btn(p, "peek-diff", t!("peek.whole_diff"), Some("D"), false, true).h(px(24.)).on_click(cx.listener(|this, _, w, cx| this.board_open_diff(w, cx)))),
            )
            .child(body)
            .into_any_element(),
    )
}

/// What it waits for and what waits for it, each a click to it.
fn links(shell: &Shell, t: &Ticket, cx: &mut Context<Shell>) -> Vec<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    let named = |dep: TicketId, cx: &mut Context<Shell>| {
        let found = model.tickets.iter().find(|x| x.id == dep);
        super::ticket_card::hover_link(h_flex().id(("link", dep as usize)), dep, super::ticket_card::Side::Below)
            .h(px(26.))
            .px_1()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(13.))
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.board_reveal(dep);
                this.board_changed(cx);
            }))
            .children(found.map(|d| icons::status(p, d.status, 12.)))
            .child(div().w(px(40.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{dep}")))
            .child(div().flex_1().min_w_0().truncate().text_color(p.text2).child(found.map(|d| d.title.clone()).unwrap_or_default()))
            .into_any_element()
    };
    let mut out = Vec::new();
    let waits: Vec<AnyElement> = t.depends_on.iter().map(|d| named(*d, cx)).collect();
    if !waits.is_empty() {
        out.push(v_flex().child(kicker(p, t!("peek.waits_for"))).children(waits).into_any_element());
    }
    let unblocks: Vec<AnyElement> = model.unblocks(t.id).into_iter().map(|d| named(d, cx)).collect();
    if !unblocks.is_empty() {
        out.push(v_flex().child(kicker(p, t!("peek.unblocks"))).children(unblocks).into_any_element());
    }
    out
}

/// The last words of the work on a ticket.
fn activity(shell: &Shell, id: TicketId, turns: usize) -> AnyElement {
    let p = shell.palette;
    let convo = shell.model.convos.values().find(|c| c.has_work_on(id));
    let blocks: Vec<Block> = convo
        .map(|c| {
            let mut c = c.clone();
            c.showing = Some(id);
            c.blocks().into_iter().filter(|b| matches!(b, Block::User(_) | Block::Reply { .. } | Block::Error { .. })).collect()
        })
        .unwrap_or_default();
    let start = blocks.len().saturating_sub(turns);
    let who = shell.model.member_on(id).map(|m| (m.name.clone(), m.role.clone())).or_else(|| {
        let t = shell.model.tickets.iter().find(|t| t.id == id)?;
        let w = rules::who(t)?.to_owned();
        Some((w.clone(), shell.model.members.iter().find(|m| m.name == w).map_or(w, |m| m.role.clone())))
    });
    let (mark, hue) = who.as_ref().map_or(("·", 0), |(n, r)| look(r, n));
    let short = |s: &str| {
        let s = s.trim();
        if s.chars().count() > 280 { format!("{}…", s.chars().take(280).collect::<String>()) } else { s.to_owned() }
    };
    v_flex()
        .gap_2()
        .child(kicker(p, t!("peek.last_words")))
        .when(blocks.is_empty(), |c| c.child(div().text_size(px(12.)).text_color(p.muted).child(t!("peek.no_words"))))
        .children(blocks[start..].iter().map(|b| {
            let (lead, color, text) = match b {
                Block::User(text) => ("›".to_owned(), p.accent, short(text)),
                Block::Reply { text, .. } => (mark.to_owned(), p.member(hue), short(text)),
                Block::Error { message, .. } => ("✗".to_owned(), p.red, short(message)),
                _ => (String::new(), p.muted, String::new()),
            };
            h_flex().gap_2().items_start().text_size(px(13.)).line_height(px(20.)).child(div().w(px(14.)).flex_none().text_color(color).child(lead)).child(div().flex_1().min_w_0().text_color(p.text2).child(text))
        }))
        .into_any_element()
}

/// An answer at the foot: `btn`'s look, its words ending in "…" (whole in
/// a tooltip) when `fill` makes it share a row with others.
fn action(p: Palette, id: &'static str, label: SharedString, key: Option<&'static str>, primary: bool, enabled: bool, fill: bool) -> Stateful<Div> {
    let b = h_flex()
        .id(id)
        .h(px(28.))
        .px_3()
        .gap_2()
        .items_center()
        .rounded(px(6.))
        .text_size(px(SMALL))
        .whitespace_nowrap()
        .when(fill, |b| b.flex_1().min_w_0().justify_center())
        .when(!fill, |b| b.flex_none())
        .when(enabled, |b| b.cursor_pointer())
        .when(!enabled, |b| b.opacity(0.4))
        .when(primary, |b| b.bg(p.primary).text_color(p.on_primary).font_weight(FontWeight::SEMIBOLD))
        .when(!primary, |b| b.border_1().border_color(p.hair).when(enabled, |b| b.hover(move |s| s.bg(p.wash))))
        .child(div().min_w_0().truncate().child(label.clone()))
        .when_some(key, |b, key| b.child(div().flex_none().text_size(px(11.)).opacity(0.6).child(key)));
    if fill { crate::tips::tip(b, p, label, key) } else { b }
}

/// The foot: the question open, the words box, and the answers — in a row,
/// or, narrow, the main one alone and the rest two to a row, all as wide.
fn foot(shell: &Shell, t: &Ticket, width: f32, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let can = shell.can_act();
    let id = t.id;
    let project = model.ticket_project_name(t).unwrap_or_else(|| t!("board.squad_folder").into());
    let way = shell.merge_way_of(&project);
    let columns = foot_columns(width);
    let fill = columns.is_some();
    let btn = |p: Palette, id: &'static str, label: SharedString, key: Option<&'static str>, primary: bool, enabled: bool| action(p, id, label, key, primary, enabled, fill);
    let mut buttons: Vec<(AnyElement, bool)> = Vec::new();
    let mut add = |b: Stateful<Div>, primary: bool| buttons.push((b.into_any_element(), primary));
    match t.status {
        TicketStatus::Review => {
            let label = match (rules::needs_way(t), way) {
                (true, Some(w)) => t!("peek.accept_way", way = w.label()),
                _ => t!("peek.accept").to_owned(),
            };
            add(btn(p, "p-yes", label.into(), Some("A"), true, can).on_click(cx.listener(|this, _, _, cx| this.board_yes(cx))), true);
            add(btn(p, "p-reply", t!("peek.request_changes").into(), Some("R"), false, can).on_click(cx.listener(|this, _, w, cx| this.board_open_reply(w, cx))), false);
        }
        // A wait is answered on its card, above: A sends it, R goes to its first box.
        TicketStatus::Waiting if !t.asks().is_empty() => {}
        TicketStatus::Waiting => {
            add(btn(p, "p-yes", t!("peek.did_it").into(), Some("A"), true, can).on_click(cx.listener(|this, _, _, cx| this.board_yes(cx))), true);
            add(btn(p, "p-reply", t!("peek.memo_and_go").into(), Some("R"), false, can).on_click(cx.listener(|this, _, w, cx| this.board_open_reply(w, cx))), false);
        }
        TicketStatus::Proposed => {
            add(btn(p, "p-yes", t!("peek.take_proposal").into(), Some("A"), true, can).on_click(cx.listener(|this, _, _, cx| this.board_yes(cx))), true);
            add(btn(p, "p-no", t!("peek.drop").into(), Some("N"), false, can).on_click(cx.listener(|this, _, _, cx| this.board_no(cx))), false);
            add(btn(p, "p-reply", t!("peek.reply_pm").into(), Some("R"), false, can).on_click(cx.listener(|this, _, w, cx| this.board_open_reply(w, cx))), false);
        }
        TicketStatus::Done if t.not_on_main() => {
            let label = match way {
                Some(MergeWay::Push) => t!("peek.merge_main_push"),
                Some(MergeWay::Merge) => t!("peek.merge_main"),
                _ => t!("peek.merge_main_ask"),
            };
            add(btn(p, "p-merge", label.into(), Some("A"), true, can).on_click(cx.listener(move |this, _, _, cx| this.board_merge(id, cx))), true);
            if t.waits_to_merge() {
                add(btn(p, "p-keep", t!("peek.keep_on_branch").into(), None, false, can).on_click(cx.listener(move |this, _, _, cx| this.board_keep(id, cx))), false);
            }
        }
        _ => {}
    }
    if rules::who(t).is_some() {
        add(btn(p, "p-talk", t!("peek.talk").into(), Some("T"), false, true).on_click(cx.listener(|this, _, w, cx| this.board_talk(w, cx))), false);
    }
    if matches!(t.status, TicketStatus::InProgress | TicketStatus::Todo | TicketStatus::Waiting | TicketStatus::Review) {
        add(
            btn(p, "p-cancel", t!("peek.cancel_ticket").into(), Some("N"), false, can).on_click(cx.listener(move |this, _, _, cx| {
                this.board.flow = Some(Flow::Cancel(id));
                cx.notify();
            })),
            false,
        );
    }
    let question = shell.board.flow.filter(|f| match f {
        Flow::Reply(x) | Flow::Cancel(x) | Flow::Method(x) => *x == id,
        Flow::Way { ticket, .. } => *ticket == id,
        _ => false,
    });
    let card = question.map(|flow| question_card(shell, flow, t, &project, cx));
    v_flex()
        .flex_none()
        .px(px(pad(width)))
        .py_3()
        .gap_3()
        .border_t_1()
        .border_color(p.hair)
        .children(card)
        .when(question.is_none_or(|f| matches!(f, Flow::Reply(_))) && !buttons.is_empty(), |c| c.child(answers(buttons, columns)))
        .into_any_element()
}

/// The answers laid out: in a row, or the main one alone and the rest `n`
/// to a row, the last row's room kept so every button is as wide.
fn answers(buttons: Vec<(AnyElement, bool)>, columns: Option<usize>) -> AnyElement {
    let Some(n) = columns else {
        return h_flex().flex_wrap().gap_2().children(buttons.into_iter().map(|(b, _)| b)).into_any_element();
    };
    let mut rest = buttons.into_iter().peekable();
    let mut rows: Vec<AnyElement> = Vec::new();
    if let Some((first, _)) = rest.next_if(|(_, primary)| *primary) {
        rows.push(h_flex().w_full().child(first).into_any_element());
    }
    loop {
        let row: Vec<AnyElement> = rest.by_ref().take(n).map(|(b, _)| b).collect();
        if row.is_empty() {
            break;
        }
        let room = n - row.len();
        rows.push(h_flex().w_full().gap_2().children(row).children((0..room).map(|_| div().flex_1())).into_any_element());
    }
    v_flex().w_full().gap_2().children(rows).into_any_element()
}

fn question_card(shell: &Shell, flow: Flow, t: &Ticket, project: &str, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let can = shell.can_act();
    let id = t.id;
    let card = || v_flex().gap_2().p_3().rounded(px(8.)).bg(p.accent_bg).border_1().border_color(p.hair);
    let ask = |text: String| div().text_size(px(13.)).line_height(px(20.)).text_color(p.text).child(text);
    let ways = |cx: &mut Context<Shell>| {
        v_flex().gap_1().children(MergeWay::ALL.into_iter().enumerate().map(|(i, way)| {
            h_flex()
                .id(("way", i))
                .h(px(28.))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(6.))
                .text_size(px(13.))
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash))
                .on_click(cx.listener(move |this, _, _, cx| this.board_choose_way(way, cx)))
                .child(kbd(p, ["1", "2", "3"][i]))
                .child(way.label())
        }))
    };
    match flow {
        Flow::Way { remember, .. } => card()
            .child(ask(t!("peek.ask_way", project = project)))
            .child(ways(cx))
            .child(
                h_flex()
                    .id("remember")
                    .gap_2()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(p.text2)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.board.flow = Some(Flow::Way { ticket: id, remember: !remember });
                        cx.notify();
                    }))
                    .child(div().size(px(14.)).flex().items_center().justify_center().rounded(px(3.)).border_1().border_color(p.accent).when(remember, |d| d.bg(p.accent).text_color(p.on_accent).text_size(px(10.)).child("✓")))
                    .child(t!("peek.remember"))
                    .child(kbd(p, "Space")),
            )
            .into_any_element(),
        Flow::Method(_) => card().child(ask(t!("peek.ask_method", id = id))).child(ways(cx)).into_any_element(),
        Flow::Cancel(_) => card()
            .child(ask(t!("peek.ask_cancel", id = id)))
            .child(
                h_flex()
                    .gap_2()
                    .child(btn(p, "cancel-yes", t!("peek.cancel_ticket_now"), Some("Y"), true, can).on_click(cx.listener(move |this, _, _, cx| this.board_cancel(id, cx))))
                    .child(btn(p, "cancel-no", t!("board.keep"), Some("N"), false, true).on_click(cx.listener(|this, _, _, cx| {
                        this.board.flow = None;
                        cx.notify();
                    }))),
            )
            .into_any_element(),
        Flow::Reply(_) => {
            let (ask_text, send) = match t.status {
                TicketStatus::Waiting => (t!("peek.ask_memo", id = id), t!("peek.memo_send")),
                TicketStatus::Proposed => (t!("peek.ask_reply_pm").to_owned(), t!("peek.reply_send")),
                TicketStatus::Review | TicketStatus::Done => (t!("peek.ask_changes", who = rules::who(t).unwrap_or("—")), t!("peek.send_back")),
                _ => (t!("peek.ask_member", who = rules::who(t).unwrap_or("—")), t!("peek.reply_send")),
            };
            let empty = shell.board.reply.read(cx).value().trim().is_empty() && t.status != TicketStatus::Waiting;
            v_flex()
                .gap_2()
                .child(div().text_size(px(12.)).text_color(p.muted).child(ask_text))
                .child(div().w_full().px_3().py_2().rounded(px(8.)).bg(p.code_bg).border_1().border_color(p.hair).text_size(px(14.)).child(Textarea::new(&shell.board.reply).appearance(false).bordered(false)))
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1())
                        .child(btn(p, "reply-cancel", t!("peek.cancel"), Some("Esc"), false, true).on_click(cx.listener(|this, _, w, cx| {
                            this.board.flow = None;
                            w.blur(cx);
                            cx.notify();
                        })))
                        .child(btn(p, "reply-send", send, Some("⌘⏎"), true, can && !empty).on_click(cx.listener(|this, _, w, cx| this.board_send_reply(w, cx)))),
                )
                .into_any_element()
        }
        Flow::Bulk { .. } | Flow::SaveView => div().into_any_element(),
    }
}

/// The name box for ⌥V, under the bar.
pub fn save_view_bar(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    if shell.board.flow != Some(Flow::SaveView) {
        return None;
    }
    Some(
        h_flex()
            .px_4()
            .py_2()
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(p.hair)
            .bg(p.raised)
            .child(div().flex_none().text_size(px(SMALL)).text_color(p.text2).child(t!("board.save_view_as")))
            .child(div().w(px(260.)).px_2().py_1().rounded(px(6.)).bg(p.code_bg).border_1().border_color(p.hair).text_size(px(SMALL)).child(Input::new(&shell.board.naming).appearance(false).bordered(false)))
            .child(btn(p, "save-view", t!("board.save"), Some("⏎"), true, true).on_click(cx.listener(|this, _, w, cx| this.board_save_view(w, cx))))
            .child(btn(p, "save-view-cancel", t!("peek.cancel"), Some("Esc"), false, true).on_click(cx.listener(|this, _, _, cx| {
                this.board.flow = None;
                cx.notify();
            })))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings GPUI's own `test` attribute in.
    use super::{BOX_PAD, LABEL, PAGE_PROPS, Props, VALUE_MIN, foot_columns, pad, props_layout, words_width};

    /// The properties' room inside the peek's box, at a peek `width`.
    fn inside(width: f32) -> f32 {
        width - 2. * pad(width) - 2. * BOX_PAD - 2.
    }

    #[test]
    fn properties_sit_beside_their_labels_when_there_is_room() {
        assert_eq!(props_layout(inside(320.)), Props::Stacked);
        assert_eq!(props_layout(inside(380.)), Props::Columns { label: LABEL });
        assert_eq!(props_layout(inside(480.)), Props::Columns { label: LABEL });
        assert_eq!(props_layout(inside(640.)), Props::Columns { label: LABEL });
        // The page's column has them beside too, the label a little narrower.
        match props_layout(PAGE_PROPS - 33.) {
            Props::Columns { label } => assert!((84. ..LABEL).contains(&label), "{label}"),
            Props::Stacked => unreachable!("the page's properties stacked"),
        }
        // A value always keeps its least room beside its label.
        for width in (200..700).step_by(10).map(|w| w as f32) {
            if let Props::Columns { label } = props_layout(width) {
                assert!(width - label - 8. >= VALUE_MIN, "{width}");
            }
        }
    }

    #[test]
    fn the_peek_pads_less_when_narrow() {
        assert_eq!(pad(320.), 12.);
        assert_eq!(pad(380.), 16.);
        assert_eq!(pad(640.), 16.);
    }

    #[test]
    fn the_answers_go_two_to_a_row_when_narrow() {
        assert_eq!(foot_columns(320.), Some(2));
        assert_eq!(foot_columns(420.), Some(2));
        assert_eq!(foot_columns(480.), None);
        assert_eq!(foot_columns(640.), None);
    }

    #[test]
    fn wide_letters_count_wider() {
        assert_eq!(words_width("abc"), 21.);
        assert_eq!(words_width("진행"), 25.);
    }
}
