//! Work nobody asked for waits for the person's yes (a squad on full autonomy:
//! a non-developer's team). On Golden Boot only a quarter of the AI spend went
//! to what the customer or the operator asked for; 38% went to a design setup
//! the designer opened by itself and the restyle that came of it.
//!
//! A ticket a member (or the PM) puts on the board that serves none of the
//! person's requests, comes of no work that does, and isn't cut from a plan is
//! self-started: it goes up as a proposal, and nothing starts it until it
//! serves a request of theirs — the PM links it
//! (`update_ticket` with `request`) once they said yes. A fix of something
//! broken (a `fix` at S1/S2) and a fix too small to matter go on as before;
//! so does equipe's own work (design QA, the daily review, a red main, the
//! platform's monitor), which never comes through here.
//!
//! The estimate is the team's own planning (it decides what is tiny and goes
//! on, and the PM weighs whether a proposal is worth the person's credit): the
//! person is never told it unless they ask what something costs (owner,
//! 2026-11-26: no estimated costs in chat).
//!
//! No runtime types here: the rules and the words.

use equipe_protocol::title::TicketLabel;

/// What a million tokens of a member's turns cost a customer, all in (input,
/// output, cache writes and reads at the strong tier, with the platform's
/// margin): read off Golden Boot's ledger, where the accounts ticket's 11M
/// tokens were charged $12.53. Only for the team's planning and for answering
/// "what does this cost?" — the platform's ledger has the exact amounts.
pub const USD_PER_MTOK: f64 = 1.1;

/// A self-started fix estimated under this goes on without a yes.
pub const TINY_USD: f64 = 0.20;

/// When nothing of the squad's has finished yet: what a typical ticket costs.
pub const TYPICAL_USD: f64 = 3.0;

/// Dollars for `tokens`.
pub fn usd(tokens: u64) -> f64 {
    tokens as f64 / 1_000_000.0 * USD_PER_MTOK
}

/// "$0.40", "$12": the amount in words, for the team (and the person only when they ask).
pub fn said(usd: f64) -> String {
    if usd < 10.0 { format!("${usd:.2}") } else { format!("${usd:.0}") }
}

/// What a new ticket is likely to cost: the median of what the squad's
/// finished tickets took (their tokens), else [`TYPICAL_USD`].
pub fn estimate(finished_tokens: &[u64]) -> f64 {
    let mut spent: Vec<u64> = finished_tokens.iter().copied().filter(|t| *t > 0).collect();
    if spent.is_empty() {
        return TYPICAL_USD;
    }
    spent.sort_unstable();
    usd(spent[spent.len() / 2])
}

/// What [`gated`] reads of a new ticket.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ask<'a> {
    /// The squad runs on its own for a non-developer (autonomy full).
    pub hands_off: bool,
    /// It serves one of the person's requests.
    pub serves_request: bool,
    /// It was made while working a ticket that serves one (its follow-up, a finding of it).
    pub of_requested_work: bool,
    /// It is cut from a plan the person approved.
    pub from_plan: bool,
    /// The person put it on the board themselves.
    pub by_person: bool,
    pub label: Option<TicketLabel>,
    /// `S1`…`S4`, when given.
    pub severity: Option<&'a str>,
    /// Its estimated cost.
    pub estimate: f64,
}

/// Whether a new ticket waits for the person's yes.
pub fn gated(a: &Ask<'_>) -> bool {
    if !a.hands_off || a.serves_request || a.of_requested_work || a.from_plan || a.by_person {
        return false;
    }
    let fix = a.label == Some(TicketLabel::Fix) || a.severity.is_some();
    let broken = matches!(a.severity.map(str::trim), Some("S1" | "S2"));
    !(fix && (broken || a.estimate < TINY_USD))
}

/// What its member is told.
pub fn reply(id: u32, usd: f64) -> String {
    format!(
        "Proposed T-{id}, not started: nobody asked for it, so it waits for the person's yes (about {} by similar work, for the \
team's planning only). The PM puts it to them in one plain line — what it would change for them, never a price — and it starts \
once it serves their request. Go on with what was asked meanwhile.",
        said(usd)
    )
}

/// What the PM is told.
pub fn pm_line(member: &str, id: u32, title: &str, usd: f64) -> String {
    format!(
        "{member} proposed T-{id} “{title}”, which nobody asked for: it doesn't start until the person says yes. Similar work \
cost about {} — that is for you to weigh whether it is worth their credit, never a line for them. If it is worth it, suggest it in \
one plain line by what it would change for them (\"The team suggests …\"), with no cost, price or credits — not even when they ask; \
when they say yes, `update_ticket` it with their `request` and it starts. Otherwise cancel it. Never start it, or another ticket \
doing the same, without their yes.",
        said(usd)
    )
}

