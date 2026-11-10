//! The designer studio (`crate::studio`): watch a ticket's design work as it
//! happens. Under the title, a line that says honestly how it is going
//! ("Working · 12m", "Waiting for you: pick a direction", "Checks failing:
//! Contrast — …", "Handed in"); then the steps of the process — brief,
//! references, directions, build, checks, hand-in — each lit from what the
//! member did, with how much and when; in the middle, what it is making: the
//! latest picture it drew, at each of the seven widths a check drew it at;
//! under it, what it left on the way, newest first — references and what to
//! learn from them, directions and the pick, critiques, each check and second
//! look, each hand-in and why it was refused — any of them opened big in the
//! middle. At the right, the member's conversation docked (as on the design
//! screen, `views/design_talk.rs`), to say "make this calmer" while looking;
//! the live browser a click away while it browses. Opened from a ticket's
//! card, the member's lane in Orchestration, or its conversation's header;
//! `studio:<ticket>` in the snapshot script. Nothing here changes anything:
//! the folder is read through the daemon (`design_studio`), the pictures
//! through `read_image`.

use equipe_core::protocol::rpc;
use equipe_core::protocol::studio::{Studio, StudioAsk};
use equipe_core::protocol::{TicketId, TicketStatus};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::visuals::{Shots, picture};
use crate::design_talk::DesignContext;
use crate::icons::{Icon, icon};
use crate::state::Member;
use crate::studio::{self as rules, Centre, Flow, Item, ItemKind, Mark, Step, Tone, Watch};
use crate::theme::{META, MONO, Palette, R_LG, R_MD, SMALL, SMALL_LINE};
use crate::tips::TipExt as _;
use crate::visuals::{Size, Viewer};
use crate::{Screen, Shell, ui};

/// The page's side padding.
const PAD: f32 = 24.;
/// A timeline card's width, and its picture's height.
const CARD_W: f32 = 204.;
const CARD_PIC_H: f32 = 112.;
/// The centre's picture at most this tall.
const STAGE_H: f32 = 380.;
/// Under this, the steps sit two to a row.
pub const NARROW: f32 = 720.;

/// The colour a tone is said in.
fn tone_color(p: Palette, tone: Tone) -> Rgba {
    let tones = crate::render::tones(&p);
    match tone {
        Tone::Plain => p.muted,
        Tone::Good => tones.added,
        Tone::Bad => p.danger(),
        Tone::Wait => p.orange,
        Tone::Busy => p.amber,
    }
}

/// A step's mark in words, for its tooltip and a screen reader.
pub fn mark_words(mark: Mark) -> &'static str {
    match mark {
        Mark::Ahead => t!("studio.mark_ahead"),
        Mark::Skipped => t!("studio.mark_skipped"),
        Mark::Done => t!("studio.mark_done"),
        Mark::Now => t!("studio.mark_now"),
        Mark::Attention => t!("studio.mark_attention"),
        Mark::Waiting => t!("studio.mark_waiting"),
    }
}

/// A step as a screen reader says it: `Checks: Now — Floor 11/11`.
pub fn step_label(s: &Step) -> String {
    match &s.count {
        Some(count) => format!("{}: {} — {count}", s.kind.label(), mark_words(s.mark)),
        None => format!("{}: {}", s.kind.label(), mark_words(s.mark)),
    }
}

/// What the centre's caption says: the screen and when it was checked, or
/// the picture's name and when it was made.
pub fn caption(c: &Centre, now: u64) -> String {
    let when = crate::activity::ago(now.saturating_sub(c.at));
    match (&c.screen, &c.direction, c.from) {
        (Some(screen), _, _) => t!("studio.centre_check", screen = screen, when = when),
        (None, Some(name), _) => t!("studio.centre_made", name = name, when = when),
        (None, None, Some(from)) if from != equipe_core::protocol::studio::PictureFrom::Design => t!("studio.centre_from", name = crate::visuals::name_of(&c.shown), from = rules::from_words(from), when = when),
        (None, None, _) => t!("studio.centre_made", name = crate::visuals::name_of(&c.shown), when = when),
    }
}

fn kind_icon(kind: ItemKind) -> Icon {
    match kind {
        ItemKind::Reference => Icon::Compass,
        ItemKind::Direction => Icon::PenTool,
        ItemKind::Critique => Icon::Message,
        ItemKind::Check => Icon::Check,
        ItemKind::Look => Icon::Eye,
        ItemKind::HandIn => Icon::Inbox,
        ItemKind::Report => Icon::FileText,
    }
}

fn kind_words(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Reference => t!("studio.item_reference"),
        ItemKind::Direction => t!("studio.item_direction"),
        ItemKind::Critique => t!("studio.kind_critique"),
        ItemKind::Check => t!("check.title"),
        ItemKind::Look => t!("look.name"),
        ItemKind::HandIn => t!("studio.item_handin"),
        ItemKind::Report => t!("studio.item_report"),
    }
}

