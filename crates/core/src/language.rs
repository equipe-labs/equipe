//! The person's language, so everything written for them — replies, tickets,
//! hand-in summaries — is written in it.
//!
//! Detected from what the person types. Scripts settle it at once (Hangul is
//! Korean, kana is Japanese); otherwise a statistical detector decides, and only
//! when it is sure — a short "ok" says nothing about anyone's language.

use whatlang::Script;

/// The language `text` is written in, by its English name: `Korean`, `French`.
pub fn detect(text: &str) -> Option<&'static str> {
    // Scripts first, counted by hand: a message mixing Hangul with code or
    // English words is still Korean, however many Latin letters it has.
    let count = |range: &dyn Fn(char) -> bool| text.chars().filter(|c| range(*c)).count();
    if count(&|c| ('\u{AC00}'..='\u{D7A3}').contains(&c) || ('\u{1100}'..='\u{11FF}').contains(&c)) >= 2 {
        return Some("Korean");
    }
    if count(&|c| ('\u{3040}'..='\u{30FF}').contains(&c)) >= 2 {
        return Some("Japanese");
    }
    if count(&|c| ('\u{4E00}'..='\u{9FFF}').contains(&c)) >= 2 {
        return Some("Chinese");
    }
    let info = whatlang::detect(text)?;
    match info.script() {
        Script::Hiragana | Script::Katakana => return Some("Japanese"),
        Script::Cyrillic | Script::Arabic | Script::Hebrew | Script::Greek | Script::Thai | Script::Devanagari => {
            return Some(info.lang().eng_name());
        }
        _ => {}
    }
    // Latin script: the detector decides, when the text is long enough to tell.
    (info.confidence() >= 0.5 && text.split_whitespace().count() >= 4).then(|| info.lang().eng_name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_decide_at_once() {
        assert_eq!(detect("웹사이트 만들자"), Some("Korean"));
        assert_eq!(detect("ウェブサイトを作ろう"), Some("Japanese"));
    }

    #[test]
    fn latin_languages_need_a_sure_detector_and_a_few_words() {
        assert_eq!(detect("Let's build the website for the launch next week"), Some("English"));
        assert_eq!(detect("Construisons le site web pour le lancement de la semaine prochaine"), Some("French"));
        assert_eq!(detect("ok"), None, "too short to say");
    }

    #[test]
    fn mixed_korean_and_code_is_still_korean() {
        assert_eq!(detect("T-2 승인할게. `npm run build` 돌려봤어?"), Some("Korean"));
    }
}
