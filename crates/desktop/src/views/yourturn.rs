//! The card a ticket put down for the person is answered on — the same in
//! the conversation and in the Inbox's peek: a header (내 차례 · T-n · who ·
//! the title), the asks numbered, each with what it wants back — a box for
//! words (a quick "괜찮음" where an OK is asked for), or a tick for a thing to
//! do — and the files it names as chips that open; at its foot "답 보내기"
//! (⌘↵), which sends every answer in one message and puts the member back to
//! work, and who is waiting. Sent, it folds to one line, "답함 · 3개 · 방금",
//! that opens to what was sent; answered on another screen, it says so the
//! moment the ticket moves. The rules are `crate::yourturn`'s.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::rc::Rc;

use equipe_core::protocol::asks::{self, Reply};
use equipe_core::protocol::{Command, Ticket, TicketId, TicketStatus};
use equipe_core::t;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::render::{self, Open};
use crate::theme::{MONO, Palette, R_LG, R_SM, SMALL, look};
use crate::tips::tip;
use crate::ui::{self, Kind};
use crate::yourturn::{self as rules, Answered, Named, Phase, Want};
use crate::{Screen, Shell};

/// Where a card is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// In the conversation: its own header, and a way to the Inbox.
    Conversation,
    /// The Inbox's peek, under the ticket's own head.
    Peek,
    /// The ticket beside the conversation: the asks to read, answered in the
    /// conversation's card.
    Side,
}

/// The answer boxes, by ticket and ask, kept while the ticket waits: the
/// conversation's card and the peek's type into the same ones.
#[derive(Default)]
pub struct TurnUi {
    pub fields: HashMap<(TicketId, usize), Entity<TextareaState>>,
    subscriptions: HashMap<(TicketId, usize), Subscription>,
}

/// What a card shows, gathered from the shell before it is drawn (rows are
/// drawn without it).
#[derive(Clone)]
pub struct Card {
    pub ticket: TicketId,
    pub title: String,
    pub who: Option<(String, Rgba)>,
    pub why: Option<String>,
    pub asks: Vec<String>,
    pub wants: Vec<Want>,
    pub named: Vec<Vec<Named>>,
    pub fields: Vec<Option<Entity<TextareaState>>>,
    pub ticked: BTreeSet<usize>,
    pub phase: Phase,
    pub answered: Option<Answered>,
    pub can_send: bool,
    pub now: u64,
}

impl Card {
    /// The card for ticket `id`, as things stand.
    pub fn of(shell: &Shell, id: TicketId, cx: &App) -> Option<Rc<Card>> {
        let model = &shell.model;
        let t = model.tickets.iter().find(|t| t.id == id);
        let answered = model.answers.get(&id).cloned();
        let phase = rules::phase(t, answered.as_ref());
        let asks = t.map(Ticket::asks).unwrap_or_default();
        let wants: Vec<Want> = asks.iter().map(|a| rules::want_of(a)).collect();
        let roots = roots(shell, t);
        let named = asks.iter().map(|a| rules::named(a, &roots)).collect();
        let fields: Vec<Option<Entity<TextareaState>>> = (0..asks.len()).map(|n| shell.turn.fields.get(&(id, n)).cloned()).collect();
        let typed: Vec<String> = fields.iter().map(|f| f.as_ref().map(|f| f.read(cx).value().to_string()).unwrap_or_default()).collect();
        let ticked = shell.board.ticked.get(&id).cloned().unwrap_or_default();
        let can_send = rules::can_send(&wants, &rules::replies(&wants, &typed, &ticked)) && shell.can_act();
        let who = t.and_then(|t| t.assignee.clone().or(t.owner.clone())).map(|name| {
            let member = model.members.iter().find(|m| m.name == name);
            let (_, hue) = member.map_or_else(|| look(&name, &name), |m| look(&m.role, &m.name));
            (name, shell.palette.member(hue))
        });
        Some(Rc::new(Card {
            ticket: id,
            title: t.map(|t| t.title.clone()).unwrap_or_default(),
            who,
            why: t.and_then(|t| t.note.as_deref()).and_then(asks::waiting_why),
            asks,
            wants,
            named,
            fields,
            ticked,
            phase,
            answered,
            can_send,
            now: crate::state::now_unix(),
        }))
    }
}

