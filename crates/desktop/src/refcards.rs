//! Reference cards in the conversation (`show_references`): a member's
//! gallery of components, screens and patterns, as the person reacts to it —
//! a heart, "not this" with its one-tap reasons, kept among the project's
//! references. No UI types here, so the rules are tested without a window;
//! `views/refcards.rs` draws the card.
//!
//! What the person does tells their taste, and goes to the daemon as the
//! taste signals it already keeps (`Command::TasteSignals`): a heart as a
//! reaction up with the picture, "not this" as one down, each reason as a
//! rejection with that reason. Taking a heart back sends nothing — a taste
//! once shown stays shown. Keeping a card writes its link and lines into the
//! references (`Command::KeepReference`); its picture never goes there.

use std::collections::HashMap;

use equipe_core::protocol::design::{PickReason, QuickReaction, TasteSignal};
use equipe_core::protocol::references::RefCard;
use equipe_core::protocol::{Command, SessionId};

/// A gallery as its note holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gallery {
    /// The member who showed it.
    pub name: String,
    /// What the person's reactions are kept by (`equipe_core::references::key`).
    pub key: String,
    pub cards: Vec<RefCard>,
}

impl Gallery {
    /// The sites its cards are from, each once, in order.
    pub fn sites(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for card in &self.cards {
            if !card.site.is_empty() && !out.contains(&card.site) {
                out.push(card.site.clone());
            }
        }
        out
    }
}

/// What the person did to one card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reaction {
    pub love: bool,
    pub not: bool,
    /// Why not, in a tap each.
    pub reasons: Vec<PickReason>,
    /// Kept among the project's references.
    pub kept: bool,
}

/// What the person did to each card, by its gallery's key and its place in it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reactions(HashMap<(String, usize), Reaction>);

/// How a card is named in the person's taste: its title and where it is from.
pub fn label(card: &RefCard) -> String {
    if card.site.is_empty() { card.title.clone() } else { format!("{} ({})", card.title, card.site) }
}

fn taste(squad: &str, signal: TasteSignal) -> Command {
    Command::TasteSignals { squad: squad.to_owned(), project: None, signals: vec![signal] }
}

impl Reactions {
    /// What the person did to card `n` of gallery `key` so far.
    pub fn of(&self, key: &str, n: usize) -> Reaction {
        self.0.get(&(key.to_owned(), n)).cloned().unwrap_or_default()
    }

    fn entry(&mut self, key: &str, n: usize) -> &mut Reaction {
        self.0.entry((key.to_owned(), n)).or_default()
    }

    /// A heart, given or taken back: given, "not this" goes and the taste is told.
    pub fn love(&mut self, squad: &str, key: &str, n: usize, card: &RefCard) -> Option<Command> {
        let r = self.entry(key, n);
        r.love = !r.love;
        if !r.love {
            return None;
        }
        r.not = false;
        r.reasons.clear();
        Some(taste(squad, TasteSignal::React { picture: Some(card.picture.clone()), label: label(card), reaction: QuickReaction::Up, reasons: Vec::new() }))
    }

    /// "Not this", said or taken back: said, the heart goes, its reasons are
    /// offered and the taste is told.
    pub fn not_this(&mut self, squad: &str, key: &str, n: usize, card: &RefCard) -> Option<Command> {
        let r = self.entry(key, n);
        r.not = !r.not;
        r.reasons.clear();
        if !r.not {
            return None;
        }
        r.love = false;
        Some(taste(squad, TasteSignal::React { picture: Some(card.picture.clone()), label: label(card), reaction: QuickReaction::Down, reasons: Vec::new() }))
    }

    /// A reason why not, tapped on or off: on, the taste is told it.
    pub fn reason(&mut self, squad: &str, key: &str, n: usize, card: &RefCard, reason: PickReason) -> Option<Command> {
        let r = self.entry(key, n);
        if !r.not {
            return None;
        }
        if let Some(at) = r.reasons.iter().position(|x| *x == reason) {
            r.reasons.remove(at);
            return None;
        }
        r.reasons.push(reason);
        Some(taste(squad, TasteSignal::Rejected { what: label(card), picture: None, reasons: vec![reason] }))
    }

