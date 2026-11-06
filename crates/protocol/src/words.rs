//! Words matched against text that may be in any language equipe speaks:
//! `locales/<code>.json`, each key's words `|`-separated per language. The
//! protocol has no screen language of its own, so only the union is read.

use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Every language's words, as compiled in.
const FILES: &[(&str, &str)] = &[
    ("en", include_str!("../locales/en.json")),
    ("de", include_str!("../locales/de.json")),
    ("es", include_str!("../locales/es.json")),
    ("fr", include_str!("../locales/fr.json")),
    ("ja", include_str!("../locales/ja.json")),
    ("ko", include_str!("../locales/ko.json")),
    ("zh-Hans", include_str!("../locales/zh-Hans.json")),
    ("zh-Hant", include_str!("../locales/zh-Hant.json")),
];

/// Each key's words in every language, read once.
fn lists() -> &'static BTreeMap<String, &'static [&'static str]> {
    static LISTS: OnceLock<BTreeMap<String, &'static [&'static str]>> = OnceLock::new();
    LISTS.get_or_init(|| {
        let mut words: BTreeMap<String, Vec<&'static str>> = BTreeMap::new();
        for (code, text) in FILES {
            let file: BTreeMap<String, String> =
                serde_json::from_str(text).unwrap_or_else(|e| panic!("protocol locales/{code}.json: {e}"));
            for (key, list) in file {
                let out = words.entry(key).or_default();
                let list: &'static str = String::leak(list);
                for word in list.split('|').filter(|w| !w.is_empty()) {
                    if !out.contains(&word) {
                        out.push(word);
                    }
                }
            }
        }
        words
            .into_iter()
            .map(|(key, list)| {
                let list: &'static [&'static str] = Vec::leak(list);
                (key, list)
            })
            .collect()
    })
}

/// `key`'s words in every language (none for a key no file has).
pub fn every(key: &str) -> &'static [&'static str] {
    lists().get(key).copied().unwrap_or_default()
}

/// A letter beyond ASCII of a script that puts spaces between its words —
/// Hangul, Cyrillic, Greek, Arabic, accented Latin — so a word in it starts
/// where no such letter comes before. Not Han, kana, Thai, Lao, Khmer or
/// Myanmar, written without spaces: a word in them may start anywhere.
pub fn spaced_letter(c: char) -> bool {
    let unspaced = matches!(c as u32,
        0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1780..=0x17FF | 0x3040..=0x30FF | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0x20000..=0x3FFFF);
    !c.is_ascii() && c.is_alphabetic() && !unspaced
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_every_key() {
        let keys = |text: &str| serde_json::from_str::<BTreeMap<String, String>>(text).unwrap().into_keys().collect::<Vec<_>>();
        let first = keys(FILES[0].1);
        assert!(!first.is_empty());
        for (code, text) in FILES {
            assert_eq!(keys(text), first, "locales/{code}.json");
        }
    }

    #[test]
    fn the_lists_hold_every_language() {
        let confirms = every("match.confirms");
        for word in ["괜찮", "confirm", "確認", "确认", "bestätig", "vérifi", "revisa"] {
            assert!(confirms.contains(&word), "{word}");
        }
        let answers = every("match.answers");
        for word in ["알려", "what ", "wer ", "?", "？", "教え", "什么", "什麼", "quel", "dime"] {
            assert!(answers.contains(&word), "{word}");
        }
        assert!(every("match.nothing").is_empty());
    }

    #[test]
    fn a_spaced_letter_is_by_script_not_by_language() {
        for c in ['한', 'д', 'λ', 'ب', 'é', 'ß'] {
            assert!(spaced_letter(c), "{c}");
        }
        for c in ['a', 'Z', '7', ' ', '?', '漢', 'か', 'カ', 'ก', '。'] {
            assert!(!spaced_letter(c), "{c}");
        }
    }
}
