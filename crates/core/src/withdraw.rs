//! `withdraw_card`: taking a card that waits on the person away when it is no
//! longer needed — a secret or key request, a sign-in, an `ask_person`, a
//! decision, a proposal to accept. Saying "ignore that card" leaves it in
//! front of the person; only taking it away removes it.
//!
//! A member takes back its own cards; the PM any open card on its squad. The
//! one-line `why` is what the person reads where the card was. equipe takes
//! a ticket's secret, sign-in and `ask_person` cards away by itself when the
//! ticket's approach changes (re-planned, restarted, sent back) or it closes
//! (`runtime::withdraw`), and reminds the PM once when its reply says a card
//! is withdrawn or to be ignored while that card is still up ([`says_withdrawn`]).
//!
//! The rules here are pure — the tool's definition, the words read in a
//! reply, how a card is named — so they are tested without a daemon.

use equipe_protocol::TicketId;
use serde_json::{Value, json};

pub const WITHDRAW_CARD: &str = "withdraw_card";

/// The longest a card's question or reason is quoted in a line.
pub const QUOTED: usize = 100;

/// The most cards a listing or a reminder names; the rest are counted.
pub const LISTED: usize = 8;

/// What kind of card, as `withdraw_card` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// A secret or key request (`ask_secret`, `request_secret`).
    Secret,
    /// A sign-in: a command-line sign-in card (`cli_sign_in`) or a site's (`sign_in`).
    SignIn,
    /// A yes asked before acting in the person's name (`ask_person`).
    Ask,
    /// Questions or a decision (`ask_decision`).
    Decision,
    /// A proposal to accept, merge or send back work (`propose_accept`).
    Accept,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Secret, Kind::SignIn, Kind::Ask, Kind::Decision, Kind::Accept];

    /// Its word in the tool's `kind`.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Secret => "secret",
            Kind::SignIn => "sign_in",
            Kind::Ask => "ask",
            Kind::Decision => "decision",
            Kind::Accept => "accept",
        }
    }

    /// The kind a word names: its own word, or one a model is likely to write for it.
    pub fn parse(word: &str) -> Option<Kind> {
        match word.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "secret" | "key" | "token" | "secret_request" | "request_secret" | "ask_secret" => Some(Kind::Secret),
            "sign_in" | "signin" | "login" | "cli_sign_in" => Some(Kind::SignIn),
            "ask" | "ask_person" | "confirm" => Some(Kind::Ask),
            "decision" | "question" | "questions" | "ask_decision" => Some(Kind::Decision),
            "accept" | "propose_accept" | "acceptance" => Some(Kind::Accept),
            _ => None,
        }
    }

    /// The kinds equipe takes away by itself when a ticket's approach changes
    /// or it closes: those that hold a member up or ask for something under
    /// the old plan. A decision about a closed ticket goes on its own
    /// (`runtime::settled`); a proposal to accept when its tickets settle.
    pub fn follows_its_ticket(self) -> bool {
        matches!(self, Kind::Secret | Kind::SignIn | Kind::Ask)
    }
}

/// One card open for the person, as the tool lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub kind: Kind,
    pub id: u64,
    /// The member that raised it.
    pub member: String,
    pub ticket: Option<TicketId>,
    /// What it asks, in a line.
    pub what: String,
}

impl Card {
    /// How it is told to a model: `secret 1731990000123 by frontend on T-3899: "Vercel deploy token"`.
    pub fn line(&self) -> String {
        let on = self.ticket.map(|t| format!(" on T-{t}")).unwrap_or_default();
        format!("{} {} by {}{on}: \"{}\"", self.kind.word(), self.id, self.member, self.what)
    }

    /// Whether `reply` points at it: its ticket's number, or its member by name.
    pub fn named_in(&self, reply: &str) -> bool {
        let ticket = self.ticket.is_some_and(|t| {
            let mut from = 0;
            while let Some((at, len)) = crate::board::ticket_ref_from(reply, from) {
                if crate::board::parse_ticket_id(&reply[at..at + len]) == Some(t) {
                    return true;
                }
                from = at + len;
            }
            false
        });
        ticket || (!self.member.is_empty() && reply.to_lowercase().contains(&self.member.to_lowercase()))
    }
}

/// The cards `cards` as lines, at most [`LISTED`] of them, the rest counted.
pub fn listing(cards: &[Card]) -> String {
    let mut lines: Vec<String> = cards.iter().take(LISTED).map(|c| format!("- {}", c.line())).collect();
    if cards.len() > LISTED {
        lines.push(format!("- … and {} more (narrow with `kind` or `ticket`)", cards.len() - LISTED));
    }
    lines.join("\n")
}

/// Whether a reply tells the person that a card is withdrawn, not needed or
/// to be ignored — in any language equipe speaks: a word for a card and a
/// word for taking it back, both (`match.card_words`, `match.withdraw_words`).
pub fn says_withdrawn(reply: &str) -> bool {
    let lower = reply.to_lowercase();
    let any = |key: &str| crate::i18n::every(key).iter().any(|w| lower.contains(&w.to_lowercase()));
    any("match.card_words") && any("match.withdraw_words")
}

/// What the PM is told at its next turn when its reply said a card was
/// withdrawn or to be ignored and `open` are still in front of the person.
pub fn reminder(open: &[Card]) -> String {
    format!(
        "Your last reply told the person a card is withdrawn, not needed or to be ignored — but it is still in front of them:\n{}\n\
Saying so doesn't take it away. Call `withdraw_card` for each that is moot (its `kind` and `id`, or its `ticket`), with a one-line \
`why` they read; if one is still needed after all, tell them so in a line.",
        listing(open)
    )
}

