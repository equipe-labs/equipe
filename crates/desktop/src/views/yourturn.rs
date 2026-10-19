//! The card a ticket put down for the person is answered on — the same in
//! the conversation and in the Inbox's peek: a header (내 차례 · T-n · who ·
//! the title), the asks numbered, each with what it wants back — a box for
//! words (a quick "괜찮음" where an OK is asked for) with "네가 정해 줘" and
//! "모르겠어요" beside it, or for a thing to do "했음", "대신 해 줘" and
//! "나중에" — one at a time, and "메모 추가" for a word on that ask; the files
//! it names as chips that open; under them a box for anything else to say to
//! the member, and at its foot "답 보내기" (⌘↵), which sends what is set in one
//! message — what isn't goes as not yet, and stays open on the ticket — and
//! puts the member back to work. Sent, it folds to one line, "답함 · 3개 ·
//! 방금", that opens to what was sent; answered on another screen, it says so
//! the moment the ticket moves. The rules are `crate::yourturn`'s.

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use equipe_core::protocol::asks::{self, Reply};
use equipe_core::protocol::{Command, Ticket, TicketId, TicketStatus};
use equipe_core::t;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::render::{self, Open};
use crate::theme::{MONO, Palette, R_LG, R_SM, SMALL, look};
use crate::tips::tip;
use crate::ui::{self, Kind};
use crate::yourturn::{self as rules, Answered, Named, Phase, Pick, Want};
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

/// A key on the card besides its boxes: an ask's pick (by its place among
/// the ask's picks), its quick OK, its "메모 추가"; the card's send button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    Pick(u8),
    Ok,
    Note,
    Send,
}

/// What a card holds while its ticket waits, by ticket and ask: the
/// conversation's card and the peek's type into the same boxes and pick the
/// same picks.
#[derive(Default)]
pub struct TurnUi {
    /// The answer boxes.
    pub fields: HashMap<(TicketId, usize), Entity<TextareaState>>,
    /// Each ask's note, once "메모 추가" opened it.
    pub notes: HashMap<(TicketId, usize), Entity<TextareaState>>,
    /// The words to the member, at the card's foot.
    pub free: HashMap<TicketId, Entity<TextareaState>>,
    /// Each ask's pick, by ticket.
    pub picks: HashMap<TicketId, HashMap<usize, Pick>>,
    /// The step the person opened (to answer again, or ahead): else the first
    /// one not yet answered is the one open.
    pub open: HashMap<TicketId, usize>,
    /// The keys Tab goes through with the boxes (the send button's ask: none, `usize::MAX`).
    keys: HashMap<(TicketId, usize, Slot), FocusHandle>,
    subscriptions: HashMap<(TicketId, usize, u8), Subscription>,
}

