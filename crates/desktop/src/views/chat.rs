//! The conversation with a member (T-54 §3.2–3.3): your messages, the member's
//! replies as Markdown, tool calls gathered into quiet groups that open in
//! place — each call with how it ended, a command's output and an edit's
//! change — news, notes, errors and commits, in a virtual list that follows
//! the end while you are there. A ticket named in a row opens beside it.
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
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{approvals, attached, composer, header, yourturn};
use crate::activity;
use crate::convo::{Block, Status, ToolLine, relative};
use crate::diffs::Kind;
use crate::render::view::{self, Look};
use crate::render::{self, Tones, hash_of, plugins, tools};
use crate::state::{Model, Presence};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{BODY, BODY_LINE, CONTROL, ICON_SM, MEASURE, META, META_LINE, MONO, Palette, R_LG, R_MD, R_SM, SMALL, SMALL_LINE, TAIL_ROOM, look};
use crate::tips::tip;

/// Where text starts inside the column: the cards' and the composer's inner
/// padding plus their border. Replies, banners and card text all start here;
/// cards and your own messages have their edge at the column's.
const TEXT_INSET: f32 = 13.;
/// How long after it arrives a row is still arriving (its entrance plays).
const ARRIVING: std::time::Duration = std::time::Duration::from_millis(400);
use crate::{Screen, Shell};

use crate::theme::COLUMN;
/// Text views kept for rows scrolled out of sight, so coming back to them
/// doesn't parse their Markdown again.
const VIEWS_KEPT: usize = 96;
/// A message of yours longer than this many lines shows its start.
const USER_LINES: usize = 8;
/// Your message's bubble: its padding across and down, and the share of the
/// column it may take.
const BUBBLE_X: f32 = 14.;
const BUBBLE_Y: f32 = 10.;
const BUBBLE_SHARE: f32 = 0.74;

/// What a row has open (or, open by itself, closed), beside its code fences
/// (numbered from 0).
const PART_MORE: u32 = 1 << 20;
const PART_THOUGHT: u32 = PART_MORE + 1;
const PART_NEWS: u32 = PART_MORE + 2;
const PART_GROUP: u32 = PART_MORE + 3;
/// A call of an open group, by its number in it.
const PART_ROW: u32 = 1 << 21;
/// The side panel's width; the conversation keeps the rest.
const SIDE: f32 = 420.;

/// Whose conversation, the ticket it shows the work of, and the project's
/// slice of it on screen (`Model::chat_slice`).
type Shown = (SessionId, Option<u32>, Option<Option<String>>);

