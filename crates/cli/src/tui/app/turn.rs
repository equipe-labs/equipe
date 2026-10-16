//! A ticket put down for the person (`block_on_person`), answered in the
//! terminal ask by ask — opened with `/answer T-n`, or r on the board. While
//! nothing is typed, single keys act on the line marked: on a thing to do, d
//! 했음, y 대신 해 줘 (the member does it), l 나중에 (it stays open); on a
//! question, a (or Enter, or any other character) writes the answer, y 네가
//! 정해 줘, ? 모르겠어요; n a note on the line; the last line, the words to the
//! member. ↑↓ move, s sends what is set (what isn't goes as not yet), Esc
//! puts the card away and keeps it as it was.

use std::collections::HashMap;

use crossterm::event::KeyCode;
use equipe_core::protocol::asks::{self, Pick, Want};
use equipe_core::protocol::{Command, TicketId, TicketStatus};
use equipe_core::t;

use super::App;

/// What the composer holds words for, while the person types them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Typing {
    Answer(usize),
    Note(usize),
    Free,
}

/// A waiting ticket's asks, as the person answers them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnForm {
    pub ticket: TicketId,
    pub asks: Vec<String>,
    pub wants: Vec<Want>,
    /// The line the keys act on: an ask, or (`asks.len()`) the words to the member.
    pub at: usize,
    pub picks: HashMap<usize, Pick>,
    pub typed: Vec<String>,
    pub notes: Vec<String>,
    pub free: String,
    pub typing: Option<Typing>,
}

impl TurnForm {
    pub fn new(ticket: TicketId, asks: Vec<String>) -> Self {
        let n = asks.len();
        let wants = asks.iter().map(|a| asks::want_of(a)).collect();
        Self { ticket, asks, wants, at: 0, picks: HashMap::new(), typed: vec![String::new(); n], notes: vec![String::new(); n], free: String::new(), typing: None }
    }

    /// Everything set, as it is sent.
    pub fn answers(&self) -> asks::Answers {
        asks::answers(&self.wants, &self.typed, &self.picks, &self.notes, &self.free)
    }

    /// Asks with nothing set.
    pub fn left(&self) -> usize {
        self.answers().replies.iter().filter(|r| r.is_empty()).count()
    }

    /// The pick a key means on line `n`: d, y, l on a thing to do; y, ? on a question.
    fn pick_of(&self, n: usize, key: char) -> Option<Pick> {
        match (self.wants.get(n)?, key) {
            (Want::Do, 'd') => Some(Pick::Done),
            (Want::Do, 'y') => Some(Pick::Yours),
            (Want::Do, 'l') => Some(Pick::Later),
            (Want::Answer | Want::Confirm, 'y') => Some(Pick::YourCall),
            (Want::Answer | Want::Confirm, '?') => Some(Pick::DontKnow),
            _ => None,
        }
    }

    /// The words held for what is being typed.
    fn held(&self, typing: Typing) -> &str {
        match typing {
            Typing::Answer(n) => self.typed.get(n).map_or("", String::as_str),
            Typing::Note(n) => self.notes.get(n).map_or("", String::as_str),
            Typing::Free => &self.free,
        }
    }

    fn keep(&mut self, typing: Typing, words: String) {
        match typing {
            Typing::Answer(n) => {
                // Words answer it: a pick standing in for them goes.
                if !words.trim().is_empty() {
                    self.picks.remove(&n);
                }
                if let Some(t) = self.typed.get_mut(n) {
                    *t = words;
                }
            }
            Typing::Note(n) => {
                if let Some(t) = self.notes.get_mut(n) {
                    *t = words;
                }
            }
            Typing::Free => self.free = words,
        }
    }
}

impl App {
    /// The card for ticket `id`, as it was left — or new.
    pub(super) fn open_turn(&mut self, id: TicketId) -> bool {
        let Some(t) = self.tickets.iter().find(|t| t.id == id && t.status == TicketStatus::Waiting) else {
            self.flash = Some(t!("app.turn_not_waiting", id = id));
            return false;
        };
        let asks = t.asks();
        if asks.is_empty() {
            return false;
        }
        if let Some(open) = self.turn_form.take() {
            self.turn_drafts.insert(open.ticket, open);
        }
        let form = self.turn_drafts.remove(&id).filter(|f| f.asks == asks).unwrap_or_else(|| TurnForm::new(id, asks));
        self.turn_form = Some(form);
        self.flash = Some(t!("app.turn_opened", id = id));
        true
    }