/// Where a ticket's files are looked for: its checkout, the conversation's
/// folder, the squad's projects.
fn roots(shell: &Shell, t: Option<&Ticket>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    out.extend(t.and_then(|t| t.worktree.clone()));
    out.extend(t.and_then(|t| t.project.clone()));
    let base = render::base_dir(shell);
    if !base.as_os_str().is_empty() {
        out.push(base);
    }
    out.extend(shell.model.projects.iter().cloned());
    out
}

/// How each card of a waiting (or answered) ticket looks now, by ticket:
/// a change draws its row again.
pub fn looks(shell: &Shell, cx: &App) -> HashMap<TicketId, u64> {
    let model = &shell.model;
    model
        .tickets
        .iter()
        .filter(|t| t.status == TicketStatus::Waiting || model.answers.contains_key(&t.id))
        .map(|t| {
            let fields = shell.turn.fields.keys().filter(|(id, _)| *id == t.id).count();
            let answered = model.answers.get(&t.id).map(|a| (a.text.clone(), a.here, a.settled));
            let _ = cx;
            (t.id, render::hash_of((format!("{:?}", t.status), t.asks(), answered, fields, &t.title)))
        })
        .collect()
}

/// The disclosure of an answered card.
pub const PART_SENT: u32 = (1 << 20) + 8;

/// How a card is drawn: by whom (`handle`), with what open.
pub struct Draw<'a> {
    pub p: Palette,
    pub tones: render::Tones,
    pub handle: WeakEntity<Shell>,
    pub key: u64,
    pub open: bool,
    pub place: Place,
    pub window: Option<&'a mut Window>,
}

/// The card: the asks to answer while it waits; one line once answered.
pub fn card(card: &Card, d: Draw<'_>, cx: &mut App) -> AnyElement {
    match (card.phase, d.place) {
        (Phase::Pending, _) | (_, Place::Side) => pending(card, &d, cx),
        _ => settled(card, d, cx),
    }
}

/// An earlier card for a ticket asked again since: the line it came as.
pub fn past(p: Palette, tones: render::Tones, news: &str) -> AnyElement {
    h_flex()
        .h(px(24.))
        .gap(px(8.))
        .items_center()
        .text_size(px(SMALL))
        .text_color(p.muted)
        .child(icon(Icon::Clock, 13., tones.warning))
        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(news.to_owned()))
        .into_any_element()
}

fn pill(p: Palette, tones: render::Tones) -> impl IntoElement {
    let _ = p;
    div()
        .flex_none()
        .px(px(7.))
        .h(px(20.))
        .flex()
        .items_center()
        .rounded_full()
        .bg(Rgba { a: 0.16, ..tones.warning })
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(tones.warning)
        .child(t!("turn.pill"))
}

/// `T-117`, a way to the ticket beside the conversation.
fn ticket_link(p: Palette, id: TicketId, key: u64, handle: &WeakEntity<Shell>) -> impl IntoElement {
    let handle = handle.clone();
    div()
        .id(("turn-ticket", key))
        .flex_none()
        .font_family(MONO)
        .text_size(px(12.5))
        .text_color(p.text2)
        .cursor_pointer()
        .hover(move |s| s.text_color(p.accent).underline())
        .child(format!("T-{id}"))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            handle.update(cx, |shell, cx| shell.side_open(id, cx)).ok();
        })
}

fn header(card: &Card, d: &Draw<'_>) -> impl IntoElement {
    let p = d.p;
    h_flex()
        .min_h(px(24.))
        .gap_2()
        .items_center()
        .text_size(px(SMALL))
        .child(pill(p, d.tones))
        .child(ticket_link(p, card.ticket, d.key, &d.handle))
        .children(card.who.clone().map(|(name, color)| h_flex().flex_none().gap(px(5.)).items_center().text_color(p.text2).child(div().size(px(6.)).rounded_full().bg(color)).child(name)))
        .child(div().min_w_0().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(card.title.clone()))
}

