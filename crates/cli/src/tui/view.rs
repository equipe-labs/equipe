//! Drawing the app — nothing here decides anything.
//!
//! Styled per docs/BRAND.md like the rest of the CLI: monochrome, one cool
//! accent for what is alive or succeeded, weight and dimming for hierarchy.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use std::collections::HashMap;

use equipe_core::board::project_name;
use equipe_core::protocol::{Entry, SessionId, Ticket, TicketStatus};

use super::app::{Agent, AgentState, App, Pane, relative, tool_verb};
use super::markdown;
use crate::ui;

/// Styles, honouring NO_COLOR and terminals without truecolor.
#[derive(Clone, Copy)]
pub struct Theme {
    color: bool,
    truecolor: bool,
}

impl Theme {
    pub fn detect() -> Self {
        let (color, truecolor) = ui::out().palette();
        Self { color, truecolor }
    }

    #[cfg(test)]
    pub fn plain() -> Self {
        Self { color: false, truecolor: false }
    }

    fn accent(self) -> Style {
        match (self.color, self.truecolor) {
            (true, true) => Style::new().fg(Color::Rgb(180, 205, 230)),
            (true, false) => Style::new().fg(Color::Indexed(152)),
            _ => Style::new().add_modifier(Modifier::BOLD),
        }
    }

    fn dim(self) -> Style {
        match (self.color, self.truecolor) {
            (true, true) => Style::new().fg(Color::Rgb(124, 126, 133)),
            (true, false) => Style::new().add_modifier(Modifier::DIM),
            _ => Style::new(),
        }
    }

    fn bold(self) -> Style {
        Style::new().add_modifier(Modifier::BOLD)
    }

    fn border(self) -> Style {
        self.dim()
    }

    /// Code in a reply: the accent, so it stands apart from prose without a box.
    fn code(self) -> Style {
        if self.color { self.accent() } else { Style::new() }
    }

    fn markdown(self) -> markdown::Styles {
        markdown::Styles { plain: Style::new(), code: self.code(), dim: self.dim() }
    }
}

/// The height the composer wants: its lines, capped, plus its border.
fn composer_height(app: &App, width: u16) -> u16 {
    let inner = width.saturating_sub(4) as usize;
    let (rows, _) = app.composer.layout(inner);
    rows.len().clamp(1, 8) as u16 + 2
}

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

/// The width a conversation is laid out at, in a terminal `total` columns wide:
/// what the squad pane and the borders leave.
fn transcript_width(total: u16) -> usize {
    let team = if total >= 72 { 28 } else { 0 };
    (total.saturating_sub(team) as usize).saturating_sub(3)
}

pub fn render(frame: &mut Frame, app: &App, theme: Theme, branch: Option<&str>, layouts: &Layouts) {
    let area = frame.area();
    let asking = approval_lines(app, theme, area.width.saturating_sub(4) as usize);
    let asking_height = if asking.is_empty() { 0 } else { asking.len() as u16 + 2 };
    let [header, body, approval, composer, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(asking_height),
        Constraint::Length(composer_height(app, area.width)),
        Constraint::Length(1),
    ])
    .areas(area);
    let team_width = if area.width >= 72 { 28 } else { 0 };
    let [team, right] = Layout::horizontal([Constraint::Length(team_width), Constraint::Min(20)]).areas(body);

    render_header(frame, header, app, theme, branch);
    if team_width > 0 {
        render_team(frame, team, app, theme);
    }
    match app.pane {
        Pane::Conversation => render_transcript(frame, right, app, theme, layouts),
        Pane::Board => render_board(frame, right, app, theme),
    }
    if asking_height > 0 {
        render_approval(frame, approval, app, asking, theme);
    }
    render_composer(frame, composer, app, theme);
    render_status(frame, status, app, theme);
}