pub struct Chat {
    scroller: Entity<MessageScrollerState>,
    pub blocks: Rc<Vec<Block>>,
    /// Whose conversation `blocks` are, the ticket it shows the work of, and
    /// the slice of it.
    shown: Option<Shown>,
    /// What each of `blocks` is about (`None`: the whole squad, or not said).
    about: Rc<Vec<Option<String>>>,
    /// The latest reply came about another project than the one shown.
    answered_in: Option<Option<String>>,
    /// The conversation follows the project in focus (settings ▸ 일반).
    pub follow: bool,
    /// Folded stretches of other projects the person opened.
    pub elsewhere: HashSet<u64>,
    /// The list was drawn anew for another slice: it fades in (by serial).
    faded_in: Option<u64>,
    seen_revision: u64,
    /// Every group of calls and run of news open (⌘⇧O).
    pub expanded: bool,
    shown_expanded: bool,
    /// Tells one conversation's rows from another's.
    serial: u64,
    /// Text views by what they show, kept across scrolling.
    views: Rc<RefCell<Views>>,
    /// What the person opened (or closed, of what opens by itself) in the
    /// rows: `(row, part)`.
    opened: Rc<HashSet<(u64, u32)>>,
    /// The same, of the conversations not on screen, by row number: each
    /// shows as it was left while the app runs.
    remembered: HashMap<Shown, HashSet<(usize, u32)>>,
    /// Tickets opened beside the conversation, the one shown last.
    pub side: Vec<u32>,
    /// An image looked at closer, over the conversation.
    pub lightbox: Option<ImageSource>,
    /// What your messages attached, as far as it is known (`crate::sent`).
    pub sent: Rc<RefCell<crate::sent::Sent>>,
    /// A message's pictures shown big, ←/→ through them.
    pub gallery: Option<crate::sent::Gallery>,
    pub find: Find,
    /// Rows that came in while the conversation was on screen, and when: they
    /// rise into place (rows scrolled back into view don't).
    arrived: Rc<HashMap<u64, std::time::Instant>>,
    /// A bubble waiting for the member, reached from the empty box with ↑
    /// (its words and when it was said): ringed, Backspace takes it back.
    pub picked: Option<(String, u64)>,
    /// Bubbles taken back, fading out until the daemon says they are gone.
    pub leaving: Rc<HashSet<String>>,
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
            about: Rc::default(),
            answered_in: None,
            follow: true,
            elsewhere: HashSet::new(),
            faded_in: None,
            seen_revision: 0,
            expanded: false,
            shown_expanded: false,
            serial: 0,
            views: Rc::default(),
            opened: Rc::default(),
            remembered: HashMap::new(),
            side: Vec::new(),
            lightbox: None,
            sent: Rc::default(),
            gallery: None,
            find: Find::default(),
            arrived: Rc::default(),
            picked: None,
            leaving: Rc::default(),
        }
    }

    /// Bring the list up to what the model shows. Cheap when nothing moved.
    pub fn sync(&mut self, model: &Model, looks: &HashMap<u32, u64>, cx: &mut App) {
        let key = model.viewed().map(|id| (id, model.opened, model.chat_slice(self.follow)));
        let moved = key != self.shown;
        if !moved && model.revision == self.seen_revision && self.expanded == self.shown_expanded {
            return;
        }
        let mut view = model.conversation(self.follow, &self.elsewhere);
        crate::yourturn::mark(&mut view.blocks, looks);
        crate::convo::mark_handed(&mut view.blocks);
        self.about = Rc::new(view.about);
        self.answered_in = view.answered_in;
        let old = std::mem::replace(&mut self.blocks, Rc::new(view.blocks));
        self.seen_revision = model.revision;
        let folds_changed = self.expanded != self.shown_expanded;
        self.shown_expanded = self.expanded;
        let n = self.blocks.len();
        if moved {
            // What was open stays so for when the conversation comes back.
            // Only the slice changed: the same conversation fades in, at its
            // end, rather than flashing.
            let resliced = matches!((&self.shown, &key), (Some(a), Some(b)) if a.0 == b.0 && a.1 == b.1);
            if let Some(old) = self.shown.take() {
                let serial = self.serial;
                let kept: HashSet<(usize, u32)> = self.opened.iter().filter(|(row, _)| row >> 24 == serial).map(|(row, part)| ((row & 0xFF_FFFF) as usize, *part)).collect();
                self.remembered.insert(old, kept);
            }
            self.serial += 1;
            let serial = self.serial;
            self.faded_in = resliced.then_some(serial);
            self.opened = Rc::new(key.as_ref().and_then(|k| self.remembered.get(k)).map(|kept| kept.iter().map(|(ix, part)| (serial << 24 | *ix as u64, *part)).collect()).unwrap_or_default());
            self.shown = key;
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
            // Rows put in between (a fold opened) rise where they come; else the new ones at the end.
            let tail = old.iter().rev().zip(self.blocks.iter().rev()).take_while(|(a, b)| a == b).count().min(old.len() - same);
            let added = n - old.len();
            for ix in n - tail - added..n - tail {
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

    /// The `n`th stretch of other projects folded in the conversation (the snapshot script's).
    #[cfg_attr(not(feature = "snapshot"), allow(dead_code))]
    pub fn nth_fold(&self, n: usize) -> Option<u64> {
        self.blocks.iter().filter_map(|b| if let Block::Elsewhere { key, .. } = b { Some(*key) } else { None }).nth(n)
    }

    /// Back to the end, following it again.
    pub fn follow_tail(&self, cx: &mut App) {
        self.scroller.update(cx, |s, cx| s.scroll_to_end(cx));
    }

    /// The row where `sub` was handed its work, as the conversation shows
    /// now — its group of calls opened, so its line is in sight.
    pub fn open_delegation(&mut self, sub: &crate::convo::Sub, cx: &mut App) -> Option<usize> {
        let (ix, _) = crate::convo::delegation_of(&self.blocks, sub)?;
        let row = self.serial << 24 | ix as u64;
        if matches!(self.blocks.get(ix), Some(Block::Steps(lines)) if lines.len() > 1) && !self.opened.contains(&(row, PART_GROUP)) {
            self.toggle(row, PART_GROUP, cx);
        }
        Some(ix)
    }

    /// Bring row `ix` to the top (the snapshot script's; a subagent's row in the sidebar).
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

    /// A file a row shows came (or is gone): every row naming it is measured again.
    pub fn remeasure_mentioning(&self, path: &str, cx: &mut App) {
        let rows: Vec<usize> = self.blocks.iter().enumerate().filter(|(_, b)| matches!(b, Block::User(t) | Block::Pending { text: t, .. } if t.contains(path))).map(|(ix, _)| ix).collect();
        if rows.is_empty() {
            return;
        }
        self.scroller.update(cx, |s, cx| {
            for ix in rows {
                s.remeasure_items(ix..ix + 1, cx);
            }
        });
    }

    /// The rows holding ticket `id`'s card are measured again (a box grew).
    pub fn remeasure_turn(&self, id: u32, cx: &mut App) {
        let rows: Vec<usize> = self.blocks.iter().enumerate().filter(|(_, b)| matches!(b, Block::YourTurn { ticket, live: true, .. } if *ticket == id)).map(|(ix, _)| ix).collect();
        if rows.is_empty() {
            return;
        }
        self.scroller.update(cx, |s, cx| {
            for ix in rows {
                s.remeasure_items(ix..ix + 1, cx);
            }
        });
    }

    /// Row `ix` is measured again (what it shows grew or shrank in place).
    pub fn remeasure_at(&self, ix: usize, cx: &mut App) {
        if ix < self.blocks.len() {
            self.scroller.update(cx, |s, cx| s.remeasure_items(ix..ix + 1, cx));
        }
    }

    /// The latest reply that embeds pictures: its row's key, and each picture
    /// (where it is, what it is called) — read against `base`.
    #[cfg_attr(not(feature = "snapshot"), allow(dead_code))]
    pub fn latest_pictures(&self, base: &std::path::Path) -> Option<(u64, Vec<(String, String)>)> {
        use equipe_core::visuals::markdown;
        self.blocks.iter().enumerate().rev().find_map(|(ix, block)| {
            let Block::Reply { text, .. } = block else { return None };
            let pictures: Vec<(String, String)> = markdown::local_images(text).into_iter().filter_map(|i| Some((markdown::lexical(&i.path, base)?.to_string_lossy().into_owned(), i.alt))).collect();
            (!pictures.is_empty()).then_some((self.serial << 24 | ix as u64, pictures))
        })
    }

    /// Every row is measured again (a gallery in one folded or unfolded).
    pub fn remeasure_all(&self, cx: &mut App) {
        self.scroller.update(cx, |s, cx| s.remeasure(cx));
    }

    /// Row `row` (a key: serial and index) is measured again, if it is of the conversation on show.
    pub fn remeasure_row(&self, row: u64, cx: &mut App) {
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

impl Chat {
    /// The text views drawn for row `ix` of the conversation on screen
    /// (`views/quote.rs` lights quoted words in them).
    pub fn row_views(&self, ix: usize) -> Vec<Entity<TextViewState>> {
        let row = self.serial << 24 | ix as u64;
        self.views.borrow().map.iter().filter(|(key, _)| *key >> 2 == row).map(|(_, (state, _))| state.clone()).collect()
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
        // Its own pane: the composer's cursor, a card's box, the header don't
        // lay a day's conversation out again (`pane.rs`).
        Some(_) => shell.panes.conversation.clone().cached(StyleRefinement::default().flex_1().min_h_0().w_full().max_w(px(COLUMN))).into_any_element(),
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
            .child(div().text_size(px(META)).line_height(px(META_LINE)).text_color(p.muted).child(hint))
    });
    let main = v_flex()
        .flex_1()
        .min_w_0()
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
                .child(v_flex().children(approvals::for_you(shell, cx)).children(approvals::approval(shell, cx)).children(super::decide::docked(shell, cx)).children(super::accept::docked(shell, cx)).children(super::team::docked(shell, cx)).children(super::switching::docked(shell, cx)).children(super::asklist::hint(shell, cx)))
                .child(composer::render(shell, cx)),
        )
        .children(lightbox(shell, cx));
    h_flex().flex_1().min_h_0().items_stretch().child(main).children(side_panel(shell, cx))
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
        .child(div().text_size(px(BODY)).line_height(px(BODY_LINE)).text_color(p.text2).text_center().child(text.into()))
}

#[derive(Clone)]
struct Who {
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
    /// The latest finished reply: its actions stay in sight.
    latest_reply: Option<usize>,
    /// Replies that end a turn: the actions go under them.
    turn_ends: HashSet<usize>,
    /// Since when the call under way has run.
    step_since: Option<std::time::Instant>,
    base: std::path::PathBuf,
    arrived: Rc<HashMap<u64, std::time::Instant>>,
    /// A hand-in's pictures, under the news of it: what they are drawn with, and whose.
    shots: super::visuals::Shots,
    pictures: HashMap<u32, Rc<Vec<String>>>,
    /// What your messages attached, drawn with.
    attached: attached::Ctx,
    /// The cards of tickets waiting for the person, as things stand.
    turns: HashMap<u32, Rc<yourturn::Card>>,
    /// What each row is about, and — while the conversation follows the
    /// project, of several — the projects in order, for the chips and dots
    /// that say so, and the one in focus (its own rows need none).
    about: Rc<Vec<Option<String>>>,
    projects: Rc<Vec<String>>,
    focus: Option<String>,
    /// The latest reply, when it asks the person things in a list: its row,
    /// the list, and the form it turns into.
    asking: Option<Rc<super::asklist::Shown>>,
    /// The squad, and the members whose browser is in the person's hands
    /// now: what a live hand-over row offers.
    squad: String,
    held: HashSet<String>,
    /// The bubble waiting that ↑ reached, those fading out, and how many
    /// wait in all (what "send now" sends).
    picked: Option<(String, u64)>,
    leaving: Rc<HashSet<String>>,
    waiting: usize,
}

impl Rows {
    /// The project row `ix` is about, as its chip or dot shows it: its name
    /// and colour — while every project shows, or in a fold of other
    /// projects' talk opened.
    fn project_of(&self, ix: usize) -> Option<(String, Rgba)> {
        let name = self.about.get(ix)?.as_ref().filter(|n| Some(*n) != self.focus.as_ref())?;
        let at = self.projects.iter().position(|p| p == name)?;
        Some((name.clone(), header::project_color(self.p, at)))
    }

    /// Whether `part` of row `key` is open: as it opens by itself
    /// (`default`), unless the person turned it.
    fn is_open(&self, key: u64, part: u32, default: bool) -> bool {
        default != self.opened.contains(&(key, part))
    }
}

/// The replies that end a turn: the last before the person speaks again, or
/// before the end.
fn turn_ends(blocks: &[Block]) -> HashSet<usize> {
    let mut out = HashSet::new();
    let mut last: Option<usize> = None;
    for (ix, block) in blocks.iter().enumerate() {
        match block {
            Block::Reply { .. } => last = Some(ix),
            Block::User(_) => out.extend(last.take()),
            _ => {}
        }
    }
    out.extend(last);
    out
}

/// The conversation with the member selected, in its pane.
pub fn conversation_pane(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let Some(member) = shell.model.selected_member().filter(|_| !shell.chat.blocks.is_empty()) else {
        return Empty.into_any_element();
    };
    let (_, hue) = look(&member.role, &member.name);
    let who = Who { name: member.name.clone(), color: shell.palette.member(hue) };
    v_flex().size_full().items_center().child(conversation(shell, who, cx)).into_any_element()
}

fn conversation(shell: &Shell, who: Who, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let blocks = shell.chat.blocks.clone();
    let working = blocks.iter().any(|b| matches!(b, Block::Reply { live: true, .. } | Block::Thinking(_)))
        || shell.model.selected_member().is_some_and(|m| m.presence == Presence::Working);
    let retry_at = (!working).then(|| blocks.iter().rposition(|b| matches!(b, Block::Reply { .. }))).flatten().filter(|&ix| blocks[..ix].iter().any(|b| matches!(b, Block::User(_))));
    let latest_reply = blocks.iter().rposition(|b| matches!(b, Block::Reply { live: false, .. }));
    let turn_ends = turn_ends(&blocks);
    let replying = blocks.iter().any(|b| matches!(b, Block::Reply { live: true, .. }));
    // The conversation sliced, of several projects: each message not of the
    // project in focus says which it was about.
    let names = shell.model.project_names();
    let slice = shell.model.chat_slice(shell.chat.follow);
    let every = slice.is_some() && names.len() > 1;
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
        latest_reply,
        turn_ends,
        step_since: shell.model.convo().and_then(|c| c.step_since()),
        base: render::base_dir(shell),
        arrived: shell.chat.arrived.clone(),
        attached: attached::Ctx::of(shell, cx.entity().downgrade(), COLUMN - 120.),
        shots: super::visuals::Shots::of(shell, cx.entity().downgrade(), cx),
        pictures: shell.model.tickets.iter().filter(|t| !t.images.is_empty() && matches!(t.status, equipe_core::protocol::TicketStatus::Review | equipe_core::protocol::TicketStatus::Done)).map(|t| (t.id, Rc::new(t.images.clone()))).collect(),
        about: shell.chat.about.clone(),
        projects: Rc::new(if every { names } else { Vec::new() }),
        focus: slice.flatten(),
        asking: super::asklist::shown(shell, cx),
        held: {
            let browser = shell.browser.read(cx);
            shell.chat.blocks.iter().filter_map(|b| if let Block::HandOver { name, live: true, .. } = b { Some(name.clone()) } else { None }).filter(|name| browser.held_by_person(&format!("{}/{name}", shell.model.squad))).collect()
        },
        squad: shell.model.squad.clone(),
        picked: shell.chat.picked.clone(),
        leaving: shell.chat.leaving.clone(),
        waiting: shell.model.convo().map_or(0, |c| c.queue.items().len()),
        turns: shell
            .chat
            .blocks
            .iter()
            .filter_map(|b| if let Block::YourTurn { ticket, live: true, .. } = b { Some(*ticket) } else { None })
            .filter_map(|id| yourturn::Card::of(shell, id, cx).map(|c| (id, c)))
            .collect(),
    });
    // While a reply streams: a small pulse over the composer, where the jump
    // button would be (which has the place while you read back).
    let following = shell.chat.scroller.read(cx).is_following_tail();
    // A reply about another project came: a pill to go there, the view stays.
    let answered = shell.chat.answered_in.clone().map(|about| answered_pill(p, about, cx));
    let pulse = (replying && following && answered.is_none()).then(|| pulse_pill(p));
    // Our own pill, not the kit's jump button: the kit's put a tooltip with the
    // same word on it, in its own white, over the conversation.
    let jump = (!following && answered.is_none()).then(|| jump_pill(p, shell.chat.scroller.clone()));
    // Words selected in it are offered to quote (`views/quote.rs`).
    let scroller = div()
        .flex_1()
        .min_h_0()
        .w_full()
        .relative()
        .on_mouse_down(MouseButton::Left, cx.listener(|shell, _, _, cx| shell.quote_pointer_down(cx)))
        // The release is heard wherever it lands: a drag that ends over the
        // composer, a card or past the window's edge still offers its words.
        .child({
            let shell = cx.entity().downgrade();
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let shell = shell.clone();
                    window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture && e.button == MouseButton::Left {
                            let _ = shell.update(cx, |shell, cx| shell.quote_pointer_up(e.position, window, cx));
                        }
                    });
                },
            )
            .absolute()
            .size_0()
        })
        .children(pulse)
        .children(jump)
        .child(
        MessageScroller::new("conversation", shell.chat.scroller.clone(), move |ix, window, cx| row(&rows, ix, window, cx))
            .jump_button(false)
            .with_row_style(StyleRefinement::default().pb(px(8.)).pl(px(TEXT_INSET)).pr(px(TEXT_INSET)))
            // Room under the last line, so it never sits on the composer's
            // edge; read back, the rows fade out there under the jump pill.
            .with_list_style(StyleRefinement::default().pt(px(8.)).pb(px(TAIL_ROOM)))
            .with_bottom_fade(p.bg)
    );
    // The pill has its own room under the list, so it covers nothing.
    let list = v_flex().flex_1().min_h_0().w_full().max_w(px(COLUMN)).px_4().child(scroller).children(answered);
    // Another project's slice: the list fades in at its end.
    match shell.chat.faded_in {
        Some(serial) if serial == shell.chat.serial => motion::fade(list, ("slice-in", serial), motion::BASE).into_any_element(),
        _ => list.into_any_element(),
    }
}