/// The asks, each answered in place, and the foot.
fn pending(card: &Card, d: &Draw<'_>, cx: &mut App) -> AnyElement {
    let p = d.p;
    let key = d.key;
    let items: Vec<AnyElement> = card.asks.iter().enumerate().map(|(n, ask)| item(card, d, n, ask, cx)).collect();
    let fields: Vec<Entity<TextareaState>> = card.fields.iter().flatten().cloned().collect();
    let tab = fields.clone();
    let body = v_flex()
        .id(("turn-card", key))
        .w_full()
        .gap_3()
        // Tab and ⇧Tab go from box to box.
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if event.keystroke.key != "tab" || tab.is_empty() {
                return;
            }
            let at = tab.iter().position(|f| f.read(cx).focus_handle(cx).is_focused(window));
            let Some(at) = at else { return };
            let next = if event.keystroke.modifiers.shift { (at + tab.len() - 1) % tab.len() } else { (at + 1) % tab.len() };
            tab[next].update(cx, |s, cx| s.focus(window, cx));
            cx.stop_propagation();
        })
        .when(d.place == Place::Conversation, |c| c.child(header(card, d)))
        .children(card.why.clone().filter(|_| d.place == Place::Conversation).map(|why| div().mt(px(-6.)).text_size(px(12.5)).line_height(px(18.)).text_color(p.muted).child(why)))
        .child(v_flex().gap_3().children(items))
        .when(d.place != Place::Side, |c| c.child(foot(card, d)));
    match d.place {
        Place::Conversation => div()
            .w_full()
            .mt_2()
            .mb_1()
            .p(px(14.))
            .rounded(px(R_LG))
            .border_1()
            .border_color(Rgba { a: 0.45, ..d.tones.warning })
            .bg(Rgba { a: 0.04, ..d.tones.warning })
            .child(body)
            .into_any_element(),
        _ => body.into_any_element(),
    }
}

/// One ask: its number, its words (Markdown), the files it names, and its
/// box or tick.
fn item(card: &Card, d: &Draw<'_>, n: usize, ask: &str, cx: &mut App) -> AnyElement {
    let p = d.p;
    let key = d.key;
    let id = card.ticket;
    let want = card.wants.get(n).copied().unwrap_or(Want::Answer);
    let done = want == Want::Do && card.ticked.contains(&n);
    let number = div()
        .flex_none()
        .size(px(20.))
        .mt(px(1.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(if done { d.tones.added } else { p.hair })
        .when(done, |c| c.bg(Rgba { a: 0.14, ..d.tones.added }))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if done { d.tones.added } else { p.text2 })
        .child(if done { "✓".to_owned() } else { (n + 1).to_string() });
    let shown = rules::mark_paths(ask, card.named.get(n).map(Vec::as_slice).unwrap_or_default());
    let text = super::chat::markdown(p, ("turn-ask", hash(key, n)), &shown, crate::theme::UI, crate::theme::UI_PROSE_LINE, if done { p.muted } else { p.text }, &d.handle);
    let files = card.named.get(n).filter(|f| !f.is_empty()).map(|files| h_flex().flex_wrap().gap_1p5().children(files.iter().enumerate().map(|(k, f)| file_chip(p, f, hash(key, n << 8 | k)))));
    let answer: Option<AnyElement> = match (want, d.place) {
        (_, Place::Side) => None,
        (Want::Do, _) => Some(tick(card, d, n, done).into_any_element()),
        (_, _) => card.fields.get(n).cloned().flatten().map(|field| {
            let ok = (want == Want::Confirm).then(|| {
                let handle = d.handle.clone();
                let label = t!("turn.ok");
                div()
                    .id(("turn-ok", hash(key, n)))
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
                    .child(icon(Icon::Check, 12., d.tones.added))
                    .child(label)
                    .on_click(move |_, window, cx| {
                        handle.update(cx, |shell, cx| shell.turn_say(id, n, label, window, cx)).ok();
                    })
            });
            h_flex()
                .w_full()
                .gap_2()
                .items_start()
                .child(div().flex_1().min_w_0().px_3().py(px(2.)).rounded(px(R_SM + 2.)).bg(p.bg).border_1().border_color(p.hair).text_size(px(14.)).child(Textarea::new(&field).appearance(false).bordered(false)))
                .children(ok)
                .into_any_element()
        }),
    };
    let _ = cx;
    h_flex()
        .w_full()
        .gap_2p5()
        .items_start()
        .child(number)
        .child(v_flex().flex_1().min_w_0().gap_2().child(text).children(files).children(answer))
        .into_any_element()
}

fn hash(key: u64, n: usize) -> u64 {
    render::hash_of((key, n))
}

/// A thing to do: ticked off when done.
fn tick(card: &Card, d: &Draw<'_>, n: usize, done: bool) -> impl IntoElement {
    let p = d.p;
    let id = card.ticket;
    let handle = d.handle.clone();
    h_flex()
        .id(("turn-tick", hash(d.key, n)))
        .gap_2()
        .items_center()
        .text_size(px(12.5))
        .text_color(if done { p.text } else { p.text2 })
        .cursor_pointer()
        .child(
            div()
                .size(px(16.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .border_color(if done { d.tones.added } else { p.muted })
                .when(done, |c| c.bg(d.tones.added).child(icon(Icon::Check, 11., p.bg))),
        )
        .child(t!("turn.done"))
        .on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.turn_tick(id, n, cx)).ok();
        })
}