/// The oldest request for permission, in a fixed place above the composer —
/// what exactly, who, why, and the keys that answer it.
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
    // A few lines of the command, never the whole script: the box must leave the
    // conversation room, and its keys must always show.
    const SHOWN: usize = 3;
    let rows: Vec<&str> = approval.action.lines().collect();
    let mut lines = Vec::new();
    for (i, row) in rows.iter().take(SHOWN).enumerate() {
        let lead = if i == 0 { format!("{verb}  ") } else { " ".repeat(verb.chars().count() + 2) };
        let room = width.saturating_sub(lead.chars().count());
        lines.push(Line::from(vec![Span::styled(lead, theme.dim()), Span::styled(ui::fit(row, room), theme.bold())]));
    }
    let writes = files_written(&approval.action);
    if !writes.is_empty() {
        lines.push(Line::from(Span::styled(ui::fit(&format!("writes {}", writes.join(", ")), width), theme.dim())));
    }
    if rows.len() > SHOWN {
        lines.push(Line::from(Span::styled(format!("… {} more lines", rows.len() - SHOWN), theme.dim())));
    }
    if let Some(reason) = &approval.reason {
        lines.push(Line::from(Span::styled(ui::fit(reason, width), theme.dim())));
    }
    let mut keys = vec![
        Span::styled("y", theme.bold()),
        Span::styled(" allow  ", theme.dim()),
        Span::styled("a", theme.bold()),
        Span::styled(" always allow  ", theme.dim()),
        Span::styled("n", theme.bold()),
        Span::styled(" decline", theme.dim()),
    ];
    if app.approvals.len() > 1 {
        keys.push(Span::styled(format!("  · {} more waiting", app.approvals.len() - 1), theme.dim()));
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

fn render_approval(frame: &mut Frame, area: Rect, app: &App, lines: Vec<Line<'static>>, theme: Theme) {
    let Some(approval) = app.approvals.first() else {
        return;
    };
    let ticket = approval.ticket.map(|t| format!(" · T-{t}")).unwrap_or_default();
    let title = Line::from(vec![
        Span::styled(" ", theme.accent()),
        Span::styled(approval.member.clone(), theme.bold()),
        Span::styled(format!(" asks{ticket} ", ), theme.accent()),
    ]);
    let block = Block::bordered().border_style(theme.accent()).title(title);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App, theme: Theme, branch: Option<&str>) {
    let mut place = display_path(&app.cwd);
    if let Some(branch) = branch {
        place.push_str(" · ");
        place.push_str(branch);
    }
    // Keep the end of a long path — the project and branch — and drop the start.
    let room = (area.width as usize).saturating_sub(12 + app.squad.chars().count());
    let count = place.chars().count();
    if count > room && room > 1 {
        place = format!("…{}", place.chars().skip(count - room + 1).collect::<String>());
    }
    let line = Line::from(vec![
        Span::styled("equipe", theme.bold()),
        Span::styled(".", theme.dim()),
        Span::raw("  "),
        Span::styled(app.squad.clone(), theme.bold()),
        Span::styled(format!("  {place}"), theme.dim()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn state_label(agent: &Agent, theme: Theme) -> Span<'static> {
    match agent.state {
        AgentState::Starting => Span::styled("… starting", theme.dim()),
        AgentState::Ready => Span::styled("○ ready", theme.dim()),
        AgentState::Working => Span::styled("● working", theme.accent()),
        AgentState::Closing => Span::styled("… leaving", theme.dim()),
        AgentState::Closed => Span::styled("left", theme.dim()),
        AgentState::Failed => Span::styled("✗ failed", theme.bold()),
    }
}

fn render_team(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let mut lines = Vec::new();
    for (i, agent) in app.agents.iter().enumerate() {
        let selected = i == app.selected && app.pane == Pane::Conversation;
        let marker = if selected { Span::styled("› ", theme.accent()) } else { Span::raw("  ") };
        let name_style = if selected { theme.bold() } else { Style::new() };
        let mut first = vec![marker, Span::styled(agent.label(), name_style)];
        if !agent.queued.is_empty() {
            let memos = if agent.queued.len() == 1 { "1 memo".to_owned() } else { format!("{} memos", agent.queued.len()) };
            first.push(Span::styled(format!(" · {memos}"), theme.dim()));
        }
        lines.push(Line::from(first));
        // What it is doing: its ticket when it has one, else its model.
        let detail = match agent.ticket {
            Some(ticket) => format!("T-{ticket}"),
            None => agent.short_model().unwrap_or_default(),
        };
        let mut second = vec![Span::raw("    "), state_label(agent, theme)];
        if !detail.is_empty() {
            second.push(Span::styled(format!("  {detail}"), theme.dim()));
        }
        lines.push(Line::from(second));
    }
    let block = Block::bordered().border_style(theme.border()).title(Span::styled(" squad ", theme.dim()));
    frame.render_widget(Paragraph::new(lines).block(block), area);
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
    for entry in agent.transcript.entries() {
        match entry {
            Entry::User { text } => {
                if !lines.is_empty() {
                    lines.push(Line::raw(""));
                }
                lines.extend(wrapped(vec![Span::styled("› ", theme.accent())], text, theme.bold(), 2, width));
            }
            Entry::Reply { text } => lines.extend(markdown::lines(text, width, &theme.markdown())),
            // Claude Code loading tool descriptions: its business, not the person's.
            Entry::Tool { tool, .. } if tool == "ToolSearch" => {}
            // A member writing down what it learned, in its own memory.
            Entry::Tool { tool, target: Some(target) } if is_memory(tool, target) => {
                let note = std::path::Path::new(target).file_name().map_or_else(|| target.clone(), |n| n.to_string_lossy().into_owned());
                lines.extend(wrapped(vec![Span::styled("· ", theme.dim())], &format!("{:<6} {note}", "recall"), theme.dim(), 2, width));
            }
            Entry::Tool { tool, target } => {
                let verb = tool_verb(tool);
                let target = target.as_deref().map(|t| relative(t, &agent.base)).unwrap_or_default();
                let mark = if matches!(verb, "edit" | "write") {
                    Span::styled("✎ ", theme.accent())
                } else {
                    Span::styled("· ", theme.dim())
                };
                lines.extend(wrapped(vec![mark], &format!("{verb:<6} {target}"), theme.dim(), 2, width));
            }
            Entry::Note { text } => {
                // A ticket starting is a chapter in the member's day.
                if text.starts_with("── ") && !lines.is_empty() {
                    lines.push(Line::raw(""));
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
        }
    }
    // Memos wait below the conversation until the turn ends.
    for memo in &agent.queued {
        lines.push(Line::raw(""));
        lines.extend(wrapped(vec![Span::styled("› ", theme.dim())], memo, theme.dim(), 2, width));
        lines.push(Line::from(Span::styled("  memo · read when this turn ends · Esc to read now", theme.dim())));
    }
    lines
}

/// Whether a tool use is a member writing to its own memory — Claude Code's
/// auto memory, which equipe keeps under the squad's members.
fn is_memory(tool: &str, target: &str) -> bool {
    matches!(tool, "Write" | "Edit" | "MultiEdit") && target.contains("/.equipe/") && target.contains("/members/") && target.contains("/memory/")
}

fn render_transcript(frame: &mut Frame, area: Rect, app: &App, theme: Theme, layouts: &Layouts) {
    let Some(agent) = app.selected() else {
        return;
    };
    let mut title = format!(" {} · {} ", agent.label(), agent.engine_label());
    if let Some(ticket) = agent.ticket.and_then(|id| app.tickets.iter().find(|t| t.id == id)) {
        title = format!(" {} · T-{} {} ", agent.label(), ticket.id, ticket.title);
    }
    let block = Block::bordered().border_style(theme.border()).title(Span::styled(title, theme.dim()));
    let inner = block.inner(area);
    let width = inner.width.saturating_sub(1) as usize;
    let lines = match layouts.get(agent, width) {
        Some(lines) => lines.to_vec(),
        None => transcript_lines(agent, theme, width),
    };
    // Follow the bottom unless the person scrolled up.
    let height = inner.height as usize;
    let bottom = lines.len().saturating_sub(height);
    let top = bottom.saturating_sub(agent.scroll_back.min(bottom));
    frame.render_widget(Paragraph::new(lines).block(block).scroll((top as u16, 0)), area);
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
        for t in tickets {
            let id = Span::styled(format!("T-{:<3}", t.id), if t.status == TicketStatus::Review { theme.bold() } else { theme.dim() });
            let title_style = if t.status.is_closed() { theme.dim() } else { Style::new() };
            lines.extend(wrapped(vec![Span::raw("  "), id, Span::raw(" ")], &t.title, title_style, 9, width));
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
                lines.extend(wrapped(vec![Span::raw(" ".repeat(9))], &facts.join(" · "), theme.dim(), 9, width));
            }
            if let Some(note) = &t.note
                && !t.status.is_closed()
            {
                lines.extend(wrapped(vec![Span::raw(" ".repeat(9))], note, theme.dim(), 9, width));
            }
            if t.status == TicketStatus::Review {
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(9)),
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
    let block = Block::bordered()
        .border_style(theme.border())
        .title(Span::styled(" board · Ctrl-B or Esc to go back ", theme.dim()));
    let inner = block.inner(area);
    let lines = board_lines(app, theme, inner.width.saturating_sub(1) as usize);
    let top = app.board_scroll.min(lines.len().saturating_sub(1));
    frame.render_widget(Paragraph::new(lines).block(block).scroll((top as u16, 0)), area);
}

fn render_composer(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    // Always say who a message goes to, so nothing reaches the wrong person.
    let mut block = Block::bordered().border_style(theme.border());
    if let Some((name, direct)) = app.addressee() {
        let mut title = vec![Span::styled(" → ", theme.dim()), Span::styled(name, theme.bold())];
        if direct {
            title.push(Span::styled(" · the PM hears about it", theme.dim()));
        }
        title.push(Span::raw(" "));
        block = block.title(Line::from(title));
    }
    let inner = block.inner(area);
    let text_width = inner.width.saturating_sub(2) as usize;
    let (rows, (cursor_row, cursor_col)) = app.composer.layout(text_width);
    // Keep the cursor's row in view when the message is taller than the box.
    let first = (cursor_row + 1).saturating_sub(inner.height as usize);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(first)
        .map(|(i, row)| {
            let lead = if i == 0 { Span::styled("› ", theme.accent()) } else { Span::raw("  ") };
            Line::from(vec![lead, Span::raw(row.clone())])
        })
        .collect();
    let content = if app.composer.text().is_empty() {
        Paragraph::new(Line::from(vec![
            Span::styled("› ", theme.accent()),
            Span::styled("Message · @name for someone else · /help", theme.dim()),
        ]))
    } else {
        Paragraph::new(lines)
    };
    frame.render_widget(content.block(block), area);
    frame.set_cursor_position((inner.x + 2 + cursor_col as u16, inner.y + (cursor_row - first) as u16));
}

fn render_status(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let left: Vec<Span> = match (&app.flash, app.selected()) {
        (Some(flash), _) => vec![Span::styled(flash.clone(), theme.bold())],
        (None, Some(agent)) => {
            let mut spans = vec![Span::styled(agent.label(), theme.dim())];
            if let Some(model) = agent.short_model() {
                spans.push(Span::styled(format!(" · {model}"), theme.dim()));
            }
            if agent.member.is_some() {
                spans.push(Span::styled(format!(" · {}", app.mode_of(agent).label()), theme.dim()));
            }
            spans.push(Span::raw("  "));
            spans.push(state_label(agent, theme));
            spans
        }
        _ => Vec::new(),
    };
    let mut facts: Vec<(String, Style)> = Vec::new();
    // Work waiting for the person comes first: it's what only they can do.
    let review = app.to_review().len();
    if review > 0 {
        facts.push((format!("{review} to review · Ctrl-B"), theme.accent()));
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
    let hints = ("Enter send · Esc stop · Tab squad · /help".to_owned(), theme.dim());
    let width = |items: &[(String, Style)]| items.iter().map(|(t, _)| t.chars().count()).sum::<usize>() + 3 * items.len().saturating_sub(1);
    let mut right = facts.clone();
    right.push(hints);
    if left_width + width(&right) + 2 > area.width as usize {
        right = facts;
    }
    let gap = (area.width as usize).saturating_sub(left_width + width(&right));
    let mut spans = left;
    if gap >= 2 && !right.is_empty() {
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

    #[test]
    fn shows_the_squad_the_conversation_and_what_the_member_does() {
        let shot = screen(&working().0, 90, 20);
        for expected in ["equipe.", "acme", "/repo · main", " squad ", "› pm", "● working", " pm · claude code ", "› fix the bug", "Looking now.", "✎ edit   src/auth.rs", " → pm ", "Esc stop"] {
            assert!(shot.contains(expected), "missing {expected:?} in:\n{shot}");
        }
        assert!(!shot.contains("**"), "Markdown is drawn, not shown:\n{shot}");
    }

    #[test]
    fn a_finished_turn_shows_tokens_and_a_ready_member() {
        let (mut app, id) = working();
        app.on_event(Event::TurnEnded {
            session: id,
            outcome: TurnOutcome::Completed,
            usage: Some(Usage { input_tokens: Some(1200), output_tokens: Some(34) }),
        });
        let shot = screen(&app, 90, 20);
        assert!(shot.contains("1,234 tokens"), "{shot}");
        assert!(shot.contains("○ ready"), "{shot}");
    }

    #[test]
    fn a_narrow_terminal_drops_the_team_pane_not_the_conversation() {
        let shot = screen(&working().0, 50, 16);
        assert!(!shot.contains(" squad "), "{shot}");
        assert!(shot.contains("Looking now."), "{shot}");
    }

    #[test]
    fn a_failed_start_shows_the_fix() {
        let (mut app, commands) = App::from_snapshot("/repo".into(), empty(), &[]);
        let Some(Command::Start { session: id, .. }) = commands.first() else { panic!() };
        app.on_event(Event::Error { session: Some(*id), message: "No engine is ready.".into(), next: Some("equipe login".into()) });
        let shot = screen(&app, 90, 16);
        assert!(shot.contains("✗ No engine is ready."), "{shot}");
        assert!(shot.contains("→ equipe login"), "{shot}");
        assert!(shot.contains("✗ failed"), "{shot}");
    }

    #[test]
    fn hangul_replies_wrap_inside_the_pane() {
        let (mut app, id) = working();
        app.on_event(Event::TextDelta { session: id, text: "\n\n에이전트가 파일을 고쳤어요 이제 리뷰해 주세요 브랜치에 커밋했어요".into() });
        let shot = screen(&app, 72, 20);
        for line in shot.lines() {
            assert!(unicode_width::UnicodeWidthStr::width(line) <= 72, "overflow: {line:?}");
        }
        assert!(shot.contains("에이전트가"), "{shot}");
    }

    #[test]
    fn a_multi_line_paste_shows_its_size_before_sending() {
        let (mut app, id) = working();
        app.on_event(Event::TurnEnded { session: id, outcome: TurnOutcome::Completed, usage: None });
        app.on_paste("line one\nline two\nline three");
        let shot = screen(&app, 90, 20);
        assert!(shot.contains("3 lines"), "{shot}");
        assert!(shot.contains("line three"), "{shot}");
    }

    #[test]
    fn a_narrow_status_bar_drops_the_hints_and_keeps_the_facts() {
        let (mut app, id) = working();
        app.on_event(Event::TurnEnded {
            session: id,
            outcome: TurnOutcome::Completed,
            usage: Some(Usage { input_tokens: Some(100), output_tokens: Some(20) }),
        });
        app.on_paste("one\ntwo\nthree");
        let shot = screen(&app, 60, 20);
        let status = shot.lines().last().unwrap();
        assert!(status.contains("3 lines") && status.contains("120 tokens"), "{status:?}");
        assert!(!status.contains("Enter send"), "{status:?}");
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
        assert!(shot.contains("○ ready  sonnet-5-5"), "{shot}");
        assert!(shot.contains("› pm · 1 memo"), "{shot}");
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
        assert!(shot.contains(" → frontend · the PM hears about it "), "{shot}");
        assert!(shot.contains("● working  T-3"), "{shot}");
        assert!(shot.contains("── T-3 · Hero — on branch equipe/frontend-t3"), "{shot}");
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
        let shot = screen(&app, 100, 30);
        let at = |text: &str| shot.find(text).unwrap_or_else(|| panic!("missing {text:?} in:\n{shot}"));
        assert!(at("YOUR REVIEW  1") < at("IN PROGRESS  1"), "{shot}");
        assert!(at("IN PROGRESS") < at("TO DO") && at("TO DO") < at("DONE"), "{shot}");
        assert!(shot.contains("T-3   Pricing section"), "{shot}");
        assert!(shot.contains("/accept T-3"), "{shot}");
        assert!(shot.lines().last().unwrap().contains("1 to review"), "{shot}");
    }

    #[test]
    fn an_empty_board_says_how_work_gets_there() {
        let (mut app, _) = working();
        app.pane = Pane::Board;
        assert!(screen(&app, 90, 16).contains("The board is empty."));
    }

    #[test]
    fn a_request_for_permission_sits_above_the_composer_with_its_keys() {
        let (mut app, _) = working();
        app.on_event(Event::ApprovalRequested {
            squad: "acme".into(),
            approval: equipe_core::protocol::Approval {
                id: 1,
                session: 2,
                member: "frontend".into(),
                ticket: Some(3),
                tool: "Bash".into(),
                action: "npm publish".into(),
                reason: Some("Release the package".into()),
            },
        });
        let shot = screen(&app, 100, 24);
        for expected in ["frontend asks · T-3", "run  npm publish", "Release the package", "y allow  a always allow  n decline"] {
            assert!(shot.contains(expected), "missing {expected:?} in:\n{shot}");
        }
        assert!(shot.lines().last().unwrap().contains("pm · haiku · auto"), "{shot}");
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
        let shot = screen(&app, 100, 20);
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
    fn a_long_script_asks_in_a_few_lines_and_says_what_it_writes() {
        let (mut app, _) = working();
        let script = "mkdir -p src/pages\ncat > astro.config.mjs <<'EOF'\nexport default {}\nEOF\ncat > tsconfig.json <<'EOF'\n{}\nEOF\necho done";
        app.on_event(Event::ApprovalRequested {
            squad: "acme".into(),
            approval: equipe_core::protocol::Approval {
                id: 1,
                session: 2,
                member: "frontend".into(),
                ticket: Some(2),
                tool: "Bash".into(),
                action: script.into(),
                reason: None,
            },
        });
        let shot = screen(&app, 100, 30);
        assert!(shot.contains("writes astro.config.mjs, tsconfig.json"), "{shot}");
        assert!(shot.contains("… 5 more lines"), "{shot}");
        assert!(shot.contains("y allow"), "the keys always show:\n{shot}");
        assert!(!shot.contains("echo done"), "{shot}");
    }

    #[test]
    fn group_digits_reads_like_a_number() {
        assert_eq!(group_digits(1234567), "1,234,567");
        assert_eq!(group_digits(999), "999");
    }
}
