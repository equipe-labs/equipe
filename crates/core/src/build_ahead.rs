//! Build ahead while the person is asked: the way a squad that runs on its own
//! for a non-developer works when something only the person can give is
//! missing — an outside account (Stripe, PayPal, a calendar, a social
//! account), a fact (prices, hours, a name), a decision, photos.
//!
//! Seen in production: a hosted team's frontend engineer checked that the
//! business had not connected Stripe yet and put its whole "let players pay
//! when they book" ticket down as waiting, with nothing built ("once it's
//! connected I'll set the prices, add the checkout, test and redeploy"); the
//! PM had even written "if it's still not ready, hand it back". The founder
//! had to say by hand that Stripe was not a blocker.
//!
//! Now:
//! - every member of such a squad is told the rule ([`RULE`]): ask once,
//!   right away; build everything else, the waiting piece behind its switch
//!   or a marked placeholder; finish it the moment it arrives, check it, and
//!   tell the person in one line;
//! - `block_on_person` for connecting an outside business account is refused
//!   where the squad has equipe's platform tools, which carry `ask_to_connect`
//!   (one Connect card) and switch the payment step on before it is connected
//!   ([`account_ask`]);
//! - a fact card the PM put to the person, once answered, comes back to the
//!   PM with the members working on placeholders for facts, so each swaps
//!   them at once ([`placeholders_line`]).
//!
//! Pure rules, tested here; the runtime applies them (`runtime.rs`
//! `block_on_person`, `runtime/decide.rs`).

use equipe_protocol::TicketId;

/// `text` cut to `max` characters, with an ellipsis when cut.
fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max { text.to_owned() } else { format!("{}…", text.chars().take(max).collect::<String>().trim_end()) }
}

/// The rule every member of a squad that runs on its own is given.
pub const RULE: &str = "Build ahead while the person is asked. When work needs something only the person can give — an \
outside account (payments, a calendar, a social account), a sign-in, a decision, a fact (prices, hours, a name), photos — \
never stop the work for it and never wait idle: (1) ask once, right away — an account with `ask_to_connect` where you have \
it, a fact or a choice only they can make with an open `ask_decision` question — and never ask again for what is already \
asked, nor write \"tell us when it's done\"; (2) build everything else now, the waiting piece in behind its switch (the \
payment step with `enable_payments` and the ready-made checkout, which says \"opens soon\" until they connect) or as a \
plausible placeholder marked in the code (`data-placeholder`, `TODO(person)`), never a visible \"[placeholder]\"; (3) the \
moment it arrives — equipe's note that it is connected, or their answer — finish the waiting piece at once, check it \
yourself, and tell the person in one line with the link: \"Stripe is connected: bookings now take payment. Here is the live \
page.\" Put a ticket down for the person only when the missing input truly gates everything on it.";

/// …and the PM, who writes the tickets and hands the answers on.
pub const RULE_PM: &str = "Never write a ticket that waits for the person (\"wait until they connect Stripe\", \"if it isn't \
ready, hand it back\"): write it to build the whole thing now, the missing piece behind its switch or a marked placeholder. \
When the person answers a fact card, hand each member on placeholders what they said, at once, so they swap them.";

/// The squad MCP server of equipe's platform tools on a hosted team's machine, which carries `ask_to_connect`.
pub const PLATFORM_SERVER: &str = "equipe-platform";

/// Outside business accounts a hosted team connects through `ask_to_connect`
/// (equipe-platform's Connections hub), by the names people and members
/// write them in any language: brands, not words.
const ACCOUNTS: &[&str] = &[
    "stripe", "paypal", "calendly", "google calendar", "google business", "search console", "instagram", "facebook", "tiktok", "linkedin",
    "threads", "google ads", "meta ads", "mailchimp", "posthog",
];

/// The account an ask names, if it is one a hosted team connects with a card.
fn account_in(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    ACCOUNTS.iter().copied().find(|a| lower.contains(a))
}

/// `block_on_person`'s asks that are connecting an outside business account
/// where the squad has `ask_to_connect`: refused back to the member with how
/// to go on instead. `None` when nothing in them names such an account.
pub fn account_ask(needs: &[String], ask: Option<&str>) -> Option<String> {
    let found = needs.iter().map(String::as_str).chain(ask).find_map(account_in)?;
    let name = found.split(' ').map(|w| { let mut c = w.chars(); c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default() }).collect::<Vec<_>>().join(" ");
    Some(format!(
        "Connecting {name} doesn't put the work down. Call `ask_to_connect` (key from `connections_hub`, with one plain line of \
why): one Connect card for the person, in the chat and in \"Waiting on you\". Then build everything else now, the piece that needs \
{name} behind its switch — for payments, `enable_payments` and the ready-made checkout, which says \"opens soon\" and works by \
itself once they connect — and hand in what you built. equipe tells you the moment it is connected: finish it, check it, and tell \
the person in one line. Never ask for keys, passwords or codes for it."
    ))
}