/// Where a reply about another project came, over the composer: a click goes
/// to that project (or every project, for the whole squad's).
fn answered_pill(p: Palette, about: Option<String>, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    // A reply about the whole squad is in the all-projects conversation, not "in All".
    let words = match &about {
        Some(project) => t!("chat.answered_in", project = project),
        None => t!("chat.answered_in_all").to_owned(),
    };
    let pill = h_flex()
        .id("answered-in")
        .h(px(30.))
        .pl_3()
        .pr_2()
        .gap(px(4.))
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
        .child(words)
        .child(icon(Icon::ChevronRight, 12., p.muted))
        .on_click(cx.listener(move |shell, _, window, cx| shell.focus_project(about.clone(), window, cx)));
    h_flex().flex_none().w_full().pt_1().pb_2().justify_center().child(motion::rise(pill, "answered-in-rise"))
}

/// A stretch about other projects, folded: a quiet line saying how many
/// things were said there; a click opens it in place (and closes it again).
fn elsewhere_row(rows: &Rows, fold: u64, said: usize, open: bool, window: &mut Window, cx: &mut App) -> AnyElement {
    let p = rows.p;
    let progress = motion::openness(("elsewhere-open", fold), open, window, cx);
    let handle = rows.handle.clone();
    let words = if open { t!("chat.elsewhere_open", count = said) } else { t!("chat.elsewhere_closed", count = said) };
    let rule = move || div().flex_1().h(px(1.)).bg(p.hair);
    h_flex()
        .id(("elsewhere", fold))
        .my_1()
        .h(px(28.))
        .gap(px(10.))
        .items_center()
        .cursor_pointer()
        .text_size(px(META))
        .text_color(p.muted)
        .hover(move |s| s.text_color(p.text2))
        .child(rule())
        .child(h_flex().flex_none().gap(px(6.)).items_center().child(words).child(chevron(p.muted, progress)))
        .child(rule())
        .on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.toggle_elsewhere(fold, cx)).ok();
        })
        .into_any_element()
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
        // The person's answers to a member's questions: one line that opens.
        Block::User(text) if crate::decide::decided(text).is_some() => {
            let decided = crate::decide::decided(text).unwrap_or(crate::decide::Decided::Closed);
            super::decide::decided_row(p, rows.tones, rows.handle.clone(), key, &decided, rows.is_open(key, super::decide::PART_DECIDED, false), window, cx)
        }
        Block::User(text) => user_row(rows, ix, key, text, opened(PART_MORE), rows.project_of(ix), cx),
        Block::Reply { text, first, live, worked } => reply_row(rows, ix, key, text, *first, *live, worked.as_ref(), window, cx),
        Block::Steps(lines) => group(rows, key, lines, window, cx),
        Block::News(news) => news_rows(rows, key, news, window, cx),
        Block::YourTurn { ticket, news, live, .. } => match rows.turns.get(ticket).filter(|_| *live) {
            Some(card) => {
                let draw = yourturn::Draw { p, tones: rows.tones, handle: rows.handle.clone(), key, open: rows.is_open(key, yourturn::PART_SENT, false), place: yourturn::Place::Conversation, window: Some(window) };
                yourturn::card(card, draw, cx)
            }
            None => yourturn::past(p, rows.tones, news),
        },
        Block::Worked(worked) => h_flex()
            .h(px(24.))
            .gap(px(6.))
            .items_center()
            .text_size(px(META))
            .text_color(p.muted)
            .child(icon(Icon::Clock, 12., p.muted))
            .child(format!("{} · {}", worked.took(), worked.at))
            .into_any_element(),
        Block::Note(text) => {
            // A ticket's heading (`── T-12 …`).
            let chapter = text.starts_with('─');
            let (markdown, full) = view::prepare(text, false, &|_| true);
            let state = rows.views.borrow_mut().get(view_key(key, 1), &markdown, cx);
            let look = Look { size: SMALL, line: SMALL_LINE, color: p.muted, live: false };
            div()
                .when(chapter, |d| d.mt_3())
                .text_size(px(SMALL))
                .line_height(px(SMALL_LINE))
                .text_color(p.muted)
                .child(view::configure(TextView::new(&state), p, look, key, Arc::new(full)))
                .into_any_element()
        }
        Block::Thought { secs, text, .. } => {
            let open = rows.is_open(key, PART_THOUGHT, false);
            let progress = motion::openness(("thought-open", key), open, window, cx);
            let head = disclosure_head(rows, ("thought", key), PART_THOUGHT, key, false, window, cx)
                .child(div().child(t!("chat.thought_for", secs = secs)))
                .child(chevron(p.muted, progress));
            let body = (progress > 0.).then(|| {
                let state = rows.views.borrow_mut().get(view_key(key, 3), text, cx);
                let look = Look { size: SMALL, line: SMALL_LINE, color: p.muted, live: false };
                motion::Reveal::new(
                    ("thought-body", key),
                    progress,
                    div()
                        .opacity(progress)
                        .pt_1()
                        .ml(px(2.))
                        .pl(px(12.))
                        .border_l_1()
                        .border_color(p.hair)
                        .text_size(px(SMALL))
                        .line_height(px(SMALL_LINE))
                        .text_color(p.muted)
                        .child(view::configure(TextView::new(&state), p, look, key, Arc::default()))
                        .into_any_element(),
                )
            });
            v_flex().child(head).children(body).into_any_element()
        }
        Block::Thinking(latest) => h_flex()
            .h(px(24.))
            .gap_2()
            .items_center()
            .text_size(px(SMALL))
            .text_color(p.muted)
            .child(ShimmerText::new(t!("chat.thinking")).id(("thinking", key)).highlight_color(Hsla::from(p.text)))
            .child(div().min_w_0().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(latest.clone()))
            .into_any_element(),
        Block::Error { message, next } => v_flex()
            .gap_1()
            .text_size(px(BODY))
            .line_height(px(BODY_LINE))
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(div().h(px(BODY_LINE)).flex().items_center().flex_none().child(icon(Icon::Alert, ICON_SM, rows.tones.danger)))
                    .child(div().min_w_0().font_weight(FontWeight::SEMIBOLD).child(SelectableText::new(("error", key), message.clone()))),
            )
            .children(next.clone().map(|next| {
                h_flex().gap_2().pl(px(22.)).text_size(px(SMALL)).child(div().text_color(p.muted).child("→")).child(div().font_family(MONO).font_weight(FontWeight::SEMIBOLD).child(SelectableText::new(("error-next", key), next)))
            }))
            .into_any_element(),
        Block::Landed { branch, stat } => v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_size(px(SMALL))
                    .child(icon(Icon::Check, 13., rows.tones.added))
                    .child(t!("chat.committed"))
                    .child(div().font_family(MONO).font_weight(FontWeight::SEMIBOLD).child(SelectableText::new(("branch", key), branch.clone()))),
            )
            .child(v_flex().pl(px(21.)).font_family(MONO).text_size(px(12.5)).line_height(px(20.)).text_color(p.muted).children(stat.lines().map(|l| div().child(l.trim().to_owned()))))
            .into_any_element(),
        Block::Pending { text, steer, at } => pending_row(rows, key, text, *steer, *at, rows.project_of(ix), cx),
        Block::Elsewhere { key: fold, said, open } => elsewhere_row(rows, *fold, *said, *open, window, cx),
        Block::HandOver { name, why, live } => handed_row(rows, key, name, why, *live),
    }
}

