//! The conversation with a member (T-54 §3.2–3.3): your messages, the member's
//! replies as Markdown, tool steps gathered and folded — each with how it
//! ended, a command's output and an edit's change — notes, errors and
//! commits, in a virtual list that follows the end while you are there.
//!
//! Streaming draws on the daemon's tick only (see `main.rs`): each tick the
//! blocks are rebuilt, the list told which rows differ, and the window drawn
//! once. When the last piece has come nothing asks for another frame.
//!
//! How text is drawn — code, links, file chips, math, diagrams — is
//! `crate::render`'s; this file lays the rows out.

use equipe_core::t;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use equipe_core::protocol::SessionId;
use gpui_kit::base::SelectableText;
use gpui_kit::base::text::{RangeHighlight, TextView, TextViewState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::shimmer::ShimmerText;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{approvals, composer, header};
use crate::convo::{Block, RunRow, Status, ToolLine, relative};
use crate::diffs::Kind;
use crate::render::view::{self, Look};
use crate::render::{self, Tones, hash_of, plugins, tools};
use crate::state::{Model, Presence};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{BODY, CONTROL, ICON_SM, MONO, Palette, R_MD, R_SM, SMALL, look};
use crate::tips::tip;

/// Where text starts inside the column: the cards' and the composer's inner
/// padding plus their border. Replies, banners and card text all start here;
/// cards and your own messages have their edge at the column's.
const TEXT_INSET: f32 = 13.;
/// How long after it arrives a row is still arriving (its entrance plays).
const ARRIVING: std::time::Duration = std::time::Duration::from_millis(400);
use crate::{Screen, Shell};

/// Reading width: about 48 Korean characters.
const COLUMN: f32 = 720.;
/// Text views kept for rows scrolled out of sight, so coming back to them
/// doesn't parse their Markdown again.
const VIEWS_KEPT: usize = 96;
/// A message of yours longer than this many lines shows its start.
const USER_LINES: usize = 12;

/// What a row has open, beside its code fences (numbered from 0).
const PART_MORE: u32 = 1 << 20;
const PART_THOUGHT: u32 = PART_MORE + 1;
const PART_OUTPUT: u32 = 1 << 21;
const PART_DIFF: u32 = 1 << 22;

pub struct Chat {
    scroller: Entity<MessageScrollerState>,
    blocks: Rc<Vec<Block>>,
    /// Whose conversation `blocks` are, and the ticket it shows the work of.
    shown: Option<(SessionId, Option<u32>)>,
    seen_revision: u64,
    /// Tool runs shown whole rather than folded (⌘⇧O).
    pub expanded: bool,
    shown_expanded: bool,
    /// Tells one conversation's rows from another's.
    serial: u64,
    /// Text views by what they show, kept across scrolling.
    views: Rc<RefCell<Views>>,
    /// What the person opened in the rows: `(row, part)`.
    opened: Rc<HashSet<(u64, u32)>>,
    /// An image looked at closer, over the conversation.
    pub lightbox: Option<ImageSource>,
    pub find: Find,
    /// Rows that came in while the conversation was on screen, and when: they
    /// rise into place (rows scrolled back into view don't).
    arrived: Rc<HashMap<u64, std::time::Instant>>,
}

/// ⌘F: words looked for in the conversation, the rows they are in, and the
/// one shown.
#[derive(Default)]
pub struct Find {
    pub input: Option<Entity<InputState>>,
    pub open: bool,
    pub query: String,
    /// Each match: its row, and which match in that row it is.
    pub hits: Vec<(usize, usize)>,
    pub at: usize,
    _subscription: Option<Subscription>,
}

#[derive(Default)]
struct Views {
    map: HashMap<u64, (Entity<TextViewState>, u64)>,
    tick: u64,
}

impl Views {
    /// The text view for `key`, showing `text`.
    fn get(&mut self, key: u64, text: &str, cx: &mut App) -> Entity<TextViewState> {
        self.tick += 1;
        let tick = self.tick;
        if let Some((state, used)) = self.map.get_mut(&key) {
            *used = tick;
            let state = state.clone();
            state.update(cx, |s, cx| s.set_text(text, cx));
            return state;
        }
        let state = cx.new(|cx| TextViewState::markdown(text, cx));
        self.map.insert(key, (state.clone(), tick));
        if self.map.len() > VIEWS_KEPT
            && let Some(oldest) = self.map.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| *k)
        {
            self.map.remove(&oldest);
        }
        state
    }
}

/// Which text view of a row: its reply or note, your message, a thought.
fn view_key(row: u64, kind: u64) -> u64 {
    row << 2 | kind
}

impl Chat {
    pub fn new(cx: &mut Context<Shell>) -> Self {
        // Elements drawn inside text views reach the shell through this.
        cx.set_global(render::ShellRef(cx.entity().downgrade()));
        Self {
            scroller: cx.new(|cx| MessageScrollerState::new(0, cx)),
            blocks: Rc::new(Vec::new()),
            shown: None,
            seen_revision: 0,
            expanded: false,
            shown_expanded: false,
            serial: 0,
            views: Rc::default(),
            opened: Rc::default(),
            lightbox: None,
            find: Find::default(),
            arrived: Rc::default(),
        }
    }

