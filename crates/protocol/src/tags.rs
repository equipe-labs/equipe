//! The attachment tags a message carries: what a screen attached, as a file on
//! the daemon's machine and a tag in the text — `[image: p]`, `[pdf: p · 12
//! pages]`, `[file: p · 3 lines]`, `[audio: p · 0:31]`, `[video: p · 1:42 ·
//! 1920×1080 · sheet: s · frames: f@0:03, …]`. The engines read the text as it
//! is; a screen finds the tags here to show the attachments themselves and
//! the words without them.

use std::ops::Range;

/// What a tag says it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagKind {
    Image,
    Video,
    Audio,
    Pdf,
    /// Text, code, documents — anything else.
    File,
}

impl TagKind {
    fn of(word: &str) -> Option<Self> {
        Some(match word {
            "image" => Self::Image,
            "video" => Self::Video,
            "audio" => Self::Audio,
            "pdf" => Self::Pdf,
            "file" => Self::File,
            _ => return None,
        })
    }
}

/// One attachment tag found in a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub kind: TagKind,
    /// Its path on the daemon's machine.
    pub path: String,
    pub pages: Option<usize>,
    pub lines: Option<usize>,
    /// A video's or a sound's length, as written (`1:42`).
    pub duration: Option<String>,
    /// A video's picture size.
    pub dims: Option<(u32, u32)>,
    /// A video's sheet of frames.
    pub sheet: Option<String>,
    /// A video's frames: each one's path and the time it was taken at.
    pub frames: Vec<(String, String)>,
    /// Where it is in the text, brackets included (bytes).
    pub range: Range<usize>,
}

impl Tag {
    /// The file's name as the person knows it: without the folder, and
    /// without the stamp equipe put in front (`1792073093606-paste.png` →
    /// `paste.png`).
    pub fn name(&self) -> &str {
        display_name(&self.path)
    }

    /// The picture that stands for it: the image itself, a video's first
    /// frame (else its sheet).
    pub fn picture(&self) -> Option<&str> {
        match self.kind {
            TagKind::Image => Some(&self.path),
            TagKind::Video => self.frames.first().map(|(p, _)| p.as_str()).or(self.sheet.as_deref()),
            _ => None,
        }
    }
}

/// A path's file name without the stamp an upload is named with.
pub fn display_name(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.split_once('-') {
        Some((stamp, rest)) if stamp.len() >= 10 && stamp.bytes().all(|b| b.is_ascii_digit()) && !rest.is_empty() => rest,
        _ => name,
    }
}

/// Every attachment tag in `text`, in order. A bracket that isn't one — an
/// unknown word, a path that isn't absolute, no closing bracket on the line —
/// stays text.
pub fn find(text: &str) -> Vec<Tag> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(open) = text[at..].find('[').map(|o| at + o) {
        match tag_at(text, open) {
            Some(tag) => {
                at = tag.range.end;
                out.push(tag);
            }
            None => at = open + 1,
        }
    }
    out
}