/// A reply: who speaks over the conversation's first, the text full width,
/// and under a turn's last the actions — copy, ask again, how long it worked
/// and when — in sight on the latest, on hover on the rest.
#[allow(clippy::too_many_arguments)]
fn reply_row(rows: &Rows, ix: usize, key: u64, text: &str, first: bool, live: bool, worked: Option<&crate::convo::Worked>, _window: &mut Window, cx: &mut App) -> AnyElement {
    let p = rows.p;
    let look = Look { size: BODY, line: BODY_LINE, color: p.prose, live };
    // A part of the reply's words, drawn in text view `kind`.
    let words = |part: &str, kind: u64, cx: &mut App| {
        let (markdown, full) = view::prepare(part, live, &|n| rows.opened.contains(&(key, n as u32)));
        let state = rows.views.borrow_mut().get(view_key(key, kind), &markdown, cx);
        view::configure(TextView::new(&state), p, look, key, Arc::new(full))
    };
    // A reply that ends asking things in a list: a way to answer them under
    // the list — or, open, the list as a form, the rest of the words around it.
    let body: AnyElement = match rows.asking.as_ref().filter(|a| a.ix == ix && !live) {
        Some(asking) => {
            let span = asking.asked.span.clone();
            let (above, below) = match &asking.open {
                Some(_) => (&text[..span.start], &text[span.end..]),
                None => (&text[..span.end], &text[span.end..]),
            };
            let middle = match &asking.open {
                Some(open) => super::asklist::form(p, rows.tones, asking, open, key, &rows.handle, cx),
                None => super::asklist::button(p, key, &rows.handle),
            };
            v_flex()
                .w_full()
                .when(!above.trim().is_empty(), |c| c.child(words(above, 0, cx)))
                .child(middle)
                .when(!below.trim().is_empty(), |c| c.child(words(below.trim_start(), 1, cx)))
                .into_any_element()
        }
        None => words(text, 0, cx).into_any_element(),
    };
    // A folder of pictures it names: a strip of them under its words.
    let strips: Vec<AnyElement> = if live {
        Vec::new()
    } else {
        let lexical = |written: &str| equipe_core::visuals::markdown::lexical(written, &rows.base).map(|p| p.to_string_lossy().into_owned());
        let pictures: Vec<(String, String)> = equipe_core::visuals::markdown::local_images(text).into_iter().filter_map(|i| Some((lexical(&i.path)?, i.alt))).collect();
        equipe_core::visuals::markdown::folders(text).iter().filter_map(|f| lexical(f)).filter_map(|folder| super::visuals::folder_strip(&rows.shots, key, &folder, &pictures, cx)).collect()
    };
    let group: SharedString = format!("reply-{key}").into();
    // After a group of calls or the news, a little air before the words.
    let after_activity = ix > 0 && matches!(rows.blocks[ix - 1], Block::Steps(_) | Block::News(_) | Block::YourTurn { .. } | Block::HandOver { .. } | Block::Thought { .. } | Block::Worked(_));
    let actions = (!live && rows.turn_ends.contains(&ix)).then(|| {
        let raw = text.to_owned();
        let mut items: Vec<AnyElement> = vec![action_button(p, ("reply-copy", key), Icon::Copy, t!("chat.copy_reply"), move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(raw.clone()))).into_any_element()];
        if rows.retry_at == Some(ix) {
            let handle = rows.handle.clone();
            items.push(
                action_button(p, ("reply-retry", key), Icon::Retry, t!("chat.retry_reply"), move |window, cx| {
                    handle.update(cx, |shell, cx| shell.retry_last(window, cx)).ok();
                })
                .into_any_element(),
            );
        }
        let when = worked.map(|w| {
            let ago = w.unix.map(|at| crate::activity::ago(crate::state::now_unix().saturating_sub(at)));
            let words = match ago {
                Some(ago) => format!("{} · {ago}", w.took()),
                None => format!("{} · {}", w.took(), w.at),
            };
            let line = h_flex().id(("reply-when", key)).pl_1().gap(px(5.)).items_center().child(icon(Icon::Clock, 12., p.muted)).child(words);
            tip(line, p, w.at.clone(), None).into_any_element()
        });
        let latest = rows.latest_reply == Some(ix);
        h_flex()
            .mt(px(6.))
            .ml(px(-5.))
            .gap(px(2.))
            .items_center()
            .text_size(px(META))
            .text_color(p.muted)
            .when(!latest, |r| r.invisible().group_hover(group.clone(), |s| s.visible()))
            .children(items)
            .children(when)
    });
    v_flex()
        .group(group.clone())
        .when(after_activity && !first, |col| col.pt(px(6.)))
        .when(first, |col| {
            col.mt_3().child(
                h_flex()
                    .mb_1()
                    .gap(px(6.))
                    .items_center()
                    .text_size(px(META))
                    .text_color(p.muted)
                    .child(div().size(px(6.)).rounded_full().bg(rows.who.color))
                    .child(rows.who.name.clone()),
            )
        })
        // The measure: a long line is hard to follow back to the next. How
        // wide it is drawn is the room a picture in it has.
        .child(rows.shots.measured(div().w_full().max_w(px(MEASURE)).text_size(px(BODY)).line_height(px(BODY_LINE)).text_color(p.prose).child(body).children(strips), crate::visuals::COLUMN_KEY))
        .children(actions)
        .into_any_element()
}

/// The project a message of yours was about: a dot of its colour and its name.
fn project_chip(p: Palette, name: String, color: Rgba) -> impl IntoElement {
    h_flex()
        .h(px(20.))
        .px(px(7.))
        .gap(px(5.))
        .items_center()
        .rounded(px(R_SM))
        .border_1()
        .border_color(p.hair)
        .text_size(px(11.5))
        .text_color(p.text2)
        .child(super::board::dot(color, 6.))
        .child(name)
}

/// A small icon button under a reply, named in its tooltip.
fn action_button(p: Palette, id: (&'static str, u64), glyph: Icon, label: &'static str, on_click: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
    let b = div()
        .id(id)
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_SM))
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .child(icon(glyph, 13., p.muted))
        .on_click(move |_, window, cx| on_click(window, cx));
    tip(b, p, label, None)
}

/// A chevron that turns from › to ⌄ as `progress` goes from 0 to 1.
fn chevron(color: Rgba, progress: f32) -> impl IntoElement {
    icon(Icon::ChevronRight, 12., color).with_transformation(Transformation::rotate(radians(progress * std::f32::consts::FRAC_PI_2)))
}

