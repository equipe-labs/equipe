//! The person's language, one pick for every screen: `equipe lang`, `/lang`.
//! See `equipe_core::i18n` for how it is found when nothing was picked.

use std::path::Path;

use equipe_core::i18n::{self, Front, Lang};
use equipe_core::t;

/// The codes to pick from, as typed.
pub fn codes() -> String {
    Lang::ALL.iter().map(|l| l.code()).collect::<Vec<_>>().join(" · ")
}

/// What a typed word picks: a language by its code or its own name, or
/// (`Some(None)`) `auto` — System. `None`: nothing equipe speaks.
pub fn parse_choice(word: &str) -> Option<Option<Lang>> {
    let word = word.trim();
    match word.to_lowercase().as_str() {
        "auto" | "system" => Some(None),
        lower => Lang::parse(word).or_else(|| Lang::ALL.into_iter().find(|l| l.native_name().to_lowercase() == lower)).map(Some),
    }
}

/// `한국어 (ko)`.
pub fn name(lang: Lang) -> String {
    format!("{} ({})", lang.native_name(), lang.code())
}

/// Keep `choice` for every screen and speak it here from now on. The
/// language now in use: the pick, or what System comes to.
pub fn choose(home: &Path, choice: Option<Lang>) -> Result<Lang, String> {
    i18n::save(home, choice)?;
    let lang = Lang::resolve_in(home, Front::Terminal);
    crate::i18n::set(lang);
    Ok(lang)
}

/// What is in use, in a line: the pick, or System and what it comes to.
pub fn now(home: &Path) -> String {
    match i18n::saved(home) {
        Some(lang) => t!("lang.now_picked", name = name(lang)),
        None => t!("lang.now_system", name = name(Lang::resolve_auto(Front::Terminal))),
    }
}

/// `equipe lang [code|auto]`: show the language, or pick one.
pub fn command(home: &Path, word: Option<&str>) -> Result<Vec<String>, String> {
    let Some(word) = word else {
        let mut lines = vec![now(home)];
        if let Some(value) = std::env::var("EQUIPE_LANG").ok().filter(|v| Lang::parse(v).is_some()) {
            lines.push(t!("lang.overridden", value = value));
        }
        lines.push(t!("lang.usage", codes = codes()));
        return Ok(lines);
    };
    let Some(choice) = parse_choice(word) else {
        return Err(t!("lang.unknown", word = word, codes = codes()));
    };
    let lang = choose(home, choice)?;
    Ok(vec![match choice {
        Some(_) => t!("lang.picked", name = name(lang)),
        None => t!("lang.picked_system", name = name(lang)),
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_is_picked_by_code_or_its_own_name_and_auto_is_system() {
        assert_eq!(parse_choice("ko"), Some(Some(Lang::Ko)));
        assert_eq!(parse_choice("zh-Hant"), Some(Some(Lang::ZhHant)));
        assert_eq!(parse_choice("zh_TW"), Some(Some(Lang::ZhHant)));
        assert_eq!(parse_choice("Deutsch"), Some(Some(Lang::De)));
        assert_eq!(parse_choice("日本語"), Some(Some(Lang::Ja)));
        assert_eq!(parse_choice("auto"), Some(None));
        assert_eq!(parse_choice("System"), Some(None));
        assert_eq!(parse_choice("klingon"), None);
    }

    #[test]
    fn showing_says_what_is_in_use_and_a_wrong_word_changes_nothing() {
        let home = std::env::temp_dir().join(format!("equipe-cli-lang-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let shown = command(&home, None).unwrap();
        assert!(shown[0].starts_with("Language: System — "), "{shown:?}");
        assert!(shown.last().unwrap().contains("zh-Hant"), "{shown:?}");
        assert!(command(&home, Some("klingon")).unwrap_err().contains("klingon"));
        assert_eq!(i18n::saved(&home), None);
        std::fs::remove_dir_all(&home).ok();
    }
}
