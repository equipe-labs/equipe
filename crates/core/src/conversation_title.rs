//! A stable, human-sized name for a conversation's first exchange.

const MAX_CHARS: usize = 40;

/// Ask a small, tool-less turn to name the subject, not repeat the question.
pub fn prompt(user: &str, reply: &str) -> String {
    let user: String = user.chars().take(4_000).collect();
    let reply: String = reply.chars().take(4_000).collect();
    format!(
        "Name the topic of this first conversation exchange. Reply with only a short, distinctive noun phrase in the user's language (ideally 2-7 words, at most 40 characters). Do not copy the question, answer it, add quotes or punctuation, or write a prefix such as Title:.\n\nUser:\n{user}\n\nAssistant:\n{reply}\n\nTopic:"
    )
}

/// A model result is deliberately strict: suspicious output falls back to the
/// person's own words instead of becoming permanent UI chrome.
pub fn clean(generated: &str) -> Option<String> {
    let line = generated
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let mut title = strip_prefix(line).trim();
    title = title.trim_matches(|c: char| {
        matches!(
            c,
            '"' | '\'' | '`' | '“' | '”' | '‘' | '’' | '「' | '」' | '『' | '』'
        )
    });
    title = title
        .trim()
        .trim_end_matches(['.', '!', '?', '。', '！', '？', '…'])
        .trim();
    let title = one_line(title);
    (!title.is_empty() && title.chars().count() <= MAX_CHARS).then_some(title)
}

/// The first user utterance, made fit for a row when generation fails.
pub fn fallback(user: &str) -> String {
    let line = user
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("New conversation");
    let line = strip_prefix(line)
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '`' | '“' | '”' | '‘' | '’' | '「' | '」' | '『' | '』'
            )
        })
        .trim_end_matches(['.', '!', '?', '。', '！', '？', '…'])
        .trim();
    let line = one_line(line);
    clip(&line, MAX_CHARS).unwrap_or_else(|| "New conversation".into())
}

pub fn resolve(generated: Option<&str>, user: &str) -> String {
    generated.and_then(clean).unwrap_or_else(|| fallback(user))
}

fn strip_prefix(line: &str) -> &str {
    for prefix in [
        "Title:", "title:", "TITLE:", "제목:", "題名:", "标题:", "標題:",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return rest;
        }
    }
    line
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(text: &str, max: usize) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= max {
        return Some(text.to_owned());
    }
    let mut cut: String = text.chars().take(max).collect();
    if let Some(space) = cut.rfind(char::is_whitespace)
        && space >= max / 2
    {
        cut.truncate(space);
    }
    Some(cut.trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_titles_are_clean_short_noun_phrases() {
        assert_eq!(
            clean("제목: “세션 제목 생성.”\n설명"),
            Some("세션 제목 생성".into())
        );
        assert_eq!(
            clean("Title: `Conversation titles`"),
            Some("Conversation titles".into())
        );
        assert_eq!(clean("   \n"), None);
        assert_eq!(clean(&"가".repeat(41)), None);
    }

    #[test]
    fn failures_and_overlong_results_fall_back_in_korean_and_english() {
        assert_eq!(
            resolve(None, "대화 제목을 더 찾기 쉽게 바꿔 줄래?"),
            "대화 제목을 더 찾기 쉽게 바꿔 줄래"
        );
        assert_eq!(
            resolve(Some(""), "How can we keep conversation titles stable?"),
            "How can we keep conversation titles"
        );
        assert_eq!(
            resolve(Some(&"x".repeat(41)), "영문과 한국어 fallback을 시험해 줘."),
            "영문과 한국어 fallback을 시험해 줘"
        );
        assert!(fallback("This is a deliberately long English question about conversation titles that should be clipped").chars().count() <= 40);
        assert!(fallback("아주 긴 한국어 질문을 제목으로 정리할 때 사십 자를 절대로 넘기지 않도록 잘라야 합니다").chars().count() <= 40);
    }
}