/// A focus handle kept for `id` while the row is drawn: Tab and the arrows
/// move through the conversation's disclosures.
fn focus_of(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> FocusHandle {
    window.use_keyed_state(id, cx, |_, cx| cx.focus_handle().tab_stop(true)).read(cx).clone()
}

/// The line that opens and closes a disclosure: a click or Enter/Space
/// toggles `part` of row `key`; Tab, ↓ and ↑ move on; Esc puts the side
/// panel away. A 1px accent ring while it has the keys, after a key moved there.
fn disclosure_head(rows: &Rows, id: (&'static str, u64), part: u32, key: u64, fill: bool, window: &mut Window, cx: &mut App) -> Stateful<Div> {
    let p = rows.p;
    let focus = focus_of(id, window, cx);
    // The ring only when the keys moved here, as the web's :focus-visible: a
    // click keeps the focus, for ↓ and Enter after it, but draws no box.
    let focused = focus.is_focused(window) && window.last_input_was_keyboard();
    let (click, keys) = (rows.handle.clone(), rows.handle.clone());
    let clicked = focus.clone();
    h_flex()
        .id(id)
        .track_focus(&focus)
        .relative()
        .min_h(px(24.))
        .gap(px(6.))
        .items_center()
        .text_size(px(SMALL))
        .line_height(px(SMALL_LINE))
        .text_color(p.muted)
        .cursor_pointer()
        .hover(move |s| if fill { s.bg(p.wash) } else { s.text_color(p.text2) })
        .when(focused, |d| {
            let out = if fill { 0. } else { -6. };
            d.child(div().absolute().top(px(if fill { 0. } else { -1. })).bottom(px(if fill { 0. } else { -1. })).left(px(out)).right(px(out)).rounded(px(R_SM)).border_1().border_color(p.accent))
        })
        .on_click(move |_, window, cx| {
            clicked.focus(window, cx);
            click.update(cx, |shell, cx| shell.chat_toggle(key, part, cx)).ok();
        })
        .on_key_down(move |event: &KeyDownEvent, window, cx| on_disclosure_key(event, &keys, key, part, window, cx))
}

fn on_disclosure_key(event: &KeyDownEvent, handle: &Handle, key: u64, part: u32, window: &mut Window, cx: &mut App) {
    let k = &event.keystroke;
    match k.key.as_str() {
        "enter" | "space" => {
            handle.update(cx, |shell, cx| shell.chat_toggle(key, part, cx)).ok();
        }
        "tab" if k.modifiers.shift => window.focus_prev(cx),
        "tab" | "down" => window.focus_next(cx),
        "up" => window.focus_prev(cx),
        "escape" => {
            handle
                .update(cx, |shell, cx| {
                    if !shell.side_close(cx) {
                        shell.composer.focus(window, cx);
                    }
                })
                .ok();
        }
        _ => return,
    }
    cx.stop_propagation();
}

/// `text` on one line, cut with an ellipsis, each `T-n` in it a way to that
/// ticket beside the conversation.
fn linked(rows: &Rows, id: impl Into<ElementId>, text: &str, color: Rgba, grow: bool) -> AnyElement {
    let p = rows.p;
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some((at, len)) = crate::convo::ticket_ref(&text[from..]) {
        ranges.push(from + at..from + at + len);
        from += at + len;
    }
    let tickets: Vec<u32> = ranges.iter().filter_map(|r| text[r.start + 2..r.end].parse().ok()).collect();
    let style = HighlightStyle { color: Some(p.text.into()), underline: Some(UnderlineStyle { thickness: px(1.), color: Some(p.hair.into()), wavy: false }), ..Default::default() };
    let styled = StyledText::new(text.to_owned()).with_highlights(ranges.iter().map(|r| (r.clone(), style)));
    let handle = rows.handle.clone();
    // Each number's card on the pointer.
    let (layout, hovered, named) = (styled.layout().clone(), ranges.clone(), tickets.clone());
    div()
        .min_w_0()
        .when(grow, |d| d.flex_1())
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_color(color)
        .child(
            InteractiveText::new(id, styled)
                .on_click(ranges, move |n, _, cx| {
                    if let Some(&ticket) = tickets.get(n) {
                        cx.stop_propagation();
                        handle.update(cx, |shell, cx| shell.side_open(ticket, cx)).ok();
                    }
                })
                .on_hover(move |ix, event, _, cx| super::ticket_card::text_hover(&layout, &hovered, &named, ix, &event, cx)),
        )
        .into_any_element()
}

/// The ticket a line names first, for its card on Space.
fn named_ticket(text: Option<String>) -> Option<u32> {
    let text = text?;
    let (at, len) = crate::convo::ticket_ref(&text)?;
    text[at + 2..at + len].parse().ok()
}

/// A group of tool calls: one quiet line — the call under way, the one call,
/// or what they did counted — opening to a card of the calls, each a line
/// that opens to its command, output or change. A failed call is a group of
/// its own, open, in red.
fn group(rows: &Rows, key: u64, lines: &[ToolLine], window: &mut Window, cx: &mut App) -> AnyElement {
    let p = rows.p;
    let t = rows.tones;
    let status = activity::status(lines);
    let failed = status == Status::Failed;
    let single = lines.len() == 1;
    // One call: the group is the call, and opens to what it has.
    let openable = !single || activity::has_more(&lines[0]);
    let open = openable && (rows.expanded || rows.is_open(key, PART_GROUP, failed));
    let progress = motion::openness(("group-open", key), open, window, cx);
    let tone = if failed { t.danger } else { p.muted };
    let mark: Option<AnyElement> = match status {
        Status::Running if !cx.reduce_motion() => Some(motion::spinner(12., p.muted)),
        Status::Running => Some(div().child("◌").into_any_element()),
        Status::Failed => Some(icon(Icon::Alert, 13., t.danger).into_any_element()),
        _ => None,
    };
    let title: AnyElement = match activity::title(lines) {
        activity::Title::Call { phrase, object } => h_flex()
            .min_w_0()
            .gap(px(6.))
            .child(div().flex_none().child(phrase))
            .child(linked(rows, ("group-object", key), &object, if failed { t.danger } else { p.text2 }, false))
            .into_any_element(),
        activity::Title::Summary(summary) => div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(summary).into_any_element(),
    };
    let one = single.then(|| &lines[0]);
    let head = disclosure_head(rows, ("group", key), PART_GROUP, key, false, window, cx)
        .text_color(tone)
        .children(mark)
        .child(title)
        .children(one.and_then(diff_stat).map(|(a, r)| stat_chip(rows, a, r)))
        .children(one.and_then(|l| l.exit.filter(|_| failed)).map(|code| div().flex_none().px_1().rounded(px(4.)).bg(t.danger_bg).text_size(px(META)).text_color(t.danger).child(t!("chat.exit_code", code = code))))
        .children(activity::total_ms(lines).map(|ms| div().flex_none().text_size(px(META)).text_color(p.muted).child(tools::duration(ms))))
        // A call under way counts its seconds, so a long one doesn't look stuck.
        .children(
            rows.step_since
                .filter(|_| status == Status::Running)
                .map(|since| since.elapsed())
                .filter(|took| took.as_secs() >= 1)
                .map(|took| div().flex_none().text_size(px(META)).text_color(p.muted).child(crate::convo::elapsed(took))),
        )
        .when(openable, |h| h.child(chevron(p.muted, progress)));
    let head = if !openable { head.cursor_default() } else { head };
    let head = super::ticket_card::space_opens(head, named_ticket(one.map(|l| l.text())));
    let body = (progress > 0.).then(|| {
        let inner = match one {
            Some(line) => call_body(rows, key, 0, line, failed, window, cx),
            None => card(rows, key, lines, window, cx),
        };
        motion::Reveal::new(("group-body", key), progress, div().opacity(progress).pt_1().pb_1().child(inner).into_any_element())
    });
    v_flex().child(head).children(body).into_any_element()
}

/// `+12 −3`, an edit's change counted.
fn diff_stat(line: &ToolLine) -> Option<(usize, usize)> {
    let card = tools::diff_card(line.diff.as_deref()?)?;
    Some((card.added, card.removed))
}

fn stat_chip(rows: &Rows, added: usize, removed: usize) -> impl IntoElement {
    h_flex().flex_none().gap_1().font_family(MONO).text_size(px(11.5)).child(div().text_color(rows.tones.added).child(format!("+{added}"))).child(div().text_color(rows.tones.removed).child(format!("−{removed}")))
}

/// The calls of an open group, one line each, hairlines between.
fn card(rows: &Rows, key: u64, lines: &[ToolLine], window: &mut Window, cx: &mut App) -> AnyElement {
    let p = rows.p;
    v_flex()
        .rounded(px(R_LG))
        .border_1()
        .border_color(p.hair)
        .children(lines.iter().enumerate().map(|(at, line)| call_row(rows, key, at as u32, line, at > 0, window, cx)))
        .into_any_element()
}

/// A call in an open group: its verb phrase, what it acted on, how long it
/// took; opening it shows the rest.
fn call_row(rows: &Rows, key: u64, at: u32, line: &ToolLine, rule: bool, window: &mut Window, cx: &mut App) -> AnyElement {
    let p = rows.p;
    let t = rows.tones;
    let more = activity::has_more(line);
    let failed = line.status == Status::Failed;
    let running = line.status == Status::Running;
    let part = PART_ROW + at;
    let open = more && rows.is_open(key, part, failed);
    let progress = motion::openness(("call-open", key << 8 | at as u64), open, window, cx);
    let head = disclosure_head(rows, ("call", key << 8 | at as u64), part, key, true, window, cx)
        .h(px(32.))
        .px_3()
        .gap_2()
        .when(!more, |h| h.cursor_default())
        .children(running.then(|| motion::spinner(11., p.muted)))
        .children(failed.then(|| icon(Icon::Alert, 12., t.danger)))
        .child(div().flex_none().text_color(if failed { t.danger } else { p.muted }).child(activity::phrase(&line.verb, running)))
        .child(linked(rows, ("call-object", key << 8 | at as u64), &line.text(), p.text2, true))
        .children(diff_stat(line).map(|(a, r)| stat_chip(rows, a, r)))
        .children(line.exit.filter(|_| failed).map(|code| div().flex_none().px_1().rounded(px(4.)).bg(t.danger_bg).text_size(px(META)).text_color(t.danger).child(t!("chat.exit_code", code = code))))
        .children(line.ms.filter(|_| !running).map(|ms| div().flex_none().text_size(px(META)).text_color(p.muted).child(tools::duration(ms))))
        .when(more, |h| h.child(chevron(p.muted, progress)));
    let head = super::ticket_card::space_opens(head, named_ticket(Some(line.text())));
    let body = (progress > 0.).then(|| motion::Reveal::new(("call-body", key << 8 | at as u64), progress, div().opacity(progress).px_3().pb_2().child(call_body(rows, key, at, line, failed, window, cx)).into_any_element()));
    v_flex().when(rule, |c| c.border_t_1().border_color(p.hair)).child(head).children(body).into_any_element()
}

/// What a call shows open: a command whole and what it printed (mono, in a
/// box that scrolls past a height), an edit's change, a long comment or
/// task as text.
fn call_body(rows: &Rows, key: u64, at: u32, line: &ToolLine, failed: bool, _window: &mut Window, cx: &mut App) -> AnyElement {
    let p = rows.p;
    let t = rows.tones;
    let id = key << 8 | at as u64;
    let mut parts: Vec<AnyElement> = Vec::new();
    let command = line.verb == "run";
    if let Some(full) = line.full.as_ref().filter(|f| f.chars().count() > 80 || f.contains('\n')) {
        if command || line.output.is_some() {
            parts.push(
                div()
                    .id(("call-command", id))
                    .max_h(px(160.))
                    .overflow_y_scroll()
                    .font_family(MONO)
                    .text_size(px(META))
                    .line_height(px(META_LINE))
                    .text_color(p.text2)
                    .child(SelectableText::new(("call-command-text", id), format!("$ {full}")))
                    .into_any_element(),
            );
        } else {
            let state = rows.views.borrow_mut().get(hash_of(("call-text", id)), full, cx);
            let look = Look { size: SMALL, line: SMALL_LINE, color: p.text2, live: false };
            parts.push(div().text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.text2).child(view::configure(TextView::new(&state), p, look, key, Arc::default())).into_any_element());
        }
    }
    if let Some((output, skipped)) = &line.output {
        parts.push(
            v_flex()
                .rounded(px(R_SM))
                .bg(if failed { t.danger_bg } else { p.code_bg })
                .px_3()
                .py(px(6.))
                .font_family(MONO)
                .text_size(px(META))
                .line_height(px(META_LINE))
                .text_color(if failed { t.danger } else { p.text2 })
                .when(*skipped > 0, |c| c.child(div().text_color(p.muted).child(t!("chat.output_before", count = *skipped))))
                .child(div().id(("call-output", id)).max_h(px(220.)).overflow_y_scroll().child(SelectableText::new(("call-output-text", id), output.trim_end().to_owned())))
                .into_any_element(),
        );
    }
    if let Some(card) = line.diff.as_deref().and_then(tools::diff_card) {
        parts.push(diff_lines(rows, id, card, line.path.as_deref()));
    }
    v_flex().gap_2().pt_1().children(parts).into_any_element()
}

/// An edit's change: the file, then its lines, in a box that scrolls.
fn diff_lines(rows: &Rows, id: u64, card: tools::DiffCard, path: Option<&str>) -> AnyElement {
    let p = rows.p;
    let t = rows.tones;
    let full = path.map_or_else(|| card.path.clone(), str::to_owned);
    let chip = plugins::file_chip(p, ("diff-file", id), &relative(&full, &rows.base), std::path::Path::new(&full).exists(), px(12.));
    v_flex()
        .rounded(px(R_SM))
        .bg(p.code_bg)
        .overflow_hidden()
        .child(h_flex().px_2().py(px(4.)).gap_2().text_size(px(META)).child(chip))
        .child(
            div().id(("diff-lines", id)).max_h(px(320.)).overflow_y_scroll().pb_1().children(card.lines.into_iter().map(move |l| {
                let (sign, bg, color) = match l.kind {
                    Kind::Add => ("+", Some(t.added_bg), p.text),
                    Kind::Del => ("−", Some(t.removed_bg), p.text2),
                    Kind::Hunk => ("", None, p.muted),
                    Kind::Note => ("", None, p.muted),
                    Kind::Context => (" ", None, p.muted),
                };
                h_flex()
                    .px_2()
                    .font_family(MONO)
                    .text_size(px(META))
                    .line_height(px(META_LINE))
                    .when_some(bg, |row, bg| row.bg(bg))
                    .text_color(color)
                    .child(div().w(px(14.)).flex_none().text_color(match l.kind {
                        Kind::Add => t.added,
                        Kind::Del => t.removed,
                        _ => p.muted,
                    }).child(sign))
                    .child(div().min_w_0().flex_1().overflow_hidden().whitespace_nowrap().child(if l.text.is_empty() { " ".to_owned() } else { l.text.replace('\t', "    ") }))
            })),
        )
        .into_any_element()
}

/// equipe's news: a line each, its icon saying what it is; more than three
/// in a row fold into one line that counts them.
fn news_rows(rows: &Rows, key: u64, news: &[crate::activity::News], window: &mut Window, cx: &mut App) -> AnyElement {
    let p = rows.p;
    // While every project shows: a dot of the project the news is about.
    let project = rows.project_of((key & 0xFF_FFFF) as usize);
    let line = |n: usize, item: &crate::activity::News, cx: &mut App| {
        let line = h_flex()
            .h(px(24.))
            .gap(px(8.))
            .items_center()
            .text_size(px(SMALL))
            .child(news_icon(rows, item.kind))
            .children(project.as_ref().map(|(name, color)| tip(div().id(("news-project", key << 8 | n as u64)).child(super::board::dot(*color, 6.)), p, name.clone(), None)))
            .child(linked(rows, ("news", key << 8 | n as u64), &item.text, p.muted, false))
            // The disk ran short: the way to make room, in a click.
            .when(item.kind == crate::activity::NewsKind::DiskShort, |r| {
                let handle = rows.handle.clone();
                r.child(
                    div()
                        .id(("news-disk", key << 8 | n as u64))
                        .flex_none()
                        .cursor_pointer()
                        .text_color(p.accent)
                        .font_weight(FontWeight::MEDIUM)
                        .hover(|d| d.underline())
                        .child(format!("· {}", t!("chat.news_disk_clean")))
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            handle.update(cx, |shell, cx| shell.open_disk(window, cx)).ok();
                        }),
                )
            });
        // Work handed in with pictures: small ones under its line.
        let ticket = (item.kind == crate::activity::NewsKind::HandedIn).then(|| crate::convo::ticket_ref(&item.text)).flatten().and_then(|(at, len)| item.text[at + 2..at + len].parse::<u32>().ok());
        let pictures = ticket.and_then(|t| rows.pictures.get(&t).map(|images| (t, images.clone())));
        match pictures.and_then(|(t, images)| super::visuals::strip(&rows.shots, key << 8 | n as u64, t, &images, Some(cx))) {
            Some(strip) => v_flex().child(line).child(div().pl(px(21.)).pt(px(2.)).pb_1().child(strip)).into_any_element(),
            None => line.into_any_element(),
        }
    };
    if news.len() <= crate::activity::NEWS_FOLDS_PAST {
        let lines: Vec<AnyElement> = news.iter().enumerate().map(|(n, item)| line(n, item, cx)).collect();
        return v_flex().children(lines).into_any_element();
    }
    let open = rows.expanded || rows.is_open(key, PART_NEWS, false);
    let progress = motion::openness(("news-open", key), open, window, cx);
    let head = disclosure_head(rows, ("news-head", key), PART_NEWS, key, false, window, cx)
        .gap(px(8.))
        .child(icon(Icon::Inbox, 13., p.muted))
        .child(t!("chat.news_count", count = news.len()))
        .child(chevron(p.muted, progress));
    let lines: Vec<AnyElement> = if progress > 0. { news.iter().enumerate().map(|(n, item)| line(n, item, cx)).collect() } else { Vec::new() };
    let body = (progress > 0.).then(|| motion::Reveal::new(("news-body", key), progress, v_flex().opacity(progress).pl(px(21.)).children(lines).into_any_element()));
    v_flex().child(head).children(body).into_any_element()
}

