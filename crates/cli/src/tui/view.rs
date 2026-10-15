//! Drawing the app — nothing here decides anything.
//!
//! Styled per docs/BRAND.md like the rest of the CLI: monochrome, one cool
//! accent for what is alive or needs the person, weight and dimming for
//! hierarchy. Space, not boxes, separates things — the way the agent consoles
//! people live in (Codex, Claude Code, opencode, Crush) draw them: a two-column
//! gutter, a blank line between turns, prose no wider than it reads well, one
//! hairline between the squad and the conversation, and a single accent bar for
//! the one thing that waits on the person.

use std::collections::HashMap;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use equipe_core::board::project_name;
use equipe_core::i18n::label;
use equipe_core::protocol::{Entry, Estimate, SessionId, TicketId, TicketStatus};

use super::app::{Agent, AgentState, App, BoardMenu, BoardRule, Divider, Hit, Pane, Section, Side, Spot, TextView, elapsed, needs_way, relative, section_of, status_word, tool_verb};
use super::markdown;
use crate::ui;
use equipe_core::t;

/// How many columns `text` takes on screen — a Korean letter takes two.
fn cols(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

/// `text` with spaces after it up to `width` columns: `{:<width$}` by what
/// shows on screen, so double-width words line up too.
fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(cols(text))))
}

/// A translated sentence around one `{name}` in it: what comes before the
/// value and what after, so the value can take its own style.
fn around<'a>(template: &'a str, name: &str) -> (&'a str, &'a str) {
    template.split_once(&format!("{{{name}}}")).unwrap_or((template, ""))
}

/// Prose is laid out no wider than this: past it, lines get hard to read.
const PROSE_MAX: usize = 96;

/// The squad pane's width — wider on a wide terminal — and the gap holding the
/// hairline beside it.
/// The squad pane's narrowest and, as a share of the body, widest.
const SQUAD_MIN: u16 = 14;
/// The side list's width unless dragged: room for a ticket's title.
const SIDE_FIT: u16 = 38;
const GAP: u16 = 4;

/// Below this width the squad pane gives way to the conversation.
const SQUAD_MIN_TOTAL: u16 = 72;

/// Styles, honouring NO_COLOR, terminals without truecolor, and light themes.
#[derive(Clone, Copy)]
pub struct Theme {
    color: bool,
    truecolor: bool,
    /// The terminal's background is light: the pale accent would vanish on it.
    light: bool,
    /// The terminal's background colour, when it told us.
    background: Option<(u8, u8, u8)>,
}

impl Theme {
    /// `background`: the terminal's answer to "what is your background colour",
    /// if it gave one.
    pub fn detect(background: Option<(u8, u8, u8)>) -> Self {
        let (color, truecolor) = ui::out().palette();
        let light = match background {
            Some((r, g, b)) => (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0 > 0.5,
            None => light_background(),
        };
        Self { color, truecolor, light, background }
    }

    #[cfg(test)]
    pub fn plain() -> Self {
        Self { color: false, truecolor: false, light: false, background: None }
    }

    /// A faint wash over the background for the person's own messages: white at
    /// 7% on a dark terminal, black at 5% on a light one. Only when the terminal
    /// said what its background is — a guess would show as a grey slab.
    fn tint(self) -> Option<Color> {
        let (r, g, b) = self.background.filter(|_| self.color && self.truecolor)?;
        let (toward, amount) = if self.light { (0.0, 0.05) } else { (255.0, 0.07) };
        let mix = |c: u8| (c as f32 + (toward - c as f32) * amount).round() as u8;
        Some(Color::Rgb(mix(r), mix(g), mix(b)))
    }

    /// Time since the first frame drawn: every animation runs off one clock.
    fn since_start(now: Instant) -> u128 {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        now.saturating_duration_since(*START.get_or_init(|| now)).as_millis()
    }

    /// The working mark: a star that opens and closes — `· ✢ ✳ ✶ ✻ ✽` and back —
    /// while its colour breathes from dim to the accent and back every two
    /// seconds, eased. Without truecolor the colour steps; without colour the
    /// star still turns.
    fn spinner(self, now: Instant) -> Span<'static> {
        const FRAMES: [&str; 10] = ["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"];
        let ms = Self::since_start(now);
        let frame = FRAMES[(ms / 120) as usize % FRAMES.len()];
        Span::styled(frame, self.breath(now))
    }

    /// From dim to the accent and back every two seconds, on a sine — no
    /// blinking, a breath.
    fn breath(self, now: Instant) -> Style {
        let phase = (Self::since_start(now) % 2000) as f32 / 2000.0;
        let wave = 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos();
        match (self.color, self.truecolor) {
            (true, true) => Style::new().fg(self.blend(wave)),
            (true, false) if wave < 0.5 => self.dim(),
            (true, false) => self.accent(),
            _ => self.bold(),
        }
    }

    /// The dim grey at 0, the accent at 1, a little past it — near white, or
    /// near black on a light terminal — at 2.
    fn blend(self, at: f32) -> Color {
        let (dim, accent, glint) = if self.light {
            ((106., 108., 116.), (46., 92., 138.), (10., 40., 80.))
        } else {
            ((92., 100., 112.), (180., 205., 230.), (240., 248., 255.))
        };
        let (from, to, t) = if at <= 1.0 { (dim, accent, at) } else { (accent, glint, at - 1.0) };
        let mix = |a: f32, b: f32| (a + (b - a) * t.clamp(0.0, 1.0)).round() as u8;
        Color::Rgb(mix(from.0, to.0), mix(from.1, to.1), mix(from.2, to.2))
    }

    /// A word with light passing over it, letter by letter, left to right every
    /// 1.8 seconds — what is working shimmers. Plain accent without truecolor.
    fn shimmer(self, word: &str, now: Instant) -> Vec<Span<'static>> {
        if !(self.color && self.truecolor) {
            return vec![Span::styled(word.to_owned(), self.accent())];
        }
        let letters: Vec<char> = word.chars().collect();
        // The light enters before the first letter and leaves after the last.
        let span = letters.len() as f32 + 8.0;
        let at = (Self::since_start(now) % 1800) as f32 / 1800.0 * span - 4.0;
        letters
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let near = (-((i as f32 - at) / 1.6).powi(2)).exp();
                Span::styled(c.to_string(), Style::new().fg(self.blend(1.0 + near)))
            })
            .collect()
    }

    fn accent(self) -> Style {
        match (self.color, self.truecolor, self.light) {
            (true, true, false) => Style::new().fg(Color::Rgb(180, 205, 230)),
            (true, true, true) => Style::new().fg(Color::Rgb(46, 92, 138)),
            (true, false, false) => Style::new().fg(Color::Indexed(152)),
            (true, false, true) => Style::new().fg(Color::Indexed(24)),
            _ => Style::new().add_modifier(Modifier::BOLD),
        }
    }

    /// A member's own colour (see `look`): soft on a dark terminal, deeper on
    /// a light one, the nearest of 256 without truecolor, none without colour.
    fn member(self, hue: usize) -> Style {
        const DARK: [(u8, u8, u8); 8] = [
            (232, 190, 120), // amber
            (236, 150, 190), // pink
            (120, 190, 240), // sky
            (140, 208, 150), // green
            (182, 162, 242), // violet
            (110, 212, 200), // teal
            (240, 160, 110), // orange
            (205, 212, 120), // lime
        ];
        const LIGHT: [(u8, u8, u8); 8] =
            [(160, 104, 20), (176, 50, 110), (30, 104, 170), (40, 128, 60), (100, 70, 190), (20, 128, 120), (180, 80, 30), (110, 120, 20)];
        const INDEXED: [u8; 8] = [179, 175, 74, 108, 141, 73, 173, 149];
        let hue = hue % DARK.len();
        match (self.color, self.truecolor, self.light) {
            (true, true, false) => Style::new().fg(Color::Rgb(DARK[hue].0, DARK[hue].1, DARK[hue].2)),
            (true, true, true) => Style::new().fg(Color::Rgb(LIGHT[hue].0, LIGHT[hue].1, LIGHT[hue].2)),
            (true, false, _) => Style::new().fg(Color::Indexed(INDEXED[hue])),
            _ => Style::new(),
        }
    }

    fn dim(self) -> Style {
        match (self.color, self.truecolor, self.light) {
            (true, true, false) => Style::new().fg(Color::Rgb(124, 126, 133)),
            (true, true, true) => Style::new().fg(Color::Rgb(106, 108, 116)),
            (true, false, _) => Style::new().add_modifier(Modifier::DIM),
            _ => Style::new(),
        }
    }

    fn bold(self) -> Style {
        Style::new().add_modifier(Modifier::BOLD)
    }

    /// Code in a reply: the accent, so it stands apart from prose without a box.
    fn code(self) -> Style {
        if self.color { self.accent() } else { Style::new() }
    }

    fn markdown(self) -> markdown::Styles {
        markdown::Styles { plain: Style::new(), code: self.code(), dim: self.dim() }
    }
}

/// Whether the terminal says its background is light (`COLORFGBG`, set by
/// iTerm2, Konsole, rxvt and others as `fg;bg`).
fn light_background() -> bool {
    std::env::var("COLORFGBG")
        .ok()
        .and_then(|v| v.rsplit(';').next().and_then(|bg| bg.parse::<u8>().ok()))
        .is_some_and(|bg| bg == 7 || bg >= 9)
}

// ── geometry ──

/// The screen less its outer margin: two columns each side, one when narrow.
fn content(area: Rect) -> Rect {
    let margin = if area.width < 60 { 1 } else { 2 };
    Rect { x: area.x + margin, width: area.width.saturating_sub(2 * margin), ..area }
}

/// How wide the squad pane is: where the person dragged its edge, or else just
/// wide enough for its names and what follows them.
fn squad_width(app: &App, body: Rect) -> u16 {
    let fit = || {
        let name = app.agents.iter().map(|a| cols(&a.label())).max().unwrap_or(4).clamp(4, 12);
        let detail = app.agents.iter().filter_map(|a| a.short_model()).map(|m| cols(&m)).max().unwrap_or(0).max(9);
        // Room for " · 65%" once a conversation is past half full.
        let full = app.agents.iter().any(|a| a.context.is_some_and(|(u, w)| w > 0 && u * 2 >= w));
        let detail = detail + if full { 6 } else { 0 };
        // marker, mark and a space, the name, two spaces, the detail, a spare
        // column — and room for tickets' titles.
        ((2 + 2 + name + 2 + detail + 1) as u16).max(SIDE_FIT)
    };
    app.squad_width.unwrap_or_else(fit).clamp(SQUAD_MIN, (body.width / 2).max(SQUAD_MIN))
}

/// The squad pane, the hairline gap, and the conversation, side by side.
fn columns(app: &App, body: Rect) -> (Option<Rect>, Option<Rect>, Rect) {
    if body.width + 4 < SQUAD_MIN_TOTAL {
        return (None, None, body);
    }
    let squad_width = squad_width(app, body);
    let [squad, gap, right] =
        Layout::horizontal([Constraint::Length(squad_width), Constraint::Length(GAP), Constraint::Min(20)]).areas(body);
    (Some(squad), Some(gap), right)
}

/// The width a conversation is laid out at, in a terminal `total` columns wide.
fn transcript_width(app: &App, total: u16) -> usize {
    let body = content(Rect::new(0, 0, total, 1));
    let (_, _, right) = columns(app, body);
    (right.width as usize).saturating_sub(1).min(PROSE_MAX)
}

// ── laid-out conversations ──

/// What a laid-out conversation depends on: the width it was laid out for, and
/// enough of the conversation to notice it changed — its length, the length of
/// its last line (a reply grows as it streams), the memos waiting.
type LayoutKey = (usize, usize, usize, usize, bool, Option<TicketId>, u64, Option<String>);

/// Tickets waiting for the person, as their cards in a conversation show
/// them: number, title and asks.
type Turns = Vec<(TicketId, String, Vec<String>)>;

fn turns(app: &App) -> Turns {
    app.tickets.iter().filter(|t| t.status == TicketStatus::Waiting).map(|t| (t.id, t.title.clone(), t.asks())).collect()
}

/// What of the waiting tickets a laid-out conversation shows.
fn turns_key(turns: &Turns) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    turns.hash(&mut h);
    h.finish()
}

/// Conversations already laid out, per member. Laying one out means reading all
/// of its Markdown; scrolling must not do that again for every wheel event.
#[derive(Default)]
pub struct Layouts {
    laid_out: HashMap<SessionId, (LayoutKey, Vec<Line<'static>>)>,
    /// How this terminal shows pictures (iTerm2, kitty, sixel, or half blocks);
    /// none in tests.
    pub picker: Option<ratatui_image::picker::Picker>,
    /// The watched browser's latest picture, ready to draw, and which it is.
    picture: std::cell::RefCell<Option<(u64, ratatui_image::protocol::StatefulProtocol)>>,
}

impl Layouts {
    pub fn with_picker(picker: ratatui_image::picker::Picker) -> Self {
        Self { picker: Some(picker), ..Self::default() }
    }

    /// Lay out the selected member's conversation for a terminal `width` columns
    /// wide, reusing the last layout when nothing changed. Returns the member and
    /// how many lines its conversation grew by, so a view scrolled up can stay put.
    pub fn prepare(&mut self, app: &App, theme: Theme, width: u16) -> Option<(SessionId, usize)> {
        if app.pane != Pane::Conversation {
            return None;
        }
        let agent = app.viewed()?;
        let text_width = transcript_width(app, width);
        let turns = turns(app);
        let key = layout_key(agent, text_width, turns_key(&turns));
        let previous = match self.laid_out.get(&agent.id) {
            Some((k, _)) if *k == key => return None,
            Some((k, lines)) if k.0 == text_width => Some(lines.len()),
            _ => None,
        };
        let lines = transcript_lines(agent, &squad_looks(app), &turns, theme, text_width);
        let grew = previous.map(|before| lines.len().saturating_sub(before));
        self.laid_out.insert(agent.id, (key, lines));
        grew.filter(|n| *n > 0).map(|n| (agent.id, n))
    }

    fn get(&self, agent: &Agent, width: usize, turns: &Turns) -> Option<&[Line<'static>]> {
        match self.laid_out.get(&agent.id) {
            Some((key, lines)) if *key == layout_key(agent, width, turns_key(turns)) => Some(lines),
            _ => None,
        }
    }
}

fn layout_key(agent: &Agent, width: usize, turns: u64) -> LayoutKey {
    let entries = agent.drawn();
    let last = entries.last().map_or(0, |entry| match entry.as_ref() {
        Entry::User { text } | Entry::Reply { text } | Entry::Note { text } => text.len(),
        Entry::Tool { tool, target, .. } => tool.len() + target.as_ref().map_or(0, String::len),
        Entry::Error { message, .. } => message.len(),
        Entry::Landed { stat, .. } => stat.len(),
        Entry::Thread { .. } => 0,
    });
    (width, entries.len(), last, agent.queued.len(), agent.expanded, agent.showing, turns, agent.thread_view.clone())
}

// ── the screen ──

/// The screen's rows: header, body, approval, hairline, composer, footer — and
/// the approval's lines, which decide its height.
struct Rows {
    header: Rect,
    body: Rect,
    approval: Rect,
    rule: Rect,
    composer: Rect,
    footer: Rect,
    asking: Vec<Line<'static>>,
}

fn rows(app: &App, theme: Theme, screen: Rect) -> Rows {
    let area = content(screen);
    let asking = approval_lines(app, theme, area.width.saturating_sub(2) as usize);
    let asking_height = if asking.is_empty() { 0 } else { asking.len() as u16 + 1 };
    let (composer_rows, _) = app.composer.layout(area.width.saturating_sub(2) as usize);
    let composer_height = composer_rows.len().clamp(1, 8) as u16;
    let [header, _, body, approval, rule, composer, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(asking_height),
        Constraint::Length(1),
        Constraint::Length(composer_height),
        Constraint::Length(1),
    ])
    .areas(area);
    Rows { header, body, approval, rule, composer, footer, asking }
}

/// A pane under its title line and the blank line after it.
fn under_title(area: Rect) -> Rect {
    let [_, _, rest] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)]).areas(area);
    rest
}

/// The conversation's text area, above its live row (working, or new lines).
fn conversation_text(area: Rect, app: &App, agent: &Agent) -> Rect {
    conversation_areas(under_title(area), app, agent).text
}

/// Room for a watched browser: above the conversation, most of the pane,
/// when the pane is tall enough to keep some conversation in sight.
fn watch_height(app: &App, agent: &Agent, room: u16) -> u16 {
    if app.watch != Some(agent.id) || room < 14 {
        return 0;
    }
    room * 3 / 5
}

/// The conversation pane under its title: the conversation, then — when there
/// is any — work waiting for the person, then the live row.
fn conversation_areas(area: Rect, app: &App, agent: &Agent) -> Areas {
    let live = (agent.scroll_back > 0 && agent.unseen > 0)
        || agent.state == AgentState::Working
        || agent.estimate.as_ref().is_some_and(|e| e.live_at.is_some())
        || ticket_hint(app).is_some();
    let watch = watch_height(app, agent, area.height);
    // With the side list to its left, what waits and where each project
    // stands are there; without it — a narrow terminal — they show here.
    let side = area.x >= SQUAD_MIN;
    let overview = if side { 0 } else { overview_height(app, area.height.saturating_sub(watch)) };
    let card = if side { 0 } else { waiting_card_height(app, area.height.saturating_sub(watch + overview)) };
    let [watch, overview, _, text, _, card, _, live] = Layout::vertical([
        Constraint::Length(watch),
        Constraint::Length(overview),
        Constraint::Length(u16::from(overview > 0)),
        Constraint::Min(1),
        Constraint::Length(u16::from(card > 0)),
        Constraint::Length(card),
        Constraint::Length(u16::from(live)),
        Constraint::Length(u16::from(live)),
    ])
    .areas(area);
    Areas { watch, overview, text, card, live }
}

/// Where the parts of the conversation pane go.
struct Areas {
    watch: Rect,
    overview: Rect,
    text: Rect,
    card: Rect,
    live: Rect,
}

/// How many projects the overview lists before it counts the rest.
const OVERVIEW_SHOWN: usize = 6;

fn overview_height(app: &App, room: u16) -> u16 {
    if !app.shows_overview() || room < 16 {
        return 0;
    }
    let projects = app.projects.len();
    // Never more than a third of the pane: the conversation comes first.
    let rows = 1 + projects.min(OVERVIEW_SHOWN) + usize::from(projects > OVERVIEW_SHOWN);
    (rows as u16).min(room / 3)
}

/// Every project at a glance, over the PM's conversation when all of them
/// are in view: a line a project with who is at it and what waits — each a
/// click into that project.
fn overview(app: &App, theme: Theme, width: usize) -> Vec<(Line<'static>, Option<usize>)> {
    let lines = app.overview();
    let looks = squad_looks(app);
    let tags = tag_width(app);
    let width_of = |s: &str| unicode_width::UnicodeWidthStr::width(s);
    let mut rows = vec![(
        Line::from(vec![
            Span::styled(t!("view.all_projects"), theme.bold()),
            Span::styled(format!("   {}", t!("view.overview_hint")), theme.dim()),
        ]),
        None,
    )];
    for line in lines.iter().take(OVERVIEW_SHOWN) {
        let project = app.projects.get(line.at).cloned();
        let mut spans = vec![Span::styled(format!("{}  ", pad(&ui::fit(&line.name, tags), tags)), project_style(app, project.as_deref(), theme))];
        // What waits, at the right: for the person first.
        let mut tail: Vec<Span<'static>> = Vec::new();
        if line.for_you > 0 {
            tail.push(Span::styled(t!("view.overview_for_you", n = line.for_you), theme.accent().add_modifier(Modifier::BOLD)));
        }
        if line.open > 0 {
            if !tail.is_empty() {
                tail.push(Span::styled(" · ", theme.dim()));
            }
            tail.push(Span::styled(t!("view.overview_open", n = line.open), theme.dim()));
        }
        let tail_width: usize = tail.iter().map(Span::width).sum();
        let mut room = width.saturating_sub(tags + 2 + tail_width + 2);
        if line.busy.is_empty() {
            let quiet = if line.for_you + line.open == 0 { t!("view.quiet") } else { t!("view.no_one_on_it") };
            let quiet = ui::fit(quiet, room);
            room = room.saturating_sub(width_of(&quiet));
            spans.push(Span::styled(quiet, theme.dim()));
        }
        for (i, busy) in line.busy.iter().enumerate() {
            let sep = if i == 0 { "" } else { " · " };
            let what = match (busy.ticket, &busy.about) {
                (Some(id), _) => format!(" T-{id}"),
                (None, Some(about)) => format!(" “{}”", ui::fit(about, 24)),
                (None, None) => String::new(),
            };
            let what = match busy.estimate.as_ref().and_then(|e| estimate_short(e, now_unix())) {
                Some(left) => format!("{what} {left}"),
                None => what,
            };
            let mark = if busy.working { "● " } else { "○ " };
            let need = width_of(sep) + width_of(mark) + width_of(&busy.member) + width_of(&what);
            if need > room {
                spans.push(Span::styled(format!("{sep}+{}", line.busy.len() - i), theme.dim()));
                room = room.saturating_sub(width_of(sep) + 3);
                break;
            }
            room -= need;
            let hue = looks.iter().find(|(name, ..)| *name == busy.member).map(|l| l.2);
            let style = hue.map_or_else(|| theme.dim(), |h| theme.member(h));
            spans.push(Span::styled(sep.to_owned(), theme.dim()));
            spans.push(Span::styled(mark.to_owned(), if busy.working { style } else { theme.dim() }));
            spans.push(Span::styled(busy.member.clone(), style));
            spans.push(Span::styled(what, if busy.working { Style::new() } else { theme.dim() }));
        }
        if !tail.is_empty() {
            spans.push(Span::raw(" ".repeat(room + 2)));
            spans.extend(tail);
        }
        rows.push((Line::from(spans), Some(line.at)));
    }
    if lines.len() > OVERVIEW_SHOWN {
        rows.push((Line::from(Span::styled(t!("view.overview_more", n = lines.len() - OVERVIEW_SHOWN), theme.dim())), None));
    }
    rows
}

/// How many tickets the card names before it counts the rest.
const CARD_SHOWN: usize = 3;

fn waiting_card_height(app: &App, room: u16) -> u16 {
    let waiting = app.for_you_here().len();
    // Never on a short screen: the conversation comes first.
    if waiting == 0 || room < 16 {
        return 0;
    }
    (1 + waiting.min(CARD_SHOWN) + usize::from(waiting > CARD_SHOWN)) as u16
}

/// Work waiting for the person, in sight under any conversation: a heading,
/// then a row a ticket — each a click from its review on the board.
fn waiting_card(app: &App, theme: Theme, width: usize) -> Vec<(Line<'static>, Option<TicketId>)> {
    let waiting = app.for_you_here();
    if waiting.is_empty() {
        return Vec::new();
    }
    let bar = || Span::styled("▎ ", theme.accent());
    let tags = tag_width(app);
    let heading = if waiting.iter().all(|t| t.status == TicketStatus::Review) { t!("view.waiting_for_your_review") } else { t!("view.waiting_for_you") };
    let mut rows = vec![(
        Line::from(vec![
            bar(),
            Span::styled(format!("{heading} · {}", waiting.len()), theme.accent().add_modifier(Modifier::BOLD)),
            Span::styled(format!("   {}", t!("view.card_hint")), theme.dim()),
        ]),
        None,
    )];
    let looks = squad_looks(app);
    for t in waiting.iter().take(CARD_SHOWN) {
        let who = t.assignee.as_ref().or(t.owner.as_ref()).cloned().unwrap_or_default();
        let style = looks.iter().find(|(name, ..)| *name == who).map_or_else(|| theme.dim(), |l| theme.member(l.2));
        let id = format!("{:<6}", format!("T-{}", t.id));
        // Owners line up: titles take a fixed width.
        let title_room = width.saturating_sub(2 + 6 + 2 + 12 + tags + 2).min(56);
        // A waiting ticket says what the person has to do, after its title.
        let needs = if t.status == TicketStatus::Waiting { t.asks() } else { Vec::new() };
        let needs = match needs.as_slice() {
            [] => String::new(),
            [one] => format!(" · {}", t!("view.card_needs", what = one)),
            [first, rest @ ..] => format!(" · {}", t!("view.card_needs", what = format!("{first} +{}", rest.len()))),
        };
        let width_of = |s: &str| unicode_width::UnicodeWidthStr::width(s);
        // The card is short of room: only work that isn't code names its kind.
        let kind = if t.kind.in_home() { format!("{} · ", label::kind(crate::i18n::lang(), t.kind)) } else { String::new() };
        let title_room = title_room.saturating_sub(cols(&kind));
        let title = ui::fit(&t.title, if needs.is_empty() { title_room } else { title_room.min(width_of(&t.title)).min(title_room / 2) });
        let needs = ui::fit(&needs, title_room.saturating_sub(width_of(&title)));
        let pad = title_room.saturating_sub(width_of(&title) + width_of(&needs));
        rows.push((
            Line::from(vec![
                bar(),
                Span::styled(id, theme.accent()),
                {
                    let tag = project_tag(app, t, theme, tags);
                    Span::styled(format!("{}  ", tag.content), tag.style)
                },
                Span::styled(kind, theme.dim()),
                Span::styled(title, Style::new()),
                Span::styled(needs, theme.accent()),
                Span::raw(" ".repeat(pad + 2)),
                Span::styled(who, style),
            ]),
            Some(t.id),
        ));
    }
    if waiting.len() > CARD_SHOWN {
        rows.push((Line::from(vec![bar(), Span::styled(t!("view.card_more", n = waiting.len() - CARD_SHOWN), theme.dim())]), None));
    }
    rows
}