/// The tag that opens at `open` (a `[`), if it is one.
fn tag_at(text: &str, open: usize) -> Option<Tag> {
    let after = &text[open + 1..];
    let (word, rest) = after.split_once(": ")?;
    let kind = TagKind::of(word)?;
    // A tag stays on its line. Its end is the first `]` followed by a space,
    // the end, or another tag — so a `]` inside a path doesn't cut it short —
    // else the first `]`.
    let line = &rest[..rest.find('\n').unwrap_or(rest.len())];
    let closes: Vec<usize> = line.match_indices(']').map(|(i, _)| i).collect();
    let close = closes
        .iter()
        .copied()
        .find(|&i| line[i + 1..].chars().next().is_none_or(|c| c.is_whitespace() || c == '['))
        .or_else(|| closes.first().copied())?;
    let body = &line[..close];
    let mut parts = body.split(" · ");
    let path = parts.next()?.trim();
    if !path.starts_with('/') || path.len() < 2 {
        return None;
    }
    let start = open;
    let end = open + 1 + word.len() + 2 + close + 1;
    let mut tag = Tag { kind, path: path.to_owned(), pages: None, lines: None, duration: None, dims: None, sheet: None, frames: Vec::new(), range: start..end };
    for part in parts {
        let part = part.trim();
        if let Some(sheet) = part.strip_prefix("sheet: ") {
            tag.sheet = Some(sheet.to_owned());
        } else if let Some(list) = part.strip_prefix("frames: ") {
            tag.frames = list.split(", ").filter_map(|f| f.rsplit_once('@')).map(|(p, at)| (p.to_owned(), at.to_owned())).collect();
        } else if let Some(n) = part.strip_suffix(" pages").or_else(|| part.strip_suffix(" page")).and_then(|n| n.parse().ok()) {
            tag.pages = Some(n);
        } else if let Some(n) = part.strip_suffix(" lines").or_else(|| part.strip_suffix(" line")).and_then(|n| n.parse().ok()) {
            tag.lines = Some(n);
        } else if is_clock(part) {
            tag.duration = Some(part.to_owned());
        } else if let Some((w, h)) = part.split_once('×').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) {
            tag.dims = Some((w, h));
        }
    }
    Some(tag)
}

/// `0:31`, `1:42`, `1:02:03`.
fn is_clock(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    (2..=3).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) && parts[1..].iter().all(|p| p.len() == 2)
}

/// A message read apart: the words without the tags, and the tags.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Split {
    pub text: String,
    pub tags: Vec<Tag>,
}

/// `text` without its attachment tags (the spaces they leave tidied), and
/// the tags.
pub fn split(text: &str) -> Split {
    let tags = find(text);
    if tags.is_empty() {
        return Split { text: text.to_owned(), tags };
    }
    Split { text: replace(text, &tags, |_| String::new()), tags }
}

/// `text` with each tag in its place as `label` words it (the terminal's
/// `[이미지] paste.png`). Spaces a removed tag leaves are tidied.
pub fn compact(text: &str, label: impl Fn(&Tag) -> String) -> String {
    let tags = find(text);
    if tags.is_empty() {
        return text.to_owned();
    }
    replace(text, &tags, label)
}

