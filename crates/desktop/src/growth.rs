//! The growth loop on screen (`equipe_core::growth`, docs/GROWTH.md): its
//! open cards wait with the other decisions; Progress lists the latest cards
//! under Growth — each its number, title and where it stands, a measured one
//! with what it did. No UI types here, so the rules are tested without a window.

use equipe_core::protocol::growth::{Card, Growth, Status, Verdict};
use equipe_core::t;

/// Cards listed in Progress, at most.
pub const SHOWN: usize = 6;

/// A card's row: `G-3`, its title, where it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub state: String,
    /// It went the wrong way.
    pub worse: bool,
}

/// Where a card stands, in the person's words: its answer, its ticket, or
/// what its latest follow-up found.
pub fn state(card: &Card) -> (String, bool) {
    let pct = |p: Option<f64>| p.map(|p| format!("{}{}%", if p >= 0.0 { "+" } else { "" }, (p * 10.0).round() / 10.0)).unwrap_or_default();
    match card.status {
        Status::Open => (t!("growth.status_open").to_owned(), false),
        Status::Later => (t!("growth.status_later").to_owned(), false),
        Status::Declined => (t!("growth.status_declined").to_owned(), false),
        Status::Dropped => (t!("growth.status_dropped").to_owned(), false),
        Status::Accepted => (t!("growth.status_accepted", ticket = card.ticket.map_or_else(|| "–".to_owned(), |t| t.to_string())), false),
        Status::Live | Status::Measured => match card.latest().filter(|o| o.failed.is_none()) {
            None => (t!("growth.status_measuring").to_owned(), false),
            Some(o) => match o.verdict {
                Verdict::Worked => (t!("growth.status_worked", pct = pct(o.change_pct)).trim().to_owned(), false),
                Verdict::NoChange => (t!("growth.status_no_change", pct = pct(o.change_pct)).trim().to_owned(), false),
                Verdict::Worse => (t!("growth.status_worse", pct = pct(o.change_pct)).trim().to_owned(), true),
            },
        },
    }
}

/// The latest cards, newest first, at most [`SHOWN`].
pub fn rows(growth: &Growth) -> Vec<Row> {
    let mut cards: Vec<&Card> = growth.cards.iter().collect();
    cards.sort_by_key(|c| std::cmp::Reverse(c.id));
    cards
        .into_iter()
        .take(SHOWN)
        .map(|c| {
            let (state, worse) = state(c);
            Row { id: format!("G-{}", c.id), title: c.title.clone(), state, worse }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::growth::{FollowUp, Outcome, Risk, Size};

    fn card(id: u32, status: Status) -> Card {
        Card {
            id,
            title: format!("card {id}"),
            change: String::new(),
            metric: "event:x".into(),
            expected: "+5%".into(),
            size: Size::S,
            risk: Risk::Low,
            evidence: Vec::new(),
            project: None,
            status,
            proposed_at: 0,
            set: 0,
            question: 0,
            answered_at: None,
            ticket: Some(12),
            request: None,
            live_at: None,
            follow_ups: Vec::new(),
            reverts: None,
        }
    }

    #[test]
    fn the_latest_cards_say_where_they_stand() {
        let mut measured = card(9, Status::Measured);
        let outcome = |verdict, pct| Outcome { verdict, before: 40.0, after: 44.0, change_pct: Some(pct), before_from: String::new(), before_to: String::new(), after_from: String::new(), after_to: String::new(), query: String::new(), at: 0, failed: None };
        measured.follow_ups = vec![FollowUp { after_days: 7, due_at: 0, outcome: Some(outcome(Verdict::Worked, 10.0)) }, FollowUp { after_days: 14, due_at: 0, outcome: Some(outcome(Verdict::Worse, -6.25)) }];
        let growth = Growth { cards: (1..=8).map(|i| card(i, Status::Open)).chain([measured]).collect(), ..Growth::default() };
        let rows = rows(&growth);
        assert_eq!(rows.len(), SHOWN, "only the latest");
        assert_eq!((rows[0].id.as_str(), rows[1].id.as_str()), ("G-9", "G-8"), "newest first");
        assert!(rows[0].worse && rows[0].state.contains("-6.3%"), "the latest follow-up: {}", rows[0].state);
        assert!(super::state(&card(1, Status::Accepted)).0.contains("T-12"));
        assert!(!super::state(&card(1, Status::Live)).1);
    }
}