/// A file or folder an ask names: on this Mac a click looks at it (⌥:
/// Finder); elsewhere its path is copied.
fn file_chip(p: Palette, f: &Named, key: u64) -> impl IntoElement {
    let folder = f.written.ends_with('/') || f.at.as_ref().is_some_and(|a| a.is_dir());
    let name = f.written.trim_end_matches('/').rsplit('/').next().unwrap_or(&f.written).to_owned();
    let label = if folder { format!("{name}/") } else { name };
    let here = f.at.is_some();
    let path = f.at.clone().unwrap_or_else(|| PathBuf::from(&f.written));
    let chip = h_flex()
        .id(("turn-file", key))
        .h(px(24.))
        .px_2()
        .gap_1p5()
        .items_center()
        .rounded(px(R_SM))
        .bg(p.code_bg)
        .border_1()
        .border_color(p.hair)
        .text_size(px(12.))
        .font_family(MONO)
        .text_color(if here { p.accent } else { p.text2 })
        .cursor_pointer()
        .hover(move |s| s.border_color(p.muted))
        .child(icon(if folder { Icon::Folder } else { Icon::File }, 12., if here { p.accent } else { p.muted }))
        .child(label)
        .on_click(move |event, _, cx| {
            cx.stop_propagation();
            let how = if event.modifiers().alt { Open::Reveal } else { Open::Look };
            render::open_file(&path, how, cx);
        });
    let words = if here { t!("turn.file_look") } else { t!("turn.file_copy") };
    tip(chip, p, format!("{} — {words}", f.written), None)
}

/// Who waits, and the buttons: to the Inbox (from the conversation) and send.
fn foot(card: &Card, d: &Draw<'_>) -> impl IntoElement {
    let p = d.p;
    let id = card.ticket;
    let only_things = card.wants.iter().all(|w| *w == Want::Do);
    let send_label = if only_things { t!("turn.carry_on") } else { t!("turn.send") };
    let send = {
        let handle = d.handle.clone();
        ui::button(p, ("turn-send", d.key), Kind::Primary, send_label).key("⌘↵").disabled(!card.can_send).on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.turn_send(id, window, cx)).ok();
        })
    };
    let board = (d.place == Place::Conversation).then(|| {
        let handle = d.handle.clone();
        ui::button(p, ("turn-board", d.key), Kind::Ghost, t!("turn.open_board")).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.open_ticket(id, cx)).ok();
        })
    });
    let waiting = card.who.as_ref().map(|(name, _)| t!("turn.waiting", name = name));
    h_flex()
        .w_full()
        .pt_3()
        .gap_3()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .child(v_flex().flex_1().min_w_0().gap_0p5().text_size(px(12.)).line_height(px(17.)).text_color(p.muted).children(waiting).when(!card.fields.iter().all(Option::is_none), |c| c.child(div().opacity(0.8).child(t!("turn.keys")))))
        .children(board)
        .child(send)
}

