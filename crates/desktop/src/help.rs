//! `/help` and ⌘/: the keys of this window and every command, from the list
//! completion uses (`commands::COMMANDS`) — so the sheet can't drift from what
//! `/` offers. Plain data.

use equipe_core::protocol::Mode;

use crate::commands::COMMANDS;
use equipe_core::t;

/// What the sheet opens with.
pub fn intro() -> &'static str {
    t!("help.intro")
}

pub struct Section {
    pub title: String,
    pub rows: Vec<(String, String)>,
}

fn rows(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(k, w)| ((*k).to_owned(), (*w).to_owned())).collect()
}

pub fn sections() -> Vec<Section> {
    vec![
        Section {
            title: t!("help.chat").into(),
            rows: rows(&[
                ("⏎", t!("help.chat_send")),
                ("⇥ (Tab)", t!("help.chat_memo")),
                (t!("help.chat_at_key"), t!("help.chat_at")),
                ("⌘↑ ⌘↓ · ⌥⌘1–9", t!("help.chat_pick")),
                ("⇧⏎ · ⌥⏎ · ⌃J", t!("help.chat_newline")),
                ("Esc · ⌘.", t!("help.chat_stop")),
                (t!("help.chat_paste_key"), t!("help.chat_paste")),
                ("→", t!("help.chat_guess")),
                ("⌘K · /", t!("help.chat_commands")),
            ]),
        },
        Section {
            title: t!("help.navigate").into(),
            rows: rows(&[
                ("⌘B", t!("help.nav_board")),
                ("⌘0 · ⌘1–9", t!("help.nav_projects")),
                ("⌘⇧O", t!("help.nav_folds")),
                ("⌘U", t!("help.nav_usage")),
                ("⌘/", t!("help.nav_help")),
                ("⌘⇧K", t!("help.nav_connect")),
                (t!("help.nav_link_key"), t!("help.nav_link")),
            ]),
        },
        Section {
            title: t!("help.answer").into(),
            rows: rows(&[("y · a · n", t!("help.answer_keys")), (t!("help.answer_board_key"), t!("help.answer_board"))]),
        },
        Section {
            title: t!("help.modes").into(),
            rows: Mode::ALL.iter().map(|m| (crate::views::settings::mode_label(*m).to_owned(), crate::views::settings::mode_summary(*m).to_owned())).collect(),
        },
        Section {
            title: t!("help.commands").into(),
            rows: COMMANDS.iter().map(|c| (format!("/{} {}", c.name, c.args).trim_end().to_owned(), crate::i18n::catalog().get(c.about).to_owned())).collect(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_completion_offers_is_in_the_help() {
        let help = sections().pop().unwrap();
        assert_eq!(help.rows.len(), COMMANDS.len());
        assert!(help.rows.iter().any(|(k, _)| k.starts_with("/usage")));
        assert!(help.rows.iter().all(|(k, about)| k.starts_with('/') && !about.is_empty()));
    }

    #[test]
    fn the_four_modes_are_explained() {
        let modes = &sections()[3];
        assert_eq!(modes.rows.len(), Mode::ALL.len());
    }
}
