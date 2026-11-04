//! What an engine's harness says into the conversation in the person's place.
//!
//! Claude Code tells a member that a command it left running in the background
//! has finished by putting a user turn into the conversation: a
//! `<task-notification>` envelope (`task-id`, `tool-use-id`, `output-file`,
//! `status`, `summary`), then a line pointing at the output file. It glues
//! `<system-reminder>` blocks onto a turn the same way. The engine echoes
//! these as if the person had typed them; they are the harness's, never the
//! person's words. [`read`] tells the two apart, so no screen shows them as
//! the person speaking, nor counts them so.

use std::borrow::Cow;

/// The envelope a finished background task comes in.
const TASK: &str = "task-notification";
/// What the harness says beside a turn, for the model only.
const REMINDER: &str = "system-reminder";
/// The line Claude Code puts after a task's envelope.
const READ_OUTPUT: &str = "Read the output file to retrieve the result:";

/// A background task the member started that has ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskDone {
    pub task_id: Option<String>,
    pub tool_use_id: Option<String>,
    /// Where its output was written, when it says.
    pub output_file: Option<String>,
    /// `completed`, `failed`, `killed`… as the engine says.
    pub status: Option<String>,
    /// What it was and how it ended, in the engine's words.
    pub summary: Option<String>,
}

impl TaskDone {
    /// Whether it ended any other way than completing.
    pub fn failed(&self) -> bool {
        self.status.as_deref().is_some_and(|s| !matches!(s.trim().to_ascii_lowercase().as_str(), "completed" | "complete" | "done" | "success" | "succeeded"))
    }
}

/// A user turn told apart: the tasks the harness reported in it, and the
/// person's own words left over (empty when there are none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said<'a> {
    pub tasks: Vec<TaskDone>,
    /// Whether the harness said anything in it at all.
    pub harness: bool,
    pub words: Cow<'a, str>,
}

impl Said<'_> {
    /// Whether the person said anything in it.
    pub fn persons(&self) -> bool {
        !self.words.trim().is_empty()
    }
}

/// `text` — a user turn as the engine echoed it — told apart. Only what opens
/// the turn or closes it counts as the harness's: an envelope the person
/// pasted inside their own words stays theirs. A turn the harness had no
/// part in comes back whole, as it was.
pub fn read(text: &str) -> Said<'_> {
    let mut tasks = Vec::new();
    let mut harness = false;
    let mut rest = text;
    loop {
        let open = rest.trim_start();
        if let Some((body, after)) = element(open, TASK) {
            tasks.push(task(body));
            harness = true;
            rest = after_pointer(after);
        } else if let Some((_, after)) = element(open, REMINDER) {
            harness = true;
            rest = after;
        } else {
            break;
        }
    }
    // Reminders glued on at the end.
    loop {
        let close = rest.trim_end();
        match close.strip_suffix(&format!("</{REMINDER}>")).and_then(|before| before.rfind(&format!("<{REMINDER}>"))) {
            Some(at) => {
                harness = true;
                rest = &close[..at];
            }
            None => break,
        }
    }
    let words = if harness { Cow::Owned(rest.trim().to_owned()) } else { Cow::Borrowed(text) };
    Said { tasks, harness, words }
}

/// The person's own words in `text`, when there are any.
pub fn words(text: &str) -> Option<Cow<'_, str>> {
    let said = read(text);
    said.persons().then_some(said.words)
}

/// Whether `text` holds nothing of the person's: all the harness's.
pub fn only_harness(text: &str) -> bool {
    let said = read(text);
    said.harness && !said.persons()
}

/// `<name>…</name>` opening `text`: what is inside, and what follows.
fn element<'a>(text: &'a str, name: &str) -> Option<(&'a str, &'a str)> {
    let inner = text.strip_prefix('<')?.strip_prefix(name)?;
    // `<name>` or `<name attr…>`, never a longer name.
    let inner = match inner.chars().next()? {
        '>' => &inner[1..],
        c if c.is_whitespace() => &inner[inner.find('>')? + 1..],
        _ => return None,
    };
    let close = format!("</{name}>");
    let end = inner.find(&close)?;
    Some((&inner[..end], &inner[end + close.len()..]))
}