/// The tool, as an MCP tool definition: the PM's takes back any open card on
/// the squad, a member's only its own.
pub fn tool(is_pm: bool) -> Value {
    let whose = if is_pm {
        "any card open for the person on the squad — a member's secret or key request, sign-in, `ask_person`, decision, or your own \
proposal to accept"
    } else {
        "your own cards still open for the person — a secret or key request, a sign-in, an `ask_person`, a decision"
    };
    json!({
        "name": WITHDRAW_CARD,
        "description": format!(
            "Take away {whose} — when it is moot: the approach changed, it was asked the old way, the person's words settled it. \
Telling the person to ignore a card leaves it in front of them; this removes it, and they read your `why` in one line where it \
was. Never tell them to ignore a card that is still up: withdraw it first. Name one by `kind` and `id`, or all of a ticket's with \
`ticket` (and `kind` to narrow); with neither, it lists the open cards you may withdraw. The member that raised it is told. When \
something is still theirs to choose, ask anew with the options as they stand now."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": Kind::ALL.iter().map(|k| k.word()).collect::<Vec<_>>(), "description": "What it is: secret (a secret or key request), sign_in, ask (ask_person), decision, or accept (a proposal to accept)." },
                "id": { "type": "integer", "description": "The card's number, as the listing (or the note of your open cards) gives it." },
                "ticket": { "type": "integer", "description": "Take away the open cards about this ticket." },
                "why": { "type": "string", "description": "Why it is no longer needed, in one line in the person's language — shown to them where the card was." },
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(kind: Kind, ticket: Option<TicketId>) -> Card {
        Card { kind, id: 17, member: "frontend".into(), ticket, what: "Add your Vercel deploy token".into() }
    }

    #[test]
    fn a_kind_is_read_from_its_word_or_a_likely_one_and_only_some_follow_their_ticket() {
        for kind in Kind::ALL {
            assert_eq!(Kind::parse(kind.word()), Some(kind));
        }
        assert_eq!(Kind::parse("request_secret"), Some(Kind::Secret));
        assert_eq!(Kind::parse("Sign-in"), Some(Kind::SignIn));
        assert_eq!(Kind::parse("ask_person"), Some(Kind::Ask));
        assert_eq!(Kind::parse("question"), Some(Kind::Decision));
        assert_eq!(Kind::parse("permission"), None);
        assert!(Kind::Secret.follows_its_ticket() && Kind::SignIn.follows_its_ticket() && Kind::Ask.follows_its_ticket());
        assert!(!Kind::Decision.follows_its_ticket() && !Kind::Accept.follows_its_ticket());
    }

    #[test]
    fn a_card_is_named_with_its_kind_id_member_and_ticket_and_a_long_list_is_counted() {
        assert_eq!(card(Kind::Secret, Some(3899)).line(), "secret 17 by frontend on T-3899: \"Add your Vercel deploy token\"");
        assert_eq!(card(Kind::Decision, None).line(), "decision 17 by frontend: \"Add your Vercel deploy token\"");
        let many: Vec<Card> = (0..LISTED + 3).map(|i| Card { id: i as u64, ..card(Kind::Ask, None) }).collect();
        let listed = listing(&many);
        assert_eq!(listed.lines().count(), LISTED + 1, "{listed}");
        assert!(listed.ends_with("… and 3 more (narrow with `kind` or `ticket`)"), "{listed}");
    }

    #[test]
    fn a_reply_points_at_a_card_by_its_ticket_or_its_member_never_by_a_number_inside_another() {
        let on = card(Kind::Secret, Some(3899));
        assert!(on.named_in("The token card on T-3899 isn't needed."));
        assert!(on.named_in("Frontend asked the old way."));
        let other = Card { member: "backend".into(), ..on.clone() };
        assert!(!other.named_in("T-38990 and MAT-3899 aren't it."));
        assert!(other.named_in("(T-3899)"));
    }

    #[test]
    fn a_reply_that_says_a_card_is_withdrawn_or_to_be_ignored_is_told_from_one_that_doesnt_in_every_language() {
        for said in [
            "Please ignore the token card — frontend asked the old way.",
            "I withdrew the card: none of it is needed now.",
            "The card is no longer needed.",
            "토큰 카드는 무시해 주세요. 이제 필요 없어서 거뒀어요.",
            "トークンのカードは無視してください。",
            "请忽略令牌卡片。",
            "Bitte ignoriere die Token-Karte.",
            "Ignora la tarjeta del token.",
            "Ignorez la carte du jeton.",
        ] {
            assert!(says_withdrawn(said), "{said}");
        }
        for said in ["The token card is up: paste it there when you can.", "I withdrew T-4 from the plan.", "Done — merged T-12."] {
            assert!(!says_withdrawn(said), "{said}");
        }
    }

    #[test]
    fn the_reminder_lists_what_is_still_up_and_says_how_to_take_it_away() {
        let text = reminder(&[card(Kind::Secret, Some(3899))]);
        assert!(text.contains("secret 17 by frontend on T-3899") && text.contains("`withdraw_card`") && text.contains("doesn't take it away"), "{text}");
    }

    #[test]
    fn the_pms_tool_reaches_every_card_on_the_squad_and_a_members_only_its_own() {
        let pm = tool(true);
        let member = tool(false);
        assert_eq!(pm["name"], WITHDRAW_CARD);
        assert!(pm["description"].as_str().unwrap().contains("any card open for the person on the squad"));
        assert!(member["description"].as_str().unwrap().contains("your own cards"));
        for t in [&pm, &member] {
            let d = t["description"].as_str().unwrap();
            assert!(d.contains("Never tell them to ignore a card that is still up"), "{d}");
            let kinds = t["inputSchema"]["properties"]["kind"]["enum"].as_array().unwrap();
            assert_eq!(kinds.len(), Kind::ALL.len());
        }
    }
}