/// The screen.
pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let Some(id) = shell.studio.ticket else { return empty(shell, t!("studio.no_ticket").into(), cx) };
    let Some(ticket) = shell.model.tickets.iter().find(|t| t.id == id) else {
        return empty(shell, t!("studio.not_found", id = id), cx);
    };
    let now = crate::state::now_unix();
    let member = shell.studio_member();
    let convo = member.and_then(|m| shell.model.convos.get(&m.session));
    let work = convo.map(|c| rules::work_of(&c.transcript, id)).unwrap_or_default();
    let happenings = rules::happenings(&work);
    let (first, last) = rules::span_of(&work);
    // Only what this ticket's work made: a shared design folder keeps every ticket's.
    let scoped = shell.studio.files().map(|f| rules::scoped(f, first));
    let files = scoped.as_ref();
    let working = member.is_some_and(|m| m.presence == crate::state::Presence::Working && m.ticket == Some(id));
    // Waiting for the person exactly as the sidebar weighs it (`Model::status`).
    let permission = member.and_then(|m| shell.model.approvals.first_for(m.session)).map(|a| format!("{} · {}", a.tool, rules::command_words(&a.action)));
    // The member's questions in the order its conversation offers them: the card names the first, which is the one the conversation opens on.
    let sets = member.map(|m| shell.model.questions_waiting().into_iter().filter(|(s, _)| s.member == m.name).map(|(s, _)| s).collect::<Vec<_>>()).unwrap_or_default();
    let question = sets.first().copied().map(|s| {
        let q = s.questions.first();
        (q.map(|q| if q.header.trim().is_empty() { q.question.clone() } else { q.header.clone() }).unwrap_or_default(), s.ticket)
    });
    let waiting_ticket = member.and_then(|m| shell.model.tickets.iter().find(|t| t.status == TicketStatus::Waiting && t.owner.as_deref() == Some(m.name.as_str()))).map(|t| t.id);
    let waiting = member.filter(|m| m.presence != crate::state::Presence::Closed).and_then(|_| rules::waiting_for(id, ticket, permission, question, waiting_ticket, files, &happenings));
    let browsing = rules::browsing(&happenings, working);
    let reviews = shell.model.tickets.iter().any(|t| t.design_qa == Some(id));
    let flow = rules::flow_of(ticket, reviews, &happenings);
    let w = Watch { ticket, happenings, files, working, waiting, flow };
    let steps = rules::steps(&w);
    let status = rules::status(&w, first, last);
    let items = rules::items(&w.happenings, files);
    let feed = rules::feed(&work, FEED_MOST);
    let found = files.map(rules::findings).unwrap_or_default();
    let notes = if found.is_empty() && flow == Flow::Qa { rules::said_so_far(&work, 4) } else { Vec::new() };
    let narrow = shell.studio_room() < NARROW;

    // The title bar: what is watched, who is at it, and the ways out.
    let header = ui::title_bar(h_flex())
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pr_3()
        .pl(px(shell.header_inset(16., cx)))
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(icon(Icon::Eye, 15., p.text2))
        .child(div().flex_none().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("studio.title")))
        .child(div().flex_none().px_1p5().h(px(20.)).flex().items_center().rounded(px(crate::theme::R_SM)).bg(p.wash).text_size(px(11.)).text_color(p.text2).child(flow.label()))
        .child(div().flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{id}")))
        .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).text_color(p.text2).child(ticket.title.clone()))
        .children(member.map(|m| who(p, m)))
        .when(browsing, |bar| {
            bar.child(
                ui::button(p, "studio-browser", ui::Kind::Secondary, t!("studio.browser"))
                    .icon(Icon::Globe)
                    .tip(t!("studio.browser_tip"), Some("⌘⇧B"))
                    .on_click(cx.listener(|this, _, window, cx| this.studio_browser(window, cx))),
            )
        })
        .children(member.map(|m| {
            let session = m.session;
            ui::icon_button(p, "studio-chat", Icon::Message, t!("studio.conversation"), None).on_click(cx.listener(move |this, _, window, cx| this.select_member(session, window, cx)))
        }));

    // The line that says how it is going: where it is, then what holds it up.
    let tone = rules::status_tone(&status.presence);
    let pending = files.map_or(0, |f| f.pending.len());
    let issue = status.issue.as_ref().map(|i| {
        h_flex()
            .flex_shrink(1.)
            .min_w_0()
            .px_2()
            .py(px(2.))
            .gap_1()
            .items_center()
            .rounded(px(crate::theme::R_SM))
            .bg(p.danger_bg())
            .child(icon(Icon::Alert, 12., p.danger()))
            .child(div().min_w_0().truncate().text_size(px(META)).text_color(p.text).child(rules::issue_words(i)))
    });
    let status_line = h_flex()
        .id("studio-status")
        .flex_none()
        .w_full()
        .min_h(px(38.))
        .px(px(PAD))
        .py_2()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .role(Role::Status)
        .aria_label(rules::status_words(&status, now))
        .child(div().flex_none().size(px(8.)).rounded_full().bg(tone_color(p, tone)))
        .child(div().flex_none().max_w(relative(0.55)).truncate().text_size(px(SMALL)).line_height(px(SMALL_LINE)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(rules::presence_words(&status.presence, now)))
        .children(issue)
        .child(div().flex_1())
        .when(pending > 0, |row| row.child(div().flex_none().text_size(px(META)).text_color(p.muted).child(t!("studio.pending", count = pending))))
        .children(member.and_then(|m| m.engine).map(|e| super::switching::label(p, e)));

    // The decision card for this ticket's direction question: the member's own question set, else the ticket's row.
    let pick_need = sets.iter().find(|s| s.ticket == Some(id)).map(|s| crate::state::Need::Questions(s.id)).unwrap_or(crate::state::Need::Ticket(id));
    // Waiting on the person: what to answer and the way to it, next to the status line, above the log.
    shell.studio.wait_button.set(false);
    let wait_card = match &status.presence {
        rules::Presence::Waiting(wait) => Some(wait_card(p, wait, files, member, pick_need, &shell.studio.wait_button, shell.studio.wait_focus.as_ref(), cx)),
        _ => None,
    };

    if !shell.studio.wait_button.get() {
        shell.studio.wait_tab_used.set(false);
    }
    let mut body = v_flex().id("studio-body").flex_1().min_h_0().w_full().overflow_y_scroll().track_scroll(&shell.studio.scroll).px(px(PAD)).pt_4().pb_10().gap_5();
    // Waiting on the person: what to answer comes before the log of what was done; a pick has its directions' stage right under it.
    let waiting_now = wait_card.is_some();
    let picking = matches!(status.presence, rules::Presence::Waiting(rules::WaitFor::Pick));
    body = body.children(wait_card);
    if picking {
        body = body.child(stage(shell, id, files, &items, now, cx));
    } else if !waiting_now {
        body = body.child(feed_card(p, &feed, now));
    }
    body = body.child(rail(p, &steps, narrow, now, waiting_now));
    if waiting_now {
        body = body.child(feed_card(p, &feed, now));
    }
    if !picking {
        body = body.child(stage(shell, id, files, &items, now, cx));
    }
    if let Some(f) = files {
        body = body.children(recent_strip(shell, id, f, now, cx));
    }
    if !found.is_empty() || !notes.is_empty() {
        body = body.child(findings_part(p, files, &found, &notes, now));
    }
    body = body.child(timeline(shell, &items, now, cx));
    v_flex().id("studio-page").flex_1().min_w_0().min_h_0().bg(p.bg).child(header).child(status_line).child(body).into_any_element()
}