/// The card's own: the free box and the send button are no ask's.
const CARD: usize = usize::MAX;
/// Which box a subscription is for.
const ANSWER: u8 = 0;
const NOTE: u8 = 1;
const FREE: u8 = 2;

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
    pub notes: Vec<Option<Entity<TextareaState>>>,
    pub free: Option<Entity<TextareaState>>,
    pub picks: HashMap<usize, Pick>,
    /// What each ask sends now.
    pub replies: Vec<Reply>,
    pub keys: HashMap<(usize, Slot), FocusHandle>,
    pub phase: Phase,
    pub answered: Option<Answered>,
    pub can_send: bool,
    pub now: u64,
    /// The one step open, answered in place; the rest are a line each.
    pub current: Option<usize>,
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
        let notes: Vec<Option<Entity<TextareaState>>> = (0..asks.len()).map(|n| shell.turn.notes.get(&(id, n)).cloned()).collect();
        let free = shell.turn.free.get(&id).cloned();
        let picks = shell.turn.picks.get(&id).cloned().unwrap_or_default();
        let answers = shell.turn_answers(id, &asks, cx);
        let current = shell.turn.open.get(&id).copied().filter(|n| *n < asks.len()).or_else(|| answers.replies.iter().position(|r| r.is_empty()));
        let can_send = rules::can_send(&answers) && shell.can_act();
        let keys = shell.turn.keys.iter().filter(|((t, ..), _)| *t == id).map(|((_, n, slot), f)| ((*n, *slot), f.clone())).collect();
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
            notes,
            free,
            picks,
            replies: answers.replies,
            keys,
            phase,
            answered,
            can_send,
            now: crate::state::now_unix(),
            current,
        }))
    }

    /// The member's name, or a word for it.
    fn member(&self) -> String {
        self.who.as_ref().map(|(name, _)| name.clone()).unwrap_or_default()
    }

    /// Asks with nothing set, which go as not yet once something else is.
    fn left(&self) -> usize {
        self.replies.iter().filter(|r| r.is_empty()).count()
    }

    /// Everything Tab goes through, in order: each ask's box, its picks, its
    /// "메모 추가" and its note; the words to the member; the send button.
    fn tab_order(&self, cx: &App) -> Vec<FocusHandle> {
        let mut out = Vec::new();
        for n in 0..self.asks.len() {
            if let Some(field) = self.fields.get(n).cloned().flatten() {
                out.push(field.read(cx).focus_handle(cx));
            }
            out.extend(self.keys.get(&(n, Slot::Ok)).cloned());
            for k in 0..rules::picks_for(self.wants[n]).len() {
                out.extend(self.keys.get(&(n, Slot::Pick(k as u8))).cloned());
            }
            out.extend(self.keys.get(&(n, Slot::Note)).cloned());
            if let Some(note) = self.notes.get(n).cloned().flatten() {
                out.push(note.read(cx).focus_handle(cx));
            }
        }
        if let Some(free) = &self.free {
            out.push(free.read(cx).focus_handle(cx));
        }
        out.extend(self.keys.get(&(CARD, Slot::Send)).cloned());
        out
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
            let mut notes: Vec<usize> = shell.turn.notes.keys().filter(|(id, _)| *id == t.id).map(|(_, n)| *n).collect();
            notes.sort_unstable();
            let mut picks: Vec<(usize, Pick)> = shell.turn.picks.get(&t.id).map(|p| p.iter().map(|(n, p)| (*n, *p)).collect()).unwrap_or_default();
            picks.sort_unstable_by_key(|(n, _)| *n);
            let answered = model.answers.get(&t.id).map(|a| (a.text.clone(), a.here, a.settled));
            let _ = cx;
            (t.id, render::hash_of((format!("{:?}", t.status), t.asks(), answered, fields, notes, format!("{picks:?}"), &t.title)))
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

/// A box on the card: the answer's, a note's, the words to the member.
fn boxed(p: Palette, field: &Entity<TextareaState>, size: f32) -> Div {
    div().w_full().min_w_0().px_3().py(px(2.)).rounded(px(R_SM + 2.)).bg(p.bg).border_1().border_color(p.hair).text_size(px(size)).child(Textarea::new(field).appearance(false).bordered(false))
}

/// The asks, each answered in place, the words to the member, and the foot.
fn pending(card: &Card, d: &Draw<'_>, cx: &mut App) -> AnyElement {
    let p = d.p;
    let key = d.key;
    // One step at a time: the open one in full, the others a line each —
    // answered ones with what they say, the ones ahead to open early.
    let one_at_a_time = d.place != Place::Side && card.asks.len() > 1;
    let items: Vec<AnyElement> = card
        .asks
        .iter()
        .enumerate()
        .map(|(n, ask)| if !one_at_a_time || card.current == Some(n) { item(card, d, n, ask, cx) } else { line(card, d, n, ask) })
        .collect();
    let progress = one_at_a_time.then(|| steps_bar(card, d));
    let order = card.tab_order(cx);
    let id = card.ticket;
    let handle = d.handle.clone();
    let answering = d.place != Place::Side;
    let free = card.free.clone().filter(|_| answering).map(|free| boxed(p, &free, 13.5).mt_1());
    let body = v_flex()
        .id(("turn-card", key))
        .w_full()
        .gap_3()
        // Tab and ⇧Tab go through the card's boxes and choices; ⌘↵ sends from any of them.
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            let k = &event.keystroke;
            if k.key == "enter" && k.modifiers.platform {
                handle.update(cx, |shell, cx| shell.turn_send(id, window, cx)).ok();
                cx.stop_propagation();
                return;
            }
            if k.key != "tab" || order.is_empty() {
                return;
            }
            let Some(at) = order.iter().position(|f| f.is_focused(window)) else { return };
            let next = if k.modifiers.shift { (at + order.len() - 1) % order.len() } else { (at + 1) % order.len() };
            window.focus(&order[next], cx);
            cx.stop_propagation();
        })
        .when(d.place == Place::Conversation, |c| c.child(header(card, d)))
        .children(card.why.clone().filter(|_| d.place == Place::Conversation).map(|why| div().mt(px(-6.)).text_size(px(12.5)).line_height(px(18.)).text_color(p.muted).child(why)))
        .children(progress)
        .child(v_flex().gap(if one_at_a_time { px(6.) } else { px(18.) }).children(items))
        .children(free)
        .when(answering, |c| c.child(foot(card, d)));
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

