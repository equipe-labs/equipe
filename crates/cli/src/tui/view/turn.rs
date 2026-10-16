//! A waiting ticket's card, open (`/answer`, r on the board), drawn where a
//! request for permission is — over the composer, under the accent bar: the
//! ticket, each ask on a line with what it has (words, or the pick, marked)
//! and its letters, its note under it, the words to the member last, and the
//! keys for the line marked.

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use equipe_core::protocol::asks::{Pick, Reply, Want, picks_for};
use equipe_core::t;

use super::super::app::{App, Typing};
use super::{Theme, cols};
use crate::ui;

/// What a pick is called, and its letter on a line wanting `want`.
fn pick_label(pick: Pick) -> (&'static str, String) {
    match pick {
        Pick::Done => ("d", t!("view.turn_done").into()),
        Pick::Yours => ("y", t!("view.turn_yours").into()),
        Pick::Later => ("l", t!("view.turn_later").into()),
        Pick::YourCall => ("y", t!("view.turn_your_call").into()),
        Pick::DontKnow => ("?", t!("view.turn_dont_know").into()),
    }
}

/// The open card, as lines over the composer.
pub(super) fn lines(app: &App, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let Some(form) = app.turn_form.as_ref() else { return Vec::new() };
    let Some(ticket) = app.tickets.iter().find(|t| t.id == form.ticket) else { return Vec::new() };
    let width = width.saturating_sub(2);
    let name = ticket.assignee.clone().or(ticket.owner.clone()).unwrap_or_default();
    let answers = form.answers();
    let mut out: Vec<Line<'static>> = Vec::new();
    let head = t!("view.your_turn_heading", id = ticket.id, title = ticket.title);
    out.push(Line::from(vec![Span::styled(ui::fit(&head, width.saturating_sub(cols(&name) + 3)), theme.accent().add_modifier(Modifier::BOLD)), Span::styled(format!(" — {name}"), theme.dim())]));
    let typing = form.typing;
    for (n, ask) in form.asks.iter().enumerate() {
        let here = form.at == n;
        let mark = if here { Span::styled("› ", theme.accent()) } else { Span::raw("  ") };
        let number = format!("{}. ", n + 1);
        let first = ask.lines().next().unwrap_or_default();
        let room = width.saturating_sub(2 + cols(&number));
        out.push(Line::from(vec![mark, Span::styled(number, theme.bold()), Span::styled(ui::fit(first, room), if here { theme.bold() } else { ratatui::style::Style::new() })]));
        // What it has: words (a question's), and the picks with their letters.
        let mut spans = vec![Span::raw("     ")];
        let reply = answers.replies.get(n).cloned().unwrap_or_default();
        if form.wants[n] != Want::Do {
            let words = form.typed[n].trim();
            let (said, style) = if typing == Some(Typing::Answer(n)) {
                (t!("view.turn_typing").into(), theme.accent())
            } else if words.is_empty() {
                (t!("view.turn_answer_empty").into(), theme.dim())
            } else {
                (words.replace('\n', " "), if matches!(reply, Reply::Text(_)) { theme.accent().add_modifier(Modifier::BOLD) } else { theme.dim() })
            };
            spans.push(Span::styled(format!("{} ", t!("view.turn_answer")), theme.dim()));
            spans.push(Span::styled(ui::fit(&said, (width / 2).max(12)), style));
            spans.push(Span::raw("   "));
        }
        for pick in picks_for(form.wants[n]) {
            let (letter, label) = pick_label(*pick);
            let chosen = form.picks.get(&n) == Some(pick);
            spans.push(Span::styled(format!("{letter} "), if here { theme.bold() } else { theme.dim() }));
            spans.push(Span::styled(if chosen { format!("● {label}") } else { label }, if chosen { theme.accent().add_modifier(Modifier::BOLD) } else { theme.dim() }));
            spans.push(Span::raw("  "));
        }
        out.push(Line::from(spans));
        let note = form.notes[n].trim();
        if typing == Some(Typing::Note(n)) || !note.is_empty() {
            let said = if typing == Some(Typing::Note(n)) { t!("view.turn_typing").into() } else { note.replace('\n', " ") };
            out.push(Line::from(vec![Span::raw("     "), Span::styled(ui::fit(&t!("view.turn_note", note = said), width.saturating_sub(5)), theme.dim())]));
        }
    }
    // The words to the member, last.
    let here = form.at == form.asks.len();
    let free = form.free.trim();
    let said = if typing == Some(Typing::Free) {
        Span::styled(t!("view.turn_typing").to_owned(), theme.accent())
    } else if free.is_empty() {
        Span::styled(t!("view.turn_free_empty", name = name).to_owned(), theme.dim())
    } else {
        Span::styled(ui::fit(&t!("view.turn_free", text = free.replace('\n', " ")), width.saturating_sub(2)), theme.accent().add_modifier(Modifier::BOLD))
    };
    out.push(Line::from(vec![if here { Span::styled("› ", theme.accent()) } else { Span::raw("  ") }, said]));
    // The keys for the line marked.
    let key = |k: &str, what: String| [Span::styled(k.to_owned(), theme.bold()), Span::styled(format!(" {what}   "), theme.dim())];
    let mut keys: Vec<Span<'static>> = Vec::new();
    if typing.is_some() {
        keys.extend(key("enter", t!("view.turn_keep").into()));
        keys.extend(key("esc", t!("view.turn_back").into()));
    } else {
        keys.extend(key("↑↓", t!("view.turn_move").into()));
        let words_here = here || form.wants.get(form.at).is_some_and(|w| *w != Want::Do);
        if words_here {
            keys.extend(key("a", t!("view.turn_write").into()));
        }
        if !here {
            keys.extend(key("n", t!("view.turn_note_key").into()));
        }
        keys.extend(key("s", t!("view.turn_send").into()));
        keys.extend(key("esc", t!("view.turn_close").into()));
        let left = form.left();
        if left > 0 && !answers.is_empty() {
            keys.push(Span::styled(t!("view.turn_left", count = left).to_owned(), theme.dim()));
        }
    }
    out.push(Line::from(keys));
    let bar = || Span::styled("▎ ", theme.accent());
    out.into_iter()
        .map(|line| {
            let mut spans = vec![bar()];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::{Layouts, render};
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent};
    use equipe_core::protocol::{Access, Engine, Event, Member, SessionInfo, SessionState, Snapshot, Ticket, TicketStatus};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::Instant;

    fn screen(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render(f, app, Theme::plain(), &Layouts::default())).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
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

    fn app() -> App {
        let info = SessionInfo {
            session: 1,
            project: "/repo".into(),
            member: Some(Member { role: "pm".into(), name: "pm".into() }),
            engine: Engine::Claude,
            model: None,
            access: Access::ReadOnly,
            workspace: None,
            state: SessionState::Ready,
            title: None,
            tokens: 0,
            context: None,
            recent: Vec::new(),
            queued: Vec::new(),
            queued_project: None,
            queued_in: Vec::new(),
            steering: Vec::new(),
            estimate: None,
            lane: None,
            work: None,
            mode: None,
            effort: None,
            subagents: Vec::new(),
        };
        let mut app = App::from_snapshot("/repo".into(), Snapshot { squad: "acme".into(), projects: vec!["/repo".into()], sessions: vec![info], ..Snapshot::default() }, &[]).0;
        let ticket: Ticket = serde_json::from_value(serde_json::json!({
            "id": 117, "title": "앱스토어 제출", "body": "", "status": "waiting", "owner": "ios", "assignee": "ios", "project": null,
            "depends_on": [], "branch": null, "created_by": "pm", "note": null,
            "asks": ["App Review 연락처를 알려 주세요: 전화번호와 이메일.", "App Store Connect에서 EU DSA 상태를 trader로 바꿔 주세요."],
        }))
        .unwrap();
        assert_eq!(ticket.status, TicketStatus::Waiting);
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket });
        app
    }

    fn press(app: &mut App, c: char) {
        app.on_key(KeyEvent::from(KeyCode::Char(c)), Instant::now());
    }

    #[test]
    fn the_card_shows_each_ask_its_letters_and_what_was_picked() {
        let mut app = app();
        for c in "/answer T-117".chars() {
            press(&mut app, c);
        }
        app.on_key(KeyEvent::from(KeyCode::Enter), Instant::now());
        app.on_key(KeyEvent::from(KeyCode::Down), Instant::now());
        press(&mut app, 'y');
        let shown = screen(&app, 110, 40);
        assert!(shown.contains("1. App Review 연락처를 알려 주세요"), "{shown}");
        assert!(shown.contains("y You decide") && shown.contains("? I don't know"), "a question's letters: {shown}");
        assert!(shown.contains("› 2. App Store Connect"), "the line marked: {shown}");
        assert!(shown.contains("d Done") && shown.contains("● Do it for me") && shown.contains("l Later"), "a thing to do's, one picked: {shown}");
        assert!(shown.contains("Anything else for ios"), "{shown}");
        assert!(shown.contains("s send") && shown.contains("1 not answered stays open"), "{shown}");
    }
}