/// One ticket whose member works on placeholders for facts only the person knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnPlaceholders {
    pub ticket: TicketId,
    pub title: String,
    /// Its member, when it has one.
    pub owner: Option<String>,
    /// The facts it asked for, as the member put them.
    pub facts: Vec<String>,
}

/// The ticket event that says a member is on placeholders for facts (written
/// where the PM is handed the question, `runtime/decide.rs`).
pub const NEEDS_A_FACT: &str = "needs a fact";
/// The ticket event that says the person's answers were handed on.
pub const FACTS_ANSWERED: &str = "facts answered";

/// Whether a ticket's record, oldest first as `(kind, text)`, still has
/// facts waiting since it was last handed answers: those facts.
pub fn facts_open<'a>(record: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<String> {
    let mut open: Vec<String> = Vec::new();
    for (kind, text) in record {
        match kind {
            NEEDS_A_FACT => open.extend(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned)),
            FACTS_ANSWERED => open.clear(),
            _ => {}
        }
    }
    open.dedup();
    open
}

/// What the PM reads under the person's answers to its card: who works on
/// placeholders and for what, and to hand it on now. `None` when nobody is.
pub fn placeholders_line(waiting: &[OnPlaceholders]) -> Option<String> {
    if waiting.is_empty() {
        return None;
    }
    let each: Vec<String> = waiting
        .iter()
        .take(6)
        .map(|w| {
            let facts = w.facts.iter().take(4).map(|f| format!("\"{}\"", clip(f, 100))).collect::<Vec<_>>().join(", ");
            format!("- T-{} · {} ({}): {facts}", w.ticket, clip(&w.title, 80), w.owner.as_deref().unwrap_or("no member yet"))
        })
        .collect();
    Some(format!(
        "Members are working on placeholders for facts like these:\n{}\nHand each one what the person said now, with a `comment` on its \
ticket: they swap each placeholder for the real answer, check it on the page, and tell the person in one line once it shows. Anything the \
person didn't answer stays a placeholder — don't ask again for it.",
        each.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connecting_an_outside_account_is_refused_with_build_ahead_guidance() {
        let said = account_ask(&["Connect your Stripe account in Growth > Payments so customers can pay".into()], None).unwrap();
        assert!(said.contains("Connecting Stripe") && said.contains("ask_to_connect") && said.contains("enable_payments") && said.contains("opens soon"), "{said}");
        assert!(account_ask(&[], Some("Connect PayPal")).unwrap().contains("Connecting Paypal"));
        assert!(account_ask(&["Google Calendar 연결하기".into()], None).is_some(), "brands are found in any language");
        // Not an outside business account: left to the other checks.
        assert!(account_ask(&["Issue a GitHub token with repo scope".into()], None).is_none());
        assert!(account_ask(&["Share the 3 sheets with the service account".into()], Some("Share the sheets")).is_none());
    }

    #[test]
    fn facts_stay_open_until_answers_are_handed_on() {
        let record = [("created", ""), (NEEDS_A_FACT, "Real prices of the packages\nOpening hours"), ("comment", "x")];
        assert_eq!(facts_open(record), ["Real prices of the packages", "Opening hours"]);
        let record = [(NEEDS_A_FACT, "Opening hours"), (FACTS_ANSWERED, ""), ("comment", "x")];
        assert!(facts_open(record).is_empty());
        let record = [(NEEDS_A_FACT, "Opening hours"), (FACTS_ANSWERED, ""), (NEEDS_A_FACT, "The coach's surname")];
        assert_eq!(facts_open(record), ["The coach's surname"]);
    }

    #[test]
    fn the_pm_is_told_who_is_on_placeholders_and_to_hand_answers_on() {
        assert!(placeholders_line(&[]).is_none());
        let line = placeholders_line(&[OnPlaceholders { ticket: 41, title: "Let customers book and pay".into(), owner: Some("frontend".into()), facts: vec!["Real prices".into()] }]).unwrap();
        assert!(line.contains("T-41 · Let customers book and pay (frontend): \"Real prices\"") && line.contains("`comment`") && line.contains("don't ask again"), "{line}");
    }

    #[test]
    fn the_rule_names_the_three_steps_and_the_one_line() {
        for w in ["ask once", "build everything else now", "behind its switch", "placeholder", "one line", "truly gates everything"] {
            assert!(RULE.contains(w), "{w}");
        }
        assert!(RULE_PM.contains("Never write a ticket that waits for the person"));
    }
}