/// How far along the steps are: a thin bar and `2 / 5`.
fn steps_bar(card: &Card, d: &Draw<'_>) -> impl IntoElement {
    let p = d.p;
    let total = card.asks.len().max(1);
    let done = card.replies.iter().filter(|r| !r.is_empty()).count();
    let share = done as f32 / total as f32;
    h_flex()
        .gap_2()
        .items_center()
        .child(div().flex_1().h(px(4.)).rounded_full().bg(p.hair).overflow_hidden().child(div().h_full().w(relative(share)).rounded_full().bg(d.tones.added)))
        .child(div().flex_none().text_size(px(12.)).text_color(p.muted).child(t!("turn.steps", done = done, total = total)))
}

/// A step that isn't open: its number (✓ when done), its words on one line,
/// and what it says back when answered. A click opens it.
fn line(card: &Card, d: &Draw<'_>, n: usize, ask: &str) -> AnyElement {
    let p = d.p;
    let id = card.ticket;
    let reply = card.replies.get(n).cloned().unwrap_or_default();
    let set = !reply.is_empty();
    let done = reply == Reply::Done;
    let words: String = ask.lines().next().unwrap_or_default().replace("**", "").replace('`', "");
    let handle = d.handle.clone();
    h_flex()
        .id(("turn-line", hash(d.key, n)))
        .w_full()
        .min_h(px(32.))
        .px_2()
        .gap_2p5()
        .items_center()
        .rounded(px(R_SM + 2.))
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.turn_open(id, n, window, cx)).ok();
        })
        .child(
            div()
                .flex_none()
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_color(if done { d.tones.added } else if set { p.accent } else { p.hair })
                .when(done, |c| c.bg(Rgba { a: 0.14, ..d.tones.added }))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if done { d.tones.added } else { p.text2 })
                .child(if done { "✓".to_owned() } else { (n + 1).to_string() }),
        )
        .child(div().flex_1().min_w_0().truncate().text_size(px(13.5)).text_color(if set { p.muted } else { p.text2 }).child(words))
        .children(set.then(|| div().flex_none().max_w(relative(0.4)).truncate().text_size(px(12.)).text_color(if done { d.tones.added } else { p.text2 }).child(reply_words(&reply))))
        .into_any_element()
}

/// What a pick is called on its button, and what it does, said in its tooltip.
fn pick_words(pick: Pick, name: &str) -> (String, String) {
    match pick {
        Pick::Done => (t!("turn.done").into(), t!("turn.tip_done", name = name)),
        Pick::Yours => (t!("turn.pick_yours").into(), t!("turn.tip_yours", name = name)),
        Pick::Later => (t!("turn.pick_later").into(), t!("turn.tip_later", name = name)),
        Pick::YourCall => (t!("turn.pick_your_call").into(), t!("turn.tip_your_call", name = name)),
        Pick::DontKnow => (t!("turn.pick_dont_know").into(), t!("turn.tip_dont_know", name = name)),
    }
}

