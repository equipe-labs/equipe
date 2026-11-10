//! A presentation a member made: Markdown slides as Marp writes them — a
//! front matter with `marp: true`, slides split by lines of `---` — so the
//! same file also opens in Marp's tools (PDF, PowerPoint). Shown one slide at
//! a time over the window (`views/deck.rs`). No UI types here.

use std::path::Path;

/// A deck rather than a document: Marp's front matter says so, or its name
/// does (`deck.md`, `slides.md`, `launch.deck.md`, `q3-slides.md`).
pub fn is_deck(path: &Path, text: &str) -> bool {
    let marp = front_matter(text).0.is_some_and(|yaml| yaml.lines().any(|l| l.trim().replace(' ', "") == "marp:true"));
    marp || named(path)
}

/// A deck by its name alone — told before it is read.
pub fn named(path: &Path) -> bool {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    crate::doc::is_doc(&path.to_string_lossy())
        && ["deck", "slides"].iter().any(|word| stem == *word || stem.ends_with(&format!(".{word}")) || stem.ends_with(&format!("-{word}")) || stem.ends_with(&format!("_{word}")))
}

/// The deck a group of calls wrote, last first: shown as a presentation one
/// click from the group, however much else it did.
pub fn written(lines: &[crate::convo::ToolLine]) -> Option<&str> {
    use crate::convo::Status;
    lines.iter().rev().filter(|l| l.edits && !matches!(l.status, Status::Running | Status::Failed | Status::Declined)).filter_map(|l| l.path.as_deref()).find(|path| named(Path::new(path)))
}

/// The front matter's YAML (between the first two `---` lines, at the very
/// top), and the text after it.
pub fn front_matter(text: &str) -> (Option<&str>, &str) {
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else { return (None, text) };
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return (Some(&rest[..at]), &rest[at + line.len()..]);
        }
        at += line.len();
    }
    (None, text)
}

/// What a reader is shown: the text without its front matter — settings for
/// the tools, not words for the person.
pub fn body(text: &str) -> &str {
    front_matter(text).1
}

/// The slides, in order: split at each `---` line outside a code fence, with
/// HTML comments (Marp's directives, a speaker's notes) left out and empty
/// slides dropped.
pub fn slides(text: &str) -> Vec<String> {
    let mut slides = vec![String::new()];
    let mut fence: Option<String> = None;
    let mut comment = false;
    for line in body(text).lines() {
        let trimmed = line.trim();
        if comment {
            comment = !trimmed.contains("-->");
            continue;
        }
        match &fence {
            Some(open) => {
                if trimmed.starts_with(open.as_str()) {
                    fence = None;
                }
            }
            None => {
                if trimmed == "---" {
                    slides.push(String::new());
                    continue;
                }
                if let Some(open) = ["```", "~~~"].iter().find(|f| trimmed.starts_with(**f)) {
                    fence = Some((*open).to_owned());
                } else if trimmed.starts_with("<!--") {
                    comment = !trimmed.contains("-->");
                    continue;
                }
            }
        }
        let slide = slides.last_mut().expect("there is always a slide");
        slide.push_str(line);
        slide.push('\n');
    }
    slides.into_iter().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()
}

/// What a slide is made of, top to bottom: words, pictures side by side, or
/// columns (a row of figures, each a big first line and what it counts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Words(String),
    Pictures(Vec<String>),
    Columns(Vec<Vec<String>>),
}

/// A slide read into its parts. Marp's decks put pictures on lines of their
/// own (`![h:470](img/a.png)`) or in HTML (`<img src="img/a.png">` in a row of
/// `<div>`s) — both are pictures here, sized to the slide; the other HTML is
/// its words, each line a column.
pub fn parts(slide: &str) -> Vec<Part> {
    let mut parts: Vec<Part> = Vec::new();
    let mut words = String::new();
    let mut lines = slide.lines().peekable();
    let flush = |words: &mut String, parts: &mut Vec<Part>| {
        if !words.trim().is_empty() {
            parts.push(Part::Words(words.trim().to_owned()));
        }
        words.clear();
    };
    while let Some(line) = lines.next() {
        if is_html(line) {
            let mut block = vec![line];
            while let Some(next) = lines.next_if(|l| is_html(l)) {
                block.push(next);
            }
            flush(&mut words, &mut parts);
            let pictures: Vec<String> = block.iter().flat_map(|l| html_pictures(l)).collect();
            if !pictures.is_empty() {
                push(&mut parts, Part::Pictures(pictures));
            }
            let columns: Vec<Vec<String>> = block.iter().map(|l| html_words(l)).filter(|w| !w.is_empty()).collect();
            match columns.len() {
                0 => {}
                1 => parts.push(Part::Words(columns[0].join("\n\n"))),
                _ => parts.push(Part::Columns(columns)),
            }
        } else if let Some(pictures) = markdown_pictures(line) {
            flush(&mut words, &mut parts);
            push(&mut parts, Part::Pictures(pictures));
        } else {
            words.push_str(line);
            words.push('\n');
        }
    }
    flush(&mut words, &mut parts);
    parts
}

