//! Design sessions: the person and the designer working on one page or
//! screen together in short rounds — minutes, not hours — with the page live
//! beside the conversation, pins dropped on it, quick reactions, and the
//! person's taste written down as it shows.
//!
//! A session is the designer's own conversation, opened on a kickoff message
//! that names the page ([`crate::Command::StartDesignSession`]); what the
//! designer shows (`equipe design session show …`) and the pins it marks done
//! are read from that conversation, never guessed. The PM may propose one
//! first ([`SessionProposal`]), at most once per ticket or screen.

use serde::{Deserialize, Serialize};

use crate::TicketId;

/// Where a session was started from: the screen's button, or the PM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionFrom {
    /// The design system screen's "Start a design session".
    DesignScreen,
    /// A designer's ticket.
    Ticket { ticket: TicketId },
    /// The designer's row on the team.
    Member,
    /// The PM: the person took its proposal, or asked it for one.
    Pm {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ticket: Option<TicketId>,
    },
}

impl SessionFrom {
    /// The ticket it is about, if any.
    pub fn ticket(&self) -> Option<TicketId> {
        match self {
            SessionFrom::Ticket { ticket } => Some(*ticket),
            SessionFrom::Pm { ticket } => *ticket,
            SessionFrom::DesignScreen | SessionFrom::Member => None,
        }
    }
}

/// The PM's proposal of a session, before work goes to the designer alone:
/// what it is about and why, in a line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionProposal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    /// The page or screen: "Pricing page".
    pub screen: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// One line on why now.
    pub why: String,
}

/// What the person did with a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalOutcome {
    /// "Start a 20–30 min session".
    Started,
    /// "Not now": not proposed again for this ticket or screen.
    NotNow,
    /// "Let the designer go ahead": the designer works alone, as before.
    GoAhead,
}

/// The longest a page's name may be, in characters.
pub const PAGE_MAX: usize = 120;

/// A page or screen's name as a session keeps it: one line, its runs of
/// space made one, at most [`PAGE_MAX`] characters; `None` when nothing is left.
pub fn page_name(raw: &str) -> Option<String> {
    let one = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.is_empty() {
        return None;
    }
    Some(if one.chars().count() > PAGE_MAX { one.chars().take(PAGE_MAX).collect::<String>().trim_end().to_owned() } else { one })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_a_session_started_goes_and_comes_back_as_it_was() {
        for from in [SessionFrom::DesignScreen, SessionFrom::Ticket { ticket: 12 }, SessionFrom::Member, SessionFrom::Pm { ticket: None }, SessionFrom::Pm { ticket: Some(4) }] {
            let wire = serde_json::to_string(&from).unwrap();
            assert_eq!(serde_json::from_str::<SessionFrom>(&wire).unwrap(), from, "{wire}");
        }
        assert_eq!(serde_json::to_string(&SessionFrom::Pm { ticket: None }).unwrap(), r#"{"kind":"pm"}"#);
        assert_eq!(SessionFrom::Ticket { ticket: 3 }.ticket(), Some(3));
        assert_eq!(SessionFrom::Member.ticket(), None);
        let proposal = SessionProposal { ticket: Some(7), screen: "Pricing page".into(), project: Some("web".into()), why: "A new screen".into() };
        assert_eq!(serde_json::from_str::<SessionProposal>(&serde_json::to_string(&proposal).unwrap()).unwrap(), proposal);
        assert_eq!(serde_json::to_string(&ProposalOutcome::NotNow).unwrap(), r#""not_now""#);
    }

    #[test]
    fn a_page_name_is_one_short_line() {
        assert_eq!(page_name("  Pricing\n page "), Some("Pricing page".into()));
        assert_eq!(page_name(" \n "), None);
        assert_eq!(page_name(&"a".repeat(300)).map(|p| p.chars().count()), Some(PAGE_MAX));
    }
}