/// How many lines of what it is doing show.
const FEED_MOST: usize = 10;

/// What it is doing now: its latest step big, with how long; the ones
/// before it under, quieter.
fn feed_card(p: Palette, feed: &[rules::FeedLine], now: u64) -> AnyElement {
    let head = div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(t!("studio.feed"));
    let mut card = v_flex().id("studio-feed").w_full().gap_1().p_3().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).role(Role::Group).aria_label(t!("studio.feed")).child(head);
    let Some((latest, earlier)) = feed.split_last() else {
        return card.child(div().text_size(px(SMALL)).text_color(p.muted).child(t!("studio.feed_empty"))).into_any_element();
    };
    let when = |l: &rules::FeedLine| l.at.map(|at| if l.running { t!("studio.now_for", span = crate::state::short_span(now.saturating_sub(at))) } else { crate::activity::ago(now.saturating_sub(at)) });
    let glyph = |l: &rules::FeedLine| match l.kind {
        rules::FeedKind::Said => Icon::Message,
        rules::FeedKind::You => Icon::Reply,
        rules::FeedKind::Note => Icon::Info,
        rules::FeedKind::Step if l.tone == Tone::Bad => Icon::Alert,
        rules::FeedKind::Step => Icon::ChevronRight,
    };
    for (n, l) in earlier.iter().enumerate().rev().take(FEED_MOST - 1).collect::<Vec<_>>().into_iter().rev() {
        card = card.child(
            h_flex()
                .id(("studio-feed-line", n))
                .w_full()
                .gap_2()
                .items_center()
                .child(icon(glyph(l), 12., if l.tone == Tone::Bad { p.danger() } else { p.muted }))
                .child(div().flex_1().min_w_0().truncate().text_size(px(META)).text_color(if l.kind == rules::FeedKind::Said { p.text } else { p.text2 }).child(l.words.clone()))
                .children(when(l).map(|w| div().flex_none().text_size(px(11.)).text_color(p.muted).child(w))),
        );
    }
    let accent = if latest.tone == Tone::Bad { p.danger() } else { p.accent };
    card = card.child(
        h_flex()
            .id("studio-feed-latest")
            .mt_1()
            .w_full()
            .px_2p5()
            .py_2()
            .gap_2()
            .items_center()
            .rounded(px(R_MD))
            .bg(p.accent_bg)
            .border_1()
            .border_color(accent)
            .child(if latest.running { div().flex_none().child(crate::motion::spinner(12., accent)).into_any_element() } else { icon(glyph(latest), 13., accent).into_any_element() })
            .child(div().flex_1().min_w_0().text_size(px(SMALL)).line_height(px(SMALL_LINE)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(latest.words.clone()))
            .children(when(latest).map(|w| div().flex_none().text_size(px(META)).font_weight(FontWeight::MEDIUM).text_color(p.text2).child(w))),
    );
    card.into_any_element()
}

/// The pictures of the work, newest first, small: any of them shown big in the middle.
fn recent_strip(shell: &Shell, id: TicketId, files: &Studio, now: u64, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let recent = rules::recent(files, RECENT_MOST);
    if recent.len() < 2 {
        return None;
    }
    let p = shell.palette;
    let shots = Shots::of(shell, cx.entity().downgrade(), cx);
    let shown = rules::centre_of(files, shell.studio.width, shell.studio.picked.as_deref()).map(|c| c.shown);
    let mut row = h_flex().id("studio-recent").w_full().gap_2().flex_wrap().items_start().role(Role::List).aria_label(t!("studio.recent"));
    for (n, pic) in recent.iter().enumerate() {
        let look = shots.ask(&pic.path, Size::Thumb, cx);
        let on = shown.as_deref() == Some(pic.path.as_str());
        let path = pic.path.clone();
        let label = match pic.width {
            Some(w) => format!("{w}px"),
            None => rules::from_words(pic.from).to_owned(),
        };
        let _ = id;
        row = row.child(
            v_flex()
                .id(("studio-recent-pic", n))
                .flex_none()
                .w(px(RECENT_W))
                .gap(px(3.))
                .cursor_pointer()
                .role(Role::ListItem)
                .aria_label(format!("{} · {}", crate::visuals::name_of(&pic.path), label))
                .child(div().rounded(px(R_MD)).border_2().border_color(if on { p.accent } else { rgba(0x00000000) }).child(picture(&shots, crate::render::hash_of(("studio-recent", &pic.path)), &pic.path, look, RECENT_W - 4., RECENT_H, R_MD, false, 2.)))
                .child(div().min_w_0().truncate().text_size(px(11.)).text_color(if on { p.text } else { p.muted }).child(format!("{label} · {}", crate::activity::ago(now.saturating_sub(pic.modified)))))
                .on_click(cx.listener(move |this, _, _, cx| this.studio_pick(Some(path.clone()), cx))),
        );
    }
    let head = div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(t!("studio.recent"));
    Some(v_flex().w_full().gap_2().child(head).child(row).into_any_element())
}

/// How many recent pictures show, and how big.
const RECENT_MOST: usize = 16;
const RECENT_W: f32 = 112.;
const RECENT_H: f32 = 76.;

