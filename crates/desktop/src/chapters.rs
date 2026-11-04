//! A conversation's chapters, for the rail at its left: a bar for each
//! thing the person asked — their message's first line, and how the answer
//! began — the one being read drawn longer; over it, the list, and a click
//! goes there (`views/chapters.rs`). A word or two ("ok", "ㄱㄱ") goes on the
//! chapter it answers rather than starting one. No UI types here.

use crate::convo::Block;

/// A chapter: the row its message is on, and what it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    pub row: usize,
    pub title: String,
    /// How the reply to it began, if it has one yet.
    pub more: Option<String>,
}

/// Fewer characters than this ask nothing new: a go-ahead, a thanks.
const SHORTEST: usize = 8;
/// A title's longest, and the answer's.
const TITLE: usize = 64;
const MORE: usize = 96;
/// Most bars on the rail: past that, those around the one being read.
pub const MOST: usize = 14;

/// The chapters of `blocks`, in order.
pub fn of(blocks: &[Block]) -> Vec<Chapter> {
    let mut out: Vec<Chapter> = Vec::new();
    for (row, block) in blocks.iter().enumerate() {
        match block {
            Block::User(text) if crate::decide::decided(text).is_none() => {
                let line = first_line(text);
                if line.chars().count() >= SHORTEST || out.is_empty() {
                    out.push(Chapter { row, title: cut(&line, TITLE), more: None });
                }
            }
            Block::Reply { text, .. } => {
                if let Some(last) = out.last_mut().filter(|c| c.more.is_none()) {
                    let line = first_line(&crate::render::prose::spoken(text));
                    last.more = (!line.is_empty()).then(|| cut(&line, MORE));
                }
            }
            _ => {}
        }
    }
    out
}

/// The chapter being read, with `top` the row at the top of the view: the
/// last that starts at or above it.
pub fn current(chapters: &[Chapter], top: usize) -> Option<usize> {
    chapters.iter().rposition(|c| c.row <= top).or((!chapters.is_empty()).then_some(0))
}

/// The chapters with a bar: all, or [`MOST`] around the one being read.
pub fn barred(len: usize, current: Option<usize>) -> std::ops::Range<usize> {
    if len <= MOST {
        return 0..len;
    }
    let at = current.unwrap_or(len - 1);
    let start = at.saturating_sub(MOST / 2).min(len - MOST);
    start..start + MOST
}

fn first_line(text: &str) -> String {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().trim_start_matches(['#', '>', '-', '*', ' ']).to_owned()
}

fn cut(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        return text.to_owned();
    }
    let kept: String = text.chars().take(most - 1).collect();
    format!("{}…", kept.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(text: &str) -> Block {
        Block::Reply { text: text.into(), first: false, live: false, worked: None }
    }

    #[test]
    fn each_thing_the_person_asked_is_a_chapter_and_a_go_ahead_is_not() {
        let blocks = [
            Block::User("우리 PM 대화 여러 개 구현해줘\n자세히는 이렇게".into()),
            Block::Steps(vec![]),
            reply("## 좋아요\n\n**한 PM**에 대화 여러 개를 붙일게요."),
            Block::User("ㄱㄱ".into()),
            reply("시작했어요."),
            Block::User("대화 속 그림 수정해줘".into()),
        ];
        let chapters = of(&blocks);
        assert_eq!(chapters.len(), 2, "{chapters:#?}");
        assert_eq!(chapters[0], Chapter { row: 0, title: "우리 PM 대화 여러 개 구현해줘".into(), more: Some("좋아요 한 PM에 대화 여러 개를 붙일게요.".into()) }, "how the answer began, without its marks");
        assert_eq!(chapters[1].row, 5);
        assert_eq!(chapters[1].more, None, "not answered yet");
        assert_eq!(of(&[Block::User("ok".into())]).len(), 1, "the first message starts the conversation, however short");
        let long = "a".repeat(200);
        assert!(of(&[Block::User(long)])[0].title.ends_with('…'));
    }

    #[test]
    fn the_chapter_read_is_the_last_begun_above_and_the_rail_keeps_to_those_around_it() {
        let chapters: Vec<Chapter> = [0, 4, 9].iter().map(|&row| Chapter { row, title: String::new(), more: None }).collect();
        assert_eq!(current(&chapters, 0), Some(0));
        assert_eq!(current(&chapters, 6), Some(1));
        assert_eq!(current(&chapters, 40), Some(2));
        assert_eq!(current(&[], 3), None);
        assert_eq!(barred(5, Some(2)), 0..5);
        assert_eq!(barred(40, Some(20)), 13..27);
        assert_eq!(barred(40, Some(1)), 0..MOST);
        assert_eq!(barred(40, None), 26..40, "the latest");
    }
}