/// The sentences of `text` that tell the PM or a member to put a cost in front
/// of the person ("say what it would cost", "with its expected cost", "about
/// $X"), with nothing in the sentence saying not to. Briefs, skills and notes
/// must have none: estimates are the team's planning (tests across the crate).
#[cfg(test)]
pub(crate) fn cost_talk(text: &str) -> Vec<String> {
    const SAY: [&str; 12] = ["tell", "say ", "says", "state", "quote", "mention", "suggest", "propose", "offer", "put it to", "puts it to", "ask them"];
    const COST: [&str; 12] = [
        "expected cost",
        "estimated cost",
        "its cost",
        "with the cost",
        "what it would cost",
        "would cost about",
        "what it costs",
        "what a round costs",
        "will cost",
        "about $",
        "cost about",
        "\"about $x\"",
    ];
    // What says not to, about the cost itself (a bare "never" elsewhere in the sentence doesn't count).
    const NOT: [&str; 14] = [
        "never a price",
        "never with a price",
        "no cost",
        "no price",
        "no \"about $x\"",
        "never quote",
        "never tell",
        "never a line for them",
        "unless they ask",
        "only when they ask",
        "planning only",
        "for the team; never",
        "without a price",
        "card",
    ];
    // Wrapped lines are one sentence: join them, then cut at the sentence's end.
    text.replace('\n', " ")
        .replace("! ", ". ")
        .replace("? ", ". ")
        .split(". ")
        .map(str::trim)
        .filter(|s| {
            let l = s.to_lowercase();
            SAY.iter().any(|w| l.contains(w)) && COST.iter().any(|w| l.contains(w)) && !NOT.iter().any(|w| l.contains(w))
        })
        .map(|s| s.chars().take(160).collect())
        .collect::<Vec<String>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_nobody_asked_for_waits_unless_it_fixes_something_broken_or_tiny() {
        let base = Ask { hands_off: true, estimate: 3.0, ..Ask::default() };
        // Golden Boot, 11-23 to 11-24: the designer's own design setup and the restyle.
        assert!(gated(&Ask { label: Some(TicketLabel::Design), ..base }), "design setup nobody asked for");
        assert!(gated(&base), "an \"improvement\"");
        // What was asked, or comes of it, or is cut from an approved plan, or the person filed, goes on.
        for ok in [Ask { serves_request: true, ..base }, Ask { of_requested_work: true, ..base }, Ask { from_plan: true, ..base }, Ask { by_person: true, ..base }] {
            assert!(!gated(&ok), "{ok:?}");
        }
        // Something broken, or a fix too small to matter, goes on.
        assert!(!gated(&Ask { label: Some(TicketLabel::Fix), severity: Some("S2"), ..base }));
        assert!(!gated(&Ask { label: Some(TicketLabel::Fix), estimate: 0.12, ..base }));
        assert!(gated(&Ask { label: Some(TicketLabel::Fix), severity: Some("S3"), ..base }), "polish nobody asked for, at full price");
        // A developer's own squad runs as before.
        assert!(!gated(&Ask { hands_off: false, ..base }));
    }

    #[test]
    fn the_estimate_is_the_median_of_finished_work_in_plain_dollars() {
        assert_eq!(estimate(&[]), TYPICAL_USD);
        assert!((estimate(&[1_000_000, 3_000_000, 0, 11_000_000]) - 3.3).abs() < 1e-9);
        assert_eq!(said(0.4), "$0.40");
        assert_eq!(said(12.53), "$13");
        assert!(reply(28, 18.6).contains("about $19") && pm_line("designer", 28, "Restyle", 18.6).contains("Never start it"));
    }

    #[test]
    fn the_estimate_is_the_teams_and_never_a_line_for_the_person() {
        // Owner, 2026-11-26, after a PM reply ended "예상 비용은 약 $1.50~$3.50예요": "예상 비용 보여주지 말자".
        let (member, pm) = (reply(28, 1.5), pm_line("designer", 28, "Logo refresh", 1.5));
        assert!(member.contains("for the team's planning only") && member.contains("never a price"), "{member}");
        assert!(pm.contains("never a line for them") && pm.contains("no cost, price or credits — not even when they ask"), "{pm}");
        assert_eq!(cost_talk(&member), Vec::<String>::new());
        assert_eq!(cost_talk(&pm), Vec::<String>::new());
        // The words this module gave before tell the PM to say the cost: the check catches them.
        assert!(!cost_talk("The PM puts it to them in one plain line — what it would change for them and what it would cost — and it starts.").is_empty());
        assert!(!cost_talk("If it is worth it, ask them in one plain line what it would change and that it would cost about $1.50; when they say yes, go.").is_empty());
    }
}
