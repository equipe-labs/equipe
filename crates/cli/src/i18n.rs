//! The screen's words in the person's language: `locales/<code>.json`, read
//! through `equipe_core::t!` (see `equipe_core::i18n`). Tests read English,
//! what they were written against.

use equipe_core::i18n::Lang;

/// Every language's words, as compiled in.
pub const FILES: &[(&str, &str)] = &[
    ("ko", include_str!("../locales/ko.json")),
    ("en", include_str!("../locales/en.json")),
    ("ja", include_str!("../locales/ja.json")),
    ("zh-Hans", include_str!("../locales/zh-Hans.json")),
    ("zh-Hant", include_str!("../locales/zh-Hant.json")),
    ("es", include_str!("../locales/es.json")),
    ("fr", include_str!("../locales/fr.json")),
    ("de", include_str!("../locales/de.json")),
];

equipe_core::words!(test = Lang::En);

/// What a built-in role does, in the language in use; a role equipe has no
/// words for keeps its own summary.
pub fn role_summary(role: &equipe_core::roles::Role) -> &'static str {
    match role.id {
        "pm" => equipe_core::t!("role.pm"),
        "designer" => equipe_core::t!("role.designer"),
        "frontend" => equipe_core::t!("role.frontend"),
        "backend" => equipe_core::t!("role.backend"),
        "mobile" => equipe_core::t!("role.mobile"),
        "qa" => equipe_core::t!("role.qa"),
        "security" => equipe_core::t!("role.security"),
        "data" => equipe_core::t!("role.data"),
        "custom" => equipe_core::t!("role.custom"),
        _ => role.summary,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_language_has_every_word_taking_the_same_values() {
        let problems = equipe_core::i18n::problems(super::FILES);
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn a_count_takes_the_form_its_language_gives_it() {
        use equipe_core::i18n::{Catalog, Lang};
        let lines = |lang: Lang, n: u64| Catalog::load(lang, super::FILES).fill("view.more_lines", &[("count", n.to_string())]);
        assert_eq!(lines(Lang::En, 1), "1 more line");
        assert_eq!(lines(Lang::En, 2), "2 more lines");
        assert_eq!(lines(Lang::En, 0), "0 more lines");
        assert_eq!(lines(Lang::Fr, 0), "0 ligne de plus", "French says one of 0 too");
        assert_eq!(lines(Lang::De, 3), "3 weitere Zeilen");
        assert_eq!(lines(Lang::Ko, 1), "1줄 더", "Korean has one form");
        assert_eq!(crate::t!("telegram.n_requests", count = 1), "1 request");
    }

    #[test]
    fn every_built_in_role_says_what_it_does_in_english_as_its_role_does() {
        for role in equipe_core::ROLES.iter().chain([&equipe_core::roles::CUSTOM]) {
            assert_eq!(super::role_summary(role), role.summary, "{}", role.id);
        }
    }
}