    /// Bring the list up to what the model shows. Cheap when nothing moved.
    pub fn sync(&mut self, model: &Model, cx: &mut App) {
        let key = model.viewed().map(|id| (id, model.opened));
        let moved = key != self.shown;
        if !moved && model.revision == self.seen_revision && self.expanded == self.shown_expanded {
            return;
        }
        let new = model.convo().map(|c| c.blocks()).unwrap_or_default();
        let old = std::mem::replace(&mut self.blocks, Rc::new(new));
        self.seen_revision = model.revision;
        let folds_changed = self.expanded != self.shown_expanded;
        self.shown_expanded = self.expanded;
        let n = self.blocks.len();
        if moved {
            self.shown = key;
            self.serial += 1;
            self.opened = Rc::default();
            self.views.borrow_mut().map.clear();
            self.scroller.update(cx, |s, cx| s.reset(n, cx));
            if self.find.open {
                self.search(cx);
            }
            return;
        }
        // Rows from the first that differs are new or changed (a streaming
        // reply is the last row growing); the list measures only those again.
        let same = old.iter().zip(self.blocks.iter()).take_while(|(a, b)| a == b).count();
        if n > old.len() {
            let now = std::time::Instant::now();
            let mut arrived: HashMap<u64, std::time::Instant> = self.arrived.iter().filter(|(_, at)| now.duration_since(**at) < ARRIVING).map(|(k, v)| (*k, *v)).collect();
            for ix in old.len()..n {
                arrived.insert(self.serial << 24 | ix as u64, now);
            }
            self.arrived = Rc::new(arrived);
        }
        self.scroller.update(cx, |s, cx| {
            if old.len() != n {
                s.splice(same..old.len(), n - same, cx);
            } else if same < n {
                s.remeasure_items(same..n, cx);
            }
            if folds_changed {
                s.remeasure(cx);
            }
            cx.notify();
        });
    }

    /// Back to the end, following it again.
    pub fn follow_tail(&self, cx: &mut App) {
        self.scroller.update(cx, |s, cx| s.scroll_to_end(cx));
    }

    /// Bring row `ix` to the top (the snapshot script's).
    #[cfg_attr(not(feature = "snapshot"), allow(dead_code))]
    pub fn scroll_to(&self, ix: usize, cx: &mut App) {
        self.scroller.update(cx, |s, cx| {
            s.scroll_to_item(ix, cx);
        });
    }

    /// Open or close a part of a row, and measure the row again.
    fn toggle(&mut self, row: u64, part: u32, cx: &mut App) {
        let mut opened = (*self.opened).clone();
        if !opened.remove(&(row, part)) {
            opened.insert((row, part));
        }
        self.opened = Rc::new(opened);
        self.remeasure_row(row, cx);
    }

    fn remeasure_row(&self, row: u64, cx: &mut App) {
        if row >> 24 != self.serial {
            return;
        }
        let ix = (row & 0xFF_FFFF) as usize;
        self.scroller.update(cx, |s, cx| {
            s.remeasure_items(ix..ix + 1, cx);
        });
    }

    // ── find ──

    /// Look for the words in the box again: every row they are in.
    fn search(&mut self, cx: &mut App) {
        let query = self.find.input.as_ref().map(|i| i.read(cx).value().to_string()).unwrap_or_default();
        self.find.query = query.trim().to_owned();
        self.find.hits = find_hits(&self.blocks, &self.find.query);
        // The match nearest the end: the conversation is read from there.
        self.find.at = self.find.hits.len().saturating_sub(1);
        self.show_hit(cx);
    }

    fn step_hit(&mut self, back: bool, cx: &mut App) {
        let n = self.find.hits.len();
        if n == 0 {
            return;
        }
        self.find.at = if back { (self.find.at + n - 1) % n } else { (self.find.at + 1) % n };
        self.show_hit(cx);
    }

    /// Scroll to the match shown and mark every match in sight.
    fn show_hit(&mut self, cx: &mut App) {
        if let Some(&(ix, _)) = self.find.hits.get(self.find.at) {
            self.scroller.update(cx, |s, cx| {
                s.scroll_to_item(ix, cx);
            });
        }
        self.paint_hits(cx);
    }

    /// Highlight the matches in every text view that has been drawn; the
    /// current one stronger, and brought into view.
    fn paint_hits(&self, cx: &mut App) {
        let query = self.find.query.to_lowercase();
        let current = self.find.hits.get(self.find.at).copied();
        let views: Vec<(u64, Entity<TextViewState>)> = self.views.borrow().map.iter().map(|(k, (s, _))| (*k, s.clone())).collect();
        let tones_soft: Hsla = rgba(0xE8BE7855).into();
        let tones_strong: Hsla = rgba(0xE8BE78C0).into();
        for (key, state) in views {
            let row = key >> 2;
            let ix = (row & 0xFF_FFFF) as usize;
            state.update(cx, |s, cx| {
                if query.is_empty() || !self.find.open || row >> 24 != self.serial {
                    s.clear_range_highlights(cx);
                    return;
                }
                let text = s.rendered_text();
                let ranges = occurrences(text.as_str(), &query);
                let mut marks = Vec::new();
                for (n, range) in ranges.iter().enumerate() {
                    let strong = current == Some((ix, n));
                    marks.push(RangeHighlight::new(range.clone(), if strong { tones_strong } else { tones_soft }));
                    if strong {
                        let _ = s.reveal_range(range.clone(), cx);
                    }
                }
                let _ = s.set_range_highlights(marks, cx);
            });
        }
    }
}

/// Where `query` (lower case) is in `text`, by byte ranges of `text`.
fn occurrences(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let lower = text.to_lowercase();
    // Lower-casing can change lengths; then there is no safe mapping back.
    if lower.len() != text.len() {
        return Vec::new();
    }
    lower.match_indices(query).map(|(at, m)| at..at + m.len()).collect()
}