/// One ask: its number, its words (Markdown), the files it names, its box,
/// its picks and "메모 추가", and its note when open.
fn item(card: &Card, d: &Draw<'_>, n: usize, ask: &str, cx: &mut App) -> AnyElement {
    let p = d.p;
    let key = d.key;
    let want = card.wants.get(n).copied().unwrap_or(Want::Answer);
    let reply = card.replies.get(n).cloned().unwrap_or_default();
    let done = reply == Reply::Done;
    let set = !reply.is_empty();
    let (edge, fill, ink) = if done {
        (d.tones.added, Some(Rgba { a: 0.14, ..d.tones.added }), d.tones.added)
    } else if set {
        (Rgba { a: 0.8, ..p.accent }, Some(Rgba { a: 0.12, ..p.accent }), p.text)
    } else {
        (p.hair, None, p.text2)
    };
    let number = div()
        .flex_none()
        .size(px(20.))
        .mt(px(1.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(edge)
        .when_some(fill, |c, f| c.bg(f))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ink)
        .child(if done { "✓".to_owned() } else { (n + 1).to_string() });
    let shown = rules::mark_paths(ask, card.named.get(n).map(Vec::as_slice).unwrap_or_default());
    let quiet = done || matches!(reply, Reply::Yours | Reply::Later);
    let text = super::chat::markdown(p, ("turn-ask", hash(key, n)), &shown, crate::theme::UI, crate::theme::UI_PROSE_LINE, if quiet { p.muted } else { p.text }, &d.handle);
    let files = card.named.get(n).filter(|f| !f.is_empty()).map(|files| h_flex().flex_wrap().gap_1p5().children(files.iter().enumerate().map(|(k, f)| file_chip(p, f, hash(key, n << 8 | k)))));
    let answering = d.place != Place::Side;
    // The box, faded while a pick stands in for it (typing takes the pick back).
    let field = card.fields.get(n).cloned().flatten().filter(|_| answering && want != Want::Do).map(|field| {
        let picked = card.picks.contains_key(&n);
        boxed(p, &field, 14.).when(picked, |c| c.opacity(0.45))
    });
    let choices = answering.then(|| choices(card, d, n, want));
    // Words typed: a button on to the next step (one at a time only).
    let typed = matches!(card.replies.get(n), Some(Reply::Text(_)));
    let next = (answering && typed && card.asks.len() > 1 && card.current == Some(n)).then(|| {
        let (handle, id) = (d.handle.clone(), card.ticket);
        ui::button(p, ("turn-next", hash(key, n)), Kind::Primary, t!("turn.next")).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.turn_next(id, cx)).ok();
        })
    });
    let note = card.notes.get(n).cloned().flatten().filter(|_| answering).map(|note| boxed(p, &note, 13.));
    let _ = cx;
    h_flex()
        .w_full()
        .gap_2p5()
        .items_start()
        .child(number)
        .child(v_flex().flex_1().min_w_0().gap_2().child(text).children(files).children(field).children(choices).children(note).children(next.map(|b| h_flex().justify_end().child(b))))
        .into_any_element()
}

/// An ask's choices: its quick OK where it asks for one, its picks (one at
/// a time, the chosen one marked), and "메모 추가".
fn choices(card: &Card, d: &Draw<'_>, n: usize, want: Want) -> impl IntoElement {
    let p = d.p;
    let id = card.ticket;
    let name = card.member();
    let mut row: Vec<AnyElement> = Vec::new();
    if want == Want::Confirm {
        let handle = d.handle.clone();
        let label = t!("turn.ok");
        let mut ok = ui::button(p, ("turn-ok", hash(d.key, n)), Kind::Secondary, label).icon(Icon::Check).on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.turn_say(id, n, label, window, cx)).ok();
        });
        if let Some(focus) = card.keys.get(&(n, Slot::Ok)) {
            ok = ok.focus(focus);
        }
        row.push(ok.into_any_element());
    }
    for (k, pick) in rules::picks_for(want).iter().copied().enumerate() {
        let handle = d.handle.clone();
        let (label, about) = pick_words(pick, &name);
        let chosen = card.picks.get(&n) == Some(&pick);
        let mut b = ui::button(p, ("turn-pick", hash(d.key, n << 4 | k)), Kind::Secondary, label).selected(chosen).tip(about, None).on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.turn_pick(id, n, pick, window, cx)).ok();
        });
        if pick == Pick::Done {
            b = b.icon(Icon::Check);
        }
        if let Some(focus) = card.keys.get(&(n, Slot::Pick(k as u8))) {
            b = b.focus(focus);
        }
        row.push(b.into_any_element());
    }
    let open = card.notes.get(n).is_some_and(Option::is_some);
    let handle = d.handle.clone();
    let words = if open { t!("turn.note_remove") } else { t!("turn.note_add") };
    let mut memo = ui::button(p, ("turn-note", hash(d.key, n)), Kind::Ghost, words)
        .icon(if open { Icon::X } else { Icon::Plus })
        .tip(t!("turn.tip_note", name = name), None)
        .on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.turn_note(id, n, window, cx)).ok();
        });
    if let Some(focus) = card.keys.get(&(n, Slot::Note)) {
        memo = memo.focus(focus);
    }
    h_flex().w_full().flex_wrap().gap_1p5().items_center().children(row).child(div().flex_1()).child(memo)
}