    /// A key while a card is open. `None`: not its key (it goes to the composer).
    pub(super) fn on_turn_key(&mut self, code: KeyCode, ctrl: bool, alt: bool) -> Option<Vec<Command>> {
        let form = self.turn_form.as_ref()?;
        // Answered elsewhere, or no longer waiting: the card goes.
        if !self.tickets.iter().any(|t| t.id == form.ticket && t.status == TicketStatus::Waiting) {
            self.turn_form = None;
            return None;
        }
        if ctrl || alt {
            return None;
        }
        if let Some(typing) = form.typing {
            return match code {
                KeyCode::Enter => {
                    let words = self.composer.text().trim().to_owned();
                    self.composer.clear();
                    let form = self.turn_form.as_mut()?;
                    form.keep(typing, words);
                    form.typing = None;
                    Some(Vec::new())
                }
                KeyCode::Esc => {
                    self.composer.clear();
                    self.turn_form.as_mut()?.typing = None;
                    Some(Vec::new())
                }
                _ => None,
            };
        }
        if !self.composer.is_empty() {
            return None;
        }
        let form = self.turn_form.as_mut()?;
        let lines = form.asks.len() + 1;
        let on_ask = form.at < form.asks.len();
        let words_here = !on_ask || form.wants[form.at] != Want::Do;
        let write = |form: &mut TurnForm, composer: &mut super::Composer, first: Option<char>| {
            let typing = if form.at < form.asks.len() { Typing::Answer(form.at) } else { Typing::Free };
            form.typing = Some(typing);
            composer.insert_str(form.held(typing));
            if let Some(c) = first {
                composer.insert_str(&c.to_string());
            }
        };
        match code {
            KeyCode::Up | KeyCode::BackTab => form.at = (form.at + lines - 1) % lines,
            KeyCode::Down | KeyCode::Tab => form.at = (form.at + 1) % lines,
            KeyCode::Esc => {
                let form = self.turn_form.take()?;
                self.turn_drafts.insert(form.ticket, form);
                self.flash = Some(t!("app.turn_kept").into());
            }
            KeyCode::Char('s') => return Some(self.turn_send()),
            KeyCode::Char('n') if on_ask => {
                let n = form.at;
                form.typing = Some(Typing::Note(n));
                let held = form.notes[n].clone();
                self.composer.insert_str(&held);
            }
            KeyCode::Char(c) if on_ask && form.pick_of(form.at, c).is_some() => {
                let pick = form.pick_of(form.at, c)?;
                asks::toggle(&mut form.picks, form.at, pick);
            }
            KeyCode::Enter | KeyCode::Char('a') if words_here => write(form, &mut self.composer, None),
            // Any other character on a line with words starts them.
            KeyCode::Char(c) if words_here && !c.is_control() => write(form, &mut self.composer, Some(c)),
            _ => {}
        }
        Some(Vec::new())
    }