/// What it found: its report's lead and items — or, before a report, what it
/// said along the way.
fn findings_part(p: Palette, files: Option<&Studio>, found: &[String], notes: &[(Option<u64>, String)], now: u64) -> AnyElement {
    let report = files.and_then(|f| f.reports.first());
    let head = h_flex()
        .gap_2()
        .items_baseline()
        .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(if found.is_empty() { t!("studio.notes_so_far") } else { t!("studio.findings") }))
        .children(report.map(|r| div().min_w_0().truncate().text_size(px(META)).text_color(p.muted).child(t!("studio.findings_from", name = r.title.clone()))));
    let mut col = v_flex().id("studio-findings").w_full().gap_1p5().p_3().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).child(head);
    if let Some(lead) = report.and_then(|r| rules::report_lead(&r.text)).filter(|_| !found.is_empty()) {
        col = col.child(div().text_size(px(SMALL)).line_height(px(SMALL_LINE)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(lead));
    }
    for (n, f) in found.iter().enumerate() {
        col = col.child(
            h_flex()
                .w_full()
                .gap_2()
                .items_start()
                .child(div().flex_none().w(px(18.)).text_size(px(META)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(format!("{}.", n + 1)))
                .child(div().flex_1().min_w_0().text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.text).child(f.clone())),
        );
    }
    for (at, words) in notes {
        col = col.child(
            h_flex()
                .w_full()
                .gap_2()
                .items_start()
                .child(icon(Icon::Message, 12., p.muted))
                .child(div().flex_1().min_w_0().text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.text).child(words.clone()))
                .children(at.map(|at| div().flex_none().text_size(px(11.)).text_color(p.muted).child(crate::activity::ago(now.saturating_sub(at))))),
        );
    }
    col.into_any_element()
}

/// Nothing to watch: why, quietly.
fn empty(shell: &Shell, text: String, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let header = ui::title_bar(h_flex()).h(px(super::header::HEIGHT)).flex_none().pl(px(shell.header_inset(16., cx))).items_center().border_b_1().border_color(p.hair).child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("studio.title")));
    v_flex().flex_1().min_h_0().bg(p.bg).child(header).child(div().flex_1().flex().items_center().justify_center().px_6().text_size(px(SMALL)).text_color(p.muted).child(text)).into_any_element()
}

/// Who is at it: their face and name.
fn who(p: Palette, m: &Member) -> impl IntoElement + use<> {
    let (_, hue) = crate::theme::look(&m.role, &m.name);
    h_flex().flex_none().gap_1p5().items_center().child(crate::theme::avatar(p, &m.role, &m.name, 18.)).child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(p.member(hue)).child(m.name.clone()))
}

/// The card for what the person is waited on for: what to answer, where, and the way there.
fn wait_card(p: Palette, wait: &rules::WaitFor, files: Option<&Studio>, member: Option<&Member>, pick_need: crate::state::Need, wait_button: &std::cell::Cell<bool>, focus: Option<&gpui_kit::FocusHandle>, cx: &mut Context<Shell>) -> AnyElement {
    use rules::WaitFor;
    let mut names: Vec<String> = Vec::new();
    let words = match wait {
        WaitFor::Pick => {
            names = files.map(|f| f.directions.iter().map(|d| d.name.clone()).collect()).unwrap_or_default();
            if names.is_empty() { t!("studio.wait_pick_plain").to_owned() } else { t!("studio.wait_pick", count = names.len()) }
        }
        WaitFor::Answer { what, ticket: None } => t!("studio.wait_question", what = what),
        WaitFor::Answer { what, ticket: Some(id) } => t!("studio.wait_question_elsewhere", id = id, what = what),
        WaitFor::Permission(what) => t!("studio.wait_ok", what = what),
        WaitFor::Ticket(id) => t!("studio.wait_ticket", id = id),
        WaitFor::Something => t!("studio.wait_this").to_owned(),
    };
    // A pick is made in the inbox's decision card; the rest is answered in the member's conversation.
    let go = if matches!(wait, WaitFor::Pick) {
        Some((t!("studio.wait_go_pick"), None))
    } else {
        member.map(|m| (if matches!(wait, WaitFor::Permission(_)) { t!("studio.wait_go_ok") } else { t!("studio.wait_go") }, Some(m.session)))
    }
    .map(|(label, session)| {
        wait_button.set(true);
        let mut button = ui::button(p, "studio-wait-go", ui::Kind::Primary, label).icon(if session.is_none() { Icon::Inbox } else { Icon::Message });
        if let Some(focus) = focus {
            button = button.focus(focus);
        }
        button.on_click(cx.listener(move |this, _, window, cx| match session {
            Some(session) => this.select_member(session, window, cx),
            None => {
                this.open_side_menu(crate::board::Menu::Inbox, cx);
                this.board_pick(pick_need, cx);
            }
        }))
    });
    h_flex()
        .id("studio-wait")
        .w_full()
        .flex_wrap()
        .gap_x_3()
        .gap_y_2()
        .items_center()
        .p_3()
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(p.orange)
        .child(div().min_w_0().flex_grow(1.).flex_shrink(1.).flex_basis(px(160.)).text_size(px(SMALL)).line_height(px(SMALL_LINE)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(v_flex().gap_1().child(div().child(words)).when(!names.is_empty(), |c| c.child(h_flex().flex_wrap().gap_x_3().text_color(p.text2).font_weight(FontWeight::NORMAL).children(names.into_iter().map(|n| div().child(n)))))))
        .children(go)
        .into_any_element()
}

/// The steps, left to right; in a narrow room, two to a row.
fn rail(p: Palette, steps: &[Step], narrow: bool, now: u64, waiting_card: bool) -> AnyElement {
    let mut row = h_flex().id("studio-rail").w_full().gap_2().flex_wrap().items_stretch().role(Role::List).aria_label(t!("studio.steps"));
    for (n, s) in steps.iter().enumerate() {
        let (fill, edge, ink) = match s.mark {
            Mark::Done => (p.raised, p.hair, p.text),
            Mark::Now => (p.accent_bg, p.accent, p.text),
            Mark::Attention => (p.danger_bg(), p.danger(), p.text),
            // The wait card above carries the orange edge; the step keeps its dot.
            Mark::Waiting => (p.raised, if waiting_card { p.hair } else { p.orange }, p.text),
            Mark::Ahead | Mark::Skipped => (p.bg, p.hair, p.muted),
        };
        let glyph: AnyElement = match s.mark {
            Mark::Done => icon(Icon::Check, 12., tone_color(p, Tone::Good)).into_any_element(),
            Mark::Now => div().size(px(8.)).rounded_full().bg(p.accent).into_any_element(),
            Mark::Attention => icon(Icon::Alert, 12., p.danger()).into_any_element(),
            Mark::Waiting => div().size(px(8.)).rounded_full().bg(p.orange).into_any_element(),
            Mark::Ahead => div().size(px(6.)).rounded_full().border_1().border_color(p.muted).into_any_element(),
            Mark::Skipped => div().w(px(8.)).h(px(1.5)).bg(p.muted).into_any_element(),
        };
        let when = s.at.map(|at| crate::activity::ago(now.saturating_sub(at)));
        let card = v_flex()
            .id(("studio-step", n))
            .flex_1()
            .min_w(px(if narrow { 140. } else { 112. }))
            .when(narrow, |c| c.flex_basis(px(160.)))
            .px_3()
            .py_2()
            .gap(px(2.))
            .rounded(px(R_MD))
            .bg(fill)
            .border_1()
            .border_color(edge)
            .role(Role::ListItem)
            .aria_label(step_label(s))
            .tip(p, s.detail.as_ref().map_or_else(|| s.kind.about().to_owned(), |d| format!("{} — {d}", s.kind.about())), None)
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(div().flex_none().size(px(14.)).flex().items_center().justify_center().child(glyph))
                    .child(div().text_size(px(META)).font_weight(FontWeight::SEMIBOLD).text_color(ink).child(format!("{}. {}", n + 1, s.kind.label()))),
            )
            .child(div().min_w_0().text_size(px(META)).line_height(px(16.)).text_color(if matches!(s.mark, Mark::Ahead | Mark::Skipped) { p.muted } else { p.text2 }).child(s.count.clone().unwrap_or_else(|| mark_words(s.mark).to_owned())))
            .children(when.map(|w| div().text_size(px(11.)).text_color(p.muted).child(w)));
        row = row.child(card);
    }
    row.into_any_element()
}

