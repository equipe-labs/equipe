//! What a ticket is called where the person scans a list: the line that says
//! what they must do when it waits on them, else its title cut cleanly — and
//! the small chip for the sort of work. No UI types here, so the rules are
//! tested without a window.

use equipe_core::protocol::Ticket;
use equipe_core::protocol::title;
use equipe_core::t;

/// The longest a title is drawn in a list before it is cut at a word; the
/// row's own width cuts it anyway on a narrow screen. Titles written to the
/// rule are far shorter.
pub const LIST_CHARS: usize = 64;

/// A ticket's name in a row or a header: `main` is what to read first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub main: String,
    /// The ticket's own title, smaller below, when `main` is its ask.
    pub below: Option<String>,
}

/// While `ticket` waits on the person: their ask ("Share the sheets · 1 min")
/// above its title; otherwise its title alone.
pub fn heading(ticket: &Ticket) -> Heading {
    match ticket.ask_headline() {
        Some(ask) => Heading { main: ask, below: Some(title::cut(&ticket.title, LIST_CHARS)) },
        None => Heading { main: title::cut(&ticket.title, LIST_CHARS), below: None },
    }
}

/// At most this many of a waiting ticket's asks are named under it; the
/// rest are counted.
pub const ASKS_SHOWN: usize = 3;

/// What Needs you says under a ticket, in the person's language: while it
/// waits on them, each open item of its checklist in plain words; how long
/// it has waited (`waited`, already in words); the request of theirs it
/// holds up — then what the heading put below. `None` with nothing to say.
pub fn needs_below(ticket: &Ticket, heading_below: Option<&str>, waited: Option<&str>, request: Option<(u64, &str)>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if ticket.status == equipe_core::protocol::TicketStatus::Waiting {
        let asks = ticket.asks();
        parts.extend(asks.iter().take(ASKS_SHOWN).map(|a| format!("☐ {}", title::cut(a.lines().next().unwrap_or_default(), 40))));
        if asks.len() > ASKS_SHOWN {
            parts.push(format!("+{}", asks.len() - ASKS_SHOWN));
        }
    }
    if let Some(waited) = waited.filter(|w| !w.is_empty()) {
        parts.push(t!("needs.waited", span = waited).to_string());
    }
    if let Some((id, name)) = request {
        parts.push(t!("needs.holds_request", id = id, title = title::cut(name, 40)).to_string());
    }
    parts.extend(heading_below.map(str::to_owned));
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// The word on a ticket's chip, in the person's language.
pub fn chip(ticket: &Ticket, lang: equipe_core::i18n::Lang) -> Option<&'static str> {
    ticket.label.map(|label| equipe_core::i18n::label::ticket_label(lang, label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::TicketStatus;
    use equipe_core::protocol::asks::waiting_note_headed;

    fn ticket(title: &str) -> Ticket {
        serde_json::from_value(serde_json::json!({ "id": 7, "title": title, "body": "", "status": "waiting", "depends_on": [], "created_by": "pm", "rank": 0 })).unwrap()
    }

    #[test]
    fn a_ticket_waiting_on_the_person_is_read_by_its_ask_with_its_title_below() {
        let mut t = ticket("Sync the exit-plan sheets every night");
        t.note = Some(waiting_note_headed(&["Share each sheet.".to_owned()], None, Some("구글시트 3개를 서비스 계정에 공유 · 1분")));
        assert_eq!(heading(&t), Heading { main: "구글시트 3개를 서비스 계정에 공유 · 1분".into(), below: Some("Sync the exit-plan sheets every night".into()) });
        t.status = TicketStatus::InProgress;
        assert_eq!(heading(&t), Heading { main: "Sync the exit-plan sheets every night".into(), below: None }, "only while it waits");
    }

    #[test]
    fn needs_you_says_what_a_wait_asks_how_long_and_which_request_it_holds_up() {
        let mut t = ticket("Infra: AWS console access");
        t.note = Some(waiting_note_headed(&["Sign in to AWS in the browser card".to_owned(), "Say which region".to_owned()], None, Some("Sign in to AWS")));
        let below = needs_below(&t, Some("Infra: AWS console access"), Some("5h"), Some((3, "Deploy the ERP"))).unwrap();
        assert_eq!(below, "☐ Sign in to AWS in the browser card · ☐ Say which region · waiting 5h · holds up R-3 “Deploy the ERP” · Infra: AWS console access");
        t.asks = (1..=5).map(|i| format!("Step {i}")).collect();
        assert!(needs_below(&t, None, None, None).unwrap().ends_with("☐ Step 3 · +2"));
        // Not waiting on them: no checklist, just what else there is to say.
        t.status = TicketStatus::Review;
        assert_eq!(needs_below(&t, None, Some("2h"), None).as_deref(), Some("waiting 2h"));
        assert_eq!(needs_below(&t, None, None, None), None);
    }

    #[test]
    fn a_waiting_ticket_without_an_ask_shows_its_title() {
        let mut t = ticket("Connect the payment test key");
        t.note = Some("Waiting on you:\n- [ ] Make a key".into());
        assert_eq!(heading(&t), Heading { main: "Connect the payment test key".into(), below: None });
    }

    #[test]
    fn a_long_old_title_is_cut_at_a_word() {
        let t = ticket("Nightly business plan recovery plan sync workflow failing two nights running because of an ECS task");
        let shown = heading(&t).main;
        assert!(shown.ends_with('…') && shown.chars().count() <= LIST_CHARS, "{shown}");
        assert!(!shown.contains("ECS ta"), "{shown}");
    }

    #[test]
    fn the_chip_is_the_label_in_the_persons_language() {
        use equipe_core::i18n::Lang;
        use equipe_core::protocol::title::TicketLabel;
        let mut t = ticket("Check the forms");
        assert_eq!(chip(&t, Lang::En), None);
        t.label = Some(TicketLabel::Check);
        assert_eq!(chip(&t, Lang::En), Some("Check"));
        assert_eq!(chip(&t, Lang::Ko), Some("확인"));
    }
}
