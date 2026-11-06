//! The PM's weekly retro on screen (`equipe_core::retro`): its run headed
//! "Weekly retro · 2026-11-06" in the PM's conversation, its proposed changes
//! called "Weekly retro · N things" where they wait, and the past retros listed in
//! Progress — the day each covers up to, one line of what it said, the row
//! it is at. No UI types here, so the rules are tested without a window.

use equipe_core::protocol::Entry;
use equipe_core::protocol::decide::QuestionSet;
use equipe_core::t;

use crate::convo::Block;

/// Past retros listed, at most.
pub const SHOWN: usize = 5;
/// A past retro's line, at its longest.
const LINE: usize = 96;

/// What the retro's changes are called where they wait: "Weekly retro · 2 things".
/// `None`: any other set.
pub fn title(set: &QuestionSet) -> Option<String> {
    set.retro.is_some().then(|| t!("decide.retro", n = set.questions.len()))
}

/// The heading a retro's run is shown under: "Weekly retro · 2026-11-06", the
/// day its week runs up to. `None`: any other message.
pub fn heading(text: &str) -> Option<String> {
    let (_, to) = equipe_core::retro::week_of(text)?;
    Some(t!("chat.retro_ran", date = to))
}

/// A retro gone by: the day its week runs up to, and one line of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Past {
    pub to: String,
    pub line: String,
}

/// The retros in a conversation, newest first, at most [`SHOWN`]: each the
/// first line the PM answered with — past its heading — or, before it has
/// answered, what the facts count.
pub fn past(entries: &[Entry]) -> Vec<Past> {
    let mut out: Vec<Past> = Vec::new();
    let mut open: Option<usize> = None;
    for entry in entries {
        match entry {
            Entry::User { text, .. } => match equipe_core::retro::week_of(text) {
                Some((_, to)) => {
                    let line = equipe_core::retro::counts(text).map(|(shipped, stuck)| t!("progress.retro_counts", shipped = shipped, stuck = stuck)).unwrap_or_default();
                    out.push(Past { to: to.to_owned(), line });
                    open = Some(out.len() - 1);
                }
                None => open = None,
            },
            Entry::Reply { text } => {
                if let Some(at) = open.take()
                    && let Some(line) = gist(text)
                {
                    out[at].line = line;
                }
            }
            _ => {}
        }
    }
    out.reverse();
    out.truncate(SHOWN);
    out
}

/// The row a retro's run is at in the conversation drawn, by the day its
/// week runs up to.
pub fn row_of(blocks: &[Block], to: &str) -> Option<usize> {
    blocks.iter().rposition(|b| matches!(b, Block::User(text) if equipe_core::retro::week_of(text).is_some_and(|(_, at)| at == to)))
}

/// The first line of a reply that says something: past its headings and
/// rules, its marks taken off, cut short.
fn gist(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.chars().all(|c| matches!(c, '-' | '*' | '_' | '=')))
        .map(|l| l.trim_start_matches(['-', '*', '>', ' ']).replace("**", "").replace('`', ""))
        .find(|l| !l.trim().is_empty())?;
    let line = line.trim();
    Some(if line.chars().count() <= LINE { line.to_owned() } else { format!("{}…", line.chars().take(LINE - 1).collect::<String>().trim_end()) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::decide::{Question, Retro};
    use equipe_core::protocol::routines::{Routine, Schedule};

    fn run(to: &str, shipped: usize, stuck: usize) -> String {
        let routine = Routine { id: 2, title: "주간 회고".into(), prompt: "이번 주를 돌아봐 주세요".into(), when: Schedule::Weekly { weekday: 5, hour: 17, minute: 0 }, on: true, last_run: None, next_run: None };
        let facts = format!("[Retro facts · 2026-10-30 – {to}]\nGathered.\n\nShipped ({shipped}):\n- x\n\nStuck, failed, sent back, late ({stuck}):\n- y");
        equipe_core::retro::message(&routine, &facts)
    }

    #[test]
    fn the_retros_changes_are_called_so_where_they_wait_and_its_run_is_headed_by_its_day() {
        let set = |retro: Option<Retro>| QuestionSet { id: 1, session: 1, member: "pm".into(), ticket: None, questions: vec![Question::default(), Question::default()], asked_at: 0, review: false, retro };
        assert_eq!(title(&set(Some(Retro::default()))).as_deref(), Some("주간 회고 · 2가지"));
        assert_eq!(title(&set(None)), None);
        assert_eq!(heading(&run("2026-11-06", 1, 0)).as_deref(), Some("주간 회고 · 2026-11-06"));
        assert_eq!(heading("[Routine · 주간 리포트]\n\n정리해 줘"), None, "any other routine keeps its own heading");
    }

    #[test]
    fn past_retros_are_listed_newest_first_each_a_line_of_what_it_said() {
        let entries = vec![
            Entry::User { text: run("2026-10-30", 3, 1), kept: Vec::new() },
            Entry::Reply { text: "## 주간 회고\n\n---\n**이번 주 3건이 메인에 들어갔고**, 막힌 건 하나예요.\n\n### 1. 출시".into() },
            Entry::User { text: "고마워요".into(), kept: Vec::new() },
            Entry::Reply { text: "네".into() },
            Entry::User { text: run("2026-11-06", 2, 4), kept: Vec::new() },
        ];
        let listed = past(&entries);
        assert_eq!(
            listed,
            [
                Past { to: "2026-11-06".into(), line: "출시 2 · 막힘 4".into() },
                Past { to: "2026-10-30".into(), line: "이번 주 3건이 메인에 들어갔고, 막힌 건 하나예요.".into() },
            ],
            "not answered yet: what the facts count"
        );
        let many: Vec<Entry> = (1..=7).map(|d| Entry::User { text: run(&format!("2026-11-0{d}"), 0, 0), kept: Vec::new() }).collect();
        assert_eq!(past(&many).iter().map(|p| p.to.as_str()).collect::<Vec<_>>(), ["2026-11-07", "2026-11-06", "2026-11-05", "2026-11-04", "2026-11-03"]);
        assert!(past(&[Entry::User { text: "[Routine · 주간 리포트]\n\n정리".into(), kept: Vec::new() }]).is_empty());
    }

    #[test]
    fn a_past_retro_opens_at_its_row() {
        let blocks = vec![Block::User("안녕".into()), Block::User(run("2026-10-30", 0, 0)), Block::Note("x".into()), Block::User(run("2026-11-06", 0, 0))];
        assert_eq!(row_of(&blocks, "2026-10-30"), Some(1));
        assert_eq!(row_of(&blocks, "2026-11-06"), Some(3));
        assert_eq!(row_of(&blocks, "2026-01-01"), None);
        let long = format!("{}끝", "가".repeat(200));
        assert_eq!(gist(&long).map(|l| l.chars().count()), Some(LINE));
    }
}