/// The middle: what it is making — or an item opened big.
fn stage(shell: &Shell, id: TicketId, files: Option<&Studio>, items: &[Item], now: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let shots = Shots::of(shell, cx.entity().downgrade(), cx);
    let frame = || v_flex().id("studio-stage").w_full().gap_3().p_4().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair);
    // An item opened: it, big, and the way back to the live picture.
    if let Some(item) = shell.studio.open.as_ref().and_then(|k| items.iter().find(|i| &i.key == k)) {
        let head = h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(icon(kind_icon(item.kind), 14., tone_color(p, item.tone)))
            .child(div().flex_none().text_size(px(META)).text_color(p.muted).child(kind_words(item.kind)))
            .child(div().flex_1().min_w_0().truncate().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(item.title.clone()))
            .child(ui::button(p, "studio-back", ui::Kind::Ghost, t!("studio.back_live")).icon(Icon::ArrowLeft).on_click(cx.listener(|this, _, _, cx| this.studio_open_item(None, cx))));
        let room = shots.width(rules_key("stage"), 720.) - 32.;
        let n = item.pictures.len().min(3);
        let tile_w = if n == 0 { 0. } else { ((room - 12. * (n as f32 - 1.)) / n as f32).max(120.) };
        let tile_h = if n == 1 { (tile_w * 0.62).min(STAGE_H) } else { 200. };
        let pictures = h_flex().w_full().gap_3().flex_wrap().children(item.pictures.iter().take(3).enumerate().map(|(k, path)| {
            let look = shots.ask(path, if n == 1 { Size::Full } else { Size::Thumb }, cx);
            let viewer = viewer_of(id, &item.pictures, path, &item.title);
            div()
                .id(("studio-open-pic", k))
                .cursor_pointer()
                .child(picture(&shots, crate::render::hash_of(("studio-open", path)), path, look, tile_w, tile_h, R_MD, false, 6.))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(v) = viewer.clone() {
                        this.open_viewer(v, cx);
                    }
                }))
        }));
        let lines = v_flex().w_full().gap_1().children(item.lines.iter().map(|l| div().text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.text2).child(l.clone())));
        let when = item.at.map(|at| crate::activity::ago(now.saturating_sub(at)));
        return shots.measured(frame(), rules_key("stage")).child(head).children(when.map(|w| div().text_size(px(META)).text_color(p.muted).child(w))).when(n > 0, |f| f.child(pictures)).child(lines).into_any_element();
    }
    let Some(files) = files else {
        let words = if shell.studio.asking || shell.studio.asked_at.is_none() { t!("studio.reading") } else { t!("studio.centre_none") };
        return frame().child(div().py_10().w_full().text_center().text_size(px(SMALL)).text_color(p.muted).child(words)).into_any_element();
    };
    let Some(centre) = rules::centre_of(files, shell.studio.width, shell.studio.picked.as_deref()) else {
        let words = if files.dir.is_none() { t!("studio.no_folder") } else { t!("studio.centre_none") };
        return frame().child(div().py_10().w_full().text_center().text_size(px(SMALL)).text_color(p.muted).child(words)).into_any_element();
    };
    // The widths a check drew it at, to switch between.
    let widths = (centre.widths.len() > 1).then(|| {
        let mut seg = h_flex().id("studio-widths").flex_none().h(px(26.)).p(px(2.)).gap(px(2.)).rounded(px(R_MD)).bg(p.wash).role(Role::RadioGroup).aria_label(t!("studio.widths"));
        for (w, _) in &centre.widths {
            let (w, on) = (*w, centre.width == Some(*w));
            seg = seg.child(
                div()
                    .id(("studio-width", w as usize))
                    .h_full()
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(crate::theme::R_SM))
                    .text_size(px(META))
                    .cursor_pointer()
                    .role(Role::RadioButton)
                    .aria_toggled(if on { Toggled::True } else { Toggled::False })
                    .aria_label(t!("studio.width_tip", n = w))
                    .when(on, |d| d.bg(p.raised).text_color(p.text).shadow_sm())
                    .when(!on, |d| d.text_color(p.text2).hover(move |s| s.bg(p.wash)))
                    .child(w.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| this.studio_width(Some(w), cx))),
            );
        }
        seg
    });
    let working = shell.studio_member().is_some_and(|m| m.presence == crate::state::Presence::Working && m.ticket == Some(id));
    let live = h_flex()
        .flex_none()
        .gap_1()
        .items_center()
        .child(div().size(px(6.)).rounded_full().bg(if working { tone_color(p, Tone::Busy) } else { p.muted }))
        .child(div().text_size(px(META)).text_color(p.muted).child(if working { t!("studio.live") } else { t!("studio.latest") }));
    // Directions out, none picked: each, side by side, to pick from.
    if !centre.choices.is_empty() {
        let n = centre.choices.len().min(3);
        let room = (shots.width(rules_key("stage"), 720.) - 32.).max(200.);
        let tile_w = ((room - 12. * (n as f32 - 1.)) / n as f32).max(140.);
        let tile_h = (tile_w * 0.62).min(STAGE_H);
        let all: Vec<String> = centre.choices.iter().map(|(_, p)| p.clone()).collect();
        let row = h_flex().w_full().gap_3().flex_wrap().children(centre.choices.iter().take(3).enumerate().map(|(k, (name, path))| {
            let look = shots.ask(path, Size::Full, cx);
            let viewer = viewer_of(id, &all, path, name);
            v_flex()
                .id(("studio-choice", k))
                .gap_1p5()
                .cursor_pointer()
                .child(picture(&shots, crate::render::hash_of(("studio-choice", path)), path, look, tile_w, tile_h, R_MD, false, 0.))
                .child(div().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(name.clone()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(v) = viewer.clone() {
                        this.open_viewer(v, cx);
                    }
                }))
        }));
        let head = h_flex().w_full().gap_3().items_center().child(live).child(div().flex_1().min_w_0().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(t!("studio.choices", count = centre.choices.len())));
        return shots.measured(frame(), rules_key("stage")).child(head).child(row).into_any_element();
    }
    let rel = files.dir.as_deref().and_then(|d| centre.shown.strip_prefix(d)).map(|r| r.trim_start_matches('/').to_owned()).unwrap_or_else(|| crate::visuals::name_of(&centre.shown).to_owned());
    let talk = super::design_talk::say(p, "studio-say", "studio-stage-pic", DesignContext::Asset { name: rel, path: centre.shown.clone() }, cx);
    let head = h_flex()
        .w_full()
        .gap_3()
        .flex_wrap()
        .items_center()
        .child(live)
        .child(div().flex_1().min_w(px(160.)).truncate().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(caption(&centre, now)))
        .children(widths);
    let room = (shots.width(rules_key("stage"), 720.) - 32.).max(200.);
    let dims = shots.best_dims(&centre.shown).unwrap_or((1280, 800));
    let (w, h) = crate::visuals::contain(dims, (room, STAGE_H));
    let look = shots.ask(&centre.shown, Size::Full, cx);
    let viewer = viewer_of(id, &centre.widths.iter().map(|(_, p)| p.clone()).collect::<Vec<_>>(), &centre.shown, &caption(&centre, now)).or_else(|| viewer_of(id, std::slice::from_ref(&centre.shown), &centre.shown, &caption(&centre, now)));
    let pic = div()
        .id("studio-stage-pic")
        .group("studio-stage-pic")
        .relative()
        .w_full()
        .flex()
        .justify_center()
        .cursor_pointer()
        .child(picture(&shots, crate::render::hash_of(("studio-stage", &centre.shown)), &centre.shown, look, w.max(80.), h.max(60.), R_MD, false, 0.))
        .child(div().absolute().top_2().right_2().child(talk))
        .tip(p, t!("studio.see_big"), None)
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(v) = viewer.clone() {
                this.open_viewer(v, cx);
            }
        }));
    shots.measured(frame(), rules_key("stage")).child(head).child(pic).into_any_element()
}