/// Answered (or on its way, or no longer asked): one line, and what was sent
/// under it when opened.
fn settled(card: &Card, d: Draw<'_>, cx: &mut App) -> AnyElement {
    let p = d.p;
    let key = d.key;
    let ago = card.answered.as_ref().and_then(|a| a.at).map(|at| crate::activity::ago(card.now.saturating_sub(at)));
    let count = card.answered.as_ref().and_then(rules::answered_count);
    let (mark, words): (AnyElement, String) = match card.phase {
        Phase::Sending if !cx.reduce_motion() => (Spinner::new().with_size(px(12.)).color(Hsla::from(p.muted)).into_any_element(), t!("turn.sending").to_owned()),
        Phase::Sending => (div().child("◌").into_any_element(), t!("turn.sending").to_owned()),
        Phase::Closed => (icon(Icon::XCircle, 13., p.muted).into_any_element(), t!("turn.closed").to_owned()),
        Phase::Answered { here } => {
            let head = if here { t!("turn.answered") } else { t!("turn.answered_elsewhere") };
            let mut parts = vec![head.to_owned()];
            parts.extend(count.map(|n| t!("turn.count", count = n)));
            parts.extend(ago);
            (icon(Icon::Check, 13., d.tones.added).into_any_element(), parts.join(" · "))
        }
        Phase::Pending => (div().into_any_element(), String::new()),
    };
    let has_more = matches!(card.phase, Phase::Answered { .. } | Phase::Sending);
    let progress = match d.window {
        Some(window) => crate::motion::openness(("turn-sent-open", key), d.open && has_more, window, cx),
        None => f32::from(u8::from(d.open && has_more)),
    };
    let handle = d.handle.clone();
    let line = h_flex()
        .id(("turn-line", key))
        .min_h(px(28.))
        .gap(px(8.))
        .items_center()
        .text_size(px(SMALL))
        .text_color(p.muted)
        .when(has_more, |l| {
            l.cursor_pointer().hover(move |s| s.text_color(p.text2)).on_click(move |_, _, cx| {
                handle.update(cx, |shell, cx| shell.chat_toggle(key, PART_SENT, cx)).ok();
            })
        })
        .child(mark)
        .child(div().flex_none().child(words))
        .child(ticket_link(p, card.ticket, key, &d.handle))
        .child(div().min_w_0().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(card.title.clone()))
        .when(has_more, |l| l.child(icon(Icon::ChevronRight, 12., p.muted).with_transformation(Transformation::rotate(radians(progress * std::f32::consts::FRAC_PI_2)))));
    let body = (progress > 0.).then(|| {
        let text = card.answered.as_ref().map(|a| a.text.clone()).unwrap_or_default();
        let inner: AnyElement = if text.trim().is_empty() {
            div().text_size(px(12.5)).text_color(p.muted).child(if card.answered.as_ref().is_some_and(|a| !a.here) { t!("turn.not_here") } else { t!("turn.answered") }).into_any_element()
        } else {
            v_flex()
                .gap_1()
                .child(div().text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("turn.sent_heading")))
                .child(div().text_size(px(13.)).line_height(px(20.)).text_color(p.text2).child(gpui_kit::base::SelectableText::new(("turn-sent-text", key), text)))
                .into_any_element()
        };
        crate::motion::Reveal::new(("turn-sent-body", key), progress, div().opacity(progress).ml(px(21.)).mt_1().px_3().py_2().rounded(px(R_SM)).bg(p.code_bg).child(inner).into_any_element())
    });
    crate::motion::fade(v_flex().w_full().child(line).children(body), ("turn-settled", render::hash_of((key, format!("{:?}", card.phase)))), crate::motion::BASE).into_any_element()
}

// ── the shell's side ──