/// Pictures one after another are one row.
fn push(parts: &mut Vec<Part>, part: Part) {
    match (parts.last_mut(), part) {
        (Some(Part::Pictures(row)), Part::Pictures(more)) => row.extend(more),
        (_, part) => parts.push(part),
    }
}

/// A line of HTML: a tag opening it (not a Markdown link in `<…>`).
fn is_html(line: &str) -> bool {
    let line = line.trim_start();
    let mut chars = line.chars().skip(1);
    line.starts_with('<') && line.contains('>') && chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '/') && !line.starts_with("<http")
}

/// The `src` of each `<img>` on a line.
fn html_pictures(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.to_ascii_lowercase().find("<img") {
        let tag = &rest[at..rest[at..].find('>').map_or(rest.len(), |end| at + end)];
        if let Some(src) = attribute(tag, "src") {
            found.push(src.to_owned());
        }
        rest = &rest[at + tag.len().max(4)..];
    }
    found
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.to_ascii_lowercase().find(&format!("{name}="))? + name.len() + 1;
    let value = &tag[at..];
    let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value = &value[1..];
    value.find(quote).map(|end| &value[..end])
}

/// The words of a line of HTML, a piece for each stretch between tags.
fn html_words(line: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut piece = String::new();
    let mut in_tag = false;
    for c in line.chars() {
        match c {
            '<' => {
                in_tag = true;
                pieces.push(std::mem::take(&mut piece));
            }
            '>' if in_tag => in_tag = false,
            c if !in_tag => piece.push(c),
            _ => {}
        }
    }
    pieces.push(piece);
    let unescape = |s: &str| s.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&");
    pieces.iter().map(|p| unescape(p.trim())).filter(|p| !p.is_empty()).collect()
}

/// A line of nothing but Markdown pictures: where each points.
fn markdown_pictures(line: &str) -> Option<Vec<String>> {
    let mut rest = line.trim();
    let mut found = Vec::new();
    while !rest.is_empty() {
        let after_alt = rest.strip_prefix("![")?.split_once("](")?.1;
        let (src, after) = match after_alt.strip_prefix('<') {
            Some(bracketed) => {
                let (src, after) = bracketed.split_once('>')?;
                (src, after.split_once(')')?.1)
            }
            None => {
                let (inside, after) = after_alt.split_once(')')?;
                (inside.split_whitespace().next()?, after)
            }
        };
        found.push(src.to_owned());
        rest = after.trim_start();
    }
    (!found.is_empty()).then_some(found)
}