fn hash(key: u64, n: usize) -> u64 {
    render::hash_of((key, n))
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

/// Who waits — and, once something is set, what stays open — and the
/// buttons: to the Inbox (from the conversation) and send.
fn foot(card: &Card, d: &Draw<'_>) -> impl IntoElement {
    let p = d.p;
    let id = card.ticket;
    let send = {
        let handle = d.handle.clone();
        let mut b = ui::button(p, ("turn-send", d.key), Kind::Primary, t!("turn.send")).key("⌘↵").disabled(!card.can_send).on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.turn_send(id, window, cx)).ok();
        });
        if let Some(focus) = card.keys.get(&(CARD, Slot::Send)) {
            b = b.focus(focus);
        }
        b
    };
    let board = (d.place == Place::Conversation).then(|| {
        let handle = d.handle.clone();
        ui::button(p, ("turn-board", d.key), Kind::Ghost, t!("turn.open_board")).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.open_ticket(id, cx)).ok();
        })
    });
    let waiting = card.who.as_ref().map(|(name, _)| t!("turn.waiting", name = name));
    let left = card.left();
    let partial = (card.can_send && left > 0).then(|| div().text_color(d.tones.warning).child(t!("turn.left", count = left)));
    // Narrow (a panel beside it), the buttons go on a line of their own
    // rather than squeezing the words a letter to a line.
    h_flex()
        .w_full()
        .flex_wrap()
        .pt_3()
        .gap_3()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .child(v_flex().flex_1().min_w(px(crate::ui::WORDS_AT_LEAST)).gap_0p5().text_size(px(12.)).line_height(px(17.)).text_color(p.muted).children(waiting).children(partial).child(div().opacity(0.8).child(t!("turn.keys"))))
        .child(h_flex().flex_none().ml_auto().gap_3().children(board).child(send))
}

/// What a reply says on screen, read back from what was sent.
fn reply_words(reply: &Reply) -> String {
    match reply {
        Reply::Text(text) => text.clone(),
        Reply::Done => t!("turn.done").into(),
        Reply::Yours => t!("turn.sent_yours").into(),
        Reply::Later => t!("turn.sent_later").into(),
        Reply::YourCall => t!("turn.pick_your_call").into(),
        Reply::DontKnow => t!("turn.pick_dont_know").into(),
        Reply::Empty => t!("turn.sent_not_yet").into(),
    }
}

/// What was sent, as the person reads it back: each ask with what came back
/// in their language, its note, their words last — or the message as it is.
fn sent_body(p: Palette, key: u64, text: &str) -> AnyElement {
    let Some(read) = asks::read(text) else {
        return div().text_size(px(13.)).line_height(px(20.)).text_color(p.text2).child(gpui_kit::base::SelectableText::new(("turn-sent-text", key), text.to_owned())).into_any_element();
    };
    let lines = read.items.iter().map(|item| {
        let open = item.reply.stays_open();
        let said = reply_words(&item.reply);
        v_flex()
            .gap_0p5()
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(div().flex_none().w(px(16.)).text_color(p.muted).child(format!("{}.", item.n)))
                    .child(div().min_w_0().flex_1().child(div().text_color(p.muted).child(item.ask.clone())).child(div().text_color(if open { p.muted } else { p.text }).child(format!("→ {said}")))),
            )
            .children((!item.note.is_empty()).then(|| div().pl(px(24.)).text_color(p.text2).child(t!("turn.sent_note", note = item.note))))
    });
    v_flex()
        .gap_1p5()
        .text_size(px(13.))
        .line_height(px(19.))
        .children(lines)
        .children((!read.free.is_empty()).then(|| div().pt_1().text_color(p.text).child(t!("turn.sent_free", text = read.free))))
        .into_any_element()
}