/// How much there is to scroll on a screen this size: half the conversation's
/// height for a page, and how far the conversation and the board can go.
pub struct Extent {
    pub page: usize,
    pub conversation_max: usize,
    pub board_max: usize,
    /// Where the clickable things are: the pane switch, the board's ticket rows.
    pub hits: Vec<(u16, u16, u16, Hit)>,
    pub divider: Option<Divider>,
    /// On the board: the rows the open ticket is shown in (first, last).
    pub board_detail: Option<(u16, u16)>,
    /// On the board: the line between the list and the ticket, to drag.
    pub board_rule: Option<BoardRule>,
    /// Where the conversation's text is, for selecting it.
    pub text: Option<TextView>,
}

pub fn extent(app: &App, theme: Theme, screen: Rect, layouts: &Layouts) -> Extent {
    let body = rows(app, theme, screen).body;
    let (_, gap, right) = columns(app, body);
    // The hairline between the squad and the rest: drag it to resize.
    let divider = gap.map(|gap| Divider { x: gap.x + 1, top: body.y, bottom: body.y + body.height, body_x: body.x });
    let board = under_title(right);
    // The switch at the title's right: clicking it changes panes.
    let switch: usize = switch_label(app, app.pane, theme).iter().map(Span::width).sum();
    let other = if app.pane == Pane::Board { Pane::Conversation } else { Pane::Board };
    let mut hits = vec![(right.x + right.width.saturating_sub(switch as u16), right.x + right.width, right.y, Hit::Pane(other))];
    // The squad's name in the header, and its ▾: the way to the other squads;
    // with several projects, their chips.
    let top = content(screen);
    let name_x = top.x + 10;
    hits.push((name_x, name_x + unicode_width::UnicodeWidthStr::width(app.squad.as_str()) as u16 + 2, top.y, Hit::Squads));
    let (squad_pane, _, _) = columns(app, body);
    let narrow = squad_pane.is_none();
    // The side list's lines: a click opens one.
    if let Some(squad) = squad_pane {
        let lines = side_lines(app, theme, squad);
        let top = side_top(app, &lines, squad.height as usize);
        for (row, (_, side)) in lines.into_iter().skip(top).enumerate().take(squad.height as usize) {
            if let Some(side) = side {
                hits.push((squad.x, squad.x + squad.width, squad.y + row as u16, Hit::Side(side)));
            }
        }
    }
    // The completions above the composer: a click takes one.
    let laid_rows = rows(app, theme, screen);
    if let Some((area, first, shown)) = choices_place(app, Rect { height: laid_rows.rule.y.saturating_sub(body.y), ..body }) {
        // Drawn over the rest, so they come first.
        for n in (0..shown).rev() {
            hits.insert(0, (area.x, area.x + area.width, area.y + 1 + n as u16, Hit::Choice(first + n)));
        }
    }
    for (from, to, hit) in header(app, theme, narrow, top.width as usize).1 {
        hits.push((top.x + from, top.x + to, top.y, hit));
    }
    let mut board_max = 0;
    let mut board_page = board.height as usize / 2;
    let mut board_detail = None;
    let mut board_text = None;
    let mut board_rule = None;
    if app.pane == Pane::Board {
        let width = (board.width as usize).saturating_sub(1).min(PROSE_MAX + 20);
        let rows = board_list(app, theme, width);
        let (list, detail) = board_areas(board, rows.len(), app.board_split);
        let body = detail_body(detail);
        let lines = ticket_detail(app, theme, detail_width(body)).len();
        board_max = lines.saturating_sub(body.height as usize);
        board_page = (body.height as usize / 2).max(1);
        board_detail = Some((body.y, body.y + body.height.saturating_sub(1)));
        // The blank row under the list and the hairline: hold either to drag.
        board_rule = Some(BoardRule { rows: (list.y + list.height, detail.y), list_top: list.y });
        // The open ticket's text can be selected, like the conversation's.
        board_text = (!app.help_open).then_some(TextView {
            x: body.x,
            y: body.y,
            width: detail_width(body) as u16,
            height: body.height,
            top: app.board_scroll.min(board_max),
        });
        let top = list_top(&rows, app.selected_ticket().map(|t| t.id), list.height as usize);
        for (i, (_, id)) in rows.iter().enumerate().skip(top).take(list.height as usize) {
            if let Some(id) = id {
                hits.push((list.x, list.x + list.width, list.y + (i - top) as u16, Hit::Ticket(*id)));
            }
        }
    }
    let Some(agent) = app.viewed() else {
        return Extent { page: board_page, conversation_max: 0, board_max, hits, divider, board_detail, board_rule, text: board_text };
    };
    let text = conversation_text(right, app, agent);
    // The waiting card's rows open their tickets too.
    if app.pane == Pane::Conversation {
        let Areas { card, overview: projects, .. } = conversation_areas(under_title(right), app, agent);
        for (i, (_, id)) in waiting_card(app, theme, card.width as usize).into_iter().enumerate().take(card.height as usize) {
            if let Some(id) = id {
                hits.push((card.x, card.x + card.width, card.y + i as u16, Hit::Ticket(id)));
            }
        }
        // The overview's lines go into their projects.
        for (i, (_, at)) in overview(app, theme, projects.width as usize).into_iter().enumerate().take(projects.height as usize) {
            if let Some(at) = at {
                hits.push((projects.x, projects.x + projects.width, projects.y + i as u16, Hit::Project(Some(at))));
            }
        }
    }
    let width = (text.width as usize).saturating_sub(1).min(PROSE_MAX);
    let fresh;
    let laid: &[Line<'static>] = match layouts.get(agent, width, &turns(app)) {
        Some(lines) => lines,
        None => {
            fresh = transcript_lines(agent, &squad_looks(app), &turns(app), theme, width);
            &fresh
        }
    };
    let lines = laid.len();
    let height = text.height as usize;
    let bottom = lines.saturating_sub(height);
    let top = bottom.saturating_sub(agent.scroll_back.min(bottom));
    let text_view = (app.pane == Pane::Conversation && !app.help_open)
        .then_some(TextView { x: text.x, y: text.y, width: width as u16, height: text.height, top });
    // Ticket numbers on screen are ways to their tickets: clicking one opens it
    // on the board. Rows as `render_transcript` shows them.
    if app.pane == Pane::Conversation {
        let known: Vec<TicketId> = app.tickets.iter().map(|t| t.id).collect();
        for (row, line) in laid[top..lines.min(top + height)].iter().enumerate() {
            let mut x = text.x;
            for span in &line.spans {
                let content: &str = &span.content;
                if let Some((at, len)) = ticket_ref(content)
                    && let Ok(id) = content[at + 2..at + len].parse::<TicketId>()
                    && known.contains(&id)
                {
                    let from = x + unicode_width::UnicodeWidthStr::width(&content[..at]) as u16;
                    hits.push((from, from + len as u16, text.y + row as u16, Hit::Ticket(id)));
                }
                x += span.width() as u16;
            }
        }
    }
    Extent {
        page: if app.pane == Pane::Board { board_page } else { text.height as usize / 2 },
        conversation_max: lines.saturating_sub(text.height as usize),
        board_max,
        hits,
        divider,
        board_detail,
        board_rule,
        text: if app.pane == Pane::Board { board_text } else { text_view },
    }
}

pub fn render(frame: &mut Frame, app: &App, theme: Theme, layouts: &Layouts) {
    let Rows { header, body, approval, rule, composer, footer, asking } = rows(app, theme, frame.area());
    let asking_height = approval.height;

    let (squad, gap, right) = columns(app, body);
    render_header(frame, header, app, theme, squad.is_none());
    if let (Some(squad), Some(gap)) = (squad, gap) {
        render_side(frame, squad, app, theme);
        render_hairline(frame, gap, app.is_dragging_divider(), theme);
    }
    match app.pane {
        Pane::Conversation if app.help_open => render_help(frame, right, theme),
        Pane::Conversation if app.usage_open => render_usage(frame, right, app, theme),
        // A ticket nobody is on now: what it is, from the board.
        Pane::Conversation if app.opened.is_some() && app.viewed().is_none() => render_ticket(frame, right, app, theme),
        Pane::Conversation => render_transcript(frame, right, app, theme, layouts),
        Pane::Board => render_board(frame, right, app, theme),
    }
    if asking_height > 0 {
        render_approval(frame, approval, asking);
    }
    // One hairline above the composer: a field without a box (nothing to copy by accident).
    frame.render_widget(Paragraph::new(Span::styled("─".repeat(rule.width as usize), theme.dim())), rule);
    render_composer(frame, composer, app, theme);
    render_footer(frame, footer, app, theme);
    render_choices(frame, Rect { height: rule.y.saturating_sub(body.y), ..body }, app, theme);
}

/// Completions, just above the composer and over whatever is there: the picked
/// one marked, each with what it does.
const CHOICES_SHOWN: usize = 8;

/// Where the completions sit: their area, the first shown, and how many.
fn choices_place(app: &App, above: Rect) -> Option<(Rect, usize, usize)> {
    let count = app.choices().len();
    if count == 0 || above.height < 3 {
        return None;
    }
    let first = (app.pick.min(count - 1) + 1).saturating_sub(CHOICES_SHOWN);
    let shown = count.saturating_sub(first).min(CHOICES_SHOWN);
    let height = (shown as u16 + 2).min(above.height);
    Some((Rect { y: above.y + above.height - height, height, ..above }, first, shown))
}

fn render_choices(frame: &mut Frame, above: Rect, app: &App, theme: Theme) {
    const SHOWN: usize = CHOICES_SHOWN;
    let choices = app.choices();
    if choices.is_empty() || above.height < 3 {
        return;
    }
    let picked = app.pick.min(choices.len() - 1);
    let first = (picked + 1).saturating_sub(SHOWN);
    let label_width = choices.iter().map(|c| cols(&c.label)).max().unwrap_or(0).min(40) + 3;
    let mut lines: Vec<Line> = vec![Line::from(Span::styled("─".repeat(above.width as usize), theme.dim()))];
    for (i, choice) in choices.iter().enumerate().skip(first).take(SHOWN) {
        let (mark, label_style) =
            if i == picked { (Span::styled("› ", theme.accent()), theme.accent().add_modifier(Modifier::BOLD)) } else { (Span::raw("  "), theme.bold()) };
        lines.push(Line::from(vec![
            mark,
            Span::styled(pad(&ui::fit(&choice.label, label_width - 3), label_width), label_style),
            Span::styled(ui::fit(&choice.about, (above.width as usize).saturating_sub(label_width + 2)), theme.dim()),
        ]));
    }
    let more = choices.len().saturating_sub(first + SHOWN);
    lines.push(Line::from(Span::styled(
        format!("  {}{}", t!("view.choices_keys"), if more > 0 { format!(" · {}", t!("view.choices_more", n = more)) } else { String::new() }),
        theme.dim(),
    )));
    let height = (lines.len() as u16).min(above.height);
    let area = Rect { y: above.y + above.height - height, height, ..above };
    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_widget(Paragraph::new(lines), area);
}

/// A moment as the person's clock reads it: `14:30` today, `10/20 14:30` later.
fn local_time(unix: i64) -> String {
    let format = |fmt: &str| -> Option<String> {
        // BSD date takes -r seconds; GNU date takes -d @seconds.
        [vec!["-r".to_owned(), unix.to_string()], vec!["-d".to_owned(), format!("@{unix}")]].into_iter().find_map(|args| {
            let out = std::process::Command::new("date").args(&args).arg(fmt).output().ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        })
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let fmt = if unix - now < 20 * 60 * 60 { "+%H:%M" } else { "+%m/%d %H:%M" };
    format(fmt).unwrap_or_default()
}

/// A time as the clock on the wall reads it: `20:52`. The offset from UTC is
/// asked of the system once, not every frame.
fn clock(unix: u64) -> String {
    static OFFSET: std::sync::LazyLock<i64> = std::sync::LazyLock::new(|| {
        let out = std::process::Command::new("date").arg("+%z").output().ok();
        let text = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
        let (sign, digits) = match text.strip_prefix('-') {
            Some(rest) => (-1, rest.to_owned()),
            None => (1, text.trim_start_matches('+').to_owned()),
        };
        let hours: i64 = digits.get(..2).and_then(|h| h.parse().ok()).unwrap_or(0);
        let minutes: i64 = digits.get(2..4).and_then(|m| m.parse().ok()).unwrap_or(0);
        sign * (hours * 3600 + minutes * 60)
    });
    let minutes = (unix as i64 + *OFFSET).div_euclid(60).rem_euclid(24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// How long until `at`, short: `12m`, `1h 20m`, `<1m`.
fn time_left(at: u64, now: u64) -> String {
    let minutes = at.saturating_sub(now).div_ceil(60);
    match minutes {
        0 => t!("view.time_under_minute").to_owned(),
        m if m < 60 => t!("view.time_minutes", m = m),
        m if m % 60 == 0 => t!("view.time_hours", h = m / 60),
        m => t!("view.time_hours_minutes", h = m / 60, m = m % 60),
    }
}

/// When a member's work is expected done, as a phrase: `done ~20:52 (12m)`,
/// or — past it — `expected ~20:52, still going`. equipe's own guess says so:
/// `usually done ~20:52 (12m)`, `longer than usual`.
fn done_phrase(at: u64, guessed: bool, now: u64) -> String {
    match (at > now, guessed) {
        (true, false) => t!("view.done_at", at = clock(at), left = time_left(at, now)),
        (true, true) => t!("view.usually_done_at", at = clock(at), left = time_left(at, now)),
        (false, false) => t!("view.expected_still_going", at = clock(at)),
        (false, true) => t!("view.longer_than_usual").to_owned(),
    }
}

/// When what it ships is expected live: `v0.28.1 on Homebrew live ~21:05 (25m)`.
fn live_phrase(estimate: &Estimate, now: u64) -> Option<String> {
    let at = estimate.live_at?;
    let what = estimate.live_what.as_deref().map(|w| format!("{w} ")).unwrap_or_default();
    Some(if at > now { t!("view.live_at", what = what, at = clock(at), left = time_left(at, now)) } else { t!("view.live_any_minute", what = what) })
}

/// The shortest word on it, for a narrow row: time left until done, else
/// until live (`↑25m`).
fn estimate_short(estimate: &Estimate, now: u64) -> Option<String> {
    let about = if estimate.guessed { "≈" } else { "~" };
    match (estimate.done_at, estimate.live_at) {
        (Some(at), _) if at > now => Some(format!("{about}{}", time_left(at, now))),
        (Some(_), _) => Some(if estimate.guessed { t!("view.estimate_long") } else { t!("view.estimate_late") }.to_owned()),
        (None, Some(at)) if at > now => Some(format!("↑{}", time_left(at, now))),
        _ => None,
    }
}

/// `/usage`: the plan's windows, then each member — how full its context is,
/// what it has processed, how much of that came from the cache.
fn render_usage(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let area = pane_title(vec![Span::styled(t!("view.usage_title"), theme.bold())], vec![Span::styled(t!("view.esc_closes"), theme.dim())], area, frame);
    let width = (area.width as usize).saturating_sub(1);
    let heading = |h: &str| Line::from(Span::styled(h.to_owned(), theme.accent().add_modifier(Modifier::BOLD)));
    let row = |k: &str, what: String| wrapped(vec![Span::styled(format!("  {}", pad(&ui::fit(k, 13), 14)), theme.bold())], &what, theme.dim(), 16, width);
    let mut lines = vec![heading(t!("view.usage_plan"))];
    match app.plan {
        Some(plan) => {
            for (label, window) in [(t!("view.window_five_hour"), plan.five_hour), (t!("view.window_week"), plan.seven_day)] {
                if let Some(w) = window {
                    lines.extend(row(label, t!("view.window_used", percent = w.used_percent, when = local_time(w.resets_at))));
                }
            }
        }
        None => lines.extend(row("—", t!("view.plan_not_reported").into())),
    }
    lines.push(Line::raw(""));
    lines.push(heading(t!("view.usage_members")));
    for agent in app.agents.iter().filter(|a| a.is_open()) {
        let context = agent
            .context
            .filter(|(_, w)| *w > 0)
            .map_or_else(
                || t!("view.context_unknown").to_owned(),
                |(u, w)| t!("view.context_of", percent = u * 100 / w, used = u / 1000, window = w / 1000),
            );
        let cache = (agent.seen.1 * 100).checked_div(agent.seen.0).map_or_else(String::new, |p| format!(" · {}", t!("view.from_cache", percent = p)));
        lines.extend(row(&agent.label(), t!("view.usage_member", context = context, tokens = short_count(agent.tokens), cache = cache)));
    }
    lines.push(Line::raw(""));
    lines.extend(wrapped(
        Vec::new(),
        t!("view.usage_note"),
        theme.dim(),
        0,
        width,
    ));
    frame.render_widget(Paragraph::new(lines), area);
}

/// `/help`: what the console does, grouped — talking, looking around,
/// answering — and every command, from the list completion uses.
fn render_help(frame: &mut Frame, area: Rect, theme: Theme) {
    let title = vec![Span::styled(t!("view.help_title"), theme.bold())];
    let area = pane_title(title, vec![Span::styled(t!("view.esc_closes"), theme.dim())], area, frame);
    let width = (area.width as usize).saturating_sub(1);
    let commands: Vec<(String, &str)> =
        super::commands::commands().iter().map(|c| (format!("/{} {}", c.name, c.args).trim_end().to_owned(), c.about)).collect();
    // Keys and commands each line up in a column as wide as their longest.
    let rows = |pairs: &[(String, &str)], column: usize| -> Vec<Line<'static>> {
        let column = column.min(width / 2) + 3;
        pairs
            .iter()
            .flat_map(|(k, what)| {
                wrapped(vec![Span::styled(format!("  {}", pad(&ui::fit(k, column - 1), column)), theme.bold())], what, theme.dim(), column + 2, width)
            })
            .collect()
    };
    let heading = |h: &str| vec![Line::raw(""), Line::from(Span::styled(h.to_owned(), theme.accent().add_modifier(Modifier::BOLD)))];
    let pairs = |list: &[(&str, &'static str)]| -> Vec<(String, &'static str)> { list.iter().map(|(k, w)| ((*k).to_owned(), *w)).collect() };
    let mut lines = wrapped(
        Vec::new(),
        t!("view.help_intro"),
        Style::new(),
        0,
        width,
    );
    let talk = pairs(&[
        ("Enter", t!("view.help_enter")),
        ("Tab", t!("view.help_tab")),
        ("@name …", t!("view.help_at_name")),
        ("Tab · Shift-Tab", t!("view.help_walk")),
        ("Alt-Enter · Ctrl-J", t!("view.help_new_line")),
        ("Esc", t!("view.help_esc")),
        ("Ctrl-V", t!("view.help_ctrl_v")),
        ("⌃⇧V", t!("view.help_ssh_paste")),
        ("→", t!("view.help_suggestion")),
    ]);
    let look = pairs(&[
        ("Ctrl-B", t!("view.help_ctrl_b")),
        ("Ctrl-P", t!("view.help_ctrl_p")),
        (t!("view.help_scroll_key"), t!("view.help_scroll")),
        ("Ctrl-W", t!("view.help_ctrl_w")),
        (t!("view.help_click_drag_key"), t!("view.help_click_drag")),
    ]);
    let answer = pairs(&[
        ("y · a · n", t!("view.help_yan")),
        (t!("view.help_on_board_key"), t!("view.help_on_board")),
    ]);
    // The keys of all three groups share one column, as wide as the widest.
    let keys = talk.iter().chain(&look).chain(&answer).map(|(k, _)| cols(k)).max().unwrap_or(0);
    lines.extend(heading(t!("view.help_talk")));
    lines.extend(rows(&talk, keys));
    lines.extend(heading(t!("view.help_look_around")));
    lines.extend(rows(&look, keys));
    lines.extend(heading(t!("view.help_answer")));
    lines.extend(rows(&answer, keys));
    lines.extend(heading(t!("view.help_modes")));
    let modes: Vec<(String, &str)> =
        equipe_core::protocol::Mode::ALL.iter().map(|m| (super::commands::mode_word(*m).to_owned(), label::mode_summary(crate::i18n::lang(), *m))).collect();
    lines.extend(rows(&modes, keys));
    lines.extend(heading(t!("view.help_commands")));
    let widest = commands.iter().map(|(k, _)| cols(k)).max().unwrap_or(0);
    lines.extend(rows(&commands, widest));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(t!("view.help_leave"), theme.dim())));
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App, theme: Theme, narrow: bool) {
    let (spans, _) = header(app, theme, narrow, area.width as usize);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// A project's colour among the squad's, by its place in the list.
fn project_style(app: &App, project: Option<&std::path::Path>, theme: Theme) -> Style {
    let at = project.and_then(|p| app.projects.iter().position(|q| q == p)).unwrap_or(0);
    theme.member([2, 6, 4, 7, 5, 1, 3, 0][at % 8])
}

/// A ticket's project, as a tag in its colour — "home" for the squad home's
/// work — padded to `width` so tags line up.
fn project_tag(app: &App, ticket: &equipe_core::protocol::Ticket, theme: Theme, width: usize) -> Span<'static> {
    let project = app.ticket_project(ticket);
    let name = project.as_deref().map_or_else(|| home_tag().to_owned(), project_name);
    let style = if project.is_some() { project_style(app, project.as_deref(), theme) } else { theme.dim() };
    Span::styled(pad(&ui::fit(&name, width), width), style)
}

/// What the squad home's work is tagged with on the board.
fn home_tag() -> &'static str {
    t!("view.home_tag")
}

/// The widest project tag a squad needs — "home" at least — up to 12 columns.
fn tag_width(app: &App) -> usize {
    app.projects.iter().map(|p| cols(&project_name(p))).chain([cols(home_tag())]).max().unwrap_or(0).min(12)
}