fn rules_key(name: &str) -> u64 {
    crate::render::hash_of(("studio", name))
}

/// The big view over `pictures`, open at `at`, each called `label` and its width.
fn viewer_of(id: TicketId, pictures: &[String], at: &str, label: &str) -> Option<Viewer> {
    let items: Vec<crate::visuals::Item> = pictures
        .iter()
        .map(|p| {
            let width = equipe_core::design::studio::width_of(p).map(|w| format!(" · {w}px")).unwrap_or_default();
            crate::visuals::Item { path: p.clone(), choice: None, label: format!("{label}{width}") }
        })
        .collect();
    let at = items.iter().position(|i| i.path == at)?;
    Some(Viewer { owner: crate::visuals::Owner::Ticket(id), items, at })
}

/// What it left on the way, newest first.
fn timeline(shell: &Shell, items: &[Item], now: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let shots = Shots::of(shell, cx.entity().downgrade(), cx);
    let head = h_flex()
        .gap_1p5()
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.text2)
        .child(t!("studio.timeline"))
        .when(!items.is_empty(), |h| h.child(div().text_color(p.muted).font_weight(FontWeight::NORMAL).child(items.len().to_string())));
    if items.is_empty() {
        return v_flex().gap_2().child(head).child(div().text_size(px(SMALL)).text_color(p.muted).child(t!("studio.timeline_empty"))).into_any_element();
    }
    // Newest first, wrapping: everything it left is in sight as the page scrolls.
    let mut strip = h_flex().id("studio-strip").w_full().gap_3().flex_wrap().items_start().role(Role::List).aria_label(t!("studio.timeline"));
    for (n, item) in items.iter().enumerate() {
        let open = shell.studio.open.as_deref() == Some(item.key.as_str());
        let key = item.key.clone();
        let pic = item.pictures.first().map(|path| {
            let look = shots.ask(path, Size::Thumb, cx);
            picture(&shots, crate::render::hash_of(("studio-card", path)), path, look, CARD_W, CARD_PIC_H, R_MD, true, 4.)
        });
        let when = item.at.map(|at| crate::activity::ago(now.saturating_sub(at)));
        let card = v_flex()
            .id(("studio-item", n))
            .flex_none()
            .w(px(CARD_W))
            .rounded(px(R_MD))
            .bg(p.raised)
            .border_1()
            .border_color(if open { p.accent } else { p.hair })
            .cursor_pointer()
            .hover(move |s| s.border_color(p.accent))
            .role(Role::ListItem)
            .aria_label(format!("{} · {}", kind_words(item.kind), item.title))
            .children(pic)
            .child(
                v_flex()
                    .p_2p5()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .child(icon(if item.tone == Tone::Bad { Icon::Alert } else { kind_icon(item.kind) }, 12., tone_color(p, item.tone)))
                            .child(div().flex_1().min_w_0().truncate().text_size(px(META)).text_color(p.muted).child(kind_words(item.kind)))
                            .children(when.map(|w| div().flex_none().text_size(px(11.)).text_color(p.muted).child(w))),
                    )
                    .child(div().min_w_0().truncate().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(item.title.clone()))
                    .children(item.lines.iter().take(2).map(|l| div().min_w_0().truncate().text_size(px(META)).text_color(p.text2).child(l.clone()))),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.studio_open_item(Some(key.clone()), cx)));
        strip = strip.child(card);
    }
    v_flex().w_full().gap_2().child(head).child(strip).into_any_element()
}

