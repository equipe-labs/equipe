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
use equipe_core::protocol::{Entry, SessionId, Ticket, TicketStatus};

use super::app::{Agent, AgentState, App, Pane, elapsed, relative, tool_verb};
use super::markdown;
use crate::ui;

/// Prose is laid out no wider than this: past it, lines get hard to read.
const PROSE_MAX: usize = 96;

/// The squad pane's width, and the gap holding the hairline beside it.
const SQUAD_WIDTH: u16 = 26;
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
}

impl Theme {
    pub fn detect() -> Self {
        let (color, truecolor) = ui::out().palette();
        Self { color, truecolor, light: light_background() }
    }

    #[cfg(test)]
    pub fn plain() -> Self {
        Self { color: false, truecolor: false, light: false }
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

/// The squad pane, the hairline gap, and the conversation, side by side.
fn columns(body: Rect) -> (Option<Rect>, Option<Rect>, Rect) {
    if body.width + 4 < SQUAD_MIN_TOTAL {
        return (None, None, body);
    }
    let [squad, gap, right] =
        Layout::horizontal([Constraint::Length(SQUAD_WIDTH), Constraint::Length(GAP), Constraint::Min(20)]).areas(body);
    (Some(squad), Some(gap), right)
}

/// The width a conversation is laid out at, in a terminal `total` columns wide.
fn transcript_width(total: u16) -> usize {
    let body = content(Rect::new(0, 0, total, 1));
    let (_, _, right) = columns(body);
    (right.width as usize).saturating_sub(1).min(PROSE_MAX)
}

// ── laid-out conversations ──

/// What a laid-out conversation depends on: the width it was laid out for, and
/// enough of the conversation to notice it changed — its length, the length of
/// its last line (a reply grows as it streams), the memos waiting.
type LayoutKey = (usize, usize, usize, usize);

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
        let text_width = transcript_width(width);
        let key = layout_key(agent, text_width);
        let previous = match self.laid_out.get(&agent.id) {
            Some((k, _)) if *k == key => return None,
            Some((k, lines)) if k.0 == text_width => Some(lines.len()),
            _ => None,
        };
        let lines = transcript_lines(agent, theme, text_width);
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
    (width, entries.len(), last, agent.queued.len())
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
fn conversation_text(area: Rect, agent: &Agent) -> Rect {
    let live = (agent.scroll_back > 0 && agent.unseen > 0) || agent.state == AgentState::Working;
    let [text, _, _] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(u16::from(live)),
        Constraint::Length(u16::from(live)),
    ])
    .areas(under_title(area));
    text
}

/// How much there is to scroll on a screen this size: half the conversation's
/// height for a page, and how far the conversation and the board can go.
pub struct Extent {
    pub page: usize,
    pub conversation_max: usize,
    pub board_max: usize,
}

pub fn extent(app: &App, theme: Theme, screen: Rect, layouts: &Layouts) -> Extent {
    let (_, _, right) = columns(rows(app, theme, screen).body);
    let board = under_title(right);
    let board_width = (board.width as usize).saturating_sub(1).min(PROSE_MAX);
    let board_max = board_lines(app, theme, board_width).len().saturating_sub(board.height as usize);
    let Some(agent) = app.selected() else {
        return Extent { page: board.height as usize / 2, conversation_max: 0, board_max };
    };
    let text = conversation_text(right, agent);
    let width = (text.width as usize).saturating_sub(1).min(PROSE_MAX);
    let lines = layouts.get(agent, width).map_or_else(|| transcript_lines(agent, theme, width).len(), <[_]>::len);
    Extent {
        page: text.height as usize / 2,
        conversation_max: lines.saturating_sub(text.height as usize),
        board_max,
    }
}

