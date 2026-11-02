//! A presentation a member made: Markdown slides as Marp writes them — a
//! front matter with `marp: true`, slides split by lines of `---` — so the
//! same file also opens in Marp's tools (PDF, PowerPoint). Shown one slide at
//! a time over the window (`views/deck.rs`). No UI types here.

use std::path::Path;

/// A deck rather than a document: Marp's front matter says so, or its name
/// does (`deck.md`, `slides.md`, `launch.deck.md`, `q3-slides.md`).
pub fn is_deck(path: &Path, text: &str) -> bool {
    let marp = front_matter(text).0.is_some_and(|yaml| yaml.lines().any(|l| l.trim().replace(' ', "") == "marp:true"));
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let named = ["deck", "slides"].iter().any(|word| stem == *word || stem.ends_with(&format!(".{word}")) || stem.ends_with(&format!("-{word}")) || stem.ends_with(&format!("_{word}")));
    marp || named
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

#[cfg(test)]
mod tests {
    use super::*;

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
