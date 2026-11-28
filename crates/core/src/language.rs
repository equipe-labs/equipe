//! The person's language, so everything written for them — replies, tickets,
//! hand-in summaries — is written in it.
//!
//! Detected from what the person types. Scripts settle it at once (Hangul is
//! Korean, kana is Japanese); otherwise a statistical detector proposes, and
//! the little words of the language must agree — a short "ok" says nothing
//! about anyone's language, and neither does a list of nouns. The trigram
//! detector alone read "Please set up our brand kit: logo files, colours, type
//! and voice." as French (0.68 sure), and an English founder got French.

use whatlang::Script;

/// The person's own words in `text`: what comes before a hidden note equipe
/// put under them (a line starting `[equipe`: the project a message from the
/// team web is about, a monitor's nudge) and without the tags of the files
/// they attached (`[image: …]`). The note is English whatever the person
/// writes in, and long: read with their words, a short French message reads
/// as English, and everything the squad wrote for them followed it.
pub fn persons_words(text: &str) -> String {
    // The sender tag the team web puts before a message (`[@akki · Akki S · owner] `) names them, in no language.
    let text = text.trim_start();
    let text = match text.strip_prefix("[@").and_then(|rest| rest.split_once(']')) {
        Some((_, rest)) => rest.trim_start(),
        None => text,
    };
    text.lines()
        .take_while(|l| !l.trim_start().starts_with("[equipe"))
        .filter(|l| {
            let l = l.trim();
            !(l.starts_with('[') && l.ends_with(']') && l[1..].split_once(':').is_some_and(|(tag, _)| !tag.is_empty() && tag.chars().all(|c| c.is_ascii_lowercase())))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

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
    // Latin script: the detector proposes, when the text is long enough to tell;
    // the language's own little words must back it.
    // Fewer than four words say nothing ("ok", "and the app?"): the person's known language stands.
    if text.split_whitespace().count() < 4 {
        return None;
    }
    latin(text, (info.confidence() >= 0.5).then(|| info.lang().eng_name()))
}

/// The Latin-script language `text` is a sentence of by its little words
/// alone, no statistics: one of the languages they are listed for, and only
/// when they clearly lead. For a line inside a reply, where a name, a list
/// of fonts or a product's English words must never read as a sentence.
pub fn sentence_language(text: &str) -> Option<&'static str> {
    if text.split_whitespace().count() < 4 {
        return None;
    }
    latin(text, None)
}

/// The little words of the Latin-script languages people write to a squad in
/// most: words a sentence of that language can hardly do without and the
/// others don't use (no "a", "on", "in", "de", "que": shared by several).
const LITTLE_WORDS: [(&str, &[&str]); 7] = [
    (
        "English",
        &[
            "the", "and", "our", "your", "you", "we", "is", "are", "to", "of", "for", "with", "this", "that", "please", "it", "can", "will", "my",
            "me", "up", "be", "have", "has", "what", "how", "should", "would", "i", "make", "add", "let's", "lets", "do", "does", "from", "about",
            "just", "need", "want", "they", "them", "its", "it's", "an", "at", "by", "or", "but", "not", "so", "if", "there", "here", "set",
        ],
    ),
    (
        "French",
        &[
            "le", "la", "les", "des", "du", "et", "est", "une", "pour", "avec", "nous", "vous", "je", "mon", "ma", "mes", "notre", "votre", "nos",
            "vos", "ce", "cette", "ces", "qui", "sur", "dans", "pas", "ne", "au", "aux", "très", "il", "elle", "merci", "c'est", "j'ai", "être",
            "faire", "fais", "mais", "ou", "aussi", "plus", "tout", "tu", "te", "peux", "pouvez", "s'il", "plaît",
        ],
    ),
    (
        "Spanish",
        &[
            "el", "los", "las", "del", "y", "es", "una", "por", "para", "con", "mi", "nuestro", "nuestra", "está", "gracias", "lo", "se", "al",
            "muy", "pero", "como", "yo", "usted", "hacer", "haz", "también", "esta", "este", "su",
        ],
    ),
    (
        "German",
        &[
            "der", "die", "das", "und", "ist", "ein", "eine", "nicht", "mit", "für", "ich", "wir", "bitte", "unser", "unsere", "auf", "zu", "den",
            "dem", "von", "auch", "sind", "wie", "mach", "mache", "machen", "es", "bei", "noch", "oder", "aber",
        ],
    ),
    (
        "Portuguese",
        &["os", "um", "uma", "não", "com", "nosso", "nossa", "você", "obrigado", "obrigada", "é", "do", "da", "dos", "das", "em", "isso", "faça", "também", "mas"],
    ),
    ("Italian", &["gli", "della", "che", "è", "per", "sono", "nostro", "nostra", "grazie", "il", "non", "anche", "fai", "questo", "questa"]),
    ("Dutch", &["het", "een", "niet", "van", "voor", "ik", "wij", "ons", "onze", "alsjeblieft", "dank", "maak", "ook", "zijn", "wat"]),
];

/// How many of `language`'s little words `words` hold.
fn little(words: &[String], language: &str) -> usize {
    LITTLE_WORDS.iter().find(|(name, _)| *name == language).map_or(0, |(_, list)| words.iter().filter(|w| list.contains(&w.as_str())).count())
}

/// The language of a Latin-script `text`, given what the detector `proposed`:
/// taken when its little words back it (two at least, and no fewer than any
/// other language's); a language the list doesn't know, taken when no listed
/// language shows; else the language whose little words clearly lead (two
/// with no rival, or three and two ahead); else none — too little to tell, so the person's known language stands.
fn latin(text: &str, proposed: Option<&'static str>) -> Option<&'static str> {
    let words: Vec<String> = text.to_lowercase().split(|c: char| !(c.is_alphabetic() || c == '\'' || c == '’')).map(|w| w.replace('’', "'")).filter(|w| !w.is_empty()).collect();
    let mut counts: Vec<(&'static str, usize)> = LITTLE_WORDS.iter().map(|(name, _)| (*name, little(&words, name))).collect();
    counts.sort_by_key(|c| std::cmp::Reverse(c.1));
    let (best, most) = counts[0];
    let next = counts[1].1;
    if let Some(name) = proposed {
        if !LITTLE_WORDS.iter().any(|(n, _)| *n == name) && most < 2 {
            return Some(name);
        }
        let backed = little(&words, name);
        if backed >= 2 && backed >= most {
            return Some(name);
        }
    }
    ((most >= 3 && most >= next + 2) || (most >= 2 && next == 0)).then_some(best)
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
    fn a_hidden_note_and_attachment_tags_are_not_the_persons_words() {
        let said = "Rends le site plus rapide\n\n[image: /home/x/uploads/a.png]\n\n[equipe: project shop. The person wrote this while looking at the project \"Shop\" (app name shop), live at https://shop.example in the team web, so it is about that product.]";
        assert_eq!(persons_words(said).trim(), "Rends le site plus rapide");
        assert_eq!(persons_words("[equipe monitor] Project shop has no confirmed goal yet. Call propose_goal once."), "");
        assert_eq!(detect(&persons_words("[equipe monitor] Weekly review of the shop, last seven days, with the numbers below")), None);
    }

    #[test]
    fn english_the_trigram_detector_reads_as_french_is_english() {
        // ai-based-0k09, 2026-11-27: a suggested request, in English, made the PM write in French.
        let said = "[@akki · akki shafiqullah · owner] Please set up our brand kit: logo files, colours, type and voice.";
        assert_eq!(persons_words(said), "Please set up our brand kit: logo files, colours, type and voice.");
        assert_eq!(detect(&persons_words(said)), Some("English"));
        assert_eq!(detect("Please set up our brand kit: logo files, colors, type and voice."), Some("English"));
        // Nouns alone say nothing: the known language stands.
        assert_eq!(detect("AI based interview prep platform for IB MBB etc"), None);
        assert_eq!(detect("logo files, colours, type, voice"), None);
        assert_eq!(detect("and the app?"), None, "three words say nothing");
    }

    #[test]
    fn a_language_switch_needs_the_persons_own_sentence_in_it() {
        assert_eq!(detect("Rends le site plus rapide"), Some("French"));
        assert_eq!(detect("Peux-tu mettre le logo en haut de la page, s'il te plaît ?"), Some("French"));
        assert_eq!(detect("Haz que la página cargue más rápido, por favor"), Some("Spanish"));
        assert_eq!(detect("Bitte mach die Seite schneller und das Logo größer"), Some("German"));
        assert_eq!(detect("Can you make the logo bigger on the home page?"), Some("English"));
    }

    #[test]
    fn mixed_korean_and_code_is_still_korean() {
        assert_eq!(detect("T-2 승인할게. `npm run build` 돌려봤어?"), Some("Korean"));
    }
}