/// Each match of `query` in the conversation: its row and its number there.
pub fn find_hits(blocks: &[Block], query: &str) -> Vec<(usize, usize)> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (ix, block) in blocks.iter().enumerate() {
        if let Some(text) = block.searchable() {
            let n = text.to_lowercase().matches(&query).count();
            hits.extend((0..n).map(|k| (ix, k)));
        }
    }
    hits
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let model = &shell.model;
    let body = match model.selected_member() {
        None => empty(p, t!("chat.no_member_title"), t!("chat.no_member_text")).into_any_element(),
        Some(member) if shell.chat.blocks.is_empty() => {
            let (title, text) = hello(model, member.role.as_str(), member.name.as_str(), member.presence);
            empty(p, title, text).into_any_element()
        }
        Some(member) => {
            let (mark, hue) = look(&member.role, &member.name);
            let look = Who { mark, name: member.name.clone(), color: p.member(hue) };
            conversation(shell, look, cx).into_any_element()
        }
    };
    // A ticket open: its number, title and where it stands, over its work.
    let banner = model.opened.and_then(|id| model.tickets.iter().find(|t| t.id == id)).map(|t| {
        let hint = match t.status {
            equipe_core::protocol::TicketStatus::Review => t!("chat.hint_review", id = t.id),
            equipe_core::protocol::TicketStatus::Waiting => t!("chat.hint_waiting", id = t.id),
            _ => t!("chat.hint_working").to_owned(),
        };
        v_flex()
            .w_full()
            .max_w(px(COLUMN))
            .px(px(16. + TEXT_INSET))
            .pt_3()
            .gap_0p5()
            .child(
                h_flex()
                    .gap_2()
                    .items_baseline()
                    .text_size(px(15.))
                    .line_height(px(22.))
                    .child(div().flex_none().font_family(MONO).text_size(px(13.)).text_color(p.accent).child(format!("T-{}", t.id)))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().font_weight(FontWeight::SEMIBOLD).child(t.title.clone())),
            )
            .child(div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(hint))
    });
    v_flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .relative()
        .children(banner)
        .children(find_bar(shell, cx))
        .child(body)
        .child(
            div()
                .w_full()
                .max_w(px(COLUMN))
                .px_4()
                .child(v_flex().children(approvals::for_you(shell, cx)).children(approvals::approval(shell, cx)))
                .child(composer::render(shell, cx)),
        )
        .children(lightbox(shell, cx))
}

/// What a blank conversation says. The PM's names the thread it is.
fn hello(model: &Model, role: &str, name: &str, presence: Presence) -> (&'static str, String) {
    if presence == Presence::Closed {
        return (t!("chat.member_left_title"), t!("chat.member_left_text", name = name));
    }
    match (role, model.focus.as_deref()) {
        ("pm", Some(project)) => (t!("chat.pm_title"), t!("chat.pm_project_text", project = project)),
        ("pm", None) => (t!("chat.pm_build_title"), t!("chat.pm_text").into()),
        _ => (t!("chat.member_title"), t!("chat.member_text").into()),
    }
}

fn empty(p: Palette, title: &'static str, text: impl Into<SharedString>) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .w_full()
        .max_w(px(480.))
        .items_center()
        .justify_center()
        .gap_3()
        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title))
        .child(div().text_size(px(BODY)).line_height(px(24.)).text_color(p.text2).text_center().child(text.into()))
}

#[derive(Clone)]
struct Who {
    mark: &'static str,
    name: String,
    color: Rgba,
}

/// What every row of one frame draws with.
struct Rows {
    p: Palette,
    tones: Tones,
    blocks: Rc<Vec<Block>>,
    who: Who,
    serial: u64,
    expanded: bool,
    opened: Rc<HashSet<(u64, u32)>>,
    views: Rc<RefCell<Views>>,
    handle: Handle,
    /// The last reply, when the turn is over: it can be asked again.
    retry_at: Option<usize>,
    base: std::path::PathBuf,
    arrived: Rc<HashMap<u64, std::time::Instant>>,
}

fn conversation(shell: &Shell, who: Who, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let blocks = shell.chat.blocks.clone();
    let working = blocks.iter().any(|b| matches!(b, Block::Reply { live: true, .. } | Block::Thinking(_)))
        || shell.model.selected_member().is_some_and(|m| m.presence == Presence::Working);
    let retry_at = (!working).then(|| blocks.iter().rposition(|b| matches!(b, Block::Reply { .. }))).flatten().filter(|&ix| blocks[..ix].iter().any(|b| matches!(b, Block::User(_))));
    let rows = Rc::new(Rows {
        p,
        tones: render::tones(&p),
        blocks,
        who,
        serial: shell.chat.serial,
        expanded: shell.chat.expanded,
        opened: shell.chat.opened.clone(),
        views: shell.chat.views.clone(),
        handle: cx.entity().downgrade(),
        retry_at,
        base: render::base_dir(shell),
        arrived: shell.chat.arrived.clone(),
    });
    div().flex_1().min_h_0().w_full().max_w(px(COLUMN)).px_4().child(
        MessageScroller::new("conversation", shell.chat.scroller.clone(), move |ix, window, cx| row(&rows, ix, window, cx))
            // It fades and slides up into place, bottom-centre over the composer.
            .with_jump_button_transition(motion::BASE)
            .with_row_style(StyleRefinement::default().pb(px(8.)).pl(px(TEXT_INSET)).pr(px(TEXT_INSET)))
            .with_jump_button_label(t!("chat.jump_to_bottom"))
            // A pill in the app's colours, with its words beside the arrow: on
            // its own the arrow sat white on white.
            .with_jump_button_style(
                StyleRefinement::default()
                    .h(px(32.))
                    .bg(Hsla::from(p.popover()))
                    .text_color(Hsla::from(p.text))
                    .border_color(Hsla::from(p.hair))
                    .shadow(motion::float_shadow(0.8)),
            )
            .with_jump_button_renderer(|button| button.label(t!("chat.jump_to_bottom")).px_3()),
    )
}