/// A member handed the person its browser: who, and what it asked in its
/// own words; on its latest, while the browser is in their hands, the way to
/// it — and to give it back, which starts the member again.
fn handed_row(rows: &Rows, key: u64, name: &str, why: &str, live: bool) -> AnyElement {
    let p = rows.p;
    let lane = format!("{}/{name}", rows.squad);
    let held = live && rows.held.contains(name);
    let open = {
        let (handle, lane) = (rows.handle.clone(), lane.clone());
        crate::ui::button(p, ("handed-open", key), if held { crate::ui::Kind::Primary } else { crate::ui::Kind::Secondary }, t!("chat.handed_open")).icon(Icon::Globe).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.reveal_handed(&lane, cx)).ok();
        })
    };
    let back = {
        let (handle, lane) = (rows.handle.clone(), lane.clone());
        crate::ui::button(p, ("handed-back", key), crate::ui::Kind::Secondary, t!("chat.handed_back")).tip(t!("chat.handed_back_tip", name = name), None).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.give_back_handed(&lane, cx)).ok();
        })
    };
    v_flex()
        .gap(px(2.))
        .child(
            h_flex()
                .min_h(px(28.))
                .gap(px(8.))
                .items_center()
                .text_size(px(SMALL))
                .child(icon(Icon::Globe, 13., if live { p.accent } else { p.muted }))
                .child(div().flex_1().min_w_0().font_weight(FontWeight::MEDIUM).text_color(if live { p.text } else { p.muted }).child(t!("chat.handed", name = name)))
                .when(live, |r| r.child(open))
                .when(held, |r| r.child(back))
                .when(live && !held, |r| r.child(div().text_size(px(12.)).text_color(p.muted).child(t!("chat.handed_done")))),
        )
        .child(div().pl(px(21.)).text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.muted).child(SelectableText::new(("handed-why", key), why.to_owned())))
        .into_any_element()
}

fn news_icon(rows: &Rows, kind: crate::activity::NewsKind) -> impl IntoElement {
    use crate::activity::NewsKind as K;
    let (p, t) = (rows.p, rows.tones);
    let (glyph, color) = match kind {
        K::HandedIn => (Icon::Inbox, p.accent),
        K::Accepted | K::Merged => (Icon::Check, t.added),
        K::Proposed => (Icon::Lightbulb, t.warning),
        K::Waiting => (Icon::Clock, t.warning),
        K::Cancelled => (Icon::XCircle, p.muted),
        K::Reopened | K::Resumed | K::SentBack => (Icon::Retry, p.muted),
        K::Left => (Icon::Users, p.muted),
        K::CannotStart => (Icon::Alert, t.danger),
        K::DeskSaved => (Icon::Pencil, p.muted),
        K::YouTold => (Icon::Message, p.muted),
        K::Stale => (Icon::Info, p.muted),
        K::DiskShort => (Icon::Warning, t.warning),
        K::DiskRoom => (Icon::Check, t.added),
    };
    icon(glyph, 13., color)
}