/// The header: the wordmark, the squad (a click lists the others), its home,
/// then a chip for each project and "all" — a click filters the board and
/// reviews to that project; "all", the default, shows the whole squad.
fn header(app: &App, theme: Theme, narrow: bool, width: usize) -> (Vec<Span<'static>>, Vec<(u16, u16, Hit)>) {
    let mut spans = vec![
        Span::styled("equipe", theme.bold()),
        Span::styled(".", theme.dim()),
        Span::raw("   "),
        Span::styled(app.squad.clone(), theme.bold()),
        // A click on the name lists the other squads.
        Span::styled(" ▾", theme.dim()),
    ];
    // Without the squad pane, the header says who is there.
    if narrow {
        let working = app.working();
        let open = app.agents.iter().filter(|a| a.is_open()).count();
        let summary = if working > 0 {
            format!("   {}", t!("view.members_working", open = open, working = working))
        } else {
            format!("   {}", t!("view.members", open = open))
        };
        spans.push(Span::styled(summary, theme.dim()));
    }
    // Who has the Mac's screen, whichever squad: there is only one.
    if let Some(desktop) = label::desktop(crate::i18n::lang(), &app.desktop, Some(&app.squad)) {
        spans.push(Span::styled(format!("   ▣ {desktop}"), theme.accent()));
    }
    let mut hits = Vec::new();
    let used = |spans: &[Span]| spans.iter().map(Span::width).sum::<usize>();
    let mut chips: Vec<Span<'static>> = Vec::new();
    let mut chip_hits = Vec::new();
    if !app.projects.is_empty() {
        // Each project's chip: its name (cut when long), how it's doing, and
        // how much it matters now — in focus, then waiting on the person,
        // then at work, then quiet.
        let one = |project: &std::path::PathBuf| -> (Vec<Span<'static>>, u8) {
            let focused = app.focus.as_ref() == Some(project);
            let style = if focused {
                project_style(app, Some(project), theme).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                theme.dim()
            };
            let mut spans = vec![Span::styled(ui::fit(&project_name(project), 18), style)];
            let status = app.project_status(project);
            if status.working > 0 {
                spans.push(Span::styled(format!(" ●{}", status.working), project_style(app, Some(project), theme)));
            }
            if status.for_you > 0 {
                spans.push(Span::styled(format!(" ⚑{}", status.for_you), theme.accent().add_modifier(Modifier::BOLD)));
            }
            let rank = if focused { 0 } else if status.for_you > 0 { 1 } else if status.working > 0 { 2 } else { 3 };
            (spans, rank)
        };
        let all: Vec<(Vec<Span<'static>>, u8)> = app.projects.iter().map(one).collect();
        let width_of = |spans: &[Span]| spans.iter().map(Span::width).sum::<usize>();
        let sep = 3; // " · "
        let all_label = t!("view.all");
        let all_chip = cols(all_label);
        let more = |hidden: usize| if hidden == 0 { 0 } else { sep + format!("+{hidden} ▾").chars().count() };
        // As much as fits next to the squad's name; the rest behind "+N".
        let room = width.saturating_sub(used(&spans) + 3);
        let mut order: Vec<usize> = (0..all.len()).collect();
        order.sort_by_key(|&i| (all[i].1, i));
        let mut shown: Vec<usize> = Vec::new();
        let mut taken = all_chip;
        for &i in &order {
            let cost = width_of(&all[i].0) + sep;
            let hidden_after = all.len() - shown.len() - 1;
            if taken + cost + more(hidden_after) <= room {
                shown.push(i);
                taken += cost;
            }
        }
        shown.sort_unstable();
        let hidden = all.len() - shown.len();
        let mut first = true;
        let mut chip = |chips: &mut Vec<Span<'static>>, parts: Vec<Span<'static>>, hit: Hit| {
            if !first {
                chips.push(Span::styled(" · ", theme.dim()));
            }
            first = false;
            let from = used(chips);
            chips.extend(parts);
            chip_hits.push((from, used(chips), hit));
        };
        for i in shown {
            chip(&mut chips, all[i].0.clone(), Hit::Project(Some(i)));
        }
        if hidden > 0 {
            chip(&mut chips, vec![Span::styled(format!("+{hidden} ▾"), theme.bold())], Hit::Projects);
        }
        let style = if app.focus.is_none() { theme.bold().add_modifier(Modifier::UNDERLINED) } else { theme.dim() };
        chip(&mut chips, vec![Span::styled(all_label, style)], Hit::Project(None));
    }
    // The squad home, between the name and the chips: what doesn't fit of it
    // goes first, keeping its end.
    let home = equipe_core::squad_home::path(&equipe_providers::credentials::equipe_home(), &app.squad);
    let mut place = display_path(&home);
    let gap = if chips.is_empty() { 0 } else { 3 };
    let room = width.saturating_sub(used(&spans) + 3 + gap + used(&chips));
    if room > 4 {
        let place_width = unicode_width::UnicodeWidthStr::width(place.as_str());
        if place_width > room {
            let tail: String = place.chars().rev().collect();
            place = format!("…{}", ui::fit(&tail, room - 1).trim_end_matches('…').chars().rev().collect::<String>());
        }
        spans.push(Span::styled(format!("   {place}"), theme.dim()));
    }
    if !chips.is_empty() {
        spans.push(Span::raw("   "));
        let at = used(&spans);
        hits.extend(chip_hits.into_iter().map(|(from, to, hit)| ((at + from) as u16, (at + to) as u16, hit)));
        spans.extend(chips);
    }
    (spans, hits)
}

/// Each member's own mark and colour, so the squad tells apart at a glance.
/// The built-in roles have theirs; anyone else gets one from their name.
fn look(role: &str, name: &str) -> (&'static str, usize) {
    match role {
        "pm" => ("◆", 0),
        "designer" => ("✦", 1),
        "frontend" => ("●", 2),
        "backend" => ("■", 3),
        "qa" => ("◎", 4),
        "data" => ("▲", 5),
        "mobile" => ("◈", 6),
        _ => {
            const MARKS: [&str; 4] = ["◇", "★", "▼", "✚"];
            let h = name.bytes().fold(7usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize));
            (MARKS[h % MARKS.len()], 6 + h % 2)
        }
    }
}

fn agent_look(agent: &Agent) -> (&'static str, usize) {
    let name = agent.label();
    look(agent.member.as_ref().map_or("", |m| m.role.as_str()), &name)
}

/// A member's colour by the name the person knows it by.
fn member_style(app: &App, name: &str, theme: Theme) -> Style {
    let hue = match app.agents.iter().find(|a| a.label() == name) {
        Some(agent) => agent_look(agent).1,
        None => look(name, name).1,
    };
    theme.member(hue)
}

/// Who it is right now, as a glyph: needs the person, working (turning, in its
/// colour), or at rest — its own mark.
fn glyph(app: &App, agent: &Agent, theme: Theme) -> Span<'static> {
    if app.asks_you(agent) {
        return Span::styled("✻", theme.accent().add_modifier(Modifier::BOLD));
    }
    let (mark, hue) = agent_look(agent);
    // At work in another project's conversation is at work too.
    let state = if agent.state == AgentState::Ready && app.lanes_working(agent) > 0 { AgentState::Working } else { agent.state };
    match state {
        AgentState::Starting => Span::styled("…", theme.dim()),
        AgentState::Ready => Span::styled(mark, theme.member(hue)),
        AgentState::Working => Span::styled(theme.spinner(Instant::now()).content, theme.member(hue).add_modifier(Modifier::BOLD)),
        AgentState::Closing | AgentState::Closed => Span::styled("·", theme.dim()),
        AgentState::Failed => Span::styled("✗", theme.bold()),
    }
}

/// What a working member is doing right now, from its latest step: reading,
/// editing, running — or thinking, between steps.
fn doing(agent: &Agent) -> &'static str {
    match agent.transcript.entries().last() {
        Some(Entry::Tool { tool, .. }) => match tool_verb(tool) {
            "read" => t!("view.doing_reading"),
            "edit" | "write" => t!("view.doing_editing"),
            "run" => t!("view.doing_running"),
            "search" => t!("view.doing_searching"),
            "fetch" => t!("view.doing_looking_up"),
            "board" => t!("view.doing_checking_board"),
            "ticket" => t!("view.doing_updating_board"),
            "comment" => t!("view.doing_commenting"),
            "hand in" => t!("view.doing_handing_in"),
            "browse" => t!("view.doing_browsing"),
            "delegate" => t!("view.doing_delegating"),
            "ask you" => t!("view.waiting_for_you_lower"),
            _ => t!("view.working"),
        },
        Some(Entry::Reply { .. }) => t!("view.doing_writing"),
        _ => t!("view.doing_thinking"),
    }
}

/// The side list, a line each: the PM, then the tickets by where they stand,
/// then the team — and what a click on each line opens.
fn side_lines(app: &App, theme: Theme, area: Rect) -> Vec<(Line<'static>, Option<Side>)> {
    let mut lines: Vec<(Line<'static>, Option<Side>)> = vec![(Line::from(Span::styled(t!("view.side_squad"), theme.dim())), None), (Line::raw(""), None)];
    let name_width = app.agents.iter().map(|a| cols(&a.label())).max().unwrap_or(4).clamp(4, 12);
    let open = (app.pane == Pane::Conversation).then(|| app.side_open());
    for side in app.side() {
        match side {
            Side::Heading(group, count) => {
                lines.push((Line::raw(""), None));
                lines.push((Line::from(Span::styled(format!("{} · {count}", group.label()), theme.dim())), None));
            }
            Side::Menu(menu) => {
                // 나에게 · 작업 중 · 모든 일, each with its count, as the desktop's menu.
                let on = app.pane == Pane::Board && app.board_menu == menu;
                let count = app.menu_count(menu);
                let marker = if on { Span::styled("› ", theme.accent()) } else { Span::raw("  ") };
                let mark = match menu {
                    BoardMenu::Inbox => "!",
                    BoardMenu::Working => "*",
                    BoardMenu::All => "o",
                };
                let accent = menu == BoardMenu::Inbox && count > 0;
                let widest = [BoardMenu::Inbox, BoardMenu::Working, BoardMenu::All].iter().map(|m| cols(m.label())).max().unwrap_or(8);
                let label = Span::styled(pad(menu.label(), widest + 2), if on { theme.bold() } else { Style::new() });
                let count = Span::styled(if count > 0 { count.to_string() } else { String::new() }, if accent { theme.accent().add_modifier(Modifier::BOLD) } else { theme.dim() });
                lines.push((Line::from(vec![marker, Span::styled(format!("{mark} "), if accent { theme.accent() } else { theme.dim() }), label, count]), Some(side)));
            }
            Side::Pm | Side::Member(_) => {
                let i = match side {
                    Side::Member(i) => i,
                    _ => match app.pm_index() {
                        Some(i) => i,
                        None => continue,
                    },
                };
                let Some(agent) = app.agents.get(i) else { continue };
                lines.push((member_line(app, agent, open == Some(side), name_width, area, theme), Some(side)));
                // The ticket it is on, under it.
                if let Some(t) = agent.ticket.and_then(|id| app.tickets.iter().find(|t| t.id == id)) {
                    let room = (area.width as usize).saturating_sub(4 + 2 + 7);
                    lines.push((
                        Line::from(vec![Span::raw("    "), Span::styled(format!("{} ", status_mark(t.status)), theme.dim()), Span::styled(format!("T-{} ", t.id), theme.dim()), Span::styled(ui::fit(&t.title, room), theme.dim())]),
                        Some(side),
                    ));
                }
                // Its subagents, under it, while they work.
                for text in subagent_rows(agent) {
                    let room = (area.width as usize).saturating_sub(6);
                    lines.push((Line::from(vec![Span::raw("    "), Span::styled(ui::fit(&text, room), theme.dim())]), Some(side)));
                }
            }
            Side::Ticket(_) => {}
        }
    }
    lines
}

fn render_side(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let lines = side_lines(app, theme, area);
    let top = side_top(app, &lines, area.height as usize);
    frame.render_widget(Paragraph::new(lines.into_iter().skip(top).map(|(line, _)| line).collect::<Vec<_>>()), area);
}

/// Where the side list starts drawing: the open line kept in sight when the
/// list is taller than the pane.
fn side_top(app: &App, lines: &[(Line<'static>, Option<Side>)], height: usize) -> usize {
    let open = lines.iter().position(|(_, s)| app.pane == Pane::Conversation && *s == Some(app.side_open()));
    open.map_or(0, |at| (at + 3).saturating_sub(height))
}

/// Someone in the side list — the PM at its head, the rest of the team at
/// its foot: its mark, its name, and what it is doing.
fn member_line(app: &App, agent: &Agent, open: bool, name_width: usize, area: Rect, theme: Theme) -> Line<'static> {
    let marker = if open { Span::styled("› ", theme.accent()) } else { Span::raw("  ") };
    let name = ui::fit(&agent.label(), name_width);
    let style = theme.member(agent_look(agent).1);
    let name = Span::styled(pad(&name, name_width), if open { style.add_modifier(Modifier::BOLD) } else { style });
    // What it is doing, the most pressing first: needs you, a memo waiting,
    // its ticket, its model.
    let detail = if app.asks_you(agent) {
        Span::styled(t!("view.asks_permission"), theme.accent())
    } else {
        let text = match (agent.queued.len() + agent.steering.len(), agent.ticket) {
            (1, _) if agent.steering.is_empty() => t!("view.one_memo").to_owned(),
            (n @ 2.., _) if agent.steering.is_empty() => t!("view.memos", n = n),
            (n @ 1.., _) => t!("view.unread", n = n),
            (0, Some(ticket)) => format!("T-{ticket}"),
            (0, None) => agent.short_model().unwrap_or_default(),
        };
        // Several of its conversations at work at once.
        let at_once = app.lanes_working(agent) + usize::from(agent.state == AgentState::Working);
        let text = if at_once > 1 { t!("view.at_once", n = at_once) } else { text };
        // When it expects to be done — or what it ships, live.
        let text = match app.estimate_of(agent).and_then(|e| estimate_short(e, now_unix())) {
            Some(left) if text.is_empty() => left,
            Some(left) => format!("{text} {left}"),
            None => text,
        };
        // A conversation past half its context, in sight.
        let text = match agent.context.filter(|(_, w)| *w > 0).map(|(u, w)| u * 100 / w) {
            Some(p) if p >= 50 => format!("{text} · {p}%"),
            _ => text,
        };
        // An open browser, and what it costs in memory.
        let text = match app.browsers.get(&agent.label()) {
            Some(mb) if *mb >= 1024 => format!("{text} · {:.1}G", *mb as f32 / 1024.0),
            Some(mb) => format!("{text} · {mb}M"),
            None => text,
        };
        Span::styled(text, theme.dim())
    };
    // With several projects, the one it works in, in its colour, first; a
    // member busy elsewhere fades while the board is filtered to one project.
    let project = app.agent_project(agent).filter(|_| app.projects.len() > 1 && agent.queued.is_empty() && !app.asks_you(agent));
    let elsewhere = app.focus.is_some() && project.is_some() && project != app.focus;
    let name = if elsewhere { Span::styled(name.content, theme.dim()) } else { name };
    let mut row = vec![marker, glyph(app, agent, theme), Span::raw(" "), name, Span::raw("  ")];
    if let Some(project) = &project {
        // Shortened before the ticket is: in a narrow pane the ticket still shows.
        let used: usize = row.iter().map(Span::width).sum::<usize>() + detail.width() + 1;
        let room = (area.width as usize).saturating_sub(used);
        if room >= 3 {
            row.push(Span::styled(format!("{} ", ui::fit(&project_name(project), room)), project_style(app, Some(project), theme)));
        }
    }
    row.push(detail);
    Line::from(row)
}

/// Most subagents shown under a member before the rest are counted.
const SUBAGENTS_SHOWN: usize = 3;

/// The lines under a member for its subagents at work.
fn subagent_rows(agent: &Agent) -> Vec<String> {
    let mut rows: Vec<String> = agent
        .subagents
        .iter()
        .take(SUBAGENTS_SHOWN)
        .map(|(_, what, step)| match step {
            Some(step) => format!("↳ {what} · {step}"),
            None => format!("↳ {what}"),
        })
        .collect();
    if agent.subagents.len() > SUBAGENTS_SHOWN {
        rows.push(format!("↳ {}", t!("view.more_subagents", count = agent.subagents.len() - SUBAGENTS_SHOWN)));
    }
    rows
}

/// What each row of the squad pane is, from its first member row: a member
/// (by index) or a line under one.
/// The squad's edge; lit while the person drags it.
fn render_hairline(frame: &mut Frame, area: Rect, held: bool, theme: Theme) {
    let style = if held { theme.accent() } else { theme.dim() };
    let lines: Vec<Line> = (0..area.height).map(|_| Line::from(Span::styled(" │", style))).collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// Wrap `text` so its first row follows `lead` and later rows are indented.
fn wrapped(lead: Vec<Span<'static>>, text: &str, style: Style, indent: usize, width: usize) -> Vec<Line<'static>> {
    let text = &*ui::cell_safe(text);
    let lead_width: usize = lead.iter().map(|s| s.width()).sum();
    let mut out = Vec::new();
    let mut lead = Some(lead);
    for (n, para) in text.split('\n').enumerate() {
        let first = if n == 0 { lead_width } else { indent };
        let body = ui::wrap(para, first, indent, width);
        for (i, row) in body.split('\n').enumerate() {
            let mut spans = match (n, i, lead.take()) {
                (0, 0, Some(lead)) => lead,
                _ if i == 0 => vec![Span::raw(" ".repeat(indent))],
                _ => Vec::new(), // `ui::wrap` already indented continuation rows
            };
            spans.push(Span::styled(row.to_owned(), style));
            out.push(Line::from(spans));
        }
    }
    out
}

/// Whether a tool use is a member writing to its own memory — Claude Code's
/// auto memory, which equipe keeps under the squad's members.
fn is_memory(tool: &str, target: &str) -> bool {
    matches!(tool, "Write" | "Edit" | "MultiEdit") && target.contains("/.equipe/") && target.contains("/members/") && target.contains("/memory/")
}

/// One line of the conversation's tool activity. Consecutive reads, searches
/// and fetches merge into one line, so looking around reads as one step.
struct ToolLine {
    verb: String,
    targets: Vec<String>,
    edits: bool,
}

/// How many targets a merged tool line names before it counts the rest.
const TARGETS_SHOWN: usize = 3;

fn tool_line(line: &ToolLine, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let mark = if line.edits { Span::styled("✎ ", theme.accent()) } else { Span::styled("· ", theme.dim()) };
    let mut named: Vec<String> = line.targets.iter().take(TARGETS_SHOWN).cloned().collect();
    if line.targets.len() > TARGETS_SHOWN {
        named.push(format!("+{}", line.targets.len() - TARGETS_SHOWN));
    }
    wrapped(vec![mark], &format!("{} {}", pad(verb_word(&line.verb), 6), named.join(", ")), theme.dim(), 2, width)
}

/// A step's verb as the person reads it — `read`, `edit`, `run` — in their
/// language; a tool equipe has no word for keeps its own name.
pub(super) fn verb_word(verb: &str) -> &str {
    match verb {
        "read" => t!("view.verb_read"),
        "edit" => t!("view.verb_edit"),
        "write" => t!("view.verb_write"),
        "run" => t!("view.verb_run"),
        "search" => t!("view.verb_search"),
        "fetch" => t!("view.verb_fetch"),
        "delegate" => t!("view.verb_delegate"),
        "board" => t!("view.verb_board"),
        "ticket" => t!("view.verb_ticket"),
        "comment" => t!("view.verb_comment"),
        "hand in" => t!("view.verb_hand_in"),
        "wait on you" => t!("view.verb_wait_on_you"),
        "ask you" => t!("view.verb_ask_you"),
        "browse" => t!("view.verb_browse"),
        "recall" => t!("view.verb_recall"),
        other => other,
    }
}

/// A run of tool use longer than this folds to its first and last lines.
const RUN_FOLDS_AT: usize = 6;

/// A run of tool lines; folded to its start and end — with how many steps lie
/// between — unless the person unfolded it (Ctrl-O).
fn tool_run(run: Vec<ToolLine>, expanded: bool, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if expanded || run.len() <= RUN_FOLDS_AT {
        for line in &run {
            lines.extend(tool_line(line, theme, width));
        }
        return lines;
    }
    let (head, tail) = (3, 2);
    for line in &run[..head] {
        lines.extend(tool_line(line, theme, width));
    }
    let hidden = run.len() - head - tail;
    lines.push(Line::from(vec![
        Span::styled(format!("· {} · ", t!("view.more_steps", count = hidden)), theme.dim()),
        Span::styled("ctrl-o", theme.bold()),
        Span::styled(format!(" {}", t!("view.to_unfold")), theme.dim()),
    ]));
    for line in &run[run.len() - tail..] {
        lines.extend(tool_line(line, theme, width));
    }
    lines
}

/// The person's own message: `›` and bold text, on a faint wash of the
/// background when the terminal told us its colour.
/// What the person attached, named rather than spelled out: each tag of a
/// message (`[image: /…/uploads/1792073093606-paste.png]`) as its kind and
/// name — `[이미지] paste.png`. The member still gets the tag as it is.
pub fn attachments_named(text: &str) -> String {
    use equipe_core::protocol::tags::{TagKind, compact};
    compact(text, |tag| {
        let name = tag.name();
        let named = match tag.kind {
            TagKind::Image => t!("view.attached_image", name = name),
            TagKind::Video => t!("view.attached_video", name = name),
            TagKind::Audio => t!("view.attached_audio", name = name),
            TagKind::Pdf => t!("view.attached_pdf", name = name),
            TagKind::File => t!("view.attached_file", name = name),
        };
        match (&tag.duration, tag.pages) {
            (Some(length), _) => format!("{named} · {length}"),
            (None, Some(pages)) => format!("{named} · {}", t!("view.attached_pages", count = pages)),
            _ => named,
        }
    })
}

fn user_message(text: &str, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let Some(tint) = theme.tint() else {
        return wrapped(vec![Span::styled("› ", theme.accent())], text, theme.bold(), 2, width);
    };
    // One column of wash on either side, and the whole width washed.
    let inner = width.saturating_sub(2);
    let washed = |spans: Vec<Span<'static>>| {
        let used: usize = spans.iter().map(Span::width).sum();
        let mut spans: Vec<Span<'static>> = spans.into_iter().map(|s| { let style = s.style.bg(tint); s.style(style) }).collect();
        spans.insert(0, Span::styled(" ", Style::new().bg(tint)));
        spans.push(Span::styled(" ".repeat(width.saturating_sub(used + 1)), Style::new().bg(tint)));
        Line::from(spans)
    };
    let blank = Line::from(Span::styled(" ".repeat(width), Style::new().bg(tint)));
    let mut lines = vec![blank.clone()];
    for line in wrapped(vec![Span::styled("› ", theme.accent())], text, theme.bold(), 2, inner) {
        lines.push(washed(line.spans));
    }
    lines.push(blank);
    lines
}

/// Who is on the squad, as the conversation shows them: name, mark, colour.
type Looks = Vec<(String, &'static str, usize)>;

fn squad_looks(app: &App) -> Looks {
    app.agents
        .iter()
        .map(|a| {
            let (mark, hue) = agent_look(a);
            (a.label(), mark, hue)
        })
        .collect()
}

/// Ticket numbers in a line — `T-8`, `T-12` — stand out: each is a way to its
/// ticket (a click opens it on the board).
fn mark_tickets(line: Line<'static>, theme: Theme) -> Line<'static> {
    if !line.spans.iter().any(|s| s.content.contains("T-")) {
        return line;
    }
    let style = line.style;
    let mut spans = Vec::new();
    for span in line.spans {
        let text = span.content.to_string();
        let mut rest = text.as_str();
        while let Some((at, len)) = ticket_ref(rest) {
            if at > 0 {
                spans.push(Span::styled(rest[..at].to_owned(), span.style));
            }
            spans.push(Span::styled(rest[at..at + len].to_owned(), span.style.patch(theme.accent())));
            rest = &rest[at + len..];
        }
        if !rest.is_empty() {
            spans.push(Span::styled(rest.to_owned(), span.style));
        }
    }
    Line::from(spans).style(style)
}

/// The first `T-<digits>` in `text` that stands as a word: where, and how long.
pub fn ticket_ref(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(found) = text[from..].find("T-") {
        let at = from + found;
        let digits = bytes[at + 2..].iter().take_while(|b| b.is_ascii_digit()).count();
        let before_ok = at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'-');
        if digits > 0 && before_ok {
            return Some((at, 2 + digits));
        }
        from = at + 2;
    }
    None
}

/// What kind of block a conversation line belongs to: a change of kind gets a
/// blank line, so replies, steps and team news read as separate things.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    User,
    Reply,
    Steps,
    News,
    Note,
}

/// A blank line, unless there is one already.
fn gap(lines: &mut Vec<Line<'static>>) {
    if lines.last().is_some_and(|l| l.width() > 0) {
        lines.push(Line::raw(""));
    }
}

/// Start a block of `kind`: a blank line between blocks of different kinds — a
/// message, a reply, a run of steps, team news — and between two replies.
fn begin(kind: Block, lines: &mut Vec<Line<'static>>, last: &mut Option<Block>) {
    if last.is_some_and(|was| was != kind || kind == Block::Reply) {
        gap(lines);
    }
    *last = Some(kind);
}

/// A note about a teammate (`designer handed in T-8 …`) or the person (`You
/// reopened T-6 …`): who, then what, so team news reads apart from replies.
fn news_lines(text: &str, looks: &Looks, theme: Theme, width: usize) -> Option<Vec<Line<'static>>> {
    let (lead, rest) = if let Some(rest) = text.strip_prefix("You ") {
        (vec![Span::styled("› ", theme.accent()), Span::styled("You", theme.bold())], rest)
    } else {
        let (name, mark, hue) = looks.iter().find(|(name, ..)| text.strip_prefix(name.as_str()).is_some_and(|r| r.starts_with(' ')))?;
        let style = theme.member(*hue);
        (vec![Span::styled(format!("{mark} "), style), Span::styled(name.clone(), style.add_modifier(Modifier::BOLD))], &text[name.len() + 1..])
    };
    let mut lead = lead;
    lead.push(Span::raw(" "));
    Some(wrapped(lead, rest, theme.dim(), 2, width).into_iter().map(|l| mark_tickets(l, theme)).collect())
}

/// A ticket waiting for the person, in the conversation: a heading, its
/// asks numbered one under another, and how to answer.
fn your_turn_lines(id: TicketId, title: &str, asks: &[String], theme: Theme, width: usize) -> Vec<Line<'static>> {
    let bar = || Span::styled("▎ ", theme.accent());
    let inner = width.saturating_sub(2);
    let mut out: Vec<Line<'static>> = wrapped(Vec::new(), &t!("view.your_turn_heading", id = id, title = title), theme.accent().add_modifier(Modifier::BOLD), 0, inner);
    for (n, ask) in asks.iter().enumerate() {
        let number = format!("{}. ", n + 1);
        let indent = number.chars().count();
        out.extend(wrapped(vec![Span::styled(number, theme.accent())], ask, Style::new(), indent, inner));
    }
    out.extend(wrapped(Vec::new(), &t!("view.your_turn_answer", id = id), theme.dim(), 0, inner));
    out.into_iter()
        .map(|line| {
            let mut spans = vec![bar()];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

fn transcript_lines(agent: &Agent, looks: &Looks, turns: &Turns, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if agent.shown().is_empty() && agent.queued.is_empty() {
        let thread_hello;
        let hello = if let Some(project) = agent.thread_view.as_deref().filter(|_| agent.is_pm()) {
            thread_hello = t!("view.hello_pm_thread", project = project);
            thread_hello.as_str()
        } else if agent.is_pm() {
            t!("view.hello_pm")
        } else if agent.edit {
            t!("view.hello_member_edits")
        } else {
            t!("view.hello_member")
        };
        return wrapped(Vec::new(), hello, theme.dim(), 0, width);
    }
    let mut last: Option<Block> = None;
    // Tool use gathers into a run until something else is said; a long run folds.
    let mut run: Vec<ToolLine> = Vec::new();
    let flush = |run: &mut Vec<ToolLine>, lines: &mut Vec<Line<'static>>, last: &mut Option<Block>| {
        if !run.is_empty() {
            begin(Block::Steps, lines, last);
            lines.extend(tool_run(std::mem::take(run), agent.expanded, theme, width));
        }
    };
    for entry in agent.drawn() {
        let entry = entry.as_ref();
        match entry {
            Entry::Thread { .. } => {}
            // Claude Code loading tool descriptions: its business, not the person's.
            Entry::Tool { tool, .. } if tool == "ToolSearch" => {}
            Entry::Tool { tool, target, .. } => {
                let (verb, target, edits) = match target {
                    // A member writing down what it learned, in its own memory.
                    Some(t) if is_memory(tool, t) => {
                        let note = std::path::Path::new(t).file_name().map_or_else(|| t.clone(), |n| n.to_string_lossy().into_owned());
                        ("recall".to_owned(), note, false)
                    }
                    _ => {
                        let verb = tool_verb(tool).to_owned();
                        let edits = matches!(verb.as_str(), "edit" | "write");
                        (verb, target.as_deref().map(|t| relative(t, &agent.base)).unwrap_or_default(), edits)
                    }
                };
                // Looking around — reads, searches, fetches — merges into one line.
                let merges = matches!(verb.as_str(), "read" | "search" | "fetch" | "recall" | "browse");
                match run.last_mut() {
                    Some(line) if merges && line.verb == verb => {
                        if !target.is_empty() && !line.targets.contains(&target) {
                            line.targets.push(target);
                        }
                    }
                    _ => run.push(ToolLine { verb, targets: if target.is_empty() { Vec::new() } else { vec![target] }, edits }),
                }
                continue;
            }
            _ => flush(&mut run, &mut lines, &mut last),
        }
        match entry {
            Entry::Thread { .. } => {}
            Entry::User { text } => {
                begin(Block::User, &mut lines, &mut last);
                gap(&mut lines);
                lines.extend(user_message(&attachments_named(text), theme, width));
                lines.push(Line::raw(""));
            }
            Entry::Reply { text } => {
                // The member's mark opens its reply; the rest hangs under it.
                begin(Block::Reply, &mut lines, &mut last);
                let (mark, hue) = agent_look(agent);
                let mut first = true;
                for line in markdown::lines(text, width.saturating_sub(2), &theme.markdown()) {
                    let lead = if first && line.width() > 0 {
                        first = false;
                        Span::styled(format!("{mark} "), theme.member(hue))
                    } else {
                        Span::raw("  ")
                    };
                    let style = line.style;
                    let mut spans = vec![lead];
                    spans.extend(line.spans);
                    lines.push(mark_tickets(Line::from(spans).style(style), theme));
                }
            }
            // Other projects' talk, folded: one quiet line saying how much.
            Entry::Note { text } if text.starts_with(super::app::FOLDED) => {
                let said: usize = text[super::app::FOLDED.len_utf8()..].parse().unwrap_or(0);
                begin(Block::Note, &mut lines, &mut last);
                gap(&mut lines);
                lines.push(Line::from(Span::styled(format!("── {} ──", t!("view.elsewhere", count = said)), theme.dim())));
            }
            // A ticket still waiting for the person: its asks one under another.
            Entry::Note { text } if equipe_core::i18n::waiting_ticket(text).is_some_and(|id| turns.iter().any(|(t, _, asks)| *t == id && !asks.is_empty())) => {
                let id = equipe_core::i18n::waiting_ticket(text).unwrap_or_default();
                if let Some((_, title, asks)) = turns.iter().find(|(t, ..)| *t == id) {
                    begin(Block::Note, &mut lines, &mut last);
                    gap(&mut lines);
                    lines.extend(your_turn_lines(id, title, asks, theme, width));
                }
            }
            Entry::Note { text } => match news_lines(&equipe_core::i18n::note(crate::i18n::lang(), text), looks, theme, width) {
                Some(news) => {
                    begin(Block::News, &mut lines, &mut last);
                    lines.extend(news);
                }
                None => {
                    // A ticket starting, a turn's length: chapters in the member's day.
                    begin(Block::Note, &mut lines, &mut last);
                    if text.starts_with("── ") || text.starts_with("─ worked") {
                        gap(&mut lines);
                    }
                    lines.extend(wrapped(Vec::new(), text, theme.dim(), 0, width).into_iter().map(|l| mark_tickets(l, theme)));
                }
            },
            Entry::Error { message, next } => {
                lines.extend(wrapped(vec![Span::styled("✗ ", theme.bold())], message, Style::new(), 2, width));
                if let Some(next) = next {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("→ ", theme.dim()),
                        Span::styled(next.clone(), theme.bold()),
                    ]));
                }
            }
            Entry::Landed { branch, stat } => {
                let (before, after) = around(t!("view.changes_committed"), "branch");
                lines.push(Line::from(vec![
                    Span::styled("✓ ", theme.accent()),
                    Span::raw(before),
                    Span::styled(branch.clone(), theme.bold()),
                    Span::raw(after),
                ]));
                for row in stat.lines() {
                    lines.push(Line::from(Span::styled(format!("  {}", row.trim()), theme.dim())));
                }
            }
            Entry::Tool { .. } => {}
        }
    }
    flush(&mut run, &mut lines, &mut last);
    // Memos wait below the conversation they go into until the turn ends —
    // each only in the conversation of the project it was left in. On a ticket
    // there is one conversation, the ticket's: every memo is for it.
    let here = |about: &Option<String>| agent.ticket.is_some() || agent.lane.is_some() || agent.in_view(about.as_deref());
    for (memo, _) in agent.queued.iter().zip(&agent.queued_in).filter(|(_, about)| here(about)) {
        gap(&mut lines);
        lines.extend(wrapped(vec![Span::styled("› ", theme.dim())], &attachments_named(memo), theme.dim(), 2, width));
        lines.push(Line::from(Span::styled(format!("  {}", t!("view.memo_note")), theme.dim())));
    }
    // Said while it works: read at its next step, the work carrying on.
    for text in &agent.steering {
        gap(&mut lines);
        lines.extend(wrapped(vec![Span::styled("› ", theme.accent())], &attachments_named(text), theme.dim(), 2, width));
        lines.push(Line::from(Span::styled(format!("  {}", t!("view.steering_note")), theme.dim())));
    }
    lines
}

