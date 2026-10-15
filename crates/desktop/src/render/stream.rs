//! A reply still streaming ends mid-sentence, often mid-markup: `**bo`,
//! `` `cod``, `[a link](htt`. Drawn as is, the markers flash as literal text
//! and then vanish. What is drawn holds the unfinished construct back until
//! the rest arrives, so partial syntax never shows.

/// The part of a streaming reply safe to draw.
pub fn hold_back(text: &str) -> &str {
    let line_start = text.rfind('\n').map_or(0, |at| at + 1);
    let line = &text[line_start..];
    if in_open_fence(&text[..line_start]) {
        return text;
    }
    // A fence being opened: its language may still be half typed.
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        return &text[..line_start];
    }
    match cut_in_line(line) {
        Some(at) => &text[..line_start + at],
        None => text,
    }
}

/// Whether the text before the last line leaves a code fence open.
fn in_open_fence(before: &str) -> bool {
    let mut open: Option<(char, usize)> = None;
    for line in before.lines() {
        let t = line.trim_start();
        let Some(marker) = t.chars().next().filter(|c| matches!(c, '`' | '~')) else { continue };
        let run = t.chars().take_while(|c| *c == marker).count();
        if run < 3 {
            continue;
        }
        open = match open {
            None => Some((marker, run)),
            Some((m, n)) if m == marker && run >= n && t[run..].trim().is_empty() => None,
            other => other,
        };
    }
    open.is_some()
}

/// Where the last line's unfinished construct starts, if it has one.
fn cut_in_line(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = 0;
    // An open code span: where its backticks start, and how many.
    let mut code: Option<(usize, usize)> = None;
    let mut strong: Vec<usize> = Vec::new();
    let mut strike: Vec<usize> = Vec::new();
    // `[` not closed yet; and a link whose target `(` is not closed.
    let mut brackets: Vec<usize> = Vec::new();
    let mut open_link: Option<usize> = None;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'`' {
            let run = bytes[i..].iter().take_while(|c| **c == b'`').count();
            code = match code {
                Some((_, n)) if n == run => None,
                None => Some((i, run)),
                open => open,
            };
            i += run;
            continue;
        }
        if code.is_some() {
            i += 1;
            continue;
        }
        if b == b'\\' {
            i += 2;
            continue;
        }
        match b {
            b'*' | b'_' if bytes.get(i + 1) == Some(&b) => {
                toggle(&mut strong, i);
                i += 2;
                continue;
            }
            b'~' if bytes.get(i + 1) == Some(&b'~') => {
                toggle(&mut strike, i);
                i += 2;
                continue;
            }
            b'[' => brackets.push(if i > 0 && bytes[i - 1] == b'!' { i - 1 } else { i }),
            b']' => {
                if let Some(open) = brackets.pop()
                    && bytes.get(i + 1) == Some(&b'(')
                {
                    // The target, to its `)`; until that comes the whole link waits.
                    match bytes[i + 2..].iter().position(|c| *c == b')') {
                        Some(n) => {
                            i += 2 + n + 1;
                            continue;
                        }
                        None => {
                            open_link = Some(open);
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    let cuts = [code.map(|(at, _)| at), strong.first().copied(), strike.first().copied(), brackets.first().copied(), open_link];
    cuts.into_iter().flatten().min()
}

/// A delimiter opens a span or closes the one open.
fn toggle(open: &mut Vec<usize>, at: usize) {
    if open.pop().is_none() {
        open.push(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfinished_markup_waits_for_the_rest() {
        assert_eq!(hold_back("plain text"), "plain text");
        assert_eq!(hold_back("so **bo"), "so ");
        assert_eq!(hold_back("so **bold** and"), "so **bold** and");
        assert_eq!(hold_back("run `cargo te"), "run ");
        assert_eq!(hold_back("run `cargo test` now"), "run `cargo test` now");
        assert_eq!(hold_back("see [the do"), "see ");
        assert_eq!(hold_back("see [the docs](https://exa"), "see ");
        assert_eq!(hold_back("see [the docs](https://x.y) ok"), "see [the docs](https://x.y) ok");
        assert_eq!(hold_back("look ![shot](/tmp/a.p"), "look ");
        assert_eq!(hold_back("~~gone"), "");
        assert_eq!(hold_back("a `x ** y` b"), "a `x ** y` b", "inside code, no markup");
        assert_eq!(hold_back("**a** and **b"), "**a** and ");
        assert_eq!(hold_back(r"escaped \*\* fine"), r"escaped \*\* fine");
    }

    #[test]
    fn bold_closed_against_korean_is_not_held() {
        let whole = "참고로 backend가 **T-114(이 기기 지우기, 릴레이·데몬 쪽)**를 검토에";
        assert_eq!(hold_back(whole), whole);
        assert_eq!(hold_back("참고로 backend가 **T-114(이 기기"), "참고로 backend가 ");
        assert_eq!(hold_back("참고로 **T-114(이 기기)**"), "참고로 **T-114(이 기기)**", "closed, though no particle yet");
        assert_eq!(hold_back("이것은**(중요)**입니다"), "이것은**(중요)**입니다");
        assert_eq!(hold_back("**[링크](https://x.y)**은 ~~T-2~~는 ~~T-"), "**[링크](https://x.y)**은 ~~T-2~~는 ");
    }

    #[test]
    fn only_the_last_line_is_looked_at() {
        assert_eq!(hold_back("**a\nnext **b"), "**a\nnext ");
        assert_eq!(hold_back("line one\n"), "line one\n");
    }

    #[test]
    fn code_streams_as_it_comes_but_a_fence_opener_waits() {
        assert_eq!(hold_back("```rust\nlet s = \"**"), "```rust\nlet s = \"**", "in a fence, nothing is markup");
        assert_eq!(hold_back("text\n```ru"), "text\n");
        assert_eq!(hold_back("```\ncode\n```\nafter **x"), "```\ncode\n```\nafter ");
    }
}
