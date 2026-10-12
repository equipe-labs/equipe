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
use equipe_core::protocol::{Entry, SessionId, TicketId, TicketStatus};

use super::app::{Agent, AgentState, App, Divider, Hit, Pane, Spot, TextView, elapsed, relative, tool_verb};
use super::markdown;
use crate::ui;

/// Prose is laid out no wider than this: past it, lines get hard to read.
const PROSE_MAX: usize = 96;

/// The squad pane's width — wider on a wide terminal — and the gap holding the
/// hairline beside it.
/// The squad pane's narrowest and, as a share of the body, widest.
const SQUAD_MIN: u16 = 14;
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
        let name = app.agents.iter().map(|a| a.label().chars().count()).max().unwrap_or(4).clamp(4, 12);
        let detail = app.agents.iter().filter_map(|a| a.short_model()).map(|m| m.chars().count()).max().unwrap_or(0).max(9);
        // marker, mark and a space, the name, two spaces, the detail, a spare column.
        (2 + 2 + name + 2 + detail + 1) as u16
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
type LayoutKey = (usize, usize, usize, usize, bool);

/// Conversations already laid out, per member. Laying one out means reading all
/// of its Markdown; scrolling must not do that again for every wheel event.
#[derive(Default)]
pub struct Layouts {
    laid_out: HashMap<SessionId, (LayoutKey, Vec<Line<'static>>)>,
}

impl Layouts {
    /// Lay out the selected member's conversation for a terminal `width` columns
    /// wide, reusing the last layout when nothing changed. Returns the member and
    /// how many lines its conversation grew by, so a view scrolled up can stay put.
    pub fn prepare(&mut self, app: &App, theme: Theme, width: u16) -> Option<(SessionId, usize)> {
        if app.pane != Pane::Conversation {
            return None;
        }
        let agent = app.selected()?;
        let text_width = transcript_width(app, width);
        let key = layout_key(agent, text_width);
        let previous = match self.laid_out.get(&agent.id) {
            Some((k, _)) if *k == key => return None,
            Some((k, lines)) if k.0 == text_width => Some(lines.len()),
            _ => None,
        };
        let lines = transcript_lines(agent, &squad_looks(app), theme, text_width);
        let grew = previous.map(|before| lines.len().saturating_sub(before));
        self.laid_out.insert(agent.id, (key, lines));
        grew.filter(|n| *n > 0).map(|n| (agent.id, n))
    }

    fn get(&self, agent: &Agent, width: usize) -> Option<&[Line<'static>]> {
        match self.laid_out.get(&agent.id) {
            Some((key, lines)) if *key == layout_key(agent, width) => Some(lines),
            _ => None,
        }
    }
}

fn layout_key(agent: &Agent, width: usize) -> LayoutKey {
    let entries = agent.transcript.entries();
    let last = entries.last().map_or(0, |entry| match entry {
        Entry::User { text } | Entry::Reply { text } | Entry::Note { text } => text.len(),
        Entry::Tool { tool, target } => tool.len() + target.as_ref().map_or(0, String::len),
        Entry::Error { message, .. } => message.len(),
        Entry::Landed { stat, .. } => stat.len(),
    });
    (width, entries.len(), last, agent.queued.len(), agent.expanded)
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
    conversation_areas(under_title(area), app, agent).0
}

/// The conversation pane under its title: the conversation, then — when there
/// is any — work waiting for the person, then the live row.
fn conversation_areas(area: Rect, app: &App, agent: &Agent) -> (Rect, Rect, Rect) {
    let live = (agent.scroll_back > 0 && agent.unseen > 0) || agent.state == AgentState::Working;
    let card = waiting_card_height(app, area.height);
    let [text, _, card, _, live] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(u16::from(card > 0)),
        Constraint::Length(card),
        Constraint::Length(u16::from(live)),
        Constraint::Length(u16::from(live)),
    ])
    .areas(area);
    (text, card, live)
}

/// How many tickets the card names before it counts the rest.
const CARD_SHOWN: usize = 3;

fn waiting_card_height(app: &App, room: u16) -> u16 {
    let waiting = app.to_review().len();
    // Never on a short screen: the conversation comes first.
    if waiting == 0 || room < 16 {
        return 0;
    }
    (1 + waiting.min(CARD_SHOWN) + usize::from(waiting > CARD_SHOWN)) as u16
}

