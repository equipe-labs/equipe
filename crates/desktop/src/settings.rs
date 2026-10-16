//! Settings (⌘,), the rules: its categories in order, how search finds rows,
//! and every key the app answers to, for the shortcuts page. Drawing is
//! `views/settings.rs`; research behind the layout is docs/SETTINGS-RESEARCH.md.

use equipe_core::t;

use crate::icons::Icon;

/// The list down the left, in the order people reach for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Category {
    #[default]
    General,
    /// The AI platforms members work on (`crate::engines`).
    Engines,
    Squad,
    Team,
    Permissions,
    Projects,
    Browser,
    Keys,
    About,
}

impl Category {
    pub const ALL: [Category; 9] = [Category::General, Category::Engines, Category::Squad, Category::Team, Category::Permissions, Category::Projects, Category::Browser, Category::Keys, Category::About];

    /// As the snapshot tool names it (`settings:team`).
    pub fn code(self) -> &'static str {
        match self {
            Category::General => "general",
            Category::Engines => "engines",
            Category::Squad => "squad",
            Category::Team => "team",
            Category::Permissions => "permissions",
            Category::Projects => "projects",
            Category::Browser => "browser",
            Category::Keys => "keys",
            Category::About => "about",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.code() == code)
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::General => t!("settings.cat_general"),
            Category::Engines => t!("settings.cat_engines"),
            Category::Squad => t!("settings.cat_squad"),
            Category::Team => t!("settings.cat_team"),
            Category::Permissions => t!("settings.cat_permissions"),
            Category::Projects => t!("settings.cat_projects"),
            Category::Browser => t!("settings.cat_browser"),
            Category::Keys => t!("settings.cat_keys"),
            Category::About => t!("settings.cat_about"),
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Category::General => Icon::Settings,
            Category::Engines => Icon::Plug,
            Category::Squad => Icon::Server,
            Category::Team => Icon::Users,
            Category::Permissions => Icon::Shield,
            Category::Projects => Icon::Folder,
            Category::Browser => Icon::Globe,
            Category::Keys => Icon::Keyboard,
            Category::About => Icon::Info,
        }
    }

    /// The next or previous category (↑↓ in the list), staying in the list.
    pub fn step(self, down: bool) -> Self {
        let at = Self::ALL.iter().position(|c| *c == self).unwrap_or(0);
        let next = if down { (at + 1).min(Self::ALL.len() - 1) } else { at.saturating_sub(1) };
        Self::ALL[next]
    }
}

/// Whether `query` finds `text`: every word of it, anywhere, in any case.
/// An empty query finds everything.
pub fn matches(query: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    query.split_whitespace().all(|word| text.contains(&word.to_lowercase()))
}

/// What a search shows: the categories that have rows it finds, in the list's
/// order, each with those rows (`rows` are each a category, the words a row
/// is found by, and what to show). A category found by its own name shows
/// all of its rows.
pub fn search<T: Clone>(query: &str, rows: &[(Category, String, T)]) -> Vec<(Category, Vec<T>)> {
    Category::ALL
        .into_iter()
        .filter_map(|category| {
            let whole = matches(query, category.label());
            let found: Vec<T> = rows.iter().filter(|(c, words, _)| *c == category && (whole || matches(query, words))).map(|(_, _, row)| row.clone()).collect();
            (!found.is_empty()).then_some((category, found))
        })
        .collect()
}

/// One key the app answers to: where it works, the keys, what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub area: &'static str,
    pub keys: &'static str,
    pub what: &'static str,
}

