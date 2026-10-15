//! The screens' words in the person's language.
//!
//! Each front end keeps its words in `locales/<code>.json` — one flat object
//! of `"key": "text"` per language, `{name}` where a value goes in — and loads
//! the person's language with English behind it: a word not translated yet
//! shows in English, never as a blank. Which language: `EQUIPE_LANG` when set,
//! else the locale the terminal says (`LC_ALL`, `LC_MESSAGES`, `LANG`), else the
//! system's preferred languages — a Mac app has only the last.

use std::collections::HashMap;

/// The languages equipe speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Ko,
    En,
    Ja,
    ZhHans,
    ZhHant,
    Es,
    Fr,
    De,
}

impl Lang {
    pub const ALL: [Lang; 8] = [Lang::Ko, Lang::En, Lang::Ja, Lang::ZhHans, Lang::ZhHant, Lang::Es, Lang::Fr, Lang::De];

    /// Its code, as locale files are named: `ko`, `zh-Hans`.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Ko => "ko",
            Lang::En => "en",
            Lang::Ja => "ja",
            Lang::ZhHans => "zh-Hans",
            Lang::ZhHant => "zh-Hant",
            Lang::Es => "es",
            Lang::Fr => "fr",
            Lang::De => "de",
        }
    }

    /// Its name in itself, for a list to pick from.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Ko => "한국어",
            Lang::En => "English",
            Lang::Ja => "日本語",
            Lang::ZhHans => "简体中文",
            Lang::ZhHant => "繁體中文",
            Lang::Es => "Español",
            Lang::Fr => "Français",
            Lang::De => "Deutsch",
        }
    }

    /// The language a locale tag names — `ko_KR.UTF-8`, `zh-Hant-TW`,
    /// `zh_HK`, `en-US`, `de` — if equipe speaks it.
    pub fn parse(tag: &str) -> Option<Lang> {
        let tag = tag.split(['.', '@']).next().unwrap_or_default().replace('_', "-").to_ascii_lowercase();
        let mut parts = tag.split('-');
        Some(match parts.next()? {
            "ko" => Lang::Ko,
            "en" => Lang::En,
            "ja" => Lang::Ja,
            "es" => Lang::Es,
            "fr" => Lang::Fr,
            "de" => Lang::De,
            "zh" => {
                let rest: Vec<&str> = parts.collect();
                if rest.iter().any(|p| matches!(*p, "hant" | "tw" | "hk" | "mo")) { Lang::ZhHant } else { Lang::ZhHans }
            }
            _ => return None,
        })
    }

    /// The person's language in a terminal: `EQUIPE_LANG`, the locale
    /// variables, then the system's; English when none is one equipe speaks.
    pub fn detect() -> Lang {
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty() && v != "C" && v != "POSIX");
        ["EQUIPE_LANG", "LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|name| env(name))
            .and_then(|tag| Lang::parse(&tag))
            .unwrap_or_else(Lang::detect_system)
    }

    /// The person's language in an app: `EQUIPE_LANG`, else the first of the
    /// system's preferred languages equipe speaks; English otherwise.
    pub fn detect_system() -> Lang {
        if let Some(lang) = std::env::var("EQUIPE_LANG").ok().and_then(|tag| Lang::parse(&tag)) {
            return lang;
        }
        sys_locale::get_locales().find_map(|tag| Lang::parse(&tag)).unwrap_or(Lang::En)
    }
}

/// A front end's words in one language, with English behind it.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub lang: Lang,
    words: HashMap<String, String>,
    english: HashMap<String, String>,
}

impl Catalog {
    /// Load `lang` from `files` — `(code, json)` for each language, as
    /// `include_str!` brings them in. A file that doesn't parse counts as empty.
    pub fn load(lang: Lang, files: &[(&str, &str)]) -> Self {
        let table = |lang: Lang| -> HashMap<String, String> {
            files.iter().find(|(code, _)| *code == lang.code()).and_then(|(_, json)| serde_json::from_str(json).ok()).unwrap_or_default()
        };
        Self { lang, words: table(lang), english: table(Lang::En) }
    }

    /// The word for `key`: in the language, else in English, else the key.
    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.words.get(key).or_else(|| self.english.get(key)).map_or(key, String::as_str)
    }

    /// The word for `key` with each `{name}` filled in.
    pub fn fill(&self, key: &str, args: &[(&str, String)]) -> String {
        fill(self.get(key), args)
    }
}

/// `text` with each `{name}` replaced by its value.
pub fn fill(text: &str, args: &[(&str, String)]) -> String {
    let mut out = text.to_owned();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// The `{name}`s a text takes, sorted — what every translation of it must take too.
pub fn placeholders(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) if after[..end].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && end > 0 => {
                out.push(after[..end].to_owned());
                rest = &after[end + 1..];
            }
            _ => rest = after,
        }
    }
    out.sort();
    out.dedup();
    out
}