pub type Handle = WeakEntity<Shell>;

/// A row: new ones rise into place.
fn row(rows: &Rows, ix: usize, window: &mut Window, cx: &mut App) -> AnyElement {
    let key = rows.serial << 24 | ix as u64;
    let body = row_body(rows, ix, window, cx);
    match rows.arrived.get(&key) {
        Some(at) if at.elapsed() < ARRIVING => motion::rise(div().child(body), ("row-in", key)).into_any_element(),
        _ => body,
    }
}

fn row_body(rows: &Rows, ix: usize, window: &mut Window, cx: &mut App) -> AnyElement {
    let p = rows.p;
    let key = rows.serial << 24 | ix as u64;
    let opened = |part: u32| rows.opened.contains(&(key, part));
    match &rows.blocks[ix] {
        Block::User(text) => user_row(rows, key, text, opened(PART_MORE), cx),
        Block::Reply { text, first, live } => {
            let (markdown, full) = view::prepare(text, *live, &|n| opened(n as u32));
            let state = rows.views.borrow_mut().get(view_key(key, 0), &markdown, cx);
            let look = Look { size: BODY, line: 24., color: p.text, live: *live };
            let body = view::configure(TextView::new(&state), p, look, key, Arc::new(full));
            let group: SharedString = format!("reply-{key}").into();
            let mut tools: Vec<AnyElement> = Vec::new();
            if !live {
                let raw = text.clone();
                tools.push(tool_button(p, ("reply-copy", key), Icon::Copy, t!("chat.copy_reply"), move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(raw.clone()))).into_any_element());
            }
            if rows.retry_at == Some(ix) {
                let handle = rows.handle.clone();
                tools.push(
                    tool_button(p, ("reply-retry", key), Icon::Retry, t!("chat.retry_reply"), move |window, cx| {
                        handle.update(cx, |shell, cx| shell.retry_last(window, cx)).ok();
                    })
                    .into_any_element(),
                );
            }
            v_flex()
                .group(group.clone())
                .relative()
                .gap_1()
                .when(*first, |col| {
                    col.mt_3().child(
                        h_flex()
                            .gap_2()
                            .text_size(px(SMALL))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rows.who.color)
                            .child(rows.who.mark)
                            .child(rows.who.name.clone()),
                    )
                })
                .child(div().text_size(px(BODY)).line_height(px(24.)).text_color(p.text).child(body))
                .when(!tools.is_empty(), |col| col.child(toolbar(p, group, *first, tools)))
                .into_any_element()
        }
        Block::Steps(lines) => steps(rows, key, lines, window, cx),
        Block::Note(text) => {
            // A ticket's heading (`── T-12 …`) or a turn's end (`─ worked …`, in any language).
            let chapter = text.starts_with('─');
            let (markdown, full) = view::prepare(text, false, &|_| true);
            let state = rows.views.borrow_mut().get(view_key(key, 1), &markdown, cx);
            let look = Look { size: SMALL, line: 20., color: p.muted, live: false };
            div()
                .when(chapter, |d| d.mt_3())
                .text_size(px(SMALL))
                .line_height(px(20.))
                .text_color(p.muted)
                .child(view::configure(TextView::new(&state), p, look, key, Arc::new(full)))
                .into_any_element()
        }
        Block::Thought { secs, text, .. } => {
            let open = opened(PART_THOUGHT);
            let handle = rows.handle.clone();
            let head = h_flex()
                .id(("thought", key))
                .gap_2()
                .text_size(px(SMALL))
                .text_color(p.muted)
                .cursor_pointer()
                .hover(move |s| s.text_color(p.text2))
                .child(div().w(px(12.)).child(if open { "▾" } else { "▸" }))
                .child(t!("chat.thought_for", secs = secs))
                .on_click(move |_, _, cx| {
                    handle.update(cx, |shell, cx| shell.chat_toggle(key, PART_THOUGHT, cx)).ok();
                });
            let body = open.then(|| {
                let state = rows.views.borrow_mut().get(view_key(key, 3), text, cx);
                let look = Look { size: SMALL, line: 20., color: p.muted, live: false };
                div()
                    .ml(px(5.))
                    .pl(px(14.))
                    .border_l_1()
                    .border_color(p.hair)
                    .text_size(px(SMALL))
                    .line_height(px(20.))
                    .text_color(p.muted)
                    .child(view::configure(TextView::new(&state), p, look, key, Arc::default()))
            });
            v_flex().gap_1().child(head).children(body).into_any_element()
        }
        Block::Thinking(latest) => h_flex()
            .gap_2()
            .text_size(px(SMALL))
            .text_color(p.muted)
            .child(div().w(px(12.)).flex_none().child("◇"))
            .child(ShimmerText::new(t!("chat.thinking")).id(("thinking", key)).highlight_color(Hsla::from(p.text)))
            .child(div().min_w_0().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(latest.clone()))
            .into_any_element(),
        Block::Error { message, next } => v_flex()
            .gap_1()
            .text_size(px(BODY))
            .line_height(px(24.))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).text_color(rows.tones.danger).child("✗"))
                    .child(div().min_w_0().font_weight(FontWeight::SEMIBOLD).child(SelectableText::new(("error", key), message.clone()))),
            )
            .children(next.clone().map(|next| {
                h_flex().gap_2().pl_5().text_size(px(SMALL)).child(div().text_color(p.muted).child("→")).child(div().font_family(MONO).font_weight(FontWeight::SEMIBOLD).child(SelectableText::new(("error-next", key), next)))
            }))
            .into_any_element(),
        Block::Landed { branch, stat } => v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .text_size(px(SMALL))
                    .child(div().text_color(p.accent).child("✓"))
                    .child(t!("chat.committed"))
                    .child(div().font_family(MONO).font_weight(FontWeight::SEMIBOLD).child(SelectableText::new(("branch", key), branch.clone()))),
            )
            .child(v_flex().pl_5().font_family(MONO).text_size(px(12.5)).line_height(px(20.)).text_color(p.muted).children(stat.lines().map(|l| div().child(l.trim().to_owned()))))
            .into_any_element(),
        Block::Memo(text) => v_flex()
            .text_color(p.muted)
            .child(h_flex().gap_3().text_size(px(BODY)).line_height(px(24.)).child(div().flex_none().child("›")).child(div().min_w_0().child(SelectableText::new(("memo", key), text.clone()))))
            .child(div().pl_5().text_size(px(SMALL)).child(t!("chat.memo_note")))
            .into_any_element(),
        Block::Steering(text) => v_flex()
            .text_color(p.muted)
            .child(h_flex().gap_3().text_size(px(BODY)).line_height(px(24.)).child(div().flex_none().text_color(p.accent).child("›")).child(div().min_w_0().child(SelectableText::new(("steering", key), text.clone()))))
            .child(div().pl_5().text_size(px(SMALL)).child(t!("chat.steering_note")))
            .into_any_element(),
    }
}