// ── what the screen does ──

impl Shell {
    /// The member at work on the ticket watched: the one on it now, else
    /// whoever it is assigned to.
    pub fn studio_member(&self) -> Option<&Member> {
        let id = self.studio.ticket?;
        let ticket = self.model.tickets.iter().find(|t| t.id == id);
        let on = self.model.members.iter().find(|m| m.ticket == Some(id));
        let named = ticket.and_then(|t| t.assignee.as_deref().or(t.owner.as_deref())).and_then(|n| self.model.members.iter().find(|m| m.name == n));
        let worked = self.model.convos.iter().find(|(_, c)| c.has_work_on(id)).and_then(|(s, _)| self.model.members.iter().find(|m| m.session == *s));
        on.or(worked).or(named)
    }

    /// What the member did on the ticket, from its conversation.
    pub fn studio_happenings(&self) -> Vec<rules::Happening> {
        let (Some(id), Some(m)) = (self.studio.ticket, self.studio_member()) else { return Vec::new() };
        self.model.convos.get(&m.session).map(|c| rules::happenings(&rules::work_of(&c.transcript, id))).unwrap_or_default()
    }

    /// The room beside the sidebar, less the conversation docked at its side.
    pub fn studio_room(&self) -> f32 {
        self.studio.room
    }

    /// Watch `ticket`'s design work: the studio, its folder asked about, the
    /// conversation beside it the member's.
    pub fn open_studio(&mut self, ticket: TicketId, window: &mut Window, cx: &mut Context<Self>) {
        self.studio.watch(ticket);
        self.studio.wait_focus.get_or_insert_with(|| cx.focus_handle().tab_stop(true));
        self.studio.wait_tab_used.set(false);
        self.show(Screen::Studio, cx);
        self.studio_ask(cx);
        if self.design_talk_open(window, cx) {
            self.design_talk_follow(window, cx);
        }
        cx.notify();
    }

