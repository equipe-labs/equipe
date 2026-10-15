//! The screen's words in the person's language: `locales/<code>.json`, read
//! through `equipe_core::t!` (see `equipe_core::i18n`). Tests read Korean,
//! what they were written against. The language is the one the person picked
//! for every front end, else the Mac's (an app has no locale of its own); a
//! new pick shows at once ([`set`]).

use equipe_core::i18n::{Front, Lang};

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

equipe_core::words!(test = Lang::Ko, front = Front::App);

/// The language picked in this app before the pick was shared by every front
/// end (`lang` in `desktop-target`): it becomes the shared pick, unless one
/// was made since, and is dropped from the app's own state.
pub fn migrate(home: &std::path::Path) {
    let mut remembered = crate::target::Remembered::load(home);
    let Some(old) = remembered.lang.take() else { return };
    if equipe_core::i18n::saved(home).is_none()
        && let Some(lang) = Lang::parse(&old)
    {
        let _ = equipe_core::i18n::save(home, Some(lang));
    }
    remembered.save(home);
}

#[cfg(test)]
mod tests {
    use equipe_core::i18n::{Lang, Words};

    #[test]
    fn every_language_has_every_word_taking_the_same_values() {
        let problems = equipe_core::i18n::problems(super::FILES);
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn a_new_language_shows_at_once() {
        // The app's own words, switched as the settings switch them — on a
        // `Words` of their own, so the other tests keep reading Korean.
        static WORDS: Words = Words::new(super::FILES, || Lang::Ko);
        let ko = WORDS.catalog().get("sidebar.language").to_owned();
        WORDS.set(Lang::En);
        let en = WORDS.catalog().get("sidebar.language");
        assert_ne!(ko, en);
        assert_eq!(en, WORDS.of(Lang::En).get("sidebar.language"));
        assert_eq!(t_now(), super::catalog().get("sidebar.language"), "t! reads the catalog in use");
    }

    fn t_now() -> &'static str {
        crate::t!("sidebar.language")
    }

    #[test]
    fn the_old_pick_becomes_everyones_once() {
        let home = std::env::temp_dir().join(format!("equipe-desktop-lang-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let mut remembered = crate::target::Remembered::load(&home);
        remembered.lang = Some("ja".into());
        remembered.save(&home);
        super::migrate(&home);
        assert_eq!(equipe_core::i18n::saved(&home), Some(Lang::Ja));
        assert_eq!(crate::target::Remembered::load(&home).lang, None, "kept in one place from now on");
        // A pick made in the shared place since wins over an old one.
        let mut remembered = crate::target::Remembered::load(&home);
        remembered.lang = Some("de".into());
        remembered.save(&home);
        super::migrate(&home);
        assert_eq!(equipe_core::i18n::saved(&home), Some(Lang::Ja));
        std::fs::remove_dir_all(&home).unwrap();
    }
}