/// Your message: selectable, its start only when it is long.
fn user_row(rows: &Rows, key: u64, text: &str, open: bool, cx: &mut App) -> AnyElement {
    let p = rows.p;
    let long = text.lines().count() > USER_LINES || text.chars().count() > USER_LINES * 56;
    let state = rows.views.borrow_mut().get(view_key(key, 2), &escape_markdown(text), cx);
    let clamped = long && !open;
    let look = Look { size: BODY, line: 24., color: p.text, live: false };
    let body = TextView::new(&state).style(view::style(p, &look)).selectable(true).when(clamped, |v| v.max_lines(USER_LINES));
    let group: SharedString = format!("user-{key}").into();
    let raw = text.to_owned();
    let copy = tool_button(p, ("user-copy", key), Icon::Copy, t!("chat.copy_reply"), move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(raw.clone()))).into_any_element();
    let handle = rows.handle.clone();
    v_flex()
        .group(group.clone())
        .relative()
        .gap_1()
        // Its edge on the column's, its words where every row's start.
        .mx(px(-TEXT_INSET))
        .px(px(TEXT_INSET))
        .py_3()
        .mt_3()
        .rounded(px(crate::theme::R_LG))
        .bg(p.wash)
        .text_size(px(BODY))
        .line_height(px(24.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(h_flex().items_start().gap_3().child(div().flex_none().text_color(p.accent).child("›")).child(div().min_w_0().flex_1().child(body)))
        .when(long, |col| {
            col.child(
                div()
                    .id(("user-more", key))
                    .pl(px(22.))
                    .text_size(px(SMALL))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(p.accent)
                    .cursor_pointer()
                    .child(if open { t!("chat.show_less") } else { t!("chat.show_more") })
                    .on_click(move |_, _, cx| {
                        handle.update(cx, |shell, cx| shell.chat_toggle(key, PART_MORE, cx)).ok();
                    }),
            )
        })
        .child(toolbar(p, group, false, vec![copy]))
        .into_any_element()
}

/// Your words as Markdown that shows them as typed: every mark escaped, every
/// line break kept, indentation not read as code.
pub fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    let lines: Vec<&str> = text.split('\n').collect();
    for (n, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start_matches(' ');
        out.extend(std::iter::repeat_n('\u{A0}', line.len() - trimmed.len()));
        for c in trimmed.chars() {
            if c.is_ascii_punctuation() {
                out.push('\\');
            }
            out.push(c);
        }
        if n + 1 < lines.len() {
            // A hard break, unless the next line is blank (a paragraph break).
            if !line.trim().is_empty() && !lines[n + 1].trim().is_empty() {
                out.push('\\');
            }
            out.push('\n');
        }
    }
    out
}

/// Small buttons over a row, shown while the pointer is on it.
fn toolbar(p: Palette, group: SharedString, under_header: bool, items: Vec<AnyElement>) -> impl IntoElement {
    h_flex()
        .absolute()
        .right(px(if under_header { 0. } else { 4. }))
        .top(px(if under_header { 10. } else { 6. }))
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(R_MD))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .invisible()
        .group_hover(group, |s| s.visible())
        .children(items)
}

/// An icon over a row (copy, ask again), named in its tooltip.
fn tool_button(p: Palette, id: (&'static str, u64), glyph: Icon, label: &'static str, on_click: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
    let b = div()
        .id(id)
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_SM))
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .child(icon(glyph, ICON_SM, p.text2))
        .on_click(move |_, window, cx| on_click(window, cx));
    tip(b, p, label, None)
}

