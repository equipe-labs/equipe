//! A change to the team the PM proposed, drawn where a request for
//! permission is — over the composer of the PM's conversation, under the
//! accent bar: what it is, why, what it is for, and `y`/`n`.

use ratatui::text::{Line, Span};

use equipe_core::protocol::team::Change;
use equipe_core::t;

use super::super::app::{App, Pane, describe_team_change};
use super::{Theme, wrapped};
use crate::ui;

/// The change the conversation on screen shows, as lines over the composer.
pub(super) fn lines(app: &App, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let Some(change) = app.shown_team_change().filter(|_| app.pane == Pane::Conversation) else {
        return Vec::new();
    };
    let width = width.saturating_sub(2);
    let mut out: Vec<Line<'static>> = vec![Line::from(vec![
        Span::styled(change.by.clone(), theme.bold()),
        Span::styled(format!(" {}", t!("view.team_proposes")), theme.accent()),
        Span::styled(format!(" · {}", describe_team_change(change)), theme.bold()),
    ])];
    out.extend(wrapped(Vec::new(), &change.reason, theme.dim(), 0, width));
    let mut facts: Vec<String> = Vec::new();
    if !change.unblocks.is_empty() {
        let ids: Vec<String> = change.unblocks.iter().map(|id| format!("T-{id}")).collect();
        facts.push(t!("view.team_first", tickets = ids.join(", ")));
    }
    if let Some(ticket) = change.busy_on {
        facts.push(t!("view.team_busy", name = change.member.as_deref().unwrap_or(&change.role), id = ticket));
    }
    if change.change == Change::Add {
        facts.push(t!("view.team_quota").to_owned());
    }
    if !facts.is_empty() {
        out.push(Line::from(Span::styled(ui::fit(&facts.join(" · "), width), theme.dim())));
    }
    let yes = if change.change == Change::Add { t!("view.team_add") } else { t!("view.team_let_go") };
    let mut keys = vec![
        Span::styled("y", theme.bold()),
        Span::styled(format!(" {yes}   "), theme.dim()),
        Span::styled("n", theme.bold()),
        Span::styled(format!(" {}", t!("view.team_no")), theme.dim()),
    ];
    let more = app.team_changes.len().saturating_sub(1);
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
    use equipe_core::protocol::team::TeamChange;
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
            estimate: None,
            lane: None,
            work: None,
            mode: None,
            effort: None,
            subagents: Vec::new(),
        }
    }

    fn app() -> App {
        let snapshot = Snapshot { squad: "acme".into(), projects: vec!["/repo".into()], sessions: vec![info(1, "pm"), info(2, "mobile")], ..Snapshot::default() };
        App::from_snapshot("/repo".into(), snapshot, &[]).0
    }

    fn change() -> TeamChange {
        TeamChange {
            id: 9,
            change: Change::Add,
            role: "mobile".into(),
            count: 1,
            member: None,
            engine: None,
            model: None,
            effort: None,
            reason: "T-117 has waited 4 hours (mobile is on T-130)".into(),
            unblocks: vec![117],
            by: "pm".into(),
            session: 1,
            proposed_at: 0,
            busy_on: None,
        }
    }

    fn press(app: &mut App, code: KeyCode) -> Vec<Command> {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE), std::time::Instant::now())
    }

    #[test]
    fn a_proposal_shows_over_the_pms_composer_and_y_approves_it() {
        let mut app = app();
        app.on_event(Event::TeamChangeProposed { squad: "acme".into(), change: change() });
        let shot = screen(&app, 110, 30);
        for expected in ["▎ pm proposes a team change · one more mobile", "▎ T-117 has waited 4 hours (mobile is on T-130)", "▎ takes T-117 first", "▎ y add   n no"] {
            assert!(shot.contains(expected), "missing {expected:?} in:\n{shot}");
        }
        assert!(shot.contains("uses your Claude/Codex plan alongside the team"), "{shot}");
        // Something typed: the keys are the composer's.
        app.composer.insert_str("yes");
        assert!(press(&mut app, KeyCode::Char('!')).is_empty());
        app.composer.clear();
        assert_eq!(press(&mut app, KeyCode::Char('y')), vec![Command::ApproveTeamChange { squad: "acme".into(), id: 9 }]);
        assert!(!screen(&app, 110, 30).contains("proposes a team change"), "gone at once");
        // Decided elsewhere, a second one leaves too.
        app.on_event(Event::TeamChangeProposed { squad: "acme".into(), change: TeamChange { id: 10, change: Change::Remove, member: Some("mobile".into()), busy_on: Some(130), ..change() } });
        let shot = screen(&app, 110, 30);
        assert!(shot.contains("let mobile go") && shot.contains("mobile is on T-130: it goes back to the board") && shot.contains("y let go   n no"), "{shot}");
        app.on_event(Event::TeamChangeClosed { squad: "acme".into(), id: 10, approved: true, members: vec!["mobile".into()] });
        assert!(app.team_changes.is_empty());
    }

    #[test]
    fn n_says_no_and_another_conversation_shows_nothing() {
        let mut app = app();
        app.on_event(Event::TeamChangeProposed { squad: "acme".into(), change: change() });
        assert_eq!(app.flash.as_deref(), Some("pm proposes a team change: one more mobile — y/n in pm's conversation"));
        app.selected = 1;
        assert!(!screen(&app, 110, 30).contains("proposes a team change ·"), "only in the PM's");
        assert!(press(&mut app, KeyCode::Char('n')).iter().all(|c| !matches!(c, Command::DeclineTeamChange { .. })));
        app.composer.clear();
        app.selected = 0;
        assert_eq!(press(&mut app, KeyCode::Char('n')), vec![Command::DeclineTeamChange { squad: "acme".into(), id: 9 }]);
    }
}