    /// s: what is set goes to the member in one message, and the card goes.
    fn turn_send(&mut self) -> Vec<Command> {
        let Some(form) = self.turn_form.as_ref() else { return Vec::new() };
        let answers = form.answers();
        if !asks::can_send(&answers) {
            self.flash = Some(t!("app.turn_nothing_set").into());
            return Vec::new();
        }
        let note = asks::answer_message(&form.asks, &answers);
        let ticket = form.ticket;
        let name = self.tickets.iter().find(|t| t.id == ticket).and_then(|t| t.assignee.clone().or(t.owner.clone())).unwrap_or_default();
        self.turn_form = None;
        self.turn_drafts.remove(&ticket);
        self.flash = Some(t!("app.turn_sent", id = ticket, name = name));
        vec![Command::Resume { squad: self.squad.clone(), ticket, note }]
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{enter, team, ticket, type_text};
    use super::*;
    use crossterm::event::KeyEvent;
    use equipe_core::protocol::Event;
    use std::time::Instant;

    fn press(app: &mut App, code: KeyCode) -> Vec<Command> {
        app.on_key(KeyEvent::from(code), Instant::now())
    }

    fn chars(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    /// T-117 as the person saw it: a question and a thing to do.
    fn waiting_app() -> App {
        let (mut app, _, _) = team();
        let mut t = ticket(117, TicketStatus::Waiting);
        t.asks = vec!["App Review 연락처를 알려 주세요: 전화번호와 이메일.".into(), "App Store Connect에서 EU DSA 상태를 trader로 바꿔 주세요.".into()];
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        app
    }

    fn sent(commands: &[Command]) -> asks::Read {
        match commands {
            [Command::Resume { ticket: 117, note, .. }] => asks::read(note).unwrap_or_else(|| panic!("{note}")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn letters_pick_one_at_a_time_and_a_writes_the_answer() {
        let mut app = waiting_app();
        type_text(&mut app, "/answer T-117");
        assert!(enter(&mut app).is_empty());
        assert!(app.turn_form.is_some(), "the card opens");
        // The question: a writes, Enter keeps it.
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "+82 10 1234 5678");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.turn_form.as_ref().unwrap().typed[0], "+82 10 1234 5678");
        assert!(app.composer.is_empty());
        // The thing to do: d, then y — one at a time; y again takes it back.
        press(&mut app, KeyCode::Down);
        chars(&mut app, "dy");
        assert_eq!(app.turn_form.as_ref().unwrap().picks.get(&1), Some(&Pick::Yours));
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.turn_form.as_ref().unwrap().picks.get(&1), None);
        chars(&mut app, "y");
        // n: a note on it.
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "2FA는 제 폰으로 와요");
        press(&mut app, KeyCode::Enter);
        // The last line: words to the member, started by typing.
        press(&mut app, KeyCode::Down);
        chars(&mut app, "급");
        type_text(&mut app, "하지 않아요");
        press(&mut app, KeyCode::Enter);
        let read = sent(&press(&mut app, KeyCode::Char('s')));
        assert_eq!(read.items[0].reply, asks::Reply::Text("+82 10 1234 5678".into()));
        assert_eq!((read.items[1].reply.clone(), read.items[1].note.as_str()), (asks::Reply::Yours, "2FA는 제 폰으로 와요"));
        assert_eq!(read.free, "급하지 않아요");
        assert!(app.turn_form.is_none(), "sent: the card goes");
    }

    #[test]
    fn a_question_is_left_to_the_member_or_not_known_and_later_stays_open() {
        let mut app = waiting_app();
        assert!(app.open_turn(117));
        chars(&mut app, "?");
        assert_eq!(app.turn_form.as_ref().unwrap().picks.get(&0), Some(&Pick::DontKnow));
        chars(&mut app, "y");
        assert_eq!(app.turn_form.as_ref().unwrap().picks.get(&0), Some(&Pick::YourCall), "one at a time");
        press(&mut app, KeyCode::Down);
        chars(&mut app, "l");
        let read = sent(&press(&mut app, KeyCode::Char('s')));
        assert_eq!(read.items.iter().map(|i| i.reply.clone()).collect::<Vec<_>>(), [asks::Reply::YourCall, asks::Reply::Later]);
        assert_eq!(read.open(), 1);
    }

    #[test]
    fn partly_or_words_alone_go_and_nothing_set_does_not() {
        let mut app = waiting_app();
        assert!(app.open_turn(117));
        assert!(press(&mut app, KeyCode::Char('s')).is_empty(), "nothing set yet");
        assert!(app.flash.is_some() && app.turn_form.is_some());
        // Only words to the member.
        press(&mut app, KeyCode::Up);
        assert_eq!(app.turn_form.as_ref().unwrap().at, 2, "↑ from the top wraps to the last line");
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, "이건 너가 직접 해");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.turn_form.as_ref().unwrap().left(), 2);
        let read = sent(&press(&mut app, KeyCode::Char('s')));
        assert_eq!((read.answered(), read.open(), read.free.as_str()), (0, 2, "이건 너가 직접 해"));
    }

    #[test]
    fn esc_puts_the_card_away_as_it_was_and_r_on_the_board_brings_it_back() {
        let mut app = waiting_app();
        assert!(app.open_turn(117));
        press(&mut app, KeyCode::Down);
        chars(&mut app, "d");
        // Typing, Esc lets the words go but not the card.
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "x");
        press(&mut app, KeyCode::Esc);
        assert!(app.turn_form.as_ref().is_some_and(|f| f.typing.is_none() && f.notes[1].is_empty()));
        press(&mut app, KeyCode::Esc);
        assert!(app.turn_form.is_none());
        app.pane = super::super::Pane::Board;
        while app.selected_ticket().map(|t| t.id) != Some(117) {
            press(&mut app, KeyCode::Down);
        }
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.turn_form.as_ref().map(|f| f.picks.get(&1).copied()), Some(Some(Pick::Done)), "as it was left");
        // Answered elsewhere meanwhile: the card goes with the next key.
        let mut done = ticket(117, TicketStatus::Todo);
        done.title = "x".into();
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: done });
        press(&mut app, KeyCode::Char('d'));
        assert!(app.turn_form.is_none());
    }
}
