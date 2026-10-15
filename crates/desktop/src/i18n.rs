//! The screen's words in the person's language: `locales/<code>.json`, read
//! through `equipe_core::t!` (see `equipe_core::i18n`). Tests read Korean,
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

equipe_core::words!(test = Lang::Ko);

#[cfg(test)]
mod tests {
    #[test]
    fn every_language_has_every_word_taking_the_same_values() {
        let problems = equipe_core::i18n::problems(super::FILES);
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