/// A run of tool steps: each with how it ended, how long it took, and — open
/// beneath it — a command's output or an edit's change.
fn steps(rows: &Rows, key: u64, lines: &[ToolLine], window: &mut Window, cx: &mut App) -> AnyElement {
    let p = rows.p;
    let reduce = cx.reduce_motion();
    let _ = window;
    v_flex()
        .font_family(MONO)
        .text_size(px(12.5))
        .line_height(px(20.))
        .text_color(p.muted)
        .children(crate::convo::run_rows(lines, rows.expanded).into_iter().enumerate().map(|(n, r)| match r {
            RunRow::Line(line) => {
                let at = lines.iter().position(|l| std::ptr::eq(l, line)).unwrap_or(n) as u32;
                step(rows, key, at, line, reduce)
            }
            RunRow::Folded(hidden) => {
                let handle = rows.handle.clone();
                h_flex()
                    .id(("unfold", key << 4 | n as u64))
                    .gap_2()
                    .cursor_pointer()
                    .hover(move |s| s.text_color(p.text2))
                    .on_click(move |_, _, cx| {
                        handle.update(cx, |shell, cx| shell.toggle_folds(cx)).ok();
                    })
                    .child(div().w(px(14.)).flex_none().child("⋮"))
                    .child(t!("chat.more_steps", hidden = hidden))
                    .into_any_element()
            }
        }))
        .into_any_element()
}

fn step(rows: &Rows, key: u64, at: u32, line: &ToolLine, reduce: bool) -> AnyElement {
    let p = rows.p;
    let t = rows.tones;
    let mark: AnyElement = match line.status {
        Status::Running if !reduce => Spinner::new().with_size(px(11.)).color(Hsla::from(p.muted)).into_any_element(),
        Status::Running => div().child("◌").into_any_element(),
        Status::Done => div().text_color(if line.edits { p.accent } else { t.added }).child("✓").into_any_element(),
        Status::Failed => div().text_color(t.danger).child("✗").into_any_element(),
        Status::Unknown => div().text_color(if line.edits { p.accent } else { p.muted }).child(if line.edits { "✎" } else { "·" }).into_any_element(),
    };
    let failed = line.status == Status::Failed;
    let head = h_flex()
        .gap_2()
        .items_center()
        .when(line.edits, |l| l.text_color(p.text2))
        .child(div().w(px(14.)).flex_none().flex().justify_center().child(mark))
        .child(div().w(px(52.)).flex_none().child(crate::convo::verb_word(&line.verb).to_owned()))
        .child(div().min_w_0().flex_1().overflow_hidden().child(SelectableText::new(("step", key << 8 | at as u64), line.text())))
        .children(line.exit.filter(|_| failed).map(|code| div().flex_none().px_1().rounded(px(4.)).bg(t.danger_bg).text_color(t.danger).child(t!("chat.exit_code", code = code))))
        .children(line.ms.filter(|_| line.status != Status::Running).map(|ms| div().flex_none().text_color(p.muted).child(tools::duration(ms))));
    let output = line.output.as_ref().map(|(text, skipped)| output_box(rows, key, at, text, *skipped, failed));
    let diff = line.diff.as_deref().and_then(tools::diff_card).map(|card| diff_card(rows, key, at, card, line.path.as_deref()));
    v_flex().child(head).children(output).children(diff).into_any_element()
}

/// A command's output: its last lines, the rest a click away.
fn output_box(rows: &Rows, key: u64, at: u32, text: &str, skipped: u32, failed: bool) -> AnyElement {
    let p = rows.p;
    let t = rows.tones;
    let part = PART_OUTPUT + at;
    let open = rows.opened.contains(&(key, part));
    let all: Vec<&str> = text.lines().collect();
    let (shown, before) = if open { (all.clone(), skipped as usize) } else { tools::output_preview(text, skipped) };
    let more = all.len() > shown.len() || open && all.len() > tools::OUTPUT_PREVIEW;
    let handle = rows.handle.clone();
    v_flex()
        .ml(px(22.))
        .mt_1()
        .mb_1()
        .px_3()
        .py(px(6.))
        .rounded(px(6.))
        .bg(if failed { t.danger_bg } else { p.code_bg })
        .border_1()
        .border_color(if failed { Rgba { a: 0.35, ..t.danger } } else { p.hair })
        .text_size(px(12.))
        .line_height(px(18.))
        .text_color(if failed { t.danger } else { p.text2 })
        .when(before > 0, |col| col.child(div().text_color(p.muted).child(t!("chat.output_before", count = before))))
        .child(SelectableText::new(("output", key << 8 | at as u64), shown.join("\n")))
        .when(more, |col| {
            col.child(
                div()
                    .id(("output-more", key << 8 | at as u64))
                    .pt_1()
                    .text_color(p.accent)
                    .cursor_pointer()
                    .child(if open { t!("chat.output_less").to_owned() } else { t!("chat.output_more", count = all.len()) })
                    .on_click(move |_, _, cx| {
                        handle.update(cx, |shell, cx| shell.chat_toggle(key, part, cx)).ok();
                    }),
            )
        })
        .into_any_element()
}

