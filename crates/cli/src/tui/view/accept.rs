//! Handed-in work put to the person to confirm, drawn where a request for
//! permission is — over the composer of the conversation it was said in,
//! under the accent bar: what it does, each ticket, what starts after, and
//! `y`/`n`.

use ratatui::text::{Line, Span};

use equipe_core::protocol::accept::Verdict;
use equipe_core::t;

use super::super::app::{App, Pane, accept_ids};
use super::{Theme, wrapped};
use crate::ui;

/// The proposal the conversation on screen shows, as lines over the composer.
pub(super) fn lines(app: &App, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let Some(p) = app.shown_accept().filter(|_| app.pane == Pane::Conversation) else {
        return Vec::new();
    };
    let width = width.saturating_sub(2);
    let tickets = accept_ids(p);
    let (ask, yes) = match p.verdict {
        Verdict::Accept => (t!("view.accept_ask", tickets = tickets), t!("view.accept_yes")),
        Verdict::Merge => (t!("view.accept_ask_merge", tickets = tickets), t!("view.accept_yes_merge")),
        Verdict::Changes => (t!("view.accept_ask_changes", tickets = tickets), t!("view.accept_yes_changes")),
    };
    let by = if p.from_words() { t!("view.accept_by_words").to_owned() } else { t!("view.accept_by", name = p.by) };
    let mut out: Vec<Line<'static>> = vec![Line::from(vec![Span::styled(ask, theme.bold()), Span::styled(format!(" · {by}"), theme.dim())])];
    if !p.summary.is_empty() {
        out.extend(wrapped(Vec::new(), &p.summary, theme.dim(), 0, width));
    }
    if p.verdict == Verdict::Changes && !p.note.is_empty() {
        out.extend(wrapped(Vec::new(), &t!("view.accept_note", note = p.note), theme.dim(), 0, width));
    }
    for id in &p.tickets {
        let title = app.tickets.iter().find(|t| t.id == *id).map(|t| t.title.clone()).unwrap_or_default();
        out.push(Line::from(Span::styled(ui::fit(&format!("  T-{id} {title}"), width), theme.dim())));
    }
    if p.verdict != Verdict::Changes {
        let starts = equipe_core::accept::starts_after(&app.tickets, &p.tickets);
        if !starts.is_empty() {
            let list = starts.iter().map(|id| format!("T-{id}")).collect::<Vec<_>>().join(", ");
            out.push(Line::from(Span::styled(ui::fit(&t!("view.accept_starts", tickets = list), width), theme.dim())));
        }
    }
    out.push(Line::from(vec![
        Span::styled("y", theme.bold()),
        Span::styled(format!(" {yes}   "), theme.dim()),
        Span::styled("n", theme.bold()),
        Span::styled(format!(" {}", t!("view.accept_no")), theme.dim()),
    ]));
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
    use equipe_core::protocol::accept::{AcceptProposal, Confirmed, FROM_WORDS, Land};
    use equipe_core::protocol::{Access, Command, Engine, Event, Member, SessionInfo, SessionState, Snapshot, Ticket};
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

    fn ticket(id: u32, status: &str, depends_on: &[u32]) -> Ticket {
        serde_json::from_value(serde_json::json!({ "id": id, "title": format!("work {id}"), "body": "", "status": status, "kind": "code", "project": "/repo", "branch": format!("equipe/t-{id}"), "depends_on": depends_on, "created_by": "pm" })).unwrap()
    }

    fn app() -> App {
        let tickets = vec![ticket(42, "review", &[]), ticket(71, "review", &[]), ticket(45, "todo", &[42])];
        let snapshot = Snapshot { squad: "acme".into(), projects: vec!["/repo".into()], sessions: vec![info(1, "pm"), info(2, "frontend")], tickets, ..Snapshot::default() };
        App::from_snapshot("/repo".into(), snapshot, &[]).0
    }

    fn proposal() -> AcceptProposal {
        AcceptProposal { id: 9, verdict: Verdict::Accept, tickets: vec![42, 71], note: String::new(), summary: "결제 수단과 요청 제한".into(), by: "pm".into(), session: 1, proposed_at: 0 }
    }

    fn press(app: &mut App, code: KeyCode) -> Vec<Command> {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE), std::time::Instant::now())
    }

    #[test]
    fn a_proposal_shows_over_its_conversations_composer_and_y_confirms_it_the_projects_way() {
        let mut app = app();
        app.on_event(Event::AcceptProposed { squad: "acme".into(), proposal: proposal() });
        assert_eq!(app.flash.as_deref(), Some("To confirm: accept T-42, T-71 — y/n in its conversation"));
        let shot = screen(&app, 110, 32);
        for expected in ["▎ Accept T-42, T-71? · pm proposes", "▎ 결제 수단과 요청 제한", "▎   T-42 work 42", "▎   T-71 work 71", "▎ Then T-45 can start", "▎ y accept   n no"] {
            assert!(shot.contains(expected), "missing {expected:?} in:\n{shot}");
        }
        // Something typed: the keys are the composer's.
        app.composer.insert_str("yes");
        assert!(press(&mut app, KeyCode::Char('!')).is_empty());
        app.composer.clear();
        // Merged the way this terminal was told for the project, as A would.
        app.merge_ways.insert("repo".into(), crate::tui::app::MergeWay::Push);
        let sent = press(&mut app, KeyCode::Char('y'));
        assert_eq!(sent, vec![Command::ConfirmAccept { squad: "acme".into(), id: 9, tickets: vec![Confirmed { ticket: 42, merge: Some(Land::Push) }, Confirmed { ticket: 71, merge: Some(Land::Push) }] }]);
        assert!(!screen(&app, 110, 32).contains("Accept T-42, T-71?"), "gone at once");
    }

    #[test]
    fn n_says_no_another_conversation_shows_nothing_and_words_say_whose_they_were() {
        let mut app = app();
        let words = AcceptProposal { verdict: Verdict::Merge, by: FROM_WORDS.into(), summary: String::new(), ..proposal() };
        app.on_event(Event::AcceptProposed { squad: "acme".into(), proposal: words.clone() });
        assert!(screen(&app, 110, 32).contains("▎ Accept and merge T-42, T-71? · from your words"));
        // No way set for the project: to merge, it merges (no push).
        let mut other = self::app();
        other.on_event(Event::AcceptProposed { squad: "acme".into(), proposal: words });
        assert_eq!(press(&mut other, KeyCode::Char('y')), vec![Command::ConfirmAccept { squad: "acme".into(), id: 9, tickets: vec![Confirmed { ticket: 42, merge: Some(Land::Merge) }, Confirmed { ticket: 71, merge: Some(Land::Merge) }] }]);
        app.selected = 1;
        assert!(!screen(&app, 110, 32).contains("merge T-42, T-71?"), "only where it was said");
        assert!(press(&mut app, KeyCode::Char('n')).iter().all(|c| !matches!(c, Command::DeclineAccept { .. })));
        app.composer.clear();
        app.selected = 0;
        assert_eq!(press(&mut app, KeyCode::Char('n')), vec![Command::DeclineAccept { squad: "acme".into(), id: 9 }]);
        app.on_event(Event::AcceptClosed { squad: "acme".into(), id: 9, confirmed: false, tickets: vec![] });
        assert!(app.accepts.is_empty());
    }
}