fn replace(text: &str, tags: &[Tag], label: impl Fn(&Tag) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for tag in tags {
        out.push_str(&text[at..tag.range.start]);
        at = tag.range.end;
        let words = label(tag);
        if words.is_empty() {
            // Gone: the space after it goes too when nothing (or a space) is before it.
            if text[at..].starts_with(' ') && (out.is_empty() || out.ends_with([' ', '\n'])) {
                at += 1;
            }
        } else {
            out.push_str(&words);
        }
    }
    out.push_str(&text[at..]);
    let lines: Vec<&str> = out.split('\n').map(str::trim_end).collect();
    lines.join("\n").trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_reads_apart_into_words_and_tags() {
        let s = split("pair 가 잘 안되는데 해결좀... [image: /Users/abel/.equipe/uploads/1792073093606-paste.png]");
        assert_eq!(s.text, "pair 가 잘 안되는데 해결좀...");
        assert_eq!(s.tags.len(), 1);
        assert_eq!(s.tags[0].kind, TagKind::Image);
        assert_eq!(s.tags[0].path, "/Users/abel/.equipe/uploads/1792073093606-paste.png");
        assert_eq!(s.tags[0].name(), "paste.png");
    }

    #[test]
    fn several_tags_mid_text_and_alone() {
        let s = split("look [image: /u/a.png] and [pdf: /u/spec.pdf · 12 pages] then this");
        assert_eq!(s.text, "look and then this");
        assert_eq!(s.tags.iter().map(|t| t.kind).collect::<Vec<_>>(), [TagKind::Image, TagKind::Pdf]);
        assert_eq!(s.tags[1].pages, Some(12));
        let alone = split("[image: /u/a.png] [image: /u/b.png]");
        assert_eq!(alone.text, "");
        assert_eq!(alone.tags.len(), 2);
        let lines = split("first line [image: /u/a.png]\nsecond");
        assert_eq!(lines.text, "first line\nsecond");
    }

    #[test]
    fn a_path_with_spaces_or_brackets_stays_whole() {
        let s = split("see [image: /Users/a b/My Shots/screen shot 1.png] ok");
        assert_eq!(s.tags[0].path, "/Users/a b/My Shots/screen shot 1.png");
        assert_eq!(s.text, "see ok");
        let bracket = split("[file: /u/notes [draft].md · 3 lines] done");
        assert_eq!(bracket.tags[0].path, "/u/notes [draft].md");
        assert_eq!(bracket.tags[0].lines, Some(3));
        assert_eq!(bracket.text, "done");
        let punct = split("this one [image: /u/a.png].");
        assert_eq!(punct.tags[0].path, "/u/a.png");
        assert_eq!(punct.text, "this one .");
    }

    #[test]
    fn what_is_not_a_tag_stays_text() {
        for text in ["[link: /u/a.png]", "[image: relative.png]", "[image: /u/a.png", "a [b] c", "[image:/u/a.png]", "[Image: /u/a.png]", "[image: /u/a\n.png]"] {
            let s = split(text);
            assert!(s.tags.is_empty(), "{text}");
            assert_eq!(s.text, text);
        }
        let s = split("[todo: x] [image: /u/a.png]");
        assert_eq!(s.text, "[todo: x]");
        assert_eq!(s.tags.len(), 1);
    }

    #[test]
    fn a_videos_tag_says_its_frames_sheet_and_length() {
        let s = split("[video: /u/a.mov · 1:42 · 1920×1080 · sheet: /u/s.jpg · frames: /u/f1.jpg@0:03, /u/f2.jpg@0:15]");
        let v = &s.tags[0];
        assert_eq!(v.kind, TagKind::Video);
        assert_eq!(v.duration.as_deref(), Some("1:42"));
        assert_eq!(v.dims, Some((1920, 1080)));
        assert_eq!(v.sheet.as_deref(), Some("/u/s.jpg"));
        assert_eq!(v.frames, [("/u/f1.jpg".to_owned(), "0:03".to_owned()), ("/u/f2.jpg".to_owned(), "0:15".to_owned())]);
        assert_eq!(v.picture(), Some("/u/f1.jpg"));
        let audio = &split("[audio: /u/m.m4a · 0:31]").tags[0];
        assert_eq!((audio.kind, audio.duration.as_deref()), (TagKind::Audio, Some("0:31")));
        let file = &split("[file: /u/a.rs · 1 line]").tags[0];
        assert_eq!(file.lines, Some(1));
        assert_eq!(split("[video: /u/a.mov]").tags[0].picture(), None);
    }

    #[test]
    fn the_terminal_shows_each_tag_compactly() {
        let shown = compact("봐 줘 [image: /u/1792073093606-paste.png] [pdf: /u/1-spec.pdf · 2 pages]", |t| format!("[{}]", t.name()));
        assert_eq!(shown, "봐 줘 [paste.png] [1-spec.pdf]");
        assert_eq!(compact("no tags [here]", |_| "x".into()), "no tags [here]");
    }

    #[test]
    fn a_name_loses_its_stamp_only() {
        assert_eq!(display_name("/u/1792073093606-paste.png"), "paste.png");
        assert_eq!(display_name("/u/1792073093606-a-b.png"), "a-b.png");
        assert_eq!(display_name("/u/my-file.png"), "my-file.png");
        assert_eq!(display_name("/u/123-x.png"), "123-x.png");
        assert_eq!(display_name("plain.png"), "plain.png");
    }
}