/// The pane's title line: who, in bold, and what they are on, dim.
fn pane_title(spans: Vec<Span<'static>>, switch: Vec<Span<'static>>, area: Rect, frame: &mut Frame) -> Rect {
    let [title, _, rest] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)]).areas(area);
    // The way to the other pane sits at the title's right, always in sight.
    let used: usize = spans.iter().map(Span::width).sum();
    let switch_width: usize = switch.iter().map(Span::width).sum();
    let mut line = spans;
    let room = title.width as usize;
    if used + switch_width + 3 <= room {
        line.push(Span::raw(" ".repeat(room - used - switch_width)));
        line.extend(switch);
    }
    frame.render_widget(Paragraph::new(Line::from(line)), title);
    rest
}

/// The way from `pane` to the other one: the board's count of reviews, or back to
/// the member's conversation — and the key, Ctrl-B.
fn switch_label(app: &App, pane: Pane, theme: Theme) -> Vec<Span<'static>> {
    match pane {
        Pane::Conversation => {
            let needs = app.needs_you_count();
            let mut spans = vec![Span::styled(t!("view.board"), theme.bold())];
            if needs > 0 {
                spans.push(Span::styled(format!(" · {} {needs}", t!("app.menu_inbox")), theme.accent()));
            }
            spans.push(Span::styled("  ctrl-b", theme.dim()));
            spans
        }
        Pane::Board => {
            let Some(agent) = app.selected() else {
                return vec![Span::styled(format!("← {}", t!("view.conversation")), theme.bold()), Span::styled("  ctrl-b", theme.dim())];
            };
            vec![
                Span::styled("← ", theme.bold()),
                Span::styled(agent.label(), theme.member(agent_look(agent).1).add_modifier(Modifier::BOLD)),
                Span::styled("  ctrl-b", theme.dim()),
            ]
        }
    }
}

/// The watched member's browser: a line saying whose and where, then the
/// latest picture of it — drawn in whatever pictures this terminal shows.
fn render_watch(frame: &mut Frame, area: Rect, app: &App, agent: &Agent, theme: Theme, layouts: &Layouts) {
    let [head, picture, _] = Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(area);
    let name = agent.label();
    let shown = app.frame.as_ref().filter(|f| f.session == agent.id);
    let mut line = vec![Span::styled(t!("view.browser_of", name = name), theme.bold())];
    if let Some(url) = shown.and_then(|f| f.url.as_deref()) {
        line.push(Span::styled(format!("  ·  {url}"), theme.dim()));
    }
    line.push(Span::styled(format!("  ·  {}", t!("view.watch_live")), theme.dim()));
    frame.render_widget(Paragraph::new(Line::from(line)), head);
    let drawn = match (shown, &layouts.picker) {
        (Some(shot), Some(picker)) => {
            let mut picture_cache = layouts.picture.borrow_mut();
            if picture_cache.as_ref().is_none_or(|(seq, _)| *seq != shot.seq)
                && let Ok(image) = image::load_from_memory(&shot.png)
            {
                *picture_cache = Some((shot.seq, picker.new_resize_protocol(image)));
            }
            match picture_cache.as_mut() {
                Some((_, protocol)) => {
                    frame.render_stateful_widget(ratatui_image::StatefulImage::default(), picture, protocol);
                    true
                }
                None => false,
            }
        }
        _ => false,
    };
    if !drawn {
        let text = if shown.is_some() {
            t!("view.watch_cant_draw", name = name)
        } else {
            t!("view.watch_waiting", name = name)
        };
        frame.render_widget(Paragraph::new(Span::styled(text, theme.dim())), picture);
    }
}

/// A ticket open with no one's work on it to show: what the board knows.
fn render_ticket(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let Some(ticket) = app.opened.and_then(|id| app.tickets.iter().find(|t| t.id == id)) else {
        return;
    };
    let title = vec![
        Span::styled(format!("T-{}", ticket.id), theme.accent().add_modifier(Modifier::BOLD)),
        Span::styled(format!("  ·  {}", label::status(crate::i18n::lang(), ticket.status)), theme.dim()),
    ];
    let area = pane_title(title, switch_label(app, Pane::Conversation, theme), area, frame);
    let width = (area.width as usize).saturating_sub(1).min(PROSE_MAX + 20);
    let mut lines = ticket_detail(app, theme, width);
    lines.push(Line::raw(""));
    let hint = ticket_hint(app).unwrap_or_else(|| t!("view.write_goes_to_pm", id = ticket.id));
    lines.push(Line::from(Span::styled(hint, theme.dim())));
    frame.render_widget(Paragraph::new(lines).scroll((app.board_scroll as u16, 0)), area);
}

/// What the person can do with a ticket open whose work is in, in a line.
fn ticket_hint(app: &App) -> Option<String> {
    let ticket = app.opened.and_then(|id| app.tickets.iter().find(|t| t.id == id))?;
    match ticket.status {
        TicketStatus::Review => Some(t!("view.hint_review", id = ticket.id)),
        TicketStatus::Waiting => Some(t!("view.hint_waiting", id = ticket.id)),
        _ => None,
    }
}

