//! A routine the PM proposed, drawn where a change to the team is — over the
//! composer of the PM's conversation, under the accent bar: its title and
//! when it runs, what the PM would be asked each time, why, and `y`/`n`.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use equipe_core::t;

use super::super::app::{App, Pane, schedule_words};
use super::{Theme, wrapped};

/// The rows of the prompt shown; a longer one ends in `…`.
const PROMPT_ROWS: usize = 6;

/// The proposal the conversation on screen shows, as lines over the composer.
pub(super) fn lines(app: &App, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let Some(proposal) = app.shown_routine_proposal().filter(|_| app.pane == Pane::Conversation) else {
        return Vec::new();
    };
    let width = width.saturating_sub(2);
    let routine = &proposal.routine;
    let mut out: Vec<Line<'static>> = vec![Line::from(vec![
        Span::styled(proposal.by.clone(), theme.bold()),
        Span::styled(format!(" {}", t!("view.routine_proposes")), theme.accent()),
        Span::styled(format!(" · {}", routine.title), theme.bold()),
        Span::styled(format!(" · {}", schedule_words(routine.when)), theme.accent()),
    ])];
    let mut prompt = wrapped(Vec::new(), routine.prompt.trim(), Style::default(), 0, width);
    if prompt.len() > PROMPT_ROWS {
        prompt.truncate(PROMPT_ROWS);
        if let Some(last) = prompt.last_mut() {
            last.spans.push(Span::raw(" …"));
        }
    }
    out.extend(prompt);
    if !proposal.reason.trim().is_empty() {
        out.extend(wrapped(Vec::new(), proposal.reason.trim(), theme.dim(), 0, width));
    }
    let mut keys = vec![
        Span::styled("y", theme.bold()),
        Span::styled(format!(" {}   ", t!("view.routine_turn_on")), theme.dim()),
        Span::styled("n", theme.bold()),
        Span::styled(format!(" {}", t!("view.routine_no")), theme.dim()),
    ];
    let more = app.routine_proposals.len().saturating_sub(1);
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

#[cfg(test)]
mod tests {
    use super::super::{Layouts, render};
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use equipe_core::protocol::routines::{Routine, RoutineProposal, Schedule};
    use equipe_core::protocol::{Access, Command, Engine, Event, Member, SessionInfo, SessionState, Snapshot};
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

    fn info(session: u64, role: &str) -> SessionInfo {
        SessionInfo {
            session,
            project: "/repo".into(),
            member: Some(Member { role: role.into(), name: role.into() }),
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
            steering_in: Vec::new(),
            estimate: None,
            lane: None,
            work: None,
            mode: None,
            effort: None,
            subagents: Vec::new(),
            compacting: None,
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot { squad: "acme".into(), projects: vec!["/repo".into()], sessions: vec![info(1, "pm"), info(2, "mobile")], ..Snapshot::default() }
    }

    fn app() -> App {
        App::from_snapshot("/repo".into(), snapshot(), &[]).0
    }

    fn proposal() -> RoutineProposal {
        RoutineProposal {
            id: 4,
            routine: Routine {
                id: 0,
                title: "Weekly check-in".into(),
                prompt: "Sum up last week and propose what's next.".into(),
                when: Schedule::Weekly { weekday: 1, hour: 9, minute: 0 },
                on: true,
                last_run: None,
                next_run: None,
            },
            reason: "You asked for a summary three Mondays running".into(),
            by: "pm".into(),
            session: 1,
            proposed_at: 0,
        }
    }

    fn press(app: &mut App, code: KeyCode) -> Vec<Command> {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE), std::time::Instant::now())
    }

    #[test]
    fn a_schedule_reads_in_words() {
        assert_eq!(schedule_words(Schedule::Weekly { weekday: 1, hour: 9, minute: 0 }), "every Monday at 09:00");
        assert_eq!(schedule_words(Schedule::Daily { hour: 16, minute: 5 }), "every day at 16:05");
    }

    #[test]
    fn a_proposal_shows_over_the_pms_composer_and_y_turns_it_on() {
        let mut app = app();
        app.on_event(Event::RoutineProposed { squad: "acme".into(), proposal: proposal() });
        let shot = screen(&app, 110, 30);
        for expected in [
            "▎ pm proposes a routine · Weekly check-in · every Monday at 09:00",
            "▎ Sum up last week and propose what's next.",
            "▎ You asked for a summary three Mondays running",
            "▎ y turn on   n no",
        ] {
            assert!(shot.contains(expected), "missing {expected:?} in:\n{shot}");
        }
        // Something typed: the keys are the composer's.
        app.composer.insert_str("yes");
        assert!(press(&mut app, KeyCode::Char('!')).is_empty());
        app.composer.clear();
        assert_eq!(press(&mut app, KeyCode::Char('y')), vec![Command::ApproveRoutine { squad: "acme".into(), id: 4 }]);
        assert!(!screen(&app, 110, 30).contains("proposes a routine"), "gone at once");
        // Decided elsewhere, a second one leaves too.
        app.on_event(Event::RoutineProposed { squad: "acme".into(), proposal: RoutineProposal { id: 5, ..proposal() } });
        assert_eq!(app.routine_proposals.len(), 1);
        app.on_event(Event::RoutineProposalClosed { squad: "acme".into(), id: 5, approved: false });
        assert!(app.routine_proposals.is_empty());
    }

    #[test]
    fn n_says_no_and_another_conversation_shows_nothing() {
        let mut app = app();
        app.on_event(Event::RoutineProposed { squad: "acme".into(), proposal: proposal() });
        assert_eq!(app.flash.as_deref(), Some("pm proposes a routine: Weekly check-in — y/n in pm's conversation"));
        app.selected = 1;
        assert!(!screen(&app, 110, 30).contains("proposes a routine ·"), "only in the PM's");
        assert!(press(&mut app, KeyCode::Char('n')).iter().all(|c| !matches!(c, Command::DeclineRoutine { .. })));
        app.composer.clear();
        app.selected = 0;
        assert_eq!(press(&mut app, KeyCode::Char('n')), vec![Command::DeclineRoutine { squad: "acme".into(), id: 4 }]);
    }

    #[test]
    fn a_snapshot_brings_its_proposals_and_idle_wake() {
        let app = App::from_snapshot("/repo".into(), Snapshot { routine_proposals: vec![proposal()], idle_wake_hours: Some(0), ..snapshot() }, &[]).0;
        assert_eq!(app.routine_proposals.len(), 1);
        assert_eq!(app.idle_wake_hours, 0);
        assert_eq!(App::from_snapshot("/repo".into(), snapshot(), &[]).0.idle_wake_hours, 24, "a day, unless set");
    }

    #[test]
    fn wake_turns_the_idle_wake_on_and_off() {
        let mut app = app();
        let run = |app: &mut App, text: &str| {
            app.composer.insert_str(text);
            press(app, KeyCode::Enter)
        };
        assert_eq!(run(&mut app, "/wake off"), vec![Command::SetIdleWake { squad: "acme".into(), hours: 0 }]);
        assert_eq!(app.idle_wake_hours, 0);
        assert_eq!(run(&mut app, "/wake on"), vec![Command::SetIdleWake { squad: "acme".into(), hours: 24 }]);
        assert!(run(&mut app, "/wake").is_empty());
        assert!(app.flash.as_deref().unwrap().contains("24"), "{:?}", app.flash);
        app.on_event(Event::IdleWakeChanged { squad: "acme".into(), hours: 0 });
        assert_eq!(app.idle_wake_hours, 0);
    }
}
