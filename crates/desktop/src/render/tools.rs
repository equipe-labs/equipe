//! A tool step's outcome as the conversation shows it: an edit's change cut
//! to a card, a command's output to its last lines, a duration in a word. No
//! UI types here.

use crate::diffs::{self, Kind, Line};

/// Lines of a command's output shown before it is opened.
pub const OUTPUT_PREVIEW: usize = 4;
/// Lines of an edit's change shown on its card before it is opened.
pub const DIFF_PREVIEW: usize = 8;

/// An edit's change, ready for its card: the file, how many lines came and
/// went, and the lines to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffCard {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    /// The change's lines, hunk headers between hunks (not before the first).
    pub lines: Vec<Line>,
}

impl DiffCard {
    /// The lines shown closed: the first hunk's start.
    pub fn preview(&self) -> &[Line] {
        let first_hunk_end = self.lines.iter().position(|l| l.kind == Kind::Hunk).unwrap_or(self.lines.len());
        &self.lines[..first_hunk_end.min(DIFF_PREVIEW)]
    }

    /// How many lines opening the card adds.
    pub fn hidden(&self) -> usize {
        self.lines.len() - self.preview().len()
    }
}

/// The card for a unified diff of one file (the first, if it has more).
pub fn diff_card(diff: &str) -> Option<DiffCard> {
    let file = diffs::parse(diff).into_iter().next()?;
    // Context far from any change is noise on a card: keep two lines around.
    let mut lines: Vec<Line> = Vec::new();
    let mut seen_hunk = false;
    for line in file.lines {
        match line.kind {
            Kind::Hunk if !seen_hunk => seen_hunk = true,
            _ => lines.push(line),
        }
    }
    let lines = trim_context(lines, 2);
    (file.added + file.removed > 0 || !lines.is_empty()).then_some(DiffCard { path: file.path, added: file.added, removed: file.removed, lines })
}

/// Context lines kept only within `keep` of a change.
fn trim_context(lines: Vec<Line>, keep: usize) -> Vec<Line> {
    let changed: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| matches!(l.kind, Kind::Add | Kind::Del)).map(|(i, _)| i).collect();
    if changed.is_empty() {
        return lines;
    }
    lines
        .into_iter()
        .enumerate()
        .filter(|(i, l)| l.kind != Kind::Context || changed.iter().any(|c| c.abs_diff(*i) <= keep))
        .map(|(_, l)| l)
        .collect()
}

/// A command's output closed: its last lines, and how many come before them
/// (counting those the daemon already left out).
pub fn output_preview(output: &str, skipped: u32) -> (Vec<&str>, usize) {
    let lines: Vec<&str> = output.lines().collect();
    let from = lines.len().saturating_sub(OUTPUT_PREVIEW);
    (lines[from..].to_vec(), from + skipped as usize)
}

/// `0.4s`, `12s`, `1m 05s`.
pub fn duration(ms: u64) -> String {
    if ms < 1000 {
        let tenths = (ms + 50) / 100;
        let secs = if tenths >= 10 { "1".to_owned() } else { format!("0.{}", tenths.max(1)) };
        return equipe_core::t!("time.secs", s = secs);
    }
    crate::convo::elapsed(std::time::Duration::from_millis(ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,9 +1,9 @@\n one\n two\n three\n four\n-five\n+FIVE\n+five and a half\n six\n seven\n eight\n@@ -40,2 +41,2 @@\n-x\n+y\n";

    #[test]
    fn a_card_counts_the_change_and_shows_the_first_hunk() {
        let card = diff_card(DIFF).unwrap();
        assert_eq!((card.path.as_str(), card.added, card.removed), ("src/a.rs", 3, 2));
        let text = |lines: &[Line]| lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>();
        // Two lines of context around the change, not the far ones.
        assert_eq!(text(card.preview()), ["three", "four", "five", "FIVE", "five and a half", "six", "seven", "eight"]);
        assert_eq!(card.hidden(), 3, "the second hunk: its header and two lines");
        assert_eq!(card.preview()[2].kind, Kind::Del);
        assert!(diff_card("not a diff").is_none());
    }

    #[test]
    fn output_shows_its_end() {
        let (lines, before) = output_preview("a\nb\nc\nd\ne\nf", 10);
        assert_eq!(lines, ["c", "d", "e", "f"]);
        assert_eq!(before, 12);
        assert_eq!(output_preview("only", 0), (vec!["only"], 0));
    }

    #[test]
    fn durations_read_short() {
        assert_eq!(duration(30), "0.1초");
        assert_eq!(duration(420), "0.4초");
        assert_eq!(duration(980), "1초");
        assert_eq!(duration(12_400), "12초");
        assert_eq!(duration(65_000), "1분 05초");
    }
}