    /// Each draw: the folder asked about again while the studio is open, and
    /// the conversation beside it the member's.
    pub fn studio_prepare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen != Screen::Studio {
            self.studio.poll = None;
            return;
        }
        let room = self.design_talk_room(window, cx);
        let docked = if self.design_talk_open(window, cx) { crate::design_talk::width(self.remembered.design_talk_width, room) } else { 0. };
        self.studio.room = room - docked;
        // The conversation panel is shut, so its box is not on the screen: keys held by it
        // reach no shortcut (⌘/, ⌘K…). The window takes them.
        if docked == 0. && self.composer.is_focused(window, cx) {
            self.root_focus.focus(window, cx);
        }
        if self.studio.poll.is_none() {
            let poll = cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(rules::EVERY).await;
                    let on = this.update(cx, |this, cx| {
                        let on = this.screen == Screen::Studio;
                        if on {
                            this.studio_ask(cx);
                        }
                        on
                    });
                    if !matches!(on, Ok(true)) {
                        break;
                    }
                }
            });
            self.studio.poll = Some(poll);
        }
        if self.design_talk_open(window, cx) && self.design_talk_behind() {
            cx.defer_in(window, |this, window, cx| this.design_talk_follow(window, cx));
        }
    }

    /// Ask the daemon what the ticket's design folder holds now.
    pub fn studio_ask(&mut self, cx: &mut Context<Self>) {
        let (Some(ticket), false) = (self.studio.ticket, self.studio.asking) else { return };
        let Some(uploader) = self.link.as_ref().map(|l| l.uploader()) else { return };
        self.studio.asking = true;
        self.studio.asked_at = Some(std::time::Instant::now());
        let squad = self.model.squad.clone();
        let asked = cx.background_spawn(async move {
            uploader.call(rpc::DESIGN_STUDIO, StudioAsk { squad, ticket }, std::time::Duration::from_secs(20)).and_then(|v| serde_json::from_value::<Studio>(v).map_err(|e| e.to_string()))
        });
        cx.spawn(async move |this, cx| {
            let answer = asked.await;
            this.update(cx, |this, cx| {
                this.studio.asking = false;
                if let Ok(studio) = answer
                    && this.studio.ticket == Some(studio.ticket)
                {
                    this.studio_arrived(studio, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn studio_arrived(&mut self, studio: Studio, cx: &mut Context<Self>) {
        let same = self.studio.files().is_some_and(|f| f.stamp == studio.stamp);
        let changed = self.studio.arrived(studio);
        if !changed.is_empty() {
            self.shots.borrow_mut().refresh(&changed);
        }
        if !same {
            cx.notify();
        }
    }

    /// A picture of the strip shown big in the middle; `None`: the newest, as it lands.
    pub fn studio_pick(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        self.studio.picked = if self.studio.picked == path { None } else { path };
        self.studio.open = None;
        self.studio.width = None;
        cx.notify();
    }

    pub fn studio_width(&mut self, width: Option<u32>, cx: &mut Context<Self>) {
        self.studio.width = width;
        cx.notify();
    }

    /// A timeline item opened in the middle, or put back (`None`).
    pub fn studio_open_item(&mut self, key: Option<String>, cx: &mut Context<Self>) {
        self.studio.open = if self.studio.open == key { None } else { key };
        if self.studio.open.is_some() {
            self.studio.scroll.set_offset(point(px(0.), px(0.)));
        }
        cx.notify();
    }

    /// The live browser, where the member is browsing: its conversation, the panel open.
    pub fn studio_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.studio_member().map(|m| m.session) {
            self.model.select(session);
            self.refresh(window, cx);
        }
        if !self.browser.read(cx).open {
            self.toggle_browser(window, cx);
        } else {
            self.show(Screen::Chat, cx);
        }
    }

    /// Whether the studio is offered for `ticket` (its card, its lane, its conversation's header).
    pub fn studio_offered(&self, ticket: TicketId) -> bool {
        let Some(t) = self.model.tickets.iter().find(|t| t.id == ticket) else { return false };
        let role = t.assignee.as_deref().or(t.owner.as_deref()).and_then(|n| self.model.members.iter().find(|m| m.name == n)).map(|m| m.role.as_str());
        let worked = self.model.convos.values().find(|c| c.has_work_on(ticket)).map(|c| rules::happenings(&rules::work_of(&c.transcript, ticket))).unwrap_or_default();
        t.status != TicketStatus::Cancelled && rules::offered(t, role.or(t.owner.as_deref()), &worked)
    }

    /// The snapshot tool's `studio:<ticket>` (`snapshot.rs`); `studio:width:<n>`,
    /// `studio:item:<n>` (the nth on the timeline, opened), `studio:back`.
    pub fn snapshot_studio(&mut self, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        match what.split_once(':') {
            Some(("width", n)) => self.studio_width(n.parse().ok(), cx),
            Some(("item", n)) => {
                let files = self.studio.files().cloned();
                let items = rules::items(&self.studio_happenings(), files.as_ref());
                let key = n.parse::<usize>().ok().and_then(|n| items.get(n)).map(|i| i.key.clone());
                self.studio.open = None;
                self.studio_open_item(key, cx);
            }
            _ if what == "back" => self.studio_open_item(None, cx),
            _ => {
                if let Ok(id) = what.trim_start_matches("T-").parse() {
                    self.open_studio(id, window, cx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{caption, mark_words, step_label};
    use crate::studio::{Centre, Mark, Step, StepKind};

    #[test]
    fn a_step_says_where_it_stands_and_how_much_there_is() {
        let s = Step { kind: StepKind::Checks, mark: Mark::Attention, count: Some("기준선 10/11".into()), at: None, detail: None };
        assert_eq!(step_label(&s), "검사: 고칠 것 있음 — 기준선 10/11");
        let ahead = Step { kind: StepKind::HandIn, mark: Mark::Ahead, count: None, at: None, detail: None };
        assert_eq!(step_label(&ahead), "넘기기: 아직");
        for mark in [Mark::Ahead, Mark::Skipped, Mark::Done, Mark::Now, Mark::Attention, Mark::Waiting] {
            assert!(!mark_words(mark).is_empty());
        }
    }

    #[test]
    fn the_centre_says_what_it_shows_and_how_fresh_it_is() {
        let check = Centre { screen: Some("onboarding/welcome".into()), widths: vec![], shown: "/w/design/checks/onboarding/welcome/1280.png".into(), width: Some(1280), at: 1_000, direction: None, choices: vec![], from: None };
        assert_eq!(caption(&check, 1_000 + 180), "onboarding/welcome · 3분 전 점검");
        let made = Centre { screen: None, widths: vec![], shown: "/w/design/mockups/welcome-v2.png".into(), width: None, at: 1_000, direction: None, choices: vec![], from: None };
        assert_eq!(caption(&made, 1_000), "welcome-v2.png · 방금");
    }

    #[test]
    fn a_picture_found_elsewhere_says_where_it_came_from() {
        let c = Centre { screen: None, widths: vec![], shown: "/tmp/t/ax5.png".into(), width: None, at: 1_000, direction: None, choices: vec![], from: Some(equipe_core::protocol::studio::PictureFrom::Opened) };
        assert_eq!(caption(&c, 1_000), "ax5.png · 본 것 · 방금");
        let rec = Centre { from: Some(equipe_core::protocol::studio::PictureFrom::Recording), ..c };
        assert_eq!(caption(&rec, 1_060), "ax5.png · 폰 녹화 · 1분 전");
    }

    /// Waiting for the person is read from what the sidebar reads
    /// (`Model::status`): the member's permission asks, its questions, its
    /// tickets waiting — never from the ticket alone.
    #[test]
    fn its_wait_is_read_from_what_the_sidebar_reads() {
        let source = include_str!("studio.rs");
        let code = &source[..source.find("#[cfg(test)]").unwrap()];
        for read in ["approvals.first_for(", "questions_waiting(", "TicketStatus::Waiting && t.owner"] {
            assert!(code.contains(read), "the studio reads {read}");
        }
    }

    /// Nothing here changes anything: the studio only reads.
    #[test]
    fn the_studio_sends_no_command_and_asks_only_what_reads() {
        let source = include_str!("studio.rs");
        let code = &source[..source.find("#[cfg(test)]").unwrap()];
        for never in ["send_command(", "Command::", "self.send(", "send_as("] {
            assert!(!code.contains(never), "the studio uses {never}");
        }
        assert!(crate::readonly::reads_method(equipe_core::protocol::rpc::DESIGN_STUDIO), "the real view may ask it");
    }
}
