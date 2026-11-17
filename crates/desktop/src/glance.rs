//! What a ticket is called where the person scans a list: the line that says
//! what they must do when it waits on them, else its title cut cleanly — and
//! the small chip for the sort of work. No UI types here, so the rules are
//! tested without a window.

use equipe_core::protocol::Ticket;
use equipe_core::protocol::title;

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
