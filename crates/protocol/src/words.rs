//! Words matched against text that may be in any language equipe speaks:
//! `locales/<code>.json`, each key's words `|`-separated per language. The
//! protocol has no screen language of its own, so only the union is read.

use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Every language's words, as compiled in.
const FILES: &[(&str, &str)] = &[
    ("ko", include_str!("../locales/ko.json")),
    ("en", include_str!("../locales/en.json")),
    ("ja", include_str!("../locales/ja.json")),
    ("zh-Hans", include_str!("../locales/zh-Hans.json")),
    ("zh-Hant", include_str!("../locales/zh-Hant.json")),
    ("es", include_str!("../locales/es.json")),
    ("fr", include_str!("../locales/fr.json")),
    ("de", include_str!("../locales/de.json")),
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
}