/// An edit's change as a card: the file, what came and went, the first lines.
fn diff_card(rows: &Rows, key: u64, at: u32, card: tools::DiffCard, path: Option<&str>) -> AnyElement {
    let p = rows.p;
    let t = rows.tones;
    let part = PART_DIFF + at;
    let open = rows.opened.contains(&(key, part));
    let hidden = card.hidden();
    let shown: Vec<crate::diffs::Line> = if open { card.lines.clone() } else { card.preview().to_vec() };
    let full = path.map_or_else(|| card.path.clone(), str::to_owned);
    // The chip names the file; beside it, the folder it is in.
    let shown_path = std::path::Path::new(&relative(&full, &rows.base)).parent().map(|d| d.display().to_string()).filter(|d| !d.is_empty()).map(|d| format!("{d}/")).unwrap_or_default();
    let chip = plugins::file_chip(p, ("diff-file", key << 8 | at as u64), std::path::Path::new(&full), None, &full, px(12.));
    let handle = rows.handle.clone();
    v_flex()
        .ml(px(22.))
        .mt_1()
        .mb_1()
        .rounded(px(6.))
        .border_1()
        .border_color(p.hair)
        .overflow_hidden()
        .child(
            h_flex()
                .px_2()
                .py(px(4.))
                .gap_2()
                .bg(p.code_bg)
                .border_b_1()
                .border_color(p.hair)
                .child(chip)
                .child(div().min_w_0().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(p.muted).child(shown_path))
                .child(div().text_color(t.added).child(format!("+{}", card.added)))
                .child(div().text_color(t.removed).child(format!("−{}", card.removed))),
        )
        .children(shown.into_iter().map(move |l| {
            let (sign, bg, color) = match l.kind {
                Kind::Add => ("+", Some(t.added_bg), p.text),
                Kind::Del => ("−", Some(t.removed_bg), p.text2),
                Kind::Hunk => ("", Some(p.code_bg), p.muted),
                Kind::Note => ("", None, p.muted),
                Kind::Context => (" ", None, p.muted),
            };
            h_flex()
                .px_2()
                .text_size(px(12.))
                .line_height(px(18.))
                .when_some(bg, |row, bg| row.bg(bg))
                .text_color(color)
                .child(div().w(px(14.)).flex_none().text_color(match l.kind {
                    Kind::Add => t.added,
                    Kind::Del => t.removed,
                    _ => p.muted,
                }).child(sign))
                .child(div().min_w_0().flex_1().overflow_hidden().whitespace_nowrap().child(if l.text.is_empty() { " ".to_owned() } else { l.text.replace('\t', "    ") }))
        }))
        .when(hidden > 0 || open, |col| {
            col.child(
                div()
                    .id(("diff-more", key << 8 | at as u64))
                    .px_2()
                    .py(px(3.))
                    .border_t_1()
                    .border_color(p.hair)
                    .text_size(px(12.))
                    .text_color(p.accent)
                    .cursor_pointer()
                    .child(if open { t!("chat.diff_less").to_owned() } else { t!("chat.diff_more", count = hidden) })
                    .on_click(move |_, _, cx| {
                        handle.update(cx, |shell, cx| shell.chat_toggle(key, part, cx)).ok();
                    }),
            )
        })
        .into_any_element()
}

/// ⌘F's bar over the conversation: the words, how many, up, down, close.
fn find_bar(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let find = &shell.chat.find;
    let input = find.input.clone().filter(|_| find.open)?;
    let p = shell.palette;
    let count = if find.query.is_empty() {
        String::new()
    } else if find.hits.is_empty() {
        t!("chat.find_none").to_owned()
    } else {
        format!("{}/{}", find.at + 1, find.hits.len())
    };
    let button = |id: &'static str, glyph: Icon, label: &'static str, keys: &'static str| {
        let b = div().id(id).size(px(24.)).flex().flex_none().items_center().justify_center().rounded(px(R_SM)).cursor_pointer().hover(move |s| s.bg(p.wash)).child(icon(glyph, ICON_SM, p.text2));
        tip(b, p, label, Some(keys))
    };
    let bar = h_flex()
        .key_context("FindBar")
        .w(px(340.))
        .h(px(CONTROL + 8.))
        .gap_1()
        .pl_1()
        .pr_1()
        .items_center()
        .rounded(px(R_MD))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .text_size(px(SMALL))
        .child(div().flex_1().min_w_0().child(Input::new(&input).appearance(false).cleanable(false)))
        .child(div().flex_none().pr_1().text_size(px(12.)).text_color(p.muted).child(count))
        .child(button("find-prev", Icon::ChevronUp, t!("chat.find_previous"), "⇧⏎").on_click(cx.listener(|this, _, _, cx| this.find_step(true, cx))))
        .child(button("find-next", Icon::ChevronDown, t!("chat.find_next"), "⏎").on_click(cx.listener(|this, _, _, cx| this.find_step(false, cx))))
        .child(button("find-close", Icon::X, t!("chat.find_close"), "Esc").on_click(cx.listener(|this, _, window, cx| this.find_close(window, cx))));
    Some(div().absolute().top(px(8.)).right(px(16.)).child(motion::pop(bar, "find-in", false)))
}

/// An image from a reply, large, over the conversation; a click puts it away.
fn lightbox(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let source = shell.chat.lightbox.clone()?;
    let backdrop = div()
            .id("lightbox")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_8()
            .bg(rgba(0x000000C8))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.chat.lightbox = None;
                cx.notify();
            }))
            .child(motion::rise(div().max_w_full().max_h_full().flex().items_center().justify_center().child(img(source).max_w_full().max_h_full().rounded(px(R_MD))), "lightbox-image"));
    Some(motion::fade(backdrop, "lightbox-in", motion::BASE))
}


