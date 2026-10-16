//! Both engines in the terminal: each plan's windows in the footer, and the
//! card over the composer — what runs low, who moves where, and the keys.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use equipe_core::protocol::engines::Why;
use equipe_core::t;

use super::super::app::engines::{engine_name, headline, move_row};
use super::super::app::{App, Pane};
use super::{Theme, pad, wrapped};
use crate::ui;

/// The footer's facts: each window's use, the engine named once both are read
/// ("Claude 5h 62%", "Codex wk 18%"); in the accent from 80%.
pub(super) fn footer(app: &App, theme: Theme) -> Vec<(String, Style)> {
    let named = app.engines.plans.len() > 1;
    let mut facts = Vec::new();
    for plan in &app.engines.plans {
        let windows = [(t!("view.window_5h"), plan.usage.five_hour), (t!("view.window_week"), plan.usage.seven_day)];
        for (i, (label, window)) in windows.into_iter().filter_map(|(l, w)| Some((l, w?))).enumerate() {
            let style = if window.used_percent >= 80 { theme.accent().add_modifier(Modifier::BOLD) } else { theme.dim() };
            let engine = if named && i == 0 { format!("{} ", engine_name(plan.engine)) } else { String::new() };
            facts.push((format!("{engine}{label} {}%", window.used_percent), style));
        }
    }
    facts
}

/// The card the conversation shows, as lines over the composer.
pub(super) fn lines(app: &App, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let Some(card) = app.shown_engine_proposal().filter(|_| app.pane == Pane::Conversation) else {
        return Vec::new();
    };
    let width = width.saturating_sub(2);
    let title = if card.why == Why::Setup { t!("engines.setup_heading") } else { t!("engines.heading") };
    let mut out: Vec<Line<'static>> = vec![Line::from(Span::styled(title.to_owned(), theme.accent()))];
    out.extend(wrapped(Vec::new(), &headline(card), theme.bold(), 0, width));
    let name_width = card.moves.iter().map(|m| m.member.chars().count()).max().unwrap_or(0).clamp(4, 14);
    for m in card.moves.iter().take(8) {
        let (who, what) = move_row(m);
        let style = if m.done { theme.dim() } else { Style::default() };
        out.push(Line::from(vec![Span::styled(format!("  {} ", pad(&ui::fit(&who, name_width), name_width)), theme.bold()), Span::styled(ui::fit(&what, width.saturating_sub(name_width + 3)), style)]));
    }
    if card.moves.len() > 8 {
        out.push(Line::from(Span::styled(format!("  {}", t!("view.more_waiting", n = card.moves.len() - 8)), theme.dim())));
    }
    let keys: Vec<(&str, &str)> = match (card.why, card.applied) {
        (_, true) => vec![("u", t!("engines.key_undo")), ("n", t!("engines.key_ok"))],
        (Why::Setup, false) => vec![("y", t!("engines.key_apply")), ("n", t!("engines.key_later"))],
        (_, false) => vec![("y", t!("engines.key_move")), ("n", t!("engines.key_keep"))],
    };
    let mut spans = Vec::new();
    for (key, what) in keys {
        spans.push(Span::styled(key.to_owned(), theme.bold()));
        spans.push(Span::styled(format!(" {what}   "), theme.dim()));
    }
    if card.why != Why::Setup {
        spans.push(Span::styled(format!("· {}", t!("engines.no_credits")), theme.dim()));
    }
    out.push(Line::from(spans));
    let bar = || Span::styled("▎ ", theme.accent());
    out.into_iter()
        .map(|line| {
            let mut spans = vec![bar()];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}