pub fn render(frame: &mut Frame, app: &App, theme: Theme, branch: Option<&str>, layouts: &Layouts) {
    let Rows { header, body, approval, rule, composer, footer, asking } = rows(app, theme, frame.area());
    let asking_height = approval.height;

    let (squad, gap, right) = columns(body);
    render_header(frame, header, app, theme, branch, squad.is_none());
    if let (Some(squad), Some(gap)) = (squad, gap) {
        render_squad(frame, squad, app, theme);
        render_hairline(frame, gap, theme);
    }
    match app.pane {
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

/// Who it is right now, as a glyph: needs the person, working, ready…
fn glyph(app: &App, agent: &Agent, theme: Theme) -> Span<'static> {
    if app.needs_you(agent) {
        return Span::styled("✻", theme.accent().add_modifier(Modifier::BOLD));
    }
    match agent.state {
        AgentState::Starting => Span::styled("…", theme.dim()),
        AgentState::Ready => Span::styled("○", theme.dim()),
        AgentState::Working => Span::styled("●", theme.accent()),
        AgentState::Closing | AgentState::Closed => Span::styled("·", theme.dim()),
        AgentState::Failed => Span::styled("✗", theme.bold()),
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
        let name = Span::styled(format!("{name:<name_width$}"), if selected { theme.bold() } else { Style::new() });
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
            Span::styled(text, theme.dim())
        };
        lines.push(Line::from(vec![marker, glyph(app, agent, theme), Span::raw(" "), name, Span::raw("  "), detail]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_hairline(frame: &mut Frame, area: Rect, theme: Theme) {
    let lines: Vec<Line> = (0..area.height).map(|_| Line::from(Span::styled(" │", theme.dim()))).collect();
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

fn transcript_lines(agent: &Agent, theme: Theme, width: usize) -> Vec<Line<'static>> {
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
    let mut pending: Option<ToolLine> = None;
    let flush = |pending: &mut Option<ToolLine>, lines: &mut Vec<Line<'static>>| {
        if let Some(line) = pending.take() {
            lines.extend(tool_line(&line, theme, width));
        }
    };
    // A blank line between turns: before each message, and between it and its answer.
    let gap = |lines: &mut Vec<Line<'static>>| {
        if lines.last().is_some_and(|l| l.width() > 0) {
            lines.push(Line::raw(""));
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
                let merges = matches!(verb.as_str(), "read" | "search" | "fetch" | "recall");
                match &mut pending {
                    Some(line) if merges && line.verb == verb => {
                        if !target.is_empty() && !line.targets.contains(&target) {
                            line.targets.push(target);
                        }
                    }
                    _ => {
                        flush(&mut pending, &mut lines);
                        let line = ToolLine { verb, targets: if target.is_empty() { Vec::new() } else { vec![target] }, edits };
                        if merges {
                            pending = Some(line);
                        } else {
                            lines.extend(tool_line(&line, theme, width));
                        }
                    }
                }
                continue;
            }
            _ => flush(&mut pending, &mut lines),
        }
        match entry {
            Entry::User { text } => {
                gap(&mut lines);
                lines.extend(wrapped(vec![Span::styled("› ", theme.accent())], text, theme.bold(), 2, width));
                lines.push(Line::raw(""));
            }
            Entry::Reply { text } => lines.extend(markdown::lines(text, width, &theme.markdown())),
            Entry::Note { text } => {
                // A ticket starting, a turn's length: chapters in the member's day.
                if text.starts_with("── ") || text.starts_with("─ worked") {
                    gap(&mut lines);
                }
                lines.extend(wrapped(Vec::new(), text, theme.dim(), 0, width));
            }
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
    flush(&mut pending, &mut lines);
    // Memos wait below the conversation until the turn ends.
    for memo in &agent.queued {
        gap(&mut lines);
        lines.extend(wrapped(vec![Span::styled("› ", theme.dim())], memo, theme.dim(), 2, width));
        lines.push(Line::from(Span::styled("  memo · read when this turn ends · Esc to read now", theme.dim())));
    }
    lines
}

/// The pane's title line: who, in bold, and what they are on, dim.
fn pane_title(spans: Vec<Span<'static>>, area: Rect, frame: &mut Frame) -> Rect {
    let [title, _, rest] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(spans)), title);
    rest
}

fn render_transcript(frame: &mut Frame, area: Rect, app: &App, theme: Theme, layouts: &Layouts) {
    let Some(agent) = app.selected() else {
        return;
    };
    let mut title = vec![Span::styled(agent.label(), theme.bold())];
    match agent.ticket.and_then(|id| app.tickets.iter().find(|t| t.id == id)) {
        Some(ticket) => title.push(Span::styled(format!("  ·  T-{} {}", ticket.id, ticket.title), theme.dim())),
        None => title.push(Span::styled(format!("  ·  {}", agent.engine_label()), theme.dim())),
    }
    let area = pane_title(title, area, frame);

    // At the bottom, when it applies: the live working row, or what arrived
    // while the person was scrolled up.
    let mut footer: Vec<Line<'static>> = Vec::new();
    if agent.scroll_back > 0 && agent.unseen > 0 {
        footer.push(Line::from(vec![
            Span::styled(format!("↓ {} new lines", agent.unseen), theme.accent()),
            Span::styled(" · End", theme.dim()),
        ]));
    } else if agent.state == AgentState::Working {
        let took = agent.working_since.map(|since| elapsed(Instant::now().duration_since(since))).unwrap_or_default();
        footer.push(Line::from(vec![
            Span::styled("● ", theme.accent()),
            Span::styled("working", theme.accent()),
            Span::styled(format!(" · {took} · esc to stop"), theme.dim()),
        ]));
    }
    let [text, _, live] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(u16::from(!footer.is_empty())),
        Constraint::Length(footer.len() as u16),
    ])
    .areas(area);

    let width = (text.width as usize).saturating_sub(1).min(PROSE_MAX);
    let fresh;
    let lines: &[Line<'static>] = match layouts.get(agent, width) {
        Some(lines) => lines,
        None => {
            fresh = transcript_lines(agent, theme, width);
            &fresh
        }
    };
    // Follow the bottom unless the person scrolled up; draw only what shows.
    let height = text.height as usize;
    let bottom = lines.len().saturating_sub(height);
    let top = bottom.saturating_sub(agent.scroll_back.min(bottom));
    let shown = lines[top..lines.len().min(top + height)].to_vec();
    frame.render_widget(Paragraph::new(shown), text);
    // Scrolled up, a thin bar on the right says where in the conversation this is.
    if top < bottom {
        render_scrollbar(frame, text, top, height, lines.len(), theme);
    }
    if !footer.is_empty() {
        frame.render_widget(Paragraph::new(footer), live);
    }
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