/// Every key, by where it works: the window, writing, answering a request,
/// the work screens. Read-only: the keys can't be changed (yet).
pub fn shortcuts() -> Vec<Shortcut> {
    let window = t!("settings.keys_window");
    let compose = t!("settings.keys_compose");
    let requests = t!("settings.keys_requests");
    let board = t!("settings.keys_board");
    let s = |area, keys, what| Shortcut { area, keys, what };
    vec![
        s(window, "⌘,", t!("settings.key_settings")),
        s(window, "⌘N", t!("sidebar.new_message")),
        s(window, "⌘K", t!("settings.key_search_all")),
        s(window, "⌘\\", t!("settings.key_sidebar")),
        s(window, "⌘B", t!("settings.key_board")),
        s(window, "⌘⇧B", t!("settings.key_browser")),
        s(window, "⌘F", t!("chat.find_placeholder")),
        s(window, "⌃0–9", t!("board.key_projects")),
        s(window, "⌘+ · ⌘-", t!("settings.key_zoom")),
        s(window, "⌘0", t!("main.menu_actual_size")),
        s(window, "⌘⇧O", t!("settings.key_folds")),
        s(window, "⌘⇧K", t!("settings.key_machine")),
        s(compose, "⏎", t!("composer.tip_send")),
        s(compose, "⇧⏎ · ⌥⏎ · ⌃J", t!("settings.key_newline")),
        s(compose, "Tab", t!("composer.tip_memo")),
        s(compose, "↑ ↓", t!("settings.key_history")),
        s(compose, "/ · @", t!("settings.key_complete")),
        s(compose, "⌘U", t!("composer.tip_attach")),
        s(compose, "⌘⇧V", t!("settings.key_paste_inline")),
        s(compose, "⌘. · Esc", t!("composer.tip_stop")),
        s(compose, "⌘⇧I", t!("composer.tip_model")),
        s(compose, "⌘⇧E", t!("composer.tip_effort")),
        s(compose, "⌘⇧M", t!("composer.tip_mode")),
        s(requests, "Y · A", t!("approvals.allow_once")),
        s(requests, "⇧A", t!("approvals.always")),
        s(requests, "N", t!("approvals.deny")),
        s(requests, "O", t!("approvals.open_preview")),
        s(board, "A", t!("board.key_yes")),
        s(board, "⇧A", t!("board.key_method")),
        s(board, "R", t!("board.key_reply")),
        s(board, "N", t!("board.key_no")),
        s(board, "D", t!("board.key_diff")),
        s(board, "C", t!("board.key_talk")),
        s(board, "M", t!("board.key_merge")),
        s(board, "U", t!("board.key_undo")),
        s(board, "J K · ↑ ↓", t!("board.key_move")),
        s(board, "Space", t!("board.key_peek")),
        s(board, "⏎", t!("board.key_page")),
        s(board, "/", t!("board.key_search")),
        s(board, "F", t!("board.key_filter")),
        s(board, "X · ⇧↑↓ · ⌘A", t!("board.key_choose")),
        s(board, "⌥V", t!("board.key_save_view")),
        s(board, "G I · G L · G B", t!("board.key_go")),
        s(board, "?", t!("board.keys_title")),
        s(board, "Esc", t!("board.key_back")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_finds_every_word_in_any_case_and_order() {
        assert!(matches("", "anything"));
        assert!(matches("  ", "anything"));
        assert!(matches("dark", "Appearance — System, Light, Dark"));
        assert!(matches("LIGHT appear", "Appearance — System, Light, Dark"));
        assert!(matches("알림 소리", "알림 · 소리 내기"));
        assert!(!matches("dark mode", "Appearance — System, Light, Dark"), "every word must be there");
    }

    #[test]
    fn search_keeps_the_lists_order_and_only_what_it_finds() {
        let rows = vec![
            (Category::Permissions, "기본 모드 auto".to_owned(), "mode"),
            (Category::General, "언어 language 한국어".to_owned(), "lang"),
            (Category::General, "모양 appearance 다크".to_owned(), "look"),
            (Category::Keys, "⌘B 작업 보드".to_owned(), "cmd-b"),
        ];
        assert_eq!(search("language", &rows), vec![(Category::General, vec!["lang"])]);
        assert_eq!(search("a", &rows).iter().map(|(c, _)| *c).collect::<Vec<_>>(), [Category::General, Category::Permissions], "General comes before Permissions, as in the list");
        assert!(search("nothing like it", &rows).is_empty());
        assert_eq!(search("", &rows).len(), 3, "an empty search shows every category with rows");
    }

    #[test]
    fn a_category_found_by_its_name_shows_all_its_rows() {
        let rows = vec![(Category::Permissions, "기본 모드".to_owned(), 1), (Category::Permissions, "항상 허용 규칙".to_owned(), 2), (Category::General, "언어".to_owned(), 3)];
        let found = search(Category::Permissions.label(), &rows);
        assert_eq!(found, vec![(Category::Permissions, vec![1, 2])]);
    }

    #[test]
    fn categories_step_within_the_list_and_round_trip_their_codes() {
        assert_eq!(Category::General.step(true), Category::Engines);
        assert_eq!(Category::General.step(false), Category::General);
        assert_eq!(Category::About.step(true), Category::About);
        assert!(Category::ALL.iter().all(|c| Category::parse(c.code()) == Some(*c)));
        assert_eq!(Category::parse("nope"), None);
    }

    #[test]
    fn every_shortcut_says_what_it_does_and_the_list_has_the_windows_keys() {
        let keys = shortcuts();
        assert!(keys.iter().all(|k| !k.keys.is_empty() && !k.what.is_empty() && !k.area.is_empty()));
        for key in ["⌘,", "⌘\\", "⌘N", "⌘K", "⌘B", "⌘⇧B", "⏎", "⇧A", "J K · ↑ ↓", "⌘+ · ⌘-", "⌘0", "⌃0–9"] {
            assert!(keys.iter().any(|k| k.keys == key), "{key} is listed");
        }
        assert!(!keys.iter().any(|k| k.keys == "⌘0–9"), "⌘0 is actual size; the slices are ⌃0–9");
        // Searchable like any row: by what it does, or by the key itself.
        assert!(keys.iter().any(|k| matches("보드", &format!("{} {}", k.keys, k.what))));
    }
}
