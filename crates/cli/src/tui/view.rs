//! Drawing the app — nothing here decides anything.
//!
//! Styled per docs/BRAND.md like the rest of the CLI: monochrome, one cool
//! accent for what is alive or succeeded, weight and dimming for hierarchy.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use equipe_core::protocol::Entry;

use super::app::{Agent, AgentState, App, relative, tool_verb};
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
}

/// The height the composer wants: its lines, capped, plus its border.
fn composer_height(app: &App, width: u16) -> u16 {
    let inner = width.saturating_sub(4) as usize;
    let (rows, _) = app.composer.layout(inner);
    rows.len().clamp(1, 8) as u16 + 2
}

pub fn render(frame: &mut Frame, app: &App, theme: Theme, branch: Option<&str>) {
    let area = frame.area();
    let [header, body, composer, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(composer_height(app, area.width)),
        Constraint::Length(1),
    ])
    .areas(area);
    let team_width = if area.width >= 72 { 28 } else { 0 };
    let [team, transcript] = Layout::horizontal([Constraint::Length(team_width), Constraint::Min(20)]).areas(body);

    render_header(frame, header, app, theme, branch);
    if team_width > 0 {
        render_team(frame, team, app, theme);
    }
    render_transcript(frame, transcript, app, theme);
    render_composer(frame, composer, app, theme);
    render_status(frame, status, app, theme);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App, theme: Theme, branch: Option<&str>) {
    let mut place = display_path(&app.cwd);
    if let Some(branch) = branch {
        place.push_str(" · ");
        place.push_str(branch);
    }
    // Keep the end of a long path — the project and branch — and drop the start.
    let room = (area.width as usize).saturating_sub(10);
    let count = place.chars().count();
    if count > room && room > 1 {
        place = format!("…{}", place.chars().skip(count - room + 1).collect::<String>());
    }
    let line = Line::from(vec![
        Span::styled("equipe", theme.bold()),
        Span::styled(".", theme.dim()),
        Span::raw("  "),
        Span::styled(place, theme.dim()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn state_label(agent: &Agent, theme: Theme) -> Span<'static> {
    match agent.state {
        AgentState::Starting => Span::styled("… starting", theme.dim()),
        AgentState::Ready => Span::styled("○ ready", theme.dim()),
        AgentState::Working => Span::styled("● working", theme.accent()),
        AgentState::Closing => Span::styled("… closing", theme.dim()),
        AgentState::Closed => Span::styled("closed", theme.dim()),
        AgentState::Failed => Span::styled("✗ failed", theme.bold()),
    }
}

fn render_team(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let mut lines = Vec::new();
    for (i, agent) in app.agents.iter().enumerate() {
        let selected = i == app.selected;
        let marker = if selected { Span::styled("› ", theme.accent()) } else { Span::raw("  ") };
        let name_style = if selected { theme.bold() } else { Style::new() };
        // Members go by name; a plain agent by its place and engine.
        let name = if agent.member.is_some() { agent.label() } else { format!("{} {}", i + 1, agent.label()) };
        let mut first = vec![marker, Span::styled(name, name_style)];
        if !agent.queued.is_empty() {
            let memos = if agent.queued.len() == 1 { "1 memo".to_owned() } else { format!("{} memos", agent.queued.len()) };
            first.push(Span::styled(format!(" · {memos}"), theme.dim()));
        }
        lines.push(Line::from(first));
        let detail = match (&agent.member, &agent.branch) {
            (None, Some(branch)) => Some(branch.strip_prefix("equipe/").unwrap_or(branch).to_owned()),
            _ => agent.short_model(),
        }
        .unwrap_or_default();
        let mut second = vec![Span::raw("    "), state_label(agent, theme)];
        if !detail.is_empty() {
            second.push(Span::styled(format!("  {detail}"), theme.dim()));
        }
        lines.push(Line::from(second));
    }
    let block = Block::bordered().border_style(theme.border()).title(Span::styled(" team ", theme.dim()));
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
            "Tell the PM what you want to build — it plans the work and splits it into tickets. \
/add designer frontend brings in teammates; Tab or @name talks to one directly, and the PM hears about it."
        } else if agent.edit {
            "Tell it what to change. It works on its own branch, in a worktree of its own; /close lands the branch."
        } else if agent.member.is_some() {
            "Talk to it directly — the PM hears about it."
        } else {
            "Ask anything. /add brings in teammates. /help lists the rest."
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
            Entry::Reply { text } => lines.extend(wrapped(Vec::new(), text, Style::new(), 0, width)),
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
            Entry::Note { text } => lines.extend(wrapped(Vec::new(), text, theme.dim(), 0, width)),
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
                for (key, command) in [("review", "git show"), ("keep", "git merge"), ("discard", "git branch -D")] {
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {key:<9}"), theme.dim()),
                        Span::styled(format!("{command} {branch}"), theme.bold()),
                    ]));
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

fn render_transcript(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let Some(agent) = app.selected() else {
        return;
    };
    let title = match &agent.member {
        Some(_) => format!(" {} · {} ", agent.label(), agent.engine_label()),
        None => format!(" {} ", agent.label()),
    };
    let block = Block::bordered().border_style(theme.border()).title(Span::styled(title, theme.dim()));
    let inner = block.inner(area);
    let lines = transcript_lines(agent, theme, inner.width.saturating_sub(1) as usize);
    // Follow the bottom unless the person scrolled up.
    let height = inner.height as usize;
    let bottom = lines.len().saturating_sub(height);
    let top = bottom.saturating_sub(agent.scroll_back.min(bottom));
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
    let placeholder = app.composer.text().is_empty();
    let content = if placeholder {
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
            spans.push(Span::raw("  "));
            spans.push(state_label(agent, theme));
            spans
        }
        _ => Vec::new(),
    };
    let tokens = app.total_tokens();
    let mut facts = Vec::new();
    // A multi-line draft (often a paste) says how big it is before it's sent.
    let draft_lines = app.composer.line_count();
    if draft_lines > 1 {
        facts.push(format!("{draft_lines} lines"));
    }
    if tokens > 0 {
        facts.push(format!("{} tokens", group_digits(tokens)));
    }
    let left_width: usize = left.iter().map(|s| s.width()).sum();
    // When space runs short, the key hints go first; the facts stay.
    let hints = "Enter send · Esc stop · Tab agents · /help";
    let with_hints = facts.iter().map(String::as_str).chain(std::iter::once(hints)).collect::<Vec<_>>().join(" · ");
    let right = if left_width + with_hints.chars().count() + 2 <= area.width as usize { with_hints } else { facts.join(" · ") };
    let gap = (area.width as usize).saturating_sub(left_width + right.chars().count());
    let mut spans = left;
    if gap >= 2 && !right.is_empty() {
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(right, theme.dim()));
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
    use equipe_core::protocol::{Command, Engine, Event, Member, Snapshot, TurnOutcome, Usage};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render(f, app, Theme::plain(), Some("main"))).unwrap();
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

    /// The first agent's id, and an app where it is mid-turn.
    fn working() -> (App, u64) {
        let (mut app, commands) = App::from_snapshot("/repo".into(), "/repo".into(), Snapshot::default());
        let Some(Command::Start { session: id, .. }) = commands.first() else { panic!() };
        let id = *id;
        app.on_event(Event::SessionStarted {
            session: id,
            project: "/repo".into(),
            engine: Engine::Claude,
            model: Some("haiku".into()),
            workspace: None,
            member: Some(Member { role: "pm".into(), name: "pm".into() }),
        });
        app.on_event(Event::UserMessage { session: id, text: "fix the bug".into() });
        app.on_event(Event::TextDelta { session: id, text: "Looking now.".into() });
        app.on_event(Event::ToolUse { session: id, tool: "Edit".into(), target: Some("src/auth.rs".into()) });
        (app, id)
    }

    fn working_app() -> App {
        working().0
    }

    #[test]
    fn shows_the_team_the_conversation_and_what_the_agent_does() {
        let shot = screen(&working_app(), 90, 20);
        for expected in ["equipe.", "/repo · main", " team ", "› pm", "● working", " pm · claude code ", "› fix the bug", "Looking now.", "✎ edit   src/auth.rs", " → pm ", "Esc stop"] {
            assert!(shot.contains(expected), "missing {expected:?} in:\n{shot}");
        }
    }

    #[test]
    fn a_finished_turn_shows_tokens_and_a_ready_agent() {
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
        let shot = screen(&working_app(), 50, 16);
        assert!(!shot.contains(" team "), "{shot}");
        assert!(shot.contains("Looking now."), "{shot}");
    }

    #[test]
    fn a_failed_start_shows_the_fix() {
        let (mut app, commands) = App::from_snapshot("/repo".into(), "/repo".into(), Snapshot::default());
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
        app.on_event(Event::TextDelta { session: id, text: "\n에이전트가 파일을 고쳤어요 이제 리뷰해 주세요 브랜치에 커밋했어요".into() });
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
        });
        app.on_event(Event::MessageQueued { session: pm, text: "and the footer".into() });
        let shot = screen(&app, 100, 24);
        assert!(shot.contains("○ ready  sonnet-5-5"), "{shot}");
        assert!(shot.contains("› pm · 1 memo"), "{shot}");
        assert!(shot.contains("› and the footer"), "{shot}");
        assert!(shot.contains("memo · read when this turn ends"), "{shot}");

        app.selected = 1;
        let shot = screen(&app, 100, 24);
        assert!(shot.contains(" → frontend · the PM hears about it "), "{shot}");
    }

    #[test]
    fn group_digits_reads_like_a_number() {
        assert_eq!(group_digits(1234567), "1,234,567");
        assert_eq!(group_digits(999), "999");
    }
}