/// Markdown at `size`, in the palette's colours; headings stay near body size
/// (a conversation is not a document), `T-n` is a way to the board. For text
/// outside the conversation's rows (the board's).
pub fn markdown(p: Palette, id: (&'static str, u64), text: &str, size: f32, line: f32, color: Rgba, _handle: &Handle) -> impl IntoElement + use<> {
    let look = Look { size, line, color, live: false };
    let (text, full) = view::prepare(text, false, &|_| true);
    div().text_size(px(size)).line_height(px(line)).text_color(color).child(view::configure(
        TextView::markdown(ElementId::NamedInteger(id.0.into(), id.1), text),
        p,
        look,
        hash_of(id),
        Arc::new(full),
    ))
}

impl Shell {
    pub fn toggle_folds(&mut self, cx: &mut Context<Self>) {
        self.chat.expanded = !self.chat.expanded;
        self.chat.sync(&self.model, cx);
        cx.notify();
    }

    /// Open or close part of a row (an output, a change, a long message).
    pub fn chat_toggle(&mut self, row: u64, part: u32, cx: &mut Context<Self>) {
        self.chat.toggle(row, part, cx);
        cx.notify();
    }

    /// A control drawn as a link in a row: a code block shown whole or cut.
    pub fn chat_control(&mut self, row: u64, what: &str, cx: &mut Context<Self>) {
        let (verb, n) = what.split_once('/').unwrap_or((what, ""));
        let Ok(n) = n.parse::<u32>() else { return };
        let open = self.chat.opened.contains(&(row, n));
        if (verb == "expand") != open {
            self.chat.toggle(row, n, cx);
        }
        cx.notify();
    }

    /// Ask the last thing said again.
    pub fn retry_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.chat.blocks.iter().rev().find_map(|b| if let Block::User(t) = b { Some(t.clone()) } else { None }) else { return };
        self.composer.set_text(&text, window, cx);
        self.send(window, cx);
    }

    /// ⌘F: the find bar, its words chosen.
    pub fn find_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.chat.find.input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("chat.find_placeholder")));
            let subscription = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    this.chat.search(cx);
                    cx.notify();
                }
                InputEvent::PressEnter { shift, .. } => this.find_step(*shift, cx),
                _ => {}
            });
            self.chat.find.input = Some(input);
            self.chat.find._subscription = Some(subscription);
        }
        self.chat.find.open = true;
        if let Some(input) = &self.chat.find.input {
            input.update(cx, |s, cx| {
                s.focus(window, cx);
                s.select_all(window, cx);
            });
        }
        self.chat.search(cx);
        cx.notify();
    }

    /// Put `words` in the find bar (the snapshot script's).
    #[cfg_attr(not(feature = "snapshot"), allow(dead_code))]
    pub fn find_type(&mut self, words: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = self.chat.find.input.clone() {
            input.update(cx, |s, cx| s.set_value(words.to_owned(), window, cx));
        }
        self.chat.search(cx);
        self.find_step(false, cx);
    }

    pub fn find_step(&mut self, back: bool, cx: &mut Context<Self>) {
        self.chat.step_hit(back, cx);
        // The row may only now be drawn: mark it once it is.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(120)).await;
            let _ = this.update(cx, |this, cx| {
                this.chat.paint_hits(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn find_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat.find.open = false;
        self.chat.paint_hits(cx);
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// Escape: the lightbox goes, then the find bar.
    pub fn chat_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.chat.lightbox.take().is_some() {
            cx.notify();
            return true;
        }
        if self.chat.find.open {
            self.find_close(window, cx);
            return true;
        }
        false
    }

    /// A line of the sidebar was clicked: the PM, someone, or a ticket — its
    /// work in the conversation's place, or the board when there is none.
    pub fn open_side(&mut self, side: crate::state::Side, window: &mut Window, cx: &mut Context<Self>) {
        if !self.model.open(side) {
            if let crate::state::Side::Ticket(id) = side {
                self.open_ticket(id, cx);
            }
            return;
        }
        self.screen = Screen::Chat;
        self.refresh(window, cx);
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// A ticket number was clicked: show it on the board.
    pub fn open_ticket(&mut self, ticket: u32, cx: &mut Context<Self>) {
        self.board_ticket = Some(ticket);
        self.show(Screen::Board, cx);
    }
}

/// The chat screen: header, then the conversation and the composer.
pub fn screen(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    v_flex().flex_1().min_h_0().child(header::render(shell, cx)).child(render(shell, cx))
}

#[cfg(test)]
mod tests {
    use super::{Block, escape_markdown, find_hits, occurrences};

    #[test]
    fn your_words_show_as_typed() {
        assert_eq!(escape_markdown("# not a heading"), "\\# not a heading");
        assert_eq!(escape_markdown("a *b* [c](d)"), "a \\*b\\* \\[c\\]\\(d\\)");
        assert_eq!(escape_markdown("one\ntwo\n\nthree"), "one\\\ntwo\n\nthree");
        assert_eq!(escape_markdown("    indented"), "\u{A0}\u{A0}\u{A0}\u{A0}indented");
        assert_eq!(escape_markdown("1. first"), "1\\. first");
        assert_eq!(escape_markdown("한국어 그대로"), "한국어 그대로");
    }

    #[test]
    fn find_counts_every_match_by_row() {
        let blocks = vec![
            Block::User("Fix the header".into()),
            Block::Reply { text: "The header and the HEADER.".into(), first: true, live: false },
            Block::Thought { n: 0, secs: 2, text: "header".into() },
            Block::Note("footer".into()),
        ];
        assert_eq!(find_hits(&blocks, "header"), [(0, 0), (1, 0), (1, 1)]);
        assert_eq!(find_hits(&blocks, "  "), []);
        assert_eq!(occurrences("A header, a Header", "header"), [2..8, 12..18]);
    }
}