fn render_transcript(frame: &mut Frame, area: Rect, app: &App, theme: Theme, layouts: &Layouts) {
    let Some(agent) = app.viewed() else {
        return;
    };
    let mut title = vec![Span::styled(agent.label(), theme.member(agent_look(agent).1).add_modifier(Modifier::BOLD))];
    // A ticket open: its number and title first, whose work it is after.
    if let Some(ticket) = app.opened.and_then(|id| app.tickets.iter().find(|t| t.id == id)) {
        title = vec![
            Span::styled(format!("T-{}", ticket.id), theme.accent().add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {}", ticket.title), theme.bold()),
            Span::styled(format!("  ·  {} · {}", agent.label(), ticket.status.label()), theme.dim()),
        ];
    } else { match agent.ticket.and_then(|id| app.tickets.iter().find(|t| t.id == id)) {
        Some(ticket) => title.push(Span::styled(format!("  ·  T-{} {}", ticket.id, ticket.title), theme.dim())),
        // With every project in view, the PM's is the whole squad's conversation.
        None if app.shows_overview() => title.push(Span::styled(format!("  ·  {}", t!("view.whole_squad")), theme.dim())),
        None => title.push(Span::styled(format!("  ·  {}", agent.engine_label()), theme.dim())),
    } }
    // How full its conversation is: past half, in the accent, with what to do.
    if let Some((used, window)) = agent.context.filter(|(_, w)| *w > 0) {
        let percent = used * 100 / window;
        let style = if percent >= 50 { theme.accent() } else { theme.dim() };
        title.push(Span::styled(format!("  ·  {}", t!("view.context_percent", percent = percent)), style));
        if percent >= 50 {
            title.push(Span::styled(" · /compact", theme.dim()));
        }
    }
    let area = pane_title(title, switch_label(app, Pane::Conversation, theme), area, frame);

    // At the bottom, when it applies: the live working row, or what arrived
    // while the person was scrolled up.
    let mut footer: Vec<Line<'static>> = Vec::new();
    if agent.scroll_back > 0 && agent.unseen > 0 {
        footer.push(Line::from(vec![
            Span::styled(t!("view.new_lines", count = agent.unseen), theme.accent()),
            Span::styled(" · End", theme.dim()),
        ]));
    } else if agent.state == AgentState::Working {
        let now = Instant::now();
        let took = agent.working_since.map(|since| elapsed(now.duration_since(since))).unwrap_or_default();
        let mut row = vec![theme.spinner(now), Span::raw(" ")];
        // A turn in another project's conversation says where.
        if !agent.in_view(agent.current_thread()) {
            row.push(Span::styled(format!("{} · ", t!("view.working_in", place = agent.current_thread().unwrap_or(t!("view.the_squad")))), theme.dim()));
        }
        row.extend(theme.shimmer(doing(agent), now));
        let shown = agent.shown_estimate();
        let expected = shown.and_then(|e| Some(done_phrase(e.done_at?, e.guessed, now_unix())));
        let live = shown.and_then(|e| live_phrase(e, now_unix()));
        row.extend([
            Span::styled(format!(" · {took}"), theme.dim()),
            Span::styled(expected.map(|e| format!(" · {e}")).unwrap_or_default(), theme.accent()),
            Span::styled(live.map(|l| format!(" · {l}")).unwrap_or_default(), theme.accent()),
            // What it has written so far, as Claude Code shows it.
            Span::styled(if agent.output > 0 { format!(" · ↓ {}", short_count(agent.output)) } else { String::new() }, theme.dim()),
            Span::styled(format!(" · {}", t!("view.esc_to_stop")), theme.dim()),
        ]);
        footer.push(Line::from(row));
    } else if let Some(live) = agent.estimate.as_ref().and_then(|e| live_phrase(e, now_unix())) {
        // Between turns, what it shipped is on its way.
        footer.push(Line::from(vec![Span::styled("◷ ", theme.accent()), Span::styled(live, theme.accent())]));
    } else if let Some(hint) = ticket_hint(app) {
        footer.push(Line::from(Span::styled(hint, theme.dim())));
    }
    let Areas { watch, overview: projects, text, card, live } = conversation_areas(area, app, agent);
    if watch.height > 0 {
        render_watch(frame, watch, app, agent, theme, layouts);
    }
    if projects.height > 0 {
        let rows: Vec<Line> = overview(app, theme, projects.width as usize).into_iter().map(|(line, _)| line).take(projects.height as usize).collect();
        frame.render_widget(Paragraph::new(rows), projects);
    }
    if card.height > 0 {
        let rows: Vec<Line> = waiting_card(app, theme, card.width as usize).into_iter().map(|(line, _)| line).collect();
        frame.render_widget(Paragraph::new(rows), card);
    }

    let width = (text.width as usize).saturating_sub(1).min(PROSE_MAX);
    let fresh;
    let lines: &[Line<'static>] = match layouts.get(agent, width, &turns(app)) {
        Some(lines) => lines,
        None => {
            fresh = transcript_lines(agent, &squad_looks(app), &turns(app), theme, width);
            &fresh
        }
    };
    // Follow the bottom unless the person scrolled up; draw only what shows.
    let height = text.height as usize;
    let bottom = lines.len().saturating_sub(height);
    let top = bottom.saturating_sub(agent.scroll_back.min(bottom));
    let shown = lines[top..lines.len().min(top + height)].to_vec();
    frame.render_widget(Paragraph::new(shown), text);
    reverse_selection(frame, app, text, top, width);
    // Scrolled up, a thin bar on the right says where in the conversation this is.
    if top < bottom {
        render_scrollbar(frame, text, top, height, lines.len(), theme);
    }
    if !footer.is_empty() {
        frame.render_widget(Paragraph::new(footer), live);
    }
}

/// What the person is selecting in `area` (showing lines from `top`), in reverse.
fn reverse_selection(frame: &mut Frame, app: &App, area: Rect, top: usize, width: usize) {
    let Some((from, to)) = app.selection.map(ordered) else {
        return;
    };
    let buffer = frame.buffer_mut();
    for row in 0..area.height {
        let line = top + row as usize;
        if line < from.0 || line > to.0 {
            continue;
        }
        let start = if line == from.0 { from.1 } else { 0 };
        let end = if line == to.0 { to.1 + 1 } else { width as u16 };
        for column in start..end.min(area.width) {
            if let Some(cell) = buffer.cell_mut((area.x + column, area.y + row)) {
                cell.set_style(cell.style().add_modifier(Modifier::REVERSED));
            }
        }
    }
}

/// A selection from where it starts to where it ends, whichever way it was dragged.
fn ordered((a, b): (Spot, Spot)) -> (Spot, Spot) {
    if a <= b { (a, b) } else { (b, a) }
}

/// The text the person selected in the conversation, as it reads: each line's
/// selected columns, trailing spaces trimmed, the marks that open a reply or a
/// step left out.
pub fn selection_text(app: &App, theme: Theme, screen: Rect, layouts: &Layouts) -> Option<String> {
    let (from, to) = ordered(app.selection?);
    let body = rows(app, theme, screen).body;
    let (_, _, right) = columns(app, body);
    if app.pane == Pane::Board {
        let board = under_title(right);
        let width = (board.width as usize).saturating_sub(1).min(PROSE_MAX + 20);
        let (_, detail) = board_areas(board, board_list(app, theme, width).len(), app.board_split);
        let lines = ticket_detail(app, theme, detail_width(detail_body(detail)));
        return selected_in(&lines, from, to);
    }
    let agent = app.viewed()?;
    let width = (conversation_text(right, app, agent).width as usize).saturating_sub(1).min(PROSE_MAX);
    let fresh;
    let lines: &[Line<'static>] = match layouts.get(agent, width, &turns(app)) {
        Some(lines) => lines,
        None => {
            fresh = transcript_lines(agent, &squad_looks(app), &turns(app), theme, width);
            &fresh
        }
    };
    selected_in(lines, from, to)
}

/// The text between two spots of laid-out lines.
fn selected_in(lines: &[Line<'static>], from: Spot, to: Spot) -> Option<String> {
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate().take(to.0 + 1).skip(from.0) {
        let start = if i == from.0 { from.1 as usize } else { 0 };
        let end = if i == to.0 { to.1 as usize + 1 } else { usize::MAX };
        let mut text = String::new();
        let mut column = 0;
        for c in line.spans.iter().flat_map(|s| s.content.chars()) {
            let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if column >= start && column < end {
                text.push(c);
            }
            column += w;
        }
        let text = text.trim_end();
        let mut chars = text.chars();
        let text = match (chars.next(), chars.next()) {
            (Some(mark), Some(' ')) if start == 0 && "◆✦●■◎▲◇★▼✚›·✎".contains(mark) => &text[mark.len_utf8() + 1..],
            _ => text,
        };
        out.push(text.to_owned());
    }
    let text = out.join("\n").trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// A one-column scrollbar at the right edge of `area`: the thumb's size is the
/// share of the text that shows, its place where that share sits.
fn render_scrollbar(frame: &mut Frame, area: Rect, top: usize, height: usize, total: usize, theme: Theme) {
    if area.width == 0 || total <= height || height == 0 {
        return;
    }
    let thumb = (height * height / total).clamp(1, height);
    let travel = height - thumb;
    let at = if total > height { top * travel / (total - height) } else { 0 };
    let column = Rect { x: area.x + area.width - 1, width: 1, ..area };
    let rows: Vec<Line> = (0..height)
        .map(|row| {
            if row >= at && row < at + thumb {
                Line::from(Span::styled("┃", theme.dim()))
            } else {
                Line::from(Span::styled("│", theme.dim().add_modifier(Modifier::DIM)))
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), column);
}

/// A ticket that waits on the person.
fn for_you(t: &equipe_core::protocol::Ticket) -> bool {
    section_of(t).is_some()
}

/// A status as one ASCII mark, so it lines up in any terminal: proposed `.`,
/// to do `o`, in progress `*`, 내 차례 `!`, 검토 `?`, done `+`, cancelled `x`.
pub fn status_mark(status: TicketStatus) -> &'static str {
    match status {
        TicketStatus::Proposed => ".",
        TicketStatus::Todo => "o",
        TicketStatus::InProgress => "*",
        TicketStatus::Waiting => "!",
        TicketStatus::Review => "?",
        TicketStatus::Done => "+",
        TicketStatus::Cancelled => "x",
    }
}

/// `+42 −7` from a branch's `git diff --stat`, when the board has it.
fn diff_short(app: &App, t: &equipe_core::protocol::Ticket) -> Option<String> {
    let (branch, stat) = app.diffs.get(&t.id)?;
    if Some(branch) != t.branch.as_ref() {
        return None;
    }
    let last = stat.lines().last()?;
    let number = |word: &str| last.split(',').find(|p| p.contains(word)).and_then(|p| p.split_whitespace().next()).and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
    Some(format!("+{} -{}", number("insertion"), number("deletion")))
}

/// What a row says after its title: what it waits for and unblocks, then
/// what is happening — the member's step, what the person has to do, who
/// proposed it, how big the change is — and when it is expected done.
fn row_meta(app: &App, t: &equipe_core::protocol::Ticket, theme: Theme) -> (Vec<Span<'static>>, String, Style, String) {
    let mut flags = Vec::new();
    if let Some(dep) = app.blocked_by(t).filter(|_| matches!(t.status, TicketStatus::Todo | TicketStatus::Proposed)) {
        flags.push(Span::styled(format!("{} ", t!("view.flag_blocked", id = dep)), theme.member(6)));
    }
    let unblocks = app.unblocks(t.id).len();
    if unblocks > 0 && !t.status.is_closed() {
        flags.push(Span::styled(format!("->{unblocks} "), theme.dim()));
    }
    if t.waits_to_merge() {
        flags.push(Span::styled(format!("{} ", t!("app.flag_not_on_main")), theme.accent()));
    } else if t.not_on_main() {
        flags.push(Span::styled(format!("{} ", t!("app.flag_kept")), theme.dim()));
    }
    let on_it = app.agents.iter().find(|a| a.ticket == Some(t.id) && a.is_open());
    let (activity, style) = match t.status {
        TicketStatus::InProgress => match on_it {
            Some(agent) if agent.state == AgentState::Working => (doing(agent).to_owned(), theme.member(0)),
            Some(agent) if agent.state == AgentState::Failed => (t!("view.flag_error").to_owned(), theme.member(1)),
            _ => (String::new(), theme.dim()),
        },
        TicketStatus::Waiting => (t.asks().into_iter().next().unwrap_or_default(), theme.accent()),
        TicketStatus::Proposed => (t!("view.proposed_by", who = t.created_by), theme.dim()),
        TicketStatus::Review | TicketStatus::Done if t.kind.in_home() => (label::kind(crate::i18n::lang(), t.kind).to_owned(), theme.dim()),
        TicketStatus::Review | TicketStatus::Done => (diff_short(app, t).unwrap_or_default(), theme.dim()),
        _ => (String::new(), theme.dim()),
    };
    let eta = on_it.and_then(|a| app.estimate_of(a)).and_then(|e| estimate_short(e, now_unix())).filter(|_| t.status == TicketStatus::InProgress).unwrap_or_default();
    (flags, activity, style, eta)
}

/// The board's list, as the menu asks: 나에게 by section, 작업 중, or 모든 일
/// by status with the finished folded into one line (e). One row a ticket:
/// its mark, number, project, title, flags, what is happening, when done, who.
fn board_list(app: &App, theme: Theme, width: usize) -> Vec<(Line<'static>, Option<u32>)> {
    let order = app.board_order();
    let picked = app.selected_ticket().map(|t| t.id);
    let query = app.board_query();
    let heading = |t: &equipe_core::protocol::Ticket| -> String {
        if let Some(q) = &query {
            return t!("view.heading_found", query = q);
        }
        match app.board_menu {
            BoardMenu::Inbox => section_of(t).map_or_else(String::new, |s| s.label().to_owned()),
            BoardMenu::Working => status_word(TicketStatus::InProgress).to_owned(),
            BoardMenu::All if t.status.is_closed() => status_word(TicketStatus::Done).to_owned(),
            BoardMenu::All => status_word(t.status).to_owned(),
        }
    };
    let mut rows: Vec<(Line<'static>, Option<u32>)> = Vec::new();
    // What waits for the person but is no ticket: requests for permission, first.
    if app.board_menu == BoardMenu::Inbox && query.is_none() && !app.approvals.is_empty() {
        rows.push((Line::from(Span::styled(format!("{}  {}", Section::Permission.label(), app.approvals.len()), theme.accent().add_modifier(Modifier::BOLD))), None));
        for a in &app.approvals {
            let ticket = a.ticket.map(|t| format!("T-{t}")).unwrap_or_default();
            let first = a.action.lines().next().unwrap_or_default();
            let room = width.saturating_sub(2 + 2 + 7 + cols(&a.member) + 3);
            rows.push((
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled("! ", theme.accent()),
                    Span::styled(format!("{ticket:<7}"), theme.dim()),
                    Span::styled(format!("{}  ", a.member), theme.bold()),
                    Span::styled(ui::fit(first, room), theme.dim()),
                ]),
                None,
            ));
        }
        rows.push((Line::from(Span::styled(format!("    {}", t!("view.answer_in_conversation")), theme.dim())), None));
    }
    let mut current: Option<String> = None;
    // Each section's whole size, so a folded one says what it left out.
    let mut totals: HashMap<String, usize> = HashMap::new();
    if query.is_none() && app.board_menu == BoardMenu::All {
        for t in app.tickets.iter().filter(|t| app.in_focus(t)) {
            *totals.entry(heading(t)).or_default() += 1;
        }
    }
    let shown = |label: &str| order.iter().filter_map(|id| app.tickets.iter().find(|t| t.id == *id)).filter(|t| heading(t) == label).count();
    let folded = |label: &str| -> Option<Line<'static>> {
        if label == status_word(TicketStatus::Done) {
            return None;
        }
        let more = totals.get(label).copied().unwrap_or(0).saturating_sub(shown(label));
        (more > 0).then(|| Line::from(Span::styled(format!("    … {}", t!("view.folded", n = more, how = t!("view.fold_more"))), theme.dim())))
    };
    let tags = tag_width(app);
    for id in &order {
        let Some(t) = app.tickets.iter().find(|t| t.id == *id) else {
            continue;
        };
        let label = heading(t);
        if current.as_ref() != Some(&label) {
            if let Some(line) = current.as_deref().and_then(folded) {
                rows.push((line, None));
            }
            if current.is_some() || !rows.is_empty() {
                rows.push((Line::raw(""), None));
            }
            let count = totals.get(&label).copied().unwrap_or_else(|| shown(&label));
            let style = if for_you(t) && app.board_menu != BoardMenu::All || matches!(t.status, TicketStatus::Review | TicketStatus::Waiting) { theme.accent().add_modifier(Modifier::BOLD) } else { theme.dim() };
            rows.push((Line::from(Span::styled(format!("{label}  {count}"), style)), None));
            // What a section whose name alone doesn't say means, under it.
            if app.board_menu == BoardMenu::Inbox && query.is_none() && section_of(t) == Some(Section::NotOnMain) {
                rows.extend(wrapped(Vec::new(), t!("app.not_on_main_hint"), theme.dim(), 0, width).into_iter().map(|line| (line, None)));
            }
            current = Some(label);
        }
        let selected = picked == Some(t.id);
        let who = t.assignee.as_ref().or(t.owner.as_ref()).cloned().unwrap_or_default();
        let (flags, activity, activity_style, eta) = row_meta(app, t, theme);
        let flags_width: usize = flags.iter().map(Span::width).sum();
        // Columns: marker, mark, id, project, title (what's left), flags, activity, when, who.
        let activity_width = 18.min(width / 5);
        let tail = flags_width + activity_width + 1 + 7 + 10;
        let title_room = width.saturating_sub(2 + 2 + 7 + tags + 2 + tail).max(8);
        let title = ui::fit(&t.title, title_room);
        let gap = title_room.saturating_sub(cols(&title));
        let closed = t.status.is_closed();
        let title_style = match (selected, closed) {
            (true, _) => theme.bold(),
            (false, true) => theme.dim(),
            (false, false) => Style::new(),
        };
        let mark_style = match t.status {
            TicketStatus::Review | TicketStatus::Waiting => theme.accent(),
            TicketStatus::InProgress => theme.member(0),
            _ => theme.dim(),
        };
        let activity = ui::fit(&activity, activity_width);
        let mut spans = vec![
            if selected { Span::styled("› ", theme.accent()) } else { Span::raw("  ") },
            Span::styled(format!("{} ", status_mark(t.status)), mark_style),
            Span::styled(format!("{:<7}", format!("T-{}", t.id)), if selected { theme.bold() } else { theme.dim() }),
            project_tag(app, t, theme, tags),
            Span::raw("  "),
            Span::styled(title, title_style),
            Span::raw(" ".repeat(gap + 1)),
        ];
        spans.extend(flags);
        spans.extend([
            Span::styled(format!("{:>w$} ", activity, w = activity_width.saturating_sub(cols(&activity).saturating_sub(activity.chars().count()))), activity_style),
            Span::styled(format!("{:>6} ", ui::fit(&eta, 6)), theme.dim()),
            Span::styled(ui::fit(&who, 10), if closed { theme.dim() } else { member_style(app, &who, theme) }),
        ]);
        rows.push((Line::from(spans), Some(t.id)));
    }
    if let Some(line) = current.as_deref().and_then(folded) {
        rows.push((line, None));
    }
    // 모든 일: the finished, folded into one line until shown.
    if app.board_menu == BoardMenu::All && query.is_none() && !app.board_expanded {
        let done = app.tickets.iter().filter(|t| t.status.is_closed() && app.in_focus(t)).count();
        if done > 0 {
            rows.push((Line::raw(""), None));
            rows.push((Line::from(vec![Span::styled(format!("{}  {done} >", status_word(TicketStatus::Done)), theme.dim()), Span::styled("  e", theme.bold()), Span::styled(format!(" {}", t!("view.fold_show")), theme.dim())]), None));
        }
    }
    rows
}

/// The picked ticket in full: what it is, who, what changed, where to look at
/// it, and the keys that act on it.
fn ticket_detail(app: &App, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let Some(t) = app.selected_ticket() else {
        // Empty because of the focus, not because there's no work.
        if let Some(focus) = app.focus.as_deref().filter(|_| !app.tickets.is_empty()) {
            return wrapped(
                Vec::new(),
                &t!("view.nothing_for_project", project = project_name(focus)),
                theme.dim(),
                0,
                width,
            );
        }
        return wrapped(
            Vec::new(),
            t!("view.board_empty"),
            theme.dim(),
            0,
            width,
        );
    };
    let mut lines = wrapped(vec![Span::styled(format!("T-{}  ", t.id), theme.bold())], &ui::cell_safe(&t.title), theme.bold(), 0, width);
    let mut facts = vec![label::kind(crate::i18n::lang(), t.kind).to_owned()];
    if let Some(who) = t.assignee.as_ref().or(t.owner.as_ref()) {
        facts.push(who.clone());
    }
    facts.push(match t.off_main {
        _ if !t.not_on_main() => status_word(t.status),
        Some(equipe_core::protocol::OffMain::Kept) => t!("app.accepted_kept"),
        _ => t!("app.accepted_not_on_main"),
    }.to_owned());
    if let Some(project) = t.project.as_deref().filter(|_| app.projects.len() > 1) {
        facts.push(project_name(project));
    }
    lines.extend(wrapped(Vec::new(), &facts.join(" · "), theme.dim(), 0, width));
    // What it waits for, and what waits for it.
    let mark = status_mark;
    let named = |id: TicketId| -> String {
        match app.tickets.iter().find(|t| t.id == id) {
            Some(d) => format!("T-{id} {} {}", mark(d.status), ui::fit(&d.title, 28)),
            None => format!("T-{id}"),
        }
    };
    let unblocks: Vec<TicketId> = app.tickets.iter().filter(|o| o.depends_on.contains(&t.id) && !o.status.is_closed()).map(|o| o.id).collect();
    let links = [(t!("view.label_after"), t.depends_on.clone()), (t!("view.label_unblocks"), unblocks)];
    let column = links.iter().map(|(label, _)| cols(label) + 1).max().unwrap_or(0).max(10);
    for (label, ids) in links {
        if !ids.is_empty() {
            let text = ids.into_iter().map(named).collect::<Vec<_>>().join(" · ");
            lines.push(Line::from(vec![Span::styled(pad(label, column), theme.dim()), Span::raw(ui::fit(&text, width.saturating_sub(column)))]));
        }
    }
    // The keys come right under the title, so a long hand-in never pushes
    // them out of sight.
    let who = t.assignee.as_ref().or(t.owner.as_ref());
    let mut actions: Vec<(&str, String)> = Vec::new();
    let way = app.merge_ways.get(&app.ticket_project(t).map(|p| project_name(&p)).unwrap_or_default()).copied();
    match t.status {
        TicketStatus::Review => {
            let accept = match way.filter(|_| needs_way(t)) {
                Some(way) => format!("{} · {}", t!("view.action_accept"), way.label()),
                None => t!("view.action_accept").into(),
            };
            actions.push(("a", accept));
            if needs_way(t) {
                actions.push(("A", t!("view.action_how").into()));
            }
            actions.push(("r", t!("view.action_ask_changes").into()));
        }
        TicketStatus::Waiting => {
            actions.push(("a", t!("view.action_did_it").into()));
            actions.push(("r", t!("view.action_resume").into()));
        }
        TicketStatus::Proposed => {
            actions.push(("a", t!("view.action_take").into()));
            actions.push(("n", t!("view.action_drop").into()));
        }
        TicketStatus::Done => {
            if t.not_on_main() {
                actions.push(("m", t!("view.action_merge").into()));
            }
            actions.push(("r", t!("view.action_reopen").into()));
        }
        TicketStatus::InProgress | TicketStatus::Todo | TicketStatus::Cancelled => {}
    }
    if matches!(t.status, TicketStatus::Review | TicketStatus::Waiting | TicketStatus::InProgress | TicketStatus::Todo) {
        actions.push(("n", t!("view.action_cancel").into()));
    }
    if t.branch.is_some() {
        actions.push(("d", t!("view.action_diff").into()));
    }
    if let Some(who) = who {
        actions.push(("c", t!("view.action_talk_to", name = who)));
    }
    actions.push(("j k", t!("view.action_tickets").into()));
    actions.push(("space", t!("view.action_fold").into()));
    actions.push(("g i/l/b", t!("view.action_menus").into()));
    actions.push(("f", t!("view.action_find").into()));
    let mut keys = Vec::new();
    for (i, (k, what)) in actions.into_iter().enumerate() {
        if i > 0 {
            keys.push(Span::raw("   "));
        }
        // The call the person is here to make stands out.
        let style = if i == 0 && for_you(t) { theme.accent().add_modifier(Modifier::BOLD) } else { theme.bold() };
        keys.push(Span::styled(k.to_owned(), style));
        keys.push(Span::styled(format!(" {what}"), theme.dim()));
    }
    // The board's keys, on the board; opened from the side list, the
    // composer and its hint below do it.
    if app.pane == Pane::Board {
        lines.push(Line::from(keys));
    }
    if let Some(note) = &t.note {
        lines.push(Line::raw(""));
        lines.extend(markdown::lines(note, width, &theme.markdown()));
    } else if !t.body.is_empty() {
        lines.push(Line::raw(""));
        lines.extend(markdown::lines(&t.body, width, &theme.markdown()));
    }
    // What changed, and where to see it.
    let (changed, branch_label, look_in) = (t!("view.label_changed"), t!("view.label_branch"), t!("view.label_look_in"));
    let column = [changed, branch_label, look_in].iter().map(|k| cols(k) + 1).max().unwrap_or(0).max(9);
    let key = |k: &str| Span::styled(pad(k, column), theme.dim());
    let mut place = Vec::new();
    if let Some((branch, stat)) = app.diffs.get(&t.id).filter(|(b, _)| Some(b) == t.branch.as_ref()) {
        let files: Vec<&str> = stat.lines().filter(|l| l.contains('|')).map(|l| l.split('|').next().unwrap_or("").trim()).collect();
        let summary = stat.lines().last().map(str::trim).unwrap_or("").to_owned();
        if !summary.is_empty() {
            place.push(Line::from(vec![key(changed), Span::raw(summary)]));
            let shown: Vec<&str> = files.iter().take(4).copied().collect();
            let more = if files.len() > 4 { format!(" +{}", files.len() - 4) } else { String::new() };
            place.push(Line::from(vec![key(""), Span::styled(ui::fit(&format!("{}{more}", shown.join(" · ")), width.saturating_sub(column)), theme.dim())]));
        }
        // The squad's folder shows no git words.
        if !t.kind.in_home() {
            place.push(Line::from(vec![key(branch_label), Span::styled(branch.clone(), theme.dim())]));
        }
    } else if let Some(branch) = t.branch.as_ref().filter(|_| !t.kind.in_home()) {
        place.push(Line::from(vec![key(branch_label), Span::styled(branch.clone(), theme.dim())]));
    }
    if let Some(worktree) = &t.worktree {
        place.push(Line::from(vec![key(look_in), Span::styled(display_path(worktree), theme.dim())]));
    }
    if !place.is_empty() {
        lines.push(Line::raw(""));
        lines.extend(place);
    }
    lines
}

/// The board pane split: the list on top — as tall as it needs, up to half —
/// and the picked ticket's detail below.
fn board_areas(area: Rect, list_rows: usize, split: Option<u16>) -> (Rect, Rect) {
    // As tall as it needs, up to half — or as the person dragged it.
    let list_height = match split {
        Some(rows) => rows.clamp(3.min(area.height), area.height.saturating_sub(5).max(3)),
        None => (list_rows as u16).min(area.height / 2).max(3.min(area.height)),
    };
    let [list, _, detail] =
        Layout::vertical([Constraint::Length(list_height), Constraint::Length(1), Constraint::Min(1)]).areas(area);
    (list, detail)
}

/// Where the list starts, so the picked ticket's row is always in sight.
fn list_top(rows: &[(Line<'static>, Option<u32>)], picked: Option<u32>, height: usize) -> usize {
    let at = rows.iter().position(|(_, id)| id.is_some() && *id == picked).unwrap_or(0);
    if at < height { 0 } else { at + 1 - height }
}

fn render_board(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let title = vec![Span::styled(app.board_menu.label(), theme.bold()), Span::styled(format!("  {}", app.menu_count(app.board_menu)), theme.dim())];
    let area = pane_title(title, switch_label(app, Pane::Board, theme), area, frame);
    let width = (area.width as usize).saturating_sub(1).min(PROSE_MAX + 20);
    let rows = board_list(app, theme, width);
    // Space folds the ticket below away: the list takes the pane.
    if !app.board_detail_open {
        let top = list_top(&rows, app.selected_ticket().map(|t| t.id), area.height as usize);
        let shown: Vec<Line> = rows.into_iter().skip(top).take(area.height as usize).map(|(line, _)| line).collect();
        frame.render_widget(Paragraph::new(shown), area);
        return;
    }
    let (list, detail) = board_areas(area, rows.len(), app.board_split);
    let top = list_top(&rows, app.selected_ticket().map(|t| t.id), list.height as usize);
    let shown: Vec<Line> = rows.into_iter().skip(top).take(list.height as usize).map(|(line, _)| line).collect();
    frame.render_widget(Paragraph::new(shown), list);
    // A hairline between the list and the ticket open below it.
    let [rule, _] = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(detail);
    // Held to drag, it lights up, as the squad's line does.
    let held = if app.is_dragging_board_rule() { theme.accent() } else { theme.dim() };
    frame.render_widget(Paragraph::new(Span::styled("─".repeat(width.min(rule.width as usize)), held)), rule);
    let body = detail_body(detail);
    let lines = ticket_detail(app, theme, detail_width(body));
    let height = body.height as usize;
    let top = app.board_scroll.min(lines.len().saturating_sub(height));
    let below = lines.len().saturating_sub(top + height);
    let mut shown: Vec<Line> = lines.into_iter().skip(top).take(height).collect();
    // A ticket longer than its room says so, at the edge where more is.
    if below > 0 && height > 2
        && let Some(last) = shown.last_mut()
    {
        *last = Line::from(vec![
            Span::styled(format!("↓ {} · ", t!("view.more_lines", count = below + 1)), theme.accent()),
            Span::styled("PgDn", theme.bold()),
            Span::styled(format!(" {}", t!("view.or_scroll_here")), theme.dim()),
        ]);
    }
    if top > 0 && height > 2
        && let Some(first) = shown.first_mut()
    {
        *first = Line::from(vec![Span::styled(format!("↑ {} · ", t!("view.lines_above", count = top + 1)), theme.dim()), Span::styled("PgUp", theme.bold())]);
    }
    frame.render_widget(Paragraph::new(shown), body);
    reverse_selection(frame, app, body, top, detail_width(body));
}

/// Under the hairline: where the open ticket is shown.
fn detail_body(detail: Rect) -> Rect {
    Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas::<2>(detail)[1]
}

fn detail_width(body: Rect) -> usize {
    (body.width as usize).saturating_sub(1).min(PROSE_MAX)
}

/// The oldest request for permission, held in place above the composer behind
/// one accent bar — what exactly, who, why, and the keys that answer it.
fn approval_lines(app: &App, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let Some(approval) = app.approvals.first() else {
        return Vec::new();
    };
    let verb = match approval.tool.as_str() {
        "Bash" => t!("view.verb_run"),
        "Write" | "Edit" | "MultiEdit" => t!("view.verb_edit"),
        "WebFetch" => t!("view.verb_fetch"),
        "Confirm" => t!("view.verb_do"),
        equipe_core::desktop_gate::SEND => t!("view.verb_send"),
        equipe_core::permissions::BROWSER_CODE => t!("view.verb_code"),
        other => other,
    };
    let bar = || Span::styled("▎ ", theme.accent());
    let width = width.saturating_sub(2);
    let ticket = approval.ticket.map(|t| format!(" · T-{t}")).unwrap_or_default();
    let mut lines = vec![Line::from(vec![
        bar(),
        Span::styled(approval.member.clone(), theme.bold()),
        Span::styled(format!(" {}{ticket}", t!("view.asks")), theme.accent()),
    ])];
    // A few lines of the command, never the whole script: the conversation keeps
    // its room, and the keys always show.
    // Code is what's being allowed: more of it shows.
    // A send shows its draft: the person reads what goes out.
    let shown = match approval.tool.as_str() {
        equipe_core::permissions::BROWSER_CODE => 12,
        equipe_core::desktop_gate::SEND => 8,
        _ => 3,
    };
    let rows: Vec<&str> = approval.action.lines().collect();
    for (i, row) in rows.iter().take(shown).enumerate() {
        let lead = if i == 0 { format!("{verb}  ") } else { " ".repeat(cols(verb) + 2) };
        let room = width.saturating_sub(cols(&lead));
        lines.push(Line::from(vec![bar(), Span::styled(lead, theme.dim()), Span::styled(ui::fit(row, room), theme.bold())]));
    }
    let writes = files_written(&approval.action);
    if !writes.is_empty() {
        lines.push(Line::from(vec![bar(), Span::styled(ui::fit(&t!("view.writes", files = writes.join(", ")), width), theme.dim())]));
    }
    if rows.len() > shown {
        lines.push(Line::from(vec![bar(), Span::styled(format!("… {}", t!("view.more_lines", count = rows.len() - shown)), theme.dim())]));
    }
    if let Some(reason) = &approval.reason {
        lines.push(Line::from(vec![bar(), Span::styled(ui::fit(reason, width), theme.dim())]));
    }
    if let Some(preview) = &approval.preview {
        lines.push(Line::from(vec![
            bar(),
            Span::styled(format!("{}  ", t!("view.screen")), theme.dim()),
            Span::styled(ui::fit(preview, width.saturating_sub(20)), theme.dim()),
            Span::styled("  o", theme.bold()),
            Span::styled(format!(" {}", t!("view.open")), theme.dim()),
        ]));
    }
    // "Always" only where it can become a rule — a chain, an edit outside the
    // worktree, is allowed once at a time; offering more would be a promise
    // equipe can't keep.
    if app.pane == Pane::Board {
        // On the board these keys act on tickets; answers wait for the conversation.
        lines.push(Line::from(vec![bar(), Span::styled("ctrl-b", theme.bold()), Span::styled(format!(" {}", t!("view.to_answer")), theme.dim())]));
        return lines;
    }
    let mut keys = vec![bar(), Span::styled("y/a", theme.bold()), Span::styled(format!(" {}   ", t!("view.allow")), theme.dim())];
    if let Some(rule) = equipe_core::permissions::rule_for(&approval.tool, &approval.action) {
        keys.push(Span::styled("A", theme.bold()));
        keys.push(Span::styled(format!(" {}   ", t!("view.always_allow", what = always_label(&rule))), theme.dim()));
    }
    keys.extend([Span::styled("n", theme.bold()), Span::styled(format!(" {}", t!("view.decline")), theme.dim())]);
    if app.approvals.len() > 1 {
        keys.push(Span::styled(format!("   · {}", t!("view.more_waiting", n = app.approvals.len() - 1)), theme.dim()));
    }
    lines.push(Line::from(keys));
    // A send asks in every mode: no mode stops it.
    if approval.tool == equipe_core::desktop_gate::SEND {
        return lines;
    }
    // Asked too often? The way to stop it, right where the asking is.
    let member = &approval.member;
    lines.push(Line::from(vec![
        bar(),
        Span::styled(ui::fit(&t!("view.mode_hint", member = member), width), theme.dim()),
    ]));
    lines
}

/// What an "always" answer would allow from now on, in short: `npm install`,
/// `docs.rs`, a folder.
fn always_label(rule: &str) -> String {
    let inner = rule.split_once('(').and_then(|(_, r)| r.strip_suffix(')')).unwrap_or(rule);
    let short = inner.trim_end_matches(":*").trim_start_matches("domain:");
    let short = short.strip_prefix('/').and_then(|p| p.strip_suffix("/**")).map_or_else(|| short.to_owned(), |dir| format!("{}/", display_path(std::path::Path::new(dir))));
    ui::fit(&short, 40)
}

/// The files a shell script writes with `cat > file`, `cat >> file` or `tee file`.
fn files_written(script: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in script.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        for (i, word) in words.iter().enumerate() {
            let target = match *word {
                ">" | ">>" if i > 0 && words[i - 1] == "cat" => words.get(i + 1),
                "tee" | "-a" if i > 0 && (words[i - 1] == "|" || words[i - 1] == "tee") => words.get(i + 1),
                _ => None,
            };
            if let Some(file) = target.filter(|f| !f.starts_with('<') && !f.starts_with('-')) {
                let file = file.trim_matches(['\'', '"']).to_owned();
                if !files.contains(&file) {
                    files.push(file);
                }
            }
        }
    }
    files
}

fn render_approval(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    // A blank line above sets it apart from the conversation.
    let [_, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    frame.render_widget(Paragraph::new(lines), body);
}

fn render_composer(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let text_width = area.width.saturating_sub(2) as usize;
    let (rows, (cursor_row, cursor_col)) = app.composer.layout(text_width);
    // Keep the cursor's row in view when the message is taller than the field.
    let first = (cursor_row + 1).saturating_sub(area.height as usize);
    let content = if let (true, Some(hint)) = (app.composer.text().is_empty(), app.hint()) {
        // What the moment calls for, greyed: → takes it.
        Paragraph::new(Line::from(vec![
            Span::styled("› ", theme.accent()),
            Span::styled(hint, theme.dim().add_modifier(Modifier::ITALIC)),
            Span::styled(format!("   {}", t!("view.to_fill_in")), theme.dim()),
        ]))
    } else if app.composer.text().is_empty() {
        let to = app.addressee().map(|(name, _)| name).unwrap_or_else(|| t!("view.the_squad").into());
        // A ticket open: what is written steers its work, or goes to the PM about it.
        let say = match (app.opened, app.viewed()) {
            (Some(id), Some(agent)) if agent.ticket == Some(id) => t!("view.compose_steer", name = agent.label(), id = id),
            (Some(id), _) => t!("view.compose_pm_about", id = id),
            _ => t!("view.compose_to", name = to),
        };
        Paragraph::new(Line::from(vec![Span::styled("› ", theme.accent()), Span::styled(say, theme.dim())]))
    } else {
        let lines: Vec<Line> = rows
            .iter()
            .enumerate()
            .skip(first)
            .map(|(i, row)| {
                let lead = if i == 0 { Span::styled("› ", theme.accent()) } else { Span::raw("  ") };
                Line::from(vec![lead, Span::raw(row.clone())])
            })
            .collect();
        Paragraph::new(lines)
    };
    frame.render_widget(content, area);
    frame.set_cursor_position((area.x + 2 + cursor_col as u16, area.y + cursor_row.saturating_sub(first) as u16));
}

/// The bottom line: who a message goes to (always — nothing reaches the wrong
/// person), or a passing message; then what waits for the person, and the keys.
fn render_footer(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let left: Vec<Span> = match (app.confirm_prompt().or_else(|| app.flash.clone()), app.addressee()) {
        (Some(flash), _) => vec![Span::styled(flash, theme.bold())],
        (None, Some((name, direct))) => {
            let style = member_style(app, &name, theme).add_modifier(Modifier::BOLD);
            let mut spans = vec![Span::styled("→ ", theme.dim()), Span::styled(name.clone(), style)];
            // Where new work goes, with a project filter on; without, the squad.
            if let Some(project) = app.focus.as_ref() {
                spans.push(Span::styled(" · ", theme.dim()));
                spans.push(Span::styled(project_name(project), project_style(app, Some(project), theme).add_modifier(Modifier::BOLD)));
            }
            // At work on another project's ticket: this conversation runs beside it.
            if let Some(agent) = app.agents.iter().find(|a| a.label() == name)
                && let (Some(ticket), Some(focus)) = (agent.ticket, app.focus.as_ref())
                && let Some(project) = app.agent_project(agent).filter(|p| p != focus)
            {
                spans.push(Span::styled(format!(" · {}", t!("view.at_work_beside", project = project_name(&project), id = ticket)), theme.dim()));
            }
            if direct {
                spans.push(Span::styled(format!(" · {}", t!("view.pm_hears")), theme.dim()));
            }
            if let Some(agent) = app.selected().filter(|a| a.member.is_some()) {
                spans.push(Span::styled(format!(" · {}", label::mode(crate::i18n::lang(), app.mode_of(agent))), theme.dim()));
            }
            spans
        }
        _ => Vec::new(),
    };
    let mut facts: Vec<(String, Style)> = Vec::new();
    // Work waiting for the person comes first: it's what only they can do.
    // One count, the same as the side list's and the desktop's: what waits for the person.
    let needs = app.needs_you_count();
    if needs > 0 {
        facts.push((format!("{} {needs} · ctrl-b", t!("app.menu_inbox")), theme.accent()));
    }
    // A multi-line draft (often a paste) says how big it is before it's sent.
    let draft_lines = app.composer.line_count();
    if draft_lines > 1 {
        facts.push((t!("view.draft_lines", n = draft_lines), theme.dim()));
    }
    // What counts on a subscription: how much of the plan's windows is used.
    // Tokens and context are on /usage.
    if let Some(plan) = app.plan {
        for (label, window) in [(t!("view.window_5h"), plan.five_hour), (t!("view.window_week"), plan.seven_day)] {
            if let Some(w) = window {
                let style = if w.used_percent >= 80 { theme.accent().add_modifier(Modifier::BOLD) } else { theme.dim() };
                facts.push((format!("{label} {}%", w.used_percent), style));
            }
        }
    }
    let left_width: usize = left.iter().map(|s| s.width()).sum();
    // Facts first; the key hints — key in bold, what it does dim — go when space runs short.
    let mut right: Vec<Vec<Span<'static>>> = facts.into_iter().map(|(text, style)| vec![Span::styled(text, style)]).collect();
    let keys: Vec<Vec<Span<'static>>> = [("enter", t!("view.key_send")), ("tab", t!("view.key_next")), ("/help", "")]
        .iter()
        .map(|(key, what)| {
            let mut spans = vec![Span::styled(*key, theme.bold())];
            if !what.is_empty() {
                spans.push(Span::styled(format!(" {what}"), theme.dim()));
            }
            spans
        })
        .collect();
    let width = |items: &[Vec<Span<'static>>]| {
        items.iter().map(|spans| spans.iter().map(Span::width).sum::<usize>()).sum::<usize>() + 3 * items.len().saturating_sub(1)
    };
    let with_keys: Vec<Vec<Span<'static>>> = right.iter().cloned().chain(keys).collect();
    if left_width + width(&with_keys) + 3 <= area.width as usize {
        right = with_keys;
    }
    let gap = (area.width as usize).saturating_sub(left_width + width(&right));
    let mut spans = left;
    if gap >= 3 && !right.is_empty() {
        spans.push(Span::raw(" ".repeat(gap)));
        for (i, item) in right.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", theme.dim()));
            }
            spans.extend(item);
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// A count as it reads at a glance: `1,234`, `812k`, `24.3M`.
fn short_count(n: u64) -> String {
    match n {
        0..10_000 => group_digits(n),
        10_000..1_000_000 => format!("{}k", n / 1000),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `~/Abel/equipe` rather than `/Users/abel/Abel/equipe`.
fn display_path(path: &std::path::Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(|p| p.to_owned())) {
        Some(rel) if rel.as_os_str().is_empty() => "~".into(),
        Some(rel) => format!("~/{}", rel.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::App;
    use equipe_core::protocol::{Command, Engine, Event, Member, Snapshot, Ticket, TicketId, TurnOutcome, Usage, Workspace};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render(f, app, Theme::plain(), &Layouts::default())).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                // A two-column character owns the next cell too; read it once.
                let mut row = String::new();
                let mut x = 0;
                while x < width {
                    let symbol = buffer[(x, y)].symbol();
                    row.push_str(symbol);
                    x += unicode_width::UnicodeWidthStr::width(symbol).max(1) as u16;
                }
                row
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn empty() -> Snapshot {
        Snapshot { squad: "acme".into(), projects: vec!["/repo".into()], ..Snapshot::default() }
    }

    /// The PM's id, and an app where it is mid-turn.
    fn working() -> (App, u64) {
        let (mut app, commands) = App::from_snapshot("/repo".into(), empty(), &[]);
        let Some(Command::Start { session: id, .. }) = commands.first() else { panic!() };
        let id = *id;
        app.on_event(Event::SessionStarted {
            session: id,
            project: "/repo".into(),
            engine: Engine::Claude,
            model: Some("haiku".into()),
            workspace: None,
            member: Some(Member { role: "pm".into(), name: "pm".into() }),
            squad: Some("acme".into()),
        });
        app.on_event(Event::UserMessage { session: id, text: "fix the bug".into() });
        app.on_event(Event::TextDelta { session: id, text: "Looking **now**.".into() });
        app.on_event(Event::ToolUse { session: id, tool: "Edit".into(), target: Some("src/auth.rs".into()) });
        (app, id)
    }

    fn ticket(id: TicketId, status: TicketStatus, title: &str) -> Ticket {
        Ticket {
            id,
            title: title.into(),
            body: String::new(),
            status,
            owner: Some("frontend".into()),
            assignee: None,
            project: Some("/repo".into()),
            depends_on: vec![],
            branch: None,
            worktree: None,
            merged: None,
            off_main: None,
            kind: equipe_core::protocol::TicketKind::Code,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
        }
    }

    fn approval(action: &str) -> equipe_core::protocol::Approval {
        equipe_core::protocol::Approval {
            id: 1,
            session: 2,
            member: "frontend".into(),
            ticket: Some(3),
            tool: "Bash".into(),
            action: action.into(),
            reason: None,
            preview: None,
        }
    }

    #[test]
    fn shows_the_squad_the_conversation_and_what_the_member_does() {
        let shot = screen(&working().0, 100, 22);
        for expected in ["equipe.", "acme", "squads/acme/home", "repo · all", "SQUAD", "pm    haiku", " │", "pm  ·  claude code", "› fix the bug", "Looking now.", "✎ edit   src/auth.rs", "editing · 0s", "esc to stop", "Board  ctrl-b", "→ pm · auto", "enter send"] {
            assert!(shot.contains(expected), "missing {expected:?} in:\n{shot}");
        }
        assert!(!shot.contains("**"), "Markdown is drawn, not shown:\n{shot}");
        assert!(!shot.contains('┌') && !shot.contains('└'), "no boxes:\n{shot}");
    }

    #[test]
    fn nothing_touches_the_screens_edge() {
        let shot = screen(&working().0, 100, 22);
        for line in shot.lines() {
            assert!(line.starts_with("  ") || line.trim().is_empty(), "a line on the edge: {line:?}");
        }
    }

    #[test]
    fn a_finished_turn_shows_how_long_it_took_a_ready_member_and_its_usage() {
        let (mut app, id) = working();
        app.agents[0].working_since = Some(Instant::now() - std::time::Duration::from_secs(65));
        app.on_event(Event::TurnEnded {
            session: id,
            outcome: TurnOutcome::Completed,
            usage: Some(Usage {
                input_tokens: Some(1200),
                output_tokens: Some(34),
                cache_tokens: Some(98_766),
                context_tokens: Some(130_000),
                context_window: Some(200_000),
            }),
        });
        let plan = equipe_core::protocol::PlanWindow { used_percent: 84, resets_at: 0 };
        app.on_event(Event::PlanUsage { usage: equipe_core::protocol::PlanUsage { five_hour: Some(plan), seven_day: None } });
        let shot = screen(&app, 130, 22);
        // On a subscription what counts is the plan: in the footer, not a token total.
        assert!(shot.lines().last().unwrap().contains("5h 84%"), "{shot}");
        assert!(!shot.contains("tokens"), "{shot}");
        // A conversation past half its context shows it in the squad list.
        assert!(shot.contains("pm    haiku · 65%"), "{shot}");
        // The totals are on /usage.
        app.usage_open = true;
        let usage = screen(&app, 130, 22);
        assert!(usage.contains("context 65% (130k of 200k) · 100k tokens · 98%"), "{usage}");
        app.usage_open = false;
        assert!(shot.contains("◆ pm"), "at rest, its own mark:\n{shot}");
        assert!(shot.contains("─ worked 1m 05s"), "{shot}");
        assert!(!shot.contains("● working"), "{shot}");
    }

    #[test]
    fn a_narrow_terminal_drops_the_squad_pane_and_says_who_is_there_in_the_header() {
        let shot = screen(&working().0, 56, 18);
        assert!(!shot.contains("SQUAD"), "{shot}");
        assert!(shot.contains("1 members · 1 working"), "{shot}");
        assert!(shot.contains("Looking now."), "{shot}");
    }

    #[test]
    fn the_header_says_who_has_the_macs_screen_and_who_waits() {
        use equipe_core::protocol::{DesktopState, MemberRef};
        let (mut app, _) = working();
        assert!(!screen(&app, 120, 18).contains("▣"), "nothing while nobody uses it");
        let here = |m: &str| MemberRef { squad: app.squad.clone(), member: m.into() };
        let desktop = DesktopState { holder: Some(here("frontend")), queue: vec![here("backend")], stopped: false };
        app.on_event(Event::Desktop { desktop });
        let shot = screen(&app, 120, 18);
        assert!(shot.contains("▣ frontend on the screen · 1 waiting"), "{shot}");
        app.on_event(Event::Desktop { desktop: DesktopState { stopped: true, ..Default::default() } });
        assert!(screen(&app, 120, 18).contains("▣ screen taken back · /desktop resume"));
    }

    #[test]
    fn a_failed_start_shows_the_fix() {
        let (mut app, commands) = App::from_snapshot("/repo".into(), empty(), &[]);
        let Some(Command::Start { session: id, .. }) = commands.first() else { panic!() };
        app.on_event(Event::Error { session: Some(*id), message: "No engine is ready.".into(), next: Some("equipe login".into()) });
        let shot = screen(&app, 100, 18);
        assert!(shot.contains("✗ No engine is ready."), "{shot}");
        assert!(shot.contains("→ equipe login"), "{shot}");
        assert!(shot.contains("✗ pm"), "{shot}");
    }

    #[test]
    fn hangul_replies_wrap_inside_the_pane() {
        let (mut app, id) = working();
        app.on_event(Event::TextDelta { session: id, text: "\n\n에이전트가 파일을 고쳤어요 이제 리뷰해 주세요 브랜치에 커밋했어요".into() });
        let shot = screen(&app, 72, 22);
        assert!(shot.contains("에이전트가"), "{shot}");
    }

    #[test]
    fn what_the_person_attached_shows_as_its_kind_and_name_not_its_path() {
        let (mut app, id) = working();
        app.on_event(Event::TurnEnded { session: id, outcome: TurnOutcome::Completed, usage: None });
        let said = "pair 가 잘 안되는데 [image: /Users/abel/.equipe/uploads/1792073093606-paste.png] [pdf: /Users/abel/.equipe/uploads/1792073093607-spec.pdf · 2 pages]";
        app.on_event(Event::UserMessage { session: id, text: said.into() });
        let shot = screen(&app, 100, 24);
        assert!(shot.contains("pair 가 잘 안되는데 [Image] paste.png [PDF]") && shot.contains("spec.pdf · 2 pages"), "{shot}");
        assert!(!shot.contains("uploads"), "{shot}");
        assert_eq!(app.agents[0].last_said().as_deref(), Some("pair 가 잘 안되는데 [Image] paste.png [PDF] spec.pdf · 2 pages"));
        assert_eq!(attachments_named("[video: /u/1792073093606-a.mov · 0:20 · frames: /u/f.jpg@0:05]"), "[Video] a.mov · 0:20");
        // What reaches the member is the tag as it was typed.
        assert!(app.agents[0].transcript.entries().iter().any(|e| matches!(e, Entry::User { text } if text == said)));
    }

    #[test]
    fn a_long_korean_reply_wraps_inside_the_pane_again() {
        let (mut app, id) = working();
        app.on_event(Event::TextDelta { session: id, text: "\n\n에이전트가 파일을 고쳤어요 이제 리뷰해 주세요 브랜치에 커밋했어요".into() });
        let shot = screen(&app, 72, 22);
        for line in shot.lines() {
            assert!(unicode_width::UnicodeWidthStr::width(line) <= 72, "overflow: {line:?}");
        }
        assert!(shot.contains("에이전트가"), "{shot}");
    }

    #[test]
    fn prose_stops_at_a_readable_width_on_a_wide_terminal() {
        let (mut app, id) = working();
        app.on_event(Event::TextDelta { session: id, text: format!("\n\n{}", "word ".repeat(60)) });
        let shot = screen(&app, 220, 22);
        let longest = shot.lines().filter(|l| l.contains("word")).map(|l| l.trim_end().len()).max().unwrap();
        // The pane starts after the margin, the side list and the hairline: 2 + 38 + 4.
        // and the reply's mark ahead of its text.
        assert!(longest <= 2 + SIDE_FIT as usize + 4 + 4 + PROSE_MAX, "{longest}");
    }

    #[test]
    fn a_multi_line_paste_shows_its_size_before_sending() {
        let (mut app, id) = working();
        app.on_event(Event::TurnEnded { session: id, outcome: TurnOutcome::Completed, usage: None });
        app.on_paste("line one\nline two\nline three");
        let shot = screen(&app, 100, 22);
        assert!(shot.contains("3 lines"), "{shot}");
        assert!(shot.contains("line three"), "{shot}");
    }

    #[test]
    fn a_narrow_footer_drops_the_hints_and_keeps_the_facts() {
        let (mut app, id) = working();
        app.on_event(Event::TurnEnded {
            session: id,
            outcome: TurnOutcome::Completed,
            usage: Some(Usage { input_tokens: Some(100), output_tokens: Some(20), ..Default::default() }),
        });
        app.on_paste("one\ntwo\nthree");
        let shot = screen(&app, 44, 22);
        let footer = shot.lines().last().unwrap();
        assert!(footer.contains("3 lines"), "{footer:?}");
        assert!(!footer.contains("enter send"), "{footer:?}");
    }

    #[test]
    fn a_memo_and_the_turn_show_in_the_projects_conversation_they_belong_to() {
        let (mut app, pm) = working();
        app.projects = vec!["/code/site".into(), "/code/app".into()];
        // The turn runs in site's conversation; a memo was left about app.
        app.on_event(Event::Thread { session: pm, project: Some("site".into()) });
        app.on_event(Event::MessageQueued { session: pm, text: "and the app icon".into(), project: Some("app".into()) });
        app.agents[0].thread_view = Some("app".into());
        let shot = screen(&app, 110, 24);
        assert!(shot.contains("› and the app icon"), "{shot}");
        assert!(shot.contains("memo · read when this turn ends"), "{shot}");
        assert!(shot.contains("in site ·"), "{shot}");
        // Looking at site: the turn is here, the memo is app's.
        app.agents[0].thread_view = Some("site".into());
        let shot = screen(&app, 110, 24);
        assert!(!shot.contains("and the app icon"), "another project's memo isn't shown here: {shot}");
        assert!(!shot.contains("memo waiting"), "{shot}");
        assert!(!shot.contains("in site ·"), "{shot}");
        assert!(shot.contains("esc to stop"), "{shot}");
        // A memo left here shows here, in full, whatever was left elsewhere first.
        app.on_event(Event::MessageQueued { session: pm, text: "and the site footer".into(), project: Some("site".into()) });
        let shot = screen(&app, 110, 24);
        assert!(shot.contains("› and the site footer"), "{shot}");
        assert!(shot.contains("memo · read when this turn ends"), "{shot}");
        assert!(!shot.contains("and the app icon"), "{shot}");
        // The turn ends: app's memo goes, into app's conversation; site's waits for the next.
        app.on_event(Event::UserMessage { session: pm, text: "and the app icon".into() });
        assert_eq!(app.agents[0].queued, ["and the site footer"]);
        assert_eq!(app.agents[0].queued_in, [Some("site".to_owned())]);
    }

    #[test]
    fn the_pm_on_a_ticket_of_its_own_answers_beside_it_in_its_one_conversation() {
        let (mut app, pm) = working();
        app.agents[0].ticket = Some(1);
        for c in "how is the app?".chars() {
            app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char(c)), Instant::now());
        }
        let sent = app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Enter), Instant::now());
        assert_eq!(sent, vec![Command::Submit { session: pm, text: "how is the app?".into() }]);
        assert!(app.flash.as_deref().unwrap().contains("answers now, beside its work on T-1"), "{:?}", app.flash);
        // The daemon runs the conversation beside the ticket: not listed, it is what shows.
        app.on_event(Event::SessionStarted {
            session: 77,
            project: "/repo".into(),
            engine: Engine::Claude,
            model: None,
            workspace: None,
            member: Some(Member { role: "pm".into(), name: "pm".into() }),
            squad: Some("acme".into()),
        });
        app.on_event(Event::LaneOpened { session: 77, of: pm, project: None });
        assert_eq!(app.agents.len(), 1);
        app.on_event(Event::UserMessage { session: 77, text: "how is the app?".into() });
        app.on_event(Event::TextDelta { session: 77, text: "The app is fine.".into() });
        assert_eq!(app.viewed().map(|a| a.id), Some(77));
        let shot = screen(&app, 110, 24);
        assert!(shot.contains("The app is fine.") && shot.contains("2 at once"), "{shot}");
        // Scrolling moves the conversation shown.
        for i in 0..60 {
            app.on_event(Event::Notice { session: 77, text: format!("line {i}") });
        }
        let laid = extent(&app, Theme::plain(), Rect::new(0, 0, 110, 24), &Layouts::default());
        app.set_extent(laid.page, laid.conversation_max, laid.board_max);
        app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::PageUp), Instant::now());
        assert!(app.lanes[0].scroll_back > 0 && app.agents[0].scroll_back == 0);
        assert!(!screen(&app, 110, 24).contains("line 59"));
        app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::End), Instant::now());
        assert!(screen(&app, 110, 24).contains("line 59"));
        // Esc stops the conversation, not the ticket's work.
        let stop = app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Esc), Instant::now());
        assert_eq!(stop, vec![Command::Interrupt { session: 77 }]);
        // Its ticket opened: the work, not the conversation.
        app.open(Side::Ticket(1));
        assert_eq!(app.viewed().map(|a| a.id), Some(pm));
        // Left idle, the conversation goes back into the PM: what was said stays.
        app.open(Side::Pm);
        app.on_event(Event::SessionClosed { session: 77 });
        assert!(app.lanes.is_empty() && app.agents.len() == 1);
        assert_eq!(app.viewed().map(|a| a.id), Some(pm));
        assert!(screen(&app, 110, 24).contains("line 59"));
    }

    #[test]
    fn without_the_side_list_the_pm_oversees_every_project_a_line_each_a_click_into_it() {
        let (mut app, _) = working();
        app.projects = vec!["/code/site".into(), "/code/app".into()];
        app.tickets = vec![
            Ticket { project: Some("/code/app".into()), ..ticket(5, TicketStatus::Review, "Login page") },
            Ticket { project: Some("/code/site".into()), ..ticket(6, TicketStatus::InProgress, "Pricing") },
        ];
        app.agents[0].ticket = Some(6);
        // A narrow terminal: no side list, so where each project stands shows here.
        let shot = screen(&app, 71, 34);
        assert!(shot.contains("All projects"), "{shot}");
        let site = shot.lines().find(|l| l.contains("site ") && l.contains("pm T-6")).unwrap_or_else(|| panic!("{shot}"));
        assert!(site.contains("1 in progress"), "{site}");
        assert!(shot.lines().any(|l| l.contains("app ") && l.contains("⚑1 inbox")), "{shot}");
        let laid = extent(&app, Theme::plain(), Rect::new(0, 0, 71, 34), &Layouts::default());
        assert!(laid.hits.iter().any(|h| h.3 == Hit::Project(Some(1))), "a line goes into its project");
        // With the side list, it is there instead.
        assert!(!screen(&app, 120, 34).contains("All projects"));
    }

    #[test]
    fn work_under_way_says_when_it_is_expected_done_and_what_it_ships_when_live() {
        let (mut app, pm) = working();
        let now = now_unix();
        app.on_event(Event::Estimated {
            session: pm,
            estimate: Some(Estimate {
                done_at: Some(now + 12 * 60),
                what: Some("release notes".into()),
                live_at: Some(now + 25 * 60),
                live_what: Some("v0.28.1 on Homebrew".into()),
                guessed: false,
            }),
        });
        let shot = screen(&app, 160, 24);
        let row = shot.lines().find(|l| l.contains("esc to stop")).unwrap_or_else(|| panic!("{shot}"));
        assert!(row.contains(&format!("done ~{} (12m)", clock(now + 12 * 60))), "{row}");
        assert!(row.contains("v0.28.1 on Homebrew live ~") && row.contains("(25m)"), "{row}");
        let squad = shot.lines().find(|l| l.contains("› ")).unwrap();
        assert!(squad.contains("~12m"), "{squad}");
        // Its turn over, what it shipped is still on its way.
        app.on_event(Event::TurnEnded { session: pm, outcome: equipe_core::protocol::TurnOutcome::Completed, usage: None });
        app.on_event(Event::Estimated {
            session: pm,
            estimate: Some(Estimate { live_at: Some(now + 25 * 60), live_what: Some("v0.28.1 on Homebrew".into()), ..Default::default() }),
        });
        let shot = screen(&app, 160, 24);
        assert!(shot.contains("◷ v0.28.1 on Homebrew live ~"), "{shot}");
        assert!(shot.lines().find(|l| l.contains("› ")).unwrap().contains("↑25m"), "{shot}");
        // Late: said so, not hidden.
        assert_eq!(done_phrase(now - 60, false, now), format!("expected ~{}, still going", clock(now - 60)));
        // equipe's own guess says so, and shows only once a turn has gone on a while.
        assert_eq!(done_phrase(now + 600, true, now), format!("usually done ~{} (10m)", clock(now + 600)));
        assert_eq!(done_phrase(now - 60, true, now), "longer than usual");
        let guess = Estimate { done_at: Some(now + 600), guessed: true, ..Default::default() };
        app.agents[0].estimate = Some(guess.clone());
        app.agents[0].working_since = Some(Instant::now());
        assert!(app.agents[0].shown_estimate().is_none());
        app.agents[0].working_since = Instant::now().checked_sub(std::time::Duration::from_secs(90));
        assert_eq!(app.agents[0].shown_estimate(), Some(&guess));
        assert_eq!(estimate_short(&guess, now).as_deref(), Some("≈10m"));
        assert_eq!(time_left(now + 80 * 60, now), "1h 20m");
    }

    #[test]
    fn said_to_a_member_at_work_it_steers_and_tab_leaves_a_memo() {
        let (mut app, pm) = working();
        for c in "skip the footer".chars() {
            app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char(c)), Instant::now());
        }
        let sent = app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Enter), Instant::now());
        assert_eq!(sent, vec![Command::Submit { session: pm, text: "skip the footer".into() }], "Enter steers");
        app.on_event(Event::SteerQueued { session: pm, text: "skip the footer".into() });
        let shot = screen(&app, 110, 24);
        assert!(shot.contains("› skip the footer") && shot.contains("steering · read at its next step"), "{shot}");
        assert!(shot.contains("1 unread"), "{shot}");
        // Read: it moves into the conversation, where it was read.
        app.on_event(Event::Steered { session: pm, text: "skip the footer".into() });
        let shot = screen(&app, 110, 24);
        assert!(!shot.contains("steering ·"), "{shot}");
        assert!(app.agents[0].transcript.entries().iter().any(|e| matches!(e, Entry::User { text } if text == "skip the footer")));
        // Tab: for after this turn.
        for c in "then the pricing page".chars() {
            app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char(c)), Instant::now());
        }
        let later = app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Tab), Instant::now());
        assert_eq!(later, vec![Command::SubmitLater { session: pm, text: "then the pricing page".into() }]);
    }

    #[test]
    fn memos_left_while_on_a_ticket_stack_up_in_sight() {
        let (mut app, pm) = working();
        app.projects = vec!["/code/site".into(), "/code/app".into()];
        app.on_event(Event::WorkStarted { session: pm, ticket: 7, title: "Settle up".into(), project: "/h/home".into(), workspace: None });
        for text in ["what was the 200 for?", "is it all done?"] {
            app.on_event(Event::MessageQueued { session: pm, text: text.into(), project: Some("home".into()) });
        }
        let shot = screen(&app, 110, 30);
        assert!(shot.contains("› what was the 200 for?") && shot.contains("› is it all done?"), "{shot}");
    }

    #[test]
    fn a_member_spoken_to_directly_says_the_pm_hears_and_memos_wait_in_sight() {
        let (mut app, pm) = working();
        app.on_event(Event::SessionStarted {
            session: 2,
            project: "/repo".into(),
            engine: Engine::Claude,
            model: Some("claude-sonnet-5-5".into()),
            workspace: None,
            member: Some(Member { role: "frontend".into(), name: "frontend".into() }),
            squad: Some("acme".into()),
        });
        app.on_event(Event::MessageQueued { session: pm, text: "and the footer".into(), project: None });
        let shot = screen(&app, 100, 24);
        assert!(shot.contains("● frontend  sonnet-5-5"), "{shot}");
        assert!(shot.contains("pm        1 memo"), "{shot}");
        assert!(shot.contains("› and the footer"), "{shot}");
        assert!(shot.contains("memo · read when this turn ends"), "{shot}");

        app.on_event(Event::WorkStarted {
            session: 2,
            ticket: 3,
            title: "Hero".into(),
            project: "/repo".into(),
            workspace: Some(Workspace { branch: "equipe/frontend-t3".into(), base: "abc".into(), path: "/w".into() }),
        });
        app.selected = 1;
        let shot = screen(&app, 100, 24);
        assert!(shot.contains("→ frontend · the PM hears about it"), "{shot}");
        assert!(shot.contains("frontend  T-3"), "{shot}");
        assert!(shot.contains("── T-3 · Hero — on branch equipe/frontend-t3"), "{shot}");
    }

    #[test]
    fn looking_around_reads_as_one_line() {
        let (mut app, id) = working();
        for file in ["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"] {
            app.on_event(Event::ToolUse { session: id, tool: "Read".into(), target: Some(format!("src/{file}")) });
        }
        app.on_event(Event::ToolUse { session: id, tool: "Edit".into(), target: Some("src/a.rs".into()) });
        let shot = screen(&app, 100, 24);
        assert!(shot.contains("· read   src/a.rs, src/b.rs, src/c.rs, +2"), "{shot}");
        assert_eq!(shot.matches("✎ edit").count(), 2, "edits stay one per line:\n{shot}");
    }

    #[test]
    fn scrolled_up_the_new_lines_are_counted_at_the_bottom() {
        let (mut app, id) = working();
        app.agents[0].scroll_back = 3;
        app.grew(id, 7);
        let shot = screen(&app, 100, 22);
        assert!(shot.contains("↓ 7 new lines · End"), "{shot}");
    }

    #[test]
    fn the_squad_pane_fits_its_names_and_its_edge_drags() {
        let (mut app, _) = working();
        let edge = |shot: &str| shot.lines().find(|l| l.contains("SQUAD")).and_then(|l| l.chars().position(|c| c == '│')).unwrap();
        let shot = screen(&app, 120, 24);
        let fitted = edge(&shot);
        assert_eq!(fitted, 2 + SIDE_FIT as usize + 1, "as wide as a ticket's title needs:\n{shot}");

        // The person takes the edge and drags it to column 50.
        let laid = extent(&app, Theme::plain(), Rect::new(0, 0, 120, 24), &Layouts::default());
        let divider = laid.divider.unwrap();
        assert_eq!(divider.x as usize, fitted);
        app.set_hits(laid.hits, laid.divider);
        app.on_click(divider.x, divider.top + 3);
        assert!(app.is_dragging());
        app.on_drag(50, divider.top + 3);
        app.on_mouse(crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left), Instant::now());
        assert!(!app.is_dragging());
        assert_eq!(edge(&screen(&app, 120, 24)), 50);
        // Never past half the screen; and a drag without hold does nothing.
        app.on_drag(110, divider.top + 3);
        assert_eq!(edge(&screen(&app, 120, 24)), 50);
        app.on_click(divider.x, divider.top + 3);
        app.on_drag(110, divider.top + 3);
        assert_eq!(edge(&screen(&app, 120, 24)), 2 + 58 + 1, "half of the 116-column body");
    }

    #[test]
    fn the_board_puts_the_persons_reviews_first() {
        let (mut app, _) = working();
        for t in [
            ticket(1, TicketStatus::Done, "Design direction"),
            ticket(2, TicketStatus::InProgress, "Landing hero"),
            ticket(3, TicketStatus::Review, "Pricing section"),
            ticket(4, TicketStatus::Todo, "Footer"),
        ] {
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        }
        app.pane = Pane::Board;
        app.flash = None;
        let shot = screen(&app, 140, 34);
        let at = |text: &str| shot.find(text).unwrap_or_else(|| panic!("missing {text:?} in:\n{shot}"));
        assert!(at("Review  1") < at("In progress  1"), "{shot}");
        assert!(at("In progress") < at("To do") && at("To do") < at("Done  1 >"), "the finished folded last:\n{shot}");
        // One row a ticket — its mark, number, project, title — the review picked first; its columns line up.
        let row = |id: &str| shot.lines().find(|l| l.contains(id)).unwrap_or_else(|| panic!("no {id} in:\n{shot}")).to_owned();
        assert!(row("T-3").contains("› ? T-3    repo  Pricing section"), "{shot}");
        assert!(row("T-2").contains("* T-2") && row("T-4").contains("o T-4"), "{shot}");
        let column = |id: &str| { let r = row(id); r[..r.find("frontend").unwrap()].chars().count() };
        assert!(["T-2", "T-4"].iter().all(|id| column(id) == column("T-3")), "owners in one column:\n{shot}");
        // Below, the picked ticket in full with the keys that act on it.
        assert!(shot.contains("code · frontend · Review"), "{shot}");
        assert!(shot.contains("a accept   r ask for changes   n cancel"), "{shot}");
        // Right under the title, however long the hand-in below runs.
        let at = |text: &str| shot.lines().position(|l| l.contains(text)).unwrap();
        assert_eq!(at("a accept"), at("code · frontend · Review") + 1, "{shot}");
        assert!(shot.contains("← pm  ctrl-b"), "the way back is in sight:\n{shot}");
        assert!(shot.lines().last().unwrap().contains("Inbox 1 · ctrl-b"), "{shot}");
        // ↓ picks the next ticket, and the detail follows.
        app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Down), Instant::now());
        let shot = screen(&app, 140, 34);
        assert!(shot.contains("› * T-2") && shot.contains("code · frontend · In progress"), "{shot}");
    }

    #[test]
    fn work_that_isnt_code_wears_its_kind_and_no_git_words() {
        let (mut app, _) = working();
        let mut task = ticket(1, TicketStatus::Review, "Competitor research");
        task.kind = equipe_core::protocol::TicketKind::Task;
        task.project = None;
        task.branch = Some("equipe/frontend-t1".into());
        task.note = Some("Three found.\n\nIn the squad's folder:\n- outputs/T-1 research/report.md".into());
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: task });
        app.pane = Pane::Board;
        app.flash = None;
        let shot = screen(&app, 140, 30);
        assert!(shot.lines().any(|l| l.contains("T-1") && l.contains("home  Competitor research") && l.contains("task")), "the squad home's work is tagged home, its kind where a diff would be:\n{shot}");
        assert!(shot.contains("task · frontend · Review"), "{shot}");
        assert!(shot.contains("outputs/T-1 research/report.md"), "{shot}");
        assert!(!shot.contains("equipe/frontend-t1"), "no branch for the squad's folder:\n{shot}");
    }

    #[test]
    fn accepted_work_not_on_main_says_so() {
        let (mut app, _) = working();
        let mut done = ticket(1, TicketStatus::Done, "Design direction");
        done.branch = Some("equipe/designer-t1".into());
        done.merged = Some(false);
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: done.clone() });
        app.pane = Pane::Board;
        app.board_menu = BoardMenu::Inbox;
        app.flash = None;
        let shot = screen(&app, 100, 24);
        assert!(shot.contains("Accepted, not in main yet  1"), "its own section in the inbox:\n{shot}");
        assert!(shot.contains("The work you accepted is only on the member's branch."), "what it means, under it:\n{shot}");
        assert!(shot.lines().any(|l| l.contains("T-1") && l.contains("Not in main")), "{shot}");
        assert!(shot.contains("code · frontend · Accepted · not in main yet") && shot.contains("m merge into main"), "{shot}");
        // Kept on its branch by the person: out of the inbox, said so on the board.
        let kept = Ticket { off_main: Some(equipe_core::protocol::OffMain::Kept), ..done.clone() };
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: kept });
        assert!(!screen(&app, 100, 24).contains("Accepted, not in main yet"));
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: Ticket { merged: Some(true), ..done } });
        assert!(!screen(&app, 100, 24).contains("not in main"));
    }

    #[test]
    fn an_empty_board_says_how_work_gets_there() {
        let (mut app, _) = working();
        app.pane = Pane::Board;
        assert!(screen(&app, 100, 18).contains("The board is empty."));
    }

    #[test]
    fn a_request_for_permission_sits_behind_one_accent_bar_with_its_keys() {
        let (mut app, _) = working();
        let mut asking = approval("npm publish");
        asking.reason = Some("Release the package".into());
        app.on_event(Event::ApprovalRequested { squad: "acme".into(), approval: asking });
        let shot = screen(&app, 100, 26);
        for expected in ["▎ frontend asks · T-3", "▎ run  npm publish", "▎ Release the package", "▎ y/a allow   A always allow npm publish   n decline", "▎ /mode frontend full stops frontend asking"] {
            assert!(shot.contains(expected), "missing {expected:?} in:\n{shot}");
        }
    }

    #[test]
    fn a_send_shows_the_draft_and_the_screen_and_is_only_ever_once() {
        let (mut app, _) = working();
        let mut asking = approval("press return in Microsoft Teams · Chat | Kim Minji\n내일 회의 3시로 옮겨요");
        asking.tool = equipe_core::desktop_gate::SEND.into();
        asking.reason = Some("Reply to Kim Minji about the meeting".into());
        asking.preview = Some("/tmp/equipe-send-1-1.png".into());
        app.on_event(Event::ApprovalRequested { squad: "acme".into(), approval: asking });
        let shot = screen(&app, 100, 26);
        for expected in ["▎ send  press return in Microsoft Teams", "내일 회의 3시로 옮겨요", "▎ screen  /tmp/equipe-send-1-1.png  o open", "▎ y/a allow   n decline"] {
            assert!(shot.contains(expected), "missing {expected:?} in:\n{shot}");
        }
        assert!(!shot.contains("always") && !shot.contains("/mode"), "no mode stops a send:\n{shot}");
    }

    #[test]
    fn a_long_script_asks_in_a_few_lines_and_says_what_it_writes() {
        let (mut app, _) = working();
        let script = "mkdir -p src/pages\ncat > astro.config.mjs <<'EOF'\nexport default {}\nEOF\ncat > tsconfig.json <<'EOF'\n{}\nEOF\necho done";
        app.on_event(Event::ApprovalRequested { squad: "acme".into(), approval: approval(script) });
        let shot = screen(&app, 100, 30);
        assert!(shot.contains("writes astro.config.mjs, tsconfig.json"), "{shot}");
        assert!(shot.contains("… 5 more lines"), "{shot}");
        assert!(shot.contains("y/a allow"), "the keys always show:\n{shot}");
        assert!(!shot.contains("echo done"), "{shot}");
    }

    #[test]
    fn engine_housekeeping_stays_out_and_memory_reads_as_memory() {
        let (mut app, id) = working();
        app.on_event(Event::ToolUse { session: id, tool: "ToolSearch".into(), target: None });
        app.on_event(Event::ToolUse {
            session: id,
            tool: "Write".into(),
            target: Some("/Users/a/.equipe/squads/acme/members/pm/memory/site.md".into()),
        });
        let shot = screen(&app, 100, 22);
        assert!(!shot.contains("ToolSearch"), "{shot}");
        assert!(shot.contains("· recall site.md"), "{shot}");
    }

    #[test]
    fn replies_carry_the_members_mark_and_team_news_reads_apart() {
        let (mut app, id) = working();
        app.on_event(Event::SessionStarted {
            session: 99,
            project: "/repo".into(),
            engine: Engine::Claude,
            model: None,
            workspace: None,
            member: Some(Member { role: "designer".into(), name: "designer".into() }),
            squad: Some("acme".into()),
        });
        app.on_event(Event::TextDelta { session: id, text: "T-8 is ready for you.".into() });
        app.on_event(Event::TurnEnded { session: id, outcome: TurnOutcome::Completed, usage: None });
        app.on_event(Event::Notice { session: id, text: "designer handed in T-8 for your review · Translations.".into() });
        app.on_event(Event::Notice { session: id, text: "You reopened T-6 · Glossary — Reopened: shorter.".into() });
        let shot = screen(&app, 140, 30);
        let lines: Vec<&str> = shot.lines().collect();
        let at = |text: &str| lines.iter().position(|l| l.contains(text)).unwrap_or_else(|| panic!("no {text:?} in:\n{shot}"));
        // The PM's reply opens with its mark; the rest of it hangs under the text.
        assert!(lines[at("Looking now.")].contains("◆ Looking now."), "{shot}");
        assert!(lines[at("T-8 is ready")].contains("◆ T-8 is ready for you."), "{shot}");
        // Steps sit apart from the replies around them.
        assert!(lines[at("src/auth.rs") - 1].trim_end().ends_with('│'), "a blank line before the steps:\n{shot}");
        // Team news: who, in their mark, then what — apart from the reply above.
        assert!(lines[at("designer handed in")].contains("✦ designer handed in T-8"), "{shot}");
        assert!(lines[at("You reopened")].contains("› You reopened T-6"), "{shot}");
        assert_eq!(at("You reopened"), at("designer handed in") + 1, "news keeps together:\n{shot}");
    }

    #[test]
    fn a_ticket_number_in_the_conversation_opens_it_on_the_board() {
        let (mut app, id) = working();
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(8, TicketStatus::Review, "Translations") });
        app.flash = None;
        app.on_event(Event::TextDelta { session: id, text: "T-8 is ready for you.".into() });
        let laid = extent(&app, Theme::plain(), Rect::new(0, 0, 100, 24), &Layouts::default());
        let shot = screen(&app, 100, 24);
        let said = shot.lines().position(|l| l.contains("is ready for you")).unwrap() as u16;
        let hit = laid.hits.iter().find(|h| h.2 == said).copied().expect("T-8 is clickable");
        assert_eq!(hit.3, Hit::Ticket(8));
        let row: String = shot.lines().nth(hit.2 as usize).unwrap().chars().skip(hit.0 as usize).take(3).collect();
        assert_eq!(row, "T-8", "the hit sits on the number:\n{shot}");
        app.set_hits(laid.hits, laid.divider);
        app.on_click(hit.0 + 1, hit.2);
        assert_eq!(app.pane, Pane::Board);
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(8));
    }

    #[test]
    fn work_waiting_for_the_person_stays_in_sight_and_a_click_away() {
        let (mut app, _) = working();
        for id in 1..=4 {
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(id, TicketStatus::Review, &format!("Piece {id}")) });
        }
        app.flash = None;
        let shot = screen(&app, 100, 30);
        // The side list counts it under 나에게, as the footer does.
        assert!(shot.lines().any(|l| l.contains("! Inbox") && l.contains(" 4 ")), "{shot}");
        assert!(shot.lines().last().unwrap().contains("Inbox 4 · ctrl-b"), "{shot}");
        // A click on it opens the board there, in section order.
        let laid = extent(&app, Theme::plain(), Rect::new(0, 0, 100, 30), &Layouts::default());
        let row = shot.lines().position(|l| l.contains("! Inbox")).unwrap() as u16;
        let hit = laid.hits.iter().find(|h| h.2 == row).copied().unwrap();
        assert_eq!(hit.3, Hit::Side(Side::Menu(BoardMenu::Inbox)));
        app.set_hits(laid.hits, laid.divider);
        app.on_click(hit.0 + 1, hit.2);
        assert_eq!((app.pane, app.board_menu), (Pane::Board, BoardMenu::Inbox));
        let board = screen(&app, 100, 30);
        assert!(board.contains("Review  4") && board.contains("› ? T-1"), "{board}");
        app.pane = Pane::Conversation;
        // Without the side list — a narrow terminal — a card under the conversation.
        let narrow = screen(&app, 70, 30);
        assert!(narrow.contains("▎ Review · 4") && narrow.contains("+1 more on the board"), "{narrow}");
        // Nothing waiting, no card.
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(1, TicketStatus::Done, "Piece 1") });
        for id in 2..=4 {
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(id, TicketStatus::Done, "x") });
        }
        assert!(!screen(&app, 70, 30).contains("▎ Review"));
    }

    #[test]
    fn a_ticket_waiting_on_the_person_shows_what_they_have_to_do() {
        let (mut app, _) = working();
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(1, TicketStatus::Review, "Pricing") });
        let mut t = ticket(2, TicketStatus::Waiting, "Analytics");
        t.note = Some("Waiting on you:\n- [ ] GitHub token\n- [ ] PostHog secrets\n(to deploy)".into());
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        app.flash = None;
        let shot = screen(&app, 120, 30);
        assert!(shot.lines().any(|l| l.contains("! Inbox") && l.contains(" 2 ")), "{shot}");
        assert!(shot.contains("Inbox 2 · ctrl-b"), "{shot}");
        // Opened, what it needs.
        app.open(Side::Ticket(2));
        let shot = screen(&app, 120, 30);
        assert!(shot.contains("GitHub token") && shot.contains("PostHog secrets"), "{shot}");
        app.pane = Pane::Board;
        app.board_menu = BoardMenu::Inbox;
        let shot = screen(&app, 120, 30);
        assert!(shot.contains("Your turn  1"), "{shot}");
        let row = |text: &str| shot.lines().position(|l| l.contains(text)).unwrap();
        assert!(row("Your turn") < row("Review  1"), "내 차례 before 검토:\n{shot}");
        assert!(shot.lines().find(|l| l.contains("Analytics")).unwrap().contains("GitHub token"), "what it needs, in its row:\n{shot}");
    }

    #[test]
    fn a_ticket_waiting_on_the_person_lists_its_asks_under_a_heading_in_the_conversation() {
        let (mut app, id) = working();
        let mut t = ticket(117, TicketStatus::Waiting, "App Store submission");
        t.asks = vec![
            "Tell me the review contact: name, phone, email. Only the review team sees it.".into(),
            "Decide when and who does the iOS review.".into(),
            "Check the shots in `outputs/T-117/shots/` and listing.json; if they're fine, just say OK.".into(),
        ];
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t.clone() });
        app.on_event(Event::Notice { session: id, text: "T-117 waits for you — backend needs something only you can do · App Store submission.".into() });
        app.flash = None;
        let shot = screen(&app, 140, 40);
        let lines: Vec<&str> = shot.lines().collect();
        let at = |text: &str| lines.iter().position(|l| l.contains(text)).unwrap_or_else(|| panic!("no {text:?} in:\n{shot}"));
        assert!(lines[at("Your turn · T-117 · App Store submission")].contains("▎ Your turn"), "{shot}");
        assert_eq!(at("1. Tell me the review contact"), at("Your turn · T-117") + 1, "{shot}");
        assert_eq!(at("2. Decide when"), at("1. Tell me the review contact") + 1, "one ask a line:\n{shot}");
        assert_eq!(at("3. Check the shots"), at("2. Decide when") + 1, "{shot}");
        assert!(lines[at("3. Check the shots")].contains("▎"), "{shot}");
        assert!(shot.contains("/resume T-117"), "answering stays as it was:\n{shot}");
        assert!(!shot.contains("waits for you —"), "not one run-on line:\n{shot}");
        // Answered: the line it came as.
        t.status = TicketStatus::InProgress;
        t.asks.clear();
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        let shot = screen(&app, 140, 40);
        assert!(shot.contains("T-117 waits for you") && !shot.contains("1. Tell me"), "{shot}");
    }

    #[test]
    fn a_tiny_terminal_draws_without_crashing() {
        let (app, _) = working();
        for height in 1..12 {
            screen(&app, 40, height);
        }
    }

    #[test]
    fn dragging_over_the_conversation_selects_and_copies_its_text() {
        let (mut app, id) = working();
        app.on_event(Event::TextDelta { session: id, text: "\n\nSecond line here.".into() });
        let screen_rect = Rect::new(0, 0, 100, 24);
        let laid = extent(&app, Theme::plain(), screen_rect, &Layouts::default());
        app.set_hits(laid.hits, laid.divider);
        app.set_text_view(laid.text);
        let shot = screen(&app, 100, 24);
        let row = |text: &str| shot.lines().position(|l| l.contains(text)).unwrap() as u16;
        let view = laid.text.unwrap();
        // From the start of the reply to the end of its second paragraph.
        app.on_click(view.x, row("Looking now."));
        app.on_drag(view.x + 40, row("Second line"));
        app.on_mouse(crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left), Instant::now());
        assert!(app.copy_selection);
        let text = selection_text(&app, Theme::plain(), screen_rect, &Layouts::default()).unwrap();
        assert_eq!(text, "Looking now.\n\nedit   src/auth.rs\n\nSecond line here.", "marks (◆, ✎) stay out");
        // It shows, reversed, until a key is pressed.
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|f| render(f, &app, Theme::plain(), &Layouts::default())).unwrap();
        let cell = terminal.backend().buffer()[(view.x + 2, row("Looking now."))].clone();
        assert!(cell.modifier.contains(Modifier::REVERSED));
        app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('x')), Instant::now());
        assert!(app.selection.is_none());
    }

    #[test]
    fn dragging_over_a_ticket_on_the_board_copies_its_text() {
        let (mut app, _) = working();
        let mut t = ticket(20, TicketStatus::Waiting, "Prove the desktop works");
        t.note = Some("Waiting on you:\n- [ ] Add this file: /private/tmp/probe/desktop-probe".into());
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        app.pane = Pane::Board;
        let screen_rect = Rect::new(0, 0, 100, 30);
        let laid = extent(&app, Theme::plain(), screen_rect, &Layouts::default());
        app.set_hits(laid.hits, laid.divider);
        app.set_text_view(laid.text);
        let shot = screen(&app, 100, 30);
        let y = shot.lines().position(|l| l.contains("/private/tmp/probe")).unwrap() as u16;
        let line = shot.lines().nth(y as usize).unwrap();
        let x = line[..line.find("/private").unwrap()].chars().count() as u16;
        app.on_click(x, y);
        app.on_drag(x + 31, y);
        app.on_mouse(crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left), Instant::now());
        assert!(app.copy_selection);
        assert_eq!(selection_text(&app, Theme::plain(), screen_rect, &Layouts::default()).as_deref(), Some("/private/tmp/probe/desktop-probe"));
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| render(f, &app, Theme::plain(), &Layouts::default())).unwrap();
        assert!(terminal.backend().buffer()[(x + 1, y)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn several_projects_show_as_chips_and_tags() {
        let (mut app, _) = working();
        app.projects = vec!["/code/site".into(), "/code/app".into()];
        let mut t = ticket(3, TicketStatus::Review, "Onboarding screens");
        t.project = Some("/code/app".into());
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        app.flash = None;
        let shot = screen(&app, 110, 30);
        assert!(shot.lines().next().unwrap().contains("site · app ⚑1 · all"), "{shot}");
        assert!(shot.lines().any(|l| l.contains("! Inbox") && l.contains(" 1 ")), "the side list counts it:\n{shot}");
        app.pane = Pane::Board;
        let shot = screen(&app, 150, 30);
        assert!(shot.contains("› ? T-3    app   Onboarding screens"), "{shot}");
        // Its chip is a click away.
        let laid = extent(&app, Theme::plain(), Rect::new(0, 0, 150, 30), &Layouts::default());
        let chip = laid.hits.iter().find(|h| h.3 == Hit::Project(Some(1))).copied().unwrap();
        let row: String = shot.lines().next().unwrap().chars().skip(chip.0 as usize).take(3).collect();
        assert_eq!(row, "app");
    }

    /// A squad of two projects with work in each and in the squad home.
    fn squad_wide(cwd: &str) -> App {
        let snapshot = Snapshot { squad: "acme".into(), projects: vec!["/code/site".into(), "/code/app".into()], ..Snapshot::default() };
        let (mut app, _) = App::from_snapshot(cwd.into(), snapshot, &[]);
        let mut hero = ticket(1, TicketStatus::InProgress, "Hero");
        hero.project = Some("/code/site".into());
        let mut screens = ticket(2, TicketStatus::Review, "Onboarding screens");
        screens.project = Some("/code/app".into());
        let mut research = ticket(3, TicketStatus::Todo, "Competitor research");
        research.project = None;
        research.kind = equipe_core::protocol::TicketKind::Task;
        for t in [hero, screens, research] {
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        }
        app.flash = None;
        app
    }

    #[test]
    fn the_console_opens_on_the_whole_squad_wherever_it_was_run() {
        let inside = squad_wide("/code/site");
        let outside = squad_wide("/tmp/somewhere");
        assert!(inside.focus.is_none(), "no project is put in focus for the folder it was run in");
        assert_eq!(screen(&inside, 120, 30), screen(&outside, 120, 30));
        let shot = screen(&inside, 120, 30);
        let header = shot.lines().next().unwrap();
        assert!(header.contains("acme ▾") && header.contains("squads/acme/home") && header.contains("site · app ⚑1 · all"), "{shot}");
        assert!(!header.contains("/code/site"), "not the folder it was run in:\n{shot}");
        // Telling the daemon: no filter left from before.
        assert_eq!(inside.unfiltered(), Command::Focus { squad: "acme".into(), project: None });
        // A squad with no project yet still shows its home.
        let (lone, _) = App::from_snapshot("/tmp".into(), Snapshot { squad: "acme".into(), ..Snapshot::default() }, &[]);
        let header = screen(&lone, 120, 30).lines().next().unwrap().to_owned();
        assert!(header.contains("squads/acme/home") && !header.contains("all"), "{header}");
    }

    #[test]
    fn the_board_tags_each_ticket_with_its_project_and_kind_and_the_home_has_a_lane() {
        let mut app = squad_wide("/code/site");
        app.pane = Pane::Board;
        let shot = screen(&app, 120, 30);
        let row = |id: &str| shot.lines().find(|l| l.contains(id)).unwrap_or_else(|| panic!("no {id} in:\n{shot}")).to_owned();
        assert!(row("T-1").contains("T-1    site  Hero"), "{shot}");
        assert!(row("T-2").contains("T-2    app   Onboarding screens"), "{shot}");
        assert!(row("T-3").contains("T-3    home  Competitor research"), "{shot}");
        let at = |text: &str| shot.find(text).unwrap_or_else(|| panic!("missing {text:?} in:\n{shot}"));
        assert!(at("Review  1") < at("In progress  1") && at("In progress  1") < at("To do  1"), "by status, no lane per project:\n{shot}");
    }

    #[test]
    fn a_project_filter_narrows_the_board_and_reviews_and_comes_off() {
        let mut app = squad_wide("/tmp");
        let laid = extent(&app, Theme::plain(), Rect::new(0, 0, 120, 30), &Layouts::default());
        app.set_hits(laid.hits.clone(), laid.divider);
        // A click on the site's chip filters to it.
        let chip = laid.hits.iter().find(|h| h.3 == Hit::Project(Some(0))).copied().unwrap();
        app.on_click(chip.0, chip.2);
        assert_eq!(app.focus.as_deref(), Some(std::path::Path::new("/code/site")));
        assert!(app.outbox.contains(&Command::Focus { squad: "acme".into(), project: Some("site".into()) }));
        assert!(app.flash.as_deref().unwrap().starts_with("Only site"), "{:?}", app.flash);
        app.flash = None;
        let shot = screen(&app, 120, 30);
        assert!(!shot.lines().any(|l| l.contains("! Inbox") && l.contains(" 1 ")), "the app's review is filtered out:\n{shot}");
        assert!(shot.lines().last().unwrap().contains("→ pm · site"), "new work goes to the site:\n{shot}");
        app.pane = Pane::Board;
        let shot = screen(&app, 120, 30);
        assert!(shot.contains("Hero") && !shot.contains("Onboarding screens") && !shot.contains("Competitor research"), "{shot}");
        // `/project all` takes it off: the whole squad again.
        assert_eq!(app.focus_on("all"), vec![Command::Focus { squad: "acme".into(), project: None }]);
        app.flash = None;
        let shot = screen(&app, 120, 30);
        assert!(shot.contains("Hero") && shot.contains("Onboarding screens") && shot.contains("Competitor research"), "{shot}");
        app.pane = Pane::Conversation;
        let shot = screen(&app, 120, 30);
        assert!(shot.lines().any(|l| l.contains("! Inbox") && l.contains(" 1 ")), "{shot}");
        assert!(!shot.lines().last().unwrap().contains("· site"), "{shot}");
    }

    #[test]
    fn several_projects_say_where_work_is_and_where_a_message_goes() {
        let (mut app, pm) = working();
        app.projects = vec!["/code/site".into(), "/code/app".into()];
        let mut review = ticket(3, TicketStatus::Review, "Onboarding screens");
        review.project = Some("/code/app".into());
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: review });
        let mut hero = ticket(4, TicketStatus::InProgress, "Hero");
        hero.project = Some("/code/site".into());
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: hero });
        app.on_event(Event::SessionStarted {
            session: 9,
            project: "/code/site".into(),
            engine: Engine::Claude,
            model: None,
            workspace: None,
            member: Some(Member { role: "frontend".into(), name: "frontend".into() }),
            squad: Some("acme".into()),
        });
        let frontend = app.agents.iter_mut().find(|a| a.id == 9).unwrap();
        frontend.ticket = Some(4);
        frontend.state = AgentState::Working;
        app.selected = app.agents.iter().position(|a| a.id == pm).unwrap();
        app.flash = None;
        let shot = screen(&app, 120, 30);
        // The header: who works where, and what waits for the person.
        assert!(shot.lines().next().unwrap().contains("site ●1 · app ⚑1 · all"), "{shot}");
        // The squad: a member's project before its ticket.
        assert!(shot.contains("site T-4"), "{shot}");
        // However narrow the side list, the ticket shows.
        app.projects[0] = "/code/marketing-website".into();
        app.tickets.iter_mut().find(|t| t.id == 4).unwrap().project = Some("/code/marketing-website".into());
        let mut squad = Terminal::new(TestBackend::new(30, 10)).unwrap();
        squad.draw(|f| render_side(f, f.area(), &app, Theme::plain())).unwrap();
        let rows: String = squad.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(rows.contains("T-4 Hero"), "{rows}");
        app.projects[0] = "/code/site".into();
        app.tickets.iter_mut().find(|t| t.id == 4).unwrap().project = Some("/code/site".into());
        // In focus, a message says where it goes.
        app.focus = Some("/code/app".into());
        let shot = screen(&app, 120, 30);
        assert!(shot.lines().last().unwrap().contains("→ pm · app"), "{shot}");
        // The board: by status, each row tagged with its project.
        app.focus = None;
        app.pane = Pane::Board;
        let shot = screen(&app, 120, 30);
        assert!(shot.contains("Review  1") && shot.lines().any(|l| l.contains("T-4") && l.contains("site")), "{shot}");
        // The help names the key.
        app.pane = Pane::Conversation;
        app.help_open = true;
        assert!(screen(&app, 120, 60).contains("Ctrl-P"));
    }

    #[test]
    fn a_long_ticket_scrolls_and_says_there_is_more() {
        let (mut app, _) = working();
        let mut t = ticket(20, TicketStatus::Waiting, "Prove the desktop works");
        let steps: Vec<String> = (1..=30).map(|n| format!("- [ ] step {n}")).collect();
        t.note = Some(format!("Waiting on you:\n{}", steps.join("\n")));
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        app.pane = Pane::Board;
        let screen_rect = Rect::new(0, 0, 100, 30);
        let shot = screen(&app, 100, 30);
        assert!(shot.contains("more lines") && !shot.contains("step 30"), "{shot}");
        let laid = extent(&app, Theme::plain(), screen_rect, &Layouts::default());
        assert!(laid.board_max > 0);
        assert!(laid.board_detail.is_some());
        app.board_scroll = laid.board_max;
        let shot = screen(&app, 100, 30);
        assert!(shot.contains("step 30") && !shot.contains("more lines"), "{shot}");
    }

    #[test]
    fn many_projects_fold_into_more_keeping_what_matters_in_sight() {
        let (mut app, _) = working();
        app.projects = (1..=9).map(|n| std::path::PathBuf::from(format!("/code/project-number-{n}"))).collect();
        let mut waiting = ticket(3, TicketStatus::Review, "Onboarding");
        waiting.project = Some("/code/project-number-8".into());
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: waiting });
        app.focus = Some("/code/project-number-6".into());
        app.flash = None;
        let laid = extent(&app, Theme::plain(), Rect::new(0, 0, 120, 30), &Layouts::default());
        let top = screen(&app, 120, 30).lines().next().unwrap().to_owned();
        // In focus, and waiting on the person: in sight; the rest behind "+N".
        assert!(top.contains("project-number-6") && top.contains("project-number-8 ⚑1"), "{top}");
        assert!(top.contains(" ▾ · all") && top.contains('+'), "{top}");
        assert!(unicode_width::UnicodeWidthStr::width(top.trim_end()) <= 120, "{top}");
        assert!(laid.hits.iter().any(|h| h.3 == Hit::Projects));
        // Shown in their own order, not by importance.
        assert!(top.find("project-number-6").unwrap() < top.find("project-number-8").unwrap());
    }

    #[test]
    fn a_message_to_a_member_at_work_elsewhere_says_where_it_goes() {
        let (mut app, _) = working();
        app.projects = vec!["/code/site".into(), "/code/app".into()];
        let mut hero = ticket(4, TicketStatus::InProgress, "Hero");
        hero.project = Some("/code/site".into());
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: hero });
        app.on_event(Event::SessionStarted {
            session: 9,
            project: "/code/site".into(),
            engine: Engine::Claude,
            model: None,
            workspace: None,
            member: Some(Member { role: "frontend".into(), name: "frontend".into() }),
            squad: Some("acme".into()),
        });
        let at = app.agents.iter().position(|a| a.id == 9).unwrap();
        app.agents[at].ticket = Some(4);
        app.selected = at;
        app.focus = Some("/code/app".into());
        app.flash = None;
        let shot = screen(&app, 140, 30);
        assert!(shot.lines().last().unwrap().contains("at work on site T-4 — this talk goes on beside it"), "{shot}");
    }

    #[test]
    fn a_ticket_says_what_it_waits_for_and_what_it_unblocks() {
        let (mut app, _) = working();
        let mut first = ticket(1, TicketStatus::Done, "Socket");
        first.depends_on = vec![];
        let mut middle = ticket(2, TicketStatus::InProgress, "Sidebar");
        middle.depends_on = vec![1];
        let mut last = ticket(3, TicketStatus::Todo, "Chat");
        last.depends_on = vec![2];
        for t in [first, middle, last] {
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        }
        app.pane = Pane::Board;
        app.board_selected = Some(2);
        let shot = screen(&app, 120, 40);
        assert!(shot.contains("after     T-1 + Socket"), "{shot}");
        assert!(shot.contains("unblocks  T-3"), "{shot}");
        // In the list, what it waits for and what it unblocks, as flags.
        app.board_menu = BoardMenu::All;
        let shot = screen(&app, 120, 40);
        assert!(shot.contains("blocked T-2"), "{shot}");
        assert!(shot.contains("->1"), "{shot}");
    }

    #[test]
    fn a_board_of_hundreds_folds_what_waits_and_finds_the_rest() {
        let (mut app, _) = working();
        for id in 1..=40 {
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(id, TicketStatus::Todo, &format!("Task {id}")) });
        }
        for id in 41..=500 {
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(id, TicketStatus::Done, &format!("Old {id}")) });
        }
        app.pane = Pane::Board;
        app.board_split = Some(40);
        let shot = screen(&app, 120, 60);
        assert!(shot.contains("To do  40") && shot.contains("… 30 more · e shows all · f finds"), "{shot}");
        assert!(shot.contains("Done  460 >  e show finished work"), "the finished in one line, no cap of five:\n{shot}");
        app.board_query = Some("old 123".into());
        let shot = screen(&app, 120, 60);
        assert!(shot.contains("FOUND") && shot.contains("Old 123"), "{shot}");
    }

    #[test]
    fn a_members_subagents_show_under_it_while_they_work() {
        let (mut app, pm) = working();
        for (sub, what) in [("t1", "Run the iOS tests"), ("t2", "Read the logs"), ("t3", "Survey the API")] {
            app.on_event(Event::SubagentStarted { session: pm, id: sub.into(), what: what.into() });
        }
        app.on_event(Event::SubagentStep { session: pm, id: "t1".into(), step: "Bash xcodebuild test".into() });
        app.squad_width = Some(50);
        let shot = screen(&app, 140, 30);
        assert!(shot.contains("↳ Run the iOS tests"), "{shot}");
        assert!(shot.contains("↳ Read the logs") && shot.contains("↳ Survey the API"), "{shot}");
        // The rows under a member still click through to the right member.
        let laid = extent(&app, Theme::plain(), Rect::new(0, 0, 140, 30), &Layouts::default());
        let pm_row = shot.lines().position(|l| l.contains("pm    haiku")).unwrap() as u16;
        assert!(laid.hits.iter().any(|h| h.2 == pm_row && h.3 == Hit::Side(Side::Pm)));
        // Done, gone; the turn's end takes the rest.
        app.on_event(Event::SubagentEnded { session: pm, id: "t2".into() });
        assert!(!screen(&app, 140, 30).contains("Read the logs"));
        app.on_event(Event::TurnEnded { session: pm, outcome: TurnOutcome::Completed, usage: None });
        assert!(!screen(&app, 140, 30).contains("↳"));
    }

    #[test]
    fn a_board_empty_under_a_focus_says_so() {
        let (mut app, _) = working();
        app.projects = vec!["/code/site".into(), "/code/app".into()];
        let mut t = ticket(1, TicketStatus::Todo, "Hero");
        t.project = Some("/code/site".into());
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        app.focus = Some("/code/app".into());
        app.pane = Pane::Board;
        let shot = screen(&app, 100, 24);
        assert!(shot.contains("Nothing on the board for app · /project all"), "{shot}");
    }

    #[test]
    fn ticket_numbers_are_found_as_words() {
        assert_eq!(ticket_ref("see T-12 now"), Some((4, 4)));
        assert_eq!(ticket_ref("T-3"), Some((0, 3)));
        assert_eq!(ticket_ref("UT-3 or T-x"), None);
    }

    #[test]
    fn a_laid_out_conversation_is_reused_and_reports_how_much_it_grew() {
        let (mut app, id) = working();
        let mut layouts = Layouts::default();
        assert_eq!(layouts.prepare(&app, Theme::plain(), 100), None, "the first layout has nothing to compare with");
        assert_eq!(layouts.prepare(&app, Theme::plain(), 100), None, "nothing changed, nothing laid out again");
        app.on_event(Event::TextDelta { session: id, text: "\n\nmore\n\nand more".into() });
        // A blank line after the steps, then the reply's three lines.
        assert_eq!(layouts.prepare(&app, Theme::plain(), 100), Some((id, 4)));
    }

    #[test]
    fn a_light_terminal_is_told_by_colorfgbg() {
        // SAFETY: tests in this module don't read COLORFGBG concurrently.
        unsafe { std::env::set_var("COLORFGBG", "0;15") };
        assert!(light_background());
        unsafe { std::env::set_var("COLORFGBG", "15;0") };
        assert!(!light_background());
        unsafe { std::env::remove_var("COLORFGBG") };
    }

    #[test]
    fn scrolled_up_a_bar_shows_where_and_the_extent_knows_the_ends() {
        let (mut app, id) = working();
        for i in 0..40 {
            app.on_event(Event::TextDelta { session: id, text: format!("\n\nline {i}") });
        }
        let extent = extent(&app, Theme::plain(), Rect::new(0, 0, 100, 24), &Layouts::default());
        assert!(extent.conversation_max > 20, "{}", extent.conversation_max);
        assert!(extent.page >= 3);
        app.set_extent(extent.page, extent.conversation_max, extent.board_max);
        app.agents[0].scroll_back = 10_000;
        app.set_extent(extent.page, extent.conversation_max, extent.board_max);
        assert_eq!(app.agents[0].scroll_back, extent.conversation_max, "clamped to the top");
        let shot = screen(&app, 100, 24);
        assert!(shot.contains('┃'), "{shot}");
        assert!(shot.contains("› fix the bug"), "at the top, the first message shows:\n{shot}");
    }

    #[test]
    fn a_long_run_of_tool_use_folds_and_o_unfolds_it() {
        let (mut app, id) = working();
        for i in 0..9 {
            app.on_event(Event::ToolUse { session: id, tool: "Edit".into(), target: Some(format!("src/f{i}.rs")) });
        }
        let shot = screen(&app, 100, 30);
        // The edit from `working()` and nine more make a run of ten: three, the fold, two.
        assert!(shot.contains("· 5 more steps · ctrl-o to unfold"), "{shot}");
        assert_eq!(shot.matches("✎ edit").count(), 5, "{shot}");
        app.agents[0].expanded = true;
        let shot = screen(&app, 100, 30);
        assert_eq!(shot.matches("✎ edit").count(), 10, "{shot}");
        assert!(!shot.contains("more steps"), "{shot}");
    }

    #[test]
    fn the_persons_messages_sit_on_a_wash_of_the_terminals_background() {
        let (app, _) = working();
        let theme = Theme { color: true, truecolor: true, light: false, background: Some((20, 20, 24)) };
        let mut terminal = Terminal::new(TestBackend::new(100, 22)).unwrap();
        terminal.draw(|f| render(f, &app, theme, &Layouts::default())).unwrap();
        let buffer = terminal.backend().buffer();
        let row = (0..22).find(|&y| (0..100).map(|x| buffer[(x, y)].symbol()).collect::<String>().contains("fix the bug")).unwrap();
        let x = (0..100).find(|&x| buffer[(x, row)].symbol() == "f").unwrap();
        // White at 7% over (20, 20, 24).
        assert_eq!(buffer[(x, row)].bg, Color::Rgb(36, 36, 40));
        // Without the terminal's answer there is no wash: a guess would show as a slab.
        assert_eq!(Theme { background: None, ..theme }.tint(), None);
    }

    #[test]
    fn every_member_has_a_mark_and_colour_of_its_own() {
        let roles = ["pm", "designer", "frontend", "backend", "qa", "data"];
        let looks: Vec<_> = roles.iter().map(|r| look(r, r)).collect();
        for (i, a) in looks.iter().enumerate() {
            for b in &looks[i + 1..] {
                assert!(a.0 != b.0 && a.1 != b.1, "{looks:?}");
            }
        }
        // Anyone else is told apart from the built-in roles, and keeps theirs.
        assert!(look("writer", "writer").1 >= 6);
        assert_eq!(look("writer", "writer"), look("writer", "writer"));
        let theme = Theme { color: true, truecolor: true, light: false, background: None };
        assert_ne!(theme.member(0), theme.member(2));
    }

    #[test]
    fn the_working_mark_turns_breathes_and_shimmers() {
        let theme = Theme { color: true, truecolor: true, light: false, background: None };
        let start = Instant::now();
        let _ = Theme::since_start(start);
        let at = |ms: u64| start + std::time::Duration::from_millis(ms);
        // The star turns, frame by frame, and comes back.
        assert_ne!(theme.spinner(at(0)).content, theme.spinner(at(120)).content);
        assert_eq!(theme.spinner(at(0)).content, theme.spinner(at(1200)).content);
        // Its colour breathes — dim, then the accent, then dim again.
        assert_ne!(theme.breath(at(0)).fg, theme.breath(at(1000)).fg);
        assert_eq!(theme.breath(at(0)).fg, theme.breath(at(2000)).fg);
        // Light passes over the word: its brightest letter moves.
        let brightest = |ms: u64| {
            let spans = theme.shimmer("working", at(ms));
            let level = |s: &Span| match s.style.fg {
                Some(Color::Rgb(r, g, b)) => r as u32 + g as u32 + b as u32,
                _ => 0,
            };
            (0..spans.len()).max_by_key(|&i| level(&spans[i])).unwrap()
        };
        assert!(brightest(500) < brightest(1000), "left to right");
        // Without truecolor it holds still in the accent.
        assert_eq!(Theme { truecolor: false, ..theme }.shimmer("working", at(500)).len(), 1);
    }

    #[test]
    fn the_squad_pane_is_as_wide_on_any_terminal_and_the_conversation_takes_the_rest() {
        let narrow = screen(&working().0, 110, 20);
        let wide = screen(&working().0, 160, 20);
        let bar = |shot: &str| shot.lines().nth(2).unwrap().find('│').unwrap();
        assert_eq!(bar(&wide), bar(&narrow), "{wide}");
    }

    #[test]
    fn big_counts_read_at_a_glance() {
        assert_eq!(short_count(1_234), "1,234");
        assert_eq!(short_count(812_400), "812k");
        assert_eq!(short_count(24_288_758), "24.3M");
    }

    #[test]
    fn group_digits_reads_like_a_number() {
        assert_eq!(group_digits(1234567), "1,234,567");
        assert_eq!(group_digits(999), "999");
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::tui::app::App;
    use equipe_core::protocol::{Command, Engine, Event, Member, Snapshot};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    #[ignore]
    fn how_long_a_long_conversation_takes_to_draw() {
        let snapshot = Snapshot { squad: "acme".into(), projects: vec!["/repo".into()], ..Snapshot::default() };
        let (mut app, commands) = App::from_snapshot("/repo".into(), snapshot, &[]);
        let Some(Command::Start { session: id, .. }) = commands.first() else { panic!() };
        let id = *id;
        app.on_event(Event::SessionStarted { session: id, project: "/repo".into(), engine: Engine::Claude, model: None, workspace: None, member: Some(Member { role: "pm".into(), name: "pm".into() }), squad: Some("acme".into()) });
        for i in 0..200 {
            app.on_event(Event::UserMessage { session: id, text: format!("question {i}") });
            app.on_event(Event::TextDelta { session: id, text: "## Plan\n\n1. **designer**: 랜딩 페이지 카피와 디자인을 맡아요. equipe가 뭔지 담아요.\n2. **frontend**: 정적 사이트 프로젝트를 세팅해요 (Astro 제안).\n\n| 티켓 | 담당 | 내용 |\n|---|---|---|\n| T-1 | designer | 카피·디자인 |\n| T-2 | frontend | Astro 셋업 |\n\nSome `code` and more prose that goes on for a while to wrap across lines in the pane.".into() });
            app.on_event(Event::ToolUse { session: id, tool: "Read".into(), target: Some("src/a.rs".into()) });
            app.on_event(Event::TurnEnded { session: id, outcome: equipe_core::protocol::TurnOutcome::Completed, usage: None });
        }
        let mut layouts = Layouts::default();
        let mut terminal = Terminal::new(TestBackend::new(140, 45)).unwrap();
        let t = std::time::Instant::now();
        layouts.prepare(&app, Theme::plain(), 140);
        let layout = t.elapsed();
        let t = std::time::Instant::now();
        for _ in 0..50 {
            app.agents[0].scroll_back += 3;
            terminal.draw(|f| render(f, &app, Theme::plain(), &layouts)).unwrap();
        }
        eprintln!("lay out once: {layout:?}; draw while scrolling: {:?} per frame", t.elapsed() / 50);
    }
}