/// The board, grouped by what needs whom: the person's reviews first.
fn board_lines(app: &App, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let groups: [(&str, &[TicketStatus]); 5] = [
        ("YOUR REVIEW", &[TicketStatus::Review]),
        ("IN PROGRESS", &[TicketStatus::InProgress]),
        ("TO DO", &[TicketStatus::Todo]),
        ("PROPOSED", &[TicketStatus::Proposed]),
        ("DONE", &[TicketStatus::Done, TicketStatus::Cancelled]),
    ];
    let several_projects = app.projects.len() > 1;
    let mut lines = Vec::new();
    for (label, statuses) in groups {
        let mut tickets: Vec<&Ticket> = app.tickets.iter().filter(|t| statuses.contains(&t.status)).collect();
        if tickets.is_empty() {
            continue;
        }
        if label == "DONE" {
            // The latest few; the rest is history.
            tickets.reverse();
            tickets.truncate(5);
        }
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        let heading = if label == "YOUR REVIEW" { theme.accent().add_modifier(Modifier::BOLD) } else { theme.dim() };
        lines.push(Line::from(Span::styled(format!("{label}  {}", tickets.len()), heading)));
        lines.push(Line::raw(""));
        for (n, t) in tickets.into_iter().enumerate() {
            // Work in flight says more than its title: each stands apart from the next.
            if n > 0 && matches!(t.status, TicketStatus::Review | TicketStatus::InProgress) {
                lines.push(Line::raw(""));
            }
            let id_style = if t.status == TicketStatus::Review { theme.bold() } else { theme.dim() };
            let id = Span::styled(format!("{:<5}", format!("T-{}", t.id)), id_style);
            let title_style = if t.status.is_closed() { theme.dim() } else { Style::new() };
            lines.extend(wrapped(vec![id], &t.title, title_style, 5, width));
            let mut facts = Vec::new();
            if let Some(who) = t.assignee.as_ref().or(t.owner.as_ref()) {
                facts.push(who.clone());
            }
            if several_projects && let Some(project) = &t.project {
                facts.push(project_name(project));
            }
            if !t.depends_on.is_empty() && t.status == TicketStatus::Todo {
                let ids: Vec<String> = t.depends_on.iter().map(|d| format!("T-{d}")).collect();
                facts.push(format!("after {}", ids.join(", ")));
            }
            if t.status == TicketStatus::Cancelled {
                facts.push("cancelled".into());
            }
            if t.status == TicketStatus::Done
                && let Some(branch) = &t.branch
            {
                facts.push(format!("git merge {branch}"));
            }
            if !facts.is_empty() {
                lines.extend(wrapped(vec![Span::raw(" ".repeat(5))], &facts.join(" · "), theme.dim(), 5, width));
            }
            if let Some(note) = &t.note
                && !t.status.is_closed()
            {
                lines.extend(wrapped(vec![Span::raw(" ".repeat(5))], note, theme.dim(), 5, width));
            }
            if t.status == TicketStatus::Review {
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(5)),
                    Span::styled(format!("/accept T-{}", t.id), theme.bold()),
                    Span::styled(format!("  or  /reopen T-{} <what to change>", t.id), theme.dim()),
                ]));
            }
        }
    }
    if lines.is_empty() {
        return wrapped(
            Vec::new(),
            "The board is empty. Tell the PM what you want to build — it puts the work here.",
            theme.dim(),
            0,
            width,
        );
    }
    lines
}

