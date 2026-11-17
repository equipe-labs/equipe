//! Reference cards: real components, screens and patterns a member shows the
//! person in the conversation (`show_references`), each a picture equipe
//! fetched and keeps outside every repository, with where it is from and a
//! line on why it fits and what to take and leave.
//!
//! A gallery travels as one of the conversation's notes (`notice`), in
//! Markdown a screen that knows nothing of cards still reads — its pictures
//! embedded, its sources linked (docs/VISUALS.md, *Reference cards*). A screen
//! that knows them draws the note as a card instead; the card's text is
//! `equipe_core::references`' to write and read.

use serde::{Deserialize, Serialize};

/// What a reference shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// One component: a command menu, a pricing table, a date picker.
    #[default]
    Component,
    /// A whole screen of a product.
    Screen,
    /// A pattern across screens: onboarding, empty states, a checkout flow.
    Pattern,
}

impl RefKind {
    pub const ALL: [RefKind; 3] = [RefKind::Component, RefKind::Screen, RefKind::Pattern];

    /// Its word in the card's text and the tool's arguments, the same in every language.
    pub fn key(self) -> &'static str {
        match self {
            RefKind::Component => "component",
            RefKind::Screen => "screen",
            RefKind::Pattern => "pattern",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        let word = word.trim();
        Self::ALL.into_iter().find(|k| k.key().eq_ignore_ascii_case(word))
    }
}

/// One card of a gallery.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefCard {
    /// The picture, on the daemon's machine (read with `read_image`): in the
    /// squad's picture store, never in a repository.
    pub picture: String,
    pub title: String,
    /// Where it is from, as a person names it (`21st.dev`, `Mobbin`).
    pub site: String,
    /// Its page there: an https address.
    pub link: String,
    pub kind: RefKind,
    /// Why it fits what the person is making, in a line.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// What to take from it, and what to leave.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub take: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub leave: String,
    /// The pattern folder it belongs in among the project's references
    /// (`command-menu`): where "add to references" files it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pattern: String,
}

/// The sites whose pictures are private study material: shown in equipe,
/// never shipped, committed or copied into a project (their terms).
pub const STUDY_ONLY: [&str; 1] = ["mobbin.com"];

impl RefCard {
    /// The host of its link, without `www.`.
    pub fn host(&self) -> String {
        let rest = self.link.trim().strip_prefix("https://").unwrap_or(self.link.trim());
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default().to_ascii_lowercase();
        host.strip_prefix("www.").map(str::to_owned).unwrap_or(host)
    }

    /// Whether its picture is study material only ([`STUDY_ONLY`]).
    pub fn study_only(&self) -> bool {
        let host = self.host();
        STUDY_ONLY.iter().any(|site| host == *site || host.ends_with(&format!(".{site}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_reads_as_its_word_and_a_cards_host_says_whether_it_is_study_only() {
        assert_eq!(RefKind::parse(" Screen "), Some(RefKind::Screen));
        assert_eq!(RefKind::parse("widget"), None);
        let card = RefCard { link: "https://www.Mobbin.com/screens/abc".into(), ..RefCard::default() };
        assert_eq!(card.host(), "mobbin.com");
        assert!(card.study_only());
        assert!(RefCard { link: "https://cdn.mobbin.com/x.png".into(), ..RefCard::default() }.study_only());
        assert!(!RefCard { link: "https://21st.dev/r/command-menu".into(), ..RefCard::default() }.study_only());
        assert!(!RefCard { link: "https://notmobbin.com/x".into(), ..RefCard::default() }.study_only());
        let json = serde_json::to_string(&RefCard { title: "Menu".into(), ..RefCard::default() }).unwrap();
        assert!(!json.contains("why") && json.contains(r#""kind":"component""#), "{json}");
    }
}
