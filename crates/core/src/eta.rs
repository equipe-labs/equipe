//! How long work will take, guessed from how long the same kind of work took
//! before — for when a member hasn't said.

use equipe_protocol::TicketKind;

/// Fewer than this and there is nothing to go by.
const MIN_SAMPLES: usize = 3;

/// The role a member's name is of: `frontend` for `frontend-2`.
pub fn role_of(name: &str) -> &str {
    match name.rsplit_once('-') {
        Some((role, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => role,
        _ => name,
    }
}

/// What `samples` (seconds) typically took: their median, once there are
/// enough to go by.
pub fn typical(samples: &[u64]) -> Option<u64> {
    if samples.len() < MIN_SAMPLES {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    Some(sorted[sorted.len() / 2])
}

/// How long a ticket of `kind` typically takes `member`: what its role took
/// for that kind lately, else what anyone took for it.
pub fn ticket_time(history: &[(String, TicketKind, u64)], member: &str, kind: TicketKind) -> Option<u64> {
    let role = role_of(member);
    let theirs: Vec<u64> = history.iter().filter(|(m, k, _)| role_of(m) == role && *k == kind).map(|h| h.2).take(30).collect();
    typical(&theirs).or_else(|| {
        let anyone: Vec<u64> = history.iter().filter(|h| h.1 == kind).map(|h| h.2).take(50).collect();
        typical(&anyone)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_role_goes_by_its_own_past_then_by_everyones() {
        let h = |m: &str, k, mins: u64| (m.to_owned(), k, mins * 60);
        let history = vec![
            h("frontend", TicketKind::Code, 10),
            h("frontend-2", TicketKind::Code, 30),
            h("frontend", TicketKind::Code, 12),
            h("frontend", TicketKind::Code, 700),
            h("backend", TicketKind::Code, 20),
            h("pm", TicketKind::Task, 4),
            h("designer", TicketKind::Task, 6),
            h("pm-2", TicketKind::Task, 8),
        ];
        // The median: one long wait doesn't sway it.
        assert_eq!(ticket_time(&history, "frontend-3", TicketKind::Code), Some(30 * 60));
        // Backend has one ticket of its own: everyone's code tickets.
        assert_eq!(ticket_time(&history, "backend", TicketKind::Code), Some(20 * 60));
        assert_eq!(ticket_time(&history, "qa", TicketKind::Task), Some(6 * 60));
        assert_eq!(ticket_time(&history, "qa", TicketKind::Action), None, "nothing to go by");
        assert_eq!(role_of("data-science"), "data-science");
    }
}
