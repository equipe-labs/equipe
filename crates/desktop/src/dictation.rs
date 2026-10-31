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

/// The language of the keyboard being typed with now (한글 → Korean, ABC →
/// English), if equipe speaks it: what the person likely speaks next.
pub fn typing_lang() -> Option<Lang> {
    #[cfg(target_os = "macos")]
    {
        tis::current_languages().iter().find_map(|tag| Lang::parse(tag))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
mod tis {
    use std::ffi::c_void;

    type CFTypeRef = *const c_void;

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn TISCopyCurrentKeyboardInputSource() -> CFTypeRef;
        fn TISGetInputSourceProperty(source: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        static kTISPropertyInputSourceLanguages: CFTypeRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: CFTypeRef);
        fn CFArrayGetCount(array: CFTypeRef) -> isize;
        fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
        fn CFStringGetCString(string: CFTypeRef, buffer: *mut i8, size: isize, encoding: u32) -> u8;
    }

    const UTF8: u32 = 0x0800_0100;

    /// The current input source's languages (`ko`, `en`), first the main one.
    pub fn current_languages() -> Vec<String> {
        let mut out = Vec::new();
        // SAFETY: TIS returns a +1 input source (released below) whose language
        // property is an array of strings it owns, read before the release.
        unsafe {
            let source = TISCopyCurrentKeyboardInputSource();
            if source.is_null() {
                return out;
            }
            let langs = TISGetInputSourceProperty(source, kTISPropertyInputSourceLanguages);
            if !langs.is_null() {
                for i in 0..CFArrayGetCount(langs) {
                    let tag = CFArrayGetValueAtIndex(langs, i);
                    let mut buf = [0i8; 64];
                    if CFStringGetCString(tag, buf.as_mut_ptr(), buf.len() as isize, UTF8) != 0 {
                        out.push(std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned());
                    }
                }
            }
            CFRelease(source);
        }
        out
    }
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
    fn the_keyboard_typed_with_is_read_without_harm() {
        // Whatever this Mac types with (a test host may have none): a language equipe speaks, or none.
        let _ = typing_lang();
        #[cfg(target_os = "macos")]
        assert!(tis::current_languages().iter().all(|tag| !tag.is_empty()));
    }

    #[test]
    fn every_language_has_its_recognizer_locale() {
        for lang in Lang::ALL {
            assert!(locale(lang).contains('-'), "{lang:?}");
        }
    }
}