    /// Kept among `project`'s references (the squad's own with none), once.
    pub fn keep(&mut self, squad: &str, session: Option<SessionId>, project: Option<String>, key: &str, n: usize, card: &RefCard) -> Option<Command> {
        let r = self.entry(key, n);
        if r.kept {
            return None;
        }
        r.kept = true;
        Some(Command::KeepReference { squad: squad.to_owned(), session, project, card: card.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::references::RefKind;

    fn card() -> RefCard {
        RefCard { picture: "/h/visuals/refs/0123456789abcdef.png".into(), title: "Command menu".into(), site: "21st.dev".into(), link: "https://21st.dev/r/cmdk".into(), kind: RefKind::Component, ..RefCard::default() }
    }

    #[test]
    fn a_heart_tells_the_taste_once_given_and_takes_not_this_away() {
        let mut r = Reactions::default();
        let c = card();
        let Some(Command::TasteSignals { squad, project, signals }) = r.love("acme", "k", 0, &c) else { panic!() };
        assert_eq!((squad.as_str(), project), ("acme", None));
        assert_eq!(signals, vec![TasteSignal::React { picture: Some(c.picture.clone()), label: "Command menu (21st.dev)".into(), reaction: QuickReaction::Up, reasons: vec![] }]);
        assert!(r.of("k", 0).love);
        assert_eq!(r.love("acme", "k", 0, &c), None, "taken back: nothing sent");
        assert!(!r.of("k", 0).love);
        assert_eq!(r.of("k", 1), Reaction::default(), "another card is untouched");
    }

    #[test]
    fn not_this_tells_the_taste_and_each_reason_tapped_on_does_too() {
        let mut r = Reactions::default();
        let c = card();
        r.love("acme", "k", 2, &c);
        assert_eq!(r.reason("acme", "k", 2, &c, PickReason::Colors), None, "no reasons before not this");
        let Some(Command::TasteSignals { signals, .. }) = r.not_this("acme", "k", 2, &c) else { panic!() };
        assert!(matches!(&signals[0], TasteSignal::React { reaction: QuickReaction::Down, .. }));
        assert!(!r.of("k", 2).love, "the heart goes");
        let Some(Command::TasteSignals { signals, .. }) = r.reason("acme", "k", 2, &c, PickReason::TooDense) else { panic!() };
        assert_eq!(signals, vec![TasteSignal::Rejected { what: "Command menu (21st.dev)".into(), picture: None, reasons: vec![PickReason::TooDense] }]);
        assert_eq!(r.of("k", 2).reasons, [PickReason::TooDense]);
        assert_eq!(r.reason("acme", "k", 2, &c, PickReason::TooDense), None, "tapped off: nothing sent");
        assert!(r.of("k", 2).reasons.is_empty());
        r.reason("acme", "k", 2, &c, PickReason::Layout);
        assert_eq!(r.not_this("acme", "k", 2, &c), None, "taken back");
        assert_eq!(r.of("k", 2), Reaction::default(), "its reasons with it");
    }

    #[test]
    fn a_card_is_kept_once_with_its_link_for_the_project_in_view() {
        let mut r = Reactions::default();
        let c = card();
        let Some(Command::KeepReference { squad, session, project, card: kept }) = r.keep("acme", Some(4), Some("web".into()), "k", 0, &c) else { panic!() };
        assert_eq!((squad.as_str(), session, project.as_deref(), kept), ("acme", Some(4), Some("web"), c.clone()));
        assert!(r.of("k", 0).kept);
        assert_eq!(r.keep("acme", Some(4), Some("web".into()), "k", 0, &c), None);
        let g = Gallery { name: "designer".into(), key: "k".into(), cards: vec![c.clone(), RefCard { site: "Mobbin".into(), ..c.clone() }, c] };
        assert_eq!(g.sites(), ["21st.dev", "Mobbin"]);
    }
}