/// A steer or a memo waiting for the member: on the right as your message
/// is, quieter — a dashed outline — and under it what it waits for and when
/// it was said, then on hover (or reached with ↑) copy, take it back and
/// send it now. Taken back, it fades out.
fn pending_row(rows: &Rows, key: u64, raw: &str, steer: bool, at: u64, project: Option<(String, Rgba)>, cx: &mut App) -> AnyElement {
    let p = rows.p;
    let parts = crate::sent::parts(raw);
    let files = attached::attachments(&rows.attached, key, &parts.tags, parts.more, true, cx);
    let picked = rows.picked.as_ref().is_some_and(|(t, a)| t == raw && *a == at);
    let leaving = rows.leaving.contains(raw);
    let group: SharedString = format!("pending-{key}").into();
    let state = if steer { t!("chat.pending_steer") } else { t!("chat.pending_memo") };
    let when = (at > 0).then(|| crate::activity::ago(crate::state::now_unix().saturating_sub(at)));
    let copied = if parts.text.is_empty() { raw.to_owned() } else { parts.text.clone() };
    let copy = tool_button(p, ("pending-copy", key), Icon::Copy, t!("chat.copy_reply"), move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(copied.clone())));
    let (handle, text) = (rows.handle.clone(), raw.to_owned());
    let back = keyed_button(p, ("pending-back", key), Icon::Withdraw, t!("chat.pending_withdraw"), Some("⌫"), move |window, cx| {
        handle.update(cx, |shell, cx| shell.withdraw_pending(text.clone(), window, cx)).ok();
    });
    let handle = rows.handle.clone();
    let now_tip = if rows.waiting > 1 { t!("chat.pending_send_now_all", count = rows.waiting) } else { t!("chat.pending_send_now_tip").to_owned() };
    let now = keyed_button(p, ("pending-now", key), Icon::SendNow, now_tip, Some("⌘↵"), move |_, cx| {
        handle.update(cx, |shell, cx| shell.send_pending_now(cx)).ok();
    });
    let bubble = (!parts.text.is_empty()).then(|| {
        div()
            .max_w(gpui_kit::relative(BUBBLE_SHARE))
            .px(px(BUBBLE_X))
            .py(px(BUBBLE_Y))
            .rounded(px(crate::theme::R_LG))
            .border_1()
            .border_dashed()
            .border_color(if picked { p.accent } else { p.hair })
            .when(picked, |b| b.bg(p.wash))
            .text_size(px(BODY))
            .line_height(px(BODY_LINE))
            .text_color(p.text2)
            .child(SelectableText::new(("pending", key), parts.text.clone()))
    });
    let label = h_flex()
        .gap(px(5.))
        .items_center()
        .child(div().size(px(6.)).rounded_full().bg(if steer { p.accent } else { p.muted }))
        .child(state)
        .children(when.map(|w| div().child(format!("· {w}"))));
    let row = v_flex()
        .group(group.clone())
        .w_full()
        .mt_4()
        .gap_1p5()
        .items_end()
        .children(files)
        .children(bubble)
        // Beneath, flush right under the bubble: the actions (on hover), what
        // it waits for and when, the project it was said about.
        .child(
            h_flex()
                .h(px(24.))
                .gap(px(6.))
                .items_center()
                .text_size(px(META))
                .text_color(p.muted)
                .child(h_flex().gap(px(2.)).items_center().when(!picked, |r| r.invisible().group_hover(group.clone(), |s| s.visible())).child(copy).child(back).child(now))
                .child(label)
                .children(project.map(|(name, color)| project_chip(p, name, color))),
        );
    if leaving {
        return row.with_animation(("pending-out", key), Animation::new(motion::BASE).with_easing(motion::ease_out), |el, t| el.opacity(1. - t)).into_any_element();
    }
    row.into_any_element()
}

