//! Which language dictation listens for. It recognizes on this Mac only
//! (never on Apple's servers), and a Mac has an on-device model only for the
//! languages its dictation was given — Korean and English often, the rest
//! not until added. So the person may pick the language apart from the app's,
//! and with none picked equipe takes the first that can be heard here: the
//! app's, then the Mac's preferred languages in order.

use equipe_core::i18n::Lang;

/// The recognizer's locale for `lang`.
pub fn locale(lang: Lang) -> &'static str {
    match lang {
        Lang::Ko => "ko-KR",
        Lang::En => "en-US",
        Lang::Ja => "ja-JP",
        Lang::ZhHans => "zh-CN",
        Lang::ZhHant => "zh-TW",
        Lang::Es => "es-ES",
        Lang::Fr => "fr-FR",
        Lang::De => "de-DE",
    }
}

/// The language to listen for: the one `picked`, else the first of the app's
/// (`app`) and the Mac's `preferred` that `heard_here` (an on-device model),
/// else the app's — whose failure then says what to add.
pub fn choose(picked: Option<Lang>, app: Lang, preferred: &[Lang], heard_here: impl Fn(Lang) -> bool) -> Lang {
    if let Some(lang) = picked {
        return lang;
    }
    std::iter::once(app).chain(preferred.iter().copied()).find(|l| heard_here(*l)).unwrap_or(app)
}

/// Whether this Mac can hear `lang` on the device.
pub fn heard_here(lang: Lang) -> bool {
    crate::transcribe::on_device(locale(lang))
}

/// Where macOS adds dictation languages.
pub const SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.Keyboard-Settings.extension";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pick_is_kept_whatever_the_mac_has() {
        assert_eq!(choose(Some(Lang::Ja), Lang::En, &[], |_| false), Lang::Ja);
    }

    #[test]
    fn with_none_picked_the_first_language_this_mac_can_hear() {
        let here = |l: Lang| matches!(l, Lang::Ko | Lang::En);
        assert_eq!(choose(None, Lang::Ko, &[Lang::En], here), Lang::Ko, "the app's, heard here");
        assert_eq!(choose(None, Lang::Ja, &[Lang::Ko, Lang::En], here), Lang::Ko, "the app's isn't: the Mac's first that is");
        assert_eq!(choose(None, Lang::De, &[Lang::Fr], here), Lang::De, "none is: the app's, whose failure says what to add");
    }

    #[test]
    fn every_language_has_its_recognizer_locale() {
        for lang in Lang::ALL {
            assert!(locale(lang).contains('-'), "{lang:?}");
        }
    }
}