/// What follows a task's envelope, past the line that points at its output.
fn after_pointer(after: &str) -> &str {
    let open = after.trim_start();
    match open.strip_prefix(READ_OUTPUT) {
        Some(line) => line.split_once('\n').map_or("", |(_, rest)| rest),
        None => after,
    }
}

/// The tags of a task's envelope, wherever they are in it.
fn task(body: &str) -> TaskDone {
    let tag = |name: &str| {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        let from = body.find(&open)? + open.len();
        let to = body[from..].find(&close)? + from;
        Some(body[from..to].trim().to_owned()).filter(|v| !v.is_empty())
    };
    TaskDone { task_id: tag("task-id"), tool_use_id: tag("tool-use-id"), output_file: tag("output-file"), status: tag("status"), summary: tag("summary") }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTIFICATION: &str = "<task-notification>\n<task-id>b16yoo5kk</task-id>\n<tool-use-id>toolu_01AbC</tool-use-id>\n<output-file>/private/tmp/claude-501/tasks/b16yoo5kk.output</output-file>\n<status>completed</status>\n<summary>Background command \"Run the tests\" completed (exit code 0)</summary>\n</task-notification>\nRead the output file to retrieve the result: /private/tmp/claude-501/tasks/b16yoo5kk.output";

    #[test]
    fn a_task_notification_is_the_harness_and_says_what_ended_and_how() {
        let said = read(NOTIFICATION);
        assert!(said.harness && !said.persons(), "{said:?}");
        assert_eq!(
            said.tasks,
            vec![TaskDone {
                task_id: Some("b16yoo5kk".into()),
                tool_use_id: Some("toolu_01AbC".into()),
                output_file: Some("/private/tmp/claude-501/tasks/b16yoo5kk.output".into()),
                status: Some("completed".into()),
                summary: Some("Background command \"Run the tests\" completed (exit code 0)".into()),
            }]
        );
        assert!(!said.tasks[0].failed());
        assert!(only_harness(NOTIFICATION) && words(NOTIFICATION).is_none());
    }

    #[test]
    fn the_envelope_is_read_whatever_its_layout_and_with_tags_missing() {
        let one_line = "  <task-notification><task-id>b1</task-id><status>failed</status><summary>exit 1</summary></task-notification>";
        let said = read(one_line);
        assert_eq!(said.tasks.len(), 1);
        assert_eq!(said.tasks[0].output_file, None);
        assert!(said.tasks[0].failed(), "failed");
        assert!(read("<task-notification><status>killed</status></task-notification>").tasks[0].failed(), "killed is not done");
        assert!(!read("<task-notification><summary>x</summary></task-notification>").tasks[0].failed(), "no status: not a failure");
        // Two at once, and the person's words read with them.
        let two = format!("{NOTIFICATION}\n{NOTIFICATION}\n\nand skip the footer");
        let said = read(&two);
        assert_eq!(said.tasks.len(), 2);
        assert_eq!(said.words, "and skip the footer");
    }

    #[test]
    fn reminders_opening_or_closing_a_turn_are_not_the_persons_words() {
        assert!(only_harness("<system-reminder>\nThe date changed.\n</system-reminder>"));
        let said = read("fix the header\n\n<system-reminder>Todo list is empty.</system-reminder>");
        assert!(said.harness && said.tasks.is_empty());
        assert_eq!(said.words, "fix the header");
        assert_eq!(words("<system-reminder>a</system-reminder>\nhi").as_deref(), Some("hi"));
    }

    #[test]
    fn the_persons_own_words_stay_whole_even_when_they_quote_an_envelope() {
        for text in ["fix the header", "  spaced  ", "I saw <task-notification><task-id>b1</task-id></task-notification> in the chat", "<task-notificationx>", "<task-notification> never closed", "<system-reminders>no</system-reminders>"] {
            let said = read(text);
            assert!(!said.harness && said.tasks.is_empty(), "{text}");
            assert!(matches!(said.words, Cow::Borrowed(t) if t == text), "{text}");
        }
    }
}
