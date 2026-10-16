//! A member's questions, drawn where a request for permission is — over the
//! composer, under the accent bar: who asks, the question and where the
//! person is (1/2), the options numbered (picked ones marked), `o` for their
//! own words, and the keys. Answered, the message the answers went in reads
//! as one line in the conversation: "✓ 결정함 · 2개 — …".

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use equipe_core::protocol::decide;
use equipe_core::t;

use super::super::app::{App, Pane, Picking};
use super::{Theme, cols, wrapped};
use crate::ui;

/// The set the conversation on screen shows, as lines over the composer.
pub(super) fn lines(app: &App, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let Some(set) = app.shown_questions().filter(|_| app.pane == Pane::Conversation) else {
        return Vec::new();
    };
    let picking = app.picking_of(set.id).cloned().unwrap_or_else(|| Picking::new(set));
    let step = picking.step.min(set.questions.len().saturating_sub(1));
    let Some(q) = set.questions.get(step) else { return Vec::new() };
    let width = width.saturating_sub(2);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut head = vec![Span::styled(set.member.clone(), theme.bold()), Span::styled(format!(" {}", t!("view.decide_asks")), theme.accent())];
    head.push(Span::styled(format!(" · {}/{}", step + 1, set.questions.len()), theme.dim()));
    if !q.header.is_empty() {
        head.push(Span::styled(format!(" · {}", q.header), theme.dim()));
    }
    if let Some(ticket) = set.ticket {
        head.push(Span::styled(format!(" · T-{ticket}"), theme.dim()));
    }
    out.push(Line::from(head));
    out.extend(wrapped(Vec::new(), &q.question, theme.bold(), 0, width));
    if q.multi_select {
        out.push(Line::from(Span::styled(t!("view.decide_multi").to_owned(), theme.dim())));
    }
    for (n, option) in q.options.iter().enumerate() {
        let picked = picking.picks.get(step).is_some_and(|p| p.contains(&n));
        let mark = match (q.multi_select, picked) {
            (true, true) => "[x] ",
            (true, false) => "[ ] ",
            (false, true) => "● ",
            (false, false) => "○ ",
        };
        let number = format!("{} ", n + 1);
        let label_style = if picked { theme.accent().add_modifier(Modifier::BOLD) } else { theme.bold() };
        let used = cols(&number) + cols(mark) + cols(&option.label);
        let mut spans = vec![Span::styled(number, theme.bold()), Span::styled(mark, if picked { theme.accent() } else { theme.dim() }), Span::styled(ui::fit(&option.label, width.saturating_sub(4)), label_style)];
        if !option.description.is_empty() && used + 4 < width {
            spans.push(Span::styled(format!(" — {}", ui::fit(&option.description, width - used - 3)), theme.dim()));
        }
        out.push(Line::from(spans));
    }
    let own = picking.others.get(step).map(|o| o.trim().to_owned()).unwrap_or_default();
    let other = if picking.typing {
        Span::styled(t!("view.decide_typing").to_owned(), theme.accent())
    } else if own.is_empty() {
        Span::styled(t!("view.decide_other").to_owned(), theme.dim())
    } else {
        Span::styled(ui::fit(&own, width.saturating_sub(4)), theme.accent().add_modifier(Modifier::BOLD))
    };
    out.push(Line::from(vec![Span::styled("o ", theme.bold()), other]));
    let last = step + 1 >= set.questions.len();
    // A question answered in their own words only has no options to pick.
    let mut keys = if q.options.is_empty() {
        Vec::new()
    } else {
        vec![Span::styled(format!("1–{}", q.options.len()), theme.bold()), Span::styled(format!(" {}   ", t!("view.decide_pick")), theme.dim())]
    };
    keys.extend([
        Span::styled("enter", theme.bold()),
        Span::styled(format!(" {}   ", if last { t!("view.decide_send") } else { t!("view.decide_next") }), theme.dim()),
        Span::styled("-", theme.bold()),
        Span::styled(format!(" {}", t!("view.decide_skip")), theme.dim()),
    ]);
    if step > 0 {
        keys.extend([Span::styled("   ←", theme.bold()), Span::styled(format!(" {}", t!("view.decide_back")), theme.dim())]);
    }
    let more = app.more_questions();
    if more > 0 {
        keys.push(Span::styled(format!("   · {}", t!("view.more_waiting", n = more)), theme.dim()));
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

/// The person's answers to a member, as their message shows in the
/// conversation: one line. `None` when the message is words of their own.
pub(super) fn decided_line(text: &str, theme: Theme, width: usize) -> Option<Line<'static>> {
    if decide::is_dismissed(text) {
        return Some(Line::from(vec![Span::styled("✕ ", theme.dim()), Span::styled(t!("view.decide_closed").to_owned(), theme.dim())]));
    }
    let read = decide::read_answers(text)?;
    let head = t!("view.decided", count = read.len());
    let said: Vec<String> = read.iter().map(|r| if r.skipped { t!("view.decide_skipped").to_owned() } else { r.answer.clone() }).collect();
    let room = width.saturating_sub(cols(&head) + 6);
    Some(Line::from(vec![Span::styled("✓ ", theme.accent()), Span::styled(head, theme.bold()), Span::styled(format!(" — {}", ui::fit(&said.join(" · "), room)), theme.dim())]))
}

#[cfg(test)]
mod tests {
    use super::super::{Layouts, render};
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use equipe_core::protocol::decide::{Answer, Choice, Question, QuestionSet};
    use equipe_core::protocol::{Access, Engine, Event, Member, SessionInfo, SessionState, Snapshot};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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

    fn pm() -> App {
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
            steering_in: vec![],
            estimate: None,
            lane: None,
            work: None,
            mode: None,
            effort: None,
        };
        App::from_snapshot("/repo".into(), Snapshot { squad: "acme".into(), projects: vec!["/repo".into()], sessions: vec![info], ..Snapshot::default() }, &[]).0
    }

    fn set() -> QuestionSet {
        let choice = |label: &str, description: &str| Choice { label: label.into(), description: description.into() };
        QuestionSet {
            id: 7,
            session: 1,
            member: "pm".into(),
            ticket: None,
            questions: vec![
                Question { question: "Which login first?".into(), header: "Login".into(), options: vec![choice("Magic link (Recommended)", "no passwords"), choice("Google", "one tap")], multi_select: false },
                Question { question: "Which screens ship?".into(), header: String::new(), options: vec![choice("Onboarding", ""), choice("Orders", ""), choice("Settings", "")], multi_select: true },
            ],
            asked_at: 0,
        }
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE), std::time::Instant::now());
    }

    #[test]
    fn the_questions_show_numbered_over_the_composer_with_their_keys() {
        let mut app = pm();
        app.on_event(Event::QuestionsAsked { squad: "acme".into(), questions: set() });
        let shot = screen(&app, 110, 30);
        let lines: Vec<&str> = shot.lines().collect();
        let at = |text: &str| lines.iter().position(|l| l.contains(text)).unwrap_or_else(|| panic!("no {text:?} in:\n{shot}"));
        assert!(lines[at("pm asks you to decide · 1/2 · Login")].contains("▎ pm"), "{shot}");
        assert_eq!(at("Which login first?"), at("asks you to decide") + 1, "{shot}");
        assert!(lines[at("1 ○ Magic link (Recommended)")].contains("— no passwords"), "{shot}");
        assert_eq!(at("2 ○ Google"), at("1 ○ Magic link") + 1, "{shot}");
        assert!(lines[at("o your own answer")].contains("▎"), "{shot}");
        assert!(lines[at("1–2 pick")].contains("enter next") && lines[at("1–2 pick")].contains("- skip"), "{shot}");

        // Picked, then on: the second, several-pick question, its own keys.
        press(&mut app, KeyCode::Char('1'));
        assert!(screen(&app, 110, 30).contains("1 ● Magic link"), "picked is marked");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Char('o'));
        let shot = screen(&app, 110, 30);
        for expected in ["· 2/2", "Pick any that apply", "1 [ ] Onboarding", "3 [x] Settings", "o type it below", "enter send", "← back"] {
            assert!(shot.contains(expected), "missing {expected:?} in:\n{shot}");
        }
    }

    #[test]
    fn the_answers_read_as_one_line_in_the_conversation() {
        let mut app = pm();
        let message = decide::answer_message(&set(), &[Answer { picked: vec![0], other: None }, Answer::skipped()]);
        app.on_event(Event::UserMessage { session: 1, text: message });
        app.on_event(Event::UserMessage { session: 1, text: decide::dismissed_message(&set()) });
        let shot = screen(&app, 110, 30);
        assert!(shot.contains("✓ Decided · 2 questions — Magic link (Recommended) · skipped"), "{shot}");
        assert!(shot.contains("✕ Closed without answering"), "{shot}");
        assert!(!shot.contains("Answers to your question"), "not the raw message:\n{shot}");
    }
}