/// What is wrong with a front end's translations: a key English lacks, one a
/// language lacks, or a translation taking other `{name}`s than English's.
pub fn problems(files: &[(&str, &str)]) -> Vec<String> {
    let table = |code: &str| -> Result<HashMap<String, String>, String> {
        let json = files.iter().find(|(c, _)| *c == code).map(|(_, j)| *j).ok_or(format!("no {code}.json"))?;
        serde_json::from_str(json).map_err(|e| format!("{code}.json: {e}"))
    };
    let english = match table("en") {
        Ok(t) => t,
        Err(e) => return vec![e],
    };
    let mut out = Vec::new();
    for lang in Lang::ALL {
        let words = match table(lang.code()) {
            Ok(t) => t,
            Err(e) => {
                out.push(e);
                continue;
            }
        };
        for (key, text) in &english {
            match words.get(key) {
                None => out.push(format!("{}: missing {key}", lang.code())),
                Some(word) if placeholders(word) != placeholders(text) => {
                    out.push(format!("{}: {key} takes {:?}, English {:?}", lang.code(), placeholders(word), placeholders(text)))
                }
                Some(_) => {}
            }
        }
        for key in words.keys().filter(|k| !english.contains_key(*k)) {
            out.push(format!("{}: {key} isn't in English", lang.code()));
        }
    }
    out.sort();
    out
}

/// Declare a front end's words, next to its `FILES` (each language's
/// `locales/<code>.json`, `include_str!`-ed): a `catalog()` in the person's
/// language — in tests always `$test`, what its tests read — and `set`.
/// Then `t!("board.accept")` is the word; `t!("chat.sent", name = who)`
/// fills it in.
#[macro_export]
macro_rules! words {
    (test = $test:expr) => {
        static CATALOG: std::sync::OnceLock<$crate::i18n::Catalog> = std::sync::OnceLock::new();

        /// The words in the person's language, loaded once.
        pub fn catalog() -> &'static $crate::i18n::Catalog {
            CATALOG.get_or_init(|| {
                let lang = if cfg!(test) { $test } else { $crate::i18n::Lang::detect() };
                $crate::i18n::Catalog::load(lang, FILES)
            })
        }

        /// Use `lang` from now on, if no word was shown yet. False after.
        #[allow(dead_code)]
        pub fn set(lang: $crate::i18n::Lang) -> bool {
            CATALOG.set($crate::i18n::Catalog::load(lang, FILES)).is_ok()
        }
    };
}

/// The word for a key, from the calling crate's `i18n::catalog()` — `crate`,
/// not `$crate`: each front end has words of its own.
#[macro_export]
#[allow(clippy::crate_in_macro_def)]
macro_rules! t {
    ($key:literal) => {
        crate::i18n::catalog().get($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        crate::i18n::catalog().fill($key, &[$((stringify!($name), ($value).to_string())),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locale_tag_names_its_language_whatever_its_shape() {
        assert_eq!(Lang::parse("ko_KR.UTF-8"), Some(Lang::Ko));
        assert_eq!(Lang::parse("en-US"), Some(Lang::En));
        assert_eq!(Lang::parse("zh-Hant-TW"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("zh_HK"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("zh-Hans-CN"), Some(Lang::ZhHans));
        assert_eq!(Lang::parse("zh"), Some(Lang::ZhHans));
        assert_eq!(Lang::parse("de_DE@euro"), Some(Lang::De));
        assert_eq!(Lang::parse("pt-BR"), None);
        assert!(Lang::ALL.iter().all(|l| Lang::parse(l.code()) == Some(*l)));
    }

    #[test]
    fn a_word_not_translated_yet_shows_in_english_and_values_fill_in() {
        let files = [("ko", r#"{"hi": "안녕 {name}"}"#), ("en", r#"{"hi": "Hi {name}", "bye": "Bye"}"#)];
        let ko = Catalog::load(Lang::Ko, &files);
        assert_eq!(ko.fill("hi", &[("name", "Abel".into())]), "안녕 Abel");
        assert_eq!(ko.get("bye"), "Bye");
        assert_eq!(ko.get("nope"), "nope");
        assert_eq!(Catalog::load(Lang::Ja, &files).get("bye"), "Bye");
    }

    #[test]
    fn translations_that_lack_a_word_or_take_other_values_are_found() {
        let mut files: Vec<(&str, &str)> = Lang::ALL.iter().map(|l| (l.code(), r#"{"hi": "x {name}"}"#)).collect();
        assert!(problems(&files).is_empty());
        files[2] = ("ja", r#"{"hi": "x {who}", "extra": "y"}"#);
        let found = problems(&files);
        assert!(found.iter().any(|p| p.starts_with("ja: hi takes")), "{found:?}");
        assert!(found.iter().any(|p| p == "ja: extra isn't in English"), "{found:?}");
        assert_eq!(placeholders("{a} and {b}, {a} — {not a name}"), ["a", "b"]);
    }
}