/// Where a picture on a deck at `deck` is: beside the deck, unless it says
/// where on its own (a full path, a web address).
pub fn picture_at(deck: &Path, src: &str) -> String {
    if src.contains("://") || src.starts_with('/') {
        return src.to_owned();
    }
    deck.parent().unwrap_or(Path::new("")).join(src).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slide_of_marps_pictures_and_figures_is_shown_as_pictures_and_columns() {
        let slide = "## You only approve what matters\n\n<div class=\"row\">\n<img src=\"img/3-review-en.png\" style=\"height:460px\">\n<img src='img/2-approve-en.png'>\n</div>\n\nMerges wait for your tap.";
        assert_eq!(
            parts(slide),
            [
                Part::Words("## You only approve what matters".into()),
                Part::Pictures(vec!["img/3-review-en.png".into(), "img/2-approve-en.png".into()]),
                Part::Words("Merges wait for your tap.".into()),
            ]
        );
        let slide = "## Proof\n\n<div class=\"row\">\n<div class=\"stat\"><div class=\"n\">264</div><div class=\"l\">tickets done</div></div>\n<div class=\"stat\"><div class=\"n\">138</div><div class=\"l\">releases &amp; more</div></div>\n</div>\n\nAll **shipped**.";
        assert_eq!(
            parts(slide),
            [
                Part::Words("## Proof".into()),
                Part::Columns(vec![vec!["264".into(), "tickets done".into()], vec!["138".into(), "releases & more".into()]]),
                Part::Words("All **shipped**.".into()),
            ]
        );
        let slide = "## It's live\n\n![h:450](img/site-hero.png)\n![bg right](<img/a b.png>) ![](https://x.io/c.png \"c\")\n\nOpen source · <https://equipe.im>";
        assert_eq!(
            parts(slide),
            [
                Part::Words("## It's live".into()),
                Part::Pictures(vec!["img/site-hero.png".into(), "img/a b.png".into(), "https://x.io/c.png".into()]),
                Part::Words("Open source · <https://equipe.im>".into()),
            ],
            "Marp's sizes in the alt are its own; a link in <…> is words"
        );
        assert_eq!(parts("# Just words\n\nsee ![x](a.png) here"), [Part::Words("# Just words\n\nsee ![x](a.png) here".into())], "a picture in a sentence stays in it");
        assert_eq!(parts("<div class=\"big\">Hi</div>"), [Part::Words("Hi".into())]);
    }

    #[test]
    fn a_deck_written_among_other_calls_is_one_click_from_them() {
        use crate::convo::Status;
        use crate::convo::ToolLine;
        let line = |verb: &str, path: &str, edits: bool, status: Status| ToolLine { path: Some(path.into()), status, ..ToolLine::new(verb, vec![path.into()], edits) };
        let lines = [
            line("Ran", "ls", false, Status::Done),
            line("Read", "/h/notes/pitch/demo-day-deck/deck.md", false, Status::Done),
            line("Wrote", "/h/notes/pitch/demo-day-deck/deck.md", true, Status::Done),
            line("Edited", "/h/notes/plan.md", true, Status::Done),
        ];
        assert_eq!(written(&lines), Some("/h/notes/pitch/demo-day-deck/deck.md"));
        assert_eq!(written(&lines[..2]), None, "read, not written");
        assert_eq!(written(&[line("Wrote", "/h/deck.md", true, Status::Failed)]), None);
        assert_eq!(written(&[line("Wrote", "/h/launch-slides.md", true, Status::Done)]), Some("/h/launch-slides.md"));
        assert!(!named(Path::new("/h/deck.png")) && !named(Path::new("/h/deck")));
    }

    #[test]
    fn a_picture_is_found_beside_its_deck() {
        let deck = Path::new("/h/notes/pitch/deck.md");
        assert_eq!(picture_at(deck, "img/a.png"), "/h/notes/pitch/img/a.png");
        assert_eq!(picture_at(deck, "/abs/b.png"), "/abs/b.png");
        assert_eq!(picture_at(deck, "https://x.io/c.png"), "https://x.io/c.png");
    }

    const DECK: &str = "---\nmarp: true\ntheme: default\n---\n\n# Design systems\n\nWhat we found\n\n---\n\n<!-- _class: lead -->\n## The answer\n\n- one\n- two\n\n<!--\nSpeaker notes:\nsay it slowly\n-->\n\n---\n\n```yaml\nkey: value\n---\n```\n\n---\n\n";

    #[test]
    fn marps_front_matter_or_the_name_makes_a_deck() {
        assert!(is_deck(Path::new("/s/notes/reports/x/report.md"), DECK));
        assert!(is_deck(Path::new("deck.md"), "# a"));
        assert!(is_deck(Path::new("/x/launch.deck.md"), "# a"));
        assert!(is_deck(Path::new("/x/q3-slides.md"), "# a"));
        assert!(!is_deck(Path::new("/x/report.md"), "# a\n\n---\n\nb"));
        assert!(!is_deck(Path::new("/x/redeck.md"), "# a"));
        assert!(!is_deck(Path::new("/x/report.md"), "---\ntitle: x\n---\n# a"));
    }

    #[test]
    fn slides_split_at_rules_outside_fences_without_comments() {
        let slides = slides(DECK);
        assert_eq!(slides.len(), 3, "{slides:#?}");
        assert_eq!(slides[0], "# Design systems\n\nWhat we found");
        assert_eq!(slides[1], "## The answer\n\n- one\n- two", "directives and notes are not shown");
        assert_eq!(slides[2], "```yaml\nkey: value\n---\n```", "a rule in a fence is the code's");
    }

    #[test]
    fn a_reader_never_sees_the_front_matter() {
        assert_eq!(body(DECK).trim_start().lines().next(), Some("# Design systems"));
        assert_eq!(body("# no front matter\n---\nx"), "# no front matter\n---\nx");
        assert_eq!(body("---\nunclosed"), "---\nunclosed");
        assert_eq!(slides("# one slide"), ["# one slide"]);
    }
}