impl Shell {
    /// Answer boxes for every ask of every waiting ticket that wants words;
    /// a ticket no longer waiting lets its boxes go.
    pub fn turn_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut wanted: Vec<(TicketId, usize)> = Vec::new();
        for t in self.model.tickets.iter().filter(|t| t.status == TicketStatus::Waiting) {
            for (n, ask) in t.asks().iter().enumerate() {
                if rules::want_of(ask) != Want::Do {
                    wanted.push((t.id, n));
                }
            }
        }
        self.turn.fields.retain(|k, _| wanted.contains(k));
        self.turn.subscriptions.retain(|k, _| wanted.contains(k));
        for k in wanted {
            if self.turn.fields.contains_key(&k) {
                continue;
            }
            let field = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 6).submit_on_enter(false).placeholder(t!("turn.answer_placeholder")));
            let (id, _) = k;
            let subscription = cx.subscribe_in(&field, window, move |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { secondary: true, .. } => this.turn_send(id, window, cx),
                InputEvent::Change => {
                    // The box may have grown a line, and the button may wake.
                    this.chat_remeasure_turn(id, cx);
                    cx.notify();
                }
                _ => {}
            });
            self.turn.fields.insert(k, field);
            self.turn.subscriptions.insert(k, subscription);
        }
    }

    /// What a waiting ticket's card would send: every answer, numbered.
    pub fn turn_message(&self, t: &Ticket, cx: &App) -> String {
        let asks = t.asks();
        let wants: Vec<Want> = asks.iter().map(|a| rules::want_of(a)).collect();
        let typed: Vec<String> = (0..asks.len()).map(|n| self.turn.fields.get(&(t.id, n)).map(|f| f.read(cx).value().to_string()).unwrap_or_default()).collect();
        let ticked = self.board.ticked.get(&t.id).cloned().unwrap_or_default();
        asks::answer_message(&asks, &rules::replies(&wants, &typed, &ticked))
    }

    /// "답 보내기": the answers to the member in one message; the ticket goes
    /// back to it, and the card says it was answered.
    pub fn turn_send(&mut self, id: TicketId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.model.tickets.iter().find(|t| t.id == id && t.status == TicketStatus::Waiting).cloned() else { return };
        let asks = t.asks();
        let wants: Vec<Want> = asks.iter().map(|a| rules::want_of(a)).collect();
        let typed: Vec<String> = (0..asks.len()).map(|n| self.turn.fields.get(&(id, n)).map(|f| f.read(cx).value().to_string()).unwrap_or_default()).collect();
        let ticked = self.board.ticked.get(&id).cloned().unwrap_or_default();
        let replies: Vec<Reply> = rules::replies(&wants, &typed, &ticked);
        if !rules::can_send(&wants, &replies) || !self.can_act() || self.model.answers.get(&id).is_some_and(|a| a.here && !a.settled) {
            return;
        }
        let note = asks::answer_message(&asks, &replies);
        self.send_command(Command::Resume { squad: self.model.squad.clone(), ticket: id, note: note.clone() });
        self.model.answer_sent(id, note);
        self.model.act_on(id);
        self.board.ticked.remove(&id);
        window.blur(cx);
        if self.screen == Screen::Chat {
            self.composer.focus(window, cx);
        }
        self.chat_sync(cx);
        self.board_sync(cx);
        cx.notify();
    }

    /// A quick answer ("괜찮음") into ask `n`'s box.
    pub fn turn_say(&mut self, id: TicketId, n: usize, words: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(field) = self.turn.fields.get(&(id, n)).cloned() {
            field.update(cx, |s, cx| {
                s.set_value(words.to_owned(), window, cx);
                s.focus(window, cx);
            });
            cx.notify();
        }
    }

    /// A thing to do ticked off, or back on.
    pub fn turn_tick(&mut self, id: TicketId, n: usize, cx: &mut Context<Self>) {
        let set = self.board.ticked.entry(id).or_default();
        if !set.remove(&n) {
            set.insert(n);
        }
        cx.notify();
    }

    /// The first box of a waiting ticket's card takes the keys (R on the board).
    pub fn turn_focus(&mut self, id: TicketId, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let first = self.turn.fields.iter().filter(|((t, _), _)| *t == id).min_by_key(|((_, n), _)| *n).map(|(_, f)| f.clone());
        match first {
            Some(field) => {
                field.update(cx, |s, cx| s.focus(window, cx));
                cx.notify();
                true
            }
            None => false,
        }
    }
}