/// Answered (or on its way, or no longer asked): one line, and what was sent
/// under it when opened.
fn settled(card: &Card, d: Draw<'_>, cx: &mut App) -> AnyElement {
    let p = d.p;
    let key = d.key;
    let ago = card.answered.as_ref().and_then(|a| a.at).map(|at| crate::activity::ago(card.now.saturating_sub(at)));
    let count = card.answered.as_ref().and_then(rules::answered_count);
    let open = card.answered.as_ref().map_or(0, rules::open_count);
    let (mark, words): (AnyElement, String) = match card.phase {
        Phase::Sending if !cx.reduce_motion() => (crate::motion::spinner(12., p.muted), t!("turn.sending").to_owned()),
        Phase::Sending => (div().child("◌").into_any_element(), t!("turn.sending").to_owned()),
        Phase::Closed => (icon(Icon::XCircle, 13., p.muted).into_any_element(), t!("turn.closed").to_owned()),
        Phase::Answered { here } => {
            let head = if here { t!("turn.answered") } else { t!("turn.answered_elsewhere") };
            let mut parts = vec![head.to_owned()];
            parts.extend(count.map(|n| t!("turn.count", count = n)));
            parts.extend((open > 0).then(|| t!("turn.open_left", count = open)));
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
            v_flex().gap_1().child(div().text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("turn.sent_heading"))).child(sent_body(p, key, &text)).into_any_element()
        };
        crate::motion::Reveal::new(("turn-sent-body", key), progress, div().opacity(progress).ml(px(21.)).mt_1().px_3().py_2().rounded(px(R_SM)).bg(p.code_bg).child(inner).into_any_element())
    });
    crate::motion::fade(v_flex().w_full().child(line).children(body), ("turn-settled", render::hash_of((key, format!("{:?}", card.phase)))), crate::motion::BASE).into_any_element()
}