/// Work waiting for the person, in sight under any conversation: a heading,
/// then a row a ticket — each a click from its review on the board.
fn waiting_card(app: &App, theme: Theme, width: usize) -> Vec<(Line<'static>, Option<TicketId>)> {
    let waiting = app.to_review();
    if waiting.is_empty() {
        return Vec::new();
    }
    let bar = || Span::styled("▎ ", theme.accent());
    let mut rows = vec![(
        Line::from(vec![
            bar(),
            Span::styled(format!("Waiting for your review · {}", waiting.len()), theme.accent().add_modifier(Modifier::BOLD)),
            Span::styled("   click one, or ctrl-b", theme.dim()),
        ]),
        None,
    )];
    let looks = squad_looks(app);
    for t in waiting.iter().take(CARD_SHOWN) {
        let who = t.assignee.as_ref().or(t.owner.as_ref()).cloned().unwrap_or_default();
        let style = looks.iter().find(|(name, ..)| *name == who).map_or_else(|| theme.dim(), |l| theme.member(l.2));
        let id = format!("{:<6}", format!("T-{}", t.id));
        // Owners line up: titles take a fixed width.
        let title_room = width.saturating_sub(2 + 6 + 2 + 12).min(56);
        let title = ui::fit(&t.title, title_room);
        let pad = title_room.saturating_sub(unicode_width::UnicodeWidthStr::width(title.as_str()));
        rows.push((
            Line::from(vec![
                bar(),
                Span::styled(id, theme.accent()),
                Span::styled(title, Style::new()),
                Span::raw(" ".repeat(pad + 2)),
                Span::styled(who, style),
            ]),
            Some(t.id),
        ));
    }
    if waiting.len() > CARD_SHOWN {
        rows.push((Line::from(vec![bar(), Span::styled(format!("+{} more on the board", waiting.len() - CARD_SHOWN), theme.dim())]), None));
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
    // The squad's name in the header, and its ▾: the way to the other squads.
    let header = content(screen);
    let name_x = header.x + 10;
    hits.push((name_x, name_x + unicode_width::UnicodeWidthStr::width(app.squad.as_str()) as u16 + 2, header.y, Hit::Squads));
    if app.pane == Pane::Board {
        let width = (board.width as usize).saturating_sub(1).min(PROSE_MAX + 20);
        let rows = board_list(app, theme, width);
        let (list, _) = board_areas(board, rows.len());
        let top = list_top(&rows, app.selected_ticket().map(|t| t.id), list.height as usize);
        for (i, (_, id)) in rows.iter().enumerate().skip(top).take(list.height as usize) {
            if let Some(id) = id {
                hits.push((list.x, list.x + list.width, list.y + (i - top) as u16, Hit::Ticket(*id)));
            }
        }
    }
    let Some(agent) = app.selected() else {
        return Extent { page: board.height as usize / 2, conversation_max: 0, board_max: 0, hits, divider, text: None };
    };
    let text = conversation_text(right, app, agent);
    // The waiting card's rows open their tickets too.
    if app.pane == Pane::Conversation {
        let (_, card, _) = conversation_areas(under_title(right), app, agent);
        for (i, (_, id)) in waiting_card(app, theme, card.width as usize).into_iter().enumerate().take(card.height as usize) {
            if let Some(id) = id {
                hits.push((card.x, card.x + card.width, card.y + i as u16, Hit::Ticket(id)));
            }
        }
    }
    let width = (text.width as usize).saturating_sub(1).min(PROSE_MAX);
    let fresh;
    let laid: &[Line<'static>] = match layouts.get(agent, width) {
        Some(lines) => lines,
        None => {
            fresh = transcript_lines(agent, &squad_looks(app), theme, width);
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
        page: text.height as usize / 2,
        conversation_max: lines.saturating_sub(text.height as usize),
        board_max: 0,
        hits,
        divider,
        text: text_view,
    }
}

pub fn render(frame: &mut Frame, app: &App, theme: Theme, branch: Option<&str>, layouts: &Layouts) {
    let Rows { header, body, approval, rule, composer, footer, asking } = rows(app, theme, frame.area());
    let asking_height = approval.height;

    let (squad, gap, right) = columns(app, body);
    render_header(frame, header, app, theme, branch, squad.is_none());
    if let (Some(squad), Some(gap)) = (squad, gap) {
        render_squad(frame, squad, app, theme);
        render_hairline(frame, gap, app.is_dragging(), theme);
    }
    match app.pane {
        Pane::Conversation if app.help_open => render_help(frame, right, theme),
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
fn render_choices(frame: &mut Frame, above: Rect, app: &App, theme: Theme) {
    const SHOWN: usize = 8;
    let choices = app.choices();
    if choices.is_empty() || above.height < 3 {
        return;
    }
    let picked = app.pick.min(choices.len() - 1);
    let first = (picked + 1).saturating_sub(SHOWN);
    let label_width = choices.iter().map(|c| c.label.chars().count()).max().unwrap_or(0).min(40) + 3;
    let mut lines: Vec<Line> = vec![Line::from(Span::styled("─".repeat(above.width as usize), theme.dim()))];
    for (i, choice) in choices.iter().enumerate().skip(first).take(SHOWN) {
        let (mark, label_style) =
            if i == picked { (Span::styled("› ", theme.accent()), theme.accent().add_modifier(Modifier::BOLD)) } else { (Span::raw("  "), theme.bold()) };
        lines.push(Line::from(vec![
            mark,
            Span::styled(format!("{:<label_width$}", ui::fit(&choice.label, label_width - 3)), label_style),
            Span::styled(ui::fit(&choice.about, (above.width as usize).saturating_sub(label_width + 2)), theme.dim()),
        ]));
    }
    let more = choices.len().saturating_sub(first + SHOWN);
    lines.push(Line::from(Span::styled(
        format!("  ↑↓ pick · tab or enter takes it · esc closes{}", if more > 0 { format!(" · {more} more") } else { String::new() }),
        theme.dim(),
    )));
    let height = (lines.len() as u16).min(above.height);
    let area = Rect { y: above.y + above.height - height, height, ..above };
    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_widget(Paragraph::new(lines), area);
}

/// `/help`: what the console does, grouped — talking, looking around,
/// answering — and every command, from the list completion uses.
fn render_help(frame: &mut Frame, area: Rect, theme: Theme) {
    let title = vec![Span::styled("Help", theme.bold())];
    let area = pane_title(title, vec![Span::styled("esc closes", theme.dim())], area, frame);
    let width = (area.width as usize).saturating_sub(1);
    let commands: Vec<(String, &str)> =
        super::commands::COMMANDS.iter().map(|c| (format!("/{} {}", c.name, c.args).trim_end().to_owned(), c.about)).collect();
    // Keys and commands each line up in a column as wide as their longest.
    let rows = |pairs: &[(String, &str)], column: usize| -> Vec<Line<'static>> {
        let column = column.min(width / 2) + 3;
        pairs
            .iter()
            .flat_map(|(k, what)| {
                wrapped(vec![Span::styled(format!("  {:<column$}", ui::fit(k, column - 1)), theme.bold())], what, theme.dim(), column + 2, width)
            })
            .collect()
    };
    let heading = |h: &str| vec![Line::raw(""), Line::from(Span::styled(h.to_owned(), theme.accent().add_modifier(Modifier::BOLD)))];
    let pairs = |list: &[(&str, &'static str)]| -> Vec<(String, &'static str)> { list.iter().map(|(k, w)| ((*k).to_owned(), *w)).collect() };
    let mut lines = wrapped(
        Vec::new(),
        "The PM is your main contact: tell it what you want, it plans the work on the board, and equipe hands each ticket to its owner.",
        Style::new(),
        0,
        width,
    );
    lines.extend(heading("TALK"));
    // The keys of all three groups share one column.
    let keys = 19;
    lines.extend(rows(&pairs(&[
        ("Enter", "send to the member named under the composer"),
        ("@name …", "send to someone else — the PM hears about it"),
        ("Tab · Shift-Tab", "pick who you talk to"),
        ("Alt-Enter · Ctrl-J", "a new line (or \\ then Enter)"),
        ("Esc", "stop a member mid-turn; a waiting memo goes in at once"),
        ("→", "take the greyed suggestion in an empty composer"),
    ]), keys));
    lines.extend(heading("LOOK AROUND"));
    lines.extend(rows(&pairs(&[
        ("Ctrl-B", "the board, and back"),
        ("PgUp · PgDn · wheel", "scroll · End for the latest · Ctrl-O unfolds steps"),
        ("click · drag", "a ticket number opens it · drag over text to copy it · drag the squad's edge to resize it"),
    ]), keys));
    lines.extend(heading("ANSWER"));
    lines.extend(rows(&pairs(&[
        ("y · a · n", "allow · always allow · decline a request"),
        ("on the board", "a accept · r ask for changes · d the diff · Enter talk to its owner"),
    ]), keys));
    lines.extend(heading("MODES — how much members do on their own · /mode [member] <mode>"));
    let modes: Vec<(String, &str)> =
        equipe_core::protocol::Mode::ALL.iter().map(|m| (m.label().to_owned(), m.summary())).collect();
    lines.extend(rows(&modes, keys));
    lines.extend(heading("COMMANDS — type / to complete"));
    let widest = commands.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);
    lines.extend(rows(&commands, widest));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled("Ctrl-C twice leaves — the squad keeps working in the background.", theme.dim())));
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App, theme: Theme, branch: Option<&str>, narrow: bool) {
    let mut place = display_path(&app.cwd);
    if let Some(branch) = branch {
        place.push_str(" · ");
        place.push_str(branch);
    }
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
        let summary = if working > 0 { format!("   {open} members · {working} working") } else { format!("   {open} members") };
        spans.push(Span::styled(summary, theme.dim()));
    }
    let used: usize = spans.iter().map(Span::width).sum();
    // Keep the end of a long path — the project and branch — and drop the start.
    let room = (area.width as usize).saturating_sub(used + 3);
    if room > 4 {
        let width = unicode_width::UnicodeWidthStr::width(place.as_str());
        if width > room {
            let tail: String = place.chars().rev().collect();
            place = format!("…{}", ui::fit(&tail, room - 1).trim_end_matches('…').chars().rev().collect::<String>());
        }
        spans.push(Span::styled(format!("   {place}"), theme.dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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
    if app.needs_you(agent) {
        return Span::styled("✻", theme.accent().add_modifier(Modifier::BOLD));
    }
    let (mark, hue) = agent_look(agent);
    match agent.state {
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
            "read" => "reading",
            "edit" | "write" => "editing",
            "run" => "running",
            "search" => "searching",
            "fetch" => "looking it up",
            "board" => "checking the board",
            "ticket" => "updating the board",
            "comment" => "commenting",
            "hand in" => "handing in",
            "browse" => "browsing",
            "ask you" => "waiting for you",
            _ => "working",
        },
        Some(Entry::Reply { .. }) => "writing",
        _ => "thinking",
    }
}

fn render_squad(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let mut lines = vec![Line::from(Span::styled("SQUAD", theme.dim())), Line::raw("")];
    let name_width = app.agents.iter().map(|a| a.label().chars().count()).max().unwrap_or(4).clamp(4, 12);
    for i in app.display_order() {
        let agent = &app.agents[i];
        let selected = i == app.selected && app.pane == Pane::Conversation;
        let marker = if selected { Span::styled("› ", theme.accent()) } else { Span::raw("  ") };
        let name = ui::fit(&agent.label(), name_width);
        let style = theme.member(agent_look(agent).1);
        let name = Span::styled(format!("{name:<name_width$}"), if selected { style.add_modifier(Modifier::BOLD) } else { style });
        // What it is doing, the most pressing first: needs you, a memo waiting,
        // its ticket, its model.
        let detail = if app.needs_you(agent) {
            Span::styled("needs you", theme.accent())
        } else {
            let text = match (agent.queued.len(), agent.ticket) {
                (1, _) => "1 memo".to_owned(),
                (n @ 2.., _) => format!("{n} memos"),
                (0, Some(ticket)) => format!("T-{ticket}"),
                (0, None) => agent.short_model().unwrap_or_default(),
            };
            // An open browser, and what it costs in memory.
            let text = match app.browsers.get(&agent.label()) {
                Some(mb) if *mb >= 1024 => format!("{text} · {:.1}G", *mb as f32 / 1024.0),
                Some(mb) => format!("{text} · {mb}M"),
                None => text,
            };
            Span::styled(text, theme.dim())
        };
        lines.push(Line::from(vec![marker, glyph(app, agent, theme), Span::raw(" "), name, Span::raw("  "), detail]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// The squad's edge; lit while the person drags it.
fn render_hairline(frame: &mut Frame, area: Rect, held: bool, theme: Theme) {
    let style = if held { theme.accent() } else { theme.dim() };
    let lines: Vec<Line> = (0..area.height).map(|_| Line::from(Span::styled(" │", style))).collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// Wrap `text` so its first row follows `lead` and later rows are indented.
fn wrapped(lead: Vec<Span<'static>>, text: &str, style: Style, indent: usize, width: usize) -> Vec<Line<'static>> {
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
    wrapped(vec![mark], &format!("{:<6} {}", line.verb, named.join(", ")), theme.dim(), 2, width)
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
        Span::styled(format!("· {hidden} more steps · "), theme.dim()),
        Span::styled("ctrl-o", theme.bold()),
        Span::styled(" to unfold", theme.dim()),
    ]));
    for line in &run[run.len() - tail..] {
        lines.extend(tool_line(line, theme, width));
    }
    lines
}

/// The person's own message: `›` and bold text, on a faint wash of the
/// background when the terminal told us its colour.
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

fn transcript_lines(agent: &Agent, looks: &Looks, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if agent.transcript.is_empty() && agent.queued.is_empty() {
        let hello = if agent.is_pm() {
            "Tell the PM what you want to build — it plans the work and puts it on the board, and equipe hands \
each ticket to its owner. Tab or @name talks to anyone directly; Ctrl-B shows the board."
        } else if agent.edit {
            "It picks up its tickets from the board and works on each in a worktree of its own. \
You can talk to it directly — the PM hears about it."
        } else {
            "Talk to it directly — the PM hears about it."
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
    for entry in agent.transcript.entries() {
        match entry {
            // Claude Code loading tool descriptions: its business, not the person's.
            Entry::Tool { tool, .. } if tool == "ToolSearch" => {}
            Entry::Tool { tool, target } => {
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
            Entry::User { text } => {
                begin(Block::User, &mut lines, &mut last);
                gap(&mut lines);
                lines.extend(user_message(text, theme, width));
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
            Entry::Note { text } => match news_lines(text, looks, theme, width) {
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
                lines.push(Line::from(vec![
                    Span::styled("✓ ", theme.accent()),
                    Span::raw("Changes committed to "),
                    Span::styled(branch.clone(), theme.bold()),
                ]));
                for row in stat.lines() {
                    lines.push(Line::from(Span::styled(format!("  {}", row.trim()), theme.dim())));
                }
            }
            Entry::Tool { .. } => {}
        }
    }
    flush(&mut run, &mut lines, &mut last);
    // Memos wait below the conversation until the turn ends.
    for memo in &agent.queued {
        gap(&mut lines);
        lines.extend(wrapped(vec![Span::styled("› ", theme.dim())], memo, theme.dim(), 2, width));
        lines.push(Line::from(Span::styled("  memo · read when this turn ends · Esc to read now", theme.dim())));
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
            let review = app.to_review().len();
            let mut spans = vec![Span::styled("Board", theme.bold())];
            if review > 0 {
                spans.push(Span::styled(format!(" · {review} to review"), theme.accent()));
            }
            spans.push(Span::styled("  ctrl-b", theme.dim()));
            spans
        }
        Pane::Board => {
            let Some(agent) = app.selected() else {
                return vec![Span::styled("← conversation", theme.bold()), Span::styled("  ctrl-b", theme.dim())];
            };
            vec![
                Span::styled("← ", theme.bold()),
                Span::styled(agent.label(), theme.member(agent_look(agent).1).add_modifier(Modifier::BOLD)),
                Span::styled("  ctrl-b", theme.dim()),
            ]
        }
    }
}

fn render_transcript(frame: &mut Frame, area: Rect, app: &App, theme: Theme, layouts: &Layouts) {
    let Some(agent) = app.selected() else {
        return;
    };
    let mut title = vec![Span::styled(agent.label(), theme.member(agent_look(agent).1).add_modifier(Modifier::BOLD))];
    match agent.ticket.and_then(|id| app.tickets.iter().find(|t| t.id == id)) {
        Some(ticket) => title.push(Span::styled(format!("  ·  T-{} {}", ticket.id, ticket.title), theme.dim())),
        None => title.push(Span::styled(format!("  ·  {}", agent.engine_label()), theme.dim())),
    }
    let area = pane_title(title, switch_label(app, Pane::Conversation, theme), area, frame);

    // At the bottom, when it applies: the live working row, or what arrived
    // while the person was scrolled up.
    let mut footer: Vec<Line<'static>> = Vec::new();
    if agent.scroll_back > 0 && agent.unseen > 0 {
        footer.push(Line::from(vec![
            Span::styled(format!("↓ {} new lines", agent.unseen), theme.accent()),
            Span::styled(" · End", theme.dim()),
        ]));
    } else if agent.state == AgentState::Working {
        let now = Instant::now();
        let took = agent.working_since.map(|since| elapsed(now.duration_since(since))).unwrap_or_default();
        let mut row = vec![theme.spinner(now), Span::raw(" ")];
        row.extend(theme.shimmer(doing(agent), now));
        row.extend([
            Span::styled(format!(" · {took} · esc to stop"), theme.dim()),
        ]);
        footer.push(Line::from(row));
    }
    let (text, card, live) = conversation_areas(area, app, agent);
    if card.height > 0 {
        let rows: Vec<Line> = waiting_card(app, theme, card.width as usize).into_iter().map(|(line, _)| line).collect();
        frame.render_widget(Paragraph::new(rows), card);
    }

    let width = (text.width as usize).saturating_sub(1).min(PROSE_MAX);
    let fresh;
    let lines: &[Line<'static>] = match layouts.get(agent, width) {
        Some(lines) => lines,
        None => {
            fresh = transcript_lines(agent, &squad_looks(app), theme, width);
            &fresh
        }
    };
    // Follow the bottom unless the person scrolled up; draw only what shows.
    let height = text.height as usize;
    let bottom = lines.len().saturating_sub(height);
    let top = bottom.saturating_sub(agent.scroll_back.min(bottom));
    let shown = lines[top..lines.len().min(top + height)].to_vec();
    frame.render_widget(Paragraph::new(shown), text);
    // What the person is selecting, in reverse.
    if let Some((from, to)) = app.selection.map(ordered) {
        let buffer = frame.buffer_mut();
        for row in 0..text.height {
            let line = top + row as usize;
            if line < from.0 || line > to.0 {
                continue;
            }
            let start = if line == from.0 { from.1 } else { 0 };
            let end = if line == to.0 { to.1 + 1 } else { width as u16 };
            for column in start..end.min(text.width) {
                if let Some(cell) = buffer.cell_mut((text.x + column, text.y + row)) {
                    cell.set_style(cell.style().add_modifier(Modifier::REVERSED));
                }
            }
        }
    }
    // Scrolled up, a thin bar on the right says where in the conversation this is.
    if top < bottom {
        render_scrollbar(frame, text, top, height, lines.len(), theme);
    }
    if !footer.is_empty() {
        frame.render_widget(Paragraph::new(footer), live);
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
    let agent = app.selected()?;
    let body = rows(app, theme, screen).body;
    let (_, _, right) = columns(app, body);
    let width = (conversation_text(right, app, agent).width as usize).saturating_sub(1).min(PROSE_MAX);
    let fresh;
    let lines: &[Line<'static>] = match layouts.get(agent, width) {
        Some(lines) => lines,
        None => {
            fresh = transcript_lines(agent, &squad_looks(app), theme, width);
            &fresh
        }
    };
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

/// The board's list: a heading per group, one row per ticket — the person's
/// reviews first. Returns the rows, and which ticket each row is.
fn board_list(app: &App, theme: Theme, width: usize) -> Vec<(Line<'static>, Option<u32>)> {
    let order = app.board_order();
    let picked = app.selected_ticket().map(|t| t.id);
    let several_projects = app.projects.len() > 1;
    let heading = |status: TicketStatus| match status {
        TicketStatus::Review => "YOUR REVIEW",
        TicketStatus::InProgress => "IN PROGRESS",
        TicketStatus::Todo => "TO DO",
        TicketStatus::Proposed => "PROPOSED",
        TicketStatus::Done | TicketStatus::Cancelled => "DONE",
    };
    let mut rows: Vec<(Line<'static>, Option<u32>)> = Vec::new();
    let mut current: Option<&str> = None;
    for id in &order {
        let Some(t) = app.tickets.iter().find(|t| t.id == *id) else {
            continue;
        };
        let label = heading(t.status);
        if current != Some(label) {
            if current.is_some() {
                rows.push((Line::raw(""), None));
            }
            let count = order
                .iter()
                .filter_map(|id| app.tickets.iter().find(|t| t.id == *id))
                .filter(|t| heading(t.status) == label)
                .count();
            let style = if t.status == TicketStatus::Review { theme.accent().add_modifier(Modifier::BOLD) } else { theme.dim() };
            rows.push((Line::from(Span::styled(format!("{label}  {count}"), style)), None));
            current = Some(label);
        }
        let selected = picked == Some(t.id);
        let who = t.assignee.as_ref().or(t.owner.as_ref()).cloned().unwrap_or_default();
        let state = match t.status {
            TicketStatus::Review => "waits for you".to_owned(),
            TicketStatus::InProgress => "working".to_owned(),
            TicketStatus::Todo if !t.depends_on.is_empty() => {
                format!("after {}", t.depends_on.iter().map(|d| format!("T-{d}")).collect::<Vec<_>>().join(", "))
            }
            TicketStatus::Todo => "to do".to_owned(),
            TicketStatus::Proposed => "proposed".to_owned(),
            TicketStatus::Done => "done".to_owned(),
            TicketStatus::Cancelled => "cancelled".to_owned(),
        };
        let place = if several_projects { t.project.as_deref().map(project_name).map(|p| format!(" · {p}")).unwrap_or_default() } else { String::new() };
        // Columns: marker, id, title (what's left), owner, state.
        // Fixed columns, so owners and states line up down the list.
        let tail_width = (12 + 14 + if several_projects { 14 } else { 0 }).min(width / 2);
        let title_room = width.saturating_sub(2 + 6 + 2 + tail_width);
        let title = ui::fit(&t.title, title_room);
        let pad = title_room.saturating_sub(unicode_width::UnicodeWidthStr::width(title.as_str()));
        let closed = t.status.is_closed();
        let title_style = match (selected, closed) {
            (true, _) => theme.bold(),
            (false, true) => theme.dim(),
            (false, false) => Style::new(),
        };
        let state_style = if t.status == TicketStatus::Review { theme.accent() } else { theme.dim() };
        rows.push((
            Line::from(vec![
                if selected { Span::styled("› ", theme.accent()) } else { Span::raw("  ") },
                Span::styled(format!("{:<6}", format!("T-{}", t.id)), if selected { theme.bold() } else { theme.dim() }),
                Span::styled(title, title_style),
                Span::raw(" ".repeat(pad + 2)),
                Span::styled(format!("{who:<10}  "), if closed { theme.dim() } else { member_style(app, &who, theme) }),
                Span::styled(ui::fit(&format!("{state}{place}"), tail_width.saturating_sub(12)), state_style),
            ]),
            Some(t.id),
        ));
    }
    rows
}

/// The picked ticket in full: what it is, who, what changed, where to look at
/// it, and the keys that act on it.
fn ticket_detail(app: &App, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let Some(t) = app.selected_ticket() else {
        return wrapped(
            Vec::new(),
            "The board is empty. Tell the PM what you want to build — it puts the work here.",
            theme.dim(),
            0,
            width,
        );
    };
    let mut lines = wrapped(vec![Span::styled(format!("T-{}  ", t.id), theme.bold())], &t.title, theme.bold(), 0, width);
    let mut facts = Vec::new();
    if let Some(who) = t.assignee.as_ref().or(t.owner.as_ref()) {
        facts.push(who.clone());
    }
    facts.push(
        match t.status {
            TicketStatus::Review => "handed in — waits for your review",
            TicketStatus::InProgress => "in progress",
            TicketStatus::Todo => "to do",
            TicketStatus::Proposed => "proposed",
            TicketStatus::Done => "done",
            TicketStatus::Cancelled => "cancelled",
        }
        .to_owned(),
    );
    if let Some(project) = t.project.as_deref().filter(|_| app.projects.len() > 1) {
        facts.push(project_name(project));
    }
    lines.extend(wrapped(Vec::new(), &facts.join(" · "), theme.dim(), 0, width));
    // The keys come right under the title, so a long hand-in never pushes
    // them out of sight.
    let who = t.assignee.as_ref().or(t.owner.as_ref());
    let mut actions: Vec<(&str, String)> = Vec::new();
    match t.status {
        TicketStatus::Review => {
            actions.push(("a", "accept".into()));
            actions.push(("r", "ask for changes".into()));
        }
        TicketStatus::Done => actions.push(("r", "reopen".into())),
        TicketStatus::InProgress | TicketStatus::Todo | TicketStatus::Proposed => actions.push(("a", "mark done".into())),
        TicketStatus::Cancelled => {}
    }
    if t.branch.is_some() {
        actions.push(("d", "see the diff".into()));
    }
    if let Some(who) = who {
        actions.push(("enter", format!("talk to {who}")));
    }
    actions.push(("↑↓", "tickets".into()));
    let mut keys = Vec::new();
    for (i, (k, what)) in actions.into_iter().enumerate() {
        if i > 0 {
            keys.push(Span::raw("   "));
        }
        // The call the person is here to make stands out.
        let style = if i == 0 && t.status == TicketStatus::Review { theme.accent().add_modifier(Modifier::BOLD) } else { theme.bold() };
        keys.push(Span::styled(k.to_owned(), style));
        keys.push(Span::styled(format!(" {what}"), theme.dim()));
    }
    lines.push(Line::from(keys));
    if let Some(note) = &t.note {
        lines.push(Line::raw(""));
        lines.extend(markdown::lines(note, width, &theme.markdown()));
    } else if !t.body.is_empty() {
        lines.push(Line::raw(""));
        lines.extend(markdown::lines(&t.body, width, &theme.markdown()));
    }
    // What changed, and where to see it.
    let key = |k: &str| Span::styled(format!("{k:<9}"), theme.dim());
    let mut place = Vec::new();
    if let Some((branch, stat)) = app.diffs.get(&t.id).filter(|(b, _)| Some(b) == t.branch.as_ref()) {
        let files: Vec<&str> = stat.lines().filter(|l| l.contains('|')).map(|l| l.split('|').next().unwrap_or("").trim()).collect();
        let summary = stat.lines().last().map(str::trim).unwrap_or("").to_owned();
        if !summary.is_empty() {
            place.push(Line::from(vec![key("changed"), Span::raw(summary)]));
            let shown: Vec<&str> = files.iter().take(4).copied().collect();
            let more = if files.len() > 4 { format!(" +{}", files.len() - 4) } else { String::new() };
            place.push(Line::from(vec![key(""), Span::styled(ui::fit(&format!("{}{more}", shown.join(" · ")), width.saturating_sub(9)), theme.dim())]));
        }
        place.push(Line::from(vec![key("branch"), Span::styled(branch.clone(), theme.dim())]));
    } else if let Some(branch) = &t.branch {
        place.push(Line::from(vec![key("branch"), Span::styled(branch.clone(), theme.dim())]));
    }
    if let Some(worktree) = &t.worktree {
        place.push(Line::from(vec![key("look in"), Span::styled(display_path(worktree), theme.dim())]));
    }
    if !place.is_empty() {
        lines.push(Line::raw(""));
        lines.extend(place);
    }
    lines
}

/// The board pane split: the list on top — as tall as it needs, up to half —
/// and the picked ticket's detail below.
fn board_areas(area: Rect, list_rows: usize) -> (Rect, Rect) {
    let list_height = (list_rows as u16).min(area.height / 2).max(3.min(area.height));
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
    let title = vec![Span::styled("Board", theme.bold())];
    let area = pane_title(title, switch_label(app, Pane::Board, theme), area, frame);
    let width = (area.width as usize).saturating_sub(1).min(PROSE_MAX + 20);
    let rows = board_list(app, theme, width);
    let (list, detail) = board_areas(area, rows.len());
    let top = list_top(&rows, app.selected_ticket().map(|t| t.id), list.height as usize);
    let shown: Vec<Line> = rows.into_iter().skip(top).take(list.height as usize).map(|(line, _)| line).collect();
    frame.render_widget(Paragraph::new(shown), list);
    // A hairline between the list and the ticket open below it.
    let [rule, body] = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(detail);
    frame.render_widget(Paragraph::new(Span::styled("─".repeat(width.min(rule.width as usize)), theme.dim())), rule);
    let detail_width = (body.width as usize).saturating_sub(1).min(PROSE_MAX);
    frame.render_widget(Paragraph::new(ticket_detail(app, theme, detail_width)), body);
}

/// The oldest request for permission, held in place above the composer behind
/// one accent bar — what exactly, who, why, and the keys that answer it.
fn approval_lines(app: &App, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let Some(approval) = app.approvals.first() else {
        return Vec::new();
    };
    let verb = match approval.tool.as_str() {
        "Bash" => "run",
        "Write" | "Edit" | "MultiEdit" => "edit",
        "WebFetch" => "fetch",
        "Confirm" => "do",
        equipe_core::permissions::BROWSER_CODE => "code",
        other => other,
    };
    let bar = || Span::styled("▎ ", theme.accent());
    let width = width.saturating_sub(2);
    let ticket = approval.ticket.map(|t| format!(" · T-{t}")).unwrap_or_default();
    let mut lines = vec![Line::from(vec![
        bar(),
        Span::styled(approval.member.clone(), theme.bold()),
        Span::styled(format!(" asks{ticket}"), theme.accent()),
    ])];
    // A few lines of the command, never the whole script: the conversation keeps
    // its room, and the keys always show.
    // Code is what's being allowed: more of it shows.
    let shown = if approval.tool == equipe_core::permissions::BROWSER_CODE { 12 } else { 3 };
    let rows: Vec<&str> = approval.action.lines().collect();
    for (i, row) in rows.iter().take(shown).enumerate() {
        let lead = if i == 0 { format!("{verb}  ") } else { " ".repeat(verb.chars().count() + 2) };
        let room = width.saturating_sub(lead.chars().count());
        lines.push(Line::from(vec![bar(), Span::styled(lead, theme.dim()), Span::styled(ui::fit(row, room), theme.bold())]));
    }
    let writes = files_written(&approval.action);
    if !writes.is_empty() {
        lines.push(Line::from(vec![bar(), Span::styled(ui::fit(&format!("writes {}", writes.join(", ")), width), theme.dim())]));
    }
    if rows.len() > shown {
        lines.push(Line::from(vec![bar(), Span::styled(format!("… {} more lines", rows.len() - shown), theme.dim())]));
    }
    if let Some(reason) = &approval.reason {
        lines.push(Line::from(vec![bar(), Span::styled(ui::fit(reason, width), theme.dim())]));
    }
    // "Always" only where it can become a rule — a chain, an edit outside the
    // worktree, is allowed once at a time; offering more would be a promise
    // equipe can't keep.
    if app.pane == Pane::Board {
        // On the board these keys act on tickets; answers wait for the conversation.
        lines.push(Line::from(vec![bar(), Span::styled("ctrl-b", theme.bold()), Span::styled(" to answer", theme.dim())]));
        return lines;
    }
    let mut keys = vec![bar(), Span::styled("y", theme.bold()), Span::styled(" allow   ", theme.dim())];
    if let Some(rule) = equipe_core::permissions::rule_for(&approval.tool, &approval.action) {
        keys.push(Span::styled("a", theme.bold()));
        keys.push(Span::styled(format!(" always allow {}   ", always_label(&rule)), theme.dim()));
    }
    keys.extend([Span::styled("n", theme.bold()), Span::styled(" decline", theme.dim())]);
    if app.approvals.len() > 1 {
        keys.push(Span::styled(format!("   · {} more waiting", app.approvals.len() - 1), theme.dim()));
    }
    lines.push(Line::from(keys));
    // Asked too often? The way to stop it, right where the asking is.
    let member = &approval.member;
    lines.push(Line::from(vec![
        bar(),
        Span::styled(ui::fit(&format!("/mode {member} full stops {member} asking · /mode for all modes"), width), theme.dim()),
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
            Span::styled("   → to fill in", theme.dim()),
        ]))
    } else if app.composer.text().is_empty() {
        let to = app.addressee().map(|(name, _)| name).unwrap_or_else(|| "the squad".into());
        Paragraph::new(Line::from(vec![
            Span::styled("› ", theme.accent()),
            Span::styled(format!("Message {to}…  @name for someone else · /help"), theme.dim()),
        ]))
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
    let left: Vec<Span> = match (&app.flash, app.addressee()) {
        (Some(flash), _) => vec![Span::styled(flash.clone(), theme.bold())],
        (None, Some((name, direct))) => {
            let style = member_style(app, &name, theme).add_modifier(Modifier::BOLD);
            let mut spans = vec![Span::styled("→ ", theme.dim()), Span::styled(name, style)];
            if direct {
                spans.push(Span::styled(" · the PM hears about it", theme.dim()));
            }
            if let Some(agent) = app.selected().filter(|a| a.member.is_some()) {
                spans.push(Span::styled(format!(" · {}", app.mode_of(agent).label()), theme.dim()));
            }
            spans
        }
        _ => Vec::new(),
    };
    let mut facts: Vec<(String, Style)> = Vec::new();
    // Work waiting for the person comes first: it's what only they can do.
    let review = app.to_review().len();
    if review > 0 {
        facts.push((format!("{review} to review · ctrl-b"), theme.accent()));
    }
    // A multi-line draft (often a paste) says how big it is before it's sent.
    let draft_lines = app.composer.line_count();
    if draft_lines > 1 {
        facts.push((format!("{draft_lines} lines"), theme.dim()));
    }
    let tokens = app.total_tokens();
    if tokens > 0 {
        facts.push((format!("{} tokens", group_digits(tokens)), theme.dim()));
    }
    let left_width: usize = left.iter().map(|s| s.width()).sum();
    // Facts first; the key hints — key in bold, what it does dim — go when space runs short.
    let mut right: Vec<Vec<Span<'static>>> = facts.into_iter().map(|(text, style)| vec![Span::styled(text, style)]).collect();
    let keys: Vec<Vec<Span<'static>>> = [("enter", "send"), ("tab", "squad"), ("/help", "")]
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
        terminal.draw(|f| render(f, app, Theme::plain(), Some("main"), &Layouts::default())).unwrap();
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
            created_by: "pm".into(),
            note: None,
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
        }
    }

    #[test]
    fn shows_the_squad_the_conversation_and_what_the_member_does() {
        let shot = screen(&working().0, 100, 22);
        for expected in ["equipe.", "acme", "/repo · main", "SQUAD", "pm    haiku", " │", "pm  ·  claude code", "› fix the bug", "Looking now.", "✎ edit   src/auth.rs", "editing · 0s", "esc to stop", "Board  ctrl-b", "→ pm · auto", "enter send"] {
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
    fn a_finished_turn_shows_tokens_how_long_it_took_and_a_ready_member() {
        let (mut app, id) = working();
        app.agents[0].working_since = Some(Instant::now() - std::time::Duration::from_secs(65));
        app.on_event(Event::TurnEnded {
            session: id,
            outcome: TurnOutcome::Completed,
            usage: Some(Usage { input_tokens: Some(1200), output_tokens: Some(34) }),
        });
        let shot = screen(&app, 100, 22);
        assert!(shot.contains("1,234 tokens"), "{shot}");
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
        // The pane starts after the margin, the (wide) squad and the hairline: 2 + 32 + 4.
        assert!(longest <= 38 + PROSE_MAX, "{longest}");
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
            usage: Some(Usage { input_tokens: Some(100), output_tokens: Some(20) }),
        });
        app.on_paste("one\ntwo\nthree");
        let shot = screen(&app, 60, 22);
        let footer = shot.lines().last().unwrap();
        assert!(footer.contains("3 lines") && footer.contains("120 tokens"), "{footer:?}");
        assert!(!footer.contains("enter send"), "{footer:?}");
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
        app.on_event(Event::MessageQueued { session: pm, text: "and the footer".into() });
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
        assert!(fitted < 30, "no wider than its names need:\n{shot}");

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
        let shot = screen(&app, 100, 34);
        let at = |text: &str| shot.find(text).unwrap_or_else(|| panic!("missing {text:?} in:\n{shot}"));
        assert!(at("YOUR REVIEW  1") < at("IN PROGRESS  1"), "{shot}");
        assert!(at("IN PROGRESS") < at("TO DO") && at("TO DO") < at("DONE"), "{shot}");
        // One row a ticket, the review picked first; its columns line up.
        let row = |id: &str| shot.lines().find(|l| l.contains(id)).unwrap_or_else(|| panic!("no {id} in:\n{shot}")).to_owned();
        assert!(row("T-3").contains("› T-3   Pricing section"), "{shot}");
        let column = |id: &str| { let r = row(id); r[..r.find("frontend").unwrap()].chars().count() };
        assert!(["T-2", "T-4", "T-1"].iter().all(|id| column(id) == column("T-3")), "owners in one column:\n{shot}");
        // Below, the picked ticket in full with the keys that act on it.
        assert!(shot.contains("handed in — waits for your review"), "{shot}");
        assert!(shot.contains("a accept   r ask for changes"), "{shot}");
        // Right under the title, however long the hand-in below runs.
        let at = |text: &str| shot.lines().position(|l| l.contains(text)).unwrap();
        assert_eq!(at("a accept"), at("waits for your review") + 1, "{shot}");
        assert!(shot.contains("← pm  ctrl-b"), "the way back is in sight:\n{shot}");
        assert!(shot.lines().last().unwrap().contains("1 to review"), "{shot}");
        // ↓ picks the next ticket, and the detail follows.
        app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Down), Instant::now());
        let shot = screen(&app, 100, 34);
        assert!(shot.contains("› T-2") && shot.contains("in progress"), "{shot}");
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
        for expected in ["▎ frontend asks · T-3", "▎ run  npm publish", "▎ Release the package", "▎ y allow   a always allow npm publish   n decline", "▎ /mode frontend full stops frontend asking"] {
            assert!(shot.contains(expected), "missing {expected:?} in:\n{shot}");
        }
    }

    #[test]
    fn a_long_script_asks_in_a_few_lines_and_says_what_it_writes() {
        let (mut app, _) = working();
        let script = "mkdir -p src/pages\ncat > astro.config.mjs <<'EOF'\nexport default {}\nEOF\ncat > tsconfig.json <<'EOF'\n{}\nEOF\necho done";
        app.on_event(Event::ApprovalRequested { squad: "acme".into(), approval: approval(script) });
        let shot = screen(&app, 100, 30);
        assert!(shot.contains("writes astro.config.mjs, tsconfig.json"), "{shot}");
        assert!(shot.contains("… 5 more lines"), "{shot}");
        assert!(shot.contains("y allow"), "the keys always show:\n{shot}");
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
        let shot = screen(&app, 100, 30);
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
        assert!(shot.contains("▎ Waiting for your review · 4"), "{shot}");
        assert!(shot.contains("▎ T-1   Piece 1") && shot.contains("+1 more on the board"), "{shot}");
        let owner = |id: &str| shot.lines().find(|l| l.contains(id)).unwrap().find("frontend").unwrap();
        assert_eq!(owner("Piece 1"), owner("Piece 3"), "owners line up:\n{shot}");
        let laid = extent(&app, Theme::plain(), Rect::new(0, 0, 100, 30), &Layouts::default());
        let row = shot.lines().position(|l| l.contains("Piece 2")).unwrap() as u16;
        let hit = laid.hits.iter().find(|h| h.2 == row).copied().unwrap();
        assert_eq!(hit.3, Hit::Ticket(2));
        // Nothing waiting, no card.
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(1, TicketStatus::Done, "Piece 1") });
        for id in 2..=4 {
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(id, TicketStatus::Done, "x") });
        }
        assert!(!screen(&app, 100, 30).contains("Waiting for your review"));
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
        terminal.draw(|f| render(f, &app, Theme::plain(), None, &Layouts::default())).unwrap();
        let cell = terminal.backend().buffer()[(view.x + 2, row("Looking now."))].clone();
        assert!(cell.modifier.contains(Modifier::REVERSED));
        app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('x')), Instant::now());
        assert!(app.selection.is_none());
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
        terminal.draw(|f| render(f, &app, theme, None, &Layouts::default())).unwrap();
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
            terminal.draw(|f| render(f, &app, Theme::plain(), None, &layouts)).unwrap();
        }
        eprintln!("lay out once: {layout:?}; draw while scrolling: {:?} per frame", t.elapsed() / 50);
    }
}