fn render_board(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let title = vec![Span::styled("Board", theme.bold()), Span::styled("  ·  ctrl-b or esc to go back", theme.dim())];
    let area = pane_title(title, area, frame);
    let width = (area.width as usize).saturating_sub(1).min(PROSE_MAX);
    let lines = board_lines(app, theme, width);
    let top = app.board_scroll.min(lines.len().saturating_sub(1));
    frame.render_widget(Paragraph::new(lines).scroll((top as u16, 0)), area);
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
    const SHOWN: usize = 3;
    let rows: Vec<&str> = approval.action.lines().collect();
    for (i, row) in rows.iter().take(SHOWN).enumerate() {
        let lead = if i == 0 { format!("{verb}  ") } else { " ".repeat(verb.chars().count() + 2) };
        let room = width.saturating_sub(lead.chars().count());
        lines.push(Line::from(vec![bar(), Span::styled(lead, theme.dim()), Span::styled(ui::fit(row, room), theme.bold())]));
    }
    let writes = files_written(&approval.action);
    if !writes.is_empty() {
        lines.push(Line::from(vec![bar(), Span::styled(ui::fit(&format!("writes {}", writes.join(", ")), width), theme.dim())]));
    }
    if rows.len() > SHOWN {
        lines.push(Line::from(vec![bar(), Span::styled(format!("… {} more lines", rows.len() - SHOWN), theme.dim())]));
    }
    if let Some(reason) = &approval.reason {
        lines.push(Line::from(vec![bar(), Span::styled(ui::fit(reason, width), theme.dim())]));
    }
    let mut keys = vec![
        bar(),
        Span::styled("y", theme.bold()),
        Span::styled(" allow   ", theme.dim()),
        Span::styled("a", theme.bold()),
        Span::styled(" always allow   ", theme.dim()),
        Span::styled("n", theme.bold()),
        Span::styled(" decline", theme.dim()),
    ];
    if app.approvals.len() > 1 {
        keys.push(Span::styled(format!("   · {} more waiting", app.approvals.len() - 1), theme.dim()));
    }
    lines.push(Line::from(keys));
    lines
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
    let content = if app.composer.text().is_empty() {
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
    frame.set_cursor_position((area.x + 2 + cursor_col as u16, area.y + (cursor_row - first) as u16));
}

/// The bottom line: who a message goes to (always — nothing reaches the wrong
/// person), or a passing message; then what waits for the person, and the keys.
fn render_footer(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let left: Vec<Span> = match (&app.flash, app.addressee()) {
        (Some(flash), _) => vec![Span::styled(flash.clone(), theme.bold())],
        (None, Some((name, direct))) => {
            let mut spans = vec![Span::styled("→ ", theme.dim()), Span::styled(name, theme.bold())];
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
    // When space runs short, the key hints go first; the facts stay.
    let hints = ("enter send · tab squad · /help".to_owned(), theme.dim());
    let width = |items: &[(String, Style)]| {
        items.iter().map(|(t, _)| t.chars().count()).sum::<usize>() + 3 * items.len().saturating_sub(1)
    };
    let mut right = facts.clone();
    right.push(hints);
    if left_width + width(&right) + 3 > area.width as usize {
        right = facts;
    }
    let gap = (area.width as usize).saturating_sub(left_width + width(&right));
    let mut spans = left;
    if gap >= 3 && !right.is_empty() {
        spans.push(Span::raw(" ".repeat(gap)));
        for (i, (text, style)) in right.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", theme.dim()));
            }
            spans.push(Span::styled(text, style));
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
    use equipe_core::protocol::{Command, Engine, Event, Member, Snapshot, TicketId, TurnOutcome, Usage, Workspace};
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
        for expected in ["equipe.", "acme", "/repo · main", "SQUAD", "› ● pm", " │", "pm  ·  claude code", "› fix the bug", "Looking now.", "✎ edit   src/auth.rs", "● working", "esc to stop", "→ pm · auto", "enter send"] {
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
        assert!(shot.contains("○ pm"), "{shot}");
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
        // The pane starts after the margin, the squad and the hairline: 2 + 26 + 4.
        assert!(longest <= 32 + PROSE_MAX, "{longest}");
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
        assert!(shot.contains("○ frontend  sonnet-5-5"), "{shot}");
        assert!(shot.contains("● pm        1 memo"), "{shot}");
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
        assert!(shot.contains("● frontend  T-3"), "{shot}");
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
        assert!(shot.contains("T-3  Pricing section"), "{shot}");
        assert!(shot.contains("/accept T-3"), "{shot}");
        assert!(shot.lines().last().unwrap().contains("1 to review"), "{shot}");
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
        for expected in ["▎ frontend asks · T-3", "▎ run  npm publish", "▎ Release the package", "▎ y allow   a always allow   n decline"] {
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
    fn a_laid_out_conversation_is_reused_and_reports_how_much_it_grew() {
        let (mut app, id) = working();
        let mut layouts = Layouts::default();
        assert_eq!(layouts.prepare(&app, Theme::plain(), 100), None, "the first layout has nothing to compare with");
        assert_eq!(layouts.prepare(&app, Theme::plain(), 100), None, "nothing changed, nothing laid out again");
        app.on_event(Event::TextDelta { session: id, text: "\n\nmore\n\nand more".into() });
        assert_eq!(layouts.prepare(&app, Theme::plain(), 100), Some((id, 3)));
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