/// What a ticket left open, under its head on the peek: "남은 것 N" and the asks.
pub fn remaining(p: Palette, t: &Ticket) -> Option<AnyElement> {
    if t.remaining.is_empty() || t.status == TicketStatus::Waiting || t.status.is_closed() {
        return None;
    }
    Some(
        v_flex()
            .gap_0p5()
            .text_size(px(12.5))
            .line_height(px(18.))
            .text_color(p.muted)
            .child(h_flex().gap_1p5().items_center().child(icon(Icon::Clock, 12., p.muted)).child(div().font_weight(FontWeight::SEMIBOLD).child(t!("turn.open_left", count = t.remaining.len()))))
            .children(t.remaining.iter().map(|ask| div().pl(px(18.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(ask.lines().next().unwrap_or_default().to_owned())))
            .into_any_element(),
    )
}

// ── the shell's side ──

impl Shell {
    /// Boxes and keys for every ask of every waiting ticket — an answer box
    /// for each that wants words, the words to the member for each card; a
    /// ticket no longer waiting lets its go.
    pub fn turn_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut wanted: Vec<(TicketId, usize)> = Vec::new();
        let mut cards: Vec<(TicketId, String)> = Vec::new();
        let mut keys: Vec<(TicketId, usize, Slot)> = Vec::new();
        for t in self.model.tickets.iter().filter(|t| t.status == TicketStatus::Waiting) {
            let asks = t.asks();
            if asks.is_empty() {
                continue;
            }
            cards.push((t.id, t.assignee.clone().or(t.owner.clone()).unwrap_or_default()));
            keys.push((t.id, CARD, Slot::Send));
            for (n, ask) in asks.iter().enumerate() {
                let want = rules::want_of(ask);
                if want != Want::Do {
                    wanted.push((t.id, n));
                }
                if want == Want::Confirm {
                    keys.push((t.id, n, Slot::Ok));
                }
                keys.extend((0..rules::picks_for(want).len()).map(|k| (t.id, n, Slot::Pick(k as u8))));
                keys.push((t.id, n, Slot::Note));
            }
        }
        let waiting = |id: &TicketId| cards.iter().any(|(t, _)| t == id);
        self.turn.fields.retain(|k, _| wanted.contains(k));
        self.turn.notes.retain(|(id, _), _| waiting(id));
        self.turn.free.retain(|id, _| waiting(id));
        self.turn.picks.retain(|id, _| waiting(id));
        self.turn.keys.retain(|k, _| keys.contains(k));
        self.turn.subscriptions.retain(|(id, n, what), _| match *what {
            ANSWER => wanted.contains(&(*id, *n)),
            _ => waiting(id),
        });
        for k in keys {
            self.turn.keys.entry(k).or_insert_with(|| cx.focus_handle().tab_stop(true));
        }
        for k in wanted {
            if self.turn.fields.contains_key(&k) {
                continue;
            }
            let field = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 6).submit_on_enter(false).placeholder(t!("turn.answer_placeholder")));
            let (id, n) = k;
            let subscription = cx.subscribe_in(&field, window, move |this, field, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { secondary: true, .. } => this.turn_send(id, window, cx),
                InputEvent::Change => {
                    // Words typed answer it: a pick standing in for them goes,
                    // and the step stays open while they are typed.
                    this.turn.open.insert(id, n);
                    if !field.read(cx).value().trim().is_empty()
                        && let Some(picks) = this.turn.picks.get_mut(&id)
                    {
                        picks.remove(&n);
                    }
                    // The box may have grown a line, and the button may wake.
                    this.chat_remeasure_turn(id, cx);
                    cx.notify();
                }
                _ => {}
            });
            self.turn.fields.insert(k, field);
            self.turn.subscriptions.insert((id, n, ANSWER), subscription);
        }
        for (id, name) in cards {
            if self.turn.free.contains_key(&id) {
                continue;
            }
            let field = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 6).submit_on_enter(false).placeholder(t!("turn.free_placeholder", name = name)));
            let subscription = self.turn_watch(id, &field, window, cx);
            self.turn.free.insert(id, field);
            self.turn.subscriptions.insert((id, CARD, FREE), subscription);
        }
    }

    /// A note's box or the words to the member: ⌘↵ sends, a change redraws.
    fn turn_watch(&mut self, id: TicketId, field: &Entity<TextareaState>, window: &mut Window, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe_in(field, window, move |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { secondary: true, .. } => this.turn_send(id, window, cx),
            InputEvent::Change => {
                this.chat_remeasure_turn(id, cx);
                cx.notify();
            }
            _ => {}
        })
    }

    /// Everything ticket `id`'s card holds now, as it would be sent.
    pub fn turn_answers(&self, id: TicketId, asks: &[String], cx: &App) -> asks::Answers {
        let wants: Vec<Want> = asks.iter().map(|a| rules::want_of(a)).collect();
        let read = |f: Option<&Entity<TextareaState>>| f.map(|f| f.read(cx).value().to_string()).unwrap_or_default();
        let typed: Vec<String> = (0..asks.len()).map(|n| read(self.turn.fields.get(&(id, n)))).collect();
        let notes: Vec<String> = (0..asks.len()).map(|n| read(self.turn.notes.get(&(id, n)))).collect();
        let picks = self.turn.picks.get(&id).cloned().unwrap_or_default();
        rules::answers(&wants, &typed, &picks, &notes, &read(self.turn.free.get(&id)))
    }

    /// What a waiting ticket's card would send, `more` words of the person's
    /// after their own: every ask, numbered.
    pub fn turn_message(&self, t: &Ticket, more: &str, cx: &App) -> String {
        let asks = t.asks();
        let mut answers = self.turn_answers(t.id, &asks, cx);
        answers.free = [answers.free.trim(), more.trim()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
        asks::answer_message(&asks, &answers)
    }

    /// "답 보내기": what is set to the member in one message; the ticket goes
    /// back to it, and the card says it was answered.
    pub fn turn_send(&mut self, id: TicketId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.model.tickets.iter().find(|t| t.id == id && t.status == TicketStatus::Waiting).cloned() else { return };
        let asks = t.asks();
        let answers = self.turn_answers(id, &asks, cx);
        if !rules::can_send(&answers) || !self.can_act() || self.model.answers.get(&id).is_some_and(|a| a.here && !a.settled) {
            return;
        }
        let note = asks::answer_message(&asks, &answers);
        self.send_command(Command::Resume { squad: self.model.squad.clone(), ticket: id, note: note.clone() });
        self.model.answer_sent(id, note);
        self.model.act_on(id);
        self.turn.picks.remove(&id);
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
            if let Some(picks) = self.turn.picks.get_mut(&id) {
                picks.remove(&n);
            }
            self.chat_remeasure_turn(id, cx);
            cx.notify();
        }
    }

    /// Step `n` opened, to answer it again or ahead of the others.
    pub fn turn_open(&mut self, id: TicketId, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.turn.open.insert(id, n);
        if let Some(field) = self.turn.fields.get(&(id, n)).cloned() {
            field.update(cx, |s, cx| s.focus(window, cx));
        }
        self.chat_remeasure_turn(id, cx);
        cx.notify();
    }

    /// The step answered in words is through: on to the next one open.
    pub fn turn_next(&mut self, id: TicketId, cx: &mut Context<Self>) {
        self.turn.open.remove(&id);
        self.chat_remeasure_turn(id, cx);
        cx.notify();
    }

    /// A pick on ask `n`, or taken back when it was the one picked.
    pub fn turn_pick(&mut self, id: TicketId, n: usize, pick: Pick, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        rules::toggle(self.turn.picks.entry(id).or_default(), n, pick);
        // Picked: on to the next step not yet answered.
        if self.turn.picks.get(&id).is_some_and(|p| p.contains_key(&n)) {
            self.turn.open.remove(&id);
        }
        self.chat_remeasure_turn(id, cx);
        self.chat_sync(cx);
        cx.notify();
    }

    /// "메모 추가": a box for a word on ask `n`, its keys at once; "메모 빼기"
    /// takes it away, words and all.
    pub fn turn_note(&mut self, id: TicketId, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.turn.notes.remove(&(id, n)).is_some() {
            self.turn.subscriptions.remove(&(id, n, NOTE));
        } else {
            let field = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 4).submit_on_enter(false).placeholder(t!("turn.note_placeholder")));
            let subscription = self.turn_watch(id, &field, window, cx);
            field.update(cx, |s, cx| s.focus(window, cx));
            self.turn.notes.insert((id, n), field);
            self.turn.subscriptions.insert((id, n, NOTE), subscription);
        }
        self.chat_remeasure_turn(id, cx);
        self.chat_sync(cx);
        cx.notify();
    }

    /// Words to the member into the card's foot box.
    pub fn turn_free(&mut self, id: TicketId, words: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(field) = self.turn.free.get(&id).cloned() {
            field.update(cx, |s, cx| {
                s.set_value(words.to_owned(), window, cx);
                s.focus(window, cx);
            });
            cx.notify();
        }
    }

    /// Words into ask `n`'s note, opening it first.
    pub fn turn_note_say(&mut self, id: TicketId, n: usize, words: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.turn.notes.contains_key(&(id, n)) {
            self.turn_note(id, n, window, cx);
        }
        if let Some(field) = self.turn.notes.get(&(id, n)).cloned() {
            field.update(cx, |s, cx| s.set_value(words.to_owned(), window, cx));
            cx.notify();
        }
    }

    /// The first box (or choice) of a waiting ticket's card takes the keys (R on the board).
    pub fn turn_focus(&mut self, id: TicketId, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(card) = Card::of(self, id, cx) else { return false };
        match card.tab_order(cx).first() {
            Some(first) => {
                window.focus(first, cx);
                cx.notify();
                true
            }
            None => false,
        }
    }
}
