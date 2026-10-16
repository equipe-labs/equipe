//! What the person says to a member while it works, as the engine gets it and
//! echoes it back: each steer opens with the project it is about (as a
//! message does), several may be read at one step, one taken back is followed
//! by the word to disregard it. Pure, so the rules are tested here.

use equipe_protocol::transcript::scope::{LOOKING_AT, WHOLE_SQUAD};

/// How the word that a steer was taken back opens.
pub const TAKEN_BACK: &str = "[equipe: the person took back a message they sent while you work, before you read it — disregard it: ";

/// A steer about `project` as it goes into the turn (`None`: the whole
/// squad, said when `whole` — the PM's desk with several projects).
pub fn words(text: &str, project: Option<&str>, whole: bool) -> String {
    match project {
        Some(project) => format!("{LOOKING_AT}{project} — this message is about it]\n\n{text}"),
        None if whole => format!("{WHOLE_SQUAD} — this message is about all of it]\n\n{text}"),
        None => text.to_owned(),
    }
}

/// One part of a message sent at once about several projects.
pub fn part(text: &str, project: Option<&str>) -> String {
    match project {
        Some(project) => format!("{LOOKING_AT}{project} — this part is about it]\n\n{text}"),
        None => format!("{WHOLE_SQUAD} — this part is about all of it]\n\n{text}"),
    }
}

/// The word that the steer `text` was taken back.
pub fn taken_back(text: &str) -> String {
    format!("{TAKEN_BACK}{text}]")
}

/// Whether a line opens a steer of equipe's.
fn opens(line: &str) -> bool {
    line.starts_with(LOOKING_AT) || line.starts_with(WHOLE_SQUAD) || line.starts_with(TAKEN_BACK)
}

/// What the engine echoed as it read the person's steers, as the steers it
/// is: each without the project it opened with (a word that one was taken
/// back stays whole) — one per steer, when several were read as one; those
/// still `pending` are found in it even without an opening.
pub fn read(raw: &str, pending: &[String]) -> Vec<String> {
    // Where each steer of equipe's opens.
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in raw.split_inclusive('\n') {
        if opens(line) && !current.trim().is_empty() {
            parts.push(std::mem::take(&mut current));
        }
        current.push_str(line);
    }
    if !current.trim().is_empty() || parts.is_empty() {
        parts.push(current);
    }
    let mut out = Vec::new();
    for part in parts {
        let part = strip(&part);
        if pending.contains(&part) || part.starts_with(TAKEN_BACK) {
            out.push(part);
            continue;
        }
        // Several without an opening, read as one: the pending ones, in order.
        match cover(&part, pending) {
            Some(found) => out.extend(found),
            None => out.push(part),
        }
    }
    out
}

/// A part without the project it opened with.
fn strip(part: &str) -> String {
    let part = part.trim();
    if (part.starts_with(LOOKING_AT) || part.starts_with(WHOLE_SQUAD))
        && let Some(end) = part.find(']')
    {
        return part[end + 1..].trim().to_owned();
    }
    part.to_owned()
}

/// `part` as pending steers one after another, if it is.
fn cover(part: &str, pending: &[String]) -> Option<Vec<String>> {
    let mut rest = part;
    let mut found = Vec::new();
    for p in pending {
        if p.is_empty() {
            continue;
        }
        if let Some(after) = rest.strip_prefix(p.as_str()) {
            found.push(p.clone());
            rest = after.trim_start();
            if rest.is_empty() {
                return (found.len() > 1).then_some(found);
            }
        }
    }
    None
}

/// What waits, `first` first in its order, then the rest as they were.
pub fn order<T: Clone>(mut waiting: Vec<(String, T)>, first: &[String]) -> Vec<(String, T)> {
    let mut out = Vec::with_capacity(waiting.len());
    for text in first {
        if let Some(at) = waiting.iter().position(|(t, _)| t == text) {
            out.push(waiting.remove(at));
        }
    }
    out.extend(waiting);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|t| (*t).to_owned()).collect()
    }

    #[test]
    fn a_steer_opens_with_the_project_it_is_about() {
        assert_eq!(words("fix it", Some("site"), true), "[equipe: the person is looking at the project site — this message is about it]\n\nfix it");
        assert!(words("all", None, true).starts_with(WHOLE_SQUAD));
        assert_eq!(words("plain", None, false), "plain");
        // The conversation reads it the way it reads a message.
        assert_eq!(equipe_protocol::transcript::scope::said_about(&words("x", Some("app"), true)), Some(Some("app".into())));
    }

    #[test]
    fn one_steer_read_is_itself_without_its_opening() {
        let p = pending(&["fix it"]);
        assert_eq!(read(&words("fix it", Some("site"), true), &p), ["fix it"]);
        assert_eq!(read("fix it", &p), ["fix it"]);
        assert_eq!(read("not pending", &p), ["not pending"], "an echo equipe didn't send is shown as it is");
    }

    #[test]
    fn several_read_at_one_step_are_each_their_own_with_their_projects_gone() {
        let p = pending(&["a\n\nwith a paragraph", "b", "c"]);
        let raw = format!("{}\n\n{}\n{}", words("a\n\nwith a paragraph", Some("site"), true), words("b", Some("app"), true), words("c", None, true));
        assert_eq!(read(&raw, &p), ["a\n\nwith a paragraph", "b", "c"]);
        // Without openings, joined as one: the pending ones in order.
        assert_eq!(read("b\n\nc", &p), ["b", "c"]);
        assert_eq!(read("b\nc", &p), ["b", "c"]);
        // A part that only starts like one stays whole.
        assert_eq!(read("bc and more", &p), ["bc and more"]);
    }

    #[test]
    fn the_word_that_one_was_taken_back_stays_whole_to_be_left_out() {
        let p = pending(&["keep"]);
        let raw = format!("{}\n\n{}", taken_back("never mind"), words("keep", Some("site"), true));
        let got = read(&raw, &p);
        assert_eq!(got.len(), 2);
        assert!(got[0].starts_with(TAKEN_BACK), "{got:?}");
        assert_eq!(got[1], "keep");
    }

    #[test]
    fn what_is_sent_at_once_goes_in_the_order_asked_then_the_rest() {
        let waiting = vec![("s1".to_owned(), 1), ("s2".to_owned(), 2), ("m1".to_owned(), 3)];
        assert_eq!(order(waiting.clone(), &pending(&["m1", "s1"])), [("m1".to_owned(), 3), ("s1".to_owned(), 1), ("s2".to_owned(), 2)]);
        assert_eq!(order(waiting.clone(), &[]), waiting);
        assert_eq!(order(waiting.clone(), &pending(&["gone"])), waiting, "a text not waiting any more is passed over");
    }
}