/// An icon button under a waiting bubble, named — and its key said — in its tooltip.
fn keyed_button(p: Palette, id: (&'static str, u64), glyph: Icon, label: impl Into<SharedString>, keys: Option<&str>, on_click: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
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
    tip(b, p, label, keys)
}

/// Your message, as Claude's apps show it: a quiet bubble on the right, as
/// wide as your words up to most of the column, no name over it; what it
/// attached just above it, its start only when it is long ("더 보기"), and on
/// hover a small row of actions beneath.
fn user_row(rows: &Rows, ix: usize, key: u64, raw: &str, open: bool, project: Option<(String, Rgba)>, cx: &mut App) -> AnyElement {
    let p = rows.p;
    let parts = crate::sent::parts(raw);
    // Asked about words of the conversation: the quote over the words.
    let split = crate::quote::split(&parts.text);
    let quoted = split.as_ref().map(|(quote, _)| super::quote::in_bubble(p, key, ix, quote, rows.handle.clone()));
    let text = split.as_ref().map_or(parts.text.as_str(), |(_, rest)| rest.as_str());
    // The pictures it embeds show under it, their markup out of its words.
    let embedded = equipe_core::visuals::markdown::local_images(text);
    let lexical = |written: &str| equipe_core::visuals::markdown::lexical(written, &rows.base).map(|p| p.to_string_lossy().into_owned());
    let pictures: Vec<(String, String)> = embedded.iter().filter_map(|i| Some((lexical(&i.path)?, i.alt.clone()))).collect();
    let without: String;
    let text = if embedded.is_empty() {
        text
    } else {
        let mut rest = String::with_capacity(text.len());
        let mut at = 0;
        for image in &embedded {
            rest.push_str(&text[at..image.span.start]);
            at = image.span.end;
        }
        rest.push_str(&text[at..]);
        without = rest.trim().to_owned();
        without.as_str()
    };
    let shown_pictures: Vec<AnyElement> = pictures.iter().map(|(path, alt)| super::visuals::inline(&rows.shots, key, path, alt, &pictures, false, cx)).collect();
    let long = text.lines().count() > USER_LINES || text.chars().count() > USER_LINES * 36;
    let clamped = long && !open;
    let look = Look { size: BODY, line: BODY_LINE, color: p.prose, live: false };
    let body = (!text.is_empty()).then(|| {
        let state = rows.views.borrow_mut().get(view_key(key, 2), &escape_markdown(text), cx);
        TextView::new(&state).style(view::style(p, &look)).selectable(true).on_link_click(|url, event, _, cx| view::follow(url, event.modifiers(), 0, cx)).when(clamped, |v| v.max_lines(USER_LINES))
    });
    let files = attached::attachments(&rows.attached, key, &parts.tags, parts.more, true, cx);
    let group: SharedString = format!("user-{key}").into();
    // What is copied is what is shown: the words (the tags only when there are none).
    let copied = if parts.text.is_empty() { raw.to_owned() } else { parts.text.clone() };
    let copy = tool_button(p, ("user-copy", key), Icon::Copy, t!("chat.copy_reply"), move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(copied.clone())));
    let handle = rows.handle.clone();
    let more = long.then(|| {
        div()
            .id(("user-more", key))
            .pt_1()
            .text_size(px(SMALL))
            .text_color(p.accent)
            .cursor_pointer()
            .child(if open { t!("chat.show_less") } else { t!("chat.show_more") })
            .on_click(move |_, _, cx| {
                handle.update(cx, |shell, cx| shell.chat_toggle(key, PART_MORE, cx)).ok();
            })
    });
    let bubble = (body.is_some() || quoted.is_some()).then(|| {
        v_flex()
            .max_w(gpui_kit::relative(BUBBLE_SHARE))
            .px(px(BUBBLE_X))
            .py(px(BUBBLE_Y))
            .gap(px(6.))
            .rounded(px(crate::theme::R_LG))
            .bg(p.wash)
            .text_size(px(BODY))
            .line_height(px(BODY_LINE))
            .text_color(p.prose)
            .children(quoted)
            .children(body)
            .children(more)
    });
    v_flex()
        .group(group.clone())
        .w_full()
        .mt_4()
        .gap_1p5()
        .items_end()
        .children(files)
        .when(!shown_pictures.is_empty(), |c| c.child(v_flex().items_end().gap_1().children(shown_pictures)))
        .children(bubble)
        // Beneath, flush right under the bubble: copy, shown on hover — its
        // room kept so nothing moves — then the project it was about, while
        // every project shows.
        .child(
            h_flex()
                .h(px(24.))
                .gap(px(6.))
                .items_center()
                .child(div().invisible().group_hover(group, |s| s.visible()).child(copy))
                .children(project.map(|(name, color)| project_chip(p, name, color))),
        )
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
        // A ticket number is a link to it, as in a reply.
        let mut rest = trimmed;
        while !rest.is_empty() {
            let (plain, link) = match crate::convo::ticket_ref(rest) {
                Some((at, len)) => (&rest[..at], Some(&rest[at..at + len])),
                None => (rest, None),
            };
            for c in plain.chars() {
                if c.is_ascii_punctuation() {
                    out.push('\\');
                }
                out.push(c);
            }
            if let Some(link) = link {
                out.push_str(&format!("[{}](ticket:{})", link.replace('-', "\\-"), &link[2..]));
            }
            rest = &rest[plain.len() + link.map_or(0, str::len)..];
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

/// Three dots that pulse while a reply streams, over the composer.
/// Read back up the conversation: a pill over the composer to the latest,
/// fading and rising into place.
fn jump_pill(p: Palette, scroller: Entity<MessageScrollerState>) -> impl IntoElement {
    let pill = h_flex()
        .id("jump-to-latest")
        .h(px(32.))
        .pl(px(10.))
        .pr_3()
        .gap(px(6.))
        .items_center()
        .rounded_full()
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .shadow(motion::float_shadow(0.8))
        .text_size(px(13.))
        .text_color(p.text)
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .child(icon(Icon::ArrowDown, 14., p.text2))
        .child(t!("chat.jump_to_bottom"))
        .on_click(move |_, _, cx| scroller.update(cx, |state, cx| state.scroll_to_end(cx)));
    div().absolute().bottom(px(12.)).left_0().right_0().flex().justify_center().child(motion::rise(pill, "jump-to-latest-in"))
}

fn pulse_pill(p: Palette) -> impl IntoElement {
    let dot = move |n: usize| {
        div().size(px(5.)).rounded_full().bg(p.muted).with_animation(
            ("pulse-dot", n),
            // A soft pulse needs no more than this many steps a second; at the
            // display's rate it redrew the whole window 60–120 times a second
            // for as long as a reply streamed (`docs/PERF-2026-10.md`).
            Animation::new(std::time::Duration::from_millis(1200)).repeat().with_max_fps(20.),
            move |d, t| {
                // Each dot a third of the way behind the one before.
                let phase = (t - n as f32 / 3.).rem_euclid(1.);
                let lift = (phase * std::f32::consts::TAU).sin().max(0.);
                d.opacity(0.35 + 0.65 * lift)
            },
        )
    };
    div().absolute().bottom(px(12.)).left_0().right_0().flex().justify_center().child(
        h_flex()
            .id("replying")
            .h(px(24.))
            .px_3()
            .gap(px(4.))
            .items_center()
            .rounded_full()
            .bg(p.popover())
            .border_1()
            .border_color(p.hair)
            .shadow(motion::float_shadow(0.5))
            .children((0..3).map(dot)),
    )
}

/// The ticket opened beside the conversation: back, its number and title, to
/// the board, close; then what the board's peek shows of it.
fn side_panel(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let id = *shell.chat.side.last()?;
    let p = shell.palette;
    let ticket = shell.model.tickets.iter().find(|t| t.id == id)?;
    let button = |name: &'static str, glyph: Icon, label: &'static str| {
        let b = div().id(name).size(px(26.)).flex().flex_none().items_center().justify_center().rounded(px(R_SM)).cursor_pointer().hover(move |s| s.bg(p.wash)).child(icon(glyph, ICON_SM, p.text2));
        tip(b, p, label, None)
    };
    let back = (shell.chat.side.len() > 1).then(|| button("side-back", Icon::ArrowLeft, t!("chat.side_back")).on_click(cx.listener(|this, _, _, cx| this.side_back(cx))));
    let head = h_flex()
        .h(px(44.))
        .flex_none()
        .px_3()
        .gap_1()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .children(back)
        .child(
            h_flex()
                .min_w_0()
                .flex_1()
                .px_1()
                .gap_2()
                .items_baseline()
                .text_size(px(SMALL))
                .child(div().flex_none().font_family(MONO).text_size(px(META)).text_color(p.muted).child(format!("T-{id}")))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().font_weight(FontWeight::SEMIBOLD).child(ticket.title.clone())),
        )
        .child(button("side-expand", Icon::Expand, t!("chat.side_expand")).on_click(cx.listener(move |this, _, _, cx| {
            this.side_close(cx);
            this.open_ticket(id, cx);
        })))
        .child(button("side-close", Icon::X, t!("chat.side_close")).on_click(cx.listener(|this, _, _, cx| {
            this.side_close(cx);
        })));
    let panel = v_flex()
        .id("side-panel")
        .w(px(SIDE))
        .flex_none()
        .h_full()
        .bg(p.bg)
        .border_l_1()
        .border_color(p.hair)
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            if event.keystroke.key == "escape" && this.side_close(cx) {
                cx.stop_propagation();
            }
        }))
        .child(head)
        .child(div().flex_1().min_h_0().child(super::peek::side(shell, id, cx)));
    Some(motion::fade(panel, "side-in", motion::BASE))
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
        .child(div().flex_none().pr_1().text_size(px(META)).text_color(p.muted).child(count))
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
    /// Open or close the group of calls at row `ix`, or its call `call` (the
    /// snapshot script's).
    #[cfg_attr(not(feature = "snapshot"), allow(dead_code))]
    pub fn chat_turn(&mut self, ix: usize, call: Option<u32>, cx: &mut Context<Self>) {
        let row = self.chat.serial << 24 | ix as u64;
        let part = match (call, self.chat.blocks.get(ix)) {
            (Some(n), _) => PART_ROW + n,
            (None, Some(Block::Thought { .. })) => PART_THOUGHT,
            (None, Some(Block::News(_))) => PART_NEWS,
            (None, Some(Block::YourTurn { .. })) => yourturn::PART_SENT,
            (None, Some(Block::User(text))) if crate::decide::decided(text).is_some() => super::decide::PART_DECIDED,
            _ => PART_GROUP,
        };
        self.chat_toggle(row, part, cx);
    }

    /// Open or close the latest answered card of `ticket` (the snapshot script's).
    #[cfg(feature = "snapshot")]
    pub fn chat_open_turn(&mut self, ticket: u32, cx: &mut Context<Self>) {
        let at = self.chat.blocks.iter().rposition(|b| matches!(b, Block::YourTurn { ticket: t, .. } if *t == ticket));
        if let Some(ix) = at {
            self.chat_turn(ix, None, cx);
        }
    }

    /// Open or close the conversation's latest decided row (the snapshot script's).
    #[cfg(feature = "snapshot")]
    pub fn chat_open_decided(&mut self, cx: &mut Context<Self>) {
        let at = self.chat.blocks.iter().rposition(|b| matches!(b, Block::User(text) if crate::decide::decided(text).is_some()));
        if let Some(ix) = at {
            self.chat_turn(ix, None, cx);
        }
    }

    /// Open `ticket` beside the conversation.
    pub fn side_open(&mut self, ticket: u32, cx: &mut Context<Self>) {
        self.chat.side.retain(|t| *t != ticket);
        self.chat.side.push(ticket);
        cx.notify();
    }

    /// Back to the ticket shown before.
    pub fn side_back(&mut self, cx: &mut Context<Self>) {
        self.chat.side.pop();
        cx.notify();
    }

    /// Put the side panel away; whether it was there.
    pub fn side_close(&mut self, cx: &mut Context<Self>) -> bool {
        let was = !self.chat.side.is_empty();
        self.chat.side.clear();
        cx.notify();
        was
    }

    pub fn toggle_folds(&mut self, cx: &mut Context<Self>) {
        self.chat.expanded = !self.chat.expanded;
        self.chat_sync(cx);
        cx.notify();
    }

    /// Open or close a stretch of other projects folded in the conversation.
    pub fn toggle_elsewhere(&mut self, fold: u64, cx: &mut Context<Self>) {
        if !self.chat.elsewhere.remove(&fold) {
            self.chat.elsewhere.insert(fold);
        }
        self.model.revision += 1;
        self.chat_sync(cx);
        cx.notify();
    }

    /// The conversation follows the project in focus, or shows whole (settings ▸ 일반).
    pub fn set_follow_project(&mut self, on: bool, cx: &mut Context<Self>) {
        self.remembered.whole_conversation = !on;
        self.remembered.save(&equipe_providers::credentials::equipe_home());
        self.chat.follow = on;
        self.chat_sync(cx);
        cx.notify();
    }

    /// The conversation brought up to what the model shows, its cards with it.
    pub fn chat_sync(&mut self, cx: &mut Context<Self>) {
        let looks = yourturn::looks(self, cx);
        self.chat.sync(&self.model, &looks, cx);
    }

    /// A card's box changed: its rows are measured again.
    pub fn chat_remeasure_turn(&mut self, id: u32, cx: &mut Context<Self>) {
        self.chat.remeasure_turn(id, cx);
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

    /// Escape: the lightbox goes, then the find bar, then the side panel.
    pub fn chat_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.chat.gallery.take().is_some() || self.chat.lightbox.take().is_some() {
            cx.notify();
            return true;
        }
        if !self.chat.find.open && self.side_close(cx) {
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
    use super::{Block, escape_markdown, find_hits, occurrences, turn_ends};

    #[test]
    fn your_words_show_as_typed() {
        assert_eq!(escape_markdown("# not a heading"), "\\# not a heading");
        assert_eq!(escape_markdown("a *b* [c](d)"), "a \\*b\\* \\[c\\]\\(d\\)");
        assert_eq!(escape_markdown("one\ntwo\n\nthree"), "one\\\ntwo\n\nthree");
        assert_eq!(escape_markdown("    indented"), "\u{A0}\u{A0}\u{A0}\u{A0}indented");
        assert_eq!(escape_markdown("1. first"), "1\\. first");
        assert_eq!(escape_markdown("한국어 그대로"), "한국어 그대로");
        assert_eq!(escape_markdown("T-12 먼저, 그다음 T-3."), "[T\\-12](ticket:12) 먼저\\, 그다음 [T\\-3](ticket:3)\\.");
        assert_eq!(escape_markdown("AT-1 no"), "AT\\-1 no", "not a word of its own");
    }

    #[test]
    fn find_counts_every_match_by_row() {
        let blocks = vec![
            Block::User("Fix the header".into()),
            Block::Reply { text: "The header and the HEADER.".into(), first: true, live: false, worked: None },
            Block::Thought { n: 0, secs: 2, text: "header".into() },
            Block::Note("footer".into()),
        ];
        assert_eq!(find_hits(&blocks, "header"), [(0, 0), (1, 0), (1, 1)]);
        assert_eq!(find_hits(&blocks, "  "), []);
        assert_eq!(occurrences("A header, a Header", "header"), [2..8, 12..18]);
    }

    #[test]
    fn the_actions_go_under_the_reply_that_ends_a_turn() {
        let reply = |t: &str| Block::Reply { text: t.into(), first: false, live: false, worked: None };
        let blocks = vec![Block::User("go".into()), reply("looking"), Block::Steps(vec![]), reply("done"), Block::User("more".into()), reply("ok"), Block::Note("n".into())];
        let mut ends: Vec<usize> = turn_ends(&blocks).into_iter().collect();
        ends.sort();
        assert_eq!(ends, [3, 5]);
    }
}
